// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RP2040 I²C controller — Synopsys **DW_apb_i2c** master model.
//!
//! Mapped at I2C0 base 0x4004_4000 (I2C1 = 0x4004_8000), size 4 KiB. See the
//! RP2040 datasheet §4.3 and the Synopsys DesignWare DW_apb_i2c databook.
//!
//! This models the controller in **master mode** — enough for firmware to run a
//! real register-level transaction against an attached [`I2cDevice`] slave:
//! set `IC_TAR`, enable via `IC_ENABLE`, push data/read commands to
//! `IC_DATA_CMD`, and pop received bytes from `IC_DATA_CMD` when `IC_STATUS.RFNE`
//! is set. The TX path is processed synchronously on each `IC_DATA_CMD` write
//! (functional, not cycle-accurate), so the attached slave sees writes and
//! supplies reads exactly as real silicon would at the byte/FSM level.
//!
//! ## Register subset modeled (offsets per RP2040 §4.3.16)
//!
//! | Offset | Name             | Behavior                                        |
//! |--------|------------------|-------------------------------------------------|
//! | 0x00   | IC_CON           | Master-mode config; stored, largely inert       |
//! | 0x04   | IC_TAR           | Target slave address [6:0] (selects the slave)  |
//! | 0x08   | IC_SAR           | Slave address (stored; slave mode not modeled)  |
//! | 0x10   | IC_DATA_CMD      | Write = data + CMD/STOP/RESTART; read = pop RX   |
//! | 0x2c   | IC_INTR_STAT     | Masked interrupt status (raw & mask)            |
//! | 0x30   | IC_INTR_MASK     | Interrupt enable mask                           |
//! | 0x34   | IC_RAW_INTR_STAT | Raw interrupt status                           |
//! | 0x38   | IC_RX_TL         | RX FIFO threshold (stored)                      |
//! | 0x3c   | IC_TX_TL         | TX FIFO threshold (stored)                      |
//! | 0x40   | IC_CLR_INTR      | Read-to-clear all software-clearable ints       |
//! | 0x54   | IC_CLR_TX_ABRT   | Read-to-clear TX_ABRT + IC_TX_ABRT_SOURCE       |
//! | 0x60   | IC_CLR_STOP_DET  | Read-to-clear STOP_DET                          |
//! | 0x6c   | IC_ENABLE        | bit0 = ENABLE                                   |
//! | 0x70   | IC_STATUS        | ACTIVITY/TFNF/TFE/RFNE/RFF/MST_ACTIVITY         |
//! | 0x74   | IC_TXFLR         | TX FIFO level (0 — drained synchronously)       |
//! | 0x78   | IC_RXFLR         | RX FIFO level                                   |
//! | 0x80   | IC_TX_ABRT_SOURCE| Abort reason (ADDR NACK)                        |
//! | 0x9c   | IC_ENABLE_STATUS | bit0 mirrors IC_ENABLE                          |
//! | 0xf8   | IC_COMP_VERSION  | DesignWare component version constant           |
//! | 0xfc   | IC_COMP_TYPE     | DesignWare "DW_apb_i2c" magic (0x4457_0140)      |
//!
//! Timing / SCL-count registers (IC_SS_SCL_HCNT … IC_FS_SPKLEN) and the other
//! IC_CLR_* strobes accept-and-ignore (stored where a read-back is needed).
//!
//! ## Fidelity gaps (for the later silicon-verify pass)
//! * No cycle timing — commands complete instantly, so TXFLR is always 0 and
//!   TX_EMPTY is always asserted. A cycle-exact pass must schedule byte times.
//! * Slave (target) mode is not modeled — only master transactions.
//! * `IC_TX_ABRT_SOURCE` only reports the 7-bit-address-NACK reason.
//! * `IC_CON` speed/addressing-mode bits are stored but do not alter behavior.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use crate::peripherals::i2c::I2cDevice;
use crate::{Peripheral, PeripheralTickResult, SimResult};

pub const I2C0_BASE: u32 = 0x4004_4000;
pub const I2C0_SIZE: u64 = 0x1000;
pub const I2C1_BASE: u32 = 0x4004_8000;
pub const I2C1_SIZE: u64 = 0x1000;

/// RP2040 NVIC IRQ line for I2C0 (`I2C0_IRQ`, §2.3.2). I2C1 is 24.
pub const I2C0_IRQ: u32 = 23;
pub const I2C1_IRQ: u32 = 24;

