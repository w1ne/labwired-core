// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RTC Controller (RTC_CNTL) peripheral for ESP32-classic.
//!
//! Per ESP32 TRM v5.0 §13. The RTC_CNTL block sits at base `0x3FF4_8000`
//! and carries the reset-cause registers, RTC slow-clock counter,
//! deep-sleep wake state, analog config registers, and the four
//! general-purpose retention scratch words (`STORE0..3`) that survive
//! deep sleep.
//!
//! ## Why this peripheral exists
//!
//! ESP32 BROM and ESP-IDF startup read this block heavily during init:
//!
//!   * `rtc_get_reset_reason(cpu)` decodes the per-core reset cause from
//!     `RESET_STATE` (offset 0x34). We seed POWERON_RESET (=1) for both
//!     cores so a first-boot probe gets a coherent answer.
//!   * `rtc_time_get()` (and friends) snapshots the 48-bit RTC slow-counter
//!     into `TIME0` (low 32) / `TIME1` (high 16) by writing the TIME_UPDATE
//!     trigger at offset 0x0C. We expose the live counter on every read of
//!     TIME0/TIME1 so callers see a monotonic value.
//!   * Arduino-ESP32's frequency detect reads `RTC_APB_FREQ_REG` at
//!     offset 0xB0; the 40 MHz encoding is `0x0050_0050` (low and high
//!     halves equal, shifted left by 1). Pre-seeded at construction.
//!   * `STORE0..3` (offsets 0x4C..0x58) are retention RAM scratch words.
//!     Plain read/write round-trip — no side effects modeled.
//!
//! Analog config registers (`ANA_CONF` at 0x30, `DIG_PWC` at 0x8C,
//! `BIAS_CONF` at 0x80) and other power-domain knobs read-as-zero
//! unless previously written, and accept any write.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;
use std::collections::HashMap;

// ── Register offsets (per ESP32 TRM v5.0 §13.5 and ESP-IDF
// `soc/esp32/include/soc/rtc_cntl_reg.h`) ─────────────────────────────────

/// RTC_CNTL_OPTIONS0_REG — bit 31 = sw_sys_rst trigger; bits[1:0] = sw_stall_*.
pub const RTC_CNTL_OPTIONS0_OFFSET: u64 = 0x00;
/// RTC_CNTL_TIME_UPDATE_REG — bit 31 = TIME_UPDATE (write 1 to snapshot),
/// bit 30 = TIME_VALID (RO, set by hardware once the snapshot has landed).
pub const RTC_CNTL_TIME_UPDATE_OFFSET: u64 = 0x0C;
/// RTC_CNTL_TIME_VALID — read-only ACK bit (bit 30). ESP-IDF's
/// `rtc_time_get` writes TIME_UPDATE (bit 31), then polls TIME_VALID
/// (bit 30) before reading TIME0/TIME1. We set this synchronously when
/// bit 31 is written so the poll exits immediately (real silicon takes
/// ~3 RTC slow-clock cycles; we model it as instant).
pub const RTC_CNTL_TIME_VALID_BIT: u32 = 1 << 30;
/// RTC_CNTL_TIME0_REG — low 32 bits of the 48-bit slow-counter snapshot.
pub const RTC_CNTL_TIME0_OFFSET: u64 = 0x10;
/// RTC_CNTL_TIME1_REG — high 16 bits (bits[15:0]) of slow-counter snapshot.
pub const RTC_CNTL_TIME1_OFFSET: u64 = 0x14;
/// RTC_CNTL_RESET_STATE_REG — bits[3:0]=PRO_CPU cause, bits[7:4]=APP_CPU cause.
pub const RTC_CNTL_RESET_STATE_OFFSET: u64 = 0x34;
/// RTC_CNTL_STORE0_REG..STORE3_REG — 4 retention scratch words.
pub const RTC_CNTL_STORE0_OFFSET: u64 = 0x4C;
pub const RTC_CNTL_STORE1_OFFSET: u64 = 0x50;
pub const RTC_CNTL_STORE2_OFFSET: u64 = 0x54;
pub const RTC_CNTL_STORE3_OFFSET: u64 = 0x58;
/// RTC_APB_FREQ_REG — APB frequency encoding probed at boot. Arduino-ESP32
/// expects `0x0050_0050` for a 40 MHz XTAL (both halves equal, low-then-high).
pub const RTC_APB_FREQ_OFFSET: u64 = 0xB0;
/// 40 MHz XTAL encoding the Arduino-ESP32 boot reads back.
pub const RTC_APB_FREQ_40MHZ: u32 = 0x0050_0050;

