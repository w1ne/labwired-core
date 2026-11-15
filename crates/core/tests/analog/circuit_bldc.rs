//! Circuit-owned currents advance one existing BLDC shaft, without a second
//! R/L integrator, inferred DC meter, timer drive, Hall sensor, or protection.
use labwired_core::physics::motor::{
    BldcFaults, BldcMotor, BldcMotorParams, InverterCommand, ShaftParams,
};

fn params() -> BldcMotorParams {
    BldcMotorParams {
        resistance_ohm: 0.35,
        inductance_h: 0.00018,
        torque_constant_nm_per_a: 0.04,
        back_emf_constant_v_per_rad_s: 0.04,
        supply_voltage_v: 0.0,
        pole_pairs: 7,
        current_limit_a: Some(1.0),
        overcurrent_trip_steps: 2,
        shaft: ShaftParams {
            inertia_kg_m2: 1e-7,
            viscous_friction_nm_per_rad_s: 0.0,
            coulomb_friction_nm: 0.0,
            load_torque_nm: 0.0,
        },
    }
}

#[test]
fn solved_phase_currents_are_consumed_without_reintegrating_or_clamping() {
    let mut a = BldcMotor::new_circuit(params()).unwrap();
    let mut alternate_rl = params();
    alternate_rl.resistance_ohm = 1.7;
    alternate_rl.inductance_h = 0.0005;
    let mut b = BldcMotor::new_circuit(alternate_rl).unwrap();
    let currents = [0.0, -2.0, 2.0];
    for _ in 0..2 {
        a.step_with_phase_currents(currents, 1e-6).unwrap();
        b.step_with_phase_currents(currents, 1e-6).unwrap();
    }
    assert_eq!(
        a.snapshot(),
        b.snapshot(),
        "R/L are already solved in the netlist"
    );
    let s = a.snapshot();
    assert_eq!(
        s.phase_currents_a, currents,
        "diagnostic overcurrent must not clamp"
    );
    assert!(s.faults.overcurrent);
    assert!(s.angular_velocity_rad_s > 0.0);
    assert!(s.phase_back_emf_v[2] > 0.0);
    assert_eq!(s.dc_bus_current_a, None);
    assert_eq!(s.dc_bus_voltage_v, None);
}

#[test]
fn phase_observations_reject_kcl_nonfinite_and_unstable_steps_atomically() {
    let mut motor = BldcMotor::new_circuit(params()).unwrap();
    motor
        .step_with_phase_currents([0.0, -1.0, 1.0], 1e-6)
        .unwrap();
    let before = motor.snapshot();
    for (current, dt) in [
        ([1.0, 1.0, 1.0], 1e-6),
        ([f64::NAN, -1.0, 1.0], 1e-6),
        ([0.0, -1.0, 1.0], -1.0),
        ([0.0, -1.0, 1.0], f64::INFINITY),
        ([0.0, -1.0, 1.0], 1.0),
    ] {
        assert!(motor.step_with_phase_currents(current, dt).is_err());
        assert_eq!(motor.snapshot(), before);
    }
}

#[test]
fn prime_reports_current_without_advancing_shaft_or_fault_counter() {
    let mut motor = BldcMotor::new_circuit(params()).unwrap();
    motor
        .step_with_phase_currents([0.0, -2.0, 2.0], 0.0)
        .unwrap();
    assert_eq!(motor.snapshot().position_rad, 0.0);
    assert_eq!(motor.snapshot().speed_rpm, 0.0);
    assert!(!motor.snapshot().faults.overcurrent);
    motor
        .step_with_phase_currents([0.0, -2.0, 2.0], 1e-6)
        .unwrap();
    assert!(!motor.snapshot().faults.overcurrent);
}

#[test]
fn circuit_mode_refuses_a_second_inverter_supply_and_electrical_fault_injection() {
    let mut motor = BldcMotor::new_circuit(params()).unwrap();
    assert!(motor
        .step(InverterCommand::six_step(0).unwrap(), 1e-6)
        .is_err());
    assert!(motor.set_supply_voltage_v(12.0).is_err());
    assert!(motor
        .set_faults(BldcFaults {
            open_phases: [false, true, false],
            ..Default::default()
        })
        .is_err());
    motor
        .set_faults(BldcFaults {
            stalled: true,
            ..Default::default()
        })
        .unwrap();
    motor
        .step_with_phase_currents([0.0, -2.0, 2.0], 1e-6)
        .unwrap();
    assert_eq!(motor.snapshot().speed_rpm, 0.0);
    assert_eq!(motor.snapshot().phase_currents_a, [0.0, -2.0, 2.0]);
    let mut invalid = params();
    invalid.supply_voltage_v = 24.0;
    assert!(BldcMotor::new_circuit(invalid).is_err());
}
