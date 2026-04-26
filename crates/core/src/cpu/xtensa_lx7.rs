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
use crate::cpu::xtensa_sr::{
    XtensaSrFile, EXCCAUSE, EPC1, EPC2, EPC3, EPC4, EPC5, EPC6, EPC7,
    EPS2, EPS3, EPS4, EPS5, EPS6, EPS7, INTERRUPT, INTENABLE, PS as PS_SR, SAR, SCOMPARE1,
    VECBASE, WINDOWBASE, WINDOWSTART,
};
use crate::decoder::{xtensa, xtensa_length, xtensa_narrow};
use crate::snapshot::{CpuSnapshot, XtensaLx7CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationError, SimulationObserver};
use std::sync::Arc;

/// Offset of _KernelExceptionVector relative to VECBASE on ESP32-S3 LX7.
///
/// Verified against Zephyr soc/xtensa/esp32s3/linker.ld:
///   `. = 0x300; KEEP(*(.KernelExceptionVector.text));`
/// and the ESP-IDF / FreeRTOS ABI which reads PS and EPC1 directly via RSR
/// for level-1 general exceptions (no EPS1 exists in the ESP32-S3 LX7 config).
const KERNEL_VECTOR_OFFSET: u32 = 0x300;

// ── Interrupt vector offsets (VECBASE-relative) ───────────────────────────────
//
// Verified against:
//   ~/.platformio/packages/toolchain-xtensa-esp32s3/xtensa-esp32s3-elf/
//     sys-include/xtensa/config/core-isa.h
// Constants: XCHAL_INTLEVEL{n}_VECOFS, XCHAL_KERNEL_VECOFS, XCHAL_NMI_VECOFS.
//
// Level 1: uses _KernelExceptionVector (XCHAL_KERNEL_VECOFS = 0x300).
//   Level-1 interrupts share the kernel exception vector; EXCCAUSE=4 (Level1Interrupt)
//   distinguishes them from synchronous exceptions in the handler.
// Level 2: XCHAL_INTLEVEL2_VECOFS = 0x180
// Level 3: XCHAL_INTLEVEL3_VECOFS = 0x1C0
// Level 4: XCHAL_INTLEVEL4_VECOFS = 0x200
// Level 5: XCHAL_INTLEVEL5_VECOFS = 0x240
// Level 6: XCHAL_INTLEVEL6_VECOFS = 0x280  (also Debug vector)
// Level 7: XCHAL_NMI_VECOFS       = 0x2C0  (NMI)
const IRQ_VECTOR_OFFSETS: [u32; 8] = [
    0x000,  // level 0: unused (placeholder)
    0x300,  // level 1: XCHAL_KERNEL_VECOFS
    0x180,  // level 2: XCHAL_INTLEVEL2_VECOFS
    0x1C0,  // level 3: XCHAL_INTLEVEL3_VECOFS
    0x200,  // level 4: XCHAL_INTLEVEL4_VECOFS
    0x240,  // level 5: XCHAL_INTLEVEL5_VECOFS
    0x280,  // level 6: XCHAL_INTLEVEL6_VECOFS (Debug)
    0x2C0,  // level 7: XCHAL_NMI_VECOFS
];

// ── IRQ priority table ────────────────────────────────────────────────────────
//
// Fixed interrupt priority levels for the 32 CPU interrupt bits on ESP32-S3 LX7.
//
// Verified against:
//   ~/.platformio/packages/toolchain-xtensa-esp32s3/xtensa-esp32s3-elf/
//     sys-include/xtensa/config/core-isa.h
// Constants: XCHAL_INT{n}_LEVEL for n = 0..31.
//
// Bits 0-10: level 1; bit 11: level 3; bit 12-13: level 1; bit 14: level 7 (NMI);
// bit 15: level 3; bit 16: level 5; bits 17-18: level 1; bits 19-21: level 2;
// bits 22-23: level 3; bit 24: level 4; bit 25: level 4; bit 26: level 5;
// bit 27: level 3; bit 28: level 4; bit 29: level 3; bit 30: level 4; bit 31: level 5.
//
// XCHAL_EXCM_LEVEL = 3: PS.EXCM masks interrupt delivery for levels 1..3.
// Levels 4..7 are "high-priority" and are NOT blocked by EXCM.
pub const IRQ_LEVELS: [u8; 32] = [
    1, 1, 1, 1, 1, 1, 1, 1,  // 0-7
    1, 1, 1, 3, 1, 1, 7, 3,  // 8-15
    5, 1, 1, 2, 2, 2, 3, 3,  // 16-23
    4, 4, 5, 3, 4, 3, 4, 5,  // 24-31
];

/// EXCCAUSE value for Level-1 interrupt entry (ISA RM §4.4.1.5).
const EXCCAUSE_LEVEL1_INTERRUPT: u8 = 4;

/// XCHAL_EXCM_LEVEL: PS.EXCM blocks delivery of interrupts at levels <= this.
/// Verified from core-isa.h: XCHAL_EXCM_LEVEL = 3.
const EXCM_LEVEL: u8 = 3;

