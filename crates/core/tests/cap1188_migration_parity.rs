// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **CAP1188, against the hand-written model it replaces.**
//!
//! `components/cap1188.rs` (645 lines) is DELETED. What it held that a register
//! map alone could not say is the LATCH — status bits and MAIN_CONTROL.INT that
//! do not clear on read, a write-0 that drops only the pads since RELEASED, and
//! a still-held pad that re-asserts the interrupt in the same write. That is
//! four rules in `configs/devices/cap1188.yaml`, and the script below walks all
//! four of them.
//!
//! [`CAP1188_GOLDEN`] is the transcript the model produced for
//! [`migration_script`], captured by running it against the model on the commit
//! that removed it. **Byte-identical**, with one named difference tested
//! separately: the model quantised an injected percentage to a whole percent
//! before converting it to counts, and the descriptor does not.
//!
//! ⚠️ The 50 % touch threshold and the 127-count full scale are MODELLING
//! CHOICES, not datasheet values — the analogue calibration loop is not
//! simulated. So is REVISION 0x83, which is silicon-revision dependent; probe
//! with PRODUCT_ID / MANUFACTURER_ID. All three are stated in the descriptor.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;

mod common;
use common::transcript::{read_reg, run_i2c, script, write_reg, Step};

const ADDR: u8 = 0x29;

fn dev() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("cap1188")
        .expect("cap1188 descriptor is not embedded");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("cap1188.yaml does not build")
}

/// The conversation the deleted `Cap1188` was driven through.
fn migration_script() -> Vec<Step<'static>> {
    script([
        read_reg(0xFD, 3), // PRODUCT_ID, MANUFACTURER_ID, REVISION
        read_reg(0x00, 4), // MAIN_CONTROL, 0x01, GENERAL_STATUS, SENSOR_INPUT_STATUS
        read_reg(0x1F, 3), // SENSITIVITY, CONFIGURATION, SENSOR_INPUT_ENABLE
        read_reg(0x27, 1), // INTERRUPT_ENABLE
        read_reg(0x44, 1), // CONFIGURATION_2
        read_reg(0x10, 8), // every delta count at rest
        // CS1 pressed firmly, CS3 at a near-threshold 49 %, CS5 just over.
        vec![
            Step::Input("touch1", 100.0),
            Step::Input("touch3", 49.0),
            Step::Input("touch5", 51.0),
        ],
        read_reg(0x00, 4),
        read_reg(0x10, 8),
        // The latch: release CS1, status STAYS until the write-0.
        vec![Step::Input("touch1", 0.0)],
        read_reg(0x00, 4),
        write_reg(0x00, &[0x00]),
        read_reg(0x00, 4), // CS5 is still held, so INT re-asserts
        // Release CS5 too, then clear again: now everything drops.
        vec![Step::Input("touch5", 0.0)],
        write_reg(0x00, &[0x00]),
        read_reg(0x00, 4),
        // Gain bits round-trip through the write mask.
        write_reg(0x00, &[0xC0]),
        read_reg(0x00, 1),
        // Disable CS5 and CS7 while CS5 is held: it stops reporting at once.
        vec![Step::Input("touch5", 100.0), Step::Input("touch7", 100.0)],
        read_reg(0x00, 4),
        write_reg(0x21, &[0xAF]), // CS5 and CS7 off
        read_reg(0x00, 4),
        read_reg(0x10, 8),
        read_reg(0x21, 1),
        // A touch on a disabled channel does nothing at all.
        vec![Step::Input("touch5", 100.0)],
        read_reg(0x00, 4),
        // Interrupt enable gates the INT bit but not the status latch.
        write_reg(0x00, &[0x00]),
        write_reg(0x27, &[0x00]),
        vec![Step::Input("touch2", 80.0)],
        read_reg(0x00, 4),
        // Noise flags are write-one-to-clear.
        read_reg(0x0A, 1),
        write_reg(0x0A, &[0xFF]),
        read_reg(0x0A, 1),
        // An undeclared address reads 0 and swallows the write.
        write_reg(0x60, &[0x5A]),
        read_reg(0x60, 2),
    ])
}

/// What `components/cap1188.rs` put on the wire for [`migration_script`].
const CAP1188_GOLDEN: &[u8] = &[
    0x50, 0x5D, 0x83, 0x00, 0x00, 0x00, 0x00, 0x2F, 0x20, 0xFF, 0xFF, 0x40, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x11, 0x7F, 0x00, 0x3E, 0x00, 0x40, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x01, 0x11, 0x01, 0x00, 0x01, 0x10, 0x00, 0x00, 0x00, 0x00, 0xC0, 0xC1, 0x00, 0x01,
    0x50, 0xC1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3E, 0x00, 0x00, 0x00, 0x00, 0x00, 0xAF, 0xC1, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00, 0x00,
];

