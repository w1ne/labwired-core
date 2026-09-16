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
//! ## Current emit scope (4.3))
//!
//! Phase 4.1 only recognised the canonical [`HOT_BB_PC`] shape and reused
//! the pre-baked [`HOT_BB_WASM`] bytes. Phase 4.3 adds a **variable-length
//! wasm byte emitter** ([`emit_generic`] + the per-op `emit_*` helpers):
//! for any straight-line basic block whose ops are in [`is_supported`] and
//! whose terminator is a statically-resolvable conditional branch (or `J`),
//! [`walk_and_emit`] now builds a fresh wasm module at runtime. The module
//! ABI is a 16-register file plus `PC`-independent branch resolution:
//!
//! ```text
//! run(a0..a15) -> (exit_code, branch_target, r0..r15)
//! ```
//!
//! The emitted body calls five host imports — `host.read_u8`,
//! `host.read_u32`, `host.write_u8`, `host.write_u32`,
//! `host.branch_target` — mirroring the pre-existing `read_u8` staging
//! contract: the host pre-stages load values (the wasm import returns `-1`
//! on an empty queue, which side-exits with [`EXIT_HOST_BUS_ERROR`]) and
//! drains write requests after the call. Memory op *addresses* are not
//! guessed by the host: [`EmittedBlock::loads`] / [`EmittedBlock::stores`]
//! carry a per-op manifest so the dispatcher can compute every effective
//! address from the live register file before/after the call.
//!
//! Refusals are explicit and named ([`EmitError`]): 16-bit loads/stores
//! (`UnsupportedOpForEmit`) and `L32R` (`UnsupportedOpForEmit` — RFC: the
//! literal pool address needs the caller's PC→offset map, which the walker
//! consumes) are deliberately out of scope. A load whose base register is
//! written earlier in the same block is refused
//! ([`EmitError::LoadBaseClobbered`]): adapters pre-stage manifest loads
//! from the entry register file, so the host would fetch from a stale
//! address. `CALL{n}`/`RETW`/`JX`/`RET` terminators stay refused
//! (`UnsupportedTerminatorForEmit`) until Phase 4.4 lands PS.CALLINC
//! handling. `J` retires through its own [`EXIT_JUMP_TAKEN`] code because
//! the interpreter's `J` arm leaves `branched` clear, unlike the conditional
//! `branch()` family; dispatchers use that code to advance PC without
//! marking the step branched (zero-overhead-loop fidelity).
//!
//! [`HOT_BB_PC`]: crate::cpu::xtensa_jit_bytes::HOT_BB_PC
//! [`HOT_BB_WASM`]: crate::cpu::xtensa_jit_bytes::HOT_BB_WASM

use crate::cpu::xtensa_jit_bytes::{
    EXIT_BRANCH_TAKEN, EXIT_FALL_THROUGH, EXIT_HOST_BUS_ERROR, EXIT_JUMP_TAKEN, HOT_BB_END,
    HOT_BB_INSTR_COUNT, HOT_BB_PC, HOT_BB_WASM,
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
    /// The block's conditional branch was taken; the caller continues at
    /// the `target` value returned by the block and sets `branched`.
    /// Wire code: [`EXIT_BRANCH_TAKEN`].
    BranchTaken,
    /// The block's unconditional `J` retired; the caller continues at the
    /// `target` value but leaves `branched` clear (the interpreter's `J`
    /// arm does not mark the step as branched). Wire code:
    /// [`EXIT_JUMP_TAKEN`].
    JumpTaken,
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
            SideExitReason::BranchTaken => EXIT_BRANCH_TAKEN,
            SideExitReason::JumpTaken => EXIT_JUMP_TAKEN,
            SideExitReason::HostBusError => EXIT_HOST_BUS_ERROR,
        }
    }
}

/// Register-level ABI of an [`EmittedBlock`] body. The canonical hot
/// block keeps its hand-baked 3-parameter shape; runtime-emitted blocks
/// use the generic 16-register file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockAbi {
    /// `run(a3, a5, l32r) -> (exit, a2, a6, a8, a10)` — the pre-baked
    /// canonical hot block (`HOT_BB_WASM`).
    HotBb,
    /// `run(a0..a15) -> (exit, target, r0..r15)` — runtime-emitted
    /// straight-line/branch blocks. `target` is the branch destination
    /// when `exit == EXIT_BRANCH_TAKEN`, `0` otherwise.
    Lx7Generic,
}

/// One load the emitted body performs, in execution order. The
/// dispatcher resolves `base_reg + imm` against the live register file,
/// reads the value through the `Bus` *before* invoking wasm, and stages
/// it for the `host.read_u8` / `host.read_u32` import to dequeue — the
/// same staging contract as the canonical hot block, generalised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadReq {
    pub base: u8,
    pub imm: u32,
    pub width: MemWidth,
}

/// One store the emitted body performs, in execution order. `value` is
/// the logical register whose low bits the body hands to the
/// `host.write_*` import; the dispatcher commits the drained
/// `(address, value)` pairs after a successful run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreReq {
    pub base: u8,
    pub imm: u32,
    pub width: MemWidth,
    pub value: u8,
}

/// Memory access width for [`LoadReq`] / [`StoreReq`]. 16-bit accesses
/// are deliberately not modelled yet (see [`EmitError`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemWidth {
    U8,
    U32,
}

