// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! PCF8574 8-bit I²C I/O expander (quasi-bidirectional port).

use crate::peripherals::i2c::I2cDevice;
use crate::sim_input::{InputChannel, SimInput, SimInputError};

const ADDR_DEFAULT: u8 = 0x20;

pub struct Pcf8574 {
    address: u8,
    port: u8,
    component_id: Option<String>,
}

impl Pcf8574 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            port: 0xFF,
            component_id: None,
        }
    }
}

impl I2cDevice for Pcf8574 {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {}

    fn write(&mut self, data: u8) {
        self.port = data;
    }

    fn read(&mut self) -> u8 {
        self.port
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }
}

pub const INPUT_CHANNELS: &[InputChannel] = &[InputChannel {
    key: "port",
    label: "Port",
    unit: "byte",
    min: 0.0,
    max: 255.0,
}];

impl SimInput for Pcf8574 {
    fn input_channels(&self) -> &'static [InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        self.port = value as u8;
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

pub struct Pcf8574Kit;
pub static PCF8574_KIT: Pcf8574Kit = Pcf8574Kit;

static PCF8574_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "pcf8574",
    label: "PCF8574 I/O Expander",
    summary: "NXP PCF8574 8-bit I²C quasi-bidirectional port expander.",
    detail: "Default address 0x20. Write one byte to latch outputs; read returns \
             the port value (host may override via SimInput port).",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x20.",
    }],
    labs: &[],
};

impl PeripheralKit for Pcf8574Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &PCF8574_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(ADDR_DEFAULT)?;
        let mut dev = Pcf8574::new(address);
        dev.set_component_id(ctx.device_id().to_string());
        ctx.attach_i2c_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_port() {
        let mut d = Pcf8574::new(0x20);
        d.write(0xA5);
        assert_eq!(d.read(), 0xA5);
    }
}
