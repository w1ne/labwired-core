// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Silicon Labs EFR32 Series-2 **GPIO external interrupts** — the block behind
//! `attachInterrupt`.
//!
//! # Where this lives and why it is its own peripheral
//!
//! The EXTI registers are in the GPIO block head (`GPIO_S_BASE + 0x400`), far
//! above the four port structs at `+0x30`.. that
//! [`crate::peripherals::gpio::GpioPort`] models. Modelling them here, as a
//! window of their own, keeps that working port model untouched — the
//! alternative was folding four live board_io-bound port windows into one
//! block-wide peripheral, which is a refactor with nothing to gain.
//!
//! Before this, the whole `0x400`.. range was unmapped: a firmware that
//! configured a pin interrupt faulted the bus.
//!
//! # The mux, from the reference manual
//!
//! EFR32xG26 RM rev 1.0 §24.3.10.1 "Standard Interrupt Generation", including
//! its figure, states the rule exactly:
//!
//! ```text
//!   p = 4 * int( n / 4 )        and the mux selects PA[p+3:p] / PB[p+3:p] / …
//! ```
//!
//! So external interrupt line `n`:
//!
//! * takes its PORT from `EXTIPSEL[n]` — 2 bits, 0=PORTA … 3=PORTD;
//! * takes its PIN from `p + EXTIPINSEL[n]`, where `p = 4*(n/4)` and
//!   `EXTIPINSEL[n]` is 2 bits.
//!
//! ⚠️ That means a line canNOT watch an arbitrary pin. EXTI1 can watch pins
//! 0..3 of any port and nothing else; watching PB07 needs a line in 4..7. A
//! model that let any line watch any pin would accept firmware the silicon
//! rejects, and the mistake would only show up on the bench.
//!
//! Both fields are 4 bits wide in the register with only the low 2 used, and
//! lines 0..7 live in the `…L` register, 8..15 in `…H`.
//!
//! # Faithfully modelled
//!
//! * The port/pin mux above, including the group-of-four restriction.
//! * Edge selection: `EXTIRISE[n]` and `EXTIFALL[n]` independently, so a line
//!   armed for neither never fires and a line armed for both fires on any
//!   change.
//! * `IF[n]` latches and is write-1-to-clear; `IEN[n]` gates the interrupt.
//! * The split vector: even lines raise `GPIO_EVEN_IRQn` (40), odd lines
//!   `GPIO_ODD_IRQn` (39) — CMSIS `efr32mg26b510f3200im48.h`. Firmware
//!   installs two different handlers and a model that raised one line for both
//!   would run the wrong one.
//! * A flag latches even with `IEN[n]` clear, which is what lets firmware poll
//!   `IF` instead of taking an interrupt.
//!
//! # Idealised — present, but not physical
//!
//! * **No `GPIO_LOCK`.** The RM (§24.3.11) says writing anything but `0xA534`
//!   to `GPIO_LOCK` locks the configuration registers, EXTIPSEL among them.
//!   That register is in the same block but not this window, and the lock is
//!   not enforced here — firmware that locks the GPIO and then reconfigures a
//!   line is accepted where silicon would ignore it.
//! * **No EM4 wake-up.** `EM4WUEN`/`EM4WUPOL` decode and store; there are no
//!   energy modes to wake from.
//! * **No PRS.** A GPIO edge is a PRS producer on silicon; nothing consumes it
//!   here.
//! * **No asynchronous sense.** Edges are observed on the bus's GPIO snapshot,
//!   so an edge narrower than one snapshot interval is not seen. A real EXTI
//!   latches asynchronously and would catch it.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

/// Window base: `GPIO_S_BASE + 0x400`. Offsets below are relative to THAT, not
/// to the GPIO block, so `0x00` here is `GPIO_EXTIPSELL`.
const OFF_EXTIPSELL: u64 = 0x00;
const OFF_EXTIPSELH: u64 = 0x04;
const OFF_EXTIPINSELL: u64 = 0x08;
const OFF_EXTIPINSELH: u64 = 0x0C;
const OFF_EXTIRISE: u64 = 0x10;
const OFF_EXTIFALL: u64 = 0x14;
const OFF_IF: u64 = 0x20;
const OFF_IEN: u64 = 0x24;
const OFF_EM4WUEN: u64 = 0x2C;
const OFF_EM4WUPOL: u64 = 0x30;

