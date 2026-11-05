// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::step_dir_motor::{
    StepDirMotor, StepDirTimingConstraint as Constraint, StepDirTimingStatus as Status,
};
use labwired_core::Bus;
use std::sync::Arc;

fn motor(device_type: &str, clock_hz: u64) -> StepDirMotor {
    StepDirMotor::new("axis", 2, 3, Some(4))
        .with_device_type(device_type)
        .with_simulation_clock_hz(clock_hz)
}

#[test]
fn legacy_counter_reports_unconfigured_timing_without_guessing_a_clock() {
    let motor = motor("a4988", 0);
    motor.on_gpio_edge(3, true, 0);
    motor.on_gpio_edge(2, true, 1);
    motor.on_gpio_edge(2, false, 2);
    let report = motor.model_report(2);
    assert_eq!(motor.steps(), 1);
    assert_eq!(report.motion_model, "ideal_commanded_position");
    assert_eq!(report.timing.status, Status::Unconfigured);
    assert_eq!(report.timing.simulation_clock_hz, None);
    assert_eq!(report.timing.limits, None);
    assert_eq!(report.timing.checked_rising_edges, 0);
    assert!(report.timing.violations.is_empty());
    assert!(report.unmodeled.contains(&"lost_steps"));
    assert!(report.unmodeled.contains(&"winding_current"));
}

#[test]
fn aliases_have_distinct_datasheet_timing_limits() {
    // A 1 GHz simulation clock makes one cycle one nanosecond.
    for (device_type, profile, pulse, direction, period) in [
        ("a4988", "a4988", 1_000, 200, 2_000),
        ("nema17", "a4988", 1_000, 200, 2_000),
        ("drv8825", "drv8825", 1_900, 650, 4_000),
        ("tmc2209", "tmc2209", 104, 20, 167),
        ("tmc2226", "tmc2226", 104, 20, 167),
    ] {
        let motor = motor(device_type, 1_000_000_000);
        let report = motor.model_report(0).timing;
        let limits = report.limits.unwrap();
        assert_eq!(report.profile, profile);
        assert_eq!(limits.step_high_min_cycles, pulse);
        assert_eq!(limits.step_low_min_cycles, pulse);
        assert_eq!(limits.dir_setup_min_cycles, direction);
        assert_eq!(limits.dir_hold_min_cycles, direction);
        assert_eq!(limits.step_period_min_cycles, period);
        assert_eq!(report.status, Status::NotObserved);
    }
}

#[test]
fn exact_minima_pass_at_multiple_simulation_frequencies() {
    for device_type in ["a4988", "drv8825", "tmc2209", "tmc2226"] {
        for clock_hz in [1_500_001, 168_000_000, 1_000_000_000] {
            let motor = motor(device_type, clock_hz);
            let limits = motor.model_report(0).timing.limits.unwrap();
            let rise = limits.step_period_min_cycles * 2;
            motor.on_gpio_edge(3, true, 0);
            motor.on_gpio_edge(2, true, rise);
            assert_eq!(motor.model_report(rise).timing.status, Status::Pending);
            motor.on_gpio_edge(3, false, rise + limits.dir_hold_min_cycles);
            motor.on_gpio_edge(2, false, rise + limits.step_high_min_cycles);
            let second_rise = rise
                + limits
                    .step_period_min_cycles
                    .max(limits.step_high_min_cycles + limits.step_low_min_cycles);
            motor.on_gpio_edge(2, true, second_rise);
            let end = second_rise + limits.step_high_min_cycles;
            motor.on_gpio_edge(2, false, end);
            let report = motor.model_report(end).timing;
            assert_eq!(motor.steps(), 0, "{device_type} at {clock_hz} Hz");
            assert_eq!(report.checked_rising_edges, 2);
            assert_eq!(report.completed_pulses, 2);
            assert_eq!(report.status, Status::WithinLimits, "{report:?}");
            assert!(report.violations.is_empty());
        }
    }
}

