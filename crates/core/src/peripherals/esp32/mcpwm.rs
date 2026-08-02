// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! MCPWM (Motor Control PWM) peripheral for ESP32-classic.
//!
//! Per ESP32 TRM v5.0 §16 and `mcpwm_reg.h`. MCPWM0 sits at `0x3FF5_E000`,
//! MCPWM1 at `0x3FF6_C000`. This models the **PWM generation path** that
//! determines a channel's output frequency and duty — the part motor/servo
//! firmware reads back and that a digital-twin actuator keys off:
//!
//!   * **Group clock** — `CLK_CFG.PRESCALE` (bits[7:0]) divides the 160 MHz
//!     PWM source: `group_clk = 160 MHz / (prescale + 1)`.
//!   * **3 timers** — `TIMERn_CFG0` (`0x04 + n*0x10`): `PRESCALE` (bits[7:0])
//!     and `PERIOD` (bits[23:8]). The timer counts `0..PERIOD`, so
//!     `f_pwm = group_clk / ((timer_prescale + 1) * PERIOD)`.
//!   * **3 operators** — `OPERATOR_TIMERSEL` (`0x38`) binds each operator to a
//!     timer (2 bits each). Each operator's comparator A
//!     (`GENn_TSTMP_A`, `0x40 + n*0x38`, bits[15:0]) is the duty compare:
//!     for the common up-count / high-at-zero / low-at-A generator config,
//!     `duty = compare_A / PERIOD` — matching `mcpwm_set_duty`.
//!
//! Like [`Ledc`](super::ledc), writes round-trip verbatim (so firmware probes
//! read back what they wrote) and the model exposes derived
//! [`Mcpwm::operator_duty_fraction`] / [`Mcpwm::operator_freq_hz`]. A
//! [`McpwmDutyObserver`] fires whenever an operator's compare-A is written,
//! so an actuator (e.g. a servo) can track the live duty without polling.
//!
//! Not modeled (round-tripped, no behavior): the generator action table,
//! dead-time, carrier, fault/trip, capture, and sync — secondary to the
//! frequency/duty a controller actually commands.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::collections::HashMap;
use std::sync::Arc;

/// Notified when an operator commits a new compare-A (duty) value, i.e. on
/// each `mcpwm_set_duty`. Lets PWM-driven actuators react to the live duty.
/// `duty_fraction` is `compare_A / PERIOD` for the operator's bound timer,
/// the same value [`Mcpwm::operator_duty_fraction`] returns.
pub trait McpwmDutyObserver: Send + Sync + std::fmt::Debug {
    fn on_duty_change(&self, operator: u64, duty_fraction: f64);
}

/// `CLK_CFG` — group-clock prescaler.
const CLK_CFG: u64 = 0x00;
/// `TIMER0_CFG0`; timers stride by `0x10`.
const TIMER0_CFG0: u64 = 0x04;
const TIMER_STRIDE: u64 = 0x10;
/// `OPERATOR_TIMERSEL` — 2 bits per operator selecting its timer.
const OPERATOR_TIMERSEL: u64 = 0x38;
/// `GEN0_TSTMP_A` — operator 0 comparator A; operators stride by `0x38`.
const GEN0_TSTMP_A: u64 = 0x40;
const OPERATOR_STRIDE: u64 = 0x38;

/// 16-bit field mask (PERIOD, compare-A).
const FIELD16: u32 = 0xFFFF;
/// 8-bit field mask (prescalers).
const PRESCALE_MASK: u32 = 0xFF;
/// PERIOD field shift in `TIMERn_CFG0`.
const PERIOD_SHIFT: u32 = 8;
/// MCPWM source clock — the 160 MHz PWM clock on ESP32-classic.
const PWM_CLK_HZ: u64 = 160_000_000;

/// ESP32-classic MCPWM peripheral. See module docs.
#[derive(Debug)]
pub struct Mcpwm {
    base: u32,
    /// Word-aligned register backing store; round-trips every write.
    regs: HashMap<u64, u32>,
    /// Actuators driven by this controller, notified on each duty commit.
    duty_observers: Vec<Arc<dyn McpwmDutyObserver>>,
}

impl Mcpwm {
    /// Canonical MMIO base of MCPWM0 on ESP32-classic.
    pub const BASE: u32 = 0x3FF5_E000;

    /// Construct a freshly-powered MCPWM block at `base`.
    pub fn new(base: u32) -> Self {
        Self {
            base,
            regs: HashMap::new(),
            duty_observers: Vec::new(),
        }
    }

