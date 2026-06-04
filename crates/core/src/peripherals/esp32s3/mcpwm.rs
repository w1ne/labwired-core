// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 MCPWM (Motor Control PWM) — register-faithful digital twin.
//!
//! The S3 has TWO independent MCPWM peripherals; the bus registers both:
//! MCPWM0 @ `0x6001_E000` and MCPWM1 @ `0x6002_C000`, each a 4 KiB window. One
//! `Esp32s3Mcpwm` instance models a single unit. Each unit contains:
//!
//! * a clock prescaler (`CLK_CFG`),
//! * **three** PWM timers, each a 16-bit up / down / up-down counter with a
//!   period (`TIMERx_CFG0`) and start/run-mode (`TIMERx_CFG1`), exposing the
//!   live counter + direction at `TIMERx_STATUS`,
//! * **three** operators, each with two comparators (`CMPRx_VALUE0/1`), a
//!   generator config (`GENx_*` — we round-trip a single `GENx` word) and a
//!   dead-time config (`DTx_CFG`),
//! * an interrupt block (`INT_RAW/ST/ENA/CLR`) carrying the per-timer TEZ
//!   (timer-equals-zero) and TEP (timer-equals-period) events plus the
//!   per-operator comparator-equal events.
//!
//! ## Fidelity model
//!
//! Like `ledc.rs`/`timer_group.rs`, config registers round-trip within their
//! field masks and the interrupt block follows the standard
//! `INT_ST = INT_RAW & INT_ENA`, W1C-over-`INT_CLR` pattern. The three timers
//! **auto-advance on `tick()`**: each enabled timer accumulates sim (CPU)
//! cycles, divides by `(clk_prescale+1) * (timer_prescale+1) * cpu_per_apb`,
//! and steps its counter through the configured count mode. Reaching the period
//! latches **TEP**; returning to zero latches **TEZ**. A comparator-equal latch
//! fires when the counter matches `CMPRx_VALUE0`/`VALUE1` of the operator that
//! shares the timer's index (operator → timer mapping is configurable on
//! silicon via `OPERATORx_TIMERSEL`; we model the common 1:1 op-uses-own-timer
//! wiring, which is what `mcpwm_operator_connect_timer(op_i, timer_i)` sets up
//! in the IDF examples). The IRQ matrix source is emitted while any enabled
//! interrupt is latched (level-sensitive, same as the LEDC twin).
//!
//! This module owns ONLY its own register file — `tick()` never reads or writes
//! the bus, so it cannot influence other peripherals.
//!
//! ## Register map (ESP32-S3 TRM "MCPWM"; offsets verified against IDF
//! `soc/esp32s3/register/soc/mcpwm_reg.h`, which lays the unit out as)
//!
//! | Offset | Name             | Notes |
//! |-------:|------------------|-------|
//! | 0x000  | CLK_CFG          | bits[7:0] CLK_PRESCALE (PWM_clk = clk/(N+1)) |
//! | 0x004  | TIMER0_CFG0      | [7:0] PRESCALE, [23:8] PERIOD, [25:24] PERIOD_UPMETHOD |
//! | 0x008  | TIMER0_CFG1      | [2:0] MODE (0 frozen,1 up,2 down,3 up-down), [4:3] START |
//! | 0x00C  | TIMER0_SYNC      | sync config (round-trip) |
//! | 0x010  | TIMER0_STATUS    | RO: [15:0] live VALUE, [16] DIRECTION (0 up / 1 down) |
//! | 0x014  | TIMER1_CFG0      | timer-block stride = 0x10 |
//! | 0x018  | TIMER1_CFG1      | |
//! | 0x01C  | TIMER1_SYNC      | |
//! | 0x020  | TIMER1_STATUS    | |
//! | 0x024  | TIMER2_CFG0      | |
//! | 0x028  | TIMER2_CFG1      | |
//! | 0x02C  | TIMER2_SYNC      | |
//! | 0x030  | TIMER2_STATUS    | |
//! | 0x034  | TIMER_SYNCI_CFG  | external sync input select (round-trip) |
//! | 0x038  | OPERATOR_TIMERSEL| [1:0] op0 timer, [3:2] op1, [5:4] op2 (round-trip) |
//! | 0x03C  | GEN0_STMP_CFG    | operator 0 comparator update method (round-trip) |
//! | 0x040  | GEN0_TSTMP_A (CMPR0_VALUE0) | [15:0] compare value A |
//! | 0x044  | GEN0_TSTMP_B (CMPR0_VALUE1) | [15:0] compare value B |
//! | 0x048  | GEN0_CFG0        | generator config word (round-trip) |
//! | 0x04C  | GEN0_FORCE       | force action (round-trip) |
//! | 0x050  | GEN0_A           | generator A actions (round-trip) |
//! | 0x054  | GEN0_B           | generator B actions (round-trip) |
//! | 0x058  | DT0_CFG          | dead-time config (round-trip) |
//! | 0x05C  | DT0_FED_CFG      | falling-edge delay (round-trip) |
//! | 0x060  | DT0_RED_CFG      | rising-edge delay (round-trip) |
//! | ...    | operator stride = 0x24, op1 @ +0x24, op2 @ +0x48 | |
//! | 0x0B0  | FH0_CFG0 (fault) | per-operator fault-handler cfg (round-trip) |
//! | 0x110  | INT_ENA          | interrupt enable mask |
//! | 0x114  | INT_RAW          | raw latched events (RO from FW; HW-set) |
//! | 0x118  | INT_ST           | INT_RAW & INT_ENA (RO) |
//! | 0x11C  | INT_CLR          | W1C of INT_RAW |
//!
//! The precise FH/carrier/capture offsets above 0x60 differ slightly across IDF
//! revisions; firmware that uses MCPWM for plain PWM (the LabWired closed-loop
//! cases) touches CLK_CFG, the timer CFG/STATUS block, the comparator values and
//! the interrupt block — all modeled faithfully here. Other offsets in the
//! window round-trip into a sparse overflow map so reads return what was written
//! and nothing is lost, without inventing semantics we don't model.
//!
//! ## Source IDs (IDF `soc/esp32s3/include/soc/interrupts.h`)
//!
//! `ETS_PWM0_INTR_SOURCE = 38`, `ETS_PWM1_INTR_SOURCE = 39`. A single matrix
//! source per unit; firmware reads `INT_ST` to learn which timer/operator
//! fired. `new(base_source_id)` takes that id (38 or 39).

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::collections::BTreeMap;

