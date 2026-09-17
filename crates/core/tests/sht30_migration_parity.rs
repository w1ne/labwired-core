// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SHT30: the declarative descriptor against the hand-written model it
//! replaces.
//!
//! The deleted `components/sht30.rs` is reproduced below as [`legacy`] — the
//! wire behaviour only — and both models are driven through the SAME I²C
//! script. Transcripts that must be identical are asserted equal byte for
//! byte; those that must differ are asserted as differences, by name.
//!
//! Held identical:
//!   * the six-byte measurement frame — T word, CRC, RH word, CRC — for a
//!     single-shot command, over the whole stimulus range;
//!   * the power-on stimulus (25 °C / 50 %RH);
//!   * 0xFF past the end of the frame.
//!
//! Deliberately DIFFERENT:
//!   * the opcode is decoded. The old model answered ANY two bytes with a
//!     measurement frame — a soft reset, a status read, a typo.
//!   * the status register reads the SHT3x power-on word instead of a
//!     temperature that was never measured.
//!   * a single-shot measurement takes the datasheet's 15 ms. The old model
//!     answered instantly, so a driver that read before its own measurement
//!     could possibly be done passed against it.
//!
//! Scripts are driven through Phase A's shared harness
//! (`tests/common/transcript.rs`), the same one the byte-parity ratchet uses.

mod common;

use common::transcript::{read_stream, run_i2c, script, send_cmd16, Step};
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::sim_input::SimInput;

const ADDR: u8 = 0x44;

// ─── the model this descriptor replaces ────────────────────────────────────

/// `crates/core/src/peripherals/components/sht30.rs` at `origin/main`, wire
/// behaviour only. This is the ONLY place the old behaviour survives.
mod legacy {
    use labwired_core::peripherals::i2c::I2cDevice;

    pub struct Sht30 {
        address: u8,
        pub temperature_c: f64,
        pub humidity_rh: f64,
        cmd: [u8; 2],
        cmd_len: u8,
        read_idx: usize,
    }

    impl Sht30 {
        pub fn new(address: u8) -> Self {
            Self {
                address,
                temperature_c: 25.0,
                humidity_rh: 50.0,
                cmd: [0; 2],
                cmd_len: 0,
                read_idx: 0,
            }
        }

        fn encode_payload(&self) -> [u8; 6] {
            let raw_t = (((self.temperature_c + 45.0) * 65535.0) / 175.0)
                .round()
                .clamp(0.0, 65535.0) as u16;
            let raw_h = ((self.humidity_rh * 65535.0) / 100.0)
                .round()
                .clamp(0.0, 65535.0) as u16;
            let t_hi = (raw_t >> 8) as u8;
            let t_lo = (raw_t & 0xFF) as u8;
            let h_hi = (raw_h >> 8) as u8;
            let h_lo = (raw_h & 0xFF) as u8;
            [
                t_hi,
                t_lo,
                crc8(&[t_hi, t_lo]),
                h_hi,
                h_lo,
                crc8(&[h_hi, h_lo]),
            ]
        }
    }

    /// CRC-8 poly 0x31, init 0xFF (Sensirion).
    fn crc8(data: &[u8]) -> u8 {
        let mut crc: u8 = 0xFF;
        for &b in data {
            crc ^= b;
            for _ in 0..8 {
                if crc & 0x80 != 0 {
                    crc = (crc << 1) ^ 0x31;
                } else {
                    crc <<= 1;
                }
            }
        }
        crc
    }

    impl I2cDevice for Sht30 {
        fn address(&self) -> u8 {
            self.address
        }

        fn start(&mut self) {
            self.cmd_len = 0;
            self.read_idx = 0;
        }

        fn write(&mut self, data: u8) {
            if (self.cmd_len as usize) < self.cmd.len() {
                self.cmd[self.cmd_len as usize] = data;
                self.cmd_len += 1;
            }
        }

        fn read(&mut self) -> u8 {
            let payload = self.encode_payload();
            let b = payload.get(self.read_idx).copied().unwrap_or(0xFF);
            self.read_idx = self.read_idx.saturating_add(1);
            b
        }
    }
}

