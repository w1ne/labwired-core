// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! TIOL112 digital IO-Link transceiver, TI SLLSFJ1D §§6.7, 8.3.6.
//!
//! EN low releases CQ. EN high drives CQ = !TX; RX = !CQ, including local
//! echo. WAKE and NFAULT are active-low open-drain outputs, with ideal board
//! pull-ups. The fault/wake inputs are explicit test stimuli, not an analog
//! current-limit or wake-pulse detector. Configured TX/RX propagation delays
//! apply only on a TimedUartNet; ordinary stream mode remains byte-functional.
//! EN settling/release use configurable scenarios, defaulting to the 4 us
//! typical values. They are not guaranteed maximum bounds. Voltage thresholds,
//! cable loading, slew and analog protection are outside this model.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, ensure, Result};

use crate::bus::{BusResidentDevice, DevicePins};
use crate::network::timed_uart::{cycles_to_ps, ps_to_cycles_ceil, LineFormat, TimedUartPort};
use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};
use crate::peripherals::rs485::PinSense;
use crate::sim_input::{InputChannel, SimInput, SimInputError};

pub const DRIVER_MAX_DELAY_PS: u64 = 1_200_000;
pub const RECEIVER_MAX_DELAY_PS: u64 = 300_000;
const TRACE_CAP: usize = 4096;

/// A byte actually admitted to the digital CQ driver, with physical polarity.
#[derive(Debug, Clone)]
pub struct CqFrame {
    pub start_ps: u64,
    pub stop_end_ps: u64,
    pub value: u16,
    pub format: LineFormat,
    pub truncated_ps: Option<u64>,
    pub en_rise_ps: u64,
}

impl CqFrame {
    /// Driven CQ level at `at_ps`; None outside this character or after EN fell.
    /// Idle UART TX=1 maps to CQ=0 while EN is high.
    pub fn cq_level_at(&self, at_ps: u64) -> Option<bool> {
        if at_ps < self.start_ps
            || at_ps >= self.stop_end_ps
            || self.truncated_ps.is_some_and(|cut| at_ps >= cut)
        {
            return None;
        }
        Some(
            !self
                .format
                .level_of_bit(self.value, (at_ps - self.start_ps) / self.format.bit_ps),
        )
    }
}

#[derive(Debug, Default)]
struct State {
    wake: bool,
    fault: bool,
    tx_frames: VecDeque<CqFrame>,
    tx_frames_total: u64,
    disabled_tx: u64,
    contention: u64,
    en_rise_ps: u64,
    enable_ready_ps: u64,
    disable_at_ps: u64,
    enable_not_ready: u64,
    clock_error: Option<String>,
    timed_port: Option<TimedUartPort>,
}

#[derive(Debug, Clone)]
struct ClockContract {
    registers: Arc<crate::peripherals::stm32g0_clock::G0ClockRegisters>,
    fixed_hz: u64,
    usart: u8,
}

/// UART-owned gate. It is deliberately separate from the RS-485 gate: there is
/// no /RE pin, CQ is inverted, and WAKE/NFAULT belong to the TI device.
#[derive(Debug, Clone)]
pub struct Tiol112 {
    id: String,
    en: PinSense,
    state: Arc<Mutex<State>>,
    pub driver_delay_ps: u64,
    pub receiver_delay_ps: u64,
    /// Explicit scenario assumptions;4us is typical, not a maximum.
    pub enable_delay_ps: u64,
    pub disable_delay_ps: u64,
    clock_contract: Option<ClockContract>,
}

