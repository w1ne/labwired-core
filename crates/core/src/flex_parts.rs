// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Part resolution for the CPU-free Flex path.
//!
//! A Flex manifest's `parts:` carries `labwired.part/v1` packs, the same
//! documents a system manifest carries (`docs/part-packs.md`). Every place
//! Flex turns a `type` into a model asks here first: a bus device, a
//! co-simulation model's `config.device_type` and a sink. A pack wins over
//! nothing: replacing a built-in part needs `overrides:`, exactly as on the
//! detailed path. A type no pack claims falls through to the embedded
//! catalog.

use labwired_config::{DeviceDescriptor, PartPack};

use crate::{SimResult, SimulationError};

fn error(message: impl Into<String>) -> SimulationError {
    SimulationError::Other(message.into())
}

/// Primitives a Flex manifest can run, and so the packs it accepts.
const FLEX_PRIMITIVES: &[&str] = &[
    "i2c_device",
    "spi_device",
    "display",
    "analog_source",
    "gpio_device",
    "onewire_device",
    "led_strip",
];

/// Check every pack of a Flex manifest before anything is built: the
/// portable envelope, the shadowing rule, and the primitive's own contract.
pub(crate) fn validate(parts: &[PartPack]) -> SimResult<()> {
    labwired_config::validate_part_packs(parts).map_err(|cause| error(format!("{cause:#}")))?;
    for pack in parts.iter().filter_map(PartPack::descriptor) {
        let primitive = pack.behavior.primitive.as_str();
        let result =
            crate::bus::part_pack::validate_shadowing(pack).and_then(|_| match primitive {
                "onewire_device" => {
                    crate::peripherals::components::declarative_onewire::validate_descriptor(pack)
                }
                "led_strip" => {
                    crate::peripherals::components::declarative_led_strip::validate_descriptor(pack)
                }
                _ if FLEX_PRIMITIVES.contains(&primitive) => {
                    crate::bus::part_pack::validate_runtime_descriptor(pack)
                }
                other => Err(anyhow::anyhow!(
                    "part pack '{}' names primitive '{other}', which Flex does not run; \
                 Flex runs {}",
                    pack.r#type,
                    FLEX_PRIMITIVES.join(", ")
                )),
            });
        result.map_err(|cause| error(format!("part pack '{}': {cause:#}", pack.r#type)))?;
    }
    Ok(())
}

/// The manifest's pack for `device_type`, if it carries one.
pub(crate) fn pack<'p>(parts: &'p [PartPack], device_type: &str) -> Option<&'p DeviceDescriptor> {
    labwired_config::resolve_part_pack(parts, device_type)
}

/// The descriptor of `device_type`: the manifest's pack, else the embedded
/// catalog's. `None`: neither describes it (a Rust-backed catalog model may
/// still exist).
pub(crate) fn descriptor(
    parts: &[PartPack],
    device_type: &str,
) -> SimResult<Option<DeviceDescriptor>> {
    if let Some(pack) = pack(parts, device_type) {
        return Ok(Some(pack.clone()));
    }
    crate::cosim::catalog_descriptor(device_type).map_err(|cause| error(format!("{cause:#}")))
}
