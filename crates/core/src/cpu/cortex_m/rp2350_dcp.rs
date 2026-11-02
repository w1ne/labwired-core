// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2350 double-precision coprocessor (DCP), RP2350 datasheet 3.6.2.
//!
//! Each Cortex-M33 has two DCP instances: coprocessor 4 is the one for the
//! current security state, coprocessor 5 always the Non-secure one. The twin
//! runs everything Secure (no TrustZone), so p4 is instance 0 and p5 is
//! instance 1. The instruction encodings are pico-sdk's
//! `hardware_dcp/dcp_instr.inc.S`; the canned sequences that use them are
//! `hardware_dcp/dcp_canned.inc.S` (datasheet Tables 109-112).
//!
//! # Model
//!
//! Register file (3.6.2.3): `xm`/`ym` (64-bit mantissas), `xe`/`ye`
//! (exponents), `xf`/`yf` (flags: sign, zero, infinity, NaN), and the 9-bit
//! status register whose top bit is the engaged flag. A "write unpacked"
//! (WXUP/WYUP/WXYU and the conversion writes) loads the operand exactly:
//! mantissa `01` + fraction, exponent field, flags. Denormal inputs are
//! flushed to zero (3.6.2.9).
//!
//! Results are computed at the final read of the canned sequence from the
//! operands in X and Y, IEEE-754 round-to-nearest-even, with denormal
//! outputs flushed to zero (3.6.2.9): RDDA = X+Y, RDDS = X-Y, RDDM = X*Y,
//! RDDD = X/Y, RDDQ = sqrt(X), RDDG = X, and the single-precision RDFx and
//! RDFG, the integer RDIC/RDUC (truncating after NTDC, rounding after NRDC,
//! saturating) and RCMP (compare flags). The conversion writes encode their
//! operand the way the canned sequences expect: WXIC/WXUC put
//! `int + 2^52 + 2^32` in X and `2^52 + 2^32` in Y, so the int-to-double
//! sequence's final RDDS (X-Y) yields the integer exactly.
//!
//! The mid-sequence mantissa reads (RXMS, RYMS, RXYH, RYMR, RXMQ) return the
//! operand mantissas, and the CPU-computed partial results written back
//! (WXMS, WXMO, WXDD, WXDQ, WXFM, WXFD, WXFQ) are accepted without changing
//! X or Y. The canned dmul/ddiv/dsqrt sequences contain no branches, so the
//! CPU side runs exactly as on silicon and the result is the correctly
//! rounded IEEE value.
//!
//! FIDELITY: sequence-level model, NOT HW-validated (2026-11-01).
//! - `ddiv_fast` / `sqrt_fast` return the correctly rounded result; silicon
//!   guarantees only < 0.5 ulp before rounding (3.6.2.9), so it can differ in
//!   the last bit.
//! - A NaN result is the default NaN (0x7FF8_0000_0000_0000 / 0x7FC0_0000);
//!   silicon NaN payload propagation is not documented.
//! - RXVD (dclassify / version read) is not modelled and faults.
//! - The REFD/WEFD word layout is the model's own (save/restore round-trips
//!   through it); silicon's bit layout is not documented.
//! - CPACR.CP4/CP5 enables are not enforced (as for the FPU).

/// Engaged flag: status bit 8 (3.6.2.3 "the final bit of the status register").
const ENGAGED: u16 = 1 << 8;
const XF_SIGN: u8 = 1;
const XF_ZERO: u8 = 2;
const XF_INF: u8 = 4;
const XF_NAN: u8 = 8;
/// `2^52 + 2^32`, the conversion bias WXIC/WXUC/WXDC/WXFC load into Y.
const CONV_BIAS: f64 = 4_503_603_922_337_792.0;
const DEFAULT_NAN64: u64 = 0x7FF8_0000_0000_0000;
const DEFAULT_NAN32: u32 = 0x7FC0_0000;

/// Integer-conversion normalise mode: set by NTDC / NRDC, read by RDIC/RDUC.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum IntMode {
    #[default]
    Truncate,
    Nearest,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Operand {
    m: u64,
    e: u16,
    f: u8,
}

