// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog devices reach the PIC18F4520 demo profile's two
//! simulation-only threshold inputs. The PIC18 port SFRs and ADC are not
//! connected to these endpoints, so this proves solving and profile routing
//! rather than firmware-visible PIC pin behavior.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use std::path::Path;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line for the declared 5 V source and diode.
fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 5.0;
    const LIMIT_OHMS: f64 = 1_000.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
    let current_at_junction_voltage =
        |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let current = current_at_junction_voltage(junction_v);
        (SUPPLY_V - junction_v - current * DIODE_RS_OHMS) / LIMIT_OHMS - current
    };
    let (mut low, mut high) = (0.0, 1.2);
    for _ in 0..100 {
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    let junction_v = (low + high) / 2.0;
    junction_v + current_at_junction_voltage(junction_v) * DIODE_RS_OHMS
}

fn voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

#[test]
fn pic18f4520_ntc_and_diode_netlists_reach_threshold_inputs() {
    let system_path = root("configs/systems/pic18f4520-demo.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load PIC18F4520 profile");
    assert_eq!(profile.name, "pic18f4520-demo");
    let analog =
        SystemManifest::from_file(root("examples/pic18f4520/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and Shockley diode models");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load PIC18F4520 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build PIC18F4520 bus");
    let input_bank = bus
        .find_peripheral_index_by_name("analog_inputs")
        .expect("chip profile declares two analog co-simulation endpoints");
    assert_eq!(
        bus.peripherals[input_bank].dev.read_gpio_input(0),
        Some(false)
    );
    assert_eq!(
        bus.peripherals[input_bank].dev.read_gpio_input(1),
        Some(false)
    );

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both generic analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both devices must bind to separate profile input endpoints"
    );

    const VIL_V: f64 = 1.5;
    const VIH_V: f64 = 3.5;
    let expected_diode_v = diode_load_line_voltage();
    assert!(
        expected_diode_v < VIL_V,
        "diode load line must be below assumed VIL"
    );
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // hold high inside the threshold band
        (80.0, false),
        (25.0, false), // hold low inside the threshold band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both generic analog netlists");
        assert_eq!(
            steps.len(),
            2,
            "both unrelated device models solve per boundary"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = voltage(&session, "board.gpio_in.analog_inputs.0");
        let expected_ntc_v = 5.0 * ntc_resistance_ohms(temperature_c)
            / (10_000.0 + ntc_resistance_ohms(temperature_c));
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        assert_eq!(
            bus.peripherals[input_bank].dev.read_gpio_input(0),
            Some(expected_ntc_level),
            "input 0 follows the solved NTC voltage with threshold hysteresis"
        );
        if temperature_c == -10.0 {
            assert!(ntc_v > VIH_V, "cold NTC voltage must exceed assumed VIH");
        } else if temperature_c == 80.0 {
            assert!(ntc_v < VIL_V, "hot NTC voltage must fall below assumed VIL");
        }

        let diode_v = voltage(&session, "board.gpio_in.analog_inputs.1");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(diode_v < VIL_V, "diode node remains below assumed VIL");
        assert_eq!(
            bus.peripherals[input_bank].dev.read_gpio_input(1),
            Some(false),
            "input 1 reads low from the solved diode voltage"
        );
    }
}
