// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-classic (Xtensa LX6) I²C controller — command-list engine.
//!
//! Mapped at base 0x3FF5_3000 (I2C0 / I2C_EXT0) with size 4 KiB. See ESP32 TRM
//! v4.6 §11.
//!
//! The classic-ESP32 I²C is the same Espressif command-list IP family as the
//! ESP32-C3 ([`crate::peripherals::esp32c3::i2c::Esp32c3I2c`]) and ESP32-S3:
//! the register map, COMD command-list semantics, FIFO behaviour and bit
//! positions are identical. Every offset and bit position below was diffed
//! against the Espressif `soc/esp32/include/soc/i2c_reg.h` register header.
//!
//! ## Classic-vs-C3/S3 differences (the only substantive ones)
//!
//! 1. **Command opcodes.** The classic chip uses the *original* opcode
//!    numbering (`hal/esp32/include/hal/i2c_ll.h`): RSTART = 0, WRITE = 1,
//!    READ = 2, STOP = 3, END = 4. The C3/S3 renumbered these (RSTART = 6,
//!    READ = 3, STOP = 2). Programming a classic command list with the C3
//!    opcodes — or vice-versa — would mis-decode every command, so this is the
//!    one field that must be family-specific.
//! 2. **Interrupt source.** `ETS_I2C_EXT0_INTR_SOURCE = 49` on the classic LX6
//!    (`soc/esp32/include/soc/soc.h`), the Xtensa `ets_isr_source_t` ordinal —
//!    NOT the S3's 42 or the C3's 29. UART0 = 34 in the same enum corroborates
//!    the ordinal base.
//! 3. **16 command slots** (COMD0..COMD15 at 0x58..0x94) versus 8 on the C3.
//! 4. **CTR reset value 0x3** (`SCL_FORCE_OUT | SDA_FORCE_OUT`, both default 1):
//!    the classic CTR has no `SAMPLE_SCL_LEVEL`/`RX_FULL_ACK_LEVEL` default-1
//!    bits that make the C3 reset 0x20B.
//!
//! ## Register subset modeled (offsets per `i2c_reg.h`, identical to C3/S3)
//!
//! | Offset | Name        | Notes                                          |
//! |--------|-------------|------------------------------------------------|
//! | 0x04   | CTR         | TRANS_START at bit 5                           |
//! | 0x08   | SR          | bit 0 = ACK_REC; rx_cnt[13:8]; tx_cnt[23:18]   |
//! | 0x10   | SLAVE_ADDR  | 7-bit address in [6:0]                         |
//! | 0x14   | FIFO_ST     | TXFIFO_START_ADDR[14:10] = TX read pointer     |
//! | 0x18   | FIFO_CONF   | RX/TX FIFO reset bits (12/13) self-clear       |
//! | 0x1C   | DATA        | Write→TX FIFO, read→pop RX FIFO                |
//! | 0x20   | INT_RAW     | bit 3 = END_DETECT; bit 7 = TRANS_COMPLETE;    |
//! |        |             | bit 10 = NACK (ACK_ERR)                         |
//! | 0x24   | INT_CLR     | Write 1 to clear matching INT_RAW bits         |
//! | 0x28   | INT_ENA     | Enable mask                                    |
//! | 0x2C   | INT_ST      | INT_RAW & INT_ENA                              |
//! | 0x58.. | COMD0..15   | 16 command slots; bit 31 = command_done        |
//!
//! All other offsets round-trip through a generic backing store (writes stored,
//! reads return them; unwritten reads give 0). The command-list engine never
//! consults the timing registers, so this is faithful for the modeled scope.

use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::peripherals::i2c::I2cDevice;
use crate::{Peripheral, PeripheralTickResult, SimResult};

pub const I2C0_BASE: u32 = 0x3FF5_3000;
pub const I2C0_SIZE: u64 = 0x1000;

/// ESP32-classic I2C0 (I2C_EXT0) interrupt source number.
///
/// `ETS_I2C_EXT0_INTR_SOURCE = 49` in the classic `soc/esp32/include/soc/soc.h`
/// `ets_isr_source_t` enum (UART0 = 34 in the same enum fixes the ordinal
/// base). NOT the S3's 42 or the C3's 29.
pub const I2C0_INTR_SOURCE_ID: u32 = 49;

