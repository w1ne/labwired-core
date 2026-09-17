// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Every `configs/devices/*.yaml` descriptor is a [`PeripheralKit`].
//!
//! # The hole this closes
//!
//! A descriptor and a kit are two different things, and only the kit is
//! visible to the browser. Attach resolution finds a descriptor on its own
//! (`bus::external_devices` step 3), so a part ported to a declarative
//! primitive RUNS the moment its YAML lands — which is exactly why nothing
//! ever failed when its manifest entry did not land with it. The palette the
//! playground draws comes from [`KitMetadata`], and a descriptor with no kit
//! contributes none: no label, no summary, no `config:` keys, no stimulus
//! channels. The part simply is not in the library, and every test stays
//! green because attach was never the thing that broke.
//!
//! Three parts sat in that hole — `keypad`, `dht22` and `rotary_encoder`, the
//! original declarative GPIO ports — plus `hc-sr04`, `dc-motor` and
//! `bldc-motor`. `hx711` escaped it only because somebody hand-wrote
//! `DeclarativeGpioKit` and a `HX711_KIT` static for it.
//!
//! # Why this is generic rather than a fourth wrapper
//!
//! Writing `DeclarativeMatrixKit`, `DeclarativeOneWireKit` and
//! `DeclarativePulseEchoKit` would fix today's six and rebuild the trap for
//! the seventh: opting in would still be a manual step somebody has to
//! remember, and forgetting it would still be silent. So the registry derives
//! a kit from EVERY row of [`labwired_config::EMBEDDED_DEVICES`] instead, for
//! any primitive, including primitives that do not exist yet.
//!
//! # One attach path, not two
//!
//! [`DeclarativeDeviceKit::attach`] makes the same call the universal
//! resolver's declarative step makes — `SystemBus::attach_declarative_device`
//! — so registering a descriptor adds a METADATA path, never a second
//! implementation of pad binding. The registry is consulted BEFORE the
//! declarative step, so these kits now claim their types first; delegating is
//! what keeps that change invisible to behaviour.
//!
//! Construction deliberately does NOT validate pin roles. Validation belongs
//! to attach (where it already happens, against the primitive's own role
//! table) and a descriptor must be able to produce manifest metadata even
//! when its primitive is serviced somewhere other than
//! `attach_declarative_device` — `dc_motor` and `bldc_motor` are converted to
//! typed motor plants at the loader boundary, and would otherwise be
//! unrepresentable in the library for no reason the user could see.

use anyhow::Result;
use labwired_config::DeviceDescriptor;

use super::{AttachCtx, KitMetadata, PeripheralKit};

/// Any declarative descriptor, as a [`PeripheralKit`].
pub struct DeclarativeDeviceKit {
    descriptor: DeviceDescriptor,
    metadata: &'static KitMetadata,
}

impl std::fmt::Debug for DeclarativeDeviceKit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeclarativeDeviceKit")
            .field("type", &self.descriptor.r#type)
            .field("primitive", &self.descriptor.behavior.primitive)
            .finish()
    }
}

impl DeclarativeDeviceKit {
    /// Derive a kit from descriptor YAML.
    ///
    /// Fails only when the YAML does not parse — metadata derivation itself
    /// is total, so a descriptor can never be dropped from the manifest for
    /// being incomplete. An incomplete descriptor fails loudly at attach,
    /// which is where it failed before this type existed.
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        Ok(Self::from_descriptor(descriptor))
    }

    pub fn from_descriptor(descriptor: DeviceDescriptor) -> Self {
        let channels = crate::peripherals::components::declarative_i2c::leak_channels(&descriptor);
        // The same derivation `hx711` already reads through: label, summary,
        // detail, `config_keys`, labs and stimulus channels straight off the
        // descriptor's `metadata:` block. Sharing it is what makes a part read
        // identically in the manifest whichever primitive it uses.
        let metadata = crate::peripherals::components::declarative_i2c::leak_gpio_metadata(
            &descriptor,
            channels,
        );
        Self {
            descriptor,
            metadata,
        }
    }

    pub fn device_type(&self) -> &str {
        &self.descriptor.r#type
    }
}

impl PeripheralKit for DeclarativeDeviceKit {
    fn metadata(&self) -> &'static KitMetadata {
        self.metadata
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        ctx.bus.attach_declarative_device(ctx.ext, &self.descriptor)
    }
}

/// Build the kit for one descriptor.
///
/// Bus-resident primitives (I²C, SPI, analog, display) already have generic
/// interpreters, and [`crate::bus::part_pack::kit_for`] is the one place that
/// chooses between them — the same chooser an out-of-tree part pack goes
/// through. Reusing it here rather than re-deciding means an in-tree
/// descriptor and a customer's own YAML produce the same kit for the same
/// primitive. Everything else — the GPIO / pin-timing family and anything
/// added later — gets [`DeclarativeDeviceKit`].
pub(crate) fn kit_for_descriptor(
    descriptor: &DeviceDescriptor,
) -> Result<&'static dyn PeripheralKit> {
    if let Some(kit) = crate::bus::part_pack::kit_for(descriptor)? {
        return Ok(kit);
    }
    Ok(Box::leak(Box::new(DeclarativeDeviceKit::from_descriptor(
        descriptor.clone(),
    ))))
}
