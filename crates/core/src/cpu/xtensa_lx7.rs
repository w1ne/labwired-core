// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 CPU backend. Glues AR file, PS, SR file with the fetch loop
//! and `Cpu` trait.
//!
//! D1: ALU reg-reg (ADD/SUB/AND/OR/XOR/NEG/ABS/ADDX*/SUBX*), MOVI, NOP/fences, BREAK.
//! D2: Shift exec (SLL/SRL/SRA/SRC/SLLI/SRLI/SRAI) + SAR-setup (SSL/SSR/SSAI/SSA8L/SSA8B).
//! Remaining instruction classes in progress.

use crate::cpu::xtensa_regs::{ArFile, Ps};
use crate::cpu::xtensa_sr::{
    XtensaSrFile, EPC1, EPC2, EPC3, EPC4, EPC5, EPC6, EPC7, EPS2, EPS3, EPS4, EPS5, EPS6, EPS7,
    EXCCAUSE, INTENABLE, INTERRUPT, PS as PS_SR, SAR, SCOMPARE1, VECBASE, WINDOWBASE, WINDOWSTART,
};
use crate::decoder::{xtensa, xtensa_length, xtensa_narrow};

/// Decode-cache slot count (direct-mapped by `(pc >> 1) & MASK`). Power of two.
const DECODE_CACHE_SIZE: usize = 8192;
const DECODE_CACHE_MASK: usize = DECODE_CACHE_SIZE - 1;
use crate::snapshot::{CpuSnapshot, XtensaLx7CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationError, SimulationObserver};
use std::sync::Arc;

/// Offset of _KernelExceptionVector relative to VECBASE on ESP32-S3 LX7.
///
/// Verified against Zephyr soc/xtensa/esp32s3/linker.ld:
///   `. = 0x300; KEEP(*(.KernelExceptionVector.text));`
/// and the ESP-IDF / FreeRTOS ABI which reads PS and EPC1 directly via RSR
/// for level-1 general exceptions (no EPS1 exists in the ESP32-S3 LX7 config).
const KERNEL_VECTOR_OFFSET: u32 = 0x300;

// ── Interrupt vector offsets (VECBASE-relative) ───────────────────────────────
//
// Verified against:
//   ~/.platformio/packages/toolchain-xtensa-esp32s3/xtensa-esp32s3-elf/
//     sys-include/xtensa/config/core-isa.h
// Constants: XCHAL_INTLEVEL{n}_VECOFS, XCHAL_KERNEL_VECOFS, XCHAL_NMI_VECOFS.
//
// Level 1: uses _KernelExceptionVector (XCHAL_KERNEL_VECOFS = 0x300).
//   Level-1 interrupts share the kernel exception vector; EXCCAUSE=4 (Level1Interrupt)
//   distinguishes them from synchronous exceptions in the handler.
// Level 2: XCHAL_INTLEVEL2_VECOFS = 0x180
// Level 3: XCHAL_INTLEVEL3_VECOFS = 0x1C0
// Level 4: XCHAL_INTLEVEL4_VECOFS = 0x200
// Level 5: XCHAL_INTLEVEL5_VECOFS = 0x240
// Level 6: XCHAL_INTLEVEL6_VECOFS = 0x280  (also Debug vector)
// Level 7: XCHAL_NMI_VECOFS       = 0x2C0  (NMI)
const IRQ_VECTOR_OFFSETS: [u32; 8] = [
    0x000, // level 0: unused (placeholder)
    0x300, // level 1: XCHAL_KERNEL_VECOFS
    0x180, // level 2: XCHAL_INTLEVEL2_VECOFS
    0x1C0, // level 3: XCHAL_INTLEVEL3_VECOFS
    0x200, // level 4: XCHAL_INTLEVEL4_VECOFS
    0x240, // level 5: XCHAL_INTLEVEL5_VECOFS
    0x280, // level 6: XCHAL_INTLEVEL6_VECOFS (Debug)
    0x2C0, // level 7: XCHAL_NMI_VECOFS
];

// ── IRQ priority table ────────────────────────────────────────────────────────
//
// Fixed interrupt priority levels for the 32 CPU interrupt bits on ESP32-S3 LX7.
//
// Verified against:
//   ~/.platformio/packages/toolchain-xtensa-esp32s3/xtensa-esp32s3-elf/
//     sys-include/xtensa/config/core-isa.h
// Constants: XCHAL_INT{n}_LEVEL for n = 0..31.
//
// Bits 0-10: level 1; bit 11: level 3; bit 12-13: level 1; bit 14: level 7 (NMI);
// bit 15: level 3; bit 16: level 5; bits 17-18: level 1; bits 19-21: level 2;
// bits 22-23: level 3; bit 24: level 4; bit 25: level 4; bit 26: level 5;
// bit 27: level 3; bit 28: level 4; bit 29: level 3; bit 30: level 4; bit 31: level 5.
//
// XCHAL_EXCM_LEVEL = 3: PS.EXCM masks interrupt delivery for levels 1..3.
// Levels 4..7 are "high-priority" and are NOT blocked by EXCM.
pub const IRQ_LEVELS: [u8; 32] = [
    1, 1, 1, 1, 1, 1, 1, 1, // 0-7
    1, 1, 1, 3, 1, 1, 7, 3, // 8-15
    5, 1, 1, 2, 2, 2, 3, 3, // 16-23
    4, 4, 5, 3, 4, 3, 4, 5, // 24-31
];

/// EXCCAUSE value for Level-1 interrupt entry (ISA RM §4.4.1.5).
const EXCCAUSE_LEVEL1_INTERRUPT: u8 = 4;

/// XCHAL_EXCM_LEVEL: PS.EXCM blocks delivery of interrupts at levels <= this.
/// Verified from core-isa.h: XCHAL_EXCM_LEVEL = 3.
#[allow(dead_code, reason = "reserved for level-gated interrupt arbitration")]
const EXCM_LEVEL: u8 = 3;

/// Round an `f32` to the nearest integer, ties to even (IEEE-754
/// round-to-nearest-even). Rust's `f32::round` rounds half *away* from zero,
/// so we implement banker's rounding explicitly to match `round.s`, which uses
/// the FPU's default rounding mode.
#[inline]
fn round_half_even(v: f32) -> f32 {
    let floor = v.floor();
    let diff = v - floor;
    if diff < 0.5 {
        floor
    } else if diff > 0.5 {
        floor + 1.0
    } else if (floor as i64) % 2 == 0 {
        floor
    } else {
        floor + 1.0
    }
}

/// Per-TCB parked hybrid CALL preserve panes (shadow-window mode).
type TaskPreserveMap = std::collections::HashMap<u32, Vec<Vec<(u8, [u32; 4])>>>;

pub struct XtensaLx7 {
    pub regs: ArFile,
    pub ps: Ps,
    pub sr: XtensaSrFile,
    /// User-Register file (URs accessed via RUR/WUR). 256 entries; the
    /// commonly-used IDs are THREADPTR (231), FCR (232), FSR (233). FCR holds
    /// the FP rounding mode + exception enables; FSR holds the sticky FP
    /// exception flags. We default FCR to round-to-nearest and treat both as
    /// plain storage — the executor never traps on FP exceptions (the GCC
    /// soft-float-free path expected here never enables them).
    pub ur: [u32; 256],
    /// Single-precision FP register file f0..f15, stored as raw u32 bit
    /// patterns so NaN payloads and signed zeros round-trip losslessly through
    /// rfr/wfr and lsi/ssi. The Xtensa LX7 FPU is single-precision only.
    pub fp: [u32; 16],
    /// Boolean registers b0..b15 (Boolean Option), packed one per bit. FP
    /// compares (oeq.s/olt.s/…) write a result bit here; movf.s/movt.s read it.
    /// Modeled minimally: only the FP compare/move instructions touch it (the
    /// integer BR-consuming branches aren't in the decoded set yet).
    pub br: u16,
    pub pc: u32,
    /// Set by the branch helper when a conditional branch's predicate
    /// fires. Read by the ZOL post-step check to distinguish a branch
    /// that happened to land at LEND (exit, no loop-back) from natural
    /// fall-through to LEND from the last body instruction (loop-back).
    /// Cleared at the start of every step.
    pub branched: bool,
    /// When true, `Cpu::step` is a no-op for this instance — used by
    /// dual-core configs to keep APP_CPU parked until PRO_CPU writes the
    /// app-cpu boot address (mirrors real silicon's reset-hold). Default
    /// false; ESP32 dual-core setup sets it on cpu_secondary at boot.
    pub halted: bool,
    /// Set when the last retired instruction was `WAITI` (PC does not
    /// advance). Subsequent steps take a cheap CCOUNT/IRQ path until a
    /// wake-capable interrupt arrives — important for dual-core APP_CPU
    /// FreeRTOS idle (`vApplicationIdleHook` / idle task Waiti loop).
    waiti_parked: bool,
    /// Faithful windowed-register mode. When true, the CPU uses the REAL
    /// Xtensa window machinery — per-access window-overflow checks that vector
    /// to the firmware's OF{4,8,12} handlers (which spill to the stack save
    /// chain), and RETW underflow to the UF handlers — with NO sim-level
    /// "shadow" spill stack. Requires firmware that installs the window vectors
    /// and builds a proper save chain (i.e. --rom-boot from a real reset).
    /// Default false: fast-boot jumps mid-execution without a primed chain, so
    /// it relies on the shadow mechanism instead.
    pub faithful_windows: bool,
    /// True for the ESP32 APP_CPU (core 1). On real silicon a core's PRID
    /// is fixed hardware (PRO_CPU 0xCDCD → core 0, APP_CPU 0xABAB → core 1)
    /// and survives reset. We remember it so `reset()` restores the right
    /// PRID — otherwise a reset re-news the SR to 0xCDCD, APP_CPU reads
    /// core_id 0, and it corrupts PRO_CPU's per-core FreeRTOS state.
    /// (SMP core identity itself is derived from PRID via `core_id()`.)
    pub app_cpu: bool,
    /// Authoritative per-CALL preserve snapshots (slot, a0..a3). RETW restores
    /// these exactly so outer a4..a7 survive deep wrap; per-slot LIFO remains
    /// for displace/WS only (classic early-boot behavior). NOT mixed into the
    /// per-slot shadow LIFO — that mixing caused steal of outer CALL8 a5 after
    /// the 16-slot window wraps (`esp_intr_alloc` fault).
    pub(crate) call_preserve_stack: Vec<Vec<(u8, [u32; 4])>>,
    /// Windowed register state saved across interrupt entry (dispatch_irq)
    /// and restored on RFE/RFI. Real silicon does not auto-save WB/ARs, but
    /// FreeRTOS Xtensa ports assume balanced windowed CALL/RETW in ISRs; any
    /// imbalance (or our shadow/call_preserve divergence) otherwise returns
    /// to the interrupted frame with the wrong WindowBase (seen: ExitCritical
    /// WB=13 → after timer ISR WB≠13 → NotifyTake a6=&xKernelLock becomes 1).
    irq_window_stack: Vec<IrqWindowFrame>,
    /// Per-TCB hybrid preserve parked when a task is switched out (shadow mode).
    /// Keyed by FreeRTOS TCB address from `pxCurrentTCBs[core]`. Restored on
    /// RFE when that TCB is current again so CALL8 a4..a7 survive NotifyTake
    /// block/wake (ipc_task a5/a6 → flash IPC).
    task_preserve_by_tcb: TaskPreserveMap,
    /// After a windowed ROM thunk returns (CALL+no ENTRY), the caller still
    /// needs to RETW. If we deliver an IRQ in that gap (e.g. ExitCritical
    /// just restored INTLEVEL via `_xtos_set_intlevel`), the ISR can leave
    /// WindowBase wrong. Defer IRQs until the next RETW closes the windowed
    /// call. Cleared on RETW / reset.
    defer_irq_until_retw: bool,
    /// IRAM/flash instruction-fetch slice cache (#119 Phase 1.2).
    fetch_cache: Option<(u64, u64, usize)>,
    /// Decode cache (#124 follow-on): direct-mapped PC → (tag, len, decoded
    /// `Instruction`). The fetch_cache removes the bus dispatch but every
    /// instruction is still re-decoded; the demos' tight busy-loops decode the
    /// same handful of PCs millions of times. Each slot carries the
    /// `cur_decode_gen` it was filled under; any fetch-cache invalidation
    /// (write into a cached code range, IRQ dispatch, snapshot restore) bumps
    /// the generation, lazily voiding every entry without a clear pass.
    decode_cache: Vec<Option<(u32, u32, crate::decoder::xtensa::Instruction)>>,
    decode_gen: Vec<u32>,
    cur_decode_gen: u32,
    /// JIT cache (Phase 3.2 pilot — issue #124).
    #[cfg(feature = "jit")]
    pub jit: Option<Box<crate::cpu::xtensa_jit::JitCache>>,
    /// Runtime knob to fully disable the JIT fast-path even when the
    /// `jit` feature is compiled in. Lets the lockstep harness force the
    /// pure-interpreter pass to actually be pure-interpreter, while the
    /// JIT pass shares the same binary. Default `true` (JIT enabled).
    #[cfg(feature = "jit")]
    pub jit_enabled: bool,
}

/// Window control saved at interrupt entry (see `irq_window_stack`).
#[derive(Clone, Debug)]
struct IrqWindowFrame {
    /// Full preserve stack of the interrupted task (pre-spill snapshot).
    /// ISR CALL/RETW can pop live preserve; on same-task RFE we restore this
    /// so RETW can use hybrid preserve for a4..a7 without relying solely on UF.
    call_preserve_stack: Vec<Vec<(u8, [u32; 4])>>,
    /// Logical a1 (SP) at interrupt entry. FreeRTOS may switch tasks inside
    /// the ISR (`_frxt_dispatch`); on RFE the SP then belongs to a different
    /// task and restoring this frame's preserve would graft the old task's
    /// window chain onto the new one (seen: ipc1 running on IDLE1's stack).
    sp_at_entry: u32,
    /// WindowStart before IRQ-entry spill. Spill sets WS=1<<WB so a later
    /// task-switch RETW underflows into the *new* stack; same-task RFE must
    /// put the original WS back or CALL8 a4..a7 (e.g. ipc_task's a5/a6
    /// pointer locals) are lost after NotifyTake returns.
    windowstart_at_entry: u16,
}

impl XtensaLx7 {
    pub fn new() -> Self {
        Self {
            regs: ArFile::new(),
            // HW-verified: PS reset = 0x1F (EXCM=1, INTLEVEL=0xF).
            // Confirmed via OpenOCD `reset halt` on real S3-Zero: ps = 0x0000001f.
            ps: Ps::from_raw(0x1F),
            sr: XtensaSrFile::new(),
            ur: [0u32; 256],
            fp: [0u32; 16],
            br: 0,
            pc: 0x4000_0400,
            branched: false,
            halted: false,
            waiti_parked: false,
            faithful_windows: false,
            app_cpu: false,
            call_preserve_stack: Vec::new(),
            irq_window_stack: Vec::new(),
            task_preserve_by_tcb: std::collections::HashMap::new(),
            defer_irq_until_retw: false,
            fetch_cache: None,
            decode_cache: vec![None; DECODE_CACHE_SIZE],
            decode_gen: vec![0; DECODE_CACHE_SIZE],
            cur_decode_gen: 1,
            #[cfg(feature = "jit")]
            jit: None,
            #[cfg(feature = "jit")]
            jit_enabled: true,
        }
    }

    /// Read FP register `f` as an `f32` (decoded from its stored bit pattern).
    #[inline]
    fn fget(&self, f: u8) -> f32 {
        f32::from_bits(self.fp[(f & 0xF) as usize])
    }

    /// Write an `f32` into FP register `f`, storing its raw bit pattern.
    #[inline]
    fn fset(&mut self, f: u8, v: f32) {
        self.fp[(f & 0xF) as usize] = v.to_bits();
    }

    /// Drop the IRAM/flash fetch slice cache. Call when the cached
    /// peripheral's contents may have changed (bus write into the
    /// cached range, runtime snapshot restore, IRQ dispatch).
    #[inline]
    fn invalidate_fetch_cache(&mut self) {
        self.fetch_cache = None;
        // Decode cache shares the fetch cache's invalidation conditions: bump
        // the generation so every cached decode is lazily voided (no clear).
        self.cur_decode_gen = self.cur_decode_gen.wrapping_add(1);
    }

    /// Invalidate the fetch cache iff `addr` (the start byte of an
    /// in-flight bus write) falls inside the cached range. Conservative:
    /// we don't try to be clever about write size — any write into the
    /// cached PC range drops the cache.
    #[inline]
    fn maybe_invalidate_for_write(&mut self, addr: u32) {
        if let Some((start, end, _)) = self.fetch_cache {
            let a = addr as u64;
            if a >= start && a < end {
                self.fetch_cache = None;
            }
        }
        // Decode-cache SMC safety: a store into Xtensa instruction space
        // (>= 0x4000_0000 — IRAM/flash/ROM; DRAM lives below) may rewrite code
        // we've cached the *decoded* form of. The fetch_cache reads live bytes
        // so it's self-healing, but the decode cache isn't — so any code-space
        // write voids the whole decode cache. Data stores (DRAM) skip this, so
        // the cache doesn't thrash on the common path.
        if addr >= 0x4000_0000 {
            self.cur_decode_gen = self.cur_decode_gen.wrapping_add(1);
        }
    }

    /// Construct an APP_CPU instance: PRID reads as 0xABAB (so
    /// `xPortGetCoreID()` returns 1) and the CPU starts halted —
    /// waiting for PRO_CPU to release it via `ets_set_appcpu_boot_addr`.
    pub fn new_app_cpu() -> Self {
        let mut cpu = Self::new();
        cpu.sr = XtensaSrFile::new_app_cpu();
        cpu.halted = true;
        // PRID was set to 0xABAB by new_app_cpu() above, so core_id() == 1.
        // app_cpu is what reset() reads to restore that PRID across a reset.
        cpu.app_cpu = true;
        cpu
    }

    /// Phase 3.2 pilot (issue #124): attempt to dispatch the current PC to
    /// a JIT-compiled block. Returns `Ok(Some(instr_count))` if the JIT
    /// handled the step (with PC, registers, and CCOUNT already updated),
    /// `Ok(None)` if the PC isn't JIT-compilable and the caller should
    /// proceed with the interpreter, or `Err` if the JIT entered a state
    /// that should propagate as a sim error.
    ///
    /// The caller has already bumped CCOUNT by 1 for this step (see the
    /// pre-fetch block in `step`). When the JIT executes N>1 instructions
    /// we add the remaining `N - 1` cycles here to keep CCOUNT honest.
    #[cfg(feature = "jit")]
    fn try_jit_step(&mut self, bus: &mut dyn Bus) -> SimResult<Option<u32>> {
        use crate::cpu::xtensa_jit::{
            JitCache, FILL_SCREEN_BLOCK_END, FILL_SCREEN_BLOCK_INSTR_COUNT, FILL_SCREEN_BLOCK_PC,
            HOT_BB_PC, LOOPV_CALL8_PC,
        };
        let pc = self.pc;
        // Hot guard: cheap exact-PC check for any of the JIT'd PCs before
        // we even touch the cache. Avoids paying the Option<Box> deref +
        // HashMap lookup on every single interpreter step.
        if pc == LOOPV_CALL8_PC {
            return self.try_jit_windowed_call(bus);
        }
        if pc == HOT_BB_PC {
            return self.try_jit_multi_op(bus);
        }
        if pc != FILL_SCREEN_BLOCK_PC {
            return Ok(None);
        }
        // Lazy-init the cache the first time we see a JIT-able PC.
        if self.jit.is_none() {
            self.jit = Some(Box::new(JitCache::new()));
        }
        let cache = self.jit.as_mut().expect("jit cache init above");
        let block = match cache.lookup_or_install(pc) {
            Some(b) => b,
            None => return Ok(None),
        };

        // Marshal register state. The fillScreen block uses a2/a3/a8/a9/a10
        // and writes back a2/a3/a11 plus zero–two bytes of memory.
        let a2 = self.regs.read_logical(2);
        let a3 = self.regs.read_logical(3);
        let a8 = self.regs.read_logical(8);
        let a9 = self.regs.read_logical(9);
        let a10 = self.regs.read_logical(10);

        let (exit, a2_n, a3_n, a11_n, pending) = block
            .run(a8, a9, a2, a3, a10)
            .map_err(|e| SimulationError::NotImplemented(format!("xtensa JIT call: {e:#}")))?;

        match exit {
            0 | 1 => {
                // Fall-through OR branch-taken: commit stores, write back regs.
                for (addr, val) in pending {
                    bus.write_u8(addr as u64, val)?;
                }
                self.regs.write_logical(2, a2_n);
                self.regs.write_logical(3, a3_n);
                self.regs.write_logical(11, a11_n);
                self.pc = if exit == 0 {
                    FILL_SCREEN_BLOCK_END
                } else {
                    FILL_SCREEN_BLOCK_PC
                };
                // We executed `FILL_SCREEN_BLOCK_INSTR_COUNT` Xtensa
                // instructions; the outer `step` already counted one, so
                // bump CCOUNT by the remainder. The CCOMPARE0 edge check
                // re-runs implicitly via the same logic the next time
                // `step` is entered (we don't repeat the check here —
                // CCOUNT advancing across a compare value will fire the
                // timer interrupt on the very next outer step's pre-fetch).
                if FILL_SCREEN_BLOCK_INSTR_COUNT > 1 {
                    use crate::cpu::xtensa_sr::CCOUNT;
                    let cc = self.sr.read(CCOUNT);
                    self.sr
                        .write(CCOUNT, cc.wrapping_add(FILL_SCREEN_BLOCK_INSTR_COUNT - 1));
                }
                self.branched = exit == 1;
                Ok(Some(FILL_SCREEN_BLOCK_INSTR_COUNT))
            }
            3 => {
                // Out-of-range bus address: leave state untouched and let
                // the interpreter retry the block from the top so the real
                // bus error surfaces with full fidelity.
                Ok(None)
            }
            other => Err(SimulationError::NotImplemented(format!(
                "xtensa JIT returned unknown side-exit code {other}"
            ))),
        }
    }

