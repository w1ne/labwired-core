// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Incremental (quadrature) rotary encoder — the common EC11-style knob.
//!
//! The encoder is two mechanical contacts, **CLK** (channel A) and **DT**
//! (channel B), both wired to MCU inputs with pull-ups. Turning the shaft opens
//! and closes them a quarter-cycle out of phase, so across one *detent* the
//! `(A,B)` pair walks a 2-bit Gray code and returns to its rest value. Firmware
//! recovers direction from which channel leads:
//!
//! ```text
//!   rest = 11 (both released/high)
//!   CW  detent:  11 -> 01 -> 00 -> 10 -> 11   (A leads B)
//!   CCW detent:  11 -> 10 -> 00 -> 01 -> 11   (B leads A)
//! ```
//!
//! This matches how the ESP32-C3 nodulo-panel firmware polls the knob and how
//! the `esp32c3_nodulo_panel` oracle drives it by hand (AB 11→01→00→10→11 per
//! CW detent). The push switch (**SW**) is an ordinary momentary button and is
//! NOT modelled here — the compiler emits it as a plain `board_io` input.
//!
//! ## Why it lives on the bus (not as an MMIO peripheral)
//!
//! Like [`HcSr04`](crate::peripherals::hc_sr04::HcSr04) and
//! [`Dht22`](crate::peripherals::components::dht22::Dht22), the encoder DRIVES
//! two pins the MCU samples as inputs and observes nothing, so it can't be a
//! plain memory-mapped device. The [`SystemBus`](crate::bus::SystemBus) holds a
//! list of [`RotaryEncoder`] links; a cheap per-tick pass drives each channel's
//! input register, touching the bus only when a level changes.
//!
//! ## Stimulus and fidelity
//!
//! Rotation is host-controlled through the standard stimulus API: a single
//! float channel, `position`, in **detents** from the origin. Advancing it by
//! `n` plays the real Gray-code edge sequence for `n` CW detents onto CLK/DT
//! (or CCW for a decrease). We do NOT snap straight to the target — the
//! intermediate `01`/`00`/`10` phases are the whole point: a firmware quadrature
//! decoder (polling *or* edge-interrupt) must see each one to count the step.
//!
//! The edges are spaced [`EDGE_INTERVAL_US`] apart in *simulated* time. A human
//! knob turns far slower than any MCU polls, so a generous fixed spacing is
//! faithful for both decoder styles and is deliberately coarse (self-paced off
//! `cpu_hz`, a handful of register writes per detent — nothing the browser
//! notices). It is an internal constant, not a per-encoder config field.

/// Simulated time between successive quadrature edges, in microseconds. A knob
/// detent is a human gesture (tens of ms), so ~2 ms per phase (~8 ms per full
/// detent) is unhurried yet lets any reasonable poll loop or edge interrupt
/// observe every intermediate phase. Verified against the nodulo-panel firmware
/// decoder in the bus test below.
const EDGE_INTERVAL_US: u64 = 2_000;

/// Phases per detent: the `(A,B)` Gray code returns to rest every 4 transitions.
const PHASES_PER_DETENT: i64 = 4;

/// One incremental rotary encoder wired to a CLK (A) and DT (B) input pin.
#[derive(Debug, Clone)]
pub struct RotaryEncoder {
    /// board_io / external-device id — targets the `position` setter.
    pub id: String,
    /// Absolute address + bit of the CLK (channel A) GPIO **input** register.
    pub clk_idr_addr: u64,
    pub clk_bit: u8,
    /// Absolute address + bit of the DT (channel B) GPIO **input** register.
    pub dt_idr_addr: u64,
    pub dt_bit: u8,
    /// CPU clock used to convert the edge interval (µs) → simulated cycles.
    pub cpu_hz: u64,

