// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SYSTIMER peripheral for ESP32-S3.
//!
//! Two 64-bit free-running counters (UNIT0, UNIT1), each clocked at 16 MHz
//! independently of CPU frequency.  Plan 2 implements the counter + the
//! load/update handshake; alarms / IRQs land in Plan 3.
//!
//! ## Register layout (ESP32-S3 TRM §16.5, partial)
//!
//! | Offset | Name              | Behaviour |
//! |-------:|-------------------|-----------|
//! |  0x00  | CONF              | bit 31 clk_en (default 1), bit 30 timer_unit0_work_en, bit 29 timer_unit1_work_en |
//! |  0x04  | UNIT0_OP          | write 1<<30 to trigger snapshot of UNIT0 into VALUE registers |
//! |  0x08  | UNIT1_OP          | same for UNIT1 |
//! |  0x18  | UNIT0_LOAD_HI     | high 32 bits of pending load |
//! |  0x1C  | UNIT0_LOAD_LO     | low 32 bits of pending load |
//! |  0x20  | UNIT1_LOAD_HI     | high 32 bits of pending load (UNIT1) |
//! |  0x24  | UNIT1_LOAD_LO     | low 32 bits of pending load (UNIT1) |
//! |  0x40  | UNIT0_VALUE_HI    | snapshot high 32 bits |
//! |  0x44  | UNIT0_VALUE_LO    | snapshot low 32 bits |
//! |  0x48  | UNIT1_VALUE_HI    | snapshot high 32 bits |
//! |  0x4C  | UNIT1_VALUE_LO    | snapshot low 32 bits |
//! |  0x60  | UNIT0_LOAD        | write 1 to commit pending load into counter |
//! |  0x64  | UNIT1_LOAD        | same for UNIT1 |

use crate::{Peripheral, PeripheralTickResult, SimResult};

const SYSTIMER_CLOCK_HZ: u64 = 16_000_000;

#[derive(Debug, Default, Clone, Copy)]
struct UnitState {
    counter: u64,
    snapshot: u64,
    load_hi: u32,
    load_lo: u32,
}

#[derive(Debug)]
pub struct Systimer {
    conf: u32,
    unit0: UnitState,
    unit1: UnitState,
    cpu_clock_hz: u32,
    /// Accumulated CPU cycles since last counter update; flushed when ≥ 1
    /// SYSTIMER tick worth of CPU cycles have elapsed.
    cpu_cycle_accum: u64,
}

impl Systimer {
    pub fn new(cpu_clock_hz: u32) -> Self {
        Self {
            // Default: clock enabled (bit 31), both units running (bits 30, 29).
            conf: 0xE000_0000,
            unit0: UnitState::default(),
            unit1: UnitState::default(),
            cpu_clock_hz,
            cpu_cycle_accum: 0,
        }
    }

    fn unit0_running(&self) -> bool {
        self.conf & (1 << 30) != 0
    }

    fn unit1_running(&self) -> bool {
        self.conf & (1 << 29) != 0
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.conf,
            0x04 | 0x08 => 0, // OP regs are write-trigger only
            0x18 => self.unit0.load_hi,
            0x1C => self.unit0.load_lo,
            0x20 => self.unit1.load_hi,
            0x24 => self.unit1.load_lo,
            0x40 => (self.unit0.snapshot >> 32) as u32,
            0x44 => (self.unit0.snapshot & 0xFFFF_FFFF) as u32,
            0x48 => (self.unit1.snapshot >> 32) as u32,
            0x4C => (self.unit1.snapshot & 0xFFFF_FFFF) as u32,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.conf = value,
            0x04 => {
                if value & (1 << 30) != 0 {
                    self.unit0.snapshot = self.unit0.counter;
                }
            }
            0x08 => {
                if value & (1 << 30) != 0 {
                    self.unit1.snapshot = self.unit1.counter;
                }
            }
            0x18 => self.unit0.load_hi = value,
            0x1C => self.unit0.load_lo = value,
            0x20 => self.unit1.load_hi = value,
            0x24 => self.unit1.load_lo = value,
            0x60 => {
                if value & 1 != 0 {
                    self.unit0.counter =
                        ((self.unit0.load_hi as u64) << 32) | (self.unit0.load_lo as u64);
                }
            }
            0x64 => {
                if value & 1 != 0 {
                    self.unit1.counter =
                        ((self.unit1.load_hi as u64) << 32) | (self.unit1.load_lo as u64);
                }
            }
            _ => {}
        }
    }
}

