// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 LEDC (LED PWM controller) — register-faithful digital twin.
//!
//! Mapped at base `DR_REG_LEDC_BASE = 0x6001_9000`, size 4 KiB. See
//! ESP32-S3 TRM §31. On the S3 the high-speed channels of the original
//! ESP32 are gone; there is a single "low speed" group whose registers are
//! still named `LSCHn` / `LSTIMERn` in `soc/ledc_reg.h`. We model that group.
//!
//! ## What firmware does with LEDC
//!
//! Arduino's `ledcSetup(ch, freq, res)` programs a timer's divider and
//! duty-resolution; `ledcWrite(ch, duty)` stages `duty << 4` into the
//! channel's DUTY register and pulses the per-channel DUTY_START bit in
//! CONF1. ESP-IDF's `ledc_set_duty` / `ledc_update_duty` do the same. Both
//! then read the value back (`ledcRead`, `ledc_get_duty`). So the dominant
//! fidelity requirement is **round-tripping the config registers** and the
//! **DUTY → DUTY_R commit** on a DUTY_START write — which is what this module
//! does. We do not synthesise a GPIO waveform; the duty/freq registers are
//! the observable state and the GPIO matrix routes the (modeled-elsewhere)
//! signal.
//!
//! ## Register subset modeled
//!
//! Offsets per `soc/esp32s3/register/soc/ledc_reg.h`:
//!
//! | Offset        | Name (per channel n / timer t)        | Notes                                  |
//! |---------------|---------------------------------------|----------------------------------------|
//! | 0x00 + n*0x14 | LEDC_LSCHn_CONF0                      | clock/timer-sel + sig_out_en (round-trip) |
//! | 0x04 + n*0x14 | LEDC_LSCHn_HPOINT                     | 14-bit hpoint (round-trip)             |
//! | 0x08 + n*0x14 | LEDC_LSCHn_DUTY                       | 19-bit staged duty (duty<<4 format)    |
//! | 0x0C + n*0x14 | LEDC_LSCHn_CONF1                      | bit31 DUTY_START commits DUTY→DUTY_R   |
//! | 0x10 + n*0x14 | LEDC_LSCHn_DUTY_R  (read-only)        | committed/active duty read-back        |
//! | 0xA0 + t*0x08 | LEDC_LSTIMERt_CONF                    | divider + duty_res (round-trip)        |
//! | 0xA4 + t*0x08 | LEDC_LSTIMERt_VALUE (read-only)       | current timer count (modeled as 0)     |
//! | 0xC0          | LEDC_INT_RAW                          | raw interrupt latches                  |
//! | 0xC4          | LEDC_INT_ST   (read-only)             | INT_RAW & INT_ENA                      |
//! | 0xC8          | LEDC_INT_ENA                          | enable mask (round-trip)               |
//! | 0xCC          | LEDC_INT_CLR  (write-1-to-clear)      | clears matching INT_RAW bits           |
//! | 0xD0          | LEDC_CONF                             | clock select; bit31 CLK_EN (round-trip)|
//!
//! INT_RAW bit layout (per header): bits 0..3 = LSTIMER0..3 overflow,
//! bits 4..11 = DUTY_CHNG_END for channels 0..7.
//!
//! All other offsets accept writes silently and read 0.

use crate::{Peripheral, PeripheralTickResult, SimResult};

pub const LEDC_BASE: u32 = 0x6001_9000;
pub const LEDC_SIZE: u64 = 0x1000;

/// `ETS_LEDC_INTR_SOURCE = 35` (level), per
/// `soc/esp32s3/include/soc/interrupts.h`. Used as the default interrupt
/// matrix source id; `new()` hard-codes it, `new_with_source` allows override.
pub const LEDC_INTR_SOURCE_ID: u32 = 35;

const NUM_CHANNELS: usize = 8;
const NUM_TIMERS: usize = 4;

/// Per-channel register block stride (CONF0, HPOINT, DUTY, CONF1, DUTY_R).
const CH_STRIDE: u64 = 0x14;
const CH_BLOCK_BASE: u64 = 0x00;
const CH_BLOCK_END: u64 = CH_BLOCK_BASE + (NUM_CHANNELS as u64) * CH_STRIDE; // 0xA0

const CH_CONF0: u64 = 0x00;
const CH_HPOINT: u64 = 0x04;
const CH_DUTY: u64 = 0x08;
const CH_CONF1: u64 = 0x0C;
const CH_DUTY_R: u64 = 0x10;

