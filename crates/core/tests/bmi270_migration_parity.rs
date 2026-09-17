// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **BMI270, against the hand-written model it replaces.**
//!
//! `components/bmi270.rs` (778 lines) is DELETED. What was in it: a register
//! pointer, an auto-incrementing burst, a `match` of forty addresses, two
//! range-to-LSB conversions, a paged sixteen-byte window and a power-on-reset
//! routine. Every one of those is a key the declarative engine already had —
//! except the streaming port the config upload needs, which this port adds as
//! `stream:` (see `RegisterSpec::stream`).
//!
//! [`BMI270_GOLDEN`] is the transcript the model produced for
//! [`migration_script`], captured by running it against the model on the commit
//! that removed it. The script is not a spot check: it walks the config-load
//! handshake both ways round, both range fields across five settings each, the
//! paged step counter on the right page and a wrong one, and a soft reset
//! followed by a re-read of everything the reset was supposed to restore.
//!
//! ⚠️ **There is no FIFO, and there never was.** The model answered
//! `CMD_FIFO_FLUSH` with a comment reading `no FIFO modelled`; `FIFO_DATA`
//! (0x24) is undeclared here and reads 0, and `fifo_flush` is accepted and does
//! nothing. Neither are the INT1/INT2 pads, SENSORTIME, the output data rates
//! in ACC_CONF/GYR_CONF, or any feature-engine output but the step counter.
//! All of that is listed in `configs/devices/bmi270.yaml` rather than faked.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;

mod common;
use common::transcript::{read_reg, run_i2c, script, write_reg, Step};

const ADDR: u8 = 0x68;

fn dev() -> GenericI2cDevice {
    let yaml =
        labwired_config::embedded_device_yaml("bmi270").expect("bmi270 descriptor is not embedded");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("bmi270.yaml does not build")
}

/// The conversation the deleted `Bmi270` was driven through.
fn migration_script() -> Vec<Step<'static>> {
    script([
        read_reg(0x00, 1), // CHIP_ID
        read_reg(0x02, 2), // ERR, STATUS at reset
        read_reg(0x21, 1), // INTERNAL_STATUS: not_init
        read_reg(0x40, 4), // ACC_CONF, ACC_RANGE, GYR_CONF, GYR_RANGE
        read_reg(0x7C, 2), // PWR_CONF, PWR_CTRL
        // The config-load handshake, the WRONG way round first: INIT_CTRL=1
        // with no blob must leave not_init.
        write_reg(0x59, &[0x01]),
        read_reg(0x21, 1),
        // Now the real sequence: stream a (tiny) blob, then close the gate.
        write_reg(0x5E, &[0x00, 0x11, 0x22, 0x33, 0xB6, 0x44]),
        write_reg(0x59, &[0x01]),
        read_reg(0x21, 1),
        // Enable both sensors; STATUS grows its two data-ready bits.
        write_reg(0x7D, &[0x06]),
        read_reg(0x03, 1),
        write_reg(0x7D, &[0x04]),
        read_reg(0x03, 1),
        // Motion at the reset ranges (±8 g, ±2000 dps).
        vec![
            Step::Input("ax", 1.0),
            Step::Input("ay", -0.5),
            Step::Input("az", 0.25),
            Step::Input("gx", 250.0),
            Step::Input("gy", -1000.0),
            Step::Input("gz", 2000.0),
            Step::Input("temp", 31.5),
        ],
        read_reg(0x0C, 12), // the six axes in one burst
        read_reg(0x18, 3),  // SENSORTIME
        read_reg(0x22, 2),  // TEMPERATURE
        // Range changes, then fresh stimulus at each.
        write_reg(0x41, &[0x00]), // ±2 g
        vec![Step::Input("ax", 1.0), Step::Input("ay", 2.0)],
        read_reg(0x0C, 4),
        write_reg(0x43, &[0x04]), // ±125 dps
        vec![Step::Input("gx", 125.0), Step::Input("gy", -60.0)],
        read_reg(0x12, 4),
        write_reg(0x43, &[0x07]), // ±15 dps
        vec![Step::Input("gz", 7.5)],
        read_reg(0x16, 2),
        // The step counter through the paged FEATURES window.
        vec![Step::Input("steps", 12345.0)],
        write_reg(0x2F, &[0x06]),
        read_reg(0x30, 6), // four count bytes plus two the window does not hold
        write_reg(0x2F, &[0x00]),
        read_reg(0x30, 4), // the wrong page reads zeros
        read_reg(0x2F, 1),
        // Soft reset returns every one of those to power-on.
        write_reg(0x7E, &[0xB6]),
        read_reg(0x7E, 1),
        read_reg(0x21, 1),
        read_reg(0x40, 4),
        read_reg(0x7C, 2),
        read_reg(0x03, 1),
        read_reg(0x0C, 12),
        write_reg(0x2F, &[0x06]),
        read_reg(0x30, 4),
        // fifo_flush is accepted and does nothing: there is no FIFO.
        write_reg(0x7E, &[0xB0]),
        read_reg(0x00, 1),
    ])
}

