// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! i.MX RT1060 LPI2C master (LPI2C1..4, IMXRT1060RM §47).
//!
//! The master is a command engine: firmware pushes command words into the
//! 4-entry transmit FIFO through `MTDR` (CMD[10:8] + DATA[7:0]) and the
//! engine executes them on the wire one after another, each taking its bit
//! time at the configured SCL rate:
//!
//! | CMD | action                                         |
//! |-----|------------------------------------------------|
//! | 000 | transmit DATA                                  |
//! | 001 | receive DATA+1 bytes into the RX FIFO          |
//! | 010 | STOP                                           |
//! | 011 | receive and discard DATA+1 bytes               |
//! | 100 | (repeated) START + address DATA                |
//! | 101 | START + address, expect NACK                   |
//! | 110 | high-speed START + address (treated as 100)    |
//! | 111 | high-speed START, expect NACK (treated as 101) |
//!
//! Status (`MSR`): TDF (TX count <= TXWATER), RDF (RX count > RXWATER), MBF
//! (commands executing), BBF (between START and STOP), and the
//! write-1-to-clear EPF/SDF/NDF/ALF/FEF/PLTF/DMF. A NACK (no device claims the
//! address, or a device refuses a byte) sets NDF, the master drives a STOP
//! (SDF) and does not start a new transfer until NDF is cleared — RM §47.3.1.3
//! "MSR[NDF]". Slaves are `I2cDevice`s attached through the bus funnel.

use super::{byte_of, Timebase};
use crate::bus::bus_trace::{BusPayload, BusTrace, I2cSym};
use crate::peripherals::i2c::I2cDevice;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;

const VERID: u32 = 0x000;
const PARAM: u32 = 0x004;
const MCR: u32 = 0x010;
const MSR: u32 = 0x014;
const MIER: u32 = 0x018;
const MDER: u32 = 0x01C;
const MCFGR0: u32 = 0x020;
const MCFGR1: u32 = 0x024;
const MCFGR2: u32 = 0x028;
const MCFGR3: u32 = 0x02C;
const MDMR: u32 = 0x040;
const MCCR0: u32 = 0x048;
const MCCR1: u32 = 0x050;
const MFCR: u32 = 0x058;
const MFSR: u32 = 0x05C;
const MTDR: u32 = 0x060;
const MRDR: u32 = 0x070;

const FIFO_DEPTH: usize = 4;
/// Internal marker for "one received byte on the wire" (not a CMD value).
const RECV_BYTE: u16 = 0xFFFF;

const MCR_MEN: u32 = 1 << 0;
const MCR_RST: u32 = 1 << 1;
const MCR_RTF: u32 = 1 << 8;
const MCR_RRF: u32 = 1 << 9;

const MSR_TDF: u32 = 1 << 0;
const MSR_RDF: u32 = 1 << 1;
const MSR_EPF: u32 = 1 << 8;
const MSR_SDF: u32 = 1 << 9;
const MSR_NDF: u32 = 1 << 10;
const MSR_ALF: u32 = 1 << 11;
const MSR_FEF: u32 = 1 << 12;
const MSR_PLTF: u32 = 1 << 13;
const MSR_DMF: u32 = 1 << 14;
const MSR_MBF: u32 = 1 << 24;
const MSR_BBF: u32 = 1 << 25;
const MSR_W1C: u32 = MSR_EPF | MSR_SDF | MSR_NDF | MSR_ALF | MSR_FEF | MSR_PLTF | MSR_DMF;

const MCFGR1_IGNACK: u32 = 1 << 9;

/// LPI2C functional clock when the chip YAML does not say otherwise:
/// PLL3 / 8 = 60 MHz (`CSCDR2.LPI2C_CLK_SEL` = 0), divided by
/// `LPI2C_CLK_PODF`+1 = 1 in the SDK board default... i.e. 60 MHz.
pub const DEFAULT_LPI2C_CLK_HZ: u64 = 60_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    Write,
    Read,
}