/// Per-timer register block stride (CONF, VALUE).
const TIMER_STRIDE: u64 = 0x08;
const TIMER_BLOCK_BASE: u64 = 0xA0;
const TIMER_BLOCK_END: u64 = TIMER_BLOCK_BASE + (NUM_TIMERS as u64) * TIMER_STRIDE; // 0xC0

const TIMER_CONF: u64 = 0x00;
const TIMER_VALUE: u64 = 0x04;

const REG_INT_RAW: u64 = 0xC0;
const REG_INT_ST: u64 = 0xC4;
const REG_INT_ENA: u64 = 0xC8;
const REG_INT_CLR: u64 = 0xCC;
const REG_CONF: u64 = 0xD0;
/// Version control register — SVD reset 0x1904_0200, fully writable.
const REG_DATE: u64 = 0xFC;
const DATE_RESET: u32 = 0x1904_0200;

/// CONF1 bit 31: DUTY_START_LSCHn. Software sets it to commit the staged DUTY
/// register into the channel's active duty (visible at DUTY_R). Real silicon
/// auto-clears it once the gradual-change machine finishes; firmware that
/// polls it expects it to read back 0, so we clear it immediately on commit.
const CONF1_DUTY_START: u32 = 1 << 31;

/// 19-bit DUTY / DUTY_R field mask (`LEDC_DUTY_LSCHn_V = 0x7FFFF`).
const DUTY_FIELD_MASK: u32 = 0x0007_FFFF;
/// 14-bit HPOINT field mask (`LEDC_HPOINT_LSCHn_V = 0x3FFF`).
const HPOINT_FIELD_MASK: u32 = 0x0000_3FFF;

/// INT_RAW / INT_ST / INT_ENA / INT_CLR bit positions (per header):
///   bits 0..3  = LSTIMERt_OVF (timer t overflow)
///   bits 4..11 = DUTY_CHNG_END_LSCHn (channel n gradual-duty change done)
// Used by tests and available for a future timer-overflow model; the
// non-test build doesn't reference it yet.
#[allow(dead_code)]
const fn timer_ovf_bit(t: usize) -> u32 {
    1 << t
}
const fn duty_chng_end_bit(ch: usize) -> u32 {
    1 << (4 + ch)
}
/// Mask of all interrupt bits we actually model (4 timers + 8 channels).
const INT_MODELED_MASK: u32 = 0x0FFF;

pub struct Esp32s3Ledc {
    /// Interrupt-matrix source id emitted while INT_ST != 0.
    intr_source_id: u32,

    // --- Per-channel config (round-tripped verbatim within field masks) ---
    ch_conf0: [u32; NUM_CHANNELS],
    ch_hpoint: [u32; NUM_CHANNELS],
    /// Staged duty written by firmware (the `duty << 4` value).
    ch_duty: [u32; NUM_CHANNELS],
    ch_conf1: [u32; NUM_CHANNELS],
    /// Committed/active duty, exposed at the read-only DUTY_R register. Updated
    /// from `ch_duty` when CONF1's DUTY_START bit is written.
    ch_duty_r: [u32; NUM_CHANNELS],

    // --- Per-timer config ---
    timer_conf: [u32; NUM_TIMERS],

    // --- Interrupts ---
    int_raw: u32,
    int_ena: u32,

    // --- Global ---
    conf: u32,
    /// DATE version register (0xFC) — reads its SVD reset until written.
    date: u32,
}

impl Esp32s3Ledc {
    /// Construct with the default LEDC interrupt source id (35).
    pub fn new() -> Self {
        Self::new_with_source(LEDC_INTR_SOURCE_ID)
    }

    /// Construct with an explicit interrupt-matrix source id (mirrors the
    /// `uart.rs` pattern where the parent passes the wiring in).
    pub fn new_with_source(intr_source_id: u32) -> Self {
        Self {
            intr_source_id,
            ch_conf0: [0; NUM_CHANNELS],
            ch_hpoint: [0; NUM_CHANNELS],
            ch_duty: [0; NUM_CHANNELS],
            // CONF1 reset: DUTY_INC (bit 30) defaults to 1 per the header
            // (`LEDC_DUTY_INC_LSCHn ... default: 1'b1`). Seed it so reads match
            // silicon before firmware touches the register.
            ch_conf1: [1 << 30; NUM_CHANNELS],
            ch_duty_r: [0; NUM_CHANNELS],
            timer_conf: [0; NUM_TIMERS],
            int_raw: 0,
            int_ena: 0,
            conf: 0,
            date: DATE_RESET,
        }
    }

