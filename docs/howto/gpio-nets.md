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
| `pull` | The board's resistor to a rail. A weak level, like a chip's internal pull (see below). |
| `latency_ns` | Wire delay from a pad edge to every member seeing it. Default 100 ns. Zero is refused, and so is anything below one cycle of the slowest member. |

## What the net does

For the drives present at one instant, each member being driving-0 or
driving-1 (strong), held by its own internal pull-up or pull-down (weak), or
released with no pull:

1. any member driving 0 gives 0, else any driving 1 gives 1;
2. else the weak sources, which are the members' internal pulls and the net's
   `pull`: all of them up gives 1, all down gives 0;
3. weak pulls to both rails with nothing driving are a resistor divider. The
   net reports `GPIO_NET_PULL_CONFLICT` (begin and end times, every member's
   drive) and reads the net's own `pull` (the board resistor is normally much
   stronger than a chip's 30-50 kOhm internal one). With no net `pull` it
   reads 0, as contention does;
4. else the net floats: it reads 0 and is flagged `GPIO_NET_FLOATING`.

A chip's internal pull is therefore part of the net. An input with
`PUPDR = 01` on an STM32, or an ATmega pad with `DDRx = 0` and `PORTx = 1`,
holds a wire high with no `pull:` on the net
(`examples/gpio-net-f1-f4`). Reports show it as the member's drive:
`pull_up` / `pull_down` next to `z`, `low` and `high`. On the pad's own
four-state trace a pulled net pad reads `h` / `l` (the IEEE 1164 weak levels).
Pads that are not on a net keep reporting `z` for an undriven input, pulled or
not.

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
EXTI interrupts, AVR INT0/INT1 and pin-change interrupts, and timer captures
see a real edge.

Each node keeps its own clock (conservative parallel discrete-event
simulation). A node may run as far as its *safe horizon*: for each net it is
on, the time the slowest member of that net has reached, plus the net's
latency. An edge nobody has reported yet happens after its driver's current
time, so its delivery is due after that horizon: every delivery a node needs is
known before the node gets there, whatever order the nodes run in and however
far apart their clocks are. After a node runs, its drive changes are merged
into its nets up to the time all their members have reached.

Results do not depend on node order, round length or how the world is driven:
`world_multichip.rs` (`gpio_net_world`) runs the example with both node orders,
with rounds from 10 ns to 100 ns, round by round, in one `run_until_ps` call and
on the old lockstep round driver (`set_gpio_lockstep`, still the driver for a
world that also has a timed UART network), and compares every counter, every
UART transcript, every applied delivery and every node's cycle count, part way
through and at the end.

A node skips idle time (a Cortex-M in `WFI`, an ATmega328P in `SLEEP`) only
when idle fast-forward is on for it (`set_idle_fast_forward(true)` on the
world's machine); the results are the same either way
(`idle_fast_forward_skips_the_avr_sleep_and_changes_nothing`).

`step_all` still advances the world by one round (the shortest latency) per
call, so a `max_steps` limit, the browser's step batches and Python's
`run_for` keep their meaning. `World::run_until_ps(t)` runs to `t` in one call,
and `World::step_rounds(n)` does `n` rounds in one call when that gives the
same result (the browser's `step_batch` uses it).

### Speed

At the default 100 ns latency a net no longer costs a multiple of the
machines' own time. `examples/gpio-net-two-boards`, 30 ms of simulated time,
release build, against the same two machines each run alone for 30 ms (median
of repeated runs on a shared machine, so treat the figures as rough):

| How it is driven | Before | Now |
|------------------|--------|-----|
| one `run_until_ps` / `step_rounds` call | n/a | 1.16x the two machines alone |
| `step_all` per round | 2.05x | 1.55x |
| `labwired test --script .../test.yaml` (40 ms, per round, wall) | 0.54 s | 0.36 s |

(`cargo test --release -p labwired-core --test world_multichip -- --ignored
--nocapture gpio_net_speed` prints the first two.)

What is left:

- A net node still runs in pieces no longer than the shortest latency of its
  nets, because its own edges come back to it after one latency. The world
  nodes built today (Cortex-M, AVR) tick their peripherals every cycle anyway,
  so the pieces cost almost nothing extra. A node that would otherwise run
  wide batches or fast-forward idle time (an ESP32-C3 ROM-boot node) loses
  that while it is on a net, so it runs slower than alone at 100 ns.
- A sleeping node does not stretch the horizon yet: a node idle in `WFI` still
  advances one latency at a time while its peers do.
- `step_all` pays a fixed cost per call (the results map), about half the
  machines' own time at 100 ns rounds. Drive long runs with `run_until_ps` or
  `step_rounds`; `latency_ns` still makes rounds longer where a wire does not
  need to be fast.

Machines on no net run unchanged and are not held to any net's latency.

## Which pads can be on a net

A net member's GPIO model has to report what it drives and accept an external
level. Supported today:

| GPIO model | Capture | Notes |
|------------|---------|-------|
| `GpioPort`, every register family (STM32 `v2` and `f1`, nRF52/54, Kinetis, EFR32 series 2, SAM, RA, i.MX RT) | push: the port reports its own edges, idle fast-forward stays on | exercised end to end on STM32 `v2` (G0B1, F401) and `f1` (F103). The EXTI raises edge interrupts for net edges on G0 and U5 (port from `EXTI_EXTICRx`), F1 (port from `AFIO_EXTICRx`) and F4 (port from `SYSCFG_EXTICRx`). Internal pulls reported: STM32 `v2` `PUPDR`, STM32 `f1` input-with-pull (`CNF = 10`, `ODR` picks the rail), nRF52 `PIN_CNF.PULL`, EFR32 `INPUTPULL`, SAM `PINCFG.PULLEN`. Kinetis, RA and i.MX RT keep their pull outside the GPIO block, so their pulls are not on the net yet. |
| `GpioPort` pads routed to a peripheral (AF) | as above | the peripheral says what its output stage does (driving, released, input) and reads the level the net delivers; see [Buses over nets](#buses-over-nets). STM32 SPI and modern I²C only. |
| `avr_gpio` (ATmega328P `portb`/`portc`/`portd`) | push: the port reports its own edges, idle fast-forward stays on | INT0/INT1 and PCINT0..2 see external edges; a core parked in `SLEEP` is skipped until the next edge or Timer0 overflow. Exercised end to end on the Uno (`gpio-net-two-boards`). An input with its `PORTx` bit set is a pull-up on the net. |
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

- The ATmega328P port model does not see `MCUCR.PUD` (it is in the CPU's IO
  space), so a pad with `PORTx = 1` counts as pulled up even when firmware
  has set `PUD`.
- A pad that firmware drives itself does not raise its own EXTI edge for its own
  transition (the STM32 EXTI model reacts to edges from outside). It does see
  the release when the wire rises after a peer let go.
- The ATmega328P samples INT0/INT1 and PCINT pads at instruction boundaries,
  so an edge is seen at the first boundary after it arrives. Waking from
  power-down, power-save or standby takes four cycles, as from idle: the
  oscillator start-up time is not modelled.
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
