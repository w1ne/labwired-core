// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 JIT — runtime-agnostic walker + emit core (#124 Phase 4.1).
//!
//! This module is **always** compiled (no `jit` feature gate, no wasmtime
//! dependency). It owns the parts of the JIT pipeline that don't care
//! whether the resulting wasm bytes end up in `wasmtime::Module::new`
//! (native) or `js_sys::WebAssembly::Module::new` (browser):
//!
//!   1. The basic-block walker ([`walk_bb`]) — decodes Xtensa instructions
//!      forward from a given PC until a terminator / unsupported opcode.
//!   2. The opcode allowlist ([`is_supported`]) + terminator predicate
//!      ([`is_terminator`]).
//!   3. The end-to-end entry point ([`walk_and_emit`]) — given a flat
//!      slice of the bus that contains the candidate PC, produces an
//!      [`EmittedBlock`] containing the wasm bytes that both backends
//!      consume.
//!
//! ## Why this lives outside the `jit` feature gate
//!
//! The browser-side prototype in `labwired-wasm` cannot enable the `jit`
//! feature (wasmtime doesn't build for `wasm32-unknown-unknown`). But it
//! still needs the walker + the emitted wasm bytes. Phase 4.0 (PR #131)
//! already established this split by baking the canonical hot-block
//! bytes at crate build time into [`crate::cpu::xtensa_jit_bytes::HOT_BB_WASM`].
//! Phase 4.1 generalises that: `walk_and_emit` accepts an arbitrary PC,
//! walks the bus to decode the BB, and returns the bytes both backends
//! consume.
//!
//! ## Current emit scope (4.2′)
//!
//! [`walk_and_emit`] now constructs the wasm bytes at runtime via
//! [`super::wasm_emit::WasmModule`] + the per-opcode helpers below
//! ([`emit_or`], [`emit_l8ui`], …). Only the canonical [`HOT_BB_PC`]
//! shape is wired up — other PCs return [`EmitError::UnsupportedShape`].
//! Output is byte-equivalent (under wasmtime parity) to the build-time-
//! baked `HOT_BB_WASM` artifact so neither backend's hot path moves.
//!
//! [`HOT_BB_PC`]: crate::cpu::xtensa_jit_bytes::HOT_BB_PC
//! [`HOT_BB_WASM`]: crate::cpu::xtensa_jit_bytes::HOT_BB_WASM

use crate::cpu::xtensa_jit_bytes::{
    EXIT_FALL_THROUGH, EXIT_HOST_BUS_ERROR, HOT_BB_END, HOT_BB_INSTR_COUNT, HOT_BB_PC,
};
use crate::cpu::xtensa_jit::wasm_emit::{
    emit_call, emit_end, emit_i32_and, emit_i32_const, emit_i32_lt_s, emit_i32_or, emit_i32_shr_u,
    emit_if_void, emit_local_get, emit_local_set, emit_locals_run, emit_return, encode_u32,
    FuncType, ValType, WasmModule,
};
use crate::decoder::xtensa::{self, Instruction};
use crate::decoder::{xtensa_length, xtensa_narrow};

// ── Public side-exit / shape vocabulary ───────────────────────────────

/// Reason an emitted block can side-exit early. The actual `i32` code in
/// the wasm body comes from [`crate::cpu::xtensa_jit_bytes`] so native +
/// browser agree on the wire values; this enum is the runtime-agnostic
/// view for diagnostics + Phase 4.2 control-flow emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideExitReason {
    /// Block executed cleanly to the terminator. Wire code:
    /// [`EXIT_FALL_THROUGH`].
    FallThrough,
    /// A host import (e.g. `read_u8`) reported a bus error. Wire code:
    /// [`EXIT_HOST_BUS_ERROR`].
    HostBusError,
}

impl SideExitReason {
    /// Wire side-exit code emitted into the wasm body. Native +
    /// browser dispatch on these identical i32 values.
    #[inline]
    pub fn wire_code(self) -> i32 {
        match self {
            SideExitReason::FallThrough => EXIT_FALL_THROUGH,
            SideExitReason::HostBusError => EXIT_HOST_BUS_ERROR,
        }
    }
}