    /// Active (committed) duty for channel `ch`, as exposed at DUTY_R. The
    /// value is in the hardware `duty << 4` format that firmware staged.
    pub fn active_duty(&self, ch: usize) -> u32 {
        self.ch_duty_r.get(ch).copied().unwrap_or(0) & DUTY_FIELD_MASK
    }

    /// If `offset` falls in the channel block, return `(channel, reg_offset)`.
    fn channel_at(offset: u64) -> Option<(usize, u64)> {
        if (CH_BLOCK_BASE..CH_BLOCK_END).contains(&offset) {
            let ch = ((offset - CH_BLOCK_BASE) / CH_STRIDE) as usize;
            let reg = (offset - CH_BLOCK_BASE) % CH_STRIDE;
            Some((ch, reg))
        } else {
            None
        }
    }

    /// If `offset` falls in the timer block, return `(timer, reg_offset)`.
    fn timer_at(offset: u64) -> Option<(usize, u64)> {
        if (TIMER_BLOCK_BASE..TIMER_BLOCK_END).contains(&offset) {
            let t = ((offset - TIMER_BLOCK_BASE) / TIMER_STRIDE) as usize;
            let reg = (offset - TIMER_BLOCK_BASE) % TIMER_STRIDE;
            Some((t, reg))
        } else {
            None
        }
    }
}

impl Default for Esp32s3Ledc {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Esp32s3Ledc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Esp32s3Ledc")
            .field("conf", &self.conf)
            .field("int_raw", &self.int_raw)
            .field("int_ena", &self.int_ena)
            .field("ch_duty_r", &self.ch_duty_r)
            .field("timer_conf", &self.timer_conf)
            .finish()
    }
}

