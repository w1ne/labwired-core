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
//! - Power-on MODE1 reads back `0x11` (SLEEP | ALLCALL), like real silicon, so
//!   the firmware's read-modify-write of MODE1 behaves as on hardware.
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

pub struct Pca9685 {
    addr: u8,
    regs: [u8; 256],
    pointer: u8,
    writes_since_start: u32,
}

impl Pca9685 {
    pub fn new() -> Self {
        let mut regs = [0u8; 256];
        regs[MODE1] = 0x11; // power-on default: SLEEP | ALLCALL
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
}
