// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! DEVICE IDENTITY HAS ONE HOME.
//!
//! Why this file exists
//! ====================
//! Seven bugs in this codebase share one shape: a concept with two homes and
//! nothing forcing them to agree. Device binding (controller walk vs typed
//! `Vec`s), display evidence (`inspect` vs the wasm per-panel accessors), the
//! run step budget (two constants), compile inputs (five inferring call sites
//! and one that did not), tick participation (derive vs a hand-asserted flag),
//! and the 3D screen surface (procedural mesh vs catalog GLB). Each was fixed
//! by giving the concept ONE home and adding a guard that fails if a second
//! home appears.
//!
//! *Device identity* is the same shape, not yet broken. Two subsystems answer
//! "which model is this binding talking about?" and they answer it differently:
//!
//!   * `inspect` (`SystemBus::inspect_devices`) resolves a device by MANIFEST
//!     DECLARATION + PLACEMENT — an id stamp, or a controller+address join
//!     against `external_devices`.
//!   * the browser (`crates/wasm/src/inspect.rs`, `crates/wasm/src/inputs.rs`)
//!     resolves the same device by `board_io` BINDING ID + a hardcoded
//!     `device_type` STRING LITERAL.
//!
//! The strings are already demonstrably unreliable.
//! `Simulator::get_uc8151d_framebuffer` gave up on them and says so in its own
//! doc comment — "the board_io binding type may say `ssd1680_tricolor_290` …
//! we ignore that and just find a `Uc8151dTricolor290`" — so two sibling
//! accessors for the same physical panel family use two different identity
//! rules today.
//!
//! What this file gates
//! ====================
//! The engine already owns a device-type vocabulary: [`kit::registry::KITS`]
//! (each kit's `KitMetadata::device_type`) plus the legacy-spelling table
//! `TYPE_ALIASES`, resolved together by [`kit::registry::lookup`]. That is the
//! ONE home. Two properties keep it that way:
//!
//! (1) `every_device_type_the_browser_keys_on_is_known_to_the_registry` — a
//!     literal the browser matches on must resolve through the registry. A
//!     device type renamed in the engine, or a typo in a new accessor, means
//!     the browser's `find` never matches: the panel renders dark, or the
//!     stimulus silently does nothing, and every Rust test still passes because
//!     `crates/wasm` is not even compiled by the core PR lane.
//!
//! (2) `the_browser_does_not_re_implement_the_device_type_alias_table` — the
//!     browser may not spell a legacy alias itself. The alias table is the part
//!     of device identity most likely to fork, because an alias is invisible
//!     until someone authors a manifest with the old spelling. It already HAD
//!     forked: `get_ssd1680_framebuffer` and `get_ssd1680_refresh_generation`
//!     each inlined `Some("ssd1680_tricolor_290") | Some("epd-2in9-tricolor")`,
//!     which is two of the three rows of `TYPE_ALIASES` copied by hand — and
//!     the row they left out, `gxepd2_290_c90c`, is a spelling the engine
//!     accepts and the browser did not. A lab authored that way attached a real
//!     panel, drove it, and rendered dark.
//!
//! Both are derived: the expected vocabulary comes from the registry (live
//! Rust data), the observed vocabulary from parsing the OTHER crate's source.
//! Neither side is a list a person maintains, so neither can drift silently.
//!
//! Why a source scan and not a call
//! ================================
//! `crates/wasm` is a `cdylib` and is NOT a workspace default-member, so the
//! core PR lane (`cargo test --lib`) never builds it. A guard that needed
//! `labwired-wasm` to link would run nowhere on a pull request — the same
//! "parked in an excluded lane" failure the concepts above are about. Reading
//! its source is what makes this gate reachable from the lane that actually
//! blocks a merge. `crates/core/tests/panel_artifact_evidence.rs` derives the
//! panel list from the same files for the same reason.

use crate::peripherals::kit::registry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// `crates/core` → repo root.
fn repo_root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// The browser-facing sources that bind a `board_io` entry to a model.
const WASM_SOURCES: &[&str] = &["crates/wasm/src/inspect.rs", "crates/wasm/src/inputs.rs"];

