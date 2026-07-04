// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 I²C0 controller — command-list engine.
//!
//! Mapped at base 0x6001_3000 with size 4 KiB. See ESP32-C3 TRM §16.
//!
//! The ESP32-C3 I²C is the SAME Espressif I²C IP family as the ESP32-S3, so the
//! register layout, COMD command-list semantics, FIFO behaviour and reset
//! values are identical to [`crate::peripherals::esp32s3::i2c::Esp32s3I2c`].
//! This model is a C3-correct port of that controller. The register map was
//! diffed against `configs/peripherals/esp32c3/i2c0.yaml` (the authoritative C3
//! layout, SVD-sourced) — every offset, field and reset value matches the S3.
//!
//! ## C3-vs-S3 differences
//!
//! The ONE substantive difference is the interrupt-matrix source number:
//!   * ESP32-S3 I2C_EXT0 = source **42** (Xtensa `ets_isr_source_t` ordinal).
//!   * ESP32-C3 I2C_EXT0 = source **29** — the RISC-V interrupt-matrix source
//!     index. Corroborated by the C3 `interrupt_core0.yaml`:
//!     `I2C_EXT0_INTR_MAP` lives at register offset 116 = `4 * 29`, and the C3
//!     `i2c0.yaml` declares `interrupts: { I2C_EXT0: 29 }`.
//!
//! ## Register subset modeled (offsets identical to S3 / C3 `i2c0.yaml`)
//!
//! | Offset | Name        | Notes                                          |
//! |--------|-------------|------------------------------------------------|
//! | 0x04   | CTR         | TRANS_START at bit 5                           |
//! | 0x08   | SR          | Status — bit 0 = RESP_REC (slave acked)        |
//! | 0x10   | SLAVE_ADDR  | 7-bit address in [6:0]                         |
//! | 0x14   | FIFO_ST     | TX/RX FIFO levels                              |
//! | 0x18   | FIFO_CONF   | RX/TX FIFO reset bits self-clear               |
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

use std::cell::RefCell;

use crate::peripherals::i2c::I2cDevice;
use crate::{Peripheral, PeripheralTickResult, SimResult};

pub const I2C0_BASE: u32 = 0x6001_3000;
pub const I2C0_SIZE: u64 = 0x1000;

/// ESP32-C3 I2C0 (I2C_EXT0) peripheral interrupt-matrix source number.
///
/// On the C3 (RISC-V) the firmware programs `I2C_EXT0_INTR_MAP` in the
/// interrupt matrix at offset `4 * source`; the C3 `interrupt_core0.yaml`
/// places that register at offset 116 = `4 * 29`, so the source index is 29 —
/// NOT the S3's 42 (which is the Xtensa `ets_isr_source_t` ordinal). The C3
/// `i2c0.yaml` likewise declares `interrupts: { I2C_EXT0: 29 }`.
pub const I2C0_INTR_SOURCE_ID: u32 = 29;

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
// Read-only APB windows into the FIFO RAM (TXFIFO_START_ADDR / RXFIFO_START_ADDR
// per C3 i2c0.yaml offsets 256 / 384). Reading shows the FIFO head byte without
// consuming it.
const REG_TXFIFO_START: u64 = 0x100;
const REG_RXFIFO_START: u64 = 0x180;

// Config / timing registers (offsets + reset values from the C3 i2c0.yaml,
// identical to the S3 layout).
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
/// CTR bit 11: CONF_UPGATE — self-clearing config-sync trigger.
const CTR_CONF_UPGATE: u32 = 1 << 11;

/// SR bit 0: set when the slave responded with ACK during the most recent
/// command. esp-hal checks this after TRANS_COMPLETE — if clear it raises
/// `AcknowledgeCheckFailed(Data)`.
const SR_RESP_REC: u32 = 1 << 0;

/// COMD bit 31: command_done. Set when a command finishes executing.
const CMD_DONE_BIT: u32 = 1 << 31;

pub const INT_END_DETECT: u32 = 1 << 3;
pub const INT_TRANS_COMPLETE: u32 = 1 << 7;
pub const INT_NACK: u32 = 1 << 10;

