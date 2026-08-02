// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The bay-occupancy lab, end to end: four REAL VCNL4010 models behind a REAL
//! TCA9548A, driven through a REAL controller's transaction state machine.
//!
//! `i2c_mux_tca9548a.rs` proves the switch with a deliberately minimal
//! stand-in, so a failure there is unambiguously the switch's fault. This file
//! is the opposite trade: nothing is faked, so it answers the question a user
//! actually has — *will my firmware work?* — for the one topology the
//! VCNL4010's fixed address forces on everybody.
//!
//! The byte sequences below are `Adafruit_VCNL4010`'s, not invented ones:
//!
//!   begin()          read8(0x81) and require (id & 0xF0) == 0x20
//!   readProximity()  read8(0x8E); write8(0x8E, ..); write8(0x80, 0x08);
//!                    spin on read8(0x80) & 0x20; then read16(0x87)
//!
//! Mapping to the twelve firmware behaviours a bay-occupancy sketch is
//! expected to have: the ENGINE owns initialisation and enumeration (1),
//! per-read channel selection (2), independent injection into four identical
//! sensors (3), every occupancy combination (7), cross-channel isolation (8),
//! and the missing/unreadable-sensor fault (9, 11). Thresholds (4), hysteresis
//! (5), debouncing (6), the display (10) and non-blocking scheduling (12) are
//! firmware logic — the engine's job for those is to deliver an exact,
//! reproducible count sequence for the firmware to act on, which
//! `a_noisy_sequence_arrives_sample_for_sample` pins.

use std::collections::HashMap;

use labwired_core::peripherals::components::i2c_factory::build_external_i2c_device;
use labwired_core::peripherals::components::tca9548a::Tca9548a;
use labwired_core::peripherals::i2c::{I2c, I2cDevice, I2cRegisterLayout};
use labwired_core::Peripheral;

const MUX_ADDR: u8 = 0x70;
/// Fixed on the VCNL4010 — no strap pin. The reason a switch is mandatory.
const SENSOR_ADDR: u8 = 0x13;

// Adafruit_VCNL4010 register names, verbatim.
const VCNL4010_COMMAND: u8 = 0x80;
const VCNL4010_PRODUCTID: u8 = 0x81;
const VCNL4010_IRLED: u8 = 0x83;
const VCNL4010_PROXIMITYDATA: u8 = 0x87;
const VCNL4010_INTCONTROL: u8 = 0x89;
const VCNL4010_INTSTAT: u8 = 0x8E;
const VCNL4010_MODTIMING: u8 = 0x8F;

const VCNL4010_MEASUREPROXIMITY: u8 = 0x08;
const VCNL4010_PROXIMITYREADY: u8 = 0x20;

// ── the lab ─────────────────────────────────────────────────────────────────

/// Four VCNL4010s, one per switch channel 0..=3, ids `bay0`..`bay3`.
///
/// `populated` says which bays actually have a sensor fitted, so a test can
/// build the same lab with a bay left empty (the missing-sensor fault).
fn bay_lab(populated: [bool; 4]) -> I2c {
    let mut mux = Tca9548a::new(MUX_ADDR);
    for (ch, fitted) in populated.iter().enumerate() {
        if !fitted {
            continue;
        }
        let dev = build_external_i2c_device("vcnl4010", &format!("bay{ch}"), &HashMap::new())
            .expect("vcnl4010 must build from the embedded descriptor");
        mux.attach(ch as u8, dev).unwrap();
    }
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32F1);
    let trace = labwired_core::bus::bus_trace::new_log();
    i2c.attach_traced("i2c1", &trace, Box::new(mux));
    i2c
}

fn full_lab() -> I2c {
    bay_lab([true; 4])
}

