// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RV32IMC integer-ALU + load/store wasm codegen (JIT framework chunks C+E).
//!
//! Chunk C turned a straight-line run of integer arithmetic / logical /
//! shift / mul-div instructions into a real wasm module. **Chunk E** widens
//! that emittable prefix to include RV32(I)C **loads and stores**
//! (`Lb,Lh,Lw,Lbu,Lhu,Sb,Sh,Sw` + compressed `CLw,CSw,CLwsp,CSwsp`): a
//! load/store does not end the block, it extends it.
//!
//! ## The register-file-in-locals model (chunk C)
//!
//! The compiled module imports one memory. Bytes `0..128` are the **guest
//! register file**, word `i` = `xi` at byte offset `i*4` (see
//! [`wasm_encode::REGS_IMPORT_MODULE`]). Each block:
//!
//!   1. **Prologue** — loads every register it *reads* into a wasm local
//!      (`x0..x31` map to locals `0..31`).
//!   2. **Body** — one emit per guest instruction, operating on locals.
//!   3. **Epilogue** — stores every register it *wrote* back to that memory.
//!
//! ## Memory binding + fault exit (chunk E)
//!
//! The guest's RAM is *not* the register-file bytes. A load/store's effective
//! address (`rs1 + imm`) is data-dependent, so the block emits an **in-wasm
//! range check** against a single contiguous guest-RAM window
//! `[ram_base, ram_base+ram_len)` (the machine's `bus.ram`, which the
//! interpreter's bus routes with priority over flash/peripherals under the
//! default config). The same one imported memory carries that RAM window at
//! byte offset [`RAM_WINDOW_OFF`]; the runtime ([`super::exec`]) copies the
//! guest RAM in before the call and back out after, but **only for blocks
//! that touch memory** — pure-ALU blocks are byte-for-byte identical to
//! chunk C (register-only import, one page, no RAM sync).
//!
//!   * **In window** → inline `iN.load`/`iN.store` at `ea - ram_base +
//!     RAM_WINDOW_OFF` (the fast path). Widths + sign/zero extension come
//!     straight from the wasm opcode (`i32.load8_s` … `i32.load16_u`).
//!   * **Out of window** (MMIO / unknown) → a **side-exit**: the block writes
//!     the *faulting instruction's* PC and the count of instructions it
//!     retired *before* the fault to dedicated slots, flushes the registers
//!     those prior instructions wrote, and returns [`WIRE_MEM_FAULT`]. The
//!     runtime resumes the interpreter at exactly that PC so it performs the
//!     real bus access (MMIO side effects stay interpreter-owned) and
//!     re-executes only the faulting load/store.
//!
//! Because a fault can occur mid-block, the writeback on the fault path
//! flushes only the registers written by instructions *strictly before* the
//! faulting one (a snapshot taken at emit time); the faulting op and every
//! instruction after it are left for the interpreter.
//!
//! Stores also clear the LR/SC `reservation`. That lives in CPU state, not in
//! the imported memory, so an inline in-window store records "a store ran"
//! into [`RES_FLAG_SLOT`]; the runtime clears `cpu.reservation` iff that flag
//! is set (a store that *faulted* is cleared by the interpreter that replays
//! it). This mirrors `RiscV::step` exactly — reservation is cleared iff a
//! store actually executed — with no spurious over-clearing (which would
//! livelock an LR/SC spinlock).

use crate::decoder::riscv::{decode_rv32, Instruction};

use super::super::frontend::ExitEdge;
use super::super::side_exit::BailReason;
use super::super::{CodeView, Pc};
use super::inst_len;
use super::wasm_encode::{build_module, enc, op};

/// Wire code the emitted body returns on a clean straight-line fall-through
/// to `end_pc`. The runtime maps it to
/// [`SideExit::Chain`](super::super::side_exit::SideExit::Chain) to `end_pc`.
pub const WIRE_FALL_THROUGH: i32 = 0;

/// Wire code a control-flow-terminated block (chunk D) returns. The block has
/// already resolved its taken/not-taken/jump-target address **in wasm** and
/// written it to the [`NEXT_PC_SLOT`] word of the register memory; the runtime
/// reads that slot and maps this code to
/// [`SideExit::Chain`](super::super::side_exit::SideExit::Chain) to it. One
/// code covers every branch/jump kind — conditional branches pick the address
/// with an in-wasm `if`, and `JALR`/`C.JR` compute a data-dependent one — so
/// the runtime never needs a per-terminator code.
pub const WIRE_CHAIN_DYNAMIC: i32 = 1;

/// Wire code for a **memory fault** side-exit (chunk E): a load/store whose
/// effective address fell outside the bound guest-RAM window. The runtime
/// maps it to
/// [`SideExit::EnterInterpreter`](super::super::side_exit::SideExit::EnterInterpreter)
/// with [`BailReason::MemoryFault`], resuming at the faulting PC published in
/// [`FAULT_PC_SLOT`].
///
/// The three wire codes are disjoint — `0` fall-through (C), `1` dynamic-chain
/// (D), `2` memory-fault (E) — and add non-overlapping arms to the single
/// `match` in [`CompiledBlock::run`](super::exec::CompiledBlock).
pub const WIRE_MEM_FAULT: i32 = 2;

/// Number of `i32` locals mapped to guest registers `x0..x31` (local index ==
/// register number).
const REG_LOCALS: u32 = 32;

/// Local index of the scratch register the memory path stashes an effective
/// address in (declared only when a block emits at least one load/store).
const SCRATCH_LOCAL: u32 = REG_LOCALS;

// ── Dedicated control slots in the imported memory ─────────────────────────
//
// The register file occupies bytes 0..128 (x0..x31). Chunk D claims word 32
// (byte 128) as its dynamic-chain next-PC slot; chunk E's three 4-byte control
// slots start at word 33 (byte 132) to stay clear of both the register bytes
// and D's slot; the guest-RAM window is mapped far above at byte 256. The
// three slot regions are disjoint — do not renumber without reconciling all.

/// Byte offset in the imported register memory of the **dynamic next-PC slot**
/// a [`WIRE_CHAIN_DYNAMIC`] block writes its resolved continuation address to.
/// It sits at word `32`, immediately past `x31` (bytes `0..128`), so it never
/// aliases a guest register. The runtime syncs `132` bytes to carry it.
pub const NEXT_PC_SLOT: i32 = 32 * 4;

/// Byte offset of the slot the faulting load/store writes its resume PC to
/// (word 33 — deliberately *not* D's word-32 next-PC slot at 128).
pub const FAULT_PC_SLOT: u32 = 132;
/// Byte offset of the slot the faulting load/store writes its retired-so-far
/// instruction count to (instructions the block completed *before* the fault).
pub const FAULT_RETIRED_SLOT: u32 = 136;
/// Byte offset of the "a store executed inline" flag (`0`/`1`). The runtime
/// clears `cpu.reservation` iff this is non-zero after the run.
pub const RES_FLAG_SLOT: u32 = 140;
/// Byte offset in the imported memory where the bound guest-RAM window begins.
/// Guest address `a` maps to wasm byte `a - ram_base + RAM_WINDOW_OFF`.
pub const RAM_WINDOW_OFF: u32 = 256;

/// The guest-RAM window a load/store block binds against: `(base, len)` bytes.
pub type RamWindow = (u32, u32);

/// Memory-binding metadata for a compiled block that touches RAM. Absent for
/// pure-ALU blocks (which import a register-only single-page memory and never
/// sync RAM).
#[derive(Debug, Clone, Copy)]
pub struct MemBinding {
    /// Number of guest-RAM bytes to sync in/out around a block run.
    pub ram_len: usize,
    /// Whether the block contains any store (drives the reservation clear).
    pub has_store: bool,
}

/// Is `inst` an integer-ALU instruction chunk C emits wasm for?
pub fn is_alu_emittable(inst: &Instruction) -> bool {
    use Instruction::*;
    matches!(
        inst,
        Lui { .. }
            | Auipc { .. }
            | Addi { .. }
            | Slti { .. }
            | Sltiu { .. }
            | Xori { .. }
            | Ori { .. }
            | Andi { .. }
            | Slli { .. }
            | Srli { .. }
            | Srai { .. }
            | Add { .. }
            | Sub { .. }
            | Sll { .. }
            | Slt { .. }
            | Sltu { .. }
            | Xor { .. }
            | Srl { .. }
            | Sra { .. }
            | Or { .. }
            | And { .. }
            | Mul { .. }
            | Mulh { .. }
            | Mulhsu { .. }
            | Mulhu { .. }
            | Div { .. }
            | Divu { .. }
            | Rem { .. }
            | Remu { .. }
            | CAddi { .. }
            | CLi { .. }
            | CMv { .. }
            | CAddi16sp { .. }
            | CAddi4spn { .. }
            | CSli { .. }
    )
}

/// Is `inst` a load/store chunk E emits wasm for?
pub fn is_mem_emittable(inst: &Instruction) -> bool {
    use Instruction::*;
    matches!(
        inst,
        Lb { .. }
            | Lh { .. }
            | Lw { .. }
            | Lbu { .. }
            | Lhu { .. }
            | Sb { .. }
            | Sh { .. }
            | Sw { .. }
            | CLw { .. }
            | CSw { .. }
            | CLwsp { .. }
            | CSwsp { .. }
    )
}

