// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Pin-map resolution tests.

use super::*;
use labwired_config::{ChipDescriptor, SystemManifest};

fn mkw41z4_bus() -> SystemBus {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips/mkw41z4.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load mkw41z4");
    let manifest = SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "pinmap-test".to_string(),
        chip: path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).expect("assemble bus")
}

#[test]
fn pin_map_populated_from_chip_pins() {
    let bus = mkw41z4_bus();
    assert_eq!(bus.pin_map.get("PC0"), Some(&("gpioc".to_string(), 0u8)));
    // KW41Z remap: "PB6" labels a gpioc pin, not gpiob.
    assert_eq!(bus.pin_map.get("PB6"), Some(&("gpioc".to_string(), 2u8)));
    assert_eq!(bus.pin_map.len(), 8);
}
