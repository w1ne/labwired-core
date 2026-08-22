// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every TIMER, USART and I2C instance this part HAS is declared, decodes, and
//! carries the right per-instance facts.
//!
//! Eight timers, one USART and two I2Cs were absent from the descriptor on the
//! reasoning that an undeclared window faults loudly where a stub would answer
//! zero. That is right for a peripheral with no model and wrong for one whose
//! model is already in the tree: `GPIO_TIMERROUTE` routes all ten timers, and a
//! sketch reaching for a second PWM timer hit a bus fault instead of the model
//! sitting next to the two that worked.
//!
//! ⚠️ Two per-instance facts on this family are NOT guessable, and both are
//! asserted here rather than trusted:
//!
//! 1. `TIMER_CNTWIDTH` — the 32-bit instances are 0, 1, 8, 9, NOT 0..3. The
//!    datasheet says "4x 32-bit" without saying which. A 16-bit timer declared
//!    32-bit gives firmware a `micros()` that never wraps.
//! 2. The CLKEN bit does not follow the instance number. TIMER0..4 are CLKEN0
//!    bits 4..8 and TIMER5..9 are CLKEN2 bits 0..4; I2C0/1 are CLKEN0 14/15
//!    while I2C2/3 are CLKEN2 9/10.

use labwired_config::ChipDescriptor;
use std::path::PathBuf;

fn descriptor() -> ChipDescriptor {
    let abs = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("configs/chips/efr32mg26.yaml");
    ChipDescriptor::from_file(&abs).expect("load the efr32mg26 descriptor")
}

/// `(id, base, clock register, clock bit)` for every instance the silicon has,
/// from `efr32mg26b510f3200im48.h` and `efr32mg26_cmu.h`.
const EXPECTED: &[(&str, u64, &str, u8)] = &[
    ("timer0", 0x4004_8000, "clken0", 4),
    ("timer1", 0x4004_C000, "clken0", 5),
    ("timer2", 0x4005_0000, "clken0", 6),
    ("timer3", 0x4005_4000, "clken0", 7),
    ("timer4", 0x4005_8000, "clken0", 8),
    ("timer5", 0x4005_C000, "clken2", 0),
    ("timer6", 0x4006_0000, "clken2", 1),
    ("timer7", 0x4006_4000, "clken2", 2),
    ("timer8", 0x4006_8000, "clken2", 3),
    ("timer9", 0x4006_C000, "clken2", 4),
    // ⚠️ USART0 is declared as `spi0`, not `usart0`. Series 2 has NO separate
    // SPI peripheral — SPI is a USART with `CTRL.SYNC` — so an instance is one
    // or the other and never both, and this chip gives USART0 to SPI while
    // USART1 is the VCOM console.
    ("spi0", 0x400A_0000, "clken0", 9),
    ("usart1", 0x400A_4000, "clken2", 7),
    ("usart2", 0x400A_8000, "clken2", 8),
    ("i2c0", 0x4B00_0000, "clken0", 14),
    ("i2c1", 0x400B_0000, "clken0", 15),
    ("i2c2", 0x400B_4000, "clken2", 9),
    ("i2c3", 0x400B_8000, "clken2", 10),
];

/// `TIMER_CNTWIDTH` per instance. The 32-bit ones are 0, 1, 8, 9.
const COUNTER_BITS: [u64; 10] = [32, 32, 16, 16, 16, 16, 16, 16, 32, 32];

#[test]
fn every_instance_is_declared_at_its_silicon_base_and_clock_bit() {
    let chip = descriptor();
    for &(id, base, reg, bit) in EXPECTED {
        let p = chip
            .peripherals
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("{id} is not declared"));
        assert_eq!(p.base_address, base, "{id} base");
        let gates = p
            .clock
            .as_ref()
            .unwrap_or_else(|| panic!("{id} declares no clock gate — it would answer while gated"));
        // These instances each require exactly ONE bit; a second would mean the
        // model answers only when both are set, which is not this silicon.
        let [gate] = gates.as_slice() else {
            panic!(
                "{id} declares {} clock bits, expected 1",
                gates.as_slice().len()
            );
        };
        assert_eq!(gate.reg, reg, "{id} clock register");
        assert_eq!(gate.bit, bit, "{id} clock bit");
    }
}

#[test]
fn every_timer_declares_the_counter_width_its_instance_actually_has() {
    let chip = descriptor();
    for (i, &bits) in COUNTER_BITS.iter().enumerate() {
        let id = format!("timer{i}");
        let p = chip
            .peripherals
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("{id} is not declared"));
        let declared = p
            .config
            .get("counter_bits")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| panic!("{id} declares no counter_bits; the model requires the key"));
        assert_eq!(
            declared, bits,
            "{id} counter_bits: the 32-bit instances are 0, 1, 8, 9 — not 0..3",
        );
    }
}

/// ⚠️ The guard on this file: a table nobody compares against the tree is a
/// list of intentions. If the descriptor grows an instance these tables do not
/// name, this fails rather than silently covering less than it claims.
#[test]
fn the_expected_table_covers_every_declared_instance_of_these_kinds() {
    let chip = descriptor();
    let declared: Vec<&str> = chip
        .peripherals
        .iter()
        .map(|p| p.id.as_str())
        .filter(|id| {
            (id.starts_with("timer")
                || id.starts_with("usart")
                || id.starts_with("spi")
                || id.starts_with("i2c"))
                && id.chars().last().is_some_and(|c| c.is_ascii_digit())
        })
        .collect();
    let expected: Vec<&str> = EXPECTED.iter().map(|&(id, ..)| id).collect();
    let missing: Vec<&&str> = declared
        .iter()
        .filter(|id| !expected.contains(id))
        .collect();
    assert!(
        missing.is_empty(),
        "declared but not in this file's table: {missing:?}",
    );
    assert_eq!(
        declared.len(),
        EXPECTED.len(),
        "the tree declares {} of these instances and the table names {}",
        declared.len(),
        EXPECTED.len(),
    );
}
