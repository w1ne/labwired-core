// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every `analog_pins:` entry on every shipped chip descriptor reaches a real
//! ADC input through the generic co-simulation route.
//!
//! The per-board analog tests prove a handful of pads end to end. This one
//! covers the rest of the map: for each chip, a mock model drives
//! `board.analog.<pad>_volts` on EVERY declared pad at once, and the session
//! must bind each path (the named peripheral exists, answers
//! `adc_channel_count`, and has the channel) and then apply each value (the
//! ADC model's `set_adc_channel_input` accepts it). A descriptor entry that
//! names a stub, a non-ADC, or a channel the model does not have fails here,
//! whichever family it belongs to — no per-family type or label list.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, CosimAdapter, CosimModelConfig, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, RoutingError};
use labwired_core::pins::External;
use std::collections::HashMap;
use std::path::Path;

fn chips_with_analog_pins() -> Vec<(String, ChipDescriptor)> {
    let mut chips: Vec<(String, ChipDescriptor)> = std::fs::read_dir(root("configs/chips"))
        .expect("read configs/chips")
        .filter_map(|entry| {
            let path = entry.expect("dir entry").path();
            if path.extension()? != "yaml" {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            let desc = ChipDescriptor::from_file(&path)
                .unwrap_or_else(|e| panic!("load {}: {e}", path.display()));
            (!desc.analog_pins.is_empty()).then_some((name, desc))
        })
        .collect();
    chips.sort_by(|a, b| a.0.cmp(&b.0));
    chips
}

/// A bare bus for one shipped chip: no external devices, so nothing but the
/// co-simulation writes the ADC inputs.
fn chip_bus(chip: &str, desc: &ChipDescriptor) -> SystemBus {
    let chip_path = root(&format!("configs/chips/{chip}.yaml"));
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: \"analog-pins-{chip}\"\nchip: \"{}\"\nexternal_devices: []\n",
        chip_path.display()
    ))
    .expect("parse manifest");
    SystemBus::from_config(desc, &manifest).unwrap_or_else(|e| panic!("{chip}: build bus: {e:#}"))
}

/// A mock model that holds `volts` on every declared analog pad of `desc`.
fn drive_every_pad(desc: &ChipDescriptor, volts: f64) -> CosimModelConfig {
    let mut outputs = HashMap::new();
    let mut statics = serde_yaml::Mapping::new();
    for pad in desc.analog_pins.keys() {
        let signal = format!("v_{}", pad.to_ascii_lowercase().replace('.', "_"));
        outputs.insert(signal.clone(), format!("board.analog.{pad}_volts"));
        statics.insert(signal.into(), volts.into());
    }
    let mut config = HashMap::new();
    config.insert("outputs".to_string(), serde_yaml::Value::Mapping(statics));
    CosimModelConfig {
        id: "every_analog_pad".to_string(),
        adapter: CosimAdapter::Mock,
        model: None,
        step_ns: 100_000,
        inputs: HashMap::new(),
        outputs,
        config,
    }
}

#[test]
fn every_declared_analog_pad_binds_and_applies_on_its_adc() {
    let chips = chips_with_analog_pins();
    assert!(
        chips.len() >= 20,
        "expected the shipped analog_pins maps, found {}",
        chips.len()
    );
    let mut problems = Vec::new();
    let mut routed = 0usize;
    for (chip, desc) in &chips {
        let mut bus = chip_bus(chip, desc);
        let models = [drive_every_pad(desc, 1.0)];
        let mut session = CosimSession::new(&models, Path::new("."), &bus)
            .unwrap_or_else(|e| panic!("{chip}: build session: {e:#}"))
            .expect("a model was declared");
        let bind: &[RoutingError] = session.binding_errors();
        problems.extend(bind.iter().map(|e| format!("{chip}: bind: {e}")));
        if !bind.is_empty() {
            continue;
        }
        let boundary = session.cycles_until_boundary(0);
        let (steps, apply) = session
            .advance_to(boundary, &mut bus)
            .unwrap_or_else(|e| panic!("{chip}: advance: {e:#}"));
        assert!(!steps.is_empty(), "{chip}: the mock model never stepped");
        problems.extend(apply.iter().map(|e| format!("{chip}: apply: {e}")));
        routed += desc.analog_pins.len();
    }
    assert!(
        problems.is_empty(),
        "analog_pins entries that do not reach an ADC input:\n{}",
        problems.join("\n")
    );
    eprintln!(
        "{routed} analog pads on {} chips bind and apply",
        chips.len()
    );
}

