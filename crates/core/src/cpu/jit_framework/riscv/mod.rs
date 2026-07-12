// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RV32IMC frontend for the universal dispatch JIT.
//!
//! This is the **first real** [`IsaFrontend`] (the scaffold shipped only the
//! [`super::frontend::PassthroughFrontend`]). The *foundation* milestone
//! implemented here is deliberately an **all-bail** frontend: it walks a
//! basic block over the flash [`CodeView`] using the shared RISC-V decoder
//! ([`crate::decoder::riscv`]), classifies every instruction, and produces a
//! correct [`BlockPlan`] — `entry_pc`, `end_pc`, `instr_count`, and the
//! side-exit `exits` map — but with an **empty `code`** body
//! ([`BlockPlan::is_stub`] is `true`). Every block therefore side-exits to
//! the interpreter, exactly like the passthrough stub.
//!
//! Why build a real walker that emits no code? Because the walk +
//! classification is the ISA-aware skeleton that the later codegen chunks
//! (integer arithmetic, branches/jumps, loads/stores, CSR/system) hang wasm
//! emission onto. Landing it all-bail lets the differential harness
//! ([`super::differential`]) prove the *entire* dispatch / host / snapshot
//! plumbing is byte-identical to the interpreter before a single wasm byte
//! is emitted — the framework's merge gate #1. Each subsequent chunk fills
//! `code` for a class of instructions and re-runs that same gate.
//!
//! ## The decode reuse
//!
//! [`crate::decoder::riscv::decode_rv32`] already decodes both 32-bit and
//! (via its `(inst & 0x3) != 0x3` prefix check) 16-bit compressed
//! instructions into one flat, `Copy` [`Instruction`] enum with
//! pre-decoded, sign-extended immediates. The frontend does **not**
//! re-implement any of that: it reads the instruction-length halfword
//! (`(halfword & 0x3) == 0x3 ? 4 : 2`), assembles the little-endian word,
//! calls `decode_rv32`, and classifies the result.

use crate::decoder::riscv::{decode_rv32, Instruction};

use super::frontend::{BlockPlan, ExitEdge, FrontendRefusal, IsaFrontend};
use super::side_exit::BailReason;
use super::{CodeView, Pc};

pub mod emit;
pub mod host;
pub mod wasm_encode;

pub use host::{snapshot_state, RiscVJitHost};

// Native (`wasmtime`) execution of emitted blocks lives behind the `jit`
// feature so the browser build (which cannot pull in wasmtime) still gets
// the pure-Rust walker + emitter above.
#[cfg(feature = "jit")]
pub mod exec;
#[cfg(feature = "jit")]
pub use exec::{CompiledBlock, EngineStats, RiscvJitEngine, RiscvWasmJit};

/// Indices into the RISC-V [`StateVec`](super::StateVec) that a batched JIT
/// run may legitimately compute differently from a per-instruction
/// interpreter run and which the differential harness should mask.
///
/// In this all-bail foundation milestone the JIT executes **zero**
/// instructions itself — every block side-exits and the interpreter runs
/// each instruction — so nothing is volatile and this is empty. It is the
/// designated hook for the codegen chunks: once blocks retire instructions
/// in a lump, any cycle/`mtime`-derived word a block samples mid-run is
/// added here. The `StateVec` layout ([`host::snapshot_state`]) already
/// excludes the free-running `mtime` CSRs, so with block-boundary comparison
/// this is expected to stay empty even after codegen lands.
pub fn differential_cycle_ignore_indices() -> Vec<usize> {
    Vec::new()
}

/// Hard cap on how many instructions one basic block may span. A basic
/// block is bounded by construction (it ends at the first control-flow or
/// unmodeled instruction), but flash could in principle contain a very long
/// straight-line run; this keeps the walk — and the eventual emitted body —
/// bounded regardless.
const MAX_BLOCK_INSTRS: u32 = 1024;

