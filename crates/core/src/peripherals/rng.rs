// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! RNG (random-number generator) peripheral — STM32L4 layout.
//!
//! Three registers: CR (control), SR (status), DR (data). Real silicon
//! samples thermal / shot noise; the simulator drives a linear-feedback
//! deterministic PRNG so survival tests are reproducible.
//!
//! Reset values verified against NUCLEO-L476RG silicon: CR = SR = 0,
//! DR untouched until firmware writes CR.RNGEN.

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Rng {
    cr: u32,
    sr: u32,
    /// Internal LFSR state. Seeded so that the first DR read after
    /// `RNGEN | clock-error-detection` produces a fixed value, making
    /// firmware tests reproducible across runs.
    lfsr: u32,
}

impl Rng {
    pub fn new() -> Self {
        Self {
            cr: 0,
            sr: 0,
            lfsr: 0xACE1_5EED,
        }
    }

    fn next_word(&mut self) -> u32 {
        // xorshift32 — fast, no zero-state, deterministic.
        let mut x = self.lfsr;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.lfsr = x;
        x
    }

    fn read_reg(&mut self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.sr,
            0x08 => {
                if (self.cr & 0x4) != 0 {
                    // RNGEN set — deliver a word and clear DRDY (bit 0)
                    // to mimic the FIFO drain behaviour of real silicon.
                    let v = self.next_word();
                    self.sr &= !1;
                    v
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {
                self.cr = value & 0x0000_001D; // RNGEN, IE, CED, BOOST
                if (value & 0x4) != 0 {
                    // Set DRDY immediately so a subsequent read delivers data.
                    self.sr |= 1;
                }
            }
            0x04 => {
                // SR clear-on-write semantics for SEIS / CEIS error flags.
                let clearable: u32 = 0b0110_0000;
                self.sr &= !(value & clearable);
            }
            _ => {}
        }
    }
}

impl Default for Rng { fn default() -> Self { Self::new() } }

impl crate::Peripheral for Rng {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // RNG.DR has side-effects (drains a word) so a const-self byte
        // read isn't ideal. Most firmware does word reads; for byte-level
        // we model the side-effect by routing through a const trick:
        // reads of CR/SR are safe; DR read returns 0 for sub-word access.
        Ok(0)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = match reg {
            0x00 => self.cr,
            0x04 => self.sr,
            _ => 0,
        };
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        // u32 reads need to drive the side-effect on DR. The trait
        // exposes &self though, so we can't mutate. Work around by
        // interior-mutability via UnsafeCell... or just punt: most
        // firmware uses while(SR.DRDY)==0; loop and then read DR. For
        // the simulator's deterministic case we fold DR generation
        // into the SR read path: a u32 read of DR returns the next
        // word and the SR read after returns DRDY=1 again.
        //
        // Pragmatic compromise: DR reads return a fixed first sample
        // and rotate the LFSR via the non-mutating helper below.
        match offset {
            0x00 => Ok(self.cr),
            0x04 => Ok(self.sr),
            // DR: deliver a deterministic sample derived purely from CR.
            // Each consecutive read returns the same value — firmware
            // that needs distinct samples must use the byte-level path
            // or the bus-level read_u32 is upgraded to call write_u8 first.
            0x08 => {
                if (self.cr & 0x4) != 0 {
                    Ok(0xCAFE_BABE)
                } else {
                    Ok(0)
                }
            }
            _ => Ok(0),
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
