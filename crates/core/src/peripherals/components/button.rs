// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Momentary push button / slide switch — one passive contact on a GPIO input.
//!
//! A button has no chip: it is a pair of terminals that either conduct or do
//! not. One terminal goes to an MCU pin, the other to a rail:
//!
//! ```text
//!   3V3 ──[ pull-up ]──┬── PC13  (MCU input, reads HIGH while released)
//!                      │
//!                    ──o o──     the button
//!                      │
//!   GND ───────────────┘         pressing shorts the pin to GND → reads LOW
//! ```
//!
//! Which level "pressed" produces is a property of the WIRING, not the part:
//! a button to GND with a pull-up is active-LOW (the common case, and what an
//! `INPUT_PULLUP` sketch expects); a button to VCC with a pull-down is
//! active-HIGH. The canvas compiler already derives that polarity from the
//! diagram and stamps it on the `board_io` binding as `active_high`, so this
//! model simply honours it.
//!
//! ## Why it lives on the bus (not as an MMIO peripheral)
//!
//! Like [`Keypad`](crate::peripherals::components::keypad::Keypad) — which is
//! literally sixteen of these — a button DRIVES a pin the MCU samples as an
//! input and answers no register read, so it cannot be memory-mapped. It is a
//! [`BusResidentDevice`](crate::bus::BusResidentDevice): one per-tick pass
//! drives its input (IDR) bit, touching the bus only on a transition.
//!
//! ## Fidelity boundary
//!
//! Contact bounce is not modelled: a press is one clean transition. Real
//! buttons bounce for ~1–10 ms and firmware is expected to debounce; a
//! bounce-free press is the faithful *debounced* case, and a debounce routine
//! written against it still behaves correctly. The pull resistor is modelled
//! only as the released level — its value is not simulated because nothing the
//! firmware can observe depends on it.
//!
//! ## Stimulus
//!
//! Host-controlled through the standard stimulus API: one channel, `pressed`,
//! `1` = pressed, `0` = released. This is what makes "press the button and
//! prove the LED toggles" scriptable for the oracle — without it a button on
//! the canvas is inert in a headless run, and every stimulus naming it is
//! rejected as an unknown channel.

/// One momentary contact wired to a single MCU input pin.
#[derive(Debug, Clone)]
pub struct Button {
    /// board_io binding id — targets the `pressed` setter.
    pub id: String,
    /// An address inside the owning GPIO peripheral's range (its base is fine)
    /// plus the pin index. The address only has to identify WHICH peripheral
    /// owns the pin — the level is applied through that peripheral's
    /// `set_gpio_input`, so this works for a per-port register model (STM32,
    /// Nordic, Kinetis) and a single GPIO-matrix model (ESP32) alike.
    pub gpio: (u64, u8),
    /// Level the pin reads while the button is PRESSED, derived from the wiring
    /// by the canvas compiler. `false` (active-low) is the common pull-up case.
    pub active_high: bool,
    /// The stimulus channel this contact answers to. One of [`CHANNELS`] — a
    /// closed vocabulary, so the key stays `&'static` and `input_channels` needs
    /// no allocation or leak.
    channel: &'static crate::sim_input::InputChannel,

    /// Whether the contact is currently closed.
    pressed: bool,
    /// Last level this button drove; `None` forces the first drive so the pin
    /// settles at its released level at boot (the IDR bit resets to 0, which is
    /// the WRONG level for an active-low button — an undriven pull-up pin must
    /// read HIGH or a `while (digitalRead(pin) == LOW)` spins forever).
    last_high: Option<bool>,
}

/// Every channel a contact may expose.
///
/// A PIR, IR-obstacle, hall or vibration sensor is the same device as a push
/// button — a digital output asserting a level on one pin — so they share this
/// model. What differs is only the WORD: "pressed" is wrong for motion or a
/// magnetic field, and a stimulus API whose vocabulary does not match the part
/// on the canvas is one an agent cannot script blind.
///
/// Closed set by design: the key must be `&'static` for
/// [`SimInput::input_channels`](crate::sim_input::SimInput::input_channels),
/// and an unknown name falling back to `pressed` is better than leaking a
/// `String` per attached part.
pub const CHANNELS: &[crate::sim_input::InputChannel] = &[
    ch("pressed", "Pressed"),
    ch("obstacle", "Obstacle detected"),
    ch("field", "Magnetic field"),
    ch("vibration", "Vibration"),
    ch("motion", "Motion detected"),
    ch("touch", "Touched"),
];

/// One boolean contact channel: 0 released / absent, 1 asserted.
const fn ch(key: &'static str, label: &'static str) -> crate::sim_input::InputChannel {
    crate::sim_input::InputChannel {
        key,
        label,
        unit: "bool",
        min: 0.0,
        max: 1.0,
    }
}

/// Resolve a channel name to its entry, falling back to `pressed`.
pub fn channel_or_pressed(name: Option<&str>) -> &'static crate::sim_input::InputChannel {
    match name {
        Some(n) => CHANNELS.iter().find(|c| c.key == n).unwrap_or(&CHANNELS[0]),
        None => &CHANNELS[0],
    }
}

impl Button {
    pub fn new(id: String, gpio: (u64, u8), active_high: bool) -> Self {
        Self::with_channel(id, gpio, active_high, None)
    }

