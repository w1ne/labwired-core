// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 JIT — multi-op basic-block emit (Phase 3.6.3 / issue #124).
//!
//! Phase 3.6.2 (#128) shipped a one-instruction CALL8 JIT and measured
//! ~6% **slower** vs baseline interpreter — proof that wasmtime call
//! overhead (~200ns/call) dwarfs the savings of replacing a single
//! interpreter dispatch (~50ns).
//!
//! This module is the fix: JIT-compile a **whole basic block** (5-10+
//! instructions) per wasm module, so one wasmtime call amortises N
//! interpreter dispatches.
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
//! * `0` — block executed cleanly to the terminator; caller commits
//!   regs (a2, a6, a8, a10) and sets PC = block-end (0x400829e4) so the
//!   interpreter picks up at the callx8.
//! * `5` — `read_u8` import returned a host bus error (signalled by
//!   pushing an error marker into the pending list). Caller falls back
//!   to interpreter for the whole block — no register or memory state
//!   visible from wasm has been committed.
//!
//! ## Why this should win when 3.6.2 didn't
//!
//! Per Phase 3.0 measurement:
//!   - Interpreter dispatch: ~50ns/instruction
//!   - Wasmtime call: ~200ns
//!
//! For an 8-instr BB:
//!   - Interpreter: 8 × 50ns = **400ns/pass**
//!   - JIT: 200ns (wasm call) + 2 × ~100ns (host imports for L8UI) +
//!     ~50ns (pure arithmetic in JITed code) = **~450ns/pass**
//!
//! That's roughly break-even per pass. But 908k hits means **even a
//! 5% per-call improvement** compounds into ~270ms on a ~5s benchmark.
//! And this block is the canary: if it doesn't speed up, no longer
//! block on this firmware will, and we'll need a different approach
//! (e.g. inlining the bus read).

#![cfg(feature = "jit")]

use std::sync::Mutex;
use wasmtime::{Engine, Func, Instance, Module, Store, TypedFunc};

use crate::decoder::xtensa::{self, Instruction};
use crate::decoder::{xtensa_length, xtensa_narrow};

// ── Side-exit codes ───────────────────────────────────────────────────

/// Block ran cleanly to terminator. Caller commits regs + advances PC.
pub const EXIT_FALL_THROUGH: i32 = 0;
/// Host L8UI import hit a bus error; caller MUST fall back to interpreter
/// and re-execute the block from the top.
pub const EXIT_HOST_BUS_ERROR: i32 = 5;

// ── Phase 3.6.3 target BB ─────────────────────────────────────────────

/// PC of the dominant hot multi-instruction BB (call_start_cpu0 delay loop).
pub const HOT_BB_PC: u32 = 0x400829cc;
/// First PC after the JITed range; the interpreter resumes here at `callx8`.
pub const HOT_BB_END: u32 = 0x400829e4;
/// Number of Xtensa instructions executed by the JIT body.
pub const HOT_BB_INSTR_COUNT: u32 = 8;
/// L32R literal address read by the block: `((0x400829e1 + 3) & ~3) + sext_offset`.
/// Resolved at compile time from the ELF; matches what the interpreter computes.
pub const HOT_BB_L32R_ADDR: u32 = 0x4008_0534;

// ── Wasm function signature ───────────────────────────────────────────

/// Inputs: (a3, a5, l32r_value)
/// Outputs: (exit_code, a2, a6, a8, a10)
type BbParams = (i32, i32, i32);
type BbReturns = (i32, i32, i32, i32, i32);
type BbRun = TypedFunc<BbParams, BbReturns>;

/// Per-call scratch slots for host imports. The L8UI import pushes
/// `Ok(u8)` or `Err(())` here; the wasm caller consumes them in order.
#[derive(Default)]
struct ScratchSlot {
    /// Byte values read from `host.read_u8`, in call order.
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
}

pub struct MultiOpResult {
    pub exit_code: i32,
    pub a2: u32,
    pub a6: u32,
    pub a8: u32,
    pub a10: u32,
}

// ── Decoded-op intermediate form ──────────────────────────────────────

/// One decoded Xtensa op + its byte length. Used by the BB walker.
#[derive(Debug, Clone)]
pub struct DecodedOp {
    pub pc: u32,
    pub len: u32,
    pub ins: Instruction,
}

