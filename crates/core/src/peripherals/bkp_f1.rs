// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! BKP (backup registers) — STM32F1 layout (RM0008 §6).
//!
//! The F1 backup domain holds 16-bit data registers that survive a system
//! reset. STM32duino keeps its F1 RTC date there, and bootloaders use them to
//! pass a "stay in DFU" flag across `NVIC_SystemReset()`. A read-as-zero stub
//! breaks both: a value written before the reset reads back 0 afterwards.
//!
//! Register map (offsets from 0x4000_6C00; ST's SVD starts the block at DR1,
//! 0x4000_6C04, which is why this model carries no SVD debug schema):
//!
//! | Offset      | Register              |
//! |-------------|-----------------------|
//! | 0x04..0x28  | DR1..DR10             |
//! | 0x2C        | RTCCR (CAL/CCO/ASOE/ASOS) |
//! | 0x30        | CR (TPE/TPAL)         |
//! | 0x34        | CSR (CTE/CTI/TPIE, TEF/TIF) |
//! | 0x40..0xBC  | DR11..DR42 (high/XL density only) |
//!
//! Low- and medium-density parts (every F103 configured here) have 10 data
//! registers. DR11..DR42 are reserved there and read 0.
//!
//! Write protection: after reset every BKP register is write-protected until
//! firmware sets PWR_CR.DBP (RM0008 §5.1.2). The bus shares DBP with this model
//! through `bind_backup_domain_write_enable`. With no PWR on the bus nothing
//! publishes DBP, so the registers are left writable.
//!
//! Not modelled: the TAMPER pin (a tamper event clears the data registers and
//! sets TEF), the RTC calibration-clock output on the tamper pin, and the
//! backup-domain reset through RCC_BDCR.BDRST.

use crate::SimResult;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const DR_LOW_FIRST: u64 = 0x04;
const DR_LOW_COUNT: usize = 10;
const DR_HIGH_FIRST: u64 = 0x40;
const MAX_DATA_REGISTERS: usize = 42;

const CSR_CTE: u32 = 1 << 0;
const CSR_CTI: u32 = 1 << 1;
const CSR_TPIE: u32 = 1 << 2;
const CSR_TEF: u32 = 1 << 8;
const CSR_TIF: u32 = 1 << 9;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BkpF1 {
    /// DR1..DRn, 16 bits each.
    data: Vec<u16>,
    rtccr: u32,
    cr: u32,
    csr: u32,
    /// PWR_CR.DBP, published by the F1 PWR model.
    #[serde(skip)]
    write_enable: Option<Arc<AtomicBool>>,
}

impl BkpF1 {
    /// `data_registers` is 10 (low/medium density) or 42 (high/XL density).
    pub fn new(data_registers: usize) -> anyhow::Result<Self> {
        if data_registers != DR_LOW_COUNT && data_registers != MAX_DATA_REGISTERS {
            anyhow::bail!(
                "bkp_f1: data_registers must be 10 (low/medium density) or 42 \
                 (high/XL density), got {data_registers}"
            );
        }
        Ok(Self {
            data: vec![0; data_registers],
            rtccr: 0,
            cr: 0,
            csr: 0,
            write_enable: None,
        })
    }

    /// Index into `data` for a data-register offset present on this part.
    fn data_index(&self, offset: u64) -> Option<usize> {
        let index = match offset {
            0x04..=0x28 => ((offset - DR_LOW_FIRST) / 4) as usize,
            0x40..=0xBC => DR_LOW_COUNT + ((offset - DR_HIGH_FIRST) / 4) as usize,
            _ => return None,
        };
        (index < self.data.len()).then_some(index)
    }

    fn writable(&self) -> bool {
        self.write_enable
            .as_ref()
            .is_none_or(|flag| flag.load(Ordering::Relaxed))
    }

