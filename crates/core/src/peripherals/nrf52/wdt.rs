// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 WDT peripheral.
//!
//! Source: nRF52840 PS rev 1.7 §6.34 (WDT). One-shot watchdog: once
//! TASKS_START is written, CRV/RREN/CONFIG become RO. The model doesn't
//! enforce the latch in *register* terms (writes are silently dropped
//! while running), so MMIO diff tests that pin pre-start state see
//! round-trip behavior matching silicon.
//!
//! # Dynamics
//!
//! The counter runs on the 32.768 kHz LFCLK, not on the CPU clock: the
//! timeout period is `(CRV + 1) / 32768` s (PS §6.34.1). The model converts
//! CPU cycles into LFCLK ticks with an exact rational accumulator at the
//! descriptor's core clock, which it takes from [`Peripheral::attach_cpu_hz`]
//! (64 MHz until then, the nRF52 core clock).
//!
//! When running, the counter is loaded with `CRV + 1` LFCLK ticks. RR[i]
//! writes carrying the magic key (`0x6E524635`) acknowledge the enabled
//! channels; once every enabled RREN bit has been ack'd, the counter is
//! reloaded and `reqstatus` cleared. If the counter hits zero before all
//! enabled channels ack, EVENTS_TIMEOUT fires and (if configured) IRQ is
//! pended. The model does **not** trigger a CPU reset
//! — it surfaces the timeout signal so test firmware can observe it.

use crate::peripherals::nrf52::rtc::LFCLK_HZ;
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

const OFF_TASKS_START: u64 = 0x000;
const OFF_EVENTS_TIMEOUT: u64 = 0x100;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_RUNSTATUS: u64 = 0x400;
const OFF_REQSTATUS: u64 = 0x404;
const OFF_CRV: u64 = 0x504;
const OFF_RREN: u64 = 0x508;
const OFF_CONFIG: u64 = 0x50C;
const OFF_RR0: u64 = 0x600;
const OFF_RR7: u64 = 0x61C;

/// Magic value firmware must write to RR[i] to reload the watchdog
/// (PS §6.34.5, table 263).
const RR_RELOAD_KEY: u32 = 0x6E52_4635;

const INTEN_TIMEOUT: u32 = 1;

/// Core clock assumed until [`Peripheral::attach_cpu_hz`] runs: the 64 MHz
/// nRF52 application core.
pub const CPU_HZ_DEFAULT: u64 = 64_000_000;

#[derive(Debug)]
pub struct Nrf52Wdt {
    events_timeout: u32,
    inten: u32,
    runstatus: u32,
    /// Per-RR pending acknowledgment bits: set when firmware writes the
    /// magic key to RR[i]; cleared on reload or timeout.
    reqstatus_pending: u32,
    crv: u32,
    rren: u32,
    config: u32,

    /// LFCLK ticks left before the timeout. `CRV + 1` after start or reload,
    /// so it needs 33 bits.
    counter: u64,
    /// Core clock that drives the CPU-cycle to LFCLK conversion.
    cpu_hz: u64,
    /// Sub-tick LFCLK phase in `cycles * LFCLK_HZ` units; always `< cpu_hz`.
    lf_phase: u64,
    /// Latches whether the watchdog has been bitten. Once true, no
    /// further countdown happens; firmware can clear EVENTS_TIMEOUT but
    /// a real chip would have reset by now — we just stop the dog.
    bitten: bool,
    clock: Option<CycleClock>,
    anchor: u64,
    arm_seq: u32,
}

impl Default for Nrf52Wdt {
    fn default() -> Self {
        Self {
            events_timeout: 0,
            inten: 0,
            runstatus: 0,
            reqstatus_pending: 0,
            crv: 0xFFFF_FFFF, // nRF52840 PS §6.34: CRV reset value
            rren: 0,
            config: 0,
            counter: 0,
            cpu_hz: CPU_HZ_DEFAULT,
            lf_phase: 0,
            bitten: false,
            clock: None,
            anchor: 0,
            arm_seq: 0,
        }
    }
}

impl Nrf52Wdt {
    pub fn new() -> Self {
        Self::default()
    }

    /// A WDT whose LFCLK runs at the CPU clock, one tick per cycle. For
    /// unit tests that count ticks directly.
    pub fn new_fast() -> Self {
        Self {
            cpu_hz: u64::from(LFCLK_HZ),
            ..Self::default()
        }
    }

    /// LFCLK ticks from start or reload to the timeout (PS §6.34.1).
    fn reload_ticks(&self) -> u64 {
        u64::from(self.crv) + 1
    }

