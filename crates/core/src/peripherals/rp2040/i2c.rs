// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 I2C — Synopsys DesignWare APB I2C (`DW_apb_i2c`, datasheet §4.3).
//!
//! Master path with attached slaves (matrix kits). Supports:
//! - Arduino Wire (poll TX_ABRT / STATUS)
//! - Zephyr `i2c_dw` (INT: TX_EMPTY + STOP_DET after DATA_CMD)
//!
//! Zephyr init requires `IC_COMP_TYPE == 0x44570140`.

use crate::peripherals::i2c::I2cDevice;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::{Cell, RefCell};

const IC_CON: u64 = 0x00;
const IC_TAR: u64 = 0x04;
const IC_DATA_CMD: u64 = 0x10;
const IC_INTR_STAT: u64 = 0x2c;
const IC_INTR_MASK: u64 = 0x30;
const IC_RAW_INTR_STAT: u64 = 0x34;
const IC_CLR_TX_ABRT: u64 = 0x54;
const IC_CLR_ACTIVITY: u64 = 0x5c;
const IC_CLR_STOP_DET: u64 = 0x60;
const IC_ENABLE: u64 = 0x6c;
const IC_STATUS: u64 = 0x70;
const IC_TXFLR: u64 = 0x74;
const IC_RXFLR: u64 = 0x78;
const IC_TX_ABRT_SOURCE: u64 = 0x80;
const IC_COMP_TYPE: u64 = 0xfc;
const IC_COMP_TYPE_MAGIC: u32 = 0x4457_0140;

// RAW / MASK interrupt bits (DW_apb_i2c).
const INTR_TX_EMPTY: u32 = 1 << 4;
const INTR_TX_ABRT: u32 = 1 << 6;
const INTR_ACTIVITY: u32 = 1 << 8;
const INTR_STOP_DET: u32 = 1 << 9;

const ABRT_7B_ADDR_NOACK: u32 = 1 << 0;
const ENABLE_ENABLE: u32 = 1 << 0;

const STATUS_ACTIVITY: u32 = 1 << 0;
const STATUS_TFNF: u32 = 1 << 1;
const STATUS_TFE: u32 = 1 << 2;
const STATUS_RFNE: u32 = 1 << 3;

const DATA_CMD_READ: u32 = 1 << 8;
const DATA_CMD_STOP: u32 = 1 << 9;

