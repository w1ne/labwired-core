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

use super::super::adc_with_flags;
use super::super::add_with_flags;
use super::super::sbc_with_flags;
use super::super::sub_with_flags;
use super::super::thumb_expand_imm;
use super::super::CortexM;
use super::super::PcAdvance;
use crate::SimResult;

impl CortexM {
    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_bfi(
        &mut self,
        rd: u8,
        rn: u8,
        lsb: u8,
        width: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let src = self.read_reg(rn);
        let dst = self.read_reg(rd);
        let mask = if width == 32 {
            !0
        } else {
            ((1u32.wrapping_shl(width as u32)).wrapping_sub(1)).wrapping_shl(lsb as u32)
        };
        let result = (dst & !mask) | ((src.wrapping_shl(lsb as u32)) & mask);
        self.write_reg(rd, result);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_bfc(
        &mut self,
        rd: u8,
        lsb: u8,
        width: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let dst = self.read_reg(rd);
        let mask = if width == 32 {
            !0
        } else {
            ((1u32.wrapping_shl(width as u32)).wrapping_sub(1)).wrapping_shl(lsb as u32)
        };
        let result = dst & !mask;
        self.write_reg(rd, result);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_sbfx(
        &mut self,
        rd: u8,
        rn: u8,
        lsb: u8,
        width: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let src = self.read_reg(rn);
        let width_mask = if width == 32 {
            !0
        } else {
            (1u32.wrapping_shl(width as u32)).wrapping_sub(1)
        };
        let val = (src.wrapping_shr(lsb as u32)) & width_mask;
        let result = if width == 32 {
            val
        } else {
            let shift = 32 - width;
            ((val.wrapping_shl(shift as u32)) as i32).wrapping_shr(shift as u32) as u32
        };
        self.write_reg(rd, result);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_ubfx(
        &mut self,
        rd: u8,
        rn: u8,
        lsb: u8,
        width: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let src = self.read_reg(rn);
        let width_mask = if width == 32 {
            !0
        } else {
            (1u32.wrapping_shl(width as u32)).wrapping_sub(1)
        };
        let result = (src.wrapping_shr(lsb as u32)) & width_mask;
        self.write_reg(rd, result);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_clz(&mut self, rd: u8, rm: u8) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let val = self.read_reg(rm);
        let result = val.leading_zeros();
        self.write_reg(rd, result);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_rbit(&mut self, rd: u8, rm: u8) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let val = self.read_reg(rm);
        let result = val.reverse_bits();
        self.write_reg(rd, result);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_simd_add_sub8(
        &mut self,
        rd: u8,
        rn: u8,
        rm: u8,
        op: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        // Per-byte parallel add/sub; each lane sets one APSR.GE bit.
        // op: 0=SADD8 1=UADD8 2=SSUB8 3=USUB8 (ARMv7-M A7.7).
        let n = self.read_reg(rn);
        let m = self.read_reg(rm);
        let mut result = 0u32;
        let mut ge = 0u32;
        for i in 0..4 {
            let nb = ((n >> (i * 8)) & 0xFF) as i32;
            let mb = ((m >> (i * 8)) & 0xFF) as i32;
            let (byte, ge_bit) = match op {
                0 => {
                    // SADD8: signed add, GE = sum >= 0
                    let s = (nb as i8 as i32) + (mb as i8 as i32);
                    ((s as u32) & 0xFF, s >= 0)
                }
                1 => {
                    // UADD8: unsigned add, GE = carry out (sum >= 0x100)
                    let s = nb + mb;
                    ((s as u32) & 0xFF, s >= 0x100)
                }
                2 => {
                    // SSUB8: signed sub, GE = diff >= 0
                    let d = (nb as i8 as i32) - (mb as i8 as i32);
                    ((d as u32) & 0xFF, d >= 0)
                }
                _ => {
                    // USUB8: unsigned sub, GE = no borrow (nb >= mb)
                    let d = nb - mb;
                    ((d as u32) & 0xFF, nb >= mb)
                }
            };
            result |= byte << (i * 8);
            if ge_bit {
                ge |= 1 << i;
            }
        }
        self.write_reg(rd, result);
        self.set_ge(ge);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_simd_add_sub16(
        &mut self,
        rd: u8,
        rn: u8,
        rm: u8,
        op: u8,
        sub: bool,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        // Per-halfword parallel add/sub (ARMv7-M A7.7). Two lanes,
        // each 16 bits; the S/U variants set two APSR.GE bits per
        // lane, the saturating and halving variants set none.
        let n = self.read_reg(rn);
        let m = self.read_reg(rm);
        let mut result = 0u32;
        let mut ge = 0u32;
        let sets_ge = op == 0x0 || op == 0x4;
        for i in 0..2 {
            let nh = (n >> (i * 16)) & 0xFFFF;
            let mh = (m >> (i * 16)) & 0xFFFF;
            // Signed lane operands (for the S/Q/SH variants).
            let ns = nh as u16 as i16 as i32;
            let ms = mh as u16 as i16 as i32;
            let (half, ge_bit) = match op {
                // SADD16 / SSUB16: signed, wrapping. GE per lane is
                // "result was non-negative".
                0x0 => {
                    let s = if sub { ns - ms } else { ns + ms };
                    ((s as u32) & 0xFFFF, s >= 0)
                }
                // QADD16 / QSUB16: signed saturating to i16.
                0x1 => {
                    let s = if sub { ns - ms } else { ns + ms };
                    ((s.clamp(-32768, 32767) as u32) & 0xFFFF, false)
                }
                // SHADD16 / SHSUB16: signed halving — the sum is
                // 17-bit and the result is its bits [16:1], which an
                // arithmetic shift of the i32 gives directly.
                0x2 => {
                    let s = if sub { ns - ms } else { ns + ms };
                    (((s >> 1) as u32) & 0xFFFF, false)
                }
                // UADD16 / USUB16: unsigned, wrapping. GE is carry
                // out for the add and "no borrow" for the subtract.
                0x4 => {
                    if sub {
                        ((nh.wrapping_sub(mh)) & 0xFFFF, nh >= mh)
                    } else {
                        let s = nh + mh;
                        (s & 0xFFFF, s >= 0x1_0000)
                    }
                }
                // UQADD16 / UQSUB16: unsigned saturating to u16.
                0x5 => {
                    if sub {
                        (nh.saturating_sub(mh), false)
                    } else {
                        ((nh + mh).min(0xFFFF), false)
                    }
                }
                // UHADD16 / UHSUB16: unsigned halving — bits [16:1]
                // of the 17-bit intermediate, so the difference is
                // masked to 17 bits before the shift rather than
                // sign-extended across the whole word.
                _ => {
                    let s = if sub {
                        ((nh as i32 - mh as i32) as u32) & 0x1_FFFF
                    } else {
                        nh + mh
                    };
                    ((s >> 1) & 0xFFFF, false)
                }
            };
            result |= half << (i * 16);
            if ge_bit {
                ge |= 0b11 << (i * 2);
            }
        }
        self.write_reg(rd, result);
        if sets_ge {
            self.set_ge(ge);
        }
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_sel(
        &mut self,
        rd: u8,
        rn: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        // SEL: pick each byte from Rn if its GE bit is set, else Rm.
        let n = self.read_reg(rn);
        let m = self.read_reg(rm);
        let ge = self.get_ge();
        let mut result = 0u32;
        for i in 0..4 {
            let src = if (ge >> i) & 1 == 1 { n } else { m };
            result |= ((src >> (i * 8)) & 0xFF) << (i * 8);
        }
        self.write_reg(rd, result);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::cpu::cortex_m) fn exec_data_proc32(
        &mut self,
        op: u8,
        rn: u8,
        rd: u8,
        rm: u8,
        imm5: u8,
        shift_type: u8,
        set_flags: bool,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let op2_raw = self.read_reg(rm);
        let carry_in = self.get_carry();
        let mut op2 = op2_raw;
        match shift_type {
            0 => op2 = op2.wrapping_shl(imm5 as u32), // LSL
            1 => {
                op2 = if imm5 == 0 {
                    0
                } else {
                    op2.wrapping_shr(imm5 as u32)
                }
            } // LSR
            2 => {
                op2 = if imm5 == 0 {
                    if (op2 & 0x80000000) != 0 {
                        0xFFFFFFFF
                    } else {
                        0
                    }
                } else {
                    ((op2 as i32) >> (imm5 as u32)) as u32
                }
            } // ASR
            3 if imm5 != 0 => op2 = op2.rotate_right(imm5 as u32), // ROR
            // ROR #0 is RRX (ARMv7-M A7.7.154): rotate Rm right
            // one bit through the carry — result (C<<31)|(Rm>>1),
            // carry-out Rm[0].
            3 => op2 = ((carry_in as u32) << 31) | (op2_raw >> 1),
            _ => {}
        }
        let op1 = self.read_reg(rn);
        // Carry-out of the barrel shifter for an immediate-shifted
        // operand (ARMv7-M A2.3.1): carry_in when no shift is
        // encoded, otherwise the last bit shifted out. The
        // arithmetic ops below ignore it (their C is the adder's);
        // the logical ops take it when S=1.
        let shifter_carry = match shift_type {
            0 if imm5 == 0 => carry_in,
            0 => ((op2_raw >> (32 - imm5 as u32)) & 1) != 0,
            1 if imm5 == 0 => (op2_raw >> 31) != 0,
            1 => ((op2_raw >> (imm5 as u32 - 1)) & 1) != 0,
            2 if imm5 == 0 => (op2_raw >> 31) != 0,
            2 => ((op2_raw >> (imm5 as u32 - 1)) & 1) != 0,
            // RRX: the bit rotated out is Rm[0].
            3 if imm5 == 0 => (op2_raw & 1) != 0,
            3 => (op2 >> 31) != 0,
            _ => carry_in,
        };
        // (result, carry-out, overflow). Arithmetic ops compute
        // true NZCV via the shared add/sub-with-flags helpers;
        // logical ops take the shifter carry-out and leave V.
        let (result, c, v) = match op {
            0x0 => (op1 & op2, shifter_carry, self.get_overflow()), // AND / TST
            0x1 => (op1 & !op2, shifter_carry, self.get_overflow()), // BIC
            0x2 => {
                let r = if rn == 0xF { op2 } else { op1 | op2 };
                (r, shifter_carry, self.get_overflow())
            } // ORR / MOV
            0x3 => {
                let r = if rn == 0xF { !op2 } else { op1 | !op2 };
                (r, shifter_carry, self.get_overflow())
            } // ORN / MVN
            0x4 => (op1 ^ op2, shifter_carry, self.get_overflow()), // EOR / TEQ
            0x6 => {
                // PKH (PKHBT/PKHTB): pack halfwords. The tb bit lives in
                // shift_type bit1 (0 => PKHBT keep op1 low / Rm high,
                // 2 => PKHTB keep op1 high / Rm low). The optional barrel
                // shift on Rm is not applied here (PKH is off the bignum
                // path and only the imm5==0 form is exercised); operands
                // come from the raw Rm. Not flag-setting.
                let r = if shift_type == 2 {
                    (op1 & 0xFFFF_0000) | (op2_raw & 0x0000_FFFF)
                } else {
                    (op1 & 0x0000_FFFF) | (op2_raw & 0xFFFF_0000)
                };
                (r, carry_in, self.get_overflow())
            } // PKH
            0x8 => add_with_flags(op1, op2),                        // ADD / CMN
            0xA => adc_with_flags(op1, op2, carry_in as u32),       // ADC
            0xB => sbc_with_flags(op1, op2, carry_in as u32),       // SBC
            0xD => sub_with_flags(op1, op2),                        // SUB / CMP
            0xE => sub_with_flags(op2, op1),                        // RSB (op2 - op1)
            _ => {
                #[cfg(debug_assertions)]
                tracing::warn!("Unknown DataProc32 op {:#x}", op);
                (op2, carry_in, self.get_overflow())
            }
        };
        if rd != 15 {
            self.write_reg(rd, result);
        }
        if set_flags {
            self.update_nzcv(result, c, v);
        }
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_data_proc_imm32(
        &mut self,
        op: u8,
        rn: u8,
        rd: u8,
        imm12: u32,
        set_flags: bool,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let imm = thumb_expand_imm(imm12);
        let val1 = self.read_reg(rn);
        let (res, c, v) = match op {
            0x0 => (val1 & imm, self.get_carry(), self.get_overflow()), // AND
            0x1 => (val1 & !imm, self.get_carry(), self.get_overflow()), // BIC
            0x2 => {
                let res = if rn == 0xF { imm } else { val1 | imm };
                (res, self.get_carry(), self.get_overflow())
            } // ORR / MOV
            0x3 => {
                let res = if rn == 0xF { !imm } else { val1 | !imm };
                (res, self.get_carry(), self.get_overflow())
            } // ORN / MVN
            0x4 => (val1 ^ imm, self.get_carry(), self.get_overflow()), // EOR
            0x8 => add_with_flags(val1, imm),                           // ADD
            0xA => adc_with_flags(val1, imm, self.get_carry() as u32),  // ADC
            0xB => sbc_with_flags(val1, imm, self.get_carry() as u32),  // SBC
            0xD => sub_with_flags(val1, imm),                           // SUB / CMP
            0xE => sub_with_flags(imm, val1),                           // RSB
            _ => {
                tracing::warn!("Unhandled T32 DataProcImm32 op {:#x}", op);
                (0, self.get_carry(), self.get_overflow())
            }
        };

        if rd != 15 {
            self.write_reg(rd, res);
        }
        if set_flags {
            self.update_nzcv(res, c, v);
        }
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_movw(&mut self, rd: u8, imm: u16) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        self.write_reg(rd, imm as u32);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_movt(&mut self, rd: u8, imm: u16) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let old_val = self.read_reg(rd);
        let new_val = (old_val & 0x0000FFFF) | ((imm as u32) << 16);
        self.write_reg(rd, new_val);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_mov_imm(
        &mut self,
        rd: u8,
        imm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        self.write_reg(rd, imm as u32);
        if !it_block {
            self.update_nz(imm as u32);
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_add_reg(
        &mut self,
        rd: u8,
        rn: u8,
        rm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let op1 = self.read_reg(rn);
        let op2 = self.read_reg(rm);
        let (res, c, v) = add_with_flags(op1, op2);
        self.write_reg(rd, res);
        if !it_block {
            self.update_nzcv(res, c, v);
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_add_imm3(
        &mut self,
        rd: u8,
        rn: u8,
        imm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let op1 = self.read_reg(rn);
        let (res, c, v) = add_with_flags(op1, imm as u32);
        self.write_reg(rd, res);
        if !it_block {
            self.update_nzcv(res, c, v);
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_add_imm8(
        &mut self,
        rd: u8,
        imm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let op1 = self.read_reg(rd);
        let (res, c, v) = add_with_flags(op1, imm as u32);
        self.write_reg(rd, res);
        if !it_block {
            self.update_nzcv(res, c, v);
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_sub_reg(
        &mut self,
        rd: u8,
        rn: u8,
        rm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let op1 = self.read_reg(rn);
        let op2 = self.read_reg(rm);
        let (res, c, v) = sub_with_flags(op1, op2);
        self.write_reg(rd, res);
        if !it_block {
            self.update_nzcv(res, c, v);
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_sub_imm3(
        &mut self,
        rd: u8,
        rn: u8,
        imm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let op1 = self.read_reg(rn);
        let (res, c, v) = sub_with_flags(op1, imm as u32);
        self.write_reg(rd, res);
        if !it_block {
            self.update_nzcv(res, c, v);
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_sub_imm8(
        &mut self,
        rd: u8,
        imm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let op1 = self.read_reg(rd);
        let (res, c, v) = sub_with_flags(op1, imm as u32);
        self.write_reg(rd, res);
        if !it_block {
            self.update_nzcv(res, c, v);
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_add_sp(&mut self, imm: u16) -> SimResult<PcAdvance> {
        let sp = self.read_reg(13).wrapping_add(imm as u32);
        self.write_reg(13, sp);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_sub_sp(&mut self, imm: u16) -> SimResult<PcAdvance> {
        let sp = self.read_reg(13).wrapping_sub(imm as u32);
        self.write_reg(13, sp);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_uxtb(&mut self, rd: u8, rm: u8) -> SimResult<PcAdvance> {
        let val = self.read_reg(rm);
        self.write_reg(rd, val & 0xFF);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_uxth(&mut self, rd: u8, rm: u8) -> SimResult<PcAdvance> {
        let val = self.read_reg(rm);
        self.write_reg(rd, val & 0xFFFF);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_sxtb(&mut self, rd: u8, rm: u8) -> SimResult<PcAdvance> {
        let val = self.read_reg(rm) as u8 as i8 as i32 as u32;
        self.write_reg(rd, val);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_sxth(&mut self, rd: u8, rm: u8) -> SimResult<PcAdvance> {
        let val = self.read_reg(rm) as u16 as i16 as i32 as u32;
        self.write_reg(rd, val);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_extend_w(
        &mut self,
        rd: u8,
        rn: u8,
        rm: u8,
        rotate: u8,
        op: u8,
    ) -> SimResult<PcAdvance> {
        // ROR Rm by `rotate` (0/8/16/24), then extract+extend.
        let v = self.read_reg(rm).rotate_right(rotate as u32);
        let ext = match op {
            0b000 => v as u16 as i16 as i32 as u32, // S*XTH
            0b001 => v & 0xFFFF,                    // U*XTH
            0b100 => v as u8 as i8 as i32 as u32,   // S*XTB
            _ => v & 0xFF,                          // U*XTB (0b101)
        };
        // Extend-and-add variants (Rn != 0xF) add Rn; the plain
        // extends encode Rn = 0xF.
        let out = if rn == 0xF {
            ext
        } else {
            self.read_reg(rn).wrapping_add(ext)
        };
        self.write_reg(rd, out);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_add_reg_high(
        &mut self,
        rd: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let val1 = self.read_reg(rd);
        let val2 = self.read_reg(rm);
        self.write_reg(rd, val1.wrapping_add(val2));
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_cmp_imm(
        &mut self,
        rn: u8,
        imm: u8,
    ) -> SimResult<PcAdvance> {
        let op1 = self.read_reg(rn);
        let (res, c, v) = sub_with_flags(op1, imm as u32);
        self.update_nzcv(res, c, v);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_cmp_reg(
        &mut self,
        rn: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let op1 = self.read_reg(rn);
        let op2 = self.read_reg(rm);
        let (res, c, v) = sub_with_flags(op1, op2);
        self.update_nzcv(res, c, v);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_mov_reg(
        &mut self,
        rd: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let val = self.read_reg(rm);
        if rd == 15 {
            // ARMv7-M: writing PC through MOV is BXWritePC, i.e. a
            // BRANCH — bit 0 selects the instruction set and is not
            // part of the address, and no sequential PC advance
            // follows. Falling through to `write_reg` alone left the
            // Thumb bit in PC and then added `pc_increment` on top,
            // landing 2 bytes past the target.
            //
            // rustc emits exactly this for a dense `match` over
            // bytes: `ADR Rn, table` / `ADD Rd, Rn, idx LSL #2` /
            // `MOV PC, Rd` into a table of 4-byte `B.W` entries.
            // Two bytes late is the middle of an entry, so the
            // second halfword of that branch executed as a stray
            // 16-bit instruction and control fell through to the
            // NEXT entry — every arm ran its successor's code. The
            // nRF52840 OBD-II scanner painted its OLED that way:
            // "RPM 3000" came out "S N 3000" (R->S, P->Q which is
            // absent so blank, M->N), while digits, dispatched by
            // arithmetic rather than by this table, stayed exact.
            self.pc = val & !1;
            __pc = PcAdvance::Zero;
        } else {
            self.write_reg(rd, val);
        }
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_and(
        &mut self,
        rd: u8,
        rm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let res = self.read_reg(rd) & self.read_reg(rm);
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
    pub(in crate::cpu::cortex_m) fn exec_bic(
        &mut self,
        rd: u8,
        rm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let res = self.read_reg(rd) & !self.read_reg(rm);
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
    pub(in crate::cpu::cortex_m) fn exec_orr(
        &mut self,
        rd: u8,
        rm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let res = self.read_reg(rd) | self.read_reg(rm);
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
    pub(in crate::cpu::cortex_m) fn exec_eor(
        &mut self,
        rd: u8,
        rm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let res = self.read_reg(rd) ^ self.read_reg(rm);
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
    pub(in crate::cpu::cortex_m) fn exec_mvn(
        &mut self,
        rd: u8,
        rm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let res = !self.read_reg(rm);
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
    pub(in crate::cpu::cortex_m) fn exec_adc(
        &mut self,
        rd: u8,
        rm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let op1 = self.read_reg(rd);
        let op2 = self.read_reg(rm);
        let carry_in = (self.xpsr >> 29) & 1;
        let (res, c, v) = adc_with_flags(op1, op2, carry_in);
        self.write_reg(rd, res);
        // T1 encoding: setflags = !InITBlock(). Leaking flags
        // from inside an IT block corrupts the CONDITION of every
        // instruction still to run in that block — measured: an
        // `orrls` before a `strls` in the same `itt ls` cleared Z
        // and the store never happened.
        if !it_block {
            self.update_nzcv(res, c, v);
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_sbc(
        &mut self,
        rd: u8,
        rm: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let op1 = self.read_reg(rd);
        let op2 = self.read_reg(rm);
        let carry_in = (self.xpsr >> 29) & 1;
        let (res, c, v) = sbc_with_flags(op1, op2, carry_in);
        self.write_reg(rd, res);
        // T1 encoding: setflags = !InITBlock(). Leaking flags
        // from inside an IT block corrupts the CONDITION of every
        // instruction still to run in that block — measured: an
        // `orrls` before a `strls` in the same `itt ls` cleared Z
        // and the store never happened.
        if !it_block {
            self.update_nzcv(res, c, v);
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_rev(&mut self, rd: u8, rm: u8) -> SimResult<PcAdvance> {
        let val = self.read_reg(rm);
        self.write_reg(rd, val.swap_bytes());
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_rev16(&mut self, rd: u8, rm: u8) -> SimResult<PcAdvance> {
        let val = self.read_reg(rm);
        let low = ((val & 0xFF) << 8) | ((val >> 8) & 0xFF);
        let high = ((val & 0x00FF0000) << 8) | ((val & 0xFF000000) >> 8);
        self.write_reg(rd, high | low);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_rev_sh(&mut self, rd: u8, rm: u8) -> SimResult<PcAdvance> {
        let val = self.read_reg(rm);
        let low = ((val & 0xFF) << 8) | ((val >> 8) & 0xFF);
        self.write_reg(rd, (low as i16) as u32);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_tst(&mut self, rn: u8, rm: u8) -> SimResult<PcAdvance> {
        let res = self.read_reg(rn) & self.read_reg(rm);
        self.update_nz(res);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_cmn(&mut self, rn: u8, rm: u8) -> SimResult<PcAdvance> {
        let op1 = self.read_reg(rn);
        let op2 = self.read_reg(rm);
        let (res, c, v) = add_with_flags(op1, op2);
        self.update_nzcv(res, c, v);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_rsbs(
        &mut self,
        rd: u8,
        rn: u8,
        it_block: bool,
    ) -> SimResult<PcAdvance> {
        let op1 = self.read_reg(rn);
        let (res, c, v) = sub_with_flags(0, op1);
        self.write_reg(rd, res);
        // T1 encoding: setflags = !InITBlock(). Leaking flags
        // from inside an IT block corrupts the CONDITION of every
        // instruction still to run in that block — measured: an
        // `orrls` before a `strls` in the same `itt ls` cleared Z
        // and the store never happened.
        if !it_block {
            self.update_nzcv(res, c, v);
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_add_sp_reg(
        &mut self,
        rd: u8,
        imm: u16,
    ) -> SimResult<PcAdvance> {
        let res = self.sp.wrapping_add(imm as u32);
        self.write_reg(rd, res);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_adr(&mut self, rd: u8, imm: u16) -> SimResult<PcAdvance> {
        let pc_val = (self.pc & !3).wrapping_add(4);
        let res = pc_val.wrapping_add(imm as u32);
        self.write_reg(rd, res);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_addw_imm(
        &mut self,
        rd: u8,
        rn: u8,
        imm: u16,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        // Plain 12-bit zero-extended immediate (T4). Distinct
        // from DataProcImm32::ADD which runs imm12 through
        // ThumbExpandImm.
        let res = self.read_reg(rn).wrapping_add(imm as u32);
        self.write_reg(rd, res);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_subw_imm(
        &mut self,
        rd: u8,
        rn: u8,
        imm: u16,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let res = self.read_reg(rn).wrapping_sub(imm as u32);
        self.write_reg(rd, res);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }
}
