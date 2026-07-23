// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! BMP280 I²C pressure + temperature sensor as an [`I2cDevice`].
//!
//! Bosch BMP280 datasheet summary:
//! - 7-bit address `0x76` (SDO=GND, default) or `0x77` (SDO=VCC).
//! - Register-based protocol with auto-incrementing pointer:
//!   master writes the register address byte, then subsequent reads
//!   sequentially return that register and the ones following it.
//! - Register map (subset modeled here):
//!   - `0x88..=0xA1` — calibration data (24 bytes, little-endian shorts)
//!   - `0xD0` — chip ID (`0x58`)
//!   - `0xE0` — soft reset (write `0xB6`)
//!   - `0xF3` — status (bit 3 = measuring, bit 0 = im_update)
//!   - `0xF4` — ctrl_meas (`osrs_t<<5 | osrs_p<<2 | mode`)
//!   - `0xF5` — config (`t_sb<<5 | filter<<2 | spi3w_en`)
//!   - `0xF7..=0xF9` — uncomp pressure (20-bit, MSB-first)
//!   - `0xFA..=0xFC` — uncomp temperature (20-bit, MSB-first)
//!
//! Calibration constants below are a representative real-silicon sample
//! per the Bosch reference datasheet; firmware compensation math will
//! produce sensible (~25 °C / ~100 kPa) outputs from the fixed raw ADC
//! values modeled here.

use crate::peripherals::i2c::I2cDevice;

const BMP280_ADDR_DEFAULT: u8 = 0x76;
const CHIP_ID: u8 = 0x58;

/// Fixed raw ADC samples — middle-of-range placeholders. Real values come
/// later from the F407 hardware oracle capture.
const ADC_T: u32 = 0x80000;
const ADC_P: u32 = 0x80000;

/// Bosch reference calibration block. Stored little-endian over registers
/// 0x88..0xA0. dig_T1..3 then dig_P1..9; T1 + P1 are unsigned, rest signed.
const CALIB: [u8; 24] = [
    0x70, 0x6B, // dig_T1 = 27504 (u16)
    0x43, 0x67, // dig_T2 = 26435 (i16)
    0x18, 0xFC, // dig_T3 = -1000 (i16)
    0x7D, 0x8E, // dig_P1 = 36477 (u16)
    0xA3, 0xD5, // dig_P2 = -10685 (i16)
    0xD0, 0x0B, // dig_P3 = 3024
    0x27, 0x0B, // dig_P4 = 2855
    0x8C, 0x00, // dig_P5 = 140
    0xF9, 0xFF, // dig_P6 = -7
    0x8C, 0x3C, // dig_P7 = 15500
    0xF8, 0xC6, // dig_P8 = -14600
    0x70, 0x17, // dig_P9 = 6000
];

#[derive(Debug)]
pub struct Bmp280 {
    address: u8,
    /// Auto-incrementing register pointer set on the first write of each
    /// transaction. Wraps within a u8.
    pointer: u8,
    /// True once the master has written a register address since `start()`.
    /// Further writes go to that register; reads return register data.
    pointer_set: bool,
    ctrl_meas: u8,
    config: u8,
    status: u8,
}

impl Bmp280 {
    pub fn new(address: u8) -> Self {
        let addr = if address == 0 {
            BMP280_ADDR_DEFAULT
        } else {
            address
        };
        Self {
            address: addr,
            pointer: 0,
            pointer_set: false,
            ctrl_meas: 0,
            config: 0,
            status: 0,
        }
    }

    /// Read the byte at `reg`. Used during sequential reads after the
    /// master has set the pointer with a write.
    fn read_register(&self, reg: u8) -> u8 {
        match reg {
            0x88..=0x9F => CALIB[(reg - 0x88) as usize],
            0xA0 => 0x00, // CALIB last byte (reserved per datasheet)
            0xA1 => 0x00,
            0xD0 => CHIP_ID,
            0xF3 => self.status,
            0xF4 => self.ctrl_meas,
            0xF5 => self.config,
            // Pressure (20-bit big-endian over F7..F9):
            0xF7 => ((ADC_P >> 12) & 0xFF) as u8,
            0xF8 => ((ADC_P >> 4) & 0xFF) as u8,
            0xF9 => ((ADC_P & 0x0F) << 4) as u8,
            // Temperature (20-bit big-endian over FA..FC):
            0xFA => ((ADC_T >> 12) & 0xFF) as u8,
            0xFB => ((ADC_T >> 4) & 0xFF) as u8,
            0xFC => ((ADC_T & 0x0F) << 4) as u8,
            _ => 0xFF,
        }
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            0xE0 if value == 0xB6 => {
                // Soft reset — clear writable state.
                self.ctrl_meas = 0;
                self.config = 0;
                self.status = 0;
            }
            0xF4 => self.ctrl_meas = value,
            0xF5 => self.config = value,
            _ => {} // Calibration block / chip ID / status / data regs are read-only.
        }
    }
}

