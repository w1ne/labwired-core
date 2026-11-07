// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Dual-input H-bridge channel twin (L298N / TB6612 / L293D / BTS7960 IBT-2).
//!
//! Tracks IN1/IN2 direction and EN (or PWM) enable level. Exposes a signed
//! "effort" in [-1, +1] for UI/oracle: +1 = forward, -1 = reverse, 0 = brake/coast.
//! IBT-2 / BTS7960 boards use LPWM/RPWM + L_EN/R_EN instead of IN1/IN2/EN:
//! RPWM drives M+ and LPWM drives M-; positive terminal voltage is RPWM high,
//! LPWM low. Reversing the motor leads reverses mechanical direction. Both enables must
//! be high. Digital GPIO edges only for v1 — duty-aware effort is not modelled.
//! No motor dynamics — honest for direction + enable labs.

use std::sync::Mutex;

#[derive(Debug, Default)]
struct State {
    in1: bool,
    in2: bool,
    /// Primary enable (EN / L_EN). True when the pin is absent.
    en: bool,
    /// Second enable (R_EN on IBT-2). True when the pin is absent.
    en2: bool,
    commanded: bool,
}

/// One H-bridge output channel.
#[derive(Debug)]
pub struct HBridgeMotor {
    in1_pin: u8,
    in2_pin: u8,
    en_pin: Option<u8>,
    /// Optional second enable (IBT-2 R_EN). When set, effort requires both EN high.
    en2_pin: Option<u8>,
    state: Mutex<State>,
    id: String,
    declared_id: Option<String>,
}

impl HBridgeMotor {
    pub fn new(id: impl Into<String>, in1: u8, in2: u8, en: Option<u8>) -> Self {
        Self::new_with_enables(id, in1, in2, en, None)
    }

    /// IBT-2 / BTS7960: LPWM/RPWM plus optional L_EN and R_EN (both must be high).
    pub fn new_ibt2(
        id: impl Into<String>,
        lpwm: u8,
        rpwm: u8,
        l_en: Option<u8>,
        r_en: Option<u8>,
    ) -> Self {
        Self::new_with_enables(id, rpwm, lpwm, l_en, r_en)
    }

    fn new_with_enables(
        id: impl Into<String>,
        in1: u8,
        in2: u8,
        en: Option<u8>,
        en2: Option<u8>,
    ) -> Self {
        Self {
            in1_pin: in1,
            in2_pin: in2,
            en_pin: en,
            en2_pin: en2,
            state: Mutex::new(State {
                // Absent enable pins read as always-on.
                en: en.is_none(),
                en2: en2.is_none(),
                ..State::default()
            }),
            id: id.into(),
            declared_id: None,
        }
    }

    /// Record the `external_devices:` entry this channel was built from.
    ///
    /// One H-bridge declaration builds up to two channel models (`<id>-a`,
    /// `<id>-b`), so [`Self::id`] is NOT the manifest id. Inspect joins a
    /// bus-resident device to its declaration by name, and without this the
    /// channels would report as undeclared hardware on a rig that plainly
    /// declared them.
    pub fn with_declared_id(mut self, declared: impl Into<String>) -> Self {
        self.declared_id = Some(declared.into());
        self
    }

