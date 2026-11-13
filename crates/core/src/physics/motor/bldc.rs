// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::f64::consts::{PI, TAU};

use super::{ModelError, Shaft, ShaftParams};

const PHASE_OFFSET_RAD: [f64; 3] = [0.0, -2.0 * PI / 3.0, 2.0 * PI / 3.0];
const HALL_SEQUENCE: [u8; 6] = [0b001, 0b101, 0b100, 0b110, 0b010, 0b011];
const TORQUE_POWER_SPEED_EPSILON_RAD_S: f64 = 1e-9;

/// One motor phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    A,
    B,
    C,
}

impl Phase {
    fn index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
            Self::C => 2,
        }
    }

    fn hall_mask(self) -> u8 {
        match self {
            Self::A => 0b100,
            Self::B => 0b010,
            Self::C => 0b001,
        }
    }
}

/// High-side and low-side gate states for one inverter leg.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GatePair {
    pub high: bool,
    pub low: bool,
}

impl GatePair {
    pub const fn off() -> Self {
        Self {
            high: false,
            low: false,
        }
    }

    pub const fn high() -> Self {
        Self {
            high: true,
            low: false,
        }
    }

    pub const fn low() -> Self {
        Self {
            high: false,
            low: true,
        }
    }
}

/// Digital command for a three-phase, two-level inverter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InverterCommand {
    pub enabled: bool,
    pub phase_a: GatePair,
    pub phase_b: GatePair,
    pub phase_c: GatePair,
}

impl InverterCommand {
    /// Disables the inverter so all three terminals float.
    pub const fn off() -> Self {
        Self {
            enabled: false,
            phase_a: GatePair::off(),
            phase_b: GatePair::off(),
            phase_c: GatePair::off(),
        }
    }

    /// Enables all three low-side switches for dynamic braking.
    pub const fn brake() -> Self {
        Self {
            enabled: true,
            phase_a: GatePair::low(),
            phase_b: GatePair::low(),
            phase_c: GatePair::low(),
        }
    }

    /// Selects one of the six canonical forward commutation states.
    ///
    /// The table is aligned with [`HallSensors`]: sectors 0 through 5 drive
    /// `C+/B-`, `A+/B-`, `A+/C-`, `B+/C-`, `B+/A-`, and `C+/A-`.
    pub fn six_step(sector: u8) -> Result<Self, ModelError> {
        Self::six_step_with_direction(sector, false)
    }

    /// Selects the opposite polarity of a canonical six-step state.
    pub fn reverse_six_step(sector: u8) -> Result<Self, ModelError> {
        Self::six_step_with_direction(sector, true)
    }

    fn six_step_with_direction(sector: u8, reverse: bool) -> Result<Self, ModelError> {
        let (high, low) = match sector {
            0 => (Phase::C, Phase::B),
            1 => (Phase::A, Phase::B),
            2 => (Phase::A, Phase::C),
            3 => (Phase::B, Phase::C),
            4 => (Phase::B, Phase::A),
            5 => (Phase::C, Phase::A),
            _ => {
                return Err(ModelError {
                    field: "commutation_sector",
                    message: "must be between 0 and 5 inclusive".to_owned(),
                });
            }
        };
        let (high, low) = if reverse { (low, high) } else { (high, low) };
        let mut gates = [GatePair::off(); 3];
        gates[high.index()] = GatePair::high();
        gates[low.index()] = GatePair::low();
        Ok(Self {
            enabled: true,
            phase_a: gates[0],
            phase_b: gates[1],
            phase_c: gates[2],
        })
    }
}

/// A non-destructive inverter diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InverterFault {
    ShootThrough { phase: Phase },
}

/// Voltage state presented by one resolved inverter leg.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PhaseTerminal {
    Bus(f64),
    Low,
    Floating,
}

/// Pure result of resolving an [`InverterCommand`].
#[derive(Debug, Clone, PartialEq)]
pub struct InverterResolution {
    pub phase_a: PhaseTerminal,
    pub phase_b: PhaseTerminal,
    pub phase_c: PhaseTerminal,
    pub faults: Vec<InverterFault>,
}

