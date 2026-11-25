// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! A CH32V003 pad turns a driven circuit voltage into the level the firmware
//! reads.
//!
//! The chip descriptor declared no `io_voltage_v` and no
//! `gpio_input_thresholds`, so a co-simulation model that drove a gate net into
//! PC4/PC3 refused to bind (`NoInputThresholds`) and two imported HeyPCB boards
//! could not start. The descriptor now transcribes the datasheet's limits,
//! which are affine in VDD rather than a single ratio:
//!
//! WCH CH32V003DS0 V1.8, Table 3-16 "General I/O static characteristics",
//! VDD 2.7-5.5 V, standard and FT pads:
//!   VIL(max) = 0.19*(VDD - 2.7) + 0.65 V = 0.19*VDD + 0.137 V
//!   VIH(min) = 0.22*(VDD - 2.7) + 1.55 V = 0.22*VDD + 0.956 V
//!
//! The test drives the route the importer produces and reads the GPIO input
//! register the firmware samples, so it fails at binding before the descriptor
//! fix and at the register if the thresholds are wrong.

use crate::bus::SystemBus;
use crate::cosim::{CosimSession, InputThresholds};
use crate::Bus;
use labwired_config::{ChipDescriptor, CosimAdapter, CosimModelConfig, SystemManifest};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn repo_root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn load(chip: &str) -> (SystemBus, ChipDescriptor) {
    let path = repo_root(&format!("configs/chips/{chip}.yaml"));
    let descriptor = ChipDescriptor::from_file(&path).expect("load chip descriptor");
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: \"ch32v003-input-levels\"\nchip: \"{}\"\nexternal_devices: []\n",
        path.display()
    ))
    .expect("parse manifest");
    let bus = SystemBus::from_config(&descriptor, &manifest).expect("build bus");
    (bus, descriptor)
}

/// A `mock` model holds one voltage on `board.gpio_in.pc4`, the gate net an
/// imported low-side LED switch drives.
fn gate_voltage(volts: f64) -> CosimModelConfig {
    let mut config = HashMap::new();
    let mut outputs = serde_yaml::Mapping::new();
    outputs.insert(
        serde_yaml::Value::String("gate".to_string()),
        serde_yaml::Value::from(volts),
    );
    config.insert("outputs".to_string(), serde_yaml::Value::Mapping(outputs));
    CosimModelConfig {
        id: "gate_drive".to_string(),
        adapter: CosimAdapter::Mock,
        model: None,
        step_ns: 1_000,
        inputs: HashMap::new(),
        outputs: HashMap::from([("gate".to_string(), "board.gpio_in.pc4".to_string())]),
        config,
    }
}

fn read_pc4(bus: &SystemBus) -> bool {
    let (addr, bit) =
        SystemBus::resolve_pin_idr_pub(bus, "pc4").expect("PC4 resolves to a GPIO input register");
    bus.read_u32(addr).expect("read the GPIO input register") >> bit & 1 != 0
}

/// Every CH32V003 package is the same die, so each copies the electrical block
/// and resolves the same thresholds at 3.3 V and at 5 V.
#[test]
fn every_ch32v003_package_transcribes_the_datasheet_input_limits() {
    for chip in [
        "ch32v003f4p6",
        "ch32v003a4m6",
        "ch32v003f4u6",
        "ch32v003j4m6",
    ] {
        let (bus, descriptor) = load(chip);
        assert_eq!(descriptor.io_voltage_v, Some(3.3), "{chip} nominal rail");
        let limits = descriptor
            .gpio_input_thresholds
            .unwrap_or_else(|| panic!("{chip} declares input thresholds"));
        assert!((limits.vil - 0.19).abs() < 1e-12, "{chip} VIL slope");
        assert!((limits.vih - 0.22).abs() < 1e-12, "{chip} VIH slope");
        assert!(
            (limits.vil_offset_v - 0.137).abs() < 1e-12,
            "{chip} VIL offset"
        );
        assert!(
            (limits.vih_offset_v - 0.956).abs() < 1e-12,
            "{chip} VIH offset"
        );

        let at_3v3 = InputThresholds::for_bus(&bus).expect("thresholds resolve");
        assert!(
            (at_3v3.vil_volts - 0.764).abs() < 1e-9,
            "{chip} VIL at 3.3 V"
        );
        assert!(
            (at_3v3.vih_volts - 1.682).abs() < 1e-9,
            "{chip} VIH at 3.3 V"
        );
        // A routed 5 V rail moves both limits; a ratio transcribed at one rail
        // would not.
        let at_5v = InputThresholds::from_ratios(5.0, limits);
        assert!((at_5v.vil_volts - 1.087).abs() < 1e-9, "{chip} VIL at 5 V");
        assert!((at_5v.vih_volts - 2.056).abs() < 1e-9, "{chip} VIH at 5 V");
    }
}

/// The imported board's route binds, and the firmware-visible pin follows the
/// voltage. 2.4 V is above VIH and 0.5 V below VIL. 0.9 V sits inside the
/// Schmitt band, where the pad keeps the level it had (low, from reset); the
/// datasheet's guaranteed limits are not a midpoint comparison.
#[test]
fn a_driven_gate_net_sets_the_ch32v003_pin_level() {
    for (volts, expected) in [(2.4, true), (0.5, false), (0.9, false)] {
        let (mut bus, _) = load("ch32v003f4p6");
        // Firmware enables the GPIOC clock before reading INDR: APB2PCENR
        // (RCC + 0x18, ch32v00x.h) bit 4. The bus gates an unclocked window
        // exactly as the silicon does, so a test that skips this reads 0.
        bus.write_u32(0x4002_1018, 1 << 4)
            .expect("enable the GPIOC clock");

        let models = [gate_voltage(volts)];
        let mut session = CosimSession::new(&models, Path::new("."), &bus)
            .expect("build session")
            .expect("the model declares a session");
        assert!(
            session.binding_errors().is_empty(),
            "{volts} V must bind: {:?}",
            session.binding_errors()
        );
        // 48 MHz core, 1 us model period: the first boundary is 48 cycles.
        let (_, errors) = session
            .advance_to(48, &mut bus)
            .expect("advance to the first boundary");
        assert!(errors.is_empty(), "{volts} V routed with {errors:?}");
        assert_eq!(read_pc4(&bus), expected, "{volts} V on PC4");
    }
}
