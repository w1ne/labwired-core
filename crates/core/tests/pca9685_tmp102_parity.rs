// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **Byte-parity harness: the declarative PCA9685 / TMP102 vs the hand-written
//! oracles.**
//!
//! The shipping PCA9685 and TMP102 are now the declarative descriptors
//! `configs/devices/pca9685.yaml` / `configs/devices/tmp102.yaml`, interpreted by
//! [`GenericI2cDevice`]. The hand-written [`Pca9685`] / [`Tmp102`] models are
//! retained *only* as the reference this file proves the declarative devices
//! byte-identical against: every test drives the OLD and NEW devices through the
//! exact same I²C script and asserts byte-equal reads (and, for PCA9685, equal
//! `servo_angle` observables).
//!
//! This is the declarative-vs-hand-written gate, and now the sole such gate:
//! the former IR component engine (and its `ir_component_equivalence.rs`) was
//! retired in Phase B, leaving one declarative stack.

use labwired_core::peripherals::components::{GenericI2cDevice, Pca9685};
use labwired_core::peripherals::esp32s3::tmp102::Tmp102;
use labwired_core::peripherals::i2c::I2cDevice;

/// One bus op. Deterministic corpus only — no randomness.
#[derive(Clone)]
enum Op {
    Start,
    Write(u8),
    Read,
}

fn declarative(device_type: &str) -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml(device_type)
        .unwrap_or_else(|| panic!("{device_type} descriptor is embedded"));
    GenericI2cDevice::from_yaml(yaml, 0)
        .unwrap_or_else(|e| panic!("{device_type}.yaml is a valid descriptor: {e}"))
}

/// Drive the oracle and the declarative device through `ops` in lockstep,
/// asserting every read is byte-identical.
#[track_caller]
fn drive_both(oracle: &mut dyn I2cDevice, decl: &mut dyn I2cDevice, ops: &[Op]) {
    assert_eq!(oracle.address(), decl.address(), "address");
    for (i, op) in ops.iter().enumerate() {
        match op {
            Op::Start => {
                oracle.start();
                decl.start();
            }
            Op::Write(b) => {
                oracle.write(*b);
                decl.write(*b);
            }
            Op::Read => assert_eq!(oracle.read(), decl.read(), "read divergence at op {i}"),
        }
    }
}

// ─── PCA9685 ────────────────────────────────────────────────────────────────

fn set_angle_ops(ops: &mut Vec<Op>, ch: u8, deg: f64) {
    let us = 500.0 + (deg / 180.0) * 1900.0;
    let ticks = (us / 20000.0 * 4096.0) as u16;
    ops.push(Op::Start);
    ops.push(Op::Write(0x06 + 4 * ch));
    ops.push(Op::Write(0x00));
    ops.push(Op::Write(0x00));
    ops.push(Op::Write((ticks & 0xFF) as u8));
    ops.push(Op::Write(((ticks >> 8) & 0x0F) as u8));
}

/// After driving `ops` into a fresh declarative PCA9685, assert every channel's
/// `servo_angle` observable equals the hand-written oracle's `channel_angle_deg`
/// (presence and value, within the IR-gate tolerance of 0.01°).
#[track_caller]
fn assert_observables_equal(oracle: &Pca9685, decl: &GenericI2cDevice) {
    for ch in 0..16u8 {
        let a = oracle.channel_angle_deg(ch);
        let b = decl.observable("servo_angle", ch);
        match (a, b) {
            (None, None) => {}
            (Some(x), Some(y)) => assert!(
                (x as f64 - y).abs() < 0.01,
                "ch {ch}: oracle {x} vs declarative {y}"
            ),
            _ => panic!("ch {ch}: presence mismatch oracle={a:?} declarative={b:?}"),
        }
    }
}

