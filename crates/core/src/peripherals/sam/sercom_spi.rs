// SPDX-License-Identifier: MIT
//! SAMD51 SERCOM SPI master, byte-level controller qualification slice.
//!
//! Register facts: DS60001507 / CMSIS SERCOM SPI map. Transfers require MODE=3,
//! ENABLE, 8-bit CHSIZE, and the configured PORT mux/DOPO. The bus owns live
//! MCLK/GCLK gating. No wire-edge fidelity, dynamic GCLK frequency derivation,
//! DMA or complete native Arcade qualification is claimed here. SPI master
//! DRE/TXC/RXC/ERROR use four consecutive, exclusively owned NVIC lines when
//! configured. DS60001507F section 10.2.2 assigns SERCOM4 IRQs 62..65.
//! A byte takes 16 * (BAUD + 1) SERCOM core-clock cycles (f_SCK = f_GCLK /
//! (2 * (BAUD + 1)), DS60001507F 35.6.2.3). With a declared core clock channel
//! that resolves on the live clock tree, the delay is in CPU cycles at the
//! clock ratio, the fraction carried across back-to-back bytes; otherwise it
//! is in nominal core-clock cycles. Pad and edge timing are not modelled.

use crate::peripherals::spi::SpiDevice;
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

const DRE: u8 = 1;
const TXC: u8 = 2;
const RXC: u8 = 4;
const ERROR: u8 = 128;
const BUFOVF: u16 = 4;

/// Board-declared output pads; no hard-coded PyBadge pins in the controller.
#[derive(Debug, Clone, Copy)]
pub struct SamSpiPads {
    pub port_base: u64,
    pub sck: u8,
    pub mosi: u8,
    pub mux: u8,
    pub dopo: u8,
    /// GCLK peripheral channel of the SERCOM core clock (PCHCTRL index).
    pub core_gclk: Option<u64>,
}

pub struct SamSercomSpi {
    ctrla: u32,
    ctrlb: u32,
    baud: u8,
    inten: u8,
    flags: u8,
    status: u16,
    rx: RefCell<VecDeque<u8>>,
    last_rx: Cell<u8>,
    holding: Option<u8>,
    /// The byte on the wire: (value, time left, time per CPU cycle). Time is
    /// in CPU cycles scaled by the core clock (see [`Self::byte_time`]).
    active: Option<(u8, u64, u64)>,
    /// Time the last byte overran its deadline, carried into a byte that
    /// follows it without a gap.
    overrun: u64,
    pads: SamSpiPads,
    devices: Vec<Box<dyn SpiDevice>>,
    selected: Vec<bool>,
    clock: Option<crate::CycleClock>,
    scheduled: bool,
    legacy_walk_forced: bool,
    irq_base: Option<u32>,
}

impl std::fmt::Debug for SamSercomSpi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SamSercomSpi")
            .field("ctrla", &self.ctrla)
            .field("flags", &self.effective_flags())
            .finish()
    }
}

