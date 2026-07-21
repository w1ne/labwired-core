// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Data-driven attach path for the GPIO / pin-timing external-device family.
//!
//! The register-mapped I²C/SPI devices already dispatch through the
//! [`PeripheralKit`](crate::peripherals::kit) registry (no hand-written
//! `from_config` arm each). The GPIO family — rotary encoder, matrix keypad,
//! DHT22, HC-SR04, NeoPixel — historically kept a bespoke `match` arm in
//! [`from_config`](super::from_config) PLUS a hand-mirrored emitter in both the
//! Rust and the TypeScript compiler. That double surface is what this module
//! begins to collapse.
//!
//! Each such device gets ONE [`labwired_config::DeviceDescriptor`] YAML under
//! `configs/devices/`. The descriptor names an irreducible **primitive** (the
//! genuinely un-data-fiable timing algorithm — the Gray-code walk, the matrix
//! reflect, the one-wire frame) and binds its abstract pins to `config:` keys.
//! [`attach`] resolves those bindings and instantiates the primitive; the
//! primitive's Rust model is unchanged, so behavior is byte-identical to the old
//! hand-written arm. Adding a device that reuses an existing primitive is then
//! one YAML file — no new Rust in the attach path.
//!
//! Phase 1 migrates the rotary encoder (`quadrature`). Keypad / DHT22 / HC-SR04
//! follow as their primitives are given descriptors; the emitter unification
//! (both engines reading the descriptor's `emit:` block) is the next step.

use super::SystemBus;
use anyhow::{anyhow, Context, Result};
use labwired_config::{DeviceDescriptor, ExternalDevice};

/// The embedded device descriptors, keyed by their `type:` string. Embedded via
/// `include_str!` so the wasm build (no `std::fs`) resolves them too — the same
/// approach as [`embedded_descriptors`](super::embedded_descriptors) for
/// register peripherals.
fn embedded_yaml(device_type: &str) -> Option<&'static str> {
    match device_type {
        "rotary_encoder" | "rotary-encoder" => Some(include_str!(
            "../../../../configs/devices/rotary_encoder.yaml"
        )),
        "keypad" => Some(include_str!("../../../../configs/devices/keypad.yaml")),
        _ => None,
    }
}

/// Parse the declarative descriptor for `device_type`, if one is embedded.
/// Returns `Ok(None)` when the type is not declarative (the caller then falls
/// through to the legacy hand-written arms).
pub(crate) fn lookup(device_type: &str) -> Result<Option<DeviceDescriptor>> {
    match embedded_yaml(device_type) {
        Some(yaml) => Ok(Some(DeviceDescriptor::from_yaml(yaml).with_context(
            || format!("Failed to parse declarative device descriptor for '{device_type}'"),
        )?)),
        None => Ok(None),
    }
}

impl SystemBus {
    /// Attach a declarative GPIO device described by `desc` for the placed
    /// `ext`. Dispatches on the descriptor's primitive; each primitive arm
    /// resolves the descriptor's pin bindings and constructs the (unchanged)
    /// Rust model, so behavior matches the former hand-written arm exactly.
    pub(crate) fn attach_declarative_device(
        &mut self,
        ext: &ExternalDevice,
        desc: &DeviceDescriptor,
    ) -> Result<()> {
        match desc.behavior.primitive.as_str() {
            "quadrature" => self.attach_quadrature(ext, desc),
            "matrix" => self.attach_matrix(ext, desc),
            other => Err(anyhow!(
                "declarative device '{}' names unknown primitive '{}'",
                ext.id,
                other
            )),
        }
    }

    /// `matrix` primitive → [`Keypad`]. Reproduces the former `"keypad"` arm:
    /// the `rows` role binds to a 4-entry list of GPIO **output** pads (ODR,
    /// which the keypad observes) and `cols` to a 4-entry list of GPIO **input**
    /// pads (IDR, which it drives); the pressed key is host-controlled through
    /// the `key` stimulus channel.
    fn attach_matrix(&mut self, ext: &ExternalDevice, desc: &DeviceDescriptor) -> Result<()> {
        use crate::peripherals::components::keypad::{Keypad, COLS, ROWS};

        let row_pins = self.pin_list_config(ext, desc, "rows")?;
        let col_pins = self.pin_list_config(ext, desc, "cols")?;

        // Rows are MCU outputs the keypad observes → resolve to ODR.
        let mut row_odr = [(0u64, 0u8); ROWS];
        for (i, pin) in row_pins.iter().enumerate() {
            row_odr[i] = Self::resolve_pin_odr(self, pin).ok_or_else(|| {
                anyhow!(
                    "keypad '{}' row_pin '{}' could not be resolved to a GPIO output",
                    ext.id,
                    pin
                )
            })?;
        }
        // Columns are MCU inputs the keypad drives → resolve to IDR.
        let mut col_idr = [(0u64, 0u8); COLS];
        for (i, pin) in col_pins.iter().enumerate() {
            col_idr[i] = Self::resolve_pin_idr(self, pin).ok_or_else(|| {
                anyhow!(
                    "keypad '{}' col_pin '{}' could not be resolved to a GPIO input",
                    ext.id,
                    pin
                )
            })?;
        }

        self.gpio_devices
            .push(Box::new(Keypad::new(ext.id.clone(), row_odr, col_idr)));
        Ok(())
    }

