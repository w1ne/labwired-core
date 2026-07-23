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
//! | 0x100  | TXFIFO_START_ADDR | RO window into TX FIFO RAM (peek head)   |
//! | 0x180  | RXFIFO_START_ADDR | RO window into RX FIFO RAM (peek head)   |
//!
//! All other offsets accept writes silently and read 0.
//!
//! The same model serves both controllers: I2C0 (base 0x6001_3000, source 42)
//! and I2C1 (base 0x6002_7000, source 43) — construct the second instance with
//! [`Esp32s3I2c::with_intr_source`].

use std::cell::RefCell;

use crate::peripherals::i2c::I2cDevice;
use crate::{Peripheral, PeripheralTickResult, SimResult};

pub const I2C0_BASE: u32 = 0x6001_3000;
pub const I2C0_SIZE: u64 = 0x1000;

/// ESP32-S3 I2C0 (I2C_EXT0) peripheral interrupt source number.
///
/// This is the interrupt-matrix source index the firmware uses to program the
/// `INTERRUPT_CORE{n}_I2C_EXT0_MAP_REG` (at matrix offset `4 * source`) and the
/// value ESP-IDF's `esp_intr_alloc` routes — it MUST match the ESP-IDF
/// `ets_isr_source_t` ordinal, i.e. `ETS_I2C_EXT0_INTR_SOURCE = 42`, NOT the
/// general interrupt-status bit numbering. (It was previously 49, which is an
/// unrelated source the firmware leaves parked at the disabled default CPU
/// interrupt 6, so I2C0 completion interrupts were never delivered and ESP-IDF's
/// interrupt-driven `i2c_master` returned `ESP_ERR_INVALID_STATE`.)
pub const I2C0_INTR_SOURCE_ID: u32 = 42;

pub const I2C1_BASE: u32 = 0x6002_7000;
pub const I2C1_SIZE: u64 = 0x1000;

/// ESP32-S3 I2C1 (I2C_EXT1) peripheral interrupt source number —
/// `ETS_I2C_EXT1_INTR_SOURCE`, immediately after I2C_EXT0 (see the
/// [`I2C0_INTR_SOURCE_ID`] doc for why the ordinal matters).
pub const I2C1_INTR_SOURCE_ID: u32 = 43;

// Core FSM / status registers
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
// Read-only APB windows into the FIFO RAM (TRM §29.6: TXFIFO_START_ADDR /
// RXFIFO_START_ADDR). Reading shows the FIFO head byte without consuming it.
const REG_TXFIFO_START: u64 = 0x100;
const REG_RXFIFO_START: u64 = 0x180;

// Config / timing registers (SVD-sourced offsets, reset values, write masks)
const REG_SCL_LOW_PERIOD: u64 = 0x00;
const REG_TO: u64 = 0x0C;
const REG_SDA_HOLD: u64 = 0x30;
const REG_SDA_SAMPLE: u64 = 0x34;
const REG_SCL_HIGH_PERIOD: u64 = 0x38;
const REG_SCL_START_HOLD: u64 = 0x40;
const REG_SCL_RSTART_SETUP: u64 = 0x44;
const REG_SCL_STOP_HOLD: u64 = 0x48;
const REG_SCL_STOP_SETUP: u64 = 0x4C;
const REG_FILTER_CFG: u64 = 0x50;
const REG_CLK_CONF: u64 = 0x54;
const REG_SCL_ST_TIME_OUT: u64 = 0x78;
const REG_SCL_MAIN_ST_TIME_OUT: u64 = 0x7C;
const REG_SCL_SP_CONF: u64 = 0x80;
const REG_SCL_STRETCH_CONF: u64 = 0x84;
const REG_DATE: u64 = 0xF8;

