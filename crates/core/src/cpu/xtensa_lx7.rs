// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 CPU backend. Glues AR file, PS, SR file with the fetch loop
//! and `Cpu` trait.
//!
//! D1: ALU reg-reg (ADD/SUB/AND/OR/XOR/NEG/ABS/ADDX*/SUBX*), MOVI, NOP/fences, BREAK.
//! D2: Shift exec (SLL/SRL/SRA/SRC/SLLI/SRLI/SRAI) + SAR-setup (SSL/SSR/SSAI/SSA8L/SSA8B).
//! Remaining instruction classes in progress.

use crate::cpu::xtensa_regs::{ArFile, Ps};
use crate::cpu::xtensa_sr::{XtensaSrFile, EXCCAUSE, SAR, SCOMPARE1, VECBASE};
use crate::decoder::{xtensa, xtensa_length, xtensa_narrow};
use crate::snapshot::{CpuSnapshot, XtensaLx7CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationError, SimulationObserver};
use std::sync::Arc;

pub struct XtensaLx7 {
    pub regs: ArFile,
    pub ps: Ps,
    pub sr: XtensaSrFile,
    pub pc: u32,
}

impl XtensaLx7 {
    pub fn new() -> Self {
        Self {
            regs: ArFile::new(),
            // HW-verified: PS reset = 0x1F (EXCM=1, INTLEVEL=0xF).
            // Confirmed via OpenOCD `reset halt` on real S3-Zero: ps = 0x0000001f.
            ps: Ps::from_raw(0x1F),
            sr: XtensaSrFile::new(),
            pc: 0x4000_0400,
        }
    }

