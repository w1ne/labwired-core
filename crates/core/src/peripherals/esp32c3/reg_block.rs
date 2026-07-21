// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic register-backed MMIO block for ESP32-C3 bring-up.
//!
//! A plain read/write register file: writes are stored, reads return the last
//! written value (reset 0). Used for blocks the boot path only configures via
//! read-modify-write and read-back — e.g. the analog I²C master / `ANA_CONFIG`
//! registers at `0x6000_E000` (`DR_REG_RTC_I2C_BASE`) that `rom_i2c_writeReg`
//! drives during PHY/clock bring-up. It is NOT a behavioural model; if a block
//! turns out to need launch/done or busy semantics, promote it to a real model
//! (see [`super::cache`]).

use crate::{Peripheral, SimResult};

#[derive(Debug)]
pub struct Esp32c3RegBlock {
    regs: Vec<u32>,
}

impl Esp32c3RegBlock {
    /// `size_bytes` is rounded up to whole 32-bit words.
    pub fn new(size_bytes: usize) -> Self {
        Self {
            regs: vec![0u32; size_bytes.div_ceil(4)],
        }
    }
}

impl Peripheral for Esp32c3RegBlock {
    // Inert walk: plain register-backed MMIO block; tick() is the trait-default no-op.
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
