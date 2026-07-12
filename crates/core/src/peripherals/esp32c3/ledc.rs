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
//!
//! ## Drive modes (walk-free plan — the C3 timer-port batch)
//!
//! Two mutually exclusive time sources, selected by ONE predicate
//! (`scheduler_mode`), mirroring the SYSTIMER / STM32 TIMx timer ports:
//!
//! * **Scheduler mode** (`event-scheduler` feature + a [`crate::CycleClock`]
//!   attached by `SystemBus::add_peripheral`): `uses_scheduler()` is true and
//!   the per-cycle walk skips this peripheral entirely. The four up-counters are
//!   advanced **lazily** — `advance_to(now)` runs from the write-path `sync_to`
//!   choke, from every `&self` register read, and from the level poll
//!   (`matrix_irq_sources`), pulling "now" from the bus-published clock. The
//!   advance is closed-form (the accumulator already converts an arbitrary
//!   elapsed-cycle window into counts in O(1)), so a read observes fresh
//!   counter/`INT_RAW` state without any walk. The next `LSTIMERx_OVF` is
//!   armed as a **scheduled event** (`take_scheduled_events` → the nearest
//!   overflow among the running timers); `on_event` materialises the latch at
//!   its exact cycle and re-arms the successor, so the OVF interrupt is
//!   delivered through the C3 interrupt matrix (`matrix_irq_sources` re-derived
//!   by the bus) even when firmware never polls the block. An arm-token
//!   (`arm_seq`, bumped on every re-arm) kills a stale in-flight event after a
//!   config write changes the period/divider/enable.
//!
//! * **Legacy mode** (feature off, or no clock attached — e.g. hand-built test
//!   buses that bypass `add_peripheral`, or the differential's
//!   `force_legacy_walk`): the per-cycle walk drives `tick_elapsed(cycles)` and
//!   the counters advance eagerly, byte-identical to the historical model.
//!
//! The two modes are mutually exclusive: `tick_elapsed` is a no-op while
//! scheduler mode is active (the walk never calls it there — the guard is
//! defensive), and the lazy `advance_to` path is anchored so repeated syncs to
//! the same cycle are idempotent. All model quirks are preserved across both
//! modes: the `RST`/`PAUSE` freeze, the `2^DUTY_RES` wrap point (clamped to the
//! 14-bit counter), the integer `CLK_DIV` divider, and the sticky W1C `OVF`
//! latch. Every counter read, `INT_RAW`/`INT_ST` value and OVF-IRQ cycle is
//! byte-identical to the walk-driven reference at tick interval 1.

use std::cell::Cell;

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

pub const LEDC_BASE: u32 = 0x6001_9000;
pub const LEDC_SIZE: u64 = 0x400;

/// `LEDC` interrupt-matrix source on the C3 (`LEDC_INT_MAP` at
/// `interrupt_core0.yaml` offset 92 = `4 * 23`).
pub const LEDC_INTR_SOURCE_ID: u32 = 23;

/// Number of low-speed timers in the C3 LEDC block.
const NUM_TIMERS: usize = 4;

/// Number of channels in the C3 LEDC block.
const NUM_CHANNELS: usize = 6;

/// Silicon reset value of each `LEDC_CHn_CONF1` register (offset `0x0C + n*0x14`):
/// the channel's `DUTY_START`/duty-increment defaults latched at power-on. These
/// registers are pure register-backed state in this model, so seeding the reset
/// value matches the silicon capture without changing timer behavior.
const CH_CONF1_RESET: u32 = 0x4000_0000;

