// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::SystemBus;
use crate::decoder::arm::{decode_thumb_16, decode_thumb_32, Instruction};
use crate::peripherals::scb::{
    ScbFaultState, CFSR_BFSR_BFARVALID, CFSR_BFSR_PRECISERR, CFSR_UFSR_UNDEFINSTR, HFSR_FORCED,
    SHCSR_BUSFAULTENA, SHCSR_USGFAULTENA,
};
use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationError, SimulationObserver};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// `LABWIRED_TRACE_INSN` / `LABWIRED_TRACE_EXC` are developer trace gates read
/// once per process. They used to be `std::env::var` calls evaluated on EVERY
/// retired instruction, which cost ~830 host instructions per simulated one
/// (environ walk + strncmp). Hoisted into `OnceLock` so the hot path pays a
/// single atomic load.
fn trace_insn_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LABWIRED_TRACE_INSN").is_ok())
}

fn trace_exc_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LABWIRED_TRACE_EXC").is_ok())
}

#[derive(Debug, Clone, Copy)]
pub struct DecodeCacheEntry {
    pub tag: u32,
    pub instruction: Instruction,
    pub opcode: u32,
    pub pc_increment: u8,
    pub cycles: u32,
}

#[derive(Debug)]
pub struct CortexM {
    pub r0: u32,
    pub r1: u32,
    pub r2: u32,
    pub r3: u32,
    pub r4: u32,
    pub r5: u32,
    pub r6: u32,
    pub r7: u32,
    pub r8: u32,
    pub r9: u32,
    pub r10: u32,
    pub r11: u32,
    pub r12: u32,
    pub sp: u32, // R13 — the *active* stack pointer (live R13).
    /// Banked Main / Process stack pointers (ARMv7-M). `sp` is always the
    /// live copy of whichever bank is currently selected; the OTHER bank is
    /// held here. Handler mode always uses MSP; Thread mode uses MSP or PSP
    /// per `CONTROL.SPSEL`. The active bank's stored field is treated as
    /// stale — `sp` is authoritative for it until the next stack switch.
    pub msp: u32,
    pub psp: u32,
    /// CONTROL register. Only SPSEL (bit 1) and nPRIV (bit 0) are modelled;
    /// FPCA (bit 2) is not (no lazy FP stacking).
    pub control: u32,
    pub lr: u32, // R14
    pub pc: u32, // R15
    pub xpsr: u32,
    /// Pending-exception bitmask, 4x64 = exceptions 0..255. The H5-class
    /// parts wire external interrupts past IRQ 47 (STM32H563 TIM12 = IRQ
    /// 120 -> exception 136), which a single u64 silently dropped —
    /// caught by foreign firmware whose time driver never ticked.
    pub pending_exceptions: [u64; 4],
    pub primask: bool, // Interrupt mask (true = disabled)
    /// FAULTMASK: when set, masks every exception except NMI (it raises the
    /// effective priority to -1). Set by `CPSID f` / `MSR FAULTMASK`, cleared by
    /// `CPSIE f` and automatically on exception return (except return from NMI).
    /// Zephyr's fault path toggles it; an unmodelled `CPS f` decoded as Unknown.
    pub faultmask: bool,
    /// BASEPRI: when non-zero, masks any exception whose priority value is
    /// numerically >= basepri (i.e. equal or lower priority). Zephyr's Cortex-M
    /// critical sections raise BASEPRI to block the scheduler/timer IRQs; an
    /// unmodelled BASEPRI let those fire mid-critical-section and corrupt kernel
    /// state. NMI/HardFault (negative priority) are never masked by it.
    pub basepri: u8,
    pub vtor: Arc<AtomicU32>, // Shared Vector Table Offset Register
    pub it_state: u8,         // Thumb IT block state
    /// Currently active exception number (0 = thread mode). Used to prevent re-entry
    /// of the same or lower-priority exception while one is already being serviced.
    pub active_exception: u32,
    /// Shared with SCB peripheral: live mirror of `active_exception` so
    /// firmware reading ICSR.VECTACTIVE sees the currently-handling
    /// exception. cortex-m-rt's DefaultHandler depends on this to
    /// route to the right IRQ branch.
    pub vectactive: Arc<AtomicU32>,
    /// Shared with SCB: SHPR1 (MemManage/BusFault/UsageFault priority).
    /// Used by `exception_priority` so dispatch honours real ARM priority
    /// rules rather than picking by exception number.
    pub shpr1: Arc<AtomicU32>,
    /// Shared with SCB: SHPR2 (SVCall priority byte 3).
    pub shpr2: Arc<AtomicU32>,
    /// Shared with SCB: SHPR3 (PendSV byte 2, SysTick byte 3). FreeRTOS
    /// sets PendSV to 0xFF (lowest), which lets the SysTick handler set
    /// PENDSVSET and only have it dispatch on return-to-thread — the
    /// load-bearing semantics for context switching.
    pub shpr3: Arc<AtomicU32>,
    /// Shared NVIC state for IRQ priority lookups via IPR.
    pub nvic_state: Option<Arc<crate::peripherals::nvic::NvicState>>,
    /// Shared with SCB: AIRCR.SYSRESETREQ latched by firmware. `step_batch`
    /// polls it after each retired instruction and ends the batch, so the
    /// machine drains the reset (`Machine::drain_scb_reset_request`) at the
    /// same committed boundary a one-instruction quantum would have stopped
    /// at — without pinning the quantum to 1 for the life of the bus.
    /// `None` on hand-built buses that never went through `configure_cortex_m`;
    /// those keep the legacy behaviour of their caller.
    pub sysreset_signal: Option<Arc<AtomicBool>>,
    /// Shared with the SCB: the ARMv7-M fault register file (SHCSR/CFSR/HFSR/
    /// BFAR) plus the master `enabled` switch for fault escalation.
    ///
    /// `None` on hand-built cores that never went through `configure_cortex_m`
    /// — no SCB means no fault registers to report through, so those keep the
    /// #880 abort contract unconditionally. See [`ScbFaultState`].
    pub faults: Option<Arc<ScbFaultState>>,
    /// Address of the data-side access that just raised
    /// `SimulationError::MemoryViolation`, latched by [`CortexM::load`] /
    /// [`CortexM::store`] so `step_internal` can tell a **data** fault (which
    /// ARMv7-M turns into a precise BusFault) from an instruction-fetch fault,
    /// an exception-entry stacking fault or a vector-table read fault — all of
    /// which are different contracts and stay on the abort path.
    ///
    /// Only ever written on the error path, so a clean step costs nothing.
    pending_data_fault: Option<u32>,
    /// Set when decode reached an instruction this model does not implement, so
    /// `step_internal` can raise UsageFault instead of returning the bare
    /// `DecodeError`.
    ///
    /// Separate from `pending_data_fault` because the two escalate differently:
    /// a data fault names an address and targets BusFault, an undefined
    /// instruction names none and targets UsageFault. Only ever written on the
    /// error path.
    pending_undef_instruction: bool,
    pub decode_cache: Box<[Option<DecodeCacheEntry>; 4096]>,
    /// FPU single-precision register file (VFPv4 single — S0..S31).
    /// Each S register is the IEEE-754 binary32 bit pattern; reads via
    /// `f32::from_bits` and writes via `f32::to_bits`. Double-precision
    /// (D0..D15 = pairs of S regs) is NOT modelled; firmware compiled for
    /// `-mfpu=fpv4-sp-d16` only emits single-precision ops anyway.
    pub fpu_s: [u32; 32],
    /// FPSCR, the VFP status/control register. Only the two mode bits that
    /// change arithmetic results are modeled: FZ (bit 24) and DN (bit 25).
    /// Everything else — exception-enable bits, cumulative flags, rounding
    /// mode — reads as zero and is not updated by VFP ops (no VMRS/VMSR
    /// instruction is decoded yet). Reset value 0, so a core that never
    /// touches FPSCR keeps the plain IEEE-754 results.
    pub fpscr: u32,
    /// True while the core is suspended in WFI sleep. Set by the `Wfi`
    /// executor when no wake-up event is pending, cleared at the top of every
    /// `step_internal`. Gates idle fast-forward; transient (not snapshotted),
    /// mirroring the RISC-V `waiting_for_interrupt` flag.
    sleeping: bool,
    /// Local byte-exclusive reservation: address and value observed by LDREXB.
    /// Comparing the value at STREXB conservatively detects conflicting bus
    /// writes without requiring every bus implementation to expose epochs;
    /// an external write of the same byte value is therefore indistinguishable.
    exclusive_byte: Option<(u32, u8)>,
    /// Opt-in Thumb-2 wasm-JIT fast path. Synced from
    /// [`crate::SimulationConfig::cortex_m_jit_enabled`] on each `step_batch`
    /// entry. Off by default — the interpreter is the behavioral oracle.
    #[cfg(feature = "jit")]
    jit_enabled: bool,
    /// Lazily-created JIT engine. `None` until the first JIT-enabled batch.
    #[cfg(feature = "jit")]
    jit_engine: Option<crate::cpu::jit_framework::cortex_m::CortexMJitEngine>,
}

impl Default for CortexM {
    fn default() -> Self {
        Self {
            r0: 0,
            r1: 0,
            r2: 0,
            r3: 0,
            r4: 0,
            r5: 0,
            r6: 0,
            r7: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            sp: 0,
            msp: 0,
            psp: 0,
            control: 0,
            lr: 0,
            pc: 0,
            xpsr: 0x01000000, // Typical reset state (Thumb bit set)
            pending_exceptions: [0; 4],
            primask: false,
            faultmask: false,
            basepri: 0,
            vtor: Arc::new(AtomicU32::new(0)),
            it_state: 0,
            active_exception: 0,
            vectactive: Arc::new(AtomicU32::new(0)),
            shpr1: Arc::new(AtomicU32::new(0)),
            shpr2: Arc::new(AtomicU32::new(0)),
            shpr3: Arc::new(AtomicU32::new(0)),
            nvic_state: None,
            sysreset_signal: None,
            faults: None,
            pending_data_fault: None,
            pending_undef_instruction: false,
            decode_cache: Box::new([None; 4096]),
            fpu_s: [0u32; 32],
            fpscr: 0,
            sleeping: false,
            exclusive_byte: None,
            #[cfg(feature = "jit")]
            jit_enabled: false,
            #[cfg(feature = "jit")]
            jit_engine: None,
        }
    }
}

/// FPSCR bit 24 — Flush-to-Zero. Denormal inputs are replaced by a zero of
/// the same sign before the operation and denormal results after it, matching
/// ARMv7-M VFPv4.
pub const FPSCR_FZ: u32 = 1 << 24;
/// FPSCR bit 25 — Default NaN. Every NaN result becomes [`VFP_DEFAULT_NAN`],
/// discarding whatever payload the host FPU produced.
pub const FPSCR_DN: u32 = 1 << 25;

/// The ARM default NaN: quiet, sign clear, zero payload.
pub const VFP_DEFAULT_NAN: u32 = 0x7FC0_0000;

/// Quiet bit of a binary32 NaN.
const F32_QUIET_BIT: u32 = 0x0040_0000;
/// Exponent field of a binary32.
const F32_EXP_MASK: u32 = 0x7F80_0000;
/// Mantissa field of a binary32.
const F32_MANT_MASK: u32 = 0x007F_FFFF;
/// Sign bit of a binary32.
const F32_SIGN_BIT: u32 = 0x8000_0000;

/// True for a binary32 NaN (exponent all ones, non-zero mantissa).
#[inline]
pub fn vfp_is_nan(bits: u32) -> bool {
    (bits & F32_EXP_MASK) == F32_EXP_MASK && (bits & F32_MANT_MASK) != 0
}

/// True for a binary32 denormal (exponent zero, non-zero mantissa).
#[inline]
pub fn vfp_is_denormal(bits: u32) -> bool {
    (bits & F32_EXP_MASK) == 0 && (bits & F32_MANT_MASK) != 0
}

/// Flush-to-Zero one operand or result: a denormal becomes a zero carrying
/// the original sign, everything else passes through untouched (NaNs included
/// — an all-ones exponent is never denormal).
#[inline]
pub fn vfp_flush_to_zero(bits: u32) -> u32 {
    if vfp_is_denormal(bits) {
        bits & F32_SIGN_BIT
    } else {
        bits
    }
}

/// Deterministic NaN rule applied to a raw arithmetic result.
///
/// The arithmetic itself only decides *whether* the result is NaN — which
/// payload a native FPU invents (x86 SSE returns a quieted input, wasm f32
/// payloads are not specified) is discarded here:
///
/// * FPSCR.DN set → the ARM default NaN, payload ignored.
/// * otherwise the first NaN operand (a, then b) quieted by setting its
///   quiet bit; sign and payload are preserved.
/// * otherwise — an invalid operation with no NaN input, e.g. `0 * inf` or
///   `inf - inf` — the default quiet NaN.
///
/// This is what makes JIT and interpreter byte-identical for NaN results:
/// both lanes call this same integer rule.
#[inline]
pub fn vfp_canonical_nan(result: u32, a: u32, b: u32, fpscr: u32) -> u32 {
    if !vfp_is_nan(result) {
        return result;
    }
    if fpscr & FPSCR_DN != 0 {
        return VFP_DEFAULT_NAN;
    }
    let first = if vfp_is_nan(a) {
        a
    } else if vfp_is_nan(b) {
        b
    } else {
        return VFP_DEFAULT_NAN;
    };
    first | F32_QUIET_BIT
}

/// Which single-precision operation [`vfp_binop`] evaluates. The numeric
/// discriminants are the wire codes of the `vfp.binop` host import; keep them
/// in sync with `emit`'s `VFP_OP_*` constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfpBinOp {
    Add = 0,
    Sub = 1,
    Mul = 2,
    Div = 3,
}

impl VfpBinOp {
    /// Decode a host-import wire code (only the compiled lane needs this).
    pub fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::Add),
            1 => Some(Self::Sub),
            2 => Some(Self::Mul),
            3 => Some(Self::Div),
            _ => None,
        }
    }

    #[inline]
    fn eval(self, a: f32, b: f32) -> f32 {
        match self {
            Self::Add => a + b,
            Self::Sub => a - b,
            Self::Mul => a * b,
            Self::Div => a / b,
        }
    }
}

/// Evaluate one VFP single-precision binop under FPSCR.FZ/DN.
///
/// The arithmetic is the plain Rust `f32` op — identical bits to the wasm
/// `f32` op for every finite result — and the FPSCR modes are applied as
/// integer post-processing so the result no longer depends on host NaN
/// behavior. This is the single source of truth for both the interpreter and
/// the compiled lane's `vfp.binop` host import.
pub fn vfp_binop(op: VfpBinOp, a_bits: u32, b_bits: u32, fpscr: u32) -> u32 {
    let fz = fpscr & FPSCR_FZ != 0;
    let a = if fz {
        vfp_flush_to_zero(a_bits)
    } else {
        a_bits
    };
    let b = if fz {
        vfp_flush_to_zero(b_bits)
    } else {
        b_bits
    };
    let result = op.eval(f32::from_bits(a), f32::from_bits(b)).to_bits();
    let result = vfp_canonical_nan(result, a, b, fpscr);
    if fz {
        vfp_flush_to_zero(result)
    } else {
        result
    }
}

/// Evaluate one VFP fused multiply-add form under FPSCR.FZ/DN, preserving the
/// interpreter's existing expression shapes:
/// `a.mul_add(b, c)`, `(-a).mul_add(b, c)`, `a.mul_add(b, -c)`,
/// `(-a).mul_add(b, -c)` selected by `neg_a` / `neg_c`.
///
/// The operand scan for NaN propagation stays `(a, b, c)` in source order;
/// the addend is never first. FMA forms are interpreter-only today (the JIT
/// does not compile them), so this is fidelity, not a differential fix.
pub fn vfp_fma(a_bits: u32, b_bits: u32, c_bits: u32, neg_a: bool, neg_c: bool, fpscr: u32) -> u32 {
    let fz = fpscr & FPSCR_FZ != 0;
    let a = if fz {
        vfp_flush_to_zero(a_bits)
    } else {
        a_bits
    };
    let b = if fz {
        vfp_flush_to_zero(b_bits)
    } else {
        b_bits
    };
    let c = if fz {
        vfp_flush_to_zero(c_bits)
    } else {
        c_bits
    };
    let fa = f32::from_bits(a);
    let fb = f32::from_bits(b);
    let fc = f32::from_bits(c);
    let result = if neg_a {
        (-fa).mul_add(fb, if neg_c { -fc } else { fc })
    } else {
        fa.mul_add(fb, if neg_c { -fc } else { fc })
    }
    .to_bits();
    let result = vfp_canonical_nan(result, a, if vfp_is_nan(b) { b } else { c }, fpscr);
    if fz {
        vfp_flush_to_zero(result)
    } else {
        result
    }
}

impl CortexM {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear_exclusive_monitor(&mut self) {
        self.exclusive_byte = None;
    }

    pub fn get_vtor(&self) -> u32 {
        self.vtor.load(Ordering::SeqCst)
    }

    pub fn set_vtor(&mut self, val: u32) {
        self.vtor.store(val, Ordering::SeqCst);
    }

    pub fn set_shared_vtor(&mut self, vtor: Arc<AtomicU32>) {
        self.vtor = vtor;
    }

    pub fn set_shared_vectactive(&mut self, vectactive: Arc<AtomicU32>) {
        self.vectactive = vectactive;
    }

    /// Wire the three SHPR atomics shared with the SCB peripheral. Once
    /// shared, `exception_priority` can read live priorities and the
    /// dispatch loop honours ARMv7-M priority rules instead of just
    /// picking the lowest-numbered pending exception.
    pub fn set_shared_shpr(
        &mut self,
        shpr1: Arc<AtomicU32>,
        shpr2: Arc<AtomicU32>,
        shpr3: Arc<AtomicU32>,
    ) {
        self.shpr1 = shpr1;
        self.shpr2 = shpr2;
        self.shpr3 = shpr3;
    }

    /// Wire the NVIC's shared state so `exception_priority` can read
    /// IPR bytes for IRQs (exception number ≥ 16).
    pub fn set_shared_nvic_state(&mut self, state: Arc<crate::peripherals::nvic::NvicState>) {
        self.nvic_state = Some(state);
    }

    /// Wire the SCB's SYSRESETREQ mirror so the batch loop can stop on the
    /// instruction that requests a system reset. See the field docs.
    pub fn set_shared_sysreset_signal(&mut self, signal: Arc<AtomicBool>) {
        self.sysreset_signal = Some(signal);
    }

    /// Wire the SCB's ARMv7-M fault register file so the core can report a
    /// fault through CFSR/HFSR/BFAR and read SHCSR to decide whether BusFault is
    /// enabled. See [`ScbFaultState`].
    pub fn set_shared_faults(&mut self, faults: Arc<ScbFaultState>) {
        self.faults = Some(faults);
    }

    /// Turn ARMv7-M fault escalation (and the SCB fault register surface) on or
    /// off. The process default is **on** (`LABWIRED_CORTEXM_FAULTS` is the
    /// opt-out); this is the explicit per-core override, and the single switch
    /// for the whole feature:
    /// the CPU and the SCB read the same `AtomicBool`, so they cannot disagree
    /// about whether firmware can enable a handler the core will never pend.
    ///
    /// No-op on a core with no SCB wired (`faults == None`).
    pub fn set_faults_enabled(&mut self, enabled: bool) {
        if let Some(f) = &self.faults {
            f.enabled.store(enabled, Ordering::Relaxed);
        }
    }

    /// True when ARMv7-M fault escalation is modelled on this core.
    #[inline(always)]
    fn faults_enabled(&self) -> bool {
        self.faults.as_ref().is_some_and(|f| f.is_enabled())
    }

    /// True once firmware has latched AIRCR.SYSRESETREQ and the machine has
    /// not yet drained it. One relaxed load; `false` when no SCB is wired.
    /// Compiled backends read this to end a window on the AIRCR store the way
    /// the interpreter does, instead of retiring past the reboot request.
    #[inline(always)]
    pub fn sysreset_latched(&self) -> bool {
        self.sysreset_signal
            .as_ref()
            .is_some_and(|f| f.load(Ordering::Relaxed))
    }

    /// ARMv7-M exception priority. Lower numeric value = higher priority.
    /// Reset(1) = -3, NMI(2) = -2, HardFault(3) = -1 are fixed. Configurable
    /// system exceptions read from SHPR1/2/3. IRQs (≥16) read from the
    /// NVIC IPR byte for that IRQ. Unmapped or unknown exceptions return
    /// 0xFF (lowest configurable priority).
    pub fn exception_priority(&self, exc: u32) -> i32 {
        match exc {
            0 => 256,
            1 => -3,
            2 => -2,
            3 => -1,
            4 => (self.shpr1.load(Ordering::Relaxed) & 0xFF) as i32,
            5 => ((self.shpr1.load(Ordering::Relaxed) >> 8) & 0xFF) as i32,
            6 => ((self.shpr1.load(Ordering::Relaxed) >> 16) & 0xFF) as i32,
            11 => ((self.shpr2.load(Ordering::Relaxed) >> 24) & 0xFF) as i32,
            14 => ((self.shpr3.load(Ordering::Relaxed) >> 16) & 0xFF) as i32,
            15 => ((self.shpr3.load(Ordering::Relaxed) >> 24) & 0xFF) as i32,
            n if n >= 16 => {
                if let Some(nvic) = &self.nvic_state {
                    nvic.ipr_priority((n - 16) as usize) as i32
                } else {
                    0xFF
                }
            }
            _ => 0xFF,
        }
    }

