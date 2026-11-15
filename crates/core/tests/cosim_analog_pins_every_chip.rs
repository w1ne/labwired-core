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