/// Is `inst` a control-flow terminator chunk D emits wasm for?
///
/// These are the branch/jump instructions the block ends **at and includes**.
/// A block is the maximal ALU + load/store prefix followed by at most one of
/// these; the terminator resolves the block's next PC in wasm and returns
/// [`WIRE_CHAIN_DYNAMIC`]. `C.JAL` is absent because the decoder maps it to
/// `Jal { rd: 1, .. }`, already covered here.
pub fn is_terminator_emittable(inst: &Instruction) -> bool {
    use Instruction::*;
    matches!(
        inst,
        Beq { .. }
            | Bne { .. }
            | Blt { .. }
            | Bge { .. }
            | Bltu { .. }
            | Bgeu { .. }
            | Jal { .. }
            | Jalr { .. }
            | CJ { .. }
            | CJr { .. }
            | CJalr { .. }
            | CBeqz { .. }
            | CBnez { .. }
    )
}

/// Is `inst` emittable in the straight-line block **body**, given whether a
/// RAM window is bound? This is the ALU set (chunk C) unioned with the
/// load/store set (chunk E). Terminators are *not* body ops — they end the
/// block ([`is_terminator_emittable`]).
///
/// Loads/stores are only emittable when a window is present; without one the
/// block ends before them and the interpreter takes over (identical to the
/// chunk-C behaviour).
fn is_emittable(inst: &Instruction, mem_ok: bool) -> bool {
    is_alu_emittable(inst) || (mem_ok && is_mem_emittable(inst))
}

/// One decoded body instruction plus the guest PC it sits at (needed for
/// `AUIPC`, which folds `pc` into a constant, and for the memory-fault resume
/// PC).
struct Op {
    pc: u32,
    inst: Instruction,
}

/// The result of emitting a block: the wasm bytes plus the metadata the
/// frontend stamps onto its [`BlockPlan`](super::super::frontend::BlockPlan).
pub struct EmittedBlock {
    /// Real wasm module bytes (magic-prefixed), consumed by the runtime.
    pub code: Vec<u8>,
    /// Guest PC one past the last instruction the block subsumes. For a
    /// fall-through (chunk-C/E) block this is the continuation the runtime
    /// chains to; for a terminator (chunk-D) block the continuation is
    /// dynamic (written to [`NEXT_PC_SLOT`]) and this is metadata only.
    pub end_pc: Pc,
    /// Number of guest instructions the block retires in one clean run.
    pub instr_count: u32,
    /// Side-exit edges. The primary (clean-exit) edge is the fall-through
    /// ([`WIRE_FALL_THROUGH`]) for a body-only block or the dynamic chain
    /// ([`WIRE_CHAIN_DYNAMIC`]) when it ends at a branch/jump; a block that
    /// touches RAM adds the memory-fault edge ([`WIRE_MEM_FAULT`]).
    pub exits: Vec<ExitEdge>,
    /// Memory-binding metadata, present iff the block contains a load/store.
    pub binding: Option<MemBinding>,
}

/// Emit a wasm block for the maximal emittable body prefix at `pc`, optionally
/// ended by one control-flow terminator (chunks C+D+E).
///
/// `window` is the guest-RAM window (`bus.ram`) loads/stores bind against;
/// when `None`, loads/stores are treated as non-emittable and the body ends
/// before the first one (chunk-C behaviour).
///
/// - The block body is the maximal straight-line prefix of ALU (chunk C) and
///   in-window load/store (chunk E) ops.
/// - If a branch/jump ([`is_terminator_emittable`]) follows that prefix (or
///   sits at `pc` with no prefix), the block **includes** it, resolves its
///   next PC in wasm, and exits with [`WIRE_CHAIN_DYNAMIC`]; otherwise it exits
///   with [`WIRE_FALL_THROUGH`] to `end_pc`.
///
/// Returns `None` when there is nothing to emit at `pc` — neither a body op
/// nor an emittable terminator (the caller keeps that PC on the interpreter —
/// never an error).
pub fn emit_block(pc: Pc, code: &CodeView<'_>, window: Option<RamWindow>) -> Option<EmittedBlock> {
    let ops = walk_ops(pc, code, window.is_some());
    let prefix_end = pc + ops.iter().map(|o| inst_len_of(o.pc, code)).sum::<u64>();

    // The instruction immediately after the body prefix; the block ends at it
    // and includes it when it is an emittable control-flow terminator.
    let terminator = decode_at(prefix_end, code).filter(|(inst, _)| is_terminator_emittable(inst));

    if ops.is_empty() && terminator.is_none() {
        return None;
    }

    // Emit the body ops (ALU + in-window load/store) into a scratch buffer,
    // recording read/write sets and memory-binding facts.
    let mut body = Body {
        window,
        ..Body::default()
    };
    for aop in &ops {
        body.emit_instruction(aop.pc, &aop.inst);
    }

    // Choose the terminating shape: dynamic chain (branch/jump) or the plain
    // fall-through wire. Both carry `PartialBlock` — telemetry only; the
    // runtime's resolved `Chain` is the correctness contract.
    let (end_pc, instr_count, wire) = if let Some((tinst, tlen)) = terminator {
        body.emit_terminator(prefix_end as u32, tlen as u32, &tinst);
        (prefix_end + tlen, ops.len() as u32 + 1, WIRE_CHAIN_DYNAMIC)
    } else {
        (prefix_end, ops.len() as u32, WIRE_FALL_THROUGH)
    };

    let mut expr = Vec::with_capacity(body.buf.len() + 16 * REG_LOCALS as usize);
    body.emit_prologue(&mut expr); // loads touched regs into locals
    expr.extend_from_slice(&body.buf); // body + terminator, on locals + inline RAM/slots
    body.emit_epilogue(&mut expr); // stores written regs back to mem
    expr.push(op::I32_CONST); // the block's clean-exit return value
    enc::sleb(&mut expr, wire as i64);

    let local_count = if body.scratch_used {
        REG_LOCALS + 1
    } else {
        REG_LOCALS
    };

    let binding = if body.has_mem {
        // len already validated as Some via `window.is_some()` gating the walk.
        let (_base, len) = window.expect("mem op emitted without a RAM window");
        Some(MemBinding {
            ram_len: len as usize,
            has_store: body.has_store,
        })
    } else {
        None
    };

    // A register-only block needs one page; a RAM-binding block needs enough
    // pages to cover the window mapped at RAM_WINDOW_OFF.
    let mem_pages = match &binding {
        Some(b) => (RAM_WINDOW_OFF as usize + b.ram_len)
            .max(1)
            .div_ceil(65536)
            .max(1) as u32,
        None => 1,
    };

    let code_bytes = build_module(local_count, mem_pages, &expr);

    // Primary clean-exit edge: the dynamic chain for a terminator block, else
    // the fall-through. A block that touches RAM adds the memory-fault edge.
    let mut exits = vec![ExitEdge {
        wire_code: wire,
        reason: BailReason::PartialBlock,
    }];
    if body.has_mem {
        exits.push(ExitEdge {
            wire_code: WIRE_MEM_FAULT,
            reason: BailReason::MemoryFault,
        });
    }

    Some(EmittedBlock {
        code: code_bytes,
        end_pc,
        instr_count,
        exits,
        binding,
    })
}

// ── Superblock / trace fusion (Milestone 1) ────────────────────────────────
//
// See [`emit_trace`] for the deadline-correctness design.

/// Byte offset of the **retired-instruction budget** slot a trace reads at
/// entry. The host writes the number of instructions the trace may retire
/// before it must return control (see the design note on [`emit_trace`]). Sits
/// past every other control slot (`RES_FLAG_SLOT` ends at 144) and below the
/// RAM window (256), so it aliases nothing.
pub const BUDGET_SLOT: u32 = 144;

/// Byte offset where a trace publishes the number of instructions it actually
/// retired on a [`WIRE_CHAIN_DYNAMIC`] exit. Unlike a single basic block
/// (which retires a fixed `instr_count`), a trace retires a runtime-variable
/// count — the sum of the interior blocks it ran before exiting — so the
/// runtime reads this slot rather than the artifact's static count. (The
/// memory-fault path already publishes its retired-so-far via
/// [`FAULT_RETIRED_SLOT`].)
pub const TRACE_RETIRED_SLOT: u32 = 148;

/// wasm local holding the trace's resolved *next PC* (the interior dispatch
/// key). `x0..x31` = locals 0..31, [`SCRATCH_LOCAL`] = 32.
const NEXT_LOCAL: u32 = REG_LOCALS + 1;
/// wasm local counting instructions retired by fully-completed interior blocks.
const RETIRED_LOCAL: u32 = REG_LOCALS + 2;
/// wasm local holding the entry budget loaded from [`BUDGET_SLOT`].
const BUDGET_LOCAL: u32 = REG_LOCALS + 3;
/// Locals a trace declares: `x0..x31` + scratch + next + retired + budget.
const TRACE_LOCALS: u32 = REG_LOCALS + 4;