#[derive(Default)]
pub struct Rp2040I2c {
    enable: u32,
    tar: u32,
    con: u32,
    intr_mask: u32,
    raw_intr: Cell<u32>,
    tx_abrt_source: Cell<u32>,
    rx_byte: Cell<Option<u8>>,
    activity: Cell<bool>,
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
        let s = Self::default();
        // FIFO empty at reset → TX_EMPTY raw bit set (DW default behaviour).
        s.raw_intr.set(INTR_TX_EMPTY);
        s
    }

    pub(crate) fn push_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.attached_devices.push(RefCell::new(device));
    }

    fn device_for(&self, addr7: u8) -> Option<usize> {
        self.attached_devices
            .iter()
            .position(|d| d.borrow().address() == addr7)
    }

    fn set_raw(&self, bits: u32) {
        self.raw_intr.set(self.raw_intr.get() | bits);
    }

    fn clr_raw(&self, bits: u32) {
        self.raw_intr.set(self.raw_intr.get() & !bits);
    }

    fn irq_pending(&self) -> bool {
        (self.raw_intr.get() & self.intr_mask) != 0
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
            IC_INTR_MASK => self.intr_mask,
            IC_RAW_INTR_STAT => self.raw_intr.get(),
            IC_INTR_STAT => self.raw_intr.get() & self.intr_mask,
            IC_CLR_TX_ABRT => {
                self.clr_raw(INTR_TX_ABRT);
                self.tx_abrt_source.set(0);
                0
            }
            IC_CLR_ACTIVITY => {
                self.clr_raw(INTR_ACTIVITY);
                self.activity.set(false);
                0
            }
            IC_CLR_STOP_DET => {
                self.clr_raw(INTR_STOP_DET);
                0
            }
            IC_DATA_CMD => self.rx_byte.take().unwrap_or(0xFF) as u32,
            IC_STATUS => {
                let mut s = STATUS_TFE | STATUS_TFNF;
                if self.activity.get() {
                    s |= STATUS_ACTIVITY;
                }
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
            IC_TX_ABRT_SOURCE => self.tx_abrt_source.get(),
            IC_COMP_TYPE => IC_COMP_TYPE_MAGIC,
            _ => 0,
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            IC_CON => self.con = value,
            IC_TAR => self.tar = value & 0x3FF,
            IC_INTR_MASK => self.intr_mask = value,
            IC_ENABLE => {
                self.enable = value;
                if value & ENABLE_ENABLE != 0 {
                    // Enabled with empty FIFO → TX_EMPTY asserted.
                    self.set_raw(INTR_TX_EMPTY);
                }
            }
            IC_DATA_CMD if self.enable & ENABLE_ENABLE != 0 => {
                let addr7 = (self.tar & 0x7F) as u8;
                let stop = value & DATA_CMD_STOP != 0;
                self.activity.set(true);
                self.set_raw(INTR_ACTIVITY);
                // Consuming a command clears TX_EMPTY until the engine finishes.
                self.clr_raw(INTR_TX_EMPTY);
                match self.device_for(addr7) {
                    None => {
                        self.set_raw(INTR_TX_ABRT);
                        self.tx_abrt_source
                            .set(self.tx_abrt_source.get() | ABRT_7B_ADDR_NOACK);
                        self.activity.set(false);
                        self.set_raw(INTR_TX_EMPTY);
                        if stop {
                            self.set_raw(INTR_STOP_DET);
                        }
                    }
                    Some(idx) => {
                        self.clr_raw(INTR_TX_ABRT);
                        self.tx_abrt_source.set(0);
                        if value & DATA_CMD_READ != 0 {
                            let b = self.attached_devices[idx].borrow_mut().read();
                            self.rx_byte.set(Some(b));
                        } else {
                            self.attached_devices[idx]
                                .borrow_mut()
                                .write((value & 0xFF) as u8);
                        }
                        // Instant complete — FIFO empty again.
                        self.set_raw(INTR_TX_EMPTY);
                        self.activity.set(false);
                        if stop {
                            self.set_raw(INTR_STOP_DET);
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

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult {
            irq: self.irq_pending(),
            ..Default::default()
        }
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
        assert_eq!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
        i2c.write_u32(IC_DATA_CMD, 0xDE | DATA_CMD_STOP).unwrap();
        assert_ne!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
        assert_ne!(
            i2c.read_u32(IC_TX_ABRT_SOURCE).unwrap() & ABRT_7B_ADDR_NOACK,
            0
        );
    }

    #[test]
    fn attached_slave_acks_write_and_stop_det() {
        struct Dev {
            addr: u8,
            last: Cell<u8>,
        }
        impl I2cDevice for Dev {
            fn address(&self) -> u8 {
                self.addr
            }
            fn read(&mut self) -> u8 {
                0
            }
            fn write(&mut self, data: u8) {
                self.last.set(data);
            }
        }
        let mut i2c = enabled_i2c();
        i2c.push_slave(Box::new(Dev {
            addr: 0x40,
            last: Cell::new(0),
        }));
        i2c.intr_mask = INTR_TX_EMPTY | INTR_STOP_DET | INTR_TX_ABRT;
        i2c.write_u32(IC_TAR, 0x40).unwrap();
        i2c.write_u32(IC_DATA_CMD, 0xAB | DATA_CMD_STOP).unwrap();
        assert_eq!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
        assert_ne!(
            i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_STOP_DET,
            0
        );
        assert_ne!(
            i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_EMPTY,
            0
        );
        assert!(i2c.tick().irq);
    }

    #[test]
    fn comp_type_is_designware_magic() {
        let i2c = Rp2040I2c::new();
        assert_eq!(i2c.read_u32(IC_COMP_TYPE).unwrap(), IC_COMP_TYPE_MAGIC);
    }

    #[test]
    fn disabled_controller_does_not_abort() {
        let mut i2c = Rp2040I2c::new();
        i2c.write_u32(IC_DATA_CMD, 0xDE).unwrap();
        assert_eq!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
    }
}
