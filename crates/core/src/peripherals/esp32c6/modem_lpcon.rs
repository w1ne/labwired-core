// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! MODEM_LPCON sticky register bank (ESP32-C6).
//!
//! Fast-boot skips the mask ROM that leaves `clk_i2c_mst_en` set. Arduino
//! `regi2c_enable_block` then asserts `regi2c_ctrl_ll_master_is_clock_enabled()`
//! (`MODEM_LPCON.clk_conf` @ +0x18 bit 2) and aborts. This block is a plain
//! sticky register file seeded with that bit so clock bring-up can proceed;
//! other fields start at 0 and accept read-modify-write.

use crate::{Peripheral, SimResult};

/// `clk_conf` word offset (`modem_lpcon_struct.h`: 7th register).
pub const CLK_CONF_OFFSET: u64 = 0x18;
/// `clk_i2c_mst_en` within `clk_conf`.
pub const CLK_I2C_MST_EN: u32 = 1 << 2;

/// Window size matching the chip yaml (`2KB`).
/// Covers MODEM_LPCON SVD regs (@+0x00..+0x44) plus the coex HW clear
/// window Arduino `coex_hw_clear_reg` addresses as base+0x400 (gap before
/// I2C_ANA_MST @ 0x600AF800).
const WINDOW_BYTES: usize = 2048;

#[derive(Debug)]
pub struct Esp32c6ModemLpcon {
    regs: Vec<u32>,
}

impl Esp32c6ModemLpcon {
    pub fn new() -> Self {
        let mut regs = vec![0u32; WINDOW_BYTES / 4];
        // Mask-ROM leave-on: analog I2C master clock enabled.
        regs[(CLK_CONF_OFFSET / 4) as usize] = CLK_I2C_MST_EN;
        Self { regs }
    }
}

impl Default for Esp32c6ModemLpcon {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32c6ModemLpcon {
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
        let cur = self.read_u32(aligned)?;
        self.write_u32(aligned, (cur & !(0xFFu32 << sh)) | ((value as u32) << sh))
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(*self.regs.get((offset / 4) as usize).unwrap_or(&0))
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
    fn seeds_i2c_mst_clock_enable() {
        let p = Esp32c6ModemLpcon::new();
        assert_eq!(p.read_u32(CLK_CONF_OFFSET).unwrap() & CLK_I2C_MST_EN, CLK_I2C_MST_EN);
    }

    #[test]
    fn writes_stick() {
        let mut p = Esp32c6ModemLpcon::new();
        p.write_u32(0x00, 0xA5A5_A5A5).unwrap();
        assert_eq!(p.read_u32(0x00).unwrap(), 0xA5A5_A5A5);
    }
}
