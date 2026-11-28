// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Three small register stubs needed for esp-hal `init()` to complete:
//!
//! * `SystemStub` — SYSTEM peripheral at 0x600C_0000.
//!   SYSCLK_CONF.SOC_CLK_SEL is round-tripped (esp-hal reads it
//!   back to know the active clock source). Other registers
//!   are write-accept / read-as-zero.
//! * `RtcCntlStub` — RTC_CNTL at 0x6000_8000. Fully cosmetic for hello-world:
//!   read-as-zero, write-accept.
//! * `EfuseStub` — EFUSE at 0x6000_7000. Returns canned MAC + chip-rev for
//!   the few fields esp-hal reads at boot.
//!
//! CHEAT(STUB): SYSTEM/RTC_CNTL/EFUSE are register stubs — read-as-zero /
//! write-accept with a few round-tripped or canned fields, no real behavior.
//! Real: model the register semantics (clock tree, RTC, eFuse). FIDELITY.md §D.

use crate::{Peripheral, SimResult};
use std::collections::HashMap;

/// SYSTEM peripheral stub.  Tracks every written word so reads return what
/// the firmware wrote (so its boot config-back-check passes). Optionally
/// reads as all-ones for unwritten offsets so that boot-time status-bit
/// polls (PLL lock, calibration RDY, etc.) trip on the first iteration —
/// this is on by default for the catch-all `mmio_rest` block where we
/// don't model individual peripherals.
#[derive(Debug)]
pub struct SystemStub {
    words: HashMap<u64, u32>,
    unwritten_read: u32,
}

impl SystemStub {
    /// Default constructor: unwritten offsets read as 0. Use this for the
    /// real SYSTEM register block, where boot code expects RAW power-on
    /// values for fields it hasn't yet programmed.
    pub fn new() -> Self {
        Self {
            words: HashMap::new(),
            unwritten_read: 0,
        }
    }

    /// Variant that reads unwritten offsets as 0xFFFF_FFFF. Suited for the
    /// catch-all mmio_rest stub: real silicon would have READY bits set in
    /// most status registers after the BROM init, and esp-hal's
    /// busy-waits exit on the first poll if the bit reads high.
    pub fn with_unwritten_ones() -> Self {
        Self {
            words: HashMap::new(),
            unwritten_read: u32::MAX,
        }
    }
}

impl Default for SystemStub {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for SystemStub {
    fn has_zero_tick_cycle_cost(&self) -> bool {
        true
    }

    // This is a register file only: all state changes happen synchronously on
    // MMIO writes, with no elapsed-time state, IRQs, DMA, or events.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self
            .words
            .get(&word_off)
            .copied()
            .unwrap_or(self.unwritten_read);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        // Prime the entry with the unwritten-read value so that partial
        // writes leave the unwritten bytes consistent with read-as-X.
        let entry = self.words.entry(word_off).or_insert(self.unwritten_read);
        *entry &= !(0xFFu32 << byte_off);
        *entry |= (value as u32) << byte_off;
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Dump the sparse word map + the unwritten-read sentinel. Compact:
    /// only addresses the firmware actually wrote to get serialized.
    fn runtime_snapshot(&self) -> Vec<u8> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            words: Vec<(u64, u32)>,
            unwritten_read: u32,
        }
        let snap = Snap {
            words: self.words.iter().map(|(k, v)| (*k, *v)).collect(),
            unwritten_read: self.unwritten_read,
        };
        bincode::serialize(&snap).expect("bincode serialize SystemStub")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            words: Vec<(u64, u32)>,
            unwritten_read: u32,
        }
        let snap: Snap = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("SystemStub snapshot decode: {e}"))
        })?;
        self.words = snap.words.into_iter().collect();
        self.unwritten_read = snap.unwritten_read;
        Ok(())
    }
}

/// SPIMEM1 flash-command controller stub (`0x6000_2000`). ESP-IDF's
/// `bootloader_flash_execute_command_common` launches a user command by setting
/// `SPI_MEM_USR` (CMD bit 18) and then busy-waits until the CMD register reads
/// 0 — on silicon the command-trigger bits auto-clear when the command
/// completes. The simulator has no flash-command latency, so we report
/// completion immediately: the CMD register (offset 0) always reads 0. Every
/// other register round-trips.
///
/// LIMITATION: command *results* (read data in the SPI_MEM_W0.. buffers) are
/// not modeled — they read 0. This unblocks completion/config commands; a
/// data-bearing flash read through this path would get zeros (the modeled data
/// path is the memory-mapped `FlashXipPeripheral`, not this command interface).
#[derive(Debug, Default)]
pub struct FlashSpiMemStub {
    words: HashMap<u64, u32>,
}

