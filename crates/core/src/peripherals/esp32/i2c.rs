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

use std::collections::BTreeMap;

use crate::peripherals::esp_i2c_core::{
    EspI2cCore, EspI2cWire, RouteGate, CMD_DONE_BIT, CTR_TRANS_START_BIT, FIFO_CAPACITY,
    FIFO_CONF_RX_RST, FIFO_CONF_TX_RST, REG_CTR, REG_DATA, REG_FIFO_CONF, REG_FIFO_ST, REG_INT_CLR,
    REG_INT_ENA, REG_INT_RAW, REG_INT_ST, REG_SLAVE_ADDR, REG_SR,
};
use crate::peripherals::i2c::I2cDevice;
use crate::peripherals::pad_lines::PadLines;
use crate::{Peripheral, PeripheralTickResult, SimResult};

use crate::peripherals::esp_i2c_core::REG_CMD0;
pub(crate) use crate::peripherals::esp_i2c_core::{I2C_LINES, LINE_SCL, LINE_SDA};
/// The family register offsets and interrupt bits, from the one place they are
/// stated. The classic part places every one of them exactly where the C3 and
/// the S3 do; what it does NOT share (16 COMD slots, the original opcode
/// numbering, the CTR reset value, 14-bit SCL period fields) stays below.
pub use crate::peripherals::esp_i2c_core::{INT_END_DETECT, INT_NACK, INT_TRANS_COMPLETE};

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

/// COMD15 at 0x94 — the classic part's LAST command slot, and the reason it
/// cannot borrow the C3/S3 timing register file: those parts decode 0x78..0x94
/// as SCL_ST_TIME_OUT / SCL_MAIN_ST_TIME_OUT / SCL_SP_CONF / SCL_STRETCH_CONF.
const REG_CMD15: u64 = 0x94;

/// CTR reset: SCL_FORCE_OUT (bit 1) | SDA_FORCE_OUT (bit 0), both default 1.
/// The classic CTR has no `SAMPLE_SCL_LEVEL` / `RX_FULL_ACK_LEVEL` default-1
/// bits, which is why this is 0x3 and the C3/S3 reset is 0x20B.
const CTR_RESET: u32 = 0x0000_0003;

/// SR bit 0: ACK_REC — set when the slave acknowledged during the most recent
/// command. esp-hal raises `AcknowledgeCheckFailed` after MST_COMPLETE if clear.
/// (Same bit as the C3/S3 `RESP_REC`; the classic SR carries no STRETCH_CAUSE
/// field above it, so [`Esp32I2c::status_register`] is not shared either.)
const SR_ACK_REC: u32 = 1 << 0;

/// Classic ESP32 has 16 COMD slots at offsets 0x58..0x94 (COMD0..COMD15).
const NUM_CMDS: usize = 16;

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

/// `I2C_SDA_HOLD_REG` (0x30), `SDA_HOLD_TIME` [9:0] APB cycles.
const REG_SDA_HOLD: u64 = 0x30;

/// The classic COMD opcode numbering, for the bit-level net engine.
const NET_OPCODES: crate::peripherals::esp_i2c_net::Opcodes =
    crate::peripherals::esp_i2c_net::Opcodes {
        rstart: 0,
        write: 1,
        read: 2,
        stop: 3,
        end: 4,
    };

/// Scheduler token for the zero-delay wake that re-derives the interrupt
/// level after a net edge latched an interrupt bit.
const NET_IRQ_TOKEN: u32 = u32::MAX - 1;

