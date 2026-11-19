// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow GPIO data/enable/input heads for four Espressif parts that previously
//! had UART-only descriptors and no GPIO model.
//!
//! Two official register layouts are modeled, and nothing past the input words:
//!
//! * ESP32-S2 and ESP32-P4 share the classic / S3 *head* (OUT @0x04, ENABLE
//!   @0x20, STRAP @0x38, IN @0x3C, IN1 @0x40). The window ends at IN1 (68
//!   bytes). GPIO matrix, PIN_x, IO_MUX and interrupts are not claimed.
//! * ESP32-C5 and ESP32-C61 share a later head (STRAP @0x00, OUT @0x04,
//!   ENABLE @0x34, IN @0x64, IN1 @0x68). The window ends at IN1 (108 bytes).
//!   That ENABLE/IN placement does *not* match `esp32c3_gpio` (ENABLE @0x20,
//!   IN @0x3C), so those parts do not reuse the C3 type.
//!
//! W1TS/W1TC semantics follow the rest of the ESP32 GPIO family. Pad
//! electricals, IO_MUX pulls, and matrix routing are outside this profile.
//!
//! Sources (ESP-IDF `master`, checked 2026-11-17):
//!   S2 `DR_REG_GPIO_BASE` 0x3F40_4000, OUT1 field 0x003F_FFFF, pin count 47:
//!     components/soc/esp32s2/register/soc/{reg_base,gpio_reg}.h
//!     components/soc/esp32s2/include/soc/soc_caps.h
//!   C5 `DR_REG_GPIO_BASE` 0x6009_1000, pin count 29, ENABLE1 field 1 bit:
//!     components/soc/esp32c5/register/soc/{reg_base,gpio_reg}.h
//!     components/soc/esp32c5/include/soc/soc_caps.h
//!   C61 `DR_REG_GPIO_BASE` 0x6009_1000, pin count 30, ENABLE1 field 2 bits:
//!     components/soc/esp32c61/register/soc/{reg_base,gpio_reg}.h
//!     components/soc/esp32c61/include/soc/soc_caps.h
//!   P4 `DR_REG_GPIO_BASE` = `DR_REG_HPPERIPH1_BASE`(0x500C_0000)+0x20000
//!     = 0x500E_0000, pin count 55, ENABLE1 field 0x01FF_FFFF (hw_ver3):
//!     components/soc/esp32p4/register/hw_ver3/soc/{reg_base,gpio_reg}.h
//!     components/soc/esp32p4/include/soc/soc_caps.h

use crate::peripherals::gpio::{GpioMode, GpioRouting};
use crate::pins::{External, InputChange, PinPort};
use crate::{Peripheral, SimResult};
use labwired_config::PeripheralConfig;
use std::sync::{Arc, Mutex};

/// Official data/enable/input offsets and pin-count / bank-1 field width.
#[derive(Debug, Clone, Copy)]
struct Layout {
    name: &'static str,
    pin_count: u8,
    bank0_mask: u32,
    bank1_mask: u32,
    strap: u64,
    out: u64,
    out_w1ts: u64,
    out_w1tc: u64,
    out1: u64,
    out1_w1ts: u64,
    out1_w1tc: u64,
    enable: u64,
    enable_w1ts: u64,
    enable_w1tc: u64,
    enable1: u64,
    enable1_w1ts: u64,
    enable1_w1tc: u64,
    input: u64,
    input1: u64,
}

impl Layout {
    /// Classic / S3-style head used by the S2 and P4 through IN1 @0x40.
    const fn classic_head(name: &'static str, pin_count: u8, bank1_mask: u32) -> Self {
        let bank0_mask = Self::bank0_mask(pin_count);
        Self {
            name,
            pin_count,
            bank0_mask,
            bank1_mask,
            strap: 0x38,
            out: 0x04,
            out_w1ts: 0x08,
            out_w1tc: 0x0C,
            out1: 0x10,
            out1_w1ts: 0x14,
            out1_w1tc: 0x18,
            enable: 0x20,
            enable_w1ts: 0x24,
            enable_w1tc: 0x28,
            enable1: 0x2C,
            enable1_w1ts: 0x30,
            enable1_w1tc: 0x34,
            input: 0x3C,
            input1: 0x40,
        }
    }

