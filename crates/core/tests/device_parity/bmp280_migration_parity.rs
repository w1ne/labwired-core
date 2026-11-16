// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! BMP280: the declarative descriptor against the hand-written model it
//! replaces.
//!
//! `components/bmp280.rs` survives as the byte-parity ORACLE (see the note
//! below for why it is not deleted), and both models are driven through the
//! SAME I²C script. Transcripts that must be identical are asserted equal byte
//! for byte; the one that must differ is asserted as a difference, by name.
//!
//! ⚠️ **The data registers are no longer a parity claim.** The hand model
//! answered the constant `0x80000` for both raw words and had no stimulus. Since
//! #3356 the descriptor inverts Bosch's compensation at its `temperature` and
//! `pressure` inputs, so 0xF7..0xFC are asserted against an independent Rust
//! copy of the datasheet's compensation code instead (section "stimulus"
//! below), computed from the calibration bytes the device itself puts on the
//! bus. Everything else is still held byte-identical to the oracle.
//!
//! Held identical:
//!   * the whole 26-byte calibration read (0x88..0xA1), including the two
//!     reserved bytes that read 0x00 where an undeclared address reads 0xFF —
//!     compared against the exact `CALIB` array the deleted model carried;
//!   * CHIP_ID = 0x58;
//!   * STATUS / CTRL_MEAS / CONFIG, their read-back, and the 0xB6 soft reset
//!     clearing all three (and NO other value doing so);
//!   * every unmodelled address reading 0xFF, and the pointer wrapping
//!     0xFF → 0x00;
//!   * writes to the calibration block and the chip id being dropped (and,
//!     on the descriptor alone, writes to the data registers).
//!
//! Deliberately DIFFERENT (its own test below):
//!   * the RESET register (0xE0) still reads 0xFF after a write, on BOTH
//!     models — but for different reasons, and the descriptor says so out
//!     loud through `write_mask: 0x00` instead of by having no read arm. The
//!     observable difference is what a write of a NON-reset value does to the
//!     three writable registers: nothing, on both. Asserted so the `when:`
//!     guard is proved rather than assumed.
//!
//! Scripts are driven through the shared harness (`tests/common/transcript.rs`),
//! the same one the byte-parity ratchet uses.

use crate::common;
use common::transcript::{read_reg, run_i2c, script, write_reg, Step};
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::sim_input::SimInput;

const ADDR: u8 = 0x76;

// ─── the model this descriptor is proven identical to ──────────────────────
//
// ⚠️ Unlike the other migration-parity tests in this directory, the oracle is
// NOT a copy pasted in here: `components/bmp280.rs` still exists. It stays
// because the ESP32 and ESP32-C3 I²C controller tests attach a register-pointer
// slave and any one will do, and `crates/core/src/peripherals/esp32c3/` is
// covered by a silicon drift-ack digest (`validation/boards/`) that an
// edit to a test module inside it would invalidate — a red `--check --drift`
// gate for a change that touches no model. The struct is excluded from the
// declarative-coverage ratchet with that reason, exactly as `veml7700.rs` and
// `pca9685.rs` are.
//
// Reading the real module rather than a copy is strictly better here: the two
// cannot drift apart, and a change to the oracle that the descriptor does not
// match fails below rather than silently becoming the new truth.

use labwired_core::peripherals::components::bmp280::{Bmp280, ADC_P, ADC_T, CALIB, CHIP_ID};

// ─── the two models under test ─────────────────────────────────────────────

fn declarative() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("bmp280")
        .expect("bmp280 descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("bmp280.yaml does not build")
}

/// Run one script against BOTH models and return `(legacy, declarative)`.
///
/// ⚠️ The same script object, not two scripts that look alike: a divergence
/// that lived in the script rather than in the models would compare equal and
/// prove nothing.
fn both(steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = Bmp280::new(ADDR);
    let mut new = declarative();
    (
        run_i2c(&mut old, steps).bytes,
        run_i2c(&mut new, steps).bytes,
    )
}

// ─── identical: the calibration block ──────────────────────────────────────

