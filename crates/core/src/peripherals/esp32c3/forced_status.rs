// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Register-backed MMIO block that force-asserts a set of status bits on read.
//!
//! The WiFi PHY/RF calibration (closed `libphy` blob) launches analog/RF/PLL
//! operations and then busy-polls hardware status bits — PLL-lock, calibration
//! "done", FSM-idle — that only a real RF front-end would assert. The simulator
//! has no physical RF (the irreducible air-gap), so those polls would spin
//! forever. This block register-backs the window (writes stored, reads return
//! the last value) but ORs a configured set of `(offset, mask)` bits into the
//! read so the firmware observes the operations as instantaneously complete.
//!
//! This is the RF air-gap cut: we run real register/instruction models
//! everywhere up to the analog/RF boundary, and report the RF side as
//! "ready/locked/done" rather than modelling physical radio behaviour.

use crate::{Peripheral, SimResult};

#[derive(Debug)]
pub struct Esp32c3ForcedStatus {
    regs: Vec<u32>,
    /// `(word_offset, or_mask)` pairs OR'd into reads of that word.
    forced: Vec<(u64, u32)>,
}

impl Esp32c3ForcedStatus {
    /// `size_bytes` window (rounded up to words); `forced` lists the status
    /// words and the bit masks to force-assert on read.
    pub fn new(size_bytes: usize, forced: Vec<(u64, u32)>) -> Self {
        Self {
            regs: vec![0u32; size_bytes.div_ceil(4)],
            forced,
        }
    }

    fn force_mask(&self, offset: u64) -> u32 {
        self.forced
            .iter()
            .filter(|(o, _)| *o == offset)
            .fold(0, |acc, (_, m)| acc | m)
    }
}

impl Peripheral for Esp32c3ForcedStatus {
    // Inert walk: register bank with read-forced status bits (RF air-gap cut); tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let w = self.read_u32(offset & !3)?;
        Ok((w >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let sh = (offset & 3) * 8;
        let cur = *self.regs.get((aligned / 4) as usize).unwrap_or(&0);
        if let Some(slot) = self.regs.get_mut((aligned / 4) as usize) {
            *slot = (cur & !(0xFFu32 << sh)) | ((value as u32) << sh);
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let stored = *self.regs.get((offset / 4) as usize).unwrap_or(&0);
        Ok(stored | self.force_mask(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
            *slot = value;
        }
        Ok(())
    }

    fn legacy_tick_active(&self) -> bool {
        false
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

    #[test]
    fn forces_configured_bits_on_read() {
        let p = Esp32c3ForcedStatus::new(0x400, vec![(0x174, 1 << 16)]);
        assert_eq!(p.read_u32(0x174).unwrap() & (1 << 16), 1 << 16);
        // Unforced words read back what was written.
        assert_eq!(p.read_u32(0x10).unwrap(), 0);
    }

    #[test]
    fn forced_bits_survive_writes() {
        let mut p = Esp32c3ForcedStatus::new(0x400, vec![(0x174, 1 << 16)]);
        p.write_u32(0x174, 0x1).unwrap();
        let v = p.read_u32(0x174).unwrap();
        assert_eq!(v & 0x1, 0x1, "stored bit preserved");
        assert_eq!(v & (1 << 16), 1 << 16, "forced bit asserted");
    }
}
