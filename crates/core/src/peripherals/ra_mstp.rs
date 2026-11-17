// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas RA module-stop control registers (MSTPCRB-D).
//!
//! RA4M1 resets module-stop bits to one. The bus clock-gate mechanism uses
//! these live values with active-low gate declarations, so firmware must clear
//! the corresponding bit before the peripheral answers MMIO. This model keeps
//! the three implemented 32-bit register words; PRCR write protection and the
//! effects on modules without explicit `clock:` declarations are not modeled.

use crate::{Peripheral, SimResult};

const RESET: u32 = u32::MAX;

#[derive(Debug, Clone)]
pub struct RaMstp {
    registers: [u32; 3],
}

impl RaMstp {
    pub fn new() -> Self {
        Self {
            registers: [RESET; 3],
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset & !3 {
            0x00 => self.registers[0], // MSTPCRB
            0x04 => self.registers[1], // MSTPCRC
            0x08 => self.registers[2], // MSTPCRD
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset & !3 {
            0x00 => self.registers[0] = value,
            0x04 => self.registers[1] = value,
            0x08 => self.registers[2] = value,
            _ => {}
        }
    }
}

impl Default for RaMstp {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for RaMstp {
    fn clock_gate_reg_offset(&self, name: &str) -> Option<u64> {
        match name.trim().to_ascii_lowercase().as_str() {
            "mstpcrb" => Some(0x00),
            "mstpcrc" => Some(0x04),
            "mstpcrd" => Some(0x08),
            _ => None,
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset);
        let shift = (offset & 3) * 8;
        Ok((word >> shift) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset & 3) * 8;
        let word = (self.read_word(reg) & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_word(reg, word);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }
}