#[test]
fn pca9685_dispense_sequence_is_byte_equivalent_with_observables() {
    let mut oracle = Pca9685::new();
    let mut decl = declarative("pca9685");
    let mut ops = vec![Op::Start, Op::Write(0x00), Op::Write(0xA1)]; // AI on
    set_angle_ops(&mut ops, 8, 15.0); // revolver → compartment 1
    set_angle_ops(&mut ops, 12, 20.0); // shutter closed
    set_angle_ops(&mut ops, 12, 90.0); // shutter open
    set_angle_ops(&mut ops, 8, 135.0); // revolver → compartment 5
                                       // Read back the channel-8 block through AI.
    ops.push(Op::Start);
    ops.push(Op::Write(0x06 + 4 * 8));
    for _ in 0..4 {
        ops.push(Op::Read);
    }
    drive_both(&mut oracle, &mut decl, &ops);
    assert_observables_equal(&oracle, &decl);
}

#[test]
fn pca9685_pointer_semantics_without_ai_are_byte_equivalent() {
    // AI off (power-on MODE1=0x11): repeated reads hit the same register; data
    // writes overwrite the same register.
    let ops = vec![
        Op::Start,
        Op::Write(0x00), // pointer = MODE1
        Op::Read,
        Op::Read,
        Op::Start,
        Op::Write(0x06),
        Op::Write(0x55), // data write with AI off
        Op::Write(0x66), // overwrites the same register
        Op::Start,
        Op::Write(0x06),
        Op::Read,
    ];
    drive_both(&mut Pca9685::new(), &mut declarative("pca9685"), &ops);
}

#[test]
fn pca9685_full_255_byte_ai_sweep_is_byte_equivalent() {
    // Walk every register: write a deterministic pattern with AI on, then read
    // the whole file back and compare byte-for-byte.
    let mut ops = vec![Op::Start, Op::Write(0x00), Op::Write(0xA1)];
    ops.push(Op::Start);
    ops.push(Op::Write(0x01)); // start after MODE1 to keep AI set
    for i in 1..=255u32 {
        ops.push(Op::Write((i.wrapping_mul(37) & 0xFF) as u8));
    }
    ops.push(Op::Start);
    ops.push(Op::Write(0x00));
    for _ in 0..=255 {
        ops.push(Op::Read);
    }
    drive_both(&mut Pca9685::new(), &mut declarative("pca9685"), &ops);
}

#[test]
fn pca9685_b2_ai_enable_timing_is_byte_equivalent() {
    // The Write(0xA1) that sets AI is checked *after* it is stored, so the AI
    // bit is visible for the auto-increment check on the same write: the pointer
    // advances 0→1 on the enabling write, and the first Read returns regs[1].
    let ops = vec![
        Op::Start,
        Op::Write(0x00), // pointer = MODE1
        Op::Write(0xA1), // stores 0xA1 into regs[0]; AI now visible → pointer → 1
        Op::Read,        // reads regs[1] = 0x00 (MODE2 reset); pointer → 2
        Op::Read,        // reads regs[2]; pointer → 3
    ];
    drive_both(&mut Pca9685::new(), &mut declarative("pca9685"), &ops);
}

#[test]
fn pca9685_b3_double_start_is_byte_equivalent() {
    let ops = vec![
        Op::Start,
        Op::Start,       // second consecutive START — must be a no-op
        Op::Write(0x00), // pointer = MODE1
        Op::Read,        // returns reset 0x11
    ];
    drive_both(&mut Pca9685::new(), &mut declarative("pca9685"), &ops);
}