/// `GPIO_EVEN_IRQn` and `GPIO_ODD_IRQn` — CMSIS `IRQn_Type`.
const IRQ_GPIO_ODD: u32 = 39;
const IRQ_GPIO_EVEN: u32 = 40;

/// External interrupt lines on this part.
const EXTI_LINES: u8 = 16;

/// Both `EXTIPSEL` and `EXTIPINSEL` pack one 4-bit field per line, of which
/// the low 2 bits are used.
const FIELD_BITS: u32 = 4;
const FIELD_MASK: u32 = 0x3;

/// EFR32 Series-2 GPIO external interrupts.
#[derive(Debug)]
pub struct Efr32s2GpioExti {
    extipsel: [u32; 2],
    extipinsel: [u32; 2],
    extirise: u32,
    extifall: u32,
    iflag: u32,
    ien: u32,
    em4wuen: u32,
    em4wupol: u32,

    /// Bus-published cycle clock, attached by `SystemBus::add_peripheral`.
    /// `Some` selects scheduler mode; `None` keeps the legacy per-cycle walk
    /// (feature off, or a hand-built test bus that bypasses the registration
    /// choke).
    clock: Option<CycleClock>,
    /// In-flight singleton guard (cancellation contract layer 2): true while a
    /// held-level re-emit event is queued for this model, so a second MMIO
    /// write does not stack a duplicate wake on top of the live chain.
    chain_live: bool,
}

impl Default for Efr32s2GpioExti {
    fn default() -> Self {
        Self::new()
    }
}

impl Efr32s2GpioExti {
    pub fn new() -> Self {
        Self {
            extipsel: [0; 2],
            extipinsel: [0; 2],
            extirise: 0,
            extifall: 0,
            iflag: 0,
            ien: 0,
            em4wuen: 0,
            em4wupol: 0,
            clock: None,
            chain_live: false,
        }
    }

    /// The `(port, pin)` line `n` watches, per RM §24.3.10.1.
    fn watched_pad(&self, line: u8) -> (u8, u8) {
        let half = (line / 8) as usize;
        let shift = ((line % 8) as u32) * FIELD_BITS;
        let port = ((self.extipsel[half] >> shift) & FIELD_MASK) as u8;
        let within = ((self.extipinsel[half] >> shift) & FIELD_MASK) as u8;
        // p = 4 * int(n / 4): the line's group of four fixes the high bits of
        // the pin, and EXTIPINSEL only chooses within that group.
        let pin = 4 * (line / 4) + within;
        (port, pin)
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            OFF_EXTIPSELL => self.extipsel[0],
            OFF_EXTIPSELH => self.extipsel[1],
            OFF_EXTIPINSELL => self.extipinsel[0],
            OFF_EXTIPINSELH => self.extipinsel[1],
            OFF_EXTIRISE => self.extirise,
            OFF_EXTIFALL => self.extifall,
            OFF_IF => self.iflag,
            OFF_IEN => self.ien,
            OFF_EM4WUEN => self.em4wuen,
            OFF_EM4WUPOL => self.em4wupol,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            OFF_EXTIPSELL => self.extipsel[0] = value,
            OFF_EXTIPSELH => self.extipsel[1] = value,
            OFF_EXTIPINSELL => self.extipinsel[0] = value,
            OFF_EXTIPINSELH => self.extipinsel[1] = value,
            OFF_EXTIRISE => self.extirise = value,
            OFF_EXTIFALL => self.extifall = value,
            // Write-1-to-clear, the Series-2 convention. `GPIO_IF_CLR` (the
            // `+0x2000` alias) reaches the same path through the bus's alias
            // fold, which is how `GPIO_IntClear` actually spells it.
            OFF_IF => self.iflag &= !value,
            OFF_IEN => self.ien = value,
            OFF_EM4WUEN => self.em4wuen = value,
            OFF_EM4WUPOL => self.em4wupol = value,
            _ => {}
        }
    }

    /// Which NVIC line a flagged EXTI line raises.
    fn irq_for(line: u8) -> u32 {
        if line % 2 == 0 {
            IRQ_GPIO_EVEN
        } else {
            IRQ_GPIO_ODD
        }
    }

    /// The set of NVIC lines the held level asserts right now — exactly the
    /// list the legacy walk re-emits on every tick. ONE derivation, shared by
    /// the walk and the event chain, so the two routes cannot drift apart.
    fn pending_irqs(&self) -> Vec<u32> {
        let active = self.iflag & self.ien & 0xFFFF;
        if active == 0 {
            return Vec::new();
        }
        // Even and odd lines are two different vectors, and a firmware installs
        // two different handlers. Raise exactly the ones that have a flag.
        let mut irqs = Vec::new();
        for line in 0..EXTI_LINES {
            if active & (1 << line) == 0 {
                continue;
            }
            let irq = Self::irq_for(line);
            if !irqs.contains(&irq) {
                irqs.push(irq);
            }
        }
        irqs
    }

    /// True while any unmasked flag is latched. Outside this window the walk
    /// emits nothing, so the event chain may stop and let the batcher run.
    fn active(&self) -> bool {
        self.iflag & self.ien & 0xFFFF != 0
    }

    /// The level-triggered tick result the legacy walk produces. Shared with
    /// the hardware-oracle forced walk so neither can drift from the other.
    fn level_tick_result(&self) -> PeripheralTickResult {
        let irqs = self.pending_irqs();
        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }

    crate::cycle_clock::scheduler_mode!();

    /// Test/differential knob: detach the clock, pinning the model to the
    /// legacy walk. This is how the walk-vs-scheduler differential builds its
    /// reference lane out of the same bus assembly.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
        self.chain_live = false;
    }
}