/// Backstop caps on trace size (a trace is otherwise bounded by hitting a
/// dynamic terminator / unmodeled cut / out-of-window edge).
const MAX_TRACE_BLOCKS: usize = 32;
pub const MAX_TRACE_INSTRS: u32 = 2048;

/// One basic block fused into a trace: its entry PC, straight-line ops, and
/// optional control-flow terminator.
struct TraceBlock {
    entry_pc: u32,
    ops: Vec<Op>,
    /// `(inst, byte-length)`; `None` for a fall-through block (ran into an
    /// unmodeled cut / view end — the trace exits to the host there).
    terminator: Option<(Instruction, u64)>,
    /// PC of the terminator (== one past the last body op).
    prefix_end: u32,
    /// Guest instructions the block retires (ops + terminator).
    instr_count: u32,
}

/// Return site (the PC immediately after the call) of a **linking direct call**
/// — a `JAL`/`C.JAL` that writes a link register (`ra`=x1 or `t0`=x5).
///
/// This is the address the callee's `ret` (a dynamic `JALR`/`C.JR` on `ra`)
/// jumps back to. Milestone 2 queues it into the trace so that, once the callee
/// has been fused (via the call's *static* target — see [`static_successors`]),
/// the callee's `ret` re-dispatches **in-wasm** to this site instead of paying a
/// host round-trip: the interior dispatch loop's `$next == entry_pc` equality is
/// the inline-cache guard, and a return to any other address simply misses and
/// exits to the host exactly as before. Deadline budget, register-liveness union
/// and the exit-flush already cover every fused block, so the spliced return site
/// inherits all of M1's correctness for free.
///
/// `None` for non-linking jumps and branches (no call/return pairing to exploit)
/// and for `C.JALR`/indirect `JALR` calls, whose target is dynamic — their
/// callee is never fused, so their return site could never be reached in-wasm
/// and splicing it would only bloat the trace.
fn call_return_site(inst: &Instruction, pc: u32, ilen: u64) -> Option<Pc> {
    use Instruction::*;
    match *inst {
        Jal { rd, .. } if rd == 1 || rd == 5 => Some((pc as u64).wrapping_add(ilen) as Pc),
        _ => None,
    }
}

/// Statically-known successor PCs of a terminator sitting at `pc` (its own PC),
/// `ilen` bytes long. Empty for a dynamic terminator (`JALR`/`C.JR`/`C.JALR`),
/// which the trace cannot follow — it exits to the host and the interpreter
/// resolves the target.
fn static_successors(inst: &Instruction, pc: u32, ilen: u64) -> Vec<Pc> {
    use Instruction::*;
    let taken = |imm: i32| pc.wrapping_add(imm as u32) as Pc;
    let fall = (pc as u64) + ilen;
    match *inst {
        Beq { imm, .. } | Bne { imm, .. } | Blt { imm, .. } | Bge { imm, .. }
        | Bltu { imm, .. } | Bgeu { imm, .. } => vec![taken(imm), fall],
        CBeqz { imm, .. } | CBnez { imm, .. } => vec![taken(imm), fall],
        Jal { imm, .. } => vec![taken(imm)],
        CJ { imm } => vec![taken(imm)],
        // Dynamic — the trace ends here.
        Jalr { .. } | CJr { .. } | CJalr { .. } => vec![],
        _ => vec![],
    }
}

/// Walk a single block at `pc` (the maximal emittable body prefix plus at most
/// one terminator), returning `None` if `pc` is not compilable (no body op and
/// no emittable terminator — the interpreter owns it).
fn walk_trace_block(pc: Pc, code: &CodeView<'_>, mem_ok: bool) -> Option<TraceBlock> {
    let ops = walk_ops(pc, code, mem_ok);
    let prefix_end = pc + ops.iter().map(|o| inst_len_of(o.pc, code)).sum::<u64>();
    let terminator =
        decode_at(prefix_end, code).filter(|(inst, _)| is_terminator_emittable(inst));
    if ops.is_empty() && terminator.is_none() {
        return None;
    }
    let instr_count = ops.len() as u32 + terminator.is_some() as u32;
    Some(TraceBlock {
        entry_pc: pc as u32,
        ops,
        terminator,
        prefix_end: prefix_end as u32,
        instr_count,
    })
}

/// Greedily collect the trace rooted at `entry`, following only *static*
/// control-flow edges (branches / `JAL` / `C.J` with compile-time targets)
/// that stay inside the fetched `code` view. `blocks[0]` is always the entry
/// block. Stops at a dynamic terminator, an unmodeled cut, an out-of-view edge,
/// or the [`MAX_TRACE_BLOCKS`]/[`MAX_TRACE_INSTRS`] caps.
fn collect_trace(entry: Pc, code: &CodeView<'_>, mem_ok: bool) -> Vec<TraceBlock> {
    let mut blocks: Vec<TraceBlock> = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    let mut queue: Vec<Pc> = vec![entry];
    let mut total = 0u32;
    while let Some(pc) = queue.pop() {
        if seen.contains(&(pc as u32)) {
            continue;
        }
        let Some(block) = walk_trace_block(pc, code, mem_ok) else {
            continue; // edge into a non-compilable PC — trace exits there.
        };
        seen.push(pc as u32);
        total += block.instr_count;
        if let Some((tinst, tlen)) = &block.terminator {
            for succ in static_successors(tinst, block.prefix_end, *tlen) {
                if !seen.contains(&(succ as u32)) && code.covers(succ) {
                    queue.push(succ);
                }
            }
            // M2 — JALR/return inline caching. Also queue the RETURN SITE of a
            // linking direct call so the fused callee's `ret` re-dispatches
            // in-wasm (guarded by the loop's `$next == entry_pc` equality)
            // instead of exiting to the host. Byte-identity is preserved: a
            // return whose resolved target is not a fused block entry misses the
            // dispatch and exits exactly as M1, and every spliced block is
            // covered by the same deadline budget / liveness union / exit flush.
            if let Some(site) = call_return_site(tinst, block.prefix_end, *tlen) {
                if !seen.contains(&(site as u32)) && code.covers(site) {
                    queue.push(site);
                }
            }
        }
        blocks.push(block);
        if blocks.len() >= MAX_TRACE_BLOCKS || total >= MAX_TRACE_INSTRS {
            break;
        }
    }
    blocks
}

/// Harvest the union register read/write sets and memory facts of a trace by
/// dry-emitting every block into a throwaway [`Body`] (its buffer is
/// discarded). The unions drive the single entry-load / exit-store and the
/// fault-path writeback (every write-local must be prologue-loaded so a
/// re-store of an untaken write is a no-op).
fn harvest_trace(blocks: &[TraceBlock], window: Option<RamWindow>) -> ([bool; 32], [bool; 32], bool, bool) {
    let mut b = Body {
        window,
        ..Body::default()
    };
    for blk in blocks {
        for op in &blk.ops {
            b.emit_instruction(op.pc, &op.inst);
        }
        if let Some((tinst, tlen)) = &blk.terminator {
            b.emit_terminator(blk.prefix_end, *tlen as u32, tinst);
        }
    }
    (b.reads, b.writes, b.has_mem, b.has_store)
}

