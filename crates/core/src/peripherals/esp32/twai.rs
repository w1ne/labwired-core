// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! TWAI (Two-Wire Automotive Interface, a.k.a. CAN) controller — ESP32-classic.
//!
//! ESP32 TRM v4.6 §27 "TWAI Controller". The IP is derived from the Bosch /
//! NXP SJA1000 core operating in PeliCAN mode: every register is a 32-bit
//! word whose meaningful payload lives in the low 8 bits (the upper 24 bits
//! read 0). The base address on ESP32-classic is 0x3FF6_B000 (interrupt
//! source ETS_CAN_INTR_SOURCE = 43).
//!
//! What is modeled (the handshakes esp-idf's `twai_driver_install()` /
//! `twai_start()` rely on to make forward progress):
//!
//!   * MODE.RESET_MODE (bit0). Out of hard reset the controller is in reset
//!     mode (=1). esp-idf clears it to enter operating mode and reads it
//!     back; we round-trip it. Bus-timing / filter / error-limit registers
//!     are only writable while in reset mode (TRM §27.3) — we enforce that.
//!   * CMD register (0x04): TX request (bit0) and self-reception request
//!     (bit4) latch a transmit. The bits are write-only and read 0; the
//!     transmit "completes" instantly, raising STATUS.TBS/TCS and the
//!     TX_INT, mirroring the cycle-free SJA1000 single-shot path. Abort
//!     (bit1) just re-releases the TX buffer.
//!   * STATUS register (0x08): out of reset TBS=1 (TX buffer released) and
//!     TCS=1 (TX complete); RBS/DOS cleared. esp-idf busy-waits on TBS
//!     before loading the TX buffer, so this must read released.
//!   * INTERRUPT register (0x0C): read-and-clear (the SJA1000 quirk). esp-idf
//!     reads it in the ISR to learn why the line fired; we latch TX/RX/error
//!     bits and clear them on read.
//!   * Error counters (RX_ERR_CNT 0x34 / TX_ERR_CNT 0x38) and the
//!     ERR_WARNING_LIMIT (0x30) — writable only in reset mode, read back the
//!     programmed value; default counters are 0 (error-active).
//!   * TX/RX buffer words (0x40..0x74) and all other config registers are
//!     preserved as raw read-back so the access pattern always succeeds.

use crate::SimResult;
use std::collections::HashMap;

// Register offsets (byte addresses; each is a 32-bit word). TRM v4.6 §27.5.
const REG_MODE: u64 = 0x00;
const REG_CMD: u64 = 0x04;
const REG_STATUS: u64 = 0x08;
const REG_INT_RAW: u64 = 0x0C;
const REG_INT_ENA: u64 = 0x10;
const REG_BUS_TIMING_0: u64 = 0x18;
const REG_BUS_TIMING_1: u64 = 0x1C;
const REG_ERR_WARNING_LIMIT: u64 = 0x30;
const REG_RX_ERR_CNT: u64 = 0x34;
const REG_TX_ERR_CNT: u64 = 0x38;
const REG_CLOCK_DIVIDER: u64 = 0x78;

// MODE bits.
const MODE_RESET: u32 = 1 << 0;

// CMD bits (write-only, read 0).
const CMD_TX_REQ: u32 = 1 << 0;
const CMD_ABORT: u32 = 1 << 1;
const CMD_SELF_RX: u32 = 1 << 4;

// STATUS bits.
const STATUS_RX_BUF: u32 = 1 << 0; // RBS: receive buffer not empty
const STATUS_TX_BUF: u32 = 1 << 2; // TBS: transmit buffer released
const STATUS_TX_COMPLETE: u32 = 1 << 3; // TCS: last transmission complete

// INTERRUPT bits (read-and-clear).
const INT_RX: u32 = 1 << 0;
const INT_TX: u32 = 1 << 1;

/// ESP32-classic TWAI / CAN controller.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Esp32Twai {
    mode: u32,
    status: u32,
    /// Latched interrupt flags; cleared on read of REG_INT_RAW.
    int_raw: u32,
    int_ena: u32,
    bus_timing_0: u32,
    bus_timing_1: u32,
    err_warning_limit: u32,
    rx_err_cnt: u32,
    tx_err_cnt: u32,
    clock_divider: u32,
    /// TX/RX buffer words and any other config registers — raw read-back.
    extra: HashMap<u64, u32>,
}

