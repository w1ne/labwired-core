// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! QuickLogic EOS S3 simple periodic timer (`SPT`, 0x4000_5C00), the block
//! Renode maps as `Timers.EOSS3_SimplePeriodicTimer` in
//! `platforms/cpus/eos-s3.repl`.
//!
//! Sources: qorc-sdk `HAL/inc/eoss3_dev.h` (`SPT_REGS_TypeDef`: `SPT_CFG`
//! 0x00 .. `MS_CNT_VAL` 0x3C; `SPT_BASE = AON_PERIPH_BASE + 0x5C00`) and
//! Renode `EOSS3_SimplePeriodicTimer.cs` (field layout, the 1 ms 30-bit
//! counter, `MS_CNT_VALUE` constant 0x40, size 0x200).
//!
//! Modelled:
//! - `TIMER_VAL` (0x34, read-only): the 30-bit software counter. It counts
//!   once per millisecond of simulated time while `SPT_CFG.SPT_EN` (bit 0)
//!   is set, derived lazily from the bus cycle clock at `clock_hz`. Clearing
//!   `SPT_EN` freezes the count; setting it resumes from the frozen value.
//! - `UPDATE_TMR_VAL` (0x2C, bits [29:0]): stores the written value and
//!   loads it into the counter (Renode loads it while running too, and
//!   restarts the millisecond phase).
//! - `SPT_CFG`: `SPT_EN` (bit 0), `INT_MASK_N` (bits [9:2]) and
//!   `FFE_TO_PERIOD` (bits [17:10]) are stored. `CLK_SRC_SEL` (bit 1) and
//!   `PMU_TO_PERIOD` (bits [21:18]) are Renode tags: they read 0.
//! - `SLEEP_MODE` (0x04, bit 0) and `SPARE_BITS` (0x30, bits [7:0]) are
//!   stored.
//! - `EVENT_CNT_VAL` (0x38) reads the last `FFE_TO_PERIOD` value written,
//!   and `MS_CNT_VAL` (0x3C) reads 0x40, both as Renode returns them.
//!
//! NOT modelled: the FFE kick-off output (no FFE model, so nothing
//! consumes it), the eight FFE interrupt timestamp inputs, the clock source
//! select, the error-compensation words 0x08..0x28 (read 0, writes
//! ignored, as in Renode), the PMU period, and any rate change when firmware
//! re-clocks the M4 away from `clock_hz`. The counter wraps at 2^30 (about
//! 12.4 days).

use crate::{CycleClock, Peripheral, SimResult};

/// Renode `Size => 0x200`.
const SIZE: u64 = 0x200;

const SPT_CFG: u64 = 0x00;
const SLEEP_MODE: u64 = 0x04;
const UPDATE_TMR_VAL: u64 = 0x2C;
const SPARE_BITS: u64 = 0x30;
const TIMER_VAL: u64 = 0x34;
const EVENT_CNT_VAL: u64 = 0x38;
const MS_CNT_VAL: u64 = 0x3C;

const SPT_EN: u32 = 1 << 0;
/// `INT_MASK_N` [9:2] and `FFE_TO_PERIOD` [17:10].
const CFG_STORED: u32 = SPT_EN | (0xFF << 2) | (0xFF << 10);
const FFE_TO_PERIOD_SHIFT: u32 = 10;
const COUNT_MASK: u32 = (1 << 30) - 1;
/// Renode `MS_CNT_VALUE` constant.
const MS_CNT_RESET: u32 = 0x40;

/// M4 clock when neither the device config nor the system declares one:
/// Renode eos-s3.repl `systickFrequency: 61440000`.
pub const DEFAULT_CLOCK_HZ: u64 = 61_440_000;

