// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 I²C0 controller — command-list engine.
//!
//! Mapped at base 0x6001_3000 with size 4 KiB. See ESP32-S3 TRM §29.
//!
//! ## Register subset modeled
//!
//! | Offset | Name        | Notes                                          |
//! |--------|-------------|------------------------------------------------|
//! | 0x04   | CTR         | TRANS_START at bit 5                           |
//! | 0x10   | SLAVE_ADDR  | 7-bit address in [6:0]                         |
//! | 0x18   | FIFO_DATA   | Write→TX FIFO, read→pop RX FIFO                |
//! | 0x1C   | FIFO_CONF   | Reset bits accept and clear (no behavior)      |
//! | 0x20   | INT_RAW     | Bit 7 = TRANS_COMPLETE; bit 11 = NACK          |
//! | 0x24   | INT_CLR     | Write 1 to clear matching INT_RAW bits         |
//! | 0x28   | INT_ENA     | Enable mask                                    |
//! | 0x2C   | INT_ST      | INT_RAW & INT_ENA                              |
//! | 0x44   | FIFO_ST     | TX/RX FIFO levels                              |
//! | 0x58.. | CMD0..CMD15 | 14-bit command words (16 entries × 4 B)        |
//!
//! All other offsets accept writes silently and read 0.

use std::cell::RefCell;

use crate::peripherals::i2c::I2cDevice;
use crate::{Peripheral, PeripheralTickResult, SimResult};

pub const I2C0_BASE: u32 = 0x6001_3000;
pub const I2C0_SIZE: u64 = 0x1000;

/// ESP32-S3 I_I2C_EXT0_INT_SOURCE per TRM §9.4 table 9-1.
pub const I2C0_INTR_SOURCE_ID: u32 = 49;

const REG_CTR: u64 = 0x04;
const REG_SLAVE_ADDR: u64 = 0x10;
const REG_FIFO_DATA: u64 = 0x18;
const REG_FIFO_CONF: u64 = 0x1C;
const REG_INT_RAW: u64 = 0x20;
const REG_INT_CLR: u64 = 0x24;
const REG_INT_ENA: u64 = 0x28;
const REG_INT_ST: u64 = 0x2C;
const REG_FIFO_ST: u64 = 0x44;
const REG_CMD0: u64 = 0x58;
const REG_CMD15: u64 = 0x94;

const CTR_TRANS_START_BIT: u32 = 1 << 5;

pub const INT_TRANS_COMPLETE: u32 = 1 << 7;
pub const INT_NACK: u32 = 1 << 11;

const NUM_CMDS: usize = 16;
const FIFO_CAPACITY: usize = 32;

pub struct Esp32s3I2c {
    ctr: u32,
    slave_addr: u32,
    int_raw: u32,
    int_ena: u32,
    fifo_conf: u32,
    cmds: [u32; NUM_CMDS],
    tx_fifo: std::collections::VecDeque<u8>,
    rx_fifo: RefCell<std::collections::VecDeque<u8>>,
    slaves: Vec<Box<dyn I2cDevice>>,
    /// Set when a command-list run sets TRANS_COMPLETE & INT_ENA has it.
    /// Drained by `tick()` into the interrupt-matrix source aggregation.
    irq_pending: bool,
}

impl Esp32s3I2c {
    pub fn new() -> Self {
        Self {
            ctr: 0,
            slave_addr: 0,
            int_raw: 0,
            int_ena: 0,
            fifo_conf: 0,
            cmds: [0; NUM_CMDS],
            tx_fifo: std::collections::VecDeque::with_capacity(FIFO_CAPACITY),
            rx_fifo: RefCell::new(std::collections::VecDeque::with_capacity(FIFO_CAPACITY)),
            slaves: Vec::new(),
            irq_pending: false,
        }
    }

    /// Attach an `I2cDevice` slave. Slaves are matched by address bits at
    /// transaction time; later additions take precedence on duplicate addresses.
    pub fn attach_slave(&mut self, slave: Box<dyn I2cDevice>) {
        self.slaves.push(slave);
    }

    fn fifo_status(&self) -> u32 {
        // Bits [4:0] = TX FIFO level, bits [12:8] = RX FIFO level.
        let rx_len = self.rx_fifo.borrow().len();
        ((self.tx_fifo.len() as u32) & 0x1F) | (((rx_len as u32) & 0x1F) << 8)
    }
}

