// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Config-build gate for the native i.MX RT1064 / Teensy 4.1 descriptors.
//!
//! Teensy 4.1 silicon is MIMXRT1062; this model is the RT1064-class cousin
//! (GPIO / LPUART / CCM class). SIM-DERIVED.

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
fn imxrt1064_from_config_builds() {
    let sys = workspace_root().join("configs/systems/teensy-41.yaml");
    let mut manifest =
        SystemManifest::from_file(&sys).unwrap_or_else(|e| panic!("load teensy-41: {e}"));
    let chip_path = sys.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip for teensy-41: {e}"));
    manifest.chip = chip_path.to_str().expect("utf-8 chip path").to_string();
    let bus = SystemBus::from_config(&chip, &manifest).expect("imxrt1064 / teensy-41 must build");

    assert_eq!(chip.name, "imxrt1064");
    assert_eq!(chip.core.as_deref(), Some("cortex-m7"));
    // XIP skipped: image linked in DTCM. cortex-m-rt needs non-overlapping
    // FLASH/RAM, so lower DTCM is flash and upper DTCM is ram.
    assert_eq!(chip.flash.base, 0x2000_0000);
    assert!(
        (0x2000_0000..0x2002_0000).contains(&chip.ram.base),
        "ram must sit in DTCM @ 0x20000000..0x20020000, got {:#x}",
        chip.ram.base
    );

    assert!(
        bus.find_peripheral_index_by_name("lpuart6").is_some(),
        "bus must expose lpuart6 (Teensy Serial1)"
    );
    assert!(
        bus.find_peripheral_index_by_name("gpio2").is_some(),
        "bus must expose gpio2 (LED GPIO2_IO03)"
    );
    assert!(
        bus.find_peripheral_index_by_name("ccm").is_some(),
        "bus must expose ccm (imx_ccm)"
    );
    assert!(
        bus.find_peripheral_index_by_name("iomuxc").is_some(),
        "bus must expose iomuxc (imx_iomuxc)"
    );
    assert!(
        bus.find_peripheral_index_by_name("systick").is_some(),
        "bus must expose systick"
    );
    assert!(
        bus.find_peripheral_index_by_name("flexspi").is_some(),
        "bus must expose flexspi stub"
    );
}