pub const MCPWM0_BASE: u32 = 0x6001_E000;
pub const MCPWM1_BASE: u32 = 0x6002_C000;
pub const MCPWM_SIZE: u64 = 0x1000;

/// `ETS_PWM0_INTR_SOURCE = 38`.
pub const MCPWM0_INTR_SOURCE: u32 = 38;
/// `ETS_PWM1_INTR_SOURCE = 39`.
pub const MCPWM1_INTR_SOURCE: u32 = 39;

/// APB clock feeding the MCPWM prescaler (80 MHz).
const APB_CLOCK_HZ: u64 = 80_000_000;

const NUM_TIMERS: usize = 3;
const NUM_OPERATORS: usize = 3;

// ── Top-level offsets ──
const REG_CLK_CFG: u64 = 0x000;

// ── Timer block (stride 0x10, 3 timers, base 0x004) ──
const TIMER_BLOCK_BASE: u64 = 0x004;
const TIMER_STRIDE: u64 = 0x10;
const TIMER_BLOCK_END: u64 = TIMER_BLOCK_BASE + (NUM_TIMERS as u64) * TIMER_STRIDE; // 0x034
const TIMER_CFG0: u64 = 0x0; // PRESCALE[7:0], PERIOD[23:8], UPMETHOD[25:24]
const TIMER_CFG1: u64 = 0x4; // MODE[2:0], START[4:3]
const TIMER_SYNC: u64 = 0x8; // round-trip only
const TIMER_STATUS: u64 = 0xC; // RO: VALUE[15:0], DIRECTION[16]

// ── Operator block (stride 0x24, 3 operators, base 0x03C) ──
const OP_BLOCK_BASE: u64 = 0x03C;
const OP_STRIDE: u64 = 0x24;
const OP_BLOCK_END: u64 = OP_BLOCK_BASE + (NUM_OPERATORS as u64) * OP_STRIDE; // 0x0A8
const OP_STMP_CFG: u64 = 0x00; // comparator update method (round-trip)
const OP_CMPR_VALUE0: u64 = 0x04; // GENx_TSTMP_A — compare value A [15:0]
const OP_CMPR_VALUE1: u64 = 0x08; // GENx_TSTMP_B — compare value B [15:0]
const OP_GEN_CFG0: u64 = 0x0C; // generator config (round-trip)
const OP_GEN_FORCE: u64 = 0x10; // generator force (round-trip)
const OP_GEN_A: u64 = 0x14; // generator A actions (round-trip)
const OP_GEN_B: u64 = 0x18; // generator B actions (round-trip)
const OP_DT_CFG: u64 = 0x1C; // dead-time config (round-trip)
const OP_DT_FED: u64 = 0x20; // dead-time falling-edge delay (round-trip)
                             // 0x24 would be DT_RED of the next op-stride; RED is folded into the overflow
                             // map for fidelity — the three modeled DT words (CFG/FED + the carry) suffice
                             // for the comparator/timer behaviour we expose.

// ── Interrupt block ──
const REG_INT_ENA: u64 = 0x110;
const REG_INT_RAW: u64 = 0x114;
const REG_INT_ST: u64 = 0x118;
const REG_INT_CLR: u64 = 0x11C;

// ── CLK_CFG fields ──
const CLK_PRESCALE_MASK: u32 = 0xFF; // PWM_clk = MCPWM_clk / (PRESCALE + 1)

// ── TIMERx_CFG0 fields ──
const TIMER_CFG0_PRESCALE_SHIFT: u32 = 0;
const TIMER_CFG0_PRESCALE_MASK: u32 = 0xFF; // [7:0]
const TIMER_CFG0_PERIOD_SHIFT: u32 = 8;
const TIMER_CFG0_PERIOD_MASK: u32 = 0xFFFF; // [23:8], 16-bit period