/// Drive one bay's proximity count through the stimulus seam — the same API an
/// agent or a YAML scenario uses. Deliberately NOT a bus write, so the
/// assertions stay about the bus path.
fn set_bay_proximity(i2c: &mut I2c, bay: u8, counts: u16) {
    let cell = &i2c.attached_devices()[0];
    let mut traced = cell.borrow_mut();
    let mux = traced
        .as_any_mut()
        .and_then(|a| a.downcast_mut::<Tca9548a>())
        .expect("slave 0 is the switch");
    let want = format!("bay{bay}");
    let mut done = false;
    mux.for_each_sim_input(&mut |si| {
        if si.component_id() == Some(want.as_str()) {
            si.set_input("proximity", counts as f64).unwrap();
            done = true;
            return true;
        }
        false
    });
    assert!(done, "no sensor stamped '{want}' behind the switch");
}

// ── STM32F1 bare-register transaction helpers ───────────────────────────────
//
// Same shape as `i2c_mux_tca9548a.rs`: real CR1/DR/SR1 writes so the
// controller's own state machine runs, rather than a shortcut into the device.

fn f1_start(i2c: &mut I2c) {
    i2c.write(0x15, 0x00).unwrap(); // clear latched errors (SR2 rc_w0)
    i2c.write(0x00, 0x01).unwrap(); // CR1.PE
    i2c.write(0x01, 0x01).unwrap(); // CR1.START
    for _ in 0..10 {
        i2c.tick();
    }
}

fn f1_stop(i2c: &mut I2c) {
    i2c.write(0x01, 0x02).unwrap(); // CR1.STOP
    for _ in 0..10 {
        i2c.tick();
    }
}

fn f1_addr(i2c: &mut I2c, addr: u8, reading: bool) {
    i2c.write(0x10, (addr << 1) | u8::from(reading)).unwrap();
    for _ in 0..40 {
        i2c.tick();
    }
}

fn f1_byte(i2c: &mut I2c, byte: u8) {
    i2c.write(0x10, byte).unwrap();
    for _ in 0..20 {
        i2c.tick();
    }
}

/// SR1.AF (bit 10) is the NACK flag; `peek` is byte-wide, so it is bit 2 of
/// the byte at 0x15.
fn f1_acked(i2c: &I2c) -> bool {
    i2c.peek(0x15).unwrap() & (1 << 2) == 0
}

/// `Adafruit_VCNL4010::write8` — address, register, value, STOP.
fn write8(i2c: &mut I2c, addr: u8, reg: u8, value: u8) {
    f1_start(i2c);
    f1_addr(i2c, addr, false);
    f1_byte(i2c, reg);
    f1_byte(i2c, value);
    f1_stop(i2c);
}

/// The TCA9548A has no register pointer — its control byte is a bare write.
fn select_channel(i2c: &mut I2c, channel: u8) {
    f1_start(i2c);
    f1_addr(i2c, MUX_ADDR, false);
    f1_byte(i2c, 1 << channel);
    f1_stop(i2c);
}

/// `write_then_read(&reg, 1, buf, n)` — pointer write, repeated START, n bytes.
fn read_n(i2c: &mut I2c, addr: u8, reg: u8, n: usize) -> Option<Vec<u8>> {
    f1_start(i2c);
    f1_addr(i2c, addr, false);
    if !f1_acked(i2c) {
        f1_stop(i2c);
        return None; // nobody at this address on the selected segment
    }
    f1_byte(i2c, reg);
    i2c.write(0x01, 0x01).unwrap(); // repeated START
    for _ in 0..10 {
        i2c.tick();
    }
    f1_addr(i2c, addr, true);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(i2c.read(0x10).unwrap());
        for _ in 0..20 {
            i2c.tick();
        }
    }
    f1_stop(i2c);
    Some(out)
}

/// `Adafruit_VCNL4010::read8`.
fn read8(i2c: &mut I2c, addr: u8, reg: u8) -> u8 {
    read_n(i2c, addr, reg, 1).expect("device did not ACK")[0]
}

/// `Adafruit_VCNL4010::read16` — one transaction, two bytes, big-endian.
fn read16(i2c: &mut I2c, addr: u8, reg: u8) -> u16 {
    let b = read_n(i2c, addr, reg, 2).expect("device did not ACK");
    u16::from_be_bytes([b[0], b[1]])
}

// ── the driver's own routines, replayed ─────────────────────────────────────

