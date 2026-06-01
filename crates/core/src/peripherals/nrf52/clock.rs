// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 POWER + CLOCK peripherals.
//!
//! Source: nRF52840 PS rev 1.7 §6.16 (POWER) and §6.6 (CLOCK).  Both
//! peripherals share base address 0x40000000 on Nordic silicon; their
//! register offsets don't conflict so one model handles the union.
//! They also share a single NVIC IRQ (POWER_CLOCK = 0).
//!
//! Why this matters: Zephyr's nRF clock driver (and most nRF SDK
//! examples) write TASKS_HFCLKSTART and busy-loop on EVENTS_HFCLKSTARTED
//! before letting the kernel proceed.  Without this model, that loop
//! never exits and the system hangs at boot.  We treat the start tasks
//! as instantaneous: the started event fires on the same tick the task
//! is written.

use crate::{Peripheral, PeripheralTickResult, SimResult};

// ── CLOCK register offsets (PS §6.6.13) ──────────────────────────────────────

const OFF_TASKS_HFCLKSTART: u64 = 0x000;
const OFF_TASKS_HFCLKSTOP: u64 = 0x004;
const OFF_TASKS_LFCLKSTART: u64 = 0x008;
const OFF_TASKS_LFCLKSTOP: u64 = 0x00C;
const OFF_TASKS_CAL: u64 = 0x010;
const OFF_TASKS_CTSTART: u64 = 0x014;
const OFF_TASKS_CTSTOP: u64 = 0x018;

const OFF_EVENTS_HFCLKSTARTED: u64 = 0x100;
const OFF_EVENTS_LFCLKSTARTED: u64 = 0x104;
const OFF_EVENTS_DONE: u64 = 0x10C;
const OFF_EVENTS_CTTO: u64 = 0x110;

const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_HFCLKRUN: u64 = 0x408;
const OFF_HFCLKSTAT: u64 = 0x40C;
const OFF_LFCLKRUN: u64 = 0x414;
const OFF_LFCLKSTAT: u64 = 0x418;
const OFF_LFCLKSRC: u64 = 0x518;
const OFF_HFXODEBOUNCE: u64 = 0x528;
const OFF_LFRCMODE: u64 = 0x5B4;

// ── POWER register offsets (PS §6.16.13) ─────────────────────────────────────
//
// POWER tasks/events live above CLOCK's. Where Nordic gave them the same
// offsets in different peripheral windows, here we use disjoint addresses
// because they share the same physical peripheral window.

const OFF_POWER_TASKS_CONSTLAT: u64 = 0x078;
const OFF_POWER_TASKS_LOWPWR: u64 = 0x07C;
const OFF_POWER_EVENTS_POFWARN: u64 = 0x108;
const OFF_POWER_EVENTS_SLEEPENTER: u64 = 0x114;
const OFF_POWER_EVENTS_SLEEPEXIT: u64 = 0x118;
const OFF_POWER_EVENTS_USBDETECTED: u64 = 0x11C;
const OFF_POWER_EVENTS_USBREMOVED: u64 = 0x120;
const OFF_POWER_EVENTS_USBPWRRDY: u64 = 0x124;
const OFF_POWER_RESETREAS: u64 = 0x400;
const OFF_POWER_RAMSTATUS: u64 = 0x428;
const OFF_POWER_USBREGSTATUS: u64 = 0x438;
const OFF_POWER_SYSTEMOFF: u64 = 0x500;
const OFF_POWER_POFCON: u64 = 0x510;
const OFF_POWER_GPREGRET: u64 = 0x51C;
const OFF_POWER_GPREGRET2: u64 = 0x520;
const OFF_POWER_DCDCEN: u64 = 0x578;
const OFF_POWER_DCDCEN0: u64 = 0x580;

// INTEN bits (PS table 23):
//   HFCLKSTARTED bit 0, LFCLKSTARTED bit 1, DONE bit 3, CTTO bit 4
const INTEN_HFCLKSTARTED: u32 = 1 << 0;
const INTEN_LFCLKSTARTED: u32 = 1 << 1;

#[derive(Debug, Default)]
pub struct Nrf52Clock {
    // CLOCK state
    events_hfclkstarted: u32,
    events_lfclkstarted: u32,
    events_done: u32,
    events_ctto: u32,
    inten: u32,
    hfclkrun: u32,
    hfclkstat: u32, // bit 0: SRC (0=RC,1=Xtal), bit 16: STATE (1=running)
    lfclkrun: u32,
    lfclkstat: u32, // bits [1:0]: SRC, bit 16: STATE
    lfclksrc: u32,
    hfxodebounce: u32,
    lfrcmode: u32,