impl Peripheral for FlashSpiMemStub {
    fn has_zero_tick_cycle_cost(&self) -> bool {
        true
    }

    /// A word map plus one constant (`CMD` always reads 0). Command completion
    /// is reported at read time, not advanced on a clock. No
    /// `tick`/`tick_elapsed` override, so the walk gets the trait default
    /// (`PeripheralTickResult::default()`) — no IRQ, DMA request, mmio-write
    /// or fired event, for every reachable state.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        if word_off == 0 {
            // CMD register: all command-trigger bits auto-clear on completion.
            return Ok(0);
        }
        let word = self.words.get(&word_off).copied().unwrap_or(0);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let entry = self.words.entry(word_off).or_insert(0);
        *entry &= !(0xFFu32 << byte_off);
        *entry |= (value as u32) << byte_off;
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// RTC / PMU / IO_MUX / RTC_IO / APB_CTRL peripheral stub. Round-trips
/// every written word so that read-modify-write sequences observe the
/// values esp-hal just wrote (e.g. clock-mux selectors, voltage rails,
/// GPIO mux). Unwritten offsets read as zero, except for status bits
/// that boot code busy-waits for (e.g. PLL_LOCK) which are seeded.
///
/// ⚠ **This stub is NOT the ESP32-classic RTC_CNTL.** Classic ESP32 builds
/// `esp32::rtc_cntl::RtcCntl` (see `peripherals/esp32/factory.rs`); the only
/// consumer of this type is the **ESP32-S3** (`system/xtensa/esp32s3.rs`).
/// So a seed added here for classic ESP32 is inert where it was aimed and
/// lands on the S3 instead — and the two do not share a register map:
///
/// | offset | ESP32 | ESP32-S3 |
/// |--------|-------|----------|
/// | 0xB4   | STORE5 (= RTC_APB_FREQ_REG) | **SWD_CONF** (super watchdog) |
/// | 0xC4   | —     | STORE5 |
///
/// A seed written here at 0xB4 for the classic part would have put
/// 0x4C4B4C4B into the S3's super-watchdog configuration. Check the offset
/// in `tests/fixtures/svd/esp32s3.svd` before adding anything to `new()`.
#[derive(Debug)]
pub struct RtcCntlStub {
    words: HashMap<u64, u32>,
}

impl Default for RtcCntlStub {
    fn default() -> Self {
        Self::new()
    }
}

impl RtcCntlStub {
    pub fn new() -> Self {
        let mut words = HashMap::new();
        // 0x6040 = SYSCON_DATE / RTC PLL status (within the 0x6000_8000-base
        // window, this is absolute 0x6000_e040). esp-hal's
        // request_pll_clk()/ensure_voltage_raised() polls bit 24 = PLL_LOCK
        // until it goes high; on real silicon the BBPLL asserts within a few
        // microseconds of being requested. We seed it as locked.
        words.insert(0x6040, 0x0100_0000);
        // APP_CPU software stall (RTC_CNTL_SW_CPU_STALL_REG @ +0xBC). On real
        // silicon the ROM releases core 1 from reset early, then holds it with
        // the SW stall; the application un-stalls it via esp_cpu_unstall(1).
        // Seed the "stalled" magic (SW_STALL_APPCPU_C1 = 0x21 in bits[31:26])
        // so the firmware's clear of it is a detectable un-stall edge — the
        // faithful APP_CPU release trigger (no firmware-symbol hooks).
        words.insert(0xBC, APPCPU_STALL_C1 << APPCPU_STALL_C1_SHIFT);
        Self { words }
    }
}

