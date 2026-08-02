// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-classic LED PWM Controller (LEDC) peripheral.
//!
//! Reference: ESP32 TRM v5.0 §14 ("LED PWM Controller"). The LEDC block
//! sits at base `0x3FF5_9000`, spans one 4 KiB APB page, and generates
//! PWM waveforms on up to 16 channels: 8 **high-speed** (HS) and 8
//! **low-speed** (LS), each bound to one of 4 HS / 4 LS timers. It is the
//! peripheral Arduino's `ledcSetup`/`ledcWrite` and ESP-IDF's `ledc_*`
//! driver program to dim LEDs, drive servos, and synthesize audio.
//!
//! Previously this address window was a generic read-as-zero round-trip
//! stub. That satisfies "don't fault", but firmware that configures a
//! channel and then reads back its duty (or reads the per-channel
//! `DUTY_R` "current duty" register the hardware latches) saw zero and
//! could not introspect its own output. This model replaces the stub
//! with a register-level state machine that:
//!
//!   * Lays out the real HS/LS channel + timer + interrupt + global
//!     register blocks at their TRM offsets and round-trips every write.
//!   * Implements the `CONF1.DUTY_START` strobe: on silicon, the staged
//!     `DUTY` value (Q-format, 4 fractional bits) is committed to the
//!     channel's live `DUTY_R` shadow when firmware sets DUTY_START. We
//!     latch `DUTY → DUTY_R` so a read of `DUTY_R` returns the value that
//!     is actually driving the pin, matching `ledc_get_duty()`.
//!   * Computes the live **duty percentage** and **PWM frequency** for a
//!     channel from its bound timer's `DUTY_RES` + `DIV_NUM` divider, so
//!     a UI or test can read back the effective output without re-deriving
//!     the divider math (see [`Ledc::channel_duty_fraction`] /
//!     [`Ledc::channel_freq_hz`]).
//!   * Implements `INT_CLR` write-1-to-clear against `INT_RAW`, mirroring
//!     the TIMG interrupt-plumbing convention already in this crate.
//!
//! ## Register map (per ESP32 TRM v5.0 §14.5, matching ESP-IDF
//! `soc/esp32/include/soc/ledc_reg.h`)
//!
//! | Offset  | Name                  | Semantics modeled                        |
//! |--------:|-----------------------|------------------------------------------|
//! | 0x0000  | HSCH0_CONF0           | TIMER_SEL[1:0], SIG_OUT_EN[2], IDLE_LV[3]|
//! | 0x0004  | HSCH0_HPOINT          | Round-trip (phase / high-point)          |
//! | 0x0008  | HSCH0_DUTY            | Staged duty, Q4 fixed-point              |
//! | 0x000C  | HSCH0_CONF1           | DUTY_START[31] strobe → latch DUTY→DUTY_R|
//! | 0x0010  | HSCH0_DUTY_R          | Read-only live duty shadow (latched)     |
//! | ...     | HSCH1..7 (stride 0x14)| Same layout, +0x14 per channel           |
//! | 0x00A0  | LSCH0_CONF0           | Low-speed channel 0 (same layout)        |
//! | ...     | LSCH1..7 (stride 0x14)|                                          |
//! | 0x0140  | HSTIMER0_CONF         | DUTY_RES[4:0], DIV_NUM[17:5], PAUSE[18], RST[19], TICK_SEL[20] |
//! | 0x0144  | HSTIMER0_VALUE        | Read-only running counter (round-trip)   |
//! | ...     | HSTIMER1..3 (stride 8)|                                          |
//! | 0x0160  | LSTIMER0_CONF         | Low-speed timer 0 (same layout)          |
//! | ...     | LSTIMER1..3 (stride 8)|                                          |
//! | 0x0180  | INT_RAW               | Round-trip (no auto-set today)           |
//! | 0x0184  | INT_ST                | Round-trip                               |
//! | 0x0188  | INT_ENA               | Round-trip                               |
//! | 0x018C  | INT_CLR               | Write-1-to-clear matching INT_RAW bits   |
//! | 0x0190  | CONF                  | APB_CLK_SEL[1:0] (1=APB80M, 2=RTC8M)     |
//! | 0x01FC  | DATE                  | Silicon date/version (canned)            |
//!
//! Any offset not enumerated above falls through to the generic word
//! store, so RMW probes from firmware still observe their own writes.
//!
//! This model is **functional, not cycle-accurate**: the timer VALUE
//! counters are not advanced and no PWM edges are emitted to GPIO (LEDC
//! output routing through the GPIO matrix is not modeled). Its job is to
//! make LEDC configuration introspectable — duty-cycle readback and the
//! derived frequency/fraction helpers — which is what firmware and a
//! waveform UI actually consume.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::collections::HashMap;
use std::sync::Arc;

