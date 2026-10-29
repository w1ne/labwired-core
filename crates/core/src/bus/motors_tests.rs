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
            coulomb_friction_nm: 0.0,
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
            coulomb_friction_nm: 0.0,
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

#[test]
fn terminal_drive_follows_the_h_bridge_truth_table() {
    // Separate PWM/enable pad: IN1/IN2 choose the state, the duty scales it.
    assert_eq!(
        terminal_drive(true, false, true, false, None, 0.7, false),
        (HBridgeState::Forward, 0.7)
    );
    assert_eq!(
        terminal_drive(true, false, false, true, None, 0.7, false),
        (HBridgeState::Reverse, 0.7)
    );
    assert_eq!(
        terminal_drive(true, false, true, true, None, 0.7, false).0,
        HBridgeState::Brake
    );
    assert_eq!(
        terminal_drive(true, false, false, false, None, 0.7, false).0,
        HBridgeState::Coast
    );
    // Enable low (TB6612 STBY) coasts whatever the inputs say; brake wins.
    assert_eq!(
        terminal_drive(false, false, true, false, None, 1.0, false).0,
        HBridgeState::Coast
    );
    assert_eq!(
        terminal_drive(true, true, true, false, None, 1.0, false).0,
        HBridgeState::Brake
    );
}

#[test]
fn terminal_drive_with_pwm_on_an_in_pin_averages_both_phases() {
    // Fast decay: PWM on IN1, IN2 low -> forward at the duty.
    assert_eq!(
        terminal_drive(true, false, false, false, Some(0), 0.3, false),
        (HBridgeState::Forward, 0.3)
    );
    // PWM on IN2, IN1 low -> reverse at the duty.
    assert_eq!(
        terminal_drive(true, false, false, false, Some(1), 0.3, false),
        (HBridgeState::Reverse, 0.3)
    );
    // Slow decay: PWM on IN2 while IN1 is high -> forward for the LOW
    // fraction of the period, brake for the rest.
    let (state, duty) = terminal_drive(true, false, true, false, Some(1), 0.25, false);
    assert_eq!(state, HBridgeState::Forward);
    assert!((duty - 0.75).abs() < 1e-12);
    // Fully low PWM on IN1 with IN2 low is coast, fully high is full forward.
    assert_eq!(
        terminal_drive(true, false, false, false, Some(0), 0.0, false).0,
        HBridgeState::Coast
    );
    assert_eq!(
        terminal_drive(true, false, false, false, Some(0), 1.0, false),
        (HBridgeState::Forward, 1.0)
    );
}

fn terminal_bus(config: &str) -> SystemBus {
    let chip: labwired_config::ChipDescriptor = serde_yaml::from_str(
        r#"
name: motor-terminal-test
arch: arm
core: cortex-m4
flash: { base: 0x08000000, size: "64KB" }
ram: { base: 0x20000000, size: "32KB" }
peripherals:
  - id: gpioa
    type: gpio
    base_address: 0x48000000
    size: "1KB"
    config: { profile: stm32v2 }
"#,
    )
    .unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        r#"
name: dc-motor-terminal
chip: unused
external_devices:
  - id: wheel
    type: dc-motor
    connection: gpio
    config:
      resistance_ohm: 1.0
      inductance_h: 0.001
      torque_constant_nm_per_a: 0.1
      back_emf_constant_v_per_rad_s: 0.1
      rotor_inertia_kg_m2: 0.0001
      viscous_friction_nm_per_rad_s: 0.001
      supply_voltage_v: 12.0
      load_torque_nm: 0.0
      encoder_cpr: 16
{config}
"#
    ))
    .unwrap();
    SystemBus::from_config(&chip, &manifest).expect("terminal-driven motor must construct")
}

fn run_cycles(bus: &mut SystemBus, cycles: u64) {
    let start = bus.motor_service_anchor();
    let mut now = start;
    while now < start + cycles {
        now += MOTOR_SERVICE_QUANTUM_CYCLES;
        bus.set_current_cycle(now);
        bus.service_motor_models();
    }
}

#[test]
fn terminal_driven_motor_reverses_and_records_both_extremes() {
    // L298N-style channel: PA0 = IN1, PA1 = IN2, ENA jumpered on (VCC).
    let mut bus = terminal_bus("      pwm_pin: VCC\n      in1_pin: PA0\n      in2_pin: PA1\n");
    const ODR: u64 = 0x4800_0014;

    bus.write_u32(ODR, 0b01).unwrap(); // IN1 high, IN2 low -> forward
    run_cycles(&mut bus, 8_000_000);
    let forward = bus.motor_snapshots()[0].clone();
    assert_eq!(forward.control_state, "forward");
    assert!(forward.speed_rpm > 100.0, "{forward:?}");
    assert_eq!(bus.motor_speed_rpm("wheel"), Some(forward.speed_rpm));

    bus.write_u32(ODR, 0b10).unwrap(); // IN1 low, IN2 high -> reverse
    run_cycles(&mut bus, 16_000_000);
    let reverse = bus.motor_snapshots()[0].clone();
    assert_eq!(reverse.control_state, "reverse");
    assert!(reverse.speed_rpm < -100.0, "{reverse:?}");
    assert!(reverse.speed_rpm_max >= forward.speed_rpm);
    assert!(reverse.speed_rpm_min <= reverse.speed_rpm);
    assert_eq!(
        reverse.speed_rpm_peak_abs,
        reverse.speed_rpm_max.max(-reverse.speed_rpm_min)
    );

    bus.write_u32(ODR, 0b11).unwrap(); // both high -> brake
    run_cycles(&mut bus, MOTOR_SERVICE_QUANTUM_CYCLES);
    assert_eq!(bus.motor_snapshots()[0].control_state, "brake");
    bus.write_u32(ODR, 0b00).unwrap(); // both low -> coast
    run_cycles(&mut bus, MOTOR_SERVICE_QUANTUM_CYCLES);
    assert_eq!(bus.motor_snapshots()[0].control_state, "coast");
    assert_eq!(bus.motor_speed_rpm("missing"), None);
}

