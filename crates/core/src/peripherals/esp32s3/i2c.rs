// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 I²C0 controller — command-list engine.
//!
//! Mapped at base 0x6001_3000 with size 4 KiB. See ESP32-S3 TRM §29.
//!
//! ## Register subset modeled
//!
//! Offsets per ESP32-S3 PAC `esp32s3::i2c0` (TRM § 29.6):
//!
//! | Offset | Name        | Notes                                          |
//! |--------|-------------|------------------------------------------------|
//! | 0x04   | CTR         | TRANS_START at bit 5                           |
//! | 0x08   | SR          | Status — bit 0 = RESP_REC (slave acked)        |
//! | 0x10   | SLAVE_ADDR  | 7-bit address in [6:0]                         |
//! | 0x14   | FIFO_ST     | TX/RX FIFO levels                              |
//! | 0x18   | FIFO_CONF   | Reset bits accept and clear (no behavior)      |
//! | 0x1C   | DATA        | Write→TX FIFO, read→pop RX FIFO                |
//! | 0x20   | INT_RAW     | Bit 3 = END_DETECT; bit 7 = TRANS_COMPLETE;    |
//! |        |             | bit 10 = NACK                                  |
//! | 0x24   | INT_CLR     | Write 1 to clear matching INT_RAW bits         |
//! | 0x28   | INT_ENA     | Enable mask                                    |
//! | 0x2C   | INT_ST      | INT_RAW & INT_ENA                              |
//! | 0x58.. | CMD0..CMD7  | 8 command slots; bit 31 = command_done         |
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
const REG_SR: u64 = 0x08;
const REG_SLAVE_ADDR: u64 = 0x10;
const REG_FIFO_ST: u64 = 0x14;
const REG_FIFO_CONF: u64 = 0x18;
const REG_DATA: u64 = 0x1C;
const REG_INT_RAW: u64 = 0x20;
const REG_INT_CLR: u64 = 0x24;
const REG_INT_ENA: u64 = 0x28;
const REG_INT_ST: u64 = 0x2C;
const REG_CMD0: u64 = 0x58;
const REG_CMD7: u64 = 0x74;

const CTR_TRANS_START_BIT: u32 = 1 << 5;

/// SR bit 0: set when the slave responded with ACK during the most recent
/// command. esp-hal checks this after TRANS_COMPLETE — if clear it raises
/// `AcknowledgeCheckFailed(Data)`.
const SR_RESP_REC: u32 = 1 << 0;

/// COMD bit 31: command_done. Set when a command finishes executing. esp-hal
/// scans every COMD register and reports `ExecutionIncomplete` if any non-END
/// command lacks this bit.
const CMD_DONE_BIT: u32 = 1 << 31;

pub const INT_END_DETECT: u32 = 1 << 3;
pub const INT_TRANS_COMPLETE: u32 = 1 << 7;
pub const INT_NACK: u32 = 1 << 10;

/// ESP32-S3 has 8 COMD slots at offsets 0x58..0x78. Higher offsets are
/// SCL/SP timing registers that we accept-and-ignore.
const NUM_CMDS: usize = 8;
const FIFO_CAPACITY: usize = 32;

