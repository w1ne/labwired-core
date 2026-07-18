// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 DMA controller (datasheet §2.5, base `0x50000000`).
//!
//! Twelve independent channels, each a self-contained transfer engine that
//! moves `TRANS_COUNT` beats of `DATA_SIZE` bytes from `READ_ADDR` to
//! `WRITE_ADDR`. This model implements the memory-to-memory datapath end to
//! end over the real chip bus (`tick_with_bus`): a beat reads `width` bytes
//! from the source, optionally byte-swaps them, writes them to the
//! destination, advances each address by `width` when its `INCR_*` bit is set
//! (with optional `RING` wrapping), and decrements `TRANS_COUNT`. When the
//! count reaches zero the channel clears `BUSY`, latches its `INTR` bit, and —
//! if `CHAIN_TO` names a *different* channel — triggers that channel, so a
//! programmed chain runs to completion with no CPU involvement.
//!
//! ## Register layout (datasheet §2.5.7)
//!
//! Each channel owns a `0x40`-byte block (`CH0` at `0x000`, `CH1` at `0x040`,
//! …). Within a block the four *alias* windows expose the same four backing
//! registers (READ_ADDR / WRITE_ADDR / TRANS_COUNT / CTRL) in different
//! orders; the last register of each alias is a **trigger** — writing it
//! applies the write and then starts the channel:
//!
//! | Off  | Alias 0            | Alias 1 (+0x10)          | Alias 2 (+0x20)          | Alias 3 (+0x30)          |
//! |-----:|--------------------|--------------------------|--------------------------|--------------------------|
//! | +0x0 | READ_ADDR          | CTRL                     | CTRL                     | CTRL                     |
//! | +0x4 | WRITE_ADDR         | READ_ADDR                | TRANS_COUNT              | WRITE_ADDR               |
//! | +0x8 | TRANS_COUNT        | WRITE_ADDR               | READ_ADDR                | TRANS_COUNT              |
//! | +0xC | **CTRL_TRIG**      | **TRANS_COUNT_TRIG**     | **WRITE_ADDR_TRIG**      | **READ_ADDR_TRIG**       |
//!
//! Reads of any alias return the live backing register; `CTRL` reads OR in the
//! live `BUSY` bit (24). The live `READ_ADDR` / `WRITE_ADDR` / `TRANS_COUNT`
//! advance as the transfer runs, so firmware polling them observes progress.
//!
//! ## `CTRL_TRIG` bit fields (datasheet §2.5.7)
//!
//! `EN` [0], `HIGH_PRIORITY` [1], `DATA_SIZE` [3:2] (0=byte,1=half,2=word),
//! `INCR_READ` [4], `INCR_WRITE` [5], `RING_SIZE` [9:6], `RING_SEL` [10],
//! `CHAIN_TO` [14:11], `TREQ_SEL` [20:15], `IRQ_QUIET` [21], `BSWAP` [22],
//! `SNIFF_EN` [23], `BUSY` [24, RO]. `TREQ_SEL == 0x3F` is `TREQ_PERMANENT`
//! (unpaced) — the memory-to-memory transfer request that runs every cycle.
//!
//! ## Interrupts (datasheet §2.5.7)
//!
//! Completion latches the channel's bit in `INTR` (unless `IRQ_QUIET` is set).
//! Two independent aggregators drive the two NVIC lines: `INTS0 =
//! (INTR | INTF0) & INTE0` asserts `DMA_IRQ_0` (NVIC 11), `INTS1 =
//! (INTR | INTF1) & INTE1` asserts `DMA_IRQ_1` (NVIC 12). Delivery is
//! level-sensitive — re-pended every tick until firmware acknowledges by
//! writing the channel bit to `INTS0`/`INTS1` (or `INTR`), matching silicon's
//! held IRQ line. `INTF0`/`INTF1` force a line without a completed transfer.
//!
//! ## Deferred behaviour (documented, not diverging)
//!
//! * **Paced (DREQ) transfers.** Only `TREQ_PERMANENT` auto-runs. A channel
//!   triggered with any other `TREQ_SEL` is a peripheral-paced transfer whose
//!   throttle is the peripheral's DREQ, which this model does not yet source;
//!   such a channel is accepted (its registers store) but does **not** begin,
//!   rather than fabricate an un-paced copy that silicon would never perform.
//!   Modelling per-peripheral DREQ is the documented follow-up (issue #577).
//! * **Sniff CRC** (`SNIFF_CTRL`/`SNIFF_DATA`) is register storage only — the
//!   checksum accumulator is not computed.

