// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! TMP102 I²C temperature sensor as an `I2cDevice`.
//!
//! Per Texas Instruments TMP102 datasheet (SBOS397I):
//! - 7-bit address 0x48 (ADD0 = GND).
//! - Pointer register selects which 16-bit data register subsequent reads/writes target.
//! - Temperature register is 12-bit, left-justified into a 16-bit big-endian value
//!   (MSB returned first), with 1 LSB = 0.0625 °C.
//! - Table 6-7 "Pointer Addresses" gives the access of each slot: pointer 00 is
//!   the Temperature Register (Read Only); 01 / 10 / 11 are the Configuration,
//!   TLOW and THIGH registers, all **Read/Write**. §6.5.3: "The Configuration
//!   Register is a 16-bit read/write register."
//!
//! This model is retained only as the byte-parity oracle for the shipping
//! declarative descriptor `configs/devices/tmp102.yaml`
//! (`crates/core/tests/pca9685_tmp102_parity.rs`). It originally absorbed and
//! ignored every write to config/TLOW/THIGH, which contradicted Table 6-7 — a
//! driver that programmed the part and read it back got the reset value. Both
//! models now implement the writable registers, each from the datasheet.

use crate::peripherals::i2c::I2cDevice;

const TMP102_ADDR: u8 = 0x48;
const TMP_INITIAL: i16 = 0x1900; // 25.0 °C left-justified in 12-bit/16-bit

// Power-on values (Tables 6-10 / 6-11 for CONFIG; "High- and Low-Limit
// Registers" for the limits: "Power-up reset values … are: THIGH = +80°C and
// TLOW = +75°C").
const CONFIG_RESET: u16 = 0x60A0; // byte 1 = 0110_0000, byte 2 = 1010_0000
const TLOW_RESET: u16 = 0x4B00; // 75 °C
const THIGH_RESET: u16 = 0x5000; // 80 °C

/// Bits of the Configuration Register a master may actually change.
///
/// Layout (Tables 6-10 / 6-11):
///   byte 1 (D15:D8) = OS  R1 R0 F1 F0 POL TM SD
///   byte 2 (D7:D0)  = CR1 CR0 AL EM  0  0  0  0
///
/// Cleared here because the datasheet gives them to silicon:
///   * `0x6000` — R1:R0, "Converter Resolution. Read-only bits… set on
///     start-up to 11" (12-bit resolution).
///   * `0x0020` — AL, "a read-only function… information about the
///     comparator mode status". With POL = 0 it reads 1 until the temperature
///     reaches THIGH, so it powers up 1; no alert condition is modelled here,
///     so it stays there.
///   * `0x000F` — byte 2 D3:D0, unused, shown as `0` and always read 0.
const CONFIG_WRITE_MASK: u16 = 0x9FD0;

/// TLOW/THIGH are left-justified limits: 12-bit (D15:D4) in normal mode and
/// 13-bit (D15:D3) in extended mode. D2:D0 are unused in BOTH modes and always
/// read 0, so the writable field is the union of the two layouts.
const LIMIT_WRITE_MASK: u16 = 0xFFF8;

#[derive(Debug)]
pub struct Tmp102 {
    pointer: u8,
    temp_raw: i16,
    /// Configuration Register (pointer 01) — read/write, `CONFIG_WRITE_MASK`.
    config: u16,
    /// TLOW limit (pointer 10) — read/write, `LIMIT_WRITE_MASK`.
    t_low: u16,
    /// THIGH limit (pointer 11) — read/write, `LIMIT_WRITE_MASK`.
    t_high: u16,
    /// Phase tracker: 0 = next read returns MSB; 1 = next read returns LSB.
    /// Reset to 0 on `start()`.
    read_phase: u8,
    /// Tracks how many writes have occurred since `start()`: the first
    /// post-start write sets the pointer, the next two are the 16-bit word of
    /// the datasheet's Write Word Format.
    writes_since_start: u32,
    /// First data byte (MSB) of a two-byte write, held until its LSB arrives —
    /// the word only lands once both bytes have been clocked in.
    pending_msb: u8,
}

impl Tmp102 {
    pub fn new() -> Self {
        Self {
            pointer: 0,
            temp_raw: TMP_INITIAL,
            config: CONFIG_RESET,
            t_low: TLOW_RESET,
            t_high: THIGH_RESET,
            read_phase: 0,
            writes_since_start: 0,
            pending_msb: 0,
        }
    }

