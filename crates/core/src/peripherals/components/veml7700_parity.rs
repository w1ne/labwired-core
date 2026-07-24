// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **Byte-parity harness: declarative VEML7700 vs the hand-written oracle.**
//!
//! The shipping VEML7700 is now the declarative descriptor
//! `configs/devices/veml7700.yaml`, interpreted by
//! [`super::declarative_i2c::GenericI2cDevice`]. The hand-written
//! [`super::veml7700::Veml7700`] model is retained *only* as the reference this
//! module proves the declarative device byte-identical against: every test here
//! drives the OLD and NEW devices through the exact same I²C script and asserts
//! the two produce byte-equal reads.
//!
//! ## Why divide, not multiply-by-reciprocal
//!
//! The oracle converts illuminance to a raw count by **dividing**:
//! `count = round(lux / resolution)`, where
//! `resolution = 0.0576 × (100 / IT) / gain`. The tempting way to express this
//! declaratively is a list of `scale_from` factors multiplied straight onto the
//! value (a counts-per-lux of `1 / resolution`). That is **not** byte-faithful:
//! on the "nice" firmware values (13.5, 27, 40.5 … lux) the WHITE channel's
//! ×1.15 pushes the true quotient onto an exact x.5 rounding tie, where
//! `round(lux / resolution)` and `round(lux × (1/resolution))` disagree by one
//! LSB (the reciprocal lands a hair below x.5 and rounds down). A dense uniform
//! random sweep never surfaces it — the ties are measure-zero — but real
//! firmware hits them constantly.
//!
//! So the declarative descriptor uses `resolution` mode (`divide_raw`) and
//! rebuilds the oracle's exact float op-tree: because the IT ratio (100/IT ∈
//! {1, 0.5, 0.25, 0.125, 2, 4}) and inverse-gain (1/gain ∈ {1, 0.5, 8, 4}) are
//! all exact powers of two in f64, folding `0.0576 × IT_factor × gain_factor`
//! left-to-right reproduces `0.0576 × (100/IT) / gain` bit-for-bit. The
//! [`nice_value_tie_sweep`] test below is the load-bearing case: it walks the
//! very lux values where a reciprocal-multiply regression would diverge, across
//! every gain × integration-time combination, on **both** ALS and WHITE.

use super::declarative_i2c::GenericI2cDevice;
use super::veml7700::{Veml7700, VEML7700_ADDR};
use crate::peripherals::i2c::I2cDevice;
use crate::sim_input::SimInputError;

// VEML7700 register pointers (datasheet).
const REG_ALS_CONF: u8 = 0x00;
const REG_ALS_WH: u8 = 0x01;
const REG_ALS_WL: u8 = 0x02;
const REG_PSM: u8 = 0x03;
const REG_ALS: u8 = 0x04;
const REG_WHITE: u8 = 0x05;
const REG_ALS_INT: u8 = 0x06;

// ─── device construction ───────────────────────────────────────────────────

/// The hand-written oracle at its default 450 lux / power-on config.
fn oracle() -> Veml7700 {
    Veml7700::new_default(VEML7700_ADDR)
}

/// The declarative device from the embedded descriptor (default address).
fn declarative() -> GenericI2cDevice {
    let yaml =
        labwired_config::embedded_device_yaml("veml7700").expect("veml7700 descriptor is embedded");
    GenericI2cDevice::from_yaml(yaml, 0).expect("veml7700.yaml is a valid descriptor")
}

// ─── I²C script helpers (generic over &mut dyn I2cDevice) ───────────────────

/// Write a 16-bit little-endian word to `reg` (pointer, low byte, high byte),
/// framed by START … STOP exactly as a controller would.
fn write_reg(d: &mut dyn I2cDevice, reg: u8, word: u16) {
    d.start();
    d.write(reg);
    d.write((word & 0xFF) as u8);
    d.write((word >> 8) as u8);
    d.stop();
}

/// Point at `reg`, repeated-START into the read phase, and read `n` bytes.
fn read_reg_n(d: &mut dyn I2cDevice, reg: u8, n: usize) -> Vec<u8> {
    d.start();
    d.write(reg);
    d.start(); // repeated START into the read phase
    (0..n).map(|_| d.read()).collect()
}

/// Read a register's 2-byte word (the common case).
fn read_reg(d: &mut dyn I2cDevice, reg: u8) -> Vec<u8> {
    read_reg_n(d, reg, 2)
}

/// Drive the `lux` input channel; returns the same Result both devices give.
fn set_lux(d: &mut dyn I2cDevice, lux: f64) -> Result<(), SimInputError> {
    d.as_sim_input_mut()
        .expect("veml7700 exposes a SimInput")
        .set_input("lux", lux)
}

