// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S2 APB SAR-ADC (digital ADC controller) — register file only.
//!
//! Base address `DR_REG_APB_SARADC_BASE = 0x3F44_0000`. The S2 map matches the
//! S3's through `ARB_CTRL` (0x38) and then diverges: `FILTER_CTRL` 0x3C,
//! `INT_ENA` 0x48, `DMA_CONF` 0x58, `APB_ADC_CLKM_CONF` 0x5C, `APB_DAC_CTRL`
//! 0x60. So the S3 `sar_adc` model cannot be reused.
//!
//! On the S2, IDF's oneshot driver (what `analogRead()` reaches) converts on
//! the RTC controller in SENS (`esp32s3::sens`, `new_esp32s2`). It only
//! *configures* this block: `adc_hal_arbiter_config` writes `ARB_CTRL`, and
//! `adc_ll_digi_set_*` read-modify-write `CTRL`/`CTRL2`/`FSM_WAIT`. So the model
//! is a register file. Each register is seeded with its reset value and stores
//! writes under its writable-bit mask. Both come from the ESP32-S2 SVD shipped
//! with arduino-esp32 (`tools/ide-debug/svd/esp32s2.svd`), not silicon dumps.
//! Status, `INT_RAW`/`INT_ST` and `FILTER_STATUS` are read-only and stay at
//! reset. `INT_CLR` is write-only and reads 0. Offsets outside the map read 0
//! and ignore writes.
//!
//! Not modelled: digital-controller conversions (continuous/DMA mode), the
//! threshold monitor, the IIR filter, the interrupt (`ETS_APB_ADC_INTR_SOURCE`
//! is never raised), and the DAC side of `APB_DAC_CTRL`.

use crate::{Peripheral, SimResult};

const CTRL_DATE: u64 = 0x3FC;
const INT_CLR: u64 = 0x054;

/// `(reset value, writable-bit mask)` for the register at `off`, or `None`
/// for a hole.
const fn spec(off: u64) -> Option<(u32, u32)> {
    match off {
        0x000 => Some((0x407F_8240, 0xDFFF_FFFB)),         // CTRL
        0x004 => Some((0x0000_A1FE, 0x01FF_FFFF)),         // CTRL2
        0x008 => Some((0x0200_0000, 0xFFFF_0000)),         // FSM
        0x00C => Some((0x00FF_0808, 0x00FF_FFFF)),         // FSM_WAIT
        0x010 | 0x014 => Some((0, 0)),                     // SAR1/2_STATUS (RO)
        0x018..=0x034 => Some((0x0F0F_0F0F, 0xFFFF_FFFF)), // SAR1/2_PATT_TAB1..4
        0x038 => Some((0x0000_0900, 0x0000_1FFC)),         // ARB_CTRL
        0x03C => Some((0x2040_0000, 0xFFFF_0003)),         // FILTER_CTRL
        0x040 => Some((0, 0)),                             // FILTER_STATUS (RO)
        0x044 => Some((0, 0xFFFF_FFFD)),                   // THRES_CTRL
        0x048 => Some((0, 0xF000_0000)),                   // INT_ENA
        0x04C | 0x050 => Some((0, 0)),                     // INT_RAW / INT_ST (RO)
        INT_CLR => Some((0, 0)),                           // WO, reads 0
        0x058 => Some((0x0000_00FF, 0xC000_FFFF)),         // DMA_CONF
        0x05C => Some((0x0000_0004, 0x006F_FFFF)),         // APB_ADC_CLKM_CONF
        0x060 => Some((0x0000_2064, 0x0001_FFFF)),         // APB_DAC_CTRL
        CTRL_DATE => Some((0x0190_7162, 0xFFFF_FFFF)),
        _ => None,
    }
}

const NWORDS: usize = 0x400 / 4;

#[derive(Debug)]
pub struct Esp32s2ApbSarAdc {
    regs: [u32; NWORDS],
}

impl Default for Esp32s2ApbSarAdc {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32s2ApbSarAdc {
    pub fn new() -> Self {
        let mut regs = [0u32; NWORDS];
        for (w, r) in regs.iter_mut().enumerate() {
            if let Some((reset, _)) = spec((w * 4) as u64) {
                *r = reset;
            }
        }
        Self { regs }
    }
}

impl Peripheral for Esp32s2ApbSarAdc {
    // Register file; no free-running state.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !3)?;
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let off = offset & !3;
        Ok(match spec(off) {
            Some(_) => self.regs[(off / 4) as usize],
            None => 0,
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let off = offset & !3;
        let shift = (offset & 3) * 8;
        let base = self.read_u32(off)?;
        let merged = (base & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_u32(off, merged)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let off = offset & !3;
        if let Some((_, wmask)) = spec(off) {
            let r = &mut self.regs[(off / 4) as usize];
            *r = (*r & !wmask) | (value & wmask);
        }
        Ok(())
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
    fn resets_follow_the_s2_svd() {
        let a = Esp32s2ApbSarAdc::new();
        assert_eq!(a.read_u32(0x000).unwrap(), 0x407F_8240);
        assert_eq!(a.read_u32(0x038).unwrap(), 0x0000_0900);
        assert_eq!(a.read_u32(0x058).unwrap(), 0x0000_00FF);
        assert_eq!(a.read_u32(CTRL_DATE).unwrap(), 0x0190_7162);
    }

    #[test]
    fn writes_store_under_mask_and_ro_regs_stick() {
        let mut a = Esp32s2ApbSarAdc::new();
        // adc_hal_arbiter_config: ARB_CTRL [12:2] writable.
        a.write_u32(0x038, 0xFFFF_FFFF).unwrap();
        assert_eq!(a.read_u32(0x038).unwrap(), 0x0000_1FFC);
        for off in [0x010u64, 0x040, 0x04C, 0x050, INT_CLR] {
            a.write_u32(off, 0xFFFF_FFFF).unwrap();
            assert_eq!(a.read_u32(off).unwrap(), 0, "RO/WO reg at {off:#x}");
        }
        // Holes do not round-trip.
        a.write_u32(0x064, 0xDEAD_BEEF).unwrap();
        assert_eq!(a.read_u32(0x064).unwrap(), 0);
    }

    #[test]
    fn byte_writes_merge() {
        let mut a = Esp32s2ApbSarAdc::new();
        a.write(0x039, 0x10).unwrap();
        assert_eq!(a.read_u32(0x038).unwrap(), 0x0000_1000);
    }
}