/// Failure reasons from [`walk_and_emit`]. None of these are bugs — they
/// just mean "this PC isn't JIT-able yet"; the caller falls back to the
/// interpreter and the BB walks back through the regular dispatch path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmitError {
    /// The walked BB doesn't match any currently-supported shape. In
    /// Phase 4.1 only the canonical [`HOT_BB_PC`] shape is recognised;
    /// Phase 4.2+ will expand the shape allowlist as per-opcode emit
    /// lands.
    UnsupportedShape,
    /// The walker refused (unsupported opcode mid-block, or the PC
    /// pointed outside the supplied `bus_slice`).
    WalkRefused,
    /// The walked block was empty (only a terminator at `pc`).
    BlockTooShort,
    /// PC outside the supplied bus slice — caller passed a slice that
    /// doesn't cover the block under consideration.
    PcOutOfRange,
}

impl core::fmt::Display for EmitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EmitError::UnsupportedShape => f.write_str("BB shape not yet supported by emit core"),
            EmitError::WalkRefused => f.write_str("BB walker refused (unsupported opcode or OOB)"),
            EmitError::BlockTooShort => f.write_str("BB walker returned no non-terminator ops"),
            EmitError::PcOutOfRange => f.write_str("PC outside the supplied bus slice"),
        }
    }
}

impl std::error::Error for EmitError {}

/// Subset of PS that affects JIT validity. Currently informational —
/// Phase 4.1 only emits straight-line arithmetic that doesn't depend on
/// PS. Phase 4.4 (CALL8/RETW) will read CALLINC/WOE from here. Carried
/// in [`walk_and_emit`]'s signature now so adding consumers later is
/// not an API break.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PsBits {
    /// Raw PS register value (see [`crate::cpu::xtensa_regs::Ps`]).
    pub raw: u32,
}

impl PsBits {
    /// Construct from a raw PS value (typically `cpu.ps.as_raw()`).
    #[inline]
    pub const fn from_raw(raw: u32) -> Self {
        Self { raw }
    }
}

/// One emit pass output: the wasm bytes plus the metadata both backends
/// need to commit register state, advance PC, and bump CCOUNT after the
/// block runs.
///
/// `wasm_bytes` is a `Vec<u8>` rather than a `&'static [u8]` so future
/// runtime-emit code (Phase 4.2+) can produce bytes per-BB without
/// requiring a static lifetime. Cloning a baked block reuses the static
/// bytes (one allocation, no per-call cost on the hot path because the
/// JitCache holds the compiled `Module` long-term).
#[derive(Debug, Clone)]
pub struct EmittedBlock {
    /// Wasm module bytes. Magic+version validated by the backend at
    /// `Module::new` time, not here.
    pub wasm_bytes: Vec<u8>,
    /// Number of Xtensa instructions the wasm body executes. Both
    /// backends advance CCOUNT by `length_in_instrs - 1` after a clean
    /// fall-through (the outer step already counted one).
    pub length_in_instrs: u32,
    /// First PC after the JIT'd range — the interpreter resumes here
    /// when the block fall-throughs.
    pub end_pc: u32,
    /// Side-exit codes the emitted body can produce, paired with their
    /// reason. Backends use this to map a returned i32 to "commit
    /// state" vs "refuse + fall back to interp".
    pub side_exit_reasons: Vec<(i32, SideExitReason)>,
}

impl EmittedBlock {
    /// Look up the [`SideExitReason`] for a wire exit code emitted by
    /// `self.wasm_bytes`. Returns `None` if the code isn't in
    /// `side_exit_reasons` — the backend treats that as a sim-level
    /// "unknown side-exit" error.
    #[inline]
    pub fn reason_for(&self, code: i32) -> Option<SideExitReason> {
        self.side_exit_reasons
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, r)| *r)
    }
}

