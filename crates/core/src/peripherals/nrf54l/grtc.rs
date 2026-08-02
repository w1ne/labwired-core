// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF54L GRTC (Global Real-time Counter).
//!
//! Source: Nordic MDK `nrf54l15_types.h` (`NRF_GRTC_Type`, struct size
//! 0x760) for the register offsets and field masks, cross-checked against
//! nrfx `hal/nrf_grtc.h` + `drivers/src/nrfx_grtc.c` for the access
//! semantics. Instance counts come from the same MDK headers and agree with
//! the Zephyr devicetree (`dts/vendor/nordic/nrf54l_05_10_15.dtsi`,
//! `grtc@e2000` with `cc-num = <12>`).
//!
//! The GRTC replaces RTC0/RTC1 on this family. Its SYSCOUNTER is a single
//! free-running 52-bit counter (32-bit low word + 20-bit high word) clocked
//! at 1 MHz — nrfx states this explicitly as
//! `NRF_GRTC_SYSCOUNTER_MAIN_FREQUENCY_HZ 1000000UL`. It is exposed through
//! four per-domain register views (`SYSCOUNTER[0..3]`), all of which read
//! the same counter; the local CPU picks one via `NRF_GRTC_DOMAIN_INDEX`.
//! The model implements all four views identically so it does not depend on
//! which domain index a given build resolves to.
//!
//! Anti-tearing (PS behaviour, and the reason the register pair exists):
//! reading SYSCOUNTERL latches the current high word, and the following
//! SYSCOUNTERH read returns that latched value rather than a live one. A
//! 32-bit rollover between the two reads therefore cannot produce a torn
//! 52-bit value. This is modelled, not skipped.
//!
//! Counter enable: nrfx does **not** start the SYSCOUNTER with TASKS_START
//! (`nrfx_grtc.c` calls `nrfy_grtc_sys_counter_set()`, which writes
//! `MODE.SYSCOUNTEREN`; the `nrf_grtc_task_trigger` helpers even assert
//! against NRF_GRTC_TASK_START, which is reserved for the owner domain and
//! PPI). Both paths are honoured here: the counter runs when TASKS_START has
//! been triggered **or** when `MODE.SYSCOUNTEREN` is set, so neither a
//! bare-metal nor an nrfx/Zephyr start sequence can leave it frozen.
//!
//! EVENTS_* semantics: hardware-generated only. Writes of 1 are ignored;
//! only writes of 0 clear the event register, as everywhere else in the
//! Nordic fabric.
//!
//! Compare: `EVENTS_COMPARE[n]` is raised when the SYSCOUNTER reaches or
//! passes the 52-bit `CC[n]` value while `CC[n].CCEN.ACTIVE` is set. GRTC
//! compares are absolute deadlines, so the test is `>=` (a CC armed slightly
//! in the past fires immediately — nrfx relies on this). The IRQ and the PPI
//! event are emitted only on the event register's 0→1 transition, so a CC
//! left matching does not storm the NVIC every tick.
//!
//! ## Drive modes (walk-free plan Part 2 — idle fast-forward)
//!
//! Two mutually exclusive time sources, selected by ONE predicate
//! (`scheduler_mode`), exactly like the SysTick / ESP32-C3 RTC exemplars:
//!
//! * **Scheduler mode** (`event-scheduler` feature + a [`CycleClock`] attached
//!   at bus registration): `uses_scheduler()` is true, the per-cycle walk skips
//!   this peripheral, and
//!   - the SYSCOUNTER is **derived lazily** from the bus-published cycle clock
//!     (`advance_to`) — a `&self` read advances `Cell`-held state to "now", so
//!     Zephyr's `sys_clock_cycle_get` polling SYSCOUNTER observes fresh time
//!     without any walk;
//!   - each armed `CC[n]` compare is scheduled as a wake event at its exact
//!     SYSCOUNTER deadline. `on_event` claims the fire (pending the group IRQ
//!     GRTC_g = `irq_base + g`) and reschedules the next armed compare, so the
//!     kernel tick is delivered at its exact cycle with no cumulative drift —
//!     and idle fast-forward can skip the whole idle window straight to the
//!     next deadline (the FF budget clamps to `next_event_deadline`).
//!
//! * **Legacy mode** (feature off, or no clock attached — hand-built test
//!   buses): the per-cycle walk drives `tick()` and the counter advances
//!   eagerly, byte-identical to the historical model.
//!
//! The compare LATCH (`EVENTS_COMPARE[n]`, the CCEN.ACTIVE self-clear, and the
//! CC0 auto-reload) is materialised in `advance_to` in BOTH modes' shared
//! `advance_and_eval` core, so a mid-window `&self` read of EVENTS_COMPARE is
//! consistent with the freshly-derived counter. Only the IRQ *delivery* rides
//! the scheduled event (batch-boundary quantised at tick interval > 1, exact at
//! interval 1) — the same split SysTick uses for COUNTFLAG vs exception-15.
//!
//! ### Tick-cost normalization
//!
//! The legacy model charged the elapsed cycles into the peripheral tick-cost
//! channel on every advance while running, inflating `total_cycles` by an
//! amount the scheduler path (which never ticks) cannot reproduce — a sim
//! artifact (a free-running counter consumes zero core cycles). Both modes now
//! charge zero cost, so the walk-on reference and the scheduler path agree
//! cycle-for-cycle. The SYSCOUNTER value firmware reads is unchanged (it always
//! tracked the *input* cycles, never this returned cost).
//!
//! What is deliberately NOT modelled: the low-frequency timer
//! (RTCOUNTER/RTCOMPARE), the PWM block, CLKOUT generation, DPPI
//! SUBSCRIBE/PUBLISH routing, and the sleep/wake arbitration behind
//! TIMEOUT / WAKETIME / SYSCOUNTER.ACTIVE. Those registers accept writes and
//! read back so a driver's configure-then-verify pass never faults or hangs,
//! but they have no behaviour. STATUS.{LFTIMER,PWM,CLKOUT} read as READY and
//! SYSCOUNTERH.BUSY reads as 0, which is what a polling driver needs to make
//! progress.
//!
//! Unverified against silicon: the exact `SYSCOUNTERH.OVERFLOW` semantics.
//! The MDK documents the bit as "the SYSCOUNTERL overflow indication after
//! reading it" but does not define the window it covers. This model sets it
//! when the high word changed between the previous and the current
//! SYSCOUNTERL read. That is consistent with the documented intent and with
//! how `nrf_grtc_sys_counter_overflow_check()` is used, but it needs bench
//! confirmation on real hardware.

use std::cell::Cell;

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

// ── Register offsets (MDK `NRF_GRTC_Type`, nrf54l15_types.h) ─────────────────

const OFF_TASKS_CAPTURE0: u64 = 0x000;
const OFF_TASKS_CAPTURE_LAST: u64 = 0x02C; // TASKS_CAPTURE[11]
const OFF_TASKS_START: u64 = 0x060;
const OFF_TASKS_STOP: u64 = 0x064;
const OFF_TASKS_CLEAR: u64 = 0x068;
const OFF_TASKS_PWMSTART: u64 = 0x06C;
const OFF_TASKS_PWMSTOP: u64 = 0x070;
const OFF_SUBSCRIBE_CAPTURE0: u64 = 0x080;
const OFF_SUBSCRIBE_CAPTURE_LAST: u64 = 0x0AC;
const OFF_EVENTS_COMPARE0: u64 = 0x100;
const OFF_EVENTS_COMPARE_LAST: u64 = 0x12C; // EVENTS_COMPARE[11]
const OFF_PUBLISH_COMPARE0: u64 = 0x180;
const OFF_PUBLISH_COMPARE_LAST: u64 = 0x1AC;
const OFF_SHORTS: u64 = 0x200;
const OFF_INTEN0: u64 = 0x300;
const OFF_INTEN_LAST: u64 = 0x33C; // INTPEND3
const OFF_EVTEN: u64 = 0x400;
const OFF_EVTENSET: u64 = 0x404;
const OFF_EVTENCLR: u64 = 0x408;
const OFF_MODE: u64 = 0x510;
const OFF_CC0: u64 = 0x520;
const OFF_CC_LAST: u64 = 0x5AC; // CC[11].CCEN
const OFF_TIMEOUT: u64 = 0x6A4;
const OFF_INTERVAL: u64 = 0x6A8;
const OFF_WAKETIME: u64 = 0x6AC;
const OFF_STATUS_LFTIMER: u64 = 0x6B0;
const OFF_STATUS_PWM: u64 = 0x6B4;
const OFF_STATUS_CLKOUT: u64 = 0x6B8;
const OFF_PWMCONFIG: u64 = 0x710;
const OFF_CLKOUT: u64 = 0x714;
const OFF_CLKCFG: u64 = 0x718;
const OFF_SYSCOUNTER0: u64 = 0x720;
const OFF_SYSCOUNTER_LAST: u64 = 0x75C; // SYSCOUNTER[3] reserved word

/// Stride of one `GRTC_CC` sub-struct: CCL / CCH / CCADD / CCEN.
const CC_STRIDE: u64 = 0x10;
/// Stride of one `GRTC_SYSCOUNTER` sub-struct: L / H / ACTIVE / reserved.
const SYSCOUNTER_STRIDE: u64 = 0x10;
/// Stride of one interrupt group: INTEN / INTENSET / INTENCLR / INTPEND.
const INT_GROUP_STRIDE: u64 = 0x10;
/// `GRTC_GRTC_NINTERRUPTS_SIZE` — four independent interrupt groups.
const NUM_INT_GROUPS: usize = 4;
/// `GRTC_SYSCOUNTER_MaxCount` — four per-domain SYSCOUNTER register views.
const NUM_SYSCOUNTER_VIEWS: u64 = 4;
/// `GRTC_CC_MaxCount` — the hardware maximum this model can represent.
const MAX_CC: usize = 12;

