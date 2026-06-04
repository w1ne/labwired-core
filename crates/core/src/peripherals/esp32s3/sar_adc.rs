// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! APB SAR-ADC controller (digital ADC controller) for the ESP32-S3.
//!
//! Base address `DR_REG_APB_SARADC_BASE = 0x6004_0000`.
//!
//! This module models the *digital* ADC controller (the "APB SARADC" block),
//! which is the path the IDF/Arduino oneshot driver and the 2nd-stage
//! bootloader's `bootloader_random_enable()` reach through MMIO. It does
//! **not** model the analog SAR cells, the RTC `SENS_*` registers, or the
//! REGI2C side-channel — those live elsewhere.
//!
//! ## What firmware expects from us
//!
//! 1. **Config round-trip.** Drivers read-modify-write CTRL / CTRL2, the
//!    pattern tables, ONETIME_SAMPLE, filter and threshold registers and
//!    expect to read back exactly what they wrote (masked to the documented
//!    bitfields). We store each config register and mask writes.
//!
//! 2. **One-time conversion completes immediately.** When firmware kicks a
//!    one-time sample (`ONETIME_SAMPLE.onetime_start`, bit 29) — or forces a
//!    measurement via `CTRL.start_force | CTRL.start` — we synthesise a
//!    deterministic 12-bit sample *on the spot*, latch it into the result
//!    register in the controller's `[channel:4 | data:12]` output format, set
//!    the matching `*_DONE` raw-interrupt bit, and (if enabled) emit the
//!    APB_ADC interrupt source. `analogRead()` / done-poll loops therefore
//!    always make progress. A peripheral that returned 0 on a polled ready
//!    bit once hung boot; this one never blocks.
//!
//! 3. **Boot survival.** `bootloader_random_enable()` performs ~8 writes to
//!    CTRL / CTRL2 / CLKM_CONF / pattern-tab / ARB_CTRL / FILTER_CTRL0 to set
//!    up free-running timer-triggered entropy sampling. It never polls a
//!    SARADC done bit, but to be safe we still auto-complete any done/valid
//!    bit a poll could wait on whenever a trigger source is armed. Every
//!    register it touches round-trips, so the boot path never observes a
//!    surprising value.
//!
//! ## Determinism
//!
//! Like `rng.rs`, sample generation is a seeded `xorshift32`. The seed is
//! fixed at construction, so the *N*-th conversion of two freshly-built
//! `Esp32s3SarAdc` instances yields byte-identical samples. LabWired runs
//! must be reproducible.
//!
//! ## Register map (offsets from base, ESP32-S3 TRM "APB SARADC")
//!
//! | Off  | Name                       | Notes                              |
//! |-----:|----------------------------|------------------------------------|
//! | 0x00 | CTRL                       | R/W config                         |
//! | 0x04 | CTRL2                      | R/W config (timer en/target, meas) |
//! | 0x08 | FILTER_CTRL1               | R/W filter factors                 |
//! | 0x0C | FSM_WAIT                   | R/W FSM wait counts                |
//! | 0x10 | SAR1_STATUS                | RO                                 |
//! | 0x14 | SAR2_STATUS                | RO                                 |
//! | 0x18 | SAR1_PATT_TAB1             | R/W pattern table 1                |
//! | 0x1C | SAR1_PATT_TAB2             | R/W                                |
//! | 0x20 | SAR1_PATT_TAB3             | R/W                                |
//! | 0x24 | SAR1_PATT_TAB4             | R/W                                |
//! | 0x28 | SAR2_PATT_TAB1             | R/W pattern table 2                |
//! | 0x2C | SAR2_PATT_TAB2             | R/W                                |
//! | 0x30 | SAR2_PATT_TAB3             | R/W                                |
//! | 0x34 | SAR2_PATT_TAB4             | R/W                                |
//! | 0x38 | ARB_CTRL                   | R/W arbiter                        |
//! | 0x3C | FILTER_CTRL0               | R/W filter reset/channel           |
//! | 0x40 | SARADC1_DATA_STATUS        | RO conversion result (17 bits)     |
//! | 0x44 | THRES0_CTRL                | R/W threshold 0                    |
//! | 0x48 | THRES1_CTRL                | R/W threshold 1                    |
//! | 0x58 | THRES_CTRL                 | R/W threshold enables              |
//! | 0x5C | INT_ENA                    | R/W interrupt enable               |
//! | 0x60 | INT_RAW                    | RO raw interrupt flags             |
//! | 0x64 | INT_ST                     | RO masked interrupt status         |
//! | 0x68 | INT_CLR                    | WO W1C interrupt clear             |
//! | 0x6C | DMA_CONF                   | R/W DMA config                     |
//! | 0x70 | CLKM_CONF                  | R/W clock config                   |
//! | 0x78 | SARADC2_DATA_STATUS        | RO conversion result (17 bits)     |
//! | 0x80 | ONETIME_SAMPLE             | R/W one-shot trigger               |
//! | 0x3FC| CTRL_DATE                  | R/W version stamp                  |
//!
//! Note: this layout matches the current ESP-IDF S3 register set in which
//! `ONETIME_SAMPLE` sits at 0x80 and `ARB_CTRL` at 0x38. (Some older IDF
//! headers placed ONETIME at 0x20; we follow the current S3 map.)