impl Peripheral for Esp32s3Ledc {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // The ESP-IDF / Arduino LEDC drivers use 32-bit accesses exclusively;
        // stray byte reads are harmless to report as 0.
        Ok(0)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if let Some((ch, reg)) = Self::channel_at(offset) {
            let v = match reg {
                CH_CONF0 => self.ch_conf0[ch],
                CH_HPOINT => self.ch_hpoint[ch],
                CH_DUTY => self.ch_duty[ch],
                CH_CONF1 => self.ch_conf1[ch],
                CH_DUTY_R => self.ch_duty_r[ch],
                _ => 0,
            };
            return Ok(v);
        }
        if let Some((t, reg)) = Self::timer_at(offset) {
            let v = match reg {
                TIMER_CONF => self.timer_conf[t],
                // VALUE is the live counter. We don't run the PWM clock, so the
                // count reads 0; firmware uses it only for diagnostics.
                TIMER_VALUE => 0,
                _ => 0,
            };
            return Ok(v);
        }
        let v = match offset {
            REG_INT_RAW => self.int_raw,
            REG_INT_ST => self.int_raw & self.int_ena,
            REG_INT_ENA => self.int_ena,
            REG_INT_CLR => 0, // write-only semantics; reads as 0
            REG_CONF => self.conf,
            REG_DATE => self.date,
            _ => 0,
        };
        Ok(v)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Byte writes ignored — the drivers write whole words.
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if let Some((ch, reg)) = Self::channel_at(offset) {
            match reg {
                CH_CONF0 => self.ch_conf0[ch] = value,
                CH_HPOINT => self.ch_hpoint[ch] = value & HPOINT_FIELD_MASK,
                CH_DUTY => self.ch_duty[ch] = value & DUTY_FIELD_MASK,
                CH_CONF1 => {
                    // Store CONF1, but the DUTY_START bit auto-clears once the
                    // commit is taken (matches polling firmware).
                    self.ch_conf1[ch] = value & !CONF1_DUTY_START;
                    if value & CONF1_DUTY_START != 0 {
                        // Commit the staged duty into the active/read-back reg.
                        self.ch_duty_r[ch] = self.ch_duty[ch] & DUTY_FIELD_MASK;
                        // Gradual-change machine "completes" instantly here, so
                        // raise the per-channel duty-change-done interrupt.
                        self.int_raw |= duty_chng_end_bit(ch);
                    }
                }
                CH_DUTY_R => {} // read-only
                _ => {}
            }
            return Ok(());
        }
        if let Some((t, reg)) = Self::timer_at(offset) {
            match reg {
                TIMER_CONF => self.timer_conf[t] = value,
                TIMER_VALUE => {} // read-only counter
                _ => {}
            }
            return Ok(());
        }
        match offset {
            // INT_RAW is hardware-set; allow firmware to also force bits (some
            // drivers do for testing). Restrict to modeled bits.
            REG_INT_RAW => self.int_raw = value & INT_MODELED_MASK,
            REG_INT_ENA => self.int_ena = value & INT_MODELED_MASK,
            // W1C: writing 1 clears the matching raw latch.
            REG_INT_CLR => self.int_raw &= !value,
            REG_INT_ST => {} // read-only (INT_RAW & INT_ENA)
            REG_CONF => self.conf = value,
            REG_DATE => self.date = value,
            _ => {} // accept-and-ignore (reserved/timing regs)
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // LEDC is a level interrupt: assert the matrix source for as long as
        // any enabled interrupt is latched. The intmatrix de-asserts when
        // INT_ST returns to 0 (firmware clears via INT_CLR).
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

    /// Absolute-within-window offset of channel `ch` register `reg`.
    fn ch_off(ch: usize, reg: u64) -> u64 {
        CH_BLOCK_BASE + (ch as u64) * CH_STRIDE + reg
    }
    fn timer_off(t: usize, reg: u64) -> u64 {
        TIMER_BLOCK_BASE + (t as u64) * TIMER_STRIDE + reg
    }

    #[test]
    fn channel_conf_and_duty_round_trip() {
        let mut p = Esp32s3Ledc::new();
        // CONF0/HPOINT/DUTY/CONF1 for a couple of channels read back what we
        // wrote (within field masks).
        p.write_u32(ch_off(0, CH_CONF0), 0x0000_0007).unwrap();
        p.write_u32(ch_off(0, CH_HPOINT), 0x1234 & HPOINT_FIELD_MASK)
            .unwrap();
        p.write_u32(ch_off(0, CH_DUTY), 0x0001_2340).unwrap(); // duty<<4
                                                               // CONF1 without DUTY_START — round-trips verbatim.
        p.write_u32(ch_off(0, CH_CONF1), 0x4000_0000).unwrap(); // DUTY_INC only

        assert_eq!(p.read_u32(ch_off(0, CH_CONF0)).unwrap(), 0x0000_0007);
        assert_eq!(
            p.read_u32(ch_off(0, CH_HPOINT)).unwrap(),
            0x1234 & HPOINT_FIELD_MASK
        );
        assert_eq!(p.read_u32(ch_off(0, CH_DUTY)).unwrap(), 0x0001_2340);
        assert_eq!(p.read_u32(ch_off(0, CH_CONF1)).unwrap(), 0x4000_0000);

        // A different channel is independent.
        p.write_u32(ch_off(7, CH_DUTY), 0x0000_00F0).unwrap();
        assert_eq!(p.read_u32(ch_off(7, CH_DUTY)).unwrap(), 0x0000_00F0);
        assert_eq!(p.read_u32(ch_off(0, CH_DUTY)).unwrap(), 0x0001_2340);
    }

    #[test]
    fn duty_field_is_masked_to_19_bits() {
        let mut p = Esp32s3Ledc::new();
        p.write_u32(ch_off(1, CH_DUTY), 0xFFFF_FFFF).unwrap();
        assert_eq!(p.read_u32(ch_off(1, CH_DUTY)).unwrap(), DUTY_FIELD_MASK);
    }

    #[test]
    fn timer_conf_round_trip() {
        let mut p = Esp32s3Ledc::new();
        p.write_u32(timer_off(2, TIMER_CONF), 0x00BE_EF00).unwrap();
        assert_eq!(p.read_u32(timer_off(2, TIMER_CONF)).unwrap(), 0x00BE_EF00);
        // VALUE is read-only and modeled as 0.
        assert_eq!(p.read_u32(timer_off(2, TIMER_VALUE)).unwrap(), 0);
    }

    #[test]
    fn duty_start_commits_staged_duty_into_duty_r() {
        // Mirrors Arduino ledcWrite: stage DUTY, then pulse DUTY_START in CONF1.
        let mut p = Esp32s3Ledc::new();
        // Before commit, DUTY_R reads the reset default (0).
        assert_eq!(p.read_u32(ch_off(3, CH_DUTY_R)).unwrap(), 0);

        p.write_u32(ch_off(3, CH_DUTY), 0x0000_8000).unwrap();
        // Staged value not yet visible at DUTY_R.
        assert_eq!(p.read_u32(ch_off(3, CH_DUTY_R)).unwrap(), 0);

        p.write_u32(ch_off(3, CH_CONF1), CONF1_DUTY_START).unwrap();
        // Now committed.
        assert_eq!(p.read_u32(ch_off(3, CH_DUTY_R)).unwrap(), 0x0000_8000);
        assert_eq!(p.active_duty(3), 0x0000_8000);
    }

    #[test]
    fn duty_start_bit_auto_clears_on_readback() {
        let mut p = Esp32s3Ledc::new();
        p.write_u32(ch_off(0, CH_CONF1), CONF1_DUTY_START | 0x4000_0000)
            .unwrap();
        // DUTY_START reads back 0; the other CONF1 bits persist.
        let c1 = p.read_u32(ch_off(0, CH_CONF1)).unwrap();
        assert_eq!(c1 & CONF1_DUTY_START, 0);
        assert_eq!(c1 & 0x4000_0000, 0x4000_0000);
    }

    #[test]
    fn duty_start_raises_duty_chng_end_interrupt() {
        let mut p = Esp32s3Ledc::new();
        p.write_u32(ch_off(5, CH_DUTY), 0x10).unwrap();
        p.write_u32(ch_off(5, CH_CONF1), CONF1_DUTY_START).unwrap();
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & duty_chng_end_bit(5),
            duty_chng_end_bit(5),
            "committing duty should latch DUTY_CHNG_END for the channel"
        );
    }

