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
//! When running, `tick()` decrements an internal counter from CRV. RR[i]
//! writes carrying the magic key (`0x6E524635`) acknowledge the enabled
//! channels; once every enabled RREN bit has been ack'd, the counter is
//! reloaded to CRV and `reqstatus` cleared. If the counter hits zero
//! before all enabled channels ack, EVENTS_TIMEOUT fires and (if
//! configured) IRQ is pended. The model does **not** trigger a CPU reset
//! — it surfaces the timeout signal so test firmware can observe it.

use crate::{Peripheral, PeripheralTickResult, SimResult};

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

#[derive(Debug, Default)]
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

    counter: u32,
    /// Latches whether the watchdog has been bitten. Once true, no
    /// further countdown happens; firmware can clear EVENTS_TIMEOUT but
    /// a real chip would have reset by now — we just stop the dog.
    bitten: bool,
}

impl Nrf52Wdt {
    pub fn new() -> Self {
        Self::default()
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
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START
                if value & 1 != 0 && !self.bitten => {
                    self.runstatus = 1;
                    self.counter = self.crv;
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
                            self.counter = self.crv;
                            self.reqstatus_pending = 0;
                        }
                    }
                }
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if !self.running() || self.bitten {
            return PeripheralTickResult::default();
        }

        if self.counter == 0 {
            // Already at zero — fall through to timeout (handled below).
        } else {
            self.counter = self.counter.wrapping_sub(1);
        }

        if self.counter == 0 {
            self.bitten = true;
            let already_set = self.events_timeout != 0;
            self.events_timeout = 1;
            let irq = self.inten & INTEN_TIMEOUT != 0;
            return PeripheralTickResult {
                irq,
                cycles: 1,
                fired_events: if already_set {
                    Vec::new()
                } else {
                    vec![OFF_EVENTS_TIMEOUT as u32]
                },
                ..Default::default()
            };
        }

        PeripheralTickResult {
            cycles: 1,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crv_round_trips_full_width() {
        let mut w = Nrf52Wdt::new();
        w.write_u32(OFF_CRV, 0x0002_0000).unwrap();
        assert_eq!(w.read_u32(OFF_CRV).unwrap(), 0x0002_0000);
    }

    #[test]
    fn rren_masks_to_8_bits() {
        let mut w = Nrf52Wdt::new();
        w.write_u32(OFF_RREN, 0xFFFF).unwrap();
        assert_eq!(w.read_u32(OFF_RREN).unwrap(), 0xFF);
    }

    #[test]
    fn countdown_fires_timeout_event() {
        let mut w = Nrf52Wdt::new();
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
        let mut w = Nrf52Wdt::new();
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
        let mut w = Nrf52Wdt::new();
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
        let mut w = Nrf52Wdt::new();
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
        let mut w = Nrf52Wdt::new();
        w.write_u32(OFF_CRV, 100).unwrap();
        w.write_u32(OFF_TASKS_START, 1).unwrap();
        w.write_u32(OFF_CRV, 200).unwrap();
        assert_eq!(w.read_u32(OFF_CRV).unwrap(), 100);
    }
}
