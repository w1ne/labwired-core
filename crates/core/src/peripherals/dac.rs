// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! DAC peripheral — the STM32 dual 12-bit DAC register map shared by the
//! F0/F1/F4/L0/L4 parts (RM0376 §14.7 for the L0x2).
//!
//! Registers:
//!   0x00 CR        Control register (channel 2 fields at bit 16)
//!   0x04 SWTRIGR   Software trigger register
//!   0x08 DHR12R1 / 0x0C DHR12L1 / 0x10 DHR8R1    channel-1 holding views
//!   0x14 DHR12R2 / 0x18 DHR12L2 / 0x1C DHR8R2    channel-2 holding views
//!   0x20 DHR12RD / 0x24 DHR12LD / 0x28 DHR8RD    dual-channel views
//!   0x2C DOR1      Channel-1 data output (read-only)
//!   0x30 DOR2      Channel-2 data output (read-only)
//!   0x34 SR        Status register
//!
//! Each channel has ONE 12-bit data holder; the right-aligned, left-aligned
//! and 8-bit registers (and the dual ones) are views of it. DHR moves to DOR
//! at once when the channel's trigger is off (`CR.TENx = 0`), or on a
//! software trigger (`TSELx = 0b111` and `SWTRIGR.SWTRIGx`, which then
//! clears). Hardware triggers (timer TRGO, EXTI 9) are not modelled: with
//! one selected, DOR holds its value. No analog output, waves or DMA.
//!
//! Reset values: all zero.

use crate::SimResult;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Dac {
    cr: u32,
    /// Channel data holders (12 bits each).
    dhr: [u32; 2],
    dor: [u32; 2],
    sr: u32,
}

/// `TSELx = 0b111`: the software trigger.
const TSEL_SOFTWARE: u32 = 0b111;

impl Dac {
    pub fn new() -> Self {
        Self::default()
    }

    fn cr_field(&self, ch: usize) -> u32 {
        self.cr >> (16 * ch)
    }

    fn triggered(&self, ch: usize) -> bool {
        self.cr_field(ch) & (1 << 2) != 0
    }

    fn software_triggered(&self, ch: usize) -> bool {
        self.triggered(ch) && (self.cr_field(ch) >> 3) & 0b111 == TSEL_SOFTWARE
    }

    /// A holding-register write: DOR follows at once unless a trigger is on.
    fn load(&mut self, ch: usize, value: u32) {
        self.dhr[ch] = value & 0xFFF;
        if !self.triggered(ch) {
            self.dor[ch] = self.dhr[ch];
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        let [d1, d2] = self.dhr;
        match offset {
            0x00 => self.cr,
            // SWTRIGx clears as soon as the transfer happens.
            0x04 => 0,
            0x08 => d1,
            0x0C => d1 << 4,
            0x10 => d1 >> 4,
            0x14 => d2,
            0x18 => d2 << 4,
            0x1C => d2 >> 4,
            0x20 => d1 | (d2 << 16),
            0x24 => (d1 << 4) | (d2 << 20),
            0x28 => (d1 >> 4) | ((d2 >> 4) << 8),
            0x2C => self.dor[0],
            0x30 => self.dor[1],
            0x34 => self.sr,
            _ => {
                crate::census_reg!("dac:Dac", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr = value,
            0x04 => {
                for ch in 0..2 {
                    if value & (1 << ch) != 0 && self.software_triggered(ch) {
                        self.dor[ch] = self.dhr[ch];
                    }
                }
            }
            0x08 => self.load(0, value),
            0x0C => self.load(0, (value >> 4) & 0xFFF),
            0x10 => self.load(0, (value & 0xFF) << 4),
            0x14 => self.load(1, value),
            0x18 => self.load(1, (value >> 4) & 0xFFF),
            0x1C => self.load(1, (value & 0xFF) << 4),
            0x20 => {
                self.load(0, value);
                self.load(1, value >> 16);
            }
            0x24 => {
                self.load(0, (value >> 4) & 0xFFF);
                self.load(1, (value >> 20) & 0xFFF);
            }
            0x28 => {
                self.load(0, (value & 0xFF) << 4);
                self.load(1, ((value >> 8) & 0xFF) << 4);
            }
            0x2C | 0x30 => {}                          // DOR is read-only
            0x34 => self.sr &= !(value & 0x2000_2000), // DMAUDRx is W1C
            _ => {
                crate::census_reg!("dac:Dac", offset, "write");
            }
        }
    }
}

impl crate::Peripheral for Dac {
    // Inert walk: register bank with no time-driven state; tick() is the trait-default no-op.
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
        let shifted = (value as u32) << (byte * 8);
        // SWTRIGR and SR's W1C bits act on the written byte alone; the
        // other registers merge it into their current value.
        let v = if matches!(reg, 0x04 | 0x34) {
            shifted
        } else {
            (self.read_reg(reg) & !(0xFF << (byte * 8))) | shifted
        };
        self.write_reg(reg, v);
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    #[test]
    fn dor_sits_at_0x2c_and_0x30_and_follows_dhr_without_a_trigger() {
        let mut dac = Dac::new();
        dac.write_u32(0x00, 1 | (1 << 16)).unwrap();
        dac.write_u32(0x08, 0x0ABC).unwrap();
        dac.write_u32(0x18, 0x1230).unwrap(); // left-aligned 0x123
        assert_eq!(dac.read_u32(0x2C).unwrap(), 0x0ABC);
        assert_eq!(dac.read_u32(0x30).unwrap(), 0x0123);
        assert_eq!(dac.read_u32(0x0C).unwrap(), 0xABC0, "DHR12L1 view");
        assert_eq!(dac.read_u32(0x10).unwrap(), 0xAB, "DHR8R1 view");
        assert_eq!(dac.read_u32(0x20).unwrap(), 0x0123_0ABC, "DHR12RD view");
        dac.write_u32(0x28, 0x5A_C3).unwrap(); // DHR8RD
        assert_eq!(dac.read_u32(0x2C).unwrap(), 0xC30);
        assert_eq!(dac.read_u32(0x30).unwrap(), 0x5A0);
    }

    #[test]
    fn a_triggered_channel_holds_dor_until_its_software_trigger() {
        let mut dac = Dac::new();
        // EN1 | TEN1 | TSEL1 = software.
        dac.write_u32(0x00, 1 | (1 << 2) | (0b111 << 3)).unwrap();
        dac.write_u32(0x08, 0x0800).unwrap();
        assert_eq!(dac.read_u32(0x2C).unwrap(), 0);
        dac.write_u32(0x04, 0b10).unwrap(); // channel 2's trigger only
        assert_eq!(dac.read_u32(0x2C).unwrap(), 0);
        dac.write_u32(0x04, 0b01).unwrap();
        assert_eq!(dac.read_u32(0x2C).unwrap(), 0x0800);
        assert_eq!(dac.read_u32(0x04).unwrap(), 0, "SWTRIG1 cleared");
        // A hardware trigger (TSEL1 = TIM6 TRGO) is not modelled: DOR holds.
        dac.write_u32(0x00, 1 | (1 << 2)).unwrap();
        dac.write_u32(0x08, 0x0100).unwrap();
        dac.write_u32(0x04, 0b01).unwrap();
        assert_eq!(dac.read_u32(0x2C).unwrap(), 0x0800);
    }
}
