// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Split out of `step_execute` in `cpu/cortex_m.rs` (pure move, no behaviour
// change). See that file's `step_execute` for the shared exception/IT-state
// plumbing that wraps these per-class dispatches.

use super::super::CortexM;
use crate::cpu::cortex_m::{vfp_binop, vfp_fma, AccessWidth, VfpBinOp};
use crate::decoder::arm::Instruction;
use crate::{Bus, SimResult};

impl CortexM {
    #[allow(clippy::too_many_lines)]
    pub(in crate::cpu::cortex_m) fn exec_vfp<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        instruction: Instruction,
        pc_increment: &mut u32,
        _it_block_instruction: &mut bool,
    ) -> SimResult<()> {
        match instruction {
            Instruction::Vldr { sd, rn, imm, add } => {
                let base = self.read_reg(rn);
                let base = if rn == 15 { base & !3 } else { base };
                let addr = if add {
                    base.wrapping_add(imm as u32)
                } else {
                    base.wrapping_sub(imm as u32)
                };
                let val = self.load(bus, addr, AccessWidth::Word)?;
                self.fpu_s[sd as usize] = val;
                *pc_increment = 4;
            }
            Instruction::Vstr { sd, rn, imm, add } => {
                let base = self.read_reg(rn);
                let addr = if add {
                    base.wrapping_add(imm as u32)
                } else {
                    base.wrapping_sub(imm as u32)
                };
                let val = self.fpu_s[sd as usize];
                self.store(bus, addr, AccessWidth::Word, val)?;
                *pc_increment = 4;
            }
            Instruction::VmulF32 { sd, sn, sm } => {
                let a = self.fpu_s[sn as usize];
                let b = self.fpu_s[sm as usize];
                self.fpu_s[sd as usize] = vfp_binop(VfpBinOp::Mul, a, b, self.fpscr);
                *pc_increment = 4;
            }
            Instruction::VaddF32 { sd, sn, sm } => {
                let a = self.fpu_s[sn as usize];
                let b = self.fpu_s[sm as usize];
                self.fpu_s[sd as usize] = vfp_binop(VfpBinOp::Add, a, b, self.fpscr);
                *pc_increment = 4;
            }
            Instruction::VsubF32 { sd, sn, sm } => {
                let a = self.fpu_s[sn as usize];
                let b = self.fpu_s[sm as usize];
                self.fpu_s[sd as usize] = vfp_binop(VfpBinOp::Sub, a, b, self.fpscr);
                *pc_increment = 4;
            }
            Instruction::VdivF32 { sd, sn, sm } => {
                let a = self.fpu_s[sn as usize];
                let b = self.fpu_s[sm as usize];
                self.fpu_s[sd as usize] = vfp_binop(VfpBinOp::Div, a, b, self.fpscr);
                *pc_increment = 4;
            }
            Instruction::VfmaF32 { sd, sn, sm } => {
                let a = self.fpu_s[sn as usize];
                let b = self.fpu_s[sm as usize];
                let c = self.fpu_s[sd as usize];
                // Fused: single rounding of (a*b)+c, not a*b rounded then +c.
                self.fpu_s[sd as usize] = vfp_fma(a, b, c, false, false, self.fpscr);
            }
            Instruction::VfmsF32 { sd, sn, sm } => {
                let a = self.fpu_s[sn as usize];
                let b = self.fpu_s[sm as usize];
                let c = self.fpu_s[sd as usize];
                self.fpu_s[sd as usize] = vfp_fma(a, b, c, true, false, self.fpscr);
            }
            Instruction::VfnmaF32 { sd, sn, sm } => {
                let a = self.fpu_s[sn as usize];
                let b = self.fpu_s[sm as usize];
                let c = self.fpu_s[sd as usize];
                self.fpu_s[sd as usize] = vfp_fma(a, b, c, false, true, self.fpscr);
            }
            Instruction::VfnmsF32 { sd, sn, sm } => {
                let a = self.fpu_s[sn as usize];
                let b = self.fpu_s[sm as usize];
                let c = self.fpu_s[sd as usize];
                self.fpu_s[sd as usize] = vfp_fma(a, b, c, true, true, self.fpscr);
            }
            Instruction::VmovSnRt { sn, rt } => {
                self.fpu_s[sn as usize] = self.read_reg(rt);
                *pc_increment = 4;
            }
            Instruction::VmovRtSn { rt, sn } => {
                self.write_reg(rt, self.fpu_s[sn as usize]);
                *pc_increment = 4;
            }
            Instruction::VmovF32Reg { sd, sm } => {
                self.fpu_s[sd as usize] = self.fpu_s[sm as usize];
                *pc_increment = 4;
            }

            Instruction::VmovF32Imm { sd, imm_bits } => {
                self.fpu_s[sd as usize] = imm_bits;
                *pc_increment = 4;
            }
            Instruction::VcvtF32FromInt {
                sd,
                sm,
                signed,
                fbits,
            } => {
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
                *pc_increment = 4;
            }
            Instruction::VcvtIntFromF32 {
                sd,
                sm,
                signed,
                fbits,
            } => {
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
                *pc_increment = 4;
            }

            // -------- VFP load/store multiple + double-precision (FPv5-D16) --------
            Instruction::VfpStoreMultiple {
                rn,
                s_first,
                count,
                add,
                wback,
            } => {
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
                *pc_increment = 4;
            }
            Instruction::VfpLoadMultiple {
                rn,
                s_first,
                count,
                add,
                wback,
            } => {
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
                *pc_increment = 4;
            }
            Instruction::Vldr64 { dd, rn, imm, add } => {
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
                *pc_increment = 4;
            }
            Instruction::Vstr64 { dd, rn, imm, add } => {
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
                *pc_increment = 4;
            }
            Instruction::VmovF64Reg { dd, dm } => {
                if (dd as usize + 1) < 32 && (dm as usize + 1) < 32 {
                    self.fpu_s[dd as usize] = self.fpu_s[dm as usize];
                    self.fpu_s[dd as usize + 1] = self.fpu_s[dm as usize + 1];
                }
                *pc_increment = 4;
            }
            Instruction::VmovDRtRt2 { dm, rt, rt2 } => {
                if (dm as usize + 1) < 32 {
                    self.fpu_s[dm as usize] = self.read_reg(rt);
                    self.fpu_s[dm as usize + 1] = self.read_reg(rt2);
                }
                *pc_increment = 4;
            }
            Instruction::VmovRtRt2D { rt, rt2, dm } => {
                let (lo, hi) = if (dm as usize + 1) < 32 {
                    (self.fpu_s[dm as usize], self.fpu_s[dm as usize + 1])
                } else {
                    (0, 0)
                };
                self.write_reg(rt, lo);
                self.write_reg(rt2, hi);
                *pc_increment = 4;
            }
            Instruction::VaddF64 { dd, dn, dm } => {
                let r = self.read_f64(dn) + self.read_f64(dm);
                self.write_f64(dd, r);
                *pc_increment = 4;
            }
            Instruction::VsubF64 { dd, dn, dm } => {
                let r = self.read_f64(dn) - self.read_f64(dm);
                self.write_f64(dd, r);
                *pc_increment = 4;
            }
            Instruction::VmulF64 { dd, dn, dm } => {
                let r = self.read_f64(dn) * self.read_f64(dm);
                self.write_f64(dd, r);
                *pc_increment = 4;
            }
            Instruction::VdivF64 { dd, dn, dm } => {
                let r = self.read_f64(dn) / self.read_f64(dm);
                self.write_f64(dd, r);
                *pc_increment = 4;
            }

            _ => unreachable!("exec_vfp called with instruction from a different class"),
        }
        Ok(())
    }
}
