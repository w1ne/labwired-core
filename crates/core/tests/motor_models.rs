// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_core::physics::motor::{
    phase_back_emf_shapes, trapezoidal_back_emf, BldcFaults, BldcMotor, BldcMotorParams,
    BrushedDcMotor, BrushedMotorParams, GatePair, HBridgeCommand, HBridgeState, HallSensors,
    Inverter, InverterCommand, InverterFault, MotorFaults, Phase, PhaseTerminal, ShaftParams,
};

const DT_S: f64 = 0.00001;

fn params(load_torque_nm: f64) -> BrushedMotorParams {
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
            load_torque_nm,
        },
    }
}

fn step_many(motor: &mut BrushedDcMotor, command: HBridgeCommand, steps: usize) {
    for _ in 0..steps {
        motor.step(command, DT_S).unwrap();
    }
}

#[test]
fn forward_accelerates_positive_and_reverse_accelerates_negative() {
    let mut forward = BrushedDcMotor::new(params(0.0)).unwrap();
    let mut reverse = BrushedDcMotor::new(params(0.0)).unwrap();

    step_many(&mut forward, HBridgeCommand::forward(1.0).unwrap(), 1_000);
    step_many(&mut reverse, HBridgeCommand::reverse(1.0).unwrap(), 1_000);

    assert!(forward.snapshot().angular_velocity_rad_s > 0.0);
    assert!(reverse.snapshot().angular_velocity_rad_s < 0.0);
}

#[test]
fn back_emf_bounds_no_load_current_and_speed() {
    let mut motor = BrushedDcMotor::new(params(0.0)).unwrap();

    step_many(&mut motor, HBridgeCommand::forward(1.0).unwrap(), 100_000);

    let snapshot = motor.snapshot();
    assert!(snapshot.angular_velocity_rad_s > 200.0);
    assert!(snapshot.angular_velocity_rad_s < 320.0);
    assert!(snapshot.current_a > 0.0);
    assert!(snapshot.current_a < 0.5);
}

#[test]
fn increased_load_reduces_steady_speed_and_increases_current() {
    let mut no_load = BrushedDcMotor::new(params(0.0)).unwrap();
    let mut loaded = BrushedDcMotor::new(params(0.01)).unwrap();
    let command = HBridgeCommand::forward(1.0).unwrap();

    step_many(&mut no_load, command, 100_000);
    step_many(&mut loaded, command, 100_000);

    let no_load = no_load.snapshot();
    let loaded = loaded.snapshot();
    assert!(loaded.angular_velocity_rad_s < no_load.angular_velocity_rad_s);
    assert!(loaded.current_a > no_load.current_a);
}

#[test]
fn brake_opposes_rotation_while_coast_only_decays_current() {
    let mut braking = BrushedDcMotor::new(params(0.0)).unwrap();
    step_many(&mut braking, HBridgeCommand::forward(1.0).unwrap(), 20_000);
    let driven = braking.snapshot();

    step_many(&mut braking, HBridgeCommand::brake(), 2_000);
    let braked = braking.snapshot();
    assert!(braked.electromagnetic_torque_nm < 0.0);
    assert!(braked.angular_velocity_rad_s < driven.angular_velocity_rad_s);

    let mut coasting = BrushedDcMotor::new(params(0.0)).unwrap();
    coasting
        .step(HBridgeCommand::forward(1.0).unwrap(), DT_S)
        .unwrap();
    let driven_current = coasting.snapshot().current_a;
    step_many(&mut coasting, HBridgeCommand::coast(), 1_000);
    assert!(coasting.snapshot().current_a.abs() < driven_current.abs());

    let mut at_rest = BrushedDcMotor::new(params(0.0)).unwrap();
    step_many(&mut at_rest, HBridgeCommand::coast(), 1_000);
    assert_eq!(at_rest.snapshot().angular_velocity_rad_s, 0.0);
    assert_eq!(at_rest.snapshot().current_a, 0.0);
}

#[test]
fn coast_uses_exact_exponential_current_decay() {
    let mut coast_fixture = params(0.0);
    coast_fixture.resistance_ohm = 1.0;
    coast_fixture.inductance_h = 1.0;
    let mut motor = BrushedDcMotor::new(coast_fixture).unwrap();
    motor.set_faults(MotorFaults { stalled: true });
    motor
        .step(HBridgeCommand::forward(1.0).unwrap(), 0.1)
        .unwrap();
    let current_before_coast = motor.snapshot().current_a;

    motor.step(HBridgeCommand::coast(), 1.0).unwrap();

    let expected_current_a = current_before_coast * (-1.0_f64).exp();
    assert!((motor.snapshot().current_a - expected_current_a).abs() < 1e-12);
}

