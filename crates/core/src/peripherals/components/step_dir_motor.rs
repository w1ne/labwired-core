// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STEP/DIR stepper twin for A4988 / DRV8825 / TMC2209-class drivers.
//!
//! Counts rising edges on STEP while EN is active (low or high per config).
//! DIR selects count direction. Exposes step count and shaft angle for
//! observability / UI. No torque dynamics — faithful for position labs.

use std::sync::Mutex;

#[derive(Debug, Clone, Copy)]
pub struct StepDirConfig {
    /// Degrees of shaft rotation per full step (e.g. 1.8° for NEMA17).
    pub degrees_per_step: f32,
    /// When true, EN low enables the driver (A4988/TMC2209 default).
    pub enable_active_low: bool,
}

impl Default for StepDirConfig {
    fn default() -> Self {
        Self {
            degrees_per_step: 1.8,
            enable_active_low: true,
        }
    }
}

#[derive(Debug, Default)]
struct State {
    step_level: bool,
    dir_level: bool,
    en_level: bool,
    /// Signed step accumulator.
    steps: i64,
    commanded: bool,
}

/// STEP/DIR stepper digital twin.
#[derive(Debug)]
pub struct StepDirMotor {
    step_pin: u8,
    dir_pin: u8,
    en_pin: Option<u8>,
    cfg: StepDirConfig,
    state: Mutex<State>,
    id: String,
}

impl StepDirMotor {
    pub fn new(id: impl Into<String>, step_pin: u8, dir_pin: u8, en_pin: Option<u8>) -> Self {
        Self {
            step_pin,
            dir_pin,
            en_pin,
            cfg: StepDirConfig::default(),
            state: Mutex::new(State::default()),
            id: id.into(),
        }
    }

    pub fn with_config(mut self, cfg: StepDirConfig) -> Self {
        self.cfg = cfg;
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn steps(&self) -> i64 {
        self.state.lock().unwrap().steps
    }

    pub fn angle_degrees(&self) -> f32 {
        let s = self.state.lock().unwrap();
        s.steps as f32 * self.cfg.degrees_per_step
    }

    pub fn is_commanded(&self) -> bool {
        self.state.lock().unwrap().commanded
    }

    pub fn on_gpio_edge(&self, pin: u8, to: bool, _sim_cycle: u64) {
        let mut s = self.state.lock().unwrap();
        if pin == self.step_pin {
            let was = s.step_level;
            s.step_level = to;
            if to && !was {
                // Rising STEP edge
                let enabled = match self.en_pin {
                    None => true,
                    Some(_) => {
                        if self.cfg.enable_active_low {
                            !s.en_level
                        } else {
                            s.en_level
                        }
                    }
                };
                if enabled {
                    if s.dir_level {
                        s.steps += 1;
                    } else {
                        s.steps -= 1;
                    }
                    s.commanded = true;
                }
            }
        } else if pin == self.dir_pin {
            s.dir_level = to;
        } else if self.en_pin == Some(pin) {
            s.en_level = to;
        }
    }
}

impl crate::peripherals::esp32s3::gpio::GpioObserver for StepDirMotor {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_gpio_edge(pin, to, sim_cycle);
    }
}

impl crate::peripherals::esp32::gpio::GpioObserver for StepDirMotor {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_gpio_edge(pin, to, sim_cycle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_increment_on_rising_edge_when_dir_high() {
        let m = StepDirMotor::new("m", 2, 3, Some(4));
        // EN low (active)
        m.on_gpio_edge(4, false, 0);
        m.on_gpio_edge(3, true, 1); // DIR high
        m.on_gpio_edge(2, true, 2);
        m.on_gpio_edge(2, false, 3);
        m.on_gpio_edge(2, true, 4);
        assert_eq!(m.steps(), 2);
        assert!((m.angle_degrees() - 3.6).abs() < 0.01);
    }

    #[test]
    fn disabled_when_en_high_active_low() {
        let m = StepDirMotor::new("m", 2, 3, Some(4));
        m.on_gpio_edge(4, true, 0); // EN high = disabled
        m.on_gpio_edge(2, true, 1);
        assert_eq!(m.steps(), 0);
        assert!(!m.is_commanded());
    }
}
