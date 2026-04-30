// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! TMP102 I²C temperature sensor as an `I2cDevice`.
//!
//! Per Texas Instruments TMP102 datasheet:
//! - 7-bit address 0x48 (ADD0 = GND).
//! - Pointer register selects which 16-bit data register subsequent reads/writes target.
//! - Temperature register is 12-bit, left-justified into a 16-bit big-endian value
//!   (MSB returned first), with 1 LSB = 0.0625 °C.
//!
//! For the simulator we only model the temperature register's read path plus
//! pointer tracking. Drift behavior is added in Task 2.

use crate::peripherals::i2c::I2cDevice;

const TMP102_ADDR: u8 = 0x48;
const TMP_INITIAL: i16 = 0x1900; // 25.0 °C left-justified in 12-bit/16-bit

#[derive(Debug)]
pub struct Tmp102 {
    pointer: u8,
    temp_raw: i16,
    /// Phase tracker: 0 = next read returns MSB; 1 = next read returns LSB.
    /// Reset to 0 on `start()`.
    read_phase: u8,
    /// Tracks how many writes have occurred since `start()` so the first
    /// post-start write sets the pointer and subsequent writes are absorbed
    /// into config/T_LOW/T_HIGH (ignored for the demo).
    writes_since_start: u32,
}

impl Tmp102 {
    pub fn new() -> Self {
        Self {
            pointer: 0,
            temp_raw: TMP_INITIAL,
            read_phase: 0,
            writes_since_start: 0,
        }
    }
}

impl Default for Tmp102 {
    fn default() -> Self {
        Self::new()
    }
}

impl I2cDevice for Tmp102 {
    fn address(&self) -> u8 {
        TMP102_ADDR
    }

    fn write(&mut self, data: u8) {
        if self.writes_since_start == 0 {
            // First write after start sets the pointer register.
            self.pointer = data & 0x03;
        }
        // Subsequent writes (e.g. config bytes) are accepted and ignored —
        // the demo never writes those registers.
        self.writes_since_start = self.writes_since_start.saturating_add(1);
    }

    fn read(&mut self) -> u8 {
        let value: u16 = match self.pointer {
            0 => self.temp_raw as u16,
            1 => 0x60A0,
            2 => 0x4B00,
            3 => 0x5000,
            _ => 0,
        };
        let byte = if self.read_phase == 0 {
            (value >> 8) as u8
        } else {
            (value & 0xFF) as u8
        };
        self.read_phase ^= 1;
        // Tick drift only at the end of a full MSB+LSB pair, and only when
        // reading the temperature register.
        if self.read_phase == 0 && self.pointer == 0 {
            self.temp_raw = self.temp_raw.wrapping_add(0x80); // +0.5 °C
            if self.temp_raw > 0x2300 {
                self.temp_raw = 0x1400; // wrap to 20 °C
            }
        }
        byte
    }

    fn start(&mut self) {
        self.read_phase = 0;
        self.writes_since_start = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_is_0x48() {
        let dev = Tmp102::new();
        assert_eq!(dev.address(), 0x48);
    }

    #[test]
    fn pointer_set_by_first_write_after_start() {
        let mut dev = Tmp102::new();
        dev.start();
        dev.write(0x01); // pointer ← 0x01 (CONFIG)
        assert_eq!(dev.pointer, 0x01);
    }

    #[test]
    fn temperature_read_returns_msb_then_lsb() {
        let mut dev = Tmp102::new();
        dev.start();
        let msb = dev.read();
        let lsb = dev.read();
        assert_eq!(msb, 0x19);
        assert_eq!(lsb, 0x00);
    }

    #[test]
    fn read_phase_resets_on_start() {
        let mut dev = Tmp102::new();
        dev.start();
        let _ = dev.read(); // advance phase to 1
        dev.start();        // reset phase
        let msb = dev.read();
        assert_eq!(msb, 0x19);
    }

    #[test]
    fn drift_increments_after_full_read() {
        let mut dev = Tmp102::new();
        dev.start();
        let _msb = dev.read();
        let _lsb = dev.read(); // full read pair → tick
        // Internal raw should have advanced by 0x80 (0.5 °C).
        assert_eq!(dev.temp_raw, 0x1980);
    }

    #[test]
    fn drift_wraps_at_35c_back_to_20c() {
        let mut dev = Tmp102::new();
        dev.temp_raw = 0x2300; // 35.0 °C
        dev.start();
        let _ = dev.read();
        let _ = dev.read(); // tick → 35.5 °C → wraps to 20.0 °C
        assert_eq!(dev.temp_raw, 0x1400);
    }

    #[test]
    fn partial_read_does_not_increment() {
        let mut dev = Tmp102::new();
        dev.start();
        let _ = dev.read(); // only MSB; phase=1
        // No tick yet — temp_raw must be unchanged.
        assert_eq!(dev.temp_raw, 0x1900);
    }
}
