// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ST L6362A digital push-pull PHY, DocID027660 Rev10 (Nov2017),
//! Tables10/14/15/19 and sections11.2/11.5/11.6.
//! IN1 is a fixed board strap; CQ=IN1 XOR IN2 and OUTIQ=IN1 XOR CQ.
//! OL is current-limitation status, not a synthetic wake command.
//!
//! Opposed known digital drivers provide an explicit overload scenario.
//! This is not an analog current calculation. OL delay2.5us (typical),
//! cutoff5ms and retry80ms are declared scenarios, not guaranteed silicon
//! timing. EN settling, electrical thresholds, current, slew and clamp energy
//! are outside the digital timing proof.

use crate::bus::{BusResidentDevice, DevicePins};
use crate::network::timed_uart::{cycles_to_ps, DigitalDrive, TimedUartPort};
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

pub const DRIVER_DELAY_PS: u64 = 270_000;
pub const RECEIVER_DELAY_PS: u64 = 200_000;
pub const OL_DELAY_PS: u64 = 2_500_000;
pub const CUTOFF_PS: u64 = 5_000_000_000;
pub const RETRY_PS: u64 = 80_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub drive: DigitalDrive,
    pub outiq: Option<bool>,
    pub diag_high: bool,
    pub ol_high: bool,
    pub current_limited: bool,
    pub cut_off: bool,
}

#[derive(Debug)]
struct State {
    vcc_valid: bool,
    overtemperature: bool,
    forced_overload: bool,
    overload_since: Option<u64>,
    retry_at: Option<u64>,
    port: Option<TimedUartPort>,
    observation: Observation,
    tx_while_disabled: u64,
    unknown_line: u64,
    line_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct L6362a {
    id: String,
    en: PinSense,
    in1: bool,
    state: Arc<Mutex<State>>,
}
impl L6362a {
    pub fn new(id: impl Into<String>, en: PinSense, in1: bool) -> Self {
        Self {
            id: id.into(),
            en,
            in1,
            state: Arc::new(Mutex::new(State {
                vcc_valid: true,
                overtemperature: false,
                forced_overload: false,
                overload_since: None,
                retry_at: None,
                port: None,
                observation: Observation {
                    drive: DigitalDrive::HighImpedance,
                    outiq: None,
                    diag_high: false,
                    ol_high: true,
                    current_limited: false,
                    cut_off: false,
                },
                tx_while_disabled: 0,
                unknown_line: 0,
                line_error: None,
            })),
        }
    }
    pub fn cq_drive(en: bool, in1: bool, in2: bool) -> DigitalDrive {
        if en {
            DigitalDrive::from_level(Some(in1 ^ in2))
        } else {
            DigitalDrive::HighImpedance
        }
    }
    pub fn receiver(in1: bool, cq: bool, vcc_valid: bool) -> bool {
        vcc_valid && (in1 ^ cq)
    }
    pub fn observation(&self) -> Observation {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .observation
    }
    pub fn set_faults(&self, vcc_valid: bool, overtemperature: bool, overload: bool) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.vcc_valid = vcc_valid;
        s.overtemperature = overtemperature;
        s.forced_overload = overload;
    }
    pub fn sample(
        &self,
        cycle: u64,
        hz: u64,
        in2: Option<bool>,
        peer: DigitalDrive,
    ) -> Observation {
        let now = cycles_to_ps(cycle, hz);
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.retry_at.is_some_and(|at| now >= at) {
            s.retry_at = None;
            s.overload_since = None;
        }
        let enabled = self.en.level() && s.vcc_valid && !s.overtemperature && s.retry_at.is_none();
        let drive = if !enabled {
            DigitalDrive::HighImpedance
        } else {
            in2.map_or(DigitalDrive::Unknown, |input| {
                Self::cq_drive(true, self.in1, input)
            })
        };
        let opposed = drive
            .level()
            .zip(peer.level())
            .is_some_and(|(own, other)| own != other);
        let limited = enabled && (s.forced_overload || opposed);
        if limited {
            s.overload_since.get_or_insert(now);
        } else {
            s.overload_since = None;
        }
        if s.overload_since
            .is_some_and(|start| now.saturating_sub(start) >= CUTOFF_PS)
        {
            s.retry_at = Some(now.saturating_add(RETRY_PS));
            s.overload_since = None;
        }
        let cut_off = s.retry_at.is_some();
        let drive = if cut_off {
            DigitalDrive::HighImpedance
        } else {
            drive
        };
        let line = match (drive, peer) {
            (DigitalDrive::Unknown, _) | (_, DigitalDrive::Unknown) => None,
            // Table9: the I/Q pin has an internal weak pull-down (250..550k).
            (DigitalDrive::HighImpedance, DigitalDrive::HighImpedance) => Some(false),
            (DigitalDrive::HighImpedance, other) | (other, DigitalDrive::HighImpedance) => {
                other.level()
            }
            (a, b) if a == b => a.level(),
            _ => None,
        };
        let outiq = if !s.vcc_valid {
            Some(false)
        } else {
            line.map(|cq| Self::receiver(self.in1, cq, true))
        };
        if line.is_none() {
            s.unknown_line += 1;
        }
        let ol = !cut_off
            && s.overload_since
                .is_some_and(|start| now.saturating_sub(start) >= OL_DELAY_PS);
        let observation = Observation {
            drive,
            outiq,
            diag_high: self.en.level() && s.vcc_valid && !s.overtemperature && !cut_off,
            ol_high: !ol,
            current_limited: limited && !cut_off,
            cut_off,
        };
        s.observation = observation;
        observation
    }
    fn ready(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.en.level()
            && s.vcc_valid
            && !s.overtemperature
            && s.retry_at.is_none()
            && !s.forced_overload
            && !s.observation.current_limited
            && s.line_error.is_none()
    }
}
impl DigitalUartTransceiver for L6362a {
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
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .line_error
            .is_none()
    }
    fn receiver_valid(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.vcc_valid && s.line_error.is_none()
    }
    fn allow_tx(&self) -> bool {
        if self.ready() {
            true
        } else {
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .tx_while_disabled += 1;
            false
        }
    }
    fn accept_peer_byte(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let driving = self.en.level() && !s.overtemperature && s.retry_at.is_none();
        !driving && s.vcc_valid && s.line_error.is_none()
    }
    fn allow_timed_tx(&self, _cycle: u64, _hz: u64) -> bool {
        self.allow_tx()
    }
    fn driving_at(&self, _cycle: u64, _hz: u64) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.en.level() && s.vcc_valid && !s.overtemperature && s.retry_at.is_none()
    }
    fn tx_inverted(&self) -> bool {
        self.in1
    }
    fn rx_inverted(&self) -> bool {
        self.in1
    }
}

