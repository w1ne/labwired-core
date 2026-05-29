// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 JIT — multi-op basic-block wasmtime adapter (#124 Phase 3.6.3
//! / Phase 4.1 refactor).
//!
//! Phase 3.6.3 introduced the multi-op hot-block JIT. Phase 4.1 split that
//! work into two halves:
//!   * [`super::emit_core`] owns the runtime-agnostic walker + the bytes
//!     `walk_and_emit` produces. No wasmtime imports.
//!   * This module is a thin wasmtime adapter — it consumes the bytes,
//!     hands them to `wasmtime::Module::new`, wires up the `host.read_u8`
//!     import, and dispatches `Func::call`. The browser stub in
//!     `labwired-wasm::jit_browser` is the equivalent JS-side adapter
//!     and will share the exact same byte stream once Phase 4.2 fills it
//!     in.
//!
//! ## Target block (BB profile, 10M-step ereader run)
//!
//! `0x400829cc` — the dominant hot block by `hits × length` metric:
//!   - 908,569 hits
//!   - 9 instructions per pass before reaching `callx8`
//!   - 8,177,121 instructions executed (≈82% of all ereader work)
//!
//! Disassembly (objdump on labwired-ereader.ino.elf):
//! ```text
//! 400829cc: 20a550        or     a10, a5, a5        ; a10 = a5  (mov pseudoinst)
//! 400829cf: 0020c0        memw                      ; memory barrier — nop in sim
//! 400829d2: 000362        l8ui   a6,  a3, 0         ; a6  = mem8[a3+0]
//! 400829d5: 0020c0        memw                      ; memory barrier
//! 400829d8: 010322        l8ui   a2,  a3, 1         ; a2  = mem8[a3+1]
//! 400829db: 742020        extui  a2,  a2, 0, 8      ; a2  = a2 & 0xFF (mask)
//! 400829de: 102260        and    a2,  a2, a6        ; a2 &= a6
//! 400829e1: f6d481        l32r   a8,  0x40080534    ; a8  = literal at 0x40080534
//! 400829e4: 0008e0        callx8 a8                 ; windowed call — TERMINATOR
//! ```
//!
//! We JIT the **first 8 instructions** (range 0x400829cc..0x400829e4)
//! and exit at the callx8. The interpreter handles the windowed call.
//!
//! ## L32R literal pre-resolution
//!
//! L32R reads `mem32[((PC+3) & ~3) + offset]`. For our target block this
//! address is `0x40080534` and the value there is `0x40008534`
//! (verified via `xtensa-esp32-elf-objdump -s`). The literal-pool region
//! lives in flash/IRAM which is immutable for our purposes. We resolve
//! the constant ONCE at JIT compile time and bake it into the wasm.
//! No host import needed for L32R.
//!
//! ## L8UI host import
//!
//! Bytes at `mem8[a3+0]` and `mem8[a3+1]` could land on any peripheral or
//! DRAM, so the wasm calls `host.read_u8(addr) -> i32` twice. The host
//! routes to `Bus::read_u8`, which keeps peripheral observers,
//! declarative-register hooks, and bus error semantics intact.
//!
//! ## Side-exit codes
//!
//! Wire codes are in [`crate::cpu::xtensa_jit_bytes`]; the higher-level
//! reason vocabulary lives in [`super::emit_core::SideExitReason`].

#![cfg(feature = "jit")]

use std::sync::Mutex;
use wasmtime::{Engine, Func, Instance, Module, Store};

use super::emit_core::{self, BlockShape, EmittedBlock, PsBits, SideExitReason};

// ── Side-exit codes + BB constants ─────────────────────────────────────
//
// Re-exported from `cpu::xtensa_jit_bytes` (which is always compiled,
// even without `--features jit`) so the browser-side prototype can use
// the same values without dragging wasmtime into its dep graph
// (#124 Phase 4).
pub use crate::cpu::xtensa_jit_bytes::{
    EXIT_BRANCH_TAKEN, EXIT_FALL_THROUGH, EXIT_HOST_BUS_ERROR, HOT_BB_END, HOT_BB_INSTR_COUNT,
    HOT_BB_L32R_ADDR, HOT_BB_PC, LOOPTASK_PC, LOOPTASK_PREFIX_END, LOOPTASK_PREFIX_INSTR_COUNT,
    LOOPTASK_TAIL_END, LOOPTASK_TAIL_INSTR_COUNT, LOOPTASK_TAIL_PC,
};

// Re-export the walker types from emit_core for back-compat with the
// existing public API (tests + xtensa_lx7.rs reach for these via
// `xtensa_jit::{walk_bb, DecodedOp}`).
pub use super::emit_core::{walk_bb, DecodedOp};

// ── Wasm function signatures ──────────────────────────────────────────
//
// Each [`BlockShape`] pairs a wasm `(params) -> (results)` signature
// with a register/PC marshalling convention. We invoke wasmtime through
// the untyped `Func::call` path so the same `MultiOpBlock` struct can
// dispatch any shape's params/results without us needing N different
// `TypedFunc<P, R>` slots.
//
// HotBbCanonical:    (a3, a5, l32r) -> (exit, a2, a6, a8, a10)
// LoopTaskPrefix:    (l32r_val, l8ui_base) -> (exit, next_pc, l8ui_at, beqz_target_pc)

/// Per-call scratch slot. The L8UI import pushes a bus-error flag here
/// if it's called with the pending queue empty.
#[derive(Default)]
struct ScratchSlot {
    /// Byte values read from `host.read_u8`, in call order. Currently
    /// unused — kept for symmetry with the browser-side queue and so
    /// future debug instrumentation can record load order.
    #[allow(dead_code, reason = "reserved for Phase 4.2 trace instrumentation")]
    bytes: Vec<u8>,
    /// True iff any L8UI import hit a host bus error.
    bus_error: bool,
}