/// RTC_CNTL_SW_CPU_STALL_REG offset within the RTC_CNTL window.
const SW_CPU_STALL_OFF: u64 = 0xBC;
/// SW_STALL_APPCPU_C1 field (RTC_CNTL_SW_STALL_APPCPU_C1_S = 20): value 0x21 in
/// bits[25:20] means "APP_CPU stalled". Clearing it (esp_cpu_unstall) releases.
const APPCPU_STALL_C1: u32 = 0x21;
const APPCPU_STALL_C1_SHIFT: u32 = 20;

impl RtcCntlStub {
    /// Bincode-serialize the sparse word map for runtime snapshots.
    fn snapshot_words(&self) -> Vec<u8> {
        let words: Vec<(u64, u32)> = self.words.iter().map(|(k, v)| (*k, *v)).collect();
        bincode::serialize(&words).expect("bincode serialize RtcCntlStub words")
    }
    fn restore_words(&mut self, bytes: &[u8]) -> SimResult<()> {
        let words: Vec<(u64, u32)> = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("RtcCntlStub snapshot decode: {e}"))
        })?;
        self.words = words.into_iter().collect();
        Ok(())
    }
}

impl Peripheral for RtcCntlStub {
    fn has_zero_tick_cycle_cost(&self) -> bool {
        true
    }

    // Inert walk: RTC_CNTL register bank; TIME_UPDATE handshake settles on write.
    // tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.words.get(&word_off).copied().unwrap_or(0);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }
    fn runtime_snapshot(&self) -> Vec<u8> {
        self.snapshot_words()
    }
    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        self.restore_words(bytes)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let entry = self.words.entry(word_off).or_insert(0);
        *entry &= !(0xFFu32 << byte_off);
        *entry |= (value as u32) << byte_off;
        // RTC_CNTL_TIME_UPDATE_REG (offset 0x0C). Per TRM §31.3: firmware
        // writes bit 31 (TIME_UPDATE) to ask the RTC controller to
        // snapshot its 48-bit counter into RTC_CNTL_TIME0_REG (0x10) /
        // RTC_CNTL_TIME1_REG (0x14). Real silicon clears bit 31 and
        // sets bit 30 (TIME_VALID) ~3 RTC cycles later. We model the
        // same handshake atomically: when TIME_UPDATE is written we
        // clear it, set TIME_VALID, and bump a virtual counter so
        // back-to-back `rtc_time_get` calls observe monotonically-
        // increasing timestamps.
        if word_off == 0x0C {
            let cur = *entry;
            if cur & (1 << 31) != 0 {
                self.words.insert(0x0C, (cur & !(1 << 31)) | (1 << 30));
                let t0 = self.words.get(&0x10).copied().unwrap_or(0);
                let t1 = self.words.get(&0x14).copied().unwrap_or(0);
                let combined = ((t1 as u64) << 32) | (t0 as u64);
                // Bump by ~1024 ticks (~7 ms at 150 kHz RTC). Arbitrary
                // but consistent so the firmware sees forward progress.
                let next = combined.wrapping_add(1024);
                self.words.insert(0x10, (next & 0xFFFF_FFFF) as u32);
                self.words.insert(0x14, (next >> 32) as u32);
            }
        }
        // APP_CPU un-stall: SW_CPU_STALL_REG (0xBC) SW_STALL_APPCPU_C1 leaving
        // the 0x21 "stalled" code (esp_cpu_unstall(1)) releases core 1. Signal
        // the run loop to boot the APP_CPU from the real ROM reset vector.
        if word_off == SW_CPU_STALL_OFF {
            // Re-read from the map (not `entry`) so we don't extend its mutable
            // borrow across the TIME_UPDATE block's `self.words.insert`.
            let w = self.words.get(&word_off).copied().unwrap_or(0);
            let c1 = (w >> APPCPU_STALL_C1_SHIFT) & 0x3F;
            if std::env::var("LABWIRED_CCDBG").is_ok() {
                eprintln!("rtc_cntl: SW_CPU_STALL write -> 0x{w:08x} (C1=0x{c1:02x})");
            }
            if c1 != APPCPU_STALL_C1 {
                crate::peripherals::esp_xtensa_common::rom_thunks::APPCPU_RESET_RELEASED
                    .with(|s| s.set(true));
            }
        }
        Ok(())
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// ESP32-classic TIMG (Timer Group) — models the RTC clock-calibration
/// state machine that esp-idf's `rtc_clk_wait_for_slow_cycle` and
/// `rtc_clk_cal_internal` rely on.
///
/// Reference: ESP32 TRM §16 Timer Group, plus §31.4 RTC clock cal.
///
/// Real silicon behavior modeled here:
///   * Firmware writes `RTC_CALI_START` (bit 31 of RTCCALICFG at 0x68)
///     together with the count of clock periods to measure (`RTC_CALI_MAX`
///     in bits[29:13]) and the clock source selector.
///   * Hardware clocks the calibration ratio for ~MAX cycles, then sets
///     `RTC_CALI_RDY` (bit 15 of RTCCALICFG) and writes the measured
///     result into `RTC_CALI_VALUE` field of RTCCALICFG1 (bits[31:7]).
///   * Firmware busy-polls RTC_CALI_RDY (or the deprecated bit 15 in
///     RTCCALICFG1 at 0x6C) and reads back the value.
///
/// We don't have a free-running RTC clock to count for real, but the
/// firmware only needs the RDY bit and a self-consistent value to make
/// forward progress. So: on every write to RTCCALICFG with START=1, we
/// SET RDY=1 immediately and stash a result derived from MAX so the
/// downstream `result_in_us = RTC_CLK_PERIOD * MAX / 8` arithmetic
/// produces a sane CPU frequency calculation.
///
/// Watchdog regs (WDTCONFIG0..WDTWPROTECT at 0x48..0x64) keep round-trip
/// behavior — esp-hal pokes them once to disable the WDT.
#[derive(Debug, Default)]
pub struct TimgStub {
    words: HashMap<u64, u32>,
}

impl TimgStub {
    pub fn new() -> Self {
        Self {
            words: HashMap::new(),
        }
    }

