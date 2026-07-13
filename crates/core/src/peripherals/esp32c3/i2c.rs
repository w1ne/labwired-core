// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 I²C0 controller — cycle-driven, bit-level command-list engine.
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
//! ## Bit-level execution
//!
//! A command list kicked by `CTR.TRANS_START` does NOT complete synchronously.
//! It executes as a bit-level state machine clocked from the machine's step
//! loop (`tick_elapsed`), stretched over simulated cycles:
//!
//! * SCL/SDA timing derives from the controller's REAL clock configuration —
//!   `CLK_CONF` (source select + integer/fractional divider) plus the
//!   `SCL_LOW_PERIOD` / `SCL_HIGH_PERIOD` (+ wait-high) / `SDA_HOLD` /
//!   `SCL_START_HOLD` / `SCL_RSTART_SETUP` / `SCL_STOP_SETUP` /
//!   `SCL_STOP_HOLD` counters, all in I²C module-clock ticks with the TRM's
//!   `reg + 1` counter semantics. If firmware leaves them at reset the reset
//!   values apply — no invented constants.
//! * SDA carries the real bit pattern: START (SDA falls while SCL high),
//!   address + R/W bits MSB-first, ACK/NACK, data bytes, repeated START and
//!   STOP (SDA rises while SCL high).
//! * Slaves stay byte-level ([`I2cDevice`], wrapped by the bus-trace choke
//!   point): the engine consults them at byte boundaries — the address is
//!   resolved (and `start()` signalled) entering the address ACK bit, a
//!   written byte is delivered entering its ACK bit, a read byte is fetched
//!   when its first bit starts clocking — and the slave-driven bits (ACK,
//!   read data) are driven onto SDA from those byte-level answers.
//! * `TRANS_COMPLETE` / `END_DETECT` / `NACK` interrupts and the COMD
//!   `command_done` bits assert at the realistic completion time, not at the
//!   `TRANS_START` write. `SR.BUS_BUSY` reads 1 while a transaction is on the
//!   wire.
//!
//! The driven line levels are published into a shared [`I2cLineLevels`] cell;
//! the C3 GPIO model reads it for pads whose output matrix routes
//! `I2CEXT0_SCL` / `I2CEXT0_SDA`, so `read_gpio_pad` (and the in-engine logic
//! analyzer sampling it) observes the real waveform.
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::peripherals::i2c::I2cDevice;
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

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
/// CTR bit 10: FSM_RST — write-trigger master FSM reset.
const CTR_FSM_RST: u32 = 1 << 10;
/// CTR bit 11: CONF_UPGATE — self-clearing config-sync trigger.
const CTR_CONF_UPGATE: u32 = 1 << 11;

/// SR bit 0: set when the slave responded with ACK during the most recent
/// command. esp-hal checks this after TRANS_COMPLETE — if clear it raises
/// `AcknowledgeCheckFailed(Data)`.
const SR_RESP_REC: u32 = 1 << 0;
/// SR bit 4: BUS_BUSY — set while a transaction is on the wire (per the C3
/// `i2c0.yaml` SR field map).
const SR_BUS_BUSY: u32 = 1 << 4;

/// COMD bit 31: command_done. Set when a command finishes executing.
const CMD_DONE_BIT: u32 = 1 << 31;

/// INT_RAW bit 1: TXFIFO_WM — the TX FIFO is at/below its watermark threshold
/// (asserted at reset, when the FIFO is empty). Real firmware's ISR services it
/// to refill the FIFO mid-burst; the bit engine raises it when a WRITE command
/// underruns so a refilling driver is signalled to feed the stalled transfer.
pub const INT_TXFIFO_WM: u32 = 1 << 1;
pub const INT_END_DETECT: u32 = 1 << 3;
pub const INT_TRANS_COMPLETE: u32 = 1 << 7;
pub const INT_NACK: u32 = 1 << 10;
const SCL_RST_SLV_EN: u32 = 1 << 0;

/// Event-scheduler token: advance the bit engine by one I²C module-clock tick.
/// The engine keeps exactly one such event in flight while a transaction is
/// active (walk-free plan): `take_scheduled_events` bootstraps it from the
/// `TRANS_START` write and `on_event` re-arms it at the next module tick until
/// the engine parks. Opaque to the scheduler.
const I2C_MODULE_TICK_TOKEN: u32 = 0;

/// ESP32-C3 has 8 COMD slots at offsets 0x58..0x78 (COMD0..COMD7 in the yaml).
const NUM_CMDS: usize = 8;
const FIFO_CAPACITY: usize = 32;

// COMD opcodes per ESP32-C3 TRM §16 / esp32c3 PAC `i2c0::comd`:
//   1 = WRITE, 2 = STOP, 3 = READ, 4 = END, 6 = RSTART
const OP_WRITE: u32 = 1;
const OP_STOP: u32 = 2;
const OP_READ: u32 = 3;
const OP_END: u32 = 4;
const OP_RSTART: u32 = 6;
/// COMD bit 10: ack_value — the ACK level the master drives after a received
/// (READ) byte. esp-hal sets it high (NACK) on the final read command.
const CMD_ACK_VALUE_BIT: u32 = 1 << 10;

/// ESP32-C3 CPU clock the engine cycle counter models. `Machine::total_cycles`
/// advances at CPU-instruction rate; the C3 wiring elsewhere (SYSTIMER: "10 CPU
/// cycles per 16 MHz tick", `Systimer::new_with_source(160_000_000, …)`) uses
/// the same 160 MHz convention, so I²C wire time shares one clock base with
/// the timers firmware uses to measure it.
const CPU_CLK_HZ: u64 = 160_000_000;
/// I²C module source clocks selectable via `CLK_CONF.SCLK_SEL` (C3 TRM):
/// 0 = XTAL_CLK (40 MHz), 1 = RC_FAST_CLK (17.5 MHz).
const XTAL_CLK_HZ: u64 = 40_000_000;
const RC_FAST_CLK_HZ: u64 = 17_500_000;

/// Push-mode logic-capture registration for the I²C line cell: which watch
/// channels observe pads currently matrix-routed to SCL / SDA. Maintained by
/// the C3 GPIO model (which owns the routing truth) via
/// [`I2cLineLevels::install_tap`]; consulted by [`I2cLineLevels::set`] so the
/// bit engine pushes an edge at the exact moment it drives a line transition.
#[derive(Debug, Default)]
struct LineTapState {
    tap: Option<crate::logic_capture::LogicTap>,
    scl_chs: Vec<u32>,
    sda_chs: Vec<u32>,
}

/// Live SDA/SCL levels of the I²C0 bus (wired-AND of controller + slave drive,
/// idle high — open-drain with pull-ups). The controller bit engine is the only
/// writer; the C3 GPIO model reads it for pads whose GPIO output matrix
/// (`FUNCn_OUT_SEL_CFG`) routes `I2CEXT0_SCL` / `I2CEXT0_SDA`, so
/// `read_gpio_pad` — and the in-engine logic analyzer sampling through it —
/// observes the real waveform on the routed pads. With push-mode capture
/// armed on a routed pad, [`set`](Self::set) additionally reports each line
/// transition into the shared logic tap (event-driven capture — no polling).
#[derive(Debug)]
pub struct I2cLineLevels {
    scl: AtomicBool,
    sda: AtomicBool,
    tap: std::sync::Mutex<LineTapState>,
}

