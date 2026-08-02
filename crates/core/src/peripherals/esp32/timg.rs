// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-classic Timer Group (TIMG0 / TIMG1) peripheral.
//!
//! Reference: ESP32 TRM v5.0 §16 (Timer Group). Each timer group exposes
//! two 64-bit general-purpose timers (T0/T1) and one Main System Watchdog
//! Timer (MWDT), all sharing one ~0xA4-byte register window.
//!
//! This model is **functional, not cycle-accurate**. Its job is to keep
//! ESP-IDF init code happy:
//!   * Register reads/writes are acknowledged at the correct offsets so
//!     state probes don't fault.
//!   * The 64-bit T0/T1 counters tick monotonically at a deterministic
//!     1 µs cadence (1 increment per `tick()`, assuming 240 MHz CPU and
//!     240 cycles per µs upstream — the bus drives `tick()` once per
//!     simulated µs, see `Bus::tick_peripherals_with_costs`).
//!   * `T0_UPDATE` / `T1_UPDATE` latches the live counter into LO/HI so
//!     subsequent register reads return a consistent 64-bit snapshot —
//!     real silicon also requires this strobe before reading LO/HI.
//!   * Watchdog feeds (any write to `WDT_FEED_REG`) are silently
//!     accepted; we don't model WDT-induced resets.
//!   * `RTCCALICFG.START` (bit 31) latches `RDY` (bit 15) immediately,
//!     preserving the calibration-loop unblock semantics from the
//!     pre-existing `TimgStub`. Without it, esp-idf's
//!     `rtc_clk_wait_for_slow_cycle` spins forever.
//!   * INT_ENA / INT_RAW / INT_ST / INT_CLR plumbing is byte-addressable
//!     and round-trips — firmware can configure timer interrupt masks
//!     even though we don't actually fire IRQs from this peripheral yet
//!     (deferred to the interrupt-routing follow-up).
//!
//! ## Register map (per ESP32 TRM §16, both TIMG0 and TIMG1)
//!
//! | Offset | Name             | Semantics modeled                   |
//! |-------:|------------------|-------------------------------------|
//! | 0x00   | T0CONFIG         | Round-trip; bit 31 = T0_EN          |
//! | 0x04   | T0LO             | Read: latched low 32 bits of t0     |
//! | 0x08   | T0HI             | Read: latched high 32 bits of t0    |
//! | 0x0C   | T0UPDATE         | Write triggers counter latch        |
//! | 0x10   | T0ALARMLO        | Round-trip                          |
//! | 0x14   | T0ALARMHI        | Round-trip                          |
//! | 0x18   | T0LOADLO         | Round-trip                          |
//! | 0x1C   | T0LOADHI         | Round-trip                          |
//! | 0x20   | T0LOAD           | Write: preload counter from LOAD*   |
//! | 0x24   | T1CONFIG         | Same layout as T0, offset by 0x24   |
//! | 0x28   | T1LO             |                                     |
//! | 0x2C   | T1HI             |                                     |
//! | 0x30   | T1UPDATE         |                                     |
//! | 0x34   | T1ALARMLO        |                                     |
//! | 0x38   | T1ALARMHI        |                                     |
//! | 0x3C   | T1LOADLO         |                                     |
//! | 0x40   | T1LOADHI         |                                     |
//! | 0x44   | T1LOAD           |                                     |
//! | 0x48   | WDTCONFIG0       | Round-trip                          |
//! | 0x4C   | WDTCONFIG1       |                                     |
//! | 0x50   | WDTCONFIG2       |                                     |
//! | 0x54   | WDTCONFIG3       |                                     |
//! | 0x58   | WDTCONFIG4       |                                     |
//! | 0x5C   | WDTFEED          | Write-only; ack silently            |
//! | 0x60   | WDTWPROTECT      | Round-trip                          |
//! | 0x68   | RTCCALICFG       | START bit latches RDY               |
//! | 0x6C   | RTCCALICFG1      | Returns canned calibration value    |
//! | 0x98   | INT_ENA          | Round-trip                          |
//! | 0x9C   | INT_RAW          | Round-trip (no auto-set today)      |
//! | 0xA0   | INT_ST           | Round-trip                          |
//! | 0xA4   | INT_CLR          | Write clears matching INT_RAW bits  |
//!
//! Offsets we don't enumerate (e.g. NTIMG_DATE at 0xF8, CLK at 0xFC)
//! fall through to a generic round-trip via the `regs` HashMap so RMW
//! probes from firmware still see their own writes.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::collections::HashMap;

