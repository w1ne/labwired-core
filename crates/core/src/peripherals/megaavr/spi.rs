// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! megaAVR 0-series SPI, host (master) side, Normal mode (ATmega4808/4809
//! data sheet DS40002173C §24; register offsets and bit positions from the
//! Microchip ATmega4809 ATDF).
//!
//! Registers: CTRLA 0x00, CTRLB 0x01, INTCTRL 0x02, INTFLAGS 0x03, DATA 0x04.
//!
//! Modelled: with CTRLA.ENABLE and CTRLA.MASTER set, a DATA write starts one
//! 8-bit transfer. When it completes, the received byte is in the Receive
//! Data register (a DATA read) and INTFLAGS.IF is set. A DATA write during a
//! transfer sets WRCOL and leaves the byte in flight unchanged. IF and WRCOL
//! clear on a DATA access that follows an INTFLAGS read which saw them set
//! (§24.5.4). CTRLA.DORD selects the bit order. A cleared ENABLE drops the
//! byte in flight.
//!
//! Pads: PORTMUX.TWISPIROUTEA SPI0[1:0] selects PA[7:4], PC[3:0] or PE[3:0]
//! (MOSI, MISO, SCK, SS in that order). A byte reaches the attached devices
//! only while the routed SCK pad is an output; devices receive 0xFF while
//! the routed MOSI pad is not an output. A device is selected by the output
//! level of its chip-select pad. MISO is taken as high when no selected
//! device drives it, so such a transfer receives 0xFF.
//!
//! Timing: one bit takes PRESC (4, 16, 64 or 128) CPU cycles, half that with
//! CLK2X (CLK_PER is taken as the CPU clock); a transfer takes 8 bits.
//!
//! Interrupt (`irq`: the part's SPI vector number): raised once when a
//! transfer completes with INTCTRL.IE set, when IE goes 1 with IF set, and
//! when WRCOL is set with IE set. Silicon clears IF when the vector executes
//! (§24.5.4); here IF clears when the request is raised, so firmware that
//! polls IF with IE set and interrupts masked does not see it. A handler
//! that returns with WRCOL still set is not re-entered, though silicon would.
//!
//! Refused rather than ignored: Buffer mode (CTRLB.BUFEN) and client mode
//! (ENABLE without MASTER). Not modelled: the multi-host SS input (the host
//! never yields to a low SS pad, whatever CTRLB.SSD says), the CTRLB.MODE
//! clock phase and polarity (stored; devices are byte-level), the SCK event
//! generator, daisy chains, and writes to INTFLAGS (ignored).

use crate::peripherals::device::{drain_spi_pin_drives, SpiDevice};
use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::cell::Cell;

const CTRLA: u64 = 0x00;
const CTRLB: u64 = 0x01;
const INTCTRL: u64 = 0x02;
const INTFLAGS: u64 = 0x03;
const DATA: u64 = 0x04;
const WINDOW: u64 = 0x05;

const CTRLA_MASK: u8 = 0x77;
const CTRLA_DORD: u8 = 1 << 6;
const CTRLA_MASTER: u8 = 1 << 5;
const CTRLA_CLK2X: u8 = 1 << 4;
const CTRLA_ENABLE: u8 = 1 << 0;
const CTRLB_MASK: u8 = 0xC7;
const CTRLB_BUFEN: u8 = 1 << 7;
const INTCTRL_MASK: u8 = 0xF1;
const INTCTRL_IE: u8 = 1 << 0;
const INTFLAGS_IF: u8 = 1 << 7;
const INTFLAGS_WRCOL: u8 = 1 << 6;

/// PORTMUX.TWISPIROUTEA on the bus (data-space 0x05E3); SPI0[1:0] is bits 1:0.
const PORTMUX_TWISPIROUTEA: u64 = 0x0001_05E3;
/// (MOSI, SCK) pads of the DEFAULT, ALT1 and ALT2 routes. Route 3 is NONE.
const ROUTES: [(&str, &str); 3] = [("PA4", "PA6"), ("PC0", "PC2"), ("PE0", "PE2")];

/// Scheduler token of the interrupt wake; transfer wakes carry `generation`,
/// which is never 0.
const IRQ_TOKEN: u32 = 0;

