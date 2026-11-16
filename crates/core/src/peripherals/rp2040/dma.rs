// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 / RP2350 DMA controller (RP2040 datasheet §2.5, RP2350 §12.6;
//! base `0x50000000`).
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
//! ## RP2350 layout (`config: { profile: rp2350 }`)
//!
//! Same channel-block aliases, but 16 channels (`0x000..0x400`) and four
//! interrupt aggregators (`INTE/INTF/INTS 0..3` at `0x404 + 0x10 * k`, NVIC
//! `DMA_IRQ_0..3` = 10..13). The pacing timers grow to 0x440..0x44C, so the
//! control block moves up by 0x20 (`MULTI_CHAN_TRIGGER` 0x450, `CHAN_ABORT`
//! 0x464, `N_CHANNELS` 0x468). Read with the RP2040 map, a pico-sdk `INTS3`
//! acknowledge at 0x430 lands on `MULTI_CHAN_TRIGGER`. `CTRL_TRIG` moves
//! too: `INCR_READ_REV` [5], `INCR_WRITE` [6], `INCR_WRITE_REV` [7],
//! `RING_SIZE` [11:8], `RING_SEL` [12], `CHAIN_TO` [16:13], `TREQ_SEL`
//! [22:17], `IRQ_QUIET` [23], `BSWAP` [24], `SNIFF_EN` [25], `BUSY` [26].
//! `TRANS_COUNT` carries `MODE` [31:28] over a 28-bit `COUNT`: `ENDLESS`
//! never decrements, `TRIGGER_SELF` re-triggers on completion. The
//! security/MPU registers (`SECCFG_*`, `MPU_*`) are not modelled.
//!
//! Both layouts reload the live counter from the last written `TRANS_COUNT`
//! on every trigger of an idle channel, so a re-triggered or chained channel
//! moves the programmed count again.
//!
//! ## Deferred behaviour (documented, not diverging)
//!
//! * **Paced (DREQ) transfers.** A channel whose `TREQ_SEL` names a
//!   peripheral DREQ (`0x00..=0x3A`) moves a beat only while the peripheral
//!   at its endpoint raises its request: the write address for a transfer
//!   into a peripheral (`Bus::dma_tx_request`, e.g. a PIO `TXFn` with room),
//!   the read address for one out of it (`Bus::dma_rx_request`, a PIO `RXFn`
//!   holding a word). The request is the endpoint's live FIFO state, read
//!   before each beat, so the channel never runs ahead of the peripheral.
//!   An endpoint that does not source its DREQ in this engine answers
//!   neither; such a channel is accepted (its registers store) but does
//!   **not** begin, rather than fabricate an un-paced copy that silicon would
//!   never perform. The DMA pacing timers (`TREQ_SEL 0x3B..=0x3E`) are not
//!   modelled and never begin either.
//! * **Sniff CRC** (`SNIFF_CTRL`/`SNIFF_DATA`) is register storage only — the
//!   checksum accumulator is not computed.

use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult};

/// Channel slots the model holds: the RP2350's sixteen. A layout uses the
/// first [`DmaMap::channels`] of them.
const MAX_CHANNELS: usize = 16;
/// Interrupt aggregators the model holds: the RP2350's four.
const MAX_IRQ_LINES: usize = 4;
/// Per-channel register-block stride.
const CHANNEL_STRIDE: u64 = 0x40;

// ── Backing-register selector within an alias decode ──
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reg {
    ReadAddr,
    WriteAddr,
    TransCount,
    Ctrl,
}

// ── Interrupt block (both chips, datasheet §2.5.7 / RP2350 §12.6.10) ──
/// `INTR`, the raw interrupt latch (W1C). Aggregator `k` sits at
/// `INTR + 0x10 * k`: `INTE` +0x4, `INTF` +0x8, `INTS` +0xC.
const INTR: u64 = 0x400;
const IRQ_LINE_STRIDE: u64 = 0x10;
#[cfg(test)]
const INTE0: u64 = 0x404;
#[cfg(test)]
const INTS0: u64 = 0x40c;
#[cfg(test)]
const INTE1: u64 = 0x414;

// ── Control block, relative to `DmaMap::ctl_base` (`MULTI_CHAN_TRIGGER`) ──
const MULTI_CHAN_TRIGGER: u64 = 0x00;
const SNIFF_CTRL: u64 = 0x04;
const SNIFF_DATA: u64 = 0x08;
const FIFO_LEVELS: u64 = 0x10; // RO, always idle (0) in this model
const CHAN_ABORT: u64 = 0x14; // W1: abort the named channels
const N_CHANNELS: u64 = 0x18; // RO: channel count

// ── CTRL_TRIG bits at the same place on both chips ──
const CTRL_EN: u32 = 1 << 0;
const CTRL_DATA_SIZE_SHIFT: u32 = 2;
const CTRL_DATA_SIZE_MASK: u32 = 0b11;
const CTRL_INCR_READ: u32 = 1 << 4;
const CTRL_RING_SIZE_MASK: u32 = 0b1111;
const CTRL_CHAIN_TO_MASK: u32 = 0b1111;
const CTRL_TREQ_SEL_MASK: u32 = 0b11_1111;