    // Flags raised by tick() on the same call the task was issued.
    pending_hfclk_started: bool,
    pending_lfclk_started: bool,

    // POWER state (register-surface; no dynamic logic).
    power_resetreas: u32,
    power_ramstatus: u32,
    power_usbregstatus: u32,
    power_pofcon: u32,
    power_gpregret: u32,
    power_gpregret2: u32,
    power_dcdcen: u32,
    power_dcdcen0: u32,
}

impl Nrf52Clock {
    pub fn new() -> Self {
        // Reset value of RAMSTATUS = 0xFFFF_FFFF (all RAM blocks on).
        Self {
            power_ramstatus: 0xFFFF_FFFF,
            // RESETREAS is sticky; assume power-on reset.
            power_resetreas: 0x0000_0001,
            ..Self::default()
        }
    }
}

impl Peripheral for Nrf52Clock {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            // CLOCK tasks read as 0.
            OFF_TASKS_HFCLKSTART | OFF_TASKS_HFCLKSTOP | OFF_TASKS_LFCLKSTART
            | OFF_TASKS_LFCLKSTOP | OFF_TASKS_CAL | OFF_TASKS_CTSTART | OFF_TASKS_CTSTOP => 0,

            OFF_EVENTS_HFCLKSTARTED => self.events_hfclkstarted,
            OFF_EVENTS_LFCLKSTARTED => self.events_lfclkstarted,
            OFF_EVENTS_DONE => self.events_done,
            OFF_EVENTS_CTTO => self.events_ctto,

            OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_HFCLKRUN => self.hfclkrun & 1,
            OFF_HFCLKSTAT => self.hfclkstat,
            OFF_LFCLKRUN => self.lfclkrun & 1,
            OFF_LFCLKSTAT => self.lfclkstat,
            OFF_LFCLKSRC => self.lfclksrc & 0x3,
            OFF_HFXODEBOUNCE => self.hfxodebounce,
            OFF_LFRCMODE => self.lfrcmode & 0x1,

            // POWER reads.
            OFF_POWER_TASKS_CONSTLAT | OFF_POWER_TASKS_LOWPWR => 0,
            OFF_POWER_EVENTS_POFWARN
            | OFF_POWER_EVENTS_SLEEPENTER
            | OFF_POWER_EVENTS_SLEEPEXIT
            | OFF_POWER_EVENTS_USBDETECTED
            | OFF_POWER_EVENTS_USBREMOVED
            | OFF_POWER_EVENTS_USBPWRRDY => 0,
            OFF_POWER_RESETREAS => self.power_resetreas,
            OFF_POWER_RAMSTATUS => self.power_ramstatus,
            OFF_POWER_USBREGSTATUS => self.power_usbregstatus,
            OFF_POWER_SYSTEMOFF => 0,
            OFF_POWER_POFCON => self.power_pofcon,
            OFF_POWER_GPREGRET => self.power_gpregret,
            OFF_POWER_GPREGRET2 => self.power_gpregret2,
            OFF_POWER_DCDCEN => self.power_dcdcen,
            OFF_POWER_DCDCEN0 => self.power_dcdcen0,

            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_HFCLKSTART => {
                if value & 1 != 0 {
                    self.hfclkrun = 1;
                    // HFCLKSTAT.STATE = running; SRC = bit 0 reflects LFCLKSRC bits — for
                    // our purposes (Zephyr clock_init), reporting 1<<16 is sufficient.
                    self.hfclkstat = (1 << 16) | 1; // xtal source, running
                    self.pending_hfclk_started = true;
                }
            }
            OFF_TASKS_HFCLKSTOP => {
                if value & 1 != 0 {
                    self.hfclkrun = 0;
                    self.hfclkstat = 0;
                }
            }
            OFF_TASKS_LFCLKSTART => {
                if value & 1 != 0 {
                    self.lfclkrun = 1;
                    self.lfclkstat = (1 << 16) | (self.lfclksrc & 0x3);
                    self.pending_lfclk_started = true;
                }
            }
            OFF_TASKS_LFCLKSTOP => {
                if value & 1 != 0 {
                    self.lfclkrun = 0;
                    self.lfclkstat = 0;
                }
            }
            OFF_TASKS_CAL | OFF_TASKS_CTSTART | OFF_TASKS_CTSTOP => {}