/// ESP32-C3 has 8 COMD slots at offsets 0x58..0x78 (COMD0..COMD7 in the yaml).
const NUM_CMDS: usize = 8;
const FIFO_CAPACITY: usize = 32;

pub struct Esp32c3I2c {
    ctr: u32,
    sr: u32,
    slave_addr: u32,
    int_raw: u32,
    int_ena: u32,
    fifo_conf: u32,
    cmds: [u32; NUM_CMDS],
    tx_fifo: std::collections::VecDeque<u8>,
    /// TX-FIFO read pointer (bytes consumed by the current command-list run).
    /// Surfaced as FIFO_ST.TXFIFO_RADDR; 0 at cold reset.
    tx_pop_count: usize,
    rx_fifo: RefCell<std::collections::VecDeque<u8>>,
    slaves: Vec<Box<dyn I2cDevice>>,
    /// Set when a command-list run sets TRANS_COMPLETE & INT_ENA has it.
    irq_pending: bool,
    /// Interrupt-matrix source this instance asserts (29 for I2C0).
    intr_source_id: u32,

    // Config / timing registers — masked storage (reset values per C3 i2c0.yaml).
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

impl Esp32c3I2c {
    pub fn new() -> Self {
        Self {
            // CTR reset 0x020B (== C3 i2c0.yaml reset_value 523):
            // SCL_FORCE_OUT|SDA_FORCE_OUT|SAMPLE_SCL_LEVEL|RX_FULL_ACK_LEVEL.
            ctr: 0x0000_020B,
            sr: 0,
            slave_addr: 0,
            // INT_RAW bit 1 (TXFIFO_WM_INT_RAW) set at reset (== yaml reset 2):
            // the empty TX FIFO is at/below the watermark threshold.
            int_raw: 0x0000_0002,
            int_ena: 0,
            // FIFO_CONF reset 0x408B (== yaml reset_value 16523):
            // RXFIFO_WM_THRHD=0xB, TXFIFO_WM_THRHD=0x4.
            fifo_conf: 0x0000_408B,
            cmds: [0; NUM_CMDS],
            tx_fifo: std::collections::VecDeque::with_capacity(FIFO_CAPACITY),
            tx_pop_count: 0,
            rx_fifo: RefCell::new(std::collections::VecDeque::with_capacity(FIFO_CAPACITY)),
            slaves: Vec::new(),
            irq_pending: false,
            intr_source_id: I2C0_INTR_SOURCE_ID,

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

    /// Construct an instance asserting a different interrupt-matrix source.
    pub fn with_intr_source(intr_source_id: u32) -> Self {
        Self {
            intr_source_id,
            ..Self::new()
        }
    }

    /// Attach an `I2cDevice` slave. Slaves are matched by address bits at
    /// transaction time; later additions take precedence on duplicate addresses.
    pub fn attach_slave(&mut self, slave: Box<dyn I2cDevice>) {
        self.slaves.push(slave);
    }

    /// Borrow the attached I²C slaves. Mirrors the generic `I2c::attached_devices`
    /// accessor so UI/inspection paths (e.g. the SSD1306 framebuffer readback)
    /// can enumerate devices on the ESP32-C3 command-list controller the same way
    /// they do on the STM32 controller. Unlike the generic `I2c`, slaves here are
    /// held directly (no `RefCell`) because the C3 engine never hands out interior
    /// mutable references during a transaction.
    pub fn attached_slaves(&self) -> &[Box<dyn I2cDevice>] {
        &self.slaves
    }

    fn fifo_status(&self) -> u32 {
        // FIFO_ST (C3 i2c0.yaml): TXFIFO_RADDR at bits 10..14 — esp-hal's
        // estimate_ack_failed_reason reads it to tell address-NACK (raddr <= 1)
        // from data-NACK. raddr is the TX-FIFO *read pointer*: the number of
        // bytes the command-list engine has consumed in the current run. It is
        // 0 at cold reset (silicon FIFO_ST reset value = 0), so this must NOT be
        // derived from `FIFO_CAPACITY - len` (which would be non-zero when the
        // FIFO has simply never been pushed).
        let tx_raddr = (self.tx_pop_count as u32) & 0x1F;
        tx_raddr << 10
    }

    fn status_register(&self) -> u32 {
        // SR (C3 i2c0.yaml): bit 0 RESP_REC, bits 8..13 RXFIFO_CNT,
        // bits 14..15 STRETCH_CAUSE (reset 0b11 == yaml reset_value 49152),
        // bits 18..23 TXFIFO_CNT.
        const SR_STRETCH_CAUSE_RESET: u32 = 0x0000_C000;
        let rx = (self.rx_fifo.borrow().len() as u32) & 0x3F;
        let tx = (self.tx_fifo.len() as u32) & 0x3F;
        (self.sr & SR_RESP_REC) | SR_STRETCH_CAUSE_RESET | (rx << 8) | (tx << 18)
    }
}

impl Default for Esp32c3I2c {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Esp32c3I2c {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Esp32c3I2c")
            .field("ctr", &self.ctr)
            .field("slave_addr", &self.slave_addr)
            .field("int_raw", &self.int_raw)
            .field("int_ena", &self.int_ena)
            .field("slaves_count", &self.slaves.len())
            .field("irq_pending", &self.irq_pending)
            .finish()
    }
}

impl Peripheral for Esp32c3I2c {
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
            eprintln!("C3 I2C R [0x{offset:02x}] = 0x{v:08x}");
        }
        Ok(v)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Byte writes ignored — the esp-hal driver writes whole words.
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if std::env::var("LABWIRED_I2C_TRACE").is_ok() {
            eprintln!("C3 I2C W [0x{offset:02x}] = 0x{value:08x}");
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
                // CONF_UPGATE (bit 11) is a self-clearing sync trigger.
                self.ctr &= !CTR_CONF_UPGATE;
            }
            REG_TO => masked_write(&mut self.reg_to, value, 0x0000_003F),
            REG_SLAVE_ADDR => self.slave_addr = value,
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
                    self.tx_pop_count = 0;
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
            REG_SCL_SP_CONF => masked_write(&mut self.reg_scl_sp_conf, value, 0x0000_00FF),
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
        // INT bit is set, mirroring real silicon (INT_RAW stays asserted until
        // the ISR writes INT_CLR).
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

    /// Custom inspection: generic register decode plus a `framebuffer` artifact
    /// for any attached SSD1306 OLED. Same pattern as the generic `I2c`
    /// controller — the C3 command-list controller walks its own slaves so the
    /// leo air-quality OLED surfaces through the universal inspect interface.
    fn inspect(
        &self,
        base: u64,
        name: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::PeripheralInspect {
        let mut pi = crate::inspect::default_inspect(self, base, name, opts);
        pi.kind = "i2c".to_string();
        for dev in self.attached_slaves() {
            let addr = dev.address();
            if let Some(oled) = dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::components::Ssd1306>())
            {
                let fb = oled.framebuffer();
                pi.artifacts.push(crate::inspect::Artifact {
                    kind: "framebuffer".to_string(),
                    id: format!("i2c@0x{:02x}", addr),
                    meta: serde_json::json!({
                        "w": oled.width(),
                        "h": oled.height(),
                        "format": "ssd1306_page",
                        "generation": crate::inspect::artifact_generation(fb),
                    }),
                    bytes: if opts.include_bytes {
                        Some(fb.to_vec())
                    } else {
                        None
                    },
                });
            }
        }
        pi
    }
}

impl Esp32c3I2c {
    /// Walk CMD0..CMD7 from the start, executing each command. A "WRITE"
    /// whose first byte follows an RSTART is interpreted as `(addr<<1)|R/W`
    /// and selects the active slave by address bits [7:1]. Subsequent WRITE
    /// bytes are delivered via `I2cDevice::write`. READ pulls bytes from the
    /// active slave via `I2cDevice::read` and pushes to the RX FIFO.
    fn run_command_list(&mut self) {
        // Opcodes per ESP32-C3 TRM §16 / esp32c3 PAC `i2c0::comd`:
        //   1 = WRITE, 2 = STOP, 3 = READ, 4 = END, 6 = RSTART
        const OP_WRITE: u32 = 1;
        const OP_STOP: u32 = 2;
        const OP_READ: u32 = 3;
        const OP_END: u32 = 4;
        const OP_RSTART: u32 = 6;

        let mut active: Option<usize> = None;
        let mut expects_addr = true;
        let mut last_op_was_end = false;

        // Reset RESP_REC and the TX-FIFO read pointer at the start of a new
        // command-list run.
        self.sr &= !SR_RESP_REC;
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
                    for i in 0..byte_num {
                        let b = self.tx_fifo.pop_front().unwrap_or(0);
                        self.tx_pop_count += 1;
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
                    break;
                }
                OP_END => {
                    last_op_was_end = true;
                    break;
                }
                _ => break, // reserved opcode — terminate
            }
        }

