// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! A64 decode and execute for [`super::Aarch64`], by the top-level encoding
//! groups of the Arm ARM (C4.1): data processing (immediate), branches /
//! exception generation / system, loads and stores, data processing
//! (register), and the SIMD&FP subset the core implements.

use super::{ec, regs, Aarch64, Exception, Flow};
use crate::{Bus, SimResult};

/// Unwrap a guest-visible memory result, or end the instruction with its
/// exception.
macro_rules! guest {
    ($e:expr) => {
        match $e? {
            Ok(v) => v,
            Err(ex) => return Ok(Flow::Exc(ex)),
        }
    };
}

#[inline]
fn undef() -> SimResult<Flow> {
    Ok(Flow::Exc(Exception::undefined()))
}

#[inline]
fn bits(v: u32, hi: u32, lo: u32) -> u32 {
    (v >> lo) & ((1u32 << (hi - lo + 1)) - 1)
}

#[inline]
fn sext(v: u64, n: u32) -> u64 {
    ((v << (64 - n)) as i64 >> (64 - n)) as u64
}

#[inline]
fn width_mask(sf: bool) -> u64 {
    if sf {
        u64::MAX
    } else {
        0xFFFF_FFFF
    }
}

/// `AddWithCarry`: (result, NZCV in PSTATE position).
pub(crate) fn add_with_carry(x: u64, y: u64, carry: bool, sf: bool) -> (u64, u64) {
    let c = u64::from(carry);
    if sf {
        let (r1, c1) = x.overflowing_add(y);
        let (r, c2) = r1.overflowing_add(c);
        let v = ((x ^ r) & (y ^ r)) >> 63 != 0;
        (r, flags(r >> 63 != 0, r == 0, c1 || c2, v))
    } else {
        let (x, y) = (x as u32, y as u32);
        let (r1, c1) = x.overflowing_add(y);
        let (r, c2) = r1.overflowing_add(c as u32);
        let v = ((x ^ r) & (y ^ r)) >> 31 != 0;
        (u64::from(r), flags(r >> 31 != 0, r == 0, c1 || c2, v))
    }
}

#[inline]
fn flags(n: bool, z: bool, c: bool, v: bool) -> u64 {
    (u64::from(n) << 31) | (u64::from(z) << 30) | (u64::from(c) << 29) | (u64::from(v) << 28)
}

#[inline]
fn nz_flags(r: u64, sf: bool) -> u64 {
    let r = r & width_mask(sf);
    let n = if sf { r >> 63 } else { r >> 31 } != 0;
    flags(n, r == 0, false, false)
}

pub(crate) fn cond_holds(cond: u32, nzcv: u64) -> bool {
    let n = nzcv & super::PSTATE_N != 0;
    let z = nzcv & super::PSTATE_Z != 0;
    let c = nzcv & super::PSTATE_C != 0;
    let v = nzcv & super::PSTATE_V != 0;
    let r = match cond >> 1 {
        0 => z,
        1 => c,
        2 => n,
        3 => v,
        4 => c && !z,
        5 => n == v,
        6 => n == v && !z,
        _ => true,
    };
    if cond & 1 != 0 && cond != 0xF {
        !r
    } else {
        r
    }
}

/// `DecodeBitMasks` (Arm ARM J1): (wmask, tmask), `None` when reserved.
pub(crate) fn decode_bit_masks(
    n: u32,
    imms: u32,
    immr: u32,
    immediate: bool,
    datasize: u32,
) -> Option<(u64, u64)> {
    let combined = (n << 6) | (!imms & 0x3F);
    if combined == 0 {
        return None;
    }
    let len = 31 - combined.leading_zeros();
    if len < 1 {
        return None;
    }
    let esize = 1u32 << len;
    if esize > datasize {
        return None;
    }
    let levels = esize - 1;
    if immediate && (imms & levels) == levels {
        return None;
    }
    let s = imms & levels;
    let r = immr & levels;
    let d = s.wrapping_sub(r) & levels;
    let emask = if esize == 64 {
        u64::MAX
    } else {
        (1u64 << esize) - 1
    };
    let ones = |k: u32| if k >= 64 { u64::MAX } else { (1u64 << k) - 1 };
    let welem = ones(s + 1);
    let telem = ones(d + 1);
    let ror = |v: u64, r: u32| -> u64 {
        if r == 0 {
            v
        } else {
            ((v >> r) | (v << (esize - r))) & emask
        }
    };
    let rep = |e: u64| -> u64 {
        let mut out = 0u64;
        let mut i = 0;
        while i < datasize {
            out |= e << i;
            i += esize;
        }
        out & width_mask(datasize == 64)
    };
    Some((rep(ror(welem, r)), rep(telem)))
}

fn shift_reg(v: u64, ty: u32, amount: u32, sf: bool) -> u64 {
    let size = if sf { 64 } else { 32 };
    let v = v & width_mask(sf);
    let amount = amount % size;
    if amount == 0 {
        return v;
    }
    let r = match ty {
        0 => v << amount,
        1 => v >> amount,
        2 => {
            if sf {
                ((v as i64) >> amount) as u64
            } else {
                ((v as u32 as i32) >> amount) as u32 as u64
            }
        }
        _ => {
            if sf {
                v.rotate_right(amount)
            } else {
                u64::from((v as u32).rotate_right(amount))
            }
        }
    };
    r & width_mask(sf)
}

fn extend_reg(v: u64, option: u32, shift: u32) -> u64 {
    let e = match option {
        0 => v & 0xFF,
        1 => v & 0xFFFF,
        2 => v & 0xFFFF_FFFF,
        3 => v,
        4 => sext(v & 0xFF, 8),
        5 => sext(v & 0xFFFF, 16),
        6 => sext(v & 0xFFFF_FFFF, 32),
        _ => v,
    };
    e << shift
}

