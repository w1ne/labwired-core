// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SYSTIMER peripheral for ESP32-S3.
//!
//! Two 64-bit free-running counters (UNIT0, UNIT1), each clocked at 16 MHz
//! independently of CPU frequency.  Plan 2 implemented the counter + the
//! load/update handshake; Plan 3 adds 3 alarms with IRQ delivery.
//!
//! ## Register layout (ESP32-S3 TRM §16.5; verified against esp32s3-pac 0.35.2)
//!
//! Offsets match the TRM / esp32s3-pac so that esp-hal's Alarm API writes
//! land in the right fields.
//!
//! | Offset | Name              | Behaviour |
//! |-------:|-------------------|-----------|
//! |  0x00  | CONF              | bit 31 clk_en, bit 30 unit0_work_en, bit 29 unit1_work_en, bit 24 target0_work_en, bit 23 target1_work_en, bit 22 target2_work_en |
//! |  0x04  | UNIT0_OP          | write 1<<30 to trigger snapshot of UNIT0; reads bit 29 (VALUE_VALID) set |
//! |  0x08  | UNIT1_OP          | same for UNIT1 |
//! |  0x0C  | UNIT0_LOAD_HI     | high 32 bits of pending load |
//! |  0x10  | UNIT0_LOAD_LO     | low 32 bits of pending load |
//! |  0x14  | UNIT1_LOAD_HI     | high 32 bits of pending load (UNIT1) |
//! |  0x18  | UNIT1_LOAD_LO     | low 32 bits of pending load (UNIT1) |
//! |  0x1C  | TARGET0_HI        | high 32 bits of UNIT alarm 0 target (pending until COMP0_LOAD) |
//! |  0x20  | TARGET0_LO        | low  32 bits of UNIT alarm 0 target |
//! |  0x24  | TARGET1_HI        | high 32 bits of UNIT alarm 1 target |
//! |  0x28  | TARGET1_LO        | low  32 bits of UNIT alarm 1 target |
//! |  0x2C  | TARGET2_HI        | high 32 bits of UNIT alarm 2 target |
//! |  0x30  | TARGET2_LO        | low  32 bits of UNIT alarm 2 target |
//! |  0x34  | TARGET0_CONF      | bit 31 timer_unit_sel, bit 30 period_mode, bits[25:0] period |
//! |  0x38  | TARGET1_CONF      | same fields for alarm 1 |
//! |  0x3C  | TARGET2_CONF      | same fields for alarm 2 |
//! |  0x40  | UNIT0_VALUE_HI    | snapshot high 32 bits |
//! |  0x44  | UNIT0_VALUE_LO    | snapshot low 32 bits |
//! |  0x48  | UNIT1_VALUE_HI    | snapshot high 32 bits |
//! |  0x4C  | UNIT1_VALUE_LO    | snapshot low 32 bits |
//! |  0x50  | COMP0_LOAD        | write bit 0 to commit pending TARGET0 / period into the active alarm |
//! |  0x54  | COMP1_LOAD        | same for alarm 1 |
//! |  0x58  | COMP2_LOAD        | same for alarm 2 |
//! |  0x5C  | UNIT0_LOAD        | write 1 to commit pending UNIT0 LOAD into counter |
//! |  0x60  | UNIT1_LOAD        | same for UNIT1 |
//! |  0x64  | INT_ENA           | bits 0/1/2 — enable IRQ for TARGET0/1/2 |
//! |  0x68  | INT_RAW           | bits 0/1/2 — pending bit per alarm (RO) |
//! |  0x6C  | INT_CLR           | write-1-to-clear pending bits |
//! |  0x70  | INT_ST            | INT_RAW & INT_ENA (RO) |
//!
//! ## TARGETx_CONF semantics
//!
//! Per the verified esp32s3-pac 0.35.2 layout (TRM §16.5):
//! * bit 31 — TIMER_UNIT_SEL: 0 = compare against UNIT0, 1 = UNIT1.
//! * bit 30 — PERIOD_MODE: when set with non-zero `period`, on fire we bump
//!   `target += period` so the next compare schedules the next event.
//! * bits[25:0] — period in SYSTIMER ticks (16 MHz nominally).
//!
//! Crucially: **TARGETx_CONF has NO alarm-enable bit**. The enable lives in
//! SYSTIMER_CONF (offset 0x00) at bits 24/23/22 (target0/1/2_work_en). An
//! earlier draft of this peripheral mistakenly modeled bit 31 of
//! TARGETx_CONF as an enable; that masked the real esp-hal write pattern
//! where `comparator.set_enable()` toggles the CONF bit and never touches
//! TARGETx_CONF, leaving the alarm permanently disabled in the simulator.
//!
//! ## COMP_LOAD commit semantics
//!
//! Real silicon double-buffers TARGETx_HI/LO and TARGETx_CONF.period.
//! Writes to those registers stage a *pending* value; the active comparator
//! only picks them up when firmware writes bit 0 of COMPx_LOAD. esp-hal's
//! `set_period` and `set_target` both end with a COMPx_LOAD write to
//! commit. The simulator stores both `pending_target`/`pending_period` and
//! the live `target`/`period` separately and copies pending → live on
//! COMPx_LOAD.
//!
//! ## Source IDs (ESP32-S3 TRM §9.4; verified against esp32s3-pac 0.35.2 `Interrupt` enum)
//!
//! Alarms emit interrupt-matrix source IDs via
//! `PeripheralTickResult.explicit_irqs`:
//!
//! * TARGET0 → source 57
//! * TARGET1 → source 58
//! * TARGET2 → source 59
//!
//! (An earlier draft hard-coded 79/80/81 — the FROM_CPU_INTR0..2 IDs.
//! That worked for the integration test because the test wrote 79 directly,
//! but esp-hal binds SYSTIMER_TARGET0 at the PAC-defined source 57.)

