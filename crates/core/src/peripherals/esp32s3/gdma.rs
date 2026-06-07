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
//! |   0x60    | OUT_CONF0         | TX config 0 (R/W round-trip) |
//! |   0x64    | OUT_CONF1         | TX config 1 (R/W round-trip) |
//! |   0x68    | OUT_INT_RAW       | bit0 OUT_DONE, bit1 OUT_EOF, bit2 OUT_DSCR_ERR, bit3 OUT_TOTAL_EOF |
//! |   0x6C    | OUT_INT_ST        | RAW & ENA (RO) |
//! |   0x70    | OUT_INT_ENA       | per-bit enable (R/W) |
//! |   0x74    | OUT_INT_CLR       | W1C of OUT_INT_RAW |
//! |   0x80    | OUT_LINK          | addr[19:0], stop[20], start[21], restart[22], park[23] (RO=1) |
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
//! ## What remains unimplemented (non-m2m peripheral-coupled transfers)
//!
//! Peripheral-paired DMA (SPI2/3, I2S, ADC, AES, SHA, …) still uses the
//! original auto-complete behaviour: writing `OUTLINK_START` latches
//! `OUT_EOF + OUT_TOTAL_EOF + OUT_DONE`; writing `INLINK_START` latches
//! `IN_SUC_EOF + IN_DONE` — without actual byte movement. This keeps those
//! peripheral drivers making forward progress without modelling the FIFO
//! handshake, matching the previous behaviour.

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

// ── OUT (TX) sub-block offsets within a channel block ──
const OUT_CONF0: u64 = 0x60;
const OUT_CONF1: u64 = 0x64;
const OUT_INT_RAW: u64 = 0x68;
const OUT_INT_ST: u64 = 0x6C;
const OUT_INT_ENA: u64 = 0x70;
const OUT_INT_CLR: u64 = 0x74;
const OUT_LINK: u64 = 0x80;

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

/// One direction (IN or OUT) of a GDMA channel.
#[derive(Debug, Default, Clone, Copy)]
struct DmaDir {
    conf0: u32,
    conf1: u32,
    /// Latched descriptor-list base address (bits[19:0] of the LINK reg).
    link_addr: u32,
    /// INT_RAW — sticky pending bits, cleared only by INT_CLR (W1C).
    int_raw: u32,
    /// INT_ENA — per-bit IRQ enable.
    int_ena: u32,
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
            OUT_CONF0 => c.tx.conf0,
            OUT_CONF1 => c.tx.conf1,
            OUT_INT_RAW => c.tx.int_raw,
            OUT_INT_ST => c.tx.int_raw & c.tx.int_ena,
            OUT_INT_ENA => c.tx.int_ena,
            OUT_INT_CLR => 0,
            OUT_LINK => (c.tx.link_addr & OUT_LINK_ADDR_MASK) | OUT_LINK_PARK_BIT,
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
                        // Peripheral-coupled mode (unimplemented beyond
                        // register model): auto-complete so the firmware
                        // polling IN_SUC_EOF can make forward progress.
                        c.rx.int_raw |= IN_SUC_EOF_BIT | IN_DONE_BIT;
                    }
                }
                // STOP: nothing to do in this register-only model.
                let _ = IN_LINK_STOP_BIT;
            }
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
                        // Peripheral-coupled auto-complete.
                        c.tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
                    }
                }
                let _ = OUT_LINK_STOP_BIT;
            }
            _ => {}
        }
    }

    /// Walk an OUT (TX) descriptor chain starting at `desc_addr` and collect
    /// all bytes from the data buffers. Returns the bytes if successful.
    ///
    /// Stops at the first descriptor whose `owner` bit is 0 (CPU-owned),
    /// at `next == 0` (end-of-list), or after `MAX_DESC_CHAIN` hops.
    fn walk_out_chain(bus: &dyn Bus, desc_addr: u64) -> Vec<u8> {
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

            if next_ptr == 0 {
                break;
            }
            addr = next_ptr;
        }
        bytes
    }

    /// Walk an IN (RX) descriptor chain starting at `desc_addr` and write
    /// `bytes` into the data buffers. Sets `IN_SUC_EOF` on the last
    /// descriptor it fills.
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

    /// True when any channel has a pending MEM_TRANS_EN descriptor walk.
    fn needs_bus_tick(&self) -> bool {
        self.channels.iter().any(|c| c.pending_m2m)
    }

    /// Execute all pending memory-to-memory descriptor walks.
    ///
    /// For each channel with `pending_m2m` set:
    /// 1. Walk the OUT (TX) descriptor chain and collect bytes.
    /// 2. Walk the IN (RX) descriptor chain and write bytes.
    /// 3. Latch `IN_SUC_EOF | IN_DONE` and `OUT_EOF | OUT_TOTAL_EOF |
    ///    OUT_DONE` in the respective INT_RAW registers.
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        for c in self.channels.iter_mut() {
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
}
