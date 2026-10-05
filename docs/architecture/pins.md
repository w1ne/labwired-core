# Pins: the one interface between a chip's pads and the world

Status: design, being implemented. Supersedes the per-model GPIO hooks on
`Peripheral` (`read_gpio_pad`, `read_gpio_pad_drive`, `set_gpio_input`,
`set_gpio_net_isolated`, `gpio_input_edge`).

## Why

Every consumer of a pad (the world's `gpio_net`, the logic analyzer, board
buttons and LEDs, motor and sensor components, co-simulation plugins, the
browser and Python bindings) used to talk to each GPIO model through a
different subset of loose `Peripheral` methods. Each model grew its own
bookkeeping for the same ideas (`net_isolated`, `externally_driven`, push-tap
snapshots), and EXTI edge routing found the port by parsing the peripheral's
name (`"gpioa"` → port 0). Adding a chip to a net meant touching all of it.

This document defines the interface instead. A GPIO model implements one
trait; everything else uses it, and nothing else.

## The model: two sides of a pad

A pad has exactly two sources of electrical truth:

1. **The chip's own output stage**, decided by the chip's registers only:
   driving low, driving high, or released, plus an optional internal pull.
2. **The outside world**: whatever the board connects to the pad (a net, a
   button, a sensor), which presents a level, or nothing.

What firmware reads (`IDR`, `PINx`, `GPIO_IN`, `SIO_GPIO_IN`) and what a probe
sees follow from those two by one shared rule (`pins::resolve`). No model
implements that rule itself.

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

/// The interface. Implemented by every GPIO model (STM32, AVR, ESP32
/// family, RP2040, nRF, ...), by co-simulation plugins that model a port,
/// and by anything else that owns pads.
pub trait PinPort {
    /// Pads this port owns (pins 0..count).
    fn pin_count(&self) -> u8;

    /// The chip's own drive on `pin`, `None` when the model cannot say
    /// (a pad handed to a peripheral signal whose drive is not published).
    fn driver(&self, pin: u8) -> Option<PadDriver>;

    /// Present `ext` to the pad's input buffer. The model updates what
    /// firmware reads and returns the input level before and after, so the
    /// caller can raise edge events. `None`: the pad cannot take input.
    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange>;
}
```

`Peripheral` gains `fn pins(&self) -> Option<&dyn PinPort>` and
`fn pins_mut(&mut self) -> Option<&mut dyn PinPort>`. Nothing else about pads
lives on `Peripheral`.

## Services built on it (written once, in `crate::pins`)

| Service | How it uses the interface |
|---|---|
| **Pad level / four-state trace** | `resolve(driver, external)` → `0`/`1`/`z`/`x`. |
| **Push capture** | The bus snapshots `driver()` of *watched* pads after each MMIO write to a `PinPort` peripheral and after each `set_external`, and reports differences to the logic tap with the cycle stamp. Unwatched pads cost nothing. Models write no capture code. Peripherals that drive AF pads from a bit engine keep publishing through `PadLines`, which already reports to the same tap. |
| **Edge events** | `set_external` returns the input change; the bus fans it out to every registered edge sink (EXTI, AVR INT/PCINT, ESP32/RP2040 GPIO interrupt blocks, timer input capture) by a `PortId` assigned when the port is attached. No name parsing. Edges caused by the chip's own output are fanned out the same way from push capture. |
| **GPIO nets** | A net member *is* `driver()`; the level the net settles on goes back through `set_external(Level)`. No isolation flag: `driver()` never includes the external level by definition. Internal pulls are weak drives in the net's resolution. |
| **Board parts** | A button is `set_external(Level)`/`Released`; an LED reads `resolve(...)`. |
| **Bindings** | Wasm, Python and the CLI address a pad as `(node, port, pin)` and reach it only through `PinPort`. |

## Implementing a new GPIO model

1. Decode your direction/output/open-drain/pull registers into `driver(pin)`.
2. Store the external level per pad and fold it into your input register in
   `set_external`; return the before/after input level.
3. Implement `pins()`/`pins_mut()` on your `Peripheral`.

That is all: logic analyzer, push capture, nets, buttons, edge interrupts and
reports follow. A conformance test suite (`pins::conformance`) runs against
every registered model: driver never echoes an external level, open-drain high
is `Off`, a pull shows up in `driver()`, `set_external` round-trips to the
input register, and push capture matches the per-cycle poll exactly.

## Migration

The old `Peripheral` GPIO methods become default shims over `PinPort` and are
removed once every caller has moved. Behaviour and existing tests must stay
identical; the conformance suite is the bar every model has to clear.
