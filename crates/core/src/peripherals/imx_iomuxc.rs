// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! NXP i.MX RT IOMUXC — sticky register stub.
//!
//! Firmware writes SW_MUX_CTL / SW_PAD_CTL / SELECT_INPUT and must read them
//! back. This model stores every word write and returns it on read; it does
//! **not** change GPIO alternate-function routing (no mux effect).

use crate::{Peripheral, SimResult};
use std::any::Any;
use std::collections::HashMap;

/// Sticky IOMUXC pad/mux register bank (no routing side effects).
#[derive(Debug, Default)]
pub struct ImxIomuxc {
    regs: HashMap<u64, u32>,
}

impl ImxIomuxc {
    pub fn new() -> Self {
        Self {
            regs: HashMap::new(),
        }
    }

    fn word_offset(offset: u64) -> u64 {
        offset & !3
    }

    fn read_word(&self, offset: u64) -> u32 {
        self.regs
            .get(&Self::word_offset(offset))
            .copied()
            .unwrap_or(0)
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        self.regs.insert(Self::word_offset(offset), value);
    }
}

impl Peripheral for ImxIomuxc {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset);
        let shift = ((offset & 3) * 8) as u32;
        Ok(((word >> shift) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let off = Self::word_offset(offset);
        let shift = ((offset & 3) * 8) as u32;
        let mut word = self.regs.get(&off).copied().unwrap_or(0);
        word = (word & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_word(off, word);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "imx_iomuxc",
            "regs": self.regs.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    #[test]
    fn writes_stick_and_read_back() {
        let mut iomux = ImxIomuxc::new();
        // Typical SW_MUX_CTL_PAD word (MUX_MODE + SION); no mux effect modelled.
        iomux.write_u32(0x14, 0x0000_0015).unwrap();
        assert_eq!(iomux.read_u32(0x14).unwrap(), 0x0000_0015);
        iomux.write_u32(0x100, 0x10B0).unwrap();
        assert_eq!(iomux.read_u32(0x100).unwrap(), 0x10B0);
    }

    #[test]
    fn unread_offsets_return_zero() {
        let iomux = ImxIomuxc::new();
        assert_eq!(iomux.read_u32(0x00).unwrap(), 0);
    }
}