impl Operand {
    fn from_f64(v: f64) -> Self {
        let bits = v.to_bits();
        let sign = if bits >> 63 != 0 { XF_SIGN } else { 0 };
        let exp = ((bits >> 52) & 0x7FF) as u16;
        let frac = bits & ((1u64 << 52) - 1);
        match exp {
            0 => Self {
                m: 0,
                e: 0,
                f: sign | XF_ZERO,
            }, // zero or flushed denormal
            0x7FF if frac == 0 => Self {
                m: 0,
                e: exp,
                f: sign | XF_INF,
            },
            0x7FF => Self {
                m: frac << 10,
                e: exp,
                f: sign | XF_NAN,
            },
            _ => Self {
                m: (1u64 << 62) | (frac << 10),
                e: exp,
                f: sign,
            },
        }
    }

    fn from_f32_bits(bits: u32) -> Self {
        let v = f32::from_bits(bits);
        // Denormal single inputs flush to (signed) zero before widening.
        let v = if v.is_subnormal() {
            0.0f32.copysign(v)
        } else {
            v
        };
        Self::from_f64(f64::from(v))
    }

    fn to_f64(self) -> f64 {
        let sign = if self.f & XF_SIGN != 0 { 1u64 << 63 } else { 0 };
        let bits = if self.f & XF_NAN != 0 {
            sign | (0x7FFu64 << 52) | ((self.m >> 10) & ((1u64 << 52) - 1)) | (1u64 << 51)
        } else if self.f & XF_INF != 0 {
            sign | (0x7FFu64 << 52)
        } else if self.f & XF_ZERO != 0 {
            sign
        } else {
            sign | (u64::from(self.e & 0x7FF) << 52) | ((self.m >> 10) & ((1u64 << 52) - 1))
        };
        f64::from_bits(bits)
    }
}

fn ftz64(r: f64) -> u64 {
    if r.is_nan() {
        DEFAULT_NAN64
    } else if r.is_subnormal() {
        0.0f64.copysign(r).to_bits()
    } else {
        r.to_bits()
    }
}

fn ftz32(r: f64) -> u32 {
    let s = r as f32;
    if s.is_nan() {
        DEFAULT_NAN32
    } else if s.is_subnormal() {
        0.0f32.copysign(s).to_bits()
    } else {
        s.to_bits()
    }
}

/// One DCP instance's architectural state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DcpState {
    x: Operand,
    y: Operand,
    status: u16,
    mode: IntMode,
}

/// The two instances (p4 = current-state/Secure, p5 = Non-secure).
#[derive(Debug, Clone, Default)]
pub struct Rp2350Dcp {
    pub inst: [DcpState; 2],
}

/// A decoded coprocessor access to p4/p5. `p` = the `*2` encoding (mrc2,
/// mrrc2: the "peek" variants that preserve the engaged flag).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DcpOp {
    Cdp { opc1: u8, crm: u8, opc2: u8 },
    Mcrr { opc1: u8, crm: u8, lo: u32, hi: u32 },
    Mrc { crm: u8, opc2: u8, p: bool },
    Mrrc { opc1: u8, crm: u8, p: bool },
}

/// Result of executing a [`DcpOp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DcpOut {
    None,
    One(u32),
    Two(u32, u32),
}

impl DcpState {
    fn engaged(&self) -> bool {
        self.status & ENGAGED != 0
    }

    fn set_engaged(&mut self, on: bool) {
        if on {
            self.status |= ENGAGED;
        } else {
            self.status &= !ENGAGED;
        }
    }

    fn xv(&self) -> f64 {
        self.x.to_f64()
    }

    fn yv(&self) -> f64 {
        self.y.to_f64()
    }

    /// RCMP / PCMP value: N = engaged (the SDK's `PCMP apsr_nzcv; bmi` save
    /// test), Z = equal, C = X >= Y, V = unordered; low 9 bits = status.
    fn cmp_word(&self) -> u32 {
        let (x, y) = (self.xv(), self.yv());
        let n = u32::from(self.engaged());
        let (z, c, v) = if x.is_nan() || y.is_nan() {
            (0, 0, 1)
        } else {
            (u32::from(x == y), u32::from(x >= y), 0)
        };
        (n << 31) | (z << 30) | (c << 29) | (v << 28) | u32::from(self.status & 0x1FF)
    }

