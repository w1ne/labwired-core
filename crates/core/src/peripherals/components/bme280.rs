// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::i2c::I2cDevice;

/// BME280 Environmental Sensor I²C Component (static factory values).
///
/// Returns hard-coded Bosch calibration coefficients and raw ADC readings that
/// compensate to approximately 25 °C / 50 %RH / 1013 hPa using the standard
/// Bosch BME280 fixed-point compensation algorithm.
#[derive(Debug, serde::Serialize)]
pub struct Bme280 {
    address: u8,
    current_register: u8,
    register_address_written: bool,

    // Writable control registers
    pub ctrl_hum: u8,
    pub ctrl_meas: u8,
    pub config: u8,
}

impl Default for Bme280 {
    fn default() -> Self {
        Self::new(0x76) // Default I²C address for BME280
    }
}

impl Bme280 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            current_register: 0,
            register_address_written: false,
            ctrl_hum: 0,
            ctrl_meas: 0,
            config: 0,
        }
    }

    fn read_register(&self, reg: u8) -> u8 {
        match reg {
            // Calibration coefficients (T) — dig_T1=28528, dig_T2=26435, dig_T3=-1000
            0x88 => 0x70,
            0x89 => 0x6F, // dig_T1 LSB, MSB
            0x8A => 0x43,
            0x8B => 0x67, // dig_T2
            0x8C => 0x18,
            0x8D => 0xFC, // dig_T3

            // Calibration (P) — Bosch reference coefficients
            0x8E => 0x4D,
            0x8F => 0x95, // dig_P1
            0x90 => 0xAF,
            0x91 => 0xD6, // dig_P2
            0x92 => 0xD0,
            0x93 => 0x0B, // dig_P3
            0x94 => 0xFD,
            0x95 => 0x1C, // dig_P4
            0x96 => 0x47,
            0x97 => 0xFF, // dig_P5
            0x98 => 0xF9,
            0x99 => 0xFF, // dig_P6
            0x9A => 0xAC,
            0x9B => 0x26, // dig_P7
            0x9C => 0x0A,
            0x9D => 0xD8, // dig_P8
            0x9E => 0xBD,
            0x9F => 0x10, // dig_P9

            // Chip ID
            0xD0 => 0x60, // BME280

            // Calibration (H)
            0xA1 => 0x4B, // dig_H1 = 75
            0xE1 => 0x5E,
            0xE2 => 0x01, // dig_H2 = 350
            0xE3 => 0x00, // dig_H3 = 0
            0xE4 => 0x13,
            0xE5 => 0x05, // dig_H4 / H5 share E5
            0xE6 => 0x00, // dig_H5 cont
            0xE7 => 0x1E, // dig_H6 = 30

            // Status: never measuring, data always available
            0xF3 => 0x00,

            // Control registers return what was last written
            0xF2 => self.ctrl_hum,
            0xF4 => self.ctrl_meas,
            0xF5 => self.config,

            // Raw measurement registers (msb, lsb, xlsb)
            // press_adc = 0x52E6F (upper 20 bits of [F7..F9])
            0xF7 => 0x52,
            0xF8 => 0xE6,
            0xF9 => 0xF0,
            // temp_adc  = 0x82BAC (upper 20 bits of [FA..FC])
            0xFA => 0x82,
            0xFB => 0xBA,
            0xFC => 0xC0,
            // hum_adc   = 0x7CCC (16-bit [FD..FE])
            0xFD => 0x7C,
            0xFE => 0xCC,

            _ => 0,
        }
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            0xF2 => self.ctrl_hum = value,
            0xF4 => self.ctrl_meas = value,
            0xF5 => self.config = value,
            0xE0 => { /* soft reset 0xB6 — ignore */ }
            _ => {}
        }
    }
}

impl I2cDevice for Bme280 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        let val = self.read_register(self.current_register);
        self.current_register = self.current_register.wrapping_add(1);
        val
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
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct Bme280Kit;
pub static BME280_KIT: Bme280Kit = Bme280Kit;

static BME280_METADATA: KitMetadata = KitMetadata {
    device_type: "bme280",
    label: "BME280 Weather",
    summary: "Bosch BME280 temp + humidity + pressure sensor over I2C.",
    detail: "Returns the same factory dummy readings the real silicon ships with (≈25 °C, \
             50 % RH, sea-level pressure). Real-time stimulus comes from the WASM bridge.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x76; 0x77 selects the SDO=VDDIO variant.",
    }],
    labs: &[LabRef {
        board_id: "bme280-weather-lab",
        chip: "stm32f103",
        example_dir: "bme280-weather-lab",
        demo_elf: "demo-bme280-weather-lab.elf",
    }],
};

impl PeripheralKit for Bme280Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &BME280_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x76)?;
        ctx.attach_i2c_device(Box::new(Bme280::new(address)))?;
        Ok(())
    }
}