impl MemWidth {
    /// Byte width of one access.
    #[inline]
    pub fn bytes(self) -> u32 {
        match self {
            MemWidth::U8 => 1,
            MemWidth::U32 => 4,
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
    /// Opcode is in the walker allowlist but the generic emitter doesn't
    /// cover it yet. Named per-op refusal: 16-bit loads/stores
    /// (`L16ui`/`L16si`/`S16i`) need `read_u16`/`write_u16` imports, and
    /// `L32r` needs the caller's PC→offset map to resolve its literal
    /// pool address (the walker consumes that closure). Refused rather
    /// than half-emitted.
    UnsupportedOpForEmit(Instruction),
    /// A load's base register was written by an earlier op in the same
    /// block. The dispatchers pre-read every [`LoadReq`] through the live
    /// `Bus` against the *entry* register file before invoking the body,
    /// so the host would stage the value from a stale address (the body's
    /// own address arithmetic is ignored by the `read_*` imports). Long
    /// load-then-dereference chains are the common case this catches;
    /// refused rather than half-emitted.
    LoadBaseClobbered { base: u8, pc: u32 },
    /// Terminator isn't statically resolvable by the generic emitter.
    /// `CALL{n}`/`RETW`/`RET`/`JX`/`BT`/`BF`/`ENTRY`/`RF*` side-exit into
    /// the interpreter (Phase 4.4 covers the windowed ones).
    UnsupportedTerminatorForEmit(Instruction),
    /// The walk collected [`GENERIC_MAX_OPS`] ops without reaching a
    /// terminator — longer than any BB the emitter is budgeted for.
    TooManyOps { len: usize, cap: usize },
}

impl core::fmt::Display for EmitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EmitError::UnsupportedShape => f.write_str("BB shape not yet supported by emit core"),
            EmitError::WalkRefused => f.write_str("BB walker refused (unsupported opcode or OOB)"),
            EmitError::BlockTooShort => f.write_str("BB walker returned no non-terminator ops"),
            EmitError::PcOutOfRange => f.write_str("PC outside the supplied bus slice"),
            EmitError::UnsupportedOpForEmit(ins) => {
                write!(f, "opcode not covered by generic emit: {ins:?}")
            }
            EmitError::LoadBaseClobbered { base, pc } => {
                write!(
                    f,
                    "load base a{base} was clobbered at pc=0x{pc:08x} before its load \
                     (host staging would read a stale address)"
                )
            }
            EmitError::UnsupportedTerminatorForEmit(ins) => {
                write!(f, "terminator not covered by generic emit: {ins:?}")
            }
            EmitError::TooManyOps { len, cap } => {
                write!(f, "BB has {len} ops without a terminator (cap {cap})")
            }
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
    /// Parameter/result shape of the `run` export.
    pub abi: BlockAbi,
    /// Number of Xtensa instructions the wasm body executes. Both
    /// backends advance CCOUNT by `length_in_instrs - 1` after a clean
    /// fall-through (the outer step already counted one).
    pub length_in_instrs: u32,
    /// First PC after the JIT'd range — the interpreter resumes here
    /// when the block fall-throughs. For a branch-terminated block this
    /// is the branch's fall-through destination.
    pub end_pc: u32,
    /// Loads the body performs, in execution order. Dispatcher resolves
    /// each `base + imm` against the live register file, reads the value
    /// through the `Bus`, and stages it for the host import queue.
    pub loads: Vec<LoadReq>,
    /// Stores the body performs, in execution order. Dispatcher drains
    /// the `(address, value)` pairs from the host import queue after a
    /// successful run and commits them via the `Bus`.
    pub stores: Vec<StoreReq>,
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

/// A walked basic block: the non-terminator ops plus (when one was hit
/// before the op budget ran out) the decoded terminator that closed it.
///
/// The canonical `walk_bb` drops the terminator for back-compat; the
/// generic emitter needs it to emit branch/exit semantics.
#[derive(Debug, Clone)]
pub struct WalkedBb {
    pub ops: Vec<DecodedOp>,
    /// Terminator that closed the block, or `None` when the op budget
    /// was exhausted first.
    pub terminator: Option<DecodedOp>,
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
    pc_to_offset: F,
    text: &[u8],
    max_ops: usize,
) -> Option<Vec<DecodedOp>>
where
    F: FnMut(u32) -> Option<usize>,
{
    walk_bb_with_terminator(start_pc, pc_to_offset, text, max_ops).map(|w| w.ops)
}

/// [`walk_bb`] variant that also returns the terminator that closed the
/// block. Used by [`walk_and_emit`]'s generic emitter to resolve branch
/// conditions and fall-through PCs.
pub fn walk_bb_with_terminator<F>(
    start_pc: u32,
    mut pc_to_offset: F,
    text: &[u8],
    max_ops: usize,
) -> Option<WalkedBb>
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
            return Some(WalkedBb {
                ops,
                terminator: Some(DecodedOp { pc, len, ins }),
            });
        }
        if !is_supported(&ins) {
            return None;
        }
        ops.push(DecodedOp { pc, len, ins });
        pc = pc.wrapping_add(len);
    }
    Some(WalkedBb {
        ops,
        terminator: None,
    })
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
            | J { .. }
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
            | Bt { .. }
            | Bf { .. }
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

/// Is this opcode in the emit-core supported set?
///
/// Keep this list narrow: any opcode here needs corresponding per-opcode
/// emit code below (`emit_*`). Phase 4.3 grew the list from the original
/// ALU/`L8ui`/`L32r` set to include the 32-bit load/store pair and
/// `Addmi`, which is the surface the two new generic shapes (ALU+branch,
/// load/store+branch) need. 16-bit accesses are allowlisted so the walker
/// can hand the emitter a named [`EmitError::UnsupportedOpForEmit`]
/// refusal instead of a silent `WalkRefused` — they are *not* emitted.
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
            | Addmi { .. }
            | Movi { .. }
            | Extui { .. }
            // Loads / stores (32-bit + byte; 16-bit allowlisted, refused)
            | L8ui { .. }
            | L32i { .. }
            | L16ui { .. }
            | L16si { .. }
            | S8i { .. }
            | S16i { .. }
            | S32i { .. }
            | L32r { .. }
            // Barriers — semantic no-ops in sim
            | Memw
            | Nop
    )
}

// ── walk_and_emit — runtime-agnostic entry point ──────────────────────

