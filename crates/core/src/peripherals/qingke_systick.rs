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
//! Interrupts: with a PFIC attached (`configure_riscv` does this when the
//! chip has a `qingke_pfic`), `STIE && CNTIF` drives PFIC interrupt 12 and
//! `SWIE` drives the software interrupt 14, both as level requests: each
//! stays requested until firmware clears CNTIF or SWIE, as the reference
//! manual tells handlers to. Without a PFIC the bits are stored only.
//!
//! The counter advances from the bus's elapsed core cycles, so it assumes
//! HCLK equals the core clock.

use crate::peripherals::qingke_pfic::QingkePficState;
use crate::{PeripheralTickResult, SimResult};
use std::sync::Arc;

const CTLR: u64 = 0x00;
const SR: u64 = 0x04;
const CNT: u64 = 0x08;
const CMP: u64 = 0x10;

const CTLR_STE: u32 = 1 << 0;
const CTLR_STIE: u32 = 1 << 1;
const CTLR_STCLK: u32 = 1 << 2;
const CTLR_STRE: u32 = 1 << 3;
const CTLR_SWIE: u32 = 1 << 31;
/// STE, STIE, STCLK, STRE and SWIE; the other bits are reserved.
const CTLR_MASK: u32 = 0x8000_000F;
const SR_CNTIF: u32 = 1 << 0;
/// PFIC interrupt numbers (WCH `IRQn_Type`).
const SYSTICK_IRQ: u32 = 12;
const SOFTWARE_IRQ: u32 = 14;

/// QingKe V2 STK register block.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct QingkeV2Systick {
    ctlr: u32,
    sr: u32,
    cnt: u32,
    cmp: u32,
    /// Core cycles not yet worth a count in HCLK/8 mode.
    prescale_rem: u8,
    /// Interrupt controller the STIE/SWIE requests drive, if the chip has one.
    #[serde(skip)]
    pfic: Option<Arc<QingkePficState>>,
}

impl QingkeV2Systick {
    pub fn new() -> Self {
        Self::default()
    }

    /// Route STIE/SWIE to `pfic` and publish the current request levels.
    pub fn attach_pfic(&mut self, pfic: Arc<QingkePficState>) {
        self.pfic = Some(pfic);
        self.publish_irq_lines();
    }

    fn publish_irq_lines(&self) {
        if let Some(pfic) = &self.pfic {
            let systick = self.ctlr & CTLR_STIE != 0 && self.sr & SR_CNTIF != 0;
            pfic.set_level(SYSTICK_IRQ, systick);
            pfic.set_level(SOFTWARE_IRQ, self.ctlr & CTLR_SWIE != 0);
        }
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
        self.publish_irq_lines();
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

    /// Take a decoded snapshot's registers, keeping the PFIC wiring (which
    /// is not part of the snapshot) and re-driving its request lines.
    fn restore_from(&mut self, restored: Self) {
        let pfic = self.pfic.take();
        *self = restored;
        self.pfic = pfic;
        self.publish_irq_lines();
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
        self.publish_irq_lines();
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
        self.restore_from(restored);
        Ok(())
    }

    fn attach_qingke_pfic(&mut self, pfic: Arc<QingkePficState>) {
        QingkeV2Systick::attach_pfic(self, pfic);
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
        self.restore_from(restored);
        Ok(())
    }
}

/// QingKe V4 STK (CH32V20x/V30x at 0xE000_F000): a 64-bit counter and
/// compare register, counting up or down.
///
/// Register map (ch32fun `ch32v20xhw.h` `SysTick_Type`, WCH CH32FV2x_V3xRM
/// STK chapter):
///
/// | Offset | Name | Meaning |
/// | --- | --- | --- |
/// | 0x00 | `STK_CTLR` | STE, STIE, STCLK, STRE as on V2; bit 4 MODE (1 = count down), bit 5 INIT (write 1: load 0 counting up, CMP counting down; reads 0), bit 31 SWIE |
/// | 0x04 | `STK_SR` | bit 0 CNTIF: up count reached CMP, or down count reached 0 |
/// | 0x08 | `STK_CNTL` / `STK_CNTH` (0x0C) | 64-bit counter, writable |
/// | 0x10 | `STK_CMPLR` / `STK_CMPHR` (0x14) | 64-bit compare value |
///
/// STRE=1 restarts from 0 after an up count reaches CMP, or from CMP after a
/// down count reaches 0, so the period is `CMP + 1` counts. STRE=0 keeps
/// counting past CMP (up) or from the maximum value after 0 (down).
/// `ch32fun` sets `CTLR = 1` at startup and its `DelaySysTick` polls the
/// free-running 64-bit CNT, reading it as two words. Interrupts and the
/// HCLK assumption are as on [`QingkeV2Systick`].
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct QingkeV4Systick {
    ctlr: u32,
    sr: u32,
    cnt: u64,
    cmp: u64,
    /// Core cycles not yet worth a count in HCLK/8 mode.
    prescale_rem: u8,
    #[serde(skip)]
    pfic: Option<Arc<QingkePficState>>,
}

const CNTH: u64 = 0x0C;
const CMPH: u64 = 0x14;
const CTLR_MODE_DOWN: u32 = 1 << 4;
const CTLR_INIT: u32 = 1 << 5;
/// STE, STIE, STCLK, STRE, MODE and SWIE; INIT is an action and reads 0.
const V4_CTLR_MASK: u32 = 0x8000_001F;

impl QingkeV4Systick {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attach_pfic(&mut self, pfic: Arc<QingkePficState>) {
        self.pfic = Some(pfic);
        self.publish_irq_lines();
    }

