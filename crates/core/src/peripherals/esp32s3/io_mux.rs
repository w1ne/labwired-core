// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 IO_MUX pad-control peripheral.
//!
//! Mapped at `0x6000_9000`. `PIN_CTRL` sits at offset `0x00`; the 49 per-pad
//! function words `IO_MUX_GPIO0..GPIO48` begin at `0x04` (stride 4, last at
//! `0xC4`), and `DATE` is at `0xFC`. GPIO shares the per-pad bank so the
//! pad-level `FUN_WPU` control behind Arduino `INPUT_PULLUP` is visible
//! outside this MMIO model — see [`Esp32s3IoMux::pad_controls`].
//!
//! ## Register map source
//!
//! Offsets, reset values and bit positions are taken from the vendored
//! ESP32-S3 SVD (`tests/fixtures/svd/esp32s3.svd`, `IO_MUX` peripheral), which
//! is the in-repo oracle for this block and agrees with ESP32-S3 TRM §6.5
//! (`IO_MUX_GPIOn_REG`). Per-pad field layout:
//!
//! ```text
//!   bit[0]      MCU_OE     output enable in sleep mode
//!   bit[1]      SLP_SEL    sleep-mode pad selection
//!   bit[2]      MCU_WPD    pull-down enable in sleep mode
//!   bit[3]      MCU_WPU    pull-up enable in sleep mode
//!   bit[4]      MCU_IE     input enable in sleep mode
//!   bit[7]      FUN_WPD    pull-down enable
//!   bit[8]      FUN_WPU    pull-up enable
//!   bit[9]      FUN_IE     input enable
//!   bits[11:10] FUN_DRV    drive strength
//!   bits[14:12] MCU_SEL    IO MUX function select
//!   bit[15]     FILTER_EN  input filter enable
//! ```
//!
//! This is the SAME layout as the ESP32-C3 (`esp32c3/io_mux.rs`) — both SVDs
//! describe `GPIO%s` identically. The previous header comment in this file
//! claimed `MCU_SEL` at bits[6:4] and `FUN_DRV` at bits[10:7], which overlaps
//! its own `FUN_PU`/`FUN_PD` claim and matches no ESP part; it was never
//! exercised because nothing read the stored word.
//!
//! ## Cold reset
//!
//! The SVD gives one `resetValue` for the whole `GPIO%s` array: `0x0000_0B00`
//! = `FUN_WPU | FUN_IE | FUN_DRV=2`. So every pad comes out of reset with its
//! weak pull-up and input buffer enabled; that is the value seeded here, not a
//! simulator-chosen blanket. The SVD does not model the per-pad variation the
//! S3 datasheet's pin summary documents (e.g. the SPI-flash and strapping pads
//! whose ROM/eFuse configuration differs), and neither does this model — the
//! C3 model has the same limitation for the same reason.

use crate::{Peripheral, SimResult};
use std::sync::{Arc, RwLock};

const PIN_CTRL: u64 = 0x00;
const GPIO0: u64 = 0x04;
const DATE: u64 = 0xFC;
/// `IO_MUX_GPIO0_REG` .. `IO_MUX_GPIO48_REG` (SVD `GPIO%s`, dim = 49).
pub(crate) const PAD_COUNT: usize = 49;
const PIN_CTRL_RESET: u32 = 0x0000_07FF;
/// `FUN_WPU | FUN_IE | FUN_DRV = 2` — the SVD's `GPIO%s` reset value.
const PAD_RESET: u32 = 0x0000_0B00;
const DATE_RESET: u32 = 0x0190_7160;
/// `FUN_WPU` — the pad's weak pull-up, bit 8.
pub(crate) const FUN_WPU: u32 = 1 << 8;

/// The per-pad function words, shared with the GPIO model so a `FUN_WPU` write
/// changes the electrical level an undriven pad reports.
pub(crate) type PadControls = Arc<RwLock<[u32; PAD_COUNT]>>;