/// Op budget for one generic emit. Real LX7 BBs are < 10 instructions;
/// the cap bounds the module size a single refused-then-re-walked miss
/// can produce and makes "no terminator inside the budget" an explicit
/// [`EmitError::TooManyOps`] refusal.
pub const GENERIC_MAX_OPS: usize = 16;

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
/// ## Phase 4.3 emit decisions
///
/// The canonical [`HOT_BB_PC`] shape still returns the pre-baked
/// [`HOT_BB_WASM`] bytes (the hot path must not regress). Every other
/// straight-line block whose ops are allowlisted and whose terminator is
/// a conditional branch or `J` goes through [`emit_generic`], which
/// builds a fresh module with the 16-register ABI. Refusals are named
/// [`EmitError`] variants and documented on [`is_supported`].
pub fn walk_and_emit<F>(
    bus_slice: &[u8],
    pc: u32,
    pc_to_offset: F,
    _ps_bits: PsBits,
) -> Result<EmittedBlock, EmitError>
where
    F: FnMut(u32) -> Option<usize>,
{
    let walked = walk_bb_with_terminator(pc, pc_to_offset, bus_slice, GENERIC_MAX_OPS)
        .ok_or(EmitError::WalkRefused)?;
    if walked.ops.is_empty() {
        return Err(EmitError::BlockTooShort);
    }

    if pc == HOT_BB_PC && matches_hot_bb_shape(&walked.ops) {
        Ok(EmittedBlock {
            wasm_bytes: HOT_BB_WASM.to_vec(),
            abi: BlockAbi::HotBb,
            length_in_instrs: HOT_BB_INSTR_COUNT,
            end_pc: HOT_BB_END,
            // Hot-block loads are dispatched by the pre-existing fast
            // path; the manifest is populated for symmetry so future
            // generic dispatchers can run the canonical block too.
            loads: vec![
                LoadReq {
                    base: 3,
                    imm: 0,
                    width: MemWidth::U8,
                },
                LoadReq {
                    base: 3,
                    imm: 1,
                    width: MemWidth::U8,
                },
            ],
            stores: Vec::new(),
            side_exit_reasons: vec![
                (EXIT_FALL_THROUGH, SideExitReason::FallThrough),
                (EXIT_HOST_BUS_ERROR, SideExitReason::HostBusError),
            ],
        })
    } else {
        emit_generic(walked)
    }
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

// ── Generic emitter — arbitrary straight-line block + branch ──────────

/// Build a generic block from a walked BB.
///
/// Shape acceptance ("the allowlist"): every non-terminator op must be
/// covered by [`emit_op`] and the terminator must be a conditional branch
/// or `J`. Anything else is refused by name:
///   * 16-bit accesses / `L32r` → [`EmitError::UnsupportedOpForEmit`]
///   * `CALL*`/`RETW`/`RET`/`JX`/... → [`EmitError::UnsupportedTerminatorForEmit`]
///   * no terminator within [`GENERIC_MAX_OPS`] → [`EmitError::TooManyOps`]
fn emit_generic(walked: WalkedBb) -> Result<EmittedBlock, EmitError> {
    let WalkedBb { ops, terminator } = walked;
    let Some(term) = terminator else {
        return Err(EmitError::TooManyOps {
            len: ops.len(),
            cap: GENERIC_MAX_OPS,
        });
    };

    let mut body = Vec::with_capacity(256);
    let mut loads = Vec::new();
    let mut stores = Vec::new();

    // Prologue: exit = 0, target = 0. A straight-line fall-through leaves
    // both at zero, which is exactly what the dispatcher commits.
    op_i32_const(&mut body, 0);
    op_local_set(&mut body, L_EXIT);
    op_i32_const(&mut body, 0);
    op_local_set(&mut body, L_TARGET);

    // Registers written so far, in execution order. A load whose base was
    // written by an earlier op cannot be pre-staged by the host from the
    // entry register file; `emit_op` refuses that shape by name.
    let mut written = [false; 16];
    for op in &ops {
        emit_op(op, &mut body, &mut loads, &mut stores, &mut written)?;
    }
    emit_terminator(&term, &mut body)?;

    // Epilogue: push (exit, target, r0..r15) for the function end.
    op_local_get(&mut body, L_EXIT);
    op_local_get(&mut body, L_TARGET);
    for r in 0..16u8 {
        op_local_get(&mut body, r);
    }
    body.push(OP_END);

    Ok(EmittedBlock {
        wasm_bytes: build_generic_module(&body),
        abi: BlockAbi::Lx7Generic,
        // The body evaluates the terminator's condition itself, so the
        // branch retires inside the block too.
        length_in_instrs: (ops.len() as u32) + 1,
        end_pc: term.pc.wrapping_add(term.len),
        loads,
        stores,
        side_exit_reasons: {
            let mut exits = vec![
                (EXIT_FALL_THROUGH, SideExitReason::FallThrough),
                (EXIT_HOST_BUS_ERROR, SideExitReason::HostBusError),
            ];
            match term.ins {
                Instruction::J { .. } => exits.push((EXIT_JUMP_TAKEN, SideExitReason::JumpTaken)),
                _ => exits.push((EXIT_BRANCH_TAKEN, SideExitReason::BranchTaken)),
            }
            exits
        },
    })
}

/// Emit one non-terminator op into `body`, recording its memory access in
/// the load/store manifests in execution order.
///
/// `written` tracks registers already written earlier in the block; a load
/// whose base is in that set is refused (see
/// [`EmitError::LoadBaseClobbered`]) because the dispatchers stage load
/// values from the entry register file.
fn emit_op(
    op: &DecodedOp,
    body: &mut Vec<u8>,
    loads: &mut Vec<LoadReq>,
    stores: &mut Vec<StoreReq>,
    written: &mut [bool; 16],
) -> Result<(), EmitError> {
    use Instruction::*;
    let ins = op.ins;
    if let L8ui { as_, .. } | L32i { as_, .. } = ins {
        if written[as_ as usize] {
            return Err(EmitError::LoadBaseClobbered {
                base: as_,
                pc: op.pc,
            });
        }
    }
    // Register written by this op, if any. Loads count: a later load that
    // dereferences an earlier load's result hits the same stale-staging
    // problem.
    let dest = match ins {
        Add { ar, .. } | Sub { ar, .. } | And { ar, .. } | Or { ar, .. } | Xor { ar, .. } => {
            Some(ar)
        }
        Addi { at, .. }
        | Addmi { at, .. }
        | Movi { at, .. }
        | Extui { at, .. }
        | L8ui { at, .. }
        | L32i { at, .. } => Some(at),
        _ => None,
    };
    match ins {
        Add { ar, as_, at } => emit_add(ar, as_, at, body),
        Sub { ar, as_, at } => emit_sub(ar, as_, at, body),
        And { ar, as_, at } => emit_and(ar, as_, at, body),
        Or { ar, as_, at } => emit_or(ar, as_, at, body),
        Xor { ar, as_, at } => emit_xor(ar, as_, at, body),
        Addi { at, as_, imm8 } => emit_addi(at, as_, imm8, body),
        Addmi { at, as_, imm } => emit_addmi(at, as_, imm, body),
        Movi { at, imm } => emit_movi(at, imm, body),
        Extui {
            ar,
            at,
            shift,
            bits,
        } => emit_extui(ar, at, shift, bits, body),
        L8ui { at, as_, imm } => {
            loads.push(LoadReq {
                base: as_,
                imm,
                width: MemWidth::U8,
            });
            emit_l8ui(at, as_, imm, body);
        }
        L32i { at, as_, imm } => {
            loads.push(LoadReq {
                base: as_,
                imm,
                width: MemWidth::U32,
            });
            emit_l32i(at, as_, imm, body);
        }
        S8i { at, as_, imm } => {
            stores.push(StoreReq {
                base: as_,
                imm,
                width: MemWidth::U8,
                value: at,
            });
            emit_s8i(at, as_, imm, body);
        }
        S32i { at, as_, imm } => {
            stores.push(StoreReq {
                base: as_,
                imm,
                width: MemWidth::U32,
                value: at,
            });
            emit_s32i(at, as_, imm, body);
        }
        Memw => emit_memw(body),
        Nop => emit_nop(body),
        // Allowlisted for a named refusal rather than a silent
        // `WalkRefused`; see `is_supported`.
        L16ui { .. } | L16si { .. } | S16i { .. } | L32r { .. } => {
            return Err(EmitError::UnsupportedOpForEmit(ins))
        }
        other => return Err(EmitError::UnsupportedOpForEmit(other)),
    }
    if let Some(d) = dest {
        written[d as usize] = true;
    }
    Ok(())
}

/// Emit branch semantics for `term`. Conditional branches compute their
/// predicate from the register-file locals and, when true, set
/// `exit = EXIT_BRANCH_TAKEN` and `target = host.branch_target(pc, off)`.
/// `J` does the same unconditionally. Everything else refuses by name.
///
/// Host-side `branch_target` is deliberate: the host owns the
/// architectural "taken PC = branch_pc + decoder_prebiased_offset"
/// arithmetic, so emit-core never re-derives Xtensa PC math and the
/// browser/native adapters agree byte-for-byte.
fn emit_terminator(term: &DecodedOp, body: &mut Vec<u8>) -> Result<(), EmitError> {
    use Instruction::*;

    // Unconditional jump: no predicate, straight to the taken arm. `J`
    // reports `EXIT_JUMP_TAKEN` rather than `EXIT_BRANCH_TAKEN` because the
    // interpreter's `J` arm leaves `branched` clear (only `branch()` does),
    // and `branched` gates the zero-overhead-loop post-instruction check.
    if let J { offset } = term.ins {
        emit_taken(body, term.pc, offset, EXIT_JUMP_TAKEN);
        return Ok(());
    }

    // Conditional families: push the i32 predicate, then `if` with only a
    // taken arm (false leaves exit/target at 0 = fall-through).
    let cond_pushed = match term.ins {
        Beq { as_, at, .. } => {
            op_local_get(body, as_);
            op_local_get(body, at);
            body.push(OP_I32_EQ);
            true
        }
        Bne { as_, at, .. } => {
            op_local_get(body, as_);
            op_local_get(body, at);
            body.push(OP_I32_NE);
            true
        }
        Blt { as_, at, .. } => {
            op_local_get(body, as_);
            op_local_get(body, at);
            body.push(OP_I32_LT_S);
            true
        }
        Bge { as_, at, .. } => {
            op_local_get(body, as_);
            op_local_get(body, at);
            body.push(OP_I32_GE_S);
            true
        }
        Bltu { as_, at, .. } => {
            op_local_get(body, as_);
            op_local_get(body, at);
            body.push(OP_I32_LT_U);
            true
        }
        Bgeu { as_, at, .. } => {
            op_local_get(body, as_);
            op_local_get(body, at);
            body.push(OP_I32_GE_U);
            true
        }
        Beqz { as_, .. } => {
            op_local_get(body, as_);
            body.push(OP_I32_EQZ);
            true
        }
        Bnez { as_, .. } => {
            op_local_get(body, as_);
            op_i32_const(body, 0);
            body.push(OP_I32_NE);
            true
        }
        Bltz { as_, .. } => {
            op_local_get(body, as_);
            op_i32_const(body, 0);
            body.push(OP_I32_LT_S);
            true
        }
        Bgez { as_, .. } => {
            op_local_get(body, as_);
            op_i32_const(body, 0);
            body.push(OP_I32_GE_S);
            true
        }
        Beqi { as_, imm, .. } => {
            op_local_get(body, as_);
            op_i32_const(body, imm);
            body.push(OP_I32_EQ);
            true
        }
        Bnei { as_, imm, .. } => {
            op_local_get(body, as_);
            op_i32_const(body, imm);
            body.push(OP_I32_NE);
            true
        }
        Blti { as_, imm, .. } => {
            op_local_get(body, as_);
            op_i32_const(body, imm);
            body.push(OP_I32_LT_S);
            true
        }
        Bgei { as_, imm, .. } => {
            op_local_get(body, as_);
            op_i32_const(body, imm);
            body.push(OP_I32_GE_S);
            true
        }
        Bltui { as_, imm, .. } => {
            op_local_get(body, as_);
            // `imm` is a u32 for the BIU family; the bit pattern is what
            // `i32.lt_u` compares, so the lossless cast is intentional.
            op_i32_const(body, imm as i32);
            body.push(OP_I32_LT_U);
            true
        }
        Bgeui { as_, imm, .. } => {
            op_local_get(body, as_);
            op_i32_const(body, imm as i32);
            body.push(OP_I32_GE_U);
            true
        }
        // `Bt`/`Bf` read the boolean-register file, which is not part of
        // the 16-register ABI; windowed terminators need Phase 4.4.
        ref other => return Err(EmitError::UnsupportedTerminatorForEmit(*other)),
    };

    if cond_pushed {
        let offset = match term.ins {
            Beq { offset, .. }
            | Bne { offset, .. }
            | Blt { offset, .. }
            | Bge { offset, .. }
            | Bltu { offset, .. }
            | Bgeu { offset, .. }
            | Beqz { offset, .. }
            | Bnez { offset, .. }
            | Bltz { offset, .. }
            | Bgez { offset, .. }
            | Beqi { offset, .. }
            | Bnei { offset, .. }
            | Blti { offset, .. }
            | Bgei { offset, .. }
            | Bltui { offset, .. }
            | Bgeui { offset, .. } => offset,
            _ => unreachable!("cond_pushed only set for conditional branches"),
        };
        body.push(OP_IF);
        body.push(OP_BLOCK_VOID);
        emit_taken(body, term.pc, offset, EXIT_BRANCH_TAKEN);
        body.push(OP_END);
    }
    Ok(())
}

/// Taken arm shared by conditional branches and `J`: set the exit code
/// and ask the host for the resolved target PC.
fn emit_taken(body: &mut Vec<u8>, branch_pc: u32, offset: i32, exit_code: i32) {
    op_i32_const(body, exit_code);
    op_local_set(body, L_EXIT);
    op_i32_const(body, branch_pc as i32);
    op_i32_const(body, offset);
    op_call(body, FN_BRANCH_TARGET);
    op_local_set(body, L_TARGET);
}

// ── Per-opcode wasm emit ──────────────────────────────────────────────
//
// Local index ABI shared by every generic module:
//   locals 0..=15 = a0..a15 (the `run` parameters)
//   16 = exit, 17 = target, 18 = addr, 19 = tmp
// Scratch locals are reused across ops — the wasm body is straight-line
// so a single slot each is enough.

const FN_READ_U8: u8 = 0;
const FN_READ_U32: u8 = 1;
const FN_WRITE_U8: u8 = 2;
const FN_WRITE_U32: u8 = 3;
const FN_BRANCH_TARGET: u8 = 4;
const FN_RUN: u8 = 5;

const L_EXIT: u8 = 16;
const L_TARGET: u8 = 17;
const L_ADDR: u8 = 18;
const L_TMP: u8 = 19;
/// Scratch locals after the 16 register parameters.
const LOCAL_COUNT: u64 = 4;

const OP_NOP: u8 = 0x01;
const OP_IF: u8 = 0x04;
const OP_END: u8 = 0x0B;
const OP_RETURN: u8 = 0x0F;
const OP_CALL: u8 = 0x10;
const OP_LOCAL_GET: u8 = 0x20;
const OP_LOCAL_SET: u8 = 0x21;
/// Empty (`[] -> []`) block type — required after `if`/`block`/`loop`.
const OP_BLOCK_VOID: u8 = 0x40;
const OP_I32_CONST: u8 = 0x41;
const OP_I32_EQZ: u8 = 0x45;
const OP_I32_EQ: u8 = 0x46;
const OP_I32_NE: u8 = 0x47;
const OP_I32_LT_S: u8 = 0x48;
const OP_I32_LT_U: u8 = 0x49;
const OP_I32_GE_S: u8 = 0x4E;
const OP_I32_GE_U: u8 = 0x4F;
const OP_I32_ADD: u8 = 0x6A;
const OP_I32_SUB: u8 = 0x6B;
const OP_I32_AND: u8 = 0x71;
const OP_I32_OR: u8 = 0x72;
const OP_I32_XOR: u8 = 0x73;
const OP_I32_SHR_U: u8 = 0x76;

/// Bus-error side exit: `(EXIT_HOST_BUS_ERROR, 0, r0..r15)` and return.
/// Register values are the current locals, so the caller commits nothing
/// new on this path — the interpreter re-runs the block from the top.
fn emit_bus_error_exit(body: &mut Vec<u8>) {
    op_i32_const(body, EXIT_HOST_BUS_ERROR);
    op_i32_const(body, 0);
    for r in 0..16u8 {
        op_local_get(body, r);
    }
    body.push(OP_RETURN);
}

/// Loads share one shape: compute `addr`, call the import, refuse on
/// `-1`, then store the (masked) value. `func` selects u8/u32.
fn emit_load(at: u8, as_: u8, imm: u32, func: u8, mask: Option<i32>, body: &mut Vec<u8>) {
    op_local_get(body, as_);
    op_i32_const(body, imm as i32);
    body.push(OP_I32_ADD);
    op_local_set(body, L_ADDR);

    op_local_get(body, L_ADDR);
    op_call(body, func);
    op_local_set(body, L_TMP);

    op_local_get(body, L_TMP);
    op_i32_const(body, 0);
    body.push(OP_I32_LT_S);
    body.push(OP_IF);
    body.push(OP_BLOCK_VOID);
    emit_bus_error_exit(body);
    body.push(OP_END);

    op_local_get(body, L_TMP);
    if let Some(m) = mask {
        op_i32_const(body, m);
        body.push(OP_I32_AND);
    }
    op_local_set(body, at);
}

/// Stores mirror [`emit_load`] but hand `(addr, value)` to the host,
/// which queues it for commit after the call. The import returns 0 on
/// success and -1 only if the host queue itself refuses (never in the
/// current adapters); the check keeps the refusal contract uniform.
fn emit_store(at: u8, as_: u8, imm: u32, func: u8, mask: Option<i32>, body: &mut Vec<u8>) {
    op_local_get(body, as_);
    op_i32_const(body, imm as i32);
    body.push(OP_I32_ADD);
    op_local_set(body, L_ADDR);

    op_local_get(body, L_ADDR);
    op_local_get(body, at);
    if let Some(m) = mask {
        op_i32_const(body, m);
        body.push(OP_I32_AND);
    }
    op_call(body, func);
    op_local_set(body, L_TMP);

    op_local_get(body, L_TMP);
    op_i32_const(body, 0);
    body.push(OP_I32_LT_S);
    body.push(OP_IF);
    body.push(OP_BLOCK_VOID);
    emit_bus_error_exit(body);
    body.push(OP_END);
}

pub(crate) fn emit_or(ar: u8, as_: u8, at: u8, out: &mut Vec<u8>) {
    op_local_get(out, as_);
    op_local_get(out, at);
    out.push(OP_I32_OR);
    op_local_set(out, ar);
}

pub(crate) fn emit_memw(_out: &mut Vec<u8>) {}

pub(crate) fn emit_nop(out: &mut Vec<u8>) {
    // A real wasm `nop` keeps instruction boundaries debuggable in module
    // dumps without affecting the stack.
    out.push(OP_NOP);
}

pub(crate) fn emit_l8ui(at: u8, as_: u8, imm: u32, out: &mut Vec<u8>) {
    emit_load(at, as_, imm, FN_READ_U8, Some(0xFF), out);
}

pub(crate) fn emit_l32i(at: u8, as_: u8, imm: u32, out: &mut Vec<u8>) {
    emit_load(at, as_, imm, FN_READ_U32, None, out);
}

pub(crate) fn emit_s8i(at: u8, as_: u8, imm: u32, out: &mut Vec<u8>) {
    emit_store(at, as_, imm, FN_WRITE_U8, Some(0xFF), out);
}

pub(crate) fn emit_s32i(at: u8, as_: u8, imm: u32, out: &mut Vec<u8>) {
    emit_store(at, as_, imm, FN_WRITE_U32, None, out);
}

#[allow(
    dead_code,
    reason = "kept so a future L32r-in-slice resolution pass can reuse the shape"
)]
pub(crate) fn emit_l32r(at: u8, literal: u32, out: &mut Vec<u8>) {
    op_i32_const(out, literal as i32);
    op_local_set(out, at);
}

