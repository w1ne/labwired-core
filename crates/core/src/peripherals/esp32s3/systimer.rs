// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SYSTIMER peripheral for ESP32-S3.
//!
//! Two 64-bit free-running counters (UNIT0, UNIT1), each clocked at 16 MHz
//! independently of CPU frequency.  Plan 2 implemented the counter + the
//! load/update handshake; Plan 3 adds 3 UNIT0 alarms with IRQ delivery.
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
//! |  0x70  | ALARM0_TARGET_HI  | high 32 bits of UNIT0 alarm 0 target |
//! |  0x74  | ALARM0_TARGET_LO  | low  32 bits of UNIT0 alarm 0 target |
//! |  0x78  | ALARM0_CONF       | bit 31 auto_reload, bit 30 enable, bits[25:0] period |
//! |  0x7C  | ALARM1_TARGET_HI  | UNIT0 alarm 1 target high |
//! |  0x80  | ALARM1_TARGET_LO  | UNIT0 alarm 1 target low |
//! |  0x84  | ALARM1_CONF       | UNIT0 alarm 1 config |
//! |  0x88  | ALARM2_TARGET_HI  | UNIT0 alarm 2 target high |
//! |  0x8C  | ALARM2_TARGET_LO  | UNIT0 alarm 2 target low |
//! |  0x90  | ALARM2_CONF       | UNIT0 alarm 2 config |
//! |  0x94  | INT_ENA           | bits 0/1/2 — enable IRQ for TARGET0/1/2 |
//! |  0x98  | INT_RAW           | bits 0/1/2 — pending bit per alarm (RO) |
//! |  0x9C  | INT_CLR           | write-1-to-clear pending bits |
//! |  0xA0  | INT_ST            | INT_RAW & INT_ENA (RO) |
//!
//! ## Notes on offset choice
//!
//! The TRM places the alarm-target/config registers earlier in the block
//! (overlapping our existing UNIT0_LOAD_HI/LO etc. layout, which itself is a
//! Plan 2 simplification). To avoid colliding with those committed offsets
//! we relocate the alarm window to 0x70-0xA0. Plan 3's demo firmware writes
//! these registers via thin helpers, so the absolute addresses are an
//! internal contract between this peripheral and the demo, not a request
//! to match TRM byte-for-byte.
//!
//! ## Source IDs (ESP32-S3 TRM §9.4)
//!
//! Alarms emit interrupt-matrix source IDs via
//! `PeripheralTickResult.explicit_irqs`:
//!
//! * TARGET0 → source 79
//! * TARGET1 → source 80
//! * TARGET2 → source 81
//!
//! UNIT1 alarms are deferred (Plan 3.5) — the demo only routes UNIT0.

use crate::{Peripheral, PeripheralTickResult, SimResult};

const SYSTIMER_CLOCK_HZ: u64 = 16_000_000;

/// SYSTIMER interrupt-matrix source IDs (ESP32-S3 TRM §9.4). Shared between
/// UNIT0 and UNIT1 on real silicon — Plan 3 only fires UNIT0 alarms.
const SYSTIMER_TARGET0_SOURCE: u32 = 79;

/// Mask for the 26-bit `period` field in ALARM_CONF.
const ALARM_PERIOD_MASK: u32 = 0x03FF_FFFF;
const ALARM_AUTO_RELOAD_BIT: u32 = 1 << 31;
const ALARM_ENABLE_BIT: u32 = 1 << 30;

/// Compose the ALARM_CONF readback word for a single alarm slot.
fn alarm_conf_word(alarm: &AlarmState) -> u32 {
    let mut v = (alarm.period as u32) & ALARM_PERIOD_MASK;
    if alarm.auto_reload {
        v |= ALARM_AUTO_RELOAD_BIT;
    }
    if alarm.enabled {
        v |= ALARM_ENABLE_BIT;
    }
    v
}

#[derive(Debug, Default, Clone, Copy)]
struct UnitState {
    counter: u64,
    snapshot: u64,
    load_hi: u32,
    load_lo: u32,
}