    /// C5 / C61 head: ENABLE @0x34, IN @0x64, IN1 @0x68.
    const fn c5_family(name: &'static str, pin_count: u8, bank1_mask: u32) -> Self {
        let bank0_mask = Self::bank0_mask(pin_count);
        Self {
            name,
            pin_count,
            bank0_mask,
            bank1_mask,
            strap: 0x00,
            out: 0x04,
            out_w1ts: 0x08,
            out_w1tc: 0x0C,
            out1: 0x10,
            out1_w1ts: 0x14,
            out1_w1tc: 0x18,
            enable: 0x34,
            enable_w1ts: 0x38,
            enable_w1tc: 0x3C,
            enable1: 0x40,
            enable1_w1ts: 0x44,
            enable1_w1tc: 0x48,
            input: 0x64,
            input1: 0x68,
        }
    }

    const fn bank0_mask(pin_count: u8) -> u32 {
        if pin_count >= 32 {
            u32::MAX
        } else {
            (1u32 << pin_count) - 1
        }
    }

    fn pin_mask(&self, pin: u8) -> Option<(bool, u32)> {
        if pin >= self.pin_count {
            return None;
        }
        if pin < 32 {
            Some((false, 1u32 << pin))
        } else {
            Some((true, 1u32 << (pin - 32)))
        }
    }

    fn is_w1t(&self, off: u64) -> bool {
        matches!(
            off,
            o if o == self.out_w1ts
                || o == self.out_w1tc
                || o == self.out1_w1ts
                || o == self.out1_w1tc
                || o == self.enable_w1ts
                || o == self.enable_w1tc
                || o == self.enable1_w1ts
                || o == self.enable1_w1tc
        )
    }
}

/// SOC_GPIO_PIN_COUNT 47; official OUT1 field is 22 bits (gpio_reg.h).
const S2: Layout = Layout::classic_head("esp32s2", 47, 0x003F_FFFF);
/// SOC_GPIO_PIN_COUNT 55; official ENABLE1 field is 25 bits (hw_ver3).
const P4: Layout = Layout::classic_head("esp32p4", 55, 0x01FF_FFFF);
/// SOC_GPIO_PIN_COUNT 29; official ENABLE1 field is 1 bit.
const C5: Layout = Layout::c5_family("esp32c5", 29, 0x0000_0001);
/// SOC_GPIO_PIN_COUNT 30; official ENABLE1 field is 2 bits.
const C61: Layout = Layout::c5_family("esp32c61", 30, 0x0000_0003);

/// Narrow two-bank GPIO data/enable/input head.
#[derive(Debug)]
pub struct EspGpioHead {
    layout: Layout,
    out0: u32,
    out1: u32,
    enable0: u32,
    enable1: u32,
    /// Last external level presented to each bank, valid where `ext_mask` is set.
    ext0: u32,
    ext1: u32,
    ext_mask0: u32,
    ext_mask1: u32,
    // P4 exposes just the four GPIO-matrix registers used to route HP I2C0
    // through GPIO7/8 in the board onboarding profile. Values and signal IDs
    // come from P4 hw_ver3/gpio_reg.h and include/soc/gpio_sig_map.h.
    p4_i2c_scl_in: u32,
    p4_i2c_sda_in: u32,
    p4_i2c_gpio7_out: u32,
    p4_i2c_gpio8_out: u32,
    p4_i2c_route: Option<crate::peripherals::esp32c3::i2c::C3I2cMatrixRoute>,
}

