// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Input-only pin endpoints for simulation profiles whose silicon GPIO
//! registers are not modelled yet.
//!
//! This is an external-input seam, not a GPIO register model. MMIO reads stay
//! zero and writes have no effect, matching the stub window it can replace.
//! Co-simulation can nevertheless drive the endpoint through `PinPort` and
//! observe the thresholded input through the normal `read_gpio_input` hook.

use crate::pins::{External, InputChange, PadDriver, PinPort};
use crate::{Peripheral, SimResult};

/// One simulation-only bank of input pins. `pin_count` is declared by the
/// owning chip profile, so no package pin count or register layout is implied.
#[derive(Debug)]
pub struct InputPinBank {
    pin_count: u8,
    levels: [bool; 32],
    external_mask: u32,
}

impl InputPinBank {
    pub fn new(pin_count: u8) -> anyhow::Result<Self> {
        anyhow::ensure!(
            (1..=32).contains(&pin_count),
            "input_pin_bank pin_count must be in 1..=32, got {pin_count}"
        );
        Ok(Self {
            pin_count,
            levels: [false; 32],
            external_mask: 0,
        })
    }
}

impl PinPort for InputPinBank {
    fn pin_count(&self) -> u8 {
        self.pin_count
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        (pin < self.pin_count).then_some(PadDriver::OFF)
    }

    fn external(&self, pin: u8) -> External {
        if pin >= self.pin_count || self.external_mask & (1 << pin) == 0 {
            External::Released
        } else {
            External::Level(self.levels[pin as usize])
        }
    }

    fn set_external(&mut self, pin: u8, external: External) -> Option<InputChange> {
        if pin >= self.pin_count {
            return None;
        }
        let before = self.levels[pin as usize];
        match external {
            External::Level(level) => {
                self.levels[pin as usize] = level;
                self.external_mask |= 1 << pin;
            }
            // A floating input retains its last sampled level in this
            // simulation endpoint; reset starts low.
            External::Released => self.external_mask &= !(1 << pin),
        }
        Some(InputChange {
            before,
            after: self.levels[pin as usize],
        })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        (pin < self.pin_count).then(|| self.levels[pin as usize])
    }
}

impl Peripheral for InputPinBank {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        self.input(pin)
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        (pin < self.pin_count).then_some(false)
    }

    fn gpio_input_is_released(&self) -> bool {
        true
    }

    fn gpio_routing(&self, pin: u8) -> Option<crate::peripherals::gpio::GpioRouting> {
        (pin < self.pin_count).then_some(crate::peripherals::gpio::GpioRouting {
            mode: crate::peripherals::gpio::GpioMode::Input,
            func: None,
        })
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": "input_pin_bank",
            "pin_count": self.pin_count,
            "levels": self.levels[..self.pin_count as usize],
            "external_mask": self.external_mask,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_bank_accepts_external_levels_and_refuses_pins_outside_its_declared_range() {
        let mut bank = InputPinBank::new(2).unwrap();
        assert_eq!(bank.read_gpio_input(0), Some(false));
        assert_eq!(bank.read_gpio_is_output(0), Some(false));
        assert_eq!(
            bank.set_external(0, External::Level(true)),
            Some(InputChange {
                before: false,
                after: true,
            })
        );
        assert_eq!(bank.read_gpio_input(0), Some(true));
        assert_eq!(bank.set_external(2, External::Level(true)), None);
        assert_eq!(bank.read_gpio_input(2), None);
    }
}