/// How a single decoded instruction affects the block walk.
///
/// The three-way split is the whole classification policy: it decides where
/// a basic block ends and which [`BailReason`] the terminating edge carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrClass {
    /// A straight-line instruction the block subsumes: arithmetic, logic,
    /// loads, stores, `LUI`/`AUIPC`, and every compressed form that decodes
    /// to one of those (`C.ADDI`, `C.LW`, `C.MV`, `C.ADD`, …). The walk
    /// includes it and continues to the next instruction.
    Sequential,
    /// A control-flow instruction the block ends **at and includes** —
    /// conditional branches, `JAL`/`JALR`, and their compressed forms
    /// (`C.J`, `C.JR`, `C.JALR`, `C.BEQZ`, `C.BNEZ`). A future codegen chunk
    /// emits these and side-exits with [`super::side_exit::SideExit::Chain`];
    /// until then the block bails with [`BailReason::PartialBlock`].
    ControlFlow,
    /// An instruction whose side effects are owned by the interpreter and
    /// which this frontend does not translate — CSR access, `ECALL`/`EBREAK`,
    /// `MRET`/`WFI`/`FENCE`, the A-extension atomics, and any `Unknown`
    /// encoding. The block is **cut before** it (the interpreter executes
    /// it), and the edge carries [`BailReason::UnsupportedInstruction`].
    Unmodeled,
}

/// Classify one decoded RISC-V instruction for the block walker.
///
/// This mirrors the control-flow / side-effect structure of
/// [`crate::cpu::riscv::RiscV::step`] without duplicating its semantics: it
/// only answers "does the block continue, end here, or cut before here?".
pub fn classify(inst: &Instruction) -> InstrClass {
    use Instruction::*;
    match inst {
        // ── Sequential: integer / logical / shifts (R- and I-type) ──────
        Lui { .. } | Auipc { .. }
        | Addi { .. } | Slti { .. } | Sltiu { .. } | Xori { .. } | Ori { .. } | Andi { .. }
        | Slli { .. } | Srli { .. } | Srai { .. }
        | Add { .. } | Sub { .. } | Sll { .. } | Slt { .. } | Sltu { .. }
        | Xor { .. } | Srl { .. } | Sra { .. } | Or { .. } | And { .. }
        // RV32M
        | Mul { .. } | Mulh { .. } | Mulhsu { .. } | Mulhu { .. }
        | Div { .. } | Divu { .. } | Rem { .. } | Remu { .. }
        // Loads / stores
        | Lb { .. } | Lh { .. } | Lw { .. } | Lbu { .. } | Lhu { .. }
        | Sb { .. } | Sh { .. } | Sw { .. }
        // Compressed-only forms that decode to non-control-flow ops.
        // (Many C.* instructions decode straight to the base variants above —
        // C.ADD→Add, C.LUI→Lui, C.SUB→Sub, C.SRLI→Srli, …; only the
        // genuinely compressed-only variants remain to be listed here.)
        | CAddi { .. } | CLi { .. } | CMv { .. }
        | CAddi16sp { .. } | CAddi4spn { .. } | CSli { .. }
        | CLw { .. } | CSw { .. } | CLwsp { .. } | CSwsp { .. } => InstrClass::Sequential,

        // ── Control flow: conditional branches, jumps (and C.* forms) ───
        Beq { .. } | Bne { .. } | Blt { .. } | Bge { .. } | Bltu { .. } | Bgeu { .. }
        | Jal { .. } | Jalr { .. }
        | CJ { .. } | CJr { .. } | CJalr { .. } | CBeqz { .. } | CBnez { .. } => {
            InstrClass::ControlFlow
        }

        // ── Unmodeled: interpreter-owned side effects + unknowns ─────────
        Fence | Ecall | Ebreak | Mret | Wfi
        | Csrrw { .. } | Csrrs { .. } | Csrrc { .. }
        | Csrrwi { .. } | Csrrsi { .. } | Csrrci { .. }
        | LrW { .. } | ScW { .. }
        | AmoSwapW { .. } | AmoAddW { .. } | AmoXorW { .. } | AmoOrW { .. } | AmoAndW { .. }
        | AmoMinW { .. } | AmoMaxW { .. } | AmoMinuW { .. } | AmoMaxuW { .. }
        | Unknown(_) => InstrClass::Unmodeled,
    }
}