    /// The manifest entry this channel came from, when it came from one.
    pub fn declared_id(&self) -> Option<&str> {
        self.declared_id.as_deref()
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Signed effort in [-1.0, 1.0].
    pub fn effort(&self) -> f32 {
        let s = self.state.lock().unwrap();
        if !(s.en && s.en2) {
            return 0.0;
        }
        // Positive means OUT1/M+ is high relative to OUT2/M-. Equal inputs
        // report zero signed effort; the physical motor model distinguishes
        // the brake and coast drive states.
        match (s.in1, s.in2) {
            (true, false) => 1.0,
            (false, true) => -1.0,
            _ => 0.0,
        }
    }

    pub fn is_commanded(&self) -> bool {
        self.state.lock().unwrap().commanded
    }

    pub fn on_gpio_edge(&self, pin: u8, to: bool, _sim_cycle: u64) {
        let mut s = self.state.lock().unwrap();
        // Several bridge terminals can share one MCU pad (commonly L_EN and
        // R_EN). An edge changes every connected terminal, not only the first.
        if pin == self.in1_pin {
            s.in1 = to;
            s.commanded = true;
        }
        if pin == self.in2_pin {
            s.in2 = to;
            s.commanded = true;
        }
        if self.en_pin == Some(pin) {
            s.en = to;
            s.commanded = true;
        }
        if self.en2_pin == Some(pin) {
            s.en2 = to;
            s.commanded = true;
        }
    }
}

impl crate::peripherals::device::GpioObserver for HBridgeMotor {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_gpio_edge(pin, to, sim_cycle);
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};
use std::sync::Arc;

const DEVICE_OUTPUT_PREFIX: &str = "device-output://";

#[derive(Debug, Clone)]
enum BridgeControlSource {
    Gpio(u8),
    DeviceOutput {
        device_id: String,
        role: String,
        virtual_pin: u8,
    },
}

impl BridgeControlSource {
    fn observer_pin(&self) -> u8 {
        match self {
            Self::Gpio(pin) => *pin,
            Self::DeviceOutput { virtual_pin, .. } => *virtual_pin,
        }
    }
}

fn parse_device_output_ref(label: &str) -> Option<(String, String)> {
    let rest = label.strip_prefix(DEVICE_OUTPUT_PREFIX)?;
    let (device_id, role) = rest.rsplit_once('/')?;
    if device_id.is_empty() || role.is_empty() {
        return None;
    }
    Some((device_id.to_string(), role.to_string()))
}

fn bridge_control_source(
    ctx: &AttachCtx<'_>,
    keys: &[&str],
    default: Option<&str>,
    virtual_pin: u8,
) -> anyhow::Result<Option<BridgeControlSource>> {
    let label = keys.iter().find_map(|key| ctx.config_str(key)).or(default);
    let Some(label) = label else {
        return Ok(None);
    };
    if let Some((device_id, role)) = parse_device_output_ref(label) {
        return Ok(Some(BridgeControlSource::DeviceOutput {
            device_id,
            role,
            virtual_pin,
        }));
    }
    let pin = ctx.parse_gpio_pin(label).ok_or_else(|| {
        anyhow::anyhow!(
            "{} '{}': control '{}' is neither a GPIO pad nor a {}<device>/<role> reference",
            ctx.device_type(),
            ctx.device_id(),
            label,
            DEVICE_OUTPUT_PREFIX,
        )
    })?;
    Ok(Some(BridgeControlSource::Gpio(pin)))
}

fn bind_bridge_sources(
    ctx: &mut AttachCtx<'_>,
    motor: &Arc<HBridgeMotor>,
    sources: &[&BridgeControlSource],
) {
    if sources
        .iter()
        .any(|source| matches!(source, BridgeControlSource::Gpio(_)))
    {
        ctx.install_gpio_observer(motor.clone());
    }
    for source in sources {
        if let BridgeControlSource::DeviceOutput {
            device_id,
            role,
            virtual_pin,
        } = source
        {
            ctx.bind_device_output_observer(
                device_id.clone(),
                role.clone(),
                motor.clone(),
                *virtual_pin,
            );
        }
    }
}

/// Dual H-bridge motor kit (L298N / TB6612 / L293D-class).
pub struct HBridgeMotorKit;
pub static H_BRIDGE_MOTOR_KIT: HBridgeMotorKit = HBridgeMotorKit;

static H_BRIDGE_METADATA: KitMetadata = KitMetadata {
    inputs: std::borrow::Cow::Borrowed(&[]),
    device_type: std::borrow::Cow::Borrowed("l298n"),
    label: std::borrow::Cow::Borrowed("H-bridge motor driver"),
    summary: std::borrow::Cow::Borrowed(
        "L298N/TB6612/L293D-class dual H-bridge twin (direction + enable effort).",
    ),
    detail: std::borrow::Cow::Borrowed(
        "Channel A from IN1/IN2/ENA (or AIN1/AIN2/PWMA). Optional channel B when \
         IN3/IN4 or BIN* keys are present. IBT-2 / BTS7960 boards use LPWM/RPWM \
         plus L_EN/R_EN: RPWM drives M+, LPWM drives M-, both enables must be \
         high, and unwired enables are low. Digital on/off effort ±1; PWM duty \
         is not observed. Aliases: tb6612, l293d, bts7960, ibt-2, ibt2.",
    ),
    transport: Transport::GpioGroup,
    category: Category::Gpio,
    config_keys: std::borrow::Cow::Borrowed(&[
        ConfigKey {
            name: std::borrow::Cow::Borrowed("in1_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("Channel A input 1 (or ain1_pin)."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("in2_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("Channel A input 2 (or ain2_pin)."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("en_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("Channel A enable (or pwma_pin)."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("lpwm_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed(
                "IBT-2 LPWM controls M- (negative terminal voltage); synonym LPWM.",
            ),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("rpwm_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed(
                "IBT-2 RPWM controls M+ (positive terminal voltage); synonym RPWM.",
            ),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("l_en_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("IBT-2 L_EN enable; synonym LEN."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("r_en_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("IBT-2 R_EN enable; synonym REN."),
        },
    ]),
    labs: std::borrow::Cow::Borrowed(&[]),
};

impl PeripheralKit for HBridgeMotorKit {
    fn metadata(&self) -> &'static KitMetadata {
        &H_BRIDGE_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let ibt2 = matches!(ctx.device_type(), "bts7960" | "ibt-2" | "ibt2")
            || ctx.ext.config.contains_key("lpwm_pin")
            || ctx.ext.config.contains_key("LPWM")
            || ctx.ext.config.contains_key("rpwm_pin")
            || ctx.ext.config.contains_key("RPWM");
        if ibt2 {
            return attach_ibt2(ctx);
        }

        // Virtual observer pins are intentionally outside every ESP32 GPIO
        // range. They identify direct device-output controls inside each motor
        // object only; no MCU register or pad is allocated.
        const DIRECT_IN1: u8 = 240;
        const DIRECT_IN2: u8 = 241;
        const DIRECT_EN: u8 = 242;

        let in1 = bridge_control_source(
            ctx,
            &["in1_pin", "AIN1", "ain1_pin", "IN1"],
            Some("GPIO16"),
            DIRECT_IN1,
        )?
        .expect("required bridge input has a default");
        let in2 = bridge_control_source(
            ctx,
            &["in2_pin", "AIN2", "ain2_pin", "IN2"],
            Some("GPIO17"),
            DIRECT_IN2,
        )?
        .expect("required bridge input has a default");
        let en =
            bridge_control_source(ctx, &["en_pin", "ENA", "pwma_pin", "PWMA"], None, DIRECT_EN)?;
        let motor = Arc::new(
            HBridgeMotor::new(
                format!("{}-a", ctx.device_id()),
                in1.observer_pin(),
                in2.observer_pin(),
                en.as_ref().map(BridgeControlSource::observer_pin),
            )
            .with_declared_id(ctx.device_id().to_string()),
        );
        let mut channel_a_sources = vec![&in1, &in2];
        if let Some(en) = en.as_ref() {
            channel_a_sources.push(en);
        }
        bind_bridge_sources(ctx, &motor, &channel_a_sources);
        ctx.bus.observe_device(motor);

        let has_b = ctx.ext.config.contains_key("in3_pin")
            || ctx.ext.config.contains_key("IN3")
            || ctx.ext.config.contains_key("bin1_pin")
            || ctx.ext.config.contains_key("BIN1");
        if has_b {
            let in3 = bridge_control_source(
                ctx,
                &["in3_pin", "BIN1", "bin1_pin", "IN3"],
                Some("GPIO18"),
                DIRECT_IN1,
            )?
            .expect("required bridge input has a default");
            let in4 = bridge_control_source(
                ctx,
                &["in4_pin", "BIN2", "bin2_pin", "IN4"],
                Some("GPIO19"),
                DIRECT_IN2,
            )?
            .expect("required bridge input has a default");
            let enb = bridge_control_source(
                ctx,
                &["enb_pin", "ENB", "pwmb_pin", "PWMB"],
                None,
                DIRECT_EN,
            )?;
            let motor_b = Arc::new(
                HBridgeMotor::new(
                    format!("{}-b", ctx.device_id()),
                    in3.observer_pin(),
                    in4.observer_pin(),
                    enb.as_ref().map(BridgeControlSource::observer_pin),
                )
                .with_declared_id(ctx.device_id().to_string()),
            );
            let mut channel_b_sources = vec![&in3, &in4];
            if let Some(enb) = enb.as_ref() {
                channel_b_sources.push(enb);
            }
            bind_bridge_sources(ctx, &motor_b, &channel_b_sources);
            ctx.bus.observe_device(motor_b);
        }
        Ok(())
    }
}

/// Attach the IBT-2 control pins using either a native GPIO observer or the
/// same pad-write seam used by servos on AVR/STM32. Port letters matter: PD7
/// and PB0 are distinct enable pins even though they are on different ports.
fn attach_ibt2(ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
    let label = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| ctx.config_str(key))
            .map(str::to_string)
    };
    let labels = [
        Some(label(&["lpwm_pin", "LPWM", "lpwm"]).unwrap_or_else(|| "GPIO16".into())),
        Some(label(&["rpwm_pin", "RPWM", "rpwm"]).unwrap_or_else(|| "GPIO17".into())),
        Some(label(&["l_en_pin", "L_EN", "LEN", "len_pin"]).unwrap_or_else(|| "GND".into())),
        Some(label(&["r_en_pin", "R_EN", "REN", "ren_pin"]).unwrap_or_else(|| "GND".into())),
    ];
    let parsed = labels
        .each_ref()
        .map(|pin| pin.as_deref().and_then(|p| ctx.parse_gpio_pin(p)));
    let native = labels
        .iter()
        .zip(parsed)
        .all(|(label, pin)| label.is_none() || pin.is_some());
    let ids = if native {
        parsed
    } else {
        [
            Some(0),
            Some(1),
            labels[2].as_ref().map(|_| 2),
            labels[3].as_ref().map(|_| 3),
        ]
    };
    let motor = Arc::new(
        HBridgeMotor::new_ibt2(
            format!("{}-a", ctx.device_id()),
            ids[0].unwrap(),
            ids[1].unwrap(),
            ids[2],
            ids[3],
        )
        .with_declared_id(ctx.device_id().to_string()),
    );
    if !native || !ctx.install_gpio_observer(motor.clone()) {
        let mut pads = Vec::new();
        let mut addrs = Vec::new();
        for (label, id) in labels.iter().zip(ids) {
            if let (Some(label), Some(id)) = (label, id) {
                if let Some(high) = crate::bus::classify_logic_rail(label) {
                    motor.on_gpio_edge(id, high, 0);
                    continue;
                }
                let (addr, bit) = ctx.resolve_pin_odr(label).ok_or_else(|| {
                    anyhow::anyhow!(
                        "BTS7960 '{}': control pin '{label}' is not a resolvable MCU pad",
                        ctx.device_id()
                    )
                })?;
                pads.push((id, addr, bit, None));
                if !addrs.contains(&addr) {
                    addrs.push(addr);
                }
            }
        }
        let mut watch = BridgePadWatch {
            id: ctx.device_id().to_string(),
            motor: motor.clone(),
            pads,
            addrs,
        };
        crate::bus::BusResidentDevice::service(&mut watch, ctx.bus, 0);
        ctx.bus.gpio_devices.push(Box::new(watch));
    }
    ctx.bus.observe_device(motor);
    Ok(())
}

#[derive(Debug)]
struct BridgePadWatch {
    id: String,
    motor: Arc<HBridgeMotor>,
    pads: Vec<(u8, u64, u8, Option<bool>)>,
    addrs: Vec<u64>,
}

impl crate::bus::BusResidentDevice for BridgePadWatch {
    fn service(&mut self, pins: &mut dyn crate::bus::DevicePins, now: u64) {
        for (id, addr, bit, previous) in &mut self.pads {
            // Unknown/high-Z AF outputs do not fall back to a stale ODR bit.
            let high = pins.known_pad_bit(*addr, *bit).unwrap_or(false);
            if *previous != Some(high) {
                *previous = Some(high);
                self.motor.on_gpio_edge(*id, high, now);
            }
        }
    }
    fn needs_per_cycle_service(&self) -> bool {
        // A timer can change an AF pad without a GPIO store. Keep the write
        // hook for immediate bit-banged edges, and sample autonomous outputs
        // on the normal peripheral tick as well.
        true
    }
    fn edge_service_addrs(&self) -> &[u64] {
        &self.addrs
    }
    fn is_attached_device(&self) -> bool {
        false
    }
    fn as_sim_input(&mut self) -> &mut dyn crate::sim_input::SimInput {
        self
    }
    fn id(&self) -> &str {
        &self.id
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl crate::sim_input::SimInput for BridgePadWatch {
    fn input_channels(&self) -> &[crate::sim_input::InputChannel] {
        &[]
    }
    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value).map(|_| ())
    }
}

/// An H-bridge board carries two independent motor channels, so ONE
/// declaration builds TWO models (`<id>-a`, `<id>-b`). Each reports its own
/// channel identity as [`model_id`](crate::bus::ObservedDevice::model_id) and
/// both join back to the declaration they came from — neither is anonymous,
/// and neither claims to be the whole board. A single-channel board declares
/// no channel id and is the whole of what was declared.
impl crate::bus::ObservedDevice for HBridgeMotor {
    fn manifest_id(&self) -> &str {
        HBridgeMotor::declared_id(self).unwrap_or_else(|| self.id())
    }

    fn model_id(&self) -> Option<&str> {
        HBridgeMotor::declared_id(self).map(|_| self.id())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_arc_any(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_when_in1_high() {
        let m = HBridgeMotor::new("a", 1, 2, Some(3));
        m.on_gpio_edge(3, true, 0);
        m.on_gpio_edge(1, true, 1);
        m.on_gpio_edge(2, false, 2);
        assert_eq!(m.effort(), 1.0);
    }

    #[test]
    fn reverse_when_in2_high() {
        let m = HBridgeMotor::new("a", 1, 2, Some(3));
        m.on_gpio_edge(3, true, 0);
        m.on_gpio_edge(1, false, 1);
        m.on_gpio_edge(2, true, 2);
        assert_eq!(m.effort(), -1.0);
    }

    /// Electrical terminal polarity: RPWM high makes M+ positive to M-.
    #[test]
    fn ibt2_forward_when_rpwm_high() {
        // pins: LPWM=1, RPWM=2, L_EN=3, R_EN=4
        let m = HBridgeMotor::new_ibt2("ibt", 1, 2, Some(3), Some(4));
        m.on_gpio_edge(3, true, 0);
        m.on_gpio_edge(4, true, 1);
        m.on_gpio_edge(1, false, 2);
        m.on_gpio_edge(2, true, 3);
        assert_eq!(m.effort(), 1.0);
    }

    /// LPWM high makes the opposite terminal positive.
    #[test]
    fn ibt2_reverse_when_lpwm_high() {
        let m = HBridgeMotor::new_ibt2("ibt", 1, 2, Some(3), Some(4));
        m.on_gpio_edge(3, true, 0);
        m.on_gpio_edge(4, true, 1);
        m.on_gpio_edge(1, true, 2);
        m.on_gpio_edge(2, false, 3);
        assert_eq!(m.effort(), -1.0);
    }

    /// Both PWM low gives zero signed effort. With both inhibits high the
    /// actual bridge shorts the terminals (brake); the physical motor model
    /// handles that distinction, which this digital observer does not expose.
    #[test]
    fn ibt2_zero_effort_when_both_pwm_low() {
        let m = HBridgeMotor::new_ibt2("ibt", 1, 2, Some(3), Some(4));
        m.on_gpio_edge(3, true, 0);
        m.on_gpio_edge(4, true, 1);
        m.on_gpio_edge(1, false, 2);
        m.on_gpio_edge(2, false, 3);
        assert_eq!(m.effort(), 0.0);
    }

    /// Both enables must be high — one low disables the bridge.
    #[test]
    fn ibt2_disabled_when_one_en_low() {
        let m = HBridgeMotor::new_ibt2("ibt", 1, 2, Some(3), Some(4));
        m.on_gpio_edge(3, true, 0);
        m.on_gpio_edge(4, false, 1);
        m.on_gpio_edge(1, true, 2);
        assert_eq!(m.effort(), 0.0);
    }
}
