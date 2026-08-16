// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! How many chips still get their pin numbers by parsing the label.
//!
//! A chip descriptor may declare `pins:` — an authoritative label → (gpio, bit)
//! map transcribed from the datasheet. `from_config.rs` prefers it over the
//! label-letter parse, because the parse is a guess: it reads `PB6` as bank B,
//! bit 6, which is right for a great many STM32 parts and wrong for anything
//! that numbers its banks differently. mkw41z4 is the chip that proved it —
//! every one of its `P*` labels lives on `gpioc`.
//!
//! ⚠️ THIS IS NOT AN AGREEMENT CHECK, AND MUST NOT BECOME ONE. `pins:` exists
//! precisely to CONTRADICT the parse. A gate asserting the two agree would fail
//! on exactly the chips the mechanism was built for, and would push someone to
//! "fix" a correct datasheet transcription to match a wrong guess.
//!
//! What can be gated is the direction of travel. The count below only shrinks:
//! a chip may not be added without either declaring its pins or being recorded
//! here as still relying on the parse. That keeps the migration honest without
//! inventing a single pin number — the numbers themselves are datasheet work,
//! one chip at a time, and a wrong one is worse than the guess it replaces
//! because it looks authoritative.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Chips whose pin numbers still come from parsing the label.
///
/// ONLY SHRINKS. Adding a name here is a deliberate, reviewable act; the test
/// below fails if an entry no longer needs to be here, so the list cannot rot
/// into a set of excuses for chips that have since been transcribed.
const PARSE_FALLBACK_CHIPS: &[&str] = &[
    "atmega328p",
    "esp32",
    "esp32c3",
    "esp32s3",
    "esp32s3-zero",
    "nrf52832",
    "nrf52840",
    "nrf5340",
    "nrf54l15",
    "rp2040",
    "rp2350",
    "stm32f103",
    "stm32f401",
    "stm32f401cdu6",
    "stm32f405",
    "stm32f407",
    "stm32f411ceu6",
    "stm32f767",
    "stm32g474re",
    "stm32h563",
    "stm32h735",
    "stm32l073",
    "stm32l476",
    "stm32wb55",
    "stm32wba52",
];

fn chips_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips")
}

/// Every shippable chip descriptor, by name. `ci-fixture-*` are synthetic and
/// are excluded here for the same reason build.rs excludes them from the
/// built-in registry.
fn shippable_chips() -> BTreeSet<String> {
    std::fs::read_dir(chips_dir())
        .expect("read configs/chips")
        .filter_map(|e| {
            let path = e.expect("dir entry").path();
            if path.extension()? != "yaml" {
                return None;
            }
            let stem = path.file_stem()?.to_str()?.to_string();
            (!stem.starts_with("ci-fixture-")).then_some(stem)
        })
        .collect()
}

fn declares_pins(chip: &str) -> bool {
    let text = std::fs::read_to_string(chips_dir().join(format!("{chip}.yaml")))
        .unwrap_or_else(|e| panic!("read {chip}.yaml: {e}"));
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("parse {chip}.yaml: {e}"));
    doc.get("pins")
        .and_then(|p| p.as_mapping())
        .is_some_and(|m| !m.is_empty())
}

#[test]
fn every_chip_is_accounted_for() {
    let listed: BTreeSet<String> = PARSE_FALLBACK_CHIPS.iter().map(|s| s.to_string()).collect();
    let chips = shippable_chips();

    let unaccounted: Vec<&String> = chips
        .iter()
        .filter(|c| !declares_pins(c) && !listed.contains(*c))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "these chips declare no `pins:` and are not recorded as parse-fallback: {unaccounted:?}.\n\
         Either transcribe the pin map from the datasheet (preferred — the parse is a guess),\n\
         or add the name to PARSE_FALLBACK_CHIPS with the rest."
    );
}

#[test]
fn the_fallback_list_only_shrinks() {
    let chips = shippable_chips();

    // An entry whose chip now declares `pins:` is stale. Removing it is the
    // point of the migration, so the gate demands it rather than letting the
    // list quietly outlive the work.
    let stale: Vec<&&str> = PARSE_FALLBACK_CHIPS
        .iter()
        .filter(|c| chips.contains(**c) && declares_pins(c))
        .collect();
    assert!(
        stale.is_empty(),
        "these chips now declare `pins:` — delete them from PARSE_FALLBACK_CHIPS: {stale:?}"
    );

    // An entry naming a chip that no longer ships is also stale.
    let gone: Vec<&&str> = PARSE_FALLBACK_CHIPS
        .iter()
        .filter(|c| !chips.contains(**c))
        .collect();
    assert!(
        gone.is_empty(),
        "PARSE_FALLBACK_CHIPS names chips that no longer ship: {gone:?}"
    );
}

#[test]
fn at_least_one_chip_proves_the_mechanism_works() {
    // The whole point of `pins:` is that it OVERRIDES the label parse. If no
    // shipped chip exercises it, the override path is untested in production
    // and this ratchet is measuring a mechanism nobody uses.
    let with_pins: Vec<String> = shippable_chips()
        .into_iter()
        .filter(|c| declares_pins(c))
        .collect();
    assert!(
        !with_pins.is_empty(),
        "no shipped chip declares `pins:` — the authoritative-pin-map path is dead code"
    );
}
