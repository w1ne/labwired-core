// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RV32F single-precision arithmetic with every RISC-V rounding mode and
//! accrued exception flag.
//!
//! The host's `f32` operators only round to nearest-even and report no
//! flags, so each operation is computed exactly in `f64` and then rounded
//! here. An `f32` sum, product or fused product-sum is exact in `f64`, given an
//! error term: TwoSum gives the exact low part of a sum, an `f32` product
//! fits in 48 bits, and a quotient or square root comes with the sign of its
//! fused residual. That exact value is then placed on the `f32` grid under
//! the requested rounding mode, with tininess detected after rounding as the
//! ISA specifies (unprivileged spec, chapter "F" Standard Extension). NaN
//! results are always the canonical NaN, `0x7FC0_0000`.

/// `fflags` bits (`fcsr[4:0]`).
pub const NX: u8 = 1 << 0;
pub const UF: u8 = 1 << 1;
pub const OF: u8 = 1 << 2;
pub const DZ: u8 = 1 << 3;
pub const NV: u8 = 1 << 4;

pub const CANONICAL_NAN: u32 = 0x7FC0_0000;

const SIGN: u32 = 0x8000_0000;
/// 2^128: the first point of the `f32` grid past `f32::MAX` with the
/// exponent range unbounded.
const OVERFLOW_POINT: f64 = 340_282_366_920_938_463_463_374_607_431_768_211_456.0;
const MIN_NORMAL: f64 = f32::MIN_POSITIVE as f64;

/// A static rounding mode (`rm` field, or `frm` for the dynamic encoding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rm {
    Rne,
    Rtz,
    Rdn,
    Rup,
    Rmm,
}

impl Rm {
    /// Decode a 3-bit mode. 5 and 6 are reserved, and 7 (dynamic) is not a
    /// mode on its own: both return `None`, which makes the instruction
    /// illegal.
    pub fn from_bits(bits: u32) -> Option<Self> {
        Some(match bits {
            0 => Self::Rne,
            1 => Self::Rtz,
            2 => Self::Rdn,
            3 => Self::Rup,
            4 => Self::Rmm,
            _ => return None,
        })
    }
}

/// Rounding expressed on a magnitude, once the sign is known.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MagRound {
    NearestEven,
    NearestAway,
    TowardZero,
    AwayFromZero,
}

fn mag_round(rm: Rm, negative: bool) -> MagRound {
    match (rm, negative) {
        (Rm::Rne, _) => MagRound::NearestEven,
        (Rm::Rmm, _) => MagRound::NearestAway,
        (Rm::Rtz, _) | (Rm::Rdn, false) | (Rm::Rup, true) => MagRound::TowardZero,
        (Rm::Rdn, true) | (Rm::Rup, false) => MagRound::AwayFromZero,
    }
}

pub fn is_nan(bits: u32) -> bool {
    (bits & 0x7F80_0000) == 0x7F80_0000 && (bits & 0x007F_FFFF) != 0
}

pub fn is_snan(bits: u32) -> bool {
    is_nan(bits) && (bits & 0x0040_0000) == 0
}

fn f(bits: u32) -> f64 {
    f32::from_bits(bits) as f64
}

/// Spacing of the `f32` grid in the binade holding `m` (`m > 0`), with the
/// exponent range unbounded above: values at or past 2^128 keep the 2^104
/// spacing of the top binade.
fn grid_spacing(m: f64) -> f64 {
    if m < MIN_NORMAL {
        return f64::from_bits(((1023 - 149) as u64) << 52);
    }
    let exp = ((m.to_bits() >> 52) & 0x7FF) as i64 - 1023;
    f64::from_bits(((exp.min(127) - 23 + 1023) as u64) << 52)
}

fn next_down_f64(m: f64) -> f64 {
    // `m` is positive and finite here.
    f64::from_bits(m.to_bits() - 1)
}

