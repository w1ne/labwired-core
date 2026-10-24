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
//! The model:
//!
//!   * Lays out the real HS/LS channel + timer + interrupt + global
//!     register blocks at their TRM offsets and round-trips every write.
//!   * Implements the `CONF1.DUTY_START` strobe: the staged `DUTY` value
//!     (Q-format, 4 fractional bits) is latched into the channel's
//!     `DUTY_R` shadow on the write, so `ledc_get_duty()` reads it back.
//!   * Computes the live **duty fraction** and **PWM frequency** for a
//!     channel ([`Ledc::channel_duty_fraction`] / [`Ledc::channel_freq_hz`]).
//!   * Implements `INT_CLR` write-1-to-clear against `INT_RAW`.
//!   * **Generates the PWM waveform** of every channel on a 16-line pad wire
//!     ([`PadLines`], lines `HS0..HS7`, `LS0..LS7`). The classic GPIO model
//!     binds those lines to the output-matrix signals `LEDC_HS_SIG_OUT0..7`
//!     (71..78) and `LEDC_LS_SIG_OUT0..7` (79..86), so a pad the firmware
//!     routes with `gpio_matrix_out` (what `ledcAttachPin` does) carries real
//!     edges for `read_gpio_pad`, the logic analyzer and `--watch-gpio`.
//!
//! ## Waveform (TRM §14.2)
//!
//! A timer counts its source clock divided by `DIV_NUM / 256` and wraps every
//! `2^DUTY_RES` counts. A channel output is high while
//! `HPOINT <= count < HPOINT + DUTY` (DUTY is `DUTY_R >> 4`), otherwise low;
//! with `SIG_OUT_EN` clear it rests at `IDLE_LV`. Source clock per timer:
//! `TICK_SEL = 0` → REF_TICK (1 MHz); `TICK_SEL = 1` → APB (80 MHz) for HS
//! timers, and for LS timers the slow clock `LEDC_CONF.APB_CLK_SEL`
//! selects (1 = APB 80 MHz, 0 = RTC8M 8 MHz). Counter tick `k` lands on CPU
//! cycle `start + floor(k * DIV_NUM * cpu_hz / (256 * src_hz))`, so the
//! fractional divider is exact on average, as on silicon (the hardware
//! dithers the fraction; the per-tick jitter is below one source clock).
//! A duty latched by `DUTY_START` takes effect at the next timer overflow,
//! as the TRM specifies, so a frame is never cut short. A timer is stopped
//! while `PAUSE`/`RST` is set, `DUTY_RES` is 0 or `DIV_NUM < 256`; a change
//! of its configuration restarts its counter at 0. The duty-fade engine
//! (`DUTY_INC`/`DUTY_NUM`/`DUTY_CYCLE`/`DUTY_SCALE`) and the dithered
//! fractional duty bits are not modelled.
//!
//! Edges are published at their exact cycle (`PadLines::set_line_at`), and
//! the scheduler is woken on each one, so a pad read between MMIO accesses
//! follows the waveform too.
//!
//! ## Register map (per ESP32 TRM v5.0 §14.5, matching ESP-IDF
//! `soc/esp32/include/soc/ledc_reg.h`)
//!
//! | Offset  | Name                  | Semantics modeled                        |
//! |--------:|-----------------------|------------------------------------------|
//! | 0x0000  | HSCH0_CONF0           | TIMER_SEL[1:0], SIG_OUT_EN[2], IDLE_LV[3]|
//! | 0x0004  | HSCH0_HPOINT          | HPOINT[19:0]                             |
//! | 0x0008  | HSCH0_DUTY            | Staged duty, Q4 fixed-point              |
//! | 0x000C  | HSCH0_CONF1           | DUTY_START[31] strobe → latch DUTY→DUTY_R|
//! | 0x0010  | HSCH0_DUTY_R          | Read-only live duty shadow (latched)     |
//! | ...     | HSCH1..7 (stride 0x14)| Same layout, +0x14 per channel           |
//! | 0x00A0  | LSCH0_CONF0           | Low-speed channel 0 (same layout)        |
//! | ...     | LSCH1..7 (stride 0x14)|                                          |
//! | 0x0140  | HSTIMER0_CONF         | DUTY_RES[4:0], DIV_NUM[22:5], PAUSE[23], RST[24], TICK_SEL[25] |
//! | 0x0144  | HSTIMER0_VALUE        | Round-trip (counter not read back)       |
//! | ...     | HSTIMER1..3 (stride 8)|                                          |
//! | 0x0160  | LSTIMER0_CONF         | Low-speed timer 0 (same layout)          |
//! | ...     | LSTIMER1..3 (stride 8)|                                          |
//! | 0x0180  | INT_RAW               | Round-trip (no auto-set today)           |
//! | 0x0184  | INT_ST                | Round-trip                               |
//! | 0x0188  | INT_ENA               | Round-trip                               |
//! | 0x018C  | INT_CLR               | Write-1-to-clear matching INT_RAW bits   |
//! | 0x0190  | CONF                  | APB_CLK_SEL[0] (1=APB80M, 0=RTC8M)       |
//! | 0x01FC  | DATE                  | Silicon date/version (canned)            |
//!
//! Any offset not enumerated above falls through to the generic word
//! store, so RMW probes from firmware still observe their own writes.

