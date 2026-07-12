// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! NXP Kinetis MCG (Multipurpose Clock Generator) — behavioural model.
//!
//! The KW41Z / Kinetis-L MCG is a bank of 8-bit control registers (C1..C8) plus
//! a read-only status register `S`. Real NXP firmware (the MCUXpresso
//! `fsl_clock.c` `CLOCK_SetFeeMode` / `CLOCK_SetXxxMode` helpers) selects a
//! clock mode by writing C1/C2/C7 and then **spins on MCG_S** until the mode has
//! "settled":
//!
//! ```text
//!   while ((MCG->S & MCG_S_IREFST_MASK) != expected) {}   // S bit4  ← C1[IREFS]
//!   while (((MCG->S & MCG_S_CLKST_MASK) >> 2) != expected) {} // S bits3:2 ← C1[CLKS]
//!   while (!(MCG->S & MCG_S_OSCINIT0_MASK)) {}             // S bit1, external OSC ready
//! ```
//!
//! A passive register bank holds `S` at its reset value (`0x10`) forever, so the
//! firmware hangs. This model recomputes `S` from the control bits on every
//! write — exactly the way [`crate::peripherals::rcc`] auto-sets the STM32 RDY
//! flags. The FLL and the (RSIM-gated) reference clock are modelled as settling
//! instantaneously; this part has **no PLL** (the SVD `MCG_S` has no LOCK0 /
//! PLLST fields), so there is no lock counter to emulate.
//!
//! Register offsets / reset values are the public CMSIS-SVD values
//! (`cmsis-svd-data: data/NXP/MKW41Z4.svd`), matching
//! `configs/peripherals/mkw41z4/mcg.yaml`.

use crate::{Peripheral, SimResult};
use std::any::Any;

// 8-bit register offsets within the MCG block.
const C1: usize = 0x00;
const C2: usize = 0x01;
const S: usize = 0x06;
/// C1..C8 occupy 0x00..=0x0D; back the whole contiguous span.
const NUM_REGS: usize = 0x0E;

// MCG_C1 fields.
const C1_IREFS: u8 = 1 << 2; // Internal Reference Select (1 = internal FLL ref)
const C1_CLKS_SHIFT: u8 = 6; // Clock Source Select [7:6]

// MCG_C2 fields.
const C2_IRCS: u8 = 1 << 0; // Internal Reference Clock Select (0 = slow, 1 = fast)
const C2_EREFS: u8 = 1 << 2; // External Reference Select (1 = crystal osc)

// MCG_S fields.
const S_IRCST: u8 = 1 << 0; // Internal Reference Clock Status
const S_OSCINIT0: u8 = 1 << 1; // OSC Initialization
const S_CLKST_SHIFT: u8 = 2; // Clock Mode Status [3:2]
const S_CLKST_MASK: u8 = 0b11 << S_CLKST_SHIFT;
const S_IREFST: u8 = 1 << 4; // Internal Reference Status

/// Behavioural Kinetis MCG. Holds the control byte registers and a derived
/// status byte that mirrors the requested clock configuration.
#[derive(Debug)]
pub struct Mcg {
    regs: [u8; NUM_REGS],
}

impl Default for Mcg {
    fn default() -> Self {
        Self::new()
    }
}

impl Mcg {
    pub fn new() -> Self {
        let mut regs = [0u8; NUM_REGS];
        // SVD reset values (data/NXP/MKW41Z4.svd).
        regs[C1] = 0x04; // IREFS=1, CLKS=00 → FEI (FLL engaged, internal ref)
        regs[C2] = 0xC0; // LOCRE0 | FCFTRIM
        regs[S] = 0x10; // IREFST=1 (internal ref), CLKST=00 (FLL), OSCINIT0=0
        regs[0x08] = 0x02; // SC: FCRDIV=001
        regs[0x0D] = 0x80; // C8: LOCRE1
        let mut m = Self { regs };
        m.recompute_status();
        m
    }