    /// Absolute Gray-code phase the shaft is currently at. Detent rest points are
    /// the multiples of [`PHASES_PER_DETENT`]; `phase / 4` is the detent position.
    phase: i64,
    /// Phase the shaft is walking toward (`target_detent * 4`). Equal to `phase`
    /// when at rest.
    target_phase: i64,
    /// Simulated cycle at which the last phase step was applied; the next step is
    /// due one edge-interval later. Anchored to "now" when a fresh move starts.
    last_step_cycle: u64,
    /// Whether a move is in progress (used to anchor `last_step_cycle` on the
    /// first tick after a retarget without needing `now` inside `set_input`).
    moving: bool,
    /// Last CLK/DT levels driven onto the input registers; `None` forces the
    /// first drive so the pins settle at their rest (both-high) value at boot.
    last_clk_high: Option<bool>,
    last_dt_high: Option<bool>,
}

impl RotaryEncoder {
    pub fn new(
        id: String,
        clk_idr_addr: u64,
        clk_bit: u8,
        dt_idr_addr: u64,
        dt_bit: u8,
        cpu_hz: u64,
    ) -> Self {
        Self {
            id,
            clk_idr_addr,
            clk_bit,
            dt_idr_addr,
            dt_bit,
            cpu_hz: cpu_hz.max(1),
            phase: 0,
            target_phase: 0,
            last_step_cycle: 0,
            moving: false,
            last_clk_high: None,
            last_dt_high: None,
        }
    }

    /// Edge spacing in simulated cycles (`EDGE_INTERVAL_US × cpu_hz / 1e6`),
    /// at least one cycle.
    fn edge_interval_cycles(&self) -> u64 {
        ((EDGE_INTERVAL_US as u128 * self.cpu_hz as u128) / 1_000_000).max(1) as u64
    }

    /// The `(CLK, DT)` levels for a Gray-code phase. Rest (`phase % 4 == 0`) is
    /// both-high; the CW walk is `11 → 01 → 00 → 10` as the phase increments.
    fn phase_levels(phase: i64) -> (bool, bool) {
        match phase.rem_euclid(PHASES_PER_DETENT) {
            0 => (true, true),   // rest
            1 => (false, true),  // A falls first (CW)
            2 => (false, false), // both low
            3 => (true, false),  // A rises first (CW) / B leads (CCW)
            _ => unreachable!(),
        }
    }

    /// Current detent position from the origin (rest points only; mid-detent
    /// this rounds toward the origin).
    pub fn position_detents(&self) -> i64 {
        self.phase.div_euclid(PHASES_PER_DETENT)
    }

    /// Set the target detent position. The shaft then walks the intervening
    /// quadrature phases one edge-interval apart until it reaches
    /// `target * 4`.
    pub fn set_position_detents(&mut self, detents: i64) {
        let target = detents.saturating_mul(PHASES_PER_DETENT);
        if target != self.target_phase {
            self.target_phase = target;
            // Re-anchor pacing on the next serviced tick (we have no `now` here).
            self.moving = false;
        }
    }

    /// Advance the shaft toward its target for simulated cycle `now`, stepping at
    /// most one phase per edge-interval. Returns the `(CLK, DT)` levels to drive
    /// after advancing.
    fn advance_to(&mut self, now: u64) -> (bool, bool) {
        if self.phase == self.target_phase {
            self.moving = false;
            return Self::phase_levels(self.phase);
        }
        // First serviced tick of a fresh move: anchor the cadence to `now` so the
        // first edge lands one interval from here, independent of host timing.
        if !self.moving {
            self.moving = true;
            self.last_step_cycle = now;
        }
        let interval = self.edge_interval_cycles();
        while self.phase != self.target_phase
            && now.saturating_sub(self.last_step_cycle) >= interval
        {
            self.phase += if self.target_phase > self.phase {
                1
            } else {
                -1
            };
            self.last_step_cycle = self.last_step_cycle.saturating_add(interval);
        }
        if self.phase == self.target_phase {
            self.moving = false;
        }
        Self::phase_levels(self.phase)
    }

