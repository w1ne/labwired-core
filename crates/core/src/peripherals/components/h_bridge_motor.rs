// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Dual-input H-bridge channel twin (L298N / TB6612 / L293D half-bridge).
//!
//! Tracks IN1/IN2 direction and EN (or PWM) enable level. Exposes a signed
//! "effort" in [-1, +1] for UI/oracle: +1 = forward, -1 = reverse, 0 = brake/coast.
//! No motor dynamics — honest for direction + enable labs.

use std::sync::Mutex;

#[derive(Debug, Default)]
struct State {
    in1: bool,
    in2: bool,
    en: bool,
    commanded: bool,
}

/// One H-bridge output channel.
#[derive(Debug)]
pub struct HBridgeMotor {
    in1_pin: u8,
    in2_pin: u8,
    en_pin: Option<u8>,
    state: Mutex<State>,
    id: String,
}

impl HBridgeMotor {
    pub fn new(id: impl Into<String>, in1: u8, in2: u8, en: Option<u8>) -> Self {
        Self {
            in1_pin: in1,
            in2_pin: in2,
            en_pin: en,
            state: Mutex::new(State {
                en: en.is_none(), // no EN pin → always "enabled"
                ..State::default()
            }),
            id: id.into(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Signed effort in [-1.0, 1.0].
    pub fn effort(&self) -> f32 {
        let s = self.state.lock().unwrap();
        if !s.en {
            return 0.0;
        }
        match (s.in1, s.in2) {
            (true, false) => 1.0,
            (false, true) => -1.0,
            _ => 0.0, // brake or coast
        }
    }

    pub fn is_commanded(&self) -> bool {
        self.state.lock().unwrap().commanded
    }

    pub fn on_gpio_edge(&self, pin: u8, to: bool, _sim_cycle: u64) {
        let mut s = self.state.lock().unwrap();
        if pin == self.in1_pin {
            s.in1 = to;
            s.commanded = true;
        } else if pin == self.in2_pin {
            s.in2 = to;
            s.commanded = true;
        } else if self.en_pin == Some(pin) {
            s.en = to;
            s.commanded = true;
        }
    }
}

impl crate::peripherals::esp32s3::gpio::GpioObserver for HBridgeMotor {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_gpio_edge(pin, to, sim_cycle);
    }
}

impl crate::peripherals::esp32::gpio::GpioObserver for HBridgeMotor {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_gpio_edge(pin, to, sim_cycle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_when_in1_high() {
        let m = HBridgeMotor::new("a", 1, 2, Some(3));
        m.on_gpio_edge(3, true, 0);
        m.on_gpio_edge(1, true, 1);
        m.on_gpio_edge(2, false, 2);
        assert_eq!(m.effort(), 1.0);
    }

    #[test]
    fn reverse_when_in2_high() {
        let m = HBridgeMotor::new("a", 1, 2, Some(3));
        m.on_gpio_edge(3, true, 0);
        m.on_gpio_edge(1, false, 1);
        m.on_gpio_edge(2, true, 2);
        assert_eq!(m.effort(), -1.0);
    }
}
