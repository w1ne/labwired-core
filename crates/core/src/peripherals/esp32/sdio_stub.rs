// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Minimal SDIO host-side stub.
//!
//! We don't simulate the SDIO peripheral, but the ESP32 BROM's
//! `slc_init_attach` writes to HOST_SLC offset 0x40 then `bgez`-polls
//! the same register until the FSM signals completion by setting bit
//! 31. On real silicon this is hardware; in sim a plain RAM stub leaves
//! bit 31 clear forever and the BROM never exits init.
//!
//! [`HostSlc`] mirrors a regular RAM peripheral but always sets bit 31
//! on the FSM-status word at offset 0x40, so the BROM's `bgez` loop
//! takes the negative-branch path and continues.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::collections::HashMap;

const FSM_STATUS_OFFSET: u64 = 0x40;
const FSM_DONE_BIT: u32 = 0x8000_0000;

#[derive(Debug, Default)]
pub struct HostSlc {
    regs: HashMap<u32, u32>,
}

impl HostSlc {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for HostSlc {
    // Inert walk: register-backed SDIO stub (FSM-done forced on read); tick() is an explicit no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = (offset & !3) as u32;
        let byte_off = (offset & 3) * 8;
        let mut word = self.regs.get(&word_off).copied().unwrap_or(0);
        if word_off as u64 == FSM_STATUS_OFFSET {
            word |= FSM_DONE_BIT;
        }
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let word_off = (offset & !3) as u32;
        let mut word = self.regs.get(&word_off).copied().unwrap_or(0);
        if word_off as u64 == FSM_STATUS_OFFSET {
            word |= FSM_DONE_BIT;
        }
        Ok(word)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = (offset & !3) as u32;
        let byte_off = (offset & 3) * 8;
        let mut word = self.regs.get(&word_off).copied().unwrap_or(0);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.regs.insert(word_off, word);
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.regs.insert((offset & !3) as u32, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fsm_status_reads_with_done_bit_set() {
        let p = HostSlc::new();
        // Even with nothing written, bit 31 reads back high.
        let v = p.read_u32(FSM_STATUS_OFFSET).unwrap();
        assert_eq!(v & FSM_DONE_BIT, FSM_DONE_BIT);
    }

    #[test]
    fn fsm_status_done_bit_survives_write() {
        let mut p = HostSlc::new();
        p.write_u32(FSM_STATUS_OFFSET, 0x1000_0000).unwrap();
        let v = p.read_u32(FSM_STATUS_OFFSET).unwrap();
        // Lower bits are what was written; bit 31 stays set.
        assert_eq!(v & 0x1000_0000, 0x1000_0000);
        assert_eq!(v & FSM_DONE_BIT, FSM_DONE_BIT);
    }

    #[test]
    fn other_offsets_behave_as_plain_ram() {
        let mut p = HostSlc::new();
        p.write_u32(0x10, 0xDEAD_BEEF).unwrap();
        assert_eq!(p.read_u32(0x10).unwrap(), 0xDEAD_BEEF);
        // No magic done-bit on unrelated offsets.
        assert_eq!(p.read_u32(0x20).unwrap(), 0);
    }
}
