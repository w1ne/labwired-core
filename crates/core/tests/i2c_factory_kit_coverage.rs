// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Gate: every `build_i2c_device` match arm must either be a PeripheralKit
//! (universal attach) or an explicit factory-only allowlist entry.
//!
//! New product sensors must be kits. Factory-only is reserved for topology
//! helpers (I²C mux) and cosim test fixtures (`shm_i2c`) — not for “we forgot
//! to register a kit”.

use labwired_core::peripherals::kit::registry;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// Types allowed to exist only in `i2c_factory` without a kit.
/// Shrink this list; never grow it without a design note.
const FACTORY_ONLY_ALLOWLIST: &[&str] = &[
    // Bus switch: attach needs the full manifest to assemble children; kit
    // AttachCtx does not carry the manifest yet (see universal-modules-design).
    "tca9548a",
    "pca9548a",
    "tca9548",
    // Shared-memory cosim fixture — not a product part.
    "shm_i2c",
];

fn workspace_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.parent().and_then(|p| p.parent()).unwrap().to_path_buf()
}

/// Parse `"type"` and `"a" | "b"` arms from `build_i2c_device`'s match.
fn factory_type_literals() -> Vec<String> {
    let path = workspace_root().join("crates/core/src/peripherals/components/i2c_factory.rs");
    let src = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    // Restrict to the build_i2c_device function body roughly.
    let start = src
        .find("pub fn build_i2c_device")
        .expect("build_i2c_device not found");
    let body = &src[start..];
    let end = body.find("\n#[cfg(test)]").unwrap_or(body.len());
    let body = &body[..end];

    let mut out = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        // "foo" =>
        if let Some(rest) = t.strip_prefix('"') {
            if let Some(endq) = rest.find('"') {
                let name = &rest[..endq];
                let after = rest[endq + 1..].trim_start();
                if after.starts_with("=>") || after.starts_with('|') {
                    out.push(name.to_string());
                }
            }
        }
        // multi: "a" | "b" | "c" =>  — collect all quoted on the line before =>
        if t.contains('|') && t.contains("=>") {
            for part in t.split('|') {
                let part = part.trim();
                if let Some(inner) = part.strip_prefix('"').and_then(|s| {
                    let e = s.find('"')?;
                    Some(&s[..e])
                }) {
                    if !inner.is_empty() {
                        out.push(inner.to_string());
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn every_factory_arm_is_kit_or_allowlisted() {
    let factory = factory_type_literals();
    assert!(
        !factory.is_empty(),
        "parsed zero factory arms — regex broke against i2c_factory.rs"
    );

    let allow: HashSet<&str> = FACTORY_ONLY_ALLOWLIST.iter().copied().collect();
    let mut missing = Vec::new();
    for ty in &factory {
        if allow.contains(ty.as_str()) {
            continue;
        }
        if registry::lookup(ty).is_some() {
            continue;
        }
        // Declarative-only devices built via factory helper still count as
        // "resolved" if the embedded YAML exists — but product path is kits.
        // Fail closed: must be a kit.
        missing.push(ty.clone());
    }

    assert!(
        missing.is_empty(),
        "i2c_factory arms with no PeripheralKit (add a kit, or an allowlist entry with a design note):\n  - {}",
        missing.join("\n  - ")
    );
}

#[test]
fn allowlist_entries_are_not_also_kits() {
    // If something is a kit, drop it from the allowlist — dual homes are the bug.
    for ty in FACTORY_ONLY_ALLOWLIST {
        assert!(
            registry::lookup(ty).is_none(),
            "factory-only allowlist entry '{ty}' is also a kit — remove it from FACTORY_ONLY_ALLOWLIST"
        );
    }
}

#[test]
fn migrated_smart_ring_sensors_are_kits() {
    for ty in ["bmi270", "max30102", "tmp117", "drv2605", "fxos8700", "cap1188", "mlx90640"] {
        assert!(
            registry::lookup(ty).is_some(),
            "expected kit for '{ty}' after legacy→kit migration"
        );
    }
    assert!(
        registry::lookup("drv2605l").is_some(),
        "drv2605l alias should resolve via TYPE_ALIASES"
    );
}
