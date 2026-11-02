// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RP2350 DCP (double-precision coprocessor, datasheet 3.6.2) driven by the
//! pico-sdk's own canned instruction sequences.
//!
//! Fixture `tests/fixtures/rp2350-dcp-canned.elf` (source and build script in
//! `tests/fixtures/rp2350-dcp-canned/`): each `t_*` function is a pico-sdk
//! `pico_double` / `pico_float` routine body — the `dcp_*_m` macro from
//! `hardware_dcp/dcp_canned.inc.S`, expanded through the SDK's unmodified
//! `dcp_instr.inc.S` — so every MCRR/MRRC/MRC/CDP word, register choice and
//! the CPU-side mantissa arithmetic (dmul partial products, ddiv/dsqrt
//! Newton steps) is exactly what an arduino-pico double operation executes.
//!
//! Results are checked two ways: hand-derived IEEE-754 bit patterns, and a
//! sweep against the host's IEEE-754 arithmetic with the DCP's documented
//! deviations applied (3.6.2.9: denormals flushed to zero on input and
//! output, round to nearest even). NaN results are compared as "is NaN".

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};

const FIXTURE: &str = "tests/fixtures/rp2350-dcp-canned.elf";
const SENTINEL: u32 = 0x2000_0000;
const STACK: u32 = 0x2007_0000;

struct Rig {
    m: Machine<CortexM>,
    elf: Vec<u8>,
}

fn load(with_dcp: bool) -> Rig {
    let chip_path = root("configs/chips/rp2350.yaml");
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("rp2350 chip");
    if !with_dcp {
        chip.coprocessors.clear();
    }
    let mut manifest =
        SystemManifest::from_file(root("configs/systems/rp2350-zero.yaml")).expect("system");
    manifest.chip = chip_path.to_str().unwrap().to_string();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    let path = root(FIXTURE);
    let image = labwired_loader::load_elf(&path).expect("fixture elf");
    m.load_firmware(&image).expect("load");
    m.bus.write_u16(u64::from(SENTINEL), 0xe7fe).unwrap();
    Rig {
        m,
        elf: std::fs::read(&path).unwrap(),
    }
}

impl Rig {
    /// Call `name(r0..r3)`; returns (r0, r1, APSR flags).
    fn call(&mut self, name: &str, a: [u32; 4]) -> (u32, u32) {
        let pc = labwired_loader::resolve_symbol_in_elf(&self.elf, name)
            .unwrap_or_else(|| panic!("symbol {name}"));
        let m = &mut self.m;
        m.cpu.set_pc(pc & !1);
        m.cpu.set_sp(STACK);
        m.cpu.lr = SENTINEL | 1;
        m.cpu.r0 = a[0];
        m.cpu.r1 = a[1];
        m.cpu.r2 = a[2];
        m.cpu.r3 = a[3];
        for _ in 0..400 {
            if m.cpu.get_pc() == SENTINEL {
                let cfsr = m.bus.read_u32(0xE000_ED28).unwrap_or(0);
                assert_eq!(cfsr, 0, "{name}: fault status {cfsr:#x}");
                return (m.cpu.r0, m.cpu.r1);
            }
            m.step()
                .unwrap_or_else(|e| panic!("{name}: step at {:#x}: {e:?}", m.cpu.get_pc()));
        }
        panic!("{name} did not return (pc {:#x})", m.cpu.get_pc());
    }

    fn d2(&mut self, name: &str, x: f64, y: f64) -> u64 {
        let (x, y) = (x.to_bits(), y.to_bits());
        let (lo, hi) = self.call(
            name,
            [x as u32, (x >> 32) as u32, y as u32, (y >> 32) as u32],
        );
        u64::from(hi) << 32 | u64::from(lo)
    }

    fn d1(&mut self, name: &str, x: f64) -> u64 {
        self.d2(name, x, 0.0)
    }
}

fn ftz_in(x: f64) -> f64 {
    if x.is_subnormal() {
        0.0f64.copysign(x)
    } else {
        x
    }
}

fn ftz_in32(x: f32) -> f32 {
    if x.is_subnormal() {
        0.0f32.copysign(x)
    } else {
        x
    }
}

/// Host IEEE-754 result with the DCP's flush-to-zero on the output.
fn ftz_out(r: f64) -> u64 {
    if r.is_subnormal() {
        0.0f64.copysign(r).to_bits()
    } else {
        r.to_bits()
    }
}

fn ftz_out32(r: f32) -> u32 {
    if r.is_subnormal() {
        0.0f32.copysign(r).to_bits()
    } else {
        r.to_bits()
    }
}