/// Silicon reset value of each `LEDC_TIMERx_CONF` register (offset `0xA0 + x*8`):
/// the `RST` bit (bit 23) is set at power-on, i.e. every timer is held in reset
/// until firmware configures and releases it. The behavioral counter already
/// honors `RST` (held at 0 while set), so this is the correct cold-reset seed.
const TIMER_CONF_RESET: u32 = 1 << 23;

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
    /// Live up-counter (the value `TIMERx_VALUE.CNT` returns). `Cell` so the
    /// scheduler-mode `&self` read/level-poll path can lazily advance it to the
    /// bus-published clock; in legacy mode only `tick_elapsed`/`write_u32`
    /// mutate it.
    counter: Cell<u32>,
    /// Sub-count accumulator: CPU cycles elapsed toward the next count. `Cell`
    /// for the same lazy `&self` advance.
    accum: Cell<u64>,
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

    /// True while this timer is clocking (neither held in reset nor paused) —
    /// the only state in which it can advance and overflow.
    fn running(&self) -> bool {
        !self.in_reset() && !self.paused()
    }

    /// Advance by `cycles` CPU cycles. Returns true if the counter wrapped past
    /// its period at least once during this elapsed window. `&self` (all mutated
    /// state is in `Cell`) so it serves both the legacy walk and the lazy
    /// scheduler-mode advance. Closed-form: the accumulator converts an
    /// arbitrary elapsed window into counts in O(1), which is exactly what makes
    /// `advance_cycles(N) == N × advance_cycles(1)` (the sticky OVF latch does
    /// not care how many times it wrapped within the window).
    fn advance_cycles(&self, cycles: u64) -> bool {
        if self.in_reset() {
            self.counter.set(0);
            self.accum.set(0);
            return false;
        }
        if self.paused() {
            return false;
        }
        let accum = self.accum.get().saturating_add(cycles);
        let per_count = self.divider();
        let counts = accum / per_count;
        self.accum.set(accum % per_count);
        if counts == 0 {
            return false;
        }
        let period = self.period() as u64;
        let next = self.counter.get() as u64 + counts;
        let overflowed = next >= period;
        self.counter.set((next % period) as u32);
        overflowed
    }

    /// Cycles from the current state until this timer's NEXT `LSTIMERx_OVF`
    /// (the exact cycle the per-cycle walk would latch it), or `None` when the
    /// timer is not clocking. The counter overflows once its value reaches the
    /// period; that needs `need_counts` more counts, and each count consumes
    /// `divider` cycles net of the accumulator already banked — so the first
    /// wrap is exactly `need_counts * divider - accum` cycles away (always
    /// `>= 1`, since `accum < divider`).
    fn cycles_to_overflow(&self) -> Option<u64> {
        if !self.running() {
            return None;
        }
        let period = self.period() as u64;
        let counter = self.counter.get() as u64;
        // `counter < period` in the normal case; a config write that shrinks the
        // period below the live counter overflows on the very next count (the
        // walk's `next >= period` fires immediately), so clamp `need_counts` to
        // at least 1.
        let need_counts = period.saturating_sub(counter).max(1);
        Some(need_counts * self.divider() - self.accum.get())
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
    /// driven by this model. `Cell` so the scheduler-mode lazy advance (a
    /// `&self` read / level poll) can latch a freshly-materialised overflow.
    int_raw: Cell<u32>,
    /// `INT_ENA` mask.
    int_ena: u32,
    /// Lazy-path anchor: the absolute CPU cycle the timers were last advanced
    /// to. Owned exclusively by `advance_to` (scheduler mode); the legacy walk
    /// never touches it.
    anchor: Cell<u64>,
    /// Arming-sequence token, bumped on every `take_scheduled_events`, so an
    /// in-flight overflow event scheduled under an older configuration dies on
    /// arrival (token mismatch) instead of racing the fresh chain.
    arm_seq: u32,
    /// Bus-published cycle clock (walk-free plan). `Some` once
    /// `SystemBus::add_peripheral` attaches it (under the `event-scheduler`
    /// feature); its presence flips the model onto the event scheduler. `None`
    /// (feature off, a hand-built bus, or the differential's
    /// `force_legacy_walk`) keeps the legacy per-cycle walk. Not serialized —
    /// re-attached by the bus.
    clock: Option<CycleClock>,
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
            self.int_raw.get(),
            self.timers[0].counter.get(),
            self.timers[1].counter.get(),
            self.timers[2].counter.get(),
            self.timers[3].counter.get()
        )
    }
}