/// What `components/bmi270.rs` put on the wire for [`migration_script`].
const BMI270_GOLDEN: &[u8] = &[
    0x24, 0x00, 0x10, 0x00, 0xA8, 0x02, 0xA9, 0x00, 0x03, 0x00, 0x00, 0x01, 0xD0, 0x90, 0x00, 0x10,
    0x00, 0xF8, 0x00, 0x04, 0x00, 0x10, 0x00, 0xC0, 0xFF, 0x7F, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00,
    0x40, 0xFF, 0x7F, 0xFF, 0x7F, 0x8F, 0xC2, 0x00, 0x40, 0x39, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xA8, 0x02, 0xA9, 0x00, 0x03, 0x00, 0x10, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x24,
];

#[test]
fn bmi270_is_byte_identical() {
    let got = run_i2c(&mut dev(), &migration_script());
    assert_eq!(
        got.bytes,
        BMI270_GOLDEN,
        "BMI270 transcript moved.\nexpected:\n{}\ngot:\n{}",
        common::transcript::Transcript {
            bytes: BMI270_GOLDEN.to_vec()
        }
        .render(),
        got.render()
    );
}

/// The anti-circular gate, on its own, in both directions. A firmware that
/// never streams the config image must not be told the feature engine started.
#[test]
fn init_ok_requires_the_config_blob_to_have_been_streamed() {
    let mut d = dev();
    let t = run_i2c(
        &mut d,
        &script([
            write_reg(0x59, &[0x01]), // INIT_CTRL = load_done, no blob
            read_reg(0x21, 1),
            write_reg(0x5E, &[0xAA]), // one byte of the image is enough
            read_reg(0x21, 1),        // still not_init: the gate is INIT_CTRL
            write_reg(0x59, &[0x01]),
            read_reg(0x21, 1),
        ]),
    );
    assert_eq!(
        t.bytes,
        vec![0x00, 0x00, 0x01],
        "not_init, not_init, init_ok — INIT_CTRL closes the gate and only the blob opens it"
    );
}

/// The reason `stream:` exists. One byte in every 256 of a firmware image is
/// 0xB6, which is `CMD = softreset`: a pointer that stepped per byte would walk
/// the 8 KB upload over CMD thirty-two times and reset the part it is
/// initialising. Here the whole burst lands on INIT_DATA and nothing else moves.
#[test]
fn the_config_upload_does_not_walk_over_the_register_map() {
    let mut d = dev();
    let mut blob: Vec<u8> = Vec::new();
    for i in 0..=255u16 {
        blob.push(i as u8); // every byte value, 0xB6 and 0x7E among them
    }
    let t = run_i2c(
        &mut d,
        &script([
            write_reg(0x41, &[0x01]), // ACC_RANGE = +/-4 g
            write_reg(0x7D, &[0x06]), // both sensors enabled
            write_reg(0x5E, &blob),
            write_reg(0x59, &[0x01]),
            read_reg(0x21, 1), // init_ok
            read_reg(0x41, 1), // ACC_RANGE survived
            read_reg(0x7D, 1), // PWR_CTRL survived
            read_reg(0x03, 1), // STATUS still reports both data-ready bits
        ]),
    );
    assert_eq!(
        t.bytes,
        vec![0x01, 0x01, 0x06, 0xD0],
        "the 256-byte config burst must touch nothing but INIT_DATA"
    );
}

