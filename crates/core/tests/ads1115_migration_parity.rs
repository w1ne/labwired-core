// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ADS1115: the declarative descriptor against the hand-written model it
//! replaces.
//!
//! The deleted `components/ads1115.rs` is reproduced below as [`legacy`] — the
//! wire behaviour only — and both models are driven through the SAME I²C
//! script. Transcripts that must be identical are asserted equal byte for
//! byte; the ones that must differ are asserted as differences, by name.
//!
//! Held identical:
//!   * the whole pointer map and the zero word an undecoded pointer answers;
//!   * CONVERSION for all four SINGLE-ENDED MUX settings at all six PGA
//!     settings, swept across each range — the descriptor MULTIPLIES by
//!     `32768 / FSR` where the model DIVIDED by `FSR`, and "those are the same
//!     double" is an argument, not a measurement;
//!   * the 16-bit MSB-first write and read-back of CONFIG and both thresholds,
//!     including OS (D15) reading set whatever firmware writes;
//!   * writes to the read-only CONVERSION register being dropped.
//!
//! Deliberately DIFFERENT (each with its own test below):
//!   * the four DIFFERENTIAL MUX settings are real subtractions. The hand model
//!     answered all four with AIN0 alone, its own comment saying "approximate
//!     as ch0 for wave-1" — and MUX = 000 (AIN0 − AIN1) is the POWER-ON
//!     setting, so a driver that never writes CONFIG was already in one of them;
//!   * a read that runs PAST the pointed 16-bit word answers 0xFF instead of
//!     repeating it.
//!
//! `source_from` is the primitive this port waited for: §9.3.3 Table 8 makes
//! CONVERSION report whichever input CONFIG's MUX bits select, and nothing in
//! Tier 1 could choose a register's SOURCE from another register's field.

mod common;

use common::transcript::{read_reg, run_i2c, script, write_reg, Step};
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::sim_input::SimInput;

const ADDR: u8 = 0x48;

/// The bits of CONFIG that are neither MUX nor PGA, taken from the POR word
/// 0x8583: single-shot mode, 128 SPS, the traditional comparator defaults.
const CONFIG_TAIL: u16 = 0x0183;

/// CONFIG with a chosen MUX (bits 14:12) and PGA (bits 11:9).
fn config(mux: u16, pga: u16) -> u16 {
    0x8000 | (mux << 12) | (pga << 9) | CONFIG_TAIL
}

// ─── the model this descriptor replaces ────────────────────────────────────

/// `crates/core/src/peripherals/components/ads1115.rs` at `origin/main`, wire
/// behaviour only. This is the ONLY place the old behaviour survives.
mod legacy {
    use labwired_core::peripherals::i2c::I2cDevice;

    const DEFAULT_CONFIG: u16 = 0x8583;

    pub struct Ads1115 {
        current_register: u8,
        register_address_written: bool,
        write_high: Option<u8>,
        read_low_pending: Option<u8>,
        pub config: u16,
        lo_thresh: u16,
        hi_thresh: u16,
        channels: [f64; 4],
    }

    impl Ads1115 {
        pub fn new() -> Self {
            Self {
                current_register: 0x00,
                register_address_written: false,
                write_high: None,
                read_low_pending: None,
                config: DEFAULT_CONFIG,
                lo_thresh: 0x8000,
                hi_thresh: 0x7FFF,
                channels: [0.0; 4],
            }
        }

        pub fn set_input(&mut self, key: &str, value: f64) {
            let ch = match key {
                "a0" => 0,
                "a1" => 1,
                "a2" => 2,
                "a3" => 3,
                other => panic!("unknown legacy channel {other}"),
            };
            self.channels[ch] = value.clamp(-4.096, 4.096);
        }

        /// ⚠️ The approximation this migration replaces: every differential
        /// MUX setting (000..011) answered AIN0.
        fn mux_channel(&self) -> usize {
            match (self.config >> 12) & 0x07 {
                0b100 => 0,
                0b101 => 1,
                0b110 => 2,
                0b111 => 3,
                _ => 0,
            }
        }

        fn fsr_v(&self) -> f64 {
            match (self.config >> 9) & 0x07 {
                0b000 => 6.144,
                0b001 => 4.096,
                0b010 => 2.048,
                0b011 => 1.024,
                0b100 => 0.512,
                _ => 0.256,
            }
        }

