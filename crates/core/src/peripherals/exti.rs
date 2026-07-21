// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// EXTI is one struct PER FAMILY behind the `Exti` enum. The F1 variant is a
// single 20-line bank; the L4 variant adds bank 2 (lines 32..39). The bank-2
// registers therefore exist ONLY on the L4 variant — an F1 EXTI cannot carry
// (or be tricked into addressing) bank-2 state. Bank-1 IRQ routing, shared by
// both families, lives in one stateless helper.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtiRegisterLayout {
    /// STM32F1 / F4-class: single bank, 20 lines, registers at 0x00..0x14.
    #[default]
    Stm32F1,
    /// STM32L4: two banks (40 lines total). Bank1 at 0x00..0x14, bank2 at
    /// 0x20..0x34. Bank-2 covers lines 32..39 (LPTIM/COMP/I2C/USART wakeup).
    Stm32L4,
}

impl FromStr for ExtiRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32l4" | "l4" => Ok(Self::Stm32L4),
            _ => Err(format!(
                "unsupported EXTI register layout '{}'; supported: stm32f1, stm32l4",
                value
            )),
        }
    }
}

/// One EXTI register bank (IMR/EMR/RTSR/FTSR/SWIER/PR).
#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
struct ExtiBank {
    imr: u32,
    emr: u32,
    rtsr: u32,
    ftsr: u32,
    swier: u32,
    pr: u32,
}

impl ExtiBank {
    /// IMR/EMR/RTSR/FTSR read at 0x00/0x04/0x08/0x0C; SWIER 0x10; PR 0x14.
    fn read(&self, off: u64) -> u32 {
        match off {
            0x00 => self.imr,
            0x04 => self.emr,
            0x08 => self.rtsr,
            0x0C => self.ftsr,
            0x10 => self.swier,
            0x14 => self.pr,
            _ => 0,
        }
    }
    /// `mask` is the implemented-line mask for this bank.
    fn write(&mut self, off: u64, value: u32, mask: u32) {
        match off {
            0x00 => self.imr = value & mask,
            0x04 => self.emr = value & mask,
            0x08 => self.rtsr = value & mask,
            0x0C => self.ftsr = value & mask,
            0x10 => {
                // SWIER: a 0->1 edge sets the matching PR bit.
                let diff = (self.swier ^ value) & value & mask;
                self.swier = value & mask;
                self.pr |= diff;
            }
            0x14 => {
                // PR is rc_w1: writing 1 clears the pending bit. Clearing a PR
                // bit also clears the matching SWIER bit (RM0008 §10.3.6) — the
                // software-event line de-asserts. Silicon-verified on the bench
                // STM32F103 (stm32f1_exec_oracle::exti_swier_sets_and_clears_pr).
                let clear = value & mask;
                self.pr &= !clear;
                self.swier &= !clear;
            }
            _ => {}
        }
    }
}

/// Bank-1 IRQ routing — identical on every family (lines 0..4 → IRQ 6..10,
/// 9..5 → 23, 15..10 → 40). Shared behaviour, not shared state.
fn route_bank1_irqs(active1: u32, irqs: &mut Vec<u32>) {
    for i in 0..5 {
        if (active1 & (1 << i)) != 0 {
            irqs.push(6 + i);
        }
    }
    if (active1 & 0x0000_03E0) != 0 {
        irqs.push(23); // EXTI9_5
    }
    if (active1 & 0x0000_FC00) != 0 {
        irqs.push(40); // EXTI15_10
    }
}

// ── STM32F1 / F4: single bank ────────────────────────────────────────────────
// The implemented-line count is part-specific (F103 = 19 lines, F4-class = more),
// so the mask is per-instance, set from the chip config's `lines` field. Default
// is the historical 20-line value for parts not yet silicon-pinned.
#[derive(Debug, serde::Serialize)]
pub struct F1Exti {
    bank1: ExtiBank,
    line_mask: u32,

    /// Bus-published cycle clock (walk-free campaign). `Some` once attached →
    /// event-schedulable; `None` keeps the legacy walk.
    #[serde(skip)]
    clock: Option<CycleClock>,
    /// Scheduler mode: `true` while the held-level re-emit event is live.
    #[serde(skip)]
    chain_live: bool,
}

impl Default for F1Exti {
    fn default() -> Self {
        Self {
            bank1: ExtiBank::default(),
            line_mask: 0x000F_FFFF, // 20 lines
            clock: None,
            chain_live: false,
        }
    }
}

// ── STM32L4: two banks (bank 2 = lines 32..39) ───────────────────────────────
#[derive(Debug, Default, serde::Serialize)]
pub struct L4Exti {
    bank1: ExtiBank,
    bank2: ExtiBank,

