// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 GPIO peripheral model.
//!
//! This follows the C3 GPIO register offsets from
//! `configs/peripherals/esp32c3/gpio.yaml`. The important behavioral delta from
//! the declarative descriptor is W1TS/W1TC side effects: Arduino/ESP-IDF GPIO
//! writes use those set/clear registers, and display peripherals read GPIO
//! output back for CS/DC/bit-banged buses.

use crate::peripherals::gpio::{GpioMode, GpioRouting};
use crate::{Peripheral, PeripheralTickResult, SimResult};

const PIN_COUNT: u8 = 26;
const PIN_MASK: u32 = (1u32 << PIN_COUNT) - 1;

/// `GPIO_FUNCn_OUT_SEL_CFG_REG` base (C3 TRM §5.12): per-PAD output routing.
/// Bits [8:0] select which peripheral output signal drives pad `n`; the sentinel
/// `SIG_GPIO_OUT` (128) means the pad is a plain GPIO output (GPIO_OUT latch).
const FUNC_OUT_SEL: u64 = 0x554;
const SIG_GPIO_OUT: u32 = 128;

/// ESP32-C3 GPIO-matrix OUTPUT signal index → signal name, for the I²C / SPI /
/// UART signals the logic analyzer cares about (from esp-idf
/// `soc/esp32c3/include/soc/gpio_sig_map.h`). Unmapped indices → `None` (null,
/// never a guess).
fn c3_out_signal_name(idx: u32) -> Option<&'static str> {
    Some(match idx {
        6 => "U0TXD",
        9 => "U1TXD",
        53 => "I2CEXT0_SCL",
        54 => "I2CEXT0_SDA",
        63 => "FSPICLK",
        64 => "FSPIQ", // FSPI MISO
        65 => "FSPID", // FSPI MOSI
        68 => "FSPICS0",
        _ => return None,
    })
}

const BT_SELECT: u64 = 0x00;
const OUT: u64 = 0x04;
const OUT_W1TS: u64 = 0x08;
const OUT_W1TC: u64 = 0x0C;
const SDIO_SELECT: u64 = 0x1C;
const ENABLE: u64 = 0x20;
const ENABLE_W1TS: u64 = 0x24;
const ENABLE_W1TC: u64 = 0x28;
/// GPIO_STRAP_REG: latched boot-mode straps. The boot ROM reads bit 3 to choose
/// SPI fast-flash boot; reset seeds it to the board-default flash-boot state.
const STRAP: u64 = 0x38;
const STRAP_SPI_FAST_FLASH_BOOT: u32 = 0x0000_0008;
const IN: u64 = 0x3C;
const STATUS: u64 = 0x44;
const STATUS_W1TS: u64 = 0x48;
const STATUS_W1TC: u64 = 0x4C;
const PCPU_INT: u64 = 0x5C;
const PCPU_NMI_INT: u64 = 0x60;
const CPUSDIO_INT: u64 = 0x64;
const PIN0: u64 = 0x74;

#[derive(Debug)]
pub struct Esp32c3Gpio {
    bt_select: u32,
    out: u32,
    sdio_select: u32,
    enable: u32,
    strap: u32,
    in_data: u32,
    status: u32,
    pin_cfg: [u32; PIN_COUNT as usize],
    /// `GPIO_FUNCn_OUT_SEL_CFG` per pad — the output-matrix selector read by
    /// `gpio_routing` to name the signal a pad is wired to.
    out_sel: [u32; PIN_COUNT as usize],
    cycle: u64,
    anchor_tick: u64,
}

impl Esp32c3Gpio {
    pub fn new() -> Self {
        Self {
            bt_select: 0,
            out: 0,
            sdio_select: 0,
            enable: 0,
            strap: STRAP_SPI_FAST_FLASH_BOOT,
            in_data: 0,
            status: 0,
            pin_cfg: [0; PIN_COUNT as usize],
            out_sel: [0; PIN_COUNT as usize],
            cycle: 0,
            anchor_tick: 0,
        }
    }

    fn out_sel_index(off: u64) -> Option<usize> {
        if (FUNC_OUT_SEL..FUNC_OUT_SEL + (PIN_COUNT as u64) * 4).contains(&off) {
            Some(((off - FUNC_OUT_SEL) / 4) as usize)
        } else {
            None
        }
    }

    pub fn out_value(&self) -> u32 {
        self.out
    }

    pub fn enable_value(&self) -> u32 {
        self.enable
    }

    pub fn set_pin_input(&mut self, pin: u8, level: bool) {
        assert!(pin < PIN_COUNT, "set_pin_input: pin {pin} >= {PIN_COUNT}");
        if level {
            self.in_data |= 1u32 << pin;
        } else {
            self.in_data &= !(1u32 << pin);
        }
    }

