// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C6 analog I²C master (`DR_REG_I2C_ANA_MST_BASE` = `0x600A_F800`).
//!
//! Arduino/IDF `regi2c_*` (and the C6 ROM helpers linked into the sketch)
//! drive RF/PLL sub-blocks through this master after enabling
//! `MODEM_LPCON.clk_i2c_mst_en`. Transactions are modelled as instantaneous:
//!
//! - reads of `I2C0/1_CTRL` clear the BUSY bit (bit 25) so busy-polls exit;
//! - reads of `ANA_CONF0` (`I2C_MST_ANA_CONF0_REG`) force `BBPLL_CAL_DONE`
//!   (bit 24) so PLL cal loops exit;
//! - reads of `I2C_BURST_STATUS` force `BURST_DONE` (bit 0).
//!
//! Every other word is sticky R/W so the enable-block RMW of `ANA_CONF*` sticks.

use crate::{Peripheral, SimResult};

const I2C0_CTRL: u64 = 0x00;
const I2C1_CTRL: u64 = 0x04;
const BURST_STATUS: u64 = 0x14;
const ANA_CONF0: u64 = 0x18;

const BUSY: u32 = 1 << 25;
const BBPLL_CAL_DONE: u32 = 1 << 24;
const BURST_DONE: u32 = 1 << 0;

const WINDOW_BYTES: usize = 1024;

#[derive(Debug)]
pub struct Esp32c6I2cAnaMst {
    regs: Vec<u32>,
}

impl Esp32c6I2cAnaMst {
    pub fn new() -> Self {
        Self {
            regs: vec![0u32; WINDOW_BYTES / 4],
        }
    }
}

impl Default for Esp32c6I2cAnaMst {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32c6I2cAnaMst {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let w = self.read_u32(offset & !3)?;
        Ok((w >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let sh = (offset & 3) * 8;
        // RMW against the raw stored word so forced RO bits on read don't
        // poison subsequent writes (ANA_CONF0 BBPLL_CAL_DONE, CTRL BUSY).
        let raw = *self.regs.get((aligned / 4) as usize).unwrap_or(&0);
        self.write_u32(aligned, (raw & !(0xFFu32 << sh)) | ((value as u32) << sh))
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let stored = *self.regs.get((offset / 4) as usize).unwrap_or(&0);
        Ok(match offset {
            I2C0_CTRL | I2C1_CTRL => stored & !BUSY,
            ANA_CONF0 => stored | BBPLL_CAL_DONE,
            BURST_STATUS => stored | BURST_DONE,
            _ => stored,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
            *slot = value;
        }
        Ok(())
    }

    fn legacy_tick_active(&self) -> bool {
        false
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
    use crate::Peripheral;

    #[test]
    fn ctrl_never_busy() {
        let mut p = Esp32c6I2cAnaMst::new();
        p.write_u32(I2C0_CTRL, BUSY | 0x123).unwrap();
        assert_eq!(p.read_u32(I2C0_CTRL).unwrap() & BUSY, 0);
        assert_eq!(p.read_u32(I2C0_CTRL).unwrap() & 0x123, 0x123);
    }

    #[test]
    fn ana_conf0_reports_bbpll_cal_done() {
        let p = Esp32c6I2cAnaMst::new();
        assert_ne!(p.read_u32(ANA_CONF0).unwrap() & BBPLL_CAL_DONE, 0);
    }

    #[test]
    fn ana_conf_rmw_sticks() {
        let mut p = Esp32c6I2cAnaMst::new();
        // Same RMW pattern regi2c_enable_block uses on ANA_CONF1/2.
        p.write_u32(0x1C, 0x00FF_BFFF).unwrap();
        p.write_u32(0x20, 0x00FF_FDFF).unwrap();
        assert_eq!(p.read_u32(0x1C).unwrap(), 0x00FF_BFFF);
        assert_eq!(p.read_u32(0x20).unwrap(), 0x00FF_FDFF);
    }
}