#[test]
fn every_alias_detects_short_step_high_low_and_direction_intervals() {
    for device_type in ["a4988", "drv8825", "tmc2209", "tmc2226"] {
        for constraint in [
            Constraint::StepHigh,
            Constraint::StepLow,
            Constraint::DirSetup,
            Constraint::DirHold,
        ] {
            let motor = motor(device_type, 1_000_000_000);
            let limits = motor.model_report(0).timing.limits.unwrap();
            let rise = 10_000;
            let dir_cycle = if constraint == Constraint::DirSetup {
                rise - limits.dir_setup_min_cycles + 1
            } else {
                0
            };
            motor.on_gpio_edge(3, true, dir_cycle);
            motor.on_gpio_edge(2, true, rise);
            match constraint {
                Constraint::StepHigh => {
                    motor.on_gpio_edge(2, false, rise + limits.step_high_min_cycles - 1);
                }
                Constraint::StepLow => {
                    let fall = rise + limits.step_high_min_cycles;
                    motor.on_gpio_edge(2, false, fall);
                    motor.on_gpio_edge(2, true, fall + limits.step_low_min_cycles - 1);
                }
                Constraint::DirHold => {
                    motor.on_gpio_edge(3, false, rise + limits.dir_hold_min_cycles - 1);
                }
                Constraint::DirSetup => {}
                _ => unreachable!(),
            }
            let report = motor.model_report(30_000).timing;
            assert_eq!(
                report.status,
                Status::Violated,
                "{device_type} {constraint:?}"
            );
            assert_eq!(report.violations.get(&constraint), Some(&1));
            // Diagnostics cannot predict the physical response to an invalid
            // pulse. The existing count remains an ideal commanded position.
            assert_eq!(motor.steps(), report.checked_rising_edges as i64);
        }
    }
}

#[test]
fn drv8825_frequency_limit_is_stricter_than_high_plus_low() {
    let motor = motor("drv8825", 1_000_000_000);
    motor.on_gpio_edge(3, true, 0);
    motor.on_gpio_edge(2, true, 10_000);
    motor.on_gpio_edge(2, false, 11_900);
    motor.on_gpio_edge(2, true, 13_800);
    let report = motor.model_report(20_000).timing;
    assert_eq!(report.violations, [(Constraint::StepPeriod, 1)].into());
    let violation = report.last_violation.unwrap();
    assert_eq!(violation.observed_cycles, 3_800);
    assert_eq!(violation.minimum_cycles, 4_000);
}

#[test]
fn tmc_driver_clock_is_separate_and_fractional_nanoseconds_remain_exact() {
    for device_type in ["tmc2209", "tmc2226"] {
        for (driver_hz, expected_cycles) in [(4_000_000, 810), (12_000_000, 310), (16_000_000, 248)]
        {
            let motor = motor(device_type, 3_000_000_000)
                .with_driver_clock_hz(driver_hz)
                .unwrap();
            let limits = motor.model_report(0).timing.limits.unwrap();
            assert_eq!(limits.step_high_min_cycles, expected_cycles);
            motor.on_gpio_edge(3, true, 0);
            motor.on_gpio_edge(2, true, 10_000);
            motor.on_gpio_edge(2, false, 10_000 + expected_cycles);
            assert_eq!(
                motor.model_report(20_000).timing.status,
                Status::WithinLimits
            );
        }
    }
    for driver_hz in [0, 3_999_999, 16_000_001, u32::MAX] {
        assert!(motor("tmc2226", 168_000_000)
            .with_driver_clock_hz(driver_hz)
            .is_err());
    }
}

#[test]
fn duplicate_pad_samples_preserve_transition_times_and_step_counts() {
    let motor = motor("a4988", 1_000_000_000);
    motor.on_gpio_edge(3, true, 0);
    motor.on_gpio_edge(3, true, 9_999);
    motor.on_gpio_edge(2, false, 9_999);
    motor.on_gpio_edge(2, true, 10_000);
    motor.on_gpio_edge(2, true, 10_999);
    motor.on_gpio_edge(2, false, 11_000);
    let report = motor.model_report(11_000).timing;
    assert_eq!(report.checked_rising_edges, 1);
    assert_eq!(report.completed_pulses, 1);
    assert_eq!(report.status, Status::WithinLimits);
    assert_eq!(motor.steps(), 1);
}

