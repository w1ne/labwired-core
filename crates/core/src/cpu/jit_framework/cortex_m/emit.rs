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
use super::super::riscv::wasm_encode::{build_module, enc, op};
use super::super::side_exit::BailReason;
use super::super::{CodeView, Pc};
use super::decode_at;

pub const WIRE_FALL_THROUGH: i32 = 0;
pub const WIRE_CHAIN_DYNAMIC: i32 = 1;
pub const WIRE_MEM_FAULT: i32 = 2;
pub const WIRE_UNSUPPORTED: i32 = 3;

const XPSR_LOCAL: u32 = 15;
const SCRATCH_LOCAL: u32 = 16;
const RESULT_LOCAL: u32 = 17;
const OP2_LOCAL: u32 = 18;
const LOCAL_COUNT: u32 = 19;

pub const NEXT_PC_SLOT: i32 = 16 * 4;
pub const FAULT_PC_SLOT: u32 = 68;
pub const FAULT_RETIRED_SLOT: u32 = 72;
pub const RES_FLAG_SLOT: u32 = 76;
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
        | Mul { .. }
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
        | LdrLit { .. } => true,
        AddRegHigh { rd, .. } | MovReg { rd, .. } if *rd != 15 => true,
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
            if *rt != 15 && *rn != 15 =>
        {
            true
        }
        LdrSp { rt, .. } | StrSp { rt, .. } if *rt != 15 => true,
        Push { .. } | Pop { p: false, .. } => true,
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
    )
}