use crate::{Peripheral, PeripheralTickResult, SimResult};

// ---- Register offsets -----------------------------------------------------
const CTRL: u64 = 0x00;
const CTRL2: u64 = 0x04;
const FILTER_CTRL1: u64 = 0x08;
const FSM_WAIT: u64 = 0x0C;
const SAR1_STATUS: u64 = 0x10;
const SAR2_STATUS: u64 = 0x14;
const SAR1_PATT_TAB1: u64 = 0x18;
const SAR1_PATT_TAB2: u64 = 0x1C;
const SAR1_PATT_TAB3: u64 = 0x20;
const SAR1_PATT_TAB4: u64 = 0x24;
const SAR2_PATT_TAB1: u64 = 0x28;
const SAR2_PATT_TAB2: u64 = 0x2C;
const SAR2_PATT_TAB3: u64 = 0x30;
const SAR2_PATT_TAB4: u64 = 0x34;
const ARB_CTRL: u64 = 0x38;
const FILTER_CTRL0: u64 = 0x3C;
const SARADC1_DATA_STATUS: u64 = 0x40;
const THRES0_CTRL: u64 = 0x44;
const THRES1_CTRL: u64 = 0x48;
const THRES_CTRL: u64 = 0x58;
const INT_ENA: u64 = 0x5C;
const INT_RAW: u64 = 0x60;
const INT_ST: u64 = 0x64;
const INT_CLR: u64 = 0x68;
const DMA_CONF: u64 = 0x6C;
const CLKM_CONF: u64 = 0x70;
const SARADC2_DATA_STATUS: u64 = 0x78;
const ONETIME_SAMPLE: u64 = 0x80;
const CTRL_DATE: u64 = 0x3FC;

// ---- Bit definitions ------------------------------------------------------
// CTRL (0x00)
const CTRL_START_FORCE: u32 = 1 << 0;
const CTRL_START: u32 = 1 << 1;

// CTRL2 (0x04)
const CTRL2_TIMER_EN: u32 = 1 << 24;

// ONETIME_SAMPLE (0x80)
const ONETIME_SAMPLE1_EN: u32 = 1 << 31;
const ONETIME_SAMPLE2_EN: u32 = 1 << 30;
const ONETIME_START: u32 = 1 << 29;
const ONETIME_CHANNEL_S: u32 = 25;
const ONETIME_CHANNEL_M: u32 = 0xF;

// INT_* bit positions (shared layout across ENA/RAW/ST/CLR)
const INT_ADC1_DONE: u32 = 1 << 31;
const INT_ADC2_DONE: u32 = 1 << 30;
const INT_THRES0_HIGH: u32 = 1 << 29;
const INT_THRES1_HIGH: u32 = 1 << 28;
const INT_THRES0_LOW: u32 = 1 << 27;
const INT_THRES1_LOW: u32 = 1 << 26;
/// Every interrupt bit the controller defines.
const INT_MASK: u32 = INT_ADC1_DONE
    | INT_ADC2_DONE
    | INT_THRES0_HIGH
    | INT_THRES1_HIGH
    | INT_THRES0_LOW
    | INT_THRES1_LOW;

/// Default value of CTRL_DATE register on real S3 silicon.
const CTRL_DATE_DEFAULT: u32 = 0x0210_1180;