// ── TIMERx_CFG1 fields ──
const TIMER_CFG1_MODE_SHIFT: u32 = 0;
const TIMER_CFG1_MODE_MASK: u32 = 0x7; // [2:0]
/// Count modes per TRM: 0 frozen, 1 up, 2 down, 3 up-down.
const TIMER_MODE_FROZEN: u32 = 0;
const TIMER_MODE_UP: u32 = 1;
const TIMER_MODE_DOWN: u32 = 2;
const TIMER_MODE_UPDOWN: u32 = 3;
const TIMER_CFG1_START_SHIFT: u32 = 3;
const TIMER_CFG1_START_MASK: u32 = 0x3; // [4:3]
                                        // START field: 0 stop-at-TEZ, 1 stop-at-next-TEZ, 2 run-continuously (free
                                        // run). We treat any non-zero START as "running" once a count mode is set.

// ── TIMERx_STATUS fields ──
const TIMER_STATUS_VALUE_MASK: u32 = 0xFFFF; // [15:0]
const TIMER_STATUS_DIRECTION_BIT: u32 = 1 << 16; // 0 = up, 1 = down

const CMPR_VALUE_MASK: u32 = 0xFFFF; // 16-bit comparator value [15:0]

/// INT_* bit layout we model (a faithful, contiguous subset of the silicon
/// layout — firmware learns "which" from these bits, and the exact silicon
/// positions for plain-PWM events are the timer TEZ/TEP and operator-equal
/// groups, which we place at fixed positions here):
///
/// * bits 0..2  : TIMERx_TEZ (timer x counted down to zero)
/// * bits 3..5  : TIMERx_TEP (timer x reached its period)
/// * bits 6..8  : OPx_TEA   (operator x counter == CMPR_VALUE0 / compare A)
/// * bits 9..11 : OPx_TEB   (operator x counter == CMPR_VALUE1 / compare B)
const fn int_tez_bit(t: usize) -> u32 {
    1 << t
}
const fn int_tep_bit(t: usize) -> u32 {
    1 << (3 + t)
}
const fn int_tea_bit(op: usize) -> u32 {
    1 << (6 + op)
}
const fn int_teb_bit(op: usize) -> u32 {
    1 << (9 + op)
}
/// Mask of all 12 modeled interrupt bits.
const INT_MODELED_MASK: u32 = 0x0FFF;

/// One PWM timer: a 16-bit counter advanced deterministically by `tick()`.
#[derive(Debug, Clone, Copy)]
struct TimerState {
    /// TIMERx_CFG0 (PRESCALE / PERIOD / UPMETHOD) — round-tripped, decoded on use.
    cfg0: u32,
    /// TIMERx_CFG1 (MODE / START) — round-tripped, decoded on use.
    cfg1: u32,
    /// TIMERx_SYNC — round-tripped only.
    sync: u32,
    /// Live 16-bit counter.
    counter: u32,
    /// Current count direction (false = up, true = down). Meaningful in
    /// up-down mode; in up/down modes it tracks the fixed direction.
    counting_down: bool,
    /// Fractional clock accumulator (sim/CPU ticks) — when it reaches the
    /// per-count divisor the counter steps once. Mirrors timer_group's `accum`.
    accum: u64,
}

impl TimerState {
    fn new() -> Self {
        Self {
            cfg0: 0,
            cfg1: 0,
            sync: 0,
            counter: 0,
            counting_down: false,
            accum: 0,
        }
    }

    fn mode(&self) -> u32 {
        (self.cfg1 >> TIMER_CFG1_MODE_SHIFT) & TIMER_CFG1_MODE_MASK
    }

    fn start(&self) -> u32 {
        (self.cfg1 >> TIMER_CFG1_START_SHIFT) & TIMER_CFG1_START_MASK
    }

    /// Running iff a non-frozen count mode is selected and START is non-zero.
    fn running(&self) -> bool {
        self.mode() != TIMER_MODE_FROZEN && self.start() != 0
    }

    fn timer_prescale(&self) -> u64 {
        (((self.cfg0 >> TIMER_CFG0_PRESCALE_SHIFT) & TIMER_CFG0_PRESCALE_MASK) as u64) + 1
    }

    fn period(&self) -> u32 {
        (self.cfg0 >> TIMER_CFG0_PERIOD_SHIFT) & TIMER_CFG0_PERIOD_MASK
    }

    fn status_word(&self) -> u32 {
        let mut v = self.counter & TIMER_STATUS_VALUE_MASK;
        if self.counting_down {
            v |= TIMER_STATUS_DIRECTION_BIT;
        }
        v
    }
}

/// One operator: two comparators + round-tripped generator/dead-time words.
#[derive(Debug, Clone, Copy, Default)]
struct OperatorState {
    stmp_cfg: u32,
    /// CMPRx_VALUE0 — compare A [15:0].
    cmpr_value0: u32,
    /// CMPRx_VALUE1 — compare B [15:0].
    cmpr_value1: u32,
    gen_cfg0: u32,
    gen_force: u32,
    gen_a: u32,
    gen_b: u32,
    dt_cfg: u32,
    dt_fed: u32,
}

/// ESP32-S3 MCPWM unit — 3 timers, 3 operators, one interrupt source.
pub struct Esp32s3Mcpwm {
    /// Interrupt-matrix source id emitted while any enabled interrupt latches.
    intr_source_id: u32,
    /// CPU clock used to scale the APB prescaler to sim ticks.
    cpu_clock_hz: u32,

    clk_cfg: u32,
    timers: [TimerState; NUM_TIMERS],
    operators: [OperatorState; NUM_OPERATORS],