/// Assert the oracle and the declarative device return byte-identical reads of
/// `reg` for the current state. Returns the (shared) bytes for further checks.
#[track_caller]
fn assert_reg_equal(o: &mut Veml7700, g: &mut GenericI2cDevice, reg: u8, ctx: &str) -> Vec<u8> {
    let ob = read_reg(o, reg);
    let gb = read_reg(g, reg);
    assert_eq!(
        ob, gb,
        "reg 0x{reg:02x} diverged ({ctx}): oracle={ob:02x?} declarative={gb:02x?}"
    );
    ob
}

// The ALS_CONF bit layout: gain in [12:11], integration time in [6:9] (4 bits).
fn als_conf(gain_field: u16, it_field: u16) -> u16 {
    ((gain_field & 0x3) << 11) | ((it_field & 0xF) << 6)
}

/// Every integration-time field the oracle distinguishes, plus a spread of the
/// reserved encodings (which must both fall through to the 100 ms / ×1 default).
const IT_FIELDS: &[u16] = &[
    0x0, 0x1, 0x2, 0x3, 0x8, 0xC, // distinguished: 100/200/400/800/50/25 ms
    0x4, 0x5, 0x6, 0x7, 0x9, 0xA, 0xB, 0xD, 0xE, 0xF, // reserved → default
];
/// All four gain fields (the field is 2 bits, so every value is defined).
const GAIN_FIELDS: &[u16] = &[0x0, 0x1, 0x2, 0x3];

// ─── power-on defaults ──────────────────────────────────────────────────────

#[test]
fn power_on_defaults_match_across_every_register() {
    let mut o = oracle();
    let mut g = declarative();
    assert_eq!(o.address(), g.address(), "default address");
    for reg in [
        REG_ALS_CONF,
        REG_ALS_WH,
        REG_ALS_WL,
        REG_PSM,
        REG_ALS,
        REG_WHITE,
        REG_ALS_INT,
    ] {
        assert_reg_equal(&mut o, &mut g, reg, "power-on");
    }
}

#[test]
fn als_int_reads_zero_and_unknown_register_reads_zero() {
    let mut o = oracle();
    let mut g = declarative();
    assert_eq!(read_reg(&mut o, REG_ALS_INT), vec![0, 0]);
    assert_eq!(read_reg(&mut g, REG_ALS_INT), vec![0, 0]);
    // An unmapped pointer reads a zero word on both.
    assert_reg_equal(&mut o, &mut g, 0x7E, "unknown register");
}

// ─── rw storage registers round-trip identically ───────────────────────────

#[test]
fn threshold_and_psm_registers_round_trip_identically() {
    let mut o = oracle();
    let mut g = declarative();
    for (reg, word) in [
        (REG_ALS_WH, 0xBEEFu16),
        (REG_ALS_WL, 0x1234),
        (REG_PSM, 0x0007),
        (REG_ALS_CONF, 0x1846),
    ] {
        write_reg(&mut o, reg, word);
        write_reg(&mut g, reg, word);
        let bytes = assert_reg_equal(&mut o, &mut g, reg, "rw round-trip");
        let read_back = (bytes[0] as u16) | ((bytes[1] as u16) << 8);
        assert_eq!(read_back, word, "reg 0x{reg:02x} stored its written word");
    }
}

// ─── the full gain × IT resolution matrix ───────────────────────────────────

/// For every gain × integration-time combination, drive a spread of lux values
/// and assert ALS + WHITE read byte-identically. This walks the whole
/// resolution table, including the reserved IT encodings.
#[test]
fn full_gain_it_matrix_matches_on_als_and_white() {
    // A spread that spans the count range without being a random sweep: small
    // values, the default, and values large enough to clamp at high gain / long
    // integration time.
    let lux_points: &[f64] = &[
        0.0, 0.01, 0.0288, 0.0576, 1.0, 13.5, 27.0, 40.5, 100.0, 450.0, 1000.0, 12000.0, 60000.0,
        120000.0,
    ];
    for &gain in GAIN_FIELDS {
        for &it in IT_FIELDS {
            let conf = als_conf(gain, it);
            let mut o = oracle();
            let mut g = declarative();
            write_reg(&mut o, REG_ALS_CONF, conf);
            write_reg(&mut g, REG_ALS_CONF, conf);
            for &lux in lux_points {
                set_lux(&mut o, lux).unwrap();
                set_lux(&mut g, lux).unwrap();
                let ctx = format!("gain={gain:#x} it={it:#x} lux={lux}");
                assert_reg_equal(&mut o, &mut g, REG_ALS, &ctx);
                assert_reg_equal(&mut o, &mut g, REG_WHITE, &ctx);
            }
        }
    }
}

// ─── the load-bearing nice-value tie sweep ──────────────────────────────────

