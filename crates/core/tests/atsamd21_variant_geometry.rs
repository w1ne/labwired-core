// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Exact SAM D21 orderable-variant geometry, checked against the datasheet.
//!
//! The public `atsamd21g18a` descriptor is the engine: PM/SYSCTRL/GCLK/WDT/
//! NVMCTRL/PORT/SERCOM register models, with their documented limits. The
//! other A-variant orderable parts differ in *geometry* — Flash, SRAM, package
//! pin count, GPIO count and (on the E) the number of SERCOM instances. This
//! gate keeps those eleven descriptors exact instead of letting one of them
//! silently inherit G18A's 256 KB/32 KB memory or six SERCOMs.
//!
//! Source: Microchip SAM D21/DA1 Family Data Sheet DS40001882H, sec. 2
//! (Configuration Summary), sec. 3 (Ordering Information) and sec. 5 (Pinout).
//! Peripheral addresses are the family-wide `ATSAMD21G18A.svd` map already
//! checked by `svd_conformance`.
//!
//! This says nothing about peripheral fidelity beyond `atsamd21g18a.yaml`.
//! It proves the descriptors load, expose the right geometry and instance set,
//! and still build the same public SAM D21 estate.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest, BUILTIN_CHIP_NAMES};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use std::collections::BTreeSet;

/// One row of DS40001882H Table 2-1 / Table 3-1, for the A suffix only.
struct Variant {
    name: &'static str,
    flash: u64,
    ram: u64,
    package: &'static str,
    pin_count: u64,
    gpio_pin_count: u64,
    sercoms: usize,
}

const VARIANTS: &[Variant] = &[
    Variant {
        name: "atsamd21e15a",
        flash: 32 * 1024,
        ram: 4 * 1024,
        package: "TQFP32/QFN32",
        pin_count: 32,
        gpio_pin_count: 26,
        sercoms: 4,
    },
    Variant {
        name: "atsamd21e16a",
        flash: 64 * 1024,
        ram: 8 * 1024,
        package: "TQFP32/QFN32",
        pin_count: 32,
        gpio_pin_count: 26,
        sercoms: 4,
    },
    Variant {
        name: "atsamd21e17a",
        flash: 128 * 1024,
        ram: 16 * 1024,
        package: "TQFP32/QFN32",
        pin_count: 32,
        gpio_pin_count: 26,
        sercoms: 4,
    },
    Variant {
        name: "atsamd21e18a",
        flash: 256 * 1024,
        ram: 32 * 1024,
        package: "TQFP32/QFN32",
        pin_count: 32,
        gpio_pin_count: 26,
        sercoms: 4,
    },
    Variant {
        name: "atsamd21g15a",
        flash: 32 * 1024,
        ram: 4 * 1024,
        package: "TQFP48/QFN48",
        pin_count: 48,
        gpio_pin_count: 38,
        sercoms: 6,
    },
    Variant {
        name: "atsamd21g16a",
        flash: 64 * 1024,
        ram: 8 * 1024,
        package: "TQFP48/QFN48",
        pin_count: 48,
        gpio_pin_count: 38,
        sercoms: 6,
    },
    Variant {
        name: "atsamd21g17a",
        flash: 128 * 1024,
        ram: 16 * 1024,
        package: "TQFP48/QFN48",
        pin_count: 48,
        gpio_pin_count: 38,
        sercoms: 6,
    },
    Variant {
        name: "atsamd21j15a",
        flash: 32 * 1024,
        ram: 4 * 1024,
        package: "TQFP64/QFN64",
        pin_count: 64,
        gpio_pin_count: 52,
        sercoms: 6,
    },
    Variant {
        name: "atsamd21j16a",
        flash: 64 * 1024,
        ram: 8 * 1024,
        package: "TQFP64/QFN64",
        pin_count: 64,
        gpio_pin_count: 52,
        sercoms: 6,
    },
    Variant {
        name: "atsamd21j17a",
        flash: 128 * 1024,
        ram: 16 * 1024,
        package: "TQFP64/QFN64",
        pin_count: 64,
        gpio_pin_count: 52,
        sercoms: 6,
    },
    Variant {
        name: "atsamd21j18a",
        flash: 256 * 1024,
        ram: 32 * 1024,
        package: "TQFP64/QFN64",
        pin_count: 64,
        gpio_pin_count: 52,
        sercoms: 6,
    },
];

const CORE_PERIPHERALS: &[&str] = &[
    "pm", "sysctrl", "gclk", "wdt", "nvmctrl", "gpio_a", "gpio_b", "nvic", "systick",
];

const ORDERABLE_SUFFIXES: &[&str] = &["AU", "AUT", "AF", "AFT", "MU", "MUT", "MF", "MFT"];

fn chip_path(name: &str) -> std::path::PathBuf {
    root(&format!("configs/chips/{name}.yaml"))
}

fn load(name: &str) -> ChipDescriptor {
    let path = chip_path(name);
    ChipDescriptor::from_file(&path).unwrap_or_else(|e| panic!("load {path:?}: {e}"))
}

