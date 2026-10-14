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
//!   * Watchdog feeds (any write to the classic `WDT_FEED_REG`) are silently
//!     accepted; we don't model WDT-induced resets. The C3/C6 MWDT layout can
//!     opt into the real four-stage chain with [`Timg::with_mwdt`] (write
//!     lock, SVD config/hold resets, stage holds, per-stage actions, feed,
//!     INT latch) — see its docs for the exact boundary of what is and is not
//!     claimed.
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

// LACT — the "legacy"/local accurate clock timer (TRM §16.3). This is the
// time source `esp_timer` runs on for ESP32-classic
// (esp_timer_impl_lac.c), so it is what `esp_timer_get_time()`, and
// therefore Arduino's `micros()` and `millis()`, ultimately read.
//
// `esp_timer_impl_get_counter_reg` strobes LACT_UPDATE, waits for LACT_LO
// to change, then reads the LO/HI pair; `esp_timer_get_time` returns that
// 64-bit value >> 1, so LACT ticks at 2 MHz for a 1 µs result — which is
// APB (80 MHz) / 40, the divider IDF programs.
const LACT_CONFIG: u64 = 0x70;
#[allow(dead_code)]
const LACT_RTC: u64 = 0x74;
const LACT_LO: u64 = 0x78;
const LACT_HI: u64 = 0x7C;
const LACT_UPDATE: u64 = 0x80;
#[allow(dead_code)]
const LACT_ALARMLO: u64 = 0x84;
#[allow(dead_code)]
const LACT_ALARMHI: u64 = 0x88;
const LACT_LOADLO: u64 = 0x8C;
const LACT_LOADHI: u64 = 0x90;
const LACT_LOAD: u64 = 0x94;

/// LACT_CONFIG.LACT_EN.
const LACT_EN_BIT: u32 = 1 << 31;
/// LACT_CONFIG.LACT_DIVIDER occupies bits [28:13] — the field
/// `esp_timer_impl_get_counter_reg` itself recovers with
/// `extui a8, a8, 13, 16`.
const LACT_DIVIDER_SHIFT: u32 = 13;
const LACT_DIVIDER_BITS: u32 = 16;
/// APB clock in MHz. `tick()` runs once per simulated microsecond, so this
/// is exactly how many APB cycles elapse per tick.
const APB_MHZ: u32 = 80;
/// ESP32-classic CPU clock in MHz (`configs/chips/esp32.yaml` `cpu_hz`).
/// Both the legacy walk (`tick()`) and the scheduler (`sync_to`) advance this
/// model in CPU cycles, so LACT converts CPU cycles to APB cycles at
/// `APB_MHZ / LACT_CPU_MHZ` — otherwise `esp_timer_get_time()` (and with it
/// Arduino `millis()`/`micros()`/`delayMicroseconds()`) runs 240x fast.
const LACT_CPU_MHZ: u32 = 240;

// Watchdog. The module's default layout is the ESP32-classic one: five config
// registers WDTCONFIG0..4 @0x48..0x58, WDTFEED @0x5C, WDTWPROTECT @0x60.
// `WDT_FEED`/`WDT_WPROTECT` are the classic offsets and the classic write
// handling stays a documented no-op.
const WDT_CONFIG0: u64 = 0x48;
const WDT_CONFIG1: u64 = 0x4C;
const WDT_CONFIG2: u64 = 0x50;
const WDT_CONFIG3: u64 = 0x54;
const WDT_CONFIG4: u64 = 0x58;
const WDT_FEED: u64 = 0x5C;
const WDT_WPROTECT: u64 = 0x60;

// ── ESP32-C3/C6 MWDT layout (opt in with `with_mwdt`) ──────────────────────
//
// The C3 and C6 TIMG have SIX stage-hold registers: WDTCONFIG0..5 @0x48..0x5C,
// WDTFEED @0x60, WDTWPROTECT @0x64 (C3 SVD `TIMG0`; C6 SVD `TIMG0` — the two
// are offset-identical). The C6 chip yaml's original comment called the old
// 0x5C/0x60 handling "a naming difference, not a behavioural one" because the
// classic handler was already a no-op; with the real MWDT path below the
// offsets matter, so they are explicit.
/// WDTCONFIG5 (STG3_HOLD) — C3/C6 only.
const WDT_CONFIG5_C3: u64 = 0x5C;
/// WDTFEED (WO) on the C3/C6 layout.
const WDT_FEED_C3: u64 = 0x60;
/// WDTWPROTECT (WDT_WKEY) on the C3/C6 layout.
const WDT_WPROTECT_C3: u64 = 0x64;
/// INT_ENA_TIMERS: bit0 T0, bit1 WDT (C3/C6 in-place of the classic INT_* @0x98).
const WDT_INT_ENA_C3: u64 = 0x70;
/// INT_RAW_TIMERS: bit0 T0, bit1 WDT.
const WDT_INT_RAW_C3: u64 = 0x74;
/// INT_ST_TIMERS: raw AND enable.
const WDT_INT_ST_C3: u64 = 0x78;
/// INT_CLR_TIMERS: W1C for bit0/bit1.
const WDT_INT_CLR_C3: u64 = 0x7C;

/// WDTWPROTECT reset value / unlock key (`WDT_WKEY`, 1356348065). Silicon:
/// "If the register contains a different value than its reset value, write
/// protection is enabled", so the block is UNLOCKED at reset. Locking is a
/// firmware action (IDF writes an arbitrary different value, usually 0).
const WDT_WKEY_VALUE: u32 = 0x50D8_3AA1;