impl Default for Bmp280 {
    fn default() -> Self {
        Self::new(BMP280_ADDR_DEFAULT)
    }
}

impl I2cDevice for Bmp280 {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        self.pointer_set = false;
    }

    fn write(&mut self, data: u8) {
        if !self.pointer_set {
            self.pointer = data;
            self.pointer_set = true;
        } else {
            self.write_register(self.pointer, data);
            self.pointer = self.pointer.wrapping_add(1);
        }
    }

    fn read(&mut self) -> u8 {
        let byte = self.read_register(self.pointer);
        self.pointer = self.pointer.wrapping_add(1);
        byte
    }
}

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Bmp280Kit;
pub static BMP280_KIT: Bmp280Kit = Bmp280Kit;

static BMP280_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "bmp280",
    label: "BMP280 Pressure",
    summary: "Bosch BMP280 I²C pressure + temperature sensor (no humidity).",
    detail: "Register map compatible with common Arduino BMP280 libraries. \
             CHIP_ID 0x58 at 0xD0; fixed ADC sample data for deterministic labs.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x76 (0x77 alternate).",
    }],
    labs: &[],
};

impl PeripheralKit for Bmp280Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &BMP280_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x76)?;
        ctx.attach_i2c_device(Box::new(Bmp280::new(address)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_address_is_0x76() {
        assert_eq!(Bmp280::default().address(), 0x76);
    }

    #[test]
    fn alternate_address_0x77_honored() {
        let d = Bmp280::new(0x77);
        assert_eq!(d.address(), 0x77);
    }

    #[test]
    fn chip_id_returns_0x58() {
        let mut d = Bmp280::default();
        // Master: write pointer 0xD0, then read.
        d.start();
        d.write(0xD0);
        // Repeated-start to switch to read direction.
        d.start();
        // Pointer state must persist across the repeated-start; the BMP280
        // datasheet says only the leading byte of a write transaction sets
        // the pointer, so a repeated-start read uses the previously-set
        // pointer. We model this by carrying `pointer` across `start()`.
        assert_eq!(d.read(), CHIP_ID);
    }

    #[test]
    fn calibration_block_is_24_bytes_starting_at_0x88() {
        let mut d = Bmp280::default();
        d.start();
        d.write(0x88);
        d.start();
        let block: Vec<u8> = (0..24).map(|_| d.read()).collect();
        assert_eq!(block, CALIB);
    }

    #[test]
    fn ctrl_meas_round_trips() {
        let mut d = Bmp280::default();
        d.start();
        d.write(0xF4);
        d.write(0x27); // osrs_t=001, osrs_p=001, mode=11 (normal)
        d.start();
        d.write(0xF4);
        d.start();
        assert_eq!(d.read(), 0x27);
    }

    #[test]
    fn soft_reset_clears_ctrl_meas() {
        let mut d = Bmp280::default();
        d.start();
        d.write(0xF4);
        d.write(0x27);
        // Soft reset
        d.start();
        d.write(0xE0);
        d.write(0xB6);
        // Read back ctrl_meas
        d.start();
        d.write(0xF4);
        d.start();
        assert_eq!(d.read(), 0x00);
    }

    #[test]
    fn temperature_block_layout_matches_adc_t() {
        let mut d = Bmp280::default();
        d.start();
        d.write(0xFA);
        d.start();
        let msb = d.read();
        let lsb = d.read();
        let xlsb = d.read();
        // Reconstruct 20-bit value the way firmware would.
        let raw = ((msb as u32) << 12) | ((lsb as u32) << 4) | ((xlsb as u32) >> 4);
        assert_eq!(raw, ADC_T);
    }
}