use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};

/// Number of DMA channels on the RP2040 (datasheet §2.5).
const NUM_CHANNELS: usize = 12;
/// Per-channel register-block stride.
const CHANNEL_STRIDE: u64 = 0x40;
/// Byte span of the twelve channel blocks (`0x000..0x300`).
const CHANNELS_END: u64 = CHANNEL_STRIDE * NUM_CHANNELS as u64;

// ── Backing-register selector within an alias decode ──
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reg {
    ReadAddr,
    WriteAddr,
    TransCount,
    Ctrl,
}

// ── Interrupt / control block register offsets (datasheet §2.5.7) ──
const INTR: u64 = 0x400; // raw interrupt latch (W1C)
const INTE0: u64 = 0x404;
const INTF0: u64 = 0x408;
const INTS0: u64 = 0x40c; // (INTR|INTF0)&INTE0; W1C acks INTR
const INTE1: u64 = 0x414;
const INTF1: u64 = 0x418;
const INTS1: u64 = 0x41c;
const MULTI_CHAN_TRIGGER: u64 = 0x430;
const SNIFF_CTRL: u64 = 0x434;
const SNIFF_DATA: u64 = 0x438;
const FIFO_LEVELS: u64 = 0x440; // RO, always idle (0) in this model
const CHAN_ABORT: u64 = 0x444; // W1: abort the named channels
const N_CHANNELS: u64 = 0x448; // RO: channel count

// ── CTRL_TRIG bit positions ──
const CTRL_EN: u32 = 1 << 0;
const CTRL_DATA_SIZE_SHIFT: u32 = 2;
const CTRL_DATA_SIZE_MASK: u32 = 0b11;
const CTRL_INCR_READ: u32 = 1 << 4;
const CTRL_INCR_WRITE: u32 = 1 << 5;
const CTRL_RING_SIZE_SHIFT: u32 = 6;
const CTRL_RING_SIZE_MASK: u32 = 0b1111;
const CTRL_RING_SEL: u32 = 1 << 10;
const CTRL_CHAIN_TO_SHIFT: u32 = 11;
const CTRL_CHAIN_TO_MASK: u32 = 0b1111;
const CTRL_TREQ_SEL_SHIFT: u32 = 15;
const CTRL_TREQ_SEL_MASK: u32 = 0b11_1111;
const CTRL_IRQ_QUIET: u32 = 1 << 21;
const CTRL_BSWAP: u32 = 1 << 22;
const CTRL_BUSY: u32 = 1 << 24;

/// `TREQ_SEL` value selecting the permanent (unpaced) transfer request — the
/// memory-to-memory mode that runs a beat every cycle.
const TREQ_PERMANENT: u32 = 0x3F;

/// NVIC IRQ position of `DMA_IRQ_0` (RP2040 §2.5.7 / vector table).
const DMA_IRQ_0: u32 = 11;
/// NVIC IRQ position of `DMA_IRQ_1`.
const DMA_IRQ_1: u32 = 12;

/// Maximum beats moved per channel per `tick_with_bus` call. One beat/cycle is
/// the silicon rate for a permanent-TREQ channel; the sim has no wall clock so
/// the absolute cadence is arbitrary, but pacing one beat per tick keeps the
/// live `TRANS_COUNT`/address progress observable and deterministic.
const BEATS_PER_TICK: u32 = 1;

