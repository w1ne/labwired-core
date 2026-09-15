// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Microchip SAMD21 PM / GCLK and SAMD51 MCLK clock-controller models.
//!
//! SAMD21 PM mask registers (DS40001882 §16): AHBMASK@0x14, APBAMASK@0x18,
//! APBBMASK@0x1C, APBCMASK@0x20. GCLK: CLKCTRL@0x02 (16-bit), GENCTRL@0x04,
//! GENDIV@0x08. SAMD51 MCLK uses a similar APB*MASK naming with different
//! offsets (AHBMASK@0x10 … APBDMASK@0x20). SAMD51 GCLK is PCHCTRL-based:
//! PCHCTRL[n] @ 0x80+4*n with CHEN at bit 6 (DS60001507) — not D21 CLKCTRL.

use crate::{Peripheral, SimResult};
use std::any::Any;

// ── SAMD21 PM ────────────────────────────────────────────────────────────────

const PM_AHBMASK: u64 = 0x14;
const PM_APBAMASK: u64 = 0x18;
const PM_APBBMASK: u64 = 0x1C;
const PM_APBCMASK: u64 = 0x20;

/// SAMD21 Power Manager — AHB/APB clock-mask registers used for bus gating.
#[derive(Debug, Default)]
pub struct SamPm {
    ahbmask: u32,
    apbamask: u32,
    apbbmask: u32,
    apbcmask: u32,
}

impl SamPm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Map a symbolic enable-register name to its byte offset within PM.
    pub fn enable_reg_offset(&self, reg: &str) -> Option<u64> {
        match reg.trim().to_ascii_uppercase().as_str() {
            "AHBMASK" => Some(PM_AHBMASK),
            "APBAMASK" => Some(PM_APBAMASK),
            "APBBMASK" => Some(PM_APBBMASK),
            "APBCMASK" => Some(PM_APBCMASK),
            _ => None,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            PM_AHBMASK => self.ahbmask,
            PM_APBAMASK => self.apbamask,
            PM_APBBMASK => self.apbbmask,
            PM_APBCMASK => self.apbcmask,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            PM_AHBMASK => self.ahbmask = value,
            PM_APBAMASK => self.apbamask = value,
            PM_APBBMASK => self.apbbmask = value,
            PM_APBCMASK => self.apbcmask = value,
            _ => {}
        }
    }
}

impl Peripheral for SamPm {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn clock_gate_reg_offset(&self, name: &str) -> Option<u64> {
        self.enable_reg_offset(name)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "sam_pm",
            "ahbmask": self.ahbmask,
            "apbamask": self.apbamask,
            "apbbmask": self.apbbmask,
            "apbcmask": self.apbcmask,
        })
    }
}

// ── SAMD21 GCLK ──────────────────────────────────────────────────────────────

const GCLK_CTRL: u64 = 0x00;
const GCLK_STATUS: u64 = 0x01;
const GCLK_CLKCTRL: u64 = 0x02;
const GCLK_GENCTRL: u64 = 0x04;
const GCLK_GENDIV: u64 = 0x08;
/// SAMD51: PCHCTRL[0] base; channel *n* lives at `GCLK_PCHCTRL0 + 4*n`.
const GCLK_PCHCTRL0: u64 = 0x80;
const PCHCTRL_COUNT: usize = 64;

const CLKCTRL_CLKEN: u16 = 1 << 14;
const GENCTRL_GENEN: u32 = 1 << 16;
const PCHCTRL_CHEN: u32 = 1 << 6;

/// SAM Generic Clock Controller — D21 CLKCTRL and D51 PCHCTRL channel enables.
#[derive(Debug)]
pub struct SamGclk {
    ctrl: u8,
    clkctrl: u16,
    /// Last written GENCTRL word (ID-selected on silicon / D21 layout).
    genctrl: u32,
    gendiv: u32,
    /// SAMD51 PCHCTRL[n] words (also drive [`Self::enabled`]).
    pchctrl: [u32; PCHCTRL_COUNT],
    /// Peripheral channel enable latched from CLKCTRL / PCHCTRL writes (by ID).
    enabled: [bool; 64],
    /// Generator enable; generator 0 defaults on (silicon GCLKGEN0).
    gen_enabled: [bool; 16],
}

impl Default for SamGclk {
    fn default() -> Self {
        Self::new()
    }
}

impl SamGclk {
    pub fn new() -> Self {
        let mut gen_enabled = [false; 16];
        gen_enabled[0] = true;
        Self {
            ctrl: 0,
            clkctrl: 0,
            genctrl: GENCTRL_GENEN, // ID=0, GENEN=1
            gendiv: 0,
            pchctrl: [0; PCHCTRL_COUNT],
            enabled: [false; 64],
            gen_enabled,
        }
    }

