// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::f64::consts::TAU;

use super::{ModelError, Shaft, ShaftParams};

/// Electrical and mechanical parameters for a reduced-order brushed DC motor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushedMotorParams {
    pub resistance_ohm: f64,
    pub inductance_h: f64,
    pub torque_constant_nm_per_a: f64,
    pub back_emf_constant_v_per_rad_s: f64,
    pub supply_voltage_v: f64,
    pub shaft: ShaftParams,
}

/// Fault inputs applied to the motor model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MotorFaults {
    pub stalled: bool,
}

/// Observable state from a completed brushed-motor integration step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushedMotorSnapshot {
    pub current_a: f64,
    pub applied_voltage_v: f64,
    pub back_emf_v: f64,
    pub electromagnetic_torque_nm: f64,
    pub position_rad: f64,
    pub wrapped_position_rad: f64,
    pub angular_velocity_rad_s: f64,
    pub speed_rpm: f64,
    pub bridge_state: HBridgeState,
    pub faults: MotorFaults,
}

/// Electrical state selected by an H-bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HBridgeState {
    Forward,
    Reverse,
    Brake,
    Coast,
}

impl HBridgeState {
    /// Converts digital bridge inputs into a deterministic state.
    ///
    /// `enable = false` always selects [`Self::Coast`]. With the bridge
    /// enabled, `brake = true` always selects [`Self::Brake`]. Otherwise the
    /// input truth table is:
    ///
    /// | `in1` | `in2` | State |
    /// |---|---|---|
    /// | false | false | Coast |
    /// | true | false | Forward |
    /// | false | true | Reverse |
    /// | true | true | Brake |
    pub fn from_pins(enable: bool, in1: bool, in2: bool, brake: bool) -> Self {
        if !enable {
            return Self::Coast;
        }
        if brake {
            return Self::Brake;
        }
        match (in1, in2) {
            (false, false) => Self::Coast,
            (true, false) => Self::Forward,
            (false, true) => Self::Reverse,
            (true, true) => Self::Brake,
        }
    }
}

/// H-bridge state and normalized PWM duty for one fixed motor step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HBridgeCommand {
    pub state: HBridgeState,
    pub duty: f64,
}

impl HBridgeCommand {
    pub fn forward(duty: f64) -> Result<Self, ModelError> {
        Self::driven(HBridgeState::Forward, duty)
    }

    pub fn reverse(duty: f64) -> Result<Self, ModelError> {
        Self::driven(HBridgeState::Reverse, duty)
    }

    pub fn brake() -> Self {
        Self {
            state: HBridgeState::Brake,
            duty: 0.0,
        }
    }

    pub fn coast() -> Self {
        Self {
            state: HBridgeState::Coast,
            duty: 0.0,
        }
    }

    fn driven(state: HBridgeState, duty: f64) -> Result<Self, ModelError> {
        validate_duty(duty)?;
        Ok(Self { state, duty })
    }

    fn validate(self) -> Result<(), ModelError> {
        validate_duty(self.duty)
    }
}

/// Deterministic fixed-step reduced-order brushed DC motor.
#[derive(Debug, Clone)]
pub struct BrushedDcMotor {
    params: BrushedMotorParams,
    shaft: Shaft,
    current_a: f64,
    applied_voltage_v: f64,
    back_emf_v: f64,
    electromagnetic_torque_nm: f64,
    speed_rpm: f64,
    bridge_state: HBridgeState,
    faults: MotorFaults,
    /// The winding current comes from outside (an analog netlist), not from
    /// this model's own electrical integration. See
    /// [`Self::new_circuit_driven`].
    circuit_driven: bool,
}