/// `Adafruit_VCNL4010::begin()`: reject the part unless the product id's high
/// nibble is 0x2, then write the power-on configuration.
fn adafruit_begin(i2c: &mut I2c) -> bool {
    let rev = read8(i2c, SENSOR_ADDR, VCNL4010_PRODUCTID);
    if (rev & 0xF0) != 0x20 {
        return false;
    }
    write8(i2c, SENSOR_ADDR, VCNL4010_IRLED, 20); // setLEDcurrent(20) → 200 mA
    let timing = read8(i2c, SENSOR_ADDR, VCNL4010_MODTIMING);
    write8(i2c, SENSOR_ADDR, VCNL4010_MODTIMING, timing & !0b111);
    write8(i2c, SENSOR_ADDR, VCNL4010_INTCONTROL, 0x08);
    true
}

/// `Adafruit_VCNL4010::readProximity()`.
///
/// The real routine spins `while (1)` on the ready bit with NO timeout, so a
/// model that never sets the bit hangs inside vendor code. The cap here is not
/// part of the driver — it exists so a regression FAILS instead of wedging the
/// test run, and its size is a decade beyond any plausible real poll count.
fn adafruit_read_proximity(i2c: &mut I2c) -> u16 {
    let i = read8(i2c, SENSOR_ADDR, VCNL4010_INTSTAT);
    write8(i2c, SENSOR_ADDR, VCNL4010_INTSTAT, i & !0x80);
    write8(
        i2c,
        SENSOR_ADDR,
        VCNL4010_COMMAND,
        VCNL4010_MEASUREPROXIMITY,
    );
    for _ in 0..1000 {
        if read8(i2c, SENSOR_ADDR, VCNL4010_COMMAND) & VCNL4010_PROXIMITYREADY != 0 {
            return read16(i2c, SENSOR_ADDR, VCNL4010_PROXIMITYDATA);
        }
    }
    panic!(
        "readProximity() never saw PROXIMITYREADY — the real driver has no timeout here \
         and would hang the user's firmware inside library code"
    );
}

// ── (1) initialisation and enumeration ──────────────────────────────────────

#[test]
fn begin_succeeds_on_every_bay_through_the_switch() {
    let mut i2c = full_lab();
    for bay in 0..4u8 {
        select_channel(&mut i2c, bay);
        assert!(
            adafruit_begin(&mut i2c),
            "Adafruit_VCNL4010::begin() must accept the model on bay {bay}"
        );
    }
}

// ── (2) the ready poll terminates ───────────────────────────────────────────

#[test]
fn the_ready_poll_terminates_instead_of_hanging() {
    let mut i2c = full_lab();
    select_channel(&mut i2c, 0);
    // Would panic (not hang) if the ready bit never appeared.
    adafruit_read_proximity(&mut i2c);
}

#[test]
fn a_write_to_command_cannot_clear_the_ready_bits() {
    let mut i2c = full_lab();
    select_channel(&mut i2c, 0);
    // The driver writes the measure-enable bit and nothing else; if COMMAND
    // were plain storage this write would land and stall the poll forever.
    write8(&mut i2c, SENSOR_ADDR, VCNL4010_COMMAND, 0x08);
    let cmd = read8(&mut i2c, SENSOR_ADDR, VCNL4010_COMMAND);
    assert_eq!(
        cmd & VCNL4010_PROXIMITYREADY,
        VCNL4010_PROXIMITYREADY,
        "prox_data_rdy must survive a firmware write to COMMAND"
    );
    assert_eq!(cmd & 0x40, 0x40, "als_data_rdy must survive it too");
}

// ── (3) independent injection into four identical addresses ─────────────────

#[test]
fn each_bay_reports_the_counts_injected_into_it() {
    let mut i2c = full_lab();
    let counts = [2400u16, 17_000, 300, 65_535];
    for (bay, &c) in counts.iter().enumerate() {
        set_bay_proximity(&mut i2c, bay as u8, c);
    }
    // Out of order on purpose: a stale channel selection reads as the
    // previously selected bay's value.
    for bay in [2u8, 0, 3, 1, 3, 0] {
        select_channel(&mut i2c, bay);
        assert_eq!(
            adafruit_read_proximity(&mut i2c),
            counts[bay as usize],
            "bay {bay} must report the counts injected into bay {bay}"
        );
    }
}