    fn read_reg(&self, offset: u64) -> u32 {
        if let Some(i) = self.data_index(offset) {
            return self.data[i] as u32;
        }
        match offset {
            0x2C => self.rtccr,
            0x30 => self.cr,
            0x34 => self.csr,
            _ => {
                crate::census_reg!("bkp_f1:BkpF1", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        if !self.writable() {
            return;
        }
        if let Some(i) = self.data_index(offset) {
            self.data[i] = value as u16;
            return;
        }
        match offset {
            // CAL[6:0], CCO(7), ASOE(8), ASOS(9).
            0x2C => self.rtccr = value & 0x3FF,
            // TPE(0), TPAL(1).
            0x30 => self.cr = value & 0x3,
            // TPIE is the only stored bit; CTE/CTI are write-1 strobes that
            // clear TEF/TIF and read 0. TEF/TIF are hardware status.
            0x34 => {
                if value & CSR_CTE != 0 {
                    self.csr &= !CSR_TEF;
                }
                if value & CSR_CTI != 0 {
                    self.csr &= !CSR_TIF;
                }
                self.csr = (self.csr & !CSR_TPIE) | (value & CSR_TPIE);
            }
            _ => {
                crate::census_reg!("bkp_f1:BkpF1", offset, "write");
            }
        }
    }
}

impl crate::Peripheral for BkpF1 {
    fn bind_backup_domain_write_enable(&mut self, enabled: Arc<AtomicBool>) {
        self.write_enable = Some(enabled);
    }

    // A system reset leaves the backup domain alone (RM0008 §6.1): the default
    // `on_system_reset` (keep state) is the silicon behaviour.

    // Inert walk: pure register bank, no tick() behaviour.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    fn unlocked(n: usize) -> (BkpF1, Arc<AtomicBool>) {
        let mut bkp = BkpF1::new(n).unwrap();
        let dbp = Arc::new(AtomicBool::new(true));
        bkp.bind_backup_domain_write_enable(dbp.clone());
        (bkp, dbp)
    }

    #[test]
    fn reset_values_are_zero() {
        let bkp = BkpF1::new(10).unwrap();
        for off in (0x00..0x100).step_by(4) {
            assert_eq!(bkp.read_u32(off).unwrap(), 0, "offset {off:#x}");
        }
    }

    #[test]
    fn data_registers_are_16_bit_at_rm0008_offsets() {
        let (mut bkp, _) = unlocked(10);
        bkp.write_u32(0x04, 0xDEAD_BEEF).unwrap(); // DR1
        bkp.write_u32(0x28, 0x1234).unwrap(); // DR10
        assert_eq!(bkp.read_u32(0x04).unwrap(), 0xBEEF, "DR1 keeps 16 bits");
        assert_eq!(bkp.read_u32(0x28).unwrap(), 0x1234, "DR10");
        // ST's SVD offset 0 is NOT DR1 on silicon.
        assert_eq!(bkp.read_u32(0x00).unwrap(), 0, "0x00 is reserved");
    }

    #[test]
    fn medium_density_has_no_dr11() {
        let (mut bkp, _) = unlocked(10);
        bkp.write_u32(0x40, 0x5555).unwrap();
        assert_eq!(
            bkp.read_u32(0x40).unwrap(),
            0,
            "DR11 reserved on 10-DR parts"
        );
        let (mut hd, _) = unlocked(42);
        hd.write_u32(0x40, 0x5555).unwrap(); // DR11
        hd.write_u32(0xBC, 0xAAAA).unwrap(); // DR42
        assert_eq!(hd.read_u32(0x40).unwrap(), 0x5555);
        assert_eq!(hd.read_u32(0xBC).unwrap(), 0xAAAA);
    }

    #[test]
    fn writes_are_dropped_until_dbp_is_set() {
        let (mut bkp, dbp) = unlocked(10);
        dbp.store(false, Ordering::Relaxed);
        bkp.write_u32(0x04, 0x00A5).unwrap();
        bkp.write_u32(0x2C, 0x7F).unwrap();
        assert_eq!(bkp.read_u32(0x04).unwrap(), 0, "DR1 locked");
        assert_eq!(bkp.read_u32(0x2C).unwrap(), 0, "RTCCR locked");
        dbp.store(true, Ordering::Relaxed);
        bkp.write_u32(0x04, 0x00A5).unwrap();
        assert_eq!(bkp.read_u32(0x04).unwrap(), 0x00A5, "DR1 after DBP");
    }

    #[test]
    fn system_reset_keeps_the_backup_domain() {
        let (mut bkp, _) = unlocked(10);
        bkp.write_u32(0x04, 0x4242).unwrap();
        bkp.write_u32(0x2C, 0x07F).unwrap();
        bkp.on_system_reset();
        assert_eq!(bkp.read_u32(0x04).unwrap(), 0x4242);
        assert_eq!(bkp.read_u32(0x2C).unwrap(), 0x07F);
    }

    #[test]
    fn control_register_masks() {
        let (mut bkp, _) = unlocked(10);
        bkp.write_u32(0x2C, 0xFFFF_FFFF).unwrap();
        bkp.write_u32(0x30, 0xFFFF_FFFF).unwrap();
        bkp.write_u32(0x34, 0xFFFF_FFFF).unwrap();
        assert_eq!(bkp.read_u32(0x2C).unwrap(), 0x3FF, "RTCCR");
        assert_eq!(bkp.read_u32(0x30).unwrap(), 0x3, "CR");
        assert_eq!(bkp.read_u32(0x34).unwrap(), CSR_TPIE, "CSR: CTE/CTI read 0");
    }

    #[test]
    fn rejects_an_unknown_data_register_count() {
        assert!(BkpF1::new(20).is_err());
    }
}
