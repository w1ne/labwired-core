// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 I2C — Synopsys DesignWare APB I2C (`DW_apb_i2c`, datasheet §4.3,
//! I2C0 base `0x40044000`).
//!
//! Master transfer engine with optional attached I²C slaves (matrix kits).
//! With the controller enabled (`IC_ENABLE.ENABLE`) and a target in `IC_TAR`,
//! the first command pushed into `IC_DATA_CMD`:
//!   * NACK path (no matching slave) — raises `IC_RAW_INTR_STAT.TX_ABRT` and
//!     records `ABRT_7B_ADDR_NOACK` in `IC_TX_ABRT_SOURCE` (TX FIFO flushed).
//!   * ACK path (slave present) — clears abort, delivers write bytes to the
//!     slave (or returns 0xFF on read CMD), so Arduino Wire probes succeed.
//!
//! Reading `IC_CLR_TX_ABRT` clears the abort (read-to-clear). `IC_STATUS`
//! reports a coherent steady state (TX FIFO empty + not full).

use crate::peripherals::i2c::I2cDevice;
use crate::{Peripheral, SimResult};
use std::cell::{Cell, RefCell};

// DW_apb_i2c register offsets (datasheet §4.3.17).
const IC_CON: u64 = 0x00;
const IC_TAR: u64 = 0x04;
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
const STATUS_RFNE: u32 = 1 << 3; // RX FIFO not empty

// IC_DATA_CMD bits.
const DATA_CMD_READ: u32 = 1 << 8; // CMD=1 → read

#[derive(Default)]
pub struct Rp2040I2c {
    enable: u32,
    tar: u32,
    con: u32,
    /// Latched abort interrupt (read-to-clear via `IC_CLR_TX_ABRT`). `Cell`
    /// because the clear happens on a `&self` read.
    tx_abrt: Cell<bool>,
    tx_abrt_source: Cell<u32>,
    /// One-byte RX hold when a slave ACKs a read command.
    rx_byte: Cell<Option<u8>>,
    /// Attached I²C slaves (matrix kits / external_devices).
    attached_devices: Vec<RefCell<Box<dyn I2cDevice>>>,
}

impl std::fmt::Debug for Rp2040I2c {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rp2040I2c")
            .field("enable", &self.enable)
            .field("tar", &self.tar)
            .field("slaves", &self.attached_devices.len())
            .finish()
    }
}

impl Rp2040I2c {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a slave — bus funnel [`crate::bus::SystemBus::attach_i2c_slave`].
    pub(crate) fn push_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.attached_devices.push(RefCell::new(device));
    }

    fn device_for(&self, addr7: u8) -> Option<usize> {
        self.attached_devices
            .iter()
            .position(|d| d.borrow().address() == addr7)
    }
}

impl Peripheral for Rp2040I2c {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let val = match offset {
            IC_CON => self.con,
            IC_TAR => self.tar,
            IC_ENABLE => self.enable,
            IC_RAW_INTR_STAT if self.tx_abrt.get() => INTR_TX_ABRT,
            IC_TX_ABRT_SOURCE => self.tx_abrt_source.get(),
            // Reading IC_CLR_TX_ABRT clears the abort interrupt (read-to-clear).
            IC_CLR_TX_ABRT => {
                self.tx_abrt.set(false);
                self.tx_abrt_source.set(0);
                0
            }
            IC_DATA_CMD => {
                // Pop held RX byte if any (master read).
                self.rx_byte.take().unwrap_or(0xFF) as u32
            }
            // TX FIFO empties immediately (transfers complete synchronously).
            IC_STATUS => {
                let mut s = STATUS_TFE | STATUS_TFNF;
                if self.rx_byte.get().is_some() {
                    s |= STATUS_RFNE;
                }
                s
            }
            IC_TXFLR => 0,
            IC_RXFLR => {
                if self.rx_byte.get().is_some() {
                    1
                } else {
                    0
                }
            }
            _ => 0,
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            IC_CON => self.con = value,
            IC_TAR => self.tar = value & 0x3FF,
            IC_ENABLE => self.enable = value,
            // A command issued while enabled drives the bus.
            IC_DATA_CMD if self.enable & ENABLE_ENABLE != 0 => {
                let addr7 = (self.tar & 0x7F) as u8;
                match self.device_for(addr7) {
                    None => {
                        self.tx_abrt.set(true);
                        self.tx_abrt_source
                            .set(self.tx_abrt_source.get() | ABRT_7B_ADDR_NOACK);
                    }
                    Some(idx) => {
                        self.tx_abrt.set(false);
                        self.tx_abrt_source.set(0);
                        if value & DATA_CMD_READ != 0 {
                            let b = self.attached_devices[idx].borrow_mut().read();
                            self.rx_byte.set(Some(b));
                        } else {
                            self.attached_devices[idx]
                                .borrow_mut()
                                .write((value & 0xFF) as u8);
                        }
                    }
                }
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
        // RX FIFO (bit 3) is always empty when no pending read data.
        assert_eq!(s & (1 << 3), 0);
    }

    #[test]
    fn attached_slave_acks_write() {
        struct Dev {
            addr: u8,
            last: u8,
        }
        impl I2cDevice for Dev {
            fn address(&self) -> u8 {
                self.addr
            }
            fn read(&mut self) -> u8 {
                0
            }
            fn write(&mut self, data: u8) {
                self.last = data;
            }
        }
        let mut i2c = enabled_i2c();
        i2c.push_slave(Box::new(Dev {
            addr: 0x40,
            last: 0,
        }));
        i2c.write_u32(IC_TAR, 0x40).unwrap();
        i2c.write_u32(IC_DATA_CMD, 0xAB).unwrap();
        assert_eq!(
            i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT,
            0,
            "present slave must not ABRT"
        );
    }
}