struct Inner {
    mcr: u32,
    msr: u32,
    mier: u32,
    mder: u32,
    mcfgr: [u32; 4],
    mdmr: u32,
    mccr0: u32,
    mccr1: u32,
    mfcr: u32,
    tx: VecDeque<u16>,
    rx: VecDeque<u8>,
    /// Command on the wire and the cycle its wire time ends.
    inflight: Option<(u16, u64)>,
    /// Receive command in progress: bytes still to clock in, keep/discard.
    receiving: Option<(u16, bool)>,
    /// Selected slave and transfer direction (between START and STOP).
    selected: Option<(usize, Dir)>,
    bus_busy: bool,
    synced: u64,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lpi2cInner")
            .field("mcr", &self.mcr)
            .field("msr", &self.msr)
            .field("tx", &self.tx)
            .field("rx", &self.rx)
            .finish_non_exhaustive()
    }
}

impl Inner {
    fn new() -> Self {
        Self {
            mcr: 0,
            msr: 0,
            mier: 0,
            mder: 0,
            mcfgr: [0; 4],
            mdmr: 0,
            mccr0: 0,
            mccr1: 0,
            mfcr: 0,
            tx: VecDeque::new(),
            rx: VecDeque::new(),
            inflight: None,
            receiving: None,
            selected: None,
            bus_busy: false,
            synced: 0,
        }
    }
    fn tdf(&self) -> bool {
        self.tx.len() <= (self.mfcr & 0x3) as usize
    }
    fn rdf(&self) -> bool {
        self.rx.len() > ((self.mfcr >> 16) & 0x3) as usize
    }
    fn msr_view(&self) -> u32 {
        let mut v = self.msr & MSR_W1C;
        if self.mcr & MCR_MEN != 0 && self.tdf() {
            v |= MSR_TDF;
        }
        if self.rdf() {
            v |= MSR_RDF;
        }
        if self.inflight.is_some()
            || self.receiving.is_some()
            || !self.tx.is_empty() && self.mcr & MCR_MEN != 0 && self.msr & MSR_NDF == 0
        {
            v |= MSR_MBF;
        }
        if self.bus_busy {
            v |= MSR_BBF;
        }
        v
    }
}

pub struct ImxrtLpi2c {
    inner: RefCell<Inner>,
    slaves: RefCell<Vec<Box<dyn I2cDevice>>>,
    time: Timebase,
    clk_hz: u64,
    trace: BusTrace,
    trace_name: String,
}

impl std::fmt::Debug for ImxrtLpi2c {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImxrtLpi2c")
            .field("inner", &self.inner)
            .field("slaves", &self.slaves.borrow().len())
            .finish_non_exhaustive()
    }
}

impl Default for ImxrtLpi2c {
    fn default() -> Self {
        Self::new(DEFAULT_LPI2C_CLK_HZ)
    }
}

impl ImxrtLpi2c {
    pub fn new(clk_hz: u64) -> Self {
        Self {
            inner: RefCell::new(Inner::new()),
            slaves: RefCell::new(Vec::new()),
            time: Timebase::default(),
            clk_hz: clk_hz.max(1),
            trace: BusTrace::default(),
            trace_name: "lpi2c".into(),
        }
    }

    /// Attach a slave (called by the bus funnel, which wraps it for tracing).
    pub fn push_slave(&mut self, dev: Box<dyn I2cDevice>) {
        self.slaves.get_mut().push(dev);
    }

    /// CPU cycles for `bits` SCL periods:
    /// SCL period = (CLKLO + CLKHI + 2 + latency) * 2^PRESCALE / fclk.
    fn bit_cycles(&self, i: &Inner, bits: u64) -> u64 {
        let clklo = (i.mccr0 & 0x3F) as u64;
        let clkhi = ((i.mccr0 >> 8) & 0x3F) as u64;
        let prescale = 1u64 << (i.mcfgr[1] & 0x7);
        let period_fclk = (clklo + clkhi + 2 + 1).max(4) * prescale;
        (bits * period_fclk * self.time.cpu_hz() / self.clk_hz).max(1)
    }

    fn trace_byte(&self, kind: I2cSym, byte: u8, ack: bool) {
        self.trace
            .push(&self.trace_name, BusPayload::I2c { kind, byte, ack });
    }

