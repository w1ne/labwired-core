// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `analog_pins:` — the pad → ADC input map a chip descriptor may declare.
//!
//! The numbers are datasheet transcription, and a wrong one is worse than a
//! missing one: co-simulation routes `board.analog.<pad>_volts` through this
//! map, so a typo sends a model's voltage to a channel the firmware never
//! converts, silently. What CAN be checked without inventing silicon facts is
//! internal consistency — every entry names an ADC the descriptor actually has,
//! a pad whose GPIO port the descriptor actually has, and no two pads share one
//! input — plus the transcriptions this map was introduced with.

use labwired_config::{AdcPinFn, ChipDescriptor};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn chips_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips")
}

fn load(chip: &str) -> ChipDescriptor {
    let path = chips_dir().join(format!("{chip}.yaml"));
    ChipDescriptor::from_file(&path).unwrap_or_else(|e| panic!("load {chip}: {e}"))
}

fn all_chips() -> Vec<String> {
    let mut chips: Vec<String> = std::fs::read_dir(chips_dir())
        .expect("read configs/chips")
        .filter_map(|e| {
            let path = e.expect("dir entry").path();
            (path.extension()? == "yaml").then(|| path.file_stem()?.to_str().map(str::to_string))?
        })
        .collect();
    chips.sort();
    chips
}

#[test]
fn every_analog_pin_names_a_real_adc_and_a_real_port() {
    let problems: Vec<_> = all_chips()
        .into_iter()
        .flat_map(|chip| analog_pin_problems(&chip, &load(&chip)))
        .collect();
    assert!(
        problems.is_empty(),
        "analog_pins problems:\n{}",
        problems.join("\n")
    );
}