impl Default for Esp32s3I2c {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Esp32s3I2c {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Esp32s3I2c")
            .field("ctr", &self.ctr)
            .field("slave_addr", &self.slave_addr)
            .field("int_raw", &self.int_raw)
            .field("int_ena", &self.int_ena)
            .field("slaves_count", &self.slaves.len())
            .field("irq_pending", &self.irq_pending)
            .finish()
    }
}

impl Peripheral for Esp32s3I2c {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // Byte reads aren't used by esp-hal's I2C driver; route everything
        // through read_u32. Returning 0 for stray byte reads is harmless.
        Ok(0)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let v = match offset {
            REG_CTR => self.ctr,
            REG_SLAVE_ADDR => self.slave_addr,
            REG_FIFO_DATA => self.rx_fifo.borrow_mut().pop_front().unwrap_or(0) as u32,
            REG_FIFO_CONF => self.fifo_conf,
            REG_INT_RAW => self.int_raw,
            REG_INT_CLR => 0,
            REG_INT_ENA => self.int_ena,
            REG_INT_ST => self.int_raw & self.int_ena,
            REG_FIFO_ST => self.fifo_status(),
            REG_CMD0..=REG_CMD15 => {
                let idx = ((offset - REG_CMD0) / 4) as usize;
                self.cmds.get(idx).copied().unwrap_or(0)
            }
            _ => 0,
        };
        Ok(v)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Byte writes ignored — the esp-hal driver writes whole words.
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            REG_CTR => {
                self.ctr = value;
                if value & CTR_TRANS_START_BIT != 0 {
                    self.run_command_list();
                    // Auto-clear TRANS_START like real silicon.
                    self.ctr &= !CTR_TRANS_START_BIT;
                }
            }
            REG_SLAVE_ADDR => self.slave_addr = value,
            REG_FIFO_DATA => {
                // Byte 0 of value goes into TX FIFO (per esp-hal usage).
                if self.tx_fifo.len() < FIFO_CAPACITY {
                    self.tx_fifo.push_back((value & 0xFF) as u8);
                }
            }
            REG_FIFO_CONF => {
                self.fifo_conf = value;
                // Bit 12 = RX_FIFO_RST; bit 13 = TX_FIFO_RST. Self-clearing.
                if value & (1 << 12) != 0 {
                    self.rx_fifo.borrow_mut().clear();
                }
                if value & (1 << 13) != 0 {
                    self.tx_fifo.clear();
                }
                self.fifo_conf &= !((1 << 12) | (1 << 13));
            }
            REG_INT_CLR => self.int_raw &= !value,
            REG_INT_ENA => self.int_ena = value,
            REG_CMD0..=REG_CMD15 => {
                let idx = ((offset - REG_CMD0) / 4) as usize;
                if let Some(slot) = self.cmds.get_mut(idx) {
                    *slot = value;
                }
            }
            _ => {} // Accept-and-ignore (timing regs, etc.)
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let mut explicit = Vec::new();
        if self.irq_pending {
            explicit.push(I2C0_INTR_SOURCE_ID);
            self.irq_pending = false;
        }
        PeripheralTickResult {
            explicit_irqs: if explicit.is_empty() { None } else { Some(explicit) },
            ..Default::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl Esp32s3I2c {
    /// Walk CMD0..CMD15 from the start, executing each command. A "WRITE"
    /// whose first byte follows an RSTART is interpreted as `(addr<<1)|R/W`
    /// and selects the active slave by address bits [7:1]. Subsequent
    /// WRITE bytes are delivered via `I2cDevice::write`. READ pulls bytes
    /// from the active slave via `I2cDevice::read` and pushes to the RX FIFO.
    fn run_command_list(&mut self) {
        const OP_RSTART: u32 = 0;
        const OP_WRITE: u32 = 1;
        const OP_READ: u32 = 2;
        const OP_STOP: u32 = 3;
        const OP_END: u32 = 4;

        // Index into self.slaves of the currently-selected device, or
        // None if no address has been latched yet (or just after RSTART).
        let mut active: Option<usize> = None;
        let mut expects_addr = true;

        for &word in &self.cmds {
            let opcode = (word >> 11) & 0x7;
            let byte_num = (word & 0xFF) as usize;
            match opcode {
                OP_RSTART => {
                    if let Some(idx) = active {
                        self.slaves[idx].start();
                    }
                    expects_addr = true;
                    active = None;
                }
                OP_WRITE => {
                    for i in 0..byte_num {
                        let b = self.tx_fifo.pop_front().unwrap_or(0);
                        if expects_addr && i == 0 {
                            // First byte of a WRITE following RSTART is addr+R/W.
                            let addr = b >> 1;
                            active = self
                                .slaves
                                .iter()
                                .position(|s| s.address() == addr);
                            if active.is_none() {
                                self.int_raw |= INT_NACK;
                            }
                            expects_addr = false;
                            // Don't deliver the addr byte to the slave's write().
                            continue;
                        }
                        if let Some(idx) = active {
                            self.slaves[idx].write(b);
                        }
                    }
                }
                OP_READ => {
                    for _ in 0..byte_num {
                        let b = if let Some(idx) = active {
                            self.slaves[idx].read()
                        } else {
                            0
                        };
                        let mut rx = self.rx_fifo.borrow_mut();
                        if rx.len() < FIFO_CAPACITY {
                            rx.push_back(b);
                        }
                    }
                }
                OP_STOP => {
                    if let Some(idx) = active {
                        self.slaves[idx].stop();
                    }
                    break;
                }
                OP_END => break,
                _ => break, // reserved opcode — terminate
            }
        }

        self.int_raw |= INT_TRANS_COMPLETE;
        if self.int_ena & (INT_TRANS_COMPLETE | INT_NACK) != 0 {
            self.irq_pending = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REG_CMD1_OFFSET: u64 = REG_CMD0 + 4;

    /// Encode a 14-bit command word: opcode | byte_num.
    fn cmd(opcode: u8, byte_num: u8) -> u32 {
        ((opcode as u32 & 0x7) << 11) | (byte_num as u32)
    }

    const CMD_RSTART: u8 = 0;
    const CMD_WRITE: u8 = 1;
    const CMD_READ: u8 = 2;
    const CMD_STOP: u8 = 3;
    const CMD_END: u8 = 4;

    #[test]
    fn ctr_round_trip() {
        let mut p = Esp32s3I2c::new();
        p.write_u32(REG_CTR, 0x0000_0010).unwrap(); // arbitrary, no TRANS_START
        assert_eq!(p.read_u32(REG_CTR).unwrap(), 0x0000_0010);
    }

    #[test]
    fn slave_addr_round_trip() {
        let mut p = Esp32s3I2c::new();
        p.write_u32(REG_SLAVE_ADDR, 0x48).unwrap();
        assert_eq!(p.read_u32(REG_SLAVE_ADDR).unwrap(), 0x48);
    }

    #[test]
    fn cmd_registers_round_trip() {
        let mut p = Esp32s3I2c::new();
        p.write_u32(REG_CMD0, 0x0000_0800).unwrap();
        p.write_u32(REG_CMD15, 0x0000_2000).unwrap();
        assert_eq!(p.read_u32(REG_CMD0).unwrap(), 0x0000_0800);
        assert_eq!(p.read_u32(REG_CMD15).unwrap(), 0x0000_2000);
    }

    #[test]
    fn unmapped_offsets_read_zero_and_accept_writes() {
        let mut p = Esp32s3I2c::new();
        p.write_u32(0xFFC, 0xDEAD_BEEF).unwrap(); // last 4-byte slot in 4 KiB
        assert_eq!(p.read_u32(0xFFC).unwrap(), 0);
    }

    #[test]
    fn fifo_status_reflects_pushes() {
        let mut p = Esp32s3I2c::new();
        p.write_u32(REG_FIFO_DATA, 0xAA).unwrap();
        p.write_u32(REG_FIFO_DATA, 0xBB).unwrap();
        p.write_u32(REG_FIFO_DATA, 0xCC).unwrap();
        let st = p.read_u32(REG_FIFO_ST).unwrap();
        assert_eq!(st & 0x1F, 3, "TX level should reflect 3 pushes");
    }

    #[test]
    fn fifo_reset_bits_clear_fifos() {
        let mut p = Esp32s3I2c::new();
        p.write_u32(REG_FIFO_DATA, 0x11).unwrap();
        p.write_u32(REG_FIFO_DATA, 0x22).unwrap();
        p.write_u32(REG_FIFO_CONF, 1 << 13).unwrap(); // TX_FIFO_RST
        let st = p.read_u32(REG_FIFO_ST).unwrap();
        assert_eq!(st & 0x1F, 0);
    }

    #[test]
    fn int_clr_clears_specified_bits() {
        let mut p = Esp32s3I2c::new();
        p.int_raw = INT_TRANS_COMPLETE | INT_NACK;
        p.write_u32(REG_INT_CLR, INT_NACK).unwrap();
        assert_eq!(p.read_u32(REG_INT_RAW).unwrap(), INT_TRANS_COMPLETE);
    }

    #[test]
    fn int_st_masks_with_int_ena() {
        let mut p = Esp32s3I2c::new();
        p.int_raw = INT_TRANS_COMPLETE | INT_NACK;
        p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap();
        assert_eq!(p.read_u32(REG_INT_ST).unwrap(), INT_TRANS_COMPLETE);
    }

    #[test]
    fn empty_command_list_with_end_completes() {
        let mut p = Esp32s3I2c::new();
        p.write_u32(REG_CMD0, cmd(CMD_END, 0)).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
            INT_TRANS_COMPLETE
        );
    }

    #[test]
    fn rstart_then_stop_completes() {
        let mut p = Esp32s3I2c::new();
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD1_OFFSET, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
            INT_TRANS_COMPLETE
        );
    }

    #[test]
    fn trans_start_auto_clears() {
        let mut p = Esp32s3I2c::new();
        p.write_u32(REG_CMD0, cmd(CMD_END, 0)).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        assert_eq!(p.read_u32(REG_CTR).unwrap() & CTR_TRANS_START_BIT, 0);
    }

    #[test]
    fn rx_fifo_pops_in_fifo_order_via_fifo_data_reads() {
        let mut p = Esp32s3I2c::new();
        // Pre-load the RX FIFO directly to test the read path in isolation.
        p.rx_fifo.borrow_mut().push_back(0xAA);
        p.rx_fifo.borrow_mut().push_back(0xBB);
        let first = p.read_u32(REG_FIFO_DATA).unwrap();
        let second = p.read_u32(REG_FIFO_DATA).unwrap();
        assert_eq!(first, 0xAA);
        assert_eq!(second, 0xBB);
    }

    use crate::peripherals::esp32s3::tmp102::Tmp102;

    #[test]
    fn write_read_drives_attached_tmp102() {
        let mut p = Esp32s3I2c::new();
        p.attach_slave(Box::new(Tmp102::new()));

        // Build the canonical TMP102 read sequence:
        //   RSTART; WRITE 2 (addr+W, pointer=0); RSTART;
        //   WRITE 1 (addr+R); READ 2; STOP.
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 12, cmd(CMD_WRITE, 1)).unwrap();
        p.write_u32(REG_CMD0 + 16, cmd(CMD_READ, 2)).unwrap();
        p.write_u32(REG_CMD0 + 20, cmd(CMD_STOP, 0)).unwrap();

        // Push TX bytes: addr+W, pointer 0, addr+R.
        p.write_u32(REG_FIFO_DATA, 0x90).unwrap(); // 0x48 << 1 | W
        p.write_u32(REG_FIFO_DATA, 0x00).unwrap(); // pointer
        p.write_u32(REG_FIFO_DATA, 0x91).unwrap(); // 0x48 << 1 | R

        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();

        // Two bytes of temperature should now be in RX FIFO: 0x19, 0x00.
        assert_eq!(p.read_u32(REG_FIFO_DATA).unwrap(), 0x19);
        assert_eq!(p.read_u32(REG_FIFO_DATA).unwrap(), 0x00);
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
            INT_TRANS_COMPLETE
        );
    }

