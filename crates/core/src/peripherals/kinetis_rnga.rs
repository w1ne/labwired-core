// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! NXP Kinetis RNGA (Random Number Generator Accelerator), as on the K64F
//! (`RNG` at `0x4002_9000`, K64 Sub-Family Reference Manual K64P144M120SF5RM,
//! chapter "Random Number Generator Accelerator (RNGA)").
//!
//! Four registers:
//!
//! | off | reg | modelled |
//! |-----|-----|----------|
//! | 0x0 | CR  | GO (sticky), HA (sticky), INTM, CLRI (write-only, clears SR.ERRI), SLP |
//! | 0x4 | SR  | LRS, ORU (clear-on-SR-read), ERRI, SLP (mirrors CR.SLP), OREG_LVL, OREG_SIZE = 1 |
//! | 0x8 | ER  | write-only entropy, folded into the generator state |
//! | 0xC | OR  | read pops one word; empty read returns 0 and flags LRS/ORU/ERRI |
//!
//! The MCUXpresso `RNGA_GetRandomData` that Zephyr's `entropy_mcux_rnga`
//! driver calls spins on `SR.OREG_LVL == 0` before reading `OR`. A passive
//! stub never fills the one-word output register, so every caller of
//! `sys_rand_get()` (Zephyr's network stack seeds TCP ISNs and ephemeral ports
//! from it) hangs.
//!
//! Idealised: silicon refills OR from two free-running ring oscillators a few
//! hundred cycles after each read. Here OR is refilled the moment it is read
//! while `GO = 1` and `SLP = 0`, so `OREG_LVL` reads 1 in normal mode. Samples
//! come from a fixed-seed xorshift32 (perturbed by `ER` writes) so runs are
//! reproducible; they are not entropy. The error interrupt (IRQ 23) is not
//! raised and `SECV` (high-assurance security violation) never sets.

use crate::{Peripheral, SimResult};
use std::any::Any;
use std::cell::Cell;

const CR: u64 = 0x0;
const SR: u64 = 0x4;
const ER: u64 = 0x8;
const OR: u64 = 0xC;

const CR_GO: u32 = 1 << 0;
const CR_HA: u32 = 1 << 1;
const CR_INTM: u32 = 1 << 2;
const CR_CLRI: u32 = 1 << 3;
const CR_SLP: u32 = 1 << 4;
/// GO and HA are sticky: once set, only a reset clears them.
const CR_STICKY: u32 = CR_GO | CR_HA;

const SR_LRS: u32 = 1 << 1;
const SR_ORU: u32 = 1 << 2;
const SR_ERRI: u32 = 1 << 3;
const SR_SLP: u32 = 1 << 4;
const SR_OREG_LVL_1: u32 = 1 << 8;
/// OREG_SIZE is fixed at one word.
const SR_OREG_SIZE_1: u32 = 1 << 16;

const SEED: u32 = 0x6B8B_4567;

#[derive(Debug)]
pub struct KinetisRnga {
    /// GO | HA | INTM | SLP (CLRI reads 0).
    cr: u32,
    /// LRS | ORU | ERRI. Interior-mutable: an OR read sets them and an SR
    /// read clears ORU, both through `&self` bus reads.
    flags: Cell<u32>,
    state: Cell<u32>,
}

impl Default for KinetisRnga {
    fn default() -> Self {
        Self::new()
    }
}

impl KinetisRnga {
    pub fn new() -> Self {
        Self {
            cr: 0,
            flags: Cell::new(0),
            state: Cell::new(SEED),
        }
    }

    fn running(&self) -> bool {
        self.cr & CR_GO != 0 && self.cr & CR_SLP == 0
    }

    fn sr(&self) -> u32 {
        let mut v = self.flags.get() | SR_OREG_SIZE_1;
        if self.cr & CR_SLP != 0 {
            v |= SR_SLP;
        }
        if self.running() {
            v |= SR_OREG_LVL_1;
        }
        v
    }

    fn next_word(&self) -> u32 {
        // xorshift32: never reaches the zero state from a non-zero seed.
        let mut x = self.state.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state.set(x);
        x
    }

    fn read_or(&self) -> u32 {
        if self.running() {
            self.flags.set(self.flags.get() & !SR_LRS);
            self.next_word()
        } else {
            self.flags.set(self.flags.get() | SR_LRS | SR_ORU | SR_ERRI);
            0
        }
    }

    fn write_cr(&mut self, value: u32) {
        if value & CR_CLRI != 0 {
            self.flags.set(self.flags.get() & !SR_ERRI);
        }
        self.cr = (self.cr & CR_STICKY) | (value & (CR_GO | CR_HA | CR_INTM | CR_SLP));
    }

    fn write_er(&mut self, value: u32) {
        let s = self.state.get() ^ value;
        self.state.set(if s == 0 { SEED } else { s });
    }
}

