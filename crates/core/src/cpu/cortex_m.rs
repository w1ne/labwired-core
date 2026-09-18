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

/// `step_execute`'s instruction match arm bodies, one method per arm, grouped
/// by instruction class. The single dispatch `match instruction` stays in
/// `step_execute`; each arm calls one `exec_*` method. `#[path]` keeps the
/// submodules inside this file's directory without moving the crate path.
#[path = "cortex_m/exec/mod.rs"]
mod exec;

/// How an executed arm advances PC relative to the decoded default. Returned
/// by every `exec_*` method instead of mutating a `&mut` out-parameter: the
/// hot `Keep` case costs nothing once the method is inlined.
#[derive(Clone, Copy)]
#[repr(u8)]
pub(in crate::cpu::cortex_m) enum PcAdvance {
    /// Leave `pc_increment` at its decoded value (the common 16-bit case).
    Keep,
    /// The arm set `self.pc` itself; add nothing.
    Zero,
    /// Advance past a 32-bit instruction.
    Add4,
}

impl PcAdvance {
    #[inline(always)]
    fn apply(self, current: u32) -> u32 {
        match self {
            PcAdvance::Keep => current,
            PcAdvance::Zero => 0,
            PcAdvance::Add4 => 4,
        }
    }
}

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
    /// Cached `LABWIRED_TRACE_INSN` verdict, read once at construction rather
    /// than through `trace_insn_enabled()`'s `OnceLock::get_or_init` on every
    /// retired instruction. The `OnceLock` was already a fix for a prior
    /// `std::env::var`-per-step regression, but its own already-initialized
    /// fast path (an `Acquire` load through the lock's state machine plus the
    /// `Option`/branch around it) still cost ~9 Ir/instruction on the batched
    /// loop — this field is a plain `bool` already resident in the `CortexM`
    /// struct callers have just touched, so the check is a single load with
    /// no atomic and no OnceLock machinery in the hot path.
    trace_insn: bool,
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
            trace_insn: trace_insn_enabled(),
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
        if self.trace_insn {
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
                    pc_increment = self.exec_bfi(rd, rn, lsb, width)?.apply(pc_increment);
                }
                Instruction::Bfc { rd, lsb, width } => {
                    pc_increment = self.exec_bfc(rd, lsb, width)?.apply(pc_increment);
                }
                Instruction::Sbfx { rd, rn, lsb, width } => {
                    pc_increment = self.exec_sbfx(rd, rn, lsb, width)?.apply(pc_increment);
                }
                Instruction::Ubfx { rd, rn, lsb, width } => {
                    pc_increment = self.exec_ubfx(rd, rn, lsb, width)?.apply(pc_increment);
                }
                Instruction::Clz { rd, rm } => {
                    pc_increment = self.exec_clz(rd, rm)?.apply(pc_increment);
                }
                Instruction::Rbit { rd, rm } => {
                    pc_increment = self.exec_rbit(rd, rm)?.apply(pc_increment);
                }
                Instruction::SimdAddSub8 { rd, rn, rm, op } => {
                    pc_increment = self.exec_simd_add_sub8(rd, rn, rm, op)?.apply(pc_increment);
                }
                Instruction::SimdAddSub16 {
                    rd,
                    rn,
                    rm,
                    op,
                    sub,
                } => {
                    pc_increment = self
                        .exec_simd_add_sub16(rd, rn, rm, op, sub)?
                        .apply(pc_increment);
                }
                Instruction::Sel { rd, rn, rm } => {
                    pc_increment = self.exec_sel(rd, rn, rm)?.apply(pc_increment);
                }
                Instruction::Sdiv { rd, rn, rm } => {
                    pc_increment = self.exec_sdiv(rd, rn, rm)?.apply(pc_increment);
                }
                Instruction::Udiv { rd, rn, rm } => {
                    pc_increment = self.exec_udiv(rd, rn, rm)?.apply(pc_increment);
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
                    pc_increment = self
                        .exec_data_proc32(op, rn, rd, rm, imm5, shift_type, set_flags)?
                        .apply(pc_increment);
                }
                Instruction::DataProcImm32 {
                    op,
                    rn,
                    rd,
                    imm12,
                    set_flags,
                } => {
                    pc_increment = self
                        .exec_data_proc_imm32(op, rn, rd, imm12, set_flags)?
                        .apply(pc_increment);
                }
                Instruction::ShiftReg32 {
                    rd,
                    rn,
                    rm,
                    shift_type,
                } => {
                    pc_increment = self
                        .exec_shift_reg32(rd, rn, rm, shift_type)?
                        .apply(pc_increment);
                }
                Instruction::Movw { rd, imm } => {
                    pc_increment = self.exec_movw(rd, imm)?.apply(pc_increment);
                }
                Instruction::Movt { rd, imm } => {
                    pc_increment = self.exec_movt(rd, imm)?.apply(pc_increment);
                }
                Instruction::LdrImm32 { rt, rn, imm12 } => {
                    pc_increment = self.exec_ldr_imm32(bus, rt, rn, imm12)?.apply(pc_increment);
                }
                Instruction::StrImm32 { rt, rn, imm12 } => {
                    pc_increment = self.exec_str_imm32(bus, rt, rn, imm12)?.apply(pc_increment);
                }
                Instruction::LdrImm32Idx {
                    rt,
                    rn,
                    imm8,
                    pre_index,
                    add,
                    writeback,
                } => {
                    pc_increment = self
                        .exec_ldr_imm32_idx(bus, rt, rn, imm8, pre_index, add, writeback)?
                        .apply(pc_increment);
                }
                Instruction::StrImm32Idx {
                    rt,
                    rn,
                    imm8,
                    pre_index,
                    add,
                    writeback,
                } => {
                    pc_increment = self
                        .exec_str_imm32_idx(bus, rt, rn, imm8, pre_index, add, writeback)?
                        .apply(pc_increment);
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
                    pc_increment = self
                        .exec_ldrd(bus, rt, rt2, rn, imm8, add_imm, index, writeback)?
                        .apply(pc_increment);
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
                    pc_increment = self
                        .exec_strd(bus, rt, rt2, rn, imm8, add_imm, index, writeback)?
                        .apply(pc_increment);
                }
                Instruction::Tbb { rn, rm } => {
                    pc_increment = self.exec_tbb(bus, rn, rm)?.apply(pc_increment);
                }
                Instruction::Tbh { rn, rm } => {
                    pc_increment = self.exec_tbh(bus, rn, rm)?.apply(pc_increment);
                }
                Instruction::Unknown32(h1, h2) => {
                    pc_increment = self.exec_unknown32(bus, h1, h2)?.apply(pc_increment);
                }
                Instruction::Nop => { /* Do nothing */ }
                Instruction::Wfi => {
                    pc_increment = self.exec_wfi()?.apply(pc_increment);
                }
                Instruction::MovImm { rd, imm } => {
                    pc_increment = self
                        .exec_mov_imm(rd, imm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::Cbz { rn, imm } => {
                    pc_increment = self.exec_cbz(rn, imm)?.apply(pc_increment);
                }
                Instruction::Cbnz { rn, imm } => {
                    pc_increment = self.exec_cbnz(rn, imm)?.apply(pc_increment);
                }
                Instruction::Branch { offset } => {
                    pc_increment = self.exec_branch(offset)?.apply(pc_increment);
                }
                Instruction::AddReg { rd, rn, rm } => {
                    pc_increment = self
                        .exec_add_reg(rd, rn, rm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::AddImm3 { rd, rn, imm } => {
                    pc_increment = self
                        .exec_add_imm3(rd, rn, imm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::AddImm8 { rd, imm } => {
                    pc_increment = self
                        .exec_add_imm8(rd, imm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::SubReg { rd, rn, rm } => {
                    pc_increment = self
                        .exec_sub_reg(rd, rn, rm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::SubImm3 { rd, rn, imm } => {
                    pc_increment = self
                        .exec_sub_imm3(rd, rn, imm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::SubImm8 { rd, imm } => {
                    pc_increment = self
                        .exec_sub_imm8(rd, imm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::AddSp { imm } => {
                    pc_increment = self.exec_add_sp(imm)?.apply(pc_increment);
                }
                Instruction::SubSp { imm } => {
                    pc_increment = self.exec_sub_sp(imm)?.apply(pc_increment);
                }
                Instruction::Uxtb { rd, rm } => {
                    pc_increment = self.exec_uxtb(rd, rm)?.apply(pc_increment);
                }
                Instruction::Uxth { rd, rm } => {
                    pc_increment = self.exec_uxth(rd, rm)?.apply(pc_increment);
                }
                Instruction::Sxtb { rd, rm } => {
                    pc_increment = self.exec_sxtb(rd, rm)?.apply(pc_increment);
                }
                Instruction::Sxth { rd, rm } => {
                    pc_increment = self.exec_sxth(rd, rm)?.apply(pc_increment);
                }
                Instruction::ExtendW {
                    rd,
                    rn,
                    rm,
                    rotate,
                    op,
                } => {
                    pc_increment = self
                        .exec_extend_w(rd, rn, rm, rotate, op)?
                        .apply(pc_increment);
                }
                Instruction::It { cond, mask } => {
                    self.it_state = (cond << 4) | mask;
                    it_block_instruction = false; // The IT instruction itself doesn't count towards the block's instructions
                }
                Instruction::AddRegHigh { rd, rm } => {
                    pc_increment = self.exec_add_reg_high(rd, rm)?.apply(pc_increment);
                }
                Instruction::CmpImm { rn, imm } => {
                    pc_increment = self.exec_cmp_imm(rn, imm)?.apply(pc_increment);
                }
                Instruction::CmpReg { rn, rm } => {
                    pc_increment = self.exec_cmp_reg(rn, rm)?.apply(pc_increment);
                }
                Instruction::MovReg { rd, rm } => {
                    pc_increment = self.exec_mov_reg(rd, rm)?.apply(pc_increment);
                }
                Instruction::And { rd, rm } => {
                    pc_increment = self
                        .exec_and(rd, rm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::Bic { rd, rm } => {
                    pc_increment = self
                        .exec_bic(rd, rm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::Orr { rd, rm } => {
                    pc_increment = self
                        .exec_orr(rd, rm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::Eor { rd, rm } => {
                    pc_increment = self
                        .exec_eor(rd, rm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::Mvn { rd, rm } => {
                    pc_increment = self
                        .exec_mvn(rd, rm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::Mul { rd, rn } => {
                    pc_increment = self
                        .exec_mul(rd, rn, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::Mul32 { rd, rn, rm } => {
                    pc_increment = self.exec_mul32(rd, rn, rm)?.apply(pc_increment);
                }
                Instruction::Cpsie { primask, faultmask } => {
                    pc_increment = self.exec_cpsie(primask, faultmask)?.apply(pc_increment);
                }
                Instruction::Cpsid { primask, faultmask } => {
                    pc_increment = self.exec_cpsid(primask, faultmask)?.apply(pc_increment);
                }
                Instruction::Lsl { rd, rm, imm } => {
                    pc_increment = self
                        .exec_lsl(rd, rm, imm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::Lsr { rd, rm, imm } => {
                    pc_increment = self
                        .exec_lsr(rd, rm, imm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::Asr { rd, rm, imm } => {
                    pc_increment = self
                        .exec_asr(rd, rm, imm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::LslReg { rd, rm } => {
                    pc_increment = self
                        .exec_lsl_reg(rd, rm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::LsrReg { rd, rm } => {
                    pc_increment = self
                        .exec_lsr_reg(rd, rm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::AsrReg { rd, rm } => {
                    pc_increment = self
                        .exec_asr_reg(rd, rm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::Adc { rd, rm } => {
                    pc_increment = self
                        .exec_adc(rd, rm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::Sbc { rd, rm } => {
                    pc_increment = self
                        .exec_sbc(rd, rm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::Ror { rd, rm } => {
                    pc_increment = self
                        .exec_ror(rd, rm, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::Rev { rd, rm } => {
                    pc_increment = self.exec_rev(rd, rm)?.apply(pc_increment);
                }
                Instruction::Rev16 { rd, rm } => {
                    pc_increment = self.exec_rev16(rd, rm)?.apply(pc_increment);
                }
                Instruction::RevSh { rd, rm } => {
                    pc_increment = self.exec_rev_sh(rd, rm)?.apply(pc_increment);
                }
                Instruction::Tst { rn, rm } => {
                    pc_increment = self.exec_tst(rn, rm)?.apply(pc_increment);
                }
                Instruction::Cmn { rn, rm } => {
                    pc_increment = self.exec_cmn(rn, rm)?.apply(pc_increment);
                }
                Instruction::Rsbs { rd, rn } => {
                    pc_increment = self
                        .exec_rsbs(rd, rn, it_block_instruction)?
                        .apply(pc_increment);
                }
                Instruction::LdrImm { rt, rn, imm } => {
                    pc_increment = self.exec_ldr_imm(bus, rt, rn, imm)?.apply(pc_increment);
                }
                Instruction::StrImm { rt, rn, imm } => {
                    pc_increment = self.exec_str_imm(bus, rt, rn, imm)?.apply(pc_increment);
                }
                Instruction::LdrReg { rt, rn, rm } => {
                    pc_increment = self.exec_ldr_reg(bus, rt, rn, rm)?.apply(pc_increment);
                }
                Instruction::StrReg { rt, rn, rm } => {
                    pc_increment = self.exec_str_reg(bus, rt, rn, rm)?.apply(pc_increment);
                }
                Instruction::LdrLit { rt, imm } => {
                    pc_increment = self.exec_ldr_lit(bus, rt, imm)?.apply(pc_increment);
                }
                Instruction::LdrSp { rt, imm } => {
                    pc_increment = self.exec_ldr_sp(bus, rt, imm)?.apply(pc_increment);
                }
                Instruction::StrSp { rt, imm } => {
                    pc_increment = self.exec_str_sp(bus, rt, imm)?.apply(pc_increment);
                }
                Instruction::AddSpReg { rd, imm } => {
                    pc_increment = self.exec_add_sp_reg(rd, imm)?.apply(pc_increment);
                }
                Instruction::Adr { rd, imm } => {
                    pc_increment = self.exec_adr(rd, imm)?.apply(pc_increment);
                }
                Instruction::AddwImm { rd, rn, imm } => {
                    pc_increment = self.exec_addw_imm(rd, rn, imm)?.apply(pc_increment);
                }
                Instruction::SubwImm { rd, rn, imm } => {
                    pc_increment = self.exec_subw_imm(rd, rn, imm)?.apply(pc_increment);
                }
                Instruction::LdrbImm { rt, rn, imm } => {
                    pc_increment = self.exec_ldrb_imm(bus, rt, rn, imm)?.apply(pc_increment);
                }
                Instruction::LdrbReg { rt, rn, rm } => {
                    pc_increment = self.exec_ldrb_reg(bus, rt, rn, rm)?.apply(pc_increment);
                }
                Instruction::StrbReg { rt, rn, rm } => {
                    pc_increment = self.exec_strb_reg(bus, rt, rn, rm)?.apply(pc_increment);
                }
                Instruction::LdrsbReg { rt, rn, rm } => {
                    pc_increment = self.exec_ldrsb_reg(bus, rt, rn, rm)?.apply(pc_increment);
                }
                Instruction::LdrhReg { rt, rn, rm } => {
                    pc_increment = self.exec_ldrh_reg(bus, rt, rn, rm)?.apply(pc_increment);
                }
                Instruction::StrhReg { rt, rn, rm } => {
                    pc_increment = self.exec_strh_reg(bus, rt, rn, rm)?.apply(pc_increment);
                }
                Instruction::LdrshReg { rt, rn, rm } => {
                    pc_increment = self.exec_ldrsh_reg(bus, rt, rn, rm)?.apply(pc_increment);
                }
                Instruction::StrbImm { rt, rn, imm } => {
                    pc_increment = self.exec_strb_imm(bus, rt, rn, imm)?.apply(pc_increment);
                }
                Instruction::LdrhImm { rt, rn, imm } => {
                    pc_increment = self.exec_ldrh_imm(bus, rt, rn, imm)?.apply(pc_increment);
                }
                Instruction::StrhImm { rt, rn, imm } => {
                    pc_increment = self.exec_strh_imm(bus, rt, rn, imm)?.apply(pc_increment);
                }
                Instruction::Bkpt { imm8 } => {
                    pc_increment = self.exec_bkpt(imm8)?.apply(pc_increment);
                }
                Instruction::Svc { .. } => {
                    pc_increment = self.exec_svc()?.apply(pc_increment);
                }
                Instruction::Push { registers, m } => {
                    pc_increment = self.exec_push(bus, registers, m)?.apply(pc_increment);
                }
                Instruction::Pop { registers, p } => {
                    pc_increment = self.exec_pop(bus, registers, p)?.apply(pc_increment);
                }
                Instruction::Ldm { rn, registers } => {
                    pc_increment = self.exec_ldm(bus, rn, registers)?.apply(pc_increment);
                }
                Instruction::Stm { rn, registers } => {
                    pc_increment = self.exec_stm(bus, rn, registers)?.apply(pc_increment);
                }
                Instruction::StmdbW {
                    rn,
                    reg_list,
                    writeback,
                } => {
                    pc_increment = self
                        .exec_stmdb_w(bus, rn, reg_list, writeback)?
                        .apply(pc_increment);
                }
                Instruction::StmiaW {
                    rn,
                    reg_list,
                    writeback,
                } => {
                    pc_increment = self
                        .exec_stmia_w(bus, rn, reg_list, writeback)?
                        .apply(pc_increment);
                }
                Instruction::LdmdbW {
                    rn,
                    reg_list,
                    writeback,
                } => {
                    pc_increment = self
                        .exec_ldmdb_w(bus, rn, reg_list, writeback)?
                        .apply(pc_increment);
                }
                Instruction::LdmiaW {
                    rn,
                    reg_list,
                    writeback,
                } => {
                    pc_increment = self
                        .exec_ldmia_w(bus, rn, reg_list, writeback)?
                        .apply(pc_increment);
                }
                Instruction::Bl { offset } => {
                    pc_increment = self.exec_bl(offset)?.apply(pc_increment);
                }
                Instruction::BranchCond { cond, offset } => {
                    pc_increment = self.exec_branch_cond(cond, offset)?.apply(pc_increment);
                }
                Instruction::Bx { rm } => {
                    pc_increment = self.exec_bx(bus, rm)?.apply(pc_increment);
                }
                Instruction::BlxReg { rm } => {
                    pc_increment = self.exec_blx_reg(bus, rm)?.apply(pc_increment);
                }
                Instruction::Barrier => {
                    pc_increment = self.exec_barrier()?.apply(pc_increment);
                }
                Instruction::Mrs { rd, sysm } => {
                    pc_increment = self.exec_mrs(rd, sysm)?.apply(pc_increment);
                }
                Instruction::Msr { sysm, rn } => {
                    pc_increment = self.exec_msr(sysm, rn)?.apply(pc_increment);
                }
                Instruction::Smull {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    pc_increment = self.exec_smull(rd_lo, rd_hi, rn, rm)?.apply(pc_increment);
                }
                Instruction::Umull {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    pc_increment = self.exec_umull(rd_lo, rd_hi, rn, rm)?.apply(pc_increment);
                }
                Instruction::Smlal {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    pc_increment = self.exec_smlal(rd_lo, rd_hi, rn, rm)?.apply(pc_increment);
                }
                Instruction::Umlal {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    pc_increment = self.exec_umlal(rd_lo, rd_hi, rn, rm)?.apply(pc_increment);
                }
                Instruction::Umaal {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                } => {
                    pc_increment = self.exec_umaal(rd_lo, rd_hi, rn, rm)?.apply(pc_increment);
                }
                Instruction::Mla { rd, rn, rm, ra } => {
                    pc_increment = self.exec_mla(rd, rn, rm, ra)?.apply(pc_increment);
                }
                Instruction::Mls { rd, rn, rm, ra } => {
                    pc_increment = self.exec_mls(rd, rn, rm, ra)?.apply(pc_increment);
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
                    pc_increment = self
                        .exec_smla_xy(rd, rn, rm, ra, n_top, m_top, accumulate)?
                        .apply(pc_increment);
                }
                Instruction::Vldr { sd, rn, imm, add } => {
                    pc_increment = self.exec_vldr(bus, sd, rn, imm, add)?.apply(pc_increment);
                }
                Instruction::Vstr { sd, rn, imm, add } => {
                    pc_increment = self.exec_vstr(bus, sd, rn, imm, add)?.apply(pc_increment);
                }
                Instruction::VmulF32 { sd, sn, sm } => {
                    pc_increment = self.exec_vmul_f32(sd, sn, sm)?.apply(pc_increment);
                }
                Instruction::VaddF32 { sd, sn, sm } => {
                    pc_increment = self.exec_vadd_f32(sd, sn, sm)?.apply(pc_increment);
                }
                Instruction::VsubF32 { sd, sn, sm } => {
                    pc_increment = self.exec_vsub_f32(sd, sn, sm)?.apply(pc_increment);
                }
                Instruction::VdivF32 { sd, sn, sm } => {
                    pc_increment = self.exec_vdiv_f32(sd, sn, sm)?.apply(pc_increment);
                }
                Instruction::VfmaF32 { sd, sn, sm } => {
                    pc_increment = self.exec_vfma_f32(sd, sn, sm)?.apply(pc_increment);
                }
                Instruction::VfmsF32 { sd, sn, sm } => {
                    pc_increment = self.exec_vfms_f32(sd, sn, sm)?.apply(pc_increment);
                }
                Instruction::VfnmaF32 { sd, sn, sm } => {
                    pc_increment = self.exec_vfnma_f32(sd, sn, sm)?.apply(pc_increment);
                }
                Instruction::VfnmsF32 { sd, sn, sm } => {
                    pc_increment = self.exec_vfnms_f32(sd, sn, sm)?.apply(pc_increment);
                }
                Instruction::VmovSnRt { sn, rt } => {
                    pc_increment = self.exec_vmov_sn_rt(sn, rt)?.apply(pc_increment);
                }
                Instruction::VmovRtSn { rt, sn } => {
                    pc_increment = self.exec_vmov_rt_sn(rt, sn)?.apply(pc_increment);
                }
                Instruction::VmovF32Reg { sd, sm } => {
                    pc_increment = self.exec_vmov_f32_reg(sd, sm)?.apply(pc_increment);
                }
                Instruction::VmovF32Imm { sd, imm_bits } => {
                    pc_increment = self.exec_vmov_f32_imm(sd, imm_bits)?.apply(pc_increment);
                }
                Instruction::VcvtF32FromInt {
                    sd,
                    sm,
                    signed,
                    fbits,
                } => {
                    pc_increment = self
                        .exec_vcvt_f32_from_int(sd, sm, signed, fbits)?
                        .apply(pc_increment);
                }
                Instruction::VcvtIntFromF32 {
                    sd,
                    sm,
                    signed,
                    fbits,
                } => {
                    pc_increment = self
                        .exec_vcvt_int_from_f32(sd, sm, signed, fbits)?
                        .apply(pc_increment);
                }
                Instruction::VfpStoreMultiple {
                    rn,
                    s_first,
                    count,
                    add,
                    wback,
                } => {
                    pc_increment = self
                        .exec_vfp_store_multiple(bus, rn, s_first, count, add, wback)?
                        .apply(pc_increment);
                }
                Instruction::VfpLoadMultiple {
                    rn,
                    s_first,
                    count,
                    add,
                    wback,
                } => {
                    pc_increment = self
                        .exec_vfp_load_multiple(bus, rn, s_first, count, add, wback)?
                        .apply(pc_increment);
                }
                Instruction::Vldr64 { dd, rn, imm, add } => {
                    pc_increment = self.exec_vldr64(bus, dd, rn, imm, add)?.apply(pc_increment);
                }
                Instruction::Vstr64 { dd, rn, imm, add } => {
                    pc_increment = self.exec_vstr64(bus, dd, rn, imm, add)?.apply(pc_increment);
                }
                Instruction::VmovF64Reg { dd, dm } => {
                    pc_increment = self.exec_vmov_f64_reg(dd, dm)?.apply(pc_increment);
                }
                Instruction::VmovDRtRt2 { dm, rt, rt2 } => {
                    pc_increment = self.exec_vmov_d_rt_rt2(dm, rt, rt2)?.apply(pc_increment);
                }
                Instruction::VmovRtRt2D { rt, rt2, dm } => {
                    pc_increment = self.exec_vmov_rt_rt2_d(rt, rt2, dm)?.apply(pc_increment);
                }
                Instruction::VaddF64 { dd, dn, dm } => {
                    pc_increment = self.exec_vadd_f64(dd, dn, dm)?.apply(pc_increment);
                }
                Instruction::VsubF64 { dd, dn, dm } => {
                    pc_increment = self.exec_vsub_f64(dd, dn, dm)?.apply(pc_increment);
                }
                Instruction::VmulF64 { dd, dn, dm } => {
                    pc_increment = self.exec_vmul_f64(dd, dn, dm)?.apply(pc_increment);
                }
                Instruction::VdivF64 { dd, dn, dm } => {
                    pc_increment = self.exec_vdiv_f64(dd, dn, dm)?.apply(pc_increment);
                }
                Instruction::Unknown(op) => {
                    pc_increment = self.exec_unknown(op)?.apply(pc_increment);
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
