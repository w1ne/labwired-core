// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Bounded SiFive FE310 GPIO model.
//!
//! This implements the software-controlled pad path used by HiFive1 Rev B
//! firmware: INPUT_VAL, INPUT_EN, OUTPUT_EN, OUTPUT_VAL, PULLUP_EN and
//! OUTPUT_XOR. The GPIO interrupt, IO-function mux and drive-strength
//! registers are intentionally outside this profile.

use crate::peripherals::gpio::GpioPortOffsets;
use crate::pins::{External, InputChange, PadDriver, PinPort};
use crate::{Peripheral, SimResult};

const INPUT_VAL: u64 = 0x00;
const INPUT_EN: u64 = 0x04;
const OUTPUT_EN: u64 = 0x08;
const OUTPUT_VAL: u64 = 0x0C;
const PULLUP_EN: u64 = 0x10;
const OUTPUT_XOR: u64 = 0x40;

/// FE310 GPIO register bank with the per-pad input/output behavior needed by
/// the imported HiFive1 Rev B profile.
#[derive(Debug, Default)]
pub struct SifiveGpio {
    input_en: u32,
    output_en: u32,
    output_val: u32,
    pullup_en: u32,
    output_xor: u32,
    external: u32,
    external_levels: u32,
}

impl SifiveGpio {
    pub fn new() -> Self {
        Self::default()
    }

    fn pin_mask(pin: u8) -> u32 {
        1u32 << pin
    }

    fn output_level(&self, pin: u8) -> bool {
        ((self.output_val ^ self.output_xor) & Self::pin_mask(pin)) != 0
    }

    fn pad_level(&self, pin: u8) -> bool {
        let bit = Self::pin_mask(pin);
        if self.output_en & bit != 0 {
            self.output_level(pin)
        } else if self.external & bit != 0 {
            self.external_levels & bit != 0
        } else {
            self.pullup_en & bit != 0
        }
    }

    fn input_value(&self) -> u32 {
        let mut value = 0;
        let mut enabled = self.input_en;
        while enabled != 0 {
            let pin = enabled.trailing_zeros() as u8;
            enabled &= enabled - 1;
            if self.pad_level(pin) {
                value |= Self::pin_mask(pin);
            }
        }
        value
    }

    fn read_register(&self, offset: u64) -> u32 {
        match offset {
            INPUT_VAL => self.input_value(),
            INPUT_EN => self.input_en,
            OUTPUT_EN => self.output_en,
            OUTPUT_VAL => self.output_val,
            PULLUP_EN => self.pullup_en,
            OUTPUT_XOR => self.output_xor,
            // This profile does not claim the FE310 interrupt or IO-function
            // register blocks. Unimplemented/reserved words read as zero.
            _ => 0,
        }
    }

    fn write_register(&mut self, offset: u64, value: u32) {
        match offset {
            // INPUT_VAL is a sampled pad value and is read-only.
            INPUT_EN => self.input_en = value,
            OUTPUT_EN => self.output_en = value,
            OUTPUT_VAL => self.output_val = value,
            PULLUP_EN => self.pullup_en = value,
            OUTPUT_XOR => self.output_xor = value,
            _ => {}
        }
    }

    fn read_input_pin(&self, pin: u8) -> Option<bool> {
        (pin < 32 && self.input_en & Self::pin_mask(pin) != 0)
            .then(|| self.input_value() & Self::pin_mask(pin) != 0)
    }

    fn merge_byte(&mut self, offset: u64, value: u8) {
        let word_offset = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        let mask = 0xFFu32 << shift;
        let old = self.read_register(word_offset);
        self.write_register(word_offset, (old & !mask) | (u32::from(value) << shift));
    }
}

impl Peripheral for SifiveGpio {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let value = self.read_register(offset & !3);
        let shift = ((offset & 3) * 8) as u32;
        Ok(((value >> shift) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.merge_byte(offset, value);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_register(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_register(offset & !3, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let value = self.read_register(offset & !3);
        let shift = ((offset & 3) * 8) as u32;
        Some(((value >> shift) & 0xFF) as u8)
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        self.read_input_pin(pin)
    }

    fn read_gpio_input_word(&self) -> u32 {
        self.input_value()
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        (pin < 32).then(|| self.output_val & Self::pin_mask(pin) != 0)
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        (pin < 32).then(|| self.output_en & Self::pin_mask(pin) != 0)
    }

    fn gpio_port_offsets(&self) -> Option<GpioPortOffsets> {
        Some(GpioPortOffsets {
            output: OUTPUT_VAL,
            input: INPUT_VAL,
        })
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }
}

impl PinPort for SifiveGpio {
    fn pin_count(&self) -> u8 {
        32
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        if pin >= 32 {
            return None;
        }
        let bit = Self::pin_mask(pin);
        Some(if self.output_en & bit != 0 {
            PadDriver::drive(self.output_level(pin))
        } else if self.pullup_en & bit != 0 {
            PadDriver::released(crate::pins::Pull::Up)
        } else {
            PadDriver::OFF
        })
    }

    fn external(&self, pin: u8) -> External {
        if pin >= 32 || self.external & Self::pin_mask(pin) == 0 {
            External::Released
        } else {
            External::Level(self.external_levels & Self::pin_mask(pin) != 0)
        }
    }

    fn set_external(&mut self, pin: u8, external: External) -> Option<InputChange> {
        if pin >= 32 {
            return None;
        }
        let bit = Self::pin_mask(pin);
        let was_enabled = self.input_en & bit != 0;
        let before = self.read_input_pin(pin).unwrap_or(false);
        match external {
            External::Released => {
                self.external &= !bit;
                self.external_levels &= !bit;
            }
            External::Level(level) => {
                self.external |= bit;
                if level {
                    self.external_levels |= bit;
                } else {
                    self.external_levels &= !bit;
                }
            }
        }
        was_enabled.then(|| InputChange {
            before,
            after: self.read_input_pin(pin).unwrap_or(before),
        })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        self.read_input_pin(pin)
    }
}

#[cfg(test)]
mod tests {
    use super::SifiveGpio;
    use crate::pins::{External, PadDriver, PinPort};
    use crate::Peripheral;

    const LED: u32 = 1 << 19;

    #[test]
    fn software_output_and_output_xor_drive_the_pad() {
        let mut gpio = SifiveGpio::new();
        gpio.write_u32(0x08, LED).unwrap();
        gpio.write_u32(0x0C, LED).unwrap();
        assert_eq!(gpio.read_u32(0x0C).unwrap(), LED);
        assert_eq!(gpio.read_gpio_is_output(19), Some(true));
        assert_eq!(gpio.driver(19), Some(PadDriver::drive(true)));

        gpio.write_u32(0x40, LED).unwrap();
        assert_eq!(gpio.read_u32(0x0C).unwrap(), LED);
        assert_eq!(gpio.driver(19), Some(PadDriver::drive(false)));
    }

    #[test]
    fn input_enable_gates_the_sampled_external_level() {
        let mut gpio = SifiveGpio::new();
        assert_eq!(gpio.read_u32(0).unwrap(), 0);
        assert!(gpio.set_external(7, External::Level(true)).is_none());
        gpio.write_u32(0x04, 1 << 7).unwrap();
        assert_eq!(gpio.read_u32(0).unwrap(), 1 << 7);
        assert_eq!(gpio.read_gpio_input(7), Some(true));
        assert_eq!(
            gpio.set_external(7, External::Level(false)).unwrap().after,
            false
        );
        assert_eq!(gpio.read_u32(0).unwrap(), 0);
    }
}