use crate::{Peripheral, PeripheralTickResult, SimResult};

const SYSTIMER_CLOCK_HZ: u64 = 16_000_000;

/// SYSTIMER interrupt-matrix source ID for TARGET0 (per esp32s3-pac
/// `Interrupt::SYSTIMER_TARGET0 = 57`).  TARGET1/2 follow at +1/+2.
const SYSTIMER_TARGET0_SOURCE: u32 = 57;

/// Mask for the 26-bit `period` field in TARGETx_CONF.
const ALARM_PERIOD_MASK: u32 = 0x03FF_FFFF;
/// TARGETx_CONF bit 31 — TIMER_UNIT_SEL (0=UNIT0, 1=UNIT1).
const ALARM_UNIT_SEL_BIT: u32 = 1 << 31;
/// TARGETx_CONF bit 30 — PERIOD_MODE (auto-reload).
const ALARM_PERIOD_MODE_BIT: u32 = 1 << 30;

// SYSTIMER_CONF (offset 0x00) bit positions for per-alarm enable.
const CONF_TARGET0_WORK_EN: u32 = 1 << 24;
const CONF_TARGET1_WORK_EN: u32 = 1 << 23;
const CONF_TARGET2_WORK_EN: u32 = 1 << 22;

#[derive(Debug, Default, Clone, Copy)]
struct UnitState {
    counter: u64,
    snapshot: u64,
    load_hi: u32,
    load_lo: u32,
}

/// Per-alarm state. One instance per alarm slot.
///
/// Pending vs. live separation models real-silicon double-buffering:
/// firmware writes to TARGETx_HI/LO/period stage values that only commit
/// to the active alarm on COMPx_LOAD bit-0 write.
#[derive(Debug, Default, Clone, Copy)]
struct AlarmState {
    /// Live (committed) 64-bit comparison target. Alarm fires when the
    /// selected unit's counter >= this value.
    target: u64,
    /// Pending TARGETx_HI/LO write — copied to `target` on COMPx_LOAD.
    pending_target: u64,
    /// INT_RAW pending bit. Sticky until INT_CLR clears it.
    pending: bool,
    /// Internal: set on the rising counter>=target edge; cleared when the
    /// target changes (commit / period bump). Distinct from `pending`
    /// because INT_CLR clears `pending` without re-arming the edge — the
    /// alarm only fires again when the target moves (period mode) or
    /// firmware re-arms via COMPx_LOAD.
    edge_latched: bool,
    /// CONF.targetN_work_en mirror — true means alarm is armed.
    enabled: bool,
    /// TARGETx_CONF bit 30 (PERIOD_MODE / auto-reload). When set with
    /// non-zero `period`, on fire we bump `target += period` so the next
    /// compare schedules the next event.
    period_mode: bool,
    /// TARGETx_CONF bits[25:0] — live period in SYSTIMER ticks.
    period: u64,
    /// Pending period — copied to `period` and applied to `target` on
    /// COMPx_LOAD. esp-hal's `set_period` writes here then commits.
    pending_period: u64,
    /// TARGETx_CONF bit 31 — true means compare against UNIT1.
    unit_sel: bool,
}

