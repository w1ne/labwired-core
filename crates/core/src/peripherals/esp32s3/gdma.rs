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
//! |   0x00    | IN_CONF0          | RX config 0 (R/W round-trip) |
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
//! ## HONEST LIMITATION — no descriptor walk / memory movement
//!
//! A `Peripheral` only receives `&mut self` in `tick()` / `write()` — it has
//! **no handle to the system bus**, so it cannot read the DMA descriptor
//! linked list pointed at by `INLINK_ADDR` / `OUTLINK_ADDR`, nor copy bytes
//! between peripheral FIFOs and RAM. Real src→dst movement would have to go
//! through the bus `DmaRequest` path (a follow-up).
//!
//! What this model **does** do, which is still far better than the old
//! `0xFFFF_FFFF` catch-all that returned garbage for every GDMA register:
//!
//! * Round-trips **all** per-channel CONF and LINK registers for all 5
//!   channels × IN/OUT, so firmware reads back what it wrote.
//! * **Auto-completes** transfers: writing a channel's `INLINK_START` latches
//!   `IN_SUC_EOF + IN_DONE` in IN_INT_RAW (and self-clears the start bit);
//!   writing `OUTLINK_START` latches `OUT_EOF + OUT_TOTAL_EOF + OUT_DONE` in
//!   OUT_INT_RAW. This lets EOF-waiting DMA firmware make forward progress.
//! * Models INT_RAW/ST/ENA/CLR per channel with W1C INT_CLR, and emits the
//!   correct per-channel interrupt-matrix source via `explicit_irqs` while
//!   that channel's INT_ST is non-zero (level-sensitive re-emit, matching
//!   the `systimer` / `uart` pattern in this crate).

use crate::{Peripheral, PeripheralTickResult, SimResult};

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
                // INLINK_START: kick the RX channel. Auto-complete the
                // transfer by latching IN_SUC_EOF + IN_DONE. The start /
                // restart bits are R/W/SC (self-clearing) so they never
                // read back as set — we simply don't store them.
                if value & (IN_LINK_START_BIT | IN_LINK_RESTART_BIT) != 0 {
                    c.rx.int_raw |= IN_SUC_EOF_BIT | IN_DONE_BIT;
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
                // OUTLINK_START: kick the TX channel. Auto-complete by
                // latching OUT_EOF + OUT_TOTAL_EOF + OUT_DONE.
                if value & (OUT_LINK_START_BIT | OUT_LINK_RESTART_BIT) != 0 {
                    c.tx.int_raw |= OUT_EOF_BIT | OUT_TOTAL_EOF_BIT | OUT_DONE_BIT;
                }
                let _ = OUT_LINK_STOP_BIT;
            }
            _ => {}
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

    #[test]
    fn inlink_start_latches_eof_and_self_clears_start() {
        let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
        let b = ch_base(2);
        g.write_word(b + IN_LINK, IN_LINK_START_BIT | 0x1234);
        let raw = g.read_word(b + IN_INT_RAW);
        assert_eq!(raw & IN_SUC_EOF_BIT, IN_SUC_EOF_BIT, "IN_SUC_EOF latched");
        assert_eq!(raw & IN_DONE_BIT, IN_DONE_BIT, "IN_DONE latched");
        // START is self-clearing: readback never shows bit 22.
        assert_eq!(g.read_word(b + IN_LINK) & IN_LINK_START_BIT, 0);
        // Address was still latched.
        assert_eq!(g.read_word(b + IN_LINK) & IN_LINK_ADDR_MASK, 0x1234);
    }

    #[test]
    fn outlink_start_latches_eof_total_eof_done() {
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
}