/// Walk forward from `start_pc`, decoding instructions out of `text`
/// (a flat slice mapping PC → byte). Stops when:
///   * a terminator (any control transfer) is reached — terminator is
///     **excluded** from the returned vec.
///   * an unsupported opcode is hit — returns `None` (refuse the whole BB).
///   * `max_ops` instructions have been collected — returns what we have.
///
/// `pc_to_offset` converts a PC to an index into `text`; returns `None`
/// if the PC is outside `text`.
pub fn walk_bb<F>(
    start_pc: u32,
    mut pc_to_offset: F,
    text: &[u8],
    max_ops: usize,
) -> Option<Vec<DecodedOp>>
where
    F: FnMut(u32) -> Option<usize>,
{
    let mut ops = Vec::with_capacity(max_ops);
    let mut pc = start_pc;
    while ops.len() < max_ops {
        let off = pc_to_offset(pc)?;
        if off >= text.len() {
            return None;
        }
        let b0 = text[off];
        let len: u32 = xtensa_length::instruction_length(b0);
        // Verify the full instruction fits inside `text`.
        if off + (len as usize) > text.len() {
            return None;
        }
        let ins = if len == 2 {
            let hw = u16::from_le_bytes([text[off], text[off + 1]]);
            xtensa_narrow::decode_narrow(hw)
        } else if len == 3 {
            let w = u32::from_le_bytes([text[off], text[off + 1], text[off + 2], 0]);
            xtensa::decode(w)
        } else {
            return None;
        };
        if is_terminator(&ins) {
            return Some(ops);
        }
        if !is_supported(&ins) {
            return None;
        }
        ops.push(DecodedOp { pc, len, ins });
        pc = pc.wrapping_add(len);
    }
    Some(ops)
}

/// Is this opcode a basic-block terminator (control transfer)?
fn is_terminator(ins: &Instruction) -> bool {
    use Instruction::*;
    matches!(
        ins,
        Call0 { .. }
            | Call4 { .. }
            | Call8 { .. }
            | Call12 { .. }
            | Callx0 { .. }
            | Callx4 { .. }
            | Callx8 { .. }
            | Callx12 { .. }
            | Ret
            | Retw
            | Jx { .. }
            | Beq { .. }
            | Bne { .. }
            | Blt { .. }
            | Bge { .. }
            | Bltu { .. }
            | Bgeu { .. }
            | Beqz { .. }
            | Bnez { .. }
            | Bltz { .. }
            | Bgez { .. }
            | Beqi { .. }
            | Bnei { .. }
            | Blti { .. }
            | Bgei { .. }
            | Bltui { .. }
            | Bgeui { .. }
            | Bany { .. }
            | Ball { .. }
            | Bnone { .. }
            | Bnall { .. }
            | Bbc { .. }
            | Bbs { .. }
            | Bbci { .. }
            | Bbsi { .. }
            | Entry { .. }
            | Rfe
            | Rfde
            | Rfi { .. }
            | Rfwo
            | Rfwu
            | Ill
    )
}

/// Is this opcode in the Phase 3.6.3 supported set?
///
/// Keep this list narrow: any new opcode here needs corresponding emit
/// code in [`emit_wat_for_block`]. Adding more is a Phase 3.6.4+ task.
fn is_supported(ins: &Instruction) -> bool {
    use Instruction::*;
    matches!(
        ins,
        // Pure arithmetic / bitwise
        Add { .. }
            | Sub { .. }
            | And { .. }
            | Or { .. }
            | Xor { .. }
            | Addi { .. }
            | Movi { .. }
            | Extui { .. }
            // Loads
            | L8ui { .. }
            | L32r { .. }
            // Barriers — semantic no-ops in sim
            | Memw
            | Nop
    )
}

// ── WAT emit for the target block ─────────────────────────────────────