#[test]
fn pca9685_servo_angle_observable_matches_across_duties_including_raw_zero() {
    // Several duty values across the range, plus a clamp-at-0 (small raw) and a
    // never-written channel (raw 0 → None on both).
    let mut oracle = Pca9685::new();
    let mut decl = declarative("pca9685");
    let mut ops = vec![Op::Start, Op::Write(0x00), Op::Write(0xA1)]; // AI on
    set_angle_ops(&mut ops, 0, 0.0);
    set_angle_ops(&mut ops, 1, 45.0);
    set_angle_ops(&mut ops, 2, 90.0);
    set_angle_ops(&mut ops, 4, 135.0);
    set_angle_ops(&mut ops, 5, 180.0);
    // Channel 3: raw OFF = 50 (nonzero but maps below 0° → clamps to 0.0).
    ops.push(Op::Start);
    ops.push(Op::Write(0x06 + 4 * 3));
    ops.push(Op::Write(0x00));
    ops.push(Op::Write(0x00));
    ops.push(Op::Write(50)); // OFF_L = 50
    ops.push(Op::Write(0x00)); // OFF_H = 0 → raw = 50
    drive_both(&mut oracle, &mut decl, &ops);
    assert_observables_equal(&oracle, &decl);
    // Channel 3 is nonzero → Some(clamped 0.0); channel 15 never written → None.
    assert_eq!(decl.observable("servo_angle", 3), Some(0.0));
    assert_eq!(oracle.channel_angle_deg(3), Some(0.0));
    assert_eq!(decl.observable("servo_angle", 15), None);
    assert_eq!(oracle.channel_angle_deg(15), None);
    // Out-of-range channel and unknown observable are None.
    assert_eq!(decl.observable("servo_angle", 16), None);
    assert_eq!(decl.observable("nope", 0), None);
}

// ─── TMP102 ─────────────────────────────────────────────────────────────────

#[test]
fn tmp102_temperature_drift_and_wrap_are_byte_equivalent() {
    let mut oracle = Tmp102::new();
    let mut decl = declarative("tmp102");
    assert_eq!(oracle.address(), decl.address(), "tmp102 address 0x48");
    // 60 full temperature reads (each framed by START + pointer 0x00 + two reads)
    // crosses the 35 °C wrap at least twice.
    let mut ops = Vec::new();
    for _ in 0..60 {
        ops.push(Op::Start);
        ops.push(Op::Write(0x00));
        ops.push(Op::Read);
        ops.push(Op::Read);
    }
    drive_both(&mut oracle, &mut decl, &ops);
}

#[test]
fn tmp102_config_tlow_thigh_read_back_identically() {
    let mut ops = Vec::new();
    for ptr in 1..=3u8 {
        ops.push(Op::Start);
        ops.push(Op::Write(ptr));
        ops.push(Op::Read); // MSB
        ops.push(Op::Read); // LSB
    }
    drive_both(&mut Tmp102::new(), &mut declarative("tmp102"), &ops);
}

#[test]
fn tmp102_config_write_is_absorbed_identically() {
    // Write pointer 0x01 (config), then a data byte (0x55) — absorbed/ignored by
    // both models; read back config and assert byte equality (proves absorb, not
    // just by-construction).
    let ops = vec![
        Op::Start,
        Op::Write(0x01), // pointer → config
        Op::Write(0x55), // absorbed by both
        Op::Start,
        Op::Write(0x01),
        Op::Read, // config MSB
        Op::Read, // config LSB
    ];
    drive_both(&mut Tmp102::new(), &mut declarative("tmp102"), &ops);
}

#[test]
fn tmp102_pointer_masking_is_byte_equivalent() {
    // Pointer decodes only the low two bits: writing 0x04 aliases to 0x00 (temp),
    // 0x06 aliases to 0x02 (T_LOW). Both models must agree.
    let ops = vec![
        // 0x04 → temp: MSB/LSB then drift.
        Op::Start,
        Op::Write(0x04),
        Op::Read,
        Op::Read,
        // 0x06 → T_LOW (0x4B00).
        Op::Start,
        Op::Write(0x06),
        Op::Read,
        Op::Read,
        // 0x07 → T_HIGH (0x5000).
        Op::Start,
        Op::Write(0x07),
        Op::Read,
        Op::Read,
    ];
    drive_both(&mut Tmp102::new(), &mut declarative("tmp102"), &ops);
}