pub(crate) fn emit_extui(ar: u8, at: u8, shift: u8, bits: u8, out: &mut Vec<u8>) {
    // Interpreter: (src >> shift) & mask, mask = u32::MAX when bits >= 32.
    let mask: i32 = if bits >= 32 {
        -1
    } else {
        ((1u32 << bits) - 1) as i32
    };
    op_local_get(out, at);
    op_i32_const(out, shift as i32);
    out.push(OP_I32_SHR_U);
    op_i32_const(out, mask);
    out.push(OP_I32_AND);
    op_local_set(out, ar);
}

pub(crate) fn emit_and(ar: u8, as_: u8, at: u8, out: &mut Vec<u8>) {
    op_local_get(out, as_);
    op_local_get(out, at);
    out.push(OP_I32_AND);
    op_local_set(out, ar);
}

pub(crate) fn emit_add(ar: u8, as_: u8, at: u8, out: &mut Vec<u8>) {
    op_local_get(out, as_);
    op_local_get(out, at);
    out.push(OP_I32_ADD);
    op_local_set(out, ar);
}

pub(crate) fn emit_sub(ar: u8, as_: u8, at: u8, out: &mut Vec<u8>) {
    op_local_get(out, as_);
    op_local_get(out, at);
    out.push(OP_I32_SUB);
    op_local_set(out, ar);
}