// ── RP2040 CTRL_TRIG bit positions ──
const CTRL_INCR_WRITE: u32 = 1 << 5;
const CTRL_RING_SIZE_SHIFT: u32 = 6;
const CTRL_RING_SEL: u32 = 1 << 10;
const CTRL_CHAIN_TO_SHIFT: u32 = 11;
const CTRL_TREQ_SEL_SHIFT: u32 = 15;
const CTRL_IRQ_QUIET: u32 = 1 << 21;
const CTRL_BSWAP: u32 = 1 << 22;
const CTRL_BUSY: u32 = 1 << 24;

/// `TREQ_SEL` value selecting the permanent (unpaced) transfer request — the
/// memory-to-memory mode that runs a beat every cycle.
const TREQ_PERMANENT: u32 = 0x3F;
/// Highest `TREQ_SEL` naming a peripheral DREQ (RP2040 §2.5.3.1); `0x3B..=0x3E`
/// are the DMA pacing timers on both chips.
const TREQ_LAST_DREQ: u32 = 0x3A;

/// NVIC IRQ position of `DMA_IRQ_0` (RP2040 §2.5.7 / vector table).
const DMA_IRQ_0: u32 = 11;
/// NVIC IRQ position of `DMA_IRQ_1`.
#[cfg(test)]
const DMA_IRQ_1: u32 = 12;

/// RP2350 `TRANS_COUNT.MODE` [31:28] and `COUNT` [27:0].
const COUNT_MODE_SHIFT: u32 = 28;
const COUNT_MASK_RP2350: u32 = 0x0FFF_FFFF;
const MODE_TRIGGER_SELF: u32 = 0x1;
const MODE_ENDLESS: u32 = 0xF;

/// Register-map variant (`config: { profile: rp2350 }` selects the RP2350).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DmaLayout {
    /// RP2040: 12 channels, two IRQ lines, `MULTI_CHAN_TRIGGER` at 0x430.
    #[default]
    Rp2040,
    /// RP2350: 16 channels, four IRQ lines, `MULTI_CHAN_TRIGGER` at 0x450,
    /// moved `CTRL` fields and a `TRANS_COUNT.MODE` field.
    Rp2350,
}

/// What differs between the two chips' DMA blocks (pico-sdk
/// `rp2040`/`rp2350` `hardware_regs/dma.h` and `intctrl.h`).
#[derive(Debug)]
struct DmaMap {
    channels: usize,
    irq_lines: usize,
    /// NVIC position of `DMA_IRQ_0`; line `k` is `first_irq + k`.
    first_irq: u32,
    /// Offset of `MULTI_CHAN_TRIGGER`, the start of the control block.
    ctl_base: u64,
    incr_write: u32,
    /// `INCR_READ_REV` / `INCR_WRITE_REV` (RP2350 only; 0 = absent).
    incr_read_rev: u32,
    incr_write_rev: u32,
    ring_size_shift: u32,
    ring_sel: u32,
    chain_to_shift: u32,
    treq_sel_shift: u32,
    irq_quiet: u32,
    bswap: u32,
    busy: u32,
    /// `TRANS_COUNT` carries `MODE` [31:28] above a 28-bit `COUNT`.
    count_mode: bool,
}

const RP2040_MAP: DmaMap = DmaMap {
    channels: 12,
    irq_lines: 2,
    first_irq: DMA_IRQ_0,
    ctl_base: 0x430,
    incr_write: CTRL_INCR_WRITE,
    incr_read_rev: 0,
    incr_write_rev: 0,
    ring_size_shift: CTRL_RING_SIZE_SHIFT,
    ring_sel: CTRL_RING_SEL,
    chain_to_shift: CTRL_CHAIN_TO_SHIFT,
    treq_sel_shift: CTRL_TREQ_SEL_SHIFT,
    irq_quiet: CTRL_IRQ_QUIET,
    bswap: CTRL_BSWAP,
    busy: CTRL_BUSY,
    count_mode: false,
};

const RP2350_MAP: DmaMap = DmaMap {
    channels: 16,
    irq_lines: 4,
    first_irq: 10,
    ctl_base: 0x450,
    incr_write: 1 << 6,
    incr_read_rev: 1 << 5,
    incr_write_rev: 1 << 7,
    ring_size_shift: 8,
    ring_sel: 1 << 12,
    chain_to_shift: 13,
    treq_sel_shift: 17,
    irq_quiet: 1 << 23,
    bswap: 1 << 24,
    busy: 1 << 26,
    count_mode: true,
};

