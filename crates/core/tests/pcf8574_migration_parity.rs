// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! PCF8574: the declarative descriptor against the hand-written model it replaces.
//!
//! `components/pcf8574.rs` is DELETED. Everything it did on the wire is pinned
//! here as golden bytes, captured from that model before it was removed, so the
//! migration is a claim this file can refuse rather than a diff someone has to
//! trust.
//!
//! Held identical (the whole of the old model's observable behaviour):
//!   * power-on port reads 0xFF;
//!   * a one-byte write latches the port, and the next read returns it — the
//!     case that needed `pointer_bytes: 0`, because the old model had no
//!     pointer and the declarative register engine assumed one;
//!   * every byte of a multi-byte read returns the port again (the datasheet's
//!     "reading from the port": each clocked byte is a fresh read, not a walk
//!     off the end of a one-byte register);
//!   * the `port` stimulus channel writes the same state firmware writes.
//!
//! DELIBERATELY NEW, and asserted so it cannot be lost silently:
//!   * the port now moves PADS. The old model latched a byte nothing could
//!     observe; the descriptor declares eight `outputs:` and eight rules, and
//!     `pads_follow_the_written_port` proves the queue the bus drains.
//!
//! Still NOT modelled, in either model — see the descriptor header:
//!   * the INPUT direction (an external edge on a pad does not update the
//!     readable port) and the open-drain INT output. Both need an I²C device
//!     that OBSERVES pads, which this engine does not have.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;

mod common;
use common::transcript::{run_i2c, Step};

const ADDR: u8 = 0x20;

/// The descriptor under test, built from the EMBEDDED yaml so this fails if the
/// part is ever dropped from `embedded_device_yaml` — the one way it could
/// silently stop existing for wasm builds.
fn declarative() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("pcf8574")
        .expect("pcf8574 descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("pcf8574.yaml does not build")
}

/// The conversation the deleted model was driven through, in the order the
/// golden bytes below record.
///
/// Read it as a driver would write it: check the power-on state, latch a
/// pattern, read it back three times in one transaction, pose the port from the
/// host, read it, drive everything low, read twice.
fn migration_script() -> Vec<Step<'static>> {
    vec![
        // 1. Power-on read: the datasheet's all-ones.
        Step::Start,
        Step::Read(1),
        Step::Stop,
        // 2. One-byte write — the whole of the PCF8574 write protocol.
        Step::Start,
        Step::Write(0xA5),
        Step::Stop,
        // 3. Three bytes out of ONE read transaction.
        Step::Start,
        Step::Read(3),
        Step::Stop,
        // 4. The host poses the port, then firmware reads it.
        Step::Input("port", 15.0),
        Step::Start,
        Step::Read(1),
        Step::Stop,
        // 5. Everything low, read back twice.
        Step::Start,
        Step::Write(0x00),
        Step::Stop,
        Step::Start,
        Step::Read(2),
        Step::Stop,
    ]
}

/// ⚠️ GOLDEN — captured from `components/pcf8574.rs` (the hand-written model)
/// on the commit that deleted it, by running [`migration_script`] against it.
/// Not a value anyone chose: it is what the old part did.
const PCF8574_GOLDEN: &[u8] = &[0xFF, 0xA5, 0xA5, 0xA5, 0x0F, 0x00, 0x00];

#[test]
fn the_wire_transcript_is_byte_identical_to_the_deleted_model() {
    let transcript = run_i2c(&mut declarative(), &migration_script());
    assert_eq!(
        transcript.bytes,
        PCF8574_GOLDEN,
        "the declarative PCF8574 moved a byte the hand-written one did not.\n\
         got:\n{}\nexpected:\n{}\n\
         If a change here is deliberate, say which datasheet line justifies it \
         and re-bless with:\n  {}",
        transcript.render(),
        common::transcript::Transcript {
            bytes: PCF8574_GOLDEN.to_vec()
        }
        .render(),
        transcript.as_literal()
    );
}

#[test]
fn a_read_past_the_first_byte_still_reads_the_port() {
    // The regression this guards: a one-byte register read with the ordinary
    // pointer engine returns open-bus 0xFF from the second byte on. The
    // datasheet says otherwise, and so did the model being replaced — a
    // PCF8575-style driver that clocks two bytes would have seen 0xFF.
    let mut dev = declarative();
    dev.start();
    dev.write(0x3C);
    dev.stop();
    dev.start();
    let bytes: Vec<u8> = (0..4).map(|_| dev.read()).collect();
    dev.stop();
    assert_eq!(bytes, vec![0x3C; 4]);
}

#[test]
fn the_port_powers_up_all_high() {
    // Datasheet §8.1. A model that powered up at 0x00 would drive every pin of
    // an expander low at reset, which on a board with LEDs to Vcc lights all
    // eight before firmware runs.
    let mut dev = declarative();
    dev.start();
    let byte = dev.read();
    dev.stop();
    assert_eq!(byte, 0xFF);
}

// ─── the deliberately NEW half ─────────────────────────────────────────────

/// The port drives pads now. The old model could not: it held a byte and
/// nothing else in the engine could see it.
///
/// This reads the queue the bus drains
/// ([`I2cDevice::take_pin_drives`]), which is the device's whole contribution —
/// the pad resolution and the write belong to the bus and are proven on a real
/// machine in `tier2_device_pins.rs`.
#[test]
fn pads_follow_the_written_port() {
    let mut dev = declarative();
    // Reset queues nothing: no rule has run, so no pin has a level yet.
    assert!(dev.take_pin_drives().is_empty());

    dev.start();
    dev.write(0b1010_0101);
    dev.stop();
    let mut drives = dev.take_pin_drives();
    drives.sort();
    assert_eq!(
        drives,
        vec![
            ("P0".to_string(), true),
            ("P1".to_string(), false),
            ("P2".to_string(), true),
            ("P3".to_string(), false),
            ("P4".to_string(), false),
            ("P5".to_string(), true),
            ("P6".to_string(), false),
            ("P7".to_string(), true),
        ],
        "every pad takes its bit from the written port"
    );

    // Writing the SAME byte queues nothing: the queue carries transitions.
    dev.start();
    dev.write(0b1010_0101);
    dev.stop();
    assert!(
        dev.take_pin_drives().is_empty(),
        "an unchanged port must not re-drive the pads"
    );

    // One bit flips ⇒ exactly one transition.
    dev.start();
    dev.write(0b1010_0100);
    dev.stop();
    assert_eq!(dev.take_pin_drives(), vec![("P0".to_string(), false)]);
}

/// The host stimulus moves the pads too, not only the readable byte. Without
/// the `on: { input: port }` rules a posed port would be readable over I²C and
/// invisible on the pins — the "set_input returns Ok and the pin never moves"
/// failure in its I²C form.
#[test]
fn a_host_driven_port_also_moves_the_pads() {
    use labwired_core::sim_input::SimInput;
    let mut dev = declarative();
    dev.start();
    dev.write(0x00);
    dev.stop();
    let _ = dev.take_pin_drives();

    dev.set_input("port", 255.0).expect("port is a channel");
    let mut drives = dev.take_pin_drives();
    drives.sort();
    assert_eq!(drives.len(), 8, "all eight pads rose");
    assert!(drives.iter().all(|(_, level)| *level));
}