    fn to_int(&self, signed: bool) -> u32 {
        let x = self.xv();
        if x.is_nan() {
            return 0;
        }
        let r = match self.mode {
            IntMode::Truncate => x.trunc(),
            IntMode::Nearest => x.round_ties_even(),
        };
        if signed {
            r.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32 as u32
        } else {
            r.clamp(0.0, f64::from(u32::MAX)) as u32
        }
    }

    /// Execute one DCP instruction. `None` = not modelled (the core faults).
    pub fn exec(&mut self, op: DcpOp) -> Option<DcpOut> {
        let out = match op {
            DcpOp::Cdp { opc1, crm, opc2 } => {
                match (opc1, crm, opc2) {
                    (0, 0, 0) => *self = Self::default(), // INIT
                    (0, 1, 0)                              // ADD0
                    | (1, 1, 0)                            // ADD1
                    | (1, 1, 1)                            // SUB1
                    | (2, 1, 0)                            // SQR0
                    | (8, 2, 0)                            // NORM
                    | (8, 2, 1)                            // NRDF
                    | (8, 0, 1) => {}                      // NRDD
                    (8, 0, 2) => self.mode = IntMode::Truncate, // NTDC
                    (8, 0, 3) => self.mode = IntMode::Nearest,  // NRDC
                    _ => return None,
                }
                if (opc1, crm, opc2) != (0, 0, 0) {
                    self.set_engaged(true);
                }
                DcpOut::None
            }
            DcpOp::Mcrr { opc1, crm, lo, hi } => {
                let d = f64::from_bits(u64::from(hi) << 32 | u64::from(lo));
                let bias = Operand::from_f64(CONV_BIAS);
                match (opc1, crm) {
                    (0, 0) => self.x.m = u64::from(hi) << 32 | u64::from(lo), // WXMD
                    (0, 1) => self.y.m = u64::from(hi) << 32 | u64::from(lo), // WYMD
                    (0, 2) => {
                        // WEFD: model layout, see REFD.
                        self.x.e = (lo & 0x3FFF) as u16;
                        self.x.f = ((lo >> 16) & 0xF) as u8;
                        self.mode = if (lo >> 20) & 1 != 0 {
                            IntMode::Nearest
                        } else {
                            IntMode::Truncate
                        };
                        self.y.e = (hi & 0x3FFF) as u16;
                        self.y.f = ((hi >> 16) & 0xF) as u8;
                        self.status = ((hi >> 20) & 0x1FF) as u16;
                        return Some(DcpOut::None);
                    }
                    (1, 0) => self.x = Operand::from_f64(d), // WXUP
                    (1, 1) => self.y = Operand::from_f64(d), // WYUP
                    (1, 2) => {
                        // WXYU: two single-precision operands.
                        self.x = Operand::from_f32_bits(lo);
                        self.y = Operand::from_f32_bits(hi);
                    }
                    (6, 0) => {
                        // WXUC: unsigned int + 2^52 + 2^32 (exact).
                        self.x = Operand::from_f64(f64::from(lo) + CONV_BIAS);
                        self.y = bias;
                    }
                    (7, 0) => {
                        // WXIC: signed int + 2^52 + 2^32 (exact).
                        self.x = Operand::from_f64(f64::from(lo as i32) + CONV_BIAS);
                        self.y = bias;
                    }
                    (8, 0) => {
                        // WXDC: double operand for a to-int conversion.
                        self.x = Operand::from_f64(d);
                        self.y = bias;
                    }
                    (9, 2) => {
                        // WXFC: single operand for a to-int conversion.
                        self.x = Operand::from_f32_bits(lo);
                        self.y = bias;
                    }
                    // WXMS, WXMO, WXDD, WXDQ, WXFM, WXFD, WXFQ: CPU partial
                    // results; the result is taken from the operands.
                    (2, 0) | (3, 0) | (4, 0) | (5, 0) | (10, 0) | (11, 0) | (12, 0) => {}
                    _ => return None,
                }
                self.set_engaged(true);
                DcpOut::None
            }
            DcpOp::Mrc { crm, opc2, p } => {
                let v = match (crm, opc2) {
                    (0, 1) => self.cmp_word(),              // RCMP / PCMP
                    (2, 0) => ftz32(self.xv() + self.yv()), // RDFA
                    (2, 1) => ftz32(self.xv() - self.yv()), // RDFS
                    (2, 2) => ftz32(self.xv() * self.yv()), // RDFM
                    (2, 3) => ftz32(self.xv() / self.yv()), // RDFD
                    (2, 4) => ftz32(self.xv().sqrt()),      // RDFQ
                    (2, 5) => ftz32(self.xv()),             // RDFG
                    (3, 0) => self.to_int(true),            // RDIC
                    (3, 1) => self.to_int(false),           // RDUC
                    _ => return None,                       // RXVD and reserved
                };
                if !p {
                    self.set_engaged(false);
                }
                DcpOut::One(v)
            }
            DcpOp::Mrrc { opc1, crm, p } => {
                let split = |w: u64| (w as u32, (w >> 32) as u32);
                let (lo, hi, engaged_after) = match (opc1, crm) {
                    (0, 8) => {
                        let (l, h) = split(self.x.m); // RXMD == PXMD
                        (l, h, Some(self.engaged()))
                    }
                    (0, 9) => {
                        let (l, h) = split(self.y.m); // RYMD clears engaged
                        (l, h, Some(p && self.engaged()))
                    }
                    (0, 10) => {
                        // REFD == PEFD (model layout).
                        let lo = u32::from(self.x.e)
                            | u32::from(self.x.f) << 16
                            | u32::from(self.mode == IntMode::Nearest) << 20;
                        let hi = u32::from(self.y.e)
                            | u32::from(self.y.f) << 16
                            | u32::from(self.status & 0x1FF) << 20;
                        (lo, hi, Some(self.engaged()))
                    }
                    (s, 4) => {
                        let (l, h) = split(self.x.m >> s); // RXMS
                        (l, h, None)
                    }
                    (s, 5) => {
                        let (l, h) = split(self.y.m >> s); // RYMS
                        (l, h, None)
                    }
                    (1, 1) => ((self.y.m >> 32) as u32, (self.x.m >> 32) as u32, None), // RXYH
                    (2, 1) => {
                        // RYMR: ym hi + a reciprocal estimate (feeds ignored writes).
                        let yh = (self.y.m >> 32).max(1);
                        ((self.y.m >> 32) as u32, ((1u64 << 62) / yh) as u32, None)
                    }
                    (4, 1) => ((self.x.m >> 32) as u32, 0x8000_0000, None), // RXMQ
                    (1, 0) => {
                        let (l, h) = split(ftz64(self.xv() + self.yv())); // RDDA
                        (l, h, Some(false))
                    }
                    (3, 0) => {
                        let (l, h) = split(ftz64(self.xv() - self.yv())); // RDDS
                        (l, h, Some(false))
                    }
                    (5, 0) => {
                        let (l, h) = split(ftz64(self.xv() * self.yv())); // RDDM
                        (l, h, Some(false))
                    }
                    (7, 0) => {
                        let (l, h) = split(ftz64(self.xv() / self.yv())); // RDDD
                        (l, h, Some(false))
                    }
                    (9, 0) => {
                        let (l, h) = split(ftz64(self.xv().sqrt())); // RDDQ
                        (l, h, Some(false))
                    }
                    (11, 0) => {
                        let (l, h) = split(ftz64(self.xv())); // RDDG
                        (l, h, Some(false))
                    }
                    _ => return None,
                };
                if !p {
                    // Mid-sequence reads (None) keep the operation engaged.
                    self.set_engaged(engaged_after.unwrap_or(true));
                }
                DcpOut::Two(lo, hi)
            }
        };
        Some(out)
    }
}

