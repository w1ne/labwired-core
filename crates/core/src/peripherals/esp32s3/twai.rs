// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! TWAI (Two-Wire Automotive Interface, CAN 2.0) controller for ESP32-S3.
//!
//! The TWAI peripheral is an SJA1000-style CAN controller. Its registers are
//! physically 8-bit but, like the SJA1000, are mapped at 32-bit spacing on
//! the ESP32-S3 — each logical register occupies the least-significant byte
//! of a 32-bit word at offsets +0x00, +0x04, +0x08, … This module mirrors the
//! struct field order in
//! `framework-espidf/components/soc/esp32s3/register/soc/twai_struct.h`.
//!
//! Base address: `DR_REG_TWAI_BASE = 0x6002_B000`.
//!
//! ## Register map (offsets, derived from `twai_dev_t` field order)
//!
//! | Offset | Name              | Access | Notes |
//! |-------:|-------------------|--------|-------|
//! | 0x00   | MODE              | R/W    | bit0 reset_mode, bit1 listen_only, bit2 self_test, bit3 acceptance_filter |
//! | 0x04   | CMD               | W      | bit0 tx_req, bit1 abort_tx, bit2 release_rx_buf, bit3 clear_data_overrun, bit4 self_rx_req |
//! | 0x08   | STATUS            | RO     | bit0 rx_buf_st, bit1 data_overrun, bit2 tx_buf_st, bit3 tx_complete, bit4 rx_st, bit5 tx_st, bit6 err_st, bit7 bus_off, bit8 miss_st |
//! | 0x0C   | INTERRUPT         | RO/RC  | bit0 rx_int, bit1 tx_int, bit2 err_int, bit3 data_overrun_int, bit5 err_passive, bit6 arb_lost, bit7 bus_err — READ CLEARS all latched bits |
//! | 0x10   | INTERRUPT_ENABLE  | R/W    | same bit positions as INTERRUPT; gates IRQ emission |
//! | 0x14   | (reserved)        | —      | |
//! | 0x18   | BUS_TIMING_0      | R/W    | bits[12:0] brp, bits[15:14] sjw |
//! | 0x1C   | BUS_TIMING_1      | R/W    | bits[3:0] tseg1, bits[6:4] tseg2, bit7 sam |
//! | 0x20   | (reserved)        | —      | output control, not supported |
//! | 0x24   | (reserved)        | —      | test register, not supported |
//! | 0x28   | (reserved)        | —      | |
//! | 0x2C   | ARB_LOST_CAP      | RO     | bits[4:0] alc |
//! | 0x30   | ERR_CODE_CAP      | RO     | error code capture |
//! | 0x34   | ERR_WARN_LIMIT    | R/W    | bits[7:0] ewl |
//! | 0x38   | RX_ERR_CNT        | R/W    | bits[7:0] rx error counter |
//! | 0x3C   | TX_ERR_CNT        | R/W    | bits[7:0] tx error counter |
//! | 0x40..0x70 | TX/RX buffer  | R/W    | 13 byte-wide shared TX/RX/acceptance-filter regs |
//! | 0x74   | RX_MSG_CNT        | RO     | bits[6:0] rmc |
//! | 0x78   | (reserved)        | —      | |
//! | 0x7C   | CLOCK_DIVIDER     | R/W    | bits[7:0] cd, bit8 clkout enable |
//!
//! ## reset_mode (MODE bit 0)
//!
//! The SJA1000 / TWAI powers up in *reset mode* (a.k.a. configuration mode):
//! `MODE.reset_mode = 1` at reset. Firmware configures BUS_TIMING, the
//! acceptance filter, and error counters while in reset mode, then clears
//! `MODE.reset_mode` to enter *operating mode* where transmit/receive is
//! possible. The bit round-trips: writing 1 re-enters config mode, writing 0
//! enters operating mode. We model the bit as plain storage (round-trip) and
//! seed it to 1 at reset; the CMD transmit path does not gate on it because
//! there is no real CAN bus attached (see below).
//!
//! ## INTERRUPT (0x0C) is READ-AND-CLEAR
//!
//! Reading the INTERRUPT register returns the currently latched interrupt
//! bits and *clears them as a side effect* — exactly the SJA1000 IR
//! behaviour. Because [`Peripheral::read`] takes `&self`, the latch is stored
//! in a [`Cell<u32>`] so the read path can clear it through interior
//! mutability. `Peripheral` is `Send` but not `Sync`, so a `Cell`/`RefCell`
//! is sound here (the same approach `usb_serial_jtag.rs` uses for its RX
//! FIFO). Note the read is byte-granular at the bus level: the byte read that
//! contains bit 0 (the LSB, where all live interrupt bits live) is the one
//! that performs the clear, so a single 8-bit or 32-bit access that touches
//! offset 0x0C clears the whole latch.
//!
//! ## Transmit model (no CAN bus attached)
//!
//! There is no physical CAN bus in the simulator, so a transmit cannot be
//! arbitrated, lost, or acknowledged by a peer. We model `CMD.tx_req` as an
//! instantaneous, always-successful transmission: STATUS.tx_complete and
//! STATUS.tx_buf_st are (re)asserted and the `tx_int` bit is latched in the
//! interrupt latch. If INTERRUPT_ENABLE.tie is set, the next `tick` emits the
//! TWAI interrupt-matrix source. No real RX frames are ever produced, so
//! STATUS.rx_buf_st stays clear unless `CMD.self_rx_req` is used (self-
//! reception loops the just-"sent" frame back, latching rx_int + rx_buf_st).
//!
//! ## Interrupt source
//!
//! `ETS_TWAI_INTR_SOURCE = 37` on the ESP32-S3. Derived from
//! `framework-espidf/components/soc/esp32s3/include/soc/interrupts.h`:
//! the enum starts at `ETS_WIFI_MAC_INTR_SOURCE = 0`, with explicit anchors
//! `ETS_LEDC_INTR_SOURCE = 35`, then `ETS_EFUSE_INTR_SOURCE` (36),
//! `ETS_TWAI_INTR_SOURCE` (37). The numeric value is supplied to
//! [`Esp32s3Twai::new`] so the integration layer can bind it to the interrupt
//! matrix.

