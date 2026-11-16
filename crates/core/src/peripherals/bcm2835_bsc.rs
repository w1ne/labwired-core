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
//! The model is transaction-level: a byte written into the TX FIFO goes out
//! on the bus as soon as a transfer is active, and RX bytes are fetched from
//! the target as long as the RX FIFO has room. There is no bit timing: DIV,
//! DEL and CLKT are stored and read back, but never slow a transfer down, and
//! a target never stretches the clock (S.CLKT never sets).
//!
//! The model is used through [`Bcm2835Bsc::read_reg`] /
//! [`Bcm2835Bsc::write_reg`] (32-bit accesses, as the hardware requires) and
//! [`Bcm2835Bsc::irq_pending`] for the level of the interrupt line.

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

const FIFO_DEPTH: usize = 16;
/// C bits that are stored (ST and CLEAR are actions, they read back as 0).
const C_STORED: u32 = C_READ | C_INTD | C_INTT | C_INTR | C_I2CEN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// No transfer.
    Idle,
    /// ST seen; the address is not on the bus yet (write waits for data).
    Pending,
    /// Address acknowledged, data phase running.
    Data,
}

pub struct Bcm2835Bsc {
    c: u32,
    /// Sticky status bits: DONE, ERR, CLKT. The rest is derived.
    sticky: u32,
    /// Last value written to DLEN.
    dlen_reload: u32,
    /// Bytes still to move in the current transfer.
    remaining: u32,
    a: u32,
    div: u32,
    del: u32,
    clkt: u32,
    phase: Phase,
    reading: bool,
    tx: VecDeque<u8>,
    rx: VecDeque<u8>,
    targets: Vec<Box<dyn I2cDevice>>,
    selected: Option<usize>,
}

impl std::fmt::Debug for Bcm2835Bsc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bcm2835Bsc")
            .field("c", &format_args!("{:#x}", self.c))
            .field("s", &format_args!("{:#x}", self.status()))
            .field("a", &format_args!("{:#x}", self.a))
            .field("remaining", &self.remaining)
            .field("phase", &self.phase)
            .field("targets", &self.targets.len())
            .finish()
    }
}

impl Default for Bcm2835Bsc {
    fn default() -> Self {
        Self::new()
    }
}

impl Bcm2835Bsc {
    pub fn new() -> Self {
        Self {
            c: 0,
            sticky: 0,
            dlen_reload: 0,
            remaining: 0,
            a: 0,
            // Reset values from the datasheet.
            div: 0x5dc,
            del: 0x0030_0030,
            clkt: 0x40,
            phase: Phase::Idle,
            reading: false,
            tx: VecDeque::new(),
            rx: VecDeque::new(),
            targets: Vec::new(),
            selected: None,
        }
    }