#[test]
fn cap1188_is_byte_identical() {
    let got = run_i2c(&mut dev(), &migration_script());
    assert_eq!(
        got.bytes,
        CAP1188_GOLDEN,
        "CAP1188 transcript moved.\nexpected:\n{}\ngot:\n{}",
        common::transcript::Transcript {
            bytes: CAP1188_GOLDEN.to_vec()
        }
        .render(),
        got.render()
    );
}

/// The latch, on its own, in the order that catches a driver out: press,
/// release, clear — and the status bit survives the release but not the clear.
#[test]
fn a_status_bit_survives_a_release_and_not_the_write_zero() {
    let mut d = dev();
    let t = run_i2c(
        &mut d,
        &script([
            vec![Step::Input("touch2", 100.0)],
            read_reg(0x03, 1),
            vec![Step::Input("touch2", 0.0)],
            read_reg(0x03, 1), // still latched
            read_reg(0x02, 1), // but the LIVE touch is gone
            write_reg(0x00, &[0x00]),
            read_reg(0x03, 1), // now cleared
        ]),
    );
    assert_eq!(
        t.bytes,
        vec![0x02, 0x02, 0x00, 0x00],
        "latched, still latched after release, GENERAL_STATUS live-clear, dropped by the write-0"
    );
}

/// The trap the model existed to reproduce: a pad still HELD re-asserts INT
/// inside the very write that was supposed to clear it, so a driver's
/// "acknowledge and move on" loop does not terminate while a finger is down.
#[test]
fn a_held_pad_reasserts_the_interrupt_in_the_clearing_write() {
    let mut d = dev();
    let t = run_i2c(
        &mut d,
        &script([
            vec![Step::Input("touch4", 100.0)],
            read_reg(0x00, 1),
            write_reg(0x00, &[0x00]),
            read_reg(0x00, 1), // INT is BACK
            read_reg(0x03, 1), // and so is the status bit
            vec![Step::Input("touch4", 0.0)],
            write_reg(0x00, &[0x00]),
            read_reg(0x00, 1),
            read_reg(0x03, 1),
        ]),
    );
    assert_eq!(
        t.bytes,
        vec![0x01, 0x01, 0x08, 0x00, 0x00],
        "asserted, re-asserted by the clear while held, and only then clearable"
    );
}

/// A disabled channel reports nothing, through all three places it was visible:
/// the live mask, the latch, and the delta count (`zero_unless`).
#[test]
fn a_disabled_channel_stops_reporting_immediately() {
    let mut d = dev();
    let t = run_i2c(
        &mut d,
        &script([
            vec![Step::Input("touch6", 100.0)],
            read_reg(0x02, 2),        // GENERAL_STATUS, SENSOR_INPUT_STATUS
            read_reg(0x15, 1),        // DELTA_COUNT_6
            write_reg(0x21, &[0xDF]), // CS6 off
            read_reg(0x02, 2),
            read_reg(0x15, 1),
            // A press on a disabled channel never registers at all.
            vec![Step::Input("touch6", 100.0)],
            read_reg(0x02, 2),
        ]),
    );
    assert_eq!(
        t.bytes,
        vec![0x01, 0x20, 0x7F, 0x00, 0x00, 0x00, 0x00, 0x00],
        "live + latched + 127 counts, then nothing at all"
    );
}

/// INTERRUPT_ENABLE gates the INT bit and NOT the status latch, which is the
/// half of the datasheet a polling driver depends on.
#[test]
fn interrupt_enable_gates_the_int_bit_and_not_the_latch() {
    let mut d = dev();
    let t = run_i2c(
        &mut d,
        &script([
            write_reg(0x27, &[0x00]),
            vec![Step::Input("touch8", 100.0)],
            read_reg(0x00, 1),
            read_reg(0x03, 1),
        ]),
    );
    assert_eq!(
        t.bytes,
        vec![0x00, 0x80],
        "no interrupt, but the status bit is there for a poll to find"
    );
}

/// **DELIBERATE DIFFERENCE.** The model stored the injected touch strength as a
/// whole percent (`percent.round() as u8`) and converted that; the descriptor
/// converts the percentage as given. They agree at every whole percent — the
/// only values the model could hold — and differ inside one, where the
/// descriptor is the finer answer. Both quantise to the same 127-count scale.
#[test]
fn a_fractional_percent_is_no_longer_rounded_before_conversion() {
    let mut d = dev();
    let t = run_i2c(
        &mut d,
        &script([vec![Step::Input("touch1", 49.6)], read_reg(0x10, 1)]),
    );
    assert_eq!(
        t.bytes[0], 62,
        "49.6 % is 62 counts. The model rounded to 50 % first and answered 63."
    );
    // And every whole percent still lands exactly where the model put it.
    for pct in 0..=100u32 {
        let got = run_i2c(
            &mut d,
            &script([
                vec![Step::Input("touch1", f64::from(pct))],
                read_reg(0x10, 1),
            ]),
        );
        assert_eq!(
            u32::from(got.bytes[0]),
            (pct * 127 / 100).min(127),
            "{pct} % must encode exactly as the model's integer `pct * 127 / 100`"
        );
    }
}