    /// Service the encoder for simulated cycle `now`: advance the phase and
    /// report the `(clk_high, dt_high)` levels the bus should drive, plus whether
    /// either changed since the last drive (so the bus can skip untouched pins).
    pub fn service(&mut self, now: u64) -> ((bool, bool), (bool, bool)) {
        let (clk, dt) = self.advance_to(now);
        let clk_changed = self.last_clk_high != Some(clk);
        let dt_changed = self.last_dt_high != Some(dt);
        self.last_clk_high = Some(clk);
        self.last_dt_high = Some(dt);
        ((clk, dt), (clk_changed, dt_changed))
    }

    /// Whether the shaft is mid-detent (an edge sequence is still playing out).
    pub fn is_moving(&self) -> bool {
        self.phase != self.target_phase
    }
}

/// Drivable target position, in detents from the origin. Rotary encoders live
/// directly on the bus (`SystemBus::gpio_devices`), so the bus input walk
/// reaches this impl and reports each encoder under its `id`.
impl crate::sim_input::SimInput for RotaryEncoder {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        use crate::sim_input::InputChannel;
        // Relative detents from the origin; range is a generous soft bound for a
        // UI slider — the model itself imposes no hard limit.
        const CH: &[InputChannel] = &[InputChannel {
            key: std::borrow::Cow::Borrowed("position"),
            label: std::borrow::Cow::Borrowed("Position"),
            unit: std::borrow::Cow::Borrowed("detents"),
            min: -1_000.0,
            max: 1_000.0,
            default: None,
        }];
        CH
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        self.set_position_detents(value.round() as i64);
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        Some(&self.id)
    }
}

impl crate::bus::BusResidentDevice for RotaryEncoder {
    /// Advance the shaft to `now` and drive whichever of its CLK/DT input-register
    /// bits changed. This is the body of the former
    /// `SystemBus::drive_rotary_encoder`, moved onto the device; the register IO
    /// stays on the far side of the [`DevicePins`](crate::bus::DevicePins) port.
    /// Two independent pins, each a transition-only IDR write.
    fn service(&mut self, pins: &mut dyn crate::bus::DevicePins, now: u64) {
        // Inherent `RotaryEncoder::service` (chosen over the trait method here —
        // inherent methods win resolution) does the phase advance + change flags.
        let ((clk_high, dt_high), (clk_changed, dt_changed)) = self.service(now);
        // ⚠️ BOTH SEAMS, THE SAME PAIR THE DHT22 DRIVES — and driving only the
        // second made this knob inert on half the catalog. `drive_idr_bit` is an
        // ordinary MMIO store to the input register, which works only where the
        // model lets a store to IDR land (STM32). On silicon whose input word is
        // READ-ONLY the store is correctly ignored and the pin never moves:
        // EFR32 Series 2 (DIN @0x14, `gpio.rs` write_reg drops it by design),
        // SAM PORT (IN @0x20) and ESP32-C3. `drive_input_bit` is the external-
        // world seam (`set_external_input`) those models actually sample.
        //
        // Measured 2026-09-05: on brd2709a the encoder read CLK=0 DT=0 forever,
        // at rest and after a 3-detent stimulus, while nucleo-f401re walked the
        // Gray code correctly from the identical diagram. The stimulus reported
        // `outcome: applied` both times — the exact "attach succeeds, set_input
        // returns Ok, and the pin never moves" failure `gpio_devices_walk_free`
        // was written about, arriving through a different door.
        if clk_changed {
            let _ = pins.drive_input_bit(self.clk_idr_addr, self.clk_bit, clk_high);
            pins.drive_idr_bit(self.clk_idr_addr, self.clk_bit, clk_high);
        }
        if dt_changed {
            let _ = pins.drive_input_bit(self.dt_idr_addr, self.dt_bit, dt_high);
            pins.drive_idr_bit(self.dt_idr_addr, self.dt_bit, dt_high);
        }
    }

    fn as_sim_input(&mut self) -> &mut dyn crate::sim_input::SimInput {
        self
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
