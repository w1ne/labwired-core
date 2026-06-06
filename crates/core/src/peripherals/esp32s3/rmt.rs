// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RMT (Remote Control / pulse transceiver) peripheral for ESP32-S3.
//!
//! The ESP32-S3 RMT has **4 TX channels (0..3) + 4 RX channels (4..7)** sharing
//! a 384×32 RAM block (RMTMEM) mapped at `DR_REG_RMT_BASE + 0x400`. This module
//! models **only the register block** (`base .. base + 0x400`); the parent maps
//! the symbol RAM separately, so we cap the modeled window at the last register
//! (`RMT_DATE` at offset 0xCC, i.e. a 0xD0-byte register file).
//!
//! ## Register map (verified against ESP-IDF
//! `components/soc/esp32s3/register/soc/rmt_reg.h`, base = DR_REG_RMT_BASE =
//! 0x6001_6000):
//!
//! | Offset | Name                | Notes |
//! |-------:|---------------------|-------|
//! | 0x00..0x1C | CH0..7 DATA      | APB-FIFO data port (RO here; RAM lives at +0x400) |
//! | 0x20..0x2C | CH0..3 CONF0     | TX channel config 0 (tx_start/mem_rd_rst/conf_update/carrier/divider) |
//! | 0x30 | CH4 CONF0             | RX channel config 0 (div/idle_thres/mem_size/carrier) |
//! | 0x34 | CH4 CONF1             | RX channel config 1 (rx_en/mem_owner/filter/conf_update) |
//! | 0x38 | CH5 CONF0 | 0x3C | CH5 CONF1 |
//! | 0x40 | CH6 CONF0 | 0x44 | CH6 CONF1 |
//! | 0x48 | CH7 CONF0 | 0x4C | CH7 CONF1 |
//! | 0x50..0x6C | CH0..7 STATUS    | status (RO) |
//! | 0x70 | INT_RAW               | per-channel raw interrupt bits |
//! | 0x74 | INT_ST                | INT_RAW & INT_ENA (RO) |
//! | 0x78 | INT_ENA               | per-channel interrupt enable |
//! | 0x7C | INT_CLR               | write-1-to-clear |
//! | 0x80..0x8C | CH0..3 CARRIER_DUTY | TX carrier high/low duty |
//! | 0x90..0x9C | CH4..7 RX_CARRIER_RM | RX carrier removal |
//! | 0xA0..0xAC | CH0..3 TX_LIM     | TX threshold / loop config |
//! | 0xB0..0xBC | CH4..7 RX_LIM     | RX threshold |
//! | 0xC0 | SYS_CONF              | clk src/enable, fractional divider, mem clk |
//! | 0xC4 | TX_SIM               | synchronous-TX-start config |
//! | 0xC8 | REF_CNT_RST          | per-channel clock-divider reset (WT) |
//! | 0xCC | DATE                 | version register |
//!
//! ## TX completion model
//!
//! There is no real wire to play RMT symbols out of the simulator. Firmware
//! that uses the RMT TX path follows a *fire-and-wait* pattern: write
//! `TX_START` (and `CONF_UPDATE`) to `CHnCONF0`, then poll `CHn_TX_END` in
//! INT_RAW (or wait on the RMT interrupt) for completion. To make that pattern
//! succeed deterministically we model transmission as **instantaneous**: when a
//! TX channel (0..3) gets a `CHnCONF0` write that asserts `TX_START` (bit 0),
//! we immediately
//!   * latch that channel's `CHn_TX_END` bit in INT_RAW, and
//!   * clear the stored `TX_START` bit (it is a write-trigger `WT` field that
//!     self-clears on real silicon once the FSM consumes it).
//!
//! The interrupt then propagates through INT_ST/INT_ENA exactly like hardware.
//!
//! ## Interrupt source
//!
//! `ETS_RMT_INTR_SOURCE = 40` (counted from `ETS_WIFI_MAC_INTR_SOURCE = 0` in
//! `components/soc/esp32s3/include/soc/interrupts.h`). The numeric value is
//! supplied to `new(source_id)` rather than hard-coded so the parent can rebind
//! it. IRQ delivery is level-sensitive: while `INT_ST != 0` we re-emit the RMT
//! source via `PeripheralTickResult.explicit_irqs` on every tick, matching the
//! SYSTIMER model (keeps the bus aggregator bit asserted until firmware ACKs at
//! the source via INT_CLR).

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// Number of TX channels (0..3).
const TX_CHANNELS: usize = 4;
/// Number of RX channels (4..7).
const RX_CHANNELS: usize = 4;

/// Total channels (0..7) exposing a CHnDATA APB-FIFO port at 0x00..0x1C.
const TOTAL_CHANNELS: usize = 8;
/// RMT per-channel RAM depth on the ESP32-S3 (48 × 32-bit entries per block).
const RMT_RAM_WORDS: usize = 48;

/// CARRIER_DUTY reset default (ESP32-S3 SVD): high-duty 0x40, low-duty 0x40.
const CARRIER_DUTY_RESET: u32 = 0x0040_0040;
/// RX_STATUS read-only idle/reset constant (ESP32-S3 SVD reset 0x0006_00C0).
const RX_STATUS_IDLE: u32 = 0x0006_00C0;