    /// A contact answering to a named channel (`obstacle`, `field`, ...).
    /// Unknown names fall back to `pressed` rather than minting a channel the
    /// discovery surface would report but nothing could drive.
    pub fn with_channel(
        id: String,
        gpio: (u64, u8),
        active_high: bool,
        channel: Option<&str>,
    ) -> Self {
        Self {
            id,
            gpio,
            active_high,
            channel: channel_or_pressed(channel),
            pressed: false,
            last_high: None,
        }
    }

    /// Whether the contact is closed. Exposed for tests and UI readback.
    pub fn is_pressed(&self) -> bool {
        self.pressed
    }

    /// Open or close the contact.
    pub fn set_pressed(&mut self, pressed: bool) {
        self.pressed = pressed;
    }

    /// The level the MCU pin reads right now: the active level while pressed,
    /// its complement (the pull resistor) while released. Pure query — no state
    /// change — so the truth table is directly testable.
    pub fn pin_level(&self) -> bool {
        if self.pressed {
            self.active_high
        } else {
            !self.active_high
        }
    }

    /// Level to drive plus whether it differs from the last one driven, so the
    /// bus can skip an untouched pin. Mirrors `Keypad::service`.
    pub fn service(&mut self) -> (bool, bool) {
        let high = self.pin_level();
        let changed = self.last_high != Some(high);
        self.last_high = Some(high);
        (high, changed)
    }
}

/// Drivable contact state: `1` pressed, `0` released. Buttons live directly on
/// the bus (`SystemBus::gpio_devices`), so the bus input walk reaches this impl
/// and reports each button under its `id` — same as the keypad and encoder.
impl crate::sim_input::SimInput for Button {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        std::slice::from_ref(self.channel)
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        // Range-checked to [0, 1] above; anything at or past the midpoint is a
        // press so a caller writing 1.0 or 0.9 gets the same obvious meaning.
        self.set_pressed(value >= 0.5);
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        Some(&self.id)
    }
}

impl crate::bus::BusResidentDevice for Button {
    /// Hold the pin at the level the contact currently produces.
    /// Combinational, so `now` is unused.
    ///
    /// Goes through `set_gpio_input` (the external-world seam) rather than an
    /// MMIO store to the input register: IDR is read-only on silicon, so a
    /// store is correctly ignored by the STM32F1 model and the button would
    /// never move its pin.
    fn service(&mut self, pins: &mut dyn crate::bus::DevicePins, _now: u64) {
        let (high, changed) = Button::service(self);
        if changed {
            let (addr, bit) = self.gpio;
            pins.drive_input_bit(addr, bit, high);
        }
    }

    fn as_sim_input(&mut self) -> &mut dyn crate::sim_input::SimInput {
        self
    }

    fn id(&self) -> &str {
        &self.id
    }

    /// A contact holds its level until something moves it, and `set_input`
    /// applies that level at the stimulus point, so a button needs no per-cycle
    /// pass. Adding a push button to a canvas therefore must not cost the bus
    /// its walk-free fast path.
    fn is_level_driven_on_stimulus(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_input::SimInput;

    fn active_low() -> Button {
        Button::new("btn_pc13".into(), (0x4001_1008, 13), false)
    }

    #[test]
    fn active_low_button_reads_high_until_pressed() {
        let mut b = active_low();
        assert!(b.pin_level(), "released pull-up pin reads HIGH");
        b.set_pressed(true);
        assert!(!b.pin_level(), "pressing shorts the pin to GND → LOW");
        b.set_pressed(false);
        assert!(b.pin_level());
    }

    #[test]
    fn active_high_button_is_the_mirror_image() {
        let mut b = Button::new("btn".into(), (0x4001_1008, 0), true);
        assert!(!b.pin_level(), "released pull-down pin reads LOW");
        b.set_pressed(true);
        assert!(b.pin_level());
    }

    #[test]
    fn first_service_drives_the_released_level() {
        // The IDR bit resets to 0. An active-low button MUST drive its pin HIGH
        // before the firmware first samples it, or a pull-up sketch sees a
        // phantom press that never releases.
        let mut b = active_low();
        assert_eq!(b.service(), (true, true), "first pass always drives");
    }

    #[test]
    fn service_reports_a_change_only_on_a_transition() {
        let mut b = active_low();
        assert_eq!(b.service(), (true, true));
        assert_eq!(b.service(), (true, false), "steady state touches nothing");
        b.set_pressed(true);
        assert_eq!(b.service(), (false, true));
        assert_eq!(b.service(), (false, false));
    }

    #[test]
    fn pressed_channel_is_the_one_drivable_channel() {
        let b = active_low();
        let channels = b.input_channels();
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].key, "pressed");
        assert_eq!((channels[0].min, channels[0].max), (0.0, 1.0));
    }

    #[test]
    fn stimulus_presses_and_releases() {
        let mut b = active_low();
        b.set_input("pressed", 1.0).expect("press");
        assert!(b.is_pressed());
        assert!(!b.pin_level());
        b.set_input("pressed", 0.0).expect("release");
        assert!(!b.is_pressed());
        assert!(b.pin_level());
    }

    #[test]
    fn stimulus_rejects_an_unknown_channel_and_an_out_of_range_value() {
        let mut b = active_low();
        assert!(b.set_input("key", 1.0).is_err());
        assert!(b.set_input("pressed", 7.0).is_err());
        assert!(!b.is_pressed(), "a rejected set must not change state");
    }

    #[test]
    fn the_component_id_is_the_board_io_binding_id() {
        assert_eq!(active_low().component_id(), Some("btn_pc13"));
    }
}