impl EspGpioHead {
    fn new(layout: Layout) -> Self {
        Self {
            layout,
            out0: 0,
            out1: 0,
            enable0: 0,
            enable1: 0,
            ext0: 0,
            ext1: 0,
            ext_mask0: 0,
            ext_mask1: 0,
            p4_i2c_scl_in: 0x3F,
            p4_i2c_sda_in: 0x3F,
            p4_i2c_gpio7_out: 256,
            p4_i2c_gpio8_out: 256,
            p4_i2c_route: (layout.name == "esp32p4").then(|| {
                Arc::new(Mutex::new(
                    crate::peripherals::esp32c3::i2c::C3I2cMatrixRouteState::default(),
                ))
            }),
        }
    }

    pub fn esp32s2() -> Self {
        Self::new(S2)
    }

    pub fn esp32p4() -> Self {
        Self::new(P4)
    }

    pub fn esp32c5() -> Self {
        Self::new(C5)
    }

    pub fn esp32c61() -> Self {
        Self::new(C61)
    }

    /// P4 HP I2C0's matrix state, shared with its command-list controller.
    pub(crate) fn i2c_matrix_route_state(
        &self,
    ) -> Option<crate::peripherals::esp32c3::i2c::C3I2cMatrixRoute> {
        self.p4_i2c_route.clone()
    }

    fn refresh_p4_i2c_route(&mut self) {
        let Some(route) = &self.p4_i2c_route else {
            return;
        };
        let sda_out = (if self.p4_i2c_gpio7_out & 0x1FF == 69 {
            1 << 7
        } else {
            0
        }) | (if self.p4_i2c_gpio8_out & 0x1FF == 69 {
            1 << 8
        } else {
            0
        });
        let scl_out = (if self.p4_i2c_gpio7_out & 0x1FF == 68 {
            1 << 7
        } else {
            0
        }) | (if self.p4_i2c_gpio8_out & 0x1FF == 68 {
            1 << 8
        } else {
            0
        });
        let input_pad = |word: u32| match word & 0x3F {
            7 => Some(7),
            8 => Some(8),
            _ => None,
        };
        let mut route = route.lock().expect("ESP32-P4 I2C matrix route poisoned");
        route.set(
            sda_out,
            scl_out,
            input_pad(self.p4_i2c_sda_in),
            input_pad(self.p4_i2c_scl_in),
        );
    }

    fn out_word(&self, bank1: bool) -> u32 {
        if bank1 {
            self.out1 & self.layout.bank1_mask
        } else {
            self.out0 & self.layout.bank0_mask
        }
    }

    fn enable_word(&self, bank1: bool) -> u32 {
        if bank1 {
            self.enable1 & self.layout.bank1_mask
        } else {
            self.enable0 & self.layout.bank0_mask
        }
    }

    /// GPIO_IN / GPIO_IN1: an externally driven pad shows that level; an
    /// enabled output loops back its latch; otherwise the pad reads low.
    /// IO_MUX pulls are not part of this profile.
    fn input_word(&self, bank1: bool) -> u32 {
        let out = self.out_word(bank1);
        let enable = self.enable_word(bank1);
        let (ext, ext_mask, mask) = if bank1 {
            (self.ext1, self.ext_mask1, self.layout.bank1_mask)
        } else {
            (self.ext0, self.ext_mask0, self.layout.bank0_mask)
        };
        let driven = (out & enable) | (ext & ext_mask);
        driven & mask
    }

    fn write_out(&mut self, bank1: bool, value: u32) {
        if bank1 {
            self.out1 = value & self.layout.bank1_mask;
        } else {
            self.out0 = value & self.layout.bank0_mask;
        }
    }

    fn write_enable(&mut self, bank1: bool, value: u32) {
        if bank1 {
            self.enable1 = value & self.layout.bank1_mask;
        } else {
            self.enable0 = value & self.layout.bank0_mask;
        }
    }

