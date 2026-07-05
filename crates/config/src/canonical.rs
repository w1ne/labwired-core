// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Canonical device config v1 — the loader skeleton.
//!
//! The canonical config is the ONE JSON shape the agent builds and both engines
//! load directly, collapsing today's split between a visual diagram JSON and a
//! resolved system/chip YAML manifest. It is an unresolved, Wokwi-style wiring
//! graph:
//!
//! ```json
//! {
//!   "version": 1,
//!   "parts": [
//!     { "id": "mcu",  "type": "nucleo-f401re" },
//!     { "id": "oled", "type": "ssd1306", "attrs": { "address": "0x3c" } }
//!   ],
//!   "connections": [ ["mcu:PB9","oled:SDA"], ["mcu:PB8","oled:SCL"] ]
//! }
//! ```
//!
//! The MCU is JUST a part; device config lives on a part's `attrs`, never on a
//! wire. A pin reference is `"partId:pinName"`.
//!
//! Phase 1 (this module) ships parsing + structural validation only. The
//! [`CanonicalConfig::resolve`] method — which lowers this graph into the
//! engine's [`crate::SystemManifest`] + chip descriptor — is a documented stub;
//! see its doc comment. The TS resolver (`diagramToConfig` in
//! `packages/board-config`) is the oracle Phase 2 must match byte-for-byte.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use thiserror::Error;

/// A single part in a canonical config. `attrs` carry device configuration
/// (e.g. an I2C `address`), never wiring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalPart {
    pub id: String,
    /// The catalog part type. `type` is a Rust keyword, hence `r#type` + rename.
    #[serde(rename = "type")]
    pub r#type: String,
    /// Free-form device attributes. Absent → `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Map<String, Value>>,
}

/// The canonical device config, v1. A `connection` is a 2-tuple of pin
/// references; serde deserializes each `["a:b","c:d"]` JSON array into the
/// tuple and rejects any array whose length is not exactly 2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalConfig {
    pub version: u32,
    pub parts: Vec<CanonicalPart>,
    pub connections: Vec<(String, String)>,
}

/// Errors from parsing or structurally validating a canonical config.
#[derive(Debug, Error)]
pub enum CanonicalError {
    /// The JSON did not parse into the canonical shape (bad JSON, unknown keys,
    /// a connection that is not a 2-element array, wrong field types, …).
    #[error("failed to parse canonical config JSON: {0}")]
    Parse(#[from] serde_json::Error),

    /// `version` is not 1.
    #[error("unsupported canonical config version {0} (expected 1)")]
    UnsupportedVersion(u32),

    /// A part `id` cannot be empty.
    #[error("part id cannot be empty")]
    EmptyPartId,

    /// A part `type` cannot be empty.
    #[error("part '{0}' has an empty type")]
    EmptyPartType(String),

    /// Two parts share an `id`.
    #[error("duplicate part id '{0}'")]
    DuplicatePartId(String),

    /// A pin reference is not of the form `partId:pin` (exactly one colon,
    /// non-empty on both sides).
    #[error("malformed pin reference '{0}' (expected 'partId:pin')")]
    MalformedPinRef(String),

    /// A connection endpoint references a part id that is not in `parts`.
    #[error("pin reference '{pin_ref}' references unknown part id '{part_id}'")]
    UnknownPart { pin_ref: String, part_id: String },

    /// [`CanonicalConfig::resolve`] is not implemented yet (Phase 2).
    #[error("canonical config resolution is not yet implemented (Phase 2)")]
    NotYetImplemented,
}

/// Parse a canonical config from JSON. Rejects unknown keys and malformed
/// connections at parse time (via `#[serde(deny_unknown_fields)]` and the tuple
/// deserializer). Does NOT run the semantic structural checks — call
/// [`CanonicalConfig::validate_structure`] for those.
pub fn parse_canonical(json: &str) -> Result<CanonicalConfig, CanonicalError> {
    Ok(serde_json::from_str(json)?)
}

/// Split a pin reference `"partId:pin"` into its two halves, enforcing exactly
/// one colon with non-empty sides. Mirrors the TS validator's `^[^:]+:[^:]+$`.
fn split_pin_ref(pin_ref: &str) -> Result<(&str, &str), CanonicalError> {
    match pin_ref.split_once(':') {
        Some((part, pin)) if !part.is_empty() && !pin.is_empty() && !pin.contains(':') => {
            Ok((part, pin))
        }
        _ => Err(CanonicalError::MalformedPinRef(pin_ref.to_string())),
    }
}

impl CanonicalConfig {
    /// Parse and structurally validate in one step.
    pub fn from_json(json: &str) -> Result<Self, CanonicalError> {
        let cfg = parse_canonical(json)?;
        cfg.validate_structure()?;
        Ok(cfg)
    }

