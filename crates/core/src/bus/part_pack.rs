// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Attach path for **manifest-carried part packs** — parts that are not built
//! into this engine.
//!
//! A system manifest may carry `parts:`, each a `labwired.part/v1` document (see
//! `docs/part-packs.md`). An `external_devices` entry whose `type:` names one is
//! modelled from that pack, so a private, vendor, or customer catalog connects
//! with no code in this repository and nothing published.
//!
//! The pack body is the same schema as the in-tree `configs/devices/*.yaml`
//! descriptors, which is the point: built-in parts and private parts are the
//! same kind of object, interpreted by the same primitives, so the private path
//! is the one we exercise every day rather than a side door that rots.
//!
//! What is NOT here is any new modelling. A pack names one of the irreducible
//! primitives (`i2c_device`, `spi_device`, `analog_source`, `quadrature`,
//! `matrix`, `one_wire`, `pulse_echo`) and this module routes it to the same construction the built-in
//! parts use. A part with a genuinely new wire protocol needs a new primitive,
//! which is a change to this crate — that boundary is real and worth being
//! straight about.
//!
//! ## Which door each primitive uses
//!
//! Built-in parts reach the bus by two different routes, and packs follow the
//! same ones rather than inventing a third:
//!
//! - **I²C** — [`i2c_device`] is called from `build_i2c_tree`, the single place
//!   every controller family (generic, ESP32-C3, the Xtensa glue, nRF52/nRF54L
//!   TWIM) turns a `type:` into a model. One hook there means a private sensor
//!   works on every MCU, not just the one we happened to try it on.
//! - **SPI** — SPI devices only ever attach through the `PeripheralKit`
//!   registry, so [`kit_for`] interns the pack as a kit and `from_config`
//!   attaches it exactly as it attaches a built-in.
//! - **Analog source** — a source owns both its ADC connection and simulator
//!   input channel, so it also attaches through its `PeripheralKit`.
//! - **GPIO / pin-timing** — no factory at all; `from_config` hands the
//!   descriptor to `attach_declarative_device`, the same call the embedded
//!   `configs/devices/*.yaml` descriptors take.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use anyhow::{Context, Result};
use labwired_config::{DeviceDescriptor, SystemManifest};

use crate::peripherals::components::declarative_analog::DeclarativeAnalogKit;
use crate::peripherals::components::{DeclarativeI2cKit, DeclarativeSpiKit};
use crate::peripherals::kit::PeripheralKit;
use crate::sim_input::SimInput;

/// Interned dynamic kits, keyed by the pack's canonical serialisation.
///
/// The registry contract is `&'static dyn PeripheralKit`, and a pack arrives at
/// runtime — so building one means leaking it. Keying on the pack's own bytes
/// makes that leak bounded and idempotent: re-running the same system reuses the
/// same kit, and editing a pack interns the edited one rather than serving a
/// stale model under the same `type:` (which would be the worst of both).
static INTERNED: LazyLock<Mutex<HashMap<String, &'static (dyn PeripheralKit + 'static)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Built-in `type:` strings a pack would shadow. Both built-in registries are
/// consulted, because "already built in" must mean the same thing to a pack
/// author regardless of which of our two internal paths happens to own the part.
fn builtin_owns(device_type: &str) -> bool {
    crate::peripherals::kit::registry::lookup(device_type).is_some()
        || labwired_config::embedded_device_yaml(device_type).is_some()
}

/// Reject an implicit replacement of an engine-owned device type.
///
/// This belongs to manifest preflight as well as lookup: a manifest carries a
/// complete portable catalog, so an unused collision is already invalid rather
/// than a deferred surprise when a later canvas places that part.
fn validate_shadowing(pack: &DeviceDescriptor) -> Result<()> {
    if builtin_owns(&pack.r#type) && pack.overrides.as_deref() != Some(pack.r#type.as_str()) {
        anyhow::bail!(
            "part pack '{}' ({}) shadows a built-in part. \
             Set `overrides: {}` to replace it deliberately, or rename the pack. \
             Which model ran is not something a bug report should have to guess at.",
            pack.r#type,
            pack.source
                .as_deref()
                .map(|s| format!("source: {s}"))
                .unwrap_or_else(|| "no declared source".to_string()),
            pack.r#type,
        );
    }
    Ok(())
}

/// Resolve `device_type` against the manifest's `parts:`, enforcing the
/// shadowing rule.
///
/// Returns `Ok(None)` when this system carries no such pack — the caller then
/// falls through to the built-in registries, which is the ordinary case.
pub(crate) fn lookup<'m>(
    manifest: &'m SystemManifest,
    device_type: &str,
) -> Result<Option<&'m DeviceDescriptor>> {
    let Some(pack) = manifest.resolve_part(device_type) else {
        return Ok(None);
    };
    validate_shadowing(pack)?;
    Ok(Some(pack))
}

