// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Config-build gate for the native R7FA4M1AB / Arduino Uno R4 Minima descriptors.

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
fn ra4m1_from_config_builds() {
    let sys = workspace_root().join("configs/systems/arduino-uno-r4-minima.yaml");
    let mut manifest = SystemManifest::from_file(&sys)
        .unwrap_or_else(|e| panic!("load arduino-uno-r4-minima: {e}"));
    let chip_path = sys.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip for arduino-uno-r4-minima: {e}"));
    manifest.chip = chip_path.to_str().expect("utf-8 chip path").to_string();
    let bus =
        SystemBus::from_config(&chip, &manifest).expect("ra4m1 / arduino-uno-r4-minima must build");
    // Uno R4 Minima Serial is SCI2 on P301/P302 (ArduinoCore-renesas MINIMA).
    assert!(
        bus.find_peripheral_index_by_name("sci2").is_some(),
        "bus must expose sci2 (Serial)"
    );
    assert!(
        bus.find_peripheral_index_by_name("ra_sysc").is_some()
            || bus.find_peripheral_index_by_name("sysc").is_some(),
        "bus must expose ra_sysc / sysc (HOCO/OSCSF)"
    );
    assert!(
        bus.find_peripheral_index_by_name("port1").is_some(),
        "bus must expose port1 (LED P111)"
    );
}