impl I2cLineLevels {
    fn new() -> Self {
        Self {
            scl: AtomicBool::new(true),
            sda: AtomicBool::new(true),
            tap: std::sync::Mutex::new(LineTapState::default()),
        }
    }

    pub fn scl(&self) -> bool {
        self.scl.load(Ordering::Relaxed)
    }

    pub fn sda(&self) -> bool {
        self.sda.load(Ordering::Relaxed)
    }

    fn set(&self, scl: bool, sda: bool) {
        let old_scl = self.scl.swap(scl, Ordering::Relaxed);
        let old_sda = self.sda.swap(sda, Ordering::Relaxed);
        if old_scl == scl && old_sda == sda {
            return;
        }
        // A line actually transitioned: report it to any watch channels whose
        // pads the GPIO matrix currently routes here. Lock taken only on
        // transitions (module-tick rate, not per engine cycle).
        let t = self.tap.lock().unwrap();
        if let Some(tap) = &t.tap {
            if old_scl != scl {
                for &ch in &t.scl_chs {
                    tap.push(ch, scl);
                }
            }
            if old_sda != sda {
                for &ch in &t.sda_chs {
                    tap.push(ch, sda);
                }
            }
        }
    }

    /// Install (or clear, with `tap = None`) the push-capture registration.
    /// Called by the C3 GPIO model at watch install time and whenever a write
    /// changes the routing of a watched pad, so the channel lists always
    /// mirror the live GPIO matrix state.
    pub(crate) fn install_tap(
        &self,
        tap: Option<crate::logic_capture::LogicTap>,
        scl_chs: Vec<u32>,
        sda_chs: Vec<u32>,
    ) {
        let mut t = self.tap.lock().unwrap();
        t.tap = tap;
        t.scl_chs = scl_chs;
        t.sda_chs = sda_chs;
    }
}

/// Wire timing snapshot, derived from the timing registers at `TRANS_START`.
/// All phase durations are in I²C module-clock ticks with the TRM's `reg + 1`
/// down-counter semantics; `num`/`den` express one module tick in engine
/// cycles as an exact fraction (`CPU_CLK_HZ · divider / source_hz`), so the
/// engine accumulates time without rounding drift.
#[derive(Debug, Clone, Copy)]
struct EngineTiming {
    /// Engine cycles per module tick = `num / den`.
    num: u64,
    den: u64,
    /// SCL low width (`SCL_LOW_PERIOD + 1`).
    low: u32,
    /// SCL high width (`SCL_HIGH_PERIOD + SCL_WAIT_HIGH_PERIOD + 1`).
    high: u32,
    /// SDA transition delay after SCL falls (`SDA_HOLD + 1`).
    sda_hold: u32,
    /// SDA-low → SCL-low hold after a (repeated) START (`SCL_START_HOLD + 1`).
    start_hold: u32,
    /// SCL-high setup before SDA falls on a repeated START
    /// (`SCL_RSTART_SETUP + 1`).
    rstart_setup: u32,
    /// SCL-high setup before SDA rises on STOP (`SCL_STOP_SETUP + 1`).
    stop_setup: u32,
    /// Bus-free hold after the STOP condition (`SCL_STOP_HOLD + 1`).
    stop_hold: u32,
}

/// Where the bit engine is inside the current wire segment. Every variant maps
/// to one fixed (SCL, SDA) pair held for a counted number of module ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EngineState {
    /// No transaction on the wire.
    Idle,
    /// END pause: command list paused, bus held (SCL low), awaiting the next
    /// `TRANS_START`.
    Paused,
    /// (Repeated) START driven: SDA low, SCL high, holding `start_hold`.
    StartHold,
    /// Repeated START, phase 1: SCL low, SDA released (`sda_hold`).
    RestartRelease,
    /// Repeated START, phase 2: SCL high with SDA high (`rstart_setup`).
    RestartSetup,
    /// TX-FIFO underrun during a WRITE: SCL held low (clock-stretch), waiting
    /// for firmware to refill the TX FIFO. The controller does NOT clock a byte
    /// until a real one is available — it never fabricates 0x00. Re-checks the
    /// FIFO every module tick and resumes the byte once it is fed.
    TxStall,
    /// Data bit: SCL low, SDA still at the previous level (`sda_hold`).
    BitLowHold,
    /// Data bit: SCL low, SDA at this bit's level (rest of `low`).
    BitLowDrive,
    /// Data bit: SCL high (`high`).
    BitHigh,
    /// STOP, phase 1: SCL low, SDA at the previous level (`sda_hold`).
    StopLowHold,
    /// STOP, phase 2: SCL low, SDA pulled low (rest of `low`).
    StopLowDrive,
    /// STOP, phase 3: SCL high, SDA still low (`stop_setup`).
    StopSetup,
    /// STOP condition driven (SDA rose while SCL high), holding `stop_hold`.
    StopHold,
}

/// Cycle-driven bit engine state. Owned by [`Esp32c3I2c`]; ticked from the
/// machine's peripheral walk via `tick_elapsed`.
#[derive(Debug)]
struct BitEngine {
    state: EngineState,
    /// Module ticks left in the current segment (≥ 1 while active).
    ticks_left: u32,
    /// Engine-cycle fraction accumulator, in units of `1/den` engine cycles.
    acc: u64,
    timing: EngineTiming,
    /// Index of the COMD slot currently executing.
    cmd_idx: usize,
    /// Bytes remaining in the current WRITE/READ command.
    bytes_left: usize,
    /// The byte currently being clocked (TX byte, or the slave's read answer).
    cur_byte: u8,
    /// Bit position inside the current byte: 0..=7 data (MSB first), 8 = ACK.
    bit_idx: u8,
    cur_is_read: bool,
    /// Master ACK level for received bytes (COMD `ack_value`).
    cur_ack_value: bool,
    /// The byte being clocked is an address frame.
    addr_byte: bool,
    /// A START has been driven and no STOP yet (includes END pauses).
    bus_held: bool,
    /// Currently driven line levels (mirror of the shared [`I2cLineLevels`]).
    scl: bool,
    sda: bool,
}