    /// Phase 3.6.2 (issue #124): JIT'd windowed CALL8 dispatch for the
    /// `_Z4loopv` call at PC 0x400d4a99 — the dominant hot block (~92%
    /// of all dispatches per Phase 3.0 data).
    ///
    /// # Semantics (must match the [`xtensa::Instruction::Call8`] arm
    /// of [`Self::execute`] byte-for-byte)
    ///
    /// 1. `ret_pc = ((PC + 3) & 0x3FFFFFFF) | (CALLINC << 30)`  (CALLINC=2)
    /// 2. `target = ((PC + 4) & ~3) + offset`
    /// 3. `spill_shadow_on_call(2)` — preserve caller's a0..a7 across the
    ///    upcoming window rotation, and conditionally shadow-save the
    ///    callee's a0..a3 slot if it's already live.
    /// 4. `write_logical(8, ret_pc)` — store return address in caller's a8
    ///    (becomes callee's a0 after ENTRY's rotation).
    /// 5. `PS.CALLINC := 2`
    /// 6. `PC := target`
    ///
    /// Steps 1+2+6 are computed inside the wasm module (all constants for
    /// the LOOPV PC) and returned as `(ret_pc_encoded, target_pc, callinc)`.
    /// Steps 3+4+5 happen here on the Rust side — replicating them inside
    /// wasm would require exposing the full 64-entry AR file plus per-slot
    /// shadow stacks to the wasm sandbox, with no perf upside.
    ///
    /// Returns:
    /// * `Ok(Some(1))` — JIT handled the step (one Xtensa instr executed,
    ///   matching the outer step's already-incremented CCOUNT).
    /// * `Ok(None)` — JIT refused (window-overflow guard or PC drift);
    ///   caller proceeds to interpreter.
    /// * `Err` — wasmtime error or unknown exit code.
    #[cfg(feature = "jit")]
    fn try_jit_windowed_call(&mut self, _bus: &mut dyn Bus) -> SimResult<Option<u32>> {
        use crate::cpu::xtensa_jit::{
            JitCache, EXIT_WINDOWED_REFUSE, LOOPV_CALL8_INSTR_COUNT, WINDOWED_EXIT_TAKEN,
        };
        let pc = self.pc;
        if self.jit.is_none() {
            self.jit = Some(Box::new(JitCache::new()));
        }
        let cache = self.jit.as_mut().expect("jit cache init above");
        let block = match cache.lookup_or_install_windowed(pc) {
            Some(b) => b,
            None => return Ok(None),
        };

        let result = block
            .run(pc, self.regs.windowstart(), self.regs.windowbase())
            .map_err(|e| {
                SimulationError::NotImplemented(format!("xtensa JIT windowed call: {e:#}"))
            })?;

        match result.exit_code {
            x if x == WINDOWED_EXIT_TAKEN => {
                // Apply the architectural side-effects in the same order
                // as the Call8 interpreter arm.
                self.spill_shadow_on_call(result.callinc);
                self.regs.write_logical(8, result.ret_pc_encoded);
                self.ps.set_callinc(result.callinc);
                self.pc = result.target_pc;
                self.branched = true;
                // CCOUNT: one Xtensa instr executed; outer step already
                // bumped CCOUNT by 1, so no further adjustment needed.
                debug_assert_eq!(LOOPV_CALL8_INSTR_COUNT, 1);
                Ok(Some(LOOPV_CALL8_INSTR_COUNT))
            }
            x if x == EXIT_WINDOWED_REFUSE => {
                cache.windowed_refusals += 1;
                Ok(None)
            }
            other => Err(SimulationError::NotImplemented(format!(
                "xtensa windowed-call JIT returned unknown side-exit code {other}"
            ))),
        }
    }

    /// Phase 3.6.3 (issue #124): multi-op BB dispatch for the
    /// `call_start_cpu0` delay loop at PC 0x400829cc.
    ///
    /// The compiled wasm body executes 8 instructions:
    /// `or a10,a5,a5 ; memw ; l8ui a6,a3,0 ; memw ; l8ui a2,a3,1 ;
    /// extui a2,a2,0,8 ; and a2,a2,a6 ; l32r a8,...` and exits with
    /// PC at 0x400829e4 (the `callx8` terminator). The host pre-reads
    /// the two byte values (`mem8[a3+0]` and `mem8[a3+1]`) through the
    /// live `Bus` before invoking wasm, and pre-resolves the L32R
    /// literal once (then re-uses it — the literal pool is read-only).
    ///
    /// Returns:
    /// * `Ok(Some(HOT_BB_INSTR_COUNT))` — JIT handled the block;
    ///   regs + PC updated.
    /// * `Ok(None)` — JIT refused (host bus error during pre-read,
    ///   or wasm signalled bus error); caller proceeds to interpreter.
    /// * `Err` — unrecoverable wasmtime / sim error.
    #[cfg(feature = "jit")]
    fn try_jit_multi_op(&mut self, bus: &mut dyn Bus) -> SimResult<Option<u32>> {
        use crate::cpu::xtensa_jit::{
            JitCache, HOT_BB_END, HOT_BB_INSTR_COUNT, HOT_BB_L32R_ADDR, MULTI_EXIT_FALL_THROUGH,
            MULTI_EXIT_HOST_BUS_ERROR,
        };
        let pc = self.pc;
        if self.jit.is_none() {
            self.jit = Some(Box::new(JitCache::new()));
        }

        // Pre-read both L8UI bytes through the live bus. If either
        // errors we refuse the JIT path entirely so the interpreter can
        // raise the genuine fault with full context.
        let a3 = self.regs.read_logical(3);
        let a5 = self.regs.read_logical(5);
        let b0 = match bus.read_u8(a3 as u64) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        let b1 = match bus.read_u8((a3.wrapping_add(1)) as u64) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        // L32R literal: pre-resolved address is constant for this BB.
        // The literal-pool memory is immutable for our purposes; we
        // re-read it once per call (still cheap; the slow path is at
        // worst the same as the interpreter would do).
        let l32r_val = bus.read_u32(HOT_BB_L32R_ADDR as u64)?;

        let cache = self.jit.as_mut().expect("jit cache init above");
        let block = match cache.lookup_or_install_multi_op(pc) {
            Some(b) => b,
            None => return Ok(None),
        };
        block.stage_loads(&[b0, b1]);
        let result = block
            .run(a3, a5, l32r_val)
            .map_err(|e| SimulationError::NotImplemented(format!("xtensa multi-op JIT: {e:#}")))?;

        match result.exit_code {
            x if x == MULTI_EXIT_FALL_THROUGH => {
                // Commit registers in the same order the interpreter
                // would have produced them.
                self.regs.write_logical(10, result.a10);
                self.regs.write_logical(6, result.a6);
                self.regs.write_logical(2, result.a2);
                self.regs.write_logical(8, result.a8);
                self.pc = HOT_BB_END;
                // CCOUNT: outer step has already counted one; bump
                // CCOUNT by the remaining (HOT_BB_INSTR_COUNT - 1).
                if HOT_BB_INSTR_COUNT > 1 {
                    use crate::cpu::xtensa_sr::CCOUNT;
                    let cc = self.sr.read(CCOUNT);
                    self.sr
                        .write(CCOUNT, cc.wrapping_add(HOT_BB_INSTR_COUNT - 1));
                }
                self.branched = false;
                Ok(Some(HOT_BB_INSTR_COUNT))
            }
            x if x == MULTI_EXIT_HOST_BUS_ERROR => {
                if let Some(c) = self.jit.as_mut() {
                    c.multi_op_refusals += 1;
                }
                Ok(None)
            }
            other => Err(SimulationError::NotImplemented(format!(
                "xtensa multi-op JIT returned unknown side-exit code {other}"
            ))),
        }
    }

    /// Read an SR by ID, with special routing for PS / WINDOWBASE / WINDOWSTART
    /// (which live outside `XtensaSrFile`).
    fn read_sr(&self, sr_id: u16) -> u32 {
        match sr_id {
            x if x == PS_SR => self.ps.as_raw(),
            x if x == WINDOWBASE => self.regs.windowbase() as u32,
            x if x == WINDOWSTART => self.regs.windowstart() as u32,
            _ => self.sr.read(sr_id),
        }
    }

    /// Write an SR by ID, with special routing for PS / WINDOWBASE / WINDOWSTART.
    fn write_sr(&mut self, sr_id: u16, val: u32) {
        match sr_id {
            x if x == PS_SR => {
                self.ps = Ps::from_raw(val);
            }
            x if x == WINDOWBASE => {
                self.regs.set_windowbase(val as u8);
            }
            x if x == WINDOWSTART => {
                self.regs.set_windowstart(val as u16);
            }
            _ => self.sr.write(sr_id, val),
        }
    }

    /// Sim-level transparent spill on CALL{n}. Before CALL{n} writes the
    /// return address into the caller's logical a{n*4}, the AR slot we'd
    /// land on (the future callee's a0..a3) may already hold a live frame's
    /// registers — this happens when the call chain wraps around the 64-AR
    /// file (8 CALL8s, 16 CALL4s, etc). Real silicon raises WindowOverflow
    /// and runs OF8/OF12 handlers that spill to a stack save chain — but
    /// that chain isn't primed on a cold first wrap, so the canonical
    /// `l32e a0, a1, -12` reads garbage. We sidestep by pushing the
    /// displaced frame's a0..a3 to a per-WB shadow stack here, and popping
    /// it back in RETW. WS bits remain consistent: the displaced frame's
    /// WS bit stays set (its data is just temporarily shadowed); on RETW
    /// from the callee we restore those four ARs.
    ///
    /// Note: leaving WS set while physical ARs are shadowed means a firmware
    /// `xthal_window_spill` walk can see a clobbered a1 and store to
    /// `0xfffffff0`. Fast-boot paths install `xthal_window_spill_thunk` to
    /// spill from the shadow; a future WS clear/restore must not break RETW
    /// underflow checks (see FIDELITY.md Batch D).
    fn spill_shadow_on_call(&mut self, callinc: u8) {
        // Faithful mode uses the real OF/UF handlers (stack save chain), not
        // the sim shadow stack.
        if self.faithful_windows {
            return;
        }
        if callinc == 0 {
            return;
        }
        let wb_old = self.regs.windowbase();
        let wb_new = wb_old.wrapping_add(callinc) & 0x0F;

        // Authoritative preserve for caller's a0..a{callinc*4-1}. Kept ONLY on
        // call_preserve_stack — never on the per-slot LIFO — so a later RETW's
        // displace sweep cannot steal an outer CALL8's a4..a7 after wrap.
        let mut preserve = Vec::with_capacity(callinc as usize);
        for k in 0..callinc {
            let slot = wb_old.wrapping_add(k) & 0x0F;
            let base = (slot as usize) * 4;
            let regs = [
                self.regs.physical(base),
                self.regs.physical(base + 1),
                self.regs.physical(base + 2),
                self.regs.physical(base + 3),
            ];
            preserve.push((slot, regs));
        }
        self.call_preserve_stack.push(preserve);

        // Displaced live slots — classic per-slot LIFO for WS re-set on RETW.
        for k in 0..4u8 {
            let slot = wb_new.wrapping_add(k) & 0x0F;
            if self.regs.windowstart_bit(slot) {
                self.regs.push_shadow(slot);
            }
        }
    }

    /// Pop one CALL's preserve snapshot and write it into the physical AR file.
    /// Used by RETW and ROM thunks that skip RETW.
    pub(crate) fn restore_call_preserve(&mut self) {
        if let Some(preserve) = self.call_preserve_stack.pop() {
            // Place panes relative to the *current* WindowBase (caller's window
            // after RETW), not the absolute slot numbers captured at CALL time.
            // After FreeRTOS task switch WB may not match the original layout,
            // so absolute slots write a0..a7 into the wrong logical registers
            // (seen: ipc_task a5/a6 landed in a9/a10 under WB=7).
            let wb = self.regs.windowbase();
            for (i, (_slot, regs)) in preserve.iter().enumerate() {
                let slot = wb.wrapping_add(i as u8) & 0x0F;
                let base = (slot as usize) * 4;
                self.regs.set_physical(base, regs[0]);
                self.regs.set_physical(base + 1, regs[1]);
                self.regs.set_physical(base + 2, regs[2]);
                self.regs.set_physical(base + 3, regs[3]);
            }
        }
    }

    /// Spill sim-only `call_preserve_stack` frames to the task stack using the
    /// Xtensa OF/UF save layout, then set `WINDOWSTART = 1<<WB`.
    ///
    /// Real silicon has already written these save areas via WindowOverflow as
    /// the call chain grew. Shadow mode keeps outer frames only in
    /// `call_preserve_stack`, so FreeRTOS interrupt-path task switches (no
    /// `xthal_window_spill`) would lose them — and RFE would re-apply the old
    /// task's preserve onto the new task. Spilling here makes the stack the
    /// authority (ROM-accurate) before any switch can happen.
    ///
    /// Does **not** modify PC (caller handles RET for the spill thunk).
    pub(crate) fn spill_call_preserve_to_stack(&mut self, bus: &mut dyn Bus) {
        // Hybrid CALL preserve → stack OF save areas for IRQ / xthal spill.
        //
        // WindowOverflow4/8 (window_vectors.S):
        //   a0..a3 @ callee_sp - 16
        //   a4..a7 @ parent_sp - 32  (parent = call[j-1].a1)
        //
        // NVS PageManager::load free-list clobber (diag FREE-LIST SLOT CLOBBER):
        // a preserve/WS pane had a1=0x3ffc4484 (= load_sp+36, a data pointer from
        // `addi a5, a1, 36`, not a real SP). Spilling a0..a3 at a1-16 wrote
        // through [load_sp+20, +36) and replaced the free-list ptr at sp+24.
        // Real windowed SPs are always 16-byte aligned (ENTRY imm12 multiple of
        // 8 and ABI keeps a1 16B-aligned). Reject any base that is not.
        //
        // a0..a3 always go to callee OF (spill_sp-16). a4..a7 for CALL8 go to
        // parent_sp-32 **only** when parent_sp is a real, stackish SP strictly
        // above this frame (WindowOverflow8 layout). We still park preserve
        // under the FreeRTOS TCB for same-IRQ RFE, but FreeRTOS task-switch
        // restores windows from the **stack** — without a4..a7 OF, xQueueReceive
        // loses a5 (= mux = queue+84) after GiveFromISR yield and re-enters
        // EnterCritical with garbage (S3 L2 RGB/RMT fault @ 0x20406a).
        //
        // Classic ESP32 DRAM is 0x3FF8_0000..0x4000_0000; ESP32-S3 internal
        // SRAM is 0x3FC8_8000..0x3FD0_0000 (chip yaml 512 KiB / model 480 KiB).
        let dram_sp = |sp: u32| {
            (0x3FF8_0000..0x4000_0000).contains(&sp) // classic ESP32
                || (0x3FC8_8000..0x3FD0_0000).contains(&sp) // ESP32-S3 SRAM
        };
        // Windowed ABI: a1 is always 16-byte aligned after ENTRY.
        let valid_sp = |sp: u32| dram_sp(sp) && (sp & 0xF) == 0;
        let current_a1 = self.regs.read_logical(1);
        let stackish = |sp: u32, ref_sp: u32| {
            if !valid_sp(sp) || !dram_sp(ref_sp) {
                return false;
            }
            let above = sp.wrapping_sub(ref_sp);
            let below = ref_sp.wrapping_sub(sp);
            (sp >= ref_sp && above < 0x1000) || (sp < ref_sp && below < 0x100)
        };
        let write4 =
            |bus: &mut dyn Bus, base_sp: u32, off: u32, r0: u32, r1: u32, r2: u32, r3: u32| {
                if !valid_sp(base_sp) || !stackish(base_sp, current_a1) {
                    return;
                }
                if (base_sp as u64) < (off as u64) + 16 {
                    return;
                }
                let b = base_sp.wrapping_sub(off);
                // a0..a3 OF is strictly below base_sp (ENTRY locals live at/above).
                if b >= base_sp {
                    return;
                }
                let _ = bus.write_u32(b as u64, r0);
                let _ = bus.write_u32(b as u64 + 4, r1);
                let _ = bus.write_u32(b as u64 + 8, r2);
                let _ = bus.write_u32(b as u64 + 12, r3);
            };

        let frames: Vec<Vec<u32>> = self
            .call_preserve_stack
            .iter()
            .filter(|p| !p.is_empty())
            .map(|p| {
                let mut regs = Vec::with_capacity(p.len() * 4);
                for &(_slot, r) in p.iter() {
                    regs.extend_from_slice(&r);
                }
                regs
            })
            .collect();
        let mut frame_sps: Vec<u32> = frames
            .iter()
            .filter(|r| r.len() >= 2)
            .map(|r| r[1])
            .filter(|&s| valid_sp(s))
            .collect();
        if valid_sp(current_a1) {
            frame_sps.push(current_a1);
        }
        frame_sps.sort_unstable();
        frame_sps.dedup();

        self.call_preserve_stack.clear();
        for i in 0..frames.len() {
            let regs = &frames[i];
            if regs.len() < 4 {
                continue;
            }
            let frame_a1 = regs[1];
            if !valid_sp(frame_a1) {
                continue; // data pointer in a1 — not a real frame
            }
            let callee_sp = if i + 1 < frames.len() {
                frames[i + 1][1]
            } else {
                current_a1
            };
            let spill_sp = if valid_sp(callee_sp)
                && callee_sp <= frame_a1.wrapping_add(8)
                && (stackish(callee_sp, current_a1) || stackish(callee_sp, frame_a1))
            {
                callee_sp
            } else if stackish(frame_a1, current_a1) {
                frame_a1
            } else {
                0
            };
            if spill_sp != 0 {
                write4(bus, spill_sp, 16, regs[0], regs[1], regs[2], regs[3]);
            }
            // CALL8/CALL12: a4..a7 live in the parent OF (parent_sp - 32).
            // Parent SP is the previous preserve frame's a1 (strictly higher).
            if regs.len() >= 8 && i > 0 {
                let parent_a1 = frames[i - 1][1];
                if valid_sp(parent_a1)
                    && parent_a1 > frame_a1
                    && stackish(parent_a1, current_a1)
                    && parent_a1.wrapping_sub(frame_a1) < 0x1000
                {
                    write4(bus, parent_a1, 32, regs[4], regs[5], regs[6], regs[7]);
                }
            }
        }

        // Leftover WS panes: CALL4 a0..a3 only for 16B-aligned known frame SPs.
        let ws = self.regs.windowstart();
        let wb = self.regs.windowbase();
        for slot in 0..16u8 {
            if (ws >> slot) & 1 == 0 {
                continue;
            }
            let dist = slot.wrapping_sub(wb) & 0x0F;
            if dist < 4 {
                continue;
            }
            let base = (slot as usize) * 4;
            let (a0, a1, a2, a3) = if let Some(snap) = self.regs.shadow_top_regs(slot) {
                (snap[0], snap[1], snap[2], snap[3])
            } else {
                (
                    self.regs.physical(base),
                    self.regs.physical(base + 1),
                    self.regs.physical(base + 2),
                    self.regs.physical(base + 3),
                )
            };
            if !valid_sp(a1) {
                continue;
            }
            let a1_ok = frame_sps.contains(&a1)
                || frame_sps
                    .iter()
                    .any(|&f| a1 < f && f.wrapping_sub(a1) < 0x80);
            if a1_ok && stackish(a1, current_a1) {
                write4(bus, a1, 16, a0, a1, a2, a3);
            }
        }

        self.regs.set_windowstart(1u16 << (wb & 0x0F));
        self.regs.set_shadow_stacks(Default::default());
    }

