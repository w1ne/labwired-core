// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **SCD41 and SGP41, against the hand-written models they replace.**
//!
//! `components/scd41.rs` (444 lines) and `components/sgp41.rs` (326) are
//! DELETED. Both were the same file: a two-byte command accumulator, a
//! `match` queuing 16-bit words, and `encode_words` stapling a Sensirion CRC-8
//! onto each one. The `match` is a table and the framing is the engine's, so
//! what is left is the table.
//!
//! Each golden constant below is the transcript its model produced, captured by
//! running the script beside it against the model on the commit that removed
//! it. **Both are byte-identical** — no named difference, on either part.
//!
//! The temperature sweep is the reason this file is not just two constants:
//! `encode:` multiplies and adds where the model added, divided and then
//! multiplied, and the two orders round apart by one LSB at −27.5 °C. The
//! literal in `scd41.yaml` is the double that closes that gap, and
//! [`scd41_temperature_encoding_matches_the_model_across_the_channel`] sweeps
//! the whole declared range so an edit to it fails here rather than drifting
//! the last bit of a word nobody re-reads.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;

mod common;
use common::transcript::{read_stream, run_i2c, script, send_cmd16, Step};

fn dev(device_type: &str, addr: u8) -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml(device_type)
        .unwrap_or_else(|| panic!("{device_type} descriptor is not embedded"));
    GenericI2cDevice::from_yaml(yaml, addr).unwrap_or_else(|e| panic!("{device_type}.yaml: {e}"))
}

// ─── SCD41 ─────────────────────────────────────────────────────────────────

/// The conversation the deleted `Scd41` was driven through.
///
/// Two of these blocks exist to pin what does NOT answer: `measure_single_shot`
/// is a write-only trigger, and `set_temperature_offset` (0x241D) is an opcode
/// this descriptor does not declare. Both must read 0xFF, which is what the
/// model's empty response buffer produced.
fn scd_script() -> Vec<Step<'static>> {
    script([
        send_cmd16(0x3682),
        read_stream(9), // get_serial_number
        send_cmd16(0x3639),
        read_stream(3), // perform_self_test
        send_cmd16(0x21B1),
        send_cmd16(0xE4B8),
        read_stream(3), // get_data_ready_status
        send_cmd16(0xEC05),
        read_stream(9), // read_measurement at the seeded defaults
        vec![
            Step::Input("co2", 1234.0),
            Step::Input("temperature", -27.5), // the one-LSB rounding edge
            Step::Input("humidity", 88.25),
        ],
        send_cmd16(0xEC05),
        read_stream(9),
        send_cmd16(0x3F86),
        send_cmd16(0x219D),
        read_stream(3), // a write-only trigger queues nothing
        send_cmd16(0x241D),
        read_stream(3), // an undeclared opcode queues nothing
        vec![
            Step::Input("co2", 40000.0),
            Step::Input("temperature", 130.0),
        ],
        send_cmd16(0xEC05),
        read_stream(9), // both channels at the top of their range
    ])
}

/// What `components/scd41.rs` put on the wire for [`scd_script`].
const SCD41_GOLDEN: &[u8] = &[
    0x4C, 0x45, 0x74, 0x4F, 0x31, 0x65, 0x00, 0x41, 0x8D, 0x00, 0x00, 0x81, 0x80, 0x06, 0x04, 0x01,
    0xC2, 0x50, 0x62, 0x03, 0x5E, 0x73, 0x33, 0x01, 0x04, 0xD2, 0x64, 0x19, 0x9A, 0xCE, 0xE1, 0xEB,
    0x28, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x9C, 0x40, 0x45, 0xFF, 0xFF, 0xAC, 0xE1, 0xEB, 0x28,
];

#[test]
fn scd41_is_byte_identical() {
    let got = run_i2c(&mut dev("scd41", 0x62), &scd_script());
    assert_eq!(
        got.bytes,
        SCD41_GOLDEN,
        "SCD41 transcript moved.\ngot:\n{}",
        got.render()
    );
}

/// The whole declared channel, at 0.1 °C, against the model's own expression.
/// A sweep rather than a spot check because the failure this guards is one LSB
/// at one temperature: the spot check that found it was luck.
#[test]
fn scd41_temperature_encoding_matches_the_model_across_the_channel() {
    // `Scd41::encode_temperature`, verbatim from the deleted model.
    fn model(t_c: f64) -> u16 {
        (((t_c + 45.0) / 175.0) * 65535.0)
            .round()
            .clamp(0.0, 65535.0) as u16
    }
    let mut d = dev("scd41", 0x62);
    let mut milli = -45_000i32;
    while milli <= 130_000 {
        let t = f64::from(milli) / 1000.0;
        let t = ((t * 1000.0).round()) / 1000.0;
        let got = run_i2c(
            &mut d,
            &script([
                vec![Step::Input("temperature", t)],
                send_cmd16(0xEC05),
                read_stream(6),
            ]),
        );
        let word = u16::from(got.bytes[3]) << 8 | u16::from(got.bytes[4]);
        assert_eq!(
            word,
            model(t),
            "temperature {t} °C encodes to {word:#06x}, the model made {:#06x}",
            model(t)
        );
        milli += 100;
    }
}

