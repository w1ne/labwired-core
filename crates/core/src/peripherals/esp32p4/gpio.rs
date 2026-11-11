//! ESP32-P4 GPIO2 output and IO_MUX subset for the Olimex ESP32-P4-DevKit LED.
//!
//! The P4 places GPIO and IO_MUX in adjacent address windows. This focused
//! model owns both windows so the GPIO2 pad function selected by firmware is
//! checked before its output reaches the board pin. It models the GPIO output
//! latches/enables for both banks, output selectors, and IO_MUX register words;
//! only GPIO2's direct GPIO pad route is electrically resolved. GPIO input
//! routing, interrupts, alternate functions, sleep behavior, and other P4
//! peripherals remain unsupported.

use crate::peripherals::gpio::{GpioMode, GpioRouting};
use crate::pins::{External, InputChange, PadDriver, PinPort};
use crate::{MmioAccessClass, Peripheral, SimResult, SimulationError};

const PIN_COUNT: u8 = 57;
const IOMUX_PIN_COUNT: u8 = 54;
const GPIO2: u8 = 2;
const GPIO_OUT_SELECT: u32 = 256;

const OUT: u64 = 0x04;
const OUT_W1TS: u64 = 0x08;
const OUT_W1TC: u64 = 0x0C;
const OUT1: u64 = 0x10;
const OUT1_W1TS: u64 = 0x14;
const OUT1_W1TC: u64 = 0x18;
const ENABLE: u64 = 0x20;
const ENABLE_W1TS: u64 = 0x24;
const ENABLE_W1TC: u64 = 0x28;
const ENABLE1: u64 = 0x2C;
const ENABLE1_W1TS: u64 = 0x30;
const ENABLE1_W1TC: u64 = 0x34;
const FUNC_OUT_SEL: u64 = 0x558;
const IOMUX_GPIO: u64 = 0x1004;
const IOMUX_MCU_SEL_SHIFT: u32 = 12;

#[derive(Debug)]
pub struct Esp32p4Gpio {
    out: u32,
    out1: u32,
    enable: u32,
    enable1: u32,
    out_sel: [u32; PIN_COUNT as usize],
    iomux: [u32; IOMUX_PIN_COUNT as usize],
}

impl Esp32p4Gpio {
    pub fn new() -> Self {
        Self {
            out: 0,
            out1: 0,
            enable: 0,
            enable1: 0,
            // The P4 SVD reset value selects the GPIO output latch. GPIO_ENABLE
            // is selected separately with OEN_SEL when software configures it.
            out_sel: [GPIO_OUT_SELECT; PIN_COUNT as usize],
            iomux: [0; IOMUX_PIN_COUNT as usize],
        }
    }

    fn pin_enabled(&self, pin: u8) -> bool {
        match pin {
            0..=31 => self.enable & (1 << pin) != 0,
            32..=56 => self.enable1 & (1 << (pin - 32)) != 0,
            _ => false,
        }
    }

    fn pin_latch(&self, pin: u8) -> bool {
        match pin {
            0..=31 => self.out & (1 << pin) != 0,
            32..=56 => self.out1 & (1 << (pin - 32)) != 0,
            _ => false,
        }
    }

    fn gpio2_is_direct_output(&self) -> bool {
        let out_cfg = self.out_sel[GPIO2 as usize];
        let mcu_sel = (self.iomux[GPIO2 as usize] >> IOMUX_MCU_SEL_SHIFT) & 0x7;
        self.pin_enabled(GPIO2)
            && out_cfg & 0x1FF == GPIO_OUT_SELECT
            && out_cfg & (1 << 10) != 0 // OEN_SEL: GPIO_ENABLE_REG drives pad enable.
            && mcu_sel == 1 // GPIO2 function 1 is the GPIO pad function.
    }

    fn read_word(&self, offset: u64) -> Option<u32> {
        Some(match offset {
            OUT | OUT_W1TS | OUT_W1TC => self.out,
            OUT1 | OUT1_W1TS | OUT1_W1TC => self.out1,
            ENABLE | ENABLE_W1TS | ENABLE_W1TC => self.enable,
            ENABLE1 | ENABLE1_W1TS | ENABLE1_W1TC => self.enable1,
            off if (FUNC_OUT_SEL..FUNC_OUT_SEL + PIN_COUNT as u64 * 4).contains(&off) => {
                self.out_sel[((off - FUNC_OUT_SEL) / 4) as usize]
            }
            off if (IOMUX_GPIO..IOMUX_GPIO + IOMUX_PIN_COUNT as u64 * 4).contains(&off) => {
                self.iomux[((off - IOMUX_GPIO) / 4) as usize]
            }
            _ => return None,
        })
    }