/// Notified when a channel commits a new duty via the `CONF1.DUTY_START`
/// strobe (i.e. on each `ledcWrite`). Lets PWM-driven actuators — a servo,
/// an ESC, an LED dimmer — react to the live duty without polling. The
/// `duty_fraction` is `duty / 2^DUTY_RES` for the channel's bound timer,
/// the same value [`Ledc::channel_duty_fraction`] returns.
pub trait LedcDutyObserver: Send + Sync + std::fmt::Debug {
    fn on_duty_change(&self, channel: u64, duty_fraction: f64);
}

// ── Channel block geometry (TRM §14.5) ───────────────────────────────────
/// First HS channel register block starts at offset 0.
const HS_CH_BASE: u64 = 0x0000;
/// First LS channel register block.
const LS_CH_BASE: u64 = 0x00A0;
/// Per-channel register stride: CONF0, HPOINT, DUTY, CONF1, DUTY_R (5 × 4).
const CH_STRIDE: u64 = 0x14;
/// Channels per speed-mode group.
const CH_COUNT: u64 = 8;

// Per-channel register offsets, relative to the channel block base.
const CH_CONF0: u64 = 0x00;
#[allow(dead_code)]
const CH_HPOINT: u64 = 0x04;
const CH_DUTY: u64 = 0x08;
const CH_CONF1: u64 = 0x0C;
const CH_DUTY_R: u64 = 0x10;

/// CONF1.DUTY_START — bit 31. Setting it commits the staged DUTY into the
/// live DUTY_R shadow (one-shot on silicon; we latch on the write).
const CONF1_DUTY_START_BIT: u32 = 1 << 31;
/// CONF0.SIG_OUT_EN — bit 2. Channel output enable.
const CONF0_SIG_OUT_EN_BIT: u32 = 1 << 2;
/// CONF0.TIMER_SEL — bits[1:0]. Which of the 4 timers in this speed mode
/// the channel is bound to.
const CONF0_TIMER_SEL_MASK: u32 = 0b11;

// ── Timer block geometry (TRM §14.5) ─────────────────────────────────────
/// First HS timer CONF register.
const HS_TIMER_BASE: u64 = 0x0140;
/// First LS timer CONF register.
const LS_TIMER_BASE: u64 = 0x0160;
/// Per-timer register stride: CONF, VALUE (2 × 4).
const TIMER_STRIDE: u64 = 0x08;
/// Timers per speed-mode group.
const TIMER_COUNT: u64 = 4;

const TIMER_CONF: u64 = 0x00;
#[allow(dead_code)]
const TIMER_VALUE: u64 = 0x04;

/// TIMER_CONF.DUTY_RES — bits[4:0]. Number of duty resolution bits; the
/// counter counts 0..(2^DUTY_RES - 1).
const TIMER_DUTY_RES_MASK: u32 = 0x1F;
/// TIMER_CONF.DIV_NUM — bits[17:5]. Fixed-point Q8 divider applied to the
/// source clock: actual divider = DIV_NUM / 256.
const TIMER_DIV_NUM_SHIFT: u32 = 5;
const TIMER_DIV_NUM_MASK: u32 = 0x1FFF;

