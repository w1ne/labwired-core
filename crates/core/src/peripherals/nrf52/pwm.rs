// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 PWM peripheral — register surface + sequence playback engine.
//!
//! Source: nRF52840 PS rev 1.7 §6.18 (PWM). Models PWM0..PWM3 — same
//! register map per instance. Drives 4 outputs per peripheral.
//!
//! # Sequence engine (deterministic, synchronous)
//!
//! **TASKS_SEQSTART[n] (0x008 / 0x00C):** if ENABLE=1, the engine decodes
//! SEQ[n].PTR / SEQ[n].CNT and reads the `CNT` 16-bit duty values out of the
//! EasyDMA buffer in guest RAM (so the sequence registers genuinely drive the
//! playback), then fires EVENTS_SEQSTARTED[n] (0x108 / 0x10C), EVENTS_SEQEND[n]
//! (0x110 / 0x114) and EVENTS_PWMPERIODEND (0x118) to reflect the sequence
//! having played to completion against COUNTERTOP. The RAM read runs via
//! dual-path EasyDMA (`tick_with_bus` for bare-bus tests, delay-0 `on_event`
//! under Machine + event-scheduler).
//!
//! **TASKS_STOP (0x004):** fires EVENTS_STOPPED (0x104) synchronously.
//!
//! # Pad waveform
//!
//! After a sequence plays, the outputs keep generating the LAST loaded step
//! until TASKS_STOP or ENABLE=0 (PS §6.17: the PWM holds the last value). That
//! steady state is what `analogWrite` means on every nRF runtime we host:
//! CODAL's `NRF52Pin::setAnalogValue` plays a one-step, four-channel sequence
//! with LOOP=0 and SHORTS=0 each time the value changes, and relies on the
//! hold. Each connected `PSEL.OUT[n]` then carries that waveform onto its pad
//! through the chip's pin-claim table ([`NrfPinClaim`] + [`PadLines`], wired
//! in `SystemBus::wire_nrf52_pads`), exactly as a GPIOTE Task channel does;
//! before this, duty never left the register file and a PWM pin read flat.
//!
//! * Clock: `16 MHz / 2^PRESCALER`, converted to CPU cycles through the chip's
//!   `cpu_hz` ([`Peripheral::attach_cpu_hz`]), so a 20 ms servo frame is 20 ms
//!   of board time.
//! * MODE Up: a channel starts at its POLARITY level (bit 15 set = high,
//!   "FallingEdge") and inverts when the counter reaches COMP; MODE UpAndDown
//!   inverts on both ramps over a `2 * COUNTERTOP` period. COMP = 0 is the
//!   inverted level all period; COMP >= COUNTERTOP is the start level.
//! * DECODER.LOAD Common / Grouped / Individual / WaveForm choose how many
//!   16-bit values make a step and which channel each feeds; WaveForm's fourth
//!   value is the step's COUNTERTOP (channel 3 carries no output).
//! * A new sequence while one is playing takes effect at the next period
//!   boundary — the counter is not restarted — so a program re-writing its
//!   duty faster than the period still produces whole periods.
//! * Edges are published with their exact cycle (`PadLines::set_line_at`),
//!   and a scheduler wake lands on each one so live pad reads follow too.
//!
//! Not modelled: multi-step playback timing (REFRESH/ENDDELAY/LOOP; the events
//! still fire at once, as before), SHORTS, interrupts, and NEXTSTEP stepping.
//!
//! # EVENTS write semantics
//!
//! SW writes of 1 are silently ignored (hardware-generated only). SW writes of
//! 0 clear the event register.

use std::sync::Arc;

use crate::peripherals::nrf52::pin_select::{NrfPinClaim, NrfPinClaims};
use crate::peripherals::pad_lines::PadLines;
use crate::{Bus, CycleClock, Peripheral, SimResult};

/// Pad-wire line names, one per output channel, in `PSEL.OUT[n]` order.
pub const PWM_LINES: &[&str] = &["OUT0", "OUT1", "OUT2", "OUT3"];

/// The PWM base clock before PRESCALER (PS §6.17.2, "PWM_CLK = 16 MHz").
const PWM_BASE_HZ: u64 = 16_000_000;
/// nRF52 core clock, used until the bus hands over the chip's own `cpu_hz`
/// (hand-built buses never do).
const DEFAULT_CPU_HZ: u64 = 64_000_000;
/// `PSEL.OUT[n]` reset value: CONNECT = Disconnected (bit 31), as on every
/// nRF `PSEL` register. A reset of 0 named P0.00, so an unused channel of a
/// playing PWM would have claimed that pad.
const PSEL_OUT_RESET: u32 = 0xFFFF_FFFF;

/// No sequence pending.
const PENDING_NONE: u8 = 0;
/// TASKS_SEQSTART0 was written.
const PENDING_SEQ0: u8 = 1;
/// TASKS_SEQSTART1 was written.
const PENDING_SEQ1: u8 = 2;

const OFF_TASKS_STOP: u64 = 0x004;
const OFF_TASKS_SEQSTART0: u64 = 0x008;
const OFF_TASKS_SEQSTART1: u64 = 0x00C;
const OFF_TASKS_NEXTSTEP: u64 = 0x010;

const OFF_EVENTS_STOPPED: u64 = 0x104;
const OFF_EVENTS_SEQSTARTED0: u64 = 0x108;
const OFF_EVENTS_SEQSTARTED1: u64 = 0x10C;
const OFF_EVENTS_SEQEND0: u64 = 0x110;
const OFF_EVENTS_SEQEND1: u64 = 0x114;
const OFF_EVENTS_PWMPERIODEND: u64 = 0x118;
const OFF_EVENTS_LOOPSDONE: u64 = 0x11C;

