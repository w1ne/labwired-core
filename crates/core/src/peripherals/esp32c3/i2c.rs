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
use std::sync::{Arc, Mutex};

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

/// One physical C3 GPIO-matrix route for an I²C device. The system manifest
/// deliberately names signals rather than target registers; this is the C3
/// lowering of the target-neutral `route: { sda: GPIOx, scl: GPIOy }` shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct C3I2cPadRoute {
    pub(crate) sda: u8,
    pub(crate) scl: u8,
}

impl C3I2cPadRoute {
    pub(crate) fn from_manifest_route(
        route: &std::collections::BTreeMap<String, String>,
    ) -> anyhow::Result<Self> {
        for signal in route.keys() {
            if signal != "sda" && signal != "scl" {
                anyhow::bail!(
                    "ESP32-C3 I2C route supports only route.sda and route.scl; found route.{signal}"
                );
            }
        }

        fn parse_pad(
            route: &std::collections::BTreeMap<String, String>,
            signal: &str,
        ) -> anyhow::Result<u8> {
            let label = route.get(signal).ok_or_else(|| {
                anyhow::anyhow!(
                    "ESP32-C3 I2C external device requires both route.sda and route.scl"
                )
            })?;
            let pin = label.strip_prefix("GPIO").ok_or_else(|| {
                anyhow::anyhow!(
                    "ESP32-C3 I2C route.{signal} must name a C3 GPIO pad such as GPIO4, got '{label}'"
                )
            })?;
            let pin: u8 = pin.parse().map_err(|_| {
                anyhow::anyhow!(
                    "ESP32-C3 I2C route.{signal} must name a C3 GPIO pad such as GPIO4, got '{label}'"
                )
            })?;
            if pin >= 26 {
                anyhow::bail!(
                    "ESP32-C3 I2C route.{signal}='{label}' is not a supported GPIO-matrix pad"
                );
            }
            Ok(pin)
        }

        let sda = parse_pad(route, "sda")?;
        let scl = parse_pad(route, "scl")?;
        if sda == scl {
            anyhow::bail!(
                "ESP32-C3 I2C route.sda and route.scl must use distinct pads (both were GPIO{sda})"
            );
        }
        Ok(Self { sda, scl })
    }
}

/// Live C3 GPIO-matrix wiring observed by the I²C controller. GPIO owns the
/// registers and refreshes this cell on every relevant write; the controller
/// consults it at address resolution, so a slave responds only when firmware
/// has configured the same physical SDA/SCL pair declared in the manifest.
#[derive(Debug, Default)]
pub(crate) struct C3I2cMatrixRouteState {
    sda_output_mask: u32,
    scl_output_mask: u32,
    sda_input: Option<u8>,
    scl_input: Option<u8>,
}

impl C3I2cMatrixRouteState {
    pub(crate) fn set(
        &mut self,
        sda_output_mask: u32,
        scl_output_mask: u32,
        sda_input: Option<u8>,
        scl_input: Option<u8>,
    ) {
        self.sda_output_mask = sda_output_mask;
        self.scl_output_mask = scl_output_mask;
        self.sda_input = sda_input;
        self.scl_input = scl_input;
    }

    fn activates(&self, route: C3I2cPadRoute) -> bool {
        let sda_mask = 1u32 << route.sda;
        let scl_mask = 1u32 << route.scl;
        self.sda_output_mask & sda_mask != 0
            && self.scl_output_mask & scl_mask != 0
            && self.sda_input == Some(route.sda)
            && self.scl_input == Some(route.scl)
    }
}

