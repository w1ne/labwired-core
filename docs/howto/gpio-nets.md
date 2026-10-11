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
`pull_up` / `pull_down` next to `z`, `low` and `high`. The world watches each
member's own output stage (a `LogicSource::Driver` channel), on which a pulled
pad reads `h` / `l` (the IEEE 1164 weak levels). A probe on the pad
(`LogicSource::Pad`, the ordinary four-state pin trace) shows the wire as the
pad sees it: `z` for an undriven input, pulled or not, the level a peer
drives, and `x` where the pad's own output fights it.

Members driving 0 and 1 together are in **contention**. The wire resolves to 0
(a low-side driver usually wins) and the net reports `GPIO_NET_CONTENTION` with
the time it began, the time it ended, and every member's drive and own cycle.
Nothing is dropped silently.

Open-drain falls out of this: pads that only pull low or release, plus
`pull: up`, make a wired-AND. If one member holds the wire low and a second
joins and then the first lets go, the wire stays low until the second releases:
one low pulse, not two (`open_drain_with_pull_up_is_a_wired_and` in
`crates/core/src/network/gpio_net.rs`).

A member's drive is what *its own* output stage does
([`PinPort::driver`](../architecture/pins.md), decoded from the chip's
registers only). What the net feeds back into the pad (the level firmware
reads from `IDR` or `PIND`) is never part of it, so a pad cannot hold a wire up
just because it once saw it high.

## Timing

Edges are delivered at `t_edge + latency_ns`, to every member including the
driver, at an exact cycle of each member (the first instruction boundary at or
after that time), through the same `PinPort::set_external` path a board button
uses (`SystemBus::set_pad_external`): EXTI interrupts, AVR INT0/INT1 and
pin-change interrupts, the ESP32-family and RP2040 GPIO interrupts, and timer
captures see a real edge.

Each node keeps its own clock (conservative parallel discrete-event
simulation). A node may run as far as its *safe horizon*: for each net it is
on, the earliest time a change it has not been told about can have happened,
plus the net's latency. Three things bound that time:

- each other member's *known time*: the time it has reached, since everything
  it drove before then is already reported;
- for a member asleep until a scheduled event (a Cortex-M in `WFI`, an
  ATmega328P in `SLEEP`, with idle fast-forward on), that event instead: it
  drives nothing before it unless an input wakes it, so the bound is the
  earliest of the event, a delivery already queued for it, an edge not yet
  merged into one of its nets (one latency later), and a peer's own known
  time plus one latency;
- the earliest edge not yet merged into the net.

A node's own edges come back to it too, one latency later. Rather than hold a
node to one latency past its own time, the scheduler runs it until its next
own drive change: a push of a net pad ends the CPU batch it happens in
(`LogicTap::set_batch_stop_channels`), the machine stops at that boundary
(`Machine::advance_to_cycle_or_net_drive_change`), and the edge bounds the
node's horizon before it runs on. A node that cannot stop there (two cores, or
a pad on the per-cycle poll) keeps its own time in its horizon.

An edge nobody has reported yet happens after its driver's known time, so its
delivery is due after the horizon: every delivery a node needs is known before
the node gets there, whatever order the nodes run in and however far apart
their clocks are. After a node runs, its drive changes are merged into its nets
up to the time all their members have reached.

Results do not depend on node order, round length or how the world is driven:
`world_multichip.rs` (`gpio_net_world`) runs the example with both node orders,
with rounds from 10 ns to 100 ns, round by round, in one `run_until_ps` call and
on the old lockstep round driver (`set_gpio_lockstep`, still the driver for a
world that also has a timed UART network), with idle fast-forward off and on,
and compares every counter, every UART transcript, every applied delivery and
every node's cycle count, part way through and at the end
(`the_per_node_scheduler_matches_the_lockstep_rounds`); the same for the
ESP32-C6 world (`the_esp32c6_world_is_the_same_on_every_driver`) and the SPI
and I²C buses over nets (`world_gpio_net_buses.rs`).

