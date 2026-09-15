// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Board signal routing for co-simulation.
//!
//! [`CosimRunner`](crate::cosim::CosimRunner) exchanges values with external
//! models through a flat signal store keyed by manifest paths. This module is
//! the half that fills that store from a running machine and writes routed
//! model outputs back into it, so a `cosim_models:` entry can name real
//! firmware pins instead of abstract observables:
//!
//! ```yaml
//! inputs:
//!   gpio: "board.gpio.pa5"            # firmware GPIO output level -> model
//!   drive: "board.gpio_output.pa5"    # is the firmware driving PA5 at all?
//!   touch: "ui.pad.pressed"           # set from outside the engine
//! outputs:
//!   v_out: "board.analog.pa0_volts"   # model node voltage -> ADC channel
//!   v_rx: "board.gpio_in.pd2"         # node voltage -> thresholded pin level
//! ```
//!
//! Paths outside the `board.` / `adc.` grammar are ordinary store paths and are
//! left untouched: a manifest that routes `control.enable` keeps behaving
//! exactly as it did before this module existed.
//!
//! Everything resolves ONCE, against the bus, in [`SignalRouter::bind`]. A pad
//! label cannot change owner mid-run, so the per-step work is an index and a
//! bit — and an unresolvable path is reported at bind time instead of silently
//! reading zero for the whole run.

use crate::bus::SystemBus;
use crate::cosim::{
    CosimInputKind, CosimRoutedModelStep, CosimRunner, CosimSignalValue, CosimSignals,
};
use crate::{
    AdvanceReport, AdvanceRequest, AdvanceStop, Cpu, Machine, Peripheral, SimResult,
    SimulationError,
};
use labwired_config::{
    CosimAdapter as ManifestCosimAdapter, CosimModelConfig, GpioInputThresholds,
};
use std::collections::BTreeSet;
use std::path::Path;

/// Default core clock assumed when a bus reports none, in Hz.
///
/// Every in-tree chip descriptor declares `cpu_hz` (a config gate enforces it),
/// so this only covers a hand-built bus in a test. Co-simulation needs a time
/// base to convert cycles into the `time_ns` an adapter is stepped to, and
/// dividing by zero is not an option; the session logs once when it falls back.
pub const FALLBACK_CPU_HZ: u64 = 16_000_000;

const NANOS_PER_SECOND: u128 = 1_000_000_000;

/// Simulated cycles → nanoseconds at `cpu_hz`, truncating.
pub fn cycles_to_ns(cycles: u64, cpu_hz: u64) -> u64 {
    if cpu_hz == 0 {
        return 0;
    }
    u64::try_from(u128::from(cycles) * NANOS_PER_SECOND / u128::from(cpu_hz)).unwrap_or(u64::MAX)
}

/// Nanoseconds → the FIRST cycle count whose [`cycles_to_ns`] is at or past
/// `ns` (ceiling division).
///
/// Ceiling, not truncation: this is used to decide how far the machine may run
/// before the next co-simulation boundary, and truncating would stop the
/// machine one cycle short of the boundary forever — the boundary would never
/// be reached and the run would crawl a cycle at a time.
pub fn ns_to_cycles(ns: u64, cpu_hz: u64) -> u64 {
    if cpu_hz == 0 {
        return 0;
    }
    let numerator = u128::from(ns) * u128::from(cpu_hz);
    let cycles = numerator.div_ceil(NANOS_PER_SECOND);
    u64::try_from(cycles).unwrap_or(u64::MAX)
}

/// Full-scale reference of the modelled STM32 ADC, in volts.
///
/// The ADC models hold injected stimuli in millivolts and own the
/// millivolt → count conversion — `Adc::set_channel_input` computes
/// `count = mV * 4095 / 3300`, i.e. 3.3 V is full scale at 12 bits. This
/// constant is here only so
/// a routed voltage can be clamped to something the pin could physically see;
/// the arithmetic deliberately stays in the ADC model, where every other
/// analog stimulus (thermistor, potentiometer, battery divider) already
/// converts.
pub const ADC_VREF_VOLTS: f64 = 3.3;

/// Prefix of the signal paths set from outside the engine:
/// `ui.<partId>.<field>`, written by a canvas part or a test stimulus through
/// [`CosimSession::set_signal`]. Every one a model reads starts at 0 / false.
pub const UI_SIGNAL_PREFIX: &str = "ui.";

/// A digital pad's input thresholds in volts: the chip descriptor's
/// `gpio_input_thresholds` ratios times its `io_voltage_v`.
///
/// This is what turns a voltage routed to `board.gpio_in.<pad>` into the level
/// the firmware reads, with the hysteresis of a Schmitt-trigger input: see
/// [`Self::next_level`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputThresholds {
    /// At or below this the pad reads low.
    pub vil_volts: f64,
    /// At or above this the pad reads high.
    pub vih_volts: f64,
}

impl InputThresholds {
    /// Thresholds for a pad supplied at `io_voltage_v`.
    pub fn from_ratios(io_voltage_v: f64, ratios: GpioInputThresholds) -> Self {
        Self {
            vil_volts: ratios.vil * io_voltage_v,
            vih_volts: ratios.vih * io_voltage_v,
        }
    }

    /// The thresholds `bus`'s chip descriptor declares, or the descriptor key
    /// that is missing.
    pub fn for_bus(bus: &SystemBus) -> Result<Self, &'static str> {
        let ratios = bus.gpio_input_thresholds.ok_or("gpio_input_thresholds")?;
        let io_voltage_v = bus.io_voltage_v.ok_or("io_voltage_v")?;
        Ok(Self::from_ratios(io_voltage_v, ratios))
    }

    /// The level a Schmitt input reads at `volts`, given the level it read
    /// last: high at or above VIH, low at or below VIL, and unchanged in the
    /// band between.
    ///
    /// Keeping the previous level inside the band is the point. A pad charging
    /// slowly through a megohm sits between the thresholds for many samples,
    /// and a single midpoint comparison would read the pin as whichever side
    /// of it the last sample landed on. `NaN` is on neither side of anything,
    /// so it also keeps the previous level.
    pub fn next_level(&self, previous: bool, volts: f64) -> bool {
        if volts >= self.vih_volts {
            true
        } else if volts <= self.vil_volts {
            false
        } else {
            previous
        }
    }
}

/// A manifest signal path that names something on the board.
///
/// Grammar (documented in `docs/cosimulation_plugins.md`):
///
/// | path | direction | type |
/// |------|-----------|------|
/// | `board.gpio.<pad>` | machine → model | `Bool` |
/// | `board.gpio_output.<pad>` | machine → model | `Bool` |
/// | `board.gpio_in.<pad>` | model → machine (also readable) | `Bool`, or `F64` volts |
/// | `board.analog.<pad>_volts` | model → machine | `F64` |
/// | `adc.<peripheral>.<channel>_volts` | model → machine | `F64` |
///
/// `<pad>` is a pad label in whatever form the chip speaks — `pa5` / `PA5` on
/// STM32, `pd2` on the ATmega328P, `p0.13` on Nordic, `gpio5` / `5` on ESP32 —
/// resolved through the same [`SystemBus`] pin resolution every other
/// pad-addressed feature uses.
///
/// `ui.<partId>.<field>` ([`UI_SIGNAL_PREFIX`]) is not a board path: nothing
/// in the machine owns it, and it is set from outside the engine with
/// [`CosimSession::set_signal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalPath {
    /// `board.gpio.<pad>` — the level the firmware is DRIVING on an output pad.
    GpioOutput { pad: String },
    /// `board.gpio_output.<pad>` — whether the firmware has made `<pad>` a
    /// general-purpose OUTPUT, read from the GPIO model's direction register
    /// (STM32 MODER / CRL-CRH, nRF DIR, ESP32 GPIO_ENABLE + output matrix, …).
    ///
    /// `board.gpio.<pad>` alone cannot tell a circuit whether the pin is
    /// driving: an input pad still has an output latch, and reading that latch
    /// as a source would clamp whatever the circuit puts on the pin.
    GpioDirection { pad: String },
    /// `board.gpio_in.<pad>` — the level an external driver holds on an input
    /// pad. A boolean is the level; a number is volts, turned into a level with
    /// the chip's [`InputThresholds`].
    GpioInput { pad: String },
    /// `board.analog.<pad>_volts` — the analog level on the ADC input the chip
    /// descriptor's `analog_pins:` assigns to `<pad>`.
    AnalogPad { pad: String },
    /// `adc.<peripheral>.<channel>_volts` — the analog level on an explicitly
    /// named ADC channel, for chips whose descriptor records no analog pads.
    AdcChannel { peripheral: String, channel: u8 },
}

