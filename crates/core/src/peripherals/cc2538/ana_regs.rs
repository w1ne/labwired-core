// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CC2538 analog bias control (`cc2538_ana_regs`) at 0x400D_6000 (SWRU319C
//! §23.16.5, Table 23-12 "ANA_REGS Register Summary").
//!
//! The block has one register, ANA_REGS_IVCTRL @ 0x04, reset 0x0000_0013:
//!
//! | Bits | Field | Reset |
//! | --- | --- | --- |
//! | 7 | reserved, RO | 0 |
//! | 6 | reserved, "always read 0" | 0 |
//! | 5:4 | DAC_CURR_CTRL | 0x1 |
//! | 3 | LODIV_BIAS_CTRL | 0 |
//! | 2 | TXMIX_DC_CTRL | 0 |
//! | 1:0 | PA_BIAS_CTRL | 0x3 |
//!
//! Bits 31:8 are listed as reserved RW with reset 0; this model keeps them at
//! 0 (no documented function to store). Every other offset in the 4 KiB window
//! reads 0 and drops writes.
//!
//! Modelled: IVCTRL storage at its reset value and field mask. NOT modelled:
//! any bias change; the RF core's output power and the DAC do not depend on
//! these fields.

use crate::{Peripheral, SimResult};

const IVCTRL: u64 = 0x04;
const IVCTRL_MASK: u32 = 0x3F;
const IVCTRL_RESET: u32 = 0x13;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cc2538AnaRegs {
    ivctrl: u32,
}

impl Default for Cc2538AnaRegs {
    fn default() -> Self {
        Self::new()
    }
}

impl Cc2538AnaRegs {
    pub fn new() -> Self {
        Self {
            ivctrl: IVCTRL_RESET,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        if offset == IVCTRL {
            self.ivctrl
        } else {
            0
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        if offset == IVCTRL {
            self.ivctrl = value & IVCTRL_MASK;
        }
    }
}

impl Peripheral for Cc2538AnaRegs {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let lane = (offset & 3) * 8;
        Ok((self.read_reg(offset & !3) >> lane) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    /// IVCTRL's fields sit in byte 0, so a byte store to lane 0 is the whole
    /// write.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_reg(offset, value as u32);
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn on_system_reset(&mut self) {
        *self = Self::new();
    }

    fn on_node_reset(&mut self) {
        *self = Self::new();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        *self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "Cc2538AnaRegs snapshot decode: {error}"
            ))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ivctrl_resets_to_0x13_and_keeps_bits_5_to_0() {
        let mut ana = Cc2538AnaRegs::new();
        assert_eq!(ana.read_u32(0x04).unwrap(), 0x13);
        ana.write_u32(0x04, 0xFFFF_FFFF).unwrap();
        assert_eq!(ana.read_u32(0x04).unwrap(), 0x3F);
        ana.write(0x04, 0x08).unwrap();
        assert_eq!(ana.read_u32(0x04).unwrap(), 0x08);
    }

    #[test]
    fn other_offsets_read_zero_and_drop_writes() {
        let mut ana = Cc2538AnaRegs::new();
        for offset in [0x00, 0x08, 0xFFC] {
            ana.write_u32(offset, 0xFFFF_FFFF).unwrap();
            assert_eq!(ana.read_u32(offset).unwrap(), 0);
        }
        assert_eq!(ana.read_u32(0x04).unwrap(), 0x13);
    }

    #[test]
    fn reset_and_snapshot_round_trip() {
        let mut ana = Cc2538AnaRegs::new();
        ana.write_u32(0x04, 0x21).unwrap();
        let saved = ana.snapshot();
        ana.on_system_reset();
        assert_eq!(ana.read_u32(0x04).unwrap(), 0x13);
        ana.restore(saved).unwrap();
        assert_eq!(ana.read_u32(0x04).unwrap(), 0x21);
    }
}
