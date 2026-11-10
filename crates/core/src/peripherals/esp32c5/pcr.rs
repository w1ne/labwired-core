// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-C5 PCR slice for UART0, I2C0 and RMT clock configuration.
//!
//! UART0_CONF, UART0_SCLK_CONF, I2C0_CONF, RMT_CONF, RMT_SCLK_CONF and
//! RMT_PD_CTRL are exposed. The function clock sources are stored as register
//! state; the board profile uses PLL_F80M at a fixed 80 MHz and enforces the
//! UART0, I2C0 and RMT APB clock gates. Dynamic clock-tree behavior is outside
//! this slice.

use crate::{Peripheral, PeripheralTickResult, SimResult};

const UART0_CONF: u64 = 0x00;
const UART0_SCLK_CONF: u64 = 0x04;
const I2C0_CONF: u64 = 0x20;
const RMT_CONF: u64 = 0x3C;
const RMT_SCLK_CONF: u64 = 0x40;
const RMT_PD_CTRL: u64 = 0x44;
const UART0_CONF_RESET: u32 = 0x0000_0005; // CLK_EN + READY
const UART0_SCLK_CONF_RESET: u32 = 0x0040_0000; // SCLK_EN, XTAL selected
const RMT_CONF_RESET: u32 = 0x0000_0005; // CLK_EN + READY
const RMT_SCLK_CONF_RESET: u32 = (1 << 20) | (1 << 12); // RC_FAST, divider 1
const RMT_PD_CTRL_RESET: u32 = 1 << 3; // RMT memory force-power-down reset state
const UART0_READY: u32 = 1 << 2;

#[derive(Debug)]
pub struct Esp32c5Pcr {
    uart0_conf: u32,
    uart0_sclk_conf: u32,
    i2c0_conf: u32,
    rmt_conf: u32,
    rmt_sclk_conf: u32,
    rmt_pd_ctrl: u32,
}

impl Default for Esp32c5Pcr {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32c5Pcr {
    pub fn new() -> Self {
        Self {
            uart0_conf: UART0_CONF_RESET & !UART0_READY,
            uart0_sclk_conf: UART0_SCLK_CONF_RESET,
            i2c0_conf: 0,
            rmt_conf: RMT_CONF_RESET & !UART0_READY,
            rmt_sclk_conf: RMT_SCLK_CONF_RESET,
            rmt_pd_ctrl: RMT_PD_CTRL_RESET,
        }
    }

    fn word(&self, offset: u64) -> u32 {
        match offset & !3 {
            UART0_CONF => self.uart0_conf | UART0_READY,
            UART0_SCLK_CONF => self.uart0_sclk_conf,
            I2C0_CONF => self.i2c0_conf,
            RMT_CONF => self.rmt_conf | UART0_READY,
            RMT_SCLK_CONF => self.rmt_sclk_conf,
            RMT_PD_CTRL => self.rmt_pd_ctrl,
            _ => 0,
        }
    }

    /// Resolve the clock-gate register names accepted by the C5 UART slice.
    pub fn gate_reg_offset(name: &str) -> Option<u64> {
        match name.to_ascii_uppercase().as_str() {
            "UART0_CONF" => Some(UART0_CONF),
            "UART0_SCLK_CONF" => Some(UART0_SCLK_CONF),
            "I2C0_CONF" => Some(I2C0_CONF),
            "RMT_CONF" => Some(RMT_CONF),
            _ => None,
        }
    }
}

impl Peripheral for Esp32c5Pcr {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let byte = (offset & 3) * 8;
        Ok(((self.word(offset) >> byte) & 0xff) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_offset = offset & !3;
        let byte = (offset & 3) * 8;
        match word_offset {
            UART0_CONF => {
                self.uart0_conf &= !(0xff << byte);
                self.uart0_conf |= u32::from(value) << byte;
                self.uart0_conf &= !UART0_READY;
            }
            UART0_SCLK_CONF => {
                self.uart0_sclk_conf &= !(0xff << byte);
                self.uart0_sclk_conf |= u32::from(value) << byte;
            }
            I2C0_CONF => {
                self.i2c0_conf &= !(0xff << byte);
                self.i2c0_conf |= u32::from(value) << byte;
            }
            RMT_CONF => {
                self.rmt_conf &= !(0xff << byte);
                self.rmt_conf |= u32::from(value) << byte;
                self.rmt_conf &= !UART0_READY;
            }
            RMT_SCLK_CONF => {
                self.rmt_sclk_conf &= !(0xff << byte);
                self.rmt_sclk_conf |= u32::from(value) << byte;
            }
            RMT_PD_CTRL => {
                self.rmt_pd_ctrl &= !(0xff << byte);
                self.rmt_pd_ctrl |= u32::from(value) << byte;
            }
            _ => {}
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            UART0_CONF => self.uart0_conf = value & !UART0_READY,
            UART0_SCLK_CONF => self.uart0_sclk_conf = value,
            I2C0_CONF => self.i2c0_conf = value,
            RMT_CONF => self.rmt_conf = value & !UART0_READY,
            RMT_SCLK_CONF => self.rmt_sclk_conf = value,
            RMT_PD_CTRL => self.rmt_pd_ctrl = value,
            _ => {}
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn clock_gate_reg_offset(&self, name: &str) -> Option<u64> {
        Self::gate_reg_offset(name)
    }

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}
