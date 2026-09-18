// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Every chip in `configs/chips/*.yaml` either builds a machine through
//! [`Session::from_chip_name`] with a committed ELF of its own architecture,
//! or is named in `NEEDS` with a reason. Nothing is silently skipped: this
//! test asserts BOTH that every non-`NEEDS` chip builds and that every
//! `NEEDS` chip still fails to build, so the list cannot go stale and hide a
//! real regression.

mod common;

use labwired_core::session::catalog::Catalog;
use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::FirmwareSource;

/// Chips that cannot build a machine from a committed ELF fixture alone, and
/// why. Every entry here must still fail today; anything not listed must
/// succeed.
const NEEDS: &[(&str, &str)] = &[(
    "ci-fixture-unknown-arch",
    "declares no known architecture (arch: unknown)",
)];

#[test]
fn every_catalog_chip_builds_or_is_listed_in_needs() {
    let cat = Catalog::discover();
    let names = cat.chip_names();
    assert!(
        names.iter().any(|n| n == "stm32f401"),
        "catalog lists stm32f401; got {names:?}"
    );
    assert_eq!(
        names.len(),
        38,
        "expected 38 chips under configs/chips/*.yaml; got {names:?}. If a chip \
         was added or removed, update this test's expectation and its NEEDS list."
    );

    let arm_fw = common::committed("tests/fixtures/uart-ok-thumbv7m.elf");
    let riscv_fw = common::committed("tests/fixtures/riscv-ci-fixture.elf");
    let avr_fw = common::committed("tests/fixtures/avr/arduino-nano-blinky.elf");
    // Committed Tier-1 Xtensa fixtures (same ones session_builder_xtensa.rs
    // uses): esp32.elf for the classic dual-core part, esp32s3.elf for both
    // S3 SKUs (esp32s3 and esp32s3-zero share the same silicon).
    let esp32_fw = common::committed("tests/fixtures/tier1/esp32.elf");
    let esp32s3_fw = common::committed("tests/fixtures/tier1/esp32s3.elf");

    let mut unexpected_failures = vec![];
    let mut unexpected_successes = vec![];

    for name in &names {
        let needs_reason = NEEDS.iter().find(|(n, _)| n == name).map(|(_, r)| *r);

        let (chip, _) = match cat.resolve(name) {
            Ok(x) => x,
            Err(e) => {
                if needs_reason.is_none() {
                    unexpected_failures.push(format!("{name}: resolve failed: {e}"));
                }
                continue;
            }
        };

        let fw: &[u8] = match chip.arch {
            labwired_config::Arch::Arm => &arm_fw,
            labwired_config::Arch::RiscV => &riscv_fw,
            labwired_config::Arch::Avr => &avr_fw,
            labwired_config::Arch::Xtensa => {
                if name == "esp32" {
                    &esp32_fw
                } else {
                    // esp32s3 and esp32s3-zero are the same silicon.
                    &esp32s3_fw
                }
            }
            labwired_config::Arch::Unknown => {
                if needs_reason.is_none() {
                    unexpected_failures.push(format!(
                        "{name}: has unknown architecture but is not listed in NEEDS"
                    ));
                    continue;
                }
                &arm_fw // arch is refused before firmware is even inspected
            }
        };

        let result = Session::from_chip_name(name, FirmwareSource::Elf(fw), OpenOptions::default());
        match (result, needs_reason) {
            (Ok(_), None) => {}
            (Ok(_), Some(reason)) => {
                unexpected_successes.push(format!("{name} (listed NEEDS reason: {reason})"))
            }
            (Err(e), None) => unexpected_failures.push(format!("{name}: {e}")),
            (Err(_), Some(_)) => {}
        }
    }

    assert!(
        unexpected_failures.is_empty(),
        "chips that failed to build and are NOT in NEEDS:\n{}",
        unexpected_failures.join("\n")
    );
    assert!(
        unexpected_successes.is_empty(),
        "chips listed in NEEDS that actually built successfully (stale NEEDS entry, \
         remove it):\n{}",
        unexpected_successes.join("\n")
    );

    // Every NEEDS entry must correspond to a real chip in the catalog.
    for (name, _) in NEEDS {
        assert!(
            names.iter().any(|n| n == name),
            "NEEDS lists '{name}' but it is not a chip in configs/chips/*.yaml"
        );
    }
}
