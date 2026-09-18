// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Move-only split of the arm bodies of `CortexM::step_execute` in
// `cpu/cortex_m.rs`. Each `exec_*` method below is the verbatim body of one
// `match instruction` arm; `step_execute` still holds the single dispatch
// match (same arms, same order) and calls one method per arm. PC advance is
// communicated by the `PcAdvance` return value, not a `&mut` out-parameter.

use super::super::vfp_binop;
use super::super::vfp_fma;
use super::super::AccessWidth;
use super::super::CortexM;
use super::super::PcAdvance;
use super::super::VfpBinOp;
use crate::Bus;
use crate::SimResult;

impl CortexM {
    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vldr<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        sd: u8,
        rn: u8,
        imm: u16,
        add: bool,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let base = self.read_reg(rn);
        let base = if rn == 15 { base & !3 } else { base };
        let addr = if add {
            base.wrapping_add(imm as u32)
        } else {
            base.wrapping_sub(imm as u32)
        };
        let val = self.load(bus, addr, AccessWidth::Word)?;
        self.fpu_s[sd as usize] = val;
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vstr<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        sd: u8,
        rn: u8,
        imm: u16,
        add: bool,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let base = self.read_reg(rn);
        let addr = if add {
            base.wrapping_add(imm as u32)
        } else {
            base.wrapping_sub(imm as u32)
        };
        let val = self.fpu_s[sd as usize];
        self.store(bus, addr, AccessWidth::Word, val)?;
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vmul_f32(
        &mut self,
        sd: u8,
        sn: u8,
        sm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let a = self.fpu_s[sn as usize];
        let b = self.fpu_s[sm as usize];
        self.fpu_s[sd as usize] = vfp_binop(VfpBinOp::Mul, a, b, self.fpscr);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vadd_f32(
        &mut self,
        sd: u8,
        sn: u8,
        sm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let a = self.fpu_s[sn as usize];
        let b = self.fpu_s[sm as usize];
        self.fpu_s[sd as usize] = vfp_binop(VfpBinOp::Add, a, b, self.fpscr);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vsub_f32(
        &mut self,
        sd: u8,
        sn: u8,
        sm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let a = self.fpu_s[sn as usize];
        let b = self.fpu_s[sm as usize];
        self.fpu_s[sd as usize] = vfp_binop(VfpBinOp::Sub, a, b, self.fpscr);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vdiv_f32(
        &mut self,
        sd: u8,
        sn: u8,
        sm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let a = self.fpu_s[sn as usize];
        let b = self.fpu_s[sm as usize];
        self.fpu_s[sd as usize] = vfp_binop(VfpBinOp::Div, a, b, self.fpscr);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vfma_f32(
        &mut self,
        sd: u8,
        sn: u8,
        sm: u8,
    ) -> SimResult<PcAdvance> {
        let a = self.fpu_s[sn as usize];
        let b = self.fpu_s[sm as usize];
        let c = self.fpu_s[sd as usize];
        // Fused: single rounding of (a*b)+c, not a*b rounded then +c.
        self.fpu_s[sd as usize] = vfp_fma(a, b, c, false, false, self.fpscr);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vfms_f32(
        &mut self,
        sd: u8,
        sn: u8,
        sm: u8,
    ) -> SimResult<PcAdvance> {
        let a = self.fpu_s[sn as usize];
        let b = self.fpu_s[sm as usize];
        let c = self.fpu_s[sd as usize];
        self.fpu_s[sd as usize] = vfp_fma(a, b, c, true, false, self.fpscr);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vfnma_f32(
        &mut self,
        sd: u8,
        sn: u8,
        sm: u8,
    ) -> SimResult<PcAdvance> {
        let a = self.fpu_s[sn as usize];
        let b = self.fpu_s[sm as usize];
        let c = self.fpu_s[sd as usize];
        self.fpu_s[sd as usize] = vfp_fma(a, b, c, false, true, self.fpscr);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vfnms_f32(
        &mut self,
        sd: u8,
        sn: u8,
        sm: u8,
    ) -> SimResult<PcAdvance> {
        let a = self.fpu_s[sn as usize];
        let b = self.fpu_s[sm as usize];
        let c = self.fpu_s[sd as usize];
        self.fpu_s[sd as usize] = vfp_fma(a, b, c, true, true, self.fpscr);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vmov_sn_rt(
        &mut self,
        sn: u8,
        rt: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        self.fpu_s[sn as usize] = self.read_reg(rt);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vmov_rt_sn(
        &mut self,
        rt: u8,
        sn: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        self.write_reg(rt, self.fpu_s[sn as usize]);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vmov_f32_reg(
        &mut self,
        sd: u8,
        sm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        self.fpu_s[sd as usize] = self.fpu_s[sm as usize];
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vmov_f32_imm(
        &mut self,
        sd: u8,
        imm_bits: u32,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        self.fpu_s[sd as usize] = imm_bits;
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vcvt_f32_from_int(
        &mut self,
        sd: u8,
        sm: u8,
        signed: bool,
        fbits: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let raw = self.fpu_s[sm as usize];
        let int_val = if signed {
            raw as i32 as f64
        } else {
            raw as f64
        };
        let scaled = if fbits > 0 {
            int_val / ((1u64 << fbits.min(31)) as f64)
        } else {
            int_val
        };
        self.fpu_s[sd as usize] = (scaled as f32).to_bits();
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vcvt_int_from_f32(
        &mut self,
        sd: u8,
        sm: u8,
        signed: bool,
        fbits: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let f = f32::from_bits(self.fpu_s[sm as usize]) as f64;
        let scaled = if fbits > 0 {
            f * ((1u64 << fbits.min(31)) as f64)
        } else {
            f
        };
        let bits = if signed {
            (scaled as i32) as u32
        } else if scaled <= 0.0 {
            0
        } else if scaled >= f64::from(u32::MAX) {
            u32::MAX
        } else {
            scaled as u32
        };
        self.fpu_s[sd as usize] = bits;
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vfp_store_multiple<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        rn: u8,
        s_first: u8,
        count: u8,
        add: bool,
        wback: bool,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let base = self.read_reg(rn);
        let total = 4u32.wrapping_mul(count as u32);
        let start = if add { base } else { base.wrapping_sub(total) };
        for i in 0..count {
            let idx = s_first as usize + i as usize;
            let val = if idx < 32 { self.fpu_s[idx] } else { 0 };
            let addr = start.wrapping_add(4 * i as u32);
            self.store(bus, addr, AccessWidth::Word, val)?;
        }
        if wback {
            let nb = if add {
                base.wrapping_add(total)
            } else {
                base.wrapping_sub(total)
            };
            self.write_reg(rn, nb);
        }
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vfp_load_multiple<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        rn: u8,
        s_first: u8,
        count: u8,
        add: bool,
        wback: bool,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let base = self.read_reg(rn);
        let total = 4u32.wrapping_mul(count as u32);
        let start = if add { base } else { base.wrapping_sub(total) };
        for i in 0..count {
            let idx = s_first as usize + i as usize;
            let addr = start.wrapping_add(4 * i as u32);
            let v = self.load(bus, addr, AccessWidth::Word)?;
            if idx < 32 {
                self.fpu_s[idx] = v;
            }
        }
        if wback {
            let nb = if add {
                base.wrapping_add(total)
            } else {
                base.wrapping_sub(total)
            };
            self.write_reg(rn, nb);
        }
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vldr64<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        dd: u8,
        rn: u8,
        imm: u16,
        add: bool,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let base = self.read_reg(rn);
        let base = if rn == 15 { base & !3 } else { base };
        let addr = if add {
            base.wrapping_add(imm as u32)
        } else {
            base.wrapping_sub(imm as u32)
        };
        for (w, off) in [(0usize, 0u32), (1, 4)] {
            let v = self.load(bus, addr.wrapping_add(off), AccessWidth::Word)?;
            if (dd as usize + w) < 32 {
                self.fpu_s[dd as usize + w] = v;
            }
        }
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vstr64<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        dd: u8,
        rn: u8,
        imm: u16,
        add: bool,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let base = self.read_reg(rn);
        let addr = if add {
            base.wrapping_add(imm as u32)
        } else {
            base.wrapping_sub(imm as u32)
        };
        for (w, off) in [(0usize, 0u32), (1, 4)] {
            let val = if (dd as usize + w) < 32 {
                self.fpu_s[dd as usize + w]
            } else {
                0
            };
            self.store(bus, addr.wrapping_add(off), AccessWidth::Word, val)?;
        }
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vmov_f64_reg(
        &mut self,
        dd: u8,
        dm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        if (dd as usize + 1) < 32 && (dm as usize + 1) < 32 {
            self.fpu_s[dd as usize] = self.fpu_s[dm as usize];
            self.fpu_s[dd as usize + 1] = self.fpu_s[dm as usize + 1];
        }
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vmov_d_rt_rt2(
        &mut self,
        dm: u8,
        rt: u8,
        rt2: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        if (dm as usize + 1) < 32 {
            self.fpu_s[dm as usize] = self.read_reg(rt);
            self.fpu_s[dm as usize + 1] = self.read_reg(rt2);
        }
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vmov_rt_rt2_d(
        &mut self,
        rt: u8,
        rt2: u8,
        dm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let (lo, hi) = if (dm as usize + 1) < 32 {
            (self.fpu_s[dm as usize], self.fpu_s[dm as usize + 1])
        } else {
            (0, 0)
        };
        self.write_reg(rt, lo);
        self.write_reg(rt2, hi);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vadd_f64(
        &mut self,
        dd: u8,
        dn: u8,
        dm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let r = self.read_f64(dn) + self.read_f64(dm);
        self.write_f64(dd, r);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vsub_f64(
        &mut self,
        dd: u8,
        dn: u8,
        dm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let r = self.read_f64(dn) - self.read_f64(dm);
        self.write_f64(dd, r);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vmul_f64(
        &mut self,
        dd: u8,
        dn: u8,
        dm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let r = self.read_f64(dn) * self.read_f64(dm);
        self.write_f64(dd, r);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_vdiv_f64(
        &mut self,
        dd: u8,
        dn: u8,
        dm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let r = self.read_f64(dn) / self.read_f64(dm);
        self.write_f64(dd, r);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }
}
