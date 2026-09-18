// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arm bodies of `XtensaLx7::execute` for the load_store instruction class,
//! moved here verbatim. `execute` keeps the single `match ins`; each arm
//! calls one of these `#[inline(always)]` methods.

use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::{Bus, SimResult};

use crate::cpu::xtensa_sr::SCOMPARE1;
impl XtensaLx7 {
    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_l8ui(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
        let val = bus.read_u8(ea)? as u32;
        self.regs.write_logical(at, val);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_l16ui(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
        let val = bus.read_u16(ea)? as u32;
        self.regs.write_logical(at, val);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_l16si(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
        let raw = bus.read_u16(ea)?;
        // Sign-extend 16-bit: cast to i16 then to i32, reinterpret as u32.
        let val = (raw as i16) as i32 as u32;
        self.regs.write_logical(at, val);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_l32i(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
        let val = bus.read_u32(ea)?;
        self.regs.write_logical(at, val);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_l32r(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        pc_rel_byte_offset: i32,
    ) -> SimResult<()> {
        let base = (self.pc.wrapping_add(3)) & !3u32;
        let ea = base.wrapping_add(pc_rel_byte_offset as u32) as u64;
        let val = bus.read_u32(ea)?;
        self.regs.write_logical(at, val);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_s8i(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let ea = self.regs.read_logical(as_).wrapping_add(imm);
        self.maybe_invalidate_for_write(ea);
        bus.write_u8(ea as u64, (self.regs.read_logical(at) & 0xFF) as u8)?;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_s16i(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let ea = self.regs.read_logical(as_).wrapping_add(imm);
        self.maybe_invalidate_for_write(ea);
        bus.write_u16(ea as u64, (self.regs.read_logical(at) & 0xFFFF) as u16)?;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_s32i(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let ea = self.regs.read_logical(as_).wrapping_add(imm);
        self.maybe_invalidate_for_write(ea);
        bus.write_u32(ea as u64, self.regs.read_logical(at))?;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_s32c1i(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let ea = self.regs.read_logical(as_).wrapping_add(imm);
        let mem32 = bus.read_u32(ea as u64)?;
        let scompare = self.sr.read(SCOMPARE1);
        if mem32 == scompare {
            self.maybe_invalidate_for_write(ea);
            bus.write_u32(ea as u64, self.regs.read_logical(at))?;
        }
        self.regs.write_logical(at, mem32);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_l32ai(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
        let val = bus.read_u32(ea)?;
        self.regs.write_logical(at, val);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_s32ri(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let ea = self.regs.read_logical(as_).wrapping_add(imm);
        self.maybe_invalidate_for_write(ea);
        bus.write_u32(ea as u64, self.regs.read_logical(at))?;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_s32e(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        if !self.ps.excm() && self.ps.ring() != 0 {
            return self.raise_general_exception(0);
        }
        let ea = self.regs.read_logical(as_).wrapping_add(imm);
        self.maybe_invalidate_for_write(ea);
        bus.write_u32(ea as u64, self.regs.read_logical(at))?;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_l32e(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        if !self.ps.excm() && self.ps.ring() != 0 {
            return self.raise_general_exception(0);
        }
        let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
        let v = bus.read_u32(ea)?;
        self.regs.write_logical(at, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }
}