            OFF_EVENTS_HFCLKSTARTED => self.events_hfclkstarted = value & 1,
            OFF_EVENTS_LFCLKSTARTED => self.events_lfclkstarted = value & 1,
            OFF_EVENTS_DONE => self.events_done = value & 1,
            OFF_EVENTS_CTTO => self.events_ctto = value & 1,

            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,

            OFF_LFCLKSRC => self.lfclksrc = value & 0x3,
            OFF_HFXODEBOUNCE => self.hfxodebounce = value & 0xFF,
            OFF_LFRCMODE => self.lfrcmode = value & 0x1,

            // POWER writes (RESETREAS / RAMSTATUS are write-1-to-clear on real silicon;
            // for sim correctness we honor that).
            OFF_POWER_RESETREAS => self.power_resetreas &= !value,
            OFF_POWER_RAMSTATUS => self.power_ramstatus &= !value,
            OFF_POWER_USBREGSTATUS => self.power_usbregstatus = value,
            OFF_POWER_SYSTEMOFF => {} // not modeled; firmware won't recover anyway
            OFF_POWER_POFCON => self.power_pofcon = value,
            OFF_POWER_GPREGRET => self.power_gpregret = value & 0xFF,
            OFF_POWER_GPREGRET2 => self.power_gpregret2 = value & 0xFF,
            OFF_POWER_DCDCEN => self.power_dcdcen = value & 1,
            OFF_POWER_DCDCEN0 => self.power_dcdcen0 = value & 1,

            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let mut irq = false;
        if self.pending_hfclk_started {
            self.pending_hfclk_started = false;
            self.events_hfclkstarted = 1;
            if self.inten & INTEN_HFCLKSTARTED != 0 {
                irq = true;
            }
        }
        if self.pending_lfclk_started {
            self.pending_lfclk_started = false;
            self.events_lfclkstarted = 1;
            if self.inten & INTEN_LFCLKSTARTED != 0 {
                irq = true;
            }
        }

        PeripheralTickResult {
            irq,
            cycles: 1,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hfclkstart_fires_event_on_next_tick() {
        let mut c = Nrf52Clock::new();
        c.write_u32(OFF_TASKS_HFCLKSTART, 1).unwrap();
        c.tick();
        assert_eq!(c.read_u32(OFF_EVENTS_HFCLKSTARTED).unwrap(), 1);
        // HFCLKRUN/STAT reflect the running state.
        assert_eq!(c.read_u32(OFF_HFCLKRUN).unwrap(), 1);
        assert_ne!(c.read_u32(OFF_HFCLKSTAT).unwrap() & (1 << 16), 0);
    }

    #[test]
    fn lfclkstart_fires_event() {
        let mut c = Nrf52Clock::new();
        c.write_u32(OFF_LFCLKSRC, 1).unwrap(); // Xtal
        c.write_u32(OFF_TASKS_LFCLKSTART, 1).unwrap();
        c.tick();
        assert_eq!(c.read_u32(OFF_EVENTS_LFCLKSTARTED).unwrap(), 1);
    }

    #[test]
    fn hfclkstarted_pends_irq_when_enabled() {
        let mut c = Nrf52Clock::new();
        c.write_u32(OFF_INTENSET, INTEN_HFCLKSTARTED).unwrap();
        c.write_u32(OFF_TASKS_HFCLKSTART, 1).unwrap();
        assert!(c.tick().irq);
    }

    #[test]
    fn resetreas_starts_with_power_on_bit_and_clears_on_write() {
        let mut c = Nrf52Clock::new();
        assert_eq!(c.read_u32(OFF_POWER_RESETREAS).unwrap(), 0x1);
        c.write_u32(OFF_POWER_RESETREAS, 0x1).unwrap();
        assert_eq!(c.read_u32(OFF_POWER_RESETREAS).unwrap(), 0);
    }

    #[test]
    fn power_gpregret_round_trips() {
        let mut c = Nrf52Clock::new();
        c.write_u32(OFF_POWER_GPREGRET, 0xA5).unwrap();
        assert_eq!(c.read_u32(OFF_POWER_GPREGRET).unwrap(), 0xA5);
    }
}