    /// `quadrature` primitive → [`RotaryEncoder`]. Reproduces the former
    /// `"rotary-encoder"` arm: both channels resolve to a GPIO **input** (IDR)
    /// register, the model walks the Gray sequence onto them, and rotation is
    /// host-controlled through the `position` stimulus channel.
    fn attach_quadrature(&mut self, ext: &ExternalDevice, desc: &DeviceDescriptor) -> Result<()> {
        let clk = self.pin_config(ext, desc, "a", "PA0")?;
        let dt = self.pin_config(ext, desc, "b", "PA1")?;
        let cpu_hz = param_u64(desc, ext, "cpu_hz", 80_000_000);

        let (clk_idr_addr, clk_bit) = Self::resolve_pin_idr(self, &clk).ok_or_else(|| {
            anyhow!(
                "rotary-encoder '{}' clk_pin '{}' could not be resolved to a GPIO input",
                ext.id,
                clk
            )
        })?;
        let (dt_idr_addr, dt_bit) = Self::resolve_pin_idr(self, &dt).ok_or_else(|| {
            anyhow!(
                "rotary-encoder '{}' dt_pin '{}' could not be resolved to a GPIO input",
                ext.id,
                dt
            )
        })?;

        self.gpio_devices.push(Box::new(
            crate::peripherals::components::rotary_encoder::RotaryEncoder::new(
                ext.id.clone(),
                clk_idr_addr,
                clk_bit,
                dt_idr_addr,
                dt_bit,
                cpu_hz,
            ),
        ));
        Ok(())
    }

    /// Resolve the pad label for the abstract pin `role`: read the `config:` key
    /// the descriptor binds that role to, falling back to `default` (preserving
    /// the old arm's fallback for a config that omits the pin).
    fn pin_config(
        &self,
        ext: &ExternalDevice,
        desc: &DeviceDescriptor,
        role: &str,
        default: &str,
    ) -> Result<String> {
        let key = desc.behavior.pins.get(role).ok_or_else(|| {
            anyhow!(
                "declarative device '{}' descriptor is missing pin role '{}'",
                ext.id,
                role
            )
        })?;
        Ok(ext
            .config
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or(default)
            .to_string())
    }

    /// Resolve a list-valued pin role (e.g. the keypad's `rows`/`cols`): read
    /// the `config:` key the descriptor binds it to as a 4-entry list of pad
    /// labels. Errors — keyed on the config field name — mirror the former
    /// hand-written `keypad` arm exactly.
    fn pin_list_config(
        &self,
        ext: &ExternalDevice,
        desc: &DeviceDescriptor,
        role: &str,
    ) -> Result<Vec<String>> {
        const EXPECTED: usize = 4;
        let key = desc.behavior.pins.get(role).ok_or_else(|| {
            anyhow!(
                "declarative device '{}' descriptor is missing pin role '{}'",
                ext.id,
                role
            )
        })?;
        let arr = ext
            .config
            .get(key)
            .and_then(|v| v.as_sequence())
            .ok_or_else(|| anyhow!("keypad '{}' config is missing a '{}' list", ext.id, key))?;
        let pins: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        if pins.len() != EXPECTED {
            return Err(anyhow!(
                "keypad '{}' expects exactly {} '{}' entries, got {}",
                ext.id,
                EXPECTED,
                key,
                pins.len()
            ));
        }
        Ok(pins)
    }
}

/// Read a `u64` primitive param: the descriptor's `params.<name>` entry gives
/// the `config:` key and default; `config[key]` overrides it when present.
/// Falls back to `fallback` when the descriptor omits the param entirely.
fn param_u64(desc: &DeviceDescriptor, ext: &ExternalDevice, name: &str, fallback: u64) -> u64 {
    let (config_key, default) = match desc.behavior.params.get(name) {
        Some(v) => (
            v.get("key").and_then(|k| k.as_str()).unwrap_or(name),
            v.get("default")
                .and_then(|d| d.as_u64())
                .unwrap_or(fallback),
        ),
        None => (name, fallback),
    };
    ext.config
        .get(config_key)
        .and_then(|v| v.as_u64())
        .unwrap_or(default)
}