/// One DMA channel's programmable state (datasheet §2.5.7).
#[derive(Debug, Clone, Copy, Default)]
struct Channel {
    read_addr: u32,
    write_addr: u32,
    trans_count: u32,
    /// `CTRL` without the read-only `BUSY` bit (which lives in `busy`).
    ctrl: u32,
    /// Live `BUSY` — true while the channel is actively transferring.
    busy: bool,
}

impl Channel {
    fn data_size(&self) -> u32 {
        (self.ctrl >> CTRL_DATA_SIZE_SHIFT) & CTRL_DATA_SIZE_MASK
    }
    /// Beat width in bytes: 1 (byte), 2 (half-word) or 4 (word).
    fn width(&self) -> u32 {
        1 << self.data_size()
    }
    fn treq_sel(&self) -> u32 {
        (self.ctrl >> CTRL_TREQ_SEL_SHIFT) & CTRL_TREQ_SEL_MASK
    }
    fn chain_to(&self) -> usize {
        ((self.ctrl >> CTRL_CHAIN_TO_SHIFT) & CTRL_CHAIN_TO_MASK) as usize
    }
    fn ring_size(&self) -> u32 {
        (self.ctrl >> CTRL_RING_SIZE_SHIFT) & CTRL_RING_SIZE_MASK
    }
    /// `CTRL` as firmware reads it: stored bits with the live `BUSY` merged in.
    fn ctrl_read(&self) -> u32 {
        if self.busy {
            self.ctrl | CTRL_BUSY
        } else {
            self.ctrl & !CTRL_BUSY
        }
    }
    /// Advance `addr` by one beat, honouring a `RING` wrap when this direction
    /// (`is_write`) matches `RING_SEL` and `RING_SIZE != 0`. The wrap keeps the
    /// low `RING_SIZE` address bits circulating within a `2^RING_SIZE` window.
    fn advance(&self, addr: u32, is_write: bool) -> u32 {
        let width = self.width();
        let ring_size = self.ring_size();
        let ring_on_write = self.ctrl & CTRL_RING_SEL != 0;
        if ring_size != 0 && is_write == ring_on_write {
            let mask = (1u32 << ring_size) - 1;
            (addr & !mask) | (addr.wrapping_add(width) & mask)
        } else {
            addr.wrapping_add(width)
        }
    }
}

/// RP2040 DMA controller — 12 channels + two interrupt aggregators.
#[derive(Debug)]
pub struct Rp2040Dma {
    channels: [Channel; NUM_CHANNELS],
    /// `INTR` — per-channel raw latched interrupt (bit `c`), write-1-clear.
    intr: u32,
    inte0: u32,
    intf0: u32,
    inte1: u32,
    intf1: u32,
    sniff_ctrl: u32,
    sniff_data: u32,
}

impl Default for Rp2040Dma {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040Dma {
    pub fn new() -> Self {
        Self {
            channels: [Channel::default(); NUM_CHANNELS],
            intr: 0,
            inte0: 0,
            intf0: 0,
            inte1: 0,
            intf1: 0,
            sniff_ctrl: 0,
            sniff_data: 0,
        }
    }

    /// Masked status for the two IRQ lines (datasheet: `INTS = (INTR|INTF)&INTE`).
    fn ints0(&self) -> u32 {
        (self.intr | self.intf0) & self.inte0
    }
    fn ints1(&self) -> u32 {
        (self.intr | self.intf1) & self.inte1
    }

