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
        compare_ticks: (duty * 1000.0) as u64,
        transition: None,
        idle: None,
        inactive_when_disabled: [None; 2],
        mode: TimerChannelOutputMode::Pwm1,
    }
}

fn pwm(duty: f64, dead_time_ticks: u16) -> TimerOutputSnapshot {
    TimerOutputSnapshot {
        channels: [channel(duty), channel(0.5), channel(0.5), channel(0.0)],
        dead_time_ticks,
        dead_time_cycles: u64::from(dead_time_ticks),
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
    let mut phase_b = channel(1.0);
    phase_b.enabled = false;
    let mut phase_c = channel(0.0);
    phase_c.enabled = false;
    phase_c.complementary_enabled = false;
    let snapshot = TimerOutputSnapshot {
        channels: [channel(duty), phase_b, phase_c, channel(0.0)],
        dead_time_ticks,
        dead_time_cycles: u64::from(dead_time_ticks),
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
    // Native CCR is integral: 20 counts × PSC divisor2 represents the same
    // 40-cycle period and quarter-duty edge at cycle10 exactly.
    snapshot.period_ticks = 20;
    snapshot.prescaler_divisor = 2;
    for channel in &mut snapshot.channels {
        channel.compare_ticks = (channel.duty_fraction * 20.0) as u64;
    }
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
    // Native CCR is integral: 20 counts × PSC divisor2 represents the same
    // 40-cycle period and quarter-duty edge at cycle10 exactly.
    snapshot.period_ticks = 20;
    snapshot.prescaler_divisor = 2;
    for channel in &mut snapshot.channels {
        channel.compare_ticks = (channel.duty_fraction * 20.0) as u64;
    }
    snapshot.counter_ticks = 3;
    snapshot.prescaler_phase = 1;
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
    for_each_pwm_segment(snapshot, 0, 1_000_000.0, |_, duration| {
        segments += 1;
        elapsed += duration;
    });
    // Period wraps do not split a physically constant gate command.
    assert_eq!(segments, 1);
    assert_eq!(elapsed, 1_000_000.0);
    assert_eq!(pwm_cycle_edges(snapshot).capacity(), 14);
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
    let pair = pwm(0.75, 100);
    assert!(!sampled_gate_pair(pair, 0, 10, 10).high);
    assert!(!sampled_gate_pair(pair, 0, 800, 800).low);
    assert!(sampled_gate_pair(pair, 0, 990, 990).low);
}

#[test]
fn sampled_gate_pair_honors_complementary_polarity() {
    let mut output = channel(0.25);
    output.complementary_active_low = true;
    let mut snapshot = pwm(0.25, 0);
    snapshot.channels[0] = output;
    let before_edge = sampled_gate_pair(snapshot, 0, 100, 100);
    let after_edge = sampled_gate_pair(snapshot, 0, 900, 900);
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
    let mut bus =
        SystemBus::from_config(&chip, &manifest).expect("terminal-driven motor must construct");
    bus.write_u32(0x4800_0000, 0x05).unwrap(); // PA0/PA1 must actually drive the inputs
    bus
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

#[test]
fn step_dir_folded_into_motor_snapshots() {
    use crate::peripherals::components::step_dir_motor::StepDirMotor;
    use std::sync::Arc;

    let mut bus = SystemBus::new();
    let motor = Arc::new(StepDirMotor::new("nema17", 3, 4, Some(8)).with_device_type("nema17"));
    // EN low, DIR high, 50 full steps → 90 deg
    motor.on_gpio_edge(8, false, 0);
    motor.on_gpio_edge(4, true, 1);
    for i in 0..50u64 {
        motor.on_gpio_edge(3, true, 2 + i * 2);
        motor.on_gpio_edge(3, false, 3 + i * 2);
    }
    bus.observe_device(motor.clone());

    let snaps = bus.motor_snapshots();
    let step = snaps
        .iter()
        .find(|s| s.id == "nema17")
        .expect("nema17 snapshot");
    assert_eq!(step.kind, STEPPER_SNAPSHOT_KIND);
    assert!((step.position_rad.to_degrees() - 90.0).abs() < 1e-9);
    assert_eq!(step.control_state, "step");

    // Disable EN → idle, angle holds
    motor.on_gpio_edge(8, true, 200);
    let snaps = bus.motor_snapshots();
    let step = snaps.iter().find(|s| s.id == "nema17").unwrap();
    assert_eq!(step.control_state, "idle");
    assert!((step.position_rad.to_degrees() - 90.0).abs() < 1e-9);
}

fn low_inertia_bldc_bus() -> SystemBus {
    low_inertia_bldc_bus_with_rcc(false)
}

fn low_inertia_bldc_bus_with_rcc(with_rcc: bool) -> SystemBus {
    let mut chip: labwired_config::ChipDescriptor = serde_yaml::from_str(
        r#"
name: bldc-service-test
arch: arm
core: cortex-m4
cpu_hz: 4000000
flash: { base: 0x08000000, size: "64KB" }
ram: { base: 0x20000000, size: "32KB" }
peripherals:
  - { id: gpioa, type: gpio, base_address: 0x48000000, size: "1KB", config: { profile: stm32v2 } }
  - { id: gpiob, type: gpio, base_address: 0x48000400, size: "1KB", config: { profile: stm32v2 } }
  - { id: gpioc, type: gpio, base_address: 0x48000800, size: "1KB", config: { profile: stm32v2 } }
  - { id: tim1, type: timer, base_address: 0x40012c00, size: "1KB", config: { advanced: true } }
"#,
    )
    .unwrap();
    if with_rcc {
        chip.peripherals.push(serde_yaml::from_str(
            "id: rcc\ntype: rcc\nbase_address: 0x40021000\nsize: 1KB\nconfig: { profile: stm32l4 }\n"
        ).unwrap());
    }
    // The shipped low-inertia plant, at the board's 4 MHz reset clock.
    let manifest: SystemManifest = serde_yaml::from_str(
        &include_str!("../../../../examples/nucleo-l476rg-bldc/system.yaml").replace(
            "simulation_clock_hz: 80000000",
            "simulation_clock_hz: 4000000",
        ),
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    bus.write_u32(0x4800_0400, 1).unwrap(); // PB0 output, real enable pad.
    bus.write_u32(0x4800_0414, 1).unwrap();
    for (offset, value) in [
        (0x18, 0x6060),
        (0x1c, 0x60),
        (0x20, 0x555),
        (0x28, 0),
        (0x2c, 199),
        (0x34, 100),
        (0x38, 100),
        (0x3c, 100),
        (0x44, 1 << 15),
        (0x00, 1),
    ] {
        bus.write_u32(0x4001_2c00 + offset, value).unwrap();
    }
    let MotorRuntime::Bldc { plant, .. } = &mut bus.motors[0] else {
        panic!("BLDC fixture");
    };
    // Establish stored winding energy and motion through accepted drive steps.
    for _ in 0..1_000 {
        let sector = plant.snapshot().commutation_sector;
        plant
            .step(InverterCommand::six_step(sector).unwrap(), 1e-6)
            .unwrap();
    }
    bus
}

fn bldc_state(bus: &SystemBus) -> crate::physics::motor::BldcMotorSnapshot {
    let MotorRuntime::Bldc { plant, .. } = &bus.motors[0] else {
        panic!("BLDC fixture");
    };
    plant.snapshot()
}

fn bldc_control_test_bus(legacy_walk: bool) -> SystemBus {
    let mut bus = low_inertia_bldc_bus();
    if legacy_walk {
        let MotorRuntime::Bldc { timer, .. } = &bus.motors[0] else {
            unreachable!();
        };
        let timer = *timer;
        bus.peripherals[timer]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<crate::peripherals::timer::Timer>()
            .unwrap()
            .force_legacy_walk();
    }
    bus
}

fn advance_bldc_control_test_clock(bus: &mut SystemBus, cycle: u64, legacy_walk: bool) {
    if legacy_walk {
        let MotorRuntime::Bldc { timer, .. } = &bus.motors[0] else {
            unreachable!();
        };
        let timer = *timer;
        // Drive the normal timer walk without adding a motor service boundary.
        // The paired scheduler case obtains the same counter from its clock.
        for _ in bus.current_cycle..cycle {
            bus.peripherals[timer].dev.tick();
        }
    }
    bus.set_current_cycle(cycle);
}

#[test]
fn bldc_service_off_paths_preserve_coasting_at_four_mhz() {
    for off in [
        "external-enable",
        "moe",
        "timer-stopped",
        "outputs-disabled",
        "no-timer",
    ] {
        let mut bus = low_inertia_bldc_bus();
        match off {
            "external-enable" => bus.write_u32(0x4800_0414, 0).unwrap(),
            "moe" => bus.write_u32(0x4001_2c44, 0).unwrap(),
            "timer-stopped" => {
                // CEN alone stops the counter; it does not disable a held
                // output gate. This fixture explicitly releases every gate.
                bus.write_u32(0x4001_2c20, 0).unwrap();
                bus.write_u32(0x4001_2c00, 0).unwrap();
            }
            "outputs-disabled" => bus.write_u32(0x4001_2c20, 0).unwrap(),
            "no-timer" => {
                let MotorRuntime::Bldc { timer, .. } = &mut bus.motors[0] else {
                    unreachable!();
                };
                *timer = 0; // A non-timer peripheral exercises the missing-PWM branch.
            }
            _ => unreachable!(),
        }
        let before = bldc_state(&bus);
        assert!(before.speed_rpm > 1.0, "{before:?}");
        assert!(before.phase_currents_a.iter().any(|v| v.abs() > 1e-3));
        let stored_energy = |state: &crate::physics::motor::BldcMotorSnapshot| {
            0.5e-7 * state.angular_velocity_rad_s.powi(2)
                + 0.5 * 0.00018 * state.phase_currents_a.iter().map(|i| i * i).sum::<f64>()
        };
        let initial_energy = stored_energy(&before);

        // Actual browser service interval: 1024 cycles / 4 MHz = 256 us.
        // The old electrical-only cap produces two rejected 128 us steps.
        bus.set_current_cycle(1024);
        bus.service_motor_models();
        let after = bldc_state(&bus);
        assert!(
            after.position_rad > before.position_rad,
            "{off}: plant froze"
        );
        let decay = (-256e-6_f64 * 0.35 / 0.00018).exp();
        for (initial, final_current) in before
            .phase_currents_a
            .into_iter()
            .zip(after.phase_currents_a)
        {
            let expected = initial * decay;
            assert!(
                (final_current - expected).abs() < 1e-10,
                "{off}: {final_current} vs {expected}"
            );
        }
        assert_eq!(after.dc_bus_current_a, 0.0);
        assert_eq!(bus.motor_snapshots()[0].control_state, format!("off:{off}"));
        assert_eq!(bus.motor_series().1, 256_000);

        // Residual winding current can change the shaft's direction. The
        // reduced freewheel model must not freeze or gain more stored energy
        // than it started with; once current decays, viscous coast is analytic.
        let mut viscous_start = None;
        for boundary in 2..=400 {
            bus.set_current_cycle(boundary * 1024);
            bus.service_motor_models();
            assert!(stored_energy(&bldc_state(&bus)) <= initial_energy * (1.0 + 1e-12));
            if boundary == 80 {
                viscous_start = Some(bldc_state(&bus));
            }
        }
        let coasted = bldc_state(&bus);
        assert!(
            coasted.position_rad.is_finite() && coasted.position_rad != after.position_rad,
            "{off}"
        );
        assert!(
            coasted.speed_rpm.abs() > 0.0 && coasted.speed_rpm.abs() < after.speed_rpm.abs(),
            "{off}: {coasted:?}"
        );
        assert!(coasted.phase_currents_a.iter().all(|v| v.abs() < 1e-10));
        assert_eq!(bus.motor_series().1, 102_400_000);
        let viscous_start = viscous_start.unwrap();
        assert!(viscous_start
            .phase_currents_a
            .iter()
            .all(|v| v.abs() < 1e-12));
        let duration = 320.0_f64 * 256e-6;
        let drag_rate = 0.000003 / 1e-7;
        let decay = (-drag_rate * duration).exp();
        let expected_speed = viscous_start.speed_rpm * decay;
        let expected_distance = viscous_start.angular_velocity_rad_s / drag_rate * (1.0 - decay);
        assert!(
            (coasted.speed_rpm / expected_speed - 1.0).abs() < 1e-3,
            "{off}"
        );
        assert!(
            ((coasted.position_rad - viscous_start.position_rad) / expected_distance - 1.0).abs()
                < 1e-3,
            "{off}"
        );
    }
}

#[test]
fn bldc_service_active_pwm_converges_with_finer_service_intervals() {
    let run = |interval: u64| {
        let mut bus = low_inertia_bldc_bus();
        let mut cycle = 0;
        while cycle < 1024 {
            cycle = (cycle + interval).min(1024);
            bus.set_current_cycle(cycle);
            bus.service_motor_models();
        }
        assert_eq!(bus.motor_snapshots()[0].control_state, "inverter");
        assert_eq!(bus.motor_series().1, 256_000);
        bldc_state(&bus)
    };
    let coarse = run(1024);
    let medium = run(2);
    let fine = run(1);
    let error = |state: &crate::physics::motor::BldcMotorSnapshot| {
        state
            .phase_currents_a
            .iter()
            .zip(fine.phase_currents_a)
            .map(|(a, b)| (a - b).abs())
            .sum::<f64>()
            / (1.0 + fine.phase_currents_a.iter().map(|v| v.abs()).sum::<f64>())
            + (state.angular_velocity_rad_s - fine.angular_velocity_rad_s).abs()
                / (1.0 + fine.angular_velocity_rad_s.abs())
            + (state.position_rad - fine.position_rad).abs() / (1.0 + fine.position_rad.abs())
    };
    let coarse_error = error(&coarse);
    let medium_error = error(&medium);
    println!("BLDC active normalized error: service {coarse_error:e}, 0.5 us {medium_error:e}");
    assert!(coarse_error < 0.02, "{coarse_error}");
    assert!(
        medium_error < coarse_error,
        "{medium_error} >= {coarse_error}"
    );
}

#[test]
fn bldc_service_rejected_step_latches_fault_without_fresh_signal_horizon() {
    let mut bus = low_inertia_bldc_bus();
    let manifest: SystemManifest = serde_yaml::from_str(
        &include_str!("../../../../examples/nucleo-l476rg-bldc/system.yaml").replace(
            "simulation_clock_hz: 80000000",
            "simulation_clock_hz: 4000000",
        ),
    )
    .unwrap();
    let MotorModelConfig::Bldc(mut healthy_config) =
        manifest.resolved_motor_models().unwrap().remove(0)
    else {
        unreachable!();
    };
    healthy_config.id = "healthy".to_owned();
    bus.motors
        .push(bus.build_bldc_motor(*healthy_config).unwrap());
    bus.set_current_cycle(4);
    bus.service_motor_models();
    assert_eq!(bus.motor_series().1, 1000);
    let live_before = bus.live_numeric_capture(0, 20).unwrap();
    assert_eq!(live_before.channels.len(), 4);
    assert!(live_before.channels.iter().all(|channel| channel.valid));
    let MotorRuntime::Bldc { plant, .. } = &mut bus.motors[0] else {
        unreachable!();
    };
    // Valid finite parameters with an overflowing torque calculation. This
    // exercises a real step failure, after the timestep has been admitted.
    let mut params = plant.params();
    params.resistance_ohm = 1.0;
    params.inductance_h = 1.0;
    params.torque_constant_nm_per_a = f64::MAX;
    params.back_emf_constant_v_per_rad_s = f64::MIN_POSITIVE;
    params.supply_voltage_v = 1e200;
    params.shaft.inertia_kg_m2 = f64::MAX;
    params.shaft.viscous_friction_nm_per_rad_s = 0.0;
    **plant = BldcMotor::new(params).unwrap();
    assert!(plant.service_timestep_s().is_ok());
    bus.write_u32(0x4001_2c20, 0x55).unwrap(); // Both pairs enabled: A high, B low, C floating.
    bus.write_u32(0x4001_2c34, 200).unwrap();
    bus.write_u32(0x4001_2c38, 0).unwrap();
    bus.set_current_cycle(1028);
    bus.service_motor_models();
    let failed = bus.motor_snapshots()[0].clone();
    assert_eq!(failed.control_state, "fault:integration");
    assert!(
        failed
            .faults
            .iter()
            .any(|f| f.contains("electromagnetic_torque_nm")),
        "{failed:?}"
    );
    assert_eq!(failed.current_a, None);
    assert_eq!(failed.phase_currents_a, None);
    assert_eq!(bus.motor_speed_rpm("drive_motor"), None);
    let (channels, end) = bus.motor_series();
    assert_eq!(end, 257_000);
    assert_eq!(channels.len(), 2);
    assert!(channels
        .iter()
        .all(|c| c.name.starts_with("motor.healthy.")));
    let MotorRuntime::Bldc { series, .. } = &bus.motors[0] else {
        unreachable!();
    };
    assert_eq!(series.end_time_ns, 1000); // Valid history remains internal.
    let live_after = bus
        .live_numeric_capture(live_before.next_cursor, 20)
        .unwrap();
    for channel in &live_after.channels {
        if channel.name.starts_with("motor.drive_motor.") {
            assert!(!channel.valid);
            assert_eq!(channel.known_until_ns, Some(1000));
            assert!(channel
                .invalid_reason
                .as_ref()
                .unwrap()
                .contains("electromagnetic_torque_nm"));
        } else {
            assert!(channel.valid);
            assert_eq!(channel.known_until_ns, Some(257_000));
        }
    }
    assert!(live_after
        .points
        .iter()
        .all(|point| live_after.channels[point.channel]
            .name
            .starts_with("motor.healthy.")));

    bus.set_current_cycle(2052);
    bus.service_motor_models();
    assert_eq!(bus.motor_snapshots()[0], failed);
    let (channels, end) = bus.motor_series();
    assert_eq!(end, 513_000);
    assert!(channels
        .iter()
        .all(|c| c.name.starts_with("motor.healthy.")));
    let final_live = bus
        .live_numeric_capture(live_after.next_cursor, 20)
        .unwrap();
    assert!(final_live
        .channels
        .iter()
        .filter(|c| !c.valid)
        .all(|c| c.known_until_ns == Some(1000)));
    assert!(final_live
        .channels
        .iter()
        .filter(|c| c.valid)
        .all(|c| c.known_until_ns == Some(513_000)));
}

#[test]
fn motor_live_numeric_continues_after_the_historical_prefix_is_full() {
    let mut bus = low_inertia_bldc_bus();
    let initial = bus.live_numeric_capture(0, 20).unwrap();
    assert!(initial.points.is_empty());
    assert!(initial.channels.iter().all(|channel| !channel.valid
        && channel.known_from_ns.is_none()
        && channel.known_until_ns.is_none()));
    bus.set_current_cycle(4);
    bus.service_motor_models();
    let before = bus.live_numeric_capture(0, 20).unwrap();
    let MotorRuntime::Bldc { series, .. } = &mut bus.motors[0] else {
        unreachable!();
    };
    // Pre-fill only the historical fixture to its unchanged production cap.
    // The next observations below still come from real accepted plant service.
    let cap = crate::analog::DEFAULT_SERIES_POINTS;
    assert_eq!(cap, 65_536);
    series.speed.points = vec![(1000, 1e9); cap];
    series.current.points = vec![(1000, 1e9); cap];
    bus.set_current_cycle(1028);
    bus.service_motor_models();
    let state = bldc_state(&bus);
    let (history, end) = bus.motor_series();
    assert_eq!(end, 257_000);
    assert!(history.iter().all(|channel| channel.points.len() == cap
        && channel.points.last() == Some(&(1000, 1e9))
        && channel.dropped == 1));
    let live = bus.live_numeric_capture(before.next_cursor, 20).unwrap();
    assert_eq!(live.epoch, before.epoch);
    assert_eq!((live.reader_missed, live.omitted), (0, 0));
    assert_eq!(live.points.len(), 2);
    assert!(live
        .points
        .iter()
        .all(|point| point.time_ns == 257_000 && point.value.is_finite()));
    assert_eq!(live.points[0].value, state.speed_rpm);
    assert_eq!(live.points[1].value, state.dc_bus_current_a);
    assert!(live
        .channels
        .iter()
        .all(|channel| channel.valid && channel.known_until_ns == Some(257_000)));
}

#[test]
fn motor_live_numeric_rewind_discards_observations_without_changing_service_anchor() {
    let mut bus = low_inertia_bldc_bus();
    bus.set_current_cycle(100);
    bus.service_motor_models();
    let old = bus.live_numeric_capture(0, 20).unwrap();
    bus.set_current_cycle(50);
    bus.service_motor_models();
    let reset = bus.live_numeric_capture(old.next_cursor, 20).unwrap();
    assert_ne!(reset.epoch, old.epoch);
    assert!(reset.reset_required);
    assert!(reset.points.is_empty());
    assert!(reset.channels.iter().all(|channel| !channel.valid));
    assert_eq!(
        bus.motor_service_anchor(),
        100,
        "capture does not change integration scheduling"
    );
    bus.set_current_cycle(51);
    bus.service_motor_models();
    assert_eq!(bus.live_numeric_capture(0, 20).unwrap().epoch, reset.epoch);
}

#[test]
fn bldc_control_writes_close_the_old_interval_before_all_store_widths() {
    for legacy_walk in [false, true] {
        for width in [1, 2, 4] {
            for (address, expected) in [
                (0x4800_0414, "off:external-enable"),
                (
                    if width == 1 { 0x4001_2c45 } else { 0x4001_2c44 },
                    "off:moe",
                ),
            ] {
                let mut automatic = bldc_control_test_bus(legacy_walk);
                let mut explicit = bldc_control_test_bus(legacy_walk);
                let write = |bus: &mut SystemBus| match width {
                    1 => bus.write_u8(address, 0),
                    2 => bus.write_u16(address, 0),
                    4 => bus.write_u32(address, 0),
                    _ => unreachable!(),
                };
                advance_bldc_control_test_clock(&mut automatic, 333, legacy_walk);
                advance_bldc_control_test_clock(&mut explicit, 333, legacy_walk);
                explicit.service_motor_models();
                write(&mut automatic).unwrap();
                write(&mut explicit).unwrap();
                assert_eq!(
                bldc_state(&automatic),
                bldc_state(&explicit),
                "legacy={legacy_walk}, {width}-byte write to {address:#x} changed the past interval"
            );
                assert_eq!(automatic.motor_series().1, 83_250);
                assert_eq!(automatic.motor_snapshots()[0].control_state, "inverter");
                let at_write = bldc_state(&automatic);

                // Only the following interval observes the new disabled command.
                advance_bldc_control_test_clock(&mut automatic, 1357, legacy_walk);
                advance_bldc_control_test_clock(&mut explicit, 1357, legacy_walk);
                automatic.service_motor_models();
                explicit.service_motor_models();
                let after = bldc_state(&automatic);
                assert_eq!(after, bldc_state(&explicit));
                assert_eq!(automatic.motor_snapshots()[0].control_state, expected);
                let decay = (-256e-6_f64 * 0.35 / 0.00018).exp();
                for (before, after) in at_write
                    .phase_currents_a
                    .into_iter()
                    .zip(after.phase_currents_a)
                {
                    assert!((after - before * decay).abs() < 1e-10);
                }
            }
        }
    }
}

#[test]
fn bldc_phase_mapping_writes_start_at_the_write_cycle() {
    for legacy_walk in [false, true] {
        for (offset, value) in [(0x24, 117), (0x28, 3), (0x14, 1), (0x00, 1)] {
            let mut bus = bldc_control_test_bus(legacy_walk);
            // Distinct phase duties make a shifted PWM interval physically visible.
            bus.write_u32(0x4001_2c34, 41).unwrap();
            bus.write_u32(0x4001_2c38, 129).unwrap();
            bus.write_u32(0x4001_2c3c, 173).unwrap();
            if offset == 0x00 {
                bus.write_u32(0x4001_2c00, 0).unwrap();
            }
            advance_bldc_control_test_clock(&mut bus, 123, legacy_walk);
            bus.service_motor_models();
            bus.write_u32(0x4001_2c00 + offset, value).unwrap();
            let MotorRuntime::Bldc { plant, timer, .. } = &bus.motors[0] else {
                unreachable!();
            };
            let mut expected = plant.clone();
            // This snapshot is sampled at the actual register-write boundary,
            // before the lazy timer advances through the next 137 cycles.
            let at_write = bus.timer_output_at(*timer).unwrap();
            let cap = expected.service_timestep_s().unwrap();
            for_each_pwm_segment(at_write, 123, 137.0, |command, cycles| {
                for dt in stable_substeps(cycles / 4_000_000.0, cap) {
                    expected.step(command, dt).unwrap();
                }
            });
            advance_bldc_control_test_clock(&mut bus, 260, legacy_walk);
            bus.service_motor_models();
            assert_eq!(
                bldc_state(&bus),
                expected.snapshot(),
                "legacy={legacy_walk}, phase-mapping register {offset:#x}"
            );
        }
    }
}

#[test]
fn unrelated_gpio_and_ram_writes_do_not_service_motor_plants() {
    let mut bus = low_inertia_bldc_bus();
    let before = bldc_state(&bus);
    bus.set_current_cycle(333);
    bus.write_u32(0x4800_0814, 1).unwrap(); // GPIOC carries feedback, not drive controls.
    bus.write_u32(0x2000_0000, 0x1234_5678).unwrap();
    assert_eq!(bldc_state(&bus), before);
    assert_eq!(bus.motor_cycle_anchor, 0);
    assert_eq!(bus.motor_series().1, 0);
}

#[test]
fn dc_control_write_preserves_the_prior_drive_interval() {
    let config = "      pwm_pin: VCC\n      in1_pin: PA0\n      in2_pin: PA1\n";
    let mut automatic = terminal_bus(config);
    let mut explicit = terminal_bus(config);
    for bus in [&mut automatic, &mut explicit] {
        bus.write_u32(0x4800_0014, 0b01).unwrap();
        bus.set_current_cycle(333);
    }
    explicit.service_motor_models();
    automatic.write_u32(0x4800_0014, 0).unwrap();
    explicit.write_u32(0x4800_0014, 0).unwrap();
    assert_eq!(automatic.motor_snapshots(), explicit.motor_snapshots());
    assert!(automatic.motor_snapshots()[0].current_a.unwrap() > 0.0);
    assert_eq!(automatic.motor_snapshots()[0].control_state, "forward");
    automatic.set_current_cycle(1357);
    automatic.service_motor_models();
    assert_eq!(automatic.motor_snapshots()[0].control_state, "coast");
}

#[test]
fn rcc_timer_divider_write_closes_the_old_motor_interval() {
    let mut automatic = low_inertia_bldc_bus_with_rcc(true);
    let mut explicit = low_inertia_bldc_bus_with_rcc(true);
    automatic.set_current_cycle(333);
    explicit.set_current_cycle(333);
    explicit.service_motor_models();
    for bus in [&mut automatic, &mut explicit] {
        bus.write_u32(0x4002_1008, 5 << 11).unwrap(); // APB2 /4 => timer kernel /2.
    }
    assert_eq!(bldc_state(&automatic), bldc_state(&explicit));
    assert_eq!(automatic.motor_series().1, 83_250);
    let MotorRuntime::Bldc { timer, plant, .. } = &automatic.motors[0] else {
        unreachable!();
    };
    let at_write = automatic.timer_output_at(*timer).unwrap();
    assert_eq!(at_write.prescaler_divisor, 2);
    let mut expected = plant.clone();
    let cap = expected.service_timestep_s().unwrap();
    for_each_pwm_segment(at_write, 333, 137.0, |command, cycles| {
        for dt in stable_substeps(cycles / 4_000_000.0, cap) {
            expected.step(command, dt).unwrap();
        }
    });
    automatic.set_current_cycle(470);
    automatic.service_motor_models();
    assert_eq!(bldc_state(&automatic), expected.snapshot());
}

#[test]
fn host_motor_mutations_take_effect_after_the_prior_interval() {
    for input in ["load", "supply", "inverter", "stall"] {
        let mut automatic = low_inertia_bldc_bus();
        let mut explicit = low_inertia_bldc_bus();
        automatic.set_current_cycle(333);
        explicit.set_current_cycle(333);
        explicit.service_motor_models();
        for bus in [&mut automatic, &mut explicit] {
            match input {
                "load" => bus.set_motor_named_input("drive_motor", "load-torque-nm", 1e-5),
                "supply" => bus.set_motor_named_input("drive_motor", "supply-voltage-v", 12.0),
                "inverter" => bus.set_motor_named_fault("drive_motor", "inverter", true),
                "stall" => bus.set_motor_stalled("drive_motor", true),
                _ => unreachable!(),
            }
            .unwrap();
        }
        assert_eq!(
            automatic.motor_snapshots(),
            explicit.motor_snapshots(),
            "{input}"
        );
        assert_eq!(automatic.motor_series().1, 83_250);
        automatic.set_current_cycle(1357);
        explicit.set_current_cycle(1357);
        automatic.service_motor_models();
        explicit.service_motor_models();
        assert_eq!(
            automatic.motor_snapshots(),
            explicit.motor_snapshots(),
            "{input}"
        );
    }

    let mut rejected = low_inertia_bldc_bus();
    let before = rejected.motor_snapshots();
    rejected.set_current_cycle(333);
    assert!(rejected
        .set_motor_named_input("drive_motor", "supply-voltage-v", 0.0)
        .is_err());
    assert!(rejected
        .set_motor_named_fault("drive_motor", "unknown", true)
        .is_err());
    assert!(rejected
        .set_motor_named_fault("drive_motor", "overcurrent", false)
        .is_err());
    assert_eq!(rejected.motor_snapshots(), before);
    assert_eq!(rejected.motor_cycle_anchor, 0);
    assert_eq!(rejected.motor_series().1, 0);
}

#[test]
fn pwm_motor_segments_use_the_same_absolute_turn_on_deadline_as_native_pads() {
    let mut snapshot = pwm(0.01, 2);
    snapshot.counter_ticks = 20;
    snapshot.channels[0].transition = Some(crate::peripherals::timer::TimerOutputTransition {
        side: 1,
        active: false,
        until_cycle: 102,
    });
    let mut observed = Vec::new();
    for_each_pwm_segment(snapshot, 100, 6.0, |command, duration| {
        observed.push((command.phase_a, duration));
    });
    assert_eq!(
        observed,
        vec![
            (
                GatePair {
                    high: false,
                    low: false
                },
                2.0
            ),
            (
                GatePair {
                    high: false,
                    low: true
                },
                4.0
            ),
        ]
    );
    // A later service interval must not replay the already-expired deadline.
    snapshot.counter_ticks = 26;
    let mut later = Vec::new();
    for_each_pwm_segment(snapshot, 106, 6.0, |command, duration| {
        later.push((command.phase_a, duration));
    });
    assert_eq!(
        later,
        vec![(
            GatePair {
                high: false,
                low: true
            },
            6.0
        )]
    );
}

#[test]
fn idle_gate_segments_match_native_pads_at_the_dead_time_boundary() {
    use crate::peripherals::timer::Timer;
    use crate::Peripheral;
    let mut timer = Timer::new_with_layout(16, true);
    let lines = timer.output_lines_arc();
    for (offset, value) in [
        (0x2c, 199),
        (0x34, 80),
        (0x18, 6 << 4),
        (0x20, 5),
        (0x44, 0x8002),
        (0, 1),
    ] {
        timer.write_u32(offset, value).unwrap();
    }
    for _ in 0..20 {
        timer.tick();
    }
    timer.write_u32(4, 1 << 9).unwrap();
    timer.write_u32(0x44, 0x0402).unwrap();
    let mut segments = Vec::new();
    for_each_pwm_segment(timer.output_snapshot(), 20, 6.0, |command, duration| {
        segments.push((command.phase_a, duration));
    });
    assert_eq!(
        segments,
        vec![
            (
                GatePair {
                    high: false,
                    low: false
                },
                2.0
            ),
            (
                GatePair {
                    high: false,
                    low: true
                },
                4.0
            ),
        ]
    );
    for (gate, duration) in segments {
        for _ in 0..duration as u64 {
            assert_eq!((lines.level(0), lines.level(4)), (gate.high, gate.low));
            timer.tick();
        }
    }
}

#[test]
fn bldc_stopped_and_idle_timer_gates_drive_the_plant() {
    let driven = InverterCommand {
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
    for state in ["held", "idle", "external-off"] {
        let mut bus = low_inertia_bldc_bus();
        bus.write_u32(0x4001_2c20, 0x55).unwrap(); // A/B pairs enabled; C disconnected.
        bus.write_u32(0x4001_2c34, 100).unwrap();
        bus.write_u32(0x4001_2c38, 0).unwrap();
        if state == "held" {
            bus.write_u32(0x4001_2c00, 0).unwrap(); // Stop at A-high/B-low compare state.
        } else {
            bus.write_u32(0x4001_2c04, (1 << 8) | (1 << 11)).unwrap(); // OIS1/OIS2N.
            bus.write_u32(0x4001_2c44, 0x0402).unwrap(); // MOE=0, OSSI=1, two-cycle delay.
            bus.write_u32(0x4001_2c18, 0).unwrap(); // Idle levels do not need a PWM mode.
            if state == "external-off" {
                bus.write_u32(0x4800_0414, 0).unwrap();
            }
        }
        let MotorRuntime::Bldc { plant, timer, .. } = &bus.motors[0] else {
            unreachable!();
        };
        if state == "held" {
            let mut segments = Vec::new();
            for_each_pwm_segment(
                bus.timer_output_at(*timer).unwrap(),
                0,
                128.0,
                |command, duration| {
                    segments.push((command, duration));
                },
            );
            assert_eq!(segments, vec![(driven, 128.0)]);
        }
        let mut expected = plant.clone();
        let cap = expected.service_timestep_s().unwrap();
        let mut advance = |command, cycles| {
            for dt in stable_substeps(cycles / 4_000_000.0, cap) {
                expected.step(command, dt).unwrap();
            }
        };
        match state {
            "held" => advance(driven, 128.0),
            "idle" => {
                advance(InverterCommand::off(), 2.0);
                advance(driven, 126.0);
            }
            "external-off" => advance(InverterCommand::off(), 128.0),
            _ => unreachable!(),
        }
        bus.set_current_cycle(128);
        bus.service_motor_models();
        assert_eq!(bldc_state(&bus), expected.snapshot(), "{state}");
        assert_eq!(bus.motor_series().1, 32_000);
        let expected_control = match state {
            "held" => "inverter:held",
            "idle" => "inverter:idle",
            _ => "off:external-enable",
        };
        assert_eq!(bus.motor_snapshots()[0].control_state, expected_control);
    }
}

#[test]
fn bldc_unsupported_enabled_output_latches_an_explicit_model_failure() {
    let mut bus = low_inertia_bldc_bus();
    bus.write_u32(0x4001_2c18, 0).unwrap(); // Enabled OC1/OC2 request an unsupported mode.
    let before = bldc_state(&bus);
    bus.set_current_cycle(1024);
    bus.service_motor_models();
    assert_eq!(bldc_state(&bus), before);
    let failed = &bus.motor_snapshots()[0];
    assert_eq!(failed.control_state, "fault:integration");
    assert_eq!(failed.current_a, None);
    assert_eq!(failed.phase_currents_a, None);
    assert!(failed
        .faults
        .iter()
        .any(|fault| fault.contains("unsupported enabled timer output mode")));
    assert!(bus.motor_series().0.is_empty());
    assert_eq!(bus.motor_series().1, 0);
    assert_eq!(bus.motor_speed_rpm("drive_motor"), None);
}
