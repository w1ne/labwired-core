// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa FP-option divide / square-root assist ops (single precision):
//! `CONST.S`, `DIV0.S`, `RECIP0.S`, `SQRT0.S`, `RSQRT0.S`, `NEXP01.S`,
//! `MKDADJ.S`, `MKSADJ.S`, `ADDEXP.S`, `ADDEXPM.S`, `MADDN.S`, `DIVN.S`.
//!
//! The LX7 FPU has no `div.s` / `sqrt.s`; libgcc (`__divsf3`,
//! `__ieee754_sqrtf`, `__recipsf2`, `__rsqrtsf2`) and the ESP32-S3 ROM
//! `__divsf3` build them from these steps:
//!
//! * `NEXP01.S` narrows the operand to `-m`, `m` in `[1, 4)`: the exponent
//!   is set to 0 or 1, keeping its parity (so the square root of the
//!   removed power of two is exact).
//! * `DIV0.S` / `SQRT0.S` seed `1/m` / `1/sqrt(m)` for that same `m`;
//!   `RECIP0.S` / `RSQRT0.S` seed `1/x` / `1/sqrt(x)` for the full operand.
//! * `MADDN.S` is a fused multiply-add rounded once to nearest, so the
//!   Newton-Raphson residuals are exact.
//! * `MKDADJ.S` / `MKSADJ.S` build the adjustment from the original operands:
//!   the result sign and the power of two removed by `NEXP01.S` (quotient:
//!   `2^(Ea-Eb)`, root: `2^(E/2)`), or the IEEE special result (NaN, signed
//!   infinity, signed zero) when an operand is zero, infinite or NaN.
//! * `ADDEXP.S` / `ADDEXPM.S` apply that adjustment to the quotient (or
//!   root) estimate and to the reciprocal term; `DIVN.S` is the final
//!   `fr + fs*ft` step, rounded once.
//!
//! Model notes, where the public ISA text stops short of a bit layout:
//! * Seeds come from a lookup indexed by the leading [`SEED_BITS`]` - 1`
//!   fraction bits of the operand: the function of that interval's
//!   midpoint, rounded to [`SEED_BITS`] significant bits (relative error
//!   < 2^-7). Newton-Raphson only needs a seed of that quality.
//! * `MADDN.S` rounds ties away from zero. With ties-to-even the reciprocal
//!   iteration stalls half an ulp low on all-ones divisor mantissas (e.g.
//!   `1 / 1.9999999`) and `__divsf3` misrounds there; with ties-away the ROM
//!   sequence is correctly rounded for every divisor mantissa (checked
//!   exhaustively for several dividends) and for randomised operands of
//!   every class, against host `f32` `/` and `sqrt`.
//! * The adjustment is a power of two, `±0`, `±inf` or NaN; `ADDEXP.S` and
//!   `ADDEXPM.S` both multiply by the power of two found in the exponent of
//!   `fs` (and xor the sign), or return the special value. Hence
//!   `addexp.s f4, f0` with `f0 = 0.5` halves `f4`, as `__ieee754_sqrtf` uses.
//! * A quotient near the edge of the f32 range needs an adjustment, or an
//!   adjusted estimate, outside f32. The register holds the IEEE rounding of
//!   that value and the CPU keeps its exact value beside it (a "wide" latch,
//!   valid while the register still holds those bits), so `DIVN.S` rounds the
//!   overflowing / subnormal result exactly once.
//! * `DIVN.S` passes an `fr` of `±0`, `±inf` or NaN straight through: that is
//!   the special result `ADDEXPM.S` installed.
//! * The FCR rounding mode is not modelled (round to nearest even), like the
//!   rest of this FPU model; FSR flags are not raised.

/// Significant bits of the `DIV0.S` / `SQRT0.S` / `RECIP0.S` / `RSQRT0.S` seed.
pub(crate) const SEED_BITS: u32 = 8;

/// Default quiet NaN produced by the FPU.
pub(crate) const QNAN: u32 = 0x7fc0_0000;

/// `CONST.S fr, imm`: 0.0, 1.0, 2.0, 0.5 for imm 0..3 (imm 4..15 are
/// reserved; they repeat the table, like the QEMU Xtensa model).
pub(crate) fn const_s(imm: u8) -> u32 {
    [0x0000_0000, 0x3f80_0000, 0x4000_0000, 0x3f00_0000][(imm & 3) as usize]
}