// ── Interrupt + global registers (TRM §14.5) ─────────────────────────────
const INT_RAW: u64 = 0x0180;
#[allow(dead_code)]
const INT_ST: u64 = 0x0184;
#[allow(dead_code)]
const INT_ENA: u64 = 0x0188;
const INT_CLR: u64 = 0x018C;
/// CONF.APB_CLK_SEL — global clock-source mux. Round-trips through the
/// generic word store; referenced by name in tests for spec clarity.
#[allow(dead_code)]
const CONF: u64 = 0x0190;
const DATE: u64 = 0x01FC;

/// Reset value of LEDC_DATE on ESP32-classic (per `ledc_reg.h`). Firmware
/// rarely reads it, but returning the silicon constant keeps version
/// probes faithful instead of read-as-zero.
const DATE_RESET: u32 = 0x1808_2600;

/// LEDC source clocks (TRM §14.2). HS channels are clocked from APB
/// (80 MHz) or the 8 MHz RTC clock per CONF.APB_CLK_SEL; LS channels
/// likewise. We model the common APB-80M default for the derived
/// frequency helper.
const APB_CLK_HZ: u64 = 80_000_000;

/// LED PWM Controller (LEDC) peripheral model.
#[derive(Debug)]
pub struct Ledc {
    /// MMIO base (0x3FF5_9000). Informational; the bus dispatches by
    /// offset so this is only for logs / multi-instance disambiguation.
    base: u32,
    /// Word-aligned register backing store. Every offset round-trips
    /// here; side-effecting offsets are intercepted before storing.
    regs: HashMap<u64, u32>,
    /// Actuators driven by this controller's PWM output, notified on each
    /// duty latch. Runtime wiring — not part of the register snapshot.
    duty_observers: Vec<Arc<dyn LedcDutyObserver>>,
}

impl Default for Ledc {
    fn default() -> Self {
        Self::new(Self::BASE)
    }
}

impl Ledc {
    /// Canonical MMIO base address on ESP32-classic (TRM §14.5).
    pub const BASE: u32 = 0x3FF5_9000;

    /// Construct a freshly-powered LEDC block. Seeds only LEDC_DATE with
    /// its silicon constant; every other register resets to zero.
    pub fn new(base: u32) -> Self {
        let mut regs = HashMap::new();
        regs.insert(DATE, DATE_RESET);
        Self {
            base,
            regs,
            duty_observers: Vec::new(),
        }
    }

    /// Register an actuator to be notified whenever a channel latches a new
    /// duty (each `ledcWrite`). See [`LedcDutyObserver`].
    pub fn add_duty_observer(&mut self, obs: Arc<dyn LedcDutyObserver>) {
        self.duty_observers.push(obs);
    }

    /// Base MMIO address (debug helper).
    pub fn base(&self) -> u32 {
        self.base
    }

    fn word(&self, off: u64) -> u32 {
        self.regs.get(&off).copied().unwrap_or(0)
    }

    /// Offset of channel `ch` (0..15, where 0..7 = HS, 8..15 = LS) register
    /// `reg` (one of CH_CONF0/CH_DUTY/CH_CONF1/CH_DUTY_R).
    fn ch_reg(ch: u64, reg: u64) -> u64 {
        if ch < CH_COUNT {
            HS_CH_BASE + ch * CH_STRIDE + reg
        } else {
            LS_CH_BASE + (ch - CH_COUNT) * CH_STRIDE + reg
        }
    }

    /// Offset of timer `tmr` (0..7, 0..3 = HS, 4..7 = LS) register `reg`.
    fn timer_reg(tmr: u64, reg: u64) -> u64 {
        if tmr < TIMER_COUNT {
            HS_TIMER_BASE + tmr * TIMER_STRIDE + reg
        } else {
            LS_TIMER_BASE + (tmr - TIMER_COUNT) * TIMER_STRIDE + reg
        }
    }

    /// If `word_off` is a channel CONF1 offset, return the channel index.
    fn channel_of_conf1(word_off: u64) -> Option<u64> {
        (0..(2 * CH_COUNT)).find(|&ch| Self::ch_reg(ch, CH_CONF1) == word_off)
    }