/// Round the exact positive magnitude `m + ε` to the `f32` grid, where `ε` is
/// smaller than half an `f64` ulp of `m` and only its sign, `dir`, is known.
/// Returns the grid value (2^128 or beyond means overflow) and whether it
/// is inexact.
fn round_magnitude(m: f64, dir: i8, mode: MagRound) -> (f64, bool) {
    let probe = if dir < 0 { next_down_f64(m) } else { m };
    let spacing = grid_spacing(probe);
    let lower = (probe / spacing).floor() * spacing;
    if lower == m && dir == 0 {
        return (m, false);
    }
    let upper = lower + spacing;
    let mid = lower + spacing / 2.0;
    let versus_mid = if m != mid {
        m.partial_cmp(&mid).unwrap()
    } else {
        dir.cmp(&0)
    };
    let take_upper = match mode {
        MagRound::TowardZero => false,
        MagRound::AwayFromZero => true,
        MagRound::NearestAway => versus_mid != std::cmp::Ordering::Less,
        MagRound::NearestEven => match versus_mid {
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Equal => ((lower / spacing) as u64) % 2 == 1,
        },
    };
    (if take_upper { upper } else { lower }, true)
}

/// Round the exact value `hi + ε` (`ε` as in [`round_magnitude`], with sign
/// `dir`) to an `f32`, raising NX/UF/OF. `hi` is finite and nonzero.
fn round_nonzero(hi: f64, dir: i8, rm: Rm) -> (u32, u8) {
    let negative = hi < 0.0;
    let m = hi.abs();
    let dir = if negative { -dir } else { dir };
    let mode = mag_round(rm, negative);
    let sign = if negative { SIGN } else { 0 };

    if m >= OVERFLOW_POINT {
        let bits = if mode == MagRound::TowardZero {
            f32::MAX.to_bits()
        } else {
            f32::INFINITY.to_bits()
        };
        return (sign | bits, OF | NX);
    }
    let (value, inexact) = round_magnitude(m, dir, mode);
    if value >= OVERFLOW_POINT {
        return (sign | f32::INFINITY.to_bits(), OF | NX);
    }
    let mut flags = 0;
    if inexact {
        flags |= NX;
        // Tininess after rounding: round again as if the exponent range
        // were unbounded below (scaling by 2^64 moves every f32 subnormal
        // into the normal range without changing which way it rounds).
        if m < MIN_NORMAL {
            let scale = 18_446_744_073_709_551_616.0; // 2^64
            let scaled = m * scale;
            let tiny =
                scaled < MIN_NORMAL || round_magnitude(scaled, dir, mode).0 < MIN_NORMAL * scale;
            if tiny {
                flags |= UF;
            }
        }
    }
    (sign | (value as f32).to_bits(), flags)
}

/// The sum `x + y` of two finite values that are exact in `f64` (an `f32`
/// or an `f32` product, so the sum cannot overflow), rounded once.
fn round_sum(x: f64, y: f64, rm: Rm) -> (u32, u8) {
    let s = x + y;
    let bb = s - x;
    let err = (x - (s - bb)) + (y - bb);
    if s == 0.0 && err == 0.0 {
        // An exact zero: both addends zero keep a shared sign; any other
        // exact cancellation is +0, or -0 when rounding down.
        let both_neg = x == 0.0 && y == 0.0 && x.is_sign_negative() && y.is_sign_negative();
        let both_pos = x == 0.0 && y == 0.0 && !x.is_sign_negative() && !y.is_sign_negative();
        let negative = both_neg || (!both_pos && rm == Rm::Rdn);
        return (if negative { SIGN } else { 0 }, 0);
    }
    // TwoSum: s == 0 only for an exact zero, handled above.
    round_nonzero(s, sign_of(err), rm)
}

fn sign_of(v: f64) -> i8 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}

fn nan_result(inputs: &[u32]) -> Option<(u32, u8)> {
    if inputs.iter().any(|&b| is_nan(b)) {
        let nv = if inputs.iter().any(|&b| is_snan(b)) {
            NV
        } else {
            0
        };
        Some((CANONICAL_NAN, nv))
    } else {
        None
    }
}

pub fn add(a: u32, b: u32, rm: Rm) -> (u32, u8) {
    if let Some(r) = nan_result(&[a, b]) {
        return r;
    }
    let (x, y) = (f(a), f(b));
    if x.is_infinite() || y.is_infinite() {
        let s = x + y;
        return if s.is_nan() {
            (CANONICAL_NAN, NV)
        } else {
            ((s as f32).to_bits(), 0)
        };
    }
    round_sum(x, y, rm)
}