use std::cell::Cell;

use crate::{Peripheral, PeripheralTickResult, SimResult};

// ── MODE (0x00) bits ──
const MODE_RESET: u32 = 1 << 0;
const MODE_LISTEN_ONLY: u32 = 1 << 1;
const MODE_SELF_TEST: u32 = 1 << 2;
const MODE_ACCEPTANCE_FILTER: u32 = 1 << 3;
const MODE_MASK: u32 = MODE_RESET | MODE_LISTEN_ONLY | MODE_SELF_TEST | MODE_ACCEPTANCE_FILTER;

// ── CMD (0x04) bits ──
const CMD_TX_REQ: u32 = 1 << 0;
const CMD_ABORT_TX: u32 = 1 << 1;
const CMD_RELEASE_RX_BUF: u32 = 1 << 2;
const CMD_CLEAR_DATA_OVERRUN: u32 = 1 << 3;
const CMD_SELF_RX_REQ: u32 = 1 << 4;

// ── STATUS (0x08) bits ──
const STATUS_RX_BUF: u32 = 1 << 0;
const STATUS_DATA_OVERRUN: u32 = 1 << 1;
const STATUS_TX_BUF: u32 = 1 << 2;
const STATUS_TX_COMPLETE: u32 = 1 << 3;
#[allow(dead_code)]
const STATUS_RX_ST: u32 = 1 << 4;
#[allow(dead_code)]
const STATUS_TX_ST: u32 = 1 << 5;
#[allow(dead_code)]
const STATUS_ERR_ST: u32 = 1 << 6;
#[allow(dead_code)]
const STATUS_BUS_OFF: u32 = 1 << 7;

// ── INTERRUPT (0x0C) / INTERRUPT_ENABLE (0x10) bits (shared positions) ──
const INT_RX: u32 = 1 << 0;
const INT_TX: u32 = 1 << 1;
#[allow(dead_code)]
const INT_ERR: u32 = 1 << 2;
#[allow(dead_code)]
const INT_DATA_OVERRUN: u32 = 1 << 3;
#[allow(dead_code)]
const INT_ERR_PASSIVE: u32 = 1 << 5;
#[allow(dead_code)]
const INT_ARB_LOST: u32 = 1 << 6;
#[allow(dead_code)]
const INT_BUS_ERR: u32 = 1 << 7;
/// All interrupt bits that physically exist (reserved bits 4 masked off).
const INT_MASK: u32 = 0b1110_1111;