    /// True if BASEPRI masks an exception of the given priority. A non-zero
    /// BASEPRI masks any exception whose priority value is numerically >=
    /// BASEPRI (equal or lower priority). NMI/HardFault (negative priority) are
    /// never masked.
    #[inline]
    fn masked_by_basepri(&self, prio: i32) -> bool {
        self.basepri != 0 && prio >= self.basepri as i32
    }

    /// Whether ANY exception is pending — the cheap precondition every
    /// pending-exception decision starts with.
    ///
    /// A four-way OR rather than `self.pending_exceptions.iter().any(..)`:
    /// the iterator form compiled to a real loop over the array and cost
    /// 16 Ir per retired instruction in `step_batch` alone
    /// (docs/performance/2026-09-17-bus-scheduler-pass.md). Same answer —
    /// `w0 | w1 | w2 | w3 != 0` is true exactly when some word is non-zero —
    /// with no branch per word.
    #[inline(always)]
    fn any_exception_pending(&self) -> bool {
        let [w0, w1, w2, w3] = self.pending_exceptions;
        (w0 | w1 | w2 | w3) != 0
    }

    /// Among the pending exceptions, return the one with the highest
    /// priority (lowest numeric value). Ties break by exception number
    /// (lower number wins, per ARMv7-M B1.5.4).
    fn highest_priority_pending(&self) -> Option<u32> {
        let mut best: Option<(i32, u32)> = None;
        for (word_idx, &word) in self.pending_exceptions.iter().enumerate() {
            let mut mask = word;
            while mask != 0 {
                let exc = (word_idx as u32) * 64 + mask.trailing_zeros();
                mask &= mask - 1;
                let prio = self.exception_priority(exc);
                best = Some(match best {
                    Some((bp, be)) if bp <= prio => (bp, be),
                    _ => (prio, exc),
                });
            }
        }
        best.map(|(_, e)| e)
    }

    /// Update both the local field and the SCB.ICSR mirror in one go.
    fn set_active_exception(&mut self, exc: u32) {
        self.active_exception = exc;
        self.vectactive
            .store(exc & 0x1FF, std::sync::atomic::Ordering::Relaxed);
    }

    fn xpsr_with_itstate(&self, xpsr: u32) -> u32 {
        let mut out = xpsr & !((0b11 << 25) | (0b11_1111 << 10));
        out |= ((self.it_state as u32) & 0b11) << 25;
        out |= (((self.it_state as u32) >> 2) & 0b11_1111) << 10;
        out
    }

    fn itstate_from_xpsr(xpsr: u32) -> u8 {
        let low = ((xpsr >> 25) & 0b11) as u8;
        let high = ((xpsr >> 10) & 0b11_1111) as u8;
        low | (high << 2)
    }

    /// Read a double-precision value from the S-register pair at low index `d_lo`
    /// (= 2*Dn). Little-endian: the low S-register holds bits [31:0].
    fn read_f64(&self, d_lo: u8) -> f64 {
        let lo = self.fpu_s.get(d_lo as usize).copied().unwrap_or(0) as u64;
        let hi = self.fpu_s.get(d_lo as usize + 1).copied().unwrap_or(0) as u64;
        f64::from_bits((hi << 32) | lo)
    }

    /// Write a double-precision value to the S-register pair at low index `d_lo`.
    fn write_f64(&mut self, d_lo: u8, v: f64) {
        let bits = v.to_bits();
        if (d_lo as usize + 1) < 32 {
            self.fpu_s[d_lo as usize] = bits as u32;
            self.fpu_s[d_lo as usize + 1] = (bits >> 32) as u32;
        }
    }

    fn read_reg(&self, n: u8) -> u32 {
        match n {
            0 => self.r0,
            1 => self.r1,
            2 => self.r2,
            3 => self.r3,
            4 => self.r4,
            5 => self.r5,
            6 => self.r6,
            7 => self.r7,
            8 => self.r8,
            9 => self.r9,
            10 => self.r10,
            11 => self.r11,
            12 => self.r12,
            13 => self.sp,
            14 => self.lr,
            15 => self.pc,
            16 => self.xpsr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, n: u8, val: u32) {
        match n {
            0 => self.r0 = val,
            1 => self.r1 = val,
            2 => self.r2 = val,
            3 => self.r3 = val,
            4 => self.r4 = val,
            5 => self.r5 = val,
            6 => self.r6 = val,
            7 => self.r7 = val,
            8 => self.r8 = val,
            9 => self.r9 = val,
            10 => self.r10 = val,
            11 => self.r11 = val,
            12 => self.r12 = val,
            13 => self.sp = val,
            14 => self.lr = val,
            15 => self.pc = val,
            16 => self.xpsr = val,
            _ => {}
        }
    }

    fn update_nz(&mut self, result: u32) {
        let n = (result >> 31) & 1;
        let z = if result == 0 { 1 } else { 0 };
        // Clear N/Z (bits 31, 30)
        self.xpsr &= !(0xC000_0000);
        self.xpsr |= (n << 31) | (z << 30);
    }

    fn get_carry(&self) -> bool {
        (self.xpsr >> 29) & 1 == 1
    }

    /// APSR.GE[3:0] live in xpsr bits [19:16]; each corresponds to one byte
    /// lane of a SIMD add/sub and is consumed by SEL. `ge` low nibble = GE[3:0].
    fn set_ge(&mut self, ge: u32) {
        self.xpsr &= !(0xF << 16);
        self.xpsr |= (ge & 0xF) << 16;
    }

    fn get_ge(&self) -> u32 {
        (self.xpsr >> 16) & 0xF
    }

    fn get_overflow(&self) -> bool {
        (self.xpsr >> 28) & 1 == 1
    }

    fn update_nzcv(&mut self, result: u32, carry: bool, overflow: bool) {
        let n = (result >> 31) & 1;
        let z = if result == 0 { 1 } else { 0 };
        let c = if carry { 1 } else { 0 };
        let v = if overflow { 1 } else { 0 };

        self.xpsr &= !(0xF000_0000);
        self.xpsr |= (n << 31) | (z << 30) | (c << 29) | (v << 28);
    }

    #[inline(always)]
    fn check_condition(&self, cond: u8) -> bool {
        let n = (self.xpsr >> 31) & 1 == 1;
        let z = (self.xpsr >> 30) & 1 == 1;
        let c = (self.xpsr >> 29) & 1 == 1;
        let v = (self.xpsr >> 28) & 1 == 1;

        match cond {
            0x0 => z,              // EQ (Equal)
            0x1 => !z,             // NE (Not Equal)
            0x2 => c,              // CS/HS (Carry Set)
            0x3 => !c,             // CC/LO (Carry Clear)
            0x4 => n,              // MI (Minus)
            0x5 => !n,             // PL (Plus)
            0x6 => v,              // VS (Overflow)
            0x7 => !v,             // VC (No Overflow)
            0x8 => c && !z,        // HI (Unsigned Higher)
            0x9 => !c || z,        // LS (Unsigned Lower or Same)
            0xA => n == v,         // GE (Signed Greater or Equal)
            0xB => n != v,         // LT (Signed Less Than)
            0xC => !z && (n == v), // GT (Signed Greater Than)
            0xD => z || (n != v),  // LE (Signed Less or Equal)
            0xE => true,           // AL (Always)
            _ => false,            // Undefined/Reserved
        }
    }

    fn branch_to<B: Bus + ?Sized>(&mut self, addr: u32, bus: &mut B) -> SimResult<()> {
        if (addr & 0xFFFFFFF0) == 0xFFFFFFF0 {
            // EXC_RETURN: valid values are 0xFFFFFFF1/F9/FD (and FPU variants E1/E9/ED)
            self.exception_return(addr, bus)?;
        } else {
            self.pc = addr & !1;
        }
        Ok(())
    }

    /// True if FAULTMASK currently blocks taking the given exception. FAULTMASK
    /// masks everything except NMI (exception 2).
    #[inline]
    fn faultmask_blocks(&self, exc: u32) -> bool {
        self.faultmask && exc != 2
    }

    /// ARMv7-M WFI wake-up condition: a pending exception whose priority would
    /// preempt the current execution priority *if it were unmasked*. This
    /// deliberately ignores PRIMASK — the canonical `__disable_irq(); wfi();`
    /// idle pattern must wake on a pend even though PRIMASK blocks the actual
    /// entry (the core then falls through without taking the exception).
    /// BASEPRI/FAULTMASK still gate: an exception they suppress would not
    /// preempt, so it is not a wake event. Mirrors the takeable-exception break
    /// in `step_batch`, minus the `!self.primask` guard.
    fn wfi_wake_pending(&self) -> bool {
        if !self.any_exception_pending() {
            return false;
        }
        let Some(exc) = self.highest_priority_pending() else {
            return false;
        };
        let exc_prio = self.exception_priority(exc);
        let active_prio = self.exception_priority(self.active_exception);
        exc_prio < active_prio && !self.masked_by_basepri(exc_prio) && !self.faultmask_blocks(exc)
    }

    /// True when the live `sp` is the Process stack: Thread mode with
    /// CONTROL.SPSEL set. Handler mode always uses MSP.
    #[inline]
    fn use_psp(&self) -> bool {
        self.active_exception == 0 && (self.control & 0x2) != 0
    }

    /// Persist the live `sp` into whichever bank it currently represents.
    /// Call this *before* a transition that changes the selected stack.
    #[inline]
    fn sync_sp_to_bank(&mut self) {
        if self.use_psp() {
            self.psp = self.sp;
        } else {
            self.msp = self.sp;
        }
    }

    /// The stored value of the bank that selection *would* make active now.
    #[inline]
    fn current_stack_value(&self) -> u32 {
        if self.use_psp() {
            self.psp
        } else {
            self.msp
        }
    }

    /// Read MSP regardless of which bank is live.
    #[inline]
    fn read_msp(&self) -> u32 {
        if self.use_psp() {
            self.msp
        } else {
            self.sp
        }
    }

    /// Read PSP regardless of which bank is live.
    #[inline]
    fn read_psp(&self) -> u32 {
        if self.use_psp() {
            self.sp
        } else {
            self.psp
        }
    }

    fn exception_return<B: Bus + ?Sized>(&mut self, exc_return: u32, bus: &mut B) -> SimResult<()> {
        // FAULTMASK is cleared automatically on exception return, except when
        // returning from NMI (exception 2).
        if self.active_exception != 2 {
            self.faultmask = false;
        }

        // We are in Handler mode, so the live `sp` is MSP — capture it.
        self.sync_sp_to_bank();

        // EXC_RETURN bit 2 selects the stack the frame was stacked on:
        // 0 → MSP (returning to a handler or Thread/MSP), 1 → PSP (Thread/PSP).
        let frame_on_psp = (exc_return & 0x4) != 0;
        let frame_ptr = if frame_on_psp { self.psp } else { self.msp };

        self.r0 = bus.read_u32(frame_ptr as u64)?;
        self.r1 = bus.read_u32(frame_ptr.wrapping_add(4) as u64)?;
        self.r2 = bus.read_u32(frame_ptr.wrapping_add(8) as u64)?;
        self.r3 = bus.read_u32(frame_ptr.wrapping_add(12) as u64)?;
        self.r12 = bus.read_u32(frame_ptr.wrapping_add(16) as u64)?;
        self.lr = bus.read_u32(frame_ptr.wrapping_add(20) as u64)?;
        self.pc = bus.read_u32(frame_ptr.wrapping_add(24) as u64)? & !1;
        self.xpsr = bus.read_u32(frame_ptr.wrapping_add(28) as u64)?;
        self.it_state = Self::itstate_from_xpsr(self.xpsr);

        // Advance the bank the frame was popped from.
        let new_sp = frame_ptr.wrapping_add(32);
        if frame_on_psp {
            self.psp = new_sp;
        } else {
            self.msp = new_sp;
        }

        // Restore active exception from stacked xPSR IPSR bits [8:0].
        // When taking an exception, we saved the previous active_exception in IPSR,
        // so restoring it here correctly handles both non-nested and nested cases.
        self.set_active_exception(self.xpsr & 0x1FF);

        // On return to Thread mode, CONTROL.SPSEL takes EXC_RETURN[2].
        if self.active_exception == 0 {
            if frame_on_psp {
                self.control |= 0x2;
            } else {
                self.control &= !0x2;
            }
        }

        // Re-point the live `sp` at whichever bank is now selected.
        self.sp = self.current_stack_value();

        tracing::debug!(
            "EXC_RETURN: frame={:#010x} restored LR={:#010x} PC={:#010x} active_exc={} sp={:#010x}",
            frame_ptr,
            self.lr,
            self.pc,
            self.active_exception,
            self.sp
        );
        Ok(())
    }
}

/// Thumb-2 wasm-JIT dispatch helpers for `Machine<CortexM>`.
#[cfg(feature = "jit")]
const CORTEX_M_JIT_HOT_THRESHOLD: u32 = 50;

/// Thumb batch gates shared by the in-tree JIT (`jit`) and out-of-tree
/// compiled backends (`jit-framework`: the browser adapter). A backend that
/// dispatches its own blocks asks these before running one, so an exception
/// the interpreter would take, or a SysTick edge the block would cross,
/// always ends the compiled window first.
#[cfg(any(feature = "jit", feature = "jit-framework"))]
impl CortexM {
    /// True when a pending exception is takeable *now*: pending, not masked by
    /// PRIMASK/BASEPRI/FAULTMASK, and higher priority than the active
    /// exception. A backend must not run a compiled block while this holds.
    pub fn jit_takeable_exception(&self) -> bool {
        if !self.any_exception_pending() {
            return false;
        }
        let Some(exc) = self.highest_priority_pending() else {
            return false;
        };
        let exc_prio = self.exception_priority(exc);
        let active_prio = self.exception_priority(self.active_exception);
        !self.masked_by_primask(exc)
            && exc_prio < active_prio
            && !self.masked_by_basepri(exc_prio)
            && !self.faultmask_blocks(exc)
    }

    /// RISC-V `block_would_cross_irq` analogue: a compiled block of `n`
    /// instructions is `n` cycles. If SysTick would underflow (TICKINT edge)
    /// inside that span, interpret instead so exception 15 pends on the same
    /// instruction the interpreter would pend it. Already-takeable exceptions
    /// also refuse the block (the interpreter would trap within one insn).
    pub fn block_would_cross_irq(&self, bus: &dyn Bus, n: u32) -> bool {
        if self.jit_takeable_exception() {
            return true;
        }
        match bus.systick_ticks_until_fire() {
            Some(h) => u64::from(n) >= h,
            None => false,
        }
    }
}

#[cfg(feature = "jit")]
impl CortexM {
    fn jit_gate_allows(&self, bus: &dyn Bus, observers: &[Arc<dyn SimulationObserver>]) -> bool {
        if !observers.is_empty() {
            return false;
        }
        // IT is interpreted insn-by-insn inside `run_jit_loop`; do not
        // disable the whole batch (that skipped compiled code after the IT).
        if bus.logic_tap().is_some_and(|t| t.push_armed()) {
            return false;
        }
        if bus.requires_cycle_accurate() {
            return false;
        }
        true
    }

    fn step_batch_jit(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &crate::SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        let mut engine = self.jit_engine.take().unwrap_or_else(|| {
            let mut e = crate::cpu::jit_framework::cortex_m::CortexMJitEngine::new(
                CORTEX_M_JIT_HOT_THRESHOLD,
            );
            if config.cortex_m_jit_min_block_instrs != 0 {
                e.set_min_profitable(config.cortex_m_jit_min_block_instrs);
            }
            e
        });
        let out = self.run_jit_loop(&mut engine, bus, observers, config, max_count);
        self.jit_engine = Some(engine);
        out
    }

    fn run_jit_loop(
        &mut self,
        engine: &mut crate::cpu::jit_framework::cortex_m::CortexMJitEngine,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &crate::SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        use crate::bus::SystemBus;
        use crate::cpu::jit_framework::block_cache::Lookup;

        // Match the interpreter `step_batch` SystemBus arm: advance the
        // in-place cycle accumulator AFTER each retirement, and do not
        // republish CycleClock (MMIO does that via `note_mmio_activity`).
        // The RISC-V JIT publishes before each dispatch and clamps to the
        // next scheduler deadline; Cortex-M interpreter does neither.
        // Computed unconditionally so this loop does not grow another
        // `#[cfg(feature = "event-scheduler")]` site; the bump below is the
        // one place the feature still forks (publish_cycle is cfg-gated).
        let live_step = u64::from(config.peripheral_tick_interval > 1);

        let mut retired: u32 = 0;
        while retired < max_count {
            let mut n: u32 = 1;
            if self.jit_takeable_exception() {
                // Match interpreter `step_batch`: at executed==0 a takeable
                // pending exception is DISPATCHED by `step_internal`. After
                // progress, break so `Machine::run` can drain before the
                // next batch takes it. Never `run_ready` while takeable.
                if retired > 0 {
                    break;
                }
                self.step(bus, observers, config)?;
                engine.note_interpreted();
            } else if self.it_state != 0 {
                self.step(bus, observers, config)?;
                engine.note_interpreted();
            } else {
                let pc = self.pc as u64;
                match engine.observe(pc) {
                    Lookup::Ready => {
                        let block_n = engine.ready_instr_count(pc).unwrap_or(0);
                        let must_interpret = block_n == 0
                            || retired + block_n > max_count
                            || self.block_would_cross_irq(bus, block_n);
                        if must_interpret {
                            self.step(bus, observers, config)?;
                            engine.note_interpreted();
                        } else {
                            let ran = if let Some(sb) =
                                bus.as_any_mut().and_then(|a| a.downcast_mut::<SystemBus>())
                            {
                                let (actual_n, next_pc, clear_exclusive, needs_interp) =
                                    engine.run_ready(pc, self, &mut sb.ram.data);
                                if clear_exclusive {
                                    self.exclusive_byte = None;
                                }
                                self.pc = next_pc as u32;
                                Some((actual_n, needs_interp))
                            } else {
                                None
                            };
                            match ran {
                                Some((actual_n, needs_interp)) => {
                                    n = actual_n;
                                    if actual_n == 0 && needs_interp {
                                        self.step(bus, observers, config)?;
                                        engine.note_interpreted();
                                        n = 1;
                                    } else if actual_n > 0 {
                                        bus.systick_consume_cycles(u64::from(actual_n));
                                        if !needs_interp {
                                            // Chain to the next compiled block without
                                            // observe() (hot-counter) or interpreter.
                                            while retired + n < max_count
                                                && !self.jit_takeable_exception()
                                                && self.it_state == 0
                                            {
                                                let npc = self.pc as u64;
                                                let bn = engine.ready_instr_count(npc).unwrap_or(0);
                                                if bn == 0
                                                    || retired + n + bn > max_count
                                                    || self.block_would_cross_irq(bus, bn)
                                                {
                                                    break;
                                                }
                                                let more = if let Some(sb) = bus
                                                    .as_any_mut()
                                                    .and_then(|a| a.downcast_mut::<SystemBus>())
                                                {
                                                    let (
                                                        extra,
                                                        next_pc,
                                                        clear_exclusive,
                                                        needs_interp,
                                                    ) = engine.run_ready(
                                                        npc,
                                                        self,
                                                        &mut sb.ram.data,
                                                    );
                                                    if clear_exclusive {
                                                        self.exclusive_byte = None;
                                                    }
                                                    self.pc = next_pc as u32;
                                                    Some((extra, needs_interp))
                                                } else {
                                                    None
                                                };
                                                match more {
                                                    Some((extra, needs_interp)) => {
                                                        if extra > 0 {
                                                            engine.note_chained();
                                                            bus.systick_consume_cycles(u64::from(
                                                                extra,
                                                            ));
                                                        }
                                                        n += extra;
                                                        if extra == 0 || needs_interp {
                                                            break;
                                                        }
                                                    }
                                                    None => break,
                                                }
                                            }
                                        }
                                    }
                                }
                                None => {
                                    self.step(bus, observers, config)?;
                                    engine.note_interpreted();
                                }
                            }
                        }
                    }
                    Lookup::Interpret { promote } => {
                        if promote {
                            if let Some(sb) =
                                bus.as_any().and_then(|a| a.downcast_ref::<SystemBus>())
                            {
                                engine.try_compile_from_bus(pc, sb);
                            }
                        }
                        self.step(bus, observers, config)?;
                        engine.note_interpreted();
                    }
                }
            }

            #[cfg(feature = "event-scheduler")]
            if live_step != 0 && n != 0 {
                if let Some(sb) = bus.as_any_mut().and_then(|a| a.downcast_mut::<SystemBus>()) {
                    sb.current_cycle += live_step * n as u64;
                } else {
                    bus.publish_cycle(bus.current_cycle() + live_step * n as u64);
                }
            }
            retired += n;
            if self.sysreset_latched() {
                break;
            }
            if config.idle_fast_forward_enabled && self.idle_fast_forward_budget(bus).is_some() {
                break;
            }
        }
        Ok(retired)
    }

    pub fn jit_stats(&self) -> Option<crate::cpu::jit_framework::cortex_m::EngineStats> {
        self.jit_engine.as_ref().map(|e| e.stats())
    }
}

impl Cpu for CortexM {
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    #[cfg(feature = "jit")]
    fn jit_engine_stats(&self) -> Option<crate::CpuJitStats> {
        self.jit_stats().map(|s| crate::CpuJitStats {
            compiled: s.compiled,
            block_runs: s.block_runs,
            block_instrs: s.block_instrs,
            interpreted: s.interpreted,
        })
    }

    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        self.pc = 0x0000_0000;
        self.sp = 0x2000_0000;
        self.pending_exceptions = [0; 4];
        self.exclusive_byte = None;
        self.set_active_exception(0);
        self.decode_cache.fill(None);

        // Out of reset the core is in Thread mode using MSP (CONTROL=0); PSP
        // is architecturally UNKNOWN — start it at 0.
        self.control = 0;
        self.psp = 0;

