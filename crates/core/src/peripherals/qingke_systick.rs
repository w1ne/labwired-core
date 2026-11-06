// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! WCH QingKe V2 system timer (STK), the 32-bit up-counter the CH32V003 maps
//! at 0xE000_F000.
//!
//! Register map (WCH `core_riscv.h` `SysTick_Type` for CH32V00x, and the
//! CH32V003 reference manual's STK chapter):
//!
//! | Offset | Name | Meaning |
//! | --- | --- | --- |
//! | 0x00 | `STK_CTLR` | bit 0 STE enable, bit 1 STIE, bit 2 STCLK (0 = HCLK/8, 1 = HCLK), bit 3 STRE auto-reload, bit 31 SWIE |
//! | 0x04 | `STK_SR` | bit 0 CNTIF: set when the counter reaches `STK_CMPLR`; software writes 0 to clear |
//! | 0x08 | `STK_CNTR` | 32-bit up-counter, writable |
//! | 0x10 | `STK_CMPLR` | 32-bit compare value |
//!
//! Both delay idioms the community uses depend on it: the WCH SDK's
//! `Delay_Us`/`Delay_Ms` clear CNTIF, load CMP, zero CNT, set STE and poll
//! CNTIF; `ch32v003fun` sets `CTLR = 1` (or 5 for HCLK) at startup and polls
//! the free-running CNT.
//!
//! STRE=1 counts `0..=CMP` and starts again from 0, so the compare period is
//! `CMP + 1` counts; STRE=0 keeps counting through CMP and wraps at 2^32.
//!
//! Not modeled: the PFIC. STIE and SWIE are stored, but no SysTick or
//! software interrupt is delivered to the core; polled CNT/CNTIF is the
//! firmware-visible surface. The counter advances from the bus's elapsed
//! core cycles, so it assumes HCLK equals the core clock.

use crate::{PeripheralTickResult, SimResult};

const CTLR: u64 = 0x00;
const SR: u64 = 0x04;
const CNT: u64 = 0x08;
const CMP: u64 = 0x10;

const CTLR_STE: u32 = 1 << 0;
const CTLR_STCLK: u32 = 1 << 2;
const CTLR_STRE: u32 = 1 << 3;
/// STE, STIE, STCLK, STRE and SWIE; the other bits are reserved.
const CTLR_MASK: u32 = 0x8000_000F;
const SR_CNTIF: u32 = 1 << 0;

/// QingKe V2 STK register block.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct QingkeV2Systick {
    ctlr: u32,
    sr: u32,
    cnt: u32,
    cmp: u32,
    /// Core cycles not yet worth a count in HCLK/8 mode.
    prescale_rem: u8,
}

impl QingkeV2Systick {
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance the counter by `ticks` counts and latch CNTIF if CMP is hit.
    fn count(&mut self, ticks: u64) {
        if ticks == 0 {
            return;
        }
        let reload = self.ctlr & CTLR_STRE != 0;
        let period = if reload {
            u64::from(self.cmp) + 1
        } else {
            1u64 << 32
        };
        // Counts until the counter next equals CMP. Sitting on CMP already
        // means a full period to the next match.
        let on_match = self.cnt == self.cmp;
        let to_match = if on_match {
            period
        } else {
            u64::from(self.cmp.wrapping_sub(self.cnt))
        };
        if on_match && reload && ticks < to_match {
            // The count after CMP restarts from 0.
            self.cnt = (ticks - 1) as u32;
            return;
        }
        if ticks < to_match {
            self.cnt = self.cnt.wrapping_add(ticks as u32);
            return;
        }
        self.sr |= SR_CNTIF;
        let after = ticks - to_match;
        self.cnt = if reload {
            // Positions after the match: CMP, 0, 1, ..., CMP - 1.
            match after % period {
                0 => self.cmp,
                k => (k - 1) as u32,
            }
        } else {
            self.cmp.wrapping_add(after as u32)
        };
    }

    fn advance(&mut self, cycles: u64) {
        if self.ctlr & CTLR_STE == 0 {
            return;
        }
        if self.ctlr & CTLR_STCLK != 0 {
            self.count(cycles);
        } else {
            let total = cycles + u64::from(self.prescale_rem);
            self.prescale_rem = (total % 8) as u8;
            self.count(total / 8);
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            CTLR => self.ctlr,
            SR => self.sr,
            CNT => self.cnt,
            CMP => self.cmp,
            _ => {
                crate::census_reg!("qingke_systick:QingkeV2Systick", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            CTLR => {
                if self.ctlr & CTLR_STE == 0 && value & CTLR_STE != 0 {
                    self.prescale_rem = 0;
                }
                self.ctlr = value & CTLR_MASK;
            }
            // CNTIF is cleared by writing 0; writing 1 leaves it unchanged.
            SR => self.sr &= value | !SR_CNTIF,
            CNT => self.cnt = value,
            CMP => self.cmp = value,
            _ => crate::census_reg!("qingke_systick:QingkeV2Systick", offset, "write"),
        }
    }
}

impl crate::Peripheral for QingkeV2Systick {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) as u32 * 8;
        let mask: u32 = 0xFF << shift;
        let v = (self.read_reg(reg) & !mask) | ((value as u32) << shift);
        self.write_reg(reg, v);
        Ok(())
    }

