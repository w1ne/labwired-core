// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists reach the Calliope mini's nRF51 ADC channels.

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
fn calliope_mini_ntc_and_diode_netlists_reach_distinct_adc_channels() {
    let system_path = root("configs/systems/calliope-mini-v1.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Calliope profile");
    assert_eq!(profile.name, "calliope-mini-v1");
    let analog_path = root("examples/calliope-mini-v1/analog-ntc-diode-system.yaml");
    let analog =
        SystemManifest::from_file(&analog_path).expect("load independent NTC and diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF51822 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Calliope system bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile carries two analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both device models must bind to distinct nRF51 ADC channels"
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

/// Node voltage of a 1 MΩ pull-up from 3.3 V into a reverse-biased diode
/// whose current is the fixture card's tunnel term plus GMIN, by bisection.
/// `JT(T) = JTUN·exp((T/Tn − 1)·KEG·EG/(NTUN·Vt) + XTITUN/NTUN·ln(T/Tn))`.
fn tunnel_pull_up_voltage(temperature_c: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const PULL_UP_OHMS: f64 = 1e6;
    const JTUN_A: f64 = 1e-9;
    const NTUN: f64 = 20.0;
    const XTITUN: f64 = 3.0;
    const KEG_EG_EV: f64 = 1.11;
    const GMIN: f64 = 1e-12;
    let kelvin = temperature_c + 273.15;
    let ratio = kelvin / (25.0 + 273.15);
    let thermal_v = 1.380_649e-23 * kelvin / 1.602_176_634e-19;
    let saturation = JTUN_A
        * ((ratio - 1.0) * KEG_EG_EV / (NTUN * thermal_v) + XTITUN / NTUN * ratio.ln()).exp();
    let residual = |node_v: f64| {
        (SUPPLY_V - node_v) / PULL_UP_OHMS
            - saturation * ((node_v / (NTUN * thermal_v)).exp() - 1.0)
            - GMIN * node_v
    };
    let (mut low, mut high) = (0.0, SUPPLY_V);
    for _ in 0..100 {
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

#[test]
fn calliope_mini_diode_tunnel_current_reaches_adc_beside_ntc_and_diode() {
    let system_path = root("configs/systems/calliope-mini-v1.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Calliope profile");
    let analog = SystemManifest::from_file(root(
        "examples/calliope-mini-v1/analog-ntc-diode-system.yaml",
    ))
    .expect("load independent NTC and diode netlists");
    let tunnel = SystemManifest::from_file(root(
        "examples/calliope-mini-v1/analog-diode-tunnel-system.yaml",
    ))
    .expect("load generic tunnel-diode netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(tunnel.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF51822 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Calliope system bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile carries three analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all netlists must bind to AIN0..AIN3"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("hold the NTC at 25 °C");
    let cycles = session.cycles_until_boundary(0);
    let (steps, errors) = session
        .advance_to(cycles, &mut bus)
        .expect("solve and route every netlist");
    assert_eq!(steps.len(), 3, "NTC, diode and tunnel netlists all solve");
    assert!(errors.is_empty(), "route errors: {errors:?}");

    // ngspice-42 `.op` of the fixture netlist.
    let mut codes = Vec::new();
    for (channel, temperature_c, ngspice_v) in [(2, 25.0, 2.974_456_604), (3, 85.0, 3.083_403_732)]
    {
        let solved = signal_voltage(&session, &format!("adc.adc.{channel}_volts"));
        let expected = tunnel_pull_up_voltage(temperature_c);
        assert!(
            (solved - expected).abs() < 1e-4,
            "AIN{channel} at {temperature_c} °C: tunnel law {expected} V, got {solved} V"
        );
        assert!(
            (solved - ngspice_v).abs() < 1e-4,
            "AIN{channel} at {temperature_c} °C: ngspice-42 {ngspice_v} V, got {solved} V"
        );
        let code = convert_channel(&mut bus, channel);
        assert_eq!(
            code,
            expected_10_bit_code(solved),
            "AIN{channel}'s firmware-visible result must follow the tunnel load line"
        );
        codes.push(code);
    }

    // Without JTUN the pull-ups sit at 3.3 V (code 938). The tunnel current
    // pulls both well below, and the hot diode conducts less at this bias.
    let tunnel_free = expected_10_bit_code(3.3);
    assert!(
        codes.iter().all(|code| tunnel_free - code >= 60),
        "tunnel current must move both codes >= 60 LSB below {tunnel_free}: {codes:?}"
    );
    assert!(
        codes[1] - codes[0] >= 20,
        "the 85 °C tunnel diode must read >= 20 LSB above the 25 °C one: {codes:?}"
    );

    let ntc_code = convert_channel(&mut bus, 0);
    let ntc_v = signal_voltage(&session, "adc.adc.0_volts");
    assert!(
        (ntc_v - 1.65).abs() < 1e-9,
        "NTC divider at 25 °C is mid-rail"
    );
    assert_eq!(ntc_code, expected_10_bit_code(ntc_v));
    let diode_v = signal_voltage(&session, "adc.adc.1_volts");
    assert!((diode_v - diode_load_line_voltage()).abs() < 2e-5);
    let diode_code = convert_channel(&mut bus, 1);
    assert_eq!(
        diode_code,
        expected_10_bit_code(diode_v),
        "the plain diode channel is unaffected by the tunnel netlist"
    );
}
