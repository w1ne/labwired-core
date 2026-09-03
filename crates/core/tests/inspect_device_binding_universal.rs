// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The contract, stated without reference to how the engine is built:
//!
//! > Every device an author places in `external_devices:` and that the engine
//! > actually built appears in `inspect`'s `devices` array, named as the author
//! > named it, wired where the author wired it.
//!
//! Nothing here knows what a "controller" is, or that I²C devices hang off a
//! peripheral while an HC-SR04 hangs off the bus. The expectations are read out
//! of the shipped `system.yaml` files with a YAML parser — the same text the
//! customer wrote — and compared against the inspect record. If the engine
//! grows a new way to bind a device and forgets to route it through the walk,
//! the rig that places one fails here.
//!
//! This is deliberately NOT the mirror of `inspect_external_devices.rs`, which
//! asserts I²C/SPI topology against a manifest embedded in the test. These
//! manifests are the ones that ship in `examples/`, and they were chosen for
//! covering the transports that are NOT I²C/SPI: GPIO (HC-SR04, TM1637),
//! analog (NTC thermistor on an ADC channel) and CAN (a UDS tester node, a
//! candump replay node).

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::inspect::InspectOpts;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Machine;
use std::path::PathBuf;

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// What the AUTHOR wrote, parsed straight out of the shipped YAML.
#[derive(Debug)]
struct Placed {
    id: String,
    device_type: String,
    connection: String,
}