    /// A word store lands as one register write, so a CNT or CMP load never
    /// passes through intermediate byte-merged values.
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset % 4 == 0 {
            self.write_reg(offset, value);
            return Ok(());
        }
        for (i, byte) in value.to_le_bytes().iter().enumerate() {
            self.write(offset + i as u64, *byte)?;
        }
        Ok(())
    }

    /// The counter is the model: it must see every elapsed cycle.
    fn needs_legacy_walk(&self) -> bool {
        true
    }

    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.advance(cycles);
        PeripheralTickResult::default()
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "QingkeV2Systick snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize QingkeV2Systick")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let restored: Self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "QingkeV2Systick runtime snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    #[test]
    fn reset_state_is_stopped_and_clear() {
        let mut t = QingkeV2Systick::new();
        let _ = t.tick_elapsed(1_000);
        for reg in [CTLR, SR, CNT, CMP] {
            assert_eq!(t.read_u32(reg).unwrap(), 0, "offset {reg:#x}");
        }
    }

    /// The WCH SDK `Delay_Ms` sequence at 48 MHz: p_ms = 6000 HCLK/8 counts.
    #[test]
    fn wch_sdk_delay_sets_cntif_after_cmp_hclk_div8_counts() {
        let mut t = QingkeV2Systick::new();
        t.write_u32(SR, 0).unwrap();
        t.write_u32(CMP, 6_000).unwrap();
        t.write_u32(CNT, 0).unwrap();
        t.write_u32(CTLR, CTLR_STE).unwrap();

        let _ = t.tick_elapsed(6_000 * 8 - 1);
        assert_eq!(t.read_u32(SR).unwrap() & SR_CNTIF, 0, "one count short");
        assert_eq!(t.read_u32(CNT).unwrap(), 5_999);

        let _ = t.tick_elapsed(1);
        assert_eq!(t.read_u32(SR).unwrap() & SR_CNTIF, 1);
        assert_eq!(t.read_u32(CNT).unwrap(), 6_000);

        // Without STRE the counter keeps going past CMP.
        let _ = t.tick_elapsed(80);
        assert_eq!(t.read_u32(CNT).unwrap(), 6_010);

        // SR &= ~1 clears the flag; a write of 1 does not set it.
        t.write_u32(SR, 0).unwrap();
        assert_eq!(t.read_u32(SR).unwrap(), 0);
        t.write_u32(SR, 1).unwrap();
        assert_eq!(t.read_u32(SR).unwrap(), 0);
    }

    #[test]
    fn prescaler_carries_partial_counts_across_ticks() {
        let mut t = QingkeV2Systick::new();
        t.write_u32(CTLR, CTLR_STE).unwrap();
        for _ in 0..20 {
            let _ = t.tick_elapsed(3);
        }
        assert_eq!(t.read_u32(CNT).unwrap(), 60 / 8);
    }

    /// `ch32v003fun` sets `CTLR = 5` (STE | STCLK) and polls the free-running
    /// CNT; with CMP left at 0 the counter must still advance and wrap.
    #[test]
    fn ch32fun_free_running_hclk_counter() {
        let mut t = QingkeV2Systick::new();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK).unwrap();
        let _ = t.tick_elapsed(48);
        assert_eq!(t.read_u32(CNT).unwrap(), 48);

        t.write_u32(CNT, u32::MAX - 1).unwrap();
        let _ = t.tick_elapsed(3);
        assert_eq!(t.read_u32(CNT).unwrap(), 1);
        assert_eq!(t.read_u32(SR).unwrap(), 1, "passing CMP = 0 latches CNTIF");
    }

    #[test]
    fn auto_reload_period_is_cmp_plus_one() {
        let mut t = QingkeV2Systick::new();
        t.write_u32(CMP, 9).unwrap();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK | CTLR_STRE)
            .unwrap();
        let _ = t.tick_elapsed(9);
        assert_eq!(t.read_u32(CNT).unwrap(), 9);
        assert_eq!(t.read_u32(SR).unwrap(), 1);
        let _ = t.tick_elapsed(1);
        assert_eq!(t.read_u32(CNT).unwrap(), 0);
        t.write_u32(SR, 0).unwrap();
        // 25 more counts: 0 -> 9 (match) -> 0..=9 (match) -> 0..=4.
        let _ = t.tick_elapsed(25);
        assert_eq!(t.read_u32(CNT).unwrap(), 5);
        assert_eq!(t.read_u32(SR).unwrap(), 1);
    }

    #[test]
    fn stopped_counter_holds_and_ignores_reserved_bits() {
        let mut t = QingkeV2Systick::new();
        t.write_u32(CNT, 42).unwrap();
        t.write_u32(CTLR, 0x7FFF_FFF0).unwrap();
        assert_eq!(t.read_u32(CTLR).unwrap(), 0);
        let _ = t.tick_elapsed(1_000);
        assert_eq!(t.read_u32(CNT).unwrap(), 42);
    }
}