// ── Register offsets ─────────────────────────────────────────────────────────
const IC_CON: u64 = 0x00;
const IC_TAR: u64 = 0x04;
const IC_SAR: u64 = 0x08;
const IC_DATA_CMD: u64 = 0x10;
const IC_SS_SCL_HCNT: u64 = 0x14;
const IC_SS_SCL_LCNT: u64 = 0x18;
const IC_FS_SCL_HCNT: u64 = 0x1c;
const IC_FS_SCL_LCNT: u64 = 0x20;
const IC_INTR_STAT: u64 = 0x2c;
const IC_INTR_MASK: u64 = 0x30;
const IC_RAW_INTR_STAT: u64 = 0x34;
const IC_RX_TL: u64 = 0x38;
const IC_TX_TL: u64 = 0x3c;
const IC_CLR_INTR: u64 = 0x40;
const IC_CLR_RX_UNDER: u64 = 0x44;
const IC_CLR_RX_OVER: u64 = 0x48;
const IC_CLR_TX_OVER: u64 = 0x4c;
const IC_CLR_RD_REQ: u64 = 0x50;
const IC_CLR_TX_ABRT: u64 = 0x54;
const IC_CLR_RX_DONE: u64 = 0x58;
const IC_CLR_ACTIVITY: u64 = 0x5c;
const IC_CLR_STOP_DET: u64 = 0x60;
const IC_CLR_START_DET: u64 = 0x64;
const IC_CLR_GEN_CALL: u64 = 0x68;
const IC_ENABLE: u64 = 0x6c;
const IC_STATUS: u64 = 0x70;
const IC_TXFLR: u64 = 0x74;
const IC_RXFLR: u64 = 0x78;
const IC_SDA_HOLD: u64 = 0x7c;
const IC_TX_ABRT_SOURCE: u64 = 0x80;
const IC_ENABLE_STATUS: u64 = 0x9c;
const IC_FS_SPKLEN: u64 = 0xa0;
const IC_COMP_PARAM_1: u64 = 0xf4;
const IC_COMP_VERSION: u64 = 0xf8;
const IC_COMP_TYPE: u64 = 0xfc;

// ── IC_DATA_CMD write fields ─────────────────────────────────────────────────
const DATA_CMD_DAT_MASK: u32 = 0xFF;
const DATA_CMD_CMD: u32 = 1 << 8; // 1 = read, 0 = write
const DATA_CMD_STOP: u32 = 1 << 9; // issue STOP after this command
const DATA_CMD_RESTART: u32 = 1 << 10; // issue RESTART before this command

// ── IC_STATUS bits ───────────────────────────────────────────────────────────
const STATUS_ACTIVITY: u32 = 1 << 0;
const STATUS_TFNF: u32 = 1 << 1; // TX FIFO not full
const STATUS_TFE: u32 = 1 << 2; // TX FIFO empty
const STATUS_RFNE: u32 = 1 << 3; // RX FIFO not empty
const STATUS_RFF: u32 = 1 << 4; // RX FIFO full
const STATUS_MST_ACTIVITY: u32 = 1 << 5;

// ── Raw interrupt bits (IC_RAW_INTR_STAT) ────────────────────────────────────
pub const INTR_RX_OVER: u32 = 1 << 1;
pub const INTR_RX_FULL: u32 = 1 << 2;
pub const INTR_TX_EMPTY: u32 = 1 << 4;
pub const INTR_TX_ABRT: u32 = 1 << 6;
pub const INTR_STOP_DET: u32 = 1 << 9;
pub const INTR_START_DET: u32 = 1 << 10;

/// Software-clearable raw-interrupt bits cleared by reading IC_CLR_INTR.
/// (TX_EMPTY / RX_FULL are hardware-driven status, not clearable this way.)
const INTR_CLEARABLE: u32 = INTR_RX_OVER | INTR_TX_ABRT | INTR_STOP_DET | INTR_START_DET;

// ── IC_TX_ABRT_SOURCE bits ───────────────────────────────────────────────────
const ABRT_7B_ADDR_NOACK: u32 = 1 << 0;

/// RP2040 DW_apb_i2c FIFO depth (both TX and RX are 16 entries deep).
const FIFO_DEPTH: usize = 16;

