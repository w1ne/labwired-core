// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! VL53L1X: the declarative descriptor against the hand-written model it
//! replaces.
//!
//! The deleted `components/vl53l1x.rs` is reproduced below as [`legacy`] — the
//! wire behaviour only — and both models are driven through the SAME I²C
//! script. Transcripts that must be identical are asserted equal byte for
//! byte; the one that must differ is asserted as a difference, by name.
//!
//! Held identical:
//!   * the 16-bit register pointer: two address bytes high-first, latched
//!     across the repeated START, wrapping at 0xFFFF;
//!   * `IDENTIFICATION__MODEL_ID` = 0xEACC, read as the 16-bit word the Pololu
//!     `init()` compares and as its two bytes separately;
//!   * the `GPIO__TIO_HV_STATUS` data-ready line: odd before ranging, even
//!     after `startContinuous()`, odd again after `stopContinuous()`;
//!   * the whole 17-byte `readResults()` burst, including the zero bytes
//!     between the three that matter, at every integer millimetre of the
//!     declared 0..4000 channel range (`raw_range_matches_at_every_millimetre`);
//!   * `init()`'s 8-byte block write at 0x002D, which walks straight over
//!     `GPIO__TIO_HV_STATUS` and must NOT disturb it;
//!   * an undeclared address answering 0x00 rather than open bus.
//!
//! Deliberately DIFFERENT (each with its own test below):
//!   * where the millimetre channel is ROUNDED. The hand model rounded the
//!     question (`set_input` stored `value.round() as u16`); the descriptor
//!     rounds the answer (the encode's `round: nearest`). They agree on every
//!     integer millimetre and can differ by one raw count on a fractional one.
//!   * `SYSTEM__MODE_START` READS BACK. It is a read/write register on silicon
//!     (ST UM2356 §2.3); the hand model had no read arm for it, so it accepted
//!     the write and then answered 0x00 for ever. The Pololu driver never
//!     reads the address, so nothing depends on either answer.
//!
//! Scripts are driven through the shared harness (`tests/common/transcript.rs`),
//! the same one the byte-parity ratchet uses.

use crate::common;
use common::transcript::{run_i2c, script, Step};
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;

const ADDR: u8 = 0x29;

const REG_GPIO_TIO_HV_STATUS: u16 = 0x0031;
const REG_SYSTEM_MODE_START: u16 = 0x0087;
const REG_RESULT_RANGE_STATUS: u16 = 0x0089;
const REG_RESULT_STREAM_COUNT: u16 = 0x008B;
const REG_IDENTIFICATION_MODEL_ID: u16 = 0x010F;

// ─── the model this descriptor replaces ────────────────────────────────────

/// `crates/core/src/peripherals/components/vl53l1x.rs` at `origin/main`, wire
/// behaviour only. This is the ONLY place the old behaviour survives.
mod legacy {
    use labwired_core::peripherals::i2c::I2cDevice;

    const REG_GPIO_TIO_HV_STATUS: u16 = 0x0031;
    const REG_SYSTEM_INTERRUPT_CLEAR: u16 = 0x0086;
    const REG_SYSTEM_MODE_START: u16 = 0x0087;
    const REG_RESULT_RANGE_STATUS: u16 = 0x0089;
    const REG_IDENTIFICATION_MODEL_ID: u16 = 0x010F;
    const MODEL_ID: u16 = 0xEACC;
    const RANGE_STATUS_VALID: u8 = 9;

    pub struct Vl53l1x {
        address: u8,
        reg_ptr: u16,
        addr_bytes: u8,
        distance_mm: u16,
        ranging: bool,
    }

    impl Vl53l1x {
        pub fn new(address: u8) -> Self {
            Self {
                address,
                reg_ptr: 0,
                addr_bytes: 0,
                distance_mm: 500,
                ranging: false,
            }
        }

        /// The old `SimInput::set_input("distance", …)`, verbatim: it rounds the
        /// QUESTION to a whole millimetre before anything else happens, which
        /// is the one behaviour this migration changes.
        pub fn set_distance_mm(&mut self, mm: u16) {
            self.distance_mm = mm;
        }

        pub fn set_input_distance(&mut self, value: f64) {
            self.set_distance_mm(value.round() as u16);
        }

        fn raw_range(&self) -> u16 {
            let mm = self.distance_mm as u32;
            let num = mm.saturating_mul(0x0800).saturating_sub(0x0400);
            ((num + 2011 / 2) / 2011) as u16
        }