fn crc32(acc: u32, data: u64, bytes: u32, castagnoli: bool) -> u32 {
    let poly = if castagnoli { 0x82F6_3B78 } else { 0xEDB8_8320 };
    let mut crc = acc;
    for i in 0..bytes {
        crc ^= ((data >> (8 * i)) & 0xFF) as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ poly
            } else {
                crc >> 1
            };
        }
    }
    crc
}

impl Aarch64 {
    pub(crate) fn exec(&mut self, bus: &mut dyn Bus, inst: u32) -> SimResult<Flow> {
        match bits(inst, 28, 25) {
            0b1000 | 0b1001 => self.dp_imm(inst),
            0b1010 | 0b1011 => self.branch_sys(bus, inst),
            0b0100 | 0b0110 | 0b1100 | 0b1110 => self.ldst(bus, inst),
            0b0101 | 0b1101 => self.dp_reg(inst),
            0b0111 | 0b1111 => self.simd_dp(inst),
            _ => undef(),
        }
    }

    fn set_flags(&mut self, f: u64) {
        self.nzcv = f;
    }

    // ---- data processing (immediate) ------------------------------------

    fn dp_imm(&mut self, inst: u32) -> SimResult<Flow> {
        let sf = inst >> 31 != 0;
        let rd = bits(inst, 4, 0);
        let rn = bits(inst, 9, 5);
        let m = width_mask(sf);
        match bits(inst, 25, 23) {
            0 | 1 => {
                let imm = (u64::from(bits(inst, 23, 5)) << 2) | u64::from(bits(inst, 30, 29));
                let imm = sext(imm, 21);
                let v = if inst >> 31 != 0 {
                    (self.pc & !0xFFF).wrapping_add(imm << 12)
                } else {
                    self.pc.wrapping_add(imm)
                };
                self.set_xr(rd, v);
            }
            2 => {
                let sub = inst & (1 << 30) != 0;
                let s = inst & (1 << 29) != 0;
                let imm = u64::from(bits(inst, 21, 10)) << (12 * bits(inst, 22, 22));
                let a = self.xsp(rn) & m;
                let (r, f) = if sub {
                    add_with_carry(a, !imm & m, true, sf)
                } else {
                    add_with_carry(a, imm, false, sf)
                };
                if s {
                    self.set_flags(f);
                    self.set_xr(rd, r & m);
                } else {
                    self.set_xsp(rd, r & m);
                }
            }
            4 => {
                let n = bits(inst, 22, 22);
                if !sf && n != 0 {
                    return undef();
                }
                let Some((imm, _)) = decode_bit_masks(
                    n,
                    bits(inst, 15, 10),
                    bits(inst, 21, 16),
                    true,
                    if sf { 64 } else { 32 },
                ) else {
                    return undef();
                };
                let a = self.xr(rn) & m;
                let r = match bits(inst, 30, 29) {
                    0 | 3 => a & imm,
                    1 => a | imm,
                    _ => a ^ imm,
                } & m;
                if bits(inst, 30, 29) == 3 {
                    self.set_flags(nz_flags(r, sf));
                    self.set_xr(rd, r);
                } else {
                    self.set_xsp(rd, r);
                }
            }
            5 => {
                let opc = bits(inst, 30, 29);
                let hw = bits(inst, 22, 21);
                if opc == 1 || (!sf && hw >= 2) {
                    return undef();
                }
                let pos = 16 * hw;
                let imm = u64::from(bits(inst, 20, 5)) << pos;
                let r = match opc {
                    0 => !imm,
                    2 => imm,
                    _ => (self.xr(rd) & !(0xFFFFu64 << pos)) | imm,
                } & m;
                self.set_xr(rd, r);
            }
            6 => {
                let opc = bits(inst, 30, 29);
                let n = bits(inst, 22, 22);
                if opc == 3 || (n != 0) != sf {
                    return undef();
                }
                let immr = bits(inst, 21, 16);
                let imms = bits(inst, 15, 10);
                let datasize = if sf { 64 } else { 32 };
                if !sf && (immr >= 32 || imms >= 32) {
                    return undef();
                }
                let Some((wmask, tmask)) = decode_bit_masks(n, imms, immr, false, datasize) else {
                    return undef();
                };
                let src = self.xr(rn) & m;
                let dst = if opc == 1 { self.xr(rd) & m } else { 0 };
                let rot = shift_reg(src, 3, immr, sf);
                let bot = (dst & !wmask) | (rot & wmask);
                let top = if opc == 0 {
                    if (src >> imms) & 1 != 0 {
                        m
                    } else {
                        0
                    }
                } else {
                    dst
                };
                self.set_xr(rd, ((top & !tmask) | (bot & tmask)) & m);
            }
            7 => {
                let n = bits(inst, 22, 22);
                if bits(inst, 30, 29) != 0 || (n != 0) != sf || inst & (1 << 21) != 0 {
                    return undef();
                }
                let lsb = bits(inst, 15, 10);
                if !sf && lsb >= 32 {
                    return undef();
                }
                let rm = bits(inst, 20, 16);
                let hi = self.xr(rn) & m;
                let lo = self.xr(rm) & m;
                let r = if lsb == 0 {
                    lo
                } else if sf {
                    (lo >> lsb) | (hi << (64 - lsb))
                } else {
                    ((lo >> lsb) | (hi << (32 - lsb))) & m
                };
                self.set_xr(rd, r);
            }
            _ => return undef(),
        }
        Ok(Flow::Next)
    }

    // ---- branches, exception generation, system -------------------------

