// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RMT (Remote Control / pulse transceiver) peripheral for ESP32-S3.
//!
//! The ESP32-S3 RMT has **4 TX channels (0..3) + 4 RX channels (4..7)** sharing
//! a 384×32 RAM block (RMTMEM) mapped at `DR_REG_RMT_BASE + 0x400`.
//!
//! The parent (`system/xtensa/esp32s3.rs`) maps this peripheral as a single
//! **0x1000-byte** window at 0x6001_6000 — there is no separate RMTMEM device.
//! Both the register file (0x00..0xD0) and the symbol RAM (0x400..0xA00) are
//! therefore this module's responsibility, and both are backed here. Offsets
//! inside the 0x1000 window that belong to neither (0xD0..0x400, 0xA00..0x1000)
//! read as 0 and ignore writes.
//!
//! ## Register map (verified against ESP-IDF
//! `components/soc/esp32s3/register/soc/rmt_reg.h`, base = DR_REG_RMT_BASE =
//! 0x6001_6000):
//!
//! | Offset | Name                | Notes |
//! |-------:|---------------------|-------|
//! | 0x00..0x1C | CH0..7 DATA      | APB-FIFO data port; aliases the same RAM as +0x400 |
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
//! | 0x400..0xA00 | RMTMEM        | 384×32 symbol RAM, shared by all 8 channels |
//!
//! ## Symbol RAM (RMTMEM)
//!
//! RMTMEM is one flat 384-word array. There are two access paths into it and
//! they alias the **same** backing store:
//!
//!   * **Direct** (0x400..0xA00) — a plain word-addressed memory. Modern
//!     ESP-IDF (`rmt_tx`) composes symbols straight into RMTMEM this way.
//!   * **APB-FIFO** (CHnDATA, 0x00..0x1C) — pushes one 32-bit entry at a time
//!     through the channel's auto-incrementing write pointer.
//!
//! Each channel `n` owns a window starting at word `n * 48`. Its **length**
//! comes from that channel's `MEM_SIZE` field (TX `CHnCONF0[19:16]`, RX
//! `CHmCONF0[27:24]`), so a channel may *borrow* subsequent blocks:
//! `MEM_SIZE = 2` gives channel 0 words 0..96. Chosen semantics:
//!
//!   * `MEM_SIZE = 0` is not a legal hardware configuration (a zero-length
//!     window has nowhere to put an entry); it is treated as one block.
//!   * A window is clamped to the end of RMTMEM, so an over-large `MEM_SIZE`
//!     lets a channel run to word 384 but never past it. Borrowing is *not*
//!     policed against the next channel's base — on silicon an overlapping
//!     `MEM_SIZE` genuinely does corrupt the neighbouring channel, and the
//!     flat store reproduces that faithfully.
//!   * The FIFO write pointer wraps modulo the window length, so no amount of
//!     CHnDATA traffic can overflow or panic.
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

use crate::bus::SystemBus;
use crate::peripherals::esp32s3::gpio::{
    Esp32s3Gpio, RMT_SIG_OUT0, RMT_SIG_OUT1, RMT_SIG_OUT2, RMT_SIG_OUT3,
};
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};

/// Number of TX channels (0..3).
const TX_CHANNELS: usize = 4;
/// Number of RX channels (4..7).
const RX_CHANNELS: usize = 4;

/// Total channels (0..7) exposing a CHnDATA APB-FIFO port at 0x00..0x1C.
const TOTAL_CHANNELS: usize = 8;
/// RMT memory block depth on the ESP32-S3 (48 × 32-bit entries per block).
/// One block is a channel's default window; `MEM_SIZE` may borrow more.
const RMT_BLOCK_WORDS: usize = 48;
/// Total RMTMEM depth: 8 blocks × 48 words = 384 × 32-bit, shared by all
/// channels (`SOC_RMT_MEM_WORDS_PER_CHANNEL * SOC_RMT_CHANNELS_PER_GROUP`).
const RMT_MEM_WORDS: usize = RMT_BLOCK_WORDS * TOTAL_CHANNELS;

/// First offset of the direct RMTMEM aperture inside the parent's 0x1000
/// window (`DR_REG_RMT_BASE + 0x400`).
const RMTMEM_START: u64 = 0x400;
/// One past the last RMTMEM byte offset: 0x400 + 384*4 = 0xA00.
const RMTMEM_END: u64 = RMTMEM_START + (RMT_MEM_WORDS as u64) * 4;

/// CARRIER_DUTY reset default (ESP32-S3 SVD): high-duty 0x40, low-duty 0x40.
const CARRIER_DUTY_RESET: u32 = 0x0040_0040;
/// RX_STATUS read-only idle/reset constant (ESP32-S3 SVD reset 0x0006_00C0).
const RX_STATUS_IDLE: u32 = 0x0006_00C0;

// ── CHnCONF0 (TX, n=0..3) bit fields ──────────────────────────────────────
/// TX_START — bit 0 (WT, self-clearing): start sending data on the channel.
pub const TX_START_BIT: u32 = 1 << 0;
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