/// Size of the modeled register block: `RMT_DATE` at 0xCC + 4 = 0xD0.
/// RMTMEM (the symbol RAM) lives separately at base + 0x400 and is NOT modeled
/// here.
pub const RMT_REG_BLOCK_SIZE: u64 = 0xD0;

// ── CHnCONF0 (TX, n=0..3) bit fields ──────────────────────────────────────
/// TX_START — bit 0 (WT, self-clearing): start sending data on the channel.
const TX_START_BIT: u32 = 1 << 0;
/// CONF_UPDATE — bit 24 (WT): config-sync strobe; self-clears.
const TX_CONF_UPDATE_BIT: u32 = 1 << 24;
/// Write-trigger bits in TX CONF0 that self-clear after a write (so they never
/// read back as 1): TX_START(0), MEM_RD_RST(1), APB_MEM_RST(2), AFIFO_RST(23),
/// CONF_UPDATE(24).
const TX_CONF0_WT_MASK: u32 = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 23) | (1 << 24);

// ── CHmCONF1 (RX, m=4..7) write-trigger bits ──────────────────────────────
/// MEM_WR_RST(1), APB_MEM_RST(2), AFIFO_RST(14), CONF_UPDATE(15) self-clear.
const RX_CONF1_WT_MASK: u32 = (1 << 1) | (1 << 2) | (1 << 14) | (1 << 15);

// ── INT_RAW / INT_ST / INT_ENA / INT_CLR bit layout (offset 0x70..0x7C) ────
// TX_END: bits [3:0] for channels 0..3
// ERR (TX): bits [7:4] for channels 0..3
// TX_THR_EVENT: bits [11:8] for channels 0..3
// TX_LOOP: bits [15:12] for channels 0..3
// RX_END: bits [19:16] for channels 4..7
// ERR (RX): bits [23:20] for channels 4..7
// RX_THR_EVENT: bits [27:24] for channels 4..7
// CH3_DMA_ACCESS_FAIL: bit 28, CH7_DMA_ACCESS_FAIL: bit 29
/// Valid interrupt bit mask (bits [29:0]).
const INT_VALID_MASK: u32 = 0x3FFF_FFFF;

/// INT_RAW bit index of `CHn_TX_END` for TX channel `n` (0..3).
const fn tx_end_bit(n: usize) -> u32 {
    1 << n
}

#[derive(Debug)]
pub struct Esp32s3Rmt {
    /// Interrupt-matrix source ID (ETS_RMT_INTR_SOURCE = 40 on ESP32-S3).
    source_id: u32,

    /// CH0..3 CONF0 (TX channels). Offsets 0x20, 0x24, 0x28, 0x2C.
    tx_conf0: [u32; TX_CHANNELS],
    /// CH4..7 CONF0 (RX channels). Offsets 0x30, 0x38, 0x40, 0x48.
    rx_conf0: [u32; RX_CHANNELS],
    /// CH4..7 CONF1 (RX channels). Offsets 0x34, 0x3C, 0x44, 0x4C.
    rx_conf1: [u32; RX_CHANNELS],

    /// CH0..3 TX_LIM. Offsets 0xA0..0xAC.
    tx_lim: [u32; TX_CHANNELS],
    /// CH4..7 RX_LIM. Offsets 0xB0..0xBC.
    rx_lim: [u32; RX_CHANNELS],

    /// CH0..3 CARRIER_DUTY. Offsets 0x80, 0x84, 0x88, 0x8C. Fully writable TX
    /// carrier high(31:16)/low(15:0) duty; reset 0x0040_0040.
    carrier_duty: [u32; TX_CHANNELS],
    /// CH4..7 RX_CARRIER_RM. Offsets 0x90, 0x94, 0x98, 0x9C. Fully writable RX
    /// carrier-removal config; reset 0.
    rx_carrier_rm: [u32; RX_CHANNELS],

    /// Per-channel APB-FIFO RAM shadow for CHnDATA (0x00..0x1C, channels 0..7).
    ///
    /// This models the **APB-FIFO access path** into each channel's RMT RAM
    /// (distinct from RMTMEM at base+0x400, which the parent maps separately).
    /// Writing CHnDATA pushes a 32-bit RMT entry via an auto-incrementing write
    /// pointer (`ch_wr`); the index wraps mod [`RMT_RAM_WORDS`] so arbitrary
    /// input never overflows nor panics.
    ///
    /// Reads are **non-destructive** and return the most-recently-written word.
    /// A pop-on-read FIFO is impossible here because `read_word(&self)` is
    /// immutable (shared by the `Peripheral::read` trait path and every other
    /// register), so a read pointer could not advance; a destructive read would
    /// also corrupt FIFO state on the byte-RMW write path (which calls
    /// `read_word` then `write_word`). The non-destructive, write-pointer-only
    /// design is both faithful (write-then-read returns the written word, as the
    /// APB-FIFO does) and robust to byte-level access.
    ///
    /// Note: the TX engine in this model is fire-and-complete and does NOT
    /// replay buffered symbols — a known FSM-depth limit consistent with the
    /// model's functional-not-cycle-exact scope.
    ch_mem: [[u32; RMT_RAM_WORDS]; TOTAL_CHANNELS],
    /// Per-channel auto-incrementing write pointer for the CHnDATA APB-FIFO.
    ch_wr: [usize; TOTAL_CHANNELS],