/// Both range fields, every documented code, against the model's own
/// conversion. A sweep because the failure mode is one LSB at one setting:
/// codes 5..7 of GYR_RANGE give ±62, ±31 and ±15 dps, where 32768/fs is not a
/// round number.
#[test]
fn every_range_code_encodes_what_the_model_encoded() {
    // `Bmi270::accel_to_lsb` and `gyro_to_lsb`, verbatim from the deleted model.
    fn accel_to_lsb(g: f64, code: u32) -> i16 {
        let fs_g = (2u32 << code) as f64;
        (g.clamp(-fs_g, fs_g) * (32768.0 / fs_g)).round() as i16
    }
    fn gyro_to_lsb(dps: f64, code: u32) -> i16 {
        let fs_dps = (2000u32 >> code) as f64;
        (dps.clamp(-fs_dps, fs_dps) * (32768.0 / fs_dps)).round() as i16
    }
    for code in 0..4u32 {
        let mut d = dev();
        let mut g = -16.0f64;
        while g <= 16.0001 {
            let t = run_i2c(
                &mut d,
                &script([
                    write_reg(0x41, &[code as u8]),
                    vec![Step::Input("ax", g)],
                    read_reg(0x0C, 2),
                ]),
            );
            let word = i16::from_le_bytes([t.bytes[0], t.bytes[1]]);
            assert_eq!(
                word,
                accel_to_lsb(g, code),
                "ACC_RANGE code {code} at {g} g"
            );
            g = ((g + 0.125) * 1000.0).round() / 1000.0;
        }
    }
    for code in 0..8u32 {
        let mut d = dev();
        let mut dps = -2000.0f64;
        while dps <= 2000.0001 {
            let t = run_i2c(
                &mut d,
                &script([
                    write_reg(0x43, &[code as u8]),
                    vec![Step::Input("gx", dps)],
                    read_reg(0x12, 2),
                ]),
            );
            let word = i16::from_le_bytes([t.bytes[0], t.bytes[1]]);
            assert_eq!(
                word,
                gyro_to_lsb(dps, code),
                "GYR_RANGE code {code} at {dps} dps"
            );
            dps = ((dps + 6.25) * 1000.0).round() / 1000.0;
        }
    }
}

/// **DELIBERATE DIFFERENCE.** The model converted at `set_input` TIME and stored
/// an i16; the descriptor converts at READ time. A firmware that changes the
/// range after a sample has been posed therefore sees the SAME acceleration
/// reported at the new scale, where the model reported the old count
/// reinterpreted — which is not a reading any silicon produces. The ADXL345
/// port made the same change for the same reason.
#[test]
fn a_range_change_is_visible_to_the_next_read() {
    let mut d = dev();
    let t = run_i2c(
        &mut d,
        &script([
            write_reg(0x41, &[0x02]), // +/-8 g
            vec![Step::Input("ax", 1.0)],
            read_reg(0x0C, 2),
            write_reg(0x41, &[0x00]), // +/-2 g, sample unchanged
            read_reg(0x0C, 2),
        ]),
    );
    assert_eq!(
        i16::from_le_bytes([t.bytes[0], t.bytes[1]]),
        4096,
        "1 g at +/-8 g is 4096 counts"
    );
    assert_eq!(
        i16::from_le_bytes([t.bytes[2], t.bytes[3]]),
        16384,
        "the SAME 1 g at +/-2 g is 16384 counts. The model answered 4096 here: it had \
         already frozen the count at the old scale."
    );
}

/// **DELIBERATE DIFFERENCE.** A read of INIT_DATA returns its stored byte and
/// does not advance the pointer, where the model read 0 and stepped. INIT_DATA
/// is write-only in the datasheet and no driver reads it; the behaviour follows
/// from `stream:` holding the cursor in both directions, which is what a port
/// is.
#[test]
fn reading_the_streaming_port_holds_the_pointer() {
    let mut d = dev();
    let t = run_i2c(
        &mut d,
        &script([write_reg(0x5E, &[0x5A]), read_reg(0x5E, 3)]),
    );
    assert_eq!(
        t.bytes,
        vec![0x5A, 0x5A, 0x5A],
        "the port serves its own byte for as long as the master clocks; the model \
         answered 0x00, 0x00, 0x00 by walking off into INIT_ADDR space"
    );
}

/// The step counter is reachable only on its own feature page, and a soft reset
/// puts the page selector back to 0 — so a driver that resets and forgets to
/// re-page reads zeros rather than a stale count.
#[test]
fn the_step_counter_lives_on_feature_page_six() {
    let mut d = dev();
    let t = run_i2c(
        &mut d,
        &script([
            vec![Step::Input("steps", 4_000_000_000.0)],
            write_reg(0x2F, &[0x06]),
            read_reg(0x30, 4),
            write_reg(0x2F, &[0x05]),
            read_reg(0x30, 4),
        ]),
    );
    assert_eq!(
        u32::from_le_bytes([t.bytes[0], t.bytes[1], t.bytes[2], t.bytes[3]]),
        4_000_000_000,
        "the 32-bit step count, little-endian at the base of the window"
    );
    assert_eq!(
        &t.bytes[4..8],
        &[0, 0, 0, 0],
        "page 5 holds a different feature output, none of which is modelled"
    );
}
