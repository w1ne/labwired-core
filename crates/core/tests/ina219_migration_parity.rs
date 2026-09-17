// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! INA219: the declarative descriptor against the hand-written model it
//! replaces.
//!
//! The deleted `components/ina219.rs` is reproduced below as [`legacy`] — the
//! wire behaviour only — and both models are driven through the SAME I²C
//! script. Transcripts that must be identical are asserted equal byte for
//! byte; the ones that must differ are asserted as differences, by name.
//!
//! Held identical:
//!   * the whole pointer map and the zero word an undecoded pointer answers;
//!   * SHUNT_VOLTAGE and CURRENT over EVERY whole milliamp of the declared
//!     ±3.2 A range — the counts the old model could resolve at all;
//!   * BUS_VOLTAGE on every 4 mV step of the declared 0..32 V range, all 8001
//!     of them, including the CNVR bit below the count;
//!   * the 16-bit MSB-first write and read-back of CONFIGURATION and
//!     CALIBRATION, and writes to the four read-only registers being dropped.
//!
//! Deliberately DIFFERENT (each with its own test below):
//!   * BUS_VOLTAGE rounds to the nearest 4 mV LSB where the model TRUNCATED;
//!   * POWER rounds once where the model truncated an intermediate to whole
//!     milliwatts before halving it into the 2 mW register;
//!   * CURRENT / SHUNT keep the 0.1 mA the register resolves, where the model
//!     quantised the stimulus to whole milliamps first;
//!   * a read that runs PAST the pointed 16-bit word answers 0xFF instead of
//!     repeating the word forever.
//!
//! The POWER register is why this port waited for `behavior.derived`: §8.5.4
//! makes it the PRODUCT of the two stimulus channels, and `source:` takes one
//! key. `power_follows_both_channels` is the test that the product is live on
//! both of them rather than a constant that happens to look right.

mod common;

use common::transcript::{read_reg, run_i2c, script, write_reg, Step};
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::sim_input::SimInput;

const ADDR: u8 = 0x40;

// ─── the model this descriptor replaces ────────────────────────────────────

/// `crates/core/src/peripherals/components/ina219.rs` at `origin/main`, wire
/// behaviour only. This is the ONLY place the old behaviour survives.
mod legacy {
    use labwired_core::peripherals::i2c::I2cDevice;

    const DEFAULT_CONFIG: u16 = 0x399F;
    const DEFAULT_CAL: u16 = 4096;

    pub struct Ina219 {
        current_register: u8,
        register_address_written: bool,
        write_high: Option<u8>,
        read_low_pending: Option<u8>,
        config: u16,
        calibration: u16,
        bus_mv: u16,
        current_ma: i16,
    }

    impl Ina219 {
        pub fn new() -> Self {
            Self {
                current_register: 0x00,
                register_address_written: false,
                write_high: None,
                read_low_pending: None,
                config: DEFAULT_CONFIG,
                calibration: DEFAULT_CAL,
                bus_mv: 3300,
                current_ma: 0,
            }
        }

        /// The old `SimInput::set_input`, verbatim — including the quantisation
        /// of the stimulus to whole millivolts / whole milliamps.
        pub fn set_input(&mut self, key: &str, value: f64) {
            match key {
                "bus_voltage" => {
                    self.bus_mv = ((value * 1000.0).round().clamp(0.0, 32_000.0) as u16).min(32_000)
                }
                "current" => {
                    self.current_ma = (value * 1000.0).round().clamp(-32_000.0, 32_000.0) as i16
                }
                other => panic!("unknown legacy channel {other}"),
            }
        }

        fn bus_register(&self) -> u16 {
            let counts = (self.bus_mv / 4).min(0x1FFF);
            (counts << 3) | 0b10
        }

        fn shunt_register(&self) -> u16 {
            let vshunt_uv = i32::from(self.current_ma) * 100;
            let counts = (vshunt_uv / 10).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            counts as u16
        }

        fn current_register_value(&self) -> u16 {
            let counts = (f64::from(self.current_ma) / 0.1).round() as i32;
            counts.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16 as u16
        }

        fn power_register(&self) -> u16 {
            let power_mw = (i32::from(self.bus_mv) * i32::from(self.current_ma.abs())) / 1000;
            let counts = (power_mw as f64 / 2.0).round() as i32;
            counts.clamp(0, 0xFFFF) as u16
        }