fn raw_yaml(name: &str) -> serde_yaml::Value {
    let path = chip_path(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

fn read_u32(bus: &SystemBus, addr: u64) -> u32 {
    (0..4)
        .map(|i| (bus.read_u8(addr + i).unwrap_or(0) as u32) << (i * 8))
        .fold(0, |a, b| a | b)
}

#[test]
fn the_eleven_inventory_ids_are_public_builtins() {
    let inventory: BTreeSet<&str> = VARIANTS.iter().map(|v| v.name).collect();
    assert_eq!(inventory.len(), 11, "inventory ids must be unique");
    for name in &inventory {
        assert!(
            BUILTIN_CHIP_NAMES.contains(name),
            "{name} is not in the generated built-in registry; configs/chips/ is the registry"
        );
    }
}

#[test]
fn variant_geometry_matches_the_datasheet() {
    for v in VARIANTS {
        let chip = load(v.name);
        assert_eq!(chip.name, v.name);
        assert_eq!(chip.core.as_deref(), Some("cortex-m0+"));
        assert_eq!(chip.cpu_hz, 48_000_000, "{}", v.name);
        assert_eq!(chip.flash.base, 0, "{}", v.name);
        assert_eq!(chip.flash.size, v.flash, "{} flash", v.name);
        assert_eq!(chip.ram.base, 0x2000_0000, "{}", v.name);
        assert_eq!(chip.ram.size, v.ram, "{} SRAM", v.name);

        let doc = raw_yaml(v.name);
        assert_eq!(doc["package"].as_str(), Some(v.package), "{}", v.name);
        assert_eq!(doc["pin_count"].as_u64(), Some(v.pin_count), "{}", v.name);
        assert_eq!(
            doc["gpio_pin_count"].as_u64(),
            Some(v.gpio_pin_count),
            "{}",
            v.name
        );
        assert_eq!(doc["variant"].as_str(), Some("A"), "{}", v.name);
        let suffixes: Vec<&str> = doc["orderable_suffixes"]
            .as_sequence()
            .unwrap_or_else(|| panic!("{} has no orderable_suffixes", v.name))
            .iter()
            .map(|s| s.as_str().expect("suffix string"))
            .collect();
        assert_eq!(suffixes, ORDERABLE_SUFFIXES, "{}", v.name);
    }
}

#[test]
fn variant_peripheral_sets_match_the_silicon_instance_counts() {
    let all_sercoms = [
        "sercom0", "sercom1", "sercom2", "sercom3", "sercom4", "sercom5",
    ];
    for v in VARIANTS {
        let chip = load(v.name);
        let ids: BTreeSet<&str> = chip.peripherals.iter().map(|p| p.id.as_str()).collect();

        let mut want: BTreeSet<&str> = CORE_PERIPHERALS.iter().copied().collect();
        want.extend(all_sercoms[..v.sercoms].iter().copied());
        assert_eq!(ids, want, "{} peripheral instance set", v.name);

        if v.sercoms == 4 {
            assert!(
                !ids.contains("sercom4"),
                "{} must not declare SERCOM4",
                v.name
            );
            assert!(
                !ids.contains("sercom5"),
                "{} must not declare SERCOM5",
                v.name
            );
        }
    }
}

#[test]
fn variant_pin_labels_use_the_family_port_groups() {
    for v in VARIANTS {
        let chip = load(v.name);
        assert_eq!(chip.pins.len(), 64, "{} pin-label map", v.name);
        let pa17 = chip.pins.get("PA17").expect("PA17");
        assert_eq!((pa17.gpio.as_str(), pa17.bit), ("gpio_a", 17));
        let pb23 = chip.pins.get("PB23").expect("PB23");
        assert_eq!((pb23.gpio.as_str(), pb23.bit), ("gpio_b", 23));
    }
}

#[test]
fn variants_reuse_the_bootable_samd21_estate() {
    for v in VARIANTS {
        let path = chip_path(v.name);
        let chip = ChipDescriptor::from_file(&path).expect("chip yaml");
        let manifest = SystemManifest {
            chip: path.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let mut bus = SystemBus::from_config(&chip, &manifest)
            .unwrap_or_else(|e| panic!("build {} bus: {e}", v.name));
        let _ = configure_cortex_m(&mut bus);

        // SYSCTRL.PCLKSR ready flags are the same hand-set boot gate the G18A
        // estate provides. Reading them here proves the declarative files were
        // resolved for this variant too, not just that the YAML parsed.
        assert_eq!(
            read_u32(&bus, 0x4000_080C),
            0x0001_8ADF,
            "{} did not resolve the shared SYSCTRL engine",
            v.name
        );

        // PORT GROUP A/B are distinct windows, as in the G18A estate.
        bus.write_u8(0x4100_4400, 0xFF).expect("write PORT A DIR");
        bus.write_u8(0x4100_4480, 0x00).expect("write PORT B DIR");
        assert_eq!(bus.read_u8(0x4100_4400).unwrap(), 0xFF, "{} PORT A", v.name);
        assert_eq!(bus.read_u8(0x4100_4480).unwrap(), 0x00, "{} PORT B", v.name);
    }
}