const CTR_TRANS_START_BIT: u32 = 1 << 5;
/// CTR bit 10: FSM_RST — write-trigger master FSM reset.
const CTR_FSM_RST: u32 = 1 << 10;
/// CTR bit 11: CONF_UPGATE — self-clearing config-sync trigger.
const CTR_CONF_UPGATE: u32 = 1 << 11;

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
const SCL_RST_SLV_EN: u32 = 1 << 0;

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
    /// Interrupt-matrix source this instance asserts: 42 for I2C0, 43 for I2C1.
    intr_source_id: u32,
    active_slave: Option<usize>,
    expects_addr: bool,

    // Config / timing registers — masked storage (SVD-accurate reset values).
    // On write: stored = (stored & !mask) | (value & mask).  Reserved bits
    // read back their reset value, not arbitrary written data.
    reg_scl_low_period: u32,   // 0x00  reset 0x0000_0000  mask 0x0000_01FF
    reg_to: u32,               // 0x0C  reset 0x0000_0010  mask 0x0000_003F
    reg_sda_hold: u32,         // 0x30  reset 0x0000_0000  mask 0x0000_01FF
    reg_sda_sample: u32,       // 0x34  reset 0x0000_0000  mask 0x0000_01FF
    reg_scl_high_period: u32,  // 0x38  reset 0x0000_0000  mask 0x0000_FFFF
    reg_scl_start_hold: u32,   // 0x40  reset 0x0000_0008  mask 0x0000_01FF
    reg_scl_rstart_setup: u32, // 0x44  reset 0x0000_0008  mask 0x0000_01FF
    reg_scl_stop_hold: u32,    // 0x48  reset 0x0000_0008  mask 0x0000_01FF
    reg_scl_stop_setup: u32,   // 0x4C  reset 0x0000_0008  mask 0x0000_01FF
    reg_filter_cfg: u32,       // 0x50  reset 0x0000_0300  mask 0x0000_03FF
    reg_clk_conf: u32,         // 0x54  reset 0x0020_0000  mask 0x003F_FFFF
    reg_scl_st_time_out: u32,  // 0x78  reset 0x0000_0010  mask 0x0000_001F
    reg_scl_main_st_time_out: u32, // 0x7C  reset 0x0000_0010  mask 0x0000_001F
    reg_scl_sp_conf: u32,      // 0x80  reset 0x0000_0000  mask 0x0000_00FF
    reg_scl_stretch_conf: u32, // 0x84  reset 0x0000_0000  mask 0x0000_3FFF
    reg_date: u32,             // 0xF8  reset 0x2007_0201  mask 0xFFFF_FFFF
}

impl Esp32s3I2c {
    pub fn new() -> Self {
        Self {
            // SVD reset values for core FSM registers.
            // CTR reset 0x020B: SCL_FORCE_OUT(0)|SDA_FORCE_OUT(1)|SAMPLE_SCL_LEVEL(3)|RX_FULL_ACK_LEVEL(9).
            ctr: 0x0000_020B,
            sr: 0,
            slave_addr: 0,
            // INT_RAW bit 1 (TXFIFO_WM_INT_RAW) is set at reset: TX FIFO is empty,
            // which is at or below the watermark threshold.
            int_raw: 0x0000_0002,
            int_ena: 0,
            // FIFO_CONF reset 0x408B: RXFIFO_WM_THRHD[0:4]=0xB, TXFIFO_WM_THRHD[5:9]=0x4.
            fifo_conf: 0x0000_408B,
            cmds: [0; NUM_CMDS],
            tx_fifo: std::collections::VecDeque::with_capacity(FIFO_CAPACITY),
            rx_fifo: RefCell::new(std::collections::VecDeque::with_capacity(FIFO_CAPACITY)),
            slaves: Vec::new(),
            irq_pending: false,
            intr_source_id: I2C0_INTR_SOURCE_ID,
            active_slave: None,
            expects_addr: true,

            // Config / timing registers initialised to SVD reset values.
            reg_scl_low_period: 0x0000_0000,
            reg_to: 0x0000_0010,
            reg_sda_hold: 0x0000_0000,
            reg_sda_sample: 0x0000_0000,
            reg_scl_high_period: 0x0000_0000,
            reg_scl_start_hold: 0x0000_0008,
            reg_scl_rstart_setup: 0x0000_0008,
            reg_scl_stop_hold: 0x0000_0008,
            reg_scl_stop_setup: 0x0000_0008,
            reg_filter_cfg: 0x0000_0300,
            reg_clk_conf: 0x0020_0000,
            reg_scl_st_time_out: 0x0000_0010,
            reg_scl_main_st_time_out: 0x0000_0010,
            reg_scl_sp_conf: 0x0000_0000,
            reg_scl_stretch_conf: 0x0000_0000,
            reg_date: 0x2007_0201,
        }
    }

