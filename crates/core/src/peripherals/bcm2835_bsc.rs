// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Broadcom Serial Controller (BSC), the I²C master of BCM2835/2836/2837/2711.
//!
//! Register map and status semantics follow the BCM2835 ARM Peripherals
//! datasheet, section 3 "BSC", and the way the Linux `i2c-bcm2835` driver
//! uses them (interrupt driven: INTT for TX, INTR for RX, INTD for done).
//!
//! # Timing
//!
//! The model is timed in nanoseconds of the caller's clock. Every access
//! carries the current time; [`Bcm2835Bsc::advance_to`] runs the bus up to
//! that time and [`Bcm2835Bsc::next_deadline`] tells the caller when the
//! next visible state change happens. The model never runs ahead of the
//! time it is given.
//!
//! SCL period `T = DIV / core_clk` (DIV = 0 means 32768; the LSB is ignored,
//! as the datasheet says). A transfer costs, in periods of T:
//!
//! * START (or repeated START): 1
//! * every byte, address byte included: 9 (8 bits + ACK)
//! * STOP: 1
//!
//! An address NACK is seen at the end of the address byte (ERR sets), then
//! STOP follows and DONE sets. A write whose TX FIFO is empty when the next
//! byte is due, or a read whose RX FIFO is full, holds the bus (SCL low)
//! until the CPU refills / drains the FIFO; timing restarts from that access.
//! Event times are computed from the period count since the last restart
//! and rounded up to whole nanoseconds, so long transfers do not drift.
//!
//! A START written while a transfer is active is latched. It runs as a
//! repeated START when the current transfer has moved its last byte (the
//! Linux driver does this for write-then-read), or as a new START after STOP
//! if it arrived only during STOP (a slow CPU).
//!
//! Not modelled: clock stretching by the target (S.CLKT never sets), NACK
//! on data bytes, FEDL/REDL (DEL) sub-period delays, 10-bit addressing.

use std::collections::VecDeque;

use crate::peripherals::device::{I2cDevice, I2cSlaveHost};

pub const REG_C: u64 = 0x00;
pub const REG_S: u64 = 0x04;
pub const REG_DLEN: u64 = 0x08;
pub const REG_A: u64 = 0x0c;
pub const REG_FIFO: u64 = 0x10;
pub const REG_DIV: u64 = 0x14;
pub const REG_DEL: u64 = 0x18;
pub const REG_CLKT: u64 = 0x1c;

pub const C_READ: u32 = 1 << 0;
pub const C_CLEAR: u32 = 0b11 << 4;
pub const C_ST: u32 = 1 << 7;
pub const C_INTD: u32 = 1 << 8;
pub const C_INTT: u32 = 1 << 9;
pub const C_INTR: u32 = 1 << 10;
pub const C_I2CEN: u32 = 1 << 15;

pub const S_TA: u32 = 1 << 0;
pub const S_DONE: u32 = 1 << 1;
pub const S_TXW: u32 = 1 << 2;
pub const S_RXR: u32 = 1 << 3;
pub const S_TXD: u32 = 1 << 4;
pub const S_RXD: u32 = 1 << 5;
pub const S_TXE: u32 = 1 << 6;
pub const S_RXF: u32 = 1 << 7;
pub const S_ERR: u32 = 1 << 8;
pub const S_CLKT: u32 = 1 << 9;

/// Core clock of the Raspberry Pi 4 guest as Linux reports it: the BSC's
/// parent clock `vpu` in `/sys/kernel/debug/clk/clk_summary` under QEMU
/// raspi4b, and the driver writes DIV = 3500 for a 100 kHz bus.
pub const DEFAULT_CORE_CLK_HZ: u64 = 350_000_000;

const FIFO_DEPTH: usize = 16;
/// C bits that are stored (ST and CLEAR are actions, they read back as 0).
const C_STORED: u32 = C_READ | C_INTD | C_INTT | C_INTR | C_I2CEN;

