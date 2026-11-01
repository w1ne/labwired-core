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
//!
//! RV1103 is declared CPU/memory geometry only. It has no modeled GPIO or pin
//! resolver to exercise, so it is explicitly accounted for separately from
//! parse fallback. This exception expires when pins or peripherals are added.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Chips whose pin numbers still come from parsing the label.
///
/// ONLY SHRINKS. Adding a name here is a deliberate, reviewable act; the test
/// below fails if an entry no longer needs to be here, so the list cannot rot
/// into a set of excuses for chips that have since been transcribed.
const PARSE_FALLBACK_CHIPS: &[&str] = &[
    // ATmega2560 rides the same AVR P<port><bit> label parse as the 328P
    // (PB7 = portb bit 7, PK0 = portk bit 0); its config declares no `pins:`.
    "atmega2560",
    "atmega328p",
    // PIC24 M0 declares no `pins:` yet — the 44-pin map transcription is an
    // M1 item (docs/pic24-m0-reference.md). The M0 fixture drives no GPIO, so
    // no pad is read as a fact and there is nothing to mis-parse.
    "dspic33fj32mc204",
    // Same PIC24 M0 scope for the 28-pin sibling; no `pins:` map yet.
    "dspic33fj32mc202",
    // PIC18F4520 L1: ports are CPU-internal SFRs and no package pad model
    // or `pins:` map exists yet, so nothing is parsed as a pad fact.
    "pic18f4520",
    // PIC16F877A L1: same as PIC18F4520, ports are CPU-internal SFRs and there
    // is no `pins:` map yet.
    "pic16f877a",
    // PIC16F1829 L1: same, ports are CPU-internal SFRs on the enhanced core.
    "pic16f1829",
    // EFR32 pin labels ARE the parse: Silicon Labs names GPIO pins P<bank><bit>
    // (PB2 = port B, bit 2) straight through the datasheet and UG594, so the
    // label-letter parse is the datasheet truth here, not a guess. There is no
    // alternate package-label scheme to transcribe a `pins:` override from.
    "efr32mg26",
    "esp32",
    "esp32c3",
    "esp32c6",
    "esp32s3",
    "esp32s3-zero",
    // The LiteX VexRiscv SoC models a UART and a timer and no GPIO bank, so
    // there are no pads to read, by parse or by map.
    "litex-vexriscv",
    // The Arty's pads are addressed numerically through the Control-and-Status
    // CSR (`board_io` binds `cas` pin N); no `P<bank><bit>` label exists to
    // parse, and the CAS model is the authoritative pad surface.
    "arty-litex-vexriscv",
    // The Vega board's SoC models UARTs and timers and no GPIO bank, so there
    // are no pads to read, by parse or by map.
    "vegaboard-ri5cy",
    // The PolarFire SoC MSS GPIO_0..2 are declared stubs (no GPIO model), and
    // the kit's LEDs/buttons sit behind the FPGA fabric; no pads to read yet.
    "mpfs095t",
    "nrf52832",
    "nrf52833",
    "nrf52840",
    "nrf5340",
    "nrf54l15",
    // Same reasoning as efr32mg26 above: Nordic names GPIO pins P<port>.<pin>
    // straight through the datasheet and the board devicetree, and
    // `parse_stm32_pin` maps that form directly onto gpio<port> / bit -- so the
    // parse IS the datasheet truth here, not a guess, and there is no alternate
    // package-label scheme to transcribe a `pins:` override from. Checked on
    // this part specifically because it is the first nRF with FOUR ports: P3.04
    // resolves to gpio3 bit 4, and the chip declares gpio3.
    "nrf54lm20a",
    "rp2040",
    "rp2350",
    "stm32f103",
    "stm32f401",
    "stm32f401cdu6",
    "stm32f405",
    "stm32f407",
    "stm32f411ceu6",
    "stm32f767",
    // Pin labels are PA2/PA3/PA5-style, the same STM32 form the parse maps
    // directly onto gpio<port>/bit; no alternate package-label scheme exists
    // to transcribe a `pins:` override from (same reasoning as the peers above).
    "stm32g071",
    "stm32g474re",
    "stm32h563",
    "stm32h735",
    "stm32l073",
    "stm32l476",
    "stm32u545",
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

fn chip_doc(chip: &str) -> serde_yaml::Value {
    let text = std::fs::read_to_string(chips_dir().join(format!("{chip}.yaml")))
        .unwrap_or_else(|e| panic!("read {chip}.yaml: {e}"));
    serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("parse {chip}.yaml: {e}"))
}

