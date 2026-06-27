// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 LED PWM controller (`LEDC`, `0x6001_9000`) — behavioral timer
//! engine with a live, time-advanced counter and overflow interrupts.
//!
//! ## Why this is not the demoted shadow-copy model
//!
//! An earlier pass left LEDC `unrecorded` on purpose: a faithful model needs a
//! counter that actually advances over time and an observable timer event, not
//! a `DUTY_START` shadow-register copy (that exact copy-on-write approach was
//! demoted on the classic ESP32 as a fake — it proves nothing a register file
//! couldn't already do). This model instead drives the four LEDC timers as
//! genuine up-counters clocked by elapsed simulation cycles: each
//! `TIMERx_VALUE.CNT` readback reflects real elapsed counts, and a counter that
//! wraps past its programmed `2^DUTY_RES` period latches the matching
//! `LSTIMERx_OVF` interrupt. That is the same class of proof the ESP32-S3 MCPWM
//! (TEZ wrap) and STM32 TIM1 (compare-match) tier-1 cells already rely on: a
//! register file cannot make a counter advance, wrap, and raise an edge.
//!
//! ## Timer model (register offsets + fields from
//! `configs/peripherals/esp32c3/ledc.yaml`)
//!
//! Each of the four low-speed timers has two registers, 8 bytes apart:
//!   - `TIMERx_CONF` (`0xA0 + x*8`): `DUTY_RES` [3:0] (period = `1 << DUTY_RES`
//!     counts), `CLK_DIV` [21:4] (an 18-bit fixed-point divider; we model its
//!     integer part `>> 8`), `PAUSE` [22] (freeze the counter), `RST` [23]
//!     (hold the counter at 0), `PARA_UP` [25] (config-commit strobe,
//!     self-clearing).
//!   - `TIMERx_VALUE` (`0xA4 + x*8`): `CNT` [13:0] — the live counter, read
//!     back directly (no latch register on the C3 LEDC).
//!
//! Overflow latches in the interrupt block (`INT_RAW` `0xC0`, `INT_ST` `0xC4`,
//! `INT_ENA` `0xC8`, `INT_CLR` `0xCC`): `LSTIMERx_OVF_INT` is bit `x`. `INT_ST`
//! is `INT_RAW & INT_ENA`; `INT_RAW` and `INT_CLR` are write-1-to-clear.
//!
//! ## Counter rate
//!
//! The simulator advances peripherals one CPU cycle per [`Peripheral::tick`]
//! (the same contract the ESP32 timer-group model uses). The LEDC base clock on
//! the C3 is APB-derived, so we treat one tick as one LEDC source pulse and
//! advance the counter every `max(1, CLK_DIV >> 8)` ticks. The fractional part
//! of `CLK_DIV` is deliberately ignored: the counter still advances and wraps
//! deterministically as a function of elapsed cycles and the programmed
//! divider/resolution, which is the property under test. Channels, `CONF`, and
//! `DATE` remain register-backed — this model adds the live timer the
//! declarative descriptor lacks, nothing more.

use crate::{Peripheral, PeripheralTickResult, SimResult};

pub const LEDC_BASE: u32 = 0x6001_9000;
pub const LEDC_SIZE: u64 = 0x400;

/// `LEDC` interrupt-matrix source on the C3 (`LEDC_INT_MAP` at
/// `interrupt_core0.yaml` offset 92 = `4 * 23`).
pub const LEDC_INTR_SOURCE_ID: u32 = 23;

/// Number of low-speed timers in the C3 LEDC block.
const NUM_TIMERS: usize = 4;

/// First timer-config register offset (`TIMER0_CONF`); the four configs are
/// 8 bytes apart (`TIMERx_VALUE` sits between them).
const TIMER0_CONF: u64 = 0xA0;
/// Last timer offset still inside the timer bank (`TIMER3_VALUE` = 0xBC).
const TIMER_BANK_END: u64 = 0xBC;