    const RTCCALICFG_OFFSET: u64 = 0x68;
    const RTCCALICFG1_OFFSET: u64 = 0x6C;
    const RTC_CALI_START_BIT: u32 = 1 << 31;
    const RTC_CALI_RDY_BIT: u32 = 1 << 15;

    /// Run the RTC calibration state machine — called whenever bits
    /// flip in RTCCALICFG.
    fn maybe_complete_calibration(&mut self) {
        let cfg = self
            .words
            .get(&Self::RTCCALICFG_OFFSET)
            .copied()
            .unwrap_or(0);
        if cfg & Self::RTC_CALI_START_BIT == 0 {
            return;
        }
        // RTC_CALI_MAX is bits[29:13] of RTCCALICFG — number of clock periods
        // to count. Clamp to a sane non-zero default if firmware passes 0.
        let max = ((cfg >> 13) & 0x1FFFF).max(1);
        // Mark RDY in RTCCALICFG.
        self.words
            .insert(Self::RTCCALICFG_OFFSET, cfg | Self::RTC_CALI_RDY_BIT);
        // RTCCALICFG1: bit 0 = RDY (legacy), bits[31:7] = VALUE.
        // VALUE = number of APB clock cycles in `max` periods of the
        // calibration clock. For RTC_SLOW_CLK ≈ 150 kHz and APB ≈ 80 MHz,
        // ratio ≈ 533 cycles per RTC period. So VALUE ≈ max * 533.
        let value = max.wrapping_mul(533) & 0x01FF_FFFF; // 25 bits
        self.words.insert(
            Self::RTCCALICFG1_OFFSET,
            (value << 7) | 1, // bit 0 = RDY (legacy field)
        );
    }
}

impl Peripheral for TimgStub {
    fn has_zero_tick_cycle_cost(&self) -> bool {
        true
    }