#[test]
fn the_calibration_block_is_identical_and_is_the_bosch_reference() {
    // Every BMP280 library slurps 0x88..0xA1 in one burst. 26 bytes: the 24
    // calibration bytes plus the two reserved ones.
    let (old, new) = both(&read_reg(0x88, 26));
    assert_eq!(old, new, "calibration transcript diverged");
    assert_eq!(&old[..24], &CALIB, "the 24 documented bytes");
    assert_eq!(
        &old[24..],
        &[0x00, 0x00],
        "0xA0/0xA1 are reserved and read 0x00, NOT the 0xFF an undeclared \
         address gives — a driver reading the block in one burst sees this"
    );

    // The block decodes to the Bosch reference coefficients, little-endian.
    let w = |i: usize| u16::from_le_bytes([old[i], old[i + 1]]);
    assert_eq!(w(0), 27504, "dig_T1");
    assert_eq!(w(2) as i16, 26435, "dig_T2");
    assert_eq!(w(4) as i16, -1000, "dig_T3");
    assert_eq!(w(6), 36477, "dig_P1");
    // ⚠️ The deleted model's own comment on this pair said `-10685`. The BYTES
    // are 0xA3 0xD5, which is 0xD5A3 = -10845. The comment was wrong and the
    // bytes are what shipped, so the descriptor carries the bytes and this
    // assertion carries the number they actually are.
    assert_eq!(w(8) as i16, -10845, "dig_P2");
    assert_eq!(w(22) as i16, 6000, "dig_P9");
}

// ─── identical: identity and the raw data words ────────────────────────────

#[test]
fn chip_id_is_identical() {
    let (old, new) = both(&read_reg(0xD0, 1));
    assert_eq!(old, new);
    assert_eq!(
        old,
        vec![CHIP_ID],
        "every library refuses to begin() otherwise"
    );
    assert_eq!(CHIP_ID, 0x58);
}

// ─── stimulus: the data block is Bosch's compensation, inverted ────────────

/// The 20-bit word a driver reassembles from three left-justified bytes.
fn word(b: &[u8]) -> i32 {
    (i32::from(b[0]) << 12) | (i32::from(b[1]) << 4) | (i32::from(b[2]) >> 4)
}

/// Bosch BMP280 datasheet (BST-BMP280-DS001) §8.2: `bmp280_compensate_T_int32`
/// and `bmp280_compensate_P_int64`, transcribed independently of the YAML and
/// fed the calibration bytes READ OFF THE BUS. Returns (°C × 100, Pa × 256).
fn bosch_compensate(cal: &[u8], adc_t: i32, adc_p: i32) -> (i32, u32) {
    let u = |i: usize| i32::from(u16::from_le_bytes([cal[i], cal[i + 1]]));
    let s = |i: usize| i32::from(i16::from_le_bytes([cal[i], cal[i + 1]]));
    let (t1, t2, t3) = (u(0), s(2), s(4));
    let (p1, p2, p3, p4, p5, p6, p7, p8, p9) =
        (u(6), s(8), s(10), s(12), s(14), s(16), s(18), s(20), s(22));

    let var1 = (((adc_t >> 3) - (t1 << 1)) * t2) >> 11;
    let var2 = (((((adc_t >> 4) - t1) * ((adc_t >> 4) - t1)) >> 12) * t3) >> 14;
    let t_fine = var1 + var2;
    let t = (t_fine * 5 + 128) >> 8;

    let mut var1 = i64::from(t_fine) - 128000;
    let mut var2 = var1 * var1 * i64::from(p6);
    var2 += (var1 * i64::from(p5)) << 17;
    var2 += i64::from(p4) << 35;
    var1 = ((var1 * var1 * i64::from(p3)) >> 8) + ((var1 * i64::from(p2)) << 12);
    var1 = (((1i64 << 47) + var1) * i64::from(p1)) >> 33;
    assert_ne!(var1, 0, "division by zero guard of the reference code");
    let mut p: i64 = 1048576 - i64::from(adc_p);
    p = (((p << 31) - var2) * 3125) / var1;
    let var1 = (i64::from(p9) * (p >> 13) * (p >> 13)) >> 25;
    let var2 = (i64::from(p8) * p) >> 19;
    p = ((p + var1 + var2) >> 8) + (i64::from(p7) << 4);
    (t, p as u32)
}

/// What firmware running Bosch's code reads: (°C, hPa).
fn firmware_reading(dev: &mut GenericI2cDevice) -> (f64, f64) {
    let cal = run_i2c(dev, &read_reg(0x88, 24)).bytes;
    let data = run_i2c(dev, &read_reg(0xF7, 6)).bytes;
    assert_eq!(data[2] & 0x0F, 0, "pressure xlsb low nibble is unused");
    assert_eq!(data[5] & 0x0F, 0, "temperature xlsb low nibble is unused");
    let (t, p) = bosch_compensate(&cal, word(&data[3..]), word(&data[..3]));
    (f64::from(t) / 100.0, f64::from(p) / 256.0 / 100.0)
}