// ─── the script vocabulary ─────────────────────────────────────────────────
//
// `send_cmd16` and `read_stream` come straight from the shared harness — a
// Sensirion part is exactly the opcode-then-frame shape they describe.

/// Opcode, wait out the datasheet conversion, then clock out `n` response
/// bytes. The wait is what makes the two models comparable at all: the
/// descriptor gates a single-shot frame on 15 ms of the device's own clock
/// (see `CONVERSION_US`), and the model it replaces answered instantly.
fn cmd_then_read(code: u16, n: usize) -> Vec<Step<'static>> {
    script([
        send_cmd16(code),
        vec![Step::AdvanceUs(CONVERSION_US)],
        read_stream(n),
    ])
}

/// Opcode then read with NO time advance — how the old model was always driven.
fn cmd_then_read_now(code: u16, n: usize) -> Vec<Step<'static>> {
    script([send_cmd16(code), read_stream(n)])
}

/// SHT3x single-shot high-repeatability maximum measurement duration
/// (datasheet §2.2, table 4).
const CONVERSION_US: u64 = 15_000;

fn declarative(t: f64, rh: f64) -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("sht30")
        .expect("sht30 descriptor is not embedded — check embedded_device_yaml");
    let mut dev = GenericI2cDevice::from_yaml(yaml, ADDR).expect("sht30.yaml does not build");
    dev.set_input("temperature", t)
        .expect("temperature channel");
    dev.set_input("humidity", rh).expect("humidity channel");
    dev
}

fn both(t: f64, rh: f64, steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy::Sht30::new(ADDR);
    old.temperature_c = t;
    old.humidity_rh = rh;
    let mut new = declarative(t, rh);
    (
        run_i2c(&mut old, steps).bytes,
        run_i2c(&mut new, steps).bytes,
    )
}

