// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! GDMA (General DMA) controller for ESP32-S3.
//!
//! Base = `DR_REG_GDMA_BASE` = `0x6003_F000`. The GDMA has **5 channels**,
//! each carrying an independent **IN (RX)** datapath and **OUT (TX)**
//! datapath, plus a global `MISC_CONF`. Peripherals (SPI2/3, I2S, ADC,
//! AES, SHA, …) are bound to a channel and the channel walks an in-RAM
//! linked list of DMA descriptors to move data to/from peripheral FIFOs.
//!
//! ## Register layout (verified against esp-idf
//! `components/soc/esp32s3/register/soc/gdma_reg.h`)
//!
//! The register file is laid out as a flat array of per-channel blocks with
//! a **per-channel stride of `0xC0`**. `GDMA_IN_CONF0_CH0_REG` is at offset
//! `0x0`, `GDMA_IN_CONF0_CH1_REG` at `0xC0`, etc. Within a channel block the
//! IN (RX) sub-block starts at `+0x00` and the OUT (TX) sub-block at `+0x60`.
//!
//! Per channel `n` (block base = `n * 0xC0`):
//!
//! | Block off | Name              | Notes |
//! |----------:|-------------------|-------|
//! |   0x00    | IN_CONF0          | RX config 0 (bit 4 = MEM_TRANS_EN) |
//! |   0x04    | IN_CONF1          | RX config 1 (R/W round-trip) |
//! |   0x08    | IN_INT_RAW        | bit0 IN_DONE, bit1 IN_SUC_EOF, bit2 IN_ERR_EOF, bit3 IN_DSCR_ERR |
//! |   0x0C    | IN_INT_ST         | RAW & ENA (RO) |
//! |   0x10    | IN_INT_ENA        | per-bit enable (R/W) |
//! |   0x14    | IN_INT_CLR        | W1C of IN_INT_RAW |
//! |   0x20    | IN_LINK           | addr[19:0], stop[21], start[22], restart[23], park[24] (RO=1) |
//! |   0x48    | IN_PERI_SEL       | bits[5:0] peripheral id (R/W); reset = 0x3F (unbound) |
//! |   0x60    | OUT_CONF0         | TX config 0 (R/W round-trip) |
//! |   0x64    | OUT_CONF1         | TX config 1 (R/W round-trip) |
//! |   0x68    | OUT_INT_RAW       | bit0 OUT_DONE, bit1 OUT_EOF, bit2 OUT_DSCR_ERR, bit3 OUT_TOTAL_EOF |
//! |   0x6C    | OUT_INT_ST        | RAW & ENA (RO) |
//! |   0x70    | OUT_INT_ENA       | per-bit enable (R/W) |
//! |   0x74    | OUT_INT_CLR       | W1C of OUT_INT_RAW |
//! |   0x80    | OUT_LINK          | addr[19:0], stop[20], start[21], restart[22], park[23] (RO=1) |
//! |   0xA8    | OUT_PERI_SEL      | bits[5:0] peripheral id (R/W); reset = 0x3F (unbound) |
//!
//! Global: `MISC_CONF` at absolute offset `0x3C8` (R/W round-trip).
//!
//! ## Interrupt sources (esp-idf `soc/esp32s3/include/soc/interrupts.h`)
//!
//! The interrupt-matrix source enum starts at 0 (`ETS_WIFI_MAC=0`), with
//! known anchors `ETS_LEDC=35`, `ETS_RMT=40`. Counting forward,
//! `ETS_DMA_IN_CH0_INTR_SOURCE = 66`, and the ten DMA sources are
//! contiguous: IN_CH0..IN_CH4 = 66..70, then OUT_CH0..OUT_CH4 = 71..75.
//! This peripheral emits source `base + n` for channel `n`'s IN line and
//! `base + 5 + n` for its OUT line, where `base` is the `dma_in_ch0_source`
//! constructor argument (66 on real ESP32-S3).
//!
//! ## Descriptor format (ESP32-S3 TRM §3.4.2 "Linked List Descriptor")
//!
//! Each descriptor is three 32-bit words in RAM (little-endian):
//!
//! | Word | Bits    | Name    | Notes |
//! |-----:|---------|---------|-------|
//! | dw0  | 31      | owner   | 1=DMA owns, 0=CPU; model skips owner=0 descriptors |
//! | dw0  | 30      | suc_eof | TX: last descriptor in chain; RX: set by HW on last |
//! | dw0  | 23:12   | length  | Bytes actually in buffer (TX) or capacity used (RX) |
//! | dw0  | 11:0    | size    | Buffer capacity in bytes |
//! | dw1  |         | buffer  | Full 32-bit bus address of the data buffer |
//! | dw2  |         | next    | Full 32-bit address of next descriptor, or 0 = EOL |
//!
//! ## Memory-to-memory (MEM_TRANS_EN) transfers — what is modelled
//!
//! When bit 4 (`MEM_TRANS_EN`) of `IN_CONF0` is set and both `OUT_LINK` and
//! `IN_LINK` receive a `START` write, the model performs a real descriptor
//! walk and byte copy via `tick_with_bus`:
//!
//! 1. Walk the OUT (TX) descriptor chain, reading bytes from each buffer.
//! 2. Walk the IN (RX) descriptor chain, writing bytes into each buffer.
//! 3. Set `IN_SUC_EOF | IN_DONE` in `IN_INT_RAW` once all bytes are written.
//! 4. Set `OUT_EOF | OUT_TOTAL_EOF | OUT_DONE` in `OUT_INT_RAW`.
//!
//! Descriptors whose `owner` bit is 0 (CPU-owned) are skipped; the walk
//! stops at the first CPU-owned descriptor or at `next == 0`.
//!
//! ## Peripheral-coupled mode — routing split
//!
//! When `MEM_TRANS_EN` (bit 4 of `IN_CONF0`) is **clear**, the link-start
//! path consults `IN_PERI_SEL` / `OUT_PERI_SEL` to decide how to proceed:
//!
//! **Coupled set** (`Spi2`, `Spi3`, `Uhci0`, `I2s0`, `I2s1`): the direction
//! is marked `pending_coupled`; `needs_bus_tick` returns `true`; byte movement
//! runs inside `tick_with_bus` via the peripheral pumps (UART and SPI2/3 are
//! implemented; I2S is Task 4 of the Slice 3A plan). For a coupled peripheral
//! without a pump EOF stays **unlatched** — the transfer visibly hangs rather
//! than silently auto-completing.
//!
//! **Fallback set** (everything else, including `AES`, `SHA`, `ADC_DAC`,
//! `RMT`, `LCD_CAM`, `Unknown`, and the reset / unbound value `0x3F`): the
//! original auto-complete behaviour is preserved — writing `OUTLINK_START`
//! latches `OUT_EOF + OUT_TOTAL_EOF + OUT_DONE`; writing `INLINK_START`
//! latches `IN_SUC_EOF + IN_DONE` — without actual byte movement. Firmware
//! that never writes `PERI_SEL` gets `0x3F` (unbound → `Unknown`) and falls
//! through here, preserving full backwards compatibility.
//!
//! ## SPI2/3 coupling mechanism — design decision
//!
//! GDMA and the GP-SPI controllers are separate peripherals on the bus and
//! the byte handoff cannot ride MMIO: the SPI `W0..W15` buffer is the CPU
//! (non-DMA) data path, and the GP-SPI block has no FIFO data-port register
//! (unlike UART0's FIFO at offset 0x00 that the UHCI0 pump uses above).
//! `PeripheralTickResult::dma_requests` (the STM32 DMA pattern) was
//! evaluated first and rejected: it expresses flat src→dst byte copies
//! issued from a bus-less `tick()` and executed afterwards by the bus, so
//! it can neither walk descriptor chains (which needs bus reads mid-walk)
//! nor obtain MISO bytes from the SPI's attached-device model. Instead the
//! SPI pump uses the same temporary-swap idiom the bus itself uses to lend
//! `&mut self` into `tick_with_bus` (see `bus/mod.rs`): downcast the bus to
//! `SystemBus`, swap the `Esp32s3Spi` instance out behind a stub, exchange
//! one burst of wire bytes via `Esp32s3Spi::dma_transfer`, swap it back.
//! TX and RX may be bound on *different* GDMA channels (ESP-IDF's
//! `gdma_new_channel` allocates them independently), so the pump pairs the
//! OUT and IN directions by PERI_SEL value, not by channel index.

use crate::peripherals::esp32s3::gpspi::Esp32s3Spi;
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};

/// Number of GDMA channels on the ESP32-S3.
const NUM_CHANNELS: usize = 5;

/// Per-channel register-block stride (`GDMA_IN_CONF0_CH1 - GDMA_IN_CONF0_CH0`).
const CHANNEL_STRIDE: u64 = 0xC0;

/// Absolute offset of the global `GDMA_MISC_CONF_REG`.
const MISC_CONF_OFFSET: u64 = 0x3C8;

// ── IN (RX) sub-block offsets within a channel block ──
const IN_CONF0: u64 = 0x00;
const IN_CONF1: u64 = 0x04;
const IN_INT_RAW: u64 = 0x08;
const IN_INT_ST: u64 = 0x0C;
const IN_INT_ENA: u64 = 0x10;
const IN_INT_CLR: u64 = 0x14;
const IN_LINK: u64 = 0x20;
/// GDMA_IN_PERI_SEL_CHn: binds IN direction to a peripheral.
/// Offset verified against esp-idf gdma_struct.h + esp-pacs esp32s3 DMA PAC.
const IN_PERI_SEL: u64 = 0x48;

// ── OUT (TX) sub-block offsets within a channel block ──
const OUT_CONF0: u64 = 0x60;
const OUT_CONF1: u64 = 0x64;
const OUT_INT_RAW: u64 = 0x68;
const OUT_INT_ST: u64 = 0x6C;
const OUT_INT_ENA: u64 = 0x70;
const OUT_INT_CLR: u64 = 0x74;
const OUT_LINK: u64 = 0x80;
/// GDMA_OUT_PERI_SEL_CHn: binds OUT direction to a peripheral.
/// Offset verified against esp-idf gdma_struct.h + esp-pacs esp32s3 DMA PAC.
const OUT_PERI_SEL: u64 = 0xA8;

// ── IN interrupt bits (IN_INT_*_CH*) ──
const IN_DONE_BIT: u32 = 1 << 0;
const IN_SUC_EOF_BIT: u32 = 1 << 1;
#[allow(dead_code)]
const IN_ERR_EOF_BIT: u32 = 1 << 2;
#[allow(dead_code)]
const IN_DSCR_ERR_BIT: u32 = 1 << 3;

// ── OUT interrupt bits (OUT_INT_*_CH*) ──
const OUT_DONE_BIT: u32 = 1 << 0;
const OUT_EOF_BIT: u32 = 1 << 1;
#[allow(dead_code)]
const OUT_DSCR_ERR_BIT: u32 = 1 << 2;
const OUT_TOTAL_EOF_BIT: u32 = 1 << 3;

// ── IN_LINK (0x20) bit positions ──
const IN_LINK_ADDR_MASK: u32 = 0x000F_FFFF;
const IN_LINK_STOP_BIT: u32 = 1 << 21;
const IN_LINK_START_BIT: u32 = 1 << 22;
const IN_LINK_RESTART_BIT: u32 = 1 << 23;
const IN_LINK_PARK_BIT: u32 = 1 << 24;

// ── OUT_LINK (0x80) bit positions ──
const OUT_LINK_ADDR_MASK: u32 = 0x000F_FFFF;
const OUT_LINK_STOP_BIT: u32 = 1 << 20;
const OUT_LINK_START_BIT: u32 = 1 << 21;
const OUT_LINK_RESTART_BIT: u32 = 1 << 22;
const OUT_LINK_PARK_BIT: u32 = 1 << 23;

// ── IN_CONF0 bit positions ──
/// MEM_TRANS_EN (bit 4): selects memory-to-memory mode on this channel.
const MEM_TRANS_EN_BIT: u32 = 1 << 4;

/// 6-bit mask for PERI_SEL fields; bits [31:6] are reserved.
const PERI_SEL_MASK: u32 = 0x3F;

/// Reset value for IN/OUT_PERI_SEL: no peripheral bound ("unbound").
/// This value is preserved across reset and keeps the legacy auto-complete
/// behaviour for firmware that never writes PERI_SEL.
const PERI_SEL_RESET: u32 = 0x3F;

/// Peripheral targets GDMA can couple to.
///
/// Values are per the ESP32-S3 TRM / `gdma_struct.h` PERI_IN_SEL / PERI_OUT_SEL
/// encoding verified in `docs/esp32s3_gdma_peri_sel.md` (Task 0 ground truth).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DmaPeripheral {
    /// GP-SPI2 master/slave (sel = 0)
    Spi2,
    /// GP-SPI3 master/slave (sel = 1)
    Spi3,
    /// UHCI0 bridge → UART DMA path (sel = 2)
    Uhci0,
    /// I2S0 TX/RX (sel = 3)
    I2s0,
    /// I2S1 TX/RX (sel = 4)
    I2s1,
    /// LCD/camera controller (sel = 5) — deferred, fallback behaviour
    LcdCam,
    /// AES accelerator (sel = 6) — fallback auto-complete
    Aes,
    /// SHA accelerator (sel = 7) — fallback auto-complete
    Sha,
    /// SAR ADC (sel = 8) — fallback auto-complete
    AdcDac,
    /// RMT controller (sel = 9) — fallback auto-complete
    Rmt,
    /// Unrecognised or unbound (0x3F = reset / no selection, others unknown)
    Unknown(u32),
}

impl DmaPeripheral {
    fn from_sel(v: u32) -> Self {
        match v & PERI_SEL_MASK {
            0 => Self::Spi2,
            1 => Self::Spi3,
            2 => Self::Uhci0,
            3 => Self::I2s0,
            4 => Self::I2s1,
            5 => Self::LcdCam,
            6 => Self::Aes,
            7 => Self::Sha,
            8 => Self::AdcDac,
            9 => Self::Rmt,
            other => Self::Unknown(other),
        }
    }

    /// True when this peripheral is in the "coupled set" — byte movement is
    /// handled by `tick_with_bus` (Tasks 2–4 fill in the implementations).
    fn is_coupled(self) -> bool {
        matches!(
            self,
            Self::Spi2 | Self::Spi3 | Self::Uhci0 | Self::I2s0 | Self::I2s1
        )
    }
}

/// High-address prefix added to the 20-bit LINK_ADDR field to form a full
/// 32-bit bus address. On ESP32-S3, GDMA descriptors must reside in internal
/// SRAM which is mapped at 0x3FC0_0000; the INLINK/OUTLINK registers carry
/// only bits [19:0] of the descriptor address and the upper 12 bits are
/// implicitly `0x3FC`. This matches the linker-assigned DRAM range and the
/// firmware `LINK_ADDR_MASK = 0x000F_FFFF` masking seen in the Tier-1
/// fixture (and in ESP-IDF drivers).
const DRAM_ADDR_PREFIX: u32 = 0x3FC0_0000;

/// Maximum number of descriptor hops per channel walk (safety guard against
/// infinite loops in corrupted descriptor chains).
const MAX_DESC_CHAIN: usize = 4096;