#[derive(Debug)]
pub struct Systimer {
    /// SYSTIMER_CONF (0x00). bits 24/23/22 are mirrored into
    /// `unit0_alarms[i].enabled` on write so the per-alarm enable check is
    /// O(1) on the tick path.
    conf: u32,
    unit0: UnitState,
    unit1: UnitState,
    cpu_clock_hz: u32,
    /// Accumulated CPU cycles since last counter update; flushed when ≥ 1
    /// SYSTIMER tick worth of CPU cycles have elapsed.
    cpu_cycle_accum: u64,
    /// Three alarm comparators. Each may be configured to compare against
    /// UNIT0 or UNIT1 via TARGETx_CONF bit 31.
    unit0_alarms: [AlarmState; 3],
    /// INT_ENA: bits 0/1/2 enable IRQ delivery for alarms 0/1/2.
    /// Pending bits in INT_RAW set regardless of INT_ENA; only IRQ
    /// emission via `explicit_irqs` is gated by these bits.
    int_ena: u32,
}

impl Systimer {
    pub fn new(cpu_clock_hz: u32) -> Self {
        Self {
            // Default: clock enabled (bit 31), both units running (bits 30, 29).
            // Per-alarm enable bits (24/23/22) start cleared.
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
            // VALUE registers. We model the snapshot as instantaneous and
            // always assert bit 29.
            0x04 | 0x08 => 1u32 << 29,

            // ── LOAD pending registers (TRM offsets) ──
            0x0C => self.unit0.load_hi,
            0x10 => self.unit0.load_lo,
            0x14 => self.unit1.load_hi,
            0x18 => self.unit1.load_lo,

            // ── TARGETx HI/LO (TRM offsets) ──
            // Reads return the *pending* (most recently written) value, per
            // PAC convention — readback shows what firmware just wrote, not
            // the live committed value.
            0x1C => (self.unit0_alarms[0].pending_target >> 32) as u32,
            0x20 => (self.unit0_alarms[0].pending_target & 0xFFFF_FFFF) as u32,
            0x24 => (self.unit0_alarms[1].pending_target >> 32) as u32,
            0x28 => (self.unit0_alarms[1].pending_target & 0xFFFF_FFFF) as u32,
            0x2C => (self.unit0_alarms[2].pending_target >> 32) as u32,
            0x30 => (self.unit0_alarms[2].pending_target & 0xFFFF_FFFF) as u32,

            // ── TARGETx_CONF (TRM offsets) ──
            // Reads expose pending period (so esp-hal's read-modify-write
            // sequences see the value they just staged).
            0x34 => alarm_conf_word_pending(&self.unit0_alarms[0]),
            0x38 => alarm_conf_word_pending(&self.unit0_alarms[1]),
            0x3C => alarm_conf_word_pending(&self.unit0_alarms[2]),

            // ── VALUE snapshot registers ──
            0x40 => (self.unit0.snapshot >> 32) as u32,
            0x44 => (self.unit0.snapshot & 0xFFFF_FFFF) as u32,
            0x48 => (self.unit1.snapshot >> 32) as u32,
            0x4C => (self.unit1.snapshot & 0xFFFF_FFFF) as u32,

            // 0x50/0x54/0x58 COMPx_LOAD are write-only commit triggers; reads
            // return 0 on real silicon.

            // 0x5C/0x60 UNITx_LOAD are write-only commit triggers.

            // ── Interrupt registers (TRM offsets) ──
            0x64 => self.int_ena,
            0x68 => self.int_raw_word(),
            // 0x6C INT_CLR is W1C; reads as 0.
            0x70 => self.int_raw_word() & self.int_ena,

            _ => 0,
        }
    }