        let vtor = self.vtor.load(Ordering::SeqCst) as u64;
        // NOT wrapped in `census_bus!`, deliberately. These two are the only
        // discards left in this file and they are named in ALLOWED_DISCARDS,
        // which is keyed on the literal source line — wrapping them changes
        // the text, so the shrink-only guard stops recognising its own two
        // documented exceptions and the whole contract test goes red. The
        // census is a measurement; it does not get to move a guard's goalposts.
        if let Ok(sp) = bus.read_u32(vtor) {
            self.sp = sp;
        }
        if let Ok(pc) = bus.read_u32(vtor + 4) {
            self.pc = pc & !1;
        }
        self.msp = self.sp;

        Ok(())
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }
    fn set_pc(&mut self, val: u32) {
        self.pc = val & !1;
    }
    fn set_sp(&mut self, val: u32) {
        self.sp = val;
        // Keep the active bank coherent (out-of-reset / external SP loads are
        // on the currently-selected stack).
        self.sync_sp_to_bank();
    }
    fn set_exception_pending(&mut self, exception_num: u32) {
        if trace_exc_enabled() {
            eprintln!("EXC pend num={} pc=0x{:08X}", exception_num, self.pc);
        }
        if exception_num < 256 {
            self.pending_exceptions[(exception_num / 64) as usize] |= 1u64 << (exception_num % 64);
        }
    }

    fn get_register(&self, id: u8) -> u32 {
        self.read_reg(id)
    }

    fn set_register(&mut self, id: u8, val: u32) {
        self.write_reg(id, val);
    }

    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::Arm(crate::snapshot::ArmCpuSnapshot {
            registers: vec![
                self.r0, self.r1, self.r2, self.r3, self.r4, self.r5, self.r6, self.r7, self.r8,
                self.r9, self.r10, self.r11, self.r12, self.sp, self.lr, self.pc,
            ],
            pc: self.pc,
            xpsr: self.xpsr,
            primask: self.primask,
            pending_exceptions: self.pending_exceptions[0],
            pending_exceptions_hi: self.pending_exceptions[1..].to_vec(),
            vtor: self.vtor.load(Ordering::Relaxed),
        })
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::Arm(s) = snapshot {
            if s.registers.len() >= 16 {
                self.r0 = s.registers[0];
                self.r1 = s.registers[1];
                self.r2 = s.registers[2];
                self.r3 = s.registers[3];
                self.r4 = s.registers[4];
                self.r5 = s.registers[5];
                self.r6 = s.registers[6];
                self.r7 = s.registers[7];
                self.r8 = s.registers[8];
                self.r9 = s.registers[9];
                self.r10 = s.registers[10];
                self.r11 = s.registers[11];
                self.r12 = s.registers[12];
                self.sp = s.registers[13];
                self.lr = s.registers[14];
                self.pc = s.pc; // Use explicit PC field
            }
            self.xpsr = s.xpsr;
            self.primask = s.primask;
            self.pending_exceptions = [0; 4];
            self.pending_exceptions[0] = s.pending_exceptions;
            for (i, w) in s.pending_exceptions_hi.iter().take(3).enumerate() {
                self.pending_exceptions[i + 1] = *w;
            }
            self.vtor.store(s.vtor, Ordering::Relaxed);
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for i in 0..13 {
            names.push(format!("R{}", i));
        }
        names.push("SP".to_string());
        names.push("LR".to_string());
        names.push("PC".to_string());
        names
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        match name.to_uppercase().as_str() {
            "R0" => Some(0),
            "R1" => Some(1),
            "R2" => Some(2),
            "R3" => Some(3),
            "R4" => Some(4),
            "R5" => Some(5),
            "R6" => Some(6),
            "R7" => Some(7),
            "R8" => Some(8),
            "R9" => Some(9),
            "R10" => Some(10),
            "R11" => Some(11),
            "R12" => Some(12),
            "SP" | "R13" => Some(13),
            "LR" | "R14" => Some(14),
            "PC" | "R15" => Some(15),
            "XPSR" => Some(16),
            _ => None,
        }
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()> {
        self.step_internal(bus, observers, config)
    }

    fn step_batch(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        #[cfg(feature = "jit")]
        {
            self.jit_enabled = config.cortex_m_jit_enabled;
            if self.jit_enabled && max_count > 1 && self.jit_gate_allows(bus, observers) {
                return self.step_batch_jit(bus, observers, config, max_count);
            }
        }

        // Push-mode logic capture: while armed, the tap clock advances once
        // per retired instruction (BEFORE executing it) so MMIO pad writes
        // stamp with the cycle boundary they become observable at. One Arc
        // clone + flag check per batch when disarmed.
        let tap = bus.logic_tap().filter(|t| t.push_armed());

        // Exact-cycle clock (issue #842) — the ARM counterpart of the
        // `exact_clock` block in `RiscV::step_batch`, which ARM never got.
        // Same contract, cheaper shape (see the accumulator note below).
        //
        // `bus.current_cycle` (and the `CycleClock` published in lock-step with
        // it) is refreshed at machine boundaries, so for the whole of a batch it
        // holds the BATCH-START cycle. Every model that advances lazily off it —
        // nRF52 TIMER/RTC, RP2040 TIMER, SysTick, DWT — therefore reads FROZEN
        // mid-batch, on both the read side (`&self` clock sync) and the write
        // side (`sync_scheduler_peripheral`, which reads `current_cycle`).
        // Firmware polling a free-running counter in a tight loop sees it stop
        // dead for a whole quantum: `TIER1 timer FAIL code=timer-not-advancing`
        // on nrf52832 / nrf52840 / rp2040.
        //
        // Advancing the cycle once per retired instruction makes those accesses
        // cycle-EXACT — instruction `i` of the batch runs at `batch_start + i`,
        // which is what interval 1 already gives, where the batch is one
        // instruction and the boundary refresh does it. That is why this is a
        // fidelity fix and not a heuristic: the poll exits on the same
        // instruction at any tick interval.
        //
        // Only ARM was affected because only ARM lacked this: RISC-V has carried
        // it since its own walk-free migration, which is why no RISC-V board is
        // on the batched-path divergence list.
        //
        // `current_cycle` is its OWN accumulator: it already holds the
        // batch-start cycle on entry, so advancing it in place AFTER each
        // retired instruction spells the whole fix as one read-modify-write
        // (`add [bus+off], reg`) and costs no loop-carried register.
        //
        // That shape is not incidental. Keeping the live cycle in a local and
        // storing it (`live = batch_start; …; bus.current_cycle = live; live +=
        // step`) needs two extra u64s alive across the `step_internal` call, so
        // both spill to the stack every iteration — measured at +3.2% Ir/step on
        // all four boards, over the 3% gate. This form measures clean.
        //
        // `live_step` is 0 at interval 1, making the update a no-op write that
        // leaves `current_cycle` pinned to batch-start for the whole batch —
        // exactly the pre-#842 behaviour, with no test-and-branch per
        // instruction.
        #[cfg(feature = "event-scheduler")]
        let live_step = u64::from(config.peripheral_tick_interval > 1);

        if !config.batch_mode_enabled {
            for i in 0..max_count {
                if let Some(tap) = &tap {
                    tap.bump_clock();
                }
                self.step(bus, observers, config)?;
                // Advance AFTER the step: the instruction just retired ran at
                // the cycle already published, and this readies the next one.
                // See the `live_step` block above.
                #[cfg(feature = "event-scheduler")]
                bus.publish_cycle(bus.current_cycle() + live_step);
                // A latched SYSRESETREQ ends the batch on the instruction that
                // wrote AIRCR, so the machine boundary applies the reset before
                // anything else retires (see `CortexM::sysreset_signal`).
                if self.sysreset_latched() {
                    return Ok(i + 1);
                }
                // WFI idle escape: leave the batch once the core is sleeping so
                // `Machine::run` can fast-forward the idle window (mirrors the
                // batch paths below and the RISC-V core).
                if config.idle_fast_forward_enabled && self.idle_fast_forward_budget(bus).is_some()
                {
                    return Ok(i + 1);
                }
            }
            return Ok(max_count);
        }

        let mut executed = 0;

        if let Some(sysbus) = bus.as_any_mut().and_then(|a| a.downcast_mut::<SystemBus>()) {
            while executed < max_count {
                // End the batch early when a takeable exception is pending:
                // its priority must be strictly higher (smaller number) than
                // the currently-active one (or 256 = thread mode baseline).
                // Only ONCE the batch has made progress (`executed > 0`) —
                // at the batch top the pending exception must instead be
                // DISPATCHED by the `step_internal` below (which takes it
                // exactly like the single-step path). Breaking at zero made
                // `Machine::run` return no-progress forever the moment a
                // walk/scheduler-pended IRQ (e.g. SysTick) became takeable
                // between batches, wedging every batched IRQ-driven Cortex-M
                // firmware (walk-free campaign B1 surfaced this — batching is
                // pointless if an armed SysTick freezes the run loop).
                if executed > 0 && self.any_exception_pending() {
                    if let Some(exc) = self.highest_priority_pending() {
                        let exc_prio = self.exception_priority(exc);
                        let active_prio = self.exception_priority(self.active_exception);
                        if !self.masked_by_primask(exc)
                            && exc_prio < active_prio
                            && !self.masked_by_basepri(exc_prio)
                            && !self.faultmask_blocks(exc)
                        {
                            break;
                        }
                    }
                }
                if let Some(tap) = &tap {
                    tap.bump_clock();
                }
                self.step_internal(sysbus, observers, config)?;
                // The hot arm. Concrete `SystemBus`, so this is one in-place
                // add on a field, not a virtual call — and deliberately NOT
                // `set_current_cycle`, whose extra job is republishing the
                // `CycleClock`. That republish is an atomic store and belongs
                // on the per-MMIO path (`note_mmio_activity`), not here.
                #[cfg(feature = "event-scheduler")]
                {
                    sysbus.current_cycle += live_step;
                }
                executed += 1;
                // See the `!batch_mode_enabled` arm: a latched SYSRESETREQ ends
                // the batch here so the reset lands on this exact boundary.
                if self.sysreset_latched() {
                    break;
                }
                // Taken branches no longer break the batch — the run loop bounds
                // it to the next peripheral tick, so bouncing back through
                // `Machine::run` at every branch was pure overhead. Only WFI
                // sleep leaves the batch, so the machine can fast-forward the
                // idle window.
                if config.idle_fast_forward_enabled
                    && self.idle_fast_forward_budget(sysbus).is_some()
                {
                    break;
                }
            }
        } else {
            while executed < max_count {
                // Same early-out rule as the SystemBus arm above: break only
                // after progress; at the batch top a takeable pending
                // exception is dispatched by `step_internal`, never spun on.
                if executed > 0 && self.any_exception_pending() {
                    if let Some(exc) = self.highest_priority_pending() {
                        let exc_prio = self.exception_priority(exc);
                        let active_prio = self.exception_priority(self.active_exception);
                        if !self.masked_by_primask(exc)
                            && exc_prio < active_prio
                            && !self.masked_by_basepri(exc_prio)
                            && !self.faultmask_blocks(exc)
                        {
                            break;
                        }
                    }
                }
                if let Some(tap) = &tap {
                    tap.bump_clock();
                }
                self.step_internal(bus, observers, config)?;
                // See the `live_step` block above.
                #[cfg(feature = "event-scheduler")]
                bus.publish_cycle(bus.current_cycle() + live_step);
                executed += 1;
                if self.sysreset_latched() {
                    break;
                }
                if config.idle_fast_forward_enabled && self.idle_fast_forward_budget(bus).is_some()
                {
                    break;
                }
            }
        }

        Ok(executed)
    }

    fn idle_fast_forward_budget(&self, _bus: &dyn Bus) -> Option<u64> {
        // Only fast-forward while the core sleeps in WFI and no wake-up event
        // has arrived. A pending wake exception (evaluated ignoring PRIMASK)
        // resumes normal execution: the machine must re-enter `step` so the
        // core either takes the exception or, under PRIMASK, falls through it.
        if !self.sleeping || self.wfi_wake_pending() {
            return None;
        }
        // Cortex-M has no core-local timer to bound the skip (SysTick lives on
        // the bus). Offer an unbounded budget; `Machine::run` clamps it to the
        // next scheduler deadline, exactly as it does for a RISC-V core whose
        // mtimecmp is disabled.
        Some(u64::MAX)
    }

    fn fast_forward_idle_cycles(&mut self, _cycles: u64) {
        // Cortex-M keeps no core-local cycle counter (unlike RISC-V mtime); the
        // machine owns `total_cycles` and advances it. Nothing to do here.
    }
}

/// Width of a Cortex-M data-side memory access.
///
/// The third argument to [`CortexM::load`] / [`CortexM::store`], which are the
/// only two doors through which this core touches the data bus.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AccessWidth {
    Byte,
    Half,
    Word,
}

impl CortexM {
    /// The ONE data-side load on this core.
    ///
    /// `bus/accessors.rs` returns `Err(SimulationError::MemoryViolation(addr))`
    /// for any address no memory region or peripheral window covers. This helper
    /// propagates that `Err` to the caller, which propagates it out of
    /// `step_internal` with `?` — the same contract `RiscV::step_internal` has
    /// always had, and the reason the same firmware bug is fatal there.
    ///
    /// It exists so there is exactly one place where a Cortex-M load can decide
    /// what a failed access means. Before it, 41 call sites decided
    /// independently, and every one of them decided "pretend it worked": a
    /// failed load left the destination register holding its previous value and
    /// the run continued green.
    ///
    /// On top of that contract it **latches the faulting address** in
    /// `pending_data_fault`, which is what lets `step_internal` tell a precise
    /// *data-access* fault — the one ARMv7-M B1.5.14 turns into a BusFault —
    /// apart from an instruction-fetch fault, an exception-entry stacking fault
    /// or a vector-table read fault. Those are different architectural
    /// contracts with different status bits, and they stay on the abort path;
    /// see [`CortexM::bus_load`].
    #[inline(always)]
    fn load<B: Bus + ?Sized>(&mut self, bus: &B, addr: u32, width: AccessWidth) -> SimResult<u32> {
        match Self::bus_load(bus, addr, width) {
            Err(SimulationError::MemoryViolation(a)) => {
                self.pending_data_fault = Some(a as u32);
                Err(SimulationError::MemoryViolation(a))
            }
            other => other,
        }
    }

