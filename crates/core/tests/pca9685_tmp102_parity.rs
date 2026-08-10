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

// ─── PCA9685 power-on state (ABSOLUTE, not a parity mirror) ─────────────────

/// Point at `addr` and read one byte, with auto-increment off so the pointer
/// does not walk. Works on either model.
fn read_reg8(d: &mut dyn I2cDevice, addr: u8) -> u8 {
    d.start();
    d.write(addr);
    d.start();
    let b = d.read();
    d.stop();
    b
}

/// **This is not a parity test.** `drive_both` only proves the two models
/// agree, and they agreed for months on an all-zero power-up that no PCA9685
/// has ever had. So this asserts the values the DOCUMENT gives, against each
/// model independently — if either regresses, this fails, even though parity
/// would stay green if both regressed together.
///
/// NXP PCA9685 datasheet Rev. 4 (16 April 2015); page cited per row.
#[test]
fn pca9685_power_on_registers_match_the_datasheet_on_both_models() {
    // (addr, value, what it is / where the datasheet says so)
    let mut expected: Vec<(u8, u8, &str)> = vec![
        (0x00, 0x11, "MODE1 SLEEP|ALLCALL (p16)"),
        (0x01, 0x04, "MODE2 OUTDRV (p16)"),
        (0x02, 0xE2, "SUBADR1 (p26)"),
        (0x03, 0xE4, "SUBADR2 (p26)"),
        (0x04, 0xE8, "SUBADR3 (p26)"),
        (0x05, 0xE0, "ALLCALLADR (p8, p26)"),
        (0xFE, 0x1E, "PRE_SCALE 200 Hz @ 25 MHz (p25 Table 8, p2)"),
    ];
    // LEDn_OFF_H = 0x09 + 4n, n = 0..15 — bit 4 is "LEDn full OFF" (p21, p25).
    // LED15_OFF_H lands on 0x45, matching the register summary (Table 4, p13).
    for n in 0..16u8 {
        expected.push((0x09 + 4 * n, 0x10, "LEDn_OFF_H full OFF (p21, p25)"));
    }
    assert_eq!(
        expected
            .iter()
            .find(|(a, ..)| *a == 0x45)
            .map(|(_, v, _)| *v),
        Some(0x10),
        "LED15_OFF_H must be 0x45 (register summary Table 4, p13)"
    );

    let mut oracle = Pca9685::new();
    let mut decl = declarative("pca9685");
    for (addr, want, why) in &expected {
        assert_eq!(
            read_reg8(&mut oracle, *addr),
            *want,
            "oracle: reg {addr:#04x} power-on must be {want:#04x} — {why}"
        );
        assert_eq!(
            read_reg8(&mut decl, *addr),
            *want,
            "declarative: reg {addr:#04x} power-on must be {want:#04x} — {why}"
        );
    }

    // ALL_LED_ON_H (0xFB) / ALL_LED_OFF_H (0xFD) stay 0x00 ON PURPOSE. Table 8
    // (p25) star-marks them with bit 4 set; the register summary Table 4 (p13)
    // types the same registers "write/read zero". The vendor contradicts
    // itself; we do not resolve it for them. This asserts the CHOICE so a
    // future edit has to argue with it rather than drift past it.
    for addr in [0xFBu8, 0xFD] {
        assert_eq!(
            read_reg8(&mut oracle, addr),
            0x00,
            "oracle: reg {addr:#04x} is a documented contradiction — left at 0"
        );
        assert_eq!(
            read_reg8(&mut decl, addr),
            0x00,
            "declarative: reg {addr:#04x} is a documented contradiction — left at 0"
        );
    }

    // Everything else powers up zero. Sweeping the rest keeps a stray reset
    // entry from being added without a datasheet citation.
    let claimed: std::collections::BTreeSet<u8> = expected.iter().map(|(a, ..)| *a).collect();
    for addr in 0..=0xFFu8 {
        if claimed.contains(&addr) {
            continue;
        }
        assert_eq!(
            read_reg8(&mut oracle, addr),
            0x00,
            "oracle: reg {addr:#04x} has an undocumented non-zero reset"
        );
        assert_eq!(
            read_reg8(&mut decl, addr),
            0x00,
            "declarative: reg {addr:#04x} has an undocumented non-zero reset"
        );
    }
}