    /// Construct an instance asserting a different interrupt-matrix source —
    /// use [`I2C1_INTR_SOURCE_ID`] for the I2C1 controller at [`I2C1_BASE`].
    pub fn with_intr_source(intr_source_id: u32) -> Self {
        Self {
            intr_source_id,
            ..Self::new()
        }
    }

    /// Raw slave push — does NOT wrap for tracing. The only production caller is
    /// the bus choke point [`crate::bus::SystemBus::attach_i2c_slave`], which
    /// wraps first. Slaves are matched by address bits at transaction time;
    /// later additions take precedence on duplicate addresses.
    pub(crate) fn push_slave(&mut self, slave: Box<dyn I2cDevice>) {
        self.slaves.push(slave);
    }

    /// Borrow the attached I²C slaves. Mirrors the generic `I2c::attached_devices`
    /// and `Esp32c3I2c::attached_slaves` accessors so UI/inspection paths (e.g. the
    /// SSD1306 framebuffer readback) can enumerate devices on the ESP32-S3
    /// command-list controller the same way they do on the STM32 and ESP32-C3
    /// controllers. Slaves are held directly (no `RefCell`) because the S3 engine
    /// never hands out interior mutable references during a transaction.
    pub fn attached_slaves(&self) -> &[Box<dyn I2cDevice>] {
        &self.slaves
    }

    /// Mutable counterpart of [`Self::attached_slaves`] — see
    /// `Esp32c3I2c::attached_slaves_mut` for why this exists.
    pub fn attached_slaves_mut(&mut self) -> &mut [Box<dyn I2cDevice>] {
        &mut self.slaves
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
        //   bits 14..15 STRETCH_CAUSE — reset value 0b11 (silicon default)
        //   bits 18..23 TXFIFO_CNT
        const SR_STRETCH_CAUSE_RESET: u32 = 0x0000_C000;
        let rx = (self.rx_fifo.borrow().len() as u32) & 0x3F;
        let tx = (self.tx_fifo.len() as u32) & 0x3F;
        (self.sr & SR_RESP_REC) | SR_STRETCH_CAUSE_RESET | (rx << 8) | (tx << 18)
    }

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
            REG_SCL_LOW_PERIOD => self.reg_scl_low_period,
            REG_CTR => self.ctr,
            REG_SR => self.status_register(),
            REG_TO => self.reg_to,
            REG_SLAVE_ADDR => self.slave_addr,
            REG_DATA => self.rx_fifo.borrow_mut().pop_front().unwrap_or(0) as u32,
            REG_FIFO_CONF => self.fifo_conf,
            REG_INT_RAW => self.int_raw,
            REG_INT_CLR => 0,
            REG_INT_ENA => self.int_ena,
            REG_INT_ST => self.int_raw & self.int_ena,
            REG_FIFO_ST => self.fifo_status(),
            REG_SDA_HOLD => self.reg_sda_hold,
            REG_SDA_SAMPLE => self.reg_sda_sample,
            REG_SCL_HIGH_PERIOD => self.reg_scl_high_period,
            REG_SCL_START_HOLD => self.reg_scl_start_hold,
            REG_SCL_RSTART_SETUP => self.reg_scl_rstart_setup,
            REG_SCL_STOP_HOLD => self.reg_scl_stop_hold,
            REG_SCL_STOP_SETUP => self.reg_scl_stop_setup,
            REG_FILTER_CFG => self.reg_filter_cfg,
            REG_CLK_CONF => self.reg_clk_conf,
            REG_CMD0..=REG_CMD7 => {
                let idx = ((offset - REG_CMD0) / 4) as usize;
                self.cmds.get(idx).copied().unwrap_or(0)
            }
            REG_SCL_ST_TIME_OUT => self.reg_scl_st_time_out,
            REG_SCL_MAIN_ST_TIME_OUT => self.reg_scl_main_st_time_out,
            REG_SCL_SP_CONF => self.reg_scl_sp_conf,
            REG_SCL_STRETCH_CONF => self.reg_scl_stretch_conf,
            REG_DATE => self.reg_date,
            // Read-only FIFO-RAM windows: peek the head byte, never consume.
            REG_TXFIFO_START => self.tx_fifo.front().copied().unwrap_or(0) as u32,
            REG_RXFIFO_START => self.rx_fifo.borrow().front().copied().unwrap_or(0) as u32,
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
        /// Apply write mask: only writable bits store; reserved bits keep their reset value.
        #[inline(always)]
        fn masked_write(stored: &mut u32, value: u32, mask: u32) {
            *stored = (*stored & !mask) | (value & mask);
        }

