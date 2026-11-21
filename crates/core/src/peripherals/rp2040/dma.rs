// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RP2040 / RP2350 DMA transfer engine.
//!
//! The default RP2040 layout retains twelve channels and IRQs 11/12. The
//! explicit RP2350 profile has sixteen channels, IRQs 10..13, shifted CTRL
//! fields, reverse/skip increments, and its own global register offsets.
//! Source: Raspberry Pi pico-sdk ee68c78d, hardware/regs/dma.h for each chip.
//!
//! A beat uses one bus access of its configured width, then advances the live
//! addresses/count. Bus errors stop the channel and latch its native error and
//! interrupt bits. A channel completion can trigger another channel. This is
//! a functional transfer model, not DMA arbitration or bus-pipeline timing.
//!
//! RP2350 PIO requests sample the real FIFO levels through the bus.
//! Other DREQ sources, timer pacing, sniff CRC and security/MPU enforcement
//! are not implemented. A paced channel never fabricates an unpaced transfer.
//! RP2350 rejects unsupported configuration before it can report success.

use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult, SimulationError};

/// Number of DMA channels on the RP2040 (datasheet §2.5).
const NUM_CHANNELS: usize = 12;
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

// ── Interrupt / control block register offsets (datasheet §2.5.7) ──
const INTR: u64 = 0x400; // raw interrupt latch (W1C)
const INTE0: u64 = 0x404;
#[cfg(test)]
const INTF0: u64 = 0x408;
#[cfg(test)]
const INTS0: u64 = 0x40c; // (INTR&INTE0)|INTF0; W1C acks INTR
#[cfg(test)]
const INTE1: u64 = 0x414;
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
#[cfg(test)]
const CTRL_INCR_READ: u32 = 1 << 4;
#[cfg(test)]
const CTRL_INCR_WRITE: u32 = 1 << 5;
const CTRL_RING_SIZE_SHIFT: u32 = 6;
const CTRL_RING_SIZE_MASK: u32 = 0b1111;
const CTRL_CHAIN_TO_SHIFT: u32 = 11;
const CTRL_CHAIN_TO_MASK: u32 = 0b1111;
const CTRL_TREQ_SEL_SHIFT: u32 = 15;
const CTRL_TREQ_SEL_MASK: u32 = 0b11_1111;
#[cfg(test)]
const CTRL_IRQ_QUIET: u32 = 1 << 21;
#[cfg(test)]
const CTRL_BUSY: u32 = 1 << 24;

/// `TREQ_SEL` value selecting the permanent (unpaced) transfer request — the
/// memory-to-memory mode that runs a beat every cycle.
const TREQ_PERMANENT: u32 = 0x3F;

/// NVIC IRQ position of `DMA_IRQ_0` (RP2040 §2.5.7 / vector table).
const DMA_IRQ_0: u32 = 11;
/// NVIC IRQ position of `DMA_IRQ_1`.
#[cfg(test)]
const DMA_IRQ_1: u32 = 12;

/// Maximum beats moved per channel per `tick_with_bus` call. One beat/cycle is
/// the silicon rate for a permanent-TREQ channel; the sim has no wall clock so
/// the absolute cadence is arbitrary, but pacing one beat per tick keeps the
/// live `TRANS_COUNT`/address progress observable and deterministic.
const BEATS_PER_TICK: u32 = 1;

/// Native register/interrupt profile. Absence of a configured profile is RP2040.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DmaLayout {
    #[default]
    Rp2040,
    Rp2350,
}

impl DmaLayout {
    fn channels(self) -> usize {
        if self == Self::Rp2350 {
            16
        } else {
            NUM_CHANNELS
        }
    }
    fn irq_count(self) -> usize {
        if self == Self::Rp2350 {
            4
        } else {
            2
        }
    }
    fn irq_base(self) -> u32 {
        if self == Self::Rp2350 {
            10
        } else {
            DMA_IRQ_0
        }
    }
    fn shifted(self, old: u32) -> u32 {
        if self == Self::Rp2350 {
            old + 2
        } else {
            old
        }
    }
    fn busy_mask(self) -> u32 {
        1 << self.shifted(24)
    }
    fn count_mask(self) -> u32 {
        if self == Self::Rp2350 {
            0x0fff_ffff
        } else {
            u32::MAX
        }
    }
    fn global(self, old: u64) -> u64 {
        if self == Self::Rp2350 {
            old + 0x20
        } else {
            old
        }
    }
    fn channel_mask(self) -> u32 {
        (1 << self.channels()) - 1
    }
}

