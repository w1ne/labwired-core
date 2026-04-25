// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! IWDG (independent watchdog) — STM32 layout, identical across families.
//!
//! Four registers: KR (key), PR (prescaler), RLR (reload), SR (status),
//! WINR (windowed mode). Real silicon resets the chip if the watchdog
//! isn't kicked within the configured timeout — our simulator just
//! latches the writes and never resets, since survival tests need
//! deterministic completion.
//!
//! Reset values verified against NUCLEO-L476RG silicon:
//!   KR = 0, PR = 0, RLR = 0x0FFF (default reload = max 12-bit value),
//!   SR = 0.

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Iwdg {
    kr: u32,
    pr: u32,
    rlr: u32,
    sr: u32,
    winr: u32,
}

impl Iwdg {
    pub fn new() -> Self {
        Self { kr: 0, pr: 0, rlr: 0x0000_0FFF, sr: 0, winr: 0x0000_0FFF }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.kr,
            0x04 => self.pr,
            0x08 => self.rlr,
            0x0C => self.sr,
            0x10 => self.winr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // KR is write-only. Specific magic values do specific things;
            // we just latch for readback observability.
            0x00 => self.kr = value & 0xFFFF,
            0x04 => self.pr = value & 0x7,
            0x08 => self.rlr = value & 0xFFF,
            0x0C => {} // SR is read-only
            0x10 => self.winr = value & 0xFFF,
            _ => {}
        }
    }
}

impl Default for Iwdg { fn default() -> Self { Self::new() } }

impl crate::Peripheral for Iwdg {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
