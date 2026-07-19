// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end stimulus coverage for the two analog input components (NTC
//! thermistor, potentiometer).
//!
//! The component-level math is unit-tested next to each model. What this file
//! covers is the part that used to be missing entirely: that the model is
//! RETAINED on the bus and that a generic `SystemBus::set_input` actually moves
//! the ADC channel the firmware converts. Before the SimInput unification the
//! kit computed one boot voltage and dropped the model, so these parts were
//! discoverable in no API and drivable through none — a component unit test
//! would still have passed. This asserts the wiring, not the arithmetic.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;

fn root(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Build a bus from an example system.yaml, resolving its relative `chip:`.
fn bus_from_example(rel_yaml: &str) -> SystemBus {
    let yaml = root(rel_yaml);
    let manifest = SystemManifest::from_file(&yaml).expect("load system.yaml");
    let chip_path = yaml.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    SystemBus::from_config(&chip, &manifest).expect("build bus")
}

/// The thermistor's `temperature` channel must be discoverable by the generic
/// input-discovery query, under the id the author wrote in system.yaml.
#[test]
fn ntc_temperature_channel_is_discoverable() {
    let mut bus = bus_from_example("examples/ntc-thermistor-lab/system.yaml");
    let inputs = bus.list_inputs();
    let found = inputs
        .iter()
        .find(|(_, ch)| ch.key == "temperature")
        .unwrap_or_else(|| panic!("no `temperature` channel; discovered: {inputs:?}"));
    assert_eq!(found.0, "thermistor", "owner should be the system.yaml id");
    assert_eq!(found.1.unit, "°C");
}

/// Driving the channel must move the ADC level, in the physically correct
/// direction: an NTC's resistance FALLS as it heats, so the pull-down divider
/// output RISES. Boot sits at 25 °C = Vref/2.
#[test]
fn ntc_temperature_drives_the_adc_channel() {
    let mut bus = bus_from_example("examples/ntc-thermistor-lab/system.yaml");

    let at_boot = adc_channel_count(&mut bus, "adc1", 0);
    assert!(
        (at_boot as i32 - 2047).abs() <= 3,
        "boot should be Vref/2 (~2047 counts) at 25 °C, got {at_boot}"
    );

    bus.set_input(None, "temperature", 80.0).expect("set hot");
    let hot = adc_channel_count(&mut bus, "adc1", 0);

    bus.set_input(None, "temperature", -10.0).expect("set cold");
    let cold = adc_channel_count(&mut bus, "adc1", 0);

    assert!(
        hot > at_boot && at_boot > cold,
        "expected hot > 25 °C > cold on a pull-down divider; \
         hot={hot} boot={at_boot} cold={cold}"
    );
}

/// Out-of-range values are rejected by the generic range check, and a rejected
/// set must NOT have disturbed the ADC.
#[test]
fn ntc_out_of_range_is_rejected_and_leaves_the_adc_untouched() {
    let mut bus = bus_from_example("examples/ntc-thermistor-lab/system.yaml");
    let before = adc_channel_count(&mut bus, "adc1", 0);
    assert!(bus.set_input(None, "temperature", 5000.0).is_err());
    assert_eq!(before, adc_channel_count(&mut bus, "adc1", 0));
}

/// Two potentiometers on one bus stay individually addressable by their
/// system.yaml ids — the `component` disambiguator. Both expose `position`, so
/// an un-narrowed set must be a typed ambiguity error rather than "first wins",
/// and each narrowed set must move only its OWN ADC channel.
#[test]
fn two_potentiometers_are_individually_addressable() {
    let chip_path = root("configs/chips/stm32f103.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let manifest = SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "two-pots".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        external_devices: vec![pot("knob", 0), pot("fader", 1)],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");

    let positions: Vec<_> = bus
        .list_inputs()
        .into_iter()
        .filter(|(_, ch)| ch.key == "position")
        .collect();
    assert_eq!(
        positions.len(),
        2,
        "expected knob + fader, got {positions:?}"
    );
    let owners: Vec<_> = positions.iter().map(|(o, _)| o.as_str()).collect();
    assert!(
        owners.contains(&"knob") && owners.contains(&"fader"),
        "owners should be the system.yaml ids, got {owners:?}"
    );

    // Un-narrowed: ambiguous, and nothing applied.
    assert!(matches!(
        bus.set_input(None, "position", 10.0),
        Err(labwired_core::sim_input::SimInputError::Ambiguous { .. })
    ));

    // Narrowed by component id: each drives its own ADC channel only.
    bus.set_input(Some("knob"), "position", 0.0).expect("knob");
    bus.set_input(Some("fader"), "position", 100.0)
        .expect("fader");

    let knob = adc_channel_count(&mut bus, "adc1", 0);
    let fader = adc_channel_count(&mut bus, "adc1", 1);
    assert!(knob < 40, "knob at 0 % should be ~0 counts, got {knob}");
    assert!(
        fader > 4000,
        "fader at 100 % should be ~4095 counts, got {fader}"
    );
}

/// A potentiometer `external_devices` entry on `adc1`, at `channel`.
fn pot(id: &str, channel: u64) -> labwired_config::ExternalDevice {
    let mut config = std::collections::HashMap::new();
    config.insert("channel".to_string(), serde_yaml::Value::from(channel));
    labwired_config::ExternalDevice {
        id: id.to_string(),
        r#type: "potentiometer".to_string(),
        connection: "adc1".to_string(),
        route: Default::default(),
        config,
    }
}

// ─── helper ────────────────────────────────────────────────────────────────

/// Read back a channel's injected 12-bit count from whichever ADC controller
/// holds it. Mirrors `SystemBus::seed_adc_channel`'s two-controller lookup,
/// including its name fallback (S3 manifests say `sar_adc_s3`; the peripheral
/// is registered as `sens_s3`). Both controllers store counts, not mV:
/// 0 mV = 0, Vref/2 = 2047, 3300 mV = 4095.
fn adc_channel_count(bus: &mut SystemBus, connection: &str, channel: u8) -> u16 {
    let indices: Vec<usize> = bus
        .find_peripheral_index_by_name(connection)
        .into_iter()
        .chain(0..bus.peripherals.len())
        .collect();
    for idx in indices {
        let Some(any) = bus.peripherals[idx].dev.as_any_mut() else {
            continue;
        };
        if let Some(adc) = any.downcast_mut::<labwired_core::peripherals::adc::Adc>() {
            return adc.channel_input_count(channel);
        }
        if let Some(sens) =
            any.downcast_mut::<labwired_core::peripherals::esp32s3::sens::Esp32s3Sens>()
        {
            return sens.channel_input_count(channel);
        }
    }
    panic!("no ADC controller found for connection '{connection}'");
}
