// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **Declarative-coverage ratchet.**
//!
//! The direction of travel for off-chip devices is YAML: a part described by a
//! `configs/devices/*.yaml` descriptor runs on native and wasm, ports without a
//! rebuild, and is reviewable by someone holding the datasheet. A part written
//! as Rust in `peripherals/components/` is engine code — it ships in every
//! binary, it can only be changed by a Rust programmer, and each one is another
//! thing the declarative engine has to stay bug-compatible with.
//!
//! Two numbers say where that migration stands, and this file pins both:
//!
//! * **the YAML count only goes up** — deleting a descriptor (or quietly
//!   replacing it with Rust) fails here;
//! * **the Rust count only goes down** — adding a hand-written device model
//!   fails here, which is the whole point. A new part is a YAML file unless
//!   there is a reason it cannot be, and "there is a reason" should be an
//!   argument made in a PR, not a file that appears.
//!
//! Neither number is a quality bar on its own. Read together they are the only
//! measurement of whether the YAML device machine is actually replacing engine
//! code or merely accumulating alongside it.

use std::collections::BTreeSet;
use std::path::PathBuf;

// ─── the baseline ──────────────────────────────────────────────────────────

/// Device types modelled as YAML today (`configs/devices/*.yaml`).
///
/// ⚠️ THE YAML COUNT ONLY GOES UP. Raise this when you add a descriptor.
const YAML_DEVICES_BASELINE: usize = 40;

/// Device models still hand-written in Rust
/// (`crates/core/src/peripherals/components/*.rs`, minus [`EXCLUDED`]).
///
/// ⚠️ THE RUST COUNT ONLY GOES DOWN. Lower this when you port one to YAML.
const RUST_DEVICES_BASELINE: usize = 57;

/// Files in `components/` that are NOT a device model, with the reason. Listed
/// here rather than pattern-matched so every exemption is a line someone wrote
/// on purpose — a silent filter is how a ratchet stops counting the thing it
/// was built to count.
const EXCLUDED: &[(&str, &str)] = &[
    ("mod.rs", "the module tree, not a device"),
    (
        "i2c_factory.rs",
        "the factory that BUILDS devices from a descriptor",
    ),
    (
        "mux_fixture.rs",
        "test-only TCA9548A topology shared by the per-controller tests",
    ),
    (
        "seven_seg_font.rs",
        "a glyph table shared by the 7-segment models",
    ),
    (
        "sensirion.rs",
        "the shared Sensirion CRC-8 helper, not a part",
    ),
    (
        "iolink_native.rs",
        "`#![cfg(feature = \"iolink-native\")]` host bridge to a real master, not a simulated device",
    ),
    (
        "veml7700.rs",
        "`#[cfg(test)]` hand-written oracle retained only to prove the YAML VEML7700 byte-identical; the shipping part is the descriptor",
    ),
    (
        "veml7700_parity.rs",
        "`#[cfg(test)]` harness for the oracle above",
    ),
    (
        "rule_machine.rs",
        "the Tier-2 rule ENGINE every declarative part's `rules:` runs on, not a part",
    ),
];

/// The declarative ENGINE itself (`declarative_*.rs`): the primitives every
/// YAML descriptor is interpreted by. Excluded by prefix because the set grows
/// with each new primitive and each addition is a move toward YAML, not away.
const ENGINE_PREFIX: &str = "declarative_";

/// Out-of-line `#[cfg(test)]` modules (`<model>_tests.rs`), which are a model's
/// TESTS and not a model.
///
/// Excluded by suffix rather than by name because they arrive in batches: the
/// inline-test-module split (#1142) created `mcp2515_tests.rs` here in one
/// commit, and each such file counted as a brand-new hand-written device — the
/// ratchet read a pure code move as the migration going backwards. A suffix
/// rule is a pattern and this file's own rule is that a silent filter is how a
/// ratchet stops counting; this one is neither silent (it is printed in the
/// exclusion list with its reason, and the matched files are named) nor able to
/// hide a real device, since `foo_tests.rs` is the compiler-visible name of a
/// test module and never of a part.
const TEST_MODULE_SUFFIX: &str = "_tests.rs";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn yaml_devices() -> BTreeSet<String> {
    let dir = repo_root().join("configs/devices");
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {dir:?}: {e}"))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect()
}