/// DesignWare component identity constants read back by the SDK/HAL.
const COMP_TYPE_MAGIC: u32 = 0x4457_0140; // "DW_apb_i2c"
const COMP_VERSION: u32 = 0x3230_312A; // "2.01*"
/// IC_COMP_PARAM_1: RX buffer depth [15:8] and TX buffer depth [23:16] each
/// encode (depth-1) = 15; MAX_SPEED_MODE = fast (0b10) in [3:2].
const COMP_PARAM_1: u32 = (15 << 16) | (15 << 8) | (0b10 << 2);

pub struct Rp2040I2c {
    con: u32,
    tar: u32,
    sar: u32,
    intr_mask: u32,
    rx_tl: u32,
    tx_tl: u32,
    enable: u32,
    sda_hold: u32,
    // Timing/SCL count registers — stored for read-back, no behavioral effect.
    ss_scl_hcnt: u32,
    ss_scl_lcnt: u32,
    fs_scl_hcnt: u32,
    fs_scl_lcnt: u32,
    fs_spklen: u32,

    /// Raw interrupt status. `Cell` because IC_CLR_* strobes clear bits on a
    /// read (which takes `&self`), like real read-to-clear silicon.
    raw_intr: Cell<u32>,
    tx_abrt_source: Cell<u32>,
    /// RX FIFO. `RefCell` because IC_DATA_CMD reads (`&self`) pop it.
    rx_fifo: RefCell<VecDeque<u8>>,

    /// Index into `slaves` of the device currently addressed, or `None` when the
    /// bus is idle (before the first command / after a STOP).
    active: Option<usize>,
    slaves: Vec<Box<dyn I2cDevice>>,

    /// NVIC line this instance asserts (23 for I2C0, 24 for I2C1).
    irq: u32,
}

impl Rp2040I2c {
    pub fn new() -> Self {
        Self {
            con: 0,
            tar: 0,
            sar: 0,
            intr_mask: 0,
            rx_tl: 0,
            tx_tl: 0,
            enable: 0,
            sda_hold: 0,
            ss_scl_hcnt: 0,
            ss_scl_lcnt: 0,
            fs_scl_hcnt: 0,
            fs_scl_lcnt: 0,
            fs_spklen: 0,
            raw_intr: Cell::new(0),
            tx_abrt_source: Cell::new(0),
            rx_fifo: RefCell::new(VecDeque::with_capacity(FIFO_DEPTH)),
            active: None,
            slaves: Vec::new(),
            irq: I2C0_IRQ,
        }
    }

    /// Construct an instance asserting a specific NVIC line — use [`I2C1_IRQ`]
    /// for the I2C1 controller at [`I2C1_BASE`].
    pub fn with_irq(irq: u32) -> Self {
        Self { irq, ..Self::new() }
    }

    /// Attach an I²C slave. Devices are matched by 7-bit address (`IC_TAR`) at
    /// transaction time; later additions win on duplicate addresses.
    pub fn attach_slave(&mut self, slave: Box<dyn I2cDevice>) {
        self.slaves.push(slave);
    }

    /// Execute one IC_DATA_CMD command against the attached slave, synchronously.
    fn process_data_cmd(&mut self, value: u32) {
        // Real silicon ignores IC_DATA_CMD writes while the controller is
        // disabled; firmware always enables first.
        if self.enable & 1 == 0 {
            return;
        }

        let data = (value & DATA_CMD_DAT_MASK) as u8;
        let is_read = value & DATA_CMD_CMD != 0;
        let stop = value & DATA_CMD_STOP != 0;
        let restart = value & DATA_CMD_RESTART != 0;

        // Begin a transaction (implicit START before the first byte), or issue a
        // repeated START when the RESTART bit is set mid-transaction.
        if self.active.is_none() {
            let addr = (self.tar & 0x7F) as u8;
            match self.slaves.iter().position(|s| s.address() == addr) {
                Some(idx) => {
                    self.slaves[idx].start();
                    self.active = Some(idx);
                    self.set_raw(INTR_START_DET);
                }
                None => {
                    // No slave acked the address → abort the transaction.
                    self.set_raw(INTR_TX_ABRT);
                    self.tx_abrt_source
                        .set(self.tx_abrt_source.get() | ABRT_7B_ADDR_NOACK);
                    return;
                }
            }
        } else if restart {
            if let Some(idx) = self.active {
                self.slaves[idx].start();
                self.set_raw(INTR_START_DET);
            }
        }

        let idx = self.active.expect("active set above");
        if is_read {
            let b = self.slaves[idx].read();
            let mut rx = self.rx_fifo.borrow_mut();
            if rx.len() < FIFO_DEPTH {
                rx.push_back(b);
            } else {
                drop(rx);
                self.set_raw(INTR_RX_OVER);
            }
        } else {
            self.slaves[idx].write(data);
        }

        if stop {
            self.slaves[idx].stop();
            self.active = None;
            self.set_raw(INTR_STOP_DET);
        }
    }