    fn read_word(&self, off: u64) -> u32 {
        let l = self.layout;
        if l.name == "esp32p4" {
            match off {
                0x268 => return self.p4_i2c_scl_in,
                0x26C => return self.p4_i2c_sda_in,
                0x574 => return self.p4_i2c_gpio7_out,
                0x578 => return self.p4_i2c_gpio8_out,
                _ => {}
            }
        }
        if off == l.out {
            self.out_word(false)
        } else if off == l.out1 {
            self.out_word(true)
        } else if off == l.enable {
            self.enable_word(false)
        } else if off == l.enable1 {
            self.enable_word(true)
        } else if off == l.input {
            self.input_word(false)
        } else if off == l.input1 {
            self.input_word(true)
        } else {
            // STRAP and W1T registers are RAZ on read (writes to W1T are
            // handled in write_word). Unmapped offsets in this window are RAZ too.
            0
        }
    }

    fn write_word(&mut self, off: u64, value: u32) {
        let l = self.layout;
        if l.name == "esp32p4" {
            let matrix_write = match off {
                // GPIO_FUNC68/69_IN_SEL_CFG: selection, inversion, and bypass.
                0x268 => {
                    self.p4_i2c_scl_in = value & 0xFF;
                    true
                }
                0x26C => {
                    self.p4_i2c_sda_in = value & 0xFF;
                    true
                }
                // GPIO_FUNC7/8_OUT_SEL_CFG: signal plus output/OEN selection.
                0x574 => {
                    self.p4_i2c_gpio7_out = value & 0xFFF;
                    true
                }
                0x578 => {
                    self.p4_i2c_gpio8_out = value & 0xFFF;
                    true
                }
                _ => false,
            };
            if matrix_write {
                self.refresh_p4_i2c_route();
                return;
            }
        }
        if off == l.out {
            self.write_out(false, value);
        } else if off == l.out_w1ts {
            self.write_out(false, self.out0 | value);
        } else if off == l.out_w1tc {
            self.write_out(false, self.out0 & !value);
        } else if off == l.out1 {
            self.write_out(true, value);
        } else if off == l.out1_w1ts {
            self.write_out(true, self.out1 | value);
        } else if off == l.out1_w1tc {
            self.write_out(true, self.out1 & !value);
        } else if off == l.enable {
            self.write_enable(false, value);
        } else if off == l.enable_w1ts {
            self.write_enable(false, self.enable0 | value);
        } else if off == l.enable_w1tc {
            self.write_enable(false, self.enable0 & !value);
        } else if off == l.enable1 {
            self.write_enable(true, value);
        } else if off == l.enable1_w1ts {
            self.write_enable(true, self.enable1 | value);
        } else if off == l.enable1_w1tc {
            self.write_enable(true, self.enable1 & !value);
        } else if off == l.strap || off == l.input || off == l.input1 {
            // STRAP and IN are read-only in this profile.
        }
    }

    fn pin_is_output(&self, pin: u8) -> Option<bool> {
        let (bank1, mask) = self.layout.pin_mask(pin)?;
        Some((self.enable_word(bank1) & mask) != 0)
    }
}

impl PinPort for EspGpioHead {
    fn pin_count(&self) -> u8 {
        self.layout.pin_count
    }

    fn driver(&self, pin: u8) -> Option<crate::pins::PadDriver> {
        let (bank1, mask) = self.layout.pin_mask(pin)?;
        if (self.enable_word(bank1) & mask) == 0 {
            Some(crate::pins::PadDriver::released(crate::pins::Pull::None))
        } else {
            Some(crate::pins::PadDriver::output(
                (self.out_word(bank1) & mask) != 0,
                false,
            ))
        }
    }

