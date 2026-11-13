// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STEP/DIR stepper twin for A4988 / DRV8825 / TMC2209-class drivers.
//!
//! Counts rising edges on STEP while EN is active (low or high per config).
//! DIR selects count direction. Exposes step count and shaft angle for
//! observability / UI. No torque dynamics — faithful for position labs.

use std::sync::Mutex;

#[derive(Debug, Clone, Copy)]
pub struct StepDirConfig {
    /// Degrees of shaft rotation per full step (e.g. 1.8° for NEMA17).
    pub degrees_per_step: f64,
    /// When true, EN low enables the driver (A4988/TMC2209 default).
    pub enable_active_low: bool,
}

impl Default for StepDirConfig {
    fn default() -> Self {
        Self {
            degrees_per_step: 1.8,
            enable_active_low: true,
        }
    }
}

#[derive(Debug, Default)]
struct State {
    step_level: bool,
    dir_level: bool,
    en_level: bool,
    /// Signed step accumulator.
    steps: i64,
    commanded: bool,
}

/// STEP/DIR stepper digital twin.
#[derive(Debug)]
pub struct StepDirMotor {
    step_pin: u8,
    dir_pin: u8,
    en_pin: Option<u8>,
    cfg: StepDirConfig,
    min_steps: Option<i64>,
    max_steps: Option<i64>,
    state: Mutex<State>,
    id: String,
    /// Raw YAML `type:` (e.g. `nema17` / `a4988`) for actuator kind export.
    device_type: String,
}

impl StepDirMotor {
    pub fn new(id: impl Into<String>, step_pin: u8, dir_pin: u8, en_pin: Option<u8>) -> Self {
        Self {
            step_pin,
            dir_pin,
            en_pin,
            cfg: StepDirConfig::default(),
            min_steps: None,
            max_steps: None,
            state: Mutex::new(State::default()),
            id: id.into(),
            // Kit attach always overrides via with_device_type(ctx.device_type()).
            // Default is the canonical kit id for hand-built test twins.
            device_type: "a4988".into(),
        }
    }

    pub fn with_config(mut self, cfg: StepDirConfig) -> Self {
        self.cfg = cfg;
        self
    }

    pub fn with_device_type(mut self, device_type: impl Into<String>) -> Self {
        self.device_type = device_type.into();
        self
    }

    /// Seed the shaft at a measured startup position. This is useful for
    /// homing simulations: firmware must drive the mechanism back to a limit
    /// switch instead of starting coincident with it.
    pub fn with_initial_steps(self, steps: i64) -> Self {
        self.state.lock().unwrap().steps = steps;
        self
    }

    pub fn with_step_limits(mut self, min_steps: Option<i64>, max_steps: Option<i64>) -> Self {
        self.min_steps = min_steps;
        self.max_steps = max_steps;
        let mut state = self.state.lock().unwrap();
        if let Some(minimum) = self.min_steps {
            state.steps = state.steps.max(minimum);
        }
        if let Some(maximum) = self.max_steps {
            state.steps = state.steps.min(maximum);
        }
        drop(state);
        self
    }

    pub fn at_min_limit(&self) -> bool {
        self.min_steps
            .is_some_and(|minimum| self.steps() <= minimum)
    }

