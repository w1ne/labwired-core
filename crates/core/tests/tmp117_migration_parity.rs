// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! TMP117: the declarative descriptor against the hand-written model it
//! replaces.
//!
//! The deleted `components/tmp117.rs` is reproduced below as [`legacy`] — the
//! wire behaviour only — and both models are driven through the SAME I²C
//! script. Transcripts that must be identical are asserted equal byte for
//! byte; the ones that must differ are asserted as differences, by name.
//!
//! Held identical:
//!   * the whole pointer map and the zero word an undecoded pointer answers;
//!   * `TEMP_RESULT` as a two's-complement 7.8125 m°C count, swept over the
//!     entire −55..150 °C channel range — the encode MULTIPLIES by 128 where
//!     the hand model DIVIDED by 0.0078125, and the sweep is what proves the
//!     two never round apart;
//!   * the 16-bit MSB-first write of every read/write register and its
//!     read-back, including the POR limits and the DATA_READY bit a write
//!     cannot touch;
//!   * writes to the read-only `TEMP_RESULT` and `DEVICE_ID` being dropped.
//!
//! Deliberately DIFFERENT (each with its own test below):
//!   * DATA_READY (CONFIGURATION D13) reads SET from power-on and stays set.
//!     The hand model left it clear until a host stimulus arrived and cleared
//!     it on every TEMP_RESULT read, so firmware that polls the flag before
//!     driving a stimulus — TI's own `tmp117_data_ready()`, SparkFun's
//!     `dataReady()` — spun forever.
//!   * a read that runs PAST the pointed 16-bit word answers 0xFF instead of
//!     repeating the LSB.
//!
//! Scripts are driven through Phase A's shared harness
//! (`tests/common/transcript.rs`), the same one the byte-parity ratchet uses.

mod common;

use common::transcript::{read_reg, run_i2c, script, write_reg, Step};
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::sim_input::SimInput;

const ADDR: u8 = 0x48;

/// CONFIGURATION D13.
const DATA_READY: u16 = 1 << 13;

// ─── the model this descriptor replaces ────────────────────────────────────

/// `crates/core/src/peripherals/components/tmp117.rs` at `origin/main`, wire
/// behaviour only. This is the ONLY place the old behaviour survives.
mod legacy {
    use labwired_core::peripherals::i2c::I2cDevice;

    const LSB_C: f64 = 0.0078125;
    const CFG_DATA_READY: u16 = 1 << 13;
    const CFG_RESET: u16 = 0x0220;

    fn celsius_to_raw(t_c: f64) -> i16 {
        (t_c / LSB_C)
            .round()
            .clamp(i16::MIN as f64, i16::MAX as f64) as i16
    }

    pub struct Tmp117 {
        address: u8,
        temp_raw: i16,
        config: u16,
        t_high: i16,
        t_low: i16,
        eeprom_ul: u16,
        eeprom1: u16,
        eeprom2: u16,
        eeprom3: u16,
        temp_offset: u16,
        data_ready: bool,
        pointer: u8,
        read_phase: u8,
        writes_since_start: u32,
        pending_msb: u8,
    }

    impl Tmp117 {
        pub fn new(address: u8) -> Self {
            Self {
                address,
                temp_raw: 0,
                config: CFG_RESET,
                t_high: celsius_to_raw(192.0),
                t_low: celsius_to_raw(-256.0),
                eeprom_ul: 0,
                eeprom1: 0,
                eeprom2: 0,
                eeprom3: 0,
                temp_offset: 0,
                data_ready: false,
                pointer: 0x00,
                read_phase: 0,
                writes_since_start: 0,
                pending_msb: 0,
            }
        }

        /// The old `SimInput::set_input("temperature", …)`, verbatim: it both
        /// converts the measurement AND raises DATA_READY, which is the
        /// behaviour this migration changes.
        pub fn set_temperature(&mut self, c: f64) {
            self.temp_raw = celsius_to_raw(c);
            self.data_ready = true;
        }

        fn reg_value(&self, ptr: u8) -> u16 {
            match ptr {
                0x00 => self.temp_raw as u16,
                0x01 => {
                    let mut c = self.config & !CFG_DATA_READY;
                    if self.data_ready {
                        c |= CFG_DATA_READY;
                    }
                    c
                }
                0x02 => self.t_high as u16,
                0x03 => self.t_low as u16,
                0x04 => self.eeprom_ul,
                0x05 => self.eeprom1,
                0x06 => self.eeprom2,
                0x07 => self.temp_offset,
                0x08 => self.eeprom3,
                0x0F => 0x0117,
                _ => 0,
            }
        }