pub struct Esp32I2c {
    /// Bus cycle clock. Its presence is what `scheduler_mode()` reads, so an
    /// attached clock is what migrates this model off the walk.
    clock: Option<crate::cycle_clock::CycleClock>,
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
    /// Attached slaves, address resolution and the RX FIFO — the part of this
    /// controller that is the same part on every Espressif I²C.
    core: EspI2cCore,
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
    /// NOT the C3/S3 [`crate::peripherals::esp_i2c_core::EspI2cTimingRegs`]: the
    /// classic timing registers have never been transcribed from an SVD, and
    /// half of their offsets are COMD slots here.
    other: BTreeMap<u64, u32>,
    /// The command list's waveform, narrated onto the pads when the list
    /// finishes (this engine charges no wire time as it runs).
    wire: EspI2cWire,
    /// Scheduler mode: an MMIO write changed the interrupt level, or clock
    /// attachment found it already asserted
    /// (`int_raw & int_ena != 0`). A scheduler event re-derives the classic
    /// DPORT level, so the change requests a zero-delay wake (see
    /// `take_scheduled_events`).
    level_wake: bool,
    /// The bit-level engine that owns SCL/SDA while the GPIO matrix routes
    /// one of them to a pad on a world `gpio_net`
    /// ([`crate::peripherals::esp_i2c_net`]). Off a net the command list
    /// runs instantly, as below.
    net: crate::peripherals::esp_i2c_net::EspI2cNet,
    /// `SR` bits the net engine owns (ACK_REC, SLAVE_RW, ARB_LOST, BUS_BUSY,
    /// SLAVE_ADDRESSED).
    net_sr: u32,
    /// Interrupt level last reported through a scheduler event in net mode.
    net_irq: crate::peripherals::esp_gpio_net::IrqLevelWatch,
}

/// Event token for the zero-delay level re-derivation wake.
const I2C_LEVEL_WAKE_TOKEN: u32 = 0x12C0_0001;

/// AHB-bus TX FIFO alias (`I2C0` at `0x6001_301c`). esp-idf `i2c_ll_write_txfifo`
/// writes here instead of the APB DATA register at `0x3FF5_301c`.
pub struct Esp32I2cAhbFifo {
    tx_fifo: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<u8>>>,
    /// Bus index of the `Esp32I2c` that owns this FIFO. Alias writes harvest
    /// the owner's pending events too. FIFO pushes do not change its IRQ level;
    /// TRANS_START on the APB controller completes the command list and can
    /// request a level wake.
    owner: usize,
}

impl std::fmt::Debug for Esp32I2cAhbFifo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Esp32I2cAhbFifo")
    }
}

impl Esp32I2c {
    crate::cycle_clock::scheduler_mode!();