const OFF_SHORTS: u64 = 0x200;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;

const OFF_ENABLE: u64 = 0x500;
const OFF_MODE: u64 = 0x504;
const OFF_COUNTERTOP: u64 = 0x508;
const OFF_PRESCALER: u64 = 0x50C;
const OFF_DECODER: u64 = 0x510;
const OFF_LOOP: u64 = 0x514;

// SEQ[0] block at 0x520..0x52F; SEQ[1] at 0x540..0x54F.
const OFF_SEQ_FIRST: u64 = 0x520;
const OFF_SEQ_LAST: u64 = 0x54C;

// PSEL.OUT[0..3] at 0x560..0x56C.
const OFF_PSEL_FIRST: u64 = 0x560;
const OFF_PSEL_LAST: u64 = 0x56C;

/// COUNTERTOP reset value (Product Specification, PWM registers).
const COUNTERTOP_RESET: u32 = 0x3FF;

#[derive(Debug, Default)]
pub struct Nrf52Pwm {
    events_stopped: u32,
    events_seqstarted: [u32; 2],
    events_seqend: [u32; 2],
    events_pwmperiodend: u32,
    events_loopsdone: u32,

    shorts: u32,
    inten: u32,

    enable: u32,
    mode: u32,
    countertop: u32,
    prescaler: u32,
    decoder: u32,
    loop_count: u32,

    seq: [u32; 12], // SEQ[0..1] x 4 words (PTR/CNT/REFRESH/ENDDELAY)
    psel_out: [u32; 4],

    /// Sequence pending for EasyDMA engine (`tick_with_bus` / `on_event`).
    /// One of PENDING_{NONE,SEQ0,SEQ1}.
    pending: u8,

    /// The waveform the outputs are generating, `None` while stopped.
    wave: Option<Wave>,
    /// A step loaded while `wave` was playing; it takes over at the next
    /// period boundary, as the silicon's compare reload does.
    next_step: Option<Step>,
    /// CPU cycle the model has been advanced to.
    anchor: u64,
    cpu_hz: u64,
    clock: Option<CycleClock>,
    /// Token of the one live scheduler wake; any other token is stale.
    arm_seq: u32,
    /// Absolute cycle the live wake targets, `None` when none is in flight.
    armed_target: Option<u64>,

    /// Per-channel pad wires, created at bus wiring time.
    lines: Option<Arc<PadLines>>,
    /// Each channel's claim on the pad its `PSEL.OUT[n]` names, live while
    /// the waveform plays.
    claims: [NrfPinClaim; 4],
}

/// One decoded sequence step: per-channel compare words and the period top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Step {
    /// Raw 16-bit values: bit 15 = POLARITY, bits 14:0 = COMP. `None` for a
    /// channel the step does not drive (WaveForm's channel 3).
    words: [Option<u16>; 4],
    /// COUNTERTOP for this step, in PWM clock ticks.
    top: u32,
    /// WaveForm load: `top` came from the sequence, not the register, so a
    /// later COUNTERTOP write does not move it.
    top_from_seq: bool,
}

/// One channel's shape within a period, in CPU cycles from the period start.
#[derive(Debug, Clone, Copy, Default)]
struct Shape {
    start_high: bool,
    toggles: [u64; 2],
    n_toggles: usize,
}

impl Shape {
    fn level_at(&self, offset: u64) -> bool {
        let flips = self.toggles[..self.n_toggles]
            .iter()
            .filter(|&&t| t <= offset)
            .count();
        self.start_high ^ (flips % 2 == 1)
    }
}

#[derive(Debug, Clone, Copy)]
struct Wave {
    step: Step,
    /// Cycle the current period began.
    period_start: u64,
    /// Period length in CPU cycles (>= 1).
    period: u64,
    shapes: [Shape; 4],
}

impl Nrf52Pwm {
    pub fn new() -> Self {
        let mut seq = [0; 12];
        // SEQ[n].REFRESH resets to 1 (Product Specification, PWM registers).
        seq[2] = 1;
        seq[10] = 1;
        Self {
            cpu_hz: DEFAULT_CPU_HZ,
            countertop: COUNTERTOP_RESET,
            seq,
            psel_out: [PSEL_OUT_RESET; 4],
            ..Self::default()
        }
    }

    fn enabled(&self) -> bool {
        self.enable & 1 != 0
    }

    /// This model's per-channel pad wires. Created at bus wiring time.
    pub(crate) fn pad_lines_arc(&mut self) -> Arc<PadLines> {
        self.lines
            .get_or_insert_with(|| Arc::new(PadLines::new(PWM_LINES, &[false; 4])))
            .clone()
    }

    /// Join the chip's pin-claim table: channel `n` claims under
    /// `first_token + n`. Config-build time only.
    pub(crate) fn install_pin_claims(&mut self, claims: &Arc<NrfPinClaims>, first_token: u32) {
        for (n, claim) in self.claims.iter_mut().enumerate() {
            claim.install(claims.clone(), first_token + n as u32);
        }
        self.sync_claims();
    }

    /// Republish every channel's claim: held while the waveform plays on an
    /// enabled PWM and the step drives that channel.
    fn sync_claims(&mut self) {
        let playing = self.enabled() && self.wave.is_some();
        for n in 0..4 {
            let drives = self.wave.is_some_and(|w| w.step.words[n].is_some());
            self.claims[n].update(self.psel_out[n], playing && drives);
        }
    }

    /// PWM clock ticks to CPU cycles at the current PRESCALER.
    fn ticks_to_cycles(&self, ticks: u64) -> u64 {
        let pwm_ticks = u128::from(ticks) << (self.prescaler & 0x7);
        (pwm_ticks * u128::from(self.cpu_hz) / u128::from(PWM_BASE_HZ)) as u64
    }