    fn push_irq_window_frame(&mut self, bus: &dyn Bus) {
        let preserve = self.call_preserve_stack.clone();
        // Park preserve under the current FreeRTOS TCB so a later resume of
        // this task can restore CALL8 a4..a7 after a task switch.
        if let Some(tcb) = self.px_current_tcb(bus) {
            if tcb != 0 {
                self.task_preserve_by_tcb.insert(tcb, preserve.clone());
            }
        }
        self.irq_window_stack.push(IrqWindowFrame {
            call_preserve_stack: preserve,
            sp_at_entry: self.regs.read_logical(1),
            windowstart_at_entry: self.regs.windowstart(),
        });
    }

    fn pop_irq_window_frame(&mut self, bus: &dyn Bus) {
        let frame = self.irq_window_stack.pop();
        let cur_sp = self.regs.read_logical(1);
        if let Some(frame) = frame {
            if cur_sp == frame.sp_at_entry {
                // Same task — restore pre-spill preserve + WS.
                self.call_preserve_stack = frame.call_preserve_stack;
                self.regs.set_windowstart(frame.windowstart_at_entry);
                if let Some(tcb) = self.px_current_tcb(bus) {
                    self.task_preserve_by_tcb.remove(&tcb);
                }
                return;
            }
        }
        // Task switch: restore the resumed task's parked preserve if any.
        if let Some(tcb) = self.px_current_tcb(bus) {
            if let Some(preserve) = self.task_preserve_by_tcb.remove(&tcb) {
                // Re-apply outer preserve panes into physical ARs (skip the
                // live window — context_restore just wrote a0..a15 there).
                // Set WS bits for those panes so RETW takes the live path;
                // restore_call_preserve then places a0..a7 relative to WB.
                let wb = self.regs.windowbase();
                let mut ws = 1u16 << (wb & 0x0F);
                for entry in preserve.iter() {
                    for &(slot, regs) in entry.iter() {
                        let slot = slot & 0x0F;
                        let dist = slot.wrapping_sub(wb) & 0x0F;
                        if dist < 4 {
                            continue; // live window
                        }
                        let base = (slot as usize) * 4;
                        self.regs.set_physical(base, regs[0]);
                        self.regs.set_physical(base + 1, regs[1]);
                        self.regs.set_physical(base + 2, regs[2]);
                        self.regs.set_physical(base + 3, regs[3]);
                        ws |= 1u16 << slot;
                    }
                }
                self.regs.set_windowstart(ws);
                self.call_preserve_stack = preserve;
                return;
            }
        }
        self.call_preserve_stack.clear();
        self.regs.set_shadow_stacks(Default::default());
    }

    /// Re-apply hybrid CALL preserve parked under the current FreeRTOS TCB
    /// when `call_preserve_stack` is empty. Needed because FreeRTOS may resume
    /// a task via `_xt_context_restore` (stack) without going through our IRQ
    /// RFE path that normally restores `task_preserve_by_tcb`. Without this,
    /// CALL8 a4..a7 (e.g. xQueueReceive's a5 = mux) are lost after
    /// `GiveFromISR` + yield (ESP32-S3 RMT / RGB L2).
    fn maybe_restore_task_preserve(&mut self, bus: &dyn Bus) {
        if self.faithful_windows || !self.call_preserve_stack.is_empty() {
            return;
        }
        let Some(tcb) = self.px_current_tcb(bus) else {
            return;
        };
        if tcb == 0 {
            return;
        }
        // Keep the entry so subsequent RETWs still see it; only remove on
        // same-task RFE (pop_irq_window_frame) when the live stack is again
        // the authority.
        let Some(preserve) = self.task_preserve_by_tcb.get(&tcb).cloned() else {
            return;
        };
        if preserve.is_empty() {
            return;
        }
        let wb = self.regs.windowbase();
        let mut ws = self.regs.windowstart();
        // Ensure live window bit stays set.
        ws |= 1u16 << (wb & 0x0F);
        for entry in preserve.iter() {
            for &(slot, regs) in entry.iter() {
                let slot = slot & 0x0F;
                let dist = slot.wrapping_sub(wb) & 0x0F;
                if dist < 4 {
                    continue;
                }
                let base = (slot as usize) * 4;
                self.regs.set_physical(base, regs[0]);
                self.regs.set_physical(base + 1, regs[1]);
                self.regs.set_physical(base + 2, regs[2]);
                self.regs.set_physical(base + 3, regs[3]);
                ws |= 1u16 << slot;
            }
        }
        self.regs.set_windowstart(ws);
        self.call_preserve_stack = preserve;
    }

    /// Read `pxCurrentTCBs[core_id]` for the running ESP32 dual-core firmware.
    ///
    /// Address is firmware-specific (`.dram0.bss`). Classic Arduino-ESP32 L0
    /// places the dual-core array at `0x3FFC_27C8`; ESP32-S3 at `0x3FC9_B2B4`.
    /// Hardcoding only classic made S3 hybrid preserve never park under the
    /// real TCB → after `vTaskDelay` task-switch RFE, CALL8 a4..a7 / WS were
    /// lost → APP hung in `_WindowUnderflow8` re-executing RETW forever.
    ///
    /// Prefer the ELF-resolved base in [`rom_thunks::PX_CURRENT_TCB_ADDR`]
    /// (CLI / diag set this from `pxCurrentTCBs`); otherwise auto-discover by
    /// probing known BSS locations for a live DRAM TCB pointer.
    fn px_current_tcb(&self, bus: &dyn Bus) -> Option<u32> {
        use crate::peripherals::esp_xtensa_common::rom_thunks::PX_CURRENT_TCB_ADDR;
        let core = self.core_id() as u32;
        let read_at =
            |base: u32| -> Option<u32> { bus.read_u32((base.wrapping_add(core * 4)) as u64).ok() };
        let looks_like_tcb = |p: u32| {
            p != 0
                && ((0x3FF8_0000..0x4000_0000).contains(&p) // classic internal DRAM
                    || (0x3FC8_8000..0x3FD0_0000).contains(&p)) // ESP32-S3 SRAM
        };

        if let Some(base) = PX_CURRENT_TCB_ADDR.with(|s| s.get()) {
            return read_at(base);
        }

        // Known Arduino-ESP32 L0 layouts (nm `pxCurrentTCBs`).
        const CANDIDATES: [u32; 2] = [
            0x3FFC_27C8, // classic ESP32
            0x3FC9_B2B4, // ESP32-S3
        ];
        for &base in &CANDIDATES {
            if let Some(tcb) = read_at(base) {
                if looks_like_tcb(tcb) {
                    return Some(tcb);
                }
            }
        }
        None
    }

    /// Drop IRQ window snapshots (context-switch / spill). See `irq_window_stack`.
    pub(crate) fn clear_irq_window_stack(&mut self) {
        self.irq_window_stack.clear();
    }

    /// See `defer_irq_until_retw`.
    pub(crate) fn set_defer_irq_until_retw(&mut self, v: bool) {
        self.defer_irq_until_retw = v;
    }

