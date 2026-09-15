// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Thumb-2 frontend for the universal dispatch JIT.
//!
//! This is an **all-bail** [`IsaFrontend`]: it walks a basic block over the
//! flash [`CodeView`] using the shared ARM decoder
//! ([`crate::decoder::arm`]), classifies every instruction, and produces a
//! correct [`BlockPlan`] — `entry_pc`, `end_pc`, `instr_count`, and the
//! side-exit `exits` map — but with an **empty `code`** body
//! ([`BlockPlan::is_stub`] is `true`). Every block therefore side-exits to
//! the interpreter, exactly like the passthrough stub.
//!
//! Why build a real walker that emits no code? Because the walk +
//! classification is the ISA-aware skeleton that later codegen chunks
//! (integer arithmetic, branches, loads/stores) hang wasm emission onto.
//! Landing it all-bail lets a differential harness prove the dispatch /
//! host / snapshot plumbing is byte-identical to the interpreter before a
//! single wasm byte is emitted.
//!
//! ## The decode reuse
//!
//! [`crate::decoder::arm::decode_thumb_16`] / [`decode_thumb_32`] already
//! decode 16-bit Thumb and 32-bit Thumb-2 into one flat, `Copy`
//! [`Instruction`] enum. The frontend does **not** re-implement any of
//! that: it reads the length-defining halfword, applies the same 32-bit
//! prefix rule the DAP uses (`(h1 >> 11) >= 0b11101`), assembles the
//! little-endian halfword(s), calls the matching decoder, and classifies
//! the result.

use crate::decoder::arm::{decode_thumb_16, decode_thumb_32, Instruction};

use super::frontend::{BlockPlan, ExitEdge, FrontendRefusal, IsaFrontend};
use super::side_exit::BailReason;
use super::{CodeView, Pc};

pub mod host;

pub use host::{snapshot_state, CortexMJitHost};

/// Indices into the Cortex-M [`StateVec`](super::StateVec) that a batched
/// JIT run may legitimately compute differently from a per-instruction
/// interpreter run and which the differential harness should mask.
///
/// In this all-bail foundation milestone the JIT executes **zero**
/// instructions itself — every block side-exits and the interpreter runs
/// each instruction — so nothing is volatile and this is empty. It is the
/// designated hook for the codegen chunks.
pub fn differential_cycle_ignore_indices() -> Vec<usize> {
    Vec::new()
}

/// Hard cap on how many instructions one basic block may span. A basic
/// block is bounded by construction (it ends at the first control-flow or
/// unmodeled instruction), but flash could in principle contain a very long
/// straight-line run; this keeps the walk — and the eventual emitted body —
/// bounded regardless.
pub(crate) const MAX_BLOCK_INSTRS: u32 = 1024;

/// How a single decoded instruction affects the block walk.
///
/// The three-way split is the whole classification policy: it decides where
/// a basic block ends and which [`BailReason`] the terminating edge carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrClass {
    /// A straight-line instruction the block subsumes: ALU, MOV, CMP, and
    /// LDR/STR (the classes later emit will fill in). The walk includes it
    /// and continues to the next instruction.
    Sequential,
    /// A control-flow instruction the block ends **at and includes** —
    /// `B`, `Bcc`, `BL`, `BX`, `BLX`. A future codegen chunk emits these
    /// and side-exits with [`super::side_exit::SideExit::Chain`]; until
    /// then the block bails with [`BailReason::PartialBlock`].
    ControlFlow,
    /// An instruction whose side effects are owned by the interpreter and
    /// which this frontend does not translate — `WFI`, `SVC`, `MRS`/`MSR`,
    /// `CPS`, `IT`, and any `Unknown` encoding. The block is **cut before**
    /// it (the interpreter executes it), and the edge carries
    /// [`BailReason::UnsupportedInstruction`].
    Unmodeled,
}

