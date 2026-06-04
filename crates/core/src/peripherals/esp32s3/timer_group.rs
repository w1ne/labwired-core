// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Timer Group peripheral (TIMGx) for ESP32-S3.
//!
//! One `Esp32s3TimerGroup` instance models a single timer group. The bus
//! registers two: TIMG0 @ 0x6001_F000 and TIMG1 @ 0x6002_0000. Each group
//! contains two general-purpose 64-bit up/down counters (T0, T1) plus the
//! Main Watchdog Timer (MWDT).
//!
//! Modelled after `systimer.rs`: a wide counter advanced by `tick()` at a
//! divided clock rate (accumulator pattern, like systimer's `cpu_cycle_accum`),
//! an UPDATE-latch that snapshots the live counter into the LO/HI read path,
//! and level-sensitive alarm IRQ delivery via `PeripheralTickResult.explicit_irqs`.
//!
//! ## Clocking
//!
//! Each timer is clocked from APB_CLK (80 MHz) through a 16-bit prescaler
//! (`Tx CONFIG` bits[28:13]). The simulator's `tick()` is one CPU cycle; at
//! the default 240 MHz CPU clock that is `CPU/APB = 3` sim ticks per APB
//! cycle. The counter therefore advances by 1 every `divider * (CPU/APB)`
//! sim ticks, tracked with a per-timer accumulator. Note the silicon quirk:
//! a divider field of **0 means 65536** (TRM §13.2.1).
//!
//! ## Register layout (ESP32-S3 TRM §13.4; verified against
//! `soc/esp32s3/register/soc/timer_group_reg.h`)
//!
//! Offsets are relative to the group base. The T1 block mirrors T0 at +0x24.
//!
//! | Offset | Name           | Behaviour |
//! |-------:|----------------|-----------|
//! | 0x00   | T0CONFIG       | bit31 EN, bit30 INCREASE(up/down), bit29 AUTORELOAD, bits[28:13] DIVIDER, bit10 ALARM_EN, bit9 USE_XTAL |
//! | 0x04   | T0LO           | RO: low 32 bits of the latched counter (valid after T0UPDATE) |
//! | 0x08   | T0HI           | RO: high 22 bits of the latched counter |
//! | 0x0C   | T0UPDATE       | write (any value) → snapshot live counter into T0LO/HI |
//! | 0x10   | T0ALARMLO      | alarm target, low 32 bits |
//! | 0x14   | T0ALARMHI      | alarm target, high 22 bits |
//! | 0x18   | T0LOADLO       | reload value, low 32 bits |
//! | 0x1C   | T0LOADHI       | reload value, high 22 bits |
//! | 0x20   | T0LOAD         | write (any value) → load T0LOADLO/HI into the live counter |
//! | 0x24   | T1CONFIG       | as T0CONFIG, for timer 1 |
//! | 0x28   | T1LO           | T1 latched counter low |
//! | 0x2C   | T1HI           | T1 latched counter high |
//! | 0x30   | T1UPDATE       | T1 snapshot trigger |
//! | 0x34   | T1ALARMLO      | T1 alarm low |
//! | 0x38   | T1ALARMHI      | T1 alarm high |
//! | 0x3C   | T1LOADLO       | T1 reload low |
//! | 0x40   | T1LOADHI       | T1 reload high |
//! | 0x44   | T1LOAD         | T1 reload trigger |
//! | 0x48   | WDTCONFIG0     | bit31 WDT_EN, stage cfg bits[30:23], reset lengths, flashboot |
//! | 0x4C   | WDTCONFIG1     | bits[31:16] MWDT clock prescaler |
//! | 0x50   | WDTCONFIG2     | stage0 timeout (MWDT clocks), default 26_000_000 |
//! | 0x54   | WDTCONFIG3     | stage1 timeout, default 0x07FF_FFFF |
//! | 0x58   | WDTCONFIG4     | stage2 timeout, default 0x000F_FFFF |
//! | 0x5C   | WDTCONFIG5     | stage3 timeout, default 0x000F_FFFF |
//! | 0x60   | WDTFEED        | write (any value) → reset the WDT counter |
//! | 0x64   | WDTWPROTECT    | write-key; default/reset key 0x50D8_3AA1 unlocks WDTCONFIG* |
//! | 0x70   | INT_ENA        | bit0 T0, bit1 T1, bit2 WDT — IRQ enable |
//! | 0x74   | INT_RAW        | bit0 T0, bit1 T1, bit2 WDT — raw pending (RO here) |
//! | 0x78   | INT_ST         | INT_RAW & INT_ENA (RO) |
//! | 0x7C   | INT_CLR        | write-1-to-clear the pending bits |
//!
//! ## Source IDs (ESP32-S3 TRM §9.4; verified against
//! `soc/esp32s3/include/soc/interrupts.h` `ETS_*_INTR_SOURCE` enum)
//!
//! The enum runs `... ETS_TIMER1=48, ETS_TIMER2=49, ETS_TG0_T0=50,
//! ETS_TG0_T1=51, ETS_TG0_WDT=52, ETS_TG1_T0=53, ETS_TG1_T1=54,
//! ETS_TG1_WDT=55`. So:
//!
//! * TIMG0_T0 → 50, TIMG0_T1 → 51, TIMG0_WDT → 52
//! * TIMG1_T0 → 53, TIMG1_T1 → 54, TIMG1_WDT → 55
//!
//! The task brief quoted 49..54; the authoritative header places the block at
//! 50..55 because two legacy `ETS_TIMER1/2` entries (48/49) precede it. We use
//! the header values. The constructor takes the group's base source id (the
//! T0 source); T1 and WDT follow at +1 / +2, exactly as `uart.rs` takes a
//! `source_id`.

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// APB clock that feeds the timer-group prescaler (80 MHz).
const APB_CLOCK_HZ: u64 = 80_000_000;