use crate::peripherals::pad_lines::PadLines;
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::sync::Arc;

pub use crate::peripherals::LedcDutyObserver;

/// Pad-wire line names, one per channel: HS0..HS7 then LS0..LS7, the order of
/// the output-matrix signals `LEDC_HS_SIG_OUT0_IDX` (71) ..
/// `LEDC_LS_SIG_OUT7_IDX` (86) in esp-idf `soc/esp32/include/soc/gpio_sig_map.h`.
pub const LEDC_LINES: &[&str] = &[
    "HS0", "HS1", "HS2", "HS3", "HS4", "HS5", "HS6", "HS7", "LS0", "LS1", "LS2", "LS3", "LS4",
    "LS5", "LS6", "LS7",
];

// ── Channel block geometry (TRM §14.5) ───────────────────────────────────
/// First HS channel register block starts at offset 0.
const HS_CH_BASE: u64 = 0x0000;
/// First LS channel register block.
const LS_CH_BASE: u64 = 0x00A0;
/// Per-channel register stride: CONF0, HPOINT, DUTY, CONF1, DUTY_R (5 × 4).
const CH_STRIDE: u64 = 0x14;
/// Channels per speed-mode group.
const CH_COUNT: u64 = 8;
/// All channels (HS + LS).
const NUM_CH: usize = 16;

// Per-channel register offsets, relative to the channel block base.
const CH_CONF0: u64 = 0x00;
const CH_HPOINT: u64 = 0x04;
const CH_DUTY: u64 = 0x08;
const CH_CONF1: u64 = 0x0C;
const CH_DUTY_R: u64 = 0x10;

/// CONF1.DUTY_START — bit 31. Setting it commits the staged DUTY into the
/// live DUTY_R shadow (one-shot on silicon; we latch on the write).
const CONF1_DUTY_START_BIT: u32 = 1 << 31;
/// CONF0.SIG_OUT_EN — bit 2. Channel output enable.
const CONF0_SIG_OUT_EN_BIT: u32 = 1 << 2;
/// CONF0.IDLE_LV — bit 3. Output level while SIG_OUT_EN is clear.
const CONF0_IDLE_LV_BIT: u32 = 1 << 3;
/// CONF0.TIMER_SEL — bits[1:0]. Which of the 4 timers in this speed mode
/// the channel is bound to.
const CONF0_TIMER_SEL_MASK: u32 = 0b11;
/// HPOINT — bits[19:0] (`LEDC_HPOINT_HSCH0_V = 0xFFFFF`).
const HPOINT_MASK: u32 = 0xF_FFFF;

// ── Timer block geometry (TRM §14.5) ─────────────────────────────────────
/// First HS timer CONF register.
const HS_TIMER_BASE: u64 = 0x0140;
/// First LS timer CONF register.
const LS_TIMER_BASE: u64 = 0x0160;
/// Per-timer register stride: CONF, VALUE (2 × 4).
const TIMER_STRIDE: u64 = 0x08;
/// Timers per speed-mode group.
const TIMER_COUNT: u64 = 4;
/// All timers (HS + LS).
const NUM_TIMERS: usize = 8;

const TIMER_CONF: u64 = 0x00;
#[allow(dead_code)]
const TIMER_VALUE: u64 = 0x04;

/// TIMER_CONF.DUTY_RES — bits[4:0]. Number of duty resolution bits; the
/// counter counts 0..(2^DUTY_RES - 1).
const TIMER_DUTY_RES_MASK: u32 = 0x1F;
/// TIMER_CONF.DIV_NUM — bits[22:5] (`LEDC_DIV_NUM_HSTIMER0_V = 0x3FFFF`).
/// Fixed-point Q8 divider applied to the source clock: divider = DIV_NUM / 256.
const TIMER_DIV_NUM_SHIFT: u32 = 5;
const TIMER_DIV_NUM_MASK: u32 = 0x3_FFFF;
/// TIMER_CONF.PAUSE — bit 23.
const TIMER_PAUSE_BIT: u32 = 1 << 23;
/// TIMER_CONF.RST — bit 24. Holds the counter at 0.
const TIMER_RST_BIT: u32 = 1 << 24;
/// TIMER_CONF.TICK_SEL — bit 25. 0 = REF_TICK, 1 = APB (HS) / SLOW_CLK (LS).
const TIMER_TICK_SEL_BIT: u32 = 1 << 25;

// ── Interrupt + global registers (TRM §14.5) ─────────────────────────────
const INT_RAW: u64 = 0x0180;
#[allow(dead_code)]
const INT_ST: u64 = 0x0184;
#[allow(dead_code)]
const INT_ENA: u64 = 0x0188;
const INT_CLR: u64 = 0x018C;
/// CONF.APB_CLK_SEL — bit 0: the LS slow clock, 1 = APB, 0 = RTC8M.
const CONF: u64 = 0x0190;
const CONF_APB_CLK_SEL_BIT: u32 = 1;
const DATE: u64 = 0x01FC;