impl SignalPath {
    /// Parse a manifest path. `None` means "not a board path" — an ordinary
    /// signal-store key, which the runner routes between models untouched.
    pub fn parse(path: &str) -> Option<Self> {
        if let Some(pad) = path.strip_prefix("board.gpio_output.") {
            return (!pad.is_empty()).then(|| Self::GpioDirection {
                pad: pad.to_string(),
            });
        }
        if let Some(pad) = path.strip_prefix("board.gpio_in.") {
            return (!pad.is_empty()).then(|| Self::GpioInput {
                pad: pad.to_string(),
            });
        }
        if let Some(pad) = path.strip_prefix("board.gpio.") {
            return (!pad.is_empty()).then(|| Self::GpioOutput {
                pad: pad.to_string(),
            });
        }
        if let Some(rest) = path.strip_prefix("board.analog.") {
            let pad = rest.strip_suffix("_volts")?;
            return (!pad.is_empty()).then(|| Self::AnalogPad {
                pad: pad.to_string(),
            });
        }
        if let Some(rest) = path.strip_prefix("adc.") {
            let rest = rest.strip_suffix("_volts")?;
            let (peripheral, channel) = rest.rsplit_once('.')?;
            if peripheral.is_empty() {
                return None;
            }
            let channel: u8 = channel.parse().ok()?;
            return Some(Self::AdcChannel {
                peripheral: peripheral.to_string(),
                channel,
            });
        }
        None
    }

    /// Can a model READ this path (machine → store)?
    pub fn is_readable(&self) -> bool {
        matches!(
            self,
            Self::GpioOutput { .. } | Self::GpioDirection { .. } | Self::GpioInput { .. }
        )
    }

    /// Can a model WRITE this path (store → machine)?
    pub fn is_writable(&self) -> bool {
        matches!(
            self,
            Self::GpioInput { .. } | Self::AnalogPad { .. } | Self::AdcChannel { .. }
        )
    }
}

/// Why one routed path could not be honoured.
///
/// Reported rather than swallowed: a co-simulation whose pin never reached the
/// firmware still runs to completion and still prints a verdict, so a silent
/// drop here is a run that proves nothing while claiming to.
///
/// No `Eq`: [`RoutingError::TypeMismatch`] carries the offending
/// [`CosimSignalValue`], whose `F64` arm makes equality partial.
#[derive(Debug, Clone, PartialEq)]
pub enum RoutingError {
    /// The pad label does not resolve on this chip.
    UnknownPad { path: String, pad: String },
    /// The pad resolves but its owning GPIO block is not on the bus.
    UnknownGpio { path: String, pad: String },
    /// The GPIO model that owns the pad cannot report the pad's direction, so
    /// `board.gpio_output.<pad>` has no honest answer on this chip.
    NoDirection { path: String, pad: String },
    /// Volts are routed to `board.gpio_in.<pad>`, but the chip descriptor does
    /// not declare `key` (`gpio_input_thresholds` or `io_voltage_v`), so no
    /// voltage can be turned into a level.
    NoInputThresholds { path: String, key: &'static str },
    /// A model output was routed to a `ui.` path, which only the outside world
    /// sets.
    ExternalOnly { path: String },
    /// [`CosimSession::set_signal`] named a path no model input reads.
    UnknownSignal { path: String },
    /// [`CosimSession::set_signal`] named a board path, which the machine owns.
    NotSettable { path: String },
    /// [`CosimSession::set_signal_number`] was handed a NaN or an infinity.
    NotFinite { path: String, value: f64 },
    /// A model output was routed to a path only the machine can drive.
    NotWritable { path: String },
    /// A model input was sourced from a path the machine cannot be read from.
    NotReadable { path: String },
    /// The chip descriptor's `analog_pins:` names no ADC input for this pad.
    /// Use the explicit `adc.<peripheral>.<channel>_volts` form instead.
    NoAdcChannel { path: String, pad: String },
    /// No ADC on the bus accepted the channel.
    AdcUnavailable { path: String, channel: u8 },
    /// The path names a peripheral this bus does not have.
    UnknownPeripheral { path: String, peripheral: String },
    /// The path names a peripheral that is not an ADC LabWired can drive.
    NotAnAdc { path: String, peripheral: String },
    /// The ADC has no such input. Its channels are `0..channels`.
    NoSuchAdcChannel {
        path: String,
        peripheral: String,
        channel: u8,
        channels: u8,
    },
    /// The owning GPIO block refused an externally driven level.
    DriveRejected { path: String },
    /// The store held a value of the wrong shape for this path.
    TypeMismatch {
        path: String,
        value: CosimSignalValue,
    },
}

impl std::fmt::Display for RoutingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownPad { path, pad } => {
                write!(
                    f,
                    "co-sim path '{path}': pad '{pad}' does not resolve on this chip"
                )
            }
            Self::UnknownGpio { path, pad } => write!(
                f,
                "co-sim path '{path}': pad '{pad}' resolves but its GPIO block is not on the bus"
            ),
            Self::NoDirection { path, pad } => write!(
                f,
                "co-sim path '{path}': the GPIO model that owns pad '{pad}' does not report pin \
                 direction, so whether the firmware drives the pad is unknown on this chip"
            ),
            Self::NoInputThresholds { path, key } => write!(
                f,
                "co-sim path '{path}' is driven with volts, but the chip descriptor declares no \
                 `{key}`, so the voltage cannot become a pin level; add `io_voltage_v` and \
                 `gpio_input_thresholds: {{ vil, vih }}` from the datasheet, or route a boolean"
            ),
            Self::ExternalOnly { path } => write!(
                f,
                "co-sim path '{path}' is set from outside the engine (a canvas part, a test \
                 stimulus); a model output cannot drive it"
            ),
            Self::UnknownSignal { path } => write!(
                f,
                "co-sim signal '{path}': no model input reads it (check the `inputs:` of the \
                 lab's cosim_models)"
            ),
            Self::NotSettable { path } => write!(
                f,
                "co-sim signal '{path}' belongs to the machine, which writes or reads it at every \
                 model boundary; only `ui.` and other model-only paths can be set from outside"
            ),
            Self::NotFinite { path, value } => {
                write!(f, "co-sim signal '{path}': {value} is not a finite number")
            }
            Self::NotWritable { path } => write!(
                f,
                "co-sim path '{path}' is a model INPUT only; a model output cannot drive it \
                 (use board.gpio_in.<pad> to drive a pin)"
            ),
            Self::NotReadable { path } => write!(
                f,
                "co-sim path '{path}' is a model OUTPUT only; it cannot be read into a model input"
            ),
            Self::NoAdcChannel { path, pad } => write!(
                f,
                "co-sim path '{path}': the chip descriptor names no ADC input for pad '{pad}' \
                 (no `analog_pins:` entry); route adc.<peripheral>.<channel>_volts instead"
            ),
            Self::AdcUnavailable { path, channel } => write!(
                f,
                "co-sim path '{path}': no ADC on the bus accepted channel {channel}"
            ),
            Self::UnknownPeripheral { path, peripheral } => write!(
                f,
                "co-sim path '{path}': there is no peripheral named '{peripheral}' on this bus"
            ),
            Self::NotAnAdc { path, peripheral } => write!(
                f,
                "co-sim path '{path}': peripheral '{peripheral}' is not an ADC, so it takes no \
                 analog level"
            ),
            Self::NoSuchAdcChannel {
                path,
                peripheral,
                channel,
                channels,
            } => match channels.checked_sub(1) {
                Some(last) => write!(
                    f,
                    "co-sim path '{path}': ADC '{peripheral}' has channels 0..={last}; \
                     there is no channel {channel}"
                ),
                None => write!(
                    f,
                    "co-sim path '{path}': ADC '{peripheral}' has no analog input channels"
                ),
            },
            Self::DriveRejected { path } => write!(
                f,
                "co-sim path '{path}': the owning GPIO block refused an external level"
            ),
            Self::TypeMismatch { path, value } => write!(
                f,
                "co-sim path '{path}': cannot route signal value {value:?} (expected a number \
                 for an analog path, a boolean or volts for a pin path)"
            ),
        }
    }
}

impl std::error::Error for RoutingError {}

/// A resolved machine → store source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadBinding {
    /// Owning peripheral index + bit, read through `Peripheral::read_gpio_output`.
    Output { peripheral: usize, bit: u8 },
    /// Owning peripheral index + pad number, read through
    /// `Peripheral::read_gpio_is_output`.
    Direction { peripheral: usize, pad: u8 },
    /// Owning peripheral index + bit, read through `Peripheral::read_gpio_input`.
    Input { peripheral: usize, bit: u8 },
}

