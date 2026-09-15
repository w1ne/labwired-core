// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Config-build gate for the native SAMD21G18A / Nano 33 IoT descriptors.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn atsamd21_from_config_builds() {
    let sys = workspace_root().join("configs/systems/nano-33-iot.yaml");
    let mut manifest =
        SystemManifest::from_file(&sys).unwrap_or_else(|e| panic!("load nano-33-iot: {e}"));
    let chip_path = sys.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip for nano-33-iot: {e}"));
    manifest.chip = chip_path.to_str().expect("utf-8 chip path").to_string();
    let bus = SystemBus::from_config(&chip, &manifest).expect("atsamd21 / nano-33-iot must build");
    // Nano 33 IoT Serial1 is SERCOM5 on PB22/PB23 (ArduinoCore-samd variant.cpp).
    assert!(
        bus.find_peripheral_index_by_name("sercom5").is_some(),
        "bus must expose sercom5 (Serial1), not sercom2"
    );
}