    /// Build the period shape a step produces under the current MODE.
    fn shape_wave(&self, step: Step, period_start: u64) -> Option<Wave> {
        let top = u64::from(step.top);
        if top == 0 {
            return None;
        }
        let up_down = self.mode & 1 != 0;
        let period = self.ticks_to_cycles(if up_down { 2 * top } else { top });
        if period == 0 {
            return None;
        }
        let mut shapes = [Shape::default(); 4];
        for (n, shape) in shapes.iter_mut().enumerate() {
            let Some(word) = step.words[n] else {
                continue;
            };
            let first_high = word & 0x8000 != 0;
            let comp = u64::from(word & 0x7FFF);
            *shape = if comp == 0 {
                Shape {
                    start_high: !first_high,
                    ..Shape::default()
                }
            } else if comp >= top {
                Shape {
                    start_high: first_high,
                    ..Shape::default()
                }
            } else if up_down {
                Shape {
                    start_high: first_high,
                    toggles: [
                        self.ticks_to_cycles(comp),
                        self.ticks_to_cycles(2 * top - comp),
                    ],
                    n_toggles: 2,
                }
            } else {
                Shape {
                    start_high: first_high,
                    toggles: [self.ticks_to_cycles(comp), 0],
                    n_toggles: 1,
                }
            };
        }
        Some(Wave {
            step,
            period_start,
            period,
            shapes,
        })
    }

    /// Drive every line to the waveform's level at absolute cycle `at`.
    fn publish_levels(&self, at: u64) {
        let (Some(lines), Some(w)) = (&self.lines, &self.wave) else {
            return;
        };
        let offset = (at - w.period_start) % w.period;
        for n in 0..4 {
            if w.step.words[n].is_some() {
                lines.set_line_at(n, w.shapes[n].level_at(offset), at);
            }
        }
    }

    /// The first instant after `after` at which any output may change: a
    /// channel toggle, or a period boundary that has a new step to load. A
    /// waveform with neither (every channel on a rail) needs no wake at all.
    fn next_edge_after(&self, after: u64) -> Option<u64> {
        let w = self.wave.as_ref()?;
        let into = (after - w.period_start) % w.period;
        let this_start = after - into;
        let mut best = None::<u64>;
        let mut toggles = false;
        for shape in &w.shapes {
            for &t in &shape.toggles[..shape.n_toggles] {
                toggles = true;
                let at = if t > into {
                    this_start + t
                } else {
                    this_start + w.period + t
                };
                best = Some(best.map_or(at, |b| b.min(at)));
            }
        }
        if toggles || self.next_step.is_some() {
            let boundary = this_start + w.period;
            best = Some(best.map_or(boundary, |b| b.min(boundary)));
        }
        best
    }

    /// Advance the waveform to `now`, publishing every edge on its own cycle.
    fn advance_to(&mut self, now: u64) {
        if now <= self.anchor {
            return;
        }
        while let Some(edge) = self.next_edge_after(self.anchor) {
            if edge > now {
                break;
            }
            let Some(w) = self.wave.as_mut() else {
                break;
            };
            if (edge - w.period_start) % w.period == 0 {
                w.period_start = edge;
                if let Some(step) = self.next_step.take() {
                    self.wave = self.shape_wave(step, edge);
                    self.sync_claims();
                }
            }
            self.anchor = edge;
            self.publish_levels(edge);
        }
        self.anchor = now;
    }

    /// Put a freshly played step on the outputs: at once from a stop, at the
    /// next period boundary while already playing.
    fn load_step(&mut self, step: Step, now: u64) {
        self.advance_to(now);
        match &self.wave {
            Some(_) => self.next_step = Some(step),
            None => {
                self.wave = self.shape_wave(step, self.anchor);
                self.next_step = None;
                self.sync_claims();
                self.publish_levels(self.anchor);
            }
        }
    }

    /// Stop the outputs; the pads fall back to their GPIO registers.
    fn stop_wave(&mut self) {
        self.wave = None;
        self.next_step = None;
        self.sync_claims();
    }

    /// The cycle the next scheduler wake should land on.
    fn wake_target(&self) -> Option<u64> {
        if self.pending != PENDING_NONE {
            return Some(self.anchor + 1);
        }
        self.next_edge_after(self.anchor)
    }

    fn now(&self) -> u64 {
        self.clock
            .as_ref()
            .map_or(self.anchor, |c| c.now().max(self.anchor))
    }
}

impl Peripheral for Nrf52Pwm {
    /// Dual-path EasyDMA: scheduler delay-0 (`on_event`) under Machine +
    /// walk-free + batched tick interval, and `tick_with_bus` for bare-bus
    /// unit tests. No time-driven walk work — SEQSTART completes on the next
    /// cycle via the scheduler under `rec_tick=512`, not at the bus-tick quantum.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_STOP | OFF_TASKS_SEQSTART0 | OFF_TASKS_SEQSTART1 | OFF_TASKS_NEXTSTEP => 0,

            OFF_EVENTS_STOPPED => self.events_stopped,
            OFF_EVENTS_SEQSTARTED0 => self.events_seqstarted[0],
            OFF_EVENTS_SEQSTARTED1 => self.events_seqstarted[1],
            OFF_EVENTS_SEQEND0 => self.events_seqend[0],
            OFF_EVENTS_SEQEND1 => self.events_seqend[1],
            OFF_EVENTS_PWMPERIODEND => self.events_pwmperiodend,
            OFF_EVENTS_LOOPSDONE => self.events_loopsdone,

            OFF_SHORTS => self.shorts,
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,

