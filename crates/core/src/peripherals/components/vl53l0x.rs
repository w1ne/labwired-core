// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! VL53L0X laser ToF distance sensor (I²C, default 0x29).
//!
//! Behavioral subset for Adafruit / Pololu-style drivers:
//! identification registers (0xC0–0xC2), SYSRANGE start (0x00),
//! RESULT_INTERRUPT_STATUS (0x13), and RESULT_RANGE_STATUS range bytes
//! (0x1E/0x1F). Distance is host-settable; the ranging algorithm is not simulated.

use crate::peripherals::i2c::I2cDevice;

const REG_SYSRANGE_START: u8 = 0x00;
const REG_RESULT_INTERRUPT_STATUS: u8 = 0x13;
const REG_RESULT_RANGE_STATUS: u8 = 0x14;
const REG_RESULT_RANGE_VAL_H: u8 = 0x1E;
const REG_RESULT_RANGE_VAL_L: u8 = 0x1F;
const REG_IDENTIFICATION_MODEL_ID: u8 = 0xC0;
const REG_IDENTIFICATION_REVISION_ID: u8 = 0xC2;

pub struct Vl53l0x {
    address: u8,
    current_register: u8,
    register_address_written: bool,
    distance_mm: u16,
    ranging: bool,
    component_id: Option<String>,
}

impl Default for Vl53l0x {
    fn default() -> Self {
        Self::new(0x29)
    }
}

impl Vl53l0x {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            current_register: 0,
            register_address_written: false,
            distance_mm: 200,
            ranging: false,
            component_id: None,
        }
    }

    pub fn set_distance_mm(&mut self, mm: u16) {
        self.distance_mm = mm;
    }

    fn read_register(&self, reg: u8) -> u8 {
        match reg {
            REG_IDENTIFICATION_MODEL_ID => 0xEE,
            0xC1 => 0xAA,
            REG_IDENTIFICATION_REVISION_ID => 0x10,
            // data ready when ranging: bit0 set in interrupt status for many libs
            REG_RESULT_INTERRUPT_STATUS if self.ranging => 0x07,
            REG_RESULT_INTERRUPT_STATUS => 0x00,
            // Device ready / range status high nibble
            REG_RESULT_RANGE_STATUS => 0x00,
            REG_RESULT_RANGE_VAL_H => (self.distance_mm >> 8) as u8,
            REG_RESULT_RANGE_VAL_L => (self.distance_mm & 0xFF) as u8,
            // Also expose range at 0x14+10 style used by some continuous reads
            0x1C => (self.distance_mm >> 8) as u8,
            0x1D => (self.distance_mm & 0xFF) as u8,
            _ => 0,
        }
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        if reg == REG_SYSRANGE_START && (value & 0x01) != 0 {
            self.ranging = true;
        }
    }
}

impl I2cDevice for Vl53l0x {
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
            self.write_register(self.current_register, data);
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
    key: "distance",
    label: "Distance",
    unit: "mm",
    min: 0.0,
    max: 2000.0,
}];

impl crate::sim_input::SimInput for Vl53l0x {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        self.set_distance_mm(value.round().clamp(0.0, 2000.0) as u16);
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

pub struct Vl53l0xKit;
pub static VL53L0X_KIT: Vl53l0xKit = Vl53l0xKit;

static VL53L0X_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "vl53l0x",
    label: "VL53L0X ToF",
    summary: "Laser time-of-flight distance sensor over I²C (ST VL53L0X).",
    detail: "STMicroelectronics VL53L0X at 0x29. Identification + range result registers \
             for common Arduino drivers; host sets distance in mm via SimInput.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x29.",
    }],
    labs: &[],
};

impl PeripheralKit for Vl53l0xKit {
    fn metadata(&self) -> &'static KitMetadata {
        &VL53L0X_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x29)?;
        ctx.attach_i2c_device(Box::new(Vl53l0x::new(address)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::i2c::I2cDevice;

    #[test]
    fn model_id_and_range() {
        let mut dev = Vl53l0x::new(0x29);
        dev.set_distance_mm(350);
        dev.stop();
        dev.write(REG_IDENTIFICATION_MODEL_ID);
        assert_eq!(dev.read(), 0xEE);
        dev.stop();
        dev.write(REG_SYSRANGE_START);
        dev.write(0x01);
        dev.stop();
        dev.write(REG_RESULT_RANGE_VAL_H);
        assert_eq!(dev.read(), 0x01);
        assert_eq!(dev.read(), 0x5E); // 350
    }
}