/// Descriptor dw0 bit positions.
const DESC_OWNER_BIT: u32 = 1 << 31;
/// Descriptor dw0 "length" field (bits [23:12]) — bytes valid in the buffer.
/// On IN (RX) descriptors the hardware writes this field on completion; the
/// writeback must CLEAR it first so stale CPU-seeded values don't OR in.
const DESC_LEN_MASK: u32 = 0xFFF << 12;

// ── UHCI0 / UART0 coupling constants ─────────────────────────────────────
//
// UHCI0 bridges GDMA to UART0 by default (ESP-IDF `uart_ll.h` / TRM §26).
// `DR_REG_UART0_BASE = 0x6000_0000` (verified: uart.rs module doc + xtensa.rs
// `configure_xtensa_esp32s3` registration).
//
// FIFO register: offset 0x00 — W pushes a TX byte; R pops one RX byte.
// STATUS register: offset 0x1C — RXFIFO_CNT[9:0] (bits 9:0),
//                                 TXFIFO_CNT[9:0] (bits 25:16).
const UART0_BASE: u64 = 0x6000_0000;
const UART0_FIFO_ADDR: u64 = UART0_BASE; // offset 0x00
const UART0_STATUS_ADDR: u64 = UART0_BASE + 0x1C;

/// STATUS register bit masks.
const UART_RXFIFO_CNT_MASK: u32 = 0x3FF; // bits [9:0]
/// TXFIFO_CNT[25:16] shift — retained for documentation; used when checking
/// TX back-pressure (not currently needed since we write unconditionally).
#[allow(dead_code)]
const UART_TXFIFO_CNT_SHIFT: u32 = 16;
#[allow(dead_code)]
const UART_TXFIFO_CNT_MASK: u32 = 0x3FF; // 10-bit field

/// Hardware FIFO depth for ESP32-S3 (`SOC_UART_FIFO_LEN = 128`).
/// Retained for documentation; the model writes unconditionally (FIFO overflow
/// is handled by the UART peripheral itself — it drops excess bytes and latches
/// RXFIFO_OVF just like silicon).
#[allow(dead_code)]
const UART_FIFO_LEN: u32 = 128;

// ── SPI2/3 coupled DMA constants ─────────────────────────────────────────
/// Registered name for GP-SPI2 in the system bus (real base `0x6002_4000`).
const SPI2_S3_NAME: &str = "spi2_s3";
/// Registered name for GP-SPI3 in the system bus (real base `0x6002_5000`).
const SPI3_S3_NAME: &str = "spi3_s3";

/// Maximum bytes transferred per `tick_with_bus` call for a coupled channel.
///
/// Bounds latency per tick to a realistic burst size. 64 bytes matches the
/// typical DMA burst used by ESP-IDF `uart_ll.h` (half the 128-deep FIFO)
/// and keeps the simulation engine responsive on long transfers.
const COUPLED_BYTES_PER_TICK: usize = 64;

/// One direction (IN or OUT) of a GDMA channel.
#[derive(Debug, Clone, Copy)]
struct DmaDir {
    conf0: u32,
    conf1: u32,
    /// Latched descriptor-list base address (bits[19:0] of the LINK reg).
    link_addr: u32,
    /// INT_RAW — sticky pending bits, cleared only by INT_CLR (W1C).
    int_raw: u32,
    /// INT_ENA — per-bit IRQ enable.
    int_ena: u32,
    /// PERI_SEL register value (6-bit masked; reset = 0x3F = unbound).
    peri_sel: u32,
    /// Set when this direction has been started in coupled mode (PERI_SEL
    /// names a coupled peripheral and MEM_TRANS_EN is clear). Cleared by
    /// `tick_with_bus` once the transfer completes (EOF latched).
    pending_coupled: bool,
    // ── Incremental coupled-walk state ──────────────────────────────────
    // These fields track progress across multiple `tick_with_bus` calls so
    // that a transfer larger than COUPLED_BYTES_PER_TICK resumes rather
    // than restarting from the head of the descriptor chain each tick.
    // M2M (one-shot) walks ignore these fields entirely.
    //
    /// Current descriptor address for the incremental walk (0 = not started).
    coupled_desc_ptr: u64,
    /// Byte offset within the current descriptor's buffer (OUT direction:
    /// how many bytes of this descriptor have been consumed; IN direction:
    /// how many bytes have been written into this descriptor so far).
    coupled_buf_offset: u32,
}

