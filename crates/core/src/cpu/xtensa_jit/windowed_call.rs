// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 JIT — windowed CALL8 emit (Phase 3.6.2 / issue #124).
//!
//! # Windowed-register one-pager (per Xtensa LX ISA RM §§4.7, 8)
//!
//! The Xtensa LX has a 64-entry physical register file (`AR[0..63]`) viewed
//! through a 16-entry logical window (`a0..a15`). The mapping is:
//!
//! ```text
//! logical aN = AR[(WindowBase * 4 + N) & 0x3F]
//! ```
//!
//! `WindowBase` is a 4-bit pointer (0..=15). `WindowStart` is a 16-bit
//! bitmask: each bit `WS[k]` is set iff a live frame occupies the slot of
//! AR registers starting at `AR[k*4]`. At reset WB=0 and WS=0b1 (the
//! initial frame at slot 0 is live).
//!
//! ## CALL8 (the instruction at 0x400d4a99 we're JIT-ing)
//!
//! ISA §8 CALL{n} encodes one of three windowed calls. CALL8 in particular:
//!
//! 1. `ret_pc = ((PC + 3) & 0x3FFFFFFF) | (2 << 30)` — the low 30 bits are
//!    the byte address of the *next* instruction; the top 2 bits encode
//!    `CALLINC = N/4 = 2` so a future RETW knows how far to rotate WB
//!    backwards.
//! 2. `target = ((PC + 4) & ~3) + sign_extend(offset)*4` — a PC-relative
//!    word offset from the *aligned* address of the next instruction.
//! 3. Write `ret_pc` into the caller's logical `a{N} = a8`. This physical
//!    register **becomes the callee's `a0`** once ENTRY rotates WB.
//! 4. `PS.CALLINC := 2`. **Critically, CALL{n} does NOT rotate WindowBase
//!    and does NOT touch WindowStart.** Those happen on the matching
//!    `ENTRY` instruction at the callee's entry point.
//! 5. `PC := target`.
//!
//! The naive "JIT must update WindowBase += 2 and set WindowStart bit"
//! reading of the spec is **incorrect for CALL** — that's ENTRY's job.
//! Doing it on CALL would diverge from the LX7 interpreter and break
//! lockstep instantly. We match the interpreter exactly: set CALLINC,
//! write the return address into the OLD frame's a8, jump.
//!
//! ## Sim-level shadow spill
//!
//! Real silicon raises WindowOverflow when ENTRY lands in an already-live
//! slot, and the OF8/OF12 vector handlers spill the displaced frame to a
//! save-area on the stack. Our sim's [`spill_shadow_on_call`] runs at
//! CALL time and pushes the displaced frame's 4 ARs to a per-WB Vec; the
//! matching RETW pops. This is invisible to firmware (the save chain is
//! never read by guest code) but it MUST execute on every CALL or RETW
//! will pop the wrong frame and corrupt registers later.
//!
//! Because the spill walks the AR file, touches per-slot Vecs, and is
//! conditional on WindowStart bits, replicating it inside wasm would
//! require either:
//!   * exposing the full AR file as a wasm-visible buffer (large surface
//!     area, breaks `Bus` abstraction), or
//!   * calling a host import once per slot check (8+ wasmtime crossings
//!     per CALL8, killing any speedup).
//!
//! The pragmatic choice: **the JIT's wasm body computes only the cheap,
//! purely-functional parts (constants, exit code) and the Rust host
//! applies the architectural side-effects via the existing interpreter
//! helpers (`spill_shadow_on_call`, `write_logical`, `set_callinc`).**
//! Lockstep validates the result against a pure-interpreter trace
//! byte-for-byte.
//!
//! ## Why this still wins (or at least, why it's worth measuring)
//!
//! The interpreter pays per-instruction overhead for: PC fetch, length
//! predecode, body decode, match-arm dispatch, generic-instruction
//! housekeeping (CCOUNT, branched-flag clear, IRQ check). The JIT
//! collapses that to a single wasmtime call + a known side-effect
//! sequence. For a single-instruction block this almost certainly does
//! NOT win on its own (call overhead dwarfs the savings), and the spec
//! is honest about this. The payoff for Phase 3.6 comes from chained
//! windowed flows we'll add in later sub-phases — this file is the
//! *correctness foundation* that lets those land safely.

#![cfg(feature = "jit")]

use wasmtime::{Engine, Instance, Module, Store, TypedFunc};