/// Mask for the SARADC*_DATA_STATUS result registers (17 bits, RO).
const DATA_RESULT_MASK: u32 = 0x0001_FFFF;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Esp32s3SarAdc {
    // --- config registers (round-tripped) ---
    ctrl: u32,
    ctrl2: u32,
    filter_ctrl1: u32,
    fsm_wait: u32,
    sar1_patt: [u32; 4],
    sar2_patt: [u32; 4],
    arb_ctrl: u32,
    filter_ctrl0: u32,
    thres0_ctrl: u32,
    thres1_ctrl: u32,
    thres_ctrl: u32,
    dma_conf: u32,
    clkm_conf: u32,
    onetime_sample: u32,
    ctrl_date: u32,

    // --- status / result registers ---
    sar1_status: u32,
    sar2_status: u32,
    adc1_data: u32,
    adc2_data: u32,

    // --- interrupt registers ---
    int_ena: u32,
    int_raw: u32,

    // --- deterministic sample source (xorshift32, seeded once) ---
    lfsr: u32,

    /// Source ID (interrupt-matrix source) to emit while INT_ST != 0.
    /// `ETS_APB_ADC_INTR_SOURCE` on the S3 = 64.
    source_id: u32,
}

impl Esp32s3SarAdc {
    /// `source_id` is the interrupt-matrix source the controller emits while
    /// any enabled+raw interrupt is pending. On the ESP32-S3 that is
    /// `ETS_APB_ADC_INTR_SOURCE` = 64 (verified against the SoC
    /// `interrupts.h` enum, which counts from `ETS_WIFI_MAC_INTR_SOURCE = 0`).
    pub fn new(source_id: u32) -> Self {
        Self {
            // Reset defaults per register header (only non-zero ones matter).
            // CTRL: WAIT_ARB_CYCLE=1 (bits 31:30), SAR_CLK_GATED=1 (bit 6),
            //       SAR1/2_PATT_LEN=15 (bits 18:15 / 22:19), SAR_CLK_DIV=4.
            ctrl: (1 << 30) | (0xF << 19) | (0xF << 15) | (4 << 7) | (1 << 6),
            // CTRL2: TIMER_TARGET=10 (bits 23:12), MAX_MEAS_NUM=255 (bits 8:1).
            ctrl2: (10 << 12) | (255 << 1),
            filter_ctrl1: 0,
            // FSM_WAIT: STANDBY_WAIT=255, RSTB_WAIT=8, XPD_WAIT=8.
            fsm_wait: (255 << 16) | (8 << 8) | 8,
            sar1_patt: [0; 4],
            sar2_patt: [0; 4],
            // ARB_CTRL: WIFI_PRIORITY=2 (bits 11:10), RTC_PRIORITY=1 (bits 9:8).
            arb_ctrl: (2 << 10) | (1 << 8),
            // FILTER_CTRL0: CHANNEL0=0xD (bits 23:19), CHANNEL1=0xD (bits 18:14).
            filter_ctrl0: (0xD << 19) | (0xD << 14),
            // THRES*_CTRL: HIGH=0x1FFF (bits 17:5), CHANNEL=13 (bits 4:0).
            thres0_ctrl: (0x1FFF << 5) | 13,
            thres1_ctrl: (0x1FFF << 5) | 13,
            thres_ctrl: 0,
            // DMA_CONF: EOF_NUM=255 (bits 15:0).
            dma_conf: 255,
            // CLKM_CONF: DIV_NUM=4 (bits 7:0).
            clkm_conf: 4,
            // ONETIME_SAMPLE: CHANNEL=13 (bits 28:25).
            onetime_sample: 13 << ONETIME_CHANNEL_S,
            ctrl_date: CTRL_DATE_DEFAULT,

            sar1_status: 0,
            sar2_status: 0,
            adc1_data: 0,
            adc2_data: 0,

            int_ena: 0,
            int_raw: 0,

            // Same constant seed family as rng.rs — fixed for reproducibility.
            lfsr: 0xACE1_5EED,

            source_id,
        }
    }

    /// xorshift32 — deterministic, never settles on zero.
    fn next_word(&mut self) -> u32 {
        let mut x = self.lfsr;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.lfsr = x;
        x
    }

