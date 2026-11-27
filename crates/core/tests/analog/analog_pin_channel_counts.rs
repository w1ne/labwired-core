// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every pad a chip descriptor publishes as an ADC input must name a channel
//! the built ADC model accepts. The analog router checks each
//! `board.analog.<pad>_volts` route against the model's
//! `adc_channel_count()`, so a pad map entry past that count binds with
//! `NoSuchAdcChannel` even though the map itself is valid silicon. The
//! descriptor-only validator (`labwired-config` `chip_analog_pins`) cannot see
//! the model, so this test builds one bus per chip and compares the two.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use std::collections::BTreeSet;

#[test]
fn every_published_adc_pad_is_a_channel_the_model_accepts() {
    let mut profiles: Vec<_> = std::fs::read_dir(root("configs/systems"))
        .expect("read configs/systems")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    profiles.sort();

    let mut seen_chips = BTreeSet::new();
    let mut checked = 0usize;
    let mut problems = Vec::new();
    for system_path in profiles {
        let Ok(manifest) = SystemManifest::from_file(&system_path) else {
            continue;
        };
        let chip_path = system_path.parent().unwrap().join(&manifest.chip);
        let Ok(chip_path) = chip_path.canonicalize() else {
            continue;
        };
        if !seen_chips.insert(chip_path.clone()) {
            continue;
        }
        let Ok(chip) = ChipDescriptor::from_file(&chip_path) else {
            continue;
        };
        let table = chip.analog_pin_table();
        if table.is_empty() {
            continue;
        }
        let bus = SystemBus::from_config(&chip, &manifest).unwrap_or_else(|error| {
            panic!("build {} for {}: {error}", chip.name, system_path.display())
        });
        for (pad, adc) in &table {
            let Some(index) = bus.find_peripheral_index_by_name(&adc.peripheral) else {
                problems.push(format!(
                    "{}: {pad} names missing '{}'",
                    chip.name, adc.peripheral
                ));
                continue;
            };
            match bus.peripherals[index].dev.adc_channel_count() {
                None => problems.push(format!(
                    "{}: {pad} names '{}', which reports no ADC channels",
                    chip.name, adc.peripheral
                )),
                Some(count) if adc.channel >= count => problems.push(format!(
                    "{}: {pad} is channel {}, but the '{}' model accepts {count}",
                    chip.name, adc.channel, adc.peripheral
                )),
                Some(_) => {}
            }
        }
        checked += 1;
    }
    // 38 chips publish an ADC pad map today. A floor at that count makes a
    // chip whose YAML stops loading fail here instead of dropping out.
    assert!(
        checked >= 38,
        "only {checked} chips with ADC pad maps were built"
    );
    assert!(
        problems.is_empty(),
        "pad map entries the router would refuse:\n{}",
        problems.join("\n")
    );
}
