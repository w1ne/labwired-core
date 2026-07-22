// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! AS5600 12-bit magnetic rotary position sensor (I²C, default 0x36).
//!
//! Models the register map used by common Arduino drivers: STATUS (0x0B),
//! RAW_ANGLE (0x0C/0x0D), ANGLE (0x0E/0x0F), AGC (0x1A), MAGNITUDE (0x1B/0x1C).
//! Host SimInput sets `angle` in degrees (0–360).

use crate::peripherals::i2c::I2cDevice;

const REG_STATUS: u8 = 0x0B;
const REG_RAW_ANGLE_H: u8 = 0x0C;
const REG_RAW_ANGLE_L: u8 = 0x0D;
const REG_ANGLE_H: u8 = 0x0E;
const REG_ANGLE_L: u8 = 0x0F;
const REG_AGC: u8 = 0x1A;
const REG_MAG_H: u8 = 0x1B;
const REG_MAG_L: u8 = 0x1C;

/// STATUS bit 5 MD = magnet detected.
const STATUS_MD: u8 = 1 << 5;

pub struct As5600 {
    address: u8,
    current_register: u8,
    register_address_written: bool,
    /// Angle in degrees 0..360.
    angle_deg: f64,
    component_id: Option<String>,
}

impl Default for As5600 {
    fn default() -> Self {
        Self::new(0x36)
    }
}

impl As5600 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            current_register: 0,
            register_address_written: false,
            angle_deg: 0.0,
            component_id: None,
        }
    }

    pub fn set_angle_deg(&mut self, deg: f64) {
        let mut d = deg % 360.0;
        if d < 0.0 {
            d += 360.0;
        }
        self.angle_deg = d;
    }

    /// 12-bit raw angle 0..4095.
    fn raw12(&self) -> u16 {
        ((self.angle_deg / 360.0) * 4096.0).round().clamp(0.0, 4095.0) as u16
    }

    fn read_register(&self, reg: u8) -> u8 {
        let raw = self.raw12();
        match reg {
            REG_STATUS => STATUS_MD, // magnet always present
            REG_RAW_ANGLE_H | REG_ANGLE_H => ((raw >> 8) & 0x0F) as u8,
            REG_RAW_ANGLE_L | REG_ANGLE_L => (raw & 0xFF) as u8,
            REG_AGC => 128,
            REG_MAG_H => 0x0C,
            REG_MAG_L => 0x80, // healthy magnitude
            _ => 0,
        }
    }
}

impl I2cDevice for As5600 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        let value = self.read_register(self.current_register);
        self.current_register = self.current_register.wrapping_add(1);
        value
    }

    fn write(&mut self, data: u8) {
        if !self.register_address_written {
            self.current_register = data;
            self.register_address_written = true;
        } else {
            // Config registers ignored for wave-1 readback fidelity.
            self.current_register = self.current_register.wrapping_add(1);
        }
    }

    fn stop(&mut self) {
        self.register_address_written = false;
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        Some(self)
    }
}

pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[crate::sim_input::InputChannel {
    key: "angle",
    label: "Angle",
    unit: "deg",
    min: 0.0,
    max: 360.0,
}];

impl crate::sim_input::SimInput for As5600 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        self.set_angle_deg(value);
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

pub struct As5600Kit;
pub static AS5600_KIT: As5600Kit = As5600Kit;

static AS5600_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "as5600",
    label: "AS5600 Magnetic Encoder",
    summary: "12-bit contactless magnetic rotary position sensor over I²C.",
    detail: "ams AS5600 (default address 0x36). Host sets angle in degrees; firmware reads \
             RAW_ANGLE / ANGLE (12-bit) and STATUS magnet-detected bit.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x36.",
    }],
    labs: &[],
};

impl PeripheralKit for As5600Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &AS5600_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x36)?;
        ctx.attach_i2c_device(Box::new(As5600::new(address)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::i2c::I2cDevice;

    #[test]
    fn angle_180_is_half_scale() {
        let mut dev = As5600::new(0x36);
        dev.set_angle_deg(180.0);
        dev.stop();
        dev.write(REG_RAW_ANGLE_H);
        let hi = dev.read();
        let lo = dev.read();
        let raw = ((u16::from(hi) & 0x0F) << 8) | u16::from(lo);
        assert!((raw as i32 - 2048).abs() < 4, "raw={raw}");
    }

    #[test]
    fn status_reports_magnet() {
        let mut dev = As5600::new(0x36);
        dev.stop();
        dev.write(REG_STATUS);
        assert_eq!(dev.read() & STATUS_MD, STATUS_MD);
    }
}