    /// Produce a fresh deterministic 12-bit sample (0..=4095).
    fn next_sample_12bit(&mut self) -> u32 {
        self.next_word() & 0x0FFF
    }

    /// Masked INT_ST = raw & ena.
    fn int_st(&self) -> u32 {
        self.int_raw & self.int_ena
    }

    /// Pack a result in the digital-controller output format:
    /// `[16:13] = channel`, `[11:0] = 12-bit data`. `analogRead()` masks
    /// off the low 12 bits, so the data is directly usable, while the
    /// channel tag in the upper bits matches the DIG output word.
    fn pack_result(channel: u32, data12: u32) -> u32 {
        (((channel & 0xF) << 13) | (data12 & 0x0FFF)) & DATA_RESULT_MASK
    }

    /// Perform a complete ADC1 one-time/forced conversion immediately:
    /// latch a deterministic sample, set the data-valid status, and raise
    /// the ADC1_DONE raw interrupt. Never blocks.
    fn complete_adc1_conversion(&mut self, channel: u32) {
        let sample = self.next_sample_12bit();
        let packed = Self::pack_result(channel, sample);
        self.adc1_data = packed;
        self.sar1_status = packed;
        self.int_raw |= INT_ADC1_DONE;
    }

    /// Same for ADC2.
    fn complete_adc2_conversion(&mut self, channel: u32) {
        let sample = self.next_sample_12bit();
        let packed = Self::pack_result(channel, sample);
        self.adc2_data = packed;
        self.sar2_status = packed;
        self.int_raw |= INT_ADC2_DONE;
    }

