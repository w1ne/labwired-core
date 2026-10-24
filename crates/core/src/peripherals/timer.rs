// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::esp_ledc_wave::{LedcDrive, LedcPads, LedcTimebase, WAVE_TOKEN_BIT};
use crate::peripherals::pad_lines::{LineDrive, PadLines};
use crate::{CycleClock, SimResult};
use std::cell::Cell;
use std::sync::Arc;

/// STM32 timer peripheral covering basic, general-purpose, and advanced-
/// control variants:
///
/// - **Basic** (TIM6/TIM7): CR1/DIER/SR/EGR/CNT/PSC/ARR only.
/// - **General-purpose** (TIM2/3/4/5): adds CR2/SMCR/CCMR1/2/CCER + 4
///   capture/compare channels (CCR1..CCR4). TIM2/TIM5 are 32-bit (set
///   `width: 32` in YAML); TIM3/TIM4 are 16-bit.
/// - **Advanced-control** (TIM1/TIM8): general-purpose plus RCR
///   (repetition counter), BDTR (break + dead-time + MOE master output
///   enable), CCMR3, CCR5/CCR6, OR1/OR2. Set `advanced: true` in YAML.
///   The MOE bit in BDTR gates all output channels — without it asserted,
///   PWM outputs stay in their idle state regardless of CCER configuration.
///
/// Counter / ARR width (16 or 32) is selectable via `width: 32` in the
/// chip yaml's `config` block.
///
/// ## Drive modes (walk-free plan Part 2, batches B2/B3)
///
/// Two mutually exclusive time sources, selected by ONE predicate
/// (`scheduler_mode`), following the SysTick (B1) exemplar:
///
/// * **Scheduler mode** (`event-scheduler` feature + a [`CycleClock`] attached
///   at bus registration): `uses_scheduler()` is true, the per-cycle walk skips
///   this peripheral entirely, and
///   - `CNT` / `SR` are **derived lazily** from the bus-published cycle clock
///     with exact closed-form replay of the walk (`advance_to`) — a `&self`
///     read advances `Cell`-held state to "now", so firmware polling `CNT` or
///     `SR` flags observes fresh time without any walk;
///   - update events (UIF) and compare matches (CC1..4IF) that the walk would
///     raise as NVIC interrupts are delivered by **scheduled events**: arming
///     writes hand the bus a `(delay, token)` via `take_scheduled_events`
///     computed in closed form from PSC/ARR/CNT/CCRx/DIER, and `on_event`
///     claims the fire and perpetuates the chain (delay-1 while the legacy
///     walk would hold the IRQ level) — `EventResult::raise_own_irq` pends the
///     peripheral's configured NVIC line through the exact same
///     `pend_irq_for_event` choke the legacy `tick()` path uses.
///
/// * **Legacy mode** (feature off, or no clock attached — hand-built test
///   buses that bypass the bus registration chokes): the per-cycle walk drives
///   `tick()` and the counter advances eagerly, byte-identical to the
///   historical model.
///
/// ### Timing contract
///
/// At tick interval 1 on the batched `Machine::run` path, IRQ pend cycles,
/// `CNT`/`SR` reads, `total_cycles` and register state are **byte-identical**
/// to the walk-driven reference (see
/// `crates/core/tests/stm32_timer_walk_differential.rs` and the in-module
/// `scheduler_matches_walk_*` property gates). At interval N > 1 IRQ pends and
/// lazy reads are quantised to the batch grid — at most one interval
/// late/stale, the same documented bound the write-path `sync_to` ships (and
/// strictly better than the legacy walk at interval N, which under the default
/// `tick_elapsed` slows the count by a factor of N).
///
/// ### Preserved semantics (differentially pinned — including the model's
/// known limitations, kept deliberately so the two drive modes are identical)
///
/// - **IRQ-level counter freeze**: while any enabled status flag is latched
///   (`SR & DIER & 0x1F != 0`) the legacy `tick()` returns the held IRQ level
///   *before* counting, so the counter does not advance until firmware clears
///   the flag. The lazy replay freezes at exactly the same tick.
/// - **Update-tick IRQ vs compare-latch IRQ timing**: an overflow with UIE
///   pends on the overflow tick itself; a compare match latches CCxIF on the
///   match tick and (via the level check) first pends on the **next** tick.
/// - **PSC is applied immediately** (the model has no prescaler buffer;
///   silicon buffers PSC until the next update event). `psc_cnt` keeps its
///   phase across a PSC rewrite, including `psc_cnt > PSC` (next tick
///   increments).
/// - **ARR/CCRx are applied immediately** (no ARPE preload modeling); CR1
///   bits other than CEN (DIR/CMS/OPM/…) are stored but do not affect
///   counting — the model always up-counts, exactly like the walk.
/// - **32-bit `ARR == 0xFFFF_FFFF` never wraps**: the walk's `cnt > arr`
///   check can never fire (u32 wrapping_add), so the counter free-runs
///   mod 2^32 with no UIF — reproduced exactly.
/// - **SMCR (external clock / encoder modes) is a pure register bank bit**:
///   the walk ignores it and always counts the CPU clock, so scheduler mode
///   is exactly as expressive — no configuration needs to stay on the walk.
///
/// ### Tick-cost normalization (B2/B3)
///
/// The legacy model charged `cycles: 1` into the peripheral tick-cost channel
/// on every overflow tick and on every held-IRQ-level tick, inflating
/// `total_cycles` — a sim artifact (real TIMx consumes zero core cycles) that
/// is structurally incompatible with deleting the walk. Both modes now charge
/// zero cost, so the walk-on reference and the scheduler path agree
/// cycle-for-cycle (the same normalization B1 applied to SysTick).
#[derive(Debug, Default, serde::Serialize)]
pub struct Timer {
    cr1: u32,
    cr2: u32,
    smcr: u32,
    dier: u32,
    /// Status flags. `Cell` so the scheduler-mode `&self` read path can
    /// lazily latch UIF/CCxIF up to the bus-published clock. In legacy mode
    /// only `tick()`/`write_reg` mutate it.
    sr: Cell<u32>,
    egr: u32,
    ccmr1: u32,
    ccmr2: u32,
    ccer: u32,
    /// Current counter value. `Cell` for the lazy `&self` advance.
    cnt: Cell<u32>,
    psc: u32,
    arr: u32,
    rcr: u32,
    ccr1: u32,
    ccr2: u32,
    ccr3: u32,
    ccr4: u32,
    bdtr: u32,
    dcr: u32,
    dmar: u32,
    or1: u32,
    ccmr3: u32,
    ccr5: u32,
    ccr6: u32,
    or2: u32,

    /// Counter / ARR width (16 or 32). Defaults to 16 for back-compat
    /// with existing F1-class chip configs.
    width: u8,

    /// Whether this instance has the advanced-control register set
    /// (TIM1/TIM8). Gates the RCR/BDTR/CCMR3/CCR5-6/OR1-2 fields.
    advanced: bool,

    /// Whether this is a **basic** timer (TIM6/TIM7): counter + UIF only,
    /// no capture/compare channels. Suppresses the compare-match flags an
    /// update event would otherwise latch on a general-purpose timer.
    basic: bool,

    // Internal state
    /// Prescaler phase counter. `Cell` for the lazy `&self` advance.
    psc_cnt: Cell<u32>,

    /// Lazy-path anchor: the absolute published cycle the counter state was
    /// last advanced to. Owned exclusively by `advance_to` (scheduler mode);
    /// the legacy walk never touches it.
    #[serde(skip)]
    anchor: Cell<u64>,
    /// Arming-sequence token: bumped on every `take_scheduled_events` so an
    /// in-flight event chain scheduled under an older configuration dies on
    /// arrival (token mismatch) instead of racing the fresh chain.
    #[serde(skip)]
    arm_seq: u32,
    /// Changes only when firmware rewrites timer state that invalidates an
    /// external phase cursor. Natural counter advancement does not bump it.
    #[serde(default)]
    phase_revision: u64,
    #[serde(skip)]
    freeze_revision: Cell<u64>,
    /// Bus-published cycle clock (walk-free plan Part 1). `Some` once the bus
    /// registration choke attaches it; `None` keeps the model on the legacy
    /// walk path.
    #[serde(skip)]
    clock: Option<CycleClock>,

    // ── Input capture (CCxS != 0) ──────────────────────────────────────────
    /// The CCER has the CCxNP bits on a general-purpose timer (F2/F4 and
    /// later). F1 general-purpose timers do not: bit 3 of each nibble is
    /// reserved there and the bench F103 sweep reads it back 0. Chip yaml
    /// `config: { input_capture: stm32f4 }` sets it.
    #[serde(skip)]
    ccer_np: bool,
    /// Per-channel ICxPSC event counter: a capture happens on every
    /// 1st/2nd/4th/8th accepted edge. Reset when CCxE is cleared.
    #[serde(skip)]
    ic_psc_count: [u8; 4],
    /// Filtered level of TI1..TI4 as last accepted (informational: the GPIO
    /// port only ever reports real level changes).
    #[serde(skip)]
    ti_level: [bool; 4],
    /// Channel OUTPUT waveform (OC1..OC4) published onto the GPIO pads the
    /// bus routed to this timer. Inert until the bus asks for the wire
    /// ([`Timer::output_lines_arc`]).
    #[serde(skip)]
    wave: TimerOutputWave,
    /// Raw input edges waiting out their ICxF digital filter, oldest first.
    /// Only populated in scheduler mode with a non-zero filter.
    #[serde(skip)]
    ic_pending: Vec<PendingInputEdge>,
}

/// One raw TIx edge that has not yet been stable for its filter's N samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingInputEdge {
    ti: u8,
    level: bool,
    /// Absolute engine cycle at which the filter accepts the edge.
    accept: u64,
}

/// Line names of a timer's channel-output wire, in channel order.
pub const TIMER_OUTPUT_LINES: &[&str] = &["CH1", "CH2", "CH3", "CH4"];

/// The channel outputs OC1..OC4 as a waveform in engine cycles.
///
/// The waveform engine is the one the ESP LEDC models use
/// ([`LedcPads`]): a channel is either a constant level or a periodic pulse
/// `[hpoint, hpoint + duty)` of a counter that advances one count every
/// `PSC + 1` cycles, published edge-exact onto a [`PadLines`] wire. A duty
/// change on a running channel takes effect at the next counter period
/// boundary, as `CCRx` preload (`OCxPE`, which STM32 HAL/Arduino PWM set)
/// does on silicon at the update event.
#[derive(Debug)]
struct TimerOutputWave {
    pads: LedcPads,
    /// The bus asked for the wire: outputs are tracked from then on.
    wired: bool,
    /// Ticks seen on the legacy walk with no cycle clock attached — the time
    /// base the waveform falls back to on a hand-built bus.
    walk_ticks: u64,
    /// Any channel currently a periodic waveform (cheap walk-path skip).
    running: bool,
}

impl Default for TimerOutputWave {
    fn default() -> Self {
        Self {
            pads: LedcPads::new(TIMER_OUTPUT_LINES),
            wired: false,
            walk_ticks: 0,
            running: false,
        }
    }
}

/// A timer's input-capture stage, as far as pad routing cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerInputStage {
    /// No CCxNP; on an F1 GPIO port the channels are fed by the fixed map.
    F1,
    /// F2/F4 generation (`input_capture: stm32f4`): CCxNP, F4 AF pad map.
    F4,
}

/// SR flag bits the input-capture path sets (RM0368 §13.4.5 / RM0008 §15.4.5).
const SR_TIF: u32 = 1 << 6;
/// CCxOF (over-capture) for channel `ch` (0-based) sits at SR bit 9 + ch.
const SR_CCOF_SHIFT: u32 = 9;