/// Per-alarm state. One instance per UNIT0 alarm slot (Plan 3 ships 3).
#[derive(Debug, Default, Clone, Copy)]
struct AlarmState {
    /// 64-bit comparison target. Alarm fires when `counter >= target`.
    target: u64,
    /// INT_RAW pending bit. Sticky until INT_CLR clears it.
    pending: bool,
    /// ALARM_CONF bit 30. Cleared at reset; firmware sets to arm the alarm.
    enabled: bool,
    /// ALARM_CONF bit 31. When set with non-zero `period`, on fire we bump
    /// `target += period` so the next compare schedules the next event.
    auto_reload: bool,
    /// ALARM_CONF bits[25:0]. Reload step in SYSTIMER ticks.
    period: u64,
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
    /// Three UNIT0 alarms (Plan 3). UNIT1 alarms are Plan 3.5; the demo
    /// only exercises UNIT0_ALARM0.
    unit0_alarms: [AlarmState; 3],
    /// INT_ENA: bits 0/1/2 enable IRQ delivery for UNIT0 alarms 0/1/2.
    /// Pending bits in INT_RAW set regardless of INT_ENA; only IRQ
    /// emission via `explicit_irqs` is gated by these bits.
    int_ena: u32,
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
            unit0_alarms: [AlarmState::default(); 3],
            int_ena: 0,
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
            // OP regs: real silicon asserts bit 29 (TIMER_UNITn_VALUE_VALID)
            // once the requested snapshot has settled (typically a few cycles
            // after the bit-30 trigger write). esp-hal's Delay loop polls
            // bit 29 to wait for the snapshot to be ready before reading the
            // VALUE registers. We model the snapshot as instantaneous, so
            // bit 29 reads back as set whenever a snapshot has ever been
            // taken — i.e. the unitN_snapshot field is non-zero, OR the
            // counter has advanced past zero (meaning a snapshot can be
            // produced on demand). Simpler: always assert bit 29.
            0x04 | 0x08 => 1u32 << 29,
            0x18 => self.unit0.load_hi,
            0x1C => self.unit0.load_lo,
            0x20 => self.unit1.load_hi,
            0x24 => self.unit1.load_lo,
            0x40 => (self.unit0.snapshot >> 32) as u32,
            0x44 => (self.unit0.snapshot & 0xFFFF_FFFF) as u32,
            0x48 => (self.unit1.snapshot >> 32) as u32,
            0x4C => (self.unit1.snapshot & 0xFFFF_FFFF) as u32,

            // Alarm registers (Plan 3, Task 4). Layout:
            //   0x70+12*i+0: ALARMi_TARGET_HI
            //   0x70+12*i+4: ALARMi_TARGET_LO
            //   0x70+12*i+8: ALARMi_CONF (bit 31 auto_reload, bit 30 enable, bits[25:0] period)
            0x70 => (self.unit0_alarms[0].target >> 32) as u32,
            0x74 => (self.unit0_alarms[0].target & 0xFFFF_FFFF) as u32,
            0x78 => alarm_conf_word(&self.unit0_alarms[0]),
            0x7C => (self.unit0_alarms[1].target >> 32) as u32,
            0x80 => (self.unit0_alarms[1].target & 0xFFFF_FFFF) as u32,
            0x84 => alarm_conf_word(&self.unit0_alarms[1]),
            0x88 => (self.unit0_alarms[2].target >> 32) as u32,
            0x8C => (self.unit0_alarms[2].target & 0xFFFF_FFFF) as u32,
            0x90 => alarm_conf_word(&self.unit0_alarms[2]),

            0x94 => self.int_ena,
            0x98 => self.int_raw_word(),
            0xA0 => self.int_raw_word() & self.int_ena,

            _ => 0,
        }
    }

    /// INT_RAW (0x98): bits 0/1/2 = pending bit per UNIT0 alarm.
    fn int_raw_word(&self) -> u32 {
        let mut v = 0u32;
        for (i, alarm) in self.unit0_alarms.iter().enumerate() {
            if alarm.pending {
                v |= 1 << i;
            }
        }
        v
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

            // ── Alarm registers (Plan 3, Task 4) ──
            0x70 => set_alarm_target_hi(&mut self.unit0_alarms[0], value),
            0x74 => set_alarm_target_lo(&mut self.unit0_alarms[0], value),
            0x78 => set_alarm_conf(&mut self.unit0_alarms[0], value),
            0x7C => set_alarm_target_hi(&mut self.unit0_alarms[1], value),
            0x80 => set_alarm_target_lo(&mut self.unit0_alarms[1], value),
            0x84 => set_alarm_conf(&mut self.unit0_alarms[1], value),
            0x88 => set_alarm_target_hi(&mut self.unit0_alarms[2], value),
            0x8C => set_alarm_target_lo(&mut self.unit0_alarms[2], value),
            0x90 => set_alarm_conf(&mut self.unit0_alarms[2], value),

            0x94 => self.int_ena = value & 0x7,
            // 0x98 INT_RAW is read-only on real silicon; ignore writes.
            0x9C => {
                // INT_CLR: write-1-to-clear pending bits.
                for (i, alarm) in self.unit0_alarms.iter_mut().enumerate() {
                    if value & (1 << i) != 0 {
                        alarm.pending = false;
                    }
                }
            }
            // 0xA0 INT_ST is read-only.
            _ => {}
        }
    }
}