// Core FSM / status registers (offsets per i2c_reg.h).
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
const REG_CMD15: u64 = 0x94;

/// CTR bit 5: TRANS_START — self-clearing master-transaction trigger.
const CTR_TRANS_START_BIT: u32 = 1 << 5;
/// CTR reset: SCL_FORCE_OUT (bit 1) | SDA_FORCE_OUT (bit 0), both default 1.
const CTR_RESET: u32 = 0x0000_0003;

/// SR bit 0: ACK_REC — set when the slave acknowledged during the most recent
/// command. esp-hal raises `AcknowledgeCheckFailed` after MST_COMPLETE if clear.
const SR_ACK_REC: u32 = 1 << 0;

/// COMD bit 31: command_done. Set when a command finishes executing.
const CMD_DONE_BIT: u32 = 1 << 31;

pub const INT_END_DETECT: u32 = 1 << 3;
pub const INT_TRANS_COMPLETE: u32 = 1 << 7;
pub const INT_NACK: u32 = 1 << 10;

/// Classic ESP32 has 16 COMD slots at offsets 0x58..0x94 (COMD0..COMD15).
const NUM_CMDS: usize = 16;
/// SOC_I2C_FIFO_LEN on the classic chip.
const FIFO_CAPACITY: usize = 32;

pub struct Esp32I2c {
    ctr: u32,
    sr: u32,
    slave_addr: u32,
    int_raw: u32,
    int_ena: u32,
    fifo_conf: u32,
    cmds: [u32; NUM_CMDS],
    /// Shared with the AHB FIFO alias at `0x6001_301c` (esp-idf writes TX here).
    tx_fifo: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<u8>>>,
    /// TX-FIFO read pointer (bytes consumed by the current command-list run).
    /// Surfaced as FIFO_ST.TXFIFO_START_ADDR; 0 at cold reset.
    tx_pop_count: usize,
    rx_fifo: RefCell<std::collections::VecDeque<u8>>,
    slaves: Vec<Box<dyn I2cDevice>>,
    /// Interrupt-matrix source this instance asserts (49 for I2C0).
    intr_source_id: u32,
    /// Round-trip backing for timing / config registers the engine ignores.
    other: BTreeMap<u64, u32>,
}

/// AHB-bus TX FIFO alias (`I2C0` at `0x6001_301c`). esp-idf `i2c_ll_write_txfifo`
/// writes here instead of the APB DATA register at `0x3FF5_301c`.
pub struct Esp32I2cAhbFifo {
    tx_fifo: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<u8>>>,
}

impl std::fmt::Debug for Esp32I2cAhbFifo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Esp32I2cAhbFifo")
    }
}

