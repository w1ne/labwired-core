// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every Cortex-M core has a SysTick timer at 0xE000_E010. Vendor SVDs start
//! above the PPB, so descriptors declare it by hand, and the Nordic family
//! shipped without it: Arduino and Zephyr firmware both program SysTick at
//! boot, so the first `SYST_CSR` write faulted and a hosted verify came back
//! `unsupported`. This pins the block on every Cortex-M descriptor.

use labwired_config::ChipDescriptor;
use std::path::PathBuf;

fn chips_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips")
}

#[test]
fn every_cortex_m_descriptor_declares_systick() {
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(chips_dir()).expect("read configs/chips") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_none_or(|ext| ext != "yaml") {
            continue;
        }
        let chip = path.file_stem().unwrap().to_string_lossy().to_string();
        // Minimal CI fixtures pin trace fingerprints; they model only what
        // their firmware touches.
        if chip.starts_with("ci-fixture") {
            continue;
        }
        let desc = ChipDescriptor::from_file(&path).unwrap_or_else(|e| panic!("load {chip}: {e}"));
        let is_cortex_m = desc
            .core
            .as_deref()
            .is_some_and(|core| core.starts_with("cortex-m"));
        if !is_cortex_m {
            continue;
        }
        let systick = desc.peripherals.iter().find(|p| p.r#type == "systick");
        match systick {
            Some(p) if p.base_address == 0xE000_E010 => {}
            Some(p) => missing.push(format!("{chip}: systick at {:#x}", p.base_address)),
            None => missing.push(format!("{chip}: no systick")),
        }
    }
    assert!(missing.is_empty(), "{missing:#?}");
}