#[derive(Debug)]
pub struct Esp32s3IoMux {
    pin_ctrl: u32,
    pads: PadControls,
    date: u32,
}

impl Esp32s3IoMux {
    pub fn new() -> Self {
        Self {
            pin_ctrl: PIN_CTRL_RESET,
            pads: Arc::new(RwLock::new([PAD_RESET; PAD_COUNT])),
            date: DATE_RESET,
        }
    }

    /// Hand out the shared pad-control cell. The GPIO model keeps this `Arc`
    /// and reads `FUN_WPU` from it on every input read; `restore` writes
    /// *through* the same cell rather than replacing it, so a resumed machine
    /// keeps its wiring.
    pub(crate) fn pad_controls(&self) -> PadControls {
        Arc::clone(&self.pads)
    }

    /// Read the current function-select word for `pin` (0..=48).
    /// Returns 0 for out-of-range pins.
    pub fn pin_func(&self, pin: u8) -> u32 {
        self.pads
            .read()
            .expect("ESP32-S3 IO_MUX pad controls poisoned")
            .get(pin as usize)
            .copied()
            .unwrap_or(0)
    }

    /// Is `pin`'s weak pull-up (`FUN_WPU`) enabled?
    pub fn fun_pull_up(&self, pin: u8) -> bool {
        self.pin_func(pin) & FUN_WPU != 0
    }

    fn pad_index(word_off: u64) -> Option<usize> {
        if (GPIO0..GPIO0 + (PAD_COUNT as u64) * 4).contains(&word_off) {
            Some(((word_off - GPIO0) / 4) as usize)
        } else {
            None
        }
    }

    fn read_word(&self, word_off: u64) -> u32 {
        if word_off == PIN_CTRL {
            self.pin_ctrl
        } else if word_off == DATE {
            self.date
        } else if let Some(pin) = Self::pad_index(word_off) {
            self.pads
                .read()
                .expect("ESP32-S3 IO_MUX pad controls poisoned")[pin]
        } else {
            0
        }
    }

    fn write_word(&mut self, word_off: u64, value: u32) {
        if word_off == PIN_CTRL {
            self.pin_ctrl = value;
        } else if word_off == DATE {
            self.date = value;
        } else if let Some(pin) = Self::pad_index(word_off) {
            self.pads
                .write()
                .expect("ESP32-S3 IO_MUX pad controls poisoned")[pin] = value;
        }
        // Offsets outside the architected map are silently dropped.
    }

    fn runtime_state(&self) -> IoMuxSnapshot {
        IoMuxSnapshot {
            pin_ctrl: self.pin_ctrl,
            pads: *self
                .pads
                .read()
                .expect("ESP32-S3 IO_MUX pad controls poisoned"),
            date: self.date,
        }
    }

    fn apply_runtime_state(&mut self, state: IoMuxSnapshot) {
        self.pin_ctrl = state.pin_ctrl;
        // Write THROUGH the shared cell — replacing the `Arc` would orphan the
        // handle GPIO already holds and silently drop every pull-up on resume.
        *self
            .pads
            .write()
            .expect("ESP32-S3 IO_MUX pad controls poisoned") = state.pads;
        self.date = state.date;
    }
}