pub fn sub(a: u32, b: u32, rm: Rm) -> (u32, u8) {
    if let Some(r) = nan_result(&[a, b]) {
        return r;
    }
    add(a, b ^ SIGN, rm)
}

pub fn mul(a: u32, b: u32, rm: Rm) -> (u32, u8) {
    if let Some(r) = nan_result(&[a, b]) {
        return r;
    }
    let p = f(a) * f(b);
    if p.is_nan() {
        return (CANONICAL_NAN, NV);
    }
    if p.is_infinite() || p == 0.0 {
        return ((p as f32).to_bits(), 0);
    }
    round_nonzero(p, 0, rm)
}

pub fn div(a: u32, b: u32, rm: Rm) -> (u32, u8) {
    if let Some(r) = nan_result(&[a, b]) {
        return r;
    }
    let (x, y) = (f(a), f(b));
    if y == 0.0 && x != 0.0 && x.is_finite() {
        return ((((a ^ b) & SIGN) | f32::INFINITY.to_bits()), DZ);
    }
    let q = x / y;
    if q.is_nan() {
        return (CANONICAL_NAN, NV);
    }
    if q.is_infinite() || q == 0.0 {
        return ((q as f32).to_bits(), 0);
    }
    // exact = q + r/y
    let r = libm::fma(-q, y, x);
    round_nonzero(q, sign_of(r) * sign_of(y), rm)
}

pub fn sqrt(a: u32, rm: Rm) -> (u32, u8) {
    if let Some(r) = nan_result(&[a]) {
        return r;
    }
    let x = f(a);
    if x == 0.0 {
        return (a, 0);
    }
    if x < 0.0 {
        return (CANONICAL_NAN, NV);
    }
    if x.is_infinite() {
        return (a, 0);
    }
    let s = x.sqrt();
    let r = libm::fma(-s, s, x);
    round_nonzero(s, sign_of(r), rm)
}

/// `±(a × b) ± c`, rounded once. `negate_product` and `negate_addend` give
/// FMSUB (addend), FNMSUB (product) and FNMADD (both).
pub fn fma(a: u32, b: u32, c: u32, negate_product: bool, negate_addend: bool, rm: Rm) -> (u32, u8) {
    let (x, y) = (f(a), f(b));
    // ∞ × 0 is invalid even when the addend is a quiet NaN.
    let inf_times_zero = (x.is_infinite() && y == 0.0) || (x == 0.0 && y.is_infinite());
    if let Some((nan, flags)) = nan_result(&[a, b, c]) {
        return (nan, flags | if inf_times_zero { NV } else { 0 });
    }
    if inf_times_zero {
        return (CANONICAL_NAN, NV);
    }
    let mut p = x * y;
    if negate_product {
        p = -p;
    }
    let mut z = f(c);
    if negate_addend {
        z = -z;
    }
    if p.is_infinite() || z.is_infinite() {
        let s = p + z;
        return if s.is_nan() {
            (CANONICAL_NAN, NV)
        } else {
            ((s as f32).to_bits(), 0)
        };
    }
    round_sum(p, z, rm)
}

pub fn sign_inject(a: u32, b: u32, funct3: u32) -> Option<u32> {
    let sign = match funct3 {
        0 => b & SIGN,
        1 => !b & SIGN,
        2 => (a ^ b) & SIGN,
        _ => return None,
    };
    Some((a & !SIGN) | sign)
}

/// FMIN (`max = false`) / FMAX: a single NaN operand yields the other one,
/// two yield the canonical NaN, and -0 orders below +0.
pub fn min_max(a: u32, b: u32, max: bool) -> (u32, u8) {
    let nv = if is_snan(a) || is_snan(b) { NV } else { 0 };
    let value = match (is_nan(a), is_nan(b)) {
        (true, true) => CANONICAL_NAN,
        (true, false) => b,
        (false, true) => a,
        (false, false) => {
            let (x, y) = (f32::from_bits(a), f32::from_bits(b));
            let a_first = if x == y {
                // Only ±0 compare equal with different bits.
                (a & SIGN != 0) != max
            } else {
                (x < y) != max
            };
            if a_first {
                a
            } else {
                b
            }
        }
    };
    (value, nv)
}