pub const PERIODS_START: u64 = 1;
pub const PERIODS_BYTE: u64 = 9;
pub const PERIODS_STOP: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BusEvent {
    /// End of the address byte: ACK/NACK known.
    Address,
    /// End of a data byte.
    Byte,
    /// End of STOP: DONE.
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Start {
    addr: u8,
    len: u32,
    read: bool,
}

#[derive(Debug)]
struct Xfer {
    addr: u8,
    read: bool,
    /// Bytes not yet completed on the bus (the byte in flight included).
    remaining: u32,
    in_flight: Option<u8>,
    target: Option<usize>,
}

pub struct Bcm2835Bsc {
    core_clk_hz: u64,
    now_ns: u64,
    c: u32,
    /// Sticky status bits: DONE, ERR, CLKT. The rest is derived.
    sticky: u32,
    dlen_reload: u32,
    a: u32,
    div: u32,
    del: u32,
    clkt: u32,
    xfer: Option<Xfer>,
    pending_start: Option<Start>,
    /// Bus timing reference: event times are `ref_ns + periods * T`.
    ref_ns: u64,
    periods: u64,
    event: Option<(u64, BusEvent)>,
    tx: VecDeque<u8>,
    rx: VecDeque<u8>,
    targets: Vec<Box<dyn I2cDevice>>,
}

impl std::fmt::Debug for Bcm2835Bsc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bcm2835Bsc")
            .field("now_ns", &self.now_ns)
            .field("c", &format_args!("{:#x}", self.c))
            .field("s", &format_args!("{:#x}", self.status()))
            .field("a", &format_args!("{:#x}", self.a))
            .field("xfer", &self.xfer)
            .field("event", &self.event)
            .field("targets", &self.targets.len())
            .finish()
    }
}

impl Default for Bcm2835Bsc {
    fn default() -> Self {
        Self::new(DEFAULT_CORE_CLK_HZ)
    }
}

impl Bcm2835Bsc {
    pub fn new(core_clk_hz: u64) -> Self {
        assert!(core_clk_hz > 0, "core clock must be non-zero");
        Self {
            core_clk_hz,
            now_ns: 0,
            c: 0,
            sticky: 0,
            dlen_reload: 0,
            a: 0,
            // Reset values from the datasheet.
            div: 0x5dc,
            del: 0x0030_0030,
            clkt: 0x40,
            xfer: None,
            pending_start: None,
            ref_ns: 0,
            periods: 0,
            event: None,
            tx: VecDeque::new(),
            rx: VecDeque::new(),
            targets: Vec::new(),
        }
    }

    pub fn core_clk_hz(&self) -> u64 {
        self.core_clk_hz
    }

    pub fn now_ns(&self) -> u64 {
        self.now_ns
    }

    /// Effective clock divider: DIV = 0 means 32768, the LSB is ignored.
    pub fn divider(&self) -> u64 {
        match self.div & 0xfffe {
            0 => 32768,
            d => d as u64,
        }
    }

    /// Nanoseconds for `periods` SCL periods at the current DIV, rounded up.
    pub fn periods_ns(&self, periods: u64) -> u64 {
        let num = periods as u128 * self.divider() as u128 * 1_000_000_000u128;
        num.div_ceil(self.core_clk_hz as u128) as u64
    }

    /// Time of the next internal state change, if a transfer is moving.
    pub fn next_deadline(&self) -> Option<u64> {
        self.event.map(|(t, _)| t)
    }

    /// Run the bus up to `t_ns`. Time never goes backwards.
    pub fn advance_to(&mut self, t_ns: u64) {
        assert!(
            t_ns >= self.now_ns,
            "BSC time went backwards: {} -> {}",
            self.now_ns,
            t_ns
        );
        while let Some((et, step)) = self.event {
            if et > t_ns {
                break;
            }
            self.event = None;
            self.now_ns = et;
            self.on_step(step);
        }
        self.now_ns = t_ns;
    }

    fn active(&self) -> bool {
        self.xfer.is_some()
    }