#[test]
fn stall_allows_current_but_holds_position_and_release_starts_from_zero() {
    let mut motor = BrushedDcMotor::new(params(0.0)).unwrap();
    let command = HBridgeCommand::forward(1.0).unwrap();
    step_many(&mut motor, command, 2_000);
    let moving = motor.snapshot();
    assert!(moving.angular_velocity_rad_s > 0.0);
    assert!(moving.back_emf_v > 0.0);

    motor.set_faults(MotorFaults { stalled: true });
    let engaged = motor.snapshot();
    assert_eq!(engaged.position_rad, moving.position_rad);
    assert_eq!(engaged.angular_velocity_rad_s, 0.0);
    assert_eq!(engaged.back_emf_v, 0.0);

    step_many(&mut motor, command, 1_000);
    let stalled = motor.snapshot();
    assert_eq!(stalled.position_rad, engaged.position_rad);
    assert_eq!(stalled.angular_velocity_rad_s, 0.0);
    assert!(stalled.current_a > engaged.current_a);

    motor.set_faults(MotorFaults { stalled: false });
    assert_eq!(motor.snapshot().angular_velocity_rad_s, 0.0);
    motor.step(command, DT_S).unwrap();
    assert!(motor.snapshot().angular_velocity_rad_s > 0.0);
}

#[test]
fn rejected_steps_leave_the_entire_snapshot_unchanged() {
    let mut motor = BrushedDcMotor::new(params(0.0)).unwrap();
    motor
        .step(HBridgeCommand::forward(0.5).unwrap(), DT_S)
        .unwrap();
    let before = motor.snapshot();

    let invalid_command = HBridgeCommand {
        state: HBridgeState::Forward,
        duty: f64::NAN,
    };
    assert_eq!(motor.step(invalid_command, DT_S).unwrap_err().field, "duty");
    assert_eq!(motor.snapshot(), before);

    let mut overflowing_current = params(0.0);
    overflowing_current.resistance_ohm = 0.5;
    overflowing_current.inductance_h = 1.0;
    overflowing_current.torque_constant_nm_per_a = f64::MIN_POSITIVE;
    overflowing_current.back_emf_constant_v_per_rad_s = f64::MIN_POSITIVE;
    overflowing_current.supply_voltage_v = f64::MAX;
    overflowing_current.shaft.inertia_kg_m2 = 1.0;
    overflowing_current.shaft.viscous_friction_nm_per_rad_s = 0.1;
    let mut overflowing_current = BrushedDcMotor::new(overflowing_current).unwrap();
    let before_overflow = overflowing_current.snapshot();
    assert_eq!(
        overflowing_current
            .step(HBridgeCommand::forward(1.0).unwrap(), 1.5)
            .unwrap_err()
            .field,
        "current_a"
    );
    assert_eq!(overflowing_current.snapshot(), before_overflow);

    let mut tiny_inertia = params(0.0);
    tiny_inertia.resistance_ohm = 1.0;
    tiny_inertia.inductance_h = 100.0;
    tiny_inertia.torque_constant_nm_per_a = 1.0;
    tiny_inertia.back_emf_constant_v_per_rad_s = f64::MIN_POSITIVE;
    tiny_inertia.supply_voltage_v = 100.0;
    tiny_inertia.shaft.inertia_kg_m2 = 5e-307;
    tiny_inertia.shaft.viscous_friction_nm_per_rad_s = 0.0;
    let mut motor = BrushedDcMotor::new(tiny_inertia).unwrap();
    let before = motor.snapshot();

    assert_eq!(
        motor
            .step(HBridgeCommand::forward(1.0).unwrap(), 10.0)
            .unwrap_err()
            .field,
        "state"
    );
    assert_eq!(motor.snapshot(), before);
}

#[test]
fn electrical_timestep_outside_stability_envelope_is_rejected_atomically() {
    let mut motor = BrushedDcMotor::new(params(0.0)).unwrap();
    let before = motor.snapshot();
    let electrical_limit_s = 2.0 * motor.params().inductance_h / motor.params().resistance_ohm;
    let outside_limit_s = f64::from_bits(electrical_limit_s.to_bits() + 1);

    let error = motor
        .step(HBridgeCommand::forward(1.0).unwrap(), outside_limit_s)
        .unwrap_err();

    assert_eq!(error.field, "dt_s");
    assert!(error.message.contains("electrical"));
    assert_eq!(motor.snapshot(), before);
}

#[test]
fn mechanical_timestep_outside_stability_envelope_is_rejected_atomically() {
    let mut mechanical_limited = params(0.0);
    mechanical_limited.resistance_ohm = 1.0;
    mechanical_limited.inductance_h = 100.0;
    mechanical_limited.shaft.inertia_kg_m2 = 1.0;
    mechanical_limited.shaft.viscous_friction_nm_per_rad_s = 1.0;
    let mut motor = BrushedDcMotor::new(mechanical_limited).unwrap();
    let before = motor.snapshot();
    let mechanical_limit_s = 2.0 * motor.params().shaft.inertia_kg_m2
        / motor.params().shaft.viscous_friction_nm_per_rad_s;
    let outside_limit_s = f64::from_bits(mechanical_limit_s.to_bits() + 1);

    let error = motor
        .step(HBridgeCommand::forward(1.0).unwrap(), outside_limit_s)
        .unwrap_err();

    assert_eq!(error.field, "dt_s");
    assert!(error.message.contains("mechanical"));
    assert_eq!(motor.snapshot(), before);
}