    fn branch_sys(&mut self, bus: &mut dyn Bus, inst: u32) -> SimResult<Flow> {
        let pc = self.pc;
        if bits(inst, 30, 26) == 0b00101 {
            let off = sext(u64::from(bits(inst, 25, 0)) << 2, 28);
            if inst >> 31 != 0 {
                self.set_xr(30, pc.wrapping_add(4));
            }
            return Ok(Flow::Jump(pc.wrapping_add(off)));
        }
        if bits(inst, 31, 25) == 0b0101010 {
            if inst & (1 << 24) != 0 || inst & (1 << 4) != 0 {
                return undef();
            }
            let off = sext(u64::from(bits(inst, 23, 5)) << 2, 21);
            return Ok(if cond_holds(bits(inst, 3, 0), self.nzcv) {
                Flow::Jump(pc.wrapping_add(off))
            } else {
                Flow::Next
            });
        }
        if bits(inst, 31, 24) == 0b1101_0100 {
            let opc = bits(inst, 23, 21);
            let imm = bits(inst, 20, 5);
            let ll = bits(inst, 1, 0);
            if bits(inst, 4, 2) != 0 {
                return undef();
            }
            return Ok(match (opc, ll) {
                (0, 1..=3) => self.svc_hvc_smc(ll, imm),
                (1, 0) => Flow::Exc(Exception::new(ec::BRK, imm)),
                _ => Flow::Exc(Exception::undefined()),
            });
        }
        if bits(inst, 31, 22) == 0b11_0101_0100 {
            return self.system(bus, inst);
        }
        if bits(inst, 30, 25) == 0b011010 {
            let sf = inst >> 31 != 0;
            let v = self.xr(bits(inst, 4, 0)) & width_mask(sf);
            let nz = inst & (1 << 24) != 0;
            let off = sext(u64::from(bits(inst, 23, 5)) << 2, 21);
            return Ok(if (v != 0) == nz {
                Flow::Jump(pc.wrapping_add(off))
            } else {
                Flow::Next
            });
        }
        if bits(inst, 30, 25) == 0b011011 {
            let bit = (bits(inst, 31, 31) << 5) | bits(inst, 23, 19);
            let set = (self.xr(bits(inst, 4, 0)) >> bit) & 1 != 0;
            let nz = inst & (1 << 24) != 0;
            let off = sext(u64::from(bits(inst, 18, 5)) << 2, 16);
            return Ok(if set == nz {
                Flow::Jump(pc.wrapping_add(off))
            } else {
                Flow::Next
            });
        }
        if bits(inst, 31, 25) == 0b1101011 {
            let opc = bits(inst, 24, 21);
            let rn = bits(inst, 9, 5);
            if bits(inst, 20, 16) != 0x1F || bits(inst, 15, 10) != 0 || bits(inst, 4, 0) != 0 {
                return undef();
            }
            return Ok(match opc {
                0 | 2 => Flow::Jump(self.xr(rn)),
                1 => {
                    let t = self.xr(rn);
                    self.set_xr(30, pc.wrapping_add(4));
                    Flow::Jump(t)
                }
                4 if rn == 31 => {
                    if self.el == 0 {
                        return undef();
                    }
                    self.eret()
                }
                _ => Flow::Exc(Exception::undefined()),
            });
        }
        undef()
    }

    fn system(&mut self, bus: &mut dyn Bus, inst: u32) -> SimResult<Flow> {
        let l = inst & (1 << 21) != 0;
        let op0 = bits(inst, 20, 19);
        let op1 = bits(inst, 18, 16);
        let crn = bits(inst, 15, 12);
        let crm = bits(inst, 11, 8);
        let op2 = bits(inst, 7, 5);
        let rt = bits(inst, 4, 0);
        if !l && op0 == 0 {
            match crn {
                // Hints. Everything in the hint space this core does not act
                // on (YIELD, SEV, PAC/BTI hints of later architectures, ...)
                // executes as a NOP, as the architecture requires.
                2 => {
                    if rt != 31 {
                        return undef();
                    }
                    if crm == 0 && op2 == 3 && !self.irq_line() {
                        self.waiting_for_interrupt = true;
                    }
                    return Ok(Flow::Next);
                }
                3 => {
                    if rt != 31 {
                        return undef();
                    }
                    return match op2 {
                        2 => {
                            self.exclusive = None;
                            Ok(Flow::Next)
                        }
                        4..=6 => Ok(Flow::Next),
                        _ => undef(),
                    };
                }
                4 => {
                    if rt != 31 {
                        return undef();
                    }
                    let imm = u64::from(crm);
                    match (op1, op2) {
                        (0, 5) => {
                            if self.el == 0 {
                                return undef();
                            }
                            self.spsel = imm & 1 != 0;
                        }
                        (3, 6) | (3, 7) => {
                            if self.el == 0 && self.sctlr[1] & (1 << 9) == 0 {
                                return undef();
                            }
                            if op2 == 6 {
                                self.daif |= imm << 6;
                            } else {
                                self.daif &= !(imm << 6);
                            }
                        }
                        _ => return undef(),
                    }
                    return Ok(Flow::Next);
                }
                _ => return undef(),
            }
        }
        if op0 == 1 {
            if l {
                return undef();
            }
            return self.sys_instruction(bus, op1, crn, crm, op2, rt);
        }
        if op0 >= 2 {
            let key = bits(inst, 20, 5);
            if matches!(key, regs::FPCR | regs::FPSR) {
                if let Some(e) = self.fp_trap() {
                    return Ok(Flow::Exc(e));
                }
            }
            if !self.sysreg_access_ok(key, !l) {
                return undef();
            }
            if l {
                let Some(v) = self.read_sysreg(key) else {
                    return undef();
                };
                self.set_xr(rt, v);
            } else {
                let v = self.xr(rt);
                match key {
                    regs::SPSEL => self.spsel = v & 1 != 0,
                    regs::CURRENTEL => return undef(),
                    _ => {
                        if !self.write_sysreg(key, v) {
                            return undef();
                        }
                    }
                }
            }
            return Ok(Flow::Next);
        }
        undef()
    }

