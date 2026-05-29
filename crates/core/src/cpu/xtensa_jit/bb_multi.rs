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
use wasmtime::{Engine, Func, Instance, Module, Store, TypedFunc};

use super::emit_core::{self, EmittedBlock, PsBits, SideExitReason};

// ── Side-exit codes + BB constants ─────────────────────────────────────
//
// Re-exported from `cpu::xtensa_jit_bytes` (which is always compiled,
// even without `--features jit`) so the browser-side prototype can use
// the same values without dragging wasmtime into its dep graph
// (#124 Phase 4).
pub use crate::cpu::xtensa_jit_bytes::{
    EXIT_FALL_THROUGH, EXIT_HOST_BUS_ERROR, HOT_BB_END, HOT_BB_INSTR_COUNT, HOT_BB_L32R_ADDR,
    HOT_BB_PC,
};

// Re-export the walker types from emit_core for back-compat with the
// existing public API (tests + xtensa_lx7.rs reach for these via
// `xtensa_jit::{walk_bb, DecodedOp}`).
pub use super::emit_core::{walk_bb, DecodedOp};

// ── Wasm function signature ───────────────────────────────────────────

/// Inputs: (a3, a5, l32r_value)
/// Outputs: (exit_code, a2, a6, a8, a10)
type BbParams = (i32, i32, i32);
type BbReturns = (i32, i32, i32, i32, i32);
type BbRun = TypedFunc<BbParams, BbReturns>;

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
    run: BbRun,
    scratch: std::sync::Arc<Mutex<ScratchSlot>>,
    pub hits: u64,
    /// L8UI host-import call sequence, populated by the caller before
    /// the wasm call. The wasm body indexes into this by position.
    pending_loads: std::sync::Arc<Mutex<Vec<u32>>>,
    /// The emit-core output we built this block from. Held so callers
    /// can interrogate `length_in_instrs`, `end_pc`, and the side-exit
    /// reason map without a separate registry. Phase 4.2 will read
    /// these for variable-length blocks.
    pub emitted: EmittedBlock,
}

pub struct MultiOpResult {
    pub exit_code: i32,
    pub a2: u32,
    pub a6: u32,
    pub a8: u32,
    pub a10: u32,
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

    /// Compile + instantiate a wasmtime block from a pre-emitted byte
    /// stream. Phase 4.2's browser adapter will have a structurally
    /// identical entry point on its side; both consume `EmittedBlock`
    /// verbatim.
    pub fn build_from_emitted(engine: &Engine, emitted: EmittedBlock) -> wasmtime::Result<Self> {
        let module = Module::new(engine, &emitted.wasm_bytes)?;
        let mut store: Store<()> = Store::new(engine, ());

        let pending_loads: std::sync::Arc<Mutex<Vec<u32>>> =
            std::sync::Arc::new(Mutex::new(Vec::with_capacity(2)));
        let scratch: std::sync::Arc<Mutex<ScratchSlot>> =
            std::sync::Arc::new(Mutex::new(ScratchSlot::default()));

        let pending_for_import = pending_loads.clone();
        let scratch_for_import = scratch.clone();

        // host.read_u8(addr): the host had already pre-staged byte values
        // into `pending_loads` (one per L8UI in BB order). The import
        // pops the next value and returns it. If `pending_loads` is empty
        // when called, we report a bus error so wasm bails cleanly.
        let read_u8: Func = Func::wrap(&mut store, move |_addr: i32| -> i32 {
            let mut p = pending_for_import.lock().unwrap();
            if p.is_empty() {
                scratch_for_import.lock().unwrap().bus_error = true;
                return -1;
            }
            let v = p.remove(0);
            v as i32
        });

        let instance = Instance::new(&mut store, &module, &[read_u8.into()])?;
        let run = instance.get_typed_func::<BbParams, BbReturns>(&mut store, "run")?;
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
    /// of L8UI ops in the BB (2 for the hot BB).
    pub fn stage_loads(&mut self, bytes: &[u8]) {
        let mut p = self.pending_loads.lock().unwrap();
        p.clear();
        p.extend(bytes.iter().map(|b| *b as u32));
        let mut s = self.scratch.lock().unwrap();
        s.bytes.clear();
        s.bus_error = false;
    }

    /// Invoke the compiled block. Caller has already staged loads via
    /// [`Self::stage_loads`].
    #[inline]
    pub fn run(&mut self, a3: u32, a5: u32, l32r_val: u32) -> wasmtime::Result<MultiOpResult> {
        let (exit, a2, a6, a8, a10) = self
            .run
            .call(&mut self.store, (a3 as i32, a5 as i32, l32r_val as i32))?;
        self.hits += 1;
        Ok(MultiOpResult {
            exit_code: exit,
            a2: a2 as u32,
            a6: a6 as u32,
            a8: a8 as u32,
            a10: a10 as u32,
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
}