    int_raw: u32,
    int_ena: u32,

    /// Verbatim store for the remaining (round-trip-only) registers in the
    /// window — sync-input cfg, operator-timer-sel, fault-handler, carrier,
    /// capture, etc. Keeps the peripheral lossless without inventing behaviour.
    overflow: BTreeMap<u64, u32>,
}

impl Esp32s3Mcpwm {
    /// Construct one MCPWM unit bound to interrupt-matrix `base_source_id`
    /// (`MCPWM0_INTR_SOURCE` = 38 for MCPWM0, `MCPWM1_INTR_SOURCE` = 39 for
    /// MCPWM1). The CPU clock defaults to 240 MHz for the APB→sim-tick scaling;
    /// use `new_with_clock` to override.
    pub fn new(base_source_id: u32) -> Self {
        Self::new_with_clock(base_source_id, 240_000_000)
    }

    /// Construct with an explicit CPU clock (mirrors `Esp32s3TimerGroup::new`'s
    /// clock argument; the prescaler maths is sim-tick-relative).
    pub fn new_with_clock(base_source_id: u32, cpu_clock_hz: u32) -> Self {
        Self {
            intr_source_id: base_source_id,
            cpu_clock_hz,
            clk_cfg: 0,
            timers: [TimerState::new(); NUM_TIMERS],
            operators: [OperatorState::default(); NUM_OPERATORS],
            int_raw: 0,
            int_ena: 0,
            overflow: BTreeMap::new(),
        }
    }

    /// Sim (CPU) ticks per APB cycle (≥1). At 240/80 this is 3.
    fn cpu_per_apb(&self) -> u64 {
        (self.cpu_clock_hz as u64)
            .saturating_div(APB_CLOCK_HZ)
            .max(1)
    }

    /// MCPWM module clock prescale: PWM_clk = MCPWM_clk / (CLK_PRESCALE + 1).
    fn clk_prescale(&self) -> u64 {
        ((self.clk_cfg & CLK_PRESCALE_MASK) as u64) + 1
    }

    /// Live counter value of timer `t` (test/inspection helper).
    pub fn timer_value(&self, t: usize) -> u32 {
        self.timers.get(t).map(|x| x.counter).unwrap_or(0)
    }

    /// If `offset` falls in the timer block, return `(timer, reg)`.
    fn timer_at(offset: u64) -> Option<(usize, u64)> {
        if (TIMER_BLOCK_BASE..TIMER_BLOCK_END).contains(&offset) {
            let t = ((offset - TIMER_BLOCK_BASE) / TIMER_STRIDE) as usize;
            let reg = (offset - TIMER_BLOCK_BASE) % TIMER_STRIDE;
            Some((t, reg))
        } else {
            None
        }
    }

