// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Exact STM32F1/F4 orderable-variant geometry, checked against ST's tables.
//!
//! The public `stm32f103`, `stm32f401` and `stm32f407` descriptors are the
//! engines. The extra inventory parts differ in orderable *geometry*: Flash,
//! SRAM, package pin count, GPIO/ADC channel counts and ordering suffixes.
//! This gate keeps the new descriptors exact instead of letting one silently
//! inherit its base part's memory or package data.
//!
//! Sources:
//!   * ST DS5792 Rev 7 / DocID15060 — STM32F103x4/x6 low-density, Table 2,
//!     Table 5 and ordering Table 58.
//!   * ST DS9716 Rev 11 — STM32F401xB/xC, Table 2 and section 8.
//!   * ST DS10086 Rev 5 — STM32F401xD/xE, Table 2 and section 8.
//!   * ST DS8626 Rev 12 — STM32F405xx/STM32F407xx, Table 2 and section 7.
//!
//! This says nothing about peripheral fidelity beyond the reference engines.
//! In particular the low-density F103 configs reuse the medium-density
//! STM32F103C8 peripheral map (including SPI2/I2C2/USART3/TIM4), because no
//! separate low-density register model exists yet. That boundary is asserted
//! here by requiring every variant's peripheral-id set to equal its public
//! reference descriptor's set, and it is documented in each F103 YAML header.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest, BUILTIN_CHIP_NAMES};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    F103,
    F401,
    F407,
}

struct Variant {
    name: &'static str,
    family: Family,
    flash: u64,
    ram: u64,
    package: &'static str,
    pin_count: u64,
    gpio_pin_count: u64,
    adc_channels: u64,
    /// Regular ADC pads the reference map actually transcribes.
    modelled_analog: usize,
    variant: &'static str,
    suffixes: &'static [&'static str],
}

const T4: &[&str] = &["U6", "U6TR", "U6A", "U6ATR", "U7", "U7TR", "U7A", "U7ATR"];
const C4: &[&str] = &[
    "T6", "T6TR", "T6A", "T6ATR", "T7", "T7TR", "T7A", "T7ATR", "U6", "U6TR", "U6A", "U6ATR", "U7",
    "U7TR", "U7A", "U7ATR",
];
const R4_LQFP: &[&str] = &["T6", "T6TR", "T6A", "T6ATR", "T7", "T7TR", "T7A", "T7ATR"];
const R4_TFBGA: &[&str] = &["H6", "H6TR", "H6A", "H6ATR", "H7", "H7TR", "H7A", "H7ATR"];
const C48: &[&str] = &["U6", "U6TR", "U7", "U7TR", "Y6", "Y6TR", "Y7", "Y7TR"];
const R64: &[&str] = &["T6", "T6TR", "T7", "T7TR"];
const V100: &[&str] = &["T6", "T6TR", "T7", "T7TR", "H6", "H6TR", "H7", "H7TR"];
const V100_1M: &[&str] = &["T6", "T6TR", "T7", "T7TR"];
const I176: &[&str] = &["T6", "T6TR", "T7", "T7TR", "H6", "H6TR", "H7", "H7TR"];
const C_PACKAGE_PINS: &[(&str, u64)] = &[("UFQFPN48", 48), ("WLCSP49", 49)];

