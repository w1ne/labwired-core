// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! WWDG (window watchdog) — STM32 layout, identical across families.
//!
//! Three registers: CR (control + counter), CFR (config + window), SR.
//! Reset values verified against NUCLEO-L476RG silicon:
//!   CR = 0x0000_0074 (T[6:0] = 0x74, default counter value),
//!   CFR = 0x0000_007F (W[6:0] = 0x7F, default window),
//!   SR = 0.

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Wwdg {
    cr: u32,
    cfr: u32,
    sr: u32,
}

impl Wwdg {
    pub fn new() -> Self {
        Self {
            cr: 0x0000_007F,
            cfr: 0x0000_007F,
            sr: 0,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.cfr,
            0x08 => self.sr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr = value & 0xFF,
            0x04 => self.cfr = value & 0xFFFF,
            0x08 => self.sr = value & 1,
            _ => {}
        }
    }
}

impl Default for Wwdg { fn default() -> Self { Self::new() } }

impl crate::Peripheral for Wwdg {
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
