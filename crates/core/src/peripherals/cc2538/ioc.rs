// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CC2538 I/O control (`cc2538_ioc`) at 0x400D_4000 (SWRU319C §9.3.2,
//! Table 9-7 "IOC Register Summary"). Renode's `cc2538.repl` leaves this
//! window unmodelled.
//!
//! | Offset | Name | Field | Reset |
//! | --- | --- | --- | --- |
//! | 0x000..=0x07C | IOC_PA0_SEL..IOC_PD7_SEL | Pxx_SEL \[4:0\]: peripheral output on the pad | 0x00 |
//! | 0x080..=0x0FC | IOC_PA0_OVER..IOC_PD7_OVER | Pxx_OVER \[3:0\]: OE 8, PUE 4, PDE 2, ANA 1 | 0x4 |
//! | 0x100..=0x150 | IOC_UARTRXD_UART0..IOC_GPT3OCP2 | INPUT_SEL \[4:0\]: pad 0 (PA0)..31 (PD7) routed to the input | 0x00 |
//!
//! Every register is RW over its field; the reserved bits above it read 0
//! (RO). Offsets past 0x150 in the 4 KiB window are not in Table 9-7: reads
//! return 0 and writes are dropped.
//!
//! Modelled: storage and read-back of every Table 9-7 register at its
//! documented reset value and field width. NOT modelled: the routing itself.
//! A SEL or INPUT_SEL value does not connect a peripheral to a pad, and an
//! OVER value does not change a pad's pull, output enable or analog mode; the
//! GPIO model takes its undriven-pad pull from the chip descriptor's
//! `pull_up_mask` (the reset state), and peripherals reach their pins
//! regardless of the IOC.

use crate::{Peripheral, SimResult};

/// 32 pads: PA0..PD7.
const PADS: usize = 32;
const SEL_BASE: u64 = 0x000;
const OVER_BASE: u64 = 0x080;
const INPUT_BASE: u64 = 0x100;
/// IOC_UARTRXD_UART0 (0x100) .. IOC_GPT3OCP2 (0x150).
const INPUTS: usize = 21;

const SEL_MASK: u32 = 0x1F;
const OVER_MASK: u32 = 0xF;
const INPUT_MASK: u32 = 0x1F;
/// IOC_Pxx_OVER reset: PUE (pull-up enable).
const OVER_RESET: u32 = 0x4;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cc2538Ioc {
    sel: Vec<u32>,
    over: Vec<u32>,
    input: Vec<u32>,
}

impl Default for Cc2538Ioc {
    fn default() -> Self {
        Self::new()
    }
}

impl Cc2538Ioc {
    pub fn new() -> Self {
        Self {
            sel: vec![0; PADS],
            over: vec![OVER_RESET; PADS],
            input: vec![0; INPUTS],
        }
    }

    fn reset_registers(&mut self) {
        *self = Self::new();
    }