    /// INT_RAW (0x68): bits 0/1/2 = pending bit per alarm.
    fn int_raw_word(&self) -> u32 {
        let mut v = 0u32;
        for (i, alarm) in self.unit0_alarms.iter().enumerate() {
            if alarm.pending {
                v |= 1 << i;
            }
        }
        v
    }

    /// Apply CONF bits 24/23/22 to per-alarm `enabled` flags. Called on any
    /// write to CONF (0x00). Mirroring keeps the tick-path check O(1).
    fn sync_alarm_enables_from_conf(&mut self) {
        self.unit0_alarms[0].enabled = self.conf & CONF_TARGET0_WORK_EN != 0;
        self.unit0_alarms[1].enabled = self.conf & CONF_TARGET1_WORK_EN != 0;
        self.unit0_alarms[2].enabled = self.conf & CONF_TARGET2_WORK_EN != 0;
    }

    /// Commit pending target / period into live for alarm `idx`. Called when
    /// firmware writes bit 0 to COMPx_LOAD. Real silicon also rebases the
    /// internal compare value to "current_count + period" when the alarm is
    /// in PERIOD_MODE, ensuring the first fire happens `period` ticks after
    /// the commit (not relative to a stale target).
    fn commit_alarm(&mut self, idx: usize) {
        let alarm = &mut self.unit0_alarms[idx];
        alarm.period = alarm.pending_period;
        // In period mode, the next fire is current_count + period (relative
        // to the unit the alarm is bound to). In target mode, the pending
        // absolute target wins.
        if alarm.period_mode && alarm.period > 0 {
            let counter = if alarm.unit_sel {
                self.unit1.counter
            } else {
                self.unit0.counter
            };
            alarm.target = counter.saturating_add(alarm.period);
        } else {
            alarm.target = alarm.pending_target;
        }
        // Re-arm: clear sticky pending and edge latches so the next
        // counter>=target rising edge can fire.
        alarm.pending = false;
        alarm.edge_latched = false;
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {
                self.conf = value;
                self.sync_alarm_enables_from_conf();
            }
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

            // ── LOAD pending registers (TRM offsets) ──
            0x0C => self.unit0.load_hi = value,
            0x10 => self.unit0.load_lo = value,
            0x14 => self.unit1.load_hi = value,
            0x18 => self.unit1.load_lo = value,

            // ── TARGETx HI/LO ──
            // Stage pending target; commit on COMPx_LOAD.
            0x1C => set_pending_target_hi(&mut self.unit0_alarms[0], value),
            0x20 => set_pending_target_lo(&mut self.unit0_alarms[0], value),
            0x24 => set_pending_target_hi(&mut self.unit0_alarms[1], value),
            0x28 => set_pending_target_lo(&mut self.unit0_alarms[1], value),
            0x2C => set_pending_target_hi(&mut self.unit0_alarms[2], value),
            0x30 => set_pending_target_lo(&mut self.unit0_alarms[2], value),

            // ── TARGETx_CONF ──
            0x34 => set_alarm_conf(&mut self.unit0_alarms[0], value),
            0x38 => set_alarm_conf(&mut self.unit0_alarms[1], value),
            0x3C => set_alarm_conf(&mut self.unit0_alarms[2], value),

            // ── COMPx_LOAD: commit pending writes into live alarm ──
            0x50 => {
                if value & 1 != 0 {
                    self.commit_alarm(0);
                }
            }
            0x54 => {
                if value & 1 != 0 {
                    self.commit_alarm(1);
                }
            }
            0x58 => {
                if value & 1 != 0 {
                    self.commit_alarm(2);
                }
            }

            // ── UNITx_LOAD commit (TRM offsets) ──
            0x5C => {
                if value & 1 != 0 {
                    self.unit0.counter =
                        ((self.unit0.load_hi as u64) << 32) | (self.unit0.load_lo as u64);
                }
            }
            0x60 => {
                if value & 1 != 0 {
                    self.unit1.counter =
                        ((self.unit1.load_hi as u64) << 32) | (self.unit1.load_lo as u64);
                }
            }

            // ── Interrupt registers (TRM offsets) ──
            0x64 => self.int_ena = value & 0x7,
            // 0x68 INT_RAW is read-only on real silicon; ignore writes.
            0x6C => {
                // INT_CLR: write-1-to-clear pending bits.
                for (i, alarm) in self.unit0_alarms.iter_mut().enumerate() {
                    if value & (1 << i) != 0 {
                        alarm.pending = false;
                    }
                }
            }
            // 0x70 INT_ST is read-only.
            _ => {}
        }
    }
}