// Per-timer register offsets (T0 block starts at 0x00, T1 block at 0x24).
// Some entries (`*_ALARM*`, `WDT_CONFIG0`, `INT_ENA`, `INT_ST`) aren't
// referenced by the model today — they round-trip through the generic
// `regs` HashMap. They're kept here for spec-completeness and to make
// future "actually fire the alarm IRQ" work a name-only diff.
const T0_CONFIG: u64 = 0x00;
const T0_LO: u64 = 0x04;
const T0_HI: u64 = 0x08;
const T0_UPDATE: u64 = 0x0C;
#[allow(dead_code)]
const T0_ALARMLO: u64 = 0x10;
#[allow(dead_code)]
const T0_ALARMHI: u64 = 0x14;
const T0_LOADLO: u64 = 0x18;
const T0_LOADHI: u64 = 0x1C;
const T0_LOAD: u64 = 0x20;

const T1_CONFIG: u64 = 0x24;
const T1_LO: u64 = 0x28;
const T1_HI: u64 = 0x2C;
const T1_UPDATE: u64 = 0x30;
#[allow(dead_code)]
const T1_ALARMLO: u64 = 0x34;
#[allow(dead_code)]
const T1_ALARMHI: u64 = 0x38;
const T1_LOADLO: u64 = 0x3C;
const T1_LOADHI: u64 = 0x40;
const T1_LOAD: u64 = 0x44;

// Watchdog
#[allow(dead_code)]
const WDT_CONFIG0: u64 = 0x48;
const WDT_FEED: u64 = 0x5C;
const WDT_WPROTECT: u64 = 0x60;

// RTC calibration (offsets match the pre-existing TimgStub).
const RTCCALICFG: u64 = 0x68;
const RTCCALICFG1: u64 = 0x6C;
const RTC_CALI_START_BIT: u32 = 1 << 31;
const RTC_CALI_RDY_BIT: u32 = 1 << 15;

/// Silicon-faithful RTC_SLOW calibration profile for a specific SoC.
///
/// The TIMG0 RTC-calibration feature counts how many `xtal_hz` reference cycles
/// elapse during `TIMG_RTC_CALI_MAX` cycles of the RTC_SLOW clock (this is the
/// "special feature of TIMG0" IDF's `rtc_clk_cal` drives — see
/// esp-idf `esp_hw_support/port/esp32c3/rtc_time.c`). When a `Timg` is given a
/// profile, the model synthesises the counted value from `xtal_hz` and
/// `slow_hz` instead of returning a hardcoded ratio — so firmware that
/// calibrates recovers exactly `slow_hz` (`freq = xtal_hz * cycles / value`),
/// welding the reported rate to the *same* constant the RTC_CNTL counter ticks
/// at. `None` keeps the legacy ESP32-classic behaviour byte-for-byte.
#[derive(Debug, Clone, Copy)]
pub struct RtcCalProfile {
    /// Reference clock the calibration counts against (40 MHz XTAL on C3).
    pub xtal_hz: u64,
    /// The modelled RTC_SLOW frequency the cal result must recover.
    pub slow_hz: u64,
}

// Interrupt plumbing. INT_ENA / INT_ST round-trip through `regs` — they're
// declared here so the spec table in the module docstring stays grep-able.
#[allow(dead_code)]
const INT_ENA: u64 = 0x98;
const INT_RAW: u64 = 0x9C;
#[allow(dead_code)]
const INT_ST: u64 = 0xA0;
const INT_CLR: u64 = 0xA4;

// CONFIG.EN is bit 31 on the ESP32-classic TIMG block.
const T_CONFIG_EN_BIT: u32 = 1 << 31;