const INT_RAW: u64 = 0xC0;
const INT_ST: u64 = 0xC4;
const INT_ENA: u64 = 0xC8;
const INT_CLR: u64 = 0xCC;

/// `DUTY_RES` field width: period = `1 << DUTY_RES`, capped to the 14-bit
/// counter width the `CNT` field exposes.
const DUTY_RES_MASK: u32 = 0xF;
/// `CLK_DIV` occupies `TIMERx_CONF` bits [21:4]; the integer part is the upper
/// 10 bits (`>> 8` within the field).
const CLK_DIV_SHIFT: u32 = 4;
const CLK_DIV_FIELD_MASK: u32 = 0x3_FFFF;
const PAUSE_BIT: u32 = 1 << 22;
const RST_BIT: u32 = 1 << 23;
const PARA_UP_BIT: u32 = 1 << 25;

/// The `CNT` field in `TIMERx_VALUE` is 14 bits wide.
const CNT_MASK: u32 = 0x3FFF;

/// All four `LSTIMERx_OVF` interrupt bits.
const OVF_INT_MASK: u32 = 0xF;

#[derive(Debug, Default, Clone)]
struct Timer {
    /// `TIMERx_CONF` raw value (config readback).
    conf: u32,
    /// Live up-counter (the value `TIMERx_VALUE.CNT` returns).
    counter: u32,
    /// Sub-count accumulator: CPU cycles elapsed toward the next count.
    accum: u64,
}

impl Timer {
    fn duty_res(&self) -> u32 {
        self.conf & DUTY_RES_MASK
    }

    /// Period in counts (`1 << DUTY_RES`). A `DUTY_RES` of 0 yields a period of
    /// 1 (overflow every count); the result is clamped to the 14-bit counter.
    fn period(&self) -> u32 {
        (1u32 << self.duty_res()).min(CNT_MASK + 1)
    }

    /// Integer part of the fixed-point `CLK_DIV` divider; at least 1.
    fn divider(&self) -> u64 {
        let field = (self.conf >> CLK_DIV_SHIFT) & CLK_DIV_FIELD_MASK;
        ((field >> 8) as u64).max(1)
    }

    fn paused(&self) -> bool {
        self.conf & PAUSE_BIT != 0
    }

    fn in_reset(&self) -> bool {
        self.conf & RST_BIT != 0
    }

    /// Advance by one CPU cycle. Returns true if the counter wrapped past its
    /// period this cycle (an overflow edge).
    fn advance(&mut self) -> bool {
        if self.in_reset() {
            self.counter = 0;
            self.accum = 0;
            return false;
        }
        if self.paused() {
            return false;
        }
        self.accum += 1;
        let per_count = self.divider();
        let mut overflowed = false;
        while self.accum >= per_count {
            self.accum -= per_count;
            self.counter += 1;
            if self.counter >= self.period() {
                self.counter = 0;
                overflowed = true;
            }
        }
        overflowed
    }
}

pub struct Esp32c3Ledc {
    /// Interrupt-matrix source ID (`LEDC` = 23 on the C3).
    source_id: u32,
    /// Register-backed storage for the whole window (word indexed). Channels,
    /// `CONF`, and `DATE` live here; the timer/interrupt offsets are handled
    /// out of band below.
    regs: Vec<u32>,
    timers: [Timer; NUM_TIMERS],
    /// Latched raw interrupt bits (`INT_RAW`); only `LSTIMERx_OVF` ([3:0]) are
    /// driven by this model.
    int_raw: u32,
    /// `INT_ENA` mask.
    int_ena: u32,
}

impl Default for Esp32c3Ledc {
    fn default() -> Self {
        Self::new(LEDC_INTR_SOURCE_ID)
    }
}

impl std::fmt::Debug for Esp32c3Ledc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32c3Ledc(src={}, int_raw=0x{:x}, cnt=[{},{},{},{}])",
            self.source_id,
            self.int_raw,
            self.timers[0].counter,
            self.timers[1].counter,
            self.timers[2].counter,
            self.timers[3].counter
        )
    }
}