        fn read_register(&self, reg: u16) -> u8 {
            match reg {
                REG_IDENTIFICATION_MODEL_ID => (MODEL_ID >> 8) as u8,
                0x0110 => (MODEL_ID & 0xFF) as u8,
                REG_GPIO_TIO_HV_STATUS => {
                    if self.ranging {
                        0x00
                    } else {
                        0x01
                    }
                }
                REG_RESULT_RANGE_STATUS => RANGE_STATUS_VALID,
                0x008B => 1,
                0x0096 => (self.raw_range() >> 8) as u8,
                0x0097 => (self.raw_range() & 0xFF) as u8,
                _ => 0,
            }
        }

        fn write_register(&mut self, reg: u16, value: u8) {
            match reg {
                REG_SYSTEM_MODE_START => self.ranging = value != 0,
                REG_SYSTEM_INTERRUPT_CLEAR => {}
                _ => {}
            }
        }
    }

    impl I2cDevice for Vl53l1x {
        fn address(&self) -> u8 {
            self.address
        }

        fn read(&mut self) -> u8 {
            let val = self.read_register(self.reg_ptr);
            self.reg_ptr = self.reg_ptr.wrapping_add(1);
            val
        }

        fn write(&mut self, data: u8) {
            match self.addr_bytes {
                0 => {
                    self.reg_ptr = (data as u16) << 8;
                    self.addr_bytes = 1;
                }
                1 => {
                    self.reg_ptr = (self.reg_ptr & 0xFF00) | data as u16;
                    self.addr_bytes = 2;
                }
                _ => {
                    self.write_register(self.reg_ptr, data);
                    self.reg_ptr = self.reg_ptr.wrapping_add(1);
                }
            }
        }

        fn start(&mut self) {
            self.addr_bytes = 0;
        }

        fn stop(&mut self) {
            self.addr_bytes = 0;
        }
    }
}

// ─── the two models under test ─────────────────────────────────────────────

fn declarative() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("vl53l1x")
        .expect("vl53l1x descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("vl53l1x.yaml does not build")
}

fn legacy() -> legacy::Vl53l1x {
    legacy::Vl53l1x::new(ADDR)
}

// ─── 16-bit-pointer script sugar ───────────────────────────────────────────

/// Point at the 16-bit `reg` (high byte first), repeated-START, read `n`
/// bytes, STOP. This is `readResults()`'s framing.
fn read_reg16(reg: u16, n: usize) -> Vec<Step<'static>> {
    vec![
        Step::Start,
        Step::Write((reg >> 8) as u8),
        Step::Write((reg & 0xFF) as u8),
        Step::Start,
        Step::Read(n),
        Step::Stop,
    ]
}

/// Write `bytes` at the 16-bit `reg`, framed START … STOP.
fn write_reg16(reg: u16, bytes: &[u8]) -> Vec<Step<'static>> {
    let mut steps = vec![
        Step::Start,
        Step::Write((reg >> 8) as u8),
        Step::Write((reg & 0xFF) as u8),
    ];
    steps.extend(bytes.iter().map(|&b| Step::Write(b)));
    steps.push(Step::Stop);
    steps
}

/// Run one script against BOTH models and return `(legacy, declarative)`.
///
/// ⚠️ The same script object, not two scripts that look alike: a divergence
/// that lived in the script rather than in the models would compare equal and
/// prove nothing.
fn both(steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy();
    let mut new = declarative();
    (
        run_i2c(&mut old, steps).bytes,
        run_i2c(&mut new, steps).bytes,
    )
}

/// Drive the distance channel on both models at a whole millimetre, then run
/// `steps`. Kept separate from [`both`] because the legacy model's channel is
/// not reachable through `SimInput` here (the impl was deleted with the file),
/// so the stimulus is applied through its own setter.
fn both_at_mm(mm: f64, steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy();
    old.set_input_distance(mm);
    let mut new = declarative();
    new.seed_input("distance", mm);
    (
        run_i2c(&mut old, steps).bytes,
        run_i2c(&mut new, steps).bytes,
    )
}

// ─── identical: the identity word ──────────────────────────────────────────

#[test]
fn model_id_is_identical() {
    // `init()` does readReg16Bit(0x010F) and aborts unless it reads 0xEACC.
    let (old, new) = both(&read_reg16(REG_IDENTIFICATION_MODEL_ID, 2));
    assert_eq!(old, new, "MODEL_ID transcript diverged");
    assert_eq!(old, vec![0xEA, 0xCC], "MODEL_ID must read 0xEACC");

    // …and the low byte alone, which is a different address, not a second byte
    // of the same read.
    let (old, new) = both(&read_reg16(0x0110, 1));
    assert_eq!(old, new);
    assert_eq!(old, vec![0xCC]);
}

// ─── identical: the data-ready state machine ───────────────────────────────