    /// `SYS`: cache maintenance, address translation, TLB maintenance.
    fn sys_instruction(
        &mut self,
        bus: &mut dyn Bus,
        op1: u32,
        crn: u32,
        crm: u32,
        op2: u32,
        rt: u32,
    ) -> SimResult<Flow> {
        let need = match op1 {
            3 => 0,
            4 => 2,
            6 => 3,
            _ => 1,
        };
        if self.el < need {
            return undef();
        }
        match crn {
            7 => {
                match (op1, crm, op2) {
                    // DC ZVA
                    (3, 4, 1) => {
                        if self.el == 0 && self.sctlr[1] & (1 << 14) == 0 {
                            return undef();
                        }
                        let va = self.xr(rt);
                        guest!(self.dc_zva(bus, va));
                    }
                    // AT S1E1R/W, S1E0R/W
                    (0, 8, 0..=3) => {
                        let va = self.xr(rt);
                        let as_el = if op2 >= 2 { 0 } else { 1 };
                        self.address_translate(bus, va, as_el, op2 & 1 != 0)?;
                    }
                    (4, 8, 0 | 1) => {
                        let va = self.xr(rt);
                        self.address_translate(bus, va, 2, op2 & 1 != 0)?;
                    }
                    (6, 8, 0 | 1) => {
                        let va = self.xr(rt);
                        self.address_translate(bus, va, 3, op2 & 1 != 0)?;
                    }
                    // IC and DC maintenance by VA / set-way: no caches are
                    // modelled, so there is nothing to clean or invalidate.
                    (0, 1 | 5, 0)
                    | (3, 5, 1)
                    | (0, 6, 1 | 2)
                    | (0, 10 | 14, 2)
                    | (3, 10..=14, 1) => {}
                    _ => return undef(),
                }
                Ok(Flow::Next)
            }
            8 => {
                // TLBI: every form flushes everything (TLB is untagged).
                self.flush_tlb();
                Ok(Flow::Next)
            }
            _ => undef(),
        }
    }

    // ---- data processing (register) -------------------------------------

    fn dp_reg(&mut self, inst: u32) -> SimResult<Flow> {
        let sf = inst >> 31 != 0;
        let m = width_mask(sf);
        let rd = bits(inst, 4, 0);
        let rn = bits(inst, 9, 5);
        let rm = bits(inst, 20, 16);
        let op1 = bits(inst, 28, 28);
        let op2 = bits(inst, 24, 21);
        if op1 == 0 {
            if op2 & 0b1000 == 0 {
                // Logical (shifted register).
                let amount = bits(inst, 15, 10);
                if !sf && amount >= 32 {
                    return undef();
                }
                let mut b = shift_reg(self.xr(rm), bits(inst, 23, 22), amount, sf);
                if inst & (1 << 21) != 0 {
                    b = !b & m;
                }
                let a = self.xr(rn) & m;
                let opc = bits(inst, 30, 29);
                let r = match opc {
                    0 | 3 => a & b,
                    1 => a | b,
                    _ => a ^ b,
                };
                if opc == 3 {
                    self.set_flags(nz_flags(r, sf));
                }
                self.set_xr(rd, r & m);
                return Ok(Flow::Next);
            }
            let sub = inst & (1 << 30) != 0;
            let s = inst & (1 << 29) != 0;
            if op2 & 1 == 0 {
                // Add/subtract (shifted register).
                let ty = bits(inst, 23, 22);
                let amount = bits(inst, 15, 10);
                if ty == 3 || (!sf && amount >= 32) {
                    return undef();
                }
                let b = shift_reg(self.xr(rm), ty, amount, sf);
                let a = self.xr(rn) & m;
                let (r, f) = if sub {
                    add_with_carry(a, !b & m, true, sf)
                } else {
                    add_with_carry(a, b, false, sf)
                };
                if s {
                    self.set_flags(f);
                }
                self.set_xr(rd, r & m);
                return Ok(Flow::Next);
            }
            // Add/subtract (extended register).
            let shift = bits(inst, 12, 10);
            if bits(inst, 23, 22) != 0 || shift > 4 {
                return undef();
            }
            let b = extend_reg(self.xr(rm), bits(inst, 15, 13), shift) & m;
            let a = self.xsp(rn) & m;
            let (r, f) = if sub {
                add_with_carry(a, !b & m, true, sf)
            } else {
                add_with_carry(a, b, false, sf)
            };
            if s {
                self.set_flags(f);
                self.set_xr(rd, r & m);
            } else {
                self.set_xsp(rd, r & m);
            }
            return Ok(Flow::Next);
        }
        match op2 {
            0 => {
                if bits(inst, 15, 10) != 0 {
                    return undef();
                }
                let sub = inst & (1 << 30) != 0;
                let a = self.xr(rn) & m;
                let b = self.xr(rm) & m;
                let b = if sub { !b & m } else { b };
                let (r, f) = add_with_carry(a, b, self.nzcv & super::PSTATE_C != 0, sf);
                if inst & (1 << 29) != 0 {
                    self.set_flags(f);
                }
                self.set_xr(rd, r & m);
            }
            2 => {
                // Conditional compare.
                if inst & (1 << 29) == 0 || inst & (1 << 10) != 0 || inst & (1 << 4) != 0 {
                    return undef();
                }
                let cond = bits(inst, 15, 12);
                if cond_holds(cond, self.nzcv) {
                    let a = self.xr(rn) & m;
                    let b = if inst & (1 << 11) != 0 {
                        u64::from(rm)
                    } else {
                        self.xr(rm) & m
                    };
                    let (_, f) = if inst & (1 << 30) != 0 {
                        add_with_carry(a, !b & m, true, sf)
                    } else {
                        add_with_carry(a, b, false, sf)
                    };
                    self.set_flags(f);
                } else {
                    self.set_flags(u64::from(bits(inst, 3, 0)) << 28);
                }
            }
            4 => {
                // Conditional select.
                if inst & (1 << 29) != 0 || inst & (1 << 11) != 0 {
                    return undef();
                }
                let op = bits(inst, 30, 30);
                let o2 = bits(inst, 10, 10);
                let r = if cond_holds(bits(inst, 15, 12), self.nzcv) {
                    self.xr(rn)
                } else {
                    let b = self.xr(rm);
                    match (op, o2) {
                        (0, 0) => b,
                        (0, _) => b.wrapping_add(1),
                        (_, 0) => !b,
                        _ => b.wrapping_neg(),
                    }
                };
                self.set_xr(rd, r & m);
            }
            6 => {
                if inst & (1 << 29) != 0 {
                    return undef();
                }
                if inst & (1 << 30) != 0 {
                    return self.dp_1src(inst, sf, rd, rn);
                }
                return self.dp_2src(inst, sf, rd, rn, rm);
            }
            8..=15 => return self.dp_3src(inst, sf, rd, rn, rm),
            _ => return undef(),
        }
        Ok(Flow::Next)
    }

