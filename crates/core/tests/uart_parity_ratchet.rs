// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! UART parity ratchet over the onboarding chip library.
//!
//! Every onboarding chip is built through `SystemBus::from_config`. A chip may
//! legitimately fail for reasons unrelated to UARTs (e.g. an odd memory size the
//! parser rejects) — those are ignored here. But if a chip fails specifically
//! because its UART type has no register layout modelled yet, the offending type
//! MUST be on the shrink-only allowlist below. That keeps two invariants:
//!
//!   * No UART is ever silently mismodelled — an unrecognised type errors.
//!   * The set of *not-yet-modelled* UARTs is visible and can only shrink: add a
//!     faithful layout (in `peripherals/uart.rs` + `uart_layout_for`) and the
//!     type disappears from the failures, so its allowlist entry becomes stale
//!     and the staleness check fails until it is removed.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// UART types with no faithful register layout yet. Shrink-only: model one and
/// delete it here. Currently EMPTY — every UART type in the onboarding library
/// has a layout. A new unmodelled type makes `onboarding_uart_parity_ratchet`
/// fail until it is modelled (or, deliberately, added here).
const UNMODELLED_UART_TYPES: &[&str] = &[];

fn dummy_manifest(path: &str) -> SystemManifest {
    SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".into(),
        name: "uart-parity".into(),
        chip: path.into(),
        external_devices: vec![],
        cosim_models: Vec::new(),
        board_io: vec![],
        peripherals: vec![],
        memory_overrides: Default::default(),
        debug_uart: None,
    }
}

#[test]
fn onboarding_uart_parity_ratchet() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/onboarding");

    let mut seen_unmodelled: BTreeSet<String> = BTreeSet::new();
    let mut unexpected: Vec<String> = Vec::new();

    for entry in std::fs::read_dir(&dir).expect("configs/chips/onboarding") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        let abs = path.to_string_lossy().to_string();
        let Ok(chip) = ChipDescriptor::from_file(&path) else {
            continue;
        };
        let Err(err) = SystemBus::from_config(&chip, &dummy_manifest(&abs)) else {
            continue; // built fine
        };
        let msg = err.to_string();
        // Only UART-layout failures are this test's concern.
        if !msg.contains("no register layout modelled") {
            continue;
        }
        // The message is: UART type '<type>' (peripheral ...) ...
        let ty = msg.split('\'').nth(1).unwrap_or("<unknown>").to_string();
        if UNMODELLED_UART_TYPES.contains(&ty.as_str()) {
            seen_unmodelled.insert(ty);
        } else {
            unexpected.push(format!(
                "{}: UART type '{ty}' is gated but not on the allowlist — model it or add it",
                path.file_name().unwrap().to_string_lossy()
            ));
        }
    }

    assert!(
        unexpected.is_empty(),
        "a UART type is gated without a model and without an allowlist entry \
         (it must not silently mismodel):\n{}",
        unexpected.join("\n")
    );

    // Staleness: an allowlisted type that no longer appears was modelled — drop it.
    let stale: Vec<&str> = UNMODELLED_UART_TYPES
        .iter()
        .copied()
        .filter(|t| !seen_unmodelled.contains(*t))
        .collect();
    assert!(
        stale.is_empty(),
        "these UART types are allowlisted as unmodelled but no longer appear as \
         failures (now modelled?) — remove them from UNMODELLED_UART_TYPES: {stale:?}"
    );
}