/// Instruction length in bytes from the low halfword, per the RISC-V
/// encoding rule: a base 32-bit instruction has its two low bits set
/// (`0b11`); anything else is a 16-bit compressed instruction.
#[inline]
pub fn inst_len(low_halfword: u16) -> u64 {
    if (low_halfword & 0x3) == 0x3 {
        4
    } else {
        2
    }
}

/// Why the block walk stopped — drives the terminating [`ExitEdge`]'s
/// [`BailReason`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Termination {
    /// Ended at (and including) a branch/jump the frontend recognizes.
    ControlFlow,
    /// Cut before an instruction the frontend does not model.
    Unmodeled,
    /// Ran off the end of the flash [`CodeView`] with no terminator (or hit
    /// the [`MAX_BLOCK_INSTRS`] cap): a partial block.
    RanOffView,
}

/// Outcome of walking a basic block — the raw material for a [`BlockPlan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockWalk {
    /// Entry PC (the block-cache key).
    pub entry_pc: Pc,
    /// PC one past the last instruction the block subsumes (its natural
    /// fall-through). For an [`Termination::Unmodeled`] cut this is the PC of
    /// the unmodeled instruction itself (the interpreter resumes there).
    pub end_pc: Pc,
    /// Number of guest instructions the block subsumes.
    pub instr_count: u32,
    /// Why the walk stopped.
    termination: Termination,
}

impl BlockWalk {
    /// The [`BailReason`] the terminating edge reports. Purely diagnostic in
    /// this all-bail milestone (the interpreter is always correct), but it
    /// gives honest telemetry: `PartialBlock` for a recognized block whose
    /// body is not yet emitted, `UnsupportedInstruction` for a cut at an
    /// instruction this frontend does not model.
    fn bail_reason(&self) -> BailReason {
        match self.termination {
            Termination::ControlFlow | Termination::RanOffView => BailReason::PartialBlock,
            Termination::Unmodeled => BailReason::UnsupportedInstruction,
        }
    }
}

/// Walk a basic block starting at `pc` over the flash `code` view.
///
/// Decodes instruction-by-instruction (reusing [`decode_rv32`]), advancing
/// by the encoded length, and stops at the first control-flow or unmodeled
/// instruction, at the end of the view, or at the [`MAX_BLOCK_INSTRS`] cap.
/// Returns `None` if `pc` is not covered by `code`.
pub fn walk_block(pc: Pc, code: &CodeView<'_>) -> Option<BlockWalk> {
    // pc itself must be inside the view.
    code.from(pc)?;

    let mut cur = pc;
    let mut instr_count = 0u32;

    loop {
        // Out of translatable flash — a partial block ending here.
        let Some(bytes) = code.from(cur) else {
            return Some(BlockWalk {
                entry_pc: pc,
                end_pc: cur,
                instr_count,
                termination: Termination::RanOffView,
            });
        };

        // Need at least the length-defining halfword.
        if bytes.len() < 2 {
            return Some(BlockWalk {
                entry_pc: pc,
                end_pc: cur,
                instr_count,
                termination: Termination::RanOffView,
            });
        }
        let low = u16::from_le_bytes([bytes[0], bytes[1]]);
        let len = inst_len(low);

        // A 4-byte instruction that runs past the view is not decodable.
        if len == 4 && bytes.len() < 4 {
            return Some(BlockWalk {
                entry_pc: pc,
                end_pc: cur,
                instr_count,
                termination: Termination::RanOffView,
            });
        }

        // Assemble the little-endian word. `decode_rv32` inspects the low 16
        // bits for a compressed instruction, so the high halfword being zero
        // for a 2-byte instruction is harmless.
        let word = if len == 4 {
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            low as u32
        };
        let inst = decode_rv32(word);

        match classify(&inst) {
            InstrClass::Unmodeled => {
                // Cut BEFORE this instruction: the interpreter owns it.
                return Some(BlockWalk {
                    entry_pc: pc,
                    end_pc: cur,
                    instr_count,
                    termination: Termination::Unmodeled,
                });
            }
            InstrClass::ControlFlow => {
                // Include the terminator; the block ends after it.
                instr_count += 1;
                return Some(BlockWalk {
                    entry_pc: pc,
                    end_pc: cur + len,
                    instr_count,
                    termination: Termination::ControlFlow,
                });
            }
            InstrClass::Sequential => {
                instr_count += 1;
                cur += len;
                if instr_count >= MAX_BLOCK_INSTRS {
                    return Some(BlockWalk {
                        entry_pc: pc,
                        end_pc: cur,
                        instr_count,
                        termination: Termination::RanOffView,
                    });
                }
            }
        }
    }
}