        fn conversion_raw(&self) -> u16 {
            let v = self.channels[self.mux_channel()];
            let fsr = self.fsr_v();
            let counts = ((v / fsr) * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
            counts as u16
        }

        fn read_register_u16(&self, reg: u8) -> u16 {
            match reg {
                0x00 => self.conversion_raw(),
                0x01 => self.config | 0x8000,
                0x02 => self.lo_thresh,
                0x03 => self.hi_thresh,
                _ => 0,
            }
        }

        fn write_register_u16(&mut self, reg: u8, value: u16) {
            match reg {
                0x01 => self.config = value,
                0x02 => self.lo_thresh = value,
                0x03 => self.hi_thresh = value,
                _ => {}
            }
        }
    }

    impl I2cDevice for Ads1115 {
        fn address(&self) -> u8 {
            0x48
        }

        fn read(&mut self) -> u8 {
            if let Some(low) = self.read_low_pending.take() {
                return low;
            }
            let word = self.read_register_u16(self.current_register);
            self.read_low_pending = Some((word & 0xFF) as u8);
            (word >> 8) as u8
        }

        fn write(&mut self, data: u8) {
            if !self.register_address_written {
                self.current_register = data;
                self.register_address_written = true;
                self.write_high = None;
                self.read_low_pending = None;
                return;
            }
            match self.write_high {
                None => self.write_high = Some(data),
                Some(high) => {
                    let word = (u16::from(high) << 8) | u16::from(data);
                    self.write_register_u16(self.current_register, word);
                    self.write_high = None;
                }
            }
        }

