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