    fn execute(&mut self, ins: xtensa::Instruction, bus: &mut dyn Bus, len: u32) -> SimResult<()> {
        use xtensa::Instruction::*;
        // F5: Per-instruction Window Overflow check (Xtensa LX ISA RM §4.7).
        //
        // Hardware fires a Window Overflow exception when an instruction
        // accesses a logical AR that aliases to a phys reg owned by a
        // different live frame. The check is: for the highest logical reg `R`
        // touched by the instruction, w = (R / 4) + 1 slots ahead of WB must
        // be free. If `WindowStart[(WB + 1)..(WB + w)]` has any set bit, fire
        // overflow with cause based on the rotation distance n (= position of
        // first set bit + 1).
        //
        // Skipped under PS.EXCM (handlers run with EXCM=1 and use S32E/L32E
        // for windowed access without further overflow checks). ENTRY has its
        // own check inside the instruction body. WOE=0 disables windowing.
        // F5 per-instruction overflow check disabled: our sim-level shadow
        // spill on CALL{n} already preserves caller's a0..a{n*4-1} across
        // window wrap-around, so vectoring to the firmware's OF handler
        // here just causes a double-fault (the handler's `l32e a0, a1, -12`
        // reads uninitialized stack with a1=0 on freshly-created task
        // frames). Leaving WOE+EXCM check gated to `false` skips the vector.
        // F5 per-instruction window-overflow check intentionally disabled
        // — see Plan 3 case study and the design comment above. Kept here
        // (#[cfg(any())] gated) for future reference if we ever revisit the
        // canonical OF-vector firmware handler path.
        // F5: per-access window-overflow check — faithful mode only. In
        // shadow mode this stays off (the shadow spill on CALL handles wraps;
        // vectoring here would double-fault on an unprimed save chain).
        if self.faithful_windows && self.ps.woe() && !self.ps.excm() && !matches!(ins, Entry { .. })
        {
            let max_reg = ins.max_logical_reg();
            if max_reg >= 4 {
                let w = (max_reg / 4) as u32; // slots ahead that need to be free
                let wb_old = self.regs.windowbase();
                let ws_full = self.regs.windowstart() as u32;
                let ws_replicated = ws_full | (ws_full << 16);
                let ws_ahead = ws_replicated >> ((wb_old as u32) + 1);
                let trailing = ws_ahead.trailing_zeros();
                if trailing < w {
                    let n = trailing + 1;
                    let wb_handler = wb_old.wrapping_add(n as u8) & 0x0F;
                    let secondary = (ws_ahead >> n).trailing_zeros();
                    let vec_ofs = match secondary {
                        0 => 0x000_u32,
                        1 => 0x080_u32,
                        _ => 0x100_u32,
                    };
                    let vecbase = self.sr.read(VECBASE);
                    self.sr.write(EPC1, self.pc);
                    self.ps.set_owb(wb_old);
                    self.regs.set_windowbase(wb_handler);
                    self.ps.set_excm(true);
                    self.pc = vecbase.wrapping_add(vec_ofs);
                    return Ok(());
                }
            }
        }
        match ins {
            Add { ar, as_, at } => {
                let v = self
                    .regs
                    .read_logical(as_)
                    .wrapping_add(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Sub { ar, as_, at } => {
                let v = self
                    .regs
                    .read_logical(as_)
                    .wrapping_sub(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            And { ar, as_, at } => {
                let v = self.regs.read_logical(as_) & self.regs.read_logical(at);
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Or { ar, as_, at } => {
                let v = self.regs.read_logical(as_) | self.regs.read_logical(at);
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Xor { ar, as_, at } => {
                let v = self.regs.read_logical(as_) ^ self.regs.read_logical(at);
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Neg { ar, at } => {
                let v = 0u32.wrapping_sub(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Abs { ar, at } => {
                // ISA RM: result is unsigned abs of the 2's-complement value.
                // i32::unsigned_abs() returns 0x80000000 for i32::MIN — matches HW behaviour.
                let x = self.regs.read_logical(at) as i32;
                self.regs.write_logical(ar, x.unsigned_abs());
                self.pc = self.pc.wrapping_add(len);
            }
            Addx2 { ar, as_, at } => {
                let v = (self.regs.read_logical(as_) << 1).wrapping_add(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Addx4 { ar, as_, at } => {
                let v = (self.regs.read_logical(as_) << 2).wrapping_add(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Addx8 { ar, as_, at } => {
                let v = (self.regs.read_logical(as_) << 3).wrapping_add(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Subx2 { ar, as_, at } => {
                let v = (self.regs.read_logical(as_) << 1).wrapping_sub(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Subx4 { ar, as_, at } => {
                let v = (self.regs.read_logical(as_) << 2).wrapping_sub(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Subx8 { ar, as_, at } => {
                let v = (self.regs.read_logical(as_) << 3).wrapping_sub(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Movi { at, imm } => {
                self.regs.write_logical(at, imm as u32);
                self.pc = self.pc.wrapping_add(len);
            }
            Break { imm_s, imm_t } => {
                use crate::peripherals::esp_xtensa_common::rom_thunks::{
                    ROM_THUNK_IMM_S, ROM_THUNK_IMM_T,
                };
                if imm_s == ROM_THUNK_IMM_S && imm_t == ROM_THUNK_IMM_T {
                    let pc = self.pc;
                    if let Some(thunk) = bus.get_rom_thunk(pc) {
                        return thunk(self, bus);
                    }
                    return Err(SimulationError::NotImplemented(format!(
                        "ROM thunk at 0x{pc:08x} not registered (BREAK 1,14 with no thunk)"
                    )));
                }
                return Err(SimulationError::BreakpointHit(self.pc));
            }
            Nop | Memw | Extw | Isync | Rsync | Esync | Dsync => {
                self.pc = self.pc.wrapping_add(len);
            }
            Moveqz { ar, as_, at } => {
                if self.regs.read_logical(at) == 0 {
                    let v = self.regs.read_logical(as_);
                    self.regs.write_logical(ar, v);
                }
                self.pc = self.pc.wrapping_add(len);
            }
            Movnez { ar, as_, at } => {
                if self.regs.read_logical(at) != 0 {
                    let v = self.regs.read_logical(as_);
                    self.regs.write_logical(ar, v);
                }
                self.pc = self.pc.wrapping_add(len);
            }
            Movltz { ar, as_, at } => {
                if (self.regs.read_logical(at) as i32) < 0 {
                    let v = self.regs.read_logical(as_);
                    self.regs.write_logical(ar, v);
                }
                self.pc = self.pc.wrapping_add(len);
            }
            Movgez { ar, as_, at } => {
                if (self.regs.read_logical(at) as i32) >= 0 {
                    let v = self.regs.read_logical(as_);
                    self.regs.write_logical(ar, v);
                }
                self.pc = self.pc.wrapping_add(len);
            }
            Waiti { level } => {
                // Set PS.INTLEVEL = level (real silicon does this before
                // entering wait state). We don't model the actual wait —
                // the CPU stays at this instruction (PC doesn't advance),
                // so a caller poll-loop sees the same PC each step and
                // can detect "halted" without us tracking extra state.
                // `waiti_parked` lets later steps skip fetch/decode until a
                // wake-capable IRQ arrives (dual-core APP idle win).
                self.ps.set_intlevel(level);
                self.waiti_parked = true;
            }
            // Xtensa Zero Overhead Loops (LOOP / LOOPNEZ / LOOPGTZ).
            // ISA RM §4.3.2: LCOUNT = as_ - 1, LBEG = PC + 3 (after LOOP),
            // LEND = PC + 3 + offset. After each instruction at PC=LEND-N
            // (last instruction in the loop body), CPU implicitly checks
            // LCOUNT > 0 and branches back to LBEG. We don't have a
            // post-instruction LEND hook today, so this implementation
            // unrolls the loop iteratively here: pre-decrement LCOUNT and
            // jump out when zero, otherwise just fall through and let the
            // body execute. The body's natural fall-through hits PC=LEND
            // which is the instruction AFTER the loop — at that point our
            // ZOL post-PC check below (see step()) re-enters the body.
            Loop { as_, offset } | Loopnez { as_, offset } | Loopgtz { as_, offset } => {
                use crate::cpu::xtensa_sr::{LBEG, LCOUNT, LEND};
                let count = self.regs.read_logical(as_);
                // ISA RM §7.4: LOOPNEZ/LOOPGTZ skip body when count is
                // 0/non-positive. LOOP always enters the body. The post-LEND
                // check decrements LCOUNT and branches back while LCOUNT > 0.
                let take = match ins {
                    Loop { .. } => true,
                    Loopnez { .. } => count != 0,
                    Loopgtz { .. } => (count as i32) > 0,
                    _ => unreachable!(),
                };
                let after = self.pc.wrapping_add(len);
                // ISA RM §7.4.1: LEND = LOOP_PC + 4 + imm8. Decoder produces
                // `offset = imm8 + 4`, so LEND = PC + offset (not PC+len+offset).
                let lend = (self.pc as i32).wrapping_add(offset) as u32;
                if take {
                    self.sr.write(LBEG, after);
                    self.sr.write(LEND, lend);
                    // LCOUNT = count - 1 (wrapping). With post-LEND check
                    // `if LCOUNT > 0 { LCOUNT--; PC = LBEG; }`, body runs
                    // exactly count times for count > 0. For LOOP-with-
                    // count=0, LCOUNT wraps to 0xFFFFFFFF so the body
                    // iterates ~unbounded — terminated only by the body's
                    // own internal branches (this is how strlen sweeps for
                    // a null byte without a fixed upper bound).
                    self.sr.write(LCOUNT, count.wrapping_sub(1));
                    self.pc = after; // fall through to loop body
                } else {
                    // LOOPNEZ/LOOPGTZ with non-positive count: skip body.
                    self.sr.write(LCOUNT, 0);
                    self.pc = lend;
                }
            }

            // ── D2: SAR-setup instructions ───────────────────────────────────
            // SSL as_: SAR = 32 - (as_ & 0x1F).
            // When as_ & 0x1F == 0, SAR = 32 — valid 6-bit value per ISA RM §8.
            Ssl { as_ } => {
                let v = 32u32 - (self.regs.read_logical(as_) & 0x1F);
                self.sr.write(SAR, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SSR as_: SAR = as_ & 0x1F.
            Ssr { as_ } => {
                let v = self.regs.read_logical(as_) & 0x1F;
                self.sr.write(SAR, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SSAI shamt: SAR = shamt & 0x1F (decoder already bounds shamt to 5 bits).
            Ssai { shamt } => {
                self.sr.write(SAR, shamt as u32 & 0x1F);
                self.pc = self.pc.wrapping_add(len);
            }
            // RER — Read External Register: AT = ext_read(AS). The ESP32-S3
            // "external register" space (RF/PHY/config, accessed via a side
            // bus) is not modeled; the boot path only probes config/feature
            // bits here, for which the silicon default reads as 0. Returning 0
            // keeps those checks on their default (feature-off) branch.
            Rer { at, as_ } => {
                let _addr = self.regs.read_logical(as_);
                self.regs.write_logical(at, 0);
                self.pc = self.pc.wrapping_add(len);
            }
            // WER — Write External Register: ext_write(AS, AT). Unmodeled
            // external space — accept and drop, as a benign config write.
            Wer { .. } => {
                self.pc = self.pc.wrapping_add(len);
            }
            // SSA8L as_: SAR = (as_ & 3) * 8. (little-endian byte-select; ISA RM §4.3.7)
            Ssa8l { as_ } => {
                let v = (self.regs.read_logical(as_) & 3) * 8;
                self.sr.write(SAR, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SSA8B as_: SAR = 32 - (as_ & 3) * 8. (big-endian byte-select; ISA RM §4.3.7)
            // When as_ & 3 == 0, SAR = 32 — valid 6-bit value (SAR accommodates 0..=63).
            Ssa8b { as_ } => {
                let v = 32u32 - (self.regs.read_logical(as_) & 3) * 8;
                self.sr.write(SAR, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── D2: Shift register instructions ──────────────────────────────
            // SLL ar, as_: ar = as_ << (32 - SAR).
            // When SAR=0, shift count = 32. Use u64 cast to avoid Rust UB
            // (u64 shifts are defined for counts 0..=63 per Rust reference).
            // (as_ as u64) << 32 = 0 for any as_, which matches ISA RM §8.
            Sll { ar, as_ } => {
                let sar = self.sr.read(SAR);
                let shift = 32u32.wrapping_sub(sar);
                // SAR ranges by setter: SSL 1..=32, SSR 0..=31, SSAI 0..=31, SSA8L {0,8,16,24}, SSA8B {32,24,16,8}.
                // wrapping_sub handles SAR=32 → shift=0 (passthrough); SAR=0 → shift=32 (u64 << 32 yields 0).
                // u64 cast is required because a u32 << 32 is undefined in Rust.
                let v = ((self.regs.read_logical(as_) as u64) << shift) as u32;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SRL ar, at: ar = at >> SAR (unsigned). SAR is 0..=31.
            // For SAR >= 32 (possible if set via WSR), result is 0 per ISA RM §8.
            Srl { ar, at } => {
                let sar = self.sr.read(SAR);
                let v = if sar >= 32 {
                    0
                } else {
                    self.regs.read_logical(at) >> sar
                };
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SRA ar, at: ar = (at as i32) >> SAR (arithmetic). SAR is 0..=31.
            // For SAR >= 32 result is all sign bits: 0xFFFFFFFF or 0x00000000.
            Sra { ar, at } => {
                let sar = self.sr.read(SAR);
                let src = self.regs.read_logical(at) as i32;
                let v = if sar >= 32 {
                    if src < 0 {
                        u32::MAX
                    } else {
                        0
                    }
                } else {
                    (src >> sar) as u32
                };
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SRC ar, as_, at: ar = low32((as_ : at) >> SAR).
            // Concatenate as_ (upper 32b) and at (lower 32b) into 64b, shift right by SAR.
            // SAR is 0..=63; u64 shifts for counts 0..=63 are safe in Rust.
            Src { ar, as_, at } => {
                let sar = self.sr.read(SAR);
                let hi = self.regs.read_logical(as_) as u64;
                let lo = self.regs.read_logical(at) as u64;
                let w = (hi << 32) | lo;
                let v = (w >> sar) as u32;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── D2: Shift immediate instructions ─────────────────────────────
            // SLLI ar, as_, shamt: ar = as_ << shamt. shamt is 1..=31 (decoder
            // computes shamt = 32 - raw, so it's the actual count, never 0 or 32).
            Slli { ar, as_, shamt } => {
                let v = self.regs.read_logical(as_) << shamt;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SRLI ar, at, shamt: ar = at >> shamt (unsigned). shamt 0..=15 from decoder.
            // Note: `at` is the t field (= shamt & 0xF per ISA encoding).
            Srli { ar, at, shamt } => {
                let v = self.regs.read_logical(at) >> shamt;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SRAI ar, at, shamt: ar = (at as i32) >> shamt (arithmetic). shamt 0..=31.
            // shamt < 32 always here (decoder range), so no need for SAR-guard.
            Srai { ar, at, shamt } => {
                let src = self.regs.read_logical(at) as i32;
                let v = (src >> shamt) as u32;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── D3: Arithmetic immediate instructions ──────────────────────────────
            // ADDI at, as_, imm8: at = as_ + sext8(imm8). Two's complement addition.
            Addi { at, as_, imm8 } => {
                let v = self.regs.read_logical(as_).wrapping_add(imm8 as u32);
                self.regs.write_logical(at, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // ADDMI at, as_, imm: at = as_ + imm, where imm = sext8(raw) << 8.
            // Decoder pre-shifts, so imm is already the full immediate value.
            Addmi { at, as_, imm } => {
                let v = self.regs.read_logical(as_).wrapping_add(imm as u32);
                self.regs.write_logical(at, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── D4: Load instructions ──────────────────────────────────────────

            // L8UI at, as_, imm: at = zero_extend(mem[as_ + imm]).
            // imm is the raw byte offset (0..=255); no alignment requirement.
            L8ui { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let val = bus.read_u8(ea)? as u32;
                self.regs.write_logical(at, val);
                self.pc = self.pc.wrapping_add(len);
            }

            // L16UI at, as_, imm: at = zero_extend(mem16[as_ + imm]).
            // Decoder pre-shifts imm by 1 (imm = raw_imm8 << 1), so imm is already
            // the byte offset. Requires 2-byte alignment; alignment check deferred to Phase G.
            L16ui { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let val = bus.read_u16(ea)? as u32;
                self.regs.write_logical(at, val);
                self.pc = self.pc.wrapping_add(len);
            }

            // L16SI at, as_, imm: at = sign_extend(mem16[as_ + imm]).
            // Decoder pre-shifts imm by 1. Sign-extend 16-bit to 32-bit.
            // Alignment check deferred to Phase G.
            L16si { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let raw = bus.read_u16(ea)?;
                // Sign-extend 16-bit: cast to i16 then to i32, reinterpret as u32.
                let val = (raw as i16) as i32 as u32;
                self.regs.write_logical(at, val);
                self.pc = self.pc.wrapping_add(len);
            }

            // L32I at, as_, imm: at = mem32[as_ + imm].
            // Decoder pre-shifts imm by 2 (imm = raw_imm8 << 2). Requires 4-byte alignment;
            // alignment check deferred to Phase G.
            L32i { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let val = bus.read_u32(ea)?;
                self.regs.write_logical(at, val);
                self.pc = self.pc.wrapping_add(len);
            }

            // L32R at, pc_rel_byte_offset:
            //   EA = ((pc + 3) & !3) + pc_rel_byte_offset
            // Decoder sign-extends imm16 as a word count and multiplies by 4 to
            // produce pc_rel_byte_offset (always negative in real code; literal pool
            // precedes the instruction). The resulting EA is always 4-byte aligned
            // (both the aligned base and the offset are multiples of 4).
            L32r {
                at,
                pc_rel_byte_offset,
            } => {
                let base = (self.pc.wrapping_add(3)) & !3u32;
                let ea = base.wrapping_add(pc_rel_byte_offset as u32) as u64;
                let val = bus.read_u32(ea)?;
                self.regs.write_logical(at, val);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── D5: Store instructions ──────────────────────────────────────
            // S8I at, as_, imm: EA = as_ + imm; mem8[EA] = at[0:7].
            // imm is the raw byte offset (0..=255); no alignment requirement.
            S8i { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm);
                self.maybe_invalidate_for_write(ea);
                bus.write_u8(ea as u64, (self.regs.read_logical(at) & 0xFF) as u8)?;
                self.pc = self.pc.wrapping_add(len);
            }

            // S16I at, as_, imm: EA = as_ + imm; mem16[EA] = at[0:15].
            // Decoder pre-shifts imm by 1. Requires 2-byte alignment;
            // alignment check deferred to Phase G.
            S16i { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm);
                self.maybe_invalidate_for_write(ea);
                bus.write_u16(ea as u64, (self.regs.read_logical(at) & 0xFFFF) as u16)?;
                self.pc = self.pc.wrapping_add(len);
            }

            // S32I at, as_, imm: EA = as_ + imm; mem32[EA] = at.
            // Decoder pre-shifts imm by 2. Requires 4-byte alignment;
            // alignment check deferred to Phase G.
            S32i { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm);
                self.maybe_invalidate_for_write(ea);
                bus.write_u32(ea as u64, self.regs.read_logical(at))?;
                self.pc = self.pc.wrapping_add(len);
            }

            // ── D6: Branch instructions ───────────────────────────────────
            // Decoder pre-bakes +4 into all branch offsets, so:
            //   taken:     self.pc = self.pc.wrapping_add(offset as u32)
            //   not-taken: self.pc = self.pc.wrapping_add(len)

            // BEQ: taken if as_ == at
            Beq { as_, at, offset } => {
                let cond = self.regs.read_logical(as_) == self.regs.read_logical(at);
                self.branch(offset, len, cond);
            }
            // BNE: taken if as_ != at
            Bne { as_, at, offset } => {
                let cond = self.regs.read_logical(as_) != self.regs.read_logical(at);
                self.branch(offset, len, cond);
            }
            // BLT: taken if (as_ as i32) < (at as i32)
            Blt { as_, at, offset } => {
                let cond =
                    (self.regs.read_logical(as_) as i32) < (self.regs.read_logical(at) as i32);
                self.branch(offset, len, cond);
            }
            // BGE: taken if (as_ as i32) >= (at as i32)
            Bge { as_, at, offset } => {
                let cond =
                    (self.regs.read_logical(as_) as i32) >= (self.regs.read_logical(at) as i32);
                self.branch(offset, len, cond);
            }
            // BLTU: taken if as_ < at (unsigned)
            Bltu { as_, at, offset } => {
                let cond = self.regs.read_logical(as_) < self.regs.read_logical(at);
                self.branch(offset, len, cond);
            }
            // BGEU: taken if as_ >= at (unsigned)
            Bgeu { as_, at, offset } => {
                let cond = self.regs.read_logical(as_) >= self.regs.read_logical(at);
                self.branch(offset, len, cond);
            }

            // ── D7: Jumps and calls ───────────────────────────────────────────

            // J offset: unconditional jump; decoder pre-bakes +4 into offset.
            // pc = pc + offset  (offset = sign_extend18(imm18) + 4)
            J { offset } => {
                self.pc = self.pc.wrapping_add(offset as u32);
            }

            // JX as_: register-indirect unconditional jump.
            // pc = a[as_]
            Jx { as_ } => {
                self.pc = self.regs.read_logical(as_);
            }

            // CALL0 offset: save return address in a0, jump to target.
            // a0 = pc + 3  (return address: byte after this 3-byte instruction)
            // target = ((pc + 4) & !3) + offset  (ISA RM §4.4; decoder: offset = sext18 * 4)
            //
            // HW-oracle (xtensa-esp-elf-as + objdump):
            //   PC=0, `call0 0x4` → bytes 0x000005 (imm18=0); HW jumps to 0x4.
            //   Formula must give: ((0+4)&!3) + 0 = 4. ✓
            //   Earlier (PC+3)&!3 was used and gave 0+0 = 0 — silently off by 4
            //   for every 4-aligned PC, which broke real ESP32-S3 firmware.
            Call0 { offset } => {
                let ret_pc = self.pc.wrapping_add(3);
                let target = (self.pc.wrapping_add(4) & !3u32).wrapping_add(offset as u32);
                self.regs.write_logical(0, ret_pc);
                self.pc = target;
            }

            // CALLX0 as_: register-indirect CALL0.
            // a0 = pc + 3, pc = a[as_]
            Callx0 { as_ } => {
                let ret_pc = self.pc.wrapping_add(3);
                let target = self.regs.read_logical(as_);
                self.regs.write_logical(0, ret_pc);
                self.pc = target;
            }

            // CALL4/8/12 offset: windowed call.
            // a[N] = (pc + 3 low-30) | (N << 30)
            //   The return address encodes the call type in bits[31:30] so that
            //   RETW can recover N = a0[31:30] after the window rotation.
            //   ISA RM §8 CALL4: "upper two bits of the return address are set to 01".
            // PS.CALLINC = N / 4  (1, 2, or 3 for CALL4, CALL8, CALL12)
            // target = ((pc + 4) & !3) + offset  (ISA RM §4.4)
            //
            // See `Call0` above for the HW-oracle proof of the (pc+4) base.
            Call4 { offset } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (1 << 30);
                let target = (self.pc.wrapping_add(4) & !3u32).wrapping_add(offset as u32);
                self.spill_shadow_on_call(1);
                self.regs.write_logical(4, ret_pc);
                self.ps.set_callinc(1);
                self.pc = target;
            }
            Call8 { offset } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (2 << 30);
                let target = (self.pc.wrapping_add(4) & !3u32).wrapping_add(offset as u32);
                self.spill_shadow_on_call(2);
                self.regs.write_logical(8, ret_pc);
                self.ps.set_callinc(2);
                self.pc = target;
            }
            Call12 { offset } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (3 << 30);
                let target = (self.pc.wrapping_add(4) & !3u32).wrapping_add(offset as u32);
                self.spill_shadow_on_call(3);
                self.regs.write_logical(12, ret_pc);
                self.ps.set_callinc(3);
                self.pc = target;
            }

            // CALLX4/8/12 as_: register-indirect windowed calls.
            // Same semantics as CALL4/8/12 but target = a[as_] (before we overwrite a[N]).
            Callx4 { as_ } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (1 << 30);
                let target = self.regs.read_logical(as_);
                self.spill_shadow_on_call(1);
                self.regs.write_logical(4, ret_pc);
                self.ps.set_callinc(1);
                self.pc = target;
            }
            Callx8 { as_ } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (2 << 30);
                let target = self.regs.read_logical(as_);
                self.spill_shadow_on_call(2);
                self.regs.write_logical(8, ret_pc);
                self.ps.set_callinc(2);
                self.pc = target;
            }
            Callx12 { as_ } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (3 << 30);
                let target = self.regs.read_logical(as_);
                self.spill_shadow_on_call(3);
                self.regs.write_logical(12, ret_pc);
                self.ps.set_callinc(3);
                self.pc = target;
            }

            // RET: CALL0 return. pc = a0.
            Ret => {
                self.pc = self.regs.read_logical(0);
            }

            // ── F1: ENTRY / RETW — windowed call prologue / epilogue ──────────

            // ENTRY as_, imm: windowed call prologue with window overflow check (F3).
            //
            // ISA RM §8 ENTRY semantics:
            //   1. WB_new = (WB_old + PS.CALLINC) mod 16
            //   F3: If WindowStart[(WB_new + 1) mod 16] == 1 → WindowOverflow exception:
            //       - EPC1 = PC (the faulting ENTRY's PC)
            //       - PS.EXCM = 1
            //       - PC = VECBASE + window_vector_offset (OF4/OF8/OF12)
            //       - WindowBase NOT rotated, WindowStart NOT modified, CALLINC NOT cleared
            //       - Return immediately (vector handler will deal with the overflow)
            //   2. WindowStart[WB_new] = 1
            //   3. PS.CALLINC = 0
            //   4. a[as_] -= imm * 8   (in the NEW window; as_ is the stack pointer)
            //   5. PC += len  (instruction is 3 bytes)
            //
            // Note: the rotation happens HERE (on ENTRY), not on CALL*. CALL* only
            // sets PS.CALLINC and stores the return address in a[N] of the OLD frame.
            // After rotation, the callee's a0 maps to the same physical reg as the
            // caller's a[CALLINC*4], which holds the return address written by CALL*.
            //
            // Window vector table (per Xtensa LX ISA RM §5.6, confirmed by Zephyr
            // arch/xtensa/core/window_vectors.S .org directives):
            //   CALLINC=1 (OF4):  VECBASE + 0x000
            //   CALLINC=2 (OF8):  VECBASE + 0x080
            //   CALLINC=3 (OF12): VECBASE + 0x100
            //
            // EXCCAUSE: window overflow exceptions do NOT use EXCCAUSE. They vector
            // independently via dedicated vector slots (not the general exception path).
            // EXCCAUSE values 5/6/7 mean AllocaCause/IntDivByZero/PrivilegedCause.
            Entry { as_, imm } => {
                let callinc = self.ps.callinc();
                let wb_old = self.regs.windowbase();
                let wb_new = wb_old.wrapping_add(callinc) & 0x0F;

                // F3: Window overflow detection. Per Xtensa ISA RM §4.7.1.6,
                // real silicon vectors to OF4/OF8/OF12 handlers that spill the
                // displaced frame to its stack save area, relying on a chain
                // of prior spills to know where the parent frame's SP is
                // (`l32e a0, a1, -12` in OF8/OF12). On a cold call chain that
                // wraps for the first time, no prior spill has primed that
                // chain, so the canonical handler reads garbage.
                //
                // We sidestep this with sim-level transparent spilling: on
                // CALL{n}, if the slot we'd land in is already live, save the
                // displaced frame's a0..a3 to a per-WB shadow stack BEFORE the
                // CALL clobbers them. On the corresponding RETW, restore.
                //
                // The displaced-frame save happens in the CALL{n} exec arms,
                // not here — by the time we reach ENTRY, the corruption has
                // already happened. See `spill_to_shadow_on_call` in this file.

                // Per Xtensa ISA RM §8.1.5 ENTRY:
                //   AR[WB_new*4 + as] = AR[WB_old*4 + as] - imm*8
                // i.e. read the SP from the CALLER's frame, subtract the
                // requested frame size, and write it into the CALLEE's frame
                // — a single value flowing across the window boundary. We
                // were reading post-rotation, which gave the callee an
                // uninitialized AR slot (typically 0) instead of caller's SP,
                // so chained CALL4 calls underflowed SP into 0xffffffXX and
                // every subsequent stack write trapped MemoryViolation.
                let caller_sp = self.regs.read_logical(as_);
                self.regs.set_windowbase(wb_new);
                self.regs.set_windowstart_bit(wb_new, true);
                self.ps.set_callinc(0);
                self.regs
                    .write_logical(as_, caller_sp.wrapping_sub(imm * 8));
                self.pc = self.pc.wrapping_add(len);
            }

            // RETW: windowed return with window underflow check (F4).
            //
            // ISA RM §8 RETW semantics:
            //   1. N = a0[31:30]  (1→CALL4, 2→CALL8, 3→CALL12)
            //   2. wb_dest = (WB_current - N) mod 16
            //   F4: If WindowStart[wb_dest] == 0 → WindowUnderflow exception:
            //       - EPC1 = PC (the faulting RETW's PC)
            //       - PS.EXCM = 1
            //       - PC = VECBASE + window_vector_offset (UF4/UF8/UF12)
            //       - WindowBase NOT rotated, WindowStart NOT modified
            //       - Return immediately (vector handler reloads the spilled frame)
            //   3. target_pc = (a[0] & 0x3FFF_FFFF) | (PC & 0xC000_0000)
            //   4. WindowStart[WB_current] = 0
            //   5. WB = wb_dest
            //   6. PC = target_pc
            //
            // Window underflow vector offsets (Xtensa LX ISA RM §5.6):
            //   N=1 (UF4):  VECBASE + 0x040
            //   N=2 (UF8):  VECBASE + 0x0C0
            //   N=3 (UF12): VECBASE + 0x140
            //
            // EXCCAUSE: window underflow exceptions do NOT use EXCCAUSE. They vector
            // independently via dedicated slots (not the general exception path).
            //
            // N=0 note: RETW with a0[31:30]=0 would indicate a CALL0 return address
            // (which should use RET, not RETW). The wildcard arm in the UF vector
            // match covers N=3; N=0 is treated as N=3 by the same arm, which is
            // benign since CALL0 toolchains never emit RETW. If strict enforcement is
            // needed, add an explicit N=0 → illegal-instruction error here.
            Retw => {
                let a0 = self.regs.read_logical(0);
                let n = (a0 >> 30) as u8; // bits[31:30] = callinc used by the call
                let wb_cur = self.regs.windowbase();
                let wb_dest = wb_cur.wrapping_sub(n) & 0x0F;

                // Shadow hybrid: if we still have a preserve entry for this RETW,
                // force the dest frame live and skip UF — restore_call_preserve
                // below reloads a0..a7 (including ipc_task a5/a6). Stack UF is
                // unreliable after FreeRTOS task switch (save areas get reused).
                if !self.faithful_windows
                    && !self.call_preserve_stack.is_empty()
                    && !self.regs.windowstart_bit(wb_dest)
                    && n > 0
                {
                    self.regs.set_windowstart_bit(wb_dest, true);
                    for k in 1..n {
                        let s = wb_dest.wrapping_add(k) & 0x0F;
                        self.regs.set_windowstart_bit(s, true);
                    }
                }

                // F4: Window underflow check — destination frame must be live.
                if !self.regs.windowstart_bit(wb_dest) {
                    // Window underflow path — symmetric to ENTRY's overflow:
                    // rotate WB *backwards* by N (the call type encoded in
                    // a0[31:30]) so the handler runs in the window the
                    // caller-of-caller occupies. Save WB → PS.OWB so RFWU
                    // can restore it. Set EXCM, EPC1, jump to UF vector.
                    //
                    // Window underflow vector offsets (Xtensa LX ISA RM §5.6):
                    const UF4_VECOFS: u32 = 0x040;
                    const UF8_VECOFS: u32 = 0x0C0;
                    const UF12_VECOFS: u32 = 0x140;
                    let vec_ofs = match n {
                        1 => UF4_VECOFS,
                        2 => UF8_VECOFS,
                        _ => UF12_VECOFS, // N=3 → UF12; N=0 also lands here (see note above)
                    };
                    let vecbase = self.sr.read(VECBASE);
                    self.sr.write(EPC1, self.pc);
                    self.ps.set_owb(wb_cur);
                    self.regs.set_windowbase(wb_dest);
                    self.ps.set_excm(true);
                    self.pc = vecbase.wrapping_add(vec_ofs);
                    // Still consumed a RETW attempt — clear thunk IRQ deferral.
                    self.defer_irq_until_retw = false;
                    return Ok(());
                }

                // Normal RETW path (destination frame is live).
                let target_pc = (a0 & 0x3FFF_FFFF) | (self.pc & 0xC000_0000);
                self.regs.set_windowstart_bit(wb_cur, false);
                self.regs.set_windowbase(wb_dest);
                self.pc = target_pc;
                // The callee just placed its return value in its a2 =
                // AR[wb_cur*4 + 2] = caller's a{n*4 + 2} after rotation.
                // Save it before the pops below — displace pop would restore
                // stale data into that physical and clobber the return value.
                let return_value = if n > 0 {
                    Some(self.regs.read_logical(n * 4 + 2))
                } else {
                    None
                };
                // Hybrid restore:
                //  1. Classic LIFO for displace (callee window) + WS re-set —
                //     same as early-boot path; LIFO holds ONLY displaces now.
                //  2. Authoritative preserve from call_preserve_stack so outer
                //     a4..a7 cannot be stolen by a wrap-around displace sweep.
                for k in 0..4u8 {
                    let slot = wb_cur.wrapping_add(k) & 0x0F;
                    if self.regs.pop_shadow(slot) {
                        self.regs.set_windowstart_bit(slot, true);
                    }
                }
                self.restore_call_preserve();
                if let Some(rv) = return_value {
                    self.regs.write_logical(n * 4 + 2, rv);
                }
                // Close the windowed-thunk IRQ deferral window (if any).
                self.defer_irq_until_retw = false;
            }

            // BANY: taken if (as_ & at) != 0
            Bany { as_, at, offset } => {
                let cond = (self.regs.read_logical(as_) & self.regs.read_logical(at)) != 0;
                self.branch(offset, len, cond);
            }
            // BALL: taken if (as_ & at) == at  (all bits of at set in as_)
            Ball { as_, at, offset } => {
                let a = self.regs.read_logical(as_);
                let b = self.regs.read_logical(at);
                let cond = (a & b) == b;
                self.branch(offset, len, cond);
            }
            // BNONE: taken if (as_ & at) == 0
            Bnone { as_, at, offset } => {
                let cond = (self.regs.read_logical(as_) & self.regs.read_logical(at)) == 0;
                self.branch(offset, len, cond);
            }
            // BNALL: taken if (as_ & at) != at  (at least one bit of at missing in as_)
            Bnall { as_, at, offset } => {
                let a = self.regs.read_logical(as_);
                let b = self.regs.read_logical(at);
                let cond = (a & b) != b;
                self.branch(offset, len, cond);
            }
            // BBC: taken if bit (at & 0x1F) of as_ is CLEAR
            Bbc { as_, at, offset } => {
                let bit = self.regs.read_logical(at) & 0x1F;
                let cond = (self.regs.read_logical(as_) >> bit) & 1 == 0;
                self.branch(offset, len, cond);
            }
            // BBS: taken if bit (at & 0x1F) of as_ is SET
            Bbs { as_, at, offset } => {
                let bit = self.regs.read_logical(at) & 0x1F;
                let cond = (self.regs.read_logical(as_) >> bit) & 1 == 1;
                self.branch(offset, len, cond);
            }
            // BBCI: taken if bit `bit` (0..=31) of as_ is CLEAR
            Bbci { as_, bit, offset } => {
                let cond = (self.regs.read_logical(as_) >> bit) & 1 == 0;
                self.branch(offset, len, cond);
            }
            // BBSI: taken if bit `bit` (0..=31) of as_ is SET
            Bbsi { as_, bit, offset } => {
                let val = self.regs.read_logical(as_);
                let cond = (val >> bit) & 1 == 1;
                if std::env::var_os("LABWIRED_TRACE_BBSI").is_some() && self.pc == 0x400ed00d {
                    eprintln!(
                        "[trace] BBSI at pc=0x{:08x} as_=a{} val=0x{:08x} bit={} cond={}",
                        self.pc, as_, val, bit, cond
                    );
                }
                self.branch(offset, len, cond);
            }
            // BEQZ: taken if as_ == 0
            Beqz { as_, offset } => {
                let cond = self.regs.read_logical(as_) == 0;
                self.branch(offset, len, cond);
            }
            // BNEZ: taken if as_ != 0
            Bnez { as_, offset } => {
                let cond = self.regs.read_logical(as_) != 0;
                self.branch(offset, len, cond);
            }
            // BLTZ: taken if (as_ as i32) < 0
            Bltz { as_, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) < 0;
                self.branch(offset, len, cond);
            }
            // BGEZ: taken if (as_ as i32) >= 0
            Bgez { as_, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) >= 0;
                self.branch(offset, len, cond);
            }
            // BEQI: taken if as_ == imm  (decoder resolved B4CONST[r] into imm: i32)
            Beqi { as_, imm, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) == imm;
                self.branch(offset, len, cond);
            }
            // BNEI: taken if as_ != imm
            Bnei { as_, imm, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) != imm;
                self.branch(offset, len, cond);
            }
            // BLTI: taken if (as_ as i32) < imm  (signed)
            Blti { as_, imm, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) < imm;
                self.branch(offset, len, cond);
            }
            // BGEI: taken if (as_ as i32) >= imm  (signed)
            Bgei { as_, imm, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) >= imm;
                self.branch(offset, len, cond);
            }
            // BLTUI: taken if as_ < imm  (unsigned; decoder resolved B4CONSTU[r] into imm: u32)
            Bltui { as_, imm, offset } => {
                let cond = self.regs.read_logical(as_) < imm;
                self.branch(offset, len, cond);
            }
            // BGEUI: taken if as_ >= imm  (unsigned)
            Bgeui { as_, imm, offset } => {
                let cond = self.regs.read_logical(as_) >= imm;
                self.branch(offset, len, cond);
            }

            // ── MUL family ────────────────────────────────────────────────────────
            // MULL: low 32 bits of unsigned 32×32 product (same bits as signed).
            // SALT: AR[r] = (AR[s] < AR[t]) signed ? 1 : 0.
            Salt { ar, as_, at } => {
                let a = self.regs.read_logical(as_) as i32;
                let b = self.regs.read_logical(at) as i32;
                self.regs.write_logical(ar, u32::from(a < b));
                self.pc = self.pc.wrapping_add(len);
            }
            // SALTU: AR[r] = (AR[s] < AR[t]) unsigned ? 1 : 0.
            Saltu { ar, as_, at } => {
                let a = self.regs.read_logical(as_);
                let b = self.regs.read_logical(at);
                self.regs.write_logical(ar, u32::from(a < b));
                self.pc = self.pc.wrapping_add(len);
            }
            Mull { ar, as_, at } => {
                let v = self
                    .regs
                    .read_logical(as_)
                    .wrapping_mul(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // MULUH: upper 32 bits of unsigned 64-bit product.
            Muluh { ar, as_, at } => {
                let a = self.regs.read_logical(as_) as u64;
                let b = self.regs.read_logical(at) as u64;
                let v = (a.wrapping_mul(b) >> 32) as u32;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // MULSH: upper 32 bits of signed 64-bit product.
            Mulsh { ar, as_, at } => {
                let a = self.regs.read_logical(as_) as i32 as i64;
                let b = self.regs.read_logical(at) as i32 as i64;
                let v = (a.wrapping_mul(b) >> 32) as u32;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // MUL16U: unsigned 16×16 → 32 product; only low 16 bits of each source used.
            Mul16u { ar, as_, at } => {
                let a = self.regs.read_logical(as_) & 0xFFFF;
                let b = self.regs.read_logical(at) & 0xFFFF;
                self.regs.write_logical(ar, a * b);
                self.pc = self.pc.wrapping_add(len);
            }
            // MUL16S: signed 16×16 → 32 product; low 16 sign-extended before multiply.
            Mul16s { ar, as_, at } => {
                let a = self.regs.read_logical(as_) as i16 as i32;
                let b = self.regs.read_logical(at) as i16 as i32;
                self.regs.write_logical(ar, (a * b) as u32);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── DIV family ────────────────────────────────────────────────────
            // Divide-by-zero: set EXCCAUSE=6 (IntegerDivideByZeroCause) and
            // return Err(ExceptionRaised). Full vector dispatch deferred to Phase G.

            // QUOS ar, as_, at: signed quotient as_ / at.
            // i32::MIN / -1 wraps to i32::MIN per ISA RM §8 (saturating result).
            Quos { ar, as_, at } => {
                let dividend = self.regs.read_logical(as_) as i32;
                let divisor = self.regs.read_logical(at) as i32;
                if divisor == 0 {
                    return self.raise_general_exception(6);
                }
                let q = dividend.wrapping_div(divisor);
                self.regs.write_logical(ar, q as u32);
                self.pc = self.pc.wrapping_add(len);
            }

            // QUOU ar, as_, at: unsigned quotient as_ / at.
            Quou { ar, as_, at } => {
                let dividend = self.regs.read_logical(as_);
                let divisor = self.regs.read_logical(at);
                if divisor == 0 {
                    return self.raise_general_exception(6);
                }
                let q = dividend / divisor;
                self.regs.write_logical(ar, q);
                self.pc = self.pc.wrapping_add(len);
            }

            // REMS ar, as_, at: signed remainder as_ % at. Sign follows dividend (Rust `%` semantics).
            // i32::MIN % -1 = 0 (overflow corner; wrapping_rem handles this).
            Rems { ar, as_, at } => {
                let dividend = self.regs.read_logical(as_) as i32;
                let divisor = self.regs.read_logical(at) as i32;
                if divisor == 0 {
                    return self.raise_general_exception(6);
                }
                let r = dividend.wrapping_rem(divisor);
                self.regs.write_logical(ar, r as u32);
                self.pc = self.pc.wrapping_add(len);
            }

            // REMU ar, as_, at: unsigned remainder as_ % at.
            Remu { ar, as_, at } => {
                let dividend = self.regs.read_logical(as_);
                let divisor = self.regs.read_logical(at);
                if divisor == 0 {
                    return self.raise_general_exception(6);
                }
                let r = dividend % divisor;
                self.regs.write_logical(ar, r);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── E3: Bit-manip instructions ────────────────────────────────────

            // NSA ar, as_: Number of Sign bits minus 1.
            // Result = clz(if as_ >= 0 then as_ else !as_) - 1.
            // For as_>=0: counts leading 0 bits minus 1 (result range 0..=31).
            // For as_<0:  counts leading 1 bits minus 1 (same range).
            // NSA(0) = 31 (clz(0)=32, 32-1=31). NSA(-1) = 31 (clz(!0xFFFF)=32, -1=31).
            Nsa { ar, as_ } => {
                let src = self.regs.read_logical(as_);
                let count = if (src as i32) >= 0 {
                    src.leading_zeros()
                } else {
                    (!src).leading_zeros()
                };
                self.regs.write_logical(ar, count - 1);
                self.pc = self.pc.wrapping_add(len);
            }

            // NSAU ar, as_: Number of leading zeros, Unsigned.
            // Result = clz(as_) for unsigned as_. NSAU(0) = 32.
            Nsau { ar, as_ } => {
                let src = self.regs.read_logical(as_);
                self.regs.write_logical(ar, src.leading_zeros());
                self.pc = self.pc.wrapping_add(len);
            }

            // MIN ar, as_, at: ar = signed min(as_, at).
            Min { ar, as_, at } => {
                let a = self.regs.read_logical(as_) as i32;
                let b = self.regs.read_logical(at) as i32;
                self.regs.write_logical(ar, a.min(b) as u32);
                self.pc = self.pc.wrapping_add(len);
            }

            // MAX ar, as_, at: ar = signed max(as_, at).
            Max { ar, as_, at } => {
                let a = self.regs.read_logical(as_) as i32;
                let b = self.regs.read_logical(at) as i32;
                self.regs.write_logical(ar, a.max(b) as u32);
                self.pc = self.pc.wrapping_add(len);
            }

            // MINU ar, as_, at: ar = unsigned min(as_, at).
            Minu { ar, as_, at } => {
                let a = self.regs.read_logical(as_);
                let b = self.regs.read_logical(at);
                self.regs.write_logical(ar, a.min(b));
                self.pc = self.pc.wrapping_add(len);
            }

            // MAXU ar, as_, at: ar = unsigned max(as_, at).
            Maxu { ar, as_, at } => {
                let a = self.regs.read_logical(as_);
                let b = self.regs.read_logical(at);
                self.regs.write_logical(ar, a.max(b));
                self.pc = self.pc.wrapping_add(len);
            }

            // SEXT ar, as_, t: sign-extend as_ from bit position t downward.
            // Decoder stores sa (7..=22) in the `t` field of the Instruction.
            // Bit[sa] of as_ is the sign bit; bits[sa-1:0] are preserved;
            // bits[31:sa] are filled with the value of bit[sa].
            // Equivalently: ((as_ as i32) << (31 - sa)) >> (31 - sa)
            Sext { ar, as_, t: sa } => {
                let src = self.regs.read_logical(as_);
                let shift = 31 - sa; // sa is 7..=22, shift is 9..=24
                let v = ((src as i32) << shift >> shift) as u32;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // CLAMPS ar, as_, t: saturate signed as_ into (sa+1)-bit signed range.
            // Decoder stores sa (7..=22) in the `t` field of the Instruction.
            // Range: [-(2^sa), 2^sa - 1].  For sa=7: [-128, 127].
            Clamps { ar, as_, t: sa } => {
                let src = self.regs.read_logical(as_) as i32;
                let max_val = (1i32 << sa) - 1;
                let min_val = -(1i32 << sa);
                let v = src.clamp(min_val, max_val);
                self.regs.write_logical(ar, v as u32);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── E4: Atomic memory instructions ───────────────────────────────

            // S32C1I at, as_, imm: Compare-and-swap.
            //
            // Semantic (ISA RM §8):
            //   EA = as_ + imm  (decoder pre-shifts imm by 2, so EA = as_ + imm directly)
            //   mem32 = bus.read_u32(EA)
            //   if mem32 == SCOMPARE1: bus.write_u32(EA, at)
            //   at = mem32  (old value always written back to at)
            //
            // Order: read mem first, compare, conditionally write, then update at.
            // For Plan 1 RAM there are no bus read/write side effects, so the order
            // only matters semantically. SCOMPARE1 is read via the SR dispatcher.
            S32c1i { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm);
                let mem32 = bus.read_u32(ea as u64)?;
                let scompare = self.sr.read(SCOMPARE1);
                if mem32 == scompare {
                    self.maybe_invalidate_for_write(ea);
                    bus.write_u32(ea as u64, self.regs.read_logical(at))?;
                }
                self.regs.write_logical(at, mem32);
                self.pc = self.pc.wrapping_add(len);
            }

            // L32AI at, as_, imm: Load Acquire Implicit.
            //
            // In Plan 1 (single-core, no SMP) this is identical to L32I.
            // The acquire barrier is a no-op; SMP ordering is deferred to Plan 4.
            // EA = as_ + imm  (decoder pre-shifts imm by 2).
            L32ai { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let val = bus.read_u32(ea)?;
                self.regs.write_logical(at, val);
                self.pc = self.pc.wrapping_add(len);
            }

            // S32RI at, as_, imm: Store Release Implicit.
            //
            // In Plan 1 (single-core, no SMP) this is identical to S32I.
            // The release barrier is a no-op; SMP ordering is deferred to Plan 4.
            // EA = as_ + imm  (decoder pre-shifts imm by 2).
            S32ri { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm);
                self.maybe_invalidate_for_write(ea);
                bus.write_u32(ea as u64, self.regs.read_logical(at))?;
                self.pc = self.pc.wrapping_add(len);
            }

            // ── F5: S32E / L32E — windowed exception store/load ──────────────
            //
            // These instructions are only valid when PS.EXCM == 1 (i.e. the CPU
            // is executing inside an exception/interrupt vector). Outside that
            // context they raise an IllegalInstruction exception (EXCCAUSE = 0).
            //
            // EA = as_ + imm  (imm is a pre-computed negative byte offset,
            // stored as two's-complement u32 by the decoder; range -64..-4).
            // Full vector dispatch for the exception path is deferred to Phase G;
            // for now we follow the E2 div-by-zero pattern and return
            // Err(ExceptionRaised { cause: 0 }).

            // S32E at, as_, imm: store at to [as_ + imm], privileged.
            //
            // The Xtensa LX ISA RM specifies S32E/L32E as privileged window-
            // spill helpers: legal when PS.EXCM=1 OR PS.RING=0 (kernel mode).
            // ESP-IDF's `_xt_lowint1` interrupt prologue runs with EXCM=0
            // (explicit PS write to 0x40021) but RING=0, then uses S32E to
            // mirror caller-save registers into the spill area below SP.
            // An EXCM-only check incorrectly faulted that path.
            S32e { at, as_, imm } => {
                if !self.ps.excm() && self.ps.ring() != 0 {
                    return self.raise_general_exception(0);
                }
                let ea = self.regs.read_logical(as_).wrapping_add(imm);
                self.maybe_invalidate_for_write(ea);
                bus.write_u32(ea as u64, self.regs.read_logical(at))?;
                self.pc = self.pc.wrapping_add(len);
            }

            // L32E at, as_, imm: load [as_ + imm] into at, privileged.
            // Same EXCM=1 OR RING=0 gate as S32E.
            L32e { at, as_, imm } => {
                if !self.ps.excm() && self.ps.ring() != 0 {
                    return self.raise_general_exception(0);
                }
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let v = bus.read_u32(ea)?;
                self.regs.write_logical(at, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── F6: MOVSP / ROTW ─────────────────────────────────────────────

            // MOVSP at, as_: move stack pointer between adjacent windowed frames.
            //
            // ISA RM §8 MOVSP semantics (Plan 1 safe-path-only implementation):
            //
            //   The instruction checks whether the *next* windowed frame (WindowBase+1)
            //   is currently live. If WindowStart[(WB+1) & 0xF] == 0 (frame not in use),
            //   this is the safe path: a[at] = a[as_], PC += len.
            //
            //   If the next frame IS in use (WS bit set), the hardware must spill/reload
            //   registers between frames before moving the SP. In Plan 1 we do not model
            //   the spill-to-memory ABI (that belongs in Phase G with full exception
            //   handler emulation). Instead, we raise EXCCAUSE=5 (AllocaCause), which is
            //   the documented exception that MOVSP triggers when it detects a live
            //   adjacent frame (per ISA RM §5.5.4: "MOVSP Window Overflow/Underflow").
            //
            // TODO(plan2): implement the full spill path: when WS[(WB+1)&0xF] is set,
            //   save a[(WB+1)*4 .. (WB+1)*4+3] to memory at [a[at]-16..a[at]-4], then
            //   perform the move, then restore from the new SP. This matches the
            //   __window_spill / alloca vector handler ABI used by GCC/ESP-IDF.
            Movsp { at, as_ } => {
                let wb = self.regs.windowbase();
                let next_idx = wb.wrapping_add(1) & 0x0F;

                if self.regs.windowstart_bit(next_idx) {
                    // Adjacent frame is live — silicon raises AllocaCause and the
                    // firmware handler spills one window to the stack save area.
                    if self.faithful_windows {
                        return self.vector_exception(5);
                    }
                    // Shadow / fast-boot mode: just perform the register move.
                    // Live frames are already on the shadow stacks for RETW.
                    // Raising AllocaCause hard-faults heap_caps_init's VLA path.
                }

                // Safe path: simple register move (stack-pointer adjust).
                let v = self.regs.read_logical(as_);
                self.regs.write_logical(at, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // ROTW n: rotate WindowBase by n (4-bit signed, range -8..=+7).
            //
            // ISA RM §8 ROTW semantics:
            //   WindowBase = (WindowBase + n) mod 16
            //   WindowStart is NOT modified.
            //
            // Privileged note: ROTW is a privileged instruction (valid only when
            // PS.RING == 0). Plan 1 does not model PS.RING (we always run at ring 0),
            // so the ring check is skipped.
            //
            // TODO(plan-priv): when PS.RING modelling is added, add a check here:
            //   if ps.ring() != 0 { raise PrivilegedCause (EXCCAUSE=8) }
            Rotw { n } => {
                let wb = self.regs.windowbase();
                // n is i8 (range -8..=+7); wrapping add modulo 16.
                let wb_new = (wb as i32).wrapping_add(n as i32).rem_euclid(16) as u8;
                self.regs.set_windowbase(wb_new);
                // WindowStart is NOT modified (ISA RM §8 ROTW).
                self.pc = self.pc.wrapping_add(len);
            }

            // ── G2: Exception / interrupt return instructions ─────────────────

            // RFE — Return From (level-1 general) Exception.
            //
            // ESP32-S3 LX7 ISA RM §4.4.2 / §8:
            //   PS.EXCM ← 0
            //   PS.INTLEVEL is left unchanged (not reset).
            //   PC ← EPC[1]
            //
            // Note: EPS1 does NOT exist on LX7 (the assembler rejects `rsr.eps1`).
            // Level-1 exceptions save PS in-place; the handler reads/modifies PS
            // directly via RSR/WSR. Only EXCM is cleared by RFE — INTLEVEL is left
            // to the handler to restore explicitly.
            // SYSCALL: raise the Syscall exception (EXCCAUSE=1, SyscallCause).
            // Per Xtensa ISA RM §4.6.5 it vectors to the (user/kernel) general
            // exception handler exactly like a synchronous exception, with
            // EPC1 pointing at the SYSCALL instruction. The firmware's
            // `_xt_to_syscall_exc` handler (window-spill for setjmp/longjmp,
            // libc) does the work and advances EPC1 past the instruction
            // before RFE. Newlib/Unity reach here during the test run.
            Syscall => {
                return self.vector_exception(1);
            }

            Rfe => {
                self.ps.set_excm(false);
                self.pc = self.sr.read(EPC1);
                if !self.faithful_windows {
                    self.pop_irq_window_frame(bus);
                }
            }

            // RFDE — Return From Debug Exception. Handled same as RFE for Plan 1:
            // clear PS.EXCM, jump to EPC1. Full DEPC/debug-exception semantics are
            // deferred to a later plan.
            Rfde => {
                self.ps.set_excm(false);
                self.pc = self.sr.read(EPC1);
                if !self.faithful_windows {
                    self.pop_irq_window_frame(bus);
                }
            }

            // RFI n — Return From Interrupt at level n (n = 2..7 on LX7).
            //
            // ISA RM §4.4.3 / §8 RFI:
            //   PS ← EPS[n]   (restore full PS from saved copy)
            //   PC ← EPC[n]
            //
            // LX7 EPC/EPS SR IDs (hardware-verified, C2 table):
            //   EPC2..EPC7 = SR IDs 178..183
            //   EPS2..EPS7 = SR IDs 194..199
            //
            // Level 1 uses RFE (no EPS1 on LX7). Levels 2..7 use RFI. Level 0
            // and 1 are not valid targets for RFI on LX7; we silently treat them
            // as no-ops (stay at current PC, no state change) since privileged
            // firmware is the only caller and should not issue invalid RFI levels.
            Rfi { level } => {
                let (eps_id, epc_id) = match level {
                    2 => (EPS2, EPC2),
                    3 => (EPS3, EPC3),
                    4 => (EPS4, EPC4),
                    5 => (EPS5, EPC5),
                    6 => (EPS6, EPC6),
                    7 => (EPS7, EPC7),
                    _ => {
                        // Invalid level — skip silently.
                        self.pc = self.pc.wrapping_add(len);
                        return Ok(());
                    }
                };
                let new_ps = self.sr.read(eps_id);
                let new_pc = self.sr.read(epc_id);
                self.ps = Ps::from_raw(new_ps);
                self.pc = new_pc;
                if !self.faithful_windows {
                    self.pop_irq_window_frame(bus);
                }
            }

            // RFWO — Return From Window Overflow handler.
            //
            // Per QEMU `target/xtensa/translate.c::translate_rfw(par=true)`:
            //   1. PS.EXCM ← 0
            //   2. WindowStart[WindowBase] ← 0  (clear the spilled frame's bit;
            //      handler is currently in that frame's window after the entry
            //      rotation)
            //   3. WindowBase ← PS.OWB          (restore via `restore_owb`)
            //   4. PC ← EPC1                    (re-execute the faulting ENTRY)
            //
            // After RFWO, the re-executed ENTRY succeeds because WS at the
            // (previously conflicting) position is now 0, and ENTRY itself
            // sets WS[wb_new] for the new frame.
            Rfwo => {
                let wb_handler = self.regs.windowbase();
                let wb_old = self.ps.owb();
                self.regs.set_windowstart_bit(wb_handler, false);
                self.regs.set_windowbase(wb_old);
                self.ps.set_excm(false);
                self.pc = self.sr.read(EPC1);
            }

            // RFWU — Return From Window Underflow handler.
            //
            // Per QEMU `translate_rfw(par=false)`:
            //   1. PS.EXCM ← 0
            //   2. WindowStart[WindowBase] ← 1  (mark the reloaded frame live;
            //      handler is currently in that frame's window after the entry
            //      rotation)
            //   3. WindowBase ← PS.OWB          (restore via `restore_owb`)
            //   4. PC ← EPC1                    (re-execute the faulting RETW)
            //
            // The re-executed RETW succeeds because WS[wb_dest] is now set.
            Rfwu => {
                let wb_handler = self.regs.windowbase();
                let wb_old = self.ps.owb();
                self.regs.set_windowstart_bit(wb_handler, true);
                self.regs.set_windowbase(wb_old);
                self.ps.set_excm(false);
                self.pc = self.sr.read(EPC1);
            }

            // ── G3: Special-Register / User-Register access ──────────────────
            //
            // RSR at, sr / WSR at, sr / XSR at, sr — atomic read/write/swap of
            // an SR. The SR file holds most SRs; PS, WINDOWBASE, WINDOWSTART
            // live elsewhere on the CPU and route through `read_sr`/`write_sr`.
            //
            // Per ISA RM §5.5, RSR/WSR for unimplemented SRs raise an
            // IllegalInstructionCause exception. We follow Plan 1 policy of
            // permissive-zero for unknown SRs (read returns 0; write is a NOP)
            // because most ESP-IDF / esp-hal startup code reads/writes SRs that
            // we model only as storage (no behavioural side-effects). Genuine
            // privilege checks (PS.RING) are not enforced here because all
            // firmware we simulate runs in ring 0.
            Rsr { at, sr } => {
                let mut v = self.read_sr(sr);
                // INTERRUPT (SR 226) is a hardware-aggregated view of pending
                // interrupts. In our model, peripheral source IDs route
                // through the bus's `pending_cpu_irqs` aggregator and never
                // touch the SR-file `INTERRUPT` slot directly. esp-hal's
                // `__level_1_interrupt` reads INTERRUPT to find which
                // peripheral source fired, so we must OR the bus-side bits
                // in here, otherwise the firmware sees INTERRUPT=0 and
                // never dispatches to the user ISR (Plan 3 Task 10 case
                // study).
                if sr == INTERRUPT {
                    // Per-core IRQ routing handled by the bus aggregator:
                    // PRO_CPU (core 0) gets peripheral source IRQs; both cores
                    // get their own cross-core FROM_CPU IPIs. APP_CPU never
                    // sees PRO_CPU's peripheral interrupts (which would unbalance
                    // its critical nesting → vPortExitCritical "nesting > 0").
                    v |= bus.pending_cpu_irqs(self.core_id());
                }
                self.regs.write_logical(at, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Wsr { at, sr } => {
                let v = self.regs.read_logical(at);
                self.write_sr(sr, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Xsr { at, sr } => {
                let new_v = self.regs.read_logical(at);
                let old_v = self.read_sr(sr);
                self.write_sr(sr, new_v);
                self.regs.write_logical(at, old_v);
                self.pc = self.pc.wrapping_add(len);
            }
            // RUR ar, ur / WUR at, ur — User-Register read/write. URs are a
            // separate 8-bit-ID space from SRs; we model them as a simple
            // [u32; 256] storage array. The commonly-used URs are THREADPTR
            // (231), FCR (232), FSR (233). Floating-point semantics of FCR/FSR
            // are not modeled; they roundtrip as plain storage.
            Rur { ar, ur } => {
                let v = self.ur[(ur as usize) & 0xFF];
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Wur { at, ur } => {
                let v = self.regs.read_logical(at);
                self.ur[(ur as usize) & 0xFF] = v;
                self.pc = self.pc.wrapping_add(len);
            }

            // EXTUI ar, at, shift, bits: ar = (at >> shift) & ((1<<bits)-1).
            // bits ∈ 1..=16, shift ∈ 0..=31. The mask wraps cleanly because
            // `1u32 << 16` is well-defined; for bits=16 we use 0xFFFF.
            Extui {
                ar,
                at,
                shift,
                bits,
            } => {
                let v = self.regs.read_logical(at);
                let mask: u32 = if bits >= 32 {
                    u32::MAX
                } else {
                    (1u32 << bits) - 1
                };
                let extracted = (v >> shift) & mask;
                self.regs.write_logical(ar, extracted);
                self.pc = self.pc.wrapping_add(len);
            }

            // RSIL at, level: atomic { at = PS; PS.INTLEVEL = level; }.
            //
            // Used by esp-hal critical sections to mask interrupts up to a
            // given priority, returning the previous PS so a later WSR.PS
            // can restore it. Per ISA RM the only PS bits modified are
            // INTLEVEL[3:0]; EXCM/UM/CALLINC/etc. are preserved.
            Rsil { at, level } => {
                let prev_ps = self.ps.as_raw();
                self.regs.write_logical(at, prev_ps);
                let new_ps = (prev_ps & !0xF) | (level as u32 & 0xF);
                self.ps = Ps::from_raw(new_ps);
                self.pc = self.pc.wrapping_add(len);
            }

            // ILL / ILL.N — illegal instruction (intentional trap).
            //
            // Xtensa ISA RM §3.5.7 "ILL.N" and §3.5.6 "ILL": these encodings
            // are defined to raise an IllegalInstructionCause exception
            // (EXCCAUSE=0, vector = VECBASE+0x300). They're emitted by the
            // toolchain as "should never reach here" safety nets — most
            // commonly the `memw; ill.n` sequence that follows a write to a
            // RTC_CNTL reset register, where on real silicon the chip resets
            // before the ILL.N is fetched. Without this arm, the simulator's
            // executor falls through to NotImplemented and reports
            // "exec: Ill", which masks the fact that the firmware reached
            // the trap deliberately.
            //
            // Encoding (HW-oracle verified, see decoder/xtensa_narrow.rs):
            //   ILL.N  → 16-bit `6d f0` (op0=0xD, r=0xF, t-field=6, s=0)
            //   ILL    → 24-bit `00 00 00` (routed via Unknown today; same
            //             EXCCAUSE=0 semantics, so leaving the alias).
            Ill => return self.raise_general_exception(0),

            // ── Single-precision FPU (Xtensa LX7 hardware FPU) ──────────────
            // The FR file is `self.fp` (raw u32 bit patterns); arithmetic goes
            // through Rust `f32`, which gives IEEE-754 round-to-nearest and
            // correct NaN/inf/signed-zero handling for free. FCR rounding-mode
            // overrides are not modeled (firmware leaves it at the default).
            AddS { fr, fs, ft } => {
                let v = self.fget(fs) + self.fget(ft);
                self.fset(fr, v);
                self.pc = self.pc.wrapping_add(len);
            }
            SubS { fr, fs, ft } => {
                let v = self.fget(fs) - self.fget(ft);
                self.fset(fr, v);
                self.pc = self.pc.wrapping_add(len);
            }
            MulS { fr, fs, ft } => {
                let v = self.fget(fs) * self.fget(ft);
                self.fset(fr, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // madd.s: fr = fr + fs*ft. f32::mul_add would give a fused (single-
            // rounding) result; the Xtensa FPU rounds the product and the sum
            // separately, so use discrete ops to match the hardware bit-for-bit.
            MaddS { fr, fs, ft } => {
                let v = self.fget(fr) + self.fget(fs) * self.fget(ft);
                self.fset(fr, v);
                self.pc = self.pc.wrapping_add(len);
            }
            MsubS { fr, fs, ft } => {
                let v = self.fget(fr) - self.fget(fs) * self.fget(ft);
                self.fset(fr, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // abs.s / neg.s operate on the sign bit only (preserve NaN payload).
            AbsS { fr, fs } => {
                let v = self.fp[(fs & 0xF) as usize] & 0x7FFF_FFFF;
                self.fp[(fr & 0xF) as usize] = v;
                self.pc = self.pc.wrapping_add(len);
            }
            NegS { fr, fs } => {
                let v = self.fp[(fs & 0xF) as usize] ^ 0x8000_0000;
                self.fp[(fr & 0xF) as usize] = v;
                self.pc = self.pc.wrapping_add(len);
            }
            MovS { fr, fs } => {
                self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
                self.pc = self.pc.wrapping_add(len);
            }
            // rfr ar, fs : AR[ar] = raw bits of f[fs] (move FR → AR, no convert).
            Rfr { ar, fs } => {
                let v = self.fp[(fs & 0xF) as usize];
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // wfr fr, as_ : f[fr] = raw bits of AR[as_] (move AR → FR, no convert).
            Wfr { fr, as_ } => {
                let v = self.regs.read_logical(as_);
                self.fp[(fr & 0xF) as usize] = v;
                self.pc = self.pc.wrapping_add(len);
            }
            // float.s fr, as_, imm : f[fr] = (f32)(i32)AR[as_] * 2^-imm.
            FloatS { fr, as_, imm } => {
                let x = self.regs.read_logical(as_) as i32 as f32;
                let v = x / (1u32 << imm) as f32;
                self.fset(fr, v);
                self.pc = self.pc.wrapping_add(len);
            }
            UfloatS { fr, as_, imm } => {
                let x = self.regs.read_logical(as_) as f32;
                let v = x / (1u32 << imm) as f32;
                self.fset(fr, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // trunc.s / utrunc.s : scale by 2^imm then round toward zero.
            // round.s / ceil.s / floor.s : round to nearest / up / down.
            // Out-of-range / NaN saturate the way Rust's `as` cast does, which
            // matches the FPU's saturating overflow behaviour closely enough
            // for the firmware paths we care about.
            TruncS { ar, fs, imm } => {
                let v = self.fget(fs) * (1u32 << imm) as f32;
                self.regs.write_logical(ar, v.trunc() as i32 as u32);
                self.pc = self.pc.wrapping_add(len);
            }
            UtruncS { ar, fs, imm } => {
                let v = self.fget(fs) * (1u32 << imm) as f32;
                self.regs.write_logical(ar, v.trunc() as u32);
                self.pc = self.pc.wrapping_add(len);
            }
            RoundS { ar, fs, imm } => {
                // round half-to-even (IEEE default), matching round.s.
                let v = self.fget(fs) * (1u32 << imm) as f32;
                self.regs
                    .write_logical(ar, round_half_even(v) as i32 as u32);
                self.pc = self.pc.wrapping_add(len);
            }
            CeilS { ar, fs, imm } => {
                let v = self.fget(fs) * (1u32 << imm) as f32;
                self.regs.write_logical(ar, v.ceil() as i32 as u32);
                self.pc = self.pc.wrapping_add(len);
            }
            FloorS { ar, fs, imm } => {
                let v = self.fget(fs) * (1u32 << imm) as f32;
                self.regs.write_logical(ar, v.floor() as i32 as u32);
                self.pc = self.pc.wrapping_add(len);
            }
            // FP conditional moves: predicate on an AR register, copy FR→FR.
            MoveqzS { fr, fs, at } => {
                if self.regs.read_logical(at) == 0 {
                    self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
                }
                self.pc = self.pc.wrapping_add(len);
            }
            MovnezS { fr, fs, at } => {
                if self.regs.read_logical(at) != 0 {
                    self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
                }
                self.pc = self.pc.wrapping_add(len);
            }
            MovltzS { fr, fs, at } => {
                if (self.regs.read_logical(at) as i32) < 0 {
                    self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
                }
                self.pc = self.pc.wrapping_add(len);
            }
            MovgezS { fr, fs, at } => {
                if (self.regs.read_logical(at) as i32) >= 0 {
                    self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
                }
                self.pc = self.pc.wrapping_add(len);
            }
            // movf.s / movt.s: predicate on boolean register BR[bt].
            MovfS { fr, fs, bt } => {
                if (self.br >> (bt & 0xF)) & 1 == 0 {
                    self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
                }
                self.pc = self.pc.wrapping_add(len);
            }
            MovtS { fr, fs, bt } => {
                if (self.br >> (bt & 0xF)) & 1 == 1 {
                    self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
                }
                self.pc = self.pc.wrapping_add(len);
            }
            // FP compare → boolean register BR[br]. Ordered predicates are
            // false when either operand is NaN; unordered are true on NaN.
            CmpS { br, fs, ft, kind } => {
                use xtensa::FpCmp::*;
                let a = self.fget(fs);
                let b = self.fget(ft);
                let unordered = a.is_nan() || b.is_nan();
                let result = match kind {
                    Un => unordered,
                    Oeq => a == b,
                    Ueq => unordered || a == b,
                    Olt => a < b,
                    Ult => unordered || a < b,
                    Ole => a <= b,
                    Ule => unordered || a <= b,
                };
                let bit = 1u16 << (br & 0xF);
                if result {
                    self.br |= bit;
                } else {
                    self.br &= !bit;
                }
                self.pc = self.pc.wrapping_add(len);
            }
            // FP loads/stores. The FR file holds raw 32-bit patterns, and the
            // bus moves 4 bytes verbatim, so memory round-trips the bit pattern.
            Lsi { ft, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let val = bus.read_u32(ea)?;
                self.fp[(ft & 0xF) as usize] = val;
                self.pc = self.pc.wrapping_add(len);
            }
            Lsiu { ft, as_, imm } => {
                let base = self.regs.read_logical(as_).wrapping_add(imm);
                let val = bus.read_u32(base as u64)?;
                self.fp[(ft & 0xF) as usize] = val;
                self.regs.write_logical(as_, base);
                self.pc = self.pc.wrapping_add(len);
            }
            Ssi { ft, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm);
                self.maybe_invalidate_for_write(ea);
                bus.write_u32(ea as u64, self.fp[(ft & 0xF) as usize])?;
                self.pc = self.pc.wrapping_add(len);
            }
            Ssiu { ft, as_, imm } => {
                let base = self.regs.read_logical(as_).wrapping_add(imm);
                self.maybe_invalidate_for_write(base);
                bus.write_u32(base as u64, self.fp[(ft & 0xF) as usize])?;
                self.regs.write_logical(as_, base);
                self.pc = self.pc.wrapping_add(len);
            }
            // Indexed FP loads/stores: EA = AR[as_] + AR[at]. The *U (xp)
            // forms write the computed address back into AR[as_].
            Lsx { fr, as_, at } => {
                let ea = self
                    .regs
                    .read_logical(as_)
                    .wrapping_add(self.regs.read_logical(at)) as u64;
                let val = bus.read_u32(ea)?;
                self.fp[(fr & 0xF) as usize] = val;
                self.pc = self.pc.wrapping_add(len);
            }
            Lsxu { fr, as_, at } => {
                let base = self
                    .regs
                    .read_logical(as_)
                    .wrapping_add(self.regs.read_logical(at));
                let val = bus.read_u32(base as u64)?;
                self.fp[(fr & 0xF) as usize] = val;
                self.regs.write_logical(as_, base);
                self.pc = self.pc.wrapping_add(len);
            }
            Ssx { fr, as_, at } => {
                let ea = self
                    .regs
                    .read_logical(as_)
                    .wrapping_add(self.regs.read_logical(at));
                self.maybe_invalidate_for_write(ea);
                bus.write_u32(ea as u64, self.fp[(fr & 0xF) as usize])?;
                self.pc = self.pc.wrapping_add(len);
            }
            Ssxu { fr, as_, at } => {
                let base = self
                    .regs
                    .read_logical(as_)
                    .wrapping_add(self.regs.read_logical(at));
                self.maybe_invalidate_for_write(base);
                bus.write_u32(base as u64, self.fp[(fr & 0xF) as usize])?;
                self.regs.write_logical(as_, base);
                self.pc = self.pc.wrapping_add(len);
            }

            // Unknown opcode: raise IllegalInstruction (EXCCAUSE=0).
            //
            // Xtensa LX7 ISA RM §5.2: executing an instruction not defined in the
            // ISA raises a general exception with EXCCAUSE=0 (IllegalInstruction).
            // This is the Plan-1 digital-twin guarantee: any byte pattern decoded
            // as Unknown by the decode layer faithfully raises EXCCAUSE=0, matching
            // real ESP32-S3 hardware behaviour.
            Unknown(_) => return self.raise_general_exception(0),

            // Defensive guard for any instruction variant not yet wired into
            // the executor. Currently unreachable (every variant is handled),
            // but kept so adding a decoder variant fails loudly at runtime
            // rather than silently mis-executing.
            #[allow(unreachable_patterns)]
            _ => return Err(SimulationError::NotImplemented(format!("exec: {:?}", ins))),
        }
        Ok(())
    }

    /// Apply branch condition: if taken, jump to `pc + offset` (offset pre-baked with +4);
    /// otherwise advance by `len` bytes.
    #[inline]
    fn branch(&mut self, offset: i32, len: u32, cond: bool) {
        if cond {
            self.pc = self.pc.wrapping_add(offset as u32);
            self.branched = true;
        } else {
            self.pc = self.pc.wrapping_add(len);
        }
    }

    // ── Interrupt dispatch helpers ────────────────────────────────────────────

    /// This CPU's core id, derived from PRID exactly as the firmware does
    /// (`xPortGetCoreID()` = `(PRID >> 13) & 1`): PRO_CPU PRID `0xCDCD` → 0,
    /// APP_CPU PRID `0xABAB` → 1.
    #[inline]
    fn core_id(&self) -> u8 {
        ((self.sr.read(crate::cpu::xtensa_sr::PRID) >> 13) & 1) as u8
    }

    /// Compute the highest priority level of any pending-and-enabled interrupt.
    ///
    /// Returns `Some(level)` if `(INTERRUPT & INTENABLE) != 0`, else `None`.
    /// The level is the maximum over all set bits in the masked-pending word,
    /// using `IRQ_LEVELS` indexed by bit position.
    fn pending_irq_level(&self, bus: &dyn Bus) -> Option<u8> {
        // Hold IRQs across the CALL→RETW gap of a windowed ROM thunk return
        // (see `defer_irq_until_retw`).
        if self.defer_irq_until_retw {
            return None;
        }
        // Plan 3: aggregate pending IRQs from two sources:
        //   1. SR-file INTERRUPT register (firmware can software-trigger via WSR).
        //   2. Bus's pending_cpu_irqs (peripheral source IDs routed through
        //      the ESP32-S3 intmatrix in tick_peripherals_with_costs).
        // Per-core routing handled by the bus: PRO_CPU takes peripheral IRQs,
        // both cores take their own cross-core FROM_CPU IPIs.
        let bus_irqs = bus.pending_cpu_irqs(self.core_id());
        let pending = (self.sr.read(INTERRUPT) | bus_irqs) & self.sr.read(INTENABLE);
        if pending == 0 {
            return None;
        }
        let max_level = (0u8..32)
            .filter(|&bit| (pending >> bit) & 1 == 1)
            .map(|bit| IRQ_LEVELS[bit as usize])
            .max()?;
        Some(max_level)
    }

    /// Dispatch an interrupt at the given priority `level`.
    ///
    /// Implements Xtensa LX ISA RM §4.4.1 "Interrupt Entry" for ESP32-S3 LX7:
    ///
    /// **Level 1** (uses kernel exception vector, shares with general exceptions):
    ///   1. EPC1     ← PC
    ///   2. EXCCAUSE ← 4 (Level1InterruptCause)
    ///   3. PS.EXCM  ← 1  (PS.INTLEVEL unchanged)
    ///   4. PC       ← VECBASE + 0x300
    ///
    /// **Levels 2..7** (dedicated high-priority interrupt vectors):
    ///   1. EPC[level] ← PC
    ///   2. EPS[level] ← PS
    ///   3. PS.INTLEVEL ← level  (PS.EXCM cleared for level > EXCM_LEVEL, unchanged otherwise)
    ///   4. PC         ← VECBASE + IRQ_VECTOR_OFFSETS[level]
    ///
    /// For levels 2..EXCM_LEVEL (2..3 on ESP32-S3), the Xtensa ISA specifies
    /// that PS.EXCM is set to 1 on entry (medium-priority interrupt entry behaves
    /// like exception entry). For levels > EXCM_LEVEL (4..7), PS.EXCM is cleared
    /// (high-priority: only INTLEVEL gates further interrupts).
    ///
    /// Returns `Ok(())` — unlike `raise_general_exception`, interrupt dispatch
    /// is not an error; the CPU simply redirects to the ISR vector.
    fn dispatch_irq(&mut self, level: u8, bus: &mut dyn Bus) -> SimResult<()> {
        // PC is about to jump into a vector — drop the IRAM/flash
        // fetch cache so the first fetch in the handler re-resolves
        // through the bus (and re-populates the cache for the new
        // region). #119 Phase 1.2.
        self.invalidate_fetch_cache();
        // Shadow mode: outer CALL frames live only in call_preserve until
        // spilled. FreeRTOS may task-switch inside the ISR without calling
        // xthal_window_spill — snapshot preserve+WS, spill to the task stack
        // (OF/UF save areas for a possible switch), then keep the snapshot for
        // same-task RFE restore (so NotifyTake→ipc_task a5/a6 survive).
        if !self.faithful_windows {
            self.push_irq_window_frame(bus); // snapshot preserve + park under TCB
            self.spill_call_preserve_to_stack(bus);
        }
        let entry_pc = self.pc;
        let vecbase = self.sr.read(VECBASE);
        let vector_offset = IRQ_VECTOR_OFFSETS[level.min(7) as usize];

        if level == 1 {
            // Level-1 interrupt: vectors to the GENERAL EXCEPTION handler
            // chosen by PS.UM (user-mode bit, PS[5]):
            //   PS.UM = 0 → KernelExceptionVector at VECBASE + 0x300
            //   PS.UM = 1 → UserExceptionVector   at VECBASE + 0x340
            // Most ESP-IDF task/interrupt code runs in user mode (PS.UM = 1),
            // and only that vector has the actual dispatch logic that branches
            // to `_xt_lowint1` for EXCCAUSE=4. The kernel vector starts with a
            // `break 1,0` panic trap — using it when PS.UM=1 would deadlock.
            self.sr.write(EPC1, entry_pc);
            self.sr.write(EXCCAUSE, EXCCAUSE_LEVEL1_INTERRUPT as u32);
            self.ps.set_excm(true);
            let level1_offset = if (self.ps.as_raw() >> 5) & 1 == 1 {
                0x340 // User
            } else {
                0x300 // Kernel
            };
            self.pc = vecbase.wrapping_add(level1_offset);
        } else {
            // Level 2..7: dedicated interrupt vector.
            // Save PC and PS into EPC[level]/EPS[level].
            let saved_ps = self.ps.as_raw();
            let epc_sr = [0u16, EPC1, EPC2, EPC3, EPC4, EPC5, EPC6, EPC7];
            let eps_sr = [0u16, 0u16, EPS2, EPS3, EPS4, EPS5, EPS6, EPS7];
            let l = level as usize;
            self.sr.write(epc_sr[l], entry_pc);
            self.sr.write(eps_sr[l], saved_ps);

            // Update PS: set INTLEVEL to the dispatched level. Always set
            // PS.EXCM=1 — high-priority interrupt handlers on ESP32 (e.g.,
            // `xt_highint5`) use S32E/L32E for the window-spill prologue,
            // and those instructions require PS.EXCM=1 to avoid raising a
            // GeneralException (cause 0). RFI restores the original PS so
            // this isn't load-bearing after the handler returns.
            let mut new_ps = self.ps;
            new_ps.set_intlevel(level);
            new_ps.set_excm(true);
            self.ps = new_ps;
            self.pc = vecbase.wrapping_add(vector_offset);
        }

        // Plan 3: clear the bus-side pending bits at this level so we don't
        // re-fire next tick. The firmware ISR is responsible for clearing
        // the underlying source-side pending bit (INT_CLR on the peripheral)
        // before the source re-asserts.
        for slot in 0..32u8 {
            if IRQ_LEVELS[slot as usize] == level {
                bus.clear_cpu_irq_pending(self.core_id(), slot);
            }
        }

        Ok(())
    }

    /// Raise a level-1 general exception (kernel vector).
    ///
    /// Implements Xtensa LX ISA RM §5.5 "General Exception" for ESP32-S3 LX7:
    ///   1. EPC1  ← PC (pre-advance; the faulting instruction's address).
    ///   2. EXCCAUSE ← cause.
    ///   3. PS.EXCM  ← 1  (masks interrupts; PS.INTLEVEL is left unchanged).
    ///   4. PC       ← VECBASE + 0x300 (_KernelExceptionVector).
    ///
    /// Note: ESP32-S3 LX7 does NOT implement EPS1 (the assembler rejects
    /// `rsr.eps1`). For level-1 exceptions, the exception handler reads PS
    /// directly via `rsr.ps` after entry. Window OF/UF exceptions do NOT use
    /// this helper — they have dedicated vector slots and different entry rules.
    ///
    /// Returns `Err(ExceptionRaised { cause, pc: EPC1 })` so callers (and tests)
    /// know a general exception was taken while the simulator state is consistent.
    fn raise_general_exception(&mut self, cause: u8) -> SimResult<()> {
        let faulting_pc = self.pc;
        self.sr.write(EPC1, faulting_pc);
        self.sr.write(EXCCAUSE, cause as u32);
        self.ps.set_excm(true);
        let vecbase = self.sr.read(VECBASE);
        self.pc = vecbase.wrapping_add(KERNEL_VECTOR_OFFSET);
        Err(SimulationError::ExceptionRaised {
            cause,
            pc: faulting_pc,
        })
    }

    /// Vector a general exception to the firmware's handler and CONTINUE
    /// (return Ok), mirroring `dispatch_irq`. Unlike `raise_general_exception`
    /// (which aborts the run for the no-handler/fast-boot paths), this drives
    /// the real Xtensa exception flow: save EPC1/EXCCAUSE, set EXCM, and jump
    /// to the User (VECBASE+0x340, PS.UM=1) or Kernel (VECBASE+0x300) exception
    /// vector. Used for MOVSP's AllocaCause (window spill) when booting real
    /// firmware that installs the handlers.
    fn vector_exception(&mut self, cause: u8) -> SimResult<()> {
        self.invalidate_fetch_cache();
        let faulting_pc = self.pc;
        self.sr.write(EPC1, faulting_pc);
        self.sr.write(EXCCAUSE, cause as u32);
        let user_mode = (self.ps.as_raw() >> 5) & 1 == 1;
        self.ps.set_excm(true);
        let vecbase = self.sr.read(VECBASE);
        let offset = if user_mode {
            0x340
        } else {
            KERNEL_VECTOR_OFFSET
        };
        self.pc = vecbase.wrapping_add(offset);
        Ok(())
    }
}

impl Default for XtensaLx7 {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu for XtensaLx7 {
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        let was_halted = self.halted;
        self.regs = ArFile::new();
        self.call_preserve_stack.clear();
        self.irq_window_stack.clear();
        self.task_preserve_by_tcb.clear();
        self.defer_irq_until_retw = false;
        // HW-verified: PS reset = 0x1F (EXCM=1, INTLEVEL=0xF — all ints masked).
        // Confirmed via OpenOCD `reset halt` on real S3-Zero: ps = 0x0000001f.
        self.ps = Ps::from_raw(0x1F);
        // VECBASE=0x40000000; PRID is fixed hardware per core and survives
        // reset — restore APP_CPU's 0xABAB so it keeps core_id 1.
        self.sr = if self.app_cpu {
            XtensaSrFile::new_app_cpu()
        } else {
            XtensaSrFile::new()
        };
        self.pc = 0x4000_0400;
        // Preserve halted state across reset — a CPU configured as
        // APP_CPU (halted at construction) must stay halted through the
        // Machine::load_firmware reset, otherwise it'd race PRO_CPU
        // through the firmware entry point.
        self.halted = was_halted;
        self.waiti_parked = false;
        Ok(())
    }

    fn halt(&mut self) {
        self.halted = true;
        self.waiti_parked = false;
    }
    fn unhalt(&mut self) {
        self.halted = false;
    }
    fn intlevel(&self) -> u8 {
        self.ps.intlevel()
    }

    fn is_parked_idle(&self) -> bool {
        // Not halted: reset-hold still needs lockstep so PRO can release APP.
        self.waiti_parked && !self.halted
    }

    fn idle_fast_forward_budget(&self, bus: &dyn Bus) -> Option<u64> {
        // Architectural WAITI park (FreeRTOS idle / vTaskDelay sleep). Only
        // offer a budget while no wake-capable interrupt is already visible.
        if !self.waiti_parked || self.halted {
            return None;
        }
        if !self.ps.excm() {
            if let Some(irq_level) = self.pending_irq_level(bus) {
                if irq_level > self.ps.intlevel() {
                    return None;
                }
            }
        }
        Some(u64::MAX)
    }

    fn fast_forward_idle_cycles(&mut self, cycles: u64) {
        // Keep CCOUNT coherent with machine total_cycles so CCOMPARE0 edges
        // still fire after an idle skip (FreeRTOS tick source on classic ESP).
        if cycles == 0 {
            return;
        }
        use crate::cpu::xtensa_sr::{CCOMPARE0, CCOUNT};
        let before = self.sr.read(CCOUNT);
        let after = before.wrapping_add(cycles as u32);
        self.sr.write(CCOUNT, after);
        let ccompare0 = self.sr.read(CCOMPARE0);
        if ccompare0 != 0 {
            // Raise timer-0 if the skipped window crossed CCOMPARE0.
            let crossed = if after >= before {
                ccompare0 > before && ccompare0 <= after
            } else {
                // wrap
                ccompare0 > before || ccompare0 <= after
            };
            if crossed {
                self.sr.raise_interrupt_bits(1 << 6);
            }
        }
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        _observers: &[Arc<dyn SimulationObserver>],
        _config: &crate::SimulationConfig,
    ) -> SimResult<()> {
        // Dual-core: a halted CPU contributes nothing — skip the entire
        // step (no CCOUNT advance, no fetch, no IRQ dispatch). Real
        // silicon's APP_CPU sits in reset until PRO_CPU releases it; we
        // model that by leaving `halted=true` until the boot-addr thunk
        // captures the entry point and unhalts the secondary CPU.
        if self.halted {
            return Ok(());
        }
        // FreeRTOS may have switched tasks via stack context restore without
        // our RFE; re-bind CALL8 preserve for the current TCB if needed.
        // Skip when WAITI-parked: idle task is not mid-context-switch.
        if !self.waiti_parked {
            self.maybe_restore_task_preserve(bus);
        }
        // ── Pre-fetch interrupt check ─────────────────────────────────────────
        // Per Xtensa ISA RM §4.4.1: check for pending interrupts before fetching
        // the next instruction.
        //
        // Dispatch conditions (all must be true):
        //   1. PS.EXCM == 0  (if EXCM=1, even high-priority ints are blocked for
        //                     medium levels; for high-priority levels EXCM is set
        //                     to 0 on entry, but we still gate on it here to avoid
        //                     re-entry from within a level-1 handler).
        //   2. (INTERRUPT & INTENABLE) != 0
        //   3. highest_pending_level > PS.INTLEVEL
        //
        // Note: INTENABLE defaults to 0 at reset, so existing tests are unaffected.
        // Advance CCOUNT one cycle per executed instruction (rough but
        // monotonic). When CCOUNT crosses CCOMPARE0, raise the timer-0
        // interrupt (bit 6 in INTERRUPT SR = ESP32 internal timer 0, level 1).
        // FreeRTOS-on-Xtensa uses this as its tick source via `_xt_int6`.
        use crate::cpu::xtensa_sr::{CCOMPARE0, CCOUNT};
        let ccount_before = self.sr.read(CCOUNT);
        let ccount_after = ccount_before.wrapping_add(1);
        self.sr.write(CCOUNT, ccount_after);
        let ccompare0 = self.sr.read(CCOMPARE0);
        if ccompare0 != 0 && ccount_before < ccompare0 && ccount_after >= ccompare0 {
            // Edge-triggered: raise pending bit 6 in INTERRUPT. Use the
            // engine-facing helper because WSR.INTERRUPT writes are ignored
            // (the SR is hardware-latched — INTSET/INTCLEAR is the SW path).
            self.sr.raise_interrupt_bits(1 << 6);
        }

        if !self.ps.excm() {
            if let Some(irq_level) = self.pending_irq_level(bus) {
                if irq_level > self.ps.intlevel() {
                    self.waiti_parked = false;
                    return self.dispatch_irq(irq_level, bus);
                }
            }
        }

        // WAITI park: stay at the same PC without re-fetching/decoding.
        // Dual-core APP_CPU spends most cycles here after FreeRTOS idle starts;
        // skipping fetch/decode is the highest-ROI dual-core idle win that
        // still advances CCOUNT (and therefore CCOMPARE0 tick edges).
        if self.waiti_parked {
            return Ok(());
        }

        // ── Phase 3.2 JIT fast-path (issue #124) ──────────────────────────────
        // If the JIT cache holds a compiled block at this PC, run it in lieu
        // of the per-instruction fetch/decode/execute. The compiled block
        // covers one full pass of the basic block; control returns here with
        // an exit code that says where to continue.
        #[cfg(feature = "jit")]
        {
            if self.jit_enabled {
                if let Some(_n) = self.try_jit_step(bus)? {
                    return Ok(());
                }
            }
        }

        let pc = self.pc;

        // Fast path: serve the fetch out of the cached IRAM/flash slice
        // pointer if PC still falls inside the cached peripheral AND we
        // have enough bytes for the worst-case 4-byte body read. This
        // dodges the bus dispatcher (peripheral lookup + virtual
        // `read_u32` + `RefCell::borrow`) entirely for hot loops
        // (#119 Phase 1.2).
        let pc_u64 = pc as u64;

        // Decode cache: a hit skips fetch AND decode entirely. Direct-mapped
        // by `(pc >> 1)`; `tag == pc` guards aliasing; the generation guards
        // staleness (bumped on every fetch-cache invalidation).
        let dc_idx = (pc as usize >> 1) & DECODE_CACHE_MASK;
        let dc_hit = if self.decode_gen[dc_idx] == self.cur_decode_gen {
            match self.decode_cache[dc_idx] {
                Some((tag, l, i)) if tag == pc => Some((l, i)),
                _ => None,
            }
        } else {
            None
        };

        let (len, ins) = if let Some((l, i)) = dc_hit {
            (l, i)
        } else {
            let cache_hit = match self.fetch_cache {
                Some((start, end, ptr_addr)) if pc_u64 >= start && pc_u64 + 4 <= end => {
                    Some((start, ptr_addr))
                }
                _ => None,
            };

            let (b0, len, ins) = if let Some((start, ptr_addr)) = cache_hit {
                // SAFETY: cache was populated by `Bus::fetch_slice`, which
                // returns a pointer into a `RamPeripheral` backing buffer.
                // The buffer is fixed-size for the peripheral's lifetime
                // (see `system::xtensa::RamPeripheral` INVARIANT) and the
                // CPU invalidates the cache on any bus write that lands in
                // the cached range, on IRQ dispatch, and on snapshot
                // restore. We've already bounds-checked `pc + 4 <= end`.
                let off = (pc_u64 - start) as usize;
                unsafe {
                    let p = (ptr_addr as *const u8).add(off);
                    let b0 = *p;
                    let len = xtensa_length::instruction_length(b0);
                    let ins = if len == 2 {
                        let hw = u16::from_le_bytes([*p, *p.add(1)]);
                        xtensa_narrow::decode_narrow(hw)
                    } else {
                        let w = u32::from_le_bytes([*p, *p.add(1), *p.add(2), *p.add(3)]);
                        xtensa::decode(w)
                    };
                    (b0, len, ins)
                }
            } else {
                // Slow path: ask the bus, then try to populate the cache
                // for next time. Non-RAM peripherals (RomThunkBank, GPIO,
                // declarative regs, …) keep returning `None` from
                // `fetch_slice` and stay on the slow path forever — that's
                // intentional: side-effect-bearing reads must run through
                // the bus.
                let b0 = bus.read_u8(pc_u64)?;
                let len = xtensa_length::instruction_length(b0);
                let ins = if len == 2 {
                    let hw = bus.read_u16(pc_u64)?;
                    xtensa_narrow::decode_narrow(hw)
                } else {
                    let w = bus.read_u32(pc_u64)?;
                    xtensa::decode(w)
                };
                if self.fetch_cache.is_none() {
                    if let Some((start, end, slice)) = bus.fetch_slice(pc_u64) {
                        // Stash pointer-as-usize; see `fetch_cache` doc for
                        // the Send + lifetime story.
                        self.fetch_cache = Some((start, end, slice.as_ptr() as usize));
                    }
                }
                (b0, len, ins)
            };

            // S32E/L32E are 3-byte wide instructions with op0=0 (QRST), op1=9
            // (LSC4), op2=4/0 — decoded by the standard wide path and dispatched
            // through QRST. No special predecode needed; byte0 low nibble = 0
            // (op0=0), so the length predecoder correctly returns 3.
            //
            // (An earlier draft routed S32E via op0=9, requiring a special
            // EXCM-gated predecode here. That decoder agreed with hand-crafted
            // test inputs but rejected real esp-hal firmware. See Plan 3 Task 10
            // case study.)

            let _ = b0; // retained for documentation parity with the slow path
            self.decode_cache[dc_idx] = Some((pc, len, ins));
            self.decode_gen[dc_idx] = self.cur_decode_gen;
            (len, ins)
        };
        self.branched = false;
        let fall_through_pc = pc.wrapping_add(len);
        self.execute(ins, bus, len)?;

        // Zero Overhead Loop post-instruction check (ISA RM §7.4.3 "Loop
        // and Branch Interaction"): the implicit branch back to LBEG
        // fires when the PREVIOUS instruction's natural fall-through path
        // reaches LEND. A taken branch that happens to land at LEND from
        // inside the body must NOT trigger loop-back — strlen relies on
        // this: it ends the loop body with `bnone …, LEND` to exit early,
        // expecting LEND to be the post-loop epilogue. The `branched` flag
        // (set inside `branch()` when a conditional branch fires) tells
        // us whether the last step took a branch or fell through.
        use crate::cpu::xtensa_sr::{LBEG, LCOUNT, LEND};
        let lcount = self.sr.read(LCOUNT);
        let lend = self.sr.read(LEND);
        if lcount > 0 && self.pc == lend && fall_through_pc == lend && !self.branched {
            self.sr.write(LCOUNT, lcount - 1);
            self.pc = self.sr.read(LBEG);
        }
        Ok(())
    }

    fn set_pc(&mut self, val: u32) {
        // External PC override (debug GDB jump, test harness, halt-state
        // reset): conservatively drop the fetch cache so we don't read
        // stale bytes if the new PC lives outside the previously cached
        // range. The PC range check would catch that anyway on the next
        // step, but doing it here means the cached pointer never points
        // past a peripheral that was simultaneously freed.
        self.invalidate_fetch_cache();
        self.pc = val;
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }

    fn set_sp(&mut self, val: u32) {
        // a1 is the stack pointer in the Xtensa windowed ABI.
        self.regs.write_logical(1, val);
    }

    fn set_exception_pending(&mut self, _exception_num: u32) {
        // Phase G implements interrupt dispatch; for Plan 1 this is a no-op.
    }

    fn raise_interrupt_bits(&mut self, mask: u32) {
        self.sr.raise_interrupt_bits(mask);
    }

    fn get_register(&self, id: u8) -> u32 {
        if id < 16 {
            self.regs.read_logical(id)
        } else {
            0
        }
    }

    fn set_register(&mut self, id: u8, val: u32) {
        if id < 16 {
            self.regs.write_logical(id, val);
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot::XtensaLx7(XtensaLx7CpuSnapshot {
            registers: (0u8..16).map(|i| self.regs.read_logical(i)).collect(),
            pc: self.pc,
            ps: self.ps.as_raw(),
            window_base: self.regs.windowbase(),
            window_start: self.regs.windowstart(),
            vecbase: self.sr.read(VECBASE),
        })
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::XtensaLx7(s) = snapshot {
            // Memory may be wholesale-replaced underneath us by the
            // surrounding restore path — drop the fetch cache so the
            // next step re-resolves through the bus. #119 Phase 1.2.
            self.invalidate_fetch_cache();
            self.pc = s.pc;
            self.ps = Ps::from_raw(s.ps);
            self.regs.set_windowbase(s.window_base);
            self.regs.set_windowstart(s.window_start);
            for (i, &v) in s.registers.iter().enumerate().take(16) {
                self.regs.write_logical(i as u8, v);
            }
            self.sr.set_raw(VECBASE, s.vecbase);
        }
    }

    fn runtime_snapshot(&self) -> (crate::runtime_snapshot::CpuKind, Vec<u8>) {
        use crate::runtime_snapshot::XtensaLx7RuntimeSnapshot;
        let snap = XtensaLx7RuntimeSnapshot {
            pc: self.pc,
            ps_raw: self.ps.as_raw(),
            phys: self.regs.phys_slice().to_vec(),
            window_base: self.regs.windowbase(),
            window_start: self.regs.windowstart(),
            shadow: self.regs.shadow_stacks().to_vec(),
            sr: self.sr.raw_storage().to_vec(),
        };
        let bytes = bincode::serialize(&snap).expect("bincode serialize XtensaLx7RuntimeSnapshot");
        (crate::runtime_snapshot::CpuKind::XtensaLx7, bytes)
    }

    fn apply_runtime_snapshot(
        &mut self,
        kind: crate::runtime_snapshot::CpuKind,
        bytes: &[u8],
    ) -> SimResult<()> {
        use crate::runtime_snapshot::{CpuKind, XtensaLx7RuntimeSnapshot};
        if kind != CpuKind::XtensaLx7 {
            return Err(SimulationError::NotImplemented(format!(
                "apply_runtime_snapshot: kind {kind:?} given to XtensaLx7"
            )));
        }
        let snap: XtensaLx7RuntimeSnapshot = bincode::deserialize(bytes).map_err(|e| {
            SimulationError::NotImplemented(format!("XtensaLx7 snapshot decode: {e}"))
        })?;
        if snap.phys.len() != 64 {
            return Err(SimulationError::NotImplemented(
                "XtensaLx7 snapshot phys must be 64 entries".into(),
            ));
        }
        if snap.sr.len() != 256 {
            return Err(SimulationError::NotImplemented(
                "XtensaLx7 snapshot sr must be 256 entries".into(),
            ));
        }
        if snap.shadow.len() != 16 {
            return Err(SimulationError::NotImplemented(
                "XtensaLx7 snapshot shadow must be 16 slots".into(),
            ));
        }
        // Runtime snapshot may swap memory contents underneath us;
        // drop the fetch cache so the next step re-resolves. #119
        // Phase 1.2.
        self.invalidate_fetch_cache();
        self.pc = snap.pc;
        self.ps = Ps::from_raw(snap.ps_raw);
        // Restore order: phys + WB/WS BEFORE shadow stacks so anything that
        // peeks at logical regs during the rest of the call sees coherent
        // state. shadow_stacks::set takes a fixed-size array — collect.
        let mut phys = [0u32; 64];
        phys.copy_from_slice(&snap.phys);
        self.regs.set_phys(phys);
        self.regs.set_windowbase(snap.window_base);
        self.regs.set_windowstart(snap.window_start);
        // Convert Vec<Vec<[u32;4]>> back to [Vec<[u32;4]>; 16].
        let shadow: [Vec<[u32; 4]>; 16] = {
            let mut arr: [Vec<[u32; 4]>; 16] = Default::default();
            for (i, stack) in snap.shadow.into_iter().enumerate().take(16) {
                arr[i] = stack;
            }
            arr
        };
        self.regs.set_shadow_stacks(shadow);
        self.call_preserve_stack.clear();
        let mut sr = [0u32; 256];
        sr.copy_from_slice(&snap.sr);
        self.sr.set_raw_storage(sr);
        Ok(())
    }

    fn get_register_names(&self) -> Vec<String> {
        (0..16).map(|i| format!("a{}", i)).collect()
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        // Xtensa AR registers are a0..a15.
        if let Some(rest) = name.strip_prefix('a') {
            let idx: u8 = rest.parse().ok()?;
            if idx < 16 {
                return Some(idx);
            }
        }
        None
    }

    #[cfg(feature = "jit")]
    fn jit_hit_count(&self) -> u64 {
        self.jit.as_ref().map(|c| c.total_hits()).unwrap_or(0)
    }
}

#[cfg(test)]
mod fp_tests {
    //! Single-precision FPU exec tests. Each test decodes the HW-oracle
    //! encoding (xtensa-esp32s3-elf-as + objdump) and runs it through
    //! `execute`, asserting against the IEEE-754 reference result.
    use super::*;
    use crate::decoder::xtensa::decode;
    use crate::{Bus, DmaRequest, SimResult, SimulationConfig};

    /// Flat-RAM bus: 64 KiB of byte-addressable memory based at 0. Enough for
    /// the lsi/ssi round-trip; all other FP ops never touch the bus.
    struct RamBus {
        mem: Vec<u8>,
        config: SimulationConfig,
    }
    impl RamBus {
        fn new() -> Self {
            Self {
                mem: vec![0u8; 0x1_0000],
                config: SimulationConfig::default(),
            }
        }
    }
    impl Bus for RamBus {
        fn read_u8(&self, addr: u64) -> SimResult<u8> {
            Ok(self.mem[addr as usize])
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
            self.mem[addr as usize] = value;
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
    }

    /// Decode `word` and execute it (3-byte wide form). Returns nothing; the
    /// caller inspects cpu state.
    fn run(cpu: &mut XtensaLx7, bus: &mut RamBus, word: u32) {
        let ins = decode(word);
        cpu.execute(ins, bus, 3).expect("exec");
    }

    #[test]
    fn add_sub_mul_madd() {
        let mut cpu = XtensaLx7::new();
        let mut bus = RamBus::new();
        cpu.fset(4, 1.5);
        cpu.fset(5, 2.25);
        // add.s f3, f4, f5 → 0x0a3450
        run(&mut cpu, &mut bus, 0x0a3450);
        assert_eq!(cpu.fget(3), 3.75);
        // sub.s f3, f4, f5 → 0x1a3450
        run(&mut cpu, &mut bus, 0x1a3450);
        assert_eq!(cpu.fget(3), -0.75);
        // mul.s f3, f4, f5 → 0x2a3450
        run(&mut cpu, &mut bus, 0x2a3450);
        assert_eq!(cpu.fget(3), 3.375);
        // madd.s f3, f4, f5 → 0x4a3450 : f3 = f3 + f4*f5 = 3.375 + 3.375
        run(&mut cpu, &mut bus, 0x4a3450);
        assert_eq!(cpu.fget(3), 6.75);
        // msub.s f3, f4, f5 → 0x5a3450 : f3 = f3 - f4*f5 = 6.75 - 3.375
        run(&mut cpu, &mut bus, 0x5a3450);
        assert_eq!(cpu.fget(3), 3.375);
    }

    #[test]
    fn float_trunc_roundtrip() {
        let mut cpu = XtensaLx7::new();
        let mut bus = RamBus::new();
        cpu.regs.write_logical(4, (-7i32) as u32);
        // float.s f3, a4, 0 → 0xca3400 : f3 = (f32)(-7)
        run(&mut cpu, &mut bus, 0xca3400);
        assert_eq!(cpu.fget(3), -7.0);
        // trunc.s a3, f3, 0 → 0x9a3300 (ar=3, fs=3, imm=0): a3 = (i32)trunc(-7.0)
        run(&mut cpu, &mut bus, 0x9a3300);
        assert_eq!(cpu.regs.read_logical(3) as i32, -7);

        // float.s with scale: a4=10, imm=1 → 10/2 = 5.0
        cpu.regs.write_logical(4, 10);
        run(&mut cpu, &mut bus, 0xca3410); // float.s f3, a4, 1
        assert_eq!(cpu.fget(3), 5.0);
        // ufloat.s f3, a4, 0 with a4 = 0xFFFF_FFFF → 4294967295.0
        cpu.regs.write_logical(4, 0xFFFF_FFFF);
        run(&mut cpu, &mut bus, 0xda3400); // ufloat.s f3, a4, 0
        assert_eq!(cpu.fget(3), 4294967295.0);
    }

    #[test]
    fn abs_neg_mov() {
        let mut cpu = XtensaLx7::new();
        let mut bus = RamBus::new();
        cpu.fset(4, -3.5);
        // abs.s f3, f4 → 0xfa3410
        run(&mut cpu, &mut bus, 0xfa3410);
        assert_eq!(cpu.fget(3), 3.5);
        // neg.s f3, f4 → 0xfa3460
        run(&mut cpu, &mut bus, 0xfa3460);
        assert_eq!(cpu.fget(3), 3.5);
        // mov.s f5, f4 → 0xfa5400 (fr=5, fs=4)
        run(&mut cpu, &mut bus, 0xfa5400);
        assert_eq!(cpu.fget(5), -3.5);
        // -0.0 sign survives neg.s / abs.s as raw bit ops.
        cpu.fset(4, 0.0);
        run(&mut cpu, &mut bus, 0xfa3460); // neg.s f3, f4 → -0.0
        assert_eq!(cpu.fp[3], 0x8000_0000);
    }

    #[test]
    fn rfr_wfr_roundtrip() {
        let mut cpu = XtensaLx7::new();
        let mut bus = RamBus::new();
        cpu.regs.write_logical(4, 0x4048_F5C3); // bits of 3.14
                                                // wfr f3, a4 → 0xfa3450
        run(&mut cpu, &mut bus, 0xfa3450);
        assert_eq!(cpu.fp[3], 0x4048_F5C3);
        // rfr a5, f3 → 0xfa5340 (ar=5, fs=3)
        run(&mut cpu, &mut bus, 0xfa5340);
        assert_eq!(cpu.regs.read_logical(5), 0x4048_F5C3);
    }

    #[test]
    fn compare_and_conditional_move() {
        let mut cpu = XtensaLx7::new();
        let mut bus = RamBus::new();
        cpu.fset(4, 1.0);
        cpu.fset(5, 2.0);
        // olt.s b0, f4, f5 → 0x4b0450 : 1.0 < 2.0 → b0 = 1
        run(&mut cpu, &mut bus, 0x4b0450);
        assert_eq!(cpu.br & 1, 1);
        // oeq.s b0, f4, f5 → 0x2b0450 : 1.0 == 2.0 → b0 = 0
        run(&mut cpu, &mut bus, 0x2b0450);
        assert_eq!(cpu.br & 1, 0);
        // un.s b0, f4, f5 → 0x1b0450 : neither NaN → 0
        run(&mut cpu, &mut bus, 0x1b0450);
        assert_eq!(cpu.br & 1, 0);
        // NaN makes un.s true.
        cpu.fset(5, f32::NAN);
        run(&mut cpu, &mut bus, 0x1b0450);
        assert_eq!(cpu.br & 1, 1);

        // Integer-predicate FP move: moveqz.s f3, f4, a5.
        cpu.fset(4, 9.0);
        cpu.fset(3, 0.0);
        cpu.regs.write_logical(5, 0);
        // moveqz.s f3, f4, a5 → 0x8b3450 : a5==0 → f3 = f4
        run(&mut cpu, &mut bus, 0x8b3450);
        assert_eq!(cpu.fget(3), 9.0);
        // Now a5 != 0: moveqz must NOT copy.
        cpu.fset(4, 1.0);
        cpu.regs.write_logical(5, 1);
        run(&mut cpu, &mut bus, 0x8b3450);
        assert_eq!(cpu.fget(3), 9.0);
    }

    #[test]
    fn lsi_ssi_roundtrip() {
        let mut cpu = XtensaLx7::new();
        let mut bus = RamBus::new();
        cpu.regs.write_logical(1, 0x100); // base address
        cpu.fset(2, 12.5);
        // ssi f2, a1, 160 → 0x284123 : mem32[0x100 + 160] = f2
        run(&mut cpu, &mut bus, 0x284123);
        assert_eq!(bus.read_u32(0x100 + 160).unwrap(), 12.5f32.to_bits());
        // lsi f5, a1, 160 → 0x280153 (ft=5) : f5 = mem32[0x100 + 160]
        run(&mut cpu, &mut bus, 0x280153);
        assert_eq!(cpu.fget(5), 12.5);
    }

    #[test]
    fn lsiu_updates_base() {
        let mut cpu = XtensaLx7::new();
        let mut bus = RamBus::new();
        cpu.regs.write_logical(1, 0x100);
        cpu.fset(2, -1.0);
        // ssip f2, a1, 4 → 0x01c123 (imm8=1<<2=4, r=0xC, s=1, t=2, op0=3):
        // EA = a1 + 4 = 0x104; store there, then write EA back into a1.
        run(&mut cpu, &mut bus, 0x01c123);
        assert_eq!(bus.read_u32(0x104).unwrap(), (-1.0f32).to_bits());
        assert_eq!(cpu.regs.read_logical(1), 0x104);
    }

    #[test]
    fn lsx_indexed() {
        let mut cpu = XtensaLx7::new();
        let mut bus = RamBus::new();
        cpu.regs.write_logical(1, 0x200); // base
        cpu.regs.write_logical(3, 0x10); // index
        cpu.fset(2, 42.0);
        // ssx f2, a1, a3 → 0x482130 : mem32[0x200+0x10] = f2
        run(&mut cpu, &mut bus, 0x482130);
        assert_eq!(bus.read_u32(0x210).unwrap(), 42.0f32.to_bits());
        // lsx f4, a1, a3 → 0x084130 (fr=4) : f4 = mem32[0x210]
        run(&mut cpu, &mut bus, 0x084130);
        assert_eq!(cpu.fget(4), 42.0);
    }
}

#[cfg(test)]
mod window_tests {
    //! Windowed CALL8/RETW must preserve the caller's a0..a7 across the
    //! callee (shadow-spill path). Regression for the ESP32 dual-core boot
    //! fault where `esp_intr_alloc_intrstatus_bind` lost a5 across
    //! `heap_caps_malloc` and then dereferenced a flash string as a pointer.
    use super::*;
    use crate::decoder::xtensa::Instruction::{Call8, Entry, Retw};
    use crate::{Bus, DmaRequest, SimResult, SimulationConfig};

    struct RamBus {
        mem: Vec<u8>,
        config: SimulationConfig,
    }
    impl RamBus {
        fn new() -> Self {
            Self {
                mem: vec![0u8; 0x1_0000],
                config: SimulationConfig::default(),
            }
        }
    }
    impl Bus for RamBus {
        fn read_u8(&self, addr: u64) -> SimResult<u8> {
            Ok(self.mem.get(addr as usize).copied().unwrap_or(0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
            if let Some(s) = self.mem.get_mut(addr as usize) {
                *s = value;
            }
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
    }

    fn exec(cpu: &mut XtensaLx7, bus: &mut RamBus, ins: crate::decoder::xtensa::Instruction) {
        cpu.execute(ins, bus, 3).expect("exec");
    }

    #[test]
    fn call8_retw_preserves_caller_a4_through_a7() {
        let mut cpu = XtensaLx7::new();
        let mut bus = RamBus::new();
        // WOE=1 so windowed ops are meaningful (shadow path still used).
        let mut ps = cpu.ps.as_raw();
        ps |= 1 << 18;
        cpu.ps = Ps::from_raw(ps);

        cpu.regs.write_logical(1, 0x2000);
        cpu.regs.write_logical(4, 0xA4A4_A4A4);
        cpu.regs.write_logical(5, 0x0000_0000); // statusreg=0 pattern from bind
        cpu.regs.write_logical(6, 0xA6A6_A6A6);
        cpu.regs.write_logical(7, 0xA7A7_A7A7);
        cpu.pc = 0x1000;

        exec(&mut cpu, &mut bus, Call8 { offset: 0x40 });
        exec(&mut cpu, &mut bus, Entry { as_: 1, imm: 4 }); // 32-byte frame
                                                            // Callee trashes a1..a7 but MUST leave a0 (return addr + call-type bits) alone.
        for i in 1..8u8 {
            cpu.regs.write_logical(i, 0x1111_0000 | i as u32);
        }
        cpu.regs.write_logical(2, 0x3FFB_C8FC); // return value → caller's a10
        exec(&mut cpu, &mut bus, Retw);

        assert_eq!(
            cpu.regs.read_logical(4),
            0xA4A4_A4A4,
            "a4 must survive CALL8/RETW"
        );
        assert_eq!(
            cpu.regs.read_logical(5),
            0x0000_0000,
            "a5 must survive CALL8/RETW (bind statusreg)"
        );
        assert_eq!(cpu.regs.read_logical(6), 0xA6A6_A6A6, "a6 must survive");
        assert_eq!(cpu.regs.read_logical(7), 0xA7A7_A7A7, "a7 must survive");
        assert_eq!(
            cpu.regs.read_logical(10),
            0x3FFB_C8FC,
            "return value in a10"
        );
    }

    #[test]
    fn nested_call8_preserves_outer_a5() {
        let mut cpu = XtensaLx7::new();
        let mut bus = RamBus::new();
        let mut ps = cpu.ps.as_raw();
        ps |= 1 << 18;
        cpu.ps = Ps::from_raw(ps);

        cpu.regs.write_logical(1, 0x3000);
        cpu.regs.write_logical(5, 0x0000_0000);
        cpu.pc = 0x2000;

        // Outer CALL8
        exec(&mut cpu, &mut bus, Call8 { offset: 0x40 });
        exec(&mut cpu, &mut bus, Entry { as_: 1, imm: 8 }); // 64-byte
                                                            // Inner CALL8 (like malloc → deeper)
        exec(&mut cpu, &mut bus, Call8 { offset: 0x40 });
        exec(&mut cpu, &mut bus, Entry { as_: 1, imm: 4 });
        for i in 1..8u8 {
            cpu.regs.write_logical(i, 0x2222_0000 | i as u32);
        }
        cpu.regs.write_logical(2, 0xAAAA);
        exec(&mut cpu, &mut bus, Retw); // back to outer callee
                                        // Outer callee also clobbers (keep a0)
        for i in 1..8u8 {
            cpu.regs.write_logical(i, 0x3333_0000 | i as u32);
        }
        cpu.regs.write_logical(2, 0xBBBB);
        exec(&mut cpu, &mut bus, Retw); // back to original

        assert_eq!(
            cpu.regs.read_logical(5),
            0x0000_0000,
            "outer a5 must survive nested CALL8"
        );
        assert_eq!(cpu.regs.read_logical(10), 0xBBBB);
    }

    /// Deep CALL8 chain that wraps the 16-slot window file — this is what
    /// `heap_caps_malloc` does in practice and is where a5 was observed
    /// corrupted on the ESP32 Arduino boot path.
    #[test]
    fn deep_call8_wrap_preserves_outer_a5() {
        let mut cpu = XtensaLx7::new();
        let mut bus = RamBus::new();
        let mut ps = cpu.ps.as_raw();
        ps |= 1 << 18;
        cpu.ps = Ps::from_raw(ps);

        cpu.regs.write_logical(1, 0x7000);
        cpu.regs.write_logical(4, 0xA4A4_A4A4);
        cpu.regs.write_logical(5, 0x0000_0000);
        cpu.regs.write_logical(6, 0xA6A6_A6A6);
        cpu.regs.write_logical(7, 0xA7A7_A7A7);
        cpu.pc = 0x4000;

        const DEPTH: usize = 12; // 12 * CALLINC2 = 24 > 16 → wraps
        for _ in 0..DEPTH {
            exec(&mut cpu, &mut bus, Call8 { offset: 0x20 });
            exec(&mut cpu, &mut bus, Entry { as_: 1, imm: 4 });
            for i in 1..8u8 {
                let v = cpu.regs.read_logical(i).wrapping_add(1);
                cpu.regs.write_logical(i, v);
            }
        }
        for d in (0..DEPTH).rev() {
            cpu.regs.write_logical(2, 0x1000 + d as u32);
            exec(&mut cpu, &mut bus, Retw);
        }

        assert_eq!(cpu.regs.read_logical(4), 0xA4A4_A4A4, "a4 after deep wrap");
        assert_eq!(
            cpu.regs.read_logical(5),
            0x0000_0000,
            "a5 after deep wrap (statusreg=0)"
        );
        assert_eq!(cpu.regs.read_logical(6), 0xA6A6_A6A6, "a6 after deep wrap");
        assert_eq!(cpu.regs.read_logical(7), 0xA7A7_A7A7, "a7 after deep wrap");
    }
}
