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

use super::super::thumb_expand_imm;
use super::super::AccessWidth;
use super::super::CortexM;
use super::super::PcAdvance;
use crate::Bus;
use crate::Cpu;
use crate::SimResult;
use crate::SimulationError;

impl CortexM {
    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_unknown32<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        h1: u16,
        h2: u16,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        // Manual fallback for complex bit patterns not yet in Instruction enum.
        //
        // LDREX Rt, [Rn, #imm8*4]: T1 = 0xE85F_TFTT where
        //   h1 = 0xE85_ | Rn, h2 = (Rt << 12) | 0xF00 | imm8
        // STREX Rd, Rt, [Rn, #imm8*4]: T1 = 0xE840_TDII where
        //   h1 = 0xE84_ | Rn, h2 = (Rt << 12) | (Rd << 8) | imm8
        //
        // Single-threaded sim has no preemption between LDREX and
        // STREX, so we model the exclusive monitor as always
        // succeeding. This matches the observable behavior of
        // atomic ops on real hardware in the uncontended case.
        if (h1 & 0xFFF0) == 0xE840 && (h2 & 0xF03F) == 0xF000 {
            // TT / TTT / TTA / TTAT — ARMv8-M "Test Target"
            // (ARMv8-M ARM, DDI 0553, "TT, TTT, TTA, TTAT").
            // Encoding T1:
            //   h1 = 0xE84_ | Rn
            //   h2 = 0xF000 | (Rd << 8) | (A << 7) | (T << 6)
            //
            // This SHARES the 0xE84x prefix with STREX, and the
            // STREX arm below only matched on h1. TT sets the
            // STREX Rt field to 0b1111 (a PC destination, which is
            // UNPREDICTABLE for a real STREX), so every TT in the
            // corpus was being executed as
            // `STREX PC, Rd, [Rn, #0]` — a *store of the PC to the
            // address being probed*. TT never accesses memory on
            // real silicon; it only queries the MPU/SAU attributes
            // of an address. That phantom store is a silent
            // memory-corruption bug wherever the probed address is
            // mapped RAM, and it is what crashed the STM32WBA52
            // Zephyr lab at 0x2002_0000: Zephyr's
            // `arm_cmse_mpu_region_get` probes one-past-the-end of
            // a buffer, which TT is explicitly allowed to do
            // because it does not dereference it.
            //
            // Response value: this core models neither an MPU nor
            // the Security Extension (SAU/IDAU), and MPU_CTRL
            // writes land in an inert SCB latch that enforces
            // nothing — so the machine genuinely has no region
            // information to report. The architecturally defined
            // way to say that is MRVALID = 0 (and, with no
            // Security Extension, SREGION/SRVALID/S/IREGION/
            // IRVALID are RES0). A zero response is therefore the
            // honest answer, and it is the conservative one: a
            // caller reads it as "not known to be accessible"
            // rather than being handed a fabricated region number
            // or an unconditional permit. Zephyr's
            // `arm_cmse_mpu_region_get` maps it to -EINVAL, which
            // is exactly what it reports on hardware with the MPU
            // off.
            //
            // A and T select which security state / privilege
            // level is queried; without the Security Extension
            // TTA/TTAT are UNDEFINED, but answering them the same
            // conservative way is still strictly better than
            // falling through to a store.
            let rd = ((h2 >> 8) & 0xF) as u8;
            // Rd = SP or PC is UNPREDICTABLE for TT; do not let a
            // malformed encoding rewrite the stack or program
            // counter. The instruction is still consumed.
            if rd != 13 && rd != 15 {
                self.set_register(rd, 0);
            }
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFF0) == 0xFA90 && (h2 & 0xF0F0) == 0xF010 {
            // ARMv7E-M QADD16: independently signed-saturating add
            // the two halfword lanes (Cortex-M4 DSP extension).
            let rn = (h1 & 0xF) as u8;
            let rd = ((h2 >> 8) & 0xF) as u8;
            let rm = (h2 & 0xF) as u8;
            let a = self.get_register(rn);
            let b = self.get_register(rm);
            let mut saturated = false;
            let lane = |shift: u32, saturated: &mut bool| -> u32 {
                let lhs = ((a >> shift) as u16) as i16 as i32;
                let rhs = ((b >> shift) as u16) as i16 as i32;
                let sum = lhs + rhs;
                let clamped = sum.clamp(i16::MIN as i32, i16::MAX as i32);
                *saturated |= clamped != sum;
                (clamped as i16 as u16 as u32) << shift
            };
            let value = lane(0, &mut saturated) | lane(16, &mut saturated);
            self.set_register(rd, value);
            if saturated {
                self.xpsr |= 1 << 27;
            }
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFF0) == 0xF380 && (h2 & 0x00C0) == 0 {
            // ARMv7-M USAT without an optional shift.
            let rn = (h1 & 0xF) as u8;
            let rd = ((h2 >> 8) & 0xF) as u8;
            let sat = (h2 & 0x1F) as u32;
            let source = self.get_register(rn) as i32;
            let max = if sat == 32 {
                u32::MAX
            } else {
                (1u32 << sat) - 1
            };
            let value = if source < 0 {
                0
            } else {
                (source as u32).min(max)
            };
            if source < 0 || source as u32 > max {
                self.xpsr |= 1 << 27;
            }
            self.set_register(rd, value);
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFF0) == 0xE850 {
            // LDREX
            let rn = (h1 & 0xF) as u8;
            let rt = ((h2 >> 12) & 0xF) as u8;
            let imm8 = (h2 & 0xFF) as u32;
            let addr = self.get_register(rn).wrapping_add(imm8 * 4);
            let val = self.load(bus, addr, AccessWidth::Word)?;
            self.set_register(rt, val);
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFF0) == 0xE840 {
            // STREX
            let rn = (h1 & 0xF) as u8;
            let rt = ((h2 >> 12) & 0xF) as u8;
            let rd = ((h2 >> 8) & 0xF) as u8;
            let imm8 = (h2 & 0xFF) as u32;
            let addr = self.get_register(rn).wrapping_add(imm8 * 4);
            let val = self.get_register(rt);
            self.store(bus, addr, AccessWidth::Word, val)?;
            // Rd = 0 → success.
            self.set_register(rd, 0);
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFF0) == 0xE8D0 && (h2 & 0x0FFF) == 0x0F4F {
            // ARMv7-M LDREXB Rt, [Rn]. Rust uses this for byte
            // atomics such as AtomicBool::compare_exchange.
            let rn = (h1 & 0xF) as u8;
            let rt = ((h2 >> 12) & 0xF) as u8;
            let address = self.get_register(rn);
            let value = self.load(bus, address, AccessWidth::Byte)? as u8;
            self.set_register(rt, u32::from(value));
            self.exclusive_byte = Some((address, value));
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFF0) == 0xE8C0 && (h2 & 0x0FF0) == 0x0F40 {
            // ARMv7-M STREXB Rd, Rt, [Rn]. The single-threaded
            // machine has no contender, so the monitor succeeds.
            let rn = (h1 & 0xF) as u8;
            let rt = ((h2 >> 12) & 0xF) as u8;
            let rd = (h2 & 0xF) as u8;
            let address = self.get_register(rn);
            let reservation_matches = match self.exclusive_byte.take() {
                Some((reserved, value)) if reserved == address => {
                    self.load(bus, address, AccessWidth::Byte)? as u8 == value
                }
                _ => false,
            };
            let succeeds = if reservation_matches {
                self.store(bus, address, AccessWidth::Byte, self.get_register(rt))?;
                true
            } else {
                false
            };
            self.set_register(rd, if succeeds { 0 } else { 1 });
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFF0) == 0xE8D0 && (h2 & 0x0F0F) == 0x0F0F {
            // Load-acquire family (ARMv8-M mainline, also on the
            // M33 with TrustZone off): LDAB/LDAH/LDA and
            // LDAEXB/LDAEXH/LDAEX.
            //   h1 = 0xE8D0 | Rn, h2 = Rt<<12 | 0xF<<8 | sz<<4 | 0xF
            //   sz: 8=B, 9=H, A=word (acquire), C=EXB, D=EXH, E=EX
            // Acquire ordering and the exclusive monitor are
            // no-ops in this single-threaded sim (same rationale
            // as LDREX above). Rust atomics on thumbv8m compile
            // to these — embassy's executor run-queue lives on
            // LDAEX/STLEX.
            let rn = (h1 & 0xF) as u8;
            let rt = ((h2 >> 12) & 0xF) as u8;
            let addr = self.get_register(rn);
            let width = match (h2 >> 4) & 0xF {
                0x8 | 0xC => Some(AccessWidth::Byte),
                0x9 | 0xD => Some(AccessWidth::Half),
                0xA | 0xE => Some(AccessWidth::Word),
                _ => None,
            };
            if let Some(width) = width {
                let val = self.load(bus, addr, width)?;
                self.set_register(rt, val);
            }
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFF0) == 0xE8C0 && (h2 & 0x0F00) == 0x0F00 {
            // Store-release family: STLB/STLH/STL ([3:0]=0xF, no
            // status register) and STLEXB/STLEXH/STLEX ([3:0]=Rd,
            // always-success monitor → Rd = 0).
            //   h1 = 0xE8C0 | Rn, h2 = Rt<<12 | 0xF<<8 | sz<<4 | Rd/0xF
            let rn = (h1 & 0xF) as u8;
            let rt = ((h2 >> 12) & 0xF) as u8;
            let addr = self.get_register(rn);
            let val = self.get_register(rt);
            let sz = (h2 >> 4) & 0xF;
            let width = match sz {
                0x8 | 0xC => Some(AccessWidth::Byte),
                0x9 | 0xD => Some(AccessWidth::Half),
                0xA | 0xE => Some(AccessWidth::Word),
                _ => None,
            };
            if let Some(width) = width {
                self.store(bus, addr, width, val)?;
            }
            if matches!(sz, 0xC..=0xE) {
                let rd = (h2 & 0xF) as u8;
                self.set_register(rd, 0); // success
            }
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFE00) == 0xE800 {
            // Table branch, load/store multiple etc — not yet
            // modeled in full; advance past the 32-bit insn.
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFE00) == 0xF800 {
            // LDR/STR (immediate) T3/T4
            let op1 = (h1 >> 4) & 0xF;
            let rn = (h1 & 0xF) as u8;
            let rt = ((h2 >> 12) & 0xF) as u8;
            let is_t4 = (op1 & 0x8) == 0;
            // Signed (LDRSB.W/LDRSH.W) vs unsigned (LDRB.W/LDRH.W)
            // is selected by h1 bit 8 (0x0100), NOT op1 bit 3:
            // op1 = h1[7:4] excludes bit 8, and op1 bit 3 (=h1 bit 7)
            // is the imm12-form selector used for is_t4 above. Using
            // it for the sign made LDRB.W T2 (0xF89x) sign-extend any
            // byte >= 0x80 (0x85 -> 0xFFFFFF85).
            let is_signed = (h1 & 0x0100) != 0;
            // When Rn=PC (rn==15), T4 form is always the PC-literal encoding
            // (LDR.W Rt, [PC, ±imm12]), never register-offset.
            let is_reg_offset = is_t4 && rn != 15 && (h2 & 0x0800) == 0;
            if !is_reg_offset {
                let mut supported = true;
                let addr: u32;
                let mut wb = false;
                let mut wb_val = 0u32;
                if !is_t4 {
                    // T3: Rn-relative with unsigned imm12
                    let base = if rn == 15 {
                        // PC-literal T3: Align(PC+4, 4) + imm12
                        (self.pc.wrapping_add(4)) & !3
                    } else {
                        self.read_reg(rn)
                    };
                    let offset = (h2 & 0xFFF) as u32;
                    addr = base.wrapping_add(offset);
                } else if rn == 15 {
                    // PC-literal T2: LDR.W Rt, [PC, ±imm12]
                    // U bit is bit 7 of h1; imm12 is h2[11:0]
                    let imm12 = (h2 & 0xFFF) as u32;
                    let u = (h1 >> 7) & 1;
                    let base = (self.pc.wrapping_add(4)) & !3;
                    addr = if u != 0 {
                        base.wrapping_add(imm12)
                    } else {
                        base.wrapping_sub(imm12)
                    };
                } else {
                    let p = (h2 >> 10) & 1;
                    let u = (h2 >> 9) & 1;
                    let w = (h2 >> 8) & 1;
                    let imm8 = (h2 & 0xFF) as i32;
                    let offset = if u != 0 { imm8 } else { -imm8 };
                    let base = self.read_reg(rn);
                    if p != 0 {
                        addr = base.wrapping_add(offset as u32);
                        if w != 0 {
                            wb = true;
                            wb_val = addr;
                        }
                    } else {
                        addr = base;
                        wb = true;
                        wb_val = base.wrapping_add(offset as u32);
                    }
                }
                let mut branch_taken = false;
                match op1 & 0x7 {
                    0 => {
                        let val = self.read_reg(rt) & 0xFF;
                        self.store(bus, addr, AccessWidth::Byte, val)?;
                    }
                    // Rt==15 = PLD/PLI preload hint — NOP (handled by `_`).
                    1 if rt != 15 => {
                        let v = self.load(bus, addr, AccessWidth::Byte)?;
                        let out = if is_signed {
                            (v as u8 as i8) as i32 as u32
                        } else {
                            v
                        };
                        self.write_reg(rt, out);
                    }
                    2 => {
                        let val = self.read_reg(rt) & 0xFFFF;
                        self.store(bus, addr, AccessWidth::Half, val)?;
                    }
                    // Rt==15 = PLDW preload hint — NOP (handled by `_`).
                    3 if rt != 15 => {
                        let v = self.load(bus, addr, AccessWidth::Half)?;
                        let out = if is_signed {
                            (v as u16 as i16) as i32 as u32
                        } else {
                            v
                        };
                        self.write_reg(rt, out);
                    }
                    4 => {
                        let val = self.read_reg(rt);
                        self.store(bus, addr, AccessWidth::Word, val)?;
                    }
                    5 => {
                        let v = self.load(bus, addr, AccessWidth::Word)?;
                        if rt == 15 {
                            if wb {
                                self.write_reg(rn, wb_val);
                                wb = false;
                            }
                            self.branch_to(v, bus)?;
                            branch_taken = true;
                        } else {
                            self.write_reg(rt, v);
                        }
                    }
                    _ => {
                        supported = false;
                    }
                }
                if supported {
                    if wb {
                        self.write_reg(rn, wb_val);
                    }
                    // Rt==15 load is a branch; suppress pc_increment
                    // (see the register-offset path below).
                    if branch_taken {
                        __pc = PcAdvance::Zero;
                    } else {
                        __pc = PcAdvance::Add4;
                    }
                }
            } else {
                // Register offset (T2)
                let rn = (h1 & 0xF) as u8;
                let rt = ((h2 >> 12) & 0xF) as u8;
                let rm = (h2 & 0xF) as u8;
                let imm2 = ((h2 >> 4) & 0x3) as u8;
                let base = self.read_reg(rn);
                let offset = self.read_reg(rm).wrapping_shl(imm2 as u32);
                let addr = base.wrapping_add(offset);
                let mut branch_taken = false;
                match op1 & 0x7 {
                    0 => {
                        let val = self.read_reg(rt) & 0xFF;
                        self.store(bus, addr, AccessWidth::Byte, val)?;
                    }
                    // Rt==15 = PLD/PLI preload hint — NOP (handled by `_`).
                    1 if rt != 15 => {
                        let v = self.load(bus, addr, AccessWidth::Byte)?;
                        let out = if is_signed {
                            (v as u8 as i8) as i32 as u32
                        } else {
                            v
                        };
                        self.write_reg(rt, out);
                    }
                    2 => {
                        let val = self.read_reg(rt) & 0xFFFF;
                        self.store(bus, addr, AccessWidth::Half, val)?;
                    }
                    // Rt==15 = PLDW preload hint — NOP (handled by `_`).
                    3 if rt != 15 => {
                        let v = self.load(bus, addr, AccessWidth::Half)?;
                        let out = if is_signed {
                            (v as u16 as i16) as i32 as u32
                        } else {
                            v
                        };
                        self.write_reg(rt, out);
                    }
                    4 => {
                        let val = self.read_reg(rt);
                        self.store(bus, addr, AccessWidth::Word, val)?;
                    }
                    5 => {
                        let v = self.load(bus, addr, AccessWidth::Word)?;
                        if rt == 15 {
                            self.branch_to(v, bus)?;
                            branch_taken = true;
                        } else {
                            self.write_reg(rt, v);
                        }
                    }
                    _ => {}
                }
                // A load into PC (Rt==15) is a branch: branch_to
                // already set PC, so the 32-bit pc_increment must be
                // suppressed (same contract as Bx). Leaving it at 4
                // landed PC one halfword past the target — this broke
                // GCC switch jump tables (`ldr.w pc,[rn,rm,lsl#n]`).
                if branch_taken {
                    __pc = PcAdvance::Zero;
                } else {
                    __pc = PcAdvance::Add4;
                }
            }
        } else if (h1 & 0xFB00) == 0xF000 && (h2 & 0x8000) == 0 {
            // Data-processing (modified immediate) - repeated here for safety but usually handled by DataProcImm32
            let i = (h1 >> 10) & 0x1;
            let op = ((h1 >> 5) & 0xF) as u8;
            let s = ((h1 >> 4) & 0x1) != 0;
            let rn = (h1 & 0xF) as u8;
            let imm3 = (h2 >> 12) & 0x7;
            let rd = ((h2 >> 8) & 0xF) as u8;
            let imm8 = h2 & 0xFF;
            let imm12 = (i << 11) | (imm3 << 8) | imm8;
            let imm32 = thumb_expand_imm(imm12 as u32);
            let op1 = self.read_reg(rn);
            let mut result = 0u32;
            let mut update_rd = true;
            match op {
                0x0 => result = op1 & imm32,                                  // AND
                0x1 => result = op1 & !imm32,                                 // BIC
                0x2 => result = if rn == 15 { imm32 } else { op1 | imm32 },   // ORR/MOV
                0x3 => result = if rn == 15 { !imm32 } else { op1 | !imm32 }, // ORN/MVN
                0x4 => result = op1 ^ imm32,                                  // EOR
                0x8 => result = op1.wrapping_add(imm32),                      // ADD
                0xD => result = op1.wrapping_sub(imm32),                      // SUB
                _ => update_rd = false,
            }
            if update_rd {
                if rd != 15 {
                    self.write_reg(rd, result);
                }
                if s {
                    self.update_nz(result);
                }
                __pc = PcAdvance::Add4;
            }
        } else if (h1 & 0xFB00) == 0xF100 && (h2 & 0x8000) == 0 {
            // Data-processing (plain binary immediate)
            let i = (h1 >> 10) & 0x1;
            let op = ((h1 >> 5) & 0xF) as u8;
            let rn = (h1 & 0xF) as u8;
            let imm3 = (h2 >> 12) & 0x7;
            let rd = ((h2 >> 8) & 0xF) as u8;
            let imm8 = h2 & 0xFF;
            let imm12 = (i << 11) | (imm3 << 8) | imm8;
            let op1 = self.read_reg(rn);
            match op {
                0x0 => {
                    self.write_reg(rd, op1.wrapping_add(imm12 as u32));
                    __pc = PcAdvance::Add4;
                } // ADD
                0xA => {
                    self.write_reg(rd, op1.wrapping_sub(imm12 as u32));
                    __pc = PcAdvance::Add4;
                } // SUB
                _ => {}
            }
        } else if (h1 & 0xF000) == 0xF000 && (h2 & 0x8000) == 0x8000 {
            // B.W / BL (handled elsewhere but just in case)
            __pc = PcAdvance::Add4;
        } else {
            tracing::warn!(
                "Unknown 32-bit instruction at {:#x}: {:#x} {:#x}",
                self.pc,
                h1,
                h2
            );
            crate::fidelity::record_undecoded(
                self.pc,
                ((h1 as u64) << 16) | (h2 as u64),
                "undecoded T32",
            );
            // As for T16 above: fault rather than skip.
            self.pending_undef_instruction = true;
            return Err(SimulationError::DecodeError(self.pc as u64));
        }
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_unknown(&mut self, op: u16) -> SimResult<PcAdvance> {
        tracing::warn!("Unknown instruction at {:#x}: Opcode {:#06x}", self.pc, op);
        crate::fidelity::record_undecoded(self.pc, op as u64, "undecoded T16");
        // Silicon raises UsageFault (UNDEFINSTR) here. This used to
        // `pc_increment = 2` and carry on, which left every register
        // stale and the run ending green — see the note on
        // `escalate_undefined_instruction`.
        self.pending_undef_instruction = true;
        Err(SimulationError::DecodeError(self.pc as u64))
    }
}