const VARIANTS: &[Variant] = &[
    // ── STM32F103 low-density x4/x6 (DS5792 Table 2/5/58) ───────────────────
    Variant {
        name: "stm32f103t4",
        family: Family::F103,
        flash: 16 * 1024,
        ram: 6 * 1024,
        package: "VFQFPN36",
        pin_count: 36,
        gpio_pin_count: 26,
        adc_channels: 10,
        modelled_analog: 10,
        variant: "x4",
        suffixes: T4,
    },
    Variant {
        name: "stm32f103t6",
        family: Family::F103,
        flash: 32 * 1024,
        ram: 10 * 1024,
        package: "VFQFPN36",
        pin_count: 36,
        gpio_pin_count: 26,
        adc_channels: 10,
        modelled_analog: 10,
        variant: "x6",
        suffixes: T4,
    },
    Variant {
        name: "stm32f103c4",
        family: Family::F103,
        flash: 16 * 1024,
        ram: 6 * 1024,
        package: "LQFP48/UFQFPN48",
        pin_count: 48,
        gpio_pin_count: 37,
        adc_channels: 10,
        modelled_analog: 10,
        variant: "x4",
        suffixes: C4,
    },
    Variant {
        name: "stm32f103c6",
        family: Family::F103,
        flash: 32 * 1024,
        ram: 10 * 1024,
        package: "LQFP48/UFQFPN48",
        pin_count: 48,
        gpio_pin_count: 37,
        adc_channels: 10,
        modelled_analog: 10,
        variant: "x6",
        suffixes: C4,
    },
    Variant {
        name: "stm32f103r4",
        family: Family::F103,
        flash: 16 * 1024,
        ram: 6 * 1024,
        package: "LQFP64",
        pin_count: 64,
        gpio_pin_count: 51,
        adc_channels: 16,
        modelled_analog: 16,
        variant: "x4",
        suffixes: R4_LQFP,
    },
    Variant {
        name: "stm32f103r6",
        family: Family::F103,
        flash: 32 * 1024,
        ram: 10 * 1024,
        package: "LQFP64",
        pin_count: 64,
        gpio_pin_count: 51,
        adc_channels: 16,
        modelled_analog: 16,
        variant: "x6",
        suffixes: R4_LQFP,
    },
    Variant {
        name: "stm32f103r4h",
        family: Family::F103,
        flash: 16 * 1024,
        ram: 6 * 1024,
        package: "TFBGA64",
        pin_count: 64,
        gpio_pin_count: 51,
        adc_channels: 15,
        modelled_analog: 15,
        variant: "x4",
        suffixes: R4_TFBGA,
    },
    Variant {
        name: "stm32f103r6h",
        family: Family::F103,
        flash: 32 * 1024,
        ram: 10 * 1024,
        package: "TFBGA64",
        pin_count: 64,
        gpio_pin_count: 51,
        adc_channels: 15,
        modelled_analog: 15,
        variant: "x6",
        suffixes: R4_TFBGA,
    },
    // ── STM32F401xB/xC (DS9716 Table 2 & ordering scheme) ───────────────────
    Variant {
        name: "stm32f401cb",
        family: Family::F401,
        flash: 128 * 1024,
        ram: 64 * 1024,
        package: "UFQFPN48/WLCSP49",
        pin_count: 48,
        gpio_pin_count: 36,
        adc_channels: 10,
        modelled_analog: 10,
        variant: "xB",
        suffixes: C48,
    },
    Variant {
        name: "stm32f401cc",
        family: Family::F401,
        flash: 256 * 1024,
        ram: 64 * 1024,
        package: "UFQFPN48/WLCSP49",
        pin_count: 48,
        gpio_pin_count: 36,
        adc_channels: 10,
        modelled_analog: 10,
        variant: "xC",
        suffixes: C48,
    },
    Variant {
        name: "stm32f401rb",
        family: Family::F401,
        flash: 128 * 1024,
        ram: 64 * 1024,
        package: "LQFP64",
        pin_count: 64,
        gpio_pin_count: 50,
        adc_channels: 16,
        modelled_analog: 16,
        variant: "xB",
        suffixes: R64,
    },
    Variant {
        name: "stm32f401rc",
        family: Family::F401,
        flash: 256 * 1024,
        ram: 64 * 1024,
        package: "LQFP64",
        pin_count: 64,
        gpio_pin_count: 50,
        adc_channels: 16,
        modelled_analog: 16,
        variant: "xC",
        suffixes: R64,
    },
    Variant {
        name: "stm32f401vb",
        family: Family::F401,
        flash: 128 * 1024,
        ram: 64 * 1024,
        package: "LQFP100/UFBGA100",
        pin_count: 100,
        gpio_pin_count: 81,
        adc_channels: 16,
        modelled_analog: 16,
        variant: "xB",
        suffixes: V100,
    },
    Variant {
        name: "stm32f401vc",
        family: Family::F401,
        flash: 256 * 1024,
        ram: 64 * 1024,
        package: "LQFP100/UFBGA100",
        pin_count: 100,
        gpio_pin_count: 81,
        adc_channels: 16,
        modelled_analog: 16,
        variant: "xC",
        suffixes: V100,
    },
    // ── STM32F401xD/xE (DS10086 Table 2 & ordering scheme) ─────────────────
    Variant {
        name: "stm32f401cd",
        family: Family::F401,
        flash: 384 * 1024,
        ram: 96 * 1024,
        package: "UFQFPN48/WLCSP49",
        pin_count: 48,
        gpio_pin_count: 36,
        adc_channels: 10,
        modelled_analog: 10,
        variant: "xD",
        suffixes: C48,
    },
    Variant {
        name: "stm32f401ce",
        family: Family::F401,
        flash: 512 * 1024,
        ram: 96 * 1024,
        package: "UFQFPN48/WLCSP49",
        pin_count: 48,
        gpio_pin_count: 36,
        adc_channels: 10,
        modelled_analog: 10,
        variant: "xE",
        suffixes: C48,
    },
    Variant {
        name: "stm32f401rd",
        family: Family::F401,
        flash: 384 * 1024,
        ram: 96 * 1024,
        package: "LQFP64",
        pin_count: 64,
        gpio_pin_count: 50,
        adc_channels: 16,
        modelled_analog: 16,
        variant: "xD",
        suffixes: R64,
    },
    Variant {
        name: "stm32f401re",
        family: Family::F401,
        flash: 512 * 1024,
        ram: 96 * 1024,
        package: "LQFP64",
        pin_count: 64,
        gpio_pin_count: 50,
        adc_channels: 16,
        modelled_analog: 16,
        variant: "xE",
        suffixes: R64,
    },
    Variant {
        name: "stm32f401vd",
        family: Family::F401,
        flash: 384 * 1024,
        ram: 96 * 1024,
        package: "LQFP100/UFBGA100",
        pin_count: 100,
        gpio_pin_count: 81,
        adc_channels: 16,
        modelled_analog: 16,
        variant: "xD",
        suffixes: V100,
    },
    Variant {
        name: "stm32f401ve",
        family: Family::F401,
        flash: 512 * 1024,
        ram: 96 * 1024,
        package: "LQFP100/UFBGA100",
        pin_count: 100,
        gpio_pin_count: 81,
        adc_channels: 16,
        modelled_analog: 16,
        variant: "xE",
        suffixes: V100,
    },
    // ── STM32F407 xE/xG (DS8626 Table 2 & part numbering) ──────────────────
    Variant {
        name: "stm32f407ve",
        family: Family::F407,
        flash: 512 * 1024,
        ram: 128 * 1024,
        package: "LQFP100",
        pin_count: 100,
        gpio_pin_count: 82,
        adc_channels: 16,
        modelled_analog: 16,
        variant: "xE",
        suffixes: V100_1M,
    },
    Variant {
        name: "stm32f407vg",
        family: Family::F407,
        flash: 1024 * 1024,
        ram: 128 * 1024,
        package: "LQFP100",
        pin_count: 100,
        gpio_pin_count: 82,
        adc_channels: 16,
        modelled_analog: 16,
        variant: "xG",
        suffixes: V100_1M,
    },
    Variant {
        name: "stm32f407ze",
        family: Family::F407,
        flash: 512 * 1024,
        ram: 128 * 1024,
        package: "LQFP144",
        pin_count: 144,
        gpio_pin_count: 114,
        adc_channels: 24,
        modelled_analog: 16,
        variant: "xE",
        suffixes: V100_1M,
    },
    Variant {
        name: "stm32f407zg",
        family: Family::F407,
        flash: 1024 * 1024,
        ram: 128 * 1024,
        package: "LQFP144",
        pin_count: 144,
        gpio_pin_count: 114,
        adc_channels: 24,
        modelled_analog: 16,
        variant: "xG",
        suffixes: V100_1M,
    },
    Variant {
        name: "stm32f407ie",
        family: Family::F407,
        flash: 512 * 1024,
        ram: 128 * 1024,
        package: "LQFP176/UFBGA176",
        pin_count: 176,
        gpio_pin_count: 140,
        adc_channels: 24,
        modelled_analog: 16,
        variant: "xE",
        suffixes: I176,
    },
    Variant {
        name: "stm32f407ig",
        family: Family::F407,
        flash: 1024 * 1024,
        ram: 128 * 1024,
        package: "LQFP176/UFBGA176",
        pin_count: 176,
        gpio_pin_count: 140,
        adc_channels: 24,
        modelled_analog: 16,
        variant: "xG",
        suffixes: I176,
    },
];

