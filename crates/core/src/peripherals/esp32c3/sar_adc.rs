// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 SAR ADC controller (`APB_SARADC`, `0x6004_0000`) — enough of the
//! conversion handshake to let the IDF's ADC self-calibration complete.
//!
//! During PHY/clock bring-up the IDF runs `adc_hal_self_calibration`, a
//! successive-approximation search for each ADC's offset-calibration code. Each
//! step (`read_cal_channel`) triggers a single conversion (set start bit at
//! `0x20`) and busy-polls the data-valid flag (`0x44` bit31 for SAR1 / bit30
//! for SAR2) before reading the 12-bit result (`0x2C` / `0x30`). The
//! declarative stub never asserts the valid flag, so the poll spins forever
//! right after `spi_flash` init.
//!
//! We model conversions as instantaneous: the valid flags always read set, and
//! the data registers return a mid-scale (2048) sample. The search is bounded
//! and only compares readings against a midpoint reference, so a constant
//! sample makes it converge to a deterministic (meaningless, but harmless) cal
//! code — exactly the "no real analog, complete gracefully" behaviour. All
//! other registers are register-backed (writes stored, reads return them).

use crate::{Peripheral, SimResult};

const SAR1_DATA: u64 = 0x2C;
const SAR2_DATA: u64 = 0x30;
const DATA_STATUS: u64 = 0x44; // bit31 SAR1 valid, bit30 SAR2 valid
const VALID_BITS: u32 = (1 << 31) | (1 << 30);
/// Mid-scale 12-bit sample (2048). The cal search only needs a stable value to
/// compare against its midpoint reference.
const MID_SAMPLE: u32 = 0x800;

#[derive(Debug)]
pub struct Esp32c3SarAdc {
    regs: Vec<u32>,
}

impl Default for Esp32c3SarAdc {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32c3SarAdc {
    pub fn new() -> Self {
        Self {
            regs: vec![0u32; 0x100 / 4],
        }
    }
}

impl Peripheral for Esp32c3SarAdc {
    // Inert walk: conversions are instantaneous (valid flags + mid-scale sample forced on read); tick() is the trait-default no-op.
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
        let cur = self.read_u32(aligned)?;
        self.write_u32(aligned, (cur & !(0xFFu32 << sh)) | ((value as u32) << sh))
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            // Conversion completes instantly: both data-valid flags read set so
            // read_cal_channel's busy-poll exits immediately.
            DATA_STATUS => *self.regs.get((offset / 4) as usize).unwrap_or(&0) | VALID_BITS,
            // Mid-scale sample for the offset-cal search.
            SAR1_DATA | SAR2_DATA => MID_SAMPLE,
            _ => *self.regs.get((offset / 4) as usize).unwrap_or(&0),
        })
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
    fn data_valid_flags_always_set() {
        let a = Esp32c3SarAdc::new();
        let st = a.read_u32(DATA_STATUS).unwrap();
        assert_ne!(st & (1 << 31), 0, "SAR1 valid");
        assert_ne!(st & (1 << 30), 0, "SAR2 valid");
    }

    #[test]
    fn data_registers_return_mid_scale() {
        let a = Esp32c3SarAdc::new();
        assert_eq!(a.read_u32(SAR1_DATA).unwrap() & 0xFFF, 0x800);
        assert_eq!(a.read_u32(SAR2_DATA).unwrap() & 0xFFF, 0x800);
    }

    #[test]
    fn other_registers_are_register_backed() {
        let mut a = Esp32c3SarAdc::new();
        a.write_u32(0x20, 0xDEAD_0000).unwrap();
        assert_eq!(a.read_u32(0x20).unwrap(), 0xDEAD_0000);
    }
}
