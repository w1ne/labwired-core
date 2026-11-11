// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-H2 GPIO direct-output subset for the focused DevKitM-1 smoke profile.
//!
//! This models the GPIO output latch, output enable, GPIO matrix selector 128,
//! and input readback for enabled direct GPIO outputs. Other matrix routes,
//! GPIO inputs, interrupts, open drain, and IO_MUX electrical behavior are not
//! part of this profile.

use crate::peripherals::gpio::{GpioMode, GpioRouting};
use crate::pins::{External, InputChange, PadDriver, PinPort};
use crate::{MmioAccessClass, Peripheral, SimResult};

const PIN_COUNT: u8 = 26;
const PIN_MASK: u32 = (1 << PIN_COUNT) - 1;
const GPIO_OUT_SELECT: u32 = 128;

const OUT: u64 = 0x04;
const OUT_W1TS: u64 = 0x08;
const OUT_W1TC: u64 = 0x0C;
const ENABLE: u64 = 0x20;
const ENABLE_W1TS: u64 = 0x24;
const ENABLE_W1TC: u64 = 0x28;
const IN: u64 = 0x3C;
const FUNC_OUT_SEL: u64 = 0x554;

#[derive(Debug, Default)]
pub struct Esp32h2Gpio {
    out: u32,
    enable: u32,
    out_sel: [u32; PIN_COUNT as usize],
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

    fn direct_output_mask(&self) -> u32 {
        (0..PIN_COUNT).fold(0, |mask, pin| {
            if self.is_direct_output(pin) {
                mask | (1 << pin)
            } else {
                mask
            }
        })
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            OUT | OUT_W1TS | OUT_W1TC => self.out,
            ENABLE | ENABLE_W1TS | ENABLE_W1TC => self.enable,
            IN => self.out & self.direct_output_mask(),
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
            off if (FUNC_OUT_SEL..FUNC_OUT_SEL + PIN_COUNT as u64 * 4).contains(&off) => {
                self.out_sel[((off - FUNC_OUT_SEL) / 4) as usize] = value & 0x1FF;
            }
            _ => {}
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
            "layout": "esp32h2_direct_gpio_output",
            "odr": self.out,
            "enable": self.enable,
        })
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        self.is_direct_output(pin)
            .then(|| self.out & (1 << pin) != 0)
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
        let mode = if self.enable & (1 << pin) == 0 {
            GpioMode::Input
        } else if self.out_sel[pin as usize] & 0x1FF == GPIO_OUT_SELECT {
            GpioMode::Output
        } else {
            GpioMode::Af
        };
        Some(GpioRouting { mode, func: None })
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