// ── Field masks (MDK field defines) ──────────────────────────────────────────

/// `GRTC_SYSCOUNTER_SYSCOUNTERH_VALUE_Msk` — the counter's top 20 bits.
const SYSCOUNTERH_VALUE_MASK: u32 = 0x000F_FFFF;
/// `GRTC_SYSCOUNTER_SYSCOUNTERH_BUSY_Msk`.
const SYSCOUNTERH_BUSY_BIT: u32 = 1 << 30;
/// `GRTC_SYSCOUNTER_SYSCOUNTERH_OVERFLOW_Msk`.
const SYSCOUNTERH_OVERFLOW_BIT: u32 = 1 << 31;
/// The SYSCOUNTER is 52 bits wide: 32 low + 20 high.
const COUNTER_MASK: u64 = 0x000F_FFFF_FFFF_FFFF;
/// `GRTC_CC_CCH_CCH_Msk` — CC high word is 20 bits, same width as the counter.
const CCH_VALUE_MASK: u32 = 0x000F_FFFF;
/// `GRTC_CC_CCADD_VALUE_Msk` — the addend is 31 bits.
const CCADD_VALUE_MASK: u32 = 0x7FFF_FFFF;
/// `GRTC_CC_CCADD_REFERENCE_Msk` — 0 adds to SYSCOUNTER, 1 adds to CC.
const CCADD_REFERENCE_CC: u32 = 1 << 31;
/// `GRTC_CC_CCEN_ACTIVE_Msk`.
const CCEN_ACTIVE: u32 = 1 << 0;
/// `GRTC_MODE_SYSCOUNTEREN_Msk` — the enable nrfx actually uses.
const MODE_SYSCOUNTEREN: u32 = 1 << 1;
/// `GRTC_MODE_AUTOEN_Msk` | `GRTC_MODE_SYSCOUNTEREN_Msk`.
const MODE_WRITABLE_MASK: u32 = 0x3;
/// `GRTC_INTEN0_COMPARE0_Pos` is 0, so COMPARE[i] sits at bit i.
const INTEN_COMPARE_SHIFT: u32 = 0;
/// `GRTC_STATUS_*_READY_Ready` — every STATUS register resets to READY (1).
const STATUS_READY: u32 = 1;
/// `GRTC_CLKCFG_ResetValue` — CLKFASTDIV=1, CLKSEL=SystemLFCLK.
const CLKCFG_RESET_VALUE: u32 = 0x0001_0001;

// ── Clocking ────────────────────────────────────────────────────────────────

/// SYSCOUNTER frequency. nrfx `hal/nrf_grtc.h`:
/// `#define NRF_GRTC_SYSCOUNTER_MAIN_FREQUENCY_HZ 1000000UL`.
pub const SYSCOUNTER_HZ: u32 = 1_000_000;

/// nRF54L15 application-core clock, per the chip profile (128 MHz).
pub const CPU_HZ_DEFAULT: u32 = 128_000_000;

/// CPU cycles per SYSCOUNTER increment: 128_000_000 / 1_000_000 = 128 exactly,
/// so no fractional accumulator is needed to hit 1 MHz without drift.
///
/// Unit tests that call `tick()` directly use [`Nrf54lGrtc::new_fast`], which
/// sets this to 1 so small tick counts advance the counter.
pub const CYCLES_PER_SYSCOUNTER_TICK: u32 = CPU_HZ_DEFAULT / SYSCOUNTER_HZ;

/// Maximum CPU-cycle horizon a single scheduled compare wake may reach. A
/// GRTC compare can be armed at an effectively-unreachable deadline (Zephyr's
/// tickless idle programs the SYSCOUNTER compare at ~2^43 for "sleep until an
/// interrupt"). A scheduled event at that cycle would NEVER drain — the
/// scheduler has no cancel, so a superseded far wake is only reclaimed when it
/// fires — so each such arm/disarm would leak a permanent live event and trip
/// the per-peripheral residency ceiling. Capping the wake to this horizon and
/// re-evaluating in `on_event` (which fires the compare only once the counter
/// actually reaches it) bounds every event's deadline, so stale wakes drain
/// within one horizon. Chosen larger than a typical kernel-tick period (~1.3 M
/// cycles = 10 ms at 128 MHz) so normal ticks schedule at their exact deadline
/// and idle fast-forward skips the whole tick window in one hop; only the rare
/// unreachable idle compare is chunked.
const MAX_SCHEDULE_HORIZON: u64 = 1 << 21;

/// NVIC position of GRTC_0 on the nRF54L15 application core (MDK IRQn table:
/// GRTC_0..GRTC_3 = 226..229). The four INTEN groups drive four *independent*
/// interrupt lines: a compare enabled in INTEN group `g` asserts GRTC_`g` =
/// `GRTC_IRQ_BASE + g`. Zephyr/nrfx on the secure app core uses
/// `GRTC_IRQ_GROUP = 2` (nrf54l15 MDK interim header), so the kernel tick
/// compare is enabled via INTENSET2 and delivered on GRTC_2 = 228 — which is
/// exactly the first entry of the resolved devicetree `interrupts` property
/// and the line `DT_IRQN(GRTC_NODE)` connects. Routing the generic peripheral
/// IRQ (GRTC_0 = 226) instead left the tick permanently undelivered.
pub const GRTC_IRQ_BASE_DEFAULT: u32 = 226;

/// Nordic nRF54L GRTC — behavioural SYSCOUNTER + compare-channel model.
pub struct Nrf54lGrtc {
    /// Number of CC / EVENTS_COMPARE / TASKS_CAPTURE channels present.
    /// nRF54L15 has 12 (DT `cc-num`, `GRTC_CC_MaxCount`). Default: 12.
    num_cc: usize,

    /// The 52-bit free-running SYSCOUNTER. `Cell` so the scheduler-mode `&self`
    /// read path can lazily advance it to the bus-published clock. In legacy
    /// mode only `tick()` mutates it.
    counter: Cell<u64>,
    /// Set by TASKS_START, cleared by TASKS_STOP.
    task_started: bool,

    /// CPU-cycle accumulator feeding the 1 MHz SYSCOUNTER tick. `Cell` for the
    /// same lazy `&self` advance as `counter`.
    cycle_accum: Cell<u32>,
    /// CPU cycles per SYSCOUNTER increment; 1 in `new_fast()`.
    cycles_per_tick: u32,

    /// High word latched by the most recent SYSCOUNTERL read, and whether a
    /// SYSCOUNTERL read has happened at all. Behind `Cell` because the
    /// `Peripheral` read path takes `&self` and this latch is a genuine
    /// read side effect in hardware.
    high_latch: Cell<u32>,
    high_latched: Cell<bool>,
    /// High word observed at the previous SYSCOUNTERL read, used to derive
    /// the (unverified) SYSCOUNTERH.OVERFLOW indication.
    prev_read_high: Cell<u32>,
    overflow_latch: Cell<bool>,

    /// CC channel state. `Cell` per element so the lazy `&self` `advance_to`
    /// can materialise the compare latch (EVENTS_COMPARE), the CCEN.ACTIVE
    /// one-shot self-clear, and the CC0 auto-reload consistently with the
    /// freshly-derived counter.
    cc_l: [Cell<u32>; MAX_CC],
    cc_h: [Cell<u32>; MAX_CC],
    cc_en: [Cell<u32>; MAX_CC],
    events_compare: [Cell<u32>; MAX_CC],

    /// INTEN[0..3] — one enable word per interrupt group.
    inten: [u32; NUM_INT_GROUPS],

    /// NVIC position of GRTC_0. Interrupt group `g` pends `irq_base + g` (the
    /// four GRTC lines are independent). Taken from the chip profile's `irq`
    /// field so the model never hard-codes a family constant; defaults to
    /// [`GRTC_IRQ_BASE_DEFAULT`] (226) for the nRF54L15 app core.
    irq_base: u32,

    // Config/state registers with no modelled behaviour: written by drivers,
    // read back verbatim.
    shorts: u32,
    evten: u32,
    mode: u32,
    timeout: u32,
    interval: u32,
    waketime: u32,
    pwmconfig: u32,
    clkout: u32,
    clkcfg: u32,
    subscribe_capture: [u32; MAX_CC],
    publish_compare: [u32; MAX_CC],
    /// SYSCOUNTER[n].ACTIVE — the keep-awake request, stored per domain view.
    syscounter_active: [u32; NUM_SYSCOUNTER_VIEWS as usize],

    // ── Scheduler integration (walk-free plan) ─────────────────────────────
    /// Lazy-path anchor: the absolute published cycle the counter state was
    /// last advanced to. Owned by `advance_to` (scheduler mode); the legacy
    /// walk never touches it.
    anchor: Cell<u64>,
    /// Interrupt groups (`irq_base + g`) whose compare fired but whose IRQ has
    /// not yet been claimed by the event drain. `on_event` translates these
    /// into `explicit_irqs`; pends merge in the NVIC ISPR exactly as multiple
    /// per-cycle walk pends do.
    pending_irq_groups: Cell<u32>,
    /// Bitmap (bit `i` = channel `i`) of EVENTS_COMPARE that fired but whose
    /// PPI `fired_events` have not yet been claimed by the event drain.
    pending_fired: Cell<u32>,
    /// Arming-sequence token: bumped when the scheduled compare deadline
    /// changes so an in-flight event chain scheduled under an older
    /// configuration dies on arrival (token mismatch) instead of racing the
    /// fresh chain.
    arm_seq: u32,
    /// Absolute cycle of the currently-scheduled compare wake, or `None` when
    /// nothing is scheduled. A compare's absolute deadline is INVARIANT as the
    /// counter advances (`anchor + cycles_until` is constant for a fixed CC),
    /// so an MMIO write that does not change the next compare re-derives the
    /// same value and schedules NOTHING — without this the GRTC re-armed on
    /// every register write and piled far-future events past the scheduler's
    /// per-peripheral residency ceiling.
    scheduled_deadline: Cell<Option<u64>>,
    /// Bus-published cycle clock. `Some` once the bus registration choke
    /// attaches it; `None` keeps the model on the legacy walk path.
    clock: Option<CycleClock>,
}