    #[inline]
    fn set_raw(&self, bits: u32) {
        self.raw_intr.set(self.raw_intr.get() | bits);
    }

    /// Raw interrupt status as seen by firmware: stored bits plus the
    /// always-asserted hardware status bits (TX_EMPTY — the TX FIFO is drained
    /// synchronously, so it is always at/below threshold).
    fn raw_intr_stat(&self) -> u32 {
        self.raw_intr.get() | INTR_TX_EMPTY
    }

    fn status(&self) -> u32 {
        let mut s = STATUS_TFNF | STATUS_TFE; // TX drained synchronously
        if self.active.is_some() {
            s |= STATUS_ACTIVITY | STATUS_MST_ACTIVITY;
        }
        let rx_len = self.rx_fifo.borrow().len();
        if rx_len != 0 {
            s |= STATUS_RFNE;
        }
        if rx_len >= FIFO_DEPTH {
            s |= STATUS_RFF;
        }
        s
    }
}

impl Default for Rp2040I2c {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Rp2040I2c {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rp2040I2c")
            .field("tar", &self.tar)
            .field("enable", &self.enable)
            .field("raw_intr", &self.raw_intr.get())
            .field("rx_fifo_len", &self.rx_fifo.borrow().len())
            .field("active", &self.active)
            .field("slaves", &self.slaves.len())
            .finish()
    }
}