    /// Execute commands whose wire time has elapsed, up to `now`. A command
    /// is issued when the engine is free and its effect (ACK/NACK, byte
    /// delivered or received, STOP) lands when its wire time ends.
    fn sync(&self) {
        let now = self.time.now();
        let mut i = self.inner.borrow_mut();
        let mut slaves = self.slaves.borrow_mut();
        for _ in 0..256 {
            if let Some((cmd, t)) = i.inflight {
                if now < t {
                    return;
                }
                i.inflight = None;
                i.synced = t;
                self.complete(&mut i, &mut slaves, cmd);
                continue;
            }
            if i.mcr & MCR_MEN == 0 {
                i.synced = now;
                return;
            }
            let start = i.synced;
            // A receive in progress clocks one byte per 9 bits into the RX FIFO.
            if let Some((_, keep)) = i.receiving {
                if keep && i.rx.len() >= FIFO_DEPTH {
                    i.synced = now;
                    return; // stall: SCL held low until the FIFO drains
                }
                i.inflight = Some((RECV_BYTE, start + self.bit_cycles(&i, 9)));
                continue;
            }
            if i.msr & MSR_NDF != 0 {
                i.synced = now;
                return; // no new transfer until NDF is cleared
            }
            let Some(cmd) = i.tx.pop_front() else {
                i.synced = now;
                return;
            };
            let bits = match (cmd >> 8) & 0x7 {
                0b100..=0b111 => {
                    if i.bus_busy {
                        // Repeated START ends the previous packet.
                        i.msr |= MSR_EPF;
                    }
                    i.bus_busy = true;
                    10
                }
                // Transmit/receive without a START: FIFO error, command dropped
                // (RM MSR[FEF]); the master does not drive the bus.
                0b000 | 0b001 | 0b011 if !i.bus_busy => {
                    i.msr |= MSR_FEF;
                    continue;
                }
                0b000 => 9,
                0b001 | 0b011 => {
                    let keep = (cmd >> 8) & 0x7 == 0b001;
                    i.receiving = Some(((cmd & 0xFF) + 1, keep));
                    continue;
                }
                _ => 2, // STOP
            };
            i.inflight = Some((cmd, start + self.bit_cycles(&i, bits)));
        }
    }

    /// The end-of-wire-time effect of one command.
    fn complete(&self, i: &mut Inner, slaves: &mut [Box<dyn I2cDevice>], cmd: u16) {
        let data = (cmd & 0xFF) as u8;
        if cmd == RECV_BYTE {
            let Some((left, keep)) = i.receiving else {
                return;
            };
            let byte = match i.selected {
                Some((idx, Dir::Read)) => slaves.get_mut(idx).map(|d| d.read()),
                _ => None,
            };
            // A byte from a slave is traced by the slave's bus-trace wrapper.
            // With no slave the bus floats high; only this model can say so.
            // The master ACKs every byte but the last of the command.
            let byte = byte.unwrap_or_else(|| {
                self.trace_byte(I2cSym::Data, 0xFF, left > 1);
                0xFF
            });
            if keep {
                i.rx.push_back(byte);
            }
            i.receiving = if left > 1 {
                Some((left - 1, keep))
            } else {
                None
            };
            return;
        }
        match (cmd >> 8) & 0x7 {
            0b100..=0b111 => {
                let expect_nack = (cmd >> 8) & 0x1 == 1;
                let addr = data >> 1;
                let dir = if data & 1 != 0 { Dir::Read } else { Dir::Write };
                let idx = slaves.iter().position(|d| d.claims_address(addr));
                let acked = idx.is_some();
                match idx {
                    Some(n) => {
                        slaves[n].select_address(addr);
                        slaves[n].start();
                        i.selected = Some((n, dir));
                    }
                    None => i.selected = None,
                }
                // An acknowledged transfer is traced by the slave's bus-trace
                // wrapper (`bus::bus_trace::wrap_i2c`), the one tracer of
                // every attached slave. This model traces only what no slave
                // sees: an address nobody acknowledged. Tracing both would put
                // every byte in the ring twice.
                if !acked {
                    self.trace_byte(
                        if dir == Dir::Read {
                            I2cSym::AddrRead
                        } else {
                            I2cSym::AddrWrite
                        },
                        data,
                        false,
                    );
                }
                if acked == expect_nack && i.mcfgr[1] & MCFGR1_IGNACK == 0 {
                    self.nack(i, slaves);
                }
            }
            0b000 => {
                let acked = match i.selected {
                    Some((idx, Dir::Write)) => {
                        slaves[idx].write(data);
                        true
                    }
                    _ => false,
                };
                if !acked {
                    self.trace_byte(I2cSym::Data, data, false);
                }
                if !acked && i.mcfgr[1] & MCFGR1_IGNACK == 0 {
                    self.nack(i, slaves);
                }
            }
            _ => {
                // STOP
                if let Some((idx, _)) = i.selected.take() {
                    slaves[idx].stop();
                }
                if i.bus_busy {
                    i.msr |= MSR_SDF | MSR_EPF;
                }
                i.bus_busy = false;
            }
        }
    }

