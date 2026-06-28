// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 SPI — ARM PrimeCell PL022 SSP (datasheet §4.4, SPI0 base
//! `0x4003c000`).
//!
//! A minimal but real SSP transfer engine: the data register (`SSPDR`) feeds a
//! transmit path and a receive FIFO, and the status register (`SSPSR`) reports
//! FIFO state. With the controller enabled (`SSPCR1.SSE`) and internal loopback
//! selected (`SSPCR1.LBM`) — the PL022's built-in self-test mode — each byte
//! written to `SSPDR` is clocked straight back into the receive FIFO, so a
//! write-then-read pair returns the same byte: a genuine modelled transfer, not
//! storage. Transfers complete within the write, so `SSPSR.BSY` is never
//! observed busy and the TX FIFO always reads empty.
//!
//! Without loopback (and with no attached slave in the chip model) a written
//! byte is still consumed by the TX path but produces no receive data — the
//! realistic outcome for an open MISO line.
//!
//! The receive FIFO is read-to-drain, so it lives behind a `RefCell`: the bus
//! read path is `&self`, but reading `SSPDR` must pop an entry.

use crate::{Peripheral, SimResult};
use std::cell::RefCell;
use std::collections::VecDeque;

// PL022 register offsets (datasheet §4.4.4).
const SSPCR1: u64 = 0x004;
const SSPDR: u64 = 0x008;
const SSPSR: u64 = 0x00c;

// SSPCR1 control bits.
const CR1_LBM: u32 = 1 << 0; // loopback mode
const CR1_SSE: u32 = 1 << 1; // synchronous serial port enable

// SSPSR status bits.
const SR_TFE: u32 = 1 << 0; // transmit FIFO empty
const SR_TNF: u32 = 1 << 1; // transmit FIFO not full
const SR_RNE: u32 = 1 << 2; // receive FIFO not empty
const SR_RFF: u32 = 1 << 3; // receive FIFO full
const SR_BSY: u32 = 1 << 4; // busy

// PL022 FIFOs are 8 entries deep.
const FIFO_DEPTH: usize = 8;

#[derive(Debug, Default)]
pub struct Rp2040Spi {
    cr1: u32,
    rx_fifo: RefCell<VecDeque<u16>>,
}

impl Rp2040Spi {
    pub fn new() -> Self {
        Self::default()
    }

    fn enabled(&self) -> bool {
        self.cr1 & CR1_SSE != 0
    }

    fn loopback(&self) -> bool {
        self.cr1 & CR1_LBM != 0
    }

    fn status(&self) -> u32 {
        // TX path drains immediately, so TX is always empty / not full and the
        // engine is never busy (SR_BSY is never asserted).
        let mut sr = SR_TFE | SR_TNF;
        let rx = self.rx_fifo.borrow();
        if !rx.is_empty() {
            sr |= SR_RNE;
        }
        if rx.len() >= FIFO_DEPTH {
            sr |= SR_RFF;
        }
        let _ = SR_BSY;
        sr
    }

    /// Pop the head of the receive FIFO (the `SSPDR` read port).
    fn pop_dr(&self) -> u32 {
        self.rx_fifo.borrow_mut().pop_front().unwrap_or(0) as u32
    }
}

impl Peripheral for Rp2040Spi {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let val = match offset {
            SSPCR1 => self.cr1,
            SSPDR => self.pop_dr(), // reading SSPDR drains the RX FIFO
            SSPSR => self.status(),
            _ => 0,
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            SSPCR1 => self.cr1 = value,
            // Internal loopback: MOSI is wired to MISO, so the byte clocks
            // straight into the receive FIFO. Non-loopback (or disabled) with no
            // attached slave: byte consumed, no RX (falls through to `_`).
            SSPDR if self.enabled() && self.loopback() => {
                let mut rx = self.rx_fifo.borrow_mut();
                if rx.len() < FIFO_DEPTH {
                    rx.push_back((value & 0xffff) as u16);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        // Reading the data register (any byte lane of it) drains one FIFO entry.
        if (offset & !0x3) == SSPDR {
            let word = self.pop_dr();
            return Ok((word >> ((offset & 0x3) * 8)) as u8);
        }
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        let shift = (offset & 0x3) * 8;
        // Avoid the read-modify-write going through the draining SSPDR read.
        let cur = if aligned == SSPDR {
            0
        } else {
            self.read_u32(aligned)?
        };
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enable_loopback(spi: &mut Rp2040Spi) {
        spi.write_u32(SSPCR1, CR1_SSE | CR1_LBM).unwrap();
    }

    #[test]
    fn loopback_roundtrips_byte() {
        let mut spi = Rp2040Spi::new();
        enable_loopback(&mut spi);
        spi.write_u32(SSPDR, 0xA5).unwrap();
        // RNE must be set after the loopback transfer.
        assert_ne!(spi.read_u32(SSPSR).unwrap() & SR_RNE, 0);
        // Draining read returns the same byte.
        let rx = spi.read_u32(SSPDR).unwrap();
        assert_eq!(rx, 0xA5);
        // FIFO drained → RNE clear.
        assert_eq!(spi.read_u32(SSPSR).unwrap() & SR_RNE, 0);
    }

    #[test]
    fn disabled_port_does_not_capture() {
        let mut spi = Rp2040Spi::new();
        // Loopback selected but SSE not set → no transfer.
        spi.write_u32(SSPCR1, CR1_LBM).unwrap();
        spi.write_u32(SSPDR, 0x42).unwrap();
        assert_eq!(spi.read_u32(SSPSR).unwrap() & SR_RNE, 0);
    }

    #[test]
    fn status_reports_tx_empty_and_not_busy() {
        let spi = Rp2040Spi::new();
        let sr = spi.read_u32(SSPSR).unwrap();
        assert_ne!(sr & SR_TFE, 0, "TX FIFO empty at reset");
        assert_ne!(sr & SR_TNF, 0, "TX FIFO not full at reset");
        assert_eq!(sr & SR_BSY, 0, "not busy at reset");
    }
}