/// Reset value of LEDC_DATE on ESP32-classic (per `ledc_reg.h`). Firmware
/// rarely reads it, but returning the silicon constant keeps version
/// probes faithful instead of read-as-zero.
const DATE_RESET: u32 = 0x1808_2600;

/// APB clock (TRM §14.2) — the derived-frequency helper's assumption too.
const APB_CLK_HZ: u64 = 80_000_000;
/// REF_TICK: APB / (APB_CTRL_APB_TICK_CONF + 1), 1 MHz as esp-idf programs it
/// (`REF_CLK_FREQ`).
const REF_TICK_HZ: u64 = 1_000_000;
/// RTC8M, the LS slow clock with `APB_CLK_SEL = 0`.
const RTC8M_HZ: u64 = 8_000_000;
/// Classic ESP32 core clock (`configs/chips/esp32.yaml`), used until the bus
/// hands over its own `cpu_hz` (a hand-built bus may hand over 0).
const DEFAULT_CPU_HZ: u64 = 240_000_000;

/// A timer that is counting: where its counter was 0 and how fast it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct TimerRun {
    /// CPU cycle of counter tick 0.
    start: u64,
    /// Cycles per counter tick = `num / den` (exact rational).
    num: u64,
    den: u64,
    /// Ticks per period, `2^DUTY_RES`.
    period: u64,
}

impl TimerRun {
    /// CPU cycle counter tick `k` lands on.
    fn cycle_of(&self, k: u64) -> u64 {
        self.start + ((k as u128 * self.num as u128) / self.den as u128) as u64
    }

    /// The last tick that has landed at or before cycle `c` (`c >= start`).
    fn tick_at(&self, c: u64) -> u64 {
        // cycle_of(k) <= c  <=>  k < (c - start + 1) * den / num
        let lim = (c - self.start + 1) as u128 * self.den as u128;
        (lim.div_ceil(self.num as u128) - 1) as u64
    }
}

/// One channel's shape inside a period, in counter ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
struct Shape {
    hpoint: u64,
    duty: u64,
}

/// Waveform state that is not register state.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct WaveState {
    /// CPU cycle the waveform has been published up to.
    anchor: u64,
    timers: [Option<TimerRun>; NUM_TIMERS],
    /// The shape each channel is generating now.
    active: [Shape; NUM_CH],
    /// A latched shape waiting for its timer's next overflow: (shape, first
    /// tick it applies from).
    pending: [Option<(Shape, u64)>; NUM_CH],
}

