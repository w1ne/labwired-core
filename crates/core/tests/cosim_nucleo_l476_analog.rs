// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices drive PA0/PA1 GPIO inputs on the NUCLEO-L476RG and its
//! read-only CI fixture. The tests assert both the solved voltages and the
//! firmware-visible GPIO input levels through the chip's datasheet thresholds.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_AHB2ENR: u64 = 0x4002_104C;
const GPIOA_MODER: u64 = 0x4800_0000;
const GPIOA_IDR: u64 = 0x4800_0010;

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

/// Independent forward-active Gummel–Poon load line, with optional Early effect.
fn bjt_collector_voltage(base_drive_v: f64, early_voltage_v: Option<f64>) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-15;
    const BF: f64 = 100.0;
    const BR: f64 = 4.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let early_factor = early_voltage_v.map_or(1.0, |vaf| 1.0 - vbc / vaf);
        let transport = IS_A * (forward - reverse) * early_factor;
        let base_emitter = (IS_A / BF) * (forward - 1.0);
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        (transport - base_collector, base_emitter + base_collector)
    };

    let collector_for_vbe = |vbe: f64| {
        let (mut low, mut high) = (0.0, SUPPLY_V);
        for _ in 0..100 {
            let collector_v = (low + high) / 2.0;
            let (collector_current, _) = currents(vbe, collector_v);
            let residual = (SUPPLY_V - collector_v) / COLLECTOR_OHMS - collector_current;
            if residual > 0.0 {
                low = collector_v;
            } else {
                high = collector_v;
            }
        }
        (low + high) / 2.0
    };

    let (mut low, mut high) = (0.0, base_drive_v);
    for _ in 0..100 {
        let vbe = (low + high) / 2.0;
        let collector_v = collector_for_vbe(vbe);
        let (_, base_current) = currents(vbe, collector_v);
        let residual = (base_drive_v - vbe) / BASE_OHMS - base_current;
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe((low + high) / 2.0)
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn assert_profile_routes_generic_analog_inputs(
    system_rel: &str,
    expected_name: &str,
    expected_external_devices: &[&str],
) {
    let system_path = root(system_rel);
    let mut profile = SystemManifest::from_file(&system_path).expect("load L476 profile");
    assert_eq!(profile.name, expected_name);
    let external_device_ids: Vec<_> = profile
        .external_devices
        .iter()
        .map(|device| device.id.as_str())
        .collect();
    assert_eq!(
        external_device_ids, expected_external_devices,
        "the analog fixture must preserve the profile's external devices"
    );
    let analog_path = root("examples/nucleo-l476rg/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L476 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build L476 system bus");
    bus.write_u32(RCC_AHB2ENR, 1).expect("enable GPIOA clock");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both generic devices must bind to distinct L476 GPIO input pads"
    );

    let expected_diode_v = diode_load_line_voltage();
    assert!(expected_diode_v < 0.99, "diode must fall below L476 VIL");
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // retain the cold high level while inside the Schmitt band
        (80.0, false),
        (25.0, false), // retain the hot low level while inside the Schmitt band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(steps.len(), 2, "both analog netlists cross the boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "board.gpio_in.pa1");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let gpioa = bus.read_u32(GPIOA_IDR).expect("read GPIOA input data");
        assert_eq!(
            gpioa & 1 != 0,
            expected_ntc_level,
            "PA0 must follow the solved NTC voltage and retain its Schmitt state"
        );
        assert_eq!(
            gpioa & 2,
            0,
            "PA1 must read low for the diode voltage below VIL"
        );
    }
}

#[test]
fn nucleo_l476rg_profiles_route_ntc_and_diode_to_gpio_thresholds() {
    assert_profile_routes_generic_analog_inputs(
        "configs/systems/nucleo-l476rg.yaml",
        "nucleo-l476rg",
        &[],
    );
    assert_profile_routes_generic_analog_inputs(
        "configs/systems/ci-fixture-l476-readonly.yaml",
        "ci-fixture-l476-readonly",
        &[],
    );
}

