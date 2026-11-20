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
        let Some(adc) = desc.peripherals.iter().find(|p| &p.id == peripheral) else {
            problems.push(format!(
                "{chip}: {pad} names ADC '{peripheral}', not declared"
            ));
            continue;
        };
        // Whether the named peripheral really converts that channel is a
        // property of the core model, not of the descriptor text: the core test
        // `cosim_analog_pins_every_chip` binds and applies every entry.
        if adc.r#type == "stub" {
            problems.push(format!("{chip}: {pad} names '{peripheral}', a stub"));
        }
        let nordic = adc.r#type == "nrf52840_saadc";
        // The chip's own `pins:` map is the authority for its pad labels
        // (PTA17, P0_12, GP26, P000 ...). Only a chip without one falls back
        // to the label grammar: P<port><bit>, Nordic P<port>.<bit>, AVR
        // port<x>, Microchip R<port><bit>.
        let located = desc
            .pins
            .iter()
            .find(|(label, _)| label.eq_ignore_ascii_case(pad))
            .map(|(_, loc)| (vec![loc.gpio.clone()], loc.bit))
            .or_else(|| {
                pad_port_and_bit(
                    pad,
                    nordic,
                    adc.r#type == "avr_adc",
                    adc.r#type == "pic24_adc_pads",
                )
            });
        let unmodelled_port = PORTS_NOT_MODELLED.iter().any(|(c, _)| *c == chip);
        match located {
            Some(_) if unmodelled_port => {}
            Some((ports, bit)) => match desc
                .peripherals
                .iter()
                .find(|p| ports.contains(&p.id) && is_port_model(&p.r#type))
            {
                Some(p) => {
                    let port = &p.id;
                    // A PIC24 port's `mask` holds the bits the package bonds.
                    if let Some(mask) = p.config.get("mask").and_then(serde_yaml::Value::as_u64) {
                        if bit >= 16 || mask & (1 << bit) == 0 {
                            problems.push(format!(
                                "{chip}: {pad} is not bonded ({port} mask {mask:#06x})"
                            ));
                        }
                    }
                    if let Some(count) =
                        p.config.get("num_pins").and_then(serde_yaml::Value::as_u64)
                    {
                        if u64::from(bit) >= count {
                            problems
                                .push(format!("{chip}: {pad} is outside {port}'s {count} pins"));
                        }
                    }
                    if bit >= 32 {
                        problems.push(format!("{chip}: {pad} has no 32-bit port bit {bit}"));
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
                None => problems.push(format!(
                    "{chip}: {pad} is on none of {ports:?}, not a declared GPIO"
                )),
            },
            None => problems.push(format!(
                "{chip}: {pad} is neither in `pins:` nor a P<port><bit> label"
            )),
        }
        let shared = SHARED_INPUTS
            .iter()
            .any(|(c, pads, _)| *c == chip && pads.contains(&pad.as_str()));
        if let Some(other) = seen.insert((peripheral.clone(), *channel), pad.clone()) {
            if shared {
                continue;
            }
            problems.push(format!(
                "{chip}: {pad} and {other} both claim {peripheral} channel {channel}"
            ));
        }
    }
    problems
}

/// A modelled GPIO port block: `gpio`, `stm32f4_gpio`, `sam4s_pio`,
/// `cc2538_pl061_gpio`, `avr_gpio`, `pic24_gpio`, or a `declarative` port.
fn is_port_model(r#type: &str) -> bool {
    r#type.contains("gpio") || r#type.ends_with("_pio") || r#type == "declarative"
}

/// Chips whose pad labels name a port the descriptor does not model as its
/// own block, so the port lookup cannot apply. The core test
/// `cosim_analog_pins_every_chip` still proves each entry converts.
const PORTS_NOT_MODELLED: &[(&str, &str)] = &[
    (
        "rp2350",
        "GPn pads are one SIO bank (`sio`), not lettered ports",
    ),
    (
        "s32k388",
        "PTxn pads are SIUL2 MSCR indices (`siul2`), not port blocks",
    ),
    (
        "mimxrt595s",
        "Pn_m pads are one `lpc_gpio` block for every port",
    ),
    ("ra4m1", "AN000-AN002 are on PORT0, which is not modelled"),
    (
        "stm32g474re",
        "PF0 (ADC1_IN10) is on GPIOF, which is not modelled",
    ),
    (
        "vegaboard-ri5cy",
        "PTB/PTC/PTD pads are on GPIOB..D, which the descriptor does not declare",
    ),
    (
        "pic16f877a",
        "PORTA/PORTE are SFRs inside the PIC16 core, not bus port blocks",
    ),
];

/// Pads that one ADC input really reaches twice. S32K388 RTD 7.0.1
/// Port_Cfg.h (see the descriptor's analog_pins note) defines the same
/// ADC0_S8/S9 and ADC1_S22/S23 signal on two MSCRs each.
const SHARED_INPUTS: &[(&str, &[&str], &str)] = &[(
    "s32k388",
    &[
        "PTA0", "PTB13", "PTA1", "PTB14", "PTD28", "PTE17", "PTD29", "PTE18",
    ],
    "RTD Port_Cfg.h: ADC0_S8/S9, ADC1_S22/S23 on two pads each",
)];

/// The port ids a pad label may live on, and its bit. Labels are
/// `P<port><bit>` (STM32, SAM, SAMD, CC2538: gpioa / gpio_a / porta / pioA),
/// `PT<port><bit>` (Kinetis, S32K), `P<digit><bit2>` (Renesas RA),
/// Nordic `P<port>.<bit>`, AVR `P<port><bit>` on `port<x>`, and Microchip
/// `R<port><bit>`.
fn pad_port_and_bit(pad: &str, nordic: bool, avr: bool, pic24: bool) -> Option<(Vec<String>, u8)> {
    let upper = pad.to_ascii_uppercase();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    if nordic {
        let (port, bit) = upper.strip_prefix('P')?.split_once('.')?;
        if port.len() != 1 || bit.len() != 2 || !digits(port) || !digits(bit) {
            return None;
        }
        return Some((vec![format!("gpio{port}")], bit.parse().ok()?));
    }
    if pic24 || avr {
        let rest = upper.strip_prefix(if pic24 { 'R' } else { 'P' })?;
        let port = rest.chars().next()?.to_ascii_lowercase();
        if !port.is_ascii_alphabetic() || !digits(&rest[1..]) {
            return None;
        }
        let prefix = if avr { "port" } else { "gpio" };
        return Some((vec![format!("{prefix}{port}")], rest[1..].parse().ok()?));
    }
    if let Some(rest) = upper.strip_prefix("GP") {
        return digits(rest).then(|| Some((vec!["sio".to_string()], rest.parse().ok()?)))?;
    }
    let rest = upper
        .strip_prefix("PT")
        .filter(|r| r.starts_with(|c: char| c.is_ascii_alphabetic()))
        .or_else(|| upper.strip_prefix('P'))?;
    if let Some((port, bit)) = rest.split_once('_') {
        return (digits(port) && digits(bit))
            .then(|| Some((vec!["gpio".to_string()], bit.parse().ok()?)))?;
    }
    let port = rest.chars().next()?;
    if port.is_ascii_digit() {
        // Renesas RA: P<port digit><two-digit bit>.
        return (rest.len() == 3 && digits(rest))
            .then(|| Some((vec![format!("port{port}")], rest[1..].parse().ok()?)))?;
    }
    if !port.is_ascii_alphabetic() || !digits(&rest[1..]) {
        return None;
    }
    let lower = port.to_ascii_lowercase();
    Some((
        vec![
            format!("gpio{lower}"),
            format!("gpio_{lower}"),
            format!("port{lower}"),
            format!("pio{port}"),
        ],
        rest[1..].parse().ok()?,
    ))
}

#[test]
fn atmega328p_bonded_analog_pads_resolve_to_their_avr_adc_channels() {
    let desc = load("atmega328p");
    assert_eq!(desc.analog_pins.len(), 6);
    for channel in 0..6 {
        assert_eq!(
            desc.analog_pins[&format!("PC{channel}")],
            AdcPinFn {
                peripheral: "adc".to_string(),
                channel,
            }
        );
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

/// Teensyduino's `pin_to_channel[]` table maps A0..A17 to the RT1062 pads and
/// ADC instances. A wrong channel is more damaging than a missing route: the
/// co-simulation would silently drive a different ADC input.
#[test]
fn imxrt1064_teensy_analog_pads_match_the_physical_adc_assignments() {
    let desc = load("imxrt1064");
    let expected = [
        ("AD_B1_02", "adc1", 7, 18),
        ("AD_B1_03", "adc1", 8, 19),
        ("AD_B1_07", "adc1", 12, 23),
        ("AD_B1_06", "adc1", 11, 22),
        ("AD_B1_01", "adc1", 6, 17),
        ("AD_B1_00", "adc1", 5, 16),
        ("AD_B1_10", "adc1", 15, 26),
        ("AD_B1_11", "adc1", 0, 27),
        ("AD_B1_08", "adc1", 13, 24),
        ("AD_B1_09", "adc1", 14, 25),
        ("AD_B0_12", "adc1", 1, 12),
        ("AD_B0_13", "adc1", 2, 13),
        ("AD_B1_14", "adc2", 3, 30),
        ("AD_B1_15", "adc2", 4, 31),
        ("AD_B1_12", "adc2", 1, 28),
        ("AD_B1_13", "adc2", 2, 29),
        ("AD_B1_04", "adc1", 9, 20),
        ("AD_B1_05", "adc1", 10, 21),
    ];
    assert_eq!(desc.analog_pins.len(), expected.len());
    for (pad, peripheral, channel, gpio_bit) in expected {
        assert_eq!(
            desc.analog_pins[pad],
            AdcPinFn {
                peripheral: peripheral.to_string(),
                channel,
            },
            "{pad} ADC route"
        );
        assert_eq!(desc.pins[pad].gpio, "gpio1", "{pad} GPIO port");
        assert_eq!(desc.pins[pad].bit, gpio_bit, "{pad} GPIO bit");
    }
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
/// G474 (32a7829f7) and WB55 (59741ae65) have since gained their own maps.
#[test]
fn families_with_a_different_assignment_declare_nothing_yet() {
    for chip in ["stm32l476", "stm32h563"] {
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
