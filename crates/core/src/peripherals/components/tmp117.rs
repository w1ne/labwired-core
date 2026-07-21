// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Texas Instruments **TMP117** high-accuracy digital temperature sensor as an
//! [`I2cDevice`].
//!
//! ## Register access (datasheet §7.5)
//! The TMP117 is a pointer-addressed I²C device with **16-bit big-endian**
//! registers. A transaction writes the 1-byte pointer, then either writes a
//! 16-bit value (MSB then LSB) to that register, or reads it back MSB first
//! then LSB. Registers modelled here:
//!
//! | Ptr  | Register        | Notes                                            |
//! |------|-----------------|--------------------------------------------------|
//! | 0x00 | `TEMP_RESULT`   | int16, **7.8125 m°C / LSB** (0x0000 = 0 °C), R/O  |
//! | 0x01 | `CONFIGURATION` | DATA_READY (bit13) + conversion-mode bits        |
//! | 0x02 | `T_HIGH_LIMIT`  | int16, same scale                                |
//! | 0x03 | `T_LOW_LIMIT`   | int16, same scale                                |
//! | 0x04 | `EEPROM_UL`     | readback storage                                 |
//! | 0x05 | `EEPROM1`       | readback storage                                 |
//! | 0x06 | `EEPROM2`       | readback storage                                 |
//! | 0x07 | `TEMP_OFFSET`   | readback storage                                 |
//! | 0x08 | `EEPROM3`       | readback storage                                 |
//! | 0x0F | `DEVICE_ID`     | `0x0117` (rev 0x0 in bits[15:12]), R/O           |
//!
//! The temperature is an externally driven variable: it changes only when a
//! driver sets it through the ONE stimulus contract,
//! [`crate::sim_input::SimInput`] (channel `temperature`, °C). Setting it marks
//! a new conversion available (`DATA_READY`), which clears when the firmware
//! reads `TEMP_RESULT`.

use crate::peripherals::i2c::I2cDevice;

/// Default 7-bit address (ADD0 → GND). Also valid: 0x49/0x4A/0x4B.
pub const TMP117_ADDR: u8 = 0x48;
/// `DEVICE_ID` value: 0x0117 in bits[11:0], revision 0x0 in bits[15:12].
pub const TMP117_DEVICE_ID: u16 = 0x0117;

/// Temperature LSB weight: 7.8125 m°C per count.
const LSB_C: f64 = 0.0078125;

/// DATA_READY flag in `CONFIGURATION`, bit 13.
const CFG_DATA_READY: u16 = 1 << 13;
/// Power-on reset value of `CONFIGURATION` (continuous-conversion mode).
const CFG_RESET: u16 = 0x0220;

// Register pointers.
const REG_TEMP_RESULT: u8 = 0x00;
const REG_CONFIGURATION: u8 = 0x01;
const REG_T_HIGH_LIMIT: u8 = 0x02;
const REG_T_LOW_LIMIT: u8 = 0x03;
const REG_EEPROM_UL: u8 = 0x04;
const REG_EEPROM1: u8 = 0x05;
const REG_EEPROM2: u8 = 0x06;
const REG_TEMP_OFFSET: u8 = 0x07;
const REG_EEPROM3: u8 = 0x08;
const REG_DEVICE_ID: u8 = 0x0F;

/// Encode a Celsius temperature into the TMP117's int16 count (7.8125 m°C/LSB).
fn celsius_to_raw(t_c: f64) -> i16 {
    (t_c / LSB_C)
        .round()
        .clamp(i16::MIN as f64, i16::MAX as f64) as i16
}

/// Decode an int16 count back to Celsius (used by tests / inspection).
#[cfg(test)]
fn raw_to_celsius(raw: i16) -> f64 {
    raw as f64 * LSB_C
}

/// Register-level TMP117 model.
pub struct Tmp117 {
    address: u8,
    /// `TEMP_RESULT` in raw counts. Externally driven.
    temp_raw: i16,
    /// Stored `CONFIGURATION` bits (DATA_READY reflected dynamically on read).
    config: u16,
    t_high: i16,
    t_low: i16,
    eeprom_ul: u16,
    eeprom1: u16,
    eeprom2: u16,
    eeprom3: u16,
    temp_offset: u16,
    /// Set when a new conversion result is available; cleared on TEMP_RESULT read.
    data_ready: bool,
    /// Selected register pointer.
    pointer: u8,
    /// Read phase: 0 = next read yields MSB; 1 = next read yields (and holds) LSB.
    read_phase: u8,
    /// Writes since the framing START: 0 sets the pointer, 1 latches the MSB,
    /// 2 stores the 16-bit value into the pointed register.
    writes_since_start: u32,
    /// MSB latched by the second write, awaiting the LSB to form a 16-bit value.
    pending_msb: u8,
    /// `external_devices` id, stamped at attach.
    component_id: Option<String>,
}