    /// Evaluate trigger sources after a config write and auto-complete any
    /// conversion firmware could poll on. This is the boot-safety net: a
    /// one-shot start, a forced measurement start, or an armed free-running
    /// timer all leave the relevant DONE/data-valid bits set so no poll
    /// loop hangs.
    fn service_triggers(&mut self) {
        let onetime_ch = (self.onetime_sample >> ONETIME_CHANNEL_S) & ONETIME_CHANNEL_M;

        // One-shot trigger (the analogRead path on current IDF).
        if self.onetime_sample & ONETIME_START != 0 {
            if self.onetime_sample & ONETIME_SAMPLE1_EN != 0 {
                self.complete_adc1_conversion(onetime_ch);
            }
            if self.onetime_sample & ONETIME_SAMPLE2_EN != 0 {
                self.complete_adc2_conversion(onetime_ch);
            }
            // If neither unit-enable bit is set, default to ADC1 so a bare
            // `onetime_start` still makes the poll loop progress.
            if self.onetime_sample & (ONETIME_SAMPLE1_EN | ONETIME_SAMPLE2_EN) == 0 {
                self.complete_adc1_conversion(onetime_ch);
            }
        }

        // SW-forced measurement start (CTRL.start_force + CTRL.start).
        if (self.ctrl & CTRL_START_FORCE != 0) && (self.ctrl & CTRL_START != 0) {
            self.complete_adc1_conversion(onetime_ch);
        }

        // Free-running timer trigger (bootloader_random entropy mode). The
        // bootloader never polls a done bit, but auto-completing here keeps
        // the data-valid path live and harmless.
        if self.ctrl2 & CTRL2_TIMER_EN != 0 {
            self.complete_adc1_conversion(onetime_ch);
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            CTRL => self.ctrl,
            CTRL2 => self.ctrl2,
            FILTER_CTRL1 => self.filter_ctrl1,
            FSM_WAIT => self.fsm_wait,
            SAR1_STATUS => self.sar1_status,
            SAR2_STATUS => self.sar2_status,
            SAR1_PATT_TAB1 => self.sar1_patt[0],
            SAR1_PATT_TAB2 => self.sar1_patt[1],
            SAR1_PATT_TAB3 => self.sar1_patt[2],
            SAR1_PATT_TAB4 => self.sar1_patt[3],
            SAR2_PATT_TAB1 => self.sar2_patt[0],
            SAR2_PATT_TAB2 => self.sar2_patt[1],
            SAR2_PATT_TAB3 => self.sar2_patt[2],
            SAR2_PATT_TAB4 => self.sar2_patt[3],
            ARB_CTRL => self.arb_ctrl,
            FILTER_CTRL0 => self.filter_ctrl0,
            SARADC1_DATA_STATUS => self.adc1_data,
            THRES0_CTRL => self.thres0_ctrl,
            THRES1_CTRL => self.thres1_ctrl,
            THRES_CTRL => self.thres_ctrl,
            INT_ENA => self.int_ena,
            INT_RAW => self.int_raw,
            INT_ST => self.int_st(),
            INT_CLR => 0, // WO
            DMA_CONF => self.dma_conf,
            CLKM_CONF => self.clkm_conf,
            SARADC2_DATA_STATUS => self.adc2_data,
            ONETIME_SAMPLE => self.onetime_sample,
            CTRL_DATE => self.ctrl_date,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            CTRL => {
                self.ctrl = value;
                self.service_triggers();
            }
            CTRL2 => {
                self.ctrl2 = value;
                self.service_triggers();
            }
            FILTER_CTRL1 => self.filter_ctrl1 = value,
            FSM_WAIT => self.fsm_wait = value,
            // SAR*_STATUS, SARADC*_DATA_STATUS are RO — ignore writes.
            SAR1_STATUS | SAR2_STATUS | SARADC1_DATA_STATUS | SARADC2_DATA_STATUS => {}
            SAR1_PATT_TAB1 => self.sar1_patt[0] = value & 0x00FF_FFFF,
            SAR1_PATT_TAB2 => self.sar1_patt[1] = value & 0x00FF_FFFF,
            SAR1_PATT_TAB3 => self.sar1_patt[2] = value & 0x00FF_FFFF,
            SAR1_PATT_TAB4 => self.sar1_patt[3] = value & 0x00FF_FFFF,
            SAR2_PATT_TAB1 => self.sar2_patt[0] = value & 0x00FF_FFFF,
            SAR2_PATT_TAB2 => self.sar2_patt[1] = value & 0x00FF_FFFF,
            SAR2_PATT_TAB3 => self.sar2_patt[2] = value & 0x00FF_FFFF,
            SAR2_PATT_TAB4 => self.sar2_patt[3] = value & 0x00FF_FFFF,
            ARB_CTRL => self.arb_ctrl = value,
            FILTER_CTRL0 => self.filter_ctrl0 = value,
            THRES0_CTRL => self.thres0_ctrl = value,
            THRES1_CTRL => self.thres1_ctrl = value,
            THRES_CTRL => self.thres_ctrl = value,
            INT_ENA => self.int_ena = value & INT_MASK,
            INT_RAW => {} // RO
            INT_ST => {}  // RO
            INT_CLR => {
                // W1C: writing 1 clears the corresponding raw bit.
                self.int_raw &= !(value & INT_MASK);
            }
            DMA_CONF => self.dma_conf = value,
            CLKM_CONF => self.clkm_conf = value,
            ONETIME_SAMPLE => {
                self.onetime_sample = value;
                self.service_triggers();
            }
            CTRL_DATE => self.ctrl_date = value,
            _ => {}
        }
    }
}

impl Default for Esp32s3SarAdc {
    fn default() -> Self {
        // ETS_APB_ADC_INTR_SOURCE on the ESP32-S3.
        Self::new(64)
    }
}

