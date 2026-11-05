// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STEP/DIR interface model for A4988 / DRV8825 / TMC2209 / TMC2226 drivers.
//!
//! Counts rising edges on STEP while EN is active (low or high per config).
//! The angle is ideal commanded position. Datasheet timing violations latch
//! diagnostics while preserving that count, because out-of-spec input timing
//! does not specify a deterministic physical response. Winding current, torque,
//! load, lost steps and driver UART registers are outside this model.

use std::collections::BTreeMap;
use std::sync::Mutex;

const DEFAULT_DRIVER_CLOCK_HZ: u32 = 12_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepDirTimingConstraint {
    StepHigh,
    StepLow,
    DirSetup,
    DirHold,
    StepPeriod,
    MonotonicCycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct StepDirTimingViolation {
    pub constraint: StepDirTimingConstraint,
    pub sim_cycle: u64,
    /// Signed so a regressing timestamp remains visible as a negative interval.
    pub observed_cycles: i128,
    pub minimum_cycles: u64,
}

/// Datasheet minima rounded up to whole ticks of the declared simulation clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct StepDirTimingLimits {
    pub step_high_min_cycles: u64,
    pub step_low_min_cycles: u64,
    pub dir_setup_min_cycles: u64,
    pub dir_hold_min_cycles: u64,
    pub step_period_min_cycles: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepDirTimingStatus {
    Unconfigured,
    NotObserved,
    Pending,
    WithinLimits,
    Violated,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StepDirTimingReport {
    pub profile: &'static str,
    pub simulation_clock_hz: Option<u64>,
    /// TMC internal/external clock. Defaults to the nominal factory trim.
    pub driver_clock_hz: Option<u32>,
    pub limits: Option<StepDirTimingLimits>,
    pub status: StepDirTimingStatus,
    pub checked_rising_edges: u64,
    pub completed_pulses: u64,
    pub pending_high_pulse: bool,
    pub violations: BTreeMap<StepDirTimingConstraint, u64>,
    pub last_violation: Option<StepDirTimingViolation>,
}

/// Carried by both native motor evidence and browser actuator snapshots.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct StepDirModelReport {
    pub motion_model: &'static str,
    pub commanded_steps: i64,
    pub degrees_per_step: f64,
    pub capabilities: &'static [&'static str],
    pub assumptions: &'static [&'static str],
    pub unmodeled: &'static [&'static str],
    pub timing: StepDirTimingReport,
}

#[derive(Debug, Clone, Copy)]
pub struct StepDirConfig {
    /// Degrees of shaft rotation per rising STEP edge, including microstepping.
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
    step_since_cycle: u64,
    dir_since_cycle: u64,
    last_rising_cycle: Option<u64>,
    last_cycle: u64,
    checked_rising_edges: u64,
    completed_pulses: u64,
    timing_violations: BTreeMap<StepDirTimingConstraint, u64>,
    last_violation: Option<StepDirTimingViolation>,
}

impl State {
    fn check_timing(
        &mut self,
        constraint: StepDirTimingConstraint,
        sim_cycle: u64,
        since_cycle: u64,
        minimum_cycles: u64,
    ) {
        let observed_cycles = sim_cycle as i128 - since_cycle as i128;
        if observed_cycles < minimum_cycles as i128 {
            let count = self.timing_violations.entry(constraint).or_default();
            *count = count.saturating_add(1);
            self.last_violation = Some(StepDirTimingViolation {
                constraint,
                sim_cycle,
                observed_cycles,
                minimum_cycles,
            });
        }
    }
}

/// STEP/DIR commanded-motion model.
#[derive(Debug)]
pub struct StepDirMotor {
    step_pin: u8,
    dir_pin: u8,
    en_pin: Option<u8>,
    cfg: StepDirConfig,
    state: Mutex<State>,
    id: String,
    /// Raw YAML `type:` (e.g. `nema17` / `a4988`) for actuator kind export.
    device_type: String,
    simulation_clock_hz: u64,
    driver_clock_hz: u32,
}