impl BrushedDcMotor {
    /// A motor whose winding is an element of an external circuit.
    ///
    /// The circuit solves the winding current (its R and L are in the netlist,
    /// together with whatever switch, flyback diode and rail drive it) and
    /// hands it in through [`Self::step_with_winding_current`]; this model
    /// only turns that current into torque, integrates the shaft, and reports
    /// the back-EMF the circuit must see. `supply_voltage_v` is not used and
    /// may be zero: the circuit owns the supply. [`Self::step`] is refused.
    pub fn new_circuit_driven(params: BrushedMotorParams) -> Result<Self, ModelError> {
        let mut checked = params;
        // Every other parameter is validated exactly as for a bridge motor.
        checked.supply_voltage_v = 1.0;
        let mut motor = Self::new(checked)?;
        motor.params.supply_voltage_v = params.supply_voltage_v.max(0.0);
        motor.circuit_driven = true;
        Ok(motor)
    }

    /// True when the winding current is supplied from outside.
    pub fn is_circuit_driven(&self) -> bool {
        self.circuit_driven
    }

    /// Advances the shaft by `dt_s` with the winding carrying `current_a`
    /// (positive = forward torque), held for the whole step.
    ///
    /// Only valid on a [`Self::new_circuit_driven`] motor. The shaft update is
    /// the same semi-implicit step a bridge motor uses; the explicit-
    /// integration envelope that applies is the mechanical one
    /// (`dt_s * viscous_friction_nm_per_rad_s / inertia_kg_m2 < 2`). The
    /// winding's electrical dynamics are the circuit's, so the electrical
    /// envelope does not apply here.
    pub fn step_with_winding_current(
        &mut self,
        current_a: f64,
        dt_s: f64,
    ) -> Result<(), ModelError> {
        if !self.circuit_driven {
            return Err(ModelError {
                field: "current_a",
                message: "this motor integrates its own winding current; \
                          only a circuit-driven motor takes it from outside"
                    .to_owned(),
            });
        }
        if !current_a.is_finite() {
            return Err(ModelError {
                field: "current_a",
                message: "must be finite".to_owned(),
            });
        }
        validate_positive("dt_s", dt_s)?;
        let friction = self.params.shaft.viscous_friction_nm_per_rad_s;
        if friction > 0.0 {
            let ratio = dt_s * friction / self.params.shaft.inertia_kg_m2;
            if !ratio.is_finite() || ratio >= 2.0 {
                return Err(ModelError {
                    field: "dt_s",
                    message: "exceeds the mechanical explicit-integration stability envelope \
                              (dt_s * viscous_friction_nm_per_rad_s / inertia_kg_m2 must be less than 2)"
                        .to_owned(),
                });
            }
        }
        let mut candidate = self.clone();
        if candidate.faults.stalled {
            candidate.shaft.hold_still();
        }
        candidate.current_a = current_a;
        candidate.electromagnetic_torque_nm = candidate.params.torque_constant_nm_per_a * current_a;
        validate_candidate(
            "electromagnetic_torque_nm",
            candidate.electromagnetic_torque_nm,
        )?;
        if !candidate.faults.stalled {
            candidate
                .shaft
                .step(candidate.electromagnetic_torque_nm, dt_s)?;
        }
        candidate.back_emf_v = candidate.params.back_emf_constant_v_per_rad_s
            * candidate.shaft.angular_velocity_rad_s();
        validate_candidate("back_emf_v", candidate.back_emf_v)?;
        // What the winding terminals see in steady state; the inductive term is
        // the circuit's and is not known here.
        candidate.applied_voltage_v =
            candidate.back_emf_v + candidate.params.resistance_ohm * current_a;
        candidate.speed_rpm = candidate.shaft.angular_velocity_rad_s() * (60.0 / TAU);
        validate_candidate("speed_rpm", candidate.speed_rpm)?;
        *self = candidate;
        Ok(())
    }

    pub fn new(params: BrushedMotorParams) -> Result<Self, ModelError> {
        validate_positive("resistance_ohm", params.resistance_ohm)?;
        validate_positive("inductance_h", params.inductance_h)?;
        validate_positive("torque_constant_nm_per_a", params.torque_constant_nm_per_a)?;
        validate_positive(
            "back_emf_constant_v_per_rad_s",
            params.back_emf_constant_v_per_rad_s,
        )?;
        validate_positive("supply_voltage_v", params.supply_voltage_v)?;
        let shaft = Shaft::new(params.shaft)?;

        Ok(Self {
            params,
            shaft,
            current_a: 0.0,
            applied_voltage_v: 0.0,
            back_emf_v: 0.0,
            electromagnetic_torque_nm: 0.0,
            speed_rpm: 0.0,
            bridge_state: HBridgeState::Coast,
            faults: MotorFaults::default(),
            circuit_driven: false,
        })
    }