/// FEQ (`funct3` 2), FLT (1), FLE (0). FEQ is a quiet comparison; FLT and
/// FLE signal on any NaN.
pub fn compare(a: u32, b: u32, funct3: u32) -> Option<(u32, u8)> {
    let any_nan = is_nan(a) || is_nan(b);
    let (x, y) = (f32::from_bits(a), f32::from_bits(b));
    Some(match funct3 {
        2 => {
            let nv = if is_snan(a) || is_snan(b) { NV } else { 0 };
            ((!any_nan && x == y) as u32, nv)
        }
        1 => ((!any_nan && x < y) as u32, if any_nan { NV } else { 0 }),
        0 => ((!any_nan && x <= y) as u32, if any_nan { NV } else { 0 }),
        _ => return None,
    })
}

pub fn classify(a: u32) -> u32 {
    let negative = a & SIGN != 0;
    let exp = (a >> 23) & 0xFF;
    let frac = a & 0x007F_FFFF;
    let bit = match (exp, frac) {
        (0xFF, 0) => {
            if negative {
                0
            } else {
                7
            }
        }
        (0xFF, _) => {
            if is_snan(a) {
                8
            } else {
                9
            }
        }
        (0, 0) => {
            if negative {
                3
            } else {
                4
            }
        }
        (0, _) => {
            if negative {
                2
            } else {
                5
            }
        }
        _ => {
            if negative {
                1
            } else {
                6
            }
        }
    };
    1 << bit
}

/// FCVT.W.S (`unsigned = false`) / FCVT.WU.S. Out-of-range values and NaN
/// raise NV and saturate; NaN converts like +∞.
pub fn to_int(a: u32, unsigned: bool, rm: Rm) -> (u32, u8) {
    let (lo, hi) = if unsigned {
        (0.0, u32::MAX as f64)
    } else {
        (i32::MIN as f64, i32::MAX as f64)
    };
    let saturate = |negative: bool| -> u32 {
        match (unsigned, negative) {
            (true, true) => 0,
            (true, false) => u32::MAX,
            (false, true) => i32::MIN as u32,
            (false, false) => i32::MAX as u32,
        }
    };
    if is_nan(a) {
        return (saturate(false), NV);
    }
    let x = f(a);
    let r = match rm {
        Rm::Rne => x.round_ties_even(),
        Rm::Rtz => x.trunc(),
        Rm::Rdn => x.floor(),
        Rm::Rup => x.ceil(),
        Rm::Rmm => x.round(),
    };
    if !(lo..=hi).contains(&r) {
        return (saturate(x < 0.0), NV);
    }
    let value = if unsigned { r as u32 } else { r as i32 as u32 };
    (value, if r != x { NX } else { 0 })
}

