// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Nuvoton NPCX Timer and Watchdog (TWD), the block Renode maps as
//! `Timers.NPCX_TWD` at 0x400D8000 on NVIC 31 of the NPCX9.
//!
//! Sources: Renode `NPCX_TWD.cs` (register map, 32768 Hz clock, lock bits,
//! the 87h/61h/63h stop and unlock sequence, the 5Ch touch, reset on a bad
//! service byte) and Renode `nuvoton_npcx9.repl` (base, NVIC 31).
//!
//! Registers are 16-bit at even offsets: `TWCFG` +0x00, `TWCP` +0x02,
//! `TWDT0` +0x04, `T0CSR` +0x06, `WDCNT` +0x08, `WDSDM` +0x0A, `TWMT0` +0x0C,
//! `TWMWD` +0x0E, `WDCP` +0x10.
//!
//! Modelled:
//! - T0 runs from reset at 32768 Hz / 2^`TWCP.MDIV` (0..10), counting down
//!   from `TWDT0` to 0, so one period is `TWDT0 + 1` ticks. Each terminal
//!   count pulses the IRQ (Renode `IRQ.Blink`) and sets `T0CSR.TC`, which a
//!   read clears. `T0CSR.RST`, a `TWDT0` write and a `TWCP` write restart
//!   the period. `TWMT0` reads the current count;
//! - the watchdog is off until the first `WDCNT` write, which loads the
//!   count and starts it (`WD_RUN`). It counts down once per 2^`WDCP` T0
//!   terminal counts, or with `TWCFG.WDCT0I` once per 2^`WDCP` T0 clock
//!   ticks. At 0 it sets `WDRST_STS` and requests a system reset;
//! - `WDSDM`: 87h, 61h, 63h stops a running watchdog, or clears the four
//!   lock bits when it is stopped; 5Ch with `TWCFG.WDSDME` reloads the count
//!   from `WDCNT`; any other byte outside the sequence resets the system;
//! - lock bits `TWCFG[3:0]` set by writing 1 and clear only through the
//!   unlock sequence or a reset. `LTWCP`, `LTWDT0` and `LWDCNT` make their
//!   registers read 0; a locked `WDCNT` write reloads the old preset.
//!
//! Differences from Renode, each toward the NPCX9 datasheet behaviour:
//! - `TWCP` and `WDCP` read back the written `MDIV` / `WDIV` field (Renode
//!   returns the divider 2^n);
//! - the watchdog decrements once per 2^`WDIV` input ticks (Renode's helper
//!   takes 2^`WDIV` + 1);
//! - the `TWDT0` reset preset is 0xFFFF (Renode initialises its limit to
//!   0xFFFF1, which the 16-bit field cannot hold);
//! - `WDRST_STS` survives the watchdog reset it records (Renode clears it in
//!   the machine reset, so firmware can never see it set);
//! - a completed 87h/61h/63h sequence returns to idle, so a second sequence
//!   is accepted (Renode stays on the third byte and resets on the next 87h).
//!
//! The count is derived from the bus cycle clock. With the event scheduler
//! one live wake fires at each T0 terminal count or the watchdog time-out,
//! whichever is first, so a core idling in WFI fast-forwards to them. The reset itself
//! is drained at an instruction boundary (`take_reset_request`).
//!
//! NOT modelled: `T0CSR.TESDIS` / `WDLTD` (Renode tags them), `TWCFG.LTWCFG`
//! gating `TWCFG` (Renode stores it only), the T0 wake-up path through MIWU,
//! and 32-bit accesses beyond the 16-bit register at the address.

use std::cell::Cell;

use crate::{CycleClock, Peripheral, PeripheralResetCause, PeripheralTickResult, SimResult};

const TWCFG: u64 = 0x00;
const TWCP: u64 = 0x02;
const TWDT0: u64 = 0x04;
const T0CSR: u64 = 0x06;
const WDCNT: u64 = 0x08;
const WDSDM: u64 = 0x0A;
const TWMT0: u64 = 0x0C;
const TWMWD: u64 = 0x0E;
const WDCP: u64 = 0x10;

