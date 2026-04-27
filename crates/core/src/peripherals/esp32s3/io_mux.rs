// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! IO_MUX peripheral for ESP32-S3.
//!
//! Per ESP32-S3 TRM §6.5. Each of the 49 GPIOs (0..48) has a 32-bit
//! function-select register at offset `pin * 4` from base 0x6000_9000.
//!
//! Plan 3 scope: round-trip storage only. The simulator does not enforce
//! the matrix routing implied by FUN_MUX bits; the peripheral exists so
//! esp-hal's pin-config sequence completes.
//!
//! Per-pin register bits (informational):
//!   bits[6:4]  MCU_SEL (function 0 = matrix, function 1 = GPIO, ...)
//!   bits[10:7] FUN_DRV (drive strength)
//!   bit[8]     FUN_PU  (pull-up enable)
//!   bit[7]     FUN_PD  (pull-down enable)
//!   bit[12]    FUN_IE  (input enable)

use crate::{Peripheral, SimResult};

const NUM_PINS: usize = 49;

#[derive(Debug)]
pub struct Esp32s3IoMux {
    pin_func: [u32; NUM_PINS],
}

impl Esp32s3IoMux {
    pub fn new() -> Self {
        Self {
            pin_func: [0; NUM_PINS],
        }
    }

    /// Read the current function-select word for `pin` (0..=48).
    /// Returns 0 for out-of-range pins.
    pub fn pin_func(&self, pin: u8) -> u32 {
        if (pin as usize) < NUM_PINS {
            self.pin_func[pin as usize]
        } else {
            0
        }
    }
}

impl Default for Esp32s3IoMux {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32s3IoMux {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let pin = (word_off / 4) as usize;
        let word = if pin < NUM_PINS {
            self.pin_func[pin]
        } else {
            0
        };
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let pin = (word_off / 4) as usize;
        if pin < NUM_PINS {
            let word = &mut self.pin_func[pin];
            *word &= !(0xFFu32 << byte_off);
            *word |= (value as u32) << byte_off;
        }
        // Out-of-range writes silently dropped.
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_zero() {
        let m = Esp32s3IoMux::new();
        for pin in 0u8..49 {
            assert_eq!(m.pin_func(pin), 0);
        }
    }

    #[test]
    fn pin0_round_trip() {
        let mut m = Esp32s3IoMux::new();
        // Pin 0 register at offset 0x00. Write FUN_DRV=2, MCU_SEL=1, FUN_IE=1.
        // Word value: (1 << 4) | (2 << 7) | (1 << 12) = 0x10 | 0x100 | 0x1000 = 0x1110.
        let val = 0x1110u32;
        for byte in 0..4u64 {
            m.write(byte, ((val >> (byte * 8)) & 0xFF) as u8).unwrap();
        }
        let mut read = 0u32;
        for byte in 0..4u64 {
            read |= (m.read(byte).unwrap() as u32) << (byte * 8);
        }
        assert_eq!(read, val);
        assert_eq!(m.pin_func(0), val);
    }

    #[test]
    fn pin2_round_trip_at_offset_8() {
        let mut m = Esp32s3IoMux::new();
        let val = 0xABCD_1234u32;
        let off = 2 * 4u64;
        for byte in 0..4u64 {
            m.write(off + byte, ((val >> (byte * 8)) & 0xFF) as u8).unwrap();
        }
        assert_eq!(m.pin_func(2), val);
    }

    #[test]
    fn pin48_in_range() {
        let mut m = Esp32s3IoMux::new();
        let val = 0x42u32;
        let off = 48 * 4u64;
        m.write(off, val as u8).unwrap();
        assert_eq!(m.pin_func(48) & 0xFF, val);
    }

    #[test]
    fn out_of_range_pin_silently_dropped() {
        let mut m = Esp32s3IoMux::new();
        // Pin 49 would be at offset 49*4 = 196.
        m.write(196, 0xAB).unwrap();
        // Read returns 0 for out-of-range.
        assert_eq!(m.read(196).unwrap(), 0);
    }

    #[test]
    fn writes_isolated_per_pin() {
        let mut m = Esp32s3IoMux::new();
        for byte in 0..4u64 {
            m.write(byte, 0xAB).unwrap();
        }
        for byte in 0..4u64 {
            m.write(4 + byte, 0xCD).unwrap();
        }
        assert_eq!(m.pin_func(0), 0xABABABABu32);
        assert_eq!(m.pin_func(1), 0xCDCDCDCDu32);
    }
}