/// Unbiased exponent `e` of a finite non-zero f64, `|v| = f * 2^e`, `f` in [1, 2).
fn ilogb(v: f64) -> i32 {
    let bits = v.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    if biased != 0 {
        biased - 1023
    } else {
        // f64 subnormal (never reached from f32 operands, kept for totality).
        let mant = bits & ((1u64 << 52) - 1);
        -1022 - (mant.leading_zeros() as i32 - 11)
    }
}

/// `2^e` as an f64 (exact for the range used here, |e| < 1000).
fn pow2(e: i32) -> f64 {
    f64::from_bits(((e + 1023) as u64) << 52)
}

/// Split finite non-zero `v` into `(m, e)` with `|v| = m * 2^e`, `m` in
/// [1, 4) and `e` even: the `NEXP01.S` narrowing.
fn narrow(v: f64) -> (f64, i32) {
    let e = ilogb(v);
    let even = e - e.rem_euclid(2);
    (v.abs() * pow2(-even), even)
}

/// Table-lookup seed model: the seed ops index a table by the leading
/// [`SEED_BITS`]` - 1` fraction bits of the operand, so the seed is `f` of
/// the middle of that operand interval, rounded to [`SEED_BITS`]
/// significant bits. `v` is finite and non-zero.
fn seed(v: f64, f: impl Fn(f64) -> f64) -> f64 {
    let e = ilogb(v);
    let frac = v.abs() * pow2(-e); // [1, 2)
    let steps = pow2(SEED_BITS as i32 - 1);
    let mid = ((frac * steps).floor() + 0.5) / steps;
    round_seed(f(mid * pow2(e)).copysign(f(v)))
}

/// Round finite non-zero `v` to [`SEED_BITS`] significant bits (nearest even).
fn round_seed(v: f64) -> f64 {
    let drop = 52 - (SEED_BITS - 1);
    let bits = v.to_bits();
    let half = 1u64 << (drop - 1);
    let mask = (1u64 << drop) - 1;
    let low = bits & mask;
    let mut hi = bits & !mask;
    if low > half || (low == half && (hi >> drop) & 1 == 1) {
        hi += 1u64 << drop;
    }
    f64::from_bits(hi)
}

fn is_finite_nonzero(v: f64) -> bool {
    v.is_finite() && v != 0.0
}

/// f64 -> f32 bits, round to nearest even; NaN becomes the default NaN.
pub(crate) fn to_f32_bits(v: f64) -> u32 {
    if v.is_nan() {
        QNAN
    } else {
        (v as f32).to_bits()
    }
}

/// `NEXP01.S`: `-m` for finite non-zero `fs` (see [`narrow`]); otherwise the
/// negated magnitude (`-0`, `-inf`) or NaN.
pub(crate) fn nexp01(fs: u32) -> u32 {
    let v = f32::from_bits(fs) as f64;
    if v.is_nan() {
        return QNAN;
    }
    if !is_finite_nonzero(v) {
        return to_f32_bits(-v.abs());
    }
    to_f32_bits(-narrow(v).0)
}

/// `DIV0.S`: seed for `1/m`, `m = |NEXP01.S(fs)|`. 1.0 for a zero, infinite
/// or NaN operand (MKDADJ.S supplies the special result then).
pub(crate) fn div0(fs: u32) -> u32 {
    let v = f32::from_bits(fs) as f64;
    if !is_finite_nonzero(v) {
        return 0x3f80_0000;
    }
    to_f32_bits(seed(narrow(v).0, |m| 1.0 / m))
}

/// `SQRT0.S`: seed for `1/sqrt(m)`, `m = |NEXP01.S(fs)|`. 1.0 for a zero,
/// infinite or NaN operand (MKSADJ.S supplies the special result then).
pub(crate) fn sqrt0(fs: u32) -> u32 {
    let v = f32::from_bits(fs) as f64;
    if !is_finite_nonzero(v) {
        return 0x3f80_0000;
    }
    to_f32_bits(seed(narrow(v).0, |m| 1.0 / m.sqrt()))
}

/// `RECIP0.S`: seed for `1/fs` (full exponent and sign). IEEE specials:
/// `±0 -> ±inf`, `±inf -> ±0`, NaN -> NaN.
pub(crate) fn recip0(fs: u32) -> u32 {
    let v = f32::from_bits(fs) as f64;
    if v.is_nan() {
        return QNAN;
    }
    if !is_finite_nonzero(v) {
        return to_f32_bits(1.0 / v);
    }
    to_f32_bits(seed(v, |x| 1.0 / x))
}