fn assert_parity(name: &str, t: f64, rh: f64, steps: &[Step<'_>]) -> Vec<u8> {
    let (old, new) = both(t, rh, steps);
    assert!(!old.is_empty(), "{name}: the script read no bytes at all");
    assert_eq!(old, new, "{name}: the YAML model changed the transcript");
    new
}

/// Sensirion CRC-8 (poly 0x31, init 0xFF), recomputed here so the assertion
/// does not borrow the implementation it is checking.
fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0xFF;
    for &b in data {
        crc ^= b;
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

// ─── held identical ────────────────────────────────────────────────────────

#[test]
fn the_single_shot_frame_is_byte_identical() {
    // The transaction every SHT3x driver issues: measurement opcode, then six
    // bytes — T MSB/LSB/CRC, RH MSB/LSB/CRC.
    let bytes = assert_parity("single shot", 25.0, 50.0, &cmd_then_read(0x2400, 6));
    assert_eq!(bytes.len(), 6);
    assert_eq!(bytes[2], crc8(&bytes[0..2]), "temperature CRC");
    assert_eq!(bytes[5], crc8(&bytes[3..5]), "humidity CRC");
}

#[test]
fn the_conversion_matches_over_the_whole_range() {
    // The descriptor's encode is `T × 65535/175 + 45 × 65535/175`; the model it
    // replaces computed `(T + 45) × 65535 / 175`. Same value, different
    // floating-point order, so the rounding could disagree at a boundary.
    // Swept in 0.25 °C / 0.5 %RH steps rather than spot-checked.
    // The declared channel range is -40..125 °C, 0..100 %RH — a value outside
    // it is rejected by `set_input`, so the sweep stays inside the contract.
    // Both models are built ONCE and re-driven, because parsing the descriptor
    // per sample turns a 0.01-step sweep into half a minute of YAML.
    let mut old = legacy::Sht30::new(ADDR);
    let mut new = declarative(25.0, 50.0);
    let steps = cmd_then_read(0x2400, 6);
    for ti in -4000..=12500 {
        let t = f64::from(ti) * 0.01;
        old.temperature_c = t;
        new.set_input("temperature", t)
            .expect("temperature channel");
        assert_eq!(
            run_i2c(&mut old, &steps).bytes,
            run_i2c(&mut new, &steps).bytes,
            "T={t}: the encoded temperature word differs"
        );
    }
    let t = 25.0;
    old.temperature_c = t;
    new.set_input("temperature", t)
        .expect("temperature channel");
    for hi in 0..=10000 {
        let rh = f64::from(hi) * 0.01;
        old.humidity_rh = rh;
        new.set_input("humidity", rh).expect("humidity channel");
        assert_eq!(
            run_i2c(&mut old, &steps).bytes,
            run_i2c(&mut new, &steps).bytes,
            "RH={rh}: the encoded humidity word differs"
        );
    }
}

#[test]
fn every_single_shot_opcode_returns_the_same_frame() {
    // Repeatability selects measurement duration and noise, neither of which is
    // modelled, so all six single-shot opcodes answer identically — and all six
    // answer what the old model answered.
    for code in [0x2400u16, 0x240B, 0x2416, 0x2C06, 0x2C0D, 0x2C10] {
        assert_parity("single shot opcode", 12.5, 77.5, &cmd_then_read(code, 6));
    }
}

#[test]
fn periodic_fetch_returns_the_measurement_frame() {
    // FETCH_DATA after a periodic-mode opcode. The old model answered both the
    // mode select and the fetch with a frame; only the fetch is compared here,
    // because the mode select is one of the deliberate differences below.
    assert_parity(
        "periodic fetch",
        -10.0,
        99.0,
        &script([send_cmd16(0x2130), cmd_then_read_now(0xE000, 6)]),
    );
}

#[test]
fn reading_past_the_frame_returns_ones() {
    assert_parity("past the frame", 25.0, 50.0, &cmd_then_read(0x2400, 9));
}

// ─── deliberately different ────────────────────────────────────────────────

#[test]
fn an_undeclared_opcode_no_longer_answers_with_a_measurement() {
    // THE deliberate change. The old model ignored the opcode entirely, so a
    // typo — or a command this descriptor does not implement — came back as a
    // plausible temperature and humidity that were never measured.
    let (old, new) = both(25.0, 50.0, &cmd_then_read_now(0xABCD, 6));

    assert_eq!(old.len(), 6);
    assert_ne!(
        old,
        vec![0xFF; 6],
        "the model this replaces answered a measurement frame"
    );
    assert_eq!(
        new,
        vec![0xFF; 6],
        "an undeclared opcode must queue no response"
    );
}

#[test]
fn the_status_register_is_a_status_word_now() {
    // 0xF32D read back a temperature frame before. It is the SHT3x power-on
    // status (alert pending + reset detected) with its Sensirion CRC.
    let (old, new) = both(25.0, 50.0, &cmd_then_read_now(0xF32D, 3));

    assert_eq!(new, vec![0x80, 0x10, crc8(&[0x80, 0x10])]);
    assert_ne!(old, new, "the old model answered a measurement here");
}

#[test]
fn a_single_shot_frame_is_not_readable_before_the_conversion_completes() {
    // THE third deliberate change, and the one Phase A unblocked: `delay_us`
    // withholds the frame until the device's own clock reaches the deadline,
    // and that clock now runs on every chip. A driver that reads too early sees
    // 0xFF — not a plausible reading it never measured.
    let early = script([
        send_cmd16(0x2400),
        vec![Step::AdvanceUs(CONVERSION_US - 1)],
        read_stream(6),
    ]);
    let (old, new) = both(25.0, 50.0, &early);
    assert_eq!(new, vec![0xFF; 6], "one µs short of the conversion time");
    assert_ne!(old, new, "the model this replaces answered instantly");

    // And one µs later it is there — the positive control, so this test cannot
    // pass by the frame never arriving at all.
    let ready = script([
        send_cmd16(0x2400),
        vec![Step::AdvanceUs(CONVERSION_US - 1), Step::AdvanceUs(1)],
        read_stream(6),
    ]);
    let (_, new) = both(25.0, 50.0, &ready);
    assert_ne!(new, vec![0xFF; 6]);
    assert_eq!(new[2], crc8(&new[0..2]), "temperature CRC");
}

#[test]
fn a_soft_reset_queues_no_response() {
    // Write-only opcode: the part ACKs and says nothing. The old model handed
    // back a measurement.
    let (old, new) = both(25.0, 50.0, &cmd_then_read_now(0x30A2, 6));

    assert_eq!(new, vec![0xFF; 6]);
    assert_ne!(old, new);
}