/// FCVT.S.W (`unsigned = false`) / FCVT.S.WU.
pub fn from_int(v: u32, unsigned: bool, rm: Rm) -> (u32, u8) {
    let x = if unsigned { v as f64 } else { v as i32 as f64 };
    if x == 0.0 {
        return (0, 0);
    }
    round_nonzero(x, 0, rm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(v: f32) -> u32 {
        v.to_bits()
    }

    #[test]
    fn nearest_even_matches_host_f32_on_simple_values() {
        assert_eq!(add(bits(1.5), bits(2.25), Rm::Rne), (bits(3.75), 0));
        assert_eq!(mul(bits(3.0), bits(-0.5), Rm::Rne), (bits(-1.5), 0));
        let (q, fl) = div(bits(1.0), bits(3.0), Rm::Rne);
        assert_eq!(q, bits(1.0 / 3.0));
        assert_eq!(fl, NX);
        assert_eq!(sqrt(bits(2.0), Rm::Rne).0, bits(2.0f32.sqrt()));
    }

    #[test]
    fn directed_rounding_brackets_one_third() {
        let down = div(bits(1.0), bits(3.0), Rm::Rdn).0;
        let up = div(bits(1.0), bits(3.0), Rm::Rup).0;
        assert_eq!(up, down + 1);
        assert_eq!(div(bits(1.0), bits(3.0), Rm::Rtz).0, down);
        assert_eq!(div(bits(-1.0), bits(3.0), Rm::Rtz).0, down | SIGN);
        assert_eq!(div(bits(-1.0), bits(3.0), Rm::Rdn).0, up | SIGN);
    }

    #[test]
    fn exceptional_cases_raise_the_spec_flags() {
        assert_eq!(
            div(bits(1.0), bits(0.0), Rm::Rne),
            (bits(f32::INFINITY), DZ)
        );
        assert_eq!(div(bits(0.0), bits(0.0), Rm::Rne), (CANONICAL_NAN, NV));
        assert_eq!(sqrt(bits(-1.0), Rm::Rne), (CANONICAL_NAN, NV));
        assert_eq!(sqrt(bits(-0.0), Rm::Rne), (bits(-0.0), 0));
        assert_eq!(
            mul(bits(f32::MAX), bits(2.0), Rm::Rne),
            (bits(f32::INFINITY), OF | NX)
        );
        assert_eq!(
            mul(bits(f32::MAX), bits(2.0), Rm::Rtz),
            (bits(f32::MAX), OF | NX)
        );
        // Smallest normal × 0.5 is an exact subnormal: no UF.
        assert_eq!(
            mul(bits(f32::MIN_POSITIVE), bits(0.5), Rm::Rne),
            (bits(f32::MIN_POSITIVE) >> 1, 0)
        );
        // Smallest subnormal × 0.5 is inexact and tiny.
        assert_eq!(mul(1, bits(0.5), Rm::Rne), (0, UF | NX));
        assert_eq!(
            fma(bits(f32::INFINITY), 0, CANONICAL_NAN, false, false, Rm::Rne).1,
            NV
        );
    }

    #[test]
    fn exact_cancellation_sign_follows_rounding_mode() {
        assert_eq!(add(bits(1.0), bits(-1.0), Rm::Rne).0, 0);
        assert_eq!(add(bits(1.0), bits(-1.0), Rm::Rdn).0, SIGN);
        assert_eq!(add(bits(-0.0), bits(-0.0), Rm::Rne).0, SIGN);
        assert_eq!(add(bits(0.0), bits(-0.0), Rm::Rdn).0, SIGN);
        assert_eq!(add(bits(0.0), bits(0.0), Rm::Rdn).0, 0);
    }

    #[test]
    fn conversions_saturate_and_round() {
        assert_eq!(to_int(bits(2.5), false, Rm::Rne), (2, NX));
        assert_eq!(to_int(bits(2.5), false, Rm::Rmm), (3, NX));
        assert_eq!(to_int(bits(-2.5), false, Rm::Rdn), ((-3i32) as u32, NX));
        assert_eq!(to_int(bits(-1.0), true, Rm::Rtz), (0, NV));
        assert_eq!(to_int(bits(-0.5), true, Rm::Rtz), (0, NX));
        assert_eq!(to_int(CANONICAL_NAN, false, Rm::Rne), (i32::MAX as u32, NV));
        assert_eq!(to_int(bits(3e9), false, Rm::Rne), (i32::MAX as u32, NV));
        assert_eq!(
            from_int(16_777_217, false, Rm::Rne),
            (bits(16_777_216.0), NX)
        );
        assert_eq!(
            from_int(16_777_217, false, Rm::Rup),
            (bits(16_777_218.0), NX)
        );
        assert_eq!(
            from_int(u32::MAX, true, Rm::Rne),
            (bits(4_294_967_296.0), NX)
        );
    }

    #[test]
    fn min_max_and_compare_nan_rules() {
        assert_eq!(min_max(CANONICAL_NAN, bits(1.0), false), (bits(1.0), 0));
        assert_eq!(min_max(0x7F80_0001, bits(1.0), true), (bits(1.0), NV));
        assert_eq!(min_max(bits(0.0), bits(-0.0), false).0, bits(-0.0));
        assert_eq!(min_max(bits(0.0), bits(-0.0), true).0, bits(0.0));
        assert_eq!(compare(CANONICAL_NAN, bits(1.0), 2), Some((0, 0)));
        assert_eq!(compare(CANONICAL_NAN, bits(1.0), 1), Some((0, NV)));
        assert_eq!(classify(bits(-0.0)), 1 << 3);
        assert_eq!(classify(0x7F80_0001), 1 << 8);
    }

    /// Differential check against the host SSE/FMA unit, which implements
    /// IEEE 754 binary32 with tininess detected after rounding — the same
    /// choice RISC-V makes — for RNE/RTZ/RDN/RUP and the five flags. RMM has
    /// no SSE encoding and is covered by the unit tests above.
    #[cfg(target_arch = "x86_64")]
    #[test]
    #[allow(deprecated)]
    fn matches_host_sse_in_every_shared_rounding_mode() {
        use std::arch::x86_64::*;
        if !std::is_x86_feature_detected!("fma") {
            return;
        }
        // MXCSR flag bits: IE ZE DE OE UE PE -> NV DZ - OF UF NX.
        fn to_fflags(mx: u32) -> u8 {
            let mut fl = 0;
            if mx & 0x01 != 0 {
                fl |= NV;
            }
            if mx & 0x04 != 0 {
                fl |= DZ;
            }
            if mx & 0x08 != 0 {
                fl |= OF;
            }
            if mx & 0x10 != 0 {
                fl |= UF;
            }
            if mx & 0x20 != 0 {
                fl |= NX;
            }
            fl
        }
        #[target_feature(enable = "fma")]
        unsafe fn host(op: u8, a: u32, b: u32, c: u32, rc: u32) -> (u32, u8) {
            let saved = _mm_getcsr();
            // All exceptions masked, no FTZ/DAZ, flags clear.
            _mm_setcsr(0x1F80 | (rc << 13));
            let (va, vb, vc) = (
                _mm_set_ss(f32::from_bits(a)),
                _mm_set_ss(f32::from_bits(b)),
                _mm_set_ss(f32::from_bits(c)),
            );
            let r = match op {
                0 => _mm_add_ss(va, vb),
                1 => _mm_sub_ss(va, vb),
                2 => _mm_mul_ss(va, vb),
                3 => _mm_div_ss(va, vb),
                4 => _mm_sqrt_ss(va),
                _ => _mm_fmadd_ss(va, vb, vc),
            };
            let mx = _mm_getcsr();
            _mm_setcsr(saved);
            (_mm_cvtss_f32(r).to_bits(), to_fflags(mx))
        }
        // Deterministic xorshift over a mix of raw bit patterns and values
        // near the overflow and subnormal edges, where rounding is hardest.
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let r = s as u32;
            match (s >> 40) % 4 {
                0 => r,
                1 => (r & 0x80FF_FFFF) | 0x0000_0000, // subnormal / tiny
                2 => (r & 0x80FF_FFFF) | 0x7E80_0000, // near f32::MAX
                _ => (r & 0x81FF_FFFF) | 0x3F00_0000, // around 1.0
            }
        };
        let modes = [(Rm::Rne, 0), (Rm::Rdn, 1), (Rm::Rup, 2), (Rm::Rtz, 3)];
        for _ in 0..100_000 {
            let (a, b, c) = (next(), next(), next());
            for &(rm, rc) in &modes {
                for op in 0..6u8 {
                    let ours = match op {
                        0 => add(a, b, rm),
                        1 => sub(a, b, rm),
                        2 => mul(a, b, rm),
                        3 => div(a, b, rm),
                        4 => sqrt(a, rm),
                        _ => fma(a, b, c, false, false, rm),
                    };
                    let theirs = unsafe { host(op, a, b, c, rc) };
                    // x86 propagates an input NaN payload; RISC-V returns the
                    // canonical NaN. Compare NaN-ness and flags there.
                    let theirs = if is_nan(theirs.0) {
                        (CANONICAL_NAN, theirs.1)
                    } else {
                        theirs
                    };
                    assert_eq!(
                        ours, theirs,
                        "op {op} rm {rm:?} a={a:#010x} b={b:#010x} c={c:#010x}"
                    );
                }
            }
        }
    }
}