#[test]
fn data_ready_follows_mode_start_identically() {
    let s = script([
        // dataReady() is (reg & 1) == 0 — so 0x01 means NOT ready.
        read_reg16(REG_GPIO_TIO_HV_STATUS, 1),
        // startContinuous() writes 0x40.
        write_reg16(REG_SYSTEM_MODE_START, &[0x40]),
        read_reg16(REG_GPIO_TIO_HV_STATUS, 1),
        // stopContinuous() writes 0x00.
        write_reg16(REG_SYSTEM_MODE_START, &[0x00]),
        read_reg16(REG_GPIO_TIO_HV_STATUS, 1),
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "data-ready transcript diverged");
    assert_eq!(
        old,
        vec![0x01, 0x00, 0x01],
        "not-ready, ready, not-ready — in that order"
    );
}

/// The NEGATIVE control for the rule above: a rule that fired on the wrong
/// event, or wrote the wrong register, would still leave the two reads in
/// `data_ready_follows_mode_start_identically` looking plausible. This drives
/// ranging ON and then reads a DIFFERENT register, proving the rule touched
/// the one register it names and nothing else.
#[test]
fn mode_start_changes_only_the_data_ready_register() {
    let s = script([
        write_reg16(REG_SYSTEM_MODE_START, &[0x40]),
        read_reg16(REG_RESULT_RANGE_STATUS, 1),
        read_reg16(REG_RESULT_STREAM_COUNT, 1),
        read_reg16(REG_IDENTIFICATION_MODEL_ID, 2),
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "a neighbouring register moved");
    assert_eq!(old, vec![0x09, 0x01, 0xEA, 0xCC]);
}

/// ⚠️ THE SECOND NAMED DIFFERENCE. `SYSTEM__MODE_START` (0x0087) is a
/// read/write register on silicon (ST UM2356 §2.3, the register list), and the
/// descriptor reads back what firmware wrote. The hand model had no read arm
/// for it at all, so it answered 0x00 for ever — a register that accepted a
/// write and then denied it.
///
/// Nothing depends on the change: the Pololu driver writes 0x40 / 0x00 and
/// never reads the address back. It is asserted here rather than left to be
/// discovered, and the two halves are asserted separately so a future edit that
/// silently removes the read-back fails rather than passing as "still equal".
#[test]
fn mode_start_reads_back_on_the_descriptor_and_not_on_the_model() {
    let s = script([
        write_reg16(REG_SYSTEM_MODE_START, &[0x40]),
        read_reg16(REG_SYSTEM_MODE_START, 1),
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, vec![0x00], "the deleted model denied its own write");
    assert_eq!(new, vec![0x40], "the descriptor reads the register back");
}

// ─── identical: the 17-byte result burst ───────────────────────────────────

/// The Pololu `read()` back-conversion, verbatim, so the test asserts against
/// the driver's arithmetic rather than against the model's.
fn pololu_range_mm(raw: u32) -> u32 {
    (raw * 2011 + 0x0400) / 0x0800
}

#[test]
fn result_burst_is_identical_and_decodes_to_the_stimulus() {
    let s = script([
        write_reg16(REG_SYSTEM_MODE_START, &[0x40]),
        read_reg16(REG_RESULT_RANGE_STATUS, 17),
    ]);
    let (old, new) = both_at_mm(742.0, &s);
    assert_eq!(old, new, "17-byte result burst diverged");
    // The mode write contributes nothing to a transcript; the burst is all of it.
    assert_eq!(old.len(), 17, "readResults() clocks 17 bytes");

    let burst = &old[..];
    assert_eq!(burst[0], 9, "range status 9 = RangeValid");
    assert_ne!(burst[2], 0, "stream_count must be non-zero");
    let raw = ((burst[13] as u32) << 8) | burst[14] as u32;
    // ±1 mm: the raw unit is 2011/2048 mm, so the model's inverse is not the
    // exact inverse of the driver's conversion — a round trip loses up to one
    // millimetre. That is the hand model's arithmetic, carried over unchanged;
    // what this test pins is that BOTH models produce the same count (the
    // assert_eq! above) and that the count is the stimulus to within the unit.
    assert!(
        (pololu_range_mm(raw) as i64 - 742).abs() <= 1,
        "the driver decoded {} mm, expected 742",
        pololu_range_mm(raw)
    );
    let _ = new;
    // Every other byte of the burst is zero — the undeclared addresses.
    for (i, b) in burst.iter().enumerate() {
        if matches!(i, 0 | 2 | 13 | 14) {
            continue;
        }
        assert_eq!(*b, 0, "burst byte {i} must read 0x00");
    }
}

/// The sweep the descriptor's two `encode:` literals exist for. The hand model
/// truncated a SUM; the descriptor rounds a QUOTIENT. They are not obviously
/// the same function, so every integer millimetre of the declared channel range
/// is compared word for word.
#[test]
fn raw_range_matches_at_every_millimetre() {
    let s = read_reg16(0x0096, 2);
    let mut checked = 0usize;
    for mm in 0..=4000u32 {
        let (old, new) = both_at_mm(f64::from(mm), &s);
        assert_eq!(
            old, new,
            "raw range word diverged at {mm} mm: legacy {old:02X?}, descriptor {new:02X?}"
        );
        checked += 1;
    }
    assert_eq!(checked, 4001, "the whole declared 0..4000 range was swept");
}

// ─── identical: the configuration blast, and the gap it walks over ─────────

#[test]
fn init_block_write_is_accepted_and_disturbs_nothing() {
    // `init()` block-writes eight bytes from 0x002D. Addresses 0x002D..0x0034
    // include 0x0031 — GPIO__TIO_HV_STATUS — which must NOT take the write:
    // the hand model dropped it on the floor, and the descriptor declares the
    // register read-only for exactly this reason.
    let s = script([
        write_reg16(0x002D, &[0x00, 0x01, 0x01, 0x01, 0x02, 0x00, 0x02, 0x08]),
        read_reg16(REG_GPIO_TIO_HV_STATUS, 1),
        write_reg16(REG_SYSTEM_MODE_START, &[0x40]),
        read_reg16(REG_RESULT_RANGE_STATUS, 17),
    ]);
    let (old, new) = both_at_mm(300.0, &s);
    assert_eq!(old, new, "init-blast transcript diverged");
    assert_eq!(
        old[0], 0x01,
        "the block write must not have cleared the not-ready bit"
    );
    let burst = &old[1..];
    let raw = ((burst[13] as u32) << 8) | burst[14] as u32;
    // ±1 mm: the raw unit is 2011/2048 mm, so the driver's round trip through
    // it is not the identity at every millimetre. Both models land on the same
    // count — that is what `assert_eq!(old, new)` above says — and the count
    // decodes to within one millimetre of the stimulus.
    assert!(
        (pololu_range_mm(raw) as i64 - 300).abs() <= 1,
        "decoded {} mm, expected 300",
        pololu_range_mm(raw)
    );
}

#[test]
fn undeclared_addresses_read_zero_identically() {
    let s = script([
        read_reg16(0x0000, 4),
        read_reg16(0x1234, 4),
        // …and the pointer wraps at 0xFFFF rather than saturating.
        read_reg16(0xFFFE, 4),
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "unmapped-address transcript diverged");
    assert_eq!(old, vec![0x00; 12]);
}

// ─── the one deliberate difference ─────────────────────────────────────────

/// ⚠️ THE NAMED DIFFERENCE. The hand model's `set_input` stored
/// `value.round() as u16`, so a fractional millimetre was thrown away before
/// the encode ever ran. The descriptor keeps the channel in f64 and rounds once,
/// at the end of the encode.
///
/// The two agree on every integer millimetre — which is every value the sweep
/// above covers and every value a shipped manifest or the channel UI produces —
/// and can differ by one raw count in between. The descriptor is the more
/// faithful of the two: it rounds the ANSWER, not the question.
#[test]
fn fractional_millimetres_round_differently_and_that_is_the_change() {
    let s = read_reg16(0x0096, 2);
    let (old, new) = both_at_mm(742.4, &s);
    assert_ne!(
        old, new,
        "the fractional-millimetre difference has been closed — if that is \
         deliberate, delete this test and say so; if it is accidental, the \
         channel has started rounding somewhere it did not"
    );

    // Both are self-consistent: the legacy word is 742 mm's word, and the
    // descriptor's is the word for 742.4 mm, which the driver reads as 742.
    let (legacy_742, _) = both_at_mm(742.0, &s);
    assert_eq!(old, legacy_742, "legacy threw the 0.4 mm away");
    let raw = ((new[0] as u32) << 8) | new[1] as u32;
    assert!((pololu_range_mm(raw) as i64 - 742).abs() <= 1);
}

/// The other half of the same statement: a HALF millimetre is where the two
/// rounding points are furthest apart, and they still land within one raw
/// count of each other. A difference bigger than that would mean the encode's
/// scale had moved, not just its rounding point.
#[test]
fn the_rounding_difference_is_at_most_one_raw_count() {
    let s = read_reg16(0x0096, 2);
    for tenth in 1..10 {
        let mm = 742.0 + f64::from(tenth) / 10.0;
        let (old, new) = both_at_mm(mm, &s);
        let o = i32::from(u16::from_be_bytes([old[0], old[1]]));
        let n = i32::from(u16::from_be_bytes([new[0], new[1]]));
        assert!(
            (o - n).abs() <= 1,
            "at {mm} mm the words are {o} and {n} — more than one count apart"
        );
    }
}
