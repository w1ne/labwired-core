// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RV32IMC integer-ALU wasm codegen (JIT framework chunk C).
//!
//! This is the **first block that executes instead of bailing**. It turns a
//! straight-line run of integer arithmetic / logical / shift / mul-div
//! instructions into a real wasm module (via [`super::wasm_encode`]) that a
//! runtime ([`super::exec`]) runs against the guest register file.
//!
//! ## The register-file-in-locals model
//!
//! The compiled module imports one memory — the **guest register file**,
//! word `i` = `xi` at byte offset `i*4` (see
//! [`wasm_encode::REGS_IMPORT_MODULE`]). Each block:
//!
//!   1. **Prologue** — loads every register it *reads* from that memory into
//!      a wasm local (`x0..x31` map to locals `0..31`).
//!   2. **Body** — one emit per guest instruction, operating purely on
//!      locals. `x0` reads as the constant `0` and writes to it are dropped
//!      (mirrors [`crate::cpu::riscv::RiscV::read_reg`]/`write_reg`).
//!   3. **Epilogue** — stores every register it *wrote* back to the memory.
//!
//! Keeping the whole block's arithmetic in locals means only the touched
//! registers cross the memory boundary, and only twice (in at entry, out at
//! exit) regardless of block length — the shape that lets a hot ALU run beat
//! the interpreter.
//!
//! ## Where a block ends (chunk-C scope)
//!
//! The walk collects the maximal prefix of **ALU-emittable** instructions
//! ([`is_alu_emittable`]) from the entry PC and stops *before* the first
//! instruction it cannot emit — a branch/jump (chunk D), a load/store
//! (chunk E), or an interpreter-owned op (CSR/system/atomics). The block
//! runs the prefix and side-exits with the [fall-through wire](WIRE_FALL_THROUGH)
//! → [`SideExit::Chain`](super::super::side_exit::SideExit::Chain) to
//! `end_pc`, where the interpreter (or a future compiled block) resumes.
//! There is no in-block control flow yet, so every chunk-C block has exactly
//! this one exit.

use crate::decoder::riscv::{decode_rv32, Instruction};

use super::super::frontend::ExitEdge;
use super::super::side_exit::BailReason;
use super::super::{CodeView, Pc};
use super::inst_len;
use super::wasm_encode::{build_module, enc, op};

/// Wire code the emitted body returns on a clean straight-line fall-through
/// to `end_pc`. The runtime maps it to
/// [`SideExit::Chain`](super::super::side_exit::SideExit::Chain). Chunks D/E
/// add further wire codes (taken branch, memory fault) alongside this one.
pub const WIRE_FALL_THROUGH: i32 = 0;

/// Number of `i32` locals every compiled block declares: one per guest
/// register `x0..x31`. Local index == register number.
const REG_LOCALS: u32 = 32;

/// Is `inst` an integer-ALU instruction chunk C emits wasm for?
///
/// This is the codegen allowlist. It is a strict subset of the walker's
/// [`InstrClass::Sequential`](super::InstrClass): loads/stores are
/// `Sequential` too but belong to chunk E, so they are **not** here — a
/// block ends before them. Anything not listed keeps the interpreter.
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

/// One decoded ALU instruction plus the guest PC it sits at (needed for
/// `AUIPC`, which folds `pc` into a constant).
struct AluOp {
    pc: u32,
    inst: Instruction,
}

/// The result of emitting a block: the wasm bytes plus the metadata the
/// frontend stamps onto its [`BlockPlan`](super::super::frontend::BlockPlan).
pub struct EmittedAluBlock {
    /// Real wasm module bytes (magic-prefixed), consumed by the runtime.
    pub code: Vec<u8>,
    /// Guest PC one past the last emitted instruction — the fall-through
    /// continuation the runtime chains to.
    pub end_pc: Pc,
    /// Number of guest instructions the block retires in one run.
    pub instr_count: u32,
    /// Side-exit edges, indexed by returned wire code. Chunk C has exactly
    /// the fall-through edge.
    pub exits: Vec<ExitEdge>,
}

/// Emit a wasm block for the maximal ALU prefix at `pc`.
///
/// Returns `None` when the instruction at `pc` is not ALU-emittable (the
/// caller keeps that PC on the interpreter — never an error).
pub fn emit_alu_block(pc: Pc, code: &CodeView<'_>) -> Option<EmittedAluBlock> {
    let ops = walk_alu_ops(pc, code);
    if ops.is_empty() {
        return None;
    }
    let end_pc = pc + ops.iter().map(|o| inst_len_of(o.pc, code)).sum::<u64>();
    let instr_count = ops.len() as u32;

    // Emit the body into a scratch buffer while recording which registers
    // are read (load set) and written (store set), then frame the function
    // expression as prologue + body + epilogue + fall-through wire code.
    let mut body = Body::default();
    for aop in &ops {
        body.emit_instruction(aop.pc, &aop.inst);
    }

    let mut expr = Vec::with_capacity(body.buf.len() + 16 * REG_LOCALS as usize);
    body.emit_prologue(&mut expr); // loads touched regs into locals
    expr.extend_from_slice(&body.buf); // body operating purely on locals
    body.emit_epilogue(&mut expr); // stores written regs back to mem
    expr.push(op::I32_CONST); // the block's return value
    enc::sleb(&mut expr, WIRE_FALL_THROUGH as i64);

    let code_bytes = build_module(REG_LOCALS, &expr);

    Some(EmittedAluBlock {
        code: code_bytes,
        end_pc,
        instr_count,
        exits: vec![ExitEdge {
            wire_code: WIRE_FALL_THROUGH,
            // The block is "partial": it retired its ALU prefix and hands
            // the following (non-ALU) instruction to the interpreter / the
            // next compiled block. Telemetry only — correctness is the
            // runtime's Chain{end_pc}.
            reason: BailReason::PartialBlock,
        }],
    })
}

