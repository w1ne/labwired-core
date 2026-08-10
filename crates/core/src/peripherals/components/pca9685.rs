// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NXP PCA9685 16-channel 12-bit I²C PWM controller as an [`I2cDevice`].
//!
//! Used by the SpiceDispenser board: two hobby servos (revolver-select on
//! channel 8, shutter on channel 12) hang off the PCA9685's PWM outputs, and
//! the ESP32-S3 firmware drives them over I²C (default address `0x40`).
//!
//! ## Modeled behavior (matches the firmware's `pca9685.py` / C++ driver)
//!
//! - 256-byte register file with a write-pointer (the "control register").
//! - The first byte after a START sets the pointer; subsequent bytes are data.
//! - **Auto-increment**: when MODE1 (reg 0x00) bit5 (`AI`) is set, the pointer
//!   advances after every data byte read or written — so the 5-byte
//!   `LEDn_ON_L … LEDn_OFF_H` block write lands in consecutive registers.
//! - **Power-on register values match the datasheet** (Rev. 4): MODE1 `0x11`
//!   (SLEEP | ALLCALL), MODE2 `0x04`, SUBADR1/2/3 `0xE2`/`0xE4`/`0xE8`,
//!   ALLCALLADR `0xE0`, every `LEDn_OFF_H` `0x10` (full OFF), PRE_SCALE `0x1E`
//!   (200 Hz) — see [`RESET_VALUES`]. A firmware read-modify-write of any of
//!   them therefore starts from what hardware would give it.
//!
//! **These registers read back; they do not act.** No output drive, subaddress
//! / All-Call bus response, full-OFF gating or PWM frequency is implemented —
//! only MODE1.AI has behaviour. The gap list lives in
//! `configs/devices/pca9685.yaml`, which is the shipping model; this file is
//! its byte-parity oracle and must carry the same values.
//!
//! Each channel's 12-bit OFF count encodes the servo pulse width; the firmware
//! uses `off = us/20000 * 4096` with `us = 500 + deg/180 * 1900`. [`channel_off`]
//! and [`channel_angle_deg`] expose the captured value so a test (or the run
//! loop) can read back the commanded servo angle — closing the dispense loop.

use crate::peripherals::i2c::I2cDevice;

/// Default 7-bit I²C address (A0..A5 tied low).
pub const PCA9685_ADDR: u8 = 0x40;

const MODE1: usize = 0x00;
const MODE1_AI: u8 = 0x20; // auto-increment enable
const LED0_ON_L: usize = 0x06; // channel 0 base; channel n at 0x06 + 4*n

/// Sparse power-on reset values, NXP PCA9685 datasheet Rev. 4 (16 April 2015).
/// Everything not listed powers up 0x00. Kept as ONE table so this oracle and
/// `configs/devices/pca9685.yaml` cannot drift apart silently — the byte-parity
/// gate in `tests/pca9685_tmp102_parity.rs` compares them register by register.
///
/// These make the registers READ BACK like silicon; the functions they select
/// are not implemented (see the descriptor YAML for the full gap list).
const RESET_VALUES: &[(usize, u8)] = &[
    (0x00, 0x11), // MODE1      SLEEP | ALLCALL                    (p16)
    (0x01, 0x04), // MODE2      OUTDRV = totem-pole outputs        (p16)
    (0x02, 0xE2), // SUBADR1                                       (p26)
    (0x03, 0xE4), // SUBADR2                                       (p26)
    (0x04, 0xE8), // SUBADR3                                       (p26)
    (0x05, 0xE0), // ALLCALLADR LED All Call address           (p8, p26)
    (0xFE, 0x1E), // PRE_SCALE  200 Hz @ 25 MHz            (p25 Table 8, p2)
];

/// `LEDn_OFF_H` (0x09 + 4n) powers up with bit 4 — "LEDn full OFF" — set, so no
/// channel drives an output out of reset (p21, p25; addresses per the register
/// summary Table 4, p13, where LED15_OFF_H is 0x45).
const LED_OFF_H_RESET: u8 = 0x10;

/// `ALL_LED_ON_H` (0xFB) / `ALL_LED_OFF_H` (0xFD) are DELIBERATELY left at
/// 0x00. Table 8 (p25) star-marks them with bit 4 set; the register summary Table 4
/// (p13) types the same registers "write/read zero". The vendor contradicts
/// itself and we do not resolve it for them. Deliberately absent from
/// [`RESET_VALUES`]; named here only so the test below can pin the choice.
#[cfg(test)]
const ALL_LED_ON_H: usize = 0xFB;
#[cfg(test)]
const ALL_LED_OFF_H: usize = 0xFD;

pub struct Pca9685 {
    addr: u8,
    regs: [u8; 256],
    pointer: u8,
    writes_since_start: u32,
}

impl Pca9685 {
    pub fn new() -> Self {
        let mut regs = [0u8; 256];
        for &(off, val) in RESET_VALUES {
            regs[off] = val;
        }
        // Every channel's OFF_H powers up "full OFF".
        for ch in 0..16usize {
            regs[LED0_ON_L + 4 * ch + 3] = LED_OFF_H_RESET;
        }
        Self {
            addr: PCA9685_ADDR,
            regs,
            pointer: 0,
            writes_since_start: 0,
        }
    }