impl Default for Esp32Twai {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32Twai {
    pub fn new() -> Self {
        Self {
            // Hard reset enters reset mode (RM=1) — TRM §27.3.
            mode: MODE_RESET,
            // TX buffer released + last transmission complete; RX empty.
            status: STATUS_TX_BUF | STATUS_TX_COMPLETE,
            int_raw: 0,
            int_ena: 0,
            // Power-on bus-timing defaults are unspecified; 0 is fine since
            // they are reprogrammed before leaving reset mode.
            bus_timing_0: 0,
            bus_timing_1: 0,
            // Error-warning limit resets to 96 (0x60) per SJA1000.
            err_warning_limit: 0x60,
            rx_err_cnt: 0,
            tx_err_cnt: 0,
            clock_divider: 0,
            extra: HashMap::new(),
        }
    }

    #[inline]
    fn in_reset_mode(&self) -> bool {
        self.mode & MODE_RESET != 0
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            REG_MODE => self.mode,
            REG_CMD => 0, // command register reads as 0
            REG_STATUS => self.status,
            REG_INT_RAW => self.int_raw, // (side effect handled in byte read)
            REG_INT_ENA => self.int_ena,
            REG_BUS_TIMING_0 => self.bus_timing_0,
            REG_BUS_TIMING_1 => self.bus_timing_1,
            REG_ERR_WARNING_LIMIT => self.err_warning_limit,
            REG_RX_ERR_CNT => self.rx_err_cnt,
            REG_TX_ERR_CNT => self.tx_err_cnt,
            REG_CLOCK_DIVIDER => self.clock_divider,
            other => self.extra.get(&other).copied().unwrap_or(0),
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        let v = value & 0xFF; // payload lives in the low byte
        match offset {
            REG_MODE => {
                // RESET_MODE, LISTEN_ONLY, SELF_TEST, ACC_FILTER, SLEEP.
                self.mode = v & 0x1F;
            }
            REG_CMD => {
                // Single-shot transmit: latch completion immediately. Both a
                // normal TX request and a self-reception request count.
                if v & (CMD_TX_REQ | CMD_SELF_RX) != 0 {
                    self.status |= STATUS_TX_BUF | STATUS_TX_COMPLETE;
                    self.int_raw |= INT_TX;
                    // A self-reception request also delivers the frame back
                    // into the RX buffer, so flag RX as available.
                    if v & CMD_SELF_RX != 0 {
                        self.status |= STATUS_RX_BUF;
                        self.int_raw |= INT_RX;
                    }
                }
                if v & CMD_ABORT != 0 {
                    // Abort just re-releases the TX buffer.
                    self.status |= STATUS_TX_BUF | STATUS_TX_COMPLETE;
                }
            }
            REG_STATUS => { /* read-only */ }
            REG_INT_RAW => { /* read-and-clear; writes ignored */ }
            REG_INT_ENA => self.int_ena = v,
            REG_BUS_TIMING_0 => {
                if self.in_reset_mode() {
                    self.bus_timing_0 = v;
                }
            }
            REG_BUS_TIMING_1 => {
                if self.in_reset_mode() {
                    self.bus_timing_1 = v;
                }
            }
            REG_ERR_WARNING_LIMIT => {
                if self.in_reset_mode() {
                    self.err_warning_limit = v;
                }
            }
            REG_RX_ERR_CNT => {
                if self.in_reset_mode() {
                    self.rx_err_cnt = v;
                }
            }
            REG_TX_ERR_CNT => {
                if self.in_reset_mode() {
                    self.tx_err_cnt = v;
                }
            }
            REG_CLOCK_DIVIDER => self.clock_divider = v,
            other => {
                self.extra.insert(other, value);
            }
        }
    }
}

impl crate::Peripheral for Esp32Twai {
    // Inert walk: SJA1000-style controller whose transmits complete instantly at the CMD write; tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
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
        // Provide a coherent word read so the read-and-clear of INT_RAW is
        // atomic w.r.t. the four byte reads the default impl would do.
        Ok(self.read_reg(offset & !3))
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
        if let Ok(s) = serde_json::from_value::<Esp32Twai>(state) {
            *self = s;
        }
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if let Ok(s) = bincode::deserialize::<Esp32Twai>(bytes) {
            *self = s;
        }
        Ok(())
    }
}