    /// Whether the peripheral channel `id` is enabled (D21 CLKCTRL or D51 PCHCTRL).
    pub fn clk_enabled(&self, id: u8) -> bool {
        self.enabled.get(id as usize).copied().unwrap_or(false)
    }

    fn apply_clkctrl(&mut self, value: u16) {
        self.clkctrl = value;
        let id = (value & 0x3F) as usize;
        if id < self.enabled.len() {
            self.enabled[id] = value & CLKCTRL_CLKEN != 0;
        }
    }

    fn apply_genctrl(&mut self, value: u32) {
        self.genctrl = value;
        let id = (value & 0xF) as usize;
        if id < self.gen_enabled.len() {
            self.gen_enabled[id] = value & GENCTRL_GENEN != 0;
        }
    }

    fn apply_pchctrl(&mut self, id: usize, value: u32) {
        if id >= self.pchctrl.len() {
            return;
        }
        self.pchctrl[id] = value;
        if id < self.enabled.len() {
            self.enabled[id] = value & PCHCTRL_CHEN != 0;
        }
    }

    fn pchctrl_index(offset: u64) -> Option<(usize, u32)> {
        if offset < GCLK_PCHCTRL0 {
            return None;
        }
        let rel = offset - GCLK_PCHCTRL0;
        let id = (rel / 4) as usize;
        if id >= PCHCTRL_COUNT {
            return None;
        }
        let byte = (rel % 4) as u32;
        Some((id, byte))
    }

    fn read_byte(&self, offset: u64) -> u8 {
        if let Some((id, byte)) = Self::pchctrl_index(offset) {
            return ((self.pchctrl[id] >> (byte * 8)) & 0xFF) as u8;
        }
        match offset {
            GCLK_CTRL => self.ctrl,
            GCLK_STATUS => 0, // never SYNCBUSY in the sim
            GCLK_CLKCTRL => (self.clkctrl & 0xFF) as u8,
            0x03 => (self.clkctrl >> 8) as u8,
            GCLK_GENCTRL => (self.genctrl & 0xFF) as u8,
            0x05 => ((self.genctrl >> 8) & 0xFF) as u8,
            0x06 => ((self.genctrl >> 16) & 0xFF) as u8,
            0x07 => ((self.genctrl >> 24) & 0xFF) as u8,
            GCLK_GENDIV => (self.gendiv & 0xFF) as u8,
            0x09 => ((self.gendiv >> 8) & 0xFF) as u8,
            0x0A => ((self.gendiv >> 16) & 0xFF) as u8,
            0x0B => ((self.gendiv >> 24) & 0xFF) as u8,
            _ => 0,
        }
    }

    fn write_byte(&mut self, offset: u64, value: u8) {
        if let Some((id, byte)) = Self::pchctrl_index(offset) {
            let mut v = self.pchctrl[id];
            let mask: u32 = 0xFF << (byte * 8);
            v = (v & !mask) | ((value as u32) << (byte * 8));
            self.apply_pchctrl(id, v);
            return;
        }
        match offset {
            GCLK_CTRL => self.ctrl = value,
            GCLK_STATUS => {}
            GCLK_CLKCTRL => {
                let v = (self.clkctrl & 0xFF00) | value as u16;
                self.apply_clkctrl(v);
            }
            0x03 => {
                let v = (self.clkctrl & 0x00FF) | ((value as u16) << 8);
                self.apply_clkctrl(v);
            }
            GCLK_GENCTRL => {
                let v = (self.genctrl & !0xFF) | value as u32;
                self.apply_genctrl(v);
            }
            0x05 => {
                let v = (self.genctrl & !(0xFF << 8)) | ((value as u32) << 8);
                self.apply_genctrl(v);
            }
            0x06 => {
                let v = (self.genctrl & !(0xFF << 16)) | ((value as u32) << 16);
                self.apply_genctrl(v);
            }
            0x07 => {
                let v = (self.genctrl & !(0xFF << 24)) | ((value as u32) << 24);
                self.apply_genctrl(v);
            }
            GCLK_GENDIV => {
                self.gendiv = (self.gendiv & !0xFF) | value as u32;
            }
            0x09 => {
                self.gendiv = (self.gendiv & !(0xFF << 8)) | ((value as u32) << 8);
            }
            0x0A => {
                self.gendiv = (self.gendiv & !(0xFF << 16)) | ((value as u32) << 16);
            }
            0x0B => {
                self.gendiv = (self.gendiv & !(0xFF << 24)) | ((value as u32) << 24);
            }
            _ => {}
        }
    }
}

impl Peripheral for SamGclk {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.read_byte(offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_byte(offset, value);
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "sam_gclk",
            "clkctrl": self.clkctrl,
            "genctrl": self.genctrl,
            "gendiv": self.gendiv,
        })
    }
}

// ── SAMD51 MCLK ──────────────────────────────────────────────────────────────