/// Source-id offsets within a group (relative to the T0 base source).
const SRC_T0_OFFSET: u32 = 0;
const SRC_T1_OFFSET: u32 = 1;
const SRC_WDT_OFFSET: u32 = 2;

// ── TxCONFIG bit layout ──
const CONFIG_EN_BIT: u32 = 1 << 31;
const CONFIG_INCREASE_BIT: u32 = 1 << 30;
const CONFIG_AUTORELOAD_BIT: u32 = 1 << 29;
const CONFIG_DIVIDER_SHIFT: u32 = 13;
const CONFIG_DIVIDER_MASK: u32 = 0xFFFF; // 16-bit field at bits[28:13]
const CONFIG_ALARM_EN_BIT: u32 = 1 << 10;
/// USE_XTAL clock-source select. Round-tripped via CONFIG but the sim always
/// clocks from APB; documented here for register fidelity.
#[allow(dead_code)]
const CONFIG_USE_XTAL_BIT: u32 = 1 << 9;

/// The 64-bit counter only carries 54 significant bits on silicon (HI is 22
/// bits). We keep the full u64 internally and mask the HI read to 22 bits.
const HI_MASK: u32 = 0x003F_FFFF;

// ── INT_* bit positions (shared T0/T1/WDT layout) ──
const INT_T0_BIT: u32 = 1 << 0;
const INT_T1_BIT: u32 = 1 << 1;
const INT_WDT_BIT: u32 = 1 << 2;

// ── WDTCONFIG0 bits ──
/// MWDT global enable. Round-tripped through WDTCONFIG0; the sim never acts on
/// a timeout (reset is a no-op), so this is documentation + test fixture only.
#[allow(dead_code)]
const WDT_EN_BIT: u32 = 1 << 31;

/// MWDT write-protect key (TIMG_WDT_WKEY reset value = 1356348065).
/// Writing this value to WDTWPROTECT unlocks the WDTCONFIG* registers; any
/// other value re-locks them.
const WDT_WKEY: u32 = 0x50D8_3AA1;

/// RTCCALICFG (0x68): RTC_CALI_START (bit31) starts a calibration, RTC_CALI_RDY
/// (bit15) signals completion. RTCCALICFG1 (0x6C) holds the measured value.
const RTC_CALI_START: u32 = 1 << 31;
const RTC_CALI_RDY: u32 = 1 << 15;

/// Reset defaults for the four WDT stage-timeout registers (TRM defaults).
const WDT_STG0_HOLD_DEFAULT: u32 = 26_000_000;
const WDT_STG1_HOLD_DEFAULT: u32 = 0x07FF_FFFF; // 134217727
const WDT_STG2_HOLD_DEFAULT: u32 = 0x000F_FFFF; // 1048575
const WDT_STG3_HOLD_DEFAULT: u32 = 0x000F_FFFF; // 1048575

/// State for one general-purpose 64-bit timer (T0 or T1).
#[derive(Debug, Clone, Copy)]
struct TimerState {
    /// Live running 64-bit counter.
    counter: u64,
    /// Latched snapshot, copied from `counter` on a TxUPDATE write. TxLO/HI
    /// reads return this (real silicon requires the UPDATE write first).
    snapshot: u64,
    /// Pending reload value composed from TxLOADLO/HI; copied into `counter`
    /// on a TxLOAD write.
    load: u64,
    /// 64-bit alarm target (TxALARMLO/HI).
    alarm: u64,
    /// TxCONFIG full register value (round-tripped; bitfields decoded on use).
    config: u32,
    /// Fractional clock accumulator — counts sim (CPU) ticks; when it reaches
    /// `divider * cpu_per_apb` the counter advances by 1. Mirrors systimer's
    /// `cpu_cycle_accum`.
    accum: u64,
    /// Sticky INT_RAW pending bit, set when the alarm fires. Cleared by
    /// INT_CLR (W1C).
    pending: bool,
    /// Edge latch: set on the counter→alarm crossing, cleared on autoreload /
    /// reload so the next crossing can re-fire. Distinct from `pending`, which
    /// INT_CLR clears without re-arming the edge.
    edge_latched: bool,
}