impl Tiol112 {
    pub fn new(id: impl Into<String>, en: PinSense) -> Self {
        Self {
            id: id.into(),
            en,
            state: Arc::new(Mutex::new(State::default())),
            driver_delay_ps: DRIVER_MAX_DELAY_PS,
            receiver_delay_ps: RECEIVER_MAX_DELAY_PS,
            enable_delay_ps: 4_000_000,
            disable_delay_ps: 4_000_000,
            clock_contract: None,
        }
    }
    pub fn driver_enabled(&self) -> bool {
        self.en.level()
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Set the logical WAKE stimulus. `true` drives the configured active-low
    /// WAKE output on the next resident-device service. This does not model
    /// detection of a physical current pulse on CQ.
    pub fn set_wake_asserted(&self, asserted: bool) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).wake = asserted;
    }
    pub fn clock_error(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clock_error
            .clone()
    }
    pub fn enable_not_ready_count(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .enable_not_ready
    }
    pub(crate) fn clock_valid(&self) -> bool {
        let Some(contract) = &self.clock_contract else {
            return true;
        };
        if let Err(error) = contract
            .registers
            .validate(contract.fixed_hz, contract.usart)
        {
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clock_error = Some(error.to_string());
            return false;
        }
        true
    }
    pub(crate) fn bind_timed_port(&self, port: TimedUartPort) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .timed_port = Some(port);
    }
    fn note_enable(&self, enabled: bool, cycle: u64, hz: u64) {
        let now_ps = cycles_to_ps(cycle, hz);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if enabled {
            state.en_rise_ps = now_ps;
            state.enable_ready_ps = now_ps.saturating_add(self.enable_delay_ps);
        } else {
            let cut = now_ps.saturating_add(self.disable_delay_ps);
            state.disable_at_ps = cut;
            if let Some(frame) = state.tx_frames.back_mut() {
                if frame.start_ps < cut && cut < frame.stop_end_ps {
                    frame.truncated_ps = Some(cut);
                }
            }
            // abort_tx takes MCU-pin time; endpoint propagation is already on
            // the wire, so subtract driver delay to place release at CQ.
            let port = state.timed_port.clone();
            drop(state);
            if let Some(port) = port {
                port.abort_tx(ps_to_cycles_ceil(
                    cut.saturating_sub(self.driver_delay_ps),
                    hz,
                ));
            }
        }
    }
    pub(crate) fn cq_driving_at(&self, cycle: u64, hz: u64) -> bool {
        let now = cycles_to_ps(cycle, hz);
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if self.driver_enabled() {
            now >= state.enable_ready_ps
        } else {
            now < state.disable_at_ps
        }
    }
    pub(crate) fn allow_timed_tx(&self, start: u64, hz: u64) -> bool {
        if !self.allow_tx() {
            return false;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if cycles_to_ps(start, hz) < state.enable_ready_ps {
            state.enable_not_ready += 1;
            // Do not synthesize a clean start bit from a partly enabled driver.
            return false;
        }
        true
    }
    /// TI truth table. A disabled driver is high impedance, not logic low.
    pub fn cq_drive(en: bool, tx: bool) -> Option<bool> {
        en.then_some(!tx)
    }
    pub fn receiver_level(cq: bool) -> bool {
        !cq
    }
    pub fn tx_frames(&self) -> Vec<CqFrame> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tx_frames
            .iter()
            .cloned()
            .collect()
    }
    pub fn tx_frame_count(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tx_frames_total
    }
    pub fn disabled_tx_count(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .disabled_tx
    }
    pub fn contention_count(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contention
    }
    /// A master byte cannot be trusted while the device drives the same CQ line.
    pub(crate) fn accept_peer_byte(&self) -> bool {
        if !self.clock_valid() {
            return false;
        }
        if !self.driver_enabled() {
            return true;
        }
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contention += 1;
        false
    }
    pub(crate) fn allow_tx(&self) -> bool {
        if !self.clock_valid() {
            return false;
        }
        if self.driver_enabled() {
            return true;
        }
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .disabled_tx += 1;
        false
    }
    pub(crate) fn record_timed_tx(
        &self,
        start_cycle: u64,
        value: u16,
        format: LineFormat,
        hz: u64,
    ) {
        let start_ps = cycles_to_ps(start_cycle, hz).saturating_add(self.driver_delay_ps);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.tx_frames.len() == TRACE_CAP {
            state.tx_frames.pop_front();
        }
        let en_rise_ps = state.en_rise_ps;
        state.tx_frames_total += 1;
        state.tx_frames.push_back(CqFrame {
            start_ps,
            stop_end_ps: start_ps.saturating_add(format.frame_ps()),
            value,
            format,
            truncated_ps: None,
            en_rise_ps,
        });
    }
}

static INPUTS: &[InputChannel] = &[
    InputChannel {
        key: Cow::Borrowed("wake"),
        label: Cow::Borrowed("WAKE asserted"),
        unit: Cow::Borrowed("bool"),
        min: 0.0,
        max: 1.0,
        default: Some(0.0),
    },
    InputChannel {
        key: Cow::Borrowed("fault"),
        label: Cow::Borrowed("NFAULT asserted"),
        unit: Cow::Borrowed("bool"),
        min: 0.0,
        max: 1.0,
        default: Some(0.0),
    },
];

#[derive(Debug)]
struct StatusPins {
    gate: Tiol112,
    wake: Option<(u64, u8)>,
    nfault: Option<(u64, u8)>,
    last: Option<(bool, bool)>,
    last_en: bool,
    hz: u64,
    en_edges: Vec<u64>,
}