/// Classify one decoded Thumb instruction for the block walker.
///
/// This only answers "does the block continue, end here, or cut before
/// here?" — it does not duplicate executor semantics.
pub fn classify(inst: &Instruction) -> InstrClass {
    use Instruction::*;
    match inst {
        // ── Sequential: ALU / MOV / CMP / LDR/STR (later emit) ──────────
        Nop
        | MovImm { .. }
        | MovReg { .. }
        | Movw { .. }
        | Movt { .. }
        | AddReg { .. }
        | AddImm3 { .. }
        | AddImm8 { .. }
        | AddSp { .. }
        | AddRegHigh { .. }
        | AddSpReg { .. }
        | AddwImm { .. }
        | SubReg { .. }
        | SubImm3 { .. }
        | SubImm8 { .. }
        | SubSp { .. }
        | SubwImm { .. }
        | Rsbs { .. }
        | CmpImm { .. }
        | CmpReg { .. }
        | Cmn { .. }
        | Tst { .. }
        | And { .. }
        | Bic { .. }
        | Orr { .. }
        | Eor { .. }
        | Mvn { .. }
        | Adc { .. }
        | Sbc { .. }
        | Lsl { .. }
        | Lsr { .. }
        | Asr { .. }
        | LslReg { .. }
        | LsrReg { .. }
        | AsrReg { .. }
        | Ror { .. }
        | LdrImm { .. }
        | StrImm { .. }
        | StrReg { .. }
        | LdrLit { .. }
        | LdrImm32 { .. }
        | StrImm32 { .. }
        | LdrImm32Idx { .. }
        | StrImm32Idx { .. }
        | LdrbImm { .. }
        | LdrbReg { .. }
        | StrbImm { .. }
        | StrbReg { .. }
        | LdrhImm { .. }
        | StrhImm { .. }
        | StrhReg { .. }
        | LdrsbReg { .. }
        | LdrhReg { .. }
        | LdrshReg { .. }
        | LdrReg { .. }
        | LdrSp { .. }
        | StrSp { .. }
        | Ldrd { .. }
        | Strd { .. }
        | Mul { .. }
        | Mul32 { .. }
        | Uxtb { .. }
        | Sxth { .. }
        | Sxtb { .. }
        | Uxth { .. }
        | ExtendW { .. }
        | Adr { .. }
        | Bfi { .. }
        | Bfc { .. }
        | Sbfx { .. }
        | Ubfx { .. }
        | Clz { .. }
        | Rbit { .. }
        | Rev { .. }
        | Rev16 { .. }
        | RevSh { .. }
        | SimdAddSub8 { .. }
        | SimdAddSub16 { .. }
        | Sel { .. }
        | Udiv { .. }
        | Sdiv { .. }
        | DataProc32 { .. }
        | DataProcImm32 { .. }
        | ShiftReg32 { .. }
        | Smull { .. }
        | Umull { .. }
        | Smlal { .. }
        | Umlal { .. }
        | Umaal { .. }
        | Mla { .. }
        | Mls { .. }
        | SmlaXy { .. } => InstrClass::Sequential,

        // ── Control flow: B, Bcc, BL, BX, BLX ───────────────────────────
        Branch { .. } | BranchCond { .. } | Bl { .. } | Bx { .. } | BlxReg { .. } => {
            InstrClass::ControlFlow
        }

        // ── Unmodeled: interpreter-owned side effects + unknowns ────────
        Wfi
        | Svc { .. }
        | Unknown(_)
        | Unknown32(_, _)
        | Cpsie { .. }
        | Cpsid { .. }
        | Push { .. }
        | Pop { .. }
        | Ldm { .. }
        | Stm { .. }
        | LdmiaW { .. }
        | StmdbW { .. }
        | StmiaW { .. }
        | LdmdbW { .. }
        | Cbz { .. }
        | Cbnz { .. }
        | Tbb { .. }
        | Tbh { .. }
        | It { .. }
        | Bkpt { .. }
        | Barrier
        | Mrs { .. }
        | Msr { .. }
        | Vldr { .. }
        | Vstr { .. }
        | VmulF32 { .. }
        | VaddF32 { .. }
        | VsubF32 { .. }
        | VdivF32 { .. }
        | VfmaF32 { .. }
        | VfmsF32 { .. }
        | VfnmaF32 { .. }
        | VfnmsF32 { .. }
        | VmovSnRt { .. }
        | VmovRtSn { .. }
        | VmovF32Reg { .. }
        | VmovF32Imm { .. }
        | VcvtF32FromInt { .. }
        | VcvtIntFromF32 { .. }
        | VfpStoreMultiple { .. }
        | VfpLoadMultiple { .. }
        | Vldr64 { .. }
        | Vstr64 { .. }
        | VmovF64Reg { .. }
        | VmovDRtRt2 { .. }
        | VmovRtRt2D { .. }
        | VaddF64 { .. }
        | VsubF64 { .. }
        | VmulF64 { .. }
        | VdivF64 { .. } => InstrClass::Unmodeled,
    }
}

/// Instruction length in bytes from the low halfword, per the Thumb-2
/// encoding rule used by the DAP: a 32-bit instruction has bits [15:11]
/// of the first halfword in `{0b11101, 0b11110, 0b11111}`; anything else
/// (including the 16-bit unconditional `B` encoding `0b11100`) is 16-bit.
#[inline]
pub fn inst_len(low_halfword: u16) -> u64 {
    if (low_halfword >> 11) >= 0b11101 {
        4
    } else {
        2
    }
}

