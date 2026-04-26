// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! TSC — STM32L4 Touch Sensing Controller (RM0351 §21).
//!
//! Register layout:
//!   CR      0x00 — control (TSCE/START/AM/PGPSC/MCV/...)
//!   IER     0x04 — interrupt enable (EOAIE/MCEIE)
//!   ICR     0x08 — interrupt clear (rc_w1)
//!   ISR     0x0C — interrupt status (EOAF/MCEF)
//!   IOHCR   0x10 — I/O hysteresis control
//!   IOASCR  0x18 — I/O analog switch control
//!   IOSCR   0x20 — I/O sampling control (which I/Os are sampling caps)
//!   IOCCR   0x28 — I/O channel control (which I/Os are channels)
//!   IOGCSR  0x30 — I/O group control / status (GxE / GxS)
//!   IOG1CR  0x34 \
//!   IOG2CR  0x38 |
//!   IOG3CR  0x3C |  per-group acquisition counter (read-only)
//!   IOG4CR  0x40 |
//!   IOG5CR  0x44 |
//!   IOG6CR  0x48 |
//!   IOG7CR  0x4C |
//!   IOG8CR  0x50 /
//!
//! Reset values per RM0351 §21.7: all 0.
//!
//! Survival-mode behaviour: writing CR.START while TSCE=1 starts an
//! acquisition. We immediately assert ISR.EOAF (end-of-acquisition flag)
//! so HAL_TSC polling exits, matching what survival fixtures expect on
//! a board with no real touch sensors wired.

use crate::SimResult;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Tsc {
    cr: u32,
    ier: u32,
    isr: u32,
    iohcr: u32,
    ioascr: u32,
    ioscr: u32,
    ioccr: u32,
    iogcsr: u32,
    iog: [u32; 8],
}

impl Tsc {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.ier,
            0x08 => 0, // ICR is write-only
            0x0C => self.isr,
            0x10 => self.iohcr,
            0x18 => self.ioascr,
            0x20 => self.ioscr,
            0x28 => self.ioccr,
            0x30 => self.iogcsr,
            0x34 => self.iog[0],
            0x38 => self.iog[1],
            0x3C => self.iog[2],
            0x40 => self.iog[3],
            0x44 => self.iog[4],
            0x48 => self.iog[5],
            0x4C => self.iog[6],
            0x50 => self.iog[7],
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {
                let was_started = (self.cr & 2) != 0;
                self.cr = value & 0x7FFF_F0FF;
                let now_started = (self.cr & 2) != 0;
                let tsce = (self.cr & 1) != 0;
                if !was_started && now_started && tsce {
                    // START transitioning 0 -> 1. On NUCLEO silicon with
                    // no real touch sensor wiring, the acquisition
                    // immediately hits the max-counter-error condition
                    // (the IOGxCR counter never reaches its threshold),
                    // so silicon asserts BOTH EOAF (end-of-acquisition,
                    // bit 0) and MCEF (max counter error flag, bit 1).
                    // GxS bits in IOGCSR stay CLEAR because the group
                    // didn't complete normally — they would only be set
                    // on a successful sensor read.
                    self.isr |= 0x03; // EOAF + MCEF
                    // START is one-shot: silicon clears it when the
                    // acquisition state machine returns to idle.
                    self.cr &= !2;
                }
            }
            0x04 => self.ier = value & 0x3,
            0x08 => {
                // ICR is rc_w1: clear matching ISR bits.
                self.isr &= !(value & 0x3);
            }
            0x0C => {} // ISR is read-only via firmware writes
            0x10 => self.iohcr = value,
            0x18 => self.ioascr = value,
            0x20 => self.ioscr = value,
            0x28 => self.ioccr = value,
            0x30 => {
                // IOGCSR: low byte (GxE) is r/w, high half (GxS) is read-only.
                self.iogcsr = (self.iogcsr & 0xFFFF_0000) | (value & 0xFF);
            }
            // IOGxCR are read-only acquisition counters
            _ => {}
        }
    }
}

impl crate::Peripheral for Tsc {
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
    use super::Tsc;
    use crate::Peripheral;

    fn write32(t: &mut Tsc, off: u64, val: u32) {
        for i in 0..4 {
            t.write(off + i, ((val >> (i * 8)) & 0xFF) as u8).unwrap();
        }
    }
    fn read32(t: &Tsc, off: u64) -> u32 {
        let mut v = 0u32;
        for i in 0..4 {
            v |= (t.read(off + i).unwrap() as u32) << (i * 8);
        }
        v
    }

    #[test]
    fn test_start_with_tsce_asserts_eoaf_and_mcef() {
        let mut t = Tsc::new();
        // TSCE | START
        write32(&mut t, 0x00, 0x03);
        let isr = read32(&t, 0x0C);
        assert_ne!(isr & 1, 0); // EOAF
        assert_ne!(isr & 2, 0); // MCEF — counter errored (no real sensor)
        // START is one-shot, should self-clear
        let cr = read32(&t, 0x00);
        assert_eq!(cr & 2, 0);
    }

    #[test]
    fn test_iogcsr_gxs_stays_clear_when_acquisition_errors() {
        let mut t = Tsc::new();
        write32(&mut t, 0x30, 0x05); // GxE bits 0 and 2
        write32(&mut t, 0x00, 0x03); // TSCE | START
        let iogcsr = read32(&t, 0x30);
        assert_eq!(iogcsr & 0xFF, 0x05); // GxE preserved
        assert_eq!((iogcsr >> 16) & 0xFF, 0x00); // GxS stays clear (MCEF path)
    }

    #[test]
    fn test_icr_clears_isr() {
        let mut t = Tsc::new();
        write32(&mut t, 0x00, 0x03); // trigger EOAF + MCEF
        write32(&mut t, 0x08, 0x03); // ICR clear both
        let isr = read32(&t, 0x0C);
        assert_eq!(isr & 0x03, 0);
    }
}