pub struct MultiOpBlock {
    store: Store<()>,
    /// Untyped wasm export. We dispatch through `Func::call` so a single
    /// `MultiOpBlock` struct can run any [`BlockShape`]'s param/result
    /// signature; the [`Self::shape`] tag tells us how to marshal both
    /// directions.
    run: Func,
    scratch: std::sync::Arc<Mutex<ScratchSlot>>,
    pub hits: u64,
    /// L8UI host-import call sequence, populated by the caller before
    /// the wasm call. The wasm body indexes into this by position.
    pending_loads: std::sync::Arc<Mutex<Vec<u32>>>,
    /// The emit-core output we built this block from. Held so callers
    /// can interrogate `length_in_instrs`, `end_pc`, the side-exit
    /// reason map, and the [`BlockShape`] tag.
    pub emitted: EmittedBlock,
}

/// Result of running the canonical HotBbCanonical block.
pub struct MultiOpResult {
    pub exit_code: i32,
    pub a2: u32,
    pub a6: u32,
    pub a8: u32,
    pub a10: u32,
}

/// Result of running a [`BlockShape::LoopTaskPrefix`] block.
///
/// Field naming mirrors the emit-core wasm signature:
/// `(exit_code, next_pc, l8ui_at, beqz_target_pc)`. `next_pc` is the
/// PC the interpreter should resume from (branch target if BEQZ was
/// taken, [`LOOPTASK_PREFIX_END`] on fall-through). `l8ui_at_value` is
/// the byte the L8UI loaded — the caller writes it into the Xtensa
/// register named by [`BlockShape::LoopTaskPrefix::l8ui_at`].
pub struct LoopTaskPrefixResult {
    pub exit_code: i32,
    pub next_pc: u32,
    pub l8ui_at_value: u32,
    pub beqz_target_pc: u32,
}

/// Result of running a [`BlockShape::LoopTaskTail`] block (Phase 4.4).
///
/// Fields mirror the emit-core wasm signature `() -> (exit_code,
/// next_pc, l32r_at_value)`. The dispatcher writes `l32r_at_value` into
/// the AR named by [`BlockShape::LoopTaskTail::l32r_at`] and sets
/// `cpu.pc = next_pc`.
pub struct LoopTaskTailResult {
    pub exit_code: i32,
    pub next_pc: u32,
    pub l32r_at_value: u32,
}

impl MultiOpBlock {
    /// Build the hot BB module + instance. Walks the BB via
    /// [`emit_core::walk_and_emit`] to fetch the bytes, then hands them
    /// to wasmtime. Failure path bubbles wasmtime errors; the caller
    /// logs and falls back to interpreter.
    ///
    /// The walker is supplied with the pre-baked [`HOT_BB_WASM`] bytes
    /// indirectly via [`emit_core::walk_and_emit`] which currently
    /// recognises only the canonical hot-BB shape. We synthesise an
    /// in-memory bus slice from the canonical disassembly (so the
    /// emit-core call path is exercised end-to-end) — this matches the
    /// `bb_multi` invariants and re-uses the build-time-baked wasm
    /// bytes byte-for-byte.
    pub fn build_hot_bb(engine: &Engine) -> wasmtime::Result<Self> {
        // Canonical disassembly bytes for the 8-instruction hot block.
        // Encoded little-endian byte-stream: each 3-byte group is the
        // reverse of the wide-instruction word value objdump prints.
        // E.g. `20a550 or a10, a5, a5` decodes to word=0x00_20_a5_50 and
        // sits in memory as bytes 0x50, 0xa5, 0x20. Pinning the canonical
        // byte stream here lets the emit-core path validate end-to-end
        // without round-tripping through the Bus trait at JIT-cache
        // build time. The `xtensa_lx7::try_jit_multi_op` path uses the
        // real bus and is covered by the lockstep harness.
        const HOT_BB_BYTES: &[u8] = &[
            0x50, 0xa5, 0x20, // or    a10, a5, a5    (word 0x20a550)
            0xc0, 0x20, 0x00, // memw                  (word 0x0020c0)
            0x62, 0x03, 0x00, // l8ui  a6,  a3, 0      (word 0x000362)
            0xc0, 0x20, 0x00, // memw                  (word 0x0020c0)
            0x22, 0x03, 0x01, // l8ui  a2,  a3, 1      (word 0x010322)
            0x20, 0x20, 0x74, // extui a2,  a2, 0, 8   (word 0x742020)
            0x60, 0x22, 0x10, // and   a2,  a2, a6     (word 0x102260)
            0x81, 0xd4, 0xf6, // l32r  a8,  0x40080534 (word 0xf6d481)
            0xe0, 0x08, 0x00, // callx8 a8 — terminator (word 0x0008e0)
        ];

        let emitted = emit_core::walk_and_emit(
            HOT_BB_BYTES,
            HOT_BB_PC,
            |pc| {
                let off = pc.wrapping_sub(HOT_BB_PC) as usize;
                if off < HOT_BB_BYTES.len() {
                    Some(off)
                } else {
                    None
                }
            },
            PsBits::default(),
        )
        .map_err(|e| wasmtime::Error::msg(format!("emit_core walk_and_emit: {e}")))?;

        Self::build_from_emitted(engine, emitted)
    }