/// The test above can fail: on families the static descriptor check knows
/// nothing about, a channel the ADC model lacks and a peripheral that is not
/// an ADC are both refused when the session binds.
#[test]
fn a_wrong_channel_or_a_non_adc_is_refused_on_every_family_shape() {
    for (chip, non_adc) in [
        ("s32k388", "siul2"),
        ("mk64f12", "gpioa"),
        ("same70q21b", "pioA"),
        ("atsamd21", "porta"),
        ("rp2350", "sio"),
        ("pic24f16ka102", "gpioa"),
    ] {
        let desc = chips_with_analog_pins()
            .into_iter()
            .find(|(name, _)| name == chip)
            .unwrap_or_else(|| panic!("{chip} declares analog_pins"))
            .1;
        let pad = desc.analog_pins.keys().next().unwrap().clone();

        let mut bad_channel = desc.clone();
        bad_channel.analog_pins.get_mut(&pad).unwrap().channel = u8::MAX;
        let mut not_an_adc = desc.clone();
        not_an_adc.analog_pins.get_mut(&pad).unwrap().peripheral = non_adc.to_string();

        for (what, broken, expect) in [
            ("channel 255", bad_channel, "NoSuchAdcChannel"),
            ("non-ADC peripheral", not_an_adc, "NotAnAdc"),
        ] {
            let bus = chip_bus(chip, &broken);
            let session = CosimSession::new(&[drive_every_pad(&broken, 1.0)], Path::new("."), &bus)
                .unwrap()
                .unwrap();
            let errors = session.binding_errors();
            assert_eq!(errors.len(), 1, "{chip}: {what} on {pad}: {errors:?}");
            assert!(
                format!("{:?}", errors[0]).starts_with(expect),
                "{chip}: {what} on {pad} must be {expect}, got {:?}",
                errors[0]
            );
        }
    }
}

/// Pads whose port the chip descriptor does not model as a pin port, so
/// `board.gpio_in.<pad>` has no input buffer to drive. Each is a real model
/// gap, listed so the test fails when the port is modelled (and the entry
/// must go) or when any OTHER pad stops resolving.
const DIGITAL_PORTS_NOT_MODELLED: &[(&str, &[&str], &str)] = &[
    (
        "mkw41z4",
        &["PTA19", "PTB1", "PTB2", "PTB3", "PTB18"],
        "only GPIOC is modelled; the `pins:` PBn labels are board names for PTC pads",
    ),
    (
        "ra4m1",
        &["P000", "P001", "P002", "P014"],
        "PORT0 is not modelled",
    ),
    ("stm32g474re", &["PF0"], "GPIOF is not modelled"),
    (
        "vegaboard-ri5cy",
        &[
            "PTB3", "PTB4", "PTB7", "PTC9", "PTC10", "PTC11", "PTC12", "PTD5", "PTD7", "PTD9",
        ],
        "GPIOB..D are not declared",
    ),
    (
        "r5f52618bgfp",
        &[
            "P40", "P41", "P42", "P43", "P44", "P45", "P46", "P47", "PE0", "PE1", "PE2", "PE3",
            "PE4", "PE5", "PE6", "PE7", "PD0", "PD1", "PD2", "PD3", "PD4",
        ],
        "PORT0-J and MPC are a declared stub",
    ),
    (
        "pic16f1829",
        &[
            "RA0", "RA1", "RA2", "RA4", "RC0", "RC1", "RC2", "RC3", "RC6", "RC7", "RB4", "RB5",
        ],
        "PORTA..PORTC are SFRs inside the PIC16 core, with no external pin drive",
    ),
    (
        "pic16f877a",
        &["RA0", "RA1", "RA2", "RA3", "RA5", "RE0", "RE1", "RE2"],
        "PORTA/PORTE are SFRs inside the PIC16 core, with no external pin drive",
    ),
    (
        "pic18f4520",
        &[
            "RA0", "RA1", "RA2", "RA3", "RA5", "RE0", "RE1", "RE2", "RB2", "RB3", "RB1", "RB4",
            "RB0",
        ],
        "PORTA/PORTB/PORTE are SFRs inside the PIC18 core, with no external pin drive",
    ),
];