    pub fn params(&self) -> BrushedMotorParams {
        self.params
    }

    pub fn faults(&self) -> MotorFaults {
        self.faults
    }

    /// Replaces active faults.
    ///
    /// Engaging stall immediately clears shaft velocity while retaining the
    /// current position. Releasing stall therefore resumes from rest.
    pub fn set_faults(&mut self, faults: MotorFaults) {
        if faults.stalled {
            self.shaft.hold_still();
            self.back_emf_v = 0.0;
            self.speed_rpm = 0.0;
        }
        self.faults = faults;
    }

    /// Atomically updates signed shaft load torque.
    pub fn set_load_torque_nm(&mut self, load_torque_nm: f64) -> Result<(), ModelError> {
        let mut candidate = self.clone();
        candidate.shaft.set_load_torque_nm(load_torque_nm)?;
        candidate.params.shaft.load_torque_nm = load_torque_nm;
        *self = candidate;
        Ok(())
    }

    /// Atomically updates the DC supply used by subsequent bridge steps.
    pub fn set_supply_voltage_v(&mut self, supply_voltage_v: f64) -> Result<(), ModelError> {
        if self.circuit_driven {
            return Err(ModelError {
                field: "supply_voltage_v",
                message: "a circuit-driven motor is supplied by its circuit's rail".to_owned(),
            });
        }
        validate_positive("supply_voltage_v", supply_voltage_v)?;
        self.params.supply_voltage_v = supply_voltage_v;
        Ok(())
    }

    pub fn snapshot(&self) -> BrushedMotorSnapshot {
        let shaft = self.shaft.snapshot();
        BrushedMotorSnapshot {
            current_a: self.current_a,
            applied_voltage_v: self.applied_voltage_v,
            back_emf_v: self.back_emf_v,
            electromagnetic_torque_nm: self.electromagnetic_torque_nm,
            position_rad: shaft.position_rad,
            wrapped_position_rad: shaft.wrapped_position_rad,
            angular_velocity_rad_s: shaft.angular_velocity_rad_s,
            speed_rpm: self.speed_rpm,
            bridge_state: self.bridge_state,
            faults: self.faults,
        }
    }

