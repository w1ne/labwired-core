// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! LPTIM — STM32 low-power timer (RM0351 §30).
//!
//! Register file: ISR/ICR/IER/CFGR/CR/CMP/ARR/CNT/CFGR2/OR.
//! All registers reset to 0. CR.ENABLE gates writes to CMP/ARR (RM §30.7.5).
//! ARR/CMP writes set ARROK/CMPOK in ISR; firmware polls these before
//! starting/restarting the counter.

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Lptim {
    isr: u32,
    icr: u32,
    ier: u32,
    cfgr: u32,
    cr: u32,
    cmp: u32,
    arr: u32,
    cnt: u32,
    cfgr2: u32,
    or: u32,
}

impl Lptim {
    pub fn new() -> Self {
        Self {
            isr: 0,
            icr: 0,
            ier: 0,
            cfgr: 0,
            cr: 0,
            cmp: 0,
            arr: 0,
            cnt: 0,
            cfgr2: 0,
            or: 0,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.isr,
            0x04 => self.icr,
            0x08 => self.ier,
            0x0C => self.cfgr,
            0x10 => self.cr,
            0x14 => self.cmp,
            0x18 => self.arr,
            0x1C => self.cnt,
            0x24 => self.cfgr2,
            0x28 => self.or,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x04 => {
                // ICR is rc_w1: clear corresponding bits in ISR
                self.isr &= !value;
                self.icr = 0;
            }
            0x08 => self.ier = value & 0x7F,
            0x0C => self.cfgr = value & 0x01FE_EEDF,
            0x10 => {
                self.cr = value & 0x0F;
                // Disabling clears CNT
                if (self.cr & 0x01) == 0 {
                    self.cnt = 0;
                }
            }
            0x14 => {
                self.cmp = value & 0xFFFF;
                self.isr |= 1 << 3; // CMPOK
            }
            0x18 => {
                self.arr = value & 0xFFFF;
                self.isr |= 1 << 4; // ARROK
            }
            0x1C => {} // CNT is read-only
            0x24 => self.cfgr2 = value & 0x33,
            0x28 => self.or = value & 0x03,
            _ => {}
        }
    }
}

impl Default for Lptim {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for Lptim {
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