impl std::fmt::Debug for Nrf54lGrtc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Nrf54lGrtc")
            .field("num_cc", &self.num_cc)
            .field("counter", &self.counter.get())
            .field("running", &self.running())
            .field("mode", &self.mode)
            .field("scheduler", &self.scheduler_mode())
            .finish()
    }
}

impl Default for Nrf54lGrtc {
    fn default() -> Self {
        Self {
            num_cc: MAX_CC,
            counter: Cell::new(0),
            task_started: false,
            cycle_accum: Cell::new(0),
            cycles_per_tick: CYCLES_PER_SYSCOUNTER_TICK,
            high_latch: Cell::new(0),
            high_latched: Cell::new(false),
            prev_read_high: Cell::new(0),
            overflow_latch: Cell::new(false),
            cc_l: std::array::from_fn(|_| Cell::new(0)),
            cc_h: std::array::from_fn(|_| Cell::new(0)),
            cc_en: std::array::from_fn(|_| Cell::new(0)),
            events_compare: std::array::from_fn(|_| Cell::new(0)),
            inten: [0u32; NUM_INT_GROUPS],
            irq_base: GRTC_IRQ_BASE_DEFAULT,
            shorts: 0,
            evten: 0,
            mode: 0,
            timeout: 0,
            interval: 0,
            waketime: 0,
            pwmconfig: 0,
            clkout: 0,
            clkcfg: CLKCFG_RESET_VALUE,
            subscribe_capture: [0u32; MAX_CC],
            publish_compare: [0u32; MAX_CC],
            syscounter_active: [0u32; NUM_SYSCOUNTER_VIEWS as usize],
            anchor: Cell::new(0),
            pending_irq_groups: Cell::new(0),
            pending_fired: Cell::new(0),
            arm_seq: 0,
            scheduled_deadline: Cell::new(None),
            clock: None,
        }
    }
}

impl Nrf54lGrtc {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with an explicit CC count. nRF54L15 uses `num_cc: 12`.
    pub fn new_with_cc(num_cc: usize) -> Self {
        Self {
            num_cc: num_cc.clamp(1, MAX_CC),
            ..Self::default()
        }
    }

    /// Construct with an explicit CC count and the NVIC position of GRTC_0
    /// (interrupt group `g` pends `irq_base + g`). The chip profile's `irq`
    /// field feeds `irq_base`; falls back to [`GRTC_IRQ_BASE_DEFAULT`] when the
    /// profile omits it.
    pub fn new_with_cc_and_irq(num_cc: usize, irq_base: u32) -> Self {
        Self {
            num_cc: num_cc.clamp(1, MAX_CC),
            irq_base,
            ..Self::default()
        }
    }

    /// Construct a "fast" GRTC whose SYSCOUNTER advances once per `tick()`
    /// instead of once per 128 CPU cycles. Intended for unit tests that call
    /// `tick()` directly and want small tick counts to move the counter.
    #[cfg(test)]
    pub fn new_fast() -> Self {
        Self {
            cycles_per_tick: 1,
            ..Self::default()
        }
    }

    /// Force the SYSCOUNTER to an arbitrary value. Test-only: there is no
    /// hardware task that presets the counter, so reaching a 32-bit rollover
    /// through `tick()` alone would take 2^32 calls.
    #[cfg(test)]
    fn set_counter(&mut self, value: u64) {
        self.counter.set(value & COUNTER_MASK);
    }

    /// True when the event scheduler owns this GRTC's time base (feature on AND
    /// bus clock attached). Everything time-related branches on this ONE
    /// predicate so the two drive modes can never mix.
    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to the
    /// legacy walk path (`uses_scheduler() == false`). Used by the walk-on-vs-
    /// scheduler differential gate to build the reference lane from the same
    /// bus assembly.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// The SYSCOUNTER runs when either enable path is asserted — see the
    /// module docs for why both are honoured.
    fn running(&self) -> bool {
        self.task_started || (self.mode & MODE_SYSCOUNTEREN != 0)
    }

    /// INTEN writable mask: COMPARE[0..num_cc) at bits 0..num_cc.
    fn inten_mask(&self) -> u32 {
        (((1u64 << self.num_cc) - 1) as u32) << INTEN_COMPARE_SHIFT
    }

    /// Bitmap of currently-latched compare events, in INTEN bit positions.
    fn event_bitmap(&self) -> u32 {
        let mut bits = 0u32;
        for i in 0..self.num_cc {
            if self.events_compare[i].get() != 0 {
                bits |= 1 << (INTEN_COMPARE_SHIFT + i as u32);
            }
        }
        bits
    }

    /// The 52-bit compare value of channel `i`.
    fn cc_value(&self, i: usize) -> u64 {
        ((self.cc_h[i].get() & CCH_VALUE_MASK) as u64) << 32 | self.cc_l[i].get() as u64
    }

    /// Write a 52-bit value into channel `i`'s CCL/CCH pair.
    fn set_cc_value(&self, i: usize, value: u64) {
        let value = value & COUNTER_MASK;
        self.cc_l[i].set(value as u32);
        self.cc_h[i].set(((value >> 32) as u32) & CCH_VALUE_MASK);
    }

    /// Read SYSCOUNTERL and latch the high word so the paired SYSCOUNTERH
    /// read cannot tear across a 32-bit rollover.
    fn read_syscounter_low(&self) -> u32 {
        let counter = self.counter.get();
        let high = ((counter >> 32) as u32) & SYSCOUNTERH_VALUE_MASK;
        // OVERFLOW reflects a low-word rollover since the previous read.
        // Semantics unconfirmed on silicon — see the module docs.
        let overflow = self.high_latched.get() && high != self.prev_read_high.get();
        self.overflow_latch.set(overflow);
        self.prev_read_high.set(high);
        self.high_latch.set(high);
        self.high_latched.set(true);
        counter as u32
    }

    /// Read SYSCOUNTERH: the value latched by the last SYSCOUNTERL read if
    /// there was one, otherwise a live sample. BUSY always reads 0 — the
    /// model's counter is always coherent, and a driver spinning on BUSY
    /// must be able to make progress.
    fn read_syscounter_high(&self) -> u32 {
        let value = if self.high_latched.get() {
            self.high_latch.get()
        } else {
            ((self.counter.get() >> 32) as u32) & SYSCOUNTERH_VALUE_MASK
        };
        let overflow = if self.overflow_latch.get() {
            SYSCOUNTERH_OVERFLOW_BIT
        } else {
            0
        };
        // BUSY is explicitly masked out rather than merely omitted: a driver
        // spinning on it must always see a readable counter.
        ((value & SYSCOUNTERH_VALUE_MASK) | overflow) & !SYSCOUNTERH_BUSY_BIT
    }

    /// Shared closed-form advance used by BOTH drive modes: consume `cycles`
    /// CPU cycles, advance the 1 MHz SYSCOUNTER, and evaluate every armed
    /// compare against the new value. Mutates only `Cell`-held state (callable
    /// from `&self`), so the lazy read path replays the walk EXACTLY.
    ///
    /// Returns `(pended_groups, fired_channels)` for THIS call:
    ///   * `pended_groups` — bitmap over interrupt groups `g` whose enabled
    ///     compare fired (→ GRTC_`g` = `irq_base + g`);
    ///   * `fired_channels` — bitmap over channels whose EVENTS_COMPARE went
    ///     0→1 (→ PPI `fired_events`).
    ///
    /// The legacy `tick()` delivers these immediately; `advance_to` accumulates
    /// them into the pending `Cell`s for the next event drain to claim.
    fn advance_and_eval(&self, cycles: u64) -> (u32, u32) {
        if !self.running() {
            return (0, 0);
        }
        let total = self.cycle_accum.get() as u64 + cycles;
        let cpt = self.cycles_per_tick as u64;
        let increments = total / cpt;
        self.cycle_accum.set((total % cpt) as u32);
        if increments == 0 {
            return (0, 0);
        }
        let new = self.counter.get().wrapping_add(increments) & COUNTER_MASK;
        self.counter.set(new);

        // Each INTEN group drives an independent GRTC line (GRTC_g =
        // irq_base + g), so a fired compare pends the line of *every* group
        // whose enable bit is set — not one collapsed generic IRQ. nrfx on the
        // secure app core enables the kernel-tick compare in group 2, so the
        // tick is delivered on GRTC_2; collapsing all groups onto GRTC_0 left
        // it undelivered. Accumulate per group.
        let mut pended_groups = 0u32;
        let mut fired = 0u32;
        for i in 0..self.num_cc {
            if self.cc_en[i].get() & CCEN_ACTIVE == 0 {
                continue;
            }
            // GRTC compares are absolute deadlines, so this is `>=`, and the
            // IRQ is emitted only on the event's 0→1 edge.
            if new >= self.cc_value(i) && self.events_compare[i].get() == 0 {
                self.events_compare[i].set(1);
                fired |= 1 << i;
                let bit = 1u32 << (INTEN_COMPARE_SHIFT + i as u32);
                for (g, group) in self.inten.iter().enumerate() {
                    if group & bit != 0 {
                        pended_groups |= 1 << g;
                    }
                }
                // A fired compare is one-shot: the hardware clears CCEN.ACTIVE
                // so the channel does not re-fire the instant firmware clears
                // EVENTS_COMPARE (which would double-tick the kernel). The sole
                // exception is CC0 running as an auto-reload interval timer,
                // where the deadline advances by INTERVAL and the channel stays
                // armed. Both match Nordic's `nhw_GRTC_compare_reached`.
                if i == 0 && self.interval != 0 {
                    let next = self.cc_value(0).wrapping_add(self.interval as u64);
                    self.set_cc_value(0, next);
                } else {
                    self.cc_en[i].set(self.cc_en[i].get() & !CCEN_ACTIVE);
                }
            }
        }
        (pended_groups, fired)
    }

