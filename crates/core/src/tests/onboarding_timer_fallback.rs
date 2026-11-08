// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! No Renode-imported chip may reach the STM32 timer by accident.
//!
//! `canonical_peripheral_type` ends in a fuzzy `contains("timer")` rule that
//! returns `timer`, and the `timer` factory arm builds the STM32 TIMx model
//! with no profile check and no warning. Any imported vendor type that is not
//! a canonical model type therefore became an STM32 timer at its window:
//! `litex_timer_csr32` on LiteX Ibex did, and its smoke passed only because an
//! `ev_pending` write landed in STM32 `CCMR1` and read back non-zero.
//!
//! The UART, SPI, I2C, ADC and RCC bins refuse non-STM32 silicon without a
//! profile, and the GPIO bin warns. The timer bin did neither, so this guard
//! closes it for `configs/chips/onboarding/`: a type there that canonicalises
//! to `timer` must be the literal `timer` or a Nordic type (the `timer` arm
//! routes `nrf*` to `Nrf52Timer`). Anything else names a real model type, or
//! is declared `type: stub` so the scoreboard counts it as a zero window.

use crate::bus::SystemBus;
use labwired_config::ChipDescriptor;
use std::path::PathBuf;

fn onboarding_chips() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/onboarding");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    paths.sort();
    paths
}

#[test]
fn onboarding_chip_types_do_not_fall_through_to_the_stm32_timer() {
    let chips = onboarding_chips();
    assert!(
        chips.len() > 200,
        "expected the imported Renode set, found {} chips",
        chips.len()
    );

    let mut offenders = Vec::new();
    for path in &chips {
        let chip = ChipDescriptor::from_file(path)
            .unwrap_or_else(|e| panic!("{} must parse: {e}", path.display()));
        for p in &chip.peripherals {
            let raw = p.r#type.to_ascii_lowercase();
            if SystemBus::canonical_peripheral_type(&raw) == "timer"
                && raw != "timer"
                && !raw.contains("nrf")
            {
                offenders.push(format!(
                    "{}: {} (type `{}`)",
                    path.file_name().unwrap().to_string_lossy(),
                    p.id,
                    p.r#type
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these imported peripherals resolve to the STM32 timer only through the fuzzy \
         `contains(\"timer\")` rule. Point them at a real model type, or declare them \
         `type: stub` with `renode_type:` so they count as zero windows:\n  {}",
        offenders.join("\n  ")
    );
}