impl SimInput for StatusPins {
    fn input_channels(&self) -> &[InputChannel] {
        INPUTS
    }
    fn set_input(&mut self, key: &str, value: f64) -> std::result::Result<(), SimInputError> {
        self.require_channel(key, value)?;
        let mut state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        match key {
            "wake" => state.wake = value != 0.0,
            "fault" => state.fault = value != 0.0,
            _ => unreachable!(),
        }
        Ok(())
    }
    fn component_id(&self) -> Option<&str> {
        Some(self.gate.id())
    }
}

impl BusResidentDevice for StatusPins {
    fn service(&mut self, pins: &mut dyn DevicePins, now: u64) {
        let enabled = self.gate.driver_enabled();
        if enabled != self.last_en {
            self.gate.note_enable(enabled, now, self.hz);
            self.last_en = enabled;
        }
        let state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        let levels = (!state.wake, !state.fault);
        drop(state);
        if self.last == Some(levels) {
            return;
        }
        for (pad, level) in [(self.wake, levels.0), (self.nfault, levels.1)] {
            if let Some((addr, bit)) = pad {
                pins.drive_input_bit(addr, bit, level);
            }
        }
        self.last = Some(levels);
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
        self.gate.id()
    }
    fn edge_service_addrs(&self) -> &[u64] {
        &self.en_edges
    }
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        let mut lines = Vec::new();
        if let Some(error) = self.gate.clock_error() {
            lines.push(format!("clock rejected: {error}"));
        }
        lines.push(format!(
            "enable_not_ready={} disabled_tx={} contention={}",
            self.gate.enable_not_ready_count(),
            self.gate.disabled_tx_count(),
            self.gate.contention_count()
        ));
        vec![crate::peripheral_log::PeripheralLog::new("tiol112", lines)]
    }
    // Status changes reach the GPIO on the next peripheral service. Keep the
    // ordinary resident-device walk enabled; the special immediate button
    // apply path does not service transceivers.
}

pub struct Tiol112Kit;
pub static TIOL112_KIT: Tiol112Kit = Tiol112Kit;
static METADATA: KitMetadata = KitMetadata {
    device_type: Cow::Borrowed("tiol112"), label: Cow::Borrowed("TIOL112"),
    summary: Cow::Borrowed("TI IO-Link digital transceiver: EN, inverted CQ, WAKE and NFAULT."),
    detail: Cow::Borrowed("Digital behavior from TI SLLSFJ1D. Timed UART models TX/RX propagation; ordinary streams are byte-functional. Analog electrical behavior, wake-current detection and enable-time bounds are not modeled."),
    transport: Transport::Uart, category: Category::Uart,
    config_keys: Cow::Borrowed(&[
        ConfigKey { name: Cow::Borrowed("en"), ty: ConfigType::Str, doc: Cow::Borrowed("Required driver-enable pad, or high/low rail. Low releases CQ.") },
        ConfigKey { name: Cow::Borrowed("wake"), ty: ConfigType::Str, doc: Cow::Borrowed("Optional WAKE active-low output pad, ideal pull-up.") },
        ConfigKey { name: Cow::Borrowed("nfault"), ty: ConfigType::Str, doc: Cow::Borrowed("Optional NFAULT active-low output pad, ideal pull-up.") },
        ConfigKey { name: Cow::Borrowed("driver_delay_ns"), ty: ConfigType::Int, doc: Cow::Borrowed("Timed-mode TX-to-CQ propagation, 0..1200 ns; default datasheet maximum1200.") },
        ConfigKey { name: Cow::Borrowed("receiver_delay_ns"), ty: ConfigType::Int, doc: Cow::Borrowed("Timed-mode CQ-to-RX propagation, 0..300 ns; default datasheet maximum300.") },
        ConfigKey { name: Cow::Borrowed("enable_delay_ns"), ty: ConfigType::Int, doc: Cow::Borrowed("Timed EN settling scenario, default4000ns TYPICAL, not a guaranteed maximum. Premature frames are rejected.") },
        ConfigKey { name: Cow::Borrowed("disable_delay_ns"), ty: ConfigType::Int, doc: Cow::Borrowed("Timed EN release scenario, default4000ns TYPICAL, not a guaranteed maximum.") },
    ]),
    labs: Cow::Borrowed(&[]), inputs: Cow::Borrowed(INPUTS),
};