impl InverterResolution {
    fn terminals(&self) -> [PhaseTerminal; 3] {
        [self.phase_a, self.phase_b, self.phase_c]
    }
}

/// Stateless three-phase inverter resolver.
#[derive(Debug, Clone, Copy, Default)]
pub struct Inverter;

impl Inverter {
    /// Resolves each leg independently.
    ///
    /// A disabled inverter always floats without reporting inactive gate
    /// combinations. When enabled, a high/low overlap reports shoot-through
    /// and floats only the affected terminal, ensuring destructive physics is
    /// never applied to the motor model.
    pub fn resolve(
        command: InverterCommand,
        bus_voltage_v: f64,
    ) -> Result<InverterResolution, ModelError> {
        validate_non_negative("bus_voltage_v", bus_voltage_v)?;
        if !command.enabled {
            return Ok(InverterResolution {
                phase_a: PhaseTerminal::Floating,
                phase_b: PhaseTerminal::Floating,
                phase_c: PhaseTerminal::Floating,
                faults: Vec::new(),
            });
        }

        let mut faults = Vec::new();
        let phase_a = resolve_leg(command.phase_a, Phase::A, bus_voltage_v, &mut faults);
        let phase_b = resolve_leg(command.phase_b, Phase::B, bus_voltage_v, &mut faults);
        let phase_c = resolve_leg(command.phase_c, Phase::C, bus_voltage_v, &mut faults);
        Ok(InverterResolution {
            phase_a,
            phase_b,
            phase_c,
            faults,
        })
    }
}

fn resolve_leg(
    gates: GatePair,
    phase: Phase,
    bus_voltage_v: f64,
    faults: &mut Vec<InverterFault>,
) -> PhaseTerminal {
    match (gates.high, gates.low) {
        (true, false) => PhaseTerminal::Bus(bus_voltage_v),
        (false, true) => PhaseTerminal::Low,
        (false, false) => PhaseTerminal::Floating,
        (true, true) => {
            faults.push(InverterFault::ShootThrough { phase });
            PhaseTerminal::Floating
        }
    }
}

/// Samples the conventional six-state, 120-degree Hall pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HallSensors {
    pole_pairs: u8,
}

impl HallSensors {
    pub fn new(pole_pairs: u8) -> Result<Self, ModelError> {
        if pole_pairs == 0 {
            return Err(ModelError {
                field: "pole_pairs",
                message: "must be between 1 and 255 inclusive".to_owned(),
            });
        }
        Ok(Self { pole_pairs })
    }

    pub fn pole_pairs(self) -> u8 {
        self.pole_pairs
    }

    /// Samples a three-bit Hall state from absolute mechanical angle.
    pub fn sample(self, mechanical_angle_rad: f64) -> Result<u8, ModelError> {
        validate_finite("mechanical_angle_rad", mechanical_angle_rad)?;
        let electrical_angle_rad = mechanical_angle_rad * f64::from(self.pole_pairs);
        if !electrical_angle_rad.is_finite() {
            return Err(ModelError {
                field: "electrical_angle_rad",
                message: "angle conversion must remain finite".to_owned(),
            });
        }
        Ok(HALL_SEQUENCE[usize::from(commutation_sector(electrical_angle_rad))])
    }
}

/// Returns the normalized trapezoidal back-EMF shape in `[-1, 1]`.
///
/// Phase A rises linearly from zero to one over 0..30 electrical degrees,
/// stays at one through 150 degrees, falls to minus one through 210 degrees,
/// stays at minus one through 330 degrees, and returns linearly to zero.
pub fn trapezoidal_back_emf(electrical_angle_rad: f64) -> Result<f64, ModelError> {
    validate_finite("electrical_angle_rad", electrical_angle_rad)?;
    Ok(trapezoidal_back_emf_unchecked(electrical_angle_rad))
}

