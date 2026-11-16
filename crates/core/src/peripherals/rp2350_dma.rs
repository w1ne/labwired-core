// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2350 DMA controller (datasheet Release 8, section 12.6; register map
//! pico-sdk rp2350 `hardware/regs/dma.h`), base 0x50000000.
//!
//! Sixteen channels with the RP2350 `CTRL` layout: `EN` [0],
//! `HIGH_PRIORITY` [1], `DATA_SIZE` [3:2], `INCR_READ` [4],
//! `INCR_READ_REV` [5], `INCR_WRITE` [6], `INCR_WRITE_REV` [7],
//! `RING_SIZE` [11:8], `RING_SEL` [12], `CHAIN_TO` [16:13], `TREQ_SEL`
//! [22:17], `IRQ_QUIET` [23], `BSWAP` [24], `SNIFF_EN` [25], `BUSY` [26],
//! `WRITE_ERROR` [29], `READ_ERROR` [30], `AHB_ERROR` [31]. Four alias
//! windows per channel as on the RP2040. `TRANS_COUNT` holds the reload
//! value (copied into the live counter on every trigger) and `MODE` [31:28]:
//! NORMAL, TRIGGER_SELF (re-trigger on completion) or ENDLESS (never
//! decrements); other modes are reserved and refused.
//!
//! Interrupts: `INTR` [0x400] is the raw per-channel latch; four sets
//! `INTEn`/`INTFn`/`INTSn` at `0x404 + 0x10*n` drive `DMA_IRQ_n` (NVIC
//! 10..13 on the RP2350; the descriptor `irq:` names line 0), with
//! `INTSn = (INTR & INTEn) | INTFn` ("write 1s to force the corresponding
//! bits in INTSn"). Writing `INTR` or any `INTSn` clears raw bits.
//!
//! Pacing: `TREQ_SEL = 0x3F` runs one beat per tick. Any other DREQ must be
//! declared in the descriptor (`config.dreq: { <n>: { peripheral, dir } }`);
//! a TX DREQ throttles on the named peripheral's `dma_tx_request` at the
//! channel's write address, an RX DREQ on `dma_rx_request` at its read
//! address — the generic bus seam, so any peripheral that reports FIFO
//! readiness paces DMA. The endpoint address must lie inside the named
//! peripheral. Undeclared DREQs, the pacing timers (DREQ 59..62), `SNIFF_EN`,
//! a DREQ endpoint that does not report readiness and a wrong endpoint are
//! refused (`NotImplemented`, on the triggering write or on the next register
//! access after the transfer pass that found it).
//!
//! Not modelled: bus priority/arbitration timing (one beat per busy channel
//! per tick, high-priority channels first), the sniffer CRC, SECCFG/MPU
//! enforcement (stored, not applied), `FIFO_LEVELS` (reads 0).

use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::collections::BTreeMap;

const CHANNELS: usize = 16;
const STRIDE: u64 = 0x40;

const CTRL_EN: u32 = 1 << 0;
const CTRL_HIGH_PRIORITY: u32 = 1 << 1;
const CTRL_INCR_READ_REV: u32 = 1 << 5;
const CTRL_INCR_WRITE_REV: u32 = 1 << 7;
const CTRL_RING_SEL: u32 = 1 << 12;
const CTRL_IRQ_QUIET: u32 = 1 << 23;
const CTRL_BSWAP: u32 = 1 << 24;
const CTRL_SNIFF_EN: u32 = 1 << 25;
const CTRL_BUSY: u32 = 1 << 26;
const CTRL_WRITE_ERROR: u32 = 1 << 29;
const CTRL_READ_ERROR: u32 = 1 << 30;
const CTRL_AHB_ERROR: u32 = 1 << 31;
/// Bits firmware stores (everything but BUSY and the error flags).
const CTRL_STORED: u32 = 0x03FF_FFFF & !CTRL_BUSY;

const TREQ_PERMANENT: u32 = 0x3F;
const COUNT_MASK: u32 = 0x0FFF_FFFF;
const MODE_NORMAL: u32 = 0x0;
const MODE_TRIGGER_SELF: u32 = 0x1;
const MODE_ENDLESS: u32 = 0xF;