fn same(got: u64, want: u64) -> bool {
    let (g, w) = (f64::from_bits(got), f64::from_bits(want));
    got == want || (g.is_nan() && w.is_nan())
}

fn specials() -> Vec<f64> {
    vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.1,
        1.0 / 3.0,
        -2.5,
        3.5,
        std::f64::consts::PI,
        1e308,
        -1e308,
        f64::MIN_POSITIVE,
        f64::from_bits(1),                     // smallest denormal
        f64::from_bits(0x000F_FFFF_FFFF_FFFF), // largest denormal
        f64::MAX,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        9007199254740993.0, // 2^53 + 1 (rounds)
        4503599627370496.5,
        1e-300,
        2147483647.5,
        -2147483648.7,
        4294967295.9,
    ]
}

/// Deterministic random doubles over the whole bit space (xorshift64*).
fn randoms(n: usize, seed: u64) -> Vec<f64> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            f64::from_bits(s.wrapping_mul(0x2545_F491_4F6C_DD1D))
        })
        .collect()
}

#[test]
fn golden_bit_patterns_from_the_sdk_sequences() {
    let mut r = load(true);
    assert_eq!(r.d2("t_dadd", 0.1, 0.2), 0x3FD3_3333_3333_3334, "0.1 + 0.2");
    assert_eq!(r.d2("t_dsub", 1.0, 0.1), 0x3FEC_CCCC_CCCC_CCCD, "1 - 0.1");
    assert_eq!(r.d2("t_dmul", 0.1, 3.0), 0x3FD3_3333_3333_3334, "0.1 * 3");
    assert_eq!(r.d2("t_ddiv", 1.0, 3.0), 0x3FD5_5555_5555_5555, "1 / 3");
    assert_eq!(r.d2("t_ddiv", 2.0, 3.0), 0x3FE5_5555_5555_5555, "2 / 3");
    assert_eq!(r.d1("t_dsqrt", 2.0), 0x3FF6_A09E_667F_3BCD, "sqrt 2");
    assert_eq!(
        r.d2("t_dmul", 1e308, 10.0),
        0x7FF0_0000_0000_0000,
        "overflow to +inf"
    );
    assert_eq!(
        r.d2("t_dadd", f64::from_bits(1), 0.0),
        0,
        "denormal input flushes"
    );
    assert_eq!(
        r.d2("t_dmul", f64::MIN_POSITIVE, 0.5),
        0,
        "denormal output flushes"
    );
    assert!(f64::from_bits(r.d2("t_dsub", f64::INFINITY, f64::INFINITY)).is_nan());
    let (lo, hi) = r.call("t_i2d", [(-2147483648i32) as u32, 0, 0, 0]);
    assert_eq!(
        u64::from(hi) << 32 | u64::from(lo),
        (-2147483648.0f64).to_bits()
    );
    let (lo, hi) = r.call("t_ui2d", [u32::MAX, 0, 0, 0]);
    assert_eq!(
        u64::from(hi) << 32 | u64::from(lo),
        4294967295.0f64.to_bits()
    );
    let b = (-2.9f64).to_bits();
    assert_eq!(
        r.call("t_d2iz", [b as u32, (b >> 32) as u32, 0, 0]).0 as i32,
        -2
    );
    let b = 2.5f64.to_bits();
    assert_eq!(
        r.call("t_d2i_r", [b as u32, (b >> 32) as u32, 0, 0]).0 as i32,
        2
    );
    let b = 1e10f64.to_bits();
    assert_eq!(
        r.call("t_d2iz", [b as u32, (b >> 32) as u32, 0, 0]).0 as i32,
        i32::MAX
    );
    let b = 0.1f64.to_bits();
    assert_eq!(
        r.call("t_d2f", [b as u32, (b >> 32) as u32, 0, 0]).0,
        0x3DCC_CCCD,
        "(float)0.1"
    );
    let (lo, hi) = r.call("t_f2d", [0x3DCC_CCCD, 0, 0, 0]);
    assert_eq!(
        u64::from(hi) << 32 | u64::from(lo),
        f64::from(0.1f32).to_bits()
    );
    assert_eq!(
        r.call("t_fadd", [1.0f32.to_bits(), 2.0f32.to_bits(), 0, 0])
            .0,
        3.0f32.to_bits()
    );
    assert_eq!(
        r.call("t_fdiv", [1.0f32.to_bits(), 3.0f32.to_bits(), 0, 0])
            .0,
        0x3EAA_AAAB
    );
    assert_eq!(
        r.call("t_fsqrt", [2.0f32.to_bits(), 0, 0, 0]).0,
        0x3FB5_04F3
    );
}