    /// Structural validation: `version == 1`, non-empty & unique part ids,
    /// non-empty part types, every pin reference well-formed, and every
    /// connection endpoint pointing at a known part. This is a PURE shape check
    /// — it does not know whether a part `type` exists in a catalog or whether a
    /// pin exists on a part (those are resolver concerns).
    pub fn validate_structure(&self) -> Result<(), CanonicalError> {
        if self.version != 1 {
            return Err(CanonicalError::UnsupportedVersion(self.version));
        }

        let mut ids: HashSet<&str> = HashSet::new();
        for part in &self.parts {
            if part.id.is_empty() {
                return Err(CanonicalError::EmptyPartId);
            }
            if part.r#type.is_empty() {
                return Err(CanonicalError::EmptyPartType(part.id.clone()));
            }
            if !ids.insert(part.id.as_str()) {
                return Err(CanonicalError::DuplicatePartId(part.id.clone()));
            }
        }

        for (a, b) in &self.connections {
            for pin_ref in [a, b] {
                let (part_id, _pin) = split_pin_ref(pin_ref)?;
                if !ids.contains(part_id) {
                    return Err(CanonicalError::UnknownPart {
                        pin_ref: pin_ref.clone(),
                        part_id: part_id.to_string(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Resolve this canonical config into the engine's runnable configuration.
    ///
    /// # Phase 2 — NOT YET IMPLEMENTED
    ///
    /// This is the port of the TS oracle `diagramToConfig`
    /// (`packages/board-config/src/diagram-to-config.ts`). It must:
    ///   - map each `part.type` to a chip/peripheral model (the MCU part's type
    ///     selects the [`crate::ChipDescriptor`]; peripheral parts become
    ///     [`crate::ExternalDevice`] / [`crate::BoardIoBinding`] entries),
    ///   - infer buses from MCU pin names (e.g. `PB9`/`PB8` → `i2c1`) using the
    ///     chip pin map,
    ///   - bind each peripheral onto the bus it is wired to,
    ///
    /// and produce a [`crate::SystemManifest`] (+ resolved chip) that matches the
    /// TS oracle's `systemYaml` byte-for-byte. Until then it returns
    /// [`CanonicalError::NotYetImplemented`] rather than a partial result.
    pub fn resolve(&self) -> Result<crate::SystemManifest, CanonicalError> {
        // Phase 2: port diagramToConfig — map part.type→chip/peripheral model,
        // infer buses from pin names, bind peripherals; must match the TS oracle.
        Err(CanonicalError::NotYetImplemented)
    }
}

#[cfg(test)]
mod canonical_tests {
    use super::*;

    const EXAMPLE: &str = r#"
    {
      "version": 1,
      "parts": [
        { "id": "mcu",  "type": "nucleo-f401re" },
        { "id": "oled", "type": "ssd1306", "attrs": { "address": "0x3c" } }
      ],
      "connections": [
        ["mcu:PB9","oled:SDA"],
        ["mcu:PB8","oled:SCL"],
        ["mcu:3V3","oled:VCC"],
        ["mcu:GND","oled:GND"]
      ]
    }
    "#;

    #[test]
    fn parses_and_validates_the_example() {
        let cfg = CanonicalConfig::from_json(EXAMPLE).expect("example is valid");
        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.parts.len(), 2);
        assert_eq!(cfg.parts[0].id, "mcu");
        assert_eq!(cfg.parts[0].r#type, "nucleo-f401re");
        assert!(cfg.parts[0].attrs.is_none());
        let attrs = cfg.parts[1].attrs.as_ref().expect("oled has attrs");
        assert_eq!(attrs.get("address").unwrap(), "0x3c");
        assert_eq!(cfg.connections.len(), 4);
        assert_eq!(cfg.connections[0], ("mcu:PB9".into(), "oled:SDA".into()));
    }

    #[test]
    fn roundtrips_through_serde() {
        let cfg = CanonicalConfig::from_json(EXAMPLE).unwrap();
        let json = serde_json::to_string(&cfg).unwrap();
        let back = parse_canonical(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn rejects_unknown_top_level_key() {
        let json = r#"{ "version":1, "parts":[], "connections":[], "board":"x" }"#;
        let err = parse_canonical(json).unwrap_err();
        assert!(matches!(err, CanonicalError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn rejects_unknown_part_key() {
        let json = r#"{ "version":1, "parts":[{"id":"m","type":"mcu","x":0}], "connections":[] }"#;
        let err = parse_canonical(json).unwrap_err();
        assert!(matches!(err, CanonicalError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn rejects_non_2tuple_connection() {
        let json = r#"{ "version":1, "parts":[{"id":"m","type":"mcu"}],
                        "connections":[["m:A","m:B","m:C"]] }"#;
        let err = parse_canonical(json).unwrap_err();
        assert!(matches!(err, CanonicalError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn rejects_bad_version() {
        let json = r#"{ "version":2, "parts":[], "connections":[] }"#;
        let err = CanonicalConfig::from_json(json).unwrap_err();
        assert!(matches!(err, CanonicalError::UnsupportedVersion(2)), "got {err:?}");
    }

    #[test]
    fn rejects_duplicate_part_id() {
        let json = r#"{ "version":1,
            "parts":[{"id":"m","type":"a"},{"id":"m","type":"b"}],
            "connections":[] }"#;
        let err = CanonicalConfig::from_json(json).unwrap_err();
        assert!(matches!(err, CanonicalError::DuplicatePartId(ref id) if id == "m"), "got {err:?}");
    }

    #[test]
    fn rejects_empty_part_type() {
        let json = r#"{ "version":1, "parts":[{"id":"m","type":""}], "connections":[] }"#;
        let err = CanonicalConfig::from_json(json).unwrap_err();
        assert!(matches!(err, CanonicalError::EmptyPartType(ref id) if id == "m"), "got {err:?}");
    }

    #[test]
    fn rejects_malformed_pin_ref() {
        for bad in ["mcuPB9", "mcu:", ":PB9", "a:b:c"] {
            let json = format!(
                r#"{{ "version":1, "parts":[{{"id":"mcu","type":"m"}}],
                     "connections":[["{bad}","mcu:PB8"]] }}"#
            );
            let err = CanonicalConfig::from_json(&json).unwrap_err();
            assert!(
                matches!(err, CanonicalError::MalformedPinRef(_)),
                "{bad}: got {err:?}"
            );
        }
    }

    #[test]
    fn rejects_endpoint_referencing_unknown_part() {
        let json = r#"{ "version":1, "parts":[{"id":"mcu","type":"m"}],
                        "connections":[["mcu:PB9","ghost:SDA"]] }"#;
        let err = CanonicalConfig::from_json(json).unwrap_err();
        assert!(
            matches!(err, CanonicalError::UnknownPart { ref part_id, .. } if part_id == "ghost"),
            "got {err:?}"
        );
    }

    /// The Phase 2 resolver stub: documented, returns `NotYetImplemented`
    /// rather than a partial/incorrect config. When Phase 2 lands, this test
    /// becomes the parity assertion: `resolve()` output's systemYaml must match
    /// the TS oracle snapshot for the same fixture, byte-for-byte.
    #[test]
    #[ignore = "Phase 2: resolve() port of diagramToConfig, asserted against the TS oracle"]
    fn resolve_is_phase_2_stub() {
        let cfg = CanonicalConfig::from_json(EXAMPLE).unwrap();
        let err = cfg.resolve().unwrap_err();
        assert!(matches!(err, CanonicalError::NotYetImplemented));
    }
}