        // END pauses execution and raises END_DETECT (bit 3); STOP (or a command
        // list that runs out without an explicit END) completes the transaction
        // and raises TRANS_COMPLETE (bit 7).
        if last_op_was_end {
            self.int_raw |= INT_END_DETECT;
        } else {
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

    // ESP32-C3 TRM §16: 1=WRITE, 2=STOP, 3=READ, 4=END, 6=RSTART.
    const CMD_WRITE: u8 = 1;
    const CMD_STOP: u8 = 2;
    const CMD_READ: u8 = 3;
    const CMD_END: u8 = 4;
    const CMD_RSTART: u8 = 6;

    #[test]
    fn i2c0_interrupt_source_is_29_not_42() {
        // C3-vs-S3 difference: the C3 routes I2C_EXT0 through interrupt-matrix
        // source 29 (I2C_EXT0_INTR_MAP at offset 116 = 4*29), NOT the S3's 42.
        assert_eq!(I2C0_INTR_SOURCE_ID, 29);
    }

    #[test]
    fn ctr_round_trip() {
        let mut p = Esp32c3I2c::new();
        p.write_u32(REG_CTR, 0x0000_0010).unwrap(); // arbitrary, no TRANS_START
        assert_eq!(p.read_u32(REG_CTR).unwrap(), 0x0000_0010);
    }

    #[test]
    fn slave_addr_round_trip() {
        let mut p = Esp32c3I2c::new();
        p.write_u32(REG_SLAVE_ADDR, 0x48).unwrap();
        assert_eq!(p.read_u32(REG_SLAVE_ADDR).unwrap(), 0x48);
    }

    #[test]
    fn cmd_registers_round_trip() {
        let mut p = Esp32c3I2c::new();
        p.write_u32(REG_CMD0, 0x0000_0800).unwrap();
        p.write_u32(REG_CMD7, 0x0000_2000).unwrap();
        assert_eq!(p.read_u32(REG_CMD0).unwrap(), 0x0000_0800);
        assert_eq!(p.read_u32(REG_CMD7).unwrap(), 0x0000_2000);
    }

    #[test]
    fn sr_txfifo_cnt_reflects_pushes() {
        let mut p = Esp32c3I2c::new();
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
        let mut p = Esp32c3I2c::new();
        p.write_u32(REG_DATA, 0x11).unwrap();
        p.write_u32(REG_DATA, 0x22).unwrap();
        p.write_u32(REG_FIFO_CONF, 1 << 13).unwrap(); // TX_FIFO_RST
        let sr = p.read_u32(REG_SR).unwrap();
        assert_eq!((sr >> 18) & 0x3F, 0);
    }

    #[test]
    fn int_clr_clears_specified_bits() {
        let mut p = Esp32c3I2c::new();
        p.int_raw = INT_TRANS_COMPLETE | INT_NACK;
        p.write_u32(REG_INT_CLR, INT_NACK).unwrap();
        assert_eq!(p.read_u32(REG_INT_RAW).unwrap(), INT_TRANS_COMPLETE);
    }

    #[test]
    fn int_st_masks_with_int_ena() {
        let mut p = Esp32c3I2c::new();
        p.int_raw = INT_TRANS_COMPLETE | INT_NACK;
        p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap();
        assert_eq!(p.read_u32(REG_INT_ST).unwrap(), INT_TRANS_COMPLETE);
    }

    #[test]
    fn end_opcode_raises_end_detect_not_trans_complete() {
        let mut p = Esp32c3I2c::new();
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
        let mut p = Esp32c3I2c::new();
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
        let mut p = Esp32c3I2c::new();
        p.write_u32(REG_CMD0, cmd(CMD_END, 0)).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        assert_eq!(p.read_u32(REG_CTR).unwrap() & CTR_TRANS_START_BIT, 0);
    }

    #[test]
    fn txfifo_start_addr_window_peeks_tx_fifo_non_destructively() {
        let mut p = Esp32c3I2c::new();
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
    }

    #[test]
    fn write_with_unmatched_address_sets_nack_int() {
        let mut p = Esp32c3I2c::new();
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
    fn config_registers_reset_values_match_c3_yaml() {
        let p = Esp32c3I2c::new();
        assert_eq!(
            p.read_u32(REG_CTR).unwrap(),
            0x0000_020B,
            "CTR reset (yaml 523)"
        );
        assert_eq!(
            p.read_u32(REG_FIFO_CONF).unwrap(),
            0x0000_408B,
            "FIFO_CONF (yaml 16523)"
        );
        assert_eq!(p.read_u32(REG_TO).unwrap(), 0x0000_0010, "TO (yaml 16)");
        assert_eq!(
            p.read_u32(REG_SCL_START_HOLD).unwrap(),
            0x0000_0008,
            "SCL_START_HOLD (yaml 8)"
        );
        assert_eq!(
            p.read_u32(REG_FILTER_CFG).unwrap(),
            0x0000_0300,
            "FILTER_CFG (yaml 768)"
        );
        assert_eq!(
            p.read_u32(REG_CLK_CONF).unwrap(),
            0x0020_0000,
            "CLK_CONF (yaml 2097152)"
        );
        assert_eq!(
            p.read_u32(REG_DATE).unwrap(),
            0x2007_0201,
            "DATE (yaml 537330177)"
        );
        let sr = p.read_u32(REG_SR).unwrap();
        assert_eq!(
            sr & 0x0000_C000,
            0x0000_C000,
            "SR STRETCH_CAUSE (yaml 49152)"
        );
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & 0x2,
            0x2,
            "INT_RAW TXFIFO_WM (yaml 2)"
        );
    }

    // ── Headline test: an attached I2cDevice round-trips a write-then-read
    //    transaction driven exactly as C3 firmware would. Uses the Bmp280
    //    register-pointer device (an existing I2cDevice).

    use crate::peripherals::components::Bmp280;

    #[test]
    fn write_read_drives_attached_bmp280() {
        let mut p = Esp32c3I2c::new();
        // Default address 0x76.
        p.attach_slave(Box::new(Bmp280::new(0x76)));

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
        // Slave acked → RESP_REC set in SR.
        assert_eq!(
            p.read_u32(REG_SR).unwrap() & SR_RESP_REC,
            SR_RESP_REC,
            "SR.RESP_REC must be set after a successful transaction"
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
    fn write_then_read_calibration_block_round_trip() {
        // Read the 24-byte calibration block starting at 0x88 — exercises a
        // multi-byte READ pulling sequential register-pointer data through the
        // RX FIFO.
        let mut p = Esp32c3I2c::new();
        p.attach_slave(Box::new(Bmp280::new(0x76)));

        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 12, cmd(CMD_WRITE, 1)).unwrap();
        p.write_u32(REG_CMD0 + 16, cmd(CMD_READ, 4)).unwrap();
        p.write_u32(REG_CMD0 + 20, cmd(CMD_STOP, 0)).unwrap();

        p.write_u32(REG_DATA, 0xEC).unwrap(); // addr+W
        p.write_u32(REG_DATA, 0x88).unwrap(); // pointer = calib start
        p.write_u32(REG_DATA, 0xED).unwrap(); // addr+R
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();

        // First four calibration bytes per the Bosch reference block.
        assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x70);
        assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x6B);
        assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x43);
        assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x67);
    }
}