impl TimerState {
    /// Reset defaults from the header: CONFIG = INCREASE | AUTORELOAD |
    /// (DIVIDER=1), i.e. bit30 | bit29 | (1<<13). EN and ALARM_EN start clear.
    fn new() -> Self {
        Self {
            counter: 0,
            snapshot: 0,
            load: 0,
            alarm: 0,
            config: CONFIG_INCREASE_BIT | CONFIG_AUTORELOAD_BIT | (1 << CONFIG_DIVIDER_SHIFT),
            accum: 0,
            pending: false,
            edge_latched: false,
        }
    }

    fn enabled(&self) -> bool {
        self.config & CONFIG_EN_BIT != 0
    }

    fn increasing(&self) -> bool {
        self.config & CONFIG_INCREASE_BIT != 0
    }

    fn autoreload(&self) -> bool {
        self.config & CONFIG_AUTORELOAD_BIT != 0
    }

    fn alarm_en(&self) -> bool {
        self.config & CONFIG_ALARM_EN_BIT != 0
    }

    /// Effective prescaler. The 16-bit field encodes 1..65535 directly, with
    /// the value **0 meaning 65536** (TRM §13.2.1).
    fn divider(&self) -> u64 {
        let d = (self.config >> CONFIG_DIVIDER_SHIFT) & CONFIG_DIVIDER_MASK;
        if d == 0 {
            65536
        } else {
            d as u64
        }
    }
}

#[derive(Debug)]
pub struct Esp32s3TimerGroup {
    t0: TimerState,
    t1: TimerState,
    cpu_clock_hz: u32,

    // ── MWDT registers (round-trip; reset action is a deliberate no-op) ──
    wdt_config0: u32,
    wdt_config1: u32,
    wdt_config2: u32,
    wdt_config3: u32,
    wdt_config4: u32,
    wdt_config5: u32,
    /// Live MWDT down/up counter; reset to 0 on WDTFEED. We never act on a
    /// timeout (see `tick()`), so this only ever provides faithful register
    /// semantics, not an actual chip reset.
    wdt_counter: u64,
    /// Last value written to WDTWPROTECT. WDTCONFIG* writes are gated unless
    /// this equals `WDT_WKEY`.
    wdt_wprotect: u32,

    /// INT_ENA (0x70): bits 0/1/2 enable IRQ delivery for T0/T1/WDT.
    int_ena: u32,
    /// Sticky WDT INT_RAW bit (W1C via INT_CLR). The GP-timer pending bits
    /// live in `TimerState::pending`.
    wdt_pending: bool,

    /// RTC clock-calibration registers (RTCCALICFG @0x68 / RTCCALICFG1 @0x6C).
    /// The bootloader calibrates RTC_SLOW_CLK against the APB clock here and
    /// busy-polls RTC_CALI_RDY — without auto-completion it spins forever.
    rtccalicfg: u32,
    rtccalicfg1: u32,

    /// Interrupt-matrix source id for T0. T1 = +1, WDT = +2.
    base_source_id: u32,
}

impl Esp32s3TimerGroup {
    /// Construct one timer group. `base_source_id` is the group's T0
    /// interrupt-matrix source (50 for TIMG0, 53 for TIMG1); T1 and WDT follow
    /// at +1 and +2. `cpu_clock_hz` scales the APB prescaler to sim ticks
    /// (mirrors `Systimer::new`).
    pub fn new(base_source_id: u32, cpu_clock_hz: u32) -> Self {
        Self {
            t0: TimerState::new(),
            t1: TimerState::new(),
            cpu_clock_hz,
            // WDTCONFIG0 reset: FLASHBOOT_MOD_EN (bit14) + reset-length
            // defaults (bits 15 and 18 set per header default 1). WDT_EN clear.
            wdt_config0: (1 << 14) | (1 << 15) | (1 << 18),
            wdt_config1: 1 << 16, // CLK_PRESCALE default 1
            wdt_config2: WDT_STG0_HOLD_DEFAULT,
            wdt_config3: WDT_STG1_HOLD_DEFAULT,
            wdt_config4: WDT_STG2_HOLD_DEFAULT,
            wdt_config5: WDT_STG3_HOLD_DEFAULT,
            wdt_counter: 0,
            // Reset value of WDTWPROTECT *is* the key, i.e. unlocked at reset.
            wdt_wprotect: WDT_WKEY,
            int_ena: 0,
            wdt_pending: false,
            rtccalicfg: 0,
            rtccalicfg1: 0,
            base_source_id,
        }
    }