fn analog_pin_problems(chip: &str, desc: &ChipDescriptor) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen: BTreeMap<(String, u8), String> = BTreeMap::new();
    for (
        pad,
        AdcPinFn {
            peripheral,
            channel,
        },
    ) in &desc.analog_pins
    {
        match desc.peripherals.iter().find(|p| &p.id == peripheral) {
            None => problems.push(format!(
                "{chip}: {pad} names ADC '{peripheral}', not declared"
            )),
            Some(p) if !matches!(p.r#type.as_str(), "adc" | "nrf52840_saadc") => {
                problems.push(format!(
                    "{chip}: {pad} names '{peripheral}', which is type '{}', not adc",
                    p.r#type
                ))
            }
            Some(_) => {}
        }
        // Validate BOTH alphaport STM32 pads and Nordic P<port>.<bit>.
        // Numeric labels are only accepted for the actual Nordic ADC IP.
        let nordic = desc
            .peripherals
            .iter()
            .any(|p| &p.id == peripheral && p.r#type == "nrf52840_saadc");
        match pad_port_and_bit(pad, nordic) {
            Some((port, bit)) => match desc
                .peripherals
                .iter()
                .find(|p| p.id == port && matches!(p.r#type.as_str(), "gpio" | "stm32f4_gpio"))
            {
                Some(p) => {
                    let count = p
                        .config
                        .get("num_pins")
                        .and_then(serde_yaml::Value::as_u64)
                        .unwrap_or(if nordic { 32 } else { 16 });
                    if u64::from(bit) >= count {
                        problems.push(format!("{chip}: {pad} is outside {port}'s {count} pins"));
                    }
                    if nordic
                        && (port != "gpio0"
                            || [2, 3, 4, 5, 28, 29, 30, 31].get(*channel as usize) != Some(&bit))
                    {
                        problems.push(format!(
                            "{chip}: {pad} is not Nordic AIN{channel}'s physical pad"
                        ));
                    }
                }
                None => problems.push(format!("{chip}: {pad} is on {port}, not a declared GPIO")),
            },
            None => problems.push(format!("{chip}: {pad} is not a P<port><bit> label")),
        }
        if let Some(other) = seen.insert((peripheral.clone(), *channel), pad.clone()) {
            problems.push(format!(
                "{chip}: {pad} and {other} both claim {peripheral} channel {channel}"
            ));
        }
    }
    problems
}

fn pad_port_and_bit(pad: &str, nordic: bool) -> Option<(String, u8)> {
    let upper = pad.to_ascii_uppercase();
    let rest = upper.strip_prefix('P')?;
    if nordic {
        let (port, bit) = rest.split_once('.')?;
        if port.len() != 1
            || bit.len() != 2
            || !port.bytes().chain(bit.bytes()).all(|c| c.is_ascii_digit())
        {
            return None;
        }
        Some((
            format!("gpio{}", port.parse::<u8>().ok()?),
            bit.parse().ok()?,
        ))
    } else {
        let port = rest.chars().next()?;
        if !port.is_ascii_alphabetic() {
            return None;
        }
        let digits = &rest[1..];
        if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some((
            format!("gpio{}", port.to_ascii_lowercase()),
            digits.parse().ok()?,
        ))
    }
}

#[test]
fn nordic_analog_pins_match_physical_ain_assignment() {
    let desc = load("nrf52833");
    assert_eq!(desc.analog_pins.len(), 8);
    for (channel, pin) in [2, 3, 4, 5, 28, 29, 30, 31].into_iter().enumerate() {
        let mapping = &desc.analog_pins[&format!("P0.{pin:02}")];
        assert_eq!(mapping.peripheral, "saadc");
        assert_eq!(usize::from(mapping.channel), channel);
    }
    assert!(analog_pin_problems("nrf52833", &desc).is_empty());
}

#[test]
fn nordic_validation_rejects_bad_models_ports_channels_and_pads() {
    let original = load("nrf52833");
    for bad_pad in ["P0.99", "P9.05", "P0.06", "P0.5", "P0.05oops", "PA5"] {
        let mut desc = original.clone();
        let mapping = desc.analog_pins.remove("P0.05").unwrap();
        desc.analog_pins.insert(bad_pad.to_string(), mapping);
        assert!(
            !analog_pin_problems("bad", &desc).is_empty(),
            "accepted {bad_pad}"
        );
    }
    let mut desc = original.clone();
    desc.analog_pins.get_mut("P0.05").unwrap().channel = 8;
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    let mut desc = original.clone();
    desc.peripherals
        .iter_mut()
        .find(|p| p.id == "saadc")
        .unwrap()
        .r#type = "uart".into();
    assert!(!analog_pin_problems("bad", &desc).is_empty());
    let mut desc = original;
    desc.peripherals
        .iter_mut()
        .find(|p| p.id == "gpio0")
        .unwrap()
        .r#type = "uart".into();
    assert!(!analog_pin_problems("bad", &desc).is_empty());
}

/// The regular-input assignment shared by the F1/F4/F7 parts in-tree:
/// PA0..PA7 = IN0..IN7, PB0/PB1 = IN8/IN9, and on packages that bond them out,
/// PC0..PC5 = IN10..IN15.
fn f1_f4_assignment(with_pc0_to_pc5: bool) -> BTreeMap<String, AdcPinFn> {
    let mut pads: Vec<(String, u8)> = (0..8).map(|i| (format!("PA{i}"), i)).collect();
    pads.push(("PB0".to_string(), 8));
    pads.push(("PB1".to_string(), 9));
    if with_pc0_to_pc5 {
        pads.extend((0..6).map(|i| (format!("PC{i}"), 10 + i)));
    }
    pads.into_iter()
        .map(|(pad, channel)| {
            (
                pad,
                AdcPinFn {
                    peripheral: "adc1".to_string(),
                    channel,
                },
            )
        })
        .collect()
}

/// The 48-pin packages (F103C8 LQFP48, F401CD/F411CE UFQFPN48) have no
/// PC0..PC5 pads, so their map stops at IN9.
#[test]
fn f1_f4_f7_descriptors_carry_the_datasheet_assignment() {
    for (chip, with_pc) in [
        ("stm32f103", false),
        ("stm32f401cdu6", false),
        ("stm32f411ceu6", false),
        ("stm32f401", true),
        ("stm32f405", true),
        ("stm32f407", true),
        ("stm32f767", true),
    ] {
        assert_eq!(load(chip).analog_pins, f1_f4_assignment(with_pc), "{chip}");
    }
}

/// Families whose assignment differs from the F1/F4 map were deliberately NOT
/// transcribed. On an L476 PA0 is ADC1_IN5 and on a G474 it is ADC1_IN1; an
/// entry there must come from that part's datasheet, not from this map.
#[test]
fn families_with_a_different_assignment_declare_nothing_yet() {
    for chip in ["stm32l476", "stm32g474re", "stm32h563", "stm32wb55"] {
        assert!(
            load(chip).analog_pins.is_empty(),
            "{chip} gained analog_pins; check them against its own datasheet and update this test"
        );
    }
}

#[test]
fn a_descriptor_without_analog_pins_parses_to_an_empty_map() {
    let yaml = "name: \"bare\"\narch: \"arm\"\ncore: \"cortex-m4\"\n\
                flash: { base: 0, size: \"4KB\" }\n\
                ram: { base: 0x20000000, size: \"1KB\" }\nperipherals: []\n";
    let desc: ChipDescriptor = serde_yaml::from_str(yaml).expect("parse");
    assert!(desc.analog_pins.is_empty());
}

#[test]
fn an_analog_pin_entry_rejects_unknown_fields() {
    let yaml = "name: \"bad\"\narch: \"arm\"\ncore: \"cortex-m4\"\n\
                flash: { base: 0, size: \"4KB\" }\n\
                ram: { base: 0x20000000, size: \"1KB\" }\nperipherals: []\n\
                analog_pins:\n  PA0: { adc: \"adc1\", channel: 0 }\n";
    assert!(
        serde_yaml::from_str::<ChipDescriptor>(yaml).is_err(),
        "a misspelled key must fail loudly, not parse as a pad with no ADC"
    );
}