/// A mock model that holds `level` on `board.gpio_in.<pad>` for each of `pads`.
fn drive_digital(pads: &[&str], level: bool) -> CosimModelConfig {
    let mut outputs = HashMap::new();
    let mut statics = serde_yaml::Mapping::new();
    for pad in pads {
        let signal = format!("l_{}", pad.to_ascii_lowercase());
        outputs.insert(signal.clone(), format!("board.gpio_in.{pad}"));
        statics.insert(signal.into(), level.into());
    }
    let mut config = HashMap::new();
    config.insert("outputs".to_string(), serde_yaml::Value::Mapping(statics));
    CosimModelConfig {
        id: "digital_pads".to_string(),
        adapter: CosimAdapter::Mock,
        model: None,
        step_ns: 100_000,
        inputs: HashMap::new(),
        outputs,
        config,
    }
}

/// Every `(peripheral name, pin)` whose input buffer an external driver
/// holds high.
fn externally_high(bus: &SystemBus) -> Vec<(String, u8)> {
    let mut high = Vec::new();
    for p in &bus.peripherals {
        let Some(port) = p.dev.pins() else { continue };
        for pin in 0..port.pin_count() {
            if port.external(pin) == External::Level(true) {
                high.push((p.name.clone(), pin));
            }
        }
    }
    high
}

/// Drive `pad` high on its own and return the one input buffer that took it,
/// or why it did not.
fn drive_one_pad_high(bus: &mut SystemBus, pad: &str) -> Result<(String, u8), String> {
    let before = externally_high(bus);
    let mut session = CosimSession::new(&[drive_digital(&[pad], true)], Path::new("."), bus)
        .map_err(|e| format!("session: {e:#}"))?
        .expect("a model was declared");
    if let [error, ..] = session.binding_errors() {
        return Err(format!("bind: {error:?}"));
    }
    let boundary = session.cycles_until_boundary(0);
    let (_, apply) = session
        .advance_to(boundary, bus)
        .map_err(|e| format!("advance: {e:#}"))?;
    if let [error, ..] = apply.as_slice() {
        return Err(format!("apply: {error}"));
    }
    let new: Vec<_> = externally_high(bus)
        .into_iter()
        .filter(|pin| !before.contains(pin))
        .collect();
    match new.as_slice() {
        [one] => Ok(one.clone()),
        other => Err(format!("drove {} input buffers: {other:?}", other.len())),
    }
}

