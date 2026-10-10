use super::*;
use crate::physics::motor::{
    BldcMotor, BldcMotorParams, BrushedDcMotor, BrushedMotorParams, GatePair, HBridgeCommand,
    HBridgeState, InverterCommand, Phase, QuadratureEncoder, ShaftParams,
};
use labwired_config::{
    BldcMotorConfig, BothInputsLow, BrushedMotorConfig, MotorModelConfig, SystemManifest,
};

pub(super) const MOTOR_STALL_INPUT: crate::sim_input::InputChannel =
    crate::sim_input::InputChannel {
        key: std::borrow::Cow::Borrowed("stall"),
        label: std::borrow::Cow::Borrowed("Mechanical stall"),
        unit: std::borrow::Cow::Borrowed("boolean"),
        min: 0.0,
        max: 1.0,
        default: None,
    };

/// Maximum production gap between motor services. This bounds exact PWM edge
/// streaming work while leaving direct diagnostic calls exact for any delta.
pub(super) const MOTOR_SERVICE_QUANTUM_CYCLES: u64 = 4096;

/// Motor physics timebase until chip descriptors expose one authoritative CPU
/// frequency. Simulator cycle deltas are deterministic; this conversion never
/// observes host time. Keep this named and isolated so a future descriptor
/// clock can replace it at construction.
#[derive(Debug, Clone, Copy)]
pub(super) struct ResolvedPin {
    peripheral: usize,
    bit: u8,
}

/// Where a motor control input (PWM / direction / brake / enable) is driven from.
///
/// Rails are classified from board rail *vocabulary* (supply vs ground), not by
/// treating the literal string `"VCC"` as a special GPIO name. An MCU pad stays
/// a [`MotorControlSource::Pad`]; an unsupported or floating label is rejected
/// with an explicit diagnostic at construction.
#[derive(Debug, Clone, Copy)]
pub(super) enum MotorControlSource {
    Pad(ResolvedPin),
    Constant(bool),
}

/// How a brushed DC plant learns its direction.
#[derive(Debug, Clone, Copy)]
pub(super) enum DcSteering {
    /// One direction input: high = forward, low = reverse.
    Direction(MotorControlSource),
    /// Terminal drive through two bridge inputs (H-bridge IN1/IN2): `10`
    /// forward, `01` reverse, `11` brake, `00` coast. `pwm_input` names the
    /// input (0 = IN1, 1 = IN2) that IS the PWM pad, for drivers that take PWM
    /// on an IN pin instead of a separate enable.
    Terminals {
        in1: MotorControlSource,
        in2: MotorControlSource,
        pwm_input: Option<u8>,
        /// `00` brakes (L298N) instead of coasting (TB6612FNG, DRV8833).
        both_low_brakes: bool,
    },
}

/// Signed speed extremes seen at any service boundary since construction.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct SpeedExtremes {
    max_rpm: f64,
    min_rpm: f64,
}

impl SpeedExtremes {
    #[inline]
    fn observe(&mut self, speed_rpm: f64) {
        if speed_rpm > self.max_rpm {
            self.max_rpm = speed_rpm;
        }
        if speed_rpm < self.min_rpm {
            self.min_rpm = speed_rpm;
        }
    }

    fn peak_abs(&self) -> f64 {
        self.max_rpm.max(-self.min_rpm)
    }
}

/// Bridge state and effective duty for one service step of a terminal-driven
/// DC plant. `pwm_duty` is the fraction of time the PWM pad is high.
///
/// With a separate PWM/enable pad the IN pins choose the state and the duty
/// scales the drive. When the PWM pad is one of the IN pins, both phases of
/// the PWM period are evaluated: fast decay (other input low) drives at the
/// duty, slow decay (other input high) drives the opposite way for the low
/// fraction of the period, which is the average terminal voltage either way.
fn terminal_drive(
    enabled: bool,
    braking: bool,
    in1: bool,
    in2: bool,
    pwm_input: Option<u8>,
    pwm_duty: f64,
    both_low_brakes: bool,
) -> (HBridgeState, f64) {
    // `11` brakes on every bridge; what `00` does is the driver's own property
    // (L298N: fast stop = brake; TB6612FNG / DRV8833: outputs off = coast).
    let state = |in1: bool, in2: bool| {
        if both_low_brakes && enabled && !braking && !in1 && !in2 {
            HBridgeState::Brake
        } else {
            HBridgeState::from_pins(enabled, in1, in2, braking)
        }
    };
    let Some(input) = pwm_input else {
        return (state(in1, in2), pwm_duty);
    };
    let phase = |pwm_high: bool| {
        if input == 0 {
            state(pwm_high, in2)
        } else {
            state(in1, pwm_high)
        }
    };
    let driven =
        |state: HBridgeState| matches!(state, HBridgeState::Forward | HBridgeState::Reverse);
    let (on, off) = (phase(true), phase(false));
    if pwm_duty <= 0.0 {
        (off, 1.0)
    } else if pwm_duty >= 1.0 {
        (on, 1.0)
    } else if driven(on) {
        (on, pwm_duty)
    } else if driven(off) {
        (off, 1.0 - pwm_duty)
    } else {
        (on, 1.0)
    }
}

/// Optional encoder observer pads. Absent wires mean the plant still evolves;
/// nothing is driven onto GPIO.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct MotorFeedbackBindings {
    encoder_a: Option<ResolvedPin>,
    encoder_b: Option<ResolvedPin>,
}

