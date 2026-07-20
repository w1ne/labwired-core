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
/// directly on the bus (`SystemBus::rotary_encoders`), so the bus input walk
/// reaches this impl and reports each encoder under its `id`.
impl crate::sim_input::SimInput for RotaryEncoder {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        use crate::sim_input::InputChannel;
        // Relative detents from the origin; range is a generous soft bound for a
        // UI slider — the model itself imposes no hard limit.
        const CH: &[InputChannel] = &[InputChannel {
            key: "position",
            label: "Position",
            unit: "detents",
            min: -1_000.0,
            max: 1_000.0,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(cpu_hz: u64) -> RotaryEncoder {
        // CLK/DT on arbitrary IDR bits — addresses irrelevant to the phase logic.
        RotaryEncoder::new("knob".into(), 0x4800_0410, 3, 0x4800_0410, 4, cpu_hz)
    }

    #[test]
    fn rest_is_both_high() {
        assert_eq!(RotaryEncoder::phase_levels(0), (true, true));
        let e = enc(1_000_000);
        assert_eq!(e.position_detents(), 0);
    }

    #[test]
    fn one_cw_detent_walks_the_gray_sequence() {
        // 1 MHz → 1 cycle/µs, so an edge every EDGE_INTERVAL_US cycles.
        let step = EDGE_INTERVAL_US; // cycles per phase at 1 MHz
        let mut e = enc(1_000_000);
        e.set_position_detents(1); // one CW detent

        // Rest before any time elapses.
        assert_eq!(e.service(0).0, (true, true));
        // The four intermediate/settling phases, one edge-interval apart:
        // 01, 00, 10, 11.
        assert_eq!(e.service(step).0, (false, true), "phase 1 = 01");
        assert_eq!(e.service(2 * step).0, (false, false), "phase 2 = 00");
        assert_eq!(e.service(3 * step).0, (true, false), "phase 3 = 10");
        assert_eq!(e.service(4 * step).0, (true, true), "phase 4 = 11 (rest)");
        assert_eq!(e.position_detents(), 1);
        assert!(!e.is_moving());
    }

    #[test]
    fn ccw_detent_leads_with_b() {
        let step = EDGE_INTERVAL_US;
        let mut e = enc(1_000_000);
        e.set_position_detents(-1); // one CCW detent: 11 -> 10 -> 00 -> 01 -> 11
        assert_eq!(e.service(0).0, (true, true));
        assert_eq!(e.service(step).0, (true, false), "phase -1 = 10 (B leads)");
        assert_eq!(e.service(2 * step).0, (false, false));
        assert_eq!(e.service(3 * step).0, (false, true));
        assert_eq!(e.service(4 * step).0, (true, true));
        assert_eq!(e.position_detents(), -1);
    }

    #[test]
    fn no_edge_before_the_interval_elapses() {
        let step = EDGE_INTERVAL_US;
        let mut e = enc(1_000_000);
        e.set_position_detents(1);
        e.service(0);
        // Half an interval later: still at rest, no phase change.
        assert_eq!(
            e.service(step / 2).0,
            (true, true),
            "no edge before interval"
        );
        assert_eq!(e.service(step).0, (false, true), "edge exactly at interval");
    }

    #[test]
    fn change_flags_only_fire_on_transitions() {
        let step = EDGE_INTERVAL_US;
        let mut e = enc(1_000_000);
        // First service drives the rest state — both flagged changed (from None).
        let (_, changed0) = e.service(0);
        assert_eq!(changed0, (true, true), "initial drive settles both pins");
        e.set_position_detents(1);
        // The service right after a retarget anchors the cadence (no edge yet).
        let (rest, none_changed) = e.service(step);
        assert_eq!(rest, (true, true));
        assert_eq!(none_changed, (false, false), "anchor tick drives no edge");
        // One interval later: phase 1 = 01 — only CLK (A) toggled 1->0; DT stays.
        let (levels, changed) = e.service(2 * step);
        assert_eq!(levels, (false, true));
        assert_eq!(changed, (true, false), "only CLK toggled into phase 1");
    }

    #[test]
    fn cpu_clock_scales_the_edge_spacing() {
        // 80 MHz → 80 cycles/µs, so an edge every EDGE_INTERVAL_US*80 cycles.
        let step = EDGE_INTERVAL_US * 80;
        let mut e = enc(80_000_000);
        e.set_position_detents(1);
        e.service(0);
        assert_eq!(e.service(step - 1).0, (true, true), "no edge just before");
        assert_eq!(e.service(step).0, (false, true), "edge at scaled interval");
    }

    #[test]
    fn set_input_position_channel_maps_to_detents() {
        use crate::sim_input::SimInput;
        let mut e = enc(1_000_000);
        assert_eq!(e.input_channels()[0].key, "position");
        e.set_input("position", 3.4).unwrap(); // rounds to 3 detents
        assert_eq!(e.target_phase, 3 * PHASES_PER_DETENT);
        e.set_input("position", -2.0).unwrap();
        assert_eq!(e.target_phase, -2 * PHASES_PER_DETENT);
        assert!(
            e.set_input("angle", 1.0).is_err(),
            "unknown channel rejected"
        );
    }

    /// End-to-end through the bus with a standard firmware-style quadrature
    /// decoder: set a target position, tick the bus while a poll loop samples the
    /// CLK/DT input pins faster than the edge interval, and confirm the decoder
    /// recovers exactly the detents driven — in both directions. This is the same
    /// poll-and-decode a real knob firmware (e.g. the nodulo panel) runs.
    #[test]
    fn quadrature_decodes_through_the_bus() {
        use crate::bus::SystemBus;
        use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
        use crate::Bus;

        const GPIOA: u64 = 0x4800_0000; // stm32v2: IDR @ 0x10
        const IDR: u64 = GPIOA + 0x10;
        let (clk_bit, dt_bit) = (3u8, 4u8);

        let mut bus = SystemBus::empty();
        bus.add_peripheral(
            "gpioa",
            GPIOA,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
        );
        bus.rotary_encoders.push(RotaryEncoder::new(
            "knob".into(),
            IDR,
            clk_bit,
            IDR,
            dt_bit,
            1_000_000, // 1 MHz → EDGE_INTERVAL_US cycles per phase
        ));

        // Standard 2-bit Gray quadrature decoder: sums quarter-steps, one detent
        // per 4. `AB` packed as (clk<<1)|dt. CW visits 11→01→00→10 as A leads.
        fn sample(bus: &SystemBus, clk_bit: u8, dt_bit: u8) -> u8 {
            let idr = bus.read_u32(IDR).unwrap();
            (((idr >> clk_bit) & 1) << 1 | ((idr >> dt_bit) & 1)) as u8
        }
        // CW successor of each AB state (11→01→00→10→11): [00→10,01→00,10→11,11→01]
        const CW_SUCC: [u8; 4] = [0b10, 0b00, 0b11, 0b01];
        fn qstep(old: u8, new: u8) -> i32 {
            if new == old {
                0
            } else if CW_SUCC[old as usize] == new {
                1
            } else if CW_SUCC[new as usize] == old {
                -1
            } else {
                0 // non-adjacent — never happens with fine sampling
            }
        }

        // Drive `detents`, poll-decode across `cycles`, return recovered detents.
        let drive_and_decode = |bus: &mut SystemBus, detents: i64, start: u64, cycles: u64| {
            bus.rotary_encoders[0].set_position_detents(detents);
            let mut last = sample(bus, clk_bit, dt_bit);
            let mut quarter = 0i32;
            let poll = 200u64; // ~10x oversample of the 2000-cycle edge interval
            let mut c = start;
            while c <= start + cycles {
                bus.set_current_cycle(c);
                bus.service_rotary_encoders();
                let s = sample(bus, clk_bit, dt_bit);
                quarter += qstep(last, s);
                last = s;
                c += poll;
            }
            quarter / 4
        };

        // Settle at rest first (both pins high).
        bus.set_current_cycle(0);
        bus.service_rotary_encoders();
        assert_eq!(sample(&bus, clk_bit, dt_bit), 0b11, "rest = both high");

        // +3 detents CW, then reverse by 5 to net -2. One detent = 4*2000 cycles;
        // give generous headroom.
        let cw = drive_and_decode(&mut bus, 3, 1_000, 40_000);
        assert_eq!(cw, 3, "decoder recovers +3 CW detents");
        assert_eq!(bus.rotary_encoders[0].position_detents(), 3);

        // Reverse to position -2: a delta of -5 detents from +3. The decoder
        // recovers the motion (-5); the encoder lands at absolute position -2.
        let delta = drive_and_decode(&mut bus, -2, 60_000, 60_000);
        assert_eq!(delta, -5, "decoder recovers the -5 detent reversal");
        assert_eq!(bus.rotary_encoders[0].position_detents(), -2);
    }

    #[test]
    fn multi_detent_move_plays_every_phase() {
        let step = EDGE_INTERVAL_US;
        let mut e = enc(1_000_000);
        e.set_position_detents(2); // +2 detents = 8 phases
        let mut seq = Vec::new();
        for k in 0..=8 {
            seq.push(e.service(k * step).0);
        }
        assert_eq!(
            seq,
            vec![
                (true, true),   // 0 rest
                (false, true),  // 1
                (false, false), // 2
                (true, false),  // 3
                (true, true),   // 4 (detent 1 rest)
                (false, true),  // 5
                (false, false), // 6
                (true, false),  // 7
                (true, true),   // 8 (detent 2 rest)
            ]
        );
        assert_eq!(e.position_detents(), 2);
    }
}