const MCLK_AHBMASK: u64 = 0x10;
const MCLK_APBAMASK: u64 = 0x14;
const MCLK_APBBMASK: u64 = 0x18;
const MCLK_APBCMASK: u64 = 0x1C;
const MCLK_APBDMASK: u64 = 0x20;

/// SAMD51 Main Clock Controller — APB/AHB mask registers for bus gating.
#[derive(Debug, Default)]
pub struct SamMclk {
    ahbmask: u32,
    apbamask: u32,
    apbbmask: u32,
    apbcmask: u32,
    apbdmask: u32,
}

impl SamMclk {
    pub fn new() -> Self {
        Self::default()
    }

    /// Map a symbolic enable-register name to its byte offset within MCLK.
    pub fn enable_reg_offset(&self, reg: &str) -> Option<u64> {
        match reg.trim().to_ascii_uppercase().as_str() {
            "AHBMASK" => Some(MCLK_AHBMASK),
            "APBAMASK" => Some(MCLK_APBAMASK),
            "APBBMASK" => Some(MCLK_APBBMASK),
            "APBCMASK" => Some(MCLK_APBCMASK),
            "APBDMASK" => Some(MCLK_APBDMASK),
            _ => None,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            MCLK_AHBMASK => self.ahbmask,
            MCLK_APBAMASK => self.apbamask,
            MCLK_APBBMASK => self.apbbmask,
            MCLK_APBCMASK => self.apbcmask,
            MCLK_APBDMASK => self.apbdmask,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            MCLK_AHBMASK => self.ahbmask = value,
            MCLK_APBAMASK => self.apbamask = value,
            MCLK_APBBMASK => self.apbbmask = value,
            MCLK_APBCMASK => self.apbcmask = value,
            MCLK_APBDMASK => self.apbdmask = value,
            _ => {}
        }
    }
}

impl Peripheral for SamMclk {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn clock_gate_reg_offset(&self, name: &str) -> Option<u64> {
        self.enable_reg_offset(name)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "sam_mclk",
            "ahbmask": self.ahbmask,
            "apbamask": self.apbamask,
            "apbbmask": self.apbbmask,
            "apbcmask": self.apbcmask,
            "apbdmask": self.apbdmask,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    #[test]
    fn pm_apbcmask_bit_roundtrip() {
        let mut pm = SamPm::new();
        assert_eq!(pm.read_u32(0x20).unwrap(), 0);
        pm.write_u32(0x20, 1 << 4).unwrap();
        assert_eq!(pm.read_u32(0x20).unwrap() & (1 << 4), 1 << 4);
    }

    #[test]
    fn gclk_clkctrl_clken_sticks() {
        let mut g = SamGclk::new();
        g.write_u16(0x02, 0x4016).unwrap(); // ID=22, GEN=0, CLKEN=1
        assert!(g.clk_enabled(22));
        assert!(!g.clk_enabled(21));
    }

    /// SAMD51 GCLK: PCHCTRL[n] at 0x80+4*n, CHEN=bit 6 (DS60001507).
    #[test]
    fn pchctrl_chen_enables_channel() {
        let mut g = SamGclk::new();
        assert!(!g.clk_enabled(24));
        // PCHCTRL[24] @ 0x80+4*24 = 0xE0; GEN=0 | CHEN=1 → 0x40
        g.write_u32(0x80 + 4 * 24, 0x40).unwrap();
        assert!(g.clk_enabled(24));
        assert!(!g.clk_enabled(23));
        // Clearing CHEN disables the channel.
        g.write_u32(0x80 + 4 * 24, 0x00).unwrap();
        assert!(!g.clk_enabled(24));
    }

    #[test]
    fn mclk_apbcmask_roundtrip() {
        let mut m = SamMclk::new();
        assert_eq!(m.enable_reg_offset("APBCMASK"), Some(0x1C));
        assert_eq!(m.enable_reg_offset("APBDMASK"), Some(0x20));
        m.write_u32(0x1C, 1 << 3).unwrap();
        assert_eq!(m.read_u32(0x1C).unwrap() & (1 << 3), 1 << 3);
        m.write_u32(0x20, 1 << 0).unwrap();
        assert_eq!(m.read_u32(0x20).unwrap() & 1, 1);
    }

    #[test]
    fn pm_enable_reg_offset_names() {
        let pm = SamPm::new();
        assert_eq!(pm.enable_reg_offset("APBCMASK"), Some(0x20));
        assert_eq!(pm.enable_reg_offset("ahbmask"), Some(0x14));
        assert_eq!(pm.enable_reg_offset("APBAMASK"), Some(0x18));
        assert_eq!(pm.enable_reg_offset("APBBMASK"), Some(0x1C));
        assert_eq!(pm.enable_reg_offset("nope"), None);
    }
}