    fn external(&self, pin: u8) -> External {
        match self.layout.pin_mask(pin) {
            Some((bank1, mask)) => {
                let (ext, ext_mask) = if bank1 {
                    (self.ext1, self.ext_mask1)
                } else {
                    (self.ext0, self.ext_mask0)
                };
                if ext_mask & mask != 0 {
                    External::Level(ext & mask != 0)
                } else {
                    External::Released
                }
            }
            None => External::Released,
        }
    }

    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        let (bank1, mask) = self.layout.pin_mask(pin)?;
        let before = (self.input_word(bank1) & mask) != 0;
        let (ext_reg, ext_mask_reg) = if bank1 {
            (&mut self.ext1, &mut self.ext_mask1)
        } else {
            (&mut self.ext0, &mut self.ext_mask0)
        };
        match ext {
            External::Level(level) => {
                *ext_mask_reg |= mask;
                if level {
                    *ext_reg |= mask;
                } else {
                    *ext_reg &= !mask;
                }
            }
            External::Released => {
                *ext_mask_reg &= !mask;
                *ext_reg &= !mask;
            }
        }
        Some(InputChange {
            before,
            after: (self.input_word(bank1) & mask) != 0,
        })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        Peripheral::read_gpio_input(self, pin)
    }
}

impl Peripheral for EspGpioHead {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let shift = (offset & 3) * 8;
        Ok(((self.read_word(word_off) >> shift) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let shift = (offset & 3) * 8;
        let contributed = (value as u32) << shift;
        if self.layout.is_w1t(word_off) {
            self.write_word(word_off, contributed);
        } else {
            let mut word = self.read_word(word_off);
            word &= !(0xFFu32 << shift);
            word |= contributed;
            self.write_word(word_off, word);
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_word(offset, value);
            Ok(())
        } else {
            for i in 0..4 {
                self.write(offset + i, ((value >> (i * 8)) & 0xFF) as u8)?;
            }
            Ok(())
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "layout": self.layout.name,
            "out0": self.out_word(false),
            "out1": self.out_word(true),
            "enable0": self.enable_word(false),
            "enable1": self.enable_word(true),
            "in0": self.input_word(false),
            "in1": self.input_word(true),
        })
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        let (bank1, mask) = self.layout.pin_mask(pin)?;
        Some((self.input_word(bank1) & mask) != 0)
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        let (bank1, mask) = self.layout.pin_mask(pin)?;
        Some((self.out_word(bank1) & mask) != 0)
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        let is_output = self.pin_is_output(pin)?;
        Some(GpioRouting {
            mode: if is_output {
                GpioMode::Output
            } else {
                GpioMode::Input
            },
            func: None,
        })
    }
}

pub fn try_build(canonical_type: &str, _p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    let head = match canonical_type {
        "esp32s2_gpio" => EspGpioHead::esp32s2(),
        "esp32p4_gpio" => EspGpioHead::esp32p4(),
        "esp32c5_gpio" => EspGpioHead::esp32c5(),
        "esp32c61_gpio" => EspGpioHead::esp32c61(),
        _ => return None,
    };
    Some(Box::new(head))
}