/// Returns A/B/C shapes with phase offsets `0`, `-2π/3`, and `+2π/3`.
pub fn phase_back_emf_shapes(electrical_angle_rad: f64) -> Result<[f64; 3], ModelError> {
    validate_finite("electrical_angle_rad", electrical_angle_rad)?;
    Ok(
        PHASE_OFFSET_RAD
            .map(|offset| trapezoidal_back_emf_unchecked(electrical_angle_rad + offset)),
    )
}

fn trapezoidal_back_emf_unchecked(electrical_angle_rad: f64) -> f64 {
    let angle = wrap_angle(electrical_angle_rad);
    let half_turns = angle / PI;
    let shape = if half_turns < 1.0 / 6.0 {
        6.0 * half_turns
    } else if half_turns < 5.0 / 6.0 {
        1.0
    } else if half_turns < 7.0 / 6.0 {
        6.0 - 6.0 * half_turns
    } else if half_turns < 11.0 / 6.0 {
        -1.0
    } else {
        6.0 * half_turns - 12.0
    };
    if shape.abs() <= 8.0 * f64::EPSILON {
        0.0
    } else if (shape.abs() - 1.0).abs() <= 8.0 * f64::EPSILON {
        shape.signum()
    } else {
        shape
    }
}

/// Electrical and mechanical parameters for the reduced-order BLDC plant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BldcMotorParams {
    pub resistance_ohm: f64,
    pub inductance_h: f64,
    pub torque_constant_nm_per_a: f64,
    pub back_emf_constant_v_per_rad_s: f64,
    pub supply_voltage_v: f64,
    pub pole_pairs: u8,
    /// Per-phase absolute current threshold. `None` disables overcurrent trip.
    pub current_limit_a: Option<f64>,
    /// Consecutive integration steps above the limit before latching.
    pub overcurrent_trip_steps: u32,
    pub shaft: ShaftParams,
}

/// Active motor and sensor faults.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BldcFaults {
    pub stalled: bool,
    pub overcurrent: bool,
    pub open_phases: [bool; 3],
    pub forced_hall_state: Option<u8>,
    pub hall_line_low: Option<Phase>,
    /// Effective fault-injected DC bus voltage.
    ///
    /// Values must be finite, positive, and no greater than the configured
    /// runtime supply voltage.
    pub undervoltage_v: Option<f64>,
}

/// Observable state after a completed fixed integration step.
#[derive(Debug, Clone, PartialEq)]
pub struct BldcMotorSnapshot {
    pub phase_currents_a: [f64; 3],
    pub phase_back_emf_v: [f64; 3],
    pub electromagnetic_torque_nm: f64,
    /// Positive current flows out of the DC bus through enabled high-side legs.
    pub dc_bus_current_a: f64,
    /// Effective bus voltage after applying any undervoltage fault.
    pub dc_bus_voltage_v: f64,
    pub position_rad: f64,
    pub wrapped_position_rad: f64,
    pub angular_velocity_rad_s: f64,
    pub speed_rpm: f64,
    pub electrical_angle_rad: f64,
    pub hall_state: u8,
    pub commutation_sector: u8,
    pub inverter_faults: Vec<InverterFault>,
    pub faults: BldcFaults,
}

/// Deterministic fixed-step BLDC, inverter, and Hall plant for six-step control.
///
/// This reduced-order model represents phase resistance, phase inductance,
/// trapezoidal back-EMF, a floating neutral, and a lumped mechanical shaft. It
/// does not model PWM switching edges and makes no FOC or SVPWM claim.
#[derive(Debug, Clone)]
pub struct BldcMotor {
    params: BldcMotorParams,
    shaft: Shaft,
    hall_sensors: HallSensors,
    phase_currents_a: [f64; 3],
    phase_back_emf_v: [f64; 3],
    electromagnetic_torque_nm: f64,
    dc_bus_current_a: f64,
    speed_rpm: f64,
    electrical_angle_rad: f64,
    hall_state: u8,
    commutation_sector: u8,
    inverter_faults: Vec<InverterFault>,
    last_terminals: [PhaseTerminal; 3],
    faults: BldcFaults,
    overcurrent_steps: u32,
}