    /// Lazy advance to absolute published cycle `now` — callable from `&self`
    /// (all mutated state is in `Cell`). Idempotent; a `now` older than the
    /// anchor is ignored (the clock is monotonic within a run). The advanced
    /// window always has constant MODE/CCx/INTEN/INTERVAL: every MMIO write
    /// syncs first (bus `sync_to` choke), so settings changes never straddle a
    /// window. Fired compares are accumulated into the pending `Cell`s for the
    /// event drain.
    fn advance_to(&self, now: u64) {
        let anchor = self.anchor.get();
        if now <= anchor {
            return;
        }
        self.anchor.set(now);
        let (groups, fired) = self.advance_and_eval(now - anchor);
        if groups != 0 {
            self.pending_irq_groups
                .set(self.pending_irq_groups.get() | groups);
        }
        if fired != 0 {
            self.pending_fired.set(self.pending_fired.get() | fired);
        }
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

    /// CPU cycles from the CURRENT (just-synced) state until the next armed,
    /// un-fired compare first fires, or `None` when nothing is armed. A compare
    /// whose deadline already lies at/behind the counter fires on the next
    /// SYSCOUNTER increment (the walk only evaluates on an increment), so its
    /// distance is the cycles to that increment.
    fn cycles_until_next_compare(&self) -> Option<u64> {
        if !self.running() {
            return None;
        }
        let cur = self.counter.get();
        let cpt = self.cycles_per_tick as u64;
        let accum = self.cycle_accum.get() as u64;
        let mut best: Option<u64> = None;
        for i in 0..self.num_cc {
            if self.cc_en[i].get() & CCEN_ACTIVE == 0 || self.events_compare[i].get() != 0 {
                continue;
            }
            let cc = self.cc_value(i);
            // Increments needed for `new >= cc`: a deadline at/behind the
            // counter fires on the very next increment (1); otherwise exactly
            // `cc - cur` increments reach it.
            let increments = if cc > cur { cc - cur } else { 1 };
            // Cycles to the `increments`-th SYSCOUNTER tick from the current
            // fractional phase: `increments * cpt - accum` (always >= 1 since
            // accum < cpt).
            let cycles = increments.saturating_mul(cpt).saturating_sub(accum);
            best = Some(best.map_or(cycles, |b| b.min(cycles)));
        }
        best
    }

    /// Absolute cycle at which the next armed compare fires, or `None`. Because
    /// the counter is synced to `anchor` before this is read (write/on_event
    /// choke), `anchor + cycles_until` is the fixed absolute deadline — the
    /// value re-writes must agree on to avoid rescheduling redundantly.
    fn next_compare_abs_deadline(&self) -> Option<u64> {
        self.cycles_until_next_compare()
            .map(|d| self.anchor.get() + d)
    }
}

impl Peripheral for Nrf54lGrtc {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        // Scheduler mode: advance the lazy counter (and materialise any compare
        // it crossed) to the published "now" first, so a polled SYSCOUNTER /
        // EVENTS_COMPARE read observes fresh, self-consistent time.
        self.sync_from_clock();
        Ok(match offset {
            // Tasks are write-only, read-as-zero.
            OFF_TASKS_CAPTURE0..=OFF_TASKS_CAPTURE_LAST => 0,
            OFF_TASKS_START | OFF_TASKS_STOP | OFF_TASKS_CLEAR | OFF_TASKS_PWMSTART
            | OFF_TASKS_PWMSTOP => 0,

            OFF_SUBSCRIBE_CAPTURE0..=OFF_SUBSCRIBE_CAPTURE_LAST if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_SUBSCRIBE_CAPTURE0) / 4) as usize;
                if i < self.num_cc {
                    self.subscribe_capture[i]
                } else {
                    0
                }
            }

