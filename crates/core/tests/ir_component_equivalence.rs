// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Byte-equivalence gate: the IR-interpreted PCA9685 must be
//! indistinguishable from the hand-written Rust model over the I2cDevice
//! interface — same bytes read, same observables — across a deterministic
//! transaction corpus including the firmware's dispense sequences.

use labwired_core::peripherals::components::{IrI2cComponent, Pca9685};
use labwired_core::peripherals::esp32s3::tmp102::Tmp102;
use labwired_core::peripherals::i2c::I2cDevice;

fn ir_pca() -> IrI2cComponent {
    let yaml = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/components/pca9685.yaml"
    ))
    .expect("spec asset");
    IrI2cComponent::new(serde_yaml::from_str(&yaml).expect("parse"), None).expect("valid")
}

/// One bus op. Deterministic corpus only — no randomness.
enum Op {
    Start,
    Write(u8),
    Read,
}

fn run_corpus(ops: &[Op]) {
    let mut rust: Box<dyn I2cDevice> = Box::new(Pca9685::new());
    let mut ir = ir_pca();
    assert_eq!(rust.address(), ir.address(), "address");
    for (i, op) in ops.iter().enumerate() {
        match op {
            Op::Start => {
                rust.start();
                ir.start();
            }
            Op::Write(b) => {
                rust.write(*b);
                ir.write(*b);
            }
            Op::Read => {
                assert_eq!(rust.read(), ir.read(), "read divergence at op {i}");
            }
        }
    }
    // Observables must agree with the Rust model's accessors on every channel.
    let rust_concrete = rust.as_any().unwrap().downcast_ref::<Pca9685>().unwrap();
    for ch in 0..16u8 {
        let a = rust_concrete.channel_angle_deg(ch);
        let b = ir.observable("servo_angle", ch);
        match (a, b) {
            (None, None) => {}
            (Some(x), Some(y)) => assert!((x - y).abs() < 0.01, "ch {ch}: {x} vs {y}"),
            _ => panic!("ch {ch}: presence mismatch {a:?} vs {b:?}"),
        }
    }
}

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

#[test]
fn dispense_sequence_is_byte_equivalent() {
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
    run_corpus(&ops);
}

#[test]
fn pointer_semantics_without_ai_are_byte_equivalent() {
    // AI off (power-on MODE1=0x11): repeated reads hit the same register.
    let ops = vec![
        Op::Start,
        Op::Write(0x00), // pointer = MODE1
        Op::Read,
        Op::Read,
        Op::Start,
        Op::Write(0x06),
        Op::Write(0x55), // data write with AI off
        Op::Write(0x66), // overwrites same register
        Op::Start,
        Op::Write(0x06),
        Op::Read,
    ];
    run_corpus(&ops);
}

#[test]
fn full_register_sweep_is_byte_equivalent() {
    // Walk every register: write a deterministic pattern with AI on, then
    // read the whole file back and compare byte-for-byte.
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
    run_corpus(&ops);
}

// ── Part B: corpus hardeners ──────────────────────────────────────────────────