#[derive(Debug)]
pub struct Esp32s3Twai {
    /// Interrupt-matrix source ID this controller drives (ETS_TWAI_INTR_SOURCE).
    source_id: u32,

    // ── Round-tripped config registers ──
    /// MODE (0x00). Powers up with reset_mode (bit0) set.
    mode: u32,
    /// INTERRUPT_ENABLE (0x10). Gates which latched bits emit an IRQ.
    int_enable: u32,
    /// BUS_TIMING_0 (0x18).
    bus_timing_0: u32,
    /// BUS_TIMING_1 (0x1C).
    bus_timing_1: u32,
    /// ERR_WARN_LIMIT (0x34).
    err_warn_limit: u32,
    /// RX_ERR_CNT (0x38).
    rx_err_cnt: u32,
    /// TX_ERR_CNT (0x3C).
    tx_err_cnt: u32,
    /// CLOCK_DIVIDER (0x7C).
    clock_divider: u32,
    /// TX/RX buffer + acceptance filter shared region (0x40..0x73), 13 bytes.
    tx_rx_buffer: [u8; 13],

    // ── STATUS (0x08) ──
    status: u32,

    /// RX message counter (0x74), bits[6:0]. Bumped on self-reception.
    rx_msg_count: u32,

    /// Latched interrupt bits (INTERRUPT, 0x0C). Read-and-clear via interior
    /// mutability since `Peripheral::read` is `&self`.
    int_latch: Cell<u32>,
}

impl Esp32s3Twai {
    /// Create a TWAI controller bound to interrupt-matrix `source_id`
    /// (ETS_TWAI_INTR_SOURCE = 37 on the ESP32-S3). Seeds SJA1000 reset
    /// defaults: MODE.reset_mode = 1, STATUS.tx_buf_st = 1 (TX buffer
    /// released / ready), STATUS.tx_complete = 1.
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            mode: MODE_RESET,
            int_enable: 0,
            bus_timing_0: 0,
            bus_timing_1: 0,
            err_warn_limit: 96, // SJA1000 reset default for EWL.
            rx_err_cnt: 0,
            tx_err_cnt: 0,
            clock_divider: 0,
            tx_rx_buffer: [0; 13],
            status: STATUS_TX_BUF | STATUS_TX_COMPLETE,
            rx_msg_count: 0,
            int_latch: Cell::new(0),
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.mode,
            // 0x04 CMD is write-only; reads as 0 on SJA1000.
            0x08 => self.status,
            0x0C => {
                // INTERRUPT: read-and-clear. Return latched bits, then clear.
                let latched = self.int_latch.get() & INT_MASK;
                self.int_latch.set(0);
                latched
            }
            0x10 => self.int_enable,
            0x18 => self.bus_timing_0,
            0x1C => self.bus_timing_1,
            0x34 => self.err_warn_limit,
            0x38 => self.rx_err_cnt,
            0x3C => self.tx_err_cnt,
            0x40..=0x70 => {
                let idx = ((offset - 0x40) / 4) as usize;
                self.tx_rx_buffer.get(idx).copied().unwrap_or(0) as u32
            }
            0x74 => self.rx_msg_count & 0x7F,
            0x7C => self.clock_divider,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.mode = value & MODE_MASK,
            0x04 => self.exec_command(value),
            // 0x08 STATUS is read-only.
            // 0x0C INTERRUPT is read-only (read-clears); ignore writes.
            0x10 => self.int_enable = value & INT_MASK,
            0x18 => self.bus_timing_0 = value & 0xFFFF,
            0x1C => self.bus_timing_1 = value & 0xFF,
            0x34 => self.err_warn_limit = value & 0xFF,
            0x38 => self.rx_err_cnt = value & 0xFF,
            0x3C => self.tx_err_cnt = value & 0xFF,
            0x40..=0x70 => {
                let idx = ((offset - 0x40) / 4) as usize;
                if let Some(slot) = self.tx_rx_buffer.get_mut(idx) {
                    *slot = (value & 0xFF) as u8;
                }
            }
            // 0x74 RX_MSG_CNT is read-only.
            0x7C => self.clock_divider = value & 0x1FF,
            _ => {}
        }
    }

    /// Execute a CMD (0x04) write per SJA1000 semantics.
    fn exec_command(&mut self, cmd: u32) {
        // self_rx_req: loop the frame back to RX (self-test / self-reception).
        if cmd & CMD_SELF_RX_REQ != 0 {
            self.complete_tx();
            // Self-reception delivers the frame into the RX buffer.
            self.status |= STATUS_RX_BUF;
            self.rx_msg_count = (self.rx_msg_count + 1) & 0x7F;
            self.latch_int(INT_RX);
        } else if cmd & CMD_TX_REQ != 0 {
            // No CAN bus attached → transmission completes immediately.
            self.complete_tx();
        }

        if cmd & CMD_ABORT_TX != 0 {
            // Abort a pending transmission: the controller returns to the
            // tx-buffer-available / tx-complete state without latching tx_int
            // (SJA1000 sets tx_complete but suppresses the TX interrupt on a
            // successful abort).
            self.status |= STATUS_TX_BUF | STATUS_TX_COMPLETE;
        }

        if cmd & CMD_RELEASE_RX_BUF != 0 {
            // Release the RX buffer: clear rx_buf_st; decrement the message
            // counter; if more messages remain, rx_buf_st stays asserted.
            if self.rx_msg_count > 0 {
                self.rx_msg_count -= 1;
            }
            if self.rx_msg_count == 0 {
                self.status &= !STATUS_RX_BUF;
            }
        }

        if cmd & CMD_CLEAR_DATA_OVERRUN != 0 {
            self.status &= !STATUS_DATA_OVERRUN;
        }
    }

    /// Mark a transmission as completed: assert tx_complete + tx_buf_st and
    /// latch tx_int.
    fn complete_tx(&mut self) {
        self.status |= STATUS_TX_COMPLETE | STATUS_TX_BUF;
        self.latch_int(INT_TX);
    }

    /// Latch interrupt bit(s) into the read-and-clear INTERRUPT register.
    fn latch_int(&self, bits: u32) {
        self.int_latch.set(self.int_latch.get() | (bits & INT_MASK));
    }
}