#[test]
fn double_arithmetic_sweep_matches_ieee_with_flush_to_zero() {
    let mut r = load(true);
    let mut xs = specials();
    xs.extend(randoms(60, 0x9E37_79B9_7F4A_7C15));
    let mut ys = specials();
    ys.extend(randoms(60, 0xD1B5_4A32_D192_ED03));
    let mut n = 0;
    for &x in &xs {
        for &y in &ys {
            let (a, b) = (ftz_in(x), ftz_in(y));
            for (f, want) in [
                ("t_dadd", a + b),
                ("t_dsub", a - b),
                ("t_dmul", a * b),
                ("t_ddiv", a / b),
                ("t_ddiv_fast", a / b),
            ] {
                let got = r.d2(f, x, y);
                assert!(
                    same(got, ftz_out(want)),
                    "{f}({x:e} [{:#018x}], {y:e} [{:#018x}]) = {got:#018x}, want {:#018x}",
                    x.to_bits(),
                    y.to_bits(),
                    ftz_out(want)
                );
                n += 1;
            }
        }
        for f in ["t_dsqrt", "t_dsqrt_fast"] {
            let got = r.d1(f, x.abs());
            assert!(same(got, ftz_out(ftz_in(x.abs()).sqrt())), "{f}({x:e})");
        }
    }
    assert!(n > 5000, "sweep ran {n} cases");
}

#[test]
fn compares_conversions_and_floats_match_ieee() {
    let mut r = load(true);
    let mut xs = specials();
    xs.extend(randoms(40, 0x1234_5678_9ABC_DEF1));
    for &x in &xs {
        let xb = x.to_bits();
        let (xl, xh) = (xb as u32, (xb >> 32) as u32);
        for &y in &xs {
            let yb = y.to_bits();
            let args = [xl, xh, yb as u32, (yb >> 32) as u32];
            let (a, b) = (ftz_in(x), ftz_in(y));
            let flags = r.call("t_dcmp_flags", args).0 >> 28;
            let (z, c, v) = (flags >> 2 & 1, flags >> 1 & 1, flags & 1);
            if a.is_nan() || b.is_nan() {
                assert_eq!((z, c, v), (0, 0, 1), "cdcmple({x:e},{y:e}) unordered");
            } else {
                assert_eq!(
                    (z, c, v),
                    (u32::from(a == b), u32::from(a >= b), 0),
                    "cdcmple({x:e},{y:e})"
                );
            }
            assert_eq!(
                r.call("t_dcmpeq", args).0,
                u32::from(a == b),
                "dcmpeq({x:e},{y:e})"
            );
            assert_eq!(
                r.call("t_dcmpun", args).0,
                u32::from(a.is_nan() || b.is_nan()),
                "dcmpun({x:e},{y:e})"
            );
        }
        // double -> int (truncate, round-to-even), -> uint, -> float.
        let sat = |v: f64| v.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
        let a = ftz_in(x);
        let want_z = if a.is_nan() { 0 } else { sat(a.trunc()) };
        let want_r = if a.is_nan() {
            0
        } else {
            sat(a.round_ties_even())
        };
        let want_u = if a.is_nan() {
            0
        } else {
            a.trunc().clamp(0.0, f64::from(u32::MAX)) as u32
        };
        assert_eq!(
            r.call("t_d2iz", [xl, xh, 0, 0]).0 as i32,
            want_z,
            "d2iz({x:e})"
        );
        assert_eq!(
            r.call("t_d2i_r", [xl, xh, 0, 0]).0 as i32,
            want_r,
            "d2i_r({x:e})"
        );
        assert_eq!(r.call("t_d2uiz", [xl, xh, 0, 0]).0, want_u, "d2uiz({x:e})");
        let got = r.call("t_d2f", [xl, xh, 0, 0]).0;
        let want = ftz_out32(a as f32);
        assert!(
            got == want || (f32::from_bits(got).is_nan() && f32::from_bits(want).is_nan()),
            "d2f({x:e}) = {got:#x}, want {want:#x}"
        );
    }
    // int -> double / float, exact or round-to-nearest-even.
    for i in [
        0i32,
        1,
        -1,
        7,
        i32::MAX,
        i32::MIN,
        16_777_217,
        -16_777_219,
        123_456_789,
    ] {
        let (lo, hi) = r.call("t_i2d", [i as u32, 0, 0, 0]);
        assert_eq!(
            u64::from(hi) << 32 | u64::from(lo),
            f64::from(i).to_bits(),
            "i2d({i})"
        );
        assert_eq!(
            r.call("t_i2f", [i as u32, 0, 0, 0]).0,
            (i as f32).to_bits(),
            "i2f({i})"
        );
        let (lo, hi) = r.call("t_ui2d", [i as u32, 0, 0, 0]);
        assert_eq!(
            u64::from(hi) << 32 | u64::from(lo),
            f64::from(i as u32).to_bits()
        );
    }
    // Single precision through WXYU: add, mul, div, sqrt, f2d, f2iz.
    let fs: Vec<f32> = randoms(50, 0x0BAD_C0DE_1234_5678)
        .into_iter()
        .map(|d| f32::from_bits((d.to_bits() >> 32) as u32))
        .chain([
            0.0,
            -0.0,
            1.0,
            0.1,
            3.0,
            f32::MIN_POSITIVE,
            f32::from_bits(1),
            f32::INFINITY,
            f32::NAN,
        ])
        .collect();
    let eq32 =
        |g: u32, w: u32| g == w || (f32::from_bits(g).is_nan() && f32::from_bits(w).is_nan());
    for &x in &fs {
        for &y in &fs {
            let args = [x.to_bits(), y.to_bits(), 0, 0];
            let (a, b) = (ftz_in32(x), ftz_in32(y));
            for (f, want) in [("t_fadd", a + b), ("t_fmul", a * b), ("t_fdiv", a / b)] {
                let got = r.call(f, args).0;
                assert!(eq32(got, ftz_out32(want)), "{f}({x:e},{y:e}) = {got:#x}");
            }
        }
        let a = ftz_in32(x);
        assert!(eq32(
            r.call("t_fsqrt", [x.abs().to_bits(), 0, 0, 0]).0,
            ftz_out32(a.abs().sqrt())
        ));
        let (lo, hi) = r.call("t_f2d", [x.to_bits(), 0, 0, 0]);
        assert!(
            same(u64::from(hi) << 32 | u64::from(lo), f64::from(a).to_bits()),
            "f2d({x:e})"
        );
        let want = if a.is_nan() {
            0
        } else {
            (a.trunc() as f64).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
        };
        assert_eq!(
            r.call("t_f2iz", [x.to_bits(), 0, 0, 0]).0 as i32,
            want,
            "f2iz({x:e})"
        );
    }
}