    /// Advances electrical and mechanical state by one fixed explicit step.
    ///
    /// Closed-winding states use explicit current integration followed by a
    /// semi-implicit shaft update. Their coupled 2x2 state matrix must satisfy
    /// the strict second-order Jury stability conditions. All bridge states
    /// also conservatively require `dt_s * resistance_ohm / inductance_h < 2`
    /// and, when viscous friction is nonzero,
    /// `dt_s * viscous_friction_nm_per_rad_s / inertia_kg_m2 < 2`. Coast
    /// current itself uses exact exponential decay.
    pub fn step(&mut self, command: HBridgeCommand, dt_s: f64) -> Result<(), ModelError> {
        if self.circuit_driven {
            return Err(ModelError {
                field: "command",
                message: "a circuit-driven motor takes its winding current from the circuit, \
                          not from a bridge command"
                    .to_owned(),
            });
        }
        command.validate()?;
        validate_positive("dt_s", dt_s)?;
        validate_timestep(self.params, command.state, dt_s)?;

        let mut candidate = self.clone();
        if candidate.faults.stalled {
            candidate.shaft.hold_still();
        }

        candidate.applied_voltage_v = match command.state {
            HBridgeState::Forward => candidate.params.supply_voltage_v * command.duty,
            HBridgeState::Reverse => -candidate.params.supply_voltage_v * command.duty,
            HBridgeState::Brake | HBridgeState::Coast => 0.0,
        };
        validate_candidate("applied_voltage_v", candidate.applied_voltage_v)?;
        let pre_step_back_emf_v = candidate.params.back_emf_constant_v_per_rad_s
            * candidate.shaft.angular_velocity_rad_s();
        validate_candidate("back_emf_v", pre_step_back_emf_v)?;

        let current_a = if command.state == HBridgeState::Coast {
            // An open bridge cannot sustain or generate winding current in this
            // reduced-order model. Exact exponential decay uses the winding L/R
            // time constant and deliberately does not model switching paths or
            // switching losses.
            let decay =
                (-dt_s * candidate.params.resistance_ohm / candidate.params.inductance_h).exp();
            validate_candidate("coast_current_decay", decay)?;
            candidate.current_a * decay
        } else {
            let current_derivative_a_per_s = (candidate.applied_voltage_v
                - candidate.params.resistance_ohm * candidate.current_a
                - pre_step_back_emf_v)
                / candidate.params.inductance_h;
            candidate.current_a + current_derivative_a_per_s * dt_s
        };
        validate_candidate("current_a", current_a)?;
        candidate.current_a = current_a;
        candidate.electromagnetic_torque_nm =
            candidate.params.torque_constant_nm_per_a * candidate.current_a;
        validate_candidate(
            "electromagnetic_torque_nm",
            candidate.electromagnetic_torque_nm,
        )?;
        if !candidate.faults.stalled {
            candidate
                .shaft
                .step(candidate.electromagnetic_torque_nm, dt_s)?;
        }
        candidate.back_emf_v = candidate.params.back_emf_constant_v_per_rad_s
            * candidate.shaft.angular_velocity_rad_s();
        validate_candidate("back_emf_v", candidate.back_emf_v)?;
        candidate.speed_rpm = candidate.shaft.angular_velocity_rad_s() * (60.0 / TAU);
        validate_candidate("speed_rpm", candidate.speed_rpm)?;
        candidate.bridge_state = command.state;
        *self = candidate;
        Ok(())
    }
}

fn validate_duty(duty: f64) -> Result<(), ModelError> {
    if !duty.is_finite() {
        return Err(ModelError {
            field: "duty",
            message: "must be finite".to_owned(),
        });
    }
    if !(0.0..=1.0).contains(&duty) {
        return Err(ModelError {
            field: "duty",
            message: "must be between zero and one inclusive".to_owned(),
        });
    }
    Ok(())
}

fn validate_positive(field: &'static str, value: f64) -> Result<(), ModelError> {
    if !value.is_finite() {
        return Err(ModelError {
            field,
            message: "must be finite".to_owned(),
        });
    }
    if value <= 0.0 {
        return Err(ModelError {
            field,
            message: "must be greater than zero".to_owned(),
        });
    }
    Ok(())
}

fn validate_candidate(field: &'static str, value: f64) -> Result<(), ModelError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ModelError {
            field,
            message: "step would produce a non-finite motor state".to_owned(),
        })
    }
}