    /// A word map whose only state machine — the RTC calibration in
    /// [`Self::maybe_complete_calibration`] — runs from `write`, not from a
    /// clock: exactly the "lazily-evaluated model that advances on MMIO
    /// access" case in the [`Peripheral::needs_legacy_walk`] contract. No
    /// `tick`/`tick_elapsed` override, so the walk gets the trait default
    /// (`PeripheralTickResult::default()`) — no IRQ, DMA request, mmio-write
    /// or fired event, for every reachable state.
    fn needs_legacy_walk(&self) -> bool {
        false
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.words.get(&word_off).copied().unwrap_or(0);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }
    fn runtime_snapshot(&self) -> Vec<u8> {
        let words: Vec<(u64, u32)> = self.words.iter().map(|(k, v)| (*k, *v)).collect();
        bincode::serialize(&words).expect("bincode serialize TimgStub")
    }
    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        let words: Vec<(u64, u32)> = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("TimgStub snapshot decode: {e}"))
        })?;
        self.words = words.into_iter().collect();
        Ok(())
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let entry = self.words.entry(word_off).or_insert(0);
        *entry &= !(0xFFu32 << byte_off);
        *entry |= (value as u32) << byte_off;
        if word_off == Self::RTCCALICFG_OFFSET {
            self.maybe_complete_calibration();
        }
        Ok(())
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// EFUSE peripheral stub.  Returns canned MAC + chip-rev for the fields
/// esp-hal reads at boot.
///
/// Per ESP32-S3 TRM §6 (eFuse Controller), the relevant fields esp-hal touches:
///
/// | Offset | Field                        | Canned value |
/// |-------:|------------------------------|--------------|
/// |  0x044 | RD_MAC_SPI_SYS_0 (MAC[3:0])  | 0x00000002   |
/// |  0x048 | RD_MAC_SPI_SYS_1 (MAC[5:4])  | 0x00000000   |
/// |  0x05C | RD_SYS_PART1_DATA0 (chip_rev)| 0x00000000   |
///
/// The canned MAC is `02:00:00:00:00:01` (locally-administered).
#[derive(Debug, Default)]
pub struct EfuseStub;

impl EfuseStub {
    pub fn new() -> Self {
        Self
    }

    /// Pack an eFuse bitfield (`bit_start`, `bit_count`) into a little-endian
    /// array of 32-bit block words, matching `esp_efuse_table.csv` /
    /// `esp_efuse_read_field_blob`.
    const fn pack_field<const N: usize>(
        mut words: [u32; N],
        bit_start: usize,
        bit_count: usize,
        value: u32,
    ) -> [u32; N] {
        let mut i = 0;
        while i < bit_count {
            if ((value >> i) & 1) != 0 {
                let bit_pos = bit_start + i;
                words[bit_pos / 32] |= 1u32 << (bit_pos % 32);
            }
            i += 1;
        }
        words
    }

    /// ESP32-S3 `EFUSE_BLK1` (`RD_MAC_SPI_SYS_0..5`, offsets `0x044..=0x058`,
    /// 6 words = 192 bits).
    ///
    /// Preserves the canned MAC (`0x0000_0002` at `0x044`, `0x0000_0000` at
    /// `0x048`) and populates `ADC2_CAL_VOL_ATTEN3` (bit 186, width 6) for
    /// factory V1 ADC curve-fitting calibration.
    const BLK1_WORDS: [u32; 6] = {
        let w = [0u32; 6];
        // MAC low word: 0x00 00 00 02 (bits 0..31 -> 0x044)
        let w = Self::pack_field(w, 0, 32, 0x0000_0002);
        // ADC2_CAL_VOL_ATTEN3: EFUSE_BLK1 bit 186, width 6 -> diff = 15 so
        // adc2_vol[3] = adc1_vol[3] - 15 + 15 = adc1_vol[3].
        Self::pack_field(w, 186, 6, 15)
    };

