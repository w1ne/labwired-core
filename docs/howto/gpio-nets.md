# GPIO nets between machines

A `gpio_net` interconnect joins GPIO pads of two or more world nodes into one
electrical net: an interrupt line, a ready line, a chip-select handshake, a wake
pin, or a shared open-drain alert line with a pull-up. Each member keeps
running its own firmware; the net decides what level the wire carries and when
each member sees it.

Runnable example: [`examples/gpio-net-two-boards`](../../examples/gpio-net-two-boards)
(an STM32G0B1 and an ATmega328P; `env-rp2040.yaml` and `env-esp32c6.yaml`
swap the ATmega328P for an RP2040 or an ESP32-C6).

## Manifest

One `gpio_net` per wire. `nodes` lists every node that owns a member, once each.

```yaml
interconnects:
  - type: gpio_net
    nodes: [avr, stm]
    config:
      name: alert              # optional label shown in reports
      pull: up                 # none (default) | up | down
      latency_ns: 100          # default 100; zero is refused
      members:
        - { node: stm, peripheral: gpiob, pin: 4 }   # open-drain output
        - { node: avr, peripheral: portd, pin: 4 }   # DDR drives low
```

| Key | Meaning |
|-----|---------|
| `members` | At least two pads on at least two nodes. `peripheral` is the node's GPIO peripheral id (`gpioa`, `portd`, `gpio` on an ESP32-family chip, `sio` on an RP2040, ...; see the table below). A pad can be on one net only; merge nets that share a pad. |
| `pull` | The resistor to a rail. Used when no member drives the wire. |
| `latency_ns` | Wire delay from a pad edge to every member seeing it. Default 100 ns. Zero is refused, and so is anything below one cycle of the slowest member. |

## What the net does

For the drives present at one instant, each member being driving-0, driving-1
or released:

1. any member driving 0 gives 0, else any driving 1 gives 1;
2. else the net's `pull`;
3. else the net floats: it reads 0 and is flagged `GPIO_NET_FLOATING`.

Members driving 0 and 1 together are in **contention**. The wire resolves to 0
(a low-side driver usually wins) and the net reports `GPIO_NET_CONTENTION` with
the time it began, the time it ended, and every member's drive and own cycle.
Nothing is dropped silently.

Open-drain falls out of this: pads that only pull low or release, plus
`pull: up`, make a wired-AND. If one member holds the wire low and a second
joins and then the first lets go, the wire stays low until the second releases:
one low pulse, not two (`open_drain_with_pull_up_is_a_wired_and` in
`crates/core/src/network/gpio_net.rs`).

A member's drive is what *its own* output stage does. What the net feeds back
into the pad (the level firmware reads from `IDR` or `PIND`) is never counted
as the pad's own drive, so a pad cannot hold a wire up just because it once
saw it high.

## Timing

Edges are delivered at `t_edge + latency_ns`, to every member including the
driver, at an exact cycle of each member (the first instruction boundary at or
after that time), through the same `set_gpio_input` path a board button uses:
EXTI interrupts and timer captures see a real edge.

The world runs in conservative rounds, the way a timed UART network does. A
round is never longer than the shortest net latency, so an edge produced in a
round can never be due at a peer before the round ends. Results do not depend
on the order nodes run in or on the round length: `world_multichip.rs` (`gpio_net_world`) runs the
example with both node orders and with rounds from 10 ns to 100 ns and compares
every counter, every UART transcript and every applied delivery.

`latency_ns` is also the speed knob. A 100 ns round is a few cycles per node,
so a world with only 100 ns nets runs slower than a lone machine; raise
`latency_ns` on wires that do not need to be that fast and rounds get longer.

Machines on no net run unchanged; the per-step cost of a machine is the same
with or without nets elsewhere in the world.

## Which pads can be on a net

A net member's GPIO model has to report what it drives and accept an external
level. Supported today:

