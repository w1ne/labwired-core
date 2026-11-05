// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CFSensor XGZP6857D200KPGPN33 digital gauge-pressure sensor.
//!
//! The non-C part fitted to the Opulo LumenPnP uses the V2.5 protocol with
//! address 0x6d, command register 0x30, and signed 24-bit pressure at 0x06.
//! Its +200 kPa range uses K=32 counts/Pa. Temperature is signed 16-bit /256.
//! A conversion takes the datasheet's recommended 20 ms wait in this model.
//! Completed samples remain readable while another conversion is in flight.
//!
//! Supported commands are combined single-shot (0x0a) and periodic combined
//! conversion (mode 3, sleep interval in 62.5 ms units). Factory OTP trimming,
//! raw bridge output, ADC noise, and the newer C-series protocol are outside
//! this functional register model.
//!
//! Source https://www.endrich.com/Datenbl%C3%A4tter/Sensoren/XGZP6857D-Pressure-Sensor-V2.5.pdf

use std::borrow::Cow;

use crate::peripherals::i2c::I2cDevice;
use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};
use crate::sim_input::{InputChannel, SimInput, SimInputError};

pub const XGZP6857D_ADDRESS: u8 = 0x6d;
pub const XGZP6857D_CONVERSION_US: u64 = 20_000;
pub const XGZP6857D_COUNTS_PER_PA: f64 = 32.0;

const INPUTS: &[InputChannel] = &[
    InputChannel {
        key: Cow::Borrowed("pressure_pa"),
        label: Cow::Borrowed("Gauge pressure"),
        unit: Cow::Borrowed("Pa"),
        min: -100_000.0,
        max: 200_000.0,
        default: Some(0.0),
    },
    InputChannel {
        key: Cow::Borrowed("temperature_c"),
        label: Cow::Borrowed("Temperature"),
        unit: Cow::Borrowed("°C"),
        min: -20.0,
        max: 100.0,
        default: Some(25.0),
    },
];

#[derive(Debug)]
pub struct Xgzp6857d {
    component_id: Option<String>,
    pressure_pa: f64,
    temperature_c: f64,
    sample: [u8; 5],
    pointer: u8,
    expecting_register: bool,
    command: u8,
    remaining_us: Option<u64>,
    sleeping: bool,
}

impl Xgzp6857d {
    pub fn new() -> Self {
        Self {
            component_id: None,
            pressure_pa: 0.0,
            temperature_c: 25.0,
            sample: [0; 5],
            pointer: 0,
            expecting_register: true,
            command: 0,
            remaining_us: None,
            sleeping: false,
        }
    }

    fn complete_conversion(&mut self) {
        let pressure = (self.pressure_pa * XGZP6857D_COUNTS_PER_PA).round() as i32;
        self.sample[..3].copy_from_slice(&pressure.to_be_bytes()[1..]);
        let temperature = (self.temperature_c * 256.0).round() as i16;
        self.sample[3..].copy_from_slice(&temperature.to_be_bytes());
        if self.command & 0x07 == 0x03 {
            self.sleeping = true;
            self.remaining_us = Some(u64::from(self.command >> 4) * 62_500);
        } else {
            self.command &= !0x08;
            self.remaining_us = None;
        }
    }

    fn write_command(&mut self, value: u8) {
        self.command = value;
        self.sleeping = false;
        self.remaining_us = None;
        if value & 0x08 != 0 && matches!(value & 0x07, 0x02 | 0x03) {
            self.remaining_us = Some(XGZP6857D_CONVERSION_US);
        } else {
            self.command &= !0x08;
        }
    }
}

impl Default for Xgzp6857d {
    fn default() -> Self {
        Self::new()
    }
}

impl I2cDevice for Xgzp6857d {
    fn address(&self) -> u8 {
        XGZP6857D_ADDRESS
    }

    fn start(&mut self) {
        self.expecting_register = true;
    }

    fn write(&mut self, data: u8) {
        if self.expecting_register {
            self.pointer = data;
            self.expecting_register = false;
            return;
        }
        if self.pointer == 0x30 {
            self.write_command(data);
        }
        self.pointer = self.pointer.wrapping_add(1);
    }

    fn read(&mut self) -> u8 {
        let data = match self.pointer {
            0x06..=0x0a => self.sample[usize::from(self.pointer - 0x06)],
            0x30 => self.command,
            _ => 0xff,
        };
        self.pointer = self.pointer.wrapping_add(1);
        data
    }

    fn advance_time_us(&mut self, mut us: u64) {
        while let Some(remaining) = self.remaining_us {
            if us < remaining {
                self.remaining_us = Some(remaining - us);
                return;
            }
            us -= remaining;
            if self.sleeping {
                self.sleeping = false;
                self.remaining_us = Some(XGZP6857D_CONVERSION_US);
            } else {
                self.complete_conversion();
            }
            if us == 0 {
                return;
            }
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }
}

impl SimInput for Xgzp6857d {
    fn input_channels(&self) -> &[InputChannel] {
        INPUTS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "pressure_pa" => self.pressure_pa = value,
            "temperature_c" => self.temperature_c = value,
            _ => unreachable!("require_channel validated the key"),
        }
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

pub struct Xgzp6857dKit;
pub static XGZP6857D_KIT: Xgzp6857dKit = Xgzp6857dKit;

static XGZP6857D_METADATA: KitMetadata = KitMetadata {
    device_type: Cow::Borrowed("xgzp6857d"),
    label: Cow::Borrowed("XGZP6857D pressure sensor"),
    summary: Cow::Borrowed("XGZP6857D200KPGPN33 gauge pressure and temperature over I²C."),
    detail: Cow::Borrowed(
        "Non-C V2.5 protocol at 0x6D. Signed 24-bit pressure with K=32 counts/Pa, \
         signed 16-bit temperature with 256 counts/°C, 20 ms conversion timing, \
         command busy status, single-shot and periodic acquisition.",
    ),
    transport: Transport::I2c,
    category: Category::I2c,
    inputs: Cow::Borrowed(INPUTS),
    config_keys: Cow::Borrowed(&[ConfigKey {
        name: Cow::Borrowed("i2c_address"),
        ty: ConfigType::Int,
        doc: Cow::Borrowed("Fixed 7-bit factory address 0x6D for this sensor variant."),
    }]),
    labs: Cow::Borrowed(&[]),
};

impl PeripheralKit for Xgzp6857dKit {
    fn metadata(&self) -> &'static KitMetadata {
        &XGZP6857D_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        anyhow::ensure!(
            ctx.i2c_address_or(XGZP6857D_ADDRESS)? == XGZP6857D_ADDRESS,
            "xgzp6857d200kpgpn33 uses fixed address 0x6d"
        );
        ctx.attach_i2c_device(Box::new(Xgzp6857d::new()))?;
        Ok(())
    }
}
