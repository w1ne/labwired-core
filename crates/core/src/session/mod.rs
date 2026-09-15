// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Scriptable single-machine session over the simulator core.
//!
//! A [`Session`] owns one machine built by
//! [`crate::system::builder::build_machine`] and advances it only when asked.
//! Nothing runs between calls, and every duration is **virtual**: an `expect`
//! timeout is simulated seconds at the board's clock, never host wall time, so
//! a timing claim is a claim about the firmware and not about the machine the
//! test happens to run on.
//!
//! Beyond time and the console, a session injects stimulus (`set_input`,
//! `set_pin`, `send`, `inject_can`, `write_u32`) and observes without advancing time (`read_memory`,
//! `read_u32`, `symbol`, `frames`, `logic`, `inspect`). `snapshot`/`restore`
//! rewind the whole session; see [`Session::restore`] for how.

pub mod catalog;
pub mod error;
pub mod frames;
pub mod machine;
mod symbols;
pub mod uart;

pub use crate::network::{CanFrame, CanRxRejection};
pub use error::{SessionError, SessionResult};
pub use frames::Frame;

use crate::inspect::{InspectOpts, MachineInspect};
use crate::logic_capture::{LogicEdgeBatch, LogicSource};
use crate::machine::{AdvanceRequest, AdvanceStop};
use crate::sim_input::InputChannel;
use crate::system::builder::{
    build_machine, BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
};
use crate::HostTimeMode;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

/// `cycles` of simulated time at `cpu_hz`, computed in u128 so it cannot
/// overflow before the result does.
pub(crate) fn duration_for_cycles(cycles: u64, cpu_hz: u64) -> Duration {
    let nanos = u128::from(cycles) * 1_000_000_000 / u128::from(cpu_hz);
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// How a session is opened.
#[derive(Debug, Clone)]
pub struct OpenOptions {
    /// Clock used to convert cycles to and from seconds. `None` (the default)
    /// takes the board's clock: `SystemManifest::cpu_hz`, else
    /// `ChipDescriptor::cpu_hz` — the same resolution the bus uses, so a
    /// session never disagrees with the peripherals about how long a second is.
    pub cpu_hz: Option<u64>,
    /// Fuel per advance batch; bounds how far past a UART match `expect` may
    /// overshoot.
    pub batch_fuel: u64,
    /// Echo the console to the host's stdout.
    pub echo_uart_stdout: bool,
    /// Host wall-clock policy. Default `MaxSpeed` (never sleep).
    pub host_time_mode: HostTimeMode,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            cpu_hz: None,
            batch_fuel: 20_000,
            echo_uart_stdout: false,
            host_time_mode: HostTimeMode::MaxSpeed,
        }
    }
}

/// Why a run returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The requested time elapsed.
    Reached,
    /// The machine stopped making progress (halt, no forward progress, or the
    /// firmware ended its own run).
    Halted,
    /// Execution reached a breakpoint at this PC.
    Breakpoint(u32),
    Error,
}

/// A successful `expect`.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    /// The matched text (lossy UTF-8).
    pub text: String,
    /// Capture groups 1.., `None` for a group that did not participate.
    pub captures: Vec<Option<String>>,
    /// Virtual time at which the match was observed.
    pub at: Duration,
}

/// An address, or a firmware symbol that names one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddrOrSymbol<'a> {
    Addr(u64),
    Symbol(&'a str),
}

/// The build inputs, owned, so [`Session::restore`] can construct the same
/// machine again.
struct OwnedBuild {
    chip: labwired_config::ChipDescriptor,
    system: labwired_config::SystemManifest,
    firmware: OwnedFirmware,
    boot: BootMode,
    blobs: BlobMap,
    options: BuildOptions,
}

enum OwnedFirmware {
    Elf(Vec<u8>),
    FlashImage {
        image: Vec<u8>,
        symbols: Option<Vec<u8>>,
    },
}

impl OwnedBuild {
    fn from_request(req: &BuildRequest<'_>) -> Self {
        Self {
            chip: req.chip.clone(),
            system: req.system.clone(),
            firmware: match req.firmware {
                FirmwareSource::Elf(bytes) => OwnedFirmware::Elf(bytes.to_vec()),
                FirmwareSource::FlashImage { image, symbols } => OwnedFirmware::FlashImage {
                    image: image.to_vec(),
                    symbols: symbols.map(<[u8]>::to_vec),
                },
            },
            boot: req.boot,
            blobs: req.blobs.clone(),
            options: req.options.clone(),
        }
    }

