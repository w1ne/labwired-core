// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices drive PA0/PA1 on the exact IO-Linki STM32G0B1 startup
//! profile. The test proves NTC and Shockley-diode solving, pad resolution,
//! G0 GPIO thresholding, and firmware-visible IDR state; it does not claim ADC
//! support or silicon validation.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_IOPENR: u64 = 0x4002_1034;
const GPIOA_MODER: u64 = 0x5000_0000;
const GPIOA_IDR: u64 = 0x5000_0010;

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

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

#[test]
fn iolinki_g0_startup_routes_generic_ntc_and_diode_to_gpio_thresholds() {
    let system_path = root("configs/systems/iolinki-g0-startup.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load customer G0 profile");
    assert_eq!(profile.name, "iolinki-g0-startup");
    assert!(profile.external_devices.is_empty());

    let analog_path = root("examples/stm32g0b1re/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G0B1 descriptor");
    assert_eq!(chip.io_voltage_v, Some(3.3));
    let thresholds = chip.gpio_input_thresholds.expect("G0 datasheet thresholds");
    assert_eq!(thresholds.vil, 0.3);
    assert_eq!(thresholds.vih, 0.7);

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build G0 system bus");
    bus.write_u32(RCC_IOPENR, 1).expect("enable GPIOA clock");
    let gpioa_moder = bus.read_u32(GPIOA_MODER).expect("read GPIOA mode");
    bus.write_u32(GPIOA_MODER, gpioa_moder & !0xF)
        .expect("configure PA0/PA1 as digital inputs");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve onto distinct PA0/PA1 pads"
    );

    let expected_diode_v = diode_load_line_voltage();
    assert!(expected_diode_v < 0.99, "diode must fall below G0 VIL");
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // keep the cold high while the input is in its threshold band
        (80.0, false),
        (25.0, false), // keep the hot low while the input is in its threshold band
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
            "at {temperature_c} °C NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "board.gpio_in.pa1");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let gpioa = bus.read_u32(GPIOA_IDR).expect("read GPIOA input data");
        assert_eq!(
            gpioa & 1 != 0,
            expected_ntc_level,
            "PA0 must follow the solved NTC voltage and retain its threshold state"
        );
        assert_eq!(gpioa & 2, 0, "PA1 must read low below the G0 VIL");
    }
}