/// A resolved store → machine sink.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WriteBinding {
    /// `(IDR address, bit)` driven through `SystemBus::drive_input_bit` — the
    /// same seam a `board_io` button and a sensor status line come through.
    /// `level` is the pad's Schmitt state: what it read last, which a voltage
    /// between VIL and VIH keeps. Low until a model says otherwise.
    PadInput { addr: u64, bit: u8, level: bool },
    /// An ADC channel seeded through `SystemBus::seed_adc_channel`, on the
    /// peripheral the manifest or the chip descriptor named.
    AdcChannel { connection: String, channel: u8 },
}

/// Routes manifest signal paths between a running machine and the co-sim
/// signal store.
#[derive(Debug, Default, Clone)]
pub struct SignalRouter {
    reads: Vec<(String, ReadBinding)>,
    writes: Vec<(String, WriteBinding)>,
    /// The chip's input thresholds, or the descriptor key that is missing.
    /// `None` only on a router built by `Default`, which binds nothing.
    thresholds: Option<Result<InputThresholds, &'static str>>,
}

impl SignalRouter {
    /// Resolve every board path named by `configs` against `bus`.
    ///
    /// Returns the router plus every path that could not be resolved. Binding
    /// is deterministic: the manifest maps are `HashMap`s, so paths are
    /// collected through a `BTreeSet` and the resulting order is the sorted
    /// path order, not a hash order that changes between processes.
    pub fn bind(configs: &[CosimModelConfig], bus: &SystemBus) -> (Self, Vec<RoutingError>) {
        let mut sources: BTreeSet<&str> = BTreeSet::new();
        let mut sinks: BTreeSet<&str> = BTreeSet::new();
        // Sinks some model is known to write volts to. Known up front for the
        // in-core analog engine (every probe is a voltage or a current) and a
        // mock's static outputs; an external process only says at run time,
        // which `apply` checks.
        let mut volts_sinks: BTreeSet<&str> = BTreeSet::new();
        for config in configs {
            sources.extend(config.inputs.values().map(String::as_str));
            for (model_signal, sink) in &config.outputs {
                sinks.insert(sink);
                if output_carries_volts(config, model_signal) {
                    volts_sinks.insert(sink);
                }
            }
        }

        let thresholds = InputThresholds::for_bus(bus);
        let mut router = Self {
            thresholds: Some(thresholds),
            ..Self::default()
        };
        let mut errors = Vec::new();

        for path in &sources {
            let Some(parsed) = SignalPath::parse(path) else {
                continue; // Plain store path — the runner owns it.
            };
            if !parsed.is_readable() {
                errors.push(RoutingError::NotReadable {
                    path: (*path).to_string(),
                });
                continue;
            }
            match Self::resolve_read(&parsed, bus, path) {
                Ok(binding) => router.reads.push(((*path).to_string(), binding)),
                Err(err) => errors.push(err),
            }
        }

        for path in &sinks {
            if path.starts_with(UI_SIGNAL_PREFIX) {
                errors.push(RoutingError::ExternalOnly {
                    path: (*path).to_string(),
                });
                continue;
            }
            let Some(parsed) = SignalPath::parse(path) else {
                continue;
            };
            if !parsed.is_writable() {
                errors.push(RoutingError::NotWritable {
                    path: (*path).to_string(),
                });
                continue;
            }
            if let (SignalPath::GpioInput { .. }, Err(key)) = (&parsed, thresholds) {
                if volts_sinks.contains(path) {
                    errors.push(RoutingError::NoInputThresholds {
                        path: (*path).to_string(),
                        key,
                    });
                    continue;
                }
            }
            match Self::resolve_write(&parsed, bus, path) {
                Ok(binding) => router.writes.push(((*path).to_string(), binding)),
                Err(err) => errors.push(err),
            }
        }

        (router, errors)
    }

    fn resolve_read(
        parsed: &SignalPath,
        bus: &SystemBus,
        path: &str,
    ) -> Result<ReadBinding, RoutingError> {
        match parsed {
            SignalPath::GpioOutput { pad } => {
                let (peripheral, bit) = resolve_pad_owner_odr(bus, pad, path)?;
                Ok(ReadBinding::Output { peripheral, bit })
            }
            SignalPath::GpioDirection { pad } => {
                let (addr, bit) = SystemBus::resolve_pin_odr(bus, pad).ok_or_else(|| {
                    RoutingError::UnknownPad {
                        path: path.to_string(),
                        pad: pad.clone(),
                    }
                })?;
                let peripheral =
                    bus.find_peripheral_index(addr)
                        .ok_or_else(|| RoutingError::UnknownGpio {
                            path: path.to_string(),
                            pad: pad.clone(),
                        })?;
                let owner = &bus.peripherals[peripheral];
                let pad_number = pad_number(owner.dev.as_ref(), addr - owner.base, bit);
                direction_binding(owner.dev.as_ref(), peripheral, pad_number, path, pad)
            }
            SignalPath::GpioInput { pad } => {
                let (peripheral, bit) = resolve_pad_owner_idr(bus, pad, path)?;
                Ok(ReadBinding::Input { peripheral, bit })
            }
            // `is_readable` already rejected the analog forms.
            SignalPath::AnalogPad { .. } | SignalPath::AdcChannel { .. } => {
                Err(RoutingError::NotReadable {
                    path: path.to_string(),
                })
            }
        }
    }

    fn resolve_write(
        parsed: &SignalPath,
        bus: &SystemBus,
        path: &str,
    ) -> Result<WriteBinding, RoutingError> {
        match parsed {
            SignalPath::GpioInput { pad } => {
                let (addr, bit) = SystemBus::resolve_pin_idr(bus, pad).ok_or_else(|| {
                    RoutingError::UnknownPad {
                        path: path.to_string(),
                        pad: pad.clone(),
                    }
                })?;
                Ok(WriteBinding::PadInput {
                    addr,
                    bit,
                    level: false,
                })
            }
            SignalPath::AnalogPad { pad } => {
                // Descriptor data only. The pad → channel assignment differs
                // between families (PA0 is ADC1_IN0 on an F401, ADC1_IN5 on an
                // L476, ADC1_IN1 on a G474), so a chip that records nothing
                // gets an error, never a guess that reads the wrong channel.
                let (connection, channel) = bus
                    .analog_pin_map
                    .get(&pad.to_ascii_uppercase())
                    .cloned()
                    .ok_or_else(|| RoutingError::NoAdcChannel {
                        path: path.to_string(),
                        pad: pad.clone(),
                    })?;
                check_adc_channel(bus, path, &connection, channel)?;
                Ok(WriteBinding::AdcChannel {
                    connection,
                    channel,
                })
            }
            SignalPath::AdcChannel {
                peripheral,
                channel,
            } => {
                check_adc_channel(bus, path, peripheral, *channel)?;
                Ok(WriteBinding::AdcChannel {
                    connection: peripheral.clone(),
                    channel: *channel,
                })
            }
            SignalPath::GpioOutput { .. } | SignalPath::GpioDirection { .. } => {
                Err(RoutingError::NotWritable {
                    path: path.to_string(),
                })
            }
        }
    }

    /// No board path is routed in either direction.
    pub fn is_empty(&self) -> bool {
        self.reads.is_empty() && self.writes.is_empty()
    }

    /// The store paths this router fills from the machine, in bind order.
    pub fn read_paths(&self) -> impl Iterator<Item = &str> {
        self.reads.iter().map(|(path, _)| path.as_str())
    }

    /// Machine → store. Called immediately before stepping the models, so a
    /// model sees the pin levels as of the boundary it is stepped to.
    ///
    /// A pad the GPIO model cannot answer for (an alternate-function pin on a
    /// block that tracks direction, say) leaves its path ABSENT rather than
    /// inserting `false`: the runner then passes no value for that input, and
    /// the model keeps whatever it had, instead of being told the pin is low.
    pub fn sample(&self, bus: &SystemBus, signals: &mut CosimSignals) {
        for (path, binding) in &self.reads {
            let level = match *binding {
                ReadBinding::Output { peripheral, bit } => bus
                    .peripherals
                    .get(peripheral)
                    .and_then(|p| p.dev.read_gpio_output(bit)),
                ReadBinding::Input { peripheral, bit } => bus
                    .peripherals
                    .get(peripheral)
                    .and_then(|p| p.dev.read_gpio_input(bit)),
                ReadBinding::Direction { peripheral, pad } => bus
                    .peripherals
                    .get(peripheral)
                    .and_then(|p| p.dev.read_gpio_is_output(pad)),
            };
            if let Some(level) = level {
                signals.insert(path.clone(), CosimSignalValue::Bool(level));
            }
        }
    }