impl Peripheral for Systimer {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    /// One CPU cycle elapses per `tick`. Convert to SYSTIMER ticks at 16 MHz.
    /// At 80 MHz CPU clock, 5 CPU cycles == 1 SYSTIMER tick.
    fn tick(&mut self) -> PeripheralTickResult {
        self.cpu_cycle_accum += 1;
        let cpu_per_systimer = (self.cpu_clock_hz as u64).saturating_div(SYSTIMER_CLOCK_HZ).max(1);
        if self.cpu_cycle_accum >= cpu_per_systimer {
            let ticks = self.cpu_cycle_accum / cpu_per_systimer;
            self.cpu_cycle_accum %= cpu_per_systimer;
            if self.unit0_running() {
                self.unit0.counter = self.unit0.counter.wrapping_add(ticks);
            }
            if self.unit1_running() {
                self.unit1.counter = self.unit1.counter.wrapping_add(ticks);
            }
        }
        PeripheralTickResult::default()
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

    #[test]
    fn defaults() {
        let s = Systimer::new(80_000_000);
        assert_eq!(s.conf & 0xE000_0000, 0xE000_0000);
        assert_eq!(s.unit0.counter, 0);
        assert_eq!(s.unit1.counter, 0);
    }

    #[test]
    fn tick_increments_counter_at_correct_rate_80mhz() {
        let mut s = Systimer::new(80_000_000);
        // 80 MHz CPU / 16 MHz SYSTIMER = 5 CPU cycles per SYSTIMER tick.
        for _ in 0..5 {
            s.tick();
        }
        assert_eq!(s.unit0.counter, 1, "after 5 CPU cycles, SYSTIMER += 1");
        for _ in 0..50 {
            s.tick();
        }
        assert_eq!(s.unit0.counter, 11, "55 CPU cycles -> 11 SYSTIMER ticks");
    }

    #[test]
    fn tick_increments_at_240mhz() {
        let mut s = Systimer::new(240_000_000);
        // 240 MHz CPU / 16 MHz SYSTIMER = 15 CPU cycles per SYSTIMER tick.
        for _ in 0..15 {
            s.tick();
        }
        assert_eq!(s.unit0.counter, 1);
    }

    #[test]
    fn op_trigger_snapshots_counter() {
        let mut s = Systimer::new(80_000_000);
        for _ in 0..50 {
            s.tick();
        }
        // Trigger snapshot of UNIT0.
        s.write_word(0x04, 1 << 30);
        let snap_lo = s.read_word(0x44);
        let snap_hi = s.read_word(0x40);
        let combined = ((snap_hi as u64) << 32) | snap_lo as u64;
        assert_eq!(combined, 10);
    }

    #[test]
    fn load_handshake_sets_counter() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x18, 0x0000_0001); // LOAD_HI = 1
        s.write_word(0x1C, 0x0000_0042); // LOAD_LO = 0x42
        s.write_word(0x60, 1); // commit
        assert_eq!(s.unit0.counter, (1u64 << 32) | 0x42);
    }

    #[test]
    fn unit1_independent_of_unit0() {
        let mut s = Systimer::new(80_000_000);
        for _ in 0..5 {
            s.tick();
        }
        assert_eq!(s.unit0.counter, 1);
        assert_eq!(s.unit1.counter, 1, "unit1 ticks alongside unit0");
        s.write_word(0x60, 1); // commit a load to unit0 (loads were 0)
        assert_eq!(s.unit0.counter, 0);
        assert_eq!(s.unit1.counter, 1, "unit1 not affected by unit0 load");
    }

    #[test]
    fn disabled_unit_does_not_tick() {
        let mut s = Systimer::new(80_000_000);
        // Clear bit 30 (unit0 work enable).
        s.write_word(0x00, 0xA000_0000);
        for _ in 0..50 {
            s.tick();
        }
        assert_eq!(s.unit0.counter, 0, "disabled unit must not tick");
        assert_eq!(s.unit1.counter, 10, "unit1 still ticks");
    }
}
