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
    /// Store-exclusive check against the local monitor (every width, every
    /// acquire/release form): consume the reservation and report whether it
    /// still covers `addr`/`width` with the value the load-exclusive saw.
    /// No reservation (none taken, or cleared by exception entry) fails.
    fn take_exclusive_reservation<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        addr: u32,
        width: AccessWidth,
    ) -> SimResult<bool> {
        Ok(match self.exclusive_subword.take() {
            Some((reserved, reserved_width, value))
                if reserved == addr && reserved_width == width =>
            {
                self.load(bus, addr, width)? == value
            }
            _ => false,
        })
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_unknown32<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        h1: u16,
        h2: u16,
    ) -> SimResult<PcAdvance> {
        // RP2350 DCP (p4/p5): only when the chip attached one; otherwise the
        // instruction falls through to the UNDEFINED fault below.
        if self.dcp.is_some() {
            let decoded = super::super::rp2350_dcp::decode(h1, h2, |r| self.get_register(r));
            if let Some((inst, op, rt, rt2)) = decoded {
                let out = self.dcp.as_mut().and_then(|d| d.inst[inst].exec(op));
                match out {
                    Some(super::super::rp2350_dcp::DcpOut::None) => return Ok(PcAdvance::Add4),
                    Some(super::super::rp2350_dcp::DcpOut::One(v)) => {
                        if rt == 15 {
                            // MRC to APSR_nzcv: flags from bits 31:28.
                            self.xpsr = (self.xpsr & !0xF000_0000) | (v & 0xF000_0000);
                        } else {
                            self.set_register(rt, v);
                        }
                        return Ok(PcAdvance::Add4);
                    }
                    Some(super::super::rp2350_dcp::DcpOut::Two(lo, hi)) => {
                        self.set_register(rt, lo);
                        self.set_register(rt2, hi);
                        return Ok(PcAdvance::Add4);
                    }
                    None => {}
                }
            }
        }
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
        } else if (h1 & 0xFF80) == 0xF300 && (h2 & 0x8000) == 0 {
            // ARMv7-M SSAT: signed saturate with an optional LSL/ASR shift.
            // The radio library's RSSI calibration uses it (`ssat r0, #7, r3`).
            let rn = (h1 & 0xF) as u8;
            let rd = ((h2 >> 8) & 0xF) as u8;
            let sat_imm = (h2 & 0x1F) as u32;
            let shift = ((((h2 >> 12) & 0x7) << 2) | ((h2 >> 6) & 0x3)) as u32;
            let asr = (h1 & 0x20) != 0;
            let raw = self.get_register(rn) as i32;
            let shifted = if asr {
                raw >> shift
            } else {
                raw.wrapping_shl(shift)
            };
            let bits = sat_imm + 1;
            let (min, max) = if bits >= 32 {
                (i32::MIN, i32::MAX)
            } else {
                let half = 1i32 << (bits - 1);
                (-half, half - 1)
            };
            let clamped = shifted.clamp(min, max);
            self.set_register(rd, clamped as u32);
            if clamped != shifted {
                self.xpsr |= 1 << 27;
            }
            __pc = PcAdvance::Add4;
        } else if matches!(h1 & 0xFFBF, 0xEEB4 | 0xEEB5)
            && (h2 & 0x0F50) == 0x0A40
            && ((h1 & 1) == 0 || (h2 & 0x002F) == 0)
        {
            // VCMP{E}.F32 Sd, Sm (T1, h1[0] = 0) and VCMP{E}.F32 Sd, #0.0
            // (T2, h1[0] = 1, h2[5] = M and h2[3:0] = Vm must be zero):
            //   h1 = 1110 1110 1D11 010x, h2 = Vd 1010 E1M0 Vm.
            // Writes FPSCR.NZCV only; the E (quiet-NaN-raises-Invalid) bit
            // matters only for the cumulative IOC flag, which is not
            // modelled. Emitted by GCC for every float `<`/`>` on
            // -mfloat-abi=hard (CODAL micro:bit V2, `vcmpe.f32 s15, s14`
            // then `vmrs APSR_nzcv, fpscr`).
            let d = ((h1 >> 6) & 1) as u8;
            let vd = ((h2 >> 12) & 0xF) as u8;
            let sd = (vd << 1) | d;
            let a = self.fpu_s[sd as usize];
            let b = if (h1 & 1) == 0 {
                let m = ((h2 >> 5) & 1) as u8;
                let vm = (h2 & 0xF) as u8;
                self.fpu_s[((vm << 1) | m) as usize]
            } else {
                0 // +0.0
            };
            let nzcv = crate::cpu::cortex_m::vfp_compare_nzcv(a, b, self.fpscr);
            self.fpscr = (self.fpscr & !crate::cpu::cortex_m::FPSCR_NZCV_MASK) | nzcv;
            __pc = PcAdvance::Add4;
        } else if matches!(h1 & 0xFFBF, 0xEEB0 | 0xEEB1)
            && matches!(h2 & 0x0FD0, 0x0A40 | 0x0AC0)
            && !((h1 & 1) == 0 && (h2 & 0x0080) == 0)
        {
            // Single-precision unary ops (T1), h1 = 1110 1110 1D11 000x,
            // h2 = Vd 1010 x1M0 Vm:
            //   h1[0] = 0, h2[7] = 1  VABS.F32   (h2[7] = 0 is VMOV.F32 reg,
            //                                     decoded elsewhere; excluded)
            //   h1[0] = 1, h2[7] = 0  VNEG.F32
            //   h1[0] = 1, h2[7] = 1  VSQRT.F32
            // VABS/VNEG are FPAbs/FPNeg: sign-bit operations that neither
            // flush denormals nor quiet NaNs. VSQRT is arithmetic.
            let d = ((h1 >> 6) & 1) as u8;
            let vd = ((h2 >> 12) & 0xF) as u8;
            let m = ((h2 >> 5) & 1) as u8;
            let vm = (h2 & 0xF) as u8;
            let sd = ((vd << 1) | d) as usize;
            let a = self.fpu_s[((vm << 1) | m) as usize];
            self.fpu_s[sd] = match ((h1 & 1) != 0, (h2 & 0x0080) != 0) {
                (false, _) => a & 0x7FFF_FFFF,
                (true, false) => a ^ 0x8000_0000,
                (true, true) => crate::cpu::cortex_m::vfp_sqrt(a, self.fpscr),
            };
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFB0) == 0xEE20 && (h2 & 0x0F50) == 0x0A40 {
            // VNMUL.F32 Sd, Sn, Sm (T1): h1 = 1110 1110 0D10 Vn,
            // h2 = Vd 1010 N1M0 Vm (VMUL is the same with h2[6] = 0).
            // FPNeg(FPMul(n, m)): the product is rounded (and FZ/DN
            // applied) first, then its sign is flipped — a NaN result
            // included, as the pseudocode negates whatever FPMul returned.
            let d = ((h1 >> 6) & 1) as u8;
            let vn = (h1 & 0xF) as u8;
            let n = ((h2 >> 7) & 1) as u8;
            let vd = ((h2 >> 12) & 0xF) as u8;
            let m = ((h2 >> 5) & 1) as u8;
            let vm = (h2 & 0xF) as u8;
            let a = self.fpu_s[((vn << 1) | n) as usize];
            let b = self.fpu_s[((vm << 1) | m) as usize];
            let product = crate::cpu::cortex_m::vfp_binop(
                crate::cpu::cortex_m::VfpBinOp::Mul,
                a,
                b,
                self.fpscr,
            );
            self.fpu_s[((vd << 1) | d) as usize] = product ^ 0x8000_0000;
            __pc = PcAdvance::Add4;
        } else if h1 == 0xEEF1 && (h2 & 0x0FFF) == 0x0A10 {
            // VMRS Rt, FPSCR (T1): h2 = Rt 1010 0001 0000. Rt = 15 is the
            // `VMRS APSR_nzcv, FPSCR` form: copy FPSCR[31:28] into the APSR
            // condition flags, which is how a float compare reaches a branch.
            let rt = ((h2 >> 12) & 0xF) as u8;
            if rt == 15 {
                let mask = crate::cpu::cortex_m::FPSCR_NZCV_MASK;
                self.xpsr = (self.xpsr & !mask) | (self.fpscr & mask);
            } else if rt != 13 {
                // Rt = SP is UNPREDICTABLE; leave SP alone.
                self.set_register(rt, self.fpscr);
            }
            __pc = PcAdvance::Add4;
        } else if h1 == 0xEEE1 && (h2 & 0x0FFF) == 0x0A10 {
            // VMSR FPSCR, Rt (T1): h2 = Rt 1010 0001 0000.
            let rt = ((h2 >> 12) & 0xF) as u8;
            if rt != 13 && rt != 15 {
                self.fpscr = self.get_register(rt) & crate::cpu::cortex_m::FPSCR_WRITABLE_MASK;
            }
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFF0) == 0xE850 {
            // LDREX
            let rn = (h1 & 0xF) as u8;
            let rt = ((h2 >> 12) & 0xF) as u8;
            let imm8 = (h2 & 0xFF) as u32;
            let addr = self.get_register(rn).wrapping_add(imm8 * 4);
            let val = self.load(bus, addr, AccessWidth::Word)?;
            self.set_register(rt, val);
            // Opens the local exclusive monitor (one per PE, shared by every
            // load-exclusive form, acquire or not).
            self.exclusive_subword = Some((addr, AccessWidth::Word, val));
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFF0) == 0xE840 {
            // STREX: succeeds iff the reservation taken by LDREX / LDAEX is
            // still intact (same address and width, value unchanged, no
            // exception entry in between). Rd = 0 success, 1 failure.
            let rn = (h1 & 0xF) as u8;
            let rt = ((h2 >> 12) & 0xF) as u8;
            let rd = ((h2 >> 8) & 0xF) as u8;
            let imm8 = (h2 & 0xFF) as u32;
            let addr = self.get_register(rn).wrapping_add(imm8 * 4);
            let val = self.get_register(rt);
            let succeeds = self.take_exclusive_reservation(bus, addr, AccessWidth::Word)?;
            if succeeds {
                self.store(bus, addr, AccessWidth::Word, val)?;
            }
            self.set_register(rd, if succeeds { 0 } else { 1 });
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFF0) == 0xE8D0 && matches!(h2 & 0x0FFF, 0x0F4F | 0x0F5F) {
            // ARMv7-M LDREXB / LDREXH Rt, [Rn] (A7.7.53 / A7.7.54, T1):
            //   h1 = 0xE8D0 | Rn, h2 = Rt<<12 | 0xF<<8 | sz<<4 | 0xF,
            //   sz = 4 (B) / 5 (H). Rust uses LDREXB for byte atomics such
            //   as AtomicBool::compare_exchange; CODAL (micro:bit V2) uses
            //   LDREXH for its 16-bit reference counts. These MUST be
            //   matched before the load-acquire arm below, which shares the
            //   0x0F0F mask and has no width for sz 4/5 — it used to retire
            //   LDREXH as a silent no-op.
            let rn = (h1 & 0xF) as u8;
            let rt = ((h2 >> 12) & 0xF) as u8;
            let width = if (h2 >> 4) & 0xF == 0x4 {
                AccessWidth::Byte
            } else {
                AccessWidth::Half
            };
            let address = self.get_register(rn);
            let value = self.load(bus, address, width)?;
            self.set_register(rt, value);
            self.exclusive_subword = Some((address, width, value));
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFF0) == 0xE8C0 && matches!(h2 & 0x0FF0, 0x0F40 | 0x0F50) {
            // ARMv7-M STREXB / STREXH Rd, Rt, [Rn] (A7.7.168 / A7.7.169,
            // T1): h2 = Rt<<12 | 0xF<<8 | sz<<4 | Rd. The single-threaded
            // machine has no contender, so the store succeeds iff the
            // reservation taken by the matching LDREX{B,H} is still intact
            // (same address and width, value unchanged, no exception or
            // CLREX-equivalent in between). Must precede the store-release
            // arm below for the same reason as the load side.
            let rn = (h1 & 0xF) as u8;
            let rt = ((h2 >> 12) & 0xF) as u8;
            let rd = (h2 & 0xF) as u8;
            let width = if (h2 >> 4) & 0xF == 0x4 {
                AccessWidth::Byte
            } else {
                AccessWidth::Half
            };
            let address = self.get_register(rn);
            let reservation_matches = self.take_exclusive_reservation(bus, address, width)?;
            let succeeds = if reservation_matches {
                self.store(bus, address, width, self.get_register(rt))?;
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
            // Acquire ordering is a no-op in this single-threaded
            // sim. Rust atomics on thumbv8m compile to these —
            // embassy's executor run-queue lives on LDAEX/STLEX.
            // LDAEXB/LDAEXH/LDAEX open the same local exclusive monitor as
            // LDREXB/LDREXH/LDREX (Armv8-M ARM B9.2: one monitor per PE, set
            // by every load-exclusive, acquire or not), so a following
            // STREXB/STREXH or STLEXB/STLEXH sees the reservation.
            // pico-sdk's RP2350 hw_claim_lock pairs LDAEXB with a plain
            // STREXB; without the reservation that STREXB always
            // reported failure and the claim loop spun forever.
            let rn = (h1 & 0xF) as u8;
            let rt = ((h2 >> 12) & 0xF) as u8;
            let addr = self.get_register(rn);
            let sz = (h2 >> 4) & 0xF;
            let width = match sz {
                0x8 | 0xC => Some(AccessWidth::Byte),
                0x9 | 0xD => Some(AccessWidth::Half),
                0xA | 0xE => Some(AccessWidth::Word),
                _ => None,
            };
            if let Some(width) = width {
                let val = self.load(bus, addr, width)?;
                self.set_register(rt, val);
                if matches!(sz, 0xC..=0xE) {
                    self.exclusive_subword = Some((addr, width, val));
                }
            }
            __pc = PcAdvance::Add4;
        } else if (h1 & 0xFFF0) == 0xE8C0 && (h2 & 0x0F00) == 0x0F00 {
            // Store-release family: STLB/STLH/STL ([3:0]=0xF, no
            // status register) and STLEXB/STLEXH/STLEX ([3:0]=Rd).
            // STLEXB/STLEXH/STLEX check the reservation exactly like
            // STREXB/STREXH/STREX above (it is one monitor).
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
            let succeeds = match (sz, width) {
                (0xC..=0xE, Some(width)) => self.take_exclusive_reservation(bus, addr, width)?,
                _ => true,
            };
            if let (true, Some(width)) = (succeeds, width) {
                self.store(bus, addr, width, val)?;
            }
            if matches!(sz, 0xC..=0xE) {
                let rd = (h2 & 0xF) as u8;
                self.set_register(rd, if succeeds { 0 } else { 1 });
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
        } else if (h1 & 0xF000) == 0xF000 && (h1 & 0xFF00) != 0xFB00 && (h2 & 0x8000) == 0x8000 {
            // Legacy branch/hint fallback. FBxx is multiply space, where
            // h2[15] is part of Ra (or RdLo), not a branch marker. Unsupported
            // or invalid DSP encodings must reach the decode fault below.
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