impl Esp32I2c {
    pub fn new() -> Self {
        Self {
            ctr: CTR_RESET,
            sr: 0,
            slave_addr: 0,
            int_raw: 0,
            int_ena: 0,
            fifo_conf: 0,
            cmds: [0; NUM_CMDS],
            tx_fifo: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::with_capacity(FIFO_CAPACITY),
            )),
            tx_pop_count: 0,
            rx_fifo: RefCell::new(std::collections::VecDeque::with_capacity(FIFO_CAPACITY)),
            slaves: Vec::new(),
            intr_source_id: I2C0_INTR_SOURCE_ID,
            other: BTreeMap::new(),
        }
    }

    /// Construct an instance asserting a different interrupt-matrix source
    /// (I2C1 = `ETS_I2C_EXT1_INTR_SOURCE` = 50).
    pub fn with_intr_source(intr_source_id: u32) -> Self {
        Self {
            intr_source_id,
            ..Self::new()
        }
    }

    /// AHB FIFO window paired with this APB I2C (same TX FIFO).
    pub fn ahb_tx_fifo_alias(&self) -> Esp32I2cAhbFifo {
        Esp32I2cAhbFifo {
            tx_fifo: std::sync::Arc::clone(&self.tx_fifo),
        }
    }

    /// Raw slave push — does NOT wrap for tracing. The only production caller is
    /// the bus choke point [`crate::bus::SystemBus::attach_i2c_slave`], which
    /// wraps first. Slaves are matched by 7-bit address at transaction time;
    /// later additions take precedence on duplicate addresses.
    pub(crate) fn push_slave(&mut self, slave: Box<dyn I2cDevice>) {
        self.slaves.push(slave);
    }

    fn fifo_status(&self) -> u32 {
        // FIFO_ST.TXFIFO_START_ADDR (bits 14..10) is the TX-FIFO read pointer:
        // bytes consumed by the current command-list run. 0 at cold reset.
        let tx_raddr = (self.tx_pop_count as u32) & 0x1F;
        tx_raddr << 10
    }

    fn status_register(&self) -> u32 {
        // SR: bit 0 ACK_REC, RXFIFO_CNT at bits 13..8, TXFIFO_CNT at bits 23..18.
        let rx = (self.rx_fifo.borrow().len() as u32) & 0x3F;
        let tx = (self.tx_fifo.lock().unwrap().len() as u32) & 0x3F;
        (self.sr & SR_ACK_REC) | (rx << 8) | (tx << 18)
    }

    /// Resolve a slave from SLAVE_ADDR (7-bit or 8-bit shifted form). Used when
    /// Arduino/ESP-IDF parks the target in SLAVE_ADDR and does not push the
    /// address byte into the TX FIFO.
    fn find_slave_from_slave_addr_register(&self) -> Option<usize> {
        let raw = self.slave_addr & 0x7FFF;
        if raw <= 0x7F {
            if let Some(idx) = self.slaves.iter().position(|s| s.address() == raw as u8) {
                return Some(idx);
            }
        }
        let shifted = ((raw >> 1) & 0x7F) as u8;
        self.slaves.iter().position(|s| s.address() == shifted)
    }
}

impl Default for Esp32I2c {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Esp32I2c {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Esp32I2c")
            .field("ctr", &self.ctr)
            .field("slave_addr", &self.slave_addr)
            .field("int_raw", &self.int_raw)
            .field("int_ena", &self.int_ena)
            .field("slaves_count", &self.slaves.len())
            .finish()
    }
}

impl Peripheral for Esp32I2cAhbFifo {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, value: u8) -> SimResult<()> {
        let mut tx = self.tx_fifo.lock().unwrap();
        if tx.len() < FIFO_CAPACITY {
            tx.push_back(value);
        }
        Ok(())
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write(offset, (value & 0xFF) as u8)
    }
    fn read_u32(&self, _offset: u64) -> SimResult<u32> {
        Ok(0)
    }
}