    fn active(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// The S register as the CPU reads it.
    pub fn status(&self) -> u32 {
        let mut s = self.sticky;
        if self.active() {
            s |= S_TA;
        }
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
        // TXW: a write is underway and the FIFO does not yet hold enough
        // data to finish it.
        if self.active()
            && !self.reading
            && self.tx.len() < FIFO_DEPTH
            && (self.tx.len() as u32) < self.remaining
        {
            s |= S_TXW;
        }
        // RXR: a read is underway and the FIFO is at least 3/4 full.
        if self.active() && self.reading && self.rx.len() >= FIFO_DEPTH * 3 / 4 {
            s |= S_RXR;
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

    /// 32-bit register read. FIFO reads pop the RX FIFO.
    pub fn read_reg(&mut self, offset: u64) -> u32 {
        match offset {
            REG_C => self.c,
            REG_S => self.status(),
            REG_DLEN => {
                if self.active() {
                    self.remaining
                } else {
                    self.dlen_reload
                }
            }
            REG_A => self.a,
            REG_FIFO => {
                let v = self.rx.pop_front().unwrap_or(0) as u32;
                self.pump();
                v
            }
            REG_DIV => self.div,
            REG_DEL => self.del,
            REG_CLKT => self.clkt,
            _ => 0,
        }
    }

    /// 32-bit register write.
    pub fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            REG_C => {
                self.c = value & C_STORED;
                if value & C_CLEAR != 0 {
                    self.clear_fifos();
                }
                if value & C_ST != 0 && value & C_I2CEN != 0 {
                    self.start();
                }
            }
            REG_S => {
                // DONE, ERR and CLKT are write-1-to-clear.
                self.sticky &= !(value & (S_DONE | S_ERR | S_CLKT));
            }
            REG_DLEN => {
                self.dlen_reload = value & 0xffff;
                if !self.active() {
                    self.remaining = self.dlen_reload;
                }
            }
            REG_A => self.a = value & 0x7f,
            REG_FIFO => {
                if self.tx.len() < FIFO_DEPTH {
                    self.tx.push_back(value as u8);
                }
                self.pump();
            }
            REG_DIV => self.div = value & 0xffff,
            REG_DEL => self.del = value,
            REG_CLKT => self.clkt = value & 0xffff,
            _ => {}
        }
    }

    fn clear_fifos(&mut self) {
        self.tx.clear();
        self.rx.clear();
    }

    /// C.ST: begin a new transfer. A start while a transfer is active is a
    /// repeated start (the driver does this for write-then-read).
    fn start(&mut self) {
        // A new start consumes the previous transfer's completion.
        self.sticky &= !S_DONE;
        self.remaining = self.dlen_reload;
        self.reading = self.c & C_READ != 0;
        self.rx.clear();
        self.phase = Phase::Pending;
        self.pump();
    }

    /// Put the address on the bus. Returns false on NACK.
    fn address_phase(&mut self) -> bool {
        let addr = self.a as u8;
        self.selected = self.targets.iter().position(|t| t.address() == addr);
        match self.selected {
            Some(i) => {
                self.targets[i].start();
                self.phase = Phase::Data;
                true
            }
            None => {
                // Address NACK: the controller sends STOP and finishes.
                self.sticky |= S_ERR;
                self.finish();
                false
            }
        }
    }

    fn finish(&mut self) {
        if let Some(i) = self.selected.take() {
            self.targets[i].stop();
        }
        self.sticky |= S_DONE;
        self.phase = Phase::Idle;
        // On NACK the hardware abandons the TX FIFO content of the transfer.
        if self.sticky & S_ERR != 0 {
            self.tx.clear();
        }
    }

    /// Move bytes between the FIFOs and the target while a transfer is active.
    fn pump(&mut self) {
        if self.phase == Phase::Pending {
            // A write with data waits for the first byte so that a TXW
            // interrupt is seen before the address goes out, as on hardware.
            let ready = self.reading || self.remaining == 0 || !self.tx.is_empty();
            if !ready || !self.address_phase() {
                return;
            }
        }
        if self.phase != Phase::Data {
            return;
        }
        let i = self.selected.expect("data phase without a target");
        if self.reading {
            while self.remaining > 0 && self.rx.len() < FIFO_DEPTH {
                let b = self.targets[i].read();
                self.rx.push_back(b);
                self.remaining -= 1;
            }
        } else {
            while self.remaining > 0 {
                let Some(b) = self.tx.pop_front() else { break };
                self.targets[i].write(b);
                self.remaining -= 1;
            }
        }
        if self.remaining == 0 {
            self.finish();
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

    fn bsc_with_bme280() -> Bcm2835Bsc {
        let mut bsc = Bcm2835Bsc::new();
        bsc.attach_slave(Box::new(Bme280::new(0x76)));
        bsc
    }

    fn start(bsc: &mut Bcm2835Bsc, addr: u32, len: u32, read: bool, last: bool) {
        let mut c = C_ST | C_I2CEN | if read { C_READ | C_INTR } else { C_INTT };
        if last {
            c |= C_INTD;
        }
        bsc.write_reg(REG_A, addr);
        bsc.write_reg(REG_DLEN, len);
        bsc.write_reg(REG_C, c);
    }

    /// The driver's completion path: clear and acknowledge.
    fn complete(bsc: &mut Bcm2835Bsc) {
        bsc.write_reg(REG_C, C_CLEAR);
        bsc.write_reg(REG_S, S_CLKT | S_ERR | S_DONE);
    }

    #[test]
    fn bsc_reset_values() {
        let mut bsc = Bcm2835Bsc::new();
        assert_eq!(bsc.read_reg(REG_S), S_TXD | S_TXE);
        assert_eq!(bsc.read_reg(REG_DIV), 0x5dc);
        assert_eq!(bsc.read_reg(REG_DEL), 0x0030_0030);
        assert_eq!(bsc.read_reg(REG_CLKT), 0x40);
        assert!(!bsc.irq_pending());
    }

    #[test]
    fn bsc_quick_write_acks_present_target() {
        let mut bsc = bsc_with_bme280();
        start(&mut bsc, 0x76, 0, false, true);
        let s = bsc.read_reg(REG_S);
        assert_eq!(s & (S_DONE | S_ERR | S_TA), S_DONE);
        assert!(bsc.irq_pending());
        complete(&mut bsc);
        assert!(!bsc.irq_pending());
    }

    #[test]
    fn bsc_address_nack_sets_err_and_done() {
        let mut bsc = bsc_with_bme280();
        start(&mut bsc, 0x77, 0, false, true);
        let s = bsc.read_reg(REG_S);
        assert_eq!(s & (S_DONE | S_ERR | S_TA), S_DONE | S_ERR);
        assert!(bsc.irq_pending());
        complete(&mut bsc);
        assert_eq!(bsc.read_reg(REG_S) & (S_DONE | S_ERR), 0);
    }

    #[test]
    fn bsc_no_target_nacks_every_address() {
        let mut bsc = Bcm2835Bsc::new();
        start(&mut bsc, 0x76, 0, false, true);
        assert_ne!(bsc.read_reg(REG_S) & S_ERR, 0);
    }

    /// The exact sequence `i2cget -y 1 0x76 0xd0` drives through the Linux
    /// driver: write msg (INTT) whose TXW interrupt fills the FIFO and then
    /// issues the read msg (INTR|INTD) as a repeated start.
    #[test]
    fn bsc_write_then_read_bme280_chip_id() {
        let mut bsc = bsc_with_bme280();
        start(&mut bsc, 0x76, 1, false, false);
        let s = bsc.read_reg(REG_S);
        assert_ne!(s & S_TA, 0, "write transfer active");
        assert_ne!(s & S_TXW, 0, "FIFO needs writing");
        assert!(bsc.irq_pending(), "INTT raises TXW");

        // ISR: fill FIFO, then start the read message.
        assert_ne!(bsc.read_reg(REG_S) & S_TXD, 0);
        bsc.write_reg(REG_FIFO, 0xd0);
        start(&mut bsc, 0x76, 1, true, true);

        let s = bsc.read_reg(REG_S);
        assert_eq!(s & (S_DONE | S_ERR), S_DONE);
        assert_ne!(s & S_RXD, 0);
        assert!(bsc.irq_pending());
        assert_eq!(bsc.read_reg(REG_FIFO), 0x60, "BME280 chip-id");
        assert_eq!(bsc.read_reg(REG_S) & S_RXD, 0);
        complete(&mut bsc);
        assert!(!bsc.irq_pending());
    }

    #[test]
    fn bsc_write_then_read_nack_completes_with_err() {
        let mut bsc = Bcm2835Bsc::new();
        start(&mut bsc, 0x76, 1, false, false);
        assert!(bsc.irq_pending());
        bsc.write_reg(REG_FIFO, 0xd0);
        start(&mut bsc, 0x76, 1, true, true);
        let s = bsc.read_reg(REG_S);
        assert_eq!(s & (S_DONE | S_ERR | S_TA | S_RXD), S_DONE | S_ERR);
        assert!(bsc.irq_pending());
    }

    #[test]
    fn bsc_long_read_uses_rxr_and_refills() {
        let mut bsc = bsc_with_bme280();
        // Point at the calibration block 0x88, then read 26 bytes.
        start(&mut bsc, 0x76, 1, false, false);
        bsc.write_reg(REG_FIFO, 0x88);
        start(&mut bsc, 0x76, 26, true, true);
        let s = bsc.read_reg(REG_S);
        assert_ne!(s & S_RXR, 0);
        assert_ne!(s & S_RXF, 0);
        assert_eq!(s & S_DONE, 0);
        assert!(bsc.irq_pending());
        let mut got = Vec::new();
        while bsc.read_reg(REG_S) & S_RXD != 0 {
            got.push(bsc.read_reg(REG_FIFO) as u8);
        }
        assert_eq!(got.len(), 26);
        assert_ne!(bsc.read_reg(REG_S) & S_DONE, 0);
    }
}