fn validate_timestep(
    params: BrushedMotorParams,
    bridge_state: HBridgeState,
    dt_s: f64,
) -> Result<(), ModelError> {
    let electrical_ratio = dt_s * params.resistance_ohm / params.inductance_h;
    if !electrical_ratio.is_finite() || electrical_ratio >= 2.0 {
        return Err(ModelError {
            field: "dt_s",
            message: "exceeds the electrical explicit-integration stability envelope \
                      (dt_s * resistance_ohm / inductance_h must be less than 2)"
                .to_owned(),
        });
    }

    let friction = params.shaft.viscous_friction_nm_per_rad_s;
    if friction > 0.0 {
        let mechanical_ratio = dt_s * friction / params.shaft.inertia_kg_m2;
        if !mechanical_ratio.is_finite() || mechanical_ratio >= 2.0 {
            return Err(ModelError {
                field: "dt_s",
                message: "exceeds the mechanical explicit-integration stability envelope \
                          (dt_s * viscous_friction_nm_per_rad_s / inertia_kg_m2 must be less than 2)"
                    .to_owned(),
            });
        }
    }

    if bridge_state != HBridgeState::Coast {
        // Homogeneous closed-winding update:
        // i' = (1-a)i - cω
        // ω' = d(1-a)i + (1-e-dc)ω
        let a = dt_s * params.resistance_ohm / params.inductance_h;
        let c = dt_s * params.back_emf_constant_v_per_rad_s / params.inductance_h;
        let d = dt_s * params.torque_constant_nm_per_a / params.shaft.inertia_kg_m2;
        let e = dt_s * friction / params.shaft.inertia_kg_m2;
        let matrix_00 = 1.0 - a;
        let matrix_01 = -c;
        let matrix_10 = d * (1.0 - a);
        let matrix_11 = 1.0 - e - d * c;
        let trace = matrix_00 + matrix_11;
        let determinant = matrix_00 * matrix_11 - matrix_01 * matrix_10;
        let jury_first = 1.0 - trace + determinant;
        let jury_second = 1.0 + trace + determinant;
        let jury_third = 1.0 - determinant;

        if !trace.is_finite()
            || !determinant.is_finite()
            || !jury_first.is_finite()
            || !jury_second.is_finite()
            || !jury_third.is_finite()
            || jury_first <= 0.0
            || jury_second <= 0.0
            || jury_third <= 0.0
        {
            return Err(ModelError {
                field: "dt_s",
                message: "violates coupled closed-winding discrete stability \
                          (strict Jury conditions are not satisfied)"
                    .to_owned(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use super::{
        BrushedDcMotor, BrushedMotorParams, HBridgeCommand, HBridgeState, MotorFaults, ShaftParams,
    };

    fn motor_params() -> BrushedMotorParams {
        BrushedMotorParams {
            resistance_ohm: 2.0,
            inductance_h: 0.001,
            torque_constant_nm_per_a: 0.02,
            back_emf_constant_v_per_rad_s: 0.02,
            supply_voltage_v: 6.0,
            shaft: ShaftParams {
                inertia_kg_m2: 0.0001,
                viscous_friction_nm_per_rad_s: 0.00001,
                coulomb_friction_nm: 0.0,
                load_torque_nm: 0.0,
            },
        }
    }

    #[test]
    fn h_bridge_commands_validate_duty_and_select_state() {
        assert_eq!(
            HBridgeCommand::forward(0.25).unwrap().state,
            HBridgeState::Forward
        );
        assert_eq!(
            HBridgeCommand::reverse(0.75).unwrap().state,
            HBridgeState::Reverse
        );
        assert_eq!(HBridgeCommand::brake().state, HBridgeState::Brake);
        assert_eq!(HBridgeCommand::coast().state, HBridgeState::Coast);

        for duty in [-0.01, 1.01, f64::NAN, f64::INFINITY] {
            let error = HBridgeCommand::forward(duty).unwrap_err();
            assert_eq!(error.field, "duty");
        }
    }

    #[test]
    fn h_bridge_pin_truth_table_is_pure_and_complete() {
        assert_eq!(
            HBridgeState::from_pins(false, false, false, false),
            HBridgeState::Coast
        );
        assert_eq!(
            HBridgeState::from_pins(true, true, false, false),
            HBridgeState::Forward
        );
        assert_eq!(
            HBridgeState::from_pins(true, false, true, false),
            HBridgeState::Reverse
        );
        assert_eq!(
            HBridgeState::from_pins(true, false, false, false),
            HBridgeState::Coast
        );
        assert_eq!(
            HBridgeState::from_pins(true, true, true, false),
            HBridgeState::Brake
        );
        assert_eq!(
            HBridgeState::from_pins(true, true, false, true),
            HBridgeState::Brake
        );
    }

    #[test]
    fn motor_parameters_reject_non_positive_and_non_finite_values() {
        let mut cases = Vec::new();
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut params = motor_params();
            params.resistance_ohm = invalid;
            cases.push((params, "resistance_ohm"));

            let mut params = motor_params();
            params.inductance_h = invalid;
            cases.push((params, "inductance_h"));

            let mut params = motor_params();
            params.torque_constant_nm_per_a = invalid;
            cases.push((params, "torque_constant_nm_per_a"));

            let mut params = motor_params();
            params.back_emf_constant_v_per_rad_s = invalid;
            cases.push((params, "back_emf_constant_v_per_rad_s"));

            let mut params = motor_params();
            params.supply_voltage_v = invalid;
            cases.push((params, "supply_voltage_v"));
        }

        for (params, expected_field) in cases {
            let error = BrushedDcMotor::new(params).unwrap_err();
            assert_eq!(error.field, expected_field);
        }

        let mut params = motor_params();
        params.shaft.inertia_kg_m2 = 0.0;
        assert_eq!(
            BrushedDcMotor::new(params).unwrap_err().field,
            "inertia_kg_m2"
        );

        let mut params = motor_params();
        params.shaft.viscous_friction_nm_per_rad_s = -0.1;
        assert_eq!(
            BrushedDcMotor::new(params).unwrap_err().field,
            "viscous_friction_nm_per_rad_s"
        );

        let mut params = motor_params();
        params.shaft.load_torque_nm = f64::NAN;
        assert_eq!(
            BrushedDcMotor::new(params).unwrap_err().field,
            "load_torque_nm"
        );
    }

    #[test]
    fn first_electrical_step_matches_winding_equation() {
        let mut motor = BrushedDcMotor::new(motor_params()).unwrap();

        motor
            .step(HBridgeCommand::forward(1.0).unwrap(), 0.00001)
            .unwrap();

        let snapshot = motor.snapshot();
        assert!((snapshot.current_a - 0.06).abs() < 1e-12);
        assert_eq!(snapshot.applied_voltage_v, 6.0);
        assert!((snapshot.back_emf_v - 0.0000024).abs() < 1e-15);
        assert_eq!(
            snapshot.back_emf_v,
            motor.params().back_emf_constant_v_per_rad_s * snapshot.angular_velocity_rad_s
        );
        assert!((snapshot.electromagnetic_torque_nm - 0.0012).abs() < 1e-12);
        assert_eq!(
            snapshot.speed_rpm,
            snapshot.angular_velocity_rad_s * 60.0 / TAU
        );
        assert_eq!(snapshot.bridge_state, HBridgeState::Forward);
        assert_eq!(snapshot.faults, MotorFaults::default());
    }

    #[test]
    fn a_circuit_driven_motor_turns_the_handed_current_into_torque_and_emf() {
        let mut params = motor_params();
        params.supply_voltage_v = 0.0;
        assert!(BrushedDcMotor::new(params).is_err());
        let mut motor = BrushedDcMotor::new_circuit_driven(params).unwrap();
        assert!(motor.is_circuit_driven());
        assert!(motor
            .step(HBridgeCommand::forward(1.0).unwrap(), 0.00001)
            .is_err());
        for _ in 0..1000 {
            motor.step_with_winding_current(0.5, 0.0001).unwrap();
        }
        let spun = motor.snapshot();
        assert!(spun.speed_rpm > 0.0);
        assert_eq!(spun.current_a, 0.5);
        assert!(
            (spun.back_emf_v - params.back_emf_constant_v_per_rad_s * spun.angular_velocity_rad_s)
                .abs()
                < 1e-12
        );
        // Reverse current brakes the shaft.
        motor.step_with_winding_current(-0.5, 0.0001).unwrap();
        assert!(motor.snapshot().speed_rpm < spun.speed_rpm);
        // Bad input leaves the state untouched.
        let before = motor.snapshot();
        assert!(motor.step_with_winding_current(f64::NAN, 0.0001).is_err());
        assert!(motor.step_with_winding_current(0.1, 0.0).is_err());
        assert_eq!(motor.snapshot(), before);
    }

    #[test]
    fn a_bridge_motor_refuses_an_external_winding_current() {
        let mut motor = BrushedDcMotor::new(motor_params()).unwrap();
        assert!(!motor.is_circuit_driven());
        assert!(motor.step_with_winding_current(0.5, 0.0001).is_err());
    }
}