/// PC of the dominant CALL8 in the ereader firmware (per Phase 3.0
/// dispatch counter data: ~92% of all dispatches land here).
///
/// Disassembly: `400d4a99: 1d d4 e5  call8  400f27e8 <_Z4loopv>`.
pub const LOOPV_CALL8_PC: u32 = 0x400d_4a99;

/// CALL8 target (constant offset from PC encoded in the instruction).
pub const LOOPV_CALL8_TARGET: u32 = 0x400f_27e8;

/// First PC after the CALL8 — used only when constructing the encoded
/// return address (`bits[29:0]`).
pub const LOOPV_CALL8_NEXT_PC: u32 = 0x400d_4a9c;

/// Number of Xtensa instructions the JIT'd block covers. The CALL8 is
/// one instruction; CCOUNT accounting in `try_jit_step` skips the bump
/// if this is 1 (the outer step already counted it).
pub const LOOPV_CALL8_INSTR_COUNT: u32 = 1;

/// Side-exit code: branch taken (caller sets PC = target and continues).
pub const EXIT_TAKEN: i32 = 1;
/// Side-exit code: window overflow / refusal to JIT. Caller falls back
/// to the interpreter for this step. Defined by the Phase 3 roadmap.
pub const EXIT_WINDOWED_REFUSE: i32 = 5;

/// Result of a windowed-CALL8 JIT invocation.
///
/// * `exit_code` — `EXIT_TAKEN` for a normal CALL, `EXIT_WINDOWED_REFUSE`
///   to refuse and let the interpreter handle the step.
/// * `ret_pc_encoded` — the value to write into logical a8 (already with
///   `CALLINC << 30` baked in).
/// * `target_pc` — the new PC.
/// * `callinc` — value for `PS.CALLINC` (2 for CALL8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowedCallResult {
    pub exit_code: i32,
    pub ret_pc_encoded: u32,
    pub target_pc: u32,
    pub callinc: u8,
}

/// Wasm function signature for the CALL8 block.
///
/// Inputs:
///   * `pc` — the current PC (so we can detect a stale cache key if the
///     PC drifts; the wasm code asserts it matches the constant).
///   * `windowstart` — current WindowStart bitmask (16 bits in low half
///     of an i32).
///   * `windowbase` — current WindowBase (0..15) in low 4 bits.
///
/// Outputs: `(exit_code, ret_pc_encoded, target_pc, callinc)` all i32.
pub type WindowedCallParams = (i32, i32, i32);
pub type WindowedCallReturns = (i32, i32, i32, i32);
pub type WindowedCallFn = TypedFunc<WindowedCallParams, WindowedCallReturns>;

/// Compiled CALL8 block.
pub struct WindowedCallBlock {
    store: Store<()>,
    run: WindowedCallFn,
    pub hits: u64,
}