    /// The S register as the CPU reads it.
    pub fn status(&self) -> u32 {
        let mut s = self.sticky;
        if self.tx.len() < FIFO_DEPTH {
            s |= S_TXD;
        }
        if self.tx.is_empty() {
            s |= S_TXE;
        }
        if !self.rx.is_empty() {
            s |= S_RXD;
        }
        if self.rx.len() == FIFO_DEPTH {
            s |= S_RXF;
        }
        if let Some(x) = &self.xfer {
            s |= S_TA;
            // TXW: a write is underway and the FIFO does not yet hold all
            // the data still to send.
            let needed = x.remaining - x.in_flight.is_some() as u32;
            if !x.read && self.tx.len() < FIFO_DEPTH && (self.tx.len() as u32) < needed {
                s |= S_TXW;
            }
            // RXR: a read is underway and the FIFO is at least 3/4 full.
            if x.read && self.rx.len() >= FIFO_DEPTH * 3 / 4 {
                s |= S_RXR;
            }
        }
        s
    }

    /// Level of the BSC interrupt output.
    pub fn irq_pending(&self) -> bool {
        let s = self.status();
        (self.c & C_INTD != 0 && s & S_DONE != 0)
            || (self.c & C_INTT != 0 && s & S_TXW != 0)
            || (self.c & C_INTR != 0 && s & S_RXR != 0)
    }

    /// 32-bit register read at time `t_ns`. FIFO reads pop the RX FIFO.
    pub fn read_reg(&mut self, t_ns: u64, offset: u64) -> u32 {
        self.advance_to(t_ns);
        match offset {
            REG_C => self.c,
            REG_S => self.status(),
            REG_DLEN => match &self.xfer {
                Some(x) => x.remaining,
                None => self.dlen_reload,
            },
            REG_A => self.a,
            REG_FIFO => {
                let v = self.rx.pop_front().unwrap_or(0) as u32;
                self.resume_if_stalled();
                v
            }
            REG_DIV => self.div,
            REG_DEL => self.del,
            REG_CLKT => self.clkt,
            _ => 0,
        }
    }

    /// 32-bit register write at time `t_ns`.
    pub fn write_reg(&mut self, t_ns: u64, offset: u64, value: u32) {
        self.advance_to(t_ns);
        match offset {
            REG_C => {
                self.c = value & C_STORED;
                if value & C_CLEAR != 0 {
                    self.tx.clear();
                    self.rx.clear();
                }
                if value & C_ST != 0 && value & C_I2CEN != 0 {
                    let start = Start {
                        addr: (self.a & 0x7f) as u8,
                        len: self.dlen_reload,
                        read: value & C_READ != 0,
                    };
                    if self.active() {
                        self.pending_start = Some(start);
                    } else {
                        self.begin(start);
                    }
                }
            }
            REG_S => {
                // DONE, ERR and CLKT are write-1-to-clear.
                self.sticky &= !(value & (S_DONE | S_ERR | S_CLKT));
            }
            REG_DLEN => self.dlen_reload = value & 0xffff,
            REG_A => self.a = value & 0x7f,
            REG_FIFO => {
                if self.tx.len() < FIFO_DEPTH {
                    self.tx.push_back(value as u8);
                }
                self.resume_if_stalled();
            }
            REG_DIV => self.div = value & 0xffff,
            REG_DEL => self.del = value,
            REG_CLKT => self.clkt = value & 0xffff,
            _ => {}
        }
    }

    fn schedule(&mut self, periods: u64, step: BusEvent) {
        self.periods += periods;
        let t = self.ref_ns + self.periods_ns(self.periods);
        self.event = Some((t, step));
    }

    /// START on an idle bus.
    fn begin(&mut self, start: Start) {
        self.sticky &= !S_DONE;
        self.ref_ns = self.now_ns;
        self.periods = 0;
        self.open(start, PERIODS_START);
    }

    /// Put a (repeated) START and the address byte on the bus.
    fn open(&mut self, start: Start, start_periods: u64) {
        self.xfer = Some(Xfer {
            addr: start.addr,
            read: start.read,
            remaining: start.len,
            in_flight: None,
            target: None,
        });
        self.schedule(start_periods + PERIODS_BYTE, BusEvent::Address);
    }

