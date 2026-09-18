// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arm bodies of `XtensaLx7::execute` for the alu instruction class,
//! moved here verbatim. `execute` keeps the single `match ins`; each arm
//! calls one of these `#[inline(always)]` methods.

use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::{Bus, SimResult};
impl XtensaLx7 {
    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_add(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let v = self
            .regs
            .read_logical(as_)
            .wrapping_add(self.regs.read_logical(at));
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_sub(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let v = self
            .regs
            .read_logical(as_)
            .wrapping_sub(self.regs.read_logical(at));
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_and(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let v = self.regs.read_logical(as_) & self.regs.read_logical(at);
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_or(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let v = self.regs.read_logical(as_) | self.regs.read_logical(at);
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_xor(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let v = self.regs.read_logical(as_) ^ self.regs.read_logical(at);
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_neg(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        at: u8,
    ) -> SimResult<()> {
        let v = 0u32.wrapping_sub(self.regs.read_logical(at));
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_abs(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        at: u8,
    ) -> SimResult<()> {
        // ISA RM: result is unsigned abs of the 2's-complement value.
        // i32::unsigned_abs() returns 0x80000000 for i32::MIN — matches HW behaviour.
        let x = self.regs.read_logical(at) as i32;
        self.regs.write_logical(ar, x.unsigned_abs());
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_addx2(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let v = (self.regs.read_logical(as_) << 1).wrapping_add(self.regs.read_logical(at));
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_addx4(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let v = (self.regs.read_logical(as_) << 2).wrapping_add(self.regs.read_logical(at));
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_addx8(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let v = (self.regs.read_logical(as_) << 3).wrapping_add(self.regs.read_logical(at));
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_subx2(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let v = (self.regs.read_logical(as_) << 1).wrapping_sub(self.regs.read_logical(at));
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_subx4(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let v = (self.regs.read_logical(as_) << 2).wrapping_sub(self.regs.read_logical(at));
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_subx8(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let v = (self.regs.read_logical(as_) << 3).wrapping_sub(self.regs.read_logical(at));
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_movi(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        at: u8,
        imm: i32,
    ) -> SimResult<()> {
        self.regs.write_logical(at, imm as u32);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_moveqz(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        if self.regs.read_logical(at) == 0 {
            let v = self.regs.read_logical(as_);
            self.regs.write_logical(ar, v);
        }
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_movnez(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        if self.regs.read_logical(at) != 0 {
            let v = self.regs.read_logical(as_);
            self.regs.write_logical(ar, v);
        }
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_movltz(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        if (self.regs.read_logical(at) as i32) < 0 {
            let v = self.regs.read_logical(as_);
            self.regs.write_logical(ar, v);
        }
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_movgez(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        if (self.regs.read_logical(at) as i32) >= 0 {
            let v = self.regs.read_logical(as_);
            self.regs.write_logical(ar, v);
        }
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_addi(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm8: i32,
    ) -> SimResult<()> {
        let v = self.regs.read_logical(as_).wrapping_add(imm8 as u32);
        self.regs.write_logical(at, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_addmi(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm: i32,
    ) -> SimResult<()> {
        let v = self.regs.read_logical(as_).wrapping_add(imm as u32);
        self.regs.write_logical(at, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_salt(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let a = self.regs.read_logical(as_) as i32;
        let b = self.regs.read_logical(at) as i32;
        self.regs.write_logical(ar, u32::from(a < b));
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_saltu(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let a = self.regs.read_logical(as_);
        let b = self.regs.read_logical(at);
        self.regs.write_logical(ar, u32::from(a < b));
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_nsa(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
    ) -> SimResult<()> {
        let src = self.regs.read_logical(as_);
        let count = if (src as i32) >= 0 {
            src.leading_zeros()
        } else {
            (!src).leading_zeros()
        };
        self.regs.write_logical(ar, count - 1);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_nsau(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
    ) -> SimResult<()> {
        let src = self.regs.read_logical(as_);
        self.regs.write_logical(ar, src.leading_zeros());
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_min(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let a = self.regs.read_logical(as_) as i32;
        let b = self.regs.read_logical(at) as i32;
        self.regs.write_logical(ar, a.min(b) as u32);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_max(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let a = self.regs.read_logical(as_) as i32;
        let b = self.regs.read_logical(at) as i32;
        self.regs.write_logical(ar, a.max(b) as u32);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_minu(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let a = self.regs.read_logical(as_);
        let b = self.regs.read_logical(at);
        self.regs.write_logical(ar, a.min(b));
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_maxu(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let a = self.regs.read_logical(as_);
        let b = self.regs.read_logical(at);
        self.regs.write_logical(ar, a.max(b));
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_sext(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        sa: u8,
    ) -> SimResult<()> {
        let src = self.regs.read_logical(as_);
        let shift = 31 - sa; // sa is 7..=22, shift is 9..=24
        let v = ((src as i32) << shift >> shift) as u32;
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_clamps(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        sa: u8,
    ) -> SimResult<()> {
        let src = self.regs.read_logical(as_) as i32;
        let max_val = (1i32 << sa) - 1;
        let min_val = -(1i32 << sa);
        let v = src.clamp(min_val, max_val);
        self.regs.write_logical(ar, v as u32);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_extui(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        at: u8,
        shift: u8,
        bits: u8,
    ) -> SimResult<()> {
        let v = self.regs.read_logical(at);
        let mask: u32 = if bits >= 32 {
            u32::MAX
        } else {
            (1u32 << bits) - 1
        };
        let extracted = (v >> shift) & mask;
        self.regs.write_logical(ar, extracted);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }
}
