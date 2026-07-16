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
//! | Offset | Name                | Reset        | Behaviour |
//! |-------:|---------------------|--------------|-----------|
//! |  0x00  | CONF                | 0x46000000   | bit 31 clk_en, bit 30 unit0_work_en, bit 29 unit1_work_en, bit 24 target0_work_en, bit 23 target1_work_en, bit 22 target2_work_en; writable mask 0xFFC00001 |
//! |  0x04  | UNIT0_OP            | 0            | write 1<<30 to trigger snapshot of UNIT0; reads bit 29 (VALUE_VALID) set |
//! |  0x08  | UNIT1_OP            | 0            | same for UNIT1 |
//! |  0x0C  | UNIT0_LOAD_HI       | 0            | high 20 bits of pending load (mask 0x000FFFFF) |
//! |  0x10  | UNIT0_LOAD_LO       | 0            | low 32 bits of pending load |
//! |  0x14  | UNIT1_LOAD_HI       | 0            | high 20 bits of pending load (UNIT1; mask 0x000FFFFF) |
//! |  0x18  | UNIT1_LOAD_LO       | 0            | low 32 bits of pending load |
//! |  0x1C  | TARGET0_HI          | 0            | high 32 bits of UNIT alarm 0 target (pending until COMP0_LOAD) |
//! |  0x20  | TARGET0_LO          | 0            | low  32 bits of UNIT alarm 0 target |
//! |  0x24  | TARGET1_HI          | 0            | high 32 bits of UNIT alarm 1 target |
//! |  0x28  | TARGET1_LO          | 0            | low  32 bits of UNIT alarm 1 target |
//! |  0x2C  | TARGET2_HI          | 0            | high 32 bits of UNIT alarm 2 target |
//! |  0x30  | TARGET2_LO          | 0            | low  32 bits of UNIT alarm 2 target |
//! |  0x34  | TARGET0_CONF        | 0            | bit 31 timer_unit_sel, bit 30 period_mode, bits[25:0] period |
//! |  0x38  | TARGET1_CONF        | 0            | same fields for alarm 1 |
//! |  0x3C  | TARGET2_CONF        | 0            | same fields for alarm 2 |
//! |  0x40  | UNIT0_VALUE_HI      | 0            | snapshot high 32 bits |
//! |  0x44  | UNIT0_VALUE_LO      | 0            | snapshot low 32 bits |
//! |  0x48  | UNIT1_VALUE_HI      | 0            | snapshot high 32 bits |
//! |  0x4C  | UNIT1_VALUE_LO      | 0            | snapshot low 32 bits |
//! |  0x50  | COMP0_LOAD          | 0            | write bit 0 to commit pending TARGET0 / period into the active alarm |
//! |  0x54  | COMP1_LOAD          | 0            | same for alarm 1 |
//! |  0x58  | COMP2_LOAD          | 0            | same for alarm 2 |
//! |  0x5C  | UNIT0_LOAD          | 0            | write 1 to commit pending UNIT0 LOAD into counter |
//! |  0x60  | UNIT1_LOAD          | 0            | same for UNIT1 |
//! |  0x64  | INT_ENA             | 0            | bits 0/1/2 — enable IRQ for TARGET0/1/2 |
//! |  0x68  | INT_RAW             | 0            | bits 0/1/2 — pending bit per alarm (RO) |
//! |  0x6C  | INT_CLR             | 0            | write-1-to-clear pending bits |
//! |  0x70  | INT_ST              | 0            | INT_RAW & INT_ENA (RO) |
//! |  0x74  | REAL_TARGET0_LO     | 0            | read-only: live committed alarm-0 target bits[31:0] |
//! |  0x78  | REAL_TARGET0_HI     | 0            | read-only: live committed alarm-0 target bits[51:32] |
//! |  0x7C  | REAL_TARGET1_LO     | 0            | read-only: live committed alarm-1 target bits[31:0] |
//! |  0x80  | REAL_TARGET1_HI     | 0            | read-only: live committed alarm-1 target bits[51:32] |
//! |  0x84  | REAL_TARGET2_LO     | 0            | read-only: live committed alarm-2 target bits[31:0] |
//! |  0x88  | REAL_TARGET2_HI     | 0            | read-only: live committed alarm-2 target bits[51:32] |
//! |  0xFC  | DATE                | 0x02012251   | HW-validated silicon date/revision register (RW) |
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

use crate::{CycleClock, MmioAccessClass, Peripheral, PeripheralTickResult, SimResult};

const SYSTIMER_CLOCK_HZ: u64 = 16_000_000;

/// SYSTIMER interrupt-matrix source ID for TARGET0 (per esp32s3-pac
/// `Interrupt::SYSTIMER_TARGET0 = 57`).  TARGET1/2 follow at +1/+2.
const SYSTIMER_TARGET0_SOURCE: u32 = 57;
const SYSTIMER_WAKE_TOKEN: u32 = 0;

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

/// SVD-derived register offsets (`scripts/gen_esp_systimer_regs.py`).
#[path = "systimer_regs.rs"]
mod regs;

/// UNIT*_OP bit 30 — trigger snapshot update.
const OP_UPDATE_BIT: u32 = 1 << 30;
/// UNIT*_OP bit 29 — VALUE_VALID (snapshot ready).
const OP_VALUE_VALID_BIT: u32 = 1 << 29;