#[test]
fn long_runs_and_regressing_timestamps_do_not_wrap_interval_checks() {
    let motor = motor("tmc2226", 1_000_000_000);
    let rise = u64::MAX - 10_000;
    motor.on_gpio_edge(3, true, rise - 20);
    motor.on_gpio_edge(2, true, rise);
    motor.on_gpio_edge(2, false, rise + 104);
    assert_eq!(
        motor.model_report(rise + 104).timing.status,
        Status::WithinLimits
    );
    motor.on_gpio_edge(4, true, rise + 103);
    let report = motor.model_report(rise + 104).timing;
    assert_eq!(report.violations, [(Constraint::MonotonicCycle, 1)].into());
    assert_eq!(report.last_violation.unwrap().observed_cycles, -1);
}

#[test]
fn disabled_input_pulses_are_checked_without_commanding_motion() {
    let motor = motor("a4988", 1_000_000_000);
    motor.on_gpio_edge(4, true, 0);
    motor.on_gpio_edge(2, true, 10_000);
    motor.on_gpio_edge(2, false, 10_001);
    assert_eq!(motor.steps(), 0);
    assert_eq!(motor.model_report(20_000).timing.checked_rising_edges, 1);
    assert_eq!(
        motor
            .model_report(20_000)
            .timing
            .violations
            .get(&Constraint::StepHigh),
        Some(&1)
    );
}

#[test]
fn native_gpio_observer_uses_bus_cycles_when_peripheral_ticks_are_decimated() {
    use labwired_core::peripherals::esp32::gpio::Esp32Gpio;
    use labwired_core::peripherals::kit::{registry, AttachCtx};

    let mut bus = SystemBus::new();
    bus.cpu_hz = 1_000_000_000;
    bus.config.peripheral_tick_interval = 512;
    bus.add_peripheral(
        "gpio",
        0x3ff4_4000,
        0x1000,
        None,
        Box::new(Esp32Gpio::new()),
    );
    let ext = serde_yaml::from_str(
        r#"
id: axis
type: a4988
connection: gpio
config:
  step_pin: GPIO2
  dir_pin: GPIO3
  en_pin: GPIO4
"#,
    )
    .unwrap();
    registry::lookup("a4988")
        .unwrap()
        .attach(&mut AttachCtx::new(&mut bus, &ext))
        .unwrap();
    bus.write_u32(0x3ff4_4008, 1 << 3).unwrap();
    // No peripheral tick occurs between these GPIO writes. The legacy GPIO
    // callback stamp stays at zero, while the engine timebase advances.
    bus.set_current_cycle(10_000);
    bus.write_u32(0x3ff4_4008, 1 << 2).unwrap();
    bus.set_current_cycle(11_000);
    bus.write_u32(0x3ff4_400c, 1 << 2).unwrap();
    let snapshot = bus.motor_snapshots().remove(0);
    assert!(snapshot.faults.is_empty());
    let report = snapshot.step_dir.unwrap();
    assert_eq!(report.commanded_steps, 1);
    assert_eq!(report.timing.status, Status::WithinLimits);
    assert_eq!(report.timing.completed_pulses, 1);
    assert!(report.timing.violations.is_empty());
}

fn stm32_fixture() -> (
    labwired_config::ChipDescriptor,
    labwired_config::SystemManifest,
) {
    let chip = labwired_config::ChipDescriptor::from_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f407.yaml"),
    )
    .unwrap();
    let manifest = serde_yaml::from_str(
        r#"
name: step-dir-timing
chip: stm32f407
external_devices:
  - id: axis
    type: tmc2226
    connection: gpioc
    config:
      step_pin: PC4
      dir_pin: PA4
      en_pin: PB0
"#,
    )
    .unwrap();
    (chip, manifest)
}

