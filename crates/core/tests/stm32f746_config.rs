// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Config-build gate for the native STM32F746 / STM32F7 Discovery descriptors.
//!
//! SIM-DERIVED — F4-class RCC/GPIO/USART reuse + Cortex-M7 DTCM map.
//! Not silicon-verified.

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
fn stm32f746_from_config_builds() {
    let sys = workspace_root().join("configs/systems/stm32f7-discovery.yaml");
    let mut manifest =
        SystemManifest::from_file(&sys).unwrap_or_else(|e| panic!("load stm32f7-discovery: {e}"));
    let chip_path = sys.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip for stm32f7-discovery: {e}"));
    manifest.chip = chip_path.to_str().expect("utf-8 chip path").to_string();
    let bus =
        SystemBus::from_config(&chip, &manifest).expect("stm32f746 / stm32f7-discovery must build");

    assert_eq!(chip.core.as_deref(), Some("cortex-m7"));
    assert_eq!(chip.flash.base, 0x0800_0000);
    assert_eq!(chip.ram.base, 0x2000_0000);
    assert_eq!(chip.ram.size, 64 * 1024, "DTCM primary RAM must be 64KB");

    assert!(
        bus.find_peripheral_index_by_name("rcc").is_some(),
        "bus must expose rcc (stm32f4 @ 0x40023800)"
    );
    assert!(
        bus.find_peripheral_index_by_name("gpioi").is_some(),
        "bus must expose gpioi (Discovery LED PI1)"
    );
    assert!(
        bus.find_peripheral_index_by_name("usart1").is_some(),
        "bus must expose usart1 (ST-LINK VCP)"
    );
    assert!(
        bus.find_peripheral_index_by_name("systick").is_some(),
        "bus must expose systick"
    );
    for stub in ["ltdc", "eth", "dma2d", "usb", "quadspi"] {
        assert!(
            bus.find_peripheral_index_by_name(stub).is_some(),
            "bus must expose {stub} stub"
        );
    }
}