    pub fn at_max_limit(&self) -> bool {
        self.max_steps
            .is_some_and(|maximum| self.steps() >= maximum)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn device_type(&self) -> &str {
        &self.device_type
    }

    pub fn steps(&self) -> i64 {
        self.state.lock().unwrap().steps
    }

    pub fn angle_degrees(&self) -> f32 {
        self.angle_degrees_f64() as f32
    }

    /// Shaft angle in degrees (for prove `position_rad`).
    pub fn angle_degrees_f64(&self) -> f64 {
        let s = self.state.lock().unwrap();
        s.steps as f64 * self.cfg.degrees_per_step
    }

    pub fn degrees_per_step(&self) -> f64 {
        self.cfg.degrees_per_step
    }

    /// True while the driver is currently enabled (EN active).
    pub fn is_enabled(&self) -> bool {
        let s = self.state.lock().unwrap();
        Self::enabled_level(&self.cfg, s.en_level, self.en_pin.is_some())
    }

    /// True after at least one STEP while enabled since EN last became active.
    /// Cleared when EN goes inactive; with no EN pin, stays true after the
    /// first edge.
    pub fn is_commanded(&self) -> bool {
        self.state.lock().unwrap().commanded
    }

    fn enabled_level(cfg: &StepDirConfig, en_level: bool, has_en: bool) -> bool {
        if !has_en {
            return true;
        }
        if cfg.enable_active_low {
            !en_level
        } else {
            en_level
        }
    }

    pub fn on_gpio_edge(&self, pin: u8, to: bool, _sim_cycle: u64) {
        let mut s = self.state.lock().unwrap();
        if pin == self.step_pin {
            let was = s.step_level;
            s.step_level = to;
            if to && !was {
                // Rising STEP edge
                let enabled = Self::enabled_level(&self.cfg, s.en_level, self.en_pin.is_some());
                if enabled {
                    let delta = if s.dir_level { 1 } else { -1 };
                    let mut next = s.steps.saturating_add(delta);
                    if let Some(minimum) = self.min_steps {
                        next = next.max(minimum);
                    }
                    if let Some(maximum) = self.max_steps {
                        next = next.min(maximum);
                    }
                    s.steps = next;
                    s.commanded = true;
                }
            }
        } else if pin == self.dir_pin {
            s.dir_level = to;
        } else if self.en_pin == Some(pin) {
            s.en_level = to;
            // EN inactive → not currently stepping; clear so control_state /
            // active can read idle after the firmware disables the driver.
            if !Self::enabled_level(&self.cfg, s.en_level, true) {
                s.commanded = false;
            }
        }
    }
}

impl crate::peripherals::device::GpioObserver for StepDirMotor {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, sim_cycle: u64) {
        self.on_gpio_edge(pin, to, sim_cycle);
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};
use std::sync::Arc;

/// STEP/DIR driver kit (A4988 / DRV8825 / TMC2209-class).
pub struct StepDirMotorKit;
pub static STEP_DIR_MOTOR_KIT: StepDirMotorKit = StepDirMotorKit;

static STEP_DIR_METADATA: KitMetadata = KitMetadata {
    inputs: std::borrow::Cow::Borrowed(&[]),
    device_type: std::borrow::Cow::Borrowed("a4988"),
    label: std::borrow::Cow::Borrowed("STEP/DIR stepper driver"),
    summary: std::borrow::Cow::Borrowed(
        "A4988/DRV8825/TMC2209-class STEP/DIR twin (step count + angle).",
    ),
    detail: std::borrow::Cow::Borrowed(
        "Counts rising STEP edges while EN is active (low); DIR selects direction. \
             Type aliases drv8825, tmc2209, and nema17 map here (tmc2209 uses 1/16 microstep cal).",
    ),
    transport: Transport::GpioGroup,
    category: Category::Gpio,
    config_keys: std::borrow::Cow::Borrowed(&[
        ConfigKey {
            name: std::borrow::Cow::Borrowed("step_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("STEP pin (default GPIO16)."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("dir_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("DIR pin (default GPIO17)."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("en_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("Optional EN pin."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("initial_steps"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("Signed shaft position at reset, in configured steps."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("min_steps"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("Optional lower travel stop in signed steps."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("max_steps"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("Optional upper travel stop in signed steps."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("min_limit_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("Optional MCU input asserted at min_steps."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("max_limit_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("Optional MCU input asserted at max_steps."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("limit_active_high"),
            ty: ConfigType::Bool,
            doc: std::borrow::Cow::Borrowed(
                "Limit output polarity; false is active-low (default).",
            ),
        },
    ]),
    labs: std::borrow::Cow::Borrowed(&[]),
};

impl PeripheralKit for StepDirMotorKit {
    fn metadata(&self) -> &'static KitMetadata {
        &STEP_DIR_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let step_label = ctx
            .config_str("step_pin")
            .or_else(|| ctx.config_str("STEP"))
            .unwrap_or("GPIO16")
            .to_string();
        let dir_label = ctx
            .config_str("dir_pin")
            .or_else(|| ctx.config_str("DIR"))
            .unwrap_or("GPIO17")
            .to_string();
        let en_label = ctx
            .config_str("en_pin")
            .or_else(|| ctx.config_str("EN"))
            .map(str::to_string);
        let parsed = [
            ctx.parse_gpio_pin(&step_label),
            ctx.parse_gpio_pin(&dir_label),
            en_label
                .as_deref()
                .and_then(|label| ctx.parse_gpio_pin(label)),
        ];
        let native = parsed[0].is_some()
            && parsed[1].is_some()
            && (en_label.is_none() || parsed[2].is_some());
        let ids = if native {
            parsed
        } else {
            [Some(0), Some(1), en_label.as_ref().map(|_| 2)]
        };
        let min_steps = ctx.config_i64("min_steps");
        let max_steps = ctx.config_i64("max_steps");
        if min_steps.zip(max_steps).is_some_and(|(min, max)| min > max) {
            anyhow::bail!(
                "STEP/DIR '{}' has min_steps greater than max_steps",
                ctx.device_id()
            );
        }
        let mut motor =
            StepDirMotor::new(ctx.device_id(), ids[0].unwrap(), ids[1].unwrap(), ids[2])
                .with_device_type(ctx.device_type());
        if ctx.device_type() == "tmc2209" {
            motor = motor.with_config(StepDirConfig {
                degrees_per_step: 1.8 / 16.0,
                enable_active_low: true,
            });
        }
        if let Some(initial_steps) = ctx.config_i64("initial_steps") {
            motor = motor.with_initial_steps(initial_steps);
        }
        motor = motor.with_step_limits(min_steps, max_steps);
        let motor = Arc::new(motor);
        let installed_native = native && ctx.install_gpio_observer(motor.clone());

        let min_limit_label = ctx.config_str("min_limit_pin").map(str::to_string);
        let max_limit_label = ctx.config_str("max_limit_pin").map(str::to_string);
        if min_limit_label.is_some() && min_steps.is_none() {
            anyhow::bail!(
                "STEP/DIR '{}' min_limit_pin requires min_steps",
                ctx.device_id()
            );
        }
        if max_limit_label.is_some() && max_steps.is_none() {
            anyhow::bail!(
                "STEP/DIR '{}' max_limit_pin requires max_steps",
                ctx.device_id()
            );
        }
        let active_high = ctx.config_bool("limit_active_high").unwrap_or(false);
        let resolve_limit = |label: Option<&str>| -> anyhow::Result<Option<(u64, u8)>> {
            label
                .map(|pin| {
                    crate::bus::SystemBus::resolve_pin_idr_pub(ctx.bus, pin).ok_or_else(|| {
                        anyhow::anyhow!(
                            "STEP/DIR '{}' limit pin '{}' is not a resolvable MCU input",
                            ctx.device_id(),
                            pin
                        )
                    })
                })
                .transpose()
        };
        let min_limit = resolve_limit(min_limit_label.as_deref())?;
        let max_limit = resolve_limit(max_limit_label.as_deref())?;

        if !installed_native || min_limit.is_some() || max_limit.is_some() {
            let mut pads = Vec::new();
            let mut addrs = Vec::new();
            if !installed_native {
                for (id, label) in [
                    (ids[0], Some(step_label.as_str())),
                    (ids[1], Some(dir_label.as_str())),
                    (ids[2], en_label.as_deref()),
                ] {
                    if let (Some(id), Some(label)) = (id, label) {
                        let (addr, bit) = ctx.resolve_pin_odr(label).ok_or_else(|| {
                            anyhow::anyhow!(
                                "STEP/DIR '{}' control pin '{}' is not a resolvable MCU output",
                                ctx.device_id(),
                                label
                            )
                        })?;
                        pads.push((id, addr, bit, None));
                        if !addrs.contains(&addr) {
                            addrs.push(addr);
                        }
                    }
                }
            } else if let Some((addr, _)) = ctx.resolve_pin_odr(&step_label) {
                addrs.push(addr);
            }
            let mut watch = StepDirPadWatch {
                id: ctx.device_id().to_string(),
                motor: motor.clone(),
                pads,
                addrs,
                min_limit: min_limit.map(|(addr, bit)| LimitDrive {
                    addr,
                    bit,
                    active_high,
                    last: None,
                }),
                max_limit: max_limit.map(|(addr, bit)| LimitDrive {
                    addr,
                    bit,
                    active_high,
                    last: None,
                }),
            };
            crate::bus::BusResidentDevice::service(&mut watch, ctx.bus, 0);
            ctx.bus.gpio_devices.push(Box::new(watch));
        }
        ctx.bus.observe_device(motor);
        Ok(())
    }
}

#[derive(Debug)]
struct LimitDrive {
    addr: u64,
    bit: u8,
    active_high: bool,
    last: Option<bool>,
}

#[derive(Debug)]
struct StepDirPadWatch {
    id: String,
    motor: Arc<StepDirMotor>,
    pads: Vec<(u8, u64, u8, Option<bool>)>,
    addrs: Vec<u64>,
    min_limit: Option<LimitDrive>,
    max_limit: Option<LimitDrive>,
}
impl crate::sim_input::SimInput for StepDirPadWatch {
    fn input_channels(&self) -> &[crate::sim_input::InputChannel] {
        &[]
    }
    fn set_input(&mut self, key: &str, _value: f64) -> Result<(), crate::sim_input::SimInputError> {
        Err(crate::sim_input::SimInputError::UnknownChannel(
            key.to_string(),
        ))
    }
    fn component_id(&self) -> Option<&str> {
        Some(&self.id)
    }
}
impl crate::bus::BusResidentDevice for StepDirPadWatch {
    fn service(&mut self, pins: &mut dyn crate::bus::DevicePins, now: u64) {
        for (id, addr, bit, previous) in &mut self.pads {
            let high = pins.known_pad_bit(*addr, *bit).unwrap_or(false);
            if *previous != Some(high) {
                *previous = Some(high);
                self.motor.on_gpio_edge(*id, high, now);
            }
        }
        for (drive, active) in [
            (self.min_limit.as_mut(), self.motor.at_min_limit()),
            (self.max_limit.as_mut(), self.motor.at_max_limit()),
        ] {
            if let Some(drive) = drive {
                let high = if active {
                    drive.active_high
                } else {
                    !drive.active_high
                };
                if drive.last != Some(high) {
                    let _ = pins.drive_input_bit(drive.addr, drive.bit, high);
                    drive.last = Some(high);
                }
            }
        }
    }
    fn needs_per_cycle_service(&self) -> bool {
        false
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

/// Readback only: the bus never drives a STEP/DIR motor, the GPIO observer
/// does. No display surface, so no evidence.
impl crate::bus::ObservedDevice for StepDirMotor {
    fn manifest_id(&self) -> &str {
        self.id()
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
    fn startup_position_is_explicit_and_does_not_mark_the_motor_commanded() {
        let m = StepDirMotor::new("m", 2, 3, Some(4)).with_initial_steps(500);
        assert_eq!(m.steps(), 500);
        assert_eq!(m.angle_degrees_f64(), 900.0);
        assert!(!m.is_commanded());
    }

    #[test]
    fn hard_travel_limits_clamp_both_directions() {
        let m = StepDirMotor::new("m", 2, 3, None)
            .with_initial_steps(1)
            .with_step_limits(Some(0), Some(2));
        // DIR low: one step reaches the minimum, further pulses cannot pass it.
        for cycle in [1, 3] {
            m.on_gpio_edge(2, true, cycle);
            m.on_gpio_edge(2, false, cycle + 1);
        }
        assert_eq!(m.steps(), 0);
        assert!(m.at_min_limit());
        assert!(!m.at_max_limit());
        // DIR high: move to the maximum, then prove another pulse is clamped.
        m.on_gpio_edge(3, true, 5);
        for cycle in [6, 8, 10] {
            m.on_gpio_edge(2, true, cycle);
            m.on_gpio_edge(2, false, cycle + 1);
        }
        assert_eq!(m.steps(), 2);
        assert!(!m.at_min_limit());
        assert!(m.at_max_limit());
    }

    #[derive(Default)]
    struct TestPins {
        outputs: std::collections::HashMap<(u64, u8), bool>,
        inputs: std::collections::HashMap<(u64, u8), bool>,
    }
    impl crate::bus::DevicePins for TestPins {
        fn output_bit(&self, addr: u64, bit: u8) -> Option<bool> {
            self.outputs.get(&(addr, bit)).copied()
        }
        fn known_pad_bit(&self, addr: u64, bit: u8) -> Option<bool> {
            self.outputs.get(&(addr, bit)).copied()
        }
        fn drive_idr_bit(&mut self, addr: u64, bit: u8, high: bool) {
            self.inputs.insert((addr, bit), high);
        }
        fn drive_input_bit(&mut self, addr: u64, bit: u8, high: bool) -> bool {
            self.inputs.insert((addr, bit), high);
            true
        }
    }

    #[test]
    fn pad_write_that_reaches_a_stop_asserts_the_limit_in_the_same_service() {
        let motor = Arc::new(
            StepDirMotor::new("axis", 0, 1, None)
                .with_initial_steps(1)
                .with_step_limits(Some(0), Some(10)),
        );
        let mut watch = StepDirPadWatch {
            id: "axis-watch".into(),
            motor: motor.clone(),
            pads: vec![(0, 0x10, 0, None), (1, 0x10, 1, None)],
            addrs: vec![0x10],
            min_limit: Some(LimitDrive {
                addr: 0x20,
                bit: 3,
                active_high: false,
                last: None,
            }),
            max_limit: None,
        };
        let mut pins = TestPins::default();
        pins.outputs.insert((0x10, 0), false);
        pins.outputs.insert((0x10, 1), false);
        crate::bus::BusResidentDevice::service(&mut watch, &mut pins, 0);
        assert_eq!(pins.inputs.get(&(0x20, 3)), Some(&true));

        // The next firmware STEP write moves to zero and the same write-hook
        // service asserts the active-low limit before the CPU can poll again.
        pins.outputs.insert((0x10, 0), true);
        crate::bus::BusResidentDevice::service(&mut watch, &mut pins, 1);
        assert_eq!(motor.steps(), 0);
        assert_eq!(pins.inputs.get(&(0x20, 3)), Some(&false));
    }

    #[test]
    fn steps_increment_on_rising_edge_when_dir_high() {
        let m = StepDirMotor::new("m", 2, 3, Some(4));
        // EN low (active)
        m.on_gpio_edge(4, false, 0);
        m.on_gpio_edge(3, true, 1); // DIR high
        m.on_gpio_edge(2, true, 2);
        m.on_gpio_edge(2, false, 3);
        m.on_gpio_edge(2, true, 4);
        assert_eq!(m.steps(), 2);
        assert!((m.angle_degrees() - 3.6).abs() < 0.01);
    }

    #[test]
    fn disabled_when_en_high_active_low() {
        let m = StepDirMotor::new("m", 2, 3, Some(4));
        m.on_gpio_edge(4, true, 0); // EN high = disabled
        m.on_gpio_edge(2, true, 1);
        assert_eq!(m.steps(), 0);
        assert!(!m.is_commanded());
    }

    #[test]
    fn nema17_alias_resolves_to_a4988_kit() {
        assert_eq!(
            crate::peripherals::kit::registry::canonical_device_type("nema17"),
            "a4988"
        );
        assert!(crate::peripherals::kit::registry::lookup("nema17").is_some());
    }

    #[test]
    fn two_hundred_full_steps_is_one_revolution() {
        let m = StepDirMotor::new("shaft", 2, 3, Some(4)).with_device_type("nema17");
        assert_eq!(m.device_type(), "nema17");
        m.on_gpio_edge(4, false, 0); // EN low
        m.on_gpio_edge(3, true, 1); // DIR high
        for i in 0..200 {
            let t = 2 + i as u64 * 2;
            m.on_gpio_edge(2, true, t);
            m.on_gpio_edge(2, false, t + 1);
        }
        assert_eq!(m.steps(), 200);
        assert!((m.angle_degrees() - 360.0).abs() < 0.01);
        assert!((m.angle_degrees_f64() - 360.0).abs() < 1e-9);
    }

    #[test]
    fn commanded_clears_when_en_disabled() {
        let m = StepDirMotor::new("m", 2, 3, Some(4));
        m.on_gpio_edge(4, false, 0); // EN low
        m.on_gpio_edge(3, true, 1);
        m.on_gpio_edge(2, true, 2);
        assert!(m.is_commanded());
        assert!(m.is_enabled());
        m.on_gpio_edge(4, true, 3); // EN high = disabled
        assert!(!m.is_enabled());
        assert!(!m.is_commanded());
        assert_eq!(m.steps(), 1); // position holds
    }
}
