// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 clock-and-reset subsystem (CLOCKS, RESETS, PSM, XOSC, PLL_SYS,
//! PLL_USB) as a single MMIO block covering `0x40008000..0x40030000`.
//!
//! The Zephyr / pico-sdk boot path brings clocks up by polling a handful of
//! "ready" bits: `RESETS.RESET_DONE`, `XOSC.STATUS.STABLE`, `PLL.CS.LOCK`, and
//! the glitchless-mux `CLOCKS.CLK_*_SELECTED` registers. This model is a
//! register store with exactly those behavioural bits wired so the polling
//! loops terminate immediately — the sim has no real oscillator to stabilise.
//! Every other register is plain read/write storage so configuration writes
//! (and their read-backs) never fault. RP2040 atomic SET/CLR/XOR aliases are
//! handled one layer up, on the bus, so this model only ever sees aligned
//! base-register accesses.

use std::collections::HashMap;

use crate::{Peripheral, SimResult};

// Absolute base addresses of the sub-blocks this peripheral covers.
const CLOCKS: u64 = 0x4000_8000;
const RESETS: u64 = 0x4000_c000;
const XOSC: u64 = 0x4002_4000;
const PLL_SYS: u64 = 0x4002_8000;
const PLL_USB: u64 = 0x4002_c000;
const ROSC: u64 = 0x4006_0000;

// CLOCKS register offsets (relative to CLOCKS base).
const CLK_REF_CTRL: u64 = 0x30;
const CLK_REF_SELECTED: u64 = 0x38;
const CLK_SYS_CTRL: u64 = 0x3c;
const CLK_SYS_SELECTED: u64 = 0x44;

#[derive(Debug)]
pub struct Rp2040ClockReset {
    base: u64,
    regs: HashMap<u64, u32>,
}

impl Rp2040ClockReset {
    pub fn new(base: u64) -> Self {
        let mut regs = HashMap::new();
        // After reset the glitchless muxes select source 0 (clk_ref←ROSC,
        // clk_sys←clk_ref), so the SELECTED one-hot reads 0x1.
        regs.insert(CLOCKS + CLK_REF_SELECTED, 0x1);
        regs.insert(CLOCKS + CLK_SYS_SELECTED, 0x1);
        // RESETS.RESET comes out of power-on with every peripheral held in
        // reset (the low 25 bits set); RESET_DONE is derived from it on read.
        regs.insert(RESETS, 0x01ff_ffff);
        Self { base, regs }
    }

    fn load(&self, abs: u64) -> u32 {
        self.regs.get(&abs).copied().unwrap_or(0)
    }
}

impl Peripheral for Rp2040ClockReset {
    /// Pure clock/reset register bank — no time-driven tick work.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let abs = self.base + offset;
        let val = match abs {
            // RESETS.RESET_DONE: a peripheral is "done" once its RESET bit is
            // cleared, so DONE is the inverse of RESET (masked to 25 bits).
            x if x == RESETS + 0x8 => !self.load(RESETS) & 0x01ff_ffff,
            // XOSC.STATUS / ROSC.STATUS: STABLE (bit 31) — always "stable".
            x if x == XOSC + 0x4 => self.load(abs) | (1 << 31),
            x if x == ROSC + 0x18 => self.load(abs) | (1 << 31),
            // PLL_SYS/PLL_USB.CS: LOCK (bit 31) — the PLL is always locked.
            x if x == PLL_SYS || x == PLL_USB => self.load(abs) | (1 << 31),
            _ => self.load(abs),
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let abs = self.base + offset;
        // Glitchless muxes: writing CLK_*_CTRL.SRC immediately reflects in the
        // matching CLK_*_SELECTED one-hot, which the driver spins on.
        if abs == CLOCKS + CLK_REF_CTRL {
            let src = value & 0x3;
            self.regs.insert(CLOCKS + CLK_REF_SELECTED, 1 << src);
        } else if abs == CLOCKS + CLK_SYS_CTRL {
            let src = value & 0x1;
            self.regs.insert(CLOCKS + CLK_SYS_SELECTED, 1 << src);
        }
        self.regs.insert(abs, value);
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        let shift = (offset & 0x3) * 8;
        let cur = self.read_u32(aligned)?;
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }
}