/// The digital side of the same map: every pad an `analog_pins:` entry names
/// is also a GPIO input (an ADC pad is a GPIO pad on every family here), and
/// `board.gpio_in.<pad>` must reach that pad's input buffer — one buffer per
/// pad, a different one for each pad — whether or not the descriptor's
/// `pins:` lists it. The label shapes are resolved by one generic rule
/// (`labwired_core::pins::parse_pad_label`), not a per-chip table.
#[test]
fn every_analog_pad_is_also_a_distinct_digital_input() {
    let mut problems = Vec::new();
    let mut unmodelled = Vec::new();
    let mut routed = 0usize;
    for (chip, desc) in chips_with_analog_pins() {
        let mut bus = chip_bus(&chip, &desc);
        let mut taken: HashMap<(String, u8), String> = HashMap::new();
        for pad in desc.analog_pins.keys() {
            match drive_one_pad_high(&mut bus, pad) {
                Ok(pin) => {
                    if let Some(other) = taken.insert(pin.clone(), pad.clone()) {
                        problems.push(format!("{chip}: {pad} and {other} share input {pin:?}"));
                    }
                    routed += 1;
                }
                Err(e) if e.starts_with("bind: UnknownPad") => {
                    unmodelled.push(format!("{chip}:{pad}"))
                }
                Err(e) => problems.push(format!("{chip}: {pad}: {e}")),
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    let mut expected: Vec<String> = DIGITAL_PORTS_NOT_MODELLED
        .iter()
        .flat_map(|(chip, pads, _)| pads.iter().map(move |pad| format!("{chip}:{pad}")))
        .collect();
    expected.sort();
    unmodelled.sort();
    assert_eq!(
        unmodelled, expected,
        "only the pads on unmodelled ports may fail to resolve"
    );
    eprintln!("{routed} analog pads also resolve as digital inputs");
}

/// Datasheet anchors for each label shape: the pad lands on the right port
/// and pin, not merely on some pin.
#[test]
fn each_label_shape_lands_on_its_datasheet_pin() {
    for (chip, pad, peripheral, pin, source) in [
        ("atsamd21", "PA08", "porta", 8, "SAM D21 PORT group 0 bit 8"),
        (
            "atsamd21g18a",
            "PA02",
            "gpio_a",
            2,
            "`pins:` PA2 (zero padding)",
        ),
        ("atsamd51", "PB08", "portb", 8, "SAM D5x PORT group 1 bit 8"),
        ("cc2538sf53", "PA2", "gpioa", 2, "CC2538 GPIO A DATA bit 2"),
        ("mk64f12", "PTB10", "gpiob", 10, "K64 GPIOB PDIR bit 10"),
        (
            "dspic33fj32mc202",
            "RB2",
            "gpiob",
            2,
            "dsPIC33F PORTB bit 2",
        ),
        ("ra4m1", "P100", "port1", 0, "RA4M1 PORT1 PIDR bit 0"),
        (
            "mimxrt595s",
            "P4_18",
            "gpio",
            146,
            "RT500 GPIO PIN[4] bit 18 = 32*4+18",
        ),
        (
            "s32k388",
            "PTB13",
            "siul2",
            45,
            "S32K3 SIUL2 MSCR 45 = 32*1+13",
        ),
        ("rp2350", "GP26", "sio", 26, "RP2350 SIO GPIO_IN bit 26"),
        ("sam4s16c", "PA19", "pioA", 19, "SAM4S PIOA PDSR bit 19"),
        ("same70q21b", "PB0", "pioB", 0, "SAM E70 PIOB PDSR bit 0"),
    ] {
        let desc = chips_with_analog_pins()
            .into_iter()
            .find(|(name, _)| name == chip)
            .unwrap_or_else(|| panic!("{chip} declares analog_pins"))
            .1;
        let mut bus = chip_bus(chip, &desc);
        let got = drive_one_pad_high(&mut bus, pad).unwrap_or_else(|e| panic!("{chip} {pad}: {e}"));
        assert_eq!(
            got,
            (peripheral.to_string(), pin),
            "{chip} {pad} must be {source}"
        );
        let port = bus.peripherals[bus.find_peripheral_index_by_name(peripheral).unwrap()]
            .dev
            .pins()
            .unwrap();
        assert_eq!(
            port.input(pin),
            Some(true),
            "{chip} {pad}: the firmware-visible input register must read the driven level"
        );
    }
}
