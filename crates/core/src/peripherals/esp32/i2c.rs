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
use crate::peripherals::i2c_waveform::I2cNarrator;
use crate::peripherals::pad_lines::PadLines;
use crate::{Peripheral, PeripheralTickResult, SimResult};

/// Read once per process — this sits on the I2C command path. See
/// `fidelity::strict` for why a per-call `env::var` is a real cost.
fn i2c_trace_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LABWIRED_I2C_TRACE").is_ok())
}

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

/// Line order for this controller's [`PadLines`]; the classic GPIO matrix routes
/// `I2CEXT0_SCL` (signal 29) / `I2CEXT0_SDA` (signal 30) to these indices.
pub(crate) const I2C_LINES: &[&str] = &["SCL", "SDA"];
pub(crate) const LINE_SCL: usize = 0;
pub(crate) const LINE_SDA: usize = 1;

/// Timing registers the narration reads to shape the waveform. Both are 14-bit
/// APB-cycle counts: `I2C_SCL_LOW_PERIOD_REG` at 0x00 and
/// `I2C_SCL_HIGH_PERIOD_REG` at 0x38, fields [13:0] (esp-idf
/// `soc/esp32/include/soc/i2c_reg.h`). They already round-trip through the
/// generic `other` store — the command-list engine has simply never read them,
/// which is why "classic ESP32 has no SCL period registers" was wrong.
const REG_SCL_LOW_PERIOD: u64 = 0x00;
const REG_SCL_HIGH_PERIOD: u64 = 0x38;
const SCL_PERIOD_MASK: u32 = 0x3FFF;