#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Systimer {
    /// SYSTIMER_CONF (0x00). SVD reset = 0x46000000 (unit0 work_en; bit 31
    /// CLK_EN is 0 at reset — clock is gated; firmware writes CONF to
    /// enable). Bits 24/23/22 are mirrored into `unit0_alarms[i].enabled`
    /// on write so the per-alarm enable check is O(1) on the tick path.
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
    /// DATE register (0xFC). Silicon-validated reset = 0x02012251.
    /// RW storage; firmware may write but reset value is the revision ID.
    date: u32,
    /// Interrupt-matrix source ID emitted for TARGET0 (TARGET1/2 follow at
    /// +1/+2). Defaults to the ESP32-S3 PAC value (57); the ESP32-C3 reuses
    /// this same IP but its matrix source IDs are 37/38/39, so the C3 wiring
    /// constructs the model with `new_with_source(_, 37)`.
    target0_source: u32,
    /// Scheduler/elapsed-mode anchor in peripheral-tick units. The C3 ROM path
    /// uses the default one CPU cycle per peripheral tick.
    last_tick: u64,
    /// Whether this instance participates in the event scheduler.
    ///
    /// C3 firmware commonly reads SYSTIMER through write-UPDATE/read-snapshot
    /// sequences. The current bus read API cannot sync scheduler-driven
    /// peripherals before reads, so C3 ROM boot keeps this false and uses the
    /// legacy per-cycle tick path for fidelity.
    scheduler_enabled: bool,
    /// Bus-published cycle clock (walk-free plan Part 1). `Some` once
    /// `SystemBus::add_peripheral` attaches it. In scheduler mode the OP-update
    /// snapshot latch pulls "now" from it so a counter read is fresh even
    /// though the per-cycle walk no longer ticks this model. Not serialized —
    /// re-attached by the bus on restore.
    #[serde(skip)]
    clock: Option<CycleClock>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SystimerSnapshotV2 {
    conf: u32,
    unit0: UnitState,
    unit1: UnitState,
    cpu_clock_hz: u32,
    cpu_cycle_accum: u64,
    unit0_alarms: [AlarmState; 3],
    int_ena: u32,
    date: u32,
    target0_source: u32,
    last_tick: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SystimerSnapshotV1 {
    conf: u32,
    unit0: UnitState,
    unit1: UnitState,
    cpu_clock_hz: u32,
    cpu_cycle_accum: u64,
    unit0_alarms: [AlarmState; 3],
    int_ena: u32,
    date: u32,
    target0_source: u32,
}

impl SystimerSnapshotV2 {
    fn from_systimer(s: &Systimer) -> Self {
        Self {
            conf: s.conf,
            unit0: s.unit0,
            unit1: s.unit1,
            cpu_clock_hz: s.cpu_clock_hz,
            cpu_cycle_accum: s.cpu_cycle_accum,
            unit0_alarms: s.unit0_alarms,
            int_ena: s.int_ena,
            date: s.date,
            target0_source: s.target0_source,
            last_tick: s.last_tick,
        }
    }

    fn into_systimer(self) -> Systimer {
        Systimer {
            conf: self.conf,
            unit0: self.unit0,
            unit1: self.unit1,
            cpu_clock_hz: self.cpu_clock_hz,
            cpu_cycle_accum: self.cpu_cycle_accum,
            unit0_alarms: self.unit0_alarms,
            int_ena: self.int_ena,
            date: self.date,
            target0_source: self.target0_source,
            last_tick: self.last_tick,
            scheduler_enabled: true,
            clock: None,
        }
    }
}

impl SystimerSnapshotV1 {
    fn into_systimer(self) -> Systimer {
        Systimer {
            conf: self.conf,
            unit0: self.unit0,
            unit1: self.unit1,
            cpu_clock_hz: self.cpu_clock_hz,
            cpu_cycle_accum: self.cpu_cycle_accum,
            unit0_alarms: self.unit0_alarms,
            int_ena: self.int_ena,
            date: self.date,
            target0_source: self.target0_source,
            last_tick: 0,
            scheduler_enabled: true,
            clock: None,
        }
    }
}

impl Systimer {
    pub fn new(cpu_clock_hz: u32) -> Self {
        Self::new_with_source(cpu_clock_hz, SYSTIMER_TARGET0_SOURCE)
    }

    /// Construct with an explicit TARGET0 interrupt-matrix source ID (TARGET1/2
    /// at +1/+2). The ESP32-C3 reuses this IP but maps the comparators to
    /// matrix sources 37/38/39 rather than the S3's 57/58/59.
    pub fn new_with_source(cpu_clock_hz: u32, target0_source: u32) -> Self {
        Self {
            // SVD/silicon reset = 0x46000000:
            //   bit 30 (UNIT0_WORK_EN) = 1 → UNIT0 runs immediately at reset.
            //   bit 29 (UNIT1_WORK_EN) = 0 → UNIT1 stopped at reset.
            //   bit 31 (CLK_EN)        = 0 → peripheral clock gated at reset;
            //     firmware writes CONF to enable the clock (esp-idf / esp-hal
            //     always writes CONF before using the timer, so CLK_EN=0 at
            //     power-on is fine in practice — the unit0 tick FSM still
            //     advances because we model counter advancement when
            //     unit0_running() is true, which checks bit 30 only).
            //   bits 26/25 set (0x06000000) — reserved/status in SVD but part
            //     of the validated reset word.
            // Per-alarm enable bits (24/23/22) start cleared (no alarms armed).
            conf: 0x4600_0000,
            unit0: UnitState::default(),
            unit1: UnitState::default(),
            cpu_clock_hz,
            cpu_cycle_accum: 0,
            unit0_alarms: [AlarmState::default(); 3],
            int_ena: 0,
            // Silicon-validated date/revision register (read via OpenOCD).
            date: 0x0201_2251,
            target0_source,
            last_tick: 0,
            scheduler_enabled: true,
            clock: None,
        }
    }

    /// Construct a SYSTIMER that stays on the legacy per-cycle tick path.
    ///
    /// This is used by ESP32-C3 ROM boot until scheduler-driven peripherals can
    /// be synchronized before MMIO reads as well as before writes.
    pub fn new_with_source_legacy_tick(cpu_clock_hz: u32, target0_source: u32) -> Self {
        let mut timer = Self::new_with_source(cpu_clock_hz, target0_source);
        timer.scheduler_enabled = false;
        timer
    }

    /// Test/differential knob: pin this instance back onto the legacy per-cycle
    /// walk (`uses_scheduler() == false`). Used by the walk-on-vs-scheduler
    /// differential gate to build the reference config from the same bus
    /// assembly (mirrors `Esp32c3RtcTimer::force_legacy_walk`).
    pub fn force_legacy_walk(&mut self) {
        self.scheduler_enabled = false;
    }

    /// Scheduler mode advances the counter lazily. Pull "now" from the
    /// bus-published clock and advance the free-running counters (no alarm
    /// evaluation — alarms fire at their exact scheduled cycle via `on_event`,
    /// never off a mere counter read). Idempotent with the write-path
    /// `sync_to`, which already advanced to the same cycle before an MMIO
    /// write is observed; this keeps the OP-update snapshot fresh for any read
    /// path that reaches the latch without a preceding bus sync. No-op in
    /// legacy mode (the walk owns the counter) or without an attached clock.
    fn sync_counters_from_clock(&mut self) {
        if !self.scheduler_enabled {
            return;
        }
        let Some(now) = self.clock.as_ref().map(|c| c.now()) else {
            return;
        };
        if now > self.last_tick {
            self.advance_cycles(now - self.last_tick);
            self.last_tick = now;
        }
    }

    /// Interrupt-matrix source IDs this SYSTIMER is asserting RIGHT NOW —
    /// per-alarm `pending && INT_ENA`, the same level condition
    /// `evaluate_alarms` emits on the legacy walk. In scheduler mode the
    /// per-cycle walk no longer re-emits this level every tick, so the bus
    /// re-derives it from here (`SystemBus::refresh_esp32c3_sched_sources`) to
    /// keep the C3 interrupt matrix level-accurate.
    fn asserted_matrix_sources(&self) -> Vec<u32> {
        let mut out = Vec::new();
        for (i, alarm) in self.unit0_alarms.iter().enumerate() {
            if alarm.pending && (self.int_ena & (1 << i) != 0) {
                out.push(self.target0_source + i as u32);
            }
        }
        out
    }

    /// Restore the (non-serialized) cycle clock and re-anchor the scheduler
    /// counter to the live clock. A snapshot is typically resumed on a FRESH
    /// machine whose cycle count restarts near zero; trusting the persisted
    /// `last_tick` (potentially millions of cycles ahead) would make the next
    /// `sync_counters_from_clock` see `now <= last_tick` and freeze the counter
    /// — the same stale-anchor trap the rtc_timer resume fix (#516) avoids. The
    /// restored counter *value* (what boot-log timestamps depend on) is kept;
    /// only the advance anchor is rebased to "now".
    fn reanchor_after_restore(&mut self, clock: Option<CycleClock>) {
        if let Some(clock) = clock {
            self.last_tick = clock.now();
            self.clock = Some(clock);
        }
    }

    fn unit0_running(&self) -> bool {
        self.conf & (1 << 30) != 0
    }

    fn unit1_running(&self) -> bool {
        self.conf & (1 << 29) != 0
    }

    fn cpu_per_systimer(&self) -> u64 {
        (self.cpu_clock_hz as u64)
            .saturating_div(SYSTIMER_CLOCK_HZ)
            .max(1)
    }

    fn advance_cycles(&mut self, cycles: u64) {
        if cycles == 0 {
            return;
        }
        self.cpu_cycle_accum = self.cpu_cycle_accum.saturating_add(cycles);
        let cpu_per_systimer = self.cpu_per_systimer();
        if self.cpu_cycle_accum < cpu_per_systimer {
            return;
        }
        let ticks = self.cpu_cycle_accum / cpu_per_systimer;
        self.cpu_cycle_accum %= cpu_per_systimer;
        if self.unit0_running() {
            self.unit0.counter = self.unit0.counter.wrapping_add(ticks);
        }
        if self.unit1_running() {
            self.unit1.counter = self.unit1.counter.wrapping_add(ticks);
        }
    }

    fn evaluate_alarms(&mut self) -> Vec<u32> {
        // ── Alarm checks ──
        // For each enabled alarm, on the rising edge from `counter < target` to
        // `counter >= target` we set the pending bit and bump the target for
        // period-mode alarms. IRQ delivery is level-sensitive while
        // `pending && int_ena`, matching the legacy tick path.
        let mut explicit_irqs = Vec::new();
        let unit0_counter = self.unit0.counter;
        let unit1_counter = self.unit1.counter;
        for (i, alarm) in self.unit0_alarms.iter_mut().enumerate() {
            if !alarm.enabled {
                continue;
            }
            let counter = if alarm.unit_sel {
                unit1_counter
            } else {
                unit0_counter
            };
            if alarm.period_mode && alarm.period > 0 {
                if counter >= alarm.target {
                    alarm.pending = true;
                    alarm.target = counter.saturating_add(alarm.period);
                }
            } else if counter >= alarm.target && !alarm.edge_latched {
                alarm.edge_latched = true;
                alarm.pending = true;
            }
            if alarm.pending && (self.int_ena & (1 << i) != 0) {
                explicit_irqs.push(self.target0_source + i as u32);
            }
        }
        explicit_irqs
    }

    fn sync_to_tick(&mut self, tick_now: u64) -> Vec<u32> {
        if tick_now > self.last_tick {
            self.advance_cycles(tick_now - self.last_tick);
            self.last_tick = tick_now;
        }
        self.evaluate_alarms()
    }

    fn next_alarm_delay_cycles(&self) -> Option<u64> {
        let cpu_per_systimer = self.cpu_per_systimer();
        let mut best: Option<u64> = None;
        for (i, alarm) in self.unit0_alarms.iter().enumerate() {
            if alarm.pending && (self.int_ena & (1 << i) != 0) {
                return Some(0);
            }
            if !alarm.enabled || alarm.pending {
                continue;
            }
            let running = if alarm.unit_sel {
                self.unit1_running()
            } else {
                self.unit0_running()
            };
            if !running {
                continue;
            }
            let counter = if alarm.unit_sel {
                self.unit1.counter
            } else {
                self.unit0.counter
            };
            let systimer_ticks = alarm.target.saturating_sub(counter);
            let cycles = if systimer_ticks == 0 {
                0
            } else {
                systimer_ticks
                    .saturating_mul(cpu_per_systimer)
                    .saturating_sub(self.cpu_cycle_accum.min(cpu_per_systimer - 1))
            };
            best = Some(best.map_or(cycles, |cur| cur.min(cycles)));
        }
        best
    }

    fn read_word(&self, offset: u64) -> u32 {
        use regs::*;
        match offset {
            CONF => self.conf,
            // OP: silicon asserts VALUE_VALID after snapshot; we model instant.
            UNIT0_OP | UNIT1_OP => OP_VALUE_VALID_BIT,

            UNIT0_LOAD_HI => self.unit0.load_hi,
            UNIT0_LOAD_LO => self.unit0.load_lo,
            UNIT1_LOAD_HI => self.unit1.load_hi,
            UNIT1_LOAD_LO => self.unit1.load_lo,

            // TARGETx HI/LO: pending (most recently written), PAC convention.
            TARGET0_HI => (self.unit0_alarms[0].pending_target >> 32) as u32,
            TARGET0_LO => (self.unit0_alarms[0].pending_target & 0xFFFF_FFFF) as u32,
            TARGET1_HI => (self.unit0_alarms[1].pending_target >> 32) as u32,
            TARGET1_LO => (self.unit0_alarms[1].pending_target & 0xFFFF_FFFF) as u32,
            TARGET2_HI => (self.unit0_alarms[2].pending_target >> 32) as u32,
            TARGET2_LO => (self.unit0_alarms[2].pending_target & 0xFFFF_FFFF) as u32,

            TARGET0_CONF => alarm_conf_word_pending(&self.unit0_alarms[0]),
            TARGET1_CONF => alarm_conf_word_pending(&self.unit0_alarms[1]),
            TARGET2_CONF => alarm_conf_word_pending(&self.unit0_alarms[2]),

            UNIT0_VALUE_HI => (self.unit0.snapshot >> 32) as u32,
            UNIT0_VALUE_LO => (self.unit0.snapshot & 0xFFFF_FFFF) as u32,
            UNIT1_VALUE_HI => (self.unit1.snapshot >> 32) as u32,
            UNIT1_VALUE_LO => (self.unit1.snapshot & 0xFFFF_FFFF) as u32,

            // COMPx_LOAD / UNITx_LOAD are write-only; reads return 0.
            INT_ENA => self.int_ena,
            INT_RAW => self.int_raw_word(),
            INT_ST => self.int_raw_word() & self.int_ena,

            REAL_TARGET0_LO => (self.unit0_alarms[0].target & 0xFFFF_FFFF) as u32,
            REAL_TARGET0_HI => ((self.unit0_alarms[0].target >> 32) & 0x000F_FFFF) as u32,
            REAL_TARGET1_LO => (self.unit0_alarms[1].target & 0xFFFF_FFFF) as u32,
            REAL_TARGET1_HI => ((self.unit0_alarms[1].target >> 32) & 0x000F_FFFF) as u32,
            REAL_TARGET2_LO => (self.unit0_alarms[2].target & 0xFFFF_FFFF) as u32,
            REAL_TARGET2_HI => ((self.unit0_alarms[2].target >> 32) & 0x000F_FFFF) as u32,

            DATE => self.date,

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
        use regs::*;
        match offset {
            CONF => {
                self.conf = value;
                self.sync_alarm_enables_from_conf();
            }
            UNIT0_OP if value & OP_UPDATE_BIT != 0 => {
                self.sync_counters_from_clock();
                self.unit0.snapshot = self.unit0.counter;
            }
            UNIT1_OP if value & OP_UPDATE_BIT != 0 => {
                self.sync_counters_from_clock();
                self.unit1.snapshot = self.unit1.counter;
            }

            UNIT0_LOAD_HI => self.unit0.load_hi = value,
            UNIT0_LOAD_LO => self.unit0.load_lo = value,
            UNIT1_LOAD_HI => self.unit1.load_hi = value,
            UNIT1_LOAD_LO => self.unit1.load_lo = value,

            // Stage pending target; commit on COMPx_LOAD.
            TARGET0_HI => set_pending_target_hi(&mut self.unit0_alarms[0], value),
            TARGET0_LO => set_pending_target_lo(&mut self.unit0_alarms[0], value),
            TARGET1_HI => set_pending_target_hi(&mut self.unit0_alarms[1], value),
            TARGET1_LO => set_pending_target_lo(&mut self.unit0_alarms[1], value),
            TARGET2_HI => set_pending_target_hi(&mut self.unit0_alarms[2], value),
            TARGET2_LO => set_pending_target_lo(&mut self.unit0_alarms[2], value),

            TARGET0_CONF => set_alarm_conf(&mut self.unit0_alarms[0], value),
            TARGET1_CONF => set_alarm_conf(&mut self.unit0_alarms[1], value),
            TARGET2_CONF => set_alarm_conf(&mut self.unit0_alarms[2], value),

            COMP0_LOAD if value & 1 != 0 => {
                self.commit_alarm(0);
            }
            COMP1_LOAD if value & 1 != 0 => {
                self.commit_alarm(1);
            }
            COMP2_LOAD if value & 1 != 0 => {
                self.commit_alarm(2);
            }

            UNIT0_LOAD if value & 1 != 0 => {
                self.unit0.counter =
                    ((self.unit0.load_hi as u64) << 32) | (self.unit0.load_lo as u64);
            }
            UNIT1_LOAD if value & 1 != 0 => {
                self.unit1.counter =
                    ((self.unit1.load_hi as u64) << 32) | (self.unit1.load_lo as u64);
            }

            INT_ENA => self.int_ena = value & 0x7,
            // INT_RAW is read-only on real silicon; ignore writes.
            INT_CLR => {
                for (i, alarm) in self.unit0_alarms.iter_mut().enumerate() {
                    if value & (1 << i) != 0 {
                        alarm.pending = false;
                    }
                }
            }
            // INT_ST / REAL_TARGETx are read-only; ignore writes.
            DATE => self.date = value,

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
    /// Freerunning snapshot path only — offsets from [`regs::FREERUNNING_POLL`].
    fn mmio_access_class(&self, offset: u64) -> MmioAccessClass {
        let word = offset & !3;
        if regs::FREERUNNING_POLL.contains(&word) {
            MmioAccessClass::FreerunningTimerPoll
        } else {
            MmioAccessClass::SideEffecting
        }
    }

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
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        let explicit_irqs = self.sync_to_tick(self.last_tick.saturating_add(cycles));
        PeripheralTickResult {
            explicit_irqs: if explicit_irqs.is_empty() {
                None
            } else {
                Some(explicit_irqs)
            },
            ..PeripheralTickResult::default()
        }
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_enabled
    }

    fn sync_to(&mut self, tick_now: u64) {
        self.sync_to_tick(tick_now);
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        // Anchor at the clock's current value so cycles elapsed before attach
        // (normally zero — attach happens at bus assembly) are not
        // retroactively credited to the counter (mirrors the rtc_timer #516
        // re-anchor contract).
        self.last_tick = clock.now();
        self.clock = Some(clock);
    }

    /// C3 interrupt-matrix level: the source IDs this SYSTIMER is asserting
    /// now. Empty unless an alarm is `pending && INT_ENA`. The bus polls this
    /// from the event path and the walk-tick aggregation so a scheduler-driven
    /// SYSTIMER keeps its level-sensitive IRQ routed even though the per-cycle
    /// walk no longer re-emits it.
    fn matrix_irq_sources(&self) -> Vec<u32> {
        self.asserted_matrix_sources()
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_enabled {
            return Vec::new();
        }
        self.next_alarm_delay_cycles()
            .map(|delay| vec![(delay, SYSTIMER_WAKE_TOKEN)])
            .unwrap_or_default()
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let explicit_irqs = self.sync_to_tick(sched.now());
        crate::sched::EventResult {
            explicit_irqs,
            reschedule_delay: self.next_alarm_delay_cycles(),
            ..Default::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    /// Capture the full 64-bit counter + comparator state. This is the source
    /// behind `esp_timer` / `esp_log_timestamp`, so a rom-boot resume that did
    /// not restore it would print different IDF log timestamps than the cold
    /// boot — the counter must carry across the snapshot for byte-exact serial.
    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(&SystimerSnapshotV2::from_systimer(self))
            .expect("bincode serialize Systimer")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        let scheduler_enabled = self.scheduler_enabled;
        let clock = self.clock.clone();
        if let Ok(restored) = bincode::deserialize::<SystimerSnapshotV2>(bytes) {
            *self = restored.into_systimer();
            self.scheduler_enabled = scheduler_enabled;
            self.reanchor_after_restore(clock);
            return Ok(());
        }
        let restored = bincode::deserialize::<SystimerSnapshotV1>(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("Systimer snapshot decode: {e}"))
        })?;
        *self = restored.into_systimer();
        self.scheduler_enabled = scheduler_enabled;
        self.reanchor_after_restore(clock);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let s = Systimer::new(80_000_000);
        // SVD/silicon reset: bit30 (UNIT0_WORK_EN) set, bit29/31 clear,
        // plus reserved bits 26/25 set → 0x46000000.
        assert_eq!(s.conf, 0x4600_0000);
        assert_eq!(s.unit0.counter, 0);
        assert_eq!(s.unit1.counter, 0);
    }

    #[test]
    fn conf_reset_is_silicon_default() {
        // Silicon-validated: CONF reads 0x46000000 on a freshly-reset device.
        let s = Systimer::new(80_000_000);
        assert_eq!(
            s.read_word(0x00),
            0x4600_0000,
            "CONF reset must match SVD/silicon value"
        );
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
    fn tick_elapsed_matches_repeated_tick_for_counter_and_alarm() {
        let mut repeated = Systimer::new_with_source(160_000_000, 37);
        let mut elapsed = Systimer::new_with_source(160_000_000, 37);

        repeated.write_word(0x64, 1);
        elapsed.write_word(0x64, 1);

        repeated.write_word(0x1C, 0);
        repeated.write_word(0x20, 2);
        repeated.write_word(0x50, 1);
        repeated.write_word(0x00, repeated.read_word(0x00) | (1 << 24));

        elapsed.write_word(0x1C, 0);
        elapsed.write_word(0x20, 2);
        elapsed.write_word(0x50, 1);
        elapsed.write_word(0x00, elapsed.read_word(0x00) | (1 << 24));

        let mut repeated_irq = None;
        for _ in 0..30 {
            repeated_irq = repeated.tick().explicit_irqs;
        }
        let elapsed_irq = elapsed.tick_elapsed(30).explicit_irqs;

        assert_eq!(elapsed.unit0.counter, repeated.unit0.counter);
        assert_eq!(elapsed.read_word(0x68), repeated.read_word(0x68));
        assert_eq!(elapsed_irq, repeated_irq);
    }

    #[test]
    fn alarm_arm_schedules_next_deadline() {
        let mut s = Systimer::new_with_source(160_000_000, 37);
        s.write_word(0x64, 1);
        s.write_word(0x1C, 0);
        s.write_word(0x20, 3);
        s.write_word(0x50, 1);
        s.write_word(0x00, s.read_word(0x00) | (1 << 24));

        assert!(s.uses_scheduler());
        assert_eq!(s.take_scheduled_events(), vec![(30, 0)]);
    }

    #[test]
    fn c3_legacy_constructor_stays_off_scheduler() {
        let mut s = Systimer::new_with_source_legacy_tick(160_000_000, 37);

        assert!(!s.uses_scheduler());
        assert!(s.take_scheduled_events().is_empty());

        s.tick_elapsed(10);
        assert_eq!(
            s.unit0.counter, 1,
            "legacy mode must still advance the live counter through ticks"
        );
    }

    #[test]
    fn restore_preserves_current_scheduler_mode() {
        let mut original = Systimer::new_with_source(160_000_000, 37);
        original.tick_elapsed(25);
        let bytes = original.runtime_snapshot();

        let mut restored = Systimer::new_with_source_legacy_tick(160_000_000, 37);
        restored.restore_runtime_snapshot(&bytes).unwrap();

        assert_eq!(restored.unit0.counter, original.unit0.counter);
        assert!(
            !restored.uses_scheduler(),
            "restore must not flip a C3 legacy-tick instance back to scheduler mode"
        );
    }

    #[test]
    fn restore_accepts_pre_scheduler_snapshot() {
        let mut original = Systimer::new_with_source(160_000_000, 37);
        original.tick_elapsed(25);
        let old = SystimerSnapshotV1 {
            conf: original.conf,
            unit0: original.unit0,
            unit1: original.unit1,
            cpu_clock_hz: original.cpu_clock_hz,
            cpu_cycle_accum: original.cpu_cycle_accum,
            unit0_alarms: original.unit0_alarms,
            int_ena: original.int_ena,
            date: original.date,
            target0_source: original.target0_source,
        };
        let bytes = bincode::serialize(&old).unwrap();

        let mut restored = Systimer::new_with_source(80_000_000, 57);
        restored.restore_runtime_snapshot(&bytes).unwrap();

        assert_eq!(restored.unit0.counter, original.unit0.counter);
        assert_eq!(restored.cpu_cycle_accum, original.cpu_cycle_accum);
        assert_eq!(restored.target0_source, 37);
        assert_eq!(restored.last_tick, 0);
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
        // Silicon reset has unit1 disabled (bit 29 = 0); enable it explicitly.
        s.write_word(0x00, s.read_word(0x00) | (1 << 29));
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
            assert!(
                r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()),
                "no fire before counter reaches target"
            );
        }
        let r = s.tick();
        assert_eq!(
            r.explicit_irqs.as_deref(),
            Some(&[57][..]),
            "TARGET0 source ID at counter==target"
        );
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
            assert!(r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()));
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
            assert!(
                r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()),
                "no IRQ when INT_ENA=0"
            );
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
        assert_eq!(
            r.explicit_irqs.as_deref(),
            Some(&[57][..]),
            "level-sensitive re-emit"
        );
        let r = s.tick();
        assert_eq!(r.explicit_irqs.as_deref(), Some(&[57][..]));
        // INT_CLR de-asserts the level → no more emits.
        s.write_word(0x6C, 1);
        let r = s.tick();
        assert!(
            r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()),
            "after INT_CLR, no more IRQs"
        );
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
            if !r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()) {
                first_fire = Some(cycle + 1);
                break;
            }
        }
        assert_eq!(
            first_fire,
            Some(50),
            "first fire at counter==10 → 50 CPU cycles"
        );
        // Target should have been bumped by period: 10 + 10 = 20.
        assert_eq!(s.unit0_alarms[0].target, 20);
        s.write_word(0x6C, 1); // clear pending
        let mut second_fire = None;
        for cycle in 0..100 {
            let r = s.tick();
            if !r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()) {
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
            assert!(s.tick().explicit_irqs.as_ref().is_none_or(|v| v.is_empty()));
        }
    }

    // ── New faithful-coverage tests (CONF/DATE/REAL_TARGET/UNIT_LOAD/INT_CLR) ──

    #[test]
    fn date_reads_silicon_value() {
        // DATE (0xFC) must return the HW-validated silicon reset value.
        let s = Systimer::new(80_000_000);
        assert_eq!(
            s.read_word(0xFC),
            0x0201_2251,
            "DATE reset must match HW-validated value 0x02012251"
        );
    }

    #[test]
    fn date_is_rw_storage() {
        let mut s = Systimer::new(80_000_000);
        s.write_word(0xFC, 0xDEAD_BEEF);
        assert_eq!(s.read_word(0xFC), 0xDEAD_BEEF);
        // Restore and verify reset value readable again after fresh instance.
        let s2 = Systimer::new(80_000_000);
        assert_eq!(s2.read_word(0xFC), 0x0201_2251);
    }

    #[test]
    fn unit_load_trigger_loads_counter() {
        // UNIT0_LOAD (0x5C): write 1 → loads counter from UNIT0_LOAD_HI/LO.
        // UNIT1_LOAD (0x60): same for UNIT1.
        let mut s = Systimer::new(80_000_000);

        // Stage UNIT0 load: 0x0000_0001_0000_ABCD
        s.write_word(0x0C, 0x0000_0001); // UNIT0_LOAD_HI
        s.write_word(0x10, 0x0000_ABCD); // UNIT0_LOAD_LO
        s.write_word(0x5C, 1); // trigger
        assert_eq!(
            s.unit0.counter,
            (1u64 << 32) | 0xABCD,
            "UNIT0 counter loaded from LOAD_HI/LO"
        );

        // Stage UNIT1 load: enable UNIT1 first so snapshot is meaningful.
        s.write_word(0x14, 0x0000_0002); // UNIT1_LOAD_HI
        s.write_word(0x18, 0x0001_2345); // UNIT1_LOAD_LO
        s.write_word(0x60, 1); // trigger
        assert_eq!(
            s.unit1.counter,
            (2u64 << 32) | 0x0001_2345,
            "UNIT1 counter loaded from LOAD_HI/LO"
        );
    }

    #[test]
    fn int_clr_clears_alarm_bits() {
        // Write-1-to-clear: writing INT_CLR clears the matching INT_RAW bits.
        let mut s = Systimer::new(80_000_000);
        // Arm all three alarms in period mode with short periods.
        for (conf_off, load_off, conf_bits) in [
            (0x34u64, 0x50u64, CONF_TARGET0_WORK_EN),
            (0x38u64, 0x54u64, CONF_TARGET1_WORK_EN),
            (0x3Cu64, 0x58u64, CONF_TARGET2_WORK_EN),
        ] {
            s.write_word(conf_off, ALARM_PERIOD_MODE_BIT | 1);
            s.write_word(load_off, 1);
            s.write_word(0x00, s.read_word(0x00) | conf_bits);
        }
        // Let all three fire.
        for _ in 0..10 {
            s.tick();
        }
        assert_eq!(s.read_word(0x68), 0x7, "all three INT_RAW bits set");
        // Clear bits 0 and 2 only.
        s.write_word(0x6C, 0x5); // bits 0 and 2
        assert_eq!(
            s.read_word(0x68),
            0x2,
            "only INT_RAW bit 1 remains after clearing 0 and 2"
        );
        // Clear remaining.
        s.write_word(0x6C, 0x7);
        assert_eq!(s.read_word(0x68), 0x0, "all INT_RAW bits cleared");
    }

    #[test]
    fn real_target_reflects_committed_target() {
        // REAL_TARGETx_LO/HI (0x74..0x88) must return the live committed
        // target for each alarm (not the pending staged value).
        let mut s = Systimer::new(80_000_000);

        // Alarm 0: target = 0x0000_0001_CAFE_BABE
        s.write_word(0x1C, 0x0000_0001); // TARGET0_HI pending
        s.write_word(0x20, 0xCAFE_BABE); // TARGET0_LO pending
        s.write_word(0x50, 1); // COMP0_LOAD — commit
        assert_eq!(s.read_word(0x74), 0xCAFE_BABE, "REAL_TARGET0_LO");
        assert_eq!(s.read_word(0x78), 0x0000_0001, "REAL_TARGET0_HI");

        // Alarm 1: target = 0x0000_0000_1234_5678
        s.write_word(0x24, 0x0000_0000); // TARGET1_HI pending
        s.write_word(0x28, 0x1234_5678); // TARGET1_LO pending
        s.write_word(0x54, 1); // COMP1_LOAD — commit
        assert_eq!(s.read_word(0x7C), 0x1234_5678, "REAL_TARGET1_LO");
        assert_eq!(s.read_word(0x80), 0x0000_0000, "REAL_TARGET1_HI");

        // Alarm 2: target = 0x000F_FFFF_FFFF_FFFF (max 52-bit value)
        s.write_word(0x2C, 0x000F_FFFF); // TARGET2_HI pending
        s.write_word(0x30, 0xFFFF_FFFF); // TARGET2_LO pending
        s.write_word(0x58, 1); // COMP2_LOAD — commit
        assert_eq!(s.read_word(0x84), 0xFFFF_FFFF, "REAL_TARGET2_LO");
        assert_eq!(s.read_word(0x88), 0x000F_FFFF, "REAL_TARGET2_HI");
    }

    #[test]
    fn real_target_before_comp_load_is_zero() {
        // Before any COMP_LOAD, live target = 0 (pending hasn't committed).
        let mut s = Systimer::new(80_000_000);
        s.write_word(0x1C, 0x0000_0001); // pending HI
        s.write_word(0x20, 0xDEAD_BEEF); // pending LO — NOT committed yet
        assert_eq!(
            s.read_word(0x74),
            0,
            "REAL_TARGET0_LO is 0 before COMP_LOAD"
        );
        assert_eq!(
            s.read_word(0x78),
            0,
            "REAL_TARGET0_HI is 0 before COMP_LOAD"
        );
    }

    #[test]
    fn conf_write_read_round_trip() {
        // CONF writes update the register; reads return the stored value.
        let mut s = Systimer::new(80_000_000);
        // Set unit0+unit1 running and alarm 0 enabled (bits 30, 29, 24).
        let new_conf = 0x4600_0000u32 | (1 << 29) | (1 << 24);
        s.write_word(0x00, new_conf);
        assert_eq!(
            s.read_word(0x00),
            new_conf,
            "CONF round-trips through write/read"
        );
        assert!(s.unit0_running(), "unit0 running after CONF write");
        assert!(s.unit1_running(), "unit1 running after CONF write");
        assert!(
            s.unit0_alarms[0].enabled,
            "alarm 0 enabled after CONF write"
        );
    }
}