const CTRL_READ_ERROR: u32 = 1 << 30;
const CTRL_WRITE_ERROR: u32 = 1 << 29;
const CTRL_AHB_ERROR: u32 = 1 << 31;
const CTRL_ERRORS: u32 = CTRL_READ_ERROR | CTRL_WRITE_ERROR;

/// One DMA channel's programmable and live state.
#[derive(Debug, Clone, Copy, Default)]
struct Channel {
    read_addr: u32,
    write_addr: u32,
    trans_count: u32,
    reload_count: u32,
    ctrl: u32,
    errors: u32,
    busy: bool,
}

impl Channel {
    fn width(&self) -> u32 {
        1 << ((self.ctrl >> CTRL_DATA_SIZE_SHIFT) & CTRL_DATA_SIZE_MASK)
    }
    fn treq_sel(&self, layout: DmaLayout) -> u32 {
        (self.ctrl >> layout.shifted(CTRL_TREQ_SEL_SHIFT)) & CTRL_TREQ_SEL_MASK
    }
    fn chain_to(&self, layout: DmaLayout) -> usize {
        ((self.ctrl >> layout.shifted(CTRL_CHAIN_TO_SHIFT)) & CTRL_CHAIN_TO_MASK) as usize
    }
    fn mode(&self, layout: DmaLayout) -> u32 {
        if layout == DmaLayout::Rp2350 {
            self.reload_count >> 28
        } else {
            0
        }
    }
    fn ctrl_read(&self, layout: DmaLayout) -> u32 {
        self.ctrl
            | self.errors
            | if self.errors != 0 { CTRL_AHB_ERROR } else { 0 }
            | if self.busy { layout.busy_mask() } else { 0 }
    }
    fn advance(&self, addr: u32, is_write: bool, layout: DmaLayout) -> u32 {
        let increment_bit = if is_write {
            if layout == DmaLayout::Rp2350 {
                6
            } else {
                5
            }
        } else {
            4
        };
        let increment = self.ctrl & (1 << increment_bit) != 0;
        let reverse =
            layout == DmaLayout::Rp2350 && self.ctrl & (1 << if is_write { 7 } else { 5 }) != 0;
        let next = match (increment, reverse) {
            (false, false) => return addr,
            (true, false) => addr.wrapping_add(self.width()),
            (true, true) => addr.wrapping_sub(self.width()),
            (false, true) => addr.wrapping_add(2 * self.width()),
        };
        let ring_size = (self.ctrl >> layout.shifted(CTRL_RING_SIZE_SHIFT)) & CTRL_RING_SIZE_MASK;
        let ring_on_write = self.ctrl & (1 << layout.shifted(10)) != 0;
        if ring_size != 0 && is_write == ring_on_write {
            let mask = (1u32 << ring_size) - 1;
            (addr & !mask) | (next & mask)
        } else {
            next
        }
    }
}

#[derive(Debug)]
pub struct Rp2040Dma {
    layout: DmaLayout,
    pio_dreq_bases: Option<[u64; 3]>,
    channels: Vec<Channel>,
    intr: u32,
    inte: [u32; 4],
    intf: [u32; 4],
    sniff_ctrl: u32,
    sniff_data: u32,
    clock: Option<CycleClock>,
    chain_live: bool,
}

impl Default for Rp2040Dma {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040Dma {
    pub fn new() -> Self {
        Self::with_layout(DmaLayout::Rp2040)
    }
    pub fn with_layout(layout: DmaLayout) -> Self {
        Self {
            layout,
            pio_dreq_bases: None,
            channels: vec![Channel::default(); layout.channels()],
            intr: 0,
            inte: [0; 4],
            intf: [0; 4],
            sniff_ctrl: 0,
            sniff_data: 0,
            clock: None,
            chain_live: false,
        }
    }
    /// The descriptor owns the three PIO MMIO windows. No silicon address
    /// is duplicated in this engine; relocation changes routing accordingly.
    pub fn with_pio_dreq_bases(mut self, bases: [u64; 3]) -> Self {
        self.pio_dreq_bases = Some(bases);
        self
    }
    // DREQ numbering is the native silicon contract, pico-sdk ee68c78d dreq.h.
    fn dreq_route(&self, request: u32) -> Option<(u64, u8)> {
        if self.layout != DmaLayout::Rp2350 || request > 23 {
            return None;
        }
        self.pio_dreq_bases
            .map(|bases| (bases[(request / 8) as usize], (request % 8) as u8))
    }
    crate::cycle_clock::scheduler_mode!();