pub(crate) fn emit_xor(ar: u8, as_: u8, at: u8, out: &mut Vec<u8>) {
    op_local_get(out, as_);
    op_local_get(out, at);
    out.push(OP_I32_XOR);
    op_local_set(out, ar);
}

pub(crate) fn emit_addi(at: u8, as_: u8, imm8: i32, out: &mut Vec<u8>) {
    op_local_get(out, as_);
    op_i32_const(out, imm8);
    out.push(OP_I32_ADD);
    op_local_set(out, at);
}

pub(crate) fn emit_addmi(at: u8, as_: u8, imm: i32, out: &mut Vec<u8>) {
    op_local_get(out, as_);
    op_i32_const(out, imm);
    out.push(OP_I32_ADD);
    op_local_set(out, at);
}

pub(crate) fn emit_movi(at: u8, imm: i32, out: &mut Vec<u8>) {
    op_i32_const(out, imm);
    op_local_set(out, at);
}

// ── Wasm module assembly ──────────────────────────────────────────────

/// Assemble a complete generic module around `body` (which must already
/// end with the function's final `end`). Import table order is fixed —
/// [`FN_READ_U8`]..[`FN_BRANCH_TARGET`] — so both backends build their
/// host import objects against the same indices once.
fn build_generic_module(body: &[u8]) -> Vec<u8> {
    let mut module = Vec::with_capacity(body.len() + 256);
    module.extend_from_slice(b"\0asm");
    module.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);

    // Type section (1): run + two import signatures.
    let mut types = Vec::new();
    uleb(&mut types, 3);
    // type 0: (i32 x16) -> (i32 x18)
    types.push(0x60);
    uleb(&mut types, 16);
    types.extend(std::iter::repeat_n(0x7F, 16));
    uleb(&mut types, 18);
    types.extend(std::iter::repeat_n(0x7F, 18));
    // type 1: (i32) -> i32 — read_u8 / read_u32
    types.push(0x60);
    uleb(&mut types, 1);
    types.push(0x7F);
    uleb(&mut types, 1);
    types.push(0x7F);
    // type 2: (i32, i32) -> i32 — write_u8 / write_u32 / branch_target
    types.push(0x60);
    uleb(&mut types, 2);
    types.push(0x7F);
    types.push(0x7F);
    uleb(&mut types, 1);
    types.push(0x7F);
    push_section(&mut module, 1, &types);

    // Import section (2).
    let imports_spec: [(&str, u64); 5] = [
        ("read_u8", 1),
        ("read_u32", 1),
        ("write_u8", 2),
        ("write_u32", 2),
        ("branch_target", 2),
    ];
    let mut imports = Vec::new();
    uleb(&mut imports, imports_spec.len() as u64);
    for (name, ty) in imports_spec {
        push_name(&mut imports, "host");
        push_name(&mut imports, name);
        imports.push(0x00); // func import
        uleb(&mut imports, ty);
    }
    push_section(&mut module, 2, &imports);

    // Function section (3): one local function of type 0.
    let mut funcs = Vec::new();
    uleb(&mut funcs, 1);
    uleb(&mut funcs, 0);
    push_section(&mut module, 3, &funcs);

    // Export section (7): `run` = function index FN_RUN.
    let mut exports = Vec::new();
    uleb(&mut exports, 1);
    push_name(&mut exports, "run");
    exports.push(0x00); // func export
    uleb(&mut exports, FN_RUN as u64);
    push_section(&mut module, 7, &exports);

    // Code section (10): one body = locals + instructions.
    let mut fn_body = Vec::with_capacity(body.len() + 8);
    uleb(&mut fn_body, 1); // one local group
    uleb(&mut fn_body, LOCAL_COUNT);
    fn_body.push(0x7F); // i32
    fn_body.extend_from_slice(body);

    let mut code = Vec::new();
    uleb(&mut code, 1);
    uleb(&mut code, fn_body.len() as u64);
    code.extend_from_slice(&fn_body);
    push_section(&mut module, 10, &code);

    module
}