#[cfg(test)]
mod regs_svd_tests {
    //! Offline gate: `systimer_regs.rs` must match the checked-in S3 SVD.
    //! Regenerate with: `python3 scripts/gen_esp_systimer_regs.py`

    use super::regs;

    fn svd_xml() -> String {
        // labwired-core package lives at crates/core; fixtures at tests/fixtures.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let candidates = [
            root.join("../../tests/fixtures/svd/esp32s3.svd"),
            root.join("tests/fixtures/svd/esp32s3.svd"),
        ];
        for p in &candidates {
            if p.exists() {
                return std::fs::read_to_string(p).expect("read svd");
            }
        }
        panic!(
            "esp32s3.svd not found; tried {:?}",
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn generated_offsets_match_svd() {
        let xml = svd_xml();
        // Parse SYSTIMER peripheral offsets from SVD (lightweight).
        let mut in_systimer = false;
        let mut depth = 0i32;
        let mut svd_map: std::collections::HashMap<String, u64> = Default::default();
        let mut cur_name: Option<String> = None;
        for line in xml.lines() {
            let t = line.trim();
            if t.contains("<name>SYSTIMER</name>") {
                in_systimer = true;
                depth = 0;
                continue;
            }
            if !in_systimer {
                continue;
            }
            if t.starts_with("<peripheral") {
                depth += 1;
            }
            if t.starts_with("</peripheral>") {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            if let Some(n) = t
                .strip_prefix("<name>")
                .and_then(|s| s.strip_suffix("</name>"))
            {
                if !n.contains("INT_MAP") {
                    cur_name = Some(n.to_string());
                }
            }
            if let Some(off) = t
                .strip_prefix("<addressOffset>")
                .and_then(|s| s.strip_suffix("</addressOffset>"))
            {
                if let Some(name) = cur_name.take() {
                    let v = u64::from_str_radix(
                        off.trim_start_matches("0x").trim_start_matches("0X"),
                        16,
                    )
                    .or_else(|_| off.parse::<u64>())
                    .unwrap_or_else(|_| panic!("bad offset {off} for {name}"));
                    svd_map.insert(name, v);
                }
            }
        }
        assert!(!svd_map.is_empty(), "parsed no SYSTIMER regs from SVD");

        let expected = [
            ("CONF", regs::CONF),
            ("UNIT0_OP", regs::UNIT0_OP),
            ("UNIT1_OP", regs::UNIT1_OP),
            ("UNIT0_VALUE_HI", regs::UNIT0_VALUE_HI),
            ("UNIT0_VALUE_LO", regs::UNIT0_VALUE_LO),
            ("UNIT1_VALUE_HI", regs::UNIT1_VALUE_HI),
            ("UNIT1_VALUE_LO", regs::UNIT1_VALUE_LO),
            ("DATE", regs::DATE),
            ("INT_ENA", regs::INT_ENA),
            ("REAL_TARGET0_LO", regs::REAL_TARGET0_LO),
        ];
        for (name, got) in expected {
            let want = *svd_map
                .get(name)
                .unwrap_or_else(|| panic!("SVD missing register {name}"));
            assert_eq!(got, want, "offset mismatch for {name}");
        }
        for &off in regs::FREERUNNING_POLL {
            assert!(
                svd_map.values().any(|&v| v == off),
                "FREERUNNING_POLL offset {off:#x} not in SVD map"
            );
        }
    }
}
