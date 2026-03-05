// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::i2c::I2cDevice;

/// MPU6050 6-DoF I2C IMU Component
#[derive(Debug, serde::Serialize)]
pub struct Mpu6050 {
    address: u8,
    current_register: u8,

    // Core registers
    pwr_mgmt_1: u8,
    who_am_i: u8,

    // Sensor data (dummy static values for now, but could be dynamic)
    accel_x: i16,
    accel_y: i16,
    accel_z: i16,
    gyro_x: i16,
    gyro_y: i16,
    gyro_z: i16,

    // Internal state tracking for I2C register pointer
    register_address_written: bool,
}

impl Default for Mpu6050 {
    fn default() -> Self {
        Self::new(0x68) // Default I2C address for MPU6050
    }
}

impl Mpu6050 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            current_register: 0,
            pwr_mgmt_1: 0x40, // Reset value (sleep mode bit set)
            who_am_i: 0x68,

            // Dummy calibration/data
            accel_x: 0x0123,
            accel_y: 0x0456,
            accel_z: 0x4000, // Roughly 1g depending on scale
            gyro_x: 0x0010,
            gyro_y: 0x0020,
            gyro_z: 0x0030,

            register_address_written: false,
        }
    }

    fn read_register(&self, reg: u8) -> u8 {
        match reg {
            0x3B => (self.accel_x >> 8) as u8,
            0x3C => (self.accel_x & 0xFF) as u8,
            0x3D => (self.accel_y >> 8) as u8,
            0x3E => (self.accel_y & 0xFF) as u8,
            0x3F => (self.accel_z >> 8) as u8,
            0x40 => (self.accel_z & 0xFF) as u8,

            0x43 => (self.gyro_x >> 8) as u8,
            0x44 => (self.gyro_x & 0xFF) as u8,
            0x45 => (self.gyro_y >> 8) as u8,
            0x46 => (self.gyro_y & 0xFF) as u8,
            0x47 => (self.gyro_z >> 8) as u8,
            0x48 => (self.gyro_z & 0xFF) as u8,

            0x6B => self.pwr_mgmt_1,
            0x75 => self.who_am_i,
            _ => 0,
        }
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        if reg == 0x6B {
            self.pwr_mgmt_1 = value;
        }
    }

    pub fn simulate_motion(&mut self) {
        // Simple tick function to alter data slightly if needed
        self.accel_x = self.accel_x.wrapping_add(10);
        self.gyro_z = self.gyro_z.wrapping_sub(5);
    }
}

impl I2cDevice for Mpu6050 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        // Read current register and auto-increment
        let val = self.read_register(self.current_register);
        self.current_register = self.current_register.wrapping_add(1);
        val
    }

    fn write(&mut self, data: u8) {
        if !self.register_address_written {
            // First byte written is the register address
            self.current_register = data;
            self.register_address_written = true;
        } else {
            // Subsequent bytes are data
            self.write_register(self.current_register, data);
            self.current_register = self.current_register.wrapping_add(1);
        }
    }

    fn stop(&mut self) {
        self.register_address_written = false;
    }
}
