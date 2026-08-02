// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 analog I²C master / ANA_CONFIG block (`0x6000_E000`,
//! `DR_REG_I2C_ANA_MST_BASE` / `DR_REG_RTC_I2C_BASE`).
//!
//! `rom_i2c_writeReg`/`readReg` and the libphy RF calibration drive this analog
//! master to reach the RF/PLL sub-blocks. After a transaction is launched (a
//! write to the command register at offset `0x5C`), the ROM busy-polls the
//! status word at offset `0x50` and waits for bits[26:24] to read 7 — the
//! analog-master FSM returning to idle/done. A plain register-backed stub
//! returns the last written value there, which never becomes 7, so the ROM
//! clock/PLL bring-up (called from the WiFi PHY calibration) spins forever.
//!
//! We model transactions as instantaneous: reads of `0x50` always report the
//! FSM-idle state (bits[26:24] = 7). Every other register is register-backed
//! (writes stored, reads return the last value), preserving the read-modify-
//! write behaviour `rom_i2c_*` relies on.

use crate::{Peripheral, SimResult};

/// Status/command-FSM register; bits[26:24] = master state (7 = idle/done).
const STATUS: u64 = 0x50;
const STATE_DONE: u32 = 0x7 << 24;
/// A second command/cal register (offset 0x4C): the libphy RF calibration
/// (`txdc_cal`) launches a transaction here then busy-polls bit24 for "done".
const CAL_CMD: u64 = 0x4C;
const CAL_DONE: u32 = 1 << 24;

#[derive(Debug)]
pub struct Esp32c3AnaI2c {
    regs: Vec<u32>,
}

impl Default for Esp32c3AnaI2c {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32c3AnaI2c {
    pub fn new() -> Self {
        Self {
            regs: vec![0u32; 0x400 / 4],
        }
    }
}

impl Peripheral for Esp32c3AnaI2c {
    // Inert walk: register-backed analog-I²C master; transactions complete at the write (FSM-done forced on read); tick() is the trait-default no-op.
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
        let stored = *self.regs.get((offset / 4) as usize).unwrap_or(&0);
        Ok(match offset {
            // Report the analog-master FSM as idle/done so the ROM's
            // busy-poll (`(reg>>24)&7 == 7`) exits immediately.
            STATUS => (stored & !STATE_DONE) | STATE_DONE,
            // Report the calibration transaction as complete (bit24) so the
            // libphy txdc_cal busy-poll exits — the RF air-gap cut: we don't
            // model physical RF, so each cal completes instantaneously.
            CAL_CMD => stored | CAL_DONE,
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

    #[test]
    fn status_reports_fsm_idle() {
        let a = Esp32c3AnaI2c::new();
        assert_eq!((a.read_u32(STATUS).unwrap() >> 24) & 7, 7, "FSM idle/done");
    }

    #[test]
    fn other_registers_are_register_backed() {
        let mut a = Esp32c3AnaI2c::new();
        a.write_u32(0x5C, 0xDEAD_BEEF).unwrap();
        assert_eq!(a.read_u32(0x5C).unwrap(), 0xDEAD_BEEF);
    }
}