    /// NACK handling: flag NDF, drive STOP, release the slave.
    fn nack(&self, i: &mut Inner, slaves: &mut [Box<dyn I2cDevice>]) {
        i.msr |= MSR_NDF;
        if let Some((idx, _)) = i.selected.take() {
            slaves[idx].stop();
        }
        if i.bus_busy {
            i.msr |= MSR_SDF | MSR_EPF;
        }
        i.bus_busy = false;
    }

    pub fn read_reg(&self, off: u32) -> u32 {
        self.sync();
        let mut i = self.inner.borrow_mut();
        match off & !3 {
            VERID => 0x0100_0003,
            PARAM => 0x0000_0202,
            MCR => i.mcr,
            MSR => i.msr_view(),
            MIER => i.mier,
            MDER => i.mder,
            MCFGR0 => i.mcfgr[0],
            MCFGR1 => i.mcfgr[1],
            MCFGR2 => i.mcfgr[2],
            MCFGR3 => i.mcfgr[3],
            MDMR => i.mdmr,
            MCCR0 => i.mccr0,
            MCCR1 => i.mccr1,
            MFCR => i.mfcr,
            MFSR => (i.tx.len() as u32) | ((i.rx.len() as u32) << 16),
            MRDR => match i.rx.pop_front() {
                Some(b) => b as u32,
                None => 1 << 14, // RXEMPTY
            },
            0x150 => 0x4000, // SASR (slave unused)
            0x170 => 0x4000, // SRDR (slave unused)
            _ => 0,
        }
    }

    pub fn write_reg(&mut self, off: u32, value: u32, mask: u32) {
        self.sync();
        {
            let mut i = self.inner.borrow_mut();
            let v = value & mask;
            let merge = |old: u32| (old & !mask) | v;
            match off & !3 {
                MCR => {
                    let new = merge(i.mcr);
                    if new & MCR_RST != 0 {
                        let keep = new & MCR_RST;
                        *i = Inner::new();
                        i.mcr = keep;
                    } else {
                        i.mcr = new & 0x0F;
                    }
                    if v & MCR_RTF != 0 {
                        i.tx.clear();
                    }
                    if v & MCR_RRF != 0 {
                        i.rx.clear();
                    }
                }
                MSR => i.msr &= !(v & MSR_W1C),
                MIER => i.mier = merge(i.mier) & 0x7F03,
                MDER => i.mder = merge(i.mder) & 0x3,
                MCFGR0 => i.mcfgr[0] = merge(i.mcfgr[0]),
                MCFGR1 => i.mcfgr[1] = merge(i.mcfgr[1]),
                MCFGR2 => i.mcfgr[2] = merge(i.mcfgr[2]),
                MCFGR3 => i.mcfgr[3] = merge(i.mcfgr[3]),
                MDMR => i.mdmr = merge(i.mdmr),
                MCCR0 => i.mccr0 = merge(i.mccr0),
                MCCR1 => i.mccr1 = merge(i.mccr1),
                MFCR => i.mfcr = merge(i.mfcr) & 0x0003_0003,
                MTDR if i.tx.len() < FIFO_DEPTH => {
                    i.tx.push_back((v & 0x7FF) as u16);
                }
                _ => {}
            }
        }
        self.sync();
    }

    fn irq(&self) -> bool {
        self.sync();
        let i = self.inner.borrow();
        i.msr_view() & i.mier & 0x7F03 != 0
    }