    /// The ONE data-side store on this core. Counterpart to [`CortexM::load`];
    /// see that doc for why. A failed store used to vanish at 25 `let _ =
    /// bus.write_*` sites.
    #[inline(always)]
    fn store<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        addr: u32,
        width: AccessWidth,
        value: u32,
    ) -> SimResult<()> {
        match Self::bus_store(bus, addr, width, value) {
            Err(SimulationError::MemoryViolation(a)) => {
                self.pending_data_fault = Some(a as u32);
                Err(SimulationError::MemoryViolation(a))
            }
            other => other,
        }
    }

    /// The raw bus load, **without** latching a data fault. Propagates `Err`
    /// exactly like [`CortexM::load`] — it is not a discard — but the failure
    /// will not be escalated into a BusFault.
    ///
    /// Used by the two accesses that are architecturally *not* precise
    /// data-access faults:
    ///   * exception-entry stacking, which on silicon raises
    ///     `BFSR.STKERR` and can end in LOCKUP rather than a recoverable
    ///     handler entry (ARMv7-M B1.5.15);
    ///   * the vector-table read, which raises `HFSR.VECTTBL`.
    ///
    /// Both are separate contracts with their own blast radius; escalating them
    /// here would also risk an unbounded re-entry loop, since the stack or the
    /// vector table is exactly what is broken.
    #[inline(always)]
    fn bus_load<B: Bus + ?Sized>(bus: &B, addr: u32, width: AccessWidth) -> SimResult<u32> {
        Ok(match width {
            AccessWidth::Byte => bus.read_u8(addr as u64)? as u32,
            AccessWidth::Half => bus.read_u16(addr as u64)? as u32,
            AccessWidth::Word => bus.read_u32(addr as u64)?,
        })
    }

    /// The raw bus store, without latching a data fault. See [`CortexM::bus_load`].
    #[inline(always)]
    fn bus_store<B: Bus + ?Sized>(
        bus: &mut B,
        addr: u32,
        width: AccessWidth,
        value: u32,
    ) -> SimResult<()> {
        match width {
            AccessWidth::Byte => bus.write_u8(addr as u64, value as u8),
            AccessWidth::Half => bus.write_u16(addr as u64, value as u16),
            AccessWidth::Word => bus.write_u32(addr as u64, value),
        }
    }

    /// ARMv7-M **execution priority** (B1.5.4 "Execution priority and priority
    /// boosting"): the priority of the currently executing code, which an
    /// exception must beat (numerically smaller) to be taken.
    ///
    /// It is the minimum of the active exception's priority and the three
    /// priority-boosting registers: `FAULTMASK` boosts to -1, `PRIMASK` to 0,
    /// and a non-zero `BASEPRI` to its own value. Thread mode with nothing
    /// boosting is 256, the "lower than anything configurable" baseline the rest
    /// of this file already uses.
    fn execution_priority(&self) -> i32 {
        let mut prio = self.exception_priority(self.active_exception);
        if self.faultmask {
            prio = prio.min(-1);
        }
        if self.primask {
            prio = prio.min(0);
        }
        if self.basepri != 0 {
            prio = prio.min(self.basepri as i32);
        }
        prio
    }

    /// True when PRIMASK blocks taking `exc`.
    ///
    /// With fault modelling **off** this is exactly `self.primask` — the blanket
    /// guard this core has always used, so nothing changes. With it **on**,
    /// PRIMASK is modelled as what ARMv7-M B1.5.4 says it is: a boost of the
    /// execution priority to 0, which by construction cannot mask NMI (-2) or
    /// HardFault (-1). Without this, an escalated HardFault raised inside a
    /// `__disable_irq()` critical section would pend and never dispatch, and the
    /// core would re-execute the faulting instruction forever.
    #[inline(always)]
    fn masked_by_primask(&self, exc: u32) -> bool {
        if !self.primask {
            return false;
        }
        if !self.faults_enabled() {
            return true;
        }
        self.exception_priority(exc) >= 0
    }

    /// ARMv7-M B1.5.14 escalation for a **precise data-access fault**.
    ///
    /// Records the fault in the status registers firmware actually reads, then
    /// decides between BusFault and HardFault:
    ///
    /// * `CFSR.BFSR.PRECISERR` (B3.2.15) — the access was synchronous with the
    ///   instruction, so the stacked PC is the faulting instruction.
    /// * `CFSR.BFSR.BFARVALID` + `BFAR` (B3.2.15 / B3.2.17) — BFAR holds the
    ///   address the access faulted on.
    /// * `HFSR.FORCED` (B3.2.16) — set **only** when the fault escalates.
    ///
    /// The escalation rule (B1.5.14): *"a fault occurs and the handler for that
    /// fault is not enabled"*, and *"an exception handler causes a fault for
    /// which the priority is the same as or lower than the currently executing
    /// exception"*. Both collapse to: pend BusFault(5) if `SHCSR.BUSFAULTENA` is
    /// set **and** BusFault's priority beats the current execution priority;
    /// otherwise pend HardFault(3) with `HFSR.FORCED`.
    ///
    /// Returns `false` when even HardFault cannot be taken. On silicon that is
    /// LOCKUP (B1.5.15), which this model does not have; the caller then leaves
    /// the original `Err` to stop the run, rather than pending an exception that
    /// can never dispatch and spinning on the faulting instruction forever.
    fn escalate_precise_data_fault(&mut self, addr: u32) -> bool {
        let Some(faults) = self.faults.clone() else {
            return false;
        };
        faults
            .cfsr
            .fetch_or(CFSR_BFSR_PRECISERR | CFSR_BFSR_BFARVALID, Ordering::Relaxed);
        faults.bfar.store(addr, Ordering::Relaxed);

        let busfault_enabled = faults.shcsr.load(Ordering::Relaxed) & SHCSR_BUSFAULTENA != 0;
        let exec_prio = self.execution_priority();
        let target = if busfault_enabled && self.exception_priority(5) < exec_prio {
            5
        } else {
            // Escalated: the HardFault handler needs to know it is standing in
            // for a configurable fault, and CFSR above tells it which one.
            faults.hfsr.fetch_or(HFSR_FORCED, Ordering::Relaxed);
            3
        };
        if self.exception_priority(target) >= exec_prio {
            return false; // LOCKUP — see the doc comment.
        }
        if trace_exc_enabled() {
            eprintln!(
                "EXC fault addr=0x{:08X} -> exc={} pc=0x{:08X}",
                addr, target, self.pc
            );
        }
        self.set_exception_pending(target);
        true
    }

    /// ARMv7-M B1.5.14 escalation for an **undefined instruction**.
    ///
    /// Same shape as [`CortexM::escalate_precise_data_fault`], with UsageFault
    /// in place of BusFault and no fault address — UFSR has no companion to
    /// BFAR:
    ///
    /// * `CFSR.UFSR.UNDEFINSTR` (B3.2.15) — the processor attempted to execute
    ///   an instruction it does not define.
    /// * `HFSR.FORCED` (B3.2.16) — set only when the fault escalates, i.e. when
    ///   `SHCSR.USGFAULTENA` is clear or UsageFault cannot preempt.
    ///
    /// Returns `false` when even HardFault cannot be taken (LOCKUP on silicon,
    /// B1.5.15). The caller then stops the run rather than pending an exception
    /// that can never dispatch and spinning on the faulting instruction.
    fn escalate_undefined_instruction(&mut self) -> bool {
        let Some(faults) = self.faults.clone() else {
            return false;
        };
        faults
            .cfsr
            .fetch_or(CFSR_UFSR_UNDEFINSTR, Ordering::Relaxed);

        let usagefault_enabled = faults.shcsr.load(Ordering::Relaxed) & SHCSR_USGFAULTENA != 0;
        let exec_prio = self.execution_priority();
        let target = if usagefault_enabled && self.exception_priority(6) < exec_prio {
            6
        } else {
            faults.hfsr.fetch_or(HFSR_FORCED, Ordering::Relaxed);
            3
        };
        if self.exception_priority(target) >= exec_prio {
            return false; // LOCKUP — see the doc comment.
        }
        if trace_exc_enabled() {
            eprintln!(
                "EXC undefined instruction -> exc={} pc=0x{:08X}",
                target, self.pc
            );
        }
        self.set_exception_pending(target);
        true
    }

    /// One instruction, with ARMv7-M fault escalation layered over
    /// [`CortexM::step_execute`].
    ///
    /// A precise data-access fault leaves the PC on the faulting instruction and
    /// returns `Ok(())`: the exception is pended here and *dispatched* by the
    /// next `step_execute`, whose entry block stacks a frame whose return
    /// address is the faulting instruction — which is what B1.5.6 requires for a
    /// synchronous fault.
    ///
    /// With fault modelling off (the `LABWIRED_CORTEXM_FAULTS` opt-out) this is
    /// `step_execute` verbatim:
    /// `pending_data_fault` is only ever written on the error path, and the
    /// `Err` is returned unchanged.
    #[inline(always)]
    fn step_internal<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()> {
        match self.step_execute(bus, observers, config) {
            Err(e) => {
                // `take` unconditionally: the latch must not survive into the
                // next step even when escalation is off.
                let undef = std::mem::take(&mut self.pending_undef_instruction);
                match self.pending_data_fault.take() {
                    Some(addr)
                        if self.faults_enabled() && self.escalate_precise_data_fault(addr) =>
                    {
                        Ok(())
                    }
                    // An undefined instruction escalates to UsageFault, or to
                    // HardFault when UsageFault is not enabled. Escalation
                    // failing means LOCKUP on silicon, so the `Err` stands and
                    // stops the run — which is still incomparably better than
                    // the old behaviour of advancing the PC and continuing.
                    _ if undef
                        && self.faults_enabled()
                        && self.escalate_undefined_instruction() =>
                    {
                        Ok(())
                    }
                    _ => Err(e),
                }
            }
            ok => ok,
        }
    }

    #[inline(always)]
    fn step_execute<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        _observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()> {
        // Leave WFI sleep before this step commits: the flag is re-armed only if
        // this instruction is itself a WFI with no wake event pending.
        self.sleeping = false;
        // Check for pending exceptions before executing instruction.
        // Use real ARMv7-M priority dispatch: pick the highest-priority
        // pending exception (smallest numeric priority value), and only
        // take it if its priority is strictly higher than the currently
        // active exception's. This is the dispatch path that makes
        // FreeRTOS PendSV-driven context switches behave correctly —
        // PendSV at priority 0xFF only runs when no other ISR is active.
        //
        // Ask "is anything pending at all" FIRST. `highest_priority_pending`
        // walks the pending bitmap through a slice iterator and consults
        // `exception_priority` per set bit; with nothing pending — every
        // instruction of every firmware that is not currently taking an
        // interrupt — it does all of that to return `None`. Measured at 60
        // Ir per instruction on nrf52840, ~26 % of the whole per-step cost
        // (docs/performance/2026-09-17-bus-scheduler-pass.md). The guard is
        // behaviour-preserving by construction: with no bit set the old
        // `unwrap_or(0)` produced `exception_num == 0`, and the `&& exception_num
        // != 0` arm below already made the whole block dead.
        let exception_num = if self.any_exception_pending() {
            self.highest_priority_pending().unwrap_or(0)
        } else {
            0
        };
        if exception_num != 0 && !self.masked_by_primask(exception_num) {
            let take_prio = self.exception_priority(exception_num);
            let active_prio = self.exception_priority(self.active_exception);
            let can_take = take_prio < active_prio
                && !self.masked_by_basepri(take_prio)
                && !self.faultmask_blocks(exception_num);

            if can_take {
                // For NVIC-routed exceptions (num >= 16): verify the NVIC ISPR bit is
                // still set before taking the exception.  Firmware may have called
                // NVIC_ClearPendingIRQ (writing NVIC ICPR) while the ISR was active,
                // which clears ISPR but leaves our cpu-side `pending_exceptions` stale.
                // Without this check the stale bit causes a spurious second ISR after the
                // real one returns.  On real ARM Cortex-M the hardware never re-latches a
                // pending bit whose ISPR was cleared by software before ISR exit.
                if exception_num >= 16 && !bus.is_nvic_irq_pending(exception_num) {
                    // Stale pending_exceptions bit — drop it without taking the exception.
                    self.pending_exceptions[(exception_num / 64) as usize] &=
                        !(1u64 << (exception_num % 64));
                    // Fall through to normal instruction execution.
                } else {
                    self.exclusive_byte = None;
                    self.pending_exceptions[(exception_num / 64) as usize] &=
                        !(1u64 << (exception_num % 64));

                    // Clear NVIC ISPR for this exception so it isn't immediately re-pended.
                    // On real ARM hardware this happens automatically when the exception is taken.
                    bus.clear_nvic_pending(exception_num);

                    // Capture the entry context BEFORE switching to Handler mode:
                    // which mode/stack we came from determines EXC_RETURN.
                    let entered_from_handler = self.active_exception != 0;
                    let entry_on_psp = self.use_psp();

                    // Perform Stacking on the CURRENT (preempted) stack.
                    let sp = self.sp;
                    let frame_ptr = sp.wrapping_sub(32);

                    // Save the previous active_exception in xPSR IPSR bits [8:0] so that
                    // exception_return can restore the correct nesting level.
                    let save_xpsr =
                        self.xpsr_with_itstate((self.xpsr & !0x1FF) | self.active_exception);

                    // Stack: R0, R1, R2, R3, R12, LR, PC, xPSR (with previous IPSR)
                    let stacked_lr = self.lr;
                    let stacked_pc = self.pc;
                    // Stacking is a data-side store like any other: if the frame
                    // does not fit in mapped memory the write must surface, not
                    // vanish. `exception_return`'s matching unstacking loads have
                    // always propagated with `?`; this makes entry symmetric.
                    // `bus_store`, not `store`: a stacking failure is
                    // BFSR.STKERR / LOCKUP territory (B1.5.15), not a precise
                    // data-access fault, and escalating it would re-enter this
                    // same broken stack forever. See `CortexM::bus_load`.
                    Self::bus_store(bus, frame_ptr, AccessWidth::Word, self.r0)?;
                    Self::bus_store(bus, frame_ptr.wrapping_add(4), AccessWidth::Word, self.r1)?;
                    Self::bus_store(bus, frame_ptr.wrapping_add(8), AccessWidth::Word, self.r2)?;
                    Self::bus_store(bus, frame_ptr.wrapping_add(12), AccessWidth::Word, self.r3)?;
                    Self::bus_store(bus, frame_ptr.wrapping_add(16), AccessWidth::Word, self.r12)?;
                    Self::bus_store(bus, frame_ptr.wrapping_add(20), AccessWidth::Word, self.lr)?;
                    Self::bus_store(bus, frame_ptr.wrapping_add(24), AccessWidth::Word, self.pc)?;
                    Self::bus_store(
                        bus,
                        frame_ptr.wrapping_add(28),
                        AccessWidth::Word,
                        save_xpsr,
                    )?;

                    // Bank the preempted stack pointer into its bank (PSP or MSP)
                    // BEFORE entering Handler mode, then switch the live `sp` to MSP.
                    if entry_on_psp {
                        self.psp = frame_ptr;
                    } else {
                        self.msp = frame_ptr;
                    }

                    // Update active exception (→ Handler mode) so nested exceptions
                    // see the correct level. Handler always runs on MSP.
                    self.set_active_exception(exception_num);
                    self.sp = self.msp;
                    self.it_state = 0;

                    // EXC_RETURN encodes the mode/stack to restore on return:
                    //   0xFFFFFFF1 → return to Handler mode (nested), frame on MSP
                    //   0xFFFFFFF9 → return to Thread/MSP
                    //   0xFFFFFFFD → return to Thread/PSP
                    self.lr = if entered_from_handler {
                        0xFFFF_FFF1
                    } else if entry_on_psp {
                        0xFFFF_FFFD
                    } else {
                        0xFFFF_FFF9
                    };

                    // Jump to ISR handler
                    let vtor = self.vtor.load(Ordering::SeqCst);
                    let vector_addr = vtor.wrapping_add(exception_num.wrapping_mul(4));
                    if trace_exc_enabled() {
                        eprintln!(
                            "EXC take num={} vtor=0x{:08X} vec=0x{:08X} fetch={:?}",
                            exception_num,
                            vtor,
                            vector_addr,
                            bus.read_u32(vector_addr as u64)
                        );
                    }
                    // `bus_load`: a failed vector-table read is HFSR.VECTTBL,
                    // not a precise data-access fault. See `CortexM::bus_load`.
                    let handler = Self::bus_load(bus, vector_addr, AccessWidth::Word)?;
                    self.pc = handler & !1;
                    tracing::debug!(
                        "EXC_ENTRY: exc={} handler={:#010x} frame={:#010x} stacked_lr={:#010x} stacked_pc={:#010x}",
                        exception_num, self.pc, frame_ptr, stacked_lr, stacked_pc
                    );

                    return Ok(());
                } // end else (NVIC ISPR still set — take the exception)
            }
            // Can't take this exception right now (lower priority than active).
            // Fall through and execute the current instruction normally.
        }
        // Fetch/Decode with optional Cache
        let cache_idx = ((self.pc >> 1) & 0xFFF) as usize;
        let entry = if config.decode_cache_enabled {
            if let Some(e) = self.decode_cache[cache_idx] {
                if e.tag == self.pc {
                    Some(e)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let (instruction, opcode, mut pc_increment, _cycles) = if let Some(e) = entry {
            (e.instruction, e.opcode, e.pc_increment as u32, e.cycles)
        } else {
            let fetch_pc = self.pc & !1;
            let h1 = bus.read_u16(fetch_pc as u64)?;
            let is_32bit = (h1 & 0xE000) == 0xE000 && (h1 & 0x1800) != 0;

            let (instr, op, pincr, cyc) = if is_32bit {
                let h2 = bus.read_u16(fetch_pc.wrapping_add(2) as u64)?;
                let instr = decode_thumb_32(h1, h2);
                let op = ((h1 as u32) << 16) | h2 as u32;
                (instr, op, 4, 2)
            } else {
                let instr = decode_thumb_16(h1);
                (instr, h1 as u32, 2, 1)
            };

            if config.decode_cache_enabled {
                self.decode_cache[cache_idx] = Some(DecodeCacheEntry {
                    tag: self.pc,
                    instruction: instr,
                    opcode: op,
                    pc_increment: pincr as u8,
                    cycles: cyc,
                });
            }

            (instr, op, pincr as u32, cyc)
        };

        // Per-instruction PC trace gated on LABWIRED_TRACE_INSN env var.
        // Use only for short runs — VERY chatty. Format suitable for grepping:
        //   INSN pc=0xPPPPPPPP op=0xOOOOOOOO
        if trace_insn_enabled() {
            eprintln!("INSN pc=0x{:08X} op=0x{:08X}", self.pc, opcode);
        }

        let retired_pc = self.pc;
        if !_observers.is_empty() {
            for observer in _observers {
                observer.on_step_start(self.pc, opcode);
            }
        }

        let mut execute = true;
        let mut it_block_instruction = false;

        if self.it_state != 0 {
            it_block_instruction = true;
            let cond = self.it_state >> 4;
            execute = self.check_condition(cond);
        }

        if execute {
            #[cfg(debug_assertions)]
            tracing::debug!(
                "PC={:#x}, Opcode={:#04x}, Instr={:?}",
                self.pc,
                opcode,
                instruction
            );

            // Execute
            match instruction {
                Instruction::Bfi { rd, rn, lsb, width } => {
                    let src = self.read_reg(rn);
                    let dst = self.read_reg(rd);
                    let mask = if width == 32 {
                        !0
                    } else {
                        ((1u32.wrapping_shl(width as u32)).wrapping_sub(1)).wrapping_shl(lsb as u32)
                    };
                    let result = (dst & !mask) | ((src.wrapping_shl(lsb as u32)) & mask);
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Bfc { rd, lsb, width } => {
                    let dst = self.read_reg(rd);
                    let mask = if width == 32 {
                        !0
                    } else {
                        ((1u32.wrapping_shl(width as u32)).wrapping_sub(1)).wrapping_shl(lsb as u32)
                    };
                    let result = dst & !mask;
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Sbfx { rd, rn, lsb, width } => {
                    let src = self.read_reg(rn);
                    let width_mask = if width == 32 {
                        !0
                    } else {
                        (1u32.wrapping_shl(width as u32)).wrapping_sub(1)
                    };
                    let val = (src.wrapping_shr(lsb as u32)) & width_mask;
                    let result = if width == 32 {
                        val
                    } else {
                        let shift = 32 - width;
                        ((val.wrapping_shl(shift as u32)) as i32).wrapping_shr(shift as u32) as u32
                    };
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Ubfx { rd, rn, lsb, width } => {
                    let src = self.read_reg(rn);
                    let width_mask = if width == 32 {
                        !0
                    } else {
                        (1u32.wrapping_shl(width as u32)).wrapping_sub(1)
                    };
                    let result = (src.wrapping_shr(lsb as u32)) & width_mask;
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Clz { rd, rm } => {
                    let val = self.read_reg(rm);
                    let result = val.leading_zeros();
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Rbit { rd, rm } => {
                    let val = self.read_reg(rm);
                    let result = val.reverse_bits();
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::SimdAddSub8 { rd, rn, rm, op } => {
                    // Per-byte parallel add/sub; each lane sets one APSR.GE bit.
                    // op: 0=SADD8 1=UADD8 2=SSUB8 3=USUB8 (ARMv7-M A7.7).
                    let n = self.read_reg(rn);
                    let m = self.read_reg(rm);
                    let mut result = 0u32;
                    let mut ge = 0u32;
                    for i in 0..4 {
                        let nb = ((n >> (i * 8)) & 0xFF) as i32;
                        let mb = ((m >> (i * 8)) & 0xFF) as i32;
                        let (byte, ge_bit) = match op {
                            0 => {
                                // SADD8: signed add, GE = sum >= 0
                                let s = (nb as i8 as i32) + (mb as i8 as i32);
                                ((s as u32) & 0xFF, s >= 0)
                            }
                            1 => {
                                // UADD8: unsigned add, GE = carry out (sum >= 0x100)
                                let s = nb + mb;
                                ((s as u32) & 0xFF, s >= 0x100)
                            }
                            2 => {
                                // SSUB8: signed sub, GE = diff >= 0
                                let d = (nb as i8 as i32) - (mb as i8 as i32);
                                ((d as u32) & 0xFF, d >= 0)
                            }
                            _ => {
                                // USUB8: unsigned sub, GE = no borrow (nb >= mb)
                                let d = nb - mb;
                                ((d as u32) & 0xFF, nb >= mb)
                            }
                        };
                        result |= byte << (i * 8);
                        if ge_bit {
                            ge |= 1 << i;
                        }
                    }
                    self.write_reg(rd, result);
                    self.set_ge(ge);
                    pc_increment = 4;
                }
                Instruction::SimdAddSub16 {
                    rd,
                    rn,
                    rm,
                    op,
                    sub,
                } => {
                    // Per-halfword parallel add/sub (ARMv7-M A7.7). Two lanes,
                    // each 16 bits; the S/U variants set two APSR.GE bits per
                    // lane, the saturating and halving variants set none.
                    let n = self.read_reg(rn);
                    let m = self.read_reg(rm);
                    let mut result = 0u32;
                    let mut ge = 0u32;
                    let sets_ge = op == 0x0 || op == 0x4;
                    for i in 0..2 {
                        let nh = (n >> (i * 16)) & 0xFFFF;
                        let mh = (m >> (i * 16)) & 0xFFFF;
                        // Signed lane operands (for the S/Q/SH variants).
                        let ns = nh as u16 as i16 as i32;
                        let ms = mh as u16 as i16 as i32;
                        let (half, ge_bit) = match op {
                            // SADD16 / SSUB16: signed, wrapping. GE per lane is
                            // "result was non-negative".
                            0x0 => {
                                let s = if sub { ns - ms } else { ns + ms };
                                ((s as u32) & 0xFFFF, s >= 0)
                            }
                            // QADD16 / QSUB16: signed saturating to i16.
                            0x1 => {
                                let s = if sub { ns - ms } else { ns + ms };
                                ((s.clamp(-32768, 32767) as u32) & 0xFFFF, false)
                            }
                            // SHADD16 / SHSUB16: signed halving — the sum is
                            // 17-bit and the result is its bits [16:1], which an
                            // arithmetic shift of the i32 gives directly.
                            0x2 => {
                                let s = if sub { ns - ms } else { ns + ms };
                                (((s >> 1) as u32) & 0xFFFF, false)
                            }
                            // UADD16 / USUB16: unsigned, wrapping. GE is carry
                            // out for the add and "no borrow" for the subtract.
                            0x4 => {
                                if sub {
                                    ((nh.wrapping_sub(mh)) & 0xFFFF, nh >= mh)
                                } else {
                                    let s = nh + mh;
                                    (s & 0xFFFF, s >= 0x1_0000)
                                }
                            }
                            // UQADD16 / UQSUB16: unsigned saturating to u16.
                            0x5 => {
                                if sub {
                                    (nh.saturating_sub(mh), false)
                                } else {
                                    ((nh + mh).min(0xFFFF), false)
                                }
                            }
                            // UHADD16 / UHSUB16: unsigned halving — bits [16:1]
                            // of the 17-bit intermediate, so the difference is
                            // masked to 17 bits before the shift rather than
                            // sign-extended across the whole word.
                            _ => {
                                let s = if sub {
                                    ((nh as i32 - mh as i32) as u32) & 0x1_FFFF
                                } else {
                                    nh + mh
                                };
                                ((s >> 1) & 0xFFFF, false)
                            }
                        };
                        result |= half << (i * 16);
                        if ge_bit {
                            ge |= 0b11 << (i * 2);
                        }
                    }
                    self.write_reg(rd, result);
                    if sets_ge {
                        self.set_ge(ge);
                    }
                    pc_increment = 4;
                }
                Instruction::Sel { rd, rn, rm } => {
                    // SEL: pick each byte from Rn if its GE bit is set, else Rm.
                    let n = self.read_reg(rn);
                    let m = self.read_reg(rm);
                    let ge = self.get_ge();
                    let mut result = 0u32;
                    for i in 0..4 {
                        let src = if (ge >> i) & 1 == 1 { n } else { m };
                        result |= ((src >> (i * 8)) & 0xFF) << (i * 8);
                    }
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Sdiv { rd, rn, rm } => {
                    let n = self.read_reg(rn) as i32;
                    let m = self.read_reg(rm) as i32;
                    let result = if m == 0 {
                        0
                    } else if n == i32::MIN && m == -1 {
                        i32::MIN as u32
                    } else {
                        (n / m) as u32
                    };
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Udiv { rd, rn, rm } => {
                    let n = self.read_reg(rn);
                    let m = self.read_reg(rm);
                    let result = n.checked_div(m).unwrap_or(0);
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::DataProc32 {
                    op,
                    rn,
                    rd,
                    rm,
                    imm5,
                    shift_type,
                    set_flags,
                } => {
                    let op2_raw = self.read_reg(rm);
                    let mut op2 = op2_raw;
                    match shift_type {
                        0 => op2 = op2.wrapping_shl(imm5 as u32), // LSL
                        1 => {
                            op2 = if imm5 == 0 {
                                0
                            } else {
                                op2.wrapping_shr(imm5 as u32)
                            }
                        } // LSR
                        2 => {
                            op2 = if imm5 == 0 {
                                if (op2 & 0x80000000) != 0 {
                                    0xFFFFFFFF
                                } else {
                                    0
                                }
                            } else {
                                ((op2 as i32) >> (imm5 as u32)) as u32
                            }
                        } // ASR
                        3 if imm5 != 0 => op2 = op2.rotate_right(imm5 as u32), // ROR
                        _ => {}
                    }
                    let op1 = self.read_reg(rn);
                    let carry_in = self.get_carry();
                    // (result, carry-out, overflow). For logical ops C/V are the
                    // preserved current flags; the barrel-shifter carry-out is not
                    // tracked here (NOTE: logical-op C reflects the prior C, not the
                    // shifter carry — only N/Z are meaningful for them). Arithmetic
                    // ops compute true NZCV via the shared add/sub-with-flags helpers.
                    let (result, c, v) = match op {
                        0x0 => (op1 & op2, carry_in, self.get_overflow()), // AND / TST
                        0x1 => (op1 & !op2, carry_in, self.get_overflow()), // BIC
                        0x2 => {
                            let r = if rn == 0xF { op2 } else { op1 | op2 };
                            (r, carry_in, self.get_overflow())
                        } // ORR / MOV
                        0x3 => {
                            let r = if rn == 0xF { !op2 } else { op1 | !op2 };
                            (r, carry_in, self.get_overflow())
                        } // ORN / MVN
                        0x4 => (op1 ^ op2, carry_in, self.get_overflow()), // EOR / TEQ
                        0x6 => {
                            // PKH (PKHBT/PKHTB): pack halfwords. The tb bit lives in
                            // shift_type bit1 (0 => PKHBT keep op1 low / Rm high,
                            // 2 => PKHTB keep op1 high / Rm low). The optional barrel
                            // shift on Rm is not applied here (PKH is off the bignum
                            // path and only the imm5==0 form is exercised); operands
                            // come from the raw Rm. Not flag-setting.
                            let r = if shift_type == 2 {
                                (op1 & 0xFFFF_0000) | (op2_raw & 0x0000_FFFF)
                            } else {
                                (op1 & 0x0000_FFFF) | (op2_raw & 0xFFFF_0000)
                            };
                            (r, carry_in, self.get_overflow())
                        } // PKH
                        0x8 => add_with_flags(op1, op2),                   // ADD / CMN
                        0xA => adc_with_flags(op1, op2, carry_in as u32),  // ADC
                        0xB => sbc_with_flags(op1, op2, carry_in as u32),  // SBC
                        0xD => sub_with_flags(op1, op2),                   // SUB / CMP
                        0xE => sub_with_flags(op2, op1),                   // RSB (op2 - op1)
                        _ => {
                            #[cfg(debug_assertions)]
                            tracing::warn!("Unknown DataProc32 op {:#x}", op);
                            (op2, carry_in, self.get_overflow())
                        }
                    };
                    if rd != 15 {
                        self.write_reg(rd, result);
                    }
                    if set_flags {
                        self.update_nzcv(result, c, v);
                    }
                    pc_increment = 4;
                }
                Instruction::DataProcImm32 {
                    op,
                    rn,
                    rd,
                    imm12,
                    set_flags,
                } => {
                    let imm = thumb_expand_imm(imm12);
                    let val1 = self.read_reg(rn);
                    let (res, c, v) = match op {
                        0x0 => (val1 & imm, self.get_carry(), self.get_overflow()), // AND
                        0x1 => (val1 & !imm, self.get_carry(), self.get_overflow()), // BIC
                        0x2 => {
                            let res = if rn == 0xF { imm } else { val1 | imm };
                            (res, self.get_carry(), self.get_overflow())
                        } // ORR / MOV
                        0x3 => {
                            let res = if rn == 0xF { !imm } else { val1 | !imm };
                            (res, self.get_carry(), self.get_overflow())
                        } // ORN / MVN
                        0x4 => (val1 ^ imm, self.get_carry(), self.get_overflow()), // EOR
                        0x8 => add_with_flags(val1, imm),                           // ADD
                        0xA => adc_with_flags(val1, imm, self.get_carry() as u32),  // ADC
                        0xB => sbc_with_flags(val1, imm, self.get_carry() as u32),  // SBC
                        0xD => sub_with_flags(val1, imm),                           // SUB / CMP
                        0xE => sub_with_flags(imm, val1),                           // RSB
                        _ => {
                            tracing::warn!("Unhandled T32 DataProcImm32 op {:#x}", op);
                            (0, self.get_carry(), self.get_overflow())
                        }
                    };

                    if rd != 15 {
                        self.write_reg(rd, res);
                    }
                    if set_flags {
                        self.update_nzcv(res, c, v);
                    }
                    pc_increment = 4;
                }
                Instruction::ShiftReg32 {
                    rd,
                    rn,
                    rm,
                    shift_type,
                } => {
                    let value = self.read_reg(rn);
                    let shift = self.read_reg(rm) & 0xFF;
                    let result = match shift_type {
                        0 => {
                            if shift >= 32 {
                                0
                            } else {
                                value.wrapping_shl(shift)
                            }
                        }
                        1 => {
                            if shift == 0 {
                                value
                            } else if shift >= 32 {
                                0
                            } else {
                                value.wrapping_shr(shift)
                            }
                        }
                        2 => {
                            if shift == 0 {
                                value
                            } else if shift >= 32 {
                                if (value & 0x8000_0000) != 0 {
                                    0xFFFF_FFFF
                                } else {
                                    0
                                }
                            } else {
                                ((value as i32) >> shift) as u32
                            }
                        }
                        3 => {
                            if shift == 0 {
                                value
                            } else {
                                value.rotate_right(shift % 32)
                            }
                        }
                        _ => value,
                    };
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Movw { rd, imm } => {
                    self.write_reg(rd, imm as u32);
                    pc_increment = 4;
                }
                Instruction::Movt { rd, imm } => {
                    let old_val = self.read_reg(rd);
                    let new_val = (old_val & 0x0000FFFF) | ((imm as u32) << 16);
                    self.write_reg(rd, new_val);
                    pc_increment = 4;
                }
                Instruction::LdrImm32 { rt, rn, imm12 } => {
                    // When rn==PC, ARM spec requires Align(PC+4, 4) as base (literal load)
                    let base = if rn == 15 {
                        (self.pc.wrapping_add(4)) & !3
                    } else {
                        self.read_reg(rn)
                    };
                    let addr = base.wrapping_add(imm12 as u32);
                    let val = self.load(bus, addr, AccessWidth::Word)?;
                    if rt == 15 {
                        // LDR PC, [...] is an interworking branch — must go through branch_to
                        self.branch_to(val, bus)?;
                        pc_increment = 0;
                    } else {
                        self.write_reg(rt, val);
                    }
                    // pc_increment stays at 4 (set by decode) unless we took a branch above
                }
                Instruction::StrImm32 { rt, rn, imm12 } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm12 as u32);
                    let val = self.read_reg(rt);
                    self.store(bus, addr, AccessWidth::Word, val)?;
                    pc_increment = 4;
                }
                Instruction::LdrImm32Idx {
                    rt,
                    rn,
                    imm8,
                    pre_index,
                    add,
                    writeback,
                } => {
                    // LDR T4 indexed. Offset address = base ± imm8. The access
                    // uses the offset address when pre_index, else the base
                    // (post-index). Writeback stores the offset address in Rn.
                    let base = self.read_reg(rn);
                    let offset = imm8 as u32;
                    let offset_addr = if add {
                        base.wrapping_add(offset)
                    } else {
                        base.wrapping_sub(offset)
                    };
                    let access_addr = if pre_index { offset_addr } else { base };
                    let val = self.load(bus, access_addr, AccessWidth::Word)?;
                    // Commit writeback before branching so a load-to-PC
                    // (function return) leaves Rn=SP correct.
                    if writeback {
                        self.write_reg(rn, offset_addr);
                    }
                    if rt == 15 {
                        // LDR PC, [...] — interworking branch (function return).
                        self.branch_to(val, bus)?;
                        pc_increment = 0;
                    } else {
                        self.write_reg(rt, val);
                        pc_increment = 4;
                    }
                }
                Instruction::StrImm32Idx {
                    rt,
                    rn,
                    imm8,
                    pre_index,
                    add,
                    writeback,
                } => {
                    let base = self.read_reg(rn);
                    let offset = imm8 as u32;
                    let offset_addr = if add {
                        base.wrapping_add(offset)
                    } else {
                        base.wrapping_sub(offset)
                    };
                    let access_addr = if pre_index { offset_addr } else { base };
                    let val = self.read_reg(rt);
                    self.store(bus, access_addr, AccessWidth::Word, val)?;
                    if writeback {
                        self.write_reg(rn, offset_addr);
                    }
                    pc_increment = 4;
                }
                Instruction::Ldrd {
                    rt,
                    rt2,
                    rn,
                    imm8,
                    add_imm,
                    index,
                    writeback,
                } => {
                    // ARMv8-M LDRD (immediate): offset_addr = Rn ± imm32;
                    // access_addr = index ? offset_addr : Rn; if writeback,
                    // Rn = offset_addr.
                    let base = self.read_reg(rn);
                    let offset_addr = if add_imm {
                        base.wrapping_add(imm8 << 2)
                    } else {
                        base.wrapping_sub(imm8 << 2)
                    };
                    let addr = if index { offset_addr } else { base };
                    let v1 = self.load(bus, addr, AccessWidth::Word)?;
                    self.write_reg(rt, v1);
                    let v2 = self.load(bus, addr.wrapping_add(4), AccessWidth::Word)?;
                    self.write_reg(rt2, v2);
                    if writeback {
                        self.write_reg(rn, offset_addr);
                    }
                    pc_increment = 4;
                }
                Instruction::Strd {
                    rt,
                    rt2,
                    rn,
                    imm8,
                    add_imm,
                    index,
                    writeback,
                } => {
                    let base = self.read_reg(rn);
                    let offset_addr = if add_imm {
                        base.wrapping_add(imm8 << 2)
                    } else {
                        base.wrapping_sub(imm8 << 2)
                    };
                    let addr = if index { offset_addr } else { base };
                    let v1 = self.read_reg(rt);
                    let v2 = self.read_reg(rt2);
                    self.store(bus, addr, AccessWidth::Word, v1)?;
                    self.store(bus, addr.wrapping_add(4), AccessWidth::Word, v2)?;
                    if writeback {
                        self.write_reg(rn, offset_addr);
                    }
                    pc_increment = 4;
                }
                Instruction::Tbb { rn, rm } => {
                    let mut base = self.read_reg(rn);
                    if rn == 15 {
                        // ARMv7-M: the table base is the in-execution PC
                        // (insn address + 4) — NOT word-aligned. The old
                        // Align(PC,4) read the table 2 bytes early whenever
                        // the TBB sat at a 2-mod-4 address; ST's
                        // HAL_DMA_RegisterCallback dispatched every
                        // callback ID into the same slot because of it.
                        base = self.pc.wrapping_add(4);
                    }
                    let index = self.read_reg(rm);
                    let addr = base.wrapping_add(index);
                    let byte = self.load(bus, addr, AccessWidth::Byte)?;
                    let offset = byte << 1;
                    self.pc = self.pc.wrapping_add(4).wrapping_add(offset);
                    pc_increment = 0;
                }
                Instruction::Tbh { rn, rm } => {
                    let mut base = self.read_reg(rn);
                    if rn == 15 {
                        // Same unaligned PC+4 base rule as TBB above.
                        base = self.pc.wrapping_add(4);
                    }
                    let index = self.read_reg(rm);
                    let addr = base.wrapping_add(index << 1);
                    let halfword = self.load(bus, addr, AccessWidth::Half)?;
                    let offset = halfword << 1;
                    self.pc = self.pc.wrapping_add(4).wrapping_add(offset);
                    pc_increment = 0;
                }
                Instruction::Unknown32(h1, h2) => {
                    // Manual fallback for complex bit patterns not yet in Instruction enum.
                    //
                    // LDREX Rt, [Rn, #imm8*4]: T1 = 0xE85F_TFTT where
                    //   h1 = 0xE85_ | Rn, h2 = (Rt << 12) | 0xF00 | imm8
                    // STREX Rd, Rt, [Rn, #imm8*4]: T1 = 0xE840_TDII where
                    //   h1 = 0xE84_ | Rn, h2 = (Rt << 12) | (Rd << 8) | imm8
                    //
                    // Single-threaded sim has no preemption between LDREX and
                    // STREX, so we model the exclusive monitor as always
                    // succeeding. This matches the observable behavior of
                    // atomic ops on real hardware in the uncontended case.
                    if (h1 & 0xFFF0) == 0xE840 && (h2 & 0xF03F) == 0xF000 {
                        // TT / TTT / TTA / TTAT — ARMv8-M "Test Target"
                        // (ARMv8-M ARM, DDI 0553, "TT, TTT, TTA, TTAT").
                        // Encoding T1:
                        //   h1 = 0xE84_ | Rn
                        //   h2 = 0xF000 | (Rd << 8) | (A << 7) | (T << 6)
                        //
                        // This SHARES the 0xE84x prefix with STREX, and the
                        // STREX arm below only matched on h1. TT sets the
                        // STREX Rt field to 0b1111 (a PC destination, which is
                        // UNPREDICTABLE for a real STREX), so every TT in the
                        // corpus was being executed as
                        // `STREX PC, Rd, [Rn, #0]` — a *store of the PC to the
                        // address being probed*. TT never accesses memory on
                        // real silicon; it only queries the MPU/SAU attributes
                        // of an address. That phantom store is a silent
                        // memory-corruption bug wherever the probed address is
                        // mapped RAM, and it is what crashed the STM32WBA52
                        // Zephyr lab at 0x2002_0000: Zephyr's
                        // `arm_cmse_mpu_region_get` probes one-past-the-end of
                        // a buffer, which TT is explicitly allowed to do
                        // because it does not dereference it.
                        //
                        // Response value: this core models neither an MPU nor
                        // the Security Extension (SAU/IDAU), and MPU_CTRL
                        // writes land in an inert SCB latch that enforces
                        // nothing — so the machine genuinely has no region
                        // information to report. The architecturally defined
                        // way to say that is MRVALID = 0 (and, with no
                        // Security Extension, SREGION/SRVALID/S/IREGION/
                        // IRVALID are RES0). A zero response is therefore the
                        // honest answer, and it is the conservative one: a
                        // caller reads it as "not known to be accessible"
                        // rather than being handed a fabricated region number
                        // or an unconditional permit. Zephyr's
                        // `arm_cmse_mpu_region_get` maps it to -EINVAL, which
                        // is exactly what it reports on hardware with the MPU
                        // off.
                        //
                        // A and T select which security state / privilege
                        // level is queried; without the Security Extension
                        // TTA/TTAT are UNDEFINED, but answering them the same
                        // conservative way is still strictly better than
                        // falling through to a store.
                        let rd = ((h2 >> 8) & 0xF) as u8;
                        // Rd = SP or PC is UNPREDICTABLE for TT; do not let a
                        // malformed encoding rewrite the stack or program
                        // counter. The instruction is still consumed.
                        if rd != 13 && rd != 15 {
                            self.set_register(rd, 0);
                        }
                        pc_increment = 4;
                    } else if (h1 & 0xFFF0) == 0xFA90 && (h2 & 0xF0F0) == 0xF010 {
                        // ARMv7E-M QADD16: independently signed-saturating add
                        // the two halfword lanes (Cortex-M4 DSP extension).
                        let rn = (h1 & 0xF) as u8;
                        let rd = ((h2 >> 8) & 0xF) as u8;
                        let rm = (h2 & 0xF) as u8;
                        let a = self.get_register(rn);
                        let b = self.get_register(rm);
                        let mut saturated = false;
                        let lane = |shift: u32, saturated: &mut bool| -> u32 {
                            let lhs = ((a >> shift) as u16) as i16 as i32;
                            let rhs = ((b >> shift) as u16) as i16 as i32;
                            let sum = lhs + rhs;
                            let clamped = sum.clamp(i16::MIN as i32, i16::MAX as i32);
                            *saturated |= clamped != sum;
                            (clamped as i16 as u16 as u32) << shift
                        };
                        let value = lane(0, &mut saturated) | lane(16, &mut saturated);
                        self.set_register(rd, value);
                        if saturated {
                            self.xpsr |= 1 << 27;
                        }
                        pc_increment = 4;
                    } else if (h1 & 0xFFF0) == 0xF380 && (h2 & 0x00C0) == 0 {
                        // ARMv7-M USAT without an optional shift.
                        let rn = (h1 & 0xF) as u8;
                        let rd = ((h2 >> 8) & 0xF) as u8;
                        let sat = (h2 & 0x1F) as u32;
                        let source = self.get_register(rn) as i32;
                        let max = if sat == 32 {
                            u32::MAX
                        } else {
                            (1u32 << sat) - 1
                        };
                        let value = if source < 0 {
                            0
                        } else {
                            (source as u32).min(max)
                        };
                        if source < 0 || source as u32 > max {
                            self.xpsr |= 1 << 27;
                        }
                        self.set_register(rd, value);
                        pc_increment = 4;
                    } else if (h1 & 0xFFF0) == 0xE850 {
                        // LDREX
                        let rn = (h1 & 0xF) as u8;
                        let rt = ((h2 >> 12) & 0xF) as u8;
                        let imm8 = (h2 & 0xFF) as u32;
                        let addr = self.get_register(rn).wrapping_add(imm8 * 4);
                        let val = self.load(bus, addr, AccessWidth::Word)?;
                        self.set_register(rt, val);
                        pc_increment = 4;
                    } else if (h1 & 0xFFF0) == 0xE840 {
                        // STREX
                        let rn = (h1 & 0xF) as u8;
                        let rt = ((h2 >> 12) & 0xF) as u8;
                        let rd = ((h2 >> 8) & 0xF) as u8;
                        let imm8 = (h2 & 0xFF) as u32;
                        let addr = self.get_register(rn).wrapping_add(imm8 * 4);
                        let val = self.get_register(rt);
                        self.store(bus, addr, AccessWidth::Word, val)?;
                        // Rd = 0 → success.
                        self.set_register(rd, 0);
                        pc_increment = 4;
                    } else if (h1 & 0xFFF0) == 0xE8D0 && (h2 & 0x0FFF) == 0x0F4F {
                        // ARMv7-M LDREXB Rt, [Rn]. Rust uses this for byte
                        // atomics such as AtomicBool::compare_exchange.
                        let rn = (h1 & 0xF) as u8;
                        let rt = ((h2 >> 12) & 0xF) as u8;
                        let address = self.get_register(rn);
                        let value = self.load(bus, address, AccessWidth::Byte)? as u8;
                        self.set_register(rt, u32::from(value));
                        self.exclusive_byte = Some((address, value));
                        pc_increment = 4;
                    } else if (h1 & 0xFFF0) == 0xE8C0 && (h2 & 0x0FF0) == 0x0F40 {
                        // ARMv7-M STREXB Rd, Rt, [Rn]. The single-threaded
                        // machine has no contender, so the monitor succeeds.
                        let rn = (h1 & 0xF) as u8;
                        let rt = ((h2 >> 12) & 0xF) as u8;
                        let rd = (h2 & 0xF) as u8;
                        let address = self.get_register(rn);
                        let reservation_matches = match self.exclusive_byte.take() {
                            Some((reserved, value)) if reserved == address => {
                                self.load(bus, address, AccessWidth::Byte)? as u8 == value
                            }
                            _ => false,
                        };
                        let succeeds = if reservation_matches {
                            self.store(bus, address, AccessWidth::Byte, self.get_register(rt))?;
                            true
                        } else {
                            false
                        };
                        self.set_register(rd, if succeeds { 0 } else { 1 });
                        pc_increment = 4;
                    } else if (h1 & 0xFFF0) == 0xE8D0 && (h2 & 0x0F0F) == 0x0F0F {
                        // Load-acquire family (ARMv8-M mainline, also on the
                        // M33 with TrustZone off): LDAB/LDAH/LDA and
                        // LDAEXB/LDAEXH/LDAEX.
                        //   h1 = 0xE8D0 | Rn, h2 = Rt<<12 | 0xF<<8 | sz<<4 | 0xF
                        //   sz: 8=B, 9=H, A=word (acquire), C=EXB, D=EXH, E=EX
                        // Acquire ordering and the exclusive monitor are
                        // no-ops in this single-threaded sim (same rationale
                        // as LDREX above). Rust atomics on thumbv8m compile
                        // to these — embassy's executor run-queue lives on
                        // LDAEX/STLEX.
                        let rn = (h1 & 0xF) as u8;
                        let rt = ((h2 >> 12) & 0xF) as u8;
                        let addr = self.get_register(rn);
                        let width = match (h2 >> 4) & 0xF {
                            0x8 | 0xC => Some(AccessWidth::Byte),
                            0x9 | 0xD => Some(AccessWidth::Half),
                            0xA | 0xE => Some(AccessWidth::Word),
                            _ => None,
                        };
                        if let Some(width) = width {
                            let val = self.load(bus, addr, width)?;
                            self.set_register(rt, val);
                        }
                        pc_increment = 4;
                    } else if (h1 & 0xFFF0) == 0xE8C0 && (h2 & 0x0F00) == 0x0F00 {
                        // Store-release family: STLB/STLH/STL ([3:0]=0xF, no
                        // status register) and STLEXB/STLEXH/STLEX ([3:0]=Rd,
                        // always-success monitor → Rd = 0).
                        //   h1 = 0xE8C0 | Rn, h2 = Rt<<12 | 0xF<<8 | sz<<4 | Rd/0xF
                        let rn = (h1 & 0xF) as u8;
                        let rt = ((h2 >> 12) & 0xF) as u8;
                        let addr = self.get_register(rn);
                        let val = self.get_register(rt);
                        let sz = (h2 >> 4) & 0xF;
                        let width = match sz {
                            0x8 | 0xC => Some(AccessWidth::Byte),
                            0x9 | 0xD => Some(AccessWidth::Half),
                            0xA | 0xE => Some(AccessWidth::Word),
                            _ => None,
                        };
                        if let Some(width) = width {
                            self.store(bus, addr, width, val)?;
                        }
                        if matches!(sz, 0xC..=0xE) {
                            let rd = (h2 & 0xF) as u8;
                            self.set_register(rd, 0); // success
                        }
                        pc_increment = 4;
                    } else if (h1 & 0xFE00) == 0xE800 {
                        // Table branch, load/store multiple etc — not yet
                        // modeled in full; advance past the 32-bit insn.
                        pc_increment = 4;
                    } else if (h1 & 0xFE00) == 0xF800 {
                        // LDR/STR (immediate) T3/T4
                        let op1 = (h1 >> 4) & 0xF;
                        let rn = (h1 & 0xF) as u8;
                        let rt = ((h2 >> 12) & 0xF) as u8;
                        let is_t4 = (op1 & 0x8) == 0;
                        // Signed (LDRSB.W/LDRSH.W) vs unsigned (LDRB.W/LDRH.W)
                        // is selected by h1 bit 8 (0x0100), NOT op1 bit 3:
                        // op1 = h1[7:4] excludes bit 8, and op1 bit 3 (=h1 bit 7)
                        // is the imm12-form selector used for is_t4 above. Using
                        // it for the sign made LDRB.W T2 (0xF89x) sign-extend any
                        // byte >= 0x80 (0x85 -> 0xFFFFFF85).
                        let is_signed = (h1 & 0x0100) != 0;
                        // When Rn=PC (rn==15), T4 form is always the PC-literal encoding
                        // (LDR.W Rt, [PC, ±imm12]), never register-offset.
                        let is_reg_offset = is_t4 && rn != 15 && (h2 & 0x0800) == 0;
                        if !is_reg_offset {
                            let mut supported = true;
                            let addr: u32;
                            let mut wb = false;
                            let mut wb_val = 0u32;
                            if !is_t4 {
                                // T3: Rn-relative with unsigned imm12
                                let base = if rn == 15 {
                                    // PC-literal T3: Align(PC+4, 4) + imm12
                                    (self.pc.wrapping_add(4)) & !3
                                } else {
                                    self.read_reg(rn)
                                };
                                let offset = (h2 & 0xFFF) as u32;
                                addr = base.wrapping_add(offset);
                            } else if rn == 15 {
                                // PC-literal T2: LDR.W Rt, [PC, ±imm12]
                                // U bit is bit 7 of h1; imm12 is h2[11:0]
                                let imm12 = (h2 & 0xFFF) as u32;
                                let u = (h1 >> 7) & 1;
                                let base = (self.pc.wrapping_add(4)) & !3;
                                addr = if u != 0 {
                                    base.wrapping_add(imm12)
                                } else {
                                    base.wrapping_sub(imm12)
                                };
                            } else {
                                let p = (h2 >> 10) & 1;
                                let u = (h2 >> 9) & 1;
                                let w = (h2 >> 8) & 1;
                                let imm8 = (h2 & 0xFF) as i32;
                                let offset = if u != 0 { imm8 } else { -imm8 };
                                let base = self.read_reg(rn);
                                if p != 0 {
                                    addr = base.wrapping_add(offset as u32);
                                    if w != 0 {
                                        wb = true;
                                        wb_val = addr;
                                    }
                                } else {
                                    addr = base;
                                    wb = true;
                                    wb_val = base.wrapping_add(offset as u32);
                                }
                            }
                            let mut branch_taken = false;
                            match op1 & 0x7 {
                                0 => {
                                    let val = self.read_reg(rt) & 0xFF;
                                    self.store(bus, addr, AccessWidth::Byte, val)?;
                                }
                                // Rt==15 = PLD/PLI preload hint — NOP (handled by `_`).
                                1 if rt != 15 => {
                                    let v = self.load(bus, addr, AccessWidth::Byte)?;
                                    let out = if is_signed {
                                        (v as u8 as i8) as i32 as u32
                                    } else {
                                        v
                                    };
                                    self.write_reg(rt, out);
                                }
                                2 => {
                                    let val = self.read_reg(rt) & 0xFFFF;
                                    self.store(bus, addr, AccessWidth::Half, val)?;
                                }
                                // Rt==15 = PLDW preload hint — NOP (handled by `_`).
                                3 if rt != 15 => {
                                    let v = self.load(bus, addr, AccessWidth::Half)?;
                                    let out = if is_signed {
                                        (v as u16 as i16) as i32 as u32
                                    } else {
                                        v
                                    };
                                    self.write_reg(rt, out);
                                }
                                4 => {
                                    let val = self.read_reg(rt);
                                    self.store(bus, addr, AccessWidth::Word, val)?;
                                }
                                5 => {
                                    let v = self.load(bus, addr, AccessWidth::Word)?;
                                    if rt == 15 {
                                        if wb {
                                            self.write_reg(rn, wb_val);
                                            wb = false;
                                        }
                                        self.branch_to(v, bus)?;
                                        branch_taken = true;
                                    } else {
                                        self.write_reg(rt, v);
                                    }
                                }
                                _ => {
                                    supported = false;
                                }
                            }
                            if supported {
                                if wb {
                                    self.write_reg(rn, wb_val);
                                }
                                // Rt==15 load is a branch; suppress pc_increment
                                // (see the register-offset path below).
                                if branch_taken {
                                    pc_increment = 0;
                                } else {
                                    pc_increment = 4;
                                }
                            }
                        } else {
                            // Register offset (T2)
                            let rn = (h1 & 0xF) as u8;
                            let rt = ((h2 >> 12) & 0xF) as u8;
                            let rm = (h2 & 0xF) as u8;
                            let imm2 = ((h2 >> 4) & 0x3) as u8;
                            let base = self.read_reg(rn);
                            let offset = self.read_reg(rm).wrapping_shl(imm2 as u32);
                            let addr = base.wrapping_add(offset);
                            let mut branch_taken = false;
                            match op1 & 0x7 {
                                0 => {
                                    let val = self.read_reg(rt) & 0xFF;
                                    self.store(bus, addr, AccessWidth::Byte, val)?;
                                }
                                // Rt==15 = PLD/PLI preload hint — NOP (handled by `_`).
                                1 if rt != 15 => {
                                    let v = self.load(bus, addr, AccessWidth::Byte)?;
                                    let out = if is_signed {
                                        (v as u8 as i8) as i32 as u32
                                    } else {
                                        v
                                    };
                                    self.write_reg(rt, out);
                                }
                                2 => {
                                    let val = self.read_reg(rt) & 0xFFFF;
                                    self.store(bus, addr, AccessWidth::Half, val)?;
                                }
                                // Rt==15 = PLDW preload hint — NOP (handled by `_`).
                                3 if rt != 15 => {
                                    let v = self.load(bus, addr, AccessWidth::Half)?;
                                    let out = if is_signed {
                                        (v as u16 as i16) as i32 as u32
                                    } else {
                                        v
                                    };
                                    self.write_reg(rt, out);
                                }
                                4 => {
                                    let val = self.read_reg(rt);
                                    self.store(bus, addr, AccessWidth::Word, val)?;
                                }
                                5 => {
                                    let v = self.load(bus, addr, AccessWidth::Word)?;
                                    if rt == 15 {
                                        self.branch_to(v, bus)?;
                                        branch_taken = true;
                                    } else {
                                        self.write_reg(rt, v);
                                    }
                                }
                                _ => {}
                            }
                            // A load into PC (Rt==15) is a branch: branch_to
                            // already set PC, so the 32-bit pc_increment must be
                            // suppressed (same contract as Bx). Leaving it at 4
                            // landed PC one halfword past the target — this broke
                            // GCC switch jump tables (`ldr.w pc,[rn,rm,lsl#n]`).
                            if branch_taken {
                                pc_increment = 0;
                            } else {
                                pc_increment = 4;
                            }
                        }
                    } else if (h1 & 0xFB00) == 0xF000 && (h2 & 0x8000) == 0 {
                        // Data-processing (modified immediate) - repeated here for safety but usually handled by DataProcImm32
                        let i = (h1 >> 10) & 0x1;
                        let op = ((h1 >> 5) & 0xF) as u8;
                        let s = ((h1 >> 4) & 0x1) != 0;
                        let rn = (h1 & 0xF) as u8;
                        let imm3 = (h2 >> 12) & 0x7;
                        let rd = ((h2 >> 8) & 0xF) as u8;
                        let imm8 = h2 & 0xFF;
                        let imm12 = (i << 11) | (imm3 << 8) | imm8;
                        let imm32 = thumb_expand_imm(imm12 as u32);
                        let op1 = self.read_reg(rn);
                        let mut result = 0u32;
                        let mut update_rd = true;
                        match op {
                            0x0 => result = op1 & imm32,                                  // AND
                            0x1 => result = op1 & !imm32,                                 // BIC
                            0x2 => result = if rn == 15 { imm32 } else { op1 | imm32 },   // ORR/MOV
                            0x3 => result = if rn == 15 { !imm32 } else { op1 | !imm32 }, // ORN/MVN
                            0x4 => result = op1 ^ imm32,                                  // EOR
                            0x8 => result = op1.wrapping_add(imm32),                      // ADD
                            0xD => result = op1.wrapping_sub(imm32),                      // SUB
                            _ => update_rd = false,
                        }
                        if update_rd {
                            if rd != 15 {
                                self.write_reg(rd, result);
                            }
                            if s {
                                self.update_nz(result);
                            }
                            pc_increment = 4;
                        }
                    } else if (h1 & 0xFB00) == 0xF100 && (h2 & 0x8000) == 0 {
                        // Data-processing (plain binary immediate)
                        let i = (h1 >> 10) & 0x1;
                        let op = ((h1 >> 5) & 0xF) as u8;
                        let rn = (h1 & 0xF) as u8;
                        let imm3 = (h2 >> 12) & 0x7;
                        let rd = ((h2 >> 8) & 0xF) as u8;
                        let imm8 = h2 & 0xFF;
                        let imm12 = (i << 11) | (imm3 << 8) | imm8;
                        let op1 = self.read_reg(rn);
                        match op {
                            0x0 => {
                                self.write_reg(rd, op1.wrapping_add(imm12 as u32));
                                pc_increment = 4;
                            } // ADD
                            0xA => {
                                self.write_reg(rd, op1.wrapping_sub(imm12 as u32));
                                pc_increment = 4;
                            } // SUB
                            _ => {}
                        }
                    } else if (h1 & 0xF000) == 0xF000 && (h2 & 0x8000) == 0x8000 {
                        // B.W / BL (handled elsewhere but just in case)
                        pc_increment = 4;
                    } else {
                        tracing::warn!(
                            "Unknown 32-bit instruction at {:#x}: {:#x} {:#x}",
                            self.pc,
                            h1,
                            h2
                        );
                        crate::fidelity::record_undecoded(
                            self.pc,
                            ((h1 as u64) << 16) | (h2 as u64),
                            "undecoded T32",
                        );
                        // As for T16 above: fault rather than skip.
                        self.pending_undef_instruction = true;
                        return Err(SimulationError::DecodeError(self.pc as u64));
                    }
                }

                Instruction::Nop => { /* Do nothing */ }
                Instruction::Wfi => {
                    // ARMv7-M WFI: complete as a NOP if a wake-up event is
                    // already pending, otherwise suspend the core until one
                    // arrives. Wake-up ignores PRIMASK (see `wfi_wake_pending`);
                    // `Machine::run` fast-forwards the idle window while
                    // `self.sleeping` holds. PC has already advanced past the
                    // WFI like any other 16-bit hint.
                    if !self.wfi_wake_pending() {
                        self.sleeping = true;
                    }
                }
                Instruction::MovImm { rd, imm } => {
                    self.write_reg(rd, imm as u32);
                    if !it_block_instruction {
                        self.update_nz(imm as u32);
                    }
                }
                // Control Flow
                Instruction::Cbz { rn, imm } => {
                    if self.read_reg(rn) == 0 {
                        self.pc = self.pc.wrapping_add(4).wrapping_add(imm as u32);
                        pc_increment = 0;
                    }
                }
                Instruction::Cbnz { rn, imm } => {
                    if self.read_reg(rn) != 0 {
                        self.pc = self.pc.wrapping_add(4).wrapping_add(imm as u32);
                        pc_increment = 0;
                    }
                }
                Instruction::Branch { offset } => {
                    let target = (self.pc as i32).wrapping_add(4).wrapping_add(offset) as u32;
                    self.pc = target;
                    pc_increment = 0;
                }
                // Arithmetic
                Instruction::AddReg { rd, rn, rm } => {
                    let op1 = self.read_reg(rn);
                    let op2 = self.read_reg(rm);
                    let (res, c, v) = add_with_flags(op1, op2);
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::AddImm3 { rd, rn, imm } => {
                    let op1 = self.read_reg(rn);
                    let (res, c, v) = add_with_flags(op1, imm as u32);
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::AddImm8 { rd, imm } => {
                    let op1 = self.read_reg(rd);
                    let (res, c, v) = add_with_flags(op1, imm as u32);
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::SubReg { rd, rn, rm } => {
                    let op1 = self.read_reg(rn);
                    let op2 = self.read_reg(rm);
                    let (res, c, v) = sub_with_flags(op1, op2);
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::SubImm3 { rd, rn, imm } => {
                    let op1 = self.read_reg(rn);
                    let (res, c, v) = sub_with_flags(op1, imm as u32);
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::SubImm8 { rd, imm } => {
                    let op1 = self.read_reg(rd);
                    let (res, c, v) = sub_with_flags(op1, imm as u32);
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::AddSp { imm } => {
                    let sp = self.read_reg(13).wrapping_add(imm as u32);
                    self.write_reg(13, sp);
                }
                Instruction::SubSp { imm } => {
                    let sp = self.read_reg(13).wrapping_sub(imm as u32);
                    self.write_reg(13, sp);
                }

                Instruction::Uxtb { rd, rm } => {
                    let val = self.read_reg(rm);
                    self.write_reg(rd, val & 0xFF);
                }
                Instruction::Uxth { rd, rm } => {
                    let val = self.read_reg(rm);
                    self.write_reg(rd, val & 0xFFFF);
                }
                Instruction::Sxtb { rd, rm } => {
                    let val = self.read_reg(rm) as u8 as i8 as i32 as u32;
                    self.write_reg(rd, val);
                }
                Instruction::Sxth { rd, rm } => {
                    let val = self.read_reg(rm) as u16 as i16 as i32 as u32;
                    self.write_reg(rd, val);
                }
                Instruction::ExtendW {
                    rd,
                    rn,
                    rm,
                    rotate,
                    op,
                } => {
                    // ROR Rm by `rotate` (0/8/16/24), then extract+extend.
                    let v = self.read_reg(rm).rotate_right(rotate as u32);
                    let ext = match op {
                        0b000 => v as u16 as i16 as i32 as u32, // S*XTH
                        0b001 => v & 0xFFFF,                    // U*XTH
                        0b100 => v as u8 as i8 as i32 as u32,   // S*XTB
                        _ => v & 0xFF,                          // U*XTB (0b101)
                    };
                    // Extend-and-add variants (Rn != 0xF) add Rn; the plain
                    // extends encode Rn = 0xF.
                    let out = if rn == 0xF {
                        ext
                    } else {
                        self.read_reg(rn).wrapping_add(ext)
                    };
                    self.write_reg(rd, out);
                }

                Instruction::It { cond, mask } => {
                    self.it_state = (cond << 4) | mask;
                    it_block_instruction = false; // The IT instruction itself doesn't count towards the block's instructions
                }
                Instruction::AddRegHigh { rd, rm } => {
                    let val1 = self.read_reg(rd);
                    let val2 = self.read_reg(rm);
                    self.write_reg(rd, val1.wrapping_add(val2));
                }
                Instruction::CmpImm { rn, imm } => {
                    let op1 = self.read_reg(rn);
                    let (res, c, v) = sub_with_flags(op1, imm as u32);
                    self.update_nzcv(res, c, v);
                }
                Instruction::CmpReg { rn, rm } => {
                    let op1 = self.read_reg(rn);
                    let op2 = self.read_reg(rm);
                    let (res, c, v) = sub_with_flags(op1, op2);
                    self.update_nzcv(res, c, v);
                }
                Instruction::MovReg { rd, rm } => {
                    let val = self.read_reg(rm);
                    if rd == 15 {
                        // ARMv7-M: writing PC through MOV is BXWritePC, i.e. a
                        // BRANCH — bit 0 selects the instruction set and is not
                        // part of the address, and no sequential PC advance
                        // follows. Falling through to `write_reg` alone left the
                        // Thumb bit in PC and then added `pc_increment` on top,
                        // landing 2 bytes past the target.
                        //
                        // rustc emits exactly this for a dense `match` over
                        // bytes: `ADR Rn, table` / `ADD Rd, Rn, idx LSL #2` /
                        // `MOV PC, Rd` into a table of 4-byte `B.W` entries.
                        // Two bytes late is the middle of an entry, so the
                        // second halfword of that branch executed as a stray
                        // 16-bit instruction and control fell through to the
                        // NEXT entry — every arm ran its successor's code. The
                        // nRF52840 OBD-II scanner painted its OLED that way:
                        // "RPM 3000" came out "S N 3000" (R->S, P->Q which is
                        // absent so blank, M->N), while digits, dispatched by
                        // arithmetic rather than by this table, stayed exact.
                        self.pc = val & !1;
                        pc_increment = 0;
                    } else {
                        self.write_reg(rd, val);
                    }
                }
                // Logic
                Instruction::And { rd, rm } => {
                    let res = self.read_reg(rd) & self.read_reg(rm);
                    self.write_reg(rd, res);
                    // T1 encoding: setflags = !InITBlock(). Leaking flags
                    // from inside an IT block corrupts the CONDITION of every
                    // instruction still to run in that block — measured: an
                    // `orrls` before a `strls` in the same `itt ls` cleared Z
                    // and the store never happened.
                    if !it_block_instruction {
                        self.update_nz(res);
                    }
                }
                Instruction::Bic { rd, rm } => {
                    let res = self.read_reg(rd) & !self.read_reg(rm);
                    self.write_reg(rd, res);
                    // T1 encoding: setflags = !InITBlock(). Leaking flags
                    // from inside an IT block corrupts the CONDITION of every
                    // instruction still to run in that block — measured: an
                    // `orrls` before a `strls` in the same `itt ls` cleared Z
                    // and the store never happened.
                    if !it_block_instruction {
                        self.update_nz(res);
                    }
                }
                Instruction::Orr { rd, rm } => {
                    let res = self.read_reg(rd) | self.read_reg(rm);
                    self.write_reg(rd, res);
                    // T1 encoding: setflags = !InITBlock(). Leaking flags
                    // from inside an IT block corrupts the CONDITION of every
                    // instruction still to run in that block — measured: an
                    // `orrls` before a `strls` in the same `itt ls` cleared Z
                    // and the store never happened.
                    if !it_block_instruction {
                        self.update_nz(res);
                    }
                }
                Instruction::Eor { rd, rm } => {
                    let res = self.read_reg(rd) ^ self.read_reg(rm);
                    self.write_reg(rd, res);
                    // T1 encoding: setflags = !InITBlock(). Leaking flags
                    // from inside an IT block corrupts the CONDITION of every
                    // instruction still to run in that block — measured: an
                    // `orrls` before a `strls` in the same `itt ls` cleared Z
                    // and the store never happened.
                    if !it_block_instruction {
                        self.update_nz(res);
                    }
                }
                Instruction::Mvn { rd, rm } => {
                    let res = !self.read_reg(rm);
                    self.write_reg(rd, res);
                    // T1 encoding: setflags = !InITBlock(). Leaking flags
                    // from inside an IT block corrupts the CONDITION of every
                    // instruction still to run in that block — measured: an
                    // `orrls` before a `strls` in the same `itt ls` cleared Z
                    // and the store never happened.
                    if !it_block_instruction {
                        self.update_nz(res);
                    }
                }
                Instruction::Mul { rd, rn } => {
                    let op1 = self.read_reg(rd);
                    let op2 = self.read_reg(rn);
                    let res = op1.wrapping_mul(op2);
                    self.write_reg(rd, res);
                    // T1 encoding: setflags = !InITBlock(). Leaking flags
                    // from inside an IT block corrupts the CONDITION of every
                    // instruction still to run in that block — measured: an
                    // `orrls` before a `strls` in the same `itt ls` cleared Z
                    // and the store never happened.
                    if !it_block_instruction {
                        self.update_nz(res);
                    }
                }
                Instruction::Mul32 { rd, rn, rm } => {
                    let op1 = self.read_reg(rn);
                    let op2 = self.read_reg(rm);
                    let res = op1.wrapping_mul(op2);
                    self.write_reg(rd, res);
                    pc_increment = 4;
                }

                Instruction::Cpsie { primask, faultmask } => {
                    if primask {
                        self.primask = false;
                    }
                    if faultmask {
                        self.faultmask = false;
                    }
                }
                Instruction::Cpsid { primask, faultmask } => {
                    if primask {
                        self.primask = true;
                    }
                    if faultmask {
                        self.faultmask = true;
                    }
                }

                // Shifts
                Instruction::Lsl { rd, rm, imm } => {
                    let val = self.read_reg(rm);
                    let res = val.wrapping_shl(imm as u32);
                    self.write_reg(rd, res);
                    // T1 shift-immediate: setflags = !InITBlock(). Inside an
                    // IT block this encoding is the flag-preserving LSL, and
                    // leaking flags here would corrupt the remaining block
                    // conditions (Tier-1 H563/WBA52 gpio-check regression).
                    if !it_block_instruction {
                        // LSL #n (n>0) sets C to the last bit shifted out:
                        // Rm[32-n]. LSL #0 is a move and leaves C unchanged.
                        // (Verified against STM32F103 silicon via thumb_oracles.)
                        if imm == 0 {
                            self.update_nz(res);
                        } else {
                            let carry = (val >> (32 - imm as u32)) & 1 == 1;
                            self.update_nzcv(res, carry, self.get_overflow());
                        }
                    }
                }
                Instruction::Lsr { rd, rm, imm } => {
                    let val = self.read_reg(rm);
                    // Thumb T1: imm5 == 0 encodes a shift of 32.
                    let n = if imm == 0 { 32 } else { imm as u32 };
                    let res = if n >= 32 { 0 } else { val.wrapping_shr(n) };
                    self.write_reg(rd, res);
                    // T1 shift-immediate: setflags = !InITBlock(). LSR #n sets C
                    // to Rm[n-1], the last bit shifted out (silicon-verified).
                    if !it_block_instruction {
                        let carry = (val >> (n - 1)) & 1 == 1;
                        self.update_nzcv(res, carry, self.get_overflow());
                    }
                }
                Instruction::Asr { rd, rm, imm } => {
                    let val = self.read_reg(rm);
                    // Thumb T1: imm5 == 0 encodes a shift of 32.
                    let n = if imm == 0 { 32 } else { imm as u32 };
                    let res = ((val as i32) >> n.min(31)) as u32;
                    self.write_reg(rd, res);
                    // T1 shift-immediate: setflags = !InITBlock(). ASR #n sets C
                    // to Rm[n-1], the last bit shifted out (silicon-verified).
                    if !it_block_instruction {
                        let carry = (val >> (n - 1)) & 1 == 1;
                        self.update_nzcv(res, carry, self.get_overflow());
                    }
                }
                Instruction::LslReg { rd, rm } => {
                    // Register-controlled shift: amount = Rm[7:0]. Carry = last
                    // bit shifted out; n==0 leaves C unchanged (ARMv7-M Shift_C).
                    // Silicon-verified on STM32F103 via thumb_oracles.
                    let val = self.read_reg(rd);
                    let shift = self.read_reg(rm) & 0xFF;
                    let (res, carry) = if shift == 0 {
                        (val, self.get_carry())
                    } else if shift < 32 {
                        (val << shift, (val >> (32 - shift)) & 1 == 1)
                    } else if shift == 32 {
                        (0, val & 1 == 1)
                    } else {
                        (0, false)
                    };
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, carry, self.get_overflow());
                    }
                }
                Instruction::LsrReg { rd, rm } => {
                    let val = self.read_reg(rd);
                    let shift = self.read_reg(rm) & 0xFF;
                    let (res, carry) = if shift == 0 {
                        (val, self.get_carry())
                    } else if shift < 32 {
                        (val >> shift, (val >> (shift - 1)) & 1 == 1)
                    } else if shift == 32 {
                        (0, (val >> 31) & 1 == 1)
                    } else {
                        (0, false)
                    };
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, carry, self.get_overflow());
                    }
                }
                Instruction::AsrReg { rd, rm } => {
                    let val = self.read_reg(rd);
                    let vali = val as i32;
                    let shift = self.read_reg(rm) & 0xFF;
                    let (res, carry) = if shift == 0 {
                        (val, self.get_carry())
                    } else if shift < 32 {
                        ((vali >> shift) as u32, (val >> (shift - 1)) & 1 == 1)
                    } else {
                        // shift >= 32: result is all sign bits; C = Rm[31].
                        ((vali >> 31) as u32, (val >> 31) & 1 == 1)
                    };
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, carry, self.get_overflow());
                    }
                }
                Instruction::Adc { rd, rm } => {
                    let op1 = self.read_reg(rd);
                    let op2 = self.read_reg(rm);
                    let carry_in = (self.xpsr >> 29) & 1;
                    let (res, c, v) = adc_with_flags(op1, op2, carry_in);
                    self.write_reg(rd, res);
                    // T1 encoding: setflags = !InITBlock(). Leaking flags
                    // from inside an IT block corrupts the CONDITION of every
                    // instruction still to run in that block — measured: an
                    // `orrls` before a `strls` in the same `itt ls` cleared Z
                    // and the store never happened.
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::Sbc { rd, rm } => {
                    let op1 = self.read_reg(rd);
                    let op2 = self.read_reg(rm);
                    let carry_in = (self.xpsr >> 29) & 1;
                    let (res, c, v) = sbc_with_flags(op1, op2, carry_in);
                    self.write_reg(rd, res);
                    // T1 encoding: setflags = !InITBlock(). Leaking flags
                    // from inside an IT block corrupts the CONDITION of every
                    // instruction still to run in that block — measured: an
                    // `orrls` before a `strls` in the same `itt ls` cleared Z
                    // and the store never happened.
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }
                Instruction::Ror { rd, rm } => {
                    // Register rotate: amount = Rm[7:0]. Carry = the rotated
                    // result's MSB; n==0 leaves C unchanged (ARMv7-M ROR_C).
                    // Silicon-verified on STM32F103 via thumb_oracles.
                    let val = self.read_reg(rd);
                    let n = self.read_reg(rm) & 0xFF;
                    let (res, carry) = if n == 0 {
                        (val, self.get_carry())
                    } else {
                        let r = val.rotate_right(n % 32);
                        (r, (r >> 31) & 1 == 1)
                    };
                    self.write_reg(rd, res);
                    if !it_block_instruction {
                        self.update_nzcv(res, carry, self.get_overflow());
                    }
                }
                Instruction::Rev { rd, rm } => {
                    let val = self.read_reg(rm);
                    self.write_reg(rd, val.swap_bytes());
                }
                Instruction::Rev16 { rd, rm } => {
                    let val = self.read_reg(rm);
                    let low = ((val & 0xFF) << 8) | ((val >> 8) & 0xFF);
                    let high = ((val & 0x00FF0000) << 8) | ((val & 0xFF000000) >> 8);
                    self.write_reg(rd, high | low);
                }
                Instruction::RevSh { rd, rm } => {
                    let val = self.read_reg(rm);
                    let low = ((val & 0xFF) << 8) | ((val >> 8) & 0xFF);
                    self.write_reg(rd, (low as i16) as u32);
                }
                Instruction::Tst { rn, rm } => {
                    let res = self.read_reg(rn) & self.read_reg(rm);
                    self.update_nz(res);
                }
                Instruction::Cmn { rn, rm } => {
                    let op1 = self.read_reg(rn);
                    let op2 = self.read_reg(rm);
                    let (res, c, v) = add_with_flags(op1, op2);
                    self.update_nzcv(res, c, v);
                }
                Instruction::Rsbs { rd, rn } => {
                    let op1 = self.read_reg(rn);
                    let (res, c, v) = sub_with_flags(0, op1);
                    self.write_reg(rd, res);
                    // T1 encoding: setflags = !InITBlock(). Leaking flags
                    // from inside an IT block corrupts the CONDITION of every
                    // instruction still to run in that block — measured: an
                    // `orrls` before a `strls` in the same `itt ls` cleared Z
                    // and the store never happened.
                    if !it_block_instruction {
                        self.update_nzcv(res, c, v);
                    }
                }

                // Memory Operations (Word)
                Instruction::LdrImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    let val = self.load(bus, addr, AccessWidth::Word)?;
                    self.write_reg(rt, val);
                    if val == 0x021d0000 {
                        tracing::info!("LDR Literal/Imm SUSPICIOUS: R{} loaded with {:#x} from {:#x} (PC={:#x})", rt, val, addr, self.pc);
                    }
                }
                Instruction::StrImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    let val = self.read_reg(rt);
                    self.store(bus, addr, AccessWidth::Word, val)?;
                }
                Instruction::LdrReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    let val = self.load(bus, addr, AccessWidth::Word)?;
                    self.write_reg(rt, val);
                }
                Instruction::StrReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    let val = self.read_reg(rt);
                    self.store(bus, addr, AccessWidth::Word, val)?;
                }

                Instruction::LdrLit { rt, imm } => {
                    let pc_val = (self.pc & !3).wrapping_add(4);
                    let addr = pc_val.wrapping_add(imm as u32);
                    let val = self.load(bus, addr, AccessWidth::Word)?;
                    self.write_reg(rt, val);
                }

                Instruction::LdrSp { rt, imm } => {
                    let addr = self.sp.wrapping_add(imm as u32);
                    let val = self.load(bus, addr, AccessWidth::Word)?;
                    self.write_reg(rt, val);
                }
                Instruction::StrSp { rt, imm } => {
                    let addr = self.sp.wrapping_add(imm as u32);
                    let val = self.read_reg(rt);
                    self.store(bus, addr, AccessWidth::Word, val)?;
                }
                Instruction::AddSpReg { rd, imm } => {
                    let res = self.sp.wrapping_add(imm as u32);
                    self.write_reg(rd, res);
                }
                Instruction::Adr { rd, imm } => {
                    let pc_val = (self.pc & !3).wrapping_add(4);
                    let res = pc_val.wrapping_add(imm as u32);
                    self.write_reg(rd, res);
                }
                Instruction::AddwImm { rd, rn, imm } => {
                    // Plain 12-bit zero-extended immediate (T4). Distinct
                    // from DataProcImm32::ADD which runs imm12 through
                    // ThumbExpandImm.
                    let res = self.read_reg(rn).wrapping_add(imm as u32);
                    self.write_reg(rd, res);
                    pc_increment = 4;
                }
                Instruction::SubwImm { rd, rn, imm } => {
                    let res = self.read_reg(rn).wrapping_sub(imm as u32);
                    self.write_reg(rd, res);
                    pc_increment = 4;
                }

                // Memory Operations (Byte)
                Instruction::LdrbImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    let val = self.load(bus, addr, AccessWidth::Byte)?;
                    self.write_reg(rt, val);
                }
                Instruction::LdrbReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    let val = self.load(bus, addr, AccessWidth::Byte)?;
                    self.write_reg(rt, val);
                }
                Instruction::StrbReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    let val = self.read_reg(rt) & 0xFF;
                    self.store(bus, addr, AccessWidth::Byte, val)?;
                }
                Instruction::LdrsbReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    let val = self.load(bus, addr, AccessWidth::Byte)?;
                    let res = (val as u8 as i8) as i32 as u32;
                    self.write_reg(rt, res);
                }
                Instruction::LdrhReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    let val = self.load(bus, addr, AccessWidth::Half)?;
                    self.write_reg(rt, val);
                }
                Instruction::StrhReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    let val = self.read_reg(rt) & 0xFFFF;
                    self.store(bus, addr, AccessWidth::Half, val)?;
                }
                Instruction::LdrshReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    let val = self.load(bus, addr, AccessWidth::Half)?;
                    let res = (val as u16 as i16) as i32 as u32;
                    self.write_reg(rt, res);
                }
                Instruction::StrbImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    let val = self.read_reg(rt) & 0xFF;
                    self.store(bus, addr, AccessWidth::Byte, val)?;
                }
                Instruction::LdrhImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    let val = self.load(bus, addr, AccessWidth::Half)?;
                    self.write_reg(rt, val);
                }
                Instruction::StrhImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    let val = self.read_reg(rt) & 0xFFFF;
                    self.store(bus, addr, AccessWidth::Half, val)?;
                }
                Instruction::Bkpt { imm8 } => {
                    // ARM semihosting uses `bkpt #0xAB` as the trap into
                    // the debugger. On real silicon openocd intercepts
                    // these and emulates the syscall (WRITEC, WRITE0,
                    // SYS_EXIT, …). The simulator doesn't emulate the
                    // syscalls itself — firmware that wants the same
                    // bytes available on both sides should also emit
                    // them via UART, which our sink already captures.
                    // Treating semihosting BKPT as a no-op here lets
                    // such dual-emit firmware run identically on sim
                    // and silicon. Any other BKPT immediate (typical
                    // for `panic!` traps or debugger breakpoints) is
                    // still a halt.
                    if imm8 != 0xAB {
                        return Err(crate::SimulationError::Halt);
                    }
                }

                Instruction::Svc { imm8: _ } => {
                    // Supervisor call. Pend the SVCall exception (number 11);
                    // the exception-entry path at the top of `step_internal`
                    // stacks the frame and vectors to the handler on the next
                    // step. Zephyr drives its fatal handler, irq_offload, and
                    // userspace syscalls through SVC, so an unmodeled SVC left
                    // the PC stuck on the instruction (ztest hung forever).
                    // The immediate selects the call on the Zephyr side; the
                    // handler recovers it from the stacked instruction, so we
                    // don't branch on it here. pc_increment stays 2 so the
                    // stacked return address points just past the SVC.
                    self.set_exception_pending(11);
                }

                // Stack Operations
                Instruction::Push { registers, m } => {
                    let mut sp = self.read_reg(13);
                    // Cycle through R14(LR), R7..R0 high to low

                    // If M (LR) is set, push LR first (highest address)
                    if m {
                        sp = sp.wrapping_sub(4);
                        let val = self.read_reg(14);
                        self.store(bus, sp, AccessWidth::Word, val)?;
                    }

                    // Registers R7 down to R0
                    for i in (0..=7).rev() {
                        if (registers & (1 << i)) != 0 {
                            sp = sp.wrapping_sub(4);
                            let val = self.read_reg(i);
                            self.store(bus, sp, AccessWidth::Word, val)?;
                        }
                    }

                    self.write_reg(13, sp);
                }
                Instruction::Pop { registers, p } => {
                    let mut sp = self.read_reg(13);

                    // Registers R0 up to R7
                    for i in 0..=7 {
                        if (registers & (1 << i)) != 0 {
                            let val = self.load(bus, sp, AccessWidth::Word)?;
                            self.write_reg(i, val);
                            sp = sp.wrapping_add(4);
                        }
                    }

                    // If P (PC) is set, pop PC (lowest address?? No, highest)
                    // POP is inverse of PUSH. PUSH pushed LR last (lowest addr) ??
                    // Wait. PUSH stores STMDB (Decrement Before). Highest reg = Highest address.
                    // R0 is lowest register. LR is highest.
                    // PUSH order: LR, R7, ... R0.
                    // Stack grows down.
                    // Low Addr [ R0 | R1 | ... | LR ] High Addr.
                    // So POP (LDMIA) should read: R0, ... R7, PC.
                    // My PUSH loop:
                    // 1. If LR, sub 4, write LR. (Top of stack, highest addr - 4)
                    // 2. Loop 7 down to 0: sub 4, write Rx.
                    // Result: R0 is at current SP. LR is at SP + n*4.

                    // My POP loop:
                    // 1. Loop 0 to 7: read, add 4. (Read R0, R1...)
                    // 2. If PC, read, add 4.

                    if p {
                        let val = self.load(bus, sp, AccessWidth::Word)?;
                        // Commit SP before branching so EXC_RETURN unstacking reads the
                        // hardware exception frame, not this function's software save area.
                        sp = sp.wrapping_add(4);
                        self.write_reg(13, sp);
                        self.branch_to(val, bus)?;
                        pc_increment = 0; // Branch taken
                    } else {
                        self.write_reg(13, sp);
                    }
                }
                Instruction::Ldm { rn, registers } => {
                    let mut base = self.read_reg(rn);
                    for i in 0..=7 {
                        if (registers & (1 << i)) != 0 {
                            let val = self.load(bus, base, AccessWidth::Word)?;
                            self.write_reg(i, val);
                            base = base.wrapping_add(4);
                        }
                    }
                    // LDM (T1) writeback: the base register is written back with
                    // the incremented address ONLY when it is NOT in the register
                    // list. When the base IS in the list (the assembler emits no
                    // `!`, e.g. `ldmia r2, {r0,r1,r2}`), the ARMv6-M architecture
                    // specifies the loaded value wins and no writeback occurs.
                    // Writing back unconditionally clobbered the just-loaded value
                    // — which corrupted the compiler's struct-copy / stacked-arg
                    // reload idiom and silently dropped a loaded argument.
                    if (registers & (1 << rn)) == 0 {
                        self.write_reg(rn, base);
                    }
                }
                Instruction::Stm { rn, registers } => {
                    let mut base = self.read_reg(rn);
                    for i in 0..=7 {
                        if (registers & (1 << i)) != 0 {
                            let val = self.read_reg(i);
                            self.store(bus, base, AccessWidth::Word, val)?;
                            base = base.wrapping_add(4);
                        }
                    }
                    self.write_reg(rn, base);
                }
                // STMDB Rn(!), {reg_list} — 32-bit store multiple, decrement before.
                // Lowest-numbered register stored at lowest address.
                Instruction::StmdbW {
                    rn,
                    reg_list,
                    writeback,
                } => {
                    let count = reg_list.count_ones();
                    let mut addr = self.read_reg(rn).wrapping_sub(count * 4);
                    let start = addr;
                    for i in 0u8..=15 {
                        if (reg_list & (1 << i)) != 0 {
                            let val = self.read_reg(i);
                            self.store(bus, addr, AccessWidth::Word, val)?;
                            addr = addr.wrapping_add(4);
                        }
                    }
                    if writeback {
                        self.write_reg(rn, start);
                    }
                    pc_increment = 4;
                }
                // STMIA.W Rn(!), {reg_list} — 32-bit store multiple, increment
                // after. Lowest-numbered register stored at the base address.
                Instruction::StmiaW {
                    rn,
                    reg_list,
                    writeback,
                } => {
                    let mut addr = self.read_reg(rn);
                    for i in 0u8..=14 {
                        if (reg_list & (1 << i)) != 0 {
                            let val = self.read_reg(i);
                            self.store(bus, addr, AccessWidth::Word, val)?;
                            addr = addr.wrapping_add(4);
                        }
                    }
                    if writeback {
                        self.write_reg(rn, addr);
                    }
                    pc_increment = 4;
                }
                // LDMDB.W Rn(!), {reg_list} — 32-bit load multiple, decrement
                // before. Lowest-numbered register loaded from the lowest address.
                Instruction::LdmdbW {
                    rn,
                    reg_list,
                    writeback,
                } => {
                    let count = reg_list.count_ones();
                    let start = self.read_reg(rn).wrapping_sub(count * 4);
                    let mut addr = start;
                    for i in 0u8..=14 {
                        if (reg_list & (1 << i)) != 0 {
                            let val = self.load(bus, addr, AccessWidth::Word)?;
                            self.write_reg(i, val);
                            addr = addr.wrapping_add(4);
                        }
                    }
                    if writeback {
                        self.write_reg(rn, start);
                    }
                    if (reg_list & (1 << 15)) != 0 {
                        let pc_val = self.load(bus, addr, AccessWidth::Word)?;
                        self.branch_to(pc_val, bus)?;
                        pc_increment = 0;
                    } else {
                        pc_increment = 4;
                    }
                }
                // LDMIA.W Rn(!), {reg_list} — 32-bit load multiple, increment after.
                // Lowest-numbered register loaded from lowest address.
                Instruction::LdmiaW {
                    rn,
                    reg_list,
                    writeback,
                } => {
                    let mut addr = self.read_reg(rn);
                    // Load R0-R14 (skip PC; handle separately to commit SP first)
                    for i in 0u8..=14 {
                        if (reg_list & (1 << i)) != 0 {
                            let val = self.load(bus, addr, AccessWidth::Word)?;
                            self.write_reg(i, val);
                            addr = addr.wrapping_add(4);
                        }
                    }
                    // Handle PC (bit 15) — commit writeback before branching
                    if (reg_list & (1 << 15)) != 0 {
                        let pc_val = self.load(bus, addr, AccessWidth::Word)?;
                        addr = addr.wrapping_add(4);
                        if writeback {
                            self.write_reg(rn, addr);
                        }
                        self.branch_to(pc_val, bus)?;
                        pc_increment = 0;
                    } else {
                        if writeback {
                            self.write_reg(rn, addr);
                        }
                        pc_increment = 4;
                    }
                }

                // Control Flow
                Instruction::Bl { offset } => {
                    // BL: Branch with Link.
                    // LR = Next Instruction Address | 1 (Thumb bit)
                    let _next_pc = self.pc.wrapping_add(4); // 32-bit instruction size for BL?
                                                            // Wait. BL is decoded as 32-bit.
                                                            // If we assume decode_thumb_16 handled a 32-bit stream, then PC increment should be adjusted?
                                                            // Or does `decode_thumb_16` return `BlPrefix` and then we handle it?
                                                            // The current `decoder` returns `Bl` with full offset if it sees the pair??
                                                            // NO. My decoder implementation for BL (in previous turn) was:
                                                            // `Instruction::Bl { offset: offset << 1 }`
                                                            // But `decode_thumb_16` ONLY sees 16 bits. It cannot see the second half!
                                                            // Real decoding of BL requires fetching 32 bits.

                    // CRITICAL CORRECTION: `decode_thumb_16` is 16-bit.
                    // BL is 32-bit (encoded as two 16-bit halves).
                    // Fetch loop fetches 16 bits.
                    // 1. Fetch High Half (0xF0xx). Returns BlPrefix?
                    // 2. Fetch Low Half (0xF8xx). Combine?

                    // My logic in decoder needs revisit. I put `Bl { offset }` thinking T1/T2 but BL is always 32-bit in Thumb-2.
                    // T1 encoding of BL doesn't exist as single 16-bit.

                    // For now, let's just implement the execution stub assuming the decoder *somehow* gave us the full BL.
                    // But since the decoder only sees 16 bits, we need to handle the prefix state in the CPU loop!

                    self.lr = self.pc.wrapping_add(4) | 1;
                    let target = (self.pc as i32).wrapping_add(4).wrapping_add(offset) as u32;
                    self.pc = target;
                    pc_increment = 0;
                }
                Instruction::BranchCond { cond, offset } => {
                    if self.check_condition(cond) {
                        let target = (self.pc as i32).wrapping_add(4).wrapping_add(offset) as u32;
                        self.pc = target;
                        pc_increment = 0;
                    }
                }
                Instruction::Bx { rm } => {
                    let target = self.read_reg(rm);
                    self.branch_to(target, bus)?;
                    pc_increment = 0;
                }

                // BLX Rm (T1): branch-with-link to register address.
                // Sets LR = (PC_of_blx + 2) | 1 before branching.
                Instruction::BlxReg { rm } => {
                    let target = self.read_reg(rm);
                    self.lr = (self.pc.wrapping_add(2)) | 1;
                    self.branch_to(target, bus)?;
                    pc_increment = 0;
                }

                // --- Thumb-2 ARMv7-M additions ---
                Instruction::Barrier => {
                    // DMB / DSB / ISB — architectural no-ops on a single-threaded
                    // simulator. They're modelled explicitly so they don't raise
                    // DecodeError; startup code and HAL inline-asm emit them
                    // routinely.
                    pc_increment = 4;
                }
                Instruction::Mrs { rd, sysm } => {
                    // IPSR (the active exception number, xPSR[8:0]) is load-bearing
                    // for Zephyr: _isr_wrapper reads it and computes `IRQ = IPSR-16`
                    // to index the software ISR table. Returning 0 made the index
                    // -16 → garbage handler. The xPSR/IPSR-bearing reads all expose
                    // the current exception number; PRIMASK, BASEPRI, FAULTMASK,
                    // the banked SPs and CONTROL are the other modelled special
                    // registers. Anything else still reads as zero.
                    let ipsr = self.active_exception & 0x1FF;
                    let val: u32 = match sysm {
                        0x00 => self.xpsr & 0xF800_0000,          // APSR (condition flags)
                        0x03 => (self.xpsr & 0xF800_0000) | ipsr, // xPSR
                        0x05 => ipsr,                             // IPSR
                        0x08 => self.read_msp(),                  // MSP
                        0x09 => self.read_psp(),                  // PSP
                        0x10 => self.primask as u32,
                        // BASEPRI (0x11) and BASEPRI_MAX (0x12) both read BASEPRI.
                        0x11 | 0x12 => self.basepri as u32,
                        0x13 => self.faultmask as u32, // FAULTMASK
                        0x14 => self.control & 0x3,    // CONTROL
                        _ => 0,
                    };
                    self.write_reg(rd, val);
                    pc_increment = 4;
                }
                Instruction::Msr { sysm, rn } => {
                    let val = self.read_reg(rn);
                    match sysm {
                        0x08 => {
                            // MSP bank. If MSP is the live stack, update `sp` too.
                            self.msp = val;
                            if !self.use_psp() {
                                self.sp = val;
                            }
                        }
                        0x09 => {
                            // PSP bank. If PSP is the live stack, update `sp` too.
                            self.psp = val;
                            if self.use_psp() {
                                self.sp = val;
                            }
                        }
                        0x10 => self.primask = (val & 1) != 0,
                        // BASEPRI: plain write of the priority mask byte.
                        0x11 => self.basepri = (val & 0xFF) as u8,
                        // BASEPRI_MAX: writes BASEPRI only if it raises the
                        // masking level (smaller non-zero value), or BASEPRI is 0.
                        0x12 => {
                            let new = (val & 0xFF) as u8;
                            if new != 0 && (self.basepri == 0 || new < self.basepri) {
                                self.basepri = new;
                            }
                        }
                        0x13 => self.faultmask = (val & 1) != 0, // FAULTMASK
                        0x14 => {
                            // CONTROL.SPSEL can switch the active thread stack.
                            // Persist the live `sp` to its bank, change SPSEL/nPRIV,
                            // then re-point `sp` at the newly-selected bank.
                            self.sync_sp_to_bank();
                            self.control = (self.control & !0x3) | (val & 0x3);
                            self.sp = self.current_stack_value();
                        }
                        _ => {}
                    }
                    pc_increment = 4;
                }
                Instruction::Smull {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    let lhs = self.read_reg(rn) as i32 as i64;
                    let rhs = self.read_reg(rm) as i32 as i64;
                    let prod = lhs.wrapping_mul(rhs) as u64;
                    self.write_reg(rd_lo, prod as u32);
                    self.write_reg(rd_hi, (prod >> 32) as u32);
                    pc_increment = 4;
                }
                Instruction::Umull {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    let prod = (self.read_reg(rn) as u64).wrapping_mul(self.read_reg(rm) as u64);
                    self.write_reg(rd_lo, prod as u32);
                    self.write_reg(rd_hi, (prod >> 32) as u32);
                    pc_increment = 4;
                }
                Instruction::Smlal {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    let acc = ((self.read_reg(rd_hi) as u64) << 32) | (self.read_reg(rd_lo) as u64);
                    let lhs = self.read_reg(rn) as i32 as i64;
                    let rhs = self.read_reg(rm) as i32 as i64;
                    let new = (acc as i64).wrapping_add(lhs.wrapping_mul(rhs)) as u64;
                    self.write_reg(rd_lo, new as u32);
                    self.write_reg(rd_hi, (new >> 32) as u32);
                    pc_increment = 4;
                }
                Instruction::Umlal {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    let acc = ((self.read_reg(rd_hi) as u64) << 32) | (self.read_reg(rd_lo) as u64);
                    let prod = (self.read_reg(rn) as u64).wrapping_mul(self.read_reg(rm) as u64);
                    let new = acc.wrapping_add(prod);
                    self.write_reg(rd_lo, new as u32);
                    self.write_reg(rd_hi, (new >> 32) as u32);
                    pc_increment = 4;
                }
                Instruction::Umaal {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    // (rd_hi:rd_lo) = Rn*Rm + rd_lo + rd_hi. Cannot overflow u64.
                    let prod = (self.read_reg(rn) as u64).wrapping_mul(self.read_reg(rm) as u64);
                    let res = prod
                        .wrapping_add(self.read_reg(rd_lo) as u64)
                        .wrapping_add(self.read_reg(rd_hi) as u64);
                    self.write_reg(rd_lo, res as u32);
                    self.write_reg(rd_hi, (res >> 32) as u32);
                    pc_increment = 4;
                }
                Instruction::Mla { rd, rn, rm, ra } => {
                    let res = self
                        .read_reg(ra)
                        .wrapping_add(self.read_reg(rn).wrapping_mul(self.read_reg(rm)));
                    self.write_reg(rd, res);
                    pc_increment = 4;
                }
                Instruction::Mls { rd, rn, rm, ra } => {
                    let res = self
                        .read_reg(ra)
                        .wrapping_sub(self.read_reg(rn).wrapping_mul(self.read_reg(rm)));
                    self.write_reg(rd, res);
                    pc_increment = 4;
                }
                Instruction::SmlaXy {
                    rd,
                    rn,
                    rm,
                    ra,
                    n_top,
                    m_top,
                    accumulate,
                } => {
                    // ⚠️ SIGNED halves, sign-extended before the multiply. Taking
                    // them as u16 would agree with the hardware on every small
                    // positive operand and disagree on every negative one — the
                    // shape of bug that passes a smoke test and fails a sensor.
                    let half = |value: u32, top: bool| -> i32 {
                        (if top {
                            (value >> 16) as u16
                        } else {
                            value as u16
                        }) as i16 as i32
                    };
                    let product =
                        half(self.read_reg(rn), n_top).wrapping_mul(half(self.read_reg(rm), m_top));
                    // The SMUL forms have no addend; `ra` there is the 0b1111
                    // encoding marker, not a register.
                    let res = if accumulate {
                        (self.read_reg(ra) as i32).wrapping_add(product)
                    } else {
                        product
                    };
                    self.write_reg(rd, res as u32);
                    pc_increment = 4;
                }

                // -------- VFPv4 single-precision (FPU) --------
                Instruction::Vldr { sd, rn, imm, add } => {
                    let base = self.read_reg(rn);
                    let base = if rn == 15 { base & !3 } else { base };
                    let addr = if add {
                        base.wrapping_add(imm as u32)
                    } else {
                        base.wrapping_sub(imm as u32)
                    };
                    let val = self.load(bus, addr, AccessWidth::Word)?;
                    self.fpu_s[sd as usize] = val;
                    pc_increment = 4;
                }
                Instruction::Vstr { sd, rn, imm, add } => {
                    let base = self.read_reg(rn);
                    let addr = if add {
                        base.wrapping_add(imm as u32)
                    } else {
                        base.wrapping_sub(imm as u32)
                    };
                    let val = self.fpu_s[sd as usize];
                    self.store(bus, addr, AccessWidth::Word, val)?;
                    pc_increment = 4;
                }
                Instruction::VmulF32 { sd, sn, sm } => {
                    let a = self.fpu_s[sn as usize];
                    let b = self.fpu_s[sm as usize];
                    self.fpu_s[sd as usize] = vfp_binop(VfpBinOp::Mul, a, b, self.fpscr);
                    pc_increment = 4;
                }
                Instruction::VaddF32 { sd, sn, sm } => {
                    let a = self.fpu_s[sn as usize];
                    let b = self.fpu_s[sm as usize];
                    self.fpu_s[sd as usize] = vfp_binop(VfpBinOp::Add, a, b, self.fpscr);
                    pc_increment = 4;
                }
                Instruction::VsubF32 { sd, sn, sm } => {
                    let a = self.fpu_s[sn as usize];
                    let b = self.fpu_s[sm as usize];
                    self.fpu_s[sd as usize] = vfp_binop(VfpBinOp::Sub, a, b, self.fpscr);
                    pc_increment = 4;
                }
                Instruction::VdivF32 { sd, sn, sm } => {
                    let a = self.fpu_s[sn as usize];
                    let b = self.fpu_s[sm as usize];
                    self.fpu_s[sd as usize] = vfp_binop(VfpBinOp::Div, a, b, self.fpscr);
                    pc_increment = 4;
                }
                Instruction::VfmaF32 { sd, sn, sm } => {
                    let a = self.fpu_s[sn as usize];
                    let b = self.fpu_s[sm as usize];
                    let c = self.fpu_s[sd as usize];
                    // Fused: single rounding of (a*b)+c, not a*b rounded then +c.
                    self.fpu_s[sd as usize] = vfp_fma(a, b, c, false, false, self.fpscr);
                }
                Instruction::VfmsF32 { sd, sn, sm } => {
                    let a = self.fpu_s[sn as usize];
                    let b = self.fpu_s[sm as usize];
                    let c = self.fpu_s[sd as usize];
                    self.fpu_s[sd as usize] = vfp_fma(a, b, c, true, false, self.fpscr);
                }
                Instruction::VfnmaF32 { sd, sn, sm } => {
                    let a = self.fpu_s[sn as usize];
                    let b = self.fpu_s[sm as usize];
                    let c = self.fpu_s[sd as usize];
                    self.fpu_s[sd as usize] = vfp_fma(a, b, c, false, true, self.fpscr);
                }
                Instruction::VfnmsF32 { sd, sn, sm } => {
                    let a = self.fpu_s[sn as usize];
                    let b = self.fpu_s[sm as usize];
                    let c = self.fpu_s[sd as usize];
                    self.fpu_s[sd as usize] = vfp_fma(a, b, c, true, true, self.fpscr);
                }
                Instruction::VmovSnRt { sn, rt } => {
                    self.fpu_s[sn as usize] = self.read_reg(rt);
                    pc_increment = 4;
                }
                Instruction::VmovRtSn { rt, sn } => {
                    self.write_reg(rt, self.fpu_s[sn as usize]);
                    pc_increment = 4;
                }
                Instruction::VmovF32Reg { sd, sm } => {
                    self.fpu_s[sd as usize] = self.fpu_s[sm as usize];
                    pc_increment = 4;
                }

                Instruction::VmovF32Imm { sd, imm_bits } => {
                    self.fpu_s[sd as usize] = imm_bits;
                    pc_increment = 4;
                }
                Instruction::VcvtF32FromInt {
                    sd,
                    sm,
                    signed,
                    fbits,
                } => {
                    let raw = self.fpu_s[sm as usize];
                    let int_val = if signed {
                        raw as i32 as f64
                    } else {
                        raw as f64
                    };
                    let scaled = if fbits > 0 {
                        int_val / ((1u64 << fbits.min(31)) as f64)
                    } else {
                        int_val
                    };
                    self.fpu_s[sd as usize] = (scaled as f32).to_bits();
                    pc_increment = 4;
                }
                Instruction::VcvtIntFromF32 {
                    sd,
                    sm,
                    signed,
                    fbits,
                } => {
                    let f = f32::from_bits(self.fpu_s[sm as usize]) as f64;
                    let scaled = if fbits > 0 {
                        f * ((1u64 << fbits.min(31)) as f64)
                    } else {
                        f
                    };
                    let bits = if signed {
                        (scaled as i32) as u32
                    } else if scaled <= 0.0 {
                        0
                    } else if scaled >= f64::from(u32::MAX) {
                        u32::MAX
                    } else {
                        scaled as u32
                    };
                    self.fpu_s[sd as usize] = bits;
                    pc_increment = 4;
                }

                // -------- VFP load/store multiple + double-precision (FPv5-D16) --------
                Instruction::VfpStoreMultiple {
                    rn,
                    s_first,
                    count,
                    add,
                    wback,
                } => {
                    let base = self.read_reg(rn);
                    let total = 4u32.wrapping_mul(count as u32);
                    let start = if add { base } else { base.wrapping_sub(total) };
                    for i in 0..count {
                        let idx = s_first as usize + i as usize;
                        let val = if idx < 32 { self.fpu_s[idx] } else { 0 };
                        let addr = start.wrapping_add(4 * i as u32);
                        self.store(bus, addr, AccessWidth::Word, val)?;
                    }
                    if wback {
                        let nb = if add {
                            base.wrapping_add(total)
                        } else {
                            base.wrapping_sub(total)
                        };
                        self.write_reg(rn, nb);
                    }
                    pc_increment = 4;
                }
                Instruction::VfpLoadMultiple {
                    rn,
                    s_first,
                    count,
                    add,
                    wback,
                } => {
                    let base = self.read_reg(rn);
                    let total = 4u32.wrapping_mul(count as u32);
                    let start = if add { base } else { base.wrapping_sub(total) };
                    for i in 0..count {
                        let idx = s_first as usize + i as usize;
                        let addr = start.wrapping_add(4 * i as u32);
                        let v = self.load(bus, addr, AccessWidth::Word)?;
                        if idx < 32 {
                            self.fpu_s[idx] = v;
                        }
                    }
                    if wback {
                        let nb = if add {
                            base.wrapping_add(total)
                        } else {
                            base.wrapping_sub(total)
                        };
                        self.write_reg(rn, nb);
                    }
                    pc_increment = 4;
                }
                Instruction::Vldr64 { dd, rn, imm, add } => {
                    let base = self.read_reg(rn);
                    let base = if rn == 15 { base & !3 } else { base };
                    let addr = if add {
                        base.wrapping_add(imm as u32)
                    } else {
                        base.wrapping_sub(imm as u32)
                    };
                    for (w, off) in [(0usize, 0u32), (1, 4)] {
                        let v = self.load(bus, addr.wrapping_add(off), AccessWidth::Word)?;
                        if (dd as usize + w) < 32 {
                            self.fpu_s[dd as usize + w] = v;
                        }
                    }
                    pc_increment = 4;
                }
                Instruction::Vstr64 { dd, rn, imm, add } => {
                    let base = self.read_reg(rn);
                    let addr = if add {
                        base.wrapping_add(imm as u32)
                    } else {
                        base.wrapping_sub(imm as u32)
                    };
                    for (w, off) in [(0usize, 0u32), (1, 4)] {
                        let val = if (dd as usize + w) < 32 {
                            self.fpu_s[dd as usize + w]
                        } else {
                            0
                        };
                        self.store(bus, addr.wrapping_add(off), AccessWidth::Word, val)?;
                    }
                    pc_increment = 4;
                }
                Instruction::VmovF64Reg { dd, dm } => {
                    if (dd as usize + 1) < 32 && (dm as usize + 1) < 32 {
                        self.fpu_s[dd as usize] = self.fpu_s[dm as usize];
                        self.fpu_s[dd as usize + 1] = self.fpu_s[dm as usize + 1];
                    }
                    pc_increment = 4;
                }
                Instruction::VmovDRtRt2 { dm, rt, rt2 } => {
                    if (dm as usize + 1) < 32 {
                        self.fpu_s[dm as usize] = self.read_reg(rt);
                        self.fpu_s[dm as usize + 1] = self.read_reg(rt2);
                    }
                    pc_increment = 4;
                }
                Instruction::VmovRtRt2D { rt, rt2, dm } => {
                    let (lo, hi) = if (dm as usize + 1) < 32 {
                        (self.fpu_s[dm as usize], self.fpu_s[dm as usize + 1])
                    } else {
                        (0, 0)
                    };
                    self.write_reg(rt, lo);
                    self.write_reg(rt2, hi);
                    pc_increment = 4;
                }
                Instruction::VaddF64 { dd, dn, dm } => {
                    let r = self.read_f64(dn) + self.read_f64(dm);
                    self.write_f64(dd, r);
                    pc_increment = 4;
                }
                Instruction::VsubF64 { dd, dn, dm } => {
                    let r = self.read_f64(dn) - self.read_f64(dm);
                    self.write_f64(dd, r);
                    pc_increment = 4;
                }
                Instruction::VmulF64 { dd, dn, dm } => {
                    let r = self.read_f64(dn) * self.read_f64(dm);
                    self.write_f64(dd, r);
                    pc_increment = 4;
                }
                Instruction::VdivF64 { dd, dn, dm } => {
                    let r = self.read_f64(dn) / self.read_f64(dm);
                    self.write_f64(dd, r);
                    pc_increment = 4;
                }

                Instruction::Unknown(op) => {
                    tracing::warn!("Unknown instruction at {:#x}: Opcode {:#06x}", self.pc, op);
                    crate::fidelity::record_undecoded(self.pc, op as u64, "undecoded T16");
                    // Silicon raises UsageFault (UNDEFINSTR) here. This used to
                    // `pc_increment = 2` and carry on, which left every register
                    // stale and the run ending green — see the note on
                    // `escalate_undefined_instruction`.
                    self.pending_undef_instruction = true;
                    return Err(SimulationError::DecodeError(self.pc as u64));
                }
            }
        }

        if it_block_instruction && self.it_state != 0 {
            // ITSTATEUpdate(): advance the low 5 bits, preserving only firstcond[3:1].
            // Bit 4 of the low field becomes cond[0] for the next instruction, so the full
            // 5-bit field must shift, not just the low nibble. This is what flips THEN/ELSE
            // instructions inside blocks such as `ITTEE`.
            self.it_state = (self.it_state & 0xE0) | (((self.it_state & 0x1F) << 1) & 0x1F);
            if (self.it_state & 0x0F) == 0 {
                self.it_state = 0;
            }
        }

        self.pc = self.pc.wrapping_add(pc_increment);

        // Building the register snapshot is pure waste when nothing observes it,
        // and this runs on every instruction. Gate it on having observers, the
        // same way on_step_start above is gated.
        if !_observers.is_empty() {
            let mut registers = [0u32; 19];
            for (i, reg) in registers.iter_mut().enumerate().take(16) {
                *reg = self.get_register(i as u8);
            }
            registers[16] = self.xpsr;
            // Standard trailer (see `SimulationObserver`): SP then PC. Both
            // already live in the arch block (r13/r15); repeating them here is
            // what lets an arch-agnostic consumer find them.
            registers[17] = self.get_register(13);
            registers[18] = self.pc;

            crate::emit_trace_event(
                _observers,
                labwired_hw_trace::TraceEvent::InstructionRetired {
                    pc: retired_pc,
                    opcode,
                },
            );
            for obs in _observers {
                obs.on_step_end(_cycles, &registers);
            }
        }

        Ok(())
    }
}