#[test]
fn scd41_co2_saturates_at_the_range_the_datasheet_gives_the_part() {
    let mut d = dev("scd41", 0x62);
    let got = run_i2c(
        &mut d,
        &script([
            vec![Step::Input("co2", 40000.0)],
            send_cmd16(0xEC05),
            read_stream(3),
        ]),
    );
    assert_eq!(
        (u16::from(got.bytes[0]) << 8) | u16::from(got.bytes[1]),
        40000,
        "§1.1 gives the SCD41 a 0..40000 ppm reportable range"
    );
}

// ─── SGP41 ─────────────────────────────────────────────────────────────────

/// The conversation the deleted `Sgp41` was driven through.
fn sgp_script() -> Vec<Step<'static>> {
    script([
        send_cmd16(0x3682),
        read_stream(9), // get_serial_number
        send_cmd16(0x280E),
        read_stream(3), // execute_self_test
        send_cmd16(0x2612),
        read_stream(3), // execute_conditioning — VOC only
        send_cmd16(0x2619),
        read_stream(6), // measure_raw_signals at the seeded defaults
        vec![
            Step::Input("voc_sraw", 41234.0),
            Step::Input("nox_sraw", 65535.0),
        ],
        send_cmd16(0x2619),
        read_stream(6),
        send_cmd16(0x3615),
        send_cmd16(0x0000),
        read_stream(3), // an undeclared opcode queues nothing
    ])
}

/// What `components/sgp41.rs` put on the wire for [`sgp_script`].
const SGP41_GOLDEN: &[u8] = &[
    0x53, 0x47, 0xE1, 0x50, 0x34, 0x67, 0x00, 0x31, 0x75, 0xD4, 0x00, 0xC6, 0x6D, 0x60, 0x2F, 0x6D,
    0x60, 0x2F, 0x3E, 0x80, 0x24, 0xA1, 0x12, 0xAB, 0xFF, 0xFF, 0xAC, 0xFF, 0xFF, 0xFF,
];

#[test]
fn sgp41_is_byte_identical() {
    let got = run_i2c(&mut dev("sgp41", 0x59), &sgp_script());
    assert_eq!(
        got.bytes,
        SGP41_GOLDEN,
        "SGP41 transcript moved.\ngot:\n{}",
        got.render()
    );
}

/// The two parameter words `measure_raw_signals` carries are RH and T
/// compensation. The model accepted and discarded them; so does the descriptor,
/// and this pins that a driver that sends them still gets its answer rather
/// than having them decoded as a second command.
#[test]
fn sgp41_measure_accepts_its_two_parameter_words() {
    let mut d = dev("sgp41", 0x59);
    let got = run_i2c(
        &mut d,
        &script([
            vec![
                Step::Start,
                Step::Write(0x26),
                Step::Write(0x19),
                Step::Write(0x80),
                Step::Write(0x00),
                Step::Write(0xA2), // RH word + CRC
                Step::Write(0x66),
                Step::Write(0x66),
                Step::Write(0x93), // T word + CRC
                Step::Stop,
            ],
            read_stream(6),
        ]),
    );
    assert_eq!(
        got.bytes,
        vec![0x6D, 0x60, 0x2F, 0x3E, 0x80, 0x24],
        "the seeded VOC and NOx ticks, unaffected by the compensation words"
    );
}

/// The CRC-8 a Sensirion driver validates. One byte per word, poly 0x31,
/// init 0xFF — asserted against an independent implementation rather than
/// against the engine's own, so a change to the engine's polynomial cannot
/// re-bless itself.
#[test]
fn every_response_word_carries_a_sensirion_crc8() {
    fn crc8(data: &[u8]) -> u8 {
        let mut crc = 0xFFu8;
        for byte in data {
            crc ^= byte;
            for _ in 0..8 {
                crc = if crc & 0x80 != 0 {
                    (crc << 1) ^ 0x31
                } else {
                    crc << 1
                };
            }
        }
        crc
    }
    for (device_type, addr, code, words) in [
        ("scd41", 0x62u8, 0xEC05u16, 3usize),
        ("sgp41", 0x59, 0x2619, 2),
    ] {
        let mut d = dev(device_type, addr);
        let got = run_i2c(&mut d, &script([send_cmd16(code), read_stream(words * 3)]));
        for w in 0..words {
            let frame = &got.bytes[w * 3..w * 3 + 3];
            assert_eq!(
                frame[2],
                crc8(&frame[..2]),
                "{device_type} word {w} of {code:#06x} carries a bad CRC-8"
            );
        }
    }
}