        fn store_reg(&mut self, ptr: u8, value: u16) {
            match ptr {
                0x01 => self.config = value & !CFG_DATA_READY,
                0x02 => self.t_high = value as i16,
                0x03 => self.t_low = value as i16,
                0x04 => self.eeprom_ul = value,
                0x05 => self.eeprom1 = value,
                0x06 => self.eeprom2 = value,
                0x07 => self.temp_offset = value,
                0x08 => self.eeprom3 = value,
                _ => {}
            }
        }
    }

    impl I2cDevice for Tmp117 {
        fn address(&self) -> u8 {
            self.address
        }

        fn write(&mut self, data: u8) {
            match self.writes_since_start {
                0 => {
                    self.pointer = data;
                    self.read_phase = 0;
                }
                1 => self.pending_msb = data,
                2 => {
                    let value = ((self.pending_msb as u16) << 8) | data as u16;
                    self.store_reg(self.pointer, value);
                }
                _ => {}
            }
            self.writes_since_start = self.writes_since_start.saturating_add(1);
        }

        fn read(&mut self) -> u8 {
            let value = self.reg_value(self.pointer);
            if self.read_phase == 0 {
                self.read_phase = 1;
                (value >> 8) as u8
            } else {
                if self.pointer == 0x00 {
                    self.data_ready = false;
                }
                (value & 0xFF) as u8
            }
        }

        fn start(&mut self) {
            self.read_phase = 0;
            self.writes_since_start = 0;
        }
    }
}

// ─── the script vocabulary ─────────────────────────────────────────────────

fn declarative() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("tmp117")
        .expect("tmp117 descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("tmp117.yaml does not build")
}

/// Both models, with the temperature channel driven on each — the state every
/// transcript that is asserted IDENTICAL starts from.
fn both(temp_c: f64, steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy::Tmp117::new(ADDR);
    old.set_temperature(temp_c);
    let mut new = declarative();
    new.set_input("temperature", temp_c)
        .expect("temperature channel");
    (
        run_i2c(&mut old, steps).bytes,
        run_i2c(&mut new, steps).bytes,
    )
}

/// Both models straight out of reset, with NO stimulus driven. This is the
/// state the DATA_READY difference is visible in.
fn both_untouched(steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy::Tmp117::new(ADDR);
    let mut new = declarative();
    (
        run_i2c(&mut old, steps).bytes,
        run_i2c(&mut new, steps).bytes,
    )
}