    /// Base MMIO address (informational).
    pub fn base(&self) -> u32 {
        self.base
    }

    /// Register an actuator notified whenever an operator commits a new duty.
    pub fn add_duty_observer(&mut self, obs: Arc<dyn McpwmDutyObserver>) {
        self.duty_observers.push(obs);
    }

    fn word(&self, off: u64) -> u32 {
        self.regs.get(&(off & !3)).copied().unwrap_or(0)
    }

    fn timer_cfg0(&self, timer: u64) -> u32 {
        self.word(TIMER0_CFG0 + timer * TIMER_STRIDE)
    }

    /// `TIMERn.PERIOD` — counts the timer wraps at (bits[23:8]).
    pub fn timer_period(&self, timer: u64) -> u32 {
        (self.timer_cfg0(timer) >> PERIOD_SHIFT) & FIELD16
    }

    /// `TIMERn.PRESCALE` (bits[7:0]).
    pub fn timer_prescale(&self, timer: u64) -> u32 {
        self.timer_cfg0(timer) & PRESCALE_MASK
    }

    /// `CLK_CFG.PRESCALE` (bits[7:0]) — the group-clock divider.
    pub fn clk_prescale(&self) -> u32 {
        self.word(CLK_CFG) & PRESCALE_MASK
    }

    /// Which timer (0..2) `operator` is bound to (`OPERATOR_TIMERSEL`).
    pub fn operator_timer(&self, operator: u64) -> u64 {
        ((self.word(OPERATOR_TIMERSEL) >> (operator * 2)) & 0x3) as u64
    }

    /// `GENn_TSTMP_A` (bits[15:0]) — operator `operator`'s comparator A.
    pub fn operator_compare_a(&self, operator: u64) -> u32 {
        self.word(GEN0_TSTMP_A + operator * OPERATOR_STRIDE) & FIELD16
    }

    /// Duty as a fraction in `[0.0, 1.0]`: `compare_A / PERIOD` for the
    /// operator's bound timer (up-count, high-at-zero / low-at-A — the
    /// `mcpwm_set_duty` convention). Returns 0.0 when the timer's period is 0.
    pub fn operator_duty_fraction(&self, operator: u64) -> f64 {
        let period = self.timer_period(self.operator_timer(operator));
        if period == 0 {
            return 0.0;
        }
        (self.operator_compare_a(operator) as f64 / period as f64).clamp(0.0, 1.0)
    }

    /// Output frequency (Hz) for `operator`'s bound timer:
    /// `160 MHz / ((clk_prescale+1) * (timer_prescale+1) * PERIOD)`.
    /// Returns 0 when the timer is unconfigured (PERIOD = 0).
    pub fn operator_freq_hz(&self, operator: u64) -> u64 {
        let timer = self.operator_timer(operator);
        let period = self.timer_period(timer) as u64;
        if period == 0 {
            return 0;
        }
        let div =
            (self.clk_prescale() as u64 + 1) * (self.timer_prescale(timer) as u64 + 1) * period;
        PWM_CLK_HZ / div
    }

    /// Offset (within the window) of an operator's `GENn_TSTMP_A`, or `None`.
    fn operator_of_tstmp_a(word_off: u64) -> Option<u64> {
        (0..3u64).find(|&op| GEN0_TSTMP_A + op * OPERATOR_STRIDE == word_off)
    }

    fn apply_write_side_effects(&mut self, word_off: u64) {
        // Committing a new comparator A is `mcpwm_set_duty`: push the live
        // duty to bound actuators (the firmware-moves-servo path, no poll).
        if let Some(op) = Self::operator_of_tstmp_a(word_off) {
            if !self.duty_observers.is_empty() {
                let frac = self.operator_duty_fraction(op);
                for obs in &self.duty_observers {
                    obs.on_duty_change(op, frac);
                }
            }
        }
    }
}

impl Peripheral for Mcpwm {
    // Inert walk: MCPWM register bank (duty introspection, no waveform generation modeled); tick() is an explicit no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.word(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let shift = (offset & 3) * 8;
        let mut word = self.word(word_off);
        word &= !(0xFFu32 << shift);
        word |= (value as u32) << shift;
        self.regs.insert(word_off, word);
        self.apply_write_side_effects(word_off);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let word_off = offset & !3;
        self.regs.insert(word_off, value);
        self.apply_write_side_effects(word_off);
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(p: &mut Mcpwm, off: u64, val: u32) {
        // Firmware writes registers as 32-bit words (s32i), not byte-by-byte.
        p.write_u32(off, val).unwrap();
    }
    fn r(p: &Mcpwm, off: u64) -> u32 {
        let mut v = 0u32;
        for i in 0..4u64 {
            v |= (p.read(off + i).unwrap() as u32) << (i * 8);
        }
        v
    }