        fn stop(&mut self) {
            self.register_address_written = false;
            self.write_high = None;
            self.read_low_pending = None;
        }
    }
}

// ─── harness ───────────────────────────────────────────────────────────────

fn declarative() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("ads1115")
        .expect("ads1115 descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("ads1115.yaml does not build")
}

/// Both models, with all four channels driven.
fn both(v: [f64; 4], steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy::Ads1115::new();
    let mut new = declarative();
    for (i, key) in ["a0", "a1", "a2", "a3"].iter().enumerate() {
        old.set_input(key, v[i]);
        new.set_input(key, v[i]).expect("channel");
    }
    (
        run_i2c(&mut old, steps).bytes,
        run_i2c(&mut new, steps).bytes,
    )
}

fn assert_parity(name: &str, v: [f64; 4], steps: &[Step<'_>]) -> Vec<u8> {
    let (old, new) = both(v, steps);
    assert!(!old.is_empty(), "{name}: the script read no bytes at all");
    assert_eq!(old, new, "{name}: the YAML model changed the transcript");
    new
}

fn word(bytes: &[u8]) -> u16 {
    assert_eq!(bytes.len(), 2, "an ADS1115 register is one 16-bit word");
    (u16::from(bytes[0]) << 8) | u16::from(bytes[1])
}

/// Select `mux`/`pga`, then read CONVERSION. The transcript is the 2-byte word.
fn convert(mux: u16, pga: u16) -> Vec<Step<'static>> {
    let cfg = config(mux, pga);
    script([
        write_reg(0x01, &[(cfg >> 8) as u8, (cfg & 0xFF) as u8]),
        read_reg(0x00, 2),
    ])
}

/// Both models' CONVERSION word at one mux/pga/stimulus point.
fn conversion(mux: u16, pga: u16, v: [f64; 4]) -> (i16, i16) {
    let (old, new) = both(v, &convert(mux, pga));
    (word(&old) as i16, word(&new) as i16)
}

const FSR: [f64; 8] = [6.144, 4.096, 2.048, 1.024, 0.512, 0.256, 0.256, 0.256];

// ─── held identical ────────────────────────────────────────────────────────

#[test]
fn the_whole_pointer_map_reads_identically() {
    // MUX = 100 (AIN0 single-ended) so CONVERSION is a setting the two models
    // agree on by construction; the differential ones have their own test.
    // Every pointer, the gaps and past the end — the old model answered a zero
    // WORD for an undecoded pointer and the descriptor has to answer the same.
    let ptrs: Vec<u8> = (0x00u8..=0x1F).collect();
    let steps = script(
        std::iter::once(convert(0b100, 0b001))
            .chain(ptrs.iter().map(|&p| read_reg(p, 2)))
            .collect::<Vec<_>>(),
    );
    let bytes = assert_parity("full map", [2.048, -1.0, 0.5, -0.25], &steps);
    // The leading `convert` contributes one word before the map walk.
    let at = |ptr: u8| word(&bytes[2 + usize::from(ptr) * 2..2 + usize::from(ptr) * 2 + 2]);
    assert_eq!(at(0x00) as i16, 16_384, "2.048 V of a ±4.096 V range");
    assert_eq!(at(0x01), config(0b100, 0b001), "CONFIG reads back, OS set");
    assert_eq!(at(0x02), 0x8000, "LO_THRESH power-on");
    assert_eq!(at(0x03), 0x7FFF, "HI_THRESH power-on");
    assert_eq!(at(0x04), 0x0000, "past the map");
    assert_eq!(at(0x1F), 0x0000, "still past the map");
}

#[test]
fn every_single_ended_channel_matches_across_every_pga_range() {
    // The encoding change this port makes is a DIVIDE turned into a MULTIPLY:
    // the hand model computed `(v / FSR) × 32768` and the descriptor carries
    // `32768 / FSR` as a `scale_from` factor. For ±6.144 V that factor is
    // 5333.333333333333, which is not exact in binary — so the claim that the
    // two never round apart is swept rather than asserted: four channels × six
    // PGA settings × 401 points across each full range.
    for pga in 0u16..=5 {
        let fsr = FSR[pga as usize];
        for ch in 0u16..4 {
            let mux = 0b100 + ch;
            for step in -200i32..=200 {
                let v = (fsr * f64::from(step) / 200.0).clamp(-4.096, 4.096);
                let mut stim = [0.0; 4];
                stim[ch as usize] = v;
                let (old, new) = conversion(mux, pga, stim);
                assert_eq!(
                    old, new,
                    "MUX {mux:03b} PGA {pga:03b} at {v} V: the count differs"
                );
            }
        }
    }
}

#[test]
fn the_pga_ranges_are_the_datasheet_full_scales() {
    // The sweep above would pass on two models that agreed on garbage, so the
    // §9.3.3 Table 9 endpoints are asserted on their own: half of full scale on
    // AIN0 is a quarter of the count range, whatever the range is.
    for pga in 0u16..=5 {
        let half = FSR[pga as usize] / 2.0;
        let (_, new) = conversion(0b100, pga, [half.min(4.096), 0.0, 0.0, 0.0]);
        if half <= 4.096 {
            assert_eq!(new, 16_384, "PGA {pga:03b}: half of full scale");
        }
    }
    // Over-range saturates rather than wrapping.
    assert_eq!(conversion(0b100, 0b101, [4.0, 0.0, 0.0, 0.0]).1, 32_767);
    assert_eq!(conversion(0b100, 0b101, [-4.0, 0.0, 0.0, 0.0]).1, -32_768);
}

#[test]
fn a_sixteen_bit_write_round_trips_through_every_writable_register() {
    for (ptr, name) in [(0x01u8, "CONFIG"), (0x02, "LO_THRESH"), (0x03, "HI_THRESH")] {
        let steps = script([write_reg(ptr, &[0x5A, 0xA5]), read_reg(ptr, 2)]);
        let bytes = assert_parity(name, [0.0; 4], &steps);
        let expect = if ptr == 0x01 { 0x5AA5 | 0x8000 } else { 0x5AA5 };
        assert_eq!(word(&bytes), expect, "{name} did not read back its write");
    }
}

#[test]
fn os_reads_set_however_firmware_writes_it() {
    // §9.3.3: OS reads 1 when the device is not performing a conversion, and
    // this model is always done. `write_mask: 0x7FFF` makes that bit silicon's,
    // exactly as the hand model's `config | 0x8000` did.
    for pattern in [[0xFFu8, 0xFF], [0x00, 0x00], [0x7F, 0xFF]] {
        let steps = script([write_reg(0x01, &pattern), read_reg(0x01, 2)]);
        let bytes = assert_parity("config write", [0.0; 4], &steps);
        assert_eq!(
            word(&bytes) & 0x8000,
            0x8000,
            "writing {pattern:02X?} cleared OS"
        );
    }
}

#[test]
fn a_write_to_the_conversion_register_is_dropped() {
    let steps = script([
        write_reg(0x01, &{
            let c = config(0b100, 0b001);
            [(c >> 8) as u8, (c & 0xFF) as u8]
        }),
        write_reg(0x00, &[0xDE, 0xAD]),
        read_reg(0x00, 2),
    ]);
    let bytes = assert_parity("read-only write", [1.024, 0.0, 0.0, 0.0], &steps);
    assert_eq!(word(&bytes) as i16, 8192, "still the measurement");
}

// ─── the mux, proved live ──────────────────────────────────────────────────

#[test]
fn conversion_follows_the_live_mux_bits() {
    // The point of `source_from`: ONE register, four different answers, chosen
    // by a bit-field of ANOTHER register. Each channel carries a distinct
    // voltage so a mux that quietly latched the wrong one cannot pass.
    let stim = [0.5, 1.0, 1.5, 2.0];
    for (mux, expect_v) in [(0b100u16, 0.5f64), (0b101, 1.0), (0b110, 1.5), (0b111, 2.0)] {
        let (old, new) = conversion(mux, 0b001, stim);
        assert_eq!(old, new, "MUX {mux:03b} is a single-ended setting");
        assert_eq!(
            new,
            (expect_v / 4.096 * 32768.0).round() as i16,
            "MUX {mux:03b} did not select {expect_v} V"
        );
    }
}

// ─── deliberately different ────────────────────────────────────────────────

#[test]
fn the_four_differential_pairs_are_real_subtractions() {
    // THE deliberate change. §9.3.3 Table 8 gives MUX 000..011 as AIN0−AIN1,
    // AIN0−AIN3, AIN1−AIN3 and AIN2−AIN3. The hand model answered ALL FOUR with
    // AIN0, so a bridge or shunt sketch measuring a small difference between two
    // large voltages read one of them instead — and MUX = 000 is the POWER-ON
    // setting, so a driver that never wrote CONFIG was already in that case.
    let stim = [2.0, 0.5, 1.0, 0.25];
    let counts = |v: f64| (v / 4.096 * 32768.0).round() as i16;
    for (mux, hi, lo) in [
        (0b000u16, 2.0, 0.5),
        (0b001, 2.0, 0.25),
        (0b010, 0.5, 0.25),
        (0b011, 1.0, 0.25),
    ] {
        let (old, new) = conversion(mux, 0b001, stim);
        assert_eq!(old, counts(2.0), "MUX {mux:03b}: the model answered AIN0");
        assert_eq!(
            new,
            counts(hi - lo),
            "MUX {mux:03b} should be the difference"
        );
        assert_ne!(old, new, "this difference is the point of the change");
    }
    // A differential pair of two EQUAL voltages reads zero — the property a
    // bridge sketch depends on, and the one the old model could never show.
    assert_eq!(conversion(0b000, 0b001, [3.0, 3.0, 0.0, 0.0]).1, 0);
    assert_eq!(
        conversion(0b000, 0b001, [3.0, 3.0, 0.0, 0.0]).0,
        counts(3.0)
    );
    // …and a NEGATIVE difference is a negative count.
    assert_eq!(
        conversion(0b000, 0b001, [0.5, 2.0, 0.0, 0.0]).1,
        counts(-1.5)
    );
}

#[test]
fn a_read_past_the_word_answers_ff_instead_of_repeating_the_word() {
    // The pointer does not auto-increment, so a master that keeps clocking is
    // reading nothing. The hand model handed it the same word again forever.
    let steps = script([
        write_reg(0x01, &{
            let c = config(0b100, 0b001);
            [(c >> 8) as u8, (c & 0xFF) as u8]
        }),
        read_reg(0x00, 6),
    ]);
    let (old, new) = both([2.048, 0.0, 0.0, 0.0], &steps);
    assert_eq!(old, vec![0x40, 0x00, 0x40, 0x00, 0x40, 0x00], "repeated");
    assert_eq!(
        new,
        vec![0x40, 0x00, 0xFF, 0xFF, 0xFF, 0xFF],
        "then nothing"
    );
    assert_eq!(&old[0..2], &new[0..2], "the word itself is identical");
}
