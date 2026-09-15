// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Config-build gate for the native SAMD51J19A / Metro M4 descriptors.

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
fn atsamd51_from_config_builds() {
    let sys = workspace_root().join("configs/systems/metro-m4.yaml");
    let mut manifest =
        SystemManifest::from_file(&sys).unwrap_or_else(|e| panic!("load metro-m4: {e}"));
    let chip_path = sys.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip for metro-m4: {e}"));
    manifest.chip = chip_path.to_str().expect("utf-8 chip path").to_string();
    let bus = SystemBus::from_config(&chip, &manifest).expect("atsamd51 / metro-m4 must build");
    // Metro M4 Serial1 is SERCOM3 on PA22/PA23 (Adafruit variants/metro_m4/variant.cpp).
    assert!(
        bus.find_peripheral_index_by_name("sercom3").is_some(),
        "bus must expose sercom3 (Serial1)"
    );
    assert!(
        bus.find_peripheral_index_by_name("mclk").is_some(),
        "bus must expose mclk"
    );
    assert!(
        bus.find_peripheral_index_by_name("gclk").is_some(),
        "bus must expose gclk"
    );
}