/// Full 32-bit ARR sentinel: the walk's `cnt > arr` overflow check can never
/// fire, so the counter free-runs mod 2^32 with no update events.
const ARR_NEVER_WRAPS: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimerChannelOutputSnapshot {
    pub enabled: bool,
    pub complementary_enabled: bool,
    pub active_low: bool,
    pub complementary_active_low: bool,
    pub duty_fraction: f64,
    pub mode: TimerChannelOutputMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerChannelOutputMode {
    Unsupported,
    Pwm1,
    Pwm2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimerOutputSnapshot {
    pub channels: [TimerChannelOutputSnapshot; 4],
    pub dead_time_ticks: u16,
    pub main_output_enabled: bool,
    pub counter_enabled: bool,
    pub period_ticks: u64,
    /// Authoritative current counter and prescaler phase. Motor service samples
    /// this before the timer advances the interval. The timer model currently
    /// implements edge-aligned up-counting.
    pub counter_ticks: u32,
    pub prescaler_divisor: u64,
    pub prescaler_phase: u32,
    pub phase_revision: u64,
    pub counter_frozen: bool,
    pub freeze_revision: u64,
    /// True when this snapshot was taken after a lazy cycle-clock sync
    /// (`event-scheduler` + attached clock). Motor PWM phase must treat CNT
    /// as already at "now" and must not advance again for the same elapsed.
    pub clock_authoritative: bool,
}

impl Timer {
    pub fn new() -> Self {
        Self::new_with_width(16)
    }

    pub fn new_with_width(width: u8) -> Self {
        Self::new_with_layout(width, false)
    }

    pub fn new_with_layout(width: u8, advanced: bool) -> Self {
        let arr_reset = if width >= 32 { 0xFFFF_FFFF } else { 0xFFFF };
        Self {
            cr1: 0,
            cr2: 0,
            smcr: 0,
            dier: 0,
            sr: Cell::new(0),
            egr: 0,
            ccmr1: 0,
            ccmr2: 0,
            ccer: 0,
            cnt: Cell::new(0),
            psc: 0,
            arr: arr_reset,
            rcr: 0,
            ccr1: 0,
            ccr2: 0,
            ccr3: 0,
            ccr4: 0,
            // BDTR resets to 0 — MOE deasserted, PWM outputs gated until
            // firmware explicitly sets BDTR.MOE bit 15.
            bdtr: 0,
            dcr: 0,
            dmar: 0,
            or1: 0,
            ccmr3: 0,
            ccr5: 0,
            ccr6: 0,
            or2: 0,
            width,
            advanced,
            basic: false,
            psc_cnt: Cell::new(0),
            anchor: Cell::new(0),
            arm_seq: 0,
            phase_revision: 0,
            freeze_revision: Cell::new(0),
            clock: None,
            ccer_np: false,
            ic_psc_count: [0; 4],
            ti_level: [false; 4],
            ic_pending: Vec::new(),
            wave: TimerOutputWave::default(),
        }
    }

    /// Give a general-purpose timer the F2/F4-generation CCER (CCxNP bits
    /// writable, so `CCxP|CCxNP` selects both edges). Builder form, like
    /// [`Self::basic`].
    pub fn ccer_np(mut self, np: bool) -> Self {
        self.ccer_np = np;
        self
    }

    /// Basic timer (TIM6/TIM7): no capture/compare channels.
    pub fn is_basic(&self) -> bool {
        self.basic
    }

    /// Declared F2/F4-generation input stage (`input_capture: stm32f4`):
    /// CCxNP present, and the bus routes this timer's F4 AF pads to it.
    pub fn has_f4_input_capture(&self) -> bool {
        self.ccer_np
    }

    /// Mark this timer as a basic timer (TIM6/TIM7): no capture/compare
    /// channels, so an update event latches only UIF. Builder form so the
    /// existing `new_with_layout` call sites stay unchanged.
    pub fn basic(mut self, basic: bool) -> Self {
        self.basic = basic;
        self
    }

    crate::cycle_clock::scheduler_mode!();

    /// Test/differential knob: detach the cycle clock, pinning the model to
    /// the legacy walk path (`uses_scheduler() == false`). Used by the
    /// walk-on-vs-scheduler differential gates to build the reference config
    /// from the same bus assembly.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// Read-only PWM state derived from the timer's register-owned truth.
    ///
    /// Under `event-scheduler` the counter is advanced lazily from the bus
    /// cycle clock; MMIO reads call `sync_from_clock` but motor service only
    /// uses this snapshot. Sync here so PWM phase tracking sees current CNT
    /// (otherwise workspace `--lib` tests unify features via labwired-wasm and
    /// freeze/unfreeze phase assertions observe a stale counter).
    pub fn output_snapshot(&self) -> TimerOutputSnapshot {
        self.sync_from_clock();
        let period = u64::from(self.arr) + 1;
        let ccr = [self.ccr1, self.ccr2, self.ccr3, self.ccr4];
        let channels = std::array::from_fn(|channel| {
            let shift = channel * 4;
            let (ccmr, lane_shift) = match channel {
                0 => (self.ccmr1, 0),
                1 => (self.ccmr1, 8),
                2 => (self.ccmr2, 0),
                _ => (self.ccmr2, 8),
            };
            let output_mode = if ((ccmr >> lane_shift) & 0x3) != 0 {
                TimerChannelOutputMode::Unsupported
            } else {
                match (ccmr >> (lane_shift + 4)) & 0x7 {
                    0b110 => TimerChannelOutputMode::Pwm1,
                    0b111 => TimerChannelOutputMode::Pwm2,
                    _ => TimerChannelOutputMode::Unsupported,
                }
            };
            TimerChannelOutputSnapshot {
                enabled: (self.ccer & (1 << shift)) != 0,
                active_low: (self.ccer & (1 << (shift + 1))) != 0,
                complementary_enabled: self.advanced && (self.ccer & (1 << (shift + 2))) != 0,
                complementary_active_low: self.advanced && (self.ccer & (1 << (shift + 3))) != 0,
                duty_fraction: (f64::from(ccr[channel]) / period as f64).clamp(0.0, 1.0),
                mode: output_mode,
            }
        });
        TimerOutputSnapshot {
            channels,
            dead_time_ticks: decode_dead_time_ticks((self.bdtr & 0xff) as u8),
            // Only advanced-control timers have the BDTR/MOE latch; a
            // general-purpose timer's main outputs are enabled whenever CEN and
            // CCxE allow. Reporting `false` for TIM2-class timers would gate
            // every Arduino `analogWrite` PWM plant off forever.
            main_output_enabled: !self.advanced || (self.bdtr & (1 << 15)) != 0,
            counter_enabled: (self.cr1 & 1) != 0,
            period_ticks: period,
            counter_ticks: self.cnt.get(),
            prescaler_divisor: u64::from(self.psc) + 1,
            prescaler_phase: self.psc_cnt.get(),
            phase_revision: self.phase_revision,
            counter_frozen: self.counter_frozen(),
            freeze_revision: self.freeze_revision.get(),
            clock_authoritative: self.scheduler_mode(),
        }
    }

    /// The channel-output wire (`CH1`..`CH4`), created on first call — bus
    /// wiring time, when a GPIO pad is routed to it. From then on every
    /// register write re-derives the outputs and every edge is published at
    /// its own cycle.
    pub fn output_lines_arc(&mut self) -> Arc<PadLines> {
        let lines = self.wave.pads.lines_arc();
        self.wave.wired = true;
        let now = self.wave_now();
        self.redrive_outputs(now);
        lines
    }

    /// Live level of output channel `ch` (0-based), `OCx` after polarity, at
    /// the cycle the waveform was last advanced to.
    pub fn output_level(&self, ch: usize) -> bool {
        self.wave.pads.level(ch)
    }

    fn wave_now(&self) -> u64 {
        let anchor = self.wave.pads.anchor();
        match &self.clock {
            Some(clock) => clock.now().max(anchor),
            None => self.wave.walk_ticks.max(anchor),
        }
    }

    fn ccr_value(&self, ch: usize) -> u32 {
        match ch {
            0 => self.ccr1,
            1 => self.ccr2,
            2 => self.ccr3,
            _ => self.ccr4,
        }
    }

    /// Re-derive every channel output from the registers at cycle `now`.
    fn redrive_outputs(&mut self, now: u64) {
        if !self.wave.wired {
            return;
        }
        self.wave.pads.advance_to(now);
        let lines = self.wave.pads.lines_arc();
        let mut running = false;
        for ch in 0..TIMER_OUTPUT_LINES.len() {
            let (drive, drives) = self.channel_output_drive(ch, now);
            running |= matches!(drive, LedcDrive::Pwm { .. });
            self.wave.pads.set_drive(ch, drive, now);
            lines.set_mode(
                ch,
                if drives {
                    LineDrive::PushPull
                } else {
                    LineDrive::Input
                },
            );
        }
        self.wave.running = running;
    }

    /// What channel `ch` (0-based) puts on its `OCx` output, and whether the
    /// timer drives the pad at all.
    ///
    /// RM0008 Rev 21 §15.4.7 (TIMx_CCMR1 `OCxM`) and §15.4.9 (TIMx_CCER) for
    /// TIM2-5, §14.4.7 / §14.4.9 / §14.4.18 (BDTR) for TIM1:
    ///
    /// * `CCxS != 00` — the channel is an input; nothing driven.
    /// * `CCxE = 0` — output off; the timer does not drive the pad.
    /// * TIM1 with `BDTR.MOE = 0` — outputs off, unless `OSSI = 1` holds an
    ///   enabled channel at its idle level `CR2.OISx`.
    /// * `OCxM` = 110 PWM mode 1: active while `CNT < CCRx`; 111 PWM mode 2:
    ///   inactive while `CNT < CCRx`; 011 toggle on `CNT == CCRx`; 100 / 101
    ///   forced inactive / active. `CCxP = 1` inverts (active low).
    /// * 000 frozen and the one-shot 001 / 010 match modes hold the present
    ///   level (the one-shot set/clear on match is not modelled).
    ///
    /// Edge-aligned up-counting only, like the counter model itself; CMS /
    /// DIR, dead time and the complementary `OCxN` outputs are not generated.
    fn channel_output_drive(&self, ch: usize, now: u64) -> (LedcDrive, bool) {
        let held = self.wave.pads.level(ch);
        if self.basic || self.ccs(ch) != 0 {
            return (LedcDrive::Level(held), false);
        }
        let nibble = self.ccer >> (ch * 4);
        let enabled = nibble & 1 != 0;
        let active_low = nibble & 2 != 0;
        if self.advanced && self.bdtr & (1 << 15) == 0 {
            if enabled && self.bdtr & (1 << 10) != 0 {
                let ois = (self.cr2 >> (8 + 2 * ch)) & 1 != 0;
                return (LedcDrive::Level(ois), true);
            }
            return (LedcDrive::Level(held), false);
        }
        if !enabled {
            return (LedcDrive::Level(held), false);
        }
        let mask = self.cnt_mask();
        let period = u64::from(self.arr & mask) + 1;
        let ccr = u64::from(self.ccr_value(ch) & mask);
        // (hpoint, duty, waveform period) of OCxREF, before polarity.
        let (hpoint, duty, wave_period) = match (self.ic_lane(ch) >> 4) & 0x7 {
            0b100 => return (LedcDrive::Level(active_low), true),
            0b101 => return (LedcDrive::Level(!active_low), true),
            0b110 => (0, ccr.min(period), period),
            0b111 => (ccr.min(period), period - ccr.min(period), period),
            0b011 if ccr < period => (ccr, period, 2 * period),
            _ => return (LedcDrive::Level(held), true),
        };
        let (hpoint, duty) = if active_low {
            ((hpoint + duty) % wave_period, wave_period - duty)
        } else {
            (hpoint, duty)
        };
        // Phase: count index 0 is the cycle CNT last read 0 with an empty
        // prescaler, so the waveform lines up with the counter firmware reads.
        let div = u64::from(self.psc) + 1;
        let elapsed =
            (u64::from(self.cnt.get()) % period) * div + u64::from(self.psc_cnt.get()).min(div - 1);
        let timer = LedcTimebase {
            period: wave_period,
            num: u128::from(div),
            den: 1,
            origin: now.saturating_sub(elapsed),
        };
        let mut drive = LedcDrive::Pwm {
            timer,
            hpoint,
            duty,
        };
        if wave_period != period && drive.level_at(now) != held {
            // Toggle mode: the output's present level, not the counter alone,
            // says which half of the double period this is.
            drive = LedcDrive::Pwm {
                timer,
                hpoint: (hpoint + period) % wave_period,
                duty,
            };
        }
        if (self.cr1 & 1) == 0 || self.counter_frozen() {
            // A stopped counter holds its compare result.
            return (LedcDrive::Level(drive.level_at(now)), true);
        }
        (drive, true)
    }

    /// IRQ-level / counter-freeze predicate: the legacy `tick()` returns the
    /// held IRQ level (and skips all counting) while any enabled status flag
    /// among UIF/CC1..4IF is latched.
    #[inline]
    fn irq_level_held(&self) -> bool {
        (self.sr.get() & self.dier & 0x1F) != 0
    }

    /// The counter-freeze predicate. The same conjunction as
    /// [`Self::irq_level_held`] restricted to UIF and the OUTPUT-compare
    /// channels' flags: the freeze is a pinned limitation of the walk replay
    /// (see the type docs), and an input-capture flag never had it — a
    /// channel with CCxS != 0 latched nothing before input capture existed.
    /// Freezing on a capture flag would stop the counter between a capture
    /// and the ISR that reads CCRx, and every measured interval would come
    /// out short by the interrupt latency. With no capture channel this is
    /// exactly `irq_level_held()`.
    #[inline]
    fn counter_frozen(&self) -> bool {
        (self.sr.get() & self.dier & self.freezing_flag_mask()) != 0
    }

    /// UIF plus CCxIF of every channel in output-compare mode.
    #[inline]
    fn freezing_flag_mask(&self) -> u32 {
        let mut mask = 0x1;
        for ch in 0..4u32 {
            if self.ccs(ch as usize) == 0 {
                mask |= 1 << (ch + 1);
            }
        }
        mask
    }

    /// An enabled input-capture flag is latched: the NVIC line is held, but
    /// the counter keeps running (silicon behaviour).
    #[inline]
    fn capture_irq_held(&self) -> bool {
        (self.sr.get() & self.dier & 0x1F & !self.freezing_flag_mask()) != 0
    }

    /// CCxS of channel `ch` (0-based): 00 output, 01/10/11 input.
    #[inline]
    fn ccs(&self, ch: usize) -> u32 {
        (self.ic_lane(ch)) & 0x3
    }

    /// The 8-bit CCMR lane of channel `ch` (0-based). In input mode it is
    /// `ICxF[7:4] ICxPSC[3:2] CCxS[1:0]`.
    #[inline]
    fn ic_lane(&self, ch: usize) -> u32 {
        match ch {
            0 => self.ccmr1 & 0xFF,
            1 => (self.ccmr1 >> 8) & 0xFF,
            2 => self.ccmr2 & 0xFF,
            _ => (self.ccmr2 >> 8) & 0xFF,
        }
    }

    fn ccr_mut(&mut self, ch: usize) -> &mut u32 {
        match ch {
            0 => &mut self.ccr1,
            1 => &mut self.ccr2,
            2 => &mut self.ccr3,
            _ => &mut self.ccr4,
        }
    }

    /// Which timer input (0 = TI1 … 3 = TI4) channel `ch` captures from, per
    /// its CCxS mapping. `None` for an output channel and for TRC (CCxS=11),
    /// which captures on the slave-mode trigger instead.
    fn ic_source(&self, ch: usize) -> Option<u8> {
        match (ch, self.ccs(ch)) {
            (0, 1) | (1, 2) => Some(0),
            (1, 1) | (0, 2) => Some(1),
            (2, 1) | (3, 2) => Some(2),
            (3, 1) | (2, 2) => Some(3),
            _ => None,
        }
    }

    /// Does an edge to `level` pass channel `ch`'s polarity selector?
    /// CCxNP:CCxP = 00 rising, 01 falling, 11 both edges; 10 is reserved and
    /// treated as rising. On a timer without CCxNP only rising/falling exist.
    fn polarity_accepts(&self, ch: usize, level: bool) -> bool {
        let shift = ch * 4;
        let p = (self.ccer >> (shift + 1)) & 1;
        let np = (self.ccer >> (shift + 3)) & 1;
        match (np, p) {
            (1, 1) => true,
            (_, 1) => !level,
            _ => level,
        }
    }

    /// ICxF digital-filter length in timer-kernel cycles for input `ti`
    /// (RM0368 §13.4.7, TIMx_CCMR1 IC1F): N consecutive samples at
    /// f_SAMPLING must agree before the filtered input moves. The filter of
    /// TIn is the one in channel n's own lane (TI1 → IC1F …), and only when
    /// that channel is in input mode — in output mode those bits are OCxM.
    /// fDTS = fCK_INT / (1, 2, 4) from CR1.CKD. The model's kernel clock is
    /// the engine cycle, so the answer is in engine cycles. The sample-phase
    /// jitter of real silicon (up to one sampling period) is not modelled;
    /// the capture lands N·period after the raw edge.
    fn filter_delay_cycles(&self, ti: u8) -> u64 {
        let ch = ti as usize;
        if self.ccs(ch) == 0 {
            return 0;
        }
        let f = (self.ic_lane(ch) >> 4) & 0xF;
        let dts: u64 = match (self.cr1 >> 8) & 0x3 {
            1 => 2,
            2 => 4,
            _ => 1,
        };
        let (div, n): (u64, u64) = match f {
            0 => (0, 0),
            1 => (1, 2),
            2 => (1, 4),
            3 => (1, 8),
            4 => (dts * 2, 6),
            5 => (dts * 2, 8),
            6 => (dts * 4, 6),
            7 => (dts * 4, 8),
            8 => (dts * 8, 6),
            9 => (dts * 8, 8),
            10 => (dts * 16, 5),
            11 => (dts * 16, 6),
            12 => (dts * 16, 8),
            13 => (dts * 32, 5),
            14 => (dts * 32, 6),
            _ => (dts * 32, 8),
        };
        div * n
    }

    /// Latch CNT into CCRx for input channel `ch` if CCxE is set and the
    /// ICxPSC divider lets this event through. Sets CCxIF, and CCxOF when
    /// CCxIF was still set from the previous capture (RM0368 §13.3.5).
    fn capture_channel(&mut self, ch: usize) {
        if (self.ccer >> (ch * 4)) & 1 == 0 {
            return;
        }
        let div = 1u8 << ((self.ic_lane(ch) >> 2) & 0x3);
        self.ic_psc_count[ch] = self.ic_psc_count[ch].saturating_add(1);
        if self.ic_psc_count[ch] < div {
            return;
        }
        self.ic_psc_count[ch] = 0;
        let flag = 1u32 << (ch + 1);
        let mut sr = self.sr.get();
        if sr & flag != 0 {
            sr |= 1 << (SR_CCOF_SHIFT + ch as u32);
        }
        sr |= flag;
        self.sr.set(sr);
        let cnt = self.cnt.get() & self.cnt_mask();
        *self.ccr_mut(ch) = cnt;
    }

    /// A FILTERED edge on input `ti` at absolute cycle `at`: capture on every
    /// channel mapped to it whose polarity accepts the edge, then run the
    /// slave-mode controller's trigger (TS) and reset mode (SMS=100).
    ///
    /// Ordering matches silicon PWM-input mode (RM0368 §13.3.6): the rising
    /// edge captures the period into CCR1 and THEN resets the counter.
    fn apply_filtered_edge(&mut self, ti: u8, level: bool, at: u64) {
        if self.scheduler_mode() {
            self.advance_to(at);
        }
        let frozen_before = self.counter_frozen();
        self.ti_level[ti as usize] = level;
        for ch in 0..4 {
            if self.ic_source(ch) == Some(ti) && self.polarity_accepts(ch, level) {
                self.capture_channel(ch);
            }
        }
        // Slave-mode trigger input TRGI (SMCR.TS): 100 TI1F_ED (both edges of
        // TI1), 101 TI1FP1 (TI1 through CC1P/CC1NP), 110 TI2FP2 (TI2 through
        // CC2P/CC2NP). The internal ITRx and ETR triggers are not modelled.
        let ts = (self.smcr >> 4) & 0x7;
        let triggered = match ts {
            4 => ti == 0,
            5 => ti == 0 && self.polarity_accepts(0, level),
            6 => ti == 1 && self.polarity_accepts(1, level),
            _ => false,
        };
        if triggered {
            // CCxS=11 (TRC): channels 1/2 capture on the trigger itself.
            for ch in 0..2 {
                if self.ccs(ch) == 3 {
                    self.capture_channel(ch);
                }
            }
            self.sr.set(self.sr.get() | SR_TIF);
            // SMS=100 reset mode: reinitialise the counter and generate an
            // update event — UIF unless CR1.URS restricts it to overflow.
            // Gated (101), trigger (110) and external-clock (111) modes are
            // not modelled.
            if self.smcr & 0x7 == 0b100 {
                self.cnt.set(0);
                self.psc_cnt.set(0);
                if (self.cr1 >> 2) & 1 == 0 {
                    self.sr.set(self.sr.get() | 1);
                }
                self.latch_compare_match_flags();
                self.phase_revision = self.phase_revision.wrapping_add(1);
            }
        }
        if frozen_before != self.counter_frozen() {
            self.freeze_revision
                .set(self.freeze_revision.get().wrapping_add(1));
        }
    }

    /// Apply every filtered edge whose acceptance cycle is at or before `now`.
    fn process_pending_edges(&mut self, now: u64) {
        while let Some(first) = self.ic_pending.first().copied() {
            if first.accept > now {
                break;
            }
            self.ic_pending.remove(0);
            self.apply_filtered_edge(first.ti, first.level, first.accept);
        }
    }

    /// A raw level change on timer input `ti` (0 = TI1) at absolute engine
    /// cycle `at`, from the pad the GPIO mux routes to TIMx_CHn.
    ///
    /// With ICxF = 0 the edge is applied at `at` exactly: in scheduler mode
    /// the lazy counter is replayed to that cycle first, so CCRx holds CNT as
    /// of the edge, not as of whenever firmware next looks. A non-zero filter
    /// defers the edge by the filter length and drops a pulse shorter than
    /// it (both of its edges), which is what N matching samples mean. The
    /// legacy walk (no cycle clock — hand-built test buses only) cannot defer
    /// and applies every edge immediately.
    pub fn input_edge(&mut self, ti: u8, level: bool, at: u64) {
        if self.basic || ti >= 4 {
            return;
        }
        if !self.scheduler_mode() {
            self.apply_filtered_edge(ti, level, at);
            return;
        }
        self.process_pending_edges(at);
        if let Some(pos) = self.ic_pending.iter().position(|p| p.ti == ti) {
            // The opposite edge of a still-unaccepted one: the pulse is
            // shorter than the filter, and the filtered input never moved.
            self.ic_pending.remove(pos);
            return;
        }
        let delay = self.filter_delay_cycles(ti);
        if delay == 0 {
            self.apply_filtered_edge(ti, level, at);
        } else {
            self.ic_pending.push(PendingInputEdge {
                ti,
                level,
                accept: at + delay,
            });
            self.ic_pending.sort_by_key(|p| p.accept);
        }
    }

    /// Latch CCxIF for every output-compare channel whose CCRx currently
    /// equals CNT. Called at an update (UG) event, which reloads CNT and so
    /// re-evaluates the compare for all four channels. At reset (CCRx=0,
    /// CCMR in output-compare mode) CNT=0 matches every channel, so SR reads
    /// 0x1F after a bare UG — silicon-verified on STM32F103 TIM2. Channels in
    /// input-capture mode (CCxS != 0) don't compare and are skipped.
    fn latch_compare_match_flags(&mut self) {
        let mask = self.cnt_mask();
        let cnt = self.cnt.get();
        let channels = [
            (self.ccr1, self.ccmr1 & 0x3, 1u32),
            (self.ccr2, (self.ccmr1 >> 8) & 0x3, 2),
            (self.ccr3, self.ccmr2 & 0x3, 3),
            (self.ccr4, (self.ccmr2 >> 8) & 0x3, 4),
        ];
        for (ccr, ccs, bit) in channels {
            if ccs == 0 && (ccr & mask) == cnt {
                self.sr.set(self.sr.get() | (1 << bit));
            }
        }
        // Advanced timers carry the output-only internal channels 5/6 whose
        // compare flags live at SR bits 16/17. Silicon-verified on STM32H563
        // TIM1 (2026-06-11): a bare UG with CCR5/CCR6 at reset reads SR with
        // CC5IF|CC6IF set on top of the channel-1..4 latch.
        if self.advanced {
            for (ccr, bit) in [(self.ccr5, 16u32), (self.ccr6, 17)] {
                if (ccr & mask) == cnt {
                    self.sr.set(self.sr.get() | (1 << bit));
                }
            }
        }
    }

    fn cnt_mask(&self) -> u32 {
        if self.width >= 32 {
            0xFFFF_FFFF
        } else {
            0xFFFF
        }
    }

    // ── Closed-form walk replay (scheduler mode) ────────────────────────────
    //
    // The legacy walk applies, per tick:
    //   if SR & DIER & 0x1F != 0 → held IRQ level, NO counting (freeze);
    //   else if !CEN → nothing;
    //   else psc_cnt += 1; if psc_cnt > PSC { psc_cnt = 0; cnt += 1;
    //        if cnt > ARR { cnt = 0; UIF; latch compares } else { latch } }
    //
    // Counter *increments* therefore happen on a fixed tick grid derived from
    // PSC (`k1` ticks to the first, then every `PSC+1`), and the value visited
    // at increment j is a pure function of (CNT, ARR, j). All flag latches and
    // IRQ fires are attached to increments, so a window of `e` ticks replays
    // exactly in O(#channels).

    /// Walk ticks until the first counter increment: `psc_cnt` starts at `c`,
    /// increments each tick, and the counter bumps on the tick where it
    /// exceeds PSC (then resets to 0). A stale `c > PSC` (PSC shrunk mid-run,
    /// applied immediately — no buffering, like the walk) bumps on the next
    /// tick.
    #[inline]
    fn ticks_to_first_increment(&self) -> u64 {
        (self.psc as u64).saturating_sub(self.psc_cnt.get() as u64) + 1
    }

    /// Number of increments until the counter first *visits* value `w`
    /// (post-increment compare, the walk's `latch_compare_match_flags` site),
    /// or `None` if unreachable. Value sequence from `v`: `v+1, …, ARR, 0(U),
    /// 1, …` (a stale `v > ARR` — CNT written above ARR — wraps to 0 on the
    /// first increment, exactly like the walk's `cnt > arr` check).
    fn increments_to_value(&self, v: u32, w: u32) -> Option<u64> {
        let arr = self.arr;
        if arr == ARR_NEVER_WRAPS {
            // Free-running mod 2^32 (32-bit reset ARR): every value is
            // visited; a match on the current value needs a full lap.
            let d = w.wrapping_sub(v);
            return Some(if d == 0 { 1u64 << 32 } else { d as u64 });
        }
        if w > arr {
            return None; // never visited (walk resets past ARR before latch)
        }
        if v > arr {
            // First increment wraps to 0, then counts 1, 2, …
            Some(1 + w as u64)
        } else if w > v {
            Some((w - v) as u64)
        } else {
            // Wrap first (ARR - v + 1 increments), then count up to w.
            // u64 math: arr - v + 1 + w can exceed u32 on 32-bit timers.
            Some(arr as u64 - v as u64 + 1 + w as u64)
        }
    }

    /// Number of increments until the first update event (wrap to 0 with
    /// UIF), or `None` if ARR never wraps (32-bit 0xFFFF_FFFF).
    fn increments_to_wrap(&self, v: u32) -> Option<u64> {
        let arr = self.arr;
        if arr == ARR_NEVER_WRAPS {
            return None;
        }
        if v > arr {
            Some(1)
        } else {
            Some((arr - v + 1) as u64)
        }
    }

    /// Counter value after `m >= 1` increments from `v` (no freeze within).
    fn value_after_increments(&self, v: u32, m: u64) -> u32 {
        let arr = self.arr;
        if arr == ARR_NEVER_WRAPS {
            return v.wrapping_add((m & 0xFFFF_FFFF) as u32);
        }
        let period = arr as u64 + 1;
        if v > arr {
            // First increment wraps to 0.
            ((m - 1) % period) as u32
        } else if m <= (arr - v) as u64 {
            v + m as u32
        } else {
            ((m - (arr - v) as u64 - 1) % period) as u32
        }
    }

    /// The increment index of the first latch of an *enabled* flag. The walk
    /// freezes counting from that increment on, and the bus pends the NVIC
    /// line on the same tick (`irq_line_level` — a level tracks its flag), so
    /// this one number serves the freeze and the event chain alike.
    fn first_enabled_event(&self) -> Option<u64> {
        if (self.cr1 & 0x1) == 0 {
            return None;
        }
        let v = self.cnt.get();
        let mut best: Option<u64> = None;
        if (self.dier & 0x1) != 0 {
            best = self.increments_to_wrap(v);
        }
        if !self.basic {
            let mask = self.cnt_mask();
            let channels = [
                (self.ccr1, self.ccmr1 & 0x3, 1u32),
                (self.ccr2, (self.ccmr1 >> 8) & 0x3, 2),
                (self.ccr3, self.ccmr2 & 0x3, 3),
                (self.ccr4, (self.ccmr2 >> 8) & 0x3, 4),
            ];
            for (ccr, ccs, bit) in channels {
                if ccs != 0 || (self.dier >> bit) & 1 == 0 {
                    continue;
                }
                if let Some(j) = self.increments_to_value(v, ccr & mask) {
                    // Strict `<` keeps update-event precedence on a tie (the
                    // overflow tick pends itself when UIE is set).
                    if best.is_none_or(|b| j < b) {
                        best = Some(j);
                    }
                }
            }
        }
        best
    }

    /// Lazy advance to absolute published cycle `now` — callable from `&self`
    /// (all mutated state is in `Cell`). Idempotent; a `now` older than the
    /// anchor is ignored. The advanced window always has constant
    /// CR1/DIER/PSC/ARR/CCRx: every MMIO write syncs first (bus `sync_to`
    /// choke), so settings changes never straddle a window. Replays the walk
    /// EXACTLY, including the enabled-flag counter freeze.
    fn advance_to(&self, now: u64) {
        let frozen_before = self.counter_frozen();
        let anchor = self.anchor.get();
        if now <= anchor {
            return;
        }
        self.anchor.set(now);
        if self.counter_frozen() || (self.cr1 & 0x1) == 0 {
            // Frozen (held IRQ level) or not enabled: the window elapses with
            // no counting — exactly the walk's early returns.
            return;
        }
        let e = now - anchor;
        let k1 = self.ticks_to_first_increment();
        if e < k1 {
            // No increment in the window: only the prescaler phase advances.
            self.psc_cnt.set(self.psc_cnt.get() + e as u32);
            return;
        }
        let period = self.psc as u64 + 1;
        let n = 1 + (e - k1) / period; // increments the un-frozen walk would do
        let freeze_j = self.first_enabled_event();
        // Increments actually applied: the walk stops counting after the
        // increment that latches an enabled flag.
        let m = match freeze_j {
            Some(j) if j <= n => j,
            _ => n,
        };
        let v = self.cnt.get();
        // Latch every flag whose first occurrence lies within the applied
        // window — disabled flags accumulate lazily without freezing, exactly
        // like the walk.
        let mut sr = self.sr.get();
        if let Some(j) = self.increments_to_wrap(v) {
            if j <= m {
                sr |= 1; // UIF
            }
        }
        if !self.basic {
            let mask = self.cnt_mask();
            let channels = [
                (self.ccr1, self.ccmr1 & 0x3, 1u32),
                (self.ccr2, (self.ccmr1 >> 8) & 0x3, 2),
                (self.ccr3, self.ccmr2 & 0x3, 3),
                (self.ccr4, (self.ccmr2 >> 8) & 0x3, 4),
            ];
            for (ccr, ccs, bit) in channels {
                if ccs != 0 {
                    continue;
                }
                if let Some(j) = self.increments_to_value(v, ccr & mask) {
                    if j <= m {
                        sr |= 1 << bit;
                    }
                }
            }
            if self.advanced {
                let mask = self.cnt_mask();
                for (ccr, bit) in [(self.ccr5, 16u32), (self.ccr6, 17)] {
                    if let Some(j) = self.increments_to_value(v, ccr & mask) {
                        if j <= m {
                            sr |= 1 << bit;
                        }
                    }
                }
            }
        }
        self.sr.set(sr);
        if frozen_before != self.counter_frozen() {
            self.freeze_revision
                .set(self.freeze_revision.get().wrapping_add(1));
        }
        self.cnt.set(self.value_after_increments(v, m));
        // Prescaler phase: an increment tick resets it to 0; if the walk
        // froze at increment `m` it stays 0 for the rest of the window,
        // otherwise the post-increment remainder ticks accumulate.
        let frozen = matches!(freeze_j, Some(j) if j <= n);
        self.psc_cnt.set(if frozen {
            0
        } else {
            ((e - k1) % period) as u32
        });
    }

    /// Pull "now" from the bus-published clock and advance. No-op without an
    /// attached clock (legacy mode — the walk advances the counter instead).
    fn sync_from_clock(&self) {
        if let Some(clock) = &self.clock {
            if self.scheduler_mode() {
                self.advance_to(clock.now());
            }
        }
    }

    /// Walk ticks from the just-synced state until the tick on which the
    /// legacy walk FIRST pends the NVIC line, for the event chain: `None` when
    /// nothing is armed (chain dies; the next relevant MMIO write re-arms).
    /// The tick is `k1 + (j-1)*(PSC+1)` for the increment that latches the
    /// enabled flag — update and compare alike.
    ///
    /// A LEVEL pends on the tick its flag latches (the bus reconciles the line
    /// from `irq_line_level()` after each walk tick, so a compare match needs
    /// no extra tick). This used to add +1 for compare matches, encoding the
    /// pre-level-reconcile walk, where only the NEXT tick's `tick()` returned
    /// `irq: true`; that made the scheduler fire one cycle after the walk and
    /// the `stm32_timer_walk_differential` compare gate went red.
    fn ticks_until_first_pend(&self) -> Option<u64> {
        if self.irq_level_held() {
            // Already held: the walk pends on the very next tick.
            return Some(1);
        }
        // A filtered input edge still waiting to be accepted needs a wake at
        // its acceptance cycle, whatever it then latches.
        let anchor = self.anchor.get();
        let filter = self
            .ic_pending
            .first()
            .map(|p| p.accept.saturating_sub(anchor).max(1));
        let counter = self
            .first_enabled_event()
            .map(|j| self.ticks_to_first_increment() + (j - 1) * (self.psc as u64 + 1));
        match (counter, filter) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.smcr,
            0x0C => self.dier,
            0x10 => self.sr.get(),
            0x14 => self.egr,
            0x18 => self.ccmr1,
            0x1C => self.ccmr2,
            0x20 => self.ccer,
            0x24 => self.cnt.get(),
            0x28 => self.psc,
            0x2C => self.arr,
            0x30 if self.advanced => self.rcr,
            0x34 => self.ccr1,
            0x38 => self.ccr2,
            0x3C => self.ccr3,
            0x40 => self.ccr4,
            0x44 if self.advanced => self.bdtr,
            0x48 => self.dcr,
            0x4C => self.dmar,
            0x50 if self.advanced => self.or1,
            0x54 if self.advanced => self.ccmr3,
            0x58 if self.advanced => self.ccr5,
            0x5C if self.advanced => self.ccr6,
            0x60 if self.advanced => self.or2,
            _ => {
                crate::census_reg!("timer:Timer", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        let frozen_before = self.counter_frozen();
        let phase_mapping_before = (
            self.cr1 & 1,
            self.ccmr1,
            self.ccmr2,
            self.cnt.get(),
            self.psc,
            self.arr,
        );
        let explicit_update = offset == 0x14 && (value & 1) != 0;
        match offset {
            0x00 => self.cr1 = value & 0x3FF,
            // CR2 writable bits differ by layout. General-purpose (TIM2-5):
            // CCDS(3)+MMS(6:4)+TI1S(7) = 0xF8, silicon-confirmed on F103 TIM2.
            // Advanced (TIM1) adds CCPC/CCUS/OISx — left at the wider mask
            // pending a TIM1 sweep to pin it.
            0x04 => {
                let mask = if self.advanced {
                    0x00FF_FFFB
                } else {
                    0x0000_00F8
                };
                self.cr2 = value & mask;
            }
            // SMCR is 16-bit on every layout (bit 3 reserved): 0xFFF7,
            // silicon-confirmed on F103 TIM2.
            0x08 => self.smcr = value & 0x0000_FFF7,
            // DIER: general-purpose has no COMIE(5)/BIE(7)/COMDE(13) — 0x5F5F,
            // silicon-confirmed on F103 TIM2. Advanced (TIM1) exposes them (0x7FFF).
            0x0C => {
                let mask = if self.advanced { 0x7FFF } else { 0x5F5F };
                self.dier = value & mask;
            }
            // TIMx_SR is rc_w0 for status flags: writing 0 clears, writing 1 keeps current.
            0x10 => self.sr.set(self.sr.get() & (value & 0x1FFFF)),
            // TIMx_EGR: only UG (bit 0) drives state — but advanced timers
            // also have CC1G-CC4G, COMG, TG, BG. We accept all and treat
            // CC*G as also setting the corresponding CC*IF flags.
            0x14 => {
                self.egr = value & 0xFF;
                if (self.egr & 0x01) != 0 {
                    // Update event: reload counter/prescaler and set UIF.
                    self.cnt.set(0);
                    self.psc_cnt.set(0);
                    self.sr.set(self.sr.get() | 1);
                    // The UG reload re-evaluates every output-compare channel:
                    // on a general-purpose/advanced timer the channels whose
                    // CCRx now equals CNT latch their CCxIF (at reset, all of
                    // them — SR=0x1F, silicon-verified on F103 TIM2). Basic
                    // timers have no compare channels, so UG sets UIF only.
                    if !self.basic {
                        self.latch_compare_match_flags();
                    }
                }
                // CCxG on a channel in INPUT mode is a software capture on
                // every timer that has channels (RM0368 §13.4.6): CNT into
                // CCRx, CCxIF, and CCxOF if CCxIF was still set.
                if !self.basic {
                    for ch in 0..4 {
                        if self.ccs(ch) != 0 && (self.egr >> (ch + 1)) & 1 != 0 {
                            self.sw_capture(ch);
                        }
                    }
                }
                if self.advanced {
                    if (self.egr & 0x02) != 0 {
                        self.sr.set(self.sr.get() | (1 << 1));
                    } // CC1IF
                    if (self.egr & 0x04) != 0 {
                        self.sr.set(self.sr.get() | (1 << 2));
                    }
                    if (self.egr & 0x08) != 0 {
                        self.sr.set(self.sr.get() | (1 << 3));
                    }
                    if (self.egr & 0x10) != 0 {
                        self.sr.set(self.sr.get() | (1 << 4));
                    }
                    if (self.egr & 0x80) != 0 {
                        self.sr.set(self.sr.get() | (1 << 7));
                    } // BIF (break)
                }
            }
            // CCMR1/2 are 16-bit on every layout: 0xFFFF, silicon-confirmed on
            // F103 TIM2 (the model previously stored the full 32 bits).
            // CCxS is write-protected while CCxE is set (bench F103 sweep:
            // "set CCxE first and CCMRx bits 0,1,8,9 stop latching").
            0x18 => {
                let keep = self.ccxs_locked_mask(0, 1);
                self.ccmr1 = ((value & 0xFFFF) & !keep) | (self.ccmr1 & keep);
            }
            0x1C => {
                let keep = self.ccxs_locked_mask(2, 3);
                self.ccmr2 = ((value & 0xFFFF) & !keep) | (self.ccmr2 & keep);
            }
            0x20 => {
                // CCER mask: F1 general-purpose timers expose CCxE (bit 0) +
                // CCxP (bit 1) per channel — 4 channels = 0x3333. F2/F4 and
                // later general-purpose timers add CCxNP (bit 3), which with
                // CCxP selects both-edge capture — 0xBBBB. Advanced timers
                // add CCxNE (bit 2) + CCxNP (bit 3) — 4 channels = 0xFFFF.
                let mask = if self.advanced {
                    0xFFFF
                } else if self.ccer_np {
                    0xBBBB
                } else {
                    0x3333
                };
                let old = self.ccer;
                self.ccer = value & mask;
                // ICxPSC's event counter restarts when capture is disabled.
                for ch in 0..4 {
                    if (old >> (ch * 4)) & 1 != 0 && (self.ccer >> (ch * 4)) & 1 == 0 {
                        self.ic_psc_count[ch] = 0;
                    }
                }
            }
            0x24 => self.cnt.set(value & self.cnt_mask()),
            0x28 => self.psc = value & 0xFFFF,
            0x2C => self.arr = value & self.cnt_mask(),
            0x30 if self.advanced => self.rcr = value & 0xFFFF,
            // In input-capture mode CCRx is read-only: it holds the last
            // capture (RM0368 §13.4.13; bench F103 sweep probes CCRx before
            // CCMRx for exactly this reason).
            0x34 | 0x38 | 0x3C | 0x40 => {
                let ch = ((offset - 0x34) / 4) as usize;
                if self.ccs(ch) == 0 {
                    let v = value & self.cnt_mask();
                    *self.ccr_mut(ch) = v;
                }
            }
            // BDTR: full register, including MOE (bit 15) which gates PWM
            // outputs. Real silicon has lock-protection for some bits via
            // LOCK[1:0]; we accept all writes for survival-mode firmware.
            0x44 if self.advanced => self.bdtr = value & 0x03FF_FFFF,
            0x48 => self.dcr = value & 0x1F1F,
            0x4C => self.dmar = value,
            0x50 if self.advanced => self.or1 = value,
            0x54 if self.advanced => self.ccmr3 = value,
            0x58 if self.advanced => self.ccr5 = value,
            0x5C if self.advanced => self.ccr6 = value,
            0x60 if self.advanced => self.or2 = value,
            _ => {
                crate::census_reg!("timer:Timer", offset, "write");
            }
        }
        let phase_mapping_after = (
            self.cr1 & 1,
            self.ccmr1,
            self.ccmr2,
            self.cnt.get(),
            self.psc,
            self.arr,
        );
        if explicit_update || phase_mapping_before != phase_mapping_after {
            self.phase_revision = self.phase_revision.wrapping_add(1);
        }
        if frozen_before != self.counter_frozen() {
            self.freeze_revision
                .set(self.freeze_revision.get().wrapping_add(1));
        }
    }
}

impl Timer {
    /// CCMR bits that must keep their value because the channel's CCxE is
    /// set: CCxS of channel `lo` (bits 1:0) and `hi` (bits 9:8) of one CCMR.
    fn ccxs_locked_mask(&self, lo: usize, hi: usize) -> u32 {
        let mut keep = 0;
        if (self.ccer >> (lo * 4)) & 1 != 0 {
            keep |= 0x3;
        }
        if (self.ccer >> (hi * 4)) & 1 != 0 {
            keep |= 0x3 << 8;
        }
        keep
    }

    /// EGR.CCxG on an input channel: capture now, ignoring ICxPSC.
    fn sw_capture(&mut self, ch: usize) {
        let flag = 1u32 << (ch + 1);
        let mut sr = self.sr.get();
        if sr & flag != 0 {
            sr |= 1 << (SR_CCOF_SHIFT + ch as u32);
        }
        self.sr.set(sr | flag);
        let cnt = self.cnt.get() & self.cnt_mask();
        *self.ccr_mut(ch) = cnt;
    }
}

fn decode_dead_time_ticks(dtg: u8) -> u16 {
    match dtg {
        0x00..=0x7f => u16::from(dtg),
        0x80..=0xbf => (64 + u16::from(dtg & 0x3f)) * 2,
        0xc0..=0xdf => (32 + u16::from(dtg & 0x1f)) * 8,
        _ => (32 + u16::from(dtg & 0x1f)) * 16,
    }
}

impl Timer {
    /// The legacy per-tick counter walk (the whole of `tick()` before the
    /// channel-output waveform existed).
    fn tick_counter(&mut self) -> crate::PeripheralTickResult {
        let frozen_before = self.counter_frozen();
        // Never runs in scheduler mode (the walk skips `uses_scheduler()`
        // peripherals; the guard keeps a stray direct call from corrupting
        // the lazily-anchored state).
        if self.scheduler_mode() {
            return crate::PeripheralTickResult::default();
        }
        // Keep the IRQ level high while any enabled status flag is latched:
        // UIF/UIE (bit 0) and the compare-match pairs CC1..4IF/CC1..4IE
        // (bits 1..4). Compare interrupts drive alarm-style time drivers
        // (CCR written ahead of CNT, CCxIE set, wake on match) — exercised
        // by foreign STM32H563 firmware and silicon-verified on the bench
        // TIM2 (2026-06-11): CC1IF pends the NVIC line with the CPU halted.
        if self.counter_frozen() {
            return crate::PeripheralTickResult {
                irq: true,
                cycles: 0,
                ..Default::default()
            };
        }

        // Counter Enable (bit 0)
        if (self.cr1 & 0x1) == 0 {
            return crate::PeripheralTickResult {
                irq: self.capture_irq_held(),
                cycles: 0,
                ..Default::default()
            };
        }

        self.psc_cnt.set(self.psc_cnt.get().wrapping_add(1));
        if self.psc_cnt.get() > self.psc {
            self.psc_cnt.set(0);
            self.cnt.set(self.cnt.get().wrapping_add(1));

            if self.cnt.get() > self.arr {
                self.cnt.set(0);
                self.sr.set(self.sr.get() | 1); // Set UIF (Update Interrupt Flag)
                if !self.basic {
                    self.latch_compare_match_flags();
                }

                // Return true if Update Interrupt Enable (UIE) is set
                if frozen_before != self.counter_frozen() {
                    self.freeze_revision
                        .set(self.freeze_revision.get().wrapping_add(1));
                }
                return crate::PeripheralTickResult {
                    irq: (self.dier & 1) != 0 || self.capture_irq_held(),
                    cycles: 0,
                    dma_signals: None,
                    ..Default::default()
                };
            }
            // Output-compare match while counting: CCxIF latches the moment
            // CNT reaches CCRx. Silicon-verified on STM32H563 TIM1
            // (2026-06-11): CC1IF rises once the running CNT crosses CCR1 in
            // PWM mode 1.
            if !self.basic {
                self.latch_compare_match_flags();
            }
        }

        if frozen_before != self.counter_frozen() {
            self.freeze_revision
                .set(self.freeze_revision.get().wrapping_add(1));
        }
        crate::PeripheralTickResult {
            irq: self.capture_irq_held(),
            cycles: 0,
            dma_signals: None,
            ..Default::default()
        }
    }
}

impl crate::Peripheral for Timer {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Scheduler mode: advance the lazy counter to the published "now"
        // first, so polled CNT/SR reads observe fresh time (batch-boundary
        // freshness; exact at interval 1 on the run path).
        self.sync_from_clock();
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        // Reading CCRx of a channel in input-capture mode clears CCxIF
        // (RM0368 §13.4.5: "cleared by software reading the TIMx_CCRx").
        if matches!(reg_offset, 0x34 | 0x38 | 0x3C | 0x40) {
            let ch = ((reg_offset - 0x34) / 4) as usize;
            if self.ccs(ch) != 0 {
                self.sr.set(self.sr.get() & !(1u32 << (ch + 1)));
            }
        }
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);

        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        if self.wave.wired {
            let now = self.wave_now();
            self.redrive_outputs(now);
        }
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        let freeze_before = self.freeze_revision.get();
        let result = self.tick_counter();
        if self.wave.wired && !self.scheduler_mode() {
            self.wave.walk_ticks += 1;
            if self.freeze_revision.get() != freeze_before {
                // The counter stopped or restarted under a held flag: the
                // outputs follow the counter, not the old phase.
                let now = self.wave_now();
                self.redrive_outputs(now);
            } else if self.wave.running {
                let now = self.wave_now();
                self.wave.pads.advance_to(now);
            }
        }
        result
    }

    fn uses_scheduler(&self) -> bool {
        // True once the bus attached its cycle clock (event-scheduler builds):
        // reads stay fresh through the lazy `advance_to` path and update/
        // compare IRQs ride scheduled events, so the walk is unnecessary.
        // Without a clock (feature off / hand-built buses) stay on the legacy
        // walk with exact historical semantics.
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        // Everything this model's `tick()` can ever do (prescaled up-count,
        // UIF/CCxIF latching, held-level NVIC pend) is event-expressible —
        // SMCR external-clock/encoder modes are inert register bits the walk
        // ignores identically, so no configuration needs a dynamic fallback.
        // In legacy mode (no clock / feature off) the walk does real work and
        // the conservative `true` stands.
        !self.scheduler_mode()
    }

    fn sync_to(&mut self, now_cycle: u64) {
        if self.scheduler_mode() {
            self.process_pending_edges(now_cycle);
            self.advance_to(now_cycle);
            if self.wave.wired {
                let now = self.wave_now();
                self.wave.pads.advance_to(now);
            }
        }
    }

    fn timer_input_stage(&self) -> Option<TimerInputStage> {
        if self.basic {
            None
        } else if self.ccer_np {
            Some(TimerInputStage::F4)
        } else {
            Some(TimerInputStage::F1)
        }
    }

    fn reads_can_deassert_irq(&self) -> bool {
        // Only a channel in input-capture mode has a read-to-clear flag.
        !self.basic && (self.ccmr1 & 0x0303 != 0 || self.ccmr2 & 0x0303 != 0)
    }

    fn timer_input_edge(&mut self, ti: u8, level: bool, cycle: u64) -> bool {
        if self.basic {
            return false;
        }
        self.input_edge(ti, level, cycle);
        true
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        // Kill any in-flight chain: the configuration (or counter) may have
        // just changed under this write, so its deadline is stale. The fresh
        // chain below carries the new token.
        // Counter-chain tokens stay below WAVE_TOKEN_BIT; the channel-output
        // waveform's edge wakes carry it.
        self.arm_seq = self.arm_seq.wrapping_add(1) & !WAVE_TOKEN_BIT;
        // `collect_scheduled_events` converts to `current_cycle + 1 + delay`;
        // the first pend lands `d` walk ticks after the just-synced state,
        // i.e. at absolute cycle `current_cycle + d` — hence `d - 1`
        // (d >= 1 always: a held level pends on the next tick, d == 1).
        let mut events: Vec<(u64, u32)> = self
            .ticks_until_first_pend()
            .map(|d| vec![(d - 1, self.arm_seq)])
            .unwrap_or_default();
        if self.wave.wired {
            events.extend(
                self.wave
                    .pads
                    .take_scheduled_events()
                    .into_iter()
                    .map(|(delay, token)| (delay, token | WAVE_TOKEN_BIT)),
            );
        }
        events
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if self.scheduler_mode() && event_token & WAVE_TOKEN_BIT != 0 {
            // A channel-output edge: publish it at its own cycle and arm the
            // next. Never touches the counter chain.
            let (_, reschedule_delay) = self
                .wave
                .pads
                .on_event(event_token & !WAVE_TOKEN_BIT, sched.now());
            return crate::sched::EventResult {
                reschedule_delay,
                ..Default::default()
            };
        }
        if !self.scheduler_mode() || event_token != self.arm_seq {
            // Stale chain (re-armed since this event was scheduled): die.
            return crate::sched::EventResult::default();
        }
        // Bring the lazy counter up to the drain cycle; this is what
        // materialises the update/compare latch this event was scheduled for.
        self.process_pending_edges(sched.now());
        self.advance_to(sched.now());
        if self.irq_level_held() {
            // The walk would return `irq: true` on this tick (overflow with
            // UIE, or the held level after a compare latch) and on every tick
            // after until firmware clears SR/DIER — perpetuate at delay 1,
            // pending the peripheral's own NVIC line through the same
            // `pend_irq_for_event` choke the legacy walk uses. Re-pends merge
            // in ISPR exactly as the walk's per-tick pends do.
            crate::sched::EventResult {
                raise_own_irq: true,
                reschedule_delay: Some(1),
                ..Default::default()
            }
        } else {
            // Not (yet) pending — defensively re-arm at the next computed
            // fire so the chain never silently dies while events are armed.
            crate::sched::EventResult {
                reschedule_delay: self.ticks_until_first_pend(),
                ..Default::default()
            }
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        // Anchor at the clock's current value so cycles that elapsed before
        // attach (normally zero — attach happens at bus assembly) are not
        // retroactively replayed into the counter.
        self.anchor.set(clock.now());
        self.clock = Some(clock);
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn wire_lines(&self) -> Option<&PadLines> {
        self.wave.pads.wire_lines()
    }

    fn timer_output_lines(&mut self, arm: bool) -> Option<Arc<PadLines>> {
        if self.basic {
            None
        } else if arm {
            Some(self.output_lines_arc())
        } else {
            Some(self.wave.pads.lines_arc())
        }
    }

    /// The timer's line IS a level — `irq_level_held()` is the same
    /// conjunction the walk re-pends on every tick. Publishing it lets the
    /// bus drop the pend again when firmware clears the flag inside the
    /// handler, which is what silicon does and what the walk alone cannot
    /// express (it only ever sets). Scheduler mode syncs first so a
    /// post-MMIO-write reconcile reads the flag the write just cleared.
    fn irq_line_level(&self) -> Option<bool> {
        self.sync_from_clock();
        Some(self.irq_level_held())
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        // Sync first so the serialized CNT/SR reflect "now" (scheduler mode);
        // no-op in legacy mode. Keeps the snapshot shape identical across
        // drive modes for the determinism gates.
        self.sync_from_clock();
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::{Timer, TimerChannelOutputMode};
    use crate::Peripheral;

    #[test]
    fn test_egr_ug_sets_uif_and_cnt_reset() {
        let mut tim = Timer::new();
        tim.write(0x24, 0x34).unwrap(); // CNT low byte
        tim.write(0x25, 0x12).unwrap(); // CNT high byte => 0x1234

        tim.write(0x14, 0x01).unwrap(); // EGR.UG

        let cnt_lo = tim.read(0x24).unwrap();
        let cnt_hi = tim.read(0x25).unwrap();
        let sr = tim.read(0x10).unwrap();
        assert_eq!((cnt_hi as u16) << 8 | cnt_lo as u16, 0);
        assert_eq!(sr & 0x1, 0x1);
    }

    #[test]
    fn test_egr_ug_latches_compare_match_flags() {
        // A bare UG from the reset state reloads CNT=0, which matches every
        // CCRx (all reset 0) in output-compare mode → SR = UIF + CC1..4IF =
        // 0x1F. Silicon-verified on STM32F103 TIM2 (stm32f1_exec_oracle).
        let mut tim = Timer::new();
        tim.write(0x14, 0x01).unwrap(); // EGR.UG
        assert_eq!(tim.read_reg(0x10), 0x1F);

        // With a CCRx moved off the (post-UG) CNT=0, that channel's compare
        // no longer matches, so its CCxIF stays clear after the next UG.
        tim.write(0x10, 0).unwrap(); // clear SR
        tim.write_reg(0x34, 0x20); // CCR1 = 0x20 (!= 0)
        tim.write(0x14, 0x01).unwrap(); // EGR.UG again
        assert_eq!(tim.read_reg(0x10), 0x1F & !0x2); // CC1IF (bit1) now clear
    }

    #[test]
    fn test_egr_ug_latches_cc5_cc6_on_advanced() {
        // Advanced timers also latch the internal output-only channels 5/6
        // at SR bits 16/17. Silicon-verified on STM32H563 TIM1 (2026-06-11):
        // SR read 0x0003001F while running from reset-zero CCR5/CCR6.
        let mut tim = Timer::new_with_layout(16, true);
        tim.write_reg(0x14, 0x01); // EGR.UG
        assert_eq!(tim.read_reg(0x10), 0x0003_001F);

        // A general-purpose instance must NOT grow the advanced-only bits.
        let mut gp = Timer::new();
        gp.write_reg(0x14, 0x01);
        assert_eq!(gp.read_reg(0x10), 0x1F);
    }

    #[test]
    fn test_basic_timer_ug_sets_only_uif() {
        // Basic timers (TIM6/7) have no capture/compare channels, so UG must
        // latch UIF alone — never the CCxIF flags a GP timer would set.
        let mut tim = Timer::new().basic(true);
        tim.write(0x14, 0x01).unwrap(); // EGR.UG
        assert_eq!(tim.read_reg(0x10), 0x1);
    }

    #[test]
    fn test_sr_write_zero_clears_uif_and_drops_irq() {
        let mut tim = Timer::new();

        // Enable UIE and set UIF via UG.
        tim.write(0x0C, 0x01).unwrap();
        tim.write(0x14, 0x01).unwrap();
        assert!(tim.tick().irq);

        // Clear UIF by writing 0 to SR bit 0.
        tim.write(0x10, 0x00).unwrap();
        assert_eq!(tim.read(0x10).unwrap() & 0x1, 0);
        assert!(!tim.tick().irq);
    }

    #[test]
    fn test_advanced_bdtr_round_trips_moe() {
        let mut tim = Timer::new_with_layout(16, true);
        // Enable MOE (bit 15) + dead-time generator value 0x40.
        tim.write(0x44, 0x40).unwrap();
        tim.write(0x45, 0x80).unwrap();
        let bdtr_lo = tim.read(0x44).unwrap();
        let bdtr_hi = tim.read(0x45).unwrap();
        assert_eq!(bdtr_lo, 0x40);
        assert_eq!(bdtr_hi, 0x80);
    }

    #[test]
    fn timer_output_snapshot_reports_disabled_outputs_and_moe() {
        let mut tim = Timer::new_with_layout(16, true);
        tim.write_u32(0x2C, 999).unwrap();
        tim.write_u32(0x34, 250).unwrap();
        tim.write_u32(0x20, 0x0005).unwrap(); // CC1E | CC1NE

        let output = tim.output_snapshot();
        assert!(!output.main_output_enabled);
        assert_eq!(output.channels[0].duty_fraction, 0.25);
        assert!(output.channels[0].enabled);
        assert!(output.channels[0].complementary_enabled);
        assert!(!output.counter_enabled);
        assert_eq!(output.period_ticks, 1000);
        assert_eq!(output.channels[0].mode, TimerChannelOutputMode::Unsupported);
    }

    #[test]
    fn timer_output_snapshot_reports_polarity_duty_and_dead_time() {
        let mut tim = Timer::new_with_layout(16, true);
        tim.write_u32(0x2C, 99).unwrap();
        tim.write_u32(0x34, 75).unwrap();
        tim.write_u32(0x20, 0x000F).unwrap(); // E/P/NE/NP
        tim.write_u32(0x18, 0x0060).unwrap(); // OC1M = PWM mode 1
        tim.write_u32(0x44, (1 << 15) | 0x40).unwrap();
        tim.write_u32(0x28, 3).unwrap(); // PSC=3: four CPU cycles per timer tick
        tim.write_u32(0x00, 1).unwrap(); // CEN
        assert!(!tim.tick().irq);
        assert!(!tim.tick().irq);

        let output = tim.output_snapshot();
        assert!(output.main_output_enabled);
        assert_eq!(output.dead_time_ticks, 64);
        assert_eq!(output.channels[0].duty_fraction, 0.75);
        assert!(output.channels[0].active_low);
        assert!(output.channels[0].complementary_active_low);
        assert!(output.counter_enabled);
        assert_eq!(output.counter_ticks, 0);
        assert_eq!(output.prescaler_divisor, 4);
        assert_eq!(output.prescaler_phase, 2);
        assert_eq!(output.period_ticks, 100);
        assert_eq!(output.channels[0].mode, TimerChannelOutputMode::Pwm1);
    }

    #[test]
    fn timer_phase_revision_tracks_only_phase_mapping_writes() {
        let mut tim = Timer::new_with_layout(16, true);
        assert_eq!(tim.output_snapshot().phase_revision, 0);
        tim.write_u32(0x34, 25).unwrap(); // CCR is a live waveform change.
        tim.write_u32(0x20, 1).unwrap(); // CCER/polarity is live too.
        tim.write_u32(0x44, 1 << 15).unwrap(); // MOE does not remap phase.
        assert_eq!(tim.output_snapshot().phase_revision, 0);

        for (offset, value) in [
            (0x24, 7),    // CNT
            (0x28, 3),    // PSC
            (0x2c, 99),   // ARR
            (0x18, 0x60), // PWM mode
            (0x00, 1),    // CEN transition
            (0x14, 1),    // explicit update/reset
        ] {
            let before = tim.output_snapshot().phase_revision;
            tim.write_u32(offset, value).unwrap();
            assert!(tim.output_snapshot().phase_revision > before);
        }
        let revision = tim.output_snapshot().phase_revision;
        assert!(!tim.tick().irq);
        assert_eq!(
            tim.output_snapshot().phase_revision,
            revision,
            "natural timer advancement must not invalidate an external cursor"
        );
    }

    #[test]
    fn test_advanced_rcr_writes_persisted() {
        let mut tim = Timer::new_with_layout(16, true);
        tim.write(0x30, 0x05).unwrap();
        assert_eq!(tim.read(0x30).unwrap(), 0x05);
    }

    #[test]
    fn test_basic_timer_ignores_advanced_regs() {
        let mut tim = Timer::new_with_layout(16, false);
        tim.write(0x44, 0x80).unwrap(); // BDTR — should no-op
        assert_eq!(tim.read(0x44).unwrap(), 0x00);
        tim.write(0x30, 0x05).unwrap(); // RCR — should no-op
        assert_eq!(tim.read(0x30).unwrap(), 0x00);
    }

    #[test]
    fn without_clock_stays_on_legacy_tick_path() {
        let tim = Timer::new();
        assert!(
            !tim.uses_scheduler(),
            "no cycle clock attached → the model must stay on the legacy walk \
             (hand-built buses that bypass the registration chokes keep exact semantics)"
        );
        assert!(tim.needs_legacy_walk());
    }

    /// The legacy tick charges ZERO cycle cost in every state (B2/B3 tick-cost
    /// normalization): an armed, overflowing, and held-level timer must never
    /// inflate `total_cycles`, so the walk-on reference and the scheduler path
    /// agree cycle-for-cycle.
    #[test]
    fn legacy_tick_charges_zero_cost_in_every_state() {
        let mut tim = Timer::new();
        tim.write_reg(0x2C, 1); // ARR = 1
        tim.write_reg(0x0C, 1); // DIER = UIE
        tim.write_reg(0x00, 1); // CR1 = CEN
        for _ in 0..10 {
            // Covers counting ticks, the overflow tick, and held-level ticks.
            assert_eq!(tim.tick().cycles, 0);
        }
    }

    // ── Input capture ──────────────────────────────────────────────────────

    /// General-purpose F4-style timer, CH1 on TI1 rising, CEN, PSC=0.
    fn ic_timer() -> Timer {
        let mut t = Timer::new_with_layout(32, false).ccer_np(true);
        t.write_reg(0x18, 0x01); // CC1S=01
        t.write_reg(0x20, 0x01); // CC1E, rising
        t.write_reg(0x00, 0x01); // CEN
        t
    }

    fn walk(t: &mut Timer, n: u32) {
        for _ in 0..n {
            t.tick();
        }
    }

    /// A walk-driven TIM3-class timer with its output wire armed, counting
    /// one count per tick (PSC = 0), ARR = 9, channel 1 in OC1M `mode`.
    fn output_timer(mode: u32, ccr1: u32, ccer: u32) -> Timer {
        let mut t = Timer::new();
        t.output_lines_arc();
        t.write_u32(0x2C, 9).unwrap(); // ARR
        t.write_u32(0x34, ccr1).unwrap(); // CCR1
        t.write_u32(0x18, mode << 4).unwrap(); // CCMR1.OC1M
        t.write_u32(0x20, ccer).unwrap(); // CCER
        t.write_u32(0x00, 1).unwrap(); // CEN
        t
    }

    /// OC1 sampled once per tick for `n` ticks.
    fn sample_oc1(t: &mut Timer, n: usize) -> Vec<bool> {
        (0..n)
            .map(|_| {
                t.tick();
                t.output_level(0)
            })
            .collect()
    }

    /// RM0008 §15.4.7 OC1M: PWM mode 1 (110) is active while CNT < CCR1, PWM
    /// mode 2 (111) inactive while CNT < CCR1; CC1P (§15.4.9) inverts. Over
    /// whole periods of ARR + 1 = 10 counts the high fraction is exact.
    #[test]
    fn pwm_modes_and_polarity_set_the_output_duty() {
        for (mode, ccer, want_high) in [
            (0b110, 0b01, 3), // PWM1: high for CNT 0..2
            (0b111, 0b01, 7), // PWM2: high for CNT 3..9
            (0b110, 0b11, 7), // PWM1, active low
            (0b111, 0b11, 3), // PWM2, active low
        ] {
            let mut t = output_timer(mode, 3, ccer);
            sample_oc1(&mut t, 10); // settle one period
            let high = sample_oc1(&mut t, 100).iter().filter(|&&h| h).count();
            assert_eq!(high, want_high * 10, "OC1M={mode:03b} CCER={ccer:02b}");
        }
    }

    /// CCR1 = 0 in PWM1 is never active, CCR1 > ARR always active (§15.3.9).
    #[test]
    fn pwm1_edge_duties_are_flat() {
        let mut off = output_timer(0b110, 0, 0b01);
        assert!(sample_oc1(&mut off, 40).iter().all(|&h| !h));
        let mut on = output_timer(0b110, 10, 0b01);
        assert!(sample_oc1(&mut on, 40).iter().all(|&h| h));
    }

    /// Toggle mode (011) flips OC1 every time CNT matches CCR1: a square wave
    /// of twice the counter period, 50 % duty.
    #[test]
    fn toggle_mode_flips_once_per_counter_period() {
        let mut t = output_timer(0b011, 4, 0b01);
        let s = sample_oc1(&mut t, 60);
        let flips: Vec<usize> = (1..s.len()).filter(|&i| s[i] != s[i - 1]).collect();
        assert!(flips.len() >= 4, "toggle must keep toggling: {s:?}");
        for w in flips.windows(2) {
            assert_eq!(w[1] - w[0], 10, "one toggle per ARR + 1 counts: {flips:?}");
        }
    }

    /// CC1E = 0: the timer does not drive the pad (line released to an
    /// input), whatever OC1M says; enabling takes it back.
    #[test]
    fn ccxe_releases_the_line() {
        use crate::peripherals::pad_lines::LineDrive;
        let mut t = output_timer(0b101, 0, 0b00);
        let lines = t.output_lines_arc();
        assert_eq!(lines.mode(0), LineDrive::Input);
        t.write_u32(0x20, 0b01).unwrap();
        assert_eq!(lines.mode(0), LineDrive::PushPull);
        assert!(lines.level(0), "OC1M=101 forced active");
    }

    #[test]
    fn capture_latches_cnt_and_sets_flag_on_the_selected_edge_only() {
        let mut t = ic_timer();
        walk(&mut t, 10);
        t.input_edge(0, false, 0); // falling: CC1P=0 ignores it
        assert_eq!(t.read_reg(0x10) & 0x2, 0);
        t.input_edge(0, true, 0);
        assert_eq!(t.read_reg(0x34), 10, "CCR1 = CNT at the edge");
        assert_eq!(t.read_reg(0x10) & 0x2, 0x2, "CC1IF");
        // Reading CCR1 in capture mode clears CC1IF.
        let _ = t.read(0x34).unwrap();
        assert_eq!(t.read_reg(0x10) & 0x2, 0);
    }

    #[test]
    fn falling_and_both_edge_polarity() {
        let mut t = ic_timer();
        t.write_reg(0x20, 0x03); // CC1E | CC1P: falling
        walk(&mut t, 3);
        t.input_edge(0, true, 0);
        assert_eq!(t.read_reg(0x10) & 0x2, 0);
        t.input_edge(0, false, 0);
        assert_eq!(t.read_reg(0x34), 3);
        t.write_reg(0x10, 0);
        t.write_reg(0x20, 0x0B); // CC1E | CC1P | CC1NP: both
        walk(&mut t, 2);
        t.input_edge(0, true, 0);
        assert_eq!(t.read_reg(0x34), 5);
        // Without CCxNP (F1 general-purpose) the NP bit does not stick.
        let mut f1 = Timer::new();
        f1.write_reg(0x20, 0x0B);
        assert_eq!(f1.read_reg(0x20), 0x03);
    }

    #[test]
    fn indirect_mapping_captures_the_other_input() {
        let mut t = ic_timer();
        t.write_reg(0x18, 0x0201); // CC1S=01 (TI1), CC2S=10 (TI1)
        t.write_reg(0x20, 0x31); // CC1E rising, CC2E|CC2P falling
        walk(&mut t, 4);
        t.input_edge(0, true, 0);
        walk(&mut t, 6);
        t.input_edge(0, false, 0);
        assert_eq!((t.read_reg(0x34), t.read_reg(0x38)), (4, 10));
        // TI2 edges do not reach a channel mapped to TI1.
        t.write_reg(0x10, 0);
        t.input_edge(1, true, 0);
        assert_eq!(t.read_reg(0x10) & 0x6, 0);
    }

    #[test]
    fn overcapture_sets_ccof_when_ccif_was_not_cleared() {
        let mut t = ic_timer();
        t.input_edge(0, true, 0);
        t.input_edge(0, false, 0);
        walk(&mut t, 2);
        t.input_edge(0, true, 0);
        assert_eq!(t.read_reg(0x10) & (1 << 9), 1 << 9, "CC1OF");
        assert_eq!(t.read_reg(0x34), 2, "the newest capture wins");
    }

    #[test]
    fn input_prescaler_captures_every_nth_edge() {
        let mut t = ic_timer();
        t.write_reg(0x20, 0x00); // CC1E off to change CCMR freely
        t.write_reg(0x18, 0x01 | (0b10 << 2)); // IC1PSC = /4
        t.write_reg(0x20, 0x01);
        for k in 1..=8u32 {
            walk(&mut t, 1);
            t.input_edge(0, true, 0);
            t.input_edge(0, false, 0);
            let captured = t.read_reg(0x10) & 0x2 != 0;
            assert_eq!(captured, k % 4 == 0, "edge {k}");
            t.write_reg(0x10, 0);
        }
        assert_eq!(t.read_reg(0x34), 8);
    }

    #[test]
    fn ccr_is_read_only_and_ccxs_locked_while_capturing() {
        let mut t = ic_timer();
        t.write_reg(0x34, 0x1234);
        assert_eq!(t.read_reg(0x34), 0, "CCR1 ignores writes in capture mode");
        t.write_reg(0x18, 0x00); // try to switch CH1 back to output
        assert_eq!(t.read_reg(0x18) & 0x3, 0x1, "CC1S locked while CC1E");
        t.write_reg(0x20, 0x00);
        t.write_reg(0x18, 0x00);
        assert_eq!(t.read_reg(0x18) & 0x3, 0);
        t.write_reg(0x34, 0x1234);
        assert_eq!(t.read_reg(0x34), 0x1234);
    }

    #[test]
    fn capture_flag_holds_the_irq_but_never_freezes_the_counter() {
        let mut t = ic_timer();
        t.write_reg(0x0C, 0x02); // CC1IE
        t.input_edge(0, true, 0);
        assert!(t.irq_level_held());
        let before = t.read_reg(0x24);
        let r = t.tick();
        assert!(r.irq, "held capture level pends");
        assert_eq!(t.read_reg(0x24), before + 1, "counter keeps running");
        assert!(!t.output_snapshot().counter_frozen);
    }

    #[test]
    fn reset_mode_on_ti1fp1_is_pwm_input() {
        let mut t = ic_timer();
        t.write_reg(0x18, 0x0201); // CC1S=TI1, CC2S=TI1
        t.write_reg(0x20, 0x31); // CC1 rising, CC2 falling
        t.write_reg(0x08, (5 << 4) | 4); // TS=TI1FP1, SMS=reset
        for _ in 0..3 {
            t.input_edge(0, true, 0);
            walk(&mut t, 30);
            t.input_edge(0, false, 0);
            walk(&mut t, 70);
        }
        t.input_edge(0, true, 0);
        assert_eq!(t.read_reg(0x34), 100, "CCR1 = period");
        assert_eq!(t.read_reg(0x38), 30, "CCR2 = high time");
        assert_eq!(t.read_reg(0x24), 0, "counter reset by the trigger");
        assert_ne!(t.read_reg(0x10) & (1 << 6), 0, "TIF");
        assert_ne!(t.read_reg(0x10) & 1, 0, "UIF from the reset update (URS=0)");
    }

    #[test]
    fn egr_ccxg_is_a_software_capture_on_input_channels() {
        let mut t = ic_timer();
        walk(&mut t, 7);
        t.write_reg(0x14, 0x02); // CC1G
        assert_eq!(t.read_reg(0x34), 7);
        assert_eq!(t.read_reg(0x10) & 0x2, 0x2);
    }

    #[cfg(feature = "event-scheduler")]
    mod scheduler_mode {
        use super::*;
        use crate::CycleClock;

        /// Mirror of the legacy per-tick walk semantics, kept in the test as
        /// an independent oracle: returns whether the walk pends the NVIC
        /// line on this tick.
        ///
        /// The bus walk pends from the peripheral's held LINE after the tick
        /// (`irq_line_level`, so a level pend tracks its flag both ways — see
        /// `reconcile_nvic_level`), not from `tick().irq`. The two agree on
        /// every tick except the compare-latch tick, where `irq_level_held()`
        /// is already true and `tick()` still returns `irq: false`; using
        /// `tick().irq` here encoded the pre-level-reconcile walk and made
        /// this oracle disagree with the bus.
        fn walk_tick_oracle(t: &mut Timer) -> bool {
            t.tick();
            t.irq_level_held()
        }

        /// Drive a scheduler-mode timer exactly the way `Machine` +
        /// `SystemBus` do at tick interval 1: publish the clock each cycle,
        /// convert write-armed events at `cycle + 1 + delay`, drain due
        /// events through `on_event` (rescheduling at `now + delay`), and
        /// record the cycles on which the event chain pends the own-IRQ.
        struct SchedHarness {
            tim: Timer,
            clock: CycleClock,
            sched: crate::sched::EventScheduler,
            bus: crate::bus::SystemBus,
            /// (deadline, token) — at most one live chain plus stale tokens.
            events: Vec<(u64, u32)>,
            now: u64,
        }

        impl SchedHarness {
            fn new(tim: Timer) -> Self {
                let clock = CycleClock::default();
                let mut tim = tim;
                tim.attach_cycle_clock(clock.clone());
                Self {
                    tim,
                    clock,
                    sched: crate::sched::EventScheduler::new(),
                    bus: crate::bus::SystemBus::new(),
                    events: Vec::new(),
                    now: 0,
                }
            }

            /// MMIO write at the current cycle, through the bus chokes'
            /// contract: sync first, write, then harvest `(delay, token)`
            /// as `now + 1 + delay` (the `collect_scheduled_events` identity).
            fn write(&mut self, offset: u64, value: u32) {
                self.tim.sync_to(self.now);
                self.tim.write_reg(offset, value);
                for (delay, token) in self.tim.take_scheduled_events() {
                    self.events.push((self.now + 1 + delay, token));
                }
            }

            /// Advance one cycle and drain due events; returns true if the
            /// chain pended the own-IRQ this cycle.
            fn step(&mut self) -> bool {
                self.now += 1;
                self.clock.publish(self.now);
                self.sched.advance_to(self.now);
                let mut pended = false;
                let due: Vec<(u64, u32)> = self
                    .events
                    .iter()
                    .copied()
                    .filter(|(d, _)| *d <= self.now)
                    .collect();
                self.events.retain(|(d, _)| *d > self.now);
                for (_, token) in due {
                    let res = self.tim.on_event(token, &mut self.sched, &mut self.bus);
                    if res.raise_own_irq {
                        pended = true;
                    }
                    if let Some(delay) = res.reschedule_delay {
                        self.events.push((self.now + delay, token));
                    }
                }
                pended
            }

            fn state(&self) -> (u32, u32, u32) {
                // Reads sync lazily, exactly like the MMIO read path.
                self.tim.sync_from_clock();
                (
                    self.tim.cnt.get(),
                    self.tim.sr.get(),
                    self.tim.psc_cnt.get(),
                )
            }
        }

        /// The heart of the fidelity gate: replay the SAME register-write
        /// script against (a) the legacy per-tick walk and (b) the lazy
        /// closed-form + event-chain scheduler path, comparing full counter
        /// state at EVERY cycle and the exact set of IRQ-pend cycles.
        fn assert_walk_identical(
            tim_factory: impl Fn() -> Timer,
            script: &[(u64, u64, u32)], // (cycle, offset, value) — applied before that cycle's tick
            cycles: u64,
            what: &str,
        ) {
            let mut walk = tim_factory();
            let mut sched = SchedHarness::new(tim_factory());

            let mut walk_pends: Vec<u64> = Vec::new();
            let mut sched_pends: Vec<u64> = Vec::new();

            for c in 1..=cycles {
                for &(sc, off, val) in script {
                    // A write during the instruction ending at cycle `c`
                    // syncs to `c - 1` (batch-start) — the harness models
                    // that; the walk applies it before tick `c`.
                    if sc == c {
                        walk.write_reg(off, val);
                        sched.now = c - 1;
                        sched.write(off, val);
                        sched.now = c - 1; // step() re-increments
                    }
                }
                if walk_tick_oracle(&mut walk) {
                    walk_pends.push(c);
                }
                sched.now = c - 1;
                if sched.step() {
                    sched_pends.push(c);
                }
                let (s_cnt, s_sr, s_psc) = sched.state();
                assert_eq!(
                    (walk.cnt.get(), walk.sr.get(), walk.psc_cnt.get()),
                    (s_cnt, s_sr, s_psc),
                    "{what}: state diverged at cycle {c}"
                );
            }
            assert_eq!(walk_pends, sched_pends, "{what}: IRQ pend cycles diverged");
        }

        fn gp32() -> Timer {
            Timer::new_with_layout(32, false)
        }

        #[test]
        fn clock_attach_flips_to_scheduler_and_walk_tick_is_inert() {
            let mut tim = Timer::new();
            tim.attach_cycle_clock(CycleClock::default());
            assert!(tim.uses_scheduler(), "clock attached → walk-independent");
            assert!(!tim.needs_legacy_walk());
            tim.write_reg(0x2C, 3);
            tim.write_reg(0x00, 1);
            let r = tim.tick();
            assert!(!r.irq);
            assert_eq!(tim.cnt.get(), 0, "tick inert in scheduler mode");
        }

        #[test]
        fn update_event_walk_identity_across_psc_arr_grid() {
            for psc in [0u32, 1, 2, 7] {
                for arr in [1u32, 3, 10, 50] {
                    let script = [
                        (1u64, 0x28u64, psc), // PSC
                        (1, 0x2C, arr),       // ARR
                        (1, 0x0C, 1),         // DIER = UIE
                        (1, 0x00, 1),         // CR1 = CEN
                        // ISR-style SR clear a while after the first fire.
                        (((psc as u64 + 1) * (arr as u64 + 1)) + 8, 0x10, 0),
                    ];
                    assert_walk_identical(
                        Timer::new,
                        &script,
                        3 * (psc as u64 + 1) * (arr as u64 + 1) + 40,
                        &format!("UIE psc={psc} arr={arr}"),
                    );
                }
            }
        }

        #[test]
        fn compare_match_walk_identity() {
            for (psc, arr, ccr) in [(0u32, 20u32, 7u32), (2, 50, 0), (1, 10, 10), (0, 5, 9)] {
                // ccr=9 > arr=5: unreachable compare — no event may ever fire.
                let script = [
                    (1u64, 0x28u64, psc),
                    (1, 0x2C, arr),
                    (1, 0x34, ccr),  // CCR1
                    (1, 0x0C, 0x02), // DIER = CC1IE
                    (1, 0x00, 1),
                    ((psc as u64 + 1) * (arr as u64 + 2) + 15, 0x10, 0), // SR clear
                ];
                assert_walk_identical(
                    Timer::new,
                    &script,
                    4 * (psc as u64 + 1) * (arr as u64 + 1) + 60,
                    &format!("CC1IE psc={psc} arr={arr} ccr={ccr}"),
                );
            }
        }

        #[test]
        fn polling_mode_walk_identity_no_dier() {
            // DIER=0: pure lazy counting + flag latching, no freeze, no IRQs.
            let script = [
                (1u64, 0x28u64, 1u32),
                (1, 0x2C, 6),
                (1, 0x34, 3),
                (1, 0x00, 1),
                (40, 0x10, 0), // poll-loop style SR clear
            ];
            assert_walk_identical(Timer::new, &script, 120, "polling DIER=0");
        }

        #[test]
        fn mid_run_reconfiguration_walk_identity() {
            // PSC/ARR/CNT/CCR rewrites mid-count (immediate-apply semantics,
            // no update-event buffering — the model's pinned behaviour),
            // EGR.UG software update, DIER enable of an already-latched flag.
            let script = [
                (1u64, 0x28u64, 1u32), // PSC=1
                (1, 0x2C, 30),         // ARR=30
                (1, 0x0C, 1),          // UIE
                (1, 0x00, 1),          // CEN
                (20, 0x28, 5),         // PSC shrink/grow mid-count (immediate)
                (35, 0x2C, 8),         // ARR below current CNT → wrap path
                (50, 0x10, 0),         // clear SR
                (60, 0x24, 100),       // CNT write above ARR
                (70, 0x14, 1),         // EGR.UG software update
                (90, 0x10, 0),
                (95, 0x0C, 0x03), // DIER: UIE|CC1IE with CCR1 latched?
                (110, 0x10, 0),
            ];
            assert_walk_identical(Timer::new, &script, 200, "mid-run reconfig");
        }

        #[test]
        fn psc_rewrite_keeps_phase_walk_identity() {
            // "PSC-buffered reload" honesty check: the model applies PSC
            // immediately (no buffer register) and keeps the prescaler phase,
            // including psc_cnt > PSC after a shrink — the scheduler path must
            // reproduce the walk's exact phase, not silicon's buffered reload.
            let script = [
                (1u64, 0x28u64, 9u32), // PSC=9
                (1, 0x2C, 100),
                (1, 0x00, 1),
                (7, 0x28, 2),  // shrink mid-phase (psc_cnt=6 > 2 → next tick bumps)
                (30, 0x28, 0), // PSC=0: every tick increments
            ];
            assert_walk_identical(Timer::new, &script, 80, "psc rewrite phase");
        }

        #[test]
        fn basic_timer_walk_identity() {
            let script = [
                (1u64, 0x28u64, 0u32),
                (1, 0x2C, 4),
                (1, 0x0C, 1),
                (1, 0x00, 1),
                (12, 0x10, 0),
            ];
            assert_walk_identical(|| Timer::new().basic(true), &script, 40, "basic TIM6/7");
        }

        #[test]
        fn advanced_timer_cc5_cc6_lazy_latch_walk_identity() {
            let script = [
                (1u64, 0x28u64, 0u32),
                (1, 0x2C, 10),
                (1, 0x58, 4), // CCR5
                (1, 0x5C, 7), // CCR6
                (1, 0x00, 1),
            ];
            assert_walk_identical(
                || Timer::new_with_layout(16, true),
                &script,
                40,
                "advanced CC5/6 lazy latch",
            );
        }

        #[test]
        fn arr_max_32bit_never_wraps_walk_identity() {
            // 32-bit reset ARR (0xFFFF_FFFF): the walk's `cnt > arr` can never
            // fire — free-run with no UIF. UIE armed must produce NO pends.
            let script = [(1u64, 0x0Cu64, 1u32), (1, 0x00, 1)];
            assert_walk_identical(gp32, &script, 100, "32-bit ARR=MAX free-run");
        }

        #[test]
        fn input_capture_channel_never_latches_or_fires() {
            // CCMR1.CC1S != 0 (input capture): the walk skips the channel in
            // latch_compare_match_flags; CC1IE must not schedule anything.
            let script = [
                (1u64, 0x18u64, 0x01u32), // CCMR1.CC1S = 01 (input)
                (1, 0x28, 0),
                (1, 0x2C, 6),
                (1, 0x34, 3),
                (1, 0x0C, 0x02), // CC1IE
                (1, 0x00, 1),
            ];
            assert_walk_identical(Timer::new, &script, 50, "input-capture CC1S!=0");
        }

        #[test]
        fn lazy_cnt_read_tracks_published_clock_exactly() {
            let clock = CycleClock::default();
            let mut tim = Timer::new();
            tim.attach_cycle_clock(clock.clone());
            // PSC=1 (increment every 2 ticks), ARR=5.
            tim.sync_to(0);
            tim.write_reg(0x28, 1);
            tim.write_reg(0x2C, 5);
            tim.write_reg(0x00, 1);
            let _ = tim.take_scheduled_events();
            clock.publish(4);
            assert_eq!(tim.read_u32(0x24).unwrap(), 2, "2 increments in 4 ticks");
            clock.publish(12);
            // 6 increments: values 1..5 then wrap to 0 at j=6 → CNT=0, UIF.
            assert_eq!(tim.read_u32(0x24).unwrap(), 0);
            assert_eq!(tim.read_u32(0x10).unwrap() & 1, 1, "UIF latched lazily");
        }

        #[test]
        fn take_scheduled_events_computes_exact_update_deadline() {
            let clock = CycleClock::default();
            let mut tim = Timer::new();
            tim.attach_cycle_clock(clock.clone());
            tim.sync_to(0);
            tim.write_reg(0x28, 2); // PSC=2 → period 3
            tim.write_reg(0x2C, 9); // ARR=9 → wrap at increment 10
            tim.write_reg(0x0C, 1); // UIE
            tim.write_reg(0x00, 1); // CEN
            let evs = tim.take_scheduled_events();
            // Fire at tick 3*10 = 30 after the synced state; bus adds
            // current_cycle + 1 + delay → delay = 29.
            assert_eq!(evs.len(), 1);
            assert_eq!(evs[0].0, 29, "update-event delay must be exact");
        }

        #[test]
        fn stale_event_chain_dies_on_token_mismatch() {
            let clock = CycleClock::default();
            let mut tim = Timer::new();
            tim.attach_cycle_clock(clock.clone());
            tim.sync_to(0);
            tim.write_reg(0x2C, 4);
            tim.write_reg(0x0C, 1);
            tim.write_reg(0x00, 1);
            let old_token = tim.take_scheduled_events()[0].1;
            // Re-arm (e.g. CNT rewrite): kills the old chain.
            tim.write_reg(0x24, 0);
            let new_token = tim.take_scheduled_events()[0].1;
            assert_ne!(old_token, new_token);
            clock.publish(500);
            let mut sched = crate::sched::EventScheduler::new();
            sched.advance_to(500);
            let mut bus = crate::bus::SystemBus::new();
            let res = tim.on_event(old_token, &mut sched, &mut bus);
            assert!(!res.raise_own_irq, "stale chain must be inert");
            assert_eq!(res.reschedule_delay, None, "stale chain must not respawn");
        }

        mod capture_scheduler {
            use super::super::*;
            use crate::CycleClock;

            fn clocked() -> (Timer, CycleClock) {
                let clock = CycleClock::default();
                let mut t = ic_timer();
                t.attach_cycle_clock(clock.clone());
                t.sync_to(0);
                (t, clock)
            }

            #[test]
            fn capture_uses_cnt_at_the_edge_cycle_not_at_the_read() {
                let (mut t, clock) = clocked();
                clock.publish(40);
                t.sync_to(40);
                t.input_edge(0, true, 40);
                clock.publish(1_000);
                assert_eq!(t.read_u32(0x34).unwrap(), 40);
                assert_eq!(t.read_u32(0x24).unwrap(), 1_000, "counter kept counting");
            }

            #[test]
            fn filter_delays_the_capture_and_swallows_short_glitches() {
                let (mut t, clock) = clocked();
                t.write_reg(0x20, 0);
                t.write_reg(0x18, 0x01 | (0b0011 << 4)); // IC1F=0011: fCK_INT, N=8
                t.write_reg(0x20, 0x01);
                // A 5-cycle glitch is shorter than the 8-sample filter.
                t.input_edge(0, true, 100);
                t.input_edge(0, false, 105);
                t.sync_to(200);
                assert_eq!(t.read_reg(0x10) & 0x2, 0, "glitch filtered out");
                // A long pulse is accepted 8 cycles after its edge.
                t.input_edge(0, true, 300);
                assert_eq!(
                    t.take_scheduled_events().first().map(|e| e.0),
                    Some(8 - 1 + 100),
                    "wake at the filter's acceptance cycle (delay relative to cycle 200)"
                );
                clock.publish(400);
                t.sync_to(400);
                assert_eq!(t.read_reg(0x34), 308, "CNT as of edge + filter");
            }
        }

        #[test]
        fn snapshot_shape_matches_legacy_mode() {
            let legacy = Timer::new();
            let mut sched = Timer::new();
            sched.attach_cycle_clock(CycleClock::default());
            let a = legacy.snapshot();
            let b = sched.snapshot();
            assert_eq!(
                a.as_object().unwrap().keys().collect::<Vec<_>>(),
                b.as_object().unwrap().keys().collect::<Vec<_>>(),
                "snapshot shape must be identical across drive modes"
            );
        }
    }
}