    pub fn new() -> Self {
        Self {
            clock: None,
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
            core: EspI2cCore::new(),
            active_slave: None,
            expects_addr: true,
            intr_source_id: I2C0_INTR_SOURCE_ID,
            wire: EspI2cWire::new(),
            other: BTreeMap::new(),
            level_wake: false,
            net: Default::default(),
            net_sr: 0,
            net_irq: Default::default(),
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
    pub fn ahb_tx_fifo_alias(&self, owner: usize) -> Esp32I2cAhbFifo {
        Esp32I2cAhbFifo {
            tx_fifo: std::sync::Arc::clone(&self.tx_fifo),
            owner,
        }
    }

    /// The shared pad-line cell for this controller, created on first use.
    /// Called at bus wiring time; an open-drain bus with pull-ups idles high.
    pub(crate) fn pad_lines_arc(&mut self) -> std::sync::Arc<PadLines> {
        self.wire.pad_lines_arc()
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

    /// The address byte as it appears on the wire when the address comes from
    /// the `SLAVE_ADDR` register rather than the TX FIFO.
    fn slave_addr_byte(&self) -> u8 {
        EspI2cCore::slave_addr_byte(self.slave_addr)
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
        let bit_time = self.bit_time_cycles();
        self.wire.flush(bit_time);
    }

    /// Raw slave push — does NOT wrap for tracing. The only production caller is
    /// the bus choke point [`crate::bus::SystemBus::attach_i2c_slave`], which
    /// wraps first.
    pub(crate) fn push_slave(&mut self, slave: Box<dyn I2cDevice>) {
        self.core.push_slave(slave);
    }

    /// Borrow attached slaves (browser sensor readback / inspect).
    pub fn attached_slaves(&self) -> &[Box<dyn I2cDevice>] {
        self.core.slaves()
    }

    fn fifo_status(&self) -> u32 {
        // FIFO_ST.TXFIFO_START_ADDR (bits 14..10) is the TX-FIFO read pointer:
        // bytes consumed by the current command-list run. 0 at cold reset.
        let tx_raddr = (self.tx_pop_count as u32) & 0x1F;
        tx_raddr << 10
    }

    fn status_register(&self) -> u32 {
        // SR: bit 0 ACK_REC, RXFIFO_CNT at bits 13..8, TXFIFO_CNT at bits 23..18.
        let rx = (self.core.rx_len() as u32) & 0x3F;
        let tx = (self.tx_fifo.lock().unwrap().len() as u32) & 0x3F;
        if self.net_mode() {
            return self.net_sr | (rx << 8) | (tx << 18);
        }
        (self.sr & SR_ACK_REC) | (rx << 8) | (tx << 18)
    }

    /// Resolve a slave from SLAVE_ADDR (7-bit or 8-bit shifted form). Used when
    /// Arduino/ESP-IDF parks the target in SLAVE_ADDR and does not push the
    /// address byte into the TX FIFO.
    ///
    /// Every classic-ESP32 slave is wired straight to the controller's pads, so
    /// the route gate reaches all of them — unlike the C3, which resolves the
    /// same way behind a live GPIO-matrix route.
    fn find_slave_from_slave_addr_register(&mut self) -> Option<usize> {
        self.core
            .find_by_slave_addr_register(self.slave_addr, &RouteGate::All)
    }

    /// Resolve the slave that answers to `address` and tell it which address
    /// the master selected.
    fn find_slave_by_address(&mut self, address: u8) -> Option<usize> {
        self.core.find_by_address(address, &RouteGate::All)
    }
}

impl Esp32I2c {
    /// `true` while the GPIO matrix routes SCL or SDA to a pad on a world
    /// `gpio_net`: the net engine owns the wire.
    fn net_mode(&self) -> bool {
        self.wire.on_net()
    }

    fn net_now(&self) -> u64 {
        self.clock
            .as_ref()
            .map_or(0, crate::cycle_clock::CycleClock::now)
    }

    /// Run `f` with the net engine, this model as its register host, and the
    /// wire, stamping the wire's reports with `now`.
    fn with_net<R>(
        &mut self,
        now: u64,
        f: impl FnOnce(&mut crate::peripherals::esp_i2c_net::EspI2cNet, &mut Self, &PadLines) -> R,
    ) -> Option<R> {
        let pads = self.wire.lines_arc()?;
        let mut net = std::mem::take(&mut self.net);
        pads.set_stamp(Some(now));
        let r = f(&mut net, self, &pads);
        pads.set_stamp(None);
        self.net = net;
        Some(r)
    }

    /// Transfers this controller ended with STOP as a master on a net.
    pub fn net_master_transfers(&self) -> u64 {
        self.net.master_transfers()
    }

    /// Transfers that addressed this controller as a slave on a net.
    pub fn net_slave_transfers(&self) -> u64 {
        self.net.slave_transfers()
    }

    /// The register writes the net engine acts on. A byte written through
    /// the AHB FIFO alias reaches the engine with the next APB write.
    fn net_write(&mut self, offset: u64, value: u32) {
        let now = self.net_now();
        self.with_net(now, |net, host, pads| {
            match offset {
                REG_CTR if value & CTR_TRANS_START_BIT != 0 => net.start(host, now, pads),
                REG_DATA => net.tx_written(host, now, pads),
                _ => {}
            }
            net.after_write(host, now, pads);
        });
    }

    fn net_step(&mut self, now: u64) {
        if self.net.deadline().is_some_and(|d| d <= now) {
            self.with_net(now, |net, host, pads| net.step(host, now, pads));
        }
    }
}

impl crate::peripherals::esp_i2c_net::EspI2cHost for Esp32I2c {
    fn opcodes(&self) -> crate::peripherals::esp_i2c_net::Opcodes {
        NET_OPCODES
    }

    fn ctr(&self) -> u32 {
        self.ctr
    }

    fn slave_addr(&self) -> u32 {
        self.slave_addr
    }

    fn fifo_conf(&self) -> u32 {
        self.fifo_conf
    }

    /// The classic part has no `SCL_STRETCH_CONF`: a slave whose TX FIFO is
    /// empty sends 0xFF rather than stretching SCL.
    fn stretch_conf(&self) -> u32 {
        0
    }

    fn cmd(&self, i: usize) -> u32 {
        self.cmds.get(i).copied().unwrap_or(0)
    }

    fn set_cmd(&mut self, i: usize, value: u32) {
        if let Some(slot) = self.cmds.get_mut(i) {
            *slot = value;
        }
    }

    fn cmd_count(&self) -> usize {
        NUM_CMDS
    }

    fn tx_pop(&mut self) -> Option<u8> {
        let b = self.tx_fifo.lock().unwrap().pop_front()?;
        self.tx_pop_count += 1;
        Some(b)
    }

    fn rx_push(&mut self, byte: u8) -> bool {
        if self.core.rx_len() >= FIFO_CAPACITY {
            return false;
        }
        self.core.rx_push(byte);
        true
    }

    fn rx_len(&self) -> usize {
        self.core.rx_len()
    }

    fn raise(&mut self, bits: u32) {
        self.int_raw |= bits;
    }

    fn sr_update(&mut self, set: u32, clear: u32) {
        self.net_sr = (self.net_sr & !clear) | set;
    }

    /// `SCL_LOW_PERIOD`, `SCL_HIGH_PERIOD` (14 bits) and `SDA_HOLD_TIME`
    /// (10 bits) count APB cycles (TRM v4.6 §11.4).
    fn timing(&self) -> crate::peripherals::i2c_bitbus::Timing {
        let apb =
            |off: u64, mask: u32| u64::from(self.decode_word(off) & mask).max(1) * CORE_PER_APB;
        crate::peripherals::i2c_bitbus::Timing::new(
            apb(REG_SCL_LOW_PERIOD, SCL_PERIOD_MASK),
            apb(REG_SCL_HIGH_PERIOD, SCL_PERIOD_MASK),
            apb(REG_SDA_HOLD, 0x3FF),
        )
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
            .field("slaves_count", &self.core.slave_count())
            .finish()
    }
}

impl Peripheral for Esp32I2cAhbFifo {
    fn scheduler_wake_owner(&self) -> Option<usize> {
        Some(self.owner)
    }
    /// A write-only alias onto `Esp32I2c`'s TX FIFO: a `write` pushes a byte,
    /// a `read` returns 0. The engine that drains that FIFO lives on
    /// [`Esp32I2c`], not here. No `tick`/`tick_elapsed` override, so the walk
    /// gets the trait default (`PeripheralTickResult::default()`) — no IRQ,
    /// DMA request, mmio-write or fired event, for every reachable state.
    fn needs_legacy_walk(&self) -> bool {
        false
    }
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
            REG_DATA => u32::from(self.core.rx_peek()),
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
    fn line_names(&self) -> &'static [&'static str] {
        I2C_LINES
    }

    fn wire_lines(&self) -> Option<&PadLines> {
        self.wire.lines()
    }

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
            self.core.rx_pop();
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
        let before = self.int_raw & self.int_ena != 0;
        let out = self.write_u32_regs(offset, value);
        if self.net_mode() {
            self.net_write(offset, value);
        }
        if self.scheduler_mode() && (self.int_raw & self.int_ena != 0) != before {
            self.level_wake = true;
        }
        out
    }

    /// A level edge caused by an MMIO write (TRANS_START completing the
    /// command list, INT_ENA, INT_CLR) must reach the DPORT matrix now. On a
    /// classic bus the scheduler re-derives levels only when an event fires
    /// (`apply_event_result` -> `deliver_scheduled_irq_levels`); without this
    /// wake, a loop task that blocks on the I2C ISR with no other scheduled
    /// activity (no UART TX in flight) waits out the driver's whole timeout
    /// (Arduino `Wire`: ~1 s per transaction, e.g. `LiquidCrystal_I2C::init`).
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        let mut events = Vec::new();
        if std::mem::take(&mut self.level_wake) {
            events.push((0, I2C_LEVEL_WAKE_TOKEN));
        }
        if self.net_mode() {
            let now = self.net_now();
            if let Some(ev) = self.net.take_event(now) {
                events.push(ev);
            }
            // An interrupt bit latched by a net edge: one event per change of
            // the level, so the interrupt matrix re-derives its sources.
            if self
                .net_irq
                .take(self.int_raw & self.int_ena != 0)
                .is_some()
            {
                events.push((0, NET_IRQ_TOKEN));
            }
        }
        events
    }