            // EVENTS_COMPARE[i]: absent channels read 0.
            OFF_EVENTS_COMPARE0..=OFF_EVENTS_COMPARE_LAST if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_EVENTS_COMPARE0) / 4) as usize;
                if i < self.num_cc {
                    self.events_compare[i].get()
                } else {
                    0
                }
            }

            OFF_PUBLISH_COMPARE0..=OFF_PUBLISH_COMPARE_LAST if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_PUBLISH_COMPARE0) / 4) as usize;
                if i < self.num_cc {
                    self.publish_compare[i]
                } else {
                    0
                }
            }

            OFF_SHORTS => self.shorts,

            // INTEN / INTENSET / INTENCLR read back the enable word; INTPEND
            // is the enabled subset of the latched compare events.
            OFF_INTEN0..=OFF_INTEN_LAST if offset.is_multiple_of(4) => {
                let group = ((offset - OFF_INTEN0) / INT_GROUP_STRIDE) as usize;
                let reg = (offset - OFF_INTEN0) % INT_GROUP_STRIDE;
                let inten = self.inten[group] & self.inten_mask();
                match reg {
                    0x0 | 0x4 | 0x8 => inten,
                    _ => inten & self.event_bitmap(), // INTPEND
                }
            }

            OFF_EVTEN | OFF_EVTENSET | OFF_EVTENCLR => self.evten,
            OFF_MODE => self.mode & MODE_WRITABLE_MASK,

            // CC[i].{CCL,CCH,CCADD,CCEN}. CCADD is write-only (MDK marks it
            // `__OM`, and `GRTC_CCADD_WRITE_ONLY` is set for this part).
            OFF_CC0..=OFF_CC_LAST if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_CC0) / CC_STRIDE) as usize;
                let reg = (offset - OFF_CC0) % CC_STRIDE;
                if i >= self.num_cc {
                    0
                } else {
                    match reg {
                        0x0 => self.cc_l[i].get(),
                        0x4 => self.cc_h[i].get() & CCH_VALUE_MASK,
                        0x8 => 0, // CCADD, write-only
                        _ => self.cc_en[i].get() & CCEN_ACTIVE,
                    }
                }
            }

            OFF_TIMEOUT => self.timeout,
            OFF_INTERVAL => self.interval,
            OFF_WAKETIME => self.waketime,
            // Nothing in the model is ever busy, and every STATUS register
            // resets to READY, so report READY unconditionally.
            OFF_STATUS_LFTIMER | OFF_STATUS_PWM | OFF_STATUS_CLKOUT => STATUS_READY,
            OFF_PWMCONFIG => self.pwmconfig,
            OFF_CLKOUT => self.clkout,
            OFF_CLKCFG => self.clkcfg,

            // SYSCOUNTER[n].{L,H,ACTIVE}: all four domain views alias the
            // same counter.
            OFF_SYSCOUNTER0..=OFF_SYSCOUNTER_LAST if offset.is_multiple_of(4) => {
                let view = ((offset - OFF_SYSCOUNTER0) / SYSCOUNTER_STRIDE) as usize;
                let reg = (offset - OFF_SYSCOUNTER0) % SYSCOUNTER_STRIDE;
                match reg {
                    0x0 => self.read_syscounter_low(),
                    0x4 => self.read_syscounter_high(),
                    0x8 => self.syscounter_active[view],
                    _ => 0, // reserved word
                }
            }

            // Everything else in the 4 KB window reads as zero rather than
            // faulting the bus.
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // TASKS_CAPTURE[i]: snapshot the SYSCOUNTER into CC[i].
            OFF_TASKS_CAPTURE0..=OFF_TASKS_CAPTURE_LAST
                if offset.is_multiple_of(4) && value & 1 != 0 =>
            {
                let i = ((offset - OFF_TASKS_CAPTURE0) / 4) as usize;
                if i < self.num_cc {
                    let now = self.counter.get();
                    self.set_cc_value(i, now);
                    // Triggering the capture task disables the compare feature
                    // on that channel (Nordic `NHW_GRTC` CAPTURE side effect).
                    self.cc_en[i].set(self.cc_en[i].get() & !CCEN_ACTIVE);
                }
            }
            OFF_TASKS_START if value & 1 != 0 => self.task_started = true,
            OFF_TASKS_STOP if value & 1 != 0 => self.task_started = false,
            OFF_TASKS_CLEAR if value & 1 != 0 => {
                self.counter.set(0);
                self.cycle_accum.set(0);
                self.high_latched.set(false);
                self.overflow_latch.set(false);
                self.prev_read_high.set(0);
            }
            OFF_TASKS_PWMSTART | OFF_TASKS_PWMSTOP => {}

            OFF_SUBSCRIBE_CAPTURE0..=OFF_SUBSCRIBE_CAPTURE_LAST if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_SUBSCRIBE_CAPTURE0) / 4) as usize;
                if i < self.num_cc {
                    self.subscribe_capture[i] = value;
                }
            }

            // EVENTS_COMPARE[i]: hardware-generated; write-1 is ignored,
            // write-0 clears. Clearing the event does NOT retract an IRQ edge
            // already latched into `pending_irq_groups` — silicon keeps a pend
            // the NVIC already saw.
            OFF_EVENTS_COMPARE0..=OFF_EVENTS_COMPARE_LAST if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_EVENTS_COMPARE0) / 4) as usize;
                if i < self.num_cc && value == 0 {
                    self.events_compare[i].set(0);
                }
            }

            OFF_PUBLISH_COMPARE0..=OFF_PUBLISH_COMPARE_LAST if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_PUBLISH_COMPARE0) / 4) as usize;
                if i < self.num_cc {
                    self.publish_compare[i] = value;
                }
            }

            OFF_SHORTS => self.shorts = value,

            OFF_INTEN0..=OFF_INTEN_LAST if offset.is_multiple_of(4) => {
                let group = ((offset - OFF_INTEN0) / INT_GROUP_STRIDE) as usize;
                let reg = (offset - OFF_INTEN0) % INT_GROUP_STRIDE;
                let mask = self.inten_mask();
                match reg {
                    0x0 => self.inten[group] = value & mask,
                    0x4 => self.inten[group] |= value & mask,
                    0x8 => self.inten[group] &= !value,
                    _ => {} // INTPEND is read-only
                }
            }

            OFF_EVTEN => self.evten = value,
            OFF_EVTENSET => self.evten |= value,
            OFF_EVTENCLR => self.evten &= !value,
            OFF_MODE => self.mode = value & MODE_WRITABLE_MASK,

            OFF_CC0..=OFF_CC_LAST if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_CC0) / CC_STRIDE) as usize;
                let reg = (offset - OFF_CC0) % CC_STRIDE;
                if i < self.num_cc {
                    match reg {
                        // Arming is a WRITE SIDE EFFECT of CCL/CCH/CCADD, not a
                        // separate CCEN write — this is how the GRTC actually
                        // arms, and the reason nrfx/Zephyr's `nrf_grtc_sys_-
                        // counter_cc_set` (CCL then CCH, never CCEN) works.
                        // Verified against Nordic's own HW model
                        // (`bsim_hw_models/.../NHW_GRTC.c`
                        // `nhw_GRTC_regw_sideeffects_CC_{CCL,CCH,CCADD}`):
                        //   * writing CCL DISABLES the channel (CCEN.ACTIVE←0),
                        //   * writing CCH ENABLES it (CCEN.ACTIVE←1),
                        //   * writing CCADD ENABLES it.
                        // The CCL-then-CCH order leaves the channel armed with
                        // the freshly written 52-bit value.
                        0x0 => {
                            self.cc_l[i].set(value);
                            self.cc_en[i].set(self.cc_en[i].get() & !CCEN_ACTIVE);
                        }
                        0x4 => {
                            self.cc_h[i].set(value & CCH_VALUE_MASK);
                            self.cc_en[i].set(self.cc_en[i].get() | CCEN_ACTIVE);
                        }
                        // CCADD: add VALUE to either the live SYSCOUNTER
                        // (REFERENCE=SYSCOUNTER) or the current CC
                        // (REFERENCE=CC), store the result in CC[i], and arm.
                        0x8 => {
                            let base = if value & CCADD_REFERENCE_CC != 0 {
                                self.cc_value(i)
                            } else {
                                self.counter.get()
                            };
                            let sum = base.wrapping_add((value & CCADD_VALUE_MASK) as u64);
                            self.set_cc_value(i, sum);
                            self.cc_en[i].set(self.cc_en[i].get() | CCEN_ACTIVE);
                        }
                        // Direct CCEN write still honoured (the disable in
                        // nrfx's cc_channel_prepare and any bare-metal arm).
                        _ => self.cc_en[i].set(value & CCEN_ACTIVE),
                    }
                }
            }

            OFF_TIMEOUT => self.timeout = value,
            OFF_INTERVAL => self.interval = value,
            OFF_WAKETIME => self.waketime = value,
            // STATUS registers are status-only in this model.
            OFF_STATUS_LFTIMER | OFF_STATUS_PWM | OFF_STATUS_CLKOUT => {}
            OFF_PWMCONFIG => self.pwmconfig = value,
            OFF_CLKOUT => self.clkout = value,
            OFF_CLKCFG => self.clkcfg = value,

            OFF_SYSCOUNTER0..=OFF_SYSCOUNTER_LAST if offset.is_multiple_of(4) => {
                let view = ((offset - OFF_SYSCOUNTER0) / SYSCOUNTER_STRIDE) as usize;
                let reg = (offset - OFF_SYSCOUNTER0) % SYSCOUNTER_STRIDE;
                // SYSCOUNTERL/H are read-only; only ACTIVE is writable.
                if reg == 0x8 {
                    self.syscounter_active[view] = value;
                }
            }

            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.legacy_advance(1)
    }

    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.legacy_advance(cycles)
    }

    fn uses_scheduler(&self) -> bool {
        // True once the bus attached its cycle clock (event-scheduler builds):
        // reads stay fresh through the lazy `advance_to` path and compare IRQs
        // ride scheduled events, so the walk is unnecessary AND idle
        // fast-forward can skip idle windows to the next compare deadline.
        // Without a clock (feature off / hand-built buses) stay on the legacy
        // walk with exact historical semantics.
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        // In scheduler mode everything the walk did (SYSCOUNTER advance +
        // compare latch/IRQ) is event-expressible, so the walk is deletable.
        // In legacy mode the walk does real work and the conservative `true`
        // stands.
        !self.scheduler_mode()
    }

    fn sync_to(&mut self, now_cycle: u64) {
        if self.scheduler_mode() {
            self.advance_to(now_cycle);
        }
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        if self.pending_irq_groups.get() != 0 || self.pending_fired.get() != 0 {
            // A compare already materialised (a read synced past its deadline
            // before this write): deliver it at the next drain. Rare (needs a
            // read between the deadline and the drain), so a fresh token here
            // does not pile up.
            self.arm_seq = self.arm_seq.wrapping_add(1);
            return vec![(0, self.arm_seq)];
        }
        // Reschedule ONLY when the next compare's absolute deadline changed
        // (a CC arm/disarm, a MODE/START toggle, a TASKS_CLEAR, a reload). A
        // write that leaves the next compare where it was re-derives the same
        // absolute deadline and schedules nothing — the existing live event
        // still fires at the right cycle, and the residency ceiling holds.
        let abs = self.next_compare_abs_deadline();
        if abs == self.scheduled_deadline.get() {
            return Vec::new();
        }
        // The deadline moved: invalidate the old chain (token bump → it dies on
        // arrival) and record the new TRUE deadline for the next write's dedup.
        self.arm_seq = self.arm_seq.wrapping_add(1);
        self.scheduled_deadline.set(abs);
        // `collect_scheduled_events` converts to `current_cycle + 1 + delay`;
        // the fire lands `d` CPU cycles after the just-synced state, i.e. at
        // absolute cycle `current_cycle + d` — hence `d - 1` (d >= 1 always).
        // The wake itself is capped to the horizon (a far compare re-evaluates
        // in `on_event`); the dedup key above stays the TRUE deadline.
        self.cycles_until_next_compare()
            .map(|d| vec![(d.min(MAX_SCHEDULE_HORIZON).saturating_sub(1), self.arm_seq)])
            .unwrap_or_default()
    }

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
        // Bring the lazy counter up to the drain cycle; this materialises the
        // compare(s) this event was scheduled for.
        self.advance_to(sched.now());
        let groups = self.pending_irq_groups.replace(0);
        let fired = self.pending_fired.replace(0);

        // Each set group bit `g` pends its own GRTC line (`irq_base + g`),
        // routed through the same `pend_irq_for_event` choke the legacy walk's
        // `explicit_irqs` use.
        let explicit_irqs = (0..NUM_INT_GROUPS as u32)
            .filter(|g| groups & (1 << g) != 0)
            .map(|g| self.irq_base + g)
            .collect();
        // PPI: one `fired_events` offset per channel whose EVENTS_COMPARE went
        // 0→1, exactly as the walk returned them.
        let fired_events = (0..self.num_cc)
            .filter(|&i| fired & (1 << i) != 0)
            .map(|i| OFF_EVENTS_COMPARE0 as u32 + 4 * i as u32)
            .collect();

        // Perpetuate the chain from the just-synced state: the delay is
        // measured from `sched.now()`, so the next compare lands at its exact
        // absolute cycle — no cumulative drift at any interval. Record its TRUE
        // absolute deadline (for the write-path dedup) but wake at the capped
        // horizon: a compare farther than the horizon re-evaluates here without
        // firing until the counter actually reaches it, so no unreachable wake
        // ever lingers.
        let reschedule = self.cycles_until_next_compare();
        self.scheduled_deadline
            .set(reschedule.map(|d| sched.now() + d));
        crate::sched::EventResult {
            explicit_irqs,
            fired_events,
            reschedule_delay: reschedule.map(|d| d.min(MAX_SCHEDULE_HORIZON)),
            ..Default::default()
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

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    // NOTE: `mmio_access_class` is deliberately left at the default
    // `SideEffecting`. SYSCOUNTERL looks like a free-running timer poll, but
    // reading it latches SYSCOUNTERH — coalescing those reads would drop the
    // latch and reintroduce exactly the tearing this model exists to prevent.
}

