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
use crate::cpu::xtensa_sr::{XtensaSrFile, SAR, VECBASE};
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
        _bus: &mut dyn Bus,
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

            _ => return Err(SimulationError::NotImplemented(format!("exec: {:?}", ins))),
        }
        Ok(())
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