/// The `LEDn_OFF_H = 0x10` reset must NOT make an untouched channel look like a
/// commanded servo position. The observable composes its 12-bit count with
/// `hi_mask: 0x0F`, so bit 4 is masked away and the raw count is still 0 —
/// `none_when_raw_zero` then reports no angle. Confirmed here rather than
/// assumed, on both models.
///
/// This is the mask coinciding with the right answer, NOT full-OFF being
/// honoured: the second half of this test pins the gap. A channel whose OFF_H
/// carries the full-OFF bit *and* a nonzero count still reports an angle, which
/// silicon would not do. If someone implements full-OFF gating, this assertion
/// is the one that must change — deliberately.
#[test]
fn pca9685_full_off_reset_reports_no_angle_but_is_not_actually_honoured() {
    let oracle = Pca9685::new();
    let decl = declarative("pca9685");
    for ch in 0..16u8 {
        assert_eq!(
            oracle.channel_angle_deg(ch),
            None,
            "oracle ch {ch}: untouched channel must report no angle despite OFF_H = 0x10"
        );
        assert_eq!(
            decl.observable("servo_angle", ch),
            None,
            "declarative ch {ch}: untouched channel must report no angle despite OFF_H = 0x10"
        );
    }

    // KNOWN GAP, pinned rather than hidden: full OFF (OFF_H bit 4) is not
    // honoured by the observable path. Write channel 0 with the full-OFF bit
    // set AND a nonzero 12-bit count; silicon holds the output fully off, both
    // models report an angle.
    let mut oracle = Pca9685::new();
    let mut decl = declarative("pca9685");
    let ops = vec![
        Op::Start,
        Op::Write(0x00),
        Op::Write(0xA1), // AI on
        Op::Start,
        Op::Write(0x06), // LED0_ON_L
        Op::Write(0x00), // ON_L
        Op::Write(0x00), // ON_H
        Op::Write(0x29), // OFF_L
        Op::Write(0x11), // OFF_H: full OFF (bit 4) + count high nibble 0x1
    ];
    drive_both(&mut oracle, &mut decl, &ops);
    assert_observables_equal(&oracle, &decl);
    let angle = decl
        .observable("servo_angle", 0)
        .expect("GAP: full OFF is ignored, so an angle is still reported");
    assert!(
        (angle - 90.0).abs() < 0.5,
        "GAP marker: full-OFF bit is masked away, angle still computed: {angle}"
    );
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
        Op::Read,        // reads regs[1] = 0x04 (MODE2 reset, OUTDRV); pointer → 2
        Op::Read,        // reads regs[2] = 0xE2 (SUBADR1 reset); pointer → 3
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
fn tmp102_short_config_write_is_absorbed_identically() {
    // Write pointer 0x01 (config), then ONE data byte (0x55).
    //
    // KNOWN GAP, pinned here rather than hidden: the datasheet's "Data
    // Transfer" note says the TMP102 "can also be used for single byte
    // updates. To update only the MS byte, terminate the communication by
    // issuing a START or STOP", so on silicon this WOULD land 0x55 in
    // config byte 1. Neither model implements it — the declarative engine
    // stores a register write only when the master supplies the register's
    // full width, and the oracle likewise waits for both bytes. This test
    // asserts only that the two models agree; closing the gap needs a
    // partial-width write in the shared engine, which is a separate change.
    let ops = vec![
        Op::Start,
        Op::Write(0x01), // pointer → config
        Op::Write(0x55), // half a word: absorbed by both
        Op::Start,
        Op::Write(0x01),
        Op::Read, // config MSB
        Op::Read, // config LSB
    ];
    drive_both(&mut Tmp102::new(), &mut declarative("tmp102"), &ops);
}

#[test]
fn tmp102_full_width_config_and_limit_writes_are_byte_equivalent() {
    // Table 6-7 marks config / TLOW / THIGH Read/Write, and both models
    // implement them with the same datasheet-derived read-only bits (CONFIG
    // R1:R0 + AL + unused D3:D0; limits' unused D2:D0). A complete two-byte
    // write must therefore read back identically on both.
    let mut ops = Vec::new();
    let program = |ops: &mut Vec<Op>, ptr: u8, msb: u8, lsb: u8| {
        ops.push(Op::Start);
        ops.push(Op::Write(ptr));
        ops.push(Op::Write(msb));
        ops.push(Op::Write(lsb));
    };
    let readback = |ops: &mut Vec<Op>, ptr: u8| {
        ops.push(Op::Start);
        ops.push(Op::Write(ptr));
        ops.push(Op::Read);
        ops.push(Op::Read);
    };
    // Shutdown + one-shot, 1 Hz, extended mode; R1:R0 and AL written as 0.
    program(&mut ops, 0x01, 0x81, 0x50);
    // Thermostat window 25 °C … 30 °C, with stray unused low bits set.
    program(&mut ops, 0x02, 0x19, 0x07);
    program(&mut ops, 0x03, 0x1E, 0x05);
    for ptr in 1..=3u8 {
        readback(&mut ops, ptr);
    }
    // Clear every firmware-owned bit and read back again.
    program(&mut ops, 0x01, 0x00, 0x00);
    readback(&mut ops, 0x01);
    // A write to the Read Only temperature register must still be dropped.
    program(&mut ops, 0x00, 0xDE, 0xAD);
    readback(&mut ops, 0x00);
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
