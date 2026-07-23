// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! INA219 high-side current / bus voltage monitor (I²C).
//!
//! Models the register map used by common Arduino/Adafruit drivers:
//! config (0x00), shunt (0x01), bus (0x02), power (0x03), current (0x04),
//! calibration (0x05). Host SimInput seeds bus voltage and current; raw
//! register values follow the TI data sheet encoding at a fixed calibration
//! so firmware reads stay stable across sketches.

use crate::peripherals::i2c::I2cDevice;

const REG_CONFIG: u8 = 0x00;
const REG_SHUNT: u8 = 0x01;
const REG_BUS: u8 = 0x02;
const REG_POWER: u8 = 0x03;
const REG_CURRENT: u8 = 0x04;
const REG_CAL: u8 = 0x05;

/// Default reset configuration (TI datasheet).
const DEFAULT_CONFIG: u16 = 0x399F;

/// Fixed calibration used by the model so current_LSB is 0.1 mA (100 µA).
/// Matches a common Adafruit init path (cal = 4096 for 0.1 Ω shunt).
const DEFAULT_CAL: u16 = 4096;

pub struct Ina219 {
    address: u8,
    current_register: u8,
    register_address_written: bool,
    /// After a register pointer write, the next two data bytes are a 16-bit write.
    write_high: Option<u8>,
    /// After a pointer set, reads return high then low of the selected register.
    read_low_pending: Option<u8>,
    config: u16,
    calibration: u16,
    /// Bus voltage in millivolts (0–32000 typical).
    bus_mv: u16,
    /// Signed current in milliamps.
    current_ma: i16,
    component_id: Option<String>,
}

impl Default for Ina219 {
    fn default() -> Self {
        Self::new(0x40)
    }
}

impl Ina219 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            current_register: REG_CONFIG,
            register_address_written: false,
            write_high: None,
            read_low_pending: None,
            config: DEFAULT_CONFIG,
            calibration: DEFAULT_CAL,
            bus_mv: 3300,
            current_ma: 0,
            component_id: None,
        }
    }

    pub fn set_bus_mv(&mut self, mv: u16) {
        self.bus_mv = mv.min(32_000);
    }

    pub fn set_current_ma(&mut self, ma: i16) {
        self.current_ma = ma;
    }

    /// Bus voltage register: bits 15:3 = voltage in 4 mV units, bit 1 CNVR, bit 0 OVF.
    fn bus_register(&self) -> u16 {
        let counts = (self.bus_mv / 4).min(0x1FFF);
        (counts << 3) | 0b10 // CNVR set
    }

    /// Shunt voltage: 10 µV / bit signed. For 0.1 Ω shunt, Vshunt_mV = I_mA * 0.1.
    fn shunt_register(&self) -> u16 {
        let vshunt_uv = i32::from(self.current_ma) * 100; // mA * 0.1Ω → µV
        let counts = (vshunt_uv / 10).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
        counts as u16
    }

    /// Current register: with cal=4096 and 0.1Ω, current_LSB ≈ 0.1 mA.
    fn current_register_value(&self) -> u16 {
        let counts = (f64::from(self.current_ma) / 0.1).round() as i32;
        counts.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16 as u16
    }

    /// Power register: power_LSB = 20 * current_LSB → 2 mW; value in those units.
    fn power_register(&self) -> u16 {
        let power_mw = (i32::from(self.bus_mv) * i32::from(self.current_ma.abs())) / 1000;
        let counts = (power_mw as f64 / 2.0).round() as i32;
        counts.clamp(0, 0xFFFF) as u16
    }

    fn read_register_u16(&self, reg: u8) -> u16 {
        match reg {
            REG_CONFIG => self.config,
            REG_SHUNT => self.shunt_register(),
            REG_BUS => self.bus_register(),
            REG_POWER => self.power_register(),
            REG_CURRENT => self.current_register_value(),
            REG_CAL => self.calibration,
            _ => 0,
        }
    }

    fn write_register_u16(&mut self, reg: u8, value: u16) {
        match reg {
            REG_CONFIG => self.config = value,
            REG_CAL => self.calibration = value,
            _ => {}
        }
    }
}

impl I2cDevice for Ina219 {
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
        key: "bus_voltage",
        label: "Bus voltage",
        unit: "V",
        min: 0.0,
        max: 32.0,
    },
    crate::sim_input::InputChannel {
        key: "current",
        label: "Current",
        unit: "A",
        min: -3.2,
        max: 3.2,
    },
];

impl crate::sim_input::SimInput for Ina219 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "bus_voltage" => self.set_bus_mv((value * 1000.0).round().clamp(0.0, 32_000.0) as u16),
            "current" => {
                let ma = (value * 1000.0).round().clamp(-32_000.0, 32_000.0) as i16;
                self.set_current_ma(ma);
            }
            _ => unreachable!("require_channel validated the key"),
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

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Ina219Kit;
pub static INA219_KIT: Ina219Kit = Ina219Kit;

static INA219_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "ina219",
    label: "INA219 Power Monitor",
    summary: "High-side current and bus voltage sensor over I²C.",
    detail: "Texas Instruments INA219 (default 7-bit address 0x40). Host stimulus sets bus \
             voltage and load current; firmware reads config/shunt/bus/current/power/cal \
             registers on the simulated I²C bus.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x40 (A0/A1 ground).",
    }],
    labs: &[],
};

impl PeripheralKit for Ina219Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &INA219_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x40)?;
        ctx.attach_i2c_device(Box::new(Ina219::new(address)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::i2c::I2cDevice;

    fn read_u16(dev: &mut Ina219, reg: u8) -> u16 {
        dev.stop();
        dev.write(reg);
        let hi = dev.read();
        let lo = dev.read();
        (u16::from(hi) << 8) | u16::from(lo)
    }

    #[test]
    fn default_config_and_bus_voltage() {
        let mut dev = Ina219::new(0x40);
        assert_eq!(dev.address(), 0x40);
        assert_eq!(read_u16(&mut dev, REG_CONFIG), DEFAULT_CONFIG);
        // 3300 mV → 825 counts * 4 mV, CNVR set
        assert_eq!(read_u16(&mut dev, REG_BUS), (825 << 3) | 0b10);
    }

    #[test]
    fn current_and_shunt_track_sim_input() {
        let mut dev = Ina219::new(0x40);
        dev.set_current_ma(100); // 100 mA
        let current = read_u16(&mut dev, REG_CURRENT) as i16;
        assert!((current - 1000).abs() < 5, "current counts={current}");
        let shunt = read_u16(&mut dev, REG_SHUNT) as i16;
        // 100 mA * 0.1 Ω = 10 mV = 1000 * 10 µV counts
        assert!((shunt - 1000).abs() < 5, "shunt counts={shunt}");
    }

    #[test]
    fn calibration_write_roundtrips() {
        let mut dev = Ina219::new(0x40);
        dev.stop();
        dev.write(REG_CAL);
        dev.write(0x10);
        dev.write(0x00);
        dev.stop();
        assert_eq!(read_u16(&mut dev, REG_CAL), 0x1000);
    }
}