fn set_alarm_target_hi(alarm: &mut AlarmState, value: u32) {
    let lo = alarm.target & 0xFFFF_FFFF;
    alarm.target = ((value as u64) << 32) | lo;
}

fn set_alarm_target_lo(alarm: &mut AlarmState, value: u32) {
    let hi = alarm.target & 0xFFFF_FFFF_0000_0000;
    alarm.target = hi | (value as u64);
}

fn set_alarm_conf(alarm: &mut AlarmState, value: u32) {
    alarm.period = (value & ALARM_PERIOD_MASK) as u64;
    alarm.auto_reload = value & ALARM_AUTO_RELOAD_BIT != 0;
    alarm.enabled = value & ALARM_ENABLE_BIT != 0;
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

        // ── Alarm checks (Plan 3, Task 4) ──
        // For each enabled UNIT0 alarm, on the rising edge from
        // `counter < target` to `counter >= target` we set the pending bit.
        // If the matching INT_ENA bit is also set, emit the SYSTIMER source
        // ID via explicit_irqs so the int-matrix routes it to the CPU.
        // Auto-reload alarms bump `target += period` on fire to schedule
        // the next event; pending stays sticky until INT_CLR.
        let mut explicit_irqs = Vec::new();
        let counter = self.unit0.counter;
        for (i, alarm) in self.unit0_alarms.iter_mut().enumerate() {
            if !alarm.enabled {
                continue;
            }
            if counter >= alarm.target && !alarm.pending {
                alarm.pending = true;
                if self.int_ena & (1 << i) != 0 {
                    explicit_irqs.push(SYSTIMER_TARGET0_SOURCE + i as u32);
                }
                if alarm.auto_reload && alarm.period > 0 {
                    alarm.target = alarm.target.saturating_add(alarm.period);
                }
            }
        }

        PeripheralTickResult {
            explicit_irqs,
            ..PeripheralTickResult::default()
        }
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

    // ── Plan 3 Task 4: alarm tests ──

    #[test]
    fn alarm_target_round_trip() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x70, 0x0000_0001); // ALARM0_TARGET_HI = 1
        s.write_word(0x74, 0x0000_0042); // ALARM0_TARGET_LO = 0x42
        assert_eq!(s.unit0_alarms[0].target, (1u64 << 32) | 0x42);
        assert_eq!(s.read_word(0x70), 1);
        assert_eq!(s.read_word(0x74), 0x42);
    }

    #[test]
    fn alarm_conf_enable_and_period() {
        let mut s = Systimer::new(80_000_000);
        // Period = 100, enable bit, no auto-reload.
        let conf = ALARM_ENABLE_BIT | 100u32;
        s.write_word(0x78, conf);
        assert!(s.unit0_alarms[0].enabled);
        assert!(!s.unit0_alarms[0].auto_reload);
        assert_eq!(s.unit0_alarms[0].period, 100);
        // Read-back preserves the same fields.
        assert_eq!(s.read_word(0x78), conf);
    }

    #[test]
    fn alarm_fires_when_counter_reaches_target() {
        let mut s = Systimer::new(80_000_000);
        // Set ALARM0 target = 5 SYSTIMER ticks = 25 CPU cycles at 80 MHz.
        s.write_word(0x70, 0); // TARGET0_HI
        s.write_word(0x74, 5); // TARGET0_LO
        s.write_word(0x78, ALARM_ENABLE_BIT); // enable bit set, no auto-reload
        s.write_word(0x94, 1); // INT_ENA bit 0
        // Tick CPU 24 cycles — alarm should not fire yet (counter < 5).
        for _ in 0..24 {
            let r = s.tick();
            assert!(r.explicit_irqs.is_empty(), "should not fire before target");
        }
        // 25th cycle: counter just reached 5, alarm fires.
        let r = s.tick();
        assert_eq!(r.explicit_irqs, vec![79], "TARGET0 source ID expected");
        assert!(s.unit0_alarms[0].pending);
        // INT_RAW reflects pending bit.
        assert_eq!(s.read_word(0x98), 1);
    }

    #[test]
    fn alarm_disabled_does_not_fire() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x70, 0);
        s.write_word(0x74, 5);
        // ALARM0_CONF left at 0 — alarm disabled.
        s.write_word(0x94, 1);
        for _ in 0..30 {
            let r = s.tick();
            assert!(r.explicit_irqs.is_empty());
        }
        assert!(!s.unit0_alarms[0].pending);
    }

    #[test]
    fn int_ena_zero_suppresses_irq_but_pending_set() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x70, 0);
        s.write_word(0x74, 5);
        s.write_word(0x78, ALARM_ENABLE_BIT);
        // INT_ENA = 0 — alarm fires (pending set) but no IRQ delivered.
        for _ in 0..30 {
            let r = s.tick();
            assert!(r.explicit_irqs.is_empty(), "no IRQ when INT_ENA=0");
        }
        assert!(
            s.unit0_alarms[0].pending,
            "pending bit set even without INT_ENA"
        );
    }

    #[test]
    fn int_clr_clears_pending() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x70, 0);
        s.write_word(0x74, 5);
        s.write_word(0x78, ALARM_ENABLE_BIT);
        s.write_word(0x94, 1);
        for _ in 0..30 {
            s.tick();
        }
        assert!(s.unit0_alarms[0].pending);
        // Write 1 to INT_CLR bit 0.
        s.write_word(0x9C, 1);
        assert!(!s.unit0_alarms[0].pending);
        assert_eq!(s.read_word(0x98), 0, "INT_RAW cleared");
    }

    #[test]
    fn int_st_masks_raw_with_ena() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x70, 0);
        s.write_word(0x74, 5);
        s.write_word(0x78, ALARM_ENABLE_BIT);
        // INT_ENA = 0 to start; alarm fires but INT_ST should be 0.
        for _ in 0..30 {
            s.tick();
        }
        assert_eq!(s.read_word(0x98), 1, "INT_RAW set");
        assert_eq!(s.read_word(0xA0), 0, "INT_ST masked by INT_ENA=0");
        // Now set INT_ENA bit 0; INT_ST should reflect.
        s.write_word(0x94, 1);
        assert_eq!(s.read_word(0xA0), 1, "INT_ST = RAW & ENA");
    }

    #[test]
    fn alarm_does_not_double_fire_while_pending() {
        // Once pending is set, subsequent ticks above target must not push
        // additional source IDs into explicit_irqs — IRQ is edge-triggered
        // on the rising counter>=target transition, latched by `pending`.
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x70, 0);
        s.write_word(0x74, 5);
        s.write_word(0x78, ALARM_ENABLE_BIT);
        s.write_word(0x94, 1);
        let mut total_irqs = 0usize;
        for _ in 0..200 {
            total_irqs += s.tick().explicit_irqs.len();
        }
        assert_eq!(total_irqs, 1, "alarm fires exactly once until INT_CLR");
    }

    #[test]
    fn auto_reload_reschedules_target() {
        // Auto-reload alarm with period=10 (SYSTIMER ticks). After the first
        // fire and an INT_CLR, the alarm should re-fire 10 ticks later.
        let mut s = Systimer::new(80_000_000); // 5 CPU cycles per SYSTIMER tick.
        s.write_word(0x70, 0);
        s.write_word(0x74, 5); // first fire at counter=5
        s.write_word(0x78, ALARM_AUTO_RELOAD_BIT | ALARM_ENABLE_BIT | 10);
        s.write_word(0x94, 1);
        // 25 CPU cycles -> counter=5, first fire.
        let mut first_fire = None;
        for cycle in 0..30 {
            let r = s.tick();
            if !r.explicit_irqs.is_empty() {
                first_fire = Some(cycle + 1);
                break;
            }
        }
        assert_eq!(first_fire, Some(25));
        // Target should now be 5 + 10 = 15.
        assert_eq!(s.unit0_alarms[0].target, 15);
        // Clear pending so the next edge can fire.
        s.write_word(0x9C, 1);
        // Need counter to reach 15 — currently 5 (just), need 10 more
        // SYSTIMER ticks = 50 more CPU cycles.
        let mut second_fire = None;
        for cycle in 0..60 {
            let r = s.tick();
            if !r.explicit_irqs.is_empty() {
                second_fire = Some(cycle + 1);
                break;
            }
        }
        assert_eq!(second_fire, Some(50), "second fire 50 CPU cycles later");
        assert_eq!(s.unit0_alarms[0].target, 25);
    }
}