pub(crate) type C3I2cMatrixRoute = Arc<Mutex<C3I2cMatrixRouteState>>;

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
/// First arming-token value. The token carried by a wake is the `arm_seq` that
/// armed it; see [`Esp32c3I2c::re_anchor`].
const I2C_FIRST_ARM_SEQ: u32 = 1;

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
    /// Physical pad route for each slave, parallel to `slaves`. `None` is an
    /// intentional low-level/direct-test attachment; manifest-backed C3
    /// external devices always carry `Some` and are gated by the GPIO matrix.
    slave_routes: Vec<Option<C3I2cPadRoute>>,
    /// Shared live GPIO-matrix state, installed by `SystemBus` once both C3
    /// GPIO and I²C peripherals exist. A declared route cannot answer until
    /// firmware programs matching input *and* output matrix entries.
    matrix_route: Option<C3I2cMatrixRoute>,
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
    /// ARMING TOKEN (layer 3 of the scheduler's cancellation contract). Bumped
    /// by [`Self::re_anchor`] whenever a register write moves the engine out
    /// from under an in-flight wake; carried as the event token so a superseded
    /// wake is discarded on arrival instead of driving the engine.
    ///
    /// The singleton `scheduled` flag alone is not enough once the wake cadence
    /// is a whole wire segment: `CTR.FSM_RST` parks the engine mid-transaction
    /// while a wake stays queued (the scheduler has no cancel API by design),
    /// and the following `CTR.TRANS_START` then sees `scheduled == true`, arms
    /// nothing, and lets the stale wake clock the fresh transaction hundreds of
    /// cycles late. Gated by `rearm_after_fsm_rst_matches_the_per_cycle_walk`,
    /// which fails without this.
    arm_seq: u32,

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
            slave_routes: Vec::new(),
            matrix_route: None,
            intr_source_id: I2C0_INTR_SOURCE_ID,
            active_slave: None,
            expects_addr: true,
            engine: BitEngine::new(),
            lines: None,
            clock: None,
            last_synced: 0,
            scheduled: false,
            arm_seq: I2C_FIRST_ARM_SEQ,

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

    /// Raw, un-routed slave attachment for direct unit fixtures. Manifest-backed
    /// C3 devices must use [`Self::push_slave_with_route`] so the GPIO matrix is
    /// part of their electrical contract.
    #[cfg(test)]
    pub(crate) fn push_slave(&mut self, slave: Box<dyn I2cDevice>) {
        self.slaves.push(slave);
        self.slave_routes.push(None);
    }

    /// Attach a manifest-backed slave with the physical C3 pads it is wired
    /// to. Unlike [`Self::push_slave`], this device will acknowledge only
    /// while GPIO's live matrix state matches this exact route.
    pub(crate) fn push_slave_with_route(
        &mut self,
        slave: Box<dyn I2cDevice>,
        route: C3I2cPadRoute,
    ) {
        self.slaves.push(slave);
        self.slave_routes.push(Some(route));
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

    /// Mutable counterpart of [`Self::attached_slaves`], for callers that must
    /// drive a slave's device protocol (register reads) rather than just
    /// inspect it. Production stimulus does NOT use this — it goes through
    /// [`crate::Peripheral::for_each_attached_sim_input`]; this exists so tests
    /// can reach a device by a path independent of the walk they are gating.
    pub fn attached_slaves_mut(&mut self) -> &mut [Box<dyn I2cDevice>] {
        &mut self.slaves
    }

    /// Share GPIO's live matrix state with this controller after both C3
    /// peripherals have been constructed on a system bus.
    pub(crate) fn set_matrix_route_state(&mut self, route: C3I2cMatrixRoute) {
        self.matrix_route = Some(route);
    }

    fn slave_route_active(&self, index: usize) -> bool {
        let Some(Some(route)) = self.slave_routes.get(index).copied() else {
            return true;
        };
        self.matrix_route
            .as_ref()
            .map(|state| {
                state
                    .lock()
                    .expect("ESP32-C3 I2C matrix route poisoned")
                    .activates(route)
            })
            .unwrap_or(false)
    }

    /// Resolve the slave that answers to `address` on an active pad route, and
    /// tell it which address the master selected.
    ///
    /// Resolution goes through `claims_address`, not `address()`: a bus switch
    /// (TCA9548A) answers for every device behind its enabled channels, and a
    /// flat `address()` comparison is first-match — four identical sensors on
    /// four channels would collapse onto one. The GPIO-matrix route gate is
    /// unchanged and still applies to the switch itself, which is the only
    /// thing physically wired to the pads.
    fn find_slave_by_address(&mut self, address: u8) -> Option<usize> {
        let idx = self.slaves.iter().enumerate().find_map(|(idx, slave)| {
            (self.slave_route_active(idx) && slave.claims_address(address)).then_some(idx)
        })?;
        self.slaves[idx].select_address(address);
        Some(idx)
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
            _ => {
                crate::census_reg!("esp32c3.i2c:Esp32c3I2c", offset, "read");
                0
            }
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
            _ => {
                crate::census_reg!("esp32c3.i2c:Esp32c3I2c", offset, "write");
            } // Accept-and-ignore (unmapped offsets)
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
    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if self.int_raw & self.int_ena != 0 {
            out.push(self.intr_source_id);
        }
    }

    /// Bootstrap the single segment event when a transaction begins clocking and
    /// none is in flight. The delay is relative to the just-synced anchor; the
    /// bus converts it to the absolute deadline `anchor + 1 + delay`, so the
    /// `- 1` here lands the wake exactly at `anchor +
    /// cycles_to_next_transition` — the cycle the walk would run that segment's
    /// end, at any tick interval (the same anchor calibration the generic SPI
    /// engine uses). `on_event` re-arms each successor.
    ///
    /// Carries `arm_seq` as the token so a wake armed for a superseded engine
    /// state is discarded on arrival rather than clocking this transaction.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.engine_active() && !self.scheduled {
            self.scheduled = true;
            vec![(
                self.cycles_to_next_transition().saturating_sub(1),
                self.arm_seq,
            )]
        } else {
            Vec::new()
        }
    }

    /// Run the wire segment that ends at this cycle, then arm the next one.
    /// Advancing to `sched.now()` via the shared anchor is delta-based, so a
    /// drain that arrives a few cycles late (tick interval > 1) or early (an
    /// intervening write re-anchored the engine) self-corrects — the
    /// accumulator only ever consumes the true elapsed cycles. The reschedule
    /// delay carries no `- 1`: the event path uses `sched.now() + delay`
    /// directly (no `+ 1` anchor offset, unlike the write path).
    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        // Superseded chain: a register write re-anchored the engine after this
        // wake was armed (see `arm_seq`). Drive nothing and re-arm nothing — the
        // LIVE chain owns `scheduled`, so this must not clear it. Letting a
        // stale wake through here is exactly the divergence that
        // `rearm_after_fsm_rst_matches_the_per_cycle_walk` pins.
        if event_token != self.arm_seq {
            return crate::sched::EventResult::default();
        }
        self.scheduled = false;
        let now = sched.now();
        if now > self.last_synced {
            self.advance_engine(now - self.last_synced);
            self.last_synced = now;
        }
        let mut res = crate::sched::EventResult::default();
        if self.engine_active() {
            res.reschedule_delay = Some(self.cycles_to_next_transition());
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

    /// Stimulus reaches every attached slave regardless of `slave_routes`: a
    /// pad route decides whether the slave ACKs a *bus transaction*, not
    /// whether an agent may pose the physical world the sensor measures. A
    /// mis-routed device should read its driven value and still go unanswered
    /// on the wire — that is the honest failure, and route-filtering here would
    /// hide it behind a `NoDevice` error instead.
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

    /// Custom inspection: generic register decode plus a `framebuffer` artifact
    /// for any attached panel. Same pattern as the generic `I2c` controller —
    /// the C3 command-list controller walks its own slaves so the leo
    /// air-quality OLED surfaces through the universal inspect interface.
    ///
    /// The artifact's CONTENTS come from the device model's own
    /// [`crate::peripherals::i2c::I2cDevice::artifacts`], not from here. This used to carry its own copy of the SSD1306 arm, one
    /// of three such copies; a panel that reported one thing on the C3 and
    /// another on STM32 was a matter of which copy someone remembered to edit.
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
            pi.artifacts
                .extend(dev.artifacts(&format!("i2c@0x{:02x}", addr), opts));
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
        // The engine is parked here, but a wake armed before it parked may still
        // be queued — `advance_engine` can park it from a write-path `sync_to`,
        // which never runs `on_event` to clear `scheduled`. Re-anchor so this
        // fresh run arms its own wake.
        self.re_anchor();
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

    /// Invalidate any wake armed for the engine state that existed BEFORE the
    /// caller changed it, and allow a fresh one to be armed.
    ///
    /// Called from every point where a register write moves the engine under an
    /// in-flight wake: [`Self::fsm_reset`] (parks it mid-transaction) and
    /// [`Self::start_transaction`] (re-snapshots `timing` and zeroes `acc`).
    /// Bumping `arm_seq` makes the queued wake dead on arrival; clearing
    /// `scheduled` lets `take_scheduled_events` arm a correct successor.
    ///
    /// Cheap and idempotent, so it is applied at both sites rather than only
    /// the one currently known to be reachable — the cost of missing a site is
    /// a silent timing divergence, and the cost of an extra call is one
    /// wrapping add.
    fn re_anchor(&mut self) {
        self.arm_seq = self.arm_seq.wrapping_add(1);
        self.scheduled = false;
    }

    /// `CTR.FSM_RST`: abort any in-flight transaction and release the lines.
    fn fsm_reset(&mut self) {
        self.re_anchor();
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
    /// Engine cycles until the next OBSERVABLE moment: the module tick on which
    /// the current wire segment ends and [`Self::transition`] runs.
    ///
    /// This is what the engine schedules its wake for, and it is why the wake
    /// cadence is a segment rather than a module tick. A module tick with
    /// `ticks_left > 1` does exactly one thing — decrement that counter.
    /// `chase()` only fires at zero, and every level change on the wire goes
    /// through `enter()` inside `transition()`, so SCL/SDA, the attached
    /// slaves, and `int_raw` are all constant until then. `advance_engine` is
    /// delta-based and drains the accumulator in a loop, so consuming N ticks
    /// in one call lands on exactly the state N single-tick calls would.
    /// Anything that reads the engine early (an MMIO access) goes through
    /// `sync_to` first and gets the same catch-up.
    ///
    /// At 100 kHz with a 40 MHz module clock a segment is ~200 module ticks, so
    /// this is ~200x fewer wakes than the per-module-tick cadence — the
    /// difference between a mean CPU batch of 19 and 124 instructions on the
    /// shipped OLED lab. Segments that genuinely need per-tick attention ask for
    /// it by construction: `TxStall` (the TX-FIFO clock-stretch retry) arms
    /// `ticks = 1`, so this returns the next module tick for it, unchanged.
    ///
    /// SAFETY CONTRACT: correct only while no register write re-anchors the
    /// engine under an in-flight wake. That is what [`Self::re_anchor`] and the
    /// `arm_seq` token exist for — without them this cadence is a real timing
    /// divergence, proven by `rearm_after_fsm_rst_matches_the_per_cycle_walk`.
    fn cycles_to_next_transition(&self) -> u64 {
        if !self.engine_active() {
            return 0;
        }
        let num = self.engine.timing.num;
        let den = self.engine.timing.den.max(1);
        // `chase()` guarantees `ticks_left >= 1` while active; `max(1)` is belt
        // and braces so a zero can never collapse this to "wake immediately,
        // forever".
        let ticks = u64::from(self.engine.ticks_left.max(1));
        // acc < num invariant → the numerator cannot underflow.
        (num.saturating_mul(ticks) - self.engine.acc).div_ceil(den)
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
            if let Some(slave_idx) = self.find_slave_by_address(addr) {
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

    #[test]
    fn manifest_route_rejects_incomplete_or_invalid_c3_pads() {
        let incomplete =
            std::collections::BTreeMap::from([("sda".to_string(), "GPIO4".to_string())]);
        assert!(C3I2cPadRoute::from_manifest_route(&incomplete)
            .unwrap_err()
            .to_string()
            .contains("route.sda and route.scl"));

        let non_c3 = std::collections::BTreeMap::from([
            ("sda".to_string(), "PB7".to_string()),
            ("scl".to_string(), "PB6".to_string()),
        ]);
        assert!(C3I2cPadRoute::from_manifest_route(&non_c3)
            .unwrap_err()
            .to_string()
            .contains("route.sda"));

        let duplicate = std::collections::BTreeMap::from([
            ("sda".to_string(), "GPIO4".to_string()),
            ("scl".to_string(), "GPIO4".to_string()),
        ]);
        assert!(C3I2cPadRoute::from_manifest_route(&duplicate)
            .unwrap_err()
            .to_string()
            .contains("distinct pads"));

        let wrong_transport_signal = std::collections::BTreeMap::from([
            ("sda".to_string(), "GPIO4".to_string()),
            ("scl".to_string(), "GPIO5".to_string()),
            ("mosi".to_string(), "GPIO6".to_string()),
        ]);
        assert!(C3I2cPadRoute::from_manifest_route(&wrong_transport_signal)
            .unwrap_err()
            .to_string()
            .contains("route.mosi"));
    }

    const REG_CMD1_OFFSET: u64 = REG_CMD0 + 4;

    // ── Wake-cadence safety: re-anchor while a wake is in flight ─────────────
    //
    // The scheduler-driven engine schedules its own successor wake. How FAR
    // ahead it may schedule is bounded by one thing: a register write can
    // re-anchor the engine underneath an in-flight wake, and the engine must
    // not then be driven by a wake computed for the state it had BEFORE the
    // write.
    //
    // `CTR.FSM_RST` is the case that reaches this. It parks the engine
    // mid-transaction (state -> Idle, ticks_left/acc -> 0) while `scheduled`
    // stays true and a wake stays queued (the scheduler has no cancel API by
    // design). A following `CTR.TRANS_START` then finds `scheduled == true`
    // and arms nothing, so the fresh transaction is driven by the STALE wake.
    // `CTR.TRANS_START` alone cannot reach it — `start_transaction` early-returns
    // while `engine_active()`.
    //
    // This is invisible at a one-module-tick cadence (the stale wake is <= 4
    // cycles out, so it fires, clears `scheduled`, and TRANS_START re-arms
    // cleanly). It becomes a real timing divergence the moment the cadence
    // widens toward the next segment transition. Arduino's
    // `i2c_ll_master_clr_bus()` writes FSM_RST, so this is a path real firmware
    // takes — see `scl_reset_slave_enable_self_clears`.
    //
    // The reference is the LEGACY PER-CYCLE WALK, which has no wakes at all and
    // therefore cannot be wrong about them.

    /// Drive a scheduler-mode engine cycle-by-cycle through a real
    /// `EventScheduler`, mirroring exactly what `SystemBus`/`Machine` do:
    /// publish the clock, arm from `take_scheduled_events` at
    /// `now + 1 + delay`, deliver due events, and honour `reschedule_delay`.
    // Gated on `event-scheduler`: `uses_scheduler()` is
    // `cfg!(feature = "event-scheduler") && clock.is_some()`, so with the
    // feature off there is no scheduler path to compare the walk against and
    // `SchedDriver::new` would trip its own assert. `core-integrity` runs
    // these via `cargo test --release -p labwired-core --features
    // event-scheduler --lib`.
    #[cfg(feature = "event-scheduler")]
    struct SchedDriver {
        dev: Esp32c3I2c,
        sched: crate::sched::EventScheduler,
        clock: CycleClock,
        bus: crate::bus::SystemBus,
        now: u64,
    }

    #[cfg(feature = "event-scheduler")]
    impl SchedDriver {
        fn new() -> Self {
            let mut dev = Esp32c3I2c::new();
            let clock = CycleClock::default();
            clock.publish(0);
            dev.attach_cycle_clock(clock.clone());
            assert!(
                dev.uses_scheduler(),
                "driver must exercise the SCHEDULER path, not the walk"
            );
            Self {
                dev,
                sched: crate::sched::EventScheduler::new(),
                clock,
                bus: crate::bus::SystemBus::new(),
                now: 0,
            }
        }

        fn arm_pending(&mut self) {
            for (delay, token) in self.dev.take_scheduled_events() {
                self.sched.schedule(self.now + 1 + delay, 0, token);
            }
        }

        fn write(&mut self, offset: u64, value: u32) {
            self.dev.sync_to(self.now);
            self.dev.write_u32(offset, value).unwrap();
            self.arm_pending();
        }

        /// Advance one cycle and deliver anything due at the new cycle.
        fn step(&mut self) {
            self.now += 1;
            self.clock.publish(self.now);
            self.sched.advance_to(self.now);
            let mut due = Vec::new();
            self.sched.drain_due_into(&mut due);
            for ev in due {
                let res = self
                    .dev
                    .on_event(ev.event_token, &mut self.sched, &mut self.bus);
                if let Some(delay) = res.reschedule_delay {
                    self.sched
                        .schedule(self.sched.now() + delay, 0, ev.event_token);
                }
            }
            self.arm_pending();
        }
    }

    /// EVERY attachable I²C device, scheduler path vs the per-cycle walk.
    ///
    /// The wake cadence is a property of the CONTROLLER, but what rides on it is
    /// the attached device: a slave decides ACK/NACK and drives read bytes onto
    /// SDA within bit timing, so a mis-timed wake shows up as a different
    /// waveform, a different RX FIFO, or a different interrupt status — and
    /// which of those it shows up as depends on the device.
    ///
    /// The two shipped OLED labs pin only SSD1306, and only its WRITE path.
    /// This walks the whole `build_i2c_device` roster through a write-then-
    /// repeated-START-read transaction, which is the shape every real sensor
    /// driver uses and which the OLED never exercises. Reference is the LEGACY
    /// PER-CYCLE WALK, which has no wakes and so cannot be wrong about them.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn every_attached_i2c_device_matches_the_per_cycle_walk() {
        use crate::peripherals::components::i2c_factory::build_i2c_device;

        // The full `build_i2c_device` roster reachable from a manifest, with the
        // address each answers on. `shm_i2c` is excluded: it is backed by a
        // shared-memory file, not a modelled device.
        const DEVICES: &[(&str, u8)] = &[
            ("tmp102", 0x48),
            ("tmp117", 0x48),
            ("pca9685", 0x40),
            ("mpu6050", 0x68),
            ("bmi270", 0x68),
            ("fxos8700", 0x1E),
            ("bme280", 0x76),
            ("bmp280", 0x76),
            ("aht20", 0x38),
            ("ina219", 0x40),
            ("max30102", 0x57),
            ("cap1188", 0x29),
            ("drv2605", 0x5A),
            ("mlx90640", 0x33),
        ];

        // One SCL period at this timing is ~1600 CPU cycles, and the sequence is
        // ~30 bits INCLUDING a restart from scratch after the FSM_RST, so the
        // budget must cover roughly two full transactions. Sized empirically
        // until the RX FIFO actually fills — see the non-vacuity assert below.
        const PRE_RST: u64 = 1_500;
        const TOTAL: u64 = 400_000;

        let mut covered = 0usize;
        let mut devices_with_rx = 0usize;
        for (name, addr) in DEVICES {
            let cfg = std::collections::HashMap::from([(
                "i2c_address".to_string(),
                serde_yaml::Value::from(*addr as u64),
            )]);
            let Some(_probe) = build_i2c_device(name, &cfg) else {
                panic!(
                    "build_i2c_device({name}) returned None — the roster in this \
                     test has drifted from the factory"
                );
            };
            covered += 1;

            // addr+W, register pointer, addr+R — the classic sensor read prologue.
            let tx_bytes: [u32; 3] = [u32::from(*addr) << 1, 0x00, (u32::from(*addr) << 1) | 1];
            // Write a register pointer, then repeated-START read 4 bytes back.
            let program = |write: &mut dyn FnMut(u64, u32)| {
                write(REG_SCL_LOW_PERIOD, 200);
                write(REG_SCL_HIGH_PERIOD, 200);
                write(REG_SDA_HOLD, 40);
                write(REG_SDA_SAMPLE, 40);
                write(REG_SCL_RSTART_SETUP, 200);
                write(REG_SCL_STOP_SETUP, 200);
                write(REG_SCL_STOP_HOLD, 200);
                write(REG_CMD0, cmd(CMD_RSTART, 0));
                write(REG_CMD1_OFFSET, cmd(CMD_WRITE, 2));
                write(REG_CMD1_OFFSET + 4, cmd(CMD_RSTART, 0));
                write(REG_CMD1_OFFSET + 8, cmd(CMD_WRITE, 1));
                write(REG_CMD1_OFFSET + 12, cmd(CMD_READ, 4));
                write(REG_CMD1_OFFSET + 16, cmd(CMD_STOP, 0));
                for b in tx_bytes {
                    write(REG_DATA, b);
                }
            };

            // ── reference: per-cycle walk ──
            let mut walk = Esp32c3I2c::new();
            walk.push_slave(build_i2c_device(name, &cfg).unwrap());
            assert!(!walk.uses_scheduler());
            let walk_lines = walk.line_levels_arc();
            {
                let mut w = |o: u64, v: u32| {
                    walk.write_u32(o, v).unwrap();
                };
                program(&mut w);
            }
            walk.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
            let mut walk_wave = Vec::with_capacity(TOTAL as usize);
            for c in 1..=TOTAL {
                walk.tick_elapsed(1);
                if c == PRE_RST {
                    // What a real driver does on a bus reset: clear the FSM,
                    // re-prime the TX FIFO (FSM_RST drains it), restart.
                    walk.write_u32(REG_CTR, CTR_FSM_RST).unwrap();
                    walk.write_u32(REG_INT_CLR, u32::MAX).unwrap();
                    for b in tx_bytes {
                        walk.write_u32(REG_DATA, b).unwrap();
                    }
                    walk.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
                }
                walk_wave.push((walk_lines.scl(), walk_lines.sda()));
            }

            // ── under test: scheduler-driven ──
            let mut sd = SchedDriver::new();
            sd.dev.push_slave(build_i2c_device(name, &cfg).unwrap());
            let sched_lines = sd.dev.line_levels_arc();
            {
                let mut pending: Vec<(u64, u32)> = Vec::new();
                let mut w = |o: u64, v: u32| pending.push((o, v));
                program(&mut w);
                for (o, v) in pending {
                    sd.write(o, v);
                }
            }
            sd.write(REG_CTR, CTR_TRANS_START_BIT);
            let mut sched_wave = Vec::with_capacity(TOTAL as usize);
            for c in 1..=TOTAL {
                sd.step();
                if c == PRE_RST {
                    sd.write(REG_CTR, CTR_FSM_RST);
                    sd.write(REG_INT_CLR, u32::MAX);
                    for b in tx_bytes {
                        sd.write(REG_DATA, b);
                    }
                    sd.write(REG_CTR, CTR_TRANS_START_BIT);
                }
                sched_wave.push((sched_lines.scl(), sched_lines.sda()));
            }

            // A device that never got clocked proves nothing about wakes.
            let edges = walk_wave.windows(2).filter(|w| w[0] != w[1]).count();
            assert!(
                edges > 8,
                "{name}: reference waveform has only {edges} edges — the \
                 transaction never clocked, so this row is vacuous"
            );

            if let Some(c) = (0..TOTAL as usize).find(|&i| walk_wave[i] != sched_wave[i]) {
                panic!(
                    "{name} @ {addr:#04x}: scheduler waveform diverges from the \
                     per-cycle walk at cycle {}: walk={:?} sched={:?}",
                    c + 1,
                    walk_wave[c],
                    sched_wave[c]
                );
            }

            // The bytes the controller actually captured must match too — the
            // waveform is what the pads saw, the FIFO is what firmware reads.
            assert_eq!(
                walk.read_u32(REG_SR).unwrap(),
                sd.dev.read_u32(REG_SR).unwrap(),
                "{name}: SR diverges"
            );
            assert_eq!(walk.int_raw, sd.dev.int_raw, "{name}: INT_RAW diverges");
            let walk_rx: Vec<u8> = walk.rx_fifo.borrow().iter().copied().collect();
            let sched_rx: Vec<u8> = sd.dev.rx_fifo.borrow().iter().copied().collect();
            assert_eq!(walk_rx, sched_rx, "{name}: RX FIFO diverges");
            if !walk_rx.is_empty() {
                devices_with_rx += 1;
            }
        }

        assert_eq!(
            covered,
            DEVICES.len(),
            "every rostered device must have been exercised"
        );
        // NON-VACUITY. Comparing two empty RX FIFOs proves nothing about read
        // timing, and that is exactly what this test did on its first draft —
        // the budget was too small for the transaction to reach the READ at all,
        // so all 14 rows compared `[] == []` and passed. Most of these devices
        // answer a register read; require that the read path actually produced
        // bytes for a solid majority of the roster.
        assert!(
            devices_with_rx * 2 >= DEVICES.len(),
            "only {devices_with_rx}/{} devices returned any RX bytes — the READ              phase is not being reached, so the FIFO comparison is vacuous",
            DEVICES.len()
        );
    }

    /// Program the same short write transaction into either engine.
    #[cfg(feature = "event-scheduler")]
    fn program_write_txn(write: &mut dyn FnMut(u64, u32)) {
        // ~100 kHz-shaped timing: long SCL half-periods, so one wire segment is
        // ~200 module ticks (~800 CPU cycles at module clk = CPU/4). Default
        // reset timing gives ~9-tick segments, which is far too short for a
        // widened wake to be observably stale.
        write(REG_SCL_LOW_PERIOD, 200);
        write(REG_SCL_HIGH_PERIOD, 200);
        write(REG_SDA_HOLD, 40);
        write(REG_SDA_SAMPLE, 40);
        write(REG_SCL_RSTART_SETUP, 200);
        write(REG_SCL_STOP_SETUP, 200);
        write(REG_SCL_STOP_HOLD, 200);
        write(REG_CMD0, cmd(CMD_RSTART, 0));
        write(REG_CMD1_OFFSET, cmd(CMD_WRITE, 2));
        write(REG_CMD1_OFFSET + 4, cmd(CMD_STOP, 0));
        write(REG_DATA, 0x3C << 1);
        write(REG_DATA, 0xA5);
    }

    /// The SCL/SDA waveform, sampled every cycle, is the observable this gate
    /// compares: it is what the GPIO matrix publishes to routed pads and what
    /// the logic analyzer captures.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn rearm_after_fsm_rst_matches_the_per_cycle_walk() {
        const PRE_RST: u64 = 1_500;
        const TOTAL: u64 = 60_000;

        // ── reference: legacy per-cycle walk (no scheduler, no wakes) ──
        let mut walk = Esp32c3I2c::new();
        assert!(
            !walk.uses_scheduler(),
            "reference must be the per-cycle walk"
        );
        let walk_lines = walk.line_levels_arc();
        {
            let mut w = |o: u64, v: u32| {
                walk.write_u32(o, v).unwrap();
            };
            program_write_txn(&mut w);
        }
        walk.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
        let mut walk_wave = Vec::with_capacity(TOTAL as usize);
        for c in 1..=TOTAL {
            walk.tick_elapsed(1);
            if c == PRE_RST {
                // Abort mid-transaction, then immediately restart it.
                walk.write_u32(REG_CTR, CTR_FSM_RST).unwrap();
                walk.write_u32(REG_CTR, CTR_TRANS_START_BIT).unwrap();
            }
            walk_wave.push((walk_lines.scl(), walk_lines.sda()));
        }

        // ── under test: scheduler-driven ──
        let mut sd = SchedDriver::new();
        let sched_lines = sd.dev.line_levels_arc();
        {
            let mut pending: Vec<(u64, u32)> = Vec::new();
            let mut w = |o: u64, v: u32| pending.push((o, v));
            program_write_txn(&mut w);
            for (o, v) in pending {
                sd.write(o, v);
            }
        }
        sd.write(REG_CTR, CTR_TRANS_START_BIT);
        let mut sched_wave = Vec::with_capacity(TOTAL as usize);
        for c in 1..=TOTAL {
            sd.step();
            if c == PRE_RST {
                sd.write(REG_CTR, CTR_FSM_RST);
                sd.write(REG_CTR, CTR_TRANS_START_BIT);
            }
            sched_wave.push((sched_lines.scl(), sched_lines.sda()));
        }

        // The waveform must actually move, or this gate proves nothing.
        let edges = walk_wave.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(
            edges > 8,
            "reference waveform has only {edges} edges — the transaction did not clock"
        );

        if let Some(c) = (0..TOTAL as usize).find(|&i| walk_wave[i] != sched_wave[i]) {
            panic!(
                "scheduler waveform diverges from the per-cycle walk at cycle {}: \
                 walk={:?} sched={:?}. A wake armed before the FSM_RST/TRANS_START \
                 re-anchor drove the engine after it — widen the wake cadence only \
                 with an arming token that kills the superseded chain.",
                c + 1,
                walk_wave[c],
                sched_wave[c]
            );
        }
    }

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
    // ── TCA9548A driven through the ESP32-C3 bit-level engine ───────────────
    //
    // The C3 does not execute its command list synchronously: the transaction
    // is clocked out bit by bit over simulated cycles, and the address frame is
    // resolved at the ACK bit (`ack_bit_level`). That is a third, independent
    // resolution site — plus the `SLAVE_ADDR` fallback beside it — and neither
    // had ever been driven with a bus switch attached.
    mod mux {
        use super::*;
        use crate::peripherals::components::mux_fixture::{
            bytes_written_to, mux_with_tags, tag_for, MUX_ADDR, SENSOR_ADDR,
        };
        use crate::peripherals::components::tca9548a::Tca9548a;

        fn controller() -> Esp32c3I2c {
            let mut p = Esp32c3I2c::new();
            p.push_slave(Box::new(mux_with_tags(4)));
            p
        }

        fn with_mux<R>(p: &Esp32c3I2c, f: impl FnOnce(&Tca9548a) -> R) -> R {
            let mux = p.attached_slaves()[0]
                .as_any()
                .and_then(|a| a.downcast_ref::<Tca9548a>())
                .expect("slave 0 is the switch");
            f(mux)
        }

        /// Program a command list + TX FIFO and clock the bit engine to a park.
        fn program(p: &mut Esp32c3I2c, list: &[(u8, u8)], tx: &[u8]) {
            p.write_u32(REG_INT_CLR, 0xFFFF_FFFF).unwrap();
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
            start_and_run(p);
        }

        fn write_bytes(p: &mut Esp32c3I2c, addr: u8, payload: &[u8]) {
            let mut tx = vec![addr << 1];
            tx.extend_from_slice(payload);
            p.write_u32(REG_SLAVE_ADDR, addr as u32).unwrap();
            program(
                p,
                &[(CMD_RSTART, 0), (CMD_WRITE, tx.len() as u8), (CMD_STOP, 0)],
                &tx,
            );
        }

        fn read_byte(p: &mut Esp32c3I2c, addr: u8) -> u8 {
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

        fn nacked(p: &Esp32c3I2c) -> bool {
            p.read_u32(REG_INT_RAW).unwrap() & INT_NACK != 0
        }

        fn probe_acked(p: &mut Esp32c3I2c, addr: u8) -> bool {
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

        #[test]
        fn the_slave_addr_register_path_also_routes_through_the_switch() {
            let mut p = controller();

            // A zero-payload WRITE has no address byte to clock, so the C3
            // engine skips it entirely; the SLAVE_ADDR fallback on this
            // controller is reached from the address frame itself, when the
            // wire address matches nothing. Park a DIFFERENT wire address and
            // let SLAVE_ADDR carry the real target.
            write_bytes(&mut p, MUX_ADDR, &[1 << 3]);
            p.write_u32(REG_SLAVE_ADDR, SENSOR_ADDR as u32).unwrap();
            program(
                &mut p,
                &[
                    (CMD_RSTART, 0),
                    (CMD_WRITE, 1),
                    (CMD_READ, 1),
                    (CMD_STOP, 0),
                ],
                &[0x00],
            );
            assert!(
                !nacked(&p),
                "SLAVE_ADDR holds 0x13 and channel 3 is enabled — the fallback \
                 must resolve through the switch"
            );
            assert_eq!(
                p.read_u32(REG_DATA).unwrap() as u8,
                tag_for(3),
                "the SLAVE_ADDR fallback must reach the sensor on the SELECTED channel"
            );

            // Isolate every channel: the same fallback must now find nothing.
            write_bytes(&mut p, MUX_ADDR, &[0x00]);
            p.write_u32(REG_SLAVE_ADDR, SENSOR_ADDR as u32).unwrap();
            program(
                &mut p,
                &[(CMD_RSTART, 0), (CMD_WRITE, 1), (CMD_STOP, 0)],
                &[0x00],
            );
            assert!(
                nacked(&p),
                "with every channel isolated the SLAVE_ADDR fallback must NACK"
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
}