/// SVD reset values for the C3/C6 MWDT surface (esp32c6.svd `TIMG0`:
/// WDTCONFIG0 0x0004_C000, WDTCONFIG1 0x0001_0000, WDTCONFIG2 0x018C_BA80,
/// WDTCONFIG3 0x07FF_FFFF, WDTCONFIG4/5 0x000F_FFFF; WDTWPROTECT resets to
/// WDT_WKEY). Seeded by [`Timg::with_mwdt`] so the unconfigured stages behave
/// like silicon's — notably, an interrupt-action stage 0 with stages 1..3 left
/// alone falls through to silicon's long default holds instead of instantly
/// wrapping.
const MWDT_RESETS: [(u64, u32); 7] = [
    (WDT_CONFIG0, 0x0004_C000),
    (WDT_CONFIG1, 0x0001_0000),
    (WDT_CONFIG2, 0x018C_BA80),
    (WDT_CONFIG3, 0x07FF_FFFF),
    (WDT_CONFIG4, 0x000F_FFFF),
    (WDT_CONFIG5_C3, 0x000F_FFFF),
    (WDT_WPROTECT_C3, WDT_WKEY_VALUE),
];
/// WDTCONFIG0.WDT_EN (bit 31).
const WDT_EN_BIT: u32 = 1 << 31;
/// WDTCONFIG0 stage-action fields, one 2-bit field per stage:
/// WDT_STG0 [30:29], WDT_STG1 [28:27], WDT_STG2 [26:25], WDT_STG3 [24:23]
/// (C6 SVD `WDTCONFIG0`; offset-identical on the C3).
const WDT_STG_SHIFTS: [u32; WDT_NUM_STAGES as usize] = [29, 27, 25, 23];
const WDT_STG_MASK: u32 = 0x3;
/// The four stage-hold registers WDTCONFIG2..5 @0x50..0x5C, indexed by stage.
const WDT_STG_HOLD_REGS: [u64; WDT_NUM_STAGES as usize] =
    [WDT_CONFIG2, WDT_CONFIG3, WDT_CONFIG4, WDT_CONFIG5_C3];
/// Stage action 1 = "interrupt" (the only stage action this model acts on;
/// 2/3 are the CPU/system reset actions — see `with_mwdt`).
const WDT_STAGE_ACTION_INT: u32 = 1;
/// INT_RAW_TIMERS.WDT_INT_RAW (bit 1).
const WDT_INT_RAW_BIT: u32 = 1 << 1;
/// MWDT stages. The hardware walks 0 -> 1 -> 2 -> 3 -> 0 (`TRM §WDT`:
/// "The watchdog timers will progress through each stage in a loop").
const WDT_NUM_STAGES: u8 = 4;

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
    regs: crate::FastMap<u64, u32>,
    /// Live 64-bit value for timer 0. Advances on every `tick()` while
    /// `T0CONFIG.EN` is set. Latched into `T0_LO`/`T0_HI` on a write to
    /// `T0_UPDATE` (and on read of LO/HI as a safety net so firmware that
    /// skips the strobe still sees forward progress).
    counter_t0: u64,
    /// Live 64-bit value for timer 1. Same semantics as `counter_t0`.
    counter_t1: u64,
    /// Live 64-bit LACT counter — the `esp_timer` time base. Advances by
    /// `APB_MHZ / divider` per tick while `LACT_CONFIG.EN` is set, with the
    /// remainder carried in `lact_rem` so a divider that does not divide 80
    /// still averages out to the right rate instead of truncating to zero.
    counter_lact: u64,
    /// Undivided APB cycles not yet converted into LACT ticks.
    lact_rem: u32,
    /// Is this a TIMG that HAS a LACT timer? Only ESP32-classic does.
    ///
    /// The ESP32-C3 reuses this model (see generic_factory.rs) but dropped
    /// LACT in favour of SYSTIMER, and put entirely different registers in
    /// the same offsets: 0x78/0x7C are INT_ST_TIMERS/INT_CLR_TIMERS and 0x80
    /// is RTCCALICFG2. Answering those offsets as LACT there would have a
    /// write to RTCCALICFG2 latch the counter over the C3's interrupt status
    /// and clear registers. So LACT is opt-in, and only the ESP32-classic
    /// factory opts in.
    lact_enabled: bool,
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
    /// C3/C6 MWDT layout + behaviour enabled (`with_mwdt`). `false` keeps the
    /// ESP32-classic shape exactly: 0x5C/0x60 are the feed/protect no-op pair,
    /// 0x70..0x7C are plain storage, and no watchdog state exists. The C3 chip
    /// yaml never opts in, so this flag is the whole chip gate.
    mwdt: bool,
    /// MWDT countdown of the ACTIVE stage, in the same model-tick unit
    /// `counter_t0` advances by (`tick()` = 1, `sync_to` = elapsed CPU cycles).
    /// Meaningful only while `mwdt`.
    wdt_countdown: u64,
    /// The watchdog's active stage (0..3). Expiry advances to the next stage
    /// and loads that stage's hold; a feed or an EN 0→1 edge returns to 0.
    wdt_stage: u8,
    /// The stage chain is armed and counting. Cleared when the watchdog is
    /// disabled; an expiry does NOT clear it (the chain loops 0..3).
    wdt_armed: bool,
    /// Sticky INT_RAW_TIMERS.WDT_INT_RAW latch. Set by any interrupt-action
    /// stage's expiry; cleared only by INT_CLR_TIMERS (a feed does NOT clear
    /// it, matching silicon).
    wdt_pending: bool,
    /// Mirror of WDTCONFIG0.WDT_EN, used to detect the 0→1 edge that arms the
    /// stage chain at stage 0. Kept as a field because
    /// `apply_write_side_effects` runs after the register store, when the old
    /// bit is already gone.
    wdt_enabled: bool,
}