    /// SYS_CONF (0xC0): clk src/enable, fractional divider, mem clk.
    sys_conf: u32,
    /// TX_SIM (0xC4): synchronous-TX-start config.
    tx_sim: u32,

    /// INT_RAW (0x70). Sticky per-channel raw interrupt bits.
    int_raw: u32,
    /// INT_ENA (0x78). Per-channel interrupt enable.
    int_ena: u32,
}

impl Esp32s3Rmt {
    /// Construct the RMT with the given interrupt-matrix `source_id`.
    /// On ESP32-S3 this is `ETS_RMT_INTR_SOURCE = 40`.
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            // TX CONF0 reset default: DIV_CNT=2 (bits[15:8]), MEM_SIZE=1
            // (bits[19:16]), CARRIER_EFF_EN=1 (20), CARRIER_EN=1 (21),
            // CARRIER_OUT_LV=1 (22) → 0x0017_0200.
            tx_conf0: [0x0017_0200; TX_CHANNELS],
            // RX CONF0 reset default: DIV_CNT=2 (bits[7:0]), IDLE_THRES=32767
            // (bits[22:8] = 0x7FFF<<8), MEM_SIZE=1 (bits[27:24]),
            // CARRIER_EN=1 (28), CARRIER_OUT_LV=1 (29).
            rx_conf0: [(2) | (0x7FFF << 8) | (1 << 24) | (1 << 28) | (1 << 29); RX_CHANNELS],
            // RX CONF1 reset default: MEM_OWNER=1 (bit 3), RX_FILTER_THRES=15
            // (bits[12:5] = 15<<5 = 0x1E0) → 0x1E8.
            rx_conf1: [(1 << 3) | (15 << 5); RX_CHANNELS],
            // TX_LIM / RX_LIM reset default: 128 (bits[8:0]).
            tx_lim: [128; TX_CHANNELS],
            rx_lim: [128; RX_CHANNELS],
            // CARRIER_DUTY reset 0x0040_0040; RX_CARRIER_RM reset 0.
            carrier_duty: [CARRIER_DUTY_RESET; TX_CHANNELS],
            rx_carrier_rm: [0; RX_CHANNELS],
            // CHnDATA APB-FIFO RAM shadow + write pointers start empty/zeroed.
            ch_mem: [[0; RMT_RAM_WORDS]; TOTAL_CHANNELS],
            ch_wr: [0; TOTAL_CHANNELS],
            // SYS_CONF reset default: SCLK_DIV_NUM=1 (bits[11:4] = 1<<4=0x10),
            // SCLK_SEL=1 (bits[25:24]), SCLK_ACTIVE=1 (26).
            sys_conf: (1 << 4) | (1 << 24) | (1 << 26),
            tx_sim: 0,
            int_raw: 0,
            int_ena: 0,
        }
    }

    /// Map an MMIO word offset to the TX channel index whose CONF0 it is, if
    /// any. TX CONF0 offsets are 0x20, 0x24, 0x28, 0x2C (channels 0..3).
    fn tx_conf0_index(offset: u64) -> Option<usize> {
        match offset {
            0x20 => Some(0),
            0x24 => Some(1),
            0x28 => Some(2),
            0x2C => Some(3),
            _ => None,
        }
    }

    /// RX channel CONF0 offsets: 0x30, 0x38, 0x40, 0x48 (channels 4..7).
    fn rx_conf0_index(offset: u64) -> Option<usize> {
        match offset {
            0x30 => Some(0),
            0x38 => Some(1),
            0x40 => Some(2),
            0x48 => Some(3),
            _ => None,
        }
    }

    /// RX channel CONF1 offsets: 0x34, 0x3C, 0x44, 0x4C (channels 4..7).
    fn rx_conf1_index(offset: u64) -> Option<usize> {
        match offset {
            0x34 => Some(0),
            0x3C => Some(1),
            0x44 => Some(2),
            0x4C => Some(3),
            _ => None,
        }
    }

    /// TX_LIM offsets: 0xA0..0xAC (channels 0..3).
    fn tx_lim_index(offset: u64) -> Option<usize> {
        match offset {
            0xA0 => Some(0),
            0xA4 => Some(1),
            0xA8 => Some(2),
            0xAC => Some(3),
            _ => None,
        }
    }

    /// RX_LIM offsets: 0xB0..0xBC (channels 4..7).
    fn rx_lim_index(offset: u64) -> Option<usize> {
        match offset {
            0xB0 => Some(0),
            0xB4 => Some(1),
            0xB8 => Some(2),
            0xBC => Some(3),
            _ => None,
        }
    }

    /// CARRIER_DUTY offsets: 0x80, 0x84, 0x88, 0x8C (TX channels 0..3).
    fn carrier_duty_index(offset: u64) -> Option<usize> {
        match offset {
            0x80 => Some(0),
            0x84 => Some(1),
            0x88 => Some(2),
            0x8C => Some(3),
            _ => None,
        }
    }

    /// RX_CARRIER_RM offsets: 0x90, 0x94, 0x98, 0x9C (RX channels 4..7).
    fn rx_carrier_rm_index(offset: u64) -> Option<usize> {
        match offset {
            0x90 => Some(0),
            0x94 => Some(1),
            0x98 => Some(2),
            0x9C => Some(3),
            _ => None,
        }
    }

    /// RX_STATUS offsets: 0x60, 0x64, 0x68, 0x6C (RX channels 4..7, RO).
    fn rx_status_index(offset: u64) -> Option<usize> {
        match offset {
            0x60 => Some(0),
            0x64 => Some(1),
            0x68 => Some(2),
            0x6C => Some(3),
            _ => None,
        }
    }

    /// TX_STATUS offsets: 0x50, 0x54, 0x58, 0x5C (TX channels 0..3, RO).
    fn tx_status_index(offset: u64) -> Option<usize> {
        match offset {
            0x50 => Some(0),
            0x54 => Some(1),
            0x58 => Some(2),
            0x5C => Some(3),
            _ => None,
        }
    }

    /// CHnDATA APB-FIFO port offsets: 0x00..0x1C → channel 0..7 (offset / 4).
    fn chndata_index(offset: u64) -> Option<usize> {
        match offset {
            0x00 | 0x04 | 0x08 | 0x0C | 0x10 | 0x14 | 0x18 | 0x1C => Some((offset / 4) as usize),
            _ => None,
        }
    }

    /// INT_ST = INT_RAW & INT_ENA (masked-interrupt status).
    fn int_st(&self) -> u32 {
        self.int_raw & self.int_ena
    }

    fn read_word(&self, offset: u64) -> u32 {
        if let Some(i) = Self::tx_conf0_index(offset) {
            return self.tx_conf0[i];
        }
        if let Some(i) = Self::rx_conf0_index(offset) {
            return self.rx_conf0[i];
        }
        if let Some(i) = Self::rx_conf1_index(offset) {
            return self.rx_conf1[i];
        }
        if let Some(i) = Self::tx_lim_index(offset) {
            return self.tx_lim[i];
        }
        if let Some(i) = Self::rx_lim_index(offset) {
            return self.rx_lim[i];
        }
        if let Some(i) = Self::carrier_duty_index(offset) {
            return self.carrier_duty[i];
        }
        if let Some(i) = Self::rx_carrier_rm_index(offset) {
            return self.rx_carrier_rm[i];
        }
        // RX_STATUS (RO): return the SVD idle/reset constant per RX channel.
        if Self::rx_status_index(offset).is_some() {
            return RX_STATUS_IDLE;
        }
        // TX_STATUS (RO): the TX engine is fire-and-complete, so TX is always
        // idle → reads the SVD reset value 0 (no nonzero state to surface).
        if Self::tx_status_index(offset).is_some() {
            return 0;
        }
        // CHnDATA APB-FIFO port (RO half here): non-destructive read of the
        // most-recently-written word (see `ch_mem` doc). Empty channel reads 0.
        if let Some(ch) = Self::chndata_index(offset) {
            return if self.ch_wr[ch] == 0 {
                0
            } else {
                self.ch_mem[ch][(self.ch_wr[ch] - 1) % RMT_RAM_WORDS]
            };
        }
        match offset {
            0x70 => self.int_raw,
            0x74 => self.int_st(),
            0x78 => self.int_ena,
            // 0x7C INT_CLR is W1C; reads as 0.
            0x7C => 0,
            0xC0 => self.sys_conf,
            0xC4 => self.tx_sim,
            // RMT_DATE (0xCC): version register. Authoritative ESP32-S3 SVD
            // reset = 0x0210_1181 (the HW read is clock-gated to 0 on a bare
            // board, so the SVD governs; this is a pure version/ID register
            // with no functional branch in any driver).
            0xCC => 0x0210_1181,
            // REF_CNT_RST (WT) — not separately modeled; read back as 0.
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        // ── TX channel CONF0 (0..3): handle the fire-and-wait TX model ──
        if let Some(i) = Self::tx_conf0_index(offset) {
            // Store everything except the self-clearing write-trigger bits.
            self.tx_conf0[i] = value & !TX_CONF0_WT_MASK;
            // TX completion: a write asserting TX_START (optionally with
            // CONF_UPDATE) starts (and, in this model, instantly finishes) a
            // transmission. Latch CHn_TX_END; the write-trigger bits already
            // self-cleared above.
            if value & TX_START_BIT != 0 {
                self.int_raw |= tx_end_bit(i);
            }
            // CONF_UPDATE alone (without TX_START) just syncs config — no
            // completion event. (Bit is consumed regardless; nothing to do.)
            let _ = TX_CONF_UPDATE_BIT;
            return;
        }
        if let Some(i) = Self::rx_conf0_index(offset) {
            self.rx_conf0[i] = value;
            return;
        }
        if let Some(i) = Self::rx_conf1_index(offset) {
            // Drop self-clearing write-trigger bits on readback.
            self.rx_conf1[i] = value & !RX_CONF1_WT_MASK;
            return;
        }
        if let Some(i) = Self::tx_lim_index(offset) {
            self.tx_lim[i] = value;
            return;
        }
        if let Some(i) = Self::rx_lim_index(offset) {
            self.rx_lim[i] = value;
            return;
        }
        // CARRIER_DUTY / RX_CARRIER_RM: fully writable, store verbatim.
        if let Some(i) = Self::carrier_duty_index(offset) {
            self.carrier_duty[i] = value;
            return;
        }
        if let Some(i) = Self::rx_carrier_rm_index(offset) {
            self.rx_carrier_rm[i] = value;
            return;
        }
        // RX_STATUS / TX_STATUS are read-only: ignore writes (fall through to
        // their index checks so they don't hit the catch-all as writable).
        if Self::rx_status_index(offset).is_some() || Self::tx_status_index(offset).is_some() {
            return;
        }
        // CHnDATA APB-FIFO port: push the 32-bit entry into the channel's RAM
        // shadow via the auto-incrementing write pointer (index wraps mod
        // RMT_RAM_WORDS so arbitrary input never overflows).
        if let Some(ch) = Self::chndata_index(offset) {
            let slot = self.ch_wr[ch] % RMT_RAM_WORDS;
            self.ch_mem[ch][slot] = value;
            self.ch_wr[ch] = self.ch_wr[ch].wrapping_add(1);
            return;
        }
        match offset {
            // INT_RAW (0x70) is R/WTC/SS on silicon (write-1-to-clear per bit).
            // Firmware normally clears via INT_CLR, but honor W1C here too.
            0x70 => self.int_raw &= !(value & INT_VALID_MASK),
            // INT_ST (0x74) is read-only.
            0x78 => self.int_ena = value & INT_VALID_MASK,
            // INT_CLR (0x7C): write-1-to-clear the matching INT_RAW bits.
            0x7C => self.int_raw &= !(value & INT_VALID_MASK),
            0xC0 => self.sys_conf = value,
            0xC4 => self.tx_sim = value,
            // REF_CNT_RST (0xC8, write-trigger) / DATE (0xCC, RO) — no mutable
            // state to update here; ignore. (CHnDATA, STATUS, CARRIER_DUTY and
            // RX_CARRIER_RM are handled by the early returns above.)
            _ => {}
        }
    }
}

