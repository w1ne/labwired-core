// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ST L6360 digital CQ port, DS8900 Rev9 tables5/12/16/23 and sections6–8.
//! Register/parity/protection state is shared with the existing I2C controller;
//! byte framing, parity, timing and collisions stay in the timed UART engine.
//! Opposed known digital drivers are an explicit overload scenario. No analog
//! current, threshold, slew, regulator transient or thermal calculation is made.
//! UART propagation uses declared 160ns TX / 100ns RX scenarios from table5's
//! typical edge values, not guaranteed limits. EN and L+ analog settling are
//! outside this digital proof. Nonzero CQ debounce is applied to static input;
//! timed UART reception rejects it rather than replacing filtering with delay.

use super::l6360_control::{L6360Control, L6360Inputs, L6360Mode, L6360Snapshot};
use crate::bus::{BusResidentDevice, DevicePins};
use crate::network::timed_uart::{cycles_to_ps, DigitalDrive, TimedUartPort};
use crate::peripherals::components::supply_domain::SupplyGate;
use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};
use crate::peripherals::rs485::PinSense;
use crate::peripherals::uart_transceiver::DigitalUartTransceiver;
use crate::sim_input::{InputChannel, SimInput, SimInputError};
use anyhow::{anyhow, ensure, Result};
use std::borrow::Cow;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

