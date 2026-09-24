// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Dual-input H-bridge channel twin (L298N / TB6612 / L293D / BTS7960 IBT-2).
//!
//! Tracks IN1/IN2 direction and EN (or PWM) enable level. Exposes a signed
//! "effort" in [-1, +1] for UI/oracle: +1 = forward, -1 = reverse, 0 = brake/coast.
//! IBT-2 / BTS7960 boards use LPWM/RPWM + L_EN/R_EN instead of IN1/IN2/EN:
//! Modulshop Arduino example maps speed>0 → LPWM=speed RPWM=0 (forward),
//! speed<0 → opposite (reverse), 0 → both PWM low (coast). Both enables must
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
        Self::new_with_enables(id, lpwm, rpwm, l_en, r_en)
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
        // L298N: IN1 high / IN2 low = forward. IBT-2 Arduino example: LPWM
        // high / RPWM low = forward (speed>0). Both-high is brake; both-low
        // is coast — effort 0 either way for this digital twin.
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
        if pin == self.in1_pin {
            s.in1 = to;
            s.commanded = true;
        } else if pin == self.in2_pin {
            s.in2 = to;
            s.commanded = true;
        } else if self.en_pin == Some(pin) {
            s.en = to;
            s.commanded = true;
        } else if self.en2_pin == Some(pin) {
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
             IN3/IN4 or BIN* keys are present. IBT-2 / BTS7960 boards use              LPWM/RPWM + L_EN/R_EN (Modulshop Arduino: speed>0 → LPWM, speed<0 →              RPWM, 0 → coast; both enables high). Digital on/off effort ±1 for              v1 — PWM duty is not observed. Aliases: tb6612, l293d, bts7960,              ibt-2, ibt2.",
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
            doc: std::borrow::Cow::Borrowed("IBT-2 LPWM (forward PWM); synonym LPWM."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("rpwm_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("IBT-2 RPWM (reverse PWM); synonym RPWM."),
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
        let ibt2 = ctx.ext.config.contains_key("lpwm_pin")
            || ctx.ext.config.contains_key("LPWM")
            || ctx.ext.config.contains_key("rpwm_pin")
            || ctx.ext.config.contains_key("RPWM");
        if ibt2 {
            let lpwm = ctx
                .config_gpio_pin("lpwm_pin", "LPWM", "GPIO16")
                .or_else(|_| ctx.config_gpio_pin("LPWM", "lpwm", "GPIO16"))?;
            let rpwm = ctx
                .config_gpio_pin("rpwm_pin", "RPWM", "GPIO17")
                .or_else(|_| ctx.config_gpio_pin("RPWM", "rpwm", "GPIO17"))?;
            let l_en = ctx
                .config_str("l_en_pin")
                .or_else(|| ctx.config_str("L_EN"))
                .or_else(|| ctx.config_str("LEN"))
                .or_else(|| ctx.config_str("len_pin"))
                .and_then(|l| ctx.parse_gpio_pin(l));
            let r_en = ctx
                .config_str("r_en_pin")
                .or_else(|| ctx.config_str("R_EN"))
                .or_else(|| ctx.config_str("REN"))
                .or_else(|| ctx.config_str("ren_pin"))
                .and_then(|l| ctx.parse_gpio_pin(l));
            let motor = Arc::new(
                HBridgeMotor::new_ibt2(format!("{}-a", ctx.device_id()), lpwm, rpwm, l_en, r_en)
                    .with_declared_id(ctx.device_id().to_string()),
            );
            ctx.install_gpio_observer(motor.clone());
            ctx.bus.observe_device(motor);
            return Ok(());
        }

        let in1 = ctx
            .config_gpio_pin("in1_pin", "AIN1", "GPIO16")
            .or_else(|_| ctx.config_gpio_pin("ain1_pin", "IN1", "GPIO16"))?;
        let in2 = ctx
            .config_gpio_pin("in2_pin", "AIN2", "GPIO17")
            .or_else(|_| ctx.config_gpio_pin("ain2_pin", "IN2", "GPIO17"))?;
        let en = ctx
            .config_str("en_pin")
            .or_else(|| ctx.config_str("ENA"))
            .or_else(|| ctx.config_str("pwma_pin"))
            .or_else(|| ctx.config_str("PWMA"))
            .and_then(|l| ctx.parse_gpio_pin(l));
        let motor = Arc::new(
            HBridgeMotor::new(format!("{}-a", ctx.device_id()), in1, in2, en)
                .with_declared_id(ctx.device_id().to_string()),
        );
        ctx.install_gpio_observer(motor.clone());
        ctx.bus.observe_device(motor);

        let has_b = ctx.ext.config.contains_key("in3_pin")
            || ctx.ext.config.contains_key("IN3")
            || ctx.ext.config.contains_key("bin1_pin")
            || ctx.ext.config.contains_key("BIN1");
        if has_b {
            if let (Ok(b1), Ok(b2)) = (
                ctx.config_gpio_pin("in3_pin", "BIN1", "GPIO18")
                    .or_else(|_| ctx.config_gpio_pin("bin1_pin", "IN3", "GPIO18")),
                ctx.config_gpio_pin("in4_pin", "BIN2", "GPIO19")
                    .or_else(|_| ctx.config_gpio_pin("bin2_pin", "IN4", "GPIO19")),
            ) {
                let enb = ctx
                    .config_str("enb_pin")
                    .or_else(|| ctx.config_str("ENB"))
                    .or_else(|| ctx.config_str("pwmb_pin"))
                    .or_else(|| ctx.config_str("PWMB"))
                    .and_then(|l| ctx.parse_gpio_pin(l));
                let motor_b = Arc::new(
                    HBridgeMotor::new(format!("{}-b", ctx.device_id()), b1, b2, enb)
                        .with_declared_id(ctx.device_id().to_string()),
                );
                ctx.install_gpio_observer(motor_b.clone());
                ctx.bus.observe_device(motor_b);
            }
        }
        Ok(())
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

    /// Modulshop IBT-2 Arduino example: speed>0 → LPWM high, RPWM low.
    #[test]
    fn ibt2_forward_when_lpwm_high() {
        // pins: LPWM=1, RPWM=2, L_EN=3, R_EN=4
        let m = HBridgeMotor::new_ibt2("ibt", 1, 2, Some(3), Some(4));
        m.on_gpio_edge(3, true, 0);
        m.on_gpio_edge(4, true, 1);
        m.on_gpio_edge(1, true, 2);
        m.on_gpio_edge(2, false, 3);
        assert_eq!(m.effort(), 1.0);
    }

    /// Modulshop IBT-2 Arduino example: speed<0 → LPWM low, RPWM high.
    #[test]
    fn ibt2_reverse_when_rpwm_high() {
        let m = HBridgeMotor::new_ibt2("ibt", 1, 2, Some(3), Some(4));
        m.on_gpio_edge(3, true, 0);
        m.on_gpio_edge(4, true, 1);
        m.on_gpio_edge(1, false, 2);
        m.on_gpio_edge(2, true, 3);
        assert_eq!(m.effort(), -1.0);
    }

    /// speed==0 → both PWM low → coast.
    #[test]
    fn ibt2_coast_when_both_pwm_low() {
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
