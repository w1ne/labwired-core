// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Canonical JSON form of the [`super::registry::kits`] slice.
//!
//! The browser/playground, the docs and the staleness gate all consume this
//! one serialisation. The generator binary and the gate test call [`json`],
//! so the writer and the checker cannot disagree about ordering or
//! formatting. Before this module existed they built the JSON separately,
//! and a change to the kit registry could make the gate stale in a way the
//! generator did not reproduce.

use serde::Serialize;

use super::registry;
use super::KitMetadata;

#[derive(Serialize)]
struct Manifest<'a> {
    schema_version: u32,
    peripherals: Vec<&'a KitMetadata>,
}

/// The current manifest schema version. Bumped when the JSON shape changes;
/// the TS reader pins to a major.
pub const SCHEMA_VERSION: u32 = 2;

/// Registry metadata sorted by `device_type`.
///
/// The manifest is sorted rather than in registration order. The UI reader
/// and the parent repository compare the file as text, so alphabetical order
/// makes a new kit a one-line insertion instead of a full-file rewrite. It
/// also keeps the order independent of where a kit sits in `KITS`.
pub fn sorted_metadata() -> Vec<&'static KitMetadata> {
    let mut peripherals: Vec<_> = registry::kits().iter().map(|kit| kit.metadata()).collect();
    peripherals.sort_by(|left, right| left.device_type.cmp(&right.device_type));
    peripherals
}

/// The exact manifest bytes: pretty JSON plus a trailing newline.
pub fn json() -> String {
    let manifest = Manifest {
        schema_version: SCHEMA_VERSION,
        peripherals: sorted_metadata(),
    };
    let mut json =
        serde_json::to_string_pretty(&manifest).expect("peripheral kit metadata always serialises");
    if !json.ends_with('\n') {
        json.push('\n');
    }
    json
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_sorted_by_device_type() {
        let metadata = sorted_metadata();
        assert_eq!(metadata.len(), registry::kits().len());
        let types: Vec<_> = metadata
            .iter()
            .map(|kit| kit.device_type.as_ref())
            .collect();
        let mut sorted = types.clone();
        sorted.sort_unstable();
        assert_eq!(types, sorted, "manifest must be sorted by device_type");
        let mut unique = types.clone();
        unique.dedup();
        assert_eq!(
            unique.len(),
            types.len(),
            "device_type values must be unique"
        );
    }

    #[test]
    fn manifest_json_is_byte_stable() {
        let first = json();
        let second = json();
        assert_eq!(first, second);
        assert!(first.ends_with('\n'), "manifest must end with a newline");
        let value: serde_json::Value = serde_json::from_str(&first).unwrap();
        assert_eq!(value["schema_version"], SCHEMA_VERSION);
        assert_eq!(
            value["peripherals"].as_array().unwrap().len(),
            registry::kits().len()
        );
    }
}
