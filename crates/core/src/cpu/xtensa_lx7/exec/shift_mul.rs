// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arm bodies of `XtensaLx7::execute` for the shift_mul instruction class,
//! moved here verbatim. `execute` keeps the single `match ins`; each arm
//! calls one of these `#[inline(always)]` methods.

use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::{Bus, SimResult};

use crate::cpu::xtensa_sr::SAR;
impl XtensaLx7 {
    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ssl(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
    ) -> SimResult<()> {
        let v = 32u32 - (self.regs.read_logical(as_) & 0x1F);
        self.sr.write(SAR, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ssr(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
    ) -> SimResult<()> {
        let v = self.regs.read_logical(as_) & 0x1F;
        self.sr.write(SAR, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ssai(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        shamt: u8,
    ) -> SimResult<()> {
        self.sr.write(SAR, shamt as u32 & 0x1F);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ssa8l(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
    ) -> SimResult<()> {
        let v = (self.regs.read_logical(as_) & 3) * 8;
        self.sr.write(SAR, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ssa8b(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
    ) -> SimResult<()> {
        let v = 32u32 - (self.regs.read_logical(as_) & 3) * 8;
        self.sr.write(SAR, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_sll(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
    ) -> SimResult<()> {
        let sar = self.sr.read(SAR);
        let shift = 32u32.wrapping_sub(sar);
        // SAR ranges by setter: SSL 1..=32, SSR 0..=31, SSAI 0..=31, SSA8L {0,8,16,24}, SSA8B {32,24,16,8}.
        // wrapping_sub handles SAR=32 → shift=0 (passthrough); SAR=0 → shift=32 (u64 << 32 yields 0).
        // u64 cast is required because a u32 << 32 is undefined in Rust.
        let v = ((self.regs.read_logical(as_) as u64) << shift) as u32;
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_srl(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        at: u8,
    ) -> SimResult<()> {
        let sar = self.sr.read(SAR);
        let v = if sar >= 32 {
            0
        } else {
            self.regs.read_logical(at) >> sar
        };
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_sra(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        at: u8,
    ) -> SimResult<()> {
        let sar = self.sr.read(SAR);
        let src = self.regs.read_logical(at) as i32;
        let v = if sar >= 32 {
            if src < 0 {
                u32::MAX
            } else {
                0
            }
        } else {
            (src >> sar) as u32
        };
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_src(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let sar = self.sr.read(SAR);
        let hi = self.regs.read_logical(as_) as u64;
        let lo = self.regs.read_logical(at) as u64;
        let w = (hi << 32) | lo;
        let v = (w >> sar) as u32;
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_slli(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        shamt: u8,
    ) -> SimResult<()> {
        let v = self.regs.read_logical(as_) << shamt;
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_srli(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        at: u8,
        shamt: u8,
    ) -> SimResult<()> {
        let v = self.regs.read_logical(at) >> shamt;
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_srai(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        at: u8,
        shamt: u8,
    ) -> SimResult<()> {
        let src = self.regs.read_logical(at) as i32;
        let v = (src >> shamt) as u32;
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_mull(
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
            .wrapping_mul(self.regs.read_logical(at));
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_muluh(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let a = self.regs.read_logical(as_) as u64;
        let b = self.regs.read_logical(at) as u64;
        let v = (a.wrapping_mul(b) >> 32) as u32;
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_mulsh(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let a = self.regs.read_logical(as_) as i32 as i64;
        let b = self.regs.read_logical(at) as i32 as i64;
        let v = (a.wrapping_mul(b) >> 32) as u32;
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_mul16u(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let a = self.regs.read_logical(as_) & 0xFFFF;
        let b = self.regs.read_logical(at) & 0xFFFF;
        self.regs.write_logical(ar, a * b);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_mul16s(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let a = self.regs.read_logical(as_) as i16 as i32;
        let b = self.regs.read_logical(at) as i16 as i32;
        self.regs.write_logical(ar, (a * b) as u32);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_quos(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let dividend = self.regs.read_logical(as_) as i32;
        let divisor = self.regs.read_logical(at) as i32;
        if divisor == 0 {
            return self.raise_general_exception(6);
        }
        let q = dividend.wrapping_div(divisor);
        self.regs.write_logical(ar, q as u32);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_quou(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let dividend = self.regs.read_logical(as_);
        let divisor = self.regs.read_logical(at);
        if divisor == 0 {
            return self.raise_general_exception(6);
        }
        let q = dividend / divisor;
        self.regs.write_logical(ar, q);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_rems(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let dividend = self.regs.read_logical(as_) as i32;
        let divisor = self.regs.read_logical(at) as i32;
        if divisor == 0 {
            return self.raise_general_exception(6);
        }
        let r = dividend.wrapping_rem(divisor);
        self.regs.write_logical(ar, r as u32);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_remu(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        as_: u8,
        at: u8,
    ) -> SimResult<()> {
        let dividend = self.regs.read_logical(as_);
        let divisor = self.regs.read_logical(at);
        if divisor == 0 {
            return self.raise_general_exception(6);
        }
        let r = dividend % divisor;
        self.regs.write_logical(ar, r);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }
}
