// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Factory: build a read-only [`SpiDevice`] from a system-manifest
//! `external_devices` entry's `type:` string + `config:` map.
//!
//! Only the declarative `type: ir` path lives here — it is the SPI sibling of
//! [`build_i2c_device`](super::build_i2c_device)'s `ir` arm. Hand-written SPI
//! devices (MAX31855, displays, shift registers) attach through the
//! [`PeripheralKit`](crate::peripherals::kit) registry instead, so this factory
//! returns `None` for every non-`ir` type and the bus loader falls through to
//! the kit pass. That keeps the two dispatch paths non-overlapping (no
//! double-attach).

use std::collections::HashMap;

use crate::peripherals::spi::SpiDevice;

/// Build a read-only SPI device for a manifest `external_devices` entry.
///
/// Returns `None` for any type other than `ir` (handled by the kit registry),
/// or when the `ir` spec is missing, unreadable, or invalid — a yaml typo then
/// surfaces as a skipped attach + warning rather than a silent empty bus.
pub fn build_spi_device(
    type_str: &str,
    config: &HashMap<String, serde_yaml::Value>,
) -> Option<Box<dyn SpiDevice>> {
    match type_str.to_ascii_lowercase().as_str() {
        "ir" => {
            let spec_path = match config.get("spec_path").and_then(|v| v.as_str()) {
                Some(p) => p,
                None => {
                    eprintln!("ir spi component: missing required 'spec_path' in config");
                    return None;
                }
            };
            let yaml = match std::fs::read_to_string(spec_path) {
                Ok(y) => y,
                Err(e) => {
                    eprintln!("ir spi component: cannot read {spec_path}: {e}");
                    return None;
                }
            };
            let spec: labwired_ir::component::IrComponent = match serde_yaml::from_str(&yaml) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("ir spi component: {spec_path} parse error: {e}");
                    return None;
                }
            };
            let cs_override = config
                .get("cs_pin")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            match crate::peripherals::components::IrSpiComponent::new(spec, cs_override) {
                Ok(d) => Some(Box::new(d)),
                Err(e) => {
                    eprintln!("ir spi component: {spec_path} invalid: {e}");
                    None
                }
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn max31855_spec_path() -> String {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../configs/components/max31855.yaml"
        )
        .to_string()
    }

    #[test]
    fn non_ir_types_fall_through_to_kit_registry() {
        // The kit-registered built-ins must NOT be built here, or the bus would
        // attach them twice.
        let cfg = HashMap::new();
        assert!(build_spi_device("max31855", &cfg).is_none());
        assert!(build_spi_device("ili9341", &cfg).is_none());
    }

    #[test]
    fn ir_builds_from_spec_path_with_default_cs() {
        let mut cfg = HashMap::new();
        cfg.insert(
            "spec_path".to_string(),
            serde_yaml::Value::String(max31855_spec_path()),
        );
        let dev = build_spi_device("ir", &cfg).expect("ir spi device should build");
        assert_eq!(dev.cs_pin(), "PA4");
    }

    #[test]
    fn ir_cs_pin_override_from_config() {
        let mut cfg = HashMap::new();
        cfg.insert(
            "spec_path".to_string(),
            serde_yaml::Value::String(max31855_spec_path()),
        );
        cfg.insert(
            "cs_pin".to_string(),
            serde_yaml::Value::String("PB12".to_string()),
        );
        let dev = build_spi_device("ir", &cfg).expect("ir spi device should build");
        assert_eq!(dev.cs_pin(), "PB12");
    }

    #[test]
    fn ir_missing_or_bad_spec_returns_none() {
        assert!(build_spi_device("ir", &HashMap::new()).is_none());
        let mut cfg = HashMap::new();
        cfg.insert(
            "spec_path".to_string(),
            serde_yaml::Value::String("/nonexistent/spec.yaml".to_string()),
        );
        assert!(build_spi_device("ir", &cfg).is_none());
    }
}