| GPIO model | Capture | Notes |
|------------|---------|-------|
| `GpioPort`, every register family (STM32 `v2` and `f1`, nRF52/54, Kinetis, EFR32 series 2, SAM, RA, i.MX RT) | push: the port reports its own edges, idle fast-forward stays on | exercised end to end on STM32 `v2` (G0B1). EXTI sees external GPIO edges on the G0 and U5 EXTI only; the F1/F4 EXTI model does not, so count edges there by polling. |
| `GpioPort` pads routed to a peripheral (AF) | as above | the peripheral says what its output stage does (driving, released, input) and reads the level the net delivers; see [Buses over nets](#buses-over-nets). STM32 SPI and modern I²C only. |
| `avr_gpio` (ATmega328P `portb`/`portc`/`portd`) | **poll**: sampled at every cycle boundary | exact, but the machine runs one instruction at a time and does not fast-forward idle time while a pad is on a net |
| ESP32 classic `gpio` (member `peripheral: gpio`, pins 0..31) | push | Drive: `GPIO_ENABLE` and `GPIO_OUT`; `GPIO_PINn.PAD_DRIVER` open drain drives only a 0 and, holding a 1, reads the wire on `GPIO_IN`. Interrupts: `GPIO_PINn.INT_TYPE` edge and level types latch `GPIO_STATUS` and raise matrix source 22 for the CPU whose INT_ENA bit is set. GPIO32..39 cannot join a net. |
| ESP32-S3 `gpio` (`peripheral: gpio`, pins 0..31) | push | As classic; matrix source 16 (`GPIO_PCPU_INT`, INT_ENA bit 13). GPIO32..48 cannot join a net. |
| ESP32-C3 / ESP32-C6 `gpio` (`peripheral: gpio`, pins 0..25) | push | As classic; matrix source 16 on the C3 and 30 on the C6. Edge types only (level types are not modelled on this block). Exercised end to end on the C6 (`env-esp32c6.yaml`). |
| RP2040 `sio` (`peripheral: sio`, pins 0..29 = GP0..GP29) | push | Drive: `GPIO_OE` and `GPIO_OUT` while IO_BANK0 selects SIO for the pad (or nothing yet); open drain is firmware toggling `GPIO_OE` with the latch at 0. Interrupts: IO_BANK0 `INTRn` / `PROC0_INTEn` edge and level bits raise `IO_IRQ_BANK0` (NVIC 13) from any change of `GPIO_IN`, the pad's own output included. Exercised end to end (`env-rp2040.yaml`). |

On the ESP32 family and the RP2040 the GPIO interrupt sees the pad as
`GPIO_IN` reports it. On the classic ESP32 and the RP2040 that includes the
pad's own output; on the C3/C6 and S3 `GPIO_IN` is the external level only,
so a pad does not interrupt on its own output there.

Any other GPIO model is refused when the world is built, naming the pad.

Limits worth knowing:

- A chip's **internal** pull-ups and pull-downs are not part of the net; put the
  pull on the net (`pull:`), as the external resistor it is.
- A pad that firmware drives itself does not raise its own EXTI edge for its own
  transition (the STM32 EXTI model reacts to edges from outside). It does see
  the release when the wire rises after a peer let go.
- The ATmega328P model has no external or pin-change interrupt yet: count its
  edges by polling `PINx`.
- A pad routed to a peripheral signal the model does not publish has no known
  drive and is refused. Pads start as plain GPIO, so this is checked when the
  world is built; a pad firmware later routes to such a signal keeps its last
  known drive on the net.
- The RP2040 `GPIOn_CTRL` override fields (`OUTOVER`, `OEOVER`, `INOVER`,
  `IRQOVER`) are stored but not applied, and only PROC0's interrupt is
  raised (the model is single-core).

## Buses over nets

A hardware SPI or I²C peripheral whose pads are on nets talks to the other
chip bit by bit, at the pads. There is no byte-level shortcut between the
peripherals: the bits are the net's edges, so the net reports, a logic
analyzer on either board and the timing all show the real waveform, and the
bus behaves like wires. Runnable example:
[`examples/gpio-net-buses`](../../examples/gpio-net-buses) (two STM32G071s).

One net per wire, as for any other signal:

```yaml
  - type: gpio_net                     # I²C: open drain, pull-up on the net
    nodes: [a, b]
    config:
      name: sda
      pull: up
      members:
        - { node: a, peripheral: gpiob, pin: 7 }   # I2C1_SDA (AF6)
        - { node: b, peripheral: gpiob, pin: 7 }
```

**SPI** (STM32 classic/FIFO SPI, `pad_map: stm32g0` routing on the G071):
SCK, MOSI, MISO and NSS are push-pull nets. A master (`MSTR=1`, `SPE=1`)
drives SCK and MOSI and samples the level the net delivers to its MISO pad at
its own sampling edge; that is what lands in `DR`. A slave (`MSTR=0`) has no
clock of its own: it reacts to the SCK, MOSI and NSS edges the net delivers,
shifts MOSI in on its sampling edge, puts its `DR` word out on MISO on its
shift edge (any CPOL/CPHA, MSB or LSB first, 8 or 16 bits; the example and
tests exercise mode 0, MSB first, 8 bits), sets
RXNE with the SPI interrupt when `RXNEIE` is set, and OVR when a frame arrives
before `DR` was read. It drives MISO only while selected: `SSM=1, SSI=0`, or
`SSM=0` with its NSS pad low. A master with `SSOE=1` drives NSS low while
enabled. A disabled SPI drives nothing.

**I²C** (STM32 modern I²C, the L4/G0 `TIMINGR` register file, `pad_map:
stm32g0` on the G071): SCL and SDA are open-drain nets; put `pull: up` on
them. The controller generates START, the 7-bit address, data, ACK/NACK,
repeated START and STOP with `SCLL`/`SCLH`/`SDADEL` timing, and counts the SCL
high period from when it *sees* SCL high, so a target holding SCL low stretches
the clock. A NACK sets `NACKF` and sends STOP; `AUTOEND` sends STOP after
`NBYTES`, otherwise `TC` holds SCL low for a repeated START. A released SDA
read back low is arbitration lost (`ARLO`). A target (`OAR1.OA1EN`) watches
START/STOP and its address on the wire, ACKs its own address only (`ADDR`,
`DIR`, `ADDCODE`), and stretches SCL until firmware clears `ADDR`, reads
`RXDR` or writes `TXDR`. A transfer to an address nobody owns is the pull-up
reading 1 in the ACK slot.

Everything is event-scheduled: a master's bits run at their exact cycles, a
slave or target runs when the net delivers an edge. The one poll is a target
or controller stretching SCL until firmware reads `RXDR` (a register read
cannot wake the model), at a quarter of the SCL low period.

Two physical limits come with the wires:

- An answer crosses the wire twice. MISO answers a clock edge only after the
  edge reached the slave and the answer came back, so two `latency_ns` must
  fit in half an SCK period; slow SCK (`CR1.BR`) or shorten the latency when
  they do not. The same holds for I²C data and ACKs inside the SCL low period.
- Give SCL and SDA (and SCK and MOSI) the same `latency_ns`. The receiver
  tells data from START/STOP by the order the two wires change in.

Not modelled on a net yet: other SPI/I²C families (nRF, ESP32, RP2040, the
STM32 F1/F4 legacy I²C, the H5 SPI v3), I²C 10-bit addressing, OAR2, general
call, `RELOAD` (more than 255 bytes), SMBus/PEC, `NOSTRETCH=1`, the I²C
filters and timeouts, SPI CRC, TI mode and DMA on a slave. Such a peripheral
on a net keeps its usual behaviour and drives its lines push-pull as before.

## Reports

`World::gpio_net_reports()` (and `result.json` `gpio_nets` from
`labwired test`, and `WasmWorld.gpio_net_report()` in the browser) lists each
net: level, edge count, contention and floating counters, each member's drive,
and the diagnostics with times in picoseconds.