impl Esp32c3Ledc {
    pub fn new(source_id: u32) -> Self {
        // Seed the register-backed window with the silicon reset state so the
        // cold-reset readback matches the captured oracle. Channel CONF1
        // registers carry a non-zero reset value; the rest power up at 0.
        let mut regs = vec![0u32; (LEDC_SIZE / 4) as usize];
        for n in 0..NUM_CHANNELS {
            let off = 0x0C + n * 0x14;
            regs[off / 4] = CH_CONF1_RESET;
        }
        // Every timer's CONF reset value holds RST (bit 23) set, matching silicon.
        let timers = core::array::from_fn(|_| Timer {
            conf: TIMER_CONF_RESET,
            ..Default::default()
        });
        Self {
            source_id,
            regs,
            timers,
            int_raw: Cell::new(0),
            int_ena: 0,
            anchor: Cell::new(0),
            arm_seq: 0,
            clock: None,
        }
    }

    /// True when the event scheduler owns this block's time base (feature on
    /// AND bus clock attached). Everything time-related branches on this ONE
    /// predicate so the two drive modes can never mix.
    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to the
    /// legacy per-cycle walk (`uses_scheduler() == false`). Used by the
    /// walk-on-vs-scheduler differential gates to build the reference config
    /// from the same bus assembly (mirrors `Esp32c3I2c::force_legacy_walk`).
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// Lazy advance every timer to absolute CPU cycle `now` — callable from
    /// `&self` (all mutated state is in `Cell`). Idempotent: repeated calls with
    /// the same `now` add nothing; a `now` older than the anchor is ignored (the
    /// clock is monotonic within a run; a stale read must never rewind). Latches
    /// `LSTIMERx_OVF` for any timer that wraps in the elapsed window — exactly
    /// what the per-cycle walk does, materialised in closed form.
    fn advance_to(&self, now: u64) {
        let anchor = self.anchor.get();
        if now <= anchor {
            return;
        }
        let delta = now - anchor;
        self.anchor.set(now);
        let mut raw = self.int_raw.get();
        for (i, timer) in self.timers.iter().enumerate() {
            if timer.advance_cycles(delta) {
                raw |= 1 << i;
            }
        }
        self.int_raw.set(raw);
    }

    /// Pull "now" from the bus-published clock and advance. No-op without an
    /// attached clock (legacy mode — the walk advances the counters instead).
    fn sync_from_clock(&self) {
        if self.scheduler_mode() {
            if let Some(clock) = &self.clock {
                self.advance_to(clock.now());
            }
        }
    }

    /// Cycles until the NEAREST overflow among the running timers (the next
    /// cycle any `LSTIMERx_OVF` latches), or `None` when every timer is
    /// stopped. This is what a single in-flight event is armed for; `on_event`
    /// re-arms the successor after each fire.
    fn cycles_to_next_overflow(&self) -> Option<u64> {
        self.timers
            .iter()
            .filter_map(|t| t.cycles_to_overflow())
            .min()
    }