impl Default for DmaDir {
    fn default() -> Self {
        Self {
            conf0: 0,
            conf1: 0,
            link_addr: 0,
            int_raw: 0,
            int_ena: 0,
            peri_sel: PERI_SEL_RESET,
            pending_coupled: false,
            coupled_desc_ptr: 0,
            coupled_buf_offset: 0,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Channel {
    rx: DmaDir,
    tx: DmaDir,
    /// True when IN_LINK received a START while MEM_TRANS_EN was set.
    in_started: bool,
    /// True when OUT_LINK received a START while MEM_TRANS_EN was set.
    out_started: bool,
    /// True when both `in_started` and `out_started` are set (i.e. both
    /// INLINK_START and OUTLINK_START have been written with MEM_TRANS_EN
    /// active). The `tick_with_bus` pass reads the OUT descriptor chain,
    /// copies bytes into the IN chain, latches EOF, then clears all flags.
    pending_m2m: bool,
}

/// ESP32-S3 GDMA controller — 5 channels × {IN, OUT}.
#[derive(Debug)]
pub struct Esp32s3Gdma {
    channels: [Channel; NUM_CHANNELS],
    /// `GDMA_MISC_CONF_REG` (round-tripped only).
    misc_conf: u32,
    /// Interrupt-matrix source ID for IN channel 0 (66 on real silicon).
    /// IN_CHn = base + n; OUT_CHn = base + 5 + n.
    dma_in_ch0_source: u32,
}

impl Esp32s3Gdma {
    /// `dma_in_ch0_source` is the interrupt-matrix source ID bound to RX
    /// channel 0 (`ETS_DMA_IN_CH0_INTR_SOURCE` = 66 on ESP32-S3). The other
    /// nine DMA lines are derived contiguously from it.
    pub fn new(dma_in_ch0_source: u32) -> Self {
        Self {
            channels: [Channel::default(); NUM_CHANNELS],
            misc_conf: 0,
            dma_in_ch0_source,
        }
    }

    /// Decode an absolute window offset into `(channel_index, block_offset)`
    /// for offsets that fall inside a per-channel block. Returns `None` for
    /// the global region (e.g. MISC_CONF) or out-of-range offsets.
    fn channel_of(offset: u64) -> Option<(usize, u64)> {
        let ch = (offset / CHANNEL_STRIDE) as usize;
        if ch >= NUM_CHANNELS {
            return None;
        }
        Some((ch, offset % CHANNEL_STRIDE))
    }

    fn read_word(&self, offset: u64) -> u32 {
        if offset == MISC_CONF_OFFSET {
            return self.misc_conf;
        }
        let Some((ch, blk)) = Self::channel_of(offset) else {
            return 0;
        };
        let c = &self.channels[ch];
        match blk {
            IN_CONF0 => c.rx.conf0,
            IN_CONF1 => c.rx.conf1,
            IN_INT_RAW => c.rx.int_raw,
            IN_INT_ST => c.rx.int_raw & c.rx.int_ena,
            IN_INT_ENA => c.rx.int_ena,
            // IN_INT_CLR is W1C/write-only; reads as 0.
            IN_INT_CLR => 0,
            // PARK bit (24) reads 1 when the channel is idle (not actively
            // walking a list). We model transfers as instantaneous, so the
            // channel is always parked; START self-clears immediately.
            IN_LINK => (c.rx.link_addr & IN_LINK_ADDR_MASK) | IN_LINK_PARK_BIT,
            IN_PERI_SEL => c.rx.peri_sel & PERI_SEL_MASK,
            OUT_CONF0 => c.tx.conf0,
            OUT_CONF1 => c.tx.conf1,
            OUT_INT_RAW => c.tx.int_raw,
            OUT_INT_ST => c.tx.int_raw & c.tx.int_ena,
            OUT_INT_ENA => c.tx.int_ena,
            OUT_INT_CLR => 0,
            OUT_LINK => (c.tx.link_addr & OUT_LINK_ADDR_MASK) | OUT_LINK_PARK_BIT,
            OUT_PERI_SEL => c.tx.peri_sel & PERI_SEL_MASK,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if offset == MISC_CONF_OFFSET {
            self.misc_conf = value;
            return;
        }
        let Some((ch, blk)) = Self::channel_of(offset) else {
            return;
        };
        let c = &mut self.channels[ch];
        match blk {
            IN_CONF0 => c.rx.conf0 = value,
            IN_CONF1 => c.rx.conf1 = value,
            // INT_RAW is R/WTC (write-to-clear via the CLR register); ignore
            // direct writes to RAW, matching silicon's CLR-driven model.
            IN_INT_RAW => {}
            // INT_ST is RO.
            IN_INT_ST => {}
            IN_INT_ENA => c.rx.int_ena = value,
            IN_INT_CLR => {
                // W1C: clear the matching IN_INT_RAW bits.
                c.rx.int_raw &= !value;
            }
            IN_LINK => {
                c.rx.link_addr = value & IN_LINK_ADDR_MASK;
                // INLINK_START: kick the RX channel.
                if value & (IN_LINK_START_BIT | IN_LINK_RESTART_BIT) != 0 {
                    if c.rx.conf0 & MEM_TRANS_EN_BIT != 0 {
                        // MEM_TRANS_EN: track that IN_LINK has been started.
                        // The actual copy runs in tick_with_bus once both
                        // IN_LINK and OUT_LINK have been kicked (the firmware
                        // may start them in either order).
                        c.in_started = true;
                        if c.out_started {
                            c.pending_m2m = true;
                        }
                    } else {
                        // Peripheral-coupled mode: route by PERI_SEL.
                        match DmaPeripheral::from_sel(c.rx.peri_sel) {
                            p if p.is_coupled() => {
                                // Coupled set (SPI2/3, UHCI0, I2S0/1): mark
                                // pending — byte movement runs in tick_with_bus.
                                // Initialise the incremental walk state from the
                                // just-latched link_addr so tick_with_bus starts
                                // at the head of the descriptor chain.
                                c.rx.pending_coupled = true;
                                c.rx.coupled_desc_ptr = Self::full_desc_addr(c.rx.link_addr);
                                c.rx.coupled_buf_offset = 0;
                            }
                            _ => {
                                // Fallback set (AES, SHA, ADC, RMT, LCD_CAM,
                                // Unknown including unbound 0x3F): auto-complete
                                // so firmware polling IN_SUC_EOF makes forward
                                // progress. Preserves all legacy behaviour for
                                // firmware that never writes PERI_SEL.
                                c.rx.int_raw |= IN_SUC_EOF_BIT | IN_DONE_BIT;
                            }
                        }
                    }
                }
                // STOP: nothing to do in this register-only model.
                let _ = IN_LINK_STOP_BIT;
            }
            IN_PERI_SEL => c.rx.peri_sel = value & PERI_SEL_MASK,
            OUT_CONF0 => c.tx.conf0 = value,
            OUT_CONF1 => c.tx.conf1 = value,
            OUT_INT_RAW => {}
            OUT_INT_ST => {}
            OUT_INT_ENA => c.tx.int_ena = value,
            OUT_INT_CLR => {
                c.tx.int_raw &= !value;
            }
            OUT_LINK => {
                c.tx.link_addr = value & OUT_LINK_ADDR_MASK;
                // OUTLINK_START: kick the TX channel.
                if value & (OUT_LINK_START_BIT | OUT_LINK_RESTART_BIT) != 0 {
                    if c.rx.conf0 & MEM_TRANS_EN_BIT != 0 {
                        // MEM_TRANS_EN: track that OUT_LINK has been started.
                        // Set pending_m2m when both sides are ready.
                        c.out_started = true;
                        if c.in_started {
                            c.pending_m2m = true;
                        }
                    } else {
                        // Peripheral-coupled mode: route by PERI_SEL.
                        match DmaPeripheral::from_sel(c.tx.peri_sel) {
                            p if p.is_coupled() => {
                                // Coupled set: mark pending; byte movement runs
                                // in tick_with_bus. Initialise the incremental
                                // walk state from the just-latched link_addr.
                                c.tx.pending_coupled = true;
                                c.tx.coupled_desc_ptr = Self::full_desc_addr(c.tx.link_addr);
                                c.tx.coupled_buf_offset = 0;
                            }
                            _ => {
                                // Fallback set: auto-complete.
                                c.tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
                            }
                        }
                    }
                }
                let _ = OUT_LINK_STOP_BIT;
            }
            OUT_PERI_SEL => c.tx.peri_sel = value & PERI_SEL_MASK,
            _ => {}
        }
    }

    /// Walk an OUT (TX) descriptor chain starting at `desc_addr` and collect
    /// all bytes from the data buffers. Returns the bytes if successful.
    ///
    /// Stops at the first descriptor whose `owner` bit is 0 (CPU-owned),
    /// at `next == 0` (end-of-list), or after `MAX_DESC_CHAIN` hops.
    ///
    /// After consuming each descriptor, writes back `dw0 & !DESC_OWNER_BIT`
    /// to the descriptor address so the CPU sees `owner=0` (descriptor
    /// returned to CPU) — matching the ESP32-S3 TRM §3.4 hardware behaviour.
    fn walk_out_chain(bus: &mut dyn Bus, desc_addr: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut addr = desc_addr;
        for _ in 0..MAX_DESC_CHAIN {
            if addr == 0 {
                break;
            }
            let dw0 = bus.read_u32(addr).unwrap_or(0);
            // Skip CPU-owned descriptors (owner=0).
            if dw0 & DESC_OWNER_BIT == 0 {
                break;
            }
            let length = (dw0 >> 12) & 0xFFF; // bits [23:12]
            let buf_ptr = bus.read_u32(addr + 4).unwrap_or(0) as u64;
            let next_ptr = bus.read_u32(addr + 8).unwrap_or(0) as u64;

            for i in 0..length {
                bytes.push(bus.read_u8(buf_ptr + i as u64).unwrap_or(0));
            }

            // Write back dw0 with owner bit cleared (descriptor returned to CPU).
            let _ = bus.write_u32(addr, dw0 & !DESC_OWNER_BIT);

            if next_ptr == 0 {
                break;
            }
            addr = next_ptr;
        }
        bytes
    }

    /// Walk an IN (RX) descriptor chain starting at `desc_addr` and write
    /// `bytes` into the data buffers.
    ///
    /// After writing `to_write` bytes into each descriptor, writes back dw0
    /// with the owner bit cleared and bits [23:12] set to `to_write` so the
    /// CPU sees `owner=0` and the actual received byte count — matching the
    /// ESP32-S3 TRM §3.4 hardware behaviour.
    fn walk_in_chain(bus: &mut dyn Bus, desc_addr: u64, bytes: &[u8]) {
        let mut remaining = bytes;
        let mut addr = desc_addr;
        for _ in 0..MAX_DESC_CHAIN {
            if addr == 0 || remaining.is_empty() {
                break;
            }
            let dw0 = bus.read_u32(addr).unwrap_or(0);
            // Skip CPU-owned descriptors.
            if dw0 & DESC_OWNER_BIT == 0 {
                break;
            }
            let size = (dw0 & 0xFFF) as usize; // bits [11:0] = capacity
            let buf_ptr = bus.read_u32(addr + 4).unwrap_or(0) as u64;
            let next_ptr = bus.read_u32(addr + 8).unwrap_or(0) as u64;

            let to_write = remaining.len().min(size);
            for (i, &b) in remaining[..to_write].iter().enumerate() {
                let _ = bus.write_u8(buf_ptr + i as u64, b);
            }
            remaining = &remaining[to_write..];

            // Write back dw0: owner bit cleared, length field set to bytes written.
            let _ = bus.write_u32(
                addr,
                (dw0 & !(DESC_OWNER_BIT | DESC_LEN_MASK)) | ((to_write as u32) << 12),
            );

            if next_ptr == 0 || remaining.is_empty() {
                break;
            }
            addr = next_ptr;
        }
    }

    /// Reconstruct the full 32-bit bus address from the 20-bit LINK_ADDR
    /// field. ESP32-S3 GDMA descriptors must reside in internal SRAM
    /// (`0x3FC0_0000`–`0x3FCF_FFFF`); the upper 12 bits are implicit.
    fn full_desc_addr(link_addr_20: u32) -> u64 {
        (DRAM_ADDR_PREFIX | (link_addr_20 & IN_LINK_ADDR_MASK)) as u64
    }

    /// Pump an OUT (TX) coupled UART transfer: walk the descriptor chain and
    /// write bytes into the UART TX FIFO via MMIO.
    ///
    /// Returns `true` when the chain is fully drained (transfer complete).
    ///
    /// **EOF policy (OUT/TX):** `OUT_EOF | OUT_TOTAL_EOF | OUT_DONE` are
    /// latched by the caller once this function returns `true` — i.e. after
    /// the last byte of the last descriptor has entered the FIFO.
    ///
    /// **Throughput bound:** at most `COUPLED_BYTES_PER_TICK` bytes per call,
    /// further limited by the available UART TX FIFO space (`UART_FIFO_LEN −
    /// TXFIFO_CNT`).  When the FIFO is full the pump backs off without
    /// advancing — the engine revisits on the next tick once the baud-rate
    /// drain has freed space.
    fn pump_uart_out(dir: &mut DmaDir, bus: &mut dyn Bus) -> bool {
        // Determine available TX FIFO space.
        let status = bus.read_u32(UART0_STATUS_ADDR).unwrap_or(0);
        let tx_in_use = ((status >> UART_TXFIFO_CNT_SHIFT) & UART_TXFIFO_CNT_MASK) as usize;
        let tx_free = (UART_FIFO_LEN as usize).saturating_sub(tx_in_use);

        let mut budget = COUPLED_BYTES_PER_TICK.min(tx_free);

        if budget == 0 {
            // FIFO full; wait for the baud-rate drain to free space.
            return false;
        }

        loop {
            let addr = dir.coupled_desc_ptr;
            if addr == 0 || budget == 0 {
                break;
            }

            let dw0 = bus.read_u32(addr).unwrap_or(0);
            // Skip CPU-owned descriptors; treat as end-of-chain.
            if dw0 & DESC_OWNER_BIT == 0 {
                return true; // chain drained / halted
            }

            let length = (dw0 >> 12) & 0xFFF; // bits [23:12]
            let buf_ptr = bus.read_u32(addr + 4).unwrap_or(0) as u64;
            let next_ptr = bus.read_u32(addr + 8).unwrap_or(0) as u64;

            // How many bytes remain in this descriptor?
            let remaining = length.saturating_sub(dir.coupled_buf_offset) as usize;
            let to_send = remaining.min(budget);

            for i in 0..to_send {
                let byte = bus
                    .read_u8(buf_ptr + (dir.coupled_buf_offset as u64) + i as u64)
                    .unwrap_or(0);
                let _ = bus.write_u8(UART0_FIFO_ADDR, byte);
            }
            budget -= to_send;
            dir.coupled_buf_offset += to_send as u32;

            if dir.coupled_buf_offset >= length {
                // Descriptor fully consumed; write back owner bit cleared.
                let _ = bus.write_u32(addr, dw0 & !DESC_OWNER_BIT);
                // Advance to next.
                dir.coupled_buf_offset = 0;
                if next_ptr == 0 {
                    dir.coupled_desc_ptr = 0;
                    return true; // end of chain
                }
                dir.coupled_desc_ptr = next_ptr;
            } else {
                // Partially consumed (budget or FIFO exhausted); resume next tick.
                break;
            }
        }
        false // not yet done
    }

    /// Pump an IN (RX) coupled UART transfer: read bytes from the UART RX FIFO
    /// via MMIO and write them into the descriptor chain.
    ///
    /// Returns `true` when EOF should be latched.
    ///
    /// **EOF policy (IN/RX):**
    /// - `IN_DONE` is latched per completed descriptor (when its capacity is
    ///   fully written), matching how ESP-IDF `uart_read_bytes` expects the
    ///   DMA engine to signal per-buffer completion.
    /// - `IN_SUC_EOF` is latched when the descriptor chain is fully written
    ///   **OR** when the UART RX FIFO empties after at least one byte has been
    ///   moved — whichever comes first. This mirrors the ESP-IDF
    ///   `uart_intr_handler_default` / UHCI EOF semantics: the driver wakes on
    ///   `IN_SUC_EOF` which fires as soon as the FIFO idle-timeout drains the
    ///   last byte into DMA, even if the descriptor still has spare capacity.
    ///
    /// **Throughput bound:** at most `COUPLED_BYTES_PER_TICK` bytes per call.
    ///
    /// Returns `(eof, in_done_latched)`.
    fn pump_uart_in(dir: &mut DmaDir, bus: &mut dyn Bus) -> (bool, bool) {
        // Read live RXFIFO_CNT from STATUS register.
        let status = bus.read_u32(UART0_STATUS_ADDR).unwrap_or(0);
        let mut rx_avail = (status & UART_RXFIFO_CNT_MASK) as usize;

        if rx_avail == 0 {
            // No bytes available; nothing to do this tick.
            return (false, false);
        }

        let mut budget = COUPLED_BYTES_PER_TICK;
        let mut any_moved = false;
        let mut in_done = false;

        loop {
            let addr = dir.coupled_desc_ptr;
            if addr == 0 || budget == 0 || rx_avail == 0 {
                break;
            }

            let dw0 = bus.read_u32(addr).unwrap_or(0);
            if dw0 & DESC_OWNER_BIT == 0 {
                // CPU-owned: treat as end-of-chain → EOF.
                let eof = any_moved;
                return (eof, in_done);
            }

            let size = dw0 & 0xFFF; // bits [11:0] = capacity
            let buf_ptr = bus.read_u32(addr + 4).unwrap_or(0) as u64;
            let next_ptr = bus.read_u32(addr + 8).unwrap_or(0) as u64;

            let remaining_cap = size.saturating_sub(dir.coupled_buf_offset) as usize;
            let to_recv = remaining_cap.min(budget).min(rx_avail);

            for i in 0..to_recv {
                let byte = bus.read_u8(UART0_FIFO_ADDR).unwrap_or(0);
                let _ = bus.write_u8(buf_ptr + (dir.coupled_buf_offset as u64) + i as u64, byte);
            }
            budget -= to_recv;
            rx_avail -= to_recv;
            dir.coupled_buf_offset += to_recv as u32;
            if to_recv > 0 {
                any_moved = true;
            }

            if dir.coupled_buf_offset >= size {
                // Descriptor capacity filled; write back owner bit cleared and length set.
                let _ = bus.write_u32(
                    addr,
                    (dw0 & !(DESC_OWNER_BIT | DESC_LEN_MASK)) | (size << 12),
                );
                in_done = true;
                dir.coupled_buf_offset = 0;
                if next_ptr == 0 {
                    dir.coupled_desc_ptr = 0;
                    return (true, true); // chain done → IN_SUC_EOF + IN_DONE
                }
                dir.coupled_desc_ptr = next_ptr;
            } else {
                // Descriptor partially filled.
                break;
            }
        }

        // Check again after the loop: if the FIFO is now empty and we moved
        // at least one byte, latch IN_SUC_EOF (FIFO-idle EOF).
        let status2 = bus.read_u32(UART0_STATUS_ADDR).unwrap_or(0);
        let rx_remaining = (status2 & UART_RXFIFO_CNT_MASK) as usize;
        let eof = any_moved && rx_remaining == 0;
        // If we produced EOF on a partially-filled descriptor (FIFO-idle path),
        // write back the descriptor dw0 with owner cleared and partial length.
        // `coupled_buf_offset > 0` guards the case where the moved bytes
        // exactly filled the previous descriptor: the current one received
        // nothing and must stay DMA-owned (silicon leaves it untouched).
        if eof && dir.coupled_desc_ptr != 0 && dir.coupled_buf_offset > 0 {
            let addr = dir.coupled_desc_ptr;
            let dw0 = bus.read_u32(addr).unwrap_or(0);
            if dw0 & DESC_OWNER_BIT != 0 {
                let partial_len = dir.coupled_buf_offset;
                let _ = bus.write_u32(
                    addr,
                    (dw0 & !(DESC_OWNER_BIT | DESC_LEN_MASK)) | (partial_len << 12),
                );
            }
        }
        (eof, in_done)
    }

    /// Read up to `budget` bytes from an OUT (TX) descriptor chain, resuming
    /// from `dir.coupled_desc_ptr` / `coupled_buf_offset`. Each fully
    /// consumed descriptor gets its owner bit written back to CPU (0).
    /// `coupled_desc_ptr` becomes 0 at end-of-chain (next == 0 or a
    /// CPU-owned descriptor). May return fewer bytes than `budget` when the
    /// chain is exhausted.
    fn coupled_out_collect(dir: &mut DmaDir, bus: &mut dyn Bus, budget: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(budget);
        while out.len() < budget {
            let addr = dir.coupled_desc_ptr;
            if addr == 0 {
                break;
            }
            let dw0 = bus.read_u32(addr).unwrap_or(0);
            if dw0 & DESC_OWNER_BIT == 0 {
                // CPU-owned: chain halted here.
                dir.coupled_desc_ptr = 0;
                break;
            }
            let length = (dw0 >> 12) & 0xFFF; // bits [23:12]
            let buf_ptr = bus.read_u32(addr + 4).unwrap_or(0) as u64;
            let next_ptr = bus.read_u32(addr + 8).unwrap_or(0) as u64;

            let remaining = length.saturating_sub(dir.coupled_buf_offset) as usize;
            let to_read = remaining.min(budget - out.len());
            for i in 0..to_read {
                out.push(
                    bus.read_u8(buf_ptr + dir.coupled_buf_offset as u64 + i as u64)
                        .unwrap_or(0),
                );
            }
            dir.coupled_buf_offset += to_read as u32;

            if dir.coupled_buf_offset >= length {
                // Descriptor fully consumed: return it to the CPU.
                let _ = bus.write_u32(addr, dw0 & !DESC_OWNER_BIT);
                dir.coupled_buf_offset = 0;
                dir.coupled_desc_ptr = if next_ptr == 0 { 0 } else { next_ptr };
            }
            // else: budget exhausted mid-descriptor; resume next tick.
        }
        out
    }

    /// Write `bytes` into an IN (RX) descriptor chain, resuming from
    /// `dir.coupled_desc_ptr` / `coupled_buf_offset`. Each filled descriptor
    /// gets owner cleared and the length field [23:12] set to its capacity.
    /// Bytes beyond the end of the chain are dropped (the chain was
    /// under-provisioned; real silicon raises a descriptor-empty error —
    /// not modelled).
    fn coupled_in_write(dir: &mut DmaDir, bus: &mut dyn Bus, bytes: &[u8]) {
        let mut written = 0usize;
        while written < bytes.len() {
            let addr = dir.coupled_desc_ptr;
            if addr == 0 {
                break;
            }
            let dw0 = bus.read_u32(addr).unwrap_or(0);
            if dw0 & DESC_OWNER_BIT == 0 {
                dir.coupled_desc_ptr = 0;
                break;
            }
            let size = dw0 & 0xFFF; // bits [11:0] = capacity
            let buf_ptr = bus.read_u32(addr + 4).unwrap_or(0) as u64;
            let next_ptr = bus.read_u32(addr + 8).unwrap_or(0) as u64;

            let cap = size.saturating_sub(dir.coupled_buf_offset) as usize;
            let n = cap.min(bytes.len() - written);
            for i in 0..n {
                let _ = bus.write_u8(
                    buf_ptr + dir.coupled_buf_offset as u64 + i as u64,
                    bytes[written + i],
                );
            }
            written += n;
            dir.coupled_buf_offset += n as u32;

            if dir.coupled_buf_offset >= size {
                // Capacity filled: owner back to CPU, received length = size.
                let _ = bus.write_u32(
                    addr,
                    (dw0 & !(DESC_OWNER_BIT | DESC_LEN_MASK)) | (size << 12),
                );
                dir.coupled_buf_offset = 0;
                dir.coupled_desc_ptr = if next_ptr == 0 { 0 } else { next_ptr };
            }
            // else: out of bytes mid-descriptor; resume next tick (or
            // finalize with a partial-length writeback at transaction end).
        }
    }

    /// Finalize an IN direction at transaction end: if the last descriptor
    /// is partially filled, write back owner=0 with the partial length so
    /// drivers polling the owner bit / length field see the real count.
    fn coupled_in_finalize(dir: &mut DmaDir, bus: &mut dyn Bus) {
        if dir.coupled_desc_ptr != 0 && dir.coupled_buf_offset > 0 {
            let addr = dir.coupled_desc_ptr;
            let dw0 = bus.read_u32(addr).unwrap_or(0);
            if dw0 & DESC_OWNER_BIT != 0 {
                let _ = bus.write_u32(
                    addr,
                    (dw0 & !(DESC_OWNER_BIT | DESC_LEN_MASK)) | (dir.coupled_buf_offset << 12),
                );
            }
        }
    }

    /// Service the in-flight DMA transaction (if any) of one GP-SPI
    /// controller. See the module-level "SPI2/3 coupling mechanism" section
    /// for the design rationale.
    ///
    /// Per tick, up to `COUPLED_BYTES_PER_TICK` wire bytes are exchanged:
    /// MOSI bytes come from the OUT chain of whichever channel has
    /// `OUT_PERI_SEL == peri` pending (0xFF idle-high filler when TX-DMA is
    /// disabled or the chain under-runs); each byte is exchanged with the
    /// SPI's attached devices via `dma_transfer`; MISO bytes land in the IN
    /// chain of whichever channel has `IN_PERI_SEL == peri` pending. The
    /// pump stalls (no progress, state retained) until every DMA-enabled
    /// direction has its GDMA link started — firmware may kick `USR` and
    /// the links in either order.
    ///
    /// On the tick that completes the transaction: the SPI latches
    /// TRANS_DONE (`dma_complete`), the TX channel latches
    /// `OUT_EOF | OUT_TOTAL_EOF | OUT_DONE`, the RX channel latches
    /// `IN_SUC_EOF | IN_DONE`, and both directions clear `pending_coupled`.
    fn pump_spi(&mut self, bus: &mut dyn Bus, peri: DmaPeripheral, spi_name: &str) {
        use crate::bus::SystemBus;
        use crate::peripherals::stub::StubPeripheral;

        let tx_idx = self
            .channels
            .iter()
            .position(|c| c.tx.pending_coupled && DmaPeripheral::from_sel(c.tx.peri_sel) == peri);
        let rx_idx = self
            .channels
            .iter()
            .position(|c| c.rx.pending_coupled && DmaPeripheral::from_sel(c.rx.peri_sel) == peri);
        if tx_idx.is_none() && rx_idx.is_none() {
            return;
        }

        let Some(sys_bus) = bus.as_any_mut().and_then(|a| a.downcast_mut::<SystemBus>()) else {
            return;
        };
        let Some(spi_idx) = sys_bus.find_peripheral_index_by_name(spi_name) else {
            return;
        };

        // Swap the SPI out from behind a stub — the same dance the bus uses
        // to lend itself into `tick_with_bus` — so we can hold `&mut` to the
        // SPI and still route descriptor reads/writes through the bus.
        let placeholder: Box<dyn Peripheral> = Box::new(StubPeripheral::new(0));
        let mut spi_dev = std::mem::replace(&mut sys_bus.peripherals[spi_idx].dev, placeholder);

        'work: {
            let Some(spi) = spi_dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<Esp32s3Spi>())
            else {
                break 'work;
            };
            let Some(pending) = spi.dma_pending() else {
                // GDMA links started but firmware hasn't kicked SPI_CMD.USR
                // yet — stall, keep pending_coupled so we revisit next tick.
                break 'work;
            };
            // Stall until every DMA-enabled direction has its link started.
            if (pending.tx_ena && tx_idx.is_none()) || (pending.rx_ena && rx_idx.is_none()) {
                break 'work;
            }

            let k = COUPLED_BYTES_PER_TICK.min(pending.total_bytes - pending.transferred);
            let mosi = match tx_idx {
                Some(i) if pending.tx_ena => {
                    let mut m =
                        Self::coupled_out_collect(&mut self.channels[i].tx, &mut *sys_bus, k);
                    // OUT chain under-provisioned: the TX FIFO under-runs and
                    // the line idles high for the rest of the burst.
                    m.resize(k, 0xFF);
                    m
                }
                // RX-only DMA transaction: nothing drives MOSI — idle high.
                _ => vec![0xFF; k],
            };
            let miso = spi.dma_transfer(&mosi);
            if let Some(i) = rx_idx {
                if pending.rx_ena {
                    Self::coupled_in_write(&mut self.channels[i].rx, &mut *sys_bus, &miso);
                }
            }

            if pending.transferred + k >= pending.total_bytes {
                // Transaction complete this tick. Only the directions the
                // transaction actually enabled latch EOF — a started link
                // the SPI never used stays pending (visible hang, matching
                // the module's coupled-without-pump philosophy).
                if let Some(i) = rx_idx {
                    if pending.rx_ena {
                        Self::coupled_in_finalize(&mut self.channels[i].rx, &mut *sys_bus);
                        let rx = &mut self.channels[i].rx;
                        rx.int_raw |= IN_SUC_EOF_BIT | IN_DONE_BIT;
                        rx.pending_coupled = false;
                        rx.coupled_desc_ptr = 0;
                        rx.coupled_buf_offset = 0;
                    }
                }
                if let Some(i) = tx_idx {
                    if pending.tx_ena {
                        let tx = &mut self.channels[i].tx;
                        tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
                        tx.pending_coupled = false;
                        tx.coupled_desc_ptr = 0;
                        tx.coupled_buf_offset = 0;
                    }
                }
                spi.dma_complete();
            }
        }

        sys_bus.peripherals[spi_idx].dev = spi_dev;
    }

    /// Execute all pending descriptor walks and coupled-mode ticks.
    ///
    /// For each channel with `pending_m2m` set:
    /// 1. Walk the OUT (TX) descriptor chain and collect bytes.
    /// 2. Walk the IN (RX) descriptor chain and write bytes.
    /// 3. Latch `IN_SUC_EOF | IN_DONE` and `OUT_EOF | OUT_TOTAL_EOF |
    ///    OUT_DONE` in the respective INT_RAW registers.
    ///
    /// For UHCI0 (UART DMA) coupled channels with `pending_coupled` set:
    /// - OUT: push bytes from the descriptor chain into the UART TX FIFO.
    ///   `OUT_EOF | OUT_TOTAL_EOF | OUT_DONE` are latched once the chain
    ///   is fully drained; `pending_coupled` is then cleared.
    /// - IN: pop bytes from the UART RX FIFO into the descriptor chain.
    ///   `IN_DONE` is latched per completed descriptor; `IN_SUC_EOF` is
    ///   latched when the chain is fully written or the FIFO idles after
    ///   ≥1 byte moved; `pending_coupled` is cleared on EOF.
    ///
    /// For SPI2/SPI3 coupled channels, `pump_spi` exchanges up to
    /// `COUPLED_BYTES_PER_TICK` wire bytes per tick with the GP-SPI
    /// controller (see its doc comment for the EOF / TRANS_DONE contract).
    ///
    /// Channels with a non-pumped coupled peripheral (I2S, Task 4) retain
    /// `pending_coupled`.
    fn do_tick_with_bus(&mut self, bus: &mut dyn Bus) {
        // SPI pumps run outside the per-channel loop: a transaction's TX and
        // RX directions may live on different channels (paired by PERI_SEL).
        self.pump_spi(bus, DmaPeripheral::Spi2, SPI2_S3_NAME);
        self.pump_spi(bus, DmaPeripheral::Spi3, SPI3_S3_NAME);

        for (ch_idx, c) in self.channels.iter_mut().enumerate() {
            // ── UHCI0 (UART) coupled OUT (TX) ────────────────────────────
            if c.tx.pending_coupled
                && DmaPeripheral::from_sel(c.tx.peri_sel) == DmaPeripheral::Uhci0
            {
                if Self::pump_uart_out(&mut c.tx, bus) {
                    c.tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
                    c.tx.pending_coupled = false;
                    c.tx.coupled_desc_ptr = 0;
                    c.tx.coupled_buf_offset = 0;
                }
                let _ = ch_idx; // suppress unused warning in future expansions
            }

            // ── UHCI0 (UART) coupled IN (RX) ─────────────────────────────
            if c.rx.pending_coupled
                && DmaPeripheral::from_sel(c.rx.peri_sel) == DmaPeripheral::Uhci0
            {
                let (eof, in_done) = Self::pump_uart_in(&mut c.rx, bus);
                if in_done {
                    c.rx.int_raw |= IN_DONE_BIT;
                }
                if eof {
                    c.rx.int_raw |= IN_SUC_EOF_BIT | IN_DONE_BIT;
                    c.rx.pending_coupled = false;
                    c.rx.coupled_desc_ptr = 0;
                    c.rx.coupled_buf_offset = 0;
                }
            }

            // ── MEM_TRANS_EN (M2M) path — one-shot, unchanged ────────────
            if !c.pending_m2m {
                continue;
            }
            c.pending_m2m = false;
            c.in_started = false;
            c.out_started = false;

            let out_desc_addr = Self::full_desc_addr(c.tx.link_addr);
            let in_desc_addr = Self::full_desc_addr(c.rx.link_addr);

            // Collect bytes from the OUT (TX) descriptor chain.
            let bytes = Self::walk_out_chain(bus, out_desc_addr);

            if !bytes.is_empty() {
                // Write bytes into the IN (RX) descriptor chain.
                Self::walk_in_chain(bus, in_desc_addr, &bytes);
            }

            // Latch completion flags regardless of byte count (mirrors how
            // real silicon behaves on a zero-length transfer).
            c.rx.int_raw |= IN_SUC_EOF_BIT | IN_DONE_BIT;
            c.tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
        }
    }
}

impl Peripheral for Esp32s3Gdma {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    /// Level-sensitive IRQ emission: while a channel's INT_ST (RAW & ENA) is
    /// non-zero, re-emit that channel's interrupt-matrix source on every
    /// tick. IN_CHn = base + n; OUT_CHn = base + 5 + n. The source stays
    /// asserted until firmware ACKs via INT_CLR — matching the `systimer`
    /// peripheral's rationale for re-emitting each tick (the bus aggregator
    /// would otherwise race the ISR's own pending-read).
    fn tick(&mut self) -> PeripheralTickResult {
        let mut explicit_irqs = Vec::new();
        for (n, c) in self.channels.iter().enumerate() {
            if c.rx.int_raw & c.rx.int_ena != 0 {
                explicit_irqs.push(self.dma_in_ch0_source + n as u32);
            }
            if c.tx.int_raw & c.tx.int_ena != 0 {
                explicit_irqs.push(self.dma_in_ch0_source + NUM_CHANNELS as u32 + n as u32);
            }
        }

        PeripheralTickResult {
            explicit_irqs: if explicit_irqs.is_empty() {
                None
            } else {
                Some(explicit_irqs)
            },
            ..PeripheralTickResult::default()
        }
    }

    /// True when any channel has a pending MEM_TRANS_EN descriptor walk or a
    /// pending coupled-mode transfer.
    ///
    /// Coupled channels with `pending_coupled` set keep the engine visiting
    /// `tick_with_bus` so the peripheral pumps (UART, SPI2/3; I2S in Task 4)
    /// can make progress. Cleared per-direction when the transfer completes.
    fn needs_bus_tick(&self) -> bool {
        self.channels
            .iter()
            .any(|c| c.pending_m2m || c.rx.pending_coupled || c.tx.pending_coupled)
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        Esp32s3Gdma::do_tick_with_bus(self, bus);
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
    use crate::bus::SystemBus;
    use crate::Bus;

    /// Real ESP32-S3 base source for RX channel 0.
    const IN_CH0_SRC: u32 = 66;

    fn ch_base(n: u64) -> u64 {
        n * CHANNEL_STRIDE
    }

    #[test]
    fn defaults_are_zeroed_with_parked_links() {
        let g = Esp32s3Gdma::new(IN_CH0_SRC);
        for n in 0..NUM_CHANNELS as u64 {
            let b = ch_base(n);
            assert_eq!(g.read_word(b + IN_CONF0), 0);
            assert_eq!(g.read_word(b + OUT_CONF0), 0);
            assert_eq!(g.read_word(b + IN_INT_RAW), 0);
            assert_eq!(g.read_word(b + OUT_INT_RAW), 0);
            // PARK bit reads set (idle) on both links.
            assert_eq!(
                g.read_word(b + IN_LINK) & IN_LINK_PARK_BIT,
                IN_LINK_PARK_BIT
            );
            assert_eq!(
                g.read_word(b + OUT_LINK) & OUT_LINK_PARK_BIT,
                OUT_LINK_PARK_BIT
            );
        }
        assert_eq!(g.read_word(MISC_CONF_OFFSET), 0);
    }

    #[test]
    fn conf_round_trip_all_channels() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        for n in 0..NUM_CHANNELS as u64 {
            let b = ch_base(n);
            g.write_word(b + IN_CONF0, 0x1000_0000 | n as u32);
            g.write_word(b + IN_CONF1, 0x2000_0000 | n as u32);
            g.write_word(b + OUT_CONF0, 0x3000_0000 | n as u32);
            g.write_word(b + OUT_CONF1, 0x4000_0000 | n as u32);
        }
        for n in 0..NUM_CHANNELS as u64 {
            let b = ch_base(n);
            assert_eq!(g.read_word(b + IN_CONF0), 0x1000_0000 | n as u32);
            assert_eq!(g.read_word(b + IN_CONF1), 0x2000_0000 | n as u32);
            assert_eq!(g.read_word(b + OUT_CONF0), 0x3000_0000 | n as u32);
            assert_eq!(g.read_word(b + OUT_CONF1), 0x4000_0000 | n as u32);
        }
    }

    #[test]
    fn link_addr_round_trip_all_channels() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        for n in 0..NUM_CHANNELS as u64 {
            let b = ch_base(n);
            // Write a 20-bit address with no start bit set.
            g.write_word(b + IN_LINK, 0x000A_BCDE);
            g.write_word(b + OUT_LINK, 0x0005_4321);
        }
        for n in 0..NUM_CHANNELS as u64 {
            let b = ch_base(n);
            assert_eq!(g.read_word(b + IN_LINK) & IN_LINK_ADDR_MASK, 0x000A_BCDE);
            assert_eq!(g.read_word(b + OUT_LINK) & OUT_LINK_ADDR_MASK, 0x0005_4321);
        }
    }

    #[test]
    fn misc_conf_round_trip() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        g.write_word(MISC_CONF_OFFSET, 0xDEAD_BEEF);
        assert_eq!(g.read_word(MISC_CONF_OFFSET), 0xDEAD_BEEF);
    }

