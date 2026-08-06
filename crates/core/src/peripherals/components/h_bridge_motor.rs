// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Dual-input H-bridge channel twin (L298N / TB6612 / L293D half-bridge).
//!
//! Tracks IN1/IN2 direction and EN (or PWM) enable level. Exposes a signed
//! "effort" in [-1, +1] for UI/oracle: +1 = forward, -1 = reverse, 0 = brake/coast.
//! No motor dynamics — honest for direction + enable labs.

use std::sync::Mutex;

#[derive(Debug, Default)]
struct State {
    in1: bool,
    in2: bool,
    en: bool,
    commanded: bool,
}

/// One H-bridge output channel.
#[derive(Debug)]
pub struct HBridgeMotor {
    in1_pin: u8,
    in2_pin: u8,
    en_pin: Option<u8>,
    state: Mutex<State>,
    id: String,
    declared_id: Option<String>,
}

impl HBridgeMotor {
    pub fn new(id: impl Into<String>, in1: u8, in2: u8, en: Option<u8>) -> Self {
        Self {
            in1_pin: in1,
            in2_pin: in2,
            en_pin: en,
            state: Mutex::new(State {
                en: en.is_none(), // no EN pin → always "enabled"
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
        if !s.en {
            return 0.0;
        }
        match (s.in1, s.in2) {
            (true, false) => 1.0,
            (false, true) => -1.0,
            _ => 0.0, // brake or coast
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
        }
    }
}

impl crate::peripherals::esp32s3::gpio::GpioObserver for HBridgeMotor {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_gpio_edge(pin, to, sim_cycle);
    }
}

impl crate::peripherals::esp32::gpio::GpioObserver for HBridgeMotor {
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
    inputs: &[],
    device_type: "l298n",
    label: "H-bridge motor driver",
    summary: "L298N/TB6612/L293D-class dual H-bridge twin (direction + enable effort).",
    detail: "Channel A from IN1/IN2/ENA (or AIN1/AIN2/PWMA). Optional channel B when \
             IN3/IN4 or BIN* keys are present. Aliases: tb6612, l293d.",
    transport: Transport::GpioGroup,
    category: Category::Gpio,
    config_keys: &[
        ConfigKey {
            name: "in1_pin",
            ty: ConfigType::Str,
            doc: "Channel A input 1 (or ain1_pin).",
        },
        ConfigKey {
            name: "in2_pin",
            ty: ConfigType::Str,
            doc: "Channel A input 2 (or ain2_pin).",
        },
        ConfigKey {
            name: "en_pin",
            ty: ConfigType::Str,
            doc: "Channel A enable (or pwma_pin).",
        },
    ],
    labs: &[],
};

impl PeripheralKit for HBridgeMotorKit {
    fn metadata(&self) -> &'static KitMetadata {
        &H_BRIDGE_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
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
        ctx.bus.h_bridge_motors.push(motor);

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
                ctx.bus.h_bridge_motors.push(motor_b);
            }
        }
        Ok(())
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
}
