// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

// Per-encoding-group blocks of the 32-bit Thumb decoder. The block bodies are
// moved verbatim (same masks, same field extraction, same internal order) from
// `arm.rs`'s former monolithic `decode_thumb_32`; the encoding checks are
// order-dependent, so `decode_thumb_32` calls these in the original block order.

use super::{vfp_dp_regs, vfp_expand_imm8, Instruction};

#[inline(always)]
pub(super) fn decode_system(h1: u16, h2: u16) -> Option<Instruction> {
    // DMB / DSB / ISB — ARMv7-M A7.7.31/30/37. CLREX (func = 2) clears the
    // exclusive monitor, which this sim models as always-succeeding — all
    // four are architectural no-ops here.
    //   h1 = 0xF3BF, h2 = 0x8F_<func>_<option>, func ∈ {2, 4, 5, 6}.
    if h1 == 0xF3BF && (h2 & 0xFF00) == 0x8F00 {
        let func = (h2 >> 4) & 0xF;
        if func == 2 || (4..=6).contains(&func) {
            return Some(Instruction::Barrier);
        }
    }
    // MRS: 1111 0011 1110 1111 <1000 rd4 sysm8>
    //      h1 = 0xF3EF, h2 & 0xF000 == 0x8000, rd in h2[11:8], sysm in h2[7:0].
    if h1 == 0xF3EF && (h2 & 0xF000) == 0x8000 {
        let rd = ((h2 >> 8) & 0xF) as u8;
        let sysm = (h2 & 0xFF) as u8;
        return Some(Instruction::Mrs { rd, sysm });
    }
    // MSR: 1111 0011 100 rn4 <1000 mask4 sysm8>
    //      h1 & 0xFFE0 == 0xF380, h2 & 0xFF00 == 0x8800 (mask=0x88 in upper nibble).
    if (h1 & 0xFFE0) == 0xF380 && (h2 & 0xFF00) == 0x8800 {
        let rn = (h1 & 0xF) as u8;
        let sysm = (h2 & 0xFF) as u8;
        return Some(Instruction::Msr { sysm, rn });
    }

    None
}

#[inline(always)]
pub(super) fn decode_long_multiply(h1: u16, h2: u16) -> Option<Instruction> {
    // SMULL / UMULL / SMLAL / UMLAL — ARMv7-M A7.7.156/204/154/202.
    //   Shared family mask: h1 & 0xFF80 == 0xFB80, h2[7:4] == 0.
    //   h1[7:4] op selector:
    //     SMULL: h1 = 1111_1011_1000_rn4  -> h1[7:4] = 0x8
    //     UMULL: h1 = 1111_1011_1010_rn4  -> h1[7:4] = 0xA
    //     SMLAL: h1 = 1111_1011_1100_rn4  -> h1[7:4] = 0xC
    //     UMLAL: h1 = 1111_1011_1110_rn4  -> h1[7:4] = 0xE
    // UMAAL — ARMv7-M A7.7.203.
    //   h1 = 1111_1011_1110_rn4 (h1[7:4] = 0xE), h2[7:4] = 0110.
    //   (rd_hi:rd_lo) = Rn*Rm + rd_lo + rd_hi.
    if (h1 & 0xFFF0) == 0xFBE0 && (h2 & 0x00F0) == 0x0060 {
        let rn = (h1 & 0xF) as u8;
        let rd_lo = ((h2 >> 12) & 0xF) as u8;
        let rd_hi = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        return Some(Instruction::Umaal {
            rd_lo,
            rd_hi,
            rn,
            rm,
        });
    }
    if (h1 & 0xFF80) == 0xFB80 && (h2 & 0x00F0) == 0x0000 {
        let op = (h1 >> 4) & 0xF;
        let rn = (h1 & 0xF) as u8;
        let rd_lo = ((h2 >> 12) & 0xF) as u8;
        let rd_hi = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        match op {
            0x8 => {
                return Some(Instruction::Smull {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                })
            }
            0xA => {
                return Some(Instruction::Umull {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                })
            }
            0xC => {
                return Some(Instruction::Smlal {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                })
            }
            0xE => {
                return Some(Instruction::Umlal {
                    rd_lo,
                    rd_hi,
                    rn,
                    rm,
                })
            }
            _ => {}
        }
    }

    None
}