    /// Decode a channel-block offset into `(channel, backing register, trigger?)`.
    fn decode_channel(offset: u64) -> Option<(usize, Reg, bool)> {
        if offset >= CHANNELS_END {
            return None;
        }
        let ch = (offset / CHANNEL_STRIDE) as usize;
        let (reg, trig) = match offset % CHANNEL_STRIDE {
            0x00 => (Reg::ReadAddr, false),
            0x04 => (Reg::WriteAddr, false),
            0x08 => (Reg::TransCount, false),
            0x0c => (Reg::Ctrl, true),  // CTRL_TRIG
            0x10 => (Reg::Ctrl, false), // AL1_CTRL
            0x14 => (Reg::ReadAddr, false),
            0x18 => (Reg::WriteAddr, false),
            0x1c => (Reg::TransCount, true), // AL1_TRANS_COUNT_TRIG
            0x20 => (Reg::Ctrl, false),      // AL2_CTRL
            0x24 => (Reg::TransCount, false),
            0x28 => (Reg::ReadAddr, false),
            0x2c => (Reg::WriteAddr, true), // AL2_WRITE_ADDR_TRIG
            0x30 => (Reg::Ctrl, false),     // AL3_CTRL
            0x34 => (Reg::WriteAddr, false),
            0x38 => (Reg::TransCount, false),
            0x3c => (Reg::ReadAddr, true), // AL3_READ_ADDR_TRIG
            _ => return None,
        };
        Some((ch, reg, trig))
    }