    #[test]
    fn int_clr_is_write_one_to_clear() {
        let mut p = Esp32s3Ledc::new();
        p.int_raw = duty_chng_end_bit(0) | timer_ovf_bit(1);
        // Clear only the timer-overflow bit.
        p.write_u32(REG_INT_CLR, timer_ovf_bit(1)).unwrap();
        assert_eq!(p.read_u32(REG_INT_RAW).unwrap(), duty_chng_end_bit(0));
        // INT_CLR reads as 0.
        assert_eq!(p.read_u32(REG_INT_CLR).unwrap(), 0);
    }

    #[test]
    fn int_st_masks_raw_with_ena() {
        let mut p = Esp32s3Ledc::new();
        p.int_raw = duty_chng_end_bit(0) | timer_ovf_bit(0);
        p.write_u32(REG_INT_ENA, timer_ovf_bit(0)).unwrap();
        assert_eq!(p.read_u32(REG_INT_ST).unwrap(), timer_ovf_bit(0));
    }

    #[test]
    fn source_emitted_only_when_enabled_int_asserts() {
        let mut p = Esp32s3Ledc::new();
        // Latch an interrupt that is NOT enabled — no source.
        p.int_raw = timer_ovf_bit(2);
        assert!(p.tick().explicit_irqs.is_none());

        // Enable it — source now emitted while ST != 0 (level behavior).
        p.write_u32(REG_INT_ENA, timer_ovf_bit(2)).unwrap();
        let r = p.tick();
        assert_eq!(r.explicit_irqs, Some(vec![LEDC_INTR_SOURCE_ID]));

        // Clear the latch — source de-asserts.
        p.write_u32(REG_INT_CLR, timer_ovf_bit(2)).unwrap();
        assert!(p.tick().explicit_irqs.is_none());
    }

    #[test]
    fn custom_source_id_is_used() {
        let mut p = Esp32s3Ledc::new_with_source(99);
        p.int_raw = timer_ovf_bit(0);
        p.write_u32(REG_INT_ENA, timer_ovf_bit(0)).unwrap();
        assert_eq!(p.tick().explicit_irqs, Some(vec![99]));
    }

    #[test]
    fn conf_clock_select_round_trips() {
        let mut p = Esp32s3Ledc::new();
        // bit31 CLK_EN + APB_CLK_SEL value.
        p.write_u32(REG_CONF, 0x8000_0001).unwrap();
        assert_eq!(p.read_u32(REG_CONF).unwrap(), 0x8000_0001);
    }

    #[test]
    fn unmapped_offsets_read_zero_and_accept_writes() {
        let mut p = Esp32s3Ledc::new();
        p.write_u32(0xFFC, 0xDEAD_BEEF).unwrap();
        assert_eq!(p.read_u32(0xFFC).unwrap(), 0);
    }

    #[test]
    fn conf1_reset_default_has_duty_inc_set() {
        // Header: LEDC_DUTY_INC_LSCHn default 1'b1.
        let p = Esp32s3Ledc::new();
        assert_eq!(p.read_u32(ch_off(0, CH_CONF1)).unwrap(), 1 << 30);
    }
}