/// Emit a fused **trace** (superblock) rooted at `pc`: many basic blocks in one
/// wasm function, guest registers kept in locals across every interior block
/// boundary, interior control flow resolved by an if-chain dispatch loop on a
/// `$next` PC local. Returns `None` when the trace would be a single block
/// (no interior fusion possible — the caller uses the per-block path) or the
/// entry is not compilable.
///
/// ## Deadline-safe atomic retirement (the correctness contract)
///
/// A fused trace retires many guest instructions in one host call. To stay
/// byte-identical to the per-instruction interpreter it must **never retire
/// past a point where the interpreter would have stopped** — the batch budget
/// (`max_count`), or a timer/IRQ trap. The per-block JIT enforces this by
/// *refusing* to run a block that would cross such a point (`retired + n >
/// max_count || block_would_cross_irq(bus, n)` in `run_jit_loop`) and
/// single-stepping instead. A trace generalises that guard to the interior.
///
/// The host computes a single **retired-instruction budget** at trace entry —
/// `min(max_count - retired, mtimecmp - mtime - 1 [iff the M-timer trap is
/// armed])` — and writes it to [`BUDGET_SLOT`]; the trace loads it into
/// `$budget`. This is exact because inside a trace *nothing but the retired
/// count advances the clock*: `mtime` is bumped by the host afterward by
/// exactly the retired count, and mtimecmp/mip/mie/mstatus/external-lines are
/// mutated only by CSR writes, traps, or MMIO — all of which are unmodeled or
/// out-of-window and therefore **exit the trace**. So "instructions retired"
/// and "cycles until the next deadline" are the same unit, and the budget
/// computed once at entry stays valid for the whole trace.
///
/// Before running each interior block `k` (retiring `n_k`), the trace checks
/// `$retired + n_k > $budget`; if so it exits to the host at block `k`'s entry
/// PC having retired only the fully-completed prior blocks — exactly what the
/// per-block guard would have done. The host then single-steps, so the trap /
/// batch edge lands on the identical instruction as a pure interpreter run.
///
/// Every rare edge exits to the host exactly as the per-block path does: a
/// dynamic terminator (`JALR`/`C.JR`) writes its resolved PC to `$next` and,
/// finding no interior match, falls out to the [`WIRE_CHAIN_DYNAMIC`] exit; an
/// out-of-window load/store publishes its fault and returns [`WIRE_MEM_FAULT`];
/// an unmodeled cut is never fused (the block simply isn't in the trace) so its
/// predecessor exits to the host at it.
pub fn emit_trace(pc: Pc, code: &CodeView<'_>, window: Option<RamWindow>) -> Option<EmittedBlock> {
    let mem_ok = window.is_some();
    let blocks = collect_trace(pc, code, mem_ok);
    // A trace is only worthwhile when the entry block ends in a *static*
    // terminator (so the dispatch loop can re-enter an interior block, e.g. a
    // hot loop's back-edge). Otherwise the per-block path already covers it.
    let entry_static = blocks
        .first()
        .and_then(|b| b.terminator.as_ref())
        .is_some_and(|(t, l)| !static_successors(t, blocks[0].prefix_end, *l).is_empty());
    if blocks.len() < 2 && !(blocks.len() == 1 && entry_static) {
        return None;
    }
    if !entry_static {
        return None;
    }

    let (u_reads, u_writes, has_mem, has_store) = harvest_trace(&blocks, window);
    // Every written register is also read (prologue-loaded), so a full
    // union-write flush at any exit re-stores an unmodified original as a no-op.
    let mut load_set = u_reads;
    for r in 0..32 {
        load_set[r] |= u_writes[r];
    }

    let entry_instr_count = blocks[0].instr_count;
    let entry_end_pc = blocks[0].prefix_end as u64
        + blocks[0].terminator.as_ref().map_or(0, |(_, l)| *l);

    // ── Emit the body into a Body with trace wiring. ──────────────────────
    let mut t = Body {
        window,
        trace_next_local: Some(NEXT_LOCAL),
        trace_union_writes: Some(u_writes),
        ..Body::default()
    };

    // Prologue: load the union live-in / maybe-written regs into locals.
    let mut expr: Vec<u8> = Vec::new();
    for r in 1..32u8 {
        if load_set[r as usize] {
            expr.push(op::I32_CONST);
            enc::sleb(&mut expr, (r as i64) * 4);
            expr.push(op::I32_LOAD);
            enc::uleb(&mut expr, 2);
            enc::uleb(&mut expr, 0);
            expr.push(op::LOCAL_SET);
            enc::uleb(&mut expr, r as u64);
        }
    }
    // $budget = load BUDGET_SLOT
    expr.push(op::I32_CONST);
    enc::sleb(&mut expr, BUDGET_SLOT as i64);
    expr.push(op::I32_LOAD);
    enc::uleb(&mut expr, 2);
    enc::uleb(&mut expr, 0);
    expr.push(op::LOCAL_SET);
    enc::uleb(&mut expr, BUDGET_LOCAL as u64);
    // $next = entry_pc  ($retired defaults to 0)
    expr.push(op::I32_CONST);
    enc::sleb(&mut expr, pc as i64);
    expr.push(op::LOCAL_SET);
    enc::uleb(&mut expr, NEXT_LOCAL as u64);

    // Dispatch loop.
    expr.push(op::LOOP);
    expr.push(op::T_EMPTY);
    for blk in &blocks {
        // if ($next == entry_pc) { ... }
        expr.push(op::LOCAL_GET);
        enc::uleb(&mut expr, NEXT_LOCAL as u64);
        expr.push(op::I32_CONST);
        enc::sleb(&mut expr, blk.entry_pc as i64);
        expr.push(op::I32_EQ);
        expr.push(op::IF);
        expr.push(op::T_EMPTY);

        // Budget: if ($retired + n_k > $budget) { /*exit*/ } else { run }
        expr.push(op::LOCAL_GET);
        enc::uleb(&mut expr, RETIRED_LOCAL as u64);
        expr.push(op::I32_CONST);
        enc::sleb(&mut expr, blk.instr_count as i64);
        expr.push(op::I32_ADD);
        expr.push(op::LOCAL_GET);
        enc::uleb(&mut expr, BUDGET_LOCAL as u64);
        expr.push(op::I32_GT_S);
        expr.push(op::IF);
        expr.push(op::T_EMPTY);
        // then: over budget — nothing (fall through to exit; $next == entry_pc).
        expr.push(op::ELSE);

        // ── block body on locals ──
        t.buf.clear();
        t.trace_retired = Some((RETIRED_LOCAL, t.emitted));
        for op in &blk.ops {
            t.emit_instruction(op.pc, &op.inst);
        }
        match &blk.terminator {
            Some((tinst, tlen)) => t.emit_terminator(blk.prefix_end, *tlen as u32, tinst),
            // Fall-through block: continue at end_pc (an unmodeled cut / view
            // end) — dispatch will find no match and exit to the host there.
            None => t.next_pc_const(blk.prefix_end as i32),
        }
        expr.extend_from_slice(&t.buf);

        // $retired += n_k
        expr.push(op::LOCAL_GET);
        enc::uleb(&mut expr, RETIRED_LOCAL as u64);
        expr.push(op::I32_CONST);
        enc::sleb(&mut expr, blk.instr_count as i64);
        expr.push(op::I32_ADD);
        expr.push(op::LOCAL_SET);
        enc::uleb(&mut expr, RETIRED_LOCAL as u64);

        // continue the dispatch loop: br to $L (loop=depth 2: else(0) if(1) loop(2))
        expr.push(op::BR);
        enc::uleb(&mut expr, 2);

        expr.push(op::END); // budget if
        expr.push(op::END); // dispatch if
    }
    expr.push(op::END); // loop

    // ── Exit tail: flush union-writes, publish $next, return CHAIN_DYNAMIC ──
    for r in 1..32u8 {
        if u_writes[r as usize] {
            expr.push(op::I32_CONST);
            enc::sleb(&mut expr, (r as i64) * 4);
            expr.push(op::LOCAL_GET);
            enc::uleb(&mut expr, r as u64);
            expr.push(op::I32_STORE);
            enc::uleb(&mut expr, 2);
            enc::uleb(&mut expr, 0);
        }
    }
    expr.push(op::I32_CONST);
    enc::sleb(&mut expr, NEXT_PC_SLOT as i64);
    expr.push(op::LOCAL_GET);
    enc::uleb(&mut expr, NEXT_LOCAL as u64);
    expr.push(op::I32_STORE);
    enc::uleb(&mut expr, 2);
    enc::uleb(&mut expr, 0);
    // Publish the actual retired count for the runtime's mtime/retired fixup.
    expr.push(op::I32_CONST);
    enc::sleb(&mut expr, TRACE_RETIRED_SLOT as i64);
    expr.push(op::LOCAL_GET);
    enc::uleb(&mut expr, RETIRED_LOCAL as u64);
    expr.push(op::I32_STORE);
    enc::uleb(&mut expr, 2);
    enc::uleb(&mut expr, 0);
    expr.push(op::I32_CONST);
    enc::sleb(&mut expr, WIRE_CHAIN_DYNAMIC as i64);

    let binding = if has_mem {
        let (_base, len) = window.expect("mem op emitted without a RAM window");
        Some(MemBinding {
            ram_len: len as usize,
            has_store,
        })
    } else {
        None
    };
    let mem_pages = match &binding {
        Some(b) => (RAM_WINDOW_OFF as usize + b.ram_len)
            .max(1)
            .div_ceil(65536)
            .max(1) as u32,
        None => 1,
    };
    let code_bytes = build_module(TRACE_LOCALS, mem_pages, &expr);

    let mut exits = vec![ExitEdge {
        wire_code: WIRE_CHAIN_DYNAMIC,
        reason: BailReason::PartialBlock,
    }];
    if has_mem {
        exits.push(ExitEdge {
            wire_code: WIRE_MEM_FAULT,
            reason: BailReason::MemoryFault,
        });
    }

    Some(EmittedBlock {
        code: code_bytes,
        end_pc: entry_end_pc,
        instr_count: entry_instr_count,
        exits,
        binding,
    })
}

/// Decode the instruction at `pc` in `code`, returning it with its byte
/// length. `None` if `pc` is outside the view or a 4-byte instruction runs
/// past its end.
fn decode_at(pc: Pc, code: &CodeView<'_>) -> Option<(Instruction, u64)> {
    let bytes = code.from(pc)?;
    if bytes.len() < 2 {
        return None;
    }
    let low = u16::from_le_bytes([bytes[0], bytes[1]]);
    let len = inst_len(low);
    if len == 4 && bytes.len() < 4 {
        return None;
    }
    let word = if len == 4 {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    } else {
        low as u32
    };
    Some((decode_rv32(word), len))
}