    fn read_word(&self, offset: u64) -> u32 {
        if let Some((ch, reg, _)) = Self::decode_channel(offset) {
            let c = &self.channels[ch];
            return match reg {
                Reg::ReadAddr => c.read_addr,
                Reg::WriteAddr => c.write_addr,
                Reg::TransCount => c.trans_count,
                Reg::Ctrl => c.ctrl_read(),
            };
        }
        match offset {
            INTR => self.intr,
            INTE0 => self.inte0,
            INTF0 => self.intf0,
            INTS0 => self.ints0(),
            INTE1 => self.inte1,
            INTF1 => self.intf1,
            INTS1 => self.ints1(),
            SNIFF_CTRL => self.sniff_ctrl,
            SNIFF_DATA => self.sniff_data,
            FIFO_LEVELS => 0, // no channel FIFO is ever non-empty in this model
            N_CHANNELS => NUM_CHANNELS as u32,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if let Some((ch, reg, trig)) = Self::decode_channel(offset) {
            let c = &mut self.channels[ch];
            match reg {
                Reg::ReadAddr => c.read_addr = value,
                Reg::WriteAddr => c.write_addr = value,
                Reg::TransCount => c.trans_count = value,
                // BUSY (bit 24) is read-only; keep it out of the stored CTRL.
                Reg::Ctrl => c.ctrl = value & !CTRL_BUSY,
            }
            if trig {
                self.try_start(ch);
            }
            return;
        }
        match offset {
            // INTR is write-1-clear of the raw latch.
            INTR => self.intr &= !value,
            INTE0 => self.inte0 = value,
            INTF0 => self.intf0 = value,
            // INTS0 write-1-clear acknowledges the raw interrupt (pico-sdk
            // `dma_channel_acknowledge_irq` writes ints0).
            INTS0 => self.intr &= !value,
            INTE1 => self.inte1 = value,
            INTF1 => self.intf1 = value,
            INTS1 => self.intr &= !value,
            MULTI_CHAN_TRIGGER => {
                for ch in 0..NUM_CHANNELS {
                    if value & (1 << ch) != 0 {
                        self.try_start(ch);
                    }
                }
            }
            CHAN_ABORT => {
                for ch in 0..NUM_CHANNELS {
                    if value & (1 << ch) != 0 {
                        self.channels[ch].busy = false;
                    }
                }
            }
            SNIFF_CTRL => self.sniff_ctrl = value,
            SNIFF_DATA => self.sniff_data = value,
            _ => {}
        }
    }

    /// Begin a channel if it is enabled, has work, and requests the permanent
    /// (unpaced) TREQ. Paced (DREQ) channels are accepted but do not auto-run
    /// (see the module-level "Deferred behaviour" note).
    fn try_start(&mut self, ch: usize) {
        let c = &mut self.channels[ch];
        if c.ctrl & CTRL_EN != 0 && c.trans_count > 0 && c.treq_sel() == TREQ_PERMANENT {
            c.busy = true;
        }
    }

    /// True while any channel is actively transferring — keeps the bus visiting
    /// `tick_with_bus` so beats keep moving.
    fn any_busy(&self) -> bool {
        self.channels.iter().any(|c| c.busy)
    }

    /// Advance every busy channel by up to `BEATS_PER_TICK` beats, moving real
    /// bytes over `bus`. A channel that drains latches its interrupt and, via
    /// `CHAIN_TO`, may trigger a successor in the same pass.
    fn run(&mut self, bus: &mut dyn Bus) {
        // A completing channel can chain to a later-indexed channel that this
        // same pass then advances; the bounded outer loop lets a full chain
        // make progress per tick without unbounded work.
        for ch in 0..NUM_CHANNELS {
            if !self.channels[ch].busy {
                continue;
            }
            for _ in 0..BEATS_PER_TICK {
                if !self.channels[ch].busy {
                    break;
                }
                self.beat(bus, ch);
            }
        }
    }

    /// Move one beat for channel `ch`; complete the channel when it reaches
    /// `TRANS_COUNT == 0`.
    fn beat(&mut self, bus: &mut dyn Bus, ch: usize) {
        let c = &mut self.channels[ch];
        let width = c.width();
        let bswap = c.ctrl & CTRL_BSWAP != 0;

        // Read `width` bytes from the source.
        let mut buf = [0u8; 4];
        for (i, b) in buf.iter_mut().take(width as usize).enumerate() {
            *b = bus.read_u8(c.read_addr as u64 + i as u64).unwrap_or(0);
        }
        // BSWAP reverses the bytes within a (half-word / word) beat.
        if bswap && width > 1 {
            buf[..width as usize].reverse();
        }
        // Write them to the destination.
        for (i, &b) in buf.iter().take(width as usize).enumerate() {
            let _ = bus.write_u8(c.write_addr as u64 + i as u64, b);
        }

        if c.ctrl & CTRL_INCR_READ != 0 {
            c.read_addr = c.advance(c.read_addr, false);
        }
        if c.ctrl & CTRL_INCR_WRITE != 0 {
            c.write_addr = c.advance(c.write_addr, true);
        }
        c.trans_count -= 1;

        if c.trans_count == 0 {
            c.busy = false;
            let irq_quiet = c.ctrl & CTRL_IRQ_QUIET != 0;
            let chain_to = c.chain_to();
            // Completion latches the raw interrupt unless IRQ_QUIET is set.
            if !irq_quiet {
                self.intr |= 1 << ch;
            }
            // CHAIN_TO == own index means "no chain" (datasheet §2.5.2.1);
            // any other value triggers that channel now.
            if chain_to != ch {
                self.try_start(chain_to);
            }
        }
    }
}

impl Peripheral for Rp2040Dma {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !0x3);
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // DMA registers are word-oriented; assemble a full word so trigger and
        // W1C semantics see the intended 32-bit value.
        let aligned = offset & !0x3;
        let shift = (offset & 0x3) * 8;
        let cur = self.read_word(aligned);
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_word(aligned, new);
        Ok(())
    }