            OFF_ENABLE => self.enable & 1,
            OFF_MODE => self.mode & 0x1,
            OFF_COUNTERTOP => self.countertop & 0x7FFF,
            OFF_PRESCALER => self.prescaler & 0x7,
            OFF_DECODER => self.decoder & 0x103,
            OFF_LOOP => self.loop_count & 0xFFFF,

            OFF_SEQ_FIRST..=OFF_SEQ_LAST if offset.is_multiple_of(4) => {
                let idx = ((offset - OFF_SEQ_FIRST) / 4) as usize;
                if idx < 12 {
                    self.seq[idx]
                } else {
                    0
                }
            }
            OFF_PSEL_FIRST..=OFF_PSEL_LAST if offset.is_multiple_of(4) => {
                self.psel_out[((offset - OFF_PSEL_FIRST) / 4) as usize]
            }

            _ => {
                crate::census_reg!("nrf52.pwm:Nrf52Pwm", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // ── TASKS (sequence engine; gated on ENABLE) ────────────────────
            OFF_TASKS_SEQSTART0 if value != 0 && self.enabled() => {
                self.pending = PENDING_SEQ0;
            }
            OFF_TASKS_SEQSTART1 if value != 0 && self.enabled() => {
                self.pending = PENDING_SEQ1;
            }
            OFF_TASKS_STOP if value != 0 && self.enabled() => {
                self.advance_to(self.now());
                // A SEQSTART not yet serviced must not play after STOP.
                self.pending = PENDING_NONE;
                self.stop_wave();
                self.events_stopped = 1;
            }
            OFF_TASKS_STOP | OFF_TASKS_SEQSTART0 | OFF_TASKS_SEQSTART1 | OFF_TASKS_NEXTSTEP => {}

            // EVENTS_*: hardware-generated. SW write-1 is ignored; SW write-0 clears.
            OFF_EVENTS_STOPPED if value == 0 => self.events_stopped = 0,
            OFF_EVENTS_SEQSTARTED0 if value == 0 => self.events_seqstarted[0] = 0,
            OFF_EVENTS_SEQSTARTED1 if value == 0 => self.events_seqstarted[1] = 0,
            OFF_EVENTS_SEQEND0 if value == 0 => self.events_seqend[0] = 0,
            OFF_EVENTS_SEQEND1 if value == 0 => self.events_seqend[1] = 0,
            OFF_EVENTS_PWMPERIODEND if value == 0 => self.events_pwmperiodend = 0,
            OFF_EVENTS_LOOPSDONE if value == 0 => self.events_loopsdone = 0,

            OFF_SHORTS => self.shorts = value,
            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,

            OFF_ENABLE => {
                self.advance_to(self.now());
                self.enable = value & 1;
                if !self.enabled() {
                    self.pending = PENDING_NONE;
                    self.stop_wave();
                }
            }
            // Timebase changes reshape the playing step at the next period
            // boundary, where the silicon reloads its compares.
            OFF_MODE => {
                self.mode = value & 0x1;
                self.reshape_at_boundary();
            }
            OFF_COUNTERTOP => {
                self.countertop = value & 0x7FFF;
                self.reshape_at_boundary();
            }
            OFF_PRESCALER => {
                self.prescaler = value & 0x7;
                self.reshape_at_boundary();
            }
            OFF_DECODER => self.decoder = value & 0x103,
            OFF_LOOP => self.loop_count = value & 0xFFFF,

            OFF_SEQ_FIRST..=OFF_SEQ_LAST if offset.is_multiple_of(4) => {
                let idx = ((offset - OFF_SEQ_FIRST) / 4) as usize;
                if idx < 12 {
                    self.seq[idx] = value;
                }
            }
            OFF_PSEL_FIRST..=OFF_PSEL_LAST if offset.is_multiple_of(4) => {
                self.advance_to(self.now());
                self.psel_out[((offset - OFF_PSEL_FIRST) / 4) as usize] = value;
                self.sync_claims();
            }
            _ => {
                crate::census_reg!("nrf52.pwm:Nrf52Pwm", offset, "write");
            }
        }
        Ok(())
    }

    /// Dual path: bus_tick for bare-bus tests; on_event for scheduler. Without
    /// the scheduler there are no edge wakes, so a playing waveform rides the
    /// bus tick instead, read against the published cycle clock.
    fn needs_bus_tick(&self) -> bool {
        // Without the scheduler a playing waveform is walked per tick, but only
        // when there are pad lines to publish its edges on: nothing else
        // observes them, and advance_to catches up lazily on the next access.
        self.pending != PENDING_NONE
            || (!cfg!(feature = "event-scheduler") && self.wave.is_some() && self.lines.is_some())
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        let now = self.now();
        if self.pending != PENDING_NONE {
            self.do_easydma_seq(bus, now);
        }
        self.advance_to(now);
    }