/// A `device_type` value looks like `oled-ssd1306` / `ssd1680_tricolor_290` —
/// a single bare token. This filter is what keeps the scan from mistaking a
/// neighbouring `format!("No lcd1602 board_io binding '{}'", …)` for an
/// identity literal; the message is prose, a device type never is.
///
/// Deliberately NOT restricted to lowercase, even though every registered type
/// is lowercase today. The point of the gate is to catch a spelling the engine
/// does not know, and `oled-SH1107` is exactly such a spelling — excluding it
/// from the scan would make the gate blind to one of the typos it exists to
/// catch. That was not hypothetical: the first deliberate-break run of this
/// gate stayed green for precisely that reason.
fn looks_like_a_device_type(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 48
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
}

/// Every string literal the file uses to DECIDE whether a `board_io` binding is
/// the device an accessor wants.
///
/// The shapes in use, all within three lines of the `device_type` access:
/// ```ignore
/// b.device_type.as_deref() == Some("lcd1602")
/// matches!(b.device_type.as_deref(), Some("a") | Some("b"))
/// match binding.device_type.as_deref() { Some(t) if t == "adxl345" => t, … }
/// ```
/// Doc comments are skipped: they *describe* the contract rather than enforce
/// it, and a comment naming a type is not a second home.
fn identity_literals(src: &str) -> BTreeSet<String> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = BTreeSet::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        if !line.contains("device_type") {
            continue;
        }
        // The predicate never spans more than the access line plus two.
        for probe in lines.iter().take((i + 3).min(lines.len())).skip(i) {
            if probe.trim_start().starts_with("//") {
                continue;
            }
            for lit in string_literals(probe) {
                if looks_like_a_device_type(&lit) {
                    out.insert(lit);
                }
            }
        }
    }
    out
}

/// Double-quoted literals on one line. `device_type` predicates contain no
/// escapes, so a plain split is exact here.
fn string_literals(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            if j >= bytes.len() {
                break;
            }
            out.push(line[start..j].to_string());
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

fn wasm_identity_literals() -> BTreeMap<&'static str, BTreeSet<String>> {
    let mut per_file = BTreeMap::new();
    for rel in WASM_SOURCES {
        let path = repo_root(rel);
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "cannot read {} ({e}). This gate reads the browser layer's source; \
                 if the file moved, update WASM_SOURCES — do not delete the gate.",
                path.display()
            )
        });
        per_file.insert(*rel, identity_literals(&src));
    }
    per_file
}

/// Every legacy spelling the engine accepts, and the canonical type it means.
/// Derived by asking the registry, not by re-listing `TYPE_ALIASES`: an alias
/// is a `device_type` that `lookup` resolves to a kit whose own
/// `metadata().device_type` is spelled differently.
fn registry_aliases() -> BTreeMap<String, String> {
    let canonical: BTreeSet<String> = registry::kits()
        .iter()
        .map(|k| k.metadata().device_type.to_string())
        .collect();
    let mut aliases = BTreeMap::new();
    for spelling in registry::known_device_types() {
        let Some(kit) = registry::lookup(&spelling) else {
            continue;
        };
        let target = kit.metadata().device_type.to_string();
        if spelling != target && !canonical.contains(&spelling) {
            aliases.insert(spelling, target);
        }
    }
    aliases
}

#[test]
fn every_device_type_the_browser_keys_on_is_known_to_the_registry() {
    let per_file = wasm_identity_literals();
    let all: BTreeSet<&String> = per_file.values().flatten().collect();

    // Anti-vacuity. If the scan stops finding literals the gate proves nothing,
    // and a silent zero is exactly how a green suite covers broken behaviour.
    //
    // The floor was 12 when every display accessor keyed on a `board_io`
    // device_type string. The one-door change deleted those: a display's
    // identity now comes from the model's own artifact, so there is no spelling
    // for the browser to get wrong on that path. Six literals remain, all in
    // sensor accessors (ntc-thermistor, max31855, neo6m-gps) that still resolve
    // by type — they are what this gate now guards. Lowering the floor to match
    // a REMOVED fork is correct; lowering it to silence a scan that broke is
    // not, which is why the number moves in the same commit that removed them.
    assert!(
        all.len() >= 6,
        "the device-type scan found only {} literals across {:?}; it is broken or \
         the browser layer moved. A vacuous gate is worse than none.",
        all.len(),
        WASM_SOURCES
    );

    let unknown: Vec<String> = all
        .iter()
        .filter(|l| registry::lookup(l).is_none())
        .map(|l| (*l).clone())
        .collect();

    assert!(
        unknown.is_empty(),
        "the browser keys on device_type(s) the engine's registry does not know: {unknown:?}\n\
         \n\
         `board_io` bindings carrying these strings will never match, so the panel \n\
         renders dark / the stimulus does nothing, with every Rust test still green \n\
         (crates/wasm is not a default-member; the core PR lane never builds it).\n\
         \n\
         Fix by making the engine and the browser agree on ONE spelling:\n\
           - register the kit in crates/core/src/peripherals/kit/registry.rs, or\n\
           - add the legacy spelling to TYPE_ALIASES there, or\n\
           - correct the literal in the browser layer.\n\
         Do not weaken this gate."
    );
}