#[test]
fn the_default_reading_is_25_c_and_1013_25_hpa_through_the_bosch_code() {
    let mut dev = declarative();
    let (t, p) = firmware_reading(&mut dev);
    assert!((t - 25.0).abs() < 0.005, "temperature {t} °C");
    assert!((p - 1013.25).abs() < 0.001, "pressure {p} hPa");
}

#[test]
fn driven_pressure_and_temperature_read_back_through_the_registers() {
    let mut dev = declarative();
    // A launch-to-apogee sweep: sea level, ~1 km, ~3 km, ~9 km, plus the
    // datasheet operating corners, at temperatures across the range.
    for (t_in, p_in) in [
        (25.0, 1013.25),
        (18.5, 898.76),
        (5.0, 701.12),
        (-40.0, 300.0),
        (85.0, 1100.0),
        (-12.34, 456.78),
    ] {
        dev.set_input("temperature", t_in).unwrap();
        dev.set_input("pressure", p_in).unwrap();
        let (t, p) = firmware_reading(&mut dev);
        assert!(
            (t - t_in).abs() < 0.005,
            "set {t_in} °C, firmware reads {t} °C"
        );
        // Q24.8 Pa: the compensation resolves 1/256 Pa; one ADC count is
        // ~0.16 Pa at these coefficients, so the inverse lands within 0.2 Pa.
        assert!(
            (p - p_in).abs() * 100.0 < 0.2,
            "set {p_in} hPa, firmware reads {p} hPa"
        );
    }
}

#[test]
fn pressure_and_temperature_are_independent_channels() {
    // Changing pressure must move only the pressure word; changing
    // temperature moves the temperature word (and, through t_fine, the
    // pressure word too, so the reading stays at the set pressure).
    let mut dev = declarative();
    let before = run_i2c(&mut dev, &read_reg(0xF7, 6)).bytes;
    dev.set_input("pressure", 850.0).unwrap();
    let after_p = run_i2c(&mut dev, &read_reg(0xF7, 6)).bytes;
    assert_ne!(before[..3], after_p[..3], "pressure word must move");
    assert_eq!(before[3..], after_p[3..], "temperature word must not move");
    dev.set_input("temperature", 0.0).unwrap();
    let (t, p) = firmware_reading(&mut dev);
    assert!(
        (t - 0.0).abs() < 0.005 && (p - 850.0).abs() * 100.0 < 0.2,
        "{t} °C {p} hPa"
    );
}

/// NEGATIVE control: the oracle's constant words are NOT what the descriptor
/// answers any more, and they do not compensate to the default stimulus. A
/// descriptor reverted to the constant reset words fails here and above.
#[test]
fn the_data_block_is_no_longer_the_oracles_constant() {
    let mut old = Bmp280::new(ADDR);
    let mut new = declarative();
    let old_data = run_i2c(&mut old, &read_reg(0xF7, 6)).bytes;
    assert_eq!(old_data, vec![0x80, 0x00, 0x00, 0x80, 0x00, 0x00]);
    assert_eq!(word(&old_data[..3]) as u32, ADC_P);
    assert_eq!(word(&old_data[3..]) as u32, ADC_T);
    assert_ne!(run_i2c(&mut new, &read_reg(0xF7, 6)).bytes, old_data);
}

#[test]
fn nan_and_unknown_inputs_are_rejected() {
    let mut dev = declarative();
    assert!(dev.set_input("pressure", f64::NAN).is_err());
    assert!(
        dev.set_input("humidity", 50.0).is_err(),
        "a BMP280 has no humidity channel"
    );
}

/// A mid-word pointer must land inside the same data registers a burst read
/// sees: a wrong `endian:` would still produce three bytes per word.
#[test]
fn a_mid_word_pointer_lands_inside_the_data_registers() {
    let mut dev = declarative();
    dev.set_input("pressure", 912.34).unwrap();
    let block = run_i2c(&mut dev, &read_reg(0xF7, 6)).bytes;
    let s = script([read_reg(0xF8, 2), read_reg(0xFB, 2), read_reg(0xF9, 1)]);
    let mid = run_i2c(&mut dev, &s).bytes;
    assert_eq!(mid, vec![block[1], block[2], block[4], block[5], block[2]]);
}

// ─── identical: what stores, what resets ───────────────────────────────────