    fn on_step(&mut self, step: BusEvent) {
        match step {
            BusEvent::Address => {
                let addr = self.xfer.as_ref().expect("address without transfer").addr;
                match self.targets.iter().position(|t| t.address() == addr) {
                    Some(i) => {
                        self.targets[i].start();
                        self.xfer.as_mut().unwrap().target = Some(i);
                        self.next_byte();
                    }
                    None => {
                        // Address NACK: ERR now, then STOP. A latched
                        // repeated start is abandoned with the transfer.
                        self.sticky |= S_ERR;
                        self.pending_start = None;
                        self.tx.clear();
                        self.schedule(PERIODS_STOP, BusEvent::Stop);
                    }
                }
            }
            BusEvent::Byte => {
                let x = self.xfer.as_mut().expect("byte without transfer");
                let i = x.target.expect("byte without target");
                if x.read {
                    let b = self.targets[i].read();
                    self.rx.push_back(b);
                } else {
                    let b = x.in_flight.take().expect("write byte not in flight");
                    self.targets[i].write(b);
                }
                let x = self.xfer.as_mut().unwrap();
                x.remaining -= 1;
                self.next_byte();
            }
            BusEvent::Stop => {
                if let Some(i) = self.xfer.take().and_then(|x| x.target) {
                    self.targets[i].stop();
                }
                self.sticky |= S_DONE;
                // A START that arrived during STOP runs once the bus is free.
                if let Some(start) = self.pending_start.take() {
                    self.begin(start);
                }
            }
        }
    }

    /// After the address or a data byte: next byte, repeated START, STOP or
    /// stall on the FIFO.
    fn next_byte(&mut self) {
        let x = self.xfer.as_mut().expect("next byte without transfer");
        if x.remaining == 0 {
            match self.pending_start.take() {
                Some(start) => self.open(start, PERIODS_START),
                None => self.schedule(PERIODS_STOP, BusEvent::Stop),
            }
            return;
        }
        if x.read {
            if self.rx.len() < FIFO_DEPTH {
                self.schedule(PERIODS_BYTE, BusEvent::Byte);
            }
        } else if let Some(b) = self.tx.pop_front() {
            x.in_flight = Some(b);
            self.schedule(PERIODS_BYTE, BusEvent::Byte);
        }
        // else: bus held until the CPU services the FIFO.
    }

    /// A FIFO access may release a bus held on a full/empty FIFO.
    fn resume_if_stalled(&mut self) {
        let stalled = self.event.is_none()
            && self
                .xfer
                .as_ref()
                .is_some_and(|x| x.target.is_some() && x.remaining > 0);
        if stalled {
            self.ref_ns = self.now_ns;
            self.periods = 0;
            self.next_byte();
        }
    }
}

impl I2cSlaveHost for Bcm2835Bsc {
    fn attach_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.targets.push(device);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::components::bme280::Bme280;

    /// DIV the Linux driver writes for 100 kHz on a 350 MHz core clock.
    const DIV_100K: u32 = 3500;
    /// One SCL period at that DIV: 3500 / 350 MHz = 10 us.
    const T: u64 = 10_000;
    const T0: u64 = 1_000_000;

    fn bsc(with_bme280: bool) -> Bcm2835Bsc {
        let mut bsc = Bcm2835Bsc::new(DEFAULT_CORE_CLK_HZ);
        if with_bme280 {
            bsc.attach_slave(Box::new(Bme280::new(0x76)));
        }
        bsc.write_reg(0, REG_DIV, DIV_100K);
        bsc
    }

    fn start(bsc: &mut Bcm2835Bsc, t: u64, addr: u32, len: u32, read: bool, last: bool) {
        let mut c = C_ST | C_I2CEN | if read { C_READ | C_INTR } else { C_INTT };
        if last {
            c |= C_INTD;
        }
        bsc.write_reg(t, REG_A, addr);
        bsc.write_reg(t, REG_DLEN, len);
        bsc.write_reg(t, REG_C, c);
    }

    fn s(bsc: &mut Bcm2835Bsc, t: u64) -> u32 {
        bsc.read_reg(t, REG_S)
    }

