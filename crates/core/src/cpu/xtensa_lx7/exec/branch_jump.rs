// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arm bodies of `XtensaLx7::execute` for the branch_jump instruction class,
//! moved here verbatim. `execute` keeps the single `match ins`; each arm
//! calls one of these `#[inline(always)]` methods.

use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::{Bus, SimResult};
impl XtensaLx7 {
    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_beq(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        at: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = self.regs.read_logical(as_) == self.regs.read_logical(at);
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bne(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        at: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = self.regs.read_logical(as_) != self.regs.read_logical(at);
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_blt(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        at: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = (self.regs.read_logical(as_) as i32) < (self.regs.read_logical(at) as i32);
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bge(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        at: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = (self.regs.read_logical(as_) as i32) >= (self.regs.read_logical(at) as i32);
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bltu(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        at: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = self.regs.read_logical(as_) < self.regs.read_logical(at);
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bgeu(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        at: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = self.regs.read_logical(as_) >= self.regs.read_logical(at);
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_j(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
        offset: i32,
    ) -> SimResult<()> {
        self.pc = self.pc.wrapping_add(offset as u32);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_jx(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
        as_: u8,
    ) -> SimResult<()> {
        self.pc = self.regs.read_logical(as_);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bany(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        at: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = (self.regs.read_logical(as_) & self.regs.read_logical(at)) != 0;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ball(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        at: u8,
        offset: i32,
    ) -> SimResult<()> {
        let a = self.regs.read_logical(as_);
        let b = self.regs.read_logical(at);
        let cond = (a & b) == b;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bnone(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        at: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = (self.regs.read_logical(as_) & self.regs.read_logical(at)) == 0;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bnall(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        at: u8,
        offset: i32,
    ) -> SimResult<()> {
        let a = self.regs.read_logical(as_);
        let b = self.regs.read_logical(at);
        let cond = (a & b) != b;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bbc(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        at: u8,
        offset: i32,
    ) -> SimResult<()> {
        let bit = self.regs.read_logical(at) & 0x1F;
        let cond = (self.regs.read_logical(as_) >> bit) & 1 == 0;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bbs(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        at: u8,
        offset: i32,
    ) -> SimResult<()> {
        let bit = self.regs.read_logical(at) & 0x1F;
        let cond = (self.regs.read_logical(as_) >> bit) & 1 == 1;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bbci(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        bit: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = (self.regs.read_logical(as_) >> bit) & 1 == 0;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bbsi(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        bit: u8,
        offset: i32,
    ) -> SimResult<()> {
        let val = self.regs.read_logical(as_);
        let cond = (val >> bit) & 1 == 1;
        if std::env::var_os("LABWIRED_TRACE_BBSI").is_some() && self.pc == 0x400ed00d {
            eprintln!(
                "[trace] BBSI at pc=0x{:08x} as_=a{} val=0x{:08x} bit={} cond={}",
                self.pc, as_, val, bit, cond
            );
        }
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_beqz(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = self.regs.read_logical(as_) == 0;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bnez(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = self.regs.read_logical(as_) != 0;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bltz(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = (self.regs.read_logical(as_) as i32) < 0;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bgez(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = (self.regs.read_logical(as_) as i32) >= 0;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bt(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        bs: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = (self.br >> (bs & 0xF)) & 1 == 1;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bf(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        bs: u8,
        offset: i32,
    ) -> SimResult<()> {
        let cond = (self.br >> (bs & 0xF)) & 1 == 0;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_beqi(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        imm: i32,
        offset: i32,
    ) -> SimResult<()> {
        let cond = (self.regs.read_logical(as_) as i32) == imm;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bnei(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        imm: i32,
        offset: i32,
    ) -> SimResult<()> {
        let cond = (self.regs.read_logical(as_) as i32) != imm;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_blti(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        imm: i32,
        offset: i32,
    ) -> SimResult<()> {
        let cond = (self.regs.read_logical(as_) as i32) < imm;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bgei(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        imm: i32,
        offset: i32,
    ) -> SimResult<()> {
        let cond = (self.regs.read_logical(as_) as i32) >= imm;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bltui(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        imm: u32,
        offset: i32,
    ) -> SimResult<()> {
        let cond = self.regs.read_logical(as_) < imm;
        self.branch(offset, len, cond);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_bgeui(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        imm: u32,
        offset: i32,
    ) -> SimResult<()> {
        let cond = self.regs.read_logical(as_) >= imm;
        self.branch(offset, len, cond);
        Ok(())
    }
}