/// The RV32IMC frontend. **Foundation milestone: all-bail** — it produces a
/// correct block plan (entry/end PC, instruction count, side-exit map) with
/// an empty `code` body, so every block side-exits to the interpreter. The
/// codegen chunks replace the empty body class-by-class.
#[derive(Debug, Clone, Copy, Default)]
pub struct RiscVFrontend;

impl RiscVFrontend {
    /// Construct the frontend.
    pub const fn new() -> Self {
        RiscVFrontend
    }
}

impl IsaFrontend for RiscVFrontend {
    fn isa_name(&self) -> &'static str {
        "rv32imc"
    }

    fn translate_block(&self, pc: Pc, code: &CodeView<'_>) -> Result<BlockPlan, FrontendRefusal> {
        if !code.covers(pc) {
            return Err(FrontendRefusal::PcOutOfRange);
        }

        // Chunk C: if the entry instruction is integer-ALU, emit real wasm
        // for the maximal ALU prefix. The block runs that prefix and
        // side-exits (fall-through Chain) to `end_pc`, where the interpreter
        // (or a future compiled block) picks up the first non-ALU
        // instruction.
        if let Some(blk) = emit::emit_alu_block(pc, code) {
            return Ok(BlockPlan {
                entry_pc: pc,
                end_pc: blk.end_pc,
                instr_count: blk.instr_count,
                code: blk.code,
                exits: blk.exits,
            });
        }

        // Entry is not ALU-emittable (a branch/jump → chunk D, a load/store →
        // chunk E, or an interpreter-owned op). Fall back to the foundation's
        // all-bail walk: a correct metadata-only plan with an empty body, so
        // `is_stub()` is true and the runtime side-exits to the interpreter.
        let walk = walk_block(pc, code).ok_or(FrontendRefusal::PcOutOfRange)?;
        if walk.instr_count == 0 {
            return Err(FrontendRefusal::BlockTooShort);
        }
        Ok(BlockPlan {
            entry_pc: walk.entry_pc,
            end_pc: walk.end_pc,
            instr_count: walk.instr_count,
            code: Vec::new(),
            exits: vec![ExitEdge {
                wire_code: 0,
                reason: walk.bail_reason(),
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Little-endian encoders for hand-built test programs.
    fn w(bytes: &mut Vec<u8>, word: u32) {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    fn c(bytes: &mut Vec<u8>, half: u16) {
        bytes.extend_from_slice(&half.to_le_bytes());
    }

    const BASE: Pc = 0x4200_0000;

    #[test]
    fn inst_len_rule() {
        assert_eq!(inst_len(0x0013), 4); // low bits 0b11 => 32-bit
        assert_eq!(inst_len(0x4501), 2); // c.li a0,0 => 16-bit
        assert_eq!(inst_len(0b11), 4);
        assert_eq!(inst_len(0b01), 2);
        assert_eq!(inst_len(0b00), 2);
        assert_eq!(inst_len(0b10), 2);
    }

    #[test]
    fn classifies_representative_instructions() {
        assert_eq!(
            classify(&Instruction::Addi {
                rd: 1,
                rs1: 0,
                imm: 5
            }),
            InstrClass::Sequential
        );
        assert_eq!(
            classify(&Instruction::Mul {
                rd: 1,
                rs1: 2,
                rs2: 3
            }),
            InstrClass::Sequential
        );
        assert_eq!(
            classify(&Instruction::Lw {
                rd: 1,
                rs1: 2,
                imm: 0
            }),
            InstrClass::Sequential
        );
        assert_eq!(
            classify(&Instruction::Beq {
                rs1: 1,
                rs2: 2,
                imm: 8
            }),
            InstrClass::ControlFlow
        );
        assert_eq!(
            classify(&Instruction::Jal { rd: 1, imm: 16 }),
            InstrClass::ControlFlow
        );
        assert_eq!(
            classify(&Instruction::CJ { imm: -4 }),
            InstrClass::ControlFlow
        );
        assert_eq!(
            classify(&Instruction::CJr { rs1: 1 }),
            InstrClass::ControlFlow
        );
        assert_eq!(classify(&Instruction::Ecall), InstrClass::Unmodeled);
        assert_eq!(
            classify(&Instruction::Csrrw {
                rd: 0,
                rs1: 1,
                csr: 0x300
            }),
            InstrClass::Unmodeled
        );
        assert_eq!(classify(&Instruction::Wfi), InstrClass::Unmodeled);
        assert_eq!(
            classify(&Instruction::AmoAddW {
                rd: 1,
                rs1: 2,
                rs2: 3
            }),
            InstrClass::Unmodeled
        );
        assert_eq!(classify(&Instruction::Unknown(0)), InstrClass::Unmodeled);
    }

    #[test]
    fn walk_sequential_run_ends_at_branch() {
        // addi x1,x0,1 ; addi x2,x0,2 ; beq x1,x2,+8
        let mut prog = Vec::new();
        w(&mut prog, 0x0010_0093); // addi x1,x0,1
        w(&mut prog, 0x0020_0113); // addi x2,x0,2
        w(&mut prog, 0x0020_8463); // beq x1,x2,8
        let view = CodeView::new(BASE, &prog);
        let walk = walk_block(BASE, &view).unwrap();
        assert_eq!(walk.entry_pc, BASE);
        assert_eq!(walk.instr_count, 3); // two addi + the branch (included)
        assert_eq!(walk.end_pc, BASE + 12);
        assert_eq!(walk.bail_reason(), BailReason::PartialBlock);
    }

    #[test]
    fn walk_cuts_before_unmodeled_ecall() {
        // addi x1,x0,1 ; ecall
        let mut prog = Vec::new();
        w(&mut prog, 0x0010_0093); // addi x1,x0,1
        w(&mut prog, 0x0000_0073); // ecall
        let view = CodeView::new(BASE, &prog);
        let walk = walk_block(BASE, &view).unwrap();
        assert_eq!(walk.instr_count, 1); // ecall is NOT included
        assert_eq!(walk.end_pc, BASE + 4); // cut point = pc of ecall
        assert_eq!(walk.bail_reason(), BailReason::UnsupportedInstruction);
    }

    #[test]
    fn walk_handles_compressed_lengths() {
        // c.li a0,1 (2 bytes) ; addi x0,x0,0 (4 bytes) ; c.j 0 (2 bytes, CF)
        let mut prog = Vec::new();
        c(&mut prog, 0x4505); // c.li a0,1
        w(&mut prog, 0x0000_0013); // addi x0,x0,0 (nop, 4 bytes)
        c(&mut prog, 0xa001); // c.j 0
        let view = CodeView::new(BASE, &prog);
        let walk = walk_block(BASE, &view).unwrap();
        assert_eq!(walk.instr_count, 3);
        // 2 + 4 + 2 = 8 bytes, block includes the c.j terminator.
        assert_eq!(walk.end_pc, BASE + 8);
        assert_eq!(walk.termination, Termination::ControlFlow);
    }

    #[test]
    fn walk_runs_off_end_of_view() {
        // Pure straight-line run with no terminator inside the view.
        let mut prog = Vec::new();
        w(&mut prog, 0x0010_0093); // addi x1,x0,1
        w(&mut prog, 0x0020_0113); // addi x2,x0,2
        let view = CodeView::new(BASE, &prog);
        let walk = walk_block(BASE, &view).unwrap();
        assert_eq!(walk.instr_count, 2);
        assert_eq!(walk.end_pc, BASE + 8);
        assert_eq!(walk.termination, Termination::RanOffView);
        assert_eq!(walk.bail_reason(), BailReason::PartialBlock);
    }

    #[test]
    fn translate_block_emits_alu_prefix() {
        // addi x1,x0,1 ; jal x0,0 (self-loop terminator). Chunk C emits the
        // addi and stops before the jal (chunk D territory).
        let mut prog = Vec::new();
        w(&mut prog, 0x0010_0093); // addi x1,x0,1
        w(&mut prog, 0x0000_006f); // jal x0,0
        let view = CodeView::new(BASE, &prog);
        let plan = RiscVFrontend::new().translate_block(BASE, &view).unwrap();
        assert!(!plan.is_stub(), "ALU entry now emits real wasm");
        assert_eq!(plan.entry_pc, BASE);
        assert_eq!(plan.instr_count, 1, "only the addi; jal excluded");
        assert_eq!(plan.end_pc, BASE + 4, "ends before the jal");
        assert_eq!(plan.exits.len(), 1);
        assert_eq!(plan.exits[0].wire_code, emit::WIRE_FALL_THROUGH);
        assert_eq!(&plan.code[0..4], &[0x00, 0x61, 0x73, 0x6d], "wasm magic");
    }

    #[test]
    fn translate_block_non_alu_entry_falls_back_to_stub() {
        // Entry is a jump (chunk D) → no ALU prefix, so the foundation's
        // all-bail metadata plan is produced.
        let mut prog = Vec::new();
        w(&mut prog, 0x0000_006f); // jal x0,0
        let view = CodeView::new(BASE, &prog);
        let plan = RiscVFrontend::new().translate_block(BASE, &view).unwrap();
        assert!(plan.is_stub(), "non-ALU entry stays all-bail");
        assert_eq!(plan.instr_count, 1);
        assert_eq!(plan.exits[0].reason, BailReason::PartialBlock);
    }

    #[test]
    fn translate_block_refuses_out_of_range_pc() {
        let prog = vec![0u8; 8];
        let view = CodeView::new(BASE, &prog);
        assert!(matches!(
            RiscVFrontend::new().translate_block(BASE + 0x1000, &view),
            Err(FrontendRefusal::PcOutOfRange)
        ));
    }

    #[test]
    fn translate_block_refuses_zero_instruction_block() {
        // Entry is itself an unmodeled instruction (ecall) → BlockTooShort.
        let mut prog = Vec::new();
        w(&mut prog, 0x0000_0073); // ecall
        let view = CodeView::new(BASE, &prog);
        assert!(matches!(
            RiscVFrontend::new().translate_block(BASE, &view),
            Err(FrontendRefusal::BlockTooShort)
        ));
    }

    #[test]
    fn isa_name_is_rv32imc() {
        assert_eq!(RiscVFrontend::new().isa_name(), "rv32imc");
    }
}