/// `RSQRT0.S`: seed for `1/sqrt(fs)`. `±0 -> ±inf`, `+inf -> +0`, negative or
/// NaN -> NaN.
pub(crate) fn rsqrt0(fs: u32) -> u32 {
    let v = f32::from_bits(fs) as f64;
    if v == 0.0 {
        return to_f32_bits(1.0 / v);
    }
    if v.is_nan() || v < 0.0 {
        return QNAN;
    }
    if v.is_infinite() {
        return 0;
    }
    to_f32_bits(seed(v, |x| 1.0 / x.sqrt()))
}

/// `MKDADJ.S fr, fs`: divide adjustment for `fs / fr` (dividend in `fs`,
/// divisor in `fr`). Returns the exact (possibly out-of-f32-range) value.
pub(crate) fn mkdadj(divisor: f64, dividend: f64) -> f64 {
    let (a, b) = (dividend, divisor);
    if a.is_nan() || b.is_nan() || (a.is_infinite() && b.is_infinite()) || (a == 0.0 && b == 0.0) {
        return f64::NAN;
    }
    let neg = a.is_sign_negative() != b.is_sign_negative();
    let mag = if a.is_infinite() || b == 0.0 {
        f64::INFINITY
    } else if a == 0.0 || b.is_infinite() {
        0.0
    } else {
        pow2(narrow(a).1 - narrow(b).1)
    };
    if neg {
        -mag
    } else {
        mag
    }
}

/// `MKSADJ.S fr, fs`: square-root adjustment for `fs`.
pub(crate) fn mksadj(x: f64) -> f64 {
    if x.is_nan() || (x < 0.0) {
        return f64::NAN;
    }
    if x == 0.0 || x.is_infinite() {
        return x;
    }
    pow2(narrow(x).1 / 2)
}

/// `ADDEXP.S` / `ADDEXPM.S fr, fs`: apply adjustment `fs` to `fr`.
pub(crate) fn addexp(fr: f64, fs: f64) -> f64 {
    if fs.is_nan() {
        return f64::NAN;
    }
    // A special adjustment (±0 / ±inf) overrides whatever the estimate
    // chain left in `fr` (it may be NaN after an infinite operand).
    let neg = fs.is_sign_negative() != (!fr.is_nan() && fr.is_sign_negative());
    let mag = if !is_finite_nonzero(fs) {
        fs.abs()
    } else if fr.is_nan() {
        return f64::NAN;
    } else {
        fr.abs() * pow2(ilogb(fs))
    };
    if neg {
        -mag
    } else {
        mag
    }
}

/// `a + p` rounded once to f32, where `a` and `p` are exact f64 values:
/// round-to-odd into f64 first, then to f32 (53 >= 24 + 2, so the second
/// rounding cannot double-round). Ties go to even, or away from zero when
/// `ties_away`.
fn add_round_once(a: f64, p: f64, ties_away: bool) -> u32 {
    let s = a + p;
    if !s.is_finite() {
        return to_f32_bits(s);
    }
    // TwoSum: err is the exact rounding error of `s`.
    let bb = s - a;
    let err = (a - (s - bb)) + (p - bb);
    if err != 0.0 {
        let bits = s.to_bits();
        let odd = if bits & 1 == 1 {
            s
        } else if (err > 0.0) == (s > 0.0) {
            f64::from_bits(bits + 1)
        } else {
            f64::from_bits(bits - 1)
        };
        return to_f32_bits(odd);
    }
    // `s` is exact. Rounding keeps the sign, so when `s` lies beyond `r` the
    // other neighbour is one ulp further from zero; an exact midpoint goes
    // there when rounding ties away.
    let r = s as f32;
    let d = s - r as f64;
    if ties_away && r.is_finite() && d != 0.0 && (d > 0.0) == (s > 0.0) {
        let away = f32::from_bits(r.to_bits() + 1);
        if away as f64 - s == d {
            return away.to_bits();
        }
    }
    r.to_bits()
}

/// `MADDN.S fr, fs, ft`: `fr + fs*ft`, fused, rounded once to nearest
/// (ties away from zero, see the module notes).
pub(crate) fn maddn(fr: u32, fs: u32, ft: u32) -> u32 {
    let a = f32::from_bits(fr) as f64;
    // 24 x 24 bit product: exact in f64.
    let p = f32::from_bits(fs) as f64 * f32::from_bits(ft) as f64;
    if a.is_nan() || p.is_nan() {
        return QNAN;
    }
    add_round_once(a, p, true)
}