#[test]
fn proximity_counts_arrive_big_endian() {
    let mut i2c = full_lab();
    set_bay_proximity(&mut i2c, 1, 0x1234);
    select_channel(&mut i2c, 1);
    let raw = read_n(&mut i2c, SENSOR_ADDR, VCNL4010_PROXIMITYDATA, 2).unwrap();
    assert_eq!(
        raw,
        vec![0x12, 0x34],
        "the VCNL4010 streams the result MSB first"
    );
}

// ── (7) every combination of simultaneous bay occupancy ─────────────────────

#[test]
fn every_combination_of_four_bays_reads_back_exactly() {
    // Counts a threshold-based sketch would classify as PRESENT / EMPTY. The
    // engine does not know about either state — it guarantees the firmware
    // sees precisely these numbers, which is what makes a threshold test
    // meaningful in the first place.
    const PRESENT: u16 = 20_000;
    const EMPTY: u16 = 2_000;

    for mask in 0u8..16 {
        let mut i2c = full_lab();
        for bay in 0..4u8 {
            let occupied = mask & (1 << bay) != 0;
            set_bay_proximity(&mut i2c, bay, if occupied { PRESENT } else { EMPTY });
        }
        for bay in 0..4u8 {
            select_channel(&mut i2c, bay);
            let expect = if mask & (1 << bay) != 0 {
                PRESENT
            } else {
                EMPTY
            };
            assert_eq!(
                adafruit_read_proximity(&mut i2c),
                expect,
                "occupancy mask {mask:#06b}, bay {bay}"
            );
        }
    }
}

// ── (8) cross-channel isolation ─────────────────────────────────────────────

#[test]
fn changing_one_bay_leaves_the_other_three_untouched() {
    let mut i2c = full_lab();
    let baseline = 5_000u16;
    for bay in 0..4u8 {
        set_bay_proximity(&mut i2c, bay, baseline);
    }

    for moved in 0..4u8 {
        set_bay_proximity(&mut i2c, moved, 40_000);
        for bay in 0..4u8 {
            select_channel(&mut i2c, bay);
            let expect = if bay == moved { 40_000 } else { baseline };
            assert_eq!(
                adafruit_read_proximity(&mut i2c),
                expect,
                "after driving bay {moved}, bay {bay} must read {expect}"
            );
        }
        set_bay_proximity(&mut i2c, moved, baseline); // restore
    }
}

#[test]
fn configuring_one_bay_does_not_configure_another() {
    let mut i2c = full_lab();
    select_channel(&mut i2c, 2);
    write8(&mut i2c, SENSOR_ADDR, VCNL4010_IRLED, 20);

    select_channel(&mut i2c, 3);
    assert_eq!(
        read8(&mut i2c, SENSOR_ADDR, VCNL4010_IRLED),
        0x00,
        "bay 3 must still hold its power-on IR LED current"
    );

    select_channel(&mut i2c, 2);
    assert_eq!(
        read8(&mut i2c, SENSOR_ADDR, VCNL4010_IRLED),
        20,
        "bay 2 must hold what was written to bay 2"
    );
}

// ── (9, 11) a missing or unreadable sensor ──────────────────────────────────

#[test]
fn an_unpopulated_bay_nacks_rather_than_answering_zero() {
    // Bay 2 has no sensor fitted — a broken solder joint, or a bay that was
    // never populated. This MUST be distinguishable from "a sensor reporting
    // zero counts": firmware that cannot tell them apart shows an empty bay
    // where it should show a fault.
    let mut i2c = bay_lab([true, true, false, true]);

    select_channel(&mut i2c, 2);
    assert!(
        read_n(&mut i2c, SENSOR_ADDR, VCNL4010_PRODUCTID, 1).is_none(),
        "an empty bay must NACK, not answer"
    );

    // The neighbours are unaffected by the hole.
    for bay in [0u8, 1, 3] {
        select_channel(&mut i2c, bay);
        assert!(
            adafruit_begin(&mut i2c),
            "bay {bay} must still enumerate with bay 2 unpopulated"
        );
    }
}

