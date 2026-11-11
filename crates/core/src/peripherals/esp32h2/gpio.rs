// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-H2 GPIO direct-output and I2C0 matrix subset for the DevKitM-1 profile.
//!
//! This models the GPIO output latch, output enable, GPIO matrix selector 128,
//! and I2CEXT0 matrix selectors 45/46. Other matrix routes, external GPIO inputs,
//! interrupts, open drain, and IO_MUX electrical behavior are not part of this
//! profile.

use crate::peripherals::gpio::{GpioMode, GpioRouting};
use crate::pins::{External, InputChange, PadDriver, PinPort};
use crate::{MmioAccessClass, Peripheral, SimResult};

const PIN_COUNT: u8 = 28;
const PIN_MASK: u32 = (1 << PIN_COUNT) - 1;
const GPIO_OUT_SELECT: u32 = 128;
const I2C0_SCL_SIGNAL: u32 = 45;
const I2C0_SDA_SIGNAL: u32 = 46;

const OUT: u64 = 0x04;
const OUT_W1TS: u64 = 0x08;
const OUT_W1TC: u64 = 0x0C;
const ENABLE: u64 = 0x20;
const ENABLE_W1TS: u64 = 0x24;
const ENABLE_W1TC: u64 = 0x28;
const IN: u64 = 0x3C;
const FUNC45_IN_SEL: u64 = 0x154 + 45 * 4;
const FUNC46_IN_SEL: u64 = 0x154 + 46 * 4;
const FUNC_OUT_SEL: u64 = 0x554;

#[derive(Debug, Default)]
pub struct Esp32h2Gpio {
    out: u32,
    enable: u32,
    out_sel: [u32; PIN_COUNT as usize],
    i2c0_scl_in_sel: u32,
    i2c0_sda_in_sel: u32,
    i2c_matrix_route: crate::peripherals::esp32c3::i2c::C3I2cMatrixRoute,
    i2c_lines: Option<std::sync::Arc<crate::peripherals::esp32c3::i2c::I2cLineLevels>>,
}

impl Esp32h2Gpio {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_direct_output(&self, pin: u8) -> bool {
        pin < PIN_COUNT
            && self.enable & (1 << pin) != 0
            && self.out_sel[pin as usize] & 0x1FF == GPIO_OUT_SELECT
    }

    fn i2c_input_pad(selector: u32) -> Option<u8> {
        // ESP32-H2 GPIO_FUNCn_IN_SEL_CFG: GPIO_SEL[5:0], SIG_IN_SEL[7].
        if selector & (1 << 7) == 0 {
            return None;
        }
        let pin = (selector & 0x3F) as u8;
        (pin < PIN_COUNT).then_some(pin)
    }

    fn refresh_i2c_matrix_route(&self) {
        let mut sda_output_mask = 0;
        let mut scl_output_mask = 0;
        for pin in 0..PIN_COUNT {
            let bit = 1u32 << pin;
            if self.enable & bit == 0 {
                continue;
            }
            let selector = self.out_sel[pin as usize];
            // The H2 OEN_SEL bit routes GPIO_ENABLE to the output pad. This
            // profile does not model the I2C peripheral's alternate OEN path.
            if selector & (1 << 9) == 0 {
                continue;
            }
            match selector & 0xFF {
                I2C0_SCL_SIGNAL => scl_output_mask |= bit,
                I2C0_SDA_SIGNAL => sda_output_mask |= bit,
                _ => {}
            }
        }
        self.i2c_matrix_route
            .lock()
            .expect("ESP32-H2 I2C matrix route poisoned")
            .set(
                sda_output_mask,
                scl_output_mask,
                Self::i2c_input_pad(self.i2c0_sda_in_sel),
                Self::i2c_input_pad(self.i2c0_scl_in_sel),
            );
    }

    fn input_word(&self) -> u32 {
        (0..PIN_COUNT).fold(0, |word, pin| {
            let bit = 1u32 << pin;
            if self.is_direct_output(pin) {
                return word | (self.out & bit);
            }
            if self.enable & bit == 0 {
                return word;
            }
            let selector = self.out_sel[pin as usize];
            if selector & (1 << 9) == 0 {
                return word;
            }
            let level = match selector & 0xFF {
                I2C0_SCL_SIGNAL => self.i2c_lines.as_ref().is_some_and(|lines| lines.scl()),
                I2C0_SDA_SIGNAL => self.i2c_lines.as_ref().is_some_and(|lines| lines.sda()),
                _ => false,
            };
            if level { word | bit } else { word }
        })
    }

    pub(crate) fn i2c_matrix_route_state(
        &self,
    ) -> crate::peripherals::esp32c3::i2c::C3I2cMatrixRoute {
        self.i2c_matrix_route.clone()
    }

