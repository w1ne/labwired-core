use super::*;
use crate::physics::motor::{
    BldcMotor, BldcMotorParams, BrushedDcMotor, BrushedMotorParams, GatePair, HBridgeCommand,
    HBridgeState, InverterCommand, Phase, QuadratureEncoder, ShaftParams,
};
use labwired_config::{BldcMotorConfig, BrushedMotorConfig, MotorModelConfig, SystemManifest};

pub(super) const MOTOR_STALL_INPUT: crate::sim_input::InputChannel =
    crate::sim_input::InputChannel {
        key: "stall",
        label: "Mechanical stall",
        unit: "boolean",
        min: 0.0,
        max: 1.0,
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

#[derive(Debug, Clone, Copy)]
pub(super) struct PwmPhaseCursor {
    revision: u64,
    freeze_revision: u64,
    counter_ticks: u32,
    prescaler_phase: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MotorSnapshot {
    pub id: String,
    pub kind: &'static str,
    pub position_rad: f64,
    pub speed_rpm: f64,
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
        pwm: ResolvedPin,
        direction: ResolvedPin,
        brake: ResolvedPin,
        enable: ResolvedPin,
        feedback: [ResolvedPin; 2],
        index: Option<ResolvedPin>,
        fault: Option<ResolvedPin>,
        simulation_clock_hz: u64,
        control_state: String,
    },
    Bldc {
        id: String,
        plant: Box<BldcMotor>,
        encoder: QuadratureEncoder,
        timer: usize,
        enable: ResolvedPin,
        hall: [ResolvedPin; 3],
        feedback: [ResolvedPin; 2],
        index: Option<ResolvedPin>,
        motor_fault: Option<ResolvedPin>,
        inverter_fault: Option<ResolvedPin>,
        overcurrent_fault: Option<ResolvedPin>,
        undervoltage_fault: Option<ResolvedPin>,
        simulation_clock_hz: u64,
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

    pub(super) fn install_motor_models(&mut self, manifest: &SystemManifest) -> anyhow::Result<()> {
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
        Ok(MotorRuntime::Dc {
            pwm: self.resolve_motor_pin(&c.id, "pwm", &c.pwm_pin)?,
            direction: self.resolve_motor_pin(&c.id, "direction", &c.direction_pin)?,
            brake: self.resolve_motor_pin(&c.id, "brake", &c.brake_pin)?,
            enable: self.resolve_motor_pin(&c.id, "enable", &c.enable_pin)?,
            feedback: [
                self.resolve_motor_input(&c.id, "encoder A", &c.encoder_a_pin)?,
                self.resolve_motor_input(&c.id, "encoder B", &c.encoder_b_pin)?,
            ],
            index: c
                .encoder_index_pin
                .as_deref()
                .map(|p| self.resolve_motor_input(&c.id, "encoder index", p))
                .transpose()?,
            fault: c
                .fault_pin
                .as_deref()
                .map(|p| self.resolve_motor_input(&c.id, "fault", p))
                .transpose()?,
            simulation_clock_hz: c.simulation_clock_hz,
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
        let is_timer = self.peripherals[timer]
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::timer::Timer>())
            .is_some();
        if !is_timer {
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
            enable: self.resolve_motor_pin(&c.id, "enable", &c.enable_pin)?,
            hall: [
                self.resolve_motor_input(&c.id, "Hall A", &c.hall_a_pin)?,
                self.resolve_motor_input(&c.id, "Hall B", &c.hall_b_pin)?,
                self.resolve_motor_input(&c.id, "Hall C", &c.hall_c_pin)?,
            ],
            feedback: [
                self.resolve_motor_input(&c.id, "encoder A", &c.encoder_a_pin)?,
                self.resolve_motor_input(&c.id, "encoder B", &c.encoder_b_pin)?,
            ],
            index: c
                .encoder_index_pin
                .as_deref()
                .map(|p| self.resolve_motor_input(&c.id, "encoder index", p))
                .transpose()?,
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

    fn drive_input(&mut self, pin: ResolvedPin, level: bool) {
        let _ = self.peripherals[pin.peripheral]
            .dev
            .set_gpio_input(pin.bit, level);
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
                    direction,
                    brake,
                    enable,
                    feedback,
                    index,
                    fault,
                    simulation_clock_hz,
                    control_state,
                    ..
                } => {
                    let dt_s = elapsed as f64 / *simulation_clock_hz as f64;
                    let enabled = self.pin_output(*enable);
                    let braking = self.pin_output(*brake);
                    let duty = f64::from(self.pin_output(*pwm));
                    let state = if !enabled {
                        HBridgeState::Coast
                    } else if braking {
                        HBridgeState::Brake
                    } else if self.pin_output(*direction) {
                        HBridgeState::Forward
                    } else {
                        HBridgeState::Reverse
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
                    let pins = encoder.sample(plant.snapshot().position_rad).ok();
                    if let Some(pins) = pins {
                        self.drive_input(feedback[0], pins.a);
                        self.drive_input(feedback[1], pins.b);
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
                    control_state,
                    injected_inverter_fault,
                    computed_inverter_fault,
                    pwm_phase_cursor,
                    ..
                } => {
                    let dt_s = elapsed as f64 / *simulation_clock_hz as f64;
                    let mut timer_output = self.peripherals[*timer]
                        .dev
                        .as_any()
                        .and_then(|a| a.downcast_ref::<crate::peripherals::timer::Timer>())
                        .map(crate::peripherals::timer::Timer::output_snapshot);
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
                    let external_enabled = self.pin_output(*enable);
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
                    for (bit, pin) in hall.iter().enumerate() {
                        self.drive_input(*pin, snapshot.hall_state & (1 << bit) != 0);
                    }
                    if let Ok(pins) = encoder.sample(snapshot.position_rad) {
                        self.drive_input(feedback[0], pins.a);
                        self.drive_input(feedback[1], pins.b);
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
                    control_state,
                    ..
                } => {
                    let s = plant.snapshot();
                    MotorSnapshot {
                        id: id.clone(),
                        kind: "dc",
                        position_rad: s.position_rad,
                        speed_rpm: s.speed_rpm,
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
mod tests {
    use super::*;
    use crate::peripherals::timer::{
        TimerChannelOutputMode, TimerChannelOutputSnapshot, TimerOutputSnapshot,
    };
    use crate::physics::motor::{BldcMotorParams, ShaftParams};

    fn channel(duty: f64) -> TimerChannelOutputSnapshot {
        TimerChannelOutputSnapshot {
            enabled: true,
            complementary_enabled: true,
            active_low: false,
            complementary_active_low: false,
            duty_fraction: duty,
            mode: TimerChannelOutputMode::Pwm1,
        }
    }

    fn pwm(duty: f64, dead_time_ticks: u16) -> TimerOutputSnapshot {
        TimerOutputSnapshot {
            channels: [channel(duty), channel(0.5), channel(0.5), channel(0.0)],
            dead_time_ticks,
            main_output_enabled: true,
            counter_enabled: true,
            period_ticks: 1000,
            counter_ticks: 0,
            prescaler_divisor: 1,
            prescaler_phase: 0,
            phase_revision: 0,
            counter_frozen: false,
            freeze_revision: 0,
            clock_authoritative: false,
        }
    }

    fn motor_response(duty: f64, dead_time_ticks: u16) -> f64 {
        let mut phase_b = channel(0.0);
        phase_b.enabled = false;
        let mut phase_c = channel(0.0);
        phase_c.enabled = false;
        phase_c.complementary_enabled = false;
        let snapshot = TimerOutputSnapshot {
            channels: [channel(duty), phase_b, phase_c, channel(0.0)],
            dead_time_ticks,
            main_output_enabled: true,
            counter_enabled: true,
            period_ticks: 1000,
            counter_ticks: 0,
            prescaler_divisor: 1,
            prescaler_phase: 0,
            phase_revision: 0,
            counter_frozen: false,
            freeze_revision: 0,
            clock_authoritative: false,
        };
        let mut motor = BldcMotor::new(BldcMotorParams {
            resistance_ohm: 1.0,
            inductance_h: 0.001,
            torque_constant_nm_per_a: 0.1,
            back_emf_constant_v_per_rad_s: 0.1,
            supply_voltage_v: 24.0,
            pole_pairs: 2,
            current_limit_a: None,
            overcurrent_trip_steps: 3,
            shaft: ShaftParams {
                inertia_kg_m2: 0.01,
                viscous_friction_nm_per_rad_s: 0.0,
                load_torque_nm: 0.0,
            },
        })
        .unwrap();
        for (command, fraction) in pwm_edge_schedule(snapshot) {
            motor.step(command, 1e-6 * fraction).unwrap();
        }
        motor.snapshot().phase_currents_a[0].abs()
    }

    fn phase_a_high_fraction(snapshot: TimerOutputSnapshot) -> f64 {
        pwm_edge_schedule(snapshot)
            .into_iter()
            .filter(|(command, _)| command.phase_a.high)
            .map(|(_, fraction)| fraction)
            .sum()
    }

    fn advance_phase(mut snapshot: TimerOutputSnapshot, cycles: u64) -> TimerOutputSnapshot {
        let divisor = snapshot.prescaler_divisor;
        let timer_cycles = u64::from(snapshot.prescaler_phase) + cycles;
        let increments = timer_cycles / divisor;
        snapshot.prescaler_phase = (timer_cycles % divisor) as u32;
        snapshot.counter_ticks =
            ((u64::from(snapshot.counter_ticks) + increments) % snapshot.period_ticks) as u32;
        snapshot
    }

    fn schedule_signature(
        snapshot: TimerOutputSnapshot,
        partitions: &[u64],
    ) -> Vec<(InverterCommand, f64)> {
        let mut snapshot = snapshot;
        let mut result: Vec<(InverterCommand, f64)> = Vec::new();
        for &cycles in partitions {
            for (command, duration) in pwm_interval_schedule(snapshot, cycles as f64) {
                if let Some((previous, previous_duration)) = result.last_mut() {
                    if *previous == command {
                        *previous_duration += duration;
                        continue;
                    }
                }
                result.push((command, duration));
            }
            snapshot = advance_phase(snapshot, cycles);
        }
        result
    }

    fn partitioned_motor_snapshot(
        snapshot: TimerOutputSnapshot,
        partitions: &[u64],
    ) -> crate::physics::motor::BldcMotorSnapshot {
        let mut motor = BldcMotor::new(BldcMotorParams {
            resistance_ohm: 1.0,
            inductance_h: 0.001,
            torque_constant_nm_per_a: 0.1,
            back_emf_constant_v_per_rad_s: 0.1,
            supply_voltage_v: 24.0,
            pole_pairs: 2,
            current_limit_a: None,
            overcurrent_trip_steps: 3,
            shaft: ShaftParams {
                inertia_kg_m2: 0.01,
                viscous_friction_nm_per_rad_s: 0.0,
                load_torque_nm: 0.0,
            },
        })
        .unwrap();
        let mut snapshot = snapshot;
        for &cycles in partitions {
            for (command, duration) in pwm_interval_schedule(snapshot, cycles as f64) {
                motor.step(command, duration / 80_000_000.0).unwrap();
            }
            snapshot = advance_phase(snapshot, cycles);
        }
        motor.snapshot()
    }

    fn assert_motor_snapshots_close(
        left: crate::physics::motor::BldcMotorSnapshot,
        right: crate::physics::motor::BldcMotorSnapshot,
    ) {
        // Partition boundaries can split one ODE step while preserving the
        // exact command sequence. Keep the tolerance near floating roundoff;
        // this is intentionally local instead of weakening snapshot equality.
        const TOLERANCE: f64 = 1e-7;
        for (left, right) in left
            .phase_currents_a
            .into_iter()
            .zip(right.phase_currents_a)
            .chain([
                (left.position_rad, right.position_rad),
                (left.speed_rpm, right.speed_rpm),
                (
                    left.electromagnetic_torque_nm,
                    right.electromagnetic_torque_nm,
                ),
            ])
        {
            assert!((left - right).abs() <= TOLERANCE, "{left} != {right}");
        }
    }

    #[test]
    fn pwm_interval_schedule_is_batching_invariant_across_periods_and_partials() {
        let mut snapshot = pwm(0.25, 0);
        snapshot.period_ticks = 10;
        snapshot.prescaler_divisor = 4;
        assert_eq!(
            schedule_signature(snapshot, &[97]),
            schedule_signature(snapshot, &[13, 29, 55]),
            "multi-period integration must retain the actual PWM period"
        );
        assert_eq!(
            schedule_signature(snapshot, &[31]),
            schedule_signature(snapshot, &[7, 11, 13]),
            "partial-period integration must retain the same command ordering"
        );
        assert_motor_snapshots_close(
            partitioned_motor_snapshot(snapshot, &[97]),
            partitioned_motor_snapshot(snapshot, &[13, 29, 55]),
        );
        assert_motor_snapshots_close(
            partitioned_motor_snapshot(snapshot, &[31]),
            partitioned_motor_snapshot(snapshot, &[7, 11, 13]),
        );
    }

    #[test]
    fn pwm_interval_schedule_starts_at_nonzero_counter_and_prescaler_phase() {
        let mut snapshot = pwm(0.25, 0);
        snapshot.period_ticks = 10;
        snapshot.prescaler_divisor = 4;
        snapshot.counter_ticks = 1;
        snapshot.prescaler_phase = 3;
        let one_shot = schedule_signature(snapshot, &[35]);
        let partitioned = schedule_signature(snapshot, &[1, 8, 17, 9]);
        assert_eq!(one_shot, partitioned);
        assert!(
            one_shot.first().unwrap().0.phase_a.high,
            "the nonzero start phase is before CCR and starts with phase A high"
        );
        assert!(
            one_shot.iter().any(|(command, _)| !command.phase_a.high)
                && one_shot.last().unwrap().0.phase_a.high,
            "the interval must cross CCR and then the timer wrap"
        );
    }

    #[test]
    fn pwm_streaming_large_minimum_period_uses_constant_memory_and_preserves_time() {
        let mut snapshot = pwm(0.0, 0);
        snapshot.period_ticks = 1;
        snapshot.channels = [channel(0.0); 4];
        let mut segments = 0usize;
        let mut elapsed = 0.0;
        for_each_pwm_segment(snapshot, 1_000_000.0, |_, duration| {
            segments += 1;
            elapsed += duration;
        });
        assert_eq!(segments, 1_000_000);
        assert_eq!(elapsed, 1_000_000.0);
        assert_eq!(normalized_pwm_edges(snapshot).capacity(), 14);
    }

    #[test]
    fn pwm_edge_schedule_preserves_fractional_duty_proportionally() {
        assert!((phase_a_high_fraction(pwm(0.25, 0)) - 0.25).abs() < 1e-12);
        assert!((phase_a_high_fraction(pwm(0.75, 0)) - 0.75).abs() < 1e-12);
        let low = motor_response(0.25, 0);
        let high = motor_response(0.75, 0);
        assert!(high > low * 2.9 && high < low * 3.1);
    }

    #[test]
    fn pwm_edge_schedule_dead_time_reduces_effective_conduction_deterministically() {
        let without_dead_time = phase_a_high_fraction(pwm(0.75, 0));
        let first = phase_a_high_fraction(pwm(0.75, 100));
        let second = phase_a_high_fraction(pwm(0.75, 100));
        assert!(first < without_dead_time);
        assert_eq!(first, second);
        assert!((first - 0.65).abs() < 1e-12);
        assert!(motor_response(0.75, 100) < motor_response(0.75, 0));
        let channel = channel(0.75);
        assert!(!sampled_gate_pair(channel, 0.01, 0.1).high);
        assert!(!sampled_gate_pair(channel, 0.99, 0.1).low);
    }

    #[test]
    fn sampled_gate_pair_honors_complementary_polarity() {
        let mut output = channel(0.25);
        output.complementary_active_low = true;
        let before_edge = sampled_gate_pair(output, 0.1, 0.0);
        let after_edge = sampled_gate_pair(output, 0.9, 0.0);
        assert!(before_edge.high);
        assert!(before_edge.low, "active-low complement is inverted");
        assert!(!after_edge.high);
        assert!(!after_edge.low);
    }
}