    #[serde(skip)]
    clock: Option<CycleClock>,
    #[serde(skip)]
    chain_live: bool,
}

impl L4Exti {
    const MASK1: u32 = 0xFFFF_FFFF; // full word for bank 1
    const MASK2: u32 = 0x0000_00FF; // lines 32..39
}

/// External Interrupt/Event Controller — one variant per chip family.
#[derive(Debug, serde::Serialize)]
pub enum Exti {
    Stm32F1(F1Exti),
    Stm32L4(L4Exti),
}

impl Default for Exti {
    fn default() -> Self {
        Self::new()
    }
}

impl Exti {
    pub fn new() -> Self {
        Self::new_with_layout(ExtiRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: ExtiRegisterLayout) -> Self {
        Self::new_with_layout_lines(layout, 0x000F_FFFF)
    }

    /// Like [`new_with_layout`] but with an explicit F1 implemented-line mask
    /// (e.g. `0x0007_FFFF` for the STM32F103's 19 lines). Ignored for L4.
    pub fn new_with_layout_lines(layout: ExtiRegisterLayout, line_mask: u32) -> Self {
        match layout {
            ExtiRegisterLayout::Stm32F1 => Self::Stm32F1(F1Exti {
                line_mask,
                ..Default::default()
            }),
            ExtiRegisterLayout::Stm32L4 => Self::Stm32L4(L4Exti::default()),
        }
    }

    /// Inject an external trigger on `line` (sets the corresponding PR bit).
    /// Bank-2 lines (32..39) exist only on the L4 variant.
    pub fn trigger_line(&mut self, line: u8) {
        match self {
            Self::Stm32F1(e) => {
                if line < 32 {
                    e.bank1.pr |= 1u32 << line;
                }
            }
            Self::Stm32L4(e) => match line {
                0..=31 => e.bank1.pr |= 1u32 << line,
                32..=39 => e.bank2.pr |= 1u32 << (line - 32),
                _ => {}
            },
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match self {
            Self::Stm32F1(e) => match offset {
                0x00..=0x14 => e.bank1.read(offset),
                _ => 0,
            },
            Self::Stm32L4(e) => match offset {
                0x00..=0x14 => e.bank1.read(offset),
                0x20..=0x34 => e.bank2.read(offset - 0x20),
                _ => 0,
            },
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match self {
            Self::Stm32F1(e) => {
                if (0x00..=0x14).contains(&offset) {
                    let mask = e.line_mask;
                    e.bank1.write(offset, value, mask);
                }
            }
            Self::Stm32L4(e) => match offset {
                0x00..=0x14 => e.bank1.write(offset, value, L4Exti::MASK1),
                0x20..=0x34 => e.bank2.write(offset - 0x20, value, L4Exti::MASK2),
                _ => {}
            },
        }
    }

    /// The set of NVIC IRQ lines the held level asserts this cycle — exactly the
    /// list `tick()` re-emits. Shared by the legacy walk and the event chain so
    /// both routes are byte-identical by construction.
    fn pending_irqs(&self) -> Vec<u32> {
        let mut irqs = Vec::new();
        match self {
            Self::Stm32F1(e) => {
                let active1 = e.bank1.pr & e.bank1.imr;
                if active1 != 0 {
                    route_bank1_irqs(active1, &mut irqs);
                }
            }
            Self::Stm32L4(e) => {
                let active1 = e.bank1.pr & e.bank1.imr;
                let active2 = e.bank2.pr & e.bank2.imr;
                if active1 != 0 {
                    route_bank1_irqs(active1, &mut irqs);
                }
                if active2 != 0 {
                    // Bank-2 wakeup lines → their peripheral's NVIC IRQ
                    // (RM0351 §13.3). Lines without an entry are tracked at the
                    // register level but don't synthesize an IRQ yet.
                    for &(line, irq) in &[
                        (35u32, 70u32), // LPUART1 wakeup
                        (36, 31),       // I2C1 wakeup
                        (37, 33),       // I2C2 wakeup
                        (38, 72),       // I2C3 wakeup
                        (39, 37),       // USART1 wakeup
                    ] {
                        if (active2 >> (line - 32)) & 1 != 0 {
                            irqs.push(irq);
                        }
                    }
                }
            }
        }
        irqs
    }

    /// True while the held level is asserted (any masked pending line). Outside
    /// this window `tick()` emits nothing, so the event chain may stop and let
    /// idle fast-forward engage; firmware clearing PR (rc_w1) drops it.
    fn active(&self) -> bool {
        match self {
            Self::Stm32F1(e) => (e.bank1.pr & e.bank1.imr) != 0,
            Self::Stm32L4(e) => (e.bank1.pr & e.bank1.imr) != 0 || (e.bank2.pr & e.bank2.imr) != 0,
        }
    }

    #[inline]
    fn scheduler_mode(&self) -> bool {
        let clock = match self {
            Self::Stm32F1(e) => &e.clock,
            Self::Stm32L4(e) => &e.clock,
        };
        cfg!(feature = "event-scheduler") && clock.is_some()
    }

    fn set_chain_live(&mut self, live: bool) {
        match self {
            Self::Stm32F1(e) => e.chain_live = live,
            Self::Stm32L4(e) => e.chain_live = live,
        }
    }

    fn chain_live(&self) -> bool {
        match self {
            Self::Stm32F1(e) => e.chain_live,
            Self::Stm32L4(e) => e.chain_live,
        }
    }

    /// Test/differential knob: detach the clock, pinning the model to the legacy
    /// walk (the walk-on reference for the differential gate).
    pub fn force_legacy_walk(&mut self) {
        match self {
            Self::Stm32F1(e) => e.clock = None,
            Self::Stm32L4(e) => e.clock = None,
        }
    }
}

impl Peripheral for Exti {
    fn read(&self, offset: u64) -> SimResult<u8> {
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

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // SWIER (0->1 edge-detect → sets PR) and PR (rc_w1) only behave
        // correctly under whole-word access. The default byte-decomposition
        // reads back the current value for the un-targeted bytes and writes it
        // back: for the rc_w1 PR that clears still-pending bits (write-1-clear),
        // and it mis-fires SWIER's edge detector. Silicon performs the STR as
        // one 32-bit transaction; mirror that by handing write_reg the whole
        // word. Silicon-verified on bench STM32F103 (stm32f1_exec_oracle::exti_*).
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Scheduler-mode instances are walk-skipped; the event chain owns the
        // held-level re-emission. Guard against a stray direct call.
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        let irqs = self.pending_irqs();
        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        match self {
            Self::Stm32F1(e) => e.clock = Some(clock),
            Self::Stm32L4(e) => e.clock = Some(clock),
        }
    }

    fn sync_to(&mut self, _now_cycle: u64) {
        // No lazily-accumulated state: PR/IMR mutate synchronously in write, and
        // the held level is re-derived from them every cycle by the event chain.
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        // Arm the held-level re-emit chain the moment a write raises a masked
        // pending line (SWIER→PR, or IMR unmasking a pending PR). Every EXTI
        // PR-setting path today is an MMIO write (SWIER edge-detect or a direct
        // PR/IMR write), so `take_scheduled_events` — drained after every MMIO
        // write — always catches the activation. (`trigger_line`, the external
        // GPIO-edge injector, has no runtime caller; were it ever wired to a bus
        // path, that path must re-arm the chain the same way, since a `&mut`
        // injector cannot itself return events.) delay-0 → deadline
        // `current_cycle + 1` = the walk's next tick.
        if self.scheduler_mode() && self.active() && !self.chain_live() {
            self.set_chain_live(true);
            vec![(0u64, 0u32)]
        } else {
            Vec::new()
        }
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() {
            return crate::sched::EventResult::default();
        }
        // Re-emit the held level every cycle while any masked line stays pending
        // — the event-path equivalent of the legacy `tick()` returning its
        // `explicit_irqs` each cycle. Perpetuate at delay 1 while active; stop
        // when firmware clears PR (rc_w1), letting fast-forward engage.
        let active = self.active();
        self.set_chain_live(active);
        crate::sched::EventResult {
            explicit_irqs: self.pending_irqs(),
            reschedule_delay: active.then_some(1),
            ..Default::default()
        }
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::{Exti, ExtiRegisterLayout};
    use crate::Peripheral;

    fn poke(exti: &mut Exti, off: u64, val: u32) {
        for i in 0..4 {
            exti.write(off + i, ((val >> (i * 8)) & 0xFF) as u8)
                .unwrap();
        }
    }

    #[test]
    fn l4_bank2_line35_routes_to_lpuart1_irq() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32L4);
        // Arm IMR2 line 3 (= EXTI line 35), then trigger via SWIER2.
        poke(&mut e, 0x20, 1 << 3);
        poke(&mut e, 0x30, 1 << 3);
        let r = e.tick();
        let irqs = r.explicit_irqs.expect("expected IRQ list");
        assert!(
            irqs.contains(&70),
            "LPUART1 IRQ 70 should fire, got {irqs:?}"
        );
    }

    #[test]
    fn l4_bank2_line38_routes_to_i2c3_irq() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32L4);
        poke(&mut e, 0x20, 1 << 6);
        poke(&mut e, 0x30, 1 << 6);
        let r = e.tick();
        let irqs = r.explicit_irqs.expect("expected IRQ list");
        assert!(irqs.contains(&72), "I2C3 IRQ 72 should fire, got {irqs:?}");
    }