// ── RMT symbol (entry) bit layout — one 32-bit RMTMEM word = two pulses ────
// Each entry encodes back-to-back pulses `(duration0, level0)`, `(duration1,
// level1)`: durations are 15-bit counts of RMT-clock ticks, levels are the pad
// output during that span. A pulse with `duration == 0` is the END marker.
/// 15-bit duration field mask (bits [14:0] for pulse 0, [30:16] for pulse 1).
const ENTRY_DUR_MASK: u32 = 0x7FFF;

/// GPIO-matrix output signal index of RMT TX channel `ch` (0..3) — what a routed
/// pad's `FUNCn_OUT_SEL` selects to receive this channel's waveform.
const fn rmt_out_signal(ch: usize) -> u32 {
    match ch {
        0 => RMT_SIG_OUT0,
        1 => RMT_SIG_OUT1,
        2 => RMT_SIG_OUT2,
        _ => RMT_SIG_OUT3,
    }
}

/// A timed TX playback in flight: the decoded pad-level edge schedule of one TX
/// channel, advanced one edge at a time by [`Esp32s3Rmt::tick_with_bus`] so the
/// routed GPIO pad emits real bit-level edges an observer (a future WS2812
/// decoder) can capture. Only `edges` that actually *change* the pad level are
/// recorded; `offset_cycle` is measured in sim cycles from playback start.
#[derive(Debug)]
struct TxPlayback {
    /// GPIO-matrix output signal of the transmitting channel (RMT_SIG_OUTn).
    signal: u32,
    /// Ascending `(offset_cycle, new_level)` pad transitions (changes only).
    edges: Vec<(u64, bool)>,
    /// Index of the next edge to emit.
    next: usize,
    /// Sim cycles elapsed since playback started (incremented per bus tick).
    play_cycle: u64,
    /// Pads the signal is routed to (`FUNCn_OUT_SEL`), resolved on the first
    /// bus tick. Empty after resolution ⇒ the channel drives no pad.
    pads: Vec<u8>,
    /// False until the routed pads have been resolved from the GPIO matrix.
    resolved: bool,
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

    /// RMTMEM — the flat 384×32 symbol RAM shared by all 8 channels.
    ///
    /// This is the single backing store for **both** access paths: the direct
    /// aperture at 0x400..0xA00 and the CHnDATA APB-FIFO port at 0x00..0x1C
    /// alias the same words. Channel `n`'s window starts at word
    /// `n * RMT_BLOCK_WORDS` and runs for `mem_window_words(n)` words (see the
    /// module docs for the `MEM_SIZE` borrowing rules).
    ///
    /// Writing CHnDATA pushes a 32-bit RMT entry via an auto-incrementing write
    /// pointer (`ch_wr`); the index wraps mod the channel's window length so
    /// arbitrary input never overflows nor panics.
    ///
    /// CHnDATA reads are **non-destructive** and return the most-recently-
    /// written word.
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
    mem: [u32; RMT_MEM_WORDS],
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

    /// Active timed TX playback, if any (RMT Stage 2). Armed on a `TX_START`
    /// write when the channel's RMTMEM holds a non-empty symbol sequence;
    /// `None` when idle, so an idle RMT reports `needs_bus_tick() == false` and
    /// costs the bus-tick pass nothing. At most one channel plays at a time —
    /// a limitation adequate for single-strip WS2812 (the Stage-3 target).
    playback: Option<TxPlayback>,

    /// Sim cycles remaining before level-IRQ emission is allowed after a
    /// `TX_START`. INT_RAW latches immediately; `explicit_irqs` stay quiet
    /// until this hits zero so the IDF `rmt_transmit` path can release its
    /// locks / reach `WaitBits` before `rmt_tx_default_isr` runs (Arduino
    /// WS2812 / `rgbLedWrite` FreeRTOS queue corruption).
    irq_holdoff_cycles: u32,
}