    fn request(&self) -> BuildRequest<'_> {
        BuildRequest {
            chip: &self.chip,
            system: &self.system,
            firmware: match &self.firmware {
                OwnedFirmware::Elf(bytes) => FirmwareSource::Elf(bytes),
                OwnedFirmware::FlashImage { image, symbols } => FirmwareSource::FlashImage {
                    image,
                    symbols: symbols.as_deref(),
                },
            },
            boot: self.boot,
            blobs: &self.blobs,
            options: self.options.clone(),
        }
    }
}

/// A session call that can change machine state, in the order the script made
/// it. Reads through the bus are here too: a read can have side effects
/// (read-to-clear status, MMIO activity bookkeeping), so a replay that skipped
/// them would not be the same run.
#[derive(Debug, Clone)]
enum Op {
    Run(u64),
    Send(Vec<u8>),
    SetInput(String, f64),
    SetInputs(Vec<(String, f64)>),
    ListInputs,
    SetPin(String, bool),
    InjectCan(String, CanFrame),
    WriteU32(u64, u32),
    ReadMemory(u64, usize),
    ReadU32(u64),
    WatchLogic(Vec<Option<LogicSource>>),
    ReadEdges(u64),
}

/// A point a session can be rewound to. See [`Session::restore`].
#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    session: u64,
    ops: Vec<Op>,
    cycles: u64,
    uart_len: usize,
    uart_cursor: usize,
    frame_cursor: u64,
}

impl SessionSnapshot {
    /// Machine cycles at the moment the snapshot was taken.
    pub fn cycles(&self) -> u64 {
        self.cycles
    }
}

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// One machine, driven step by step from a script.
pub struct Session {
    machine: Box<dyn machine::SessionMachine>,
    uart: uart::UartStream,
    board_io: Vec<labwired_config::BoardIoBinding>,
    /// The ELF (or companion symbols ELF) symbols resolve against.
    firmware_bytes: Vec<u8>,
    symbols: OnceLock<HashMap<String, symbols::Symbol>>,
    /// Highest bus-trace `seq` already returned by [`Session::frames`].
    frame_cursor: u64,
    cpu_hz: u64,
    opts: OpenOptions,
    build: OwnedBuild,
    journal: Mutex<Vec<Op>>,
    id: u64,
}

impl Session {
    /// Open a session for a catalog chip by name (`configs/chips/<chip>.yaml`),
    /// with `firmware` loaded fast-boot. See [`catalog::Catalog::resolve`] for
    /// how the chip and its system manifest are found.
    pub fn from_chip_name(
        chip: &str,
        firmware: FirmwareSource<'_>,
        opts: OpenOptions,
    ) -> anyhow::Result<Session> {
        let cat = catalog::Catalog::discover();
        let (chip, manifest) = cat.resolve(chip)?;
        let blobs = crate::system::builder::BlobMap::new();
        Session::open(
            BuildRequest {
                chip: &chip,
                system: &manifest,
                firmware,
                boot: crate::system::builder::BootMode::FastBoot,
                blobs: &blobs,
                options: BuildOptions::default(),
            },
            opts,
        )
    }