fn rust_devices() -> BTreeSet<String> {
    let dir = repo_root().join("crates/core/src/peripherals/components");
    let excluded: BTreeSet<&str> = EXCLUDED.iter().map(|(f, _)| *f).collect();
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {dir:?}: {e}"))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .filter(|name| {
            !excluded.contains(name.as_str())
                && !name.starts_with(ENGINE_PREFIX)
                && !name.ends_with(TEST_MODULE_SUFFIX)
        })
        .collect()
}

/// The out-of-line test modules the suffix rule dropped, so the exclusion is
/// reported by NAME rather than as a count.
fn excluded_test_modules() -> BTreeSet<String> {
    let dir = repo_root().join("crates/core/src/peripherals/components");
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {dir:?}: {e}"))
        .map(|e| e.expect("dir entry").path())
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(TEST_MODULE_SUFFIX))
        .collect()
}

#[test]
fn declarative_coverage_only_improves() {
    let yaml = yaml_devices();
    let rust = rust_devices();

    println!("declarative coverage:");
    println!(
        "  YAML device descriptors : {:>3}  (baseline {YAML_DEVICES_BASELINE}, only goes UP)",
        yaml.len()
    );
    println!(
        "  hand-written Rust models: {:>3}  (baseline {RUST_DEVICES_BASELINE}, only goes DOWN)",
        rust.len()
    );
    println!("  excluded from the Rust count ({}):", EXCLUDED.len());
    for (file, why) in EXCLUDED {
        println!("    {file:<22} {why}");
    }
    println!(
        "    {ENGINE_PREFIX}*.rs{:<11} the declarative engine itself",
        ""
    );
    println!(
        "    *{TEST_MODULE_SUFFIX:<21} an out-of-line #[cfg(test)] module, not a device: {:?}",
        excluded_test_modules().iter().collect::<Vec<_>>()
    );
    println!("  YAML: {:?}", yaml.iter().collect::<Vec<_>>());

    assert!(
        yaml.len() >= YAML_DEVICES_BASELINE,
        "the YAML device count FELL, {} < {YAML_DEVICES_BASELINE}. A descriptor was \
         deleted or renamed away. If that is deliberate (a part was dropped from the \
         catalog) lower the baseline in the same commit and say why; if a part moved \
         BACK to Rust, that is the direction this ratchet exists to refuse.",
        yaml.len()
    );
    assert!(
        rust.len() <= RUST_DEVICES_BASELINE,
        "the hand-written Rust device count GREW, {} > {RUST_DEVICES_BASELINE}. A new \
         device belongs in configs/devices/*.yaml unless the declarative primitives \
         genuinely cannot express it. If they cannot, raise the baseline in the same \
         commit with the reason — and consider whether the missing primitive is the \
         real change.\\nCurrent Rust models: {:?}",
        rust.len(),
        rust.iter().collect::<Vec<_>>()
    );

    // A baseline that has drifted BELOW the truth silently stops ratcheting:
    // it would accept a regression back up to the stale number. Same for YAML.
    assert_eq!(
        yaml.len(),
        YAML_DEVICES_BASELINE,
        "the YAML count improved to {} — raise YAML_DEVICES_BASELINE to lock it in",
        yaml.len()
    );
    assert_eq!(
        rust.len(),
        RUST_DEVICES_BASELINE,
        "the Rust count improved to {} — lower RUST_DEVICES_BASELINE to lock it in",
        rust.len()
    );
}

/// Every exclusion must name a file that exists. A stale entry is how an
/// exclusion list starts excluding nothing while still reading as deliberate.
#[test]
fn every_exclusion_names_a_real_file() {
    let dir = repo_root().join("crates/core/src/peripherals/components");
    for (file, why) in EXCLUDED {
        assert!(
            dir.join(file).is_file(),
            "EXCLUDED lists {file} ({why}) but no such file exists in {dir:?}"
        );
    }
}