    /// Commit a channel's staged DUTY into its DUTY_R live shadow. The
    /// hardware copies `DUTY` (Q4 fixed-point) into the running duty
    /// register when CONF1.DUTY_START is asserted; `ledc_get_duty()`
    /// reads the integer part back from DUTY_R.
    fn latch_duty(&mut self, ch: u64) {
        let staged = self.word(Self::ch_reg(ch, CH_DUTY));
        self.regs.insert(Self::ch_reg(ch, CH_DUTY_R), staged);
    }

    /// Which timer (global index 0..7) channel `ch` is bound to, per its
    /// CONF0.TIMER_SEL field and speed mode.
    fn channel_timer(&self, ch: u64) -> u64 {
        let conf0 = self.word(Self::ch_reg(ch, CH_CONF0));
        let sel = (conf0 & CONF0_TIMER_SEL_MASK) as u64;
        if ch < CH_COUNT {
            sel // HS timer 0..3
        } else {
            TIMER_COUNT + sel // LS timer 4..7
        }
    }

    /// Live integer duty value (DUTY_R >> 4 — the 4 LSBs are the
    /// fractional part of the Q4 duty register).
    pub fn channel_duty(&self, ch: u64) -> u32 {
        self.word(Self::ch_reg(ch, CH_DUTY_R)) >> 4
    }

    /// True if the channel's output stage is enabled (CONF0.SIG_OUT_EN).
    pub fn channel_output_enabled(&self, ch: u64) -> bool {
        self.word(Self::ch_reg(ch, CH_CONF0)) & CONF0_SIG_OUT_EN_BIT != 0
    }

    /// Duty as a fraction in [0.0, 1.0] given the bound timer's resolution
    /// (`duty / 2^DUTY_RES`). Returns 0.0 if the timer's DUTY_RES is 0
    /// (period of one tick — degenerate, no PWM).
    pub fn channel_duty_fraction(&self, ch: u64) -> f64 {
        let tmr = self.channel_timer(ch);
        let conf = self.word(Self::timer_reg(tmr, TIMER_CONF));
        let duty_res = conf & TIMER_DUTY_RES_MASK;
        if duty_res == 0 {
            return 0.0;
        }
        let period = 1u64 << duty_res;
        self.channel_duty(ch) as f64 / period as f64
    }

    /// Derived PWM output frequency in Hz for channel `ch`, from its bound
    /// timer's divider and resolution, assuming the APB-80M source clock:
    /// `f = APB / ((DIV_NUM/256) * 2^DUTY_RES)`. Returns 0 when the timer
    /// is unconfigured (DUTY_RES=0 or DIV_NUM=0).
    pub fn channel_freq_hz(&self, ch: u64) -> u64 {
        let tmr = self.channel_timer(ch);
        let conf = self.word(Self::timer_reg(tmr, TIMER_CONF));
        let duty_res = conf & TIMER_DUTY_RES_MASK;
        let div_num = (conf >> TIMER_DIV_NUM_SHIFT) & TIMER_DIV_NUM_MASK;
        if duty_res == 0 || div_num == 0 {
            return 0;
        }
        let period = 1u64 << duty_res;
        // div_num is Q8 (×256). f = APB*256 / (div_num * period).
        let denom = div_num as u64 * period;
        (APB_CLK_HZ * 256) / denom
    }

    /// Dispatch the per-word side effects of a register write. Factored so
    /// byte-granular and word-granular writes produce identical state.
    /// Idempotent: each trigger reads live state and writes deterministic
    /// values, so one-per-word or four-per-word calls converge.
    fn apply_write_side_effects(&mut self, word_off: u64) {
        if let Some(ch) = Self::channel_of_conf1(word_off) {
            // DUTY_START strobe latches the staged duty into the live
            // shadow. We do NOT clear DUTY_START (silicon self-clears it
            // after the next timer overflow; firmware treats it as a
            // fire-and-forget command and re-asserts on each update).
            if self.word(word_off) & CONF1_DUTY_START_BIT != 0 {
                self.latch_duty(ch);
                // Push the freshly-latched duty to any bound actuators — the
                // `ledcWrite` → servo-moves path, with no polling.
                if !self.duty_observers.is_empty() {
                    let frac = self.channel_duty_fraction(ch);
                    for obs in &self.duty_observers {
                        obs.on_duty_change(ch, frac);
                    }
                }
            }
            return;
        }
        if word_off == INT_CLR {
            let mask = self.word(INT_CLR);
            let raw = self.word(INT_RAW);
            self.regs.insert(INT_RAW, raw & !mask);
        }
    }
}