    fn pin_cfg_index(off: u64) -> Option<usize> {
        if (PIN0..PIN0 + (PIN_COUNT as u64) * 4).contains(&off) {
            Some(((off - PIN0) / 4) as usize)
        } else {
            None
        }
    }

    fn read_word(&self, word_off: u64) -> u32 {
        match word_off {
            BT_SELECT => self.bt_select,
            OUT | OUT_W1TS | OUT_W1TC => self.out,
            SDIO_SELECT => self.sdio_select,
            ENABLE | ENABLE_W1TS | ENABLE_W1TC => self.enable,
            STRAP => self.strap,
            IN => self.in_data,
            STATUS | STATUS_W1TS | STATUS_W1TC => self.status,
            PCPU_INT | PCPU_NMI_INT | CPUSDIO_INT => self.status,
            off => {
                if let Some(idx) = Self::out_sel_index(off) {
                    self.out_sel[idx]
                } else {
                    Self::pin_cfg_index(off)
                        .map(|idx| self.pin_cfg[idx])
                        .unwrap_or(0)
                }
            }
        }
    }

    fn write_word(&mut self, word_off: u64, value: u32) {
        let value = value & PIN_MASK;
        match word_off {
            BT_SELECT => self.bt_select = value,
            OUT => self.out = value,
            OUT_W1TS => self.out |= value,
            OUT_W1TC => self.out &= !value,
            SDIO_SELECT => self.sdio_select = value,
            ENABLE => self.enable = value,
            ENABLE_W1TS => self.enable |= value,
            ENABLE_W1TC => self.enable &= !value,
            STRAP | IN => {}
            STATUS => self.status = value,
            STATUS_W1TS => self.status |= value,
            STATUS_W1TC => self.status &= !value,
            PCPU_INT | PCPU_NMI_INT | CPUSDIO_INT => {}
            off => {
                if let Some(idx) = Self::out_sel_index(off) {
                    self.out_sel[idx] = value;
                } else if let Some(idx) = Self::pin_cfg_index(off) {
                    self.pin_cfg[idx] = value;
                }
            }
        }
    }

    fn write_byte_special(&mut self, word_off: u64, byte_off: u64, value: u8) -> bool {
        let mask = (value as u32) << (byte_off * 8);
        match word_off {
            OUT_W1TS => self.out |= mask & PIN_MASK,
            OUT_W1TC => self.out &= !(mask & PIN_MASK),
            ENABLE_W1TS => self.enable |= mask & PIN_MASK,
            ENABLE_W1TC => self.enable &= !(mask & PIN_MASK),
            STATUS_W1TS => self.status |= mask & PIN_MASK,
            STATUS_W1TC => self.status &= !(mask & PIN_MASK),
            _ => return false,
        }
        true
    }
}