    /// RTC calibration completion (carried over from the classic `Timg` /
    /// `TimgStub`): when firmware sets RTCCALICFG.START (bit31) we immediately
    /// latch RTC_CALI_RDY (bit15) and stash a derived cycle count in
    /// RTCCALICFG1, so `rtc_clk_cal`'s poll completes in one read. (A
    /// round-trip-only model leaves RDY clear and the bootloader hangs.)
    fn maybe_complete_rtc_calibration(&mut self) {
        if self.rtccalicfg & RTC_CALI_START != 0 {
            self.rtccalicfg |= RTC_CALI_RDY;
            // max = RTCCALICFG.RTC_CALI_MAX (bits[31:13]); ~533 APB cycles per
            // RTC_SLOW_CLK period at 80 MHz; RTCCALICFG1.VALUE is bits[31:7].
            let max = ((self.rtccalicfg >> 13) & 0x1FFFF).max(1);
            let value = max.wrapping_mul(533) & 0x01FF_FFFF;
            self.rtccalicfg1 = (value << 7) | 1;
        }
    }

    /// True when WDTWPROTECT currently holds the unlock key.
    fn wdt_unlocked(&self) -> bool {
        self.wdt_wprotect == WDT_WKEY
    }

    /// Number of sim (CPU) ticks per APB cycle. At 240 MHz CPU / 80 MHz APB
    /// this is 3; at 80 MHz it is 1. Floored to at least 1.
    fn cpu_per_apb(&self) -> u64 {
        (self.cpu_clock_hz as u64)
            .saturating_div(APB_CLOCK_HZ)
            .max(1)
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            // ── Timer 0 ──
            0x00 => self.t0.config,
            0x04 => (self.t0.snapshot & 0xFFFF_FFFF) as u32,
            0x08 => ((self.t0.snapshot >> 32) as u32) & HI_MASK,
            // 0x0C T0UPDATE is write-only (R/W/SC); reads as 0.
            0x10 => (self.t0.alarm & 0xFFFF_FFFF) as u32,
            0x14 => ((self.t0.alarm >> 32) as u32) & HI_MASK,
            0x18 => (self.t0.load & 0xFFFF_FFFF) as u32,
            0x1C => ((self.t0.load >> 32) as u32) & HI_MASK,
            // 0x20 T0LOAD is write-only.

            // ── Timer 1 (T0 block + 0x24) ──
            0x24 => self.t1.config,
            0x28 => (self.t1.snapshot & 0xFFFF_FFFF) as u32,
            0x2C => ((self.t1.snapshot >> 32) as u32) & HI_MASK,
            // 0x30 T1UPDATE write-only.
            0x34 => (self.t1.alarm & 0xFFFF_FFFF) as u32,
            0x38 => ((self.t1.alarm >> 32) as u32) & HI_MASK,
            0x3C => (self.t1.load & 0xFFFF_FFFF) as u32,
            0x40 => ((self.t1.load >> 32) as u32) & HI_MASK,
            // 0x44 T1LOAD write-only.

            // ── MWDT ──
            0x48 => self.wdt_config0,
            0x4C => self.wdt_config1,
            0x50 => self.wdt_config2,
            0x54 => self.wdt_config3,
            0x58 => self.wdt_config4,
            0x5C => self.wdt_config5,
            // 0x60 WDTFEED write-only.
            0x64 => self.wdt_wprotect,

            // ── RTC calibration ──
            0x68 => self.rtccalicfg,
            0x6C => self.rtccalicfg1,

            // ── Interrupts ──
            0x70 => self.int_ena,
            0x74 => self.int_raw_word(),
            0x78 => self.int_raw_word() & self.int_ena,
            // 0x7C INT_CLR is W1C; reads as 0.
            _ => 0,
        }
    }

    /// INT_RAW (0x74): bit0 = T0 pending, bit1 = T1 pending, bit2 = WDT.
    fn int_raw_word(&self) -> u32 {
        let mut v = 0u32;
        if self.t0.pending {
            v |= INT_T0_BIT;
        }
        if self.t1.pending {
            v |= INT_T1_BIT;
        }
        if self.wdt_pending {
            v |= INT_WDT_BIT;
        }
        v
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            // ── Timer 0 ──
            0x00 => self.t0.config = value,
            // T0LO/HI are RO snapshot outputs; ignore writes.
            0x0C => self.t0.snapshot = self.t0.counter, // UPDATE: latch live counter
            0x10 => set_lo(&mut self.t0.alarm, value),
            0x14 => set_hi(&mut self.t0.alarm, value),
            0x18 => set_lo(&mut self.t0.load, value),
            0x1C => set_hi(&mut self.t0.load, value),
            0x20 => {
                // LOAD: copy reload value into the live counter and re-arm.
                self.t0.counter = self.t0.load;
                self.t0.edge_latched = false;
            }

            // ── Timer 1 ──
            0x24 => self.t1.config = value,
            0x30 => self.t1.snapshot = self.t1.counter,
            0x34 => set_lo(&mut self.t1.alarm, value),
            0x38 => set_hi(&mut self.t1.alarm, value),
            0x3C => set_lo(&mut self.t1.load, value),
            0x40 => set_hi(&mut self.t1.load, value),
            0x44 => {
                self.t1.counter = self.t1.load;
                self.t1.edge_latched = false;
            }

            // ── MWDT config: gated by the write-protect key ──
            0x48 if self.wdt_unlocked() => self.wdt_config0 = value,
            0x4C if self.wdt_unlocked() => self.wdt_config1 = value,
            0x50 if self.wdt_unlocked() => self.wdt_config2 = value,
            0x54 if self.wdt_unlocked() => self.wdt_config3 = value,
            0x58 if self.wdt_unlocked() => self.wdt_config4 = value,
            0x5C if self.wdt_unlocked() => self.wdt_config5 = value,
            0x60 => {
                // WDTFEED: any write resets the WDT counter. We deliberately do
                // NOT model a chip reset on timeout — triggering one would abort
                // long sim runs. The counter is reset here so feed semantics are
                // observable, but `tick()` never acts on a timeout (no-op).
                self.wdt_counter = 0;
            }
            0x64 => self.wdt_wprotect = value, // WDTWPROTECT is always writable

            // ── RTC calibration: START latches RDY + the measured value ──
            0x68 => {
                self.rtccalicfg = value;
                self.maybe_complete_rtc_calibration();
            }
            0x6C => self.rtccalicfg1 = value,

            // ── Interrupts ──
            0x70 => self.int_ena = value & 0x7,
            // 0x74 INT_RAW: not firmware-writable in our model; ignore.
            0x7C => {
                // INT_CLR: write-1-to-clear the sticky pending bits.
                if value & INT_T0_BIT != 0 {
                    self.t0.pending = false;
                }
                if value & INT_T1_BIT != 0 {
                    self.t1.pending = false;
                }
                if value & INT_WDT_BIT != 0 {
                    self.wdt_pending = false;
                }
            }
            // Unhandled / read-only offsets: ignore (matches systimer).
            _ => {}
        }
    }

    /// Advance one timer by however many APB cycles have accumulated this
    /// tick, handling alarm crossing + autoreload. Returns true if the alarm
    /// fired (pending edge) this advance.
    fn advance_timer(timer: &mut TimerState, cpu_ticks: u64, cpu_per_apb: u64) {
        if !timer.enabled() {
            return;
        }
        timer.accum += cpu_ticks;
        let cpu_per_count = timer.divider().saturating_mul(cpu_per_apb).max(1);
        if timer.accum < cpu_per_count {
            return;
        }
        let counts = timer.accum / cpu_per_count;
        timer.accum %= cpu_per_count;

        if timer.increasing() {
            timer.counter = timer.counter.wrapping_add(counts);
        } else {
            timer.counter = timer.counter.wrapping_sub(counts);
        }

        // Alarm crossing. For an up-counter the alarm fires when the counter
        // reaches/passes the target; for a down-counter when it falls to/below.
        if timer.alarm_en() && !timer.edge_latched {
            let crossed = if timer.increasing() {
                timer.counter >= timer.alarm
            } else {
                timer.counter <= timer.alarm
            };
            if crossed {
                timer.edge_latched = true;
                timer.pending = true;
                if timer.autoreload() {
                    // Reload the counter and re-arm so the next period fires.
                    timer.counter = timer.load;
                    timer.edge_latched = false;
                }
            }
        }
    }
}

