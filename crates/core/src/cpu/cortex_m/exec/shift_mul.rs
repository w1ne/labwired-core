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

use super::super::CortexM;
use super::super::PcAdvance;
use crate::SimResult;

impl CortexM {
    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_sdiv(
        &mut self,
        rd: u8,
        rn: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let n = self.read_reg(rn) as i32;
        let m = self.read_reg(rm) as i32;
        let result = if m == 0 {
            0
        } else if n == i32::MIN && m == -1 {
            i32::MIN as u32
        } else {
            (n / m) as u32
        };
        self.write_reg(rd, result);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_udiv(
        &mut self,
        rd: u8,
        rn: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let n = self.read_reg(rn);
        let m = self.read_reg(rm);
        let result = n.checked_div(m).unwrap_or(0);
        self.write_reg(rd, result);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_shift_reg32(
        &mut self,
        rd: u8,
        rn: u8,
        rm: u8,
        shift_type: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let value = self.read_reg(rn);
        let shift = self.read_reg(rm) & 0xFF;
        let result = match shift_type {
            0 => {
                if shift >= 32 {
                    0
                } else {
                    value.wrapping_shl(shift)
                }
            }
            1 => {
                if shift == 0 {
                    value
                } else if shift >= 32 {
                    0
                } else {
                    value.wrapping_shr(shift)
                }
            }
            2 => {
                if shift == 0 {
                    value
                } else if shift >= 32 {
                    if (value & 0x8000_0000) != 0 {
                        0xFFFF_FFFF
                    } else {
                        0
                    }
                } else {
                    ((value as i32) >> shift) as u32
                }
            }
            3 => {
                if shift == 0 {
                    value
                } else {
                    value.rotate_right(shift % 32)
                }
            }
            _ => value,
        };
        self.write_reg(rd, result);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_mul(
        &mut self,
        rd: u8,
        rn: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let op1 = self.read_reg(rd);
        let op2 = self.read_reg(rn);
        let res = op1.wrapping_mul(op2);
        self.write_reg(rd, res);
        // T1 encoding: setflags = !InITBlock(). Leaking flags
        // from inside an IT block corrupts the CONDITION of every
        // instruction still to run in that block — measured: an
        // `orrls` before a `strls` in the same `itt ls` cleared Z
        // and the store never happened.
        if !it_block {
            self.update_nz(res);
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_mul32(
        &mut self,
        rd: u8,
        rn: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let op1 = self.read_reg(rn);
        let op2 = self.read_reg(rm);
        let res = op1.wrapping_mul(op2);
        self.write_reg(rd, res);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_lsl(
        &mut self,
        rd: u8,
        rm: u8,
        imm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let val = self.read_reg(rm);
        let res = val.wrapping_shl(imm as u32);
        self.write_reg(rd, res);
        // T1 shift-immediate: setflags = !InITBlock(). Inside an
        // IT block this encoding is the flag-preserving LSL, and
        // leaking flags here would corrupt the remaining block
        // conditions (Tier-1 H563/WBA52 gpio-check regression).
        if !it_block {
            // LSL #n (n>0) sets C to the last bit shifted out:
            // Rm[32-n]. LSL #0 is a move and leaves C unchanged.
            // (Verified against STM32F103 silicon via thumb_oracles.)
            if imm == 0 {
                self.update_nz(res);
            } else {
                let carry = (val >> (32 - imm as u32)) & 1 == 1;
                self.update_nzcv(res, carry, self.get_overflow());
            }
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_lsr(
        &mut self,
        rd: u8,
        rm: u8,
        imm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let val = self.read_reg(rm);
        // Thumb T1: imm5 == 0 encodes a shift of 32.
        let n = if imm == 0 { 32 } else { imm as u32 };
        let res = if n >= 32 { 0 } else { val.wrapping_shr(n) };
        self.write_reg(rd, res);
        // T1 shift-immediate: setflags = !InITBlock(). LSR #n sets C
        // to Rm[n-1], the last bit shifted out (silicon-verified).
        if !it_block {
            let carry = (val >> (n - 1)) & 1 == 1;
            self.update_nzcv(res, carry, self.get_overflow());
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_asr(
        &mut self,
        rd: u8,
        rm: u8,
        imm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let val = self.read_reg(rm);
        // Thumb T1: imm5 == 0 encodes a shift of 32.
        let n = if imm == 0 { 32 } else { imm as u32 };
        let res = ((val as i32) >> n.min(31)) as u32;
        self.write_reg(rd, res);
        // T1 shift-immediate: setflags = !InITBlock(). ASR #n sets C
        // to Rm[n-1], the last bit shifted out (silicon-verified).
        if !it_block {
            let carry = (val >> (n - 1)) & 1 == 1;
            self.update_nzcv(res, carry, self.get_overflow());
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_lsl_reg(
        &mut self,
        rd: u8,
        rm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        // Register-controlled shift: amount = Rm[7:0]. Carry = last
        // bit shifted out; n==0 leaves C unchanged (ARMv7-M Shift_C).
        // Silicon-verified on STM32F103 via thumb_oracles.
        let val = self.read_reg(rd);
        let shift = self.read_reg(rm) & 0xFF;
        let (res, carry) = if shift == 0 {
            (val, self.get_carry())
        } else if shift < 32 {
            (val << shift, (val >> (32 - shift)) & 1 == 1)
        } else if shift == 32 {
            (0, val & 1 == 1)
        } else {
            (0, false)
        };
        self.write_reg(rd, res);
        if !it_block {
            self.update_nzcv(res, carry, self.get_overflow());
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_lsr_reg(
        &mut self,
        rd: u8,
        rm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let val = self.read_reg(rd);
        let shift = self.read_reg(rm) & 0xFF;
        let (res, carry) = if shift == 0 {
            (val, self.get_carry())
        } else if shift < 32 {
            (val >> shift, (val >> (shift - 1)) & 1 == 1)
        } else if shift == 32 {
            (0, (val >> 31) & 1 == 1)
        } else {
            (0, false)
        };
        self.write_reg(rd, res);
        if !it_block {
            self.update_nzcv(res, carry, self.get_overflow());
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_asr_reg(
        &mut self,
        rd: u8,
        rm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let val = self.read_reg(rd);
        let vali = val as i32;
        let shift = self.read_reg(rm) & 0xFF;
        let (res, carry) = if shift == 0 {
            (val, self.get_carry())
        } else if shift < 32 {
            ((vali >> shift) as u32, (val >> (shift - 1)) & 1 == 1)
        } else {
            // shift >= 32: result is all sign bits; C = Rm[31].
            ((vali >> 31) as u32, (val >> 31) & 1 == 1)
        };
        self.write_reg(rd, res);
        if !it_block {
            self.update_nzcv(res, carry, self.get_overflow());
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_ror(
        &mut self,
        rd: u8,
        rm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        // Register rotate: amount = Rm[7:0]. Carry = the rotated
        // result's MSB; n==0 leaves C unchanged (ARMv7-M ROR_C).
        // Silicon-verified on STM32F103 via thumb_oracles.
        let val = self.read_reg(rd);
        let n = self.read_reg(rm) & 0xFF;
        let (res, carry) = if n == 0 {
            (val, self.get_carry())
        } else {
            let r = val.rotate_right(n % 32);
            (r, (r >> 31) & 1 == 1)
        };
        self.write_reg(rd, res);
        if !it_block {
            self.update_nzcv(res, carry, self.get_overflow());
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_smull(
        &mut self,
        rd_lo: u8,
        rd_hi: u8,
        rn: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let lhs = self.read_reg(rn) as i32 as i64;
        let rhs = self.read_reg(rm) as i32 as i64;
        let prod = lhs.wrapping_mul(rhs) as u64;
        self.write_reg(rd_lo, prod as u32);
        self.write_reg(rd_hi, (prod >> 32) as u32);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_umull(
        &mut self,
        rd_lo: u8,
        rd_hi: u8,
        rn: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let prod = (self.read_reg(rn) as u64).wrapping_mul(self.read_reg(rm) as u64);
        self.write_reg(rd_lo, prod as u32);
        self.write_reg(rd_hi, (prod >> 32) as u32);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_smlal(
        &mut self,
        rd_lo: u8,
        rd_hi: u8,
        rn: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let acc = ((self.read_reg(rd_hi) as u64) << 32) | (self.read_reg(rd_lo) as u64);
        let lhs = self.read_reg(rn) as i32 as i64;
        let rhs = self.read_reg(rm) as i32 as i64;
        let new = (acc as i64).wrapping_add(lhs.wrapping_mul(rhs)) as u64;
        self.write_reg(rd_lo, new as u32);
        self.write_reg(rd_hi, (new >> 32) as u32);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_umlal(
        &mut self,
        rd_lo: u8,
        rd_hi: u8,
        rn: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let acc = ((self.read_reg(rd_hi) as u64) << 32) | (self.read_reg(rd_lo) as u64);
        let prod = (self.read_reg(rn) as u64).wrapping_mul(self.read_reg(rm) as u64);
        let new = acc.wrapping_add(prod);
        self.write_reg(rd_lo, new as u32);
        self.write_reg(rd_hi, (new >> 32) as u32);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_umaal(
        &mut self,
        rd_lo: u8,
        rd_hi: u8,
        rn: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        // (rd_hi:rd_lo) = Rn*Rm + rd_lo + rd_hi. Cannot overflow u64.
        let prod = (self.read_reg(rn) as u64).wrapping_mul(self.read_reg(rm) as u64);
        let res = prod
            .wrapping_add(self.read_reg(rd_lo) as u64)
            .wrapping_add(self.read_reg(rd_hi) as u64);
        self.write_reg(rd_lo, res as u32);
        self.write_reg(rd_hi, (res >> 32) as u32);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_mla(
        &mut self,
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let res = self
            .read_reg(ra)
            .wrapping_add(self.read_reg(rn).wrapping_mul(self.read_reg(rm)));
        self.write_reg(rd, res);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_mls(
        &mut self,
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let res = self
            .read_reg(ra)
            .wrapping_sub(self.read_reg(rn).wrapping_mul(self.read_reg(rm)));
        self.write_reg(rd, res);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::cpu::cortex_m) fn exec_smla_xy(
        &mut self,
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
        n_top: bool,
        m_top: bool,
        accumulate: bool,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        // ⚠️ SIGNED halves, sign-extended before the multiply. Taking
        // them as u16 would agree with the hardware on every small
        // positive operand and disagree on every negative one — the
        // shape of bug that passes a smoke test and fails a sensor.
        let half = |value: u32, top: bool| -> i32 {
            (if top {
                (value >> 16) as u16
            } else {
                value as u16
            }) as i16 as i32
        };
        let product = half(self.read_reg(rn), n_top).wrapping_mul(half(self.read_reg(rm), m_top));
        // The SMUL forms have no addend; `ra` there is the 0b1111
        // encoding marker, not a register.
        let res = if accumulate {
            (self.read_reg(ra) as i32).wrapping_add(product)
        } else {
            product
        };
        self.write_reg(rd, res as u32);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }
}