    /// ESP32-S3 `EFUSE_BLK2` (`RD_SYS_PART1_DATA0..7`, offsets `0x05C..=0x078`,
    /// 8 words = 256 bits).
    ///
    /// Populates factory V1 ADC curve-fitting calibration fields from ESP-IDF
    /// v5.3.2 `components/efuse/esp32s3/esp_efuse_table.csv` so
    /// `esp_efuse_rtc_calib_get_ver()` returns `1` and Arduino
    /// `analogReadMilliVolts()` / `adc_cali_create_scheme_curve_fitting()`
    /// succeeds without aborting with `ESP_ERR_NOT_FOUND` /
    /// `ESP_ERR_NOT_SUPPORTED`.
    ///
    /// Calibration voltages (`ADC1_CAL_VOL_ATTEN0..3` diffs = `[70, 100, 215,
    /// 50]` -> decoded digi `[3635, 2765, 1965, 950]`) keep the ATTEN3
    /// curve-fitting ceiling above the ESP32-S3 datasheet's `3100 mV` range
    /// (`3203 mV` at raw `4095`). Pair with [`esp32s3_adc1_mv_to_raw`] in
    /// `Esp32s3Sens` / `Esp32s3SarAdc` so `analogReadMilliVolts()` round-trips
    /// injected voltages to within `<= 2 mV` across `0..3200 mV` with no
    /// near-3 V roll-off.
    const BLK2_WORDS: [u32; 8] = {
        let w = [0u32; 8];
        // BLK_VERSION_MAJOR = 1 (V1 ADC calibration): bit 128, width 2
        let w = Self::pack_field(w, 128, 2, 1);
        // TEMP_CALIB = 0 (0.0 C offset): bit 132, width 9
        let w = Self::pack_field(w, 132, 9, 0);
        // OCODE = 100 (nominal ESP32-S3 oscillator calibration code): bit 141, width 8
        let w = Self::pack_field(w, 141, 8, 100);
        // ADC1/ADC2 init code diffs = 0 -> nominal init codes (1850..2020)
        // ADC1_INIT_CODE_ATTEN0..3: bits 149(8), 157(6), 163(6), 169(6)
        // ADC2_INIT_CODE_ATTEN0..3: bits 175(8), 183(6), 189(6), 195(6)
        // ADC1_CAL_VOL_ATTEN0..3: bits 201(8), 209(8), 217(8), 225(8)
        let w = Self::pack_field(w, 201, 8, 70);
        let w = Self::pack_field(w, 209, 8, 100);
        let w = Self::pack_field(w, 217, 8, 215);
        let w = Self::pack_field(w, 225, 8, 50);
        // ADC2_CAL_VOL_ATTEN0..2: bits 233(8), 241(7), 248(7) -> match ADC1
        let w = Self::pack_field(w, 233, 8, 40);
        let w = Self::pack_field(w, 241, 7, 10);
        Self::pack_field(w, 248, 7, 20)
    };
}

/// Decoded ESP32-S3 V1 ADC1 calibration digital values (`[ATTEN0, ATTEN1,
/// ATTEN2, ATTEN3]`) exposed by [`EfuseStub`]. All four attenuations calibrate
/// against `850 mV` in ESP-IDF v5.3.2 `esp_efuse_rtc_calib_get_cal_voltage`.
pub const ESP32S3_V1_CAL_VOL: [u32; 4] = [3635, 2765, 1965, 950];

/// Exact ESP-IDF v5.3.2 ESP32-S3 `ADC_ATTEN_DB_12` (ATTEN3) V1 curve-fitting
/// polynomial (`adc_cali_raw_to_voltage`), matching [`EfuseStub`]'s `v1_3`.
pub fn esp32s3_atten3_curve_fit_mv(raw: u16) -> u16 {
    const COEFF_A_SCALING: u64 = 65536;
    const COEFF_A: u64 = (COEFF_A_SCALING * 850) / ESP32S3_V1_CAL_VOL[3] as u64;
    const TERMS: [(u64, u64, i64); 5] = [
        (644_403_418_269_478, 1_000_000_000_000_000, -1),
        (644_334_888_647_536, 10_000_000_000_000_000, -1),
        (1_297_891_447_611, 10_000_000_000_000_000, 1),
        (70_769_718, 1_000_000_000_000_000, -1),
        (13_515, 1_000_000_000_000_000, 1),
    ];

    let v_cali_1 = (u64::from(raw.min(4095)) * COEFF_A) / COEFF_A_SCALING;
    if v_cali_1 == 0 {
        return 0;
    }
    let mut var = 1u128;
    let mut err = 0i64;
    for (idx, &(c, m, sign)) in TERMS.iter().enumerate() {
        if idx > 0 {
            var *= u128::from(v_cali_1);
        }
        err += ((var * u128::from(c)) / u128::from(m)) as i64 * sign;
    }
    (v_cali_1 as i64 - err).clamp(0, i64::from(u16::MAX)) as u16
}