pub const SUPPORTED_TYPES: &[&str] = &[
    "esp32s2_gpio",
    "esp32p4_gpio",
    "esp32c5_gpio",
    "esp32c61_gpio",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_w1ts_loopback(gpio: &mut EspGpioHead, pin: u8) {
        let (bank1, mask) = gpio.layout.pin_mask(pin).expect("pin in range");
        let out = if bank1 {
            gpio.layout.out1
        } else {
            gpio.layout.out
        };
        let out_w1ts = if bank1 {
            gpio.layout.out1_w1ts
        } else {
            gpio.layout.out_w1ts
        };
        let out_w1tc = if bank1 {
            gpio.layout.out1_w1tc
        } else {
            gpio.layout.out_w1tc
        };
        let enable_w1ts = if bank1 {
            gpio.layout.enable1_w1ts
        } else {
            gpio.layout.enable_w1ts
        };
        let input = if bank1 {
            gpio.layout.input1
        } else {
            gpio.layout.input
        };

        gpio.write_u32(out_w1ts, mask).unwrap();
        gpio.write_u32(enable_w1ts, mask).unwrap();
        assert_eq!(gpio.read_u32(out).unwrap() & mask, mask, "OUT latch");
        assert_eq!(gpio.read_u32(input).unwrap() & mask, mask, "IN loops back");
        assert_eq!(gpio.read_gpio_output(pin), Some(true));
        assert_eq!(gpio.gpio_routing(pin).unwrap().mode, GpioMode::Output);

        gpio.write_u32(out_w1tc, mask).unwrap();
        assert_eq!(gpio.read_u32(out).unwrap() & mask, 0);
        assert_eq!(gpio.read_u32(input).unwrap() & mask, 0);
    }

    #[test]
    fn c5_enable_and_in_are_not_the_c3_offsets() {
        let mut gpio = EspGpioHead::esp32c5();
        let mask = 1u32 << 11;
        // A C3-shaped model would accept ENABLE_W1TS at 0x24 and IN at 0x3C.
        gpio.write_u32(0x24, mask).unwrap();
        gpio.write_u32(0x08, mask).unwrap();
        assert_eq!(gpio.read_u32(0x20).unwrap(), 0, "C3 ENABLE @0x20 is unused");
        assert_eq!(gpio.read_u32(0x3C).unwrap(), 0, "C3 IN @0x3C is unused");
        assert_w1ts_loopback(&mut gpio, 11);
        assert_eq!(gpio.layout.enable, 0x34);
        assert_eq!(gpio.layout.input, 0x64);
        assert_eq!(gpio.read_u32(0x34).unwrap() & mask, mask);
        assert_eq!(gpio.pin_count(), 29);
    }

    #[test]
    fn c61_uart_rx_pin_is_gpio10() {
        let mut gpio = EspGpioHead::esp32c61();
        assert_eq!(gpio.pin_count(), 30);
        assert_w1ts_loopback(&mut gpio, 10);
        assert_w1ts_loopback(&mut gpio, 11);
        assert_eq!(gpio.layout.enable, 0x34);
        assert_eq!(gpio.layout.input, 0x64);
    }

    #[test]
    fn s2_uart_pins_live_in_bank1() {
        let mut gpio = EspGpioHead::esp32s2();
        assert_eq!(gpio.pin_count(), 47);
        assert_eq!(gpio.layout.enable, 0x20);
        assert_eq!(gpio.layout.input, 0x3C);
        assert_w1ts_loopback(&mut gpio, 43);
        // GPIO43 is OUT1 bit 11 (cleared again by the helper). Re-set it and
        // check the bank-1 word at the official OUT1 offset.
        gpio.write_u32(0x14, 1 << 11).unwrap();
        gpio.write_u32(0x30, 1 << 11).unwrap();
        assert_eq!(gpio.read_u32(0x10).unwrap() & (1 << 11), 1 << 11);
        assert_w1ts_loopback(&mut gpio, 44);
    }

    #[test]
    fn p4_uart_pins_live_in_bank1() {
        let mut gpio = EspGpioHead::esp32p4();
        assert_eq!(gpio.pin_count(), 55);
        assert_eq!(gpio.layout.enable, 0x20);
        assert_w1ts_loopback(&mut gpio, 37);
        gpio.write_u32(0x14, 1 << 5).unwrap();
        gpio.write_u32(0x30, 1 << 5).unwrap();
        assert_eq!(gpio.read_u32(0x10).unwrap() & (1 << 5), 1 << 5);
        assert_w1ts_loopback(&mut gpio, 38);
    }

    fn dummy_cfg() -> PeripheralConfig {
        PeripheralConfig {
            id: "gpio".into(),
            r#type: "esp32s2_gpio".into(),
            base_address: 0,
            size: None,
            irq: None,
            irq_controller: None,
            clock: None,
            config: Default::default(),
        }
    }

    #[test]
    fn factory_rejects_unknown_types() {
        let cfg = dummy_cfg();
        assert!(try_build("esp32c3_gpio", &cfg).is_none());
        assert!(try_build("esp32s2_gpio", &cfg).is_some());
        assert!(try_build("esp32c5_gpio", &cfg).is_some());
        assert!(try_build("esp32c61_gpio", &cfg).is_some());
        assert!(try_build("esp32p4_gpio", &cfg).is_some());
    }
}