// ── Walker — decoded ops + control predicates ─────────────────────────

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
pub fn is_terminator(ins: &Instruction) -> bool {
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

/// Is this opcode in the Phase 4.1 supported set?
///
/// Keep this list narrow: any new opcode here needs corresponding
/// per-opcode emit code below (`emit_*`). Adding more is Phase 4.2+
/// work and must come paired with lockstep coverage.
pub fn is_supported(ins: &Instruction) -> bool {
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

// ── walk_and_emit — runtime-agnostic entry point ──────────────────────

/// Walk the BB starting at `pc` (using `bus_slice` indexed by
/// `pc_to_offset`) and produce an [`EmittedBlock`] of wasm bytes both
/// backends can consume.
///
/// `bus_slice` is the flat slice of host memory the BB is decoded out
/// of; `pc_to_offset` maps a PC to an index inside that slice. The
/// canonical caller is `try_jit_multi_op` which extracts an IRAM slice
/// covering `pc` and supplies the offset closure.
///
/// `ps_bits` is currently informational — see [`PsBits`].
///
/// ## Phase 4.2′ scope cap
///
/// Only the canonical [`HOT_BB_PC`] shape is recognised. The wasm bytes
/// are now constructed at runtime via [`WasmModule`] + per-opcode
/// [`emit_or`] / [`emit_l8ui`] / … helpers, replacing the build-time
/// `hot_bb.wat` round-trip. Output bytes match [`HOT_BB_WASM`]'s ABI
/// exactly: same import (`host.read_u8`), same `run` export, same
/// `(a3, a5, l32r) -> (exit, a2, a6, a8, a10)` signature, same 5×i32
/// locals run, same side-exit codes. Other shapes return
/// [`EmitError::UnsupportedShape`].
pub fn walk_and_emit<F>(
    bus_slice: &[u8],
    pc: u32,
    pc_to_offset: F,
    _ps_bits: PsBits,
) -> Result<EmittedBlock, EmitError>
where
    F: FnMut(u32) -> Option<usize>,
{
    // Cap the walk at `HOT_BB_INSTR_COUNT + 2` — Phase 4.2′ only emits
    // the 8-op hot block; budget two slack ops so a shape mismatch
    // surfaces as UnsupportedShape rather than a silent truncation.
    let max_ops = (HOT_BB_INSTR_COUNT as usize) + 2;
    let ops = walk_bb(pc, pc_to_offset, bus_slice, max_ops).ok_or(EmitError::WalkRefused)?;
    if ops.is_empty() {
        return Err(EmitError::BlockTooShort);
    }

    if pc != HOT_BB_PC || !matches_hot_bb_shape(&ops) {
        return Err(EmitError::UnsupportedShape);
    }

    Ok(EmittedBlock {
        wasm_bytes: build_hot_bb_module(&ops),
        length_in_instrs: HOT_BB_INSTR_COUNT,
        end_pc: HOT_BB_END,
        side_exit_reasons: vec![
            (EXIT_FALL_THROUGH, SideExitReason::FallThrough),
            (EXIT_HOST_BUS_ERROR, SideExitReason::HostBusError),
        ],
    })
}

// ── Hot-BB runtime emit ───────────────────────────────────────────────
//
// Canonical wasm locals for the hot block, mirroring `hot_bb.wat`'s
// param + local declarations. Wasm function locals are indexed as
// params first, then declared locals — keep these in sync with the
// type and locals run in `build_hot_bb_module`.
const LOCAL_A3: u32 = 0;
const LOCAL_A5: u32 = 1;
const LOCAL_L32R: u32 = 2;
const LOCAL_A2: u32 = 3;
const LOCAL_A6: u32 = 4;
const LOCAL_A8: u32 = 5;
const LOCAL_A10: u32 = 6;
const LOCAL_TMP: u32 = 7;

/// Map an Xtensa register number used by the hot block to its wasm
/// local index. Only the registers `matches_hot_bb_shape` accepts are
/// supported.
fn hot_bb_local_for(reg: u8) -> u32 {
    match reg {
        2 => LOCAL_A2,
        3 => LOCAL_A3,
        5 => LOCAL_A5,
        6 => LOCAL_A6,
        8 => LOCAL_A8,
        10 => LOCAL_A10,
        _ => panic!("hot_bb shape match must reject unknown reg {reg}"),
    }
}

/// Build the wasm module for the canonical hot block via the
/// [`WasmModule`] builder + per-opcode emit helpers below. Matches
/// `HOT_BB_WASM`'s ABI exactly (verified by the parity test).
fn build_hot_bb_module(ops: &[DecodedOp]) -> Vec<u8> {
    let mut m = WasmModule::new();
    let t_read_u8 = m.add_type(FuncType {
        params: vec![ValType::I32],
        results: vec![ValType::I32],
    });
    let t_run = m.add_type(FuncType {
        params: vec![ValType::I32, ValType::I32, ValType::I32],
        results: vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32, ValType::I32],
    });
    let f_read_u8 = m.add_func_import("host", "read_u8", t_read_u8);

    let mut body = Vec::with_capacity(128);
    // Locals vec: one run of 5 × i32 (a2, a6, a8, a10, tmp).
    encode_u32(1, &mut body);
    emit_locals_run(5, ValType::I32, &mut body);

    for op in ops {
        match op.ins {
            Instruction::Or { ar, as_, at } => {
                emit_or(
                    hot_bb_local_for(ar),
                    hot_bb_local_for(as_),
                    hot_bb_local_for(at),
                    &mut body,
                );
            }
            Instruction::Memw => emit_memw(&mut body),
            Instruction::L8ui { at, as_, imm } => {
                emit_l8ui(
                    hot_bb_local_for(at),
                    hot_bb_local_for(as_),
                    imm,
                    f_read_u8,
                    LOCAL_TMP,
                    &mut body,
                );
            }
            Instruction::Extui {
                ar,
                at,
                shift,
                bits,
            } => {
                emit_extui(
                    hot_bb_local_for(ar),
                    hot_bb_local_for(at),
                    shift,
                    bits,
                    &mut body,
                );
            }
            Instruction::And { ar, as_, at } => {
                emit_and(
                    hot_bb_local_for(ar),
                    hot_bb_local_for(as_),
                    hot_bb_local_for(at),
                    &mut body,
                );
            }
            Instruction::L32r { at, .. } => {
                emit_l32r(hot_bb_local_for(at), LOCAL_L32R, &mut body);
            }
            Instruction::Nop => emit_nop(&mut body),
            _ => unreachable!("hot_bb shape match must reject unknown op"),
        }
    }

    // Tail: push the 5-tuple result and close the function.
    emit_i32_const(EXIT_FALL_THROUGH, &mut body);
    emit_local_get(LOCAL_A2, &mut body);
    emit_local_get(LOCAL_A6, &mut body);
    emit_local_get(LOCAL_A8, &mut body);
    emit_local_get(LOCAL_A10, &mut body);
    emit_end(&mut body);

    let f_run = m.add_func(t_run, body);
    m.add_func_export("run", f_run);
    m.finish()
}

/// Does this decoded op sequence match the canonical hot-BB shape?
///
/// `0x400829cc`:
/// ```text
///   or    a10,a5,a5
///   memw
///   l8ui  a6,a3,0
///   memw
///   l8ui  a2,a3,1
///   extui a2,a2,0,8
///   and   a2,a2,a6
///   l32r  a8,0x40080534
/// ```
fn matches_hot_bb_shape(ops: &[DecodedOp]) -> bool {
    use Instruction::*;
    if ops.len() != HOT_BB_INSTR_COUNT as usize {
        return false;
    }
    matches!(
        ops[0].ins,
        Or {
            ar: 10,
            as_: 5,
            at: 5
        }
    ) && matches!(ops[1].ins, Memw)
        && matches!(
            ops[2].ins,
            L8ui {
                at: 6,
                as_: 3,
                imm: 0
            }
        )
        && matches!(ops[3].ins, Memw)
        && matches!(
            ops[4].ins,
            L8ui {
                at: 2,
                as_: 3,
                imm: 1
            }
        )
        && matches!(
            ops[5].ins,
            Extui {
                ar: 2,
                at: 2,
                shift: 0,
                bits: 8,
            }
        )
        && matches!(
            ops[6].ins,
            And {
                ar: 2,
                as_: 2,
                at: 6
            }
        )
        && matches!(ops[7].ins, L32r { at: 8, .. })
}

// ── Per-opcode emit helpers ───────────────────────────────────────────
//
// Phase 4.2′: real wasm-byte emission per Xtensa opcode supported by the
// hot block. Each helper takes wasm local indices (not Xtensa register
// numbers) so the caller — `build_hot_bb_module` — owns the
// reg→local mapping. Bytes match `HOT_BB_WASM` exactly (parity test).

/// `or ar, as, at` — `ar = as | at`.
pub(crate) fn emit_or(ar: u32, as_: u32, at: u32, out: &mut Vec<u8>) {
    emit_local_get(as_, out);
    emit_local_get(at, out);
    emit_i32_or(out);
    emit_local_set(ar, out);
}

/// `memw` — semantic no-op (memory barrier in the simulator).
pub(crate) fn emit_memw(_out: &mut Vec<u8>) {}

/// `nop` / `nop.n` — semantic no-op.
pub(crate) fn emit_nop(_out: &mut Vec<u8>) {}

/// `l8ui at, as, imm` — `at = read_u8(as + imm)`. The host import
/// returns a negative i32 on bus error; we side-exit early with
/// `EXIT_HOST_BUS_ERROR` and zeroed-out result locals (matching
/// `HOT_BB_WASM`).
pub(crate) fn emit_l8ui(
    at: u32,
    as_: u32,
    imm: u32,
    read_u8_func: u32,
    tmp: u32,
    out: &mut Vec<u8>,
) {
    emit_local_get(as_, out);
    if imm != 0 {
        emit_i32_const(imm as i32, out);
        // i32.add
        out.push(0x6A);
    }
    emit_call(read_u8_func, out);
    emit_local_set(tmp, out);
    emit_local_get(tmp, out);
    emit_i32_const(0, out);
    emit_i32_lt_s(out);
    emit_if_void(out);
    emit_i32_const(EXIT_HOST_BUS_ERROR, out);
    emit_i32_const(0, out);
    emit_i32_const(0, out);
    emit_i32_const(0, out);
    emit_i32_const(0, out);
    emit_return(out);
    emit_end(out);
    emit_local_get(tmp, out);
    emit_i32_const(0xFF, out);
    emit_i32_and(out);
    emit_local_set(at, out);
}

/// `l32r at, literal_addr` — pre-resolved literal supplied as a wasm
/// param. We just copy the param into the destination local.
pub(crate) fn emit_l32r(at: u32, l32r_param: u32, out: &mut Vec<u8>) {
    emit_local_get(l32r_param, out);
    emit_local_set(at, out);
}

/// `extui ar, at, shift, bits` — `ar = (at >> shift) & ((1 << bits) - 1)`.
pub(crate) fn emit_extui(ar: u32, at: u32, shift: u8, bits: u8, out: &mut Vec<u8>) {
    emit_local_get(at, out);
    if shift != 0 {
        emit_i32_const(shift as i32, out);
        emit_i32_shr_u(out);
    }
    let mask: i32 = if bits >= 32 {
        -1
    } else {
        ((1u32 << bits) - 1) as i32
    };
    emit_i32_const(mask, out);
    emit_i32_and(out);
    emit_local_set(ar, out);
}

/// `and ar, as, at` — `ar = as & at`.
pub(crate) fn emit_and(ar: u32, as_: u32, at: u32, out: &mut Vec<u8>) {
    emit_local_get(as_, out);
    emit_local_get(at, out);
    emit_i32_and(out);
    emit_local_set(ar, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Walker stops at a terminator and excludes it from the returned vec.
    #[test]
    fn walker_stops_at_terminator() {
        // 4 bytes: two NOP.N (0x3d 0xf0) then a RET.N (0x0d 0xf0).
        let text: Vec<u8> = vec![0x3d, 0xf0, 0x3d, 0xf0, 0x0d, 0xf0];
        let ops = walk_bb(0, |pc| Some(pc as usize), &text, 16).unwrap();
        assert_eq!(ops.len(), 2, "should collect 2 NOP.Ns then stop at RET.N");
        for op in &ops {
            assert!(matches!(op.ins, Instruction::Nop));
        }
    }

    /// Walker refuses unsupported opcodes (returns None).
    #[test]
    fn walker_refuses_unsupported() {
        // SSL a3 — not in `is_supported`.
        let text: Vec<u8> = vec![0x40, 0x13, 0x40, 0x00, 0x00, 0x00];
        let ops = walk_bb(0, |pc| Some(pc as usize), &text, 16);
        assert!(ops.is_none(), "must refuse unsupported opcode");
    }

    /// `walk_and_emit` returns `UnsupportedShape` for non-hot-BB PCs.
    #[test]
    fn walk_and_emit_unknown_pc_unsupported() {
        let text: Vec<u8> = vec![0x3d, 0xf0, 0x0d, 0xf0];
        let err = walk_and_emit(&text, 0, |pc| Some(pc as usize), PsBits::default()).unwrap_err();
        assert_eq!(err, EmitError::UnsupportedShape);
    }

    /// `walk_and_emit` propagates walker failures as `WalkRefused`.
    #[test]
    fn walk_and_emit_walker_refused_propagates() {
        // SSL a3 — refused by walker.
        let text: Vec<u8> = vec![0x40, 0x13, 0x40];
        let err = walk_and_emit(&text, 0, |pc| Some(pc as usize), PsBits::default()).unwrap_err();
        assert_eq!(err, EmitError::WalkRefused);
    }

    /// Functional parity: `walk_and_emit` on the canonical hot block
    /// matches the pre-baked `HOT_BB_WASM` for every input we test. Both
    /// are instantiated under wasmtime with the same `host.read_u8`
    /// import (returns `(addr & 0xFF) as i32`) and the 5-tuple results
    /// are compared byte-for-byte. Gated on `feature = "jit"` because
    /// the parity check needs wasmtime.
    #[cfg(feature = "jit")]
    #[test]
    fn walk_and_emit_matches_hot_bb_wasm_under_wasmtime() {
        use crate::cpu::xtensa_jit_bytes::HOT_BB_WASM;
        use wasmtime::{Engine, Func, Instance, Module, Store, TypedFunc};

        const HOT_BB_BYTES: &[u8] = &[
            0x50, 0xa5, 0x20, // or    a10, a5, a5
            0xc0, 0x20, 0x00, // memw
            0x62, 0x03, 0x00, // l8ui  a6,  a3, 0
            0xc0, 0x20, 0x00, // memw
            0x22, 0x03, 0x01, // l8ui  a2,  a3, 1
            0x20, 0x20, 0x74, // extui a2,  a2, 0, 8
            0x60, 0x22, 0x10, // and   a2,  a2, a6
            0x81, 0xd4, 0xf6, // l32r  a8,  0x40080534
            0xe0, 0x08, 0x00, // callx8 a8 — terminator
        ];
        let emitted = walk_and_emit(
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
        .expect("hot bb emits");

        type Params = (i32, i32, i32);
        type Returns = (i32, i32, i32, i32, i32);

        fn instantiate(engine: &Engine, bytes: &[u8]) -> (Store<()>, TypedFunc<Params, Returns>) {
            let module = Module::new(engine, bytes).expect("wasmtime accepts module");
            let mut store = Store::new(engine, ());
            // host.read_u8(addr) -> (addr & 0xFF) as i32. Deterministic
            // and non-negative so the bus-error early-exit path stays
            // dormant for these inputs.
            let read_u8 = Func::wrap(&mut store, |addr: i32| -> i32 { (addr as u32 & 0xFF) as i32 });
            let inst = Instance::new(&mut store, &module, &[read_u8.into()])
                .expect("instantiate");
            let run = inst
                .get_typed_func::<Params, Returns>(&mut store, "run")
                .expect("run export");
            (store, run)
        }

        let engine = Engine::default();
        let (mut s_ref, run_ref) = instantiate(&engine, HOT_BB_WASM);
        let (mut s_new, run_new) = instantiate(&engine, &emitted.wasm_bytes);

        let inputs: [(i32, i32, i32); 4] = [
            (0, 0, 0x40080534),
            (0xDEADBEEFu32 as i32, 0x12345678, 0x40080534),
            (-1, -1, 0x40080534),
            (0x3FFB_0000, 0x1234, 0x40008534u32 as i32),
        ];
        for input in inputs {
            let r_ref = run_ref.call(&mut s_ref, input).expect("ref runs");
            let r_new = run_new.call(&mut s_new, input).expect("new runs");
            assert_eq!(
                r_ref, r_new,
                "parity mismatch for input {:?}: ref={:?} new={:?}",
                input, r_ref, r_new
            );
        }
    }

    /// `SideExitReason::wire_code` round-trips through
    /// `EmittedBlock::reason_for` (sanity for backends that index by
    /// the i32 returned from wasm).
    #[test]
    fn side_exit_reason_round_trips() {
        let block = EmittedBlock {
            wasm_bytes: vec![0, b'a', b's', b'm', 1, 0, 0, 0],
            length_in_instrs: 1,
            end_pc: 0,
            side_exit_reasons: vec![
                (EXIT_FALL_THROUGH, SideExitReason::FallThrough),
                (EXIT_HOST_BUS_ERROR, SideExitReason::HostBusError),
            ],
        };
        assert_eq!(
            block.reason_for(EXIT_FALL_THROUGH),
            Some(SideExitReason::FallThrough)
        );
        assert_eq!(
            block.reason_for(EXIT_HOST_BUS_ERROR),
            Some(SideExitReason::HostBusError)
        );
        assert_eq!(block.reason_for(42), None);
    }
}
