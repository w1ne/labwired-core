// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Pin-map resolution tests.

use super::*;
use labwired_config::{ChipDescriptor, SystemManifest};

fn chip_bus(chip_name: &str) -> SystemBus {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../configs/chips/{chip_name}.yaml"));
    let chip = ChipDescriptor::from_file(&path).expect("load chip");
    let manifest = SystemManifest {
        max_peripheral_tick_interval: None,
        parts: Vec::new(),
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "pinmap-test".to_string(),
        chip: path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).expect("assemble bus")
}

#[test]
fn pin_map_populated_from_chip_pins() {
    let bus = chip_bus("mkw41z4");
    assert_eq!(bus.pin_map.get("PC0"), Some(&("gpioc".to_string(), 0u8)));
    // KW41Z remap: "PB6" labels a gpioc pin, not gpiob.
    assert_eq!(bus.pin_map.get("PB6"), Some(&("gpioc".to_string(), 2u8)));
    assert_eq!(bus.pin_map.len(), 8);
}

/// Architect production smoke `catalog-gpio-output` drives a seven-segment
/// display from G474 PA0. A partial pin map refused that pad, so every
/// general-purpose pad, and the TIM1_CH1 pad PA8, must resolve to an output.
#[test]
fn stm32g474re_resolves_general_purpose_outputs() {
    let bus = chip_bus("stm32g474re");
    for pin in ["PA0", "PA5", "PA8", "PB0", "PC13"] {
        assert!(
            SystemBus::resolve_pin_odr(&bus, pin).is_some(),
            "{pin} must resolve to a GPIO output on stm32g474re"
        );
    }
}

/// Board compilers use the generic `GPIO<n>` spelling while the RP2040 chip
/// descriptor follows the Pico pinout's `GP<n>` spelling. Both names must go
/// through the descriptor's same authoritative pad map.
#[test]
fn rp2040_generic_gpio_labels_resolve_to_gp_pad_map_entries() {
    let bus = chip_bus("rp2040");

    for pin in [8, 9] {
        let pico_label = format!("GP{pin}");
        let generic_label = format!("GPIO{pin}");
        assert_eq!(
            SystemBus::resolve_pin_odr(&bus, &generic_label),
            SystemBus::resolve_pin_odr(&bus, &pico_label),
            "{generic_label} and {pico_label} must resolve to the same output pad"
        );
        assert_eq!(
            SystemBus::resolve_pin_idr(&bus, &generic_label),
            SystemBus::resolve_pin_idr(&bus, &pico_label),
            "{generic_label} and {pico_label} must resolve to the same input pad"
        );
    }

    assert!(
        SystemBus::resolve_pin_odr(&bus, "GPIO30").is_none(),
        "generic labels must not invent pads past the RP2040's GP29"
    );
}