    /// Build the loopTask prefix block (`L32R → L8UI → BEQZ` at
    /// [`LOOPTASK_PC`]). Phase 4.3.6 wires this into
    /// [`super::JitCache::lookup_or_install_multi_op`] alongside the
    /// canonical hot block. The bus bytes are synthesised from the same
    /// canonical disassembly the emit-core lockstep test exercises so
    /// the cache-build path doesn't require a live `Bus` at construction
    /// time (mirrors `build_hot_bb`'s design).
    pub fn build_looptask_prefix(engine: &Engine) -> wasmtime::Result<Self> {
        // Synthetic slice layout: 4 bytes of literal pool, 1 byte pad,
        // then four 3-byte Xtensa instructions. The L32R encoding uses
        // imm16=0xFFFE which the decoder turns into
        // pc_rel_byte_offset = -8, i.e. EA = ((LOOPTASK_PC+3) & ~3) - 8
        // = LOOPTASK_PC - 5 (=`0x400d_4a88`). The 4-byte literal at
        // PC-5..PC-1 does NOT overlap the L32R instruction at PC..PC+3
        // (1 byte unused gap at PC-1).
        const LITERAL_POOL_VALUE: u32 = 0x4008_0534;
        let literal_le = LITERAL_POOL_VALUE.to_le_bytes();
        let mut bus_slice: Vec<u8> = literal_le.to_vec();
        bus_slice.push(0); // 1-byte gap at PC-1 (not read)
        bus_slice.extend_from_slice(&[
            0x21, 0xFE, 0xFF, // L32R a2, ... (imm16=0xFFFE → EA = PC-5)
            0x32, 0x02, 0x00, // L8UI a3, a2, 0
            0x16, 0x83, 0x00, // BEQZ a3, +12
            0x25, 0x00, 0x00, // CALL8 (terminator, not emitted)
        ]);
        // slice_base_pc chosen so that
        //   pc_to_offset(LOOPTASK_PC - 5) = 0  (literal at slice start)
        //   pc_to_offset(LOOPTASK_PC)     = 5  (instructions after lit+gap)
        let slice_base_pc = LOOPTASK_PC.wrapping_sub(5);

        let emitted = emit_core::walk_and_emit(
            &bus_slice,
            LOOPTASK_PC,
            |pc| {
                let off = pc.wrapping_sub(slice_base_pc) as usize;
                if off < bus_slice.len() {
                    Some(off)
                } else {
                    None
                }
            },
            PsBits::default(),
        )
        .map_err(|e| wasmtime::Error::msg(format!("emit_core walk_and_emit: {e}")))?;

        Self::build_from_emitted(engine, emitted)
    }

    /// Build the loopTask tail block (`L32R → BEQZ` at
    /// [`LOOPTASK_TAIL_PC`]). Synthesizes the canonical 0x400d4a9c byte
    /// stream the ereader firmware exposes (matching the disasm in
    /// `xtensa_jit_bytes`) so the cache builder doesn't need a live
    /// `Bus`. The shape recognizer constant-folds both the L32R literal
    /// and the BEQZ direction, so the resulting wasm body is a fixed
    /// return tuple.
    pub fn build_looptask_tail(engine: &Engine) -> wasmtime::Result<Self> {
        // Canonical bytes: L32R a8, &serialEventRun ; BEQZ a8, loopTask_top.
        // The L32R pc_rel_byte_offset is computed from the real
        // disassembly (imm16 = 0xFEED → ea = ((PC+3) & ~3) + (-72) ≈ ...).
        // We synthesize the slice with a literal pool at PC-32 that holds
        // `serialEventRun = NULL` so the recognizer constant-folds BEQZ
        // as statically-taken (the steady-state ereader case).
        //
        // imm16 encoding for L32R: pc_rel_byte_offset = (imm16 -
        // 0x10000) * 4 (since bit 15 is set for backward refs). For
        // pc_rel_byte_offset = -32 → imm16 = 0xFFF8. L32R word layout:
        // imm16<<8 | at<<4 | opcode → 0xFFF8<<8 | 8<<4 | 0x1 = 0xFFF881.
        // Little-endian bytes: [0x81, 0xF8, 0xFF].
        //
        // BEQZ a8, +offset: target = LOOPTASK_PC (back to the prefix
        // start, which sits at LOOPTASK_TAIL_PC - 15). The decoder bakes
        // +4 into its stored offset, so the offset we carry is
        // target - after_l32r_pc = LOOPTASK_PC - (LOOPTASK_TAIL_PC + 3) =
        // -18. The encoded imm12 satisfies signext(imm12)+4 == -18 →
        // imm12 = -22 = 0xFEA. BEQZ word layout: imm12<<12 | as_<<8 |
        // 0x16 = 0xFEA816. Little-endian bytes: [0x16, 0xA8, 0xFE].
        let mut bus_slice: Vec<u8> = Vec::new();
        // Literal pool at PC-32: 4 bytes of zero (= NULL pointer, the
        // steady-state value of `serialEventRun`).
        const LITERAL_POOL_VALUE: u32 = 0;
        for _ in 0..32 {
            bus_slice.push(0);
        }
        // Overwrite the 4 bytes at offset 0 with the literal pool value
        // (still all zero here — kept explicit for symmetry with the
        // prefix builder).
        for (i, b) in LITERAL_POOL_VALUE.to_le_bytes().iter().enumerate() {
            bus_slice[i] = *b;
        }
        // Instructions at LOOPTASK_TAIL_PC.
        bus_slice.extend_from_slice(&[
            0x81, 0xF8, 0xFF, // L32R a8, ... (pc_rel_byte_offset = -32 → EA = TAIL_PC - 32)
            0x16, 0xA8, 0xFE, // BEQZ a8, LOOPTASK_PC
            0x65, 0x3C, 0xFE, // CALL8 (terminator, never executed by JIT body)
            0x06, 0xF9, 0xFF, // J (terminator, never executed by JIT body)
        ]);
        // slice_base_pc: pc_to_offset(LOOPTASK_TAIL_PC - 32) == 0.
        let slice_base_pc = LOOPTASK_TAIL_PC.wrapping_sub(32);

        let emitted = emit_core::walk_and_emit(
            &bus_slice,
            LOOPTASK_TAIL_PC,
            |pc| {
                let off = pc.wrapping_sub(slice_base_pc) as usize;
                if off < bus_slice.len() {
                    Some(off)
                } else {
                    None
                }
            },
            PsBits::default(),
        )
        .map_err(|e| wasmtime::Error::msg(format!("emit_core walk_and_emit: {e}")))?;

        Self::build_from_emitted(engine, emitted)
    }