    fn uses_scheduler(&self) -> bool {
        true
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn attach_cpu_hz(&mut self, hz: u64) {
        self.cpu_hz = hz.max(1);
    }

    fn sync_to(&mut self, now_cycle: u64) {
        self.advance_to(now_cycle);
    }

    /// One live wake, on the next thing that happens: the EasyDMA drain
    /// (delay 0 → next cycle) or the next output edge. The token discipline is
    /// the nRF TIMER's: a LATER target needs no new wake (the live one arrives
    /// first and re-arms), the SAME target is already armed, and only an
    /// EARLIER one takes a fresh token, which retires the old wake on arrival.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        let Some(target) = self.wake_target() else {
            return Vec::new();
        };
        if self.armed_target.is_some_and(|live| target >= live) {
            return Vec::new();
        }
        self.arm_seq = self.arm_seq.wrapping_add(1);
        self.armed_target = Some(target);
        vec![(target - self.anchor - 1, self.arm_seq)]
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token != self.arm_seq {
            return crate::sched::EventResult::default();
        }
        let now = sched.now().max(self.anchor);
        if self.pending != PENDING_NONE {
            self.do_easydma_seq(bus, now);
        }
        self.advance_to(now);
        // Re-armed under this same token by the machine.
        self.armed_target = self.wake_target();
        crate::sched::EventResult {
            reschedule_delay: self.armed_target.map(|t| t - now),
            ..Default::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl Nrf52Pwm {
    /// Re-derive the playing step from the current MODE / PRESCALER /
    /// COUNTERTOP, effective at the next period boundary.
    fn reshape_at_boundary(&mut self) {
        // Settle elapsed edges first, so the next boundary is counted from
        // the waveform as it stands at this write, not from a stale anchor.
        self.advance_to(self.now());
        let Some(w) = self.wave else {
            return;
        };
        let mut step = self.next_step.unwrap_or(w.step);
        if !step.top_from_seq {
            step.top = self.countertop;
        }
        self.next_step = Some(step);
    }

    /// Decode the step at halfword index `at` of a sequence, per DECODER.LOAD.
    fn decode_step(&self, halfwords: &[u16], at: usize) -> Step {
        let v = |i: usize| halfwords[at + i];
        let (words, top, top_from_seq) = match self.decoder & 0x3 {
            0 => ([Some(v(0)); 4], self.countertop, false), // Common
            1 => (
                [Some(v(0)), Some(v(0)), Some(v(1)), Some(v(1))], // Grouped
                self.countertop,
                false,
            ),
            2 => (
                [Some(v(0)), Some(v(1)), Some(v(2)), Some(v(3))], // Individual
                self.countertop,
                false,
            ),
            _ => (
                [Some(v(0)), Some(v(1)), Some(v(2)), None], // WaveForm
                u32::from(v(3) & 0x7FFF),
                true,
            ),
        };
        Step {
            words,
            top,
            top_from_seq,
        }
    }

    /// Sequence playback engine shared by `tick_with_bus` and `on_event`.
    /// Reads SEQ[n].CNT duty values from guest RAM at SEQ[n].PTR, fires
    /// SEQSTARTED[n], SEQEND[n], and PWMPERIODEND, and leaves the last step
    /// playing on the outputs (see the module docs).
    fn do_easydma_seq(&mut self, bus: &mut dyn Bus, now: u64) {
        let n = match self.pending {
            PENDING_SEQ0 => 0usize,
            PENDING_SEQ1 => 1usize,
            _ => return,
        };
        self.pending = PENDING_NONE;

        // SEQ[n] block: PTR at seq[n*8], CNT at seq[n*8 + 1].
        let ptr = self.seq[n * 8] as u64;
        let cnt = (self.seq[n * 8 + 1] & 0x7FFF) as usize;

        // Read each 16-bit duty value out of the EasyDMA buffer.
        let halfwords: Vec<u16> = (0..cnt)
            .map(|i| {
                let base = ptr + (i as u64) * 2;
                let lo = bus.read_u8(base).unwrap_or(0);
                let hi = bus.read_u8(base + 1).unwrap_or(0);
                u16::from_le_bytes([lo, hi])
            })
            .collect();

        self.events_seqstarted[n] = 1;
        self.events_seqend[n] = 1;
        self.events_pwmperiodend = 1;

        // The outputs hold the LAST complete step. A sequence shorter than
        // one step loads nothing.
        let per_step = match self.decoder & 0x3 {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        if halfwords.len() >= per_step {
            let last = (halfwords.len() / per_step - 1) * per_step;
            let step = self.decode_step(&halfwords, last);
            self.load_step(step, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bus, DmaRequest, SimulationConfig};
    use std::collections::HashMap;

    // ── Minimal flat-RAM bus (mirrors the TWIM test harness) ──────────────────
    struct FlatRam {
        mem: HashMap<u64, u8>,
        config: SimulationConfig,
    }

    impl FlatRam {
        fn new() -> Self {
            Self {
                mem: HashMap::new(),
                config: SimulationConfig::default(),
            }
        }
        fn write_slice(&mut self, base: u64, data: &[u8]) {
            for (i, &b) in data.iter().enumerate() {
                self.mem.insert(base + i as u64, b);
            }
        }
    }

    impl Bus for FlatRam {
        fn read_u8(&self, addr: u64) -> crate::SimResult<u8> {
            Ok(*self.mem.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> crate::SimResult<()> {
            self.mem.insert(addr, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> crate::SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
    }

    #[test]
    fn countertop_masks_to_15_bits() {
        let mut p = Nrf52Pwm::new();
        p.write_u32(OFF_COUNTERTOP, 0xFFFF_FFFF).unwrap();
        assert_eq!(p.read_u32(OFF_COUNTERTOP).unwrap(), 0x7FFF);
    }

    #[test]
    fn psel_out_resets_disconnected_and_claims_nothing() {
        let claims = Arc::new(crate::peripherals::nrf52::pin_select::nrf_pin_claims());
        let (mut p, _lines) = playing(0, 0, 100, &[0x8000 | 50]); // Common: all 4 play
        p.install_pin_claims(&claims, 0);
        for n in 0..4u64 {
            assert_eq!(p.read_u32(OFF_PSEL_FIRST + 4 * n).unwrap(), PSEL_OUT_RESET);
        }
        assert_eq!(
            claims.selector(0, 0),
            None,
            "P0.00 is not claimed by a reset PSEL"
        );
    }

    #[test]
    fn countertop_and_refresh_reset_to_the_product_specification() {
        let p = Nrf52Pwm::new();
        assert_eq!(p.read_u32(OFF_COUNTERTOP).unwrap(), 0x3FF);
        assert_eq!(p.read_u32(OFF_SEQ_FIRST + 8).unwrap(), 1, "SEQ[0].REFRESH");
        assert_eq!(
            p.read_u32(OFF_SEQ_FIRST + 0x28).unwrap(),
            1,
            "SEQ[1].REFRESH"
        );
    }

    #[test]
    fn stop_cancels_a_sequence_not_yet_started() {
        let mut p = Nrf52Pwm::new();
        let mut bus = FlatRam::new();
        bus.write_slice(0x2000_0000, &[0x10, 0x80]);
        p.write_u32(OFF_ENABLE, 1).unwrap();
        p.write_u32(OFF_SEQ_FIRST, 0x2000_0000).unwrap();
        p.write_u32(OFF_SEQ_FIRST + 4, 1).unwrap();
        p.write_u32(OFF_TASKS_SEQSTART0, 1).unwrap();
        p.write_u32(OFF_TASKS_STOP, 1).unwrap();
        p.tick_with_bus(&mut bus);
        assert_eq!(p.read_u32(OFF_EVENTS_SEQSTARTED0).unwrap(), 0);
        assert!(p.wave.is_none(), "nothing plays after STOP");
    }

    #[test]
    fn psel_out_round_trips() {
        let mut p = Nrf52Pwm::new();
        p.write_u32(0x560, 13).unwrap(); // PSEL.OUT[0] = P0.13
        assert_eq!(p.read_u32(0x560).unwrap(), 13);
    }

    #[test]
    fn seqstart0_plays_sequence_and_fires_events() {
        let mut p = Nrf52Pwm::new();
        let mut bus = FlatRam::new();
        let base: u64 = 0x2000_0000;
        // Two 16-bit duty samples.
        bus.write_slice(base, &[0x10, 0x80, 0x20, 0x80]);

        p.write_u32(OFF_ENABLE, 1).unwrap();
        p.write_u32(OFF_COUNTERTOP, 1000).unwrap();
        p.write_u32(OFF_SEQ_FIRST, base as u32).unwrap(); // SEQ[0].PTR
        p.write_u32(OFF_SEQ_FIRST + 4, 2).unwrap(); // SEQ[0].CNT = 2

        p.write_u32(OFF_TASKS_SEQSTART0, 1).unwrap();
        assert_eq!(p.read_u32(OFF_EVENTS_SEQEND0).unwrap(), 0, "before tick");
        assert!(p.needs_bus_tick());

        p.tick_with_bus(&mut bus);

        assert_eq!(
            p.read_u32(OFF_EVENTS_SEQSTARTED0).unwrap(),
            1,
            "SEQSTARTED0"
        );
        assert_eq!(p.read_u32(OFF_EVENTS_SEQEND0).unwrap(), 1, "SEQEND0");
        assert_eq!(
            p.read_u32(OFF_EVENTS_PWMPERIODEND).unwrap(),
            1,
            "PWMPERIODEND"
        );
        assert_eq!(p.pending, PENDING_NONE, "pending cleared");
        assert!(p.wave.is_some(), "the last step keeps playing");
    }

    #[test]
    fn seqstart1_uses_seq1_block() {
        let mut p = Nrf52Pwm::new();
        let mut bus = FlatRam::new();
        let base: u64 = 0x2000_0100;
        bus.write_slice(base, &[0xAA, 0x00]);

        p.write_u32(OFF_ENABLE, 1).unwrap();
        // SEQ[1].PTR is at 0x540, CNT at 0x544.
        p.write_u32(0x540, base as u32).unwrap();
        p.write_u32(0x544, 1).unwrap();

        p.write_u32(OFF_TASKS_SEQSTART1, 1).unwrap();
        p.tick_with_bus(&mut bus);

        assert_eq!(p.read_u32(OFF_EVENTS_SEQSTARTED1).unwrap(), 1);
        assert_eq!(p.read_u32(OFF_EVENTS_SEQEND1).unwrap(), 1);
    }

    #[test]
    fn seqstart_ignored_when_disabled() {
        let mut p = Nrf52Pwm::new();
        // ENABLE left at 0.
        p.write_u32(OFF_TASKS_SEQSTART0, 1).unwrap();
        assert!(!p.needs_bus_tick(), "disabled PWM does not arm a sequence");
    }

    #[test]
    fn stop_sets_stopped_when_enabled() {
        let mut p = Nrf52Pwm::new();
        p.write_u32(OFF_ENABLE, 1).unwrap();
        p.write_u32(OFF_TASKS_STOP, 1).unwrap();
        assert_eq!(p.read_u32(OFF_EVENTS_STOPPED).unwrap(), 1);
    }

    #[test]
    fn seqstart_schedules_delay0_event() {
        let mut p = Nrf52Pwm::new();
        p.write_u32(OFF_ENABLE, 1).unwrap();
        assert!(p.uses_scheduler());
        assert!(p.take_scheduled_events().is_empty());
        p.write_u32(OFF_TASKS_SEQSTART0, 1).unwrap();
        assert_eq!(p.take_scheduled_events(), vec![(0, 1)]);
    }

    #[test]
    fn on_event_completes_easydma_seq() {
        use crate::sched::EventScheduler;

        let mut p = Nrf52Pwm::new();
        let mut bus = FlatRam::new();
        let base: u64 = 0x2000_0000;
        bus.write_slice(base, &[0x10, 0x80, 0x20, 0x80]);

        p.write_u32(OFF_ENABLE, 1).unwrap();
        p.write_u32(OFF_SEQ_FIRST, base as u32).unwrap();
        p.write_u32(OFF_SEQ_FIRST + 4, 2).unwrap();
        p.write_u32(OFF_TASKS_SEQSTART0, 1).unwrap();
        let [(_, token)] = p.take_scheduled_events()[..] else {
            panic!("SEQSTART arms one wake");
        };

        let mut sched = EventScheduler::new();
        let _ = p.on_event(token, &mut sched, &mut bus);

        assert_eq!(p.read_u32(OFF_EVENTS_SEQSTARTED0).unwrap(), 1);
        assert_eq!(p.read_u32(OFF_EVENTS_SEQEND0).unwrap(), 1);
        assert_eq!(p.read_u32(OFF_EVENTS_PWMPERIODEND).unwrap(), 1);
        assert!(!p.needs_bus_tick());
    }

    // ── Pad waveform ─────────────────────────────────────────────────────────

    const SEQ_RAM: u64 = 0x2000_0000;

    /// PWM playing `values` (one step, DECODER.LOAD = `load`) at PRESCALER 0 and
    /// COUNTERTOP `top`, started at cycle 0 on a 64 MHz core: 4 cycles a tick.
    fn playing(load: u32, mode: u32, top: u32, values: &[u16]) -> (Nrf52Pwm, Arc<PadLines>) {
        let mut p = Nrf52Pwm::new();
        let lines = p.pad_lines_arc();
        let mut bus = FlatRam::new();
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        bus.write_slice(SEQ_RAM, &bytes);
        p.write_u32(OFF_ENABLE, 1).unwrap();
        p.write_u32(OFF_MODE, mode).unwrap();
        p.write_u32(OFF_COUNTERTOP, top).unwrap();
        p.write_u32(OFF_DECODER, load).unwrap();
        p.write_u32(OFF_SEQ_FIRST, SEQ_RAM as u32).unwrap();
        p.write_u32(OFF_SEQ_FIRST + 4, values.len() as u32).unwrap();
        p.write_u32(OFF_TASKS_SEQSTART0, 1).unwrap();
        p.tick_with_bus(&mut bus);
        (p, lines)
    }

    /// Cycles `line` spends high over `[from, to)`, sampled every cycle.
    fn high_cycles(p: &mut Nrf52Pwm, lines: &PadLines, line: usize, from: u64, to: u64) -> u64 {
        (from..to)
            .filter(|&c| {
                p.sync_to(c);
                lines.level(line)
            })
            .count() as u64
    }

    #[test]
    fn up_mode_rising_edge_polarity_is_high_from_comp_to_top() {
        // CODAL's analog write: no POLARITY bit, COMP = top * (1 - duty).
        // top 100, COMP 75 → high for the last 25 ticks = 100 of 400 cycles.
        let (mut p, lines) = playing(2, 0, 100, &[75, 0, 0, 0]);
        assert!(!lines.level(0), "starts low");
        assert_eq!(high_cycles(&mut p, &lines, 0, 0, 4 * 400), 4 * 100);
        // The edge is on the cycle COMP names, not near it.
        p.sync_to(4 * 400 + 299);
        assert!(!lines.level(0), "cycle 299 of the period is still low");
        p.sync_to(4 * 400 + 300);
        assert!(lines.level(0), "cycle 300 is high");
    }

    #[test]
    fn falling_edge_polarity_is_high_until_comp() {
        // Adafruit/Arduino: bit 15 set, COMP = high time. 30 of 100 ticks.
        let (mut p, lines) = playing(2, 0, 100, &[0x8000 | 30, 0, 0, 0]);
        assert!(lines.level(0), "starts high");
        assert_eq!(high_cycles(&mut p, &lines, 0, 0, 3 * 400), 3 * 120);
    }

    #[test]
    fn comp_zero_and_comp_at_top_are_the_rails() {
        let (mut p, lines) = playing(2, 0, 100, &[0, 100, 0x8000, 0x8000 | 100]);
        let span = 2 * 400;
        assert_eq!(
            high_cycles(&mut p, &lines, 0, 0, span),
            span,
            "rise at 0 = on"
        );
        let (mut p, lines) = playing(2, 0, 100, &[0, 100, 0x8000, 0x8000 | 100]);
        assert_eq!(
            high_cycles(&mut p, &lines, 1, 0, span),
            0,
            "rise never = off"
        );
        let (mut p, lines) = playing(2, 0, 100, &[0, 100, 0x8000, 0x8000 | 100]);
        assert_eq!(
            high_cycles(&mut p, &lines, 2, 0, span),
            0,
            "fall at 0 = off"
        );
        let (mut p, lines) = playing(2, 0, 100, &[0, 100, 0x8000, 0x8000 | 100]);
        assert_eq!(
            high_cycles(&mut p, &lines, 3, 0, span),
            span,
            "fall never = on"
        );
    }

    #[test]
    fn up_and_down_mode_is_centred_over_twice_the_top() {
        // top 100 → 200-tick period; COMP 60, rising polarity → high while the
        // counter is above 60: ticks 60..140, 80 of 200 = 320 of 800 cycles.
        let (mut p, lines) = playing(2, 1, 100, &[60, 0, 0, 0]);
        assert_eq!(high_cycles(&mut p, &lines, 0, 0, 2 * 800), 2 * 320);
        p.sync_to(2 * 800 + 239);
        assert!(!lines.level(0));
        p.sync_to(2 * 800 + 240);
        assert!(lines.level(0));
        p.sync_to(2 * 800 + 559);
        assert!(lines.level(0));
        p.sync_to(2 * 800 + 560);
        assert!(!lines.level(0));
    }

    #[test]
    fn decoder_load_modes_fan_the_values_out() {
        // Common: one value drives all four.
        let (mut p, lines) = playing(0, 0, 100, &[0x8000 | 50]);
        p.sync_to(10);
        assert!((0..4).all(|n| lines.level(n)));
        // Grouped: first value → 0,1; second → 2,3.
        let (mut p, lines) = playing(1, 0, 100, &[0x8000 | 50, 0x8000]);
        p.sync_to(10);
        assert_eq!(
            (0..4).map(|n| lines.level(n)).collect::<Vec<_>>(),
            [true, true, false, false]
        );
        // WaveForm: fourth value is the step's top; 20 of 40 ticks high.
        let (mut p, lines) = playing(3, 0, 1000, &[0x8000 | 20, 0, 0, 40]);
        assert_eq!(high_cycles(&mut p, &lines, 0, 0, 2 * 160), 2 * 80);
    }

    #[test]
    fn the_last_step_of_a_sequence_is_the_one_that_holds() {
        // Two Individual steps; the outputs keep the second.
        let (mut p, lines) = playing(2, 0, 100, &[0x8000 | 10, 0, 0, 0, 0x8000 | 90, 0, 0, 0]);
        assert_eq!(high_cycles(&mut p, &lines, 0, 0, 400), 360);
    }

    #[test]
    fn prescaler_and_cpu_hz_set_the_period_in_board_time() {
        // CODAL's 50 Hz analog period: PRESCALER 4 (1 MHz), COUNTERTOP 20000.
        // At 64 MHz that is 1_280_000 cycles = 20 ms.
        let mut p = Nrf52Pwm::new();
        p.write_u32(OFF_PRESCALER, 4).unwrap();
        assert_eq!(p.ticks_to_cycles(20_000), 1_280_000);
        p.attach_cpu_hz(16_000_000);
        assert_eq!(p.ticks_to_cycles(20_000), 320_000);
    }

    #[test]
    fn a_new_value_while_playing_waits_for_the_period_boundary() {
        let (mut p, lines) = playing(2, 0, 100, &[0x8000 | 50, 0, 0, 0]);
        let mut bus = FlatRam::new();
        bus.write_slice(SEQ_RAM, &(0x8000u16 | 10).to_le_bytes());
        bus.write_slice(SEQ_RAM + 2, &[0; 6]);
        p.sync_to(100); // mid-period: high (0..200)
        p.write_u32(OFF_TASKS_SEQSTART0, 1).unwrap();
        p.tick_with_bus(&mut bus);
        // Still the old shape for the rest of this period…
        assert_eq!(high_cycles(&mut p, &lines, 0, 100, 400), 100);
        // …then the new one: 10 ticks = 40 cycles.
        assert_eq!(high_cycles(&mut p, &lines, 0, 400, 800), 40);
    }

    #[test]
    fn stop_and_disable_release_the_pad() {
        let claims = Arc::new(crate::peripherals::nrf52::pin_select::nrf_pin_claims());
        let (mut p, _lines) = playing(2, 0, 100, &[0x8000 | 50, 0, 0, 0]);
        p.install_pin_claims(&claims, 7);
        assert_eq!(
            claims.selector(0, 2),
            None,
            "PSEL.OUT[0] disconnected at reset"
        );
        p.write_u32(OFF_PSEL_FIRST, 2).unwrap(); // P0.02
        assert_eq!(
            claims.selector(0, 2),
            Some(7),
            "a playing channel owns its pad"
        );
        p.write_u32(OFF_TASKS_STOP, 1).unwrap();
        assert_eq!(
            claims.selector(0, 2),
            None,
            "STOP hands the pad back to GPIO"
        );

        let (mut p, _lines) = playing(2, 0, 100, &[0x8000 | 50, 0, 0, 0]);
        p.install_pin_claims(&claims, 7);
        p.write_u32(OFF_PSEL_FIRST, 2).unwrap();
        assert_eq!(claims.selector(0, 2), Some(7));
        p.write_u32(OFF_ENABLE, 0).unwrap();
        assert_eq!(claims.selector(0, 2), None, "ENABLE=0 hands the pad back");
    }

    #[test]
    fn edges_carry_their_own_cycle_even_when_time_jumps() {
        use crate::logic_capture::LogicTap;
        let (mut p, lines) = playing(2, 0, 100, &[75, 0, 0, 0]);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![], vec![], vec![]]);
        // One jump across three periods — no sample in between.
        p.sync_to(3 * 400 + 10);
        let edges: Vec<(u64, bool)> = tap
            .take_events()
            .iter()
            .map(|e| (e.cycle, e.value))
            .collect();
        assert_eq!(
            edges,
            [
                (300, true),
                (400, false),
                (700, true),
                (800, false),
                (1100, true),
                (1200, false)
            ]
        );
    }

    #[test]
    fn the_scheduler_wakes_on_each_edge_and_goes_quiet_when_stopped() {
        use crate::sched::EventScheduler;
        let (mut p, _lines) = playing(2, 0, 100, &[75, 0, 0, 0]);
        // Rise at 300: the harvest delay is measured from the synced anchor
        // and lands one cycle past it plus the delay.
        let [(delay, token)] = p.take_scheduled_events()[..] else {
            panic!("a playing waveform arms a wake");
        };
        assert_eq!(delay, 300 - 1);
        assert!(
            p.take_scheduled_events().is_empty(),
            "the same wake is not re-armed"
        );
        let mut sched = EventScheduler::new();
        let mut bus = FlatRam::new();
        sched.advance_to(300);
        let r = p.on_event(token, &mut sched, &mut bus);
        assert_eq!(
            r.reschedule_delay,
            Some(100),
            "next edge: the period end at 400"
        );
        assert!(p
            .on_event(token.wrapping_add(9), &mut sched, &mut bus)
            .reschedule_delay
            .is_none());
        p.write_u32(OFF_TASKS_STOP, 1).unwrap();
        sched.advance_to(400);
        assert_eq!(
            p.on_event(token, &mut sched, &mut bus).reschedule_delay,
            None
        );
    }
}