impl Peripheral for Esp32I2c {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // Byte reads aren't used by the I2C driver; route via read_u32.
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
            REG_CMD0..=REG_CMD15 => {
                let idx = ((offset - REG_CMD0) / 4) as usize;
                self.cmds.get(idx).copied().unwrap_or(0)
            }
            other => self.other.get(&other).copied().unwrap_or(0),
        };
        if std::env::var("LABWIRED_I2C_TRACE").is_ok() {
            eprintln!("ESP32 I2C R [0x{offset:02x}] = 0x{v:08x}");
        }
        Ok(v)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Byte writes ignored — the driver writes whole words (except the FIFO
        // data register, which is also driven word-wide via write_u32).
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if std::env::var("LABWIRED_I2C_TRACE").is_ok() {
            eprintln!("ESP32 I2C W [0x{offset:02x}] = 0x{value:08x}");
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
            REG_DATA => {
                let mut tx = self.tx_fifo.lock().unwrap();
                if tx.len() < FIFO_CAPACITY {
                    tx.push_back((value & 0xFF) as u8);
                }
            }
            REG_FIFO_CONF => {
                self.fifo_conf = value;
                // Bit 12 = RX_FIFO_RST; bit 13 = TX_FIFO_RST. Self-clearing.
                if value & (1 << 12) != 0 {
                    self.rx_fifo.borrow_mut().clear();
                }
                if value & (1 << 13) != 0 {
                    self.tx_fifo.lock().unwrap().clear();
                    self.tx_pop_count = 0;
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
            other => {
                self.other.insert(other, value);
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // LEVEL interrupt: assert the I2C0 source every tick while any enabled
        // INT bit is set, mirroring real silicon (INT_RAW stays asserted until
        // the ISR writes INT_CLR).
        let explicit = if self.int_raw & self.int_ena != 0 {
            Some(vec![self.intr_source_id])
        } else {
            None
        };
        PeripheralTickResult {
            explicit_irqs: explicit,
            ..Default::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn drives_central_i2c_time(&self) -> bool {
        true
    }

    fn advance_attached_i2c_us(&mut self, us: u64) {
        if us == 0 {
            return;
        }
        for slave in self.slaves.iter_mut() {
            slave.advance_time_us(us);
        }
    }

    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for slave in self.slaves.iter_mut() {
            if let Some(si) = slave.as_sim_input_mut() {
                if f(si) {
                    return true;
                }
            }
        }
        false
    }
}

impl Esp32I2c {
    /// Walk COMD0..COMD15 from the start, executing each command. A WRITE whose
    /// first byte follows an RSTART is interpreted as `(addr<<1)|R/W` and selects
    /// the active slave by address bits [7:1]. Subsequent WRITE bytes are
    /// delivered via `I2cDevice::write`; READ pulls bytes from the active slave
    /// and pushes to the RX FIFO.
    fn run_command_list(&mut self) {
        // Classic-ESP32 opcodes (hal/esp32/include/hal/i2c_ll.h):
        //   0 = RSTART, 1 = WRITE, 2 = READ, 3 = STOP, 4 = END
        const OP_RSTART: u32 = 0;
        const OP_WRITE: u32 = 1;
        const OP_READ: u32 = 2;
        const OP_STOP: u32 = 3;
        const OP_END: u32 = 4;

        let mut active: Option<usize> = None;
        let mut expects_addr = true;
        let mut last_op_was_end = false;

        // Reset ACK_REC and the TX-FIFO read pointer at the start of a run.
        self.sr &= !SR_ACK_REC;
        self.tx_pop_count = 0;

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
                    // Empty WRITE (byte_num=0) after RSTART: Arduino Wire probe
                    // often parks the 7-bit target in SLAVE_ADDR and issues a
                    // zero-payload WRITE. Resolve the slave from SLAVE_ADDR so
                    // matrix L3 ACK succeeds (mirrors ESP32-S3 engine).
                    if expects_addr && byte_num == 0 {
                        active = self.find_slave_from_slave_addr_register();
                        if let Some(slave_idx) = active {
                            self.slaves[slave_idx].start();
                            self.sr |= SR_ACK_REC;
                        } else {
                            self.int_raw |= INT_NACK;
                        }
                        expects_addr = false;
                    }
                    for i in 0..byte_num {
                        let b = self.tx_fifo.lock().unwrap().pop_front().unwrap_or(0);
                        self.tx_pop_count += 1;
                        if expects_addr && i == 0 {
                            // First byte of a WRITE following RSTART is addr+R/W.
                            let addr = b >> 1;
                            active = self.slaves.iter().position(|s| s.address() == addr);
                            if active.is_none() {
                                // Fallback: address only in SLAVE_ADDR, payload in FIFO.
                                active = self.find_slave_from_slave_addr_register();
                                if let Some(slave_idx) = active {
                                    self.slaves[slave_idx].start();
                                    self.sr |= SR_ACK_REC;
                                    // First FIFO byte is data when SLAVE_ADDR holds target.
                                    self.slaves[slave_idx].write(b);
                                    expects_addr = false;
                                    continue;
                                }
                                self.int_raw |= INT_NACK;
                            } else {
                                self.sr |= SR_ACK_REC;
                            }
                            expects_addr = false;
                            // Don't deliver the addr byte to the slave's write().
                            continue;
                        }
                        if let Some(slave_idx) = active {
                            self.slaves[slave_idx].write(b);
                            self.sr |= SR_ACK_REC;
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
                        self.sr |= SR_ACK_REC;
                    }
                    self.cmds[idx] |= CMD_DONE_BIT;
                }
                OP_STOP => {
                    if let Some(slave_idx) = active {
                        self.slaves[slave_idx].stop();
                    }
                    self.cmds[idx] |= CMD_DONE_BIT;
                    break;
                }
                OP_END => {
                    last_op_was_end = true;
                    break;
                }
                _ => break, // reserved opcode — terminate
            }
        }

        // END pauses execution and raises END_DETECT; STOP (or a list that runs
        // out without an explicit END) completes and raises TRANS_COMPLETE.
        if last_op_was_end {
            self.int_raw |= INT_END_DETECT;
        } else {
            self.int_raw |= INT_TRANS_COMPLETE;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a 14-bit command word: opcode | byte_num.
    fn cmd(opcode: u8, byte_num: u8) -> u32 {
        ((opcode as u32 & 0x7) << 11) | (byte_num as u32)
    }

    // Classic-ESP32 opcodes: 0=RSTART, 1=WRITE, 2=READ, 3=STOP, 4=END.
    const CMD_RSTART: u8 = 0;
    const CMD_WRITE: u8 = 1;
    const CMD_READ: u8 = 2;
    const CMD_STOP: u8 = 3;
    const CMD_END: u8 = 4;

    #[test]
    fn i2c0_interrupt_source_is_49() {
        // Classic-vs-S3/C3: classic routes I2C_EXT0 through ets_isr_source_t
        // ordinal 49, NOT the S3's 42 or the C3's 29.
        assert_eq!(I2C0_INTR_SOURCE_ID, 49);
    }

    #[test]
    fn ctr_reset_is_force_out_bits() {
        let p = Esp32I2c::new();
        assert_eq!(p.read_u32(REG_CTR).unwrap(), 0x0000_0003);
    }

    #[test]
    fn ctr_round_trip() {
        let mut p = Esp32I2c::new();
        p.write_u32(REG_CTR, 0x0000_0010).unwrap(); // arbitrary, no TRANS_START
        assert_eq!(p.read_u32(REG_CTR).unwrap(), 0x0000_0010);
    }

    #[test]
    fn slave_addr_round_trip() {
        let mut p = Esp32I2c::new();
        p.write_u32(REG_SLAVE_ADDR, 0x48).unwrap();
        assert_eq!(p.read_u32(REG_SLAVE_ADDR).unwrap(), 0x48);
    }

    #[test]
    fn has_sixteen_command_slots() {
        let mut p = Esp32I2c::new();
        p.write_u32(REG_CMD0, 0x0000_0800).unwrap();
        p.write_u32(REG_CMD15, 0x0000_2000).unwrap();
        assert_eq!(p.read_u32(REG_CMD0).unwrap(), 0x0000_0800);
        assert_eq!(p.read_u32(REG_CMD15).unwrap(), 0x0000_2000);
    }

    #[test]
    fn sr_txfifo_cnt_reflects_pushes() {
        let mut p = Esp32I2c::new();
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
    fn fifo_reset_bit_clears_tx_fifo() {
        let mut p = Esp32I2c::new();
        p.write_u32(REG_DATA, 0x11).unwrap();
        p.write_u32(REG_DATA, 0x22).unwrap();
        p.write_u32(REG_FIFO_CONF, 1 << 13).unwrap(); // TX_FIFO_RST
        let sr = p.read_u32(REG_SR).unwrap();
        assert_eq!((sr >> 18) & 0x3F, 0);
    }

    #[test]
    fn int_clr_clears_specified_bits() {
        let mut p = Esp32I2c::new();
        p.int_raw = INT_TRANS_COMPLETE | INT_NACK;
        p.write_u32(REG_INT_CLR, INT_NACK).unwrap();
        assert_eq!(p.read_u32(REG_INT_RAW).unwrap(), INT_TRANS_COMPLETE);
    }

    #[test]
    fn int_st_masks_with_int_ena() {
        let mut p = Esp32I2c::new();
        p.int_raw = INT_TRANS_COMPLETE | INT_NACK;
        p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap();
        assert_eq!(p.read_u32(REG_INT_ST).unwrap(), INT_TRANS_COMPLETE);
    }

    #[test]
    fn end_opcode_raises_end_detect_not_trans_complete() {
        let mut p = Esp32I2c::new();
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
        let mut p = Esp32I2c::new();
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
            INT_TRANS_COMPLETE
        );
    }

    #[test]
    fn trans_start_auto_clears() {
        let mut p = Esp32I2c::new();
        p.write_u32(REG_CMD0, cmd(CMD_END, 0)).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        assert_eq!(p.read_u32(REG_CTR).unwrap() & CTR_TRANS_START_BIT, 0);
    }

    #[test]
    fn write_with_unmatched_address_sets_nack_int() {
        let mut p = Esp32I2c::new();
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
    fn fifo_st_read_pointer_tracks_consumed_tx_bytes() {
        // A WRITE of 2 bytes following RSTART consumes 2 TX-FIFO bytes; the
        // FIFO_ST TXFIFO_START_ADDR field (bits 14..10) reports that pointer.
        let mut p = Esp32I2c::new();
        p.push_slave(Box::new(crate::peripherals::components::Bmp280::new(0x76)));
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_DATA, 0xEC).unwrap(); // addr+W
        p.write_u32(REG_DATA, 0xD0).unwrap(); // pointer byte
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        let fifo_st = p.read_u32(REG_FIFO_ST).unwrap();
        assert_eq!((fifo_st >> 10) & 0x1F, 2, "TX read pointer should be 2");
    }

    // ── Headline test: an attached I2cDevice round-trips a write-then-read
    //    transaction driven exactly as classic-ESP32 firmware would. Uses the
    //    Bmp280 register-pointer device (an existing I2cDevice).
    use crate::peripherals::components::Bmp280;

    #[test]
    fn write_read_drives_attached_bmp280() {
        let mut p = Esp32I2c::new();
        // Default address 0x76.
        p.push_slave(Box::new(Bmp280::new(0x76)));

        // Canonical register-pointer read: set pointer to 0xD0 (chip-id), then
        // repeated-start and read one byte. CHIP_ID for BMP280 is 0x58.
        //   RSTART; WRITE 2 (addr+W, pointer=0xD0); RSTART;
        //   WRITE 1 (addr+R); READ 1; STOP.
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 12, cmd(CMD_WRITE, 1)).unwrap();
        p.write_u32(REG_CMD0 + 16, cmd(CMD_READ, 1)).unwrap();
        p.write_u32(REG_CMD0 + 20, cmd(CMD_STOP, 0)).unwrap();

        // Push TX bytes: addr+W (0x76<<1=0xEC), pointer 0xD0, addr+R (0xED).
        p.write_u32(REG_DATA, 0xEC).unwrap();
        p.write_u32(REG_DATA, 0xD0).unwrap();
        p.write_u32(REG_DATA, 0xED).unwrap();

        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();

        // Address must have matched (no NACK).
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_NACK,
            0,
            "BMP280 at 0x76 must ACK its address"
        );
        // Slave acked → ACK_REC set in SR.
        assert_eq!(
            p.read_u32(REG_SR).unwrap() & SR_ACK_REC,
            SR_ACK_REC,
            "SR.ACK_REC must be set after a successful transaction"
        );
        // The chip-id byte 0x58 should be in the RX FIFO.
        assert_eq!(
            p.read_u32(REG_DATA).unwrap(),
            0x58,
            "BMP280 CHIP_ID round-trip"
        );
        // STOP completed the transaction.
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
            INT_TRANS_COMPLETE
        );
    }

    #[test]
    fn tick_asserts_source_when_enabled_int_pending() {
        let mut p = Esp32I2c::new();
        p.int_raw = INT_TRANS_COMPLETE;
        p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap();
        let r = p.tick();
        assert_eq!(r.explicit_irqs, Some(vec![49]));
    }

    #[test]
    fn tick_silent_when_int_disabled() {
        let mut p = Esp32I2c::new();
        p.int_raw = INT_TRANS_COMPLETE; // raw set but not enabled
        let r = p.tick();
        assert_eq!(r.explicit_irqs, None);
    }
}