        fn read_register_u16(&self, reg: u8) -> u16 {
            match reg {
                0x00 => self.config,
                0x01 => self.shunt_register(),
                0x02 => self.bus_register(),
                0x03 => self.power_register(),
                0x04 => self.current_register_value(),
                0x05 => self.calibration,
                _ => 0,
            }
        }

        fn write_register_u16(&mut self, reg: u8, value: u16) {
            match reg {
                0x00 => self.config = value,
                0x05 => self.calibration = value,
                _ => {}
            }
        }
    }

    impl I2cDevice for Ina219 {
        fn address(&self) -> u8 {
            0x40
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

        fn start(&mut self) {
            self.register_address_written = false;
            self.write_high = None;
            self.read_low_pending = None;
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
    let yaml = labwired_config::embedded_device_yaml("ina219")
        .expect("ina219 descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("ina219.yaml does not build")
}

/// Both models, with both stimulus channels driven — the state every transcript
/// asserted IDENTICAL starts from.
fn both(bus_v: f64, current_a: f64, steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy::Ina219::new();
    old.set_input("bus_voltage", bus_v);
    old.set_input("current", current_a);
    let mut new = declarative();
    new.set_input("bus_voltage", bus_v).expect("bus_voltage");
    new.set_input("current", current_a).expect("current");
    (
        run_i2c(&mut old, steps).bytes,
        run_i2c(&mut new, steps).bytes,
    )
}

fn assert_parity(name: &str, bus_v: f64, current_a: f64, steps: &[Step<'_>]) -> Vec<u8> {
    let (old, new) = both(bus_v, current_a, steps);
    assert!(!old.is_empty(), "{name}: the script read no bytes at all");
    assert_eq!(old, new, "{name}: the YAML model changed the transcript");
    new
}

fn word(bytes: &[u8]) -> u16 {
    assert_eq!(bytes.len(), 2, "an INA219 register is one 16-bit word");
    (u16::from(bytes[0]) << 8) | u16::from(bytes[1])
}

/// One register, both models, at one operating point.
fn reg_words(bus_v: f64, current_a: f64, ptr: u8) -> (u16, u16) {
    let (old, new) = both(bus_v, current_a, &read_reg(ptr, 2));
    (word(&old), word(&new))
}

// ─── held identical ────────────────────────────────────────────────────────

#[test]
fn the_whole_pointer_map_reads_identically() {
    // Every pointer the old model decoded, the gaps around them and the ones
    // past the end of the map — the gaps matter, because the old model
    // answered a zero WORD for them and the descriptor has to answer the same.
    // 3.3 V is on the 4 mV grid (825 LSBs) and 100 mA is a whole milliamp, so
    // this point is one where the quantisation differences below cannot hide.
    let ptrs: Vec<u8> = (0x00u8..=0x1F).collect();
    let steps = script(ptrs.iter().map(|&p| read_reg(p, 2)));
    let bytes = assert_parity("full map", 3.3, 0.1, &steps);
    assert_eq!(bytes.len(), ptrs.len() * 2);
    let at = |ptr: u8| word(&bytes[usize::from(ptr) * 2..usize::from(ptr) * 2 + 2]);
    assert_eq!(at(0x00), 0x399F, "CONFIGURATION power-on");
    assert_eq!(at(0x01), 1000, "100 mA over 0.1 Ω = 10 mV = 1000 × 10 µV");
    assert_eq!(at(0x02), (825 << 3) | 0b10, "3.3 V = 825 × 4 mV, CNVR set");
    assert_eq!(at(0x03), 165, "3.3 V × 0.1 A = 330 mW = 165 × 2 mW");
    assert_eq!(at(0x04), 1000, "100 mA at 0.1 mA/LSB");
    assert_eq!(at(0x05), 4096, "CALIBRATION seeded to the 32 V / 2 A point");
    assert_eq!(at(0x06), 0x0000, "past the map");
    assert_eq!(at(0x1F), 0x0000, "still past the map");
}

#[test]
fn current_and_shunt_match_over_every_whole_milliamp() {
    // The hand model quantised the stimulus to whole milliamps, so those are
    // the only currents on which the two CAN agree — and on all 6401 of them
    // they do, for both registers, which is what makes the sub-milliamp
    // difference below a resolution change and not a scaling error.
    for ma in -3200i32..=3200 {
        let a = f64::from(ma) / 1000.0;
        let (old, new) = both(3.3, a, &script([read_reg(0x01, 2), read_reg(0x04, 2)]));
        assert_eq!(old, new, "{ma} mA: SHUNT/CURRENT differ");
        assert_eq!(word(&new[0..2]) as i16, (ma * 10) as i16, "{ma} mA shunt");
        assert_eq!(word(&new[2..4]) as i16, (ma * 10) as i16, "{ma} mA current");
    }
}

#[test]
fn bus_voltage_matches_on_every_four_millivolt_step() {
    // 8001 steps — the whole declared 0..32 V range at the register's own LSB.
    for mv in (0..=32_000).step_by(4) {
        let v = f64::from(mv) / 1000.0;
        let (old, new) = reg_words(v, 0.0, 0x02);
        assert_eq!(old, new, "{mv} mV: the BUS_VOLTAGE word differs");
        assert_eq!(new, ((mv as u16 / 4) << 3) | 0b10, "{mv} mV counts");
    }
}

#[test]
fn a_sixteen_bit_write_round_trips_through_both_writable_registers() {
    for (ptr, name) in [(0x00u8, "CONFIGURATION"), (0x05, "CALIBRATION")] {
        let steps = script([write_reg(ptr, &[0x5A, 0xA5]), read_reg(ptr, 2)]);
        let bytes = assert_parity(name, 3.3, 0.1, &steps);
        assert_eq!(word(&bytes), 0x5AA5, "{name} did not read back its write");
    }
}

#[test]
fn a_write_to_a_read_only_register_is_dropped() {
    // SHUNT / BUS / POWER / CURRENT are measurements. A driver that writes one
    // must read back the measurement, not what it wrote.
    let steps = script(
        [0x01u8, 0x02, 0x03, 0x04]
            .iter()
            .map(|&p| script([write_reg(p, &[0xDE, 0xAD]), read_reg(p, 2)])),
    );
    let bytes = assert_parity("read-only writes", 3.3, 0.1, &steps);
    assert_eq!(word(&bytes[0..2]), 1000, "SHUNT is still the measurement");
    assert_eq!(word(&bytes[2..4]), (825 << 3) | 0b10, "BUS unchanged");
    assert_eq!(word(&bytes[4..6]), 165, "POWER unchanged");
    assert_eq!(word(&bytes[6..8]), 1000, "CURRENT unchanged");
}

#[test]
fn the_pointer_survives_the_repeated_start_of_a_read() {
    // THE transaction every driver issues. It works only because the pointer is
    // not cleared by the START that frames the read phase.
    let bytes = assert_parity("pointed read", 12.0, 1.5, &read_reg(0x04, 2));
    assert_eq!(word(&bytes) as i16, 15_000, "1.5 A at 0.1 mA/LSB");
}

// ─── the derived source, proved live ───────────────────────────────────────

#[test]
fn power_follows_both_channels() {
    // The point of `behavior.derived`: POWER is not a constant and not a
    // function of one channel. Move either input and it moves; zero either and
    // it is zero; flip the current's SIGN and it does not move, because §8.5.4
    // makes the register the magnitude of the power and the CURRENT register
    // carries the direction.
    let p = |v: f64, a: f64| reg_words(v, a, 0x03).1;
    assert_eq!(p(12.0, 1.0), 6000, "12 W = 6000 × 2 mW");
    assert_eq!(
        p(24.0, 1.0),
        12_000,
        "doubling the voltage doubles the power"
    );
    assert_eq!(p(12.0, 2.0), 12_000, "doubling the current does too");
    assert_eq!(
        p(12.0, -1.0),
        6000,
        "the sign of the current does not change it"
    );
    assert_eq!(p(0.0, 2.0), 0, "no voltage, no power");
    assert_eq!(p(24.0, 0.0), 0, "no current, no power");
    // …and the CURRENT register is where the direction went.
    assert_eq!(reg_words(12.0, -1.0, 0x04).1 as i16, -10_000);
}

#[test]
fn power_is_still_the_old_models_answer_wherever_its_truncation_did_not_bite() {
    // A power monitor's POWER register is the one a driver actually prints, so
    // the agreement is asserted over a grid rather than at one convenient
    // point: every 7th millivolt against every 13th milliamp.
    let mut compared = 0usize;
    for mv in (0..=32_000).step_by(7 * 40) {
        for ma in (-3200..=3200).step_by(13 * 8) {
            let (old, new) = reg_words(f64::from(mv) / 1000.0, f64::from(ma) / 1000.0, 0x03);
            assert!(
                old.abs_diff(new) <= 1,
                "{mv} mV × {ma} mA: POWER differs by more than one 2 mW LSB ({old} vs {new})"
            );
            compared += 1;
        }
    }
    assert!(compared > 4000, "the grid collapsed to {compared} points");
}

// ─── deliberately different ────────────────────────────────────────────────

#[test]
fn bus_voltage_rounds_to_the_nearest_lsb_instead_of_truncating() {
    // THE first deliberate change. The hand model held the bus voltage in an
    // integer millivolt field and divided it by 4, so 3.303 V reported 3.300 V
    // — an error of up to a whole LSB, always downward. The descriptor rounds,
    // which is what `encode` does for every part in the engine and what halves
    // the quantisation error. Measured over every millivolt of the range:
    // nothing moves by more than one LSB, and the direction is always the
    // model's answer being LOW.
    let mut differed = 0usize;
    let mut worst = 0u16;
    for mv in 0..=32_000u32 {
        let (old, new) = reg_words(f64::from(mv) / 1000.0, 0.0, 0x02);
        let (old_counts, new_counts) = (old >> 3, new >> 3);
        worst = worst.max(old_counts.abs_diff(new_counts));
        if old != new {
            differed += 1;
            assert!(new_counts > old_counts, "{mv} mV: the descriptor read LOW");
        }
    }
    assert_eq!(worst, 1, "no reading may move by more than one 4 mV LSB");
    // Half the millivolts in the range are more than 2 mV above their truncated
    // LSB, and those are exactly the ones that round up.
    assert!(
        (15_000..17_000).contains(&differed),
        "{differed} of 32001 millivolts moved — expected about half"
    );
    // The named vector: 3.303 V is 825.75 LSBs.
    assert_eq!(
        reg_words(3.303, 0.0, 0x02).0 >> 3,
        825,
        "the model truncated"
    );
    assert_eq!(
        reg_words(3.303, 0.0, 0x02).1 >> 3,
        826,
        "the descriptor rounds"
    );
}

#[test]
fn current_keeps_the_tenth_of_a_milliamp_the_register_resolves() {
    // §8.5.1 with CALIBRATION = 4096 over a 0.1 Ω shunt makes current_LSB
    // 100 µA, so the CURRENT register resolves a tenth of a milliamp. The hand
    // model stored the stimulus in an integer MILLIAMP field first and threw
    // that tenth away before it ever reached the encoding.
    let (old, new) = reg_words(3.3, 0.100_05, 0x04);
    assert_eq!(old, 1000, "the model quantised 100.05 mA to 100 mA");
    assert_eq!(
        new, 1001,
        "the descriptor keeps the 0.1 mA the register has"
    );
    // The same tenth in the SHUNT register, which shares the encoding.
    assert_eq!(reg_words(3.3, 0.100_05, 0x01).0, 1000);
    assert_eq!(reg_words(3.3, 0.100_05, 0x01).1, 1001);
}

#[test]
fn power_rounds_once_instead_of_truncating_an_intermediate_milliwatt() {
    // The hand model computed `bus_mV × |I_mA| / 1000` in INTEGER milliwatts
    // and then halved that into the 2 mW register, so the intermediate was
    // truncated before the only rounding happened. The descriptor evaluates
    // §8.5.4's product once and rounds once. 0.175 V × 2.52 A is the first
    // point on the grid where the two orders disagree.
    let (old, new) = reg_words(0.175, 2.52, 0x03);
    assert_eq!(old, 221, "441 mW truncated from 441.0 mW, then halved");
    assert_eq!(new, 220, "441.0 mW halved once: 220.5 → 220 counts");
    assert_eq!(old.abs_diff(new), 1, "one 2 mW LSB, no more");
}

#[test]
fn a_read_past_the_word_answers_ff_instead_of_repeating_the_word() {
    // The pointer does not auto-increment, so a master that keeps clocking past
    // the 16-bit word is reading nothing. The hand model handed it the same two
    // bytes again, forever; the engine hands it 0xFF, which is what an undriven
    // SDA line reads and what every other non-incrementing declarative device
    // already answers.
    let (old, new) = both(3.3, 0.1, &read_reg(0x04, 6));
    assert_eq!(old, vec![0x03, 0xE8, 0x03, 0xE8, 0x03, 0xE8], "repeated");
    assert_eq!(
        new,
        vec![0x03, 0xE8, 0xFF, 0xFF, 0xFF, 0xFF],
        "then nothing"
    );
    assert_eq!(&old[0..2], &new[0..2], "the word itself is identical");
}