impl Nrf54lGrtc {
    /// Legacy per-cycle walk advance. Never runs in scheduler mode (the walk
    /// skips `uses_scheduler()` peripherals; the guard keeps a stray direct
    /// call from corrupting the lazily-anchored state). Charges ZERO tick cost
    /// so `total_cycles` is byte-identical to the scheduler path (see the
    /// tick-cost normalization note in the module docs).
    fn legacy_advance(&mut self, cycles: u64) -> PeripheralTickResult {
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        let (groups, fired) = self.advance_and_eval(cycles);
        let explicit_irqs = if groups != 0 {
            Some(
                (0..NUM_INT_GROUPS as u32)
                    .filter(|g| groups & (1 << g) != 0)
                    .map(|g| self.irq_base + g)
                    .collect(),
            )
        } else {
            None
        };
        let fired_events = (0..self.num_cc)
            .filter(|&i| fired & (1 << i) != 0)
            .map(|i| OFF_EVENTS_COMPARE0 as u32 + 4 * i as u32)
            .collect();
        PeripheralTickResult {
            explicit_irqs,
            fired_events,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SYSCOUNTER[0].SYSCOUNTERL / .SYSCOUNTERH.
    const SYSCOUNTERL: u64 = OFF_SYSCOUNTER0;
    const SYSCOUNTERH: u64 = OFF_SYSCOUNTER0 + 4;

    fn cc_reg(i: u64, reg: u64) -> u64 {
        OFF_CC0 + i * CC_STRIDE + reg
    }

    #[test]
    fn syscounter_starts_at_zero_and_needs_a_start() {
        let mut g = Nrf54lGrtc::new_fast();
        assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 0);
        for _ in 0..10 {
            g.tick();
        }
        assert_eq!(
            g.read_u32(SYSCOUNTERL).unwrap(),
            0,
            "SYSCOUNTER must not advance before it is started"
        );
    }

    #[test]
    fn syscounter_advances_after_tasks_start_and_freezes_on_stop() {
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..10 {
            g.tick();
        }
        assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 10);

        g.write_u32(OFF_TASKS_STOP, 1).unwrap();
        for _ in 0..10 {
            g.tick();
        }
        assert_eq!(
            g.read_u32(SYSCOUNTERL).unwrap(),
            10,
            "SYSCOUNTER must freeze while stopped"
        );
    }

