// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! BH1750 ambient light sensor (I²C lux meter).
//!
//! Default address 0x23 (ADDR pin low). Continuous high-res mode (0x10) returns
//! a 16-bit big-endian lux raw value; host SimInput `lux` maps to raw ≈ lux×1.2.

use crate::peripherals::i2c::I2cDevice;
use crate::sim_input::{InputChannel, SimInput, SimInputError};

const ADDR_DEFAULT: u8 = 0x23;
const CMD_POWER_ON: u8 = 0x01;
const CMD_RESET: u8 = 0x07;
const CMD_CONT_HRES: u8 = 0x10;

pub struct Bh1750 {
    address: u8,
    powered: bool,
    lux: f64,
    read_hi: bool,
    component_id: Option<String>,
}

impl Bh1750 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            powered: false,
            lux: 100.0,
            read_hi: true,
            component_id: None,
        }
    }

    fn raw(&self) -> u16 {
        (self.lux * 1.2).round().clamp(0.0, 65535.0) as u16
    }
}

impl I2cDevice for Bh1750 {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        self.read_hi = true;
    }

    fn write(&mut self, data: u8) {
        match data {
            CMD_POWER_ON | CMD_CONT_HRES => self.powered = true,
            CMD_RESET => {}
            _ => self.powered = true,
        }
    }

    fn read(&mut self) -> u8 {
        if !self.powered {
            return 0x00;
        }
        let raw = self.raw();
        if self.read_hi {
            self.read_hi = false;
            (raw >> 8) as u8
        } else {
            self.read_hi = true;
            (raw & 0xFF) as u8
        }
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }
}

pub const INPUT_CHANNELS: &[InputChannel] = &[InputChannel {
    key: "lux",
    label: "Illuminance",
    unit: "lx",
    min: 0.0,
    max: 100_000.0,
}];

impl SimInput for Bh1750 {
    fn input_channels(&self) -> &'static [InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        self.lux = value;
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Bh1750Kit;
pub static BH1750_KIT: Bh1750Kit = Bh1750Kit;

static BH1750_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "bh1750",
    label: "BH1750 Ambient Light",
    summary: "ROHM BH1750 I²C digital light sensor (lux).",
    detail: "Default address 0x23. Continuous H-res mode returns 16-bit BE raw ≈ lux×1.2. \
             Host sets lux via SimInput.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x23 (ADDR low).",
    }],
    labs: &[],
};

impl PeripheralKit for Bh1750Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &BH1750_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(ADDR_DEFAULT)?;
        let mut dev = Bh1750::new(address);
        dev.set_component_id(ctx.device_id().to_string());
        ctx.attach_i2c_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lux_100_encodes_to_raw() {
        let mut d = Bh1750::new(0x23);
        d.write(CMD_POWER_ON);
        d.write(CMD_CONT_HRES);
        d.start();
        let hi = d.read();
        let lo = d.read();
        let raw = ((hi as u16) << 8) | lo as u16;
        assert!((raw as i32 - 120).abs() < 2, "raw={raw}");
    }
}