fn reference(family: Family) -> &'static str {
    match family {
        Family::F103 => "stm32f103",
        Family::F401 => "stm32f401",
        Family::F407 => "stm32f407",
    }
}

fn core_name(family: Family) -> &'static str {
    match family {
        Family::F103 => "cortex-m3",
        Family::F401 | Family::F407 => "cortex-m4",
    }
}

fn cpu_hz(family: Family) -> u64 {
    match family {
        Family::F103 => 72_000_000,
        Family::F401 => 84_000_000,
        Family::F407 => 168_000_000,
    }
}

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
fn the_inventory_ids_are_public_builtins() {
    let inventory: BTreeSet<&str> = VARIANTS.iter().map(|v| v.name).collect();
    assert_eq!(
        inventory.len(),
        VARIANTS.len(),
        "inventory ids must be unique"
    );
    assert_eq!(inventory.len(), 26, "unexpected inventory size");
    for name in &inventory {
        assert!(
            BUILTIN_CHIP_NAMES.contains(name),
            "{name} is not in the generated built-in registry; configs/chips/ is the registry"
        );
        let resolved = ChipDescriptor::resolve(name, Path::new("/nonexistent"))
            .unwrap_or_else(|e| panic!("resolve bare built-in {name}: {e}"));
        assert_eq!(resolved.name, *name, "{name} resolved to another part");
    }
}