impl Timg {
    /// Create a new TIMG instance for the given base address.
    pub fn new(base: u32) -> Self {
        Self {
            base,
            regs: crate::FastMap::default(),
            counter_t0: 0,
            counter_t1: 0,
            counter_lact: 0,
            lact_rem: 0,
            lact_enabled: false,
            anchor_tick: 0,
            rtc_cal: None,
            mwdt: false,
            wdt_countdown: 0,
            wdt_stage: 0,
            wdt_armed: false,
            wdt_pending: false,
            wdt_enabled: false,
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

    /// Declare that this TIMG has a LACT timer (ESP32-classic only).
    ///
    /// Without this, every LACT offset falls through to the generic
    /// round-trip map, which is exactly the behaviour a chip that has no LACT
    /// needs — see the note on `lact_enabled`.
    pub fn with_lact(mut self) -> Self {
        self.lact_enabled = true;
        self
    }

    /// Opt into the C3/C6 MWDT layout and behaviour.
    ///
    /// What this enables (offsets per the C3/C6 SVD, see the `*_C3` constants):
    ///
    /// * **Write protection.** WDTWPROTECT (@0x64) resets to [`WDT_WKEY_VALUE`]
    ///   (unlocked, matching silicon). While it holds any other value, writes to
    ///   WDTCONFIG0..5 (@0x48..0x5C, the six-register C3/C6 layout) are dropped.
    ///   WDTFEED and WDTWPROTECT themselves stay writable while locked, as on
    ///   silicon (IDF feeds the watchdog without unlocking it).
    /// * **SVD reset values.** WDTCONFIG0..5 are seeded with their esp32c6.svd
    ///   resets (see [`MWDT_RESETS`]), so the unconfigured stages hold silicon's
    ///   long defaults instead of zero.
    /// * **WDTCONFIG0/1 (and 2..5) round-trip** while unlocked, including the
    ///   enable bit and every stage field.
    /// * **The full four-stage chain.** On the WDTCONFIG0 0→1 EN edge the model
    ///   loads stage 0's hold from WDTCONFIG2 (STG0_HOLD) and counts it down in
    ///   model ticks. When a stage expires the configured action is taken (see
    ///   below), the counter reloads the NEXT stage's hold (STG1_HOLD @0x54,
    ///   STG2_HOLD @0x58, STG3_HOLD @0x5C) and the active stage advances
    ///   `0 -> 1 -> 2 -> 3 -> 0`, looping as silicon does (TRM: "the watchdog
    ///   timers will progress through each stage in a loop"). A write to WDTFEED
    ///   (@0x60) reloads stage 0 — "if a watchdog timer is fed by software, the
    ///   timer will return to stage 0" — including while the block is
    ///   write-protected.
    /// * **Stage actions.** WDTCONFIG0's STG0..3 fields encode 0 = off,
    ///   1 = interrupt, 2 = reset CPU, 3 = reset system. For EVERY stage whose
    ///   action is interrupt, its expiry sets INT_RAW_TIMERS.WDT_INT_RAW
    ///   (bit 1); INT_CLR_TIMERS is W1C and is the only thing that clears the
    ///   latch. Off stages consume their hold and advance the chain without an
    ///   action. The reset actions advance the chain and latch nothing — see
    ///   the exclusion below.
    /// * **A walk-driven clock.** Unlike the scheduler-driven GP timers, an
    ///   MWDT must expire with simulated time even when firmware only READS its
    ///   status (a poll loop issues no MMIO writes, and the scheduler path only
    ///   syncs on writes). So `uses_scheduler()` is false while `mwdt` is set:
    ///   the legacy per-tick walk drives the countdown and `sync_to` is a no-op.
    ///   One model tick is one peripheral tick interval (512 cycles under the
    ///   default CLI config); the hold count is therefore in WALK TICKS, not at
    ///   the silicon 12.5 ns × prescaler rate. WDTCONFIG1's prescaler field is
    ///   stored but does not scale the countdown, and a zero hold consumes one
    ///   walk tick per stage transition so the chain always makes progress.
    ///
    /// What this deliberately does NOT claim (and a fixture must not check):
    /// the CPU / system reset actions (STG = 2/3 advances the chain but no
    /// reset is ever performed — no safe reset-request path exists on the bus
    /// for a peripheral), the silicon timeout rate, and interrupt-matrix
    /// delivery of the latch (INT_RAW/INT_ST only).
    pub fn with_mwdt(mut self) -> Self {
        self.mwdt = true;
        // Silicon reset values for the whole config/hold surface: WDTWPROTECT
        // resets to its key (i.e. UNLOCKED), and WDTCONFIG0..5 to their SVD
        // seeds. Without the latter, an unconfigured stage 1..3 would have a
        // zero hold and the chain would wrap back to stage 0 instantly.
        for &(off, reset) in &MWDT_RESETS {
            self.regs.insert(off, reset);
        }
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

    /// Live LACT counter (debug helper / test introspection).
    pub fn counter_lact(&self) -> u64 {
        self.counter_lact
    }

    /// Programmed LACT prescaler, or `None` when LACT is disabled or the
    /// divider is still zero. Silicon divides by 65536 when the field reads
    /// zero; we treat it as "not yet programmed" and hold the counter still,
    /// because a firmware that has not configured LACT has no business
    /// seeing it advance and a stopped clock is easier to diagnose than one
    /// running 65536× slow.
    fn lact_divider(&self) -> Option<u32> {
        if !self.lact_enabled {
            return None;
        }
        let cfg = self.word(LACT_CONFIG);
        if cfg & LACT_EN_BIT == 0 {
            return None;
        }
        let mask = (1u32 << LACT_DIVIDER_BITS) - 1;
        match (cfg >> LACT_DIVIDER_SHIFT) & mask {
            0 => None,
            d => Some(d),
        }
    }

    /// Advance LACT by `cpu_cycles` CPU cycles (the unit both `tick()` and
    /// `sync_to` deliver). One CPU cycle is `APB_MHZ / LACT_CPU_MHZ` APB
    /// cycles; the fractional APB remainder is carried in `lact_rem`, scaled
    /// by `LACT_CPU_MHZ`, so no time is lost across calls.
    fn advance_lact(&mut self, cpu_cycles: u64) {
        let Some(div) = self.lact_divider() else {
            return;
        };
        // Units: APB cycles x LACT_CPU_MHZ. Saturating rather than wrapping:
        // a run long enough to overflow this has bigger problems.
        let scaled = cpu_cycles
            .saturating_mul(APB_MHZ as u64)
            .saturating_add(self.lact_rem as u64);
        let per_tick = div as u64 * LACT_CPU_MHZ as u64;
        self.counter_lact = self.counter_lact.wrapping_add(scaled / per_tick);
        self.lact_rem = (scaled % per_tick) as u32;
    }

    /// Latch the live LACT counter into LACT_LO/LACT_HI. Real silicon
    /// requires the LACT_UPDATE strobe before the pair is coherent, and
    /// `esp_timer_impl_get_counter_reg` spins until LO *changes* after that
    /// strobe — so the latch must genuinely move once the counter has.
    fn latch_lact(&mut self) {
        self.regs.insert(LACT_LO, self.counter_lact as u32);
        self.regs.insert(LACT_HI, (self.counter_lact >> 32) as u32);
    }

    fn preload_lact(&mut self) {
        let lo = self.word(LACT_LOADLO) as u64;
        let hi = self.word(LACT_LOADHI) as u64;
        self.counter_lact = (hi << 32) | lo;
        self.lact_rem = 0;
        self.latch_lact();
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

    // ── MWDT (C3/C6) ────────────────────────────────────────────────────────

    /// True when a write to `word_off` must be dropped by the MWDT write lock:
    /// the C3/C6 config surface while WDTWPROTECT holds anything but the key.
    fn mwdt_config_write_blocked(&self, word_off: u64) -> bool {
        self.mwdt
            && matches!(
                word_off,
                WDT_CONFIG0
                    | WDT_CONFIG1
                    | WDT_CONFIG2
                    | WDT_CONFIG3
                    | WDT_CONFIG4
                    | WDT_CONFIG5_C3
            )
            && self.word(WDT_WPROTECT_C3) != WDT_WKEY_VALUE
    }

    /// MWDT stage action field for `stage` (0..3). 0 = off, 1 = interrupt,
    /// 2 = reset CPU, 3 = reset system.
    fn mwdt_stage_action(&self, stage: u8) -> u32 {
        (self.word(WDT_CONFIG0) >> WDT_STG_SHIFTS[stage as usize]) & WDT_STG_MASK
    }

    /// Arm the stage chain at stage 0 with its programmed hold (WDTCONFIG2).
    /// Called on the WDTCONFIG0 0→1 EN edge and on every feed — the two events
    /// silicon defines as "return to stage 0 and reset the counter".
    fn mwdt_reload(&mut self) {
        self.wdt_stage = 0;
        self.wdt_countdown = u64::from(self.word(WDT_STG_HOLD_REGS[0]));
        self.wdt_armed = true;
    }

    /// WDTCONFIG0 write side effect: on the 0→1 EN edge, arm the chain at
    /// stage 0; on the 1→0 edge, stop it (a disabled WDT holds its counter
    /// still). A pending INT latch is NOT touched — only INT_CLR clears it.
    fn on_mwdt_config0_write(&mut self) {
        let enabled = self.word(WDT_CONFIG0) & WDT_EN_BIT != 0;
        if enabled && !self.wdt_enabled {
            self.mwdt_reload();
        }
        if !enabled {
            self.wdt_countdown = 0;
            self.wdt_armed = false;
        }
        self.wdt_enabled = enabled;
    }

    /// WDTFEED write: a feed restarts the chain at stage 0. Silicon accepts
    /// any value ("Write any value to feed the MWDT") and the feed is not
    /// blocked by the write lock. A feed while disabled does nothing.
    fn mwdt_feed(&mut self) {
        if self.wdt_enabled {
            self.mwdt_reload();
        }
    }

    /// Expire the active stage: take its configured action (interrupt-action
    /// stages latch `INT_RAW_TIMERS.WDT_INT_RAW`; off/reset stages take no
    /// modelled action), advance to the next stage in the 0→3 loop, and load
    /// that stage's hold.
    fn mwdt_expire_stage(&mut self) {
        if self.mwdt_stage_action(self.wdt_stage) == WDT_STAGE_ACTION_INT {
            self.wdt_pending = true;
        }
        self.wdt_stage = (self.wdt_stage + 1) % WDT_NUM_STAGES;
        self.wdt_countdown = u64::from(self.word(WDT_STG_HOLD_REGS[self.wdt_stage as usize]));
    }

    /// Advance the stage chain by `ticks` model ticks, expiring stages as
    /// their holds elapse. Each active-stage hold is counted in ticks; a
    /// zero-length stage consumes one tick per transition so an all-zero hold
    /// surface cannot spin the model. The chain loops 0→3→0 until disabled.
    fn advance_mwdt(&mut self, ticks: u64) {
        if !self.mwdt || !self.wdt_armed {
            return;
        }
        let mut remaining = ticks;
        while remaining > 0 {
            if self.wdt_countdown > remaining {
                // The active stage outlives this advance — just count down.
                self.wdt_countdown -= remaining;
                return;
            }
            // The stage expires within this advance. The tick that reaches a
            // non-zero hold's zero is the expiry tick; a zero hold needs its
            // own tick so `off`/unset stages still advance one per tick.
            if self.wdt_countdown > 0 {
                remaining -= self.wdt_countdown;
            } else {
                remaining -= 1;
            }
            self.wdt_countdown = 0;
            self.mwdt_expire_stage();
        }
    }

    /// INT_RAW_TIMERS value this model produces (T0 raw is not modelled → 0).
    fn mwdt_int_raw(&self) -> u32 {
        if self.wdt_pending {
            WDT_INT_RAW_BIT
        } else {
            0
        }
    }

    /// INT_ST_TIMERS = INT_RAW_TIMERS & INT_ENA_TIMERS.
    fn mwdt_int_st(&self) -> u32 {
        self.mwdt_int_raw() & self.word(WDT_INT_ENA_C3)
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
            LACT_UPDATE if self.lact_enabled => self.latch_lact(),
            LACT_LOAD if self.lact_enabled => self.preload_lact(),
            // ── C3/C6 MWDT (only under `with_mwdt`; offsets above) ──────────
            WDT_CONFIG0 if self.mwdt => self.on_mwdt_config0_write(),
            WDT_FEED_C3 if self.mwdt => self.mwdt_feed(),
            WDT_INT_CLR_C3 if self.mwdt => {
                // W1C: clear the sticky WDT latch only.
                if self.word(WDT_INT_CLR_C3) & WDT_INT_RAW_BIT != 0 {
                    self.wdt_pending = false;
                }
            }
            // ── ESP32-classic feed/protect pair (documented no-op) ──────────
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
            _ => {
                crate::census_reg!("esp32.timg:Timg", word_off, "write");
            }
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
    /// Side-effect-free probe, so `inspect` can show real values instead of
    /// zeros.
    ///
    /// Delegating to `read` is safe here for one specific reason, and it is
    /// worth stating because it is NOT safe in general: `Peripheral::read`
    /// takes `&self`, so the only way a read could disturb the model is
    /// through interior mutability, and this model has none -- no `Cell`,
    /// `RefCell`, `Atomic*` or `Mutex` anywhere. Every read is a pure function
    /// of state the debugger is allowed to look at.
    ///
    /// Do NOT copy this into a peripheral that does have interior mutability
    /// without checking its read path first. A read-to-clear status register
    /// would be cleared by the act of displaying it, and the firmware under
    /// test would then miss the event -- the exact failure `inspect`'s
    /// peek-only contract exists to prevent.
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

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
            LACT_LO if self.lact_enabled => self
                .regs
                .get(&LACT_LO)
                .copied()
                .unwrap_or(self.counter_lact as u32),
            LACT_HI if self.lact_enabled => self
                .regs
                .get(&LACT_HI)
                .copied()
                .unwrap_or((self.counter_lact >> 32) as u32),
            // C3/C6 MWDT status is computed, not stored: bit1 latches on
            // stage-0 expiry and only INT_CLR_TIMERS clears it.
            WDT_INT_RAW_C3 if self.mwdt => self.mwdt_int_raw(),
            WDT_INT_ST_C3 if self.mwdt => self.mwdt_int_st(),
            _ => self.word(word_off),
        };
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        // C3/C6 MWDT write lock: config writes are dropped while WDTWPROTECT
        // holds a value other than the key. Gated on `mwdt`, so the classic
        // shape and the C3 chip yaml (which never opts in) are unchanged.
        if self.mwdt_config_write_blocked(word_off) {
            return Ok(());
        }
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
            LACT_LO if self.lact_enabled => self
                .regs
                .get(&LACT_LO)
                .copied()
                .unwrap_or(self.counter_lact as u32),
            LACT_HI if self.lact_enabled => self
                .regs
                .get(&LACT_HI)
                .copied()
                .unwrap_or((self.counter_lact >> 32) as u32),
            WDT_INT_RAW_C3 if self.mwdt => self.mwdt_int_raw(),
            WDT_INT_ST_C3 if self.mwdt => self.mwdt_int_st(),
            _ => self.word(word_off),
        };
        Ok(word)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let word_off = offset & !3;
        if self.mwdt_config_write_blocked(word_off) {
            return Ok(());
        }
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
        self.advance_lact(1);
        self.advance_mwdt(1);
        // No interrupt firing this round — see module docs. Routing is
        // a separate task.
        PeripheralTickResult::default()
    }

    /// Phase 2B.2 (issue #192): TIMG is the first peripheral to migrate to the
    /// event scheduler. With the `event-scheduler` feature on, the bus stops
    /// calling `tick()` every cycle and instead calls `sync_to` lazily on MMIO
    /// access. (No-op effect when the feature is off — the bus ignores this.)
    ///
    /// The C3/C6 MWDT variant (`with_mwdt`) opts back out: its countdown must
    /// advance with simulated time even for a read-only status poll, and the
    /// scheduler only syncs on MMIO writes. See `with_mwdt`.
    fn uses_scheduler(&self) -> bool {
        !self.mwdt
    }

    /// Advance the enabled counters to peripheral-tick `tick_now`. Equivalent
    /// to having called `tick()` once per intervening tick while enabled, but
    /// computed in O(1) at access time instead of once per cycle. Disabled
    /// timers don't accrue: `anchor_tick` is always moved to `tick_now`, so a
    /// span spent disabled is excluded once the timer is re-enabled. `tick_now`
    /// is monotonic (driven by `Machine::total_cycles`), so the delta is never
    /// negative; `saturating_sub` guards the degenerate equal/rewind case.
    fn sync_to(&mut self, tick_now: u64) {
        // The MWDT variant is walk-driven (see `uses_scheduler`); accepting a
        // sync there would double-count against the walk's `tick()`.
        if self.mwdt {
            return;
        }
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
        self.advance_lact(delta);
        self.advance_mwdt(delta);
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

    /// LACT must be INERT on a TIMG that was not told it has one.
    ///
    /// The ESP32-C3 reuses this model with entirely different registers in
    /// the LACT window: 0x78/0x7C are INT_ST_TIMERS/INT_CLR_TIMERS and 0x80
    /// is RTCCALICFG2. Before `with_lact()` existed, a C3 firmware writing
    /// RTCCALICFG2 would latch the counter straight over its own interrupt
    /// status and clear registers, and a read of INT_ST_TIMERS would return
    /// a timer count. Both are silent corruption, so this pins the default.
    #[test]
    fn lact_offsets_are_plain_storage_without_with_lact() {
        let mut t = Timg::new(0x6001_F000); // C3 TIMG0 base — no with_lact()
                                            // Enable-looking bits in what the C3 calls INT_ENA_TIMERS.
        write_u32(&mut t, 0x70, 0xFFFF_FFFF);
        // What the C3 calls INT_ST_TIMERS / INT_CLR_TIMERS.
        write_u32(&mut t, 0x78, 0x1111_1111);
        write_u32(&mut t, 0x7C, 0x2222_2222);
        for _ in 0..1000 {
            t.tick();
        }
        // A write to RTCCALICFG2 must NOT latch a counter over 0x78/0x7C.
        write_u32(&mut t, 0x80, 1);
        assert_eq!(
            read_u32(&t, 0x78),
            0x1111_1111,
            "INT_ST_TIMERS was clobbered by a LACT latch"
        );
        assert_eq!(
            read_u32(&t, 0x7C),
            0x2222_2222,
            "INT_CLR_TIMERS was clobbered by a LACT latch"
        );
        assert_eq!(
            t.counter_lact(),
            0,
            "LACT advanced on a chip that has no LACT"
        );
    }

    /// ...and LIVE when it is declared, so the guard above is not vacuous.
    #[test]
    fn lact_counts_and_latches_when_declared() {
        let mut t = Timg::new(0x3FF5_F000).with_lact();
        // LACT_EN | divider 40 at bits [28:13] — what esp_timer programs for
        // the 2 MHz tick esp_timer_get_time() halves into microseconds.
        write_u32(&mut t, 0x70, (1 << 31) | (40 << 13));
        // tick() is one CPU cycle: 1000 us at 240 MHz is 240_000 cycles.
        for _ in 0..240_000 {
            t.tick();
        }
        // 1000 us at APB 80 MHz / 40 = 2 ticks per us.
        assert_eq!(t.counter_lact(), 2000);
        write_u32(&mut t, 0x80, 1); // LACT_UPDATE latches LO/HI
        assert_eq!(read_u32(&t, 0x78), 2000);
        assert_eq!(read_u32(&t, 0x7C), 0);
    }

    /// esp_timer must run at wall rate against the CPU clock: one simulated
    /// second (240 M CPU cycles, scheduler `sync_to`) is exactly 1 s of LACT
    /// time (2 M ticks at the 2 MHz esp_timer programs), including when the
    /// span arrives in odd-sized pieces that do not divide the APB ratio.
    #[test]
    fn lact_sync_to_tracks_cpu_clock_not_cycles_as_microseconds() {
        let mut t = Timg::new(0x3FF5_F000).with_lact();
        write_u32(&mut t, 0x70, (1 << 31) | (40 << 13));
        let mut now = 0u64;
        for step in [1u64, 7, 239, 1_000, 123_457].iter().cycle() {
            if now >= 240_000_000 {
                break;
            }
            now = (now + step).min(240_000_000);
            t.sync_to(now);
        }
        assert_eq!(
            t.counter_lact(),
            2_000_000,
            "1 s of CPU cycles = 1 s of esp_timer"
        );
    }

    // ── C3/C6 MWDT (with_mwdt) ──────────────────────────────────────────────

    /// Encode a WDTCONFIG0 value: EN plus a stage-0 action of `action`.
    fn wdt_cfg0(action: u32) -> u32 {
        WDT_EN_BIT | (action << WDT_STG_SHIFTS[0])
    }

    /// Encode a WDTCONFIG0 value with one action per stage (index = stage).
    fn wdt_cfg0_stages(actions: [u32; WDT_NUM_STAGES as usize]) -> u32 {
        let mut v = WDT_EN_BIT;
        for (stage, action) in actions.iter().enumerate() {
            v |= (action & WDT_STG_MASK) << WDT_STG_SHIFTS[stage];
        }
        v
    }

    /// The MWDT is unlocked at reset: WDTWPROTECT reads the key and a config
    /// write sticks immediately.
    #[test]
    fn mwdt_resets_unlocked_and_stores_config() {
        let mut t = Timg::new(0x6000_8000).with_mwdt();
        assert_eq!(
            read_u32(&t, WDT_WPROTECT_C3),
            WDT_WKEY_VALUE,
            "WDTWPROTECT reset value is the unlock key on silicon"
        );
        write_u32(&mut t, WDT_CONFIG1, 0x0001_2345);
        assert_eq!(read_u32(&t, WDT_CONFIG1), 0x0001_2345);
        write_u32(&mut t, WDT_CONFIG2, 0x0000_2710);
        assert_eq!(read_u32(&t, WDT_CONFIG2), 0x0000_2710);
        write_u32(&mut t, WDT_CONFIG5_C3, 0x0000_0064);
        assert_eq!(read_u32(&t, WDT_CONFIG5_C3), 0x0000_0064);
    }

    /// While WDTWPROTECT holds anything but the key, WDTCONFIG0..5 writes are
    /// dropped; WDTFEED and WDTWPROTECT stay writable.
    #[test]
    fn mwdt_write_lock_gates_config_but_not_feed() {
        let mut t = Timg::new(0x6000_8000).with_mwdt();
        write_u32(&mut t, WDT_CONFIG2, 10);
        write_u32(&mut t, WDT_WPROTECT_C3, 0); // lock
        write_u32(&mut t, WDT_CONFIG0, wdt_cfg0(1));
        assert_eq!(
            read_u32(&t, WDT_CONFIG0) & WDT_EN_BIT,
            0,
            "locked WDTCONFIG0 write must be dropped"
        );
        write_u32(&mut t, WDT_CONFIG2, 99);
        assert_eq!(read_u32(&t, WDT_CONFIG2), 10, "locked hold write dropped");

        write_u32(&mut t, WDT_WPROTECT_C3, WDT_WKEY_VALUE); // unlock
        write_u32(&mut t, WDT_CONFIG0, wdt_cfg0(1));
        assert_ne!(read_u32(&t, WDT_CONFIG0) & WDT_EN_BIT, 0);
        write_u32(&mut t, WDT_WPROTECT_C3, 0); // lock again
        write_u32(&mut t, WDT_FEED_C3, 0x1234_5678); // feed is not gated
        for _ in 0..9 {
            t.tick();
        }
        assert_eq!(read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT, 0);
        t.tick();
        assert_ne!(
            read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT,
            0,
            "feed must have re-armed the countdown while locked"
        );
    }

    /// Expiry latches INT_RAW_TIMERS.WDT_INT_RAW; INT_CLR is W1C; feed re-arms
    /// a full hold; expiry never auto-reloads.
    #[test]
    fn mwdt_expiry_latches_and_feed_rearms() {
        let mut t = Timg::new(0x6000_8000).with_mwdt();
        write_u32(&mut t, WDT_CONFIG2, 10); // STG0_HOLD
        write_u32(&mut t, WDT_CONFIG0, wdt_cfg0(1)); // enable, stage0=interrupt

        for _ in 0..9 {
            t.tick();
        }
        assert_eq!(
            read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT,
            0,
            "must not latch before the hold count elapses"
        );
        t.tick(); // 10th tick → expiry
        assert_ne!(read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT, 0);
        assert_eq!(
            read_u32(&t, WDT_INT_ST_C3) & WDT_INT_RAW_BIT,
            0,
            "INT_ST stays clear while INT_ENA_TIMERS.WDT_INT_ENA is clear"
        );

        // Stage 0 is one-shot until the chain re-arms it: after this expiry
        // the watchdog is in stage 1, whose hold is its (long) SVD reset, so
        // no second latch arrives within this bounded window and none comes
        // from stage 0 until a feed.
        write_u32(&mut t, WDT_INT_CLR_C3, WDT_INT_RAW_BIT);
        assert_eq!(read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT, 0);
        for _ in 0..100 {
            t.tick();
        }
        assert_eq!(
            read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT,
            0,
            "the armed stage must not re-latch within its hold"
        );

        // Feed re-arms the full hold count.
        write_u32(&mut t, WDT_FEED_C3, WDT_WKEY_VALUE);
        for _ in 0..9 {
            t.tick();
        }
        assert_eq!(read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT, 0);
        t.tick();
        assert_ne!(
            read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT,
            0,
            "feed must re-arm the countdown from WDTCONFIG2"
        );
    }

    /// INT_ENA_TIMERS gates INT_ST_TIMERS, not the raw latch.
    #[test]
    fn mwdt_int_st_follows_enable() {
        let mut t = Timg::new(0x6000_8000).with_mwdt();
        write_u32(&mut t, WDT_CONFIG2, 1);
        write_u32(&mut t, WDT_CONFIG0, wdt_cfg0(1));
        t.tick();
        assert_ne!(read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT, 0);
        assert_eq!(read_u32(&t, WDT_INT_ST_C3) & WDT_INT_RAW_BIT, 0);
        write_u32(&mut t, WDT_INT_ENA_C3, WDT_INT_RAW_BIT);
        assert_ne!(read_u32(&t, WDT_INT_ST_C3) & WDT_INT_RAW_BIT, 0);
    }

    /// Stage action "off" (0) and the reset actions (2/3) never latch the
    /// interrupt; the model performs no reset.
    #[test]
    fn mwdt_only_latches_the_interrupt_action() {
        for action in [0u32, 2, 3] {
            let mut t = Timg::new(0x6000_8000).with_mwdt();
            write_u32(&mut t, WDT_CONFIG2, 1);
            write_u32(&mut t, WDT_CONFIG0, wdt_cfg0(action));
            for _ in 0..100 {
                t.tick();
            }
            assert_eq!(
                read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT,
                0,
                "stage action {action} must not latch the WDT interrupt"
            );
        }
    }

    /// A disabled MWDT does not count even when a hold value is programmed,
    /// and disabling stops an armed countdown.
    #[test]
    fn mwdt_disabled_does_not_count() {
        let mut t = Timg::new(0x6000_8000).with_mwdt();
        write_u32(&mut t, WDT_CONFIG2, 1); // hold programmed, EN still clear
        for _ in 0..100 {
            t.tick();
        }
        assert_eq!(read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT, 0);

        write_u32(&mut t, WDT_CONFIG0, wdt_cfg0(1));
        t.tick();
        t.tick();
        assert_ne!(read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT, 0);
        write_u32(&mut t, WDT_INT_CLR_C3, WDT_INT_RAW_BIT);
        write_u32(&mut t, WDT_CONFIG0, wdt_cfg0(1) & !WDT_EN_BIT); // disable
        for _ in 0..1000 {
            t.tick();
        }
        assert_eq!(
            read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT,
            0,
            "disabled MWDT must hold its counter still"
        );
    }

    /// The MWDT variant is WALK-driven: `uses_scheduler()` is false and a
    /// stray `sync_to` must not double-count against the walk.
    #[test]
    fn mwdt_is_walk_driven() {
        let mut t = Timg::new(0x6000_8000).with_mwdt();
        assert!(
            !t.uses_scheduler(),
            "a read-only status poll must still see time pass"
        );
        write_u32(&mut t, WDT_CONFIG2, 1000);
        write_u32(&mut t, WDT_CONFIG0, wdt_cfg0(1));
        t.sync_to(999);
        assert_eq!(
            read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT,
            0,
            "sync_to is inert on the walk-driven variant"
        );
        for _ in 0..1000 {
            t.tick();
        }
        assert_ne!(read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT, 0);
    }

    // ── C3/C6 MWDT stage chain (stages 1..3) ────────────────────────────────

    /// `with_mwdt` seeds the SVD reset values for the whole config/hold
    /// surface, not zeros — an unconfigured stage 1..3 holds silicon's long
    /// default, and WDTWPROTECT its unlock key.
    #[test]
    fn mwdt_seeds_the_svd_reset_values() {
        let t = Timg::new(0x6000_8000).with_mwdt();
        assert_eq!(read_u32(&t, WDT_CONFIG0), 0x0004_C000);
        assert_eq!(read_u32(&t, WDT_CONFIG1), 0x0001_0000);
        assert_eq!(read_u32(&t, WDT_CONFIG2), 0x018C_BA80);
        assert_eq!(read_u32(&t, WDT_CONFIG3), 0x07FF_FFFF);
        assert_eq!(read_u32(&t, WDT_CONFIG4), 0x000F_FFFF);
        assert_eq!(read_u32(&t, WDT_CONFIG5_C3), 0x000F_FFFF);
        assert_eq!(read_u32(&t, WDT_WPROTECT_C3), WDT_WKEY_VALUE);
    }

    /// The chain walks 0→1→2→3→0: every interrupt-action stage latches at the
    /// cumulative sum of the holds, in order, and the fourth expiry wraps back
    /// to stage 0 (its hold starts the next round).
    #[test]
    fn mwdt_stage_chain_latches_every_interrupt_stage_in_order() {
        let mut t = Timg::new(0x6000_8000).with_mwdt();
        write_u32(&mut t, WDT_CONFIG2, 10); // STG0_HOLD
        write_u32(&mut t, WDT_CONFIG3, 20); // STG1_HOLD
        write_u32(&mut t, WDT_CONFIG4, 30); // STG2_HOLD
        write_u32(&mut t, WDT_CONFIG5_C3, 40); // STG3_HOLD
        write_u32(&mut t, WDT_CONFIG0, wdt_cfg0_stages([1, 1, 1, 1]));

        let mut tick = 0u32;
        // Latch times: 10 (stage 0), 30 (stage 1), 60 (stage 2), 100 (stage 3),
        // then 110 — stage 0 again after the wrap.
        for &want in &[10u32, 30, 60, 100, 110] {
            while tick < want - 1 {
                t.tick();
                tick += 1;
                assert_eq!(
                    read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT,
                    0,
                    "latch at tick {tick}; stage chain must latch first at {want}"
                );
            }
            t.tick();
            tick += 1;
            assert_ne!(
                read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT,
                0,
                "stage expiry at tick {want} must latch WDT_INT_RAW"
            );
            write_u32(&mut t, WDT_INT_CLR_C3, WDT_INT_RAW_BIT);
        }
    }

    /// Every stage consumes ITS OWN hold register: an off stage 0 still burns
    /// its hold, and the interrupt-action stage 1 latches only after
    /// STG0_HOLD + STG1_HOLD — not after either hold alone.
    #[test]
    fn mwdt_stage_holds_and_off_stages_advance_the_chain() {
        let mut t = Timg::new(0x6000_8000).with_mwdt();
        write_u32(&mut t, WDT_CONFIG2, 5); // STG0_HOLD, stage 0 action = off
        write_u32(&mut t, WDT_CONFIG3, 7); // STG1_HOLD, stage 1 action = INT
        write_u32(&mut t, WDT_CONFIG4, 11); // STG2_HOLD, stage 2 action = off
        write_u32(&mut t, WDT_CONFIG5_C3, 13); // STG3_HOLD, stage 3 action = INT
        write_u32(&mut t, WDT_CONFIG0, wdt_cfg0_stages([0, 1, 0, 1]));

        let mut tick = 0u32;
        for &want in &[12u32, 36] {
            while tick < want - 1 {
                t.tick();
                tick += 1;
                assert_eq!(
                    read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT,
                    0,
                    "latch at tick {tick}; want {want} (holds must accumulate)"
                );
            }
            t.tick();
            tick += 1;
            assert_ne!(read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT, 0);
            write_u32(&mut t, WDT_INT_CLR_C3, WDT_INT_RAW_BIT);
        }
    }

    /// The reset actions (2 = reset CPU, 3 = reset system) do not latch and do
    /// not stop the chain: the first interrupt-action stage latches only at
    /// the sum of the holds ahead of it, and the loop keeps advancing
    /// afterwards. No reset is performed (no safe bus path exists).
    #[test]
    fn mwdt_reset_stage_actions_advance_without_latching() {
        let mut t = Timg::new(0x6000_8000).with_mwdt();
        write_u32(&mut t, WDT_CONFIG2, 5);
        write_u32(&mut t, WDT_CONFIG3, 7);
        write_u32(&mut t, WDT_CONFIG4, 11);
        write_u32(&mut t, WDT_CONFIG5_C3, 13);
        // STG0 = reset CPU, STG1 = reset system, STG2 = INT, STG3 = off.
        write_u32(&mut t, WDT_CONFIG0, wdt_cfg0_stages([2, 3, 1, 0]));

        let mut tick = 0u32;
        // First latch: after 5 + 7 + 11 = 23 ticks (stage 2).
        while tick < 23 - 1 {
            t.tick();
            tick += 1;
            assert_eq!(
                read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT,
                0,
                "reset-action stages must not latch (tick {tick})"
            );
        }
        t.tick();
        tick += 1;
        assert_ne!(read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT, 0);
        write_u32(&mut t, WDT_INT_CLR_C3, WDT_INT_RAW_BIT);

        // The chain kept running through the off stage 3 (13), the reset-action
        // stages 0/1 (5 + 7) and back to the interrupt stage 2 (11): the next
        // latch is at 23 + 13 + 5 + 7 + 11 = 59.
        while tick < 59 - 1 {
            t.tick();
            tick += 1;
            assert_eq!(
                read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT,
                0,
                "second latch must wait for the wrapped stage-2 expiry (tick {tick})"
            );
        }
        t.tick();
        assert_ne!(read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT, 0);
    }

    /// A feed restarts at stage 0: mid-way through stage 1, a feed must give
    /// stage 0's hold again — not stage 1's remaining or stage 2's hold.
    #[test]
    fn mwdt_feed_returns_to_stage_zero() {
        let mut t = Timg::new(0x6000_8000).with_mwdt();
        write_u32(&mut t, WDT_CONFIG2, 10);
        write_u32(&mut t, WDT_CONFIG3, 100);
        write_u32(&mut t, WDT_CONFIG4, 100);
        write_u32(&mut t, WDT_CONFIG5_C3, 100);
        write_u32(&mut t, WDT_CONFIG0, wdt_cfg0_stages([1, 1, 1, 1]));

        for _ in 0..10 {
            t.tick(); // stage 0 expires, stage 1 now has 100 ticks
        }
        assert_ne!(read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT, 0);
        write_u32(&mut t, WDT_INT_CLR_C3, WDT_INT_RAW_BIT);

        for _ in 0..5 {
            t.tick(); // 5 ticks of stage 1 have elapsed
        }
        write_u32(&mut t, WDT_FEED_C3, WDT_WKEY_VALUE);

        // Stage 0's hold (10) — not stage 1's remaining (95) or stage 2 (100).
        for _ in 0..9 {
            t.tick();
            assert_eq!(read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT, 0);
        }
        t.tick();
        assert_ne!(
            read_u32(&t, WDT_INT_RAW_C3) & WDT_INT_RAW_BIT,
            0,
            "feed must re-arm at stage 0 with STG0_HOLD"
        );
    }

    /// Without `with_mwdt` nothing about the classic shape changes: the C3
    /// chip yaml never opts in, so this pins the regression boundary.
    #[test]
    fn classic_wdt_path_unchanged_without_with_mwdt() {
        let mut t = Timg::new(0x6001_F000);
        // Classic feed/protect pair — round-trip storage, no behaviour.
        write_u32(&mut t, WDT_FEED, 0x1234_5678);
        assert_eq!(read_u32(&t, WDT_FEED), 0x1234_5678);
        write_u32(&mut t, WDT_WPROTECT, 0xDEAD_BEEF);
        assert_eq!(read_u32(&t, WDT_WPROTECT), 0xDEAD_BEEF);
        // The C3 offsets are plain storage: a write to 0x60 (C3 feed) lands in
        // `regs` and does not create watchdog state; 0x74 stays a plain bank.
        write_u32(&mut t, WDT_FEED_C3, 0xAAAA_AAAA);
        assert_eq!(read_u32(&t, WDT_FEED_C3), 0xAAAA_AAAA);
        write_u32(&mut t, WDT_INT_RAW_C3, 0xFFFF_FFFF);
        for _ in 0..10_000 {
            t.tick();
        }
        assert_eq!(
            read_u32(&t, WDT_INT_RAW_C3),
            0xFFFF_FFFF,
            "no WDT model may latch bits onto an opted-out TIMG"
        );
    }
}