    fn wire_input_edge(&mut self, line: usize, level: bool, cycle: u64) -> bool {
        if !self.net_mode() {
            return false;
        }
        self.with_net(cycle, |net, host, pads| {
            net.on_edge(host, line, level, cycle, pads)
        });
        true
    }

    fn sync_to(&mut self, now: u64) {
        self.net_step(now);
    }

    /// The wake carries no work: firing it is what makes the event path
    /// re-poll `matrix_irq_sources_into` and route the current level.
    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token == crate::peripherals::esp_i2c_net::NET_TOKEN {
            let now = sched.now();
            let next = self
                .with_net(now, |net, host, pads| net.on_event(host, now, pads))
                .flatten();
            return crate::sched::EventResult {
                reschedule_delay: next,
                ..Default::default()
            };
        }
        crate::sched::EventResult::default()
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.net.deadline().is_some() {
            let now = self.net_now();
            self.net_step(now);
        }
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

    /// The same level, for the DPORT scheduler arm.
    ///
    /// `deliver_scheduled_irq_levels` polls this when an event fires. Like
    /// `tick()`, this only reports the current `int_raw & int_ena` level. The
    /// MMIO edge wake triggers re-derivation even when no other peripheral
    /// has scheduled work; its handler does not mutate or reschedule the level.
    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if self.int_raw & self.int_ena != 0 {
            out.push(self.intr_source_id);
        }
    }

    /// The walk need not drive this model, and BOTH build configurations are
    /// covered — which is the part worth checking rather than assuming:
    ///
    /// * with `event-scheduler`: the production walk skips `uses_scheduler()`
    ///   peripherals (`bus/tick.rs`, gated on the same feature), and the level
    ///   reaches the CPU through `matrix_irq_sources_into` + the DPORT arm in
    ///   `deliver_scheduled_irq_levels`;
    /// * without it: that skip does not compile, `legacy_walk_disabled` is
    ///   never read, so the walk always runs and `tick()` emits the level as
    ///   it always did.
    ///
    /// The two halves are coupled by construction — walk deletion and the
    /// matrix poll are gated on the SAME feature — so there is no build in
    /// which the level has no route. That coupling is why this needs no
    /// `cfg!` of its own.
    fn attach_cycle_clock(&mut self, clock: crate::cycle_clock::CycleClock) {
        self.clock = Some(clock);
        // A caller may configure an asserted level before bus attachment.
        // Bootstrap must export it even if no subsequent MMIO write occurs.
        self.level_wake |= self.scheduler_mode() && self.int_raw & self.int_ena != 0;
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    /// CONDITIONAL, not a literal `false`.
    ///
    /// `walk_starvation_contract` rule A rejects a literal `false` on a model
    /// whose `tick()` does walk work, and this one's does -- it emits the level
    /// as `explicit_irqs`. The rule is right and the first version of this was
    /// wrong: declaring walk-independence while still needing the walk to
    /// deliver an IRQ is exactly the starvation shape the contract exists for.
    ///
    /// Keyed on an attached clock instead, so the two halves cannot separate:
    /// no clock means no scheduler, which means the walk still drives `tick()`
    /// and the IRQ still goes out the way it always did.
    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
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

    fn has_attached_time_devices(&self) -> bool {
        !self.attached_slaves().is_empty()
    }

    fn advance_attached_i2c_us(&mut self, us: u64) {
        self.core.advance_time_us(us);
    }

    /// Tier 2: collect this controller's attached devices' pin drives.
    fn drain_attached_pin_drives(&mut self, out: &mut Vec<(String, String, bool)>) {
        self.core.drain_pin_drives(out);
    }

    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        self.core.for_each_sim_input(f)
    }

    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        self.core.for_each_attached_device(f);
    }
}