static INPUTS: &[InputChannel] = &[
    InputChannel {
        key: Cow::Borrowed("vcc_valid"),
        label: Cow::Borrowed("Supply above UVLO"),
        unit: Cow::Borrowed("bool"),
        min: 0.,
        max: 1.,
        default: Some(1.),
    },
    InputChannel {
        key: Cow::Borrowed("overtemperature"),
        label: Cow::Borrowed("Thermal shutdown"),
        unit: Cow::Borrowed("bool"),
        min: 0.,
        max: 1.,
        default: Some(0.),
    },
    InputChannel {
        key: Cow::Borrowed("overload"),
        label: Cow::Borrowed("Explicit current-limit scenario"),
        unit: Cow::Borrowed("bool"),
        min: 0.,
        max: 1.,
        default: Some(0.),
    },
];

#[derive(Debug)]
struct Pins {
    gate: L6362a,
    in2: (u64, u8),
    outiq: (u64, u8),
    diag: (u64, u8),
    ol: (u64, u8),
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
            "vcc_valid" => s.vcc_valid = value != 0.,
            "overtemperature" => s.overtemperature = value != 0.,
            "overload" => s.forced_overload = value != 0.,
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
        let input = pins.known_pad_bit(self.in2.0, self.in2.1);
        let observation = self.gate.sample(now, self.hz, input, peer);
        if let Some(port) = port {
            if let Err(error) = port.drive_digital(now, observation.drive) {
                self.gate
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .line_error = Some(error.to_string());
            }
        }
        if self.last != Some(observation) {
            pins.drive_input_bit(self.diag.0, self.diag.1, observation.diag_high);
            pins.drive_input_bit(self.ol.0, self.ol.1, observation.ol_high);
            if let Some(level) = observation.outiq {
                pins.drive_input_bit(self.outiq.0, self.outiq.1, level);
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
        vec![crate::peripheral_log::PeripheralLog::new("l6362a", vec![format!(
            "PP IN1={} diag={} OL={} cutoff={} disabled_tx={} unknown_line={} line_error={:?}; digital scenario OL2.5us cutoff5ms retry80ms",
            self.gate.in1, s.observation.diag_high, s.observation.ol_high, s.observation.cut_off,
            s.tx_while_disabled, s.unknown_line, s.line_error
        )])]
    }
}

pub struct L6362aKit;
pub static L6362A_KIT: L6362aKit = L6362aKit;
static METADATA: KitMetadata = KitMetadata {
    device_type: Cow::Borrowed("l6362a"), label: Cow::Borrowed("ST L6362A"),
    summary: Cow::Borrowed("Push-pull digital IO-Link PHY with real IN1 polarity, EN/DIAG and OL."),
    detail: Cow::Borrowed("DocID027660 Rev10 digital truth tables; same timed UART engine. Opposed digital drivers are an explicit overload scenario; no analog current, voltage, slew or thermal calculation."),
    transport: Transport::Uart, category: Category::Uart,
    config_keys: Cow::Borrowed(&[
        ConfigKey { name: Cow::Borrowed("en"), ty: ConfigType::Str, doc: Cow::Borrowed("Required MCU enable output pad.") },
        ConfigKey { name: Cow::Borrowed("in1"), ty: ConfigType::Str, doc: Cow::Borrowed("Required fixed high/low board strap; live IN1 switching is unsupported.") },
        ConfigKey { name: Cow::Borrowed("in2"), ty: ConfigType::Str, doc: Cow::Borrowed("Required MCU TX/SIO pad.") },
        ConfigKey { name: Cow::Borrowed("outiq"), ty: ConfigType::Str, doc: Cow::Borrowed("Required MCU RX pad.") },
        ConfigKey { name: Cow::Borrowed("diag"), ty: ConfigType::Str, doc: Cow::Borrowed("Required EN/DIAG sense pad, through the actual board resistor network.") },
        ConfigKey { name: Cow::Borrowed("ol"), ty: ConfigType::Str, doc: Cow::Borrowed("Required active-low OL pad with ideal board pull-up.") },
    ]),
    labs: Cow::Borrowed(&[]), inputs: Cow::Borrowed(INPUTS),
};
fn pad(ctx: &AttachCtx<'_>, key: &str) -> Result<(u64, u8)> {
    let label = ctx
        .config_str(key)
        .ok_or_else(|| anyhow!("l6362a requires config.{key}"))?;
    crate::bus::SystemBus::resolve_pin_idr_pub(ctx.bus, label)
        .ok_or_else(|| anyhow!("l6362a: {key} '{label}' is not a GPIO pad"))
}
impl PeripheralKit for L6362aKit {
    fn metadata(&self) -> &KitMetadata {
        &METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        for key in ctx.ext.config.keys() {
            ensure!(
                matches!(key.as_str(), "en" | "in1" | "in2" | "outiq" | "diag" | "ol"),
                "l6362a: unsupported config key '{key}'"
            );
        }
        let in1 = match ctx.config_str("in1") {
            Some("low" | "gnd" | "0") => false,
            Some("high" | "vdd" | "1") => true,
            _ => anyhow::bail!("l6362a: in1 must be a fixed low/high board strap"),
        };
        let en_label = ctx
            .config_str("en")
            .ok_or_else(|| anyhow!("l6362a requires config.en"))?
            .to_string();
        let (addr, bit) = pad(ctx, "en")?;
        let idx = ctx
            .bus
            .find_peripheral_index(addr)
            .ok_or_else(|| anyhow!("l6362a: EN GPIO block missing"))?;
        let cell = Arc::new(AtomicBool::new(false));
        ensure!(
            ctx.bus.peripherals[idx]
                .dev
                .watch_pad_level(bit, cell.clone()),
            "l6362a: EN pad observation unsupported"
        );
        let gate = L6362a::new(ctx.device_id(), PinSense::Cell(cell), in1);
        let in2_label = ctx
            .config_str("in2")
            .ok_or_else(|| anyhow!("l6362a requires config.in2"))?
            .to_string();
        let in2 = ctx
            .resolve_pin_odr(&in2_label)
            .ok_or_else(|| anyhow!("l6362a: IN2 GPIO block missing"))?;
        let mut attached = Pins {
            gate: gate.clone(),
            in2,
            outiq: pad(ctx, "outiq")?,
            diag: pad(ctx, "diag")?,
            ol: pad(ctx, "ol")?,
            hz: ctx.bus.cpu_hz,
            edges: [ctx.resolve_pin_odr(&en_label).map(|p| p.0), Some(in2.0)]
                .into_iter()
                .flatten()
                .collect(),
            last: None,
        };
        let now = ctx.bus.current_cycle;
        attached.service(ctx.bus, now);
        ctx.uart()?.set_digital_transceiver(Arc::new(gate))?;
        ctx.bus.gpio_devices.push(Box::new(attached));
        Ok(())
    }
}