/// The SDK's saving_func wrapper: an operation interrupted mid-sequence
/// (engaged) survives a nested `__aeabi_dadd`, which PCMP detects, saves the
/// six state words around and restores.
#[test]
fn saving_wrapper_preserves_an_engaged_sequence() {
    let mut r = load(true);
    let (a, b) = (10.0f64.to_bits(), 4.0f64.to_bits());
    r.call(
        "t_engage_dsub",
        [a as u32, (a >> 32) as u32, b as u32, (b >> 32) as u32],
    );
    assert_eq!(
        r.d2("t_dadd_saving", 0.1, 0.2),
        0x3FD3_3333_3333_3334,
        "nested dadd"
    );
    let (lo, hi) = r.call("t_finish_dsub", [0; 4]);
    assert_eq!(
        f64::from_bits(u64::from(hi) << 32 | u64::from(lo)),
        6.0,
        "resumed 10 - 4"
    );
    // Without the save (plain t_dadd) the interrupted operands are lost.
    r.call(
        "t_engage_dsub",
        [a as u32, (a >> 32) as u32, b as u32, (b >> 32) as u32],
    );
    r.d2("t_dadd", 0.1, 0.2);
    let (lo, hi) = r.call("t_finish_dsub", [0; 4]);
    assert_ne!(f64::from_bits(u64::from(hi) << 32 | u64::from(lo)), 6.0);
}

/// A chip that declares no DCP keeps p4 UNDEFINED (every other Cortex-M).
#[test]
fn without_the_dcp_declared_p4_is_undefined() {
    let mut r = load(false);
    let pc = labwired_loader::resolve_symbol_in_elf(&r.elf, "t_dadd").unwrap();
    let m = &mut r.m;
    m.cpu.set_pc(pc & !1);
    m.cpu.set_sp(STACK);
    m.cpu.lr = SENTINEL | 1;
    let _ = m.step();
    let cfsr = m.bus.read_u32(0xE000_ED28).unwrap_or(0);
    assert_ne!(cfsr & (1 << 16), 0, "UNDEFINSTR expected, CFSR {cfsr:#x}");
}