pub const DRIVER_DELAY_PS: u64 = 160_000;
pub const RECEIVER_DELAY_PS: u64 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub drive: DigitalDrive,
    pub outcq: Option<bool>,
    pub irq_high: bool,
    pub lplus_enabled: bool,
    pub opposed_drivers: bool,
}
#[derive(Debug, Default)]
struct Debounce {
    candidate: Option<bool>,
    since_ns: u64,
    output: Option<bool>,
}
impl Debounce {
    fn sample(&mut self, input: Option<bool>, now_ns: u64, delay_ns: u64) -> Option<bool> {
        let Some(input) = input else {
            self.candidate = None;
            self.output = None;
            return None;
        };
        if self.candidate != Some(input) {
            self.candidate = Some(input);
            self.since_ns = now_ns;
        }
        if now_ns.saturating_sub(self.since_ns) >= delay_ns {
            self.output = Some(input);
        }
        self.output
    }
}
#[derive(Debug)]
struct State {
    scenario: L6360Inputs,
    supply: SupplyGate,
    port: Option<TimedUartPort>,
    observation: Observation,
    debounce: Debounce,
    tx_rejected: u64,
    unknown_line: u64,
    line_error: Option<String>,
}
#[derive(Debug, Clone)]
pub struct L6360 {
    id: String,
    control: L6360Control,
    encq: PinSense,
    enl: PinSense,
    reset_n: PinSense,
    state: Arc<Mutex<State>>,
}
impl L6360 {
    pub fn new(
        id: impl Into<String>,
        control: L6360Control,
        encq: PinSense,
        enl: PinSense,
        reset_n: PinSense,
    ) -> Self {
        Self {
            id: id.into(),
            control,
            encq,
            enl,
            reset_n,
            state: Arc::new(Mutex::new(State {
                scenario: L6360Inputs::default(),
                supply: SupplyGate::legacy(true),
                port: None,
                debounce: Debounce::default(),
                observation: Observation {
                    drive: DigitalDrive::HighImpedance,
                    outcq: None,
                    irq_high: true,
                    lplus_enabled: false,
                    opposed_drivers: false,
                },
                tx_rejected: 0,
                unknown_line: 0,
                line_error: None,
            })),
        }
    }
    pub fn control(&self) -> L6360Control {
        self.control.clone()
    }
    pub fn observation(&self) -> Observation {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .observation
    }
    /// Table23. Unknown INCQ never becomes a guessed driven logic level.
    pub fn requested_drive(mode: L6360Mode, encq: bool, incq: Option<bool>) -> DigitalDrive {
        use DigitalDrive::{High, HighImpedance, Low, Unknown};
        match mode {
            L6360Mode::Off | L6360Mode::TriState => HighImpedance,
            L6360Mode::LowSideOn => Low,
            L6360Mode::HighSideOn => High,
            _ if !encq => HighImpedance,
            L6360Mode::PushPull => incq.map_or(Unknown, |v| if v { Low } else { High }),
            L6360Mode::LowSide => incq.map_or(Unknown, |v| if v { Low } else { HighImpedance }),
            L6360Mode::HighSide => incq.map_or(Unknown, |v| if v { HighImpedance } else { High }),
        }
    }
    pub fn sample(
        &self,
        cycle: u64,
        hz: u64,
        incq: Option<bool>,
        peer: DigitalDrive,
    ) -> Observation {
        let now_ns = cycles_to_ps(cycle, hz) / 1000;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut input = state.scenario;
        input.vcc_valid &= state.supply.powered();
        input.encq = self.encq.level();
        input.enl = self.enl.level();
        input.reset_n = self.reset_n.level();
        let before = self.control.snapshot();
        let requested = Self::requested_drive(before.mode, input.encq, incq);
        let opposed = requested
            .level()
            .zip(peer.level())
            .is_some_and(|(a, b)| a != b)
            && input.vcc_valid
            && input.reset_n;
        input.cq_overload |= opposed;
        if let Err(error) = self.control.update_inputs(now_ns, input) {
            state.line_error = Some(error.to_string());
        }
        let snap = self.control.snapshot();
        let drive = if snap.protocol_error.is_some() || state.line_error.is_some() {
            DigitalDrive::Unknown
        } else if snap.cq_forced_shutdown {
            DigitalDrive::HighImpedance
        } else {
            Self::requested_drive(snap.mode, input.encq, incq)
        };
        let line = match (drive, peer) {
            (DigitalDrive::Unknown, _) | (_, DigitalDrive::Unknown) => None,
            (DigitalDrive::HighImpedance, DigitalDrive::HighImpedance) => {
                snap.cq_pulldown_enabled.then_some(false)
            }
            (DigitalDrive::HighImpedance, other) | (other, DigitalDrive::HighImpedance) => {
                other.level()
            }
            (a, b) if a == b => a.level(),
            _ => None,
        };
        if line.is_none() {
            state.unknown_line += 1;
        }
        let outcq = if !input.reset_n
            || !input.vcc_valid
            || snap.mode == L6360Mode::Off
            || snap.protocol_error.is_some()
        {
            state.debounce = Debounce::default();
            None
        } else if input.encq {
            state.debounce = Debounce::default();
            Some(true)
        } else {
            state
                .debounce
                .sample(line, now_ns, snap.cq_debounce_ns)
                .map(|v| !v)
        };
        let observation = Observation {
            drive,
            outcq,
            irq_high: !snap.irq_asserted,
            lplus_enabled: snap.lplus_driver_enabled,
            opposed_drivers: opposed,
        };
        state.observation = observation;
        observation
    }
    fn serial_state(&self) -> Option<L6360Snapshot> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let snap = self.control.snapshot();
        (state.supply.powered()
            && state.line_error.is_none()
            && snap.protocol_error.is_none()
            && self.reset_n.level()
            && snap.inputs.vcc_valid)
            .then_some(snap)
    }
}
impl DigitalUartTransceiver for L6360 {
    fn id(&self) -> &str {
        &self.id
    }
    fn driver_delay_ps(&self) -> u64 {
        DRIVER_DELAY_PS
    }
    fn receiver_delay_ps(&self) -> u64 {
        RECEIVER_DELAY_PS
    }
    fn bind_timed_port(&self, port: TimedUartPort) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).port = Some(port);
    }
    fn clock_valid(&self) -> bool {
        self.serial_state().is_some()
    }
    fn receiver_valid(&self) -> bool {
        // DS8900 table 23: ENCQ=1 forces OUTCQ high. It does not echo
        // the port's own transmission into a still-enabled MCU USART.
        !self.encq.level()
            && self
                .serial_state()
                .is_some_and(|s| s.mode != L6360Mode::Off && s.cq_debounce_ns == 0)
    }
    fn allow_tx(&self) -> bool {
        let allowed = self.serial_state().is_some_and(|s| {
            s.mode == L6360Mode::PushPull && self.encq.level() && !s.cq_forced_shutdown
        });
        if !allowed {
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .tx_rejected += 1;
        }
        allowed
    }
    fn accept_peer_byte(&self) -> bool {
        self.receiver_valid() && !self.encq.level()
    }
    fn allow_timed_tx(&self, _cycle: u64, _hz: u64) -> bool {
        self.allow_tx()
    }
    fn driving_at(&self, _cycle: u64, _hz: u64) -> bool {
        self.serial_state().is_some_and(|s| {
            !s.cq_forced_shutdown
                && match s.mode {
                    L6360Mode::Off | L6360Mode::TriState => false,
                    L6360Mode::LowSideOn | L6360Mode::HighSideOn => true,
                    _ => self.encq.level(),
                }
        })
    }
    fn tx_inverted(&self) -> bool {
        true
    }
    fn rx_inverted(&self) -> bool {
        true
    }
}
static INPUTS: &[InputChannel] = &[
    InputChannel {
        key: Cow::Borrowed("vcc_valid"),
        label: Cow::Borrowed("Supply valid"),
        unit: Cow::Borrowed("bool"),
        min: 0.,
        max: 1.,
        default: Some(1.),
    },
    InputChannel {
        key: Cow::Borrowed("lplus_voltage_valid"),
        label: Cow::Borrowed("L+ voltage valid"),
        unit: Cow::Borrowed("bool"),
        min: 0.,
        max: 1.,
        default: Some(1.),
    },
    InputChannel {
        key: Cow::Borrowed("regulator_undervoltage"),
        label: Cow::Borrowed("Regulator undervoltage"),
        unit: Cow::Borrowed("bool"),
        min: 0.,
        max: 1.,
        default: Some(0.),
    },
    InputChannel {
        key: Cow::Borrowed("overtemperature"),
        label: Cow::Borrowed("Overtemperature"),
        unit: Cow::Borrowed("bool"),
        min: 0.,
        max: 1.,
        default: Some(0.),
    },
    InputChannel {
        key: Cow::Borrowed("cq_overload"),
        label: Cow::Borrowed("CQ overload scenario"),
        unit: Cow::Borrowed("bool"),
        min: 0.,
        max: 1.,
        default: Some(0.),
    },
    InputChannel {
        key: Cow::Borrowed("lplus_overload"),
        label: Cow::Borrowed("L+ overload scenario"),
        unit: Cow::Borrowed("bool"),
        min: 0.,
        max: 1.,
        default: Some(0.),
    },
];
#[derive(Debug)]
struct Pins {
    gate: L6360,
    incq: (u64, u8),
    outcq: (u64, u8),
    irq: (u64, u8),
    hz: u64,
    edges: Vec<u64>,
    last: Option<Observation>,
}
impl SimInput for Pins {
    fn input_channels(&self) -> &[InputChannel] {
        INPUTS
    }
    fn set_input(&mut self, key: &str, value: f64) -> std::result::Result<(), SimInputError> {
        self.require_channel(key, value)?;
        let mut s = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        match key {
            "vcc_valid" => s.scenario.vcc_valid = value != 0.,
            "lplus_voltage_valid" => s.scenario.lplus_voltage_valid = value != 0.,
            "regulator_undervoltage" => s.scenario.regulator_undervoltage = value != 0.,
            "overtemperature" => s.scenario.overtemperature = value != 0.,
            "cq_overload" => s.scenario.cq_overload = value != 0.,
            "lplus_overload" => s.scenario.lplus_overload = value != 0.,
            _ => unreachable!(),
        }
        Ok(())
    }
    fn component_id(&self) -> Option<&str> {
        Some(&self.gate.id)
    }
}
impl BusResidentDevice for Pins {
    fn service(&mut self, pins: &mut dyn DevicePins, now: u64) {
        let port = self
            .gate
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .port
            .clone();
        let peer = port
            .as_ref()
            .map_or(DigitalDrive::HighImpedance, |p| p.peer_digital_drive(now));
        let observation = self.gate.sample(
            now,
            self.hz,
            pins.known_pad_bit(self.incq.0, self.incq.1),
            peer,
        );
        if let Some(port) = port {
            if let Err(e) = port.drive_digital(now, observation.drive) {
                self.gate
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .line_error = Some(e.to_string());
            }
        }
        if self.last != Some(observation) {
            pins.drive_input_bit(self.irq.0, self.irq.1, observation.irq_high);
            if let Some(level) = observation.outcq {
                pins.drive_input_bit(self.outcq.0, self.outcq.1, level);
            }
            self.last = Some(observation);
        }
    }
    fn as_sim_input(&mut self) -> &mut dyn SimInput {
        self
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn id(&self) -> &str {
        &self.gate.id
    }
    fn edge_service_addrs(&self) -> &[u64] {
        &self.edges
    }
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        let s = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        let c = self.gate.control.snapshot();
        vec![crate::peripheral_log::PeripheralLog::new("l6360",vec![format!("address={:#04x} mode={:?} status={:#04x} parity={:#04x} CQ={:?} L+={} IRQ={} rejected_tx={} unknown_line={} cq_debounce_ns={} protocol_error={:?} line_error={:?}; static digital truth, UART zero-CQ-debounce profile",c.address,c.mode,c.status,c.parity,s.observation.drive,s.observation.lplus_enabled,!s.observation.irq_high,s.tx_rejected,s.unknown_line,c.cq_debounce_ns,c.protocol_error,s.line_error)])]
    }
}
pub struct L6360Kit;
pub static L6360_KIT: L6360Kit = L6360Kit;
static METADATA:KitMetadata=KitMetadata{
    device_type:Cow::Borrowed("l6360"),label:Cow::Borrowed("ST L6360"),
    summary:Cow::Borrowed("I2C-configured IO-Link master PHY with actual CQ mode, parity and fault state."),
    detail:Cow::Borrowed("DS8900 Rev9 digital truth and declared timing scenarios. UART uses the shared engine; nonzero CQ debounce rejects UART reception. I/Q, LED outputs and analog power/thermal behavior are outside this kit."),
    transport:Transport::I2c,category:Category::Uart,
    config_keys:Cow::Borrowed(&[
        ConfigKey{name:Cow::Borrowed("address"),ty:ConfigType::Int,doc:Cow::Borrowed("Required 7-bit address 0x60..0x67 from SA straps.")},
        ConfigKey{name:Cow::Borrowed("uart"),ty:ConfigType::Str,doc:Cow::Borrowed("Required existing MCU UART instance.")},
        ConfigKey{name:Cow::Borrowed("encq"),ty:ConfigType::Str,doc:Cow::Borrowed("Required MCU CQ enable output pad.")},
        ConfigKey{name:Cow::Borrowed("enl"),ty:ConfigType::Str,doc:Cow::Borrowed("Required MCU L+ enable output pad.")},
        ConfigKey{name:Cow::Borrowed("reset_n"),ty:ConfigType::Str,doc:Cow::Borrowed("Required active-low reset output pad.")},
        ConfigKey{name:Cow::Borrowed("incq"),ty:ConfigType::Str,doc:Cow::Borrowed("Required MCU UART TX / wake timer output pad.")},
        ConfigKey{name:Cow::Borrowed("outcq"),ty:ConfigType::Str,doc:Cow::Borrowed("Required MCU UART RX pad.")},
        ConfigKey{name:Cow::Borrowed("irq"),ty:ConfigType::Str,doc:Cow::Borrowed("Required active-low IRQ pad with ideal board pull-up.")},
    ]),labs:Cow::Borrowed(&[]),inputs:Cow::Borrowed(INPUTS),
};
fn pad(ctx: &AttachCtx<'_>, key: &str) -> Result<(u64, u8)> {
    let name = ctx
        .config_str(key)
        .ok_or_else(|| anyhow!("l6360 requires config.{key}"))?;
    crate::bus::SystemBus::resolve_pin_idr_pub(ctx.bus, name)
        .ok_or_else(|| anyhow!("l6360 {key} '{name}' is not a GPIO pad"))
}
fn output(ctx: &AttachCtx<'_>, key: &str) -> Result<(u64, u8)> {
    let name = ctx
        .config_str(key)
        .ok_or_else(|| anyhow!("l6360 requires config.{key}"))?;
    ctx.resolve_pin_odr(name)
        .ok_or_else(|| anyhow!("l6360 {key} '{name}' has no GPIO output"))
}
fn watch(ctx: &mut AttachCtx<'_>, key: &str) -> Result<PinSense> {
    let (addr, bit) = pad(ctx, key)?;
    let idx = ctx
        .bus
        .find_peripheral_index(addr)
        .ok_or_else(|| anyhow!("l6360 {key} GPIO block missing"))?;
    let cell = Arc::new(AtomicBool::new(false));
    ensure!(
        ctx.bus.peripherals[idx]
            .dev
            .watch_pad_level(bit, cell.clone()),
        "l6360 {key} pad observation unsupported"
    );
    Ok(PinSense::Cell(cell))
}
impl PeripheralKit for L6360Kit {
    fn metadata(&self) -> &KitMetadata {
        &METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        for key in ctx.ext.config.keys() {
            ensure!(
                matches!(
                    key.as_str(),
                    "address"
                        | "uart"
                        | "encq"
                        | "enl"
                        | "reset_n"
                        | "incq"
                        | "outcq"
                        | "irq"
                        | "powered"
                        | "supply_domain"
                        | "supply_min_v"
                        | "supply_rails"
                ),
                "l6360 unsupported config key '{key}'"
            );
        }
        let address = ctx
            .config_i64("address")
            .and_then(|v| u8::try_from(v).ok())
            .ok_or_else(|| anyhow!("l6360 requires numeric address"))?;
        let control = L6360Control::new(address)?;
        let uart = ctx
            .config_str("uart")
            .ok_or_else(|| anyhow!("l6360 requires config.uart"))?
            .to_string();
        let idx = ctx
            .bus
            .find_peripheral_index_by_name(&uart)
            .ok_or_else(|| anyhow!("l6360 UART '{uart}' missing"))?;
        let id = ctx.device_id().to_string();
        let gate = L6360::new(
            id,
            control.clone(),
            watch(ctx, "encq")?,
            watch(ctx, "enl")?,
            watch(ctx, "reset_n")?,
        );
        let mut pins = Pins {
            gate: gate.clone(),
            incq: output(ctx, "incq")?,
            outcq: pad(ctx, "outcq")?,
            irq: pad(ctx, "irq")?,
            hz: ctx.bus.cpu_hz,
            edges: ["encq", "enl", "reset_n", "incq"]
                .into_iter()
                .map(|k| output(ctx, k).map(|p| p.0))
                .collect::<Result<_>>()?,
            last: None,
        };
        let supply = ctx.attach_i2c_device_with_supply(Box::new(control.i2c_device()))?;
        gate.state.lock().unwrap_or_else(|e| e.into_inner()).supply = supply;
        let now = ctx.bus.current_cycle;
        pins.service(ctx.bus, now);
        ctx.bus.peripherals[idx]
            .dev
            .attach_digital_uart_transceiver(Arc::new(gate))?;
        ctx.bus.gpio_devices.push(Box::new(pins));
        Ok(())
    }
}