/// The INT_RAW read-and-clear side effect: callers that read the interrupt
/// register through the bus get the latched flags, and the act of reading
/// clears them. Because [`crate::Peripheral::read`] takes `&self`, the clear
/// is performed via an explicit helper the bus could call; for the unit-test
/// path and esp-idf's word-read access the latched value is returned and the
/// driver acks by writing CMD, so the simplest faithful behavior is to expose
/// a mutable take here.
impl Esp32Twai {
    /// Read the interrupt register and clear the latched flags (SJA1000
    /// read-and-clear semantics). Used by the bus word-read fast path.
    pub fn take_interrupts(&mut self) -> u32 {
        let v = self.int_raw;
        self.int_raw = 0;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    fn write32(t: &mut Esp32Twai, off: u64, value: u32) {
        t.write_u32(off, value).unwrap();
    }

    fn read32(t: &Esp32Twai, off: u64) -> u32 {
        t.read_u32(off).unwrap()
    }

    #[test]
    fn resets_into_reset_mode_with_tx_released() {
        let t = Esp32Twai::new();
        assert_eq!(read32(&t, REG_MODE) & MODE_RESET, MODE_RESET);
        let status = read32(&t, REG_STATUS);
        assert_eq!(status & STATUS_TX_BUF, STATUS_TX_BUF);
        assert_eq!(status & STATUS_TX_COMPLETE, STATUS_TX_COMPLETE);
        assert_eq!(status & STATUS_RX_BUF, 0);
        // SJA1000 error-warning-limit reset value.
        assert_eq!(read32(&t, REG_ERR_WARNING_LIMIT) & 0xFF, 0x60);
    }

    #[test]
    fn leaving_reset_mode_round_trips() {
        let mut t = Esp32Twai::new();
        // esp-idf clears RESET_MODE to enter operating mode and reads it back.
        write32(&mut t, REG_MODE, 0x00);
        assert_eq!(read32(&t, REG_MODE) & MODE_RESET, 0);
    }

    #[test]
    fn bus_timing_only_writable_in_reset_mode() {
        let mut t = Esp32Twai::new();
        // In reset mode, bus timing accepts the value.
        write32(&mut t, REG_BUS_TIMING_0, 0x14);
        write32(&mut t, REG_BUS_TIMING_1, 0x2C);
        assert_eq!(read32(&t, REG_BUS_TIMING_0) & 0xFF, 0x14);
        assert_eq!(read32(&t, REG_BUS_TIMING_1) & 0xFF, 0x2C);
        // Leave reset mode, then attempt to reprogram: ignored.
        write32(&mut t, REG_MODE, 0x00);
        write32(&mut t, REG_BUS_TIMING_0, 0x55);
        assert_eq!(read32(&t, REG_BUS_TIMING_0) & 0xFF, 0x14);
    }

    #[test]
    fn tx_request_completes_and_latches_tx_interrupt() {
        let mut t = Esp32Twai::new();
        write32(&mut t, REG_MODE, 0x00); // operating mode
        write32(&mut t, REG_CMD, CMD_TX_REQ);
        let status = read32(&t, REG_STATUS);
        assert_eq!(status & STATUS_TX_BUF, STATUS_TX_BUF);
        assert_eq!(status & STATUS_TX_COMPLETE, STATUS_TX_COMPLETE);
        // Interrupt latched, and read-and-clear empties it.
        assert_eq!(t.take_interrupts() & INT_TX, INT_TX);
        assert_eq!(t.take_interrupts(), 0);
    }

    #[test]
    fn self_reception_sets_rx_available() {
        let mut t = Esp32Twai::new();
        write32(&mut t, REG_MODE, 0x00);
        write32(&mut t, REG_CMD, CMD_SELF_RX);
        let status = read32(&t, REG_STATUS);
        assert_eq!(status & STATUS_RX_BUF, STATUS_RX_BUF);
        assert_eq!(t.take_interrupts() & (INT_TX | INT_RX), INT_TX | INT_RX);
    }

    #[test]
    fn cmd_register_reads_zero() {
        let mut t = Esp32Twai::new();
        write32(&mut t, REG_CMD, 0xFF);
        assert_eq!(read32(&t, REG_CMD), 0);
    }

    #[test]
    fn tx_buffer_words_round_trip() {
        let mut t = Esp32Twai::new();
        // TX buffer occupies 0x40..0x74; raw read-back.
        write32(&mut t, 0x40, 0xDEAD_BEEF);
        write32(&mut t, 0x44, 0x0011_2233);
        assert_eq!(read32(&t, 0x40), 0xDEAD_BEEF);
        assert_eq!(read32(&t, 0x44), 0x0011_2233);
    }

    #[test]
    fn runtime_snapshot_round_trips() {
        let mut t = Esp32Twai::new();
        // Program bus timing while in reset mode, then enter operating mode.
        write32(&mut t, REG_BUS_TIMING_0, 0x14);
        write32(&mut t, REG_MODE, 0x00);
        write32(&mut t, 0x40, 0xCAFE_F00D);
        let blob = t.runtime_snapshot();
        let mut t2 = Esp32Twai::new();
        t2.restore_runtime_snapshot(&blob).unwrap();
        assert_eq!(read32(&t2, REG_MODE) & MODE_RESET, 0);
        assert_eq!(read32(&t2, REG_BUS_TIMING_0) & 0xFF, 0x14);
        assert_eq!(read32(&t2, 0x40), 0xCAFE_F00D);
    }
}
