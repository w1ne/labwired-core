// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Thumb / Thumb-2 frontend for the universal dispatch JIT.
//!
//! Same contract as the RV32IMC frontend: the interpreter remains the spec.
//! Compiled blocks exit on MMIO, WFI, CPS/MRS/MSR, incomplete IT, and any
//! instruction this frontend does not model. Complete IT blocks of
//! emittable ALU are compiled (predicated; 16-bit DP does not set flags).
//! Cycle accounting is 1 retired guest
//! instruction per boundary, matching `CortexM::step_batch`.

use crate::decoder::arm::{decode_thumb_16, decode_thumb_32, Instruction};

use super::frontend::{BlockPlan, ExitEdge, FrontendRefusal, IsaFrontend};
use super::side_exit::BailReason;
use super::{CodeView, Pc};

pub mod emit;
pub mod host;

pub use emit::{MemBinding, RamWindow};
pub use host::{snapshot_state, CortexMJitHost};

#[cfg(feature = "jit")]
pub mod exec;
#[cfg(feature = "jit")]
pub use exec::{
    CompiledBlock, CortexMJitEngine, CortexMWasmJit, EngineStats, MIN_PROFITABLE_BLOCK_INSTRS,
};

/// Nothing in the Cortex-M [`StateVec`](super::StateVec) is cycle-derived
/// (SysTick lives on the bus, not in the core), so the differential harness
/// compares every word.
pub fn differential_cycle_ignore_indices() -> Vec<usize> {
    Vec::new()
}

pub(crate) const MAX_BLOCK_INSTRS: u32 = 1024;

/// How a decoded Thumb instruction affects the block walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrClass {
    Sequential,
    ControlFlow,
    Unmodeled,
}

/// True when the first halfword is a 32-bit Thumb-2 prefix.
#[inline]
pub fn is_thumb32(h1: u16) -> bool {
    (h1 & 0xE000) == 0xE000 && (h1 & 0x1800) != 0
}

/// Instruction length in bytes from the first halfword.
#[inline]
pub fn inst_len(h1: u16) -> u64 {
    if is_thumb32(h1) {
        4
    } else {
        2
    }
}