/// Compose the low 32 bits of a 64-bit register field.
fn set_lo(field: &mut u64, value: u32) {
    *field = (*field & 0xFFFF_FFFF_0000_0000) | (value as u64);
}

/// Compose the high 22 bits of a 64-bit register field (HI is masked to 22b).
fn set_hi(field: &mut u64, value: u32) {
    let hi = (value & HI_MASK) as u64;
    *field = (*field & 0x0000_0000_FFFF_FFFF) | (hi << 32);
}

impl Peripheral for Esp32s3TimerGroup {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        // For write-trigger registers (UPDATE/LOAD/FEED) we want any byte
        // write to fire; reconstruct the word from current state + the byte.
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    /// One CPU cycle elapses per `tick`. Advance both enabled timers at their
    /// divided APB rate and emit level-sensitive alarm IRQs.
    fn tick(&mut self) -> PeripheralTickResult {
        let cpu_per_apb = self.cpu_per_apb();

        Self::advance_timer(&mut self.t0, 1, cpu_per_apb);
        Self::advance_timer(&mut self.t1, 1, cpu_per_apb);

        // MWDT: we model the counter but never act on a timeout. Advancing it
        // is harmless and keeps register reads plausible, but a real chip would
        // assert a reset/IRQ here — deliberately omitted so long sim runs don't
        // abort. (See WDTFEED handler.) Left as a no-op on purpose.

        // Level-sensitive IRQ delivery: while a pending bit is set AND its
        // INT_ENA bit is set, re-emit the matrix source every tick. Same
        // rationale as systimer — keeps the bus aggregator bit asserted until
        // firmware ACKs via INT_CLR.
        let mut explicit_irqs = Vec::new();
        if self.t0.pending && (self.int_ena & INT_T0_BIT != 0) {
            explicit_irqs.push(self.base_source_id + SRC_T0_OFFSET);
        }
        if self.t1.pending && (self.int_ena & INT_T1_BIT != 0) {
            explicit_irqs.push(self.base_source_id + SRC_T1_OFFSET);
        }
        if self.wdt_pending && (self.int_ena & INT_WDT_BIT != 0) {
            explicit_irqs.push(self.base_source_id + SRC_WDT_OFFSET);
        }

        PeripheralTickResult {
            explicit_irqs: if explicit_irqs.is_empty() {
                None
            } else {
                Some(explicit_irqs)
            },
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

    // TIMG0 base source = 50 (per soc/interrupts.h). Used throughout.
    const TIMG0_T0_SOURCE: u32 = 50;

    fn new_timg0() -> Esp32s3TimerGroup {
        Esp32s3TimerGroup::new(TIMG0_T0_SOURCE, 240_000_000)
    }

    /// Helper: enable T0 with a given divider (up-counter, no alarm).
    fn enable_t0(tg: &mut Esp32s3TimerGroup, divider: u32) {
        let cfg =
            CONFIG_EN_BIT | CONFIG_INCREASE_BIT | ((divider & 0xFFFF) << CONFIG_DIVIDER_SHIFT);
        tg.write_word(0x00, cfg);
    }

    #[test]
    fn defaults_seed_config_and_wdt() {
        let tg = new_timg0();
        // INCREASE | AUTORELOAD | DIVIDER=1, EN clear.
        assert_eq!(tg.t0.config & CONFIG_EN_BIT, 0);
        assert!(tg.t0.increasing());
        assert!(tg.t0.autoreload());
        assert_eq!(tg.t0.divider(), 1);
        // WDT unlocked at reset; stage0 hold default.
        assert!(tg.wdt_unlocked());
        assert_eq!(tg.read_word(0x50), WDT_STG0_HOLD_DEFAULT);
    }

    #[test]
    fn counter_advances_at_divider_rate() {
        let mut tg = new_timg0();
        // 240 MHz CPU / 80 MHz APB = 3 sim ticks per APB cycle.
        // Divider 1 → 1 count per 3 ticks.
        enable_t0(&mut tg, 1);
        for _ in 0..30 {
            tg.tick();
        }
        // Snapshot to read.
        tg.write_word(0x0C, 0);
        let lo = tg.read_word(0x04);
        assert_eq!(lo, 10, "30 sim ticks / (div1 * 3) = 10 counts");
    }

    #[test]
    fn counter_respects_divider_two() {
        let mut tg = new_timg0();
        // Divider 2 → 1 count per (2*3)=6 sim ticks.
        enable_t0(&mut tg, 2);
        for _ in 0..60 {
            tg.tick();
        }
        tg.write_word(0x0C, 0);
        assert_eq!(tg.read_word(0x04), 10, "60 / (2*3) = 10 counts");
    }

    #[test]
    fn divider_zero_means_65536() {
        let mut tg = new_timg0();
        tg.write_word(0x00, CONFIG_EN_BIT | CONFIG_INCREASE_BIT); // DIVIDER field = 0
        assert_eq!(tg.t0.divider(), 65536);
    }

    #[test]
    fn disabled_timer_does_not_advance() {
        let mut tg = new_timg0();
        // EN clear (default). Tick a lot.
        for _ in 0..300 {
            tg.tick();
        }
        tg.write_word(0x0C, 0);
        assert_eq!(tg.read_word(0x04), 0);
    }

    #[test]
    fn update_latch_then_lo_hi_read_returns_snapshot() {
        let mut tg = new_timg0();
        enable_t0(&mut tg, 1);
        for _ in 0..30 {
            tg.tick();
        }
        // Before UPDATE, LO/HI read the (stale) snapshot = 0.
        assert_eq!(tg.read_word(0x04), 0, "no UPDATE yet → snapshot is 0");
        // UPDATE latches the live counter (10).
        tg.write_word(0x0C, 0);
        assert_eq!(tg.read_word(0x04), 10);
        assert_eq!(tg.read_word(0x08), 0, "high word still 0 for small counts");
        // Advancing further does not change the latch until the next UPDATE.
        for _ in 0..30 {
            tg.tick();
        }
        assert_eq!(tg.read_word(0x04), 10, "latch frozen until next UPDATE");
        tg.write_word(0x0C, 0);
        assert_eq!(tg.read_word(0x04), 20);
    }

    #[test]
    fn load_sets_the_counter() {
        let mut tg = new_timg0();
        // LOADHI = 1, LOADLO = 0x42 → counter = (1<<32)|0x42 on LOAD.
        tg.write_word(0x18, 0x0000_0042); // T0LOADLO
        tg.write_word(0x1C, 0x0000_0001); // T0LOADHI
        tg.write_word(0x20, 1); // T0LOAD trigger
        tg.write_word(0x0C, 0); // UPDATE to read
        assert_eq!(tg.read_word(0x04), 0x42);
        assert_eq!(tg.read_word(0x08), 1);
    }

    #[test]
    fn config_round_trips() {
        let mut tg = new_timg0();
        let cfg = CONFIG_EN_BIT
            | CONFIG_INCREASE_BIT
            | CONFIG_AUTORELOAD_BIT
            | CONFIG_ALARM_EN_BIT
            | CONFIG_USE_XTAL_BIT
            | (7 << CONFIG_DIVIDER_SHIFT);
        tg.write_word(0x00, cfg);
        assert_eq!(tg.read_word(0x00), cfg);
        assert!(tg.t0.enabled());
        assert!(tg.t0.alarm_en());
        assert_eq!(tg.t0.divider(), 7);
    }

    #[test]
    fn alarm_fires_and_emits_source() {
        let mut tg = new_timg0();
        // Alarm at count 5, up-counter, divider 1, alarm_en, NO autoreload.
        tg.write_word(0x10, 5); // T0ALARMLO = 5
        tg.write_word(0x14, 0); // T0ALARMHI = 0
        let cfg =
            CONFIG_EN_BIT | CONFIG_INCREASE_BIT | CONFIG_ALARM_EN_BIT | (1 << CONFIG_DIVIDER_SHIFT);
        tg.write_word(0x00, cfg);
        tg.write_word(0x70, INT_T0_BIT); // INT_ENA T0

        // count 5 needs 5 * (div1 * 3) = 15 sim ticks.
        for _ in 0..14 {
            let r = tg.tick();
            assert!(
                r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()),
                "no fire before reaching alarm"
            );
        }
        let r = tg.tick();
        assert_eq!(
            r.explicit_irqs.as_deref(),
            Some(&[TIMG0_T0_SOURCE][..]),
            "TIMG0_T0 source emitted at alarm"
        );
        assert!(tg.t0.pending);
        assert_eq!(tg.read_word(0x74) & INT_T0_BIT, INT_T0_BIT, "INT_RAW set");
        assert_eq!(tg.read_word(0x78) & INT_T0_BIT, INT_T0_BIT, "INT_ST set");
    }

    #[test]
    fn alarm_t1_emits_base_plus_one() {
        let mut tg = new_timg0();
        // Configure T1 (block at +0x24).
        tg.write_word(0x34, 3); // T1ALARMLO = 3
        let cfg =
            CONFIG_EN_BIT | CONFIG_INCREASE_BIT | CONFIG_ALARM_EN_BIT | (1 << CONFIG_DIVIDER_SHIFT);
        tg.write_word(0x24, cfg);
        tg.write_word(0x70, INT_T1_BIT);
        let mut fired = None;
        for cycle in 0..100 {
            let r = tg.tick();
            if r.explicit_irqs.as_ref().is_some_and(|v| !v.is_empty()) {
                fired = Some((cycle + 1, r.explicit_irqs.unwrap()));
                break;
            }
        }
        let (_, srcs) = fired.expect("T1 alarm should fire");
        assert_eq!(srcs, vec![TIMG0_T0_SOURCE + 1], "TIMG0_T1 = base + 1");
    }

    #[test]
    fn autoreload_reschedules_alarm() {
        let mut tg = new_timg0();
        // LOAD value 0, alarm at 4, autoreload on, divider 1.
        tg.write_word(0x10, 4); // alarm = 4
        let cfg = CONFIG_EN_BIT
            | CONFIG_INCREASE_BIT
            | CONFIG_AUTORELOAD_BIT
            | CONFIG_ALARM_EN_BIT
            | (1 << CONFIG_DIVIDER_SHIFT);
        tg.write_word(0x00, cfg);
        tg.write_word(0x70, INT_T0_BIT);

        // First fire at count 4 → 12 sim ticks.
        let mut first = None;
        for cycle in 0..50 {
            if tg.tick().explicit_irqs.is_some() {
                first = Some(cycle + 1);
                break;
            }
        }
        assert_eq!(first, Some(12));
        // On autoreload the counter was reset to load(0). Clear pending then
        // confirm it re-fires another ~12 ticks later.
        tg.write_word(0x7C, INT_T0_BIT); // INT_CLR
        assert!(!tg.t0.pending);
        let mut second = None;
        for cycle in 0..50 {
            if tg.tick().explicit_irqs.is_some() {
                second = Some(cycle + 1);
                break;
            }
        }
        assert_eq!(
            second,
            Some(12),
            "autoreloaded alarm re-fires one period later"
        );
    }

    #[test]
    fn int_clr_is_w1c() {
        let mut tg = new_timg0();
        tg.write_word(0x10, 2); // alarm = 2
        let cfg =
            CONFIG_EN_BIT | CONFIG_INCREASE_BIT | CONFIG_ALARM_EN_BIT | (1 << CONFIG_DIVIDER_SHIFT);
        tg.write_word(0x00, cfg);
        tg.write_word(0x70, INT_T0_BIT);
        for _ in 0..30 {
            tg.tick();
        }
        assert!(tg.t0.pending);
        // Writing a 0 to the T0 bit must NOT clear it (W1C).
        tg.write_word(0x7C, 0);
        assert!(tg.t0.pending, "writing 0 does not clear");
        // Writing 1 clears.
        tg.write_word(0x7C, INT_T0_BIT);
        assert!(!tg.t0.pending);
        assert_eq!(tg.read_word(0x74) & INT_T0_BIT, 0, "INT_RAW cleared");
    }

    #[test]
    fn int_ena_gates_irq_emission_but_not_pending() {
        let mut tg = new_timg0();
        tg.write_word(0x10, 2);
        let cfg =
            CONFIG_EN_BIT | CONFIG_INCREASE_BIT | CONFIG_ALARM_EN_BIT | (1 << CONFIG_DIVIDER_SHIFT);
        tg.write_word(0x00, cfg);
        // INT_ENA left 0 → no IRQ emitted, but pending still sets.
        for _ in 0..30 {
            let r = tg.tick();
            assert!(r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()));
        }
        assert!(tg.t0.pending, "pending sets regardless of INT_ENA");
        assert_eq!(tg.read_word(0x78), 0, "INT_ST masked by INT_ENA=0");
    }

