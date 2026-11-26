// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The nRF54L WDT30/WDT31 descriptors must build the LFCLK watchdog model and
//! route its STOPPED event interrupt on the SVD-defined NVIC lines.

use crate::common::root;
use labwired_config::{ChipDescriptor, PeripheralConfig, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

const TYPE: &str = "nrf54l_wdt";
const TASKS_START: u64 = 0x000;
const TASKS_STOP: u64 = 0x004;
const EVENTS_STOPPED: u64 = 0x104;
const INTENSET: u64 = 0x304;
const RUNSTATUS: u64 = 0x400;
const CRV: u64 = 0x504;
const CONFIG: u64 = 0x50C;
const TSEN: u64 = 0x520;
const TSEN_STOP_KEY: u32 = 0x6E52_4635;

fn build_bus(chip_path: &std::path::Path, chip: &ChipDescriptor) -> SystemBus {
    let manifest = SystemManifest {
        chip: chip_path.to_string_lossy().into_owned(),
        ..Default::default()
    };
    SystemBus::from_config(chip, &manifest)
        .unwrap_or_else(|e| panic!("assemble {}: {e}", chip_path.display()))
}

fn wdt_instances(chip: &ChipDescriptor) -> Vec<&PeripheralConfig> {
    chip.peripherals
        .iter()
        .filter(|p| p.r#type == TYPE)
        .collect()
}

/// Secure WDT instances from the vendor SVD: `(descriptor id, base, irq)`.
fn svd_secure_wdts(svd_path: &std::path::Path) -> Vec<(String, u64, u32)> {
    let text = std::fs::read_to_string(svd_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", svd_path.display()));
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    let mut base: Option<u64> = None;
    let mut irq: Option<u32> = None;
    let mut in_interrupt = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("<peripheral") {
            name = None;
            base = None;
            irq = None;
            in_interrupt = false;
        } else if let Some(v) = t
            .strip_prefix("<name>")
            .and_then(|s| s.strip_suffix("</name>"))
        {
            if name.is_none() {
                name = Some(v.trim().to_string());
            }
        } else if let Some(v) = t
            .strip_prefix("<baseAddress>")
            .and_then(|s| s.strip_suffix("</baseAddress>"))
        {
            let v = v.trim();
            let hex = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X"));
            base = u64::from_str_radix(hex.unwrap_or(v), 16).ok();
        } else if t == "<interrupt>" {
            in_interrupt = true;
        } else if in_interrupt {
            if let Some(v) = t
                .strip_prefix("<value>")
                .and_then(|s| s.strip_suffix("</value>"))
            {
                irq = v.trim().parse().ok();
                in_interrupt = false;
            }
        } else if t == "</peripheral>" {
            if let (Some(n), Some(b), Some(i)) = (name.take(), base, irq) {
                if n.starts_with("GLOBAL_WDT") && n.ends_with("_S") {
                    let id = n
                        .trim_start_matches("GLOBAL_")
                        .trim_end_matches("_S")
                        .to_ascii_lowercase();
                    out.push((id, b, i));
                }
            }
            in_interrupt = false;
        }
    }
    out.sort();
    out
}

#[test]
fn nrf54l_wdt_descriptors_run_the_stop_sequence_and_pend_the_svd_irq() {
    for (part, svd_file) in [
        ("nrf54l15", "nrf54l15.svd"),
        ("nrf54lm20a", "nrf54lm20a.svd"),
    ] {
        let chip_path = root(&format!("configs/chips/{part}.yaml"));
        let chip =
            ChipDescriptor::from_file(&chip_path).unwrap_or_else(|e| panic!("load {part}: {e}"));
        let svd_path = root(&format!("tests/fixtures/real_world/{svd_file}"));
        let expected = svd_secure_wdts(&svd_path);
        assert_eq!(expected.len(), 2, "{svd_file}: secure WDT estate");
        let instances = wdt_instances(&chip);
        assert_eq!(
            instances.len(),
            2,
            "{part}: both WDT instances are modelled"
        );

        for (id, expected_base, expected_irq) in expected {
            let wdt = instances
                .iter()
                .find(|wdt| wdt.id == id)
                .unwrap_or_else(|| panic!("{part}: missing SVD WDT {id}"));
            assert_eq!(wdt.r#type, TYPE, "{part}/{id} model type");
            assert_eq!(wdt.base_address, expected_base, "{part}/{id} SVD base");
            assert_eq!(wdt.irq, Some(expected_irq), "{part}/{id} SVD IRQ");

            let mut bus = build_bus(&chip_path, &chip);
            let (_cpu, _nvic_state) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
            let base = wdt.base_address;
            bus.write_u32(base + CRV, 0).unwrap();
            bus.write_u32(base + CONFIG, 0x41).unwrap(); // SLEEP=Run, STOPEN(bit 6)=Enable
            bus.write_u32(base + INTENSET, 1 << 1).unwrap(); // STOPPED
            bus.write_u32(base + TSEN, TSEN_STOP_KEY).unwrap();
            bus.write_u32(base + TASKS_START, 1).unwrap();
            assert_eq!(bus.read_u32(base + RUNSTATUS).unwrap(), 1);

            bus.write_u32(base + TASKS_STOP, 1).unwrap();
            assert_eq!(bus.read_u32(base + RUNSTATUS).unwrap(), 0);
            assert_eq!(bus.read_u32(base + EVENTS_STOPPED).unwrap(), 1);

            let iser = 0xE000_E100 + u64::from(expected_irq / 32) * 4;
            let ispr = 0xE000_E200 + u64::from(expected_irq / 32) * 4;
            let bit = 1 << (expected_irq % 32);
            bus.write_u32(iser, bit).unwrap();
            assert_ne!(
                bus.read_u32(ispr).unwrap() & bit,
                0,
                "{part}/{} IRQ",
                wdt.id
            );

            bus.write_u32(base + EVENTS_STOPPED, 0).unwrap();
            assert_eq!(bus.read_u32(base + EVENTS_STOPPED).unwrap(), 0);
            assert_eq!(
                bus.read_u32(ispr).unwrap() & bit,
                0,
                "cleared event drops IRQ"
            );
            assert_ne!(bus.read_u32(iser).unwrap() & bit, 0);
        }
    }
}