    /// Store → machine. Called immediately after stepping the models.
    ///
    /// A pin path takes a boolean as the level, exactly as it always has, and
    /// a number of volts through the chip's [`InputThresholds`], keeping the
    /// pad's Schmitt state between calls.
    pub fn apply(&mut self, bus: &mut SystemBus, signals: &CosimSignals) -> Vec<RoutingError> {
        let mut errors = Vec::new();
        for (path, binding) in &mut self.writes {
            let Some(value) = signals.get(path.as_str()) else {
                continue; // No model produced this output on this step.
            };
            match binding {
                WriteBinding::PadInput { addr, bit, level } => {
                    let next = match value {
                        CosimSignalValue::F64(volts) => match self.thresholds {
                            Some(Ok(thresholds)) => thresholds.next_level(*level, *volts),
                            Some(Err(key)) => {
                                errors.push(RoutingError::NoInputThresholds {
                                    path: path.clone(),
                                    key,
                                });
                                continue;
                            }
                            None => {
                                errors.push(RoutingError::NoInputThresholds {
                                    path: path.clone(),
                                    key: "gpio_input_thresholds",
                                });
                                continue;
                            }
                        },
                        other => match signal_as_bool(other) {
                            Some(next) => next,
                            None => {
                                errors.push(RoutingError::TypeMismatch {
                                    path: path.clone(),
                                    value: value.clone(),
                                });
                                continue;
                            }
                        },
                    };
                    *level = next;
                    if !bus.drive_input_bit(*addr, *bit, next) {
                        errors.push(RoutingError::DriveRejected { path: path.clone() });
                    }
                }
                WriteBinding::AdcChannel {
                    connection,
                    channel,
                } => {
                    let Some(volts) = signal_as_f64(value) else {
                        errors.push(RoutingError::TypeMismatch {
                            path: path.clone(),
                            value: value.clone(),
                        });
                        continue;
                    };
                    let millivolts = volts_to_millivolts(volts);
                    if !bus.seed_adc_channel(connection, *channel, millivolts) {
                        errors.push(RoutingError::AdcUnavailable {
                            path: path.clone(),
                            channel: *channel,
                        });
                    }
                }
            }
        }
        errors
    }
}

/// Refuse an ADC route the converter cannot take: a peripheral the bus does
/// not have, one that is not an ADC, or a channel that ADC does not have.
///
/// Checked at bind time against the named peripheral itself. The apply path
/// cannot catch these: an ADC model silently drops a channel it does not
/// have, so the route would step every period and land nowhere.
fn check_adc_channel(
    bus: &SystemBus,
    path: &str,
    peripheral: &str,
    channel: u8,
) -> Result<(), RoutingError> {
    let index = bus
        .find_peripheral_index_by_name(peripheral)
        .ok_or_else(|| RoutingError::UnknownPeripheral {
            path: path.to_string(),
            peripheral: peripheral.to_string(),
        })?;
    let channels = bus.peripherals[index]
        .dev
        .adc_channel_count()
        .ok_or_else(|| RoutingError::NotAnAdc {
            path: path.to_string(),
            peripheral: peripheral.to_string(),
        })?;
    if channel >= channels {
        return Err(RoutingError::NoSuchAdcChannel {
            path: path.to_string(),
            peripheral: peripheral.to_string(),
            channel,
            channels,
        });
    }
    Ok(())
}

/// Clamp a routed node voltage to the millivolt level an ADC model takes.
///
/// The ADC owns millivolts → counts (see [`ADC_VREF_VOLTS`]), so this is only
/// the unit change plus the clamp a real pin imposes: a SPICE node can ring
/// below ground or above the rail, and a pin cannot present either to the
/// converter. Rounds half away from zero, which is deterministic.
///
/// `NaN` is the one input with no honest answer — it is not high, low, or
/// anywhere between — so it reads as 0 rather than being fed to a comparison
/// that would quietly answer `false` in both directions.
pub fn volts_to_millivolts(volts: f64) -> u16 {
    if volts.is_nan() {
        return 0;
    }
    (volts.clamp(0.0, ADC_VREF_VOLTS) * 1000.0).round() as u16
}

/// A non-volts pin value as a level. `F64` never reaches here: on a pin path
/// it is volts, which go through [`InputThresholds::next_level`].
fn signal_as_bool(value: &CosimSignalValue) -> Option<bool> {
    match value {
        CosimSignalValue::Bool(value) => Some(*value),
        CosimSignalValue::I64(value) => Some(*value != 0),
        CosimSignalValue::F64(_) | CosimSignalValue::Text(_) => None,
    }
}

/// Is `model_signal` known, before the run, to carry volts?
///
/// The in-core analog engine's outputs are probe readings, always numbers. A
/// mock's are its static `config.outputs`, read with the same rule the mock
/// adapter uses (an integer is an `I64`, anything else numeric an `F64`). Any
/// other adapter only says at run time.
fn output_carries_volts(config: &CosimModelConfig, model_signal: &str) -> bool {
    match config.adapter {
        ManifestCosimAdapter::Analog => true,
        ManifestCosimAdapter::Mock => matches!(
            config
                .config
                .get("outputs")
                .and_then(|outputs| outputs.get(model_signal)),
            Some(serde_yaml::Value::Number(number))
                if number.as_i64().is_none() && number.as_f64().is_some()
        ),
        ManifestCosimAdapter::ExternalProcess | ManifestCosimAdapter::Fmi => false,
    }
}

fn signal_as_f64(value: &CosimSignalValue) -> Option<f64> {
    match value {
        CosimSignalValue::F64(value) => Some(*value),
        CosimSignalValue::I64(value) => Some(*value as f64),
        // A boolean on an analog path is a pin level, not a voltage: it would
        // silently become 0 V / 0.001 V. Reject it so the manifest says what
        // it means.
        CosimSignalValue::Bool(_) | CosimSignalValue::Text(_) => None,
    }
}

/// Resolve a pad label to `(owning peripheral index, bit)` through its OUTPUT
/// register, through the same pin resolution MMIO routing and every other
/// pad-addressed feature already use: the chip's declared `pins:` map first,
/// then the STM32/Nordic label parse, then the ESP32 GPIO forms.
fn resolve_pad_owner_odr(
    bus: &SystemBus,
    pad: &str,
    path: &str,
) -> Result<(usize, u8), RoutingError> {
    let (addr, bit) =
        SystemBus::resolve_pin_odr(bus, pad).ok_or_else(|| RoutingError::UnknownPad {
            path: path.to_string(),
            pad: pad.to_string(),
        })?;
    let idx = bus
        .find_peripheral_index(addr)
        .ok_or_else(|| RoutingError::UnknownGpio {
            path: path.to_string(),
            pad: pad.to_string(),
        })?;
    Ok((idx, bit))
}

/// Register offset of the ESP32 family's second output bank (`GPIO_OUT1`),
/// where [`SystemBus::resolve_pin_odr`] places pads 32 and up as bank-relative
/// bits.
const ESP32_GPIO_OUT1_OFFSET: u64 = 0x10;

/// The pad NUMBER an `(ODR address, bit)` resolution names.
///
/// A GPIO port's bit is its pad (`Peripheral::gpio_port_offsets` says so). The
/// ESP32 family's single `gpio` block is the exception: its resolver splits
/// pads 32.. into `GPIO_OUT1` as bank-relative bits, while
/// `read_gpio_is_output` numbers pads absolutely. Handing it the bank bit
/// would report GPIO1's direction for GPIO33.
fn pad_number(owner: &dyn Peripheral, register_offset: u64, bit: u8) -> u8 {
    let is_port = owner.gpio_port_offsets().is_some();
    if !is_port && register_offset == ESP32_GPIO_OUT1_OFFSET {
        bit.saturating_add(32)
    } else {
        bit
    }
}

/// Bind `board.gpio_output.<pad>` to its owner, or refuse a GPIO model that
/// cannot say which way the pad points.
///
/// Checked here, once, because the alternative is worse than an error: a pad
/// whose direction reads as absent would never close the circuit's driver, and
/// one read as `false` would silently disconnect the firmware from the net for
/// the whole run.
fn direction_binding(
    owner: &dyn Peripheral,
    peripheral: usize,
    pad_number: u8,
    path: &str,
    pad: &str,
) -> Result<ReadBinding, RoutingError> {
    if owner.read_gpio_is_output(pad_number).is_none() {
        return Err(RoutingError::NoDirection {
            path: path.to_string(),
            pad: pad.to_string(),
        });
    }
    Ok(ReadBinding::Direction {
        peripheral,
        pad: pad_number,
    })
}