impl BldcMotor {
    pub fn new(params: BldcMotorParams) -> Result<Self, ModelError> {
        validate_positive("resistance_ohm", params.resistance_ohm)?;
        validate_positive("inductance_h", params.inductance_h)?;
        validate_positive("torque_constant_nm_per_a", params.torque_constant_nm_per_a)?;
        validate_positive(
            "back_emf_constant_v_per_rad_s",
            params.back_emf_constant_v_per_rad_s,
        )?;
        validate_positive("supply_voltage_v", params.supply_voltage_v)?;
        if let Some(limit) = params.current_limit_a {
            validate_positive("current_limit_a", limit)?;
            if params.overcurrent_trip_steps == 0 {
                return Err(ModelError {
                    field: "overcurrent_trip_steps",
                    message: "must be greater than zero when current_limit_a is set".into(),
                });
            }
        }
        let hall_sensors = HallSensors::new(params.pole_pairs)?;
        let shaft = Shaft::new(params.shaft)?;
        let hall_state = hall_sensors.sample(0.0)?;

        Ok(Self {
            params,
            shaft,
            hall_sensors,
            phase_currents_a: [0.0; 3],
            phase_back_emf_v: [0.0; 3],
            electromagnetic_torque_nm: 0.0,
            dc_bus_current_a: 0.0,
            speed_rpm: 0.0,
            electrical_angle_rad: 0.0,
            hall_state,
            commutation_sector: 0,
            inverter_faults: Vec::new(),
            last_terminals: [PhaseTerminal::Floating; 3],
            faults: BldcFaults::default(),
            overcurrent_steps: 0,
        })
    }

    pub fn params(&self) -> BldcMotorParams {
        self.params
    }

    /// Bus-service substep with a 2.5% normalized-step accuracy budget inside
    /// the coupled electrical/mechanical envelope enforced by `step`.
    /// The electrical `L/R` time constant alone does not bound a low-inertia
    /// motor; merely staying stable also leaves large first-order errors.
    pub(crate) fn service_timestep_s(&self) -> Result<f64, ModelError> {
        let [electrical_rate, mechanical_rate, coupling_rate] = integration_rates(self.params);
        let step_s = 0.025 / (electrical_rate + mechanical_rate + coupling_rate);
        validate_positive("dt_s", step_s)?;
        validate_timestep(self.params, step_s)?;
        Ok(step_s)
    }

    pub fn faults(&self) -> BldcFaults {
        self.faults
    }

    pub fn snapshot(&self) -> BldcMotorSnapshot {
        let shaft = self.shaft.snapshot();
        BldcMotorSnapshot {
            phase_currents_a: self.phase_currents_a,
            phase_back_emf_v: self.phase_back_emf_v,
            electromagnetic_torque_nm: self.electromagnetic_torque_nm,
            dc_bus_current_a: self.dc_bus_current_a,
            dc_bus_voltage_v: self.effective_bus_voltage_v(),
            position_rad: shaft.position_rad,
            wrapped_position_rad: shaft.wrapped_position_rad,
            angular_velocity_rad_s: shaft.angular_velocity_rad_s,
            speed_rpm: self.speed_rpm,
            electrical_angle_rad: self.electrical_angle_rad,
            hall_state: self.hall_state,
            commutation_sector: self.commutation_sector,
            inverter_faults: self.inverter_faults.clone(),
            faults: self.faults,
        }
    }

    /// Atomically replaces active motor and Hall faults.
    pub fn set_faults(&mut self, faults: BldcFaults) -> Result<(), ModelError> {
        validate_faults(faults, self.params.supply_voltage_v)?;
        let mut candidate = self.clone();
        candidate.faults = faults;
        if !faults.overcurrent {
            candidate.overcurrent_steps = 0;
        }
        for (index, is_open) in faults.open_phases.into_iter().enumerate() {
            if is_open {
                candidate.phase_currents_a[index] = 0.0;
            }
        }
        project_zero_sum(&mut candidate.phase_currents_a, faults.open_phases);
        validate_array("phase_currents_a", candidate.phase_currents_a)?;
        if faults.stalled {
            candidate.shaft.hold_still();
        }
        candidate.refresh_derived_state()?;
        *self = candidate;
        Ok(())
    }