    #[test]
    fn write_with_unmatched_address_sets_nack_int() {
        let mut p = Esp32s3I2c::new();
        // No slaves attached.
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 1)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_FIFO_DATA, 0xA0).unwrap(); // some addr+W, no slave
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_NACK,
            INT_NACK,
            "INT_NACK should fire when no slave matches"
        );
    }

    #[test]
    fn write_then_read_pointer_round_trip() {
        // Set pointer to CONFIG (0x01) via WRITE, then READ should return
        // CONFIG canned value 0x60A0 high byte then low byte.
        let mut p = Esp32s3I2c::new();
        p.attach_slave(Box::new(Tmp102::new()));

        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 12, cmd(CMD_WRITE, 1)).unwrap();
        p.write_u32(REG_CMD0 + 16, cmd(CMD_READ, 2)).unwrap();
        p.write_u32(REG_CMD0 + 20, cmd(CMD_STOP, 0)).unwrap();

        p.write_u32(REG_FIFO_DATA, 0x90).unwrap(); // addr+W
        p.write_u32(REG_FIFO_DATA, 0x01).unwrap(); // pointer = CONFIG
        p.write_u32(REG_FIFO_DATA, 0x91).unwrap(); // addr+R
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();

        assert_eq!(p.read_u32(REG_FIFO_DATA).unwrap(), 0x60);
        assert_eq!(p.read_u32(REG_FIFO_DATA).unwrap(), 0xA0);
    }
}