impl DmaLayout {
    fn map(self) -> &'static DmaMap {
        match self {
            DmaLayout::Rp2040 => &RP2040_MAP,
            DmaLayout::Rp2350 => &RP2350_MAP,
        }
    }
}

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
    /// Live transfer counter (`COUNT` only).
    trans_count: u32,
    /// `TRANS_COUNT` as last written: the reload value copied into the live
    /// counter on each trigger, with the RP2350 `MODE` field on top.
    reload: u32,
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
    fn treq_sel(&self, map: &DmaMap) -> u32 {
        (self.ctrl >> map.treq_sel_shift) & CTRL_TREQ_SEL_MASK
    }
    fn chain_to(&self, map: &DmaMap) -> usize {
        ((self.ctrl >> map.chain_to_shift) & CTRL_CHAIN_TO_MASK) as usize
    }
    fn ring_size(&self, map: &DmaMap) -> u32 {
        (self.ctrl >> map.ring_size_shift) & CTRL_RING_SIZE_MASK
    }
    /// RP2350 `TRANS_COUNT.MODE` (always normal on the RP2040).
    fn mode(&self, map: &DmaMap) -> u32 {
        if map.count_mode {
            self.reload >> COUNT_MODE_SHIFT
        } else {
            0
        }
    }
    fn reload_count(&self, map: &DmaMap) -> u32 {
        if map.count_mode {
            self.reload & COUNT_MASK_RP2350
        } else {
            self.reload
        }
    }
    /// `TRANS_COUNT` as firmware reads it: the live counter, with the
    /// written `MODE` field on top on the RP2350.
    fn trans_count_read(&self, map: &DmaMap) -> u32 {
        if map.count_mode {
            (self.reload & !COUNT_MASK_RP2350) | self.trans_count
        } else {
            self.trans_count
        }
    }
    /// `CTRL` as firmware reads it: stored bits with the live `BUSY` merged in.
    fn ctrl_read(&self, map: &DmaMap) -> u32 {
        if self.busy {
            self.ctrl | map.busy
        } else {
            self.ctrl & !map.busy
        }
    }
    /// Advance `addr` by one beat, honouring a `RING` wrap when this direction
    /// (`is_write`) matches `RING_SEL` and `RING_SIZE != 0`. The wrap keeps the
    /// low `RING_SIZE` address bits circulating within a `2^RING_SIZE` window.
    /// An RP2350 `INCR_*_REV` bit steps the address down instead of up.
    fn advance(&self, map: &DmaMap, addr: u32, is_write: bool) -> u32 {
        let rev = if is_write {
            map.incr_write_rev
        } else {
            map.incr_read_rev
        };
        let step = if rev != 0 && self.ctrl & rev != 0 {
            self.width().wrapping_neg()
        } else {
            self.width()
        };
        let ring_size = self.ring_size(map);
        let ring_on_write = self.ctrl & map.ring_sel != 0;
        if ring_size != 0 && is_write == ring_on_write {
            let mask = (1u32 << ring_size) - 1;
            (addr & !mask) | (addr.wrapping_add(step) & mask)
        } else {
            addr.wrapping_add(step)
        }
    }
}

/// RP2040 / RP2350 DMA controller — 12 or 16 channels + two or four
/// interrupt aggregators, per [`DmaLayout`].
#[derive(Debug)]
pub struct Rp2040Dma {
    map: &'static DmaMap,
    channels: [Channel; MAX_CHANNELS],
    /// `INTR` — per-channel raw latched interrupt (bit `c`), write-1-clear.
    intr: u32,
    /// `INTE0..3` / `INTF0..3`; a layout uses the first `irq_lines`.
    inte: [u32; MAX_IRQ_LINES],
    intf: [u32; MAX_IRQ_LINES],
    sniff_ctrl: u32,
    sniff_data: u32,
    /// Bus-published cycle clock: presence means scheduler mode (walk-free).
    clock: Option<CycleClock>,
    /// True while a transfer / level-IRQ event chain is live.
    chain_live: bool,
}

impl Default for Rp2040Dma {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040Dma {
    pub fn new() -> Self {
        Self {
            map: DmaLayout::Rp2040.map(),
            channels: [Channel::default(); MAX_CHANNELS],
            intr: 0,
            inte: [0; MAX_IRQ_LINES],
            intf: [0; MAX_IRQ_LINES],
            sniff_ctrl: 0,
            sniff_data: 0,
            clock: None,
            chain_live: false,
        }
    }

    /// Select the register map (factory `config: { profile: rp2350 }`).
    pub fn with_layout(mut self, layout: DmaLayout) -> Self {
        self.map = layout.map();
        self
    }

    crate::cycle_clock::scheduler_mode!();

    /// True while there is transfer work or a held IRQ level to re-pend.
    fn needs_event_chain(&self) -> bool {
        self.any_busy() || (0..self.map.irq_lines).any(|k| self.ints(k) != 0)
    }

    /// Level-sensitive IRQ vector for the DMA lines.
    fn level_irqs(&self) -> PeripheralTickResult {
        let irqs: Vec<u32> = (0..self.map.irq_lines)
            .filter(|&k| self.ints(k) != 0)
            .map(|k| self.map.first_irq + k as u32)
            .collect();
        PeripheralTickResult {
            explicit_irqs: if irqs.is_empty() { None } else { Some(irqs) },
            ..Default::default()
        }
    }

