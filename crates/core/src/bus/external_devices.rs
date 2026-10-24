// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! The canonical external-device resolution order, shared by every machine-
//! construction path (`SystemBus::from_config`, the Xtensa attacher) so the
//! precedence can never drift between chips:
//!
//!   1. manifest-carried parts (`parts:`, `part_pack`) — most specific wins
//!   2. the `PeripheralKit` registry — the engine's built-in catalog
//!   3. declarative `configs/devices/*.yaml` descriptors
//!
//! Anything still unclaimed returns [`UniversalResolution::Unrecognized`] and
//! the CALLER decides the policy (a legacy factory, chip-specific arms, a
//! hard error). History: this order used to be re-implemented per chip
//! family; the copies diverged (a shadowed OLED arm resolved the same
//! manifest differently per chip, and an unknown type warned-and-skipped on
//! one path while erroring on another). One copy now.

use crate::bus::SystemBus;

/// Outcome of the universal resolution pass for one declared device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UniversalResolution {
    /// A universal arm claimed the device and attached it.
    Attached,
    /// Nothing universal claims this type; the caller's chip-specific
    /// residue (legacy factories, hand arms, error policy) decides.
    Unrecognized,
}

/// Resolve and attach one `external_devices` entry through the canonical
/// order. See the module docs for the precedence contract.
pub fn attach_external_device_universal(
    bus: &mut SystemBus,
    manifest: &labwired_config::SystemManifest,
    ext: &labwired_config::ExternalDevice,
) -> anyhow::Result<UniversalResolution> {
    // 1. Parts this manifest CARRIES (`parts:`) — a private catalog, a vendor
    //    library, a customer's own sensor. They resolve first because they
    //    are the most specific thing anyone said about this system;
    //    shadowing a built-in is rejected inside `lookup`.
    if let Some(pack) = super::part_pack::lookup(manifest, &ext.r#type)? {
        if let Some(kit) = super::part_pack::kit_for(pack)? {
            let mut ctx = crate::peripherals::kit::AttachCtx::new(bus, ext);
            kit.attach(&mut ctx)?;
        } else {
            // Not bus-resident: GPIO / pin-timing primitives take the same
            // path an embedded descriptor takes.
            bus.attach_declarative_device(ext, pack)?;
        }
        return Ok(UniversalResolution::Attached);
    }
    // 2. The PeripheralKit registry: each device ships its own `attach` next
    //    to its model instead of a hand-written arm in a chip path.
    if let Some(kit) = crate::peripherals::kit::registry::lookup(&ext.r#type) {
        let mut ctx = crate::peripherals::kit::AttachCtx::new(bus, ext);
        kit.attach(&mut ctx)?;
        return Ok(UniversalResolution::Attached);
    }
    // 3. Declarative `configs/devices/*.yaml` descriptors (GPIO / pin-timing
    //    family): resolve pin bindings, instantiate the named primitive.
    if let Some(desc) = super::declarative_device::lookup(&ext.r#type)? {
        bus.attach_declarative_device(ext, &desc)?;
        return Ok(UniversalResolution::Attached);
    }
    Ok(UniversalResolution::Unrecognized)
}

/// The shared "nothing claims this device" error. Used by paths that treat
/// an unattachable declaration as a hard failure (a green run with a
/// silently missing device is worse than no run).
pub fn unsupported_external_device_error(
    family: &str,
    ext: &labwired_config::ExternalDevice,
) -> anyhow::Error {
    anyhow::anyhow!(
        "{family} external_devices: unsupported type '{}' on '{}'",
        ext.r#type,
        ext.id
    )
}

impl SystemBus {
    /// Attach a programmatic platform's manifest parts after its controllers
    /// are installed, using the same resolution order as config-built buses.
    /// Declarations are recorded first because kits resolve other placements.
    /// An attachment error may follow earlier successful attachments; discard
    /// that incomplete bus instead of retrying and duplicating those devices.
    /// This single-use funnel rejects duplicate IDs and recorded placements.
    pub fn attach_manifest_external_devices(
        &mut self,
        manifest: &labwired_config::SystemManifest,
    ) -> anyhow::Result<()> {
        if !self.external_device_decls.is_empty() {
            anyhow::bail!(
                "manifest external devices already recorded; refusing repeated attachment"
            );
        }
        let mut ids = std::collections::HashSet::new();
        for ext in &manifest.external_devices {
            if !ids.insert(ext.id.as_str()) {
                anyhow::bail!("duplicate external device id '{}'", ext.id);
            }
        }
        self.record_external_devices(manifest);
        for ext in &manifest.external_devices {
            if attach_external_device_universal(self, manifest, ext)?
                == UniversalResolution::Unrecognized
            {
                return Err(unsupported_external_device_error(
                    "programmatic platform",
                    ext,
                ));
            }
        }
        self.recompute_walk_deletable();
        Ok(())
    }
}