impl WindowedCallBlock {
    /// Build the CALL8 block for [`LOOPV_CALL8_PC`].
    pub fn build_loopv(engine: &Engine) -> wasmtime::Result<Self> {
        // Hand-written WAT.
        //
        // The wasm body computes:
        //   * The encoded return address: `(NEXT_PC & 0x3FFF_FFFF) | (CALLINC << 30)`.
        //     This is a compile-time constant for the LOOPV block; we still
        //     emit the arithmetic so the same template will work for other
        //     CALL8 PCs in future sub-phases.
        //   * The target PC (constant).
        //   * The CALLINC value (constant: 2 for CALL8).
        //
        // Window-overflow refusal protocol: if BOTH bits `WS[wb_new]` and
        // `WS[wb_new+1]` are set after the CALLINC=2 rotation, return
        // EXIT_WINDOWED_REFUSE so the interpreter can handle it. This is
        // overly conservative (the interpreter handles overflow itself via
        // the shadow-spill mechanism), but it keeps the JIT semantically
        // safe even if the shadow-spill assumptions ever change.
        //
        // Actually, since our interp's sim-level spill always succeeds
        // (never raises WindowOverflow), we never need to refuse — but the
        // exit code path is wired so the JIT can opt out later if we ever
        // re-enable the hardware overflow vector.
        let next_pc = LOOPV_CALL8_NEXT_PC;
        let target = LOOPV_CALL8_TARGET;
        let ret_pc_const = (next_pc & 0x3FFF_FFFF) | (2u32 << 30);

        let wat = format!(
            r#"(module
  (func (export "run")
        (param $pc i32) (param $windowstart i32) (param $windowbase i32)
        (result i32 i32 i32 i32)
    (local $wb_new i32)

    ;; Stale-PC guard. If the caller dispatched us at the wrong PC the
    ;; cache key is stale — refuse and let interp handle the step.
    (if (i32.ne (local.get $pc) (i32.const {pc_const}))
      (then
        (return (i32.const {refuse}) (i32.const 0) (i32.const 0) (i32.const 0))))

    ;; Compute wb_new = (windowbase + callinc) & 0x0F  (callinc=2)
    (local.set $wb_new
      (i32.and (i32.add (local.get $windowbase) (i32.const 2)) (i32.const 0x0F)))

    ;; OPTIONAL refusal: if WindowStart bit at wb_new+1 is set, the next
    ;; ENTRY in the callee would step into a still-live frame and need
    ;; the shadow-spill helper. We DO NOT refuse on that condition here
    ;; because the interpreter's spill_shadow_on_call handles it
    ;; transparently — refusing would change observable timing and make
    ;; the JIT useless on every other call. Kept as a no-op block to
    ;; document the decision.
    (drop (local.get $wb_new))
    (drop (local.get $windowstart))

    ;; Return the constants. The host applies spill_shadow_on_call(2),
    ;; writes ret_pc into logical a8, sets PS.CALLINC=2, sets PC=target.
    (i32.const {taken})        ;; exit_code
    (i32.const {ret_pc_const}) ;; ret_pc_encoded
    (i32.const {target})       ;; target_pc
    (i32.const 2)              ;; callinc
  )
)
"#,
            pc_const = LOOPV_CALL8_PC as i32,
            refuse = EXIT_WINDOWED_REFUSE,
            taken = EXIT_TAKEN,
            ret_pc_const = ret_pc_const as i32,
            target = target as i32,
        );

        let module = Module::new(engine, wat)?;
        let mut store: Store<()> = Store::new(engine, ());
        let instance = Instance::new(&mut store, &module, &[])?;
        let run = instance
            .get_typed_func::<WindowedCallParams, WindowedCallReturns>(&mut store, "run")?;
        Ok(Self {
            store,
            run,
            hits: 0,
        })
    }

    /// Invoke the compiled CALL8 block.
    #[inline]
    pub fn run(
        &mut self,
        pc: u32,
        windowstart: u16,
        windowbase: u8,
    ) -> wasmtime::Result<WindowedCallResult> {
        let (exit, ret_pc, target, callinc) = self.run.call(
            &mut self.store,
            (pc as i32, windowstart as i32, windowbase as i32),
        )?;
        self.hits += 1;
        Ok(WindowedCallResult {
            exit_code: exit,
            ret_pc_encoded: ret_pc as u32,
            target_pc: target as u32,
            callinc: (callinc & 0xFF) as u8,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopv_call8_returns_constants() {
        let engine = Engine::default();
        let mut block = WindowedCallBlock::build_loopv(&engine).expect("compile");
        let res = block.run(LOOPV_CALL8_PC, 0x0001, 0).expect("run");
        assert_eq!(res.exit_code, EXIT_TAKEN);
        assert_eq!(res.target_pc, LOOPV_CALL8_TARGET);
        assert_eq!(res.callinc, 2);
        // Encoded ret_pc: (0x400d4a9c & 0x3FFF_FFFF) | (2 << 30) = 0x800d4a9c
        assert_eq!(res.ret_pc_encoded, 0x800d_4a9c);
        assert_eq!(block.hits, 1);
    }

    #[test]
    fn stale_pc_returns_refuse() {
        let engine = Engine::default();
        let mut block = WindowedCallBlock::build_loopv(&engine).expect("compile");
        let res = block.run(0x4000_0000, 0x0001, 0).expect("run");
        assert_eq!(res.exit_code, EXIT_WINDOWED_REFUSE);
    }

    #[test]
    fn multiple_hits_match() {
        let engine = Engine::default();
        let mut block = WindowedCallBlock::build_loopv(&engine).expect("compile");
        let r1 = block.run(LOOPV_CALL8_PC, 0x0001, 0).expect("run");
        let r2 = block.run(LOOPV_CALL8_PC, 0xFFFF, 7).expect("run");
        let r3 = block.run(LOOPV_CALL8_PC, 0x0042, 15).expect("run");
        assert_eq!(r1, r2);
        assert_eq!(r2, r3);
        assert_eq!(block.hits, 3);
    }
}