/// Classify one decoded Thumb instruction for the block walker.
pub fn classify(inst: &Instruction) -> InstrClass {
    use Instruction::*;
    match inst {
        Nop
        | Barrier
        | MovImm { .. }
        | Movw { .. }
        | Movt { .. }
        | AddReg { .. }
        | AddImm3 { .. }
        | AddImm8 { .. }
        | AddSp { .. }
        | AddSpReg { .. }
        | AddRegHigh { .. }
        | AddwImm { .. }
        | SubReg { .. }
        | SubImm3 { .. }
        | SubImm8 { .. }
        | SubSp { .. }
        | SubwImm { .. }
        | CmpImm { .. }
        | CmpReg { .. }
        | Cmn { .. }
        | Tst { .. }
        | And { .. }
        | Bic { .. }
        | Orr { .. }
        | Eor { .. }
        | Mvn { .. }
        | Lsl { .. }
        | Lsr { .. }
        | Asr { .. }
        | LslReg { .. }
        | LsrReg { .. }
        | AsrReg { .. }
        | Ror { .. }
        | Adc { .. }
        | Sbc { .. }
        | Rsbs { .. }
        | Mul { .. }
        | Mul32 { .. }
        | DataProc32 { .. }
        | DataProcImm32 { .. }
        | Uxtb { .. }
        | Uxth { .. }
        | Sxtb { .. }
        | Sxth { .. }
        | Clz { .. }
        | Rbit { .. }
        | Rev { .. }
        | Rev16 { .. }
        | RevSh { .. }
        | Adr { .. }
        | VaddF32 { .. }
        | VsubF32 { .. }
        | VmulF32 { .. }
        | VdivF32 { .. }
        | VmovF32Reg { .. }
        | VmovF32Imm { .. }
        | VmovSnRt { .. }
        | VmovRtSn { .. }
        | Vldr { .. }
        | Vstr { .. }
        | LdrImm { .. }
        | StrImm { .. }
        | LdrbImm { .. }
        | StrbImm { .. }
        | LdrhImm { .. }
        | StrhImm { .. }
        | LdrReg { .. }
        | StrReg { .. }
        | LdrbReg { .. }
        | StrbReg { .. }
        | LdrhReg { .. }
        | StrhReg { .. }
        | LdrsbReg { .. }
        | LdrshReg { .. }
        | StrImm32 { .. }
        | StrImm32Idx { .. }
        | LdrLit { .. }
        | LdrSp { .. }
        | StrSp { .. }
        | Push { .. }
        | Pop { p: false, .. }
        | Ldm { .. }
        | Stm { .. }
        | StmiaW { .. }
        | StmdbW { .. } => InstrClass::Sequential,

        LdrImm32 { rt, .. } if *rt != 15 => InstrClass::Sequential,
        LdrImm32 { .. } => InstrClass::ControlFlow,
        LdrImm32Idx { rt, .. } if *rt != 15 => InstrClass::Sequential,
        LdrImm32Idx { .. } => InstrClass::ControlFlow,

        LdmiaW { reg_list, .. } | LdmdbW { reg_list, .. } if (*reg_list & (1 << 15)) == 0 => {
            InstrClass::Sequential
        }

        Branch { .. }
        | BranchCond { .. }
        | Cbz { .. }
        | Cbnz { .. }
        | Bl { .. }
        | Bx { .. }
        | BlxReg { .. }
        | Pop { p: true, .. }
        | MovReg { rd: 15, .. }
        | LdmiaW { .. }
        | LdmdbW { .. } => InstrClass::ControlFlow,

        MovReg { .. } => InstrClass::Sequential,

        Wfi
        | It { .. }
        | Cpsie { .. }
        | Cpsid { .. }
        | Mrs { .. }
        | Msr { .. }
        | Svc { .. }
        | Bkpt { .. }
        | Tbb { .. }
        | Tbh { .. }
        | Unknown(_)
        | Unknown32(_, _) => InstrClass::Unmodeled,

        _ => InstrClass::Unmodeled,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Termination {
    ControlFlow,
    Unmodeled,
    RanOffView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockWalk {
    pub entry_pc: Pc,
    pub end_pc: Pc,
    pub instr_count: u32,
    termination: Termination,
}

impl BlockWalk {
    fn bail_reason(&self) -> BailReason {
        match self.termination {
            Termination::ControlFlow | Termination::RanOffView => BailReason::PartialBlock,
            Termination::Unmodeled => BailReason::UnsupportedInstruction,
        }
    }
}

/// Decode the instruction at `pc` in `code`.
pub fn decode_at(pc: Pc, code: &CodeView<'_>) -> Option<(Instruction, u64)> {
    let bytes = code.from(pc)?;
    if bytes.len() < 2 {
        return None;
    }
    let h1 = u16::from_le_bytes([bytes[0], bytes[1]]);
    let len = inst_len(h1);
    if len == 4 {
        if bytes.len() < 4 {
            return None;
        }
        let h2 = u16::from_le_bytes([bytes[2], bytes[3]]);
        Some((decode_thumb_32(h1, h2), 4))
    } else {
        Some((decode_thumb_16(h1), 2))
    }
}

/// Walk a basic block starting at `pc`.
pub fn walk_block(pc: Pc, code: &CodeView<'_>) -> Option<BlockWalk> {
    code.from(pc)?;
    let mut cur = pc;
    let mut instr_count = 0u32;
    loop {
        let Some((inst, len)) = decode_at(cur, code) else {
            return Some(BlockWalk {
                entry_pc: pc,
                end_pc: cur,
                instr_count,
                termination: Termination::RanOffView,
            });
        };
        match classify(&inst) {
            InstrClass::Unmodeled => {
                return Some(BlockWalk {
                    entry_pc: pc,
                    end_pc: cur,
                    instr_count,
                    termination: Termination::Unmodeled,
                });
            }
            InstrClass::ControlFlow => {
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

/// Thumb-2 frontend. Emits wasm for the maximal ALU + load/store prefix,
/// optionally ended by one control-flow terminator.
#[derive(Debug, Clone, Copy, Default)]
pub struct CortexMFrontend {
    ram_window: Option<RamWindow>,
}

impl CortexMFrontend {
    pub const fn new() -> Self {
        CortexMFrontend { ram_window: None }
    }

    pub const fn with_ram_window(base: u32, len: u32) -> Self {
        CortexMFrontend {
            ram_window: Some((base, len)),
        }
    }

    pub fn set_ram_window(&mut self, base: u32, len: u32) {
        self.ram_window = Some((base, len));
    }

    pub fn translate_block_thumb(
        &self,
        pc: Pc,
        code: &CodeView<'_>,
    ) -> Result<(BlockPlan, Option<MemBinding>), FrontendRefusal> {
        if !code.covers(pc) {
            return Err(FrontendRefusal::PcOutOfRange);
        }
        if let Some(blk) = emit::emit_block(pc, code, self.ram_window) {
            let plan = BlockPlan {
                entry_pc: pc,
                end_pc: blk.end_pc,
                instr_count: blk.instr_count,
                code: blk.code,
                exits: blk.exits,
            };
            return Ok((plan, blk.binding));
        }
        let walk = walk_block(pc, code).ok_or(FrontendRefusal::PcOutOfRange)?;
        if walk.instr_count == 0 {
            return Err(FrontendRefusal::BlockTooShort);
        }
        let plan = BlockPlan {
            entry_pc: walk.entry_pc,
            end_pc: walk.end_pc,
            instr_count: walk.instr_count,
            code: Vec::new(),
            exits: vec![ExitEdge {
                wire_code: 0,
                reason: walk.bail_reason(),
            }],
        };
        Ok((plan, None))
    }
}

impl IsaFrontend for CortexMFrontend {
    fn isa_name(&self) -> &'static str {
        "thumb2"
    }

    fn translate_block(&self, pc: Pc, code: &CodeView<'_>) -> Result<BlockPlan, FrontendRefusal> {
        self.translate_block_thumb(pc, code).map(|(plan, _)| plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(bytes: &mut Vec<u8>, half: u16) {
        bytes.extend_from_slice(&half.to_le_bytes());
    }

    const BASE: Pc = 0x0000_1000;

    #[test]
    fn inst_len_rule() {
        assert_eq!(inst_len(0x2001), 2); // movs r0, #1
        assert_eq!(inst_len(0xBF30), 2); // wfi
        assert_eq!(inst_len(0xF000), 4); // bl prefix
        assert_eq!(inst_len(0xE000), 2); // b T2 (unconditional 16-bit)
    }

    #[test]
    fn classifies_representative_instructions() {
        assert_eq!(
            classify(&Instruction::AddImm8 { rd: 0, imm: 1 }),
            InstrClass::Sequential
        );
        assert_eq!(
            classify(&Instruction::Branch { offset: -4 }),
            InstrClass::ControlFlow
        );
        assert_eq!(classify(&Instruction::Wfi), InstrClass::Unmodeled);
        assert_eq!(
            classify(&Instruction::It { cond: 0, mask: 1 }),
            InstrClass::Unmodeled
        );
        assert_eq!(
            classify(&Instruction::MovReg { rd: 15, rm: 0 }),
            InstrClass::ControlFlow
        );
        assert_eq!(
            classify(&Instruction::MovReg { rd: 0, rm: 1 }),
            InstrClass::Sequential
        );
        assert_eq!(
            classify(&Instruction::LdrImm32 {
                rt: 15,
                rn: 0,
                imm12: 0
            }),
            InstrClass::ControlFlow
        );
        assert_eq!(
            classify(&Instruction::LdrImm32 {
                rt: 0,
                rn: 0,
                imm12: 0
            }),
            InstrClass::Sequential
        );
        assert_eq!(
            classify(&Instruction::VaddF32 {
                sd: 0,
                sn: 0,
                sm: 0
            }),
            InstrClass::Sequential
        );
    }

    #[test]
    fn walk_sequential_run_ends_at_branch() {
        let mut prog = Vec::new();
        h(&mut prog, 0x2001); // movs r0, #1
        h(&mut prog, 0x3001); // adds r0, #1
        h(&mut prog, 0xE7FE); // b .
        let view = CodeView::new(BASE, &prog);
        let walk = walk_block(BASE, &view).unwrap();
        assert_eq!(walk.instr_count, 3);
        assert_eq!(walk.end_pc, BASE + 6);
        assert_eq!(walk.bail_reason(), BailReason::PartialBlock);
    }

    #[test]
    fn walk_cuts_before_wfi() {
        let mut prog = Vec::new();
        h(&mut prog, 0x2001); // movs r0, #1
        h(&mut prog, 0xBF30); // wfi
        let view = CodeView::new(BASE, &prog);
        let walk = walk_block(BASE, &view).unwrap();
        assert_eq!(walk.instr_count, 1);
        assert_eq!(walk.end_pc, BASE + 2);
        assert_eq!(walk.bail_reason(), BailReason::UnsupportedInstruction);
    }

    #[test]
    fn translate_block_emits_alu_prefix_and_terminator() {
        let mut prog = Vec::new();
        h(&mut prog, 0x2001); // movs r0, #1
        h(&mut prog, 0xE7FE); // b .
        let view = CodeView::new(BASE, &prog);
        let plan = CortexMFrontend::new().translate_block(BASE, &view).unwrap();
        assert!(!plan.is_stub(), "ALU + branch emits real wasm");
        assert_eq!(plan.instr_count, 2);
        assert_eq!(&plan.code[0..4], &[0x00, 0x61, 0x73, 0x6d]);
    }

    #[test]
    fn translate_block_refuses_wfi_entry() {
        let mut prog = Vec::new();
        h(&mut prog, 0xBF30);
        let view = CodeView::new(BASE, &prog);
        assert!(matches!(
            CortexMFrontend::new().translate_block(BASE, &view),
            Err(FrontendRefusal::BlockTooShort)
        ));
    }

    #[test]
    fn isa_name_is_thumb2() {
        assert_eq!(CortexMFrontend::new().isa_name(), "thumb2");
    }
}
