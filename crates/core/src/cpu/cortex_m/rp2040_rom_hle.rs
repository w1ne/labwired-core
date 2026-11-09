// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RP2040 boot-ROM soft-float / soft-double, high-level emulated.
//!
//! The clean-room ROM (`crates/core/roms/rp2040/rp2040_bootrom.S`) points
//! every 'SF' and 'SD' table entry at `bkpt #imm ; bx lr`, where
//! `imm = 0xC0 + offset / 4` for the float table and `0xE0 + offset / 4` for
//! the double table. This module serves those immediates, so the twin
//! carries no Raspberry Pi float code.
//!
//! Behaviour is the public RP2040 datasheet's, section 2.8.3.2:
//! - inputs: denormals are zero and NaNs are infinities (sign kept);
//! - outputs: denormals flush to zero and NaNs become infinities;
//! - round to nearest, ties to even (Rust's IEEE arithmetic);
//! - float/double -> integer and fixed-point conversions round towards
//!   -Infinity and clamp to the destination range;
//! - `sqrt` and `ln` of an out-of-domain input return -Infinity;
//! - `cmp` (offset 0x54) returns -1 / 0 / 1;
//! - offset 0x48 is V3 `sincos`: sin in r0 (r0:r1 for double), cos in r1
//!   (r2:r3 for double).
//!
//! Arguments and results use the AAPCS soft-float registers. Only result
//! registers change. Scientific functions go through the `libm` crate so
//! native and wasm builds produce the same bits. Their exact last bit can
//! differ from silicon: the datasheet only promises 1 ULP.
//!
//! The trap is armed only when the RP2040 ROM magic ('M','u',0x01) is at
//! 0x10 and the instruction is in the 16 KiB ROM, so another chip's
//! `bkpt #0xC0..0xFF` still halts as before.

use super::{CortexM, PcAdvance};
use crate::{Bus, SimResult};

const ROM_END: u32 = 0x4000;
const ROM_MAGIC: u32 = 0x0001_754D; // 'M','u',0x01 at 0x10, version byte masked off

/// True when `imm8` names a soft-float/double table entry.
#[inline]
pub(in crate::cpu::cortex_m) fn is_hle_imm(imm8: u8) -> bool {
    imm8 >= 0xC0
}

/// Serve one ROM float trap, or `None` when the trap is not armed.
#[inline(never)]
pub(in crate::cpu::cortex_m) fn handle<B: Bus + ?Sized>(
    cpu: &mut CortexM,
    bus: &mut B,
    imm8: u8,
) -> Option<SimResult<PcAdvance>> {
    if cpu.pc >= ROM_END {
        return None;
    }
    match bus.read_u32(0x10) {
        Ok(w) if w & 0x00FF_FFFF == ROM_MAGIC => {}
        _ => return None,
    }
    let args = [cpu.r0, cpu.r1, cpu.r2, cpu.r3];
    let out = if imm8 >= 0xE0 {
        double_op(imm8 - 0xE0, args)
    } else {
        float_op(imm8 - 0xC0, args)
    };
    match out {
        Out::W1(a) => cpu.r0 = a,
        Out::W2(a, b) => {
            cpu.r0 = a;
            cpu.r1 = b;
        }
        Out::W4(a, b, c, d) => {
            cpu.r0 = a;
            cpu.r1 = b;
            cpu.r2 = c;
            cpu.r3 = d;
        }
    }
    Some(Ok(PcAdvance::Keep))
}

#[derive(Debug, PartialEq, Eq)]
enum Out {
    W1(u32),
    W2(u32, u32),
    W4(u32, u32, u32, u32),
}

fn lo_hi(v: u64) -> Out {
    Out::W2(v as u32, (v >> 32) as u32)
}