    #[test]
    fn f1_layout_does_not_synth_bank2_irqs() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32F1);
        poke(&mut e, 0x20, 1 << 3); // bank 2 doesn't even exist on F1
        poke(&mut e, 0x30, 1 << 3);
        let r = e.tick();
        // No bank-2 -> no IRQ
        assert!(r.explicit_irqs.is_none() || r.explicit_irqs.unwrap().is_empty());
    }

    #[test]
    fn f1_bank1_swier_sets_pr_and_routes() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32F1);
        poke(&mut e, 0x00, 1 << 0); // IMR line 0
        poke(&mut e, 0x10, 1 << 0); // SWIER line 0 -> PR
        let r = e.tick();
        assert!(r.explicit_irqs.expect("irqs").contains(&6), "EXTI0 -> IRQ6");
    }

    #[test]
    fn all_variants_flip_to_scheduler_when_clock_attached() {
        for layout in [ExtiRegisterLayout::Stm32F1, ExtiRegisterLayout::Stm32L4] {
            let mut e = Exti::new_with_layout(layout);
            assert!(e.needs_legacy_walk() && !e.uses_scheduler());
            e.attach_cycle_clock(crate::CycleClock::default());
            #[cfg(feature = "event-scheduler")]
            {
                assert!(e.uses_scheduler() && !e.needs_legacy_walk());
                // Held level latched, but the walk tick is inert in scheduler mode.
                e.write_u32(0x00, 1).unwrap();
                e.write_u32(0x10, 1).unwrap();
                assert!(e.tick().explicit_irqs.is_none());
                e.force_legacy_walk();
                assert!(e.needs_legacy_walk() && !e.uses_scheduler());
            }
        }
    }

    #[test]
    fn f1_word_write_pr_clear_is_atomic_and_clears_swier() {
        // Whole-word access (as a 32-bit STR performs). SWIER=0x5 software-
        // triggers lines 0 and 2 -> PR=0x5; an rc_w1 word-write of 0x1 clears
        // ONLY line 0 (the default byte-decomposition would also wipe line 2 by
        // reading PR back and re-writing it). Clearing PR line 0 also clears
        // SWIER line 0. Silicon-verified on bench F103 (exti_swier oracle).
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32F1);
        e.write_u32(0x00, 0x5).unwrap(); // IMR lines 0,2
        e.write_u32(0x10, 0x5).unwrap(); // SWIER lines 0,2 -> PR
        assert_eq!(e.read_u32(0x14).unwrap(), 0x5, "PR set on lines 0,2");
        e.write_u32(0x14, 0x1).unwrap(); // rc_w1: clear line 0 only
        assert_eq!(e.read_u32(0x14).unwrap(), 0x4, "PR line 2 still pending");
        assert_eq!(
            e.read_u32(0x10).unwrap(),
            0x4,
            "SWIER line 0 cleared with PR"
        );
    }
}