        match offset {
            REG_SCL_LOW_PERIOD => masked_write(&mut self.reg_scl_low_period, value, 0x0000_01FF),
            REG_CTR => {
                self.ctr = value;
                if value & CTR_TRANS_START_BIT != 0 {
                    self.run_command_list();
                    // Auto-clear TRANS_START like real silicon.
                    self.ctr &= !CTR_TRANS_START_BIT;
                }
                // CONF_UPGATE (bit 11) is a self-clearing sync trigger: writing 1
                // latches the timing/config registers into the FSM, then the bit
                // clears automatically (ESP32-S3 TRM §29.4). ESP-IDF's i2c_master
                // driver writes it and polls for it to clear; if it stays set the
                // driver concludes the controller is wedged and aborts the
                // transfer (ESP_ERR_INVALID_STATE). esp-hal never sets it.
                self.ctr &= !(CTR_FSM_RST | CTR_CONF_UPGATE);
            }
            REG_TO => masked_write(&mut self.reg_to, value, 0x0000_003F),
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
            REG_SDA_HOLD => masked_write(&mut self.reg_sda_hold, value, 0x0000_01FF),
            REG_SDA_SAMPLE => masked_write(&mut self.reg_sda_sample, value, 0x0000_01FF),
            REG_SCL_HIGH_PERIOD => masked_write(&mut self.reg_scl_high_period, value, 0x0000_FFFF),
            REG_SCL_START_HOLD => masked_write(&mut self.reg_scl_start_hold, value, 0x0000_01FF),
            REG_SCL_RSTART_SETUP => {
                masked_write(&mut self.reg_scl_rstart_setup, value, 0x0000_01FF)
            }
            REG_SCL_STOP_HOLD => masked_write(&mut self.reg_scl_stop_hold, value, 0x0000_01FF),
            REG_SCL_STOP_SETUP => masked_write(&mut self.reg_scl_stop_setup, value, 0x0000_01FF),
            REG_FILTER_CFG => masked_write(&mut self.reg_filter_cfg, value, 0x0000_03FF),
            REG_CLK_CONF => masked_write(&mut self.reg_clk_conf, value, 0x003F_FFFF),
            REG_CMD0..=REG_CMD7 => {
                let idx = ((offset - REG_CMD0) / 4) as usize;
                if let Some(slot) = self.cmds.get_mut(idx) {
                    *slot = value;
                }
            }
            REG_SCL_ST_TIME_OUT => masked_write(&mut self.reg_scl_st_time_out, value, 0x0000_001F),
            REG_SCL_MAIN_ST_TIME_OUT => {
                masked_write(&mut self.reg_scl_main_st_time_out, value, 0x0000_001F)
            }
            REG_SCL_SP_CONF => {
                masked_write(&mut self.reg_scl_sp_conf, value, 0x0000_00FF);
                // SCL_RST_SLV_EN is R/W/SC. Arduino's S3 bus-clear helper
                // writes it and then polls until hardware clears it.
                self.reg_scl_sp_conf &= !SCL_RST_SLV_EN;
            }
            REG_SCL_STRETCH_CONF => {
                masked_write(&mut self.reg_scl_stretch_conf, value, 0x0000_3FFF)
            }
            REG_DATE => self.reg_date = value, // fully writable (mask = 0xFFFF_FFFF)
            _ => {}                            // Accept-and-ignore (unmapped offsets)
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let mut explicit = Vec::new();
        // LEVEL interrupt: assert the I2C0 source every tick while any enabled
        // INT bit is set, mirroring real silicon (INT_RAW stays asserted until the
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
            explicit.push(self.intr_source_id);
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

        // Index into self.slaves of the currently-selected device. END pauses
        // the command list, so the selected slave can carry into the next run.
        let mut active = self.active_slave;
        let mut expects_addr = self.expects_addr;
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
                    // Zero-payload WRITE after RSTART: Wire probe via SLAVE_ADDR.
                    if expects_addr && byte_num == 0 {
                        active = self.find_slave_from_slave_addr_register();
                        if let Some(slave_idx) = active {
                            self.slaves[slave_idx].start();
                            self.sr |= SR_RESP_REC;
                        } else {
                            self.int_raw |= INT_NACK;
                        }
                        expects_addr = false;
                    }
                    for i in 0..byte_num {
                        let b = self.tx_fifo.pop_front().unwrap_or(0);
                        if expects_addr && i == 0 {
                            // First byte of a WRITE following RSTART is addr+R/W.
                            let addr = b >> 1;
                            active = self.slaves.iter().position(|s| s.address() == addr);
                            if let Some(slave_idx) = active {
                                // Slave acknowledged its address. Signal START
                                // to the selected device — on the wire the
                                // START + address frame precedes the data, and
                                // the bus-trace wrapper reconstructs the
                                // address frame from this call.
                                self.slaves[slave_idx].start();
                                self.sr |= SR_RESP_REC;
                                expects_addr = false;
                                // Don't deliver the addr byte to the slave's write().
                                continue;
                            }

                            // ESP-IDF/Arduino can program the address in
                            // SLAVE_ADDR and put only payload bytes in TXFIFO.
                            // In that shape the first FIFO byte is real data.
                            active = self.find_slave_from_slave_addr_register();
                            if let Some(slave_idx) = active {
                                self.slaves[slave_idx].start();
                                self.sr |= SR_RESP_REC;
                            } else {
                                self.int_raw |= INT_NACK;
                                expects_addr = false;
                                continue;
                            }
                            expects_addr = false;
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
                    active = None;
                    expects_addr = true;
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
            self.active_slave = active;
            self.expects_addr = expects_addr;
            self.int_raw |= INT_END_DETECT;
        } else if hit_stop {
            self.active_slave = None;
            self.expects_addr = true;
            self.int_raw |= INT_TRANS_COMPLETE;
        } else {
            // Empty cmd list or reserved opcode — set TRANS_COMPLETE so the
            // driver's wait loop unblocks rather than hanging.
            self.active_slave = None;
            self.expects_addr = true;
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
    fn i2c0_interrupt_source_is_ets_i2c_ext0() {
        // Regression guard: ESP-IDF routes I2C0 (I2C_EXT0) through interrupt
        // source 42 — its `ets_isr_source_t` ordinal (`ETS_I2C_EXT0_INTR_SOURCE
        // = 42` on ESP32-S3, verified against the toolchain SoC headers). The
        // interrupt matrix MAP register the firmware programs is at offset
        // `4 * source`, so `tick()` MUST assert this exact source for ESP-IDF's
        // interrupt-driven `i2c_master` ISR to be dispatched. A wrong value
        // (it was once 49) leaves the source parked at the disabled default
        // CPU interrupt and the driver fails with `ESP_ERR_INVALID_STATE`.
        assert_eq!(I2C0_INTR_SOURCE_ID, 42);
    }

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

    #[test]
    fn txfifo_start_addr_window_peeks_tx_fifo_non_destructively() {
        let mut p = Esp32s3I2c::new();
        assert_eq!(
            p.read_u32(REG_TXFIFO_START).unwrap(),
            0,
            "empty TX FIFO reads 0"
        );
        p.write_u32(REG_DATA, 0xAA).unwrap();
        p.write_u32(REG_DATA, 0xBB).unwrap();
        assert_eq!(p.read_u32(REG_TXFIFO_START).unwrap(), 0xAA);
        assert_eq!(
            p.read_u32(REG_TXFIFO_START).unwrap(),
            0xAA,
            "peek is non-destructive"
        );
        let sr = p.read_u32(REG_SR).unwrap();
        assert_eq!((sr >> 18) & 0x3F, 2, "peek must not consume TX FIFO bytes");
        // Read-only window: writes are ignored.
        p.write_u32(REG_TXFIFO_START, 0xFFFF_FFFF).unwrap();
        assert_eq!(p.read_u32(REG_TXFIFO_START).unwrap(), 0xAA);
    }

    #[test]
    fn rxfifo_start_addr_window_peeks_rx_fifo_non_destructively() {
        let p = Esp32s3I2c::new();
        assert_eq!(
            p.read_u32(REG_RXFIFO_START).unwrap(),
            0,
            "empty RX FIFO reads 0"
        );
        p.rx_fifo.borrow_mut().push_back(0x19);
        p.rx_fifo.borrow_mut().push_back(0x00);
        assert_eq!(p.read_u32(REG_RXFIFO_START).unwrap(), 0x19);
        assert_eq!(
            p.read_u32(REG_RXFIFO_START).unwrap(),
            0x19,
            "peek is non-destructive"
        );
        assert_eq!(
            p.read_u32(REG_DATA).unwrap(),
            0x19,
            "DATA pop unaffected by peeks"
        );
    }

    #[test]
    fn i2c1_instance_asserts_its_own_intr_source() {
        assert_eq!(I2C1_BASE, 0x6002_7000);
        // ETS_I2C_EXT1_INTR_SOURCE — must be the ets_isr_source_t ordinal,
        // immediately after ETS_I2C_EXT0_INTR_SOURCE (42).
        assert_eq!(I2C1_INTR_SOURCE_ID, 43);
        let mut p = Esp32s3I2c::with_intr_source(I2C1_INTR_SOURCE_ID);
        // NACK on an empty bus with INT_ENA set → tick asserts source 43.
        p.write_u32(REG_INT_ENA, INT_NACK).unwrap();
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 1)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_DATA, 0xA0).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        let r = p.tick();
        assert_eq!(r.explicit_irqs, Some(vec![I2C1_INTR_SOURCE_ID]));
    }

    use crate::peripherals::esp32s3::tmp102::Tmp102;

    #[test]
    fn write_read_drives_attached_tmp102() {
        let mut p = Esp32s3I2c::new();
        p.push_slave(Box::new(Tmp102::new()));

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

    /// Regression guard for the S3-OLED-blank-in-embed bug (the ESP32-S3 sibling
    /// of the ESP32-C3 `leo_oled_i2c_readback` guard). The playground/embed reads
    /// the OLED framebuffer through `WasmSimulator::get_ssd1306_framebuffer`, which
    /// enumerates I²C slaves on the named controller. That accessor understood the
    /// generic STM32 `I2c` and the `Esp32c3I2c`, but NOT the `Esp32s3I2c` — so an
    /// SSD1306 on an S3 board returned "not an I2C controller" and rendered blank.
    ///
    /// This test drives a real GDDRAM write through the S3 command-list engine
    /// (the exact register path firmware uses) and then reads the framebuffer back
    /// through `attached_slaves()` — the accessor `inspect.rs` now downcasts to —
    /// asserting the bytes are the real ones written, not a blank/zero fill.
    #[test]
    fn ssd1306_gddram_is_readable_through_esp32s3_i2c() {
        use crate::peripherals::components::Ssd1306;

        let mut p = Esp32s3I2c::new();
        p.push_slave(Box::new(Ssd1306::new(0x3C)));

        // Single I²C write to the OLED: RSTART; WRITE 6; STOP.
        // TX = [addr+W, control=0x40 (data stream), then four GDDRAM bytes].
        // Default addressing mode is horizontal from (page 0, col 0), so the four
        // data bytes land at gddram[0..4].
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 6)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();

        p.write_u32(REG_DATA, 0x78).unwrap(); // 0x3C << 1 | W
        p.write_u32(REG_DATA, 0x40).unwrap(); // Co=0, D/C#=1 → data stream
        p.write_u32(REG_DATA, 0xFF).unwrap();
        p.write_u32(REG_DATA, 0x3C).unwrap();
        p.write_u32(REG_DATA, 0xAA).unwrap();
        p.write_u32(REG_DATA, 0x55).unwrap();

        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
            INT_TRANS_COMPLETE,
            "the OLED write transaction must complete on the S3 engine"
        );

        // The exact enumeration path `get_ssd1306_framebuffer` uses for the S3.
        let oled = p
            .attached_slaves()
            .iter()
            .filter(|d| d.address() == 0x3C)
            .find_map(|d| d.as_any().and_then(|a| a.downcast_ref::<Ssd1306>()))
            .expect("an SSD1306 must be reachable at 0x3C through Esp32s3I2c");

        let fb = oled.framebuffer();
        assert_eq!(
            fb.len(),
            1024,
            "SSD1306 GDDRAM framebuffer must be 128x64/8 = 1024 bytes"
        );
        assert_eq!(
            &fb[0..4],
            &[0xFF, 0x3C, 0xAA, 0x55],
            "readback must return the real bytes written through the S3 I2C engine"
        );
        assert_eq!(
            oled.ink_bytes(),
            4,
            "exactly the four written bytes are lit"
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
        p.push_slave(Box::new(Tmp102::new()));

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

    // ── Config / timing register tests ───────────────────────────────────────

    #[test]
    fn i2c_config_registers_reset_values() {
        let p = Esp32s3I2c::new();
        assert_eq!(
            p.read_u32(REG_SCL_LOW_PERIOD).unwrap(),
            0x0000_0000,
            "SCL_LOW_PERIOD"
        );
        assert_eq!(p.read_u32(REG_TO).unwrap(), 0x0000_0010, "TO");
        assert_eq!(p.read_u32(REG_SDA_HOLD).unwrap(), 0x0000_0000, "SDA_HOLD");
        assert_eq!(
            p.read_u32(REG_SDA_SAMPLE).unwrap(),
            0x0000_0000,
            "SDA_SAMPLE"
        );
        assert_eq!(
            p.read_u32(REG_SCL_HIGH_PERIOD).unwrap(),
            0x0000_0000,
            "SCL_HIGH_PERIOD"
        );
        assert_eq!(
            p.read_u32(REG_SCL_START_HOLD).unwrap(),
            0x0000_0008,
            "SCL_START_HOLD"
        );
        assert_eq!(
            p.read_u32(REG_SCL_RSTART_SETUP).unwrap(),
            0x0000_0008,
            "SCL_RSTART_SETUP"
        );
        assert_eq!(
            p.read_u32(REG_SCL_STOP_HOLD).unwrap(),
            0x0000_0008,
            "SCL_STOP_HOLD"
        );
        assert_eq!(
            p.read_u32(REG_SCL_STOP_SETUP).unwrap(),
            0x0000_0008,
            "SCL_STOP_SETUP"
        );
        assert_eq!(
            p.read_u32(REG_FILTER_CFG).unwrap(),
            0x0000_0300,
            "FILTER_CFG"
        );
        assert_eq!(p.read_u32(REG_CLK_CONF).unwrap(), 0x0020_0000, "CLK_CONF");
        assert_eq!(
            p.read_u32(REG_SCL_ST_TIME_OUT).unwrap(),
            0x0000_0010,
            "SCL_ST_TIME_OUT"
        );
        assert_eq!(
            p.read_u32(REG_SCL_MAIN_ST_TIME_OUT).unwrap(),
            0x0000_0010,
            "SCL_MAIN_ST_TIME_OUT"
        );
        assert_eq!(
            p.read_u32(REG_SCL_SP_CONF).unwrap(),
            0x0000_0000,
            "SCL_SP_CONF"
        );
        assert_eq!(
            p.read_u32(REG_SCL_STRETCH_CONF).unwrap(),
            0x0000_0000,
            "SCL_STRETCH_CONF"
        );
        assert_eq!(p.read_u32(REG_DATE).unwrap(), 0x2007_0201, "DATE");
    }

    #[test]
    fn i2c_config_register_write_mask() {
        let mut p = Esp32s3I2c::new();

        // SCL_LOW_PERIOD: mask 0x1FF — upper bits must not store.
        p.write_u32(REG_SCL_LOW_PERIOD, 0xFFFF_FFFF).unwrap();
        assert_eq!(p.read_u32(REG_SCL_LOW_PERIOD).unwrap(), 0x0000_01FF);

        // FILTER_CFG: mask 0x3FF.
        p.write_u32(REG_FILTER_CFG, 0xFFFF_FFFF).unwrap();
        assert_eq!(p.read_u32(REG_FILTER_CFG).unwrap(), 0x0000_03FF);

        // SCL_SP_CONF: mask 0xFF, with SCL_RST_SLV_EN bit 0 self-clearing.
        p.write_u32(REG_SCL_SP_CONF, 0xFFFF_FFFF).unwrap();
        assert_eq!(p.read_u32(REG_SCL_SP_CONF).unwrap(), 0x0000_00FE);

        // TO: mask 0x3F — bits 6..31 are reserved, read back reset (0x10 & ~0x3F == 0,
        // so reserved bits are 0; full write reads back 0x3F only).
        p.write_u32(REG_TO, 0xFFFF_FFFF).unwrap();
        assert_eq!(p.read_u32(REG_TO).unwrap(), 0x0000_003F);

        // CLK_CONF: mask 0x3F_FFFF.
        p.write_u32(REG_CLK_CONF, 0xFFFF_FFFF).unwrap();
        assert_eq!(p.read_u32(REG_CLK_CONF).unwrap(), 0x003F_FFFF);

        // SCL_ST_TIME_OUT: mask 0x1F.
        p.write_u32(REG_SCL_ST_TIME_OUT, 0xFFFF_FFFF).unwrap();
        assert_eq!(p.read_u32(REG_SCL_ST_TIME_OUT).unwrap(), 0x0000_001F);

        // DATE: fully writable.
        p.write_u32(REG_DATE, 0xDEAD_BEEF).unwrap();
        assert_eq!(p.read_u32(REG_DATE).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn ctr_reset_is_silicon_default() {
        // Fresh model must read CTR == 0x20B before any write.
        let p = Esp32s3I2c::new();
        assert_eq!(
            p.read_u32(REG_CTR).unwrap(),
            0x0000_020B,
            "CTR reset value must match silicon default 0x20B"
        );
    }

    #[test]
    fn fifo_conf_reset_is_silicon_default() {
        // Fresh model must read FIFO_CONF == 0x408B before any write.
        let p = Esp32s3I2c::new();
        assert_eq!(
            p.read_u32(REG_FIFO_CONF).unwrap(),
            0x0000_408B,
            "FIFO_CONF reset value must match silicon default 0x408B"
        );
    }

    #[test]
    fn sr_reset_has_stretch_cause() {
        // A fresh model's SR must have STRETCH_CAUSE bits[15:14] == 0b11.
        let p = Esp32s3I2c::new();
        let sr = p.read_u32(REG_SR).unwrap();
        assert_eq!(
            sr & 0x0000_C000,
            0x0000_C000,
            "SR STRETCH_CAUSE[15:14] must be 0b11 at reset, SR=0x{sr:08x}"
        );
    }

    #[test]
    fn int_raw_reset_has_txfifo_wm_set() {
        // TXFIFO_WM_INT_RAW (bit 1) is set at reset: TX FIFO starts empty
        // which is at or below the watermark threshold.
        let p = Esp32s3I2c::new();
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & 0x2,
            0x2,
            "INT_RAW bit 1 (TXFIFO_WM) must be set at reset"
        );
    }

    #[test]
    fn unmapped_offset_between_regs_reads_zero() {
        // Offset 0xC0 is between SCL_STRETCH_CONF (0x84) and DATE (0xF8) —
        // not a real register; catch-all must return 0 and ignore writes.
        let mut p = Esp32s3I2c::new();
        p.write_u32(0xC0, 0xDEAD_BEEF).unwrap();
        assert_eq!(p.read_u32(0xC0).unwrap(), 0, "unmapped 0xC0 must read 0");
    }
}