impl Tmp117 {
    /// `address == 0` selects the default 0x48.
    pub fn new(address: u8) -> Self {
        let address = if address == 0 { TMP117_ADDR } else { address };
        Self {
            address,
            temp_raw: 0,
            config: CFG_RESET,
            t_high: celsius_to_raw(192.0), // datasheet POR default
            t_low: celsius_to_raw(-256.0),
            eeprom_ul: 0,
            eeprom1: 0,
            eeprom2: 0,
            eeprom3: 0,
            temp_offset: 0,
            data_ready: false,
            pointer: REG_TEMP_RESULT,
            read_phase: 0,
            writes_since_start: 0,
            pending_msb: 0,
            component_id: None,
        }
    }

    /// The 16-bit value currently readable at `ptr` (DATA_READY reflected).
    fn reg_value(&self, ptr: u8) -> u16 {
        match ptr {
            REG_TEMP_RESULT => self.temp_raw as u16,
            REG_CONFIGURATION => {
                let mut c = self.config & !CFG_DATA_READY;
                if self.data_ready {
                    c |= CFG_DATA_READY;
                }
                c
            }
            REG_T_HIGH_LIMIT => self.t_high as u16,
            REG_T_LOW_LIMIT => self.t_low as u16,
            REG_EEPROM_UL => self.eeprom_ul,
            REG_EEPROM1 => self.eeprom1,
            REG_EEPROM2 => self.eeprom2,
            REG_TEMP_OFFSET => self.temp_offset,
            REG_EEPROM3 => self.eeprom3,
            REG_DEVICE_ID => TMP117_DEVICE_ID,
            _ => 0,
        }
    }

    /// Store a 16-bit value into `ptr`. `TEMP_RESULT` and `DEVICE_ID` are
    /// read-only and silently ignore writes.
    fn store_reg(&mut self, ptr: u8, value: u16) {
        match ptr {
            // DATA_READY is a status bit, not host-writable.
            REG_CONFIGURATION => self.config = value & !CFG_DATA_READY,
            REG_T_HIGH_LIMIT => self.t_high = value as i16,
            REG_T_LOW_LIMIT => self.t_low = value as i16,
            REG_EEPROM_UL => self.eeprom_ul = value,
            REG_EEPROM1 => self.eeprom1 = value,
            REG_EEPROM2 => self.eeprom2 = value,
            REG_TEMP_OFFSET => self.temp_offset = value,
            REG_EEPROM3 => self.eeprom3 = value,
            // TEMP_RESULT (0x00) and DEVICE_ID (0x0F) are read-only.
            _ => {}
        }
    }
}

impl Default for Tmp117 {
    fn default() -> Self {
        Self::new(TMP117_ADDR)
    }
}

impl I2cDevice for Tmp117 {
    fn address(&self) -> u8 {
        self.address
    }

    fn write(&mut self, data: u8) {
        match self.writes_since_start {
            0 => {
                self.pointer = data;
                self.read_phase = 0;
            }
            1 => self.pending_msb = data,
            2 => {
                let value = ((self.pending_msb as u16) << 8) | data as u16;
                self.store_reg(self.pointer, value);
            }
            _ => {}
        }
        self.writes_since_start = self.writes_since_start.saturating_add(1);
    }

    fn read(&mut self) -> u8 {
        let value = self.reg_value(self.pointer);
        if self.read_phase == 0 {
            self.read_phase = 1;
            (value >> 8) as u8 // MSB first (big-endian)
        } else {
            // LSB. Reading TEMP_RESULT clears DATA_READY per the datasheet.
            if self.pointer == REG_TEMP_RESULT {
                self.data_ready = false;
            }
            (value & 0xFF) as u8
        }
    }

    fn start(&mut self) {
        self.read_phase = 0;
        self.writes_since_start = 0;
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

/// Drivable channels. Range is the TMP117's specified operating span.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[crate::sim_input::InputChannel {
    key: "temperature",
    label: "Temperature",
    unit: "°C",
    min: -55.0,
    max: 150.0,
}];

impl crate::sim_input::SimInput for Tmp117 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "temperature" => {
                self.temp_raw = celsius_to_raw(value);
                self.data_ready = true; // a new conversion result is available
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_input::SimInput;

    /// Read a 16-bit register big-endian: point, (repeated) START, MSB, LSB.
    fn read_reg(d: &mut Tmp117, ptr: u8) -> u16 {
        d.start();
        d.write(ptr);
        d.start();
        let msb = d.read() as u16;
        let lsb = d.read() as u16;
        (msb << 8) | lsb
    }

    /// Write a 16-bit value big-endian: point + MSB + LSB in one framed write.
    fn write_reg(d: &mut Tmp117, ptr: u8, value: u16) {
        d.start();
        d.write(ptr);
        d.write((value >> 8) as u8);
        d.write((value & 0xFF) as u8);
        d.stop();
    }

    #[test]
    fn address_defaults_to_0x48() {
        assert_eq!(Tmp117::new(0).address(), 0x48);
        assert_eq!(Tmp117::default().address(), 0x48);
        assert_eq!(Tmp117::new(0x4A).address(), 0x4A);
    }

    #[test]
    fn device_id_is_0x0117_msb_then_lsb() {
        let mut d = Tmp117::new(TMP117_ADDR);
        d.start();
        d.write(REG_DEVICE_ID);
        d.start();
        let msb = d.read();
        let lsb = d.read();
        assert_eq!(msb, 0x01, "MSB first");
        assert_eq!(lsb, 0x17, "then LSB");
        assert_eq!(((msb as u16) << 8) | lsb as u16, TMP117_DEVICE_ID);
    }