/// The case that exposes a reciprocal-multiply regression. Sweeps a dense grid
/// of "nice" firmware lux values — the exact family (13.5, 27, 40.5, …) where
/// the WHITE channel's ×1.15 lands the quotient on an x.5 rounding tie — across
/// every gain × IT combination, on both ALS and WHITE, and asserts the
/// declarative device is byte-identical to the divide-based oracle. If the
/// descriptor ever regresses to multiplying by a `1/resolution` counts-per-lux,
/// these ties diverge by one LSB and this test fails.
#[test]
fn nice_value_tie_sweep() {
    // 0.0 … 5000.0 in steps of 0.5 — the "nice" decimals firmware actually
    // reports, dense enough that the ×1.15 WHITE path repeatedly hits x.5 ties.
    let nice_luxes: Vec<f64> = (0..=10_000).map(|k| k as f64 * 0.5).collect();
    let mut mismatches = 0usize;
    for &gain in GAIN_FIELDS {
        for &it in &[0x0u16, 0x1, 0x2, 0x3, 0x8, 0xC] {
            let conf = als_conf(gain, it);
            let mut o = oracle();
            let mut g = declarative();
            write_reg(&mut o, REG_ALS_CONF, conf);
            write_reg(&mut g, REG_ALS_CONF, conf);
            for &lux in &nice_luxes {
                set_lux(&mut o, lux).unwrap();
                set_lux(&mut g, lux).unwrap();
                for reg in [REG_ALS, REG_WHITE] {
                    if read_reg(&mut o, reg) != read_reg(&mut g, reg) {
                        mismatches += 1;
                    }
                }
            }
        }
    }
    assert_eq!(
        mismatches, 0,
        "declarative VEML7700 diverged from the divide-based oracle on {mismatches} nice-value ties"
    );
}

// ─── set_input validation parity ────────────────────────────────────────────

#[test]
fn set_input_range_rejection_matches() {
    let mut o = oracle();
    let mut g = declarative();
    // Below range and above the 120 000 lx maximum are rejected on both.
    assert!(set_lux(&mut o, -1.0).is_err());
    assert!(set_lux(&mut g, -1.0).is_err());
    assert!(set_lux(&mut o, 120_000.1).is_err());
    assert!(set_lux(&mut g, 120_000.1).is_err());
    // The exact bounds are accepted on both.
    assert!(set_lux(&mut o, 0.0).is_ok());
    assert!(set_lux(&mut g, 0.0).is_ok());
    assert!(set_lux(&mut o, 120_000.0).is_ok());
    assert!(set_lux(&mut g, 120_000.0).is_ok());
}

#[test]
fn unknown_input_channel_is_rejected_on_both() {
    let mut o = oracle();
    let mut g = declarative();
    assert!(o
        .as_sim_input_mut()
        .unwrap()
        .set_input("brightness", 10.0)
        .is_err());
    assert!(g
        .as_sim_input_mut()
        .unwrap()
        .set_input("brightness", 10.0)
        .is_err());
}

// ─── transaction-framing edge cases ─────────────────────────────────────────

#[test]
fn over_read_past_the_word_yields_ff_on_both() {
    let mut o = oracle();
    let mut g = declarative();
    // A third byte past the 16-bit word reads 0xFF on both.
    let ob = read_reg_n(&mut o, REG_ALS, 3);
    let gb = read_reg_n(&mut g, REG_ALS, 3);
    assert_eq!(ob, gb, "over-read: oracle={ob:02x?} declarative={gb:02x?}");
    assert_eq!(ob[2], 0xFF, "3rd byte is a not-present marker");
}

#[test]
fn incomplete_config_write_does_not_store_on_either() {
    let mut o = oracle();
    let mut g = declarative();
    // First a real config write so a stored value exists to (not) overwrite.
    write_reg(&mut o, REG_ALS_CONF, 0x0846);
    write_reg(&mut g, REG_ALS_CONF, 0x0846);
    // Now a partial write: pointer + a single data byte, no high byte.
    o.start();
    o.write(REG_ALS_CONF);
    o.write(0xFF);
    o.stop();
    g.start();
    g.write(REG_ALS_CONF);
    g.write(0xFF);
    g.stop();
    // Neither device commits the incomplete word.
    let bytes = assert_reg_equal(&mut o, &mut g, REG_ALS_CONF, "after partial write");
    assert_eq!(
        (bytes[0] as u16) | ((bytes[1] as u16) << 8),
        0x0846,
        "partial write left the prior value intact"
    );
}

#[test]
fn repeated_reads_are_stable_and_identical() {
    // Reading the same register many times without re-driving lux returns the
    // same bytes on both devices (no self-running scene).
    let mut o = oracle();
    let mut g = declarative();
    set_lux(&mut o, 333.0).unwrap();
    set_lux(&mut g, 333.0).unwrap();
    let first = read_reg(&mut o, REG_ALS);
    for _ in 0..16 {
        assert_eq!(read_reg(&mut o, REG_ALS), first, "oracle stable");
        assert_eq!(
            read_reg(&mut g, REG_ALS),
            first,
            "declarative matches + stable"
        );
    }
}