pub struct XtensaLx7 {
    pub regs: ArFile,
    pub ps: Ps,
    pub sr: XtensaSrFile,
    /// User-Register file (URs accessed via RUR/WUR). 256 entries; the
    /// commonly-used IDs are THREADPTR (231), FCR (232), FSR (233).
    pub ur: [u32; 256],
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
            ur: [0u32; 256],
            pc: 0x4000_0400,
        }
    }

    /// Read an SR by ID, with special routing for PS / WINDOWBASE / WINDOWSTART
    /// (which live outside `XtensaSrFile`).
    fn read_sr(&self, sr_id: u16) -> u32 {
        match sr_id {
            x if x == PS_SR        => self.ps.as_raw(),
            x if x == WINDOWBASE   => self.regs.windowbase() as u32,
            x if x == WINDOWSTART  => self.regs.windowstart() as u32,
            _ => self.sr.read(sr_id),
        }
    }

    /// Write an SR by ID, with special routing for PS / WINDOWBASE / WINDOWSTART.
    fn write_sr(&mut self, sr_id: u16, val: u32) {
        match sr_id {
            x if x == PS_SR        => { self.ps = Ps::from_raw(val); }
            x if x == WINDOWBASE   => { self.regs.set_windowbase(val as u8); }
            x if x == WINDOWSTART  => { self.regs.set_windowstart(val as u16); }
            _ => self.sr.write(sr_id, val),
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
            Break { imm_s, imm_t } => {
                use crate::peripherals::esp32s3::rom_thunks::{
                    ROM_THUNK_IMM_S, ROM_THUNK_IMM_T,
                };
                if imm_s == ROM_THUNK_IMM_S && imm_t == ROM_THUNK_IMM_T {
                    let pc = self.pc;
                    if let Some(thunk) = bus.get_rom_thunk(pc) {
                        return thunk(self, bus);
                    }
                    return Err(SimulationError::NotImplemented(format!(
                        "ROM thunk at 0x{pc:08x} not registered (BREAK 1,14 with no thunk)"
                    )));
                }
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
            // target = ((pc + 4) & !3) + offset  (ISA RM §4.4; decoder: offset = sext18 * 4)
            //
            // HW-oracle (xtensa-esp-elf-as + objdump):
            //   PC=0, `call0 0x4` → bytes 0x000005 (imm18=0); HW jumps to 0x4.
            //   Formula must give: ((0+4)&!3) + 0 = 4. ✓
            //   Earlier (PC+3)&!3 was used and gave 0+0 = 0 — silently off by 4
            //   for every 4-aligned PC, which broke real ESP32-S3 firmware.
            Call0 { offset } => {
                let ret_pc = self.pc.wrapping_add(3);
                let target = (self.pc.wrapping_add(4) & !3u32).wrapping_add(offset as u32);
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
            // target = ((pc + 4) & !3) + offset  (ISA RM §4.4)
            //
            // See `Call0` above for the HW-oracle proof of the (pc+4) base.
            Call4 { offset } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (1 << 30);
                let target = (self.pc.wrapping_add(4) & !3u32).wrapping_add(offset as u32);
                self.regs.write_logical(4, ret_pc);
                self.ps.set_callinc(1);
                self.pc = target;
            }
            Call8 { offset } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (2 << 30);
                let target = (self.pc.wrapping_add(4) & !3u32).wrapping_add(offset as u32);
                self.regs.write_logical(8, ret_pc);
                self.ps.set_callinc(2);
                self.pc = target;
            }
            Call12 { offset } => {
                let raw_ret = self.pc.wrapping_add(3);
                let ret_pc = (raw_ret & 0x3FFF_FFFF) | (3 << 30);
                let target = (self.pc.wrapping_add(4) & !3u32).wrapping_add(offset as u32);
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

            // ENTRY as_, imm: windowed call prologue with window overflow check (F3).
            //
            // ISA RM §8 ENTRY semantics:
            //   1. WB_new = (WB_old + PS.CALLINC) mod 16
            //   F3: If WindowStart[(WB_new + 1) mod 16] == 1 → WindowOverflow exception:
            //       - EPC1 = PC (the faulting ENTRY's PC)
            //       - PS.EXCM = 1
            //       - PC = VECBASE + window_vector_offset (OF4/OF8/OF12)
            //       - WindowBase NOT rotated, WindowStart NOT modified, CALLINC NOT cleared
            //       - Return immediately (vector handler will deal with the overflow)
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
            // Window vector table (per Xtensa LX ISA RM §5.6, confirmed by Zephyr
            // arch/xtensa/core/window_vectors.S .org directives):
            //   CALLINC=1 (OF4):  VECBASE + 0x000
            //   CALLINC=2 (OF8):  VECBASE + 0x080
            //   CALLINC=3 (OF12): VECBASE + 0x100
            //
            // EXCCAUSE: window overflow exceptions do NOT use EXCCAUSE. They vector
            // independently via dedicated vector slots (not the general exception path).
            // EXCCAUSE values 5/6/7 mean AllocaCause/IntDivByZero/PrivilegedCause.
            Entry { as_, imm } => {
                let callinc = self.ps.callinc();
                let wb_old = self.regs.windowbase();
                let wb_new = wb_old.wrapping_add(callinc) & 0x0F;

                // F3: Window overflow check — check the frame AHEAD of wb_new.
                // ISA RM: if WindowStart[(wb_new + 1) mod 16] == 1, overflow.
                let check_idx = wb_new.wrapping_add(1) & 0x0F;
                if self.regs.windowstart_bit(check_idx) {
                    // Window overflow: save state, redirect to overflow vector.
                    // Window vector offsets (Xtensa LX ISA RM §5.6):
                    //   OF4  (CALLINC=1): VECBASE + 0x000
                    //   OF8  (CALLINC=2): VECBASE + 0x080
                    //   OF12 (CALLINC=3): VECBASE + 0x100
                    const OF4_VECOFS:  u32 = 0x000;
                    const OF8_VECOFS:  u32 = 0x080;
                    const OF12_VECOFS: u32 = 0x100;
                    let vec_ofs = match callinc {
                        1 => OF4_VECOFS,
                        2 => OF8_VECOFS,
                        _ => OF12_VECOFS,  // callinc=3 → OF12; callinc=0 can't overflow
                    };
                    let vecbase = self.sr.read(VECBASE);
                    self.sr.write(EPC1, self.pc);
                    self.ps.set_excm(true);
                    self.pc = vecbase.wrapping_add(vec_ofs);
                    // Do NOT rotate WindowBase, do NOT set WindowStart, do NOT clear CALLINC.
                    return Ok(());
                }

                // Per Xtensa ISA RM §8.1.5 ENTRY:
                //   AR[WB_new*4 + as] = AR[WB_old*4 + as] - imm*8
                // i.e. read the SP from the CALLER's frame, subtract the
                // requested frame size, and write it into the CALLEE's frame
                // — a single value flowing across the window boundary. We
                // were reading post-rotation, which gave the callee an
                // uninitialized AR slot (typically 0) instead of caller's SP,
                // so chained CALL4 calls underflowed SP into 0xffffffXX and
                // every subsequent stack write trapped MemoryViolation.
                let caller_sp = self.regs.read_logical(as_);
                self.regs.set_windowbase(wb_new);
                self.regs.set_windowstart_bit(wb_new, true);
                self.ps.set_callinc(0);
                self.regs.write_logical(as_, caller_sp.wrapping_sub(imm * 8));
                self.pc = self.pc.wrapping_add(len);
            }

            // RETW: windowed return with window underflow check (F4).
            //
            // ISA RM §8 RETW semantics:
            //   1. N = a0[31:30]  (1→CALL4, 2→CALL8, 3→CALL12)
            //   2. wb_dest = (WB_current - N) mod 16
            //   F4: If WindowStart[wb_dest] == 0 → WindowUnderflow exception:
            //       - EPC1 = PC (the faulting RETW's PC)
            //       - PS.EXCM = 1
            //       - PC = VECBASE + window_vector_offset (UF4/UF8/UF12)
            //       - WindowBase NOT rotated, WindowStart NOT modified
            //       - Return immediately (vector handler reloads the spilled frame)
            //   3. target_pc = (a[0] & 0x3FFF_FFFF) | (PC & 0xC000_0000)
            //   4. WindowStart[WB_current] = 0
            //   5. WB = wb_dest
            //   6. PC = target_pc
            //
            // Window underflow vector offsets (Xtensa LX ISA RM §5.6):
            //   N=1 (UF4):  VECBASE + 0x040
            //   N=2 (UF8):  VECBASE + 0x0C0
            //   N=3 (UF12): VECBASE + 0x140
            //
            // EXCCAUSE: window underflow exceptions do NOT use EXCCAUSE. They vector
            // independently via dedicated slots (not the general exception path).
            //
            // N=0 note: RETW with a0[31:30]=0 would indicate a CALL0 return address
            // (which should use RET, not RETW). The wildcard arm in the UF vector
            // match covers N=3; N=0 is treated as N=3 by the same arm, which is
            // benign since CALL0 toolchains never emit RETW. If strict enforcement is
            // needed, add an explicit N=0 → illegal-instruction error here.
            Retw => {
                let a0 = self.regs.read_logical(0);
                let n = (a0 >> 30) as u8;               // bits[31:30] = callinc used by the call
                let wb_cur = self.regs.windowbase();
                let wb_dest = wb_cur.wrapping_sub(n) & 0x0F;

                // F4: Window underflow check — destination frame must be live.
                if !self.regs.windowstart_bit(wb_dest) {
                    // Window underflow vector offsets (Xtensa LX ISA RM §5.6):
                    const UF4_VECOFS:  u32 = 0x040;
                    const UF8_VECOFS:  u32 = 0x0C0;
                    const UF12_VECOFS: u32 = 0x140;
                    let vec_ofs = match n {
                        1 => UF4_VECOFS,
                        2 => UF8_VECOFS,
                        _ => UF12_VECOFS,  // N=3 → UF12; N=0 also lands here (see note above)
                    };
                    let vecbase = self.sr.read(VECBASE);
                    self.sr.write(EPC1, self.pc);
                    self.ps.set_excm(true);
                    self.pc = vecbase.wrapping_add(vec_ofs);
                    // Do NOT rotate WindowBase, do NOT modify WindowStart.
                    return Ok(());
                }

                // Normal RETW path (destination frame is live).
                let target_pc = (a0 & 0x3FFF_FFFF) | (self.pc & 0xC000_0000);
                self.regs.set_windowstart_bit(wb_cur, false);
                self.regs.set_windowbase(wb_dest);
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
                    return self.raise_general_exception(6);
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
                    return self.raise_general_exception(6);
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
                    return self.raise_general_exception(6);
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
                    return self.raise_general_exception(6);
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

            // ── F5: S32E / L32E — windowed exception store/load ──────────────
            //
            // These instructions are only valid when PS.EXCM == 1 (i.e. the CPU
            // is executing inside an exception/interrupt vector). Outside that
            // context they raise an IllegalInstruction exception (EXCCAUSE = 0).
            //
            // EA = as_ + imm  (imm is a pre-computed negative byte offset,
            // stored as two's-complement u32 by the decoder; range -64..-4).
            // Full vector dispatch for the exception path is deferred to Phase G;
            // for now we follow the E2 div-by-zero pattern and return
            // Err(ExceptionRaised { cause: 0 }).

            // S32E at, as_, imm: store at to [as_ + imm], PS.EXCM-gated.
            S32e { at, as_, imm } => {
                if !self.ps.excm() {
                    return self.raise_general_exception(0);
                }
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                bus.write_u32(ea, self.regs.read_logical(at))?;
                self.pc = self.pc.wrapping_add(len);
            }

            // L32E at, as_, imm: load [as_ + imm] into at, PS.EXCM-gated.
            L32e { at, as_, imm } => {
                if !self.ps.excm() {
                    return self.raise_general_exception(0);
                }
                let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
                let v = bus.read_u32(ea)?;
                self.regs.write_logical(at, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // ── F6: MOVSP / ROTW ─────────────────────────────────────────────

            // MOVSP at, as_: move stack pointer between adjacent windowed frames.
            //
            // ISA RM §8 MOVSP semantics (Plan 1 safe-path-only implementation):
            //
            //   The instruction checks whether the *next* windowed frame (WindowBase+1)
            //   is currently live. If WindowStart[(WB+1) & 0xF] == 0 (frame not in use),
            //   this is the safe path: a[at] = a[as_], PC += len.
            //
            //   If the next frame IS in use (WS bit set), the hardware must spill/reload
            //   registers between frames before moving the SP. In Plan 1 we do not model
            //   the spill-to-memory ABI (that belongs in Phase G with full exception
            //   handler emulation). Instead, we raise EXCCAUSE=5 (AllocaCause), which is
            //   the documented exception that MOVSP triggers when it detects a live
            //   adjacent frame (per ISA RM §5.5.4: "MOVSP Window Overflow/Underflow").
            //
            // TODO(plan2): implement the full spill path: when WS[(WB+1)&0xF] is set,
            //   save a[(WB+1)*4 .. (WB+1)*4+3] to memory at [a[at]-16..a[at]-4], then
            //   perform the move, then restore from the new SP. This matches the
            //   __window_spill / alloca vector handler ABI used by GCC/ESP-IDF.
            Movsp { at, as_ } => {
                let wb = self.regs.windowbase();
                let next_idx = wb.wrapping_add(1) & 0x0F;

                if self.regs.windowstart_bit(next_idx) {
                    // Adjacent frame is live — spill path required.
                    // Plan 1: raise AllocaCause (EXCCAUSE=5) via the general exception path.
                    // TODO(plan2): implement register spill instead.
                    return self.raise_general_exception(5);
                }

                // Safe path: adjacent frame is free, simple register move.
                let v = self.regs.read_logical(as_);
                self.regs.write_logical(at, v);
                self.pc = self.pc.wrapping_add(len);
            }

            // ROTW n: rotate WindowBase by n (4-bit signed, range -8..=+7).
            //
            // ISA RM §8 ROTW semantics:
            //   WindowBase = (WindowBase + n) mod 16
            //   WindowStart is NOT modified.
            //
            // Privileged note: ROTW is a privileged instruction (valid only when
            // PS.RING == 0). Plan 1 does not model PS.RING (we always run at ring 0),
            // so the ring check is skipped.
            //
            // TODO(plan-priv): when PS.RING modelling is added, add a check here:
            //   if ps.ring() != 0 { raise PrivilegedCause (EXCCAUSE=8) }
            Rotw { n } => {
                let wb = self.regs.windowbase();
                // n is i8 (range -8..=+7); wrapping add modulo 16.
                let wb_new = (wb as i32).wrapping_add(n as i32).rem_euclid(16) as u8;
                self.regs.set_windowbase(wb_new);
                // WindowStart is NOT modified (ISA RM §8 ROTW).
                self.pc = self.pc.wrapping_add(len);
            }

            // ── G2: Exception / interrupt return instructions ─────────────────

            // RFE — Return From (level-1 general) Exception.
            //
            // ESP32-S3 LX7 ISA RM §4.4.2 / §8:
            //   PS.EXCM ← 0
            //   PS.INTLEVEL is left unchanged (not reset).
            //   PC ← EPC[1]
            //
            // Note: EPS1 does NOT exist on LX7 (the assembler rejects `rsr.eps1`).
            // Level-1 exceptions save PS in-place; the handler reads/modifies PS
            // directly via RSR/WSR. Only EXCM is cleared by RFE — INTLEVEL is left
            // to the handler to restore explicitly.
            Rfe => {
                self.ps.set_excm(false);
                self.pc = self.sr.read(EPC1);
            }

            // RFDE — Return From Debug Exception. Handled same as RFE for Plan 1:
            // clear PS.EXCM, jump to EPC1. Full DEPC/debug-exception semantics are
            // deferred to a later plan.
            Rfde => {
                self.ps.set_excm(false);
                self.pc = self.sr.read(EPC1);
            }

            // RFI n — Return From Interrupt at level n (n = 2..7 on LX7).
            //
            // ISA RM §4.4.3 / §8 RFI:
            //   PS ← EPS[n]   (restore full PS from saved copy)
            //   PC ← EPC[n]
            //
            // LX7 EPC/EPS SR IDs (hardware-verified, C2 table):
            //   EPC2..EPC7 = SR IDs 178..183
            //   EPS2..EPS7 = SR IDs 194..199
            //
            // Level 1 uses RFE (no EPS1 on LX7). Levels 2..7 use RFI. Level 0
            // and 1 are not valid targets for RFI on LX7; we silently treat them
            // as no-ops (stay at current PC, no state change) since privileged
            // firmware is the only caller and should not issue invalid RFI levels.
            Rfi { level } => {
                let (eps_id, epc_id) = match level {
                    2 => (EPS2, EPC2),
                    3 => (EPS3, EPC3),
                    4 => (EPS4, EPC4),
                    5 => (EPS5, EPC5),
                    6 => (EPS6, EPC6),
                    7 => (EPS7, EPC7),
                    _ => {
                        // Invalid level — skip silently.
                        self.pc = self.pc.wrapping_add(len);
                        return Ok(());
                    }
                };
                let new_ps = self.sr.read(eps_id);
                let new_pc = self.sr.read(epc_id);
                self.ps = Ps::from_raw(new_ps);
                self.pc = new_pc;
            }

            // RFWO — Return From Window Overflow handler.
            //
            // Called at the end of the WindowOverflow vector handler after the
            // overflowed frame's registers have been spilled to the stack.
            //
            // ISA RM §4.4.5 / §8 RFWO (canonical hardware semantics):
            //   s = WindowBase  (save old WB)
            //   WindowBase ← (WindowBase + PS.CALLINC) mod NWINDOWS
            //   WindowStart[s] ← 0     (clear the old frame's WS bit — it's spilled)
            //   PS.EXCM ← 0
            //   PC ← EPC1
            //
            // Note on our exception-entry model: our ENTRY-overflow handler does NOT
            // rotate WB on exception entry (WB stays at wb_old). The canonical
            // hardware DOES rotate WB to call[j] on overflow entry, then RFWO
            // advances to call[i]. In our model, WB = call[i]'s position already
            // (since ENTRY fired before completing its rotation), so RFWO here
            // performs the rotation that ENTRY would have done: WB += CALLINC.
            //
            // After RFWO, EPC1 points back to the ENTRY instruction which will
            // re-execute. The re-execution succeeds because the overflow frame was
            // spilled (PS.EXCM=0, WS bit cleared by the handler via normal stores).
            Rfwo => {
                let wb_old = self.regs.windowbase();
                let callinc = self.ps.callinc();
                let wb_new = wb_old.wrapping_add(callinc) & 0x0F;
                self.regs.set_windowstart_bit(wb_old, false);  // clear spilled frame
                self.regs.set_windowbase(wb_new);
                self.regs.set_windowstart_bit(wb_new, true);   // new frame is live
                self.ps.set_excm(false);
                self.pc = self.sr.read(EPC1);
            }

            // RFWU — Return From Window Underflow handler.
            //
            // Called at the end of the WindowUnderflow vector handler after the
            // underflowed frame's registers have been reloaded from the stack.
            //
            // ISA RM §4.4.5 / §8 RFWU (canonical hardware semantics):
            //   WindowBase ← (WindowBase - 1) mod NWINDOWS
            //   s = WindowBase  (new WB)
            //   WindowStart[s] ← 1     (mark the reloaded frame live)
            //   PS.EXCM ← 0
            //   PC ← EPC1
            //
            // Note on our exception-entry model: our RETW-underflow handler does NOT
            // rotate WB on exception entry (WB stays at wb_cur = callee). The
            // canonical hardware rotates WB to call[i] (= wb_dest) before entering
            // the UF handler. In our model:
            //   - For UF4: hardware would rotate by -1; RFWU -1 gives total -1.
            //   - For UF8/12: hardware would rotate by -N; RFWU gives total -N-1.
            // This is a known discrepancy in the Plan 1 model. UF8/12 are not
            // exercised in Plan 1 tests; UF4 is the primary case.
            //
            // After RFWU, EPC1 points to the RETW instruction which will re-execute.
            // The re-execution succeeds because WS[wb_dest] is now set.
            Rfwu => {
                let wb_old = self.regs.windowbase();
                let wb_new = wb_old.wrapping_sub(1) & 0x0F;
                self.regs.set_windowbase(wb_new);
                self.regs.set_windowstart_bit(wb_new, true);   // reloaded frame is live
                self.ps.set_excm(false);
                self.pc = self.sr.read(EPC1);
            }

            // ── G3: Special-Register / User-Register access ──────────────────
            //
            // RSR at, sr / WSR at, sr / XSR at, sr — atomic read/write/swap of
            // an SR. The SR file holds most SRs; PS, WINDOWBASE, WINDOWSTART
            // live elsewhere on the CPU and route through `read_sr`/`write_sr`.
            //
            // Per ISA RM §5.5, RSR/WSR for unimplemented SRs raise an
            // IllegalInstructionCause exception. We follow Plan 1 policy of
            // permissive-zero for unknown SRs (read returns 0; write is a NOP)
            // because most ESP-IDF / esp-hal startup code reads/writes SRs that
            // we model only as storage (no behavioural side-effects). Genuine
            // privilege checks (PS.RING) are not enforced here because all
            // firmware we simulate runs in ring 0.
            Rsr { at, sr } => {
                let v = self.read_sr(sr);
                self.regs.write_logical(at, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Wsr { at, sr } => {
                let v = self.regs.read_logical(at);
                self.write_sr(sr, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Xsr { at, sr } => {
                let new_v = self.regs.read_logical(at);
                let old_v = self.read_sr(sr);
                self.write_sr(sr, new_v);
                self.regs.write_logical(at, old_v);
                self.pc = self.pc.wrapping_add(len);
            }
            // RUR ar, ur / WUR at, ur — User-Register read/write. URs are a
            // separate 8-bit-ID space from SRs; we model them as a simple
            // [u32; 256] storage array. The commonly-used URs are THREADPTR
            // (231), FCR (232), FSR (233). Floating-point semantics of FCR/FSR
            // are not modeled; they roundtrip as plain storage.
            Rur { ar, ur } => {
                let v = self.ur[(ur as usize) & 0xFF];
                self.regs.write_logical(ar, v);
                self.pc = self.pc.wrapping_add(len);
            }
            Wur { at, ur } => {
                let v = self.regs.read_logical(at);
                self.ur[(ur as usize) & 0xFF] = v;
                self.pc = self.pc.wrapping_add(len);
            }

            // EXTUI ar, at, shift, bits: ar = (at >> shift) & ((1<<bits)-1).
            // bits ∈ 1..=16, shift ∈ 0..=31. The mask wraps cleanly because
            // `1u32 << 16` is well-defined; for bits=16 we use 0xFFFF.
            Extui { ar, at, shift, bits } => {
                let v = self.regs.read_logical(at);
                let mask: u32 = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
                let extracted = (v >> shift) & mask;
                self.regs.write_logical(ar, extracted);
                self.pc = self.pc.wrapping_add(len);
            }

            // RSIL at, level: atomic { at = PS; PS.INTLEVEL = level; }.
            //
            // Used by esp-hal critical sections to mask interrupts up to a
            // given priority, returning the previous PS so a later WSR.PS
            // can restore it. Per ISA RM the only PS bits modified are
            // INTLEVEL[3:0]; EXCM/UM/CALLINC/etc. are preserved.
            Rsil { at, level } => {
                let prev_ps = self.ps.as_raw();
                self.regs.write_logical(at, prev_ps);
                let new_ps = (prev_ps & !0xF) | (level as u32 & 0xF);
                self.ps = Ps::from_raw(new_ps);
                self.pc = self.pc.wrapping_add(len);
            }

            // Unknown opcode: raise IllegalInstruction (EXCCAUSE=0).
            //
            // Xtensa LX7 ISA RM §5.2: executing an instruction not defined in the
            // ISA raises a general exception with EXCCAUSE=0 (IllegalInstruction).
            // This is the Plan-1 digital-twin guarantee: any byte pattern decoded
            // as Unknown by the decode layer faithfully raises EXCCAUSE=0, matching
            // real ESP32-S3 hardware behaviour.
            Unknown(_) => return self.raise_general_exception(0),

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

    // ── Interrupt dispatch helpers ────────────────────────────────────────────

    /// Compute the highest priority level of any pending-and-enabled interrupt.
    ///
    /// Returns `Some(level)` if `(INTERRUPT & INTENABLE) != 0`, else `None`.
    /// The level is the maximum over all set bits in the masked-pending word,
    /// using `IRQ_LEVELS` indexed by bit position.
    fn pending_irq_level(&self, bus: &dyn Bus) -> Option<u8> {
        // Plan 3: aggregate pending IRQs from two sources:
        //   1. SR-file INTERRUPT register (firmware can software-trigger via WSR).
        //   2. Bus's pending_cpu_irqs (peripheral source IDs routed through
        //      the ESP32-S3 intmatrix in tick_peripherals_with_costs).
        let pending = (self.sr.read(INTERRUPT) | bus.pending_cpu_irqs()) & self.sr.read(INTENABLE);
        if pending == 0 {
            return None;
        }
        let max_level = (0u8..32)
            .filter(|&bit| (pending >> bit) & 1 == 1)
            .map(|bit| IRQ_LEVELS[bit as usize])
            .max()?;
        Some(max_level)
    }

    /// Dispatch an interrupt at the given priority `level`.
    ///
    /// Implements Xtensa LX ISA RM §4.4.1 "Interrupt Entry" for ESP32-S3 LX7:
    ///
    /// **Level 1** (uses kernel exception vector, shares with general exceptions):
    ///   1. EPC1     ← PC
    ///   2. EXCCAUSE ← 4 (Level1InterruptCause)
    ///   3. PS.EXCM  ← 1  (PS.INTLEVEL unchanged)
    ///   4. PC       ← VECBASE + 0x300
    ///
    /// **Levels 2..7** (dedicated high-priority interrupt vectors):
    ///   1. EPC[level] ← PC
    ///   2. EPS[level] ← PS
    ///   3. PS.INTLEVEL ← level  (PS.EXCM cleared for level > EXCM_LEVEL, unchanged otherwise)
    ///   4. PC         ← VECBASE + IRQ_VECTOR_OFFSETS[level]
    ///
    /// For levels 2..EXCM_LEVEL (2..3 on ESP32-S3), the Xtensa ISA specifies
    /// that PS.EXCM is set to 1 on entry (medium-priority interrupt entry behaves
    /// like exception entry). For levels > EXCM_LEVEL (4..7), PS.EXCM is cleared
    /// (high-priority: only INTLEVEL gates further interrupts).
    ///
    /// Returns `Ok(())` — unlike `raise_general_exception`, interrupt dispatch
    /// is not an error; the CPU simply redirects to the ISR vector.
    fn dispatch_irq(&mut self, level: u8, bus: &mut dyn Bus) -> SimResult<()> {
        let entry_pc = self.pc;
        let vecbase = self.sr.read(VECBASE);
        let vector_offset = IRQ_VECTOR_OFFSETS[level.min(7) as usize];

        if level == 1 {
            // Level-1 interrupt: kernel vector, EXCM=1, no EPS1.
            self.sr.write(EPC1, entry_pc);
            self.sr.write(EXCCAUSE, EXCCAUSE_LEVEL1_INTERRUPT as u32);
            self.ps.set_excm(true);
            self.pc = vecbase.wrapping_add(vector_offset);
        } else {
            // Level 2..7: dedicated interrupt vector.
            // Save PC and PS into EPC[level]/EPS[level].
            let saved_ps = self.ps.as_raw();
            let epc_sr = [0u16, EPC1, EPC2, EPC3, EPC4, EPC5, EPC6, EPC7];
            let eps_sr = [0u16, 0u16, EPS2, EPS3, EPS4, EPS5, EPS6, EPS7];
            let l = level as usize;
            self.sr.write(epc_sr[l], entry_pc);
            self.sr.write(eps_sr[l], saved_ps);

            // Update PS: set INTLEVEL to the dispatched level.
            // For medium-priority (level <= EXCM_LEVEL): also set EXCM=1.
            // For high-priority (level > EXCM_LEVEL): clear EXCM.
            let mut new_ps = self.ps;
            new_ps.set_intlevel(level);
            if level <= EXCM_LEVEL {
                new_ps.set_excm(true);
            } else {
                new_ps.set_excm(false);
            }
            self.ps = new_ps;
            self.pc = vecbase.wrapping_add(vector_offset);
        }

        // Plan 3: clear the bus-side pending bits at this level so we don't
        // re-fire next tick. The firmware ISR is responsible for clearing
        // the underlying source-side pending bit (INT_CLR on the peripheral)
        // before the source re-asserts.
        for slot in 0..32u8 {
            if IRQ_LEVELS[slot as usize] == level {
                bus.clear_cpu_irq_pending(slot);
            }
        }

        Ok(())
    }

    /// Raise a level-1 general exception (kernel vector).
    ///
    /// Implements Xtensa LX ISA RM §5.5 "General Exception" for ESP32-S3 LX7:
    ///   1. EPC1  ← PC (pre-advance; the faulting instruction's address).
    ///   2. EXCCAUSE ← cause.
    ///   3. PS.EXCM  ← 1  (masks interrupts; PS.INTLEVEL is left unchanged).
    ///   4. PC       ← VECBASE + 0x300 (_KernelExceptionVector).
    ///
    /// Note: ESP32-S3 LX7 does NOT implement EPS1 (the assembler rejects
    /// `rsr.eps1`). For level-1 exceptions, the exception handler reads PS
    /// directly via `rsr.ps` after entry. Window OF/UF exceptions do NOT use
    /// this helper — they have dedicated vector slots and different entry rules.
    ///
    /// Returns `Err(ExceptionRaised { cause, pc: EPC1 })` so callers (and tests)
    /// know a general exception was taken while the simulator state is consistent.
    fn raise_general_exception(&mut self, cause: u8) -> SimResult<()> {
        let faulting_pc = self.pc;
        self.sr.write(EPC1, faulting_pc);
        self.sr.write(EXCCAUSE, cause as u32);
        self.ps.set_excm(true);
        let vecbase = self.sr.read(VECBASE);
        self.pc = vecbase.wrapping_add(KERNEL_VECTOR_OFFSET);
        Err(SimulationError::ExceptionRaised { cause, pc: faulting_pc })
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
        // ── Pre-fetch interrupt check ─────────────────────────────────────────
        // Per Xtensa ISA RM §4.4.1: check for pending interrupts before fetching
        // the next instruction.
        //
        // Dispatch conditions (all must be true):
        //   1. PS.EXCM == 0  (if EXCM=1, even high-priority ints are blocked for
        //                     medium levels; for high-priority levels EXCM is set
        //                     to 0 on entry, but we still gate on it here to avoid
        //                     re-entry from within a level-1 handler).
        //   2. (INTERRUPT & INTENABLE) != 0
        //   3. highest_pending_level > PS.INTLEVEL
        //
        // Note: INTENABLE defaults to 0 at reset, so existing tests are unaffected.
        if !self.ps.excm() {
            if let Some(irq_level) = self.pending_irq_level(bus) {
                if irq_level > self.ps.intlevel() {
                    return self.dispatch_irq(irq_level, bus);
                }
            }
        }

        let pc = self.pc;
        let b0 = bus.read_u8(pc as u64)?;
        let len = xtensa_length::instruction_length(b0);

        // S32E/L32E exception: these are 24-bit wide instructions whose op0 field
        // (bits[3:0] of the 24-bit word, i.e. byte0's low nibble) equals 0x9 — the
        // same value used by the narrow S32I.N density instruction. They cannot be
        // distinguished from narrow instructions by byte0 alone.
        //
        // CRITICAL: S32E/L32E are architecturally only valid inside exception
        // context (PS.EXCM=1). Outside EXCM, op0=0x9 must be treated as narrow
        // S32I.N, NOT as the start of a 3-byte wide instruction. Without this
        // gate, s32i.n a0, a1, 0 (bytes 0x09, 0x10) followed by any QRST op
        // (byte0 ending in 0x0, e.g. ADD, OR, MOVI, NOP) would speculatively
        // read byte2 and falsely match the L32E pattern, corrupting the PC
        // advance from 2 to 3.
        //
        // Encoding invariants (HW-oracle verified):
        //   S32E: byte0 bits[7:4] = 0x4 (subop), byte0 bits[3:0] = 0x9 (op0)
        //   L32E: byte0 bits[7:4] = 0x0 (subop), byte0 bits[3:0] = 0x9 (op0)
        //   byte2 bits[3:0] = 0x0 (op1 field is always 0 for both)
        //
        // We read all 3 bytes speculatively when byte0 matches, check byte2's
        // low nibble, and route to the wide decoder when confirmed.
        let is_s32e_or_l32e = if self.ps.excm() && len == 2 && (b0 & 0x0F) == 0x9 {
            // Speculatively read byte2.  Bus reads are non-destructive so this
            // is safe even if the instruction turns out to be 2-byte narrow.
            let b2 = bus.read_u8(pc as u64 + 2)?;
            let subop = (b0 >> 4) & 0xF;
            // subop=4 → S32E, subop=0 → L32E; byte2 low nibble must be 0 (op1=0).
            (subop == 4 || subop == 0) && (b2 & 0x0F) == 0
        } else {
            false
        };

        let ins = if is_s32e_or_l32e {
            let w = bus.read_u32(pc as u64)?;
            xtensa::decode(w)
        } else if len == 2 {
            let hw = bus.read_u16(pc as u64)?;
            xtensa_narrow::decode_narrow(hw)
        } else {
            let w = bus.read_u32(pc as u64)?;
            xtensa::decode(w)
        };
        let effective_len = if is_s32e_or_l32e { 3 } else { len };
        self.execute(ins, bus, effective_len)
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