#[test]
fn begin_rejects_a_bay_whose_sensor_does_not_answer() {
    let mut i2c = bay_lab([false, true, true, true]);
    select_channel(&mut i2c, 0);
    // `begin()` reads the product id first; a NACK'd read must not be mistaken
    // for a valid part. Reading through the helper that reports the NACK.
    assert!(
        read_n(&mut i2c, SENSOR_ADDR, VCNL4010_PRODUCTID, 1).is_none(),
        "begin()'s first read must fail on an unpopulated bay"
    );
}

// ── (6) support for debounce/filter testing ─────────────────────────────────

#[test]
fn a_noisy_sequence_arrives_sample_for_sample() {
    // A firmware debounce test is only meaningful if the engine delivers the
    // exact sequence the test author wrote — including the single-sample
    // glitch in the middle, which a naive filter would let through.
    let sequence = [
        2_000u16, 2_050, 1_980, 31_000, // ← one-sample glitch
        2_010, 2_030, 22_000, 21_800, 22_100, // ← a real arrival
    ];
    let mut i2c = full_lab();
    let mut seen = Vec::with_capacity(sequence.len());
    for &sample in &sequence {
        set_bay_proximity(&mut i2c, 0, sample);
        select_channel(&mut i2c, 0);
        seen.push(adafruit_read_proximity(&mut i2c));
    }
    assert_eq!(
        seen,
        sequence.to_vec(),
        "every injected sample must reach the firmware unaltered and in order"
    );
}

// ── the model's own power-on state ──────────────────────────────────────────

#[test]
fn power_on_registers_match_the_descriptor() {
    let mut i2c = full_lab();
    select_channel(&mut i2c, 0);
    assert_eq!(
        read8(&mut i2c, SENSOR_ADDR, VCNL4010_PRODUCTID),
        0x21,
        "product 0x2, revision 0x1"
    );
    assert_eq!(
        read8(&mut i2c, SENSOR_ADDR, VCNL4010_MODTIMING),
        0x81,
        "390.625 kHz, delay 0, dead time 1"
    );
    assert_eq!(read8(&mut i2c, SENSOR_ADDR, VCNL4010_INTSTAT), 0x00);
}

#[test]
fn configuration_registers_hold_what_the_driver_wrote() {
    let mut i2c = full_lab();
    select_channel(&mut i2c, 0);
    assert!(adafruit_begin(&mut i2c));
    assert_eq!(
        read8(&mut i2c, SENSOR_ADDR, VCNL4010_IRLED),
        20,
        "setLEDcurrent(20)"
    );
    assert_eq!(
        read8(&mut i2c, SENSOR_ADDR, VCNL4010_MODTIMING),
        0x80,
        "setFrequency() clears the low three bits of MOD_TIMING"
    );
    assert_eq!(
        read8(&mut i2c, SENSOR_ADDR, VCNL4010_INTCONTROL),
        0x08,
        "begin() enables the proximity-ready interrupt source"
    );
}

