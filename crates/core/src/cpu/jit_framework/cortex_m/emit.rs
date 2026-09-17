// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Thumb / Thumb-2 wasm codegen for the Cortex-M JIT.
//!
//! Mirrors the RV32IMC emit core: a straight-line ALU + in-window load/store
//! prefix, optional control-flow terminator, MMIO side-exit. Flag updates
//! match `CortexM::update_nz` / `update_nzcv` / `add_with_flags` /
//! `sub_with_flags` exactly.

use crate::decoder::arm::Instruction;

use super::super::frontend::ExitEdge;
use super::super::riscv::wasm_encode::{build_module, build_module_ram_host, enc, op};
use super::super::side_exit::BailReason;
use super::super::{CodeView, Pc};
use super::decode_at;

pub const WIRE_FALL_THROUGH: i32 = 0;
pub const WIRE_CHAIN_DYNAMIC: i32 = 1;
pub const WIRE_MEM_FAULT: i32 = 2;
pub const WIRE_UNSUPPORTED: i32 = 3;

/// Wire codes for the `vfp.binop` host import. The host maps these back to
/// [`crate::cpu::cortex_m::VfpBinOp`]; the discriminants must match.
pub const VFP_OP_ADD: i32 = 0;
pub const VFP_OP_SUB: i32 = 1;
pub const VFP_OP_MUL: i32 = 2;
pub const VFP_OP_DIV: i32 = 3;

const XPSR_LOCAL: u32 = 15;
const SCRATCH_LOCAL: u32 = 16;
const RESULT_LOCAL: u32 = 17;
const OP2_LOCAL: u32 = 18;
const LOCAL_COUNT: u32 = 19;

pub const NEXT_PC_SLOT: i32 = 16 * 4;
pub const FAULT_PC_SLOT: u32 = 68;
pub const FAULT_RETIRED_SLOT: u32 = 72;
pub const RES_FLAG_SLOT: u32 = 76;
/// IT state (`cond << 4 | mask`) as of BEFORE the faulting instruction.
/// Written by every [`Body::emit_fault`]/[`Body::emit_unsupported`] and
/// re-applied by the runtime on an interpreter resume, so a side-exit in the
/// middle of an IT body does not replay that instruction unpredicated.
pub const IT_STATE_SLOT: u32 = 80;
pub const RAM_WINDOW_OFF: u32 = 256;

pub type RamWindow = (u32, u32);

#[derive(Debug, Clone, Copy)]
pub struct MemBinding {
    pub ram_len: usize,
    pub has_store: bool,
}

pub fn is_alu_emittable(inst: &Instruction) -> bool {
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
        | LdrLit { .. }
        | VaddF32 { .. }
        | VsubF32 { .. }
        | VmulF32 { .. }
        | VdivF32 { .. }
        | VmovF32Reg { .. }
        | VmovF32Imm { .. }
        | VmovSnRt { .. }
        | VmovRtSn { .. } => true,
        AddRegHigh { rd, .. } | MovReg { rd, .. } if *rd != 15 => true,
        DataProcImm32 { op, .. } | DataProc32 { op, .. } if dataproc_op_emittable(*op) => true,
        _ => false,
    }
}

pub fn is_mem_emittable(inst: &Instruction) -> bool {
    use Instruction::*;
    match inst {
        LdrImm { rt, rn, .. }
        | StrImm { rt, rn, .. }
        | LdrbImm { rt, rn, .. }
        | StrbImm { rt, rn, .. }
        | LdrhImm { rt, rn, .. }
        | StrhImm { rt, rn, .. }
        | LdrReg { rt, rn, .. }
        | StrReg { rt, rn, .. }
        | LdrbReg { rt, rn, .. }
        | StrbReg { rt, rn, .. }
        | LdrhReg { rt, rn, .. }
        | StrhReg { rt, rn, .. }
        | LdrsbReg { rt, rn, .. }
        | LdrshReg { rt, rn, .. }
        | LdrImm32 { rt, rn, .. }
        | StrImm32 { rt, rn, .. }
        | LdrImm32Idx { rt, rn, .. }
        | StrImm32Idx { rt, rn, .. }
            if *rt != 15 && *rn != 15 =>
        {
            true
        }
        LdrSp { rt, .. } | StrSp { rt, .. } if *rt != 15 => true,
        Push { .. } | Pop { p: false, .. } => true,
        Ldm { .. } | Stm { .. } => true,
        LdmiaW { rn, reg_list, .. } | LdmdbW { rn, reg_list, .. }
            if *rn != 15 && (*reg_list & (1 << 15)) == 0 =>
        {
            true
        }
        StmiaW { rn, .. } | StmdbW { rn, .. } if *rn != 15 => true,
        Vldr { rn, .. } | Vstr { rn, .. } if *rn != 15 => true,
        _ => false,
    }
}

pub fn is_terminator_emittable(inst: &Instruction) -> bool {
    use Instruction::*;
    matches!(
        inst,
        Branch { .. }
            | BranchCond { .. }
            | Cbz { .. }
            | Cbnz { .. }
            | Bl { .. }
            | Bx { .. }
            | BlxReg { .. }
            | MovReg { rd: 15, .. }
            | Pop { p: true, .. }
            | LdrImm32 { rt: 15, .. }
            | LdrImm32Idx { rt: 15, .. }
    ) || matches!(
        inst,
        LdmiaW { rn, reg_list, .. } | LdmdbW { rn, reg_list, .. }
            if *rn != 15 && (*reg_list & (1 << 15)) != 0
    )
}

fn is_emittable(inst: &Instruction, mem_ok: bool) -> bool {
    is_alu_emittable(inst) || (mem_ok && is_mem_emittable(inst))
}

fn dataproc_op_emittable(op: u8) -> bool {
    matches!(
        op,
        0x0 | 0x1 | 0x2 | 0x3 | 0x4 | 0x8 | 0xA | 0xB | 0xD | 0xE
    )
}

/// ARM ThumbExpandImm — same as `cortex_m::thumb_expand_imm`.
fn thumb_expand_imm(imm12: u32) -> u32 {
    let i = (imm12 >> 11) & 1;
    let imm3 = (imm12 >> 8) & 7;
    let imm8 = imm12 & 0xFF;
    if i == 0 && (imm3 >> 2) == 0 {
        match imm3 {
            0 => imm8,
            1 => (imm8 << 16) | imm8,
            2 => (imm8 << 24) | (imm8 << 8),
            3 => (imm8 << 24) | (imm8 << 16) | (imm8 << 8) | imm8,
            _ => unreachable!(),
        }
    } else {
        let val = 0x80 | (imm8 & 0x7F);
        let n = (i << 4) | (imm3 << 1) | (imm8 >> 7);
        val.rotate_right(n)
    }
}

struct Op {
    pc: u32,
    inst: Instruction,
}

pub struct EmittedBlock {
    pub code: Vec<u8>,
    pub end_pc: Pc,
    pub instr_count: u32,
    pub exits: Vec<ExitEdge>,
    pub binding: Option<MemBinding>,
}