A node skips idle time (a Cortex-M in `WFI`, an ATmega328P in `SLEEP`) only
when idle fast-forward is on for it (`set_idle_fast_forward(true)` on the
world's machine); the results are the same either way
(`idle_fast_forward_skips_the_avr_sleep_and_changes_nothing`).

`step_all` still advances the world by one round (the shortest latency) per
call, so a `max_steps` limit, the browser's step batches and Python's
`run_for` keep their meaning. `World::step_all_into(&mut results)` is the same
call with a result buffer (`StepResults`) kept from round to round instead of a
new map of node ids each time; `labwired test` and Python step with it.
`World::run_until_ps(t)` runs to `t` in one call, and `World::step_rounds(n)`
does `n` rounds in one call when that gives the same result (the browser's
`step_batch` uses it).

### Speed

`examples/gpio-net-two-boards`, 30 ms of simulated time, release build with
the event scheduler, against the same two machines each run alone for 30 ms.
Instruction counts (callgrind, millions, deterministic; the ratio is to the
two machines alone, 1023 M) say more than wall time on a shared machine:

| How it is driven | Before | Now |
|------------------|--------|-----|
| one `run_until_ps` / `step_rounds` call | 1303 M (1.27x) | 1015 M (0.99x) |
| the same, idle fast-forward on | 1395 M | 1104 M |
| `step_all` per round | 2284 M (2.23x) | 1985 M (1.94x) |
| `step_all_into` per round (`labwired test`, Python) | n/a | 1556 M (1.52x) |
| `labwired test --script .../test.yaml` (40 ms, per round, whole process) | 3516 M, 0.32 s wall | 2335 M, 0.26 s wall |

Wall time agrees within its noise (median of 6 runs, ±15% on a loaded
4-core machine): `run_until_ps` 0.124 s before, 0.083 s now, against 0.077 s
for the two machines alone. With idle fast-forward on there is no fair
"alone" figure: alone, the AVR sleeps through the whole run waiting for a peer
that is not there. The ESP32-C6 world (`env-esp32c6.yaml`, 2 ms, instruction
counts) costs what its C6 costs: `run_until_ps` is 1.01x the two machines
alone (1.04x before), `step_all` 1.09x (1.11x).

(`cargo test --release -p labwired-core --features event-scheduler --test
world_multichip -- --ignored --nocapture gpio_net_speed` prints the wall
times and how many pieces the scheduler cut the run into;
`GPIO_NET_BENCH_ENV=env-esp32c6.yaml` and `GPIO_NET_BENCH_FF=1` pick the
other cases.)

What made the difference:

- A node runs until its own next net pad drive change, not one latency past
  its own time, and its pad state is read only when it stopped for one. On a
  net it keeps whatever batch width and idle fast-forward it has alone, and two
  busy nodes leapfrog by two latencies instead of one.
- A node asleep until its next event, with idle fast-forward on, holds its
  peers back only as far as that event, or whatever input could wake it first.
- A call that ends within one latency of the slowest node (a `step_all`
  round) skips the horizons altogether, and `step_all_into` reuses its result
  buffer instead of building a map of node ids per round.

What is still slower than the machines alone, and why:

- `step_all` stops every node at every round end: at 100 ns and 16 MHz that is
  one or two instructions per node per call, so the per-call cost (and the map
  `step_all` returns) dominates. Drive long runs with `run_until_ps` or
  `step_rounds`; `latency_ns` makes rounds longer where a wire does not need
  to be fast.
- Two busy nodes cannot run far apart: neither can know when the other will
  next drive a pad, so each runs at most about two latencies ahead of the
  other (the example's 30 ms run is 240 000 pieces, both firmware spinning in
  their final loops). Here the pieces cost about nothing in instructions and
  some 10% in wall time; a node whose every stop is expensive pays per piece.
- A sleeping node stretches its peers only with idle fast-forward on, and only
  until something can wake it: an edge a peer drives brings its horizon back
  to that edge plus one latency.
- A node with two cores (ESP32, ESP32-S3) or with a net pad on the per-cycle
  poll cannot stop at its own drive change, so it is still held to one latency
  past its own time.
- A node with push capture armed (every net node) does not use the CPU fast
  paths that skip the tap (the RISC-V spin and poll windows, the Cortex-M
  Thumb RAM chunks); that is the cost of stamping every pad write.
- An ESP32-C3 node booting through its ROM was not measured (the ROM image is
  not in this tree); its wide batches now end at its own net pad pushes
  rather than every latency, which is the case the batch stop exists for.

Machines on no net run unchanged and are not held to any net's latency.

## Which pads can be on a net

A net member's GPIO model has to implement the pin interface
([`docs/architecture/pins.md`](../architecture/pins.md)): report what it
drives and accept an external level. Every GPIO model in tree does:

| GPIO model | Capture | Notes |
|------------|---------|-------|
| `GpioPort`, every register family (STM32 `v2` and `f1`, nRF52/54, Kinetis, EFR32 series 2, SAM, RA, i.MX RT) | push: the port reports its own edges, idle fast-forward stays on | exercised end to end on STM32 `v2` (G0B1, F401) and `f1` (F103). The EXTI raises edge interrupts for net edges on G0 and U5 (port from `EXTI_EXTICRx`), F1 (port from `AFIO_EXTICRx`), F4 and L0 (port from `SYSCFG_EXTICRx`). Internal pulls on the net, and read by an input nothing outside drives: STM32 `v2` `PUPDR`, STM32 `f1` input-with-pull (`CNF = 10`, `ODR` picks the rail), nRF52 `PIN_CNF.PULL`, EFR32 `INPUTPULL`, SAM `PINCFG.PULLEN`, and, from the block that keeps them outside the GPIO port (the port's `pad_control:` in the chip yaml), Kinetis `PORTx_PCRn.PE/PS`, RA `PmnPFS.PCR` and i.MX RT IOMUXC `SW_PAD_CTL` `PKE/PUE/PUS`. |
| `GpioPort` pads routed to a peripheral (AF) | as above | the peripheral says what its output stage does (driving, released, input) and reads the level the net delivers; see [Buses over nets](#buses-over-nets). STM32 SPI, modern I²C and legacy (F1/F2/F4) I²C; the RP2040 SIO does the same for SPI and I²C pads (`FUNCSEL`). |
| `avr_gpio` (ATmega328P `portb`/`portc`/`portd`) | push: the port reports its own edges, idle fast-forward stays on | INT0/INT1 and PCINT0..2 see external edges; a core parked in `SLEEP` is skipped until the next edge or Timer0 overflow. Exercised end to end on the Uno (`gpio-net-two-boards`). An input with its `PORTx` bit set is a pull-up on the net, unless firmware sets `MCUCR.PUD`, which takes every pull-up off. |
| ESP32 classic `gpio` (member `peripheral: gpio`, pins 0..31) | push | Drive: `GPIO_ENABLE` and `GPIO_OUT`; `GPIO_PINn.PAD_DRIVER` open drain drives only a 0 and, holding a 1, reads the wire on `GPIO_IN`. Interrupts: `GPIO_PINn.INT_TYPE` edge and level types latch `GPIO_STATUS` and raise matrix source 22 for the CPU whose INT_ENA bit is set. GPIO32..39 cannot join a net. The IO_MUX pulls are not modelled on this part. |
| ESP32-S3 `gpio` (`peripheral: gpio`, pins 0..31) | push | As classic; matrix source 16 (`GPIO_PCPU_INT`, INT_ENA bit 13). The IO_MUX `FUN_WPU` pull-up is on the net. GPIO32..48 cannot join a net. |
| ESP32-C3 / ESP32-C6 `gpio` (`peripheral: gpio`, pins 0..25) | push | As classic; matrix source 16 on the C3 and 30 on the C6. The IO_MUX `FUN_WPU` pull-up is on the net where the IO_MUX is wired. Edge types only (level types are not modelled on this block). Exercised end to end on the C6 (`env-esp32c6.yaml`). |
| Declarative `GPIO` descriptor (`ENABLE` / `OUT` / `IN` registers) | poll | Drive from `ENABLE` and `OUT`; no pulls. |
| RP2040 `sio` (`peripheral: sio`, pins 0..29 = GP0..GP29) | push | Drive: `GPIO_OE` and `GPIO_OUT` while IO_BANK0 selects SIO for the pad (or nothing yet); open drain is firmware toggling `GPIO_OE` with the latch at 0. Interrupts: IO_BANK0 `INTRn` / `PROC0_INTEn` edge and level bits raise `IO_IRQ_BANK0` (NVIC 13) from any change of `GPIO_IN`, the pad's own output included. Exercised end to end (`env-rp2040.yaml`). |

On the ESP32 family and the RP2040 the GPIO interrupt sees the pad as
`GPIO_IN` reports it. On the classic ESP32 and the RP2040 that includes the
pad's own output; on the C3/C6 and S3 `GPIO_IN` is the external level only,
so a pad does not interrupt on its own output there.

A peripheral that owns no pins, or a pad whose drive is unknown, is refused
when the world is built, naming the pad.

Limits worth knowing:

- On Kinetis, RA and i.MX RT the pull comes from the block the port names
  in `pad_control:` (`mkw41z4` `gpioc` → `portc`, `ra4m1` `port1`/`port3` →
  `pfs`, `imxrt1064` `gpio2` → `iomuxc`). A port declared without it has no
  pulls. i.MX RT pads must be contiguous in IOMUXC from the port's pin 0
  (true for GPIO1, GPIO2 and GPIO4, not GPIO3).
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

## Adding net support to a GPIO model

There is nothing net-specific to add: a GPIO model that implements
`pins::PinPort` is on nets, the logic analyzer, board buttons and EXTI at
once. The steps are in
[`docs/architecture/pins.md`](../architecture/pins.md#implementing-a-new-gpio-model);
for a net the parts that matter are:

1. `driver(pin)`: the pad's own output stage, decoded from the direction,
   output, open-drain and pull registers. Registers only. An open-drain
   output holding a 1 is `Out::Off`; an internal pull is `pull`, which the
   net counts as a weak source. A pad handed to a peripheral signal whose
   drive the model cannot see is `None`, and the world refuses it.
2. `set_external(pin, External::Level(l))`: the level the net settles on.
   Store it beside the registers (never in them, so `driver()` cannot see
   it), fold it into the input register firmware reads, latch whatever
   interrupt the block raises from that register, and return the input
   before and after. The bus fans the change out to EXTI and timer captures
   by the port's `PortId`.
3. Push capture, so a net does not force the node onto the per-cycle poll:
   keep the `PadWatch` `install_watch` hands you and bracket every register
   write and `set_external` with `pins::watch_begin(self)` /
   `pins::watch_end(self)`.
4. If peripheral lines can be routed to the pad, implement `join_wire`
   (mark the line as shared, `PadLines::mark_on_net`) and `routes_changed`.
5. Register the model in `crates/core/src/pins/conformance.rs`. The suite
   checks the driver never echoes the net's level, open drain, pulls, the
   input round trip, and that push capture equals the poll.

## Buses over nets

A hardware SPI or I²C peripheral whose pads are on nets talks to the other
chip bit by bit, at the pads. There is no byte-level shortcut between the
peripherals: the bits are the net's edges, so the net reports, a logic
analyzer on either board and the timing all show the real waveform, and the
bus behaves like wires. Runnable examples:
[`examples/gpio-net-buses`](../../examples/gpio-net-buses) (STM32G071 pairs,
an STM32F401 pair, STM32F103 / STM32F401 / STM32G071 mixed I²C, and an
RP2040 against a G071 on SPI and on I²C).

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

| Peripheral | Register file | Pads (chip yaml) | Roles |
|------------|---------------|------------------|-------|
| STM32 SPI | classic / FIFO (`CR1`/`CR2`/`SR`/`DR`) | G071 AF0 incl. NSS (`pad_map: stm32g0`); F4 AF5 (SCK/MISO/MOSI, no NSS) | master, slave |
| STM32 I²C, modern | L4/G0 `TIMINGR` (`ISR`/`ICR`) | G071 AF6 (`pad_map: stm32g0`); L4 AF4 | controller, target |
| STM32 I²C, legacy | F1/F2/F4 (`SR1`/`SR2`/`CCR`/`TRISE`) | F103 default mapping (PB6/PB7, PB10/PB11); F401 AF4 | master, slave |
| RP2040 SPI | PL022 (`SSPCR0`/`SSPCR1`/`SSPDR`/`SSPSR`/`SSPCPSR`) | IO_BANK0 `FUNCSEL` 1: SCK, TX, RX, CSn | master, slave |
| RP2040 I²C | DW_apb_i2c (`IC_CON`/`IC_TAR`/`IC_SAR`/`IC_DATA_CMD`/...) | IO_BANK0 `FUNCSEL` 3: SDA, SCL | master, slave |

**SPI** (STM32 classic/FIFO SPI): SCK, MOSI, MISO and NSS are push-pull
nets. A master (`MSTR=1`, `SPE=1`) drives SCK and MOSI and samples the level
the net delivers to its MISO pad at its own sampling edge; that is what lands
in `DR`. A slave (`MSTR=0`) has no clock of its own: it reacts to the SCK,
MOSI and NSS edges the net delivers, shifts MOSI in on its sampling edge and
puts its transmit word out on MISO on its shift edge. Any CPOL/CPHA, MSB or
LSB first, 8 or 16 bits (`DFF`): modes 0-3, LSB-first and 16-bit frames each
have an end-to-end test. A slave sets RXNE with the SPI interrupt when
`RXNEIE` is set, and OVR when a frame arrives before `DR` was read; `TXEIE`
interrupts whenever the transmit buffer is empty (enabling it with TXE set
interrupts at once), so a slave can feed every answer from its TXE interrupt.
A `DR` write fills the slave's transmit buffer even while `SPE=0`. A frame
that starts with nothing written to `DR` is an underrun: the shift register
goes out as it is, which is the previous frame received (0 after reset),
and `Spi::net_slave_underruns()` counts it (logged once). It drives MISO only
while selected: `SSM=1, SSI=0`, or `SSM=0` with its NSS pad low. A master
with `SSOE=1` drives NSS low while enabled. A disabled SPI drives nothing. A
word store to `DR` is one frame (bits 31:16 are not part of the register).

**I²C, modern** (STM32 L4/G0 `TIMINGR` register file): SCL and SDA are
open-drain nets; put `pull: up` on them. The controller generates START, the
address, data, ACK/NACK, repeated START and STOP with `SCLL`/`SCLH`/`SDADEL`
timing, and counts the SCL high period from when it *sees* SCL high, so a
target holding SCL low stretches the clock. A NACK sets `NACKF` and sends
STOP; `AUTOEND` sends STOP after `NBYTES`, otherwise `TC` holds SCL low for a
repeated START. With `RELOAD`, `TCR` holds SCL low after `NBYTES` until
firmware writes the next `NBYTES` (transfers above 255 bytes; the last byte
of a chunk is ACKed). `ADD10` sends the 10-bit sequence: write header and low
byte, and for a read a repeated START with the read header (only the read
header with `HEAD10R`). A released SDA read back low is arbitration lost
(`ARLO`). A target watches START/STOP and its address on the wire: `OAR1`
(7-bit, or 10-bit with `OA1MODE`), `OAR2` with `OA2MSK`, and the general call
with `GCEN`; it ACKs those only, sets `ADDR`, `DIR` and `ADDCODE` (`11110xx`
for a 10-bit match), and stretches SCL until firmware clears `ADDR`, reads
`RXDR` or writes `TXDR`. With `NOSTRETCH=1` it does not stretch: an empty
`TXDR` goes out as it is and a byte arriving while `RXDR` is full is NACKed
and lost, both with `OVR`. A transfer to an address nobody owns is the
pull-up reading 1 in the ACK slot.

**I²C, legacy** (STM32 F1/F2/F4 `CR1`/`CR2`/`OAR1`/`OAR2`/`DR`/`SR1`/`SR2`/
`CCR`/`TRISE`): the same open-drain wire, driven by the register sequences of
RM0008/RM0368. Master: `START` sets SB/MSL/BUSY once START is on the wire;
the `DR` write after it is the address byte, whose ACK sets `ADDR` (`TRA`
from R/W) and whose NACK sets `AF`. A transmitter takes `DR` after `ADDR` is
cleared (SR1 then SR2 read) and holds SCL low with `BTF` while `DR` is empty;
a receiver ACKs by `CR1.ACK` (with `POS`, the value in force one byte
earlier) and holds SCL low with `BTF` while a byte waits behind a full `DR`.
`STOP` goes out at once from a held bus, otherwise after the byte in flight.
Slave: it ACKs `OAR1` (7-bit), `OAR2` with `ENDUAL` and the general call
with `ENGC` while `CR1.ACK` is set, stretches SCL while `ADDR` is up and
while `DR` is full (receiver) or empty (transmitter), sets `AF` when the
master NACKs what it sent and `STOPF` when a write ends with STOP. A byte
loaded into `DR` that never went out stays there and is the first one the
next read gets. `NOSTRETCH` skips the stretch after the address. Timing is
`CCR`'s (standard mode, fast mode, `DUTY`) in PCLK1 periods taken as core
cycles. Event and error interrupts (`ITEVTEN`/`ITBUFEN`/`ITERREN`, separate
error vector where the chip yaml gives `irq_error`) are raised when a flag
is set and when an enable is written while its flag is up.

**SPI, RP2040** (PL022, Motorola frame format): SCK, TX, RX and CSn. A
master (`SSPCR1.MS = 0`) clocks the words queued in its 8-deep TX FIFO at
`CPSDVSR × (1 + SCR)` core cycles per bit with SCK idling at `SPO` and the
edges `SPH` gives; TX is MOSI and RX (sampled at each sampling edge) is MISO,
and each word lands in the 8-deep RX FIFO. CSn is low for a frame; with
`SPH = 0` it goes high for one SCK period between back-to-back frames, with
`SPH = 1` it stays low until the FIFO runs dry. A slave (`MS = 1`) reacts to
the CSn, SCK and RX (MOSI) edges the net delivers, drives TX (MISO) only
while CSn is low and `SOD` is clear, and answers from its TX FIFO; a frame
with the TX FIFO empty sends zeros and is counted
(`Rp2040Spi::net_underruns`). `SSPSR` follows the FIFOs and the shift engine.
Tested against an STM32G071 both ways round (RP2040 master in mode 0 with
hardware CSn, RP2040 slave in mode 3). Not on a net: the PL022 interrupt
registers, DMA, TI/Microwire formats and loopback.

**I²C, RP2040** (DW_apb_i2c): the same open-drain wire. A master
(`IC_CON.MASTER_MODE`) works through its 16-deep command FIFO
(`IC_DATA_CMD`: a byte to write or a read, with `STOP` and `RESTART`):
START and the `IC_TAR` 7-bit address when the FIFO has a command, a
repeated START with the address again on a change of direction or
`RESTART`, STOP after a command that asks for it, and SCL held low while the
FIFO is empty without one (the RP2040's `IC_EMPTYFIFO_HOLD_MASTER_EN`). A
read is ACKed unless it carries `STOP`. A NACKed address or byte aborts with
`TX_ABRT` (`ABRT_7B_ADDR_NOACK` / `ABRT_TXDATA_NOACK`), flushes the FIFO and
sends STOP. Timing is `IC_SS/FS_SCL_LCNT`/`HCNT` with `IC_SDA_HOLD`. A slave
(`MASTER_MODE = 0`, `IC_SLAVE_DISABLE = 0`) ACKs `IC_SAR`, fills the RX FIFO
(`FIRST_DATA_BYTE` on the first byte after the address), answers a read from
its TX FIFO and otherwise holds SCL low with `RD_REQ` until firmware writes
`IC_DATA_CMD`; the master's NACK ends a read with `RX_DONE`. `STOP_DET`,
`START_DET`, `RX_FULL`/`IC_RX_TL`, `TX_EMPTY`/`IC_TX_TL` and the `IC_CLR_*`
registers follow the DW names; the tests poll `IC_RAW_INTR_STAT`. Tested
against an STM32G071 both ways round, including a slave that answers too
slowly for the clock. Not on a net: 10-bit addressing, the general call,
SMBus, DMA and `RX_FIFO_FULL_HLD_CTRL` (a full RX FIFO drops the byte with
`RX_OVER`).

Clock stretching is tested on every target: an STM32G071 (modern), an
STM32F401 (legacy) and an RP2040 slave whose firmware answers several SCL low
periods late each hold the controller, and the bytes and clock count come
out the same. A target that ends a stretch with a new bit on SDA releases
SCL a data setup time later (`SDADEL`-style, one quarter of its own SCL low
period on the legacy and RP2040 engines), so the change is never read as
START or STOP.

The legacy and modern controllers meet only on the wire, so an F103 target
answers a G071 controller as it answers an F401 one (tested both ways round
between F103 and F401, and G071 → F103).

Everything is event-scheduled: a master's bits run at their exact cycles, a
slave or target runs when the net delivers an edge. The one poll is a chip
holding SCL low until firmware does something with a register *read*, which
cannot wake the model: reading `RXDR` (modern), or clearing `ADDR` and
reading `DR` (legacy). It looks every quarter SCL low period, and at every
register write.

Two physical limits come with the wires:

- An answer crosses the wire twice. MISO answers a clock edge only after the
  edge reached the slave and the answer came back, so two `latency_ns` must
  fit in half an SCK period; slow SCK (`CR1.BR`) or shorten the latency when
  they do not. The same holds for I²C data and ACKs inside the SCL low period.
- Give SCL and SDA (and SCK and MOSI) the same `latency_ns`. The receiver
  tells data from START/STOP by the order the two wires change in.

The results do not depend on node order or on the world's round size; each
example's tests run every exchange three ways (as written, nodes swapped,
37 ns rounds) and compare what every pad saw, cycle by cycle.

Not modelled on a net yet: other SPI/I²C families (nRF52 SPIM/SPIS and
TWIM/TWIS, ESP32 SPI/I²C, the H5 SPI v3), the hardware NSS pad of the F4 SPI, legacy-I²C 10-bit addressing,
SMBus/PEC, modern-I²C slave byte control (`SBC`), the I²C filters and
timeouts, SPI CRC, TI mode and DMA on a slave. Such a peripheral on a net
keeps its usual behaviour and drives its lines push-pull as before.

## Reports

`World::gpio_net_reports()` (and `result.json` `gpio_nets` from
`labwired test`, and `WasmWorld.gpio_net_report()` in the browser) lists each
net: level, edge count, contention and floating counters, each member's drive,
and the diagnostics with times in picoseconds.