/// LED PWM Controller (LEDC) peripheral model.
#[derive(Debug)]
pub struct Ledc {
    /// MMIO base (0x3FF5_9000). Informational; the bus dispatches by
    /// offset so this is only for logs / multi-instance disambiguation.
    base: u32,
    /// Word-aligned register backing store. Every offset round-trips
    /// here; side-effecting offsets are intercepted before storing.
    regs: crate::FastMap<u64, u32>,
    /// Actuators driven by this controller's PWM output, notified on each
    /// duty latch. Runtime wiring — not part of the register snapshot.
    duty_observers: Vec<Arc<dyn LedcDutyObserver>>,
    wave: WaveState,
    cpu_hz: u64,
    clock: Option<CycleClock>,
    /// Per-channel pad wire, created at bus wiring time.
    lines: Option<Arc<PadLines>>,
    /// Token of the one live scheduler wake; any other token is stale.
    arm_seq: u32,
    /// Absolute cycle the live wake targets, `None` when none is in flight.
    armed_target: Option<u64>,
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
        let mut regs: crate::FastMap<u64, u32> = crate::FastMap::default();
        regs.insert(DATE, DATE_RESET);
        Self {
            base,
            regs,
            duty_observers: Vec::new(),
            wave: WaveState::default(),
            cpu_hz: DEFAULT_CPU_HZ,
            clock: None,
            lines: None,
            arm_seq: 0,
            armed_target: None,
        }
    }

    /// Register an actuator to be notified whenever a channel latches a new
    /// duty (each `ledcWrite`). See [`LedcDutyObserver`].
    pub fn add_duty_observer(&mut self, obs: Arc<dyn LedcDutyObserver>) {
        self.duty_observers.push(obs);
    }

    /// This model's 16-line pad wire (HS0..HS7, LS0..LS7). Created at bus
    /// wiring time by `SystemBus::wire_esp32_ledc_pads`; the lines start at
    /// each channel's current level (all low at reset).
    pub(crate) fn pad_lines_arc(&mut self) -> Arc<PadLines> {
        if self.lines.is_none() {
            let idle: Vec<bool> = (0..NUM_CH)
                .map(|ch| self.level_at(ch, self.wave.anchor))
                .collect();
            self.lines = Some(Arc::new(PadLines::new(LEDC_LINES, &idle)));
        }
        self.lines.clone().expect("just created")
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

    /// If `word_off` is register `reg` of some channel, that channel.
    fn channel_of(word_off: u64, reg: u64) -> Option<u64> {
        (0..(2 * CH_COUNT)).find(|&ch| Self::ch_reg(ch, reg) == word_off)
    }

    /// If `word_off` is a timer CONF register, that timer.
    fn timer_of_conf(word_off: u64) -> Option<u64> {
        (0..(2 * TIMER_COUNT)).find(|&t| Self::timer_reg(t, TIMER_CONF) == word_off)
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
    /// timer's source clock, divider and resolution:
    /// `f = src / ((DIV_NUM/256) * 2^DUTY_RES)`. Returns 0 when the timer
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
        // div_num is Q8 (×256). f = src*256 / (div_num * period).
        let denom = div_num as u64 * period;
        (self.timer_src_hz(tmr) * 256) / denom
    }

    /// Source clock of timer `tmr` in Hz (TRM §14.2.2).
    fn timer_src_hz(&self, tmr: u64) -> u64 {
        let conf = self.word(Self::timer_reg(tmr, TIMER_CONF));
        if conf & TIMER_TICK_SEL_BIT == 0 {
            REF_TICK_HZ
        } else if tmr < TIMER_COUNT || self.word(CONF) & CONF_APB_CLK_SEL_BIT != 0 {
            APB_CLK_HZ
        } else {
            RTC8M_HZ
        }
    }

    // ── Waveform engine ──────────────────────────────────────────────────

    /// How timer `tmr` would run if (re)started at `start`, or `None` while
    /// it is stopped.
    fn timer_run(&self, tmr: u64, start: u64) -> Option<TimerRun> {
        let conf = self.word(Self::timer_reg(tmr, TIMER_CONF));
        let duty_res = conf & TIMER_DUTY_RES_MASK;
        let div = u64::from((conf >> TIMER_DIV_NUM_SHIFT) & TIMER_DIV_NUM_MASK);
        if conf & (TIMER_PAUSE_BIT | TIMER_RST_BIT) != 0 || duty_res == 0 || div < 256 {
            return None;
        }
        let (num, den) = (div * self.cpu_hz, 256 * self.timer_src_hz(tmr));
        let g = gcd(num, den);
        Some(TimerRun {
            start,
            num: num / g,
            den: den / g,
            period: 1u64 << duty_res.min(31),
        })
    }

    /// The shape channel `ch`'s registers ask for now.
    fn register_shape(&self, ch: u64) -> Shape {
        Shape {
            hpoint: u64::from(self.word(Self::ch_reg(ch, CH_HPOINT)) & HPOINT_MASK),
            duty: u64::from(self.channel_duty(ch)),
        }
    }

    /// The shape channel `ch` generates in the period containing tick `k`.
    fn shape_for(&self, ch: usize, k: u64, run: &TimerRun) -> Shape {
        match self.wave.pending[ch] {
            Some((shape, from)) if (k / run.period) * run.period >= from => shape,
            _ => self.wave.active[ch],
        }
    }

    /// Level of channel `ch` at CPU cycle `c` (`c >= anchor`).
    fn level_at(&self, ch: usize, c: u64) -> bool {
        let conf0 = self.word(Self::ch_reg(ch as u64, CH_CONF0));
        let idle = conf0 & CONF0_IDLE_LV_BIT != 0;
        if conf0 & CONF0_SIG_OUT_EN_BIT == 0 {
            return idle;
        }
        let Some(run) = self.wave.timers[self.channel_timer(ch as u64) as usize] else {
            return idle;
        };
        if c < run.start {
            return idle;
        }
        let k = run.tick_at(c);
        let shape = self.shape_for(ch, k, &run);
        let pos = k % run.period;
        let end = (shape.hpoint + shape.duty).min(run.period);
        shape.duty > 0 && pos >= shape.hpoint && pos < end
    }

    /// The first cycle after `c` at which any channel may change level.
    fn next_edge_after(&self, c: u64) -> Option<u64> {
        let mut best: Option<u64> = None;
        for ch in 0..NUM_CH {
            let conf0 = self.word(Self::ch_reg(ch as u64, CH_CONF0));
            if conf0 & CONF0_SIG_OUT_EN_BIT == 0 {
                continue;
            }
            let Some(run) = self.wave.timers[self.channel_timer(ch as u64) as usize] else {
                continue;
            };
            // The first tick strictly after `c`.
            let k_next = if c < run.start { 0 } else { run.tick_at(c) + 1 };
            let n = k_next / run.period;
            let mut cand: Option<u64> = None;
            for period in [n, n + 1] {
                let base = period * run.period;
                let shape = self.shape_for(ch, base, &run);
                let fall = (shape.hpoint + shape.duty).min(run.period);
                let toggles = shape.duty > 0 && shape.hpoint < fall;
                let mut ticks = Vec::with_capacity(3);
                if toggles {
                    ticks.push(base + shape.hpoint);
                    ticks.push(base + fall);
                }
                // A pending shape takes over at an overflow, which can move
                // the level by itself (e.g. from a rail to a pulse).
                if self.wave.pending[ch].is_some() {
                    ticks.push(base);
                }
                for t in ticks {
                    if t >= k_next {
                        cand = Some(cand.map_or(t, |b| b.min(t)));
                    }
                }
            }
            if let Some(t) = cand {
                let at = run.cycle_of(t);
                best = Some(best.map_or(at, |b| b.min(at)));
            }
        }
        best
    }

    /// Fold every pending shape whose overflow has passed by cycle `c` into
    /// the active one.
    fn commit_pending(&mut self, c: u64) {
        for ch in 0..NUM_CH {
            let Some((shape, from)) = self.wave.pending[ch] else {
                continue;
            };
            let run = self.wave.timers[self.channel_timer(ch as u64) as usize];
            let due = match run {
                Some(run) => c >= run.start && run.tick_at(c) >= from,
                None => true,
            };
            if due {
                self.wave.active[ch] = shape;
                self.wave.pending[ch] = None;
            }
        }
    }

    fn publish_levels(&self, at: u64) {
        let Some(lines) = &self.lines else {
            return;
        };
        for ch in 0..NUM_CH {
            lines.set_line_at(ch, self.level_at(ch, at), at);
        }
    }

    /// Advance the waveform to cycle `now`, publishing every edge on its own
    /// cycle.
    fn advance_to(&mut self, now: u64) {
        if now <= self.wave.anchor {
            return;
        }
        while let Some(edge) = self.next_edge_after(self.wave.anchor) {
            if edge > now {
                break;
            }
            self.wave.anchor = edge;
            self.commit_pending(edge);
            self.publish_levels(edge);
        }
        self.wave.anchor = now;
        self.commit_pending(now);
    }

    fn now(&self) -> u64 {
        self.clock
            .as_ref()
            .map_or(self.wave.anchor, |c| c.now().max(self.wave.anchor))
    }

    /// Re-derive timer `tmr`'s run after a CONF write at cycle `now`: a timer
    /// whose configuration changed restarts at count 0, one that did not keeps
    /// its phase.
    fn reconfigure_timer(&mut self, tmr: u64, now: u64) {
        let old = self.wave.timers[tmr as usize];
        let fresh = self.timer_run(tmr, now);
        let same = match (old, fresh) {
            (Some(o), Some(f)) => (o.num, o.den, o.period) == (f.num, f.den, f.period),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.wave.timers[tmr as usize] = fresh;
        // A restarted counter has no "next overflow" in the old timeline:
        // every waiting shape on it applies from count 0.
        for ch in 0..NUM_CH {
            if self.channel_timer(ch as u64) == tmr {
                if let Some((shape, _)) = self.wave.pending[ch].take() {
                    self.wave.active[ch] = shape;
                }
            }
        }
    }

    /// A DUTY_START strobe on channel `ch` at cycle `now`: the new shape
    /// waits for the next overflow of a running timer, and applies at once to
    /// a stopped one.
    fn schedule_shape(&mut self, ch: u64, now: u64) {
        let shape = self.register_shape(ch);
        let tmr = self.channel_timer(ch) as usize;
        match self.wave.timers[tmr] {
            Some(run) if now >= run.start => {
                let k = run.tick_at(now);
                let next = (k / run.period + 1) * run.period;
                self.wave.pending[ch as usize] = Some((shape, next));
            }
            _ => {
                self.wave.active[ch as usize] = shape;
                self.wave.pending[ch as usize] = None;
            }
        }
    }

    /// Dispatch the per-word side effects of a register write. Factored so
    /// byte-granular and word-granular writes produce identical state.
    /// Idempotent: each trigger reads live state and writes deterministic
    /// values, so one-per-word or four-per-word calls converge.
    fn apply_write_side_effects(&mut self, word_off: u64, now: u64) {
        if let Some(ch) = Self::channel_of(word_off, CH_CONF1) {
            // DUTY_START strobe latches the staged duty into the live
            // shadow. We do NOT clear DUTY_START (silicon self-clears it
            // after the next timer overflow; firmware treats it as a
            // fire-and-forget command and re-asserts on each update).
            if self.word(word_off) & CONF1_DUTY_START_BIT != 0 {
                self.latch_duty(ch);
                self.schedule_shape(ch, now);
                // Push the freshly-latched duty to any bound actuators — the
                // `ledcWrite` → servo-moves path, with no polling.
                if !self.duty_observers.is_empty() {
                    let frac = self.channel_duty_fraction(ch);
                    for obs in &self.duty_observers {
                        obs.on_duty_change(ch, frac);
                    }
                }
            }
        } else if let Some(ch) = Self::channel_of(word_off, CH_CONF0) {
            // A timer re-bind has no common overflow with the old timer.
            if let Some((shape, _)) = self.wave.pending[ch as usize].take() {
                self.wave.active[ch as usize] = shape;
            }
        } else if let Some(tmr) = Self::timer_of_conf(word_off) {
            self.reconfigure_timer(tmr, now);
        } else if word_off == CONF {
            // The LS slow clock moved: LS timers on SLOW_CLK change rate.
            for tmr in TIMER_COUNT..2 * TIMER_COUNT {
                self.reconfigure_timer(tmr, now);
            }
        } else if word_off == INT_CLR {
            let mask = self.word(INT_CLR);
            let raw = self.word(INT_RAW);
            self.regs.insert(INT_RAW, raw & !mask);
        }
        self.publish_levels(now);
    }

    /// The cycle the next scheduler wake should land on.
    fn wake_target(&self) -> Option<u64> {
        self.next_edge_after(self.wave.anchor)
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// Snapshot layout. `wave` is absent from snapshots taken before the
/// waveform engine existed; those still decode (see `restore`).
#[derive(serde::Serialize, serde::Deserialize)]
struct Snap {
    base: u32,
    regs: Vec<(u64, u32)>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SnapV2 {
    base: u32,
    regs: Vec<(u64, u32)>,
    wave: WaveState,
}

impl Peripheral for Ledc {
    fn add_ledc_duty_observer(&mut self, obs: Arc<dyn LedcDutyObserver>) -> bool {
        self.add_duty_observer(obs);
        true
    }

    // Scheduler-driven: the waveform advances lazily on sync/read and on one
    // scheduled wake per edge, never on the per-cycle walk.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn uses_scheduler(&self) -> bool {
        true
    }

    /// Without the event scheduler there are no edge wakes, so a running
    /// waveform with a pad wire rides the bus tick instead.
    fn needs_bus_tick(&self) -> bool {
        self.lines.is_some()
            && self.wave.timers.iter().any(Option::is_some)
            && self.clock.as_ref().is_some_and(|c| !c.scheduler_enabled())
    }

    fn wire_lines(&self) -> Option<&PadLines> {
        self.lines.as_deref()
    }

    fn tick_with_bus(&mut self, _bus: &mut dyn crate::Bus) {
        let now = self.now();
        self.advance_to(now);
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn attach_cpu_hz(&mut self, hz: u64) {
        if hz != 0 {
            self.cpu_hz = hz;
        }
    }

    fn sync_to(&mut self, now_cycle: u64) {
        self.advance_to(now_cycle);
    }

    /// One live wake, on the next edge. A LATER target needs no new wake (the
    /// live one arrives first and re-arms), the SAME target is already armed,
    /// and only an EARLIER one takes a fresh token.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        let Some(target) = self.wake_target() else {
            return Vec::new();
        };
        if self.armed_target.is_some_and(|live| target >= live) {
            return Vec::new();
        }
        self.arm_seq = self.arm_seq.wrapping_add(1);
        self.armed_target = Some(target);
        vec![(target - self.wave.anchor - 1, self.arm_seq)]
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token != self.arm_seq {
            return crate::sched::EventResult::default();
        }
        let now = sched.now().max(self.wave.anchor);
        self.advance_to(now);
        self.armed_target = self.wake_target();
        crate::sched::EventResult {
            reschedule_delay: self.armed_target.map(|t| t - now),
            ..Default::default()
        }
    }

    /// Side-effect-free probe, so `inspect` can show real values instead of
    /// zeros.
    ///
    /// Delegating to `read` is safe here for one specific reason, and it is
    /// worth stating because it is NOT safe in general: `Peripheral::read`
    /// takes `&self`, so the only way a read could disturb the model is
    /// through interior mutability, and this model has none -- no `Cell`,
    /// `RefCell`, `Atomic*` or `Mutex` anywhere. Every read is a pure function
    /// of state the debugger is allowed to look at.
    ///
    /// Do NOT copy this into a peripheral that does have interior mutability
    /// without checking its read path first. A read-to-clear status register
    /// would be cleared by the act of displaying it, and the firmware under
    /// test would then miss the event -- the exact failure `inspect`'s
    /// peek-only contract exists to prevent.
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let now = self.now();
        self.advance_to(now);
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let entry = self.regs.entry(word_off).or_insert(0);
        *entry &= !(0xFFu32 << byte_off);
        *entry |= (value as u32) << byte_off;
        self.apply_write_side_effects(word_off, now);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let now = self.now();
        self.advance_to(now);
        let word_off = offset & !3;
        self.regs.insert(word_off, value);
        self.apply_write_side_effects(word_off, now);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Never walked (`needs_legacy_walk` is false); time arrives through
        // `sync_to`, the scheduler wake and the bus-tick fallback.
        PeripheralTickResult::default()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        let snap = SnapV2 {
            base: self.base,
            regs: self.regs.iter().map(|(k, v)| (*k, *v)).collect(),
            wave: self.wave.clone(),
        };
        bincode::serialize(&snap).expect("bincode serialize Ledc")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        let (base, regs, wave) = match bincode::deserialize::<SnapV2>(bytes) {
            Ok(s) => (s.base, s.regs, Some(s.wave)),
            Err(_) => {
                let s: Snap = bincode::deserialize(bytes).map_err(|e| {
                    crate::SimulationError::NotImplemented(format!("Ledc snapshot decode: {e}"))
                })?;
                (s.base, s.regs, None)
            }
        };
        self.base = base;
        self.regs = regs.into_iter().collect();
        self.wave = match wave {
            Some(w) => w,
            None => {
                // A pre-waveform snapshot: restart every running timer at the
                // restore point with the committed duties.
                let mut w = WaveState::default();
                for tmr in 0..2 * TIMER_COUNT {
                    w.timers[tmr as usize] = self.timer_run(tmr, 0);
                }
                for ch in 0..NUM_CH {
                    w.active[ch] = self.register_shape(ch as u64);
                }
                w
            }
        };
        self.armed_target = None;
        let anchor = self.wave.anchor;
        self.publish_levels(anchor);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::pad_lines::PadLines;

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
        // TICK_SEL = 1: the HS timer counts APB.
        let conf = duty_res | (div_num << TIMER_DIV_NUM_SHIFT) | TIMER_TICK_SEL_BIT;
        write_u32(&mut p, Ledc::timer_reg(0, TIMER_CONF), conf);
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF0), 0); // TIMER_SEL=0
        assert_eq!(p.channel_freq_hz(0), 5000);
        // TICK_SEL = 0 is REF_TICK (1 MHz), 80x slower: 62 Hz (62.5 floored).
        write_u32(
            &mut p,
            Ledc::timer_reg(0, TIMER_CONF),
            duty_res | (div_num << TIMER_DIV_NUM_SHIFT),
        );
        assert_eq!(p.channel_freq_hz(0), 62);
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

    // ── PWM waveform on the pad wire ─────────────────────────────────────
    //
    // Expected cycles are derived here from the TRM formula, not from the
    // model: counter tick k lands at CPU cycle floor(k * DIV/256 * cpu/src).

    const CPU_HZ: u64 = 240_000_000;
    const TICK_SEL_BIT: u32 = 1 << 25;

    /// `HSTIMERx_CONF` as esp-idf writes it: DUTY_RES [4:0], DIV_NUM [22:5].
    fn timer_conf(duty_res: u32, div_num: u32, tick_sel_apb: bool) -> u32 {
        duty_res | (div_num << 5) | if tick_sel_apb { TICK_SEL_BIT } else { 0 }
    }

    /// One channel driven the way arduino-esp32 drives it: timer, channel
    /// CONF0 (TIMER_SEL + SIG_OUT_EN), staged duty, DUTY_START.
    fn wave(ch: u64, conf: u32, duty: u32) -> (Ledc, Arc<PadLines>) {
        let mut p = Ledc::new(Ledc::BASE);
        p.attach_cpu_hz(CPU_HZ);
        let lines = p.pad_lines_arc();
        let tmr = if ch < CH_COUNT { 0 } else { TIMER_COUNT };
        write_u32(&mut p, Ledc::timer_reg(tmr, TIMER_CONF), conf);
        write_u32(&mut p, Ledc::ch_reg(ch, CH_CONF0), CONF0_SIG_OUT_EN_BIT);
        write_u32(&mut p, Ledc::ch_reg(ch, CH_DUTY), duty << 4);
        write_u32(&mut p, Ledc::ch_reg(ch, CH_CONF1), CONF1_DUTY_START_BIT);
        (p, lines)
    }

    /// Cycles at which line `line` changes level in [from, to), found by
    /// syncing the model one cycle at a time around the expected edges.
    fn level_at(p: &mut Ledc, lines: &PadLines, line: usize, cycle: u64) -> bool {
        p.sync_to(cycle);
        lines.level(line)
    }

    #[test]
    fn arduino_50hz_14bit_ref_tick_servo_pulse_lands_on_its_cycles() {
        // ledcSetup(0, 50, 14) on classic ESP32 picks REF_TICK (1 MHz) with
        // DIV_NUM = 313 (1.22265625) — the value the hosted twin read back
        // from the real firmware. ledcWrite(0, 819).
        let (mut p, lines) = wave(0, timer_conf(14, 313, false), 819);
        let period = 16384u64 * 313 * CPU_HZ / (256 * 1_000_000); // 4_807_680
        let high = 819u64 * 313 * CPU_HZ / (256 * 1_000_000); // 240_325 = 1001.36 us
        assert_eq!(period, 4_807_680); // 20.032 ms: REF_TICK cannot divide 50 Hz exactly
        assert_eq!(high, 240_325);
        // The first latch waits for the first overflow (TRM: a new duty
        // applies from the next period), so frame 0 is low.
        assert!(!level_at(&mut p, &lines, 0, 1));
        for n in 1..4u64 {
            let start = n * period;
            assert!(
                level_at(&mut p, &lines, 0, start + 1),
                "frame {n} starts high"
            );
            assert!(
                level_at(&mut p, &lines, 0, start + high - 1),
                "frame {n} still high"
            );
            assert!(
                !level_at(&mut p, &lines, 0, start + high),
                "frame {n} falls at {high}"
            );
            assert!(
                !level_at(&mut p, &lines, 0, start + period - 1),
                "frame {n} low to the end"
            );
        }
    }

    #[test]
    fn apb_clocked_timer_gives_an_exact_20ms_frame() {
        // TICK_SEL = APB (80 MHz): DIV_NUM = 80e6*256/(50*16384) = 25000.
        let (mut p, lines) = wave(1, timer_conf(14, 25_000, true), 1229);
        let period = 4_800_000u64; // 20.000 ms at 240 MHz
        let high = 1229u64 * 25_000 * 3 / 256; // 360_058 (1500.24 us)
        assert!(level_at(&mut p, &lines, 1, period + high - 1));
        assert!(!level_at(&mut p, &lines, 1, period + high));
        assert!(!level_at(&mut p, &lines, 1, 2 * period - 1));
        assert!(level_at(&mut p, &lines, 1, 2 * period));
    }

    #[test]
    fn edges_carry_their_own_cycle_when_time_jumps() {
        use crate::logic_capture::LogicTap;
        let (mut p, lines) = wave(0, timer_conf(10, 256, true), 256);
        // 1024 APB ticks x 3 CPU cycles = 3072-cycle period, 768 high.
        let tap = LogicTap::new();
        let mut channels = vec![Vec::new(); 16];
        channels[0] = vec![0];
        lines.install_tap(Some(tap.clone()), channels);
        p.sync_to(3 * 3072 + 10); // one jump over three frames
        let edges: Vec<(u64, bool)> = tap
            .take_events()
            .iter()
            .map(|e| (e.cycle, e.value))
            .collect();
        assert_eq!(
            edges,
            vec![
                (3072, true),
                (3840, false),
                (6144, true),
                (6912, false),
                (9216, true)
            ],
        );
    }

    #[test]
    fn a_new_duty_takes_effect_at_the_next_overflow() {
        let (mut p, lines) = wave(0, timer_conf(10, 256, true), 256);
        p.sync_to(3072 + 100); // mid-frame 1, high (3072..3840)
        write_u32(&mut p, Ledc::ch_reg(0, CH_DUTY), 512 << 4);
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF1), CONF1_DUTY_START_BIT);
        // Frame 1 keeps the old duty …
        assert!(level_at(&mut p, &lines, 0, 3072 + 767));
        assert!(!level_at(&mut p, &lines, 0, 3072 + 768));
        // … frame 2 has the new one: 512 ticks = 1536 cycles.
        assert!(level_at(&mut p, &lines, 0, 6144 + 1535));
        assert!(!level_at(&mut p, &lines, 0, 6144 + 1536));
    }

    #[test]
    fn disabled_output_rests_at_idle_level_and_unconfigured_timer_is_flat() {
        let mut p = Ledc::new(Ledc::BASE);
        p.attach_cpu_hz(CPU_HZ);
        let lines = p.pad_lines_arc();
        // IDLE_LV = 1, SIG_OUT_EN = 0, timer running: pad rests high.
        write_u32(
            &mut p,
            Ledc::timer_reg(0, TIMER_CONF),
            timer_conf(10, 256, true),
        );
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF0), 1 << 3);
        write_u32(&mut p, Ledc::ch_reg(0, CH_DUTY), 256 << 4);
        write_u32(&mut p, Ledc::ch_reg(0, CH_CONF1), CONF1_DUTY_START_BIT);
        for c in [0u64, 500, 1000, 5000] {
            assert!(level_at(&mut p, &lines, 0, c), "idle high at {c}");
        }
        // Channel 2 on an unconfigured timer 1 never moves.
        write_u32(&mut p, Ledc::ch_reg(2, CH_CONF0), CONF0_SIG_OUT_EN_BIT | 1);
        write_u32(&mut p, Ledc::ch_reg(2, CH_DUTY), 100 << 4);
        write_u32(&mut p, Ledc::ch_reg(2, CH_CONF1), CONF1_DUTY_START_BIT);
        assert!(!level_at(&mut p, &lines, 2, 10_000));
    }

    #[test]
    fn low_speed_channel_uses_its_own_timer_bank_and_slow_clock() {
        // LS timer 0 with TICK_SEL=1 and LEDC_CONF.APB_CLK_SEL=1 → APB.
        let mut p = Ledc::new(Ledc::BASE);
        p.attach_cpu_hz(CPU_HZ);
        let lines = p.pad_lines_arc();
        write_u32(&mut p, CONF, 1);
        write_u32(
            &mut p,
            Ledc::timer_reg(TIMER_COUNT, TIMER_CONF),
            timer_conf(10, 256, true),
        );
        write_u32(&mut p, Ledc::ch_reg(8, CH_CONF0), CONF0_SIG_OUT_EN_BIT);
        write_u32(&mut p, Ledc::ch_reg(8, CH_DUTY), 256 << 4);
        write_u32(&mut p, Ledc::ch_reg(8, CH_CONF1), CONF1_DUTY_START_BIT);
        assert!(level_at(&mut p, &lines, 8, 3072 + 767));
        assert!(!level_at(&mut p, &lines, 8, 3072 + 768));
        assert!(level_at(&mut p, &lines, 8, 6144));
        // HS channel 0 is untouched.
        assert!(!lines.level(0));
    }

    #[test]
    fn the_scheduler_is_asked_to_wake_on_the_next_edge() {
        let (mut p, _lines) = wave(0, timer_conf(10, 256, true), 256);
        p.sync_to(0);
        let ev = p.take_scheduled_events();
        assert_eq!(ev.len(), 1, "a running channel arms one wake");
        // The first duty applies at the first overflow (3072); the wake lands
        // ON it (delay measured from the synced anchor: anchor + 1 + delay).
        assert_eq!(ev[0].0, 3072 - 1);
    }

    #[test]
    fn snapshot_round_trip_keeps_the_waveform_phase() {
        let (mut p, _lines) = wave(0, timer_conf(10, 256, true), 256);
        p.sync_to(1000);
        let snap = p.runtime_snapshot();
        let mut r = Ledc::new(0);
        r.attach_cpu_hz(CPU_HZ);
        let lines = r.pad_lines_arc();
        r.restore_runtime_snapshot(&snap).unwrap();
        assert!(
            level_at(&mut r, &lines, 0, 3072 + 10),
            "frame 1 rises at 3072"
        );
        assert!(!level_at(&mut r, &lines, 0, 3072 + 768));
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