#[test]
fn coupled_closed_winding_instability_is_rejected_atomically() {
    let mut high_coupling = params(0.0);
    high_coupling.resistance_ohm = 1.0;
    high_coupling.inductance_h = 1.0;
    high_coupling.torque_constant_nm_per_a = 100.0;
    high_coupling.back_emf_constant_v_per_rad_s = 100.0;
    high_coupling.shaft.inertia_kg_m2 = 1.0;
    high_coupling.shaft.viscous_friction_nm_per_rad_s = 0.0;
    let mut motor = BrushedDcMotor::new(high_coupling).unwrap();
    let before = motor.snapshot();

    let error = motor
        .step(HBridgeCommand::forward(1.0).unwrap(), 0.1)
        .unwrap_err();

    assert_eq!(error.field, "dt_s");
    assert!(error.message.contains("coupled"));
    assert_eq!(motor.snapshot(), before);
}

#[test]
fn non_finite_rpm_conversion_is_rejected_atomically() {
    let mut extreme_speed = params(0.0);
    extreme_speed.resistance_ohm = 1.0;
    extreme_speed.inductance_h = 1.0;
    extreme_speed.torque_constant_nm_per_a = 1.0;
    extreme_speed.back_emf_constant_v_per_rad_s = f64::MIN_POSITIVE;
    extreme_speed.supply_voltage_v = 1.0;
    extreme_speed.shaft.inertia_kg_m2 = 5e-308;
    extreme_speed.shaft.viscous_friction_nm_per_rad_s = 0.0;
    let mut motor = BrushedDcMotor::new(extreme_speed).unwrap();
    let before = motor.snapshot();

    let error = motor
        .step(HBridgeCommand::forward(1.0).unwrap(), 1.0)
        .unwrap_err();

    assert_eq!(error.field, "speed_rpm");
    assert_eq!(motor.snapshot(), before);
}

#[test]
fn normal_fixture_is_inside_coupled_stability_envelope() {
    let mut motor = BrushedDcMotor::new(params(0.0)).unwrap();

    motor
        .step(HBridgeCommand::forward(1.0).unwrap(), DT_S)
        .unwrap();

    assert!(motor.snapshot().current_a > 0.0);
}

#[test]
fn identical_command_streams_are_exactly_deterministic() {
    let mut first = BrushedDcMotor::new(params(0.001)).unwrap();
    let mut second = BrushedDcMotor::new(params(0.001)).unwrap();

    for step in 0..10_000 {
        let command = match step % 4 {
            0 => HBridgeCommand::forward(0.75).unwrap(),
            1 => HBridgeCommand::reverse(0.25).unwrap(),
            2 => HBridgeCommand::brake(),
            _ => HBridgeCommand::coast(),
        };
        first.step(command, DT_S).unwrap();
        second.step(command, DT_S).unwrap();
    }

    assert_eq!(first.params(), second.params());
    assert_eq!(first.snapshot(), second.snapshot());
}

fn bldc_params(load_torque_nm: f64) -> BldcMotorParams {
    BldcMotorParams {
        resistance_ohm: 2.0,
        inductance_h: 0.001,
        torque_constant_nm_per_a: 0.02,
        back_emf_constant_v_per_rad_s: 0.02,
        supply_voltage_v: 6.0,
        pole_pairs: 1,
        current_limit_a: None,
        overcurrent_trip_steps: 3,
        shaft: ShaftParams {
            inertia_kg_m2: 0.0001,
            viscous_friction_nm_per_rad_s: 0.00001,
            coulomb_friction_nm: 0.0,
            load_torque_nm,
        },
    }
}

#[test]
fn bldc_overcurrent_requires_consecutive_steps_latches_and_clears_explicitly() {
    let mut fixture = bldc_params(0.0);
    fixture.current_limit_a = Some(0.01);
    fixture.overcurrent_trip_steps = 3;
    let mut motor = BldcMotor::new(fixture).unwrap();
    let drive = InverterCommand::six_step(0).unwrap();

    motor.step(drive, DT_S).unwrap();
    assert!(!motor.snapshot().faults.overcurrent);
    motor.step(drive, DT_S).unwrap();
    assert!(!motor.snapshot().faults.overcurrent);
    motor.step(drive, DT_S).unwrap();
    assert!(motor.snapshot().faults.overcurrent);

    motor.step(InverterCommand::off(), DT_S).unwrap();
    assert!(
        motor.snapshot().faults.overcurrent,
        "trip must remain latched"
    );

    motor.set_faults(BldcFaults::default()).unwrap();
    assert!(!motor.snapshot().faults.overcurrent);
    motor.step(drive, DT_S).unwrap();
    assert!(!motor.snapshot().faults.overcurrent);
}