#[test]
fn stm32_cross_port_binding_preserves_declared_clock_and_exports_timing_faults() {
    let (chip, manifest) = stm32_fixture();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    bus.write_u32(0x4002_3830, 0x07).unwrap();
    bus.write_u32(0x4002_0000, 1 << 8).unwrap();
    bus.write_u32(0x4002_0400, 1).unwrap();
    bus.write_u32(0x4002_0800, 1 << 8).unwrap();
    bus.set_current_cycle(1_000);
    bus.write_u32(0x4002_0018, 1 << 4).unwrap();
    bus.write_u32(0x4002_0818, 1 << 4).unwrap();
    bus.set_current_cycle(1_001);
    bus.write_u32(0x4002_0818, 1 << 20).unwrap();
    let snapshot = bus.motor_snapshots().remove(0);
    assert_eq!(snapshot.faults, ["step-dir-timing"]);
    let report = snapshot.step_dir.as_ref().unwrap();
    assert_eq!(report.commanded_steps, 1);
    assert_eq!(report.timing.simulation_clock_hz, Some(168_000_000));
    assert_eq!(report.timing.driver_clock_hz, Some(12_000_000));
    assert_eq!(report.timing.limits.unwrap().step_high_min_cycles, 18);
    assert_eq!(
        report.timing.violations.get(&Constraint::DirSetup),
        Some(&1)
    );
    assert_eq!(
        report.timing.violations.get(&Constraint::StepHigh),
        Some(&1)
    );
    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(json["step_dir"]["timing"]["status"], "violated");
    assert_eq!(json["step_dir"]["timing"]["violations"]["step_high"], 1);
    assert_eq!(json["step_dir"]["motion_model"], "ideal_commanded_position");
}

#[test]
fn kit_clock_overrides_are_honored_and_invalid_values_fail_attachment() {
    let (chip, mut manifest) = stm32_fixture();
    manifest.external_devices[0]
        .config
        .insert("simulation_clock_hz".into(), 1_000_000_000u64.into());
    manifest.external_devices[0]
        .config
        .insert("driver_clock_hz".into(), 16_000_000u64.into());
    let bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let snapshot = bus.motor_snapshots().remove(0);
    let report = snapshot.step_dir.unwrap().timing;
    assert_eq!(report.simulation_clock_hz, Some(1_000_000_000));
    assert_eq!(report.driver_clock_hz, Some(16_000_000));
    assert_eq!(report.limits.unwrap().step_high_min_cycles, 83);
    for (key, invalid) in [
        ("simulation_clock_hz", 0.into()),
        ("simulation_clock_hz", (-1).into()),
        ("simulation_clock_hz", "fast".into()),
        ("driver_clock_hz", 0.into()),
        ("driver_clock_hz", 16_000_001.into()),
    ] {
        let mut invalid_manifest = manifest.clone();
        invalid_manifest.external_devices[0]
            .config
            .insert(key.into(), invalid);
        assert!(
            SystemBus::from_config(&chip, &invalid_manifest).is_err(),
            "{key}"
        );
    }
}

#[test]
fn timing_faults_latch_into_native_snapshots_after_later_valid_motion() {
    let mut bus = SystemBus::new();
    let motor = Arc::new(motor("a4988", 1_000_000_000));
    motor.on_gpio_edge(3, true, 0);
    motor.on_gpio_edge(2, true, 10_000);
    motor.on_gpio_edge(2, false, 10_001);
    motor.on_gpio_edge(2, true, 20_000);
    motor.on_gpio_edge(2, false, 21_000);
    bus.observe_device(motor);
    bus.set_current_cycle(30_000);
    let snapshot = bus.motor_snapshots().remove(0);
    assert_eq!(snapshot.faults, ["step-dir-timing"]);
    assert_eq!(snapshot.step_dir.unwrap().timing.status, Status::Violated);
}