/// Convert a physical pad voltage (`millivolts`) into the 12-bit ESP32-S3 SAR
/// ADC1 raw count for attenuation `atten` (`0..=3`, where `3` is 12 dB /
/// `ADC_ATTEN_DB_12`).
///
/// For `atten == 3`, inverts [`esp32s3_atten3_curve_fit_mv`] so real ESP-IDF /
/// Arduino `analogReadMilliVolts()` recovers the injected voltage to within
/// `<= 1 mV` across `0..3200 mV` instead of rolling off near 3 V.
pub fn esp32s3_adc1_mv_to_raw(millivolts: u16, atten: u8) -> u16 {
    if millivolts == 0 {
        return 0;
    }
    let atten_idx = (atten & 0x3) as usize;
    if atten_idx < 3 {
        let cal = u64::from(ESP32S3_V1_CAL_VOL[atten_idx]);
        return ((u64::from(millivolts) * cal + 425) / 850).min(4095) as u16;
    }

    // `esp32s3_atten3_curve_fit_mv` is strictly non-decreasing on 0..=4095
    // (ceiling 3203 mV at raw = 4095). Binary-search for the closest raw code.
    let mut lo: u16 = 0;
    let mut hi: u16 = 4095;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if esp32s3_atten3_curve_fit_mv(mid) < millivolts {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo > 0 {
        let above = esp32s3_atten3_curve_fit_mv(lo).abs_diff(millivolts);
        let below = esp32s3_atten3_curve_fit_mv(lo - 1).abs_diff(millivolts);
        if below < above {
            return lo - 1;
        }
    }
    lo
}

impl Peripheral for EfuseStub {
    fn has_zero_tick_cycle_cost(&self) -> bool {
        true
    }

    // eFuse reads are immutable canned values; the stub has no time-driven
    // behavior to service from the legacy walk.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word: u32 = match word_off {
            0x044..=0x058 => Self::BLK1_WORDS[((word_off - 0x044) / 4) as usize],
            0x05C..=0x078 => Self::BLK2_WORDS[((word_off - 0x05C) / 4) as usize],
            _ => {
                crate::census_reg!("esp_xtensa_common.system_stub:EfuseStub", word_off, "read");
                0
            }
        };
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_stub_round_trips_words() {
        let mut s = SystemStub::new();
        s.write(0x10, 0xAB).unwrap();
        s.write(0x11, 0xCD).unwrap();
        s.write(0x12, 0xEF).unwrap();
        s.write(0x13, 0x12).unwrap();
        assert_eq!(s.read(0x10).unwrap(), 0xAB);
        assert_eq!(s.read(0x11).unwrap(), 0xCD);
        assert_eq!(s.read(0x13).unwrap(), 0x12);
    }

    #[test]
    fn rtc_cntl_stub_unwritten_reads_zero() {
        let s = RtcCntlStub::new();
        for off in 0..16u64 {
            assert_eq!(s.read(off).unwrap(), 0);
        }
    }

    #[test]
    fn rtc_cntl_stub_round_trips_writes() {
        let mut s = RtcCntlStub::new();
        s.write(0x40, 0x12).unwrap();
        s.write(0x41, 0x34).unwrap();
        assert_eq!(s.read(0x40).unwrap(), 0x12);
        assert_eq!(s.read(0x41).unwrap(), 0x34);
    }

    #[test]
    fn efuse_returns_canned_mac() {
        let s = EfuseStub::new();
        // MAC low byte at 0x044 = 0x02.
        assert_eq!(s.read(0x044).unwrap(), 0x02);
        assert_eq!(s.read(0x045).unwrap(), 0x00);
        // MAC high byte at 0x048 = 0x00 (no high MAC bytes).
        assert_eq!(s.read(0x048).unwrap(), 0x00);
        // chip_rev at 0x05C = 0.
        assert_eq!(s.read(0x05C).unwrap(), 0x00);
    }

    #[test]
    fn efuse_unknown_offset_reads_zero() {
        let s = EfuseStub::new();
        assert_eq!(s.read(0x100).unwrap(), 0);
    }

    /// Decode an eFuse bitfield from `EfuseStub` using the exact bit-extraction
    /// order of ESP-IDF's `esp_efuse_read_field_blob`.
    fn read_efuse_bits(
        efuse: &EfuseStub,
        blk_base: u64,
        bit_start: usize,
        bit_count: usize,
    ) -> u32 {
        let mut out = 0u32;
        for i in 0..bit_count {
            let bit_pos = bit_start + i;
            let byte = efuse.read(blk_base + (bit_pos / 8) as u64).unwrap();
            if ((byte >> (bit_pos % 8)) & 1) != 0 {
                out |= 1u32 << i;
            }
        }
        out
    }

    #[test]
    fn efuse_exposes_esp32s3_v1_adc_calibration_fields() {
        let efuse = EfuseStub::new();
        // BLK_VERSION_MAJOR == 1 (ESP_EFUSE_ADC_CALIB_VER)
        assert_eq!(read_efuse_bits(&efuse, 0x05C, 128, 2), 1);
        // TEMP_CALIB == 0, OCODE == 100
        assert_eq!(read_efuse_bits(&efuse, 0x05C, 132, 9), 0);
        assert_eq!(read_efuse_bits(&efuse, 0x05C, 141, 8), 100);

        // Decode ADC1 & ADC2 calibration voltages using the exact ESP-IDF v5.3.2
        // `esp_efuse_rtc_calib_get_cal_voltage` V1 cumulative-diff formula.
        let adc1_d0 = read_efuse_bits(&efuse, 0x05C, 201, 8);
        let adc1_d1 = read_efuse_bits(&efuse, 0x05C, 209, 8);
        let adc1_d2 = read_efuse_bits(&efuse, 0x05C, 217, 8);
        let adc1_d3 = read_efuse_bits(&efuse, 0x05C, 225, 8);

        let v1_3 = adc1_d3 + 900;
        let v1_2 = adc1_d2 + v1_3 + 800;
        let v1_1 = adc1_d1 + v1_2 + 700;
        let v1_0 = adc1_d0 + v1_1 + 800;
        assert_eq!([v1_0, v1_1, v1_2, v1_3], ESP32S3_V1_CAL_VOL);

        let adc2_d0 = read_efuse_bits(&efuse, 0x05C, 233, 8);
        let adc2_d1 = read_efuse_bits(&efuse, 0x05C, 241, 7);
        let adc2_d2 = read_efuse_bits(&efuse, 0x05C, 248, 7);
        let adc2_d3 = read_efuse_bits(&efuse, 0x044, 186, 6);

        let v2_3 = v1_3 - adc2_d3 + 15;
        let v2_2 = v1_2 - adc2_d2 + 20;
        let v2_1 = v1_1 - adc2_d1 + 10;
        let v2_0 = v1_0 - adc2_d0 + 40;
        assert_eq!([v2_0, v2_1, v2_2, v2_3], ESP32S3_V1_CAL_VOL);

        // Verify ESP-IDF v5.3.2 ATTEN3 curve-fitting polynomial round-trips
        // `esp32s3_adc1_mv_to_raw(mv, 3)` back to millivolts within <= 2 mV
        // across the full 0..3200 mV range (including near-3 V sensor divider
        // points such as 2970 mV soil-moisture and 3100 mV 500 lx LDR).
        const COEFF_A_SCALING: u64 = 65536;
        let coeff_a = (COEFF_A_SCALING * 850) / u64::from(v1_3);
        let terms: [(u64, u64, i64); 5] = [
            (644_403_418_269_478, 1_000_000_000_000_000, -1),
            (644_334_888_647_536, 10_000_000_000_000_000, -1),
            (1_297_891_447_611, 10_000_000_000_000_000, 1),
            (70_769_718, 1_000_000_000_000_000, -1),
            (13_515, 1_000_000_000_000_000, 1),
        ];
        for mv in [
            0u16, 330, 660, 850, 1000, 1650, 2000, 2500, 2970, 3100, 3200,
        ] {
            let raw = u64::from(esp32s3_adc1_mv_to_raw(mv, 3));
            let v_cali_1 = (raw * coeff_a) / COEFF_A_SCALING;
            let out_mv = if v_cali_1 == 0 {
                0i64
            } else {
                let mut var = 1u128;
                let mut err = 0i64;
                for (idx, &(c, m, sign)) in terms.iter().enumerate() {
                    if idx > 0 {
                        var *= u128::from(v_cali_1);
                    }
                    err += ((var * u128::from(c)) / u128::from(m)) as i64 * sign;
                }
                v_cali_1 as i64 - err
            };
            assert_eq!(out_mv, i64::from(esp32s3_atten3_curve_fit_mv(raw as u16)));
            assert!(
                (out_mv - i64::from(mv)).abs() <= 2,
                "ATTEN3 curve fit at {mv} mV returned {out_mv} mV (raw={raw})"
            );
        }
    }
}
