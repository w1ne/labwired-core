// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The readback-only device registry reports what the six typed fields did.
//!
//! `SystemBus` used to carry six `Vec<Arc<Concrete>>` fields — a WS2812 strip,
//! a hobby servo, a STEP/DIR stepper, an H-bridge channel, a parallel ILI9341
//! panel, a unipolar stepper — one public field per off-chip part the bus does
//! nothing with, each with its own arm in the attached-device walk. They are
//! now one `Vec<Arc<dyn ObservedDevice>>` walked by one arm.
//!
//! Collapsing six arms into one means the identity logic each arm carried has
//! to survive on the models. Five of the six were `id()` and nothing else. The
//! sixth — the H-bridge — did real work and had no behavioural test at all:
//! one `external_devices:` declaration builds TWO models (`<id>-a`, `<id>-b`),
//! which both join back to that one declaration while each reports its own
//! channel identity. Get it wrong and a two-motor board inspects as one
//! anonymous device, or as two devices both claiming to be the whole board.
//!
//! So that is what these pin, alongside a plain single-model part sharing the
//! same registry. They are written against `inspect` rather than against the
//! registry, because `inspect` is what a customer sees.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::inspect::{DeviceInspect, InspectOpts};
use std::path::PathBuf;

/// An ESP32-C3 with a dual-channel L298N and a servo on plain GPIO. Both are
/// readback-only parts: the bus holds them and drives neither.
const TWO_CHANNEL_BOARD: &str = r#"
name: observed-registry-rig
chip: "../chips/esp32.yaml"
external_devices:
  - id: drive
    type: l298n
    connection: gpio
    config:
      in1_pin: "GPIO0"
      in2_pin: "GPIO1"
      in3_pin: "GPIO2"
      in4_pin: "GPIO3"
  - id: tilt
    type: servo
    connection: gpio
    config:
      pin: "GPIO5"
      model: sg90
"#;

fn rig(yaml: &str) -> SystemBus {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32.yaml"))
        .expect("read ESP32 chip descriptor");
    let manifest: SystemManifest = serde_yaml::from_str(yaml).expect("parse manifest");
    SystemBus::from_config(&chip, &manifest).expect("build bus")
}

fn devices(yaml: &str) -> Vec<DeviceInspect> {
    rig(yaml).inspect_devices(None, &InspectOpts::default())
}

/// One declaration, two motor channels, and the join keeps both facts.
///
/// This is the arm with the most logic in it and it had no test of its own.
/// After the collapse the same two facts have to come off the model
/// (`manifest_id` / `model_id`) instead of off a hand-written arm.
#[test]
fn one_h_bridge_declaration_inspects_as_two_named_channels() {
    let devices = devices(TWO_CHANNEL_BOARD);

    let channels: Vec<&DeviceInspect> = devices
        .iter()
        .filter(|d| d.id == "drive" || d.id.starts_with("drive-"))
        .collect();

    assert_eq!(
        channels.len(),
        2,
        "an L298N with IN1..IN4 is two independent motors, so it must inspect \
         as two devices — got {:?}",
        devices.iter().map(|d| &d.id).collect::<Vec<_>>()
    );

    // Both are the author's declaration, not a synthesized name: a channel that
    // failed to join would report `declared: false` and an invented id.
    for ch in &channels {
        assert!(
            ch.declared,
            "channel {:?} lost its join to the `drive` declaration — it would \
             show up in a customer's rig as an unnamed device",
            ch.id
        );
    }

    // …and they are distinguishable from each other. Two entries that both
    // answered to "drive" would be worse than one: the UI could not tell the
    // motors apart. The channel identity IS the reported id.
    let ids: Vec<&str> = channels.iter().map(|c| c.id.as_str()).collect();
    assert!(
        ids.contains(&"drive-a") && ids.contains(&"drive-b"),
        "each channel must report its own identity (`drive-a` / `drive-b`); got {ids:?}"
    );
}

/// A servo shares the same registry and must not be disturbed by it.
///
/// Anti-vacuity for the test above as much as anything: if the registry walk
/// emitted nothing at all, the H-bridge assertions could only fail on the
/// count, and a reader could not tell a broken join from a broken walk.
#[test]
fn a_servo_in_the_same_registry_still_reports_under_its_own_id() {
    let devices = devices(TWO_CHANNEL_BOARD);
    let servo = devices.iter().find(|d| d.id == "tilt").unwrap_or_else(|| {
        panic!(
            "servo missing; got {:?}",
            devices.iter().map(|d| &d.id).collect::<Vec<_>>()
        )
    });
    assert!(servo.declared, "the servo is declared, not synthesized");
}

/// Every readback-only part reaches the walk, whatever type it is.
///
/// The collapse replaced six per-type arms with one, so the failure mode moved:
/// it is no longer "somebody forgot an arm" but "somebody's model is not in the
/// registry". Both look the same from outside — a part that simulates and
/// reports nothing — so this counts what came back.
#[test]
fn the_registry_reports_one_entry_per_model_it_holds() {
    let bus = rig(TWO_CHANNEL_BOARD);

    // Two H-bridge channels + one servo.
    assert_eq!(
        bus.observed.len(),
        3,
        "registry holds one model per readback-only part: {:?}",
        bus.observed
            .iter()
            .map(|d| d.manifest_id())
            .collect::<Vec<_>>()
    );

    let mut walked = 0usize;
    bus.for_each_attached_device(&mut |_, _| walked += 1);
    assert!(
        walked >= bus.observed.len(),
        "the walk emitted {walked} devices but the bus holds {} readback-only \
         models alone — a model the bus holds and the walk does not emit is a \
         device that simulates and inspects as nothing",
        bus.observed.len()
    );
}