#[test]
fn variant_geometry_matches_the_st_datasheets() {
    for v in VARIANTS {
        let chip = load(v.name);
        assert_eq!(chip.name, v.name);
        assert_eq!(
            chip.core.as_deref(),
            Some(core_name(v.family)),
            "{}",
            v.name
        );
        assert_eq!(chip.cpu_hz, cpu_hz(v.family), "{}", v.name);
        assert_eq!(chip.flash.base, 0x0800_0000, "{}", v.name);
        assert_eq!(chip.flash.size, v.flash, "{} flash", v.name);
        assert_eq!(chip.ram.base, 0x2000_0000, "{}", v.name);
        assert_eq!(chip.ram.size, v.ram, "{} SRAM", v.name);

        let doc = raw_yaml(v.name);
        assert_eq!(doc["package"].as_str(), Some(v.package), "{}", v.name);
        if doc.get("packages").is_some() {
            // The C-package F401s group UFQFPN48 and WLCSP49. Their physical
            // pin/ball counts differ (48 vs 49), so the descriptor must carry
            // the exact per-package counts instead of one ambiguous pin_count.
            let packages = doc["packages"]
                .as_mapping()
                .unwrap_or_else(|| panic!("{} packages is not a map", v.name));
            let got: BTreeMap<&str, u64> = packages
                .iter()
                .map(|(name, body)| {
                    (
                        name.as_str().expect("package name"),
                        body["pin_count"].as_u64().expect("package pin_count"),
                    )
                })
                .collect();
            let want: BTreeMap<&str, u64> = C_PACKAGE_PINS.iter().copied().collect();
            assert_eq!(got, want, "{} package pin counts", v.name);
            assert!(
                doc.get("pin_count").is_none(),
                "{} keeps a package-ambiguous top-level pin_count",
                v.name
            );
        } else {
            assert_eq!(doc["pin_count"].as_u64(), Some(v.pin_count), "{}", v.name);
        }
        assert_eq!(
            doc["gpio_pin_count"].as_u64(),
            Some(v.gpio_pin_count),
            "{}",
            v.name
        );
        assert_eq!(
            doc["adc_channels"].as_u64(),
            Some(v.adc_channels),
            "{}",
            v.name
        );
        assert_eq!(doc["variant"].as_str(), Some(v.variant), "{}", v.name);
        let suffixes: Vec<&str> = doc["orderable_suffixes"]
            .as_sequence()
            .unwrap_or_else(|| panic!("{} has no orderable_suffixes", v.name))
            .iter()
            .map(|s| s.as_str().expect("suffix string"))
            .collect();
        assert_eq!(suffixes, v.suffixes, "{}", v.name);
    }
}