    /// Configure timer 0 for ~50 Hz at the given PERIOD, operator 0 → timer 0.
    /// 160 MHz / ((clk+1)(tp+1)*PERIOD) = 50 Hz with clk=159, tp=0, PERIOD=20000.
    fn setup_50hz(p: &mut Mcpwm) {
        w(p, CLK_CFG, 159); // group_clk = 160e6/160 = 1 MHz
        w(p, TIMER0_CFG0, 20000 << PERIOD_SHIFT); // prescale 0, period 20000 → 1MHz/20000 = 50 Hz
        w(p, OPERATOR_TIMERSEL, 0); // operator 0 → timer 0
    }

    #[test]
    fn registers_round_trip() {
        let mut p = Mcpwm::new(Mcpwm::BASE);
        let timer1_cfg0 = TIMER0_CFG0 + TIMER_STRIDE;
        w(&mut p, timer1_cfg0, 0x00AB_CD12);
        assert_eq!(r(&p, timer1_cfg0), 0x00AB_CD12);
    }

    #[test]
    fn derives_50hz_frequency() {
        let mut p = Mcpwm::new(Mcpwm::BASE);
        setup_50hz(&mut p);
        assert_eq!(p.operator_freq_hz(0), 50);
        assert_eq!(p.timer_period(0), 20000);
    }

    #[test]
    fn duty_fraction_from_compare_a() {
        let mut p = Mcpwm::new(Mcpwm::BASE);
        setup_50hz(&mut p);
        // 1.5 ms / 20 ms = 0.075 → compare = 0.075 * 20000 = 1500.
        w(&mut p, GEN0_TSTMP_A, 1500);
        assert!((p.operator_duty_fraction(0) - 0.075).abs() < 1e-6);
    }

    #[test]
    fn operator_timer_select() {
        let mut p = Mcpwm::new(Mcpwm::BASE);
        // operator 1 → timer 2 (bits [3:2] = 2).
        w(&mut p, OPERATOR_TIMERSEL, 2 << 2);
        assert_eq!(p.operator_timer(1), 2);
        // operator 0 still timer 0.
        assert_eq!(p.operator_timer(0), 0);
    }

    #[test]
    fn unconfigured_timer_is_safe() {
        let p = Mcpwm::new(Mcpwm::BASE);
        assert_eq!(p.operator_duty_fraction(0), 0.0);
        assert_eq!(p.operator_freq_hz(0), 0);
    }

    #[test]
    fn duty_observer_fires_on_compare_write() {
        use std::sync::Mutex;
        #[derive(Debug)]
        struct Rec(Mutex<Vec<(u64, f64)>>);
        impl McpwmDutyObserver for Rec {
            fn on_duty_change(&self, op: u64, frac: f64) {
                self.0.lock().unwrap().push((op, frac));
            }
        }
        let rec = Arc::new(Rec(Mutex::new(Vec::new())));
        let mut p = Mcpwm::new(Mcpwm::BASE);
        p.add_duty_observer(Arc::clone(&rec) as Arc<dyn McpwmDutyObserver>);
        setup_50hz(&mut p);
        w(&mut p, GEN0_TSTMP_A, 2000); // 0.10 duty
        let got = rec.0.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, 0);
        assert!((got[0].1 - 0.10).abs() < 1e-6);
    }

    #[test]
    fn mcpwm_drives_a_bound_servo_to_center() {
        use crate::peripherals::components::servo::{McpwmServoDriver, Servo};
        // Bind a servo to MCPWM operator 0, then move it purely by writing
        // MCPWM registers (mcpwm_init + mcpwm_set_duty) — no glue.
        let mut p = Mcpwm::new(Mcpwm::BASE);
        let servo = Arc::new(Servo::standard(15));
        p.add_duty_observer(Arc::new(McpwmServoDriver::new(0, Arc::clone(&servo))));
        setup_50hz(&mut p);
        // mcpwm_set_duty center: 1.5 ms / 20 ms = 0.075 → compare = 1500.
        w(&mut p, GEN0_TSTMP_A, 1500);
        assert!(
            (servo.angle_degrees() - 90.0).abs() < 1.0,
            "angle {}",
            servo.angle_degrees()
        );
    }
}