#[test]
fn ctrl_meas_and_config_round_trip_identically() {
    let s = script([
        write_reg(0xF4, &[0x27]), // osrs_t=1, osrs_p=1, mode=normal
        write_reg(0xF5, &[0xA0]), // t_sb=101, filter=0, spi3w=0
        read_reg(0xF3, 3),        // STATUS, CTRL_MEAS, CONFIG in one burst
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "configuration read-back diverged");
    assert_eq!(old, vec![0x00, 0x27, 0xA0], "STATUS stays 0x00");
}

#[test]
fn the_soft_reset_clears_the_three_writable_registers_identically() {
    let s = script([
        write_reg(0xF4, &[0x27]),
        write_reg(0xF5, &[0xA0]),
        write_reg(0xE0, &[0xB6]), // §5.4.2: the soft-reset magic
        read_reg(0xF3, 3),
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "soft-reset transcript diverged");
    assert_eq!(old, vec![0x00, 0x00, 0x00], "all three cleared");
}

/// The NEGATIVE control for the `when: "written == 0xB6"` guard. A rule with no
/// guard — or a guard that always held — would clear the registers on ANY write
/// to 0xE0, and the test above would still pass.
#[test]
fn a_non_magic_write_to_the_reset_register_clears_nothing_on_either_model() {
    for value in [0x00u8, 0x01, 0xB5, 0xB7, 0xFF] {
        let s = script([
            write_reg(0xF4, &[0x27]),
            write_reg(0xF5, &[0xA0]),
            write_reg(0xE0, &[value]),
            read_reg(0xF3, 3),
        ]);
        let (old, new) = both(&s);
        assert_eq!(old, new, "reset-guard transcript diverged at 0x{value:02X}");
        assert_eq!(
            old,
            vec![0x00, 0x27, 0xA0],
            "0x{value:02X} is not 0xB6 and must reset nothing"
        );
    }
}

#[test]
fn the_reset_register_reads_open_bus_on_both_models() {
    // The hand model had no read arm for 0xE0 and fell through to 0xFF; the
    // descriptor says the same thing with `write_mask: 0x00` over a 0xFF reset
    // value, so the address accepts a write, acts on it, and still answers open
    // bus. Asserted AFTER a write, which is where a register that stored the
    // byte would give itself away.
    let s = script([write_reg(0xE0, &[0xB6]), read_reg(0xE0, 1)]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "reset read-back diverged");
    assert_eq!(old, vec![0xFF]);
}

#[test]
fn read_only_registers_drop_the_write_identically() {
    let s = script([
        // The calibration block and the chip id are silicon's. A burst write
        // over each must change nothing.
        write_reg(0x88, &[0xDE, 0xAD, 0xBE, 0xEF]),
        write_reg(0xD0, &[0x00]),
        read_reg(0x88, 4),
        read_reg(0xD0, 1),
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "read-only drop transcript diverged");
    assert_eq!(
        old,
        vec![
            0x70, 0x6B, 0x43, 0x67, // dig_T1, dig_T2 — untouched
            0x58, // CHIP_ID
        ]
    );

    // The data registers are the stimulus's, not the bus master's.
    let mut dev = declarative();
    let before = run_i2c(&mut dev, &read_reg(0xF7, 6)).bytes;
    let after = run_i2c(
        &mut dev,
        &script([
            write_reg(0xF7, &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]),
            read_reg(0xF7, 6),
        ]),
    )
    .bytes;
    assert_eq!(before, after, "a write to the data block is dropped");
}

// ─── identical: the unmodelled space and the pointer wrap ──────────────────

#[test]
fn unmodelled_addresses_read_open_bus_and_the_pointer_wraps() {
    let s = script([
        read_reg(0x00, 4),
        read_reg(0x80, 8), // 0x80..0x87 — just below the calibration block
        read_reg(0xFD, 4), // 0xFD, 0xFE, 0xFF, then the wrap to 0x00
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "open-bus/wrap transcript diverged");
    assert_eq!(old, vec![0xFF; 16], "every one of them is open bus");
}

#[test]
fn the_pointer_wrap_lands_on_a_real_register_identically() {
    // 0xFF → 0x00 is a wrap, not a saturation — but every address either side
    // of it reads 0xFF, so the wrap alone proves nothing. Walk from 0xFE far
    // enough to reach the calibration block, which is the first address whose
    // answer is not open bus.
    let (old, new) = both(&read_reg(0xFE, 0x8B));
    assert_eq!(old, new, "long-walk transcript diverged");
    // 0xFE, 0xFF → open bus; then 0x00..0x87 → open bus; then dig_T1 low byte.
    assert_eq!(old.len(), 0x8B);
    assert_eq!(old[..0x8A], [0xFF; 0x8A], "everything up to 0x88");
    assert_eq!(old[0x8A], 0x70, "the pointer wrapped and reached dig_T1");
}