impl Peripheral for Esp32s3Rmt {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        // Read-modify-write the byte lane, then re-run word-write semantics so
        // the WT/TX-completion logic fires once the strobe byte lands.
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    /// Level-sensitive IRQ delivery: while any enabled interrupt is pending
    /// (INT_ST != 0), re-emit the RMT source on every tick so the bus
    /// aggregator keeps the CPU's pending line asserted until firmware ACKs at
    /// the source (INT_CLR). Mirrors the SYSTIMER model.
    fn tick(&mut self) -> PeripheralTickResult {
        let explicit_irqs = if self.int_st() != 0 {
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

    /// ETS_RMT_INTR_SOURCE on ESP32-S3 (counted from
    /// ETS_WIFI_MAC_INTR_SOURCE = 0 in soc/interrupts.h).
    const RMT_SOURCE: u32 = 40;

    fn rmt() -> Esp32s3Rmt {
        Esp32s3Rmt::new(RMT_SOURCE)
    }

    #[test]
    fn reset_defaults_seeded() {
        let r = rmt();
        // TX CONF0 default 0x0017_0200 (DIV_CNT=2, MEM_SIZE=1, carrier bits).
        assert_eq!(r.read_word(0x20), 0x0017_0200);
        assert_eq!(r.read_word(0x2C), 0x0017_0200);
        // RX CONF0 default.
        let rx0 = 2u32 | (0x7FFF << 8) | (1 << 24) | (1 << 28) | (1 << 29);
        assert_eq!(r.read_word(0x30), rx0);
        assert_eq!(r.read_word(0x48), rx0);
        // RX CONF1 default 0x1E8.
        assert_eq!(r.read_word(0x34), 0x1E8);
        // TX_LIM / RX_LIM default 128.
        assert_eq!(r.read_word(0xA0), 128);
        assert_eq!(r.read_word(0xB0), 128);
        // SYS_CONF default.
        assert_eq!(r.read_word(0xC0), (1 << 4) | (1 << 24) | (1 << 26));
        // DATE version. Authoritative ESP32-S3 SVD reset = 0x0210_1181 (the
        // HW read is clock-gated to 0 on a bare board, so the SVD governs).
        assert_eq!(r.read_word(0xCC), 0x0210_1181);
    }

    #[test]
    fn conf_round_trip() {
        let mut r = rmt();
        // TX CONF0: write a value with no WT bits set; reads back verbatim.
        // 0x002B_FF00 → DIV_CNT=0xFF, MEM_SIZE=0xB, carrier bits 20..22
        // (bit 23 AFIFO_RST left clear so it is not masked).
        r.write_word(0x24, 0x002B_FF00);
        assert_eq!(r.read_word(0x24), 0x002B_FF00);

        // RX CONF0 round-trips fully.
        r.write_word(0x38, 0xDEAD_BEEF);
        assert_eq!(r.read_word(0x38), 0xDEAD_BEEF);

        // RX CONF1: WT bits (1,2,14,15) self-clear; the rest round-trips.
        // RX_EN(0)+MEM_OWNER(3)+filter_thres + a WT bit (15) → WT drops out.
        let written = (1 << 0) | (1 << 3) | (0xAB << 5) | (1 << 15);
        r.write_word(0x44, written);
        assert_eq!(r.read_word(0x44), written & !RX_CONF1_WT_MASK);

        // SYS_CONF, TX_SIM, TX_LIM, RX_LIM round-trip.
        r.write_word(0xC0, 0x1234_5678);
        assert_eq!(r.read_word(0xC0), 0x1234_5678);
        r.write_word(0xC4, 0x1F);
        assert_eq!(r.read_word(0xC4), 0x1F);
        r.write_word(0xA8, 0x55);
        assert_eq!(r.read_word(0xA8), 0x55);
        r.write_word(0xBC, 0xAA);
        assert_eq!(r.read_word(0xBC), 0xAA);
    }

    #[test]
    fn all_eight_channel_confs_round_trip() {
        let mut r = rmt();
        // TX channels 0..3 CONF0.
        for (i, off) in [0x20u64, 0x24, 0x28, 0x2C].iter().enumerate() {
            let v = 0x0010_0000 | ((i as u32) << 8); // no WT bits
            r.write_word(*off, v);
            assert_eq!(r.read_word(*off), v, "tx conf0 ch{i}");
        }
        // RX channels 4..7 CONF0 + CONF1.
        for (i, (c0, c1)) in [(0x30u64, 0x34u64), (0x38, 0x3C), (0x40, 0x44), (0x48, 0x4C)]
            .iter()
            .enumerate()
        {
            let v0 = 0x0100_0000 | (i as u32);
            r.write_word(*c0, v0);
            assert_eq!(r.read_word(*c0), v0, "rx conf0 ch{}", i + 4);
            let v1 = ((i as u32) << 5) & !RX_CONF1_WT_MASK;
            r.write_word(*c1, v1);
            assert_eq!(r.read_word(*c1), v1, "rx conf1 ch{}", i + 4);
        }
    }

    #[test]
    fn tx_start_latches_tx_end_and_autoclears() {
        let mut r = rmt();
        // Arm channel 2 with TX_START (bit 0) | CONF_UPDATE (bit 24) plus some
        // config bits in the divider field.
        let conf = TX_START_BIT | TX_CONF_UPDATE_BIT | (5 << 8);
        r.write_word(0x28, conf); // CH2CONF0

        // TX_END for channel 2 latched in INT_RAW (bit 2).
        assert_eq!(r.read_word(0x70) & tx_end_bit(2), tx_end_bit(2));

        // tx_start (and conf_update) auto-cleared in stored CONF0; the config
        // payload (divider) remains.
        let readback = r.read_word(0x28);
        assert_eq!(readback & TX_START_BIT, 0, "tx_start must auto-clear");
        assert_eq!(readback & TX_CONF_UPDATE_BIT, 0, "conf_update auto-clears");
        assert_eq!(readback & (0xFF << 8), 5 << 8, "config payload retained");
    }

    #[test]
    fn tx_start_each_channel_sets_distinct_bit() {
        for (off, ch) in [(0x20u64, 0usize), (0x24, 1), (0x28, 2), (0x2C, 3)] {
            let mut r = rmt();
            r.write_word(off, TX_START_BIT);
            assert_eq!(
                r.read_word(0x70),
                tx_end_bit(ch),
                "channel {ch} TX_END bit only"
            );
        }
    }

    #[test]
    fn conf_update_without_tx_start_does_not_complete() {
        let mut r = rmt();
        // CONF_UPDATE alone (config sync, no transmit) → no TX_END.
        r.write_word(0x20, TX_CONF_UPDATE_BIT | (3 << 8));
        assert_eq!(r.read_word(0x70), 0, "no TX_END without TX_START");
        assert_eq!(r.read_word(0x20) & (0xFF << 8), 3 << 8);
    }

    #[test]
    fn int_clr_w1c_clears_only_written_bits() {
        let mut r = rmt();
        // Latch TX_END on channels 0 and 3.
        r.write_word(0x20, TX_START_BIT);
        r.write_word(0x2C, TX_START_BIT);
        assert_eq!(r.read_word(0x70), tx_end_bit(0) | tx_end_bit(3));

        // INT_CLR bit 0 only — clears channel 0, leaves channel 3.
        r.write_word(0x7C, tx_end_bit(0));
        assert_eq!(r.read_word(0x70), tx_end_bit(3));

        // Clear the rest.
        r.write_word(0x7C, tx_end_bit(3));
        assert_eq!(r.read_word(0x70), 0);
        // INT_CLR reads back as 0.
        assert_eq!(r.read_word(0x7C), 0);
    }

    #[test]
    fn int_st_masks_raw_with_ena() {
        let mut r = rmt();
        r.write_word(0x20, TX_START_BIT); // TX_END ch0 raw
        assert_eq!(r.read_word(0x70), tx_end_bit(0));
        // INT_ENA = 0 → INT_ST masked to 0.
        assert_eq!(r.read_word(0x74), 0, "INT_ST masked when ENA=0");
        // Enable ch0 TX_END.
        r.write_word(0x78, tx_end_bit(0));
        assert_eq!(r.read_word(0x74), tx_end_bit(0), "INT_ST = RAW & ENA");
    }

    #[test]
    fn tick_emits_source_while_int_st_set() {
        let mut r = rmt();
        // No pending → no IRQ.
        assert!(r.tick().explicit_irqs.is_none());

        // Latch a TX_END and enable it.
        r.write_word(0x20, TX_START_BIT);
        r.write_word(0x78, tx_end_bit(0));

        // Level-sensitive: emits the RMT source every tick while INT_ST != 0.
        assert_eq!(r.tick().explicit_irqs.as_deref(), Some(&[RMT_SOURCE][..]));
        assert_eq!(r.tick().explicit_irqs.as_deref(), Some(&[RMT_SOURCE][..]));

        // ACK via INT_CLR → IRQ de-asserts.
        r.write_word(0x7C, tx_end_bit(0));
        assert!(r.tick().explicit_irqs.is_none(), "no IRQ after INT_CLR");
    }

    #[test]
    fn int_raw_pending_but_disabled_emits_no_irq() {
        let mut r = rmt();
        r.write_word(0x20, TX_START_BIT); // raw pending, ENA=0
        assert_eq!(r.read_word(0x70), tx_end_bit(0));
        assert!(
            r.tick().explicit_irqs.is_none(),
            "raw pending without ENA must not emit"
        );
    }

    #[test]
    fn byte_writes_compose_tx_start_word() {
        // Firmware may write CONF0 a byte at a time; the TX-completion logic
        // must fire once the low byte (carrying TX_START) lands.
        let mut r = rmt();
        // Write high bytes first (no TX_START yet).
        r.write(0x21, 0x05).unwrap(); // bits[15:8] = divider
        assert_eq!(r.read_word(0x70), 0, "no completion before TX_START byte");
        // Now the low byte with TX_START.
        r.write(0x20, 0x01).unwrap();
        assert_eq!(r.read_word(0x70), tx_end_bit(0), "TX_END latched");
    }

    #[test]
    fn source_id_is_configurable() {
        let mut r = Esp32s3Rmt::new(99);
        r.write_word(0x20, TX_START_BIT);
        r.write_word(0x78, tx_end_bit(0));
        assert_eq!(r.tick().explicit_irqs.as_deref(), Some(&[99][..]));
    }

    // ── Task 1: CARRIER_DUTY ch0..3 (0x80..0x8C) ──────────────────────────
    #[test]
    fn carrier_duty_reset_and_round_trip() {
        let mut r = rmt();
        // Reset value 0x0040_0040 (high duty 0x40, low duty 0x40) on all 4.
        assert_eq!(r.read_word(0x80), 0x0040_0040, "CH0 CARRIER_DUTY reset");
        assert_eq!(r.read_word(0x84), 0x0040_0040, "CH1 CARRIER_DUTY reset");
        assert_eq!(r.read_word(0x88), 0x0040_0040, "CH2 CARRIER_DUTY reset");
        assert_eq!(r.read_word(0x8C), 0x0040_0040, "CH3 CARRIER_DUTY reset");
        // Fully writable: arbitrary value round-trips verbatim.
        r.write_word(0x84, 0x1234_ABCD);
        assert_eq!(r.read_word(0x84), 0x1234_ABCD);
        // Other channels unaffected (isolation).
        assert_eq!(r.read_word(0x80), 0x0040_0040);
        assert_eq!(r.read_word(0x88), 0x0040_0040);
    }

    // ── Task 2: RX_CARRIER_RM ch4..7 (0x90..0x9C) ─────────────────────────
    #[test]
    fn rx_carrier_rm_reset_and_round_trip() {
        let mut r = rmt();
        // Reset 0.
        assert_eq!(r.read_word(0x90), 0, "CH4 RX_CARRIER_RM reset");
        assert_eq!(r.read_word(0x9C), 0, "CH7 RX_CARRIER_RM reset");
        // Fully writable round-trip.
        r.write_word(0x98, 0xCAFE_F00D);
        assert_eq!(r.read_word(0x98), 0xCAFE_F00D);
        assert_eq!(r.read_word(0x90), 0, "isolation");
    }

    // ── Task 3: RX_STATUS ch4..7 (0x60..0x6C) ─────────────────────────────
    #[test]
    fn rx_status_returns_idle_constant_and_is_read_only() {
        let mut r = rmt();
        // Read-only idle/reset constant 0x0006_00C0 per channel.
        assert_eq!(r.read_word(0x60), 0x0006_00C0, "CH4 RX_STATUS idle");
        assert_eq!(r.read_word(0x64), 0x0006_00C0, "CH5 RX_STATUS idle");
        assert_eq!(r.read_word(0x68), 0x0006_00C0, "CH6 RX_STATUS idle");
        assert_eq!(r.read_word(0x6C), 0x0006_00C0, "CH7 RX_STATUS idle");
        // Read-only: a write does not change it.
        r.write_word(0x60, 0xFFFF_FFFF);
        assert_eq!(r.read_word(0x60), 0x0006_00C0, "RX_STATUS ignores writes");
    }

    // ── Task 4: TX_STATUS ch0..3 (0x50..0x5C) ─────────────────────────────
    #[test]
    fn tx_status_idle_zero_and_read_only() {
        let mut r = rmt();
        // Fire-and-complete TX engine → TX is always idle → status reads 0.
        assert_eq!(r.read_word(0x50), 0, "CH0 TX_STATUS idle");
        assert_eq!(r.read_word(0x5C), 0, "CH3 TX_STATUS idle");
        // Read-only: writes ignored.
        r.write_word(0x50, 0xFFFF_FFFF);
        assert_eq!(r.read_word(0x50), 0, "TX_STATUS ignores writes");
    }

    // ── Task 5: CHnDATA APB-FIFO port (0x00..0x1C) ────────────────────────
    #[test]
    fn chndata_fifo_stores_and_reads_back_written_word() {
        let mut r = rmt();
        // Write two distinct words to CH2DATA (0x08).
        r.write_word(0x08, 0x1111_2222);
        // Non-destructive read returns the most-recently-written word.
        assert_eq!(r.read_word(0x08), 0x1111_2222, "read sees last write");
        r.write_word(0x08, 0x3333_4444);
        assert_eq!(r.read_word(0x08), 0x3333_4444, "read sees newest write");
        // Both words landed in CH2's RAM in write order via the auto-incr ptr.
        assert_eq!(r.ch_mem[2][0], 0x1111_2222, "first word at slot 0");
        assert_eq!(r.ch_mem[2][1], 0x3333_4444, "second word at slot 1");
        assert_eq!(r.ch_wr[2], 2, "write pointer advanced twice");
    }

    #[test]
    fn chndata_channel_isolation() {
        let mut r = rmt();
        // Writing CH2 must not disturb CH3's RAM or read port.
        r.write_word(0x08, 0xAAAA_AAAA); // CH2
        assert_eq!(r.read_word(0x0C), 0, "CH3 read unaffected (empty)");
        r.write_word(0x0C, 0xBBBB_BBBB); // CH3
        assert_eq!(r.read_word(0x08), 0xAAAA_AAAA, "CH2 still holds its word");
        assert_eq!(r.read_word(0x0C), 0xBBBB_BBBB, "CH3 holds its own word");
        assert_eq!(r.ch_wr[2], 1);
        assert_eq!(r.ch_wr[3], 1);
    }

    #[test]
    fn chndata_write_pointer_wraps_at_ram_size() {
        let mut r = rmt();
        // Push more than 48 entries on CH0; pointer index wraps mod 48 and the
        // bounded RAM never overflows (write must never panic).
        for i in 0..50u32 {
            r.write_word(0x00, 0xD000_0000 | i);
        }
        // Slot 0 was overwritten by entry 48; slot 1 by entry 49.
        assert_eq!(r.ch_mem[0][0], 0xD000_0000 | 48);
        assert_eq!(r.ch_mem[0][1], 0xD000_0000 | 49);
        assert_eq!(
            r.ch_mem[0][2],
            0xD000_0000 | 2,
            "untouched since first pass"
        );
        // Last write (entry 49) is what a read returns.
        assert_eq!(r.read_word(0x00), 0xD000_0000 | 49);
    }

    #[test]
    fn chndata_byte_writes_are_panic_free_and_assemble_correct_final_word() {
        // The APB-FIFO data port is word-accessed by real drivers; the byte
        // path is non-idiomatic but must stay panic-free under panic=abort and
        // leave the FIFO in a sane state. Because read_word is non-destructive
        // (it does not advance ch_wr), the byte RMW reads the in-progress word
        // back each time, so each of the four byte-writes pushes one
        // progressively-assembled entry (0xEF, 0xBEEF, 0xADBEEF, 0xDEADBEEF).
        // The fully-assembled word is the last one pushed and is what a read
        // returns — which is the behaviour that matters to a (word-accessing)
        // driver.
        let mut r = rmt();
        r.write(0x04, 0xEF).unwrap(); // CH1DATA, low byte first
        r.write(0x05, 0xBE).unwrap();
        r.write(0x06, 0xAD).unwrap();
        r.write(0x07, 0xDE).unwrap();
        // The newest entry is the complete word, and that is what reads see.
        assert_eq!(
            r.read_word(0x04),
            0xDEAD_BEEF,
            "final assembled word readable"
        );
        assert_eq!(
            r.ch_mem[1][3], 0xDEAD_BEEF,
            "complete word landed at slot 3"
        );
        // Four byte-writes → four pushes of the evolving entry; the pointer is
        // sane and no index ever went out of range.
        assert_eq!(r.ch_wr[1], 4, "one push per byte-write RMW");
    }
}
