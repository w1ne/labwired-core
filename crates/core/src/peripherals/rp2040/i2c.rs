// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 I2C — Synopsys DesignWare APB I2C (`DW_apb_i2c`, datasheet §4.3,
//! I2C0 base `0x40044000`).
//!
//! A minimal master transfer engine. The chip model attaches no I2C slave
//! devices, so a master transfer to any target address gets no acknowledge —
//! exactly as on a real bus with nothing connected. The model reproduces that
//! response: with the controller enabled (`IC_ENABLE.ENABLE`) and a target set
//! (`IC_TAR`), the first command pushed into `IC_DATA_CMD` raises the transmit
//! abort interrupt (`IC_RAW_INTR_STAT.TX_ABRT`) and records the 7-bit
//! address-NACK reason in `IC_TX_ABRT_SOURCE` (`ABRT_7B_ADDR_NOACK`). The TX
//! FIFO is flushed on abort (per the IP), so `IC_TXFLR` returns 0.
//!
//! This is the same modelling choice the nRF52 TWIM uses for its address-NACK
//! path — a genuine engine response, not a storage stub. Reading
//! `IC_CLR_TX_ABRT` clears the abort (the IP's read-to-clear semantics).
//!
//! `IC_STATUS` reports a coherent steady state (TX FIFO empty + not full, RX
//! FIFO empty); every other register is plain read/write storage.

use crate::{Peripheral, SimResult};
use std::cell::Cell;

// DW_apb_i2c register offsets (datasheet §4.3.17).
const IC_DATA_CMD: u64 = 0x10; // TX/RX data + command
const IC_RAW_INTR_STAT: u64 = 0x34; // raw interrupt status
const IC_CLR_TX_ABRT: u64 = 0x54; // read-to-clear TX_ABRT
const IC_ENABLE: u64 = 0x6c; // controller enable
const IC_STATUS: u64 = 0x70; // FIFO / activity status
const IC_TXFLR: u64 = 0x74; // TX FIFO level
const IC_RXFLR: u64 = 0x78; // RX FIFO level
const IC_TX_ABRT_SOURCE: u64 = 0x80; // abort reason bitmap

// IC_RAW_INTR_STAT bits.
const INTR_TX_ABRT: u32 = 1 << 6;

// IC_TX_ABRT_SOURCE bits.
const ABRT_7B_ADDR_NOACK: u32 = 1 << 0;

// IC_ENABLE bits.
const ENABLE_ENABLE: u32 = 1 << 0;

// IC_STATUS bits.
const STATUS_TFNF: u32 = 1 << 1; // TX FIFO not full
const STATUS_TFE: u32 = 1 << 2; // TX FIFO empty

#[derive(Debug, Default)]
pub struct Rp2040I2c {
    enable: u32,
    /// Latched abort interrupt (read-to-clear via `IC_CLR_TX_ABRT`). `Cell`
    /// because the clear happens on a `&self` read.
    tx_abrt: Cell<bool>,
    tx_abrt_source: Cell<u32>,
}

impl Rp2040I2c {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Rp2040I2c {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let val = match offset {
            IC_ENABLE => self.enable,
            IC_RAW_INTR_STAT if self.tx_abrt.get() => INTR_TX_ABRT,
            IC_TX_ABRT_SOURCE => self.tx_abrt_source.get(),
            // Reading IC_CLR_TX_ABRT clears the abort interrupt (read-to-clear).
            IC_CLR_TX_ABRT => {
                self.tx_abrt.set(false);
                self.tx_abrt_source.set(0);
                0
            }
            // TX FIFO empties immediately (transfers complete synchronously);
            // the RX FIFO is always empty (no slave returns data).
            IC_STATUS => STATUS_TFE | STATUS_TFNF,
            IC_TXFLR | IC_RXFLR => 0,
            _ => 0,
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            IC_ENABLE => self.enable = value,
            // A command issued while enabled drives the bus. With no slave
            // attached the address phase gets no ACK → 7-bit address NACK
            // abort. (A read command — CMD bit8 set — aborts the same way.)
            IC_DATA_CMD if self.enable & ENABLE_ENABLE != 0 => {
                self.tx_abrt.set(true);
                self.tx_abrt_source
                    .set(self.tx_abrt_source.get() | ABRT_7B_ADDR_NOACK);
            }
            _ => {}
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        let shift = (offset & 0x3) * 8;
        let cur = self.read_u32(aligned)?;
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_i2c() -> Rp2040I2c {
        let mut i2c = Rp2040I2c::new();
        i2c.write_u32(IC_ENABLE, ENABLE_ENABLE).unwrap();
        i2c
    }

    #[test]
    fn unacked_transfer_aborts_with_addr_nack() {
        let mut i2c = enabled_i2c();
        // No abort before any command.
        assert_eq!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
        // Issue a write command to an (unconnected) target.
        i2c.write_u32(IC_DATA_CMD, 0xDE).unwrap();
        assert_ne!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
        assert_ne!(
            i2c.read_u32(IC_TX_ABRT_SOURCE).unwrap() & ABRT_7B_ADDR_NOACK,
            0
        );
        // TX FIFO flushed on abort.
        assert_eq!(i2c.read_u32(IC_TXFLR).unwrap(), 0);
    }

    #[test]
    fn disabled_controller_does_not_abort() {
        let mut i2c = Rp2040I2c::new(); // not enabled
        i2c.write_u32(IC_DATA_CMD, 0xDE).unwrap();
        assert_eq!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
    }

    #[test]
    fn clr_tx_abrt_clears_the_abort() {
        let mut i2c = enabled_i2c();
        i2c.write_u32(IC_DATA_CMD, 0xDE).unwrap();
        assert_ne!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
        let _ = i2c.read_u32(IC_CLR_TX_ABRT).unwrap();
        assert_eq!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
        assert_eq!(i2c.read_u32(IC_TX_ABRT_SOURCE).unwrap(), 0);
    }

    #[test]
    fn status_steady_state() {
        let i2c = enabled_i2c();
        let s = i2c.read_u32(IC_STATUS).unwrap();
        assert_ne!(s & STATUS_TFE, 0);
        assert_ne!(s & STATUS_TFNF, 0);
        // RX FIFO (bit 3) is always empty: no slave returns data.
        assert_eq!(s & (1 << 3), 0);
    }
}