#[test]
fn variants_reuse_their_public_reference_peripheral_map() {
    // This is the explicit fidelity boundary. A variant is a geometry slice,
    // not a new peripheral model: its instance set must stay byte-identical to
    // the public base engine. For the low-density F103s this records the known
    // limitation that the medium-density map (SPI2/I2C2/USART3/TIM4) is reused
    // even though x4/x6 silicon has fewer instances.
    for v in VARIANTS {
        let reference = load(reference(v.family));
        let variant = load(v.name);
        let want: BTreeSet<&str> = reference
            .peripherals
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        let got: BTreeSet<&str> = variant.peripherals.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(got, want, "{} no longer reuses {}", v.name, reference.name);
    }
}

#[test]
fn variant_analog_pads_follow_the_package_featured_map() {
    let f1_f4 = |with_pc: bool| {
        let mut pads: Vec<(String, u8)> = (0..8).map(|i| (format!("PA{i}"), i)).collect();
        pads.push(("PB0".to_string(), 8));
        pads.push(("PB1".to_string(), 9));
        if with_pc {
            pads.extend((0..6).map(|i| (format!("PC{i}"), 10 + i)));
        }
        pads
    };
    for v in VARIANTS {
        let chip = load(v.name);
        let expected = if v.adc_channels == 15 {
            // TFBGA64 R4/R6: PC3 is VREF+, so IN13 has no pad. Keep the
            // family channel numbers for the five PC pads that are bonded.
            let mut pads = f1_f4(false);
            pads.extend([
                ("PC0".to_string(), 10),
                ("PC1".to_string(), 11),
                ("PC2".to_string(), 12),
                ("PC4".to_string(), 14),
                ("PC5".to_string(), 15),
            ]);
            pads
        } else {
            f1_f4(v.modelled_analog == 16)
        };
        let got: Vec<(String, u8)> = chip
            .analog_pins
            .iter()
            .map(|(pad, pin)| (pad.clone(), pin.channel))
            .collect();
        assert_eq!(got, expected, "{} analog pin map", v.name);
        if v.adc_channels == 15 {
            assert!(
                !chip.analog_pins.contains_key("PC3"),
                "{} must not expose PC3/ADC12_IN13 on TFBGA64",
                v.name
            );
        }
    }
}

#[test]
fn variant_pin_labels_use_the_reference_port_groups() {
    for v in VARIANTS {
        let chip = load(v.name);
        let expected = match v.family {
            Family::F103 | Family::F401 => 3 * 16,
            Family::F407 => 5 * 16,
        };
        // TFBGA64 replaces PC3 with VREF+; the package-specific descriptor
        // omits that pad from its authoritative pin-label map.
        let expected = if v.adc_channels == 15 {
            expected - 1
        } else {
            expected
        };
        assert_eq!(chip.pins.len(), expected, "{} pin-label map", v.name);
        for (port, gpio) in [("PA", "gpioa"), ("PB", "gpiob"), ("PC", "gpioc")] {
            let pin = chip
                .pins
                .get(&format!("{port}7"))
                .unwrap_or_else(|| panic!("{} has no {port}7", v.name));
            assert_eq!((pin.gpio.as_str(), pin.bit), (gpio, 7), "{}", v.name);
        }
    }
}

#[test]
fn variants_build_the_shared_public_estate() {
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

        // Reading the shared RCC block proves the declarative peripheral files
        // were resolved for this variant too, not merely that the YAML parsed.
        let rcc = chip
            .peripherals
            .iter()
            .find(|p| p.id == "rcc")
            .unwrap_or_else(|| panic!("{} has no rcc", v.name));
        let value = read_u32(&bus, rcc.base_address);
        assert!(
            bus.read_u32(rcc.base_address).is_ok(),
            "{} RCC window at {:#x} does not answer (read {value:#x})",
            v.name,
            rcc.base_address
        );
    }
}
