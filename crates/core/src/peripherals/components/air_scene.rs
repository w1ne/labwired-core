// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Shared stimulus for the Leo air-quality sensor models.
//!
//! The demo only lands if the numbers *move*: the headline story is a room
//! filling up — CO₂ climbing from fresh (~450 ppm) toward stuffy (~1400 ppm)
//! as a first-order approach to a target, so the firmware's plain-language
//! verdict flips from "air's good" to "CO₂ climbing, crack a window" live.
//!
//! Each sensor device owns its own [`Ramp`]s seeded from the system-manifest
//! `config:` block. There is no shared clock — a `Ramp` advances one notch each
//! time the firmware takes a fresh measurement from that device, so the values
//! are fully deterministic (no RNG) and reproducible run-to-run.

/// A deterministic first-order approach from `start` toward `target`.
///
/// `value = target + (start - target) * (1 - alpha)^step`. With `alpha` in
/// `(0, 1]`, larger `alpha` climbs faster. `alpha = 0` freezes at `start`
/// (a flat scene); `alpha = 1` jumps straight to `target`.
#[derive(Debug, Clone, Copy)]
pub struct Ramp {
    start: f64,
    target: f64,
    alpha: f64,
    step: u32,
}

impl Ramp {
    pub fn new(start: f64, target: f64, alpha: f64) -> Self {
        Self {
            start,
            target,
            alpha: alpha.clamp(0.0, 1.0),
            step: 0,
        }
    }

    /// A flat ramp that holds `value` forever (for metrics a scenario pins).
    pub fn flat(value: f64) -> Self {
        Self::new(value, value, 0.0)
    }

    /// Current value without advancing.
    pub fn value(&self) -> f64 {
        self.target + (self.start - self.target) * (1.0 - self.alpha).powi(self.step as i32)
    }

    /// Advance one measurement notch and return the new value.
    pub fn advance(&mut self) -> f64 {
        self.step = self.step.saturating_add(1);
        self.value()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_ramp_holds_value() {
        let mut r = Ramp::flat(450.0);
        assert_eq!(r.value(), 450.0);
        assert_eq!(r.advance(), 450.0);
        assert_eq!(r.advance(), 450.0);
    }

    #[test]
    fn ramp_starts_at_start_and_climbs_toward_target() {
        let mut r = Ramp::new(450.0, 1400.0, 0.1);
        assert_eq!(r.value(), 450.0); // step 0 == start
        let first = r.advance();
        assert!(
            first > 450.0 && first < 1400.0,
            "climbs but not past target"
        );
        // Monotonic toward target.
        let mut prev = first;
        for _ in 0..60 {
            let v = r.advance();
            assert!(v >= prev - 1e-9, "must not decrease");
            assert!(v <= 1400.0 + 1e-9, "never overshoots target");
            prev = v;
        }
        assert!(prev > 1300.0, "after many steps it is close to target");
    }

    #[test]
    fn ramp_can_descend_for_dimming_light() {
        let mut r = Ramp::new(800.0, 120.0, 0.15);
        let v = r.advance();
        assert!(v < 800.0 && v > 120.0);
    }

    #[test]
    fn alpha_one_jumps_to_target() {
        let mut r = Ramp::new(0.0, 999.0, 1.0);
        assert_eq!(r.advance(), 999.0);
    }
}