// ── the weld: the manifest the TS compiler actually emits ───────────────────
//
// Everything above builds the lab in Rust. This case starts from the exact
// `external_devices:` block `packages/board-config` emits for the four-bay
// diagram (see `test/i2c-mux-emit.test.ts`, which asserts the same bytes from
// the other side) and proves core reconstructs the intended topology from it.
//
// Without this, the two halves could each be self-consistently wrong: the
// emitter could name a channel core ignores, or core could expect a shape the
// emitter never produces, and both test suites would stay green.
//
// Core is a submodule and cannot read the parent repo, so these bytes are a
// vendored copy rather than a shared file. The weld is on the OTHER side:
// `emits exactly the manifest core is tested against` in that TS file asserts
// the compiler's output equals this string exactly, so changing the emit
// format fails there and names this fixture to update.
const EMITTED_MANIFEST: &str = r#"
name: "playground-board"
chip: "esp32s3"
external_devices:
  - id: "mux"
    type: "tca9548a"
    connection: "i2c0"
    route:
      sda: "GPIO8"
      scl: "GPIO9"
    config:
      i2c_address: 0x70
  - id: "bay0"
    type: "vcnl4010"
    connection: "mux"
    channel: 0
    config:
      i2c_address: 0x13
  - id: "bay1"
    type: "vcnl4010"
    connection: "mux"
    channel: 1
    config:
      i2c_address: 0x13
  - id: "bay2"
    type: "vcnl4010"
    connection: "mux"
    channel: 2
    config:
      i2c_address: 0x13
  - id: "bay3"
    type: "vcnl4010"
    connection: "mux"
    channel: 3
    config:
      i2c_address: 0x13
"#;

#[test]
fn the_compilers_manifest_builds_the_intended_topology() {
    use labwired_core::peripherals::components::{build_i2c_tree, i2c_mux_child_ids};

    let manifest: labwired_config::SystemManifest =
        serde_yaml::from_str(EMITTED_MANIFEST).expect("the emitted manifest must parse");

    // The four sensors are recognised as switch children, so no attach path
    // puts them straight on the controller (which would be the silent
    // mis-wiring the whole topology exists to prevent).
    assert_eq!(
        i2c_mux_child_ids(&manifest),
        vec!["bay0", "bay1", "bay2", "bay3"]
    );

    let device = build_i2c_tree(&manifest, &manifest.external_devices[0])
        .unwrap()
        .expect("the switch must build");
    let mux = device
        .as_any()
        .and_then(|a| a.downcast_ref::<Tca9548a>())
        .expect("the assembled unit is the switch itself");
    assert_eq!(mux.address(), MUX_ADDR);

    for ch in 0..4u8 {
        let behind = mux.channel_devices(ch);
        assert_eq!(behind.len(), 1, "one sensor on channel {ch}");
        assert_eq!(
            behind[0].address(),
            SENSOR_ADDR,
            "channel {ch} carries a VCNL4010 at its fixed address"
        );
    }
    assert!(
        mux.channel_devices(4).is_empty(),
        "channels 4..7 are unpopulated in this diagram"
    );
}

#[test]
fn the_compilers_manifest_drives_every_bay_through_a_controller() {
    let manifest: labwired_config::SystemManifest =
        serde_yaml::from_str(EMITTED_MANIFEST).expect("the emitted manifest must parse");
    let device = labwired_core::peripherals::components::build_i2c_tree(
        &manifest,
        &manifest.external_devices[0],
    )
    .unwrap()
    .expect("the switch must build");

    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32F1);
    let trace = labwired_core::bus::bus_trace::new_log();
    i2c.attach_traced("i2c1", &trace, device);

    // The ids come from the diagram, so an agent (or a YAML scenario) drives a
    // bay by the name the user gave it on the canvas.
    let counts = [1_200u16, 24_000, 900, 33_333];
    for (bay, &c) in counts.iter().enumerate() {
        let cell = &i2c.attached_devices()[0];
        let mut traced = cell.borrow_mut();
        let mux = traced
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<Tca9548a>())
            .expect("slave 0 is the switch");
        let want = format!("bay{bay}");
        let mut done = false;
        mux.for_each_sim_input(&mut |si| {
            if si.component_id() == Some(want.as_str()) {
                si.set_input("proximity", c as f64).unwrap();
                done = true;
                return true;
            }
            false
        });
        assert!(done, "the manifest must stamp the diagram id '{want}'");
    }

    for bay in [3u8, 1, 0, 2] {
        select_channel(&mut i2c, bay);
        assert!(
            adafruit_begin(&mut i2c),
            "begin() must succeed on bay {bay} of the compiled board"
        );
        assert_eq!(
            adafruit_read_proximity(&mut i2c),
            counts[bay as usize],
            "bay {bay} of the compiled board"
        );
    }
}
