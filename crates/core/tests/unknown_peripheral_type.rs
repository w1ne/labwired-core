// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! An unknown peripheral `type:` must fail the load.
//!
//! The guarantee under test is a product guarantee, not a code shape: **when a
//! run reports success, something was modelled.** A chip descriptor that names
//! silicon this engine has never implemented used to get a zero-filled stub, so
//! firmware could drive that address, read zeros, and finish green. These tests
//! are written from that guarantee — a chip YAML naming a type nobody
//! implemented must not produce a bus at all — rather than from the match arms
//! in `bus/from_config.rs`. Nothing here enumerates the supported types, so
//! adding or removing a factory arm cannot make these tests pass vacuously.
//!
//! The measured exception list lives in `bus/known_stubs.rs`; the ratchet below
//! keeps it honest and shrinking.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::known_stubs::KNOWN_STUBBED_PERIPHERAL_TYPES;
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn manifest_for(chip_path: &str) -> SystemManifest {
    SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".into(),
        name: "unknown-peripheral-type".into(),
        chip: chip_path.into(),
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        peripherals: vec![],
        memory_overrides: Default::default(),
        debug_uart: None,
        wifi_ap: None,
    }
}

/// A minimal, self-contained chip that wires exactly one peripheral of
/// `periph_type`. Nothing else in the descriptor can fail, so whatever the
/// loader does is a verdict on that one type.
fn one_peripheral_chip(periph_type: &str) -> ChipDescriptor {
    let yaml = format!(
        r#"
name: "guard-probe"
arch: "arm"
core: "cortex-m3"
flash: {{ base: 0x08000000, size: "16KB" }}
ram: {{ base: 0x20000000, size: "8KB" }}
peripherals:
  - id: "probe0"
    type: "{periph_type}"
    base_address: 0x40000000
    size: "1KB"
"#
    );
    serde_yaml::from_str(&yaml).expect("probe chip parses")
}

/// THE guarantee. A `type:` no one implemented must stop the load dead.
///
/// The name below is deliberately not a near-miss of anything real: it contains
/// no substring the type canonicalizer keys on, so it cannot be quietly
/// resolved to some other model and it is not on the allowlist.
#[test]
fn a_peripheral_type_this_engine_does_not_implement_fails_the_load() {
    let chip = one_peripheral_chip("acme_widget_9000");
    let Err(err) = SystemBus::from_config(&chip, &manifest_for("configs/chips/acme.yaml")) else {
        panic!("an unimplemented peripheral type must not produce a bus");
    };
    let msg = format!("{err:#}");

    // The operator has to be able to act on this without reading the source, so
    // the message must name what, where, and in which file.
    assert!(
        msg.contains("acme_widget_9000"),
        "error must name the offending type: {msg}"
    );
    assert!(
        msg.contains("probe0"),
        "error must name the offending peripheral: {msg}"
    );
    assert!(
        msg.contains("configs/chips/acme.yaml"),
        "error must name the chip file: {msg}"
    );
}

/// The same guarantee stated as the failure it prevents: there must be no bus
/// on which that address answers a read at all. If the loader ever went back to
/// stubbing, this is the observation that would catch it — a successful read of
/// 0 from silicon nobody modelled.
#[test]
fn an_unimplemented_type_never_yields_a_bus_that_answers_zeros() {
    let chip = one_peripheral_chip("acme_widget_9000");
    match SystemBus::from_config(&chip, &manifest_for("configs/chips/acme.yaml")) {
        Err(_) => {}
        Ok(bus) => panic!(
            "load succeeded for an unimplemented peripheral type; a read at its base \
             returned {:?} — a green run over silicon that was never modelled",
            bus.read_u32(0x4000_0000)
        ),
    }
}

/// The supported escape hatch stays open: a chip that *means* "this window
/// decodes and reads zero" says so with `type: stub`, and that still loads.
/// Without this, the guard above would have no honest way to express a
/// deliberately unmodelled window and authors would be pushed back into
/// inventing type names.
#[test]
fn an_explicitly_declared_stub_still_loads() {
    let chip = one_peripheral_chip("stub");
    let bus = SystemBus::from_config(&chip, &manifest_for("configs/chips/probe.yaml"))
        .expect("an explicit `type: stub` is a declaration, not an omission");
    assert_eq!(
        bus.read_u32(0x4000_0000).ok(),
        Some(0),
        "an explicit stub answers zeros — that is what it declares"
    );
}