fn pad(ctx: &AttachCtx<'_>, key: &str) -> Result<Option<(u64, u8)>> {
    let Some(value) = ctx.ext.config.get(key) else {
        return Ok(None);
    };
    let label = value
        .as_str()
        .ok_or_else(|| anyhow!("tiol112: {key} must name a GPIO pad"))?;
    Ok(Some(
        crate::bus::SystemBus::resolve_pin_idr_pub(ctx.bus, label)
            .ok_or_else(|| anyhow!("tiol112: {key} '{label}' is not a GPIO pad on this chip"))?,
    ))
}

impl PeripheralKit for Tiol112Kit {
    fn metadata(&self) -> &KitMetadata {
        &METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        for key in ctx.ext.config.keys() {
            ensure!(
                matches!(
                    key.as_str(),
                    "en" | "wake"
                        | "nfault"
                        | "driver_delay_ns"
                        | "receiver_delay_ns"
                        | "enable_delay_ns"
                        | "disable_delay_ns"
                ),
                "tiol112: unsupported config key '{key}'"
            );
        }
        let en = ctx
            .config_str("en")
            .ok_or_else(|| anyhow!("tiol112 requires config.en (GPIO pad or high/low)"))?
            .to_string();
        let sense = match en.to_ascii_lowercase().as_str() {
            "high" | "vcc" | "1" => PinSense::Const(true),
            "low" | "gnd" | "0" => PinSense::Const(false),
            _ => {
                let (addr, bit) = pad(ctx, "en")?.expect("en was required");
                let idx = ctx
                    .bus
                    .find_peripheral_index(addr)
                    .ok_or_else(|| anyhow!("tiol112 EN has no GPIO block"))?;
                let cell = Arc::new(AtomicBool::new(false));
                ensure!(
                    ctx.bus.peripherals[idx]
                        .dev
                        .watch_pad_level(bit, cell.clone()),
                    "tiol112: GPIO model cannot observe EN '{en}'"
                );
                PinSense::Cell(cell)
            }
        };
        let mut gate = Tiol112::new(ctx.device_id(), sense);
        if let Some(idx) = ctx.bus.find_peripheral_index_by_name("rcc") {
            if let Some(rcc) = ctx.bus.peripherals[idx]
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<crate::peripherals::rcc::Rcc>())
            {
                if let Some(registers) = rcc.g0_clock_registers() {
                    let usart = match ctx.connection() {
                        "usart1" => 1,
                        "usart2" => 2,
                        _ => anyhow::bail!("G0 TIOL112 supports USART1/2 only"),
                    };
                    gate.clock_contract = Some(ClockContract {
                        registers,
                        fixed_hz: ctx.bus.cpu_hz,
                        usart,
                    });
                }
            }
        }
        for (key, maximum, delay) in [
            ("driver_delay_ns", 1200, &mut gate.driver_delay_ps),
            ("receiver_delay_ns", 300, &mut gate.receiver_delay_ps),
        ] {
            if let Some(value) = ctx.ext.config.get(key) {
                let ns = value
                    .as_u64()
                    .ok_or_else(|| anyhow!("tiol112: {key} must be an integer nanosecond value"))?;
                ensure!(
                    ns <= maximum,
                    "tiol112: {key} exceeds supported datasheet range 0..{maximum}ns"
                );
                *delay = ns * 1000;
            }
        }
        for (key, delay) in [
            ("enable_delay_ns", &mut gate.enable_delay_ps),
            ("disable_delay_ns", &mut gate.disable_delay_ps),
        ] {
            if let Some(value) = ctx.ext.config.get(key) {
                let ns = value
                    .as_u64()
                    .ok_or_else(|| anyhow!("tiol112: {key} must be integer nanoseconds"))?;
                *delay = ns
                    .checked_mul(1000)
                    .ok_or_else(|| anyhow!("tiol112: {key} overflows model time"))?;
            }
        }
        let mut pins = StatusPins {
            gate: gate.clone(),
            wake: pad(ctx, "wake")?,
            nfault: pad(ctx, "nfault")?,
            last: None,
            last_en: gate.driver_enabled(),
            hz: ctx.bus.cpu_hz,
            en_edges: ctx
                .resolve_pin_odr(&en)
                .map(|(addr, _)| vec![addr])
                .unwrap_or_default(),
        };
        let now = ctx.bus.current_cycle;
        pins.service(ctx.bus, now);
        ctx.uart()?.set_tiol112(gate)?;
        ctx.bus.gpio_devices.push(Box::new(pins));
        Ok(())
    }
}