/// Walk the maximal run of emittable body instructions (ALU + in-window
/// load/store) from `pc`.
fn walk_ops(pc: Pc, code: &CodeView<'_>, mem_ok: bool) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut cur = pc;
    while let Some(bytes) = code.from(cur) {
        if bytes.len() < 2 {
            break;
        }
        let low = u16::from_le_bytes([bytes[0], bytes[1]]);
        let len = inst_len(low);
        if len == 4 && bytes.len() < 4 {
            break;
        }
        let word = if len == 4 {
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            low as u32
        };
        let inst = decode_rv32(word);
        if !is_emittable(&inst, mem_ok) {
            break;
        }
        ops.push(Op {
            pc: cur as u32,
            inst,
        });
        cur += len;
        if ops.len() as u32 >= super::MAX_BLOCK_INSTRS {
            break;
        }
    }
    ops
}

/// Byte length of the instruction at `pc` in `code` (2 or 4).
fn inst_len_of(pc: u32, code: &CodeView<'_>) -> u64 {
    let bytes = code.from(pc as Pc).expect("op pc was in view during walk");
    inst_len(u16::from_le_bytes([bytes[0], bytes[1]]))
}

/// A range-checked memory access the emit core lowers to an inline
/// `iN.load`/`iN.store` fast path plus an out-of-window fault exit.
enum MemAccess {
    /// A load into `rd` using the given wasm load opcode (which already
    /// carries the width + sign/zero extension).
    Load { rd: u8, opcode: u8 },
    /// A store of `rs2` using the given wasm store opcode (width).
    Store { rs2: u8, opcode: u8 },
}

impl MemAccess {
    /// Access width in bytes (1, 2, or 4).
    fn width(&self) -> u32 {
        match self {
            MemAccess::Load { opcode, .. } => match *opcode {
                op::I32_LOAD8_S | op::I32_LOAD8_U => 1,
                op::I32_LOAD16_S | op::I32_LOAD16_U => 2,
                _ => 4,
            },
            MemAccess::Store { opcode, .. } => match *opcode {
                op::I32_STORE8 => 1,
                op::I32_STORE16 => 2,
                _ => 4,
            },
        }
    }
}

/// Accumulates the body opcodes plus the read/write register sets and the
/// chunk-E memory-binding facts.
#[derive(Default)]
struct Body {
    buf: Vec<u8>,
    /// Registers read anywhere in the block (loaded in the prologue).
    reads: [bool; 32],
    /// Registers written anywhere in the block (stored in the epilogue).
    writes: [bool; 32],
    /// Bound guest-RAM window (`None` ⇒ no load/store is emittable).
    window: Option<RamWindow>,
    /// Whether the block emitted any load/store (needs the scratch local,
    /// RAM sync, and the fault exit).
    has_mem: bool,
    /// Whether the block emitted any store (drives the reservation clear).
    has_store: bool,
    /// Whether the scratch local was declared.
    scratch_used: bool,
    /// Count of instructions fully emitted so far — the retired-so-far value
    /// a mid-block fault publishes.
    emitted: u32,
    // ── trace-mode wiring (all `None`/`0` for a single-block emit) ────────
    /// When `Some(idx)`, terminators write their resolved next PC into wasm
    /// local `idx` (`$next`) instead of the [`NEXT_PC_SLOT`] memory slot, so
    /// the enclosing dispatch loop can `br_table`/if-chain on it. See
    /// [`emit_trace`].
    trace_next_local: Option<u32>,
    /// When `Some((idx, block_start))`, a mid-block memory fault publishes
    /// `FAULT_RETIRED = local[idx] + (self.emitted - block_start)` — the
    /// trace's running retired count (fully-retired prior blocks, in the local)
    /// plus the count of this block's instructions retired before the fault —
    /// rather than the single-block `self.emitted`.
    trace_retired: Option<(u32, u32)>,
    /// When `Some(set)`, a fault flushes the whole trace-union write set (every
    /// write-local is prologue-loaded, so re-storing an unmodified one is a
    /// no-op) instead of only the intra-block `writes_before` snapshot.
    trace_union_writes: Option<[bool; 32]>,
}

impl Body {
    /// Push `local.get`/`i32.const 0` for reading guest register `r`.
    fn read(&mut self, r: u8) {
        if r == 0 {
            self.buf.push(op::I32_CONST);
            enc::sleb(&mut self.buf, 0);
        } else {
            self.reads[r as usize] = true;
            self.buf.push(op::LOCAL_GET);
            enc::uleb(&mut self.buf, r as u64);
        }
    }

    /// Consume the stack top as the new value of guest register `r`
    /// (`local.set`, or `drop` for `x0`).
    fn write(&mut self, r: u8) {
        if r == 0 {
            self.buf.push(op::DROP);
        } else {
            self.writes[r as usize] = true;
            self.buf.push(op::LOCAL_SET);
            enc::uleb(&mut self.buf, r as u64);
        }
    }

    /// Push an `i32.const`.
    fn i32_const(&mut self, v: i32) {
        self.buf.push(op::I32_CONST);
        enc::sleb(&mut self.buf, v as i64);
    }

    /// Push `local.get $scratch`.
    fn scratch_get(&mut self) {
        self.buf.push(op::LOCAL_GET);
        enc::uleb(&mut self.buf, SCRATCH_LOCAL as u64);
    }

    /// Emit `read(rs1) read(rs2) <opcode> write(rd)`.
    fn binop(&mut self, rd: u8, rs1: u8, rs2: u8, opcode: u8) {
        self.read(rs1);
        self.read(rs2);
        self.buf.push(opcode);
        self.write(rd);
    }

    /// Emit `read(rs1) i32.const(imm) <opcode> write(rd)`.
    fn binop_imm(&mut self, rd: u8, rs1: u8, imm: i32, opcode: u8) {
        self.read(rs1);
        self.i32_const(imm);
        self.buf.push(opcode);
        self.write(rd);
    }

    /// Emit the high-half multiply family.
    fn mulh(&mut self, rd: u8, rs1: u8, rs2: u8, ext1: u8, ext2: u8) {
        self.read(rs1);
        self.buf.push(ext1);
        self.read(rs2);
        self.buf.push(ext2);
        self.buf.push(op::I64_MUL);
        self.buf.push(op::I64_CONST);
        enc::sleb(&mut self.buf, 32);
        self.buf.push(op::I64_SHR_U);
        self.buf.push(op::I32_WRAP_I64);
        self.write(rd);
    }

    /// Push `read(r) i32.eqz`.
    fn is_zero(&mut self, r: u8) {
        self.read(r);
        self.buf.push(op::I32_EQZ);
    }

    /// Push the signed-division overflow predicate.
    fn is_signed_overflow(&mut self, rs1: u8, rs2: u8) {
        self.read(rs1);
        self.i32_const(i32::MIN);
        self.buf.push(op::I32_EQ);
        self.read(rs2);
        self.i32_const(-1);
        self.buf.push(op::I32_EQ);
        self.buf.push(op::I32_AND);
    }

    /// Open an `if (result i32)` on the condition already on the stack.
    fn if_i32(&mut self) {
        self.buf.push(op::IF);
        self.buf.push(op::T_I32);
    }

    /// Emit `i32.store` of a constant value at a constant byte address (used
    /// for the fault/flag control slots; 4-byte aligned).
    fn store_const_at(&mut self, addr: u32, val: i32) {
        self.i32_const(addr as i32);
        self.i32_const(val);
        self.buf.push(op::I32_STORE);
        enc::uleb(&mut self.buf, 2); // align 2^2 = 4
        enc::uleb(&mut self.buf, 0); // offset
    }

    /// Emit a range-checked load/store. `pc` is the faulting instruction's own
    /// PC (published on the out-of-window fault path). `addr_reg`/`imm` form
    /// the effective address `rs1 + imm`.
    fn emit_mem(&mut self, pc: u32, addr_reg: u8, imm: i32, access: MemAccess) {
        let (ram_base, ram_len) = self
            .window
            .expect("emit_mem called without a bound RAM window");
        let ram_end = ram_base.wrapping_add(ram_len);
        let width = access.width();
        // Highest effective address the access can start at and still fit
        // wholly inside the window — mirrors the interpreter's LinearMemory
        // bound `addr + (width-1) < base + len`.
        let hi = ram_end.wrapping_sub(width);
        // wasm byte = ea - ram_base + RAM_WINDOW_OFF = ea + delta.
        let delta = RAM_WINDOW_OFF.wrapping_sub(ram_base);

        self.has_mem = true;
        self.scratch_used = true;

        // ea = read(addr_reg) + imm  →  $scratch
        self.read(addr_reg);
        self.i32_const(imm);
        self.buf.push(op::I32_ADD);
        self.buf.push(op::LOCAL_SET);
        enc::uleb(&mut self.buf, SCRATCH_LOCAL as u64);

        // in_window = (ea >=u ram_base) & (ea <=u hi)
        self.scratch_get();
        self.i32_const(ram_base as i32);
        self.buf.push(op::I32_GE_U);
        self.scratch_get();
        self.i32_const(hi as i32);
        self.buf.push(op::I32_LE_U);
        self.buf.push(op::I32_AND);

        // if in_window { fast path } else { fault } — empty block type: the
        // then-arm leaves nothing on the stack, the else-arm ends unreachable.
        self.buf.push(op::IF);
        self.buf.push(op::T_EMPTY);

        // Snapshot the write set BEFORE the then-arm so a load's own `rd`
        // (written only on the taken fast path) is excluded from *this* op's
        // fault writeback.
        let writes_before = self.writes;

        match access {
            MemAccess::Load { rd, opcode } => {
                self.scratch_get();
                self.i32_const(delta as i32);
                self.buf.push(op::I32_ADD);
                self.buf.push(opcode);
                enc::uleb(&mut self.buf, 0); // align 0 (byte; unaligned-safe)
                enc::uleb(&mut self.buf, 0); // offset
                self.write(rd);
            }
            MemAccess::Store { rs2, opcode } => {
                self.scratch_get();
                self.i32_const(delta as i32);
                self.buf.push(op::I32_ADD); // address
                self.read(rs2); // value
                self.buf.push(opcode);
                enc::uleb(&mut self.buf, 0); // align 0
                enc::uleb(&mut self.buf, 0); // offset
                                             // Record that a store executed → runtime clears reservation.
                self.store_const_at(RES_FLAG_SLOT, 1);
                self.has_store = true;
            }
        }

        self.buf.push(op::ELSE);
        self.emit_fault(pc, &writes_before);
        self.buf.push(op::END);
    }

