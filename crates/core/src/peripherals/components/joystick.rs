// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Two potentiometer axes and a push contact, under one public component id.
//! The axes reuse the declarative potentiometer voltage divider. Unwired
//! outputs are not offered by discovery: they cannot drive an MCU pin.

use std::borrow::Cow;

use anyhow::{bail, Context, Result};

use super::button::Button;
use super::declarative_analog::{DeclarativeAnalogDevice, POTENTIOMETER_KIT};
use crate::bus::sim_inputs::AnalogSource;
use crate::bus::SystemBus;
use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};
use crate::sim_input::{InputChannel, SimInput, SimInputError};

const INPUTS: &[InputChannel] = &[
    InputChannel {
        key: Cow::Borrowed("x"),
        label: Cow::Borrowed("X axis"),
        unit: Cow::Borrowed("normalized"),
        min: -1.0,
        max: 1.0,
        default: Some(0.0),
    },
    InputChannel {
        key: Cow::Borrowed("y"),
        label: Cow::Borrowed("Y axis"),
        unit: Cow::Borrowed("normalized"),
        min: -1.0,
        max: 1.0,
        default: Some(0.0),
    },
    InputChannel {
        key: Cow::Borrowed("pressed"),
        label: Cow::Borrowed("Pressed"),
        unit: Cow::Borrowed("bool"),
        min: 0.0,
        max: 1.0,
        default: Some(0.0),
    },
];

struct JoystickAxis {
    pot: DeclarativeAnalogDevice,
    input: &'static InputChannel,
}

impl SimInput for JoystickAxis {
    fn input_channels(&self) -> &[InputChannel] {
        std::slice::from_ref(self.input)
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        self.pot.set_input("position", (value + 1.0) * 50.0)
    }

    fn component_id(&self) -> Option<&str> {
        self.pot.component_id()
    }

    fn set_component_id(&mut self, id: String) {
        self.pot.set_component_id(id);
    }
}

impl AnalogSource for JoystickAxis {
    fn output_mv(&self) -> u16 {
        self.pot.output_mv()
    }
}

pub struct JoystickKit;
pub static JOYSTICK_KIT: JoystickKit = JoystickKit;

const METADATA: KitMetadata = KitMetadata {
    device_type: Cow::Borrowed("joystick"),
    label: Cow::Borrowed("Analog Joystick"),
    summary: Cow::Borrowed("Two ratiometric potentiometer axes and a push contact."),
    detail: Cow::Borrowed(
        "x/y range -1..1, centered at zero. Each connected axis spans GND to VCC. \
         SW is an active-low momentary contact. Mechanical bounce, axis noise and \
         spring dynamics are not modeled.",
    ),
    transport: Transport::Analog,
    category: Category::Analog,
    config_keys: Cow::Borrowed(&[
        ConfigKey {
            name: Cow::Borrowed("x_channel"),
            ty: ConfigType::Int,
            doc: Cow::Borrowed("ADC channel for VRX; absent when unwired."),
        },
        ConfigKey {
            name: Cow::Borrowed("y_channel"),
            ty: ConfigType::Int,
            doc: Cow::Borrowed("ADC channel for VRY; absent when unwired."),
        },
        ConfigKey {
            name: Cow::Borrowed("x_connection"),
            ty: ConfigType::Str,
            doc: Cow::Borrowed("Optional ADC controller for VRX; defaults to connection."),
        },
        ConfigKey {
            name: Cow::Borrowed("y_connection"),
            ty: ConfigType::Str,
            doc: Cow::Borrowed("Optional ADC controller for VRY; defaults to connection."),
        },
        ConfigKey {
            name: Cow::Borrowed("sw_pin"),
            ty: ConfigType::Str,
            doc: Cow::Borrowed("MCU input wired to the active-low SW contact; optional."),
        },
        ConfigKey {
            name: Cow::Borrowed("supply_mv"),
            ty: ConfigType::Float,
            doc: Cow::Borrowed("Voltage across the axis potentiometers, millivolts; default 3300."),
        },
        ConfigKey {
            name: Cow::Borrowed("x"),
            ty: ConfigType::Float,
            doc: Cow::Borrowed("Initial X axis position (-1..1, default 0)."),
        },
        ConfigKey {
            name: Cow::Borrowed("y"),
            ty: ConfigType::Float,
            doc: Cow::Borrowed("Initial Y axis position (-1..1, default 0)."),
        },
    ]),
    labs: Cow::Borrowed(&[]),
    inputs: Cow::Borrowed(INPUTS),
};

impl PeripheralKit for JoystickKit {
    fn metadata(&self) -> &KitMetadata {
        &METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        let mut attached = false;
        for input in &INPUTS[..2] {
            let channel_key = format!("{}_channel", input.key);
            let Some(value) = ctx.ext.config.get(&channel_key) else {
                continue;
            };
            let channel = value
                .as_u64()
                .and_then(|v| u8::try_from(v).ok())
                .with_context(|| {
                    format!(
                        "joystick '{}': {channel_key} must be an integer in 0..255",
                        ctx.device_id()
                    )
                })?;
            let mut pot = POTENTIOMETER_KIT.build(channel)?;
            pot.set_supply_mv(ctx.config_f64("supply_mv").unwrap_or(3300.0), Some(3300.0))?;
            let mut source = JoystickAxis { pot, input };
            source.set_input(
                input.key.as_ref(),
                ctx.config_f64(input.key.as_ref()).unwrap_or(0.0),
            )?;
            let mut axis = ctx.ext.clone();
            if let Some(connection) = ctx.config_str(&format!("{}_connection", input.key)) {
                axis.connection = connection.to_string();
            }
            AttachCtx::new(ctx.bus, &axis).attach_analog_source(channel, Box::new(source))?;
            attached = true;
        }
        if let Some(pin) = ctx.config_str("sw_pin").map(str::to_string) {
            let gpio = SystemBus::resolve_pin_idr_pub(ctx.bus, &pin).with_context(|| {
                format!(
                    "joystick '{}': unresolved SW input '{pin}'",
                    ctx.device_id()
                )
            })?;
            ctx.drive_pin_input(&pin, true)?;
            ctx.bus.gpio_devices.push(Box::new(Button::new(
                ctx.device_id().to_string(),
                gpio,
                false,
            )));
            attached = true;
        }
        if !attached {
            bail!(
                "joystick '{}': wire x_channel, y_channel or sw_pin",
                ctx.device_id()
            );
        }
        Ok(())
    }
}