    /// Atomically updates signed shaft load torque.
    pub fn set_load_torque_nm(&mut self, load_torque_nm: f64) -> Result<(), ModelError> {
        let mut candidate = self.clone();
        candidate.shaft.set_load_torque_nm(load_torque_nm)?;
        candidate.params.shaft.load_torque_nm = load_torque_nm;
        *self = candidate;
        Ok(())
    }

    /// Atomically updates the DC supply used by subsequent inverter steps.
    pub fn set_supply_voltage_v(&mut self, supply_voltage_v: f64) -> Result<(), ModelError> {
        validate_positive("supply_voltage_v", supply_voltage_v)?;
        let mut candidate = self.clone();
        candidate.params.supply_voltage_v = supply_voltage_v;
        validate_faults(candidate.faults, supply_voltage_v)?;
        *self = candidate;
        Ok(())
    }

    /// Advances the plant by one deterministic fixed step.
    ///
    /// Connected phases use explicit phase-winding integration. Their floating
    /// neutral is chosen so the connected winding derivatives sum to zero:
    /// `v_neutral = mean(v_terminal - R*i - e)`. Floating phases use exact
    /// open-circuit `L/R` decay, a deliberate reduced-order freewheel model;
    /// a faulted-open phase is held at exactly zero. The mean of all non-open
    /// candidate currents is removed after integration to enforce the
    /// three-wire current invariant.
    ///
    /// Away from zero speed torque is electromagnetic phase power divided by
    /// mechanical speed, `sum(e_phase*i_phase)/omega`. At speeds within
    /// `1e-9 rad/s` of zero the startup-safe fallback is
    /// `Kt * sum(normalized_emf_shape_phase*i_phase)`.
    pub fn step(&mut self, command: InverterCommand, dt_s: f64) -> Result<(), ModelError> {
        validate_positive("dt_s", dt_s)?;
        validate_timestep(self.params, dt_s)?;
        let resolution = Inverter::resolve(command, self.effective_bus_voltage_v())?;
        let terminals = resolution.terminals();

        let mut candidate = self.clone();
        if candidate.faults.stalled {
            candidate.shaft.hold_still();
        }

        let pre_step_omega = candidate.shaft.angular_velocity_rad_s();
        let shapes = phase_back_emf_shapes(candidate.electrical_angle_rad)?;
        let pre_step_emf = shapes
            .map(|shape| candidate.params.back_emf_constant_v_per_rad_s * pre_step_omega * shape);
        validate_array("phase_back_emf_v", pre_step_emf)?;

        let open_phases = candidate.faults.open_phases;
        let connected = terminals.map(|terminal| !matches!(terminal, PhaseTerminal::Floating));
        let connected_count = connected
            .iter()
            .enumerate()
            .filter(|(index, is_connected)| **is_connected && !open_phases[*index])
            .count();
        let neutral_voltage_v = if connected_count >= 2 {
            let sum = terminals
                .iter()
                .enumerate()
                .filter(|(index, terminal)| {
                    !matches!(terminal, PhaseTerminal::Floating) && !open_phases[*index]
                })
                .map(|(index, terminal)| {
                    terminal_voltage(*terminal)
                        - candidate.params.resistance_ohm * candidate.phase_currents_a[index]
                        - pre_step_emf[index]
                })
                .sum::<f64>();
            sum / connected_count as f64
        } else {
            0.0
        };
        validate_finite("neutral_voltage_v", neutral_voltage_v)?;

        let decay = (-dt_s * candidate.params.resistance_ohm / candidate.params.inductance_h).exp();
        validate_finite("open_phase_current_decay", decay)?;
        let mut phase_currents_a = candidate.phase_currents_a;
        for index in 0..3 {
            if open_phases[index] {
                phase_currents_a[index] = 0.0;
            } else if connected[index] && connected_count >= 2 {
                let derivative_a_per_s = (terminal_voltage(terminals[index])
                    - neutral_voltage_v
                    - candidate.params.resistance_ohm * candidate.phase_currents_a[index]
                    - pre_step_emf[index])
                    / candidate.params.inductance_h;
                phase_currents_a[index] =
                    candidate.phase_currents_a[index] + derivative_a_per_s * dt_s;
            } else {
                phase_currents_a[index] = candidate.phase_currents_a[index] * decay;
            }
        }
        project_zero_sum(&mut phase_currents_a, open_phases);
        validate_array("phase_currents_a", phase_currents_a)?;

        let torque_nm = calculate_electromagnetic_torque(
            candidate.params.torque_constant_nm_per_a,
            pre_step_omega,
            shapes,
            pre_step_emf,
            phase_currents_a,
        );
        validate_finite("electromagnetic_torque_nm", torque_nm)?;

        candidate.phase_currents_a = phase_currents_a;
        if candidate
            .params
            .current_limit_a
            .is_some_and(|limit| phase_currents_a.iter().any(|current| current.abs() > limit))
        {
            candidate.overcurrent_steps = candidate.overcurrent_steps.saturating_add(1);
            if candidate.overcurrent_steps >= candidate.params.overcurrent_trip_steps {
                candidate.faults.overcurrent = true;
            }
        } else {
            candidate.overcurrent_steps = 0;
        }
        candidate.electromagnetic_torque_nm = torque_nm;
        candidate.inverter_faults = resolution.faults;
        candidate.last_terminals = terminals;
        candidate.dc_bus_current_a =
            conducted_bus_current(terminals, phase_currents_a, open_phases);
        validate_finite("dc_bus_current_a", candidate.dc_bus_current_a)?;

        if !candidate.faults.stalled {
            candidate.shaft.step(torque_nm, dt_s)?;
        }
        candidate.refresh_observables()?;
        *self = candidate;
        Ok(())
    }