    fn needs_bus_tick(&self) -> bool {
        self.any_busy()
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.run(bus);
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Level-sensitive delivery: assert each DMA line while its aggregated
        // status holds, re-pending every tick until firmware acknowledges.
        let mut irqs = Vec::new();
        if self.ints0() != 0 {
            irqs.push(DMA_IRQ_0);
        }
        if self.ints1() != 0 {
            irqs.push(DMA_IRQ_1);
        }
        PeripheralTickResult {
            explicit_irqs: if irqs.is_empty() { None } else { Some(irqs) },
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
    use crate::bus::SystemBus;

    // Channel-block helpers.
    fn ch(n: u64) -> u64 {
        n * CHANNEL_STRIDE
    }
    const READ_ADDR: u64 = 0x00;
    const WRITE_ADDR: u64 = 0x04;
    const TRANS_COUNT: u64 = 0x08;
    const CTRL_TRIG: u64 = 0x0c;

    fn ctrl(en: bool, size: u32, incr_r: bool, incr_w: bool, chain_to: usize) -> u32 {
        let mut v = 0;
        if en {
            v |= CTRL_EN;
        }
        v |= (size & CTRL_DATA_SIZE_MASK) << CTRL_DATA_SIZE_SHIFT;
        if incr_r {
            v |= CTRL_INCR_READ;
        }
        if incr_w {
            v |= CTRL_INCR_WRITE;
        }
        v |= (chain_to as u32) << CTRL_CHAIN_TO_SHIFT;
        v |= TREQ_PERMANENT << CTRL_TREQ_SEL_SHIFT;
        v
    }

    /// A default bus (1 MB RAM mapped at 0x2000_0000) for exercising the
    /// transfer engine directly. All test buffers live inside that window.
    fn ram_bus() -> SystemBus {
        SystemBus::new()
    }

    fn drain(dma: &mut Rp2040Dma, bus: &mut dyn Bus) {
        // Run until no channel is busy (bounded).
        for _ in 0..10_000 {
            if !dma.any_busy() {
                break;
            }
            dma.run(bus);
        }
        assert!(!dma.any_busy(), "transfer did not complete");
    }

    #[test]
    fn decode_aliases_map_to_backing_registers() {
        // Every alias's trigger register is the +0xC slot; alias 0 CTRL_TRIG,
        // alias 1 TRANS_COUNT_TRIG, alias 2 WRITE_ADDR_TRIG, alias 3 READ_ADDR_TRIG.
        assert_eq!(
            Rp2040Dma::decode_channel(0x0c).map(|(c, _, t)| (c, t)),
            Some((0, true))
        );
        assert!(matches!(
            Rp2040Dma::decode_channel(0x1c),
            Some((0, Reg::TransCount, true))
        ));
        assert!(matches!(
            Rp2040Dma::decode_channel(0x2c),
            Some((0, Reg::WriteAddr, true))
        ));
        assert!(matches!(
            Rp2040Dma::decode_channel(0x3c),
            Some((0, Reg::ReadAddr, true))
        ));
        // AL1_READ_ADDR (0x14) aliases the same backing READ_ADDR as 0x00.
        assert!(matches!(
            Rp2040Dma::decode_channel(0x14),
            Some((0, Reg::ReadAddr, false))
        ));
        // Channel 3 block.
        assert_eq!(
            Rp2040Dma::decode_channel(ch(3) + CTRL_TRIG).map(|(c, _, _)| c),
            Some(3)
        );
    }

    #[test]
    fn m2m_copies_bytes_and_zeroes_count() {
        let mut bus = ram_bus();
        let src = 0x2000_1000u32;
        let dst = 0x2000_2000u32;
        let data: [u8; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        for (i, &b) in data.iter().enumerate() {
            bus.write_u8(src as u64 + i as u64, b).unwrap();
        }

        let mut dma = Rp2040Dma::new();
        dma.write_word(READ_ADDR, src);
        dma.write_word(WRITE_ADDR, dst);
        dma.write_word(TRANS_COUNT, 8);
        // byte-size, incr both, no chain (chain_to = self = 0), then trigger.
        dma.write_word(CTRL_TRIG, ctrl(true, 0, true, true, 0));
        assert!(dma.any_busy(), "CTRL_TRIG with EN starts the channel");

        drain(&mut dma, &mut bus);

        for (i, &b) in data.iter().enumerate() {
            assert_eq!(bus.read_u8(dst as u64 + i as u64).unwrap(), b, "byte {i}");
        }
        assert_eq!(dma.read_word(TRANS_COUNT), 0, "TRANS_COUNT drains to 0");
        // Incrementing addresses landed one-past-the-end.
        assert_eq!(dma.read_word(READ_ADDR), src + 8);
        assert_eq!(dma.read_word(WRITE_ADDR), dst + 8);
        // Completion latched the channel-0 raw interrupt.
        assert_eq!(dma.read_word(INTR) & 1, 1);
    }

    #[test]
    fn word_beats_move_four_bytes_each() {
        let mut bus = ram_bus();
        let src = 0x2000_1000u32;
        let dst = 0x2000_2000u32;
        for i in 0..8u32 {
            bus.write_u8(src as u64 + i as u64, i as u8).unwrap();
        }
        let mut dma = Rp2040Dma::new();
        dma.write_word(READ_ADDR, src);
        dma.write_word(WRITE_ADDR, dst);
        dma.write_word(TRANS_COUNT, 2); // 2 word beats = 8 bytes
        dma.write_word(CTRL_TRIG, ctrl(true, 2, true, true, 0));
        drain(&mut dma, &mut bus);
        for i in 0..8u32 {
            assert_eq!(bus.read_u8(dst as u64 + i as u64).unwrap(), i as u8);
        }
        assert_eq!(dma.read_word(READ_ADDR), src + 8);
    }

    #[test]
    fn fixed_write_address_streams_to_one_location() {
        let mut bus = ram_bus();
        let src = 0x2000_1000u32;
        let dst = 0x2000_2000u32;
        for i in 0..4u32 {
            bus.write_u8(src as u64 + i as u64, (i + 1) as u8).unwrap();
        }
        let mut dma = Rp2040Dma::new();
        dma.write_word(READ_ADDR, src);
        dma.write_word(WRITE_ADDR, dst);
        dma.write_word(TRANS_COUNT, 4);
        // incr read, fixed write (INCR_WRITE clear) — last byte wins at dst.
        dma.write_word(CTRL_TRIG, ctrl(true, 0, true, false, 0));
        drain(&mut dma, &mut bus);
        assert_eq!(
            bus.read_u8(dst as u64).unwrap(),
            4,
            "fixed dst holds last byte"
        );
        assert_eq!(dma.read_word(WRITE_ADDR), dst, "write addr did not advance");
    }

    #[test]
    fn chain_to_hands_off_to_second_channel() {
        let mut bus = ram_bus();
        let src0 = 0x2000_1000u32;
        let dst0 = 0x2000_2000u32;
        let src1 = 0x2000_3000u32;
        let dst1 = 0x2000_4000u32;
        for i in 0..4u32 {
            bus.write_u8(src0 as u64 + i as u64, 0xA0 | i as u8)
                .unwrap();
            bus.write_u8(src1 as u64 + i as u64, 0xB0 | i as u8)
                .unwrap();
        }
        let mut dma = Rp2040Dma::new();
        // Channel 1 pre-armed (enabled, permanent TREQ) but NOT triggered.
        dma.write_word(ch(1) + READ_ADDR, src1);
        dma.write_word(ch(1) + WRITE_ADDR, dst1);
        dma.write_word(ch(1) + TRANS_COUNT, 4);
        dma.write_word(ch(1) + 0x10, ctrl(true, 0, true, true, 1)); // AL1_CTRL: no trigger
        assert!(!dma.any_busy(), "channel 1 must not run until chained");

        // Channel 0 chains to channel 1 on completion.
        dma.write_word(ch(0) + READ_ADDR, src0);
        dma.write_word(ch(0) + WRITE_ADDR, dst0);
        dma.write_word(ch(0) + TRANS_COUNT, 4);
        dma.write_word(ch(0) + CTRL_TRIG, ctrl(true, 0, true, true, 1));
        drain(&mut dma, &mut bus);

        for i in 0..4u32 {
            assert_eq!(bus.read_u8(dst0 as u64 + i as u64).unwrap(), 0xA0 | i as u8);
            assert_eq!(bus.read_u8(dst1 as u64 + i as u64).unwrap(), 0xB0 | i as u8);
        }
        // Both channels latched completion interrupts.
        assert_eq!(dma.read_word(INTR) & 0b11, 0b11);
    }

    #[test]
    fn ints0_masks_and_acknowledges() {
        let mut dma = Rp2040Dma::new();
        dma.intr = 0b101; // channels 0 and 2 raw-pending
        dma.write_word(INTE0, 0b001); // only channel 0 enabled on line 0
        assert_eq!(dma.ints0(), 0b001, "INTS0 = INTR & INTE0");
        assert_eq!(dma.tick().explicit_irqs, Some(vec![DMA_IRQ_0]));
        // Ack channel 0 via INTS0 write-1-clear.
        dma.write_word(INTS0, 0b001);
        assert_eq!(dma.intr, 0b100, "INTS0 W1C cleared channel 0 raw bit");
        assert!(
            dma.tick().explicit_irqs.is_none(),
            "line 0 clears after ack"
        );
    }

    #[test]
    fn second_irq_line_is_independent() {
        let mut dma = Rp2040Dma::new();
        dma.intr = 0b10; // channel 1 pending
        dma.write_word(INTE1, 0b10); // routed to DMA_IRQ_1 only
        assert_eq!(dma.tick().explicit_irqs, Some(vec![DMA_IRQ_1]));
    }

    #[test]
    fn irq_quiet_channel_latches_no_interrupt() {
        let mut bus = ram_bus();
        bus.write_u8(0x2000_1000, 0x5A).unwrap();
        let mut dma = Rp2040Dma::new();
        dma.write_word(READ_ADDR, 0x2000_1000);
        dma.write_word(WRITE_ADDR, 0x2000_2000);
        dma.write_word(TRANS_COUNT, 1);
        dma.write_word(CTRL_TRIG, ctrl(true, 0, true, true, 0) | CTRL_IRQ_QUIET);
        drain(&mut dma, &mut bus);
        assert_eq!(
            bus.read_u8(0x2000_2000).unwrap(),
            0x5A,
            "transfer still ran"
        );
        assert_eq!(dma.read_word(INTR), 0, "IRQ_QUIET suppresses the raw latch");
    }

    #[test]
    fn paced_treq_channel_does_not_auto_run() {
        let mut dma = Rp2040Dma::new();
        dma.write_word(READ_ADDR, 0x2000_1000);
        dma.write_word(WRITE_ADDR, 0x2000_2000);
        dma.write_word(TRANS_COUNT, 4);
        // TREQ_SEL = 0 (a paced DREQ), EN set, triggered: must NOT start.
        let paced = CTRL_EN; // treq_sel field left 0
        dma.write_word(CTRL_TRIG, paced);
        assert!(
            !dma.any_busy(),
            "paced channel waits for a DREQ we don't source"
        );
    }

    #[test]
    fn abort_clears_busy() {
        let mut dma = Rp2040Dma::new();
        dma.write_word(READ_ADDR, 0x2000_1000);
        dma.write_word(WRITE_ADDR, 0x2000_2000);
        dma.write_word(TRANS_COUNT, 100);
        dma.write_word(CTRL_TRIG, ctrl(true, 0, true, true, 0));
        assert!(dma.any_busy());
        dma.write_word(CHAN_ABORT, 1);
        assert!(!dma.any_busy(), "CHAN_ABORT halts the channel");
    }

    #[test]
    fn n_channels_reads_twelve() {
        let dma = Rp2040Dma::new();
        assert_eq!(dma.read_word(N_CHANNELS), 12);
    }

    #[test]
    fn ctrl_read_reflects_busy_bit() {
        let mut dma = Rp2040Dma::new();
        dma.write_word(READ_ADDR, 0x2000_1000);
        dma.write_word(WRITE_ADDR, 0x2000_2000);
        dma.write_word(TRANS_COUNT, 50);
        dma.write_word(CTRL_TRIG, ctrl(true, 0, true, true, 0));
        assert_ne!(
            dma.read_word(CTRL_TRIG) & CTRL_BUSY,
            0,
            "BUSY set mid-transfer"
        );
    }
}