    /// If `offset` falls in the operator block, return `(operator, reg)`.
    fn operator_at(offset: u64) -> Option<(usize, u64)> {
        if (OP_BLOCK_BASE..OP_BLOCK_END).contains(&offset) {
            let op = ((offset - OP_BLOCK_BASE) / OP_STRIDE) as usize;
            let reg = (offset - OP_BLOCK_BASE) % OP_STRIDE;
            Some((op, reg))
        } else {
            None
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        if offset == REG_CLK_CFG {
            return self.clk_cfg;
        }
        if let Some((t, reg)) = Self::timer_at(offset) {
            return match reg {
                TIMER_CFG0 => self.timers[t].cfg0,
                TIMER_CFG1 => self.timers[t].cfg1,
                TIMER_SYNC => self.timers[t].sync,
                TIMER_STATUS => self.timers[t].status_word(),
                _ => 0,
            };
        }
        if let Some((op, reg)) = Self::operator_at(offset) {
            return match reg {
                OP_STMP_CFG => self.operators[op].stmp_cfg,
                OP_CMPR_VALUE0 => self.operators[op].cmpr_value0,
                OP_CMPR_VALUE1 => self.operators[op].cmpr_value1,
                OP_GEN_CFG0 => self.operators[op].gen_cfg0,
                OP_GEN_FORCE => self.operators[op].gen_force,
                OP_GEN_A => self.operators[op].gen_a,
                OP_GEN_B => self.operators[op].gen_b,
                OP_DT_CFG => self.operators[op].dt_cfg,
                OP_DT_FED => self.operators[op].dt_fed,
                _ => self.overflow.get(&offset).copied().unwrap_or(0),
            };
        }
        match offset {
            REG_INT_ENA => self.int_ena,
            REG_INT_RAW => self.int_raw,
            REG_INT_ST => self.int_raw & self.int_ena,
            REG_INT_CLR => 0, // write-only
            _ => self.overflow.get(&offset).copied().unwrap_or(0),
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if offset == REG_CLK_CFG {
            self.clk_cfg = value;
            return;
        }
        if let Some((t, reg)) = Self::timer_at(offset) {
            match reg {
                TIMER_CFG0 => self.timers[t].cfg0 = value,
                TIMER_CFG1 => {
                    let was_running = self.timers[t].running();
                    self.timers[t].cfg1 = value;
                    // On a fresh start, seed the direction from the count mode
                    // and reset the fractional accumulator so timing is
                    // deterministic from the start edge.
                    if !was_running && self.timers[t].running() {
                        self.timers[t].counting_down = self.timers[t].mode() == TIMER_MODE_DOWN;
                        self.timers[t].accum = 0;
                        if self.timers[t].mode() == TIMER_MODE_DOWN {
                            // Down mode begins from the period value.
                            self.timers[t].counter = self.timers[t].period();
                        }
                    }
                }
                TIMER_SYNC => self.timers[t].sync = value,
                TIMER_STATUS => {} // read-only
                _ => {}
            }
            return;
        }
        if let Some((op, reg)) = Self::operator_at(offset) {
            match reg {
                OP_STMP_CFG => self.operators[op].stmp_cfg = value,
                OP_CMPR_VALUE0 => self.operators[op].cmpr_value0 = value & CMPR_VALUE_MASK,
                OP_CMPR_VALUE1 => self.operators[op].cmpr_value1 = value & CMPR_VALUE_MASK,
                OP_GEN_CFG0 => self.operators[op].gen_cfg0 = value,
                OP_GEN_FORCE => self.operators[op].gen_force = value,
                OP_GEN_A => self.operators[op].gen_a = value,
                OP_GEN_B => self.operators[op].gen_b = value,
                OP_DT_CFG => self.operators[op].dt_cfg = value,
                OP_DT_FED => self.operators[op].dt_fed = value,
                _ => {
                    self.overflow.insert(offset, value);
                }
            }
            return;
        }
        match offset {
            REG_INT_ENA => self.int_ena = value & INT_MODELED_MASK,
            // INT_RAW is HW-set; firmware may force modeled bits (some drivers
            // do for self-test). Restrict to modeled bits.
            REG_INT_RAW => self.int_raw = value & INT_MODELED_MASK,
            REG_INT_ST => {}                       // read-only
            REG_INT_CLR => self.int_raw &= !value, // W1C
            _ => {
                self.overflow.insert(offset, value);
            }
        }
    }

    /// Advance one timer by one sim tick, latching TEZ/TEP and the matching
    /// operator's comparator-equal events into `int_raw`.
    fn advance_timer(
        timer: &mut TimerState,
        op: &OperatorState,
        t_index: usize,
        cpu_per_apb: u64,
        clk_prescale: u64,
        int_raw: &mut u32,
    ) {
        if !timer.running() {
            return;
        }
        let period = timer.period();
        if period == 0 {
            return; // degenerate config; nothing to count toward
        }

        // sim ticks per single counter step.
        let cpu_per_count = clk_prescale
            .saturating_mul(timer.timer_prescale())
            .saturating_mul(cpu_per_apb)
            .max(1);
        timer.accum += 1;
        if timer.accum < cpu_per_count {
            return;
        }
        let mut steps = timer.accum / cpu_per_count;
        timer.accum %= cpu_per_count;

        // Step one count at a time so every TEZ/TEP/compare crossing latches,
        // even when multiple counts elapse in a single tick.
        while steps > 0 {
            steps -= 1;
            match timer.mode() {
                TIMER_MODE_UP => {
                    if timer.counter >= period {
                        // At/over period: wrap to 0 → TEP then TEZ.
                        *int_raw |= int_tep_bit(t_index);
                        timer.counter = 0;
                        *int_raw |= int_tez_bit(t_index);
                    } else {
                        timer.counter += 1;
                        if timer.counter == period {
                            *int_raw |= int_tep_bit(t_index);
                        }
                    }
                    timer.counting_down = false;
                }
                TIMER_MODE_DOWN => {
                    if timer.counter == 0 {
                        *int_raw |= int_tez_bit(t_index);
                        timer.counter = period;
                        *int_raw |= int_tep_bit(t_index);
                    } else {
                        timer.counter -= 1;
                        if timer.counter == 0 {
                            *int_raw |= int_tez_bit(t_index);
                        }
                    }
                    timer.counting_down = true;
                }
                TIMER_MODE_UPDOWN => {
                    if !timer.counting_down {
                        if timer.counter >= period {
                            *int_raw |= int_tep_bit(t_index);
                            timer.counting_down = true;
                            if timer.counter > 0 {
                                timer.counter -= 1;
                            }
                        } else {
                            timer.counter += 1;
                            if timer.counter == period {
                                *int_raw |= int_tep_bit(t_index);
                                timer.counting_down = true;
                            }
                        }
                    } else if timer.counter == 0 {
                        *int_raw |= int_tez_bit(t_index);
                        timer.counting_down = false;
                        timer.counter += 1;
                    } else {
                        timer.counter -= 1;
                        if timer.counter == 0 {
                            *int_raw |= int_tez_bit(t_index);
                            timer.counting_down = false;
                        }
                    }
                }
                _ => {}
            }

            // Comparator-equal events for the operator sharing this index
            // (modeled 1:1 op↔timer wiring). Compare against the post-step
            // counter value.
            if timer.counter == (op.cmpr_value0 & CMPR_VALUE_MASK) {
                *int_raw |= int_tea_bit(t_index);
            }
            if timer.counter == (op.cmpr_value1 & CMPR_VALUE_MASK) {
                *int_raw |= int_teb_bit(t_index);
            }
        }
    }
}

impl Default for Esp32s3Mcpwm {
    fn default() -> Self {
        Self::new(MCPWM0_INTR_SOURCE)
    }
}

impl std::fmt::Debug for Esp32s3Mcpwm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Esp32s3Mcpwm")
            .field("source", &self.intr_source_id)
            .field("clk_cfg", &self.clk_cfg)
            .field("timer_values", &self.timers.map(|t| t.counter))
            .field("int_raw", &format_args!("{:#05x}", self.int_raw))
            .field("int_ena", &format_args!("{:#05x}", self.int_ena))
            .finish()
    }
}