    #[test]
    fn bsc_reset_values() {
        let mut bsc = Bcm2835Bsc::default();
        assert_eq!(bsc.read_reg(0, REG_S), S_TXD | S_TXE);
        assert_eq!(bsc.read_reg(0, REG_DIV), 0x5dc);
        assert_eq!(bsc.read_reg(0, REG_DEL), 0x0030_0030);
        assert_eq!(bsc.read_reg(0, REG_CLKT), 0x40);
        assert!(!bsc.irq_pending());
        assert_eq!(bsc.next_deadline(), None);
    }

    #[test]
    fn bsc_scl_period_from_div() {
        let mut bsc = bsc(false);
        assert_eq!(bsc.periods_ns(1), T);
        bsc.write_reg(0, REG_DIV, 0);
        assert_eq!(bsc.divider(), 32768);
        bsc.write_reg(0, REG_DIV, 3501);
        assert_eq!(bsc.divider(), 3500, "LSB ignored");
    }

    #[test]
    fn bsc_status_right_after_start() {
        let mut bsc = bsc(true);
        start(&mut bsc, T0, 0x76, 0, false, true);
        let st = s(&mut bsc, T0);
        assert_eq!(st & (S_TA | S_DONE), S_TA, "TA=1, DONE=0 right after START");
        assert!(!bsc.irq_pending());
        assert_eq!(bsc.next_deadline(), Some(T0 + 10 * T), "address byte ends");
    }

    #[test]
    fn bsc_done_and_irq_at_exact_time() {
        let mut bsc = bsc(true);
        start(&mut bsc, T0, 0x76, 0, false, true);
        // START + address byte + STOP = 11 periods.
        let done = T0 + (PERIODS_START + PERIODS_BYTE + PERIODS_STOP) * T;
        assert_eq!(s(&mut bsc, done - 1) & (S_TA | S_DONE), S_TA);
        assert!(!bsc.irq_pending());
        assert_eq!(bsc.next_deadline(), Some(done));
        assert_eq!(s(&mut bsc, done) & (S_TA | S_DONE | S_ERR), S_DONE);
        assert!(bsc.irq_pending());
        assert_eq!(bsc.next_deadline(), None);
    }

    #[test]
    fn bsc_nack_at_address_byte_time() {
        let mut bsc = bsc(true);
        start(&mut bsc, T0, 0x77, 0, false, true);
        let addr_end = T0 + (PERIODS_START + PERIODS_BYTE) * T;
        assert_eq!(s(&mut bsc, addr_end - 1) & S_ERR, 0);
        let st = s(&mut bsc, addr_end);
        assert_eq!(
            st & (S_ERR | S_TA | S_DONE),
            S_ERR | S_TA,
            "ERR, STOP pending"
        );
        let st = s(&mut bsc, addr_end + PERIODS_STOP * T);
        assert_eq!(st & (S_ERR | S_TA | S_DONE), S_ERR | S_DONE);
        assert!(bsc.irq_pending());
        bsc.write_reg(addr_end + 20 * T, REG_C, C_CLEAR);
        bsc.write_reg(addr_end + 20 * T, REG_S, S_CLKT | S_ERR | S_DONE);
        assert!(!bsc.irq_pending());
    }

    /// The exact sequence `i2cget -y 1 0x76 0xd0` drives through the Linux
    /// driver: write msg (INTT); its TXW interrupt fills the FIFO and latches
    /// the read msg (INTR|INTD), which runs as a repeated START.
    #[test]
    fn bsc_i2cget_chip_id_timing() {
        let mut bsc = bsc(true);
        start(&mut bsc, T0, 0x76, 1, false, false);
        assert_ne!(s(&mut bsc, T0) & S_TXW, 0);
        assert!(bsc.irq_pending(), "INTT raises TXW at START");
        // ISR, a few us later: fill FIFO, latch the read.
        let isr = T0 + 3_000;
        bsc.write_reg(isr, REG_FIFO, 0xd0);
        start(&mut bsc, isr, 0x76, 1, true, true);
        assert!(
            !bsc.irq_pending(),
            "FIFO holds the data, read still latched"
        );
        // START + addr + data + Sr + addr + data + STOP = 39 periods.
        let done = T0 + 39 * T;
        assert_eq!(s(&mut bsc, done - 1) & (S_TA | S_DONE), S_TA);
        assert_eq!(
            s(&mut bsc, done) & (S_TA | S_DONE | S_ERR | S_RXD),
            S_DONE | S_RXD
        );
        assert!(bsc.irq_pending());
        assert_eq!(bsc.read_reg(done, REG_FIFO), 0x60, "BME280 chip-id");
    }