    /// Without MEM_TRANS_EN, INLINK_START still auto-completes (peripheral-
    /// coupled mode: no byte movement, but EOF is latched immediately).
    #[test]
    fn inlink_start_latches_eof_without_mem_trans_en() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        // MEM_TRANS_EN is NOT set → peripheral-coupled auto-complete.
        g.write_word(b + IN_LINK, IN_LINK_START_BIT | 0x1234);
        let raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(raw & IN_SUC_EOF_BIT, IN_SUC_EOF_BIT, "IN_SUC_EOF latched");
        assert_eq!(raw & IN_DONE_BIT, IN_DONE_BIT, "IN_DONE latched");
        // START is self-clearing: readback never shows bit 22.
        assert_eq!(g.read_word(b + IN_LINK) & IN_LINK_START_BIT, 0);
        // Address was still latched.
        assert_eq!(g.read_word(b + IN_LINK) & IN_LINK_ADDR_MASK, 0x1234);
    }

    /// With MEM_TRANS_EN set, INLINK_START must NOT auto-latch EOF —
    /// the bus-tick path owns that. `pending_m2m` is only set once both
    /// IN_LINK and OUT_LINK have been kicked; INLINK_START alone is not
    /// sufficient (the firmware may start IN before OUT or vice versa).
    #[test]
    fn inlink_start_with_mem_trans_en_defers_eof() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_CONF0, MEM_TRANS_EN_BIT);
        g.write_word(b + IN_LINK, IN_LINK_START_BIT | 0x1000);
        // EOF must NOT be set yet — neither tick_with_bus nor OUT_LINK has run.
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "EOF must not be set after IN_LINK alone"
        );
        // needs_bus_tick is false until OUT_LINK also kicks.
        assert!(!g.needs_bus_tick(), "pending_m2m must wait for OUT_LINK");

        // Now kick OUT_LINK — this arms pending_m2m.
        g.write_word(b + OUT_LINK, OUT_LINK_START_BIT | 0x2000);
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "EOF still not set — tick_with_bus must run"
        );
        assert!(
            g.needs_bus_tick(),
            "pending_m2m must be flagged after both STARTs"
        );
    }

    #[test]
    fn outlink_start_latches_eof_without_mem_trans_en() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(3);
        g.write_word(b + OUT_LINK, OUT_LINK_START_BIT | 0x2222);
        let raw = g.read_word(b + OUT_INT_RAW);
        assert_eq!(raw & OUT_EOF_BIT, OUT_EOF_BIT, "OUT_EOF latched");
        assert_eq!(
            raw & OUT_TOTAL_EOF_BIT,
            OUT_TOTAL_EOF_BIT,
            "OUT_TOTAL_EOF latched"
        );
        assert_eq!(raw & OUT_DONE_BIT, OUT_DONE_BIT, "OUT_DONE latched");
        assert_eq!(g.read_word(b + OUT_LINK) & OUT_LINK_START_BIT, 0);
    }

    #[test]
    fn int_clr_is_w1c() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        // Peripheral-coupled mode (no MEM_TRANS_EN) so EOF auto-latches.
        g.write_word(b + IN_LINK, IN_LINK_START_BIT);
        assert_eq!(g.read_word(b + IN_INT_RAW), IN_SUC_EOF_BIT | IN_DONE_BIT);
        // Clear only IN_DONE (bit 0); IN_SUC_EOF must remain.
        g.write_word(b + IN_INT_CLR, IN_DONE_BIT);
        assert_eq!(g.read_word(b + IN_INT_RAW), IN_SUC_EOF_BIT);
        // Clear the rest.
        g.write_word(b + IN_INT_CLR, IN_SUC_EOF_BIT);
        assert_eq!(g.read_word(b + IN_INT_RAW), 0);
    }

    #[test]
    fn int_st_masks_raw_with_ena() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(1);
        g.write_word(b + IN_LINK, IN_LINK_START_BIT);
        // ENA = 0 → INT_ST is 0 despite RAW set.
        assert_ne!(g.read_word(b + IN_INT_RAW), 0);
        assert_eq!(g.read_word(b + IN_INT_ST), 0);
        // Enable IN_SUC_EOF only.
        g.write_word(b + IN_INT_ENA, IN_SUC_EOF_BIT);
        assert_eq!(g.read_word(b + IN_INT_ST), IN_SUC_EOF_BIT);
    }

    #[test]
    fn tick_emits_in_channel_source_while_st_set() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        // Channel 4 RX: enable + complete.
        let b = ch_base(4);
        g.write_word(b + IN_INT_ENA, IN_SUC_EOF_BIT | IN_DONE_BIT);
        g.write_word(b + IN_LINK, IN_LINK_START_BIT);
        let r = g.tick();
        assert_eq!(
            r.explicit_irqs.as_deref(),
            Some(&[IN_CH0_SRC + 4][..]),
            "IN_CH4 source = base + 4 = 70"
        );
        // Level-sensitive: still emits on the next tick.
        let r = g.tick();
        assert_eq!(r.explicit_irqs.as_deref(), Some(&[IN_CH0_SRC + 4][..]));
        // ACK via INT_CLR de-asserts the level.
        g.write_word(b + IN_INT_CLR, IN_SUC_EOF_BIT | IN_DONE_BIT);
        let r = g.tick();
        assert!(r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()));
    }

    #[test]
    fn tick_emits_out_channel_source() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        // Channel 0 TX: OUT_CH0 source = base + 5 = 71.
        let b = ch_base(0);
        g.write_word(b + OUT_INT_ENA, OUT_EOF_BIT);
        g.write_word(b + OUT_LINK, OUT_LINK_START_BIT);
        let r = g.tick();
        assert_eq!(
            r.explicit_irqs.as_deref(),
            Some(&[IN_CH0_SRC + NUM_CHANNELS as u32][..]),
            "OUT_CH0 source = base + 5 = 71"
        );
    }

    #[test]
    fn no_irq_when_ena_zero_even_if_complete() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        g.write_word(b + IN_LINK, IN_LINK_START_BIT);
        // RAW set but ENA = 0 → no IRQ.
        let r = g.tick();
        assert!(r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()));
    }

    #[test]
    fn channels_are_independent() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        g.write_word(ch_base(0) + IN_CONF0, 0xAAAA_AAAA);
        // Channel 1 IN_CONF0 must be untouched.
        assert_eq!(g.read_word(ch_base(1) + IN_CONF0), 0);
        g.write_word(ch_base(1) + IN_LINK, IN_LINK_START_BIT);
        // Channel 0 INT_RAW must be untouched by channel 1's completion.
        assert_eq!(g.read_word(ch_base(0) + IN_INT_RAW), 0);
        assert_ne!(g.read_word(ch_base(1) + IN_INT_RAW), 0);
    }

    #[test]
    fn byte_granular_access_matches_word() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        // Byte writes assemble into the CONF0 word.
        g.write(b + IN_CONF0, 0x78).unwrap();
        g.write(b + IN_CONF0 + 1, 0x56).unwrap();
        g.write(b + IN_CONF0 + 2, 0x34).unwrap();
        g.write(b + IN_CONF0 + 3, 0x12).unwrap();
        assert_eq!(g.read_u32(b + IN_CONF0).unwrap(), 0x1234_5678);
        assert_eq!(g.read(b + IN_CONF0 + 2).unwrap(), 0x34);
    }

    // ── mem-to-mem (MEM_TRANS_EN) descriptor-walk tests ───────────────────

    /// Helper: write a DMA descriptor (3 words: dw0, buffer, next) into the
    /// bus at `addr`.
    fn write_desc(bus: &mut SystemBus, addr: u64, dw0: u32, buffer: u64, next: u64) {
        bus.write_u32(addr, dw0).unwrap();
        bus.write_u32(addr + 4, buffer as u32).unwrap();
        bus.write_u32(addr + 8, next as u32).unwrap();
    }

    /// Encode a TX descriptor dw0: owner=DMA, suc_eof, length, size.
    fn tx_dw0(len: u32) -> u32 {
        (1 << 31) | (1 << 30) | (len << 12) | len
    }

    /// Encode an RX descriptor dw0: owner=DMA, size (no length yet).
    fn rx_dw0(size: u32) -> u32 {
        (1 << 31) | size
    }

    /// Build a `SystemBus` with 256 KiB of DRAM registered at the
    /// ESP32-S3 DRAM base (`0x3FC8_8000`). The mem-to-mem tests need a
    /// real addressable region so the descriptor-walk reads and buffer
    /// writes can go through the bus router without a MemoryViolation.
    fn bus_with_dram() -> SystemBus {
        use crate::system::xtensa::RamPeripheral;
        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "dram_test",
            0x3FC8_8000,
            256 * 1024,
            None,
            Box::new(RamPeripheral::new(256 * 1024)),
        );
        bus
    }

    /// Build the fixture's exact register sequence and verify bytes moved.
    ///
    /// This test mirrors what `check_dma()` in the Tier-1 fixture does:
    /// place real linked-list descriptors in DRAM, kick OUT then IN with
    /// MEM_TRANS_EN, poll IN_SUC_EOF, verify src == dst byte-by-byte.
    #[test]
    fn m2m_single_descriptor_bytes_move() {
        let mut bus = bus_with_dram();

        // Source buffer at 0x3FC8_8000 (DRAM base in the S3 model).
        let src_addr: u64 = 0x3FC8_8000;
        let dst_addr: u64 = 0x3FC8_9000;
        let src_data: &[u8] = b"TIER1-GDMA-M2M!\0";
        let len = src_data.len() as u32;

        for (i, &b) in src_data.iter().enumerate() {
            bus.write_u8(src_addr + i as u64, b).unwrap();
        }

        // TX descriptor at 0x3FC8_A000, RX descriptor at 0x3FC8_B000.
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;

        write_desc(&mut bus, tx_desc, tx_dw0(len), src_addr, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(len), dst_addr, 0);

        // Build GDMA and perform the fixture's exact register sequence.
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0); // channel 0

        // Enable MEM_TRANS_EN.
        g.write_word(b + IN_CONF0, MEM_TRANS_EN_BIT);

        // Fixture: clear pending interrupts.
        g.write_word(b + IN_INT_CLR, 0xFFFF_FFFF);
        g.write_word(b + OUT_INT_CLR, 0xFFFF_FFFF);

        // Kick: INLINK_START with rx descriptor address (lower 20 bits).
        g.write_word(
            b + IN_LINK,
            ((rx_desc as u32) & IN_LINK_ADDR_MASK) | IN_LINK_START_BIT,
        );
        // Kick: OUTLINK_START with tx descriptor address (lower 20 bits).
        g.write_word(
            b + OUT_LINK,
            ((tx_desc as u32) & OUT_LINK_ADDR_MASK) | OUT_LINK_START_BIT,
        );

        // Before tick_with_bus: IN_SUC_EOF must NOT be set.
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "EOF must not be set before tick_with_bus"
        );

        // Execute the descriptor walk.
        g.tick_with_bus(&mut bus);

        // IN_SUC_EOF must now be set.
        assert_ne!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "IN_SUC_EOF must be set after tick_with_bus"
        );

        // Bytes must have moved.
        for (i, &expected) in src_data.iter().enumerate() {
            let got = bus.read_u8(dst_addr + i as u64).unwrap();
            assert_eq!(got, expected, "dst[{i}] = {got:#04x} want {expected:#04x}");
        }

        // needs_bus_tick must be false after the walk.
        assert!(!g.needs_bus_tick(), "pending_m2m must be cleared");
    }

    /// Owner bit = 0 (CPU-owned): descriptor must be skipped, no bytes moved,
    /// but IN_SUC_EOF is still latched (zero-length completion).
    #[test]
    fn m2m_cpu_owned_descriptor_skipped() {
        let mut bus = bus_with_dram();

        let src_addr: u64 = 0x3FC8_8000;
        let dst_addr: u64 = 0x3FC8_9000;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;

        bus.write_u8(src_addr, 0xAB).unwrap();
        bus.write_u8(dst_addr, 0x00).unwrap();

        // TX descriptor with owner=CPU (bit 31 = 0) — should be skipped.
        let dw0_cpu = (1u32 << 30) | (1 << 12) | 1; // no owner bit
        write_desc(&mut bus, tx_desc, dw0_cpu, src_addr, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(1), dst_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_CONF0, MEM_TRANS_EN_BIT);
        g.write_word(
            b + IN_LINK,
            ((rx_desc as u32) & IN_LINK_ADDR_MASK) | IN_LINK_START_BIT,
        );
        g.write_word(
            b + OUT_LINK,
            ((tx_desc as u32) & OUT_LINK_ADDR_MASK) | OUT_LINK_START_BIT,
        );
        g.tick_with_bus(&mut bus);

        // No bytes moved (TX descriptor was CPU-owned, walk produced 0 bytes).
        assert_eq!(
            bus.read_u8(dst_addr).unwrap(),
            0x00,
            "dst must be untouched when TX descriptor is CPU-owned"
        );
        // Completion flags are still latched.
        assert_ne!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "IN_SUC_EOF latched even for zero-byte transfer"
        );
    }

    /// Two-descriptor OUT chain → single RX descriptor: verifies multi-link
    /// chain walking.
    #[test]
    fn m2m_two_tx_descriptors_chained() {
        let mut bus = bus_with_dram();

        // Two source buffers of 4 bytes each.
        let src1: u64 = 0x3FC8_8000;
        let src2: u64 = 0x3FC8_8010;
        let dst: u64 = 0x3FC8_9000;
        let tx_desc1: u64 = 0x3FC8_A000;
        let tx_desc2: u64 = 0x3FC8_A010;
        let rx_desc: u64 = 0x3FC8_B000;

        let data1 = [0x11u8, 0x22, 0x33, 0x44];
        let data2 = [0x55u8, 0x66, 0x77, 0x88];
        for (i, &b) in data1.iter().enumerate() {
            bus.write_u8(src1 + i as u64, b).unwrap();
        }
        for (i, &b) in data2.iter().enumerate() {
            bus.write_u8(src2 + i as u64, b).unwrap();
        }

        // TX chain: desc1 → desc2 → EOL.
        // desc1: owner=DMA, NOT suc_eof (not last), length=4, size=4.
        let dw0_1 = (1u32 << 31) | (4 << 12) | 4; // no suc_eof
        write_desc(&mut bus, tx_desc1, dw0_1, src1, tx_desc2);
        // desc2: owner=DMA, suc_eof, length=4, size=4, next=0.
        write_desc(&mut bus, tx_desc2, tx_dw0(4), src2, 0);

        // RX: single descriptor big enough for both chunks (8 bytes).
        write_desc(&mut bus, rx_desc, rx_dw0(8), dst, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(1);
        g.write_word(b + IN_CONF0, MEM_TRANS_EN_BIT);
        g.write_word(
            b + IN_LINK,
            ((rx_desc as u32) & IN_LINK_ADDR_MASK) | IN_LINK_START_BIT,
        );
        g.write_word(
            b + OUT_LINK,
            ((tx_desc1 as u32) & OUT_LINK_ADDR_MASK) | OUT_LINK_START_BIT,
        );
        g.tick_with_bus(&mut bus);

        // All 8 bytes must have arrived at dst.
        let expected = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        for (i, &exp) in expected.iter().enumerate() {
            let got = bus.read_u8(dst + i as u64).unwrap();
            assert_eq!(got, exp, "dst[{i}] = {got:#04x} want {exp:#04x}");
        }
        assert_ne!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "IN_SUC_EOF after two-descriptor chain"
        );
    }

    // ── PERI_SEL register tests ───────────────────────────────────────────

    /// Round-trip: write a valid sel value to IN_PERI_SEL on ch2, read back.
    #[test]
    fn in_peri_sel_round_trip() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        // sel = 3 (I2S0).
        g.write_word(b + IN_PERI_SEL, 0x03);
        assert_eq!(g.read_word(b + IN_PERI_SEL), 0x03);
    }

    /// Mask: writing 0xFF to IN_PERI_SEL reads back only the low 6 bits (0x3F).
    #[test]
    fn in_peri_sel_mask_enforced() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        g.write_word(b + IN_PERI_SEL, 0xFF);
        assert_eq!(g.read_word(b + IN_PERI_SEL), 0x3F, "only 6 bits are stored");
    }

    /// Reset value of IN_PERI_SEL is 0x3F (unbound).
    #[test]
    fn in_peri_sel_reset_value() {
        let g = Esp32s3Gdma::new(IN_CH0_SRC);
        for n in 0..NUM_CHANNELS as u64 {
            assert_eq!(
                g.read_word(ch_base(n) + IN_PERI_SEL),
                PERI_SEL_RESET,
                "ch{n} IN_PERI_SEL reset must be 0x3F"
            );
        }
    }

    /// Round-trip: write a valid sel value to OUT_PERI_SEL on ch2, read back.
    #[test]
    fn out_peri_sel_round_trip() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        // sel = 3 (I2S0).
        g.write_word(b + OUT_PERI_SEL, 0x03);
        assert_eq!(g.read_word(b + OUT_PERI_SEL), 0x03);
    }

    /// Mask: writing 0xFF to OUT_PERI_SEL reads back only the low 6 bits.
    #[test]
    fn out_peri_sel_mask_enforced() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        g.write_word(b + OUT_PERI_SEL, 0xFF);
        assert_eq!(g.read_word(b + OUT_PERI_SEL), 0x3F);
    }

    /// Reset value of OUT_PERI_SEL is 0x3F (unbound).
    #[test]
    fn out_peri_sel_reset_value() {
        let g = Esp32s3Gdma::new(IN_CH0_SRC);
        for n in 0..NUM_CHANNELS as u64 {
            assert_eq!(
                g.read_word(ch_base(n) + OUT_PERI_SEL),
                PERI_SEL_RESET,
                "ch{n} OUT_PERI_SEL reset must be 0x3F"
            );
        }
    }

    /// PERI_SEL registers on different channels are independent.
    #[test]
    fn peri_sel_channels_independent() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        g.write_word(ch_base(0) + IN_PERI_SEL, 0x02); // UHCI0
        g.write_word(ch_base(1) + IN_PERI_SEL, 0x07); // SHA
        assert_eq!(g.read_word(ch_base(0) + IN_PERI_SEL), 0x02);
        assert_eq!(g.read_word(ch_base(1) + IN_PERI_SEL), 0x07);
        // Unwritten channels stay at reset.
        assert_eq!(g.read_word(ch_base(2) + IN_PERI_SEL), PERI_SEL_RESET);
    }

    // ── Coupled-set start tests ───────────────────────────────────────────

    /// Coupled IN: set OUT_PERI_SEL = UHCI0 (2), MEM_TRANS_EN clear,
    /// write OUTLINK_START → OUT_INT_RAW must have NO OUT_EOF bits.
    #[test]
    fn coupled_out_start_does_not_latch_eof() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        // MEM_TRANS_EN must be clear (default).
        g.write_word(b + OUT_PERI_SEL, 2); // UHCI0 = coupled
        g.write_word(b + OUT_LINK, OUT_LINK_START_BIT | 0x1000);
        let raw = g.read_word(b + OUT_INT_RAW);
        assert_eq!(
            raw & (OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT),
            0,
            "coupled OUT start must NOT latch EOF"
        );
        // needs_bus_tick returns true because pending_coupled is set.
        assert!(
            g.needs_bus_tick(),
            "needs_bus_tick must be true for coupled OUT"
        );
    }

    /// Coupled IN: set IN_PERI_SEL = UHCI0 (2), MEM_TRANS_EN clear,
    /// write INLINK_START → IN_INT_RAW must have NO IN_SUC_EOF/IN_DONE bits.
    #[test]
    fn coupled_in_start_does_not_latch_eof() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(1);
        g.write_word(b + IN_PERI_SEL, 2); // UHCI0 = coupled
        g.write_word(b + IN_LINK, IN_LINK_START_BIT | 0x2000);
        let raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(
            raw & (IN_SUC_EOF_BIT | IN_DONE_BIT),
            0,
            "coupled IN start must NOT latch EOF"
        );
        assert!(
            g.needs_bus_tick(),
            "needs_bus_tick must be true for coupled IN"
        );
    }

    /// All five coupled peripheral values for OUT direction do NOT latch EOF.
    #[test]
    fn coupled_out_all_five_peripherals_no_eof() {
        for sel in [0u32, 1, 2, 3, 4] {
            // sel 0=SPI2, 1=SPI3, 2=UHCI0, 3=I2S0, 4=I2S1
            let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
            let b = ch_base(0);
            g.write_word(b + OUT_PERI_SEL, sel);
            g.write_word(b + OUT_LINK, OUT_LINK_START_BIT | 0x100);
            let raw = g.read_word(b + OUT_INT_RAW);
            assert_eq!(
                raw & (OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT),
                0,
                "sel={sel} should not latch OUT_EOF"
            );
        }
    }

    /// All five coupled peripheral values for IN direction do NOT latch EOF.
    #[test]
    fn coupled_in_all_five_peripherals_no_eof() {
        for sel in [0u32, 1, 2, 3, 4] {
            let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
            let b = ch_base(0);
            g.write_word(b + IN_PERI_SEL, sel);
            g.write_word(b + IN_LINK, IN_LINK_START_BIT | 0x100);
            let raw = g.read_word(b + IN_INT_RAW);
            assert_eq!(
                raw & (IN_SUC_EOF_BIT | IN_DONE_BIT),
                0,
                "sel={sel} should not latch IN_SUC_EOF"
            );
        }
    }

    // ── Fallback-set auto-complete tests ─────────────────────────────────

    /// Fallback OUT: SHA (sel=7) auto-completes immediately on OUTLINK_START.
    #[test]
    fn fallback_out_sha_auto_completes() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 7); // SHA = fallback
        g.write_word(b + OUT_LINK, OUT_LINK_START_BIT | 0x3000);
        let raw = g.read_word(b + OUT_INT_RAW);
        assert_eq!(raw & OUT_EOF_BIT, OUT_EOF_BIT, "OUT_EOF must be latched");
        assert_eq!(
            raw & OUT_TOTAL_EOF_BIT,
            OUT_TOTAL_EOF_BIT,
            "OUT_TOTAL_EOF must be latched"
        );
        assert_eq!(raw & OUT_DONE_BIT, OUT_DONE_BIT, "OUT_DONE must be latched");
        assert!(!g.needs_bus_tick(), "no bus tick needed for fallback OUT");
    }

    /// Fallback IN: SHA (sel=7) auto-completes immediately on INLINK_START.
    #[test]
    fn fallback_in_sha_auto_completes() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_PERI_SEL, 7); // SHA = fallback
        g.write_word(b + IN_LINK, IN_LINK_START_BIT | 0x4000);
        let raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(
            raw & IN_SUC_EOF_BIT,
            IN_SUC_EOF_BIT,
            "IN_SUC_EOF must be latched"
        );
        assert_eq!(raw & IN_DONE_BIT, IN_DONE_BIT, "IN_DONE must be latched");
    }

    /// Unbound reset value (0x3F) behaves as fallback for OUT direction.
    /// Firmware that never writes PERI_SEL must get auto-complete (legacy
    /// compatibility promise).
    #[test]
    fn unbound_out_reset_value_is_fallback() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(3);
        // Do NOT write PERI_SEL; it stays at 0x3F (Unknown).
        g.write_word(b + OUT_LINK, OUT_LINK_START_BIT | 0x5000);
        let raw = g.read_word(b + OUT_INT_RAW);
        assert_ne!(
            raw & (OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT),
            0,
            "unbound PERI_SEL (reset 0x3F) must auto-complete OUT"
        );
    }

    /// Unbound reset value (0x3F) behaves as fallback for IN direction.
    #[test]
    fn unbound_in_reset_value_is_fallback() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(3);
        // Do NOT write PERI_SEL; it stays at 0x3F (Unknown).
        g.write_word(b + IN_LINK, IN_LINK_START_BIT | 0x6000);
        let raw = g.read_word(b + IN_INT_RAW);
        assert_ne!(
            raw & (IN_SUC_EOF_BIT | IN_DONE_BIT),
            0,
            "unbound PERI_SEL (reset 0x3F) must auto-complete IN"
        );
    }

    // ── UART (UHCI0) coupled transfer tests ──────────────────────────────
    //
    // These tests require a composite bus with:
    //   - DRAM region at 0x3FC8_8000 (for descriptors and data buffers).
    //   - GDMA peripheral at 0x6003_F000 (real ESP32-S3 base).
    //   - Esp32s3Uart at 0x6000_0000 (UART0, real ESP32-S3 base).
    //
    // The GDMA struct is also held directly for register-level manipulation.
    // The bus is used only for the byte-pump paths (descriptor reads,
    // UART FIFO read/write, STATUS read).

    use crate::peripherals::esp32s3::uart::Esp32s3Uart;
    use std::sync::{Arc, Mutex};

    /// Build a composite `SystemBus` with DRAM, GDMA, and UART0 registered at
    /// their real ESP32-S3 addresses.  Returns `(bus, sink)` where `sink` is
    /// the shared TX capture buffer attached to UART0.
    ///
    /// GDMA is added to the bus so tick_with_bus can reach the UART FIFO
    /// via normal bus reads/writes.  The caller also holds a separate
    /// `Esp32s3Gdma` instance for register-level manipulation; descriptor
    /// walks use `bus` directly.
    fn uart_test_bus() -> (SystemBus, Arc<Mutex<Vec<u8>>>) {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut bus = SystemBus::new();
        // 256 KiB DRAM at 0x3FC8_8000 — descriptors and data buffers go here.
        bus.add_peripheral(
            "dram_test",
            0x3FC8_8000,
            256 * 1024,
            None,
            Box::new(crate::system::xtensa::RamPeripheral::new(256 * 1024)),
        );
        // UART0 at the real DR_REG_UART0_BASE.
        let mut uart = Esp32s3Uart::new(false, 27);
        uart.set_sink(Some(sink.clone()));
        bus.add_peripheral("uart0_test", UART0_BASE, 0x100, None, Box::new(uart));
        (bus, sink)
    }

    /// Write a 3-word DMA descriptor into the bus at `addr`.
    fn write_desc_uart(bus: &mut SystemBus, addr: u64, dw0: u32, buffer: u64, next: u64) {
        bus.write_u32(addr, dw0).unwrap();
        bus.write_u32(addr + 4, buffer as u32).unwrap();
        bus.write_u32(addr + 8, next as u32).unwrap();
    }

    // ── TX (OUT) tests ────────────────────────────────────────────────────

    /// TX basic: write "HELLO" into a single descriptor, set PERI_SEL=UHCI0,
    /// start OUT link → after ticks, UART TX sink contains "HELLO"; EOF latched;
    /// pending_coupled cleared (needs_bus_tick returns false).
    #[test]
    fn uart_tx_hello_via_descriptors() {
        let (mut bus, sink) = uart_test_bus();
        let payload = b"HELLO";
        let buf_addr: u64 = 0x3FC8_8000;
        let desc_addr: u64 = 0x3FC8_A000;

        // Write payload into DRAM.
        for (i, &b) in payload.iter().enumerate() {
            bus.write_u8(buf_addr + i as u64, b).unwrap();
        }
        // Single TX descriptor: owner=DMA, suc_eof, length=5, size=5.
        let dw0 = (1u32 << 31) | (1 << 30) | (5 << 12) | 5;
        write_desc_uart(&mut bus, desc_addr, dw0, buf_addr, 0);

        // Set up GDMA channel 0 for UHCI0 OUT.
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 2); // UHCI0
                                           // desc_addr bits[19:0] = 0xA000.
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (desc_addr as u32 & OUT_LINK_ADDR_MASK),
        );

        // pending_coupled must be set; EOF not latched yet.
        assert!(g.needs_bus_tick(), "needs_bus_tick after OUT start");
        assert_eq!(
            g.read_word(b + OUT_INT_RAW) & (OUT_EOF_BIT | OUT_DONE_BIT),
            0
        );

        // Drain via tick_with_bus (5 bytes, well within COUPLED_BYTES_PER_TICK).
        g.do_tick_with_bus(&mut bus);

        // EOF must be latched.
        let raw = g.read_word(b + OUT_INT_RAW);
        assert_eq!(raw & OUT_EOF_BIT, OUT_EOF_BIT, "OUT_EOF must be set");
        assert_eq!(raw & OUT_TOTAL_EOF_BIT, OUT_TOTAL_EOF_BIT, "OUT_TOTAL_EOF");
        assert_eq!(raw & OUT_DONE_BIT, OUT_DONE_BIT, "OUT_DONE");
        // pending_coupled cleared → needs_bus_tick false.
        assert!(
            !g.needs_bus_tick(),
            "needs_bus_tick must be false after completion"
        );

        // UART TX FIFO should have received the bytes; drain them via tick.
        // reset CLKDIV=694 → ~20820 ticks/byte; 5 bytes × 25000 is generous.
        let uart_idx = bus.find_peripheral_index_by_name("uart0_test").unwrap();
        let uart = bus.peripherals[uart_idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32s3Uart>()
            .unwrap();
        for _ in 0..200_000u64 {
            uart.tick();
        }

        let got = sink.lock().unwrap().clone();
        assert_eq!(got, b"HELLO", "UART sink must contain HELLO, got {:?}", got);
    }

    /// TX larger than COUPLED_BYTES_PER_TICK (200 bytes): completes over
    /// multiple ticks, order preserved.
    #[test]
    fn uart_tx_large_transfer_multi_tick() {
        let (mut bus, sink) = uart_test_bus();
        let count: usize = 200;
        let buf_addr: u64 = 0x3FC8_8000;
        let desc_addr: u64 = 0x3FC8_A000;

        // Write 200 bytes of sequential data.
        let payload: Vec<u8> = (0u8..200).collect();
        for (i, &b) in payload.iter().enumerate() {
            bus.write_u8(buf_addr + i as u64, b).unwrap();
        }
        let dw0 = (1u32 << 31) | (1 << 30) | ((count as u32) << 12) | count as u32;
        write_desc_uart(&mut bus, desc_addr, dw0, buf_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 2);
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (desc_addr as u32 & OUT_LINK_ADDR_MASK),
        );

        // Drive ticks until completion.  Interleave UART drain between GDMA
        // ticks so the TX FIFO never stays full (200 bytes > FIFO depth 128).
        // 20820 ticks/byte × 128 bytes ≈ 2.7 M uart ticks clears a full FIFO.
        let uart_idx = bus.find_peripheral_index_by_name("uart0_test").unwrap();
        let mut ticks = 0usize;
        while g.needs_bus_tick() {
            g.do_tick_with_bus(&mut bus);
            ticks += 1;
            // Drain UART between GDMA ticks to free FIFO space.
            let uart = bus.peripherals[uart_idx]
                .dev
                .as_any_mut()
                .unwrap()
                .downcast_mut::<Esp32s3Uart>()
                .unwrap();
            for _ in 0..3_000_000u64 {
                uart.tick();
            }
            assert!(ticks < 500, "transfer did not complete in reasonable ticks");
        }

        // At least 2 GDMA ticks (first fills FIFO with ≤64 bytes from budget,
        // but FIFO may already be partially drained by interleaved uart ticks;
        // so the exact count depends on timing — just verify > 1).
        assert!(
            ticks >= 1,
            "expected ≥1 gdma tick for 200 bytes, got {ticks}"
        );

        // Final UART drain.
        let uart = bus.peripherals[uart_idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32s3Uart>()
            .unwrap();
        for _ in 0..5_000_000u64 {
            uart.tick();
        }

        let got = sink.lock().unwrap().clone();
        assert_eq!(
            got.len(),
            count,
            "expected {count} bytes in sink, got {}",
            got.len()
        );
        assert_eq!(got, payload, "byte order must be preserved");

        // EOF flags set.
        let raw = g.read_word(b + OUT_INT_RAW);
        assert_eq!(raw & OUT_EOF_BIT, OUT_EOF_BIT, "OUT_EOF after large TX");
        assert!(!g.needs_bus_tick(), "needs_bus_tick cleared");
    }

    // ── RX (IN) tests ─────────────────────────────────────────────────────

    /// Helper: push bytes into UART0 RX FIFO via the bus's peripheral index.
    fn push_uart_rx(bus: &mut SystemBus, bytes: &[u8]) {
        let idx = bus.find_peripheral_index_by_name("uart0_test").unwrap();
        let uart = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32s3Uart>()
            .unwrap();
        for &b in bytes {
            uart.push_rx(b);
        }
    }

    /// RX basic: inject bytes into the UART RX FIFO, provide an IN descriptor
    /// with enough capacity → bytes land in memory; IN_SUC_EOF + IN_DONE set;
    /// pending_coupled cleared.
    #[test]
    fn uart_rx_bytes_land_in_descriptor() {
        let (mut bus, _sink) = uart_test_bus();
        let rx_data = b"WORLD";
        push_uart_rx(&mut bus, rx_data);

        let dst_addr: u64 = 0x3FC8_9000;
        let desc_addr: u64 = 0x3FC8_B000;
        // RX descriptor: owner=DMA, size=32 (plenty of space).
        let dw0 = (1u32 << 31) | 32u32;
        write_desc_uart(&mut bus, desc_addr, dw0, dst_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_PERI_SEL, 2); // UHCI0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (desc_addr as u32 & IN_LINK_ADDR_MASK),
        );

        assert!(g.needs_bus_tick());
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & (IN_SUC_EOF_BIT | IN_DONE_BIT),
            0
        );

        // One tick should drain 5 bytes (within budget).
        g.do_tick_with_bus(&mut bus);

        // Check INT_RAW.
        let raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(
            raw & IN_SUC_EOF_BIT,
            IN_SUC_EOF_BIT,
            "IN_SUC_EOF must be set"
        );
        assert_eq!(raw & IN_DONE_BIT, IN_DONE_BIT, "IN_DONE must be set");
        assert!(!g.needs_bus_tick(), "pending_coupled cleared");

        // Bytes must have landed in memory.
        for (i, &expected) in rx_data.iter().enumerate() {
            let got = bus.read_u8(dst_addr + i as u64).unwrap();
            assert_eq!(got, expected, "dst[{i}] mismatch");
        }
    }

    /// RX partial: descriptor smaller than FIFO content → first descriptor
    /// filled + IN_DONE, chain continues on next tick.
    #[test]
    fn uart_rx_partial_fill_continues_next_tick() {
        let (mut bus, _sink) = uart_test_bus();
        // Push 8 bytes; first descriptor only holds 4.
        push_uart_rx(&mut bus, b"ABCDEFGH");

        let dst1: u64 = 0x3FC8_9000;
        let dst2: u64 = 0x3FC8_9010;
        let desc1: u64 = 0x3FC8_B000;
        let desc2: u64 = 0x3FC8_B010;

        // Two descriptors of 4 bytes each, chained.
        let dw0_1 = (1u32 << 31) | 4u32; // cap=4
        let dw0_2 = (1u32 << 31) | 4u32;
        write_desc_uart(&mut bus, desc1, dw0_1, dst1, desc2);
        write_desc_uart(&mut bus, desc2, dw0_2, dst2, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(1);
        g.write_word(b + IN_PERI_SEL, 2); // UHCI0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (desc1 as u32 & IN_LINK_ADDR_MASK),
        );

        // First tick: drain 4 bytes → desc1 filled; IN_DONE set; transfer still
        // pending (8 bytes total but desc1 full, chain advances to desc2).
        g.do_tick_with_bus(&mut bus);

        // IN_DONE should be set (first descriptor completed).
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & IN_DONE_BIT,
            IN_DONE_BIT,
            "IN_DONE after first desc filled"
        );

        // Second tick drains remaining 4 bytes.
        // The first tick may have drained all 8 if within budget — that's fine
        // too. Just run until done.
        let mut ticks = 0;
        while g.needs_bus_tick() {
            g.do_tick_with_bus(&mut bus);
            ticks += 1;
            assert!(ticks < 100, "did not complete in reasonable ticks");
        }

        // All 8 bytes must be in memory.
        let expected = b"ABCDEFGH";
        for (i, &exp) in expected.iter().enumerate() {
            let got = if i < 4 {
                bus.read_u8(dst1 + i as u64).unwrap()
            } else {
                bus.read_u8(dst2 + (i - 4) as u64).unwrap()
            };
            assert_eq!(got, exp, "byte[{i}] mismatch");
        }

        // IN_SUC_EOF must be set at the end.
        assert_eq!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            IN_SUC_EOF_BIT,
            "IN_SUC_EOF must be latched after full chain"
        );
    }

    // ── Interrupt (explicit_irqs) tests ───────────────────────────────────

    /// TX completion with INT_ENA set → explicit_irqs carries OUT_CH0 source.
    #[test]
    fn uart_tx_irq_fires_on_eof() {
        let (mut bus, _sink) = uart_test_bus();
        let buf_addr: u64 = 0x3FC8_8000;
        let desc_addr: u64 = 0x3FC8_A000;
        bus.write_u8(buf_addr, b'X').unwrap();
        let dw0 = (1u32 << 31) | (1 << 30) | (1 << 12) | 1;
        write_desc_uart(&mut bus, desc_addr, dw0, buf_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        // Enable OUT_EOF interrupt (bit 1).
        g.write_word(
            b + OUT_INT_ENA,
            OUT_EOF_BIT | OUT_DONE_BIT | OUT_TOTAL_EOF_BIT,
        );
        g.write_word(b + OUT_PERI_SEL, 2);
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (desc_addr as u32 & OUT_LINK_ADDR_MASK),
        );

        g.do_tick_with_bus(&mut bus);

        // Now tick() should emit the OUT_CH0 source (base + 5 + 0 = 71).
        let result = g.tick();
        let irqs = result.explicit_irqs.unwrap_or_default();
        let expected_src = IN_CH0_SRC + NUM_CHANNELS as u32; // 66 + 5 = 71
        assert!(
            irqs.contains(&expected_src),
            "expected OUT_CH0 source {expected_src} in {irqs:?}"
        );
    }

    /// RX completion with INT_ENA set → explicit_irqs carries IN_CH0 source.
    #[test]
    fn uart_rx_irq_fires_on_suc_eof() {
        let (mut bus, _sink) = uart_test_bus();
        push_uart_rx(&mut bus, b"Z");

        let dst: u64 = 0x3FC8_9000;
        let desc: u64 = 0x3FC8_B000;
        let dw0 = (1u32 << 31) | 16u32;
        write_desc_uart(&mut bus, desc, dw0, dst, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_INT_ENA, IN_SUC_EOF_BIT | IN_DONE_BIT);
        g.write_word(b + IN_PERI_SEL, 2); // UHCI0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (desc as u32 & IN_LINK_ADDR_MASK),
        );

        g.do_tick_with_bus(&mut bus);

        let result = g.tick();
        let irqs = result.explicit_irqs.unwrap_or_default();
        let expected_src = IN_CH0_SRC; // channel 0 IN = 66
        assert!(
            irqs.contains(&expected_src),
            "expected IN_CH0 source {expected_src} in {irqs:?}"
        );
    }

    // ── Owner-bit writeback tests ─────────────────────────────────────────

    /// M2M: after tick_with_bus, TX descriptor dw0 must have owner bit cleared.
    #[test]
    fn m2m_tx_owner_bit_cleared_after_walk() {
        let mut bus = bus_with_dram();
        let src_addr: u64 = 0x3FC8_8000;
        let dst_addr: u64 = 0x3FC8_9000;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;
        bus.write_u8(src_addr, 0xAB).unwrap();
        write_desc(&mut bus, tx_desc, tx_dw0(1), src_addr, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(1), dst_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_CONF0, MEM_TRANS_EN_BIT);
        g.write_word(
            b + IN_LINK,
            ((rx_desc as u32) & IN_LINK_ADDR_MASK) | IN_LINK_START_BIT,
        );
        g.write_word(
            b + OUT_LINK,
            ((tx_desc as u32) & OUT_LINK_ADDR_MASK) | OUT_LINK_START_BIT,
        );
        g.tick_with_bus(&mut bus);

        let dw0_after = bus.read_u32(tx_desc).unwrap();
        assert_eq!(
            dw0_after & DESC_OWNER_BIT,
            0,
            "TX descriptor owner bit must be cleared after M2M walk"
        );
    }

    /// M2M: after tick_with_bus, RX descriptor dw0 must have owner bit cleared
    /// and bits [23:12] must contain the received byte count.
    #[test]
    fn m2m_rx_owner_bit_cleared_and_length_set() {
        let mut bus = bus_with_dram();
        let src_addr: u64 = 0x3FC8_8000;
        let dst_addr: u64 = 0x3FC8_9000;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;
        let len = 7u32;
        for i in 0..len {
            bus.write_u8(src_addr + i as u64, i as u8).unwrap();
        }
        write_desc(&mut bus, tx_desc, tx_dw0(len), src_addr, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(len), dst_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_CONF0, MEM_TRANS_EN_BIT);
        g.write_word(
            b + IN_LINK,
            ((rx_desc as u32) & IN_LINK_ADDR_MASK) | IN_LINK_START_BIT,
        );
        g.write_word(
            b + OUT_LINK,
            ((tx_desc as u32) & OUT_LINK_ADDR_MASK) | OUT_LINK_START_BIT,
        );
        g.tick_with_bus(&mut bus);

        let dw0_after = bus.read_u32(rx_desc).unwrap();
        assert_eq!(
            dw0_after & DESC_OWNER_BIT,
            0,
            "RX descriptor owner bit must be cleared"
        );
        let written_len = (dw0_after >> 12) & 0xFFF;
        assert_eq!(
            written_len, len,
            "RX descriptor length field must equal bytes written"
        );
    }

    /// UART TX: after coupled OUT completes, TX descriptor dw0 owner bit = 0.
    #[test]
    fn uart_tx_owner_bit_cleared_on_completion() {
        let (mut bus, _sink) = uart_test_bus();
        let buf_addr: u64 = 0x3FC8_8000;
        let desc_addr: u64 = 0x3FC8_A000;
        bus.write_u8(buf_addr, b'X').unwrap();
        let dw0 = (1u32 << 31) | (1 << 30) | (1 << 12) | 1;
        write_desc_uart(&mut bus, desc_addr, dw0, buf_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 2); // UHCI0
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (desc_addr as u32 & OUT_LINK_ADDR_MASK),
        );
        g.do_tick_with_bus(&mut bus);

        let dw0_after = bus.read_u32(desc_addr).unwrap();
        assert_eq!(
            dw0_after & DESC_OWNER_BIT,
            0,
            "UART TX descriptor owner bit must be cleared after completion"
        );
    }

    /// UART RX: after coupled IN completes, RX descriptor dw0 owner bit = 0
    /// and bits [23:12] contain the received byte count.
    #[test]
    fn uart_rx_owner_bit_cleared_and_received_length_set() {
        let (mut bus, _sink) = uart_test_bus();
        let rx_data = b"ABCDE";
        push_uart_rx(&mut bus, rx_data);

        let dst_addr: u64 = 0x3FC8_9000;
        let desc_addr: u64 = 0x3FC8_B000;
        let cap = 16u32;
        let dw0 = (1u32 << 31) | cap;
        write_desc_uart(&mut bus, desc_addr, dw0, dst_addr, 0);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + IN_PERI_SEL, 2); // UHCI0
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (desc_addr as u32 & IN_LINK_ADDR_MASK),
        );
        g.do_tick_with_bus(&mut bus);

        let dw0_after = bus.read_u32(desc_addr).unwrap();
        assert_eq!(
            dw0_after & DESC_OWNER_BIT,
            0,
            "UART RX descriptor owner bit must be cleared"
        );
        let written_len = (dw0_after >> 12) & 0xFFF;
        assert_eq!(
            written_len as usize,
            rx_data.len(),
            "RX descriptor length field must equal bytes received"
        );
    }

    // ── SPI2/3 DMA coupling tests ─────────────────────────────────────────

    // SPI register offsets and bits — mirrors gpspi.rs / ESP-IDF
    // `soc/esp32s3/register/soc/spi_reg.h`.
    const SPI_CMD_REG: u64 = 0x00;
    const SPI_MS_DLEN_REG: u64 = 0x1C;
    const SPI_DMA_CONF_REG: u64 = 0x30;
    const SPI_DMA_INT_RAW_REG: u64 = 0x3C;
    const SPI_USR_BIT: u32 = 1 << 24;
    const SPI_TRANS_DONE_BIT: u32 = 1 << 12;
    /// `SPI_DMA_TX_ENA : R/W ;bitpos:[28]` (spi_reg.h).
    const SPI_DMA_TX_ENA_BIT: u32 = 1 << 28;
    /// `SPI_DMA_RX_ENA : R/W ;bitpos:[27]` (spi_reg.h).
    const SPI_DMA_RX_ENA_BIT: u32 = 1 << 27;

    const SPI3_BASE: u64 = 0x6002_5000;
    const SPI2_BASE: u64 = 0x6002_4000;

    /// Test device following the `Recorder` pattern from `esp32/spi.rs`
    /// tests, with a non-trivial MISO (`mosi ^ 0xA5`) so full-duplex
    /// byte pairing is actually verified.
    struct XorDevice {
        seen: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    }
    impl crate::peripherals::spi::SpiDevice for XorDevice {
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.seen.lock().unwrap().push(mosi);
            mosi ^ 0xA5
        }
        fn cs_pin(&self) -> &str {
            "GPIO10"
        }
    }

    /// Build a composite `SystemBus` with DRAM at 0x3FC8_8000 and SPI3 at
    /// its real base. Optionally attaches an `XorDevice`; returns the
    /// shared MOSI log when one is attached.
    fn spi3_test_bus(
        with_device: bool,
    ) -> (SystemBus, Option<std::sync::Arc<std::sync::Mutex<Vec<u8>>>>) {
        let mut bus = bus_with_dram();
        let mut spi = Esp32s3Spi::new(22);
        let log = if with_device {
            let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            spi.attach(Box::new(XorDevice { seen: seen.clone() }));
            Some(seen)
        } else {
            None
        };
        bus.add_peripheral("spi3_s3", SPI3_BASE, 0x100, None, Box::new(spi));
        (bus, log)
    }

    fn spi_write_u32(bus: &mut SystemBus, base: u64, reg_off: u64, val: u32) {
        bus.write_u32(base + reg_off, val).unwrap();
    }

    fn spi_read_u32(bus: &mut SystemBus, base: u64, reg_off: u64) -> u32 {
        bus.read_u32(base + reg_off).unwrap()
    }

    /// Kick a DMA-mode SPI transaction of `n` bytes (TX+RX enabled).
    fn kick_spi_dma(bus: &mut SystemBus, base: u64, n: usize) {
        spi_write_u32(
            bus,
            base,
            SPI_DMA_CONF_REG,
            SPI_DMA_TX_ENA_BIT | SPI_DMA_RX_ENA_BIT,
        );
        spi_write_u32(bus, base, SPI_MS_DLEN_REG, (n as u32) * 8 - 1);
        spi_write_u32(bus, base, SPI_CMD_REG, SPI_USR_BIT);
    }

    /// Drive `tick_with_bus` until the GDMA goes idle (bounded).
    fn tick_until_idle(g: &mut Esp32s3Gdma, bus: &mut SystemBus, max_ticks: usize) -> usize {
        for t in 0..max_ticks {
            if !g.needs_bus_tick() {
                return t;
            }
            g.tick_with_bus(bus);
        }
        max_ticks
    }

    /// SPI3 DMA with an attached device: descriptor-fed MOSI bytes reach
    /// the device byte-for-byte; device-fed MISO bytes land in the IN
    /// descriptor buffer; TRANS_DONE + OUT_EOF + IN_SUC_EOF all latch.
    #[test]
    fn spi3_dma_device_mosi_miso_roundtrip() {
        let (mut bus, log) = spi3_test_bus(true);
        let n: usize = 8;
        let payload: Vec<u8> = vec![0x01, 0x80, 0xFF, 0x00, 0x5A, 0xA5, 0x10, 0x7E];
        let tx_buf: u64 = 0x3FC8_8000;
        let rx_buf: u64 = 0x3FC8_9000;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;

        for (i, &b) in payload.iter().enumerate() {
            bus.write_u8(tx_buf + i as u64, b).unwrap();
        }
        write_desc(&mut bus, tx_desc, tx_dw0(n as u32), tx_buf, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(n as u32), rx_buf, 0);

        kick_spi_dma(&mut bus, SPI3_BASE, n);
        // USR held + TRANS_DONE deferred until GDMA services the transaction.
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "USR must stay set while DMA pending"
        );
        assert_eq!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0,
            "TRANS_DONE must wait for GDMA"
        );

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 1); // SPI3
        g.write_word(b + IN_PERI_SEL, 1); // SPI3
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (tx_desc as u32 & OUT_LINK_ADDR_MASK),
        );
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (rx_desc as u32 & IN_LINK_ADDR_MASK),
        );
        assert!(g.needs_bus_tick(), "coupled SPI3 transfer pending");

        let ticks = tick_until_idle(&mut g, &mut bus, 8);
        assert_eq!(ticks, 1, "8-byte transfer completes in one tick");

        // MOSI reached the device byte-for-byte.
        assert_eq!(
            *log.as_ref().unwrap().lock().unwrap(),
            payload,
            "device must see the descriptor bytes in order"
        );
        // Device MISO (mosi ^ 0xA5) landed in the IN buffer.
        for (i, &b) in payload.iter().enumerate() {
            assert_eq!(
                bus.read_u8(rx_buf + i as u64).unwrap(),
                b ^ 0xA5,
                "MISO byte [{i}]"
            );
        }
        // SPI completion: TRANS_DONE latched, USR cleared.
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0,
            "TRANS_DONE latched"
        );
        assert_eq!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "USR cleared"
        );
        // GDMA completion: both EOFs latched.
        let out_raw = g.read_word(b + OUT_INT_RAW);
        assert_eq!(out_raw & OUT_EOF_BIT, OUT_EOF_BIT, "OUT_EOF");
        assert_eq!(
            out_raw & OUT_TOTAL_EOF_BIT,
            OUT_TOTAL_EOF_BIT,
            "OUT_TOTAL_EOF"
        );
        assert_eq!(out_raw & OUT_DONE_BIT, OUT_DONE_BIT, "OUT_DONE");
        let in_raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(in_raw & IN_SUC_EOF_BIT, IN_SUC_EOF_BIT, "IN_SUC_EOF");
        assert_eq!(in_raw & IN_DONE_BIT, IN_DONE_BIT, "IN_DONE");
    }

    /// >64-byte multi-descriptor transaction: 200 bytes over two OUT and two
    /// IN descriptors, pumped incrementally (64 bytes/tick → 4 ticks), with
    /// owner-bit writeback and IN length fields verified on every descriptor.
    #[test]
    fn spi3_dma_multi_descriptor_200_bytes_multi_tick() {
        let (mut bus, log) = spi3_test_bus(true);
        let n: usize = 200;
        let payload: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
        let tx_buf1: u64 = 0x3FC8_8000;
        let tx_buf2: u64 = 0x3FC8_8100;
        let rx_buf1: u64 = 0x3FC8_9000;
        let rx_buf2: u64 = 0x3FC8_9100;
        let tx_d1: u64 = 0x3FC8_A000;
        let tx_d2: u64 = 0x3FC8_A010;
        let rx_d1: u64 = 0x3FC8_B000;
        let rx_d2: u64 = 0x3FC8_B010;

        // OUT chain: 120 + 80 bytes.
        for (i, &b) in payload[..120].iter().enumerate() {
            bus.write_u8(tx_buf1 + i as u64, b).unwrap();
        }
        for (i, &b) in payload[120..].iter().enumerate() {
            bus.write_u8(tx_buf2 + i as u64, b).unwrap();
        }
        write_desc(
            &mut bus,
            tx_d1,
            (1 << 31) | (120 << 12) | 120,
            tx_buf1,
            tx_d2,
        );
        write_desc(&mut bus, tx_d2, tx_dw0(80), tx_buf2, 0);
        // IN chain: 128 + 128 capacity (200 bytes → second ends partial at 72).
        write_desc(&mut bus, rx_d1, rx_dw0(128), rx_buf1, rx_d2);
        write_desc(&mut bus, rx_d2, rx_dw0(128), rx_buf2, 0);

        kick_spi_dma(&mut bus, SPI3_BASE, n);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        g.write_word(b + OUT_PERI_SEL, 1); // SPI3
        g.write_word(b + IN_PERI_SEL, 1); // SPI3
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (tx_d1 as u32 & OUT_LINK_ADDR_MASK),
        );
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (rx_d1 as u32 & IN_LINK_ADDR_MASK),
        );

        // Incremental: after one tick (64 bytes) the transaction must NOT
        // be complete — USR still set, no EOF, engine still pending.
        g.tick_with_bus(&mut bus);
        assert!(g.needs_bus_tick(), "200-byte transfer needs >1 tick");
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "USR still set mid-transfer"
        );
        assert_eq!(
            g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT,
            0,
            "no OUT_EOF mid-transfer"
        );

        // 200 bytes at 64/tick → 3 more ticks.
        let extra = tick_until_idle(&mut g, &mut bus, 16);
        assert_eq!(extra, 3, "deterministic 4-tick total for 200 bytes");

        // Device saw all 200 descriptor bytes in order.
        assert_eq!(*log.as_ref().unwrap().lock().unwrap(), payload);
        // MISO landed across both IN buffers.
        for i in 0..128usize {
            assert_eq!(
                bus.read_u8(rx_buf1 + i as u64).unwrap(),
                payload[i] ^ 0xA5,
                "rx_buf1[{i}]"
            );
        }
        for i in 0..72usize {
            assert_eq!(
                bus.read_u8(rx_buf2 + i as u64).unwrap(),
                payload[128 + i] ^ 0xA5,
                "rx_buf2[{i}]"
            );
        }
        // Owner-bit writeback on every consumed descriptor.
        for (name, d) in [
            ("tx_d1", tx_d1),
            ("tx_d2", tx_d2),
            ("rx_d1", rx_d1),
            ("rx_d2", rx_d2),
        ] {
            assert_eq!(
                bus.read_u32(d).unwrap() & DESC_OWNER_BIT,
                0,
                "{name} owner must be 0"
            );
        }
        // IN length fields: full first descriptor, partial second.
        assert_eq!(
            (bus.read_u32(rx_d1).unwrap() >> 12) & 0xFFF,
            128,
            "rx_d1 len"
        );
        assert_eq!(
            (bus.read_u32(rx_d2).unwrap() >> 12) & 0xFFF,
            72,
            "rx_d2 len"
        );
        // Completion flags.
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0
        );
        assert_ne!(g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT, 0);
        assert_ne!(g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT, 0);
    }

    /// TX and RX bound on DIFFERENT channels (ESP-IDF allocates them
    /// independently): the pump pairs directions by PERI_SEL, and each
    /// channel latches its own EOF.
    #[test]
    fn spi3_dma_tx_rx_on_different_channels() {
        let (mut bus, _) = spi3_test_bus(false);
        let n: usize = 4;
        let tx_buf: u64 = 0x3FC8_8000;
        let rx_buf: u64 = 0x3FC8_9000;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;
        for i in 0..n {
            bus.write_u8(tx_buf + i as u64, 0x40 + i as u8).unwrap();
        }
        write_desc(&mut bus, tx_desc, tx_dw0(n as u32), tx_buf, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(n as u32), rx_buf, 0);

        kick_spi_dma(&mut bus, SPI3_BASE, n);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b_tx = ch_base(1);
        let b_rx = ch_base(3);
        g.write_word(b_tx + OUT_PERI_SEL, 1); // SPI3 OUT on ch1
        g.write_word(b_rx + IN_PERI_SEL, 1); // SPI3 IN on ch3
        g.write_word(
            b_tx + OUT_LINK,
            OUT_LINK_START_BIT | (tx_desc as u32 & OUT_LINK_ADDR_MASK),
        );
        g.write_word(
            b_rx + IN_LINK,
            IN_LINK_START_BIT | (rx_desc as u32 & IN_LINK_ADDR_MASK),
        );

        let ticks = tick_until_idle(&mut g, &mut bus, 8);
        assert_eq!(ticks, 1);
        // No device → MISO floats high.
        for i in 0..n {
            assert_eq!(bus.read_u8(rx_buf + i as u64).unwrap(), 0xFF);
        }
        assert_ne!(
            g.read_word(b_tx + OUT_INT_RAW) & OUT_EOF_BIT,
            0,
            "ch1 OUT_EOF"
        );
        assert_ne!(
            g.read_word(b_rx + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "ch3 IN_SUC_EOF"
        );
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0
        );
    }

    /// SPI2 routes through PERI_SEL value 0 to the `spi2_s3` instance.
    #[test]
    fn spi2_dma_routes_by_peri_sel_zero() {
        let mut bus = bus_with_dram();
        bus.add_peripheral(
            "spi2_s3",
            SPI2_BASE,
            0x100,
            None,
            Box::new(Esp32s3Spi::new(21)),
        );
        let n: usize = 4;
        let tx_buf: u64 = 0x3FC8_8000;
        let tx_desc: u64 = 0x3FC8_A000;
        write_desc(&mut bus, tx_desc, tx_dw0(n as u32), tx_buf, 0);

        // TX-only DMA transaction.
        spi_write_u32(&mut bus, SPI2_BASE, SPI_DMA_CONF_REG, SPI_DMA_TX_ENA_BIT);
        spi_write_u32(&mut bus, SPI2_BASE, SPI_MS_DLEN_REG, (n as u32) * 8 - 1);
        spi_write_u32(&mut bus, SPI2_BASE, SPI_CMD_REG, SPI_USR_BIT);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 0); // SPI2
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (tx_desc as u32 & OUT_LINK_ADDR_MASK),
        );

        let ticks = tick_until_idle(&mut g, &mut bus, 8);
        assert_eq!(ticks, 1);
        assert_ne!(g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT, 0, "OUT_EOF");
        assert_ne!(
            spi_read_u32(&mut bus, SPI2_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0,
            "SPI2 TRANS_DONE"
        );
        assert_eq!(
            bus.read_u32(tx_desc).unwrap() & DESC_OWNER_BIT,
            0,
            "TX descriptor returned to CPU"
        );
    }

    /// Order independence: kicking SPI_CMD.USR BEFORE the GDMA links stalls
    /// the pump; starting the links afterwards completes the transaction.
    #[test]
    fn spi3_dma_usr_before_links_stalls_then_completes() {
        let (mut bus, _) = spi3_test_bus(false);
        let n: usize = 4;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;
        write_desc(&mut bus, tx_desc, tx_dw0(n as u32), 0x3FC8_8000, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(n as u32), 0x3FC8_9000, 0);

        kick_spi_dma(&mut bus, SPI3_BASE, n);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 1);
        g.write_word(b + IN_PERI_SEL, 1);
        // Only the OUT link started: RX is DMA-enabled but has no chain yet,
        // so the pump must stall without consuming the transaction.
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (tx_desc as u32 & OUT_LINK_ADDR_MASK),
        );
        g.tick_with_bus(&mut bus);
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "stalled: USR still set with IN link missing"
        );
        assert!(g.needs_bus_tick(), "still pending while stalled");

        // Now start the IN link → completes.
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (rx_desc as u32 & IN_LINK_ADDR_MASK),
        );
        let ticks = tick_until_idle(&mut g, &mut bus, 8);
        assert_eq!(ticks, 1);
        assert_eq!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "USR cleared after both links started"
        );
    }

    /// SPI3 DMA: TRANS_DONE and OUT_EOF/IN_SUC_EOF all latched on the
    /// completion tick.
    #[test]
    fn spi3_dma_trans_done_and_eof_ordering() {
        let (mut bus, _) = spi3_test_bus(false);
        let n: usize = 4;
        let tx_desc: u64 = 0x3FC8_A000;
        let rx_desc: u64 = 0x3FC8_B000;
        write_desc(&mut bus, tx_desc, tx_dw0(n as u32), 0x3FC8_8000, 0);
        write_desc(&mut bus, rx_desc, rx_dw0(n as u32), 0x3FC8_9000, 0);

        kick_spi_dma(&mut bus, SPI3_BASE, n);

        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(0);
        g.write_word(b + OUT_PERI_SEL, 1);
        g.write_word(b + IN_PERI_SEL, 1);
        g.write_word(
            b + OUT_LINK,
            OUT_LINK_START_BIT | (tx_desc as u32 & OUT_LINK_ADDR_MASK),
        );
        g.write_word(
            b + IN_LINK,
            IN_LINK_START_BIT | (rx_desc as u32 & IN_LINK_ADDR_MASK),
        );

        g.tick_with_bus(&mut bus);

        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0,
            "SPI TRANS_DONE"
        );
        assert_ne!(g.read_word(b + OUT_INT_RAW) & OUT_EOF_BIT, 0, "OUT_EOF");
        assert_ne!(
            g.read_word(b + IN_INT_RAW) & IN_SUC_EOF_BIT,
            0,
            "IN_SUC_EOF"
        );
        assert!(!g.needs_bus_tick(), "idle after completion");
    }

    /// Non-DMA regression: USR with the DMA enables clear keeps the
    /// immediate W-buffer completion path byte-identical.
    #[test]
    fn spi3_non_dma_regression() {
        let (mut bus, _) = spi3_test_bus(false);
        spi_write_u32(&mut bus, SPI3_BASE, SPI_MS_DLEN_REG, 32 - 1);
        spi_write_u32(&mut bus, SPI3_BASE, SPI_CMD_REG, SPI_USR_BIT);
        assert_eq!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_CMD_REG) & SPI_USR_BIT,
            0,
            "non-DMA USR auto-clears immediately"
        );
        assert_ne!(
            spi_read_u32(&mut bus, SPI3_BASE, SPI_DMA_INT_RAW_REG) & SPI_TRANS_DONE_BIT,
            0,
            "non-DMA TRANS_DONE latches immediately"
        );
        // MISO region = 0xFF in the W buffer (CPU path).
        assert_eq!(bus.read_u32(SPI3_BASE + 0x98).unwrap(), 0xFFFF_FFFF, "W0");
    }
}