// Thumb expand immediate - implements ARM's modified immediate constant expansion
fn thumb_expand_imm(imm12: u32) -> u32 {
    let i = (imm12 >> 11) & 1;
    let imm3 = (imm12 >> 8) & 7;
    let imm8 = imm12 & 0xFF;

    if i == 0 && (imm3 >> 2) == 0 {
        // i:imm3 is 0000, 0001, 0010, 0011.
        // Match repetition patterns:
        match imm3 {
            0 => imm8,                       // 00000000 00000000 00000000 abcdefgh
            1 => (imm8 << 16) | imm8,        // 00000000 abcdefgh 00000000 abcdefgh
            2 => (imm8 << 24) | (imm8 << 8), // abcdefgh 00000000 abcdefgh 00000000
            3 => (imm8 << 24) | (imm8 << 16) | (imm8 << 8) | imm8, // abcdefgh abcdefgh abcdefgh abcdefgh
            _ => unreachable!(),
        }
    } else {
        // Rotated immediate
        // The value to rotate is '1' concatenated with bits 6:0 of imm8.
        let val = 0x80 | (imm8 & 0x7F);
        // The rotation amount 'n' is i:imm3:imm8[7]
        let n = (i << 4) | (imm3 << 1) | (imm8 >> 7);
        val.rotate_right(n)
    }
}