#[test]
fn bldc_overcurrent_counter_resets_below_threshold() {
    let mut fixture = bldc_params(0.0);
    fixture.current_limit_a = Some(0.05);
    fixture.overcurrent_trip_steps = 3;
    let mut motor = BldcMotor::new(fixture).unwrap();
    let drive = InverterCommand::six_step(0).unwrap();

    motor.step(drive, DT_S).unwrap();
    motor.step(InverterCommand::off(), 0.0004).unwrap();
    for _ in 0..2 {
        motor.step(drive, DT_S).unwrap();
    }

    assert!(!motor.snapshot().faults.overcurrent);
}

#[test]
fn bldc_overcurrent_configuration_rejects_invalid_thresholds() {
    let mut fixture = bldc_params(0.0);
    fixture.current_limit_a = Some(0.0);
    assert_eq!(
        BldcMotor::new(fixture).unwrap_err().field,
        "current_limit_a"
    );

    fixture.current_limit_a = Some(1.0);
    fixture.overcurrent_trip_steps = 0;
    assert_eq!(
        BldcMotor::new(fixture).unwrap_err().field,
        "overcurrent_trip_steps"
    );
}

fn step_bldc_commutated(motor: &mut BldcMotor, reverse: bool, steps: usize) {
    for _ in 0..steps {
        let sector = motor.snapshot().commutation_sector;
        let command = if reverse {
            InverterCommand::reverse_six_step(sector).unwrap()
        } else {
            InverterCommand::six_step(sector).unwrap()
        };
        motor.step(command, DT_S).unwrap();
    }
}

fn snapshot_torque(motor: &BldcMotor) -> f64 {
    let snapshot = motor.snapshot();
    let params = motor.params();
    let shapes = phase_back_emf_shapes(snapshot.electrical_angle_rad).unwrap();
    if snapshot.angular_velocity_rad_s.abs() <= 1e-9 {
        params.torque_constant_nm_per_a
            * shapes
                .into_iter()
                .zip(snapshot.phase_currents_a)
                .map(|(shape, current)| shape * current)
                .sum::<f64>()
    } else {
        snapshot
            .phase_back_emf_v
            .into_iter()
            .zip(snapshot.phase_currents_a)
            .map(|(emf, current)| emf * current)
            .sum::<f64>()
            / snapshot.angular_velocity_rad_s
    }
}

fn snapshot_bus_current(motor: &BldcMotor, command: InverterCommand) -> f64 {
    let snapshot = motor.snapshot();
    let gates = [command.phase_a, command.phase_b, command.phase_c];
    gates
        .into_iter()
        .enumerate()
        .filter(|(index, gates)| gates.high && !gates.low && !snapshot.faults.open_phases[*index])
        .map(|(index, _)| snapshot.phase_currents_a[index])
        .sum()
}

#[test]
fn bldc_trapezoid_is_normalized_periodic_and_phase_shifted() {
    let pi = std::f64::consts::PI;
    assert_eq!(trapezoidal_back_emf(0.0).unwrap(), 0.0);
    assert_eq!(trapezoidal_back_emf(pi / 6.0).unwrap(), 1.0);
    assert_eq!(trapezoidal_back_emf(pi / 2.0).unwrap(), 1.0);
    assert_eq!(trapezoidal_back_emf(pi).unwrap(), 0.0);
    assert_eq!(trapezoidal_back_emf(7.0 * pi / 6.0).unwrap(), -1.0);
    assert_eq!(trapezoidal_back_emf(3.0 * pi / 2.0).unwrap(), -1.0);
    assert_eq!(
        trapezoidal_back_emf(pi / 5.0).unwrap(),
        trapezoidal_back_emf(pi / 5.0 + std::f64::consts::TAU).unwrap()
    );
    assert_eq!(phase_back_emf_shapes(0.0).unwrap(), [0.0, -1.0, 1.0]);
    assert_eq!(
        trapezoidal_back_emf(f64::NAN).unwrap_err().field,
        "electrical_angle_rad"
    );
}