#[inline(always)]
pub(super) fn decode_mul_acc_divide_early(h1: u16, h2: u16) -> Option<Instruction> {
    // MLA / MLS — ARMv7-M A7.7.100/101.
    //   h1 & 0xFFF0 == 0xFB00, h2[7:4] distinguishes:
    //     MLA: h2[7:4] = 0x0
    //     MLS: h2[7:4] = 0x1
    //   Encoding: rd in h2[11:8], rn in h1[3:0], rm in h2[3:0], ra in h2[15:12].
    if (h1 & 0xFFF0) == 0xFB00 {
        let ra = ((h2 >> 12) & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rn = (h1 & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        match (h2 >> 4) & 0xF {
            0x0 if ra != 0xF => return Some(Instruction::Mla { rd, rn, rm, ra }),
            0x0 if ra == 0xF => {
                // ra == 0xF is MUL (already handled elsewhere in the decoder tree),
                // fall through to existing logic.
            }
            0x1 => return Some(Instruction::Mls { rd, rn, rm, ra }),
            _ => {}
        }
    }
    // SMLABB/BT/TB/TT, SMULBB/BT/TB/TT — ARMv7-M A7.7.166 / A7.7.171.
    //
    //   1111 1011 0001 nnnn | aaaa dddd 00 N M mmmm
    //
    // ⚠️ NOT an exotic DSP intrinsic nobody's code reaches. This is what GCC
    // emits at -Os for `base + K * index` when it can prove both halves fit in
    // 16 bits, and Arduino's `digitalWrite` is exactly that shape — the GPIO
    // port base is `0x4003C000 + 0x30 * (pin >> 4)`. Undecoded, it was a silent
    // no-op (see `record_undecoded`), so `digitalWrite` computed a garbage port
    // address and BLINK DID NOT BLINK on EFR32MG26 while every other cell of
    // that board's Arduino column passed.
    //
    // h2[7:6] must be 00: 01/10/11 in that field are SMLAD/SMLAWx/SMLSD, which
    // are different instructions and are still undecoded rather than
    // approximated by this one.
    if (h1 & 0xFFF0) == 0xFB10 && (h2 & 0x00C0) == 0 {
        let ra = ((h2 >> 12) & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rn = (h1 & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        return Some(Instruction::SmlaXy {
            rd,
            rn,
            rm,
            ra,
            n_top: (h2 & 0x0020) != 0,
            m_top: (h2 & 0x0010) != 0,
            // Ra == 0b1111 is the SMUL form: no addend, and Ra is an encoding
            // marker rather than a register to read.
            accumulate: ra != 0xF,
        });
    }

    None
}

#[inline(always)]
pub(super) fn decode_vfp_single(h1: u16, h2: u16) -> Option<Instruction> {
    // -------------------------------------------------------------
    // VFPv4 single-precision (FPU) — must be checked before the
    // generic 32-bit data-processing matcher because the encoding
    // shares h1[15:12] = 1110 with several non-FPU patterns.
    //
    // All single-precision encodings have h2[11:8] = 1010. Double-
    // precision (h2[11:8] = 1011) is intentionally not handled and
    // falls through to Unknown32.
    // -------------------------------------------------------------

    // VLDR.F32 / VSTR.F32 (T1):
    //   1110 1101 UD 0L nnnn dddd 1010 imm8
    //   h1 = 0xED?? where bits[11:8]=1101, h1[5]=0, h1[4]=L (1=load,
    //   0=store). Mask 0xFF30 fixes the high byte (0xED), bit 5 (=0),
    //   and bit 4 (L); the L bit is what separates VLDR from VSTR.
    if (h1 & 0xFF30) == 0xED10 && (h2 & 0x0F00) == 0x0A00 {
        // VLDR (L = 1)
        let u = (h1 >> 7) & 1;
        let d = (h1 >> 6) & 1;
        let rn = (h1 & 0xF) as u8;
        let vd = ((h2 >> 12) & 0xF) as u8;
        let imm8 = h2 & 0xFF;
        let sd = (vd << 1) | (d as u8);
        let imm = imm8 << 2;
        let add = u != 0;
        return Some(Instruction::Vldr { sd, rn, imm, add });
    }
    if (h1 & 0xFF30) == 0xED00 && (h2 & 0x0F00) == 0x0A00 {
        // VSTR (L = 0)
        let u = (h1 >> 7) & 1;
        let d = (h1 >> 6) & 1;
        let rn = (h1 & 0xF) as u8;
        let vd = ((h2 >> 12) & 0xF) as u8;
        let imm8 = h2 & 0xFF;
        let sd = (vd << 1) | (d as u8);
        let imm = imm8 << 2;
        let add = u != 0;
        return Some(Instruction::Vstr { sd, rn, imm, add });
    }
    // VMUL.F32 / VADD.F32 / VSUB.F32 / VDIV.F32 — three-register
    // single-precision floating-point arithmetic (T2):
    //   1110 1110 oDpQ nnnn dddd 1010 N0M0 mmmm
    // op selector in h1[7:4]: 0010=VMUL, 0011=VADD/VSUB (op_b in h2[6]),
    //   1000=VDIV. These are the four most common Cortex-M4F float ops
    //   the C compiler emits for `*`, `+`, `-`, `/`.
    if (h1 & 0xFFB0) == 0xEE20 && (h2 & 0x0F50) == 0x0A00 {
        // VMUL.F32: h2 bit 6 must be 0 (otherwise it's VNMUL, not
        // modelled), h2 bit 4 must be 0, h2[11:8]=1010. Bits 7 (N) and
        // 5 (M) are register-number bits that vary per encoding.
        let d = (h1 >> 6) & 1;
        let n = (h2 >> 7) & 1;
        let m = (h2 >> 5) & 1;
        let vn = (h1 & 0xF) as u8;
        let vd = ((h2 >> 12) & 0xF) as u8;
        let vm = (h2 & 0xF) as u8;
        let sd = (vd << 1) | (d as u8);
        let sn = (vn << 1) | (n as u8);
        let sm = (vm << 1) | (m as u8);
        return Some(Instruction::VmulF32 { sd, sn, sm });
    }
    if (h1 & 0xFFB0) == 0xEE30 && (h2 & 0x0F10) == 0x0A00 {
        // VADD.F32 (op_b = 0) / VSUB.F32 (op_b = h2[6] = 1). Mask 0x0F10
        // fixes only h2[11:8] = 1010 and h2[4] = 0; bits 7 (N), 6 (op_b),
        // 5 (M) all vary.
        let d = (h1 >> 6) & 1;
        let n = (h2 >> 7) & 1;
        let m = (h2 >> 5) & 1;
        let op_b = (h2 >> 6) & 1;
        let vn = (h1 & 0xF) as u8;
        let vd = ((h2 >> 12) & 0xF) as u8;
        let vm = (h2 & 0xF) as u8;
        let sd = (vd << 1) | (d as u8);
        let sn = (vn << 1) | (n as u8);
        let sm = (vm << 1) | (m as u8);
        return Some(if op_b == 0 {
            Instruction::VaddF32 { sd, sn, sm }
        } else {
            Instruction::VsubF32 { sd, sn, sm }
        });
    }
    if (h1 & 0xFFB0) == 0xEE80 && (h2 & 0x0F50) == 0x0A00 {
        // VDIV.F32: same shape as VMUL — fix h2[11:8]=1010, bit 6, bit 4.
        let d = (h1 >> 6) & 1;
        let n = (h2 >> 7) & 1;
        let m = (h2 >> 5) & 1;
        let vn = (h1 & 0xF) as u8;
        let vd = ((h2 >> 12) & 0xF) as u8;
        let vm = (h2 & 0xF) as u8;
        let sd = (vd << 1) | (d as u8);
        let sn = (vn << 1) | (n as u8);
        let sm = (vm << 1) | (m as u8);
        return Some(Instruction::VdivF32 { sd, sn, sm });
    }
    // VFMA.F32 / VFMS.F32 — fused multiply-accumulate (T2):
    //   1110 1110 0D10 nnnn dddd 1010 N0M0 mmmm
    // opc1[23:20] = 1D10 (D free); opc3 = h2[6] selects VFMA (0) / VFMS (1).
    // These are FUSED: the product is not rounded before the addition —
    // see execute() where `f32::mul_add` is used, not `a * b + c`.
    if (h1 & 0xFFB0) == 0xEEA0 && (h2 & 0x0F10) == 0x0A00 {
        let d = (h1 >> 6) & 1;
        let n = (h2 >> 7) & 1;
        let m = (h2 >> 5) & 1;
        let opc3 = (h2 >> 6) & 1;
        let vn = (h1 & 0xF) as u8;
        let vd = ((h2 >> 12) & 0xF) as u8;
        let vm = (h2 & 0xF) as u8;
        let sd = (vd << 1) | (d as u8);
        let sn = (vn << 1) | (n as u8);
        let sm = (vm << 1) | (m as u8);
        return Some(if opc3 == 0 {
            Instruction::VfmaF32 { sd, sn, sm }
        } else {
            Instruction::VfmsF32 { sd, sn, sm }
        });
    }

    // VFNMA.F32 / VFNMS.F32 — fused negated multiply-accumulate/subtract (T2):
    //   1110 1110 0D01 nnnn dddd 1010 N0M0 mmmm
    // opc1[23:20] = 1D01 (D free); opc3 = h2[6] selects VFNMA (0) / VFNMS (1).
    if (h1 & 0xFFB0) == 0xEE90 && (h2 & 0x0F10) == 0x0A00 {
        let d = (h1 >> 6) & 1;
        let n = (h2 >> 7) & 1;
        let m = (h2 >> 5) & 1;
        let opc3 = (h2 >> 6) & 1;
        let vn = (h1 & 0xF) as u8;
        let vd = ((h2 >> 12) & 0xF) as u8;
        let vm = (h2 & 0xF) as u8;
        let sd = (vd << 1) | (d as u8);
        let sn = (vn << 1) | (n as u8);
        let sm = (vm << 1) | (m as u8);
        return Some(if opc3 == 0 {
            Instruction::VfnmaF32 { sd, sn, sm }
        } else {
            Instruction::VfnmsF32 { sd, sn, sm }
        });
    }
    // VMOV (single, GP register ↔ S register) — T1:
    //   1110 1110 000 op nnnn tttt 1010 N0010000
    //   h1 & 0xFFE0 == 0xEE00, op = h1[4] (0 = VMOV Sn,Rt; 1 = VMOV Rt,Sn).
    if (h1 & 0xFFE0) == 0xEE00 && (h2 & 0x0F7F) == 0x0A10 {
        let op = (h1 >> 4) & 1;
        let vn = (h1 & 0xF) as u8;
        let n = (h2 >> 7) & 1;
        let rt = ((h2 >> 12) & 0xF) as u8;
        let sn = (vn << 1) | (n as u8);
        return Some(if op == 0 {
            Instruction::VmovSnRt { sn, rt }
        } else {
            Instruction::VmovRtSn { rt, sn }
        });
    }
    // VMOV.F32 Sd, Sm (T2 register-to-register float move):
    //   1110 1110 1D11 0000 dddd 1010 01M0 mmmm
    if (h1 & 0xFFB0) == 0xEEB0 && (h1 & 0x000F) == 0x0000 && (h2 & 0x0FD0) == 0x0A40 {
        let d = (h1 >> 6) & 1;
        let m = (h2 >> 5) & 1;
        let vd = ((h2 >> 12) & 0xF) as u8;
        let vm = (h2 & 0xF) as u8;
        let sd = (vd << 1) | (d as u8);
        let sm = (vm << 1) | (m as u8);
        return Some(Instruction::VmovF32Reg { sd, sm });
    }
    // VFP unary group (VMOV imm / VCVT int↔float / VCVT fixed): h1 top matches
    // 1110 1110 1D11 xxxx with h2[11:8]=1010. VMOV-reg handled above.
    // Observed Arduino-M33 (H563) startup emissions:
    //   eef7 5a00  vmov.f32 s11, #1.0
    //   eeb8 7ae7  vcvt.f32.s32 s14, s15
    //   eef8 7ae7  vcvt.f32.s32 s15, s15
    //   eefc 7ac7  vcvt.u32.f32 s15, s14
    //   eefa 6ae9  vcvt.f32.s32 s13, s13, #13
    if (h1 & 0xFFB0) == 0xEEB0 && (h2 & 0x0F00) == 0x0A00 {
        let d = (h1 >> 6) & 1;
        let m = (h2 >> 5) & 1;
        let vd = ((h2 >> 12) & 0xF) as u8;
        let vm = (h2 & 0xF) as u8;
        let sd = (vd << 1) | (d as u8);
        let sm = (vm << 1) | (m as u8);
        let lo = (h1 & 0xF) as u8;
        let h2_mid = ((h2 >> 4) & 0xF) as u8; // bits 7:4

        // VMOV.F32 Sd, #imm — h2[7:4]=0000 (imm4L in bits 3:0).
        if h2_mid == 0x0 {
            let imm8 = (lo << 4) | ((h2 & 0xF) as u8);
            return Some(Instruction::VmovF32Imm {
                sd,
                imm_bits: vfp_expand_imm8(imm8),
            });
        }

        // Integer VCVT (no #fbits). opc2 in h1[2:0] with h1[3]=1; op = h2[7].
        if (lo & 0x8) != 0 && lo != 0xA && lo != 0xB {
            let opc2 = lo & 0x7;
            let op = ((h2 >> 7) & 1) as u8;
            match (opc2, op) {
                (0b000, 1) => {
                    return Some(Instruction::VcvtF32FromInt {
                        sd,
                        sm,
                        signed: true,
                        fbits: 0,
                    });
                }
                (0b000, 0) => {
                    return Some(Instruction::VcvtF32FromInt {
                        sd,
                        sm,
                        signed: false,
                        fbits: 0,
                    });
                }
                (0b100, 1) => {
                    return Some(Instruction::VcvtIntFromF32 {
                        sd,
                        sm,
                        signed: false,
                        fbits: 0,
                    });
                }
                (0b101, 1) => {
                    return Some(Instruction::VcvtIntFromF32 {
                        sd,
                        sm,
                        signed: true,
                        fbits: 0,
                    });
                }
                (0b100, 0) | (0b101, 0) => {
                    return Some(Instruction::VcvtF32FromInt {
                        sd,
                        sm,
                        signed: opc2 == 0b101,
                        fbits: 0,
                    });
                }
                _ => {}
            }
        }

        // Fixed-point VCVT (to float).
        if (lo == 0xA || lo == 0xB) && (h2 & 0x0010) == 0 {
            let imm4 = (h2 & 0xF) as u8;
            let odd = ((h2 >> 5) & 1) != 0;
            let pair = 16u8.saturating_sub(imm4);
            let fbits = if odd {
                pair.saturating_mul(2).saturating_sub(1)
            } else {
                pair.saturating_mul(2)
            };
            return Some(Instruction::VcvtF32FromInt {
                sd,
                sm: sd,
                signed: lo == 0xA,
                fbits: fbits.clamp(1, 32),
            });
        }
    }

    None
}

#[inline(always)]
pub(super) fn decode_vfp_double(h1: u16, h2: u16) -> Option<Instruction> {
    // -------- VFP double-precision + load/store multiple (Cortex-M7 FPv5-D16) --------
    // Double-precision shares the encoding of the single-precision ops above but
    // with h2[11:8] = 1011 (single is 1010). A double Dn maps to the S-register
    // pair (2n, 2n+1), so `dd`/`dn`/`dm` below are the LOW S-index (2*Dreg).

    // VLDR.F64 / VSTR.F64 (T1): 1110 1101 UD0L nnnn dddd 1011 imm8
    if (h1 & 0xFF30) == 0xED10 && (h2 & 0x0F00) == 0x0B00 {
        let u = (h1 >> 7) & 1;
        let d = (h1 >> 6) & 1;
        let rn = (h1 & 0xF) as u8;
        let vd = ((h2 >> 12) & 0xF) as u8;
        let dd = (((d as u8) << 4) | vd) << 1;
        return Some(Instruction::Vldr64 {
            dd,
            rn,
            imm: (h2 & 0xFF) << 2,
            add: u != 0,
        });
    }
    if (h1 & 0xFF30) == 0xED00 && (h2 & 0x0F00) == 0x0B00 {
        let u = (h1 >> 7) & 1;
        let d = (h1 >> 6) & 1;
        let rn = (h1 & 0xF) as u8;
        let vd = ((h2 >> 12) & 0xF) as u8;
        let dd = (((d as u8) << 4) | vd) << 1;
        return Some(Instruction::Vstr64 {
            dd,
            rn,
            imm: (h2 & 0xFF) << 2,
            add: u != 0,
        });
    }
    // VLDM/VSTM/VPUSH/VPOP (register list), single (S=0) or double (S=1):
    //   1110 110P UDWL nnnn dddd 101S imm8   (imm8 = number of 32-bit words)
    // The P=1,W=0 offset form is VLDR/VSTR (single-register), matched above.
    if (h1 & 0xFE00) == 0xEC00 && (h2 & 0x0E00) == 0x0A00 {
        let p = (h1 >> 8) & 1;
        let u = (h1 >> 7) & 1;
        let d = (h1 >> 6) & 1;
        let w = (h1 >> 5) & 1;
        let l = (h1 >> 4) & 1;
        let rn = (h1 & 0xF) as u8;
        let vd = ((h2 >> 12) & 0xF) as u8;
        let s = (h2 >> 8) & 1;
        let imm8 = (h2 & 0xFF) as u8;
        if !(p == 1 && w == 0) {
            let s_first = if s == 0 {
                (vd << 1) | (d as u8)
            } else {
                (((d as u8) << 4) | vd) << 1
            };
            let add = u != 0;
            let wback = w != 0;
            return Some(if l == 1 {
                Instruction::VfpLoadMultiple {
                    rn,
                    s_first,
                    count: imm8,
                    add,
                    wback,
                }
            } else {
                Instruction::VfpStoreMultiple {
                    rn,
                    s_first,
                    count: imm8,
                    add,
                    wback,
                }
            });
        }
    }
    // VMOV.F64 Dd, Dm: 1110 1110 1D11 0000 dddd 1011 01M0 mmmm
    if (h1 & 0xFFB0) == 0xEEB0 && (h1 & 0x000F) == 0x0000 && (h2 & 0x0FD0) == 0x0B40 {
        let d = (h1 >> 6) & 1;
        let m = (h2 >> 5) & 1;
        let vd = ((h2 >> 12) & 0xF) as u8;
        let vm = (h2 & 0xF) as u8;
        let dd = (((d as u8) << 4) | vd) << 1;
        let dm = (((m as u8) << 4) | vm) << 1;
        return Some(Instruction::VmovF64Reg { dd, dm });
    }
    // VMOV Dm,Rt,Rt2 (L=0) / VMOV Rt,Rt2,Dm (L=1):
    //   1110 1100 010L tttt tttt 1011 00M1 mmmm
    if (h1 & 0xFFE0) == 0xEC40 && (h2 & 0x0FD0) == 0x0B10 {
        let l = (h1 >> 4) & 1;
        let rt2 = (h1 & 0xF) as u8;
        let rt = ((h2 >> 12) & 0xF) as u8;
        let m = (h2 >> 5) & 1;
        let vm = (h2 & 0xF) as u8;
        let dm = (((m as u8) << 4) | vm) << 1;
        return Some(if l == 1 {
            Instruction::VmovRtRt2D { rt, rt2, dm }
        } else {
            Instruction::VmovDRtRt2 { dm, rt, rt2 }
        });
    }
    // VMUL.F64 / VADD.F64 / VSUB.F64 / VDIV.F64 — three-register double arithmetic.
    if (h1 & 0xFFB0) == 0xEE20 && (h2 & 0x0F50) == 0x0B00 {
        let (dd, dn, dm) = vfp_dp_regs(h1, h2);
        return Some(Instruction::VmulF64 { dd, dn, dm });
    }
    if (h1 & 0xFFB0) == 0xEE30 && (h2 & 0x0F10) == 0x0B00 {
        let (dd, dn, dm) = vfp_dp_regs(h1, h2);
        return Some(if (h2 >> 6) & 1 == 0 {
            Instruction::VaddF64 { dd, dn, dm }
        } else {
            Instruction::VsubF64 { dd, dn, dm }
        });
    }
    if (h1 & 0xFFB0) == 0xEE80 && (h2 & 0x0F50) == 0x0B00 {
        let (dd, dn, dm) = vfp_dp_regs(h1, h2);
        return Some(Instruction::VdivF64 { dd, dn, dm });
    }

    None
}

#[inline(always)]
pub(super) fn decode_dp_modified_imm_early(h1: u16, h2: u16) -> Option<Instruction> {
    // Data processing (modified immediate) / Plain binary immediate
    // 1111 0 <i1> 0 <op> <S> <Rn> 0 <imm3> <Rd> <imm8>
    if (h1 & 0xFB00) == 0xF000 && (h2 & 0x8000) == 0 && (h2 & 0x0700) != 0x0700 {
        let op = ((h1 >> 5) & 0xF) as u8;
        let s = (h1 & 0x0010) != 0;
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;

        let i = (h1 >> 10) & 1;
        let imm3 = (h2 >> 12) & 7;
        let imm8 = h2 & 0xFF;
        let imm12 = (i << 11) | (imm3 << 8) | imm8;

        return Some(Instruction::DataProcImm32 {
            op,
            rn,
            rd,
            imm12: imm12 as u32,
            set_flags: s,
        });
    }

    None
}

#[inline(always)]
pub(super) fn decode_dp_shifted_reg_early(h1: u16, h2: u16) -> Option<Instruction> {
    // Data Processing (Reg) - For LSL, LSR, ASR, ROR, etc
    // 1110 1010 ... (EA..)
    if (h1 & 0xFE00) == 0xEA00 && (h2 & 0x8000) == 0 {
        let op = ((h1 >> 5) & 0xF) as u8;
        let s = ((h1 >> 4) & 0x1) != 0;
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;

        let imm3 = ((h2 >> 12) & 0x7) as u8;
        let imm2 = ((h2 >> 6) & 0x3) as u8;
        let imm5 = (imm3 << 2) | imm2;
        let shift_type = ((h2 >> 4) & 0x3) as u8;

        return Some(Instruction::DataProc32 {
            op,
            rn,
            rd,
            rm,
            imm5,
            shift_type,
            set_flags: s,
        });
    }

    None
}

#[inline(always)]
pub(super) fn decode_dp_modified_imm_late(h1: u16, h2: u16) -> Option<Instruction> {
    // Data-processing (modified immediate) T1: 1111 0 i 00 op4 S rn 0 imm3 rd imm8
    // F0xx or F1xx.
    if ((h1 & 0xFB00) == 0xF000 || (h1 & 0xFB00) == 0xF100) && (h2 & 0x8000) == 0 {
        let i = (h1 >> 10) & 1;
        let op = ((h1 >> 5) & 0xF) as u8;
        let s = (h1 & (1 << 4)) != 0;
        let rn = (h1 & 0xF) as u8;
        let imm3 = ((h2 >> 12) & 0x7) as u32;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let imm8 = (h2 & 0xFF) as u32;

        let imm12 = ((i as u32) << 11) | (imm3 << 8) | imm8;

        return Some(Instruction::DataProcImm32 {
            op,
            rn,
            rd,
            imm12,
            set_flags: s,
        });
    }

    None
}

#[inline(always)]
pub(super) fn decode_dp_plain_imm_early(h1: u16, h2: u16) -> Option<Instruction> {
    // MOVW: 1111 0 i 10 0100 imm4 0 imm3 rd imm8 -> F24..
    // h2[15]==0 distinguishes the plain-immediate data-processing group from the
    // branch/misc-control group (h2[15]==1); without it a B<cond>.W (T3) whose
    // first halfword is 0xF24x (cond=LS) is mis-decoded as MOVW and never branches.
    if (h1 & 0xFBF0) == 0xF240 && (h2 & 0x8000) == 0 {
        let i = (h1 >> 10) & 1;
        let imm4 = h1 & 0xF;
        let imm3 = (h2 >> 12) & 7;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let imm8 = h2 & 0xFF;
        let imm16 = (imm4 << 12) | (i << 11) | (imm3 << 8) | imm8;
        return Some(Instruction::Movw { rd, imm: imm16 });
    }
    // MOVT: 1111 0 i 10 1100 imm4 0 imm3 rd imm8 -> F2C..
    // (h2[15]==0 guard: see MOVW above — keeps B<cond>.W out of this arm.)
    if (h1 & 0xFBF0) == 0xF2C0 && (h2 & 0x8000) == 0 {
        let i = (h1 >> 10) & 1;
        let imm4 = h1 & 0xF;
        let imm3 = (h2 >> 12) & 7;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let imm8 = h2 & 0xFF;
        let imm16 = (imm4 << 12) | (i << 11) | (imm3 << 8) | imm8;
        return Some(Instruction::Movt { rd, imm: imm16 });
    }

    None
}

#[inline(always)]
pub(super) fn decode_dp_shifted_reg_mid(h1: u16, h2: u16) -> Option<Instruction> {
    // Shift by register (Thumb-2): LSL.W path (kept for the H563 LSL-only
    // case; same bug as below was here — moved op extraction to h1[6:5]).
    if (h1 & 0xFFE0) == 0xFA00 && (h2 & 0xF0F0) == 0xF000 {
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        let shift_type = ((h1 >> 5) & 0x3) as u8;
        return Some(Instruction::ShiftReg32 {
            rd,
            rn,
            rm,
            shift_type,
        });
    }

    None
}

#[inline(always)]
pub(super) fn decode_bitfield_early(h1: u16, h2: u16) -> Option<Instruction> {
    // 1. Bitfield and Miscellaneous Instructions
    // Encoding: 1111 0011 0110 ... => F36x ...
    if (h1 & 0xFFF0) == 0xF360 {
        let _op = (h1 & 0xF) as u8;
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;

        // BFI / BFC
        if (h2 & 0x8000) == 0 {
            let lsbbb = ((h2 >> 12) & 0x7) << 2 | ((h2 >> 6) & 0x3);
            // Encoding of msb in h2 is mmmmm
            let msb = (h2 & 0x1F) as u8;
            let lsb = lsbbb as u8; // 5 bits

            // Width = msb - lsb + 1
            // If msb < lsb, it's UNPREDICTABLE (or handled as 0 length?)
            if msb >= lsb {
                let width = msb - lsb + 1;
                if rn == 0xF {
                    return Some(Instruction::Bfc { rd, lsb, width });
                } else {
                    return Some(Instruction::Bfi { rd, rn, lsb, width });
                }
            }
        }
    }

    None
}

#[inline(always)]
pub(super) fn decode_dp_plain_imm_late(h1: u16, h2: u16) -> Option<Instruction> {
    // ADDW (T4 plain immediate, Rn != PC) / ADR.W (T3, Rn == PC, positive offset):
    //   1111 0 i 10 0000 nnnn 0 imm3 dddd imm8 -> F200..F20F (with i bit at 10).
    // imm12 = i:imm3:imm8, zero-extended to 32 bits — NOT ThumbExpandImm.
    // (h2[15]==0 guard: see MOVW above — F20x also collides with B<cond>.W, cond=HI.)
    if (h1 & 0xFBF0) == 0xF200 && (h2 & 0x8000) == 0 {
        let i = (h1 >> 10) & 1;
        let rn = (h1 & 0xF) as u8;
        let imm3 = (h2 >> 12) & 7;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let imm8 = h2 & 0xFF;
        let imm12 = (i << 11) | (imm3 << 8) | imm8;
        if rn == 15 {
            return Some(Instruction::Adr { rd, imm: imm12 });
        } else {
            return Some(Instruction::AddwImm { rd, rn, imm: imm12 });
        }
    }
    // SUBW (T4) / ADR.W (T2, Rn == PC, negative offset):
    //   1111 0 i 10 1010 nnnn 0 imm3 dddd imm8 -> F2A0..F2AF.
    // (h2[15]==0 guard: see MOVW above — keeps B<cond>.W out of this arm.)
    if (h1 & 0xFBF0) == 0xF2A0 && (h2 & 0x8000) == 0 {
        let i = (h1 >> 10) & 1;
        let rn = (h1 & 0xF) as u8;
        let imm3 = (h2 >> 12) & 7;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let imm8 = h2 & 0xFF;
        let imm12 = (i << 11) | (imm3 << 8) | imm8;
        if rn == 15 {
            // ADR.W with negative offset — encoded as PC - imm. The
            // existing Instruction::Adr models PC + imm, so we encode the
            // 'negative' form by leaving it to the caller for now and
            // emitting Adr with raw imm12 (callers that hit Rn==PC for
            // SUBW are rare in our targets — file an issue if needed).
            return Some(Instruction::Adr { rd, imm: imm12 });
        } else {
            return Some(Instruction::SubwImm { rd, rn, imm: imm12 });
        }
    }

    None
}

#[inline(always)]
pub(super) fn decode_ldst_single(h1: u16, h2: u16) -> Option<Instruction> {
    // LDR (immediate) T4: 1111 1000 0101 Rn | Rt 1 P U W imm8 -> F85x.
    // h2 bit 11 == 1 marks the indexed/writeback (T4) form; distinguished from
    // the LDR (register) T2 form (h2 bit 11 == 0). The plain positive-offset
    // case (P=1,U=1,W=0) overlaps T3 semantics but is encoded here on F85x.
    // Models `ldr.w pc, [sp], #4` (P=0,U=1,W=1) — the function-return idiom
    // that was previously Unknown32, silently skipping the return branch.
    if (h1 & 0xFFF0) == 0xF850 && (h2 & 0x0800) != 0 {
        let rn = (h1 & 0xF) as u8;
        let rt = ((h2 >> 12) & 0xF) as u8;
        let pre_index = (h2 & 0x0400) != 0; // P
        let add = (h2 & 0x0200) != 0; // U
        let writeback = (h2 & 0x0100) != 0; // W
        let imm8 = (h2 & 0xFF) as u8;
        return Some(Instruction::LdrImm32Idx {
            rt,
            rn,
            imm8,
            pre_index,
            add,
            writeback,
        });
    }
    // STR (immediate) T4: 1111 1000 0100 Rn | Rt 1 P U W imm8 -> F84x.
    // Same h2 bit-11 marker. Models `str.w rt,[rn],#imm` / `[rn,#imm]!`.
    if (h1 & 0xFFF0) == 0xF840 && (h2 & 0x0800) != 0 {
        let rn = (h1 & 0xF) as u8;
        let rt = ((h2 >> 12) & 0xF) as u8;
        let pre_index = (h2 & 0x0400) != 0;
        let add = (h2 & 0x0200) != 0;
        let writeback = (h2 & 0x0100) != 0;
        let imm8 = (h2 & 0xFF) as u8;
        return Some(Instruction::StrImm32Idx {
            rt,
            rn,
            imm8,
            pre_index,
            add,
            writeback,
        });
    }
    // LDR.W (immediate) (T3): 1111 1000 1101 ... -> F8D..
    if (h1 & 0xFFF0) == 0xF8D0 {
        let rn = (h1 & 0xF) as u8;
        let rt = ((h2 >> 12) & 0xF) as u8;
        let imm12 = h2 & 0xFFF;
        return Some(Instruction::LdrImm32 { rt, rn, imm12 });
    }
    // STR.W (immediate) (T3): 1111 1000 1100 ... -> F8C..
    if (h1 & 0xFFF0) == 0xF8C0 {
        let rn = (h1 & 0xF) as u8;
        let rt = ((h2 >> 12) & 0xF) as u8;
        let imm12 = h2 & 0xFFF;
        return Some(Instruction::StrImm32 { rt, rn, imm12 });
    }

    None
}

#[inline(always)]
pub(super) fn decode_bitfield_late(h1: u16, h2: u16) -> Option<Instruction> {
    // SBFX / UBFX
    // (h2[15]==0 guard: see MOVW above — F34x also collides with B<cond>.W, cond=LE.)
    if ((h1 & 0xFFF0) == 0xF340 || (h1 & 0xFFF0) == 0xF3C0) && (h2 & 0x8000) == 0 {
        let is_unsigned = (h1 & 0x0080) != 0; // F3C0 vs F340 (0x0080 bit)
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;

        let lsb = (((h2 >> 12) & 0x7) << 2 | ((h2 >> 6) & 0x3)) as u8; // 5 bits
        let width_m1 = (h2 & 0x1F) as u8;
        let width = width_m1 + 1;

        if is_unsigned {
            return Some(Instruction::Ubfx { rd, rn, lsb, width });
        } else {
            return Some(Instruction::Sbfx { rd, rn, lsb, width });
        }
    }

    None
}

#[inline(always)]
pub(super) fn decode_extend_misc_early(h1: u16, h2: u16) -> Option<Instruction> {
    // Register-extend, wide (T2): the plain {S,U}XT{B,H}.W (Rn=0xF) and the
    // extend-and-add {S,U}XTA{B,H}.W (Rn!=0xF).
    //   h1 = 1111 1010 0 op nnnn ; op: 000=SXT(A)H 001=UXT(A)H 100=SXT(A)B
    //   101=UXT(A)B (010/011 = ..B16, not modeled). nnnn = Rn (0xF = no add).
    //   h2 = 1111 dddd 10 rr mmmm  (rr = rotate/8).
    // clang emits e.g. `uxth.w r2, ip` = FA1F F28C (extend via high register)
    // and `uxtah r6, r3, r0` = FA13 F680 (4 + path_len). Without this the insn
    // decoded to Unknown32 and was skipped, leaving Rd stale.
    if (h1 & 0xFF80) == 0xFA00 && (h2 & 0xF080) == 0xF080 {
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        let rotate = (((h2 >> 4) & 0x3) * 8) as u8;
        let op = ((h1 >> 4) & 0x7) as u8;
        if op == 0b000 || op == 0b001 || op == 0b100 || op == 0b101 {
            return Some(Instruction::ExtendW {
                rd,
                rn,
                rm,
                rotate,
                op,
            });
        }
    }
    // Misc Instructions: REV, REV16, REVSH, CLZ, RBIT
    // All start with 1111 1010 ... (FA..)
    if (h1 & 0xFF80) == 0xFA80 {
        let rn = (h1 & 0xF) as u8; // Rm in decoding usually
        let rm = rn; // Encoding uses Rm in H1

        let rd = ((h2 >> 8) & 0xF) as u8;

        if (h1 & 0xFFF0) == 0xFA90 {
            // FA9m
            let op = (h2 >> 4) & 0xF;
            match op {
                0x8 => return Some(Instruction::Rev { rd, rm }),
                0x9 => return Some(Instruction::Rev16 { rd, rm }),
                0xA => return Some(Instruction::Rbit { rd, rm }),
                0xB => return Some(Instruction::RevSh { rd, rm }),
                _ => {}
            }
        } else if (h1 & 0xFFF0) == 0xFAB0 {
            // FABm
            let op = (h2 >> 4) & 0xF;
            if op == 0x8 {
                return Some(Instruction::Clz { rd, rm });
            }
        }
    }

    None
}

#[inline(always)]
pub(super) fn decode_simd(h1: u16, h2: u16) -> Option<Instruction> {
    // SIMD byte add/sub + SEL (Cortex-M4 DSP). Encodings (ARMv7-M):
    //   SADD8 h1=FA8n h2=Fd0m   UADD8 h1=FA8n h2=Fd4m
    //   SSUB8 h1=FACn h2=Fd0m   USUB8 h1=FACn h2=Fd4m
    //   SEL   h1=FAAn h2=Fd8m
    if (h2 & 0xF000) == 0xF000 {
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        let h2op = (h2 >> 4) & 0xF;
        if (h1 & 0xFFF0) == 0xFA80 && h2op == 0x0 {
            return Some(Instruction::SimdAddSub8 { rd, rn, rm, op: 0 }); // SADD8
        }
        if (h1 & 0xFFF0) == 0xFA80 && h2op == 0x4 {
            return Some(Instruction::SimdAddSub8 { rd, rn, rm, op: 1 }); // UADD8
        }
        if (h1 & 0xFFF0) == 0xFAC0 && h2op == 0x0 {
            return Some(Instruction::SimdAddSub8 { rd, rn, rm, op: 2 }); // SSUB8
        }
        if (h1 & 0xFFF0) == 0xFAC0 && h2op == 0x4 {
            return Some(Instruction::SimdAddSub8 { rd, rn, rm, op: 3 }); // USUB8
        }
        if (h1 & 0xFFF0) == 0xFAA0 && h2op == 0x8 {
            return Some(Instruction::Sel { rd, rn, rm });
        }
        // Parallel halfword add/sub: h1 = FA9n (ADD16) / FADn (SUB16), with
        // h2[7:4] selecting the signed/saturating/halving variant. The FA9n
        // REV/RBIT block above claims h2[7:4] = 8..B and returns before this,
        // so the remaining selectors are unambiguous.
        let group16 = h1 & 0xFFF0;
        if (group16 == 0xFA90 || group16 == 0xFAD0)
            && matches!(h2op, 0x0 | 0x1 | 0x2 | 0x4 | 0x5 | 0x6)
        {
            return Some(Instruction::SimdAddSub16 {
                rd,
                rn,
                rm,
                op: h2op as u8,
                sub: group16 == 0xFAD0,
            });
        }
    }

    None
}

#[inline(always)]
pub(super) fn decode_dp_shifted_reg_late(h1: u16, h2: u16) -> Option<Instruction> {
    // Shift by register (Thumb-2): LSL.W / LSR.W / ASR.W / ROR.W Rd, Rn, Rm.
    // ARM ARM encoding (T2):
    //   1111 1010 0 <op> S nnnn  1111 dddd 0000 mmmm
    // The shift type op lives in h1 bits [6:5], NOT h2:
    //   op = 00 -> LSL, 01 -> LSR, 10 -> ASR, 11 -> ROR
    // (Hardware verified against NUCLEO-L476RG: `lsr.w r2, r0, r3` =
    // FA20 F203 was being decoded as LSL because shift_type was read from
    // h2[5:4] = 0 instead of h1[6:5] = 01.)
    if (h1 & 0xFF80) == 0xFA00 && (h2 & 0xF0F0) == 0xF000 {
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        let shift_type = ((h1 >> 5) & 0x3) as u8;
        return Some(Instruction::ShiftReg32 {
            rd,
            rn,
            rm,
            shift_type,
        });
    }

    None
}

#[inline(always)]
pub(super) fn decode_extend_misc_late(h1: u16, h2: u16) -> Option<Instruction> {
    // UXTB.W etc (Miscellaneous)
    if (h1 & 0xFFC0) == 0xFA40 {
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        return Some(Instruction::Uxtb { rd, rm });
    }

    None
}

#[inline(always)]
pub(super) fn decode_ldst_dual_exclusive(h1: u16, h2: u16) -> Option<Instruction> {
    // LDREX / STREX (Encoding T1, ARMv7-M B6.7.79 / B6.7.198).
    // Must be checked before LDRD/STRD T1 because they share the h1
    // prefix but have a specific bit pattern in h1[11:4]:
    //   STREX h1 = 0xE84_, LDREX h1 = 0xE85_  (Rn in low nibble).
    // The CPU's Unknown32 fallback handles the actual load/store.
    if (h1 & 0xFFF0) == 0xE840 || (h1 & 0xFFF0) == 0xE850 {
        return Some(Instruction::Unknown32(h1, h2));
    }
    // LDRD / STRD / TBB / TBH / STMDB / LDMIA.W (Encoding E8xx/E9xx)
    if (h1 & 0xFE00) == 0xE800 {
        let is_load = (h1 & (1 << 4)) != 0;
        let rn = (h1 & 0xF) as u8;
        let rt = ((h2 >> 12) & 0xF) as u8;
        let rt2 = ((h2 >> 8) & 0xF) as u8;
        let imm8 = (h2 & 0xFF) as u32;

        // ARMv7-M TBB/TBH encoding T1:
        //   1110100011010 Rn  1111 0000 000 H Rm
        // h2 bits 15:12 = 0xF, bits 11:5 = 0, bit 4 = H (0=TBB, 1=TBH),
        // bits 3:0 = Rm. The mask must exclude bit 4 (the H selector) OR
        // TBH gets rejected and falls through to Unknown32. This was a real
        // bug: pin_SetF1AFPin (and every stm32duino pin dispatcher) uses
        // TBH at PC=0x08001804 → was returning Unknown32 → PC advance 4
        // landed in the dispatch table → CPU executed table halfwords as
        // instructions → eventual jump to a garbage address (e.g. 0x002B002F
        // from table entry 0x002B002B + thumb prefetch).
        if (h1 & 0x01F0) == 0x00D0 && (h2 & 0xFFE0) == 0xF000 {
            let rm = (h2 & 0xF) as u8;
            let is_tbh = (h2 & 0x0010) != 0;
            if is_tbh {
                return Some(Instruction::Tbh { rn, rm });
            } else {
                return Some(Instruction::Tbb { rn, rm });
            }
        } else if (h1 & 0x40) == 0 {
            // LDM/STM.W: bit6=0 distinguishes LDM/STM from STRD/LDRD (bit6=1).
            // The addressing mode is the U bit (h1 bit7): U=1 -> increment-after
            // (IA), U=0 -> decrement-before (DB). Decoding STM as always-DB and
            // LDM as always-IA is only correct for push/pop; a plain
            // STMIA.W/LDMDB.W (e.g. the compiler's struct-copy idiom) needs the
            // real mode or the access lands at the wrong address.
            // H2 is the full 16-bit register list (bit n = register n, bit14=LR, bit15=PC).
            let writeback = (h1 & 0x20) != 0;
            let increment = (h1 & 0x80) != 0;
            let reg_list = h2;
            return Some(match (is_load, increment) {
                (true, true) => Instruction::LdmiaW {
                    rn,
                    reg_list,
                    writeback,
                },
                (true, false) => Instruction::LdmdbW {
                    rn,
                    reg_list,
                    writeback,
                },
                (false, true) => Instruction::StmiaW {
                    rn,
                    reg_list,
                    writeback,
                },
                (false, false) => Instruction::StmdbW {
                    rn,
                    reg_list,
                    writeback,
                },
            });
        } else if is_load {
            // LDREX (T1, ARMv7-M B6.7.79) shares the h1 = 0xE85x prefix
            // with LDRD; the distinguishing field is h2[11:8] = 0xF.
            // When that field is 0xF and h1[20] = 1 (load), it's LDREX.
            // Otherwise it's LDRD. We let the CPU's Unknown32 fallback
            // handle LDREX itself.
            if (h2 & 0x0F00) == 0x0F00 {
                return Some(Instruction::Unknown32(h1, h2));
            }
            // U bit (h1[7]): 1 = add imm8*4, 0 = subtract. mbedTLS AES reads
            // round keys via `ldrd Rt,Rt2,[Rn,#-imm]` (U=0); ignoring U read
            // the wrong key and corrupted the cipher output.
            let add_imm = (h1 & 0x80) != 0;
            // P (h1[8]) selects pre/post indexing; W (h1[5]) selects writeback.
            // Pre-indexed-with-writeback (`ldrd Rt,Rt2,[Rn,#imm]!`) and
            // post-indexed (`ldrd Rt,Rt2,[Rn],#imm`) both update Rn; the
            // libgcc 64-bit divide helper used by mbedTLS bignum relies on
            // the writeback form to restore SP, so ignoring W corrupted the
            // stack frame and crashed the RSA verify path.
            let index = (h1 & 0x100) != 0;
            let writeback = (h1 & 0x20) != 0;
            return Some(Instruction::Ldrd {
                rt,
                rt2,
                rn,
                imm8,
                add_imm,
                index,
                writeback,
            });
        } else {
            // STREX (T1, B6.7.198) — distinguished from STRD the same
            // way. h2[15:12]=Rt (value), h2[11:8]=Rd (success flag);
            // for STRD h2[11:8]=Rt2 of the doubleword pair, which is
            // never 0xF in well-formed code.
            // For STREX the Rd field can be any of r0-r12; the safer
            // heuristic is the h1 specific bit pattern: STREX is exactly
            // h1 = 0xE84x (no other STRD variant matches). LDREX/STREX
            // are distinguished from LDRD/STRD T1 by h1 having a fixed
            // value rather than the writeback/pre-index/up variations
            // available to LDRD/STRD.
            //
            // The simplest reliable check: STREX has h1 with bits
            // 27..22 = 1000_01 and bit 21 = 0, bit 20 = 0. LDRD/STRD
            // T1 have bit 22 = 1 with P/U/W/L varying — that overlap
            // means we have to look at h2 too. h2[11:8] = 0xF reliably
            // marks REX variants.
            if (h2 & 0x0F00) == 0x0F00 {
                // Unlikely for STRD; treat as STREX.
                return Some(Instruction::Unknown32(h1, h2));
            }
            let add_imm = (h1 & 0x80) != 0;
            let index = (h1 & 0x100) != 0;
            let writeback = (h1 & 0x20) != 0;
            return Some(Instruction::Strd {
                rt,
                rt2,
                rn,
                imm8,
                add_imm,
                index,
                writeback,
            });
        }
    }

    None
}

#[inline(always)]
pub(super) fn decode_branch(h1: u16, h2: u16) -> Option<Instruction> {
    // B<c>.W (Thumb-2 conditional branch, T3)
    // 1111 0 S cond imm6 10 J1 0 J2 imm11
    if (h1 & 0xF800) == 0xF000 && (h2 & 0xD000) == 0x8000 {
        let s = ((h1 >> 10) & 0x1) as i32;
        let cond = ((h1 >> 6) & 0xF) as u8;
        if cond != 0xE && cond != 0xF {
            let imm6 = (h1 & 0x3F) as i32;
            let j1 = ((h2 >> 13) & 0x1) as i32;
            let j2 = ((h2 >> 11) & 0x1) as i32;
            let imm11 = (h2 & 0x7FF) as i32;

            let mut offset = (s << 20) | (j2 << 19) | (j1 << 18) | (imm6 << 12) | (imm11 << 1);
            if (offset & (1 << 20)) != 0 {
                offset |= !0x001F_FFFF;
            }

            return Some(Instruction::BranchCond { cond, offset });
        }
    }
    // B.W / BL: H1[15:11]=11110, H2[15]=1, H2[12]=1.
    // H2[12]=1 is required by both BL T1 and B.W T4 and distinguishes them from
    // other 32-bit instructions with H2[15]=1 (e.g. NOP.W = F3AF 8000, where H2[12]=0).
    if (h1 & 0xF800) == 0xF000 && (h2 & 0x9000) == 0x9000 {
        let s = ((h1 >> 10) & 0x1) as i32;
        let j1 = ((h2 >> 13) & 0x1) as i32;
        let j2 = ((h2 >> 11) & 0x1) as i32;
        let i1 = (!(j1 ^ s)) & 0x1;
        let i2 = (!(j2 ^ s)) & 0x1;
        let imm11 = (h2 & 0x7FF) as i32;

        // BL T1: H2[bit14]=1. B.W T4: H2[bit14]=0. Both have H2[bit12]=1.
        let is_bl = (h2 & 0x4000) != 0;
        let imm_h1 = (h1 & 0x3FF) as i32; // Both BL and B.W use 10-bit immediate from H1

        let mut offset = (s << 24) | (i1 << 23) | (i2 << 22) | (imm_h1 << 12) | (imm11 << 1);

        if (offset & (1 << 24)) != 0 {
            offset |= !0x01FF_FFFF;
        }

        if is_bl {
            return Some(Instruction::Bl { offset });
        } else {
            return Some(Instruction::Branch { offset });
        }
    }

    None
}

#[inline(always)]
pub(super) fn decode_mul_acc_divide_late(h1: u16, h2: u16) -> Option<Instruction> {
    // MUL.W (T2): 1111 1011 0000 nnnn 1111 dddd 0000 mmmm -> FB0. F.0.
    // Distinguishes from MLA (Ra != 0xF in h2[15:12]).
    if (h1 & 0xFFF0) == 0xFB00 && (h2 & 0xF0F0) == 0xF000 {
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        return Some(Instruction::Mul32 { rd, rn, rm });
    }
    // UDIV / SDIV: 1111 1011 10x1 ... -> FB9.. / FBB..
    if (h1 & 0xFFD0) == 0xFB90 && (h2 & 0xF0F0) == 0xF0F0 {
        let is_unsigned = (h1 & 0x0020) != 0;
        let rn = (h1 & 0xF) as u8;
        let rd = ((h2 >> 8) & 0xF) as u8;
        let rm = (h2 & 0xF) as u8;
        if is_unsigned {
            return Some(Instruction::Udiv { rd, rn, rm });
        } else {
            return Some(Instruction::Sdiv { rd, rn, rm });
        }
    }

    None
}
