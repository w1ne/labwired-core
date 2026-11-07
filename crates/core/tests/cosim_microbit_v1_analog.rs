// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists reach the micro:bit V1's nRF51 single-sample ADC.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC: u64 = 0x4000_7000;

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

fn convert_channel(bus: &mut SystemBus, channel: u8) -> u16 {
    // RES=10 bit, INPSEL=1/3 prescale, REFSEL=1.2 V VBG, PSEL=AIN[channel].
    let config = 2 | (2 << 2) | (u32::from(1u8 << channel) << 8);
    bus.write_u32(ADC + 0x504, config)
        .expect("configure nRF51 ADC conversion");
    bus.write_u32(ADC + 0x500, 1).expect("enable nRF51 ADC");
    bus.write_u32(ADC, 1).expect("start ADC conversion");
    assert_eq!(bus.read_u32(ADC + 0x400).unwrap(), 1, "START sets BUSY");
    bus.tick_peripherals_fully();
    assert_eq!(
        bus.read_u32(ADC + 0x400).unwrap(),
        0,
        "conversion clears BUSY"
    );
    assert_eq!(bus.read_u32(ADC + 0x100).unwrap(), 1, "conversion sets END");
    let result = bus.read_u32(ADC + 0x508).unwrap() as u16;
    bus.write_u32(ADC + 0x100, 0)
        .expect("clear END before next sample");
    result
}

fn expected_10_bit_code(volts: f64) -> u16 {
    let input_mv = (volts * 1000.0 + 0.5).floor() as u64;
    ((input_mv * 1024 / 3 / 1200).min(1023)) as u16
}

#[test]
fn microbit_v1_ntc_and_diode_netlists_reach_distinct_adc_channels() {
    let system_path = root("configs/systems/microbit-v1.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load micro:bit V1 profile");
    assert_eq!(profile.name, "microbit-v1");
    assert_eq!(profile.external_devices.len(), 1);
    assert_eq!(profile.external_devices[0].id, "led_matrix");
    let board_io_ids: Vec<_> = profile.board_io.iter().map(|io| io.id.as_str()).collect();
    assert_eq!(board_io_ids, ["button_a", "button_b"]);

    let analog_path = root("examples/microbit-v1/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF51822 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build micro:bit V1 system bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile carries two analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both generic devices must bind to distinct nRF51 ADC channels"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both device netlists");
        assert_eq!(steps.len(), 2, "both independent analog devices solve");
        assert!(
            errors.is_empty(),
            "unexpected analog route errors: {errors:?}"
        );

        let ntc_v = signal_voltage(&session, "adc.adc.0_volts");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (resistance + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the Beta NTC divider at {temperature_c} °C should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "adc.adc.1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let ntc_code = convert_channel(&mut bus, 0);
        let diode_code = convert_channel(&mut bus, 1);
        assert_eq!(
            ntc_code,
            expected_10_bit_code(ntc_v),
            "AIN0's firmware-visible result must follow the solved NTC voltage"
        );
        assert_eq!(
            diode_code,
            expected_10_bit_code(diode_v),
            "AIN1's independent result must follow the Shockley load line"
        );
        assert_eq!(
            convert_channel(&mut bus, 2),
            0,
            "the unconnected AIN2 must not inherit either driven analog input"
        );
        samples.push((temperature_c, ntc_v, ntc_code, diode_code));
    }

    assert!(
        samples[0].1 > samples[1].1 && samples[1].1 > samples[2].1,
        "the NTC divider voltage should fall as the NTC heats: {samples:?}"
    );
    assert!(
        samples[0].2 > samples[1].2 && samples[1].2 > samples[2].2,
        "the firmware ADC result should track the temperature-dependent voltage: {samples:?}"
    );
    assert!(
        samples.iter().all(|sample| sample.3 == samples[0].3),
        "the independent diode channel should stay constant while the NTC changes: {samples:?}"
    );
}