/// CPU cycles per APB cycle. The I²C timing registers count APB periods
/// (TRM v4.6 §11) while the engine's cycle axis is CPU cycles; the classic
/// ESP32's default CPU_CLK/APB_CLK split is 240 MHz / 80 MHz = 3, the same
/// ratio and the same reason as the S3 model.
const CORE_PER_APB: u64 = 3;

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
    /// Mid-transfer continuation across command-list bursts. The classic-ESP32
    /// legacy IDF driver splits one logical transfer into several TRANS_START
    /// bursts joined by the END opcode, which SUSPENDS the command sequence
    /// (TRM §11) rather than terminating it: the selected slave and the
    /// address-phase flag carry into the next burst so a follow-on WRITE
    /// delivers data (not a fresh address) and a READ pulls from the same
    /// slave. STOP or natural completion clears them back to the reset shape.
    active_slave: Option<usize>,
    expects_addr: bool,
    /// Interrupt-matrix source this instance asserts (49 for I2C0).
    intr_source_id: u32,
    /// Round-trip backing for timing / config registers the engine ignores.
    other: BTreeMap<u64, u32>,
    /// Wire levels published to matrix-routed SCL/SDA pads, so an analyzer
    /// clipped to this bus measures a real waveform instead of a flat line.
    /// Created lazily at bus wiring time; `None` on any bus where nothing
    /// routes these pads, and then every wire call below costs one check.
    lines: Option<std::sync::Arc<PadLines>>,
    /// Frames of the command list currently executing, `(byte, acked)`,
    /// narrated onto the pads as ONE transaction when the list finishes.
    wire_frames: Vec<(u8, bool)>,
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
            active_slave: None,
            expects_addr: true,
            intr_source_id: I2C0_INTR_SOURCE_ID,
            lines: None,
            wire_frames: Vec::new(),
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

    /// The shared pad-line cell for this controller, created on first use.
    /// Called at bus wiring time; an open-drain bus with pull-ups idles high.
    pub(crate) fn pad_lines_arc(&mut self) -> std::sync::Arc<PadLines> {
        self.lines
            .get_or_insert_with(|| std::sync::Arc::new(PadLines::new(I2C_LINES, &[true, true])))
            .clone()
    }

    /// Engine cycles in one SCL period, from this controller's OWN timing
    /// registers: `SCL_LOW_PERIOD + SCL_HIGH_PERIOD` APB cycles (TRM v4.6 §11)
    /// times [`CORE_PER_APB`].
    ///
    /// Read back through `decode_word` rather than mirrored into new fields, so
    /// there is exactly ONE place a value for offset 0x00 / 0x38 comes from and
    /// a debugger cannot disagree with the narrator. At the register reset —
    /// both zero, firmware has not programmed timing yet — this falls to a
    /// floor so a waveform is still shaped rather than degenerate.
    fn bit_time_cycles(&self) -> u64 {
        let low = self.decode_word(REG_SCL_LOW_PERIOD) & SCL_PERIOD_MASK;
        let high = self.decode_word(REG_SCL_HIGH_PERIOD) & SCL_PERIOD_MASK;
        (u64::from(low + high) * CORE_PER_APB).max(16)
    }

    /// Record a frame this transaction put on the wire. Buffered, not published
    /// — see [`Self::wire_flush`]. Free when no pad routes this controller.
    fn wire_push(&mut self, byte: u8, acked: bool) {
        if self.lines.is_some() {
            self.wire_frames.push((byte, acked));
        }
    }

    /// Undo the last recorded frame.
    ///
    /// The executor can only tell which shape a WRITE has once it has looked at
    /// the first FIFO byte: in the ESP-IDF/Arduino shape that byte is payload
    /// and the address lives in `SLAVE_ADDR`, so a frame provisionally recorded
    /// as the address has to be taken back and re-recorded as data.
    fn wire_pop_last_addr(&mut self) {
        self.wire_frames.pop();
    }

    /// The address byte as it appears on the wire when the address comes from
    /// the `SLAVE_ADDR` register rather than the TX FIFO: 7-bit address in bits
    /// [6:0], shifted up with a write direction bit.
    fn slave_addr_byte(&self) -> u8 {
        ((self.slave_addr & 0x7F) as u8) << 1
    }

    /// Publish the finished command list's waveform onto the routed pads.
    ///
    /// This controller executes its whole command list synchronously on the
    /// `TRANS_START` write and charges no wire time at all, so the narration is
    /// anchored to END at the present cycle: it occupies the cycles just before
    /// the write, which the bus genuinely spent idle. Every stamp is therefore
    /// in the past, where the capture layer keeps it verbatim — which is also
    /// exactly why the classic GPIO port must accept a PUSH tap, since a poll
    /// sampler cannot observe a past cycle. See
    /// [`crate::peripherals::i2c_waveform`] for what a narrated waveform does
    /// and does not model.
    fn wire_flush(&mut self) {
        let Some(lines) = self.lines.clone() else {
            self.wire_frames.clear();
            return;
        };
        if self.wire_frames.is_empty() {
            return;
        }
        let mut wave = I2cNarrator::new(LINE_SCL, LINE_SDA, self.bit_time_cycles());
        wave.start();
        for &(byte, acked) in &self.wire_frames {
            wave.frame(byte, acked);
        }
        wave.stop();
        self.wire_frames.clear();
        let now = lines.tap_clock().unwrap_or(0);
        // A transaction fired before the run has accumulated its own duration is
        // compressed to fit, not spiked: the bytes survive, the measured rate
        // does not. See `NarrationFit`.
        let _fit = wave.emit_ending_at(&lines, now);
    }

    /// Raw slave push — does NOT wrap for tracing. The only production caller is
    /// the bus choke point [`crate::bus::SystemBus::attach_i2c_slave`], which
    /// wraps first. Slaves are matched by 7-bit address at transaction time;
    /// later additions take precedence on duplicate addresses.
    pub(crate) fn push_slave(&mut self, slave: Box<dyn I2cDevice>) {
        self.slaves.push(slave);
    }

    /// Borrow attached slaves (browser sensor readback / inspect).
    pub fn attached_slaves(&self) -> &[Box<dyn I2cDevice>] {
        &self.slaves
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
    fn find_slave_from_slave_addr_register(&mut self) -> Option<usize> {
        let raw = self.slave_addr & 0x7FFF;
        if raw <= 0x7F {
            if let Some(idx) = self.find_slave_by_address(raw as u8) {
                return Some(idx);
            }
        }
        let shifted = ((raw >> 1) & 0x7F) as u8;
        self.find_slave_by_address(shifted)
    }

    /// Resolve the slave that answers to `address` and tell it which address
    /// the master selected.
    ///
    /// Resolution goes through `claims_address`, not `address()`: a bus switch
    /// (TCA9548A) answers for every device behind its enabled channels, and a
    /// flat `address()` comparison is first-match — four identical sensors on
    /// four channels would collapse onto one.
    fn find_slave_by_address(&mut self, address: u8) -> Option<usize> {
        let idx = self.slaves.iter().position(|s| s.claims_address(address))?;
        self.slaves[idx].select_address(address);
        Some(idx)
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

impl Esp32I2c {
    /// Decode one 32-bit register WITHOUT touching model state.
    ///
    /// One decode, two callers: `read_u32` (which then applies REG_DATA's FIFO
    /// pop) and `peek` (which does not). Keeping it in one place is why a
    /// debugger and the firmware cannot disagree about a register's contents.
    fn decode_word(&self, offset: u64) -> u32 {
        match offset {
            REG_CTR => self.ctr,
            REG_SR => self.status_register(),
            REG_SLAVE_ADDR => self.slave_addr,
            // The value a read RETURNS; the pop it also causes belongs to
            // `read_u32`.
            REG_DATA => self.rx_fifo.borrow().front().copied().unwrap_or(0) as u32,
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
        }
    }
}

impl Peripheral for Esp32I2c {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // Byte reads aren't used by the I2C driver; route via read_u32.
        Ok(0)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let v = self.decode_word(offset);
        if offset == REG_DATA {
            // The ONLY side effect in this register file: a REG_DATA read pops
            // the RX FIFO, on silicon and here. `decode_word` reported the byte;
            // consuming it is the read's job, not the decode's.
            self.rx_fifo.borrow_mut().pop_front();
        }
        if i2c_trace_enabled() {
            eprintln!("ESP32 I2C R [0x{offset:02x}] = 0x{v:08x}");
        }
        Ok(v)
    }

    /// Side-effect-free probe, so `inspect` can show this controller's real
    /// registers instead of reporting them unreadable.
    ///
    /// Shares `decode_word` with `read_u32` rather than repeating the match, so
    /// a debugger view and a firmware read can never disagree about what a
    /// register contains. The difference is only what happens afterwards: a
    /// read of REG_DATA pops the RX FIFO, and a peek must not — otherwise
    /// looking at the panel in a debugger would eat the byte the firmware was
    /// about to receive, which is exactly the failure `inspect`'s peek-only
    /// contract exists to prevent.
    fn peek(&self, offset: u64) -> Option<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        Some(((self.decode_word(word_off) >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Byte writes ignored — the driver writes whole words (except the FIFO
        // data register, which is also driven word-wide via write_u32).
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if i2c_trace_enabled() {
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
            // `for_each_sim_input`, not `as_sim_input_mut`: a container slave
            // (TCA9548A mux) exposes the inputs of the devices behind it, which
            // a single-surface accessor cannot represent.
            if slave.for_each_sim_input(f) {
                return true;
            }
        }
        false
    }

    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for dev in &self.slaves {
            crate::inspect::visit_i2c_device(&**dev, f);
        }
    }
}

impl Esp32I2c {
    /// Walk COMD0..COMD15 from the start, executing each command. A WRITE whose
    /// first byte follows an RSTART is interpreted as `(addr<<1)|R/W` and selects
    /// the active slave by address bits [7:1]. Subsequent WRITE bytes are
    /// delivered via `I2cDevice::write`; READ pulls bytes from the active slave
    /// and pushes to the RX FIFO.
    ///
    /// The selected slave and address-phase state seed from `self.active_slave`
    /// / `self.expects_addr`, which a prior END-terminated burst left behind, so
    /// a transfer split across multiple TRANS_START bursts (the legacy IDF
    /// driver's shape) resumes rather than re-decoding a data byte as an
    /// address. RSTART begins a fresh address phase; STOP/completion clears it.
    fn run_command_list(&mut self) {
        // Classic-ESP32 opcodes (hal/esp32/include/hal/i2c_ll.h):
        //   0 = RSTART, 1 = WRITE, 2 = READ, 3 = STOP, 4 = END
        const OP_RSTART: u32 = 0;
        const OP_WRITE: u32 = 1;
        const OP_READ: u32 = 2;
        const OP_STOP: u32 = 3;
        const OP_END: u32 = 4;

        // END pauses the command list (TRM §11): the selected slave and
        // address-phase flag carry over from the previous burst so a follow-on
        // WRITE/READ resumes the in-flight transfer instead of re-addressing.
        let mut active = self.active_slave;
        let mut expects_addr = self.expects_addr;
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
                        // The address frame crossed the wire either way; whether
                        // it was ACKed is what the analyzer shows.
                        self.wire_push(self.slave_addr_byte(), active.is_some());
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
                            active = self.find_slave_by_address(addr);
                            self.wire_push(b, active.is_some());
                            if active.is_none() {
                                // Fallback: address only in SLAVE_ADDR, payload in FIFO.
                                active = self.find_slave_from_slave_addr_register();
                                // The address came from SLAVE_ADDR, so the wire
                                // carried THAT frame, not the FIFO byte we had
                                // provisionally recorded as an address.
                                self.wire_pop_last_addr();
                                self.wire_push(self.slave_addr_byte(), active.is_some());
                                if let Some(slave_idx) = active {
                                    self.slaves[slave_idx].start();
                                    self.sr |= SR_ACK_REC;
                                    // First FIFO byte is data when SLAVE_ADDR holds target.
                                    self.slaves[slave_idx].write(b);
                                    self.wire_push(b, true);
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
                            self.wire_push(b, true);
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
                        if active.is_some() {
                            // The master ACKs each byte it reads; the final NACK
                            // is modelled by the STOP that follows.
                            self.wire_push(b, true);
                        }
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

        // END pauses execution and raises END_DETECT; the selected slave and
        // address-phase flag persist so the next TRANS_START burst resumes the
        // suspended transfer. STOP (or a list that runs out without an explicit
        // END) completes and raises TRANS_COMPLETE, clearing the continuation
        // back to the reset shape.
        if last_op_was_end {
            self.active_slave = active;
            self.expects_addr = expects_addr;
            self.int_raw |= INT_END_DETECT;
        } else {
            // STOP (or a list that ran out) completes the transaction: NOW the
            // whole thing goes on the wire as one narrated waveform. END keeps
            // the frames buffered on purpose — the legacy IDF driver splits one
            // logical transfer across several bursts, and narrating each burst
            // separately would put three STARTs on the trace for one transfer.
            self.wire_flush();
            self.active_slave = None;
            self.expects_addr = true;
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

    /// Inspecting a bus in a debugger must not eat the byte the firmware was
    /// about to read. On this controller a REG_DATA read POPS the RX FIFO, so
    /// this is the one register where peek and read must differ -- and the
    /// difference must be that peek leaves the queue alone.
    #[test]
    fn peek_does_not_drain_the_rx_fifo() {
        let i2c = Esp32I2c::new();
        i2c.rx_fifo.borrow_mut().extend([0x12u8, 0x34]);

        assert_eq!(i2c.peek(REG_DATA), Some(0x12), "peek reports the head byte");
        assert_eq!(
            i2c.peek(REG_DATA),
            Some(0x12),
            "and again -- nothing consumed"
        );
        assert_eq!(
            i2c.rx_fifo.borrow().len(),
            2,
            "peeking left the FIFO untouched"
        );

        assert_eq!(
            i2c.read_u32(REG_DATA).unwrap(),
            0x12,
            "the read still gets it"
        );
        assert_eq!(
            i2c.rx_fifo.borrow().len(),
            1,
            "and the read is what consumed it"
        );
        assert_eq!(
            i2c.peek(REG_DATA),
            Some(0x34),
            "peek now sees the next byte"
        );
    }

    /// Everywhere else a debugger and the firmware must agree byte for byte,
    /// or the register view is a second, separate model.
    #[test]
    fn peek_agrees_with_read_on_every_non_data_register() {
        let mut i2c = Esp32I2c::new();
        i2c.write_u32(REG_CTR, 0x0000_0113).unwrap();
        i2c.write_u32(REG_SLAVE_ADDR, 0x0000_0076).unwrap();
        i2c.write_u32(REG_CMD0, cmd(0, 1)).unwrap();
        for word_off in [
            REG_CTR,
            REG_SR,
            REG_SLAVE_ADDR,
            REG_FIFO_CONF,
            REG_INT_RAW,
            REG_INT_ENA,
            REG_INT_ST,
            REG_FIFO_ST,
            REG_CMD0,
        ] {
            let word = i2c.read_u32(word_off).unwrap();
            for lane in 0..4u64 {
                let expected = ((word >> (lane * 8)) & 0xFF) as u8;
                assert_eq!(
                    i2c.peek(word_off + lane),
                    Some(expected),
                    "peek disagrees with read at offset {:#04x}",
                    word_off + lane
                );
            }
        }
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

    /// Minimal I2cDevice that records the data bytes written to it, so a test
    /// can prove a payload byte reached the slave (rather than being swallowed
    /// as an address). `Arc<Mutex<..>>` keeps it `Send` per the trait bound.
    struct RecordingSlave {
        addr: u8,
        writes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    }

    impl I2cDevice for RecordingSlave {
        fn address(&self) -> u8 {
            self.addr
        }
        fn read(&mut self) -> u8 {
            0
        }
        fn write(&mut self, data: u8) {
            self.writes.lock().unwrap().push(data);
        }
    }

    // ── Legacy-IDF multi-burst shape: the classic-ESP32 arduino-esp32 2.x
    //    driver splits `beginTransmission(0x40); write(0x00); endTransmission()`
    //    into three TRANS_START bursts joined by END. Burst 2 carries only the
    //    data byte and NO RSTART, so the controller must resume the transfer
    //    addressed in burst 1 rather than decode 0x00 as a fresh address.
    #[test]
    fn legacy_multiburst_write_continues_across_end() {
        let mut p = Esp32I2c::new();
        let writes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        p.push_slave(Box::new(RecordingSlave {
            addr: 0x40,
            writes: std::sync::Arc::clone(&writes),
        }));

        // Burst 1: RSTART + WRITE(addr, 1) + END. Address 0x40<<1 = 0x80.
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 1)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_END, 0)).unwrap();
        p.write_u32(REG_DATA, 0x80).unwrap(); // addr+W for 0x40
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        let ir1 = p.read_u32(REG_INT_RAW).unwrap();
        assert_eq!(
            ir1 & INT_NACK,
            0,
            "address 0x40 must ACK — no NACK in burst 1"
        );
        assert_eq!(
            ir1 & INT_END_DETECT,
            INT_END_DETECT,
            "END must suspend the transfer and raise END_DETECT"
        );
        p.write_u32(REG_INT_CLR, 0xFFFF_FFFF).unwrap();

        // Burst 2: WRITE(data, 1) + END — NO RSTART. The 0xAB byte is payload
        // for the slave addressed in burst 1, not a new address phase.
        p.write_u32(REG_CMD0, cmd(CMD_WRITE, 1)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_END, 0)).unwrap();
        p.write_u32(REG_DATA, 0xAB).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        let ir2 = p.read_u32(REG_INT_RAW).unwrap();
        assert_eq!(
            ir2 & INT_NACK,
            0,
            "a resumed WRITE byte must NOT be mis-decoded as a fresh (unmatched) address"
        );
        assert_eq!(ir2 & INT_END_DETECT, INT_END_DETECT);
        p.write_u32(REG_INT_CLR, 0xFFFF_FFFF).unwrap();

        // Burst 3: STOP completes the transaction.
        p.write_u32(REG_CMD0, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
            INT_TRANS_COMPLETE,
            "STOP completes the transfer and raises TRANS_COMPLETE"
        );

        // The slave received exactly the data byte — not the address byte.
        assert_eq!(
            &*writes.lock().unwrap(),
            &[0xAB],
            "slave must receive the data byte delivered across the END boundary"
        );
    }

    #[test]
    fn legacy_multiburst_write_nacks_when_burst1_addr_unmatched() {
        let mut p = Esp32I2c::new();
        p.push_slave(Box::new(RecordingSlave {
            addr: 0x40,
            writes: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }));

        // Burst 1 addresses 0x50 (0xA0>>1) — no slave there → NACK, and no
        // continuation is armed for any follow-on burst.
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 1)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_END, 0)).unwrap();
        p.write_u32(REG_DATA, 0xA0).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_NACK,
            INT_NACK,
            "an unmatched address in burst 1 must still NACK"
        );
    }

    // ── TCA9548A driven through the classic-ESP32 command-list engine ───────
    //
    // The switch's own unit tests and `tests/i2c_mux_tca9548a.rs` prove it
    // against the STM32F1 legacy peripheral. This engine is a completely
    // different address-resolution site — including a `SLAVE_ADDR` fallback the
    // STM32 has no equivalent of — and it got the `claims_address` /
    // `select_address` change with no switch ever driven through it.
    mod mux {
        use super::*;
        use crate::peripherals::components::mux_fixture::{
            bytes_written_to, mux_with_tags, tag_for, MUX_ADDR, SENSOR_ADDR,
        };
        use crate::peripherals::components::tca9548a::Tca9548a;

        fn controller() -> Esp32I2c {
            let mut p = Esp32I2c::new();
            p.push_slave(Box::new(mux_with_tags(4)));
            p
        }

        fn with_mux<R>(p: &Esp32I2c, f: impl FnOnce(&Tca9548a) -> R) -> R {
            let mux = p.slaves[0]
                .as_any()
                .and_then(|a| a.downcast_ref::<Tca9548a>())
                .expect("slave 0 is the switch");
            f(mux)
        }

        /// Clear the previous run's latched interrupts so this run's NACK
        /// verdict is about this run.
        fn clear_ints(p: &mut Esp32I2c) {
            p.write_u32(REG_INT_CLR, 0xFFFF_FFFF).unwrap();
        }

        fn program(p: &mut Esp32I2c, list: &[(u8, u8)], tx: &[u8]) {
            clear_ints(p);
            // Flush both FIFOs without disturbing the watermark fields.
            let conf = p.read_u32(REG_FIFO_CONF).unwrap();
            p.write_u32(REG_FIFO_CONF, conf | (1 << 12) | (1 << 13))
                .unwrap();
            for (i, (op, n)) in list.iter().enumerate() {
                p.write_u32(REG_CMD0 + 4 * i as u64, cmd(*op, *n)).unwrap();
            }
            for b in tx {
                p.write_u32(REG_DATA, *b as u32).unwrap();
            }
            p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        }

        /// `RSTART; WRITE n(addr+W, payload…); STOP` — the address byte rides in
        /// the TX FIFO, which is the shape esp-hal emits.
        fn write_bytes(p: &mut Esp32I2c, addr: u8, payload: &[u8]) {
            let mut tx = vec![addr << 1];
            tx.extend_from_slice(payload);
            p.write_u32(REG_SLAVE_ADDR, addr as u32).unwrap();
            program(
                p,
                &[(CMD_RSTART, 0), (CMD_WRITE, tx.len() as u8), (CMD_STOP, 0)],
                &tx,
            );
        }

        /// `RSTART; WRITE 1(addr+R); READ 1; STOP`, returning the received byte.
        fn read_byte(p: &mut Esp32I2c, addr: u8) -> u8 {
            p.write_u32(REG_SLAVE_ADDR, addr as u32).unwrap();
            program(
                p,
                &[
                    (CMD_RSTART, 0),
                    (CMD_WRITE, 1),
                    (CMD_READ, 1),
                    (CMD_STOP, 0),
                ],
                &[(addr << 1) | 1],
            );
            p.read_u32(REG_DATA).unwrap() as u8
        }

        fn nacked(p: &Esp32I2c) -> bool {
            p.read_u32(REG_INT_RAW).unwrap() & INT_NACK != 0
        }

        /// Address-only probe: `RSTART; WRITE 1(addr+W); STOP`.
        fn probe_acked(p: &mut Esp32I2c, addr: u8) -> bool {
            p.write_u32(REG_SLAVE_ADDR, addr as u32).unwrap();
            program(
                p,
                &[(CMD_RSTART, 0), (CMD_WRITE, 1), (CMD_STOP, 0)],
                &[addr << 1],
            );
            !nacked(p)
        }

        #[test]
        fn four_sensors_at_one_address_answer_independently() {
            let mut p = controller();
            for ch in 0..4u8 {
                write_bytes(&mut p, MUX_ADDR, &[1 << ch]);
                assert_eq!(
                    read_byte(&mut p, SENSOR_ADDR),
                    tag_for(ch),
                    "channel {ch} must be answered by the sensor wired to it"
                );
            }
        }

        #[test]
        fn switching_channels_changes_which_sensor_answers() {
            let mut p = controller();
            for ch in [2u8, 0, 3, 1, 3, 0] {
                write_bytes(&mut p, MUX_ADDR, &[1 << ch]);
                assert_eq!(read_byte(&mut p, SENSOR_ADDR), tag_for(ch), "channel {ch}");
            }
        }

        #[test]
        fn control_register_reads_back_over_the_bus() {
            let mut p = controller();
            write_bytes(&mut p, MUX_ADDR, &[0b0000_1010]);
            assert!(
                probe_acked(&mut p, MUX_ADDR),
                "the switch ACKs its own address"
            );
            assert_eq!(read_byte(&mut p, MUX_ADDR), 0b0000_1010);
        }

        #[test]
        fn a_sensor_on_a_disabled_channel_does_not_answer() {
            let mut p = controller();
            assert!(
                !probe_acked(&mut p, SENSOR_ADDR),
                "with all channels disabled the sensor address must raise INT_NACK, \
                 exactly as an unpopulated bus does"
            );

            write_bytes(&mut p, MUX_ADDR, &[1 << 1]);
            assert!(probe_acked(&mut p, SENSOR_ADDR));
            assert_eq!(read_byte(&mut p, SENSOR_ADDR), tag_for(1));

            write_bytes(&mut p, MUX_ADDR, &[0x00]);
            assert!(
                !probe_acked(&mut p, SENSOR_ADDR),
                "re-isolating the switch takes the sensor off the bus again"
            );
        }

        /// The OTHER address-resolution site on this engine: a zero-payload
        /// WRITE resolves the target from the `SLAVE_ADDR` register instead of
        /// the FIFO. It must consult the switch exactly the same way — and must
        /// NOT resolve a device the switch has isolated.
        #[test]
        fn the_slave_addr_register_path_also_routes_through_the_switch() {
            let mut p = controller();

            // Isolated: the parked address must not resolve.
            p.write_u32(REG_SLAVE_ADDR, SENSOR_ADDR as u32).unwrap();
            program(
                &mut p,
                &[(CMD_RSTART, 0), (CMD_WRITE, 0), (CMD_STOP, 0)],
                &[],
            );
            assert!(
                nacked(&p),
                "a SLAVE_ADDR-parked probe must NACK while every channel is isolated"
            );

            // Enable channel 3, then probe + read through the same path.
            write_bytes(&mut p, MUX_ADDR, &[1 << 3]);
            p.write_u32(REG_SLAVE_ADDR, SENSOR_ADDR as u32).unwrap();
            program(
                &mut p,
                &[
                    (CMD_RSTART, 0),
                    (CMD_WRITE, 0),
                    (CMD_READ, 1),
                    (CMD_STOP, 0),
                ],
                &[],
            );
            assert!(!nacked(&p), "channel 3 is enabled; 0x13 must ACK");
            assert_eq!(
                p.read_u32(REG_DATA).unwrap() as u8,
                tag_for(3),
                "the SLAVE_ADDR path must reach the sensor on the SELECTED channel"
            );
        }

        #[test]
        fn a_write_reaches_only_the_selected_channel() {
            let mut p = controller();
            write_bytes(&mut p, MUX_ADDR, &[1 << 2]);
            write_bytes(&mut p, SENSOR_ADDR, &[0x5A]);

            with_mux(&p, |mux| {
                assert_eq!(bytes_written_to(mux, 2), vec![0x5A]);
                for ch in [0u8, 1, 3] {
                    assert!(
                        bytes_written_to(mux, ch).is_empty(),
                        "channel {ch} is isolated and must receive nothing"
                    );
                }
            });
        }
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