impl Esp32s3Rmt {
    /// Construct the RMT with the given interrupt-matrix `source_id`.
    /// On ESP32-S3 this is `ETS_RMT_INTR_SOURCE = 40`.
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            // TX CONF0 reset default: DIV_CNT=2 (bits[15:8]), MEM_SIZE=1
            // (bits[19:16]), CARRIER_EFF_EN=1 (20), CARRIER_EN=1 (21),
            // CARRIER_OUT_LV=1 (22) → 0x0071_0200.
            //
            // NOTE: this constant read 0x0017_0200 until RMTMEM was backed —
            // a transposition of the two upper nibbles that contradicted the
            // comment above on every field it names (it decodes to MEM_SIZE=7,
            // CARRIER_EN=0, CARRIER_OUT_LV=0). It was inert while MEM_SIZE was
            // unused, but it is load-bearing now: MEM_SIZE=7 would give every
            // TX channel a 336-word window at reset and let a channel-0 FIFO
            // burst overwrite channels 1..6.
            tx_conf0: [0x0071_0200; TX_CHANNELS],
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
            // RMTMEM symbol RAM + FIFO write pointers start empty/zeroed.
            mem: [0; RMT_MEM_WORDS],
            ch_wr: [0; TOTAL_CHANNELS],
            // SYS_CONF reset default: SCLK_DIV_NUM=1 (bits[11:4] = 1<<4=0x10),
            // SCLK_SEL=1 (bits[25:24]), SCLK_ACTIVE=1 (26).
            sys_conf: (1 << 4) | (1 << 24) | (1 << 26),
            tx_sim: 0,
            int_raw: 0,
            int_ena: 0,
            playback: None,
            irq_holdoff_cycles: 0,
        }
    }

    /// Hold off TX-done IRQ for this many sim cycles after TX_START.
    /// Long enough for IDF `rmt_transmit` to return into the caller's
    /// WaitBits while still short for WS2812 bit times at 10 MHz RMT clk.
    /// Sim cycles between TX_START and first TX-done IRQ. Must cover
    /// `rmt_transmit` return + `rmtWrite` blocking on the completion queue.
    /// Too short → GiveFromISR with no waiter → permanent block or queue
    /// corruption (Arduino S3 RGB L2 @ 0x20406a).
    const TX_IRQ_HOLDOFF: u32 = 2000;

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

    /// Direct RMTMEM aperture (0x400..0xA00) → flat word index into `mem`.
    /// Offsets outside the aperture (including the 0xD0..0x400 and
    /// 0xA00..0x1000 holes in the parent's window) map to `None`.
    fn rmtmem_index(offset: u64) -> Option<usize> {
        if (RMTMEM_START..RMTMEM_END).contains(&offset) {
            Some(((offset - RMTMEM_START) / 4) as usize)
        } else {
            None
        }
    }

    /// `MEM_SIZE` for channel `ch`, in 48-word blocks. TX channels 0..3 carry
    /// it in `CHnCONF0[19:16]`; RX channels 4..7 in `CHmCONF0[27:24]`.
    /// `MEM_SIZE = 0` is not a legal hardware setting (a zero-length window has
    /// nowhere to store an entry) and is treated as the reset value, 1 block.
    fn mem_size_blocks(&self, ch: usize) -> usize {
        let raw = if ch < TX_CHANNELS {
            (self.tx_conf0[ch] >> 16) & 0xF
        } else {
            (self.rx_conf0[ch - TX_CHANNELS] >> 24) & 0xF
        } as usize;
        raw.max(1)
    }

    /// Length in words of channel `ch`'s RMTMEM window, honouring `MEM_SIZE`
    /// block borrowing. Clamped to the end of RMTMEM so an over-large
    /// `MEM_SIZE` can never index past the 384-word store. Overlap with the
    /// next channel's block is deliberately *not* policed — silicon corrupts
    /// the neighbour in exactly that case.
    fn mem_window_words(&self, ch: usize) -> usize {
        let base = ch * RMT_BLOCK_WORDS;
        (self.mem_size_blocks(ch) * RMT_BLOCK_WORDS).min(RMT_MEM_WORDS - base)
    }

    /// INT_ST = INT_RAW & INT_ENA (masked-interrupt status).
    fn int_st(&self) -> u32 {
        self.int_raw & self.int_ena
    }

    /// Sim cycles per RMT-clock tick for TX channel `ch` — its `DIV_CNT`
    /// (`CHnCONF0[15:8]`, min 1). The model treats the RMT source clock as one
    /// tick per sim cycle, so a symbol duration of `d` RMT ticks spans
    /// `d * DIV_CNT` sim cycles. (The SYS_CONF fractional divider is not applied
    /// — a documented Stage-2 approximation; integer `DIV_CNT` carries the WS2812
    /// bit-timing that matters.)
    fn cycles_per_rmt_tick(&self, ch: usize) -> u64 {
        (((self.tx_conf0[ch] >> 8) & 0xFF) as u64).max(1)
    }

    /// Decode TX channel `ch`'s RMTMEM window into an ascending list of pad
    /// level *transitions* `(offset_cycle, new_level)`. Walks entries from the
    /// channel's window base until an END marker (`duration == 0`) or the window
    /// end; each entry contributes two `(duration, level)` pulses. The line
    /// starts at the idle level (low); only actual changes are emitted, so a
    /// run of same-level pulses collapses to nothing.
    fn decode_tx_edges(&self, ch: usize) -> Vec<(u64, bool)> {
        let base = ch * RMT_BLOCK_WORDS;
        let len = self.mem_window_words(ch);
        let per_tick = self.cycles_per_rmt_tick(ch);
        let mut edges = Vec::new();
        let mut level = false; // idle low
        let mut offset = 0u64;
        for i in 0..len {
            let w = self.mem[base + i];
            for (dur, lvl) in [
                ((w & ENTRY_DUR_MASK) as u64, (w >> 15) & 1 != 0),
                (((w >> 16) & ENTRY_DUR_MASK) as u64, (w >> 31) & 1 != 0),
            ] {
                if dur == 0 {
                    return edges; // END marker terminates the sequence
                }
                if lvl != level {
                    edges.push((offset, lvl));
                    level = lvl;
                }
                offset += dur * per_tick;
            }
        }
        edges
    }

    /// Arm a timed pad playback for TX channel `ch` (called on a `TX_START`
    /// write, alongside the instantaneous `TX_END` latch). Decodes the channel's
    /// symbols; if they produce at least one pad edge, stores the schedule so the
    /// bus-tick pass plays it out on the routed pad. An empty schedule (idle RAM)
    /// leaves `playback` untouched, so `needs_bus_tick` stays false.
    fn arm_tx_playback(&mut self, ch: usize) {
        let edges = self.decode_tx_edges(ch);
        if edges.is_empty() {
            return;
        }
        self.playback = Some(TxPlayback {
            signal: rmt_out_signal(ch),
            edges,
            next: 0,
            play_cycle: 0,
            pads: Vec::new(),
            resolved: false,
        });
    }

    /// Borrow the sibling ESP32-S3 GPIO peripheral off the bus (during the
    /// `tick_with_bus` swap dance this RMT is lent `&mut SystemBus`, which still
    /// holds the GPIO), run `f` against it, and return its result. `None` if the
    /// bus is not a `SystemBus` or carries no `Esp32s3Gpio` named "gpio".
    fn with_gpio<R>(bus: &mut dyn Bus, f: impl FnOnce(&mut Esp32s3Gpio) -> R) -> Option<R> {
        let sb = bus.as_any_mut()?.downcast_mut::<SystemBus>()?;
        let idx = sb.find_peripheral_index_by_name("gpio")?;
        let gpio = sb.peripherals[idx]
            .dev
            .as_any_mut()?
            .downcast_mut::<Esp32s3Gpio>()?;
        Some(f(gpio))
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
        // most-recently-written word (see `mem` doc). Empty channel reads 0.
        if let Some(ch) = Self::chndata_index(offset) {
            return if self.ch_wr[ch] == 0 {
                0
            } else {
                let len = self.mem_window_words(ch);
                self.mem[ch * RMT_BLOCK_WORDS + (self.ch_wr[ch] - 1) % len]
            };
        }
        // Direct RMTMEM aperture: plain word-addressed symbol RAM.
        if let Some(i) = Self::rmtmem_index(offset) {
            return self.mem[i];
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
            // TX start: latch CHn_TX_END in INT_RAW immediately (status/poll),
            // but hold off level-IRQ emission (see `irq_holdoff_cycles`).
            if value & TX_START_BIT != 0 {
                self.int_raw |= tx_end_bit(i);
                self.irq_holdoff_cycles = Self::TX_IRQ_HOLDOFF;
                // RMT Stage 2: timed pad waveform for observers / logic capture.
                self.arm_tx_playback(i);
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
        // CHnDATA APB-FIFO port: push the 32-bit entry into the channel's
        // RMTMEM window via the auto-incrementing write pointer (the index
        // wraps mod the window length so arbitrary input never overflows).
        // This lands in the SAME store the direct 0x400 aperture exposes.
        if let Some(ch) = Self::chndata_index(offset) {
            let len = self.mem_window_words(ch);
            let slot = ch * RMT_BLOCK_WORDS + self.ch_wr[ch] % len;
            self.mem[slot] = value;
            self.ch_wr[ch] = self.ch_wr[ch].wrapping_add(1);
            return;
        }
        // Direct RMTMEM aperture: plain word-addressed symbol RAM.
        if let Some(i) = Self::rmtmem_index(offset) {
            self.mem[i] = value;
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
    /// (INT_ST != 0) **and** the post-TX_START holdoff has expired, re-emit
    /// the RMT source on every tick so the bus aggregator keeps the CPU's
    /// pending line asserted until firmware ACKs at the source (INT_CLR).
    fn tick(&mut self) -> PeripheralTickResult {
        if self.irq_holdoff_cycles > 0 {
            self.irq_holdoff_cycles = self.irq_holdoff_cycles.saturating_sub(1);
        }
        let explicit_irqs = if self.irq_holdoff_cycles == 0 && self.int_st() != 0 {
            Some(vec![self.source_id])
        } else {
            None
        };
        PeripheralTickResult {
            explicit_irqs,
            ..PeripheralTickResult::default()
        }
    }

    /// Bus-tick opt-in: active while a timed playback is in flight.
    fn needs_bus_tick(&self) -> bool {
        self.playback.is_some()
    }

    fn needs_legacy_walk(&self) -> bool {
        true
    }

    /// Play out the armed TX waveform onto the routed GPIO pad(s), one bus tick
    /// at a time. On the first tick the routed pads are resolved from the GPIO
    /// matrix (`FUNCn_OUT_SEL`); if the channel is not routed to any pad the
    /// playback is dropped (nothing to drive). Thereafter each edge whose
    /// scheduled offset has arrived is driven via
    /// [`Esp32s3Gpio::drive_pad_output`], reaching every registered
    /// `GpioObserver` with the correct sim-cycle timing.
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if self.playback.is_none() {
            return;
        }
        // Resolve routed pads on the first bus tick of the playback.
        if !self.playback.as_ref().unwrap().resolved {
            let signal = self.playback.as_ref().unwrap().signal;
            let pads =
                Self::with_gpio(bus, |g| g.pads_for_output_signal(signal)).unwrap_or_default();
            let pb = self.playback.as_mut().unwrap();
            pb.resolved = true;
            pb.pads = pads;
            if pb.pads.is_empty() {
                self.playback = None; // channel drives no pad — nothing to play
                return;
            }
        }
        // Collect the edges due at or before the current play cycle.
        let (levels, done) = {
            let pb = self.playback.as_mut().unwrap();
            let now = pb.play_cycle;
            let mut levels = Vec::new();
            while pb.next < pb.edges.len() && pb.edges[pb.next].0 <= now {
                levels.push(pb.edges[pb.next].1);
                pb.next += 1;
            }
            pb.play_cycle += 1;
            (levels, pb.next >= pb.edges.len())
        };
        if !levels.is_empty() {
            let pads = self.playback.as_ref().unwrap().pads.clone();
            Self::with_gpio(bus, |g| {
                for level in levels {
                    for &pad in &pads {
                        g.drive_pad_output(pad, level);
                    }
                }
            });
        }
        if done {
            self.playback = None;
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

    /// Drain TX IRQ holdoff so subsequent ticks emit level IRQs.
    fn retire_tx_end(r: &mut Esp32s3Rmt) {
        r.irq_holdoff_cycles = 0;
        let _ = r.tick();
    }

    #[test]
    fn reset_defaults_seeded() {
        let r = rmt();
        // TX CONF0 default 0x0071_0200 (DIV_CNT=2, MEM_SIZE=1, carrier bits
        // 20/21/22 set). Decoded field-by-field so a transposed constant
        // cannot pass unnoticed again.
        assert_eq!(r.read_word(0x20), 0x0071_0200);
        assert_eq!(r.read_word(0x2C), 0x0071_0200);
        let c0 = r.read_word(0x20);
        assert_eq!((c0 >> 8) & 0xFF, 2, "DIV_CNT=2");
        assert_eq!((c0 >> 16) & 0xF, 1, "MEM_SIZE=1 block");
        assert_eq!((c0 >> 20) & 1, 1, "CARRIER_EFF_EN");
        assert_eq!((c0 >> 21) & 1, 1, "CARRIER_EN");
        assert_eq!((c0 >> 22) & 1, 1, "CARRIER_OUT_LV");
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

        // INT_RAW latches immediately; IRQ emission is deferred one tick.
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

        // Latch TX_END and enable it; holdoff must expire before emit.
        r.write_word(0x20, TX_START_BIT);
        r.write_word(0x78, tx_end_bit(0));
        // During holdoff, no IRQ even though INT_ST is live.
        assert!(r.tick().explicit_irqs.is_none(), "holdoff suppresses IRQ");
        retire_tx_end(&mut r);

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
        retire_tx_end(&mut r);
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
        retire_tx_end(&mut r);
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
        // Both words landed in CH2's RMTMEM window in write order via the
        // auto-incrementing pointer (CH2 base = 2 * 48 = word 96).
        let base = 2 * RMT_BLOCK_WORDS;
        assert_eq!(r.mem[base], 0x1111_2222, "first word at slot 0");
        assert_eq!(r.mem[base + 1], 0x3333_4444, "second word at slot 1");
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
        assert_eq!(r.mem[0], 0xD000_0000 | 48);
        assert_eq!(r.mem[1], 0xD000_0000 | 49);
        assert_eq!(r.mem[2], 0xD000_0000 | 2, "untouched since first pass");
        // The wrap stayed inside CH0's one-block window: CH1's block (word 48)
        // was never touched.
        assert_eq!(r.mem[RMT_BLOCK_WORDS], 0, "did not spill into CH1");
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
            r.mem[RMT_BLOCK_WORDS + 3],
            0xDEAD_BEEF,
            "complete word landed at CH1 slot 3"
        );
        // Four byte-writes → four pushes of the evolving entry; the pointer is
        // sane and no index ever went out of range.
        assert_eq!(r.ch_wr[1], 4, "one push per byte-write RMW");
    }

    // ── RMTMEM symbol RAM (0x400..0xA00) ──────────────────────────────────
    //
    // The parent maps this peripheral with a 0x1000-byte window, so these
    // offsets route here. Before RMTMEM was backed they fell through
    // `write_word`'s catch-all and were silently discarded — which broke
    // modern ESP-IDF `rmt_tx`, since it composes symbols directly into RMTMEM
    // rather than through the CHnDATA FIFO.

    /// Word offset of RMTMEM word `n` within the peripheral window.
    const fn rmtmem_off(n: usize) -> u64 {
        RMTMEM_START + (n as u64) * 4
    }

    #[test]
    fn rmtmem_direct_write_reads_back() {
        let mut r = rmt();
        // A symbol written straight into RMTMEM must survive (this is the bug).
        r.write_word(rmtmem_off(0), 0x8010_0010);
        assert_eq!(r.read_word(rmtmem_off(0)), 0x8010_0010, "word 0 persists");

        // Works across the whole 384-word store, including the last word.
        r.write_word(rmtmem_off(383), 0xCAFE_F00D);
        assert_eq!(r.read_word(rmtmem_off(383)), 0xCAFE_F00D, "last word");

        // Untouched words still read 0.
        assert_eq!(r.read_word(rmtmem_off(1)), 0, "unwritten word reads 0");
    }

    #[test]
    fn rmtmem_and_chndata_alias_the_same_store() {
        // The core of the fix: the APB-FIFO port and the direct aperture are
        // two views of ONE backing store, not two separate buffers.
        let mut r = rmt();

        // FIFO → RMTMEM. CH0's window starts at word 0.
        r.write_word(0x00, 0x1234_5678);
        assert_eq!(
            r.read_word(rmtmem_off(0)),
            0x1234_5678,
            "CH0DATA push visible at RMTMEM word 0"
        );

        // CH3's window starts at word 3 * 48 = 144.
        r.write_word(0x0C, 0xA5A5_1111);
        assert_eq!(
            r.read_word(rmtmem_off(3 * RMT_BLOCK_WORDS)),
            0xA5A5_1111,
            "CH3DATA push visible at RMTMEM word 144"
        );

        // RMTMEM → FIFO read port. A direct write to the slot the CH1 pointer
        // last used is what the CH1DATA read returns.
        r.write_word(0x04, 0x0000_0000); // advance CH1's pointer to slot 1
        r.write_word(rmtmem_off(RMT_BLOCK_WORDS), 0xBEEF_0001);
        assert_eq!(
            r.read_word(0x04),
            0xBEEF_0001,
            "CH1DATA read sees the direct RMTMEM write at its last slot"
        );
    }

    #[test]
    fn rmtmem_channel_windows_are_isolated() {
        let mut r = rmt();
        // Fill channel 1's whole default block through the direct aperture.
        for i in 0..RMT_BLOCK_WORDS {
            r.write_word(rmtmem_off(RMT_BLOCK_WORDS + i), 0x1100_0000 | i as u32);
        }
        // Channel 0's block is untouched.
        for i in 0..RMT_BLOCK_WORDS {
            assert_eq!(r.read_word(rmtmem_off(i)), 0, "CH0 word {i} undisturbed");
        }
        // And channel 0's FIFO port still sees an empty channel.
        assert_eq!(r.read_word(0x00), 0, "CH0DATA still empty");

        // Pushing on CH0 does not disturb channel 1's contents.
        r.write_word(0x00, 0xDEAD_0000);
        assert_eq!(
            r.read_word(rmtmem_off(RMT_BLOCK_WORDS)),
            0x1100_0000,
            "CH1 word 0 survives a CH0 push"
        );
    }

    #[test]
    fn mem_size_two_lets_channel0_borrow_the_second_block() {
        let mut r = rmt();
        // Default MEM_SIZE=1 → CH0 owns 48 words.
        assert_eq!(r.mem_window_words(0), RMT_BLOCK_WORDS, "default 1 block");

        // Set CH0CONF0 MEM_SIZE (bits[19:16]) = 2 → CH0 owns 96 words.
        r.write_word(0x20, 2 << 16);
        assert_eq!(r.mem_size_blocks(0), 2, "MEM_SIZE decoded");
        assert_eq!(r.mem_window_words(0), 2 * RMT_BLOCK_WORDS, "96 words");

        // Push 50 entries: with a 96-word window entry 48 lands in the SECOND
        // block (word 48) instead of wrapping onto word 0.
        for i in 0..50u32 {
            r.write_word(0x00, 0xE000_0000 | i);
        }
        assert_eq!(r.mem[0], 0xE000_0000, "word 0 NOT overwritten");
        assert_eq!(
            r.mem[RMT_BLOCK_WORDS],
            0xE000_0000 | 48,
            "entry 48 borrowed into the second block"
        );
        assert_eq!(r.mem[RMT_BLOCK_WORDS + 1], 0xE000_0000 | 49);
        // The borrowed block is visible through the direct aperture too.
        assert_eq!(
            r.read_word(rmtmem_off(RMT_BLOCK_WORDS)),
            0xE000_0000 | 48,
            "borrowed word readable at RMTMEM"
        );

        // Now it wraps at 96, not 48.
        for i in 50..97u32 {
            r.write_word(0x00, 0xE000_0000 | i);
        }
        assert_eq!(r.mem[0], 0xE000_0000 | 96, "wrapped at the 96-word window");
    }

    #[test]
    fn mem_size_window_is_clamped_to_the_end_of_rmtmem() {
        let mut r = rmt();
        // MEM_SIZE=0 is illegal on silicon; treated as one block so the FIFO
        // modulus is never zero (a `% 0` would panic).
        r.write_word(0x20, 0);
        assert_eq!(r.mem_size_blocks(0), 1, "MEM_SIZE=0 → 1 block");
        r.write_word(0x00, 0xABCD_0000);
        assert_eq!(r.mem[0], 0xABCD_0000, "MEM_SIZE=0 channel still usable");

        // An over-large MEM_SIZE on the LAST channel is clamped to the end of
        // the 384-word store rather than indexing out of bounds.
        r.write_word(0x48, 0xF << 24); // CH7 CONF0, MEM_SIZE=15
        assert_eq!(r.mem_size_blocks(7), 15);
        assert_eq!(
            r.mem_window_words(7),
            RMT_BLOCK_WORDS,
            "CH7 clamped to the single block that fits"
        );
        // Hammer the FIFO: must stay in bounds and never panic.
        for i in 0..200u32 {
            r.write_word(0x1C, i);
        }
        assert_eq!(r.read_word(0x1C), 199, "CH7 FIFO still sane");
    }

    #[test]
    fn rmtmem_byte_and_halfword_writes_are_panic_free() {
        // RMTMEM is word-accessed by real drivers, but sub-word access must
        // stay panic-free under panic=abort (mirrors the CHnDATA byte test).
        // Unlike CHnDATA there is no write pointer, so the byte-lane RMW
        // assembles in place and the final word is exact.
        let mut r = rmt();
        let off = rmtmem_off(5);
        r.write(off, 0xEF).unwrap();
        r.write(off + 1, 0xBE).unwrap();
        r.write(off + 2, 0xAD).unwrap();
        r.write(off + 3, 0xDE).unwrap();
        assert_eq!(
            r.read_word(off),
            0xDEAD_BEEF,
            "byte lanes assemble in place"
        );

        // Byte reads return the matching lane.
        assert_eq!(r.read(off).unwrap(), 0xEF);
        assert_eq!(r.read(off + 3).unwrap(), 0xDE);

        // Halfword-ish access (two byte writes) into the last word, and an
        // unaligned offset inside the aperture — neither may panic.
        let last = rmtmem_off(383);
        r.write(last, 0x34).unwrap();
        r.write(last + 1, 0x12).unwrap();
        assert_eq!(r.read_word(last), 0x0000_1234);
    }

    // ── RMT Stage 2: timed pad playback → GpioObserver ────────────────────

    /// Symbol decode is pure: entries → ascending pad-level transitions, honoring
    /// the idle level, END markers, and the DIV_CNT sim-cycle scaling.
    #[test]
    fn decode_tx_edges_builds_timed_transition_list() {
        let mut r = rmt();
        // DIV_CNT=1, MEM_SIZE=1 on CH0.
        r.write_word(0x20, (1 << 8) | (1 << 16));
        // E0: (3,H)(2,L)  E1: (4,H)(2,L)  E2: END. Idle low.
        r.mem[0] = 3 | (1 << 15) | (2 << 16);
        r.mem[1] = 4 | (1 << 15) | (2 << 16);
        r.mem[2] = 0;
        // Cumulative offsets: H@0, L@3, H@(3+2)=5, L@(5+4)=9.
        assert_eq!(
            r.decode_tx_edges(0),
            vec![(0, true), (3, false), (5, true), (9, false)]
        );

        // DIV_CNT=4 scales every offset by 4.
        r.write_word(0x20, (4 << 8) | (1 << 16));
        assert_eq!(
            r.decode_tx_edges(0),
            vec![(0, true), (12, false), (20, true), (36, false)]
        );

        // A leading same-as-idle (low) pulse emits no edge until the first high.
        r.write_word(0x20, (1 << 8) | (1 << 16));
        r.mem[0] = 5 | (2 << 16) | (1 << 31); // (5,L)(2,H) — level0 low (bit15=0)
        r.mem[1] = 0;
        assert_eq!(r.decode_tx_edges(0), vec![(5, true)]);
    }

    /// End-to-end: an RMT TX channel routed to GPIO48 through the output matrix
    /// drives the pad with the exact timed edge sequence its symbols encode, and
    /// a GpioObserver captures every edge with correct sim-cycle timing. This is
    /// the RMT → pad → observer foundation a WS2812 decoder (Stage 3) sits on.
    #[test]
    fn tx_playback_drives_routed_pad_with_timed_edges() {
        use crate::peripherals::esp32s3::gpio::{Esp32s3Gpio, GpioObserver, RMT_SIG_OUT0};
        use std::sync::{Arc, Mutex};

        #[derive(Debug, Default)]
        struct Rec {
            events: Mutex<Vec<(u8, bool, bool, u64)>>,
        }
        impl GpioObserver for Rec {
            fn on_pin_change(&self, pin: u8, from: bool, to: bool, cyc: u64) {
                self.events.lock().unwrap().push((pin, from, to, cyc));
            }
        }

        const GPIO_BASE: u64 = 0x6000_4000;
        const RMT_BASE: u64 = 0x6001_6000;
        // GPIO48's FUNC_OUT_SEL_CFG (base 0x554, stride 4).
        const FUNC_OUT_SEL48: u64 = GPIO_BASE + 0x554 + 48 * 4;

        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "gpio",
            GPIO_BASE,
            0x1000,
            None,
            Box::new(Esp32s3Gpio::new()),
        );
        bus.add_peripheral("rmt", RMT_BASE, 0x1000, Some(40), Box::new(rmt()));

        // Register a recording observer on the GPIO pad.
        let obs = Arc::new(Rec::default());
        {
            let idx = bus.find_peripheral_index_by_name("gpio").unwrap();
            let g = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<Esp32s3Gpio>()
                .unwrap();
            g.add_observer(obs.clone());
        }

        // Route GPIO48 (onboard NeoPixel) to RMT channel 0's output signal.
        bus.write_u32(FUNC_OUT_SEL48, RMT_SIG_OUT0).unwrap();

        // Load a 2-entry symbol sequence into CH0's RMTMEM (direct aperture,
        // 0x400). Entry = dur0 | lvl0<<15 | dur1<<16 | lvl1<<31.
        //   E0: (3,H)(2,L)  E1: (4,H)(2,L)  E2: END → H@0, L@3, H@5, L@9.
        bus.write_u32(RMT_BASE + 0x400, 3 | (1 << 15) | (2 << 16))
            .unwrap();
        bus.write_u32(RMT_BASE + 0x404, 4 | (1 << 15) | (2 << 16))
            .unwrap();
        bus.write_u32(RMT_BASE + 0x408, 0).unwrap();

        // Configure CH0 (DIV_CNT=1, MEM_SIZE=1) THEN start — the byte-decomposed
        // write path arms the playback when the TX_START byte lands, capturing
        // the just-written DIV/MEM.
        bus.write_u32(RMT_BASE + 0x20, (1 << 8) | (1 << 16))
            .unwrap();
        bus.write_u32(RMT_BASE + 0x20, TX_START_BIT | (1 << 8) | (1 << 16))
            .unwrap();

        // One bus tick per sim cycle; edge at offset D lands at sim cycle D.
        for _ in 0..12 {
            bus.tick_peripherals();
        }

        assert_eq!(
            *obs.events.lock().unwrap(),
            vec![
                (48, false, true, 0),
                (48, true, false, 3),
                (48, false, true, 5),
                (48, true, false, 9),
            ],
            "RMT must drive GPIO48 with the exact timed edge sequence"
        );
        // Playback drained → the RMT drops off the bus-tick pass (idle cost 0).
        let idx = bus.find_peripheral_index_by_name("rmt").unwrap();
        let r = bus.peripherals[idx]
            .dev
            .as_any()
            .unwrap()
            .downcast_ref::<Esp32s3Rmt>()
            .unwrap();
        assert!(!r.needs_bus_tick(), "playback finished → no more bus ticks");
    }

    /// An unrouted TX channel (no pad selects its signal) plays nothing: the
    /// playback resolves to no pad and is dropped, so nothing is driven and the
    /// bus-tick opt-out disengages. Guards the byte-identical rule — an RMT
    /// TX_START with the GPIO matrix at reset must not disturb any pad.
    #[test]
    fn tx_playback_without_routing_drives_no_pad() {
        use crate::peripherals::esp32s3::gpio::Esp32s3Gpio;

        const GPIO_BASE: u64 = 0x6000_4000;
        const RMT_BASE: u64 = 0x6001_6000;

        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "gpio",
            GPIO_BASE,
            0x1000,
            None,
            Box::new(Esp32s3Gpio::new()),
        );
        bus.add_peripheral("rmt", RMT_BASE, 0x1000, Some(40), Box::new(rmt()));

        // Load symbols but DO NOT route any pad to an RMT signal.
        bus.write_u32(RMT_BASE + 0x400, 3 | (1 << 15) | (2 << 16))
            .unwrap();
        bus.write_u32(RMT_BASE + 0x404, 0).unwrap();
        bus.write_u32(RMT_BASE + 0x20, (1 << 8) | (1 << 16))
            .unwrap();
        bus.write_u32(RMT_BASE + 0x20, TX_START_BIT | (1 << 8) | (1 << 16))
            .unwrap();

        for _ in 0..8 {
            bus.tick_peripherals();
        }

        // No pad was driven: GPIO OUT (bank 0) and OUT1 (bank 1) stay 0.
        assert_eq!(bus.read_u32(GPIO_BASE + 0x04).unwrap(), 0, "OUT untouched");
        assert_eq!(bus.read_u32(GPIO_BASE + 0x10).unwrap(), 0, "OUT1 untouched");
        // TX_END latched after deferred tick (bus.tick_peripherals above).
        assert_eq!(bus.read_u32(RMT_BASE + 0x70).unwrap(), tx_end_bit(0));
    }

    #[test]
    fn unmapped_offsets_in_the_1000_window_are_sane() {
        let mut r = rmt();
        // The parent maps 0x1000 bytes; the holes between the register file
        // (ends 0xD0), RMTMEM (0x400..0xA00) and the end of the window must
        // read 0 and swallow writes without panicking or aliasing RMTMEM.
        for off in [0xD0u64, 0x100, 0x3FC, 0xA00, 0xC00, 0xFFC] {
            assert_eq!(r.read_word(off), 0, "unmapped {off:#x} reads 0");
            r.write_word(off, 0xFFFF_FFFF);
            assert_eq!(r.read_word(off), 0, "unmapped {off:#x} still reads 0");
        }
        // Crucially, those writes did not land anywhere in RMTMEM.
        assert!(r.mem.iter().all(|&w| w == 0), "RMTMEM untouched by holes");

        // Byte access to the holes is equally harmless.
        r.write(0x3FF, 0xAA).unwrap();
        r.write(0xFFF, 0xAA).unwrap();
        assert_eq!(r.read(0x3FF).unwrap(), 0);
        assert!(r.mem.iter().all(|&w| w == 0), "RMTMEM still untouched");
    }
}