impl Peripheral for Esp32s3Mcpwm {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        let byte_off = (offset & 3) * 8;
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

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset & !3, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let word = self.read_word(offset & !3);
        let byte_off = (offset & 3) * 8;
        Some(((word >> byte_off) & 0xFF) as u8)
    }

    /// One CPU cycle elapses per tick. Advance all running timers at their
    /// divided PWM-clock rate, latch TEZ/TEP + comparator events, and emit the
    /// unit's matrix source while any enabled interrupt is latched (level).
    fn tick(&mut self) -> PeripheralTickResult {
        let cpu_per_apb = self.cpu_per_apb();
        let clk_prescale = self.clk_prescale();
        let mut int_raw = self.int_raw;
        for t in 0..NUM_TIMERS {
            // Operator t shares timer t's index in our 1:1 wiring model.
            let op = self.operators[t];
            Self::advance_timer(
                &mut self.timers[t],
                &op,
                t,
                cpu_per_apb,
                clk_prescale,
                &mut int_raw,
            );
        }
        self.int_raw = int_raw;

        let asserted = (self.int_raw & self.int_ena & INT_MODELED_MASK) != 0;
        PeripheralTickResult {
            explicit_irqs: if asserted {
                Some(vec![self.intr_source_id])
            } else {
                None
            },
            ..Default::default()
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

    fn timer_off(t: usize, reg: u64) -> u64 {
        TIMER_BLOCK_BASE + (t as u64) * TIMER_STRIDE + reg
    }
    fn op_off(op: usize, reg: u64) -> u64 {
        OP_BLOCK_BASE + (op as u64) * OP_STRIDE + reg
    }

    /// Build a CFG0 word from prescale + period.
    fn cfg0(prescale: u32, period: u32) -> u32 {
        ((prescale & TIMER_CFG0_PRESCALE_MASK) << TIMER_CFG0_PRESCALE_SHIFT)
            | ((period & TIMER_CFG0_PERIOD_MASK) << TIMER_CFG0_PERIOD_SHIFT)
    }
    /// Build a CFG1 word from mode + start.
    fn cfg1(mode: u32, start: u32) -> u32 {
        ((mode & TIMER_CFG1_MODE_MASK) << TIMER_CFG1_MODE_SHIFT)
            | ((start & TIMER_CFG1_START_MASK) << TIMER_CFG1_START_SHIFT)
    }

    /// Run a CPU-80MHz unit so cpu_per_apb == 1 (simplest timing).
    fn new_unit() -> Esp32s3Mcpwm {
        Esp32s3Mcpwm::new_with_clock(MCPWM0_INTR_SOURCE, 80_000_000)
    }

    #[test]
    fn source_ids_are_38_and_39() {
        assert_eq!(MCPWM0_INTR_SOURCE, 38);
        assert_eq!(MCPWM1_INTR_SOURCE, 39);
        let u0 = Esp32s3Mcpwm::new(MCPWM0_INTR_SOURCE);
        let u1 = Esp32s3Mcpwm::new(MCPWM1_INTR_SOURCE);
        assert_eq!(u0.intr_source_id, 38);
        assert_eq!(u1.intr_source_id, 39);
    }

    #[test]
    fn reset_state_is_quiet() {
        let p = new_unit();
        assert_eq!(p.read_word(REG_CLK_CFG), 0);
        assert_eq!(p.read_word(REG_INT_RAW), 0);
        assert_eq!(p.read_word(REG_INT_ST), 0);
        for t in 0..NUM_TIMERS {
            assert_eq!(p.read_word(timer_off(t, TIMER_STATUS)), 0);
        }
    }

    #[test]
    fn clk_and_timer_cfg_round_trip() {
        let mut p = new_unit();
        p.write_u32(REG_CLK_CFG, 0x0000_0007).unwrap();
        assert_eq!(p.read_u32(REG_CLK_CFG).unwrap(), 0x0000_0007);

        // CFG0 / CFG1 round-trip verbatim for each timer, independently.
        p.write_u32(timer_off(1, TIMER_CFG0), cfg0(3, 1000))
            .unwrap();
        p.write_u32(timer_off(1, TIMER_CFG1), cfg1(TIMER_MODE_UP, 2))
            .unwrap();
        assert_eq!(p.read_u32(timer_off(1, TIMER_CFG0)).unwrap(), cfg0(3, 1000));
        assert_eq!(
            p.read_u32(timer_off(1, TIMER_CFG1)).unwrap(),
            cfg1(TIMER_MODE_UP, 2)
        );
        // Timer 0 untouched.
        assert_eq!(p.read_u32(timer_off(0, TIMER_CFG0)).unwrap(), 0);
        // SYNC round-trips.
        p.write_u32(timer_off(2, TIMER_SYNC), 0xABCD).unwrap();
        assert_eq!(p.read_u32(timer_off(2, TIMER_SYNC)).unwrap(), 0xABCD);
    }

    #[test]
    fn comparator_values_round_trip_masked_to_16_bits() {
        let mut p = new_unit();
        p.write_u32(op_off(0, OP_CMPR_VALUE0), 0xFFFF_1234).unwrap();
        p.write_u32(op_off(0, OP_CMPR_VALUE1), 0x0000_5678).unwrap();
        assert_eq!(p.read_u32(op_off(0, OP_CMPR_VALUE0)).unwrap(), 0x1234);
        assert_eq!(p.read_u32(op_off(0, OP_CMPR_VALUE1)).unwrap(), 0x5678);
        // Other operator words round-trip verbatim.
        p.write_u32(op_off(2, OP_GEN_A), 0xDEAD_BEEF).unwrap();
        p.write_u32(op_off(2, OP_DT_CFG), 0x0000_00FF).unwrap();
        assert_eq!(p.read_u32(op_off(2, OP_GEN_A)).unwrap(), 0xDEAD_BEEF);
        assert_eq!(p.read_u32(op_off(2, OP_DT_CFG)).unwrap(), 0x0000_00FF);
    }

    #[test]
    fn disabled_timer_does_not_advance() {
        let mut p = new_unit();
        // Period set but MODE frozen / START 0 → no counting.
        p.write_u32(timer_off(0, TIMER_CFG0), cfg0(0, 100)).unwrap();
        for _ in 0..500 {
            p.tick();
        }
        assert_eq!(
            p.read_word(timer_off(0, TIMER_STATUS)) & TIMER_STATUS_VALUE_MASK,
            0
        );
    }

    #[test]
    fn up_mode_counter_advances_at_prescale_rate() {
        let mut p = new_unit(); // cpu_per_apb == 1
                                // prescale 0 → div (0+1)*(clk0+1)=1 → 1 count/tick. period large.
        p.write_u32(timer_off(0, TIMER_CFG0), cfg0(0, 10_000))
            .unwrap();
        p.write_u32(timer_off(0, TIMER_CFG1), cfg1(TIMER_MODE_UP, 2))
            .unwrap();
        for _ in 0..5 {
            p.tick();
        }
        assert_eq!(p.timer_value(0), 5);
        // Direction is "up" (bit clear).
        assert_eq!(
            p.read_word(timer_off(0, TIMER_STATUS)) & TIMER_STATUS_DIRECTION_BIT,
            0
        );
    }

    #[test]
    fn timer_prescale_divides_count_rate() {
        let mut p = new_unit();
        // timer prescale 3 → (3+1)=4 ticks per count.
        p.write_u32(timer_off(0, TIMER_CFG0), cfg0(3, 10_000))
            .unwrap();
        p.write_u32(timer_off(0, TIMER_CFG1), cfg1(TIMER_MODE_UP, 2))
            .unwrap();
        for _ in 0..40 {
            p.tick();
        }
        assert_eq!(p.timer_value(0), 10, "40 ticks / 4 = 10 counts");
    }

    #[test]
    fn clk_prescale_divides_count_rate() {
        let mut p = new_unit();
        p.write_u32(REG_CLK_CFG, 1).unwrap(); // (1+1) = 2 ticks per count
        p.write_u32(timer_off(0, TIMER_CFG0), cfg0(0, 10_000))
            .unwrap();
        p.write_u32(timer_off(0, TIMER_CFG1), cfg1(TIMER_MODE_UP, 2))
            .unwrap();
        for _ in 0..20 {
            p.tick();
        }
        assert_eq!(p.timer_value(0), 10, "20 ticks / 2 = 10 counts");
    }

    #[test]
    fn up_mode_period_rollover_latches_tep_then_tez() {
        let mut p = new_unit();
        // Small period so we roll over quickly: period 3, 1 count/tick.
        p.write_u32(timer_off(0, TIMER_CFG0), cfg0(0, 3)).unwrap();
        p.write_u32(timer_off(0, TIMER_CFG1), cfg1(TIMER_MODE_UP, 2))
            .unwrap();
        // Tick to count 1,2,3 (period reached on the 3rd → TEP).
        p.tick();
        p.tick();
        p.tick();
        assert_eq!(p.timer_value(0), 3);
        assert_ne!(
            p.read_word(REG_INT_RAW) & int_tep_bit(0),
            0,
            "TEP latched at period"
        );
        // Next tick wraps to 0 → TEZ latches.
        p.tick();
        assert_eq!(p.timer_value(0), 0);
        assert_ne!(
            p.read_word(REG_INT_RAW) & int_tez_bit(0),
            0,
            "TEZ latched at zero"
        );
    }

    #[test]
    fn down_mode_starts_at_period_and_latches_tez() {
        let mut p = new_unit();
        p.write_u32(timer_off(1, TIMER_CFG0), cfg0(0, 3)).unwrap();
        p.write_u32(timer_off(1, TIMER_CFG1), cfg1(TIMER_MODE_DOWN, 2))
            .unwrap();
        // Down mode seeds counter to period at start.
        assert_eq!(p.timer_value(1), 3);
        assert_ne!(
            p.read_word(timer_off(1, TIMER_STATUS)) & TIMER_STATUS_DIRECTION_BIT,
            0,
            "direction = down"
        );
        // Count down 3,2,1,0 → TEZ at zero.
        for _ in 0..3 {
            p.tick();
        }
        assert_eq!(p.timer_value(1), 0);
        assert_ne!(p.read_word(REG_INT_RAW) & int_tez_bit(1), 0, "TEZ at zero");
    }

    #[test]
    fn comparator_equal_latches_tea() {
        let mut p = new_unit();
        // Operator 0 compare A = 2; timer 0 period 10, up mode.
        p.write_u32(op_off(0, OP_CMPR_VALUE0), 2).unwrap();
        p.write_u32(timer_off(0, TIMER_CFG0), cfg0(0, 10)).unwrap();
        p.write_u32(timer_off(0, TIMER_CFG1), cfg1(TIMER_MODE_UP, 2))
            .unwrap();
        // Before reaching 2: no TEA.
        p.tick(); // 1
        assert_eq!(p.read_word(REG_INT_RAW) & int_tea_bit(0), 0);
        p.tick(); // 2 → TEA
        assert_ne!(
            p.read_word(REG_INT_RAW) & int_tea_bit(0),
            0,
            "compare-A equal latches TEA"
        );
    }

    #[test]
    fn int_clr_is_write_one_to_clear() {
        let mut p = new_unit();
        p.write_u32(REG_INT_RAW, int_tez_bit(0) | int_tep_bit(1))
            .unwrap();
        assert_eq!(p.read_word(REG_INT_RAW), int_tez_bit(0) | int_tep_bit(1));
        // Clear only the TEZ bit.
        p.write_u32(REG_INT_CLR, int_tez_bit(0)).unwrap();
        assert_eq!(p.read_word(REG_INT_RAW), int_tep_bit(1));
        // INT_CLR reads as 0.
        assert_eq!(p.read_word(REG_INT_CLR), 0);
    }

    #[test]
    fn int_st_masks_raw_with_ena() {
        let mut p = new_unit();
        p.write_u32(REG_INT_RAW, int_tez_bit(0) | int_tep_bit(0))
            .unwrap();
        p.write_u32(REG_INT_ENA, int_tep_bit(0)).unwrap();
        assert_eq!(p.read_word(REG_INT_ST), int_tep_bit(0));
    }

    #[test]
    fn source_emitted_only_when_enabled_int_asserts() {
        let mut p = new_unit();
        // Latch an event that is NOT enabled — no source.
        p.write_u32(REG_INT_RAW, int_tez_bit(2)).unwrap();
        assert!(p.tick().explicit_irqs.is_none());
        // Enable it — source emitted while ST != 0 (level behaviour).
        p.write_u32(REG_INT_ENA, int_tez_bit(2)).unwrap();
        let r = p.tick();
        assert_eq!(r.explicit_irqs, Some(vec![MCPWM0_INTR_SOURCE]));
        // Clear the latch — source de-asserts.
        p.write_u32(REG_INT_CLR, int_tez_bit(2)).unwrap();
        assert!(p.tick().explicit_irqs.is_none());
    }

    #[test]
    fn running_timer_drives_enabled_irq_end_to_end() {
        let mut p = new_unit();
        // period 2, up mode, enable TEP for timer 0.
        p.write_u32(timer_off(0, TIMER_CFG0), cfg0(0, 2)).unwrap();
        p.write_u32(timer_off(0, TIMER_CFG1), cfg1(TIMER_MODE_UP, 2))
            .unwrap();
        p.write_u32(REG_INT_ENA, int_tep_bit(0)).unwrap();
        let mut fired = false;
        for _ in 0..3 {
            if p.tick().explicit_irqs.as_deref() == Some(&[MCPWM0_INTR_SOURCE][..]) {
                fired = true;
                break;
            }
        }
        assert!(
            fired,
            "TEP IRQ should reach the matrix source while enabled"
        );
    }

    #[test]
    fn unit1_uses_source_39() {
        let mut p = Esp32s3Mcpwm::new_with_clock(MCPWM1_INTR_SOURCE, 80_000_000);
        p.write_u32(REG_INT_RAW, int_tez_bit(0)).unwrap();
        p.write_u32(REG_INT_ENA, int_tez_bit(0)).unwrap();
        assert_eq!(p.tick().explicit_irqs, Some(vec![39]));
    }

    #[test]
    fn overflow_offsets_round_trip_and_are_lossless() {
        let mut p = new_unit();
        // An unmodeled offset within the window (e.g. OPERATOR_TIMERSEL 0x038).
        p.write_u32(0x038, 0x0000_0024).unwrap();
        assert_eq!(p.read_u32(0x038).unwrap(), 0x0000_0024);
        // Byte access composes too.
        p.write(0x038, 0xAB).unwrap();
        assert_eq!(p.read(0x038).unwrap(), 0xAB);
    }

    #[test]
    fn byte_access_composes_into_words() {
        let mut p = new_unit();
        p.write(timer_off(0, TIMER_CFG0), 0x34).unwrap();
        p.write(timer_off(0, TIMER_CFG0) + 1, 0x12).unwrap();
        assert_eq!(p.read_u32(timer_off(0, TIMER_CFG0)).unwrap(), 0x1234);
        assert_eq!(p.read(timer_off(0, TIMER_CFG0) + 1).unwrap(), 0x12);
    }
}
