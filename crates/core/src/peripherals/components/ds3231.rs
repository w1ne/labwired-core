// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! DS3231 I²C real-time clock (Maxim).
//!
//! Models BCD timekeeping registers 0x00–0x06 (seconds…year) and control/
//! status (0x0E/0x0F) enough for common Arduino RTC libraries. Host can
//! seed calendar fields via SimInput; the model does not free-run wall time
//! unless advanced by tests — reads return the last set values.

use crate::peripherals::i2c::I2cDevice;

fn to_bcd(v: u8) -> u8 {
    ((v / 10) << 4) | (v % 10)
}

fn from_bcd(v: u8) -> u8 {
    ((v >> 4) * 10) + (v & 0x0F)
}

pub struct Ds3231 {
    address: u8,
    current_register: u8,
    register_address_written: bool,
    /// Time: sec, min, hour, dow, date, month, year (0–99)
    time: [u8; 7],
    control: u8,
    status: u8,
    aging: u8,
    temp_msb: u8,
    temp_lsb: u8,
    component_id: Option<String>,
}

impl Default for Ds3231 {
    fn default() -> Self {
        Self::new(0x68)
    }
}

impl Ds3231 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            current_register: 0,
            register_address_written: false,
            // 2026-07-22 12:00:00 Wednesday (dow=4 if Sun=1…Sat=7 → Wed=4)
            time: [0, 0, 12, 4, 22, 7, 26],
            control: 0x1C,
            status: 0x00,
            aging: 0,
            temp_msb: 25, // 25 °C
            temp_lsb: 0,
            component_id: None,
        }
    }

    pub fn set_time(&mut self, sec: u8, min: u8, hour: u8, dow: u8, date: u8, month: u8, year: u8) {
        self.time = [
            sec.min(59),
            min.min(59),
            hour.min(23),
            dow.clamp(1, 7),
            date.clamp(1, 31),
            month.clamp(1, 12),
            year.min(99),
        ];
    }

    fn read_reg(&self, reg: u8) -> u8 {
        match reg {
            0x00..=0x06 => to_bcd(self.time[reg as usize]),
            0x07..=0x0D => 0, // alarms — unset
            0x0E => self.control,
            0x0F => self.status,
            0x10 => self.aging,
            0x11 => self.temp_msb,
            0x12 => self.temp_lsb,
            _ => 0,
        }
    }

    fn write_reg(&mut self, reg: u8, value: u8) {
        match reg {
            0x00..=0x06 => {
                let mut v = from_bcd(value & 0x7F);
                match reg {
                    0x00 | 0x01 => v = v.min(59),
                    0x02 => v = v.min(23),
                    0x03 => v = v.clamp(1, 7),
                    0x04 => v = v.clamp(1, 31),
                    0x05 => v = (v & 0x1F).clamp(1, 12),
                    0x06 => v = v.min(99),
                    _ => {}
                }
                self.time[reg as usize] = v;
            }
            0x0E => self.control = value,
            0x0F => self.status = value & !0x80, // OSF clearable
            0x10 => self.aging = value,
            _ => {}
        }
    }
}

impl I2cDevice for Ds3231 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        let value = self.read_reg(self.current_register);
        self.current_register = self.current_register.wrapping_add(1);
        value
    }

    fn write(&mut self, data: u8) {
        if !self.register_address_written {
            self.current_register = data;
            self.register_address_written = true;
        } else {
            self.write_reg(self.current_register, data);
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
        key: "unix_time",
        label: "Unix time",
        unit: "s",
        min: 0.0,
        max: 4_102_444_800.0, // ~2100
    },
    crate::sim_input::InputChannel {
        key: "temperature",
        label: "Temperature",
        unit: "C",
        min: -40.0,
        max: 85.0,
    },
];

impl crate::sim_input::SimInput for Ds3231 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "unix_time" => {
                // Compact civil conversion from Unix seconds (UTC), enough for demos.
                let mut days = (value as i64) / 86_400;
                let mut rem = (value as i64).rem_euclid(86_400) as u32;
                let hour = (rem / 3600) as u8;
                rem %= 3600;
                let min = (rem / 60) as u8;
                let sec = (rem % 60) as u8;
                // 1970-01-01 was Thursday = 5 if Sun=1
                let mut dow = (((days + 4) % 7) + 1) as u8;
                if dow == 0 {
                    dow = 7;
                }
                // Approximate Y/M/D via civil algorithm (Howard Hinnant)
                days += 719_468;
                let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
                let doe = (days - era * 146_097) as u64;
                let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
                let y = (yoe as i64) + era * 400;
                let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
                let mp = (5 * doy + 2) / 153;
                let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
                let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
                let y = if m <= 2 { y + 1 } else { y };
                let year = ((y % 100) as u8).min(99);
                self.set_time(sec, min, hour, dow, d.max(1), m.max(1), year);
            }
            "temperature" => {
                let t = value.round() as i16;
                self.temp_msb = t as u8;
                self.temp_lsb = 0;
            }
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

pub struct Ds3231Kit;
pub static DS3231_KIT: Ds3231Kit = Ds3231Kit;

static DS3231_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "ds3231",
    label: "DS3231 RTC",
    summary: "I²C real-time clock with temperature-compensated crystal.",
    detail: "Maxim DS3231 (address 0x68). BCD time registers 0x00–0x06 and control/status \
             for common Arduino RTC libraries. Host can set unix_time or temperature via SimInput.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x68.",
    }],
    labs: &[],
};

impl PeripheralKit for Ds3231Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &DS3231_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x68)?;
        ctx.attach_i2c_device(Box::new(Ds3231::new(address)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::i2c::I2cDevice;

    #[test]
    fn bcd_time_roundtrip() {
        let mut dev = Ds3231::new(0x68);
        dev.set_time(45, 30, 14, 3, 15, 6, 26);
        dev.stop();
        dev.write(0x00);
        assert_eq!(from_bcd(dev.read()), 45); // sec
        assert_eq!(from_bcd(dev.read()), 30); // min
        assert_eq!(from_bcd(dev.read()), 14); // hour
    }

    #[test]
    fn write_seconds_register() {
        let mut dev = Ds3231::new(0x68);
        dev.stop();
        dev.write(0x00);
        dev.write(to_bcd(12));
        dev.stop();
        dev.write(0x00);
        assert_eq!(from_bcd(dev.read()), 12);
    }
}