/// Decode a 32-bit Thumb coprocessor instruction aimed at p4/p5. Returns
/// `(instance, op, rt, rt2)`; `rt`/`rt2` are the core registers to read
/// (MCRR) or write (MRC/MRRC). `None` for every other coprocessor.
pub fn decode(h1: u16, h2: u16, read_reg: impl Fn(u8) -> u32) -> Option<(usize, DcpOp, u8, u8)> {
    let coproc = (h2 >> 8) & 0xF;
    let inst = match coproc {
        4 => 0,
        5 => 1,
        _ => return None,
    };
    let t2 = match h1 >> 12 {
        0xE => false,
        0xF => true,
        _ => return None,
    };
    let rt = ((h2 >> 12) & 0xF) as u8;
    let crm = (h2 & 0xF) as u8;
    match (h1 >> 4) & 0xFF {
        0xC4 | 0xC5 => {
            // MCRR / MRRC (A7.7.75 / A7.7.80): Rt2 in h1[3:0].
            let rt2 = (h1 & 0xF) as u8;
            let opc1 = ((h2 >> 4) & 0xF) as u8;
            if (h1 >> 4) & 0x1 == 0 {
                if t2 {
                    return None; // mcrr2 is not a DCP instruction
                }
                let (lo, hi) = (read_reg(rt), read_reg(rt2));
                Some((inst, DcpOp::Mcrr { opc1, crm, lo, hi }, rt, rt2))
            } else {
                Some((inst, DcpOp::Mrrc { opc1, crm, p: t2 }, rt, rt2))
            }
        }
        b if b >> 4 == 0xE => {
            let crn = h1 & 0xF;
            if h2 & 0x10 != 0 {
                // MCR / MRC: opc1 = h1[7:5], L = h1[4], opc2 = h2[7:5].
                let opc1 = (h1 >> 5) & 0x7;
                let l = (h1 >> 4) & 1;
                let opc2 = ((h2 >> 5) & 0x7) as u8;
                if l == 1 && opc1 == 0 && crn == 0 {
                    Some((inst, DcpOp::Mrc { crm, opc2, p: t2 }, rt, 0))
                } else {
                    None // MCR and other MRC forms: not DCP instructions
                }
            } else {
                // CDP: opc1 = h1[7:4], CRn = h1[3:0], CRd = h2[15:12].
                if t2 || crn != 0 || rt != 0 {
                    return None;
                }
                let opc1 = ((h1 >> 4) & 0xF) as u8;
                let opc2 = ((h2 >> 5) & 0x7) as u8;
                Some((inst, DcpOp::Cdp { opc1, crm, opc2 }, 0, 0))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &mut DcpState, ops: &[DcpOp]) -> DcpOut {
        let mut last = DcpOut::None;
        for &op in ops {
            last = s.exec(op).expect("modelled op");
        }
        last
    }

    fn wd(opc1: u8, crm: u8, v: f64) -> DcpOp {
        let b = v.to_bits();
        DcpOp::Mcrr {
            opc1,
            crm,
            lo: b as u32,
            hi: (b >> 32) as u32,
        }
    }

    const ADD0: DcpOp = DcpOp::Cdp {
        opc1: 0,
        crm: 1,
        opc2: 0,
    };
    const ADD1: DcpOp = DcpOp::Cdp {
        opc1: 1,
        crm: 1,
        opc2: 0,
    };
    const NRDD: DcpOp = DcpOp::Cdp {
        opc1: 8,
        crm: 0,
        opc2: 1,
    };

    fn two(o: DcpOut) -> u64 {
        match o {
            DcpOut::Two(l, h) => u64::from(h) << 32 | u64::from(l),
            o => panic!("expected two words, got {o:?}"),
        }
    }

    #[test]
    fn dadd_sequence_is_ieee_and_clears_engaged() {
        let mut s = DcpState::default();
        let r = run(
            &mut s,
            &[
                wd(1, 0, 0.1),
                wd(1, 1, 0.2),
                ADD0,
                ADD1,
                NRDD,
                DcpOp::Mrrc {
                    opc1: 1,
                    crm: 0,
                    p: false,
                },
            ],
        );
        assert_eq!(f64::from_bits(two(r)), 0.1 + 0.2);
        assert!(!s.engaged());
    }

    #[test]
    fn engaged_is_set_mid_sequence_and_save_restore_round_trips() {
        let mut s = DcpState::default();
        run(&mut s, &[wd(1, 0, 3.0), wd(1, 1, 7.0), ADD0]);
        assert!(s.engaged());
        let pcmp = run(
            &mut s,
            &[DcpOp::Mrc {
                crm: 0,
                opc2: 1,
                p: true,
            }],
        );
        assert!(
            matches!(pcmp, DcpOut::One(v) if v >> 31 == 1),
            "PCMP N = engaged"
        );
        assert!(s.engaged(), "PCMP preserves engaged");
        // generic_save_state: PXMD, PYMD, REFD; an ISR clobbers; restore.
        let xm = two(run(
            &mut s,
            &[DcpOp::Mrrc {
                opc1: 0,
                crm: 8,
                p: true,
            }],
        ));
        let ym = two(run(
            &mut s,
            &[DcpOp::Mrrc {
                opc1: 0,
                crm: 9,
                p: true,
            }],
        ));
        let ef = two(run(
            &mut s,
            &[DcpOp::Mrrc {
                opc1: 0,
                crm: 10,
                p: false,
            }],
        ));
        let saved = s.clone();
        run(
            &mut s,
            &[
                wd(1, 0, 100.0),
                wd(1, 1, 1.0),
                ADD0,
                ADD1,
                NRDD,
                DcpOp::Mrrc {
                    opc1: 1,
                    crm: 0,
                    p: false,
                },
            ],
        );
        let w = |v: u64| (v as u32, (v >> 32) as u32);
        let (l, h) = w(xm);
        run(
            &mut s,
            &[DcpOp::Mcrr {
                opc1: 0,
                crm: 0,
                lo: l,
                hi: h,
            }],
        );
        let (l, h) = w(ym);
        run(
            &mut s,
            &[DcpOp::Mcrr {
                opc1: 0,
                crm: 1,
                lo: l,
                hi: h,
            }],
        );
        let (l, h) = w(ef);
        run(
            &mut s,
            &[DcpOp::Mcrr {
                opc1: 0,
                crm: 2,
                lo: l,
                hi: h,
            }],
        );
        assert_eq!(s, saved);
        let r = run(
            &mut s,
            &[
                ADD1,
                NRDD,
                DcpOp::Mrrc {
                    opc1: 3,
                    crm: 0,
                    p: false,
                },
            ],
        );
        assert_eq!(
            f64::from_bits(two(r)),
            -4.0,
            "resumed sequence finishes 3 - 7"
        );
    }

    #[test]
    fn int_conversions_use_the_bias_trick_and_saturate() {
        let mut s = DcpState::default();
        // dcp_int2double_m: WXIC; ADD0; SUB1; NRDD; RDDS.
        let r = run(
            &mut s,
            &[
                DcpOp::Mcrr {
                    opc1: 7,
                    crm: 0,
                    lo: (-123456789i32) as u32,
                    hi: 0,
                },
                ADD0,
                DcpOp::Cdp {
                    opc1: 1,
                    crm: 1,
                    opc2: 1,
                },
                NRDD,
                DcpOp::Mrrc {
                    opc1: 3,
                    crm: 0,
                    p: false,
                },
            ],
        );
        assert_eq!(f64::from_bits(two(r)), -123456789.0);
        // dcp_double2int_m (NTDC) truncates, double2int_r (NRDC) rounds to even.
        let ntdc = DcpOp::Cdp {
            opc1: 8,
            crm: 0,
            opc2: 2,
        };
        let nrdc = DcpOp::Cdp {
            opc1: 8,
            crm: 0,
            opc2: 3,
        };
        let rdic = DcpOp::Mrc {
            crm: 3,
            opc2: 0,
            p: false,
        };
        let rduc = DcpOp::Mrc {
            crm: 3,
            opc2: 1,
            p: false,
        };
        for (v, mode, rd, want) in [
            (-2.7, ntdc, rdic, -2i64),
            (2.5, nrdc, rdic, 2),
            (3.5, nrdc, rdic, 4),
            (1e12, ntdc, rdic, i32::MAX as i64),
            (-1e12, ntdc, rdic, i32::MIN as i64),
            (-5.0, ntdc, rduc, 0),
            (5e9, ntdc, rduc, u32::MAX as i64),
            (f64::NAN, ntdc, rdic, 0),
        ] {
            let o = run(&mut s, &[wd(8, 0, v), ADD0, ADD1, mode, rd]);
            let DcpOut::One(got) = o else { panic!() };
            let got = if rd == rdic {
                got as i32 as i64
            } else {
                got as i64
            };
            assert_eq!(got, want, "{v}");
        }
    }

    #[test]
    fn denormals_flush_and_nan_is_default() {
        let mut s = DcpState::default();
        let tiny = f64::from_bits(1); // smallest denormal
        let r = run(
            &mut s,
            &[
                wd(1, 0, tiny),
                wd(1, 1, 1.0),
                DcpOp::Mrrc {
                    opc1: 5,
                    crm: 0,
                    p: false,
                },
            ],
        );
        assert_eq!(two(r), 0, "denormal input flushes to +0");
        let r = run(
            &mut s,
            &[
                wd(1, 0, f64::MIN_POSITIVE),
                wd(1, 1, 0.5),
                DcpOp::Mrrc {
                    opc1: 5,
                    crm: 0,
                    p: false,
                },
            ],
        );
        assert_eq!(two(r), 0, "denormal output flushes to +0");
        let r = run(
            &mut s,
            &[
                wd(1, 0, -1.0),
                DcpOp::Mrrc {
                    opc1: 9,
                    crm: 0,
                    p: false,
                },
            ],
        );
        assert_eq!(two(r), DEFAULT_NAN64);
    }

    #[test]
    fn rcmp_flags_match_the_sdk_conditions() {
        let mut s = DcpState::default();
        let rcmp = DcpOp::Mrc {
            crm: 0,
            opc2: 1,
            p: false,
        };
        let flags = |s: &mut DcpState, a: f64, b: f64| match run(
            s,
            &[wd(1, 0, a), wd(1, 1, b), ADD0, rcmp],
        ) {
            DcpOut::One(v) => v >> 28,
            _ => unreachable!(),
        };
        // N Z C V
        assert_eq!(flags(&mut s, 1.0, 1.0) & 0b0111, 0b0110, "eq: Z C");
        assert_eq!(flags(&mut s, 2.0, 1.0) & 0b0111, 0b0010, "gt: C");
        assert_eq!(flags(&mut s, 1.0, 2.0) & 0b0111, 0b0000, "lt");
        assert_eq!(
            flags(&mut s, f64::NAN, 2.0) & 0b0111,
            0b0001,
            "unordered: V, C clear"
        );
        assert_eq!(flags(&mut s, 0.0, -0.0) & 0b0111, 0b0110, "+0 == -0");
        assert!(!s.engaged(), "RCMP clears engaged");
    }

    #[test]
    fn decode_matches_the_sdk_encodings() {
        let r = |_r: u8| 0u32;
        // mrc p4,#0,r0,c0,c0,#1  (RCMP r0, runtime_init_per_core_enable_coprocessors)
        assert_eq!(
            decode(0xEE10, 0x0430, r),
            Some((
                0,
                DcpOp::Mrc {
                    crm: 0,
                    opc2: 1,
                    p: false
                },
                0,
                0
            ))
        );
        // mrc2 p4,#0,APSR_nzcv,c0,c0,#1 (PCMP apsr_nzcv)
        assert_eq!(
            decode(0xFE10, 0xF430, r),
            Some((
                0,
                DcpOp::Mrc {
                    crm: 0,
                    opc2: 1,
                    p: true
                },
                15,
                0
            ))
        );
        // mcrr p4,#1,r0,r1,c0 (WXUP r0,r1)
        assert!(matches!(
            decode(0xEC41, 0x0410, r),
            Some((
                0,
                DcpOp::Mcrr {
                    opc1: 1,
                    crm: 0,
                    ..
                },
                0,
                1
            ))
        ));
        // mrrc p4,#1,r0,r1,c0 (RDDA r0,r1)
        assert_eq!(
            decode(0xEC51, 0x0410, r),
            Some((
                0,
                DcpOp::Mrrc {
                    opc1: 1,
                    crm: 0,
                    p: false
                },
                0,
                1
            ))
        );
        // cdp p4,#8,c0,c0,c0,#1 (NRDD)
        assert_eq!(
            decode(0xEE80, 0x0420, r),
            Some((
                0,
                DcpOp::Cdp {
                    opc1: 8,
                    crm: 0,
                    opc2: 1
                },
                0,
                0
            ))
        );
        // p5 is the second instance; p7 (RCP) and p10 (VFP) are not ours.
        assert_eq!(decode(0xEE10, 0x0530, r).map(|d| d.0), Some(1));
        assert_eq!(decode(0xEE10, 0x0730, r), None);
        assert_eq!(decode(0xEE10, 0x0A10, r), None);
    }
}
