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
| `GpioPort` pads routed to a peripheral (AF) | as above | the peripheral says what its output stage does (driving, released, input) and reads the level the net delivers; see [Buses over nets](#buses-over-nets). STM32 SPI, modern I²C and legacy (F1/F2/F4) I²C; the RP2040 SIO does the same for SPI and I²C pads (`FUNCSEL`); the nRF52 `PSEL` pin selects (SPIM/SPIS/TWIM/TWIS) likewise. |
| `avr_gpio` (ATmega328P `portb`/`portc`/`portd`) | push: the port reports its own edges, idle fast-forward stays on | INT0/INT1 and PCINT0..2 see external edges; a core parked in `SLEEP` is skipped until the next edge or Timer0 overflow. Exercised end to end on the Uno (`gpio-net-two-boards`). An input with its `PORTx` bit set is a pull-up on the net, unless firmware sets `MCUCR.PUD`, which takes every pull-up off. |
| ESP32 classic `gpio` (member `peripheral: gpio`, pins 0..39) | push | Drive: `GPIO_ENABLE` and `GPIO_OUT` (GPIO0..31), `GPIO_ENABLE1` and `GPIO_OUT1` at bit `pin - 32` (GPIO32..39); `GPIO_PINn.PAD_DRIVER` open drain drives only a 0 and, holding a 1, reads the wire on `GPIO_IN` / `GPIO_IN1`. GPIO34..39 are input-only, as on silicon: their `ENABLE1` bits store but never drive, and the matrix never routes an output to them; they take input and feed matrix input signals. Interrupts: `GPIO_PINn.INT_TYPE` edge and level types latch `GPIO_STATUS` / `GPIO_STATUS1` and raise matrix source 22 for the CPU whose INT_ENA bit is set (`GPIO_ACPU_INT(1)`, `GPIO_PCPU_INT(1)`). The IO_MUX pulls are not modelled on this part; the RTC-capable pads (GPIO0, 2, 4, 12..15, 25..27, 32, 33) take their pull from RTC_IO `RUE` / `RDE` in either domain (errata 3.6), reset values included (GPIO0, 14, 15 pulled up, GPIO2, 4, 12, 13 down). A pad whose RTC_IO `MUX_SEL` is set is driven by the RTC GPIO instead; see [RTC IO](#rtc-io-esp32-and-esp32-s3). Pads the GPIO matrix routes to VSPI, HSPI, I2C0 or I2C1 (`GPIO_FUNCn_OUT_SEL_CFG` out, `GPIO_FUNCn_IN_SEL_CFG` in, both indexed by the pad number in either bank) carry those peripherals' lines; see [Buses over nets](#buses-over-nets). |
| ESP32-S3 `gpio` (`peripheral: gpio`, pins 0..48) | push | As classic, both banks (`GPIO_OUT1` / `GPIO_ENABLE1` / `GPIO_IN1` / `GPIO_STATUS1` at bit `pin - 32`); matrix source 16 (`GPIO_PCPU_INT` / `GPIO_PCPU_INT1`, INT_ENA bit 13). The IO_MUX `FUN_WPU` pull-up is on the net. GPIO49..53 have registers but no pad, and cannot join a net. Pads the GPIO matrix routes to GP-SPI2, GP-SPI3, I2C0 or I2C1 carry those peripherals' lines. GPIO0..21 with RTC_IO `MUX_SEL` set are driven by the RTC GPIO; see [RTC IO](#rtc-io-esp32-and-esp32-s3). |
| ESP32-C3 / ESP32-C6 `gpio` (`peripheral: gpio`, pins 0..25) | push | As classic; matrix source 16 on the C3 and 30 on the C6. The IO_MUX `FUN_WPU` pull-up is on the net where the IO_MUX is wired. Edge types only (level types are not modelled on this block). Exercised end to end on the C6 (`env-esp32c6.yaml`). Pads the GPIO matrix routes to GP-SPI2 or I2C0 (`FUNCn_OUT_SEL_CFG` out, `FUNCn_IN_SEL_CFG` in, each chip's own signal numbers) carry those peripherals' lines; see [Buses over nets](#buses-over-nets). |
| Declarative `GPIO` descriptor (`ENABLE` / `OUT` / `IN` registers) | poll | Drive from `ENABLE` and `OUT`; no pulls. |
| RP2040 `sio` (`peripheral: sio`, pins 0..29 = GP0..GP29) | push | Drive: `GPIO_OE` and `GPIO_OUT` while IO_BANK0 selects SIO for the pad (or nothing yet); open drain is firmware toggling `GPIO_OE` with the latch at 0. Interrupts: IO_BANK0 `INTRn` / `PROC0_INTEn` edge and level bits raise `IO_IRQ_BANK0` (NVIC 13) from any change of `GPIO_IN`, the pad's own output included. Exercised end to end (`env-rp2040.yaml`). |

On the ESP32 family and the RP2040 the GPIO interrupt sees the pad as
`GPIO_IN` reports it. On the classic ESP32 and the RP2040 that includes the
pad's own output; on the C3/C6 and S3 `GPIO_IN` is the external level only,
so a pad does not interrupt on its own output there.

A peripheral that owns no pins, or a pad whose drive is unknown, is refused
when the world is built, naming the pad.

### RTC IO (ESP32 and ESP32-S3)

The RTC-capable pads (classic GPIO0, 2, 4, 12..15, 25..27, 32..39 =
RTC_GPIO0..17; S3 GPIO0..21 = RTC_GPIO0..21) have a second owner, the RTC
IO MUX (`rtcio` at 0x3FF4_8400 on the classic part, `rtc_io` at 0x6000_8400
on the S3). Its pad register's `MUX_SEL` picks the side: with it set the pad
is driven by `RTC_GPIO_OUT` / `RTC_GPIO_ENABLE` (and their `_W1TS` /
`_W1TC`), `RTC_GPIO_PINn.PAD_DRIVER` makes it open drain, and `RUE` / `RDE`
pull it; `GPIO_OUT`, `GPIO_ENABLE` and the matrix no longer reach it (a
routed peripheral line goes dark until `MUX_SEL` clears). The net member is
still the GPIO port (`peripheral: gpio`, the GPIO number). Mind the
numbering on the classic part: RTC GPIO numbers are not GPIO numbers
(GPIO0 is RTC_GPIO11, GPIO25 RTC_GPIO6, GPIO32 RTC_GPIO9), and RTC GPIO n
sits at bit `14 + n` of the RTC GPIO registers (`10 + n` on the S3).

The input side follows the TRM: the pad's input reaches both the RTC GPIO
(`RTC_GPIO_IN`) and the digital side (`GPIO_IN`, matrix inputs), through
the input enable `MUX_SEL` selects (`FUN_IE` in the pad register: with it
clear both read 0). `RTC_GPIO_PINn.INT_TYPE` latches `RTC_GPIO_STATUS` (edge
and level types; `_W1TC` clears). That latch reaches no CPU interrupt on
either part (`RTC_CNTL_INT_ENA` has no RTC GPIO bit; RTC_CORE, source 46 /
39, is `RTC_CNTL_INT_ST`), so firmware polls it. On the classic part a pad
register's `HOLD` freezes the RTC output stage; the S3 keeps its holds in
RTC_CNTL, which is not modelled, nor is a hold on a digital-domain pad.
Every `MUX_SEL` resets to 0. Firmware that uses an RTC-capable pad
digitally clears it first, as ESP-IDF's `gpio_config()` does
(`rtc_gpio_deinit`), and the example's Xtensa firmware does too
(`RTC_DIGITAL` in `src/esp32x.inc`). `env-rtc-gpio-esp32-esp32s3.yaml`
runs RTC GPIO across three nets between a classic ESP32 and an ESP32-S3
(an open-drain RTC output released to the peer's `RUE`, edges counted from
`RTC_GPIO_STATUS`) and hands GPIO27 from the RTC domain back to the digital
output it set up from the start: the net sees the digital 1 only once
`MUX_SEL` clears.

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
STM32F401, STM32L476 and STM32G474 pairs on the hardware NSS pads,
STM32F103 / STM32F401 / STM32G071 mixed I²C, and an RP2040, an nRF52840
(SPIM0 and SPIM3 with its hardware CSN and D/CX), an ESP32-C3, an ESP32-C6,
an ESP32-S3 and a classic ESP32 against a G071 on SPI and on I²C).

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
| STM32 SPI | classic F1/F4/L0 (`CR1.DFF`) / FIFO G0/L4/G4/WB/F7 (`CR2.DS`, `FRXTH`, `FRLVL`/`FTLVL`); DMA requests (`TXDMAEN`/`RXDMAEN`, `LDMA_TX`/`LDMA_RX`, [SPI DMA](#spi-dma)) | NSS included on every table: G071 AF0 (`pad_map: stm32g0`); F4/F7 AF5 (PA4/PA15, PB9/PB12); L476 AF5/AF6 (SPI1 PA4/PA15/PE12, SPI2 PB9/PB12/PD0, SPI3 PA4/PA15 on AF6); G474 (`pad_map: stm32g4`, SPI1 PA4/PA15/PG5, SPI2 PB12/PF0 and PD15 on AF6, SPI3 PA4/PA15 on AF6); WB55 (`pad_map: stm32wb`, SPI1 PA4/PA15/PB2, SPI2 PB9/PB12/PD0); L073 (`pad_map: stm32l0`, SPI1 AF0 PA4/PA15, PE12 AF2; SPI2 PB12 AF0, PB9 AF5, PD0 AF1) | master, slave |
| STM32 I²C, modern | L4/G0 `TIMINGR` (`ISR`/`ICR`) | G071 AF6 (`pad_map: stm32g0`); L4 AF4 | controller, target |
| STM32 I²C, legacy | F1/F2/F4 (`SR1`/`SR2`/`CCR`/`TRISE`) | F103 default mapping (PB6/PB7, PB10/PB11); F401 AF4 | master, slave |
| RP2040 SPI | PL022 (`SSPCR0`/`SSPCR1`/`SSPDR`/`SSPSR`/`SSPCPSR`) | IO_BANK0 `FUNCSEL` 1: SCK, TX, RX, CSn | master, slave |
| RP2040 I²C | DW_apb_i2c (`IC_CON`/`IC_TAR`/`IC_SAR`/`IC_DATA_CMD`/...) | IO_BANK0 `FUNCSEL` 3: SDA, SCL | master, slave |
| nRF52 SPI | SPIM / SPIS, EasyDMA (`TXD`/`RXD` `PTR`/`MAXCNT`/`AMOUNT`) | any pad, by `PSEL.SCK`/`MOSI`/`MISO` (and SPIS `PSEL.CSN`; SPIM3 `PSEL.CSN` and `PSELDCX`) | master (`ENABLE = 7`), slave (`ENABLE = 2`) |
| nRF52 I²C | TWIM / TWIS, EasyDMA | any pad, by `PSEL.SCL`/`SDA` | controller (`ENABLE = 6`), target (`ENABLE = 9`) |
| ESP32-C3 / ESP32-C6 GP-SPI2 | Espressif GP-SPI (`SPI_CMD`/`SPI_USER`/`SPI_MS_DLEN`/`W0..W15`/`SPI_SLAVE`), or by GDMA (`SPI_DMA_CONF`, [SPI DMA](#spi-dma)) | GPIO matrix: `FSPICLK`, `FSPID`, `FSPIQ`, `FSPICS0` (63/65/64/68 on both), any pad | master, slave |
| ESP32-C3 / ESP32-C6 I²C0 | Espressif command list (`CTR`/`COMD0..7`/`DATA` FIFOs/`SLAVE_ADDR`) | GPIO matrix: `I2CEXT0_SCL`/`SDA` (53/54 on the C3, 45/46 on the C6), any pad, out and back in | master, slave |
| ESP32-S3 GP-SPI2 / GP-SPI3 | Espressif GP-SPI, as the C3 | GPIO matrix: SPI2 `FSPICLK`, `FSPID`, `FSPIQ`, `FSPICS0` (101/103/102/110, `spi2`), SPI3 `SPI3_CLK`, `SPI3_D`, `SPI3_Q`, `SPI3_CS0` (66/68/67/71, `spi3`), pads 0..48 | master, slave |
| ESP32-S3 I²C0 / I²C1 | Espressif command list, as the C3 | GPIO matrix: `I2CEXT0_SCL`/`SDA` (89/90, `i2c0`), `I2CEXT1_SCL`/`SDA` (91/92, `i2c1`), pads 0..48, out and back in | master, slave |
| ESP32 (classic) VSPI / HSPI | classic SPI (`SPI_CMD.USR` bit 18, `SPI_USER`, `SPI_MOSI_DLEN`/`MISO_DLEN`, `SPI_PIN`, `SPI_SLAVE`, `W0..W15` at 0x80) | GPIO matrix: VSPI `VSPICLK`/`VSPID`/`VSPIQ`/`VSPICS0` (63/65/64/68, `spi3`), HSPI `HSPICLK`/`HSPID`/`HSPIQ`/`HSPICS0` (8/10/9/11, `spi2`), outputs on pads 0..33, inputs on pads 0..39 | master, slave |
| ESP32 (classic) I²C0 / I²C1 | Espressif command list, classic opcodes (`COMD0..15`) | GPIO matrix: `I2CEXT0_SCL`/`SDA` (29/30, `i2c0`), `I2CEXT1_SCL`/`SDA` (95/96, `i2c1` at 0x3FF6_7000), pads 0..33 out and back in (0..39 in) | master, slave (no stretching) |

**SPI** (STM32 classic/FIFO SPI): SCK, MOSI, MISO and NSS are push-pull
nets. A master (`MSTR=1`, `SPE=1`) drives SCK and MOSI and samples the level
the net delivers to its MISO pad at its own sampling edge; that is what lands
in `DR`. A slave (`MSTR=0`) has no clock of its own: it reacts to the SCK,
MOSI and NSS edges the net delivers, shifts MOSI in on its sampling edge and
puts its transmit word out on MISO on its shift edge. Any CPOL/CPHA, MSB or
LSB first; 8 or 16 bits (`CR1.DFF`) on the classic F1/F4/L0 SPI, 4 to 16
bits (`CR2.DS`) on the FIFO-type G0/L4/G4/WB/F7 SPI: modes 0-3, LSB-first,
16-bit frames on an F401 pair, and 4-, 8-, 12- and 16-bit frames on a G071
pair (each chip as master) each have an end-to-end test. A slave sets RXNE
with the SPI interrupt when `RXNEIE` is set; `TXEIE` interrupts whenever TXE
is set (enabling it with TXE set interrupts at once), so a slave can feed
every answer from its TXE interrupt. A `DR` write fills the slave's transmit
buffer even while `SPE=0`. A frame that starts with nothing written to `DR`
is an underrun: the shift register goes out as it is, which is the previous
frame received (0 after reset), and `Spi::net_slave_underruns()` counts it
(logged once). It drives MISO only while selected: `SSM=1, SSI=0`, or `SSM=0`
with its NSS pad low. A master with `SSOE=1` drives NSS low while enabled. A
disabled SPI drives nothing. A word store to `DR` is one frame (bits 31:16
are not part of the register).

The two register files differ in their buffers, as on silicon. Classic: one
receive buffer, RXNE on every frame, OVR when a frame arrives before `DR`
was read, one transmit buffer. FIFO-type (RM0444 §35.5.9): 32-bit RX and TX
FIFOs, a frame of up to 8 bits taking one byte and a wider one two; RXNE
when the RX FIFO holds `CR2.FRXTH`'s threshold (one byte with `FRXTH=1`,
two with the reset `FRXTH=0` — so 8-bit firmware that never sets `FRXTH`
waits for a second frame, as it does on a real G0); TXE while the TX FIFO is
at most half full; `SR.FRLVL`/`FTLVL` report both levels; OVR when a frame
arrives to a full RX FIFO. `DR` accesses pack: an 8-bit access moves one
byte, a 16-bit one two (two frames at `DS` ≤ 8). A FIFO master with
`NSSP=1`, `SSOE=1` and `CPHA=0` pulses NSS high for one SCK period between
back-to-back frames and leaves it high after the last. The tables come from
ST's open pin data (the CubeMX source of each datasheet's AF table); the G4,
WB and L0 have their own (`pad_map:`) because they disagree with the L4 or
F4 one on some pad (PE12-PE15/AF5 are SPI4 on a G4; the L0's SPI1 is on
AF0). Tested: an L476 pair and a G474 pair, master `SSOE=1` / slave `SSM=0`
on PA4, with and without `NSSP` (every frame its own NSS low period when the
master polls between frames), and a slave whose NSS net is removed, which is
never selected.

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

**nRF52** (nRF52840 instance 0 at 0x40003000, the window `ENABLE` turns
into SPIM, SPIS, TWIM or TWIS; also the standalone TWIM1 and SPIM2): the
pads are whichever ones the `PSEL` registers name, so a peripheral is on a
net when the pad its pin select names is (and it is enabled). A write that
moves a `PSEL` or `ENABLE` is bracketed like an RP2040 `FUNCSEL` write: the
port reports what moved and the net's level reaches the newly routed line.
The window is one register file: what firmware programs before `ENABLE`
(buffers, `CONFIG`, `SHORTS`, `INTEN`) reaches every personality. EasyDMA
reads a TX buffer when its transfer is armed (`TASKS_START`, `STARTTX`,
`TASKS_RELEASE`, `PREPARETX`) and writes the RX buffer before the event that
ends it; the events raise the interrupt where `INTEN` asks for it.

- *SPIM*: `TASKS_START` clocks `max(TXD.MAXCNT, RXD.MAXCNT)` bytes back to
  back at `FREQUENCY` in the `CONFIG` shape (`ORC` past the TX buffer),
  sampling MISO at the net's level; `EVENTS_STARTED`, then `ENDTX`,
  `ENDRX`, `END` with the amounts. `TASKS_STOP` ends after the byte on the
  wire (`STOPPED`). On SPIM0..2 chip select is a GPIO. SPIM3 (0x4002F000,
  `spi3` on the nRF52833/nRF52840) drives its own: with `PSEL.CSN`
  connected, CSN sits at its inactive level while enabled (`CSNPOL` 0:
  active low), is asserted once the TX buffer is read, and the first byte
  starts `IFTIMING.CSNDUR` 64 MHz cycles later (first SCK edge `CSNDUR` +
  half an SCK period after the CSN edge); after the last SCK edge CSN is
  held `CSNDUR` more, released, and only then `END` fires. The next
  assertion waits until CSN has been inactive for `CSNDUR`. With `PSELDCX`
  connected D/CX idles high and is low for the first `DCXCNT` bytes (all of
  them at 0xF): it falls with CSN, rises at the last SCK edge of the last
  command byte, and is high again when CSN is released. `IFTIMING.RXDELAY`
  is stored, not modelled.
- *SPIS*: the semaphore decides. `TASKS_RELEASE` gives the slave the
  buffers; CSN low then starts a granted transaction (`SEMSTAT = 2`) that
  shifts out the TX buffer, then `ORC` (`STATUS.OVERREAD`), and fills the RX
  buffer up to `MAXCNT` (`STATUS.OVERFLOW` past it). With the CPU holding
  the semaphore it sends `DEF`. CSN high: `ENDRX`, `END`, the amounts, and
  the semaphore back to the CPU with `ACQUIRED` when it asked
  (`TASKS_ACQUIRE`) or `SHORTS.END_ACQUIRE` is set. MISO is driven only
  while CSN is low.
- *TWIM*: `STARTTX`/`STARTRX` send START (repeated START from a held bus)
  and `ADDRESS`; `LASTTX`/`LASTRX` fire at the start of the last byte with
  their shortcuts (`LASTTX_STARTRX`, `_SUSPEND`, `_STOP`, `LASTRX_STARTTX`,
  `_SUSPEND`, `_STOP`). A received byte is NACKed when STOP or a change of
  direction follows. A NACK sets `ERRORSRC.ANACK`/`DNACK` with `ERROR` and
  holds the bus until `TASKS_STOP` (nrfx's `RESUME` + `STOP`). With nothing
  left to do the TWIM holds SCL low. Timing: `FREQUENCY` (K100/K250/K400 of
  64 MHz), SCL low and high half a bit each.
- *TWIS*: ACKs `ADDRESS[0]`/`[1]` as `CONFIG` enables them (`MATCH`),
  fires `WRITE`/`READ`, and stretches SCL until firmware prepares a buffer
  (`TASKS_PREPARERX`/`PREPARETX`, or `RESUME` after `WRITE_SUSPEND`/
  `READ_SUSPEND`); `RXSTARTED`/`TXSTARTED` follow. Past `RXD.MAXCNT` a byte
  is NACKed with `ERRORSRC.OVERFLOW | DNACK`; past the TX buffer `ORC` goes
  out (`OVERREAD`). Each phase uses up its prepare; STOP fires `STOPPED`
  with the amounts.

Tested against an STM32G071 both ways round on each bus (SPIM master /
SPIS slave in mode 0, TWIM controller / TWIS target with the `env-i2c.yaml`
register-file exchange, the NACK of an absent address, a slow G071 target
holding the TWIM and a slow TWIS holding the G071), and SPIM3 on its
hardware CSN against the G071's hardware NSS (`env-spi-nrf52-spim3.yaml`:
the CSN-to-SCK and SCK-to-CSN times, D/CX around a one-byte command, and an
active-high `CSNPOL` that deselects the G071 for the transfer). Not on a
net: SPIM3's CSN/D/CX for an attached device model (off a net the bytes
cross inside one call; the nRF54L map does select such a device), the legacy
SPI/TWI (`ENABLE = 1`/`5`), the TWIM `ERRORSRC.OVERRUN`, the nRF5340 and
nRF54 SPIM/TWIM, and the SPIS/TWIS on a window other than a serial
instance (the nRF52840 chip yaml has them only at 0x40003000).

**SPI, ESP32-C3 / ESP32-C6** (GP-SPI2; the C6 runs the same model): each
line reaches its pad through the GPIO matrix, out (`FUNCn_OUT_SEL_CFG` with
the pad's output stage on) and in (`FUNCn_IN_SEL_CFG`, `SEL` set). A master
(`SPI_SLAVE.MODE = 0`) runs the transaction `SPI_CMD.USR` starts on the
wire, bit by bit at the `SPI_CLOCK` rate: CS0 low (unless `CS0_DIS`) for
`CS_SETUP`, the command, address and dummy phases when enabled, then the
data, full duplex (`DOUTDIN`: `MS_DLEN + 1` bits out of `W0..W15` while
MISO is sampled) or half duplex (a write phase, then a read phase); the bits
read land in `W0..`, `USR` clears and `SPI_TRANS_DONE` latches after
`CS_HOLD`. CPOL is `MISC.CK_IDLE_EDGE`, CPHA `USER.CK_OUT_EDGE` XOR CPOL (how
esp-idf and esp-hal set the four modes), bit order `CTRL.WR/RD_BIT_ORDER`.
A slave (`MODE = 1`, CPU-controlled) answers from `W0..W15` as they are when
CS0 falls, drives MISO only while selected, and when CS0 rises writes the
bits it sampled to `W0..`, their count to `SLAVE1.DATA_BITLEN`, and latches
`SPI_TRANS_DONE` (CPOL `CK_IDLE_EDGE`, CPHA `SLAVE.CLK_MODE_13`, as
`spi_ll_slave_set_mode` sets them). Tested against an STM32G071 both ways
round on the C3 and the C6 (register-level firmware), and with esp-hal
1.1's blocking master driver (hardware CS0) on the C3. Both roles also run
by GDMA (see [SPI DMA](#spi-dma)). Not on a net: dual/quad modes, segmented
transfers and the master's CS polarity bits.

**I²C, ESP32-C3 / ESP32-C6** (I2C0): the same open-drain wire engine as the
legacy STM32 and RP2040 I²C, driven by the command list. A master
(`CTR.MS_MODE`) runs `COMD0..` on `TRANS_START`: `RSTART`, `WRITE n` from
the TX FIFO (the address is its first byte, as esp-idf and esp-hal queue it)
with each ACK slot compared to `ack_exp` when `ack_check_en` is set,
`READ n` into the RX FIFO answered with `ack_value`, `STOP`
(`TRANS_COMPLETE`) and `END` (`END_DETECT`, SCL held low until the next
`TRANS_START`); each command sets `command_done`, `SR.RESP_REC` records the
ACKs. A mismatched ACK latches `NACK` and the master sends STOP (on silicon
the driver resets the controller after a NACK, which also frees the bus).
Timing is `CLK_CONF` with `SCL_LOW/HIGH_PERIOD` and `SDA_HOLD`. A slave
(`MS_MODE = 0`) ACKs its 7-bit `SLAVE_ADDR`, fills the RX FIFO
(`RXFIFO_WM`), answers reads from the TX FIFO and, with it empty and
`SLAVE_SCL_STRETCH_EN` set, holds SCL low with `SLAVE_STRETCH` until
firmware writes `SLAVE_SCL_STRETCH_CLR`; STOP after it was addressed latches
`TRANS_COMPLETE`. Tested against an STM32G071 both ways round on the C3 and
the C6, including a NACK and a slow target on either side that stretches
SCL, and with esp-hal 1.1's blocking master driver on the C3. Not on a net:
10-bit addressing, the general call, `SLAVE_BYTE_ACK_CTL`, the FIFO-address
and non-FIFO modes, filters and timeouts.

**ESP32-S3 and classic ESP32.** The S3 GP-SPI2 and I2C0 are the C3's
register files and run the same two engines, with the S3's matrix numbers
(`FUNCn_IN_SEL_CFG`: pad [5:0], `SEL` bit 7) and its I²C timing
(`CLK_CONF` source clock, 240 MHz core). The classic part's SPI is the older
IP: the same fields at other offsets and bits (`SPI_CMD.USR` 18, `SPI_USER`
`CS_HOLD` 4 / `CS_SETUP` 5 / `CK_OUT_EDGE` 7, `SPI_CTRL2` setup and hold
times, `SPI_MOSI_DLEN` / `SPI_MISO_DLEN`, `SPI_PIN` `CK_IDLE_EDGE` /
`CS0_DIS` / `CS_KEEP_ACTIVE`, `W0` at 0x80); it answers the GP-SPI engine
through a translated register view. A classic slave is `SPI_SLAVE.SLAVE_MODE`
(bit 30) with CPHA from `SPI_USER.CK_I_EDGE` as esp-idf's
`spi_ll_slave_set_mode` sets it (mode 0: `CK_I_EDGE` = 1); its bit count
lands in `SPI_SLV_RD_BIT`, and master and slave both report done in
`SPI_SLAVE.TRANS_DONE` (bit 4). VSPI (`spi3`, 0x3FF6_5000) and HSPI
(`spi2`, 0x3FF6_4000, now on the bus) each have their own matrix signals.
The S3's second GP-SPI (`spi3`, 0x6002_5000) and I2C1 (`i2c1`, 0x6002_7000)
are the same models as SPI2 / I2C0 with their own matrix signals, and so is
the classic I2C1 (`i2c1`, 0x3FF6_7000, interrupt source 50, AHB FIFO alias
0x6002_701C), which is now on the classic bus.
The classic I2C0 and I2C1 run the command-list engine with the classic opcodes
(RSTART 0, WRITE 1, READ 2, STOP 3, END 4) and `SCL_LOW/HIGH_PERIOD` and
`SDA_HOLD` in APB cycles; it has no `SCL_STRETCH_CONF`, so a classic target
whose TX FIFO is empty sends 0xFF instead of stretching: firmware keeps the
TX FIFO loaded ahead of a read, as the example does. A TX byte written
through the classic AHB FIFO alias (0x6001_301C) reaches a master `WRITE`
waiting for it only with the next APB register write. Tested against an
STM32G071 both ways round on each part (`env-spi-esp32s3*.yaml`,
`env-spi-esp32*.yaml`, `env-i2c-esp32s3*.yaml`, `env-i2c-esp32*.yaml`;
hand-written Xtensa assembly, see the example's `build.sh`), including an
address NACK, a slow G071 target holding either controller, and a slow S3
target stretching the G071's clock. The second instances run on bank-1
pads (GPIO32 and up) in the same tests: the S3's GP-SPI3 on GPIO34..37 and
I2C1 on GPIO41/42 (`env-spi-esp32s3-spi3*.yaml`,
`env-i2c-esp32s3-i2c1*.yaml`), the classic I2C1 on GPIO32/33
(`env-i2c-esp32-i2c1*.yaml`), and the classic VSPI master / HSPI slave with
MISO, or SCK, MOSI and CS0, on the input-only GPIO34..36
(`env-spi-esp32-hi*.yaml`). Plain GPIO crosses bank-1 pads between a classic
ESP32 and an ESP32-S3 (`env-gpio-esp32-esp32s3.yaml`: `OUT1` onto the other
part's `IN1`, each edge latched in `GPIO_STATUS1`).

The ESP matrix routes follow firmware: a line becomes a net line the first
time firmware routes it (in or out) to a pad on a net. IO_MUX direct
functions (`MCU_SEL` other than GPIO) and `IN_INV_SEL`/`INV_SEL` are not
followed, so choose pads without an IO_MUX function for the signal (esp-hal
then uses the matrix as well).

Clock stretching is tested on every target: an STM32G071 (modern), an
STM32F401 (legacy), an RP2040 and an ESP32-C3 slave whose firmware answers
several SCL low periods late each hold the controller, and the bytes and
clock count come out the same; a slow G071 target holds an ESP32-C3
controller (register level and esp-hal), an ESP32-S3 and a classic ESP32
controller the same way, and a slow ESP32-S3 target holds the G071. A target that ends a stretch with a new bit on SDA releases
SCL a data setup time later (`SDADEL`-style, one quarter of its own SCL low
period on the legacy and RP2040 engines, 250 ns on the nRF52 TWIS), so the
change is never read as START or STOP.

The legacy and modern controllers meet only on the wire, so an F103 target
answers a G071 controller as it answers an F401 one (tested both ways round
between F103 and F401, and G071 → F103).

Everything is event-scheduled: a master's bits run at their exact cycles, a
slave or target runs when the net delivers an edge (at the cycle the world
applies it, even on a timed core that publishes its cycle to the bus only at
peripheral ticks). The one poll is a chip
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

Not modelled on a net yet: other SPI/I²C families (the H5 SPI v3, whose NSS
pads stay unrouted), legacy-I²C
10-bit addressing, SMBus/PEC, modern-I²C slave byte control (`SBC`), the I²C
filters and timeouts, SPI CRC, TI mode and the SPI mode fault (`MODF`). Such
a peripheral on a net keeps its usual behaviour and drives its lines
push-pull as before.

### SPI DMA

The data of an SPI transfer can move by DMA on either chip, master or slave,
on a net or off it (byte-level devices on the bus); the bits on the wire are
the same as with the CPU feeding `DR` or `W0..W15`.

**STM32** (classic F1/F4/L0 and FIFO G0/L4/G4/WB/F7 SPI). `CR2.TXDMAEN`
raises a request while TXE is set (the classic port's buffer is empty; the
FIFO port's TX FIFO is at or below half full) and the SPI is enabled;
`CR2.RXDMAEN` while RXNE is set (on the FIFO port it follows `FRXTH`). Each
request is held until the `DR` write (TX) or read (RX) that answers it, so
the DMA moves one item per free slot or received frame. The request reaches
the DMA channel the chip's routing registers select: the F1's fixed channels
and the L4's `CSELR` (`type: dma`, `config.requests: { spi1_tx: 3, spi1_rx:
[2, 1] }`), the F4's `SxCR.CHSEL` (`type: stm32f4_dma`, `config.requests: {
spi1_tx: [[3, 3], [5, 3]] }`, stream and channel), and on the G0 the
DMAMUX (`type: dmamux`, `DMAMUX_CxCR.DMAREQ_ID`, channel x feeding DMA1
channel x + 1). The G071, F103, F401, F407 and L476 chip files declare their
SPI routes. Each DMA item is one access at its programmed width, so a
half-word item packs two frames at `DS` ≤ 8 bits; with `CR2.LDMA_TX` the last
item of a transfer (the DMA tells the SPI, as its end-of-transfer signal
does) carries one frame, and with `LDMA_RX` and `FRXTH = 0` the odd last byte
is requested below the 16-bit threshold. Enable the DMA channel before the
SPI's request bit, as HAL does: a request raised while no channel is enabled
is not seen again until it is answered.

**ESP32-C3 / ESP32-C6** (GP-SPI2 by GDMA). With `SPI_DMA_CONF.DMA_TX_ENA`
the transaction's MOSI bytes (a master's data phase, a slave's answer) come
from what the GDMA OUT link bound to SPI2 (`OUT_PERI_SEL = 0`) pushed into
the controller when it was started; with `DMA_RX_ENA` the bytes received go
to the IN link (`IN_PERI_SEL = 0`), whose descriptors get the data, their
length, `suc_eof` on the last and the owner bit back to the CPU. The data
phase may then exceed the 64-byte W buffer. The transaction completes
(`USR` clears, `SPI_TRANS_DONE`) once the IN link holds the bytes, so
firmware that sees it done finds them in memory; `IN_SUC_EOF` / `IN_DONE`
and `OUT_EOF` / `OUT_TOTAL_EOF` / `OUT_DONE` latch in the GDMA. The SPI's
AFIFO reset bits (`SPI_DMA_CONF` 29..31) empty its DMA FIFOs, as drivers do
before each transaction. The C3 GDMA (`esp32c3_gdma`) also copies memory to
memory (`IN_CONF0.MEM_TRANS_EN`).

**The STM32 DMA controllers themselves.** One data item per arbitration slot
(one slot per cycle) per controller: when several channels or streams have
work, the slot goes to the highest software priority (`CCR.PL`, `SxCR.PL`;
GPDMA `CCR.PRIO`, 11 first), then the lowest channel or stream number, and a
request that loses the slot stays latched. An item that faults on the bus (an
unmapped address) sets `TEIF` (GPDMA `DTEF`) and disables its channel or
stream, as silicon does. The F4/F7 stream FIFO (`SxFCR.DMDIS` = 1, and always
for memory-to-memory) is modelled: requests fill or drain a 16-byte FIFO at
the peripheral port (`PBURST` moves 4/8/16 items per request), the memory
port moves `FTH` worth (1/4 .. full) as `MSIZE` beats (`MBURST` bursts),
`PSIZE` ≠ `MSIZE` packs and unpacks little-endian, `SxNDTR` counts `PSIZE`
items, `SxFCR.FS` reads the level, and clearing `EN` flushes a P2M FIFO to
memory before the stream stops (`EN` reads 1 until then, `TCIF` set).
Direct mode forces `MSIZE` = `PSIZE` and no bursts at enable. `FEIF` is set
for a threshold that does not hold a whole number of memory bursts (RM0090
Table 48: the stream does not enable) and for a FIFO overrun (P2M) or
underrun (M2P) when a request comes. On the G0 the DMAMUX request generators
(`RGxCR`) turn edges of EXTI lines 0..15 (the pad `EXTI_EXTICRx` selects) or
of the channel events `dmamux_evt0..3` into `GNBREQ + 1` requests, and
request synchronisation (`CxCR.SE`, `SYNC_ID`, `SPOL`, `NBREQ`) lets
`NBREQ + 1` requests through per edge, holding the rest; `EGE` emits a
channel's event every `NBREQ + 1` requests; `RGSR.OF` / `CSR.SOF` report
overruns.

**GDMA descriptor errors.** On the GP-SPI2 path a CPU-owned descriptor under
`*_CHECK_OWNER`, or a descriptor, buffer or next link outside the internal
SRAM the DMA reaches, stops the walk with `IN_DSCR_ERR` / `OUT_DSCR_ERR`; an
IN list shorter than the data sets `IN_DSCR_EMPTY` (no `IN_SUC_EOF`); an OUT
list that ends without a `suc_eof` descriptor sends its bytes without
`OUT_EOF`. `*_DONE` latch per descriptor consumed.

Tested end to end over nets (`examples/gpio-net-buses`): a G071 master and a
G071 slave both by DMA through the DMAMUX (`env-spi-dma.yaml`), an F401
master whose DMA2 streams first complete two memory-to-memory copies in
priority order and then move SPI1 both ways through their FIFOs in half-word
INCR4 bursts against the G071 slave (`env-spi-f4-dma.yaml`), and an ESP32-C3
and an ESP32-C6, master and slave by GDMA, against a G071 by DMA
(`env-spi-esp32c3-dma.yaml`, `env-spi-esp32c3-dma-slave.yaml`), each run with
the nodes swapped and with 37 ns rounds.

Not modelled: `DMEIF` (the request handshake never loses a direct-mode
item), bus timing inside a slot (a burst or FIFO drain moves at once), the
GPDMA's weighted round robin among low-priority channels (strict `PRIO`
order instead), DMAMUX inputs with no source in the model (G071 LPTIM1_OUT,
LPTIM2_OUT and TIM14_OC), the G071's DMA / DMAMUX interrupt lines (the chip
file wires none), GDMA descriptor errors on the C6 memory-to-memory path, the
GP-SPI's segmented DMA transfers, and DMA for the other Espressif peripherals
(UHCI0, I2S, crypto, ADC stall visibly on the C3 and C6 GDMA).

## Reports

`World::gpio_net_reports()` (and `result.json` `gpio_nets` from
`labwired test`, and `WasmWorld.gpio_net_report()` in the browser) lists each
net: level, edge count, contention and floating counters, each member's drive,
and the diagnostics with times in picoseconds.