/// `DIVN.S fr, fs, ft`: final `fr + fs*ft`, rounded once. `fr` and `ft` are
/// the exact (wide) adjusted values. A special `fr` (zero, inf, NaN) is the
/// result itself.
pub(crate) fn divn(fr: f64, fs: u32, ft: f64) -> u32 {
    if fr.is_nan() {
        return QNAN;
    }
    if !is_finite_nonzero(fr) {
        return to_f32_bits(fr);
    }
    let p = f32::from_bits(fs) as f64 * ft;
    if p.is_nan() {
        return QNAN;
    }
    add_round_once(fr, p, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny FR file with the wide latch, mirroring the CPU glue.
    struct Fr {
        bits: [u32; 16],
        wide: [Option<(u32, f64)>; 16],
    }
    impl Fr {
        fn new() -> Self {
            Self {
                bits: [0; 16],
                wide: [None; 16],
            }
        }
        fn get(&self, i: usize) -> f64 {
            match self.wide[i] {
                Some((b, v)) if b == self.bits[i] => v,
                _ => f32::from_bits(self.bits[i]) as f64,
            }
        }
        fn set(&mut self, i: usize, b: u32) {
            self.bits[i] = b;
            self.wide[i] = None;
        }
        fn set_wide(&mut self, i: usize, v: f64) {
            let b = to_f32_bits(v);
            self.bits[i] = b;
            self.wide[i] = if v.is_finite() && f32::from_bits(b) as f64 != v {
                Some((b, v))
            } else {
                None
            };
        }
        fn neg(&mut self, r: usize, s: usize) {
            let b = self.bits[s] ^ 0x8000_0000;
            self.set(r, b);
        }
        fn mov(&mut self, r: usize, s: usize) {
            let b = self.bits[s];
            self.set(r, b);
        }
        fn maddn(&mut self, r: usize, s: usize, t: usize) {
            let b = maddn(self.bits[r], self.bits[s], self.bits[t]);
            self.set(r, b);
        }
        fn addexp(&mut self, r: usize, s: usize) {
            let v = addexp(self.get(r), self.get(s));
            self.set_wide(r, v);
        }
    }

    /// The ESP32-S3 ROM `__divsf3` body (0x40056124), op for op.
    fn rom_divsf3(a: f32, b: f32) -> f32 {
        let mut f = Fr::new();
        f.set(1, a.to_bits());
        f.set(2, b.to_bits());
        let v = div0(f.bits[2]);
        f.set(3, v); // div0.s f3, f2
        let v = nexp01(f.bits[2]);
        f.set(4, v); // nexp01.s f4, f2
        f.set(5, const_s(1));
        f.maddn(5, 4, 3);
        f.mov(6, 3);
        f.mov(7, 2);
        let v = nexp01(f.bits[1]);
        f.set(2, v); // nexp01.s f2, f1
        f.maddn(6, 5, 6);
        f.set(5, const_s(1));
        f.set(0, const_s(0));
        f.neg(8, 2);
        f.maddn(5, 4, 6);
        f.maddn(0, 8, 3);
        let v = mkdadj(f.get(7), f.get(1));
        f.set_wide(7, v); // mkdadj.s f7, f1
        f.maddn(6, 5, 6);
        f.maddn(8, 4, 0);
        f.set(3, const_s(1));
        f.maddn(3, 4, 6);
        f.maddn(0, 8, 6);
        f.neg(2, 2);
        f.maddn(6, 3, 6);
        f.maddn(2, 4, 0);
        f.addexp(0, 7); // addexpm.s f0, f7
        f.addexp(6, 7); // addexp.s f6, f7
        f32::from_bits(divn(f.get(0), f.bits[2], f.get(6)))
    }

    /// libgcc `__ieee754_sqrtf` (esp-14.2.0 esp32s3 libgcc.a), op for op.
    fn libgcc_sqrtf(x: f32) -> f32 {
        let mut f = Fr::new();
        f.set(1, x.to_bits());
        let v = sqrt0(f.bits[1]);
        f.set(2, v);
        f.set(3, const_s(0));
        f.maddn(3, 2, 2);
        let v = nexp01(f.bits[1]);
        f.set(4, v);
        f.set(0, const_s(3));
        f.addexp(4, 0);
        f.maddn(0, 3, 4);
        let v = nexp01(f.bits[1]);
        f.set(3, v);
        f.neg(5, 3);
        f.maddn(2, 0, 2);
        f.set(0, const_s(0));
        f.set(6, const_s(0));
        f.set(7, const_s(0));
        f.maddn(0, 5, 2);
        f.maddn(6, 2, 4);
        f.set(4, const_s(3));
        f.maddn(7, 4, 2);
        f.maddn(3, 0, 0);
        f.maddn(4, 6, 2);
        f.neg(2, 7);
        f.maddn(0, 3, 2);
        f.maddn(7, 4, 7);
        let v = mksadj(f.get(1));
        f.set_wide(2, v);
        let v = nexp01(f.bits[1]);
        f.set(1, v);
        f.maddn(1, 0, 0);
        f.neg(3, 7);
        f.addexp(0, 2); // addexpm.s f0, f2
        f.addexp(3, 2); // addexp.s f3, f2
        f32::from_bits(divn(f.get(0), f.bits[1], f.get(3)))
    }

    fn same(got: f32, want: f32) -> bool {
        (got.is_nan() && want.is_nan()) || got.to_bits() == want.to_bits()
    }

    /// xorshift64*: deterministic operand stream.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 32) as u32
        }
    }

    const EDGES: [u32; 22] = [
        0x0000_0000,
        0x8000_0000,
        0x7f80_0000,
        0xff80_0000,
        0x7fc0_0000,
        0x0000_0001,
        0x8000_0001,
        0x007f_ffff,
        0x0080_0000,
        0x0080_0001,
        0x7f7f_ffff,
        0xff7f_ffff,
        0x3f80_0000,
        0xbf80_0000,
        0x4040_0000,
        0x3f80_0001,
        0x3f7f_ffff,
        0x4000_0000,
        0x3eaa_aaab,
        0x4120_0000,
        0x0040_0000,
        0x7f00_0000,
    ];

    #[test]
    fn const_s_table() {
        assert_eq!(f32::from_bits(const_s(0)), 0.0);
        assert_eq!(f32::from_bits(const_s(1)), 1.0);
        assert_eq!(f32::from_bits(const_s(2)), 2.0);
        assert_eq!(f32::from_bits(const_s(3)), 0.5);
    }

    #[test]
    fn nexp01_narrows_to_one_to_four_keeping_exponent_parity() {
        assert_eq!(f32::from_bits(nexp01(8.0f32.to_bits())), -2.0); // 2^3 -> 2 * 2^2
        assert_eq!(f32::from_bits(nexp01(3.0f32.to_bits())), -3.0);
        assert_eq!(f32::from_bits(nexp01((-0.375f32).to_bits())), -1.5);
        assert_eq!(f32::from_bits(nexp01(0x0000_0001)), -2.0); // 2^-149
        assert_eq!(nexp01(0), 0x8000_0000);
        assert_eq!(nexp01(0x7f80_0000), 0xff80_0000);
    }

    #[test]
    fn seeds_are_within_2_pow_minus_7() {
        let mut r = Rng(0x9e37_79b9_7f4a_7c15);
        for _ in 0..20_000 {
            let x = f32::from_bits(r.next() & 0x7fff_ffff);
            if !(x.is_finite() && x != 0.0) {
                continue;
            }
            let m = -(f32::from_bits(nexp01(x.to_bits())) as f64);
            let y = f32::from_bits(div0(x.to_bits())) as f64;
            assert!((y * m - 1.0).abs() <= 1.0 / 128.0, "div0 {x}");
            let y = f32::from_bits(sqrt0(x.to_bits())) as f64;
            assert!((y * y * m - 1.0).abs() <= 1.0 / 64.0, "sqrt0 {x}");
            let y = f32::from_bits(recip0(x.to_bits())) as f64;
            if y.is_finite() && y != 0.0 && (x as f64) > 1e-38 && (x as f64) < 1e38 {
                assert!((y * x as f64 - 1.0).abs() <= 1.0 / 128.0, "recip0 {x}");
            }
            let y = f32::from_bits(rsqrt0(x.to_bits())) as f64;
            assert!((y * y * x as f64 - 1.0).abs() <= 1.0 / 64.0, "rsqrt0 {x}");
        }
    }

    #[test]
    fn maddn_is_fused() {
        // 1 + (1+2^-23)(1-2^-23) = 2 - 2^-46: unfused gives 2.0 exactly.
        let a = f32::from_bits(0x3f80_0001);
        let b = f32::from_bits(0x3f7f_fffe);
        let r = f32::from_bits(maddn((-1.0f32).to_bits(), a.to_bits(), b.to_bits()));
        assert_eq!(r as f64, a as f64 * b as f64 - 1.0);
        assert_ne!(r, 0.0);
    }

    #[test]
    fn rom_divsf3_sequence_is_ieee_correct_on_representative_values() {
        let cases: [(f32, f32); 16] = [
            (1.0, 3.0),
            (1.0, 1.999_999_9), // all-ones divisor mantissa
            (f32::MIN_POSITIVE, 0.999_999_94),
            (2.0, 3.0),
            (10.0, 4.0),
            (-7.0, 2.0),
            (1.0, -0.0),
            (0.0, 5.0),
            (-0.0, 5.0),
            (20_000_000.0, 3.0),
            (1.0e38, 1.0e-3), // overflow -> inf
            (1.0e-38, 1.0e3), // subnormal result
            (f32::MIN_POSITIVE, 3.0),
            (f32::MAX, 0.5),
            (f32::MAX, 2.0),
            (1.0, f32::INFINITY),
        ];
        for (a, b) in cases {
            let got = rom_divsf3(a, b);
            assert!(
                same(got, a / b),
                "{a:e} / {b:e}: got {got:e}, want {:e}",
                a / b
            );
        }
        assert!(rom_divsf3(0.0, 0.0).is_nan());
        assert!(rom_divsf3(f32::INFINITY, f32::INFINITY).is_nan());
        assert!(rom_divsf3(f32::NAN, 1.0).is_nan());
        assert_eq!(rom_divsf3(f32::INFINITY, -2.0), f32::NEG_INFINITY);
    }

    #[test]
    fn rom_divsf3_sequence_matches_ieee_on_random_and_edge_operands() {
        for &a in &EDGES {
            for &b in &EDGES {
                let (a, b) = (f32::from_bits(a), f32::from_bits(b));
                assert!(same(rom_divsf3(a, b), a / b), "{a:e} / {b:e}");
            }
        }
        let mut r = Rng(0x1234_5678_9abc_def1);
        for i in 0..400_000 {
            let mut a = r.next();
            let mut b = r.next();
            // Half the draws: operands near 1 (no range effects).
            if i & 1 == 0 {
                a = (a & 0x807f_ffff) | 0x3f80_0000;
                b = (b & 0x807f_ffff) | 0x3f80_0000;
            }
            let (a, b) = (f32::from_bits(a), f32::from_bits(b));
            assert!(same(rom_divsf3(a, b), a / b), "{a:e} / {b:e}");
        }
    }

    #[test]
    fn libgcc_sqrtf_sequence_matches_ieee() {
        for &x in &EDGES {
            let x = f32::from_bits(x);
            assert!(same(libgcc_sqrtf(x), x.sqrt()), "sqrt {x:e}");
        }
        let mut r = Rng(0xdead_beef_cafe_f00d);
        for _ in 0..200_000 {
            let x = f32::from_bits(r.next() & 0x7fff_ffff);
            assert!(same(libgcc_sqrtf(x), x.sqrt()), "sqrt {x:e}");
        }
        assert!(libgcc_sqrtf(-1.0).is_nan());
        assert_eq!(libgcc_sqrtf(-0.0).to_bits(), 0x8000_0000);
    }

    #[test]
    fn libgcc_recip_and_rsqrt_sequences_converge() {
        // __recipsf2: recip0, then two `const 1; msub; maddn` refinements.
        // msub.s is the non-fused FPU op (product and sum rounded apart).
        let recip = |x: f32| {
            let mut y = f32::from_bits(recip0(x.to_bits()));
            for _ in 0..2 {
                let e = 1.0f32 - x * y;
                y = f32::from_bits(maddn(y.to_bits(), y.to_bits(), e.to_bits()));
            }
            y
        };
        let rsqrt = |x: f32| {
            let mut y = f32::from_bits(rsqrt0(x.to_bits()));
            for _ in 0..2 {
                let h = 0.5f32 * y;
                let e = 1.0f32 - (x * y) * y;
                y = f32::from_bits(maddn(y.to_bits(), h.to_bits(), e.to_bits()));
            }
            y
        };
        for x in [3.0f32, 0.1, 7.5e9, 1.0e-20, 1.0, 2.0] {
            let r = recip(x) as f64;
            assert!((r * x as f64 - 1.0).abs() < 2e-7, "recip {x}");
            let r = rsqrt(x) as f64;
            assert!((r * r * x as f64 - 1.0).abs() < 4e-7, "rsqrt {x}");
        }
    }
}