impl Default for Esp32c3Gpio {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32c3Gpio {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = offset & 3;
        if self.write_byte_special(word_off, byte_off, value) {
            return Ok(());
        }
        let shift = byte_off * 8;
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << shift);
        word |= (value as u32) << shift;
        self.write_word(word_off, word);
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        if offset & 1 == 0 {
            let word_off = offset & !3;
            let shift = (offset & 3) * 8;
            if matches!(
                word_off,
                OUT_W1TS | OUT_W1TC | ENABLE_W1TS | ENABLE_W1TC | STATUS_W1TS | STATUS_W1TC
            ) {
                self.write_word(word_off, (value as u32) << shift);
                return Ok(());
            }
        }
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, (value >> 8) as u8)
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
            "layout": "esp32c3",
            "odr": self.out,
            "idr": self.in_data,
            "enable": self.enable,
            "strap": self.strap,
            "status": self.status,
        })
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        if pin >= PIN_COUNT {
            return None;
        }
        Some((self.in_data & (1u32 << pin)) != 0)
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        if pin >= PIN_COUNT {
            return None;
        }
        Some((self.out & (1u32 << pin)) != 0)
    }

    fn read_gpio_pad(&self, pin: u8) -> Option<bool> {
        if pin >= PIN_COUNT {
            return None;
        }
        let mask = 1u32 << pin;
        // ENABLE is the output driver: enabled pins show the OUT latch,
        // everything else shows the (externally driven) input level.
        Some(if (self.enable & mask) != 0 {
            (self.out & mask) != 0
        } else {
            (self.in_data & mask) != 0
        })
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        if pin >= PIN_COUNT {
            return None;
        }
        let mask = 1u32 << pin;
        if (self.enable & mask) == 0 {
            // Output driver disabled → the pad is an input. The input matrix
            // (FUNCn_IN_SEL, indexed by signal) is not tracked, so we cannot name
            // the signal — func stays None rather than a guess.
            return Some(GpioRouting {
                mode: GpioMode::Input,
                func: None,
            });
        }
        // Output driver enabled: consult the per-pad output-matrix selector.
        let sig = self.out_sel[pin as usize] & 0x1FF;
        if sig == SIG_GPIO_OUT {
            // Pad driven directly by the GPIO_OUT latch — a plain GPIO output.
            Some(GpioRouting {
                mode: GpioMode::Output,
                func: None,
            })
        } else {
            // Pad routed to a peripheral output signal (alternate function).
            Some(GpioRouting {
                mode: GpioMode::Af,
                func: c3_out_signal_name(sig).map(String::from),
            })
        }
    }

    fn set_gpio_input(&mut self, pin: u8, level: bool) -> bool {
        if pin >= PIN_COUNT {
            return false;
        }
        self.set_pin_input(pin, level);
        true
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.cycle = self.cycle.wrapping_add(1);
        PeripheralTickResult::default()
    }

    fn uses_scheduler(&self) -> bool {
        true
    }

    fn sync_to(&mut self, tick_now: u64) {
        if tick_now <= self.anchor_tick {
            return;
        }
        self.cycle = self.cycle.wrapping_add(tick_now - self.anchor_tick);
        self.anchor_tick = tick_now;
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn w1ts_and_w1tc_update_output_register() {
        let mut gpio = Esp32c3Gpio::new();
        gpio.write_u32(OUT_W1TS, (1 << 4) | (1 << 5)).unwrap();
        assert_eq!(gpio.out_value(), (1 << 4) | (1 << 5));

        gpio.write_u32(OUT_W1TC, 1 << 4).unwrap();
        assert_eq!(gpio.out_value(), 1 << 5);
    }

    #[test]
    fn c3_pin_config_uses_c3_pin0_offset() {
        let mut gpio = Esp32c3Gpio::new();
        gpio.write_u32(PIN0 + 3 * 4, 0x2 << 7).unwrap();
        assert_eq!(gpio.read_word(PIN0 + 3 * 4), 0x2 << 7);
        assert_eq!(gpio.read_word(0x88), 0);
    }

    #[test]
    fn reset_strap_selects_spi_flash_boot() {
        let mut gpio = Esp32c3Gpio::new();
        assert_eq!(gpio.read_word(STRAP), STRAP_SPI_FAST_FLASH_BOOT);

        gpio.write_u32(STRAP, 0).unwrap();
        assert_eq!(gpio.read_word(STRAP), STRAP_SPI_FAST_FLASH_BOOT);
    }

    #[test]
    fn gpio_routing_resolves_the_output_matrix() {
        use crate::peripherals::gpio::GpioMode;
        let mut g = Esp32c3Gpio::new();

        // pin5: output driver on + routed to I2CEXT0_SDA (signal index 54) → AF.
        g.write_u32(ENABLE_W1TS, 1 << 5).unwrap();
        g.write_u32(FUNC_OUT_SEL + 5 * 4, 54).unwrap();
        let r5 = g.gpio_routing(5).unwrap();
        assert_eq!(r5.mode, GpioMode::Af);
        assert_eq!(r5.func.as_deref(), Some("I2CEXT0_SDA"));

        // pin6: output driver on + GPIO_OUT sentinel (128) → plain output, no func.
        g.write_u32(ENABLE_W1TS, 1 << 6).unwrap();
        g.write_u32(FUNC_OUT_SEL + 6 * 4, SIG_GPIO_OUT).unwrap();
        let r6 = g.gpio_routing(6).unwrap();
        assert_eq!(r6.mode, GpioMode::Output);
        assert!(r6.func.is_none());

        // pin7: routed to FSPICLK (index 63).
        g.write_u32(ENABLE_W1TS, 1 << 7).unwrap();
        g.write_u32(FUNC_OUT_SEL + 7 * 4, 63).unwrap();
        assert_eq!(g.gpio_routing(7).unwrap().func.as_deref(), Some("FSPICLK"));

        // pin8: output driver off → input, func unknown (input matrix not tracked).
        let r8 = g.gpio_routing(8).unwrap();
        assert_eq!(r8.mode, GpioMode::Input);
        assert!(r8.func.is_none());

        // pin9: enabled but an unmapped signal index → AF with func null (no guess).
        g.write_u32(ENABLE_W1TS, 1 << 9).unwrap();
        g.write_u32(FUNC_OUT_SEL + 9 * 4, 200).unwrap();
        let r9 = g.gpio_routing(9).unwrap();
        assert_eq!(r9.mode, GpioMode::Af);
        assert!(r9.func.is_none());

        assert!(g.gpio_routing(PIN_COUNT).is_none(), "out-of-range pin");
    }
}