/// Emit the WAT body for the 0x400829cc block. This is hand-written for
/// the specific opcode sequence; future Phase 3.6.4 work will generalise
/// `emit_op_wat` to walk an arbitrary `DecodedOp` slice.
///
/// The wasm module exports `run(a3: i32, a5: i32, l32r_val: i32) ->
/// (exit_code, a2, a6, a8, a10)`. It calls `host.read_u8(addr)` twice
/// for the L8UI ops. If either returns -1, exit code is HOST_BUS_ERROR.
fn emit_hot_bb_wat() -> String {
    format!(
        r#"(module
  (import "host" "read_u8" (func $read_u8 (param i32) (result i32)))
  (func (export "run")
        (param $a3 i32) (param $a5 i32) (param $l32r i32)
        (result i32 i32 i32 i32 i32)
    (local $a2 i32)
    (local $a6 i32)
    (local $a8 i32)
    (local $a10 i32)
    (local $tmp i32)

    ;; 1. or a10, a5, a5  -> a10 = a5
    (local.set $a10 (local.get $a5))

    ;; 2. memw — barrier, semantic no-op in sim

    ;; 3. l8ui a6, a3, 0  -> a6 = read_u8(a3 + 0)
    (local.set $tmp (call $read_u8 (local.get $a3)))
    (if (i32.lt_s (local.get $tmp) (i32.const 0))
      (then
        (return (i32.const {bus_err}) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0))))
    (local.set $a6 (i32.and (local.get $tmp) (i32.const 0xFF)))

    ;; 4. memw — barrier

    ;; 5. l8ui a2, a3, 1  -> a2 = read_u8(a3 + 1)
    (local.set $tmp (call $read_u8 (i32.add (local.get $a3) (i32.const 1))))
    (if (i32.lt_s (local.get $tmp) (i32.const 0))
      (then
        (return (i32.const {bus_err}) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0))))
    (local.set $a2 (i32.and (local.get $tmp) (i32.const 0xFF)))

    ;; 6. extui a2, a2, 0, 8  -> a2 = (a2 >> 0) & ((1<<8) - 1) = a2 & 0xFF
    ;;    (already byte after l8ui, but emit for correctness)
    (local.set $a2 (i32.and (local.get $a2) (i32.const 0xFF)))

    ;; 7. and a2, a2, a6  -> a2 &= a6
    (local.set $a2 (i32.and (local.get $a2) (local.get $a6)))

    ;; 8. l32r a8, 0x40080534  -> a8 = pre-resolved literal
    (local.set $a8 (local.get $l32r))

    ;; Exit clean: callx8 a8 is the terminator, interpreter handles it.
    (i32.const {ok})
    (local.get $a2)
    (local.get $a6)
    (local.get $a8)
    (local.get $a10)
  )
)
"#,
        ok = EXIT_FALL_THROUGH,
        bus_err = EXIT_HOST_BUS_ERROR,
    )
}

impl MultiOpBlock {
    /// Build the hot BB module + instance. Failure path bubbles wasmtime
    /// errors; caller logs + falls back to interpreter.
    pub fn build_hot_bb(engine: &Engine) -> wasmtime::Result<Self> {
        let wat = emit_hot_bb_wat();
        let module = Module::new(engine, wat)?;
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
                // No pre-staged value → host signals "couldn't satisfy
                // this load". Pushed into scratch so caller knows.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity: walker stops at a terminator.
    #[test]
    fn walker_stops_at_terminator() {
        // Build a 4-byte fake "text" containing one ADDI followed by a RET.N.
        // ADDI a3, a4, 5 — 3 bytes wide form: tricky to hand-encode; use
        // a real ELF byte pattern instead. We'll use NOP.N (2 bytes 0x3d 0xf0)
        // for the supported op and RET.N (2 bytes 0x0d 0xf0) for terminator.
        let text: Vec<u8> = vec![0x3d, 0xf0, 0x3d, 0xf0, 0x0d, 0xf0];
        let ops = walk_bb(0, |pc| Some(pc as usize), &text, 16).unwrap();
        assert_eq!(ops.len(), 2, "should collect 2 NOP.Ns then stop at RET.N");
        for op in &ops {
            assert!(matches!(op.ins, Instruction::Nop));
        }
    }

    /// Walker refuses unsupported opcodes.
    #[test]
    fn walker_refuses_unsupported() {
        // SSL — supported by neither the walker's allowlist nor the LX7
        // execute arm we care about. Bytes for `ssl a3`: 0x40, 0x13, 0x40.
        let text: Vec<u8> = vec![0x40, 0x13, 0x40, 0x00, 0x00, 0x00];
        let ops = walk_bb(0, |pc| Some(pc as usize), &text, 16);
        assert!(ops.is_none(), "must refuse unsupported opcode");
    }

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