    /// A slow CPU latches the read only after the write's last byte, while
    /// STOP is on the bus: the read must still run, as a new START.
    #[test]
    fn bsc_start_during_stop_runs_after_stop() {
        let mut bsc = bsc(true);
        start(&mut bsc, T0, 0x76, 1, false, false);
        bsc.write_reg(T0 + 3_000, REG_FIFO, 0xd0);
        // write done at T0 + 19T; STOP until T0 + 20T.
        let late = T0 + 19 * T + 4_000;
        start(&mut bsc, late, 0x76, 1, true, true);
        let stop_end = T0 + 20 * T;
        let st = s(&mut bsc, stop_end);
        assert_eq!(st & (S_TA | S_DONE), S_TA, "read started, DONE consumed");
        // START + addr + data + STOP from the end of the previous STOP.
        let done = stop_end + 20 * T;
        assert_eq!(s(&mut bsc, done) & (S_TA | S_DONE | S_RXD), S_DONE | S_RXD);
        assert_eq!(bsc.read_reg(done, REG_FIFO), 0x60);
    }

    #[test]
    fn bsc_i2cget_without_target_fails_at_first_address() {
        let mut bsc = bsc(false);
        start(&mut bsc, T0, 0x76, 1, false, false);
        bsc.write_reg(T0 + 3_000, REG_FIFO, 0xd0);
        start(&mut bsc, T0 + 3_000, 0x76, 1, true, true);
        let done = T0 + 11 * T;
        let st = s(&mut bsc, done);
        assert_eq!(st & (S_DONE | S_ERR | S_TA | S_RXD), S_DONE | S_ERR);
        assert!(bsc.irq_pending());
    }

    #[test]
    fn bsc_write_stalls_on_empty_fifo() {
        let mut bsc = bsc(true);
        start(&mut bsc, T0, 0x76, 2, false, true);
        // Address ends at T0+10T; FIFO empty, so the bus holds.
        assert_eq!(s(&mut bsc, T0 + 50 * T) & (S_TA | S_TXW), S_TA | S_TXW);
        assert_eq!(bsc.next_deadline(), None);
        let t1 = T0 + 50 * T;
        bsc.write_reg(t1, REG_FIFO, 0xf4);
        bsc.write_reg(t1, REG_FIFO, 0x27);
        // Two bytes + STOP from the refill.
        assert_eq!(bsc.next_deadline(), Some(t1 + 9 * T));
        assert_ne!(s(&mut bsc, t1 + 19 * T) & S_DONE, 0);
    }

    #[test]
    fn bsc_long_read_uses_rxr_and_refills() {
        let mut bsc = bsc(true);
        start(&mut bsc, T0, 0x76, 1, false, false);
        bsc.write_reg(T0, REG_FIFO, 0x88);
        start(&mut bsc, T0, 0x76, 26, true, true);
        // After 12 data bytes RXR sets.
        // START, addr, 0x88, Sr, addr, first data byte = 38 periods.
        let first_data = T0 + 38 * T;
        let t12 = first_data + 11 * 9 * T;
        assert_eq!(s(&mut bsc, t12 - 1) & S_RXR, 0);
        assert_ne!(s(&mut bsc, t12) & S_RXR, 0);
        assert!(bsc.irq_pending());
        let mut got = Vec::new();
        let mut t = t12;
        while got.len() < 26 {
            while bsc.read_reg(t, REG_S) & S_RXD != 0 {
                got.push(bsc.read_reg(t, REG_FIFO) as u8);
            }
            t += 9 * T;
        }
        assert_eq!(got.len(), 26);
        assert_ne!(s(&mut bsc, t + 10 * T) & S_DONE, 0);
    }
}