impl StepDirMotor {
    pub fn new(id: impl Into<String>, step_pin: u8, dir_pin: u8, en_pin: Option<u8>) -> Self {
        Self {
            step_pin,
            dir_pin,
            en_pin,
            cfg: StepDirConfig::default(),
            state: Mutex::new(State::default()),
            id: id.into(),
            // Kit attach always overrides via with_device_type(ctx.device_type()).
            // Default is the canonical kit id for hand-built test twins.
            device_type: "a4988".into(),
            // Hand-built legacy twins keep counting without an invented clock.
            simulation_clock_hz: 0,
            driver_clock_hz: DEFAULT_DRIVER_CLOCK_HZ,
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

    /// Fixed timebase of `sim_cycle`. Zero leaves timing explicitly unconfigured.
    pub fn with_simulation_clock_hz(mut self, simulation_clock_hz: u64) -> Self {
        self.simulation_clock_hz = simulation_clock_hz;
        self
    }

    /// TMC2209/2226 clock, independent of the MCU simulation timebase.
    pub fn with_driver_clock_hz(mut self, driver_clock_hz: u32) -> anyhow::Result<Self> {
        anyhow::ensure!(
            (4_000_000..=16_000_000).contains(&driver_clock_hz),
            "driver_clock_hz must be within the TMC2209/2226 operating range 4000000..=16000000"
        );
        self.driver_clock_hz = driver_clock_hz;
        Ok(self)
    }

    fn timing_profile(&self) -> &'static str {
        match self.device_type.as_str() {
            "drv8825" => "drv8825",
            "tmc2209" => "tmc2209",
            "tmc2226" => "tmc2226",
            // The historical NEMA17 alias assumes an A4988 interface.
            _ => "a4988",
        }
    }

    fn is_tmc(&self) -> bool {
        matches!(self.timing_profile(), "tmc2209" | "tmc2226")
    }

    fn timing_limits(&self) -> Option<StepDirTimingLimits> {
        if self.simulation_clock_hz == 0 {
            return None;
        }
        let hz = self.simulation_clock_hz as u128;
        let ns_cycles = |ns: u128| (hz * ns).div_ceil(1_000_000_000) as u64;
        let (step, dir, period) = match self.timing_profile() {
            // A4988 datasheet, p6, Figure 1 logic-interface timing.
            // https://www.allegromicro.com/-/media/files/datasheets/a4988-datasheet.pdf
            "a4988" => (ns_cycles(1_000), ns_cycles(200), ns_cycles(2_000)),
            // DRV8825, section 7.6. Its 250 kHz limit is stricter than
            // the sum of the 1.9 us high and low minima.
            // https://www.ti.com/lit/ds/symlink/drv8825.pdf
            "drv8825" => (ns_cycles(1_900), ns_cycles(650), ns_cycles(4_000)),
            _ => {
                // TMC2209 rev1.09 / TMC2226 rev1.10, section 13.1.
                // https://www.analog.com/media/en/technical-documentation/data-sheets/TMC2209_datasheet_rev1.09.pdf
                // https://www.analog.com/media/en/technical-documentation/data-sheets/tmc2226_datasheet_rev1.10.pdf
                // STEP >= max(tFILTSD, tCLK + 20 ns). tFILTSD <= 30 ns,
                // so tCLK + 20 ns dominates throughout the 4..16 MHz range.
                // Keep the ratio exact until rounding to simulation cycles.
                // The table's 100 ns is typical, not a universal minimum.
                let driver_hz = self.driver_clock_hz as u128;
                let step = (hz * (1_000_000_000 + 20 * driver_hz))
                    .div_ceil(driver_hz * 1_000_000_000) as u64;
                let period = (hz * 2).div_ceil(driver_hz) as u64;
                (step, ns_cycles(20), period)
            }
        };
        Some(StepDirTimingLimits {
            step_high_min_cycles: step,
            step_low_min_cycles: step,
            dir_setup_min_cycles: dir,
            dir_hold_min_cycles: dir,
            step_period_min_cycles: period,
        })
    }

    /// Report timing observed through `sim_cycle`, including any unfinished
    /// minimum-high or DIR-hold interval. Diagnostics latch for this model's
    /// lifetime. Every observed STEP pulse is checked, including while disabled.
    pub fn model_report(&self, sim_cycle: u64) -> StepDirModelReport {
        let s = self.state.lock().unwrap();
        let limits = self.timing_limits();
        let now = sim_cycle.max(s.last_cycle);
        let status = match limits {
            None => StepDirTimingStatus::Unconfigured,
            Some(_) if !s.timing_violations.is_empty() => StepDirTimingStatus::Violated,
            Some(_) if s.checked_rising_edges == 0 => StepDirTimingStatus::NotObserved,
            Some(limits)
                if (s.step_level
                    && now.saturating_sub(s.step_since_cycle) < limits.step_high_min_cycles)
                    || s.last_rising_cycle.is_some_and(|rise| {
                        now.saturating_sub(rise) < limits.dir_hold_min_cycles
                    }) =>
            {
                StepDirTimingStatus::Pending
            }
            Some(_) => StepDirTimingStatus::WithinLimits,
        };
        StepDirModelReport {
            motion_model: "ideal_commanded_position",
            commanded_steps: s.steps,
            degrees_per_step: self.cfg.degrees_per_step,
            capabilities: &[
                "step_dir_enable",
                "fixed_microstep_angle",
                "input_timing_checks",
            ],
            assumptions: &[
                "ideal_logic_levels",
                "initial_step_dir_enable_low",
                "rising_edge_step_mode",
                "fixed_simulation_clock",
                "configured_driver_clock",
            ],
            unmodeled: &[
                "winding_current",
                "torque_load_dynamics",
                "lost_steps",
                "driver_uart",
                "supply_and_thermal_faults",
                "enable_sleep_reset_timing",
                "input_filter_response",
                "clock_tolerance",
            ],
            timing: StepDirTimingReport {
                profile: self.timing_profile(),
                simulation_clock_hz: (self.simulation_clock_hz != 0)
                    .then_some(self.simulation_clock_hz),
                driver_clock_hz: self.is_tmc().then_some(self.driver_clock_hz),
                limits,
                status,
                checked_rising_edges: s.checked_rising_edges,
                completed_pulses: s.completed_pulses,
                pending_high_pulse: s.step_level,
                violations: s.timing_violations.clone(),
                last_violation: s.last_violation,
            },
        }
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

    /// Ideal commanded angle in degrees (for prove `position_rad`).
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

    pub fn on_gpio_edge(&self, pin: u8, to: bool, sim_cycle: u64) {
        if pin != self.step_pin && pin != self.dir_pin && self.en_pin != Some(pin) {
            return;
        }
        let mut s = self.state.lock().unwrap();
        let limits = self.timing_limits();
        if limits.is_some() {
            let last_cycle = s.last_cycle;
            s.check_timing(
                StepDirTimingConstraint::MonotonicCycle,
                sim_cycle,
                last_cycle,
                0,
            );
        }
        s.last_cycle = sim_cycle;
        if pin == self.step_pin {
            let was = s.step_level;
            if to == was {
                return;
            }
            if let Some(limits) = limits {
                let step_since = s.step_since_cycle;
                if to {
                    s.check_timing(
                        StepDirTimingConstraint::StepLow,
                        sim_cycle,
                        step_since,
                        limits.step_low_min_cycles,
                    );
                    let dir_since = s.dir_since_cycle;
                    s.check_timing(
                        StepDirTimingConstraint::DirSetup,
                        sim_cycle,
                        dir_since,
                        limits.dir_setup_min_cycles,
                    );
                    if let Some(rise) = s.last_rising_cycle {
                        s.check_timing(
                            StepDirTimingConstraint::StepPeriod,
                            sim_cycle,
                            rise,
                            limits.step_period_min_cycles,
                        );
                    }
                    s.checked_rising_edges += 1;
                } else {
                    s.check_timing(
                        StepDirTimingConstraint::StepHigh,
                        sim_cycle,
                        step_since,
                        limits.step_high_min_cycles,
                    );
                    s.completed_pulses += 1;
                }
            }
            s.step_level = to;
            s.step_since_cycle = sim_cycle;
            if to {
                s.last_rising_cycle = Some(sim_cycle);
                // Rising STEP edge
                let enabled = Self::enabled_level(&self.cfg, s.en_level, self.en_pin.is_some());
                if enabled {
                    if s.dir_level {
                        s.steps += 1;
                    } else {
                        s.steps -= 1;
                    }
                    s.commanded = true;
                }
            }
        } else if pin == self.dir_pin && to != s.dir_level {
            if let (Some(limits), Some(rise)) = (limits, s.last_rising_cycle) {
                s.check_timing(
                    StepDirTimingConstraint::DirHold,
                    sim_cycle,
                    rise,
                    limits.dir_hold_min_cycles,
                );
            }
            s.dir_level = to;
            s.dir_since_cycle = sim_cycle;
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

/// STEP/DIR driver kit (A4988 / DRV8825 / TMC2209 / TMC2226).
pub struct StepDirMotorKit;
pub static STEP_DIR_MOTOR_KIT: StepDirMotorKit = StepDirMotorKit;

static STEP_DIR_METADATA: KitMetadata = KitMetadata {
    inputs: std::borrow::Cow::Borrowed(&[]),
    device_type: std::borrow::Cow::Borrowed("a4988"),
    label: std::borrow::Cow::Borrowed("STEP/DIR stepper driver"),
    summary: std::borrow::Cow::Borrowed(
        "A4988/DRV8825/TMC2209/TMC2226 commanded position with input timing diagnostics.",
    ),
    detail: std::borrow::Cow::Borrowed(
        "Counts rising STEP edges while EN is active (low). DIR selects direction. \
             Type aliases drv8825, tmc2209, tmc2226, and nema17 map here. \
             The nema17 alias assumes an A4988 driver. \
             The TMC2209 model retains its 1/16 microstep default, and TMC2226 uses its unstrapped \
             1/8 setting. Configurable step angle represents the board's selected \
             microstep mode. Position is ideal commanded motion. STEP high/low, \
             STEP period and DIR setup/hold diagnostics use each driver's datasheet \
             limits and the declared simulation clock. Violations latch without \
             discarding commanded edges. Snapshots expose timing status and model \
             capabilities. Winding current, torque/load dynamics, lost steps, \
             driver UART, power faults and thermal behavior are unmodeled.",
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
            name: std::borrow::Cow::Borrowed("degrees_per_step"),
            ty: ConfigType::Float,
            doc: std::borrow::Cow::Borrowed(
                "Mechanical degrees per rising STEP edge, finite and positive.",
            ),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("simulation_clock_hz"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed(
                "Positive fixed frequency of GPIO timestamps. Defaults to the system cpu_hz. Timing is unconfigured when neither declares a clock.",
            ),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("driver_clock_hz"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed(
                "TMC2209/2226 clock, 4000000..=16000000 Hz, default nominal 12000000 Hz. STEP high/low uses tCLK + 20 ns. Clock tolerance is unmodeled.",
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
            .to_owned();
        let dir_label = ctx
            .config_str("dir_pin")
            .or_else(|| ctx.config_str("DIR"))
            .unwrap_or("GPIO17")
            .to_owned();
        let en_label = ctx
            .config_str("en_pin")
            .or_else(|| ctx.config_str("EN"))
            .map(str::to_owned);
        // Native edge-observer ports use their pad number. Other MCU ports
        // bind the named physical pads to distinct local signal numbers.
        let pin_number = |label: &str, local: u8| {
            ctx.parse_gpio_pin(label)
                .or_else(|| ctx.resolve_pin_odr(label).map(|_| local))
                .ok_or_else(|| anyhow::anyhow!("unresolvable STEP/DIR pad '{label}'"))
        };
        let step = pin_number(&step_label, 0)?;
        let dir = pin_number(&dir_label, 1)?;
        let en = en_label
            .as_deref()
            .map(|label| pin_number(label, 2))
            .transpose()?;
        let mut motor =
            StepDirMotor::new(ctx.device_id(), step, dir, en).with_device_type(ctx.device_type());
        let default_angle = match ctx.device_type() {
            "tmc2209" => 1.8 / 16.0,
            "tmc2226" => 1.8 / 8.0,
            _ => 1.8,
        };
        let degrees_per_step = ctx.config_f64("degrees_per_step").unwrap_or(default_angle);
        anyhow::ensure!(
            degrees_per_step.is_finite() && degrees_per_step > 0.0,
            "degrees_per_step must be finite and positive"
        );
        motor = motor.with_config(StepDirConfig {
            degrees_per_step,
            enable_active_low: true,
        });
        let simulation_clock_hz = match ctx.ext.config.get("simulation_clock_hz") {
            Some(value) => value
                .as_u64()
                .filter(|hz| *hz > 0)
                .ok_or_else(|| anyhow::anyhow!("simulation_clock_hz must be a positive integer"))?,
            None => ctx.bus.cpu_hz,
        };
        let driver_clock_hz = match ctx.ext.config.get("driver_clock_hz") {
            Some(value) => value
                .as_u64()
                .and_then(|hz| u32::try_from(hz).ok())
                .ok_or_else(|| anyhow::anyhow!("driver_clock_hz must be a positive integer"))?,
            None => DEFAULT_DRIVER_CLOCK_HZ,
        };
        motor = motor
            .with_simulation_clock_hz(simulation_clock_hz)
            .with_driver_clock_hz(driver_clock_hz)?;
        let mut motor = Arc::new(motor);
        let observer = Arc::new(StepDirCycleObserver {
            motor: motor.clone(),
            clock: ctx.bus.cycle_clock.clone(),
        });
        if !ctx.install_gpio_observer(observer) {
            // A pad number is local to its GPIO port. PC4 and PA4 are distinct
            // wires even though the permissive ESP label parser returns 4
            // for both. The physical binding below maps to unique role ids.
            motor = Arc::new(
                StepDirMotor::new(ctx.device_id(), 0, 1, en.map(|_| 2))
                    .with_device_type(ctx.device_type())
                    .with_config(motor.cfg)
                    .with_simulation_clock_hz(motor.simulation_clock_hz)
                    .with_driver_clock_hz(motor.driver_clock_hz)?,
            );
            let resolve_pad = |label: &str| {
                ctx.resolve_pin_odr(label).ok_or_else(|| {
                    anyhow::anyhow!("STEP/DIR pad '{label}' has no observable output")
                })
            };
            let step_pad = resolve_pad(&step_label)?;
            let dir_pad = resolve_pad(&dir_label)?;
            let en_pad = en_label.as_deref().map(resolve_pad).transpose()?;
            let mut addrs = vec![step_pad.0, dir_pad.0];
            if let Some((addr, _)) = en_pad {
                addrs.push(addr);
            }
            addrs.sort_unstable();
            addrs.dedup();
            ctx.bus.gpio_devices.push(Box::new(StepDirPadWatch {
                motor: motor.clone(),
                step_pad,
                dir_pad,
                en_pad,
                addrs,
            }));
        }
        ctx.bus.observe_device(motor);
        Ok(())
    }
}

/// Native GPIO observers may supply peripheral-tick timestamps. Use the bus's
/// CPU-cycle clock so timing has the same units with either service cadence.
#[derive(Debug)]
struct StepDirCycleObserver {
    motor: Arc<StepDirMotor>,
    clock: crate::CycleClock,
}

impl crate::peripherals::device::GpioObserver for StepDirCycleObserver {
    fn on_pin_change(&self, pin: u8, _from: bool, to: bool, _sim_cycle: u64) {
        self.motor.on_gpio_edge(pin, to, self.clock.now());
    }
}

/// Synchronous output-write binding for ports with no native edge observer.
/// A STEP pulse can rise and fall between peripheral ticks, so every store
/// to any of the three physical GPIO ports samples this device immediately.
#[derive(Debug)]
struct StepDirPadWatch {
    motor: Arc<StepDirMotor>,
    step_pad: (u64, u8),
    dir_pad: (u64, u8),
    en_pad: Option<(u64, u8)>,
    addrs: Vec<u64>,
}

impl crate::bus::BusResidentDevice for StepDirPadWatch {
    fn service(&mut self, _pins: &mut dyn crate::bus::DevicePins, _now: u64) {}

    fn service_edge(&mut self, pins: &mut dyn crate::bus::DevicePins, now: u64) {
        let (Some(step), Some(dir)) = (
            pins.known_pad_bit(self.step_pad.0, self.step_pad.1),
            pins.known_pad_bit(self.dir_pad.0, self.dir_pad.1),
        ) else {
            return;
        };
        if let Some((addr, bit)) = self.en_pad {
            let Some(enabled) = pins.known_pad_bit(addr, bit) else {
                return;
            };
            self.motor
                .on_gpio_edge(self.motor.en_pin.unwrap(), enabled, now);
        }
        // DIR and EN settle before STEP for a single store that changes both.
        self.motor.on_gpio_edge(self.motor.dir_pin, dir, now);
        self.motor.on_gpio_edge(self.motor.step_pin, step, now);
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
        self.motor.id()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl crate::sim_input::SimInput for StepDirPadWatch {
    fn input_channels(&self) -> &[crate::sim_input::InputChannel] {
        &[]
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value).map(|_| ())
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