#[test]
fn corpus_hardeners_are_byte_equivalent() {
    // B1: Clamp-at-0 — a channel with small nonzero OFF value clamps to 0.0°.
    // raw ticks = 50 → angle = 50 * 0.46258224 + (-47.368423) = 23.129 - 47.368 = -24.239
    // clamped to 0.0 by the [0.0, 180.0] clamp range.
    {
        let mut ops = vec![Op::Start, Op::Write(0x00), Op::Write(0xA1)]; // AI on
                                                                         // Write channel 3: ON_L=0, ON_H=0, OFF_L=50, OFF_H=0 (raw=50).
        ops.push(Op::Start);
        ops.push(Op::Write(0x06 + 4 * 3)); // LED3_ON_L
        ops.push(Op::Write(0x00)); // ON_L
        ops.push(Op::Write(0x00)); // ON_H
        ops.push(Op::Write(50)); // OFF_L = 50
        ops.push(Op::Write(0x00)); // OFF_H = 0 → raw = 50
                                   // Read the 4-byte block back and confirm byte equality.
        ops.push(Op::Start);
        ops.push(Op::Write(0x06 + 4 * 3));
        for _ in 0..4 {
            ops.push(Op::Read);
        }
        run_corpus(&ops);
        // Both models clamp raw=50 to 0.0°.  Verify via a direct IR observable read.
        let mut ir = ir_pca();
        for op in &ops {
            match op {
                Op::Start => ir.start(),
                Op::Write(b) => ir.write(*b),
                Op::Read => {
                    let _ = ir.read();
                }
            }
        }
        let angle = ir
            .observable("servo_angle", 3)
            .expect("raw=50, nonzero → Some");
        assert!(angle == 0.0, "expected 0.0° (clamped), got {angle}");
    }

    // B2: AI-enable timing — the Write(0xA1) that sets AI is checked *after*
    // it is stored, so the AI bit is visible for the auto-increment check on
    // the same write. The pointer advances to 1 on the enabling write, and
    // the first subsequent Read returns regs[1] (MODE2), not regs[0].
    {
        let ops = vec![
            Op::Start,
            Op::Write(0x00), // pointer = MODE1 (0x00)
            Op::Write(0xA1), // stores 0xA1 into regs[0]; AI bit now visible
            // Auto-increment check happens after the store: AI is set, so
            // pointer advances from 0 to 1. The next Read will return regs[1].
            Op::Read, // reads regs[1] = 0x00 (MODE2 reset); pointer → 2
            Op::Read, // reads regs[2]; pointer → 3
        ];
        run_corpus(&ops);
    }

    // B3: Double-START idempotence — consecutive STARTs before a normal sequence
    // are harmless; pointer is set correctly by the first write after the last START.
    {
        let ops = vec![
            Op::Start,
            Op::Start,       // second consecutive START — must be a no-op
            Op::Write(0x00), // pointer = MODE1
            Op::Read,        // should return reset value 0x11
        ];
        run_corpus(&ops);
    }
}

// ── TMP102 ────────────────────────────────────────────────────────────────────

fn ir_tmp102() -> IrI2cComponent {
    let yaml = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/components/tmp102.yaml"
    ))
    .expect("spec asset");
    IrI2cComponent::new(serde_yaml::from_str(&yaml).expect("parse"), None).expect("valid")
}

#[test]
fn tmp102_temperature_reads_and_drift_are_byte_equivalent() {
    let mut rust = Tmp102::new();
    let mut ir = ir_tmp102();
    assert_eq!(rust.address(), ir.address());
    // 60 full temperature reads: crosses the 35 °C wrap at least once
    // ((0x2300 - 0x1900) / 0x80 = 20 reads to first wrap; 60 reads crosses the wrap twice
    // (first at read 20, then every 31 reads)).
    for i in 0..60 {
        rust.start();
        ir.start();
        rust.write(0x00);
        ir.write(0x00);
        for half in 0..2 {
            assert_eq!(rust.read(), ir.read(), "read {i}.{half}");
        }
    }
    // Config / T_LOW / T_HIGH read back identically (MSB then LSB).
    for ptr in 1..=3u8 {
        rust.start();
        ir.start();
        rust.write(ptr);
        ir.write(ptr);
        assert_eq!(rust.read(), ir.read(), "ptr {ptr} MSB");
        assert_eq!(rust.read(), ir.read(), "ptr {ptr} LSB");
    }
    // Config-write cross-validation: write pointer 0x01, write a data byte (0x55)
    // — absorbed by both models; then read back config (MSB + LSB) and assert
    // byte equality. Proves absorb-and-ignore is byte-identical, not just by-construction.
    rust.start();
    ir.start();
    rust.write(0x01); // pointer → config register
    ir.write(0x01);
    rust.write(0x55); // absorbed/ignored by both models
    ir.write(0x55);
    rust.start();
    ir.start();
    rust.write(0x01);
    ir.write(0x01);
    assert_eq!(rust.read(), ir.read(), "config MSB after write(0x55)");
    assert_eq!(rust.read(), ir.read(), "config LSB after write(0x55)");
}