const INTR: u64 = 0x400;
const IRQ_SETS_END: u64 = 0x440;
const TIMER0: u64 = 0x440;
const TIMER3: u64 = 0x44C;
const MULTI_CHAN_TRIGGER: u64 = 0x450;
const SNIFF_CTRL: u64 = 0x454;
const SNIFF_DATA: u64 = 0x458;
const FIFO_LEVELS: u64 = 0x460;
const CHAN_ABORT: u64 = 0x464;
const N_CHANNELS: u64 = 0x468;
const SECCFG_START: u64 = 0x480;
const MPU_END: u64 = 0x544;
const DBG_START: u64 = 0x800;
const DBG_END: u64 = DBG_START + STRIDE * CHANNELS as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DreqDir {
    /// Throttle on the endpoint accepting a write (TX FIFO not full).
    Tx,
    /// Throttle on the endpoint holding data (RX FIFO not empty).
    Rx,
}

/// One declared system DREQ: which peripheral raises it and in which
/// direction.
#[derive(Debug, Clone)]
pub struct Dreq {
    pub peripheral: String,
    pub dir: DreqDir,
}

#[derive(Debug, Clone, Copy, Default)]
struct Channel {
    read: u32,
    write: u32,
    /// TRANS_COUNT as written: MODE + reload count.
    reload: u32,
    /// Live remaining count.
    count: u32,
    ctrl: u32,
    errors: u32,
    busy: bool,
}

impl Channel {
    fn width(&self) -> u32 {
        1 << ((self.ctrl >> 2) & 3)
    }
    fn treq(&self) -> u32 {
        (self.ctrl >> 17) & 0x3F
    }
    fn chain_to(&self) -> usize {
        ((self.ctrl >> 13) & 0xF) as usize
    }
    fn mode(&self) -> u32 {
        self.reload >> 28
    }
    fn ctrl_read(&self) -> u32 {
        let mut v = self.ctrl | self.errors;
        if self.errors != 0 {
            v |= CTRL_AHB_ERROR;
        }
        if self.busy {
            v |= CTRL_BUSY;
        }
        v
    }
    /// Next address for one direction: INCR (+width), INCR+REV (-width) or
    /// fixed; RING wraps the low RING_SIZE bits of the selected direction.
    fn advance(&self, addr: u32, write: bool) -> u32 {
        let (incr, rev) = if write {
            (
                self.ctrl & (1 << 6) != 0,
                self.ctrl & CTRL_INCR_WRITE_REV != 0,
            )
        } else {
            (
                self.ctrl & (1 << 4) != 0,
                self.ctrl & CTRL_INCR_READ_REV != 0,
            )
        };
        if !incr {
            return addr;
        }
        let w = self.width();
        let next = if rev {
            addr.wrapping_sub(w)
        } else {
            addr.wrapping_add(w)
        };
        let ring = (self.ctrl >> 8) & 0xF;
        if ring != 0 && (self.ctrl & CTRL_RING_SEL != 0) == write {
            let mask = (1u32 << ring) - 1;
            (addr & !mask) | (next & mask)
        } else {
            next
        }
    }
}

#[derive(Debug)]
pub struct Rp2350Dma {
    channels: [Channel; CHANNELS],
    intr: u32,
    inte: [u32; 4],
    intf: [u32; 4],
    timers: [u32; 4],
    sniff_ctrl: u32,
    sniff_data: u32,
    seccfg_mpu: BTreeMap<u64, u32>,
    /// NVIC line of DMA_IRQ_0; DMA_IRQ_n is `irq0 + n`.
    irq0: u32,
    dreqs: BTreeMap<u32, Dreq>,
    /// A refusal found during a transfer pass, reported on the next access.
    failure: Option<String>,
    clock: Option<CycleClock>,
    chain_live: bool,
    /// Round-robin cursor over normal-priority channels.
    cursor: usize,
}