impl SamSercomSpi {
    pub fn new(pads: SamSpiPads) -> Self {
        Self {
            ctrla: 0,
            ctrlb: 0,
            baud: 0,
            inten: 0,
            flags: 0,
            status: 0,
            rx: RefCell::new(VecDeque::new()),
            last_rx: Cell::new(0),
            holding: None,
            active: None,
            overrun: 0,
            pads,
            devices: Vec::new(),
            selected: Vec::new(),
            clock: None,
            scheduled: false,
            legacy_walk_forced: false,
            irq_base: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn sched_driven(&self) -> bool {
        self.scheduler_mode() && !self.legacy_walk_forced
    }

    fn has_bus_work(&self) -> bool {
        self.holding.is_some() || self.active.is_some() || self.selected.iter().any(|s| *s)
    }

    fn refresh_chip_select(&mut self, bus: &mut dyn Bus) {
        self.selected.resize(self.devices.len(), false);
        for (dev, selected) in self.devices.iter_mut().zip(&mut self.selected) {
            let next = if dev.cs_pin().is_empty() {
                true
            } else {
                bus.read_gpio_output_by_label(dev.cs_pin()) == Some(false)
            };
            if next != *selected {
                if next {
                    dev.cs_select();
                } else {
                    dev.cs_release();
                }
            }
            *selected = next;
        }
    }

    fn pending_irqs(&self) -> Vec<u32> {
        let Some(base) = self.irq_base else {
            return Vec::new();
        };
        let active = self.effective_flags() & self.inten;
        [DRE, TXC, RXC, ERROR]
            .into_iter()
            .enumerate()
            .filter_map(|(line, mask)| (active & mask != 0).then_some(base + line as u32))
            .collect()
    }

    fn has_work(&self) -> bool {
        self.has_bus_work() || (self.irq_base.is_some() && self.effective_flags() & self.inten != 0)
    }

    fn advance_frame(&mut self, cycles: u64) {
        if self.enabled() {
            if let Some((_, left, per)) = &mut self.active {
                let elapsed = cycles * *per;
                if *left == 0 {
                    self.overrun += elapsed; // finished, not yet retired
                } else {
                    self.overrun = elapsed.saturating_sub(*left);
                    *left = left.saturating_sub(elapsed);
                }
            }
        }
    }

    fn enabled(&self) -> bool {
        self.ctrla & 2 != 0 && (self.ctrla >> 2) & 7 == 3 && self.ctrlb & 7 == 0
    }

    fn effective_flags(&self) -> u8 {
        self.flags
            | if self.enabled() && self.holding.is_none() {
                DRE
            } else {
                0
            }
            | if !self.rx.borrow().is_empty() { RXC } else { 0 }
    }

    fn reg(offset: u64) -> (u64, u32) {
        let base = match offset {
            0..=3 => 0,
            4..=7 => 4,
            0x0c => 0x0c,
            0x14 => 0x14,
            0x16 => 0x16,
            0x18 => 0x18,
            0x1a..=0x1b => 0x1a,
            0x1c..=0x1f => 0x1c,
            0x28..=0x2b => 0x28,
            _ => offset,
        };
        (base, ((offset - base) * 8) as u32)
    }

    fn value(&self, base: u64, consume: bool) -> u32 {
        match base {
            0 => self.ctrla,
            4 => self.ctrlb,
            0x0c => self.baud.into(),
            0x14 | 0x16 => self.inten.into(),
            0x18 => self.effective_flags().into(),
            0x1a => self.status.into(),
            0x1c => 0,
            0x28 => {
                if consume {
                    if let Some(v) = self.rx.borrow_mut().pop_front() {
                        self.last_rx.set(v);
                    }
                } else if let Some(v) = self.rx.borrow().front() {
                    return u32::from(*v);
                }
                self.last_rx.get().into()
            }
            _ => 0,
        }
    }

    fn store(&mut self, offset: u64, value: u32, width: u32) {
        let (base, shift) = Self::reg(offset);
        let mask = (if width == 32 {
            u32::MAX
        } else {
            (1u32 << width) - 1
        }) << shift;
        let bits = (value << shift) & mask;
        match base {
            0 => {
                let next = ((self.ctrla & !mask) | bits) & 0x7f33019f;
                if next & 1 != 0 {
                    for (dev, selected) in self.devices.iter_mut().zip(&mut self.selected) {
                        if *selected {
                            dev.cs_release();
                            *selected = false;
                        }
                    }
                    self.ctrla = 0;
                    self.ctrlb = 0;
                    self.baud = 0;
                    self.inten = 0;
                    self.flags = 0;
                    self.status = 0;
                    self.rx.borrow_mut().clear();
                    self.last_rx.set(0);
                    self.holding = None;
                    self.active = None;
                } else {
                    // Configuration is enable-protected; ENABLE itself is writable.
                    self.ctrla = if self.ctrla & 2 != 0 {
                        (self.ctrla & !2) | (next & 2)
                    } else {
                        next & !1
                    };
                    if self.ctrla & 2 == 0 {
                        self.holding = None;
                        self.active = None;
                    }
                }
            }
            4 if self.ctrla & 2 == 0 => self.ctrlb = ((self.ctrlb & !mask) | bits) & 0x0002e247,
            0x0c if self.ctrla & 2 == 0 => self.baud = bits as u8,
            0x14 => self.inten &= !(bits as u8),
            0x16 => self.inten |= bits as u8 & (DRE | TXC | RXC | ERROR),
            0x18 => self.flags &= !(bits as u8 & (TXC | ERROR)),
            0x1a => self.status &= !(bits as u16 & BUFOVF),
            0x28 if shift == 0 && self.enabled() && self.holding.is_none() => {
                self.holding = Some(value as u8);
                self.flags &= !TXC;
            }
            _ => {}
        }
    }

    fn pads_ready(&self, bus: &dyn Bus) -> bool {
        if (self.ctrla >> 16) & 3 != self.pads.dopo as u32 {
            return false;
        }
        [self.pads.sck, self.pads.mosi].iter().all(|&pin| {
            let cfg = bus
                .read_u8(self.pads.port_base + 0x40 + u64::from(pin))
                .ok();
            let mux = bus
                .read_u8(self.pads.port_base + 0x30 + u64::from(pin / 2))
                .ok();
            cfg.is_some_and(|v| v & 1 != 0)
                && mux.is_some_and(|v| (v >> ((pin & 1) * 4)) & 15 == self.pads.mux)
        })
    }
}

impl SamSercomSpi {
    /// One byte's duration and its decrement per CPU cycle: 16 * (BAUD + 1)
    /// core-clock cycles, scaled to CPU cycles by f_CPU / f_GCLK when both
    /// resolve, so `left / per` is the duration in CPU cycles.
    fn byte_time(&self, bus: &dyn Bus) -> (u64, u64) {
        let core = 16 * (u64::from(self.baud) + 1);
        let rates = self.pads.core_gclk.and_then(|channel| {
            Some((
                super::clock_tree::peripheral_hz(bus, channel)?,
                super::clock_tree::cpu_hz(bus)?,
            ))
        });
        match rates {
            Some((gclk, cpu)) if gclk > 0 && cpu > 0 => (core * cpu, gclk),
            _ => (core, 1),
        }
    }
}

impl Peripheral for SamSercomSpi {
    fn attach_irq_line(&mut self, irq: Option<u32>) {
        // Four consecutive NVIC positions, never wrap or claim system exceptions.
        self.irq_base = irq.filter(|irq| *irq <= 252);
    }
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        if let Some(base) = self.irq_base {
            let active = self.effective_flags() & self.inten;
            for (line, mask) in [DRE, TXC, RXC, ERROR].into_iter().enumerate() {
                report(base + line as u32, active & mask != 0);
            }
        }
    }
    fn reads_can_deassert_irq(&self) -> bool {
        true // Reading the low DATA lane consumes RX, unlike a debug peek.
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
    fn spi_attached_devices(&self) -> Option<&Vec<Box<dyn SpiDevice>>> {
        Some(&self.devices)
    }
    fn spi_attached_devices_mut(&mut self) -> Option<&mut Vec<Box<dyn SpiDevice>>> {
        Some(&mut self.devices)
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let (base, shift) = Self::reg(offset);
        Ok((self.value(base, offset == 0x28) >> shift) as u8)
    }
    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        let (base, shift) = Self::reg(offset);
        Ok((self.value(base, offset == 0x28) >> shift) as u16)
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let (base, shift) = Self::reg(offset);
        Ok(self.value(base, offset == 0x28) >> shift)
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        let (base, shift) = Self::reg(offset);
        Some((self.value(base, false) >> shift) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.store(offset, value.into(), 8);
        Ok(())
    }
    fn dma_tx_request(&self, offset: u64) -> Option<bool> {
        (offset == 0x28).then(|| self.enabled() && self.holding.is_none())
    }
    fn try_dma_write_tx_u8(&mut self, offset: u64, value: u8) -> SimResult<bool> {
        match self.dma_tx_request(offset) {
            Some(true) => {
                // Exclusive access: the same readiness predicate guards DATA
                // in store(), so this call really places this byte in holding.
                self.store(offset, value.into(), 8);
                Ok(true)
            }
            Some(false) => Ok(false),
            None => Err(crate::SimulationError::NotImplemented(
                "SAM SPI byte TX DMA only supports the low DATA lane".into(),
            )),
        }
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.store(offset, value.into(), 16);
        Ok(())
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.store(offset, value, 32);
        Ok(())
    }
    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_elapsed(1)
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        if !self.sched_driven() {
            self.advance_frame(cycles);
        }
        PeripheralTickResult {
            explicit_irqs: (!self.sched_driven()
                && self.irq_base.is_some()
                && self.effective_flags() & self.inten != 0)
                .then(|| self.pending_irqs()),
            ..Default::default()
        }
    }
    fn tick_elapsed_forced(&mut self, cycles: u64) -> PeripheralTickResult {
        self.advance_frame(cycles);
        PeripheralTickResult {
            explicit_irqs: (self.irq_base.is_some() && self.effective_flags() & self.inten != 0)
                .then(|| self.pending_irqs()),
            ..Default::default()
        }
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.clock = Some(clock);
    }
    fn uses_scheduler(&self) -> bool {
        self.sched_driven()
    }
    fn force_legacy_walk(&mut self) -> bool {
        self.legacy_walk_forced = true;
        true
    }
    fn needs_legacy_walk(&self) -> bool {
        !self.sched_driven()
    }
    fn requires_clocked_tick(&self) -> bool {
        // A cancelled wake has no clocked work. Let on_event retire it even
        // if firmware gated the controller immediately after SWRST/disable.
        self.has_bus_work()
    }
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.sched_driven() && self.has_work() && !self.scheduled {
            self.scheduled = true;
            vec![(0, 0)]
        } else {
            Vec::new()
        }
    }
    fn on_event(
        &mut self,
        _token: u32,
        _sched: &mut crate::sched::EventScheduler,
        bus: &mut dyn Bus,
    ) -> crate::sched::EventResult {
        // SWRST/disable can cancel a queued frame before its wake is delivered.
        // A stale wake must not select a slave (or perpetuate itself) on idle CS.
        if !self.has_work() {
            self.scheduled = false;
            return crate::sched::EventResult::default();
        }
        // Match the historical PRE-tick bus pass, then the countdown tick.
        // Active transfers intentionally retain one-cycle service; the win is
        // removing idle SPI from the walk, not changing BAUD/CS/mux semantics.
        if self.has_bus_work() {
            self.tick_with_bus_forced(bus);
            self.advance_frame(1);
        }
        // Re-emit held levels until firmware masks/acknowledges them, just as
        // the reference walk does. Pure IRQ service must not select a slave.
        self.scheduled = self.has_work();
        crate::sched::EventResult {
            explicit_irqs: self.pending_irqs(),
            reschedule_delay: self.scheduled.then_some(1),
            ..Default::default()
        }
    }
    fn needs_bus_tick(&self) -> bool {
        !self.sched_driven() && self.has_bus_work()
    }
    fn needs_bus_tick_forced(&self) -> bool {
        self.has_bus_work()
    }
    fn prepare_dma_tx(&mut self, bus: &mut dyn Bus) {
        self.refresh_chip_select(bus);
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if !self.sched_driven() {
            self.tick_with_bus_forced(bus);
        }
    }
    fn tick_with_bus_forced(&mut self, bus: &mut dyn Bus) {
        self.refresh_chip_select(bus);
        if !self.enabled() || !self.pads_ready(bus) {
            return;
        }
        let mut completed = false;
        if let Some((tx, 0, _)) = self.active {
            let tx = if self.ctrla & (1 << 30) != 0 {
                tx.reverse_bits()
            } else {
                tx
            };
            let mut rx = 0xff;
            for (dev, selected) in self.devices.iter_mut().zip(&self.selected) {
                if *selected {
                    rx &= dev.transfer(tx);
                }
            }
            if self.ctrla & (1 << 30) != 0 {
                rx = rx.reverse_bits();
            }
            if self.ctrlb & (1 << 17) != 0 {
                if self.rx.borrow().len() < 2 {
                    self.rx.borrow_mut().push_back(rx);
                } else {
                    self.status |= BUFOVF;
                    self.flags |= ERROR;
                }
            }
            self.active = None;
            completed = true;
        }
        if self.active.is_none() {
            if let Some(tx) = self.holding.take() {
                let (left, per) = self.byte_time(bus);
                // Back to back, the next byte starts where the last one ended.
                let carried = if completed {
                    self.overrun.min(left - 1)
                } else {
                    0
                };
                self.active = Some((tx, left - carried, per));
            } else if completed {
                self.flags |= TXC;
            }
            self.overrun = 0;
        }
    }
    fn drives_central_device_time(&self) -> bool {
        true
    }
    fn advance_attached_device_time_us(&mut self, us: u64) {
        for d in &mut self.devices {
            d.advance_time_us(us);
        }
    }
    fn drain_attached_pin_drives(&mut self, out: &mut Vec<(String, String, bool)>) {
        for d in &mut self.devices {
            crate::peripherals::device::drain_spi_pin_drives(&mut **d, out);
        }
    }
    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for d in &self.devices {
            crate::inspect::visit_spi_device(&**d, f);
        }
    }
    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for d in &mut self.devices {
            if let Some(si) = d.as_sim_input_mut() {
                if f(si) {
                    return true;
                }
            }
        }
        false
    }
}