/// [`resolve_pad_owner_odr`]'s input-register twin.
fn resolve_pad_owner_idr(
    bus: &SystemBus,
    pad: &str,
    path: &str,
) -> Result<(usize, u8), RoutingError> {
    let (addr, bit) =
        SystemBus::resolve_pin_idr(bus, pad).ok_or_else(|| RoutingError::UnknownPad {
            path: path.to_string(),
            pad: pad.to_string(),
        })?;
    let idx = bus
        .find_peripheral_index(addr)
        .ok_or_else(|| RoutingError::UnknownGpio {
            path: path.to_string(),
            pad: pad.to_string(),
        })?;
    Ok((idx, bit))
}

/// A co-simulation bound to one machine: the runner, the pin routing, the
/// signal store, and the cycle ↔ nanosecond time base that keeps them in
/// lockstep.
///
/// The run loop that owns the machine only has to ask two things — how far may
/// I run ([`Self::cycles_until_boundary`]), and here is where I got to
/// ([`Self::advance_to`]) — so the lockstep rule lives in one place rather than
/// being re-derived by every caller.
pub struct CosimSession {
    runner: CosimRunner,
    router: SignalRouter,
    signals: CosimSignals,
    cpu_hz: u64,
    /// The finest model period: the granularity the machine is chopped at.
    step_ns: u64,
    /// Simulated time of the next boundary the machine must not run past.
    next_boundary_ns: u64,
    /// True when `cpu_hz` came from [`FALLBACK_CPU_HZ`] rather than the bus.
    fallback_clock: bool,
    /// Paths the manifest named that could not be resolved against this bus.
    binding_errors: Vec<RoutingError>,
    /// Every apply-time routing failure [`Self::advance`] has already handed
    /// back, by message, so each distinct one is reported once per run.
    reported_errors: BTreeSet<String>,
}

/// What one lockstep advance did: the machine's report, and what happened at
/// the model boundaries it reached.
#[derive(Debug, Clone)]
pub struct CosimAdvance {
    /// The machine's own accounting. For [`CosimSession::advance_budget`] it
    /// sums every chunk, and `stop` is the stop that ended the last one.
    pub report: AdvanceReport,
    /// Every model step taken at a boundary this advance reached, in order.
    /// Empty when no boundary was reached.
    pub routed: Vec<CosimRoutedModelStep>,
    /// Apply-time routing failures seen for the FIRST time. A failure an
    /// earlier advance already returned is not repeated: at a 100 us step a
    /// broken ADC route would otherwise be one identical line per period.
    pub new_routing_errors: Vec<RoutingError>,
}

impl From<AdvanceReport> for CosimAdvance {
    /// An advance that reached no model: the shape a caller with no session
    /// hands on, so one code path can handle both.
    fn from(report: AdvanceReport) -> Self {
        Self {
            report,
            routed: Vec::new(),
            new_routing_errors: Vec::new(),
        }
    }
}

/// Why a lockstep advance failed.
#[derive(Debug)]
pub enum CosimAdvanceError {
    /// The machine itself failed. No model was stepped for the failing chunk,
    /// and — as with [`Machine::advance`] — the CPU may already have retired
    /// part of its batch.
    Machine(SimulationError),
    /// The machine advanced, then a model failed at the boundary it reached.
    /// `report` accounts for the machine work that did commit.
    Model {
        report: AdvanceReport,
        error: SimulationError,
    },
}

impl std::fmt::Display for CosimAdvanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Machine(error) => write!(f, "{error}"),
            Self::Model { error, .. } => write!(f, "co-sim model step failed: {error}"),
        }
    }
}

impl std::error::Error for CosimAdvanceError {}

/// `total` followed by `next`: counters add, the stop is `next`'s.
fn chain_reports(total: AdvanceReport, next: AdvanceReport) -> AdvanceReport {
    AdvanceReport::new(
        next.stop,
        total.fuel_consumed + next.fuel_consumed,
        total.primary_steps + next.primary_steps,
        total.secondary_steps + next.secondary_steps,
        total.elapsed_cycles + next.elapsed_cycles,
        total.idle_cycles + next.idle_cycles,
        total.cpu_batches + next.cpu_batches,
    )
}

impl CosimSession {
    /// Build a session for `configs`, resolving relative `model:` paths against
    /// `base_dir` and every board path against `bus`.
    ///
    /// Returns `Ok(None)` when `configs` is empty — the caller then does
    /// nothing at all, which is what keeps a manifest without `cosim_models`
    /// byte-identical to a build without this feature.
    ///
    /// Unresolvable paths are NOT an error here: they are carried on the
    /// session as [`Self::binding_errors`] so the caller decides what an
    /// unroutable pin means for its run.
    pub fn new(
        configs: &[CosimModelConfig],
        base_dir: &Path,
        bus: &SystemBus,
    ) -> SimResult<Option<Self>> {
        if configs.is_empty() {
            return Ok(None);
        }
        let runner = CosimRunner::from_configs_with_base(configs, base_dir)?;
        let (router, binding_errors) = SignalRouter::bind(configs, bus);
        let fallback_clock = bus.cpu_hz == 0;
        let cpu_hz = if fallback_clock {
            FALLBACK_CPU_HZ
        } else {
            bus.cpu_hz
        };
        // The finest declared period is the lockstep granularity: stepping at
        // anything coarser would let the machine run past a faster model's
        // boundary before that model saw the pin levels that produced it.
        let step_ns = configs
            .iter()
            .map(|config| config.step_ns)
            .filter(|step| *step > 0)
            .min()
            .unwrap_or(1);
        // Every `ui.` path a model reads exists from the first step, at 0 /
        // false, so a circuit sees "released" before anyone has touched it
        // rather than whatever DC value its netlist happened to write.
        let mut signals = CosimSignals::new();
        for path in runner.input_paths() {
            if path.starts_with(UI_SIGNAL_PREFIX) {
                let initial = match runner.input_kind(path) {
                    Some(CosimInputKind::Bool) => CosimSignalValue::Bool(false),
                    Some(CosimInputKind::Number) | None => CosimSignalValue::F64(0.0),
                };
                signals.insert(path.to_string(), initial);
            }
        }
        Ok(Some(Self {
            runner,
            router,
            signals,
            cpu_hz,
            step_ns,
            next_boundary_ns: step_ns,
            fallback_clock,
            binding_errors,
            reported_errors: BTreeSet::new(),
        }))
    }

    /// Every manifest path that did not resolve against the machine's bus.
    pub fn binding_errors(&self) -> &[RoutingError] {
        &self.binding_errors
    }

    /// How many models this session steps.
    pub fn model_count(&self) -> usize {
        self.runner.model_count()
    }

    /// The runner's analog waveform ring. Every `adapter: analog` model writes
    /// its routed outputs and `config.trace` channels here as it steps; publish
    /// it with [`Machine::attach_analog_trace`](crate::Machine::attach_analog_trace)
    /// so `Machine::analog_trace_snapshot` and `--analog-trace` read the samples
    /// this session produces. A session with no analog model hands back a ring
    /// with no channels.
    pub fn analog_trace_registry(&self) -> crate::analog::AnalogTraceRegistry {
        self.runner.analog_trace_registry()
    }

    /// The effective core clock, in Hz.
    pub fn cpu_hz(&self) -> u64 {
        self.cpu_hz
    }

    /// Whether [`FALLBACK_CPU_HZ`] stood in for a bus that reported no clock.
    pub fn uses_fallback_clock(&self) -> bool {
        self.fallback_clock
    }

    /// The lockstep granularity, in nanoseconds (the finest model `step_ns`).
    pub fn step_ns(&self) -> u64 {
        self.step_ns
    }

    /// The signal store, for inspection after a step.
    pub fn signals(&self) -> &CosimSignals {
        &self.signals
    }

    /// Set a signal from outside the engine: a canvas part (`ui.touch.pressed`
    /// while a finger is on the pad), or a test script's `cosim_signal`
    /// stimulus.
    ///
    /// The value lands in the store now; every model that reads `path` sees it
    /// from its next step on. It stays until set again.
    ///
    /// `path` must be one some model's `inputs:` reads — a name no model reads
    /// is almost always a typo, and setting it would change nothing while
    /// reporting success. A board path is refused too: the machine rewrites it
    /// at every boundary, so the value would not survive to the next step.
    pub fn set_signal(&mut self, path: &str, value: CosimSignalValue) -> Result<(), RoutingError> {
        if SignalPath::parse(path).is_some() {
            return Err(RoutingError::NotSettable {
                path: path.to_string(),
            });
        }
        if !self.runner.reads_path(path) {
            return Err(RoutingError::UnknownSignal {
                path: path.to_string(),
            });
        }
        self.signals.insert(path.to_string(), value);
        Ok(())
    }