impl Peripheral for Esp32s3Twai {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        // INTERRUPT (0x0C) read-clears the whole latch; restrict the clearing
        // side effect to the byte access that carries the live bits (LSB).
        // For any other byte of 0x0C, return the (already-cleared) value
        // without re-triggering the clear.
        if word_off == 0x0C && byte_off != 0 {
            // Bits 8..31 are reserved/zero; never clear on a high-byte read.
            return Ok(0);
        }
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        // Read-modify-write at word granularity, but avoid going through
        // read_word for 0x0C (which has the read-clear side effect). Config
        // registers live entirely in the LSB, so reconstructing the current
        // word from a side-effect-free snapshot is safe.
        let current = match word_off {
            0x0C => self.int_latch.get(), // never cleared by a write path
            0x08 => self.status,
            0x04 => 0, // CMD reads as 0
            _ => self.read_word(word_off),
        };
        let mut word = current;
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Level-sensitive IRQ delivery: while any latched bit is also enabled,
        // emit the TWAI source. The latch is sticky until firmware reads
        // INTERRUPT (read-clears), mirroring the level-triggered nature of
        // ETS_TWAI_INTR_SOURCE.
        let pending = self.int_latch.get() & self.int_enable & INT_MASK;
        let explicit_irqs = if pending != 0 {
            Some(vec![self.source_id])
        } else {
            None
        };
        PeripheralTickResult {
            explicit_irqs,
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

    const TWAI_SOURCE: u32 = 37;

    fn write_word(t: &mut Esp32s3Twai, off: u64, val: u32) {
        t.write_u32(off, val).unwrap();
    }
    fn read_word(t: &Esp32s3Twai, off: u64) -> u32 {
        t.read_u32(off).unwrap()
    }

    #[test]
    fn reset_defaults() {
        let t = Esp32s3Twai::new(TWAI_SOURCE);
        // SJA1000 powers up in reset (config) mode.
        assert_eq!(t.mode & MODE_RESET, MODE_RESET, "reset_mode set at reset");
        // TX buffer available + tx_complete asserted out of reset.
        assert_eq!(read_word(&t, 0x08) & STATUS_TX_BUF, STATUS_TX_BUF);
        assert_eq!(read_word(&t, 0x08) & STATUS_TX_COMPLETE, STATUS_TX_COMPLETE);
        assert_eq!(read_word(&t, 0x0C), 0, "no interrupts latched at reset");
    }

    #[test]
    fn mode_round_trip_config_vs_operating() {
        let mut t = Esp32s3Twai::new(TWAI_SOURCE);
        // Enter operating mode: clear reset_mode, set listen_only + self_test.
        write_word(&mut t, 0x00, MODE_LISTEN_ONLY | MODE_SELF_TEST);
        let m = read_word(&t, 0x00);
        assert_eq!(m & MODE_RESET, 0, "reset_mode cleared → operating mode");
        assert_eq!(m & MODE_LISTEN_ONLY, MODE_LISTEN_ONLY);
        assert_eq!(m & MODE_SELF_TEST, MODE_SELF_TEST);
        // Re-enter config mode.
        write_word(&mut t, 0x00, MODE_RESET);
        assert_eq!(read_word(&t, 0x00), MODE_RESET);
    }

    #[test]
    fn bus_timing_round_trip() {
        let mut t = Esp32s3Twai::new(TWAI_SOURCE);
        // BTR0: brp=0x1FF, sjw=3 → bits[12:0]=0x1FF, bits[15:14]=3.
        let btr0 = 0x01FF | (0b11 << 14);
        write_word(&mut t, 0x18, btr0);
        assert_eq!(read_word(&t, 0x18), btr0);
        // BTR1: tseg1=0xC, tseg2=0x5, sam=1.
        let btr1 = 0x0C | (0x5 << 4) | (1 << 7);
        write_word(&mut t, 0x1C, btr1);
        assert_eq!(read_word(&t, 0x1C), btr1);
    }

    #[test]
    fn error_counters_and_clock_divider_round_trip() {
        let mut t = Esp32s3Twai::new(TWAI_SOURCE);
        write_word(&mut t, 0x38, 0x55); // RX_ERR_CNT
        write_word(&mut t, 0x3C, 0xAA); // TX_ERR_CNT
        write_word(&mut t, 0x34, 0x7F); // ERR_WARN_LIMIT
        write_word(&mut t, 0x7C, 0x1FF); // CLOCK_DIVIDER (cd + clkout en)
        assert_eq!(read_word(&t, 0x38), 0x55);
        assert_eq!(read_word(&t, 0x3C), 0xAA);
        assert_eq!(read_word(&t, 0x34), 0x7F);
        assert_eq!(read_word(&t, 0x7C), 0x1FF);
    }

    #[test]
    fn tx_buffer_round_trip() {
        let mut t = Esp32s3Twai::new(TWAI_SOURCE);
        for i in 0..13u64 {
            write_word(&mut t, 0x40 + i * 4, (0x10 + i) as u32);
        }
        for i in 0..13u64 {
            assert_eq!(read_word(&t, 0x40 + i * 4), (0x10 + i) as u32);
        }
    }

    #[test]
    fn tx_req_completes_and_latches_tx_int() {
        let mut t = Esp32s3Twai::new(TWAI_SOURCE);
        // Clear tx_complete first to observe it being re-asserted.
        t.status &= !STATUS_TX_COMPLETE;
        write_word(&mut t, 0x04, CMD_TX_REQ);
        let st = read_word(&t, 0x08);
        assert_eq!(
            st & STATUS_TX_COMPLETE,
            STATUS_TX_COMPLETE,
            "tx_complete set"
        );
        assert_eq!(st & STATUS_TX_BUF, STATUS_TX_BUF, "tx_buf_st set");
        // tx_int latched (peek without read-clear via the Cell).
        assert_eq!(t.int_latch.get() & INT_TX, INT_TX, "tx_int latched");
    }

    #[test]
    fn interrupt_register_read_and_clear() {
        let mut t = Esp32s3Twai::new(TWAI_SOURCE);
        write_word(&mut t, 0x04, CMD_TX_REQ); // latches tx_int
                                              // First read returns the latched bit...
        assert_eq!(
            read_word(&t, 0x0C) & INT_TX,
            INT_TX,
            "INTERRUPT returns latched"
        );
        // ...and clears it.
        assert_eq!(read_word(&t, 0x0C), 0, "read-and-clear cleared the latch");
    }

    #[test]
    fn byte_read_of_interrupt_clears_via_lsb_only() {
        let mut t = Esp32s3Twai::new(TWAI_SOURCE);
        write_word(&mut t, 0x04, CMD_TX_REQ);
        // High byte read must NOT clear the latch.
        assert_eq!(t.read(0x0F).unwrap(), 0);
        assert_eq!(
            t.int_latch.get() & INT_TX,
            INT_TX,
            "high-byte read preserved latch"
        );
        // LSB read clears.
        assert_eq!(t.read(0x0C).unwrap() & 0x02, 0x02);
        assert_eq!(t.int_latch.get(), 0, "LSB read cleared latch");
    }

    #[test]
    fn source_emitted_while_enabled_int_asserts() {
        let mut t = Esp32s3Twai::new(TWAI_SOURCE);
        // Enable TX interrupt.
        write_word(&mut t, 0x10, INT_TX);
        // Before any latch → no IRQ.
        assert!(t.tick().explicit_irqs.is_none());
        // Latch tx_int via a transmit request.
        write_word(&mut t, 0x04, CMD_TX_REQ);
        let r = t.tick();
        assert_eq!(r.explicit_irqs.as_deref(), Some(&[TWAI_SOURCE][..]));
        // Level-sensitive: stays asserted on the next tick while latched+enabled.
        assert_eq!(t.tick().explicit_irqs.as_deref(), Some(&[TWAI_SOURCE][..]));
        // Reading INTERRUPT clears the latch → IRQ de-asserts.
        let _ = read_word(&t, 0x0C);
        assert!(
            t.tick().explicit_irqs.is_none(),
            "IRQ de-asserts after read-clear"
        );
    }

    #[test]
    fn int_enable_gates_emission() {
        let mut t = Esp32s3Twai::new(TWAI_SOURCE);
        // Latch tx_int but leave INTERRUPT_ENABLE = 0.
        write_word(&mut t, 0x04, CMD_TX_REQ);
        assert!(
            t.tick().explicit_irqs.is_none(),
            "disabled int does not emit"
        );
        // Latch persists (not read yet); enabling it now emits.
        write_word(&mut t, 0x10, INT_TX);
        assert_eq!(t.tick().explicit_irqs.as_deref(), Some(&[TWAI_SOURCE][..]));
    }

    #[test]
    fn self_rx_req_latches_rx_and_bumps_counter() {
        let mut t = Esp32s3Twai::new(TWAI_SOURCE);
        write_word(&mut t, 0x04, CMD_SELF_RX_REQ);
        assert_eq!(read_word(&t, 0x08) & STATUS_RX_BUF, STATUS_RX_BUF);
        assert_eq!(read_word(&t, 0x74), 1, "rx message counter bumped");
        // Both tx_int (loopback transmit) and rx_int latched.
        assert_eq!(t.int_latch.get() & INT_RX, INT_RX);
        assert_eq!(t.int_latch.get() & INT_TX, INT_TX);
        // Release the RX buffer clears rx_buf_st and decrements the counter.
        write_word(&mut t, 0x04, CMD_RELEASE_RX_BUF);
        assert_eq!(read_word(&t, 0x08) & STATUS_RX_BUF, 0);
        assert_eq!(read_word(&t, 0x74), 0);
    }

    #[test]
    fn clear_data_overrun_clears_status() {
        let mut t = Esp32s3Twai::new(TWAI_SOURCE);
        t.status |= STATUS_DATA_OVERRUN;
        write_word(&mut t, 0x04, CMD_CLEAR_DATA_OVERRUN);
        assert_eq!(read_word(&t, 0x08) & STATUS_DATA_OVERRUN, 0);
    }
}