/// Validate every part pack a manifest carries before any simulator path
/// starts attaching devices.
///
/// `SystemManifest::validate_parts` owns the portable envelope (schema,
/// duplicate names, and path entries). This adds the engine-owned semantic
/// check for each supported primitive, including packs that are not referenced
/// by the current canvas. A saved manifest is a complete portable catalog: a
/// malformed leaf must not wait for a later diagram to happen to use it.
pub(crate) fn validate_manifest(manifest: &SystemManifest) -> Result<()> {
    manifest.validate_parts()?;
    for entry in &manifest.parts {
        let pack = entry.descriptor().ok_or_else(|| {
            anyhow::anyhow!(
                "part pack path entry reached runtime validation after manifest validation"
            )
        })?;
        validate_shadowing(pack)?;
        validate_runtime_descriptor(pack)?;
    }
    Ok(())
}

/// Validate one pack against the primitive that will eventually interpret it,
/// without constructing/interning a runtime kit. The same seven primitive
/// names are the public `labwired.part/v1` contract.
fn validate_runtime_descriptor(pack: &DeviceDescriptor) -> Result<()> {
    let result = match pack.behavior.primitive.as_str() {
        "i2c_device" => crate::peripherals::components::declarative_i2c::validate_descriptor(pack),
        "spi_device" => crate::peripherals::components::declarative_spi::validate_descriptor(pack),
        "analog_source" => {
            crate::peripherals::components::declarative_analog::validate_descriptor(pack)
        }
        "quadrature" | "matrix" | "one_wire" | "pulse_echo" => {
            super::declarative_device::validate_descriptor(pack)
        }
        primitive => anyhow::bail!(
            "part pack '{}' names unsupported primitive '{}'. Supported primitives are \
             i2c_device, spi_device, analog_source, quadrature, matrix, one_wire, pulse_echo",
            pack.r#type,
            primitive
        ),
    };
    result.with_context(|| {
        format!(
            "part pack '{}' is not a valid {}",
            pack.r#type, pack.behavior.primitive
        )
    })
}

/// Intern a pack as a `PeripheralKit` for bus-resident (I²C / SPI / analog)
/// primitives. Returns `Ok(None)` for a primitive that is not bus-resident —
/// the GPIO / pin-timing family, which attaches through
/// [`super::declarative_device`] instead.
pub(crate) fn kit_for(pack: &DeviceDescriptor) -> Result<Option<&'static dyn PeripheralKit>> {
    let transport = match pack.behavior.primitive.as_str() {
        "i2c_device" => Transport::I2c,
        "spi_device" => Transport::Spi,
        "analog_source" => Transport::Analog,
        _ => return Ok(None),
    };

    let key = serde_yaml::to_string(pack)
        .with_context(|| format!("part pack '{}' could not be canonicalised", pack.r#type))?;

    let mut interned = INTERNED
        .lock()
        .map_err(|_| anyhow::anyhow!("part-pack registry lock poisoned"))?;
    if let Some(kit) = interned.get(&key) {
        return Ok(Some(*kit));
    }

    let kit: &'static dyn PeripheralKit = match transport {
        Transport::I2c => Box::leak(Box::new(DeclarativeI2cKit::from_yaml(&key).with_context(
            || format!("part pack '{}' is not a valid i2c_device", pack.r#type),
        )?)),
        Transport::Spi => Box::leak(Box::new(DeclarativeSpiKit::from_yaml(&key).with_context(
            || format!("part pack '{}' is not a valid spi_device", pack.r#type),
        )?)),
        Transport::Analog => Box::leak(Box::new(
            DeclarativeAnalogKit::from_yaml(&key).with_context(|| {
                format!("part pack '{}' is not a valid analog_source", pack.r#type)
            })?,
        )),
    };
    interned.insert(key, kit);
    Ok(Some(kit))
}

enum Transport {
    I2c,
    Spi,
    Analog,
}

/// Build the I²C model for `ext` from a manifest-carried pack, if one claims
/// its `type:`.
///
/// This is called from [`build_i2c_tree`](crate::peripherals::components::build_i2c_tree)
/// — the one place every I²C attach path (generic `from_config`, ESP32-C3, the
/// Xtensa glue, nRF52/nRF54L TWIM) turns a `type:` into a model. Resolving here
/// rather than in each caller is what makes "my private sensor works" mean the
/// same thing on every MCU instead of on whichever one we happened to test.
pub(crate) fn i2c_device(
    manifest: &SystemManifest,
    ext: &labwired_config::ExternalDevice,
) -> Result<Option<Box<dyn crate::peripherals::i2c::I2cDevice>>> {
    let Some(pack) = lookup(manifest, &ext.r#type)? else {
        return Ok(None);
    };
    if pack.behavior.primitive != "i2c_device" {
        return Ok(None);
    }
    let yaml = serde_yaml::to_string(pack)
        .with_context(|| format!("part pack '{}' could not be canonicalised", pack.r#type))?;
    // 0 tells GenericI2cDevice to use the pack's `default_address`.
    let address = ext
        .config
        .get("i2c_address")
        .and_then(|v| v.as_u64())
        .map(|a| a as u8)
        .unwrap_or(0);
    let mut device = crate::peripherals::components::GenericI2cDevice::from_yaml(&yaml, address)
        .with_context(|| {
            format!(
                "part pack '{}' ({}) is not a valid i2c_device",
                pack.r#type,
                pack.source.as_deref().unwrap_or("no declared source")
            )
        })?;
    // Same identity stamping the built-in factory does, so a pack's stimulus
    // channels are addressable by the id the manifest author wrote.
    device.set_component_id(ext.id.clone());
    Ok(Some(Box::new(device)))
}