    fn publish_irq_lines(&self) {
        if let Some(pfic) = &self.pfic {
            let systick = self.ctlr & CTLR_STIE != 0 && self.sr & SR_CNTIF != 0;
            pfic.set_level(SYSTICK_IRQ, systick);
            pfic.set_level(SOFTWARE_IRQ, self.ctlr & CTLR_SWIE != 0);
        }
    }

    fn latch_cntif(&mut self) {
        self.sr |= SR_CNTIF;
        self.publish_irq_lines();
    }

    /// Advance by `ticks` counts. In either direction the counter walks a
    /// cycle that passes the flag position (CMP up, 0 down) once per period.
    fn count(&mut self, ticks: u64) {
        if ticks == 0 {
            return;
        }
        let reload = self.ctlr & CTLR_STRE != 0;
        if self.ctlr & CTLR_MODE_DOWN == 0 {
            let to_match = match self.cmp.wrapping_sub(self.cnt) {
                // Already on CMP: a full period away (2^64 without reload).
                0 if reload => self.cmp.saturating_add(1),
                0 => u64::MAX,
                d => d,
            };
            if ticks < to_match {
                self.cnt = if reload && self.cnt == self.cmp {
                    ticks - 1
                } else {
                    self.cnt.wrapping_add(ticks)
                };
                return;
            }
            self.latch_cntif();
            let after = ticks - to_match;
            self.cnt = if reload {
                match after % self.cmp.saturating_add(1) {
                    0 => self.cmp,
                    k => k - 1,
                }
            } else {
                self.cmp.wrapping_add(after)
            };
        } else {
            let to_zero = match self.cnt {
                0 if reload => self.cmp.saturating_add(1),
                0 => u64::MAX,
                c => c,
            };
            if ticks < to_zero {
                self.cnt = if reload && self.cnt == 0 {
                    self.cmp - (ticks - 1)
                } else {
                    self.cnt.wrapping_sub(ticks)
                };
                return;
            }
            self.latch_cntif();
            let after = ticks - to_zero;
            self.cnt = if reload {
                match after % self.cmp.saturating_add(1) {
                    0 => 0,
                    k => self.cmp - (k - 1),
                }
            } else {
                0u64.wrapping_sub(after)
            };
        }
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

    fn restore_from(&mut self, restored: Self) {
        let pfic = self.pfic.take();
        *self = restored;
        self.pfic = pfic;
        self.publish_irq_lines();
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            CTLR => self.ctlr,
            SR => self.sr,
            CNT => self.cnt as u32,
            CNTH => (self.cnt >> 32) as u32,
            CMP => self.cmp as u32,
            CMPH => (self.cmp >> 32) as u32,
            _ => {
                crate::census_reg!("qingke_systick:QingkeV4Systick", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        let low = |v: u64| (v & !0xFFFF_FFFF) | u64::from(value);
        let high = |v: u64| (v & 0xFFFF_FFFF) | (u64::from(value) << 32);
        match offset {
            CTLR => {
                if self.ctlr & CTLR_STE == 0 && value & CTLR_STE != 0 {
                    self.prescale_rem = 0;
                }
                self.ctlr = value & V4_CTLR_MASK;
                if value & CTLR_INIT != 0 {
                    self.cnt = if value & CTLR_MODE_DOWN != 0 {
                        self.cmp
                    } else {
                        0
                    };
                }
            }
            SR => self.sr &= value | !SR_CNTIF,
            CNT => self.cnt = low(self.cnt),
            CNTH => self.cnt = high(self.cnt),
            CMP => self.cmp = low(self.cmp),
            CMPH => self.cmp = high(self.cmp),
            _ => crate::census_reg!("qingke_systick:QingkeV4Systick", offset, "write"),
        }
        self.publish_irq_lines();
    }
}

impl crate::Peripheral for QingkeV4Systick {
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
                "QingkeV4Systick snapshot decode: {error}"
            ))
        })?;
        self.restore_from(restored);
        Ok(())
    }

    fn attach_qingke_pfic(&mut self, pfic: Arc<QingkePficState>) {
        QingkeV4Systick::attach_pfic(self, pfic);
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize QingkeV4Systick")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let restored: Self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "QingkeV4Systick runtime snapshot decode: {error}"
            ))
        })?;
        self.restore_from(restored);
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

    #[test]
    fn stie_and_swie_drive_pfic_levels() {
        let pfic = Arc::new(QingkePficState::default());
        let mut t = QingkeV2Systick::new();
        t.attach_pfic(pfic.clone());
        let level =
            |irq: u32| pfic.level[0].load(std::sync::atomic::Ordering::SeqCst) & (1 << irq) != 0;

        // SYSTICK_Interrupt example: CMP, CNT = 0, CTLR = 0xF.
        t.write_u32(CMP, 9).unwrap();
        t.write_u32(CTLR, 0xF).unwrap();
        let _ = t.tick_elapsed(9);
        assert!(level(SYSTICK_IRQ), "CNTIF with STIE requests IRQ 12");
        // The handler's SysTick->SR = 0 drops the request on the same store.
        t.write_u32(SR, 0).unwrap();
        assert!(!level(SYSTICK_IRQ));

        // CNTIF without STIE requests nothing.
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK | CTLR_STRE)
            .unwrap();
        let _ = t.tick_elapsed(10);
        assert_eq!(t.read_u32(SR).unwrap(), 1);
        assert!(!level(SYSTICK_IRQ));

        t.write_u32(CTLR, CTLR_SWIE).unwrap();
        assert!(level(SOFTWARE_IRQ));
        t.write_u32(CTLR, 0).unwrap();
        assert!(!level(SOFTWARE_IRQ));
    }

    #[test]
    fn v4_ch32fun_free_running_64bit_counter_carries_into_cnth() {
        let mut t = QingkeV4Systick::new();
        t.write_u32(CNT, 0xFFFF_FFF0).unwrap();
        t.write_u32(CTLR, CTLR_STE).unwrap();
        let _ = t.tick_elapsed(0x20 * 8);
        assert_eq!(t.read_u32(CNT).unwrap(), 0x10);
        assert_eq!(t.read_u32(CNTH).unwrap(), 1);
        // CMP stays 0, so the up count passed it long ago: no flag.
        assert_eq!(t.read_u32(SR).unwrap(), 0);
    }

    #[test]
    fn v4_up_count_reload_period_is_cmp_plus_one() {
        let mut t = QingkeV4Systick::new();
        t.write_u32(CMP, 999).unwrap();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK | CTLR_STRE | CTLR_INIT)
            .unwrap();
        assert_eq!(t.read_u32(CTLR).unwrap() & CTLR_INIT, 0, "INIT reads 0");
        let _ = t.tick_elapsed(999);
        assert_eq!(t.read_u32(SR).unwrap(), 1);
        assert_eq!(t.read_u32(CNT).unwrap(), 999);
        let _ = t.tick_elapsed(1);
        assert_eq!(t.read_u32(CNT).unwrap(), 0);
        t.write_u32(SR, 0).unwrap();
        let _ = t.tick_elapsed(1000 * 3 + 5);
        assert_eq!(t.read_u32(SR).unwrap(), 1);
        assert_eq!(t.read_u32(CNT).unwrap(), 5);
    }

    #[test]
    fn v4_down_count_flags_at_zero_and_reloads_cmp() {
        let mut t = QingkeV4Systick::new();
        t.write_u32(CMP, 99).unwrap();
        t.write_u32(CMPH, 0).unwrap();
        let ctlr = CTLR_STE | CTLR_STCLK | CTLR_STRE | CTLR_MODE_DOWN;
        t.write_u32(CTLR, ctlr | CTLR_INIT).unwrap();
        assert_eq!(t.read_u32(CNT).unwrap(), 99, "INIT loads CMP counting down");
        let _ = t.tick_elapsed(98);
        assert_eq!((t.read_u32(CNT).unwrap(), t.read_u32(SR).unwrap()), (1, 0));
        let _ = t.tick_elapsed(1);
        assert_eq!((t.read_u32(CNT).unwrap(), t.read_u32(SR).unwrap()), (0, 1));
        let _ = t.tick_elapsed(1);
        assert_eq!(t.read_u32(CNT).unwrap(), 99);
        // Without STRE it runs on from the maximum value.
        t.write_u32(CTLR, ctlr & !CTLR_STRE).unwrap();
        t.write_u32(CNT, 1).unwrap();
        let _ = t.tick_elapsed(3);
        assert_eq!(t.read_u32(CNT).unwrap(), 0xFFFF_FFFE);
        assert_eq!(t.read_u32(CNTH).unwrap(), 0xFFFF_FFFF);
    }

    #[test]
    fn v4_stie_drives_pfic_level_12() {
        let pfic = Arc::new(QingkePficState::default());
        let mut t = QingkeV4Systick::new();
        t.attach_pfic(pfic.clone());
        t.write_u32(CMP, 10).unwrap();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK | CTLR_STIE)
            .unwrap();
        let level =
            || pfic.level[0].load(std::sync::atomic::Ordering::SeqCst) & (1 << SYSTICK_IRQ) != 0;
        let _ = t.tick_elapsed(10);
        assert!(level());
        t.write_u32(SR, 0).unwrap();
        assert!(!level());
    }
}