fn assert_parity(name: &str, temp_c: f64, steps: &[Step<'_>]) -> Vec<u8> {
    let (old, new) = both(temp_c, steps);
    assert!(!old.is_empty(), "{name}: the script read no bytes at all");
    assert_eq!(old, new, "{name}: the YAML model changed the transcript");
    new
}

fn word(bytes: &[u8]) -> u16 {
    assert_eq!(bytes.len(), 2, "a TMP117 register is one 16-bit word");
    (u16::from(bytes[0]) << 8) | u16::from(bytes[1])
}

// ─── held identical ────────────────────────────────────────────────────────

#[test]
fn the_whole_pointer_map_reads_identically() {
    // Every pointer the old model decoded, plus the gaps between them and the
    // ones past the end of the map. The gaps matter: the old model answered a
    // zero WORD for them, and the descriptor has to answer the same.
    //
    // CONFIGURATION (0x01) is walked by its own tests below, not here: this
    // script reads TEMP_RESULT first, which is exactly the event that cleared
    // DATA_READY in the model being replaced, so 0x01 is the one pointer the
    // two deliberately disagree on. Skipped by NAME with that reason rather
    // than by trimming the range, which would have hidden the gaps too.
    let ptrs: Vec<u8> = (0x00u8..=0x1F).filter(|p| *p != 0x01).collect();
    let steps = script(ptrs.iter().map(|&p| read_reg(p, 2)));
    let bytes = assert_parity("full map", 25.0, &steps);
    assert_eq!(bytes.len(), ptrs.len() * 2);
    let at = |ptr: u8| -> u16 {
        let i = ptrs.iter().position(|&p| p == ptr).expect("pointer walked");
        word(&bytes[i * 2..i * 2 + 2])
    };
    assert_eq!(at(0x00), 0x0C80, "25 °C");
    assert_eq!(at(0x02), 0x6000, "T_HIGH +192 °C");
    assert_eq!(at(0x03), 0x8000, "T_LOW −256 °C");
    assert_eq!(at(0x0F), 0x0117, "DEVICE_ID");
    assert_eq!(at(0x10), 0x0000, "past the map");
}

#[test]
fn the_temperature_word_matches_over_the_whole_channel_range() {
    // The descriptor MULTIPLIES by 128 where the hand model DIVIDED by
    // 0.0078125. Both are exact in binary (0.0078125 is 2⁻⁷), but "should be
    // the same double" is an argument, not a measurement — so every count in
    // the declared −55..150 °C range is compared, 26241 of them, in 1/128 °C
    // steps rather than at a handful of convenient temperatures.
    for count in -7040i32..=19200 {
        let c = f64::from(count) / 128.0;
        let (old, new) = both(c, &read_reg(0x00, 2));
        assert_eq!(old, new, "{c} °C: the encoded word differs");
        assert_eq!(word(&new), count as u16, "{c} °C is not {count} counts");
    }
}

#[test]
fn a_negative_temperature_is_two_s_complement() {
    // The sweep above would pass on a model that simply agreed on garbage, so
    // the datasheet vector is asserted on its own: −25 °C is −3200 counts.
    let bytes = assert_parity("negative", -25.0, &read_reg(0x00, 2));
    assert_eq!(word(&bytes), (-3200i16) as u16);
    assert_eq!(bytes, vec![0xF3, 0x80]);
}

#[test]
fn a_sixteen_bit_write_round_trips_through_every_writable_register() {
    // The pointer/MSB/LSB write phase, and the read-back of it, for each of
    // the seven registers the datasheet marks read/write.
    for (ptr, name) in [
        (0x01u8, "CONFIGURATION"),
        (0x02, "T_HIGH_LIMIT"),
        (0x03, "T_LOW_LIMIT"),
        (0x04, "EEPROM_UL"),
        (0x05, "EEPROM1"),
        (0x06, "EEPROM2"),
        (0x07, "TEMP_OFFSET"),
        (0x08, "EEPROM3"),
    ] {
        let steps = script([write_reg(ptr, &[0x5A, 0xA5]), read_reg(ptr, 2)]);
        let bytes = assert_parity(name, 25.0, &steps);
        let expect = if ptr == 0x01 {
            0x5AA5 | DATA_READY
        } else {
            0x5AA5
        };
        assert_eq!(word(&bytes), expect, "{name} did not read back its write");
    }
}

#[test]
fn a_write_cannot_touch_data_ready() {
    // D13 is silicon's: §7.6.2 marks it read-only, and both models keep it out
    // of the store. Writing 1s everywhere must not change the answer, and
    // writing 0s everywhere must not clear it.
    for pattern in [[0xFFu8, 0xFF], [0x00, 0x00]] {
        let steps = script([write_reg(0x01, &pattern), read_reg(0x01, 2)]);
        let bytes = assert_parity("config write", 25.0, &steps);
        assert_eq!(
            word(&bytes) & DATA_READY,
            DATA_READY,
            "writing {pattern:02X?} moved DATA_READY"
        );
    }
}

#[test]
fn a_write_to_a_read_only_register_is_dropped() {
    // TEMP_RESULT and DEVICE_ID are read-only. A driver that writes them must
    // read back the measurement / the identity, not what it wrote.
    let steps = script([
        write_reg(0x00, &[0xDE, 0xAD]),
        read_reg(0x00, 2),
        write_reg(0x0F, &[0xDE, 0xAD]),
        read_reg(0x0F, 2),
    ]);
    let bytes = assert_parity("read-only writes", 25.0, &steps);
    assert_eq!(word(&bytes[0..2]), 0x0C80, "TEMP_RESULT is still 25 °C");
    assert_eq!(
        word(&bytes[2..4]),
        0x0117,
        "DEVICE_ID is still the identity"
    );
}

#[test]
fn a_short_write_lands_nowhere_in_either_model() {
    // A frame that ends after the MSB stores nothing: the old model latched
    // the byte and waited for an LSB that never came, and the engine stores a
    // register write only at its full declared width. Same answer, different
    // reason — worth pinning, because a half-word write that DID land would be
    // a silent corruption a driver could not see.
    let steps = script([write_reg(0x05, &[0x5A]), read_reg(0x05, 2)]);
    let bytes = assert_parity("short write", 25.0, &steps);
    assert_eq!(word(&bytes), 0x0000, "the MSB-only write is absorbed");
}

#[test]
fn the_pointer_survives_the_repeated_start_of_a_read() {
    // THE transaction every driver issues: write the pointer, repeated START,
    // read the word. It works only because the pointer is not cleared by the
    // START that frames the read phase.
    let bytes = assert_parity("pointed read", 100.0, &read_reg(0x00, 2));
    assert_eq!(word(&bytes), 12800, "100 °C = 12800 counts");
}

// ─── deliberately different ────────────────────────────────────────────────

#[test]
fn data_ready_reads_set_from_power_on_instead_of_waiting_for_a_stimulus() {
    // THE deliberate change. The hand model left DATA_READY clear until a host
    // drove `set_input("temperature", …)`, so the flag reported "the simulator
    // was poked" rather than "the part converted" — and firmware that polls it
    // before any stimulus arrives (TI's `tmp117_data_ready()`, SparkFun's
    // `dataReady()`) spun forever against a part that, per §7.6.2, powers up
    // in continuous-conversion mode (MOD = 00) and is therefore converting
    // before firmware reaches the bus.
    let (old, new) = both_untouched(&read_reg(0x01, 2));

    assert_eq!(
        word(&old),
        0x0220,
        "the model this replaces powered up idle"
    );
    assert_eq!(word(&new), 0x2220, "the descriptor powers up converting");
    assert_eq!(word(&old) & DATA_READY, 0, "…with the flag clear");
    assert_eq!(word(&new) & DATA_READY, DATA_READY, "…and here, set");
    assert_ne!(old, new, "this difference is the point of the change");

    // Everything else in the word is untouched: MOD / CONV / AVG still read
    // the datasheet's POR configuration, so only D13 moved.
    assert_eq!(word(&new) & !DATA_READY, word(&old), "only D13 differs");
}

#[test]
fn data_ready_no_longer_clears_when_temp_result_is_read() {
    // The other half of the same change, and the part that is NOT an
    // improvement: §7.6.2 clears D13 on a read of TEMP_RESULT, and the hand
    // model did. Reproducing it needs a conversion timer that SETS the bit on
    // its deadline and a cross-register clear-on-read that zeroes one bit of
    // ANOTHER register — `on_read: clear` zeroes the read register's own word,
    // so Tier 1 cannot express it. Recorded here so the gap is a line someone
    // wrote, not a silence.
    let steps = script([read_reg(0x00, 2), read_reg(0x01, 2)]);
    let (old, new) = both(25.0, &steps);

    assert_eq!(
        &old[0..2],
        &new[0..2],
        "the measurement itself is identical"
    );
    assert_eq!(word(&old[2..4]) & DATA_READY, 0, "the old model cleared it");
    assert_eq!(
        word(&new[2..4]) & DATA_READY,
        DATA_READY,
        "the descriptor holds it set"
    );
}

#[test]
fn a_read_past_the_word_answers_ff_instead_of_repeating_the_lsb() {
    // The pointer does not auto-increment, so a master that keeps clocking
    // past the 16-bit word is reading nothing. The hand model handed it the
    // LSB again, forever; the engine hands it 0xFF, which is what an
    // un-driven SDA line reads and what every other non-incrementing
    // declarative device already answers. A driver that relies on the repeat
    // is reading a register the part never sent.
    let (old, new) = both(25.0, &read_reg(0x00, 5));

    assert_eq!(old, vec![0x0C, 0x80, 0x80, 0x80, 0x80], "LSB repeated");
    assert_eq!(new, vec![0x0C, 0x80, 0xFF, 0xFF, 0xFF], "then nothing");
    assert_eq!(&old[0..2], &new[0..2], "the word itself is identical");
}

#[test]
fn an_over_long_write_stores_the_word_and_drops_the_tail_in_both() {
    // Four data bytes into a 16-bit register. Both models take the first two
    // as the word and ignore what follows — the old one because its write
    // counter stopped mattering after the third byte, the engine because a
    // register write lands at exactly the declared width and every later byte
    // of the same frame misses it. Same answer, different reason, which is
    // precisely the kind of coincidence worth pinning: a driver with an
    // off-by-one length is not silently writing a different register now than
    // it was before.
    let steps = script([
        write_reg(0x05, &[0x11, 0x22, 0x33, 0x44]),
        read_reg(0x05, 2),
    ]);
    let bytes = assert_parity("over-long write", 25.0, &steps);
    assert_eq!(
        word(&bytes),
        0x1122,
        "the first two data bytes are the word"
    );
}