/// Why the block walk stopped — drives the terminating [`ExitEdge`]'s
/// [`BailReason`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Termination {
    /// Ended at (and including) a branch the frontend recognizes.
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
/// Decodes instruction-by-instruction (reusing [`decode_thumb_16`] /
/// [`decode_thumb_32`]), advancing by the encoded length, and stops at the
/// first control-flow or unmodeled instruction, at the end of the view, or
/// at the [`MAX_BLOCK_INSTRS`] cap. Returns `None` if `pc` is not covered
/// by `code`.
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
        let h1 = u16::from_le_bytes([bytes[0], bytes[1]]);
        let len = inst_len(h1);

        // A 4-byte instruction that runs past the view is not decodable.
        if len == 4 && bytes.len() < 4 {
            return Some(BlockWalk {
                entry_pc: pc,
                end_pc: cur,
                instr_count,
                termination: Termination::RanOffView,
            });
        }

        let inst = if len == 4 {
            let h2 = u16::from_le_bytes([bytes[2], bytes[3]]);
            decode_thumb_32(h1, h2)
        } else {
            decode_thumb_16(h1)
        };

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

/// The Thumb-2 frontend. This all-bail foundation walks and classifies but
/// emits no wasm: every block is a metadata-only stub that side-exits to
/// the interpreter.
#[derive(Debug, Clone, Copy, Default)]
pub struct ThumbFrontend;

impl ThumbFrontend {
    /// Construct the all-bail Thumb-2 frontend.
    pub const fn new() -> Self {
        ThumbFrontend
    }
}

impl IsaFrontend for ThumbFrontend {
    fn isa_name(&self) -> &'static str {
        "thumb2"
    }

    fn translate_block(&self, pc: Pc, code: &CodeView<'_>) -> Result<BlockPlan, FrontendRefusal> {
        if !code.covers(pc) {
            return Err(FrontendRefusal::PcOutOfRange);
        }

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

    fn h(bytes: &mut Vec<u8>, half: u16) {
        bytes.extend_from_slice(&half.to_le_bytes());
    }

    const BASE: Pc = 0;

    #[test]
    fn inst_len_rule_matches_dap() {
        // 16-bit unconditional B (bits [15:11] = 0b11100) is *not* 32-bit.
        assert_eq!(inst_len(0xE000), 2);
        assert_eq!(inst_len(0xE7FE), 2);
        assert_eq!(inst_len(0x2001), 2); // MOV r0, #1

        // 32-bit prefixes: 0b11101 / 0b11110 / 0b11111.
        assert_eq!(inst_len(0xE800), 4);
        assert_eq!(inst_len(0xF000), 4);
        assert_eq!(inst_len(0xF800), 4);
        assert!((0xE800u16 >> 11) >= 0b11101);
        assert!((0xE7FEu16 >> 11) < 0b11101);
    }

    #[test]
    fn isa_name_is_thumb2() {
        assert_eq!(ThumbFrontend::new().isa_name(), "thumb2");
    }

    #[test]
    fn walk_includes_branch_terminator() {
        let mut prog = Vec::new();
        h(&mut prog, 0x2001); // MOV r0, #1
        h(&mut prog, 0x1C40); // ADDS r0, r0, #1
        h(&mut prog, 0xE7FC); // B back to entry
        let view = CodeView::new(BASE, &prog);
        let walk = walk_block(BASE, &view).unwrap();
        assert_eq!(walk.entry_pc, BASE);
        assert_eq!(walk.instr_count, 3);
        assert_eq!(walk.end_pc, BASE + 6);
        assert_eq!(walk.termination, Termination::ControlFlow);
        assert_eq!(walk.bail_reason(), BailReason::PartialBlock);
    }

    #[test]
    fn walk_cuts_before_wfi() {
        let mut prog = Vec::new();
        h(&mut prog, 0x2001); // MOV r0, #1
        h(&mut prog, 0xBF30); // WFI
        let view = CodeView::new(BASE, &prog);
        let walk = walk_block(BASE, &view).unwrap();
        assert_eq!(walk.instr_count, 1);
        assert_eq!(walk.end_pc, BASE + 2);
        assert_eq!(walk.termination, Termination::Unmodeled);
        assert_eq!(walk.bail_reason(), BailReason::UnsupportedInstruction);
    }

    #[test]
    fn translate_block_refuses_wfi_only() {
        let mut prog = Vec::new();
        h(&mut prog, 0xBF30);
        let view = CodeView::new(BASE, &prog);
        assert!(matches!(
            ThumbFrontend::new().translate_block(BASE, &view),
            Err(FrontendRefusal::BlockTooShort)
        ));
    }

    #[test]
    fn translate_block_refuses_out_of_range_pc() {
        let prog = vec![0u8; 8];
        let view = CodeView::new(BASE, &prog);
        assert!(matches!(
            ThumbFrontend::new().translate_block(BASE + 0x1000, &view),
            Err(FrontendRefusal::PcOutOfRange)
        ));
    }
}
