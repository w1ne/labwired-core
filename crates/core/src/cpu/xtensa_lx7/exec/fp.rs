// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arm bodies of `XtensaLx7::execute` for the fp instruction class,
//! moved here verbatim. `execute` keeps the single `match ins`; each arm
//! calls one of these `#[inline(always)]` methods.

use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::{Bus, SimResult};

use super::super::round_half_even;
use crate::decoder::xtensa;
use crate::decoder::xtensa::FpCmp;
impl XtensaLx7 {
    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_add_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
        ft: u8,
    ) -> SimResult<()> {
        let v = self.fget(fs) + self.fget(ft);
        self.fset(fr, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_sub_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
        ft: u8,
    ) -> SimResult<()> {
        let v = self.fget(fs) - self.fget(ft);
        self.fset(fr, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_mul_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
        ft: u8,
    ) -> SimResult<()> {
        let v = self.fget(fs) * self.fget(ft);
        self.fset(fr, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_madd_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
        ft: u8,
    ) -> SimResult<()> {
        let v = self.fget(fr) + self.fget(fs) * self.fget(ft);
        self.fset(fr, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_msub_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
        ft: u8,
    ) -> SimResult<()> {
        let v = self.fget(fr) - self.fget(fs) * self.fget(ft);
        self.fset(fr, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_abs_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
    ) -> SimResult<()> {
        let v = self.fp[(fs & 0xF) as usize] & 0x7FFF_FFFF;
        self.fp[(fr & 0xF) as usize] = v;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_neg_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
    ) -> SimResult<()> {
        let v = self.fp[(fs & 0xF) as usize] ^ 0x8000_0000;
        self.fp[(fr & 0xF) as usize] = v;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_mov_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
    ) -> SimResult<()> {
        self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_rfr(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        fs: u8,
    ) -> SimResult<()> {
        let v = self.fp[(fs & 0xF) as usize];
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_wfr(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        as_: u8,
    ) -> SimResult<()> {
        let v = self.regs.read_logical(as_);
        self.fp[(fr & 0xF) as usize] = v;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_float_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        as_: u8,
        imm: u8,
    ) -> SimResult<()> {
        let x = self.regs.read_logical(as_) as i32 as f32;
        let v = x / (1u32 << imm) as f32;
        self.fset(fr, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ufloat_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        as_: u8,
        imm: u8,
    ) -> SimResult<()> {
        let x = self.regs.read_logical(as_) as f32;
        let v = x / (1u32 << imm) as f32;
        self.fset(fr, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_trunc_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        fs: u8,
        imm: u8,
    ) -> SimResult<()> {
        let v = self.fget(fs) * (1u32 << imm) as f32;
        self.regs.write_logical(ar, v.trunc() as i32 as u32);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_utrunc_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        fs: u8,
        imm: u8,
    ) -> SimResult<()> {
        let v = self.fget(fs) * (1u32 << imm) as f32;
        self.regs.write_logical(ar, v.trunc() as u32);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_round_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        fs: u8,
        imm: u8,
    ) -> SimResult<()> {
        // round half-to-even (IEEE default), matching round.s.
        let v = self.fget(fs) * (1u32 << imm) as f32;
        self.regs
            .write_logical(ar, round_half_even(v) as i32 as u32);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ceil_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        fs: u8,
        imm: u8,
    ) -> SimResult<()> {
        let v = self.fget(fs) * (1u32 << imm) as f32;
        self.regs.write_logical(ar, v.ceil() as i32 as u32);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_floor_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        fs: u8,
        imm: u8,
    ) -> SimResult<()> {
        let v = self.fget(fs) * (1u32 << imm) as f32;
        self.regs.write_logical(ar, v.floor() as i32 as u32);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_moveqz_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
        at: u8,
    ) -> SimResult<()> {
        if self.regs.read_logical(at) == 0 {
            self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
        }
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_movnez_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
        at: u8,
    ) -> SimResult<()> {
        if self.regs.read_logical(at) != 0 {
            self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
        }
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_movltz_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
        at: u8,
    ) -> SimResult<()> {
        if (self.regs.read_logical(at) as i32) < 0 {
            self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
        }
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_movgez_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
        at: u8,
    ) -> SimResult<()> {
        if (self.regs.read_logical(at) as i32) >= 0 {
            self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
        }
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_movf_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
        bt: u8,
    ) -> SimResult<()> {
        if (self.br >> (bt & 0xF)) & 1 == 0 {
            self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
        }
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_movt_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        fs: u8,
        bt: u8,
    ) -> SimResult<()> {
        if (self.br >> (bt & 0xF)) & 1 == 1 {
            self.fp[(fr & 0xF) as usize] = self.fp[(fs & 0xF) as usize];
        }
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_cmp_s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        br: u8,
        fs: u8,
        ft: u8,
        kind: FpCmp,
    ) -> SimResult<()> {
        use xtensa::FpCmp::*;
        let a = self.fget(fs);
        let b = self.fget(ft);
        let unordered = a.is_nan() || b.is_nan();
        let result = match kind {
            Un => unordered,
            Oeq => a == b,
            Ueq => unordered || a == b,
            Olt => a < b,
            Ult => unordered || a < b,
            Ole => a <= b,
            Ule => unordered || a <= b,
        };
        let bit = 1u16 << (br & 0xF);
        if result {
            self.br |= bit;
        } else {
            self.br &= !bit;
        }
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_lsi(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        ft: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let ea = self.regs.read_logical(as_).wrapping_add(imm) as u64;
        let val = bus.read_u32(ea)?;
        self.fp[(ft & 0xF) as usize] = val;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_lsiu(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        ft: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let base = self.regs.read_logical(as_).wrapping_add(imm);
        let val = bus.read_u32(base as u64)?;
        self.fp[(ft & 0xF) as usize] = val;
        self.regs.write_logical(as_, base);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ssi(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        ft: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let ea = self.regs.read_logical(as_).wrapping_add(imm);
        self.maybe_invalidate_for_write(ea);
        bus.write_u32(ea as u64, self.fp[(ft & 0xF) as usize])?;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ssiu(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        ft: u8,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let base = self.regs.read_logical(as_).wrapping_add(imm);
        self.maybe_invalidate_for_write(base);
        bus.write_u32(base as u64, self.fp[(ft & 0xF) as usize])?;
        self.regs.write_logical(as_, base);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_lsx(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let ea = self
            .regs
            .read_logical(as_)
            .wrapping_add(self.regs.read_logical(at)) as u64;
        let val = bus.read_u32(ea)?;
        self.fp[(fr & 0xF) as usize] = val;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_lsxu(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let base = self
            .regs
            .read_logical(as_)
            .wrapping_add(self.regs.read_logical(at));
        let val = bus.read_u32(base as u64)?;
        self.fp[(fr & 0xF) as usize] = val;
        self.regs.write_logical(as_, base);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ssx(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let ea = self
            .regs
            .read_logical(as_)
            .wrapping_add(self.regs.read_logical(at));
        self.maybe_invalidate_for_write(ea);
        bus.write_u32(ea as u64, self.fp[(fr & 0xF) as usize])?;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ssxu(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        fr: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let base = self
            .regs
            .read_logical(as_)
            .wrapping_add(self.regs.read_logical(at));
        self.maybe_invalidate_for_write(base);
        bus.write_u32(base as u64, self.fp[(fr & 0xF) as usize])?;
        self.regs.write_logical(as_, base);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }
}