/// Timer Group (TIMG0 or TIMG1) peripheral model.
///
/// The `base` field is informational — the bus already routes reads/writes
/// relative to offset 0, so the model only sees `offset` in `read`/`write`.
/// We keep `base` so logging / snapshot dumps can disambiguate TIMG0 vs
/// TIMG1 in a multi-instance trace.
#[derive(Debug)]
pub struct Timg {
    /// Peripheral base address (0x3FF5_F000 for TIMG0, 0x3FF6_0000 for
    /// TIMG1). Kept for debugging only.
    base: u32,
    /// Word-aligned register backing store. Any offset not explicitly
    /// computed in `read()` falls through to this map (or zero).
    regs: HashMap<u64, u32>,
    /// Live 64-bit value for timer 0. Advances on every `tick()` while
    /// `T0CONFIG.EN` is set. Latched into `T0_LO`/`T0_HI` on a write to
    /// `T0_UPDATE` (and on read of LO/HI as a safety net so firmware that
    /// skips the strobe still sees forward progress).
    counter_t0: u64,
    /// Live 64-bit value for timer 1. Same semantics as `counter_t0`.
    counter_t1: u64,
    /// Phase 2B.2 (issue #192): peripheral-tick index of the last `sync_to`.
    /// In scheduler mode the counters no longer advance one-per-`tick()`;
    /// instead `sync_to(now)` lazily adds `(now - anchor_tick)` to each
    /// enabled counter on MMIO access, and `tick()` is never called (the bus
    /// skips `uses_scheduler()` peripherals in its per-cycle walk). Unused in
    /// the legacy (flag-off) build, where `tick()` drives the counters.
    anchor_tick: u64,
    /// Silicon-faithful RTC_SLOW calibration profile. `Some` for the ESP32-C3
    /// TIMG0 (the calibration timer IDF drives): the RTCCALICFG result is
    /// derived from the modelled RTC_SLOW rate through the real register
    /// protocol. `None` keeps the ESP32-classic canned-ratio behaviour
    /// byte-for-byte.
    rtc_cal: Option<RtcCalProfile>,
}

impl Timg {
    /// Create a new TIMG instance for the given base address.
    pub fn new(base: u32) -> Self {
        Self {
            base,
            regs: HashMap::new(),
            counter_t0: 0,
            counter_t1: 0,
            anchor_tick: 0,
            rtc_cal: None,
        }
    }

    /// Attach a silicon-faithful RTC_SLOW calibration profile (ESP32-C3 TIMG0).
    /// With a profile, RTCCALICFG uses the real C3 field layout (MAX at bits
    /// [30:16]) and returns a counted value derived from the profile's
    /// frequencies, so `rtc_clk_cal` recovers exactly `profile.slow_hz`.
    pub fn with_rtc_cal(mut self, profile: RtcCalProfile) -> Self {
        self.rtc_cal = Some(profile);
        self
    }

    /// Base address this instance is registered at (debug helper).
    pub fn base(&self) -> u32 {
        self.base
    }

    /// Live T0 counter (debug helper / test introspection).
    pub fn counter_t0(&self) -> u64 {
        self.counter_t0
    }

    /// Live T1 counter (debug helper / test introspection).
    pub fn counter_t1(&self) -> u64 {
        self.counter_t1
    }

    fn word(&self, off: u64) -> u32 {
        self.regs.get(&off).copied().unwrap_or(0)
    }

    fn is_t0_enabled(&self) -> bool {
        self.word(T0_CONFIG) & T_CONFIG_EN_BIT != 0
    }

    fn is_t1_enabled(&self) -> bool {
        self.word(T1_CONFIG) & T_CONFIG_EN_BIT != 0
    }

    /// Latch the live `counter_t0` into the T0_LO/T0_HI register pair so
    /// the next firmware read sees a coherent 64-bit value.
    fn latch_t0(&mut self) {
        self.regs.insert(T0_LO, self.counter_t0 as u32);
        self.regs.insert(T0_HI, (self.counter_t0 >> 32) as u32);
    }

    fn latch_t1(&mut self) {
        self.regs.insert(T1_LO, self.counter_t1 as u32);
        self.regs.insert(T1_HI, (self.counter_t1 >> 32) as u32);
    }

    /// Preload T0 from the LOADLO/LOADHI register pair (silicon copies
    /// those into the running counter on any write to T0_LOAD).
    fn preload_t0(&mut self) {
        let lo = self.word(T0_LOADLO) as u64;
        let hi = self.word(T0_LOADHI) as u64;
        self.counter_t0 = (hi << 32) | lo;
        self.latch_t0();
    }

    fn preload_t1(&mut self) {
        let lo = self.word(T1_LOADLO) as u64;
        let hi = self.word(T1_LOADHI) as u64;
        self.counter_t1 = (hi << 32) | lo;
        self.latch_t1();
    }