    /// Commit a completed two-byte write to the register the pointer selects,
    /// honouring the read-only bits inside it. Pointer 00 is the Temperature
    /// Register, which Table 6-7 marks Read Only — the word is dropped.
    fn store_word(&mut self, word: u16) {
        let (slot, mask) = match self.pointer {
            1 => (&mut self.config, CONFIG_WRITE_MASK),
            2 => (&mut self.t_low, LIMIT_WRITE_MASK),
            3 => (&mut self.t_high, LIMIT_WRITE_MASK),
            _ => return,
        };
        *slot = (*slot & !mask) | (word & mask);
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
        match self.writes_since_start {
            // First write after start sets the pointer register.
            0 => self.pointer = data & 0x03,
            // Write Word Format: MSB first, then LSB (§"Writing and Reading
            // Operation": "Register bytes are sent with the most significant
            // byte first"). The word lands only once both bytes are in.
            //
            // KNOWN GAP: the datasheet's "Data Transfer" note also allows a
            // single-byte update ("To update only the MS byte, terminate the
            // communication by issuing a START or STOP"). Neither this oracle
            // nor the declarative engine implements a partial-width write, so
            // an MS-byte-only update is absorbed by both. They agree; they are
            // both approximating here.
            1 => self.pending_msb = data,
            2 => {
                let word = (u16::from(self.pending_msb) << 8) | u16::from(data);
                self.store_word(word);
            }
            // Extra bytes past one 16-bit word are absorbed.
            _ => {}
        }
        self.writes_since_start = self.writes_since_start.saturating_add(1);
    }

    fn read(&mut self) -> u8 {
        let value: u16 = match self.pointer {
            0 => self.temp_raw as u16,
            1 => self.config,
            2 => self.t_low,
            3 => self.t_high,
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
        dev.start(); // reset phase
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

    /// Point at `ptr`, clock in a 16-bit word MSB-first (Write Word Format).
    fn write_word(dev: &mut Tmp102, ptr: u8, msb: u8, lsb: u8) {
        dev.start();
        dev.write(ptr);
        dev.write(msb);
        dev.write(lsb);
    }

    /// Point at `ptr`, repeated START, read the 16-bit word back (Read Word
    /// Format).
    fn read_word(dev: &mut Tmp102, ptr: u8) -> (u8, u8) {
        dev.start();
        dev.write(ptr);
        dev.start();
        (dev.read(), dev.read())
    }

    #[test]
    fn config_is_read_write_with_silicon_owned_bits_held() {
        // Table 6-7: pointer 01 is Read/Write. Write OS | SD, 1 Hz, extended
        // mode — and R1:R0 = 00, AL = 0, which silicon must ignore.
        let mut dev = Tmp102::new();
        assert_eq!(read_word(&mut dev, 0x01), (0x60, 0xA0));
        write_word(&mut dev, 0x01, 0x81, 0x50);
        assert_eq!(read_word(&mut dev, 0x01), (0xE1, 0x70));
        // Zeroing every firmware bit still leaves R1:R0 = 11 and AL = 1.
        write_word(&mut dev, 0x01, 0x00, 0x00);
        assert_eq!(read_word(&mut dev, 0x01), (0x60, 0x20));
    }

    #[test]
    fn limits_are_read_write_and_left_justified() {
        let mut dev = Tmp102::new();
        write_word(&mut dev, 0x02, 0x19, 0x07); // TLOW = 25 °C, stray low bits
        write_word(&mut dev, 0x03, 0x1E, 0x00); // THIGH = 30 °C
        assert_eq!(read_word(&mut dev, 0x02), (0x19, 0x00));
        assert_eq!(read_word(&mut dev, 0x03), (0x1E, 0x00));
    }

    #[test]
    fn temperature_register_rejects_writes() {
        let mut dev = Tmp102::new();
        write_word(&mut dev, 0x00, 0xDE, 0xAD);
        assert_eq!(read_word(&mut dev, 0x00), (0x19, 0x00));
    }

    #[test]
    fn a_short_write_does_not_land() {
        // Only the pointer and one data byte. Silicon would take this as an
        // MS-byte-only update; this model (like the declarative engine it is
        // the oracle for) waits for the full word. Pinned so the shared
        // approximation is visible rather than assumed. See `write`.
        let mut dev = Tmp102::new();
        dev.start();
        dev.write(0x01);
        dev.write(0x55);
        assert_eq!(read_word(&mut dev, 0x01), (0x60, 0xA0));
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