impl Peripheral for Ledc {
    // Inert walk: classic-ESP32 LEDC is a config-introspection register bank — no PWM edges or timer-counter advance modeled (unlike the C3 LEDC, whose live up-counters DO real tick work); tick() is an explicit no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let entry = self.regs.entry(word_off).or_insert(0);
        *entry &= !(0xFFu32 << byte_off);
        *entry |= (value as u32) << byte_off;
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

    fn tick(&mut self) -> PeripheralTickResult {
        // No PWM edge emission or VALUE-counter advance modeled — see
        // module docs. Configuration introspection is the deliverable.
        PeripheralTickResult::default()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            base: u32,
            regs: Vec<(u64, u32)>,
        }
        let snap = Snap {
            base: self.base,
            regs: self.regs.iter().map(|(k, v)| (*k, *v)).collect(),
        };
        bincode::serialize(&snap).expect("bincode serialize Ledc")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            base: u32,
            regs: Vec<(u64, u32)>,
        }
        let snap: Snap = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("Ledc snapshot decode: {e}"))
        })?;
        self.base = snap.base;
        self.regs = snap.regs.into_iter().collect();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_u32(p: &mut Ledc, off: u64, val: u32) {
        for i in 0..4 {
            p.write(off + i, ((val >> (i * 8)) & 0xFF) as u8).unwrap();
        }
    }

    fn read_u32(p: &Ledc, off: u64) -> u32 {
        let mut v = 0u32;
        for i in 0..4 {
            v |= (p.read(off + i).unwrap() as u32) << (i * 8);
        }
        v
    }

    #[test]
    fn date_reads_silicon_constant() {
        let p = Ledc::new(Ledc::BASE);
        assert_eq!(read_u32(&p, DATE), DATE_RESET);
    }

    #[test]
    fn conf0_round_trips() {
        let mut p = Ledc::new(Ledc::BASE);
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF0), 0x0000_000D);
        assert_eq!(read_u32(&p, Ledc::ch_reg(0, CH_CONF0)), 0x0000_000D);
    }

    #[test]
    fn duty_start_latches_staged_duty_into_shadow() {
        // ledcWrite path: stage DUTY, then assert CONF1.DUTY_START. The
        // live DUTY_R shadow must reflect the staged value, and the
        // integer-duty helper must drop the 4 fractional LSBs.
        let mut p = Ledc::new(Ledc::BASE);
        // Stage duty = 512 in Q4 (i.e. integer 512 → register 512<<4).
        let staged = 512u32 << 4;
        write_u32(&mut p, Ledc::ch_reg(0, CH_DUTY), staged);
        // Before the strobe, DUTY_R is still zero.
        assert_eq!(read_u32(&p, Ledc::ch_reg(0, CH_DUTY_R)), 0);
        // Assert DUTY_START.
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF1), CONF1_DUTY_START_BIT);
        assert_eq!(read_u32(&p, Ledc::ch_reg(0, CH_DUTY_R)), staged);
        assert_eq!(p.channel_duty(0), 512);
    }

    #[test]
    fn conf1_without_duty_start_does_not_latch() {
        let mut p = Ledc::new(Ledc::BASE);
        write_u32(&mut p, Ledc::ch_reg(0, CH_DUTY), 100u32 << 4);
        // Write CONF1 with DUTY_START clear — shadow must stay 0.
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF1), 0x0000_0001);
        assert_eq!(read_u32(&p, Ledc::ch_reg(0, CH_DUTY_R)), 0);
    }

    #[test]
    fn low_speed_channel_block_is_independent() {
        // LS channel 0 is global index 8; its DUTY_R must not alias HS0.
        let mut p = Ledc::new(Ledc::BASE);
        write_u32(&mut p, Ledc::ch_reg(0, CH_DUTY), 10u32 << 4);
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF1), CONF1_DUTY_START_BIT);
        write_u32(&mut p, Ledc::ch_reg(8, CH_DUTY), 20u32 << 4);
        write_u32(&mut p, Ledc::ch_reg(8, CH_CONF1), CONF1_DUTY_START_BIT);
        assert_eq!(p.channel_duty(0), 10);
        assert_eq!(p.channel_duty(8), 20);
    }

    #[test]
    fn duty_fraction_uses_bound_timer_resolution() {
        // Bind HS channel 0 to HS timer 0, set DUTY_RES=10 (period 1024),
        // duty 256 → 25%.
        let mut p = Ledc::new(Ledc::BASE);
        // Timer 0 CONF: DUTY_RES=10.
        write_u32(&mut p, Ledc::timer_reg(0, TIMER_CONF), 10);
        // Channel 0 CONF0: TIMER_SEL=0, SIG_OUT_EN.
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF0), CONF0_SIG_OUT_EN_BIT);
        write_u32(&mut p, Ledc::ch_reg(0, CH_DUTY), 256u32 << 4);
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF1), CONF1_DUTY_START_BIT);
        assert!((p.channel_duty_fraction(0) - 0.25).abs() < 1e-9);
        assert!(p.channel_output_enabled(0));
    }

    #[test]
    fn channel_binds_to_selected_timer() {
        // Channel 1 selects timer 2; DUTY_RES lives on timer 2, not 0.
        let mut p = Ledc::new(Ledc::BASE);
        write_u32(&mut p, Ledc::timer_reg(2, TIMER_CONF), 8); // period 256
        write_u32(&mut p, Ledc::ch_reg(1, CH_CONF0), 2); // TIMER_SEL=2
        write_u32(&mut p, Ledc::ch_reg(1, CH_DUTY), 64u32 << 4);
        write_u32(&mut p, Ledc::ch_reg(1, CH_CONF1), CONF1_DUTY_START_BIT);
        assert!((p.channel_duty_fraction(1) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn derived_frequency_matches_divider_math() {
        // 5 kHz @ 13-bit resolution on APB-80M: DIV_NUM = APB*256 /
        // (freq * 2^res) = 80e6*256 / (5000 * 8192) = 500.
        let mut p = Ledc::new(Ledc::BASE);
        let duty_res = 13u32;
        let div_num = 500u32;
        let conf = duty_res | (div_num << TIMER_DIV_NUM_SHIFT);
        write_u32(&mut p, Ledc::timer_reg(0, TIMER_CONF), conf);
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF0), 0); // TIMER_SEL=0
        assert_eq!(p.channel_freq_hz(0), 5000);
    }

    #[test]
    fn unconfigured_timer_yields_zero_freq() {
        let p = Ledc::new(Ledc::BASE);
        assert_eq!(p.channel_freq_hz(0), 0);
        assert_eq!(p.channel_duty_fraction(0), 0.0);
    }

    #[test]
    fn int_clr_clears_matching_int_raw_bits() {
        let mut p = Ledc::new(Ledc::BASE);
        write_u32(&mut p, INT_RAW, 0b1011);
        write_u32(&mut p, INT_CLR, 0b0010);
        assert_eq!(read_u32(&p, INT_RAW), 0b1001);
    }

    #[test]
    fn conf_apb_clk_sel_round_trips() {
        let mut p = Ledc::new(Ledc::BASE);
        write_u32(&mut p, CONF, 0x0000_0001); // APB_CLK_SEL = APB80M
        assert_eq!(read_u32(&p, CONF), 0x0000_0001);
    }

    #[test]
    fn unknown_offsets_round_trip() {
        let mut p = Ledc::new(Ledc::BASE);
        write_u32(&mut p, 0x0C0, 0xCAFE_BABE);
        assert_eq!(read_u32(&p, 0x0C0), 0xCAFE_BABE);
    }

    #[test]
    fn base_is_esp32_classic_canonical_address() {
        assert_eq!(Ledc::new(Ledc::BASE).base(), 0x3FF5_9000);
    }

    #[test]
    fn runtime_snapshot_round_trip_preserves_state() {
        let mut p = Ledc::new(Ledc::BASE);
        write_u32(&mut p, Ledc::timer_reg(0, TIMER_CONF), 10);
        write_u32(&mut p, Ledc::ch_reg(0, CH_DUTY), 300u32 << 4);
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF1), CONF1_DUTY_START_BIT);
        let snap = p.runtime_snapshot();

        let mut restored = Ledc::new(0);
        restored.restore_runtime_snapshot(&snap).unwrap();
        assert_eq!(restored.base(), 0x3FF5_9000);
        assert_eq!(restored.channel_duty(0), 300);
        assert_eq!(read_u32(&restored, DATE), DATE_RESET);
    }

    #[test]
    fn ledc_duty_drives_a_servo_to_center() {
        use crate::peripherals::components::servo::Servo;
        // Mirror the Arduino `ledcSetup(ch, 50 Hz, 14-bit)` + `ledcWrite(ch,
        // duty)` sequence at the register level, then let a servo read the
        // resulting duty fraction back — the end-to-end PWM-drives-actuator
        // path. Channel 0 selects timer 0 by reset default (CONF0.TIMER_SEL=0).
        let mut p = Ledc::new(Ledc::BASE);
        // Timer 0: 14-bit duty resolution → period = 2^14 = 16384.
        write_u32(&mut p, Ledc::timer_reg(0, TIMER_CONF), 14);
        // ledcWrite(0, 1229): 1229 / 16384 ≈ 0.075 duty = 1.5 ms / 20 ms.
        write_u32(&mut p, Ledc::ch_reg(0, CH_DUTY), 1229u32 << 4);
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF1), CONF1_DUTY_START_BIT);

        let frac = p.channel_duty_fraction(0);
        assert!((frac - 0.075).abs() < 0.001, "duty fraction {frac}");

        let servo = Servo::standard(5);
        servo.apply_duty_fraction(frac);
        assert!(
            (servo.angle_degrees() - 90.0).abs() < 1.0,
            "servo angle {}",
            servo.angle_degrees()
        );
    }

    #[test]
    fn ledc_write_drives_a_bound_servo_with_no_glue() {
        use crate::peripherals::components::servo::{LedcServoDriver, Servo};
        use std::sync::Arc;
        // Bind a servo to LEDC channel 0 via the duty-observer hook, then move
        // it purely by writing LEDC registers (ledcSetup + ledcWrite) — no
        // manual apply_duty_fraction. This is the firmware-drives-actuator path.
        let mut p = Ledc::new(Ledc::BASE);
        let servo = Arc::new(Servo::standard(5));
        p.add_duty_observer(Arc::new(LedcServoDriver::new(0, Arc::clone(&servo))));

        // ledcSetup: timer 0, 14-bit resolution (period = 16384).
        write_u32(&mut p, Ledc::timer_reg(0, TIMER_CONF), 14);

        // ledcWrite(0, 819): 819/16384 ≈ 0.05 → 1.0 ms → 0°.
        write_u32(&mut p, Ledc::ch_reg(0, CH_DUTY), 819u32 << 4);
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF1), CONF1_DUTY_START_BIT);
        assert!(servo.angle_degrees() < 2.0, "min={}", servo.angle_degrees());

        // ledcWrite(0, 1638): 1638/16384 ≈ 0.10 → 2.0 ms → 180°.
        write_u32(&mut p, Ledc::ch_reg(0, CH_DUTY), 1638u32 << 4);
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF1), CONF1_DUTY_START_BIT);
        assert!(
            (servo.angle_degrees() - 180.0).abs() < 2.0,
            "max={}",
            servo.angle_degrees()
        );
    }
}