fn chip_descriptors() -> Vec<PathBuf> {
    let root = repo_root();
    let mut out = Vec::new();
    for dir in [
        root.join("configs/chips"),
        root.join("configs/chips/onboarding"),
    ] {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            panic!("missing chip directory {}", dir.display());
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("yaml") {
                out.push(p);
            }
        }
    }
    out.sort();
    assert!(
        out.len() > 200,
        "expected the whole shipped chip library, found {} descriptors — the scan is \
         looking in the wrong place and every assertion over it would be vacuous",
        out.len()
    );
    out
}

fn system_manifests() -> Vec<PathBuf> {
    let root = repo_root();
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = ["configs/systems", "examples", "validation"]
        .iter()
        .map(|d| root.join(d))
        .collect();
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|s| s.to_str()) == Some("yaml") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// No lab may break. Every chip descriptor and every shipped system manifest
/// that the engine can load must keep loading: failing for some other,
/// pre-existing reason is out of scope here, but failing *because of this
/// guard* is a lab that used to run and no longer does.
#[test]
fn no_shipped_config_is_rejected_by_the_unknown_type_guard() {
    let mut broken: Vec<String> = Vec::new();

    for path in chip_descriptors() {
        let abs = path.to_string_lossy().to_string();
        let Ok(chip) = ChipDescriptor::from_file(&path) else {
            continue; // pre-existing parse failure, not this guard's doing
        };
        if let Err(e) = SystemBus::from_config(&chip, &manifest_for(&abs)) {
            let msg = format!("{e:#}");
            if msg.contains("unknown peripheral type") {
                broken.push(format!("{}: {msg}", path.display()));
            }
        }
    }

    for path in system_manifests() {
        let Ok(mut manifest) = SystemManifest::from_file(&path) else {
            continue;
        };
        if manifest.chip.is_empty() {
            continue;
        }
        let anchored = path.parent().unwrap_or(Path::new(".")).join(&manifest.chip);
        manifest.chip = anchored.to_string_lossy().to_string();
        let Ok(chip) = ChipDescriptor::from_file(&anchored) else {
            continue;
        };
        if let Err(e) = SystemBus::from_config(&chip, &manifest) {
            let msg = format!("{e:#}");
            if msg.contains("unknown peripheral type") {
                broken.push(format!("{}: {msg}", path.display()));
            }
        }
    }

    assert!(
        broken.is_empty(),
        "the unknown-type guard rejects {} shipped config(s) that loaded before. \
         Do NOT weaken the guard: either the type deserves a model, or it belongs in \
         KNOWN_STUBBED_PERIPHERAL_TYPES with a written reason.\n{}",
        broken.len(),
        broken.join("\n")
    );
}

/// Shrink-only. An allowlist entry is debt; when the last chip that names the
/// type stops naming it (because it got modelled, or the descriptor changed),
/// the entry is dead and must go. Without this the list would quietly keep
/// growing and stop describing anything real.
///
/// Membership is checked against the descriptors' *declared* type strings
/// rather than against the loader's canonicalized form on purpose: if
/// canonicalization ever renames one of these, the entry reads as stale and
/// this test fails — the safe direction, since the alternative is an entry that
/// silently covers a type nobody can see in a YAML.
#[test]
fn known_stub_allowlist_has_no_stale_entries() {
    let mut declared: BTreeSet<String> = BTreeSet::new();
    for path in chip_descriptors() {
        let Ok(chip) = ChipDescriptor::from_file(&path) else {
            continue;
        };
        for p in &chip.peripherals {
            declared.insert(p.r#type.to_ascii_lowercase());
        }
    }

    let stale: Vec<&str> = KNOWN_STUBBED_PERIPHERAL_TYPES
        .iter()
        .map(|(t, _)| *t)
        .filter(|t| !declared.contains(*t))
        .collect();

    assert!(
        stale.is_empty(),
        "{} known-stub allowlist entr(ies) are no longer named by any shipped chip \
         descriptor. Delete them — this list may only shrink: {stale:?}",
        stale.len()
    );
}

/// Every entry must actually carry a reason, and the table must be sorted and
/// free of duplicates so a second entry for the same type cannot hide behind
/// the first.
#[test]
fn known_stub_allowlist_is_sorted_unique_and_reasoned() {
    let mut prev: Option<&str> = None;
    for (t, reason) in KNOWN_STUBBED_PERIPHERAL_TYPES {
        assert!(
            !reason.trim().is_empty(),
            "allowlist entry '{t}' has no written reason"
        );
        assert!(
            reason.len() > 30,
            "allowlist entry '{t}' has a placeholder reason: {reason:?}"
        );
        assert_eq!(
            *t,
            t.to_ascii_lowercase(),
            "allowlist keys are canonical (lowercase) type strings"
        );
        if let Some(p) = prev {
            assert!(
                p < *t,
                "allowlist must be sorted and unique; '{p}' precedes '{t}'"
            );
        }
        prev = Some(t);
    }
}