// ── Walk-free differential: held-level EXTI walk vs scheduler ────────────────
#[cfg(all(test, feature = "event-scheduler"))]
mod scheduler_diff {
    use super::*;
    use crate::Peripheral;

    #[derive(Clone, Copy)]
    enum Op {
        /// 32-bit register write at cycle (as firmware STRs it).
        Write(u64, u32),
    }

    fn build(layout: ExtiRegisterLayout, scheduler: bool) -> Exti {
        let mut e = Exti::new_with_layout(layout);
        if scheduler {
            e.attach_cycle_clock(CycleClock::default());
        }
        e
    }

    /// Drive the SAME op script against (a) the per-cycle walk and (b) the event
    /// path; assert the emitted IRQ set AND the register snapshot are identical
    /// every cycle. An `Op` scheduled at cycle `c` is applied before that cycle's
    /// tick. This is the held-level analogue of the I2C `kinetis_scheduler` gate.
    fn assert_walk_identical(layout: ExtiRegisterLayout, script: &[(u64, Op)], cycles: u64) {
        let mut walk = build(layout, false);
        let mut sched = build(layout, true);
        let clock = match &sched {
            Exti::Stm32F1(e) => e.clock.clone(),
            Exti::Stm32L4(e) => e.clock.clone(),
        }
        .unwrap();

        // (deadline_cycle, token) event queue, driven exactly like Machine +
        // SystemBus at tick interval 1.
        let mut events: Vec<(u64, u32)> = Vec::new();
        let bus = &mut crate::bus::SystemBus::new();

        for c in 1..=cycles {
            for (sc, Op::Write(off, val)) in script.iter().copied() {
                if sc == c {
                    walk.write_u32(off, val).unwrap();
                    // Scheduler: write, then harvest events at cycle+1+delay
                    // (`now == c - 1` at the point of the write).
                    sched.write_u32(off, val).unwrap();
                    for (delay, token) in sched.take_scheduled_events() {
                        events.push((c - 1 + 1 + delay, token));
                    }
                }
            }

            // Walk: tick emits the held level this cycle.
            let walk_irqs = walk.tick().explicit_irqs.unwrap_or_default();

            // Scheduler: publish the clock, drain due events through on_event.
            clock.publish(c);
            let due: Vec<(u64, u32)> = events.iter().copied().filter(|(d, _)| *d <= c).collect();
            events.retain(|(d, _)| *d > c);
            let mut esched = crate::sched::EventScheduler::new();
            esched.advance_to(c);
            let mut sched_irqs = Vec::new();
            for (_, token) in due {
                let res = sched.on_event(token, &mut esched, bus);
                sched_irqs.extend(res.explicit_irqs);
                if let Some(delay) = res.reschedule_delay {
                    events.push((c + delay, token));
                }
            }

            assert_eq!(
                walk_irqs, sched_irqs,
                "emitted IRQ set diverged at cycle {c}"
            );
            assert_eq!(
                walk.snapshot(),
                sched.snapshot(),
                "register snapshot diverged at cycle {c}"
            );
        }
    }