fn push_section(module: &mut Vec<u8>, id: u8, content: &[u8]) {
    module.push(id);
    uleb(module, content.len() as u64);
    module.extend_from_slice(content);
}

fn push_name(out: &mut Vec<u8>, name: &str) {
    uleb(out, name.len() as u64);
    out.extend_from_slice(name.as_bytes());
}

fn uleb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

fn sleb(out: &mut Vec<u8>, mut v: i64) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        let done = (v == 0 && byte & 0x40 == 0) || (v == -1 && byte & 0x40 != 0);
        if done {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

fn op_i32_const(out: &mut Vec<u8>, v: i32) {
    out.push(OP_I32_CONST);
    sleb(out, v as i64);
}

fn op_local_get(out: &mut Vec<u8>, idx: u8) {
    out.push(OP_LOCAL_GET);
    uleb(out, idx as u64);
}

fn op_local_set(out: &mut Vec<u8>, idx: u8) {
    out.push(OP_LOCAL_SET);
    uleb(out, idx as u64);
}

fn op_call(out: &mut Vec<u8>, func: u8) {
    out.push(OP_CALL);
    uleb(out, func as u64);
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

    /// The terminator-aware walker hands back the decoded terminator the
    /// generic emitter needs; the back-compat `walk_bb` still drops it.
    #[test]
    fn walker_returns_terminator() {
        let text: Vec<u8> = vec![0x3d, 0xf0, 0x0d, 0xf0];
        let walked = walk_bb_with_terminator(0, |pc| Some(pc as usize), &text, 16).unwrap();
        assert_eq!(walked.ops.len(), 1);
        let term = walked.terminator.expect("RET.N must be returned");
        assert_eq!(term.pc, 2);
        assert_eq!(term.len, 2);
        assert!(matches!(term.ins, Instruction::Ret));
    }

    /// Wide-byte helper for the hand-encoded test programs below.
    fn le3(word: u32) -> [u8; 3] {
        [
            (word & 0xFF) as u8,
            ((word >> 8) & 0xFF) as u8,
            ((word >> 16) & 0xFF) as u8,
        ]
    }

    /// `addi a2, a2, 7 ; bnez a2, self` — the canonical generic
    /// ALU+branch shape. Emit must produce a fresh (non-hot) module with
    /// the generic ABI and full side-exit vocabulary.
    #[test]
    fn walk_and_emit_generic_alu_branch() {
        // addi a2,a2,7 → 0x07C222; bnez a2 (pc 3, offset -6) → 0xFF6256.
        let text: Vec<u8> = [le3(0x07C222), le3(0xFF6256)].concat();
        let block = walk_and_emit(&text, 0, |pc| Some(pc as usize), PsBits::default())
            .expect("generic emit must accept ALU+branch");

        assert_eq!(block.abi, BlockAbi::Lx7Generic);
        assert_eq!(block.length_in_instrs, 2, "addi + the branch itself");
        assert_eq!(block.end_pc, 6, "branch falls through to pc+len");
        assert!(block.loads.is_empty());
        assert!(block.stores.is_empty());
        assert_eq!(
            block.reason_for(EXIT_BRANCH_TAKEN),
            Some(SideExitReason::BranchTaken)
        );
        assert_eq!(&block.wasm_bytes[0..4], b"\0asm");
    }

    /// Load/store+branch shape: the emitter must record the load and
    /// store manifests in execution order so dispatchers can pre-read and
    /// post-commit exactly what the body touches.
    #[test]
    fn walk_and_emit_generic_records_mem_manifests() {
        // l32i a5,a4,0 ; s32i a5,a4,0 ; addi a4,a4,-4 ; bnez a4,back.
        let text: Vec<u8> = [le3(0x002452), le3(0x006452), le3(0xFCC442), le3(0xFF3456)].concat();
        let block = walk_and_emit(&text, 0, |pc| Some(pc as usize), PsBits::default())
            .expect("generic emit must accept load/store+branch");

        assert_eq!(block.abi, BlockAbi::Lx7Generic);
        assert_eq!(block.length_in_instrs, 4);
        assert_eq!(block.end_pc, 12);
        assert_eq!(
            block.loads,
            vec![LoadReq {
                base: 4,
                imm: 0,
                width: MemWidth::U32
            }]
        );
        assert_eq!(
            block.stores,
            vec![StoreReq {
                base: 4,
                imm: 0,
                width: MemWidth::U32,
                value: 5
            }]
        );
    }

    /// 16-bit accesses are allowlisted by the walker so the emitter can
    /// refuse them *by name* instead of the walker silently refusing.
    #[test]
    fn walk_and_emit_refuses_16bit_named() {
        // l16ui a2, a4, 0 → 0x1422 (r=1); bnez a4 → 0xFF3456.
        let text: Vec<u8> = [le3(0x001422), le3(0xFF3456)].concat();
        let err = walk_and_emit(&text, 0, |pc| Some(pc as usize), PsBits::default()).unwrap_err();
        assert!(
            matches!(
                err,
                EmitError::UnsupportedOpForEmit(Instruction::L16ui { .. })
            ),
            "expected named L16ui refusal, got {err:?}"
        );
    }

    /// A load whose base register was written earlier must be refused by
    /// name: the host pre-stages manifest loads from the *entry* register
    /// file, so emitting it would fetch from a stale address.
    #[test]
    fn walk_and_emit_refuses_clobbered_load_base_named() {
        // l32i a2,a3,0 ; l8ui a4,a2,0 ; bnez a2,self.
        let text: Vec<u8> = [le3(0x002322), le3(0x000242), le3(0xFF6256)].concat();
        let err = walk_and_emit(&text, 0, |pc| Some(pc as usize), PsBits::default()).unwrap_err();
        assert!(
            matches!(err, EmitError::LoadBaseClobbered { base: 2, pc: 3 }),
            "expected named clobbered-base refusal, got {err:?}"
        );
    }

    /// A base register written *after* its load is fine: the host stages
    /// from the entry value, which is exactly what the body would read.
    #[test]
    fn walk_and_emit_allows_load_base_written_after_load() {
        // l32i a5,a4,0 ; addi a4,a4,-4 ; bnez a5,self.
        let text: Vec<u8> = [le3(0x002452), le3(0xFCC442), le3(0xFF6556)].concat();
        let block = walk_and_emit(&text, 0, |pc| Some(pc as usize), PsBits::default())
            .expect("base clobbered after the load must still emit");
        assert_eq!(block.loads.len(), 1);
    }

    /// `J` retires through its own wire code so dispatchers can advance PC
    /// without setting `branched` (the interpreter's `J` arm leaves it
    /// clear; conditional branches go through `EXIT_BRANCH_TAKEN`).
    #[test]
    fn walk_and_emit_jump_uses_jump_exit_code() {
        // addi a2,a2,7 ; j 0 (branch at pc 3, target 0 → imm18 = -7).
        let text: Vec<u8> = [le3(0x07C222), le3(0xFFFE46)].concat();
        let block = walk_and_emit(&text, 0, |pc| Some(pc as usize), PsBits::default()).unwrap();
        assert_eq!(block.abi, BlockAbi::Lx7Generic);
        assert_eq!(block.length_in_instrs, 2);
        assert_eq!(
            block.end_pc, 6,
            "J falls through to pc+len when not taken (never)"
        );
        assert_eq!(block.reason_for(EXIT_BRANCH_TAKEN), None);
        assert_eq!(
            block.reason_for(EXIT_JUMP_TAKEN),
            Some(SideExitReason::JumpTaken)
        );
    }

    /// `CALL8` terminators stay a Phase 4.4 problem: named refusal, not
    /// a half-emitted block.
    #[test]
    fn walk_and_emit_refuses_call_terminator_named() {
        // l8ui a2,a3,0 ; call8 (terminator, op0=5, n=2 → 0x25).
        let text: Vec<u8> = [le3(0x000322), le3(0x000025)].concat();
        let err = walk_and_emit(&text, 0, |pc| Some(pc as usize), PsBits::default()).unwrap_err();
        assert!(
            matches!(
                err,
                EmitError::UnsupportedTerminatorForEmit(Instruction::Call8 { .. })
            ),
            "expected named CALL8 refusal, got {err:?}"
        );
    }

    /// A block without a terminator inside the budget refuses with the
    /// cap spelled out rather than truncating silently.
    #[test]
    fn walk_and_emit_refuses_over_budget() {
        // 17 NOP.Ns — one past GENERIC_MAX_OPS, no terminator.
        let mut text = Vec::new();
        for _ in 0..(GENERIC_MAX_OPS + 1) {
            text.extend_from_slice(&[0x3d, 0xf0]);
        }
        let err = walk_and_emit(&text, 0, |pc| Some(pc as usize), PsBits::default()).unwrap_err();
        assert_eq!(
            err,
            EmitError::TooManyOps {
                len: GENERIC_MAX_OPS,
                cap: GENERIC_MAX_OPS
            }
        );
    }

    /// Emit is deterministic: identical input bytes must produce
    /// byte-identical modules (the browser/native caches key on PC but
    /// both adapters rely on the emit contract).
    #[test]
    fn generic_emit_is_deterministic() {
        let text: Vec<u8> = [le3(0x07C222), le3(0xFF6256)].concat();
        let a = walk_and_emit(&text, 0, |pc| Some(pc as usize), PsBits::default()).unwrap();
        let b = walk_and_emit(&text, 0, |pc| Some(pc as usize), PsBits::default()).unwrap();
        assert_eq!(a.wasm_bytes, b.wasm_bytes);
    }

    /// `walk_and_emit` propagates walker failures as `WalkRefused`.
    #[test]
    fn walk_and_emit_walker_refused_propagates() {
        // SSL a3 — refused by walker.
        let text: Vec<u8> = vec![0x40, 0x13, 0x40];
        let err = walk_and_emit(&text, 0, |pc| Some(pc as usize), PsBits::default()).unwrap_err();
        assert_eq!(err, EmitError::WalkRefused);
    }

    /// `SideExitReason::wire_code` round-trips through
    /// `EmittedBlock::reason_for` (sanity for backends that index by
    /// the i32 returned from wasm).
    #[test]
    fn side_exit_reason_round_trips() {
        let block = EmittedBlock {
            wasm_bytes: vec![0, b'a', b's', b'm', 1, 0, 0, 0],
            abi: BlockAbi::HotBb,
            length_in_instrs: 1,
            end_pc: 0,
            loads: Vec::new(),
            stores: Vec::new(),
            side_exit_reasons: vec![
                (EXIT_FALL_THROUGH, SideExitReason::FallThrough),
                (EXIT_BRANCH_TAKEN, SideExitReason::BranchTaken),
                (EXIT_HOST_BUS_ERROR, SideExitReason::HostBusError),
            ],
        };
        assert_eq!(
            block.reason_for(EXIT_FALL_THROUGH),
            Some(SideExitReason::FallThrough)
        );
        assert_eq!(
            block.reason_for(EXIT_BRANCH_TAKEN),
            Some(SideExitReason::BranchTaken)
        );
        assert_eq!(
            block.reason_for(EXIT_HOST_BUS_ERROR),
            Some(SideExitReason::HostBusError)
        );
        assert_eq!(block.reason_for(42), None);
    }

    /// Signed LEB128 must round-trip the full i32 range the emitter uses
    /// for immediates and offsets (including the negative branch offsets
    /// that are the common case).
    #[test]
    fn sleb128_round_trips_i32() {
        for v in [0i32, 1, -1, 63, 64, -64, -65, i32::MIN, i32::MAX, -10, 1020] {
            let mut bytes = Vec::new();
            sleb(&mut bytes, v as i64);
            // Minimal decoder for the test.
            let mut result: i64 = 0;
            let mut shift = 0;
            let mut i = 0;
            loop {
                let b = bytes[i];
                result |= ((b & 0x7F) as i64) << shift;
                shift += 7;
                i += 1;
                if b & 0x80 == 0 {
                    if shift < 64 && b & 0x40 != 0 {
                        result |= -1i64 << shift;
                    }
                    break;
                }
            }
            assert_eq!(result as i32, v, "sleb round-trip for {v}");
        }
    }
}