/// Walk the maximal run of ALU-emittable instructions from `pc`.
fn walk_alu_ops(pc: Pc, code: &CodeView<'_>) -> Vec<AluOp> {
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
        if !is_alu_emittable(&inst) {
            break;
        }
        ops.push(AluOp {
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

/// Accumulates the body opcodes plus the read/write register sets.
#[derive(Default)]
struct Body {
    buf: Vec<u8>,
    /// Registers read anywhere in the block (loaded in the prologue).
    reads: [bool; 32],
    /// Registers written anywhere in the block (stored in the epilogue).
    writes: [bool; 32],
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

    /// Consume the value currently on the stack as the new value of guest
    /// register `r` (`local.set`, or `drop` for `x0`).
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

    /// Emit the high-half multiply family: extend both operands to i64 with
    /// `s1`/`s2`, `i64.mul`, `>> 32`, wrap to i32.
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

    /// Push `read(r) i32.eqz` (is register `r` zero?).
    fn is_zero(&mut self, r: u8) {
        self.read(r);
        self.buf.push(op::I32_EQZ);
    }

    /// Push `(read(rs1) == i32::MIN) && (read(rs2) == -1)` — the signed
    /// division overflow predicate.
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

    /// Emit one guest instruction. `pc` is the instruction's own PC (for
    /// `AUIPC`).
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

            // ── immediate shifts (shamt is a decoded 5-bit constant) ────
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
            // wasm masks the shift count to 5 bits — identical to the
            // interpreter's explicit `& 0x1F`.
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
                // divisor == 0 -> -1
                self.is_zero(rs2);
                self.if_i32();
                self.i32_const(-1);
                self.buf.push(op::ELSE);
                // INT_MIN / -1 -> INT_MIN (== dividend); else div_s.
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
                // divisor == 0 -> u32::MAX (0xFFFF_FFFF == -1 bits); else div_u.
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
                // divisor == 0 -> dividend; INT_MIN/-1 -> 0; else rem_s.
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
                // divisor == 0 -> dividend; else rem_u.
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

            // ── compressed forms ───────────────────────────────────────
            // `C.ADDI rd, imm` (rd == 0 is a hint/nop — the x0-drop makes it
            // a no-op automatically, matching the interpreter's guard).
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
            // `C.ADDI4SPN rd, uimm` — rd = x2 + uimm (uimm zero-extended).
            CAddi4spn { rd, imm } => self.binop_imm(rd, 2, imm as i32, op::I32_ADD),
            CSli { rd, shamt } => self.binop_imm(rd, rd, shamt as i32, op::I32_SHL),

            // Anything else must not reach here — the walk stops before it.
            other => unreachable!("non-ALU instruction reached emit: {other:?}"),
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

    fn enc_addi(rd: u32, rs1: u32, imm: i32) -> u32 {
        ((imm as u32 & 0xFFF) << 20) | (rs1 << 15) | (rd << 7) | 0x13
    }
    fn enc_add(rd: u32, rs1: u32, rs2: u32) -> u32 {
        (rs2 << 20) | (rs1 << 15) | (rd << 7) | 0x33
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
        assert!(emit_alu_block(BASE, &cv).is_none());
    }

    #[test]
    fn walks_alu_prefix_and_stops_before_non_alu() {
        // addi x1,x0,1 ; add x2,x1,x1 ; ecall
        let prog = view(&[enc_addi(1, 0, 1), enc_add(2, 1, 1), enc_ecall()]);
        let cv = CodeView::new(BASE, &prog);
        let blk = emit_alu_block(BASE, &cv).unwrap();
        assert_eq!(blk.instr_count, 2, "ecall is not included");
        assert_eq!(blk.end_pc, BASE + 8, "ends before the ecall");
        assert_eq!(blk.exits.len(), 1);
        assert_eq!(blk.exits[0].wire_code, WIRE_FALL_THROUGH);
        // Real wasm bytes.
        assert_eq!(&blk.code[0..4], &[0x00, 0x61, 0x73, 0x6d]);
    }

    #[cfg(feature = "jit")]
    #[test]
    fn emitted_module_validates_in_wasmtime() {
        // Emit a block exercising several op classes and assert the bytes are
        // a valid wasm module (the strongest cheap structural check).
        let prog = view(&[
            enc_addi(1, 0, 5), // addi x1,x0,5
            enc_add(2, 1, 1),  // add  x2,x1,x1
            enc_add(1, 1, 2),  // add  x1,x1,x2
            enc_ecall(),       // terminator (not emitted)
        ]);
        let cv = CodeView::new(BASE, &prog);
        let blk = emit_alu_block(BASE, &cv).unwrap();
        let engine = wasmtime::Engine::default();
        wasmtime::Module::new(&engine, &blk.code).expect("emitted module must validate");
        assert_eq!(blk.instr_count, 3);
    }
}