pub struct MegaAvrSpi {
    ctrla: u8,
    ctrlb: u8,
    intctrl: u8,
    /// INTFLAGS. A DATA read clears the flags an INTFLAGS read saw set.
    flags: Cell<u8>,
    /// Flags an INTFLAGS read saw set; the next DATA access clears them.
    seen: Cell<u8>,
    /// Receive Data register.
    rx: u8,
    /// The byte in flight.
    shift: Option<u8>,
    /// CPU cycle at which `shift` completes.
    due: u64,
    generation: u32,
    /// A transfer wake waits to be handed to the scheduler.
    new_wake: bool,
    /// An interrupt assertion waits to be raised.
    irq_event: bool,
    irq_live: bool,
    /// SPI vector number.
    irq: u32,
    devices: Vec<Box<dyn SpiDevice>>,
    clock: Option<CycleClock>,
}

impl std::fmt::Debug for MegaAvrSpi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MegaAvrSpi")
            .field("irq", &self.irq)
            .field("ctrla", &self.ctrla)
            .field("intflags", &self.flags.get())
            .field("devices", &self.devices.len())
            .finish_non_exhaustive()
    }
}

impl MegaAvrSpi {
    pub fn new(irq: u32) -> Self {
        Self {
            ctrla: 0,
            ctrlb: 0,
            intctrl: 0,
            flags: Cell::new(0),
            seen: Cell::new(0),
            rx: 0,
            shift: None,
            due: 0,
            generation: 0,
            new_wake: false,
            irq_event: false,
            irq_live: false,
            irq,
            devices: Vec::new(),
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn refuse(what: String) -> SimResult<()> {
        Err(SimulationError::NotImplemented(format!(
            "megaAVR SPI: {what} is not modelled"
        )))
    }

    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    fn host_enabled(&self) -> bool {
        self.ctrla & (CTRLA_ENABLE | CTRLA_MASTER) == CTRLA_ENABLE | CTRLA_MASTER
    }

    /// CPU cycles one 8-bit transfer takes.
    fn byte_cycles(&self) -> u64 {
        let bit = [4, 16, 64, 128][usize::from((self.ctrla >> 1) & 3)];
        8 * if self.ctrla & CTRLA_CLK2X != 0 {
            bit / 2
        } else {
            bit
        }
    }

    /// A DATA access clears the flags the last INTFLAGS read saw set.
    fn data_access(&self) {
        self.flags.set(self.flags.get() & !self.seen.take());
    }

    /// Set IF, or raise the vector in its place while IE is set.
    fn transfer_done(&mut self) {
        if self.intctrl & INTCTRL_IE != 0 {
            self.irq_event = true;
        } else {
            self.flags.set(self.flags.get() | INTFLAGS_IF);
        }
    }

    /// Deliver chip-select edges the port write hook has not, from the live
    /// pad levels.
    fn sync_chip_selects(&mut self, bus: &dyn Bus) {
        for dev in &mut self.devices {
            let Some(high) = bus.read_gpio_output_by_label(dev.cs_pin()) else {
                continue;
            };
            match (dev.is_selected(), dev.cs_selected_at(high)) {
                (false, true) => dev.cs_select(),
                (true, false) => dev.cs_release(),
                _ => {}
            }
        }
    }

    fn complete(&mut self, bus: &dyn Bus) {
        let Some(tx) = self.shift.take() else {
            return;
        };
        let route = usize::from(bus.read_u8(PORTMUX_TWISPIROUTEA).unwrap_or(0) & 3);
        let driven = |pad: &str| bus.read_gpio_output_by_label(pad).is_some();
        let (mosi, sck) = ROUTES
            .get(route)
            .map_or((false, false), |(mosi, sck)| (driven(mosi), driven(sck)));
        let lsb_first = self.ctrla & CTRLA_DORD != 0;
        let mut rx = 0xFF;
        if sck {
            let wire = match (mosi, lsb_first) {
                (false, _) => 0xFF,
                (true, true) => tx.reverse_bits(),
                (true, false) => tx,
            };
            self.sync_chip_selects(bus);
            for dev in &mut self.devices {
                let selected = dev.cs_pin().is_empty() || dev.is_selected();
                if selected || dev.clocks_when_deselected() {
                    let miso = dev.transfer(wire);
                    if selected {
                        rx &= miso;
                    }
                }
            }
            if lsb_first {
                rx = rx.reverse_bits();
            }
        }
        self.rx = rx;
        self.transfer_done();
    }

    fn reg(&self, offset: u64) -> u8 {
        match offset {
            CTRLA => self.ctrla,
            CTRLB => self.ctrlb,
            INTCTRL => self.intctrl,
            INTFLAGS => self.flags.get(),
            DATA => self.rx,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            CTRLA => {
                if value & (CTRLA_ENABLE | CTRLA_MASTER) == CTRLA_ENABLE {
                    return Self::refuse(format!("client mode (CTRLA {value:#04x})"));
                }
                self.ctrla = value & CTRLA_MASK;
                if value & CTRLA_ENABLE == 0 {
                    self.shift = None;
                }
            }
            CTRLB => {
                if value & CTRLB_BUFEN != 0 {
                    return Self::refuse(format!("Buffer mode (CTRLB {value:#04x})"));
                }
                self.ctrlb = value & CTRLB_MASK;
            }
            INTCTRL => {
                let rising = value & !self.intctrl;
                self.intctrl = value & INTCTRL_MASK;
                if rising & INTCTRL_IE != 0 && self.flags.get() & INTFLAGS_IF != 0 {
                    self.flags.set(self.flags.get() & !INTFLAGS_IF);
                    self.irq_event = true;
                }
            }
            DATA => {
                self.data_access();
                if !self.host_enabled() {
                    return Ok(());
                }
                if self.shift.is_some() {
                    self.flags.set(self.flags.get() | INTFLAGS_WRCOL);
                    if self.intctrl & INTCTRL_IE != 0 {
                        self.irq_event = true;
                    }
                    return Ok(());
                }
                self.shift = Some(value);
                self.due = self.now() + self.byte_cycles();
                self.generation = self.generation.wrapping_add(1).max(1);
                self.new_wake = true;
            }
            _ => {}
        }
        Ok(())
    }

    fn take_irq(&mut self) -> Vec<u32> {
        if std::mem::take(&mut self.irq_event) {
            vec![self.irq]
        } else {
            Vec::new()
        }
    }
}

impl Peripheral for MegaAvrSpi {
    // The bus attach funnel reaches `spi_attached_devices_mut` through these.
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
    fn peek(&self, offset: u64) -> Option<u8> {
        (offset < WINDOW).then(|| self.reg(offset))
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let value = self.reg(offset);
        match offset {
            INTFLAGS => self.seen.set(self.seen.get() | value),
            DATA => self.data_access(),
            _ => {}
        }
        Ok(value)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_reg(offset, value)
    }
    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && (self.shift.is_some() || self.irq_event)
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }
    fn needs_bus_tick(&self) -> bool {
        !self.scheduler_mode() && self.shift.is_some()
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        // Without a clock a transfer completes on the next tick.
        if !self.scheduler_mode() && self.clock.as_ref().is_none_or(|c| c.now() >= self.due) {
            self.complete(bus);
        }
    }
    fn tick(&mut self) -> PeripheralTickResult {
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        let irqs = self.take_irq();
        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        let mut events = Vec::new();
        if std::mem::take(&mut self.new_wake) && self.shift.is_some() {
            let delay = self.due.saturating_sub(self.now()).max(1);
            events.push((delay, self.generation));
        }
        if self.irq_event && !self.irq_live {
            self.irq_live = true;
            events.push((1, IRQ_TOKEN));
        }
        events
    }
    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        bus: &mut dyn Bus,
    ) -> crate::sched::EventResult {
        if event_token == IRQ_TOKEN {
            self.irq_live = false;
        } else if event_token == self.generation {
            self.complete(bus);
        }
        crate::sched::EventResult {
            explicit_irqs: self.take_irq(),
            ..Default::default()
        }
    }
    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }
    fn on_node_reset(&mut self) {
        let devices = std::mem::take(&mut self.devices);
        let clock = self.clock.take();
        // A stale wake must not complete a transfer started after the reset.
        let generation = self.generation;
        *self = Self::new(self.irq);
        self.devices = devices;
        self.clock = clock;
        self.generation = generation;
    }
    fn drives_central_device_time(&self) -> bool {
        true
    }
    fn has_attached_time_devices(&self) -> bool {
        !self.devices.is_empty()
    }
    fn advance_attached_device_time_us(&mut self, us: u64) {
        for dev in &mut self.devices {
            dev.advance_time_us(us);
        }
    }
    fn drain_attached_pin_drives(&mut self, out: &mut Vec<(String, String, bool)>) {
        for dev in &mut self.devices {
            drain_spi_pin_drives(&mut **dev, out);
        }
    }
    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for dev in &mut self.devices {
            if let Some(input) = dev.as_sim_input_mut() {
                if f(input) {
                    return true;
                }
            }
        }
        false
    }
    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for dev in &self.devices {
            crate::inspect::visit_spi_device(&**dev, f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DmaRequest, SimulationConfig};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    const VECTOR: u32 = 16;
    const HOST: u8 = CTRLA_MASTER | CTRLA_ENABLE;

    /// The PORTMUX route and the pads that are outputs, with their levels.
    struct Pads {
        route: u8,
        outputs: HashMap<&'static str, bool>,
        config: SimulationConfig,
    }

    impl Pads {
        /// The Nano Every wiring: SPI0 on PE[3:0], SS = PE3 high.
        fn nano_every() -> Self {
            Self {
                route: 2,
                outputs: HashMap::from([("PE0", false), ("PE2", false), ("PE3", true)]),
                config: SimulationConfig::default(),
            }
        }
    }

    impl Bus for Pads {
        fn read_u8(&self, addr: u64) -> SimResult<u8> {
            Ok(if addr == PORTMUX_TWISPIROUTEA {
                self.route
            } else {
                0
            })
        }
        fn write_u8(&mut self, _addr: u64, _value: u8) -> SimResult<()> {
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
        fn read_gpio_output_by_label(&self, pin: &str) -> Option<bool> {
            self.outputs.get(pin).copied()
        }
    }

    #[derive(Debug, Default)]
    struct Wire {
        selected: bool,
        mosi: Vec<u8>,
        miso: Vec<u8>,
        log: Vec<&'static str>,
    }

    /// A device on PE3 that answers with `miso` in order, then 0x00.
    struct Dev(Arc<Mutex<Wire>>);

    impl SpiDevice for Dev {
        fn cs_select(&mut self) {
            let mut w = self.0.lock().unwrap();
            w.selected = true;
            w.log.push("select");
        }
        fn cs_release(&mut self) {
            let mut w = self.0.lock().unwrap();
            w.selected = false;
            w.log.push("release");
        }
        fn is_selected(&self) -> bool {
            self.0.lock().unwrap().selected
        }
        fn transfer(&mut self, mosi: u8) -> u8 {
            let mut w = self.0.lock().unwrap();
            w.mosi.push(mosi);
            if w.miso.is_empty() {
                0
            } else {
                w.miso.remove(0)
            }
        }
        fn cs_pin(&self) -> &str {
            "PE3"
        }
    }

    fn spi(miso: &[u8]) -> (MegaAvrSpi, Arc<Mutex<Wire>>) {
        let wire = Arc::new(Mutex::new(Wire {
            miso: miso.to_vec(),
            ..Wire::default()
        }));
        let mut spi = MegaAvrSpi::new(VECTOR);
        spi.devices.push(Box::new(Dev(wire.clone())));
        spi.write(CTRLA, HOST).unwrap();
        (spi, wire)
    }

    /// Walk until the transfer in flight completes; the vectors it raised.
    fn settle(spi: &mut MegaAvrSpi, bus: &mut Pads) -> Vec<u32> {
        let mut irqs = Vec::new();
        while spi.legacy_tick_active() {
            if spi.needs_bus_tick() {
                spi.tick_with_bus(bus);
            }
            irqs.extend(spi.tick().explicit_irqs.unwrap_or_default());
        }
        irqs
    }

    /// The ArduinoCore-megaavr `SPIClass::transfer()` sequence.
    fn transfer(spi: &mut MegaAvrSpi, bus: &mut Pads, tx: u8) -> u8 {
        spi.write(DATA, tx).unwrap();
        assert_eq!(spi.read(INTFLAGS).unwrap() & INTFLAGS_IF, 0, "in flight");
        settle(spi, bus);
        assert_ne!(spi.read(INTFLAGS).unwrap() & INTFLAGS_IF, 0, "complete");
        spi.read(DATA).unwrap()
    }

    #[test]
    fn selected_device_exchanges_exact_bytes() {
        let (mut spi, wire) = spi(&[0x01, 0x90, 0x16, 0x00]);
        let mut bus = Pads::nano_every();
        bus.outputs.insert("PE3", false);
        let rx: Vec<u8> = [0xA1, 0xB2, 0xC3, 0xD4]
            .into_iter()
            .map(|tx| transfer(&mut spi, &mut bus, tx))
            .collect();
        assert_eq!(rx, [0x01, 0x90, 0x16, 0x00]);
        bus.outputs.insert("PE3", true);
        assert_eq!(transfer(&mut spi, &mut bus, 0x55), 0xFF, "CS is high");
        let wire = wire.lock().unwrap();
        assert_eq!(wire.mosi, [0xA1, 0xB2, 0xC3, 0xD4]);
        assert_eq!(wire.log, ["select", "release"]);
    }

    #[test]
    fn interrupt_flag_clears_on_data_access_after_an_intflags_read() {
        let (mut spi, _) = spi(&[0x42]);
        let mut bus = Pads::nano_every();
        bus.outputs.insert("PE3", false);
        spi.write(DATA, 0).unwrap();
        settle(&mut spi, &mut bus);
        assert_eq!(spi.peek(INTFLAGS), Some(INTFLAGS_IF));
        assert_eq!(spi.read(DATA).unwrap(), 0x42);
        assert_eq!(spi.peek(INTFLAGS), Some(INTFLAGS_IF), "IF was not read");
        assert_eq!(spi.read(INTFLAGS).unwrap(), INTFLAGS_IF);
        assert_eq!(spi.peek(INTFLAGS), Some(INTFLAGS_IF), "DATA not accessed");
        assert_eq!(spi.read(DATA).unwrap(), 0x42);
        assert_eq!(spi.peek(INTFLAGS), Some(0));
        spi.write(INTFLAGS, 0xFF).unwrap();
        assert_eq!(spi.peek(INTFLAGS), Some(0));
    }

    #[test]
    fn data_write_during_a_transfer_sets_wrcol_and_keeps_the_byte() {
        let (mut spi, wire) = spi(&[]);
        let mut bus = Pads::nano_every();
        bus.outputs.insert("PE3", false);
        spi.write(DATA, 0x11).unwrap();
        spi.write(DATA, 0x22).unwrap();
        assert_eq!(spi.read(INTFLAGS).unwrap(), INTFLAGS_WRCOL);
        settle(&mut spi, &mut bus);
        assert_eq!(wire.lock().unwrap().mosi, [0x11]);
        assert_eq!(spi.read(INTFLAGS).unwrap(), INTFLAGS_IF | INTFLAGS_WRCOL);
        spi.read(DATA).unwrap();
        assert_eq!(spi.read(INTFLAGS).unwrap(), 0);
    }

    #[test]
    fn interrupt_enable_raises_the_vector_in_place_of_the_flag() {
        let (mut spi, _) = spi(&[0x42]);
        let mut bus = Pads::nano_every();
        bus.outputs.insert("PE3", false);
        spi.write(DATA, 0).unwrap();
        assert!(settle(&mut spi, &mut bus).is_empty());
        spi.write(INTCTRL, INTCTRL_IE).unwrap();
        assert_eq!(settle(&mut spi, &mut bus), [VECTOR], "IE goes 1 with IF");
        assert_eq!(spi.peek(INTFLAGS), Some(0));
        spi.write(DATA, 0).unwrap();
        assert_eq!(settle(&mut spi, &mut bus), [VECTOR]);
        assert_eq!(spi.peek(INTFLAGS), Some(0), "the vector clears IF");
        spi.write(DATA, 0).unwrap();
        spi.write(DATA, 0).unwrap();
        assert_eq!(settle(&mut spi, &mut bus), [VECTOR], "WRCOL and IF: one");
        assert_eq!(spi.peek(INTFLAGS), Some(INTFLAGS_WRCOL));
    }

    #[test]
    fn transfer_takes_eight_prescaled_bit_times() {
        let mut bus = Pads::nano_every();
        for (ctrla, cycles) in [
            (HOST, 32),
            (HOST | CTRLA_CLK2X, 16),
            (HOST | 0x02, 128),
            (HOST | 0x06, 1024),
        ] {
            let (mut spi, _) = spi(&[]);
            let clock = CycleClock::default();
            spi.attach_cycle_clock(clock.clone());
            spi.write(CTRLA, ctrla).unwrap();
            spi.write(DATA, 0).unwrap();
            if spi.scheduler_mode() {
                assert_eq!(spi.take_scheduled_events(), [(cycles, spi.generation)]);
                continue;
            }
            clock.publish(cycles - 1);
            spi.tick_with_bus(&mut bus);
            assert_eq!(spi.peek(INTFLAGS), Some(0), "CTRLA {ctrla:#04x}");
            clock.publish(cycles);
            spi.tick_with_bus(&mut bus);
            assert_eq!(spi.peek(INTFLAGS), Some(INTFLAGS_IF), "CTRLA {ctrla:#04x}");
        }
    }

    #[test]
    fn dord_reverses_the_bit_order_both_ways() {
        let (mut spi, wire) = spi(&[0x01]);
        let mut bus = Pads::nano_every();
        bus.outputs.insert("PE3", false);
        spi.write(CTRLA, HOST | CTRLA_DORD).unwrap();
        assert_eq!(transfer(&mut spi, &mut bus, 0x0F), 0x80);
        assert_eq!(wire.lock().unwrap().mosi, [0xF0]);
    }

    #[test]
    fn bytes_need_the_routed_sck_and_mosi_pads_as_outputs() {
        let (mut spi, wire) = spi(&[0x42, 0x43]);
        let mut bus = Pads::nano_every();
        bus.outputs.insert("PE3", false);
        bus.outputs.remove("PE0");
        assert_eq!(transfer(&mut spi, &mut bus, 0x12), 0x42);
        assert_eq!(wire.lock().unwrap().mosi, [0xFF], "MOSI is an input");
        bus.outputs.insert("PE0", false);
        // SCK is an input, the DEFAULT route, the NONE route: no clock.
        bus.outputs.remove("PE2");
        assert_eq!(transfer(&mut spi, &mut bus, 0x12), 0xFF);
        bus.outputs.insert("PE2", false);
        for route in [0, 3] {
            bus.route = route;
            assert_eq!(transfer(&mut spi, &mut bus, 0x12), 0xFF);
        }
        assert_eq!(wire.lock().unwrap().mosi, [0xFF]);
        bus.route = 2;
        assert_eq!(transfer(&mut spi, &mut bus, 0x12), 0x43);
    }

    #[test]
    fn disabled_host_starts_no_transfer_and_disable_drops_the_byte() {
        let (mut spi, wire) = spi(&[0x42]);
        let mut bus = Pads::nano_every();
        bus.outputs.insert("PE3", false);
        spi.write(CTRLA, CTRLA_MASTER).unwrap();
        spi.write(DATA, 0x12).unwrap();
        assert!(!spi.legacy_tick_active());
        spi.write(CTRLA, HOST).unwrap();
        spi.write(DATA, 0x12).unwrap();
        spi.write(CTRLA, CTRLA_MASTER).unwrap();
        settle(&mut spi, &mut bus);
        assert_eq!(spi.peek(INTFLAGS), Some(0));
        assert!(wire.lock().unwrap().mosi.is_empty());
    }

    #[test]
    fn unmodelled_modes_are_refused() {
        let mut spi = MegaAvrSpi::new(VECTOR);
        assert!(spi.write(CTRLA, CTRLA_ENABLE).is_err(), "client mode");
        assert!(spi.write(CTRLB, CTRLB_BUFEN | 0x04).is_err(), "Buffer mode");
        // ArduinoCore-megaavr SPIClass::begin() / config() / end().
        for (offset, value) in [(CTRLB, 0x04), (CTRLA, HOST), (CTRLB, 0x07), (CTRLA, 0x20)] {
            spi.write(offset, value).unwrap();
        }
        assert_eq!(spi.peek(CTRLB), Some(0x07));
    }
}