/// EOS S3 `SPT` (see the module docs).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EosS3Spt {
    clock_hz: u64,
    /// `clock_hz` came from the device config; the system clock must not
    /// override it.
    clock_fixed: bool,
    cfg: u32,
    sleep_mode: u32,
    update_value: u32,
    spare: u32,
    ffe_period: u32,
    /// Counter value at `start_cycle` (or the frozen value when stopped).
    base: u32,
    /// CPU cycle the counter last started counting from; `None` = stopped.
    start_cycle: Option<u64>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl Default for EosS3Spt {
    fn default() -> Self {
        Self::new(None)
    }
}

impl EosS3Spt {
    /// `clock_hz`: the device config's clock, or `None` to take the system
    /// `cpu_hz` (falling back to [`DEFAULT_CLOCK_HZ`]).
    pub fn new(clock_hz: Option<u64>) -> Self {
        Self {
            clock_hz: clock_hz.unwrap_or(DEFAULT_CLOCK_HZ).max(1),
            clock_fixed: clock_hz.is_some(),
            cfg: 0,
            sleep_mode: 0,
            update_value: 0,
            spare: 0,
            ffe_period: 0,
            base: 0,
            start_cycle: None,
            clock: None,
        }
    }

    fn now_cycle(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    /// The 30-bit counter now.
    pub fn count(&self) -> u32 {
        let Some(start) = self.start_cycle else {
            return self.base;
        };
        let elapsed = self.now_cycle().saturating_sub(start);
        let ms = u128::from(elapsed) * 1000 / u128::from(self.clock_hz);
        (u128::from(self.base) + ms) as u32 & COUNT_MASK
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            SPT_CFG => self.cfg,
            SLEEP_MODE => self.sleep_mode,
            UPDATE_TMR_VAL => self.update_value,
            SPARE_BITS => self.spare,
            TIMER_VAL => self.count(),
            EVENT_CNT_VAL => self.ffe_period,
            MS_CNT_VAL => MS_CNT_RESET,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            SPT_CFG => {
                let was_on = self.cfg & SPT_EN != 0;
                self.cfg = value & CFG_STORED;
                self.ffe_period = (value >> FFE_TO_PERIOD_SHIFT) & 0xFF;
                match (was_on, self.cfg & SPT_EN != 0) {
                    (false, true) => self.start_cycle = Some(self.now_cycle()),
                    (true, false) => {
                        self.base = self.count();
                        self.start_cycle = None;
                    }
                    _ => {}
                }
            }
            SLEEP_MODE => self.sleep_mode = value & 1,
            UPDATE_TMR_VAL => {
                self.update_value = value & COUNT_MASK;
                self.base = self.update_value;
                if self.start_cycle.is_some() {
                    self.start_cycle = Some(self.now_cycle());
                }
            }
            SPARE_BITS => self.spare = value & 0xFF,
            TIMER_VAL | EVENT_CNT_VAL | MS_CNT_VAL => {}
            // Error-compensation words: Renode does not define them either.
            0x08..=0x28 => {}
            _ => crate::census_reg!("eos_s3_spt:EosS3Spt", offset, "write"),
        }
    }
}