/// `OPTIONS0` bit 31 — `SW_SYS_RST`. Writing 1 triggers a whole-system
/// software reset on real silicon: the CPU restarts at the reset vector
/// (`0x4000_0400`) and execution does NOT return from the store. The BROM
/// relies on this in `_rtc_trigger_sw_system_reset` (called from
/// `_ResetHandler_efuse_check_patch`); falling through the store hits a
/// defensive `ILL.N` sentinel.
pub const RTC_CNTL_OPTIONS0_SW_SYS_RST_BIT: u32 = 1 << 31;

// Reset-cause field layout in `RESET_STATE`, matching the ESP32 BROM
// `rtc_get_reset_reason` decode (`extui a2, a2, 0, 6` for PRO_CPU and
// `extui a2, a2, 6, 6` for APP_CPU — verified against the real BROM ELF):
//   bits[5:0]   = PROCPU_RESET_CAUSE  (6-bit field)
//   bits[11:6]  = APPCPU_RESET_CAUSE  (6-bit field)
// The earlier 4-bit packing put the APP_CPU cause at bit 4, so POWERON on
// both cores read back as 0x11 (=17) through the BROM's 6-bit PRO_CPU
// extract — an out-of-range boot index that traps `main` at ets_main.c:404.
const RESET_CAUSE_PROCPU_SHIFT: u32 = 0;
const RESET_CAUSE_APPCPU_SHIFT: u32 = 6;
const RESET_CAUSE_MASK: u32 = 0x3F;

/// Reset-cause enum values (ESP-IDF `esp_rom_rtc_get_reset_reason`).
pub const POWERON_RESET: u32 = 1;
#[allow(dead_code)]
pub const SW_RESET: u32 = 3;

/// RTC Controller peripheral.
///
/// Word-granular sparse storage (HashMap) keeps the model compact —
/// most of the 0x200-byte address window is unused on a first-boot
/// probe. Side-effect logic lives in `write`; reads of TIME0/TIME1
/// pull from the live `slow_counter` so back-to-back queries observe
/// monotonic progress without firmware needing to trigger TIME_UPDATE.
#[derive(Debug)]
pub struct RtcCntl {
    /// Base MMIO address (for debugging / logs only — not used in
    /// offset math since the bus already dispatches by offset).
    base: u32,
    /// Backing word store. Indexed by 4-byte-aligned offset.
    regs: HashMap<u32, u32>,
    /// RTC slow-clock counter (48-bit on real silicon; we use u64 for
    /// arithmetic ease — only low 48 bits surface through TIME0/TIME1).
    slow_counter: u64,
    /// Latch set when firmware writes OPTIONS0 bit 31 (`SW_SYS_RST`). The
    /// machine step loop drains this between instructions and re-points
    /// the CPU at the reset vector, matching real-silicon semantics where
    /// the store never returns. Held in a `Cell` so it can be drained via
    /// the shared `&dyn Peripheral` reference the bus hands out.
    reset_requested: Cell<bool>,
    /// Phase 2B.3c (issue #192): peripheral-tick index of the last `sync_to`.
    /// In scheduler mode `slow_counter` advances lazily here instead of one
    /// per `tick()`. Firmware reads RTC time via the TIME_UPDATE strobe (an
    /// MMIO write), which syncs first — so the latched value is current.
    /// Unused in the legacy (flag-off) build.
    anchor_tick: u64,
}

impl Default for RtcCntl {
    fn default() -> Self {
        Self::new()
    }
}

impl RtcCntl {
    /// Canonical MMIO base address on ESP32-classic.
    pub const BASE: u32 = 0x3FF4_8000;

    /// Construct a freshly-powered RTC_CNTL block.
    ///
    /// Seeds:
    ///   * `RESET_STATE` = POWERON_RESET for both PRO_CPU and APP_CPU.
    ///   * `RTC_APB_FREQ_REG` = 40 MHz encoding (0x0050_0050).
    ///   * `slow_counter` = 0.
    pub fn new() -> Self {
        let mut regs = HashMap::new();
        let reset_state = (POWERON_RESET << RESET_CAUSE_PROCPU_SHIFT)
            | (POWERON_RESET << RESET_CAUSE_APPCPU_SHIFT);
        regs.insert(RTC_CNTL_RESET_STATE_OFFSET as u32, reset_state);
        regs.insert(RTC_APB_FREQ_OFFSET as u32, RTC_APB_FREQ_40MHZ);
        Self {
            base: Self::BASE,
            regs,
            slow_counter: 0,
            reset_requested: Cell::new(false),
            anchor_tick: 0,
        }
    }

