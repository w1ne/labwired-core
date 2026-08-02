// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! ADS1115 16-bit I²C ADC (Texas Instruments).
//!
//! Models the register map used by Adafruit/Arduino drivers:
//! conversion (0x00), config (0x01), lo/hi threshold (0x02/0x03).
//! Host SimInput seeds single-ended channel voltages A0–A3 in volts;
//! conversion results follow the ±4.096 V FSR encoding (default PGA).

use crate::peripherals::i2c::I2cDevice;

const REG_CONVERSION: u8 = 0x00;
const REG_CONFIG: u8 = 0x01;
const REG_LO_THRESH: u8 = 0x02;
const REG_HI_THRESH: u8 = 0x03;

/// Default config after reset (OS=1, AIN0 single-ended, PGA ±2.048V, 128SPS, …).
/// We use ±4.096 V FSR (PGA=001) after a typical Adafruit begin() rewrite.
const DEFAULT_CONFIG: u16 = 0x8583;

pub struct Ads1115 {
    address: u8,
    current_register: u8,
    register_address_written: bool,
    write_high: Option<u8>,
    read_low_pending: Option<u8>,
    config: u16,
    lo_thresh: u16,
    hi_thresh: u16,
    /// Channel voltages in volts (A0–A3).
    channels: [f64; 4],
    component_id: Option<String>,
}

impl Default for Ads1115 {
    fn default() -> Self {
        Self::new(0x48)
    }
}

impl Ads1115 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            current_register: REG_CONVERSION,
            register_address_written: false,
            write_high: None,
            read_low_pending: None,
            config: DEFAULT_CONFIG,
            lo_thresh: 0x8000,
            hi_thresh: 0x7FFF,
            channels: [0.0; 4],
            component_id: None,
        }
    }

    pub fn set_channel_v(&mut self, ch: usize, volts: f64) {
        if ch < 4 {
            self.channels[ch] = volts.clamp(-4.096, 4.096);
        }
    }

    /// MUX bits [14:12] when single-ended AIN0–AIN3 (100–111).
    fn mux_channel(&self) -> usize {
        let mux = (self.config >> 12) & 0x07;
        match mux {
            0b100 => 0,
            0b101 => 1,
            0b110 => 2,
            0b111 => 3,
            // Differential pairs: approximate as ch0 for wave-1.
            _ => 0,
        }
    }

    /// PGA full-scale range in volts from config bits [11:9].
    fn fsr_v(&self) -> f64 {
        match (self.config >> 9) & 0x07 {
            0b000 => 6.144,
            0b001 => 4.096,
            0b010 => 2.048,
            0b011 => 1.024,
            0b100 => 0.512,
            _ => 0.256,
        }
    }

    fn conversion_raw(&self) -> u16 {
        let v = self.channels[self.mux_channel()];
        let fsr = self.fsr_v();
        let counts = ((v / fsr) * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
        counts as u16
    }

    fn read_register_u16(&self, reg: u8) -> u16 {
        match reg {
            REG_CONVERSION => self.conversion_raw(),
            REG_CONFIG => self.config | 0x8000, // OS = conversion ready
            REG_LO_THRESH => self.lo_thresh,
            REG_HI_THRESH => self.hi_thresh,
            _ => 0,
        }
    }

    fn write_register_u16(&mut self, reg: u8, value: u16) {
        match reg {
            REG_CONFIG => self.config = value,
            REG_LO_THRESH => self.lo_thresh = value,
            REG_HI_THRESH => self.hi_thresh = value,
            _ => {}
        }
    }
}

impl I2cDevice for Ads1115 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        if let Some(low) = self.read_low_pending.take() {
            return low;
        }
        let word = self.read_register_u16(self.current_register);
        self.read_low_pending = Some((word & 0xFF) as u8);
        (word >> 8) as u8
    }

    fn write(&mut self, data: u8) {
        if !self.register_address_written {
            self.current_register = data;
            self.register_address_written = true;
            self.write_high = None;
            self.read_low_pending = None;
            return;
        }
        match self.write_high {
            None => self.write_high = Some(data),
            Some(high) => {
                let word = (u16::from(high) << 8) | u16::from(data);
                self.write_register_u16(self.current_register, word);
                self.write_high = None;
            }
        }
    }

    fn stop(&mut self) {
        self.register_address_written = false;
        self.write_high = None;
        self.read_low_pending = None;
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
        key: "a0",
        label: "A0",
        unit: "V",
        min: -4.096,
        max: 4.096,
    },
    crate::sim_input::InputChannel {
        key: "a1",
        label: "A1",
        unit: "V",
        min: -4.096,
        max: 4.096,
    },
    crate::sim_input::InputChannel {
        key: "a2",
        label: "A2",
        unit: "V",
        min: -4.096,
        max: 4.096,
    },
    crate::sim_input::InputChannel {
        key: "a3",
        label: "A3",
        unit: "V",
        min: -4.096,
        max: 4.096,
    },
];

impl crate::sim_input::SimInput for Ads1115 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        let ch = match key {
            "a0" => 0,
            "a1" => 1,
            "a2" => 2,
            "a3" => 3,
            _ => unreachable!(),
        };
        self.set_channel_v(ch, value);
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

pub struct Ads1115Kit;
pub static ADS1115_KIT: Ads1115Kit = Ads1115Kit;

static ADS1115_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "ads1115",
    label: "ADS1115 16-bit ADC",
    summary: "4-channel 16-bit I²C ADC (TI ADS1115).",
    detail: "Texas Instruments ADS1115 (default address 0x48). Host stimulus sets A0–A3 \
             voltages; firmware reads conversion/config over I²C with the standard pointer \
             protocol used by Arduino ADS1X15 drivers.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x48 (ADDR→GND).",
    }],
    labs: &[],
};

impl PeripheralKit for Ads1115Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &ADS1115_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x48)?;
        ctx.attach_i2c_device(Box::new(Ads1115::new(address)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::i2c::I2cDevice;

    fn read_u16(dev: &mut Ads1115, reg: u8) -> u16 {
        dev.stop();
        dev.write(reg);
        let hi = dev.read();
        let lo = dev.read();
        (u16::from(hi) << 8) | u16::from(lo)
    }

    #[test]
    fn conversion_tracks_channel_voltage() {
        let mut dev = Ads1115::new(0x48);
        // Single-ended AIN0 (MUX=100), PGA ±4.096 (PGA=001) → 0xC383
        dev.config = 0xC383;
        dev.set_channel_v(0, 2.048);
        let raw = read_u16(&mut dev, REG_CONVERSION) as i16;
        // 2.048 / 4.096 * 32768 ≈ 16384
        assert!((raw - 16384).abs() < 20, "raw={raw}");
    }

    #[test]
    fn config_write_selects_channel() {
        let mut dev = Ads1115::new(0x48);
        dev.set_channel_v(2, 1.0);
        // MUX AIN2 single-ended = 110
        dev.stop();
        dev.write(REG_CONFIG);
        dev.write(0xE1); // high: OS + MUX110 + PGA001…
        dev.write(0x83);
        dev.stop();
        assert_eq!(dev.mux_channel(), 2);
    }
}
