# Pins: the one interface between a chip's pads and the world

Status: implemented (`crates/core/src/pins/`). Every GPIO model in tree is
on it; the old per-model GPIO hooks on `Peripheral` are shims over it (see
[Shims](#shims-that-remain)).

## Why

Every consumer of a pad (the world's `gpio_net`, the logic analyzer, board
buttons and LEDs, motor and sensor components, co-simulation plugins, the
browser and Python bindings) used to talk to each GPIO model through a
different subset of loose `Peripheral` methods. Each model grew its own
bookkeeping for the same ideas (`net_isolated` masks, `externally_driven`,
push-tap snapshots, a "drive" that meant something different per model), and
EXTI edge routing found the port by parsing the peripheral's name at every
edge (`"gpioa"` → port 0). Adding a chip to a net meant touching all of it.

This is the interface instead. A GPIO model implements one trait; everything
else uses it.

## The model: two sides of a pad

A pad has exactly two sources of electrical truth:

1. **The chip's own output stage**, decided by the chip's registers only:
   driving low, driving high, or released, plus an optional internal pull.
2. **The outside world**: whatever the board connects to the pad (a net, a
   button, a sensor), which presents a level, or nothing.

```rust
/// What this chip's output stage does to the pad. Register truth only:
/// never includes a level applied from outside.
pub struct PadDriver {
    pub out: Out,     // Off | Low | High   (open-drain "1" is Off)
    pub pull: Pull,   // None | Up | Down   (internal resistor)
}

/// What the outside world presents to the pad.
pub enum External {
    Released,          // nothing connected / connection let go
    Level(bool),       // something drives the wire to this level
}

/// The firmware-visible input before and after `set_external`.
pub struct InputChange { pub before: bool, pub after: bool }

pub trait PinPort {
    fn pin_count(&self) -> u8;
    /// The chip's own drive, `None` when the model cannot say (a pad handed
    /// to a peripheral signal whose drive is not published).
    fn driver(&self, pin: u8) -> Option<PadDriver>;
    /// What the outside world currently presents (the last `set_external`).
    fn external(&self, pin: u8) -> External;
    /// Present `ext` to the pad's input buffer; `None`: the pad takes no input.
    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange>;
    /// The firmware-visible input level (IDR, PINx, GPIO_IN, SIO GPIO_IN).
    fn input(&self, pin: u8) -> Option<bool>;
    /// What a logic probe on the pad reads. Default: `resolve`, falling back
    /// to `input()` where the rule leaves the level open (a floating pad).
    fn level(&self, pin: u8) -> Option<bool> { /* resolve(...) */ }

    // Optional hooks, all defaulted:
    fn join_wire(&mut self, pin: u8) {}                 // pad joined a multi-chip wire
    fn install_watch(&mut self, w: Option<PadWatch>) -> bool { false }
    fn take_watch(&mut self) -> Option<PadWatch> { None }
    fn put_watch(&mut self, w: PadWatch) {}
    fn routes_changed(&mut self) {}                     // re-register line taps
    fn pad_control(&self) -> Option<(&str, PadControl)> { None } // pulls kept elsewhere
    fn set_config_pull(&mut self, pin: u8, pull: Pull) -> bool { false }
    fn set_pull_ups_disabled(&mut self, disabled: bool) -> bool { false } // AVR PUD
}
```

`Peripheral` has `fn pins(&self) -> Option<&dyn PinPort>` and
`fn pins_mut(&mut self) -> Option<&mut dyn PinPort>`, `None` by default.

Three methods more than the first sketch, each for a reason the code made:

- `external()`: the four-state trace needs both sides of the pad. Every model
  already stored the external level; now it says so.
- `input()`: what firmware reads, which is what `set_external` reports and
  what the conformance round trip checks.
- `level()`: what a probe reads. It defaults to the shared rule; a model
  overrides it only where its input register has a documented quirk the
  probe must keep agreeing with (below).

Two more, for pulls a GPIO port does not keep itself:

- `pad_control()` / `set_config_pull(pin, pull)`: Kinetis (`PORTx_PCRn`),
  Renesas RA (`PmnPFS`) and i.MX RT (IOMUXC `SW_PAD_CTL_PAD_*`) configure a
  pad's pull in a pad-control block, not in the GPIO port. The port names
  that block (`pad_control:` in the chip yaml, a `pins::PadControl` encoding);
  the bus links the two when the port is attached, and after every write to
  the block decodes each pad's pull and hands it over, bracketed for push
  capture like a write to the port itself. The port reports it in
  `driver().pull` and folds it into its input register.
- `set_pull_ups_disabled(bool)`: a chip-wide switch outside the port. The
  ATmega CPU owns `MCUCR`; when firmware changes `PUD` it calls
  `Bus::set_pull_ups_disabled`, and the bus hands it to every port
  (bracketed). The ATmega port drops every pull-up while it is set.

### The shared rule: `pins::resolve(driver, external)`

| Own output stage | Outside | Level | Drive |
|---|---|---|---|
| drives `l` | released | `l` | driven |
| drives `l` | presents `l` | `l` | driven |
| drives `l` | presents `!l` | `l` (the chip's own, as every input register reads) | contention |
| released | presents `e` | `e` | driven (from outside) |
| released, pull up/down | released | `1` / `0` | weak (`h` / `l`) |
| released, no pull | released | undetermined | high-Z |

Two views are built on it and nothing else:

- `pins::probe_drive(port, pin)`: what a probe clipped to the pad sees, the
  four-state pin trace. `resolve` with an internal pull reported as `z`, so
  ordinary traces read `z` for an undriven input, pulled or not. Driver
  unknown: an external level is "driven", otherwise nothing.
- `pins::own_drive(port, pin)`: the chip's own output stage alone,
  `resolve(driver, Released)`, so a pull is a weak level. This is what a
  world `gpio_net` member is.

## Services built on it

| Service | How it uses the interface |
|---|---|
| **Pad level / four-state trace** | `LogicSource::Pad { peripheral, pin }`: level `PinPort::level`, drive `probe_drive`. `Peripheral::read_gpio_pad` / `read_gpio_pad_drive` are shims over exactly these. |
| **Own-drive channel** | `LogicSource::Driver { peripheral, pin }`: level `PinPort::level`, drive `own_drive`. New: a probe on what *this chip* drives, which is how a world watches its net members. |
| **Push capture** | `Machine::logic_watch` builds one `pins::PadWatch` per port (the tap plus the watched `(pin, channel, view)` triples) and installs it with `PinPort::install_watch`. A model brackets every pad mutation (register write, `set_external`) with `pins::watch_begin` / `pins::watch_end`; the watch reads each watched pad through the trait before and after and pushes what changed. Models write no snapshot or report code. Writes to another block that move a port's pads (C3/S3 `IO_MUX` pulls, RP2040 `IO_BANK0` FUNCSEL) are bracketed by the bus the same way, through `pins_mut()`, with no downcast. Peripherals that drive AF pads from a bit engine keep publishing through `PadLines`, registered by the model in `routes_changed`. A port that does not install a watch (the declarative GPIO) stays on the per-cycle poll. |
| **Edge events** | `SystemBus::set_pad_external(idx, pin, ext)` calls `set_external`, and fans a changed input out to every edge sink by the port's `PortId` (`Peripheral::gpio_input_edge`: the STM32 EXTI, which resolves the port select through its own or the AFIO/SYSCFG mux). The `PortId` is assigned once, when the bus attaches the port (`rebuild_peripheral_ranges`), from the port's id in the chip yaml (`gpioa` = 0); the fan-out never looks at a name. The same call delivers timer input-capture edges, peripheral line inputs, and arms the port's own scheduler-driven interrupt logic (ESP32-family GPIO matrix line, RP2040 `IO_BANK0` via the SIO wake owner); those blocks latch their status inside the model from the same input change. The ATmega INT/PCINT logic samples `PINx` at instruction boundaries and needs no fan-out. |
| **GPIO nets** | A member *is* `driver()`: the world joins its pads (`Machine::join_net_pads`: the pad must have a known driver; `PinPort::join_wire` tells routed peripheral lines they share a wire) and watches them as `LogicSource::Driver` channels; the level the net settles on goes back through `set_external(Level)`. No isolation flag anywhere: `driver()` never includes the external level by definition, and internal pulls are weak drives in the net's resolution. |
| **Board parts / bindings** | A button is `set_external(Level)` through the bus; an LED reads `level()`. Wasm, Python and the CLI reach pads through the bus and these shims. |

## Implementing a new GPIO model

1. Decode your direction/output/open-drain/pull registers into `driver(pin)`.
   Registers only: never fold in what `set_external` stored.
2. Store the external level per pad (a mask and the levels) and fold it into
   your input register in `set_external`; return the input before and after.
   Answer `external(pin)` from that storage. `Released` hands the pad back to
   its pull (or leaves a floating input at its last level). An input with a
   pull and nothing outside reads the pull's rail in the input register: the
   pull drives the pad on silicon.
3. `input()` is your input register; leave `level()` defaulted unless your
   input register has a quirk (document it on the override).
4. Implement `pins()` / `pins_mut()` on your `Peripheral`. If the pull lives
   in another block, return it from `pad_control()` (adding a `PadControl`
   encoding if yours is new) and take it in `set_config_pull`.
5. For push capture: store the `PadWatch` (`install_watch`, `take_watch`,
   `put_watch`) and bracket each register write and `set_external` with
   `pins::watch_begin(self)` / `pins::watch_end(self)`.
6. Add a `Rig` for it in `crates/core/src/pins/conformance.rs`.

That is all: logic analyzer, four-state trace, push capture, nets, buttons,
EXTI and reports follow.

## Conformance

`crates/core/src/pins/conformance.rs` runs against every model in tree
(STM32 `v2` and `f1`, nRF52, Kinetis, EFR32 series 2, SAM, RA, i.MX RT, the
ATmega port, RP2040 SIO, ESP32 classic, ESP32-C3/C6, ESP32-S3, the
declarative `GPIO` descriptor), putting a pad into each mode the model has
through its own registers:

- `driver()` never echoes an external level (presenting either level, or
  releasing, leaves it unchanged), and `external()` reports what was
  presented;
- an input is `Off`, a push-pull output drives its latch, an open-drain 1 is
  `Off`, an open-drain 0 is `Low`;
- a pull shows up in `driver()` and as a weak `own_drive`, and the input
  register reads its rail while nothing outside holds the pad (from reset,
  and again after the outside lets go); a pull configured in a pad-control
  block (Kinetis, RA, i.MX RT) is written there and handed over the way the
  bus does it;
- `set_external` round-trips to the input register and reports the input
  before and after; a pin past the port is refused;
- `level()` agrees with `resolve` wherever the rule determines a level, and
  `probe_drive` is `resolve` with pulls as `z`;
- push capture equals the per-cycle poll exactly (levels and four-state
  drives, a probe channel and an own-drive channel on the same pad), run in a
  machine with `logic_force_poll_capture`.

### Model choices the rule leaves open

Every model folds its internal pull into the input register: a pulled input
nothing outside holds reads the pull's level, from reset on, as `resolve`
says. No model is exempt. (Until this was enforced, the STM32 `f1`, EFR32
series 2, SAM and ATmega input registers read the last latched level there,
0 from reset, so `INPUT_PULLUP` with nothing attached read 0.) The ATmega
port drops its pull-ups while `MCUCR.PUD` is set.

Two choices consistent with the rule (it leaves a floating pad to the
model):

- An open-drain output holding a 1 that nothing outside holds keeps reading
  its latch (STM32, ESP32 family), standing in for the board pull-up these
  models have no other source for. As soon as the outside presents a level
  (a net always does) it reads that level.
- ESP32-S3: the interface covers bank 0 (GPIO0..31), the pads with input
  storage and a probe level in the model.

What the pull models leave out:

- Kinetis: the pull acts only on a pin with a digital `MUX` (non-zero); the
  GPIO port does not otherwise model `MUX`.
- Renesas RA: `PmnPFS.PCR` is the only `PmnPFS` field with an effect.
  `PDR`/`PODR` there are the same flip-flops as `PCNTR1` on silicon but are
  stored only, and `PIDR` reads 0.
- i.MX RT: the port's pads must be contiguous in IOMUXC from pin 0 (GPIO1,
  GPIO2, GPIO4); the keeper (`PUE = 0`) is not modelled and counts as no
  pull.
- EFR32 series 2: only `INPUTPULL` / `INPUTPULLFILTER` carry a pull; the
  `WIREDOR`/`WIREDAND` pull variants are not decoded.

## Shims that remain

| Old `Peripheral` method | Now |
|---|---|
| `read_gpio_pad` | default shim over `PinPort::level` (callers that only want a level: SPI/I²C waveform checks, analog mux, resident devices, inspect, many tests). |
| `read_gpio_pad_drive` | default shim over `pins::probe_drive`. |
| `set_gpio_input` | default shim over `set_external(Level)`; it raises no edge events, so callers that hold a bus use `SystemBus::set_peripheral_gpio_input` / `set_pad_external`. |
| `set_gpio_net_isolated` | removed, with every per-model isolation mask. |
| `install_logic_tap` | kept for peripherals that own pads without being a pin port (the ESP32-C3 RMT); pin ports use `install_watch`. |
| `gpio_input_edge` / `exti_line_source` | kept: they are the edge-sink and port-mux side, addressed by `PortId`. |
| `read_gpio_input`, `read_gpio_output`, `gpio_routing`, ... | unchanged: they answer other questions (latch, direction, function). |