    fn dp_1src(&mut self, inst: u32, sf: bool, rd: u32, rn: u32) -> SimResult<Flow> {
        if bits(inst, 20, 16) != 0 {
            return undef();
        }
        let a = self.xr(rn);
        let r = match (bits(inst, 15, 10), sf) {
            (0, true) => a.reverse_bits(),
            (0, false) => u64::from((a as u32).reverse_bits()),
            (1, _) => {
                let v = a & width_mask(sf);
                ((v & 0x00FF_00FF_00FF_00FF) << 8) | ((v >> 8) & 0x00FF_00FF_00FF_00FF)
            }
            (2, false) => u64::from((a as u32).swap_bytes()),
            (2, true) => {
                let lo = (a as u32).swap_bytes();
                let hi = ((a >> 32) as u32).swap_bytes();
                (u64::from(hi) << 32) | u64::from(lo)
            }
            (3, true) => a.swap_bytes(),
            (4, true) => u64::from(a.leading_zeros()),
            (4, false) => u64::from((a as u32).leading_zeros()),
            (5, true) => u64::from((a ^ ((a as i64 >> 1) as u64)).leading_zeros() - 1),
            (5, false) => {
                let w = a as u32;
                u64::from((w ^ ((w as i32 >> 1) as u32)).leading_zeros() - 1)
            }
            _ => return undef(),
        };
        self.set_xr(rd, r & width_mask(sf));
        Ok(Flow::Next)
    }

    fn dp_2src(&mut self, inst: u32, sf: bool, rd: u32, rn: u32, rm: u32) -> SimResult<Flow> {
        let m = width_mask(sf);
        let a = self.xr(rn) & m;
        let b = self.xr(rm) & m;
        let opcode = bits(inst, 15, 10);
        let r = match opcode {
            2 => a.checked_div(b).unwrap_or(0),
            3 => {
                if b == 0 {
                    0
                } else if sf {
                    (a as i64).wrapping_div(b as i64) as u64
                } else {
                    (a as u32 as i32).wrapping_div(b as u32 as i32) as u32 as u64
                }
            }
            8..=11 => shift_reg(a, opcode - 8, (b % if sf { 64 } else { 32 }) as u32, sf),
            0x10..=0x17 => {
                let sz = opcode & 3;
                if (sz == 3) != sf {
                    return undef();
                }
                u64::from(crc32(a as u32, self.xr(rm), 1 << sz, opcode & 4 != 0))
            }
            _ => return undef(),
        };
        self.set_xr(rd, r & m);
        Ok(Flow::Next)
    }

    fn dp_3src(&mut self, inst: u32, sf: bool, rd: u32, rn: u32, rm: u32) -> SimResult<Flow> {
        if bits(inst, 30, 29) != 0 {
            return undef();
        }
        let m = width_mask(sf);
        let ra = bits(inst, 14, 10);
        let o0 = inst & (1 << 15) != 0;
        let a = self.xr(rn);
        let b = self.xr(rm);
        let acc = self.xr(ra);
        let r = match bits(inst, 23, 21) {
            0 => {
                let p = a.wrapping_mul(b);
                if o0 {
                    acc.wrapping_sub(p)
                } else {
                    acc.wrapping_add(p)
                }
            }
            1 if sf => {
                let p = (a as u32 as i32 as i64).wrapping_mul(b as u32 as i32 as i64) as u64;
                if o0 {
                    acc.wrapping_sub(p)
                } else {
                    acc.wrapping_add(p)
                }
            }
            2 if sf && !o0 => ((i128::from(a as i64) * i128::from(b as i64)) >> 64) as u64,
            5 if sf => {
                let p = u64::from(a as u32) * u64::from(b as u32);
                if o0 {
                    acc.wrapping_sub(p)
                } else {
                    acc.wrapping_add(p)
                }
            }
            6 if sf && !o0 => ((u128::from(a) * u128::from(b)) >> 64) as u64,
            _ => return undef(),
        };
        self.set_xr(rd, r & m);
        Ok(Flow::Next)
    }

    // ---- loads and stores -----------------------------------------------

    /// Unprivileged forms (LDTR/STTR) access as EL0 from EL1.
    fn unpriv_el(&self) -> Option<u8> {
        (self.el == 1).then_some(0)
    }

    fn load_v(
        &mut self,
        bus: &mut dyn Bus,
        va: u64,
        bytes: u64,
        as_el: Option<u8>,
    ) -> SimResult<Result<u128, Exception>> {
        if bytes == 16 {
            let lo = match self.read_va(bus, va, 8, as_el)? {
                Ok(v) => v,
                Err(e) => return Ok(Err(e)),
            };
            let hi = match self.read_va(bus, va.wrapping_add(8), 8, as_el)? {
                Ok(v) => v,
                Err(e) => return Ok(Err(e)),
            };
            return Ok(Ok(u128::from(lo) | (u128::from(hi) << 64)));
        }
        Ok(self.read_va(bus, va, bytes, as_el)?.map(u128::from))
    }

    fn store_v(
        &mut self,
        bus: &mut dyn Bus,
        va: u64,
        bytes: u64,
        v: u128,
        as_el: Option<u8>,
    ) -> SimResult<Result<(), Exception>> {
        if bytes == 16 {
            if let Err(e) = self.write_va(bus, va, 8, v as u64, as_el)? {
                return Ok(Err(e));
            }
            return self.write_va(bus, va.wrapping_add(8), 8, (v >> 64) as u64, as_el);
        }
        let mask = if bytes == 8 {
            u64::MAX
        } else {
            (1u64 << (8 * bytes)) - 1
        };
        self.write_va(bus, va, bytes, v as u64 & mask, as_el)
    }

