// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Passive SPDT end-stop: COM connects to NC at rest and NO when pressed.
//! The existing contact model supplies deterministic debounced behavior. The
//! wiring compiler supplies the COM rail polarity and connected MCU inputs.
//! Open-contact levels represent the circuit's pull resistor; bounce and
//! contact resistance are not modeled. Contact arrangement reference:
//! https://omronfs.omron.com/en_US/ecb/products/pdf/en-ss.pdf

use std::borrow::Cow;

use anyhow::{bail, Context, Result};

use super::button::{Button, CHANNELS};
use crate::bus::SystemBus;
use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct LimitSwitchKit;
pub static LIMIT_SWITCH_KIT: LimitSwitchKit = LimitSwitchKit;

const METADATA: KitMetadata = KitMetadata {
    device_type: Cow::Borrowed("limit-switch"),
    label: Cow::Borrowed("Limit Switch (SPDT)"),
    summary: Cow::Borrowed(
        "Mechanical end-stop with common, normally-open and normally-closed contacts.",
    ),
    detail: Cow::Borrowed(
        "Pressed=0 connects COM to NC; pressed=1 connects COM to NO. \
         Wire one or both contacts to pulled MCU inputs. common_high reflects \
         the COM rail. Bounce and contact resistance are not modeled.",
    ),
    transport: Transport::GpioGroup,
    category: Category::Gpio,
    config_keys: Cow::Borrowed(&[
        ConfigKey {
            name: Cow::Borrowed("no_pin"),
            ty: ConfigType::Str,
            doc: Cow::Borrowed("MCU input connected to NO; optional when NC is wired."),
        },
        ConfigKey {
            name: Cow::Borrowed("nc_pin"),
            ty: ConfigType::Str,
            doc: Cow::Borrowed("MCU input connected to NC; optional when NO is wired."),
        },
        ConfigKey {
            name: Cow::Borrowed("common_high"),
            ty: ConfigType::Bool,
            doc: Cow::Borrowed(
                "COM rail: false=GND with input pull-ups, true=VCC with pull-downs.",
            ),
        },
    ]),
    labs: Cow::Borrowed(&[]),
    inputs: Cow::Borrowed(std::slice::from_ref(&CHANNELS[0])),
};

impl PeripheralKit for LimitSwitchKit {
    fn metadata(&self) -> &KitMetadata {
        &METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        let common_high = ctx.config_bool("common_high").unwrap_or(false);
        let mut contacts = Vec::new();
        for (key, active_high) in [("no_pin", common_high), ("nc_pin", !common_high)] {
            if let Some(pin) = ctx.config_str(key) {
                let gpio = SystemBus::resolve_pin_idr_pub(ctx.bus, pin).with_context(|| {
                    format!(
                        "limit-switch '{}': unresolved {key} '{pin}'",
                        ctx.device_id()
                    )
                })?;
                contacts.push((pin.to_string(), gpio, active_high));
            }
        }
        let Some((_, first, active_high)) = contacts.first() else {
            bail!("limit-switch '{}': wire no_pin or nc_pin", ctx.device_id());
        };
        if contacts.len() == 2 && contacts[0].1 == contacts[1].1 {
            bail!(
                "limit-switch '{}': NO and NC cannot share one MCU input",
                ctx.device_id()
            );
        }
        let mut contact = Button::new(ctx.device_id().to_string(), *first, *active_high);
        for (_, gpio, active_high) in contacts.iter().skip(1) {
            contact.add_contact(*gpio, *active_high);
        }
        for (pin, _, active_high) in contacts {
            ctx.drive_pin_input(&pin, !active_high)?;
        }
        ctx.bus.gpio_devices.push(Box::new(contact));
        Ok(())
    }
}