    /// Masked status for IRQ line `k` (datasheet: `INTS = (INTR|INTF)&INTE`).
    fn ints(&self, k: usize) -> u32 {
        (self.intr | self.intf[k]) & self.inte[k]
    }

    /// Decode a channel-block offset into `(channel, backing register, trigger?)`.
    fn decode_channel(&self, offset: u64) -> Option<(usize, Reg, bool)> {
        if offset >= CHANNEL_STRIDE * self.map.channels as u64 {
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

    /// Decode an interrupt-block offset into `(line, register within line)`:
    /// 0x0 `INTR` (line 0 only), 0x4 `INTE`, 0x8 `INTF`, 0xC `INTS`.
    fn decode_irq(&self, offset: u64) -> Option<(usize, u64)> {
        let rel = offset.checked_sub(INTR)?;
        let line = (rel / IRQ_LINE_STRIDE) as usize;
        let reg = rel % IRQ_LINE_STRIDE;
        (line < self.map.irq_lines && (reg != 0 || line == 0)).then_some((line, reg))
    }

    fn read_word(&self, offset: u64) -> u32 {
        let map = self.map;
        if let Some((ch, reg, _)) = self.decode_channel(offset) {
            let c = &self.channels[ch];
            return match reg {
                Reg::ReadAddr => c.read_addr,
                Reg::WriteAddr => c.write_addr,
                Reg::TransCount => c.trans_count_read(map),
                Reg::Ctrl => c.ctrl_read(map),
            };
        }
        if let Some((k, reg)) = self.decode_irq(offset) {
            return match reg {
                0x0 => self.intr,
                0x4 => self.inte[k],
                0x8 => self.intf[k],
                _ => self.ints(k),
            };
        }
        match offset.wrapping_sub(map.ctl_base) {
            SNIFF_CTRL => self.sniff_ctrl,
            SNIFF_DATA => self.sniff_data,
            FIFO_LEVELS => 0, // no channel FIFO is ever non-empty in this model
            N_CHANNELS => map.channels as u32,
            _ => {
                crate::census_reg!("rp2040.dma:Rp2040Dma", offset, "read");
                0
            }
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let map = self.map;
        if let Some((ch, reg, trig)) = self.decode_channel(offset) {
            let c = &mut self.channels[ch];
            match reg {
                Reg::ReadAddr => c.read_addr = value,
                Reg::WriteAddr => c.write_addr = value,
                Reg::TransCount => {
                    c.reload = value;
                    // While idle the write also shows in the live counter.
                    if !c.busy {
                        c.trans_count = c.reload_count(map);
                    }
                }
                // BUSY is read-only; keep it out of the stored CTRL.
                Reg::Ctrl => c.ctrl = value & !map.busy,
            }
            if trig {
                self.try_start(ch);
            }
            return;
        }
        if let Some((k, reg)) = self.decode_irq(offset) {
            match reg {
                // INTR is write-1-clear of the raw latch.
                0x0 => self.intr &= !value,
                0x4 => self.inte[k] = value,
                0x8 => self.intf[k] = value,
                // INTSk write-1-clear acknowledges the raw interrupt (pico-sdk
                // `dma_channel_acknowledge_irq` writes ints0).
                _ => self.intr &= !value,
            }
            return;
        }
        match offset.wrapping_sub(map.ctl_base) {
            MULTI_CHAN_TRIGGER => {
                for ch in 0..map.channels {
                    if value & (1 << ch) != 0 {
                        self.try_start(ch);
                    }
                }
            }
            CHAN_ABORT => {
                for ch in 0..map.channels {
                    if value & (1 << ch) != 0 {
                        self.channels[ch].busy = false;
                    }
                }
            }
            SNIFF_CTRL => self.sniff_ctrl = value,
            SNIFF_DATA => self.sniff_data = value,
            _ => {
                crate::census_reg!("rp2040.dma:Rp2040Dma", offset, "write");
            }
        }
    }

    /// Trigger channel `ch`: an idle channel copies its reload value into the
    /// live counter, then begins if it is enabled, has work, and requests the
    /// permanent (unpaced) TREQ or a DREQ. Paced (DREQ) channels then move a
    /// beat only while their endpoint requests one (see the module docs).
    /// Triggering a busy channel has no effect.
    fn try_start(&mut self, ch: usize) {
        let map = self.map;
        let c = &mut self.channels[ch];
        if c.busy {
            return;
        }
        c.trans_count = c.reload_count(map);
        let treq = c.treq_sel(map);
        if c.ctrl & CTRL_EN != 0
            && (c.trans_count > 0 || c.mode(map) == MODE_ENDLESS)
            && (treq == TREQ_PERMANENT || treq <= TREQ_LAST_DREQ)
        {
            c.busy = true;
        }
    }

    /// For a paced channel: whether its endpoint's DREQ is raised now.
    /// `None` when neither endpoint sources a DREQ in this engine.
    fn dreq(c: &Channel, bus: &dyn Bus) -> Option<bool> {
        bus.dma_tx_request(u64::from(c.write_addr))
            .or_else(|| bus.dma_rx_request(u64::from(c.read_addr)))
    }

    /// True while any channel is actively transferring — keeps the bus visiting
    /// `tick_with_bus` so beats keep moving.
    fn any_busy(&self) -> bool {
        self.channels[..self.map.channels].iter().any(|c| c.busy)
    }

    /// Advance every busy channel by up to `BEATS_PER_TICK` beats, moving real
    /// bytes over `bus`. A channel that drains latches its interrupt and, via
    /// `CHAIN_TO`, may trigger a successor in the same pass.
    fn run(&mut self, bus: &mut dyn Bus) {
        // A completing channel can chain to a later-indexed channel that this
        // same pass then advances; the bounded outer loop lets a full chain
        // make progress per tick without unbounded work.
        for ch in 0..self.map.channels {
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
        let map = self.map;
        if self.channels[ch].treq_sel(map) != TREQ_PERMANENT {
            match Self::dreq(&self.channels[ch], bus) {
                Some(true) => {}
                // DREQ low: the channel waits, BUSY held.
                Some(false) => return,
                // Nothing sources this DREQ: never begin (module docs).
                None => {
                    self.channels[ch].busy = false;
                    return;
                }
            }
        }
        let c = &mut self.channels[ch];
        let width = c.width();
        let bswap = c.ctrl & map.bswap != 0;

        if width == 4 && c.read_addr & 3 == 0 && c.write_addr & 3 == 0 {
            // A word beat is one 32-bit access on each side, so a FIFO
            // register (a PIO `RXFn`) pops once, not per byte.
            crate::with_device_write_bypass(bus, |bus| {
                let mut word = bus.read_u32(u64::from(c.read_addr)).unwrap_or(0);
                if bswap {
                    word = word.swap_bytes();
                }
                let _ = bus.write_u32(u64::from(c.write_addr), word);
            });
        } else {
            // Read `width` bytes from the source.
            let mut buf = [0u8; 4];
            crate::with_device_write_bypass(bus, |bus| {
                for (i, b) in buf.iter_mut().take(width as usize).enumerate() {
                    *b = bus.read_u8(c.read_addr as u64 + i as u64).unwrap_or(0);
                }
            });
            // BSWAP reverses the bytes within a (half-word / word) beat.
            if bswap && width > 1 {
                buf[..width as usize].reverse();
            }
            // Write them to the destination.
            crate::with_device_write_bypass(bus, |bus| {
                for (i, &b) in buf.iter().take(width as usize).enumerate() {
                    let _ = bus.write_u8(c.write_addr as u64 + i as u64, b);
                }
            });
        }

        if c.ctrl & CTRL_INCR_READ != 0 {
            c.read_addr = c.advance(map, c.read_addr, false);
        }
        if c.ctrl & map.incr_write != 0 {
            c.write_addr = c.advance(map, c.write_addr, true);
        }
        let mode = c.mode(map);
        if mode == MODE_ENDLESS {
            // RP2350 endless mode: the counter never decrements.
            return;
        }
        c.trans_count -= 1;

        if c.trans_count == 0 {
            c.busy = false;
            let irq_quiet = c.ctrl & map.irq_quiet != 0;
            let chain_to = c.chain_to(map);
            // Completion latches the raw interrupt unless IRQ_QUIET is set.
            if !irq_quiet {
                self.intr |= 1 << ch;
            }
            // RP2350 TRIGGER_SELF re-triggers the channel on completion.
            if mode == MODE_TRIGGER_SELF {
                self.try_start(ch);
            }
            // CHAIN_TO == own index means "no chain" (datasheet §2.5.2.1);
            // any other value triggers that channel now.
            if chain_to != ch && chain_to < map.channels {
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
        // Scheduler mode owns transfers via `on_event` (avoids double-drive).
        !self.scheduler_mode() && self.any_busy()
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if self.scheduler_mode() {
            return;
        }
        self.run(bus);
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Level-sensitive delivery: assert each DMA line while its aggregated
        // status holds, re-pending every tick until firmware acknowledges.
        // Scheduler mode skips the walk; the event chain owns re-emission.
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        self.level_irqs()
    }

    /// Oracle settle path: re-assert the held DMA lines even under scheduler
    /// mode. The bare-CPU oracle has no `Machine` draining `on_event`, so
    /// without this the completion IRQ is never delivered and the ISR never
    /// runs — the `tick_elapsed_forced` hole the `core-ci` "Feature gate —
    /// hardware oracle under event-scheduler" step exists to catch.
    fn tick_elapsed_forced(&mut self, _cycles: u64) -> PeripheralTickResult {
        self.level_irqs()
    }

    /// Oracle settle path, transfer half: this model moves its bytes in
    /// `tick_with_bus`, and scheduler mode deliberately hides it from the
    /// ordinary bus-tick pass so the walk cannot double-drive the event chain.
    /// The forced pass re-derives membership from here, so the frozen-CPU
    /// oracle still gets exactly one beat per settle tick — the same
    /// `BEATS_PER_TICK` cadence the legacy walk and `on_event` both use.
    fn needs_bus_tick_forced(&self) -> bool {
        self.any_busy()
    }

    fn tick_with_bus_forced(&mut self, bus: &mut dyn Bus) {
        self.run(bus);
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() || !self.needs_event_chain() || self.chain_live {
            return Vec::new();
        }
        self.chain_live = true;
        // delay-0 → deadline current_cycle+1 (walk's next tick).
        vec![(0, 0)]
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        bus: &mut dyn Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() {
            return crate::sched::EventResult::default();
        }
        // One beat per event cycle, matching BEATS_PER_TICK on the walk path.
        self.run(bus);
        let active = self.needs_event_chain();
        self.chain_live = active;
        crate::sched::EventResult {
            explicit_irqs: self.level_irqs().explicit_irqs.unwrap_or_default(),
            reschedule_delay: active.then_some(1),
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
            Rp2040Dma::new()
                .decode_channel(0x0c)
                .map(|(c, _, t)| (c, t)),
            Some((0, true))
        );
        assert!(matches!(
            Rp2040Dma::new().decode_channel(0x1c),
            Some((0, Reg::TransCount, true))
        ));
        assert!(matches!(
            Rp2040Dma::new().decode_channel(0x2c),
            Some((0, Reg::WriteAddr, true))
        ));
        assert!(matches!(
            Rp2040Dma::new().decode_channel(0x3c),
            Some((0, Reg::ReadAddr, true))
        ));
        // AL1_READ_ADDR (0x14) aliases the same backing READ_ADDR as 0x00.
        assert!(matches!(
            Rp2040Dma::new().decode_channel(0x14),
            Some((0, Reg::ReadAddr, false))
        ));
        // Channel 3 block.
        assert_eq!(
            Rp2040Dma::new()
                .decode_channel(ch(3) + CTRL_TRIG)
                .map(|(c, _, _)| c),
            Some(3)
        );
    }

    #[test]
    fn dma_transfer_bypasses_cpu_store_policy_and_watches() {
        let mut bus = ram_bus();
        let src = 0x2000_1000u32;
        let dst = 0x2000_2000u32;
        for i in 0..8u32 {
            bus.write_u8(src as u64 + i as u64, i as u8).unwrap();
        }
        let mut dma = Rp2040Dma::new();
        dma.write_word(READ_ADDR, src);
        dma.write_word(WRITE_ADDR, dst);
        dma.write_word(TRANS_COUNT, 8);
        dma.write_word(CTRL_TRIG, ctrl(true, 0, true, true, 0));

        bus.memory_policy.ram = crate::bus::CpuWritePolicy::Fault;
        assert!(bus.add_debug_watch(dst as u64, 8, false, true));
        assert!(bus.add_debug_watch(src as u64, 8, true, false));

        drain(&mut dma, &mut bus);

        for i in 0..8u64 {
            assert_eq!(bus.read_u8(dst as u64 + i).unwrap(), i as u8);
        }
        assert_eq!(
            bus.memory_policy.violations_ram, 0,
            "DMA stores must bypass the cpu_writes policy"
        );
        assert_eq!(
            bus.take_debug_watch_hit(),
            None,
            "neither a DMA store nor a DMA load may latch a watch"
        );

        // A direct CPU store is still gated and still latches.
        let err = bus.write_u8(dst as u64, 0xAA).unwrap_err();
        assert!(matches!(err, crate::SimulationError::MemoryViolation(_)));
        assert_eq!(bus.memory_policy.violations_ram, 1);
        assert_eq!(bus.take_debug_watch_hit(), Some(dst as u64));
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
        assert_eq!(dma.ints(0), 0b001, "INTS0 = INTR & INTE0");
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
    fn paced_treq_channel_without_a_dreq_source_does_not_run() {
        let mut bus = ram_bus();
        bus.write_u8(0x2000_1000, 0x5A).unwrap();
        let mut dma = Rp2040Dma::new();
        dma.write_word(READ_ADDR, 0x2000_1000);
        dma.write_word(WRITE_ADDR, 0x2000_2000);
        dma.write_word(TRANS_COUNT, 4);
        // TREQ_SEL = 0 (DREQ_PIO0_TX0), EN set, triggered — but both ends
        // are RAM, so nothing raises that DREQ: no beat may move.
        let paced = CTRL_EN; // treq_sel field left 0
        dma.write_word(CTRL_TRIG, paced);
        drain(&mut dma, &mut bus);
        assert!(
            !dma.any_busy(),
            "paced channel waits for a DREQ we don't source"
        );
        assert_eq!(bus.read_u8(0x2000_2000).unwrap(), 0, "no beat moved");
        assert_eq!(dma.read_word(TRANS_COUNT), 4);
    }

    /// DREQ pacing end to end on a bus: channel 0 feeds PIO0 `TXF0` (TREQ 0,
    /// `DREQ_PIO0_TX0`), the state machine echoes each word (`pull block;
    /// mov isr, osr; push block`), channel 1 drains `RXF0` (TREQ 4,
    /// `DREQ_PIO0_RX0`). Eight words through a four-deep FIFO only arrive if
    /// both channels wait on their request instead of running ahead.
    #[test]
    fn paced_channels_follow_the_pio_fifo_requests() {
        const PIO0: u64 = 0x5020_0000;
        const SRC: u32 = 0x2000_1000;
        const DST: u32 = 0x2000_2000;
        let mut bus = ram_bus();
        bus.add_peripheral(
            "pio0",
            PIO0,
            0x1000,
            None,
            Box::new(crate::peripherals::pio::Pio::new()),
        );
        let pio = bus.find_peripheral_index_by_name("pio0").unwrap();
        for (i, op) in [0x80a0u32, 0xa0c7, 0x8020].into_iter().enumerate() {
            bus.write_u32(PIO0 + 0x048 + 4 * i as u64, op).unwrap();
        }
        // EXECCTRL: wrap_top 2, wrap_bottom 0.
        bus.write_u32(PIO0 + 0x0cc, 2 << 12).unwrap();
        bus.write_u32(PIO0, 1).unwrap(); // SM0 enable
        for i in 0..8u32 {
            bus.write_u32(u64::from(SRC + 4 * i), 0xC0DE_0000 | i)
                .unwrap();
        }
        let mut dma = Rp2040Dma::new();
        let word = 2;
        // `ctrl` selects TREQ_PERMANENT; replace it with the PIO DREQ.
        let paced = |c: u32, treq: u32| {
            (c & !(TREQ_PERMANENT << CTRL_TREQ_SEL_SHIFT)) | (treq << CTRL_TREQ_SEL_SHIFT)
        };
        dma.write_word(READ_ADDR, SRC);
        dma.write_word(WRITE_ADDR, (PIO0 + 0x010) as u32);
        dma.write_word(TRANS_COUNT, 8);
        dma.write_word(CTRL_TRIG, paced(ctrl(true, word, true, false, 0), 0));
        dma.write_word(0x40 + READ_ADDR, (PIO0 + 0x020) as u32);
        dma.write_word(0x40 + WRITE_ADDR, DST);
        dma.write_word(0x40 + TRANS_COUNT, 8);
        dma.write_word(0x40 + CTRL_TRIG, paced(ctrl(true, word, false, true, 1), 4));
        for _ in 0..1_000 {
            if !dma.any_busy() {
                break;
            }
            dma.run(&mut bus);
            bus.peripherals[pio].dev.tick();
        }
        assert!(!dma.any_busy(), "both paced channels completed");
        for i in 0..8u32 {
            assert_eq!(
                bus.read_u32(u64::from(DST + 4 * i)).unwrap(),
                0xC0DE_0000 | i
            );
        }
    }

    #[test]
    fn abort_clears_busy() {
        let mut dma = Rp2040Dma::new();
        dma.write_word(READ_ADDR, 0x2000_1000);
        dma.write_word(WRITE_ADDR, 0x2000_2000);
        dma.write_word(TRANS_COUNT, 100);
        dma.write_word(CTRL_TRIG, ctrl(true, 0, true, true, 0));
        assert!(dma.any_busy());
        dma.write_word(RP2040_MAP.ctl_base + CHAN_ABORT, 1);
        assert!(!dma.any_busy(), "CHAN_ABORT halts the channel");
    }

    #[test]
    fn n_channels_reads_twelve() {
        let dma = Rp2040Dma::new();
        assert_eq!(dma.read_word(RP2040_MAP.ctl_base + N_CHANNELS), 12);
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

    /// RP2350 `CTRL_TRIG`: `INCR_WRITE` [6], `CHAIN_TO` [16:13],
    /// `TREQ_SEL` [22:17] (pico-sdk `rp2350/hardware_regs/dma.h`).
    fn ctrl_rp2350(size: u32, incr_r: bool, incr_w: bool, chain_to: usize) -> u32 {
        let mut v = CTRL_EN | (size & CTRL_DATA_SIZE_MASK) << CTRL_DATA_SIZE_SHIFT;
        if incr_r {
            v |= CTRL_INCR_READ;
        }
        if incr_w {
            v |= RP2350_MAP.incr_write;
        }
        v |= (chain_to as u32) << RP2350_MAP.chain_to_shift;
        v | TREQ_PERMANENT << RP2350_MAP.treq_sel_shift
    }

    fn rp2350() -> Rp2040Dma {
        Rp2040Dma::new().with_layout(DmaLayout::Rp2350)
    }

    #[test]
    fn rp2350_has_sixteen_channels_and_four_irq_lines() {
        let mut bus = ram_bus();
        let (src, dst) = (0x2000_1000u32, 0x2000_2000u32);
        bus.write_u32(src as u64, 0xCAFE_F00D).unwrap();
        let mut dma = rp2350();
        assert_eq!(dma.read_word(0x468), 16, "N_CHANNELS at 0x468");
        // Channel 15 is past the RP2040's twelve.
        dma.write_word(ch(15) + READ_ADDR, src);
        dma.write_word(ch(15) + WRITE_ADDR, dst);
        dma.write_word(ch(15) + TRANS_COUNT, 1);
        dma.write_word(ch(15) + CTRL_TRIG, ctrl_rp2350(2, true, true, 15));
        drain(&mut dma, &mut bus);
        assert_eq!(bus.read_u32(dst as u64).unwrap(), 0xCAFE_F00D);
        assert_eq!(dma.read_word(INTR), 1 << 15);
        // INTE3 (0x434) routes it to DMA_IRQ_3 = NVIC 13.
        dma.write_word(0x434, 1 << 15);
        assert_eq!(dma.tick().explicit_irqs, Some(vec![13]));
        // INTS3 (0x43c) write-1-clear acknowledges it.
        dma.write_word(0x43c, 1 << 15);
        assert_eq!(dma.read_word(INTR), 0);
        assert_eq!(dma.tick().explicit_irqs, None);
    }

    /// 0x430 is `INTS3` on the RP2350, not `MULTI_CHAN_TRIGGER`: an
    /// all-ones acknowledge must not start (or index past) any channel.
    #[test]
    fn rp2350_ints3_ack_is_not_a_multi_channel_trigger() {
        let mut dma = rp2350();
        for c in 0..16 {
            dma.write_word(ch(c) + TRANS_COUNT, 4);
            dma.write_word(ch(c) + 0x10, ctrl_rp2350(0, true, true, c as usize));
        }
        dma.write_word(0x43c, 0xFFFF);
        assert!(!dma.any_busy(), "INTS3 ack started a channel");
        dma.write_word(0x450, 1 << 13); // MULTI_CHAN_TRIGGER
        assert!(dma.channels[13].busy, "MULTI_CHAN_TRIGGER at 0x450");
        dma.write_word(0x464, 1 << 13); // CHAN_ABORT
        assert!(!dma.any_busy());
    }

    #[test]
    fn rp2350_busy_is_bit_26_and_irq_0_is_nvic_10() {
        let mut dma = rp2350();
        dma.write_word(READ_ADDR, 0x2000_1000);
        dma.write_word(WRITE_ADDR, 0x2000_2000);
        dma.write_word(TRANS_COUNT, 50);
        dma.write_word(CTRL_TRIG, ctrl_rp2350(0, true, true, 0));
        assert_ne!(dma.read_word(CTRL_TRIG) & (1 << 26), 0, "BUSY [26]");
        dma.intr = 1;
        dma.write_word(INTE0, 1);
        assert_eq!(dma.tick().explicit_irqs, Some(vec![10]));
    }

    #[test]
    fn rp2350_trigger_self_reruns_and_endless_never_drains() {
        let mut bus = ram_bus();
        let mut dma = rp2350();
        dma.write_word(READ_ADDR, 0x2000_1000);
        dma.write_word(WRITE_ADDR, 0x2000_2000);
        // MODE = TRIGGER_SELF, COUNT = 2: after two beats the channel
        // reloads and runs again (reads keep advancing).
        dma.write_word(TRANS_COUNT, MODE_TRIGGER_SELF << COUNT_MODE_SHIFT | 2);
        dma.write_word(CTRL_TRIG, ctrl_rp2350(0, true, false, 0));
        for _ in 0..5 {
            dma.run(&mut bus);
        }
        assert!(dma.any_busy(), "TRIGGER_SELF re-triggered");
        assert_eq!(dma.read_word(READ_ADDR), 0x2000_1005);
        assert_eq!(dma.read_word(TRANS_COUNT) & COUNT_MASK_RP2350, 1);
        dma.write_word(RP2350_MAP.ctl_base + CHAN_ABORT, 1);

        // MODE = ENDLESS: the counter holds.
        dma.write_word(TRANS_COUNT, MODE_ENDLESS << COUNT_MODE_SHIFT | 3);
        dma.write_word(CTRL_TRIG, ctrl_rp2350(0, true, false, 0));
        for _ in 0..10 {
            dma.run(&mut bus);
        }
        assert!(dma.any_busy());
        assert_eq!(dma.read_word(TRANS_COUNT) & COUNT_MASK_RP2350, 3);
    }

    #[test]
    fn retrigger_reloads_the_written_count() {
        let mut bus = ram_bus();
        let mut dma = Rp2040Dma::new();
        dma.write_word(READ_ADDR, 0x2000_1000);
        dma.write_word(WRITE_ADDR, 0x2000_2000);
        dma.write_word(TRANS_COUNT, 3);
        dma.write_word(CTRL_TRIG, ctrl(true, 0, true, true, 0));
        drain(&mut dma, &mut bus);
        // Silicon copies the reload value into the live counter on every
        // trigger, so a second trigger moves another three beats.
        dma.write_word(CTRL_TRIG, ctrl(true, 0, true, true, 0));
        assert!(dma.any_busy());
        drain(&mut dma, &mut bus);
        assert_eq!(dma.read_word(READ_ADDR), 0x2000_1006);
    }
}