pub struct Esp32s3I2c {
    ctr: u32,
    sr: u32,
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
            sr: 0,
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
        // Per ESP32-S3 PAC `i2c0::fifo_st`:
        //   bits  0..4  RXFIFO_RADDR
        //   bits  5..9  RXFIFO_WADDR
        //   bits 10..14 TXFIFO_RADDR ← used by esp-hal's estimate_ack_failed_reason
        //   bits 15..19 TXFIFO_WADDR
        // We approximate raddr with the bytes that have already been popped
        // (FIFO_CAPACITY - tx_len). esp-hal compares <= 1 to distinguish
        // address-NACK from data-NACK; we only need the comparison to be right.
        let tx_raddr = (FIFO_CAPACITY as u32 - self.tx_fifo.len() as u32).min(0x1F);
        tx_raddr << 10
    }

    fn status_register(&self) -> u32 {
        // Per ESP32-S3 PAC `i2c0::sr`:
        //   bit  0      RESP_REC (slave acked the most recent byte)
        //   bits 8..13  RXFIFO_CNT
        //   bits 18..23 TXFIFO_CNT
        let rx = (self.rx_fifo.borrow().len() as u32) & 0x3F;
        let tx = (self.tx_fifo.len() as u32) & 0x3F;
        (self.sr & SR_RESP_REC) | (rx << 8) | (tx << 18)
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
            REG_SR => self.status_register(),
            REG_SLAVE_ADDR => self.slave_addr,
            REG_DATA => self.rx_fifo.borrow_mut().pop_front().unwrap_or(0) as u32,
            REG_FIFO_CONF => self.fifo_conf,
            REG_INT_RAW => self.int_raw,
            REG_INT_CLR => 0,
            REG_INT_ENA => self.int_ena,
            REG_INT_ST => self.int_raw & self.int_ena,
            REG_FIFO_ST => self.fifo_status(),
            REG_CMD0..=REG_CMD7 => {
                let idx = ((offset - REG_CMD0) / 4) as usize;
                self.cmds.get(idx).copied().unwrap_or(0)
            }
            _ => 0,
        };
        if std::env::var("LABWIRED_I2C_TRACE").is_ok() {
            eprintln!("I2C R [0x{offset:02x}] = 0x{v:08x}");
        }
        Ok(v)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Byte writes ignored — the esp-hal driver writes whole words.
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if std::env::var("LABWIRED_I2C_TRACE").is_ok() {
            eprintln!("I2C W [0x{offset:02x}] = 0x{value:08x}");
        }
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
            // Byte 0 of value goes into TX FIFO (per esp-hal usage).
            // Drop the byte silently if the FIFO is full — esp-hal checks
            // FIFO_ST before pushing, so the bound check is defensive only.
            REG_DATA if self.tx_fifo.len() < FIFO_CAPACITY => {
                self.tx_fifo.push_back((value & 0xFF) as u8);
            }
            REG_DATA => {}
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
            REG_CMD0..=REG_CMD7 => {
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
        // LEVEL interrupt: assert source 49 every tick while any enabled INT
        // bit is set, mirroring real silicon (INT_RAW stays asserted until the
        // ISR writes INT_CLR). The previous one-shot pulse (`irq_pending`
        // drained in a single tick) was fine for esp-hal's *polling* driver
        // but was LOST by ESP-IDF's interrupt-driven `i2c_master` driver if the
        // CPU happened to be masked the tick the transaction completed — the
        // driver then blocked on its ISR semaphore forever and timed out
        // (ESP_ERR_INVALID_STATE). De-asserts when the ISR clears INT_RAW
        // (INT_CLR) or disables INT_ENA; esp-hal leaves INT_ENA clear so its
        // path is unaffected.
        self.irq_pending = false;
        if self.int_raw & self.int_ena != 0 {
            explicit.push(I2C0_INTR_SOURCE_ID);
        }
        PeripheralTickResult {
            explicit_irqs: if explicit.is_empty() {
                None
            } else {
                Some(explicit)
            },
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
        // Opcodes per ESP32-S3 TRM § 29.5 / esp32s3 PAC `i2c0::comd::OPCODE`:
        //   1 = WRITE, 2 = STOP, 3 = READ, 4 = END, 6 = RSTART
        const OP_WRITE: u32 = 1;
        const OP_STOP: u32 = 2;
        const OP_READ: u32 = 3;
        const OP_END: u32 = 4;
        const OP_RSTART: u32 = 6;

        // Index into self.slaves of the currently-selected device, or
        // None if no address has been latched yet (or just after RSTART).
        let mut active: Option<usize> = None;
        let mut expects_addr = true;
        let mut last_op_was_end = false;
        let mut hit_stop = false;

        // Reset RESP_REC at the start of a new command-list run; the slave
        // sets it back to 1 below if any byte is acknowledged.
        self.sr &= !SR_RESP_REC;

        for idx in 0..self.cmds.len() {
            let word = self.cmds[idx];
            let opcode = (word >> 11) & 0x7;
            let byte_num = (word & 0xFF) as usize;
            match opcode {
                OP_RSTART => {
                    if let Some(slave_idx) = active {
                        self.slaves[slave_idx].start();
                    }
                    expects_addr = true;
                    active = None;
                    self.cmds[idx] |= CMD_DONE_BIT;
                }
                OP_WRITE => {
                    for i in 0..byte_num {
                        let b = self.tx_fifo.pop_front().unwrap_or(0);
                        if expects_addr && i == 0 {
                            // First byte of a WRITE following RSTART is addr+R/W.
                            let addr = b >> 1;
                            active = self.slaves.iter().position(|s| s.address() == addr);
                            if active.is_none() {
                                self.int_raw |= INT_NACK;
                            } else {
                                // Slave acknowledged its address.
                                self.sr |= SR_RESP_REC;
                            }
                            expects_addr = false;
                            // Don't deliver the addr byte to the slave's write().
                            continue;
                        }
                        if let Some(slave_idx) = active {
                            self.slaves[slave_idx].write(b);
                            // Slave acknowledged the data byte.
                            self.sr |= SR_RESP_REC;
                        }
                    }
                    self.cmds[idx] |= CMD_DONE_BIT;
                }
                OP_READ => {
                    for _ in 0..byte_num {
                        let b = if let Some(slave_idx) = active {
                            self.slaves[slave_idx].read()
                        } else {
                            0
                        };
                        let mut rx = self.rx_fifo.borrow_mut();
                        if rx.len() < FIFO_CAPACITY {
                            rx.push_back(b);
                        }
                    }
                    if active.is_some() {
                        self.sr |= SR_RESP_REC;
                    }
                    self.cmds[idx] |= CMD_DONE_BIT;
                }
                OP_STOP => {
                    if let Some(slave_idx) = active {
                        self.slaves[slave_idx].stop();
                    }
                    self.cmds[idx] |= CMD_DONE_BIT;
                    hit_stop = true;
                    break;
                }
                OP_END => {
                    last_op_was_end = true;
                    break;
                }
                _ => break, // reserved opcode — terminate
            }
        }

        // Per ESP32-S3 TRM § 29.5: END pauses execution and raises
        // END_DETECT (bit 3). STOP completes the transaction and raises
        // TRANS_COMPLETE (bit 7). esp-hal blocks on (END_DETECT | TX_COMPLETE)
        // and uses END_DETECT to chain phase 2 of a write_read.
        if last_op_was_end {
            self.int_raw |= INT_END_DETECT;
        } else if hit_stop {
            self.int_raw |= INT_TRANS_COMPLETE;
        } else {
            // Empty cmd list or reserved opcode — set TRANS_COMPLETE so the
            // driver's wait loop unblocks rather than hanging.
            self.int_raw |= INT_TRANS_COMPLETE;
        }
        if self.int_ena & (INT_TRANS_COMPLETE | INT_END_DETECT | INT_NACK) != 0 {
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

    // ESP32-S3 TRM § 29.5: 1=WRITE, 2=STOP, 3=READ, 4=END, 6=RSTART.
    const CMD_WRITE: u8 = 1;
    const CMD_STOP: u8 = 2;
    const CMD_READ: u8 = 3;
    const CMD_END: u8 = 4;
    const CMD_RSTART: u8 = 6;

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
        p.write_u32(REG_CMD7, 0x0000_2000).unwrap();
        assert_eq!(p.read_u32(REG_CMD0).unwrap(), 0x0000_0800);
        assert_eq!(p.read_u32(REG_CMD7).unwrap(), 0x0000_2000);
    }

    #[test]
    fn unmapped_offsets_read_zero_and_accept_writes() {
        let mut p = Esp32s3I2c::new();
        p.write_u32(0xFFC, 0xDEAD_BEEF).unwrap(); // last 4-byte slot in 4 KiB
        assert_eq!(p.read_u32(0xFFC).unwrap(), 0);
    }

    #[test]
    fn sr_txfifo_cnt_reflects_pushes() {
        // esp-hal reads TX/RX FIFO counts from SR.txfifo_cnt (bits 18..23)
        // and SR.rxfifo_cnt (bits 8..13), not from FIFO_ST.
        let mut p = Esp32s3I2c::new();
        p.write_u32(REG_DATA, 0xAA).unwrap();
        p.write_u32(REG_DATA, 0xBB).unwrap();
        p.write_u32(REG_DATA, 0xCC).unwrap();
        let sr = p.read_u32(REG_SR).unwrap();
        assert_eq!(
            (sr >> 18) & 0x3F,
            3,
            "SR.txfifo_cnt should reflect 3 pushes"
        );
    }

    #[test]
    fn fifo_reset_bits_clear_fifos() {
        let mut p = Esp32s3I2c::new();
        p.write_u32(REG_DATA, 0x11).unwrap();
        p.write_u32(REG_DATA, 0x22).unwrap();
        p.write_u32(REG_FIFO_CONF, 1 << 13).unwrap(); // TX_FIFO_RST
        let sr = p.read_u32(REG_SR).unwrap();
        assert_eq!((sr >> 18) & 0x3F, 0);
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
    fn end_opcode_raises_end_detect_not_trans_complete() {
        // Per ESP32-S3 TRM § 29.5: the END opcode pauses the controller and
        // raises END_DETECT (bit 3). esp-hal uses END_DETECT to chain phase 2
        // of a write_read transaction. STOP is what raises TRANS_COMPLETE.
        let mut p = Esp32s3I2c::new();
        p.write_u32(REG_CMD0, cmd(CMD_END, 0)).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        let int_raw = p.read_u32(REG_INT_RAW).unwrap();
        assert_eq!(
            int_raw & INT_END_DETECT,
            INT_END_DETECT,
            "END must raise END_DETECT"
        );
        assert_eq!(
            int_raw & INT_TRANS_COMPLETE,
            0,
            "END must NOT raise TRANS_COMPLETE"
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
        let p = Esp32s3I2c::new();
        // Pre-load the RX FIFO directly to test the read path in isolation.
        p.rx_fifo.borrow_mut().push_back(0xAA);
        p.rx_fifo.borrow_mut().push_back(0xBB);
        let first = p.read_u32(REG_DATA).unwrap();
        let second = p.read_u32(REG_DATA).unwrap();
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
        p.write_u32(REG_DATA, 0x90).unwrap(); // 0x48 << 1 | W
        p.write_u32(REG_DATA, 0x00).unwrap(); // pointer
        p.write_u32(REG_DATA, 0x91).unwrap(); // 0x48 << 1 | R

        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();

        // Two bytes of temperature should now be in RX FIFO: 0x19, 0x00.
        assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x19);
        assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x00);
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
        p.write_u32(REG_DATA, 0xA0).unwrap(); // some addr+W, no slave
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

        p.write_u32(REG_DATA, 0x90).unwrap(); // addr+W
        p.write_u32(REG_DATA, 0x01).unwrap(); // pointer = CONFIG
        p.write_u32(REG_DATA, 0x91).unwrap(); // addr+R
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();

        assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x60);
        assert_eq!(p.read_u32(REG_DATA).unwrap(), 0xA0);
    }
}