    /// One register transfer for the LDR/STR family. `opc` and `size` as
    /// encoded; returns `None` for the unallocated combinations.
    #[allow(clippy::too_many_arguments)]
    fn transfer(
        &mut self,
        bus: &mut dyn Bus,
        size: u32,
        v: bool,
        opc: u32,
        rt: u32,
        va: u64,
        as_el: Option<u8>,
    ) -> SimResult<Option<Result<(), Exception>>> {
        if v {
            let bytes = if opc & 2 != 0 {
                if size != 0 {
                    return Ok(None);
                }
                16
            } else {
                1u64 << size
            };
            if opc & 1 != 0 {
                let val = match self.load_v(bus, va, bytes, as_el)? {
                    Ok(v) => v,
                    Err(e) => return Ok(Some(Err(e))),
                };
                self.v[rt as usize] = val;
            } else {
                let val = self.v[rt as usize];
                if let Err(e) = self.store_v(bus, va, bytes, val, as_el)? {
                    return Ok(Some(Err(e)));
                }
            }
            return Ok(Some(Ok(())));
        }
        let bytes = 1u64 << size;
        match opc {
            0 => {
                let val = self.xr(rt);
                let val = if bytes == 8 {
                    val
                } else {
                    val & ((1u64 << (8 * bytes)) - 1)
                };
                Ok(Some(self.write_va(bus, va, bytes, val, as_el)?))
            }
            1 => Ok(Some(
                self.read_va(bus, va, bytes, as_el)?
                    .map(|x| self.set_xr(rt, x)),
            )),
            2 if size == 3 => Ok(Some(Ok(()))), // PRFM
            2 | 3 => {
                if opc == 3 && size >= 2 {
                    return Ok(None);
                }
                let r = self.read_va(bus, va, bytes, as_el)?;
                Ok(Some(r.map(|x| {
                    let s = sext(x, 8 * bytes as u32);
                    self.set_xr(rt, if opc == 3 { s & 0xFFFF_FFFF } else { s });
                })))
            }
            _ => Ok(None),
        }
    }

    fn ldst(&mut self, bus: &mut dyn Bus, inst: u32) -> SimResult<Flow> {
        let v = inst & (1 << 26) != 0;
        if v {
            if let Some(e) = self.fp_trap() {
                return Ok(Flow::Exc(e));
            }
        }
        let rt = bits(inst, 4, 0);
        let rn = bits(inst, 9, 5);
        if bits(inst, 29, 24) == 0b001000 && !v {
            return self.ldst_exclusive(bus, inst);
        }
        if bits(inst, 29, 27) == 0b011 && bits(inst, 25, 24) == 0 {
            // Load register (literal).
            let opc = bits(inst, 31, 30);
            let va = self
                .pc
                .wrapping_add(sext(u64::from(bits(inst, 23, 5)) << 2, 21));
            if v {
                let bytes = match opc {
                    0 => 4,
                    1 => 8,
                    2 => 16,
                    _ => return undef(),
                };
                let val = guest!(self.load_v(bus, va, bytes, None));
                self.v[rt as usize] = val;
                return Ok(Flow::Next);
            }
            match opc {
                0 => {
                    let x = guest!(self.read_va(bus, va, 4, None));
                    self.set_xr(rt, x);
                }
                1 => {
                    let x = guest!(self.read_va(bus, va, 8, None));
                    self.set_xr(rt, x);
                }
                2 => {
                    let x = guest!(self.read_va(bus, va, 4, None));
                    self.set_xr(rt, sext(x, 32));
                }
                _ => {} // PRFM (literal)
            }
            return Ok(Flow::Next);
        }
        if bits(inst, 29, 27) == 0b101 {
            return self.ldst_pair(bus, inst);
        }
        if bits(inst, 29, 27) != 0b111 {
            return undef();
        }
        let size = bits(inst, 31, 30);
        let opc = bits(inst, 23, 22);
        let scale = if v && opc & 2 != 0 { 4 } else { size };
        let base = self.xsp(rn);
        if inst & (1 << 24) != 0 {
            let va = base.wrapping_add(u64::from(bits(inst, 21, 10)) << scale);
            return Ok(match self.transfer(bus, size, v, opc, rt, va, None)? {
                None => Flow::Exc(Exception::undefined()),
                Some(Err(e)) => Flow::Exc(e),
                Some(Ok(())) => Flow::Next,
            });
        }
        if inst & (1 << 21) == 0 {
            let imm = sext(u64::from(bits(inst, 20, 12)), 9);
            let mode = bits(inst, 11, 10);
            let (va, wb, as_el) = match mode {
                0 => (base.wrapping_add(imm), None, None),
                1 => (base, Some(base.wrapping_add(imm)), None),
                2 => {
                    if v {
                        return undef();
                    }
                    (base.wrapping_add(imm), None, self.unpriv_el())
                }
                _ => {
                    let a = base.wrapping_add(imm);
                    (a, Some(a), None)
                }
            };
            return Ok(match self.transfer(bus, size, v, opc, rt, va, as_el)? {
                None => Flow::Exc(Exception::undefined()),
                Some(Err(e)) => Flow::Exc(e),
                Some(Ok(())) => {
                    if let Some(w) = wb {
                        self.set_xsp(rn, w);
                    }
                    Flow::Next
                }
            });
        }
        if bits(inst, 11, 10) != 0b10 {
            // LSE atomics (Armv8.1) and pointer-authenticated loads (8.3):
            // not on an Armv8.0 core.
            return undef();
        }
        let option = bits(inst, 15, 13);
        if option & 2 == 0 {
            return undef();
        }
        let shift = if inst & (1 << 12) != 0 { scale } else { 0 };
        let off = extend_reg(self.xr(bits(inst, 20, 16)), option, shift);
        let va = base.wrapping_add(off);
        Ok(match self.transfer(bus, size, v, opc, rt, va, None)? {
            None => Flow::Exc(Exception::undefined()),
            Some(Err(e)) => Flow::Exc(e),
            Some(Ok(())) => Flow::Next,
        })
    }