impl Esp32c3Ledc {
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            regs: vec![0u32; (LEDC_SIZE / 4) as usize],
            timers: Default::default(),
            int_raw: 0,
            int_ena: 0,
        }
    }

    fn int_st(&self) -> u32 {
        self.int_raw & self.int_ena
    }

    /// If `offset` is a timer register, return `(timer index, is_value_reg)`.
    fn timer_at(offset: u64) -> Option<(usize, bool)> {
        if !(TIMER0_CONF..=TIMER_BANK_END).contains(&offset) {
            return None;
        }
        let rel = offset - TIMER0_CONF;
        let idx = (rel / 8) as usize;
        let is_value = rel % 8 == 4;
        Some((idx, is_value))
    }

    fn reg(&self, off: u64) -> u32 {
        *self.regs.get((off / 4) as usize).unwrap_or(&0)
    }
}

impl Peripheral for Esp32c3Ledc {
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
        let off = offset & !3;
        if let Some((idx, is_value)) = Self::timer_at(off) {
            return Ok(if is_value {
                self.timers[idx].counter & CNT_MASK
            } else {
                self.timers[idx].conf
            });
        }
        Ok(match off {
            INT_RAW => self.int_raw,
            INT_ST => self.int_st(),
            INT_ENA => self.int_ena,
            INT_CLR => 0, // write-only
            o => self.reg(o),
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let off = offset & !3;
        if let Some((idx, is_value)) = Self::timer_at(off) {
            if is_value {
                // CNT is read-only on silicon; ignore writes.
            } else {
                // Store the config (PARA_UP is a self-clearing commit strobe);
                // applying DUTY_RES/CLK_DIV immediately is sufficient here.
                self.timers[idx].conf = value & !PARA_UP_BIT;
                if value & RST_BIT != 0 {
                    self.timers[idx].counter = 0;
                    self.timers[idx].accum = 0;
                }
            }
            return Ok(());
        }
        match off {
            // R/WTC and W1C: writing 1s clears those latched overflow bits.
            INT_RAW | INT_CLR => {
                self.int_raw &= !(value & OVF_INT_MASK);
            }
            INT_ENA => {
                self.int_ena = value & OVF_INT_MASK;
            }
            INT_ST => {} // read-only
            o => {
                if let Some(slot) = self.regs.get_mut((o / 4) as usize) {
                    *slot = value;
                }
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        for (i, timer) in self.timers.iter_mut().enumerate() {
            if timer.advance() {
                self.int_raw |= 1 << i;
            }
        }
        PeripheralTickResult {
            explicit_irqs: if self.int_st() != 0 {
                Some(vec![self.source_id])
            } else {
                None
            },
            ..PeripheralTickResult::default()
        }
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

    const TIMER0_CONF_OFF: u64 = 0xA0;
    const TIMER0_VALUE_OFF: u64 = 0xA4;

    /// Compose a `TIMERx_CONF` value: `duty_res` resolution, integer divider
    /// `div_int`, timer running (PAUSE/RST clear).
    fn conf(duty_res: u32, div_int: u32) -> u32 {
        (duty_res & DUTY_RES_MASK) | (((div_int & 0x3FF) << 8) << CLK_DIV_SHIFT)
    }

    fn step(l: &mut Esp32c3Ledc, n: usize) {
        for _ in 0..n {
            l.tick();
        }
    }

    #[test]
    fn counter_advances_over_time() {
        let mut l = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
        // Big period so it does not wrap during this window; divider 1.
        l.write_u32(TIMER0_CONF_OFF, conf(13, 1)).unwrap();
        assert_eq!(l.read_u32(TIMER0_VALUE_OFF).unwrap(), 0, "counter starts 0");
        step(&mut l, 100);
        let v = l.read_u32(TIMER0_VALUE_OFF).unwrap();
        assert!(v > 0, "counter advanced with elapsed cycles, got {v}");
    }

    #[test]
    fn divider_slows_the_counter() {
        let mut l = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
        l.write_u32(TIMER0_CONF_OFF, conf(13, 10)).unwrap();
        step(&mut l, 100);
        // 100 cycles / divider 10 ≈ 10 counts (not 100).
        let v = l.read_u32(TIMER0_VALUE_OFF).unwrap();
        assert_eq!(v, 10, "integer divider gates the count rate");
    }

    #[test]
    fn overflow_latches_after_a_full_period() {
        let mut l = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
        // Period = 1 << 4 = 16 counts, divider 1.
        l.write_u32(TIMER0_CONF_OFF, conf(4, 1)).unwrap();
        assert_eq!(
            l.read_u32(INT_RAW).unwrap() & 1,
            0,
            "no overflow before running"
        );
        step(&mut l, 20); // > one period
        assert_eq!(
            l.read_u32(INT_RAW).unwrap() & 1,
            1,
            "LSTIMER0_OVF latches on wrap"
        );
    }

    #[test]
    fn pause_freezes_the_counter() {
        let mut l = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
        l.write_u32(TIMER0_CONF_OFF, conf(13, 1)).unwrap();
        step(&mut l, 50);
        let v = l.read_u32(TIMER0_VALUE_OFF).unwrap();
        // Re-write the same config with PAUSE set.
        l.write_u32(TIMER0_CONF_OFF, conf(13, 1) | PAUSE_BIT)
            .unwrap();
        step(&mut l, 100);
        assert_eq!(
            l.read_u32(TIMER0_VALUE_OFF).unwrap(),
            v,
            "paused counter does not advance"
        );
    }

    #[test]
    fn reset_clears_the_counter() {
        let mut l = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
        l.write_u32(TIMER0_CONF_OFF, conf(13, 1)).unwrap();
        step(&mut l, 50);
        assert!(l.read_u32(TIMER0_VALUE_OFF).unwrap() > 0);
        l.write_u32(TIMER0_CONF_OFF, conf(13, 1) | RST_BIT).unwrap();
        assert_eq!(
            l.read_u32(TIMER0_VALUE_OFF).unwrap(),
            0,
            "RST holds the counter at 0"
        );
        step(&mut l, 50);
        assert_eq!(
            l.read_u32(TIMER0_VALUE_OFF).unwrap(),
            0,
            "counter stays at 0 while RST is held"
        );
    }

    #[test]
    fn int_st_masks_with_ena_and_emits_source() {
        let mut l = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
        l.write_u32(TIMER0_CONF_OFF, conf(4, 1)).unwrap();
        step(&mut l, 20);
        assert_ne!(l.read_u32(INT_RAW).unwrap() & 1, 0, "raw overflow set");
        assert_eq!(l.read_u32(INT_ST).unwrap(), 0, "ST gated by ENA");
        l.write_u32(INT_ENA, 1).unwrap();
        assert_eq!(l.read_u32(INT_ST).unwrap() & 1, 1, "ST follows raw & ena");
        assert_eq!(l.tick().explicit_irqs, Some(vec![LEDC_INTR_SOURCE_ID]));
    }

    #[test]
    fn int_clr_is_w1c() {
        let mut l = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
        l.write_u32(TIMER0_CONF_OFF, conf(4, 1)).unwrap();
        step(&mut l, 20);
        assert_ne!(l.read_u32(INT_RAW).unwrap() & 1, 0);
        l.write_u32(INT_CLR, 1).unwrap();
        assert_eq!(l.read_u32(INT_RAW).unwrap() & 1, 0, "W1C clears overflow");
    }

    #[test]
    fn channel_registers_are_register_backed() {
        let mut l = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
        // CH0_DUTY @ 0x08.
        l.write_u32(0x08, 0x0007_FFFF).unwrap();
        assert_eq!(l.read_u32(0x08).unwrap(), 0x0007_FFFF);
    }
}