/// Compose TARGETx_CONF readback exposing the *pending* period (real
/// silicon would expose live, but esp-hal's RMW patterns expect to read
/// back what they just wrote before the COMP_LOAD commit).
fn alarm_conf_word_pending(alarm: &AlarmState) -> u32 {
    let mut v = (alarm.pending_period as u32) & ALARM_PERIOD_MASK;
    if alarm.period_mode {
        v |= ALARM_PERIOD_MODE_BIT;
    }
    if alarm.unit_sel {
        v |= ALARM_UNIT_SEL_BIT;
    }
    v
}

fn set_pending_target_hi(alarm: &mut AlarmState, value: u32) {
    let lo = alarm.pending_target & 0xFFFF_FFFF;
    alarm.pending_target = ((value as u64) << 32) | lo;
}

fn set_pending_target_lo(alarm: &mut AlarmState, value: u32) {
    let hi = alarm.pending_target & 0xFFFF_FFFF_0000_0000;
    alarm.pending_target = hi | (value as u64);
}

fn set_alarm_conf(alarm: &mut AlarmState, value: u32) {
    alarm.pending_period = (value & ALARM_PERIOD_MASK) as u64;
    alarm.period_mode = value & ALARM_PERIOD_MODE_BIT != 0;
    alarm.unit_sel = value & ALARM_UNIT_SEL_BIT != 0;
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

        // ── Alarm checks ──
        // For each enabled alarm, on the rising edge from
        // `counter < target` to `counter >= target` we set the pending bit
        // and bump the target for period-mode alarms.
        //
        // IRQ delivery is *level-sensitive*: as long as `pending && int_ena`,
        // we keep emitting the SYSTIMER source ID on every tick. This
        // matches real-silicon behaviour where the peripheral's IRQ line
        // stays asserted until firmware writes INT_CLR. Without this,
        // dispatch_irq's one-shot clear of `bus.pending_cpu_irqs` would
        // race the firmware's own INTERRUPT-SR read inside the ISR — by
        // the time __level_1_interrupt iterates through pending sources,
        // both the bus aggregator bit and the SR bit are 0, so the ISR
        // exits without invoking the user handler. (Plan 3 Task 10 case
        // study — the ISR ran but never called alarm_isr; INT_RAW stayed
        // sticky, and the LED never toggled.)
        let mut explicit_irqs = Vec::new();
        let unit0_counter = self.unit0.counter;
        let unit1_counter = self.unit1.counter;
        for (i, alarm) in self.unit0_alarms.iter_mut().enumerate() {
            if !alarm.enabled {
                continue;
            }
            let counter = if alarm.unit_sel { unit1_counter } else { unit0_counter };
            // Detect rising edge: counter >= target with edge_latched still
            // clear. On edge, latch + set sticky pending (visible via
            // INT_RAW). For period-mode, bump target by period and re-arm
            // the latch so the next period boundary can fire.
            if counter >= alarm.target && !alarm.edge_latched {
                alarm.edge_latched = true;
                alarm.pending = true;
                if alarm.period_mode && alarm.period > 0 {
                    alarm.target = alarm.target.saturating_add(alarm.period);
                    // Allow the next edge (counter < new_target → counter
                    // >= new_target) to re-fire without firmware intervention.
                    alarm.edge_latched = false;
                }
            }
            // Level-sensitive IRQ: while pending && int_ena, emit the
            // source on every tick. The bus aggregator OR's into
            // pending_cpu_irqs, which dispatch_irq clears on entry — but
            // because we re-emit each tick, the firmware sees a stable
            // pending_cpu_irqs/INTERRUPT bit while it iterates handlers.
            if alarm.pending && (self.int_ena & (1 << i) != 0) {
                explicit_irqs.push(SYSTIMER_TARGET0_SOURCE + i as u32);
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
    fn op_read_asserts_value_valid_bit() {
        // esp-hal's Delay polls UNIT0_OP bit 29 (VALUE_VALID) before reading
        // the VALUE registers.  We model snapshots as instantaneous and
        // always assert bit 29 so the busy-wait exits.  Plan 2 hello-world
        // depends on this.
        let s = Systimer::new(80_000_000);
        assert_eq!(s.read_word(0x04) & (1 << 29), 1 << 29);
        assert_eq!(s.read_word(0x08) & (1 << 29), 1 << 29);
    }

    #[test]
    fn load_handshake_sets_counter() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x0C, 0x0000_0001); // UNIT0_LOAD_HI = 1
        s.write_word(0x10, 0x0000_0042); // UNIT0_LOAD_LO = 0x42
        s.write_word(0x5C, 1); // commit
        assert_eq!(s.unit0.counter, (1u64 << 32) | 0x42);
    }

    #[test]
    fn unit1_load_handshake_sets_counter() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x14, 0x0000_0002); // UNIT1_LOAD_HI = 2
        s.write_word(0x18, 0x0000_00AA); // UNIT1_LOAD_LO = 0xAA
        s.write_word(0x60, 1); // commit
        assert_eq!(s.unit1.counter, (2u64 << 32) | 0xAA);
    }

    #[test]
    fn unit1_independent_of_unit0() {
        let mut s = Systimer::new(80_000_000);
        for _ in 0..5 {
            s.tick();
        }
        assert_eq!(s.unit0.counter, 1);
        assert_eq!(s.unit1.counter, 1, "unit1 ticks alongside unit0");
        s.write_word(0x5C, 1); // commit a load to unit0 (loads were 0)
        assert_eq!(s.unit0.counter, 0);
        assert_eq!(s.unit1.counter, 1, "unit1 not affected by unit0 load");
    }

    #[test]
    fn disabled_unit_does_not_tick() {
        let mut s = Systimer::new(80_000_000);
        // Clear bit 30 (unit0 work enable). Keep bit 29 (unit1 work enable).
        s.write_word(0x00, 0xA000_0000);
        for _ in 0..50 {
            s.tick();
        }
        assert_eq!(s.unit0.counter, 0, "disabled unit must not tick");
        assert_eq!(s.unit1.counter, 10, "unit1 still ticks");
    }

    // ── Plan 3 Task 4 / Task 10 alarm tests (TRM-correct: enable in CONF) ──

    #[test]
    fn alarm_pending_target_stages_until_comp_load() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x1C, 0x0000_0001); // TARGET0_HI = 1 (pending)
        s.write_word(0x20, 0x0000_0042); // TARGET0_LO = 0x42 (pending)
        // Live target unchanged before commit.
        assert_eq!(s.unit0_alarms[0].target, 0);
        assert_eq!(s.unit0_alarms[0].pending_target, (1u64 << 32) | 0x42);
        // COMP0_LOAD commits.
        s.write_word(0x50, 1);
        assert_eq!(s.unit0_alarms[0].target, (1u64 << 32) | 0x42);
        // Readback of TARGETx_HI/LO returns pending (just-written) value.
        assert_eq!(s.read_word(0x1C), 1);
        assert_eq!(s.read_word(0x20), 0x42);
    }

    #[test]
    fn alarm_conf_period_and_mode_bits() {
        let mut s = Systimer::new(80_000_000);
        // Period = 100, period_mode (bit 30), unit_sel = 0.
        let conf = ALARM_PERIOD_MODE_BIT | 100u32;
        s.write_word(0x34, conf);
        assert!(s.unit0_alarms[0].period_mode);
        assert!(!s.unit0_alarms[0].unit_sel);
        assert_eq!(s.unit0_alarms[0].pending_period, 100);
        // Read-back exposes pending period + flags.
        assert_eq!(s.read_word(0x34), conf);
    }

    #[test]
    fn alarm_conf_unit_sel_round_trip() {
        let mut s = Systimer::new(80_000_000);
        let conf = ALARM_UNIT_SEL_BIT | ALARM_PERIOD_MODE_BIT | 50u32;
        s.write_word(0x34, conf);
        assert!(s.unit0_alarms[0].period_mode);
        assert!(s.unit0_alarms[0].unit_sel);
        assert_eq!(s.read_word(0x34), conf);
    }

    #[test]
    fn alarm_fires_on_period_mode_after_comp_load_and_conf_enable() {
        // Real-silicon arming sequence:
        //   1. Write TARGETx_CONF with period + PERIOD_MODE.
        //   2. Write COMPx_LOAD bit 0 (commits period; rebases target to
        //      counter+period).
        //   3. Write CONF.targetN_work_en (1<<24 for alarm 0).
        //   4. Write INT_ENA bit 0 to enable IRQ delivery.
        let mut s = Systimer::new(80_000_000);
        // Period = 5 SYSTIMER ticks = 25 CPU cycles at 80MHz.
        s.write_word(0x34, ALARM_PERIOD_MODE_BIT | 5);
        s.write_word(0x50, 1); // COMP0_LOAD: target = counter(0) + 5 = 5
        s.write_word(0x00, s.read_word(0x00) | CONF_TARGET0_WORK_EN);
        s.write_word(0x64, 1); // INT_ENA bit 0
        for _ in 0..24 {
            let r = s.tick();
            assert!(r.explicit_irqs.is_empty(), "no fire before counter reaches target");
        }
        let r = s.tick();
        assert_eq!(r.explicit_irqs, vec![57], "TARGET0 source ID at counter==target");
        assert!(s.unit0_alarms[0].pending);
        assert_eq!(s.read_word(0x68), 1, "INT_RAW reflects pending");
    }

    #[test]
    fn alarm_disabled_when_conf_enable_clear() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x34, 5); // period=5, no period_mode, no enable
        s.write_word(0x50, 1); // commit
        // CONF.target0_work_en still 0 — alarm disabled.
        s.write_word(0x64, 1);
        for _ in 0..30 {
            let r = s.tick();
            assert!(r.explicit_irqs.is_empty());
        }
        assert!(!s.unit0_alarms[0].pending);
    }

    #[test]
    fn int_ena_zero_suppresses_irq_but_pending_set() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x34, ALARM_PERIOD_MODE_BIT | 5);
        s.write_word(0x50, 1);
        s.write_word(0x00, s.read_word(0x00) | CONF_TARGET0_WORK_EN);
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
        s.write_word(0x34, ALARM_PERIOD_MODE_BIT | 5);
        s.write_word(0x50, 1);
        s.write_word(0x00, s.read_word(0x00) | CONF_TARGET0_WORK_EN);
        s.write_word(0x64, 1);
        for _ in 0..30 {
            s.tick();
        }
        assert!(s.unit0_alarms[0].pending);
        // Write 1 to INT_CLR bit 0.
        s.write_word(0x6C, 1);
        assert!(!s.unit0_alarms[0].pending);
        assert_eq!(s.read_word(0x68), 0, "INT_RAW cleared");
    }

    #[test]
    fn int_st_masks_raw_with_ena() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x34, ALARM_PERIOD_MODE_BIT | 5);
        s.write_word(0x50, 1);
        s.write_word(0x00, s.read_word(0x00) | CONF_TARGET0_WORK_EN);
        // INT_ENA = 0 to start; alarm fires but INT_ST should be 0.
        for _ in 0..30 {
            s.tick();
        }
        assert_eq!(s.read_word(0x68), 1, "INT_RAW set");
        assert_eq!(s.read_word(0x70), 0, "INT_ST masked by INT_ENA=0");
        // Now set INT_ENA bit 0; INT_ST should reflect.
        s.write_word(0x64, 1);
        assert_eq!(s.read_word(0x70), 1, "INT_ST = RAW & ENA");
    }

    #[test]
    fn alarm_pending_bit_set_only_once_on_rising_edge() {
        // The TARGET0 *pending* latch is edge-triggered: once `counter >=
        // target` flips pending true, subsequent ticks above target don't
        // re-bump pending. (IRQ delivery itself is level-sensitive; see
        // `alarm_emits_irq_every_tick_while_pending`.)
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x34, 5); // period=5, no period_mode → fires once
        s.write_word(0x50, 1);
        s.write_word(0x00, s.read_word(0x00) | CONF_TARGET0_WORK_EN);
        s.write_word(0x64, 1);
        // Tick well past target; the alarm should latch pending exactly once.
        for _ in 0..200 {
            s.tick();
        }
        assert!(s.unit0_alarms[0].pending);
        // Clearing returns the alarm to "not pending"; without re-arming
        // (target remains old, no period_mode), it stays cleared.
        s.write_word(0x6C, 1);
        assert!(!s.unit0_alarms[0].pending);
    }

    #[test]
    fn alarm_emits_irq_every_tick_while_pending() {
        // IRQ delivery is level-sensitive: while pending && int_ena, the
        // SYSTIMER source ID is re-emitted on every tick so the bus
        // aggregator keeps the CPU's pending_cpu_irqs bit asserted until
        // firmware ACKs at the source via INT_CLR. Real-silicon model;
        // without this, dispatch_irq's one-shot clear races the firmware's
        // own INTERRUPT read inside the ISR (Plan 3 Task 10 case study).
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x34, 5);
        s.write_word(0x50, 1);
        s.write_word(0x00, s.read_word(0x00) | CONF_TARGET0_WORK_EN);
        s.write_word(0x64, 1);
        // Tick past the first edge; pending now latched.
        for _ in 0..30 {
            s.tick();
        }
        assert!(s.unit0_alarms[0].pending);
        // Subsequent ticks should keep emitting the source ID.
        let r = s.tick();
        assert_eq!(r.explicit_irqs, vec![57], "level-sensitive re-emit");
        let r = s.tick();
        assert_eq!(r.explicit_irqs, vec![57]);
        // INT_CLR de-asserts the level → no more emits.
        s.write_word(0x6C, 1);
        let r = s.tick();
        assert!(r.explicit_irqs.is_empty(), "after INT_CLR, no more IRQs");
    }

    #[test]
    fn period_mode_reschedules_after_int_clr() {
        // Period-mode (auto-reload) alarm with period=10 SYSTIMER ticks.
        // After the first fire and an INT_CLR, the alarm re-fires 10 ticks later.
        let mut s = Systimer::new(80_000_000); // 5 CPU cycles per SYSTIMER tick.
        s.write_word(0x34, ALARM_PERIOD_MODE_BIT | 10);
        s.write_word(0x50, 1); // commit: target = 0 + 10 = 10
        s.write_word(0x00, s.read_word(0x00) | CONF_TARGET0_WORK_EN);
        s.write_word(0x64, 1);
        let mut first_fire = None;
        for cycle in 0..100 {
            let r = s.tick();
            if !r.explicit_irqs.is_empty() {
                first_fire = Some(cycle + 1);
                break;
            }
        }
        assert_eq!(first_fire, Some(50), "first fire at counter==10 → 50 CPU cycles");
        // Target should have been bumped by period: 10 + 10 = 20.
        assert_eq!(s.unit0_alarms[0].target, 20);
        s.write_word(0x6C, 1); // clear pending
        let mut second_fire = None;
        for cycle in 0..100 {
            let r = s.tick();
            if !r.explicit_irqs.is_empty() {
                second_fire = Some(cycle + 1);
                break;
            }
        }
        assert_eq!(second_fire, Some(50), "second fire 50 CPU cycles later");
        assert_eq!(s.unit0_alarms[0].target, 30);
    }

    #[test]
    fn comp_load_in_target_mode_uses_pending_target_absolute() {
        // Without period_mode, COMP_LOAD takes the pending TARGET value as
        // an absolute target.
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x1C, 0); // pending HI = 0
        s.write_word(0x20, 0x42); // pending LO = 0x42
        s.write_word(0x50, 1); // commit
        assert_eq!(s.unit0_alarms[0].target, 0x42);
        // No CONF enable → no fire.
        for _ in 0..1000 {
            assert!(s.tick().explicit_irqs.is_empty());
        }
    }
}
