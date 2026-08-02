// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{CycleClock, SimResult};
use std::cell::Cell;

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
    /// Bus-published cycle clock (walk-free plan Part 1). `Some` once the bus
    /// registration choke attaches it; `None` keeps the model on the legacy
    /// walk path.
    #[serde(skip)]
    clock: Option<CycleClock>,
}

/// Full 32-bit ARR sentinel: the walk's `cnt > arr` overflow check can never
/// fire, so the counter free-runs mod 2^32 with no update events.
const ARR_NEVER_WRAPS: u32 = u32::MAX;

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
            clock: None,
        }
    }

    /// Mark this timer as a basic timer (TIM6/TIM7): no capture/compare
    /// channels, so an update event latches only UIF. Builder form so the
    /// existing `new_with_layout` call sites stay unchanged.
    pub fn basic(mut self, basic: bool) -> Self {
        self.basic = basic;
        self
    }

    /// True when the event scheduler owns this timer's time base (feature on
    /// AND bus clock attached). Everything time-related branches on this ONE
    /// predicate so the two drive modes can never mix.
    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to
    /// the legacy walk path (`uses_scheduler() == false`). Used by the
    /// walk-on-vs-scheduler differential gates to build the reference config
    /// from the same bus assembly.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// IRQ-level / counter-freeze predicate: the legacy `tick()` returns the
    /// held IRQ level (and skips all counting) while any enabled status flag
    /// among UIF/CC1..4IF is latched.
    #[inline]
    fn irq_level_held(&self) -> bool {
        (self.sr.get() & self.dier & 0x1F) != 0
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

    /// The increment index of the first latch of an *enabled* flag (the walk
    /// freezes counting from the next tick on, and pends the NVIC line), and
    /// whether the pend lands on the SAME tick as the increment (update event
    /// with UIE — the walk returns `irq` from the overflow tick itself) or on
    /// the NEXT tick (compare match — latched on the match tick, first pended
    /// by the level check one tick later).
    fn first_enabled_event(&self) -> Option<(u64, bool)> {
        if (self.cr1 & 0x1) == 0 {
            return None;
        }
        let v = self.cnt.get();
        let mut best: Option<(u64, bool)> = None;
        if (self.dier & 0x1) != 0 {
            if let Some(j) = self.increments_to_wrap(v) {
                best = Some((j, true));
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
                if ccs != 0 || (self.dier >> bit) & 1 == 0 {
                    continue;
                }
                if let Some(j) = self.increments_to_value(v, ccr & mask) {
                    // Strict `<` keeps update-event precedence on a tie (the
                    // walk pends the overflow tick itself when UIE is set).
                    if best.is_none_or(|(b, _)| j < b) {
                        best = Some((j, false));
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
        let anchor = self.anchor.get();
        if now <= anchor {
            return;
        }
        self.anchor.set(now);
        if self.irq_level_held() || (self.cr1 & 0x1) == 0 {
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
        let freeze_j = self.first_enabled_event().map(|(j, _)| j);
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
    /// legacy walk would FIRST pend the NVIC line, for the event chain:
    /// `None` when nothing is armed (chain dies; the next relevant MMIO write
    /// re-arms). The tick is `k1 + (j-1)*(PSC+1)` for the increment, plus one
    /// for compare matches (level-pended one tick after the latch).
    fn ticks_until_first_pend(&self) -> Option<u64> {
        if self.irq_level_held() {
            // Already held: the walk pends on the very next tick.
            return Some(1);
        }
        let (j, same_tick) = self.first_enabled_event()?;
        let t = self.ticks_to_first_increment() + (j - 1) * (self.psc as u64 + 1);
        Some(if same_tick { t } else { t + 1 })
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
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
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
            0x18 => self.ccmr1 = value & 0xFFFF,
            0x1C => self.ccmr2 = value & 0xFFFF,
            0x20 => {
                // CCER mask: general-purpose timers expose CCxE (bit 0) +
                // CCxP (bit 1) per channel — 4 channels = 0x3333.
                // Advanced timers add CCxNE (bit 2) + CCxNP (bit 3) for
                // the complementary output — 4 channels = 0xFFFF.
                let mask = if self.advanced { 0xFFFF } else { 0x3333 };
                self.ccer = value & mask;
            }
            0x24 => self.cnt.set(value & self.cnt_mask()),
            0x28 => self.psc = value & 0xFFFF,
            0x2C => self.arr = value & self.cnt_mask(),
            0x30 if self.advanced => self.rcr = value & 0xFFFF,
            0x34 => self.ccr1 = value & self.cnt_mask(),
            0x38 => self.ccr2 = value & self.cnt_mask(),
            0x3C => self.ccr3 = value & self.cnt_mask(),
            0x40 => self.ccr4 = value & self.cnt_mask(),
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
            _ => {}
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
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
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
        if self.irq_level_held() {
            return crate::PeripheralTickResult {
                irq: true,
                cycles: 0,
                ..Default::default()
            };
        }

        // Counter Enable (bit 0)
        if (self.cr1 & 0x1) == 0 {
            return crate::PeripheralTickResult {
                irq: false,
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
                return crate::PeripheralTickResult {
                    irq: (self.dier & 1) != 0,
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

        crate::PeripheralTickResult {
            irq: false,
            cycles: 0,
            dma_signals: None,
            ..Default::default()
        }
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
            self.advance_to(now_cycle);
        }
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        // Kill any in-flight chain: the configuration (or counter) may have
        // just changed under this write, so its deadline is stale. The fresh
        // chain below carries the new token.
        self.arm_seq = self.arm_seq.wrapping_add(1);
        // `collect_scheduled_events` converts to `current_cycle + 1 + delay`;
        // the first pend lands `d` walk ticks after the just-synced state,
        // i.e. at absolute cycle `current_cycle + d` — hence `d - 1`
        // (d >= 1 always: a held level pends on the next tick, d == 1).
        self.ticks_until_first_pend()
            .map(|d| vec![(d - 1, self.arm_seq)])
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
        // Bring the lazy counter up to the drain cycle; this is what
        // materialises the update/compare latch this event was scheduled for.
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
    use super::Timer;
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

    #[cfg(feature = "event-scheduler")]
    mod scheduler_mode {
        use super::*;
        use crate::CycleClock;

        /// Mirror of the legacy per-tick walk semantics, kept in the test as
        /// an independent oracle: returns whether the walk pends the NVIC
        /// line on this tick.
        fn walk_tick_oracle(t: &mut Timer) -> bool {
            t.tick().irq
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