    fn ints(&self, line: usize) -> u32 {
        (self.intr & self.inte[line]) | self.intf[line]
    }
    #[cfg(test)]
    fn ints0(&self) -> u32 {
        self.ints(0)
    }
    fn needs_event_chain(&self) -> bool {
        self.any_busy() || (0..self.layout.irq_count()).any(|i| self.ints(i) != 0)
    }
    fn level_irqs(&self) -> PeripheralTickResult {
        let irqs: Vec<u32> = (0..self.layout.irq_count())
            .filter(|&i| self.ints(i) != 0)
            .map(|i| self.layout.irq_base() + i as u32)
            .collect();
        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }

    fn decode_channel(&self, offset: u64) -> Option<(usize, Reg, bool)> {
        if offset >= CHANNEL_STRIDE * self.channels.len() as u64 {
            return None;
        }
        let ch = (offset / CHANNEL_STRIDE) as usize;
        let local = offset % CHANNEL_STRIDE;
        let alias = local / 0x10;
        let slot = (local % 0x10) / 4;
        let reg = match (alias, slot) {
            (0, 0) | (1, 1) | (2, 2) | (3, 3) => Reg::ReadAddr,
            (0, 1) | (1, 2) | (2, 3) | (3, 1) => Reg::WriteAddr,
            (0, 2) | (1, 3) | (2, 1) | (3, 2) => Reg::TransCount,
            _ => Reg::Ctrl,
        };
        Some((ch, reg, slot == 3))
    }
    fn irq_register(&self, offset: u64) -> Option<(usize, u64)> {
        if offset < INTE0 {
            return None;
        }
        let line = ((offset - INTE0) / 0x10) as usize;
        let reg = (offset - INTE0) % 0x10;
        (line < self.layout.irq_count() && reg <= 8).then_some((line, reg))
    }
    fn debug_register(&self, offset: u64) -> Option<(usize, u64)> {
        if offset < 0x800 {
            return None;
        }
        let ch = ((offset - 0x800) / 0x40) as usize;
        let reg = (offset - 0x800) % 0x40;
        (ch < self.channels.len() && reg <= 4).then_some((ch, reg))
    }
    fn read_word(&self, offset: u64) -> u32 {
        if let Some((ch, reg, _)) = self.decode_channel(offset) {
            let c = &self.channels[ch];
            return match reg {
                Reg::ReadAddr => c.read_addr,
                Reg::WriteAddr => c.write_addr,
                Reg::TransCount => c.trans_count,
                Reg::Ctrl => c.ctrl_read(self.layout),
            };
        }
        if let Some((line, reg)) = self.irq_register(offset) {
            return match reg {
                0 => self.inte[line],
                4 => self.intf[line],
                _ => self.ints(line),
            };
        }
        if let Some((ch, reg)) = self.debug_register(offset) {
            return if reg == 4 {
                self.channels[ch].reload_count
            } else {
                0
            };
        }
        match offset {
            INTR => self.intr,
            o if o == self.layout.global(SNIFF_CTRL) => self.sniff_ctrl,
            o if o == self.layout.global(SNIFF_DATA) => self.sniff_data,
            o if o == self.layout.global(N_CHANNELS) => self.channels.len() as u32,
            o if o == self.layout.global(FIFO_LEVELS) => 0,
            0x480..=0x4cc if self.layout == DmaLayout::Rp2350 => 3,
            0x4d0 if self.layout == DmaLayout::Rp2350 => 0x3ff,
            _ => 0,
        }
    }
    fn validate_word(&self, offset: u64, value: Option<u32>) -> SimResult<()> {
        if offset & 3 != 0 {
            return Err(SimulationError::MemoryViolation(offset));
        }
        if let Some((_, reg, _)) = self.decode_channel(offset) {
            if let Some(value) = value {
                if reg == Reg::Ctrl && (value >> CTRL_DATA_SIZE_SHIFT) & 3 == 3 {
                    return Err(SimulationError::NotImplemented(
                        "RP DMA reserved DATA_SIZE=3".into(),
                    ));
                }
                if self.layout == DmaLayout::Rp2350
                    && reg == Reg::TransCount
                    && !matches!(value >> 28, 0 | 1 | 15)
                {
                    return Err(SimulationError::NotImplemented(
                        "RP2350 reserved DMA transfer-count mode".into(),
                    ));
                }
                let request =
                    (value >> self.layout.shifted(CTRL_TREQ_SEL_SHIFT)) & CTRL_TREQ_SEL_MASK;
                if self.layout == DmaLayout::Rp2350
                    && reg == Reg::Ctrl
                    && value & CTRL_EN != 0
                    && request != TREQ_PERMANENT
                    && self.dreq_route(request).is_none()
                {
                    return Err(SimulationError::NotImplemented(
                        "RP2350 DMA request source is not modeled".into(),
                    ));
                }
                if self.layout == DmaLayout::Rp2350 && reg == Reg::Ctrl && value & (1 << 25) != 0 {
                    return Err(SimulationError::NotImplemented(
                        "RP2350 DMA sniff computation is not modeled".into(),
                    ));
                }
            }
            return Ok(());
        }
        if self.irq_register(offset).is_some() || self.debug_register(offset).is_some() {
            return Ok(());
        }
        if self.layout == DmaLayout::Rp2040 {
            return Ok(());
        }
        let known = matches!(offset, INTR | 0x440..=0x458 | 0x460..=0x468 | 0x480..=0x4d0 | 0x500);
        if !known {
            return Err(SimulationError::MemoryViolation(offset));
        }
        if let Some(value) = value {
            let unsupported = match offset {
                0x440..=0x44c => value != 0,
                0x454 => value & 1 != 0,
                0x480..=0x4cc => value != 3,
                0x4d0 => value != 0x3ff,
                0x500 => value != 0,
                _ => false,
            };
            if unsupported {
                return Err(SimulationError::NotImplemented(
                    "RP2350 DMA timer pacing, sniff CRC and security/MPU changes are not modeled"
                        .into(),
                ));
            }
        }
        Ok(())
    }
    fn write_word(&mut self, offset: u64, value: u32) {
        if let Some((ch, reg, trig)) = self.decode_channel(offset) {
            let c = &mut self.channels[ch];
            match reg {
                Reg::ReadAddr => c.read_addr = value,
                Reg::WriteAddr => c.write_addr = value,
                Reg::TransCount => {
                    c.reload_count = value;
                    if !c.busy {
                        c.trans_count = value;
                    }
                }
                Reg::Ctrl => {
                    c.errors &= !(value & CTRL_ERRORS);
                    let mask = if self.layout == DmaLayout::Rp2350 {
                        0x03ff_ffff
                    } else {
                        0x00ff_ffff
                    };
                    c.ctrl = value & mask;
                }
            }
            if trig && value != 0 {
                self.try_start(ch);
            }
            // IRQ_QUIET null triggers mark the end of a control-block chain.
            if trig && value == 0 && c_quiet(self.channels[ch].ctrl, self.layout) {
                self.intr |= 1 << ch;
            }
            return;
        }
        if let Some((line, reg)) = self.irq_register(offset) {
            let value = value & self.layout.channel_mask();
            match reg {
                0 => self.inte[line] = value,
                4 => self.intf[line] = value,
                _ => self.intr &= !value,
            }
            return;
        }
        match offset {
            INTR => self.intr &= !value,
            o if o == self.layout.global(MULTI_CHAN_TRIGGER) => {
                for ch in 0..self.channels.len() {
                    if value & (1 << ch) != 0 {
                        self.try_start(ch);
                    }
                }
            }
            o if o == self.layout.global(CHAN_ABORT) => {
                for ch in 0..self.channels.len() {
                    if value & (1 << ch) != 0 {
                        self.channels[ch].busy = false;
                    }
                }
            }
            o if o == self.layout.global(SNIFF_CTRL) => self.sniff_ctrl = value,
            o if o == self.layout.global(SNIFF_DATA) => self.sniff_data = value,
            _ => {}
        }
    }
    fn try_start(&mut self, ch: usize) {
        let paced = self
            .channels
            .get(ch)
            .is_some_and(|c| self.dreq_route(c.treq_sel(self.layout)).is_some());
        let Some(c) = self.channels.get_mut(ch) else {
            return;
        };
        if !c.busy
            && c.ctrl & CTRL_EN != 0
            && c.reload_count & self.layout.count_mask() > 0
            && (c.treq_sel(self.layout) == TREQ_PERMANENT || paced)
        {
            c.trans_count = c.reload_count;
            c.busy = true;
        }
    }
    fn any_busy(&self) -> bool {
        self.channels.iter().any(|c| c.busy)
    }
    fn run(&mut self, bus: &mut dyn Bus) {
        for ch in 0..self.channels.len() {
            for _ in 0..BEATS_PER_TICK {
                if !self.channels[ch].busy || self.channels[ch].ctrl & CTRL_EN == 0 {
                    break;
                }
                let request = self.channels[ch].treq_sel(self.layout);
                if request != TREQ_PERMANENT {
                    let ready = self
                        .dreq_route(request)
                        .and_then(|(addr, line)| bus.dma_request_active_at(addr, line));
                    if ready != Some(true) {
                        break;
                    }
                }
                self.beat(bus, ch);
            }
        }
    }
    fn bus_error(&mut self, ch: usize, read: bool) {
        self.channels[ch].busy = false;
        self.channels[ch].errors |= if read {
            CTRL_READ_ERROR
        } else {
            CTRL_WRITE_ERROR
        };
        self.intr |= 1 << ch;
    }
    fn beat(&mut self, bus: &mut dyn Bus, ch: usize) {
        let c = self.channels[ch];
        let width = c.width();
        let read = crate::with_device_write_bypass(bus, |bus| match width {
            1 => bus.read_u8(c.read_addr as u64).map(u32::from),
            2 => bus.read_u16(c.read_addr as u64).map(u32::from),
            _ => bus.read_u32(c.read_addr as u64),
        });
        let Ok(mut value) = read else {
            self.bus_error(ch, true);
            return;
        };
        if c.ctrl & (1 << self.layout.shifted(22)) != 0 {
            value = match width {
                2 => u32::from((value as u16).swap_bytes()),
                4 => value.swap_bytes(),
                _ => value,
            };
        }
        let write = crate::with_device_write_bypass(bus, |bus| match width {
            1 => bus.write_u8(c.write_addr as u64, value as u8),
            2 => bus.write_u16(c.write_addr as u64, value as u16),
            _ => bus.write_u32(c.write_addr as u64, value),
        });
        if write.is_err() {
            self.bus_error(ch, false);
            return;
        }
        let active = &mut self.channels[ch];
        active.read_addr = c.advance(c.read_addr, false, self.layout);
        active.write_addr = c.advance(c.write_addr, true, self.layout);
        if c.mode(self.layout) == 15 {
            return;
        }
        active.trans_count -= 1;
        if active.trans_count & self.layout.count_mask() == 0 {
            active.busy = false;
            if !c_quiet(c.ctrl, self.layout) {
                self.intr |= 1 << ch;
            }
            let chain = c.chain_to(self.layout);
            if chain != ch {
                self.try_start(chain);
            }
            if c.mode(self.layout) == 1 {
                self.try_start(ch);
            }
        }
    }
}

fn c_quiet(ctrl: u32, layout: DmaLayout) -> bool {
    ctrl & (1 << layout.shifted(21)) != 0
}

impl Peripheral for Rp2040Dma {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        self.validate_word(offset, None)?;
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.validate_word(offset, Some(value))?;
        self.write_word(offset, value);
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        self.validate_word(offset & !3, None)?;
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
        self.validate_word(aligned, Some(new))?;
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

    fn on_system_reset(&mut self) {
        let clock = self.clock.clone();
        let pio_dreq_bases = self.pio_dreq_bases;
        *self = Self::with_layout(self.layout);
        self.clock = clock;
        self.pio_dreq_bases = pio_dreq_bases;
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
    fn force_interrupt_is_independent_of_enable_mask() {
        let mut dma = Rp2040Dma::new();
        dma.write_u32(INTF0, 1 << 3).unwrap();
        assert_eq!(dma.read_u32(INTE0).unwrap(), 0);
        assert_eq!(dma.read_u32(INTS0).unwrap(), 1 << 3);
        assert_eq!(
            dma.tick_elapsed_forced(1).explicit_irqs,
            Some(vec![DMA_IRQ_0])
        );
        dma.write_u32(INTF0, 0).unwrap();
        assert!(dma.tick_elapsed_forced(1).explicit_irqs.is_none());
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