    /// [`Self::set_signal`] for a caller that only has a number — the browser
    /// bridge and a test script's `value:`.
    ///
    /// For an input its models call a logic level ([`CosimInputKind::Bool`],
    /// e.g. an analog switch control) 0 is false and anything else true, so a
    /// press is `1` whatever the circuit's supply. Every other input takes the
    /// number as given.
    pub fn set_signal_number(&mut self, path: &str, value: f64) -> Result<(), RoutingError> {
        if !value.is_finite() {
            return Err(RoutingError::NotFinite {
                path: path.to_string(),
                value,
            });
        }
        let value = match self.runner.input_kind(path) {
            Some(CosimInputKind::Bool) => CosimSignalValue::Bool(value != 0.0),
            Some(CosimInputKind::Number) | None => CosimSignalValue::F64(value),
        };
        self.set_signal(path, value)
    }

    /// The machine-sourced values the models were last handed, in bind order —
    /// the readout that answers "what did the model actually see on that pin?"
    /// without re-reading the bus. Empty before the first boundary.
    pub fn sampled_inputs(&self) -> Vec<(&str, &CosimSignalValue)> {
        self.router
            .read_paths()
            .filter_map(|path| self.signals.get_key_value(path))
            .map(|(path, value)| (path.as_str(), value))
            .collect()
    }

    /// How many more simulated cycles the machine may run before it would pass
    /// the next co-simulation boundary. Never zero, so a run always makes
    /// progress.
    pub fn cycles_until_boundary(&self, total_cycles: u64) -> u64 {
        let boundary = ns_to_cycles(self.next_boundary_ns, self.cpu_hz);
        boundary.saturating_sub(total_cycles).max(1)
    }

    /// Step every model whose boundary `total_cycles` has reached, sampling the
    /// machine's pins first and applying routed outputs back afterwards.
    ///
    /// Returns the routed steps (empty when no boundary was reached) and every
    /// routing error the apply pass hit.
    pub fn advance_to(
        &mut self,
        total_cycles: u64,
        bus: &mut SystemBus,
    ) -> SimResult<(Vec<CosimRoutedModelStep>, Vec<RoutingError>)> {
        let time_ns = cycles_to_ns(total_cycles, self.cpu_hz);
        if time_ns < self.next_boundary_ns {
            return Ok((Vec::new(), Vec::new()));
        }
        self.router.sample(bus, &mut self.signals);
        let routed = self
            .runner
            .step_until_with_signals(time_ns, &mut self.signals)?;
        let errors = self.router.apply(bus, &self.signals);
        // Land on the first boundary strictly after the time just reached, so
        // a long advance that crossed several periods does not replay them.
        self.next_boundary_ns = (time_ns / self.step_ns + 1).saturating_mul(self.step_ns);
        Ok((routed, errors))
    }

    /// One lockstep advance of `machine`: `request`, with its simulated-cycle
    /// budget capped at the next model boundary, then every model due at the
    /// point the machine reached.
    ///
    /// The cap is what keeps a model from being handed pin levels from its
    /// future. A fuel budget cannot express it — fuel counts scheduling quanta
    /// and idle skips, and one idle skip can cross milliseconds of device time
    /// — so the cap goes on `simulated_cycles`, the clock a boundary is defined
    /// in. A request that already carries a tighter cycle budget keeps it.
    ///
    /// Models are NOT stepped when the machine stopped for good or did not
    /// move: a firmware exit has no next instruction to hand a model's answer
    /// to, and a machine that made no progress reached no new time.
    ///
    /// This is the one place a machine is stepped in lockstep with its models.
    /// `labwired test` calls it once per run-loop iteration; a caller that
    /// wants a whole budget spent calls [`Self::advance_budget`], which is this
    /// in a loop.
    pub fn advance<C: Cpu>(
        &mut self,
        machine: &mut Machine<C>,
        request: AdvanceRequest,
    ) -> Result<CosimAdvance, CosimAdvanceError> {
        let to_boundary = self.cycles_until_boundary(machine.total_cycles);
        let cycle_limit = request
            .limits()
            .simulated_cycles
            .map_or(to_boundary, |limit| limit.min(to_boundary));
        let report = machine
            .advance(request.with_cycle_limit(cycle_limit))
            .map_err(CosimAdvanceError::Machine)?;

        let stopped_for_good = matches!(report.stop, AdvanceStop::FirmwareExit { .. });
        let made_no_progress = report.primary_steps == 0 && report.idle_cycles == 0;
        if stopped_for_good || made_no_progress {
            return Ok(report.into());
        }

        let (routed, errors) = self
            .advance_to(machine.total_cycles, &mut machine.bus)
            .map_err(|error| CosimAdvanceError::Model { report, error })?;
        let new_routing_errors = errors
            .into_iter()
            .filter(|err| self.reported_errors.insert(err.to_string()))
            .collect();
        Ok(CosimAdvance {
            report,
            routed,
            new_routing_errors,
        })
    }

