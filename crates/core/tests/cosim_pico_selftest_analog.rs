// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog devices route through separate Pico ADC input channels.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC_BASE: u64 = 0x4004_C000;
const ADC_CS: u64 = ADC_BASE;
const ADC_RESULT: u64 = ADC_BASE + 0x04;
const CS_EN: u32 = 1;
const CS_START_ONCE: u32 = 1 << 2;
const CS_AINSEL_SHIFT: u32 = 12;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line for the 3.3 V source and series diode.
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

/// Exercise the RP2040's firmware-visible AINSEL / START_ONCE / RESULT path.
fn convert_channel(bus: &mut SystemBus, channel: u8) -> u16 {
    bus.write_u32(
        ADC_CS,
        CS_EN | (u32::from(channel) << CS_AINSEL_SHIFT) | CS_START_ONCE,
    )
    .expect("select ADC input and start one conversion");
    bus.read_u32(ADC_RESULT)
        .expect("read the converted ADC result") as u16
}

fn expected_adc_code(volts: f64) -> u16 {
    let millivolts = (volts * 1000.0).round() as u64;
    ((millivolts * 4096 / 3300).min(4095)) as u16
}

#[test]
fn pico_selftest_ntc_and_diode_netlists_reach_adc_channels_end_to_end() {
    let system_path = root("configs/systems/pico-selftest.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Pico selftest profile");
    assert_eq!(profile.name, "pico-selftest");
    assert!(profile
        .peripherals
        .iter()
        .any(|peripheral| peripheral.id == "simctl"));

    let analog_path = root("examples/pico-selftest/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path)
        .expect("load generic NTC and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load RP2040 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Pico selftest bus");
    assert!(bus.find_peripheral_index_by_name("simctl").is_some());

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal co-simulation")
        .expect("fixture carries both generic analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both devices must resolve to distinct RP2040 ADC channels"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature input");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve both netlists and route the ADC pad voltages");
        assert_eq!(
            steps.len(),
            2,
            "both unrelated devices solve at the boundary"
        );
        assert!(
            errors.is_empty(),
            "unexpected ADC routing errors: {errors:?}"
        );

        let ntc_v = signal_voltage(&session, "adc.adc.0_volts");
        let expected_ntc_v = 3.3 * ntc_resistance_ohms(temperature_c)
            / (10_000.0 + ntc_resistance_ohms(temperature_c));
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "adc.adc.1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let ntc_code = convert_channel(&mut bus, 0);
        let diode_code = convert_channel(&mut bus, 1);
        assert_eq!(
            ntc_code,
            expected_adc_code(ntc_v),
            "the register-visible ADC0 result must follow the solved NTC voltage"
        );
        assert_eq!(
            diode_code,
            expected_adc_code(diode_v),
            "the independent ADC1 result must follow the Shockley load line"
        );
        assert_eq!(
            convert_channel(&mut bus, 2),
            0,
            "undriven ADC2 remains at ground"
        );
        samples.push((temperature_c, ntc_v, ntc_code, diode_code));
    }

    assert!(
        samples[0].1 > samples[1].1 && samples[1].1 > samples[2].1,
        "the NTC divider voltage should fall as it heats: {samples:?}"
    );
    assert!(
        samples[0].2 > samples[1].2 && samples[1].2 > samples[2].2,
        "the 12-bit ADC result should track the NTC voltage: {samples:?}"
    );
    assert!(
        samples.iter().all(|sample| sample.3 == samples[0].3),
        "the independent diode channel stays constant as the NTC changes: {samples:?}"
    );
}