impl Peripheral for Efr32s2GpioExti {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) * 8;
        let merged = (self.read_word(reg) & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_word(reg, merged);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    /// Everything `tick_elapsed` can ever do is re-assert a HELD level derived
    /// from `IF & IEN` — no accumulated state, nothing that decays. That is
    /// exactly what the event chain below re-emits, cycle for cycle, so in
    /// scheduler mode the walk has nothing left to contribute. In legacy mode
    /// (feature off / no clock) the walk is still the only thing that raises
    /// the interrupt and the conservative `true` stands.
    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    /// Nothing accumulates between observations: `IF` is latched by
    /// `observe_gpio_change` and cleared by an MMIO write, and `IEN` only
    /// changes under a write. There is no lazy state to bring forward.
    fn sync_to(&mut self, _now_cycle: u64) {}

    /// Arm the held-level re-emit chain the moment the model becomes active.
    ///
    /// Two paths reach this, and BOTH matter:
    ///
    /// * an MMIO write — firmware setting `IEN` over an already-latched `IF`,
    ///   or writing `IF_SET` — which the bus drains through
    ///   `collect_scheduled_events` after every write; and
    /// * a GPIO edge, which is a CROSS-peripheral activation no write choke
    ///   ever sees. The bus harvests here too, but only from the models whose
    ///   `observe_gpio_change` returned `true` — which is why this model's
    ///   returns whether it latched anything.
    ///
    /// delay-0 → deadline `current_cycle + 1`, which is the cycle the walk's
    /// next tick would have emitted on.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.scheduler_mode() && self.active() && !self.chain_live {
            self.chain_live = true;
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
        // Re-emit the held level every cycle while any unmasked flag stays
        // latched — the event-path equivalent of the walk's per-tick
        // `explicit_irqs`. Stop when firmware clears IF (write-1-to-clear) or
        // masks IEN, which is what lets the batcher run again.
        let active = self.active();
        self.chain_live = active;
        crate::sched::EventResult {
            explicit_irqs: self.pending_irqs(),
            reschedule_delay: active.then_some(1),
            ..Default::default()
        }
    }

    /// The bus hands every GPIO TRANSITION here. This is the whole model: an
    /// edge on a pad a line watches, with the matching polarity armed, latches
    /// that line's flag.
    ///
    /// ⚠️ Every tuple in `changes` IS an edge — the bus diffs the ports itself
    /// and adopts the boot levels as a silent baseline, so a pad that merely
    /// STARTS high is never reported. `level` is therefore the direction: 1 is
    /// a rising edge, 0 a falling one.
    ///
    /// Keeping a second previous-level table here does not add safety, it
    /// breaks the model: the bus reports each transition exactly once, so a
    /// local baseline swallows the FIRST edge of every pad — measured, on a
    /// button whose only press never fired.
    /// This model consumes GPIO edges, so the bus must keep its per-cycle
    /// edge-detection pass alive even on a walk-free fast path. See
    /// [`crate::Peripheral::observes_gpio_edges`].
    fn observes_gpio_edges(&self) -> bool {
        true
    }

    fn observe_gpio_change(&mut self, changes: &[(u8, u8, u8)]) -> bool {
        let mut latched = false;
        for &(port, pin, level) in changes {
            let rising = level != 0;

            for line in 0..EXTI_LINES {
                let (sel_port, sel_pin) = self.watched_pad(line);
                if sel_port != port || sel_pin != pin {
                    continue;
                }
                let armed = if rising {
                    self.extirise & (1 << line) != 0
                } else {
                    self.extifall & (1 << line) != 0
                };
                if !armed {
                    continue;
                }
                self.iflag |= 1 << line;
                latched = true;
            }
        }
        latched
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        // A scheduler-mode instance is walk-skipped and the event chain owns
        // the held-level re-emission; the guard is here to keep a stray direct
        // call from double-raising the line.
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        self.level_tick_result()
    }

    /// The bare-CPU hardware oracle freezes the CPU and deliberately asks for
    /// the pre-scheduler one-tick level emission, so the `scheduler_mode()`
    /// no-op in [`Self::tick_elapsed`] must NOT apply here — that guard exists
    /// to catch a stray walk call in production, and a forced tick is the
    /// opposite of stray. Same contract the STM32 `Exti` and the DMA models
    /// already carry, and for the same reason: without it the oracle sees NO
    /// interrupt at all, and only under `cargo test --workspace` (where Cargo
    /// unifies `event-scheduler` on for a crate that never asked for it).
    fn tick_elapsed_forced(&mut self, _cycles: u64) -> PeripheralTickResult {
        self.level_tick_result()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// ⚠️ The mutable accessor DEFAULTS TO `None`. The differential harness
    /// reaches this model mutably to pin it back onto the walk; implementing
    /// only the shared accessor compiles, passes every unit test, and silently
    /// makes the reference lane identical to the candidate lane — a
    /// differential that can no longer fail.
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point line `n` at `(port, pin)`. Panics if the pin is not in the line's
    /// group of four — which is the silicon rule, and a test that got it wrong
    /// should say so rather than quietly watch a different pad.
    fn select(exti: &mut Efr32s2GpioExti, line: u8, port: u8, pin: u8) {
        assert_eq!(
            pin / 4,
            line / 4,
            "EXTI{line} cannot watch pin {pin}: p = 4*int(n/4) restricts it to \
             pins {}..{}",
            4 * (line / 4),
            4 * (line / 4) + 3
        );
        let half = (line / 8) as usize;
        let shift = ((line % 8) as u32) * FIELD_BITS;
        let off_psel = if half == 0 {
            OFF_EXTIPSELL
        } else {
            OFF_EXTIPSELH
        };
        let off_pinsel = if half == 0 {
            OFF_EXTIPINSELL
        } else {
            OFF_EXTIPINSELH
        };
        let psel = exti.read_word(off_psel) | ((port as u32) << shift);
        exti.write_word(off_psel, psel);
        let pinsel = exti.read_word(off_pinsel) | (((pin % 4) as u32) << shift);
        exti.write_word(off_pinsel, pinsel);
    }

    /// Drive one transition. `from` is only there to read as a direction at the
    /// call site — the bus reports transitions, so `to` is what decides the
    /// edge.
    fn edge(exti: &mut Efr32s2GpioExti, port: u8, pin: u8, from: u8, to: u8) {
        debug_assert_ne!(from, to, "a transition must change the level");
        exti.observe_gpio_change(&[(port, pin, to)]);
    }

    #[test]
    fn a_rising_edge_on_a_watched_pad_latches_its_line() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0); // EXTI0 watches PB00 (BTN0 on BRD2709A)
        exti.write_word(OFF_EXTIRISE, 1 << 0);

        edge(&mut exti, 1, 0, 0, 1);
        assert_eq!(exti.read_word(OFF_IF) & 1, 1);
    }

    #[test]
    fn a_line_armed_for_rising_ignores_a_falling_edge() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0);
        exti.write_word(OFF_EXTIRISE, 1 << 0);

        edge(&mut exti, 1, 0, 1, 0);
        assert_eq!(exti.read_word(OFF_IF) & 1, 0);
    }

    /// A button is active-low on this board, so the press is a FALLING edge —
    /// the one `attachInterrupt(..., FALLING)` arms.
    #[test]
    fn a_falling_edge_latches_when_extifall_is_armed() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0);
        exti.write_word(OFF_EXTIFALL, 1 << 0);

        edge(&mut exti, 1, 0, 1, 0);
        assert_eq!(exti.read_word(OFF_IF) & 1, 1);
    }

    #[test]
    fn a_line_armed_for_neither_edge_never_fires() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0);
        edge(&mut exti, 1, 0, 0, 1);
        edge(&mut exti, 1, 0, 1, 0);
        assert_eq!(exti.read_word(OFF_IF), 0);
    }

    /// The port select has to matter: PB00 and PA00 are different pads.
    #[test]
    fn an_edge_on_another_port_does_not_fire_the_line() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0); // PORTB
        exti.write_word(OFF_EXTIRISE, 1 << 0);

        edge(&mut exti, 0, 0, 0, 1); // PA00
        assert_eq!(exti.read_word(OFF_IF) & 1, 0);
    }

    /// The group-of-four rule, which is the part a model is most likely to get
    /// wrong: EXTI4 selects within pins 4..7, so `EXTIPINSEL4 = 1` is PIN5.
    #[test]
    fn the_pin_is_the_group_base_plus_extipinsel() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 4, 2, 5); // EXTI4, PORTC, pin 5 = group base 4 + 1
        assert_eq!(exti.watched_pad(4), (2, 5));
        exti.write_word(OFF_EXTIRISE, 1 << 4);

        edge(&mut exti, 2, 5, 0, 1);
        assert_eq!(exti.read_word(OFF_IF) & (1 << 4), 1 << 4);

        // ...and pin 1, which shares the EXTIPINSEL value but not the group,
        // must not fire it.
        exti.write_word(OFF_IF, 0xFFFF);
        edge(&mut exti, 2, 1, 0, 1);
        assert_eq!(exti.read_word(OFF_IF), 0);
    }

    #[test]
    fn lines_eight_and_up_use_the_high_registers() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 9, 3, 9); // EXTI9, PORTD, pin 9 = group base 8 + 1
        assert_eq!(exti.read_word(OFF_EXTIPSELL), 0, "L is for lines 0..7");
        assert_ne!(exti.read_word(OFF_EXTIPSELH), 0);
        exti.write_word(OFF_EXTIRISE, 1 << 9);

        edge(&mut exti, 3, 9, 0, 1);
        assert_eq!(exti.read_word(OFF_IF) & (1 << 9), 1 << 9);
    }

    #[test]
    fn the_flag_latches_without_ien_and_clears_on_write_one() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0);
        exti.write_word(OFF_EXTIRISE, 1 << 0);

        edge(&mut exti, 1, 0, 0, 1);
        assert_eq!(exti.read_word(OFF_IF) & 1, 1, "polling works without IEN");
        assert!(
            exti.tick_elapsed(1).explicit_irqs.is_none(),
            "IEN clear must not raise an interrupt"
        );

        exti.write_word(OFF_IF, 1);
        assert_eq!(exti.read_word(OFF_IF), 0);
    }

    /// Even and odd lines are two different vectors. A model that raised one
    /// for both runs the wrong handler.
    #[test]
    fn even_and_odd_lines_raise_their_own_vectors() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0); // even
        select(&mut exti, 1, 1, 1); // odd
        exti.write_word(OFF_EXTIRISE, (1 << 0) | (1 << 1));
        exti.write_word(OFF_IEN, 1 << 0);

        edge(&mut exti, 1, 0, 0, 1);
        assert_eq!(
            exti.tick_elapsed(1).explicit_irqs,
            Some(vec![IRQ_GPIO_EVEN]),
            "EXTI0 is even"
        );

        exti.write_word(OFF_IF, 0xFFFF);
        exti.write_word(OFF_IEN, 1 << 1);
        edge(&mut exti, 1, 1, 0, 1);
        assert_eq!(
            exti.tick_elapsed(1).explicit_irqs,
            Some(vec![IRQ_GPIO_ODD]),
            "EXTI1 is odd"
        );
    }

    /// ⚠️ The FIRST transition a pad reports must fire, and this is the test
    /// that says so.
    ///
    /// The bus diffs the ports itself and adopts the boot levels as a silent
    /// baseline, so every tuple it delivers is already an edge. A model that
    /// kept its own previous-level table would treat the first one as its
    /// baseline and swallow it — which is exactly what happened: a button
    /// whose only press was its first never fired its interrupt.
    #[test]
    fn the_first_transition_a_pad_reports_is_an_edge_not_a_baseline() {
        let mut exti = Efr32s2GpioExti::new();
        select(&mut exti, 0, 1, 0);
        exti.write_word(OFF_EXTIFALL, 1 << 0);

        // One tuple, no prior observation of this pad at all.
        assert!(exti.observe_gpio_change(&[(1, 0, 0)]));
        assert_eq!(exti.read_word(OFF_IF) & 1, 1);
    }
}