    /// Emit the out-of-window fault tail: flush the registers written by
    /// instructions before this one, publish the resume PC + retired count,
    /// and return [`WIRE_MEM_FAULT`].
    fn emit_fault(&mut self, pc: u32, writes_before: &[bool; 32]) {
        // Trace mode flushes the whole trace-union write set (every write-local
        // is prologue-loaded, so re-storing an unmodified one is a no-op),
        // because registers written by *earlier fused blocks* live only in
        // locals and must reach memory on any exit. Single-block mode flushes
        // just the intra-block `writes_before` snapshot (the faulting op and
        // everything after it are left to the interpreter).
        let flush = self.trace_union_writes.unwrap_or(*writes_before);
        for r in 1..32u8 {
            if flush[r as usize] {
                self.i32_const((r as i32) * 4);
                self.buf.push(op::LOCAL_GET);
                enc::uleb(&mut self.buf, r as u64);
                self.buf.push(op::I32_STORE);
                enc::uleb(&mut self.buf, 2);
                enc::uleb(&mut self.buf, 0);
            }
        }
        self.store_const_at(FAULT_PC_SLOT, pc as i32);
        match self.trace_retired {
            // Single block: retired-so-far is the compile-time emit count.
            None => self.store_const_at(FAULT_RETIRED_SLOT, self.emitted as i32),
            // Trace: FAULT_RETIRED = $retired (fully-retired prior blocks) plus
            // this block's compile-time offset to the faulting instruction.
            Some((retired_local, block_start)) => {
                let block_base = self.emitted - block_start;
                self.i32_const(FAULT_RETIRED_SLOT as i32);
                self.buf.push(op::LOCAL_GET);
                enc::uleb(&mut self.buf, retired_local as u64);
                self.i32_const(block_base as i32);
                self.buf.push(op::I32_ADD);
                self.buf.push(op::I32_STORE);
                enc::uleb(&mut self.buf, 2);
                enc::uleb(&mut self.buf, 0);
            }
        }
        self.i32_const(WIRE_MEM_FAULT);
        self.buf.push(op::RETURN);
    }

    /// Emit one guest instruction. `pc` is the instruction's own PC.
    fn emit_instruction(&mut self, pc: u32, inst: &Instruction) {
        use Instruction::*;
        match *inst {
            // ── upper immediates ───────────────────────────────────────
            Lui { rd, imm } => {
                self.i32_const(imm as i32);
                self.write(rd);
            }
            Auipc { rd, imm } => {
                self.i32_const(pc.wrapping_add(imm) as i32);
                self.write(rd);
            }

            // ── register-immediate arithmetic / logic ──────────────────
            Addi { rd, rs1, imm } => self.binop_imm(rd, rs1, imm, op::I32_ADD),
            Xori { rd, rs1, imm } => self.binop_imm(rd, rs1, imm, op::I32_XOR),
            Ori { rd, rs1, imm } => self.binop_imm(rd, rs1, imm, op::I32_OR),
            Andi { rd, rs1, imm } => self.binop_imm(rd, rs1, imm, op::I32_AND),
            Slti { rd, rs1, imm } => self.binop_imm(rd, rs1, imm, op::I32_LT_S),
            Sltiu { rd, rs1, imm } => self.binop_imm(rd, rs1, imm, op::I32_LT_U),

            // ── immediate shifts ───────────────────────────────────────
            Slli { rd, rs1, shamt } => self.binop_imm(rd, rs1, shamt as i32, op::I32_SHL),
            Srli { rd, rs1, shamt } => self.binop_imm(rd, rs1, shamt as i32, op::I32_SHR_U),
            Srai { rd, rs1, shamt } => self.binop_imm(rd, rs1, shamt as i32, op::I32_SHR_S),

            // ── register-register arithmetic / logic ───────────────────
            Add { rd, rs1, rs2 } => self.binop(rd, rs1, rs2, op::I32_ADD),
            Sub { rd, rs1, rs2 } => self.binop(rd, rs1, rs2, op::I32_SUB),
            Xor { rd, rs1, rs2 } => self.binop(rd, rs1, rs2, op::I32_XOR),
            Or { rd, rs1, rs2 } => self.binop(rd, rs1, rs2, op::I32_OR),
            And { rd, rs1, rs2 } => self.binop(rd, rs1, rs2, op::I32_AND),
            Slt { rd, rs1, rs2 } => self.binop(rd, rs1, rs2, op::I32_LT_S),
            Sltu { rd, rs1, rs2 } => self.binop(rd, rs1, rs2, op::I32_LT_U),
            Sll { rd, rs1, rs2 } => self.binop(rd, rs1, rs2, op::I32_SHL),
            Srl { rd, rs1, rs2 } => self.binop(rd, rs1, rs2, op::I32_SHR_U),
            Sra { rd, rs1, rs2 } => self.binop(rd, rs1, rs2, op::I32_SHR_S),

            // ── RV32M multiply ─────────────────────────────────────────
            Mul { rd, rs1, rs2 } => self.binop(rd, rs1, rs2, op::I32_MUL),
            Mulh { rd, rs1, rs2 } => {
                self.mulh(rd, rs1, rs2, op::I64_EXTEND_I32_S, op::I64_EXTEND_I32_S)
            }
            Mulhsu { rd, rs1, rs2 } => {
                self.mulh(rd, rs1, rs2, op::I64_EXTEND_I32_S, op::I64_EXTEND_I32_U)
            }
            Mulhu { rd, rs1, rs2 } => {
                self.mulh(rd, rs1, rs2, op::I64_EXTEND_I32_U, op::I64_EXTEND_I32_U)
            }

            // ── RV32M divide / remainder (trap-free, guarded) ──────────
            Div { rd, rs1, rs2 } => {
                self.is_zero(rs2);
                self.if_i32();
                self.i32_const(-1);
                self.buf.push(op::ELSE);
                self.is_signed_overflow(rs1, rs2);
                self.if_i32();
                self.read(rs1);
                self.buf.push(op::ELSE);
                self.read(rs1);
                self.read(rs2);
                self.buf.push(op::I32_DIV_S);
                self.buf.push(op::END);
                self.buf.push(op::END);
                self.write(rd);
            }
            Divu { rd, rs1, rs2 } => {
                self.is_zero(rs2);
                self.if_i32();
                self.i32_const(-1);
                self.buf.push(op::ELSE);
                self.read(rs1);
                self.read(rs2);
                self.buf.push(op::I32_DIV_U);
                self.buf.push(op::END);
                self.write(rd);
            }
            Rem { rd, rs1, rs2 } => {
                self.is_zero(rs2);
                self.if_i32();
                self.read(rs1);
                self.buf.push(op::ELSE);
                self.is_signed_overflow(rs1, rs2);
                self.if_i32();
                self.i32_const(0);
                self.buf.push(op::ELSE);
                self.read(rs1);
                self.read(rs2);
                self.buf.push(op::I32_REM_S);
                self.buf.push(op::END);
                self.buf.push(op::END);
                self.write(rd);
            }
            Remu { rd, rs1, rs2 } => {
                self.is_zero(rs2);
                self.if_i32();
                self.read(rs1);
                self.buf.push(op::ELSE);
                self.read(rs1);
                self.read(rs2);
                self.buf.push(op::I32_REM_U);
                self.buf.push(op::END);
                self.write(rd);
            }

            // ── loads (sign/zero extension baked into the wasm opcode) ──
            Lb { rd, rs1, imm } => self.emit_mem(
                pc,
                rs1,
                imm,
                MemAccess::Load {
                    rd,
                    opcode: op::I32_LOAD8_S,
                },
            ),
            Lbu { rd, rs1, imm } => self.emit_mem(
                pc,
                rs1,
                imm,
                MemAccess::Load {
                    rd,
                    opcode: op::I32_LOAD8_U,
                },
            ),
            Lh { rd, rs1, imm } => self.emit_mem(
                pc,
                rs1,
                imm,
                MemAccess::Load {
                    rd,
                    opcode: op::I32_LOAD16_S,
                },
            ),
            Lhu { rd, rs1, imm } => self.emit_mem(
                pc,
                rs1,
                imm,
                MemAccess::Load {
                    rd,
                    opcode: op::I32_LOAD16_U,
                },
            ),
            Lw { rd, rs1, imm } => self.emit_mem(
                pc,
                rs1,
                imm,
                MemAccess::Load {
                    rd,
                    opcode: op::I32_LOAD,
                },
            ),

            // ── stores ─────────────────────────────────────────────────
            Sb { rs1, rs2, imm } => self.emit_mem(
                pc,
                rs1,
                imm,
                MemAccess::Store {
                    rs2,
                    opcode: op::I32_STORE8,
                },
            ),
            Sh { rs1, rs2, imm } => self.emit_mem(
                pc,
                rs1,
                imm,
                MemAccess::Store {
                    rs2,
                    opcode: op::I32_STORE16,
                },
            ),
            Sw { rs1, rs2, imm } => self.emit_mem(
                pc,
                rs1,
                imm,
                MemAccess::Store {
                    rs2,
                    opcode: op::I32_STORE,
                },
            ),

            // ── compressed loads/stores (imm zero-extended) ────────────
            CLw { rd, rs1, imm } => self.emit_mem(
                pc,
                rs1,
                imm as i32,
                MemAccess::Load {
                    rd,
                    opcode: op::I32_LOAD,
                },
            ),
            CSw { rs2, rs1, imm } => self.emit_mem(
                pc,
                rs1,
                imm as i32,
                MemAccess::Store {
                    rs2,
                    opcode: op::I32_STORE,
                },
            ),
            // C.LWSP / C.SWSP address off x2 (sp).
            CLwsp { rd, imm } => self.emit_mem(
                pc,
                2,
                imm as i32,
                MemAccess::Load {
                    rd,
                    opcode: op::I32_LOAD,
                },
            ),
            CSwsp { rs2, imm } => self.emit_mem(
                pc,
                2,
                imm as i32,
                MemAccess::Store {
                    rs2,
                    opcode: op::I32_STORE,
                },
            ),

            // ── compressed ALU forms ───────────────────────────────────
            CAddi { rd, imm } => self.binop_imm(rd, rd, imm, op::I32_ADD),
            CLi { rd, imm } => {
                self.i32_const(imm);
                self.write(rd);
            }
            CMv { rd, rs2 } => {
                self.read(rs2);
                self.write(rd);
            }
            CAddi16sp { imm } => self.binop_imm(2, 2, imm, op::I32_ADD),
            CAddi4spn { rd, imm } => self.binop_imm(rd, 2, imm as i32, op::I32_ADD),
            CSli { rd, shamt } => self.binop_imm(rd, rd, shamt as i32, op::I32_SHL),

            // Anything else must not reach here — the walk stops before it.
            other => unreachable!("non-emittable instruction reached emit: {other:?}"),
        }
        self.emitted += 1;
    }

