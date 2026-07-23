// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! 4-wire unipolar stepper (28BYJ-48 + ULN2003) twin.
//!
//! Watches IN1–IN4 coil phases and advances a half-step position when the
//! driven pattern changes. 28BYJ-48 gearing: ~2048 half-steps / rev (common
//! clone mapping) → ~0.176° / half-step.

use std::sync::Mutex;

#[derive(Debug, Default)]
struct State {
    phases: [bool; 4],
    half_steps: i64,
    last_idx: Option<i8>,
    commanded: bool,
}

/// Half-step sequence index for 28BYJ-48 (IN1..IN4).
const HALF_SEQ: [[bool; 4]; 8] = [
    [true, false, false, false],
    [true, true, false, false],
    [false, true, false, false],
    [false, true, true, false],
    [false, false, true, false],
    [false, false, true, true],
    [false, false, false, true],
    [true, false, false, true],
];

#[derive(Debug)]
pub struct UnipolarStepper {
    pins: [u8; 4],
    /// Degrees per half-step (default 28BYJ-48 ~ 360/2048).
    deg_per_half: f32,
    state: Mutex<State>,
    id: String,
}

impl UnipolarStepper {
    pub fn new_28byj48(id: impl Into<String>, pins: [u8; 4]) -> Self {
        Self {
            pins,
            deg_per_half: 360.0 / 2048.0,
            state: Mutex::new(State::default()),
            id: id.into(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn half_steps(&self) -> i64 {
        self.state.lock().unwrap().half_steps
    }

    pub fn angle_degrees(&self) -> f32 {
        self.half_steps() as f32 * self.deg_per_half
    }

    pub fn is_commanded(&self) -> bool {
        self.state.lock().unwrap().commanded
    }

    fn seq_index(phases: [bool; 4]) -> Option<i8> {
        HALF_SEQ.iter().position(|&p| p == phases).map(|i| i as i8)
    }

    pub fn on_gpio_edge(&self, pin: u8, to: bool, _sim_cycle: u64) {
        let mut s = self.state.lock().unwrap();
        let mut hit = false;
        for (i, &p) in self.pins.iter().enumerate() {
            if p == pin {
                s.phases[i] = to;
                hit = true;
            }
        }
        if !hit {
            return;
        }
        if let Some(idx) = Self::seq_index(s.phases) {
            if let Some(prev) = s.last_idx {
                let mut delta = idx - prev;
                if delta > 4 {
                    delta -= 8;
                }
                if delta < -4 {
                    delta += 8;
                }
                if delta != 0 {
                    s.half_steps += delta as i64;
                    s.commanded = true;
                }
            }
            s.last_idx = Some(idx);
        }
    }
}

impl crate::peripherals::esp32s3::gpio::GpioObserver for UnipolarStepper {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_gpio_edge(pin, to, sim_cycle);
    }
}

impl crate::peripherals::esp32::gpio::GpioObserver for UnipolarStepper {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_gpio_edge(pin, to, sim_cycle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_step_sequence_advances() {
        let m = UnipolarStepper::new_28byj48("m", [1, 2, 3, 4]);
        // Walk first three half-steps
        let seq = [
            [true, false, false, false],
            [true, true, false, false],
            [false, true, false, false],
        ];
        for (t, phases) in seq.iter().enumerate() {
            for (i, &on) in phases.iter().enumerate() {
                m.on_gpio_edge(i as u8 + 1, on, t as u64);
            }
        }
        assert!(m.half_steps() >= 1);
        assert!(m.is_commanded());
    }
}