    fn execute(
        &mut self,
        ins: xtensa::Instruction,
        bus: &mut dyn Bus,
        len: u32,
    ) -> SimResult<()> {
        use xtensa::Instruction::*;
        match ins {
            Add { ar, as_, at } => {
                let v = self.regs.read_logical(as_).wrapping_add(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Sub { ar, as_, at } => {
                let v = self.regs.read_logical(as_).wrapping_sub(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            And { ar, as_, at } => {
                let v = self.regs.read_logical(as_) & self.regs.read_logical(at);
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Or { ar, as_, at } => {
                let v = self.regs.read_logical(as_) | self.regs.read_logical(at);
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Xor { ar, as_, at } => {
                let v = self.regs.read_logical(as_) ^ self.regs.read_logical(at);
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Neg { ar, at } => {
                let v = 0u32.wrapping_sub(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Abs { ar, at } => {
                // ISA RM: result is unsigned abs of the 2's-complement value.
                // i32::unsigned_abs() returns 0x80000000 for i32::MIN — matches HW behaviour.
                let x = self.regs.read_logical(at) as i32;
                self.regs.write_logical(ar, x.unsigned_abs());
                self.pc = self.pc.wrapping_add(len);
            }
            Addx2 { ar, as_, at } => {
                let v = (self.regs.read_logical(as_) << 1).wrapping_add(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Addx4 { ar, as_, at } => {
                let v = (self.regs.read_logical(as_) << 2).wrapping_add(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Addx8 { ar, as_, at } => {
                let v = (self.regs.read_logical(as_) << 3).wrapping_add(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Subx2 { ar, as_, at } => {
                let v = (self.regs.read_logical(as_) << 1).wrapping_sub(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Subx4 { ar, as_, at } => {
                let v = (self.regs.read_logical(as_) << 2).wrapping_sub(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Subx8 { ar, as_, at } => {
                let v = (self.regs.read_logical(as_) << 3).wrapping_sub(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Movi { at, imm } => {
                self.regs.write_logical(at, imm as u32);
                self.pc = self.pc.wrapping_add(len);
            }
            Break { .. } => {
                return Err(SimulationError::BreakpointHit(self.pc));
            }
            Nop | Memw | Extw | Isync | Rsync | Esync | Dsync => {
                self.pc = self.pc.wrapping_add(len);
            }

            // ── D2: SAR-setup instructions ───────────────────────────────────
            // SSL as_: SAR = 32 - (as_ & 0x1F).
            // When as_ & 0x1F == 0, SAR = 32 — valid 6-bit value per ISA RM §8.
            Ssl { as_ } => {
                let v = 32u32 - (self.regs.read_logical(as_) & 0x1F);
                self.sr.write(SAR, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SSR as_: SAR = as_ & 0x1F.
            Ssr { as_ } => {
                let v = self.regs.read_logical(as_) & 0x1F;
                self.sr.write(SAR, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SSAI shamt: SAR = shamt & 0x1F (decoder already bounds shamt to 5 bits).
            Ssai { shamt } => {
                self.sr.write(SAR, shamt as u32 & 0x1F);
                self.pc = self.pc.wrapping_add(len);
            }
            // SSA8L as_: SAR = (as_ & 3) * 8. (little-endian byte-select; ISA RM §4.3.7)
            Ssa8l { as_ } => {
                let v = (self.regs.read_logical(as_) & 3) * 8;
                self.sr.write(SAR, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SSA8B as_: SAR = 32 - (as_ & 3) * 8. (big-endian byte-select; ISA RM §4.3.7)
            // When as_ & 3 == 0, SAR = 32 — valid 6-bit value (SAR accommodates 0..=63).
            Ssa8b { as_ } => {
                let v = 32u32 - (self.regs.read_logical(as_) & 3) * 8;
                self.sr.write(SAR, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── D2: Shift register instructions ──────────────────────────────
            // SLL ar, as_: ar = as_ << (32 - SAR).
            // When SAR=0, shift count = 32. Use u64 cast to avoid Rust UB
            // (u64 shifts are defined for counts 0..=63 per Rust reference).
            // (as_ as u64) << 32 = 0 for any as_, which matches ISA RM §8.
            Sll { ar, as_ } => {
                let sar = self.sr.read(SAR);
                let shift = 32u32.wrapping_sub(sar);
                // SAR ranges by setter: SSL 1..=32, SSR 0..=31, SSAI 0..=31, SSA8L {0,8,16,24}, SSA8B {32,24,16,8}.
                // wrapping_sub handles SAR=32 → shift=0 (passthrough); SAR=0 → shift=32 (u64 << 32 yields 0).
                // u64 cast is required because a u32 << 32 is undefined in Rust.
                let v = ((self.regs.read_logical(as_) as u64) << shift) as u32;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SRL ar, at: ar = at >> SAR (unsigned). SAR is 0..=31.
            // For SAR >= 32 (possible if set via WSR), result is 0 per ISA RM §8.
            Srl { ar, at } => {
                let sar = self.sr.read(SAR);
                let v = if sar >= 32 {
                    0
                } else {
                    self.regs.read_logical(at) >> sar
                };
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SRA ar, at: ar = (at as i32) >> SAR (arithmetic). SAR is 0..=31.
            // For SAR >= 32 result is all sign bits: 0xFFFFFFFF or 0x00000000.
            Sra { ar, at } => {
                let sar = self.sr.read(SAR);
                let src = self.regs.read_logical(at) as i32;
                let v = if sar >= 32 {
                    if src < 0 { u32::MAX } else { 0 }
                } else {
                    (src >> sar) as u32
                };
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SRC ar, as_, at: ar = low32((as_ : at) >> SAR).
            // Concatenate as_ (upper 32b) and at (lower 32b) into 64b, shift right by SAR.
            // SAR is 0..=63; u64 shifts for counts 0..=63 are safe in Rust.
            Src { ar, as_, at } => {
                let sar = self.sr.read(SAR);
                let hi = self.regs.read_logical(as_) as u64;
                let lo = self.regs.read_logical(at) as u64;
                let w = (hi << 32) | lo;
                let v = (w >> sar) as u32;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── D2: Shift immediate instructions ─────────────────────────────
            // SLLI ar, as_, shamt: ar = as_ << shamt. shamt is 1..=31 (decoder
            // computes shamt = 32 - raw, so it's the actual count, never 0 or 32).
            Slli { ar, as_, shamt } => {
                let v = self.regs.read_logical(as_) << shamt;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SRLI ar, at, shamt: ar = at >> shamt (unsigned). shamt 0..=15 from decoder.
            // Note: `at` is the t field (= shamt & 0xF per ISA encoding).
            Srli { ar, at, shamt } => {
                let v = self.regs.read_logical(at) >> shamt;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // SRAI ar, at, shamt: ar = (at as i32) >> shamt (arithmetic). shamt 0..=31.
            // shamt < 32 always here (decoder range), so no need for SAR-guard.
            Srai { ar, at, shamt } => {
                let src = self.regs.read_logical(at) as i32;
                let v = (src >> shamt) as u32;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── D3: Arithmetic immediate instructions ──────────────────────────────
            // ADDI at, as_, imm8: at = as_ + sext8(imm8). Two's complement addition.
            Addi { at, as_, imm8 } => {
                let v = self.regs.read_logical(as_).wrapping_add(imm8 as u32);
                self.regs.write_logical(at, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // ADDMI at, as_, imm: at = as_ + imm, where imm = sext8(raw) << 8.
            // Decoder pre-shifts, so imm is already the full immediate value.
            Addmi { at, as_, imm } => {
                let v = self.regs.read_logical(as_).wrapping_add(imm as u32);
                self.regs.write_logical(at, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── D4: Load instructions ──────────────────────────────────────────

            // L8UI at, as_, imm: at = zero_extend(mem[as_ + imm]).
            // imm is the raw byte offset (0..=255); no alignment requirement.
            L8ui { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let val = bus.read_u8(ea)? as u32;
                self.regs.write_logical(at, val);
                self.pc = self.pc.wrapping_add(len);
            }

            // L16UI at, as_, imm: at = zero_extend(mem16[as_ + imm]).
            // Decoder pre-shifts imm by 1 (imm = raw_imm8 << 1), so imm is already
            // the byte offset. Requires 2-byte alignment; alignment check deferred to Phase G.
            L16ui { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let val = bus.read_u16(ea)? as u32;
                self.regs.write_logical(at, val);
                self.pc = self.pc.wrapping_add(len);
            }

            // L16SI at, as_, imm: at = sign_extend(mem16[as_ + imm]).
            // Decoder pre-shifts imm by 1. Sign-extend 16-bit to 32-bit.
            // Alignment check deferred to Phase G.
            L16si { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let raw = bus.read_u16(ea)?;
                // Sign-extend 16-bit: cast to i16 then to i32, reinterpret as u32.
                let val = (raw as i16) as i32 as u32;
                self.regs.write_logical(at, val);
                self.pc = self.pc.wrapping_add(len);
            }

            // L32I at, as_, imm: at = mem32[as_ + imm].
            // Decoder pre-shifts imm by 2 (imm = raw_imm8 << 2). Requires 4-byte alignment;
            // alignment check deferred to Phase G.
            L32i { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let val = bus.read_u32(ea)?;
                self.regs.write_logical(at, val);
                self.pc = self.pc.wrapping_add(len);
            }

            // L32R at, pc_rel_byte_offset:
            //   EA = ((pc + 3) & !3) + pc_rel_byte_offset
            // Decoder sign-extends imm16 as a word count and multiplies by 4 to
            // produce pc_rel_byte_offset (always negative in real code; literal pool
            // precedes the instruction). The resulting EA is always 4-byte aligned
            // (both the aligned base and the offset are multiples of 4).
            L32r { at, pc_rel_byte_offset } => {
                let base = (self.pc.wrapping_add(3)) & !3u32;
                let ea = base.wrapping_add(pc_rel_byte_offset as u32) as u64;
                let val = bus.read_u32(ea)?;
                self.regs.write_logical(at, val);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── D5: Store instructions ──────────────────────────────────────
            // S8I at, as_, imm: EA = as_ + imm; mem8[EA] = at[0:7].
            // imm is the raw byte offset (0..=255); no alignment requirement.
            S8i { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                bus.write_u8(ea, (self.regs.read_logical(at) & 0xFF) as u8)?;
                self.pc = self.pc.wrapping_add(len);
            }

            // S16I at, as_, imm: EA = as_ + imm; mem16[EA] = at[0:15].
            // Decoder pre-shifts imm by 1. Requires 2-byte alignment;
            // alignment check deferred to Phase G.
            S16i { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                bus.write_u16(ea, (self.regs.read_logical(at) & 0xFFFF) as u16)?;
                self.pc = self.pc.wrapping_add(len);
            }

            // S32I at, as_, imm: EA = as_ + imm; mem32[EA] = at.
            // Decoder pre-shifts imm by 2. Requires 4-byte alignment;
            // alignment check deferred to Phase G.
            S32i { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                bus.write_u32(ea, self.regs.read_logical(at))?;
                self.pc = self.pc.wrapping_add(len);
            }

            // ── D6: Branch instructions ───────────────────────────────────
            // Decoder pre-bakes +4 into all branch offsets, so:
            //   taken:     self.pc = self.pc.wrapping_add(offset as u32)
            //   not-taken: self.pc = self.pc.wrapping_add(len)

            // BEQ: taken if as_ == at
            Beq { as_, at, offset } => {
                let cond = self.regs.read_logical(as_) == self.regs.read_logical(at);
                self.branch(offset, len, cond);
            }
            // BNE: taken if as_ != at
            Bne { as_, at, offset } => {
                let cond = self.regs.read_logical(as_) != self.regs.read_logical(at);
                self.branch(offset, len, cond);
            }
            // BLT: taken if (as_ as i32) < (at as i32)
            Blt { as_, at, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) < (self.regs.read_logical(at) as i32);
                self.branch(offset, len, cond);
            }
            // BGE: taken if (as_ as i32) >= (at as i32)
            Bge { as_, at, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) >= (self.regs.read_logical(at) as i32);
                self.branch(offset, len, cond);
            }
            // BLTU: taken if as_ < at (unsigned)
            Bltu { as_, at, offset } => {
                let cond = self.regs.read_logical(as_) < self.regs.read_logical(at);
                self.branch(offset, len, cond);
            }
            // BGEU: taken if as_ >= at (unsigned)
            Bgeu { as_, at, offset } => {
                let cond = self.regs.read_logical(as_) >= self.regs.read_logical(at);
                self.branch(offset, len, cond);
            }

            // ── D7: Jumps and calls ───────────────────────────────────────────

            // J offset: unconditional jump; decoder pre-bakes +4 into offset.
            // pc = pc + offset  (offset = sign_extend18(imm18) + 4)
            J { offset } => {
                self.pc = self.pc.wrapping_add(offset as u32);
            }

            // JX as_: register-indirect unconditional jump.
            // pc = a[as_]
            Jx { as_ } => {
                self.pc = self.regs.read_logical(as_);
            }

            // CALL0 offset: save return address in a0, jump to target.
            // a0 = pc + 3  (return address: byte after this 3-byte instruction)
            // target = ((pc + 3) & !3) + offset  (ISA RM §4.4; decoder: offset = sext18 * 4)
            Call0 { offset } => {
                let ret_pc = self.pc.wrapping_add(3);
                let target = (self.pc.wrapping_add(3) & !3u32).wrapping_add(offset as u32);
                self.regs.write_logical(0, ret_pc);
                self.pc = target;
            }

            // CALLX0 as_: register-indirect CALL0.
            // a0 = pc + 3, pc = a[as_]
            Callx0 { as_ } => {
                let ret_pc = self.pc.wrapping_add(3);
                let target = self.regs.read_logical(as_);
                self.regs.write_logical(0, ret_pc);
                self.pc = target;
            }

            // CALL4/8/12 offset: windowed call.
            // a[N] = (pc + 3 low-30) | (N << 30)
            //   The return address encodes the call type in bits[31:30] so that
            //   RETW can recover N = a0[31:30] after the window rotation.
            //   ISA RM §8 CALL4: "upper two bits of the return address are set to 01".
            // PS.CALLINC = N / 4  (1, 2, or 3 for CALL4, CALL8, CALL12)
            // target = ((pc + 3) & !3) + offset  (ISA RM §4.4)
            Call4 { offset } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (1 << 30);
                let target = (self.pc.wrapping_add(3) & !3u32).wrapping_add(offset as u32);
                self.regs.write_logical(4, ret_pc);
                self.ps.set_callinc(1);
                self.pc = target;
            }
            Call8 { offset } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (2 << 30);
                let target = (self.pc.wrapping_add(3) & !3u32).wrapping_add(offset as u32);
                self.regs.write_logical(8, ret_pc);
                self.ps.set_callinc(2);
                self.pc = target;
            }
            Call12 { offset } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (3 << 30);
                let target = (self.pc.wrapping_add(3) & !3u32).wrapping_add(offset as u32);
                self.regs.write_logical(12, ret_pc);
                self.ps.set_callinc(3);
                self.pc = target;
            }

            // CALLX4/8/12 as_: register-indirect windowed calls.
            // Same semantics as CALL4/8/12 but target = a[as_] (before we overwrite a[N]).
            Callx4 { as_ } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (1 << 30);
                let target = self.regs.read_logical(as_);
                self.regs.write_logical(4, ret_pc);
                self.ps.set_callinc(1);
                self.pc = target;
            }
            Callx8 { as_ } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (2 << 30);
                let target = self.regs.read_logical(as_);
                self.regs.write_logical(8, ret_pc);
                self.ps.set_callinc(2);
                self.pc = target;
            }
            Callx12 { as_ } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (3 << 30);
                let target = self.regs.read_logical(as_);
                self.regs.write_logical(12, ret_pc);
                self.ps.set_callinc(3);
                self.pc = target;
            }

            // RET: CALL0 return. pc = a0.
            Ret => {
                self.pc = self.regs.read_logical(0);
            }

            // ── F1: ENTRY / RETW — windowed call prologue / epilogue ──────────

            // ENTRY as_, imm: windowed call prologue (no overflow check — deferred to F3).
            //
            // ISA RM §8 ENTRY semantics:
            //   1. WB_new = (WB_old + PS.CALLINC) mod 16
            //   2. WindowStart[WB_new] = 1
            //   3. PS.CALLINC = 0
            //   4. a[as_] -= imm * 8   (in the NEW window; as_ is the stack pointer)
            //   5. PC += len  (instruction is 3 bytes)
            //
            // Note: the rotation happens HERE (on ENTRY), not on CALL*. CALL* only
            // sets PS.CALLINC and stores the return address in a[N] of the OLD frame.
            // After rotation, the callee's a0 maps to the same physical reg as the
            // caller's a[CALLINC*4], which holds the return address written by CALL*.
            //
            // Overflow check (WindowStart[(WB_new+1) mod 16] == 1 → raise exception)
            // is deferred to F3.
            Entry { as_, imm } => {
                let callinc = self.ps.callinc();
                let wb_old = self.regs.windowbase();
                let wb_new = wb_old.wrapping_add(callinc) & 0x0F;
                self.regs.set_windowbase(wb_new);
                self.regs.set_windowstart_bit(wb_new, true);
                self.ps.set_callinc(0);
                // a[as_] in the NEW window (post-rotation) is decremented by imm * 8.
                let sp = self.regs.read_logical(as_);
                self.regs.write_logical(as_, sp.wrapping_sub(imm * 8));
                self.pc = self.pc.wrapping_add(len);
            }

            // RETW: windowed return (no underflow check — deferred to F4).
            //
            // ISA RM §8 RETW semantics:
            //   1. N = a0[31:30]  (1→CALL4, 2→CALL8, 3→CALL12)
            //   2. target_pc = (a[0] & 0x3FFF_FFFF) | (PC & 0xC000_0000)
            //   3. WindowStart[WB_current] = 0
            //   4. WB_new = (WB_current - N) mod 16
            //   5. PC = target_pc
            //
            // Underflow check (WindowStart[WB_new] == 0 → raise exception)
            // is deferred to F4.
            Retw => {
                let a0 = self.regs.read_logical(0);
                let n = (a0 >> 30) as u8;               // bits[31:30] = callinc used by the call
                let target_pc = (a0 & 0x3FFF_FFFF) | (self.pc & 0xC000_0000);
                let wb_cur = self.regs.windowbase();
                self.regs.set_windowstart_bit(wb_cur, false);
                let wb_new = wb_cur.wrapping_sub(n) & 0x0F;
                self.regs.set_windowbase(wb_new);
                self.pc = target_pc;
            }

            // BANY: taken if (as_ & at) != 0
            Bany { as_, at, offset } => {
                let cond = (self.regs.read_logical(as_) & self.regs.read_logical(at)) != 0;
                self.branch(offset, len, cond);
            }
            // BALL: taken if (as_ & at) == at  (all bits of at set in as_)
            Ball { as_, at, offset } => {
                let a = self.regs.read_logical(as_);
                let b = self.regs.read_logical(at);
                let cond = (a & b) == b;
                self.branch(offset, len, cond);
            }
            // BNONE: taken if (as_ & at) == 0
            Bnone { as_, at, offset } => {
                let cond = (self.regs.read_logical(as_) & self.regs.read_logical(at)) == 0;
                self.branch(offset, len, cond);
            }
            // BNALL: taken if (as_ & at) != at  (at least one bit of at missing in as_)
            Bnall { as_, at, offset } => {
                let a = self.regs.read_logical(as_);
                let b = self.regs.read_logical(at);
                let cond = (a & b) != b;
                self.branch(offset, len, cond);
            }
            // BBC: taken if bit (at & 0x1F) of as_ is CLEAR
            Bbc { as_, at, offset } => {
                let bit = self.regs.read_logical(at) & 0x1F;
                let cond = (self.regs.read_logical(as_) >> bit) & 1 == 0;
                self.branch(offset, len, cond);
            }
            // BBS: taken if bit (at & 0x1F) of as_ is SET
            Bbs { as_, at, offset } => {
                let bit = self.regs.read_logical(at) & 0x1F;
                let cond = (self.regs.read_logical(as_) >> bit) & 1 == 1;
                self.branch(offset, len, cond);
            }
            // BBCI: taken if bit `bit` (0..=31) of as_ is CLEAR
            Bbci { as_, bit, offset } => {
                let cond = (self.regs.read_logical(as_) >> bit) & 1 == 0;
                self.branch(offset, len, cond);
            }
            // BBSI: taken if bit `bit` (0..=31) of as_ is SET
            Bbsi { as_, bit, offset } => {
                let cond = (self.regs.read_logical(as_) >> bit) & 1 == 1;
                self.branch(offset, len, cond);
            }
            // BEQZ: taken if as_ == 0
            Beqz { as_, offset } => {
                let cond = self.regs.read_logical(as_) == 0;
                self.branch(offset, len, cond);
            }
            // BNEZ: taken if as_ != 0
            Bnez { as_, offset } => {
                let cond = self.regs.read_logical(as_) != 0;
                self.branch(offset, len, cond);
            }
            // BLTZ: taken if (as_ as i32) < 0
            Bltz { as_, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) < 0;
                self.branch(offset, len, cond);
            }
            // BGEZ: taken if (as_ as i32) >= 0
            Bgez { as_, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) >= 0;
                self.branch(offset, len, cond);
            }
            // BEQI: taken if as_ == imm  (decoder resolved B4CONST[r] into imm: i32)
            Beqi { as_, imm, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) == imm;
                self.branch(offset, len, cond);
            }
            // BNEI: taken if as_ != imm
            Bnei { as_, imm, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) != imm;
                self.branch(offset, len, cond);
            }
            // BLTI: taken if (as_ as i32) < imm  (signed)
            Blti { as_, imm, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) < imm;
                self.branch(offset, len, cond);
            }
            // BGEI: taken if (as_ as i32) >= imm  (signed)
            Bgei { as_, imm, offset } => {
                let cond = (self.regs.read_logical(as_) as i32) >= imm;
                self.branch(offset, len, cond);
            }
            // BLTUI: taken if as_ < imm  (unsigned; decoder resolved B4CONSTU[r] into imm: u32)
            Bltui { as_, imm, offset } => {
                let cond = self.regs.read_logical(as_) < imm;
                self.branch(offset, len, cond);
            }
            // BGEUI: taken if as_ >= imm  (unsigned)
            Bgeui { as_, imm, offset } => {
                let cond = self.regs.read_logical(as_) >= imm;
                self.branch(offset, len, cond);
            }

            // ── MUL family ────────────────────────────────────────────────────────
            // MULL: low 32 bits of unsigned 32×32 product (same bits as signed).
            Mull { ar, as_, at } => {
                let v = self.regs.read_logical(as_).wrapping_mul(self.regs.read_logical(at));
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // MULUH: upper 32 bits of unsigned 64-bit product.
            Muluh { ar, as_, at } => {
                let a = self.regs.read_logical(as_) as u64;
                let b = self.regs.read_logical(at) as u64;
                let v = (a.wrapping_mul(b) >> 32) as u32;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // MULSH: upper 32 bits of signed 64-bit product.
            Mulsh { ar, as_, at } => {
                let a = self.regs.read_logical(as_) as i32 as i64;
                let b = self.regs.read_logical(at) as i32 as i64;
                let v = (a.wrapping_mul(b) >> 32) as u32;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            // MUL16U: unsigned 16×16 → 32 product; only low 16 bits of each source used.
            Mul16u { ar, as_, at } => {
                let a = self.regs.read_logical(as_) & 0xFFFF;
                let b = self.regs.read_logical(at) & 0xFFFF;
                self.regs.write_logical(ar, a * b);
                self.pc = self.pc.wrapping_add(len);
            }
            // MUL16S: signed 16×16 → 32 product; low 16 sign-extended before multiply.
            Mul16s { ar, as_, at } => {
                let a = self.regs.read_logical(as_) as i16 as i32;
                let b = self.regs.read_logical(at) as i16 as i32;
                self.regs.write_logical(ar, (a * b) as u32);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── DIV family ────────────────────────────────────────────────────
            // Divide-by-zero: set EXCCAUSE=6 (IntegerDivideByZeroCause) and
            // return Err(ExceptionRaised). Full vector dispatch deferred to Phase G.

            // QUOS ar, as_, at: signed quotient as_ / at.
            // i32::MIN / -1 wraps to i32::MIN per ISA RM §8 (saturating result).
            Quos { ar, as_, at } => {
                let dividend = self.regs.read_logical(as_) as i32;
                let divisor  = self.regs.read_logical(at)  as i32;
                if divisor == 0 {
                    self.sr.write(EXCCAUSE, 6);
                    return Err(SimulationError::ExceptionRaised { cause: 6, pc: self.pc });
                }
                let q = dividend.wrapping_div(divisor);
                self.regs.write_logical(ar, q as u32);
                self.pc = self.pc.wrapping_add(len);
            }

            // QUOU ar, as_, at: unsigned quotient as_ / at.
            Quou { ar, as_, at } => {
                let dividend = self.regs.read_logical(as_);
                let divisor  = self.regs.read_logical(at);
                if divisor == 0 {
                    self.sr.write(EXCCAUSE, 6);
                    return Err(SimulationError::ExceptionRaised { cause: 6, pc: self.pc });
                }
                let q = dividend / divisor;
                self.regs.write_logical(ar, q);
                self.pc = self.pc.wrapping_add(len);
            }

            // REMS ar, as_, at: signed remainder as_ % at. Sign follows dividend (Rust `%` semantics).
            // i32::MIN % -1 = 0 (overflow corner; wrapping_rem handles this).
            Rems { ar, as_, at } => {
                let dividend = self.regs.read_logical(as_) as i32;
                let divisor  = self.regs.read_logical(at)  as i32;
                if divisor == 0 {
                    self.sr.write(EXCCAUSE, 6);
                    return Err(SimulationError::ExceptionRaised { cause: 6, pc: self.pc });
                }
                let r = dividend.wrapping_rem(divisor);
                self.regs.write_logical(ar, r as u32);
                self.pc = self.pc.wrapping_add(len);
            }

            // REMU ar, as_, at: unsigned remainder as_ % at.
            Remu { ar, as_, at } => {
                let dividend = self.regs.read_logical(as_);
                let divisor  = self.regs.read_logical(at);
                if divisor == 0 {
                    self.sr.write(EXCCAUSE, 6);
                    return Err(SimulationError::ExceptionRaised { cause: 6, pc: self.pc });
                }
                let r = dividend % divisor;
                self.regs.write_logical(ar, r);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── E3: Bit-manip instructions ────────────────────────────────────

            // NSA ar, as_: Number of Sign bits minus 1.
            // Result = clz(if as_ >= 0 then as_ else !as_) - 1.
            // For as_>=0: counts leading 0 bits minus 1 (result range 0..=31).
            // For as_<0:  counts leading 1 bits minus 1 (same range).
            // NSA(0) = 31 (clz(0)=32, 32-1=31). NSA(-1) = 31 (clz(!0xFFFF)=32, -1=31).
            Nsa { ar, as_ } => {
                let src = self.regs.read_logical(as_);
                let count = if (src as i32) >= 0 {
                    src.leading_zeros()
                } else {
                    (!src).leading_zeros()
                };
                self.regs.write_logical(ar, count - 1);
                self.pc = self.pc.wrapping_add(len);
            }

            // NSAU ar, as_: Number of leading zeros, Unsigned.
            // Result = clz(as_) for unsigned as_. NSAU(0) = 32.
            Nsau { ar, as_ } => {
                let src = self.regs.read_logical(as_);
                self.regs.write_logical(ar, src.leading_zeros());
                self.pc = self.pc.wrapping_add(len);
            }

            // MIN ar, as_, at: ar = signed min(as_, at).
            Min { ar, as_, at } => {
                let a = self.regs.read_logical(as_) as i32;
                let b = self.regs.read_logical(at)  as i32;
                self.regs.write_logical(ar, a.min(b) as u32);
                self.pc = self.pc.wrapping_add(len);
            }

            // MAX ar, as_, at: ar = signed max(as_, at).
            Max { ar, as_, at } => {
                let a = self.regs.read_logical(as_) as i32;
                let b = self.regs.read_logical(at)  as i32;
                self.regs.write_logical(ar, a.max(b) as u32);
                self.pc = self.pc.wrapping_add(len);
            }

            // MINU ar, as_, at: ar = unsigned min(as_, at).
            Minu { ar, as_, at } => {
                let a = self.regs.read_logical(as_);
                let b = self.regs.read_logical(at);
                self.regs.write_logical(ar, a.min(b));
                self.pc = self.pc.wrapping_add(len);
            }

            // MAXU ar, as_, at: ar = unsigned max(as_, at).
            Maxu { ar, as_, at } => {
                let a = self.regs.read_logical(as_);
                let b = self.regs.read_logical(at);
                self.regs.write_logical(ar, a.max(b));
                self.pc = self.pc.wrapping_add(len);
            }

            // SEXT ar, as_, t: sign-extend as_ from bit position t downward.
            // Decoder stores sa (7..=22) in the `t` field of the Instruction.
            // Bit[sa] of as_ is the sign bit; bits[sa-1:0] are preserved;
            // bits[31:sa] are filled with the value of bit[sa].
            // Equivalently: ((as_ as i32) << (31 - sa)) >> (31 - sa)
            Sext { ar, as_, t: sa } => {
                let src = self.regs.read_logical(as_);
                let shift = 31 - sa;  // sa is 7..=22, shift is 9..=24
                let v = ((src as i32) << shift >> shift) as u32;
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // CLAMPS ar, as_, t: saturate signed as_ into (sa+1)-bit signed range.
            // Decoder stores sa (7..=22) in the `t` field of the Instruction.
            // Range: [-(2^sa), 2^sa - 1].  For sa=7: [-128, 127].
            Clamps { ar, as_, t: sa } => {
                let src = self.regs.read_logical(as_) as i32;
                let max_val = (1i32 << sa) - 1;
                let min_val = -(1i32 << sa);
                let v = src.clamp(min_val, max_val);
                self.regs.write_logical(ar, v as u32);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── E4: Atomic memory instructions ───────────────────────────────

            // S32C1I at, as_, imm: Compare-and-swap.
            //
            // Semantic (ISA RM §8):
            //   EA = as_ + imm  (decoder pre-shifts imm by 2, so EA = as_ + imm directly)
            //   mem32 = bus.read_u32(EA)
            //   if mem32 == SCOMPARE1: bus.write_u32(EA, at)
            //   at = mem32  (old value always written back to at)
            //
            // Order: read mem first, compare, conditionally write, then update at.
            // For Plan 1 RAM there are no bus read/write side effects, so the order
            // only matters semantically. SCOMPARE1 is read via the SR dispatcher.
            S32c1i { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let mem32 = bus.read_u32(ea)?;
                let scompare = self.sr.read(SCOMPARE1);
                if mem32 == scompare {
                    bus.write_u32(ea, self.regs.read_logical(at))?;
                }
                self.regs.write_logical(at, mem32);
                self.pc = self.pc.wrapping_add(len);
            }

            // L32AI at, as_, imm: Load Acquire Implicit.
            //
            // In Plan 1 (single-core, no SMP) this is identical to L32I.
            // The acquire barrier is a no-op; SMP ordering is deferred to Plan 4.
            // EA = as_ + imm  (decoder pre-shifts imm by 2).
            L32ai { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let val = bus.read_u32(ea)?;
                self.regs.write_logical(at, val);
                self.pc = self.pc.wrapping_add(len);
            }

            // S32RI at, as_, imm: Store Release Implicit.
            //
            // In Plan 1 (single-core, no SMP) this is identical to S32I.
            // The release barrier is a no-op; SMP ordering is deferred to Plan 4.
            // EA = as_ + imm  (decoder pre-shifts imm by 2).
            S32ri { at, as_, imm } => {
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                bus.write_u32(ea, self.regs.read_logical(at))?;
                self.pc = self.pc.wrapping_add(len);
            }

            _ => return Err(SimulationError::NotImplemented(format!("exec: {:?}", ins))),
        }
        Ok(())
    }

    /// Apply branch condition: if taken, jump to `pc + offset` (offset pre-baked with +4);
    /// otherwise advance by `len` bytes.
    #[inline]
    fn branch(&mut self, offset: i32, len: u32, cond: bool) {
        if cond {
            self.pc = self.pc.wrapping_add(offset as u32);
        } else {
            self.pc = self.pc.wrapping_add(len);
        }
    }
}

impl Default for XtensaLx7 {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu for XtensaLx7 {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.regs = ArFile::new();
        // HW-verified: PS reset = 0x1F (EXCM=1, INTLEVEL=0xF — all ints masked).
        // Confirmed via OpenOCD `reset halt` on real S3-Zero: ps = 0x0000001f.
        self.ps = Ps::from_raw(0x1F);
        self.sr = XtensaSrFile::new(); // sets VECBASE=0x40000000, PRID=0xCDCD
        self.pc = 0x4000_0400;
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        _observers: &[Arc<dyn SimulationObserver>],
    ) -> SimResult<()> {
        let pc = self.pc;
        let b0 = bus.read_u8(pc as u64)?;
        let len = xtensa_length::instruction_length(b0);
        let ins = if len == 2 {
            let hw = bus.read_u16(pc as u64)?;
            xtensa_narrow::decode_narrow(hw)
        } else {
            let w = bus.read_u32(pc as u64)?;
            xtensa::decode(w)
        };
        self.execute(ins, bus, len)
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }

    fn set_sp(&mut self, val: u32) {
        // a1 is the stack pointer in the Xtensa windowed ABI.
        self.regs.write_logical(1, val);
    }

    fn set_exception_pending(&mut self, _exception_num: u32) {
        // Phase G implements interrupt dispatch; for Plan 1 this is a no-op.
    }

    fn get_register(&self, id: u8) -> u32 {
        if id < 16 {
            self.regs.read_logical(id)
        } else {
            0
        }
    }

    fn set_register(&mut self, id: u8, val: u32) {
        if id < 16 {
            self.regs.write_logical(id, val);
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot::XtensaLx7(XtensaLx7CpuSnapshot {
            registers: (0u8..16).map(|i| self.regs.read_logical(i)).collect(),
            pc: self.pc,
            ps: self.ps.as_raw(),
            window_base: self.regs.windowbase(),
            window_start: self.regs.windowstart(),
            vecbase: self.sr.read(VECBASE),
        })
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::XtensaLx7(s) = snapshot {
            self.pc = s.pc;
            self.ps = Ps::from_raw(s.ps);
            self.regs.set_windowbase(s.window_base);
            self.regs.set_windowstart(s.window_start);
            for (i, &v) in s.registers.iter().enumerate().take(16) {
                self.regs.write_logical(i as u8, v);
            }
            self.sr.set_raw(VECBASE, s.vecbase);
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        (0..16).map(|i| format!("a{}", i)).collect()
    }
}