    /// Compile + instantiate a wasmtime block from a pre-emitted byte
    /// stream. Phase 4.2's browser adapter has a structurally
    /// identical entry point on its side; both consume `EmittedBlock`
    /// verbatim. Shape-aware result decoding lives in [`Self::run_hot_bb`]
    /// and [`Self::run_looptask_prefix`] — pick the one that matches
    /// `self.emitted.shape`.
    pub fn build_from_emitted(engine: &Engine, emitted: EmittedBlock) -> wasmtime::Result<Self> {
        let module = Module::new(engine, &emitted.wasm_bytes)?;
        let mut store: Store<()> = Store::new(engine, ());

        let pending_loads: std::sync::Arc<Mutex<Vec<u32>>> =
            std::sync::Arc::new(Mutex::new(Vec::with_capacity(2)));
        let scratch: std::sync::Arc<Mutex<ScratchSlot>> =
            std::sync::Arc::new(Mutex::new(ScratchSlot::default()));

        // host.read_u8 is only wired for shapes that perform L8UI in the
        // wasm body. The Phase 4.4 [`BlockShape::LoopTaskTail`] returns a
        // pre-baked constant tuple — no imports needed.
        let needs_read_u8 = !matches!(emitted.shape, BlockShape::LoopTaskTail { .. });

        let instance = if needs_read_u8 {
            let pending_for_import = pending_loads.clone();
            let scratch_for_import = scratch.clone();
            // host.read_u8(addr): the host had already pre-staged byte
            // values into `pending_loads` (one per L8UI in BB order). The
            // import pops the next value and returns it. If
            // `pending_loads` is empty when called, we report a bus error
            // so wasm bails cleanly.
            let read_u8: Func = Func::wrap(&mut store, move |_addr: i32| -> i32 {
                let mut p = pending_for_import.lock().unwrap();
                if p.is_empty() {
                    scratch_for_import.lock().unwrap().bus_error = true;
                    return -1;
                }
                let v = p.remove(0);
                v as i32
            });
            Instance::new(&mut store, &module, &[read_u8.into()])?
        } else {
            Instance::new(&mut store, &module, &[])?
        };
        let run = instance.get_func(&mut store, "run").ok_or_else(|| {
            wasmtime::Error::msg("emit-core wasm module missing required `run` export")
        })?;
        Ok(Self {
            store,
            run,
            scratch,
            hits: 0,
            pending_loads,
            emitted,
        })
    }

    /// Stage `bytes` into the host-side queue. The wasm body's import
    /// calls dequeue these in order. `bytes.len()` must equal the number
    /// of L8UI ops in the BB (2 for the hot BB, 1 for the loopTask
    /// prefix).
    pub fn stage_loads(&mut self, bytes: &[u8]) {
        let mut p = self.pending_loads.lock().unwrap();
        p.clear();
        p.extend(bytes.iter().map(|b| *b as u32));
        let mut s = self.scratch.lock().unwrap();
        s.bytes.clear();
        s.bus_error = false;
    }

    /// Shape tag for this block. Mirror of `self.emitted.shape`; callers
    /// branch on this to pick the right `run_*` variant.
    #[inline]
    pub fn shape(&self) -> BlockShape {
        self.emitted.shape
    }

    /// Invoke a [`BlockShape::HotBbCanonical`] block. Panics in debug
    /// builds if the block's shape isn't HotBbCanonical — production
    /// callers should branch on [`Self::shape`] first.
    #[inline]
    pub fn run_hot_bb(
        &mut self,
        a3: u32,
        a5: u32,
        l32r_val: u32,
    ) -> wasmtime::Result<MultiOpResult> {
        debug_assert_eq!(self.emitted.shape, BlockShape::HotBbCanonical);
        use wasmtime::Val;
        let params = [
            Val::I32(a3 as i32),
            Val::I32(a5 as i32),
            Val::I32(l32r_val as i32),
        ];
        let mut results = [Val::I32(0); 5];
        self.run.call(&mut self.store, &params, &mut results)?;
        self.hits += 1;
        Ok(MultiOpResult {
            exit_code: results[0].i32().unwrap_or(0),
            a2: results[1].i32().unwrap_or(0) as u32,
            a6: results[2].i32().unwrap_or(0) as u32,
            a8: results[3].i32().unwrap_or(0) as u32,
            a10: results[4].i32().unwrap_or(0) as u32,
        })
    }

    /// Back-compat shim for [`Self::run_hot_bb`]. Existing call sites
    /// (`xtensa_lx7::try_jit_multi_op`, prior tests) used the bare
    /// `run` name; keep that working so this commit stays focused on
    /// the dispatch refactor.
    #[inline]
    pub fn run(&mut self, a3: u32, a5: u32, l32r_val: u32) -> wasmtime::Result<MultiOpResult> {
        self.run_hot_bb(a3, a5, l32r_val)
    }

    /// Invoke a [`BlockShape::LoopTaskPrefix`] block. Caller stages the
    /// single L8UI byte via [`Self::stage_loads`] beforehand.
    /// Signature: `(l32r_val, l8ui_base) -> (exit, next_pc, l8ui_at,
    /// beqz_target_pc)`.
    #[inline]
    pub fn run_looptask_prefix(
        &mut self,
        l32r_val: u32,
        l8ui_base: u32,
    ) -> wasmtime::Result<LoopTaskPrefixResult> {
        debug_assert!(matches!(
            self.emitted.shape,
            BlockShape::LoopTaskPrefix { .. }
        ));
        use wasmtime::Val;
        let params = [Val::I32(l32r_val as i32), Val::I32(l8ui_base as i32)];
        let mut results = [Val::I32(0); 4];
        self.run.call(&mut self.store, &params, &mut results)?;
        self.hits += 1;
        Ok(LoopTaskPrefixResult {
            exit_code: results[0].i32().unwrap_or(0),
            next_pc: results[1].i32().unwrap_or(0) as u32,
            l8ui_at_value: results[2].i32().unwrap_or(0) as u32,
            beqz_target_pc: results[3].i32().unwrap_or(0) as u32,
        })
    }