    fn effective_bus_voltage_v(&self) -> f64 {
        self.faults
            .undervoltage_v
            .unwrap_or(self.params.supply_voltage_v)
    }

    fn refresh_derived_state(&mut self) -> Result<(), ModelError> {
        self.refresh_observables()?;
        let shapes = phase_back_emf_shapes(self.electrical_angle_rad)?;
        self.electromagnetic_torque_nm = calculate_electromagnetic_torque(
            self.params.torque_constant_nm_per_a,
            self.shaft.angular_velocity_rad_s(),
            shapes,
            self.phase_back_emf_v,
            self.phase_currents_a,
        );
        validate_finite("electromagnetic_torque_nm", self.electromagnetic_torque_nm)?;
        self.dc_bus_current_a = conducted_bus_current(
            self.last_terminals,
            self.phase_currents_a,
            self.faults.open_phases,
        );
        validate_finite("dc_bus_current_a", self.dc_bus_current_a)?;
        Ok(())
    }

    fn refresh_observables(&mut self) -> Result<(), ModelError> {
        let omega = self.shaft.angular_velocity_rad_s();
        let electrical_angle_rad = self.shaft.position_rad() * f64::from(self.params.pole_pairs);
        validate_finite("electrical_angle_rad", electrical_angle_rad)?;
        self.electrical_angle_rad = wrap_angle(electrical_angle_rad);
        let shapes = phase_back_emf_shapes(self.electrical_angle_rad)?;
        self.phase_back_emf_v =
            shapes.map(|shape| self.params.back_emf_constant_v_per_rad_s * omega * shape);
        validate_array("phase_back_emf_v", self.phase_back_emf_v)?;
        self.speed_rpm = omega * (60.0 / TAU);
        validate_finite("speed_rpm", self.speed_rpm)?;
        self.commutation_sector = commutation_sector(self.electrical_angle_rad);
        let mut hall_state = self.hall_sensors.sample(self.shaft.position_rad())?;
        if let Some(forced_hall_state) = self.faults.forced_hall_state {
            hall_state = forced_hall_state;
        }
        if let Some(hall_line_low) = self.faults.hall_line_low {
            hall_state &= !hall_line_low.hall_mask();
        }
        self.hall_state = hall_state;
        Ok(())
    }
}