    #[test]
    fn rtc_cali_start_latches_rdy_and_value() {
        // The bootloader's rtc_clk_cal busy-polls RTC_CALI_RDY after START;
        // without auto-completion the boot hangs (regression that this models).
        let mut tg = new_timg0();
        assert_eq!(
            tg.read_word(0x68) & RTC_CALI_RDY,
            0,
            "RDY clear before START"
        );
        // Start a calibration with a max-cycle count in bits[31:13].
        let max = 1024u32;
        tg.write_word(0x68, RTC_CALI_START | (max << 13));
        assert_eq!(
            tg.read_word(0x68) & RTC_CALI_RDY,
            RTC_CALI_RDY,
            "RDY latched on START"
        );
        assert_ne!(tg.read_word(0x6C), 0, "RTCCALICFG1 holds a measured value");
        assert_eq!(tg.read_word(0x6C) & 1, 1, "value valid bit set");
    }

    #[test]
    fn wdt_feed_resets_counter() {
        let mut tg = new_timg0();
        tg.wdt_counter = 12345; // pretend it ran up
        tg.write_word(0x60, 0xDEAD_BEEF); // WDTFEED (any value)
        assert_eq!(tg.wdt_counter, 0, "feed resets WDT counter");
    }

    #[test]
    fn wdt_wprotect_gates_config_writes() {
        let mut tg = new_timg0();
        // Lock by writing a non-key value.
        tg.write_word(0x64, 0x0000_0000);
        assert!(!tg.wdt_unlocked());
        let before = tg.read_word(0x48);
        tg.write_word(0x48, 0xFFFF_FFFF); // should be ignored while locked
        assert_eq!(
            tg.read_word(0x48),
            before,
            "locked: WDTCONFIG0 write ignored"
        );
        // Unlock with the key, then the write lands.
        tg.write_word(0x64, WDT_WKEY);
        assert!(tg.wdt_unlocked());
        tg.write_word(0x48, WDT_EN_BIT);
        assert_eq!(tg.read_word(0x48), WDT_EN_BIT, "unlocked: write applied");
    }

    #[test]
    fn down_counter_alarm_fires_on_underflow_target() {
        let mut tg = new_timg0();
        // Load 10, count down, alarm at 5, divider 1.
        tg.write_word(0x18, 10); // LOADLO = 10
        tg.write_word(0x20, 1); // LOAD → counter = 10
        tg.write_word(0x10, 5); // alarm = 5
                                // EN | (INCREASE cleared = down) | ALARM_EN | div1.
        let cfg = CONFIG_EN_BIT | CONFIG_ALARM_EN_BIT | (1 << CONFIG_DIVIDER_SHIFT);
        tg.write_word(0x00, cfg);
        tg.write_word(0x70, INT_T0_BIT);
        assert!(!tg.t0.increasing());
        // Count 10 → 5 is 5 decrements = 15 sim ticks.
        let mut fired = None;
        for cycle in 0..60 {
            if tg.tick().explicit_irqs.is_some() {
                fired = Some(cycle + 1);
                break;
            }
        }
        assert_eq!(fired, Some(15), "down-counter alarm fires at target");
    }
}