    /// Spend `request`'s whole fuel and cycle budget in lockstep: repeated
    /// [`Self::advance`] calls, each stopped on a model boundary, until the
    /// budget is spent or the machine stops for any other reason (breakpoint,
    /// no progress, firmware exit).
    ///
    /// The returned report is what one [`Machine::advance`] with the same
    /// request would account — fuel, steps and cycles summed over the chunks —
    /// which is what lets a caller like the browser's `step_batch` keep its
    /// contract whether or not a session is attached. A request with neither
    /// budget runs until the machine stops on its own, exactly as
    /// [`Machine::advance`] would.
    pub fn advance_budget<C: Cpu>(
        &mut self,
        machine: &mut Machine<C>,
        request: AdvanceRequest,
    ) -> Result<CosimAdvance, CosimAdvanceError> {
        let limits = request.limits();
        let mut total: Option<CosimAdvance> = None;
        loop {
            let (fuel_spent, cycles_spent) = total.as_ref().map_or((0, 0), |advance| {
                (advance.report.fuel_consumed, advance.report.elapsed_cycles)
            });
            let mut chunk =
                request.with_fuel_limit(limits.fuel.map(|fuel| fuel.saturating_sub(fuel_spent)));
            let cycles_left = limits
                .simulated_cycles
                .map(|cycles| cycles.saturating_sub(cycles_spent));
            if let Some(cycles) = cycles_left {
                chunk = chunk.with_cycle_limit(cycles);
            }

            let step = match self.advance(machine, chunk) {
                Ok(step) => step,
                Err(CosimAdvanceError::Model { report, error }) => {
                    let report = match &total {
                        Some(advance) => chain_reports(advance.report, report),
                        None => report,
                    };
                    return Err(CosimAdvanceError::Model { report, error });
                }
                Err(machine_error) => return Err(machine_error),
            };

            let chunk_report = step.report;
            total = Some(match total {
                None => step,
                Some(mut advance) => {
                    advance.report = chain_reports(advance.report, step.report);
                    advance.routed.extend(step.routed);
                    advance.new_routing_errors.extend(step.new_routing_errors);
                    advance
                }
            });

            let budget_spent = match chunk_report.stop {
                // The chunk's fuel was everything that was left.
                AdvanceStop::FuelLimit => true,
                // Either the request's own cycle budget, or just a boundary.
                AdvanceStop::CycleLimit => {
                    cycles_left.is_some_and(|left| chunk_report.elapsed_cycles >= left)
                }
                AdvanceStop::Breakpoint(_)
                | AdvanceStop::NoProgress
                | AdvanceStop::FirmwareExit { .. } => true,
            };
            let made_no_progress = chunk_report.primary_steps == 0 && chunk_report.idle_cycles == 0;
            if budget_spent || made_no_progress {
                return Ok(total.expect("at least one chunk ran"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::{CosimAdapter, CosimModelConfig};
    use std::collections::HashMap;

    fn mock_model(
        id: &str,
        step_ns: u64,
        inputs: &[(&str, &str)],
        outputs: &[(&str, &str)],
        static_outputs: &[(&str, serde_yaml::Value)],
    ) -> CosimModelConfig {
        let mut config = HashMap::new();
        if !static_outputs.is_empty() {
            let mapping: serde_yaml::Mapping = static_outputs
                .iter()
                .map(|(k, v)| (serde_yaml::Value::String((*k).to_string()), v.clone()))
                .collect();
            config.insert("outputs".to_string(), serde_yaml::Value::Mapping(mapping));
        }
        CosimModelConfig {
            id: id.to_string(),
            adapter: CosimAdapter::Mock,
            model: None,
            step_ns,
            inputs: inputs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            outputs: outputs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            config,
        }
    }

    // ── Path grammar ────────────────────────────────────────────────────────

    #[test]
    fn parses_every_board_path_form() {
        assert_eq!(
            SignalPath::parse("board.gpio.pa5"),
            Some(SignalPath::GpioOutput {
                pad: "pa5".to_string()
            })
        );
        assert_eq!(
            SignalPath::parse("board.gpio_in.pc13"),
            Some(SignalPath::GpioInput {
                pad: "pc13".to_string()
            })
        );
        assert_eq!(
            SignalPath::parse("board.analog.pa0_volts"),
            Some(SignalPath::AnalogPad {
                pad: "pa0".to_string()
            })
        );
        assert_eq!(
            SignalPath::parse("adc.adc1.3_volts"),
            Some(SignalPath::AdcChannel {
                peripheral: "adc1".to_string(),
                channel: 3
            })
        );
    }

    #[test]
    fn parses_the_direction_path() {
        assert_eq!(
            SignalPath::parse("board.gpio_output.pa5"),
            Some(SignalPath::GpioDirection {
                pad: "pa5".to_string()
            })
        );
        assert_eq!(SignalPath::parse("board.gpio_output."), None);
        let direction = SignalPath::parse("board.gpio_output.p0.13").unwrap();
        assert_eq!(
            direction,
            SignalPath::GpioDirection {
                pad: "p0.13".to_string()
            }
        );
        assert!(direction.is_readable());
        assert!(!direction.is_writable());
    }

    /// A GPIO model with an output latch but no direction register. The
    /// direction path must refuse it when the session is built: answering
    /// `false` would disconnect the firmware from the circuit for the whole run
    /// and nothing would say so.
    #[derive(Debug, Default)]
    struct LatchOnlyGpio;

    impl Peripheral for LatchOnlyGpio {
        fn read(&self, _offset: u64) -> SimResult<u8> {
            Ok(0)
        }
        fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
            Ok(())
        }
        fn read_gpio_output(&self, pin: u8) -> Option<bool> {
            (pin < 32).then_some(false)
        }
    }

    #[test]
    fn a_gpio_model_without_a_direction_register_is_refused() {
        let err = direction_binding(&LatchOnlyGpio, 3, 5, "board.gpio_output.pa5", "pa5")
            .expect_err("no direction register, no binding");
        assert_eq!(
            err,
            RoutingError::NoDirection {
                path: "board.gpio_output.pa5".to_string(),
                pad: "pa5".to_string(),
            }
        );
        let message = err.to_string();
        assert!(message.contains("board.gpio_output.pa5"), "{message}");
        assert!(message.contains("direction"), "{message}");
    }

    /// Only the ESP32 family's second output bank shifts the bit. A SAM port's
    /// OUT register also sits at 0x10, and its bit IS the pad.
    #[test]
    fn the_second_esp32_output_bank_names_pads_from_32() {
        assert_eq!(pad_number(&LatchOnlyGpio, ESP32_GPIO_OUT1_OFFSET, 1), 33);
        assert_eq!(pad_number(&LatchOnlyGpio, 0x04, 1), 1);
        let port = crate::peripherals::gpio::GpioPort::new_with_layout(
            crate::peripherals::gpio::GpioRegisterLayout::SamPort,
        );
        assert_eq!(pad_number(&port, 0x10, 1), 1);
    }

    #[test]
    fn a_direction_path_cannot_be_driven_by_a_model() {
        let bus = crate::bus::SystemBus::new();
        let configs = [mock_model(
            "m",
            1_000,
            &[],
            &[("out", "board.gpio_output.pa5")],
            &[("out", serde_yaml::Value::Bool(true))],
        )];
        let (_, errors) = SignalRouter::bind(&configs, &bus);
        assert_eq!(
            errors,
            vec![RoutingError::NotWritable {
                path: "board.gpio_output.pa5".to_string()
            }]
        );
    }

    /// A Nordic pad label carries a dot. Splitting the path on every dot would
    /// truncate `p0.13` to `p0`, which resolves to a DIFFERENT pin (bit 0 of
    /// the same port) rather than failing — so the grammar strips a fixed
    /// prefix and hands the whole remainder to pin resolution.
    #[test]
    fn keeps_dotted_nordic_pad_labels_intact() {
        assert_eq!(
            SignalPath::parse("board.gpio.p0.13"),
            Some(SignalPath::GpioOutput {
                pad: "p0.13".to_string()
            })
        );
    }

    #[test]
    fn plain_store_paths_are_not_board_paths() {
        // The `cosim-plant-demo` manifest shape must keep working untouched.
        assert_eq!(SignalPath::parse("control.enable"), None);
        assert_eq!(SignalPath::parse("plant.output.voltage"), None);
        assert_eq!(SignalPath::parse("observables.shaft_speed_rpm"), None);
        // Board-ish but not the grammar: no pad, no `_volts` suffix.
        assert_eq!(SignalPath::parse("board.gpio."), None);
        assert_eq!(SignalPath::parse("board.analog.pa0"), None);
        assert_eq!(SignalPath::parse("adc.adc1_volts"), None);
        assert_eq!(SignalPath::parse("adc.adc1.x_volts"), None);
    }

    #[test]
    fn directions_are_enforced_per_path_kind() {
        assert!(SignalPath::parse("board.gpio.pa5").unwrap().is_readable());
        assert!(!SignalPath::parse("board.gpio.pa5").unwrap().is_writable());
        assert!(SignalPath::parse("board.gpio_in.pc13")
            .unwrap()
            .is_writable());
        assert!(SignalPath::parse("board.analog.pa0_volts")
            .unwrap()
            .is_writable());
        assert!(!SignalPath::parse("board.analog.pa0_volts")
            .unwrap()
            .is_readable());
    }

    // ── Pad → ADC channel ───────────────────────────────────────────────────

    /// A bus built from no descriptor records no analog pads. The pad form must
    /// refuse, and say which form to use instead — reading channel 0 would be
    /// the wrong pin on most families and nothing would report it.
    #[test]
    fn a_pad_without_descriptor_analog_data_is_refused() {
        let bus = crate::bus::SystemBus::new();
        let configs = [mock_model(
            "m",
            1_000,
            &[],
            &[("v", "board.analog.pa0_volts")],
            &[("v", serde_yaml::Value::from(1.0))],
        )];
        let (router, errors) = SignalRouter::bind(&configs, &bus);
        assert!(router.is_empty());
        assert_eq!(
            errors,
            vec![RoutingError::NoAdcChannel {
                path: "board.analog.pa0_volts".to_string(),
                pad: "pa0".to_string(),
            }]
        );
        assert!(errors[0]
            .to_string()
            .contains("adc.<peripheral>.<channel>_volts"));
    }

    // ── Volts → the ADC's millivolts ────────────────────────────────────────

    /// Half of a 3.3 V reference. The ADC model turns 1650 mV into
    /// `1650 * 4095 / 3300` = 2047 counts at 12 bits — the same count
    /// `examples/ntc-thermistor-lab` asserts for its divider midpoint, which is
    /// the point of leaving the conversion in the ADC instead of redoing it
    /// here with a second rounding rule.
    #[test]
    fn converts_volts_to_the_adc_millivolt_unit() {
        assert_eq!(volts_to_millivolts(1.65), 1650);
        assert_eq!(volts_to_millivolts(0.0), 0);
        assert_eq!(volts_to_millivolts(3.3), 3300);
        assert_eq!(volts_to_millivolts(2.0805), 2081);
    }

    /// A SPICE node can ring below ground or above the rail; a pin cannot
    /// present that to the converter.
    #[test]
    fn clamps_voltages_a_pin_could_not_present() {
        assert_eq!(volts_to_millivolts(-1.0), 0);
        assert_eq!(volts_to_millivolts(12.0), 3300);
        assert_eq!(volts_to_millivolts(f64::NAN), 0);
        assert_eq!(volts_to_millivolts(f64::INFINITY), 3300);
    }

    // ── Time base ───────────────────────────────────────────────────────────

    #[test]
    fn converts_cycles_and_nanoseconds_at_the_core_clock() {
        // 84 MHz: 100 us is 8400 cycles.
        assert_eq!(cycles_to_ns(8_400, 84_000_000), 100_000);
        assert_eq!(ns_to_cycles(100_000, 84_000_000), 8_400);
        assert_eq!(cycles_to_ns(0, 84_000_000), 0);
    }

    /// Truncating here would put the boundary one cycle BELOW the time it
    /// represents, so `cycles_until_boundary` would return 1 forever and the
    /// run would crawl a cycle at a time without ever reaching the boundary.
    #[test]
    fn rounds_nanoseconds_up_to_a_reachable_cycle() {
        // 8 MHz: one cycle is 125 ns, so 100 ns must round UP to 1 cycle.
        assert_eq!(ns_to_cycles(100, 8_000_000), 1);
        assert_eq!(cycles_to_ns(1, 8_000_000), 125);
        assert!(cycles_to_ns(ns_to_cycles(100, 8_000_000), 8_000_000) >= 100);
    }

    #[test]
    fn a_zero_clock_cannot_divide_by_zero() {
        assert_eq!(cycles_to_ns(1_000, 0), 0);
        assert_eq!(ns_to_cycles(1_000, 0), 0);
    }

    // ── Binding ─────────────────────────────────────────────────────────────

    #[test]
    fn binding_rejects_a_model_output_routed_to_a_firmware_driven_pin() {
        let bus = crate::bus::SystemBus::new();
        let configs = [mock_model(
            "m",
            1_000,
            &[],
            &[("out", "board.gpio.pa5")],
            &[("out", serde_yaml::Value::Bool(true))],
        )];
        let (_, errors) = SignalRouter::bind(&configs, &bus);
        assert_eq!(
            errors,
            vec![RoutingError::NotWritable {
                path: "board.gpio.pa5".to_string()
            }]
        );
    }

    #[test]
    fn binding_rejects_a_model_input_sourced_from_an_analog_sink() {
        let bus = crate::bus::SystemBus::new();
        let configs = [mock_model(
            "m",
            1_000,
            &[("v", "board.analog.pa0_volts")],
            &[],
            &[],
        )];
        let (_, errors) = SignalRouter::bind(&configs, &bus);
        assert_eq!(
            errors,
            vec![RoutingError::NotReadable {
                path: "board.analog.pa0_volts".to_string()
            }]
        );
    }

    /// The generic plant demo routes only plain store paths. Binding must
    /// resolve nothing and complain about nothing, or adding pin routing would
    /// have broken every manifest that predates it.
    #[test]
    fn plain_store_manifests_bind_to_an_empty_router() {
        let bus = crate::bus::SystemBus::new();
        let configs = [mock_model(
            "plant",
            10_000,
            &[("enable", "control.enable")],
            &[("v_out", "plant.output.voltage")],
            &[("v_out", serde_yaml::Value::from(1.0))],
        )];
        let (router, errors) = SignalRouter::bind(&configs, &bus);
        assert!(router.is_empty());
        assert!(errors.is_empty(), "unexpected routing errors: {errors:?}");
    }

    // ── Volts → pin level (Schmitt input) ───────────────────────────────────

    /// The ATmega328P at 5 V: VIL 0.3·VCC = 1.5 V, VIH 0.6·VCC = 3.0 V.
    fn atmega_thresholds() -> InputThresholds {
        InputThresholds::from_ratios(5.0, GpioInputThresholds { vil: 0.3, vih: 0.6 })
    }

    #[test]
    fn thresholds_are_the_ratios_of_the_io_supply() {
        let t = atmega_thresholds();
        assert!((t.vil_volts - 1.5).abs() < 1e-12, "{t:?}");
        assert!((t.vih_volts - 3.0).abs() < 1e-12, "{t:?}");
    }

    #[test]
    fn a_rising_voltage_reads_high_from_vih() {
        let t = atmega_thresholds();
        let mut level = false;
        for volts in [0.0, 1.0, 2.0, 2.9] {
            level = t.next_level(level, volts);
            assert!(!level, "{volts} V is below VIH; the pad still reads low");
        }
        level = t.next_level(level, 3.2);
        assert!(level, "3.2 V is above VIH");
        level = t.next_level(level, 4.9);
        assert!(level);
    }

    #[test]
    fn a_falling_voltage_reads_low_from_vil() {
        let t = atmega_thresholds();
        let mut level = true;
        for volts in [5.0, 3.0, 2.0, 1.6] {
            level = t.next_level(level, volts);
            assert!(level, "{volts} V is above VIL; the pad still reads high");
        }
        level = t.next_level(level, 1.4);
        assert!(!level, "1.4 V is below VIL");
        level = t.next_level(level, 0.0);
        assert!(!level);
    }

    /// Between VIL and VIH the answer is whatever the pad read last, in both
    /// directions. A midpoint comparator would flip at 2.25 V here.
    #[test]
    fn the_band_between_the_thresholds_keeps_the_previous_level() {
        let t = atmega_thresholds();
        for volts in [1.51, 2.0, 2.25, 2.5, 2.99] {
            assert!(!t.next_level(false, volts), "{volts} V held low");
            assert!(t.next_level(true, volts), "{volts} V held high");
        }
        assert!(t.next_level(true, f64::NAN), "NaN is on neither side");
        assert!(!t.next_level(false, f64::NAN), "NaN is on neither side");
    }

    /// The thresholds are inclusive: a ramp that lands exactly on VIH reads
    /// high on that sample, and one that lands exactly on VIL reads low.
    #[test]
    fn a_ramp_landing_exactly_on_a_threshold_crosses_it() {
        let t = atmega_thresholds();
        let below = t.vih_volts.next_down();
        assert!(
            !t.next_level(false, below),
            "one ulp under VIH is still low"
        );
        assert!(t.next_level(false, t.vih_volts), "exactly VIH reads high");

        let above = t.vil_volts.next_up();
        assert!(t.next_level(true, above), "one ulp over VIL is still high");
        assert!(!t.next_level(true, t.vil_volts), "exactly VIL reads low");

        // A ramp in 0.1 V steps from 0 V, sampled the way a model boundary
        // samples it, first reads high on the sample at VIH.
        let mut level = false;
        let mut first_high = None;
        for step in 0..=50 {
            let volts = if step == 30 {
                t.vih_volts
            } else {
                f64::from(step) * 0.1
            };
            level = t.next_level(level, volts);
            if level && first_high.is_none() {
                first_high = Some(step);
            }
        }
        assert_eq!(first_high, Some(30));
    }

    /// Volts on a pin path need the chip's thresholds. A bus built from no
    /// descriptor has none, so the route is refused when binding, naming the
    /// descriptor key to add.
    #[test]
    fn volts_to_a_pin_without_thresholds_is_refused_naming_the_key() {
        let bus = crate::bus::SystemBus::new();
        let configs = [mock_model(
            "m",
            1_000,
            &[],
            &[("v_pad", "board.gpio_in.pd2")],
            &[("v_pad", serde_yaml::Value::from(2.5))],
        )];
        let (router, errors) = SignalRouter::bind(&configs, &bus);
        assert!(router.is_empty());
        assert_eq!(
            errors,
            vec![RoutingError::NoInputThresholds {
                path: "board.gpio_in.pd2".to_string(),
                key: "gpio_input_thresholds",
            }]
        );
        let message = errors[0].to_string();
        assert!(message.contains("`gpio_input_thresholds`"), "{message}");

        // A boolean needs no thresholds: the same bus gets as far as resolving
        // the pad, which a bare bus cannot.
        let configs = [mock_model(
            "m",
            1_000,
            &[],
            &[("pressed", "board.gpio_in.pd2")],
            &[("pressed", serde_yaml::Value::Bool(true))],
        )];
        let (_, errors) = SignalRouter::bind(&configs, &bus);
        assert!(
            matches!(errors.as_slice(), [RoutingError::UnknownPad { .. }]),
            "{errors:?}"
        );
    }

    /// An analog model's outputs are volts whatever it computes, so its pin
    /// route is refused before any step. The missing supply is named when the
    /// ratios are there and the rail is not.
    #[test]
    fn an_analog_pin_route_names_whichever_key_is_missing() {
        let mut bus = crate::bus::SystemBus::new();
        bus.gpio_input_thresholds = Some(GpioInputThresholds { vil: 0.3, vih: 0.6 });
        let mut config = mock_model(
            "circuit",
            1_000,
            &[],
            &[("in_pd2", "board.gpio_in.pd2")],
            &[],
        );
        config.adapter = CosimAdapter::Analog;
        let (_, errors) = SignalRouter::bind(&[config], &bus);
        assert_eq!(
            errors,
            vec![RoutingError::NoInputThresholds {
                path: "board.gpio_in.pd2".to_string(),
                key: "io_voltage_v",
            }]
        );
    }

    /// `ui.` paths are set from outside the engine. A model output routed to
    /// one would fight every press.
    #[test]
    fn a_model_output_cannot_drive_a_ui_path() {
        let bus = crate::bus::SystemBus::new();
        let configs = [mock_model(
            "m",
            1_000,
            &[],
            &[("pressed", "ui.touch.pressed")],
            &[("pressed", serde_yaml::Value::Bool(true))],
        )];
        let (_, errors) = SignalRouter::bind(&configs, &bus);
        assert_eq!(
            errors,
            vec![RoutingError::ExternalOnly {
                path: "ui.touch.pressed".to_string()
            }]
        );
    }

    #[test]
    fn an_empty_model_list_makes_no_session() {
        let bus = crate::bus::SystemBus::new();
        let session =
            CosimSession::new(&[], Path::new("."), &bus).expect("no models is not an error");
        assert!(session.is_none());
    }
}