#[test]
fn hall_sensors_follow_forward_and_reverse_six_sector_sequences() {
    let hall = HallSensors::new(1).unwrap();
    let center = std::f64::consts::TAU / 12.0;
    let width = std::f64::consts::TAU / 6.0;
    let forward = (0..6)
        .map(|sector| hall.sample(center + f64::from(sector) * width).unwrap())
        .collect::<Vec<_>>();
    let reverse = (0..6)
        .map(|sector| hall.sample(-center - f64::from(sector) * width).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(forward, [0b001, 0b101, 0b100, 0b110, 0b010, 0b011]);
    assert_eq!(reverse, [0b011, 0b010, 0b110, 0b100, 0b101, 0b001]);
    assert_eq!(HallSensors::new(0).unwrap_err().field, "pole_pairs");
    assert_eq!(
        hall.sample(f64::INFINITY).unwrap_err().field,
        "mechanical_angle_rad"
    );
}

#[test]
fn inverter_resolves_legs_and_makes_shoot_through_safe() {
    let command = InverterCommand {
        enabled: true,
        phase_a: GatePair {
            high: true,
            low: false,
        },
        phase_b: GatePair {
            high: false,
            low: true,
        },
        phase_c: GatePair {
            high: false,
            low: false,
        },
    };
    let resolved = Inverter::resolve(command, 12.0).unwrap();
    assert_eq!(resolved.phase_a, PhaseTerminal::Bus(12.0));
    assert_eq!(resolved.phase_b, PhaseTerminal::Low);
    assert_eq!(resolved.phase_c, PhaseTerminal::Floating);
    assert!(resolved.faults.is_empty());

    let disabled = Inverter::resolve(
        InverterCommand {
            enabled: false,
            phase_a: GatePair {
                high: true,
                low: true,
            },
            ..InverterCommand::off()
        },
        12.0,
    )
    .unwrap();
    assert_eq!(disabled.phase_a, PhaseTerminal::Floating);
    assert!(disabled.faults.is_empty());

    let shoot_through = Inverter::resolve(
        InverterCommand {
            enabled: true,
            phase_c: GatePair {
                high: true,
                low: true,
            },
            ..InverterCommand::off()
        },
        12.0,
    )
    .unwrap();
    assert_eq!(shoot_through.phase_c, PhaseTerminal::Floating);
    assert_eq!(
        shoot_through.faults,
        vec![InverterFault::ShootThrough { phase: Phase::C }]
    );
    assert_eq!(
        Inverter::resolve(command, f64::NAN).unwrap_err().field,
        "bus_voltage_v"
    );
}

#[test]
fn canonical_six_step_commands_cover_all_exact_gate_mappings() {
    let off = GatePair::off();
    let high = GatePair::high();
    let low = GatePair::low();
    let expected = [
        [off, low, high],
        [high, low, off],
        [high, off, low],
        [off, high, low],
        [low, high, off],
        [low, off, high],
    ];

    for (sector, gates) in expected.into_iter().enumerate() {
        let forward = InverterCommand::six_step(sector as u8).unwrap();
        assert_eq!([forward.phase_a, forward.phase_b, forward.phase_c], gates);
        let reverse = InverterCommand::reverse_six_step(sector as u8).unwrap();
        assert_eq!(
            [reverse.phase_a, reverse.phase_b, reverse.phase_c],
            gates.map(|gate| GatePair {
                high: gate.low,
                low: gate.high,
            })
        );
    }
}

#[test]
fn bldc_parameters_commands_and_faults_are_validated() {
    let mut invalid = bldc_params(0.0);
    invalid.inductance_h = 0.0;
    assert_eq!(BldcMotor::new(invalid).unwrap_err().field, "inductance_h");

    invalid = bldc_params(0.0);
    invalid.pole_pairs = 0;
    assert_eq!(BldcMotor::new(invalid).unwrap_err().field, "pole_pairs");

    assert_eq!(
        InverterCommand::six_step(6).unwrap_err().field,
        "commutation_sector"
    );

    let mut motor = BldcMotor::new(bldc_params(0.0)).unwrap();
    let before = motor.snapshot();
    assert_eq!(
        motor
            .set_faults(BldcFaults {
                forced_hall_state: Some(8),
                ..BldcFaults::default()
            })
            .unwrap_err()
            .field,
        "forced_hall_state"
    );
    assert_eq!(motor.snapshot(), before);
}

#[test]
fn canonical_six_step_commutation_starts_and_visits_every_hall_state() {
    let mut motor = BldcMotor::new(bldc_params(0.0)).unwrap();
    let mut hall_seen = [false; 8];
    let mut maximum_current_a = 0.0_f64;

    for _ in 0..200_000 {
        let snapshot = motor.snapshot();
        hall_seen[usize::from(snapshot.hall_state)] = true;
        maximum_current_a = maximum_current_a.max(
            snapshot
                .phase_currents_a
                .into_iter()
                .map(f64::abs)
                .fold(0.0, f64::max),
        );
        motor
            .step(
                InverterCommand::six_step(snapshot.commutation_sector).unwrap(),
                DT_S,
            )
            .unwrap();
    }

    let snapshot = motor.snapshot();
    assert!(snapshot.electromagnetic_torque_nm.is_finite());
    assert!(snapshot.angular_velocity_rad_s > 0.0);
    assert!(maximum_current_a < 10.0);
    for hall in [0b001_u8, 0b101, 0b100, 0b110, 0b010, 0b011] {
        assert!(
            hall_seen[usize::from(hall)],
            "Hall state {hall:03b} not seen"
        );
    }
}

#[test]
fn reverse_six_step_commutation_rotates_negative() {
    let mut motor = BldcMotor::new(bldc_params(0.0)).unwrap();
    step_bldc_commutated(&mut motor, true, 30_000);
    assert!(motor.snapshot().angular_velocity_rad_s < 0.0);
}

#[test]
fn bldc_load_step_lowers_speed_and_increases_bus_current() {
    let mut baseline = BldcMotor::new(bldc_params(0.0)).unwrap();
    let mut loaded = BldcMotor::new(bldc_params(0.0)).unwrap();
    step_bldc_commutated(&mut baseline, false, 50_000);
    step_bldc_commutated(&mut loaded, false, 50_000);
    loaded.set_load_torque_nm(0.01).unwrap();

    let mut baseline_current = 0.0;
    let mut loaded_current = 0.0;
    for _ in 0..50_000 {
        step_bldc_commutated(&mut baseline, false, 1);
        step_bldc_commutated(&mut loaded, false, 1);
        baseline_current += baseline.snapshot().dc_bus_current_a.abs();
        loaded_current += loaded.snapshot().dc_bus_current_a.abs();
    }

    assert!(loaded.snapshot().angular_velocity_rad_s < baseline.snapshot().angular_velocity_rad_s);
    assert!(loaded_current > baseline_current);
}

#[test]
fn bldc_supply_voltage_can_be_lowered_atomically() {
    let mut motor = BldcMotor::new(bldc_params(0.0)).unwrap();
    motor.set_supply_voltage_v(1.0).unwrap();
    assert_eq!(motor.params().supply_voltage_v, 1.0);
    assert_eq!(motor.snapshot().dc_bus_voltage_v, 1.0);

    let before = motor.snapshot();
    assert_eq!(
        motor.set_supply_voltage_v(f64::NAN).unwrap_err().field,
        "supply_voltage_v"
    );
    assert_eq!(motor.snapshot(), before);
}

#[test]
fn bldc_undervoltage_fault_controls_effective_bus_and_response() {
    let mut nominal = BldcMotor::new(bldc_params(0.0)).unwrap();
    let mut undervoltage = BldcMotor::new(bldc_params(0.0)).unwrap();
    undervoltage
        .set_faults(BldcFaults {
            undervoltage_v: Some(1.0),
            ..BldcFaults::default()
        })
        .unwrap();
    assert_eq!(undervoltage.snapshot().dc_bus_voltage_v, 1.0);
    assert_eq!(undervoltage.snapshot().faults.undervoltage_v, Some(1.0));

    step_bldc_commutated(&mut nominal, false, 30_000);
    step_bldc_commutated(&mut undervoltage, false, 30_000);
    assert!(
        undervoltage.snapshot().angular_velocity_rad_s < nominal.snapshot().angular_velocity_rad_s
    );

    undervoltage.set_faults(BldcFaults::default()).unwrap();
    assert_eq!(undervoltage.snapshot().dc_bus_voltage_v, 6.0);
    assert_eq!(undervoltage.snapshot().faults.undervoltage_v, None);
}

#[test]
fn invalid_undervoltage_faults_leave_snapshot_unchanged() {
    let mut motor = BldcMotor::new(bldc_params(0.0)).unwrap();
    let before = motor.snapshot();
    for undervoltage_v in [0.0, f64::NAN, 7.0] {
        assert_eq!(
            motor
                .set_faults(BldcFaults {
                    undervoltage_v: Some(undervoltage_v),
                    ..BldcFaults::default()
                })
                .unwrap_err()
                .field,
            "undervoltage_v"
        );
        assert_eq!(motor.snapshot(), before);
    }
}

#[test]
fn bldc_stall_holds_position_and_releases_from_zero() {
    let mut motor = BldcMotor::new(bldc_params(0.0)).unwrap();
    step_bldc_commutated(&mut motor, false, 2_000);
    motor
        .set_faults(BldcFaults {
            stalled: true,
            ..BldcFaults::default()
        })
        .unwrap();
    let held_position = motor.snapshot().position_rad;
    let current_before = motor.snapshot().phase_currents_a;
    step_bldc_commutated(&mut motor, false, 500);
    assert_eq!(motor.snapshot().position_rad, held_position);
    assert_eq!(motor.snapshot().angular_velocity_rad_s, 0.0);
    assert_ne!(motor.snapshot().phase_currents_a, current_before);

    motor.set_faults(BldcFaults::default()).unwrap();
    assert_eq!(motor.snapshot().angular_velocity_rad_s, 0.0);
    step_bldc_commutated(&mut motor, false, 1);
    assert!(motor.snapshot().angular_velocity_rad_s > 0.0);
}

#[test]
fn bldc_open_phase_is_zero_and_other_currents_balance() {
    let mut motor = BldcMotor::new(bldc_params(0.0)).unwrap();
    step_bldc_commutated(&mut motor, false, 2_000);
    let command = InverterCommand::six_step(motor.snapshot().commutation_sector).unwrap();
    motor.step(command, DT_S).unwrap();
    motor
        .set_faults(BldcFaults {
            open_phases: [true, false, false],
            ..BldcFaults::default()
        })
        .unwrap();
    let snapshot = motor.snapshot();
    let currents = snapshot.phase_currents_a;
    assert_eq!(currents[0], 0.0);
    assert!((currents[1] + currents[2]).abs() < 1e-12);
    assert_eq!(snapshot.electromagnetic_torque_nm, snapshot_torque(&motor));
    assert_eq!(
        snapshot.dc_bus_current_a,
        snapshot_bus_current(&motor, command)
    );
}

#[test]
fn bldc_multiple_open_phases_are_all_held_at_zero() {
    let mut motor = BldcMotor::new(bldc_params(0.0)).unwrap();
    step_bldc_commutated(&mut motor, false, 2_000);
    motor
        .set_faults(BldcFaults {
            open_phases: [true, true, false],
            ..BldcFaults::default()
        })
        .unwrap();

    let snapshot = motor.snapshot();
    assert_eq!(snapshot.faults.open_phases, [true, true, false]);
    assert_eq!(snapshot.phase_currents_a, [0.0; 3]);

    let command = InverterCommand::six_step(snapshot.commutation_sector).unwrap();
    motor.step(command, DT_S).unwrap();
    assert_eq!(motor.snapshot().phase_currents_a, [0.0; 3]);
}

#[test]
fn engaging_stall_immediately_recomputes_derived_motor_values() {
    let mut motor = BldcMotor::new(bldc_params(0.0)).unwrap();
    step_bldc_commutated(&mut motor, false, 2_000);
    let command = InverterCommand::six_step(motor.snapshot().commutation_sector).unwrap();
    motor.step(command, DT_S).unwrap();

    motor
        .set_faults(BldcFaults {
            stalled: true,
            ..BldcFaults::default()
        })
        .unwrap();

    let snapshot = motor.snapshot();
    assert_eq!(snapshot.angular_velocity_rad_s, 0.0);
    assert_eq!(snapshot.speed_rpm, 0.0);
    assert_eq!(snapshot.phase_back_emf_v, [0.0; 3]);
    assert_eq!(snapshot.electromagnetic_torque_nm, snapshot_torque(&motor));
    assert_eq!(
        snapshot.dc_bus_current_a,
        snapshot_bus_current(&motor, command)
    );
}

#[test]
fn bldc_hall_faults_are_observable_and_typed() {
    let mut motor = BldcMotor::new(bldc_params(0.0)).unwrap();
    motor
        .set_faults(BldcFaults {
            forced_hall_state: Some(0b111),
            hall_line_low: Some(Phase::B),
            ..BldcFaults::default()
        })
        .unwrap();
    assert_eq!(motor.snapshot().hall_state, 0b101);
    assert_eq!(motor.snapshot().faults.forced_hall_state, Some(0b111));
}

#[test]
fn bldc_shoot_through_is_reported_without_nonfinite_physics() {
    let mut motor = BldcMotor::new(bldc_params(0.0)).unwrap();
    motor
        .step(
            InverterCommand {
                enabled: true,
                phase_a: GatePair {
                    high: true,
                    low: true,
                },
                ..InverterCommand::off()
            },
            DT_S,
        )
        .unwrap();
    let snapshot = motor.snapshot();
    assert_eq!(
        snapshot.inverter_faults,
        vec![InverterFault::ShootThrough { phase: Phase::A }]
    );
    assert!(snapshot
        .phase_currents_a
        .iter()
        .all(|value| value.is_finite()));
    assert!(snapshot
        .phase_back_emf_v
        .iter()
        .all(|value| value.is_finite()));
}

#[test]
fn rejected_bldc_steps_are_atomic() {
    let mut motor = BldcMotor::new(bldc_params(0.0)).unwrap();
    let before = motor.snapshot();
    assert_eq!(
        motor
            .step(InverterCommand::off(), f64::NAN)
            .unwrap_err()
            .field,
        "dt_s"
    );
    assert_eq!(motor.snapshot(), before);

    let electrical_limit = motor.params().inductance_h / motor.params().resistance_ohm;
    assert_eq!(
        motor
            .step(InverterCommand::off(), electrical_limit)
            .unwrap_err()
            .field,
        "dt_s"
    );
    assert_eq!(motor.snapshot(), before);

    let mut overflowing = bldc_params(0.0);
    overflowing.resistance_ohm = 1.0;
    overflowing.inductance_h = 1.0;
    overflowing.torque_constant_nm_per_a = f64::MAX;
    overflowing.back_emf_constant_v_per_rad_s = f64::MIN_POSITIVE;
    overflowing.supply_voltage_v = 100.0;
    overflowing.shaft.inertia_kg_m2 = f64::MAX;
    overflowing.shaft.viscous_friction_nm_per_rad_s = 0.0;
    let mut overflowing = BldcMotor::new(overflowing).unwrap();
    let before_overflow = overflowing.snapshot();
    assert_eq!(
        overflowing
            .step(InverterCommand::six_step(0).unwrap(), 0.1)
            .unwrap_err()
            .field,
        "electromagnetic_torque_nm"
    );
    assert_eq!(overflowing.snapshot(), before_overflow);
}

#[test]
fn coupled_bldc_instability_is_rejected_atomically() {
    let mut unstable = bldc_params(0.0);
    unstable.resistance_ohm = 1.0;
    unstable.inductance_h = 1.0;
    unstable.torque_constant_nm_per_a = 0.000001;
    unstable.back_emf_constant_v_per_rad_s = 100.0;
    unstable.shaft.inertia_kg_m2 = 1.0;
    unstable.shaft.viscous_friction_nm_per_rad_s = 0.0;
    let mut motor = BldcMotor::new(unstable).unwrap();
    let before = motor.snapshot();

    let error = motor
        .step(InverterCommand::six_step(0).unwrap(), 0.01)
        .unwrap_err();

    assert_eq!(error.field, "dt_s");
    assert!(error.message.contains("coupled"));
    assert_eq!(motor.snapshot(), before);
}

#[test]
fn bldc_observables_stay_finite_and_currents_sum_to_zero() {
    let mut motor = BldcMotor::new(bldc_params(0.0)).unwrap();
    for _ in 0..10_000 {
        let sector = motor.snapshot().commutation_sector;
        motor
            .step(InverterCommand::six_step(sector).unwrap(), DT_S)
            .unwrap();
        let snapshot = motor.snapshot();
        assert!(snapshot
            .phase_currents_a
            .iter()
            .all(|value| value.is_finite()));
        assert!(snapshot
            .phase_back_emf_v
            .iter()
            .all(|value| value.is_finite()));
        assert!(snapshot.electromagnetic_torque_nm.is_finite());
        assert!(snapshot.dc_bus_current_a.is_finite());
        assert!(snapshot.dc_bus_voltage_v.is_finite());
        assert!(snapshot.position_rad.is_finite());
        assert!(snapshot.wrapped_position_rad.is_finite());
        assert!(snapshot.angular_velocity_rad_s.is_finite());
        assert!(snapshot.speed_rpm.is_finite());
        assert!(snapshot.electrical_angle_rad.is_finite());
        assert!(snapshot.phase_currents_a.iter().sum::<f64>().abs() < 1e-11);
    }
}

#[test]
fn identical_bldc_command_traces_are_bit_exact() {
    let mut first = BldcMotor::new(bldc_params(0.001)).unwrap();
    let mut second = BldcMotor::new(bldc_params(0.001)).unwrap();

    for step in 0..100_000 {
        let command = match step % 13 {
            0 => InverterCommand::off(),
            1 => InverterCommand::brake(),
            _ => InverterCommand::six_step(first.snapshot().commutation_sector).unwrap(),
        };
        first.step(command, DT_S).unwrap();
        second.step(command, DT_S).unwrap();
    }

    assert_eq!(first.params(), second.params());
    assert_eq!(first.snapshot(), second.snapshot());
}

/// The catalog `dc_motor` part's default parameters, read from the part pack
/// itself so this test follows whatever the catalog ships.
fn catalog_dc_motor_params() -> BrushedMotorParams {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/devices/dc_motor.yaml");
    let pack: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let default = |key: &str| {
        pack["behavior"]["params"][key]["default"]
            .as_f64()
            .unwrap_or_else(|| panic!("dc_motor.yaml has a numeric default for {key}"))
    };
    BrushedMotorParams {
        resistance_ohm: default("resistance_ohm"),
        inductance_h: default("inductance_h"),
        torque_constant_nm_per_a: default("torque_constant_nm_per_a"),
        back_emf_constant_v_per_rad_s: default("back_emf_constant_v_per_rad_s"),
        supply_voltage_v: default("supply_voltage_v"),
        shaft: ShaftParams {
            inertia_kg_m2: default("rotor_inertia_kg_m2"),
            viscous_friction_nm_per_rad_s: default("viscous_friction_nm_per_rad_s"),
            coulomb_friction_nm: default("coulomb_friction_nm"),
            load_torque_nm: default("load_torque_nm"),
        },
    }
}

#[test]
fn the_catalog_dc_motor_coasts_to_a_stop_within_a_second() {
    let params = catalog_dc_motor_params();
    assert!(params.shaft.coulomb_friction_nm > 0.0);
    let mut motor = BrushedDcMotor::new(params).unwrap();
    // 0.3 s at full supply: well past the electromechanical time constant.
    step_many(&mut motor, HBridgeCommand::forward(1.0).unwrap(), 30_000);
    let running = motor.snapshot();
    assert!(running.speed_rpm > 1000.0, "{} rpm", running.speed_rpm);
    // The dry friction costs a small running current, not the speed.
    let no_load_rpm = params.supply_voltage_v / params.back_emf_constant_v_per_rad_s * 60.0
        / std::f64::consts::TAU;
    assert!(
        running.speed_rpm > 0.95 * no_load_rpm,
        "{} rpm",
        running.speed_rpm
    );
    assert!(
        running.current_a > 0.0 && running.current_a < 0.2,
        "{} A",
        running.current_a
    );

    let coast = HBridgeCommand::coast();
    let mut steps = 0usize;
    while motor.snapshot().speed_rpm != 0.0 {
        motor.step(coast, DT_S).unwrap();
        assert!(motor.snapshot().speed_rpm >= 0.0, "never reverses");
        steps += 1;
        assert!(steps <= 100_000, "stopped within 1 s of coasting");
    }
    assert!(
        steps > 10_000,
        "a real spin-down, not an instant stop: {steps} steps"
    );
    step_many(&mut motor, coast, 10_000);
    assert_eq!(motor.snapshot().speed_rpm, 0.0, "stiction holds it");
}