impl Esp32I2c {
    /// The register-file write itself; `Peripheral::write_u32` wraps it to
    /// notice interrupt-level edges.
    fn write_u32_regs(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if i2c_trace_enabled() {
            eprintln!("ESP32 I2C W [0x{offset:02x}] = 0x{value:08x}");
        }
        match offset {
            // On a net the engine runs the command list on the wire
            // (`net_write` below); TRANS_START never reads back.
            REG_CTR if self.net_mode() => self.ctr = value & !CTR_TRANS_START_BIT,
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
                if value & FIFO_CONF_RX_RST != 0 {
                    self.core.rx_clear();
                }
                if value & FIFO_CONF_TX_RST != 0 {
                    self.tx_fifo.lock().unwrap().clear();
                    self.tx_pop_count = 0;
                }
                self.fifo_conf &= !(FIFO_CONF_RX_RST | FIFO_CONF_TX_RST);
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
                        self.core.slave_mut(slave_idx).start();
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
                        let addr_byte = self.slave_addr_byte();
                        self.wire.push(addr_byte, active.is_some());
                        if let Some(slave_idx) = active {
                            self.core.slave_mut(slave_idx).start();
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
                            self.wire.push(b, active.is_some());
                            if active.is_none() {
                                // Fallback: address only in SLAVE_ADDR, payload in FIFO.
                                active = self.find_slave_from_slave_addr_register();
                                // The address came from SLAVE_ADDR, so the wire
                                // carried THAT frame, not the FIFO byte we had
                                // provisionally recorded as an address.
                                self.wire.pop_last_addr();
                                let addr_byte = self.slave_addr_byte();
                                self.wire.push(addr_byte, active.is_some());
                                if let Some(slave_idx) = active {
                                    self.core.slave_mut(slave_idx).start();
                                    self.sr |= SR_ACK_REC;
                                    // First FIFO byte is data when SLAVE_ADDR holds target.
                                    self.core.slave_mut(slave_idx).write(b);
                                    self.wire.push(b, true);
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
                            self.core.slave_mut(slave_idx).write(b);
                            self.wire.push(b, true);
                            self.sr |= SR_ACK_REC;
                        }
                    }
                    self.cmds[idx] |= CMD_DONE_BIT;
                }
                OP_READ => {
                    for _ in 0..byte_num {
                        let b = if let Some(slave_idx) = active {
                            self.core.slave_mut(slave_idx).read()
                        } else {
                            0
                        };
                        if active.is_some() {
                            // The master ACKs each byte it reads; the final NACK
                            // is modelled by the STOP that follows.
                            self.wire.push(b, true);
                        }
                        self.core.rx_push(b);
                    }
                    if active.is_some() {
                        self.sr |= SR_ACK_REC;
                    }
                    self.cmds[idx] |= CMD_DONE_BIT;
                }
                OP_STOP => {
                    if let Some(slave_idx) = active {
                        self.core.slave_mut(slave_idx).stop();
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
    use crate::peripherals::esp_i2c_core::esp_i2c_family_suite;

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
        i2c.core.rx_push(0x12);
        i2c.core.rx_push(0x34);

        assert_eq!(i2c.peek(REG_DATA), Some(0x12), "peek reports the head byte");
        assert_eq!(
            i2c.peek(REG_DATA),
            Some(0x12),
            "and again -- nothing consumed"
        );
        assert_eq!(i2c.core.rx_len(), 2, "peeking left the FIFO untouched");

        assert_eq!(
            i2c.read_u32(REG_DATA).unwrap(),
            0x12,
            "the read still gets it"
        );
        assert_eq!(i2c.core.rx_len(), 1, "and the read is what consumed it");
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

    /// Scheduler mode: an MMIO write that flips the interrupt level hands the
    /// scheduler exactly one zero-delay wake, so the classic DPORT route is
    /// re-derived at the write's scheduled deadline without an unrelated
    /// event (the loop task otherwise waits out the whole Wire timeout).
    #[test]
    fn level_edge_from_mmio_write_requests_one_scheduler_wake() {
        let mut p = Esp32I2c::new();
        p.attach_cycle_clock(crate::cycle_clock::CycleClock::default());
        let sched = cfg!(feature = "event-scheduler");
        p.int_raw = INT_TRANS_COMPLETE;
        assert!(p.take_scheduled_events().is_empty(), "no write, no wake");
        p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap(); // level rises
        assert_eq!(
            p.take_scheduled_events(),
            if sched {
                vec![(0, I2C_LEVEL_WAKE_TOKEN)]
            } else {
                vec![]
            }
        );
        assert!(p.take_scheduled_events().is_empty(), "one wake per edge");
        p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap(); // level unchanged
        assert!(p.take_scheduled_events().is_empty(), "no edge, no wake");
        p.write_u32(REG_INT_CLR, INT_TRANS_COMPLETE).unwrap(); // level falls
        assert_eq!(p.take_scheduled_events().len(), usize::from(sched));
    }

    /// Without a clock (legacy walk) the walk's `tick()` carries the level, so
    /// no wake is requested.
    #[test]
    fn walk_mode_requests_no_level_wake() {
        let mut p = Esp32I2c::new();
        p.int_raw = INT_TRANS_COMPLETE;
        p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap();
        assert!(p.take_scheduled_events().is_empty());
    }

    #[test]
    fn attaching_clock_exports_an_already_asserted_level_once() {
        let mut p = Esp32I2c::new();
        p.write_u32(REG_CMD0, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap();
        assert!(p.take_scheduled_events().is_empty(), "still on the walk");
        p.attach_cycle_clock(crate::cycle_clock::CycleClock::default());
        assert_eq!(
            p.take_scheduled_events().len(),
            usize::from(p.scheduler_mode()),
            "bootstrap must export the preconfigured IRQ level"
        );
        assert!(p.take_scheduled_events().is_empty());
    }

    #[test]
    fn unharvested_level_edges_coalesce_and_masked_changes_do_not_wake() {
        let mut p = Esp32I2c::new();
        p.attach_cycle_clock(crate::cycle_clock::CycleClock::default());
        p.write_u32(REG_CMD0, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        assert!(p.take_scheduled_events().is_empty(), "masked completion");
        p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap();
        p.write_u32(REG_INT_ENA, 0).unwrap();
        p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap();
        assert_eq!(
            p.take_scheduled_events().len(),
            usize::from(p.scheduler_mode())
        );
        assert!(p.take_scheduled_events().is_empty());
        // Adding an enabled cause while already asserted is not a level edge.
        p.write_u32(REG_CMD0, cmd(CMD_END, 0)).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE | INT_END_DETECT)
            .unwrap();
        assert!(p.take_scheduled_events().is_empty());
        p.write_u32(REG_INT_CLR, INT_END_DETECT).unwrap();
        assert!(
            p.take_scheduled_events().is_empty(),
            "another enabled cause remains"
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

    /// Classic-ESP32 (and S3) command-list engines execute the whole list
    /// inside the TRANS_START write; the C3 clocks a bit engine, so its
    /// `start_and_run` is a different function in its own file. The shared
    /// suite calls whichever this module passes as `launch`.
    fn start_and_run(p: &mut Esp32I2c) {
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
    }

    esp_i2c_family_suite! {
        ctrl: Esp32I2c,
        p_name: p,
        launch: start_and_run,
        ack_bit: SR_ACK_REC,
        cmd_last: REG_CMD15,
        cmd_slots_test: has_sixteen_command_slots,
        fifo_reset_test: fifo_reset_bit_clears_tx_fifo,
        bmp280_test: write_read_drives_attached_bmp280,
        mux_slave_addr_test: the_slave_addr_register_path_also_routes_through_the_switch,
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