fn is_emittable(inst: &Instruction, mem_ok: bool) -> bool {
    is_alu_emittable(inst) || (mem_ok && is_mem_emittable(inst))
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

    let binding = if body.has_mem {
        let (_base, len) = window.expect("mem op emitted without a RAM window");
        Some(MemBinding {
            ram_len: len as usize,
            has_store: body.has_store,
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
    let code_bytes = build_module(LOCAL_COUNT, mem_pages, &expr);

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

fn walk_ops(pc: Pc, code: &CodeView<'_>, mem_ok: bool) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut cur = pc;
    while let Some((inst, len)) = decode_at(cur, code) {
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
}

#[derive(Default)]
struct Body {
    buf: Vec<u8>,
    reads: [bool; 16],
    writes: [bool; 16],
    window: Option<RamWindow>,
    has_mem: bool,
    has_store: bool,
    has_unsupported: bool,
    emitted: u32,
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
    fn update_nz(&mut self) {
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
        self.i32_const(WIRE_UNSUPPORTED);
        self.buf.push(op::RETURN);
    }

    fn emit_mem(&mut self, pc: u32, addr_reg: u8, imm: i32, access: MemAccess) {
        let (ram_base, ram_len) = self.window.expect("emit_mem without window");
        let ram_end = ram_base.wrapping_add(ram_len);
        let width = access.width();
        let hi = ram_end.wrapping_sub(width);
        let delta = RAM_WINDOW_OFF.wrapping_sub(ram_base);
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
            MemAccess::Load { rd, opcode } => {
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(delta as i32);
                self.buf.push(op::I32_ADD);
                self.buf.push(opcode);
                enc::uleb(&mut self.buf, 0);
                enc::uleb(&mut self.buf, 0);
                self.write(rd);
            }
            MemAccess::Store { rs2, opcode } => {
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(delta as i32);
                self.buf.push(op::I32_ADD);
                self.read(rs2, pc);
                self.buf.push(opcode);
                enc::uleb(&mut self.buf, 0);
                enc::uleb(&mut self.buf, 0);
                self.store_const_at(RES_FLAG_SLOT, 1);
                self.has_store = true;
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
        let wasm_addr = addr.wrapping_sub(ram_base).wrapping_add(RAM_WINDOW_OFF);
        match access {
            MemAccess::Load { rd, opcode } => {
                self.i32_const(wasm_addr as i32);
                self.buf.push(opcode);
                enc::uleb(&mut self.buf, 0);
                enc::uleb(&mut self.buf, 0);
                self.write(rd);
            }
            MemAccess::Store { rs2, opcode } => {
                self.i32_const(wasm_addr as i32);
                self.read(rs2, pc);
                self.buf.push(opcode);
                enc::uleb(&mut self.buf, 0);
                enc::uleb(&mut self.buf, 0);
                self.store_const_at(RES_FLAG_SLOT, 1);
                self.has_store = true;
            }
        }
    }

    fn emit_instruction(&mut self, pc: u32, inst: &Instruction, code: &CodeView<'_>) {
        use Instruction::*;
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
                self.update_nz();
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
            Pop { registers, p } if !p => self.emit_pop(pc, registers),
            _ => unreachable!("non-emittable instruction reached emit: {inst:?}"),
        }
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

    fn emit_mem_reg(&mut self, pc: u32, rn: u8, rm: u8, access: MemAccess) {
        let (ram_base, ram_len) = self.window.expect("emit_mem_reg without window");
        let ram_end = ram_base.wrapping_add(ram_len);
        let width = access.width();
        let hi = ram_end.wrapping_sub(width);
        let delta = RAM_WINDOW_OFF.wrapping_sub(ram_base);
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
            MemAccess::Load { rd, opcode } => {
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(delta as i32);
                self.buf.push(op::I32_ADD);
                self.buf.push(opcode);
                enc::uleb(&mut self.buf, 0);
                enc::uleb(&mut self.buf, 0);
                self.write(rd);
            }
            MemAccess::Store { rs2, opcode } => {
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(delta as i32);
                self.buf.push(op::I32_ADD);
                self.read(rs2, pc);
                self.buf.push(opcode);
                enc::uleb(&mut self.buf, 0);
                enc::uleb(&mut self.buf, 0);
                self.store_const_at(RES_FLAG_SLOT, 1);
                self.has_store = true;
            }
        }
        self.buf.push(op::ELSE);
        self.emit_fault(pc, &writes_before);
        self.buf.push(op::END);
    }

    fn emit_push(&mut self, pc: u32, registers: u8, m: bool) {
        // Match interpreter: LR first (if M), then R7..R0.
        if m {
            self.read(13, pc);
            self.i32_const(4);
            self.buf.push(op::I32_SUB);
            self.write(13);
            self.emit_store_at_ea(pc, 14);
        }
        for i in (0..=7).rev() {
            if (registers & (1 << i)) != 0 {
                self.read(13, pc);
                self.i32_const(4);
                self.buf.push(op::I32_SUB);
                self.write(13);
                self.emit_store_at_ea(pc, i);
            }
        }
    }

    fn emit_pop(&mut self, pc: u32, registers: u8) {
        for i in 0..=7u8 {
            if (registers & (1 << i)) != 0 {
                self.emit_load_at_sp(pc, i);
                self.read(13, pc);
                self.i32_const(4);
                self.buf.push(op::I32_ADD);
                self.write(13);
            }
        }
    }

    fn emit_store_at_ea(&mut self, pc: u32, rs2: u8) {
        // SP is already the address (in local 13). Range-check and store.
        self.emit_mem(
            pc,
            13,
            0,
            MemAccess::Store {
                rs2,
                opcode: op::I32_STORE,
            },
        );
    }

    fn emit_load_at_sp(&mut self, pc: u32, rd: u8) {
        self.emit_mem(
            pc,
            13,
            0,
            MemAccess::Load {
                rd,
                opcode: op::I32_LOAD,
            },
        );
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
                // EXC_RETURN: (rm & 0xFFFFFFF0) == 0xFFFFFFF0 → interpreter.
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(0xFFFFFFF0_u32 as i32);
                self.buf.push(op::I32_AND);
                self.i32_const(0xFFFFFFF0_u32 as i32);
                self.buf.push(op::I32_EQ);
                self.buf.push(op::IF);
                self.buf.push(op::T_EMPTY);
                self.emit_unsupported(pc, &writes_before);
                self.buf.push(op::END);
                self.i32_const(NEXT_PC_SLOT);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(!1);
                self.buf.push(op::I32_AND);
                self.store_next_pc();
            }
            MovReg { rd: 15, rm } => {
                let writes_before = self.writes;
                self.read(rm, pc);
                self.local_set(SCRATCH_LOCAL);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(0xFFFFFFF0_u32 as i32);
                self.buf.push(op::I32_AND);
                self.i32_const(0xFFFFFFF0_u32 as i32);
                self.buf.push(op::I32_EQ);
                self.buf.push(op::IF);
                self.buf.push(op::T_EMPTY);
                self.emit_unsupported(pc, &writes_before);
                self.buf.push(op::END);
                self.i32_const(NEXT_PC_SLOT);
                self.local_get(SCRATCH_LOCAL);
                self.i32_const(!1);
                self.buf.push(op::I32_AND);
                self.store_next_pc();
            }
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