    /// Returns true (and clears the latch) if firmware has triggered a
    /// software system reset by writing `OPTIONS0` bit 31 since the last
    /// drain. Called by the machine step loop between CPU instructions so
    /// the reset takes effect at a clean boundary — neither the CPU nor
    /// any peripheral sees a half-applied state.
    pub fn drain_reset_request(&self) -> bool {
        self.reset_requested.replace(false)
    }

    /// Peek without clearing. Batch planning only forces quantum-1 while a
    /// SW_SYS_RST is latched — unlike SCB, RTC presence alone must not kill
    /// dual-core WAITI primary batching for the whole run.
    pub fn reset_request_pending(&self) -> bool {
        self.reset_requested.get()
    }

    /// Base MMIO address (informational).
    pub fn base(&self) -> u32 {
        self.base
    }

    /// Set the per-core reset cause. Used by reset-injection tests.
    pub fn set_reset_cause(&mut self, procpu: u32, appcpu: u32) {
        let v = ((procpu & RESET_CAUSE_MASK) << RESET_CAUSE_PROCPU_SHIFT)
            | ((appcpu & RESET_CAUSE_MASK) << RESET_CAUSE_APPCPU_SHIFT);
        self.regs.insert(RTC_CNTL_RESET_STATE_OFFSET as u32, v);
    }

    /// Read the live slow-counter value (tests use this to verify tick()
    /// progression without going through MMIO).
    pub fn slow_counter(&self) -> u64 {
        self.slow_counter
    }

    fn read_word(&self, word_off: u32) -> u32 {
        // TIME0/TIME1 always reflect the live counter (every read snapshots,
        // matching the conservative interpretation — firmware that ignores
        // TIME_UPDATE still gets a monotonic value).
        match u64::from(word_off) {
            RTC_CNTL_TIME0_OFFSET => (self.slow_counter & 0xFFFF_FFFF) as u32,
            RTC_CNTL_TIME1_OFFSET => ((self.slow_counter >> 32) & 0xFFFF) as u32,
            _ => self.regs.get(&word_off).copied().unwrap_or(0),
        }
    }

    fn write_word(&mut self, word_off: u32, value: u32) {
        // TIME_UPDATE handshake: bit 31 set → snapshot the live counter
        // into TIME0/TIME1 entries, clear the trigger, and set the
        // TIME_VALID ACK bit so the firmware's `bbci a8, 30, loop`
        // poll exits. Real silicon takes ~3 RTC slow-clock cycles; we
        // model it as instant.
        if u64::from(word_off) == RTC_CNTL_TIME_UPDATE_OFFSET && (value & (1 << 31)) != 0 {
            let snap = self.slow_counter;
            self.regs
                .insert(RTC_CNTL_TIME0_OFFSET as u32, (snap & 0xFFFF_FFFF) as u32);
            self.regs
                .insert(RTC_CNTL_TIME1_OFFSET as u32, ((snap >> 32) & 0xFFFF) as u32);
            // Clear TIME_UPDATE trigger and set TIME_VALID (bit 30) ACK.
            self.regs
                .insert(word_off, (value & !(1u32 << 31)) | RTC_CNTL_TIME_VALID_BIT);
            return;
        }
        // SW_SYS_RST: latch the request and clear the trigger bit in
        // backing store before the machine drains the reset. The CPU is
        // re-pointed at the reset vector by `Machine::step` between
        // instructions, so any further bits in `value` (e.g. sw_stall_*
        // in bits[1:0]) are still observable until the reset lands.
        if u64::from(word_off) == RTC_CNTL_OPTIONS0_OFFSET
            && (value & RTC_CNTL_OPTIONS0_SW_SYS_RST_BIT) != 0
        {
            self.reset_requested.set(true);
            self.regs
                .insert(word_off, value & !RTC_CNTL_OPTIONS0_SW_SYS_RST_BIT);
            return;
        }
        self.regs.insert(word_off, value);
    }
}

