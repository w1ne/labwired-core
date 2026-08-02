// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! BNO055 9-DoF absolute orientation IMU (I²C, default 0x28).
//!
//! Models the Adafruit/Bosch register subset used by common Arduino sketches:
//! CHIP_ID (0x00 = 0xA0), OPR_MODE (0x3D), UNIT_SEL (0x3B), SYS_TRIGGER (0x3F),
//! and Euler heading/roll/pitch (0x1A–0x1F) in degrees × 16 (default units).
//! Host SimInput sets euler angles; fusion is not simulated.

use crate::peripherals::i2c::I2cDevice;

const REG_CHIP_ID: u8 = 0x00;
const REG_ACC_ID: u8 = 0x01;
const REG_MAG_ID: u8 = 0x02;
const REG_GYR_ID: u8 = 0x03;
const REG_PAGE_ID: u8 = 0x07;
const REG_EUL_H_LSB: u8 = 0x1A;
const REG_EUL_H_MSB: u8 = 0x1B;
const REG_EUL_R_LSB: u8 = 0x1C;
const REG_EUL_R_MSB: u8 = 0x1D;
const REG_EUL_P_LSB: u8 = 0x1E;
const REG_EUL_P_MSB: u8 = 0x1F;
const REG_UNIT_SEL: u8 = 0x3B;
const REG_OPR_MODE: u8 = 0x3D;
const REG_PWR_MODE: u8 = 0x3E;
const REG_SYS_TRIGGER: u8 = 0x3F;
const REG_SYS_STATUS: u8 = 0x39;
const REG_SYS_ERR: u8 = 0x3A;
const REG_CALIB_STAT: u8 = 0x35;

const CHIP_ID: u8 = 0xA0;
const ACC_ID: u8 = 0xFB;
const MAG_ID: u8 = 0x32;
const GYR_ID: u8 = 0x0F;

pub struct Bno055 {
    address: u8,
    current_register: u8,
    register_address_written: bool,
    page: u8,
    opr_mode: u8,
    unit_sel: u8,
    pwr_mode: u8,
    /// Euler angles in degrees.
    heading: f64,
    roll: f64,
    pitch: f64,
    component_id: Option<String>,
}

impl Default for Bno055 {
    fn default() -> Self {
        Self::new(0x28)
    }
}

impl Bno055 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            current_register: 0,
            register_address_written: false,
            page: 0,
            opr_mode: 0x00, // CONFIGMODE
            unit_sel: 0x00,
            pwr_mode: 0x00,
            heading: 0.0,
            roll: 0.0,
            pitch: 0.0,
            component_id: None,
        }
    }

    pub fn set_euler_deg(&mut self, heading: f64, roll: f64, pitch: f64) {
        self.heading = heading;
        self.roll = roll;
        self.pitch = pitch;
    }

    /// Degrees × 16 as i16 little-endian (Adafruit default UNIT_SEL).
    fn eul_i16(deg: f64) -> i16 {
        (deg * 16.0).round().clamp(i16::MIN as f64, i16::MAX as f64) as i16
    }

    fn read_register(&self, reg: u8) -> u8 {
        if self.page != 0 {
            return 0;
        }
        match reg {
            REG_CHIP_ID => CHIP_ID,
            REG_ACC_ID => ACC_ID,
            REG_MAG_ID => MAG_ID,
            REG_GYR_ID => GYR_ID,
            REG_PAGE_ID => self.page,
            REG_EUL_H_LSB => (Self::eul_i16(self.heading) as u16 & 0xFF) as u8,
            REG_EUL_H_MSB => ((Self::eul_i16(self.heading) as u16) >> 8) as u8,
            REG_EUL_R_LSB => (Self::eul_i16(self.roll) as u16 & 0xFF) as u8,
            REG_EUL_R_MSB => ((Self::eul_i16(self.roll) as u16) >> 8) as u8,
            REG_EUL_P_LSB => (Self::eul_i16(self.pitch) as u16 & 0xFF) as u8,
            REG_EUL_P_MSB => ((Self::eul_i16(self.pitch) as u16) >> 8) as u8,
            REG_UNIT_SEL => self.unit_sel,
            REG_OPR_MODE => self.opr_mode,
            REG_PWR_MODE => self.pwr_mode,
            REG_SYS_STATUS => 0x05, // sensor fusion algorithm running
            REG_SYS_ERR => 0x00,
            REG_CALIB_STAT => 0xFF, // fully calibrated
            _ => 0,
        }
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            REG_PAGE_ID => self.page = value & 0x01,
            REG_OPR_MODE => self.opr_mode = value,
            REG_UNIT_SEL => self.unit_sel = value,
            REG_PWR_MODE => self.pwr_mode = value,
            REG_SYS_TRIGGER => {
                // soft reset bit 5 — ignore, stay powered
                let _ = value;
            }
            _ => {}
        }
    }
}

impl I2cDevice for Bno055 {
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

pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "heading",
        label: "Heading",
        unit: "deg",
        min: 0.0,
        max: 360.0,
    },
    crate::sim_input::InputChannel {
        key: "roll",
        label: "Roll",
        unit: "deg",
        min: -180.0,
        max: 180.0,
    },
    crate::sim_input::InputChannel {
        key: "pitch",
        label: "Pitch",
        unit: "deg",
        min: -180.0,
        max: 180.0,
    },
];

impl crate::sim_input::SimInput for Bno055 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "heading" => self.heading = value,
            "roll" => self.roll = value,
            "pitch" => self.pitch = value,
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

pub struct Bno055Kit;
pub static BNO055_KIT: Bno055Kit = Bno055Kit;

static BNO055_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "bno055",
    label: "BNO055 IMU",
    summary: "9-DoF absolute orientation sensor (Bosch BNO055) over I²C.",
    detail: "CHIP_ID 0xA0, operation mode, and Euler heading/roll/pitch registers for Adafruit \
             BNO055 sketches. Host sets euler angles via SimInput; fusion is not simulated.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x28 (COM3 low).",
    }],
    labs: &[],
};

impl PeripheralKit for Bno055Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &BNO055_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x28)?;
        ctx.attach_i2c_device(Box::new(Bno055::new(address)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::i2c::I2cDevice;

    #[test]
    fn chip_id_and_euler() {
        let mut dev = Bno055::new(0x28);
        dev.set_euler_deg(90.0, 0.0, -10.0);
        dev.stop();
        dev.write(REG_CHIP_ID);
        assert_eq!(dev.read(), CHIP_ID);
        dev.stop();
        dev.write(REG_EUL_H_LSB);
        let lsb = dev.read();
        let msb = dev.read();
        let h = i16::from_le_bytes([lsb, msb]);
        assert_eq!(h, 90 * 16);
    }

    #[test]
    fn opr_mode_write() {
        let mut dev = Bno055::new(0x28);
        dev.stop();
        dev.write(REG_OPR_MODE);
        dev.write(0x0C); // NDOF
        dev.stop();
        dev.write(REG_OPR_MODE);
        assert_eq!(dev.read(), 0x0C);
    }
}