impl Peripheral for Rp2040I2c {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // The SDK/HAL accesses every I2C register as a 32-bit word; route
        // through read_u32. Stray byte reads return 0 harmlessly.
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Byte writes ignored — all register writes arrive as 32-bit words.
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let v = match offset {
            IC_CON => self.con,
            IC_TAR => self.tar,
            IC_SAR => self.sar,
            // Pop the RX FIFO head; [11:8] would carry flags on real HW, 0 here.
            IC_DATA_CMD => self.rx_fifo.borrow_mut().pop_front().unwrap_or(0) as u32,
            IC_SS_SCL_HCNT => self.ss_scl_hcnt,
            IC_SS_SCL_LCNT => self.ss_scl_lcnt,
            IC_FS_SCL_HCNT => self.fs_scl_hcnt,
            IC_FS_SCL_LCNT => self.fs_scl_lcnt,
            IC_INTR_STAT => self.raw_intr_stat() & self.intr_mask,
            IC_INTR_MASK => self.intr_mask,
            IC_RAW_INTR_STAT => self.raw_intr_stat(),
            IC_RX_TL => self.rx_tl,
            IC_TX_TL => self.tx_tl,
            // Read-to-clear strobes: reading returns the pre-clear value and
            // clears the corresponding raw bit(s).
            IC_CLR_INTR => {
                let prev = self.raw_intr.get();
                self.raw_intr.set(prev & !INTR_CLEARABLE);
                self.tx_abrt_source.set(0);
                0
            }
            IC_CLR_TX_ABRT => {
                self.raw_intr.set(self.raw_intr.get() & !INTR_TX_ABRT);
                self.tx_abrt_source.set(0);
                0
            }
            IC_CLR_STOP_DET => {
                self.raw_intr.set(self.raw_intr.get() & !INTR_STOP_DET);
                0
            }
            IC_CLR_START_DET => {
                self.raw_intr.set(self.raw_intr.get() & !INTR_START_DET);
                0
            }
            IC_CLR_RX_OVER => {
                self.raw_intr.set(self.raw_intr.get() & !INTR_RX_OVER);
                0
            }
            IC_CLR_RX_UNDER | IC_CLR_TX_OVER | IC_CLR_RD_REQ | IC_CLR_RX_DONE
            | IC_CLR_ACTIVITY | IC_CLR_GEN_CALL => 0,
            IC_ENABLE => self.enable,
            IC_STATUS => self.status(),
            IC_TXFLR => 0, // TX FIFO drained synchronously
            IC_RXFLR => self.rx_fifo.borrow().len() as u32,
            IC_SDA_HOLD => self.sda_hold,
            IC_TX_ABRT_SOURCE => self.tx_abrt_source.get(),
            IC_ENABLE_STATUS => self.enable & 1,
            IC_FS_SPKLEN => self.fs_spklen,
            IC_COMP_PARAM_1 => COMP_PARAM_1,
            IC_COMP_VERSION => COMP_VERSION,
            IC_COMP_TYPE => COMP_TYPE_MAGIC,
            _ => 0,
        };
        Ok(v)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            IC_CON => self.con = value,
            IC_TAR => self.tar = value,
            IC_SAR => self.sar = value,
            IC_DATA_CMD => self.process_data_cmd(value),
            IC_SS_SCL_HCNT => self.ss_scl_hcnt = value,
            IC_SS_SCL_LCNT => self.ss_scl_lcnt = value,
            IC_FS_SCL_HCNT => self.fs_scl_hcnt = value,
            IC_FS_SCL_LCNT => self.fs_scl_lcnt = value,
            IC_INTR_MASK => self.intr_mask = value,
            IC_RX_TL => self.rx_tl = value,
            IC_TX_TL => self.tx_tl = value,
            IC_ENABLE => {
                self.enable = value;
                // Disabling aborts any in-flight transaction and flushes state.
                if value & 1 == 0 {
                    self.active = None;
                }
            }
            IC_SDA_HOLD => self.sda_hold = value,
            IC_FS_SPKLEN => self.fs_spklen = value,
            // Timing regs & IC_CLR_* / status regs: accept-and-ignore.
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Level-style interrupt: assert this controller's NVIC line while any
        // enabled interrupt is pending. `explicit_irqs` pends the numeric line
        // directly, so delivery does not depend on a yaml `irq:` being set.
        let pending = self.raw_intr_stat() & self.intr_mask != 0;
        PeripheralTickResult {
            explicit_irqs: if pending { Some(vec![self.irq]) } else { None },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::esp32s3::tmp102::Tmp102;

    #[test]
    fn base_and_irq_constants() {
        assert_eq!(I2C0_BASE, 0x4004_4000);
        assert_eq!(I2C1_BASE, 0x4004_8000);
        assert_eq!(I2C0_IRQ, 23);
        assert_eq!(I2C1_IRQ, 24);
    }

    #[test]
    fn comp_type_magic_identifies_dw_apb_i2c() {
        let p = Rp2040I2c::new();
        assert_eq!(p.read_u32(IC_COMP_TYPE).unwrap(), 0x4457_0140);
    }

    #[test]
    fn status_reset_has_tx_empty_flags() {
        let p = Rp2040I2c::new();
        let s = p.read_u32(IC_STATUS).unwrap();
        assert_eq!(s & STATUS_TFNF, STATUS_TFNF, "TFNF set at reset");
        assert_eq!(s & STATUS_TFE, STATUS_TFE, "TFE set at reset");
        assert_eq!(s & STATUS_RFNE, 0, "RFNE clear (RX empty) at reset");
    }

    #[test]
    fn tar_and_enable_round_trip() {
        let mut p = Rp2040I2c::new();
        p.write_u32(IC_TAR, 0x48).unwrap();
        p.write_u32(IC_ENABLE, 1).unwrap();
        assert_eq!(p.read_u32(IC_TAR).unwrap(), 0x48);
        assert_eq!(p.read_u32(IC_ENABLE).unwrap(), 1);
        assert_eq!(p.read_u32(IC_ENABLE_STATUS).unwrap(), 1);
    }

    /// Full master write_read against a TMP102: set pointer 0, restart-read 2
    /// bytes → 0x19, 0x00 (25.0 °C left-justified).
    #[test]
    fn write_read_drives_attached_tmp102() {
        let mut p = Rp2040I2c::new();
        p.attach_slave(Box::new(Tmp102::new()));

        p.write_u32(IC_ENABLE, 1).unwrap();
        p.write_u32(IC_TAR, 0x48).unwrap();

        // Write pointer 0x00, no STOP (keeps the bus for the repeated start).
        p.write_u32(IC_DATA_CMD, 0x00).unwrap();
        // Read byte 1 with RESTART, byte 2 with STOP.
        p.write_u32(IC_DATA_CMD, DATA_CMD_RESTART | DATA_CMD_CMD).unwrap();
        p.write_u32(IC_DATA_CMD, DATA_CMD_STOP | DATA_CMD_CMD).unwrap();

        // RX FIFO now holds the two temperature bytes.
        assert_eq!(p.read_u32(IC_RXFLR).unwrap(), 2);
        assert_ne!(p.read_u32(IC_STATUS).unwrap() & STATUS_RFNE, 0);
        assert_eq!(p.read_u32(IC_DATA_CMD).unwrap() & 0xFF, 0x19);
        assert_eq!(p.read_u32(IC_DATA_CMD).unwrap() & 0xFF, 0x00);

        // STOP raised STOP_DET; ACTIVITY cleared after the STOP.
        assert_ne!(p.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_STOP_DET, 0);
        assert_eq!(p.read_u32(IC_STATUS).unwrap() & STATUS_ACTIVITY, 0);
    }

    #[test]
    fn read_config_register_via_pointer() {
        // Point at CONFIG (0x01); TMP102 returns canned 0x60A0.
        let mut p = Rp2040I2c::new();
        p.attach_slave(Box::new(Tmp102::new()));
        p.write_u32(IC_ENABLE, 1).unwrap();
        p.write_u32(IC_TAR, 0x48).unwrap();

        p.write_u32(IC_DATA_CMD, 0x01).unwrap(); // pointer = CONFIG
        p.write_u32(IC_DATA_CMD, DATA_CMD_RESTART | DATA_CMD_CMD).unwrap();
        p.write_u32(IC_DATA_CMD, DATA_CMD_STOP | DATA_CMD_CMD).unwrap();

        assert_eq!(p.read_u32(IC_DATA_CMD).unwrap() & 0xFF, 0x60);
        assert_eq!(p.read_u32(IC_DATA_CMD).unwrap() & 0xFF, 0xA0);
    }

    #[test]
    fn unmatched_address_aborts_with_nack_source() {
        let mut p = Rp2040I2c::new();
        p.attach_slave(Box::new(Tmp102::new())); // addr 0x48
        p.write_u32(IC_ENABLE, 1).unwrap();
        p.write_u32(IC_TAR, 0x50).unwrap(); // nobody home
        p.write_u32(IC_DATA_CMD, 0x00).unwrap();

        assert_ne!(p.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
        assert_ne!(p.read_u32(IC_TX_ABRT_SOURCE).unwrap() & ABRT_7B_ADDR_NOACK, 0);
        // Reading IC_CLR_TX_ABRT clears the abort.
        let _ = p.read_u32(IC_CLR_TX_ABRT).unwrap();
        assert_eq!(p.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
        assert_eq!(p.read_u32(IC_TX_ABRT_SOURCE).unwrap(), 0);
    }

    #[test]
    fn data_cmd_ignored_while_disabled() {
        let mut p = Rp2040I2c::new();
        p.attach_slave(Box::new(Tmp102::new()));
        p.write_u32(IC_TAR, 0x48).unwrap();
        // No enable → command dropped, no RX data, no activity.
        p.write_u32(IC_DATA_CMD, DATA_CMD_CMD).unwrap();
        assert_eq!(p.read_u32(IC_RXFLR).unwrap(), 0);
    }

    #[test]
    fn clr_intr_clears_stop_and_start() {
        let mut p = Rp2040I2c::new();
        p.attach_slave(Box::new(Tmp102::new()));
        p.write_u32(IC_ENABLE, 1).unwrap();
        p.write_u32(IC_TAR, 0x48).unwrap();
        p.write_u32(IC_DATA_CMD, DATA_CMD_STOP).unwrap(); // write + stop
        assert_ne!(p.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_STOP_DET, 0);
        let _ = p.read_u32(IC_CLR_INTR).unwrap();
        assert_eq!(p.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_STOP_DET, 0);
        assert_eq!(p.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_START_DET, 0);
    }

    #[test]
    fn tick_asserts_irq_when_masked_intr_pending() {
        let mut p = Rp2040I2c::new();
        p.attach_slave(Box::new(Tmp102::new()));
        p.write_u32(IC_ENABLE, 1).unwrap();
        p.write_u32(IC_TAR, 0x48).unwrap();
        p.write_u32(IC_INTR_MASK, INTR_STOP_DET).unwrap();
        p.write_u32(IC_DATA_CMD, DATA_CMD_STOP).unwrap();
        let r = p.tick();
        assert_eq!(r.explicit_irqs, Some(vec![I2C0_IRQ]));
    }
}