    fn ldst_pair(&mut self, bus: &mut dyn Bus, inst: u32) -> SimResult<Flow> {
        let v = inst & (1 << 26) != 0;
        let opc = bits(inst, 31, 30);
        let load = inst & (1 << 22) != 0;
        let mode = bits(inst, 24, 23);
        let rt = bits(inst, 4, 0);
        let rt2 = bits(inst, 14, 10);
        let rn = bits(inst, 9, 5);
        let (bytes, signed) = if v {
            match opc {
                0 => (4, false),
                1 => (8, false),
                2 => (16, false),
                _ => return undef(),
            }
        } else {
            match (opc, load) {
                (0, _) => (4, false),
                (1, true) if mode != 0 => (4, true),
                (2, _) => (8, false),
                _ => return undef(),
            }
        };
        let imm = sext(u64::from(bits(inst, 21, 15)), 7).wrapping_mul(bytes);
        let base = self.xsp(rn);
        let (va, wb) = match mode {
            0 | 2 => (base.wrapping_add(imm), None),
            1 => (base, Some(base.wrapping_add(imm))),
            _ => {
                let a = base.wrapping_add(imm);
                (a, Some(a))
            }
        };
        let va2 = va.wrapping_add(bytes);
        if v {
            if load {
                let a = guest!(self.load_v(bus, va, bytes, None));
                let b = guest!(self.load_v(bus, va2, bytes, None));
                self.v[rt as usize] = a;
                self.v[rt2 as usize] = b;
            } else {
                let (a, b) = (self.v[rt as usize], self.v[rt2 as usize]);
                guest!(self.store_v(bus, va, bytes, a, None));
                guest!(self.store_v(bus, va2, bytes, b, None));
            }
        } else if load {
            let a = guest!(self.read_va(bus, va, bytes, None));
            let b = guest!(self.read_va(bus, va2, bytes, None));
            let fix = |x: u64| if signed { sext(x, 32) } else { x };
            self.set_xr(rt, fix(a));
            self.set_xr(rt2, fix(b));
        } else {
            let m = if bytes == 8 { u64::MAX } else { 0xFFFF_FFFF };
            let (a, b) = (self.xr(rt) & m, self.xr(rt2) & m);
            guest!(self.write_va(bus, va, bytes, a, None));
            guest!(self.write_va(bus, va2, bytes, b, None));
        }
        if let Some(w) = wb {
            self.set_xsp(rn, w);
        }
        Ok(Flow::Next)
    }

    fn ldst_exclusive(&mut self, bus: &mut dyn Bus, inst: u32) -> SimResult<Flow> {
        let size = bits(inst, 31, 30);
        let o2 = inst & (1 << 23) != 0;
        let load = inst & (1 << 22) != 0;
        let o1 = inst & (1 << 21) != 0;
        let o0 = inst & (1 << 15) != 0;
        let rs = bits(inst, 20, 16);
        let rt2 = bits(inst, 14, 10);
        let rn = bits(inst, 9, 5);
        let rt = bits(inst, 4, 0);
        let va = self.xsp(rn);
        let bytes = 1u64 << size;
        if o2 {
            // LDAR/STLR (o0 = 1). The o0 = 0 forms (LDLAR/STLLR) and the
            // o1 = 1 forms (CAS) are Armv8.1.
            if o1 || !o0 {
                return undef();
            }
            if load {
                let x = guest!(self.read_va(bus, va, bytes, None));
                self.set_xr(rt, x);
            } else {
                let x = self.xr(rt);
                let x = if bytes == 8 {
                    x
                } else {
                    x & ((1u64 << (8 * bytes)) - 1)
                };
                guest!(self.write_va(bus, va, bytes, x, None));
            }
            return Ok(Flow::Next);
        }
        if o1 && size < 2 {
            return undef();
        }
        // LDXP/STXP: size 10 moves two words, size 11 two doublewords.
        let total = if o1 { 2 * bytes } else { bytes };
        let pa = match self.translate(
            bus,
            va,
            if load {
                super::Access::Read
            } else {
                super::Access::Write
            },
            None,
        )? {
            Ok(pa) => pa,
            Err(f) => {
                let a = if load {
                    super::Access::Read
                } else {
                    super::Access::Write
                };
                return Ok(Flow::Exc(self.abort(va, f, a)));
            }
        };
        if load {
            if o1 {
                let a = guest!(self.read_va(bus, va, bytes, None));
                let b = guest!(self.read_va(bus, va.wrapping_add(bytes), bytes, None));
                self.set_xr(rt, a);
                self.set_xr(rt2, b);
            } else {
                let a = guest!(self.read_va(bus, va, bytes, None));
                self.set_xr(rt, a);
            }
            self.exclusive = Some((pa, total));
            return Ok(Flow::Next);
        }
        let ok = self.exclusive == Some((pa, total));
        self.exclusive = None;
        if ok {
            let m = if bytes == 8 {
                u64::MAX
            } else {
                (1u64 << (8 * bytes)) - 1
            };
            let a = self.xr(rt) & m;
            guest!(self.write_va(bus, va, bytes, a, None));
            if o1 {
                let b = self.xr(rt2) & m;
                guest!(self.write_va(bus, va.wrapping_add(bytes), bytes, b, None));
            }
        }
        self.set_xr(rs, u64::from(!ok));
        Ok(Flow::Next)
    }

    // ---- SIMD&FP data processing ----------------------------------------