    /// Dispatch the per-word side effects of a register write. Factored
    /// out so both byte-granular `write` and word-granular `write_u32`
    /// produce identical observable state. Idempotent for all the
    /// triggers below (they read live state from `regs`/counters and
    /// write back deterministic values), so calling once per word or
    /// four times per word produces the same outcome.
    fn apply_write_side_effects(&mut self, word_off: u64) {
        match word_off {
            T0_UPDATE => self.latch_t0(),
            T1_UPDATE => self.latch_t1(),
            T0_LOAD => self.preload_t0(),
            T1_LOAD => self.preload_t1(),
            WDT_FEED | WDT_WPROTECT => {
                // Watchdog feed / write-protect: round-trip the value.
                // WDT timing/reset behavior isn't modeled.
            }
            INT_CLR => {
                // Write-1-to-clear: clear matching bits in INT_RAW.
                let mask = self.word(INT_CLR);
                let raw = self.word(INT_RAW);
                self.regs.insert(INT_RAW, raw & !mask);
            }
            RTCCALICFG => self.maybe_complete_rtc_calibration(),
            _ => {}
        }
    }

    /// Apply RTC calibration completion semantics. When firmware sets
    /// `RTCCALICFG.START`, we immediately mark `RDY` and stash a counted value
    /// in `RTCCALICFG1` so the calibration loop completes in one read.
    ///
    /// With a [`RtcCalProfile`] (ESP32-C3 TIMG0) the value is *derived from the
    /// modelled RTC_SLOW rate* through the real register protocol; without one
    /// the legacy ESP32-classic canned ratio is preserved byte-for-byte.
    fn maybe_complete_rtc_calibration(&mut self) {
        let cfg = self.word(RTCCALICFG);
        if cfg & RTC_CALI_START_BIT == 0 {
            return;
        }
        match self.rtc_cal {
            // ── ESP32-C3 TIMG0: silicon-faithful, self-consistent ──────────
            // TIMG0 counts how many XTAL cycles elapse during MAX cycles of
            // RTC_SLOW (esp-idf rtc_clk_cal). C3 field layout (TRM / IDF
            // soc/esp32c3/timer_group_reg.h): MAX = RTCCALICFG bits[30:16],
            // RDY = bit15, VALUE = RTCCALICFG1 bits[31:7]. Synthesise the
            // counted XTAL cycles from the profile so that IDF's inverse
            // (`freq = xtal_hz * slowclk_cycles / value`) recovers exactly
            // `slow_hz` — the SAME constant the RTC_CNTL counter ticks at.
            Some(profile) => {
                let slowclk_cycles = u64::from((cfg >> 16) & 0x7FFF).max(1);
                let xtal_cycles =
                    (profile.xtal_hz * slowclk_cycles + profile.slow_hz / 2) / profile.slow_hz;
                let value = (xtal_cycles as u32) & 0x01FF_FFFF; // 25-bit VALUE field
                self.regs.insert(RTCCALICFG, cfg | RTC_CALI_RDY_BIT);
                self.regs.insert(RTCCALICFG1, value << 7);
            }
            // ── ESP32-classic: preserved canned ratio (no behaviour change) ─
            None => {
                let max = ((cfg >> 13) & 0x1FFFF).max(1);
                self.regs.insert(RTCCALICFG, cfg | RTC_CALI_RDY_BIT);
                // ratio ≈ 533 cycles per RTC_SLOW_CLK period at APB=80 MHz.
                let value = max.wrapping_mul(533) & 0x01FF_FFFF;
                self.regs.insert(RTCCALICFG1, (value << 7) | 1);
            }
        }
    }
}

impl Peripheral for Timg {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;

