//! Cortex-M floating-point extension: the FP context control registers in the
//! System Control Space, and the state the core needs to stack an FP context.
//!
//! An Armv7-M / Armv8-M core with the FP extension (Cortex-M4F, M7, M33, ...)
//! has, besides the S registers and FPSCR, a small register block at
//! 0xE000_EF34 (ARMv7-M ARM DDI 0403E B3.2.2 and the FP extension chapter;
//! ARMv8-M ARM DDI 0553 B11.1, D1.2):
//!
//! | Address      | Register | What it does here                               |
//! |--------------|----------|-------------------------------------------------|
//! | 0xE000_EF34  | FPCCR    | ASPEN (31), LSPEN (30); reset 0xC000_0000       |
//! | 0xE000_EF38  | FPCAR    | address of the S0 slot of the last FP frame     |
//! | 0xE000_EF3C  | FPDSCR   | AHP/DN/FZ/RMode a new FP context starts with    |
//! | 0xE000_EF40  | MVFR0    | media and FP feature register 0 (read-only)     |
//! | 0xE000_EF44  | MVFR1    | media and FP feature register 1 (read-only)     |
//! | 0xE000_EF48  | MVFR2    | media and FP feature register 2 (read-only)     |
//!
//! A chip whose core has the FP extension declares this model:
//!
//! ```yaml
//! - id: "fpu"
//!   type: "cortex_m_fpu"
//!   base_address: 0xE000EF30
//!   size: 0x1C
//! ```
//!
//! `config` may override `mvfr0`/`mvfr1`/`mvfr2`; the defaults are the
//! single-precision FPv5 values the Cortex-M33 TRM (100230) gives.
//!
//! `configure_cortex_m` finds it through `Peripheral::fp_context_state` and
//! hands the shared [`FpContextState`] to the core, which then:
//!
//! - sets CONTROL.FPCA (and loads FPSCR[26:22] from FPDSCR) on the first FP
//!   instruction of a context when FPCCR.ASPEN is set (`ExecuteFPCheck`);
//! - stacks the extended 0x68-byte frame (R0-R3, R12, LR, PC, xPSR, S0-S15,
//!   FPSCR, reserved) on exception entry while CONTROL.FPCA is set, returns
//!   EXC_RETURN with bit 4 clear (0xFFFF_FFE1/E9/ED), and clears FPCA;
//! - on exception return with EXC_RETURN[4] = 0 unstacks S0-S15 and FPSCR,
//!   and sets CONTROL.FPCA = NOT EXC_RETURN[4];
//! - exposes CONTROL.FPCA through MRS/MSR.
//!
//! Without this model none of that happens: CONTROL bit 2 reads zero, every
//! frame is the basic 8-word one and the window is unmapped (the previous
//! behaviour, which a Zephyr `CONFIG_FPU_SHARING` image BusFaults on).
//!
//! Not modelled: lazy state preservation. With LSPEN set, silicon only
//! reserves the S0-S15/FPSCR slots at entry and fills them when the handler
//! first touches the FPU (FPCCR.LSPACT marks the pending save). This model
//! always fills them at entry, so LSPACT always reads 0. The stacked values and
//! what software sees after the return are the same either way; what differs is
//! the cycle cost of the entry and any software that reads LSPACT or the
//! HFRDY/MMRDY/BFRDY/MONRDY/USER/THREAD bits, which read 0. The Armv8-M
//! Security Extension FPCCR bits (S, SFRDY, TS, CLRONRET, ...) are not
//! modelled either: the core runs Non-secure-only.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::{Peripheral, SimResult};

/// FPCCR.ASPEN: set CONTROL.FPCA on FP instruction execution.
pub const FPCCR_ASPEN: u32 = 1 << 31;
/// FPCCR.LSPEN: lazy state preservation (stored; stacking is always eager).
pub const FPCCR_LSPEN: u32 = 1 << 30;
const FPCCR_RESET: u32 = FPCCR_ASPEN | FPCCR_LSPEN;
const FPCCR_WRITABLE: u32 = FPCCR_ASPEN | FPCCR_LSPEN;
/// FPCAR[31:3]: the address is doubleword-aligned.
const FPCAR_WRITABLE: u32 = !0x7;
/// FPDSCR AHP (26), DN (25), FZ (24), RMode (23:22).
pub const FPDSCR_MASK: u32 = 0x07C0_0000;

const FPCCR: u64 = 0x04;
const FPCAR: u64 = 0x08;
const FPDSCR: u64 = 0x0C;
const MVFR0: u64 = 0x10;
const MVFR1: u64 = 0x14;
const MVFR2: u64 = 0x18;

/// Cortex-M33 single-precision FPv5 (Cortex-M33 TRM 100230, FPU registers).
pub const DEFAULT_MVFR0: u32 = 0x1011_0021;
pub const DEFAULT_MVFR1: u32 = 0x1100_0011;
pub const DEFAULT_MVFR2: u32 = 0x0000_0040;

/// The FP context control state shared between this model and the core.
#[derive(Debug)]
pub struct FpContextState {
    fpccr: AtomicU32,
    fpcar: AtomicU32,
    fpdscr: AtomicU32,
}

impl Default for FpContextState {
    fn default() -> Self {
        Self {
            fpccr: AtomicU32::new(FPCCR_RESET),
            fpcar: AtomicU32::new(0),
            fpdscr: AtomicU32::new(0),
        }
    }
}