    fn simd_dp(&mut self, inst: u32) -> SimResult<Flow> {
        if let Some(e) = self.fp_trap() {
            return Ok(Flow::Exc(e));
        }
        let rd = bits(inst, 4, 0) as usize;
        let rn = bits(inst, 9, 5) as usize;
        let q = inst & (1 << 30) != 0;
        let low = |v: u128| if q { v } else { v & u128::from(u64::MAX) };
        // FMOV (general).
        if inst & 0x7F20_FC00 == 0x1E20_0000 {
            let sf = inst >> 31;
            let ty = bits(inst, 23, 22);
            let rmode = bits(inst, 20, 19);
            let opcode = bits(inst, 18, 16);
            match (sf, ty, rmode, opcode) {
                (0, 0, 0, 6) => self.set_xr(rd as u32, self.v[rn] as u32 as u64),
                (0, 0, 0, 7) => self.v[rd] = u128::from(self.xr(rn as u32) as u32),
                (1, 1, 0, 6) => self.set_xr(rd as u32, self.v[rn] as u64),
                (1, 1, 0, 7) => self.v[rd] = u128::from(self.xr(rn as u32)),
                (1, 2, 1, 6) => self.set_xr(rd as u32, (self.v[rn] >> 64) as u64),
                (1, 2, 1, 7) => {
                    let lo = self.v[rd] & u128::from(u64::MAX);
                    self.v[rd] = lo | (u128::from(self.xr(rn as u32)) << 64);
                }
                _ => return undef(),
            }
            return Ok(Flow::Next);
        }
        // FMOV (register), single and double.
        if inst & 0xFF3F_FC00 == 0x1E20_4000 && bits(inst, 23, 22) < 2 {
            let m = if bits(inst, 23, 22) == 0 {
                u128::from(u32::MAX)
            } else {
                u128::from(u64::MAX)
            };
            self.v[rd] = self.v[rn] & m;
            return Ok(Flow::Next);
        }
        // CNT Vd.<8B|16B>, Vn.<T>.
        if inst & 0xBFFF_FC00 == 0x0E20_5800 {
            let src = self.v[rn];
            let mut r = 0u128;
            for i in 0..16 {
                let b = ((src >> (8 * i)) & 0xFF) as u8;
                r |= u128::from(b.count_ones()) << (8 * i);
            }
            self.v[rd] = low(r);
            return Ok(Flow::Next);
        }
        // ADDV <V><d>, Vn.<T> (B, H, and S with Q).
        if inst & 0xBF3F_FC00 == 0x0E31_B800 {
            let size = bits(inst, 23, 22);
            if size == 3 || (size == 2 && !q) {
                return undef();
            }
            let esize = 8u32 << size;
            let n = if q { 128 / esize } else { 64 / esize };
            let emask = (1u128 << esize) - 1;
            let src = self.v[rn];
            let mut sum = 0u128;
            for i in 0..n {
                sum = sum.wrapping_add((src >> (esize * i)) & emask);
            }
            self.v[rd] = sum & emask;
            return Ok(Flow::Next);
        }
        // ADD Dd, Dn, Dm (scalar, 64-bit only).
        if inst & 0xFFE0_FC00 == 0x5EE0_8400 {
            let rm = bits(inst, 20, 16) as usize;
            let r = (self.v[rn] as u64).wrapping_add(self.v[rm] as u64);
            self.v[rd] = u128::from(r);
            return Ok(Flow::Next);
        }
        // ADD Vd.<T>, Vn.<T>, Vm.<T> (vector).
        if inst & 0xBF20_FC00 == 0x0E20_8400 {
            let size = bits(inst, 23, 22);
            if size == 3 && !q {
                return undef();
            }
            let rm = bits(inst, 20, 16) as usize;
            let esize = 8u32 << size;
            let emask = if esize == 64 {
                u128::from(u64::MAX)
            } else {
                (1u128 << esize) - 1
            };
            let (a, b) = (self.v[rn], self.v[rm]);
            let mut r = 0u128;
            let mut i = 0;
            while i < 128 {
                let e = ((a >> i) & emask).wrapping_add((b >> i) & emask) & emask;
                r |= e << i;
                i += esize;
            }
            self.v[rd] = low(r);
            return Ok(Flow::Next);
        }
        // ORR Vd, Vn, Vm (MOV vector alias when Vn == Vm).
        if inst & 0xBFE0_FC00 == 0x0EA0_1C00 {
            let rm = bits(inst, 20, 16) as usize;
            self.v[rd] = low(self.v[rn] | self.v[rm]);
            return Ok(Flow::Next);
        }
        // DUP Vd.<T>, <R>n ; UMOV <R>d, Vn.<Ts>[i] ; INS Vd.<Ts>[i], <R>n.
        let imm5 = bits(inst, 20, 16);
        if imm5 & 0xF != 0 {
            let esize = 8u32 << imm5.trailing_zeros();
            let index = imm5 >> (imm5.trailing_zeros() + 1);
            let emask = if esize == 64 {
                u128::from(u64::MAX)
            } else {
                (1u128 << esize) - 1
            };
            if inst & 0xBFE0_FC00 == 0x0E00_0C00 && esize <= 64 {
                let e = u128::from(self.xr(rn as u32)) & emask;
                let mut r = 0u128;
                let mut i = 0;
                while i < 128 {
                    r |= e << i;
                    i += esize;
                }
                self.v[rd] = low(r);
                return Ok(Flow::Next);
            }
            if inst & 0xBFE0_FC00 == 0x0E00_3C00 && esize <= 64 {
                if (esize == 64) != q {
                    return undef();
                }
                let e = (self.v[rn] >> (esize * index)) & emask;
                self.set_xr(rd as u32, e as u64);
                return Ok(Flow::Next);
            }
            if inst & 0xFFE0_FC00 == 0x4E00_1C00 && esize <= 64 {
                let e = u128::from(self.xr(rn as u32)) & emask;
                let sh = esize * index;
                self.v[rd] = (self.v[rd] & !(emask << sh)) | (e << sh);
                return Ok(Flow::Next);
            }
        }
        undef()
    }
}