impl Peripheral for EosS3Spt {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_reg(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let word = (self.read_reg(aligned) & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_reg(aligned, word);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset < SIZE {
            self.write_reg(offset & !3, value);
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    /// The count is read lazily from the bus cycle clock; nothing is raised.
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn attach_cpu_hz(&mut self, hz: u64) {
        if !self.clock_fixed && hz > 0 {
            self.clock_hz = hz;
        }
    }

    fn on_system_reset(&mut self) {
        let clock = self.clock.take();
        let (clock_hz, clock_fixed) = (self.clock_hz, self.clock_fixed);
        *self = Self::new(None);
        self.clock_hz = clock_hz;
        self.clock_fixed = clock_fixed;
        self.clock = clock;
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spt_at(hz: u64) -> (EosS3Spt, CycleClock) {
        let clock = CycleClock::default();
        let mut t = EosS3Spt::new(Some(hz));
        t.attach_cycle_clock(clock.clone());
        (t, clock)
    }

    #[test]
    fn counts_milliseconds_only_while_enabled() {
        let (mut t, clock) = spt_at(1_000_000);
        clock.publish(5_000);
        assert_eq!(t.read_u32(TIMER_VAL).unwrap(), 0, "disabled at reset");
        t.write_u32(SPT_CFG, SPT_EN).unwrap();
        clock.publish(5_999);
        assert_eq!(t.read_u32(TIMER_VAL).unwrap(), 0, "999 us");
        clock.publish(6_000);
        assert_eq!(t.read_u32(TIMER_VAL).unwrap(), 1);
        clock.publish(8_500);
        assert_eq!(t.read_u32(TIMER_VAL).unwrap(), 3);
        t.write_u32(SPT_CFG, 0).unwrap();
        clock.publish(100_000);
        assert_eq!(t.read_u32(TIMER_VAL).unwrap(), 3, "frozen");
        t.write_u32(SPT_CFG, SPT_EN).unwrap();
        clock.publish(102_000);
        assert_eq!(t.read_u32(TIMER_VAL).unwrap(), 5, "resumed");
    }

    #[test]
    fn update_loads_the_counter_and_wraps_at_30_bits() {
        let (mut t, clock) = spt_at(1_000);
        t.write_u32(UPDATE_TMR_VAL, 0xFFFF_FFFE).unwrap();
        assert_eq!(t.read_u32(UPDATE_TMR_VAL).unwrap(), 0x3FFF_FFFE);
        assert_eq!(t.read_u32(TIMER_VAL).unwrap(), 0x3FFF_FFFE);
        t.write_u32(SPT_CFG, SPT_EN).unwrap();
        clock.publish(3);
        assert_eq!(t.read_u32(TIMER_VAL).unwrap(), 1, "wrapped");
        t.write_u32(UPDATE_TMR_VAL, 100).unwrap();
        clock.publish(5);
        assert_eq!(t.read_u32(TIMER_VAL).unwrap(), 102, "loaded while running");
    }

    #[test]
    fn config_fields_and_constants() {
        let mut t = EosS3Spt::new(None);
        t.write_u32(SPT_CFG, 0xFFFF_FFFE).unwrap();
        assert_eq!(t.read_u32(SPT_CFG).unwrap(), 0x0003_FFFC, "tags read 0");
        assert_eq!(t.read_u32(EVENT_CNT_VAL).unwrap(), 0xFF);
        assert_eq!(t.read_u32(MS_CNT_VAL).unwrap(), 0x40);
        t.write_u32(SLEEP_MODE, 0xFF).unwrap();
        assert_eq!(t.read_u32(SLEEP_MODE).unwrap(), 1);
        t.write_u32(SPARE_BITS, 0x1234).unwrap();
        assert_eq!(t.read_u32(SPARE_BITS).unwrap(), 0x34);
        t.write_u32(0x08, 0xFFFF_FFFF).unwrap();
        assert_eq!(t.read_u32(0x08).unwrap(), 0);
        t.write_u32(TIMER_VAL, 7).unwrap();
        assert_eq!(t.read_u32(TIMER_VAL).unwrap(), 0, "read-only");
    }

    #[test]
    fn system_clock_applies_unless_configured() {
        let mut t = EosS3Spt::new(None);
        t.attach_cpu_hz(0);
        assert_eq!(t.clock_hz, DEFAULT_CLOCK_HZ);
        t.attach_cpu_hz(80_000_000);
        assert_eq!(t.clock_hz, 80_000_000);
        let mut fixed = EosS3Spt::new(Some(1_000));
        fixed.attach_cpu_hz(80_000_000);
        assert_eq!(fixed.clock_hz, 1_000);
        fixed.write_u32(UPDATE_TMR_VAL, 9).unwrap();
        fixed.on_system_reset();
        assert_eq!(fixed.clock_hz, 1_000);
        assert_eq!(fixed.read_u32(TIMER_VAL).unwrap(), 0);
    }
}