    #[test]
    fn f1_swier_hold_and_clear_walk_identity() {
        // IMR lines 0,2; SWIER trigger → PR held (IRQs re-emit every cycle);
        // firmware clears PR line 0 (rc_w1) mid-hold; then clears line 2.
        let script = [
            (1u64, Op::Write(0x00, 0x5)), // IMR lines 0,2
            (1, Op::Write(0x10, 0x5)),    // SWIER lines 0,2 → PR (held level)
            (5, Op::Write(0x14, 0x1)),    // clear PR line 0 → EXTI0 stops
            (9, Op::Write(0x14, 0x4)),    // clear PR line 2 → level drops
        ];
        assert_walk_identical(ExtiRegisterLayout::Stm32F1, &script, 14);
    }

    #[test]
    fn f1_grouped_irq_line_walk_identity() {
        // Lines in the EXTI9_5 group (shared IRQ 23) plus EXTI15_10 (IRQ 40).
        let script = [
            (1u64, Op::Write(0x00, (1 << 7) | (1 << 12))), // IMR lines 7,12
            (1, Op::Write(0x10, (1 << 7) | (1 << 12))),    // SWIER → PR
            (6, Op::Write(0x14, 1 << 7)),                  // clear line 7 (IRQ23 drops)
            (9, Op::Write(0x14, 1 << 12)),                 // clear line 12 (IRQ40 drops)
        ];
        assert_walk_identical(ExtiRegisterLayout::Stm32F1, &script, 14);
    }

    #[test]
    fn l4_bank2_wakeup_hold_walk_identity() {
        // Bank-2 line 36 (I2C1 wakeup → IRQ 31) plus a bank-1 line, held and
        // cleared independently across banks.
        let script = [
            (1u64, Op::Write(0x00, 1 << 1)), // IMR1 line 1
            (1, Op::Write(0x10, 1 << 1)),    // SWIER1 → PR1
            (1, Op::Write(0x20, 1 << 4)),    // IMR2 line 36
            (1, Op::Write(0x30, 1 << 4)),    // SWIER2 → PR2 (I2C1 wakeup)
            (6, Op::Write(0x14, 1 << 1)),    // clear bank-1
            (10, Op::Write(0x34, 1 << 4)),   // clear bank-2
        ];
        assert_walk_identical(ExtiRegisterLayout::Stm32L4, &script, 15);
    }

    #[test]
    fn imr_unmask_after_pending_arms_walk_identity() {
        // PR set while masked (no IRQ), THEN IMR unmasks it — the write that
        // raises the level must arm the chain. Validates the arming predicate.
        let script = [
            (1u64, Op::Write(0x10, 0x2)), // SWIER line 1 → PR (but IMR=0 → masked)
            (5, Op::Write(0x00, 0x2)),    // IMR line 1 → level rises here
            (9, Op::Write(0x14, 0x2)),    // clear
        ];
        assert_walk_identical(ExtiRegisterLayout::Stm32F1, &script, 13);
    }
}