impl Peripheral for RtcCntl {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = (offset & !3) as u32;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    // Word-granular read fast path: same `read_word` as the byte path,
    // looked up once instead of four times. Pure register lookup +
    // TIME0/TIME1 live-counter view; no side effects on read.
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word((offset & !3) as u32))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = (offset & !3) as u32;
        let byte_off = (offset & 3) * 8;
        // Read-modify-write on the existing word (or 0 if unwritten).
        let mut word = self.regs.get(&word_off).copied().unwrap_or(0);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Advance the slow-counter by 1 per tick. Real silicon ticks at
        // ~150 kHz; we don't simulate sub-cycle precision — firmware
        // that needs wall-clock timing uses Systimer instead.
        self.slow_counter = self.slow_counter.wrapping_add(1);
        PeripheralTickResult::default()
    }

    /// Phase 2B.3c (issue #192): RTC_CNTL is migrated to the event scheduler.
    /// Flag-on, the bus stops calling `tick()` every cycle and `sync_to`
    /// advances `slow_counter` lazily on MMIO access. Flag-off, `tick()` still
    /// drives it.
    fn uses_scheduler(&self) -> bool {
        true
    }

    /// Advance `slow_counter` to peripheral-tick `tick_now` — equivalent to
    /// having called `tick()` once per intervening tick. The counter is
    /// free-running (no enable bit), so it always accrues. Monotonic guard
    /// matches the TIMG migration.
    fn sync_to(&mut self, tick_now: u64) {
        if tick_now <= self.anchor_tick {
            return;
        }
        self.slow_counter = self.slow_counter.wrapping_add(tick_now - self.anchor_tick);
        self.anchor_tick = tick_now;
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            regs: Vec<(u32, u32)>,
            slow_counter: u64,
            reset_requested: bool,
        }
        let snap = Snap {
            regs: self.regs.iter().map(|(k, v)| (*k, *v)).collect(),
            slow_counter: self.slow_counter,
            reset_requested: self.reset_requested.get(),
        };
        bincode::serialize(&snap).expect("bincode serialize RtcCntl")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            regs: Vec<(u32, u32)>,
            slow_counter: u64,
            #[serde(default)]
            reset_requested: bool,
        }
        let snap: Snap = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("RtcCntl snapshot decode: {e}"))
        })?;
        self.regs = snap.regs.into_iter().collect();
        self.slow_counter = snap.slow_counter;
        self.reset_requested.set(snap.reset_requested);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u32_at(p: &RtcCntl, offset: u64) -> u32 {
        let mut v = 0u32;
        for i in 0..4u64 {
            v |= (p.read(offset + i).unwrap() as u32) << (i * 8);
        }
        v
    }

    fn write_u32_at(p: &mut RtcCntl, offset: u64, value: u32) {
        for i in 0..4u64 {
            p.write(offset + i, ((value >> (i * 8)) & 0xFF) as u8)
                .unwrap();
        }
    }

    #[test]
    fn new_reports_poweron_reset_for_both_cores() {
        let p = RtcCntl::new();
        let v = read_u32_at(&p, RTC_CNTL_RESET_STATE_OFFSET);
        let procpu = (v >> RESET_CAUSE_PROCPU_SHIFT) & RESET_CAUSE_MASK;
        let appcpu = (v >> RESET_CAUSE_APPCPU_SHIFT) & RESET_CAUSE_MASK;
        assert_eq!(procpu, POWERON_RESET, "PRO_CPU reset cause");
        assert_eq!(appcpu, POWERON_RESET, "APP_CPU reset cause");
    }

    #[test]
    fn rtc_apb_freq_reads_40mhz_encoding_before_any_write() {
        let p = RtcCntl::new();
        assert_eq!(
            read_u32_at(&p, RTC_APB_FREQ_OFFSET),
            RTC_APB_FREQ_40MHZ,
            "RTC_APB_FREQ_REG must read back the 40 MHz encoding (0x0050_0050) \
             at construction so Arduino-ESP32's XTAL probe sees a sane value \
             without needing a wasm-layer fake-write."
        );
    }

    #[test]
    fn store0_round_trips_a_value() {
        let mut p = RtcCntl::new();
        write_u32_at(&mut p, RTC_CNTL_STORE0_OFFSET, 0xDEAD_BEEF);
        assert_eq!(read_u32_at(&p, RTC_CNTL_STORE0_OFFSET), 0xDEAD_BEEF);
    }

    #[test]
    fn all_store_words_round_trip_independently() {
        // Sanity: writing STORE0 doesn't bleed into STORE1..3.
        let mut p = RtcCntl::new();
        write_u32_at(&mut p, RTC_CNTL_STORE0_OFFSET, 0x1111_1111);
        write_u32_at(&mut p, RTC_CNTL_STORE1_OFFSET, 0x2222_2222);
        write_u32_at(&mut p, RTC_CNTL_STORE2_OFFSET, 0x3333_3333);
        write_u32_at(&mut p, RTC_CNTL_STORE3_OFFSET, 0x4444_4444);
        assert_eq!(read_u32_at(&p, RTC_CNTL_STORE0_OFFSET), 0x1111_1111);
        assert_eq!(read_u32_at(&p, RTC_CNTL_STORE1_OFFSET), 0x2222_2222);
        assert_eq!(read_u32_at(&p, RTC_CNTL_STORE2_OFFSET), 0x3333_3333);
        assert_eq!(read_u32_at(&p, RTC_CNTL_STORE3_OFFSET), 0x4444_4444);
    }

    #[test]
    fn tick_advances_slow_counter() {
        let mut p = RtcCntl::new();
        assert_eq!(p.slow_counter(), 0);
        p.tick();
        p.tick();
        p.tick();
        assert_eq!(p.slow_counter(), 3);
    }

    #[test]
    fn time0_reads_reflect_live_slow_counter() {
        let mut p = RtcCntl::new();
        for _ in 0..5 {
            p.tick();
        }
        assert_eq!(read_u32_at(&p, RTC_CNTL_TIME0_OFFSET), 5);
        assert_eq!(read_u32_at(&p, RTC_CNTL_TIME1_OFFSET), 0);
    }

    #[test]
    fn time_update_handshake_clears_trigger_bit() {
        let mut p = RtcCntl::new();
        p.tick();
        p.tick();
        write_u32_at(&mut p, RTC_CNTL_TIME_UPDATE_OFFSET, 1 << 31);
        let trig = read_u32_at(&p, RTC_CNTL_TIME_UPDATE_OFFSET);
        assert_eq!(trig & (1 << 31), 0, "TIME_UPDATE bit must auto-clear");
    }

    #[test]
    fn time_update_handshake_sets_time_valid_ack() {
        // ESP-IDF's `rtc_time_get` (esp_hw_support/port/esp32/rtc_time.c)
        // writes TIME_UPDATE (bit 31) then polls TIME_VALID (bit 30):
        //
        //   WRITE_PERI_REG(RTC_CNTL_TIME_UPDATE_REG, RTC_CNTL_TIME_UPDATE);
        //   while (GET_PERI_REG_MASK(RTC_CNTL_TIME_UPDATE_REG,
        //                            RTC_CNTL_TIME_VALID) == 0) {
        //       esp_rom_delay_us(1);
        //   }
        //
        // Without the ACK the loop runs forever — exactly the stall the
        // labwired-ereader e2e test hit before this fix.
        let mut p = RtcCntl::new();
        write_u32_at(&mut p, RTC_CNTL_TIME_UPDATE_OFFSET, 1 << 31);
        let v = read_u32_at(&p, RTC_CNTL_TIME_UPDATE_OFFSET);
        assert!(
            v & RTC_CNTL_TIME_VALID_BIT != 0,
            "TIME_VALID (bit 30) must be set after TIME_UPDATE write"
        );
    }

    #[test]
    fn ana_conf_dig_pwc_bias_conf_writes_accepted_round_trip() {
        // Power-domain control words have no modeled side effects in this
        // peripheral — they round-trip like regular storage. Reads before
        // any write return 0.
        let mut p = RtcCntl::new();
        assert_eq!(read_u32_at(&p, 0x30), 0, "ANA_CONF unwritten reads 0");
        assert_eq!(read_u32_at(&p, 0x80), 0, "BIAS_CONF unwritten reads 0");
        assert_eq!(read_u32_at(&p, 0x8C), 0, "DIG_PWC unwritten reads 0");
        write_u32_at(&mut p, 0x30, 0xAAAA_5555);
        write_u32_at(&mut p, 0x80, 0xCAFE_F00D);
        write_u32_at(&mut p, 0x8C, 0x1234_5678);
        assert_eq!(read_u32_at(&p, 0x30), 0xAAAA_5555);
        assert_eq!(read_u32_at(&p, 0x80), 0xCAFE_F00D);
        assert_eq!(read_u32_at(&p, 0x8C), 0x1234_5678);
    }

    #[test]
    fn options0_low_bits_round_trip_without_triggering_reset() {
        // OPTIONS0 holds the sw_stall_* fields in bits[1:0] alongside the
        // SW_SYS_RST trigger in bit 31. Writes that leave bit 31 clear
        // must round-trip verbatim and must NOT latch a reset request.
        let mut p = RtcCntl::new();
        write_u32_at(&mut p, RTC_CNTL_OPTIONS0_OFFSET, 0x0000_0003);
        assert_eq!(read_u32_at(&p, RTC_CNTL_OPTIONS0_OFFSET), 0x0000_0003);
        assert!(
            !p.drain_reset_request(),
            "no reset must be requested without OPTIONS0 bit 31"
        );
    }

    #[test]
    fn options0_sw_sys_rst_bit_latches_reset_and_self_clears() {
        // Writing OPTIONS0 with bit 31 set models the SW_SYS_RST trigger.
        // The latch must fire exactly once (drains true on first call,
        // false afterwards) and the backing register must read back
        // without bit 31 — real silicon clears the bit as part of the
        // reset sequence, and the BROM never sees the post-store value
        // because the store never returns.
        let mut p = RtcCntl::new();
        write_u32_at(&mut p, RTC_CNTL_OPTIONS0_OFFSET, 0x8000_0001);
        assert!(p.drain_reset_request(), "first drain must report request");
        assert!(
            !p.drain_reset_request(),
            "second drain must be empty (latch is one-shot)"
        );
        assert_eq!(
            read_u32_at(&p, RTC_CNTL_OPTIONS0_OFFSET) & RTC_CNTL_OPTIONS0_SW_SYS_RST_BIT,
            0,
            "SW_SYS_RST bit must be cleared in backing store after the write"
        );
        // Low bits (sw_stall_*) survive the same write.
        assert_eq!(read_u32_at(&p, RTC_CNTL_OPTIONS0_OFFSET) & 0x3, 0x1);
    }

    #[test]
    fn drain_reset_request_is_false_on_fresh_peripheral() {
        let p = RtcCntl::new();
        assert!(!p.drain_reset_request());
    }

    #[test]
    fn set_reset_cause_changes_reset_state_word() {
        let mut p = RtcCntl::new();
        p.set_reset_cause(SW_RESET, POWERON_RESET);
        let v = read_u32_at(&p, RTC_CNTL_RESET_STATE_OFFSET);
        assert_eq!((v >> RESET_CAUSE_PROCPU_SHIFT) & RESET_CAUSE_MASK, SW_RESET);
        assert_eq!(
            (v >> RESET_CAUSE_APPCPU_SHIFT) & RESET_CAUSE_MASK,
            POWERON_RESET
        );
    }

    #[test]
    fn runtime_snapshot_round_trip_preserves_state() {
        let mut p = RtcCntl::new();
        write_u32_at(&mut p, RTC_CNTL_STORE0_OFFSET, 0xC0FF_EE00);
        for _ in 0..7 {
            p.tick();
        }
        let snap = p.runtime_snapshot();

        let mut restored = RtcCntl::new();
        restored.restore_runtime_snapshot(&snap).unwrap();
        assert_eq!(restored.slow_counter(), 7);
        assert_eq!(read_u32_at(&restored, RTC_CNTL_STORE0_OFFSET), 0xC0FF_EE00);
        assert_eq!(
            read_u32_at(&restored, RTC_APB_FREQ_OFFSET),
            RTC_APB_FREQ_40MHZ
        );
    }

    #[test]
    fn runtime_snapshot_preserves_pending_reset_request() {
        // If a snapshot is taken between the OPTIONS0 write and the
        // machine's per-step drain, the pending reset must survive the
        // round-trip so the restored simulator still re-points the CPU
        // at the reset vector on its next step.
        let mut p = RtcCntl::new();
        write_u32_at(&mut p, RTC_CNTL_OPTIONS0_OFFSET, 0x8000_0000);
        // Latch is set; serialize WITHOUT draining.
        let snap = p.runtime_snapshot();

        let mut restored = RtcCntl::new();
        restored.restore_runtime_snapshot(&snap).unwrap();
        assert!(
            restored.drain_reset_request(),
            "restored peripheral must carry the pending reset"
        );
    }

    #[test]
    fn base_is_esp32_classic_canonical_address() {
        let p = RtcCntl::new();
        assert_eq!(p.base(), 0x3FF4_8000);
    }
}
