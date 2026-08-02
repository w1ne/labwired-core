// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// A built-in chip (`chip: "stm32f103"`) is resolved from the binary, so it has
// no directory on disk to resolve relative paths against. Any peripheral
// `debug_schema` it names must therefore come from the embedded descriptor
// registry — otherwise `resolve_peripheral_path` falls back to a path that
// does not exist in the consumer's repo and the schema is silently dropped.
//
// These two tests are the drift guards for that contract. Without them, adding
// a chip file or a `debug_schema:` line degrades built-in chips quietly.

use labwired_config::{embedded_chip_yaml, ChipDescriptor, BUILTIN_CHIP_NAMES};
use std::path::PathBuf;

fn configs_chips_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips")
}

/// Every chip descriptor we ship is offered as a built-in name. A new
/// `configs/chips/*.yaml` that nobody added to `BUILTIN_CHIP_NAMES` would be
/// invisible to `chip: "<name>"` while looking supported.
#[test]
fn every_shipped_chip_file_is_offered_as_a_builtin() {
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(configs_chips_dir()).expect("read configs/chips") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_none_or(|e| e != "yaml") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        // `ci-fixture-*` are internal harness fixtures, not silicon we offer.
        if stem.starts_with("ci-fixture-") {
            continue;
        }
        if !BUILTIN_CHIP_NAMES.contains(&stem.as_str()) {
            missing.push(stem);
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "these chip descriptors ship but are not reachable as `chip: \"<name>\"`. \
         Add them to BUILTIN_CHIP_NAMES and embedded_chip_yaml: {missing:?}"
    );
}

/// A built-in chip must not depend on a file next to it, because there is no
/// "next to it" once it is compiled into the binary.
#[test]
fn no_builtin_chip_depends_on_a_file_beside_it() {
    let mut unresolvable = Vec::new();
    for name in BUILTIN_CHIP_NAMES {
        let yaml = embedded_chip_yaml(name).expect("advertised name is embedded");
        let chip: ChipDescriptor =
            serde_yaml::from_str(yaml).unwrap_or_else(|e| panic!("parse '{name}': {e}"));
        for peripheral in &chip.peripherals {
            let Some(schema) = peripheral
                .config
                .get("debug_schema")
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            if crate::bus::embedded_descriptors::lookup(schema).is_none() {
                unresolvable.push(format!("{name}: {} -> {schema}", peripheral.id));
            }
        }
    }
    assert!(
        unresolvable.is_empty(),
        "these built-in chips reference peripheral descriptors that are not embedded, so \
         they resolve to a path that will not exist for a consumer of the released binary. \
         Embed them in bus::embedded_descriptors: {unresolvable:#?}"
    );
}