impl Peripheral for KinetisRnga {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !3)?;
        Ok((word >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset & 3) * 8;
        match offset & !3 {
            // Byte stores into CR merge with the current value; CLRI only
            // acts if this byte carries it.
            CR => self.write_cr((self.cr & !(0xFF << shift)) | ((value as u32) << shift)),
            ER => self.write_er((value as u32) << shift),
            _ => {}
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            CR => self.cr,
            SR => {
                let v = self.sr();
                self.flags.set(self.flags.get() & !SR_ORU);
                v
            }
            OR => self.read_or(),
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            CR => self.write_cr(value),
            ER => self.write_er(value),
            _ => {}
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let word = match offset & !3 {
            CR => self.cr,
            SR => self.sr(),
            _ => 0,
        };
        Some((word >> ((offset & 3) * 8)) as u8)
    }

    /// Walk-free: every output is computed on access and there is no
    /// `tick()` override, so the per-cycle walk can change nothing.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "RNGA",
            "cr": self.cr,
            "sr": self.sr(),
            "state": self.state.get(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_state_is_empty_with_one_word_size() {
        let r = KinetisRnga::new();
        assert_eq!(r.read_u32(CR).unwrap(), 0);
        assert_eq!(r.read_u32(SR).unwrap(), SR_OREG_SIZE_1);
    }

    #[test]
    fn go_fills_the_output_register_and_reads_differ() {
        // RNGA_Init: CR = 0, then CR.GO = 1; rnga_ReadEntropy spins on
        // OREG_LVL before reading OR.
        let mut r = KinetisRnga::new();
        r.write_u32(CR, 0).unwrap();
        r.write_u32(CR, CR_GO).unwrap();
        assert_ne!(r.read_u32(SR).unwrap() & 0xFF00, 0);
        let a = r.read_u32(OR).unwrap();
        let b = r.read_u32(OR).unwrap();
        assert_ne!(a, b);
        assert_eq!(r.read_u32(SR).unwrap() & (SR_LRS | SR_ORU | SR_ERRI), 0);
    }

    #[test]
    fn go_and_ha_are_sticky() {
        let mut r = KinetisRnga::new();
        r.write_u32(CR, CR_GO | CR_HA | CR_INTM).unwrap();
        r.write_u32(CR, 0).unwrap();
        assert_eq!(r.read_u32(CR).unwrap(), CR_GO | CR_HA);
    }

    #[test]
    fn sleep_empties_the_output_register() {
        // entropy_mcux_rnga parks the RNGA in sleep between requests.
        let mut r = KinetisRnga::new();
        r.write_u32(CR, CR_GO | CR_SLP).unwrap();
        let sr = r.read_u32(SR).unwrap();
        assert_eq!(sr & 0xFF00, 0);
        assert_ne!(sr & SR_SLP, 0);
        r.write_u32(CR, CR_GO).unwrap();
        assert_eq!(r.read_u32(SR).unwrap() & (0xFF00 | SR_SLP), SR_OREG_LVL_1);
    }

    #[test]
    fn empty_read_flags_underflow_and_sr_read_clears_oru_only() {
        let mut r = KinetisRnga::new();
        assert_eq!(r.read_u32(OR).unwrap(), 0);
        let sr = r.read_u32(SR).unwrap();
        assert_eq!(sr & (SR_LRS | SR_ORU | SR_ERRI), SR_LRS | SR_ORU | SR_ERRI);
        // ORU clears on the SR read; ERRI waits for CLRI; LRS for a valid read.
        assert_eq!(
            r.read_u32(SR).unwrap() & (SR_LRS | SR_ORU | SR_ERRI),
            SR_LRS | SR_ERRI
        );
        r.write_u32(CR, CR_CLRI).unwrap();
        assert_eq!(r.read_u32(SR).unwrap() & SR_ERRI, 0);
        assert_eq!(r.read_u32(CR).unwrap() & CR_CLRI, 0, "CLRI reads 0");
        r.write_u32(CR, CR_GO).unwrap();
        r.read_u32(OR).unwrap();
        assert_eq!(r.read_u32(SR).unwrap() & SR_LRS, 0);
    }

    #[test]
    fn sequence_is_deterministic_and_seeded_by_er() {
        let run = |seed: Option<u32>| {
            let mut r = KinetisRnga::new();
            if let Some(s) = seed {
                r.write_u32(ER, s).unwrap();
            }
            r.write_u32(CR, CR_GO).unwrap();
            (0..4).map(|_| r.read_u32(OR).unwrap()).collect::<Vec<_>>()
        };
        assert_eq!(run(None), run(None));
        assert_eq!(run(Some(1234)), run(Some(1234)));
        assert_ne!(run(None), run(Some(1234)));
    }
}