    /// Build the machine through [`build_machine`] and wrap it.
    pub fn open(mut req: BuildRequest<'_>, opts: OpenOptions) -> anyhow::Result<Session> {
        let cpu_hz = opts
            .cpu_hz
            .unwrap_or_else(|| req.system.cpu_hz.unwrap_or(req.chip.cpu_hz));
        if cpu_hz == 0 {
            anyhow::bail!(
                "chip '{}' declares no cpu_hz and none was passed in OpenOptions; a session \
                 cannot convert cycles to time without a clock",
                req.chip.name
            );
        }
        if opts.batch_fuel == 0 {
            anyhow::bail!("OpenOptions::batch_fuel must be greater than zero");
        }
        req.options.echo_uart_stdout |= opts.echo_uart_stdout;
        let build = OwnedBuild::from_request(&req);
        let built = build_machine(req)?;
        let mut machine = built.machine;
        machine.set_host_time_mode(opts.host_time_mode);
        Ok(Session {
            machine,
            uart: uart::UartStream::new(built.uart),
            board_io: built.board_io,
            firmware_bytes: built.firmware_bytes,
            symbols: OnceLock::new(),
            frame_cursor: 0,
            cpu_hz,
            opts,
            build,
            journal: Mutex::new(Vec::new()),
            id: NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed),
        })
    }

    fn record(&self, op: Op) {
        self.journal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(op);
    }

    /// Simulated machine cycles since the machine was built.
    pub fn cycles(&self) -> u64 {
        self.machine.cycles()
    }

    /// The clock this session converts cycles with, in Hz.
    pub fn cpu_hz(&self) -> u64 {
        self.cpu_hz
    }

    /// Host wall-clock policy for [`Self::run_for`] / [`Self::run_cycles`].
    pub fn host_time_mode(&self) -> HostTimeMode {
        self.machine.host_time_mode()
    }

    /// Set the host wall-clock policy for subsequent advances.
    pub fn set_host_time_mode(&mut self, mode: HostTimeMode) {
        self.machine.set_host_time_mode(mode);
    }

    /// Virtual time: cycles / cpu_hz.
    pub fn time(&self) -> Duration {
        duration_for_cycles(self.cycles(), self.cpu_hz)
    }

    /// Cycles covering `d` at this session's clock, rounded up, at least one.
    fn cycles_for(&self, d: Duration) -> u64 {
        let c = (d.as_nanos() * u128::from(self.cpu_hz)).div_ceil(1_000_000_000);
        u64::try_from(c).unwrap_or(u64::MAX).max(1)
    }

    /// Advance by `virtual_time` of simulated time.
    pub fn run_for(&mut self, virtual_time: Duration) -> SessionResult<StopReason> {
        let cycles = self.cycles_for(virtual_time);
        self.run_cycles(cycles)
    }

    /// Advance by exactly `cycles` machine cycles, unless the machine stops
    /// first.
    pub fn run_cycles(&mut self, cycles: u64) -> SessionResult<StopReason> {
        self.record(Op::Run(cycles));
        self.advance_cycles(cycles)
    }

    fn advance_cycles(&mut self, cycles: u64) -> SessionResult<StopReason> {
        let target = self.machine.cycles().saturating_add(cycles);
        while self.machine.cycles() < target {
            let remaining = target - self.machine.cycles();
            let req = AdvanceRequest::run(Some(self.opts.batch_fuel)).with_cycle_limit(remaining);
            match self.machine.advance(req) {
                Ok(report) => match report.stop {
                    AdvanceStop::Breakpoint(pc) => return Ok(StopReason::Breakpoint(pc)),
                    AdvanceStop::NoProgress | AdvanceStop::FirmwareExit { .. } => {
                        return Ok(StopReason::Halted)
                    }
                    _ if report.primary_steps == 0 && report.idle_cycles == 0 => {
                        return Ok(StopReason::Halted)
                    }
                    _ => {}
                },
                Err(crate::SimulationError::Halt) => return Ok(StopReason::Halted),
                Err(crate::SimulationError::BreakpointHit(pc)) => {
                    return Ok(StopReason::Breakpoint(pc))
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(StopReason::Reached)
    }

    /// Wait, in virtual time, for `pattern` (a regex) on the console.
    ///
    /// Unread bytes are matched first, so output a previous call ran past is
    /// never lost. On a match the cursor moves to the end of it. The budget is
    /// `timeout` of simulated time; if it runs out, or the machine stops before
    /// a match, the result is [`SessionError::ExpectTimeout`].
    pub fn expect(&mut self, pattern: &str, timeout: Duration) -> SessionResult<Match> {
        let re = regex::bytes::Regex::new(pattern)
            .map_err(|e| SessionError::Other(format!("invalid expect pattern /{pattern}/: {e}")))?;
        let deadline = self
            .machine
            .cycles()
            .saturating_add(self.cycles_for(timeout));
        loop {
            if let Some(m) = self.take_match(&re) {
                return Ok(m);
            }
            let now = self.machine.cycles();
            if now >= deadline {
                return Err(self.expect_timeout(pattern, timeout, false));
            }
            let step = (deadline - now).min(self.opts.batch_fuel);
            match self.run_cycles(step)? {
                StopReason::Reached => {}
                StopReason::Breakpoint(pc) => {
                    if let Some(m) = self.take_match(&re) {
                        return Ok(m);
                    }
                    return Err(crate::SimulationError::BreakpointHit(pc).into());
                }
                StopReason::Halted | StopReason::Error => {
                    if let Some(m) = self.take_match(&re) {
                        return Ok(m);
                    }
                    return Err(self.expect_timeout(pattern, timeout, true));
                }
            }
        }
    }

    fn take_match(&mut self, re: &regex::bytes::Regex) -> Option<Match> {
        let unread = self.uart.unread();
        let caps = re.captures(&unread)?;
        let whole = caps.get(0)?;
        let m = Match {
            text: String::from_utf8_lossy(whole.as_bytes()).into_owned(),
            captures: caps
                .iter()
                .skip(1)
                .map(|g| g.map(|g| String::from_utf8_lossy(g.as_bytes()).into_owned()))
                .collect(),
            at: self.time(),
        };
        self.uart.consume(whole.end());
        Some(m)
    }

    fn expect_timeout(&self, pattern: &str, timeout: Duration, halted: bool) -> SessionError {
        let transcript = self.uart_transcript();
        let skip = transcript.chars().count().saturating_sub(200);
        SessionError::ExpectTimeout {
            pattern: pattern.to_string(),
            virtual_seconds: timeout.as_secs_f64(),
            tail: transcript.chars().skip(skip).collect(),
            halted,
        }
    }

    /// Queue bytes on every UART RX feeder; firmware sees them as time
    /// advances.
    pub fn send(&mut self, bytes: &[u8]) {
        self.record(Op::Send(bytes.to_vec()));
        self.uart.send(bytes);
    }

    /// Drain console bytes not yet read by `read_uart` or matched by `expect`.
    pub fn read_uart(&mut self) -> Vec<u8> {
        let unread = self.uart.unread();
        self.uart.consume(unread.len());
        unread
    }

    /// Everything the console has printed since the session opened.
    pub fn uart_transcript(&self) -> String {
        String::from_utf8_lossy(&self.uart.all()).into_owned()
    }

    // ── stimulus ────────────────────────────────────────────────────────────

    /// Drive one input channel (an engineering-unit value on whichever attached
    /// device exposes `channel`). A channel no device exposes, or more than one
    /// does, is a typed [`SessionError::Input`].
    pub fn set_input(&mut self, channel: &str, value: f64) -> SessionResult<()> {
        self.record(Op::SetInput(channel.to_string(), value));
        Ok(self.machine.set_input(channel, value)?)
    }

    /// Apply several input sets as one atomic transaction: all apply, or none
    /// do and nothing moves.
    pub fn set_inputs(&mut self, sets: &[(String, f64)]) -> SessionResult<()> {
        self.record(Op::SetInputs(sets.to_vec()));
        Ok(self.machine.set_inputs(sets)?)
    }

    /// Every drivable input channel, with the device that owns it.
    pub fn list_inputs(&mut self) -> Vec<(String, InputChannel)> {
        self.record(Op::ListInputs);
        self.machine.list_inputs()
    }

    /// Assert or release a `board_io` input binding (a button, a detect line)
    /// by its id, e.g. `"user_button"`.
    ///
    /// `active` is the binding's logical state: `true` holds the pin at its
    /// active level (high for `active_high`, low otherwise), `false` at the
    /// other. It goes through the same `set_gpio_input` seam the browser's
    /// board-IO buttons use, and the level holds until changed. An id that is
    /// not an input binding is [`SessionError::UnknownPin`].
    pub fn set_pin(&mut self, id: &str, active: bool) -> SessionResult<()> {
        self.record(Op::SetPin(id.to_string(), active));
        self.drive_pin(id, active)
    }

    fn drive_pin(&mut self, id: &str, active: bool) -> SessionResult<()> {
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == id && b.signal == labwired_config::BoardIoSignal::Input)
            .ok_or_else(|| SessionError::UnknownPin(id.to_string()))?;
        let level = if binding.active_high { active } else { !active };
        self.machine
            .set_gpio_input(&binding.peripheral, binding.pin, level)
            .map_err(|e| match e {
                machine::GpioInputError::UnknownPeripheral => {
                    SessionError::UnknownPeripheral(binding.peripheral.clone())
                }
                machine::GpioInputError::NotDrivable => SessionError::Other(format!(
                    "peripheral '{}' does not expose GPIO input control",
                    binding.peripheral
                )),
            })
    }

    /// Deliver `frame` to the receive path of the CAN controller named `bus`
    /// (`"bxcan1"`, `"fdcan1"`), as if it had arrived from the wire.
    ///
    /// The controller decides, as silicon does: its clock must be enabled, it
    /// must be out of initialization, and a bxCAN needs an active acceptance
    /// filter that matches and room in RX FIFO0. A frame it does not take is
    /// [`SessionError::CanRejected`] with the reason. An accepted frame shows up
    /// in [`Self::frames`] as an `rx` CAN event, and the controller raises its
    /// RX interrupt if firmware enabled one.
    ///
    /// `bus` naming no peripheral is [`SessionError::UnknownPeripheral`], a
    /// peripheral that is not a CAN controller is
    /// [`SessionError::NotACanController`], and a malformed frame (an 11-bit id
    /// above 0x7FF, a length CAN or CAN-FD cannot carry, BRS without FD, a
    /// remote FD frame) is [`SessionError::InvalidCanFrame`].
    pub fn inject_can(&mut self, bus: &str, frame: CanFrame) -> SessionResult<()> {
        validate_can_frame(&frame)?;
        self.record(Op::InjectCan(bus.to_string(), frame.clone()));
        self.deliver_can(bus, frame)
    }

    fn deliver_can(&mut self, bus: &str, frame: CanFrame) -> SessionResult<()> {
        use crate::network::CanInjectError;
        self.machine.inject_can(bus, frame).map_err(|e| match e {
            CanInjectError::UnknownPeripheral => SessionError::UnknownPeripheral(bus.to_string()),
            CanInjectError::NotACanController => SessionError::NotACanController(bus.to_string()),
            CanInjectError::Rejected(reason) => SessionError::CanRejected {
                bus: bus.to_string(),
                reason,
            },
        })
    }

    /// Write a little-endian word at an address or symbol through the bus, as
    /// firmware would: peripheral registers take it with their write side
    /// effects (a W1C clears, an enable bit enables).
    pub fn write_u32(&mut self, at: AddrOrSymbol<'_>, value: u32) -> SessionResult<()> {
        let addr = self.resolve(at)?;
        self.record(Op::WriteU32(addr, value));
        Ok(self.machine.bus_write_u32(addr, value)?)
    }

    // ── observe ─────────────────────────────────────────────────────────────

    /// `len` bytes from `addr` through the bus, as firmware would read them.
    /// An address the bus does not map is an error.
    ///
    /// This is the real read path, so read side effects fire (a read-to-clear
    /// status register is cleared). To look at a register without touching
    /// it, use [`Self::inspect`].
    pub fn read_memory(&self, addr: u64, len: usize) -> SessionResult<Vec<u8>> {
        self.record(Op::ReadMemory(addr, len));
        self.bus_read_memory(addr, len)
    }

    fn bus_read_memory(&self, addr: u64, len: usize) -> SessionResult<Vec<u8>> {
        let last = addr.saturating_add(len.saturating_sub(1) as u64);
        let (Ok(start), Ok(_)) = (u32::try_from(addr), u32::try_from(last)) else {
            return Err(crate::SimulationError::MemoryViolation(addr.max(1 << 32)).into());
        };
        Ok(self.machine.read_memory(start, len)?)
    }

    /// A little-endian word at an address or symbol, through the bus's
    /// width-aware read path (the one the CLI's `memory_value` assertion
    /// uses). A function symbol is read where its code lives, Thumb bit
    /// cleared.
    pub fn read_u32(&self, at: AddrOrSymbol<'_>) -> SessionResult<u32> {
        let addr = self.resolve(at)?;
        self.record(Op::ReadU32(addr));
        Ok(self.machine.bus_read_u32(addr)?)
    }

    /// The address `at` names; a function symbol resolves to where its code
    /// lives (Thumb bit cleared).
    fn resolve(&self, at: AddrOrSymbol<'_>) -> SessionResult<u64> {
        match at {
            AddrOrSymbol::Addr(addr) => Ok(addr),
            AddrOrSymbol::Symbol(name) => self
                .symbol_table()
                .get(name)
                .map(|s| s.location)
                .ok_or_else(|| SessionError::UnknownSymbol(name.to_string())),
        }
    }

    /// The address of `name` in the firmware's symbol table, as the ELF records
    /// it (a Thumb function keeps bit 0, like its vector-table entry). `None`
    /// when the symbol is absent or the firmware carries no symbols.
    pub fn symbol(&self, name: &str) -> Option<u64> {
        self.symbol_table().get(name).map(|s| s.value)
    }

    fn symbol_table(&self) -> &HashMap<String, symbols::Symbol> {
        self.symbols
            .get_or_init(|| symbols::table(&self.firmware_bytes))
    }

    /// Bus traffic (I²C, SPI, UART, CAN) since the previous call, oldest
    /// first. No event is ever returned twice.
    ///
    /// The machine keeps a bounded ring of recent events; a script that lets
    /// more traffic pass between calls than the ring holds sees the oldest of
    /// it evicted, which shows as a gap in `seq`.
    pub fn frames(&mut self) -> Vec<Frame> {
        let cursor = self.frame_cursor;
        let frames: Vec<Frame> = self
            .machine
            .bus_trace_events()
            .into_iter()
            .filter(|ev| ev.seq > cursor)
            .map(|ev| frames::from_event(ev, self.cpu_hz))
            .collect();
        if let Some(last) = frames.last() {
            self.frame_cursor = last.seq;
        }
        frames
    }

    /// Arm the logic analyzer on GPIO pads, given as `(peripheral, pin)`, and
    /// return each pad's current level. Replaces any previous watch set; an
    /// empty set disarms. A channel's index in `pads` is its `ch` in
    /// [`Self::logic`]. An unknown peripheral arms nothing.
    pub fn watch_logic(&mut self, pads: &[(&str, u8)]) -> SessionResult<Vec<Option<bool>>> {
        let peripherals = self.machine.get_peripherals();
        let resolved = pads
            .iter()
            .map(|(name, pin)| {
                peripherals
                    .iter()
                    .position(|(p, _, _)| p == name)
                    .map(|idx| Some(LogicSource::pad(idx, *pin)))
                    .ok_or_else(|| SessionError::UnknownPeripheral((*name).to_string()))
            })
            .collect::<SessionResult<Vec<_>>>()?;
        self.record(Op::WatchLogic(resolved.clone()));
        Ok(self.machine.logic_watch(&resolved))
    }

    /// Logic edges newer than `cursor` on the pads armed by
    /// [`Self::watch_logic`]. Pass `0` first, then the returned `cursor`.
    pub fn logic(&mut self, cursor: u64) -> LogicEdgeBatch {
        self.record(Op::ReadEdges(cursor));
        self.machine.logic_read_edges(cursor)
    }

    /// Side-effect-free decode of one peripheral (`Some(name)`) or all of them
    /// and their attached devices.
    pub fn inspect(&self, name: Option<&str>) -> MachineInspect {
        self.machine.inspect(name, &InspectOpts::default())
    }

    // ── snapshot ────────────────────────────────────────────────────────────

    /// Mark the current point so [`Self::restore`] can return to it.
    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            session: self.id,
            ops: self
                .journal
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            cycles: self.cycles(),
            uart_len: self.uart.len(),
            uart_cursor: self.uart.cursor(),
            frame_cursor: self.frame_cursor,
        }
    }

    /// Return the session to `snap`: machine, attached devices, console, bus
    /// trace, logic watch, and the read cursors, all as they were.
    ///
    /// The machine's own snapshot type carries CPU registers and the few
    /// peripherals that serialise themselves; RAM, most peripheral and device
    /// state, and the event scheduler are not in it, so restoring from it would
    /// report success over a machine that never existed. Instead the session
    /// rebuilds the machine from its original inputs and replays, in order,
    /// every state-changing call made before the snapshot (runs, stimulus, bus
    /// reads). The simulator is deterministic in virtual time, so that is the
    /// same machine, not an approximation, and the replay checks it: if the
    /// cycle count or console length differ from the snapshot's, the result is
    /// an error, and the session is left at the replayed point. The cost is
    /// re-simulating up to the snapshot's time.
    ///
    /// A snapshot belongs to the session that took it; any other session
    /// refuses it.
    pub fn restore(&mut self, snap: &SessionSnapshot) -> SessionResult<()> {
        if snap.session != self.id {
            return Err(SessionError::Other(
                "snapshot was taken on a different session; restore replays this \
                 session's own firmware and stimulus"
                    .into(),
            ));
        }
        let built = build_machine(self.build.request())
            .map_err(|e| SessionError::Other(format!("restore: rebuilding the machine: {e:#}")))?;
        self.machine = built.machine;
        self.uart = uart::UartStream::new(built.uart);
        self.board_io = built.board_io;
        for op in &snap.ops {
            self.replay(op);
        }
        *self.journal.lock().unwrap_or_else(PoisonError::into_inner) = snap.ops.clone();
        if self.cycles() != snap.cycles || self.uart.len() != snap.uart_len {
            return Err(SessionError::Other(format!(
                "restore diverged: replay reached cycle {} with {} console bytes, the \
                 snapshot was taken at cycle {} with {}",
                self.cycles(),
                self.uart.len(),
                snap.cycles,
                snap.uart_len
            )));
        }
        self.uart.set_cursor(snap.uart_cursor);
        self.frame_cursor = snap.frame_cursor;
        Ok(())
    }

    /// Re-apply one journaled call. Results are discarded: the original call
    /// already reported them, and replaying an error reproduces its effect
    /// (usually none) exactly.
    fn replay(&mut self, op: &Op) {
        match op {
            Op::Run(cycles) => {
                let _ = self.advance_cycles(*cycles);
            }
            Op::Send(bytes) => self.uart.send(bytes),
            Op::SetInput(channel, value) => {
                let _ = self.machine.set_input(channel, *value);
            }
            Op::SetInputs(sets) => {
                let _ = self.machine.set_inputs(sets);
            }
            Op::ListInputs => {
                let _ = self.machine.list_inputs();
            }
            Op::SetPin(id, active) => {
                let _ = self.drive_pin(id, *active);
            }
            Op::InjectCan(bus, frame) => {
                let _ = self.deliver_can(bus, frame.clone());
            }
            Op::WriteU32(addr, value) => {
                let _ = self.machine.bus_write_u32(*addr, *value);
            }
            Op::ReadMemory(addr, len) => {
                let _ = self.bus_read_memory(*addr, *len);
            }
            Op::ReadU32(addr) => {
                let _ = self.machine.bus_read_u32(*addr);
            }
            Op::WatchLogic(sources) => {
                let _ = self.machine.logic_watch(sources);
            }
            Op::ReadEdges(cursor) => {
                let _ = self.machine.logic_read_edges(*cursor);
            }
        }
    }
}

/// Lengths a CAN-FD data field can have (DLC 0..=15).
const FD_LENGTHS: [usize; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];

/// Reject a frame no CAN or CAN-FD controller could have received, so a
/// controller model never silently truncates an id or a payload.
fn validate_can_frame(frame: &CanFrame) -> SessionResult<()> {
    let invalid = |msg: String| Err(SessionError::InvalidCanFrame(msg));
    let max_id = if frame.extended { 0x1FFF_FFFF } else { 0x7FF };
    if frame.id > max_id {
        let kind = if frame.extended {
            "29-bit extended"
        } else {
            "11-bit standard"
        };
        return invalid(format!(
            "id {:#x} does not fit an {kind} identifier",
            frame.id
        ));
    }
    let len = frame.data.len();
    if frame.fd {
        if !FD_LENGTHS.contains(&len) {
            return invalid(format!("{len} bytes is not a CAN-FD data length"));
        }
        if frame.remote {
            return invalid("CAN-FD has no remote frames".into());
        }
    } else {
        if len > 8 {
            return invalid(format!("{len} bytes exceeds a classic CAN frame's 8"));
        }
        if frame.bitrate_switch {
            return invalid("bitrate switch is only defined for CAN-FD frames".into());
        }
    }
    Ok(())
}