#[test]
fn terminal_and_direction_forms_are_mutually_exclusive() {
    let mut manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: dc-motor-bad
chip: unused
external_devices:
  - id: wheel
    type: dc-motor
    connection: gpio
    config:
      resistance_ohm: 1.0
      inductance_h: 0.001
      torque_constant_nm_per_a: 0.1
      back_emf_constant_v_per_rad_s: 0.1
      rotor_inertia_kg_m2: 0.01
      viscous_friction_nm_per_rad_s: 0.001
      supply_voltage_v: 12.0
      load_torque_nm: 0.0
      encoder_cpr: 16
      pwm_pin: VCC
      direction_pin: PA2
      in1_pin: PA0
      in2_pin: PA1
"#,
    )
    .unwrap();
    let err = manifest.resolved_motor_models().unwrap_err().to_string();
    assert!(err.contains("mutually exclusive"), "{err}");

    let config = &mut manifest.external_devices[0].config;
    config.remove("direction_pin");
    config.remove("in2_pin");
    let err = manifest.resolved_motor_models().unwrap_err().to_string();
    assert!(err.contains("in1_pin and in2_pin"), "{err}");

    config_remove_all(&mut manifest);
    let err = manifest.resolved_motor_models().unwrap_err().to_string();
    assert!(err.contains("needs a direction source"), "{err}");
}

fn config_remove_all(manifest: &mut SystemManifest) {
    let config = &mut manifest.external_devices[0].config;
    for key in ["direction_pin", "in1_pin", "in2_pin"] {
        config.remove(key);
    }
}

#[test]
fn both_inputs_low_is_a_per_driver_property() {
    // TB6612FNG / DRV8833: IN1 = IN2 = L leaves the outputs high-impedance.
    assert_eq!(
        terminal_drive(true, false, false, false, None, 1.0, false).0,
        HBridgeState::Coast
    );
    // L298N: with EN high, C = D = L is a fast motor stop (brake).
    assert_eq!(
        terminal_drive(true, false, false, false, None, 1.0, true).0,
        HBridgeState::Brake
    );
    // Both: H/H brakes, and a low enable still coasts the L298N.
    assert_eq!(
        terminal_drive(true, false, true, true, None, 1.0, false).0,
        HBridgeState::Brake
    );
    assert_eq!(
        terminal_drive(false, false, false, false, None, 1.0, true).0,
        HBridgeState::Coast
    );
    // PWM on IN1 with IN2 low on an L298N-style bridge: off phase brakes,
    // on phase drives forward at the duty.
    assert_eq!(
        terminal_drive(true, false, false, false, Some(0), 0.4, true),
        (HBridgeState::Forward, 0.4)
    );
}

#[test]
fn l298n_stops_by_braking_and_tb6612_by_coasting() {
    const ODR: u64 = 0x4800_0014;
    for (both_low, expected) in [("brake", "brake"), ("coast", "coast")] {
        let mut bus = terminal_bus(&format!(
            "      pwm_pin: VCC\n      in1_pin: PA0\n      in2_pin: PA1\n      both_inputs_low: {both_low}\n"
        ));
        bus.write_u32(ODR, 0b01).unwrap();
        run_cycles(&mut bus, 4_000_000);
        let spinning = bus.motor_snapshots()[0].speed_rpm;
        assert!(spinning > 100.0, "{both_low}: {spinning}");
        bus.write_u32(ODR, 0b00).unwrap();
        run_cycles(&mut bus, MOTOR_SERVICE_QUANTUM_CYCLES);
        assert_eq!(bus.motor_snapshots()[0].control_state, expected);
    }
}

#[test]
fn both_inputs_low_needs_terminal_drive() {
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: dc-motor-bad
chip: unused
external_devices:
  - id: wheel
    type: dc-motor
    connection: gpio
    config:
      resistance_ohm: 1.0
      inductance_h: 0.001
      torque_constant_nm_per_a: 0.1
      back_emf_constant_v_per_rad_s: 0.1
      rotor_inertia_kg_m2: 0.01
      viscous_friction_nm_per_rad_s: 0.001
      supply_voltage_v: 12.0
      load_torque_nm: 0.0
      encoder_cpr: 16
      pwm_pin: VCC
      direction_pin: PA2
      both_inputs_low: brake
"#,
    )
    .unwrap();
    let err = manifest.resolved_motor_models().unwrap_err().to_string();
    assert!(err.contains("both_inputs_low"), "{err}");
}