    /// Invoke a [`BlockShape::LoopTaskTail`] block. Signature: `() ->
    /// (exit, next_pc, l32r_at_value)`. The caller commits
    /// `l32r_at_value` into the AR named by the shape descriptor's
    /// `l32r_at` and advances PC to `next_pc`.
    #[inline]
    pub fn run_looptask_tail(&mut self) -> wasmtime::Result<LoopTaskTailResult> {
        debug_assert!(matches!(
            self.emitted.shape,
            BlockShape::LoopTaskTail { .. }
        ));
        use wasmtime::Val;
        let params: [Val; 0] = [];
        let mut results = [Val::I32(0); 3];
        self.run.call(&mut self.store, &params, &mut results)?;
        self.hits += 1;
        Ok(LoopTaskTailResult {
            exit_code: results[0].i32().unwrap_or(0),
            next_pc: results[1].i32().unwrap_or(0) as u32,
            l32r_at_value: results[2].i32().unwrap_or(0) as u32,
        })
    }

    /// Classify a wire `exit_code` into the runtime-agnostic
    /// [`SideExitReason`] vocabulary. Useful for tests + Phase 4.2
    /// side-exit handling.
    pub fn classify_exit(&self, exit_code: i32) -> Option<SideExitReason> {
        self.emitted.reason_for(exit_code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the hot-BB JIT, stage two byte values, and verify the
    /// arithmetic matches the interpreter exactly.
    #[test]
    fn hot_bb_arithmetic_matches_interp() {
        let engine = Engine::default();
        let mut block = MultiOpBlock::build_hot_bb(&engine).expect("compile");

        // Stage two bytes: mem8[a3+0] = 0xAB, mem8[a3+1] = 0xCD.
        block.stage_loads(&[0xAB, 0xCD]);

        let res = block
            .run(
                /*a3*/ 0x3FFB_0000,
                /*a5*/ 0x1234,
                /*l32r*/ 0x40008534,
            )
            .expect("wasm call");

        assert_eq!(res.exit_code, EXIT_FALL_THROUGH);
        // a10 = a5
        assert_eq!(res.a10, 0x1234);
        // a6 = mem8[a3+0] = 0xAB
        assert_eq!(res.a6, 0xAB);
        // a2 = mem8[a3+1] & 0xFF & a6 = 0xCD & 0xAB = 0x89
        assert_eq!(res.a2, 0xCD & 0xAB);
        // a8 = pre-resolved L32R literal
        assert_eq!(res.a8, 0x40008534);

        assert_eq!(block.hits, 1);
    }

    /// Block's `emitted` metadata matches the architectural constants
    /// (Phase 4.1 sanity for the emit-core handoff).
    #[test]
    fn hot_bb_emitted_metadata_matches() {
        let engine = Engine::default();
        let block = MultiOpBlock::build_hot_bb(&engine).expect("compile");
        assert_eq!(block.emitted.length_in_instrs, HOT_BB_INSTR_COUNT);
        assert_eq!(block.emitted.end_pc, HOT_BB_END);
        assert_eq!(
            block.classify_exit(EXIT_FALL_THROUGH),
            Some(SideExitReason::FallThrough)
        );
        assert_eq!(
            block.classify_exit(EXIT_HOST_BUS_ERROR),
            Some(SideExitReason::HostBusError)
        );
    }

    /// If the caller doesn't stage enough bytes, the host import returns
    /// -1 and the block exits with EXIT_HOST_BUS_ERROR — no register
    /// commits.
    #[test]
    fn hot_bb_unstaged_bytes_signals_bus_error() {
        let engine = Engine::default();
        let mut block = MultiOpBlock::build_hot_bb(&engine).expect("compile");
        // Stage zero bytes — the first L8UI's host import will trip.
        block.stage_loads(&[]);
        let res = block
            .run(0x3FFB_0000, 0x1234, 0x40008534)
            .expect("wasm call");
        assert_eq!(res.exit_code, EXIT_HOST_BUS_ERROR);
    }

    /// Phase 4.3.6: `build_looptask_prefix` produces a runnable block
    /// tagged with the right shape, and dispatching it both ways
    /// (BEQZ taken vs not taken) updates `next_pc` per emit-core's
    /// canonical loopTask prefix decode.
    #[test]
    fn looptask_prefix_dispatches_taken_and_not_taken() {
        let engine = Engine::default();
        let mut block = MultiOpBlock::build_looptask_prefix(&engine).expect("compile");
        // Shape tag is what the cache key consumer dispatches on.
        match block.shape() {
            BlockShape::LoopTaskPrefix {
                l32r_at,
                l8ui_at,
                l8ui_base_at,
                l32r_literal_value: _,
                beqz_target_pc: _,
            } => {
                // Per the canonical bytes pinned in `build_looptask_prefix`:
                // L32R writes a2, L8UI writes a3, L8UI base = a2.
                assert_eq!(l32r_at, 2);
                assert_eq!(l8ui_at, 3);
                assert_eq!(l8ui_base_at, 2);
            }
            other => panic!("expected LoopTaskPrefix shape, got {other:?}"),
        }
        assert_eq!(block.emitted.length_in_instrs, LOOPTASK_PREFIX_INSTR_COUNT);
        assert_eq!(block.emitted.end_pc, LOOPTASK_PREFIX_END);

        // L8UI returns 0 → BEQZ taken. next_pc == beqz_target_pc ==
        // after_l8ui_pc(LOOPTASK_PC + 6) + offset(12).
        let expected_target = LOOPTASK_PC + 6 + 12;
        block.stage_loads(&[0x00]);
        let res = block
            .run_looptask_prefix(/* l32r_val */ 0x4008_0534, /* l8ui_base */ 0x3FFB_0000)
            .expect("wasm call");
        assert_eq!(res.exit_code, EXIT_BRANCH_TAKEN);
        assert_eq!(res.next_pc, expected_target);
        assert_eq!(res.l8ui_at_value, 0);
        assert_eq!(res.beqz_target_pc, expected_target);

        // L8UI returns non-zero → BEQZ falls through. next_pc ==
        // LOOPTASK_PREFIX_END; l8ui_at carries the loaded byte.
        block.stage_loads(&[0x7A]);
        let res = block
            .run_looptask_prefix(0x4008_0534, 0x3FFB_0000)
            .expect("wasm call");
        assert_eq!(res.exit_code, EXIT_FALL_THROUGH);
        assert_eq!(res.next_pc, LOOPTASK_PREFIX_END);
        assert_eq!(res.l8ui_at_value, 0x7A);

        // Two successful calls → two hits.
        assert_eq!(block.hits, 2);
    }

    // ── Phase 4.3.7 dispatcher lockstep tests ────────────────────────
    //
    // The JIT body returned by `run_looptask_prefix` was validated in
    // isolation by `looptask_prefix_dispatches_taken_and_not_taken`.
    // What's NEW in 4.3.7 is the I/O staging in `try_jit_step` (native
    // side): reading the L8UI base register from the AR file, pre-
    // reading the L8UI byte through the live bus, dispatching, and
    // committing the L32R + L8UI dst writes plus the new PC. These
    // tests drive a synthetic `XtensaLx7` + MockBus through the JIT
    // path and the pure-interp path independently, diffing the
    // resulting (pc, AR) state. Any divergence means the staging
    // wired the wrong register or skipped a writeback.

    /// Minimal in-memory `Bus` for dispatcher tests. Maps `HashMap<u64,
    /// u8>` for byte-granular access; word reads fall back to the
    /// default `Bus::read_u32` (4 byte loads).
    #[cfg(test)]
    struct DispatcherTestBus {
        mem: std::collections::HashMap<u64, u8>,
        config: crate::SimulationConfig,
    }

    #[cfg(test)]
    impl DispatcherTestBus {
        fn new() -> Self {
            Self {
                mem: std::collections::HashMap::new(),
                config: crate::SimulationConfig::default(),
            }
        }
        fn write_byte(&mut self, addr: u32, v: u8) {
            self.mem.insert(addr as u64, v);
        }
        fn write_word_le(&mut self, addr: u32, v: u32) {
            for i in 0..4 {
                self.mem.insert((addr + i) as u64, (v >> (i * 8)) as u8);
            }
        }
    }

    #[cfg(test)]
    impl crate::Bus for DispatcherTestBus {
        fn read_u8(&self, addr: u64) -> crate::SimResult<u8> {
            Ok(*self.mem.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> crate::SimResult<()> {
            self.mem.insert(addr, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[crate::DmaRequest]) -> crate::SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &crate::SimulationConfig {
            &self.config
        }
    }

    /// Layout a memory image for the loopTask prefix at `LOOPTASK_PC`
    /// matching the synthetic byte stream `build_looptask_prefix`
    /// hardcodes (so the JIT-side constant-folded L32R literal matches
    /// what the interpreter reads from the live bus).
    ///
    /// `l8ui_target_byte` is the byte the L8UI reads from the literal
    /// pool's resolved value (which is also the L8UI base address —
    /// the L32R loads a pointer, the L8UI dereferences it).
    #[cfg(test)]
    fn populate_looptask_image(bus: &mut DispatcherTestBus, l8ui_target_byte: u8) {
        const LITERAL_POOL_VALUE: u32 = 0x4008_0534;
        // Literal pool: L32R imm16=0xFFFE → pc_rel_byte_offset = -8 →
        // EA = ((PC+3) & !3) - 8 = PC - 5. The 4-byte literal occupies
        // [PC-5..PC-1).
        bus.write_word_le(LOOPTASK_PC.wrapping_sub(5), LITERAL_POOL_VALUE);
        // Instructions at LOOPTASK_PC: L32R, L8UI, BEQZ, then a
        // terminator (CALL8 — never executed by the JIT body but
        // present so the walker's terminator predicate fires cleanly
        // if the interpreter ever decodes it).
        let instrs: &[u8] = &[
            0x21, 0xFE, 0xFF, // L32R a2, ... (imm16=0xFFFE → EA = PC-5)
            0x32, 0x02, 0x00, // L8UI a3, a2, 0
            0x16, 0x83, 0x00, // BEQZ a3, +12
            0x25, 0x00, 0x00, // CALL8 (terminator; not exercised)
        ];
        for (i, b) in instrs.iter().enumerate() {
            bus.write_byte(LOOPTASK_PC + i as u32, *b);
        }
        // L8UI dereferences the literal pool value as a pointer.
        bus.write_byte(LITERAL_POOL_VALUE, l8ui_target_byte);
    }

    /// Run the loopTask prefix at `LOOPTASK_PC` through either the JIT
    /// (`jit_enabled=true`) or the pure interpreter (`jit_enabled=false`)
    /// until PC leaves the prefix range, then return the post-state
    /// `(pc, a2, a3)`. The JIT executes the whole prefix in one
    /// `step()` call; the interpreter needs three (one per Xtensa
    /// instruction). The post-state diff is what tells us the
    /// dispatcher staging matches the interpreter byte-for-byte.
    #[cfg(test)]
    fn run_looptask_once(jit_enabled: bool, l8ui_byte: u8) -> (u32, u32, u32) {
        use crate::cpu::XtensaLx7;
        use crate::Cpu;
        let observers: Vec<std::sync::Arc<dyn crate::SimulationObserver>> = Vec::new();
        let config = crate::SimulationConfig::default();

        let mut cpu = XtensaLx7::new();
        cpu.jit_enabled = jit_enabled;
        cpu.pc = LOOPTASK_PC;
        // Wake the CPU from its reset EXCM=1 so the interp path doesn't
        // chase exception vectors — we're testing one block in isolation.
        cpu.ps = crate::cpu::xtensa_regs::Ps::from_raw(0);

        let mut bus = DispatcherTestBus::new();
        populate_looptask_image(&mut bus, l8ui_byte);

        // Step until PC leaves the prefix range. Cap at 8 steps to bound
        // a runaway: interp needs 3 (L32R + L8UI + BEQZ), JIT needs 1.
        for _ in 0..8 {
            let pc = cpu.pc;
            if pc < LOOPTASK_PC || pc >= LOOPTASK_PREFIX_END {
                break;
            }
            cpu.step(&mut bus, &observers, &config).expect("step ok");
        }

        (cpu.pc, cpu.regs.read_logical(2), cpu.regs.read_logical(3))
    }

    /// Lockstep: BEQZ-taken path (L8UI loads 0x00). JIT and interp must
    /// agree on post-state PC + AR2 (L32R literal) + AR3 (L8UI byte).
    /// This exercises the full Phase 4.3.7 dispatcher wiring: L8UI base
    /// register read from the AR file, L32R literal constant-folded at
    /// compile time, both writes committed, PC set to the BEQZ target.
    #[test]
    fn looptask_prefix_jit_matches_interp_branch_taken() {
        let (jit_pc, jit_a2, jit_a3) = run_looptask_once(true, 0x00);
        let (int_pc, int_a2, int_a3) = run_looptask_once(false, 0x00);
        assert_eq!(jit_pc, int_pc, "PC mismatch: jit={jit_pc:#x} interp={int_pc:#x}");
        assert_eq!(jit_a2, int_a2, "a2 mismatch");
        assert_eq!(jit_a3, int_a3, "a3 mismatch");
        // Architectural truth: BEQZ taken targets LOOPTASK_PC + 6 + 12.
        assert_eq!(jit_pc, LOOPTASK_PC + 6 + 12);
        // a2 = L32R literal value (constant-folded into the JIT body).
        assert_eq!(jit_a2, 0x4008_0534);
        // a3 = the byte the L8UI loaded.
        assert_eq!(jit_a3, 0x00);
    }

    /// Lockstep: BEQZ-not-taken path (L8UI loads non-zero). JIT and
    /// interp must agree on post-state PC + AR2 + AR3. The PC after a
    /// fall-through is `LOOPTASK_PREFIX_END`, the byte after BEQZ.
    #[test]
    fn looptask_prefix_jit_matches_interp_branch_not_taken() {
        let (jit_pc, jit_a2, jit_a3) = run_looptask_once(true, 0x7A);
        let (int_pc, int_a2, int_a3) = run_looptask_once(false, 0x7A);
        assert_eq!(jit_pc, int_pc, "PC mismatch: jit={jit_pc:#x} interp={int_pc:#x}");
        assert_eq!(jit_a2, int_a2, "a2 mismatch");
        assert_eq!(jit_a3, int_a3, "a3 mismatch");
        // Architectural truth: BEQZ not taken falls through to PC+3
        // (BEQZ is 3 bytes); the interp single-steps the BEQZ so its
        // post-state PC is also LOOPTASK_PC + 6 + 3 = LOOPTASK_PREFIX_END.
        assert_eq!(jit_pc, LOOPTASK_PREFIX_END);
        assert_eq!(jit_a2, 0x4008_0534);
        assert_eq!(jit_a3, 0x7A);
    }

    /// Phase 4.3.6: `JitCache::lookup_or_install_multi_op` returns a
    /// working block for both HOT_BB_PC and LOOPTASK_PC. This is the
    /// cache-level contract that wires shape-aware dispatch into the
    /// rest of the engine. We verify by checking the shape tag is
    /// correct for each PC; the actual run paths are covered by
    /// `hot_bb_arithmetic_matches_interp` and
    /// `looptask_prefix_dispatches_taken_and_not_taken`.
    #[test]
    fn jit_cache_lookup_dispatches_both_shapes() {
        use crate::cpu::xtensa_jit::JitCache;
        let mut cache = JitCache::new();

        // HOT_BB_PC installs and reports HotBbCanonical.
        let hot = cache
            .lookup_or_install_multi_op(HOT_BB_PC)
            .expect("hot bb installs");
        assert_eq!(hot.shape(), BlockShape::HotBbCanonical);

        // LOOPTASK_PC installs and reports LoopTaskPrefix.
        let lt = cache
            .lookup_or_install_multi_op(LOOPTASK_PC)
            .expect("loopTask prefix installs");
        assert!(
            matches!(lt.shape(), BlockShape::LoopTaskPrefix { .. }),
            "loopTask block must be LoopTaskPrefix-shaped, got {:?}",
            lt.shape()
        );

        // LOOPTASK_TAIL_PC installs and reports LoopTaskTail (Phase 4.4).
        let tail = cache
            .lookup_or_install_multi_op(LOOPTASK_TAIL_PC)
            .expect("loopTask tail installs");
        assert!(
            matches!(tail.shape(), BlockShape::LoopTaskTail { .. }),
            "loopTask tail must be LoopTaskTail-shaped, got {:?}",
            tail.shape()
        );

        // An unknown PC still refuses (cache stays narrow).
        assert!(cache.lookup_or_install_multi_op(0xDEAD_BEEF).is_none());
    }

    // ── Phase 4.4 loopTask tail dispatch tests ───────────────────────
    //
    // The tail block (`L32R → BEQZ` at LOOPTASK_TAIL_PC) is the
    // counterpart to the prefix block. Both halves of the BEQZ are
    // statically resolved at walk_and_emit time against the live bus's
    // literal pool, so the wasm body is a constant return tuple. These
    // tests exercise the dispatcher end-to-end through `Cpu::step` so
    // we catch any divergence between the JIT path and the pure-interp
    // path on either branch direction.

    /// Layout a memory image for the loopTask tail block. The L32R
    /// loads from a literal pool at PC-32; `literal` is the value
    /// stored there (controls whether BEQZ is statically taken).
    #[cfg(test)]
    fn populate_looptask_tail_image(bus: &mut DispatcherTestBus, literal: u32) {
        // Literal pool at PC-32 (matches the synthesized fixture in
        // `build_looptask_tail`).
        bus.write_word_le(LOOPTASK_TAIL_PC.wrapping_sub(32), literal);
        let instrs: &[u8] = &[
            0x81, 0xF8, 0xFF, // L32R a8, ... (EA = PC - 32)
            0x16, 0xA8, 0xFE, // BEQZ a8, LOOPTASK_PC
            0x65, 0x3C, 0xFE, // CALL8 (terminator; not exercised)
            0x06, 0xF9, 0xFF, // J (terminator; not exercised)
        ];
        for (i, b) in instrs.iter().enumerate() {
            bus.write_byte(LOOPTASK_TAIL_PC + i as u32, *b);
        }
    }

    /// Run the loopTask tail at `LOOPTASK_TAIL_PC` through either the
    /// JIT or the pure interpreter until PC leaves the tail range,
    /// then return `(pc, a8)`. The JIT executes the whole tail in one
    /// step; the interpreter needs two.
    #[cfg(test)]
    fn run_looptask_tail_once(jit_enabled: bool, literal: u32) -> (u32, u32) {
        use crate::cpu::XtensaLx7;
        use crate::Cpu;
        let observers: Vec<std::sync::Arc<dyn crate::SimulationObserver>> = Vec::new();
        let config = crate::SimulationConfig::default();

        let mut cpu = XtensaLx7::new();
        cpu.jit_enabled = jit_enabled;
        cpu.pc = LOOPTASK_TAIL_PC;
        cpu.ps = crate::cpu::xtensa_regs::Ps::from_raw(0);

        let mut bus = DispatcherTestBus::new();
        populate_looptask_tail_image(&mut bus, literal);

        // Step until PC leaves the tail range. Cap at 8 to bound a
        // runaway: interp needs 2, JIT needs 1. When BEQZ takes we
        // land at LOOPTASK_PC (outside the tail range — loop top).
        for _ in 0..8 {
            let pc = cpu.pc;
            if pc < LOOPTASK_TAIL_PC || pc >= LOOPTASK_TAIL_END {
                break;
            }
            cpu.step(&mut bus, &observers, &config).expect("step ok");
        }
        (cpu.pc, cpu.regs.read_logical(8))
    }

    /// Lockstep: BEQZ-taken path (literal == 0). JIT and interp must
    /// agree on post-state PC + a8. PC must land at LOOPTASK_PC.
    #[test]
    fn looptask_tail_jit_matches_interp_branch_taken() {
        let (jit_pc, jit_a8) = run_looptask_tail_once(true, 0);
        let (int_pc, int_a8) = run_looptask_tail_once(false, 0);
        assert_eq!(jit_pc, int_pc, "PC mismatch jit={jit_pc:#x} int={int_pc:#x}");
        assert_eq!(jit_a8, int_a8, "a8 mismatch jit={jit_a8:#x} int={int_a8:#x}");
        assert_eq!(jit_pc, LOOPTASK_PC, "BEQZ taken should land at loopTask top");
        assert_eq!(jit_a8, 0, "L32R wrote the literal (0)");
    }

    /// Lockstep: BEQZ-not-taken path (literal != 0). JIT and interp
    /// must agree on post-state PC + a8. PC must land at
    /// LOOPTASK_TAIL_END (fall-through to CALL8 serialEventRun).
    #[test]
    fn looptask_tail_jit_matches_interp_branch_not_taken() {
        let literal = 0x400d_2e68u32; // canonical ereader: &_Z14serialEventRunv
        let (jit_pc, jit_a8) = run_looptask_tail_once(true, literal);
        let (int_pc, int_a8) = run_looptask_tail_once(false, literal);
        assert_eq!(jit_pc, int_pc, "PC mismatch jit={jit_pc:#x} int={int_pc:#x}");
        assert_eq!(jit_a8, int_a8, "a8 mismatch jit={jit_a8:#x} int={int_a8:#x}");
        assert_eq!(jit_pc, LOOPTASK_TAIL_END);
        assert_eq!(jit_a8, literal);
    }

    /// `build_looptask_tail` produces a block tagged with the right
    /// shape and dispatching it through `run_looptask_tail` yields a
    /// statically-resolved 3-tuple matching the BEQZ direction.
    #[test]
    fn looptask_tail_dispatches_constant_tuple() {
        let engine = Engine::default();
        let mut block = MultiOpBlock::build_looptask_tail(&engine).expect("compile");
        // Canonical synthesized fixture has literal == 0 → BEQZ taken.
        match block.shape() {
            BlockShape::LoopTaskTail {
                l32r_at,
                l32r_literal_value,
                statically_taken,
                ..
            } => {
                assert_eq!(l32r_at, 8, "loopTask tail L32R writes a8");
                assert_eq!(l32r_literal_value, 0);
                assert!(statically_taken);
            }
            other => panic!("expected LoopTaskTail, got {other:?}"),
        }
        let res = block.run_looptask_tail().expect("run");
        assert_eq!(res.exit_code, EXIT_BRANCH_TAKEN);
        assert_eq!(res.next_pc, LOOPTASK_PC);
        assert_eq!(res.l32r_at_value, 0);
        assert_eq!(block.hits, 1);
    }
}
