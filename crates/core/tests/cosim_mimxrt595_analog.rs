// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices resolve to the MIMXRT595-EVK's modeled SW2/SW1 GPIO
//! pads and are asserted through LPC GPIO PIN. The voltage thresholds below
//! are test assumptions only: no MIMXRT595 silicon threshold or ADC claim.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, GpioInputThresholds, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const GPIO_PIN: u64 = 0x4010_2100;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 3.3;
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
fn mimxrt595_evk_ntc_and_diode_netlists_reach_gpio_pin_register() {
    let system_path = root("configs/systems/mimxrt595-evk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load MIMXRT595-EVK profile");
    assert_eq!(profile.name, "mimxrt595-evk");
    assert!(profile.external_devices.is_empty());

    let analog_path = root("examples/mimxrt595-evk/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path)
        .expect("load generic NTC divider and Shockley diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("load MIMXRT595S descriptor");
    // Public Part Knowledge has no verified MIMXRT595 threshold record. Keep
    // voltage-to-level routing testable with explicit simulation-only ratios.
    chip.io_voltage_v = Some(3.3);
    chip.gpio_input_thresholds = Some(GpioInputThresholds { vil: 0.3, vih: 0.7 });

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build MIMXRT595-EVK bus");
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "P0_10"),
        Some((GPIO_PIN, 10)),
        "SW2 must resolve to port 0's PIN register"
    );
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "P0_25"),
        Some((GPIO_PIN, 25)),
        "SW1 must resolve to port 0's PIN register"
    );

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both unrelated analog models must bind to distinct physical GPIO pad mappings"
    );

    let expected_diode_v = diode_load_line_voltage();
    assert!(expected_diode_v < 0.99, "diode should fall below test VIL");
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // retain the cold high level inside the assumed band
        (80.0, false),
        (25.0, false), // retain the hot low level inside the assumed band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both generic analog netlists");
        assert_eq!(steps.len(), 2, "both unrelated netlists solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = voltage(&session, "board.gpio_in.P0_10");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = voltage(&session, "board.gpio_in.P0_25");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(diode_v < 0.99, "diode node stays below test VIL");

        let pin = bus.read_u32(GPIO_PIN).expect("read LPC GPIO PIN[0]");
        assert_eq!(
            pin & (1 << 10) != 0,
            expected_ntc_level,
            "P0_10 follows the solved NTC voltage and retains its Schmitt state"
        );
        assert_eq!(
            pin & (1 << 25),
            0,
            "P0_25 reads low from the diode load line"
        );
    }
}
