// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Sensirion SHT30 I²C temperature + humidity sensor (common AE breakout).
//!
//! Single-shot high-repeatability command `0x24 0x00` then 6-byte response
//! (T MSB/LSB/CRC, RH MSB/LSB/CRC). Fixed default 25 °C / 50 %RH with SimInput.

use crate::peripherals::i2c::I2cDevice;
use crate::sim_input::{InputChannel, SimInput, SimInputError};

const ADDR_DEFAULT: u8 = 0x44;

pub struct Sht30 {
    address: u8,
    temperature_c: f64,
    humidity_rh: f64,
    cmd: [u8; 2],
    cmd_len: u8,
    read_idx: usize,
    component_id: Option<String>,
}

impl Sht30 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            temperature_c: 25.0,
            humidity_rh: 50.0,
            cmd: [0; 2],
            cmd_len: 0,
            read_idx: 0,
            component_id: None,
        }
    }

    fn encode_payload(&self) -> [u8; 6] {
        // raw_t = (T + 45) * 65535 / 175
        // raw_h = RH * 65535 / 100
        let raw_t = (((self.temperature_c + 45.0) * 65535.0) / 175.0)
            .round()
            .clamp(0.0, 65535.0) as u16;
        let raw_h = ((self.humidity_rh * 65535.0) / 100.0)
            .round()
            .clamp(0.0, 65535.0) as u16;
        let t_hi = (raw_t >> 8) as u8;
        let t_lo = (raw_t & 0xFF) as u8;
        let h_hi = (raw_h >> 8) as u8;
        let h_lo = (raw_h & 0xFF) as u8;
        [
            t_hi,
            t_lo,
            crc8(&[t_hi, t_lo]),
            h_hi,
            h_lo,
            crc8(&[h_hi, h_lo]),
        ]
    }
}

/// CRC-8 poly 0x31, init 0xFF (Sensirion).
fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0xFF;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            if crc & 0x80 != 0 {
                crc = (crc << 1) ^ 0x31;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

impl I2cDevice for Sht30 {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        self.cmd_len = 0;
        self.read_idx = 0;
    }

    fn write(&mut self, data: u8) {
        if (self.cmd_len as usize) < self.cmd.len() {
            self.cmd[self.cmd_len as usize] = data;
            self.cmd_len += 1;
        }
    }

    fn read(&mut self) -> u8 {
        let payload = self.encode_payload();
        let b = payload.get(self.read_idx).copied().unwrap_or(0xFF);
        self.read_idx = self.read_idx.saturating_add(1);
        b
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }
}

pub const INPUT_CHANNELS: &[InputChannel] = &[
    InputChannel {
        key: "temperature",
        label: "Temperature",
        unit: "C",
        min: -40.0,
        max: 125.0,
    },
    InputChannel {
        key: "humidity",
        label: "Humidity",
        unit: "%RH",
        min: 0.0,
        max: 100.0,
    },
];

impl SimInput for Sht30 {
    fn input_channels(&self) -> &'static [InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "temperature" => self.temperature_c = value,
            "humidity" => self.humidity_rh = value,
            _ => unreachable!(),
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

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Sht30Kit;
pub static SHT30_KIT: Sht30Kit = Sht30Kit;

static SHT30_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "sht30",
    label: "SHT30 Temp/Humidity",
    summary: "Sensirion SHT30 I²C temperature and humidity sensor.",
    detail: "Default address 0x44. Single-shot high-repeatability framing with CRC. \
             Host sets temperature (°C) and humidity (%RH).",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x44.",
    }],
    labs: &[],
};

impl PeripheralKit for Sht30Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &SHT30_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(ADDR_DEFAULT)?;
        let mut dev = Sht30::new(address);
        dev.set_component_id(ctx.device_id().to_string());
        ctx.attach_i2c_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_payload_crc_ok() {
        let d = Sht30::new(0x44);
        let p = d.encode_payload();
        assert_eq!(p[2], crc8(&p[0..2]));
        assert_eq!(p[5], crc8(&p[3..5]));
    }
}