pub const TWCFG_LTWCFG: u16 = 1 << 0;
pub const TWCFG_LTWCP: u16 = 1 << 1;
pub const TWCFG_LTWDT0: u16 = 1 << 2;
pub const TWCFG_LWDCNT: u16 = 1 << 3;
pub const TWCFG_WDCT0I: u16 = 1 << 4;
pub const TWCFG_WDSDME: u16 = 1 << 5;
const TWCFG_LOCKS: u16 = 0x0F;

pub const T0CSR_RST: u16 = 1 << 0;
pub const T0CSR_TC: u16 = 1 << 1;
pub const T0CSR_WDRST_STS: u16 = 1 << 4;
pub const T0CSR_WD_RUN: u16 = 1 << 5;

/// The NPCX LFCLK that clocks T0 (Renode `DefaultFrequency`).
pub const DEFAULT_LFCLK_HZ: u64 = 32_768;
const T0_PRESET_RESET: u64 = 0xFFFF;
/// `WDCNT` preset after reset (Renode `WatchdogCounterDefaultValue`).
const WD_PRESET_RESET: u8 = 0x0F;
const TOUCH: u8 = 0x5C;
const SEQUENCE: [u8; 3] = [0x87, 0x61, 0x63];

/// NPCX TWD (see the module docs).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NpcxTwd {
    lfclk_hz: u64,
    cpu_hz: u64,
    /// Cycle of T0 tick 0 (the current period start).
    t0_epoch: u64,
    /// `TWDT0` + 1.
    t0_limit: u64,
    mdiv: u8,
    wdiv: u8,
    /// `TWCFG[5:0]`.
    twcfg: u16,
    wdrst_sts: bool,
    wd_enabled: bool,
    /// Watchdog count at `synced_ticks`.
    wd_value: u64,
    /// Watchdog input ticks since the last decrement, at `synced_ticks`.
    wd_phase: u64,
    /// T0 ticks since `t0_epoch` the watchdog state is valid for.
    synced_ticks: u64,
    wd_preset: u8,
    /// Bytes of the 87h/61h/63h sequence seen so far.
    seq: u8,
    /// `TC` carried over from before the last period restart.
    tc_latched: Cell<bool>,
    /// Terminal counts since `t0_epoch` that a `T0CSR` read has consumed.
    tc_seen: Cell<u64>,
    /// Terminal counts since `t0_epoch` already pulsed on the IRQ.
    irq_counts: u64,
    reset_latched: bool,
    /// Cycle at which the running watchdog reaches 0.
    reset_due: Option<u64>,
    /// The time base when no cycle clock is attached (hand-built buses).
    walk_cycles: u64,
    /// Token of the one live scheduler wake; bumped only when a write needs
    /// a wake earlier than the live one, which then fires stale.
    generation: u32,
    /// Cycle of the live wake.
    wake_at: Option<u64>,
    #[serde(skip)]
    pending_events: Vec<(u64, u32)>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl NpcxTwd {
    pub fn new(lfclk_hz: u64) -> Self {
        Self {
            lfclk_hz: lfclk_hz.max(1),
            cpu_hz: 1_000_000,
            t0_epoch: 0,
            t0_limit: T0_PRESET_RESET + 1,
            mdiv: 0,
            wdiv: 0,
            twcfg: 0,
            wdrst_sts: false,
            wd_enabled: false,
            wd_value: u64::from(WD_PRESET_RESET),
            wd_phase: 0,
            synced_ticks: 0,
            wd_preset: WD_PRESET_RESET,
            seq: 0,
            tc_latched: Cell::new(false),
            tc_seen: Cell::new(0),
            irq_counts: 0,
            reset_latched: false,
            reset_due: None,
            walk_cycles: 0,
            generation: 0,
            wake_at: None,
            pending_events: Vec::new(),
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn now(&self) -> u64 {
        self.clock
            .as_ref()
            .map_or(self.walk_cycles, CycleClock::now)
    }

    /// CPU cycles per T0 tick, as a fraction over `lfclk_hz`.
    fn tick_num(&self) -> u128 {
        u128::from(self.cpu_hz.max(1)) << self.mdiv
    }

    /// T0 ticks since `t0_epoch`.
    fn ticks(&self) -> u64 {
        let elapsed = u128::from(self.now().saturating_sub(self.t0_epoch));
        (elapsed * u128::from(self.lfclk_hz) / self.tick_num()) as u64
    }

    /// First cycle at which `ticks()` reaches `tick`.
    fn cycle_of_tick(&self, tick: u64) -> u64 {
        let at = (u128::from(tick) * self.tick_num()).div_ceil(u128::from(self.lfclk_hz));
        (u128::from(self.t0_epoch) + at).min(u128::from(u64::MAX)) as u64
    }

    fn counts(&self, ticks: u64) -> u64 {
        ticks / self.t0_limit
    }

    fn wdct0i(&self) -> bool {
        self.twcfg & TWCFG_WDCT0I != 0
    }

    /// Watchdog input ticks up to T0 tick `ticks`.
    fn wd_src(&self, ticks: u64) -> u64 {
        if self.wdct0i() {
            ticks
        } else {
            self.counts(ticks)
        }
    }

    /// (count, expired) of the watchdog at T0 tick `ticks`.
    fn wd_at(&self, ticks: u64) -> (u64, bool) {
        if !self.wd_enabled {
            return (self.wd_value, false);
        }
        let total = self.wd_phase + self.wd_src(ticks) - self.wd_src(self.synced_ticks);
        let dec = total >> self.wdiv;
        if dec >= self.wd_value.max(1) {
            (0, true)
        } else {
            (self.wd_value - dec, false)
        }
    }

    /// Bring the watchdog state up to now; latch a time-out.
    fn sync(&mut self) {
        let ticks = self.ticks();
        if self.wd_enabled {
            let total = self.wd_phase + self.wd_src(ticks) - self.wd_src(self.synced_ticks);
            let dec = total >> self.wdiv;
            if dec >= self.wd_value.max(1) {
                self.alarm();
            } else {
                self.wd_value -= dec;
                self.wd_phase = total & ((1 << self.wdiv) - 1);
            }
        }
        self.synced_ticks = ticks;
    }

    /// Watchdog time-out or a bad `WDSDM` byte: reset the system.
    fn alarm(&mut self) {
        self.wdrst_sts = true;
        self.reset_latched = true;
        self.wd_value = u64::from(WD_PRESET_RESET);
        self.wd_phase = 0;
        self.reset_due = None;
    }

    fn compute_reset_due(&self) -> Option<u64> {
        if !self.wd_enabled {
            return None;
        }
        let need = (self.wd_value.max(1) << self.wdiv) - self.wd_phase;
        let src = self.wd_src(self.synced_ticks) + need;
        let tick = if self.wdct0i() {
            src
        } else {
            src.saturating_mul(self.t0_limit)
        };
        Some(self.cycle_of_tick(tick))
    }

    /// After a state change: recompute the time-out and the wake.
    fn rearm_watchdog(&mut self) {
        self.reset_due = self.compute_reset_due();
        self.arm();
    }

    /// The next cycle that needs a wake: the next T0 terminal count or the
    /// watchdog time-out, whichever is first.
    fn next_wake(&self) -> u64 {
        let next = (self.counts(self.ticks()) + 1).saturating_mul(self.t0_limit);
        let count = self.cycle_of_tick(next);
        self.reset_due.map_or(count, |due| due.min(count))
    }

    /// Keep ONE live scheduler wake (the scheduler's per-peripheral ceiling):
    /// supersede the live wake only when the next one must come earlier. A
    /// wake that arrives early finds nothing due and re-arms.
    fn arm(&mut self) {
        if !self.scheduler_mode() {
            return;
        }
        let want = self.next_wake().max(self.now() + 1);
        if self.wake_at.is_none_or(|at| want < at || at <= self.now()) {
            self.generation = self.generation.wrapping_add(1);
            self.wake_at = Some(want);
            self.pending_events
                .push((want - self.now(), self.generation));
        }
    }

    /// Start a new T0 period now (`RST`, `TWDT0`, `TWCP`).
    fn restart_t0(&mut self, apply: impl FnOnce(&mut Self)) {
        self.sync();
        let counts = self.counts(self.ticks());
        if counts > self.tc_seen.get() {
            self.tc_latched.set(true);
        }
        self.t0_epoch = self.now();
        self.synced_ticks = 0;
        self.tc_seen.set(0);
        self.irq_counts = 0;
        apply(self);
        self.rearm_watchdog();
    }

    fn tc(&self) -> bool {
        self.tc_latched.get() || self.counts(self.ticks()) > self.tc_seen.get()
    }

    fn consume_tc(&self) {
        self.tc_latched.set(false);
        self.tc_seen.set(self.counts(self.ticks()));
    }

    fn locked(&self, bit: u16) -> bool {
        self.twcfg & bit != 0
    }

    fn read_reg(&self, offset: u64) -> u16 {
        match offset {
            TWCFG => self.twcfg,
            TWCP if self.locked(TWCFG_LTWCP) => 0,
            TWCP => u16::from(self.mdiv),
            TWDT0 if self.locked(TWCFG_LTWDT0) => 0,
            TWDT0 => (self.t0_limit - 1) as u16,
            T0CSR => {
                (if self.wd_enabled { T0CSR_WD_RUN } else { 0 })
                    | (if self.wdrst_sts { T0CSR_WDRST_STS } else { 0 })
                    | (if self.tc() { T0CSR_TC } else { 0 })
            }
            WDCNT if self.locked(TWCFG_LWDCNT) => 0,
            WDCNT => u16::from(self.wd_preset),
            WDSDM => 0,
            TWMT0 => {
                let ticks = self.ticks();
                (self.t0_limit - 1 - ticks % self.t0_limit) as u16
            }
            TWMWD => (self.wd_at(self.ticks()).0 & 0xFF) as u16,
            WDCP if self.locked(TWCFG_LTWCP) => 0,
            WDCP => u16::from(self.wdiv),
            _ => {
                crate::census_reg!("npcx_twd", offset, "read");
                0
            }
        }
    }

    /// A read through the bus: a `T0CSR` read that covers `TC` clears it.
    fn read_consuming(&self, offset: u64) -> u16 {
        let value = self.read_reg(offset);
        if offset == T0CSR {
            self.consume_tc();
        }
        value
    }

    fn write_reg(&mut self, offset: u64, value: u16) {
        match offset {
            TWCFG => {
                self.sync();
                // Lock bits only set; WDCT0I and WDSDME follow the write.
                self.twcfg = (self.twcfg & TWCFG_LOCKS) | (value & 0x3F);
                self.rearm_watchdog();
            }
            TWCP => {
                let mdiv = (value & 0xF) as u8;
                if !self.locked(TWCFG_LTWCP) && mdiv <= 10 {
                    self.restart_t0(|t| t.mdiv = mdiv);
                }
            }
            TWDT0 if !self.locked(TWCFG_LTWDT0) => {
                self.restart_t0(|t| t.t0_limit = u64::from(value) + 1);
            }
            T0CSR => {
                if value & T0CSR_WDRST_STS != 0 {
                    self.wdrst_sts = false;
                }
                if value & T0CSR_RST != 0 {
                    self.restart_t0(|_| {});
                }
            }
            WDCNT => {
                self.sync();
                let value = value as u8;
                self.wd_value = if self.locked(TWCFG_LWDCNT) {
                    u64::from(self.wd_preset)
                } else {
                    u64::from(value)
                };
                self.wd_preset = value;
                self.wd_enabled = true;
                self.rearm_watchdog();
            }
            WDSDM => self.write_wdsdm(value as u8),
            WDCP if !self.locked(TWCFG_LTWCP) => {
                self.sync();
                self.wdiv = (value & 0xF) as u8;
                self.wd_phase = 0;
                self.rearm_watchdog();
            }
            // Locked writes are dropped; the counters are read-only.
            TWDT0 | WDCP | TWMT0 | TWMWD => {}
            _ => crate::census_reg!("npcx_twd", offset, "write"),
        }
    }

    fn write_wdsdm(&mut self, data: u8) {
        self.sync();
        self.seq = if data == SEQUENCE[usize::from(self.seq)] {
            self.seq + 1
        } else {
            0
        };
        if self.seq == 3 {
            self.seq = 0;
            if self.wd_enabled {
                self.wd_enabled = false;
            } else {
                self.twcfg &= !TWCFG_LOCKS;
            }
        } else if self.twcfg & TWCFG_WDSDME != 0 && data == TOUCH {
            self.wd_value = u64::from(self.wd_preset);
        } else if self.seq == 0 {
            self.alarm();
        }
        self.rearm_watchdog();
    }

    /// Power-on state; `WDRST_STS` and the T0 event chain survive.
    fn power_on(&mut self) {
        let keep = (self.wdrst_sts, self.generation, self.lfclk_hz, self.cpu_hz);
        let wake_at = self.wake_at;
        let clock = self.clock.take();
        let walk = self.walk_cycles;
        *self = Self::new(keep.2);
        self.cpu_hz = keep.3;
        self.wdrst_sts = keep.0;
        self.generation = keep.1;
        self.wake_at = wake_at;
        self.walk_cycles = walk;
        self.clock = clock;
        self.t0_epoch = self.now();
    }
}

impl Peripheral for NpcxTwd {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !1;
        let value = if offset & 1 == 0 {
            self.read_consuming(reg)
        } else {
            self.read_reg(reg)
        };
        Ok((value >> ((offset & 1) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !1;
        let current = self.read_reg(reg);
        match (reg, offset & 1) {
            // TWDT0 is the only 16-bit field; merge the other byte.
            (TWDT0, 0) => self.write_reg(reg, (current & 0xFF00) | u16::from(value)),
            (TWDT0, _) => self.write_reg(reg, (current & 0x00FF) | (u16::from(value) << 8)),
            // The upper byte of every other register is reserved.
            (_, 0) => self.write_reg(reg, u16::from(value)),
            _ => {}
        }
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(self.read_consuming(offset & !1))
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write_reg(offset & !1, value);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let reg = offset & !1;
        Ok(u32::from(self.read_consuming(reg)) | (u32::from(self.read_reg(reg + 2)) << 16))
    }

    /// Renode has no double-word translation for this block; only the
    /// register at the address takes the low half.
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !1, value as u16);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some((self.read_reg(offset & !1) >> ((offset & 1) * 8)) as u8)
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    /// T0 runs from reset, so the walk is needed whenever there is no
    /// scheduler to deliver its terminal counts.
    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode()
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        if self.clock.is_none() {
            self.walk_cycles = self.walk_cycles.wrapping_add(cycles);
        }
        let counts = self.counts(self.ticks());
        let irq = counts > self.irq_counts;
        self.irq_counts = counts;
        PeripheralTickResult::with_irq(irq)
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        std::mem::take(&mut self.pending_events)
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token != self.generation {
            return crate::sched::EventResult::default();
        }
        // A due watchdog time-out is drained at the next instruction
        // boundary (`take_reset_request`).
        let counts = self.counts(self.ticks());
        let raise = counts > self.irq_counts;
        self.irq_counts = self.irq_counts.max(counts);
        let now = self.now();
        let next = self.next_wake().max(now + 1);
        self.wake_at = Some(next);
        crate::sched::EventResult {
            raise_own_irq: raise,
            reschedule_delay: Some(next - now),
            ..Default::default()
        }
    }

    fn reset_request_capable(&self) -> bool {
        true
    }

    fn latched_reset_cause(&self) -> Option<PeripheralResetCause> {
        let due = self.reset_due.is_some_and(|due| self.now() >= due);
        (self.reset_latched || due).then_some(PeripheralResetCause::Watchdog)
    }

    fn take_reset_request(&mut self) -> bool {
        if self.reset_due.is_some_and(|due| self.now() >= due) {
            self.sync();
        }
        std::mem::take(&mut self.reset_latched)
    }

    fn on_system_reset(&mut self) {
        self.power_on();
    }

    fn on_node_reset(&mut self) {
        self.power_on();
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.t0_epoch = clock.now();
        self.synced_ticks = 0;
        self.clock = Some(clock);
        self.wake_at = None;
        self.arm();
    }

    fn attach_cpu_hz(&mut self, hz: u64) {
        self.cpu_hz = hz.max(1);
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One CPU cycle per T0 tick at MDIV 0.
    fn twd() -> NpcxTwd {
        let mut t = NpcxTwd::new(DEFAULT_LFCLK_HZ);
        t.attach_cpu_hz(DEFAULT_LFCLK_HZ);
        t
    }

    fn rd(t: &NpcxTwd, off: u64) -> u16 {
        t.read_u16(off).unwrap()
    }

    fn wr(t: &mut NpcxTwd, off: u64, v: u16) {
        t.write_u16(off, v).unwrap();
    }

    #[test]
    fn reset_values() {
        let t = twd();
        assert_eq!(rd(&t, TWCFG), 0);
        assert_eq!(rd(&t, TWCP), 0);
        assert_eq!(rd(&t, TWDT0), 0xFFFF);
        assert_eq!(rd(&t, T0CSR), 0);
        assert_eq!(rd(&t, WDCNT), 0x0F);
        assert_eq!(rd(&t, TWMT0), 0xFFFF);
        assert_eq!(rd(&t, WDCP), 0);
    }

    #[test]
    fn t0_counts_down_and_pulses_once_per_period() {
        let mut t = twd();
        wr(&mut t, TWDT0, 9);
        t.tick_elapsed(3);
        assert_eq!(rd(&t, TWMT0), 6);
        assert!(!t.tick_elapsed(6).irq);
        assert_eq!(rd(&t, TWMT0), 0);
        assert!(t.tick_elapsed(1).irq, "terminal count");
        assert_eq!(rd(&t, TWMT0), 9);
        assert!(!t.tick_elapsed(9).irq, "a pulse, not a level");
        assert!(t.tick_elapsed(1).irq);
    }

    #[test]
    fn mdiv_divides_the_clock_and_reads_back_the_field() {
        let mut t = twd();
        wr(&mut t, TWDT0, 0);
        wr(&mut t, TWCP, 3);
        assert_eq!(rd(&t, TWCP), 3);
        assert!(!t.tick_elapsed(7).irq);
        assert!(t.tick_elapsed(1).irq, "8 cycles per tick");
        wr(&mut t, TWCP, 11);
        assert_eq!(rd(&t, TWCP), 3, "MDIV above 10 is ignored");
    }

    #[test]
    fn tc_is_cleared_by_the_read() {
        let mut t = twd();
        wr(&mut t, TWDT0, 4);
        t.tick_elapsed(5);
        assert_eq!(t.peek(T0CSR), Some(T0CSR_TC as u8), "peek keeps TC");
        assert_eq!(rd(&t, T0CSR), T0CSR_TC);
        assert_eq!(rd(&t, T0CSR), 0);
        // A restart keeps an unread TC.
        t.tick_elapsed(5);
        wr(&mut t, T0CSR, T0CSR_RST);
        assert_eq!(rd(&t, T0CSR), T0CSR_TC);
        assert_eq!(rd(&t, TWMT0), 4, "RST restarts the period");
    }

    #[test]
    fn watchdog_counts_t0_periods_and_resets() {
        let mut t = twd();
        wr(&mut t, TWDT0, 9);
        wr(&mut t, WDCNT, 3);
        assert_eq!(rd(&t, T0CSR) & T0CSR_WD_RUN, T0CSR_WD_RUN);
        t.tick_elapsed(10);
        assert_eq!(rd(&t, TWMWD), 2);
        t.tick_elapsed(19);
        assert_eq!(rd(&t, TWMWD), 1);
        assert_eq!(t.latched_reset_cause(), None);
        assert!(!t.take_reset_request());
        t.tick_elapsed(1);
        assert_eq!(
            t.latched_reset_cause(),
            Some(PeripheralResetCause::Watchdog)
        );
        assert!(t.take_reset_request());
        assert!(!t.take_reset_request(), "taken once");
        t.on_system_reset();
        assert_eq!(rd(&t, T0CSR), T0CSR_WDRST_STS, "WD_RUN clear, status kept");
        wr(&mut t, T0CSR, T0CSR_WDRST_STS);
        assert_eq!(rd(&t, T0CSR), 0);
        assert_eq!(rd(&t, TWDT0), 0xFFFF);
    }

    #[test]
    fn wdct0i_and_wdcp_clock_the_watchdog_from_the_prescaled_clock() {
        let mut t = twd();
        wr(&mut t, TWCFG, TWCFG_WDCT0I);
        wr(&mut t, WDCP, 2);
        wr(&mut t, WDCNT, 2);
        t.tick_elapsed(4);
        assert_eq!(rd(&t, TWMWD), 1);
        t.tick_elapsed(3);
        assert!(!t.take_reset_request());
        t.tick_elapsed(1);
        assert!(t.take_reset_request());
    }

    #[test]
    fn touch_reloads_and_the_sequence_stops_then_unlocks() {
        let mut t = twd();
        wr(&mut t, TWDT0, 0);
        wr(&mut t, TWCFG, TWCFG_WDSDME);
        wr(&mut t, WDCNT, 5);
        t.tick_elapsed(3);
        assert_eq!(rd(&t, TWMWD), 2);
        wr(&mut t, WDSDM, u16::from(TOUCH));
        assert_eq!(rd(&t, TWMWD), 5);
        for b in SEQUENCE {
            wr(&mut t, WDSDM, u16::from(b));
        }
        assert_eq!(rd(&t, T0CSR) & T0CSR_WD_RUN, 0, "stopped");
        t.tick_elapsed(100);
        assert!(!t.take_reset_request());
        // Locks, then unlock with the watchdog stopped.
        wr(&mut t, TWCFG, TWCFG_LTWDT0 | TWCFG_LTWCP);
        assert_eq!(rd(&t, TWCFG), TWCFG_LTWDT0 | TWCFG_LTWCP);
        wr(&mut t, TWDT0, 7);
        assert_eq!(rd(&t, TWDT0), 0, "locked reads 0");
        wr(&mut t, TWCFG, 0);
        assert_eq!(rd(&t, TWCFG), TWCFG_LTWDT0 | TWCFG_LTWCP, "set-only");
        for b in SEQUENCE {
            wr(&mut t, WDSDM, u16::from(b));
        }
        assert_eq!(rd(&t, TWCFG), 0);
        assert_eq!(rd(&t, TWDT0), 0, "the locked write was dropped");
        assert!(!t.take_reset_request());
    }

    #[test]
    fn a_bad_service_byte_resets_at_once() {
        let mut t = twd();
        wr(&mut t, WDSDM, 0x87);
        wr(&mut t, WDSDM, 0x61);
        assert!(!t.take_reset_request(), "inside the sequence");
        wr(&mut t, WDSDM, 0x55);
        assert!(t.take_reset_request());
        assert_eq!(rd(&t, T0CSR) & T0CSR_WDRST_STS, T0CSR_WDRST_STS);
        // Without WDSDME, 5Ch is a bad byte too.
        wr(&mut t, WDSDM, u16::from(TOUCH));
        assert!(t.take_reset_request());
    }

    #[test]
    fn locked_wdcnt_reloads_the_old_preset() {
        let mut t = twd();
        wr(&mut t, TWDT0, 0);
        wr(&mut t, WDCNT, 8);
        wr(&mut t, TWCFG, TWCFG_LWDCNT);
        wr(&mut t, WDCNT, 3);
        assert_eq!(rd(&t, TWMWD), 8);
        assert_eq!(rd(&t, WDCNT), 0);
    }

    #[test]
    fn byte_stores_reach_the_low_byte_only() {
        let mut t = twd();
        t.write(TWDT0, 0x34).unwrap();
        t.write(TWDT0 + 1, 0x12).unwrap();
        assert_eq!(rd(&t, TWDT0), 0x1234);
        // The upper byte of WDSDM is reserved: no service byte, no reset.
        t.write(WDSDM + 1, 0x00).unwrap();
        assert!(!t.take_reset_request());
    }

    #[test]
    fn counts_from_the_cycle_clock_and_schedules_the_chain() {
        let clock = CycleClock::default();
        clock.publish(100);
        let mut t = NpcxTwd::new(DEFAULT_LFCLK_HZ);
        // 15 MHz: 457.76 cycles per tick.
        t.attach_cpu_hz(15_000_000);
        t.attach_cycle_clock(clock.clone());
        let boot = t.take_scheduled_events();
        wr(&mut t, TWDT0, 31);
        let events = t.take_scheduled_events();
        // 32 ticks = 32 * 15e6 / 32768 = 14,648.4 cycles.
        if t.scheduler_mode() {
            assert_eq!(boot.len(), 1);
            assert_eq!(events, vec![(14_649, t.generation)]);
        } else {
            assert!(boot.is_empty() && events.is_empty());
        }
        clock.publish(100 + 14_648);
        assert_eq!(rd(&t, TWMT0), 0);
        clock.publish(100 + 14_649);
        assert_eq!(rd(&t, TWMT0), 31);
        let mut sched = crate::sched::EventScheduler::new();
        let mut bus = crate::bus::SystemBus::new();
        let gen = t.generation;
        let r = t.on_event(gen, &mut sched, &mut bus);
        assert!(r.raise_own_irq);
        assert!(!t.on_event(gen, &mut sched, &mut bus).raise_own_irq, "once");
        assert!(
            !t.on_event(gen.wrapping_sub(1), &mut sched, &mut bus)
                .raise_own_irq
        );
        // Watchdog: 2 periods from the write.
        wr(&mut t, WDCNT, 2);
        assert_eq!(t.reset_due, Some(100 + 14_649 + 29_297));
        clock.publish(100 + 14_649 + 29_296);
        assert!(!t.take_reset_request());
        clock.publish(100 + 14_649 + 29_297);
        assert!(t.take_reset_request());
    }

    #[test]
    fn service_writes_keep_one_live_wake() {
        let clock = CycleClock::default();
        let mut t = twd();
        t.attach_cycle_clock(clock.clone());
        wr(&mut t, TWDT0, 31);
        wr(&mut t, TWCFG, TWCFG_WDSDME);
        wr(&mut t, WDCNT, 10);
        let _ = t.take_scheduled_events();
        for _ in 0..20 {
            wr(&mut t, WDSDM, u16::from(TOUCH));
        }
        for b in SEQUENCE {
            wr(&mut t, WDSDM, u16::from(b));
        }
        assert!(
            t.take_scheduled_events().is_empty(),
            "no earlier wake needed"
        );
        // WDCT0I: the time-out (4 ticks) is due before the next count (32).
        wr(&mut t, TWCFG, TWCFG_WDCT0I);
        wr(&mut t, WDCNT, 4);
        let events = t.take_scheduled_events();
        if t.scheduler_mode() {
            assert_eq!(events, vec![(4, t.generation)]);
        } else {
            assert!(events.is_empty());
        }
    }
}