    /// CPU cycles until `counter` reaches zero from the current phase.
    fn cycles_until_timeout(&self) -> u64 {
        if self.counter == 0 {
            return 1;
        }
        let need = u128::from(self.counter) * u128::from(self.cpu_hz) - u128::from(self.lf_phase);
        let lf = u128::from(LFCLK_HZ);
        need.div_ceil(lf).clamp(1, u128::from(u64::MAX)) as u64
    }

    fn running(&self) -> bool {
        self.runstatus & 1 != 0
    }

    /// REQSTATUS = bits that still NEED to be ack'd before a reload.
    /// On reset every channel needs ack; once ack'd, bit clears.
    fn reqstatus_view(&self) -> u32 {
        let rren = self.rren & 0xFF;
        // A bit reads 1 if reload is required (enabled AND not pending ack).
        rren & !self.reqstatus_pending
    }
}

impl Peripheral for Nrf52Wdt {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_START => 0,
            OFF_EVENTS_TIMEOUT => self.events_timeout,
            OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_RUNSTATUS => self.runstatus & 1,
            OFF_REQSTATUS => self.reqstatus_view(),
            OFF_CRV => self.crv,
            OFF_RREN => self.rren & 0xFF,
            OFF_CONFIG => self.config & 0x9,
            OFF_RR0..=OFF_RR7 if offset.is_multiple_of(4) => 0, // WO
            _ => {
                crate::census_reg!("nrf52.wdt:Nrf52Wdt", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START
                if value & 1 != 0 && !self.bitten => {
                    self.runstatus = 1;
                    self.counter = self.reload_ticks();
                    self.lf_phase = 0;
                    // On start every enabled channel needs a fresh ack.
                    self.reqstatus_pending = 0;
                }
            OFF_EVENTS_TIMEOUT => self.events_timeout = value & 1,
            OFF_INTENSET => self.inten |= value & 1,
            OFF_INTENCLR => self.inten &= !value,
            OFF_CRV
                // Per PS §6.34.5 CRV is RO once running; drop writes silently.
                if !self.running() => {
                    self.crv = value;
                }
            OFF_RREN
                if !self.running() => {
                    self.rren = value & 0xFF;
                }
            OFF_CONFIG
                if !self.running() => {
                    self.config = value & 0x9;
                }
            OFF_RR0..=OFF_RR7 if offset.is_multiple_of(4)
                && self.running() && value == RR_RELOAD_KEY => {
                    let i = ((offset - OFF_RR0) / 4) as usize;
                    let bit = 1u32 << i;
                    // Only ack channels that are enabled.
                    if self.rren & bit != 0 {
                        self.reqstatus_pending |= bit;
                        // If every enabled RREN bit has been ack'd, reload.
                        if self.reqstatus_pending & self.rren == self.rren {
                            self.counter = self.reload_ticks();
                            self.reqstatus_pending = 0;
                        }
                    }
                }
            _ => { crate::census_reg!("nrf52.wdt:Nrf52Wdt", offset, "write"); }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.advance_cycles(1)
    }

    fn uses_scheduler(&self) -> bool {
        self.clock.is_some()
    }

    fn needs_legacy_walk(&self) -> bool {
        self.clock.is_none()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn attach_cpu_hz(&mut self, hz: u64) {
        if hz == 0 {
            return;
        }
        self.cpu_hz = hz;
        // The phase is in units of the old clock; drop the partial tick.
        self.lf_phase = 0;
    }

    fn sync_to(&mut self, now_cycle: u64) {
        if self.clock.is_none() || now_cycle <= self.anchor {
            return;
        }
        let delta = now_cycle - self.anchor;
        self.anchor = now_cycle;
        let _ = self.advance_cycles(delta);
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.clock.is_none() || !self.running() || self.bitten {
            return Vec::new();
        }
        // Fire when counter would hit 0 (or next cycle if already 0).
        let d = self.cycles_until_timeout();
        self.arm_seq = self.arm_seq.wrapping_add(1);
        vec![(d.saturating_sub(1), self.arm_seq)]
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if self.clock.is_none() || event_token != self.arm_seq {
            return crate::sched::EventResult::default();
        }
        let now = sched.now();
        let res = if now > self.anchor {
            let d = now - self.anchor;
            self.anchor = now;
            self.advance_cycles(d)
        } else {
            self.advance_cycles(1)
        };
        let next = if self.running() && !self.bitten {
            Some(self.cycles_until_timeout())
        } else {
            None
        };
        crate::sched::EventResult {
            raise_own_irq: res.irq,
            fired_events: res.fired_events,
            reschedule_delay: next.map(|d| d.saturating_sub(1)),
            ..Default::default()
        }
    }
}

impl Nrf52Wdt {
    fn advance_cycles(&mut self, cycles: u64) -> PeripheralTickResult {
        if !self.running() || self.bitten || cycles == 0 {
            return PeripheralTickResult::default();
        }
        let mut irq = false;
        let mut fired = Vec::new();
        let total = u128::from(self.lf_phase) + u128::from(cycles) * u128::from(LFCLK_HZ);
        let cpu_hz = u128::from(self.cpu_hz);
        let ticks = total / cpu_hz;
        self.lf_phase = (total % cpu_hz) as u64;
        if ticks < u128::from(self.counter) {
            self.counter -= ticks as u64;
        } else {
            self.counter = 0;
            self.bitten = true;
            let already = self.events_timeout != 0;
            self.events_timeout = 1;
            if self.inten & INTEN_TIMEOUT != 0 {
                irq = true;
            }
            if !already {
                fired.push(OFF_EVENTS_TIMEOUT as u32);
            }
        }
        PeripheralTickResult {
            irq,
            cycles: 1,
            fired_events: fired,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crv_starts_at_reset_value() {
        let w = Nrf52Wdt::new_fast();
        assert_eq!(w.read_u32(OFF_CRV).unwrap(), 0xFFFF_FFFF);
    }

    #[test]
    fn crv_round_trips_full_width_when_stopped() {
        let mut w = Nrf52Wdt::new_fast();
        w.write_u32(OFF_CRV, 0x0002_0000).unwrap();
        assert_eq!(w.read_u32(OFF_CRV).unwrap(), 0x0002_0000);
    }

    #[test]
    fn rren_masks_to_8_bits() {
        let mut w = Nrf52Wdt::new_fast();
        w.write_u32(OFF_RREN, 0xFFFF).unwrap();
        assert_eq!(w.read_u32(OFF_RREN).unwrap(), 0xFF);
    }

    #[test]
    fn countdown_fires_timeout_event() {
        let mut w = Nrf52Wdt::new_fast();
        w.write_u32(OFF_CRV, 5).unwrap();
        w.write_u32(OFF_RREN, 1).unwrap();
        w.write_u32(OFF_INTENSET, 1).unwrap();
        w.write_u32(OFF_TASKS_START, 1).unwrap();

        let mut irq_seen = false;
        for _ in 0..10 {
            if w.tick().irq {
                irq_seen = true;
            }
        }
        assert!(irq_seen, "WDT should pend IRQ on timeout");
        assert_eq!(w.read_u32(OFF_EVENTS_TIMEOUT).unwrap(), 1);
    }

    #[test]
    fn rr_magic_key_reload_extends_countdown() {
        let mut w = Nrf52Wdt::new_fast();
        w.write_u32(OFF_CRV, 10).unwrap();
        w.write_u32(OFF_RREN, 1).unwrap();
        w.write_u32(OFF_TASKS_START, 1).unwrap();

        for _ in 0..8 {
            w.tick(); // counter: 9..2
        }
        // Reload before timeout.
        w.write_u32(OFF_RR0, RR_RELOAD_KEY).unwrap();
        // Should now have ~10 more ticks of life.
        for _ in 0..8 {
            w.tick();
        }
        assert_eq!(w.read_u32(OFF_EVENTS_TIMEOUT).unwrap(), 0);
    }

    #[test]
    fn rr_wrong_key_does_not_reload() {
        let mut w = Nrf52Wdt::new_fast();
        w.write_u32(OFF_CRV, 5).unwrap();
        w.write_u32(OFF_RREN, 1).unwrap();
        w.write_u32(OFF_TASKS_START, 1).unwrap();

        w.tick();
        w.write_u32(OFF_RR0, 0xDEAD_BEEF).unwrap(); // wrong key
        for _ in 0..6 {
            w.tick();
        }
        assert_eq!(w.read_u32(OFF_EVENTS_TIMEOUT).unwrap(), 1);
    }

    #[test]
    fn rr_disabled_channel_ignored() {
        // Two channels enabled (RREN=0b11); writing key to only RR0
        // should NOT cause a reload.
        let mut w = Nrf52Wdt::new_fast();
        w.write_u32(OFF_CRV, 5).unwrap();
        w.write_u32(OFF_RREN, 0b11).unwrap();
        w.write_u32(OFF_TASKS_START, 1).unwrap();

        w.tick(); // counter=4
        w.write_u32(OFF_RR0, RR_RELOAD_KEY).unwrap();
        // Partial ack; reqstatus_view should show ch1 still needing ack.
        assert_eq!(w.read_u32(OFF_REQSTATUS).unwrap(), 0b10);
        // Ack the second channel — full reload.
        w.write_u32(OFF_RR0 + 4, RR_RELOAD_KEY).unwrap();
        assert_eq!(w.read_u32(OFF_REQSTATUS).unwrap(), 0b11);
        // ↑ both pending bits ack'd → reqstatus_pending=0b11 → view returns rren & !pending = 0
        // Hmm — actually reload should clear pending, so view = rren & !0 = 0b11.
        // Wait — the implementation sets pending=0 on reload. So both bits should read 1 (needs ack again).
    }

    #[test]
    fn writes_to_crv_while_running_are_dropped() {
        let mut w = Nrf52Wdt::new_fast();
        w.write_u32(OFF_CRV, 100).unwrap();
        w.write_u32(OFF_TASKS_START, 1).unwrap();
        w.write_u32(OFF_CRV, 200).unwrap();
        assert_eq!(w.read_u32(OFF_CRV).unwrap(), 100);
    }

    /// Cycles until the walk sets EVENTS_TIMEOUT.
    fn walk_until_timeout(w: &mut Nrf52Wdt, limit: u64) -> Option<u64> {
        for n in 1..=limit {
            w.tick();
            if w.read_u32(OFF_EVENTS_TIMEOUT).unwrap() == 1 {
                return Some(n);
            }
        }
        None
    }

    #[test]
    fn timeout_is_crv_plus_one_lfclk_ticks_at_64_mhz() {
        // 33 LFCLK ticks at 64 MHz = 64_453.125 cycles -> cycle 64_454.
        let mut w = Nrf52Wdt::new();
        w.write_u32(OFF_CRV, 32).unwrap();
        w.write_u32(OFF_TASKS_START, 1).unwrap();
        assert_eq!(walk_until_timeout(&mut w, 70_000), Some(64_454));
    }

    #[test]
    fn timeout_follows_attached_core_clock() {
        // nRF51 at 16 MHz: 33 ticks = 16_113.28 cycles -> cycle 16_114.
        let mut w = Nrf52Wdt::new();
        w.attach_cpu_hz(16_000_000);
        w.write_u32(OFF_CRV, 32).unwrap();
        w.write_u32(OFF_TASKS_START, 1).unwrap();
        assert_eq!(walk_until_timeout(&mut w, 20_000), Some(16_114));
    }

    #[test]
    fn zero_core_clock_keeps_previous_ratio() {
        let mut w = Nrf52Wdt::new_fast();
        w.attach_cpu_hz(0);
        w.write_u32(OFF_CRV, 4).unwrap();
        w.write_u32(OFF_TASKS_START, 1).unwrap();
        assert_eq!(walk_until_timeout(&mut w, 10), Some(5));
    }

    #[test]
    fn reload_restores_full_period() {
        let mut w = Nrf52Wdt::new_fast();
        w.write_u32(OFF_CRV, 9).unwrap();
        w.write_u32(OFF_RREN, 1).unwrap();
        w.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..7 {
            w.tick();
        }
        w.write_u32(OFF_RR0, RR_RELOAD_KEY).unwrap();
        // CRV + 1 = 10 ticks from the reload.
        assert_eq!(walk_until_timeout(&mut w, 20), Some(10));
    }

    #[test]
    fn scheduled_timeout_delay_counts_lfclk_ticks() {
        let mut w = Nrf52Wdt::new();
        w.attach_cycle_clock(CycleClock::default());
        w.write_u32(OFF_CRV, 32).unwrap();
        w.write_u32(OFF_TASKS_START, 1).unwrap();
        let ev = w.take_scheduled_events();
        // The scheduler fires at `delay + 1` cycles from now.
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].0 + 1, 64_454);
        // A sync one cycle short leaves the event clear; the next cycle sets it.
        w.sync_to(64_453);
        assert_eq!(w.read_u32(OFF_EVENTS_TIMEOUT).unwrap(), 0);
        w.sync_to(64_454);
        assert_eq!(w.read_u32(OFF_EVENTS_TIMEOUT).unwrap(), 1);
    }

    #[test]
    fn largest_crv_does_not_overflow() {
        let mut w = Nrf52Wdt::new();
        w.write_u32(OFF_TASKS_START, 1).unwrap();
        // CRV reset value 0xFFFF_FFFF: 2^32 LFCLK ticks, about 36.4 hours.
        assert_eq!(w.cycles_until_timeout(), (1u64 << 32) * 64_000_000 / 32_768);
    }
}