impl Peripheral for Esp32s3SarAdc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let word = self.read_reg(reg);
        Ok(((word >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn write_word_32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Emit the APB_ADC interrupt source while any enabled+raw flag is set.
        if self.int_st() != 0 {
            PeripheralTickResult {
                explicit_irqs: Some(vec![self.source_id]),
                ..Default::default()
            }
        } else {
            PeripheralTickResult::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Ok(s) = serde_json::from_value::<Esp32s3SarAdc>(state) {
            *self = s;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: u32 = 64; // ETS_APB_ADC_INTR_SOURCE on ESP32-S3.

    #[test]
    fn config_registers_round_trip() {
        let mut a = Esp32s3SarAdc::new(SRC);
        // Pattern tables are 24-bit; pick values exercising masking.
        a.write_u32(SAR1_PATT_TAB1, 0x00AF_FFFF).unwrap();
        a.write_u32(SAR2_PATT_TAB1, 0x12_3456).unwrap();
        a.write_u32(FILTER_CTRL0, 0xDEAD_BEEF).unwrap();
        a.write_u32(THRES0_CTRL, 0x1234_5678).unwrap();
        a.write_u32(CLKM_CONF, 0x0020_0003).unwrap();
        a.write_u32(DMA_CONF, 0x8000_00FF).unwrap();

        assert_eq!(a.read_u32(SAR1_PATT_TAB1).unwrap(), 0x00AF_FFFF);
        assert_eq!(a.read_u32(SAR2_PATT_TAB1).unwrap(), 0x0012_3456);
        assert_eq!(a.read_u32(FILTER_CTRL0).unwrap(), 0xDEAD_BEEF);
        assert_eq!(a.read_u32(THRES0_CTRL).unwrap(), 0x1234_5678);
        assert_eq!(a.read_u32(CLKM_CONF).unwrap(), 0x0020_0003);
        assert_eq!(a.read_u32(DMA_CONF).unwrap(), 0x8000_00FF);

        // Pattern-table upper byte is masked off (24-bit field).
        a.write_u32(SAR1_PATT_TAB2, 0xFFFF_FFFF).unwrap();
        assert_eq!(a.read_u32(SAR1_PATT_TAB2).unwrap(), 0x00FF_FFFF);

        // CTRL_DATE reset default matches silicon.
        assert_eq!(a.read_u32(CTRL_DATE).unwrap(), CTRL_DATE_DEFAULT);
    }

    #[test]
    fn onetime_conversion_completes_done_and_12bit() {
        let mut a = Esp32s3SarAdc::new(SRC);
        // Arm ADC1 one-shot on channel 5, then start.
        let cfg = ONETIME_SAMPLE1_EN | ONETIME_START | (5 << ONETIME_CHANNEL_S);
        a.write_u32(ONETIME_SAMPLE, cfg).unwrap();

        // DONE raw bit must be set immediately.
        let raw = a.read_u32(INT_RAW).unwrap();
        assert_ne!(raw & INT_ADC1_DONE, 0, "ADC1_DONE not raised");

        // Result register: channel tag in [16:13], 12-bit data in [11:0].
        let result = a.read_u32(SARADC1_DATA_STATUS).unwrap();
        let data12 = result & 0x0FFF;
        let chan = (result >> 13) & 0xF;
        assert_eq!(chan, 5, "channel tag wrong");
        assert!(data12 <= 4095, "data not 12-bit: {data12}");
        // Status register mirrors the result.
        assert_eq!(a.read_u32(SAR1_STATUS).unwrap(), result);
    }

    #[test]
    fn forced_start_completes() {
        let mut a = Esp32s3SarAdc::new(SRC);
        a.write_u32(
            CTRL,
            a.read_u32(CTRL).unwrap() | CTRL_START_FORCE | CTRL_START,
        )
        .unwrap();
        assert_ne!(a.read_u32(INT_RAW).unwrap() & INT_ADC1_DONE, 0);
        // DATA field is the low 12 bits; the conversion result must fit it.
        let data = a.read_u32(SARADC1_DATA_STATUS).unwrap() & 0x0FFF;
        assert!(data <= 4095);
    }

    #[test]
    fn timer_trigger_never_blocks() {
        // bootloader_random entropy mode: enabling the timer must leave the
        // data-valid / done path live so any poll completes.
        let mut a = Esp32s3SarAdc::new(SRC);
        a.write_u32(CTRL2, a.read_u32(CTRL2).unwrap() | CTRL2_TIMER_EN)
            .unwrap();
        assert_ne!(a.read_u32(INT_RAW).unwrap() & INT_ADC1_DONE, 0);
    }

    #[test]
    fn samples_are_deterministic_across_instances() {
        let mut a = Esp32s3SarAdc::new(SRC);
        let mut b = Esp32s3SarAdc::new(SRC);
        let cfg = ONETIME_SAMPLE1_EN | ONETIME_START | (3 << ONETIME_CHANNEL_S);

        let mut seq_a = Vec::new();
        let mut seq_b = Vec::new();
        for _ in 0..8 {
            a.write_u32(INT_CLR, INT_ADC1_DONE).unwrap();
            a.write_u32(ONETIME_SAMPLE, cfg).unwrap();
            seq_a.push(a.read_u32(SARADC1_DATA_STATUS).unwrap());

            b.write_u32(INT_CLR, INT_ADC1_DONE).unwrap();
            b.write_u32(ONETIME_SAMPLE, cfg).unwrap();
            seq_b.push(b.read_u32(SARADC1_DATA_STATUS).unwrap());
        }
        assert_eq!(seq_a, seq_b, "sample sequences diverged");
        // And the sequence actually varies (not a stuck constant).
        assert!(
            seq_a.windows(2).any(|w| w[0] != w[1]),
            "samples never changed"
        );
    }

    #[test]
    fn int_clr_is_w1c() {
        let mut a = Esp32s3SarAdc::new(SRC);
        // Raise both DONE bits.
        let cfg = ONETIME_SAMPLE1_EN | ONETIME_SAMPLE2_EN | ONETIME_START;
        a.write_u32(ONETIME_SAMPLE, cfg).unwrap();
        assert_ne!(a.read_u32(INT_RAW).unwrap() & INT_ADC1_DONE, 0);
        assert_ne!(a.read_u32(INT_RAW).unwrap() & INT_ADC2_DONE, 0);

        // Clearing only ADC1_DONE leaves ADC2_DONE set (W1C, per-bit).
        a.write_u32(INT_CLR, INT_ADC1_DONE).unwrap();
        assert_eq!(a.read_u32(INT_RAW).unwrap() & INT_ADC1_DONE, 0);
        assert_ne!(a.read_u32(INT_RAW).unwrap() & INT_ADC2_DONE, 0);

        // Writing 0 clears nothing.
        a.write_u32(INT_CLR, 0).unwrap();
        assert_ne!(a.read_u32(INT_RAW).unwrap() & INT_ADC2_DONE, 0);

        // Clearing ADC2_DONE too leaves raw empty.
        a.write_u32(INT_CLR, INT_ADC2_DONE).unwrap();
        assert_eq!(a.read_u32(INT_RAW).unwrap() & INT_MASK, 0);
    }

    #[test]
    fn int_st_is_raw_and_ena() {
        let mut a = Esp32s3SarAdc::new(SRC);
        // Raise ADC1_DONE but leave enable clear → INT_ST stays 0.
        a.write_u32(ONETIME_SAMPLE, ONETIME_SAMPLE1_EN | ONETIME_START)
            .unwrap();
        assert_ne!(a.read_u32(INT_RAW).unwrap() & INT_ADC1_DONE, 0);
        assert_eq!(a.read_u32(INT_ST).unwrap(), 0, "ST set without ENA");

        // Enable it → ST reflects raw&ena.
        a.write_u32(INT_ENA, INT_ADC1_DONE).unwrap();
        assert_ne!(a.read_u32(INT_ST).unwrap() & INT_ADC1_DONE, 0);
    }

    #[test]
    fn emits_source_while_int_st_set() {
        let mut a = Esp32s3SarAdc::new(SRC);
        // No pending interrupt → no IRQ emitted.
        assert!(a.tick().explicit_irqs.is_none());

        // Raise + enable → tick emits the source.
        a.write_u32(INT_ENA, INT_ADC1_DONE).unwrap();
        a.write_u32(ONETIME_SAMPLE, ONETIME_SAMPLE1_EN | ONETIME_START)
            .unwrap();
        let r = a.tick();
        assert_eq!(r.explicit_irqs, Some(vec![SRC]));

        // Clear the raw bit → emission stops.
        a.write_u32(INT_CLR, INT_ADC1_DONE).unwrap();
        assert!(a.tick().explicit_irqs.is_none());
    }

    #[test]
    fn reset_defaults_seeded() {
        let a = Esp32s3SarAdc::new(SRC);
        // CTRL: WAIT_ARB_CYCLE=1, SAR_CLK_GATED=1, PATT_LEN fields=0xF, DIV=4.
        assert_eq!((a.read_u32(CTRL).unwrap() >> 30) & 0x3, 1);
        assert_ne!(a.read_u32(CTRL).unwrap() & (1 << 6), 0);
        // CTRL_DATE version stamp.
        assert_eq!(a.read_u32(CTRL_DATE).unwrap(), CTRL_DATE_DEFAULT);
        // FSM_WAIT standby=255.
        assert_eq!((a.read_u32(FSM_WAIT).unwrap() >> 16) & 0xFF, 255);
        // No interrupts pending at reset.
        assert_eq!(a.read_u32(INT_RAW).unwrap(), 0);
    }
}