impl Rp2350Dma {
    pub fn new(irq0: u32, dreqs: BTreeMap<u32, Dreq>) -> Self {
        Self {
            channels: [Channel::default(); CHANNELS],
            intr: 0,
            inte: [0; 4],
            intf: [0; 4],
            timers: [0; 4],
            sniff_ctrl: 0,
            sniff_data: 0,
            seccfg_mpu: BTreeMap::new(),
            irq0,
            dreqs,
            failure: None,
            clock: None,
            chain_live: false,
            cursor: 0,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn refuse(what: impl std::fmt::Display) -> SimulationError {
        SimulationError::NotImplemented(format!("RP2350 DMA: {what}"))
    }

    fn check_failure(&self) -> SimResult<()> {
        match &self.failure {
            Some(f) => Err(Self::refuse(f)),
            None => Ok(()),
        }
    }

    fn ints(&self, n: usize) -> u32 {
        (self.intr & self.inte[n]) | self.intf[n]
    }

    fn level_irqs(&self) -> Vec<u32> {
        (0..4)
            .filter(|&n| self.ints(n) != 0)
            .map(|n| self.irq0 + n as u32)
            .collect()
    }

    fn any_busy(&self) -> bool {
        self.failure.is_none() && self.channels.iter().any(|c| c.busy)
    }

    fn needs_event_chain(&self) -> bool {
        self.any_busy() || !self.level_irqs().is_empty()
    }

    fn decode(offset: u64) -> Option<(usize, u8, bool)> {
        if offset >= STRIDE * CHANNELS as u64 || offset & 3 != 0 {
            return None;
        }
        // 0 READ_ADDR, 1 WRITE_ADDR, 2 TRANS_COUNT, 3 CTRL; +0xC triggers.
        let reg = match offset % STRIDE {
            0x00 | 0x14 | 0x28 | 0x3C => 0,
            0x04 | 0x18 | 0x2C | 0x34 => 1,
            0x08 | 0x1C | 0x24 | 0x38 => 2,
            _ => 3,
        };
        Some(((offset / STRIDE) as usize, reg, offset % 0x10 == 0xC))
    }

    /// Start channel `n` if enabled: load the count from the reload value.
    fn trigger(&mut self, n: usize) -> SimResult<()> {
        let c = self.channels[n];
        if c.ctrl & CTRL_EN == 0 {
            return Ok(());
        }
        let treq = c.treq();
        if c.ctrl & CTRL_SNIFF_EN != 0 {
            return Err(Self::refuse(format!(
                "channel {n}: SNIFF_EN (CRC sniffer) is not modelled"
            )));
        }
        if (59..=62).contains(&treq) {
            return Err(Self::refuse(format!(
                "channel {n}: pacing timer TREQ {treq} is not modelled"
            )));
        }
        if treq != TREQ_PERMANENT && !self.dreqs.contains_key(&treq) {
            return Err(Self::refuse(format!(
                "channel {n}: DREQ {treq} is not declared for this chip"
            )));
        }
        let ch = &mut self.channels[n];
        ch.count = ch.reload & COUNT_MASK;
        ch.busy = ch.count != 0 || ch.mode() == MODE_ENDLESS;
        Ok(())
    }

    /// Whether channel `n`'s DREQ admits a beat now.
    fn ready(&mut self, bus: &dyn Bus, n: usize) -> bool {
        let c = self.channels[n];
        let treq = c.treq();
        if treq == TREQ_PERMANENT {
            return true;
        }
        let Some(dreq) = self.dreqs.get(&treq) else {
            self.failure = Some(format!("channel {n}: DREQ {treq} not declared"));
            return false;
        };
        let addr = u64::from(match dreq.dir {
            DreqDir::Tx => c.write,
            DreqDir::Rx => c.read,
        });
        let owner = bus.peripheral_id_at(addr);
        if owner != Some(dreq.peripheral.as_str()) {
            self.failure =
                Some(format!(
                "channel {n}: DREQ {treq} belongs to `{}` but the {} address {addr:#010x} is {}",
                dreq.peripheral,
                if dreq.dir == DreqDir::Tx { "write" } else { "read" },
                owner.map_or("unmapped".to_string(), |o| format!("`{o}`"))
            ));
            return false;
        }
        let r = match dreq.dir {
            DreqDir::Tx => bus.dma_tx_request(addr),
            DreqDir::Rx => bus.dma_rx_request(addr),
        };
        match r {
            Some(ready) => ready,
            None => {
                self.failure = Some(format!(
                    "channel {n}: `{}` does not report DMA readiness at {addr:#010x}",
                    dreq.peripheral
                ));
                false
            }
        }
    }

    fn beat(&mut self, bus: &mut dyn Bus, n: usize) {
        if !self.channels[n].busy || !self.ready(bus, n) {
            return;
        }
        let c = self.channels[n];
        let width = c.width();
        let value = crate::with_device_write_bypass(bus, |bus| match width {
            1 => bus.read_u8(u64::from(c.read)).map(u32::from),
            2 => bus.read_u16(u64::from(c.read)).map(u32::from),
            _ => bus.read_u32(u64::from(c.read)),
        });
        let Ok(mut value) = value else {
            self.bus_error(n, CTRL_READ_ERROR);
            return;
        };
        if c.ctrl & CTRL_BSWAP != 0 {
            value = match width {
                2 => u32::from((value as u16).swap_bytes()),
                4 => value.swap_bytes(),
                _ => value,
            };
        }
        let written = crate::with_device_write_bypass(bus, |bus| match width {
            1 => bus.write_u8(u64::from(c.write), value as u8),
            2 => bus.write_u16(u64::from(c.write), value as u16),
            _ => bus.write_u32(u64::from(c.write), value),
        });
        if written.is_err() {
            self.bus_error(n, CTRL_WRITE_ERROR);
            return;
        }
        let ch = &mut self.channels[n];
        ch.read = c.advance(c.read, false);
        ch.write = c.advance(c.write, true);
        if c.mode() == MODE_ENDLESS {
            return;
        }
        ch.count -= 1;
        if ch.count == 0 {
            ch.busy = false;
            if c.ctrl & CTRL_IRQ_QUIET == 0 {
                self.intr |= 1 << n;
            }
            let chain = c.chain_to();
            if chain != n {
                if let Err(e) = self.trigger(chain) {
                    self.failure = Some(e.to_string());
                }
            }
            if c.mode() == MODE_TRIGGER_SELF {
                if let Err(e) = self.trigger(n) {
                    self.failure = Some(e.to_string());
                }
            }
        }
    }

    fn bus_error(&mut self, n: usize, which: u32) {
        let ch = &mut self.channels[n];
        ch.errors |= which;
        ch.busy = false;
        self.intr |= 1 << n;
    }

    /// One transfer round: every busy high-priority channel, then one
    /// normal-priority channel in round-robin order (datasheet 12.6.2.3:
    /// high-priority channels are served first in each round).
    fn run(&mut self, bus: &mut dyn Bus) {
        if self.failure.is_some() {
            return;
        }
        for n in 0..CHANNELS {
            if self.channels[n].busy && self.channels[n].ctrl & CTRL_HIGH_PRIORITY != 0 {
                self.beat(bus, n);
            }
        }
        for step in 0..CHANNELS {
            let n = (self.cursor + step) % CHANNELS;
            if self.channels[n].busy && self.channels[n].ctrl & CTRL_HIGH_PRIORITY == 0 {
                self.beat(bus, n);
            }
        }
        self.cursor = (self.cursor + 1) % CHANNELS;
    }

    fn read_word(&self, offset: u64) -> SimResult<u32> {
        self.check_failure()?;
        if let Some((n, reg, _)) = Self::decode(offset) {
            let c = &self.channels[n];
            return Ok(match reg {
                0 => c.read,
                1 => c.write,
                // Reads return the live count with the MODE bits.
                2 => {
                    (c.reload & !COUNT_MASK)
                        | if c.busy {
                            c.count
                        } else {
                            c.count & COUNT_MASK
                        }
                }
                _ => c.ctrl_read(),
            });
        }
        Ok(match offset {
            INTR => self.intr,
            o if (INTR + 4..IRQ_SETS_END).contains(&o) && o & 3 == 0 => {
                let set = ((o - INTR) / 0x10) as usize;
                match o & 0xF {
                    0x0 => self.intr,
                    0x4 => self.inte[set],
                    0x8 => self.intf[set],
                    _ => self.ints(set),
                }
            }
            o @ TIMER0..=TIMER3 => self.timers[((o - TIMER0) / 4) as usize],
            MULTI_CHAN_TRIGGER => 0,
            SNIFF_CTRL => self.sniff_ctrl,
            SNIFF_DATA => self.sniff_data,
            FIFO_LEVELS | CHAN_ABORT => 0,
            N_CHANNELS => CHANNELS as u32,
            o if (SECCFG_START..MPU_END).contains(&o) => {
                self.seccfg_mpu.get(&o).copied().unwrap_or(0)
            }
            o if (DBG_START..DBG_END).contains(&o) => {
                let c = &self.channels[((o - DBG_START) / STRIDE) as usize];
                match (o - DBG_START) % STRIDE {
                    0x0 => 0,                     // CTDREQ: no DREQ counter is modelled
                    0x4 => c.reload & COUNT_MASK, // TCR: next reload count
                    _ => 0,
                }
            }
            other => return Err(Self::refuse(format!("read of unmapped offset {other:#x}"))),
        })
    }

    fn write_word(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.check_failure()?;
        if let Some((n, reg, trig)) = Self::decode(offset) {
            let c = &mut self.channels[n];
            match reg {
                0 => c.read = value,
                1 => c.write = value,
                2 => {
                    let mode = value >> 28;
                    if !matches!(mode, MODE_NORMAL | MODE_TRIGGER_SELF | MODE_ENDLESS) {
                        return Err(Self::refuse(format!(
                            "channel {n}: TRANS_COUNT.MODE {mode:#x} is reserved"
                        )));
                    }
                    c.reload = value;
                }
                _ => {
                    // Writing 1 to READ_ERROR/WRITE_ERROR clears them.
                    c.errors &= !(value & (CTRL_READ_ERROR | CTRL_WRITE_ERROR));
                    c.ctrl = value & CTRL_STORED;
                    if c.ctrl & CTRL_EN == 0 {
                        // A channel disabled mid-transfer pauses (12.6.2).
                    }
                }
            }
            if trig {
                self.trigger(n)?;
            }
            return Ok(());
        }
        match offset {
            INTR => self.intr &= !value,
            o if (INTR + 4..IRQ_SETS_END).contains(&o) && o & 3 == 0 => {
                let set = ((o - INTR) / 0x10) as usize;
                match o & 0xF {
                    0x0 => self.intr &= !value,
                    0x4 => self.inte[set] = value & 0xFFFF,
                    0x8 => self.intf[set] = value & 0xFFFF,
                    _ => self.intr &= !value,
                }
            }
            o @ TIMER0..=TIMER3 => self.timers[((o - TIMER0) / 4) as usize] = value,
            MULTI_CHAN_TRIGGER => {
                for n in 0..CHANNELS {
                    if value & (1 << n) != 0 {
                        self.trigger(n)?;
                    }
                }
            }
            SNIFF_CTRL => self.sniff_ctrl = value,
            SNIFF_DATA => self.sniff_data = value,
            CHAN_ABORT => {
                for n in 0..CHANNELS {
                    if value & (1 << n) != 0 {
                        self.channels[n].busy = false;
                    }
                }
            }
            FIFO_LEVELS | N_CHANNELS => {}
            o if (SECCFG_START..MPU_END).contains(&o) => {
                self.seccfg_mpu.insert(o, value);
            }
            o if (DBG_START..DBG_END).contains(&o) => {}
            other => return Err(Self::refuse(format!("write of unmapped offset {other:#x}"))),
        }
        Ok(())
    }
}

impl Peripheral for Rp2350Dma {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        self.read_word(offset)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value)
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset & !3)? >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let cur = self.read_word(aligned)?;
        self.write_word(
            aligned,
            (cur & !(0xFF << shift)) | (u32::from(value) << shift),
        )
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn needs_bus_tick(&self) -> bool {
        !self.scheduler_mode() && self.any_busy()
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if !self.scheduler_mode() {
            self.run(bus);
        }
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        let irqs = self.level_irqs();
        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }

    fn tick_elapsed_forced(&mut self, _cycles: u64) -> PeripheralTickResult {
        let irqs = self.level_irqs();
        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }

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
        self.run(bus);
        let active = self.needs_event_chain();
        self.chain_live = active;
        crate::sched::EventResult {
            explicit_irqs: self.level_irqs(),
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

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "busy": self.channels.iter().enumerate().filter(|(_, c)| c.busy).map(|(n, _)| n).collect::<Vec<_>>(),
            "intr": self.intr,
            "inte": self.inte,
            "failure": self.failure,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;

    const CTRL_TRIG: u64 = 0x0C;
    const TRANS_COUNT: u64 = 0x08;

    fn ctrl(size: u32, incr_r: bool, incr_w: bool, chain: usize, treq: u32) -> u32 {
        CTRL_EN
            | (size << 2)
            | if incr_r { 1 << 4 } else { 0 }
            | if incr_w { 1 << 6 } else { 0 }
            | ((chain as u32) << 13)
            | (treq << 17)
    }

    fn drain(dma: &mut Rp2350Dma, bus: &mut dyn Bus) {
        for _ in 0..10_000 {
            if !dma.any_busy() {
                return;
            }
            dma.run(bus);
        }
        panic!("transfer did not complete");
    }

    #[test]
    fn rp2350_ctrl_layout_copies_and_reloads_on_retrigger() {
        let mut bus = SystemBus::new();
        for i in 0..8u64 {
            bus.write_u8(0x2000_0000 + i, i as u8 + 1).unwrap();
        }
        let mut dma = Rp2350Dma::new(10, BTreeMap::new());
        dma.write_word(0x00, 0x2000_0000).unwrap();
        dma.write_word(0x04, 0x2000_0100).unwrap();
        dma.write_word(TRANS_COUNT, 8).unwrap();
        dma.write_word(CTRL_TRIG, ctrl(0, true, true, 0, TREQ_PERMANENT))
            .unwrap();
        assert_ne!(
            dma.read_word(CTRL_TRIG).unwrap() & CTRL_BUSY,
            0,
            "BUSY is bit 26"
        );
        drain(&mut dma, &mut bus);
        for i in 0..8u64 {
            assert_eq!(bus.read_u8(0x2000_0100 + i).unwrap(), i as u8 + 1);
        }
        assert_eq!(dma.intr, 1);
        // A re-trigger reloads the count from TRANS_COUNT.
        dma.write_word(0x04, 0x2000_0200).unwrap();
        dma.write_word(0x00, 0x2000_0000).unwrap();
        dma.write_word(0x3C, 0x2000_0000).unwrap(); // AL3_READ_ADDR_TRIG
        drain(&mut dma, &mut bus);
        assert_eq!(bus.read_u8(0x2000_0207).unwrap(), 8);
    }

    #[test]
    fn four_irq_sets_drive_lines_10_to_13() {
        let mut dma = Rp2350Dma::new(10, BTreeMap::new());
        dma.intr = 0b11;
        dma.write_word(0x404, 0b01).unwrap(); // INTE0
        dma.write_word(0x434, 0b10).unwrap(); // INTE3
        assert_eq!(dma.level_irqs(), vec![10, 13]);
        assert_eq!(dma.read_word(0x43C).unwrap(), 0b10, "INTS3");
        dma.write_word(0x40C, 0b01).unwrap(); // INTS0 W1C
        assert_eq!(dma.level_irqs(), vec![13]);
        dma.write_word(0x428, 0b100).unwrap(); // INTF2 forces
        assert_eq!(dma.level_irqs(), vec![12, 13]);
        assert_eq!(dma.read_word(N_CHANNELS).unwrap(), 16);
    }

    #[test]
    fn reverse_increment_and_endless_mode() {
        let mut bus = SystemBus::new();
        for i in 0..4u64 {
            bus.write_u8(0x2000_0000 + i, 0x10 + i as u8).unwrap();
        }
        let mut dma = Rp2350Dma::new(10, BTreeMap::new());
        dma.write_word(0x00, 0x2000_0003).unwrap();
        dma.write_word(0x04, 0x2000_0100).unwrap();
        dma.write_word(TRANS_COUNT, 4).unwrap();
        dma.write_word(
            CTRL_TRIG,
            ctrl(0, true, true, 0, TREQ_PERMANENT) | CTRL_INCR_READ_REV,
        )
        .unwrap();
        drain(&mut dma, &mut bus);
        assert_eq!(bus.read_u8(0x2000_0100).unwrap(), 0x13);
        assert_eq!(bus.read_u8(0x2000_0103).unwrap(), 0x10);

        dma.write_word(0x40 + TRANS_COUNT, 0xF000_0000).unwrap();
        dma.write_word(0x40, 0x2000_0000).unwrap();
        dma.write_word(0x44, 0x2000_0200).unwrap();
        dma.write_word(0x40 + CTRL_TRIG, ctrl(0, false, false, 1, TREQ_PERMANENT))
            .unwrap();
        for _ in 0..100 {
            dma.run(&mut bus);
        }
        assert!(dma.channels[1].busy, "ENDLESS never completes");
        dma.write_word(CHAN_ABORT, 1 << 1).unwrap();
        assert!(!dma.channels[1].busy);
    }

    #[test]
    fn refuses_undeclared_dreqs_timers_sniff_and_reserved_modes() {
        let mut dma = Rp2350Dma::new(10, BTreeMap::new());
        dma.write_word(TRANS_COUNT, 1).unwrap();
        let e = dma
            .write_word(CTRL_TRIG, ctrl(0, false, false, 0, 24))
            .unwrap_err();
        assert!(e.to_string().contains("DREQ 24 is not declared"), "{e}");
        let e = dma
            .write_word(CTRL_TRIG, ctrl(0, false, false, 0, 59))
            .unwrap_err();
        assert!(e.to_string().contains("pacing timer"), "{e}");
        let e = dma
            .write_word(
                CTRL_TRIG,
                ctrl(0, false, false, 0, TREQ_PERMANENT) | CTRL_SNIFF_EN,
            )
            .unwrap_err();
        assert!(e.to_string().contains("SNIFF_EN"), "{e}");
        let e = dma.write_word(TRANS_COUNT, 0x2000_0001).unwrap_err();
        assert!(e.to_string().contains("reserved"), "{e}");
    }
}