    /// Settle MCG_S from the current control bits, the way the silicon clock FSM
    /// does after a mode change. No PLL, no lock delay — each requested source
    /// is treated as immediately stable.
    fn recompute_status(&mut self) {
        let c1 = self.regs[C1];
        let c2 = self.regs[C2];
        // Preserve any reserved/high bits; recompute only the modelled fields.
        let mut s = self.regs[S] & !(S_IRCST | S_OSCINIT0 | S_CLKST_MASK | S_IREFST);

        // IREFST (bit4): the FLL reference is the internal ref iff C1[IREFS]=1.
        // FEE clears IREFS → IREFST goes 0, which is what CLOCK_SetFeeMode waits
        // for.
        if c1 & C1_IREFS != 0 {
            s |= S_IREFST;
        }

        // IRCST (bit0): reflects the fast/slow IRC actually selected (C2[IRCS]).
        if c2 & C2_IRCS != 0 {
            s |= S_IRCST;
        }

        // OSCINIT0 (bit1): the external oscillator reports "initialised" once it
        // is selected as the external reference (C2[EREFS]=1). In external-clock
        // mode (EREFS=0) it is never set — and the SDK only polls it when
        // EREFS=1, so mirroring EREFS is both correct and sufficient.
        if c2 & C2_EREFS != 0 {
            s |= S_OSCINIT0;
        }

        // CLKST (bits3:2) tracks the requested clock source C1[CLKS], each
        // modelled as immediately ready: 00 FLL, 01 internal ref, 10 external
        // ref, 11 reserved.
        let clks = (c1 >> C1_CLKS_SHIFT) & 0b11;
        s |= clks << S_CLKST_SHIFT;

        self.regs[S] = s;
    }
}

impl Peripheral for Mcg {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.regs.get(offset as usize).copied().unwrap_or(0))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let off = offset as usize;
        if off >= NUM_REGS {
            return Ok(());
        }
        // MCG_S is hardware status: read-only, never written by the CPU.
        if off == S {
            return Ok(());
        }
        self.regs[off] = value;
        // Any control write can change the settled status.
        self.recompute_status();
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.regs.get(offset as usize).copied()
    }

    /// Walk-free (kw41z): the MCG is purely combinational — `MCG_S` is
    /// recomputed from the control bits on every write (`recompute_status`) and
    /// there is NO `tick()`/`tick_elapsed()` override, so the model has zero
    /// time-dependent state. The clock modes settle instantaneously (no PLL, no
    /// lock counter), so the per-cycle walk can never change any observable
    /// output. Proven inert (`needs_legacy_walk() == false`) by the differential
    /// gate `crates/core/tests/kw41z_walk_free_differential.rs`.
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
        serde_json::json!({ "peripheral": "MCG", "regs": self.regs.to_vec() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_state_is_fei() {
        let mcg = Mcg::new();
        // FEI: internal ref selected (IREFST=1), FLL output (CLKST=00).
        assert_eq!(mcg.read(S as u64).unwrap(), 0x10);
        assert_eq!(mcg.read(C1 as u64).unwrap(), 0x04);
    }

    #[test]
    fn fee_transition_clears_irefst_and_keeps_clkst_fll() {
        // CLOCK_SetFeeMode writes C1 = CLKS(0) | FRDIV(5) | IREFS(0) = 0x28,
        // then waits IREFST==0 and CLKST==00.
        let mut mcg = Mcg::new();
        mcg.write(C1 as u64, 0x28).unwrap();
        let s = mcg.read(S as u64).unwrap();
        assert_eq!(s & S_IREFST, 0, "IREFST must clear when IREFS=0 (external)");
        assert_eq!(s & S_CLKST_MASK, 0, "CLKST stays 00 (FLL output) in FEE");
    }

    #[test]
    fn external_clock_select_sets_clkst_external() {
        // FBE/BLPE: CLKS=10 (external ref). CLKST must follow to 10.
        let mut mcg = Mcg::new();
        mcg.write(C1 as u64, 0b10 << C1_CLKS_SHIFT).unwrap();
        let s = mcg.read(S as u64).unwrap();
        assert_eq!((s & S_CLKST_MASK) >> S_CLKST_SHIFT, 0b10);
    }

    #[test]
    fn erefs_sets_oscinit0() {
        // Selecting the crystal oscillator (C2[EREFS]=1) reports OSCINIT0.
        let mut mcg = Mcg::new();
        assert_eq!(mcg.read(S as u64).unwrap() & S_OSCINIT0, 0);
        mcg.write(C2 as u64, 0xC0 | C2_EREFS).unwrap();
        assert_eq!(mcg.read(S as u64).unwrap() & S_OSCINIT0, S_OSCINIT0);
    }

    #[test]
    fn status_register_is_read_only() {
        let mut mcg = Mcg::new();
        mcg.write(S as u64, 0xFF).unwrap();
        assert_eq!(mcg.read(S as u64).unwrap(), 0x10, "writes to S are ignored");
    }

    #[test]
    fn control_register_readback() {
        // CLOCK_SetFeeMode polls `while (MCG->C4 != written)` — control regs
        // must read back what was written.
        let mut mcg = Mcg::new();
        mcg.write(0x03, 0x20).unwrap(); // C4 = DRST_DRS(1)
        assert_eq!(mcg.read(0x03).unwrap(), 0x20);
    }
}
