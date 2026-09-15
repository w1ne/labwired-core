// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `io_voltage_v` and `gpio_input_thresholds` — the pad electrical data a chip
//! descriptor may declare.
//!
//! Co-simulation turns a model's node voltage into the level the firmware reads
//! through these numbers, so they are datasheet transcriptions, cited in each
//! descriptor. What this file pins is the transcriptions themselves and the
//! shape every descriptor must keep: the two keys come together, because a
//! ratio without the rail it is a ratio of has no voltage.

use labwired_config::{ChipDescriptor, GpioInputThresholds};
use std::path::PathBuf;

fn chips_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips")
}

fn load(chip: &str) -> ChipDescriptor {
    let path = chips_dir().join(format!("{chip}.yaml"));
    ChipDescriptor::from_file(&path).unwrap_or_else(|e| panic!("load {chip}: {e}"))
}

#[test]
fn every_descriptor_declares_both_keys_or_neither() {
    let mut problems = Vec::new();
    for entry in std::fs::read_dir(chips_dir()).expect("read configs/chips") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_none_or(|ext| ext != "yaml") {
            continue;
        }
        let chip = path.file_stem().unwrap().to_string_lossy().to_string();
        let desc = load(&chip);
        if desc.io_voltage_v.is_some() != desc.gpio_input_thresholds.is_some() {
            problems.push(format!(
                "{chip}: io_voltage_v {:?} but gpio_input_thresholds {:?}",
                desc.io_voltage_v, desc.gpio_input_thresholds
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

/// The transcriptions this data was introduced with. Each descriptor's comment
/// names the datasheet table; a change here is a change to that citation.
#[test]
fn datasheet_transcriptions() {
    for (chip, volts, vil, vih) in [
        // DS40002061B Table 30-1, VCC 2.4-5.5 V.
        ("atmega328p", 5.0, 0.3, 0.6),
        // DS5319 Table 36, "All I/Os except BOOT0".
        ("stm32f103", 3.3, 0.35, 0.65),
        // DS10086 Table 54.
        ("stm32f401", 3.3, 0.3, 0.7),
        ("stm32f401cdu6", 3.3, 0.3, 0.7),
        // DS8626 Table 48.
        ("stm32f405", 3.3, 0.3, 0.7),
        ("stm32f407", 3.3, 0.3, 0.7),
        // DS10314 Table 53.
        ("stm32f411ceu6", 3.3, 0.3, 0.7),
    ] {
        let desc = load(chip);
        assert_eq!(desc.io_voltage_v, Some(volts), "{chip}");
        assert_eq!(
            desc.gpio_input_thresholds,
            Some(GpioInputThresholds { vil, vih }),
            "{chip}"
        );
    }
}

/// A chip that transcribes nothing stays empty: the absence is what makes a
/// voltage route to its pads a build error instead of a guess.
#[test]
fn an_untranscribed_chip_declares_nothing() {
    let desc = load("rp2040");
    assert_eq!(desc.io_voltage_v, None);
    assert_eq!(desc.gpio_input_thresholds, None);
}

fn parse_levels(block: &str) -> Result<ChipDescriptor, serde_yaml::Error> {
    serde_yaml::from_str(&format!(
        "name: t\narch: arm\nflash: {{ base: 0, size: 1KB }}\nram: {{ base: 0x20000000, size: 1KB }}\n\
         peripherals: []\n{block}"
    ))
}

#[test]
fn malformed_levels_are_refused_when_the_descriptor_loads() {
    for (block, expected) in [
        (
            "gpio_input_thresholds: { vil: 0.7, vih: 0.3 }\n",
            "0 < vil < vih < 1",
        ),
        (
            "gpio_input_thresholds: { vil: 0.3, vih: 3.0 }\n",
            "0 < vil < vih < 1",
        ),
        (
            "gpio_input_thresholds: { vil: 0.0, vih: 0.5 }\n",
            "0 < vil < vih < 1",
        ),
        (
            "gpio_input_thresholds: { vil: 0.3, vih: 0.7, hysteresis: 0.1 }\n",
            "hysteresis",
        ),
        ("gpio_input_thresholds: { vil: 0.3 }\n", "vih"),
        ("io_voltage_v: -3.3\n", "io_voltage_v must be a positive"),
        ("io_voltage_v: 0\n", "io_voltage_v must be a positive"),
    ] {
        let err = parse_levels(block).expect_err(block).to_string();
        assert!(err.contains(expected), "{block}: {err}");
    }

    let ok = parse_levels("io_voltage_v: 3.3\ngpio_input_thresholds: { vil: 0.3, vih: 0.7 }\n")
        .expect("well-formed levels load");
    assert_eq!(ok.io_voltage_v, Some(3.3));
    // Serialization round-trips the keys it was written with.
    let text = serde_yaml::to_string(&ok).unwrap();
    assert!(text.contains("io_voltage_v: 3.3"), "{text}");
    assert!(text.contains("vih: 0.7"), "{text}");
}