impl Default for Esp32s3IoMux {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct IoMuxSnapshot {
    pin_ctrl: u32,
    #[serde(with = "serde_pads")]
    pads: [u32; PAD_COUNT],
    date: u32,
}

/// `[u32; 49]` is past serde's built-in array impls, so round-trip it as a
/// slice and re-check the length on the way back in.
mod serde_pads {
    use super::PAD_COUNT;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(pads: &[u32; PAD_COUNT], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(pads.iter())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u32; PAD_COUNT], D::Error> {
        let raw = Vec::<u32>::deserialize(d)?;
        <[u32; PAD_COUNT]>::try_from(raw.as_slice()).map_err(|_| {
            serde::de::Error::invalid_length(raw.len(), &"49 ESP32-S3 IO_MUX pad words")
        })
    }
}

impl Peripheral for Esp32s3IoMux {
    // Pin-function selection is a synchronous register file. It has no
    // elapsed-time state, IRQs, DMA, or scheduled events.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let word = self.read_word(offset & !3);
        Some(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let shift = (offset & 3) * 8;
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << shift);
        word |= (value as u32) << shift;
        self.write_word(word_off, word);
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_word(offset, value);
            Ok(())
        } else {
            for byte in 0..4 {
                self.write(offset + byte, ((value >> (byte * 8)) & 0xFF) as u8)?;
            }
            Ok(())
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self.runtime_state()).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Ok(state) = serde_json::from_value::<IoMuxSnapshot>(state) {
            self.apply_runtime_state(state);
        }
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(&self.runtime_state()).expect("bincode serialize Esp32s3IoMux")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let state: IoMuxSnapshot = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("Esp32s3IoMux snapshot decode: {error}"))
        })?;
        self.apply_runtime_state(state);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_u32(m: &mut Esp32s3IoMux, off: u64, val: u32) {
        for byte in 0..4u64 {
            m.write(off + byte, ((val >> (byte * 8)) & 0xFF) as u8)
                .unwrap();
        }
    }

    fn read_u32(m: &Esp32s3IoMux, off: u64) -> u32 {
        let mut read = 0u32;
        for byte in 0..4u64 {
            read |= (m.read(off + byte).unwrap() as u32) << (byte * 8);
        }
        read
    }

    /// The SVD's cold-reset state, register by register — including the pad
    /// bank starting at 0x04, NOT at the peripheral base.
    #[test]
    fn cold_reset_matches_the_svd_pin_ctrl_pad_bank_and_date() {
        let m = Esp32s3IoMux::new();

        assert_eq!(read_u32(&m, PIN_CTRL), PIN_CTRL_RESET);
        for pin in 0..PAD_COUNT as u64 {
            assert_eq!(
                read_u32(&m, GPIO0 + pin * 4),
                0x0000_0B00,
                "IO_MUX_GPIO{pin}_REG must hold the SVD cold reset"
            );
        }
        assert_eq!(read_u32(&m, DATE), 0x0190_7160);
        // GPIO48 is the last pad word (0xC4); 0xC8 is outside the bank.
        assert_eq!(read_u32(&m, GPIO0 + 48 * 4), 0x0000_0B00);
        assert_eq!(read_u32(&m, GPIO0 + 49 * 4), 0);
    }

    /// `FUN_WPU` (bit 8) is what Arduino `INPUT_PULLUP` sets, and it is already
    /// set at cold reset.
    #[test]
    fn fun_pull_up_reads_bit_eight_of_the_pad_word() {
        let mut m = Esp32s3IoMux::new();
        assert!(m.fun_pull_up(5), "FUN_WPU is set at cold reset");

        // Clear FUN_WPU on GPIO5 only.
        write_u32(&mut m, GPIO0 + 5 * 4, 0x0000_0A00);
        assert!(!m.fun_pull_up(5));
        assert!(m.fun_pull_up(6), "neighbouring pads are untouched");

        // Set it again the way esp-hal does: FUN_WPU | FUN_IE | MCU_SEL=1.
        write_u32(&mut m, GPIO0 + 5 * 4, FUN_WPU | (1 << 9) | (1 << 12));
        assert!(m.fun_pull_up(5));
        // Out-of-range pads have no pad word and therefore no pull-up.
        assert!(!m.fun_pull_up(49));
        assert!(!m.fun_pull_up(200));
    }

    #[test]
    fn pad_words_round_trip_per_pin_without_a_write_mask() {
        let mut m = Esp32s3IoMux::new();
        write_u32(&mut m, GPIO0, 0xABCD_1234);
        write_u32(&mut m, GPIO0 + 4, 0x0000_1B02);
        assert_eq!(m.pin_func(0), 0xABCD_1234);
        assert_eq!(m.pin_func(1), 0x0000_1B02);
        assert_eq!(m.pin_func(2), PAD_RESET, "unwritten pads keep their reset");

        write_u32(&mut m, PIN_CTRL, 0xA5A5_F123);
        write_u32(&mut m, DATE, 0xDEAD_BEEF);
        assert_eq!(read_u32(&m, PIN_CTRL), 0xA5A5_F123);
        assert_eq!(read_u32(&m, DATE), 0xDEAD_BEEF);
    }

    #[test]
    fn out_of_range_offsets_ignore_writes_and_read_zero() {
        let mut m = Esp32s3IoMux::new();
        // 0xC8 is one word past IO_MUX_GPIO48_REG.
        write_u32(&mut m, 0xC8, 0xFFFF_FFFF);
        assert_eq!(read_u32(&m, 0xC8), 0);
        write_u32(&mut m, 0xF0, 0xFFFF_FFFF);
        assert_eq!(read_u32(&m, 0xF0), 0);
    }

    #[test]
    fn word_write_and_byte_write_agree() {
        let mut byte_wise = Esp32s3IoMux::new();
        write_u32(&mut byte_wise, GPIO0 + 7 * 4, 0x1234_5678);

        let mut word_wise = Esp32s3IoMux::new();
        word_wise.write_u32(GPIO0 + 7 * 4, 0x1234_5678).unwrap();

        assert_eq!(byte_wise.pin_func(7), word_wise.pin_func(7));
        assert_eq!(word_wise.pin_func(7), 0x1234_5678);
    }

    #[test]
    fn io_mux_is_inert_for_the_legacy_tick_walk() {
        assert!(
            !Esp32s3IoMux::new().needs_legacy_walk(),
            "IO_MUX only changes at MMIO write time"
        );
    }

    /// A resumed machine must keep its pull-ups: `restore` writes through the
    /// SHARED cell, so the GPIO handle taken before the snapshot still sees the
    /// restored words without any re-wiring step.
    #[test]
    fn runtime_snapshot_restores_pad_words_through_the_shared_cell() {
        let mut original = Esp32s3IoMux::new();
        original.write_u32(PIN_CTRL, 0x0000_0321).unwrap();
        // GPIO6 keeps its pull-up; GPIO7 has it cleared.
        original.write_u32(GPIO0 + 7 * 4, 0x0000_0A00).unwrap();
        original.write_u32(DATE, 0x0BAD_C0DE).unwrap();
        let blob = original.runtime_snapshot();

        let mut resumed = Esp32s3IoMux::new();
        // Take the handle BEFORE the restore, exactly as bus wiring does.
        let mut gpio = crate::peripherals::esp32s3::gpio::Esp32s3Gpio::new();
        gpio.set_pad_controls(resumed.pad_controls());
        resumed.restore_runtime_snapshot(&blob).unwrap();

        assert_eq!(resumed.read_u32(PIN_CTRL).unwrap(), 0x0000_0321);
        assert_eq!(resumed.read_u32(DATE).unwrap(), 0x0BAD_C0DE);
        assert!(resumed.fun_pull_up(6));
        assert!(!resumed.fun_pull_up(7));
        assert_eq!(
            gpio.read_gpio_input(6),
            Some(true),
            "the pre-snapshot GPIO handle sees the restored pull-up"
        );
        assert_eq!(gpio.read_gpio_input(7), Some(false));
    }

    #[test]
    fn json_snapshot_round_trips() {
        let mut original = Esp32s3IoMux::new();
        original.write_u32(GPIO0 + 9 * 4, 0x0000_1B02).unwrap();
        let json = original.snapshot();

        let mut resumed = Esp32s3IoMux::new();
        resumed.restore(json).unwrap();
        assert_eq!(resumed.pin_func(9), 0x0000_1B02);
    }
}