    fn write_word(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OUT => self.out = value,
            OUT_W1TS => self.out |= value,
            OUT_W1TC => self.out &= !value,
            OUT1 => self.out1 = value & 0x01FF_FFFF,
            OUT1_W1TS => self.out1 |= value & 0x01FF_FFFF,
            OUT1_W1TC => self.out1 &= !(value & 0x01FF_FFFF),
            ENABLE => self.enable = value,
            ENABLE_W1TS => self.enable |= value,
            ENABLE_W1TC => self.enable &= !value,
            ENABLE1 => self.enable1 = value & 0x01FF_FFFF,
            ENABLE1_W1TS => self.enable1 |= value & 0x01FF_FFFF,
            ENABLE1_W1TC => self.enable1 &= !(value & 0x01FF_FFFF),
            off if (FUNC_OUT_SEL..FUNC_OUT_SEL + PIN_COUNT as u64 * 4).contains(&off) => {
                self.out_sel[((off - FUNC_OUT_SEL) / 4) as usize] = value & 0xFFF;
            }
            off if (IOMUX_GPIO..IOMUX_GPIO + IOMUX_PIN_COUNT as u64 * 4).contains(&off) => {
                self.iomux[((off - IOMUX_GPIO) / 4) as usize] = value;
            }
            _ => {
                return Err(SimulationError::NotImplemented(format!(
                    "ESP32-P4 GPIO focused model: unmapped register offset 0x{offset:X}"
                )))
            }
        }
        Ok(())
    }

    fn read_byte(&self, offset: u64) -> SimResult<u8> {
        let word_offset = offset & !3;
        let word = self.read_word(word_offset).ok_or_else(|| {
            SimulationError::NotImplemented(format!(
                "ESP32-P4 GPIO focused model: unmapped register offset 0x{word_offset:X}"
            ))
        })?;
        Ok((word >> ((offset & 3) * 8)) as u8)
    }

    fn write_byte(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_offset = offset & !3;
        let shift = (offset & 3) * 8;
        if matches!(
            word_offset,
            OUT_W1TS
                | OUT_W1TC
                | OUT1_W1TS
                | OUT1_W1TC
                | ENABLE_W1TS
                | ENABLE_W1TC
                | ENABLE1_W1TS
                | ENABLE1_W1TC
        ) {
            self.write_word(word_offset, (value as u32) << shift)
        } else {
            let mut word = self.read_word(word_offset).ok_or_else(|| {
                SimulationError::NotImplemented(format!(
                    "ESP32-P4 GPIO focused model: unmapped register offset 0x{word_offset:X}"
                ))
            })?;
            word = (word & !(0xFF << shift)) | ((value as u32) << shift);
            self.write_word(word_offset, word)
        }
    }
}

impl Default for Esp32p4Gpio {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32p4Gpio {
    fn mmio_access_class(&self, _offset: u64) -> MmioAccessClass {
        MmioAccessClass::SideEffecting
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        self.read_byte(offset)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read_word(offset & !3)
            .map(|word| (word >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_byte(offset, value)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_word(offset, value)
        } else {
            for i in 0..4 {
                self.write_byte(offset + i, (value >> (i * 8)) as u8)?;
            }
            Ok(())
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "layout": "esp32p4_gpio2_led_subset",
            "out": self.out,
            "enable": self.enable,
            "gpio2_out_sel": self.out_sel[GPIO2 as usize],
            "gpio2_iomux": self.iomux[GPIO2 as usize],
        })
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        (pin == GPIO2 && self.gpio2_is_direct_output()).then(|| self.pin_latch(GPIO2))
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        (pin < PIN_COUNT).then(|| self.pin_latch(pin))
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        if pin != GPIO2 {
            return None;
        }
        let mode = if self.gpio2_is_direct_output() {
            GpioMode::Output
        } else {
            GpioMode::Af
        };
        Some(GpioRouting { mode, func: None })
    }
}

impl PinPort for Esp32p4Gpio {
    fn pin_count(&self) -> u8 {
        PIN_COUNT
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        if pin == GPIO2 && self.gpio2_is_direct_output() {
            Some(PadDriver::drive(self.pin_latch(GPIO2)))
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
        (pin == GPIO2 && self.gpio2_is_direct_output()).then(|| self.pin_latch(GPIO2))
    }
}
