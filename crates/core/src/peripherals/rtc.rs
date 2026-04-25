// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! RTC (real-time clock) — STM32L4 layout.
//!
//! L4 RTC is much richer than the F1 RTC: shadow / write-protect /
//! tampering / wakeup units. We model the basic time/date registers
//! (TR/DR/CR/ISR/PRER/WUTR/CALIBR/ALRMAR/ALRMBR), enough for HAL_RTC_Init()
//! to complete and most calendar firmware to read TR/DR.
//!
//! Reset values per RM0351 §38: TR=0, DR=0x2101 (year=00 day=01 month=01
//! weekday=Monday), CR=0, ISR=0x0007 (write flags asserted by default).

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Rtc {
    tr: u32,
    dr: u32,
    cr: u32,
    isr: u32,
    prer: u32,
    wutr: u32,
    alrmar: u32,
    alrmbr: u32,
    wpr: u32,
    write_unlocked: bool,
}

impl Rtc {
    pub fn new() -> Self {
        Self {
            tr: 0,
            dr: 0x0000_2101, // year=00, month=01, day=01, weekday=Mon
            cr: 0,
            // ALRAWF + ALRBWF + WUTWF + RSF (registers synchronised flag,
            // set by hardware once shadow registers track the calendar).
            isr: 0x0000_0027,
            prer: 0x007F_00FF,
            wutr: 0x0000_FFFF,
            alrmar: 0,
            alrmbr: 0,
            wpr: 0,
            write_unlocked: false,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.tr,
            0x04 => self.dr,
            0x08 => self.cr,
            0x0C => self.isr,
            0x10 => self.prer,
            0x14 => self.wutr,
            0x1C => self.alrmar,
            0x20 => self.alrmbr,
            0x24 => self.wpr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // RTC registers (except WPR and BKP) require write-unlock first
            // by writing 0xCA then 0x53 to WPR. We keep the gate but accept
            // unlocked writes to avoid stuck firmware in survival tests.
            0x24 => {
                let key = value & 0xFF;
                if key == 0xFF {
                    self.write_unlocked = false;
                } else if key == 0xCA {
                    // half-unlocked
                    self.wpr = key;
                } else if key == 0x53 && self.wpr == 0xCA {
                    self.write_unlocked = true;
                    self.wpr = 0;
                } else {
                    self.write_unlocked = false;
                    self.wpr = 0;
                }
            }
            // Initialization registers gated by INITF in ISR. For sim we
            // accept writes regardless — most HAL flows polll ISR.RSF.
            0x00 => self.tr = value & 0x007F_7F7F,
            0x04 => self.dr = value & 0x00FF_FF3F,
            0x08 => self.cr = value,
            0x0C => {
                // ISR is rc_w0 for INITF / RSF, rc_w1 for ALRAF / ALRBF
                // status flags. Approximation: clear bits requested.
                self.isr = value;
            }
            0x10 => self.prer = value & 0x007F_7FFF,
            0x14 => self.wutr = value & 0xFFFF,
            0x1C => self.alrmar = value,
            0x20 => self.alrmbr = value,
            _ => {}
        }
    }
}

impl Default for Rtc { fn default() -> Self { Self::new() } }

impl crate::Peripheral for Rtc {
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