    /// DMA request line 0: transmit (TDDE & TDF) or receive (RDDE & RDF).
    fn dma_line(&self, line: u8) -> bool {
        self.sync();
        let i = self.inner.borrow();
        // One DMAMUX source per LPI2C instance carries both directions.
        let tx = i.mder & 1 != 0 && i.tdf() && i.mcr & MCR_MEN != 0;
        let rx = i.mder & 2 != 0 && i.rdf();
        match line {
            0 => tx || rx,
            _ => false,
        }
    }
}

impl ImxrtLpi2c {
    /// Recompute the cached interrupt line (see `Timebase::level`).
    fn refresh_irq(&self) {
        self.time.set_level(self.irq());
    }

    fn tick_inner(&mut self, cycles: u64) -> PeripheralTickResult {
        self.time.advance(cycles);
        self.sync();
        let i = self.inner.borrow();
        let until = match i.inflight {
            Some((_, t)) => Some(t),
            None if i.receiving.is_some() || (!i.tx.is_empty() && i.msr & MSR_NDF == 0) => {
                Some(self.time.now() + 1)
            }
            None => None,
        };
        super::wake_hint(self.time.now(), until)
    }
}

impl Peripheral for ImxrtLpi2c {
    /// Walked only while timed work is in flight or the interrupt line is
    /// asserted (so its deassert is reconciled); MMIO re-arms it.
    fn legacy_tick_active(&self) -> bool {
        ({
            let i = self.inner.borrow();
            i.inflight.is_some() || i.receiving.is_some() || !i.tx.is_empty()
        }) || self.time.level()
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let v = byte_of(self.read_reg(offset as u32), offset);
        self.refresh_irq();
        Ok(v)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset & 3) * 8;
        self.write_reg(offset as u32, (value as u32) << shift, 0xFF << shift);
        self.refresh_irq();
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let v = self.read_reg(offset as u32);
        self.refresh_irq();
        Ok(v)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset as u32, value, u32::MAX);
        self.refresh_irq();
        Ok(())
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        let off = offset as u32 & !3;
        if off == MRDR {
            let i = self.inner.borrow();
            return Some(byte_of(
                i.rx.front().map(|&b| b as u32).unwrap_or(1 << 14),
                offset,
            ));
        }
        Some(byte_of(self.read_reg(off), offset))
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        let r = self.tick_inner(cycles);
        self.refresh_irq();
        r
    }
    fn dma_request_active(&self, line: u8) -> bool {
        self.dma_line(line)
    }
    fn irq_line_level(&self) -> Option<bool> {
        Some(self.time.level())
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.time.attach_clock(clock);
    }
    fn attach_cpu_hz(&mut self, hz: u64) {
        self.time.attach_cpu_hz(hz);
    }
    /// The attached slaves, for `inspect` and for device logs.
    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for dev in self.slaves.borrow().iter() {
            crate::inspect::visit_i2c_device(&**dev, f);
        }
    }
    fn attach_bus_trace(&mut self, name: &str, trace: &BusTrace) {
        self.trace = trace.clone();
        self.trace_name = name.to_string();
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        let i = self.inner.borrow();
        serde_json::json!({
            "peripheral": "imxrt_lpi2c",
            "mcr": i.mcr,
            "msr": i.msr_view(),
            "slaves": self.slaves.borrow().len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CycleClock;

    struct Echo {
        addr: u8,
        last: u8,
        log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }
    impl I2cDevice for Echo {
        fn address(&self) -> u8 {
            self.addr
        }
        fn read(&mut self) -> u8 {
            self.log.lock().unwrap().push("R".into());
            self.last
        }
        fn write(&mut self, d: u8) {
            self.log.lock().unwrap().push(format!("W{d:02x}"));
            self.last = d;
        }
        fn start(&mut self) {
            self.log.lock().unwrap().push("S".into());
        }
        fn stop(&mut self) {
            self.log.lock().unwrap().push("P".into());
        }
    }

    fn master() -> (ImxrtLpi2c, CycleClock) {
        let mut m = ImxrtLpi2c::default();
        let c = CycleClock::default();
        m.attach_cycle_clock(c.clone());
        // SDK LPI2C_MasterInit @ 100 kHz-ish: prescale 2^1, CLKLO/HI.
        m.write_reg(MCFGR1, 1, u32::MAX);
        m.write_reg(MCCR0, (0x1D) | (0x13 << 8), u32::MAX);
        m.write_reg(MCR, MCR_MEN, u32::MAX);
        (m, c)
    }

    #[test]
    fn no_device_at_address_sets_ndf_and_stops() {
        let (mut m, c) = master();
        // START + 0x54 write, data 0x00, STOP
        m.write_reg(MTDR, (0b100 << 8) | (0x54 << 1), u32::MAX);
        assert_ne!(
            m.read_reg(MSR) & MSR_MBF,
            0,
            "busy while the address clocks out"
        );
        assert_eq!(m.read_reg(MSR) & MSR_NDF, 0);
        c.publish(1_000_000);
        let msr = m.read_reg(MSR);
        assert_ne!(msr & MSR_NDF, 0, "NACK detected");
        assert_ne!(msr & MSR_SDF, 0, "master drove STOP");
        assert_eq!(msr & MSR_BBF, 0, "bus released");
        // Commands queued behind a NACK wait for NDF to clear.
        m.write_reg(MTDR, 0x12, u32::MAX);
        c.publish(2_000_000);
        assert_eq!(m.read_reg(MFSR) & 0x7, 1);
        m.write_reg(MSR, MSR_NDF, u32::MAX);
        assert_eq!(m.read_reg(MSR) & MSR_NDF, 0);
    }

    /// Every byte is in the bus trace once: the slave's trace wrapper
    /// records acknowledged traffic, the controller only a NACK.
    #[test]
    fn bus_trace_has_each_byte_once() {
        let (mut m, c) = master();
        let trace = BusTrace::default();
        m.attach_bus_trace("lpi2c1", &trace);
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        m.push_slave(crate::bus::bus_trace::wrap_i2c(
            "lpi2c1",
            &trace,
            Box::new(Echo {
                addr: 0x54,
                last: 0,
                log,
            }),
        ));
        // Write 0x00 0x58 to 0x54, then address 0x1B (nobody there).
        m.write_reg(MTDR, (0b100 << 8) | (0x54 << 1), u32::MAX);
        m.write_reg(MTDR, 0x00, u32::MAX);
        m.write_reg(MTDR, 0x58, u32::MAX);
        m.write_reg(MTDR, 0b010 << 8, u32::MAX);
        c.publish(10_000_000);
        m.write_reg(MTDR, (0b100 << 8) | (0x1B << 1), u32::MAX);
        c.publish(20_000_000);
        m.read_reg(MSR);
        let lines: Vec<String> = trace
            .snapshot()
            .iter()
            .map(|e| e.payload.to_string())
            .collect();
        assert_eq!(
            lines,
            [
                "addr 0x54 W ack",
                "data 0x00 ack",
                "data 0x58 ack",
                "addr 0x1b W nack"
            ]
        );
    }

    #[test]
    fn write_then_read_transaction_with_a_slave() {
        let (mut m, c) = master();
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        m.push_slave(Box::new(Echo {
            addr: 0x54,
            last: 0,
            log: log.clone(),
        }));
        m.write_reg(MTDR, (0b100 << 8) | (0x54 << 1), u32::MAX);
        m.write_reg(MTDR, 0xA5, u32::MAX);
        m.write_reg(MTDR, (0b100 << 8) | (0x54 << 1) | 1, u32::MAX);
        m.write_reg(MTDR, 0b001 << 8, u32::MAX); // receive 1 byte
        c.publish(10_000_000);
        m.write_reg(MTDR, 0b010 << 8, u32::MAX); // STOP
        c.publish(20_000_000);
        assert_eq!(m.read_reg(MRDR), 0xA5);
        assert_eq!(m.read_reg(MRDR), 1 << 14, "RX empty");
        let msr = m.read_reg(MSR);
        assert_eq!(msr & MSR_NDF, 0);
        assert_ne!(msr & MSR_SDF, 0);
        assert_eq!(*log.lock().unwrap(), vec!["S", "Wa5", "S", "R", "P"]);
    }
}