    pub fn with_address(addr: u8) -> Self {
        let mut d = Self::new();
        d.addr = addr;
        d
    }

    fn auto_increment(&self) -> bool {
        self.regs[MODE1] & MODE1_AI != 0
    }

    /// 12-bit OFF count last written to `channel` (0..15); the servo pulse width.
    pub fn channel_off(&self, channel: u8) -> u16 {
        let base = LED0_ON_L + 4 * channel as usize;
        let off_l = self.regs[base + 2] as u16;
        let off_h = (self.regs[base + 3] as u16) & 0x0F;
        (off_h << 8) | off_l
    }

    /// Commanded servo angle (degrees) for `channel`, inverting the firmware's
    /// `us = 500 + deg/180*1900`, `off = us/20000*4096` mapping. Returns `None`
    /// before any PWM has been written to the channel (OFF count still 0).
    pub fn channel_angle_deg(&self, channel: u8) -> Option<f32> {
        let off = self.channel_off(channel);
        if off == 0 {
            return None;
        }
        let us = off as f32 / 4096.0 * 20000.0;
        Some(((us - 500.0) / 1900.0 * 180.0).clamp(0.0, 180.0))
    }
}

impl Default for Pca9685 {
    fn default() -> Self {
        Self::new()
    }
}

impl I2cDevice for Pca9685 {
    fn address(&self) -> u8 {
        self.addr
    }

    fn start(&mut self) {
        self.writes_since_start = 0;
    }

    fn write(&mut self, data: u8) {
        if self.writes_since_start == 0 {
            // First byte after START selects the control register (pointer).
            self.pointer = data;
        } else {
            let reg = self.pointer as usize;
            self.regs[reg] = data;
            // A channel's OFF_H byte (LEDn base + 3) completes its pulse width;
            // log the resulting servo angle so a dispense is visible in the run.
            if reg >= LED0_ON_L && (reg - LED0_ON_L) % 4 == 3 {
                let ch = ((reg - LED0_ON_L) / 4) as u8;
                if let Some(deg) = self.channel_angle_deg(ch) {
                    eprintln!(
                        "PCA9685: channel {ch} servo -> {deg:.0}° (OFF={})",
                        self.channel_off(ch)
                    );
                }
            }
            if self.auto_increment() {
                self.pointer = self.pointer.wrapping_add(1);
            }
        }
        self.writes_since_start = self.writes_since_start.saturating_add(1);
    }

    fn read(&mut self) -> u8 {
        let v = self.regs[self.pointer as usize];
        if self.auto_increment() {
            self.pointer = self.pointer.wrapping_add(1);
        }
        v
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

    // Replays the firmware's pcaSetAngle(ch, deg) over the I2cDevice interface
    // and checks the angle reads back. AI must be enabled first (MODE1 |= 0x20).
    fn set_angle(d: &mut Pca9685, ch: u8, deg: f64) {
        let us = 500.0 + (deg / 180.0) * 1900.0;
        let ticks = (us / 20000.0 * 4096.0) as u16;
        let base = 0x06 + 4 * ch;
        d.start();
        d.write(base); // pointer
        d.write(0x00); // ON_L
        d.write(0x00); // ON_H
        d.write((ticks & 0xFF) as u8); // OFF_L
        d.write(((ticks >> 8) & 0x0F) as u8); // OFF_H
    }

    #[test]
    fn enabling_ai_then_setting_angles_reads_back() {
        let mut d = Pca9685::new();
        // Firmware enables auto-increment: MODE1 |= AI.
        d.start();
        d.write(MODE1 as u8);
        d.write(0xA1); // RESTART | AI | ALLCALL
        assert!(d.auto_increment());

        set_angle(&mut d, 8, 15.0); // revolver -> compartment 1 (15°)
        set_angle(&mut d, 12, 20.0); // shutter closed (20°)

        let rev = d.channel_angle_deg(8).expect("revolver set");
        let shut = d.channel_angle_deg(12).expect("shutter set");
        assert!((rev - 15.0).abs() < 1.5, "revolver ~15°, got {rev}");
        assert!((shut - 20.0).abs() < 1.5, "shutter ~20°, got {shut}");
    }

    #[test]
    fn power_on_mode1_is_silicon_default() {
        let mut d = Pca9685::new();
        d.start();
        d.write(MODE1 as u8); // pointer = MODE1
        assert_eq!(d.read(), 0x11); // SLEEP | ALLCALL, as on real PCA9685
    }

    /// The two registers the datasheet contradicts itself about stay 0x00, and
    /// this test is the record of that decision. Table 8 (p25) star-marks
    /// ALL_LED_ON_H / ALL_LED_OFF_H with bit 4 set; the register summary Table 4
    /// (p13) types the same registers "write/read zero". We do not resolve a
    /// contradiction the vendor has not resolved — so if someone later sets
    /// them, they have to argue with this assertion rather than drift past it.
    #[test]
    fn contradicted_all_led_h_registers_stay_zero() {
        let d = Pca9685::new();
        assert_eq!(d.regs[ALL_LED_ON_H], 0x00, "0xFB: p25 vs p13 disagree");
        assert_eq!(d.regs[ALL_LED_OFF_H], 0x00, "0xFD: p25 vs p13 disagree");
    }
}