    /// The register a word offset names and its field mask, if Table 9-7
    /// defines one.
    fn slot(&mut self, offset: u64) -> Option<(&mut u32, u32)> {
        let index = |base: u64| ((offset - base) / 4) as usize;
        match offset {
            o if o < OVER_BASE => Some((&mut self.sel[index(SEL_BASE)], SEL_MASK)),
            o if o < INPUT_BASE => Some((&mut self.over[index(OVER_BASE)], OVER_MASK)),
            o if o < INPUT_BASE + 4 * INPUTS as u64 => {
                Some((&mut self.input[index(INPUT_BASE)], INPUT_MASK))
            }
            _ => None,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        let index = |base: u64| ((offset - base) / 4) as usize;
        match offset {
            o if o < OVER_BASE => self.sel[index(SEL_BASE)],
            o if o < INPUT_BASE => self.over[index(OVER_BASE)],
            o if o < INPUT_BASE + 4 * INPUTS as u64 => self.input[index(INPUT_BASE)],
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        if let Some((reg, mask)) = self.slot(offset) {
            *reg = value & mask;
        }
    }
}

impl Peripheral for Cc2538Ioc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let lane = (offset & 3) * 8;
        Ok((self.read_reg(offset & !3) >> lane) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    /// Every field sits in byte 0 of its register, so a byte store to lane 0
    /// is the whole write; the upper lanes are reserved.
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
        self.reset_registers();
    }

    fn on_node_reset(&mut self) {
        self.reset_registers();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        *self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("Cc2538Ioc snapshot decode: {error}"))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_values_follow_table_9_7() {
        let ioc = Cc2538Ioc::new();
        assert_eq!(ioc.read_u32(0x000).unwrap(), 0); // IOC_PA0_SEL
        assert_eq!(ioc.read_u32(0x07C).unwrap(), 0); // IOC_PD7_SEL
        assert_eq!(ioc.read_u32(0x080).unwrap(), 0x4); // IOC_PA0_OVER
        assert_eq!(ioc.read_u32(0x0FC).unwrap(), 0x4); // IOC_PD7_OVER
        assert_eq!(ioc.read_u32(0x100).unwrap(), 0); // IOC_UARTRXD_UART0
        assert_eq!(ioc.read_u32(0x150).unwrap(), 0); // IOC_GPT3OCP2
    }

    #[test]
    fn writes_keep_only_the_field_bits() {
        let mut ioc = Cc2538Ioc::new();
        ioc.write_u32(0x004, 0xFFFF_FFFF).unwrap();
        ioc.write_u32(0x084, 0xFFFF_FFFF).unwrap();
        ioc.write_u32(0x150, 0xFFFF_FFFF).unwrap();
        assert_eq!(ioc.read_u32(0x004).unwrap(), 0x1F);
        assert_eq!(ioc.read_u32(0x084).unwrap(), 0xF);
        assert_eq!(ioc.read_u32(0x150).unwrap(), 0x1F);
        // Neighbours are untouched.
        assert_eq!(ioc.read_u32(0x000).unwrap(), 0);
        assert_eq!(ioc.read_u32(0x080).unwrap(), 0x4);
        assert_eq!(ioc.read_u32(0x14C).unwrap(), 0);
    }

    #[test]
    fn offsets_past_table_9_7_read_zero_and_drop_writes() {
        let mut ioc = Cc2538Ioc::new();
        ioc.write_u32(0x154, 0xFFFF_FFFF).unwrap();
        ioc.write_u32(0xFFC, 0xFFFF_FFFF).unwrap();
        assert_eq!(ioc.read_u32(0x154).unwrap(), 0);
        assert_eq!(ioc.read_u32(0xFFC).unwrap(), 0);
    }

    #[test]
    fn byte_access_hits_lane_zero_only() {
        let mut ioc = Cc2538Ioc::new();
        ioc.write(0x088, 0x8).unwrap();
        ioc.write(0x089, 0xFF).unwrap();
        assert_eq!(ioc.read_u32(0x088).unwrap(), 0x8);
        assert_eq!(ioc.read(0x088).unwrap(), 0x8);
        assert_eq!(ioc.read(0x089).unwrap(), 0);
    }

    #[test]
    fn reset_and_snapshot_round_trip() {
        let mut ioc = Cc2538Ioc::new();
        ioc.write_u32(0x100, 3).unwrap();
        ioc.write_u32(0x080, 0x8).unwrap();
        let saved = ioc.snapshot();
        ioc.on_system_reset();
        assert_eq!(ioc.read_u32(0x100).unwrap(), 0);
        assert_eq!(ioc.read_u32(0x080).unwrap(), 0x4);
        ioc.restore(saved).unwrap();
        assert_eq!(ioc.read_u32(0x100).unwrap(), 3);
        assert_eq!(ioc.read_u32(0x080).unwrap(), 0x8);
    }
}