fn add_with_flags(op1: u32, op2: u32) -> (u32, bool, bool) {
    let (res, overflow1) = op1.overflowing_add(op2);
    let carry = overflow1;
    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    let overflow = (neg_op1 == neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}

fn adc_with_flags(op1: u32, op2: u32, carry_in: u32) -> (u32, bool, bool) {
    let (res1, c1) = op1.overflowing_add(op2);
    let (res, c2) = res1.overflowing_add(carry_in);
    let carry = c1 || c2;

    // Overflow: operands have same sign AND result has different sign
    // Effectively (op1 + op2 + carry) overflowed signed range.
    // Approximate check:
    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    // Overflow if inputs same sign, output different
    // Note: Carry_in 0 or 1 usually doesn't change sign logic much, but rigorous check:
    // Sign of (op1 + op2 + carry). It's simpler to rely on basic sign logic or specific algo.
    // ARM ref: Overflow = (op1<31> == op2<31>) && (res<31> != op1<31>)
    // Wait, carry_in effectively adds small value.
    // If op1=MAX, op2=1, c=0 -> overflow pos to neg.
    // Standard V flag logic:
    let overflow = (neg_op1 == neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}

fn sub_with_flags(op1: u32, op2: u32) -> (u32, bool, bool) {
    let (res, borrow) = op1.overflowing_sub(op2);
    let carry = !borrow;
    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    let overflow = (neg_op1 != neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}

fn sbc_with_flags(op1: u32, op2: u32, carry_in: u32) -> (u32, bool, bool) {
    // SBC: op1 - op2 - NOT(carry) = op1 - op2 - (1 - carry)
    let borrow_in = 1 - carry_in;
    let (res1, b1) = op1.overflowing_sub(op2);
    let (res, b2) = res1.overflowing_sub(borrow_in);
    let borrow = b1 || b2;
    let carry = !borrow;

    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    let overflow = (neg_op1 != neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}

#[cfg(test)]
#[path = "cortex_m_tests.rs"]
mod tests;