fn terminal_voltage(terminal: PhaseTerminal) -> f64 {
    match terminal {
        PhaseTerminal::Bus(voltage) => voltage,
        PhaseTerminal::Low | PhaseTerminal::Floating => 0.0,
    }
}

fn conducted_bus_current(
    terminals: [PhaseTerminal; 3],
    currents_a: [f64; 3],
    open_phases: [bool; 3],
) -> f64 {
    terminals
        .iter()
        .enumerate()
        .filter(|(index, terminal)| {
            matches!(terminal, PhaseTerminal::Bus(_)) && !open_phases[*index]
        })
        .map(|(index, _)| currents_a[index])
        .sum()
}

fn commutation_sector(electrical_angle_rad: f64) -> u8 {
    let sector_width = TAU / 6.0;
    let sector = (wrap_angle(electrical_angle_rad) / sector_width).floor() as u8;
    sector.min(5)
}

fn wrap_angle(angle_rad: f64) -> f64 {
    let wrapped = angle_rad.rem_euclid(TAU);
    if wrapped == TAU {
        0.0
    } else {
        wrapped
    }
}

fn dot_product(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn calculate_electromagnetic_torque(
    torque_constant_nm_per_a: f64,
    angular_velocity_rad_s: f64,
    shapes: [f64; 3],
    phase_back_emf_v: [f64; 3],
    phase_currents_a: [f64; 3],
) -> f64 {
    if angular_velocity_rad_s.abs() <= TORQUE_POWER_SPEED_EPSILON_RAD_S {
        torque_constant_nm_per_a * dot_product(shapes, phase_currents_a)
    } else {
        dot_product(phase_back_emf_v, phase_currents_a) / angular_velocity_rad_s
    }
}

fn project_zero_sum(currents: &mut [f64; 3], open_phases: [bool; 3]) {
    let active_count = open_phases.iter().filter(|is_open| !**is_open).count();
    if active_count == 0 {
        *currents = [0.0; 3];
        return;
    }
    let mean = currents
        .iter()
        .enumerate()
        .filter(|(index, _)| !open_phases[*index])
        .map(|(_, current)| *current)
        .sum::<f64>()
        / active_count as f64;
    for (index, current) in currents.iter_mut().enumerate() {
        if open_phases[index] {
            *current = 0.0;
        } else {
            *current -= mean;
        }
    }

    // Remove the final floating-point residual from one non-open phase.
    let residual = currents.iter().sum::<f64>();
    let correction_index = (0..3)
        .rev()
        .find(|index| !open_phases[*index])
        .expect("at least one phase remains");
    currents[correction_index] -= residual;
}

fn validate_faults(faults: BldcFaults, supply_voltage_v: f64) -> Result<(), ModelError> {
    if faults
        .forced_hall_state
        .is_some_and(|hall_state| hall_state > 0b111)
    {
        return Err(ModelError {
            field: "forced_hall_state",
            message: "must be a three-bit value between 0 and 7".to_owned(),
        });
    }
    if let Some(undervoltage_v) = faults.undervoltage_v {
        if !undervoltage_v.is_finite() || undervoltage_v <= 0.0 || undervoltage_v > supply_voltage_v
        {
            return Err(ModelError {
                field: "undervoltage_v",
                message: "must be finite, greater than zero, and no greater than the runtime supply voltage"
                    .to_owned(),
            });
        }
    }
    Ok(())
}

fn integration_rates(params: BldcMotorParams) -> [f64; 3] {
    // The zero-speed branch uses Kt while the power-consistent running branch
    // has effective torque gain Ke. Bound the stronger of the two so unequal
    // constants cannot make this envelope optimistic.
    let torque_gain = params
        .torque_constant_nm_per_a
        .max(params.back_emf_constant_v_per_rad_s);
    let coupling_rate = ((torque_gain * params.back_emf_constant_v_per_rad_s) * 3.0
        / params.inductance_h
        / params.shaft.inertia_kg_m2)
        .sqrt();
    [
        params.resistance_ohm / params.inductance_h,
        params.shaft.viscous_friction_nm_per_rad_s / params.shaft.inertia_kg_m2,
        coupling_rate,
    ]
}

fn validate_timestep(params: BldcMotorParams, dt_s: f64) -> Result<(), ModelError> {
    let [electrical_rate, mechanical_rate, coupling_rate] = integration_rates(params);
    let electrical_ratio = dt_s * electrical_rate;
    if !electrical_ratio.is_finite() || electrical_ratio >= 1.0 {
        return Err(ModelError {
            field: "dt_s",
            message: "exceeds the conservative electrical integration envelope \
                      (dt_s * resistance_ohm / inductance_h must be less than 1)"
                .to_owned(),
        });
    }

    let mechanical_ratio = dt_s * mechanical_rate;
    let coupled_ratio = electrical_ratio + mechanical_ratio + dt_s * coupling_rate;
    if !mechanical_ratio.is_finite()
        || !coupling_rate.is_finite()
        || !coupled_ratio.is_finite()
        || mechanical_ratio >= 1.0
        || coupled_ratio >= 1.0
    {
        return Err(ModelError {
            field: "dt_s",
            message: "exceeds the conservative coupled electromechanical integration envelope"
                .to_owned(),
        });
    }
    Ok(())
}

fn validate_positive(field: &'static str, value: f64) -> Result<(), ModelError> {
    validate_finite(field, value)?;
    if value <= 0.0 {
        return Err(ModelError {
            field,
            message: "must be greater than zero".to_owned(),
        });
    }
    Ok(())
}

fn validate_non_negative(field: &'static str, value: f64) -> Result<(), ModelError> {
    validate_finite(field, value)?;
    if value < 0.0 {
        return Err(ModelError {
            field,
            message: "must be non-negative".to_owned(),
        });
    }
    Ok(())
}

fn validate_finite(field: &'static str, value: f64) -> Result<(), ModelError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ModelError {
            field,
            message: "must be finite".to_owned(),
        })
    }
}