#[test]
fn the_browser_does_not_re_implement_the_device_type_alias_table() {
    let aliases = registry_aliases();

    // Anti-vacuity, both halves: the alias table must be non-empty for the gate
    // to mean anything, and the scan must still be finding literals.
    assert!(
        !aliases.is_empty(),
        "no aliases resolved out of the registry — registry::known_device_types() or \
         TYPE_ALIASES changed shape and this gate is now vacuous."
    );
    let per_file = wasm_identity_literals();
    assert!(
        // Same floor move as the sibling, same reason: the one-door change
        // removed the display device_type literals rather than the scan losing
        // them. Keep the two numbers in step.
        per_file.values().flatten().count() >= 6,
        "the device-type scan went vacuous; see the sibling test."
    );

    let mut offenders: Vec<String> = Vec::new();
    for (file, literals) in &per_file {
        for lit in literals {
            if let Some(canonical) = aliases.get(lit) {
                offenders.push(format!("{file}: \"{lit}\" (alias for \"{canonical}\")"));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "the browser layer spells device-type ALIASES itself:\n  {}\n\
         \n\
         That is a second home for the alias table. The first is TYPE_ALIASES in\n\
         crates/core/src/peripherals/kit/registry.rs, and the two have already\n\
         disagreed: the browser inlined two of its three rows and silently dropped\n\
         `gxepd2_290_c90c`, so a lab authored with that spelling attached a real\n\
         panel, drove it, and rendered dark.\n\
         \n\
         Resolve the binding's type through the registry instead of matching the\n\
         alias by hand:\n\
             labwired_core::peripherals::kit::registry::canonical_device_type(t)\n\
                 == \"ssd1680_tricolor_290\"\n\
         Then every alias the engine accepts works in the browser, forever, with\n\
         no second list to maintain.",
        offenders.join("\n  ")
    );
}

#[cfg(test)]
mod scanner_controls {
    //! Positive/negative controls for the parser the gates above depend on.
    //! A scanner that silently found nothing would make both gates pass for
    //! any source at all.
    use super::*;

    #[test]
    fn finds_each_predicate_shape_the_browser_actually_uses() {
        let src = r#"
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("lcd1602"))
            matches!(
                b.device_type.as_deref(),
                Some("oled-ssd1306") | Some("oled-ssd1306-128x32")
            )
            let device_type = match binding.device_type.as_deref() {
                Some(t) if t == "adxl345" || t == "mpu6050" => t,
                _ => continue,
            };
        "#;
        let found = identity_literals(src);
        for want in [
            "lcd1602",
            "oled-ssd1306",
            "oled-ssd1306-128x32",
            "adxl345",
            "mpu6050",
        ] {
            assert!(
                found.contains(want),
                "shape not recognised: {want} in {found:?}"
            );
        }
    }

    #[test]
    fn ignores_prose_and_doc_comments() {
        let src = r#"
            /// `device_id` must match a `board_io` binding with `device_type: "oled-sh1107"`.
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("pcd8544"))
            .ok_or_else(|| {
                JsValue::from_str(&format!("No pcd8544 board_io binding '{}'", device_id))
            })?;
        "#;
        let found = identity_literals(src);
        assert!(
            found.contains("pcd8544"),
            "missed the real predicate: {found:?}"
        );
        assert!(
            !found.contains("oled-sh1107"),
            "a doc comment is not a second home: {found:?}"
        );
        assert!(
            !found.iter().any(|l| l.contains(' ')),
            "prose leaked into the literal set: {found:?}"
        );
    }

    #[test]
    fn the_alias_derivation_actually_finds_the_legacy_spellings() {
        let aliases = registry_aliases();
        assert_eq!(
            aliases.get("epd-2in9-tricolor").map(String::as_str),
            Some("ssd1680_tricolor_290"),
            "alias derivation missed a known TYPE_ALIASES row: {aliases:?}"
        );
        assert!(
            !aliases.contains_key("ssd1680_tricolor_290"),
            "a canonical device_type must not be reported as an alias: {aliases:?}"
        );
    }
}