#[test]
fn iolink_multiport_demo_routes_generic_ntc_and_diode_to_gpio_thresholds() {
    assert_profile_routes_generic_analog_inputs(
        "configs/systems/iolink-multiport-demo.yaml",
        "iolink-multiport-demo",
        &["iolink_master_4p"],
    );
}

#[test]
fn nucleo_l476rg_bjt_early_effect_reaches_pa2_pa3_gpio_inputs() {
    let system_path = root("configs/systems/nucleo-l476rg.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L476RG profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-l476rg/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC divider and Shockley diode");
    let bjt =
        SystemManifest::from_file(root("examples/nucleo-l476rg/analog-bjt-early-effect.yaml"))
            .expect("load matched generic Gummel–Poon NPN branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L476 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L476RG bus");
    for (pad, bit) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(
            SystemBus::resolve_pin_idr_pub(&bus, pad),
            Some((GPIOA_IDR, bit)),
            "{pad} must resolve to its modeled GPIOA IDR bit"
        );
    }
    bus.write_u32(RCC_AHB2ENR, 1).expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_MODER).expect("read GPIOA mode");
    bus.write_u32(GPIOA_MODER, moder & !0xFF)
        .expect("configure PA0-PA3 as digital inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched BJT models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must route independently to PA0 through PA3"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", -10.0)
        .expect("set independent NTC source");
    session
        .set_signal_number("ui.bjt.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    let vil_v = chip.io_voltage_v.unwrap() * chip.gpio_input_thresholds.as_ref().unwrap().vil;
    let vih_v = chip.io_voltage_v.unwrap() * chip.gpio_input_thresholds.as_ref().unwrap().vih;
    let expected_ntc_resistance = ntc_resistance_ohms(-10.0);
    let expected_ntc_v = 3.3 * expected_ntc_resistance / (10_000.0 + expected_ntc_resistance);
    let expected_diode_v = diode_load_line_voltage();
    let mut early_high = false;
    let mut reference_high = false;
    let mut cycles = 0;
    for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route unrelated devices and matched BJT branches");
        assert_eq!(steps.len(), 4, "all four analog branches solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let diode_v = signal_voltage(&session, "board.gpio_in.pa1");
        let early_v = signal_voltage(&session, "board.gpio_in.pa2");
        let reference_v = signal_voltage(&session, "board.gpio_in.pa3");
        let expected_early_v = bjt_collector_voltage(base_drive_v, Some(50.0));
        let expected_reference_v = bjt_collector_voltage(base_drive_v, None);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "unrelated NTC voltage should remain {expected_ntc_v} V, got {ntc_v} V"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(
            (early_v - expected_early_v).abs() < 5e-5,
            "VAF=50 collector voltage at base drive {base_drive_v} V should be {expected_early_v} V, got {early_v} V"
        );
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "collector voltage without Early effect at base drive {base_drive_v} V should be {expected_reference_v} V, got {reference_v} V"
        );
        if base_drive_v >= 0.7 {
            assert!(
                early_v < reference_v - 0.005,
                "finite VAF increases collector current and lowers collector voltage: {early_v} V vs {reference_v} V"
            );
        }

        if early_v >= vih_v {
            early_high = true;
        } else if early_v <= vil_v {
            early_high = false;
        }
        if reference_v >= vih_v {
            reference_high = true;
        } else if reference_v <= vil_v {
            reference_high = false;
        }
        let idr = bus.read_u32(GPIOA_IDR).expect("read GPIOA input levels");
        assert_ne!(idr & 1, 0, "the cold NTC remains above VIH on PA0");
        assert_eq!(idr & 2, 0, "the diode remains below VIL on PA1");
        assert_eq!(
            idr & 4 != 0,
            early_high,
            "PA2 follows the finite-VAF voltage"
        );
        assert_eq!(
            idr & 8 != 0,
            reference_high,
            "PA3 follows the no-Early-effect voltage"
        );
    }
}