        // Counter reads use the latched LO/HI registers (set on
        // T_UPDATE write or load). For consumers that skip the strobe,
        // we fall back to a live view of the in-RAM counter so they
        // still observe forward progress instead of a stuck zero.
        let word = match word_off {
            T0_LO => self
                .regs
                .get(&T0_LO)
                .copied()
                .unwrap_or(self.counter_t0 as u32),
            T0_HI => self
                .regs
                .get(&T0_HI)
                .copied()
                .unwrap_or((self.counter_t0 >> 32) as u32),
            T1_LO => self
                .regs
                .get(&T1_LO)
                .copied()
                .unwrap_or(self.counter_t1 as u32),
            T1_HI => self
                .regs
                .get(&T1_HI)
                .copied()
                .unwrap_or((self.counter_t1 >> 32) as u32),
            _ => self.word(word_off),
        };
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let entry = self.regs.entry(word_off).or_insert(0);
        *entry &= !(0xFFu32 << byte_off);
        *entry |= (value as u32) << byte_off;
        self.apply_write_side_effects(word_off);
        Ok(())
    }

    // Word-granular fast paths. The default trait impls would issue 4×
    // byte ops (each hashing through `regs`); the bench polls T0_LO and
    // INT_RAW heavily. Reads look up the word once with the same LO/HI
    // counter fallback as the byte path. Writes overwrite the word in
    // one shot, then dispatch side effects via the shared helper so the
    // observable state is identical to four byte writes.
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let word_off = offset & !3;
        let word = match word_off {
            T0_LO => self
                .regs
                .get(&T0_LO)
                .copied()
                .unwrap_or(self.counter_t0 as u32),
            T0_HI => self
                .regs
                .get(&T0_HI)
                .copied()
                .unwrap_or((self.counter_t0 >> 32) as u32),
            T1_LO => self
                .regs
                .get(&T1_LO)
                .copied()
                .unwrap_or(self.counter_t1 as u32),
            T1_HI => self
                .regs
                .get(&T1_HI)
                .copied()
                .unwrap_or((self.counter_t1 >> 32) as u32),
            _ => self.word(word_off),
        };
        Ok(word)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let word_off = offset & !3;
        self.regs.insert(word_off, value);
        self.apply_write_side_effects(word_off);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Each `tick()` advances the deterministic 1-µs clock. The bus
        // calls us once per simulated µs (240 CPU cycles at 240 MHz),
        // so a saturating +1 here gives a 1 MHz timer — exactly the
        // ESP-IDF default for the 80 MHz APB / 80 divider.
        if self.is_t0_enabled() {
            self.counter_t0 = self.counter_t0.wrapping_add(1);
        }
        if self.is_t1_enabled() {
            self.counter_t1 = self.counter_t1.wrapping_add(1);
        }
        // No interrupt firing this round — see module docs. Routing is
        // a separate task.
        PeripheralTickResult::default()
    }

    /// Phase 2B.2 (issue #192): TIMG is the first peripheral to migrate to the
    /// event scheduler. With the `event-scheduler` feature on, the bus stops
    /// calling `tick()` every cycle and instead calls `sync_to` lazily on MMIO
    /// access. (No-op effect when the feature is off — the bus ignores this.)
    fn uses_scheduler(&self) -> bool {
        true
    }

    /// Advance the enabled counters to peripheral-tick `tick_now`. Equivalent
    /// to having called `tick()` once per intervening tick while enabled, but
    /// computed in O(1) at access time instead of once per cycle. Disabled
    /// timers don't accrue: `anchor_tick` is always moved to `tick_now`, so a
    /// span spent disabled is excluded once the timer is re-enabled. `tick_now`
    /// is monotonic (driven by `Machine::total_cycles`), so the delta is never
    /// negative; `saturating_sub` guards the degenerate equal/rewind case.
    fn sync_to(&mut self, tick_now: u64) {
        // Monotonic guard: a non-advancing or rewinding `tick_now` is a no-op,
        // and the anchor never moves backward (else the next sync would
        // double-count the reclaimed span).
        if tick_now <= self.anchor_tick {
            return;
        }
        let delta = tick_now - self.anchor_tick;
        if self.is_t0_enabled() {
            self.counter_t0 = self.counter_t0.wrapping_add(delta);
        }
        if self.is_t1_enabled() {
            self.counter_t1 = self.counter_t1.wrapping_add(delta);
        }
        self.anchor_tick = tick_now;
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

    fn write_u32(t: &mut Timg, off: u64, val: u32) {
        for i in 0..4 {
            t.write(off + i, ((val >> (i * 8)) & 0xFF) as u8).unwrap();
        }
    }

    fn read_u32(t: &Timg, off: u64) -> u32 {
        let mut v = 0u32;
        for i in 0..4 {
            v |= (t.read(off + i).unwrap() as u32) << (i * 8);
        }
        v
    }

    #[test]
    fn config_round_trips() {
        let mut t = Timg::new(0x3FF5_F000);
        // Writing T0CONFIG returns what was written byte-by-byte.
        write_u32(&mut t, T0_CONFIG, 0xDEAD_BEEF);
        assert_eq!(read_u32(&t, T0_CONFIG), 0xDEAD_BEEF);
    }

    #[test]
    fn t0_lo_monotonically_increases_when_enabled() {
        let mut t = Timg::new(0x3FF5_F000);
        // Enable T0 by setting CONFIG.EN (bit 31).
        write_u32(&mut t, T0_CONFIG, T_CONFIG_EN_BIT);

        // Advance the simulated clock by some ticks.
        for _ in 0..100 {
            t.tick();
        }
        // Latch then read.
        write_u32(&mut t, T0_UPDATE, 1);
        let v1 = read_u32(&t, T0_LO);
        assert!(v1 >= 100, "counter should have advanced by ≥100, got {v1}");

        for _ in 0..50 {
            t.tick();
        }
        write_u32(&mut t, T0_UPDATE, 1);
        let v2 = read_u32(&t, T0_LO);
        assert!(v2 > v1, "counter must be monotonically increasing");
        assert_eq!(v2 - v1, 50, "counter should advance by exactly 50 ticks");
    }

    #[test]
    fn lazy_sync_advances_enabled_counter() {
        // Phase 2B.2: in scheduler mode the counter advances via sync_to, not
        // tick(). Enable T0, sync to tick 100, strobe, read → 100.
        let mut t = Timg::new(0x3FF5_F000);
        write_u32(&mut t, T0_CONFIG, T_CONFIG_EN_BIT);
        t.sync_to(100);
        write_u32(&mut t, T0_UPDATE, 1);
        assert_eq!(read_u32(&t, T0_LO), 100);

        // A second sync adds only the new delta.
        t.sync_to(175);
        write_u32(&mut t, T0_UPDATE, 1);
        assert_eq!(read_u32(&t, T0_LO), 175);
    }

    #[test]
    fn lazy_sync_excludes_disabled_span() {
        // A span spent disabled must not accrue: sync past it while disabled,
        // then enable and sync again — only the post-enable delta counts.
        let mut t = Timg::new(0x3FF5_F000);
        t.sync_to(50); // disabled: anchor moves, counter stays 0
        write_u32(&mut t, T0_CONFIG, T_CONFIG_EN_BIT);
        t.sync_to(100); // enabled: +50
        write_u32(&mut t, T0_UPDATE, 1);
        assert_eq!(read_u32(&t, T0_LO), 50, "disabled span [0,50) excluded");
    }

    #[test]
    fn lazy_sync_is_monotonic() {
        // A non-advancing or rewinding tick_now is a no-op (saturating delta).
        let mut t = Timg::new(0x3FF5_F000);
        write_u32(&mut t, T0_CONFIG, T_CONFIG_EN_BIT);
        t.sync_to(100);
        t.sync_to(40); // rewind ignored
        t.sync_to(100); // same value → no double count
        write_u32(&mut t, T0_UPDATE, 1);
        assert_eq!(read_u32(&t, T0_LO), 100);
    }

    #[test]
    fn uses_scheduler_is_true() {
        assert!(Timg::new(0x3FF5_F000).uses_scheduler());
    }

    #[test]
    fn t0_does_not_advance_when_disabled() {
        let mut t = Timg::new(0x3FF5_F000);
        // CONFIG.EN cleared (default).
        for _ in 0..100 {
            t.tick();
        }
        write_u32(&mut t, T0_UPDATE, 1);
        assert_eq!(read_u32(&t, T0_LO), 0);
    }

    #[test]
    fn wdt_feed_is_acknowledged_silently() {
        let mut t = Timg::new(0x3FF5_F000);
        // Any write to WDT_FEED must not panic and must round-trip in regs.
        write_u32(&mut t, WDT_FEED, 0x5000_0000);
        // No publicly observable state change beyond the register backing
        // store; assertion is "no panic, returns Ok".
        // Read still works (returns the written value — no auto-clear).
        assert_eq!(read_u32(&t, WDT_FEED), 0x5000_0000);
    }

    #[test]
    fn preload_copies_loadlo_loadhi_into_counter() {
        let mut t = Timg::new(0x3FF5_F000);
        write_u32(&mut t, T0_LOADLO, 0x1111_2222);
        write_u32(&mut t, T0_LOADHI, 0x0000_0003);
        // Writing T0_LOAD triggers the preload.
        write_u32(&mut t, T0_LOAD, 1);
        write_u32(&mut t, T0_UPDATE, 1);
        assert_eq!(read_u32(&t, T0_LO), 0x1111_2222);
        assert_eq!(read_u32(&t, T0_HI), 0x0000_0003);
    }

    #[test]
    fn rtc_cali_start_latches_rdy() {
        // Preserves the TimgStub behavior that downstream RTC code expects.
        let mut t = Timg::new(0x3FF5_F000);
        // START=1, MAX=0x100 (in bits[29:13] → shift left by 13).
        let cfg = RTC_CALI_START_BIT | (0x100u32 << 13);
        write_u32(&mut t, RTCCALICFG, cfg);
        let read_back = read_u32(&t, RTCCALICFG);
        assert!(
            read_back & RTC_CALI_RDY_BIT != 0,
            "RDY bit should be set immediately after START"
        );
    }

    #[test]
    fn classic_rtc_cali_value_is_unchanged() {
        // Regression pin for the ESP32-classic (profile = None) path: the exact
        // canned value the pre-split model produced must be byte-for-byte
        // preserved. MAX=0x100 at bits[29:13] → model reads 0x100, value =
        // 0x100*533 = 0x21500, RTCCALICFG1 = (value<<7)|1.
        let mut t = Timg::new(0x3FF5_F000);
        write_u32(&mut t, RTCCALICFG, RTC_CALI_START_BIT | (0x100u32 << 13));
        assert!(read_u32(&t, RTCCALICFG) & RTC_CALI_RDY_BIT != 0);
        assert_eq!(read_u32(&t, RTCCALICFG1), ((0x100u32 * 533) << 7) | 1);
    }

    #[test]
    fn c3_rtc_cali_reports_the_modelled_slow_rate() {
        // ESP32-C3 TIMG0 with the silicon cal profile: drive the exact IDF
        // register protocol (MAX at bits[30:16], START bit31, poll RDY bit15,
        // read VALUE at RTCCALICFG1 bits[31:7]) and confirm IDF's inverse
        // recovers the modelled RTC_SLOW rate — no hardcoded value.
        const XTAL: u64 = 40_000_000;
        const SLOW: u64 = 148_150;
        let mut t = Timg::new(0x6001_F000).with_rtc_cal(RtcCalProfile {
            xtal_hz: XTAL,
            slow_hz: SLOW,
        });
        for &cycles in &[100u32, 1024, 3000, 0x7FFF] {
            // CLK_SEL=0 (RTC_MUX/150k), START_CYCLING=0, MAX=cycles, START=1.
            let cfg = RTC_CALI_START_BIT | (cycles << 16);
            write_u32(&mut t, RTCCALICFG, cfg);
            assert!(
                read_u32(&t, RTCCALICFG) & RTC_CALI_RDY_BIT != 0,
                "RDY must latch after START (cycles={cycles})"
            );
            let value = (read_u32(&t, RTCCALICFG1) >> 7) as u64; // VALUE = XTAL cycles
            assert!(value > 0, "cal value must be non-zero (cycles={cycles})");
            // IDF: freq = xtal_hz * slowclk_cycles / xtal_cycles.
            let recovered = XTAL * u64::from(cycles) / value;
            let err = recovered.abs_diff(SLOW);
            assert!(
                err <= 20,
                "cycles={cycles}: recovered {recovered} Hz must ~= modelled {SLOW} Hz \
                 (err {err} Hz, rounding only)"
            );
        }
    }

    #[test]
    fn int_clr_clears_matching_int_raw_bits() {
        let mut t = Timg::new(0x3FF5_F000);
        // Pre-load INT_RAW with bits 0 and 1 set.
        write_u32(&mut t, INT_RAW, 0b11);
        // Clear bit 0.
        write_u32(&mut t, INT_CLR, 0b01);
        assert_eq!(read_u32(&t, INT_RAW), 0b10);
    }

    #[test]
    fn unknown_offsets_round_trip() {
        // ESP-IDF init pokes at offsets we don't explicitly model; they
        // must round-trip through `regs` rather than read-as-zero, so RMW
        // sequences see their own writes.
        let mut t = Timg::new(0x3FF5_F000);
        write_u32(&mut t, 0x70, 0xCAFE_BABE);
        assert_eq!(read_u32(&t, 0x70), 0xCAFE_BABE);
    }
}