fn placed_devices(rel_yaml: &str) -> Vec<Placed> {
    let text = std::fs::read_to_string(repo(rel_yaml)).expect("read system.yaml");
    let doc: serde_yaml::Value = serde_yaml::from_str(&text).expect("parse system.yaml");
    doc.get("external_devices")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .map(|e| Placed {
                    id: e["id"].as_str().expect("id").to_string(),
                    device_type: e["type"].as_str().expect("type").to_string(),
                    connection: e["connection"].as_str().expect("connection").to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Assemble the rig with no firmware. Placement is a property of the manifest;
/// nothing has to run for a device to be wired, and a test that booted firmware
/// first would be measuring the boot.
fn machine_from_example(rel_yaml: &str) -> Machine<CortexM> {
    let yaml = repo(rel_yaml);
    let manifest = SystemManifest::from_file(&yaml).expect("load system.yaml");
    let chip_path = yaml.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    bus.refresh_peripheral_index();
    Machine::new(cpu, bus)
}

/// The whole contract, for one shipped rig.
fn assert_every_placed_device_is_inspectable(rel_yaml: &str) {
    let placed = placed_devices(rel_yaml);
    assert!(
        !placed.is_empty(),
        "{rel_yaml} places no external devices — wrong fixture for this test"
    );
    let machine = machine_from_example(rel_yaml);
    let inspect = machine.inspect(None, &InspectOpts::default());
    let seen: Vec<&str> = inspect.devices.iter().map(|d| d.id.as_str()).collect();

    for want in &placed {
        let got = inspect
            .devices
            .iter()
            .find(|d| d.id == want.id)
            .unwrap_or_else(|| {
                panic!(
                    "{rel_yaml}: device '{}' ({}) is placed on '{}' but does not appear in \
                     inspect's devices array; inspect reported {seen:?}",
                    want.id, want.device_type, want.connection
                )
            });
        assert!(
            got.declared,
            "{rel_yaml}: '{}' is declared in the manifest, so the record must say so",
            want.id
        );
        assert_eq!(
            got.device_type.as_deref(),
            Some(want.device_type.as_str()),
            "{rel_yaml}: '{}' reports the type the author wrote",
            want.id
        );
        assert_eq!(
            got.attachment.bus.as_deref(),
            Some(want.connection.as_str()),
            "{rel_yaml}: '{}' reports the connection the author wrote",
            want.id
        );
    }
}

/// GPIO, two ways: a pulse-echo ultrasonic sensor on TRIG/ECHO pins, alongside
/// an SPI panel on the same rig. The panel was already visible; the sensor was
/// not, and both must be.
#[test]
fn hc_sr04_on_gpio_pins_is_inspectable() {
    assert_every_placed_device_is_inspectable("examples/nokia5110-invaders-lab/system.yaml");
}

/// A bit-banged two-wire display: no controller peripheral owns it, the GPIO
/// write-hook clocks it.
#[test]
fn tm1637_bit_banged_display_is_inspectable() {
    assert_every_placed_device_is_inspectable("examples/tm1637-7seg-lab/system.yaml");
}

/// An analog part: it drives an ADC channel's level, it has no bus address at
/// all, and it is exactly the shape ("no address, no chip-select") the record
/// already supports for an SPI panel.
#[test]
fn analog_thermistor_is_inspectable() {
    assert_every_placed_device_is_inspectable("examples/ntc-thermistor-lab/system.yaml");
}

/// A CAN node: a second, off-board ECU-tester participant on `fdcan1`.
#[test]
fn can_uds_tester_node_is_inspectable() {
    assert_every_placed_device_is_inspectable("examples/h563-uds-ecu/system.yaml");
}

/// A candump replay node on classic bxCAN — a different CAN family, and a
/// different attach path, from the UDS tester above.
#[test]
fn can_log_player_node_is_inspectable() {
    assert_every_placed_device_is_inspectable("examples/f103-j1939-monitor/replay-system.yaml");
}

/// Not an assertion — the reproducible way to produce the `devices` array for
/// several rigs as JSON, so a change to this seam can be diffed against the
/// same dump taken before it. `cargo test -p labwired-core --test
/// inspect_device_binding_universal -- --ignored --nocapture`.
///
/// The bay-occupancy rig is the one that must not move: it is a customer
/// delivery, it is I²C/SPI only, and its dump is expected to be byte-identical
/// across any change to bus-resident binding.
#[test]
#[ignore = "diagnostic dump, not an assertion"]
fn dump_devices_json() {
    let bay = {
        let yaml = repo("examples/esp32-bay-occupancy/system.yaml");
        let manifest = SystemManifest::from_file(&yaml).expect("load system.yaml");
        let mut bus = SystemBus::new();
        let cpu = labwired_core::system::xtensa::configure_xtensa_esp32(&mut bus);
        labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, &manifest)
            .expect("attach");
        bus.refresh_peripheral_index();
        Machine::new(cpu, bus).inspect(None, &InspectOpts::default())
    };
    println!("=== esp32-bay-occupancy ===");
    println!("{}", serde_json::to_string_pretty(&bay.devices).unwrap());

    for rig in [
        "examples/nokia5110-invaders-lab/system.yaml",
        "examples/tm1637-7seg-lab/system.yaml",
        "examples/ntc-thermistor-lab/system.yaml",
        "examples/h563-uds-ecu/system.yaml",
        "examples/f103-j1939-monitor/replay-system.yaml",
    ] {
        let inspect = machine_from_example(rig).inspect(None, &InspectOpts::default());
        println!("=== {rig} ===");
        println!(
            "{}",
            serde_json::to_string_pretty(&inspect.devices).unwrap()
        );
    }
}

/// The fails-if-forked guard, at the source level.
///
/// The walk cannot discover a collection of device models by reflection, so
/// every one the bus holds has to be listed in
/// `SystemBus::for_each_bus_resident_device`. A collection added to `SystemBus`
/// and NOT listed there is a device family that simulates perfectly and reports
/// nothing — silent, partial, and noticed by whichever customer happens to
/// place one. This catches it at the source instead.
///
/// Field names are read out of `bus/mod.rs`; a collection counts as holding
/// device models if its type names one of the model modules. The known-10
/// assertion below is not the gate — it is the anti-vacuity check, so a parser
/// that silently matched nothing cannot pass this test by finding no work.
#[test]
fn attached_device_walk_covers_every_bus_collection() {
    let src = core_src("bus/mod.rs");
    let walk = std::fs::read_to_string(core_src("bus/attached_devices.rs")).expect("read walk");
    let collections = system_bus_device_collections(&src);

    for known in [
        "hcsr04",
        "gpio_devices",
        // The readback-only registry: six typed `Vec<Arc<Concrete>>` fields
        // (ws2812 / servos / step_dir_motors / h_bridge_motors /
        // ili9341_parallel / unipolar_steppers) collapsed into ONE list of
        // `dyn ObservedDevice`, walked by ONE arm.
        "observed",
        "tm1637",
        "hx711",
        "seven_segment",
        "analog_inputs",
        "can_diagnostic_testers",
        "can_uds_testers",
        "can_log_players",
    ] {
        assert!(
            collections.iter().any(|c| c == known),
            "field scan lost '{known}' — the scan itself is broken, so this test \
             would have passed by measuring nothing; found {collections:?}"
        );
    }

    for field in &collections {
        assert!(
            walk.contains(&format!("self.{field}")),
            "SystemBus::{field} holds device models but \
             bus/attached_devices.rs never walks it — every device in it would \
             simulate and inspect as nothing. Add an arm to \
             `for_each_bus_resident_device`."
        );
    }
}

fn core_src(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(rel)
}

/// Field names of every `SystemBus` collection that holds external-device
/// models, parsed out of the struct declaration.
fn system_bus_device_collections(path: &std::path::Path) -> Vec<String> {
    const MODEL_MARKERS: [&str; 8] = [
        "peripherals::components::",
        "peripherals::hc_sr04::",
        "BusResidentDevice",
        "ObservedDevice",
        "CanDiagnosticTester",
        "CanUdsTester",
        "CanLogPlayer",
        "AnalogInputSource",
    ];
    let src = std::fs::read_to_string(path).expect("read bus/mod.rs");
    let body = src
        .split_once("pub struct SystemBus {")
        .expect("SystemBus declaration")
        .1;
    let body = body.split_once("\n}").expect("end of struct").0;

    let mut out = Vec::new();
    let mut decl = String::new();
    let mut depth = 0i32;
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.is_empty() {
            continue;
        }
        decl.push_str(line);
        depth += line.matches('<').count() as i32;
        depth -= line.matches('>').count() as i32;
        if depth > 0 || !decl.ends_with(',') {
            continue;
        }
        let one = std::mem::take(&mut decl);
        let Some(rest) = one.strip_prefix("pub ") else {
            continue;
        };
        let Some((name, ty)) = rest.split_once(':') else {
            continue;
        };
        if ty.contains("Vec<") && MODEL_MARKERS.iter().any(|m| ty.contains(m)) {
            out.push(name.trim().to_string());
        }
    }
    out
}

/// The rig this whole seam was built for. It is I²C/SPI only, so it passed
/// before; it is here so a change that fixes the new transports by breaking the
/// old ones cannot pass.
#[test]
fn bay_occupancy_i2c_and_spi_still_inspectable() {
    let placed = placed_devices("examples/esp32-bay-occupancy/system.yaml");
    let yaml = repo("examples/esp32-bay-occupancy/system.yaml");
    let manifest = SystemManifest::from_file(&yaml).expect("load system.yaml");
    let mut bus = SystemBus::new();
    let cpu = labwired_core::system::xtensa::configure_xtensa_esp32(&mut bus);
    labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, &manifest)
        .expect("attach external devices");
    bus.refresh_peripheral_index();
    let machine = Machine::new(cpu, bus);
    let inspect = machine.inspect(None, &InspectOpts::default());
    let seen: Vec<&str> = inspect.devices.iter().map(|d| d.id.as_str()).collect();
    for want in &placed {
        assert!(
            inspect
                .devices
                .iter()
                .any(|d| d.id == want.id && d.declared),
            "'{}' placed but not inspectable; got {seen:?}",
            want.id
        );
    }
}