fn declares_pins(chip: &str) -> bool {
    let doc = chip_doc(chip);
    doc.get("pins")
        .and_then(|p| p.as_mapping())
        .is_some_and(|m| !m.is_empty())
}

/// A classification of these exact unmodeled-pin SoCs (RV1103; the XCK26 RPU,
/// whose MIO pins belong to the PS, not the R5F), not modeled-pin coverage.
fn declared_only(chip: &str, doc: &serde_yaml::Value) -> bool {
    let core = match chip {
        "rv1103" => "cortex-a7",
        "xck26-rpu" => "cortex-r5f",
        _ => return false,
    };
    doc.get("name").and_then(|v| v.as_str()) == Some(chip)
        && doc.get("arch").and_then(|v| v.as_str()) == Some("arm")
        && doc.get("core").and_then(|v| v.as_str()) == Some(core)
        && doc
            .get("peripherals")
            .and_then(|v| v.as_sequence())
            .is_some_and(|p| p.is_empty())
        && (doc.get("pins").is_none()
            || doc
                .get("pins")
                .and_then(|v| v.as_mapping())
                .is_some_and(|p| p.is_empty()))
}

#[test]
fn every_chip_is_accounted_for() {
    let listed: BTreeSet<String> = PARSE_FALLBACK_CHIPS.iter().map(|s| s.to_string()).collect();
    let chips = shippable_chips();

    let unaccounted: Vec<&String> = chips
        .iter()
        .filter(|c| !declares_pins(c) && !listed.contains(*c) && !declared_only(c, &chip_doc(c)))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "these chips declare no `pins:` and are not recorded as parse-fallback or declared-only: {unaccounted:?}.\n\
         Either transcribe the pin map from the datasheet (preferred — the parse is a guess),\n\
         or add the name to PARSE_FALLBACK_CHIPS with the rest."
    );
}

#[test]
fn declared_only_does_not_claim_pins_or_hide_a_new_model() {
    let doc = chip_doc("rv1103");
    assert!(shippable_chips().contains("rv1103"));
    assert!(!PARSE_FALLBACK_CHIPS.contains(&"rv1103"));
    assert!(
        declared_only("rv1103", &doc),
        "RV1103 has gained pins/peripherals or changed identity; review its declared-only classification"
    );
    assert!(!declared_only("another-cortex-a7", &doc));
    for (field, value) in [
        ("name", "another-cortex-a7"),
        ("arch", "xtensa"),
        ("core", "cortex-m3"),
    ] {
        let mut changed = doc.clone();
        changed[field] = serde_yaml::Value::String(value.into());
        assert!(!declared_only("rv1103", &changed), "changed {field}");
    }
    let mut missing_core = doc.clone();
    missing_core["core"] = serde_yaml::Value::Null;
    assert!(!declared_only("rv1103", &missing_core));

    let mut modeled_gpio = doc.clone();
    modeled_gpio["peripherals"] =
        serde_yaml::from_str("[{ name: gpio1, base_address: 0xff538000, size: 4096, kind: gpio }]")
            .unwrap();
    assert!(!declared_only("rv1103", &modeled_gpio));

    let mut mapped_pin = doc.clone();
    mapped_pin["pins"] = serde_yaml::from_str("{ GPIO1_B2: { gpio: gpio1, bit: 10 } }").unwrap();
    assert!(!declared_only("rv1103", &mapped_pin));

    assert!(!declared_only("stm32f103", &chip_doc("stm32f103")));
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