    /// Begin committing a resolved next PC. In single-block mode this pushes
    /// the [`NEXT_PC_SLOT`] store address (so the value computed next sits above
    /// it for the closing `i32.store`); in trace mode it pushes nothing (the
    /// value is captured by a closing `local.set $next`). See [`end_next_pc`].
    fn begin_next_pc(&mut self) {
        if self.trace_next_local.is_none() {
            self.i32_const(NEXT_PC_SLOT);
        }
    }

    /// Close a next-PC commit opened by [`begin_next_pc`], consuming the value
    /// on the stack: `i32.store` to the slot (single-block) or `local.set $next`
    /// (trace). Byte-identical to the previous `store_next_pc` in slot mode.
    fn end_next_pc(&mut self) {
        match self.trace_next_local {
            None => {
                self.buf.push(op::I32_STORE);
                enc::uleb(&mut self.buf, 2); // align = 2 (4-byte)
                enc::uleb(&mut self.buf, 0); // offset = 0 (address == slot)
            }
            Some(idx) => {
                self.buf.push(op::LOCAL_SET);
                enc::uleb(&mut self.buf, idx as u64);
            }
        }
    }

    /// Write a **constant** next PC (`Jal`, `C.J`, and the two arms of a
    /// conditional branch all resolve to compile-time addresses).
    fn next_pc_const(&mut self, v: i32) {
        self.begin_next_pc();
        self.i32_const(v);
        self.end_next_pc();
    }

    /// Emit a two-register conditional branch: `next = cmp(rs1,rs2) ? pc+imm
    /// : pc+ilen`, stored to the slot. `cmp` is the wasm predicate opcode
    /// (`I32_EQ`, `I32_LT_S`, …) mirroring the interpreter's comparison.
    fn cond_branch(&mut self, rs1: u8, rs2: u8, cmp: u8, pc: u32, ilen: u32, imm: i32) {
        self.begin_next_pc(); // store address (stays below the `if`) in slot mode
        self.read(rs1);
        self.read(rs2);
        self.buf.push(cmp);
        self.if_i32();
        self.i32_const(pc.wrapping_add(imm as u32) as i32); // taken
        self.buf.push(op::ELSE);
        self.i32_const(pc.wrapping_add(ilen) as i32); // not taken
        self.buf.push(op::END);
        self.end_next_pc();
    }

    /// Emit a compressed compare-with-zero branch (`C.BEQZ`/`C.BNEZ`):
    /// `next = (rs1 == 0)==want_zero ? pc+imm : pc+ilen`.
    fn cond_branch_zero(&mut self, rs1: u8, want_zero: bool, pc: u32, ilen: u32, imm: i32) {
        self.begin_next_pc();
        self.read(rs1);
        // `C.BEQZ` takes when rs1 == 0 → test with `i32.eqz`; `C.BNEZ` takes
        // when rs1 != 0 → the raw value is already truthy for `if`.
        if want_zero {
            self.buf.push(op::I32_EQZ);
        }
        self.if_i32();
        self.i32_const(pc.wrapping_add(imm as u32) as i32); // taken
        self.buf.push(op::ELSE);
        self.i32_const(pc.wrapping_add(ilen) as i32); // not taken
        self.buf.push(op::END);
        self.end_next_pc();
    }

    /// Emit `rs1 & !1` (jump-target low-bit mask) onto the stack.
    fn read_masked(&mut self, rs1: u8) {
        self.read(rs1);
        self.i32_const(!1); // 0xFFFF_FFFE
        self.buf.push(op::I32_AND);
    }

    /// Emit the block's control-flow terminator. `pc` is its own guest PC,
    /// `ilen` its byte length. Mirrors the branch/jump arms of
    /// [`crate::cpu::riscv::RiscV::step`] exactly — including link-register
    /// timing (the next PC is resolved *before* the link write, so `rd == rs1`
    /// `JALR` reads the old `rs1`) and the `& !1` target mask.
    fn emit_terminator(&mut self, pc: u32, ilen: u32, inst: &Instruction) {
        use Instruction::*;
        match *inst {
            Beq { rs1, rs2, imm } => self.cond_branch(rs1, rs2, op::I32_EQ, pc, ilen, imm),
            Bne { rs1, rs2, imm } => self.cond_branch(rs1, rs2, op::I32_NE, pc, ilen, imm),
            Blt { rs1, rs2, imm } => self.cond_branch(rs1, rs2, op::I32_LT_S, pc, ilen, imm),
            Bge { rs1, rs2, imm } => self.cond_branch(rs1, rs2, op::I32_GE_S, pc, ilen, imm),
            Bltu { rs1, rs2, imm } => self.cond_branch(rs1, rs2, op::I32_LT_U, pc, ilen, imm),
            Bgeu { rs1, rs2, imm } => self.cond_branch(rs1, rs2, op::I32_GE_U, pc, ilen, imm),
            CBeqz { rs1, imm } => self.cond_branch_zero(rs1, true, pc, ilen, imm),
            CBnez { rs1, imm } => self.cond_branch_zero(rs1, false, pc, ilen, imm),

            // Unconditional jump: next = pc + imm; link rd = pc + ilen (the
            // decoder maps 2-byte C.JAL to Jal { rd: 1 }, so ilen links it at
            // pc+2 while a 4-byte JAL links at pc+4).
            Jal { rd, imm } => {
                self.next_pc_const(pc.wrapping_add(imm as u32) as i32);
                self.i32_const(pc.wrapping_add(ilen) as i32);
                self.write(rd);
            }
            // Indirect jump: next = (rs1 + imm) & !1, computed BEFORE the link
            // write so `jalr rd, rd, imm` reads the pre-write rs1.
            Jalr { rd, rs1, imm } => {
                self.begin_next_pc();
                self.read(rs1);
                self.i32_const(imm);
                self.buf.push(op::I32_ADD);
                self.i32_const(!1);
                self.buf.push(op::I32_AND);
                self.end_next_pc();
                self.i32_const(pc.wrapping_add(ilen) as i32);
                self.write(rd);
            }

            CJ { imm } => self.next_pc_const(pc.wrapping_add(imm as u32) as i32),
            // C.JR: next = rs1 & !1 (no link).
            CJr { rs1 } => {
                self.begin_next_pc();
                self.read_masked(rs1);
                self.end_next_pc();
            }
            // C.JALR: next = rs1 & !1; link x1 = pc + 2 (always 2-byte).
            CJalr { rs1 } => {
                self.begin_next_pc();
                self.read_masked(rs1);
                self.end_next_pc();
                self.i32_const(pc.wrapping_add(2) as i32);
                self.write(1);
            }

            other => unreachable!("non-terminator reached emit_terminator: {other:?}"),
        }
    }