    pub(crate) fn set_i2c_lines(
        &mut self,
        lines: std::sync::Arc<crate::peripherals::esp32c3::i2c::I2cLineLevels>,
    ) {
        self.i2c_lines = Some(lines);
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            OUT | OUT_W1TS | OUT_W1TC => self.out,
            ENABLE | ENABLE_W1TS | ENABLE_W1TC => self.enable,
            IN => self.input_word(),
            FUNC45_IN_SEL => self.i2c0_scl_in_sel,
            FUNC46_IN_SEL => self.i2c0_sda_in_sel,
            off if (FUNC_OUT_SEL..FUNC_OUT_SEL + PIN_COUNT as u64 * 4).contains(&off) => {
                self.out_sel[((off - FUNC_OUT_SEL) / 4) as usize]
            }
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let value = value & PIN_MASK;
        match offset {
            OUT => self.out = value,
            OUT_W1TS => self.out |= value,
            OUT_W1TC => self.out &= !value,
            ENABLE => self.enable = value,
            ENABLE_W1TS => self.enable |= value,
            ENABLE_W1TC => self.enable &= !value,
            FUNC45_IN_SEL => self.i2c0_scl_in_sel = value,
            FUNC46_IN_SEL => self.i2c0_sda_in_sel = value,
            off if (FUNC_OUT_SEL..FUNC_OUT_SEL + PIN_COUNT as u64 * 4).contains(&off) => {
                self.out_sel[((off - FUNC_OUT_SEL) / 4) as usize] = value & 0x7FF;
            }
            _ => {}
        }
        if matches!(
            offset,
            ENABLE | ENABLE_W1TS | ENABLE_W1TC | FUNC45_IN_SEL | FUNC46_IN_SEL
        ) || (FUNC_OUT_SEL..FUNC_OUT_SEL + PIN_COUNT as u64 * 4).contains(&offset)
        {
            self.refresh_i2c_matrix_route();
        }
    }
}

impl Peripheral for Esp32h2Gpio {
    fn mmio_access_class(&self, offset: u64) -> MmioAccessClass {
        if offset & !3 == IN {
            MmioAccessClass::SideEffectFree
        } else {
            MmioAccessClass::SideEffecting
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok((word >> ((offset & 3) * 8)) as u8)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let word = self.read_word(offset & !3);
        Some((word >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_offset = offset & !3;
        let shift = (offset & 3) * 8;
        if matches!(word_offset, OUT_W1TS | OUT_W1TC | ENABLE_W1TS | ENABLE_W1TC) {
            self.write_word(word_offset, (value as u32) << shift);
        } else {
            let mut word = self.read_word(word_offset);
            word = (word & !(0xFF << shift)) | ((value as u32) << shift);
            self.write_word(word_offset, word);
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_word(offset, value);
            Ok(())
        } else {
            for i in 0..4 {
                self.write(offset + i, (value >> (i * 8)) as u8)?;
            }
            Ok(())
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "layout": "esp32h2_gpio_direct_output_i2c0_matrix",
            "odr": self.out,
            "enable": self.enable,
            "gpio4_func_out_sel": self.out_sel[4],
            "gpio5_func_out_sel": self.out_sel[5],
            "gpio10_func_out_sel": self.out_sel[10],
            "i2c0_scl_in_sel": self.i2c0_scl_in_sel,
            "i2c0_sda_in_sel": self.i2c0_sda_in_sel,
        })
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        if self.is_direct_output(pin) {
            return Some(self.out & (1 << pin) != 0);
        }
        if pin >= PIN_COUNT || self.enable & (1 << pin) == 0 {
            return None;
        }
        let selector = self.out_sel[pin as usize];
        if selector & (1 << 9) == 0 {
            return None;
        }
        match selector & 0xFF {
            I2C0_SCL_SIGNAL => self.i2c_lines.as_ref().map(|lines| lines.scl()),
            I2C0_SDA_SIGNAL => self.i2c_lines.as_ref().map(|lines| lines.sda()),
            _ => None,
        }
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        (pin < PIN_COUNT).then(|| self.out & (1 << pin) != 0)
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        if pin >= PIN_COUNT {
            return None;
        }
        let signal = self.out_sel[pin as usize] & 0xFF;
        let mode = if self.enable & (1 << pin) == 0 {
            GpioMode::Input
        } else if signal == GPIO_OUT_SELECT {
            GpioMode::Output
        } else {
            GpioMode::Af
        };
        let func = match signal {
            I2C0_SCL_SIGNAL => Some("I2CEXT0_SCL".to_string()),
            I2C0_SDA_SIGNAL => Some("I2CEXT0_SDA".to_string()),
            _ => None,
        };
        Some(GpioRouting { mode, func })
    }
}

impl PinPort for Esp32h2Gpio {
    fn pin_count(&self) -> u8 {
        PIN_COUNT
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        if self.is_direct_output(pin) {
            Some(PadDriver::drive(self.out & (1 << pin) != 0))
        } else if pin < PIN_COUNT {
            Some(PadDriver::OFF)
        } else {
            None
        }
    }

    fn external(&self, _pin: u8) -> External {
        External::Released
    }

    fn set_external(&mut self, _pin: u8, _ext: External) -> Option<InputChange> {
        None
    }

    fn input(&self, pin: u8) -> Option<bool> {
        self.read_gpio_input(pin)
    }

    fn level(&self, pin: u8) -> Option<bool> {
        self.is_direct_output(pin)
            .then(|| self.out & (1 << pin) != 0)
    }
}