    fn int_st(&self) -> u32 {
        self.int_raw.get() & self.int_ena
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
        // Scheduler mode: bring every lazy counter up to the published "now"
        // first, so a polled TIMERx_VALUE / INT_RAW read observes fresh time
        // (exact at interval 1; batch-boundary fresh otherwise). No-op in
        // legacy mode (the walk already advanced the counters).
        self.sync_from_clock();
        let off = offset & !3;
        if let Some((idx, is_value)) = Self::timer_at(off) {
            return Ok(if is_value {
                self.timers[idx].counter.get() & CNT_MASK
            } else {
                self.timers[idx].conf
            });
        }
        Ok(match off {
            INT_RAW => self.int_raw.get(),
            INT_ST => self.int_st(),
            INT_ENA => self.int_ena,
            INT_CLR => 0, // write-only
            o => self.reg(o),
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // Bring the lazy counters up to "now" BEFORE the config change lands, so
        // a period/divider/enable rewrite (or an INT_CLR acknowledge) applies to
        // the up-to-date counter/INT_RAW state — the closed-form window must
        // never straddle a settings change. The bus write choke already ran
        // `sync_to(current_cycle)`; this keeps direct/unit-test writes correct
        // too, and is idempotent. No-op in legacy mode.
        self.sync_from_clock();
        let off = offset & !3;
        if let Some((idx, is_value)) = Self::timer_at(off) {
            if is_value {
                // CNT is read-only on silicon; ignore writes.
            } else {
                // Store the config (PARA_UP is a self-clearing commit strobe);
                // applying DUTY_RES/CLK_DIV immediately is sufficient here.
                self.timers[idx].conf = value & !PARA_UP_BIT;
                if value & RST_BIT != 0 {
                    self.timers[idx].counter.set(0);
                    self.timers[idx].accum.set(0);
                }
            }
            return Ok(());
        }
        match off {
            // R/WTC and W1C: writing 1s clears those latched overflow bits.
            INT_RAW | INT_CLR => {
                self.int_raw
                    .set(self.int_raw.get() & !(value & OVF_INT_MASK));
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
        self.tick_elapsed(1)
    }

    /// LEGACY per-cycle walk drive: advance every timer by `cycles` and latch
    /// any overflow, then re-assert the level interrupt while enabled. In
    /// scheduler mode ([`Self::uses_scheduler`] true) the walk skips this
    /// peripheral entirely and the counters advance via the lazy `sync_to` /
    /// scheduled-event path instead; the guard keeps a stray direct call from
    /// double-counting against the lazy anchor.
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        if !self.scheduler_mode() {
            let mut raw = self.int_raw.get();
            for (i, timer) in self.timers.iter().enumerate() {
                if timer.advance_cycles(cycles) {
                    raw |= 1 << i;
                }
            }
            self.int_raw.set(raw);
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

    fn legacy_tick_active(&self) -> bool {
        self.int_st() != 0 || self.timers.iter().any(|timer| timer.running())
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    /// Walk-free plan: driven by the event scheduler once the bus has attached
    /// its cycle clock (production `add_peripheral` always does, under the
    /// `event-scheduler` feature). The per-cycle walk then skips this
    /// peripheral; the counters advance via `sync_to` (write path) + lazy reads,
    /// and each `LSTIMERx_OVF` rides a scheduled event (`take_scheduled_events`
    /// / `on_event`) so the IRQ is delivered at its exact cycle even when
    /// firmware never polls. Without a clock (feature off, a hand-built bus, or
    /// `force_legacy_walk`) it stays on the legacy walk so those callers keep
    /// the old exact semantics.
    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        // Every effect of the legacy `tick_elapsed` (prescaled up-count, sticky
        // OVF latch, level-IRQ re-assert) is reproduced in scheduler mode by the
        // lazy advance + scheduled overflow events, so the walk is unnecessary
        // there. In legacy mode (no clock / feature off) the walk does real work
        // and the conservative `true` stands.
        !self.scheduler_mode()
    }

    /// Anchor every timer's lazy state to CPU cycle `now_cycle`, advancing over
    /// the cycles elapsed since the last sync. The bus calls this before every
    /// MMIO write (so a config / INT_CLR write observes up-to-date counters) and
    /// it composes with `on_event` through the shared `anchor` without
    /// double-counting.
    fn sync_to(&mut self, now_cycle: u64) {
        if self.scheduler_mode() {
            self.advance_to(now_cycle);
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        // Anchor at the clock's current value so cycles that elapsed before
        // attach (normally zero — attach happens at bus assembly) are not
        // retroactively credited to the counters (the #516 re-anchor contract).
        self.anchor.set(clock.now());
        self.clock = Some(clock);
    }

    /// C3 interrupt-matrix level: the `LEDC` source while any enabled OVF bit is
    /// latched — the exact condition `tick_elapsed` pushes on the legacy walk.
    /// Syncs the lazy counters first so an overflow that just came due is
    /// reflected (the bus polls this on the event path and every walk-tick
    /// aggregation, `refresh_esp32c3_sched_sources`), keeping the level-sensitive
    /// IRQ routed and de-asserting the tick after firmware writes INT_CLR.
    fn matrix_irq_sources(&self) -> Vec<u32> {
        self.sync_from_clock();
        if self.int_st() != 0 {
            vec![self.source_id]
        } else {
            Vec::new()
        }
    }

    /// Arm the nearest overflow as a single in-flight event. A fresh `arm_seq`
    /// generation is stamped on every call so an event scheduled under an older
    /// configuration (before this write) dies on arrival (token mismatch in
    /// `on_event`). The delay is relative to the just-synced anchor; the bus
    /// converts it to the absolute deadline `anchor + 1 + delay`, so the
    /// `saturating_sub(1)` lands the fire exactly at `anchor +
    /// cycles_to_next_overflow` — the cycle the walk would latch it (mirrors the
    /// generic `+ 1` write-path anchor offset). `on_event` re-arms thereafter.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        self.arm_seq = self.arm_seq.wrapping_add(1);
        match self.cycles_to_next_overflow() {
            Some(cycles) => vec![(cycles.saturating_sub(1), self.arm_seq)],
            None => Vec::new(),
        }
    }

    /// Fire the nearest overflow at its exact cycle: advance every timer to the
    /// drain cycle (materialising the latch this event was scheduled for), then
    /// re-arm the successor while any timer is still clocking. The counters do
    /// not freeze on a latched IRQ (LEDC keeps free-running), so the chain is
    /// purely about delivering each OVF level at the right cycle; the bus
    /// re-derives the matrix source from `matrix_irq_sources` after this handler.
    /// The reschedule delay carries no `- 1`: the event path uses `sched.now() +
    /// delay` directly (no `+ 1` anchor offset, unlike the write path).
    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() || event_token != self.arm_seq {
            // Stale chain (re-armed since this event was scheduled): die.
            return crate::sched::EventResult::default();
        }
        self.advance_to(sched.now());
        let mut res = crate::sched::EventResult::default();
        if let Some(cycles) = self.cycles_to_next_overflow() {
            res.reschedule_delay = Some(cycles);
        }
        res
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
    fn tick_elapsed_matches_repeated_tick() {
        let mut repeated = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
        let mut elapsed = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
        repeated.write_u32(TIMER0_CONF_OFF, conf(4, 2)).unwrap();
        elapsed.write_u32(TIMER0_CONF_OFF, conf(4, 2)).unwrap();
        repeated.write_u32(INT_ENA, 1).unwrap();
        elapsed.write_u32(INT_ENA, 1).unwrap();

        let mut repeated_irq = None;
        for _ in 0..40 {
            repeated_irq = repeated.tick().explicit_irqs;
        }
        let elapsed_irq = elapsed.tick_elapsed(40).explicit_irqs;

        assert_eq!(
            elapsed.read_u32(TIMER0_VALUE_OFF).unwrap(),
            repeated.read_u32(TIMER0_VALUE_OFF).unwrap()
        );
        assert_eq!(
            elapsed.read_u32(INT_RAW).unwrap(),
            repeated.read_u32(INT_RAW).unwrap()
        );
        assert_eq!(elapsed_irq, repeated_irq);
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

    #[test]
    fn without_clock_stays_on_legacy_tick_path() {
        let l = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
        assert!(
            !l.uses_scheduler(),
            "no cycle clock attached → the model must stay on the legacy walk \
             (hand-built buses that bypass add_peripheral keep exact semantics)"
        );
        assert!(l.needs_legacy_walk());
    }

    /// Walk-free timer-port fidelity gate: the lazy closed-form advance + the
    /// scheduled-overflow event chain must reproduce the legacy per-cycle walk
    /// EXACTLY — every TIMERx_VALUE read, INT_RAW value and OVF level-assert
    /// cycle byte-identical at tick interval 1.
    #[cfg(feature = "event-scheduler")]
    mod scheduler_mode {
        use super::*;
        use crate::CycleClock;

        const TIMER1_CONF_OFF: u64 = 0xB0;

        /// Drive a scheduler-mode LEDC exactly the way `Machine` + `SystemBus`
        /// do at tick interval 1: publish the clock each cycle, convert
        /// write-armed events at `cycle + 1 + delay`, drain due events through
        /// `on_event` (rescheduling at `now + delay`).
        struct SchedHarness {
            ledc: Esp32c3Ledc,
            clock: CycleClock,
            sched: crate::sched::EventScheduler,
            bus: crate::bus::SystemBus,
            /// (deadline, token) — at most one live chain plus stale tokens.
            events: Vec<(u64, u32)>,
            now: u64,
        }

        impl SchedHarness {
            fn new() -> Self {
                let clock = CycleClock::default();
                let mut ledc = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
                ledc.attach_cycle_clock(clock.clone());
                Self {
                    ledc,
                    clock,
                    sched: crate::sched::EventScheduler::new(),
                    bus: crate::bus::SystemBus::new(),
                    events: Vec::new(),
                    now: 0,
                }
            }

            /// MMIO write at the current cycle, through the bus chokes' contract:
            /// sync first, write, then harvest `(delay, token)` as
            /// `now + 1 + delay` (the `collect_scheduled_events` identity).
            fn write(&mut self, off: u64, val: u32) {
                self.ledc.sync_to(self.now);
                self.ledc.write_u32(off, val).unwrap();
                for (delay, token) in self.ledc.take_scheduled_events() {
                    self.events.push((self.now + 1 + delay, token));
                }
            }

            /// Advance one cycle and drain due events.
            fn step(&mut self) {
                self.now += 1;
                self.clock.publish(self.now);
                self.sched.advance_to(self.now);
                let due: Vec<(u64, u32)> = self
                    .events
                    .iter()
                    .copied()
                    .filter(|(d, _)| *d <= self.now)
                    .collect();
                self.events.retain(|(d, _)| *d > self.now);
                for (_, token) in due {
                    let res = self.ledc.on_event(token, &mut self.sched, &mut self.bus);
                    if let Some(delay) = res.reschedule_delay {
                        self.events.push((self.now + delay, token));
                    }
                }
            }

            /// Event-materialised INT_RAW — read WITHOUT syncing, so it reflects
            /// only what the scheduled events latched (proves delivery without a
            /// polling read).
            fn event_int_raw(&self) -> u32 {
                self.ledc.int_raw.get()
            }

            /// Synced MMIO read (the real firmware read path).
            fn read(&self, off: u64) -> u32 {
                self.ledc.read_u32(off).unwrap()
            }
        }

        /// Replay the SAME register-write script against (a) the legacy per-tick
        /// walk and (b) the lazy closed-form + event-chain scheduler path,
        /// comparing all four counters + INT_RAW at EVERY cycle and the exact set
        /// of OVF level-assert cycles. Returns the count of cycles the OVF level
        /// was asserted in the reference run (for the caller's non-vacuity gate).
        fn assert_walk_identical(script: &[(u64, u64, u32)], cycles: u64, what: &str) -> usize {
            let mut walk = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID); // no clock → legacy
            let mut sched = SchedHarness::new();
            let mut walk_assert: Vec<u64> = Vec::new();
            let mut sched_assert: Vec<u64> = Vec::new();

            for c in 1..=cycles {
                sched.now = c - 1;
                for &(sc, off, val) in script {
                    if sc == c {
                        walk.write_u32(off, val).unwrap();
                        sched.now = c - 1;
                        sched.write(off, val);
                    }
                }
                sched.now = c - 1;

                // Walk reference: tick at cycle c.
                if walk.tick().explicit_irqs.is_some() {
                    walk_assert.push(c);
                }
                // Scheduler: advance to cycle c (drain events), then poll the
                // matrix level exactly as the bus aggregation does.
                sched.step();
                if !sched.ledc.matrix_irq_sources().is_empty() {
                    sched_assert.push(c);
                }

                // Byte-identity of every observable timer register.
                for t in 0..NUM_TIMERS as u64 {
                    let voff = 0xA4 + t * 8;
                    assert_eq!(
                        walk.read_u32(voff).unwrap(),
                        sched.read(voff),
                        "{what}: TIMER{t} counter diverged at cycle {c}"
                    );
                }
                assert_eq!(
                    walk.read_u32(INT_RAW).unwrap(),
                    sched.read(INT_RAW),
                    "{what}: INT_RAW diverged at cycle {c}"
                );
                assert_eq!(
                    walk.read_u32(INT_ST).unwrap(),
                    sched.read(INT_ST),
                    "{what}: INT_ST diverged at cycle {c}"
                );
            }
            assert_eq!(
                walk_assert, sched_assert,
                "{what}: OVF level-assert cycles diverged"
            );
            walk_assert.len()
        }

        #[test]
        fn clock_attach_flips_to_scheduler() {
            let mut l = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
            l.attach_cycle_clock(CycleClock::default());
            assert!(l.uses_scheduler(), "clock attached → walk-independent");
            assert!(!l.needs_legacy_walk());
        }

        #[test]
        fn single_timer_update_walk_identity() {
            // period 16, divider 1, OVF int enabled: overflows every 16 cycles.
            let script = [
                (1u64, TIMER0_CONF_OFF, conf(4, 1)),
                (1, INT_ENA, 1),
                // ISR clears the OVF a while after the first fire.
                (40, INT_CLR, 1),
            ];
            let fired = assert_walk_identical(&script, 120, "single-timer period 16 div 1");
            assert!(fired > 0, "non-vacuity: the timer must actually overflow");
        }

        #[test]
        fn divided_timer_walk_identity() {
            // period 8, divider 5: overflow at cycle 8*5 = 40, then every 40.
            let script = [
                (1u64, TIMER0_CONF_OFF, conf(3, 5)),
                (1, INT_ENA, 1),
                (100, INT_CLR, 1),
            ];
            let fired = assert_walk_identical(&script, 200, "divided period 8 div 5");
            assert!(fired > 0, "non-vacuity: the divided timer must overflow");
        }

        #[test]
        fn two_timers_different_periods_walk_identity() {
            // Timer0 period 16 div 1 (OVF@16,32,…), Timer1 period 8 div 3
            // (OVF@24,48,…); both OVF ints enabled — proves the single min-event
            // chain materialises EACH timer's overflow at its own cycle.
            let script = [
                (1u64, TIMER0_CONF_OFF, conf(4, 1)),
                (1, TIMER1_CONF_OFF, conf(3, 3)),
                (1, INT_ENA, 0b11),
                (200, INT_CLR, 0b11),
            ];
            let fired = assert_walk_identical(&script, 300, "two timers 16/1 + 8/3");
            assert!(fired > 0, "non-vacuity: both timers must overflow");
        }

        #[test]
        fn pause_and_reset_midrun_walk_identity() {
            let running = conf(4, 1);
            let script = [
                (1u64, TIMER0_CONF_OFF, running),
                (1, INT_ENA, 1),
                (10, TIMER0_CONF_OFF, running | PAUSE_BIT), // freeze
                (30, TIMER0_CONF_OFF, running),             // resume from frozen phase
                (60, INT_CLR, 1),
                (70, TIMER0_CONF_OFF, running | RST_BIT), // hold at 0
                (90, TIMER0_CONF_OFF, running),           // release
            ];
            let fired = assert_walk_identical(&script, 200, "pause/reset mid-run");
            assert!(
                fired > 0,
                "non-vacuity: the timer must overflow across the run"
            );
        }

        #[test]
        fn disabled_int_still_latches_int_raw_walk_identity() {
            // OVF int DISABLED (INT_ENA=0): INT_RAW must still latch on wrap
            // (sticky), byte-identical to the walk, but the level never asserts.
            let script = [(1u64, TIMER0_CONF_OFF, conf(4, 1))];
            let fired = assert_walk_identical(&script, 80, "disabled-int latch");
            assert_eq!(fired, 0, "no enabled int → the matrix level never asserts");
        }

        #[test]
        fn interval_64_batched_matches_walk() {
            // Both the walk reference and the scheduler drain quantise to the
            // batch grid at interval 64: advance both by 64-cycle jumps and
            // assert INT_RAW + counters agree at every batch boundary. period 16
            // div 1 → overflows resolve inside each batch, so the sticky latch
            // must match.
            let mut walk = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
            let clock = CycleClock::default();
            let mut sched = Esp32c3Ledc::new(LEDC_INTR_SOURCE_ID);
            sched.attach_cycle_clock(clock.clone());

            walk.write_u32(TIMER0_CONF_OFF, conf(4, 1)).unwrap();
            walk.write_u32(INT_ENA, 1).unwrap();
            clock.publish(0);
            sched.sync_to(0);
            sched.write_u32(TIMER0_CONF_OFF, conf(4, 1)).unwrap();
            sched.write_u32(INT_ENA, 1).unwrap();

            let mut any_ovf = false;
            for b in 1..=20u64 {
                let now = b * 64;
                walk.tick_elapsed(64);
                clock.publish(now);
                // A read syncs the scheduler counters to the batch boundary.
                assert_eq!(
                    walk.read_u32(TIMER0_VALUE_OFF).unwrap(),
                    sched.read_u32(TIMER0_VALUE_OFF).unwrap(),
                    "interval-64 counter diverged at cycle {now}"
                );
                assert_eq!(
                    walk.read_u32(INT_RAW).unwrap(),
                    sched.read_u32(INT_RAW).unwrap(),
                    "interval-64 INT_RAW diverged at cycle {now}"
                );
                if walk.read_u32(INT_RAW).unwrap() & 1 != 0 {
                    any_ovf = true;
                }
            }
            assert!(
                any_ovf,
                "non-vacuity: the timer must overflow at interval 64"
            );
        }

        #[test]
        fn on_event_delivers_overflow_without_polling() {
            // The headline event-path proof: with NO MMIO poll, the scheduled
            // event must latch LSTIMER0_OVF at the exact overflow cycle and
            // re-arm the successor. period 16 div 1 → OVF at cycle 16, 32, …
            let mut h = SchedHarness::new();
            h.write(TIMER0_CONF_OFF, conf(4, 1));
            h.write(INT_ENA, 1);

            for _ in 0..15 {
                h.step();
            }
            assert_eq!(
                h.event_int_raw() & 1,
                0,
                "no overflow before cycle 16 (event-materialised, no poll)"
            );
            h.step(); // cycle 16
            assert_eq!(
                h.event_int_raw() & 1,
                1,
                "the scheduled event latched LSTIMER0_OVF at cycle 16 with no MMIO poll"
            );

            // Advance to just before the next overflow, clear, and confirm the
            // re-armed chain latches the next wrap at cycle 32.
            for _ in 0..15 {
                h.step();
            }
            h.write(INT_CLR, 1); // now = 31
            assert_eq!(h.event_int_raw() & 1, 0, "INT_CLR cleared the latch");
            h.step(); // cycle 32
            assert_eq!(
                h.event_int_raw() & 1,
                1,
                "the event chain re-armed and latched the next overflow at cycle 32"
            );
        }

        #[test]
        fn stale_event_dies_after_reconfig() {
            let mut h = SchedHarness::new();
            h.write(TIMER0_CONF_OFF, conf(4, 1));
            let old_token = h.events.last().unwrap().1;
            // Reconfigure (period change): the fresh arm bumps the token, so the
            // in-flight event must die on arrival.
            h.write(TIMER0_CONF_OFF, conf(6, 1));
            let new_token = h.events.last().unwrap().1;
            assert_ne!(old_token, new_token, "reconfig must re-stamp the arm token");

            let res = h.ledc.on_event(old_token, &mut h.sched, &mut h.bus);
            assert!(
                res.reschedule_delay.is_none(),
                "a stale-token event must not raise or respawn"
            );
        }
    }
}