/// Supply / ground rail vocabulary shared with board-config power-rails
/// (union of the names LabWired already treats as rails). Classification is by
/// normalized identity, not "pin named VCC".
fn classify_logic_rail(label: &str) -> Option<bool> {
    let n = label.trim().to_ascii_uppercase();
    // Strip a trailing `.N` multi-instance suffix (GND.2 → GND).
    let n = n.split('.').next().unwrap_or(&n);
    const SUPPLY: &[&str] = &[
        "3V3", "3.3V", "5V", "VCC", "VDD", "VDD33", "VCC33", "VBUS", "VUSB", "VIN", "VMCU", "P3V",
    ];
    const GROUND: &[&str] = &["GND", "VSS", "AGND", "DGND", "GNDA", "0", "GROUND"];
    if SUPPLY.contains(&n) {
        Some(true)
    } else if GROUND.contains(&n) {
        Some(false)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PwmPhaseCursor {
    revision: u64,
    freeze_revision: u64,
    counter_ticks: u32,
    prescaler_phase: u32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MotorSnapshot {
    pub id: String,
    pub kind: &'static str,
    pub position_rad: f64,
    pub speed_rpm: f64,
    /// Largest `|speed_rpm|` observed at any service boundary so far. A
    /// final-state-only snapshot cannot prove a transient band was reached;
    /// this is the post-hoc equivalent of the CLI's latched in-run assertion.
    pub speed_rpm_peak_abs: f64,
    /// Most positive (forward) `speed_rpm` seen so far; 0 if never forward.
    pub speed_rpm_max: f64,
    /// Most negative (reverse) `speed_rpm` seen so far; 0 if never reverse.
    pub speed_rpm_min: f64,
    pub torque_nm: f64,
    pub current_a: Option<f64>,
    pub phase_currents_a: Option<[f64; 3]>,
    pub bus_voltage_v: f64,
    pub commutation_sector: Option<u8>,
    pub control_state: String,
    pub faults: Vec<String>,
}

pub(super) enum MotorRuntime {
    Dc {
        id: String,
        plant: Box<BrushedDcMotor>,
        encoder: QuadratureEncoder,
        pwm: MotorControlSource,
        steering: DcSteering,
        brake: MotorControlSource,
        enable: MotorControlSource,
        feedback: MotorFeedbackBindings,
        index: Option<ResolvedPin>,
        fault: Option<ResolvedPin>,
        /// Timer peripheral index + 1-based channel that owns the PWM pin. When
        /// present the plant samples the timer output while that channel is a
        /// PWM output, instead of the PWM pin's ODR latch (which never moves in
        /// alternate-function mode).
        timer: Option<(usize, u8)>,
        simulation_clock_hz: u64,
        speed_extremes: SpeedExtremes,
        control_state: String,
    },
    Bldc {
        id: String,
        plant: Box<BldcMotor>,
        encoder: QuadratureEncoder,
        timer: usize,
        enable: MotorControlSource,
        hall: [ResolvedPin; 3],
        feedback: MotorFeedbackBindings,
        index: Option<ResolvedPin>,
        motor_fault: Option<ResolvedPin>,
        inverter_fault: Option<ResolvedPin>,
        overcurrent_fault: Option<ResolvedPin>,
        undervoltage_fault: Option<ResolvedPin>,
        simulation_clock_hz: u64,
        speed_extremes: SpeedExtremes,
        control_state: String,
        injected_inverter_fault: bool,
        computed_inverter_fault: bool,
        pwm_phase_cursor: Option<Box<PwmPhaseCursor>>,
    },
}

impl SystemBus {
    pub(crate) fn next_motor_service_deadline_cycle(&self) -> Option<u64> {
        (!self.motors.is_empty()).then(|| {
            self.motor_cycle_anchor
                .saturating_add(MOTOR_SERVICE_QUANTUM_CYCLES)
        })
    }

    #[cfg(test)]
    pub(crate) fn motor_service_anchor(&self) -> u64 {
        self.motor_cycle_anchor
    }

    pub(crate) fn install_motor_models(&mut self, manifest: &SystemManifest) -> anyhow::Result<()> {
        for config in manifest.resolved_motor_models()? {
            self.motors.push(match config {
                MotorModelConfig::Dc(config) => self.build_dc_motor(*config)?,
                MotorModelConfig::Bldc(config) => self.build_bldc_motor(*config)?,
            });
        }
        self.motor_cycle_anchor = self.current_cycle;
        Ok(())
    }

    fn resolve_motor_pin(
        &self,
        motor: &str,
        role: &str,
        label: &str,
    ) -> anyhow::Result<ResolvedPin> {
        let (addr, bit) = Self::resolve_pin_odr(self, label).ok_or_else(|| {
            anyhow::anyhow!("motor '{motor}': {role} pin '{label}' is not a compatible GPIO pin")
        })?;
        let peripheral = self.find_peripheral_index(addr).ok_or_else(|| {
            anyhow::anyhow!("motor '{motor}': {role} pin '{label}' has no GPIO peripheral")
        })?;
        Ok(ResolvedPin { peripheral, bit })
    }

    fn resolve_motor_input(
        &self,
        motor: &str,
        role: &str,
        label: &str,
    ) -> anyhow::Result<ResolvedPin> {
        let (addr, bit) = Self::resolve_pin_idr(self, label).ok_or_else(|| {
            anyhow::anyhow!("motor '{motor}': {role} pin '{label}' is not a compatible GPIO input")
        })?;
        let peripheral = self.find_peripheral_index(addr).ok_or_else(|| {
            anyhow::anyhow!("motor '{motor}': {role} pin '{label}' has no GPIO peripheral")
        })?;
        Ok(ResolvedPin { peripheral, bit })
    }

    /// The PWM output snapshot of the peripheral at `index`, or `None` when it
    /// is not an STM32-style timer. One downcast site for every motor arm.
    fn timer_output_at(
        &self,
        index: usize,
    ) -> Option<crate::peripherals::timer::TimerOutputSnapshot> {
        self.peripherals[index]
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::timer::Timer>())
            .map(crate::peripherals::timer::Timer::output_snapshot)
    }

    /// Resolve a control input from the compiled net label: powered logic rail →
    /// `Constant(true)`, ground → `Constant(false)`, MCU-driven pad → `Pad`,
    /// anything else → explicit diagnostic (floating / unsupported).
    fn resolve_motor_control(
        &self,
        motor: &str,
        role: &str,
        label: &str,
    ) -> anyhow::Result<MotorControlSource> {
        if let Some(level) = classify_logic_rail(label) {
            return Ok(MotorControlSource::Constant(level));
        }
        match self.resolve_motor_pin(motor, role, label) {
            Ok(pin) => Ok(MotorControlSource::Pad(pin)),
            Err(_) => Err(anyhow::anyhow!(
                "motor '{motor}': {role} '{label}' is not an MCU-driven pad or a                  known powered/ground logic rail (unsupported or floating net)"
            )),
        }
    }

    /// Direction source for a brushed DC plant: one direction pad, or the two
    /// bridge inputs of a terminal-driven (H-bridge) motor. Config validation
    /// already enforced that exactly one form is present.
    fn resolve_dc_steering(&self, c: &BrushedMotorConfig) -> anyhow::Result<DcSteering> {
        match (
            c.direction_pin.as_deref(),
            c.in1_pin.as_deref(),
            c.in2_pin.as_deref(),
        ) {
            (Some(direction), None, None) => Ok(DcSteering::Direction(
                self.resolve_motor_control(&c.id, "direction", direction)?,
            )),
            (None, Some(in1), Some(in2)) => {
                let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
                let pwm_input = if same(&c.pwm_pin, in1) {
                    Some(0)
                } else if same(&c.pwm_pin, in2) {
                    Some(1)
                } else {
                    None
                };
                Ok(DcSteering::Terminals {
                    in1: self.resolve_motor_control(&c.id, "in1", in1)?,
                    in2: self.resolve_motor_control(&c.id, "in2", in2)?,
                    pwm_input,
                    both_low_brakes: c.both_inputs_low == Some(BothInputsLow::Brake),
                })
            }
            _ => anyhow::bail!(
                "motor '{}': set either direction_pin or both in1_pin and in2_pin",
                c.id
            ),
        }
    }

    fn resolve_optional_motor_input(
        &self,
        motor: &str,
        role: &str,
        label: Option<&str>,
    ) -> anyhow::Result<Option<ResolvedPin>> {
        label
            .map(|p| self.resolve_motor_input(motor, role, p))
            .transpose()
    }

    fn build_dc_motor(&self, c: BrushedMotorConfig) -> anyhow::Result<MotorRuntime> {
        let shaft = ShaftParams {
            inertia_kg_m2: c.rotor_inertia_kg_m2,
            viscous_friction_nm_per_rad_s: c.viscous_friction_nm_per_rad_s,
            load_torque_nm: c.load_torque_nm,
        };
        let plant = BrushedDcMotor::new(BrushedMotorParams {
            resistance_ohm: c.resistance_ohm,
            inductance_h: c.inductance_h,
            torque_constant_nm_per_a: c.torque_constant_nm_per_a,
            back_emf_constant_v_per_rad_s: c.back_emf_constant_v_per_rad_s,
            supply_voltage_v: c.supply_voltage_v,
            shaft,
        })?;
        // Hardware PWM: the emitter fills both fields only when the PWM pin has
        // a timer alternate function, so either one being absent keeps the
        // legacy ODR path.
        let timer = match (
            c.timer_name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty()),
            c.timer_channel,
        ) {
            (Some(name), Some(channel)) => {
                // Advanced timers are declared as `tim1_pwm` on several chips
                // while pin maps and emitters name the timer `tim1`.
                let index = self
                    .find_peripheral_index_by_name(name)
                    .or_else(|| self.find_peripheral_index_by_name(&format!("{name}_pwm")))
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "motor '{}': DC motor PWM timer '{name}' is not a configured peripheral (set timer_name in motor config)",
                            c.id
                        )
                    })?;
                if self.timer_output_at(index).is_none() {
                    anyhow::bail!(
                        "motor '{}': peripheral '{name}' is not an STM32 timer",
                        c.id
                    );
                }
                if !(1..=4).contains(&channel) {
                    anyhow::bail!(
                        "motor '{}': timer_channel {channel} is outside the timer's 1..=4 channels",
                        c.id
                    );
                }
                Some((index, channel))
            }
            _ => None,
        };
        Ok(MotorRuntime::Dc {
            pwm: self.resolve_motor_control(&c.id, "pwm", &c.pwm_pin)?,
            steering: self.resolve_dc_steering(&c)?,
            brake: match c.brake_pin.as_deref() {
                Some(p) => self.resolve_motor_control(&c.id, "brake", p)?,
                None => MotorControlSource::Constant(false),
            },
            enable: match c.enable_pin.as_deref() {
                Some(p) => self.resolve_motor_control(&c.id, "enable", p)?,
                None => MotorControlSource::Constant(true),
            },
            feedback: MotorFeedbackBindings {
                encoder_a: self.resolve_optional_motor_input(
                    &c.id,
                    "encoder A",
                    c.encoder_a_pin.as_deref(),
                )?,
                encoder_b: self.resolve_optional_motor_input(
                    &c.id,
                    "encoder B",
                    c.encoder_b_pin.as_deref(),
                )?,
            },
            index: self.resolve_optional_motor_input(
                &c.id,
                "encoder index",
                c.encoder_index_pin.as_deref(),
            )?,
            fault: self.resolve_optional_motor_input(&c.id, "fault", c.fault_pin.as_deref())?,
            timer,
            simulation_clock_hz: c.simulation_clock_hz,
            speed_extremes: SpeedExtremes::default(),
            control_state: "coast".to_owned(),
            encoder: QuadratureEncoder::new(c.encoder_cpr)?,
            id: c.id,
            plant: Box::new(plant),
        })
    }

    fn build_bldc_motor(&self, c: BldcMotorConfig) -> anyhow::Result<MotorRuntime> {
        // Resolve all declared phase pads now, even though TIM1 owns their
        // runtime levels. This rejects nonexistent/incompatible AF bindings
        // before firmware starts.
        for (role, pin) in [
            ("phase A high", &c.phase_a_high_pin),
            ("phase A low", &c.phase_a_low_pin),
            ("phase B high", &c.phase_b_high_pin),
            ("phase B low", &c.phase_b_low_pin),
            ("phase C high", &c.phase_c_high_pin),
            ("phase C low", &c.phase_c_low_pin),
        ] {
            self.resolve_motor_pin(&c.id, role, pin)?;
        }
        let timer_name = if c.timer_name.trim().is_empty() {
            "tim1"
        } else {
            c.timer_name.trim()
        };
        let timer = self
            .find_peripheral_index_by_name(timer_name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "motor '{}': BLDC requires advanced timer '{timer_name}' (set timer_name in motor config)",
                    c.id
                )
            })?;
        if self.timer_output_at(timer).is_none() {
            anyhow::bail!(
                "motor '{}': peripheral '{timer_name}' is not an STM32 timer",
                c.id
            );
        }
        let plant = BldcMotor::new(BldcMotorParams {
            resistance_ohm: c.resistance_ohm,
            inductance_h: c.inductance_h,
            torque_constant_nm_per_a: c.torque_constant_nm_per_a,
            back_emf_constant_v_per_rad_s: c.back_emf_constant_v_per_rad_s,
            supply_voltage_v: c.supply_voltage_v,
            pole_pairs: c.pole_pairs,
            current_limit_a: c.current_limit_a,
            overcurrent_trip_steps: c.overcurrent_trip_steps,
            shaft: ShaftParams {
                inertia_kg_m2: c.rotor_inertia_kg_m2,
                viscous_friction_nm_per_rad_s: c.viscous_friction_nm_per_rad_s,
                load_torque_nm: c.load_torque_nm,
            },
        })?;
        Ok(MotorRuntime::Bldc {
            id: c.id.clone(),
            plant: Box::new(plant),
            encoder: QuadratureEncoder::new(c.encoder_cpr)?,
            timer,
            enable: self.resolve_motor_control(&c.id, "enable", &c.enable_pin)?,
            hall: [
                self.resolve_motor_input(&c.id, "Hall A", &c.hall_a_pin)?,
                self.resolve_motor_input(&c.id, "Hall B", &c.hall_b_pin)?,
                self.resolve_motor_input(&c.id, "Hall C", &c.hall_c_pin)?,
            ],
            feedback: MotorFeedbackBindings {
                encoder_a: self.resolve_optional_motor_input(
                    &c.id,
                    "encoder A",
                    c.encoder_a_pin.as_deref(),
                )?,
                encoder_b: self.resolve_optional_motor_input(
                    &c.id,
                    "encoder B",
                    c.encoder_b_pin.as_deref(),
                )?,
            },
            index: self.resolve_optional_motor_input(
                &c.id,
                "encoder index",
                c.encoder_index_pin.as_deref(),
            )?,
            motor_fault: c
                .motor_fault_pin
                .as_deref()
                .map(|p| self.resolve_motor_input(&c.id, "motor fault", p))
                .transpose()?,
            inverter_fault: c
                .inverter_fault_pin
                .as_deref()
                .map(|p| self.resolve_motor_input(&c.id, "inverter fault", p))
                .transpose()?,
            overcurrent_fault: c
                .overcurrent_fault_pin
                .as_deref()
                .map(|p| self.resolve_motor_input(&c.id, "overcurrent fault", p))
                .transpose()?,
            undervoltage_fault: c
                .undervoltage_fault_pin
                .as_deref()
                .map(|p| self.resolve_motor_input(&c.id, "undervoltage fault", p))
                .transpose()?,
            simulation_clock_hz: c.simulation_clock_hz,
            speed_extremes: SpeedExtremes::default(),
            control_state: "off:timer-stopped".to_owned(),
            injected_inverter_fault: false,
            computed_inverter_fault: false,
            pwm_phase_cursor: None,
        })
    }

    fn pin_output(&self, pin: ResolvedPin) -> bool {
        self.peripherals[pin.peripheral]
            .dev
            .read_gpio_output(pin.bit)
            .unwrap_or(false)
    }

    fn control_level(&self, source: MotorControlSource) -> bool {
        match source {
            MotorControlSource::Pad(pin) => self.pin_output(pin),
            MotorControlSource::Constant(level) => level,
        }
    }

    fn drive_input(&mut self, pin: ResolvedPin, level: bool) {
        let _ = self.set_peripheral_gpio_input(pin.peripheral, pin.bit, level);
    }

    /// Per-tick motor-plant service. Split so the "no motors on this bus" case
    /// — every bus that is not a motor lab — is an inlinable empty-vector check
    /// instead of a call into the body below.
    ///
    /// This is called on EVERY guest instruction. Profiling the ESP32-S3 Doom
    /// twin put ~2-3% of the whole process in `service_motor_models` while it
    /// did nothing at all: the body is far too large to inline, so a bus with no
    /// motors still paid a call, a prologue and two loads per instruction. Read
    /// that as a cadence signal, not a motor bug.
    #[inline]
    pub(crate) fn service_motor_models(&mut self) {
        if self.motors.is_empty() {
            return;
        }
        self.service_motor_models_impl();
    }

    fn service_motor_models_impl(&mut self) {
        let elapsed = self.current_cycle.saturating_sub(self.motor_cycle_anchor);
        if elapsed == 0 {
            return;
        }
        self.motor_cycle_anchor = self.current_cycle;
        let mut motors = std::mem::take(&mut self.motors);
        for motor in &mut motors {
            match motor {
                MotorRuntime::Dc {
                    plant,
                    encoder,
                    pwm,
                    steering,
                    brake,
                    enable,
                    feedback,
                    index,
                    fault,
                    timer,
                    simulation_clock_hz,
                    speed_extremes,
                    control_state,
                    ..
                } => {
                    let dt_s = elapsed as f64 / *simulation_clock_hz as f64;
                    // Consume the timer ONLY while its channel actually owns the
                    // pad as a PWM output. A timer-driven pin sits in alternate-
                    // function mode, so its ODR never moves and the timer's duty
                    // is the truth; firmware that drives the pad as plain GPIO
                    // never configures the channel, so fall back to the latch.
                    let (duty, timer_gate) = match *timer {
                        Some((timer, channel)) => {
                            use crate::peripherals::timer::TimerChannelOutputMode;
                            let output = self.timer_output_at(timer);
                            let channel_index = usize::from(channel).saturating_sub(1);
                            let owns_pad = output.as_ref().is_some_and(|pwm| {
                                pwm.channels.get(channel_index).is_some_and(|ch| {
                                    ch.enabled
                                        && matches!(
                                            ch.mode,
                                            TimerChannelOutputMode::Pwm1
                                                | TimerChannelOutputMode::Pwm2
                                        )
                                })
                            });
                            match output {
                                Some(pwm) if owns_pad => (
                                    pwm.channels[channel_index].duty_fraction,
                                    pwm.main_output_enabled && pwm.counter_enabled,
                                ),
                                _ => (f64::from(self.control_level(*pwm)), true),
                            }
                        }
                        None => (f64::from(self.control_level(*pwm)), true),
                    };
                    let braking = self.control_level(*brake);
                    let (state, duty) = match *steering {
                        DcSteering::Direction(direction) => {
                            let enabled = self.control_level(*enable) && timer_gate;
                            let state = if !enabled {
                                HBridgeState::Coast
                            } else if braking {
                                HBridgeState::Brake
                            } else if self.control_level(direction) {
                                HBridgeState::Forward
                            } else {
                                HBridgeState::Reverse
                            };
                            (state, duty)
                        }
                        DcSteering::Terminals {
                            in1,
                            in2,
                            pwm_input,
                            both_low_brakes,
                        } => {
                            // PWM on an IN pin: a stopped timer leaves that input
                            // low, it does not disable the bridge. A separate
                            // PWM/enable pad that stops gates the bridge off.
                            let (enabled, duty) = if pwm_input.is_some() {
                                (
                                    self.control_level(*enable),
                                    if timer_gate { duty } else { 0.0 },
                                )
                            } else {
                                (self.control_level(*enable) && timer_gate, duty)
                            };
                            terminal_drive(
                                enabled,
                                braking,
                                self.control_level(in1),
                                self.control_level(in2),
                                pwm_input,
                                duty,
                                both_low_brakes,
                            )
                        }
                    };
                    let command = match state {
                        HBridgeState::Forward => HBridgeCommand::forward(duty),
                        HBridgeState::Reverse => HBridgeCommand::reverse(duty),
                        HBridgeState::Brake => Ok(HBridgeCommand::brake()),
                        HBridgeState::Coast => Ok(HBridgeCommand::coast()),
                    };
                    *control_state = format!("{state:?}").to_ascii_lowercase();
                    if let Ok(command) = command {
                        let params = plant.params();
                        for step_s in stable_substeps(
                            dt_s,
                            0.25 * params.inductance_h / params.resistance_ohm,
                        ) {
                            if plant.step(command, step_s).is_err() {
                                break;
                            }
                        }
                    }
                    let snapshot = plant.snapshot();
                    speed_extremes.observe(snapshot.speed_rpm);
                    let pins = encoder.sample(snapshot.position_rad).ok();
                    if let Some(pins) = pins {
                        if let Some(a) = feedback.encoder_a {
                            self.drive_input(a, pins.a);
                        }
                        if let Some(b) = feedback.encoder_b {
                            self.drive_input(b, pins.b);
                        }
                        if let Some(index) = index {
                            self.drive_input(*index, pins.index);
                        }
                    }
                    if let Some(fault) = fault {
                        self.drive_input(*fault, plant.faults().stalled);
                    }
                }
                MotorRuntime::Bldc {
                    plant,
                    encoder,
                    timer,
                    enable,
                    hall,
                    feedback,
                    index,
                    motor_fault,
                    inverter_fault,
                    overcurrent_fault,
                    undervoltage_fault,
                    simulation_clock_hz,
                    speed_extremes,
                    control_state,
                    injected_inverter_fault,
                    computed_inverter_fault,
                    pwm_phase_cursor,
                    ..
                } => {
                    let dt_s = elapsed as f64 / *simulation_clock_hz as f64;
                    let mut timer_output = self.timer_output_at(*timer);
                    if let Some(pwm) = &mut timer_output {
                        let cursor = pwm_phase_cursor.get_or_insert_with(|| {
                            Box::new(PwmPhaseCursor {
                                revision: pwm.phase_revision,
                                freeze_revision: pwm.freeze_revision,
                                counter_ticks: pwm.counter_ticks,
                                prescaler_phase: pwm.prescaler_phase,
                            })
                        });
                        if cursor.revision != pwm.phase_revision
                            || cursor.freeze_revision != pwm.freeze_revision
                        {
                            **cursor = PwmPhaseCursor {
                                revision: pwm.phase_revision,
                                freeze_revision: pwm.freeze_revision,
                                counter_ticks: pwm.counter_ticks,
                                prescaler_phase: pwm.prescaler_phase,
                            };
                            // Lazy clock already brought CNT to "now" for this
                            // service; advancing again would double-count.
                            // Legacy walk still needs the elapsed advance —
                            // motor runs before the timer tick.
                            if pwm.counter_enabled
                                && !pwm.counter_frozen
                                && !pwm.clock_authoritative
                            {
                                advance_pwm_phase_cursor(cursor.as_mut(), *pwm, elapsed);
                            }
                        } else {
                            // Steady state: track phase across the elapsed
                            // window independently of the upcoming timer walk.
                            pwm.counter_ticks = cursor.counter_ticks;
                            pwm.prescaler_phase = cursor.prescaler_phase;
                            if pwm.counter_enabled && !pwm.counter_frozen {
                                advance_pwm_phase_cursor(cursor.as_mut(), *pwm, elapsed);
                            }
                        }
                    } else {
                        *pwm_phase_cursor = None;
                    }
                    let external_enabled = self.control_level(*enable);
                    let valid_pwm = timer_output.is_some_and(|pwm| {
                        pwm.channels[..3].iter().all(|channel| {
                            matches!(
                                channel.mode,
                                crate::peripherals::timer::TimerChannelOutputMode::Pwm1
                                    | crate::peripherals::timer::TimerChannelOutputMode::Pwm2
                            )
                        })
                    });
                    *control_state = match timer_output {
                        Some(_) if !external_enabled => "off:external-enable",
                        Some(pwm) if !pwm.counter_enabled => "off:timer-stopped",
                        Some(pwm) if !pwm.main_output_enabled => "off:moe",
                        Some(_) if !valid_pwm => "off:unsupported-mode",
                        Some(_) => "inverter",
                        None => "off:no-timer",
                    }
                    .to_owned();
                    let params = plant.params();
                    *computed_inverter_fault = false;
                    if let Some(pwm) = timer_output.filter(|pwm| {
                        external_enabled
                            && pwm.counter_enabled
                            && pwm.main_output_enabled
                            && valid_pwm
                            && !*injected_inverter_fault
                    }) {
                        for_each_pwm_segment(pwm, elapsed as f64, |command, duration_cycles| {
                            for step_s in stable_substeps(
                                duration_cycles / *simulation_clock_hz as f64,
                                0.25 * params.inductance_h / params.resistance_ohm,
                            ) {
                                if plant.step(command, step_s).is_err() {
                                    break;
                                }
                                *computed_inverter_fault |=
                                    !plant.snapshot().inverter_faults.is_empty();
                            }
                        });
                    } else {
                        for step_s in stable_substeps(
                            dt_s,
                            0.25 * params.inductance_h / params.resistance_ohm,
                        ) {
                            if plant.step(InverterCommand::off(), step_s).is_err() {
                                break;
                            }
                        }
                    }
                    let snapshot = plant.snapshot();
                    speed_extremes.observe(snapshot.speed_rpm);
                    for (bit, pin) in hall.iter().enumerate() {
                        self.drive_input(*pin, snapshot.hall_state & (1 << bit) != 0);
                    }
                    if let Ok(pins) = encoder.sample(snapshot.position_rad) {
                        if let Some(a) = feedback.encoder_a {
                            self.drive_input(a, pins.a);
                        }
                        if let Some(b) = feedback.encoder_b {
                            self.drive_input(b, pins.b);
                        }
                        if let Some(index) = index {
                            self.drive_input(*index, pins.index);
                        }
                    }
                    if let Some(pin) = motor_fault {
                        self.drive_input(
                            *pin,
                            snapshot.faults.stalled
                                || snapshot.faults.open_phases.iter().any(|is_open| *is_open),
                        );
                    }
                    if let Some(pin) = inverter_fault {
                        self.drive_input(
                            *pin,
                            *injected_inverter_fault || *computed_inverter_fault,
                        );
                    }
                    if let Some(pin) = overcurrent_fault {
                        self.drive_input(*pin, snapshot.faults.overcurrent);
                    }
                    if let Some(pin) = undervoltage_fault {
                        self.drive_input(*pin, snapshot.faults.undervoltage_v.is_some());
                    }
                    if *injected_inverter_fault || *computed_inverter_fault {
                        *control_state = "fault:inverter".to_owned();
                    }
                }
            }
        }
        self.motors = motors;
    }

    pub fn motor_snapshots(&self) -> Vec<MotorSnapshot> {
        self.motors
            .iter()
            .map(|motor| match motor {
                MotorRuntime::Dc {
                    id,
                    plant,
                    speed_extremes,
                    control_state,
                    ..
                } => {
                    let s = plant.snapshot();
                    MotorSnapshot {
                        id: id.clone(),
                        kind: "dc",
                        position_rad: s.position_rad,
                        speed_rpm: s.speed_rpm,
                        speed_rpm_peak_abs: speed_extremes.peak_abs(),
                        speed_rpm_max: speed_extremes.max_rpm,
                        speed_rpm_min: speed_extremes.min_rpm,
                        torque_nm: s.electromagnetic_torque_nm,
                        current_a: Some(s.current_a),
                        phase_currents_a: None,
                        bus_voltage_v: plant.params().supply_voltage_v,
                        commutation_sector: None,
                        control_state: control_state.clone(),
                        faults: s
                            .faults
                            .stalled
                            .then(|| "stalled".to_owned())
                            .into_iter()
                            .collect(),
                    }
                }
                MotorRuntime::Bldc {
                    id,
                    plant,
                    speed_extremes,
                    control_state,
                    injected_inverter_fault,
                    computed_inverter_fault,
                    ..
                } => {
                    let s = plant.snapshot();
                    let mut faults = Vec::new();
                    if s.faults.stalled {
                        faults.push("stalled".to_owned());
                    }
                    if s.faults.overcurrent {
                        faults.push("overcurrent".to_owned());
                    }
                    if s.faults.undervoltage_v.is_some() {
                        faults.push("undervoltage".to_owned());
                    }
                    for (phase, is_open) in [
                        ("open-phase-a", s.faults.open_phases[0]),
                        ("open-phase-b", s.faults.open_phases[1]),
                        ("open-phase-c", s.faults.open_phases[2]),
                    ] {
                        if is_open {
                            faults.push(phase.to_owned());
                        }
                    }
                    if s.faults.hall_line_low == Some(Phase::B) {
                        faults.push("hall-b-low".to_owned());
                    }
                    if s.faults.forced_hall_state == Some(0) {
                        faults.push("invalid-hall".to_owned());
                    }
                    if *injected_inverter_fault || *computed_inverter_fault {
                        faults.push("inverter".to_owned());
                    }
                    MotorSnapshot {
                        id: id.clone(),
                        kind: "bldc",
                        position_rad: s.position_rad,
                        speed_rpm: s.speed_rpm,
                        speed_rpm_peak_abs: speed_extremes.peak_abs(),
                        speed_rpm_max: speed_extremes.max_rpm,
                        speed_rpm_min: speed_extremes.min_rpm,
                        torque_nm: s.electromagnetic_torque_nm,
                        current_a: Some(s.dc_bus_current_a),
                        phase_currents_a: Some(s.phase_currents_a),
                        bus_voltage_v: s.dc_bus_voltage_v,
                        commutation_sector: Some(s.commutation_sector),
                        control_state: control_state.clone(),
                        faults,
                    }
                }
            })
            .collect()
    }

    pub fn set_motor_stalled(&mut self, id: &str, stalled: bool) -> Result<(), String> {
        let motor =
            self.motors
                .iter_mut()
                .find(|motor| match motor {
                    MotorRuntime::Dc { id: motor_id, .. }
                    | MotorRuntime::Bldc { id: motor_id, .. } => motor_id == id,
                })
                .ok_or_else(|| format!("unknown motor '{id}'"))?;
        match motor {
            MotorRuntime::Dc { plant, .. } => {
                plant.set_faults(crate::physics::motor::MotorFaults { stalled });
            }
            MotorRuntime::Bldc { plant, .. } => {
                let mut faults = plant.faults();
                faults.stalled = stalled;
                plant
                    .set_faults(faults)
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    /// Returns the stable core kind name for a configured motor.
    /// Signed speed of one motor without building every snapshot. The CLI's
    /// `motor_speed_reached` check runs at every observation boundary, so it
    /// must not allocate.
    pub fn motor_speed_rpm(&self, id: &str) -> Option<f64> {
        self.motors.iter().find_map(|motor| match motor {
            MotorRuntime::Dc { id: m, plant, .. } if m == id => Some(plant.snapshot().speed_rpm),
            MotorRuntime::Bldc { id: m, plant, .. } if m == id => Some(plant.snapshot().speed_rpm),
            _ => None,
        })
    }

    pub fn motor_kind(&self, id: &str) -> Option<&'static str> {
        self.motors.iter().find_map(|motor| match motor {
            MotorRuntime::Dc { id: motor_id, .. } if motor_id == id => Some("dc"),
            MotorRuntime::Bldc { id: motor_id, .. } if motor_id == id => Some("bldc"),
            _ => None,
        })
    }

    /// Updates a named, explicitly allowlisted motor plant input.
    pub fn set_motor_named_input(
        &mut self,
        id: &str,
        name: &str,
        value: f64,
    ) -> Result<(), String> {
        if !value.is_finite() {
            return Err(format!("motor '{id}': {name} must be finite"));
        }
        let motor =
            self.motors
                .iter_mut()
                .find(|motor| match motor {
                    MotorRuntime::Dc { id: motor_id, .. }
                    | MotorRuntime::Bldc { id: motor_id, .. } => motor_id == id,
                })
                .ok_or_else(|| format!("unknown motor '{id}'"))?;
        match (motor, name) {
            (MotorRuntime::Dc { plant, .. }, "load-torque-nm") => plant.set_load_torque_nm(value),
            (MotorRuntime::Bldc { plant, .. }, "load-torque-nm") => plant.set_load_torque_nm(value),
            (MotorRuntime::Dc { plant, .. }, "supply-voltage-v") => {
                plant.set_supply_voltage_v(value)
            }
            (MotorRuntime::Bldc { plant, .. }, "supply-voltage-v") => {
                plant.set_supply_voltage_v(value)
            }
            (_, _) => return Err(format!("unknown motor input '{name}'")),
        }
        .map_err(|error| error.to_string())
    }

    /// Updates one explicitly supported injected fault.
    pub fn set_motor_named_fault(
        &mut self,
        id: &str,
        fault: &str,
        active: bool,
    ) -> Result<(), String> {
        let motor =
            self.motors
                .iter_mut()
                .find(|motor| match motor {
                    MotorRuntime::Dc { id: motor_id, .. }
                    | MotorRuntime::Bldc { id: motor_id, .. } => motor_id == id,
                })
                .ok_or_else(|| format!("unknown motor '{id}'"))?;
        match motor {
            MotorRuntime::Dc { plant, .. } => {
                if fault != "stall" {
                    return Err(format!("fault '{fault}' requires a BLDC motor"));
                }
                plant.set_faults(crate::physics::motor::MotorFaults { stalled: active });
                Ok(())
            }
            MotorRuntime::Bldc {
                plant,
                injected_inverter_fault,
                ..
            } => {
                if fault == "inverter" {
                    *injected_inverter_fault = active;
                    return Ok(());
                }
                let mut faults = plant.faults();
                match fault {
                    "stall" => faults.stalled = active,
                    "open-phase-a" => {
                        faults.open_phases[0] = active;
                    }
                    "open-phase-b" => {
                        faults.open_phases[1] = active;
                    }
                    "open-phase-c" => {
                        faults.open_phases[2] = active;
                    }
                    "undervoltage" => {
                        faults.undervoltage_v =
                            active.then(|| plant.params().supply_voltage_v * 0.5);
                    }
                    "hall-b-low" => faults.hall_line_low = active.then_some(Phase::B),
                    "invalid-hall" => faults.forced_hall_state = active.then_some(0),
                    "overcurrent" if active => faults.overcurrent = true,
                    "overcurrent" => {
                        return Err("overcurrent is latched and cannot be cleared".to_owned())
                    }
                    _ => return Err(format!("unknown motor fault '{fault}'")),
                }
                plant.set_faults(faults).map_err(|error| error.to_string())
            }
        }
    }

    pub(super) fn matching_motor_stall_inputs(
        &self,
        component: Option<&str>,
        channel: &str,
    ) -> usize {
        if channel != MOTOR_STALL_INPUT.key {
            return 0;
        }
        self.motors
            .iter()
            .filter(|motor| {
                let id = match motor {
                    MotorRuntime::Dc { id, .. } | MotorRuntime::Bldc { id, .. } => id,
                };
                component.is_none_or(|component| component == id)
            })
            .count()
    }

    pub(super) fn set_motor_input(
        &mut self,
        component: Option<&str>,
        channel: &str,
        value: f64,
    ) -> Result<bool, crate::sim_input::SimInputError> {
        use crate::sim_input::SimInputError;
        if self.matching_motor_stall_inputs(component, channel) != 1 {
            return Ok(false);
        }
        if !(MOTOR_STALL_INPUT.min..=MOTOR_STALL_INPUT.max).contains(&value) {
            return Err(SimInputError::OutOfRange {
                key: channel.to_owned(),
                value,
                min: MOTOR_STALL_INPUT.min,
                max: MOTOR_STALL_INPUT.max,
            });
        }
        let id = self
            .motors
            .iter()
            .find_map(|motor| {
                let id = match motor {
                    MotorRuntime::Dc { id, .. } | MotorRuntime::Bldc { id, .. } => id,
                };
                component
                    .is_none_or(|component| component == id)
                    .then(|| id.clone())
            })
            .expect("exactly one matching motor was counted");
        self.set_motor_stalled(&id, value >= 0.5)
            .map_err(|_| SimInputError::NoDevice(format!("{id}/{channel}")))?;
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn motor_pwm_phase(&self, id: &str) -> Option<(u32, u32)> {
        self.motors.iter().find_map(|motor| match motor {
            MotorRuntime::Bldc {
                id: motor_id,
                pwm_phase_cursor: Some(cursor),
                ..
            } if motor_id == id => Some((cursor.counter_ticks, cursor.prescaler_phase)),
            _ => None,
        })
    }
}

#[cfg(test)]
fn pwm_edge_schedule(
    pwm: crate::peripherals::timer::TimerOutputSnapshot,
) -> Vec<(InverterCommand, f64)> {
    let period_cycles = (pwm.period_ticks * pwm.prescaler_divisor) as f64;
    pwm_interval_schedule(pwm, period_cycles)
        .into_iter()
        .map(|(command, cycles)| (command, cycles / period_cycles))
        .collect()
}

#[cfg(test)]
fn pwm_interval_schedule(
    pwm: crate::peripherals::timer::TimerOutputSnapshot,
    elapsed_cycles: f64,
) -> Vec<(InverterCommand, f64)> {
    let mut segments = Vec::new();
    for_each_pwm_segment(pwm, elapsed_cycles, |command, duration| {
        segments.push((command, duration));
    });
    segments
}

/// Streams at most one PWM period's edge table at a time. Memory is bounded by
/// the 14 possible boundaries (start/end plus four per phase), independent of
/// the elapsed period count.
fn for_each_pwm_segment(
    pwm: crate::peripherals::timer::TimerOutputSnapshot,
    elapsed_cycles: f64,
    mut emit: impl FnMut(InverterCommand, f64),
) {
    let dead = (f64::from(pwm.dead_time_ticks) / pwm.period_ticks as f64).clamp(0.0, 1.0);
    let period_cycles = (pwm.period_ticks * pwm.prescaler_divisor) as f64;
    // A PSC rewrite may leave the timer's raw phase above the new divisor;
    // the timer then increments once on the next CPU cycle. Represent that
    // state as the final subcycle rather than inventing extra increments.
    let prescaler_phase = u64::from(pwm.prescaler_phase).min(pwm.prescaler_divisor - 1);
    let start =
        f64::from(pwm.counter_ticks) * pwm.prescaler_divisor as f64 + prescaler_phase as f64;
    let normalized_edges = normalized_pwm_edges(pwm);
    let mut remaining = elapsed_cycles;
    let mut phase_cycles = start.rem_euclid(period_cycles);
    let mut edges = Vec::with_capacity(normalized_edges.len() + 2);
    while remaining > 0.0 {
        let window_end = (phase_cycles + remaining).min(period_cycles);
        edges.clear();
        edges.push(phase_cycles);
        edges.extend(
            normalized_edges
                .iter()
                .map(|edge| edge * period_cycles)
                .filter(|edge| *edge > phase_cycles && *edge < window_end),
        );
        edges.push(window_end);
        for window in edges.windows(2) {
            let duration_cycles = window[1] - window[0];
            if duration_cycles > 0.0 {
                let phase = ((window[0] + window[1]) / 2.0) / period_cycles;
                let gates: [GatePair; 3] = std::array::from_fn(|index| {
                    sampled_gate_pair(pwm.channels[index], phase, dead)
                });
                emit(
                    InverterCommand {
                        enabled: true,
                        phase_a: gates[0],
                        phase_b: gates[1],
                        phase_c: gates[2],
                    },
                    duration_cycles,
                );
            }
        }
        let consumed = window_end - phase_cycles;
        remaining -= consumed;
        phase_cycles = 0.0;
    }
}

fn normalized_pwm_edges(pwm: crate::peripherals::timer::TimerOutputSnapshot) -> Vec<f64> {
    let dead = (f64::from(pwm.dead_time_ticks) / pwm.period_ticks as f64).clamp(0.0, 1.0);
    let mut normalized_edges = Vec::with_capacity(14);
    normalized_edges.extend([0.0, 1.0]);
    for channel in &pwm.channels[..3] {
        let duty = channel.duty_fraction;
        normalized_edges.push((dead / 2.0).clamp(0.0, 1.0));
        normalized_edges.push((duty - dead / 2.0).clamp(0.0, 1.0));
        normalized_edges.push((duty + dead / 2.0).clamp(0.0, 1.0));
        normalized_edges.push((1.0 - dead / 2.0).clamp(0.0, 1.0));
    }
    normalized_edges.sort_by(f64::total_cmp);
    normalized_edges.dedup();
    normalized_edges
}

fn advance_pwm_phase_cursor(
    cursor: &mut PwmPhaseCursor,
    pwm: crate::peripherals::timer::TimerOutputSnapshot,
    elapsed_cycles: u64,
) {
    let divisor = pwm.prescaler_divisor;
    let phase = u64::from(cursor.prescaler_phase).min(divisor - 1);
    let timer_cycles = phase + elapsed_cycles;
    let increments = timer_cycles / divisor;
    cursor.prescaler_phase = (timer_cycles % divisor) as u32;
    cursor.counter_ticks =
        ((u64::from(cursor.counter_ticks) + increments) % pwm.period_ticks) as u32;
}

fn sampled_gate_pair(
    channel: crate::peripherals::timer::TimerChannelOutputSnapshot,
    phase: f64,
    dead: f64,
) -> GatePair {
    use crate::peripherals::timer::TimerChannelOutputMode;
    let low_edge = (channel.duty_fraction - dead / 2.0).clamp(0.0, 1.0);
    let high_edge = (channel.duty_fraction + dead / 2.0).clamp(0.0, 1.0);
    let wrap_start = (dead / 2.0).clamp(0.0, 1.0);
    let wrap_end = (1.0 - dead / 2.0).clamp(0.0, 1.0);
    let (main_raw, complementary_raw) = match channel.mode {
        TimerChannelOutputMode::Pwm1 => (
            phase >= wrap_start && phase < low_edge,
            phase >= high_edge && phase < wrap_end,
        ),
        TimerChannelOutputMode::Pwm2 => (
            phase >= high_edge && phase < wrap_end,
            phase >= wrap_start && phase < low_edge,
        ),
        TimerChannelOutputMode::Unsupported => (false, false),
    };
    GatePair {
        high: channel.enabled && (main_raw ^ channel.active_low),
        low: channel.complementary_enabled
            && (complementary_raw ^ channel.complementary_active_low),
    }
}

fn stable_substeps(total_s: f64, max_step_s: f64) -> impl Iterator<Item = f64> {
    let count = (total_s / max_step_s).ceil().max(1.0) as u64;
    std::iter::repeat_n(total_s / count as f64, count as usize)
}

#[cfg(test)]
#[path = "motors_tests.rs"]
mod tests;