    #[test]
    fn mode_syscounteren_also_starts_the_counter() {
        // nrfx starts the SYSCOUNTER through MODE, never TASKS_START.
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
        for _ in 0..5 {
            g.tick();
        }
        assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 5);
        assert_eq!(g.read_u32(OFF_MODE).unwrap(), MODE_SYSCOUNTEREN);
    }

    #[test]
    fn tasks_clear_zeroes_the_counter() {
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..7 {
            g.tick();
        }
        assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 7);
        g.write_u32(OFF_TASKS_CLEAR, 1).unwrap();
        assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 0);
    }

    #[test]
    fn low_read_latches_high_across_a_32_bit_rollover() {
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(OFF_TASKS_START, 1).unwrap();
        // One tick short of the 32-bit boundary, high word still 0.
        g.set_counter(0xFFFF_FFFF);

        let low = g.read_u32(SYSCOUNTERL).unwrap();
        assert_eq!(low, 0xFFFF_FFFF);

        // The counter rolls into the high word between the paired reads.
        g.tick();
        assert_eq!(g.counter.get(), 0x1_0000_0000);

        let high = g.read_u32(SYSCOUNTERH).unwrap() & SYSCOUNTERH_VALUE_MASK;
        assert_eq!(
            high, 0,
            "SYSCOUNTERH must return the value latched by the SYSCOUNTERL \
             read, otherwise the 52-bit value tears to 0x1_FFFF_FFFF"
        );

        // A fresh pair sees the new high word.
        assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 0);
        assert_eq!(g.read_u32(SYSCOUNTERH).unwrap() & SYSCOUNTERH_VALUE_MASK, 1);
    }

    #[test]
    fn syscounter_high_reports_overflow_after_a_rollover() {
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(OFF_TASKS_START, 1).unwrap();
        g.set_counter(0xFFFF_FFFF);

        g.read_u32(SYSCOUNTERL).unwrap();
        assert_eq!(
            g.read_u32(SYSCOUNTERH).unwrap() & SYSCOUNTERH_OVERFLOW_BIT,
            0
        );

        g.tick(); // rolls into the high word
        g.read_u32(SYSCOUNTERL).unwrap();
        assert_ne!(
            g.read_u32(SYSCOUNTERH).unwrap() & SYSCOUNTERH_OVERFLOW_BIT,
            0,
            "OVERFLOW must flag a low-word rollover between reads"
        );
    }

    #[test]
    fn syscounter_high_never_reports_busy() {
        let g = Nrf54lGrtc::new();
        assert_eq!(g.read_u32(SYSCOUNTERH).unwrap() & SYSCOUNTERH_BUSY_BIT, 0);
    }

    #[test]
    fn all_four_domain_views_alias_one_counter() {
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..4 {
            g.tick();
        }
        for view in 0..NUM_SYSCOUNTER_VIEWS {
            let off = OFF_SYSCOUNTER0 + view * SYSCOUNTER_STRIDE;
            assert_eq!(g.read_u32(off).unwrap(), 4, "SYSCOUNTER[{view}] view");
        }
    }

    #[test]
    fn compare_fires_event_and_intpend_when_enabled() {
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(cc_reg(0, 0x0), 5).unwrap(); // CC[0].CCL = 5
        g.write_u32(cc_reg(0, 0xC), CCEN_ACTIVE).unwrap(); // CC[0].CCEN
        g.write_u32(OFF_INTEN0 + 0x4, 1 << 0).unwrap(); // INTENSET0.COMPARE0
        g.write_u32(OFF_TASKS_START, 1).unwrap();

        let mut irqs = 0;
        for _ in 0..12 {
            if let Some(lines) = g.tick().explicit_irqs {
                // Group 0's compare pends GRTC_0 = irq_base (226 by default).
                assert_eq!(lines, vec![GRTC_IRQ_BASE_DEFAULT]);
                irqs += 1;
            }
        }
        assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0).unwrap(), 1);
        assert_eq!(irqs, 1, "IRQ must be raised once, on the event's 0→1 edge");
        assert_eq!(
            g.read_u32(OFF_INTEN0 + 0xC).unwrap(),
            1 << 0,
            "INTPEND0 must show the enabled, latched compare"
        );
    }

    #[test]
    fn compare_pends_the_group_that_enabled_it() {
        // nrfx on the secure app core enables the kernel-tick compare in INTEN
        // group 2 and expects it on GRTC_2 = irq_base + 2. A compare enabled in
        // group g must pend exactly line irq_base + g — not a collapsed GRTC_0.
        let mut g = Nrf54lGrtc::new_with_cc_and_irq(12, GRTC_IRQ_BASE_DEFAULT);
        // Arm CC[6] the way nrfx does: CCL then CCH (the CCH write arms it).
        g.write_u32(cc_reg(6, 0x0), 5).unwrap();
        g.write_u32(cc_reg(6, 0x4), 0).unwrap();
        // INTENSET2.COMPARE6.
        g.write_u32(OFF_INTEN0 + 2 * INT_GROUP_STRIDE + 0x4, 1 << 6)
            .unwrap();
        g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();

        let mut pended = None;
        for _ in 0..(6 * CYCLES_PER_SYSCOUNTER_TICK) {
            if let Some(lines) = g.tick().explicit_irqs {
                pended = Some(lines);
                break;
            }
        }
        assert_eq!(
            pended,
            Some(vec![GRTC_IRQ_BASE_DEFAULT + 2]),
            "a group-2 compare must pend GRTC_2 = irq_base + 2"
        );
    }

    #[test]
    fn writing_cch_arms_and_writing_ccl_disarms() {
        // The arm bit is a write side effect of CCL/CCH, exactly how nrfx's
        // `nrf_grtc_sys_counter_cc_set` (CCL then CCH, never CCEN) arms the
        // system-timer compare.
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(cc_reg(3, 0x0), 4).unwrap(); // CCL → disarmed
        assert_eq!(g.read_u32(cc_reg(3, 0xC)).unwrap(), 0, "CCL write disarms");
        g.write_u32(cc_reg(3, 0x4), 0).unwrap(); // CCH → armed
        assert_eq!(
            g.read_u32(cc_reg(3, 0xC)).unwrap(),
            CCEN_ACTIVE,
            "CCH write arms the channel"
        );
        g.write_u32(OFF_INTEN0 + 0x4, 1 << 3).unwrap();
        g.write_u32(OFF_TASKS_START, 1).unwrap();
        let mut fired = false;
        for _ in 0..8 {
            if g.tick().explicit_irqs.is_some() {
                fired = true;
            }
        }
        assert!(fired, "a CCH-armed compare must fire");
    }

    #[test]
    fn fired_compare_is_one_shot_and_does_not_refire_after_event_clear() {
        // After a compare fires the hardware clears CCEN.ACTIVE, so clearing
        // EVENTS_COMPARE (as the ISR does) must NOT immediately re-fire — that
        // would double-tick the kernel.
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(cc_reg(0, 0x0), 3).unwrap();
        g.write_u32(cc_reg(0, 0x4), 0).unwrap(); // arm CC[0] at 3
        g.write_u32(OFF_INTEN0 + 0x4, 1 << 0).unwrap();
        g.write_u32(OFF_TASKS_START, 1).unwrap();

        let mut irqs = 0;
        for _ in 0..5 {
            if g.tick().explicit_irqs.is_some() {
                irqs += 1;
            }
        }
        assert_eq!(irqs, 1);
        assert_eq!(
            g.read_u32(cc_reg(0, 0xC)).unwrap(),
            0,
            "CCEN.ACTIVE must self-clear on fire (one-shot)"
        );
        // ISR clears the event; the counter is still past the stale CC.
        g.write_u32(OFF_EVENTS_COMPARE0, 0).unwrap();
        for _ in 0..5 {
            assert!(
                g.tick().explicit_irqs.is_none(),
                "a one-shot compare must not re-fire after event clear"
            );
        }
    }

    #[test]
    fn compare_does_not_interrupt_when_disabled() {
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(cc_reg(0, 0x0), 5).unwrap();
        g.write_u32(cc_reg(0, 0xC), CCEN_ACTIVE).unwrap();
        // INTEN left clear.
        g.write_u32(OFF_TASKS_START, 1).unwrap();

        let mut irqs = 0;
        for _ in 0..12 {
            if g.tick().explicit_irqs.is_some() {
                irqs += 1;
            }
        }
        assert_eq!(irqs, 0, "a masked compare must not raise the IRQ");
        assert_eq!(
            g.read_u32(OFF_EVENTS_COMPARE0).unwrap(),
            1,
            "the event still latches while the interrupt is masked"
        );
        assert_eq!(g.read_u32(OFF_INTEN0 + 0xC).unwrap(), 0, "INTPEND0 empty");
    }

    #[test]
    fn inactive_cc_channel_never_compares() {
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(cc_reg(0, 0x0), 3).unwrap();
        // CCEN.ACTIVE left clear.
        g.write_u32(OFF_INTEN0 + 0x4, 1 << 0).unwrap();
        g.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..10 {
            assert!(g.tick().explicit_irqs.is_none());
        }
        assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0).unwrap(), 0);
    }

    #[test]
    fn compare_uses_the_full_52_bit_value() {
        let mut g = Nrf54lGrtc::new_fast();
        // CC[1] = 0x2_0000_0005 — needs both CCL and CCH.
        g.write_u32(cc_reg(1, 0x0), 5).unwrap();
        g.write_u32(cc_reg(1, 0x4), 2).unwrap();
        g.write_u32(cc_reg(1, 0xC), CCEN_ACTIVE).unwrap();
        g.write_u32(OFF_TASKS_START, 1).unwrap();

        g.set_counter(0x1_FFFF_FFFF);
        g.tick(); // → 0x2_0000_0000, still below the compare
        assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0 + 4).unwrap(), 0);
        g.set_counter(0x2_0000_0004);
        g.tick(); // → 0x2_0000_0005
        assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0 + 4).unwrap(), 1);
    }

    #[test]
    fn events_write_one_ignored_write_zero_clears() {
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(OFF_EVENTS_COMPARE0, 1).unwrap();
        assert_eq!(
            g.read_u32(OFF_EVENTS_COMPARE0).unwrap(),
            0,
            "EVENTS_COMPARE write-1 must be a no-op"
        );

        g.write_u32(cc_reg(0, 0xC), CCEN_ACTIVE).unwrap();
        g.write_u32(OFF_TASKS_START, 1).unwrap();
        g.tick();
        assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0).unwrap(), 1);
        g.write_u32(OFF_EVENTS_COMPARE0, 0).unwrap();
        assert_eq!(
            g.read_u32(OFF_EVENTS_COMPARE0).unwrap(),
            0,
            "write-0 must clear the event"
        );
    }

    #[test]
    fn tasks_capture_snapshots_the_counter_into_cc() {
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(OFF_TASKS_START, 1).unwrap();
        g.set_counter(0x3_0000_0009);
        g.write_u32(OFF_TASKS_CAPTURE0 + 4 * 2, 1).unwrap(); // TASKS_CAPTURE[2]
        assert_eq!(g.read_u32(cc_reg(2, 0x0)).unwrap(), 0x0000_0009);
        assert_eq!(g.read_u32(cc_reg(2, 0x4)).unwrap(), 0x3);
    }

    #[test]
    fn ccadd_adds_to_the_syscounter_or_the_cc() {
        let mut g = Nrf54lGrtc::new_fast();
        g.set_counter(100);
        // REFERENCE=SYSCOUNTER (bit 31 clear): CC[0] = counter + 50.
        g.write_u32(cc_reg(0, 0x8), 50).unwrap();
        assert_eq!(g.read_u32(cc_reg(0, 0x0)).unwrap(), 150);
        // REFERENCE=CC (bit 31 set): CC[0] += 25.
        g.write_u32(cc_reg(0, 0x8), CCADD_REFERENCE_CC | 25)
            .unwrap();
        assert_eq!(g.read_u32(cc_reg(0, 0x0)).unwrap(), 175);
        // CCADD is write-only.
        assert_eq!(g.read_u32(cc_reg(0, 0x8)).unwrap(), 0);
    }

    #[test]
    fn out_of_range_cc_channel_is_ignored_not_panicking() {
        // A six-channel instance: CC[6..11] and their events are absent.
        let mut g = Nrf54lGrtc::new_with_cc(6);
        g.write_u32(cc_reg(9, 0x0), 0xDEAD_BEEF).unwrap();
        g.write_u32(cc_reg(9, 0x4), 0xF).unwrap();
        g.write_u32(cc_reg(9, 0xC), CCEN_ACTIVE).unwrap();
        assert_eq!(g.read_u32(cc_reg(9, 0x0)).unwrap(), 0);
        assert_eq!(g.read_u32(cc_reg(9, 0xC)).unwrap(), 0);
        assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0 + 4 * 9).unwrap(), 0);
        g.write_u32(OFF_TASKS_CAPTURE0 + 4 * 9, 1).unwrap();
        g.write_u32(OFF_EVENTS_COMPARE0 + 4 * 9, 0).unwrap();

        // The last real channel still works.
        g.write_u32(cc_reg(5, 0x0), 0x1234).unwrap();
        assert_eq!(g.read_u32(cc_reg(5, 0x0)).unwrap(), 0x1234);
    }

    #[test]
    fn inten_masked_to_num_cc() {
        let mut g = Nrf54lGrtc::new(); // 12 channels
        g.write_u32(OFF_INTEN0 + 0x4, 0xFFFF_FFFF).unwrap();
        assert_eq!(g.read_u32(OFF_INTEN0).unwrap(), 0x0000_0FFF);

        let mut g6 = Nrf54lGrtc::new_with_cc(6);
        g6.write_u32(OFF_INTEN0 + 0x4, 0xFFFF_FFFF).unwrap();
        assert_eq!(g6.read_u32(OFF_INTEN0).unwrap(), 0x0000_003F);
    }

    #[test]
    fn intenclr_clears_and_all_four_groups_are_independent() {
        let mut g = Nrf54lGrtc::new();
        for group in 0..NUM_INT_GROUPS as u64 {
            let base = OFF_INTEN0 + group * INT_GROUP_STRIDE;
            g.write_u32(base + 0x4, 1 << group).unwrap();
        }
        for group in 0..NUM_INT_GROUPS as u64 {
            let base = OFF_INTEN0 + group * INT_GROUP_STRIDE;
            assert_eq!(g.read_u32(base).unwrap(), 1 << group);
        }
        g.write_u32(OFF_INTEN0 + 0x8, 1 << 0).unwrap(); // INTENCLR0
        assert_eq!(g.read_u32(OFF_INTEN0).unwrap(), 0);
        assert_eq!(
            g.read_u32(OFF_INTEN0 + INT_GROUP_STRIDE).unwrap(),
            1 << 1,
            "clearing group 0 must not touch group 1"
        );
    }

    #[test]
    fn intpend_is_read_only() {
        let mut g = Nrf54lGrtc::new();
        g.write_u32(OFF_INTEN0 + 0xC, 0xFFFF_FFFF).unwrap();
        assert_eq!(g.read_u32(OFF_INTEN0 + 0xC).unwrap(), 0);
    }

    #[test]
    fn status_registers_read_ready() {
        let g = Nrf54lGrtc::new();
        assert_eq!(g.read_u32(OFF_STATUS_LFTIMER).unwrap(), STATUS_READY);
        assert_eq!(g.read_u32(OFF_STATUS_PWM).unwrap(), STATUS_READY);
        assert_eq!(g.read_u32(OFF_STATUS_CLKOUT).unwrap(), STATUS_READY);
    }

    #[test]
    fn clkcfg_holds_its_reset_value_and_reads_back_writes() {
        let mut g = Nrf54lGrtc::new();
        assert_eq!(g.read_u32(OFF_CLKCFG).unwrap(), CLKCFG_RESET_VALUE);
        g.write_u32(OFF_CLKCFG, 0x0002_0004).unwrap();
        assert_eq!(g.read_u32(OFF_CLKCFG).unwrap(), 0x0002_0004);
    }

    #[test]
    fn syscounter_registers_are_read_only_but_active_is_writable() {
        let mut g = Nrf54lGrtc::new_fast();
        g.write_u32(OFF_TASKS_START, 1).unwrap();
        g.tick();
        g.write_u32(SYSCOUNTERL, 0xFFFF_FFFF).unwrap();
        g.write_u32(SYSCOUNTERH, 0xFFFF_FFFF).unwrap();
        assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 1);

        g.write_u32(OFF_SYSCOUNTER0 + 0x8, 1).unwrap();
        assert_eq!(g.read_u32(OFF_SYSCOUNTER0 + 0x8).unwrap(), 1);
    }

    #[test]
    fn unimplemented_offsets_read_zero_without_faulting() {
        let g = Nrf54lGrtc::new();
        for off in [
            0x030u64,
            0x0C0,
            0x160,
            0x1F0,
            0x204,
            0x400 + 0x40,
            0x500,
            0xFFC,
        ] {
            assert_eq!(g.read_u32(off).unwrap(), 0, "offset {off:#05x}");
        }
    }

    #[test]
    fn tick_elapsed_matches_repeated_ticks() {
        let mut repeated = Nrf54lGrtc::new();
        let mut elapsed = Nrf54lGrtc::new();
        repeated.write_u32(OFF_TASKS_START, 1).unwrap();
        elapsed.write_u32(OFF_TASKS_START, 1).unwrap();

        let cycles = 3 * CYCLES_PER_SYSCOUNTER_TICK as u64 + 7;
        for _ in 0..cycles {
            repeated.tick();
        }
        elapsed.tick_elapsed(cycles);

        assert_eq!(repeated.read_u32(SYSCOUNTERL).unwrap(), 3);
        assert_eq!(
            elapsed.read_u32(SYSCOUNTERL).unwrap(),
            repeated.read_u32(SYSCOUNTERL).unwrap()
        );
    }

    #[test]
    fn syscounter_runs_at_one_megahertz_against_the_cpu_clock() {
        // 128 CPU cycles at 128 MHz is exactly 1 µs, i.e. one SYSCOUNTER tick.
        let mut g = Nrf54lGrtc::new();
        g.write_u32(OFF_TASKS_START, 1).unwrap();
        g.tick_elapsed(CPU_HZ_DEFAULT as u64 / 1000); // 1 ms of CPU cycles
        assert_eq!(
            g.read_u32(SYSCOUNTERL).unwrap(),
            SYSCOUNTER_HZ / 1000,
            "1 ms of CPU time must be 1000 SYSCOUNTER ticks"
        );
    }

    #[test]
    fn legacy_tick_charges_zero_cost() {
        // Tick-cost normalization: a free-running counter consumes no core
        // cycles, so the walk must charge zero (the scheduler path can't
        // reproduce a per-tick cost, and total_cycles must agree across modes).
        let mut g = Nrf54lGrtc::new();
        g.write_u32(OFF_TASKS_START, 1).unwrap();
        assert_eq!(g.tick().cycles, 0);
        assert_eq!(g.tick_elapsed(256).cycles, 0);
    }

    #[test]
    fn without_clock_stays_on_legacy_tick_path() {
        let g = Nrf54lGrtc::new();
        assert!(
            !g.uses_scheduler(),
            "no cycle clock attached → the model must stay on the legacy walk"
        );
        assert!(g.needs_legacy_walk());
    }

    #[cfg(feature = "event-scheduler")]
    mod scheduler_mode {
        use super::*;

        fn armed_scheduler() -> (Nrf54lGrtc, CycleClock) {
            let clock = CycleClock::default();
            let mut g = Nrf54lGrtc::new_fast(); // 1 CPU cycle per SYSCOUNTER tick
            g.attach_cycle_clock(clock.clone());
            (g, clock)
        }

        #[test]
        fn clock_attach_flips_to_scheduler_and_walk_tick_is_inert() {
            let (mut g, _clock) = armed_scheduler();
            assert!(g.uses_scheduler(), "clock attached → walk-independent");
            assert!(!g.needs_legacy_walk());
            g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
            // A stray walk tick must not double-count against the lazy anchor.
            let r = g.tick();
            assert!(r.explicit_irqs.is_none());
            assert_eq!(
                g.read_u32(SYSCOUNTERL).unwrap(),
                0,
                "tick inert in scheduler mode; counter derives from the clock"
            );
        }

        #[test]
        fn lazy_syscounter_read_tracks_published_clock_exactly() {
            let (mut g, clock) = armed_scheduler();
            g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
            g.sync_to(0);
            clock.publish(5);
            assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 5);
            clock.publish(9);
            assert_eq!(g.read_u32(SYSCOUNTERL).unwrap(), 9);
        }

        #[test]
        fn arming_write_schedules_the_exact_compare_deadline() {
            let (mut g, _clock) = armed_scheduler();
            g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
            g.sync_to(0);
            // CC[0] = 100; from counter 0 the fire lands 100 ticks after the
            // synced state; the bus adds current_cycle + 1, so the peripheral
            // hands out d - 1 = 99.
            g.write_u32(cc_reg(0, 0x0), 100).unwrap();
            g.write_u32(cc_reg(0, 0x4), 0).unwrap(); // CCH arms
            let evs = g.take_scheduled_events();
            assert_eq!(evs.len(), 1);
            assert_eq!(evs[0].0, 99, "delay must be cycles-to-fire minus one");
        }

        #[test]
        fn on_event_pends_the_group_irq_and_reschedules() {
            let (mut g, clock) = armed_scheduler();
            g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
            g.sync_to(0);
            g.write_u32(cc_reg(0, 0x0), 100).unwrap();
            g.write_u32(cc_reg(0, 0x4), 0).unwrap();
            g.write_u32(OFF_INTEN0 + 0x4, 1 << 0).unwrap(); // group 0 enables COMPARE0
            let token = g.take_scheduled_events()[0].1;

            clock.publish(100); // drain at the exact fire cycle
            let mut sched = crate::sched::EventScheduler::new();
            sched.advance_to(100);
            let mut bus = crate::bus::SystemBus::new();
            let res = g.on_event(token, &mut sched, &mut bus);
            assert_eq!(
                res.explicit_irqs,
                vec![GRTC_IRQ_BASE_DEFAULT],
                "group-0 compare pends GRTC_0"
            );
            assert_eq!(res.fired_events, vec![OFF_EVENTS_COMPARE0 as u32]);
            assert_eq!(g.read_u32(OFF_EVENTS_COMPARE0).unwrap(), 1);
            // One-shot: no further compare armed → nothing to reschedule.
            assert_eq!(res.reschedule_delay, None);

            // A second drain at the same cycle claims nothing.
            let res2 = g.on_event(token, &mut sched, &mut bus);
            assert!(res2.explicit_irqs.is_empty(), "no double-claim");
        }

        #[test]
        fn stale_event_chain_dies_on_token_mismatch() {
            let (mut g, clock) = armed_scheduler();
            g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
            g.sync_to(0);
            g.write_u32(cc_reg(0, 0x0), 100).unwrap();
            g.write_u32(cc_reg(0, 0x4), 0).unwrap();
            let old_token = g.take_scheduled_events()[0].1;
            // Re-arm (kills the old chain).
            g.write_u32(cc_reg(0, 0x0), 50).unwrap();
            g.write_u32(cc_reg(0, 0x4), 0).unwrap();
            let new_token = g.take_scheduled_events()[0].1;
            assert_ne!(old_token, new_token);

            clock.publish(500);
            let mut sched = crate::sched::EventScheduler::new();
            sched.advance_to(500);
            let mut bus = crate::bus::SystemBus::new();
            let res = g.on_event(old_token, &mut sched, &mut bus);
            assert!(res.explicit_irqs.is_empty(), "stale chain must be inert");
            assert_eq!(res.reschedule_delay, None, "stale chain must not respawn");
        }

        #[test]
        fn masked_compare_still_latches_but_schedules_no_irq() {
            let (mut g, clock) = armed_scheduler();
            g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
            g.sync_to(0);
            g.write_u32(cc_reg(0, 0x0), 10).unwrap();
            g.write_u32(cc_reg(0, 0x4), 0).unwrap(); // armed, INTEN clear
            let token = g.take_scheduled_events()[0].1;

            clock.publish(10);
            let mut sched = crate::sched::EventScheduler::new();
            sched.advance_to(10);
            let mut bus = crate::bus::SystemBus::new();
            let res = g.on_event(token, &mut sched, &mut bus);
            assert!(
                res.explicit_irqs.is_empty(),
                "a masked compare must not pend an IRQ"
            );
            assert_eq!(
                g.read_u32(OFF_EVENTS_COMPARE0).unwrap(),
                1,
                "the event still latches while the interrupt is masked"
            );
        }

        #[test]
        fn cc0_interval_reload_reschedules_the_next_period() {
            let (mut g, clock) = armed_scheduler();
            g.write_u32(OFF_MODE, MODE_SYSCOUNTEREN).unwrap();
            g.write_u32(OFF_INTERVAL, 20).unwrap();
            g.sync_to(0);
            g.write_u32(cc_reg(0, 0x0), 20).unwrap();
            g.write_u32(cc_reg(0, 0x4), 0).unwrap();
            g.write_u32(OFF_INTEN0 + 0x4, 1 << 0).unwrap();
            let token = g.take_scheduled_events()[0].1;

            clock.publish(20);
            let mut sched = crate::sched::EventScheduler::new();
            sched.advance_to(20);
            let mut bus = crate::bus::SystemBus::new();
            let res = g.on_event(token, &mut sched, &mut bus);
            assert_eq!(res.explicit_irqs, vec![GRTC_IRQ_BASE_DEFAULT]);
            // CC0 auto-reloads by INTERVAL and stays armed (deadline → 40).
            assert_eq!(g.read_u32(cc_reg(0, 0x0)).unwrap(), 40);
            assert_eq!(g.read_u32(cc_reg(0, 0xC)).unwrap(), CCEN_ACTIVE);
            // But EVENTS_COMPARE[0] is still latched, so — exactly like the
            // legacy walk's `&& events_compare == 0` guard — the channel cannot
            // re-fire and nothing is rescheduled until the ISR clears it.
            assert_eq!(res.reschedule_delay, None);

            // ISR clears the event; the arming-recompute (which the bus runs on
            // that write) now schedules the next period at 40 (20 ticks away).
            g.write_u32(OFF_EVENTS_COMPARE0, 0).unwrap();
            g.sync_to(20);
            let next = g.take_scheduled_events();
            assert_eq!(
                next.first().map(|e| e.0),
                Some(19),
                "next period at cycle 40"
            );
        }
    }
}