impl BitEngine {
    fn new() -> Self {
        Self {
            state: EngineState::Idle,
            ticks_left: 0,
            acc: 0,
            // Placeholder; recomputed from the registers at every TRANS_START.
            timing: EngineTiming {
                num: CPU_CLK_HZ,
                den: XTAL_CLK_HZ,
                low: 1,
                high: 1,
                sda_hold: 1,
                start_hold: 9,
                rstart_setup: 9,
                stop_setup: 9,
                stop_hold: 9,
            },
            cmd_idx: 0,
            bytes_left: 0,
            cur_byte: 0,
            bit_idx: 0,
            cur_is_read: false,
            cur_ack_value: false,
            addr_byte: false,
            bus_held: false,
            scl: true,
            sda: true,
        }
    }
}

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
    /// Interrupt-matrix source this instance asserts (29 for I2C0).
    intr_source_id: u32,
    active_slave: Option<usize>,
    expects_addr: bool,
    /// Cycle-driven bit engine executing the command list on the wire.
    engine: BitEngine,
    /// Shared SDA/SCL line levels, read by the C3 GPIO model for matrix-routed
    /// pads. Created lazily by [`Self::line_levels_arc`] at bus wiring time.
    lines: Option<Arc<I2cLineLevels>>,

    /// Bus-published cycle clock (walk-free plan). `Some` once
    /// `SystemBus::add_peripheral` attaches it. Its presence (under the
    /// `event-scheduler` feature) flips the model onto the event scheduler:
    /// the per-cycle walk skips it and the bit engine is driven by
    /// self-perpetuating module-tick events instead. `None` (feature off, a
    /// hand-built bus, or the differential's `force_legacy_walk`) keeps the
    /// legacy per-cycle walk. Not serialized — re-attached by the bus.
    clock: Option<CycleClock>,
    /// CPU cycle the bit engine has been advanced to (scheduler mode anchor).
    /// The write path (`sync_to`) and the module-tick event (`on_event`) both
    /// advance the engine by `now - last_synced` and bump this, so the two
    /// paths compose without double-counting elapsed cycles.
    last_synced: u64,
    /// `true` while exactly one module-tick event is in flight for this engine
    /// (walk-free plan). Guards against re-bootstrapping a second event on a
    /// later MMIO write while a transaction is already clocking: only
    /// `take_scheduled_events` (no event in flight) may arm one, and `on_event`
    /// re-arms the single successor. Mirrors the generic SPI `scheduled` gate.
    scheduled: bool,

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
            intr_source_id: I2C0_INTR_SOURCE_ID,
            active_slave: None,
            expects_addr: true,
            engine: BitEngine::new(),
            lines: None,
            clock: None,
            last_synced: 0,
            scheduled: false,

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

    /// Raw slave push — does NOT wrap for tracing. The only production caller is
    /// the bus choke point [`crate::bus::SystemBus::attach_i2c_slave`], which
    /// wraps first. Slaves are matched by address bits at transaction time;
    /// later additions take precedence on duplicate addresses.
    pub(crate) fn push_slave(&mut self, slave: Box<dyn I2cDevice>) {
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
        // SR (C3 i2c0.yaml): bit 0 RESP_REC, bit 4 BUS_BUSY, bits 8..13
        // RXFIFO_CNT, bits 14..15 STRETCH_CAUSE (reset 0b11 == yaml
        // reset_value 49152), bits 18..23 TXFIFO_CNT.
        const SR_STRETCH_CAUSE_RESET: u32 = 0x0000_C000;
        let rx = (self.rx_fifo.borrow().len() as u32) & 0x3F;
        let tx = (self.tx_fifo.len() as u32) & 0x3F;
        let busy = if self.engine_active() || self.engine.bus_held {
            SR_BUS_BUSY
        } else {
            0
        };
        (self.sr & SR_RESP_REC) | busy | SR_STRETCH_CAUSE_RESET | (rx << 8) | (tx << 18)
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
            .field("engine_state", &self.engine.state)
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
                if value & CTR_FSM_RST != 0 {
                    // Master FSM reset: abort any in-flight transaction and
                    // release the (open-drain) lines back to bus-idle.
                    self.fsm_reset();
                }
                if value & CTR_TRANS_START_BIT != 0 {
                    self.start_transaction();
                    // Auto-clear TRANS_START like real silicon.
                    self.ctr &= !CTR_TRANS_START_BIT;
                }
                // One-shot control bits self-clear after the write-triggered
                // operation is accepted.
                self.ctr &= !(CTR_FSM_RST | CTR_CONF_UPGATE);
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
            REG_SCL_SP_CONF => {
                masked_write(&mut self.reg_scl_sp_conf, value, 0x0000_00FF);
                // SCL_RST_SLV_EN is R/W/SC. Arduino's C3 bus-clear helper
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
        self.tick_elapsed(1)
    }

    /// Advance the bit engine by `cycles` engine cycles, then assert the level
    /// interrupt. The engine converts elapsed engine cycles into I²C
    /// module-clock ticks through the exact `num/den` fraction snapshotted at
    /// `TRANS_START`, so wire timing is independent of the peripheral tick
    /// interval the host chose.
    ///
    /// This is the LEGACY per-cycle walk path. In scheduler mode
    /// ([`Self::uses_scheduler`] true) the walk skips this peripheral entirely
    /// and the engine is driven by module-tick events instead; the guard keeps
    /// a stray direct call from advancing the engine twice.
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        if !self.uses_scheduler() {
            self.advance_engine(cycles);
        }
        // LEVEL interrupt: assert the I2C0 source every tick while any enabled
        // INT bit is set, mirroring real silicon (INT_RAW stays asserted until
        // the ISR writes INT_CLR).
        let mut explicit = Vec::new();
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

    fn legacy_tick_active(&self) -> bool {
        self.engine_active() || self.int_raw & self.int_ena != 0
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    /// Walk-free plan: driven by the event scheduler once the bus has attached
    /// its cycle clock (production `add_peripheral` always does, under the
    /// `event-scheduler` feature). The per-cycle walk then skips this
    /// peripheral; the bit engine advances via `sync_to` (write path) and
    /// self-perpetuating module-tick events (`on_event`). Without a clock
    /// (feature off, a hand-built bus, or `force_legacy_walk`) it stays on the
    /// legacy walk so those callers keep the old exact semantics.
    fn uses_scheduler(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Anchor the bit engine to CPU cycle `now`, advancing it over the cycles
    /// elapsed since the last sync. The bus calls this before every MMIO write
    /// (so a `TRANS_START` / config / `INT_CLR` write observes the up-to-date
    /// engine) and it composes with `on_event` through the shared `last_synced`
    /// anchor without double-counting.
    fn sync_to(&mut self, now: u64) {
        if now > self.last_synced {
            self.advance_engine(now - self.last_synced);
            self.last_synced = now;
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        // Anchor at the clock's current value so cycles elapsed before attach
        // (normally zero — attach happens at bus assembly) are not retroactively
        // credited to the engine (mirrors the rtc_timer #516 re-anchor contract).
        self.last_synced = clock.now();
        self.clock = Some(clock);
    }

    /// C3 interrupt-matrix level: the I2C0 source while any enabled INT bit is
    /// set — the exact condition `tick_elapsed` pushes on the legacy walk. In
    /// scheduler mode the walk no longer re-emits it, so the bus re-derives the
    /// level from here (`refresh_esp32c3_sched_sources`, polled on the event
    /// path and the walk-tick aggregation) so the level-sensitive IRQ stays
    /// routed and de-asserts the tick after firmware writes INT_CLR.
    fn matrix_irq_sources(&self) -> Vec<u32> {
        if self.int_raw & self.int_ena != 0 {
            vec![self.intr_source_id]
        } else {
            Vec::new()
        }
    }

    /// Bootstrap the single module-tick event when a transaction begins clocking
    /// and none is in flight. The delay is relative to the just-synced anchor;
    /// the bus converts it to the absolute deadline `anchor + 1 + delay`, so the
    /// `- 1` here lands the first module tick exactly at `anchor +
    /// cycles_to_next_module_tick` — the cycle the walk would fire it, at any
    /// tick interval (the same anchor calibration the generic SPI engine uses).
    /// `on_event` re-arms every subsequent tick.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.engine_active() && !self.scheduled {
            self.scheduled = true;
            vec![(
                self.cycles_to_next_module_tick().saturating_sub(1),
                I2C_MODULE_TICK_TOKEN,
            )]
        } else {
            Vec::new()
        }
    }

    /// Fire one module tick at its exact cycle, then re-arm the successor while
    /// the engine keeps clocking. Advancing to `sched.now()` via the shared
    /// anchor is delta-based, so a drain that arrives a few cycles late (tick
    /// interval > 1) or early (a stale event after an intervening write
    /// re-anchored the engine) self-corrects — the accumulator only ever
    /// consumes the true elapsed cycles. The reschedule delay carries no `- 1`:
    /// the event path uses `sched.now() + delay` directly (no `+ 1` anchor
    /// offset, unlike the write path).
    fn on_event(
        &mut self,
        _event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        self.scheduled = false;
        let now = sched.now();
        if now > self.last_synced {
            self.advance_engine(now - self.last_synced);
            self.last_synced = now;
        }
        let mut res = crate::sched::EventResult::default();
        if self.engine_active() {
            res.reschedule_delay = Some(self.cycles_to_next_module_tick());
            self.scheduled = true;
        }
        res
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
                        "ink_bytes": oled.ink_bytes(),
                        "lit_pixels": oled.lit_pixels(),
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

// ── Bit engine ───────────────────────────────────────────────────────────────
//
// A command list executes on the wire as a chain of fixed-level segments, each
// a counted number of I²C module-clock ticks. Slaves stay byte-level: the
// engine consults them exactly at byte boundaries and drives the slave-decided
// bits (ACK, read data) onto SDA within bit timing, like real silicon.
impl Esp32c3I2c {
    /// `true` while a transaction is actively clocking on the wire (an END
    /// pause is NOT active — the engine waits for the next `TRANS_START`).
    pub(crate) fn engine_active(&self) -> bool {
        !matches!(self.engine.state, EngineState::Idle | EngineState::Paused)
    }

    /// Get-or-create the shared line-level cell (bus wiring hands the same
    /// `Arc` to the C3 GPIO model).
    pub(crate) fn line_levels_arc(&mut self) -> Arc<I2cLineLevels> {
        if self.lines.is_none() {
            let lines = Arc::new(I2cLineLevels::new());
            lines.set(self.engine.scl, self.engine.sda);
            self.lines = Some(lines);
        }
        self.lines.as_ref().unwrap().clone()
    }

    fn set_lines(&mut self, scl: bool, sda: bool) {
        if self.engine.scl != scl || self.engine.sda != sda {
            self.engine.scl = scl;
            self.engine.sda = sda;
            if let Some(lines) = &self.lines {
                lines.set(scl, sda);
            }
        }
    }

    /// Derive the wire timing from the live clock/timing registers. Reset
    /// values (the datasheet defaults) apply when firmware never programs
    /// them — the derivation has no fallback constants of its own.
    fn timing_from_regs(&self) -> EngineTiming {
        let clk = self.reg_clk_conf;
        let div_num = (clk & 0xFF) as u64 + 1;
        let div_a = ((clk >> 8) & 0x3F) as u64;
        let div_b = ((clk >> 14) & 0x3F) as u64;
        let src_hz = if clk & (1 << 20) != 0 {
            RC_FAST_CLK_HZ
        } else {
            XTAL_CLK_HZ
        };
        // Fractional divider: module clock = src / (div_num + div_b / div_a);
        // div_a == 0 disables the fractional part.
        let (a, b) = if div_a == 0 { (1, 0) } else { (div_a, div_b) };
        EngineTiming {
            num: CPU_CLK_HZ * (div_num * a + b),
            den: src_hz * a,
            low: (self.reg_scl_low_period & 0x1FF) + 1,
            high: (self.reg_scl_high_period & 0x1FF) + ((self.reg_scl_high_period >> 9) & 0x7F) + 1,
            sda_hold: (self.reg_sda_hold & 0x1FF) + 1,
            start_hold: (self.reg_scl_start_hold & 0x1FF) + 1,
            rstart_setup: (self.reg_scl_rstart_setup & 0x1FF) + 1,
            stop_setup: (self.reg_scl_stop_setup & 0x1FF) + 1,
            stop_hold: (self.reg_scl_stop_hold & 0x1FF) + 1,
        }
    }

    /// `CTR.TRANS_START`: snapshot timing and begin executing CMD0..CMD7 on
    /// the wire. Ignored while a transaction is already clocking (silicon's
    /// FSM is busy). Resuming from an END pause continues the held bus.
    fn start_transaction(&mut self) {
        if self.engine_active() {
            return;
        }
        self.engine.timing = self.timing_from_regs();
        self.engine.acc = 0;
        self.engine.cmd_idx = 0;
        // Reset RESP_REC and the TX-FIFO read pointer at the start of a new
        // command-list run.
        self.sr &= !SR_RESP_REC;
        self.tx_pop_count = 0;
        self.advance_command();
        self.chase();
    }

    /// `CTR.FSM_RST`: abort any in-flight transaction and release the lines.
    fn fsm_reset(&mut self) {
        self.engine.state = EngineState::Idle;
        self.engine.ticks_left = 0;
        self.engine.acc = 0;
        self.engine.bus_held = false;
        self.active_slave = None;
        self.expects_addr = true;
        self.set_lines(true, true);
    }

    /// Enter a wire segment: drive the levels and arm its tick counter.
    fn enter(&mut self, state: EngineState, ticks: u32, scl: bool, sda: bool) {
        self.set_lines(scl, sda);
        self.engine.state = state;
        self.engine.ticks_left = ticks;
    }

    /// Advance the bit engine by `cycles` engine cycles, firing module ticks as
    /// the `num/den` accumulator crosses. Shared by BOTH drive paths: the
    /// legacy per-cycle walk ([`Self::tick_elapsed`]) and the scheduler
    /// (`sync_to`/`on_event`). The accumulator is in units of `1/den` engine
    /// cycles; the invariant `acc < num` holds on entry and exit (the `while`
    /// drains it), so the same cycle→module-tick mapping applies whether one
    /// cycle or a whole batch is advanced in a single call — the source of the
    /// walk-vs-scheduler byte-identity.
    fn advance_engine(&mut self, cycles: u64) {
        if !self.engine_active() {
            return;
        }
        self.engine.acc += cycles.saturating_mul(self.engine.timing.den);
        while self.engine.acc >= self.engine.timing.num {
            self.engine.acc -= self.engine.timing.num;
            self.module_tick();
            if !self.engine_active() {
                self.engine.acc = 0;
                break;
            }
        }
    }

    /// Engine cycles until the accumulator next reaches `num` — i.e. until the
    /// next module tick fires, from the current (post-`advance_engine`,
    /// `acc < num`) state. `ceil((num - acc) / den)`, always ≥ 1 while the
    /// engine is active (`num/den` ≥ 4). Undefined (returns 0) when parked; only
    /// called while `engine_active()`.
    fn cycles_to_next_module_tick(&self) -> u64 {
        if !self.engine_active() {
            return 0;
        }
        let num = self.engine.timing.num;
        let den = self.engine.timing.den.max(1);
        // acc < num invariant → num - acc ≥ 1.
        (num - self.engine.acc).div_ceil(den)
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to the
    /// legacy per-cycle walk (`uses_scheduler() == false`). Used by the
    /// walk-on-vs-scheduler differential gate to build the reference config from
    /// the same bus assembly (mirrors `Esp32c3RtcTimer::force_legacy_walk`).
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// One I²C module-clock tick.
    fn module_tick(&mut self) {
        if !self.engine_active() {
            return;
        }
        self.engine.ticks_left = self.engine.ticks_left.saturating_sub(1);
        self.chase();
    }

    /// Run segment transitions until the engine is parked or a non-empty
    /// segment is armed (zero-length segments — e.g. `low - sda_hold == 0` —
    /// chain through within the same module tick).
    fn chase(&mut self) {
        while self.engine_active() && self.engine.ticks_left == 0 {
            self.transition();
        }
    }

    /// Dispatch the command at `cmd_idx`. Commands with wire time arm their
    /// first segment; END / reserved / list-exhaustion park the engine.
    fn advance_command(&mut self) {
        loop {
            if self.engine.cmd_idx >= NUM_CMDS {
                // List ran out without STOP/END: complete the run (legacy
                // behavioral contract preserved).
                self.complete_without_stop();
                return;
            }
            let word = self.cmds[self.engine.cmd_idx];
            let opcode = (word >> 11) & 0x7;
            let byte_num = (word & 0xFF) as usize;
            match opcode {
                OP_RSTART => {
                    // Frame boundary for the (trace-wrapped) previous slave.
                    if let Some(slave_idx) = self.active_slave {
                        self.slaves[slave_idx].start();
                    }
                    self.active_slave = None;
                    self.expects_addr = true;
                    let t = self.engine.timing;
                    if self.engine.bus_held {
                        // Repeated START: release SDA during low, SCL back
                        // high, then SDA falls.
                        self.enter(EngineState::RestartRelease, t.sda_hold, false, true);
                    } else {
                        // Fresh START from bus-idle: SDA falls while SCL high.
                        self.engine.bus_held = true;
                        self.enter(EngineState::StartHold, t.start_hold, true, false);
                    }
                    return;
                }
                OP_WRITE | OP_READ => {
                    if byte_num == 0 {
                        self.cmds[self.engine.cmd_idx] |= CMD_DONE_BIT;
                        self.engine.cmd_idx += 1;
                        continue;
                    }
                    self.engine.cur_is_read = opcode == OP_READ;
                    self.engine.cur_ack_value = word & CMD_ACK_VALUE_BIT != 0;
                    self.engine.bytes_left = byte_num;
                    self.engine.bus_held = true;
                    self.begin_byte();
                    return;
                }
                OP_STOP => {
                    let t = self.engine.timing;
                    let sda = self.engine.sda;
                    self.enter(EngineState::StopLowHold, t.sda_hold, false, sda);
                    return;
                }
                OP_END => {
                    // Pause the list: SCL parked low (bus held), END_DETECT.
                    self.engine.state = EngineState::Paused;
                    self.engine.ticks_left = 0;
                    if self.engine.bus_held {
                        let sda = self.engine.sda;
                        self.set_lines(false, sda);
                    }
                    self.int_raw |= INT_END_DETECT;
                    return;
                }
                _ => {
                    // Reserved opcode — terminate the run.
                    self.complete_without_stop();
                    return;
                }
            }
        }
    }

    /// Start clocking the next byte of the current WRITE/READ command: fetch
    /// the byte at the byte boundary (TX FIFO pop, or the slave's `read()`
    /// answer that the wire then carries bit by bit) and drive SCL low.
    fn begin_byte(&mut self) {
        self.engine.bit_idx = 0;
        self.engine.addr_byte = self.expects_addr && !self.engine.cur_is_read;
        if self.engine.cur_is_read {
            self.engine.cur_byte = match self.active_slave {
                Some(slave_idx) => self.slaves[slave_idx].read(),
                None => 0,
            };
        } else {
            // WRITE: pull the next byte from the TX FIFO. On underrun the real
            // ESP32-C3 controller does NOT invent a 0x00 — it holds SCL low
            // (clock-stretch) and asserts TXFIFO_WM so firmware's ISR refills,
            // then resumes clocking the real byte. A `unwrap_or(0)` here would
            // clock spurious zeros into the slave whenever the FIFO drains
            // faster than firmware refills it mid-burst (e.g. a 128-byte
            // SSD1306 page through the 32-byte FIFO), corrupting the transfer.
            match self.tx_fifo.pop_front() {
                Some(b) => {
                    self.tx_pop_count += 1;
                    self.engine.cur_byte = b;
                }
                None => {
                    // Underrun: signal the watermark and stall until refilled.
                    self.int_raw |= INT_TXFIFO_WM;
                    let sda = self.engine.sda;
                    self.enter(EngineState::TxStall, 1, false, sda);
                    return;
                }
            }
        }
        let t = self.engine.timing;
        let sda = self.engine.sda;
        self.enter(EngineState::BitLowHold, t.sda_hold.min(t.low), false, sda);
    }

    /// Byte-boundary side effects entering the ACK bit; returns the SDA level
    /// driven during the ACK bit (low = ACK).
    fn ack_bit_level(&mut self) -> bool {
        if self.engine.cur_is_read {
            // Received byte lands in the RX FIFO; the master drives the ACK
            // level the command word asked for (COMD.ack_value).
            let mut rx = self.rx_fifo.borrow_mut();
            if rx.len() < FIFO_CAPACITY {
                rx.push_back(self.engine.cur_byte);
            }
            drop(rx);
            return self.engine.cur_ack_value;
        }
        let b = self.engine.cur_byte;
        if self.engine.addr_byte {
            // Address frame: resolve the slave by the wire address bits.
            let addr = b >> 1;
            self.expects_addr = false;
            if let Some(slave_idx) = self.slaves.iter().position(|s| s.address() == addr) {
                // Slave acknowledged its address. Signal START to the selected
                // device — the bus-trace wrapper reconstructs the address
                // frame from this call.
                self.active_slave = Some(slave_idx);
                self.slaves[slave_idx].start();
                self.sr |= SR_RESP_REC;
                return false;
            }
            // ESP-IDF/Arduino can program the address in SLAVE_ADDR and put
            // only payload bytes in TXFIFO. In that shape the first FIFO byte
            // is real data and is delivered to the slave.
            if let Some(slave_idx) = self.find_slave_from_slave_addr_register() {
                self.active_slave = Some(slave_idx);
                self.slaves[slave_idx].start();
                self.sr |= SR_RESP_REC;
                self.slaves[slave_idx].write(b);
                return false;
            }
            self.active_slave = None;
            self.int_raw |= INT_NACK;
            return true;
        }
        // Data byte of a WRITE.
        if let Some(slave_idx) = self.active_slave {
            self.slaves[slave_idx].write(b);
            self.sr |= SR_RESP_REC;
            false
        } else {
            true
        }
    }

    /// The ACK bit finished clocking: advance to the next byte or command.
    fn finish_byte(&mut self) {
        self.engine.bytes_left -= 1;
        if self.engine.bytes_left > 0 {
            self.begin_byte();
            return;
        }
        if self.engine.cur_is_read && self.active_slave.is_some() {
            self.sr |= SR_RESP_REC;
        }
        self.cmds[self.engine.cmd_idx] |= CMD_DONE_BIT;
        self.engine.cmd_idx += 1;
        self.advance_command();
    }

    /// A list that ran out (or hit a reserved opcode) without STOP/END:
    /// complete the run and release the open-drain lines to idle.
    fn complete_without_stop(&mut self) {
        self.active_slave = None;
        self.expects_addr = true;
        self.engine.bus_held = false;
        self.engine.state = EngineState::Idle;
        self.engine.ticks_left = 0;
        self.set_lines(true, true);
        self.int_raw |= INT_TRANS_COMPLETE;
    }

    /// The current segment's tick counter expired: drive the next segment.
    fn transition(&mut self) {
        let t = self.engine.timing;
        match self.engine.state {
            EngineState::Idle | EngineState::Paused => {}
            EngineState::TxStall => {
                // Clock-stretch waiting for a TX-FIFO refill. Retry the byte:
                // `begin_byte` clocks it if one is now available, or re-arms the
                // stall (one retry per module tick) while the FIFO is still dry.
                self.begin_byte();
            }
            EngineState::StartHold => {
                // START condition held — the RSTART command is done; SCL falls
                // when the next command's first segment begins.
                self.cmds[self.engine.cmd_idx] |= CMD_DONE_BIT;
                self.engine.cmd_idx += 1;
                self.advance_command();
            }
            EngineState::RestartRelease => {
                self.enter(EngineState::RestartSetup, t.rstart_setup, true, true);
            }
            EngineState::RestartSetup => {
                // SDA falls while SCL high — the repeated START condition.
                self.enter(EngineState::StartHold, t.start_hold, true, false);
            }
            EngineState::BitLowHold => {
                let sda = if self.engine.bit_idx < 8 {
                    (self.engine.cur_byte >> (7 - self.engine.bit_idx)) & 1 != 0
                } else {
                    // ACK bit: byte-boundary side effects decide the level.
                    self.ack_bit_level()
                };
                let drive = t.low - t.sda_hold.min(t.low);
                self.enter(EngineState::BitLowDrive, drive, false, sda);
            }
            EngineState::BitLowDrive => {
                let sda = self.engine.sda;
                self.enter(EngineState::BitHigh, t.high, true, sda);
            }
            EngineState::BitHigh => {
                self.engine.bit_idx += 1;
                if self.engine.bit_idx <= 8 {
                    let sda = self.engine.sda;
                    self.enter(EngineState::BitLowHold, t.sda_hold.min(t.low), false, sda);
                } else {
                    self.finish_byte();
                }
            }
            EngineState::StopLowHold => {
                let drive = t.low - t.sda_hold.min(t.low);
                self.enter(EngineState::StopLowDrive, drive, false, false);
            }
            EngineState::StopLowDrive => {
                self.enter(EngineState::StopSetup, t.stop_setup, true, false);
            }
            EngineState::StopSetup => {
                // SDA rises while SCL high — the STOP condition.
                self.enter(EngineState::StopHold, t.stop_hold, true, true);
            }
            EngineState::StopHold => {
                if let Some(slave_idx) = self.active_slave {
                    self.slaves[slave_idx].stop();
                }
                self.active_slave = None;
                self.expects_addr = true;
                self.engine.bus_held = false;
                self.cmds[self.engine.cmd_idx] |= CMD_DONE_BIT;
                self.engine.state = EngineState::Idle;
                self.engine.ticks_left = 0;
                self.int_raw |= INT_TRANS_COMPLETE;
            }
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

    /// Clock the bit engine to completion (command lists execute over
    /// simulated cycles now, not synchronously on the TRANS_START write).
    fn run_engine(p: &mut Esp32c3I2c) {
        for _ in 0..1_000_000 {
            if !p.engine_active() {
                return;
            }
            p.tick_elapsed(64);
        }
        panic!("C3 I2C bit engine did not complete");
    }

    /// Kick TRANS_START and clock the engine until it parks (STOP complete,
    /// END pause, or list termination).
    fn start_and_run(p: &mut Esp32c3I2c) {
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        run_engine(p);
    }

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
        assert!(
            !p.legacy_tick_active(),
            "disabled C3 I2C level IRQs must stay out of the legacy tick walk"
        );
        assert!(
            p.legacy_tick_dynamic(),
            "C3 I2C updates tick membership when INT_ST changes"
        );
        p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap();
        assert_eq!(p.read_u32(REG_INT_ST).unwrap(), INT_TRANS_COMPLETE);
        assert!(
            p.legacy_tick_active(),
            "enabled C3 I2C level IRQ must re-enter the legacy tick walk"
        );
        p.write_u32(REG_INT_CLR, INT_TRANS_COMPLETE).unwrap();
        assert!(
            !p.legacy_tick_active(),
            "cleared C3 I2C level IRQ must leave the legacy tick walk"
        );
    }

    #[test]
    fn end_opcode_raises_end_detect_not_trans_complete() {
        let mut p = Esp32c3I2c::new();
        p.write_u32(REG_CMD0, cmd(CMD_END, 0)).unwrap();
        start_and_run(&mut p);
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
        start_and_run(&mut p);
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
            INT_TRANS_COMPLETE
        );
    }

    #[test]
    fn trans_start_auto_clears() {
        let mut p = Esp32c3I2c::new();
        p.write_u32(REG_CMD0, cmd(CMD_END, 0)).unwrap();
        start_and_run(&mut p);
        assert_eq!(p.read_u32(REG_CTR).unwrap() & CTR_TRANS_START_BIT, 0);
    }

    #[test]
    fn one_shot_control_bits_auto_clear() {
        let mut p = Esp32c3I2c::new();
        p.write_u32(REG_CTR, CTR_FSM_RST | CTR_CONF_UPGATE).unwrap();
        assert_eq!(
            p.read_u32(REG_CTR).unwrap() & (CTR_FSM_RST | CTR_CONF_UPGATE),
            0
        );
    }

    #[test]
    fn scl_reset_slave_enable_self_clears() {
        let mut p = Esp32c3I2c::new();
        // Exact value observed in Arduino's i2c_ll_master_clr_bus(): enable
        // plus 9 SCL pulses encoded in SCL_RST_SLV_NUM bits [5:1].
        p.write_u32(REG_SCL_SP_CONF, 0x13).unwrap();
        assert_eq!(
            p.read_u32(REG_SCL_SP_CONF).unwrap(),
            0x12,
            "SCL_RST_SLV_EN must self-clear while preserving pulse count"
        );
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
        start_and_run(&mut p);
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

        start_and_run(&mut p);

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
    fn inspect_ssd1306_framebuffer_reports_ink_metrics() {
        use crate::inspect::InspectOpts;
        use crate::peripherals::components::Ssd1306;

        let mut p = Esp32c3I2c::new();
        p.push_slave(Box::new(Ssd1306::new(0x3C)));

        // Same transaction shape as the C3 OLED firmware:
        // RSTART; WRITE 3 (addr+W, control=0x40, one framebuffer byte); STOP.
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 3)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_DATA, 0x78).unwrap(); // 0x3C << 1, write
        p.write_u32(REG_DATA, 0x40).unwrap(); // SSD1306 data stream
        p.write_u32(REG_DATA, 0xAA).unwrap(); // four lit pixels in byte 0
        start_and_run(&mut p);

        let pi = p.inspect(0x6001_3000, "i2c0", &InspectOpts::default());
        let fb = pi
            .artifacts
            .iter()
            .find(|a| a.kind == "framebuffer")
            .expect("framebuffer artifact present");
        assert_eq!(fb.meta["ink_bytes"], 1);
        assert_eq!(fb.meta["lit_pixels"], 4);
    }

    #[test]
    fn register_addressed_write_delivers_payload_to_ssd1306() {
        use crate::inspect::InspectOpts;
        use crate::peripherals::components::Ssd1306;

        let mut p = Esp32c3I2c::new();
        p.push_slave(Box::new(Ssd1306::new(0x3C)));

        // Arduino-ESP32 / ESP-IDF may program SLAVE_ADDR with addr<<1 and
        // write only the SSD1306 payload bytes to TXFIFO: control byte 0x40,
        // then data 0xAA.
        p.write_u32(REG_SLAVE_ADDR, 0x3C << 1).unwrap();
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_DATA, 0x40).unwrap();
        p.write_u32(REG_DATA, 0xAA).unwrap();
        start_and_run(&mut p);

        assert_eq!(p.read_u32(REG_INT_RAW).unwrap() & INT_NACK, 0);
        let pi = p.inspect(0x6001_3000, "i2c0", &InspectOpts::default());
        let fb = pi
            .artifacts
            .iter()
            .find(|a| a.kind == "framebuffer")
            .expect("framebuffer artifact present");
        assert_eq!(fb.meta["ink_bytes"], 1);
        assert_eq!(fb.meta["lit_pixels"], 4);
    }

    #[test]
    fn end_paused_address_phase_carries_active_slave() {
        use crate::inspect::InspectOpts;
        use crate::peripherals::components::Ssd1306;

        let mut p = Esp32c3I2c::new();
        p.push_slave(Box::new(Ssd1306::new(0x3C)));

        // Arduino-ESP32 splits a write: address phase ends with END_DETECT,
        // then payload bytes are sent by a second command-list run.
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 1)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_END, 0)).unwrap();
        p.write_u32(REG_DATA, 0x78).unwrap();
        start_and_run(&mut p);
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_END_DETECT,
            INT_END_DETECT
        );
        p.write_u32(REG_INT_CLR, INT_END_DETECT).unwrap();

        p.write_u32(REG_CMD0, cmd(CMD_WRITE, 2)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_CMD0 + 8, 0).unwrap();
        p.write_u32(REG_DATA, 0x40).unwrap();
        p.write_u32(REG_DATA, 0xAA).unwrap();
        start_and_run(&mut p);

        assert_eq!(p.read_u32(REG_INT_RAW).unwrap() & INT_NACK, 0);
        let pi = p.inspect(0x6001_3000, "i2c0", &InspectOpts::default());
        let fb = pi
            .artifacts
            .iter()
            .find(|a| a.kind == "framebuffer")
            .expect("framebuffer artifact present");
        assert_eq!(fb.meta["ink_bytes"], 1);
        assert_eq!(fb.meta["lit_pixels"], 4);
    }

    #[test]
    fn write_then_read_calibration_block_round_trip() {
        // Read the 24-byte calibration block starting at 0x88 — exercises a
        // multi-byte READ pulling sequential register-pointer data through the
        // RX FIFO.
        let mut p = Esp32c3I2c::new();
        p.push_slave(Box::new(Bmp280::new(0x76)));

        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 12, cmd(CMD_WRITE, 1)).unwrap();
        p.write_u32(REG_CMD0 + 16, cmd(CMD_READ, 4)).unwrap();
        p.write_u32(REG_CMD0 + 20, cmd(CMD_STOP, 0)).unwrap();

        p.write_u32(REG_DATA, 0xEC).unwrap(); // addr+W
        p.write_u32(REG_DATA, 0x88).unwrap(); // pointer = calib start
        p.write_u32(REG_DATA, 0xED).unwrap(); // addr+R
        start_and_run(&mut p);

        // First four calibration bytes per the Bosch reference block.
        assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x70);
        assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x6B);
        assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x43);
        assert_eq!(p.read_u32(REG_DATA).unwrap(), 0x67);
    }

    /// The headline fidelity contract: TRANS_COMPLETE does NOT assert on the
    /// TRANS_START write. The transaction clocks over simulated cycles at the
    /// rate the (reset-default) clock registers dictate, SR.BUS_BUSY reads 1
    /// on the wire, and completion lands at the exact analytically-derived
    /// cycle.
    #[test]
    fn trans_complete_asserts_at_derived_wire_time_not_instantly() {
        let mut p = Esp32c3I2c::new();
        p.push_slave(Box::new(Bmp280::new(0x76)));
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_DATA, 0xEC).unwrap(); // addr+W
        p.write_u32(REG_DATA, 0xD0).unwrap(); // pointer
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();

        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
            0,
            "TRANS_COMPLETE must not assert instantly on TRANS_START"
        );
        assert!(p.engine_active(), "engine must be clocking the wire");
        assert_eq!(
            p.read_u32(REG_SR).unwrap() & SR_BUS_BUSY,
            SR_BUS_BUSY,
            "SR.BUS_BUSY must read 1 while the transaction is on the wire"
        );

        let mut cycles = 0u64;
        while p.engine_active() {
            p.tick_elapsed(1);
            cycles += 1;
            assert!(cycles < 10_000_000, "engine never completed");
        }
        // Reset-default timing (datasheet reset values, firmware programmed
        // nothing): module tick = 4 engine cycles (XTAL 40 MHz, divider 1, on
        // the 160 MHz cycle base). Wire time in module ticks:
        //   START:  SCL_START_HOLD 8+1                       =  9
        //   bits:   2 bytes x 9 bits x (low 0+1 + high 0+0+1) = 36
        //   STOP:   low 1 + SCL_STOP_SETUP 8+1 + SCL_STOP_HOLD 8+1 = 19
        // total = 64 module ticks = 256 engine cycles.
        assert_eq!(
            cycles, 256,
            "completion time must derive from the registers"
        );
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
            INT_TRANS_COMPLETE
        );
        assert_eq!(p.read_u32(REG_SR).unwrap() & SR_BUS_BUSY, 0);
    }

    /// Timing derivation follows the PROGRAMMED registers: a 100 kHz-style
    /// configuration (as esp-hal would write) stretches the same transaction
    /// accordingly. SCL period = (low + high) module ticks; all counters use
    /// the TRM's `reg + 1` semantics.
    #[test]
    fn scl_timing_follows_programmed_registers() {
        let mut p = Esp32c3I2c::new();
        // 400-tick SCL period at 40 MHz module clock = 100 kHz.
        p.write_u32(REG_SCL_LOW_PERIOD, 199).unwrap(); // low = 200 ticks
        p.write_u32(REG_SCL_HIGH_PERIOD, 180 | (19 << 9)).unwrap(); // high = 200
        p.write_u32(REG_SDA_HOLD, 29).unwrap(); // 30 ticks
        p.write_u32(REG_SCL_START_HOLD, 199).unwrap();
        p.write_u32(REG_SCL_STOP_SETUP, 199).unwrap();
        p.write_u32(REG_SCL_STOP_HOLD, 199).unwrap();

        // One-byte write to an absent slave (NACK still clocks all 9 bits).
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 1)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_DATA, 0xA0).unwrap();
        p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();

        let mut cycles = 0u64;
        while p.engine_active() {
            p.tick_elapsed(1);
            cycles += 1;
            assert!(cycles < 10_000_000, "engine never completed");
        }
        // START 200 + 9 bits x 400 + STOP (200 low + 200 setup + 200 hold)
        // = 4400 module ticks x 4 engine cycles = 17600 cycles.
        assert_eq!(cycles, 17_600);
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_NACK,
            INT_NACK,
            "absent slave must NACK"
        );
    }

    #[test]
    fn set_bus_trace_records_transactions_for_attached_slaves() {
        use crate::peripherals::components::Bmp280;

        let log = crate::bus::bus_trace::new_log();
        let mut p = Esp32c3I2c::new();
        // The bus choke point wraps before push; emulate it here.
        p.push_slave(crate::bus::bus_trace::wrap_i2c(
            "i2c0",
            &log,
            Box::new(Bmp280::new(0x76)),
        ));

        // Same canonical pointer-write transaction as
        // write_read_drives_attached_bmp280: RSTART; WRITE 2; STOP.
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_DATA, 0xEC).unwrap(); // addr+W
        p.write_u32(REG_DATA, 0xD0).unwrap(); // pointer
        start_and_run(&mut p);

        let events = log.snapshot();
        assert!(
            !events.is_empty(),
            "tracing wrapper must record I2C traffic on the C3 controller"
        );
        assert!(events.iter().all(|e| e.bus == "i2c0"));
        // The controller must signal START at address match so the trace
        // carries a decodable address frame, not just raw data bytes.
        assert!(
            events.iter().any(|e| matches!(
                &e.payload,
                crate::bus::bus_trace::BusPayload::I2c {
                    kind: crate::bus::bus_trace::I2cSym::AddrWrite,
                    ..
                }
            )),
            "trace must contain an address frame for transaction decode"
        );
    }

    /// A realistic SSD1306 pixel-data burst: four full GDDRAM pages
    /// (128×4 = 512 data bytes) streamed the way a display driver does — each
    /// transfer is far larger than the 32-byte TX FIFO, so the FIFO underruns
    /// and must be refilled mid-WRITE (the watermark / OP_END refill the IDF and
    /// Arduino I²C drivers rely on).
    ///
    /// The real ESP32-C3 controller holds SCL low (clock-stretch) on a TX-FIFO
    /// underrun and resumes when firmware refills; it NEVER invents a 0x00. A
    /// model that pops a spurious 0x00 on underrun (`pop_front().unwrap_or(0)`)
    /// clocks bogus bytes into the panel — the extra pixels land in GDDRAM as
    /// zeros (and shift every real byte that follows), so the OLED reads back an
    /// all-but-blank framebuffer even though the CPU/serial/LED are healthy.
    ///
    /// Every existing OLED test only ever sends a 2–3 byte prologue that fits in
    /// one FIFO load, so this multi-chunk burst is the first coverage of the
    /// underrun-refill path.
    #[test]
    fn multi_chunk_pixel_burst_delivers_every_byte_to_ssd1306() {
        use crate::peripherals::components::Ssd1306;

        const ADDR7: u8 = 0x3C;
        const ADDR_W: u32 = (ADDR7 as u32) << 1; // 0x78, R/W = write

        let mut p = Esp32c3I2c::new();
        p.push_slave(Box::new(Ssd1306::new(ADDR7)));

        // ── Init: a short command transaction that fits in ONE FIFO load (the
        //    prologue that already works in the field). Horizontal addressing,
        //    full 128×64 window, display on. Control byte 0x00 = command stream.
        let init = [0x20u8, 0x00, 0x21, 0x00, 0x7F, 0x22, 0x00, 0x07, 0xAF];
        p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
        p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, (2 + init.len()) as u8))
            .unwrap();
        p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
        p.write_u32(REG_DATA, ADDR_W).unwrap();
        p.write_u32(REG_DATA, 0x00).unwrap(); // command-stream control byte
        for b in init {
            p.write_u32(REG_DATA, b as u32).unwrap();
        }
        start_and_run(&mut p);
        assert_eq!(
            p.read_u32(REG_INT_RAW).unwrap() & INT_NACK,
            0,
            "init prologue must ACK"
        );

        // ── Pixel data: four full pages. Distinct nonzero pattern so a dropped
        //    byte (read back as 0x00) or a shifted byte is caught at its exact
        //    GDDRAM position.
        const N_PAGES: usize = 4;
        const DATA_LEN: usize = 128 * N_PAGES; // 512 bytes → N = 4
        let pattern: Vec<u8> = (0..DATA_LEN).map(|i| ((i % 251) + 1) as u8).collect();

        // Stream one page (128 bytes) per transaction, exactly how
        // Adafruit_SSD1306 pushes the framebuffer with the 0x40 data control
        // byte. Each WRITE command is addr(1) + control(1) + 128 data = 130
        // bytes — over 4× the 32-byte TX FIFO — so it underruns and is refilled
        // mid-command.
        for page in 0..N_PAGES {
            let page_data = &pattern[page * 128..(page + 1) * 128];
            let mut payload = Vec::with_capacity(2 + 128);
            payload.push(ADDR_W as u8);
            payload.push(0x40); // SSD1306 data-stream control byte
            payload.extend_from_slice(page_data);

            p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
            p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, payload.len() as u8))
                .unwrap();
            p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();

            // Preload the TX FIFO to capacity, then kick the transaction.
            let mut next = 0usize;
            while next < payload.len() && p.tx_fifo.len() < FIFO_CAPACITY {
                p.write_u32(REG_DATA, payload[next] as u32).unwrap();
                next += 1;
            }
            p.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();

            // Clock the engine, refilling the TX FIFO only once it has actually
            // drained — modelling an ISR that services the watermark / empty
            // interrupt with real latency. A faithful controller holds SCL low
            // until the refill lands; a controller that pops 0x00 on underrun
            // has already clocked bogus bytes into the panel by then.
            let mut guard = 0u64;
            while p.engine_active() {
                if p.tx_fifo.is_empty() && next < payload.len() {
                    while next < payload.len() && p.tx_fifo.len() < FIFO_CAPACITY {
                        p.write_u32(REG_DATA, payload[next] as u32).unwrap();
                        next += 1;
                    }
                }
                p.tick_elapsed(512);
                guard += 1;
                assert!(guard < 1_000_000, "engine never completed page {page}");
            }
            assert_eq!(
                next,
                payload.len(),
                "every byte of page {page} must have been pulled from the FIFO, \
                 not fabricated as 0x00 on underrun"
            );
            assert_eq!(
                p.read_u32(REG_INT_RAW).unwrap() & INT_NACK,
                0,
                "page {page} data burst must ACK"
            );
        }

        // ── Read back GDDRAM: every pixel byte must equal what was written, with
        //    no spurious 0x00 from a FIFO underrun and no positional shift.
        let oled = p
            .attached_slaves()
            .iter()
            .find_map(|d| d.as_any().and_then(|a| a.downcast_ref::<Ssd1306>()))
            .expect("SSD1306 attached");
        let fb = oled.framebuffer();
        assert_eq!(
            &fb[..DATA_LEN],
            &pattern[..],
            "multi-chunk pixel burst must land byte-exact in GDDRAM (a 0x00 or a \
             shift here is the black-OLED underrun bug)"
        );
        assert_eq!(
            oled.ink_bytes(),
            DATA_LEN,
            "all {DATA_LEN} written pixel bytes are nonzero and must be lit"
        );
    }
}