fn validate_array(field: &'static str, values: [f64; 3]) -> Result<(), ModelError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(ModelError {
            field,
            message: "step would produce a non-finite motor state".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        calculate_electromagnetic_torque, phase_back_emf_shapes, GatePair, HallSensors, Inverter,
        InverterCommand, InverterFault, Phase, PhaseTerminal,
    };

    #[test]
    fn phase_shapes_use_120_degree_offsets() {
        assert_eq!(phase_back_emf_shapes(0.0).unwrap(), [0.0, -1.0, 1.0]);
    }

    #[test]
    fn hall_centers_match_the_canonical_sequence() {
        let hall = HallSensors::new(1).unwrap();
        let states = (0..6)
            .map(|sector| {
                hall.sample((f64::from(sector) + 0.5) * std::f64::consts::TAU / 6.0)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(states, [0b001, 0b101, 0b100, 0b110, 0b010, 0b011]);
    }

    #[test]
    fn shoot_through_floats_only_the_affected_leg() {
        let result = Inverter::resolve(
            InverterCommand {
                enabled: true,
                phase_a: GatePair {
                    high: true,
                    low: true,
                },
                phase_b: GatePair::high(),
                phase_c: GatePair::low(),
            },
            6.0,
        )
        .unwrap();
        assert_eq!(result.phase_a, PhaseTerminal::Floating);
        assert_eq!(result.phase_b, PhaseTerminal::Bus(6.0));
        assert_eq!(result.phase_c, PhaseTerminal::Low);
        assert_eq!(
            result.faults,
            [InverterFault::ShootThrough { phase: Phase::A }]
        );
    }

    #[test]
    fn tiny_speed_uses_startup_torque_without_power_division() {
        let torque = calculate_electromagnetic_torque(
            0.02,
            f64::MIN_POSITIVE,
            [0.0, -1.0, 1.0],
            [0.0; 3],
            [0.0, -1.0, 1.0],
        );
        assert_eq!(torque, 0.04);
    }
}