    /// Emit the prologue: `local.set r (i32.load (r*4))` for each read reg.
    fn emit_prologue(&self, out: &mut Vec<u8>) {
        for r in 1..32u8 {
            if self.reads[r as usize] {
                out.push(op::I32_CONST);
                enc::sleb(out, (r as i64) * 4);
                out.push(op::I32_LOAD);
                enc::uleb(out, 2); // align = 2 (2^2 = 4-byte)
                enc::uleb(out, 0); // offset = 0
                out.push(op::LOCAL_SET);
                enc::uleb(out, r as u64);
            }
        }
    }

    /// Emit the epilogue: `i32.store (r*4) (local.get r)` for each write reg.
    fn emit_epilogue(&self, out: &mut Vec<u8>) {
        for r in 1..32u8 {
            if self.writes[r as usize] {
                out.push(op::I32_CONST);
                enc::sleb(out, (r as i64) * 4);
                out.push(op::LOCAL_GET);
                enc::uleb(out, r as u64);
                out.push(op::I32_STORE);
                enc::uleb(out, 2);
                enc::uleb(out, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: Pc = 0x4200_0000;
    const RAM_BASE: u32 = 0x8000_0000;
    const RAM_LEN: u32 = 0x1_0000;

    fn enc_jal(rd: u32, imm: i32) -> u32 {
        let u = imm as u32;
        ((u >> 20 & 1) << 31)
            | ((u >> 1 & 0x3ff) << 21)
            | ((u >> 11 & 1) << 20)
            | ((u >> 12 & 0xff) << 12)
            | (rd << 7)
            | 0x6f
    }
    fn enc_jalr(rd: u32, rs1: u32, imm: i32) -> u32 {
        ((imm as u32 & 0xfff) << 20) | (rs1 << 15) | (rd << 7) | 0x67
    }

    /// M2 return-site splicing: `call_return_site` links only for `JAL ra/t0`.
    #[test]
    fn call_return_site_only_for_linking_jal() {
        // jal ra, +12  → return site = pc + 4
        let ra = decode_rv32(enc_jal(1, 12));
        assert_eq!(call_return_site(&ra, BASE as u32, 4), Some(BASE + 4));
        // jal t0, +12  → also a call (x5 link)
        let t0 = decode_rv32(enc_jal(5, 12));
        assert_eq!(call_return_site(&t0, BASE as u32, 4), Some(BASE + 4));
        // jal x0, +12  → plain jump, no link → not a call
        let j = decode_rv32(enc_jal(0, 12));
        assert_eq!(call_return_site(&j, BASE as u32, 4), None);
        // jalr is dynamic-target — excluded from static splicing
        let jr = decode_rv32(enc_jalr(1, 6, 0));
        assert_eq!(call_return_site(&jr, BASE as u32, 4), None);
    }

    /// M2 mechanism: a hot loop that `JAL ra`-calls a fused helper gets the
    /// helper's RETURN SITE spliced into the same trace, so the helper's `ret`
    /// re-dispatches in-wasm. Without the splice the trace would stop at 2
    /// blocks (entry + helper) and the ret would exit to the host.
    #[test]
    fn collect_trace_splices_direct_call_return_site() {
        // BASE+0 : addi t0,t0,1          (loop-header body)
        // BASE+4 : jal  ra, +12          (call helper at BASE+16; ret site BASE+8)
        // BASE+8 : addi t1,t1,1          (continuation after the call)
        // BASE+12: jal  x0, -12          (back-edge to BASE+0 → entry is static)
        // BASE+16: addi t2,t2,1          (helper body)
        // BASE+20: jalr x0, ra, 0        (ret)
        let prog = view(&[
            enc_addi(5, 5, 1),
            enc_jal(1, 12),
            enc_addi(6, 6, 1),
            enc_jal(0, -12),
            enc_addi(7, 7, 1),
            enc_jalr(0, 1, 0),
        ]);
        let cv = CodeView::new(BASE, &prog);
        let entries: Vec<u32> = collect_trace(BASE, &cv, false)
            .iter()
            .map(|b| b.entry_pc)
            .collect();
        assert!(
            entries.contains(&(BASE as u32 + 8)),
            "return site BASE+8 must be fused; got blocks {entries:x?}"
        );
        assert!(
            entries.contains(&(BASE as u32 + 16)),
            "helper BASE+16 must be fused; got blocks {entries:x?}"
        );
        // A full trace emits and validates in wasmtime (see cfg(jit) test below).
        assert!(emit_trace(BASE, &cv, None).is_some(), "trace must form");
    }

    fn enc_addi(rd: u32, rs1: u32, imm: i32) -> u32 {
        ((imm as u32 & 0xFFF) << 20) | (rs1 << 15) | (rd << 7) | 0x13
    }
    fn enc_add(rd: u32, rs1: u32, rs2: u32) -> u32 {
        (rs2 << 20) | (rs1 << 15) | (rd << 7) | 0x33
    }
    fn enc_lw(rd: u32, rs1: u32, imm: i32) -> u32 {
        ((imm as u32 & 0xFFF) << 20) | (rs1 << 15) | (0b010 << 12) | (rd << 7) | 0x03
    }
    fn enc_sw(rs1: u32, rs2: u32, imm: i32) -> u32 {
        let u = imm as u32 & 0xFFF;
        ((u >> 5) << 25) | (rs2 << 20) | (rs1 << 15) | (0b010 << 12) | ((u & 0x1F) << 7) | 0x23
    }
    fn enc_ecall() -> u32 {
        0x0000_0073
    }

    fn view(words: &[u32]) -> Vec<u8> {
        let mut b = Vec::new();
        for w in words {
            b.extend_from_slice(&w.to_le_bytes());
        }
        b
    }

    #[test]
    fn refuses_non_alu_entry() {
        let prog = view(&[enc_ecall()]);
        let cv = CodeView::new(BASE, &prog);
        assert!(emit_block(BASE, &cv, None).is_none());
    }

    #[test]
    fn walks_alu_prefix_and_stops_before_non_alu() {
        let prog = view(&[enc_addi(1, 0, 1), enc_add(2, 1, 1), enc_ecall()]);
        let cv = CodeView::new(BASE, &prog);
        let blk = emit_block(BASE, &cv, None).unwrap();
        assert_eq!(blk.instr_count, 2, "ecall is not included");
        assert_eq!(blk.end_pc, BASE + 8, "ends before the ecall");
        assert_eq!(blk.exits.len(), 1);
        assert_eq!(blk.exits[0].wire_code, WIRE_FALL_THROUGH);
        assert!(blk.binding.is_none(), "pure-ALU block has no RAM binding");
        assert_eq!(&blk.code[0..4], &[0x00, 0x61, 0x73, 0x6d]);
    }

    #[test]
    fn windowless_stops_before_load() {
        // Without a RAM window, a load is not emittable → block ends before it.
        let prog = view(&[enc_addi(1, 0, 1), enc_lw(2, 1, 0), enc_ecall()]);
        let cv = CodeView::new(BASE, &prog);
        let blk = emit_block(BASE, &cv, None).unwrap();
        assert_eq!(blk.instr_count, 1, "only the addi; load excluded");
        assert!(blk.binding.is_none());
    }

    #[test]
    fn window_makes_loads_stores_emittable() {
        // addi ; lw ; sw ; ecall — with a window, addi+lw+sw are one block.
        let prog = view(&[
            enc_addi(1, 0, 0),
            enc_lw(2, 1, 0),
            enc_sw(1, 2, 4),
            enc_ecall(),
        ]);
        let cv = CodeView::new(BASE, &prog);
        let blk = emit_block(BASE, &cv, Some((RAM_BASE, RAM_LEN))).unwrap();
        assert_eq!(blk.instr_count, 3, "addi + lw + sw");
        assert_eq!(blk.end_pc, BASE + 12);
        let binding = blk.binding.expect("load/store block has a RAM binding");
        assert_eq!(binding.ram_len, RAM_LEN as usize);
        assert!(binding.has_store);
        // Two exits: fall-through + memory fault.
        assert_eq!(blk.exits.len(), 2);
        assert!(blk
            .exits
            .iter()
            .any(|e| e.wire_code == WIRE_MEM_FAULT && e.reason == BailReason::MemoryFault));
    }

    #[cfg(feature = "jit")]
    #[test]
    fn emitted_mem_module_validates_in_wasmtime() {
        let prog = view(&[
            enc_addi(1, 0, 0),
            enc_lw(2, 1, 0),
            enc_sw(1, 2, 8),
            enc_ecall(),
        ]);
        let cv = CodeView::new(BASE, &prog);
        let blk = emit_block(BASE, &cv, Some((RAM_BASE, RAM_LEN))).unwrap();
        let engine = wasmtime::Engine::default();
        wasmtime::Module::new(&engine, &blk.code).expect("emitted mem module must validate");
    }
}