pub fn emit_block(pc: Pc, code: &CodeView<'_>, window: Option<RamWindow>) -> Option<EmittedBlock> {
    let ops = walk_ops(pc, code, window.is_some());
    let prefix_end = pc + ops.iter().map(|o| inst_len_of(o.pc, code)).sum::<u64>();
    let terminator = decode_at(prefix_end, code).filter(|(inst, _)| is_terminator_emittable(inst));
    if ops.is_empty() && terminator.is_none() {
        return None;
    }

    let mut body = Body {
        window,
        ..Body::default()
    };
    for aop in &ops {
        body.emit_instruction(aop.pc, &aop.inst, code);
    }

    let (end_pc, instr_count, wire) = if let Some((tinst, tlen)) = terminator {
        body.emit_terminator(prefix_end as u32, tlen as u32, &tinst);
        (prefix_end + tlen, ops.len() as u32 + 1, WIRE_CHAIN_DYNAMIC)
    } else {
        (prefix_end, ops.len() as u32, WIRE_FALL_THROUGH)
    };

    let mut expr = Vec::with_capacity(body.buf.len() + 256);
    body.emit_prologue(&mut expr);
    expr.extend_from_slice(&body.buf);
    body.emit_epilogue(&mut expr);
    expr.push(op::I32_CONST);
    enc::sleb(&mut expr, wire as i64);

    let binding = if body.has_mem || body.has_vfp {
        let len = window.map(|(_, l)| l as usize).unwrap_or(0);
        Some(MemBinding {
            ram_len: len,
            has_store: body.has_store,
        })
    } else {
        None
    };
    let code_bytes = if body.has_mem || body.has_vfp {
        build_module_ram_host(LOCAL_COUNT, &expr)
    } else {
        build_module(LOCAL_COUNT, 1, &expr)
    };

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
    if body.has_unsupported {
        exits.push(ExitEdge {
            wire_code: WIRE_UNSUPPORTED,
            reason: BailReason::UnsupportedInstruction,
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

fn it_body_len(mask: u8) -> u32 {
    if mask == 0 {
        0
    } else {
        4 - mask.trailing_zeros()
    }
}

/// Why one instruction is refused a place in a compiled IT body. The body is
/// compiled only when every slot is predicable, because `Body.it_state` models
/// exactly one live IT block; an unnamed refusal here silently changes what
/// `ready_instr_count` reports, so every case is spelled out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ItBodyBail {
    /// A nested IT needs a second live IT state; the walker only tracks one.
    NestedIt,
    /// LDR literal may resolve to the code view, a windowed load, or a fault
    /// depending on the link address; kept on the interpreter (the one
    /// deliberate hole in IT mem coverage).
    LdrLit,
    /// Not ALU-emittable and not mem-emittable (control flow, unmodeled, or
    /// memory with no RAM window to bound it).
    Unsupported,
}

/// `None` when `inst` can join a compiled IT body, else the named refusal.
pub(super) fn it_body_bail(inst: &Instruction, mem_ok: bool) -> Option<ItBodyBail> {
    if matches!(inst, Instruction::It { .. }) {
        return Some(ItBodyBail::NestedIt);
    }
    if matches!(inst, Instruction::LdrLit { .. }) {
        return Some(ItBodyBail::LdrLit);
    }
    if is_alu_emittable(inst) || (mem_ok && is_mem_emittable(inst)) {
        None
    } else {
        Some(ItBodyBail::Unsupported)
    }
}

fn walk_ops(pc: Pc, code: &CodeView<'_>, mem_ok: bool) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut cur = pc;
    while let Some((inst, len)) = decode_at(cur, code) {
        if let Instruction::It { mask, .. } = inst {
            let n = it_body_len(mask);
            if n == 0 || n > 4 {
                break;
            }
            let mut peek_pc = cur + len;
            let mut body = Vec::new();
            let mut ok = true;
            for _ in 0..n {
                let Some((pi, pl)) = decode_at(peek_pc, code) else {
                    ok = false;
                    break;
                };
                if let Some(bail) = it_body_bail(&pi, mem_ok) {
                    tracing::debug!("IT body bail at {peek_pc:#x}: {bail:?}");
                    ok = false;
                    break;
                }
                body.push(Op {
                    pc: peek_pc as u32,
                    inst: pi,
                });
                peek_pc += pl;
            }
            if !ok || ops.len() as u32 + 1 + n > super::MAX_BLOCK_INSTRS {
                break;
            }
            ops.push(Op {
                pc: cur as u32,
                inst,
            });
            ops.extend(body);
            cur = peek_pc;
            continue;
        }
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

fn inst_len_of(pc: u32, code: &CodeView<'_>) -> u64 {
    decode_at(pc as Pc, code).map(|(_, l)| l).unwrap_or(2)
}

enum MemAccess {
    Load { rd: u8, opcode: u8 },
    Store { rs2: u8, opcode: u8 },
}

enum ShiftKind {
    Lsl,
    Lsr,
    Asr,
    Ror,
}

impl MemAccess {
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

    fn load_signed(&self) -> bool {
        matches!(
            self,
            MemAccess::Load {
                opcode: op::I32_LOAD8_S | op::I32_LOAD16_S,
                ..
            }
        )
    }
}

#[derive(Default)]
struct Body {
    buf: Vec<u8>,
    reads: [bool; 16],
    writes: [bool; 16],
    window: Option<RamWindow>,
    has_mem: bool,
    has_store: bool,
    has_vfp: bool,
    has_unsupported: bool,
    emitted: u32,
    it_state: u8,
}

impl Body {
    fn read(&mut self, r: u8, insn_pc: u32) {
        if r == 15 {
            self.i32_const(insn_pc.wrapping_add(4) as i32);
            return;
        }
        self.reads[r as usize] = true;
        self.buf.push(op::LOCAL_GET);
        enc::uleb(&mut self.buf, r as u64);
    }

    /// `read_reg(15)` is the raw insn PC, not PC+4. Used by high-register
    /// ADD/MOV; Adr, LdrLit, and branch offsets keep `read`'s PC+4.
    fn read_gpr_or_pc_raw(&mut self, r: u8, insn_pc: u32) {
        if r == 15 {
            self.i32_const(insn_pc as i32);
            return;
        }
        self.read(r, insn_pc);
    }

    fn write(&mut self, r: u8) {
        debug_assert!(r < 15);
        self.writes[r as usize] = true;
        self.buf.push(op::LOCAL_SET);
        enc::uleb(&mut self.buf, r as u64);
    }

    fn i32_const(&mut self, v: i32) {
        self.buf.push(op::I32_CONST);
        enc::sleb(&mut self.buf, v as i64);
    }

    fn local_get(&mut self, i: u32) {
        self.buf.push(op::LOCAL_GET);
        enc::uleb(&mut self.buf, i as u64);
    }

    fn local_set(&mut self, i: u32) {
        self.buf.push(op::LOCAL_SET);
        enc::uleb(&mut self.buf, i as u64);
    }

    fn local_tee(&mut self, i: u32) {
        self.buf.push(op::LOCAL_TEE);
        enc::uleb(&mut self.buf, i as u64);
    }

    fn xpsr_touch(&mut self) {
        self.reads[XPSR_LOCAL as usize] = true;
        self.writes[XPSR_LOCAL as usize] = true;
    }

    fn push_result(&mut self) {
        self.local_get(RESULT_LOCAL);
    }

    fn set_result_from_stack(&mut self) {
        self.local_tee(RESULT_LOCAL);
    }

    /// NZ only (C and V unchanged). Result is in RESULT.
    /// T1 encodings use `setflags = !InITBlock()`; TST/TEQ/CMP/CMN call
    /// [`Self::update_nz_unconditional`].
    fn update_nz(&mut self) {
        if self.it_state != 0 {
            return;
        }
        self.update_nz_unconditional();
    }

    fn update_nz_unconditional(&mut self) {
        self.xpsr_touch();
        self.local_get(XPSR_LOCAL);
        self.i32_const(0x3FFF_FFFF_u32 as i32);
        self.buf.push(op::I32_AND);
        self.push_result();
        self.i32_const(0x8000_0000_u32 as i32);
        self.buf.push(op::I32_AND);
        self.buf.push(op::I32_OR);
        self.push_result();
        self.buf.push(op::I32_EQZ);
        self.i32_const(30);
        self.buf.push(op::I32_SHL);
        self.buf.push(op::I32_OR);
        self.local_set(XPSR_LOCAL);
    }

    fn update_nzcv(&mut self, carry_then_overflow: bool) {
        // Expects [carry, overflow] on stack, overflow on top, if carry_then_overflow.
        // We emit a dedicated helper that takes carry and overflow already computed
        // into SCRATCH (overflow) with carry still on the stack.
        let _ = carry_then_overflow;
        if self.it_state != 0 {
            self.buf.push(op::DROP);
            self.buf.push(op::DROP);
            return;
        }
        self.xpsr_touch();
        // stack: c, v  (v on top)
        self.local_set(SCRATCH_LOCAL); // v
                                       // stack: c
        self.i32_const(29);
        self.buf.push(op::I32_SHL); // c<<29
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(28);
        self.buf.push(op::I32_SHL); // v<<28
        self.buf.push(op::I32_OR);
        self.push_result();
        self.i32_const(0x8000_0000_u32 as i32);
        self.buf.push(op::I32_AND); // N
        self.buf.push(op::I32_OR);
        self.push_result();
        self.buf.push(op::I32_EQZ);
        self.i32_const(30);
        self.buf.push(op::I32_SHL); // Z
        self.buf.push(op::I32_OR);
        self.local_get(XPSR_LOCAL);
        self.i32_const(0x0FFF_FFFF);
        self.buf.push(op::I32_AND);
        self.buf.push(op::I32_OR);
        self.local_set(XPSR_LOCAL);
    }

    fn add_with_flags(
        &mut self,
        rd: Option<u8>,
        insn_pc: u32,
        rn: u8,
        op2_imm: Option<i32>,
        rm: Option<u8>,
    ) {
        self.read(rn, insn_pc);
        self.local_set(SCRATCH_LOCAL); // op1
        match (op2_imm, rm) {
            (Some(imm), _) => self.i32_const(imm),
            (_, Some(r)) => self.read(r, insn_pc),
            _ => unreachable!(),
        }
        self.local_tee(OP2_LOCAL);
        self.local_get(SCRATCH_LOCAL);
        self.buf.push(op::I32_ADD);
        self.set_result_from_stack();
        if let Some(d) = rd {
            self.write(d);
        } else {
            self.buf.push(op::DROP);
        }
        if self.it_state != 0 && rd.is_some() {
            return;
        }
        // carry = res <u op1
        self.push_result();
        self.local_get(SCRATCH_LOCAL);
        self.buf.push(op::I32_LT_U);
        // overflow = (~(op1^op2) & (op1^res)) >> 31 — use OP2_LOCAL, never
        // re-read rm (rd==rm would observe the result).
        self.local_get(SCRATCH_LOCAL); // op1
        self.local_get(OP2_LOCAL);
        self.buf.push(op::I32_XOR);
        self.i32_const(-1);
        self.buf.push(op::I32_XOR);
        self.local_get(SCRATCH_LOCAL);
        self.push_result();
        self.buf.push(op::I32_XOR);
        self.buf.push(op::I32_AND);
        self.i32_const(31);
        self.buf.push(op::I32_SHR_U);
        self.update_nzcv(true);
    }

    fn sub_with_flags(
        &mut self,
        rd: Option<u8>,
        insn_pc: u32,
        rn: u8,
        op2_imm: Option<i32>,
        rm: Option<u8>,
    ) {
        self.read(rn, insn_pc);
        self.local_set(SCRATCH_LOCAL);
        self.local_get(SCRATCH_LOCAL);
        match (op2_imm, rm) {
            (Some(imm), _) => self.i32_const(imm),
            (_, Some(r)) => self.read(r, insn_pc),
            _ => unreachable!(),
        }
        self.local_tee(OP2_LOCAL);
        self.buf.push(op::I32_SUB);
        self.set_result_from_stack();
        if let Some(d) = rd {
            self.write(d);
        } else {
            self.buf.push(op::DROP);
        }
        if self.it_state != 0 && rd.is_some() {
            return;
        }
        // ARM C = NOT borrow. borrow iff res >u op1. C = res <=u op1.
        self.push_result();
        self.local_get(SCRATCH_LOCAL);
        self.buf.push(op::I32_LE_U);
        // overflow = (op1^op2) & (op1^res) >> 31
        self.local_get(SCRATCH_LOCAL);
        self.local_get(OP2_LOCAL);
        self.buf.push(op::I32_XOR);
        self.local_get(SCRATCH_LOCAL);
        self.push_result();
        self.buf.push(op::I32_XOR);
        self.buf.push(op::I32_AND);
        self.i32_const(31);
        self.buf.push(op::I32_SHR_U);
        self.update_nzcv(true);
    }

    fn push_old_carry(&mut self) {
        self.local_get(XPSR_LOCAL);
        self.i32_const(29);
        self.buf.push(op::I32_SHR_U);
        self.i32_const(1);
        self.buf.push(op::I32_AND);
    }

    fn push_old_overflow(&mut self) {
        self.local_get(XPSR_LOCAL);
        self.i32_const(28);
        self.buf.push(op::I32_SHR_U);
        self.i32_const(1);
        self.buf.push(op::I32_AND);
    }

    fn emit_adc(&mut self, pc: u32, rd: u8, rm: u8) {
        self.xpsr_touch();
        self.read(rd, pc);
        self.local_set(SCRATCH_LOCAL);
        self.read(rm, pc);
        self.local_set(OP2_LOCAL);
        self.push_old_carry();
        self.local_get(SCRATCH_LOCAL);
        self.local_get(OP2_LOCAL);
        self.buf.push(op::I32_ADD);
        self.buf.push(op::I32_ADD);
        self.set_result_from_stack();
        self.write(rd);
        // C = (op1+op2 <u op1) | (res <u op1+op2)
        self.local_get(SCRATCH_LOCAL);
        self.local_get(OP2_LOCAL);
        self.buf.push(op::I32_ADD);
        self.local_get(SCRATCH_LOCAL);
        self.buf.push(op::I32_LT_U);
        self.push_result();
        self.local_get(SCRATCH_LOCAL);
        self.local_get(OP2_LOCAL);
        self.buf.push(op::I32_ADD);
        self.buf.push(op::I32_LT_U);
        self.buf.push(op::I32_OR);
        // V = (~(op1^op2) & (op1^res)) >> 31
        self.local_get(SCRATCH_LOCAL);
        self.local_get(OP2_LOCAL);
        self.buf.push(op::I32_XOR);
        self.i32_const(-1);
        self.buf.push(op::I32_XOR);
        self.local_get(SCRATCH_LOCAL);
        self.push_result();
        self.buf.push(op::I32_XOR);
        self.buf.push(op::I32_AND);
        self.i32_const(31);
        self.buf.push(op::I32_SHR_U);
        self.update_nzcv(true);
    }

    fn emit_sbc(&mut self, pc: u32, rd: u8, rm: u8) {
        self.xpsr_touch();
        self.read(rd, pc);
        self.local_set(SCRATCH_LOCAL);
        self.read(rm, pc);
        self.local_set(OP2_LOCAL);
        self.local_get(SCRATCH_LOCAL);
        self.local_get(OP2_LOCAL);
        self.buf.push(op::I32_SUB);
        self.push_old_carry();
        self.buf.push(op::I32_EQZ); // borrow_in = !C
        self.buf.push(op::I32_SUB);
        self.set_result_from_stack();
        self.write(rd);
        // C = !((tmp >u op1) | (res >u tmp)), tmp = op1 - op2
        self.local_get(SCRATCH_LOCAL);
        self.local_get(OP2_LOCAL);
        self.buf.push(op::I32_SUB);
        self.local_get(SCRATCH_LOCAL);
        self.buf.push(op::I32_GT_U);
        self.push_result();
        self.local_get(SCRATCH_LOCAL);
        self.local_get(OP2_LOCAL);
        self.buf.push(op::I32_SUB);
        self.buf.push(op::I32_GT_U);
        self.buf.push(op::I32_OR);
        self.buf.push(op::I32_EQZ);
        // V = (op1^op2) & (op1^res) >> 31
        self.local_get(SCRATCH_LOCAL);
        self.local_get(OP2_LOCAL);
        self.buf.push(op::I32_XOR);
        self.local_get(SCRATCH_LOCAL);
        self.push_result();
        self.buf.push(op::I32_XOR);
        self.buf.push(op::I32_AND);
        self.i32_const(31);
        self.buf.push(op::I32_SHR_U);
        self.update_nzcv(true);
    }

    fn emit_rsbs(&mut self, pc: u32, rd: u8, rn: u8) {
        self.i32_const(0);
        self.local_set(SCRATCH_LOCAL);
        self.local_get(SCRATCH_LOCAL);
        self.read(rn, pc);
        self.local_tee(OP2_LOCAL);
        self.buf.push(op::I32_SUB);
        self.set_result_from_stack();
        self.write(rd);
        self.push_result();
        self.local_get(SCRATCH_LOCAL);
        self.buf.push(op::I32_LE_U);
        self.local_get(SCRATCH_LOCAL);
        self.local_get(OP2_LOCAL);
        self.buf.push(op::I32_XOR);
        self.local_get(SCRATCH_LOCAL);
        self.push_result();
        self.buf.push(op::I32_XOR);
        self.buf.push(op::I32_AND);
        self.i32_const(31);
        self.buf.push(op::I32_SHR_U);
        self.update_nzcv(true);
    }

    fn maybe_write(&mut self, rd: u8) {
        if rd != 15 {
            self.write(rd);
        } else {
            self.buf.push(op::DROP);
        }
    }

    fn finish_dataproc_logical(&mut self, rd: u8, set_flags: bool) {
        self.set_result_from_stack();
        self.maybe_write(rd);
        if set_flags {
            self.xpsr_touch();
            self.push_old_carry();
            self.push_old_overflow();
            self.update_nzcv(true);
        }
    }

    fn emit_dataproc_add_flags(&mut self, rd: u8, set_flags: bool) {
        // op1 in SCRATCH, op2 in OP2, compute res = op1+op2
        self.local_get(SCRATCH_LOCAL);
        self.local_get(OP2_LOCAL);
        self.buf.push(op::I32_ADD);
        self.set_result_from_stack();
        self.maybe_write(rd);
        if !set_flags {
            return;
        }
        self.push_result();
        self.local_get(SCRATCH_LOCAL);
        self.buf.push(op::I32_LT_U);
        self.local_get(SCRATCH_LOCAL);
        self.local_get(OP2_LOCAL);
        self.buf.push(op::I32_XOR);
        self.i32_const(-1);
        self.buf.push(op::I32_XOR);
        self.local_get(SCRATCH_LOCAL);
        self.push_result();
        self.buf.push(op::I32_XOR);
        self.buf.push(op::I32_AND);
        self.i32_const(31);
        self.buf.push(op::I32_SHR_U);
        self.update_nzcv(true);
    }

    fn emit_dataproc_sub_flags(&mut self, rd: u8, set_flags: bool, reverse: bool) {
        // op1 in SCRATCH, op2 in OP2. reverse => op2 - op1 (RSB).
        if reverse {
            self.local_get(OP2_LOCAL);
            self.local_get(SCRATCH_LOCAL);
        } else {
            self.local_get(SCRATCH_LOCAL);
            self.local_get(OP2_LOCAL);
        }
        self.buf.push(op::I32_SUB);
        self.set_result_from_stack();
        self.maybe_write(rd);
        if !set_flags {
            return;
        }
        let op1 = if reverse { OP2_LOCAL } else { SCRATCH_LOCAL };
        let op2 = if reverse { SCRATCH_LOCAL } else { OP2_LOCAL };
        self.push_result();
        self.local_get(op1);
        self.buf.push(op::I32_LE_U);
        self.local_get(op1);
        self.local_get(op2);
        self.buf.push(op::I32_XOR);
        self.local_get(op1);
        self.push_result();
        self.buf.push(op::I32_XOR);
        self.buf.push(op::I32_AND);
        self.i32_const(31);
        self.buf.push(op::I32_SHR_U);
        self.update_nzcv(true);
    }

    fn emit_dataproc_adc_sbc(&mut self, rd: u8, set_flags: bool, is_sbc: bool) {
        self.xpsr_touch();
        if is_sbc {
            self.local_get(SCRATCH_LOCAL);
            self.local_get(OP2_LOCAL);
            self.buf.push(op::I32_SUB);
            self.push_old_carry();
            self.buf.push(op::I32_EQZ);
            self.buf.push(op::I32_SUB);
        } else {
            self.push_old_carry();
            self.local_get(SCRATCH_LOCAL);
            self.local_get(OP2_LOCAL);
            self.buf.push(op::I32_ADD);
            self.buf.push(op::I32_ADD);
        }
        self.set_result_from_stack();
        self.maybe_write(rd);
        if !set_flags {
            return;
        }
        if is_sbc {
            self.local_get(SCRATCH_LOCAL);
            self.local_get(OP2_LOCAL);
            self.buf.push(op::I32_SUB);
            self.local_get(SCRATCH_LOCAL);
            self.buf.push(op::I32_GT_U);
            self.push_result();
            self.local_get(SCRATCH_LOCAL);
            self.local_get(OP2_LOCAL);
            self.buf.push(op::I32_SUB);
            self.buf.push(op::I32_GT_U);
            self.buf.push(op::I32_OR);
            self.buf.push(op::I32_EQZ);
            self.local_get(SCRATCH_LOCAL);
            self.local_get(OP2_LOCAL);
            self.buf.push(op::I32_XOR);
            self.local_get(SCRATCH_LOCAL);
            self.push_result();
            self.buf.push(op::I32_XOR);
            self.buf.push(op::I32_AND);
            self.i32_const(31);
            self.buf.push(op::I32_SHR_U);
        } else {
            self.local_get(SCRATCH_LOCAL);
            self.local_get(OP2_LOCAL);
            self.buf.push(op::I32_ADD);
            self.local_get(SCRATCH_LOCAL);
            self.buf.push(op::I32_LT_U);
            self.push_result();
            self.local_get(SCRATCH_LOCAL);
            self.local_get(OP2_LOCAL);
            self.buf.push(op::I32_ADD);
            self.buf.push(op::I32_LT_U);
            self.buf.push(op::I32_OR);
            self.local_get(SCRATCH_LOCAL);
            self.local_get(OP2_LOCAL);
            self.buf.push(op::I32_XOR);
            self.i32_const(-1);
            self.buf.push(op::I32_XOR);
            self.local_get(SCRATCH_LOCAL);
            self.push_result();
            self.buf.push(op::I32_XOR);
            self.buf.push(op::I32_AND);
            self.i32_const(31);
            self.buf.push(op::I32_SHR_U);
        }
        self.update_nzcv(true);
    }

    fn emit_dataproc_from_ops(&mut self, op: u8, rn: u8, rd: u8, set_flags: bool) {
        // op1 in SCRATCH, op2 in OP2.
        match op {
            0x0 => {
                self.local_get(SCRATCH_LOCAL);
                self.local_get(OP2_LOCAL);
                self.buf.push(op::I32_AND);
                self.finish_dataproc_logical(rd, set_flags);
            }
            0x1 => {
                self.local_get(SCRATCH_LOCAL);
                self.local_get(OP2_LOCAL);
                self.i32_const(-1);
                self.buf.push(op::I32_XOR);
                self.buf.push(op::I32_AND);
                self.finish_dataproc_logical(rd, set_flags);
            }
            0x2 => {
                if rn == 0xF {
                    self.local_get(OP2_LOCAL);
                } else {
                    self.local_get(SCRATCH_LOCAL);
                    self.local_get(OP2_LOCAL);
                    self.buf.push(op::I32_OR);
                }
                self.finish_dataproc_logical(rd, set_flags);
            }
            0x3 => {
                if rn == 0xF {
                    self.local_get(OP2_LOCAL);
                    self.i32_const(-1);
                    self.buf.push(op::I32_XOR);
                } else {
                    self.local_get(SCRATCH_LOCAL);
                    self.local_get(OP2_LOCAL);
                    self.i32_const(-1);
                    self.buf.push(op::I32_XOR);
                    self.buf.push(op::I32_OR);
                }
                self.finish_dataproc_logical(rd, set_flags);
            }
            0x4 => {
                self.local_get(SCRATCH_LOCAL);
                self.local_get(OP2_LOCAL);
                self.buf.push(op::I32_XOR);
                self.finish_dataproc_logical(rd, set_flags);
            }
            0x8 => self.emit_dataproc_add_flags(rd, set_flags),
            0xA => self.emit_dataproc_adc_sbc(rd, set_flags, false),
            0xB => self.emit_dataproc_adc_sbc(rd, set_flags, true),
            0xD => self.emit_dataproc_sub_flags(rd, set_flags, false),
            0xE => self.emit_dataproc_sub_flags(rd, set_flags, true),
            _ => unreachable!("non-emittable dataproc op {op:#x}"),
        }
    }

    fn emit_dataproc_imm(&mut self, pc: u32, op: u8, rn: u8, rd: u8, imm12: u32, set_flags: bool) {
        let imm = thumb_expand_imm(imm12) as i32;
        self.read_gpr_or_pc_raw(rn, pc);
        self.local_set(SCRATCH_LOCAL);
        self.i32_const(imm);
        self.local_set(OP2_LOCAL);
        self.emit_dataproc_from_ops(op, rn, rd, set_flags);
    }

    fn emit_shifted_rm(&mut self, pc: u32, rm: u8, imm5: u8, shift_type: u8) {
        self.read_gpr_or_pc_raw(rm, pc);
        match shift_type {
            0 if imm5 != 0 => {
                self.i32_const(imm5 as i32);
                self.buf.push(op::I32_SHL);
            }
            1 if imm5 == 0 => {
                self.buf.push(op::DROP);
                self.i32_const(0);
            }
            1 => {
                self.i32_const(imm5 as i32);
                self.buf.push(op::I32_SHR_U);
            }
            2 if imm5 == 0 => {
                self.i32_const(31);
                self.buf.push(op::I32_SHR_S);
            }
            2 => {
                self.i32_const(imm5 as i32);
                self.buf.push(op::I32_SHR_S);
            }
            3 if imm5 != 0 => {
                self.i32_const(imm5 as i32);
                self.buf.push(op::I32_ROTR);
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_dataproc_reg(
        &mut self,
        pc: u32,
        op: u8,
        rn: u8,
        rd: u8,
        rm: u8,
        imm5: u8,
        shift_type: u8,
        set_flags: bool,
    ) {
        self.read_gpr_or_pc_raw(rn, pc);
        self.local_set(SCRATCH_LOCAL);
        self.emit_shifted_rm(pc, rm, imm5, shift_type);
        self.local_set(OP2_LOCAL);
        self.emit_dataproc_from_ops(op, rn, rd, set_flags);
    }

    fn emit_shift_reg(&mut self, pc: u32, rd: u8, rm: u8, kind: ShiftKind) {
        self.xpsr_touch();
        self.read(rd, pc);
        self.local_set(OP2_LOCAL);
        self.read(rm, pc);
        self.i32_const(0xFF);
        self.buf.push(op::I32_AND);
        self.local_set(SCRATCH_LOCAL);

        self.local_get(SCRATCH_LOCAL);
        self.buf.push(op::I32_EQZ);
        self.buf.push(op::IF);
        self.buf.push(op::T_I32);
        self.local_get(OP2_LOCAL);
        self.buf.push(op::ELSE);
        match kind {
            ShiftKind::Lsl => {
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(32);
                self.buf.push(op::I32_LT_U);
                self.buf.push(op::IF);
                self.buf.push(op::T_I32);
                self.local_get(OP2_LOCAL);
                self.local_get(SCRATCH_LOCAL);
                self.buf.push(op::I32_SHL);
                self.buf.push(op::ELSE);
                self.i32_const(0);
                self.buf.push(op::END);
            }
            ShiftKind::Lsr => {
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(32);
                self.buf.push(op::I32_LT_U);
                self.buf.push(op::IF);
                self.buf.push(op::T_I32);
                self.local_get(OP2_LOCAL);
                self.local_get(SCRATCH_LOCAL);
                self.buf.push(op::I32_SHR_U);
                self.buf.push(op::ELSE);
                self.i32_const(0);
                self.buf.push(op::END);
            }
            ShiftKind::Asr => {
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(32);
                self.buf.push(op::I32_LT_U);
                self.buf.push(op::IF);
                self.buf.push(op::T_I32);
                self.local_get(OP2_LOCAL);
                self.local_get(SCRATCH_LOCAL);
                self.buf.push(op::I32_SHR_S);
                self.buf.push(op::ELSE);
                self.local_get(OP2_LOCAL);
                self.i32_const(31);
                self.buf.push(op::I32_SHR_S);
                self.buf.push(op::END);
            }
            ShiftKind::Ror => {
                self.local_get(OP2_LOCAL);
                self.local_get(SCRATCH_LOCAL);
                self.buf.push(op::I32_ROTR);
            }
        }
        self.buf.push(op::END);
        self.set_result_from_stack();
        self.write(rd);

        self.local_get(SCRATCH_LOCAL);
        self.buf.push(op::I32_EQZ);
        self.buf.push(op::IF);
        self.buf.push(op::T_I32);
        self.push_old_carry();
        self.buf.push(op::ELSE);
        match kind {
            ShiftKind::Lsl => {
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(32);
                self.buf.push(op::I32_LT_U);
                self.buf.push(op::IF);
                self.buf.push(op::T_I32);
                self.local_get(OP2_LOCAL);
                self.i32_const(32);
                self.local_get(SCRATCH_LOCAL);
                self.buf.push(op::I32_SUB);
                self.buf.push(op::I32_SHR_U);
                self.i32_const(1);
                self.buf.push(op::I32_AND);
                self.buf.push(op::ELSE);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(32);
                self.buf.push(op::I32_EQ);
                self.buf.push(op::IF);
                self.buf.push(op::T_I32);
                self.local_get(OP2_LOCAL);
                self.i32_const(1);
                self.buf.push(op::I32_AND);
                self.buf.push(op::ELSE);
                self.i32_const(0);
                self.buf.push(op::END);
                self.buf.push(op::END);
            }
            ShiftKind::Lsr => {
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(32);
                self.buf.push(op::I32_LT_U);
                self.buf.push(op::IF);
                self.buf.push(op::T_I32);
                self.local_get(OP2_LOCAL);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(1);
                self.buf.push(op::I32_SUB);
                self.buf.push(op::I32_SHR_U);
                self.i32_const(1);
                self.buf.push(op::I32_AND);
                self.buf.push(op::ELSE);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(32);
                self.buf.push(op::I32_EQ);
                self.buf.push(op::IF);
                self.buf.push(op::T_I32);
                self.local_get(OP2_LOCAL);
                self.i32_const(31);
                self.buf.push(op::I32_SHR_U);
                self.i32_const(1);
                self.buf.push(op::I32_AND);
                self.buf.push(op::ELSE);
                self.i32_const(0);
                self.buf.push(op::END);
                self.buf.push(op::END);
            }
            ShiftKind::Asr => {
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(32);
                self.buf.push(op::I32_LT_U);
                self.buf.push(op::IF);
                self.buf.push(op::T_I32);
                self.local_get(OP2_LOCAL);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(1);
                self.buf.push(op::I32_SUB);
                self.buf.push(op::I32_SHR_U);
                self.i32_const(1);
                self.buf.push(op::I32_AND);
                self.buf.push(op::ELSE);
                self.local_get(OP2_LOCAL);
                self.i32_const(31);
                self.buf.push(op::I32_SHR_U);
                self.i32_const(1);
                self.buf.push(op::I32_AND);
                self.buf.push(op::END);
            }
            ShiftKind::Ror => {
                self.push_result();
                self.i32_const(31);
                self.buf.push(op::I32_SHR_U);
            }
        }
        self.buf.push(op::END);
        self.push_old_overflow();
        self.update_nzcv(true);
    }

    fn logic_nz(&mut self, rd: u8, insn_pc: u32, rm: u8, opcode: u8, not_rm: bool) {
        self.read(rd, insn_pc);
        self.read(rm, insn_pc);
        if not_rm {
            self.i32_const(-1);
            self.buf.push(op::I32_XOR);
        }
        self.buf.push(opcode);
        self.set_result_from_stack();
        self.write(rd);
        self.update_nz();
    }

    fn store_const_at(&mut self, addr: u32, val: i32) {
        self.i32_const(addr as i32);
        self.i32_const(val);
        self.buf.push(op::I32_STORE);
        enc::uleb(&mut self.buf, 2);
        enc::uleb(&mut self.buf, 0);
    }

    fn emit_fault(&mut self, pc: u32, writes_before: &[bool; 16]) {
        for r in 0..16u8 {
            if writes_before[r as usize] {
                self.i32_const((r as i32) * 4);
                self.local_get(r as u32);
                self.buf.push(op::I32_STORE);
                enc::uleb(&mut self.buf, 2);
                enc::uleb(&mut self.buf, 0);
            }
        }
        self.store_const_at(FAULT_PC_SLOT, pc as i32);
        self.store_const_at(FAULT_RETIRED_SLOT, self.emitted as i32);
        self.store_const_at(IT_STATE_SLOT, self.it_state as i32);
        self.i32_const(WIRE_MEM_FAULT);
        self.buf.push(op::RETURN);
    }

    fn emit_unsupported(&mut self, pc: u32, writes_before: &[bool; 16]) {
        self.has_unsupported = true;
        for r in 0..16u8 {
            if writes_before[r as usize] {
                self.i32_const((r as i32) * 4);
                self.local_get(r as u32);
                self.buf.push(op::I32_STORE);
                enc::uleb(&mut self.buf, 2);
                enc::uleb(&mut self.buf, 0);
            }
        }
        self.store_const_at(FAULT_PC_SLOT, pc as i32);
        self.store_const_at(FAULT_RETIRED_SLOT, self.emitted as i32);
        self.store_const_at(IT_STATE_SLOT, self.it_state as i32);
        self.i32_const(WIRE_UNSUPPORTED);
        self.buf.push(op::RETURN);
    }

    fn ram_offset_from_scratch(&mut self) {
        let ram_base = self.window.expect("ram_offset without window").0;
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(ram_base as i32);
        self.buf.push(op::I32_SUB);
    }

    fn emit_host_load(&mut self, access: &MemAccess) {
        self.ram_offset_from_scratch();
        self.i32_const(access.width() as i32);
        self.i32_const(i32::from(access.load_signed()));
        self.buf.push(op::CALL);
        enc::uleb(&mut self.buf, 0);
    }

    fn emit_host_store(&mut self, pc: u32, rs2: u8, width: u32, raw_pc: bool) {
        self.ram_offset_from_scratch();
        if raw_pc {
            self.read_gpr_or_pc_raw(rs2, pc);
        } else {
            self.read(rs2, pc);
        }
        self.i32_const(width as i32);
        self.buf.push(op::CALL);
        enc::uleb(&mut self.buf, 1);
        self.store_const_at(RES_FLAG_SLOT, 1);
        self.has_store = true;
    }

    fn emit_vfp_mem(&mut self, pc: u32, sd: u8, rn: u8, imm: i32, add: bool, is_load: bool) {
        let (ram_base, ram_len) = self.window.expect("emit_vfp_mem without window");
        let ram_end = ram_base.wrapping_add(ram_len);
        let hi = ram_end.wrapping_sub(4);
        self.has_mem = true;
        self.has_vfp = true;
        self.read(rn, pc);
        self.i32_const(imm);
        self.buf.push(if add { op::I32_ADD } else { op::I32_SUB });
        self.local_set(SCRATCH_LOCAL);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(ram_base as i32);
        self.buf.push(op::I32_GE_U);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(hi as i32);
        self.buf.push(op::I32_LE_U);
        self.buf.push(op::I32_AND);
        self.buf.push(op::IF);
        self.buf.push(op::T_EMPTY);
        let writes_before = self.writes;
        if is_load {
            self.i32_const(sd as i32);
            self.emit_host_load(&MemAccess::Load {
                rd: 0,
                opcode: op::I32_LOAD,
            });
            self.buf.push(op::CALL);
            enc::uleb(&mut self.buf, 3);
        } else {
            self.ram_offset_from_scratch();
            self.i32_const(sd as i32);
            self.buf.push(op::CALL);
            enc::uleb(&mut self.buf, 2);
            self.i32_const(4);
            self.buf.push(op::CALL);
            enc::uleb(&mut self.buf, 1);
            self.store_const_at(RES_FLAG_SLOT, 1);
            self.has_store = true;
        }
        self.buf.push(op::ELSE);
        self.emit_fault(pc, &writes_before);
        self.buf.push(op::END);
    }

    fn emit_vfp_binop(&mut self, sd: u8, sn: u8, sm: u8, fop: i32) {
        self.has_vfp = true;
        self.i32_const(sd as i32);
        self.i32_const(fop);
        self.i32_const(sn as i32);
        self.buf.push(op::CALL);
        enc::uleb(&mut self.buf, 2);
        self.i32_const(sm as i32);
        self.buf.push(op::CALL);
        enc::uleb(&mut self.buf, 2);
        // `vfp.binop` evaluates the op host-side through the same
        // `cortex_m::vfp_binop` helper the interpreter uses, so FZ/DN and the
        // NaN canonicalization cannot drift between the two lanes.
        self.buf.push(op::CALL);
        enc::uleb(&mut self.buf, 4);
        self.buf.push(op::CALL);
        enc::uleb(&mut self.buf, 3);
    }

    fn emit_mem(&mut self, pc: u32, addr_reg: u8, imm: i32, access: MemAccess) {
        let (ram_base, ram_len) = self.window.expect("emit_mem without window");
        let ram_end = ram_base.wrapping_add(ram_len);
        let width = access.width();
        let hi = ram_end.wrapping_sub(width);
        self.has_mem = true;
        self.read(addr_reg, pc);
        self.i32_const(imm);
        self.buf.push(op::I32_ADD);
        self.local_set(SCRATCH_LOCAL);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(ram_base as i32);
        self.buf.push(op::I32_GE_U);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(hi as i32);
        self.buf.push(op::I32_LE_U);
        self.buf.push(op::I32_AND);
        self.buf.push(op::IF);
        self.buf.push(op::T_EMPTY);
        let writes_before = self.writes;
        match access {
            MemAccess::Load { rd, .. } => {
                self.emit_host_load(&access);
                self.write(rd);
            }
            MemAccess::Store { rs2, .. } => {
                self.emit_host_store(pc, rs2, access.width(), false);
            }
        }
        self.buf.push(op::ELSE);
        self.emit_fault(pc, &writes_before);
        self.buf.push(op::END);
    }

    fn emit_mem_abs(&mut self, pc: u32, addr: u32, access: MemAccess) {
        let (ram_base, ram_len) = match self.window {
            Some(w) => w,
            None => {
                let writes_before = self.writes;
                self.emit_fault(pc, &writes_before);
                return;
            }
        };
        let ram_end = ram_base.wrapping_add(ram_len);
        let width = access.width();
        let in_window = addr >= ram_base && addr.wrapping_add(width) <= ram_end;
        if !in_window {
            let writes_before = self.writes;
            self.has_mem = true;
            self.emit_fault(pc, &writes_before);
            return;
        }
        self.has_mem = true;
        self.i32_const(addr as i32);
        self.local_set(SCRATCH_LOCAL);
        match access {
            MemAccess::Load { rd, .. } => {
                self.emit_host_load(&access);
                self.write(rd);
            }
            MemAccess::Store { rs2, .. } => {
                self.emit_host_store(pc, rs2, access.width(), false);
            }
        }
    }

    fn begin_pred(&mut self) -> bool {
        if self.it_state == 0 {
            return false;
        }
        self.push_condition(self.it_state >> 4);
        self.buf.push(op::IF);
        self.buf.push(op::T_EMPTY);
        true
    }

    fn end_pred(&mut self, pred: bool) {
        if !pred {
            return;
        }
        self.buf.push(op::END);
        self.it_state = (self.it_state & 0xE0) | (((self.it_state & 0x1F) << 1) & 0x1F);
        if self.it_state & 0x0F == 0 {
            self.it_state = 0;
        }
    }

    fn emit_instruction(&mut self, pc: u32, inst: &Instruction, code: &CodeView<'_>) {
        use Instruction::*;
        if let It { cond, mask } = *inst {
            self.it_state = (cond << 4) | mask;
            self.emitted += 1;
            return;
        }
        let pred = self.begin_pred();
        match *inst {
            Nop | Barrier => {}
            MovImm { rd, imm } => {
                self.i32_const(imm as i32);
                self.set_result_from_stack();
                self.write(rd);
                self.update_nz();
            }
            Movw { rd, imm } => {
                self.i32_const(imm as u32 as i32);
                self.write(rd);
            }
            Movt { rd, imm } => {
                self.read(rd, pc);
                self.i32_const(0x0000_FFFF);
                self.buf.push(op::I32_AND);
                self.i32_const(((imm as u32) << 16) as i32);
                self.buf.push(op::I32_OR);
                self.write(rd);
            }
            MovReg { rd, rm } if rd != 15 => {
                self.read_gpr_or_pc_raw(rm, pc);
                self.write(rd);
            }
            AddReg { rd, rn, rm } => self.add_with_flags(Some(rd), pc, rn, None, Some(rm)),
            AddImm3 { rd, rn, imm } => {
                self.add_with_flags(Some(rd), pc, rn, Some(imm as i32), None)
            }
            AddImm8 { rd, imm } => self.add_with_flags(Some(rd), pc, rd, Some(imm as i32), None),
            AddwImm { rd, rn, imm } => {
                self.read(rn, pc);
                self.i32_const(imm as i32);
                self.buf.push(op::I32_ADD);
                self.write(rd);
            }
            AddSp { imm } => {
                self.read(13, pc);
                self.i32_const(imm as i32);
                self.buf.push(op::I32_ADD);
                self.write(13);
            }
            AddSpReg { rd, imm } => {
                self.read(13, pc);
                self.i32_const(imm as i32);
                self.buf.push(op::I32_ADD);
                self.write(rd);
            }
            AddRegHigh { rd, rm } if rd != 15 => {
                self.read_gpr_or_pc_raw(rd, pc);
                self.read_gpr_or_pc_raw(rm, pc);
                self.buf.push(op::I32_ADD);
                self.write(rd);
            }
            SubReg { rd, rn, rm } => self.sub_with_flags(Some(rd), pc, rn, None, Some(rm)),
            SubImm3 { rd, rn, imm } => {
                self.sub_with_flags(Some(rd), pc, rn, Some(imm as i32), None)
            }
            SubImm8 { rd, imm } => self.sub_with_flags(Some(rd), pc, rd, Some(imm as i32), None),
            SubwImm { rd, rn, imm } => {
                self.read(rn, pc);
                self.i32_const(imm as i32);
                self.buf.push(op::I32_SUB);
                self.write(rd);
            }
            SubSp { imm } => {
                self.read(13, pc);
                self.i32_const(imm as i32);
                self.buf.push(op::I32_SUB);
                self.write(13);
            }
            CmpImm { rn, imm } => self.sub_with_flags(None, pc, rn, Some(imm as i32), None),
            CmpReg { rn, rm } => self.sub_with_flags(None, pc, rn, None, Some(rm)),
            Cmn { rn, rm } => self.add_with_flags(None, pc, rn, None, Some(rm)),
            Tst { rn, rm } => {
                self.read(rn, pc);
                self.read(rm, pc);
                self.buf.push(op::I32_AND);
                self.set_result_from_stack();
                self.buf.push(op::DROP);
                self.update_nz_unconditional();
            }
            And { rd, rm } => self.logic_nz(rd, pc, rm, op::I32_AND, false),
            Orr { rd, rm } => self.logic_nz(rd, pc, rm, op::I32_OR, false),
            Eor { rd, rm } => self.logic_nz(rd, pc, rm, op::I32_XOR, false),
            Bic { rd, rm } => self.logic_nz(rd, pc, rm, op::I32_AND, true),
            Mvn { rd, rm } => {
                self.read(rm, pc);
                self.i32_const(-1);
                self.buf.push(op::I32_XOR);
                self.set_result_from_stack();
                self.write(rd);
                self.update_nz();
            }
            Lsl { rd, rm, imm } => {
                self.read(rm, pc);
                self.local_set(SCRATCH_LOCAL);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(imm as i32);
                self.buf.push(op::I32_SHL);
                self.set_result_from_stack();
                self.write(rd);
                if imm == 0 {
                    self.update_nz();
                } else {
                    self.local_get(SCRATCH_LOCAL);
                    self.i32_const(32 - imm as i32);
                    self.buf.push(op::I32_SHR_U);
                    self.i32_const(1);
                    self.buf.push(op::I32_AND);
                    self.xpsr_touch();
                    self.local_get(XPSR_LOCAL);
                    self.i32_const(1 << 28);
                    self.buf.push(op::I32_AND);
                    self.i32_const(28);
                    self.buf.push(op::I32_SHR_U); // old V as 0/1
                    self.update_nzcv(true);
                }
            }
            Lsr { rd, rm, imm } => {
                let n = if imm == 0 { 32 } else { imm as u32 };
                self.read(rm, pc);
                self.local_set(SCRATCH_LOCAL);
                if n >= 32 {
                    self.i32_const(0);
                } else {
                    self.local_get(SCRATCH_LOCAL);
                    self.i32_const(n as i32);
                    self.buf.push(op::I32_SHR_U);
                }
                self.set_result_from_stack();
                self.write(rd);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const((n - 1) as i32);
                self.buf.push(op::I32_SHR_U);
                self.i32_const(1);
                self.buf.push(op::I32_AND);
                self.xpsr_touch();
                self.local_get(XPSR_LOCAL);
                self.i32_const(1 << 28);
                self.buf.push(op::I32_AND);
                self.i32_const(28);
                self.buf.push(op::I32_SHR_U);
                self.update_nzcv(true);
            }
            Asr { rd, rm, imm } => {
                let n = if imm == 0 { 32 } else { imm as u32 };
                self.read(rm, pc);
                self.local_set(SCRATCH_LOCAL);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(n.min(31) as i32);
                self.buf.push(op::I32_SHR_S);
                self.set_result_from_stack();
                self.write(rd);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const((n - 1) as i32);
                self.buf.push(op::I32_SHR_U);
                self.i32_const(1);
                self.buf.push(op::I32_AND);
                self.xpsr_touch();
                self.local_get(XPSR_LOCAL);
                self.i32_const(1 << 28);
                self.buf.push(op::I32_AND);
                self.i32_const(28);
                self.buf.push(op::I32_SHR_U);
                self.update_nzcv(true);
            }
            Mul { rd, rn } => {
                self.read(rd, pc);
                self.read(rn, pc);
                self.buf.push(op::I32_MUL);
                self.set_result_from_stack();
                self.write(rd);
                self.update_nz();
            }
            Mul32 { rd, rn, rm } => {
                self.read(rn, pc);
                self.read(rm, pc);
                self.buf.push(op::I32_MUL);
                self.write(rd);
            }
            Adc { rd, rm } => self.emit_adc(pc, rd, rm),
            Sbc { rd, rm } => self.emit_sbc(pc, rd, rm),
            Rsbs { rd, rn } => self.emit_rsbs(pc, rd, rn),
            DataProcImm32 {
                op,
                rn,
                rd,
                imm12,
                set_flags,
            } => self.emit_dataproc_imm(pc, op, rn, rd, imm12, set_flags),
            DataProc32 {
                op,
                rn,
                rd,
                rm,
                imm5,
                shift_type,
                set_flags,
            } => self.emit_dataproc_reg(pc, op, rn, rd, rm, imm5, shift_type, set_flags),
            LslReg { rd, rm } => self.emit_shift_reg(pc, rd, rm, ShiftKind::Lsl),
            LsrReg { rd, rm } => self.emit_shift_reg(pc, rd, rm, ShiftKind::Lsr),
            AsrReg { rd, rm } => self.emit_shift_reg(pc, rd, rm, ShiftKind::Asr),
            Ror { rd, rm } => self.emit_shift_reg(pc, rd, rm, ShiftKind::Ror),
            Uxtb { rd, rm } => {
                self.read(rm, pc);
                self.i32_const(0xFF);
                self.buf.push(op::I32_AND);
                self.write(rd);
            }
            Uxth { rd, rm } => {
                self.read(rm, pc);
                self.i32_const(0xFFFF);
                self.buf.push(op::I32_AND);
                self.write(rd);
            }
            Sxtb { rd, rm } => {
                self.read(rm, pc);
                self.i32_const(24);
                self.buf.push(op::I32_SHL);
                self.i32_const(24);
                self.buf.push(op::I32_SHR_S);
                self.write(rd);
            }
            Sxth { rd, rm } => {
                self.read(rm, pc);
                self.i32_const(16);
                self.buf.push(op::I32_SHL);
                self.i32_const(16);
                self.buf.push(op::I32_SHR_S);
                self.write(rd);
            }
            Clz { rd, rm } => {
                self.read(rm, pc);
                self.buf.push(op::I32_CLZ);
                self.write(rd);
            }
            Rev { rd, rm } => {
                // bswap32 via shifts
                self.read(rm, pc);
                self.local_set(SCRATCH_LOCAL);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(24);
                self.buf.push(op::I32_SHL);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(8);
                self.buf.push(op::I32_SHL);
                self.i32_const(0x00FF_0000);
                self.buf.push(op::I32_AND);
                self.buf.push(op::I32_OR);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(8);
                self.buf.push(op::I32_SHR_U);
                self.i32_const(0x0000_FF00);
                self.buf.push(op::I32_AND);
                self.buf.push(op::I32_OR);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(24);
                self.buf.push(op::I32_SHR_U);
                self.buf.push(op::I32_OR);
                self.write(rd);
            }
            Rev16 { rd, rm } => {
                self.read(rm, pc);
                self.local_set(SCRATCH_LOCAL);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(8);
                self.buf.push(op::I32_SHL);
                self.i32_const(0xFF00_FF00_u32 as i32);
                self.buf.push(op::I32_AND);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(8);
                self.buf.push(op::I32_SHR_U);
                self.i32_const(0x00FF_00FF);
                self.buf.push(op::I32_AND);
                self.buf.push(op::I32_OR);
                self.write(rd);
            }
            RevSh { rd, rm } => {
                self.read(rm, pc);
                self.i32_const(8);
                self.buf.push(op::I32_SHL);
                self.i32_const(16);
                self.buf.push(op::I32_SHR_S);
                self.write(rd);
            }
            Rbit { rd, rm } => {
                // 32-bit bit reverse: loop-unrolled nibble swap then bit swap.
                self.read(rm, pc);
                self.local_set(SCRATCH_LOCAL);
                // Use the classic parallel reverse.
                // x = ((x >> 1) & 0x55555555) | ((x & 0x55555555) << 1)
                self.bitrev_pass(0x5555_5555_u32 as i32, 1);
                self.bitrev_pass(0x3333_3333, 2);
                self.bitrev_pass(0x0F0F_0F0F, 4);
                self.bitrev_pass(0x00FF_00FF, 8);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(16);
                self.buf.push(op::I32_SHL);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(16);
                self.buf.push(op::I32_SHR_U);
                self.buf.push(op::I32_OR);
                self.write(rd);
            }
            Adr { rd, imm } => {
                let base = (pc & !3).wrapping_add(4).wrapping_add(imm as u32);
                self.i32_const(base as i32);
                self.write(rd);
            }
            LdrImm { rt, rn, imm } => self.emit_mem(
                pc,
                rn,
                imm as i32,
                MemAccess::Load {
                    rd: rt,
                    opcode: op::I32_LOAD,
                },
            ),
            StrImm { rt, rn, imm } => self.emit_mem(
                pc,
                rn,
                imm as i32,
                MemAccess::Store {
                    rs2: rt,
                    opcode: op::I32_STORE,
                },
            ),
            LdrbImm { rt, rn, imm } => self.emit_mem(
                pc,
                rn,
                imm as i32,
                MemAccess::Load {
                    rd: rt,
                    opcode: op::I32_LOAD8_U,
                },
            ),
            StrbImm { rt, rn, imm } => self.emit_mem(
                pc,
                rn,
                imm as i32,
                MemAccess::Store {
                    rs2: rt,
                    opcode: op::I32_STORE8,
                },
            ),
            LdrhImm { rt, rn, imm } => self.emit_mem(
                pc,
                rn,
                imm as i32,
                MemAccess::Load {
                    rd: rt,
                    opcode: op::I32_LOAD16_U,
                },
            ),
            StrhImm { rt, rn, imm } => self.emit_mem(
                pc,
                rn,
                imm as i32,
                MemAccess::Store {
                    rs2: rt,
                    opcode: op::I32_STORE16,
                },
            ),
            LdrReg { rt, rn, rm } => self.emit_mem_reg(
                pc,
                rn,
                rm,
                MemAccess::Load {
                    rd: rt,
                    opcode: op::I32_LOAD,
                },
            ),
            StrReg { rt, rn, rm } => self.emit_mem_reg(
                pc,
                rn,
                rm,
                MemAccess::Store {
                    rs2: rt,
                    opcode: op::I32_STORE,
                },
            ),
            LdrbReg { rt, rn, rm } => self.emit_mem_reg(
                pc,
                rn,
                rm,
                MemAccess::Load {
                    rd: rt,
                    opcode: op::I32_LOAD8_U,
                },
            ),
            StrbReg { rt, rn, rm } => self.emit_mem_reg(
                pc,
                rn,
                rm,
                MemAccess::Store {
                    rs2: rt,
                    opcode: op::I32_STORE8,
                },
            ),
            LdrhReg { rt, rn, rm } => self.emit_mem_reg(
                pc,
                rn,
                rm,
                MemAccess::Load {
                    rd: rt,
                    opcode: op::I32_LOAD16_U,
                },
            ),
            StrhReg { rt, rn, rm } => self.emit_mem_reg(
                pc,
                rn,
                rm,
                MemAccess::Store {
                    rs2: rt,
                    opcode: op::I32_STORE16,
                },
            ),
            LdrsbReg { rt, rn, rm } => self.emit_mem_reg(
                pc,
                rn,
                rm,
                MemAccess::Load {
                    rd: rt,
                    opcode: op::I32_LOAD8_S,
                },
            ),
            LdrshReg { rt, rn, rm } => self.emit_mem_reg(
                pc,
                rn,
                rm,
                MemAccess::Load {
                    rd: rt,
                    opcode: op::I32_LOAD16_S,
                },
            ),
            LdrImm32 { rt, rn, imm12 } => self.emit_mem(
                pc,
                rn,
                imm12 as i32,
                MemAccess::Load {
                    rd: rt,
                    opcode: op::I32_LOAD,
                },
            ),
            StrImm32 { rt, rn, imm12 } => self.emit_mem(
                pc,
                rn,
                imm12 as i32,
                MemAccess::Store {
                    rs2: rt,
                    opcode: op::I32_STORE,
                },
            ),
            LdrImm32Idx {
                rt,
                rn,
                imm8,
                pre_index,
                add,
                writeback,
            } => self.emit_mem_idx(pc, rt, rn, imm8, pre_index, add, writeback, true),
            StrImm32Idx {
                rt,
                rn,
                imm8,
                pre_index,
                add,
                writeback,
            } => self.emit_mem_idx(pc, rt, rn, imm8, pre_index, add, writeback, false),
            LdrSp { rt, imm } => self.emit_mem(
                pc,
                13,
                imm as i32,
                MemAccess::Load {
                    rd: rt,
                    opcode: op::I32_LOAD,
                },
            ),
            StrSp { rt, imm } => self.emit_mem(
                pc,
                13,
                imm as i32,
                MemAccess::Store {
                    rs2: rt,
                    opcode: op::I32_STORE,
                },
            ),
            Vldr { sd, rn, imm, add } => self.emit_vfp_mem(pc, sd, rn, imm as i32, add, true),
            Vstr { sd, rn, imm, add } => self.emit_vfp_mem(pc, sd, rn, imm as i32, add, false),
            VaddF32 { sd, sn, sm } => self.emit_vfp_binop(sd, sn, sm, VFP_OP_ADD),
            VsubF32 { sd, sn, sm } => self.emit_vfp_binop(sd, sn, sm, VFP_OP_SUB),
            VmulF32 { sd, sn, sm } => self.emit_vfp_binop(sd, sn, sm, VFP_OP_MUL),
            VdivF32 { sd, sn, sm } => self.emit_vfp_binop(sd, sn, sm, VFP_OP_DIV),
            VmovF32Reg { sd, sm } => {
                self.has_vfp = true;
                self.i32_const(sd as i32);
                self.i32_const(sm as i32);
                self.buf.push(op::CALL);
                enc::uleb(&mut self.buf, 2);
                self.buf.push(op::CALL);
                enc::uleb(&mut self.buf, 3);
            }
            VmovF32Imm { sd, imm_bits } => {
                self.has_vfp = true;
                self.i32_const(sd as i32);
                self.i32_const(imm_bits as i32);
                self.buf.push(op::CALL);
                enc::uleb(&mut self.buf, 3);
            }
            VmovSnRt { sn, rt } => {
                self.has_vfp = true;
                self.i32_const(sn as i32);
                self.read(rt, pc);
                self.buf.push(op::CALL);
                enc::uleb(&mut self.buf, 3);
            }
            VmovRtSn { rt, sn } => {
                self.has_vfp = true;
                self.i32_const(sn as i32);
                self.buf.push(op::CALL);
                enc::uleb(&mut self.buf, 2);
                self.write(rt);
            }
            LdrLit { rt, imm } => {
                let addr = (pc & !3).wrapping_add(4).wrapping_add(imm as u32);
                if let Some(bytes) = code.from(addr as Pc) {
                    if bytes.len() >= 4 {
                        let val = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                        self.i32_const(val as i32);
                        self.write(rt);
                    } else {
                        self.emit_mem_abs(
                            pc,
                            addr,
                            MemAccess::Load {
                                rd: rt,
                                opcode: op::I32_LOAD,
                            },
                        );
                    }
                } else {
                    self.emit_mem_abs(
                        pc,
                        addr,
                        MemAccess::Load {
                            rd: rt,
                            opcode: op::I32_LOAD,
                        },
                    );
                }
            }
            Push { registers, m } => self.emit_push(pc, registers, m),
            Pop { registers, p } if !p => self.emit_ldm_stm(pc, 13, registers, true),
            Ldm { rn, registers } => self.emit_ldm_stm(pc, rn, registers, true),
            Stm { rn, registers } => self.emit_ldm_stm(pc, rn, registers, false),
            LdmiaW {
                rn,
                reg_list,
                writeback,
            } => self.emit_ldm_stm_wide(pc, rn, reg_list, writeback, true, false),
            LdmdbW {
                rn,
                reg_list,
                writeback,
            } => self.emit_ldm_stm_wide(pc, rn, reg_list, writeback, true, true),
            StmiaW {
                rn,
                reg_list,
                writeback,
            } => self.emit_ldm_stm_wide(pc, rn, reg_list, writeback, false, false),
            StmdbW {
                rn,
                reg_list,
                writeback,
            } => self.emit_ldm_stm_wide(pc, rn, reg_list, writeback, false, true),
            _ => unreachable!("non-emittable instruction reached emit: {inst:?}"),
        }
        self.end_pred(pred);
        self.emitted += 1;
    }

    fn bitrev_pass(&mut self, mask: i32, shift: i32) {
        // scratch = ((scratch >> shift) & mask) | ((scratch & mask) << shift)
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(shift);
        self.buf.push(op::I32_SHR_U);
        self.i32_const(mask);
        self.buf.push(op::I32_AND);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(mask);
        self.buf.push(op::I32_AND);
        self.i32_const(shift);
        self.buf.push(op::I32_SHL);
        self.buf.push(op::I32_OR);
        self.local_set(SCRATCH_LOCAL);
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_mem_idx(
        &mut self,
        pc: u32,
        rt: u8,
        rn: u8,
        imm8: u8,
        pre_index: bool,
        add: bool,
        writeback: bool,
        is_load: bool,
    ) {
        let (ram_base, ram_len) = self.window.expect("emit_mem_idx without window");
        let ram_end = ram_base.wrapping_add(ram_len);
        let hi = ram_end.wrapping_sub(4);
        self.has_mem = true;

        self.read(rn, pc);
        self.local_set(OP2_LOCAL);
        if pre_index {
            self.local_get(OP2_LOCAL);
            self.i32_const(imm8 as i32);
            if add {
                self.buf.push(op::I32_ADD);
            } else {
                self.buf.push(op::I32_SUB);
            }
        } else {
            self.local_get(OP2_LOCAL);
        }
        self.local_set(SCRATCH_LOCAL);

        self.local_get(SCRATCH_LOCAL);
        self.i32_const(ram_base as i32);
        self.buf.push(op::I32_GE_U);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(hi as i32);
        self.buf.push(op::I32_LE_U);
        self.buf.push(op::I32_AND);
        self.buf.push(op::IF);
        self.buf.push(op::T_EMPTY);
        let writes_before = self.writes;

        if is_load {
            self.emit_host_load(&MemAccess::Load {
                rd: rt,
                opcode: op::I32_LOAD,
            });
            self.local_set(RESULT_LOCAL);
            if writeback {
                self.local_get(OP2_LOCAL);
                self.i32_const(imm8 as i32);
                if add {
                    self.buf.push(op::I32_ADD);
                } else {
                    self.buf.push(op::I32_SUB);
                }
                self.write(rn);
            }
            self.local_get(RESULT_LOCAL);
            self.write(rt);
        } else {
            self.emit_host_store(pc, rt, 4, false);
            if writeback {
                self.local_get(OP2_LOCAL);
                self.i32_const(imm8 as i32);
                if add {
                    self.buf.push(op::I32_ADD);
                } else {
                    self.buf.push(op::I32_SUB);
                }
                self.write(rn);
            }
        }

        self.buf.push(op::ELSE);
        self.emit_fault(pc, &writes_before);
        self.buf.push(op::END);
    }

    fn emit_mem_reg(&mut self, pc: u32, rn: u8, rm: u8, access: MemAccess) {
        let (ram_base, ram_len) = self.window.expect("emit_mem_reg without window");
        let ram_end = ram_base.wrapping_add(ram_len);
        let width = access.width();
        let hi = ram_end.wrapping_sub(width);
        self.has_mem = true;
        self.read(rn, pc);
        self.read(rm, pc);
        self.buf.push(op::I32_ADD);
        self.local_set(SCRATCH_LOCAL);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(ram_base as i32);
        self.buf.push(op::I32_GE_U);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(hi as i32);
        self.buf.push(op::I32_LE_U);
        self.buf.push(op::I32_AND);
        self.buf.push(op::IF);
        self.buf.push(op::T_EMPTY);
        let writes_before = self.writes;
        match access {
            MemAccess::Load { rd, .. } => {
                self.emit_host_load(&access);
                self.write(rd);
            }
            MemAccess::Store { rs2, .. } => {
                self.emit_host_store(pc, rs2, access.width(), false);
            }
        }
        self.buf.push(op::ELSE);
        self.emit_fault(pc, &writes_before);
        self.buf.push(op::END);
    }

    fn emit_push_ea_in_window(&mut self, ram_base: u32, hi: u32) {
        // scratch = scratch - 4; result &= (scratch in [ram_base, hi])
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(4);
        self.buf.push(op::I32_SUB);
        self.local_tee(SCRATCH_LOCAL);
        self.i32_const(ram_base as i32);
        self.buf.push(op::I32_GE_U);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(hi as i32);
        self.buf.push(op::I32_LE_U);
        self.buf.push(op::I32_AND);
        self.local_get(RESULT_LOCAL);
        self.buf.push(op::I32_AND);
        self.local_set(RESULT_LOCAL);
    }

    fn emit_store_sp_off(&mut self, pc: u32, off: i32, rs2: u8) {
        self.read(13, pc);
        self.i32_const(off);
        self.buf.push(op::I32_ADD);
        self.local_set(SCRATCH_LOCAL);
        self.emit_host_store(pc, rs2, 4, false);
    }

    fn emit_push(&mut self, pc: u32, registers: u8, m: bool) {
        // Interpreter decrements a local SP and commits r13 only after every
        // store. Range-check the whole list first: any out-of-window EA
        // side-exits with SP unchanged and no stores, so the interpreter
        // re-executes the PUSH from the original SP.
        let mut count = u32::from(m);
        for i in 0..=7 {
            if (registers & (1 << i)) != 0 {
                count += 1;
            }
        }
        if count == 0 {
            return;
        }

        let (ram_base, ram_len) = self.window.expect("emit_push without window");
        let ram_end = ram_base.wrapping_add(ram_len);
        let hi = ram_end.wrapping_sub(4);
        self.has_mem = true;

        self.i32_const(1);
        self.local_set(RESULT_LOCAL);
        self.read(13, pc);
        self.local_set(SCRATCH_LOCAL);
        if m {
            self.emit_push_ea_in_window(ram_base, hi);
        }
        for i in (0..=7).rev() {
            if (registers & (1 << i)) != 0 {
                self.emit_push_ea_in_window(ram_base, hi);
            }
        }

        self.local_get(RESULT_LOCAL);
        self.buf.push(op::IF);
        self.buf.push(op::T_EMPTY);
        let writes_before = self.writes;

        let mut off: i32 = 0;
        if m {
            off -= 4;
            self.emit_store_sp_off(pc, off, 14);
        }
        for i in (0..=7).rev() {
            if (registers & (1 << i)) != 0 {
                off -= 4;
                self.emit_store_sp_off(pc, off, i);
            }
        }
        self.read(13, pc);
        self.i32_const((4 * count) as i32);
        self.buf.push(op::I32_SUB);
        self.write(13);

        self.buf.push(op::ELSE);
        self.emit_fault(pc, &writes_before);
        self.buf.push(op::END);
    }

    fn emit_word_at_base_off(&mut self, pc: u32, off: i32, access: MemAccess) {
        self.local_get(OP2_LOCAL);
        self.i32_const(off);
        self.buf.push(op::I32_ADD);
        self.local_set(SCRATCH_LOCAL);
        match access {
            MemAccess::Load { rd, .. } => {
                self.emit_host_load(&access);
                self.write(rd);
            }
            MemAccess::Store { rs2, .. } => {
                self.emit_host_store(pc, rs2, access.width(), true);
            }
        }
    }

    fn emit_ldm_stm(&mut self, pc: u32, rn: u8, registers: u8, is_load: bool) {
        let mut count = 0u32;
        for i in 0..=7 {
            if (registers & (1 << i)) != 0 {
                count += 1;
            }
        }
        if count == 0 {
            return;
        }
        let (ram_base, ram_len) = self.window.expect("emit_ldm_stm without window");
        let ram_end = ram_base.wrapping_add(ram_len);
        let hi = ram_end.wrapping_sub(4);
        self.has_mem = true;

        self.i32_const(1);
        self.local_set(RESULT_LOCAL);
        self.read(rn, pc);
        self.local_set(SCRATCH_LOCAL);
        for _ in 0..count {
            self.emit_ldm_ea_in_window(ram_base, hi);
        }

        self.local_get(RESULT_LOCAL);
        self.buf.push(op::IF);
        self.buf.push(op::T_EMPTY);
        let writes_before = self.writes;

        self.read(rn, pc);
        self.local_set(OP2_LOCAL);
        let mut off: i32 = 0;
        for i in 0..=7u8 {
            if (registers & (1 << i)) != 0 {
                if is_load {
                    self.emit_word_at_base_off(
                        pc,
                        off,
                        MemAccess::Load {
                            rd: i,
                            opcode: op::I32_LOAD,
                        },
                    );
                } else {
                    self.emit_word_at_base_off(
                        pc,
                        off,
                        MemAccess::Store {
                            rs2: i,
                            opcode: op::I32_STORE,
                        },
                    );
                }
                off += 4;
            }
        }
        // 16-bit LDM writeback is suppressed when Rn is in the list (loaded
        // value wins). STM always writebacks. POP is LDMIA SP! — Rn is 13,
        // which is never in the 8-bit r0–r7 list.
        let rn_in_list = rn < 8 && (registers & (1 << rn)) != 0;
        if !is_load || !rn_in_list {
            self.local_get(OP2_LOCAL);
            self.i32_const((4 * count) as i32);
            self.buf.push(op::I32_ADD);
            self.write(rn);
        }

        self.buf.push(op::ELSE);
        self.emit_fault(pc, &writes_before);
        self.buf.push(op::END);
    }

    fn emit_ldm_stm_wide(
        &mut self,
        pc: u32,
        rn: u8,
        reg_list: u16,
        writeback: bool,
        is_load: bool,
        decrement_before: bool,
    ) {
        let last = if is_load || !decrement_before { 14 } else { 15 };
        let mut count = 0u32;
        for i in 0..=last {
            if (reg_list & (1 << i)) != 0 {
                count += 1;
            }
        }
        if count == 0 {
            return;
        }
        let (ram_base, ram_len) = self.window.expect("emit_ldm_stm_wide without window");
        let ram_end = ram_base.wrapping_add(ram_len);
        let hi = ram_end.wrapping_sub(4);
        self.has_mem = true;

        self.i32_const(1);
        self.local_set(RESULT_LOCAL);
        self.read(rn, pc);
        if decrement_before {
            self.i32_const((4 * count) as i32);
            self.buf.push(op::I32_SUB);
        }
        self.local_set(SCRATCH_LOCAL);
        for _ in 0..count {
            self.emit_ldm_ea_in_window(ram_base, hi);
        }

        self.local_get(RESULT_LOCAL);
        self.buf.push(op::IF);
        self.buf.push(op::T_EMPTY);
        let writes_before = self.writes;

        self.read(rn, pc);
        if decrement_before {
            self.i32_const((4 * count) as i32);
            self.buf.push(op::I32_SUB);
        }
        self.local_set(OP2_LOCAL);
        let mut off: i32 = 0;
        for i in 0..=last {
            if (reg_list & (1 << i)) != 0 {
                if is_load {
                    self.emit_word_at_base_off(
                        pc,
                        off,
                        MemAccess::Load {
                            rd: i,
                            opcode: op::I32_LOAD,
                        },
                    );
                } else {
                    self.emit_word_at_base_off(
                        pc,
                        off,
                        MemAccess::Store {
                            rs2: i,
                            opcode: op::I32_STORE,
                        },
                    );
                }
                off += 4;
            }
        }
        if writeback {
            if decrement_before {
                self.local_get(OP2_LOCAL);
                self.write(rn);
            } else {
                self.local_get(OP2_LOCAL);
                self.i32_const((4 * count) as i32);
                self.buf.push(op::I32_ADD);
                self.write(rn);
            }
        }

        self.buf.push(op::ELSE);
        self.emit_fault(pc, &writes_before);
        self.buf.push(op::END);
    }

    fn emit_ldm_w_pc(
        &mut self,
        pc: u32,
        rn: u8,
        reg_list: u16,
        writeback: bool,
        decrement_before: bool,
    ) {
        let mut count = 1u32;
        for i in 0..=14 {
            if (reg_list & (1 << i)) != 0 {
                count += 1;
            }
        }
        let writes_before = self.writes;
        let Some((ram_base, ram_len)) = self.window else {
            self.emit_unsupported(pc, &writes_before);
            return;
        };
        let ram_end = ram_base.wrapping_add(ram_len);
        let hi = ram_end.wrapping_sub(4);
        self.has_mem = true;

        self.i32_const(1);
        self.local_set(RESULT_LOCAL);
        self.read(rn, pc);
        if decrement_before {
            self.i32_const((4 * count) as i32);
            self.buf.push(op::I32_SUB);
        }
        self.local_set(SCRATCH_LOCAL);
        for _ in 0..count {
            self.emit_ldm_ea_in_window(ram_base, hi);
        }

        self.local_get(RESULT_LOCAL);
        self.buf.push(op::IF);
        self.buf.push(op::T_EMPTY);

        self.read(rn, pc);
        if decrement_before {
            self.i32_const((4 * count) as i32);
            self.buf.push(op::I32_SUB);
        }
        self.local_set(OP2_LOCAL);
        let mut off: i32 = 0;
        for i in 0..=14u8 {
            if (reg_list & (1 << i)) != 0 {
                self.emit_word_at_base_off(
                    pc,
                    off,
                    MemAccess::Load {
                        rd: i,
                        opcode: op::I32_LOAD,
                    },
                );
                off += 4;
            }
        }
        self.local_get(OP2_LOCAL);
        self.i32_const(off);
        self.buf.push(op::I32_ADD);
        self.local_set(SCRATCH_LOCAL);
        self.emit_host_load(&MemAccess::Load {
            rd: 0,
            opcode: op::I32_LOAD,
        });
        self.local_set(SCRATCH_LOCAL);

        if writeback {
            if decrement_before {
                self.local_get(OP2_LOCAL);
                self.write(rn);
            } else {
                self.local_get(OP2_LOCAL);
                self.i32_const((4 * count) as i32);
                self.buf.push(op::I32_ADD);
                self.write(rn);
            }
        }

        self.emit_exc_return_or_next_pc(pc, &writes_before);

        self.buf.push(op::ELSE);
        self.emit_fault(pc, &writes_before);
        self.buf.push(op::END);
    }

    fn emit_ldm_ea_in_window(&mut self, ram_base: u32, hi: u32) {
        // result &= (scratch in [ram_base, hi]); scratch += 4
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(ram_base as i32);
        self.buf.push(op::I32_GE_U);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(hi as i32);
        self.buf.push(op::I32_LE_U);
        self.buf.push(op::I32_AND);
        self.local_get(RESULT_LOCAL);
        self.buf.push(op::I32_AND);
        self.local_set(RESULT_LOCAL);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(4);
        self.buf.push(op::I32_ADD);
        self.local_set(SCRATCH_LOCAL);
    }

    fn emit_load_sp_off(&mut self, pc: u32, off: i32, rd: u8) {
        self.read(13, pc);
        self.i32_const(off);
        self.buf.push(op::I32_ADD);
        self.local_set(SCRATCH_LOCAL);
        self.emit_host_load(&MemAccess::Load {
            rd,
            opcode: op::I32_LOAD,
        });
        self.write(rd);
    }

    fn emit_ldr_pc_from_scratch(&mut self, pc: u32) {
        let writes_before = self.writes;
        let Some((ram_base, ram_len)) = self.window else {
            self.emit_unsupported(pc, &writes_before);
            return;
        };
        let hi = ram_base.wrapping_add(ram_len).wrapping_sub(4);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(ram_base as i32);
        self.buf.push(op::I32_GE_U);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(hi as i32);
        self.buf.push(op::I32_LE_U);
        self.buf.push(op::I32_AND);
        self.buf.push(op::IF);
        self.buf.push(op::T_EMPTY);
        self.emit_host_load(&MemAccess::Load {
            rd: 0,
            opcode: op::I32_LOAD,
        });
        self.local_set(SCRATCH_LOCAL);
        self.emit_exc_return_or_next_pc(pc, &writes_before);
        self.buf.push(op::ELSE);
        self.emit_fault(pc, &writes_before);
        self.buf.push(op::END);
    }

    fn emit_exc_return_or_next_pc(&mut self, pc: u32, writes_before: &[bool; 16]) {
        // EXC_RETURN: (addr & 0xFFFFFFF0) == 0xFFFFFFF0 → interpreter.
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(0xFFFFFFF0_u32 as i32);
        self.buf.push(op::I32_AND);
        self.i32_const(0xFFFFFFF0_u32 as i32);
        self.buf.push(op::I32_EQ);
        self.buf.push(op::IF);
        self.buf.push(op::T_EMPTY);
        self.emit_unsupported(pc, writes_before);
        self.buf.push(op::END);
        self.i32_const(NEXT_PC_SLOT);
        self.local_get(SCRATCH_LOCAL);
        self.i32_const(!1);
        self.buf.push(op::I32_AND);
        self.store_next_pc();
    }

    fn emit_pop_pc(&mut self, pc: u32, registers: u8) {
        // Interpreter loads r0-r7, then PC, commits SP, then branch_to.
        // Range-check the whole list first; any out-of-window EA side-exits
        // with SP unchanged so the interpreter re-executes the POP.
        let mut count = 1u32;
        for i in 0..=7 {
            if (registers & (1 << i)) != 0 {
                count += 1;
            }
        }
        let writes_before = self.writes;
        let Some((ram_base, ram_len)) = self.window else {
            self.emit_unsupported(pc, &writes_before);
            return;
        };
        let ram_end = ram_base.wrapping_add(ram_len);
        let hi = ram_end.wrapping_sub(4);
        self.has_mem = true;

        self.i32_const(1);
        self.local_set(RESULT_LOCAL);
        self.read(13, pc);
        self.local_set(SCRATCH_LOCAL);
        for _ in 0..count {
            self.emit_ldm_ea_in_window(ram_base, hi);
        }

        self.local_get(RESULT_LOCAL);
        self.buf.push(op::IF);
        self.buf.push(op::T_EMPTY);

        let mut off: i32 = 0;
        for i in 0..=7u8 {
            if (registers & (1 << i)) != 0 {
                self.emit_load_sp_off(pc, off, i);
                off += 4;
            }
        }
        self.read(13, pc);
        self.i32_const(off);
        self.buf.push(op::I32_ADD);
        self.local_set(SCRATCH_LOCAL);
        self.emit_host_load(&MemAccess::Load {
            rd: 0,
            opcode: op::I32_LOAD,
        });
        self.local_set(SCRATCH_LOCAL);

        self.read(13, pc);
        self.i32_const((4 * count) as i32);
        self.buf.push(op::I32_ADD);
        self.write(13);

        self.emit_exc_return_or_next_pc(pc, &writes_before);

        self.buf.push(op::ELSE);
        self.emit_fault(pc, &writes_before);
        self.buf.push(op::END);
    }

    fn store_next_pc(&mut self) {
        self.buf.push(op::I32_STORE);
        enc::uleb(&mut self.buf, 2);
        enc::uleb(&mut self.buf, 0);
    }

    fn next_pc_const(&mut self, v: i32) {
        self.i32_const(NEXT_PC_SLOT);
        self.i32_const(v);
        self.store_next_pc();
    }

    /// Push 0/1 for APSR condition `cond`.
    fn push_condition(&mut self, cond: u8) {
        self.xpsr_touch();
        self.local_get(XPSR_LOCAL);
        self.local_set(SCRATCH_LOCAL);
        // n,z,c,v from bits 31,30,29,28
        let n = |b: &mut Self| {
            b.local_get(SCRATCH_LOCAL);
            b.i32_const(31);
            b.buf.push(op::I32_SHR_U);
        };
        let z = |b: &mut Self| {
            b.local_get(SCRATCH_LOCAL);
            b.i32_const(30);
            b.buf.push(op::I32_SHR_U);
            b.i32_const(1);
            b.buf.push(op::I32_AND);
        };
        let c = |b: &mut Self| {
            b.local_get(SCRATCH_LOCAL);
            b.i32_const(29);
            b.buf.push(op::I32_SHR_U);
            b.i32_const(1);
            b.buf.push(op::I32_AND);
        };
        let v = |b: &mut Self| {
            b.local_get(SCRATCH_LOCAL);
            b.i32_const(28);
            b.buf.push(op::I32_SHR_U);
            b.i32_const(1);
            b.buf.push(op::I32_AND);
        };
        match cond {
            0x0 => z(self),
            0x1 => {
                z(self);
                self.buf.push(op::I32_EQZ);
            }
            0x2 => c(self),
            0x3 => {
                c(self);
                self.buf.push(op::I32_EQZ);
            }
            0x4 => n(self),
            0x5 => {
                n(self);
                self.buf.push(op::I32_EQZ);
            }
            0x6 => v(self),
            0x7 => {
                v(self);
                self.buf.push(op::I32_EQZ);
            }
            0x8 => {
                c(self);
                z(self);
                self.buf.push(op::I32_EQZ);
                self.buf.push(op::I32_AND);
            }
            0x9 => {
                c(self);
                self.buf.push(op::I32_EQZ);
                z(self);
                self.buf.push(op::I32_OR);
            }
            0xA => {
                n(self);
                v(self);
                self.buf.push(op::I32_EQ);
            }
            0xB => {
                n(self);
                v(self);
                self.buf.push(op::I32_NE);
            }
            0xC => {
                z(self);
                self.buf.push(op::I32_EQZ);
                n(self);
                v(self);
                self.buf.push(op::I32_EQ);
                self.buf.push(op::I32_AND);
            }
            0xD => {
                z(self);
                n(self);
                v(self);
                self.buf.push(op::I32_NE);
                self.buf.push(op::I32_OR);
            }
            0xE => self.i32_const(1),
            _ => self.i32_const(0),
        }
    }

    fn emit_terminator(&mut self, pc: u32, ilen: u32, inst: &Instruction) {
        use Instruction::*;
        match *inst {
            Branch { offset } => {
                self.next_pc_const(pc.wrapping_add(4).wrapping_add(offset as u32) as i32);
            }
            BranchCond { cond, offset } => {
                self.i32_const(NEXT_PC_SLOT);
                self.push_condition(cond);
                self.buf.push(op::IF);
                self.buf.push(op::T_I32);
                self.i32_const(pc.wrapping_add(4).wrapping_add(offset as u32) as i32);
                self.buf.push(op::ELSE);
                self.i32_const(pc.wrapping_add(ilen) as i32);
                self.buf.push(op::END);
                self.store_next_pc();
            }
            Cbz { rn, imm } => {
                self.i32_const(NEXT_PC_SLOT);
                self.read(rn, pc);
                self.buf.push(op::I32_EQZ);
                self.buf.push(op::IF);
                self.buf.push(op::T_I32);
                self.i32_const(pc.wrapping_add(4).wrapping_add(imm as u32) as i32);
                self.buf.push(op::ELSE);
                self.i32_const(pc.wrapping_add(ilen) as i32);
                self.buf.push(op::END);
                self.store_next_pc();
            }
            Cbnz { rn, imm } => {
                self.i32_const(NEXT_PC_SLOT);
                self.read(rn, pc);
                self.buf.push(op::IF);
                self.buf.push(op::T_I32);
                self.i32_const(pc.wrapping_add(4).wrapping_add(imm as u32) as i32);
                self.buf.push(op::ELSE);
                self.i32_const(pc.wrapping_add(ilen) as i32);
                self.buf.push(op::END);
                self.store_next_pc();
            }
            Bl { offset } => {
                self.i32_const((pc.wrapping_add(4) | 1) as i32);
                self.write(14);
                self.next_pc_const(pc.wrapping_add(4).wrapping_add(offset as u32) as i32);
            }
            Bx { rm } | BlxReg { rm } => {
                let writes_before = self.writes;
                if matches!(inst, BlxReg { .. }) {
                    self.i32_const((pc.wrapping_add(2) | 1) as i32);
                    self.write(14);
                }
                self.read(rm, pc);
                self.local_set(SCRATCH_LOCAL);
                self.emit_exc_return_or_next_pc(pc, &writes_before);
            }
            MovReg { rd: 15, rm } => {
                let writes_before = self.writes;
                self.read(rm, pc);
                self.local_set(SCRATCH_LOCAL);
                self.emit_exc_return_or_next_pc(pc, &writes_before);
            }
            Pop { registers, p: true } => self.emit_pop_pc(pc, registers),
            LdrImm32 { rt: 15, rn, imm12 } => {
                self.has_mem = true;
                self.read(rn, pc);
                self.i32_const(imm12 as i32);
                self.buf.push(op::I32_ADD);
                self.local_set(SCRATCH_LOCAL);
                self.emit_ldr_pc_from_scratch(pc);
            }
            LdrImm32Idx {
                rt: 15,
                rn,
                imm8,
                pre_index,
                add,
                writeback,
            } => {
                self.has_mem = true;
                self.read(rn, pc);
                self.local_set(OP2_LOCAL);
                if pre_index {
                    self.local_get(OP2_LOCAL);
                    self.i32_const(imm8 as i32);
                    if add {
                        self.buf.push(op::I32_ADD);
                    } else {
                        self.buf.push(op::I32_SUB);
                    }
                } else {
                    self.local_get(OP2_LOCAL);
                }
                self.local_set(SCRATCH_LOCAL);
                let writes_before = self.writes;
                let Some((ram_base, ram_len)) = self.window else {
                    self.emit_unsupported(pc, &writes_before);
                    return;
                };
                let hi = ram_base.wrapping_add(ram_len).wrapping_sub(4);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(ram_base as i32);
                self.buf.push(op::I32_GE_U);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(hi as i32);
                self.buf.push(op::I32_LE_U);
                self.buf.push(op::I32_AND);
                self.buf.push(op::IF);
                self.buf.push(op::T_EMPTY);
                self.emit_host_load(&MemAccess::Load {
                    rd: 0,
                    opcode: op::I32_LOAD,
                });
                self.local_set(SCRATCH_LOCAL);
                if writeback {
                    self.local_get(OP2_LOCAL);
                    self.i32_const(imm8 as i32);
                    if add {
                        self.buf.push(op::I32_ADD);
                    } else {
                        self.buf.push(op::I32_SUB);
                    }
                    self.write(rn);
                }
                self.emit_exc_return_or_next_pc(pc, &writes_before);
                self.buf.push(op::ELSE);
                self.emit_fault(pc, &writes_before);
                self.buf.push(op::END);
            }
            LdmiaW {
                rn,
                reg_list,
                writeback,
            } => self.emit_ldm_w_pc(pc, rn, reg_list, writeback, false),
            LdmdbW {
                rn,
                reg_list,
                writeback,
            } => self.emit_ldm_w_pc(pc, rn, reg_list, writeback, true),
            other => unreachable!("non-terminator reached emit_terminator: {other:?}"),
        }
    }

    fn emit_prologue(&self, out: &mut Vec<u8>) {
        for r in 0..16u8 {
            if self.reads[r as usize] {
                out.push(op::I32_CONST);
                enc::sleb(out, (r as i64) * 4);
                out.push(op::I32_LOAD);
                enc::uleb(out, 2);
                enc::uleb(out, 0);
                out.push(op::LOCAL_SET);
                enc::uleb(out, r as u64);
            }
        }
    }

    fn emit_epilogue(&self, out: &mut Vec<u8>) {
        for r in 0..16u8 {
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