    #[test]
    fn temperature_round_trips_within_one_lsb() {
        let mut d = Tmp117::new(TMP117_ADDR);
        d.set_input("temperature", 30.0).unwrap();
        let raw = read_reg(&mut d, REG_TEMP_RESULT) as i16;
        let decoded = raw as f64 * LSB_C;
        assert!((decoded - 30.0).abs() <= LSB_C, "decoded {decoded:.4} °C");
        // 30 °C / 0.0078125 = 3840 exactly.
        assert_eq!(raw, 3840);
    }

    #[test]
    fn negative_temperature_encodes_signed() {
        let mut d = Tmp117::new(TMP117_ADDR);
        d.set_input("temperature", -10.0).unwrap();
        let raw = read_reg(&mut d, REG_TEMP_RESULT) as i16;
        assert!(raw < 0, "sign preserved, raw = {raw}");
        // -10 °C / 0.0078125 = -1280 exactly → 0xFB00 two's complement.
        assert_eq!(raw, -1280);
        assert_eq!(raw as u16, 0xFB00);
        let decoded = raw as f64 * LSB_C;
        assert!((decoded + 10.0).abs() <= LSB_C, "decoded {decoded:.4} °C");
    }

    #[test]
    fn temp_result_is_big_endian_msb_first() {
        let mut d = Tmp117::new(TMP117_ADDR);
        // raw = 0x1234 → MSB 0x12, LSB 0x34.
        d.temp_raw = 0x1234;
        d.start();
        d.write(REG_TEMP_RESULT);
        d.start();
        assert_eq!(d.read(), 0x12);
        assert_eq!(d.read(), 0x34);
    }

    #[test]
    fn zero_raw_is_zero_celsius() {
        let mut d = Tmp117::new(TMP117_ADDR);
        d.set_input("temperature", 0.0).unwrap();
        assert_eq!(read_reg(&mut d, REG_TEMP_RESULT), 0x0000);
        assert_eq!(raw_to_celsius(0), 0.0);
    }

    #[test]
    fn configuration_write_reads_back() {
        let mut d = Tmp117::new(TMP117_ADDR);
        // A value with DATA_READY (bit13) clear so the readback is exact.
        write_reg(&mut d, REG_CONFIGURATION, 0x0360);
        assert_eq!(read_reg(&mut d, REG_CONFIGURATION), 0x0360);
    }

    #[test]
    fn data_ready_sets_on_new_result_and_clears_on_temp_read() {
        let mut d = Tmp117::new(TMP117_ADDR);
        // Clear any status; nothing converted yet.
        write_reg(&mut d, REG_CONFIGURATION, 0x0000);
        assert_eq!(read_reg(&mut d, REG_CONFIGURATION) & CFG_DATA_READY, 0);
        // New conversion → DATA_READY set.
        d.set_input("temperature", 25.0).unwrap();
        assert_eq!(
            read_reg(&mut d, REG_CONFIGURATION) & CFG_DATA_READY,
            CFG_DATA_READY
        );
        // Reading TEMP_RESULT clears it.
        let _ = read_reg(&mut d, REG_TEMP_RESULT);
        assert_eq!(read_reg(&mut d, REG_CONFIGURATION) & CFG_DATA_READY, 0);
    }

    #[test]
    fn device_id_is_read_only() {
        let mut d = Tmp117::new(TMP117_ADDR);
        write_reg(&mut d, REG_DEVICE_ID, 0xDEAD);
        assert_eq!(read_reg(&mut d, REG_DEVICE_ID), TMP117_DEVICE_ID);
    }

    #[test]
    fn temp_result_is_read_only() {
        let mut d = Tmp117::new(TMP117_ADDR);
        d.set_input("temperature", 42.0).unwrap();
        let before = read_reg(&mut d, REG_TEMP_RESULT);
        write_reg(&mut d, REG_TEMP_RESULT, 0x0000);
        assert_eq!(read_reg(&mut d, REG_TEMP_RESULT), before);
    }

    #[test]
    fn limit_registers_store_signed_values() {
        let mut d = Tmp117::new(TMP117_ADDR);
        write_reg(&mut d, REG_T_HIGH_LIMIT, celsius_to_raw(75.0) as u16);
        write_reg(&mut d, REG_T_LOW_LIMIT, celsius_to_raw(-20.0) as u16);
        assert_eq!(
            read_reg(&mut d, REG_T_HIGH_LIMIT) as i16,
            celsius_to_raw(75.0)
        );
        assert_eq!(
            read_reg(&mut d, REG_T_LOW_LIMIT) as i16,
            celsius_to_raw(-20.0)
        );
    }

    #[test]
    fn out_of_range_temperature_is_rejected() {
        let mut d = Tmp117::new(TMP117_ADDR);
        assert!(d.set_input("temperature", 200.0).is_err());
        assert!(d.set_input("humidity", 20.0).is_err());
    }
}