fn join(lo: u32, hi: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

// ---- input / output conditioning (datasheet 2.8.3.2.1) -------------------

fn fin(bits: u32) -> f32 {
    let exp = bits & 0x7F80_0000;
    let sign = bits & 0x8000_0000;
    if exp == 0 {
        f32::from_bits(sign) // denormal or zero -> signed zero
    } else if exp == 0x7F80_0000 {
        f32::from_bits(sign | 0x7F80_0000) // NaN or inf -> inf
    } else {
        f32::from_bits(bits)
    }
}

fn fout(v: f32) -> u32 {
    let bits = v.to_bits();
    let sign = bits & 0x8000_0000;
    if v.is_nan() {
        sign | 0x7F80_0000
    } else if bits & 0x7F80_0000 == 0 {
        sign
    } else {
        bits
    }
}

fn din(bits: u64) -> f64 {
    let exp = bits & 0x7FF0_0000_0000_0000;
    let sign = bits & 0x8000_0000_0000_0000;
    if exp == 0 {
        f64::from_bits(sign)
    } else if exp == 0x7FF0_0000_0000_0000 {
        f64::from_bits(sign | 0x7FF0_0000_0000_0000)
    } else {
        f64::from_bits(bits)
    }
}

fn dout(v: f64) -> u64 {
    let bits = v.to_bits();
    let sign = bits & 0x8000_0000_0000_0000;
    if v.is_nan() {
        sign | 0x7FF0_0000_0000_0000
    } else if bits & 0x7FF0_0000_0000_0000 == 0 {
        sign
    } else {
        bits
    }
}

/// `v * 2^n` floored. Exact for any f64 `v` unless it over/underflows, which
/// only pushes the result further into the clamp.
fn scaled_floor(v: f64, n: i32) -> f64 {
    let s = libm::ldexp(v, n);
    libm::floor(s)
}

fn to_i32(v: f64) -> u32 {
    (v as i32) as u32 // `as` saturates; inf clamps
}
fn to_u32(v: f64) -> u32 {
    v as u32
}
fn to_i64(v: f64) -> u64 {
    (v as i64) as u64
}
fn to_u64(v: f64) -> u64 {
    v as u64
}

fn cmp3<T: PartialOrd>(a: T, b: T) -> u32 {
    if a < b {
        (-1i32) as u32
    } else if a > b {
        1
    } else {
        0
    }
}

/// `x / 2^n` as f32 from an exactly representable f64 `x`: one rounding.
fn f32_from_f64_scaled(x: f64, n: i32) -> f32 {
    libm::ldexp(x, -n) as f32
}

// ---- float table ('SF') --------------------------------------------------

fn float_op(slot: u8, a: [u32; 4]) -> Out {
    let x = fin(a[0]);
    let y = fin(a[1]);
    let n = a[1] as i32; // fixed-point position for 32-bit fix ops
    let n64 = a[2] as i32; // for 64-bit fix ops (value in r0:r1)
    let f = |v: f32| Out::W1(fout(v));
    match slot {
        0x00 => f(x + y),
        0x01 => f(x - y),
        0x02 => f(x * y),
        0x03 => f(x / y),
        // 0x10 / 0x14 are deprecated; serve them as the 3-way compare.
        0x04 | 0x05 | 0x15 => Out::W1(cmp3(x, y)),
        0x06 => f(if x < 0.0 {
            f32::NEG_INFINITY
        } else {
            libm::sqrtf(x)
        }),
        0x07 => Out::W1(to_i32(libm::floor(f64::from(x)))),
        0x08 => Out::W1(to_i32(scaled_floor(f64::from(x), n))),
        0x09 => Out::W1(to_u32(libm::floor(f64::from(x)))),
        0x0A => Out::W1(to_u32(scaled_floor(f64::from(x), n))),
        0x0B => f(a[0] as i32 as f32),
        0x0C => f(f32_from_f64_scaled(f64::from(a[0] as i32), n)),
        0x0D => f(a[0] as f32),
        0x0E => f(f32_from_f64_scaled(f64::from(a[0]), n)),
        0x0F => f(libm::cos(f64::from(x)) as f32),
        0x10 => f(libm::sin(f64::from(x)) as f32),
        0x11 => f(libm::tan(f64::from(x)) as f32),
        0x12 => {
            let v = f64::from(x);
            Out::W2(fout(libm::sin(v) as f32), fout(libm::cos(v) as f32))
        }
        0x13 => f(libm::exp(f64::from(x)) as f32),
        0x14 => f(if x <= 0.0 {
            f32::NEG_INFINITY
        } else {
            libm::log(f64::from(x)) as f32
        }),
        0x16 => f(libm::atan2(f64::from(x), f64::from(y)) as f32),
        0x17 => f(join(a[0], a[1]) as i64 as f32),
        0x18 => f(f32_scaled_64(join(a[0], a[1]) as i64 as f32, n64)),
        0x19 => f(join(a[0], a[1]) as f32),
        0x1A => f(f32_scaled_64(join(a[0], a[1]) as f32, n64)),
        0x1B => lo_hi(to_i64(libm::floor(f64::from(x)))),
        0x1C => lo_hi(to_i64(scaled_floor(f64::from(x), n))),
        0x1D => lo_hi(to_u64(libm::floor(f64::from(x)))),
        0x1E => lo_hi(to_u64(scaled_floor(f64::from(x), n))),
        0x1F => lo_hi(dout(f64::from(x))),
        _ => Out::W1(0),
    }
}

/// `v / 2^n` for a value already rounded once to f32: scaling an f32 in f64
/// is exact, so the final cast is the only other rounding (only reached on
/// underflow/overflow).
fn f32_scaled_64(v: f32, n: i32) -> f32 {
    libm::ldexp(f64::from(v), -n) as f32
}

// ---- double table ('SD') -------------------------------------------------

fn double_op(slot: u8, a: [u32; 4]) -> Out {
    let xb = join(a[0], a[1]);
    let x = din(xb);
    let y = din(join(a[2], a[3]));
    let n = a[2] as i32; // fixed-point position (value in r0:r1)
    let d = |v: f64| lo_hi(dout(v));
    match slot {
        0x00 => d(x + y),
        0x01 => d(x - y),
        0x02 => d(x * y),
        0x03 => d(x / y),
        0x04 | 0x05 | 0x15 => Out::W1(cmp3(x, y)),
        0x06 => d(if x < 0.0 {
            f64::NEG_INFINITY
        } else {
            libm::sqrt(x)
        }),
        0x07 => Out::W1(to_i32(libm::floor(x))),
        0x08 => Out::W1(to_i32(scaled_floor(x, n))),
        0x09 => Out::W1(to_u32(libm::floor(x))),
        0x0A => Out::W1(to_u32(scaled_floor(x, n))),
        // 32-bit sources: value in r0, n in r1.
        0x0B => d(f64::from(a[0] as i32)),
        0x0C => d(libm::ldexp(f64::from(a[0] as i32), -(a[1] as i32))),
        0x0D => d(f64::from(a[0])),
        0x0E => d(libm::ldexp(f64::from(a[0]), -(a[1] as i32))),
        0x0F => d(libm::cos(x)),
        0x10 => d(libm::sin(x)),
        0x11 => d(libm::tan(x)),
        0x12 => {
            let s = dout(libm::sin(x));
            let c = dout(libm::cos(x));
            Out::W4(s as u32, (s >> 32) as u32, c as u32, (c >> 32) as u32)
        }
        0x13 => d(libm::exp(x)),
        0x14 => d(if x <= 0.0 {
            f64::NEG_INFINITY
        } else {
            libm::log(x)
        }),
        0x16 => d(libm::atan2(x, y)),
        0x17 => d(xb as i64 as f64),
        0x18 => d(libm::ldexp(xb as i64 as f64, -n)),
        0x19 => d(xb as f64),
        0x1A => d(libm::ldexp(xb as f64, -n)),
        0x1B => lo_hi(to_i64(libm::floor(x))),
        0x1C => lo_hi(to_i64(scaled_floor(x, n))),
        0x1D => lo_hi(to_u64(libm::floor(x))),
        0x1E => lo_hi(to_u64(scaled_floor(x, n))),
        0x1F => Out::W1(fout(x as f32)),
        _ => Out::W1(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f1(slot: u8, a: f32, b: u32) -> u32 {
        match float_op(slot, [a.to_bits(), b, 0, 0]) {
            Out::W1(v) => v,
            o => panic!("{o:?}"),
        }
    }
    fn ff(slot: u8, a: f32, b: f32) -> f32 {
        f32::from_bits(f1(slot, a, b.to_bits()))
    }
    fn dd(slot: u8, a: f64, b: f64) -> f64 {
        let (x, y) = (a.to_bits(), b.to_bits());
        match double_op(
            slot,
            [x as u32, (x >> 32) as u32, y as u32, (y >> 32) as u32],
        ) {
            Out::W2(lo, hi) => f64::from_bits(join(lo, hi)),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn basic_float_ops_are_ieee() {
        assert_eq!(ff(0x00, 1.5, 2.25), 3.75);
        assert_eq!(ff(0x01, 1.5, 2.25), -0.75);
        assert_eq!(ff(0x02, 1.5, 2.25), 3.375);
        assert_eq!(ff(0x03, 1.0, 3.0), 1.0f32 / 3.0);
        assert_eq!(ff(0x06, 2.0, 0.0), 2.0f32.sqrt());
        assert_eq!(f1(0x15, 1.0, 2.0f32.to_bits()), u32::MAX);
        assert_eq!(f1(0x15, -0.0, 0.0f32.to_bits()), 0);
    }

    #[test]
    fn datasheet_edge_rules() {
        // denormal input is zero; NaN input is infinity; NaN output is infinity
        let denorm = f32::from_bits(1);
        assert_eq!(ff(0x00, denorm, 0.0), 0.0);
        assert_eq!(ff(0x00, f32::NAN, 1.0), f32::INFINITY);
        assert!(ff(0x01, f32::INFINITY, f32::INFINITY).is_infinite());
        // denormal output flushes to zero
        assert_eq!(ff(0x02, 1e-30, 1e-10), 0.0);
        // sqrt / ln of out-of-domain input
        assert_eq!(ff(0x06, -4.0, 0.0), f32::NEG_INFINITY);
        assert_eq!(ff(0x14, 0.0, 0.0), f32::NEG_INFINITY);
    }

    #[test]
    fn conversions_floor_and_clamp() {
        assert_eq!(f1(0x07, -1.5, 0) as i32, -2);
        assert_eq!(f1(0x07, 3e9, 0) as i32, i32::MAX);
        assert_eq!(f1(0x08, 0.5, 16), 0x8000); // datasheet example
        assert_eq!(f1(0x09, -1.0, 0), 0);
        assert_eq!(f1(0x0A, 0.5, 16), 0x8000);
        assert_eq!(
            float_op(0x0C, [0x8000, 16, 0, 0]),
            Out::W1(0.5f32.to_bits())
        );
        assert_eq!(
            float_op(0x0B, [(-7i32) as u32, 0, 0, 0]),
            Out::W1((-7.0f32).to_bits())
        );
        assert_eq!(
            float_op(0x1F, [1.25f32.to_bits(), 0, 0, 0]),
            lo_hi(1.25f64.to_bits())
        );
        assert_eq!(
            float_op(0x1B, [(-2.5f32).to_bits(), 0, 0, 0]),
            lo_hi((-3i64) as u64)
        );
    }

    #[test]
    fn double_ops() {
        assert_eq!(dd(0x00, 0.1, 0.2), 0.1 + 0.2);
        assert_eq!(dd(0x03, 1.0, 3.0), 1.0 / 3.0);
        assert_eq!(dd(0x06, -1.0, 0.0), f64::NEG_INFINITY);
        assert_eq!(dd(0x10, 0.5, 0.0), libm::sin(0.5));
        let one = 1.0f64.to_bits();
        assert_eq!(
            double_op(0x15, [0, 0, one as u32, (one >> 32) as u32]),
            Out::W1(u32::MAX)
        );
        assert_eq!(
            double_op(0x0B, [(-3i32) as u32, 0, 0, 0]),
            lo_hi((-3.0f64).to_bits())
        );
        let half = 0.5f64.to_bits();
        assert_eq!(
            double_op(0x08, [half as u32, (half >> 32) as u32, 16, 0]),
            Out::W1(0x8000)
        );
        assert_eq!(
            double_op(0x1F, [half as u32, (half >> 32) as u32, 0, 0]),
            Out::W1(0.5f32.to_bits())
        );
        match double_op(0x12, [0, 0, 0, 0]) {
            Out::W4(s0, s1, c0, c1) => {
                assert_eq!(f64::from_bits(join(s0, s1)), 0.0);
                assert_eq!(f64::from_bits(join(c0, c1)), 1.0);
            }
            o => panic!("{o:?}"),
        }
    }
}