impl FpContextState {
    #[inline]
    pub fn fpccr(&self) -> u32 {
        self.fpccr.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn aspen(&self) -> bool {
        self.fpccr() & FPCCR_ASPEN != 0
    }

    #[inline]
    pub fn fpdscr(&self) -> u32 {
        self.fpdscr.load(Ordering::Relaxed)
    }

    pub fn fpcar(&self) -> u32 {
        self.fpcar.load(Ordering::Relaxed)
    }

    /// The core stacked an extended frame whose S0 slot is at `addr`.
    #[inline]
    pub fn set_fpcar(&self, addr: u32) {
        self.fpcar.store(addr & FPCAR_WRITABLE, Ordering::Relaxed);
    }

    fn set_fpccr(&self, v: u32) {
        self.fpccr.store(v & FPCCR_WRITABLE, Ordering::Relaxed);
    }

    fn set_fpdscr(&self, v: u32) {
        self.fpdscr.store(v & FPDSCR_MASK, Ordering::Relaxed);
    }

    fn reset(&self) {
        self.fpccr.store(FPCCR_RESET, Ordering::Relaxed);
        self.fpcar.store(0, Ordering::Relaxed);
        self.fpdscr.store(0, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub struct CortexMFpu {
    state: Arc<FpContextState>,
    mvfr: [u32; 3],
}

impl CortexMFpu {
    pub fn new(mvfr: [u32; 3]) -> Self {
        Self {
            state: Arc::new(FpContextState::default()),
            mvfr,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            FPCCR => self.state.fpccr(),
            FPCAR => self.state.fpcar(),
            FPDSCR => self.state.fpdscr(),
            MVFR0 => self.mvfr[0],
            MVFR1 => self.mvfr[1],
            MVFR2 => self.mvfr[2],
            _ => 0,
        }
    }
}

impl Default for CortexMFpu {
    fn default() -> Self {
        Self::new([DEFAULT_MVFR0, DEFAULT_MVFR1, DEFAULT_MVFR2])
    }
}

impl Peripheral for CortexMFpu {
    // A pure register window with no `tick()`.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let v = self.read_reg(offset & !3);
        Ok((v >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset & 3) * 8;
        let old = self.read_reg(offset & !3);
        let v = (old & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_u32(offset & !3, v)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            FPCCR => self.state.set_fpccr(value),
            FPCAR => self.state.set_fpcar(value),
            FPDSCR => self.state.set_fpdscr(value),
            // MVFR0-2 are read-only; 0xEF30 is reserved.
            _ => crate::census_reg!("cortex_m_fpu:CortexMFpu", offset & !3, "write"),
        }
        Ok(())
    }

    fn fp_context_state(&self) -> Option<Arc<FpContextState>> {
        Some(self.state.clone())
    }

    fn on_node_reset(&mut self) {
        self.state.reset();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "fpccr": self.state.fpccr(),
            "fpcar": self.state.fpcar(),
            "fpdscr": self.state.fpdscr(),
        })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let get = |k: &str| state.get(k).and_then(|v| v.as_u64()).map(|v| v as u32);
        if let Some(v) = get("fpccr") {
            self.state.set_fpccr(v);
        }
        if let Some(v) = get("fpcar") {
            self.state.set_fpcar(v);
        }
        if let Some(v) = get("fpdscr") {
            self.state.set_fpdscr(v);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_values_and_writable_bits() {
        let mut f = CortexMFpu::default();
        // FPCCR resets with ASPEN|LSPEN (0xE000_EF34 = base + 4).
        assert_eq!(f.read_reg(FPCCR), 0xC000_0000);
        assert_eq!(f.read_reg(MVFR0), DEFAULT_MVFR0);
        f.write_u32(FPCCR, 0xFFFF_FFFF).unwrap();
        assert_eq!(f.read_reg(FPCCR), 0xC000_0000, "LSPACT and RDY bits read 0");
        f.write_u32(FPCCR, 0).unwrap();
        assert!(!f.state.aspen());
        f.write_u32(FPCAR, 0x2000_1007).unwrap();
        assert_eq!(f.read_reg(FPCAR), 0x2000_1000);
        f.write_u32(FPDSCR, 0xFFFF_FFFF).unwrap();
        assert_eq!(f.read_reg(FPDSCR), FPDSCR_MASK);
        f.write_u32(MVFR0, 0).unwrap();
        assert_eq!(f.read_reg(MVFR0), DEFAULT_MVFR0, "MVFR0 is read-only");
        // Byte store into FPCCR's top byte clears ASPEN/LSPEN only.
        f.write(FPCCR + 3, 0xC0).unwrap();
        assert_eq!(f.read_reg(FPCCR), 0xC000_0000);
        f.on_node_reset();
        assert_eq!(f.read_reg(FPCCR), 0xC000_0000);
        assert_eq!(f.read_reg(FPDSCR), 0);
    }

    #[test]
    fn snapshot_round_trips() {
        let mut f = CortexMFpu::default();
        f.write_u32(FPCCR, FPCCR_ASPEN).unwrap();
        f.write_u32(FPDSCR, 1 << 24).unwrap();
        let s = f.snapshot();
        let mut g = CortexMFpu::default();
        g.restore(s).unwrap();
        assert_eq!(g.read_reg(FPCCR), FPCCR_ASPEN);
        assert_eq!(g.read_reg(FPDSCR), 1 << 24);
    }
}
