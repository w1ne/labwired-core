// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA device models route through Metro M4 analog pads into ADC0.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC0: u64 = 0x4300_1c00;
const MCLK_APBDMASK: u64 = 0x4000_0820;
const GCLK_PCHCTRL40: u64 = 0x4000_1d20;
const PORTA: u64 = 0x4100_8000;
const PORTB: u64 = 0x4100_8080;

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

fn expected_code(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    ((millivolts * (1 << 12) / 3300).min((1 << 12) - 1)) as u16
}

fn convert_adc0(bus: &mut SystemBus, ain: u8) -> u16 {
    bus.write_u16(ADC0 + 0x04, 0x1800 | u16::from(ain))
        .expect("select single-ended ADC0 input");
    bus.write_u8(ADC0 + 0x14, 1 << 1)
        .expect("start ADC0 conversion");
    bus.tick_peripherals();
    assert_ne!(
        bus.read_u8(ADC0 + 0x2e).expect("read ADC0 INTFLAG") & 1,
        0,
        "clocked conversion must set RESRDY"
    );
    bus.read_u16(ADC0 + 0x40).expect("read ADC0 RESULT")
}

fn enable_function_b(bus: &mut SystemBus, port: u64, pin: u8) {
    // Even and odd pins share one PMUX byte; keep the other pin's nibble.
    let pmux_address = port + 0x30 + u64::from(pin / 2);
    let (keep, function_b) = if pin & 1 == 0 {
        (0xf0, 1)
    } else {
        (0x0f, 1 << 4)
    };
    let pmux = bus.read_u8(pmux_address).expect("read PMUX");
    bus.write_u8(pmux_address, (pmux & keep) | function_b)
        .expect("select ADC peripheral function B");
    bus.write_u8(port + 0x40 + u64::from(pin), 1)
        .expect("enable peripheral mux on analog pad");
}

#[test]
fn metro_m4_ntc_and_diode_netlists_reach_adc0_a0_and_a4() {
    let system_path = root("configs/systems/metro-m4.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Metro M4 profile");
    assert_eq!(profile.name, "metro-m4");
    let analog = SystemManifest::from_file(root("examples/metro-m4/analog-ntc-diode-system.yaml"))
        .expect("load generic NTC divider and Shockley diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ATSAMD51J19A descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Metro M4 system bus");
    bus.write_u32(MCLK_APBDMASK, 1 << 7)
        .expect("enable ADC0 APB clock");
    bus.write_u32(GCLK_PCHCTRL40, 1 << 6)
        .expect("enable ADC0 generic clock");
    bus.write_u8(ADC0 + 0x08, 3)
        .expect("select VDDANA reference");
    bus.write_u8(ADC0, 1 << 1).expect("enable ADC0");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "PA02/AIN0 and PB08/AIN2 must bind through the chip's analog pin map"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut ntc_samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(steps.len(), 2, "both independent netlists must solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = voltage(&session, "board.analog.pa02_volts");
        let expected_ntc_v = 3.3 * 2_200.0 / (ntc_resistance_ohms(temperature_c) + 2_200.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the Beta NTC divider at {temperature_c} °C should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = voltage(&session, "board.analog.pb08_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        if ntc_samples.is_empty() {
            assert_eq!(
                convert_adc0(&mut bus, 0),
                0,
                "A0 remains disconnected from ADC0 until PMUX function B is enabled"
            );
            enable_function_b(&mut bus, PORTA, 2); // A0 / PA02 / AIN0
            enable_function_b(&mut bus, PORTB, 8); // A4 / PB08 / AIN2
        }

        let ntc_code = convert_adc0(&mut bus, 0);
        let diode_code = convert_adc0(&mut bus, 2);
        let undriven_code = convert_adc0(&mut bus, 1);
        assert_eq!(
            ntc_code,
            expected_code(ntc_v),
            "ADC0 AIN0 must convert the routed, temperature-dependent voltage"
        );
        assert_eq!(
            diode_code,
            expected_code(diode_v),
            "ADC0 AIN2 must independently convert the Shockley diode voltage"
        );
        assert_eq!(undriven_code, 0, "the adjacent undriven AIN1 stays at zero");
        ntc_samples.push((temperature_c, ntc_v, ntc_code));
    }

    assert!(
        ntc_samples[2].1 > ntc_samples[1].1 && ntc_samples[1].1 > ntc_samples[0].1,
        "the divider voltage must rise with NTC temperature: {ntc_samples:?}"
    );
    assert!(
        ntc_samples[2].2 > ntc_samples[1].2 && ntc_samples[1].2 > ntc_samples[0].2,
        "the ADC result must track the solved NTC voltage: {ntc_samples:?}"
    );
}

/// Independent forward-biased bulk-junction load line: `supply` drives the
/// bulk through 100 kΩ into two parallel ideal junctions whose summed
/// saturation current is `saturation_a`, at the engine's fixed 300 K.
fn bulk_junction_voltage(supply_v: f64, saturation_a: f64) -> f64 {
    const LOAD_OHMS: f64 = 100_000.0;
    const THERMAL_V: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;
    let residual = |bulk_v: f64| {
        (supply_v - bulk_v) / LOAD_OHMS - saturation_a * ((bulk_v / THERMAL_V).exp() - 1.0)
    };
    let (mut low, mut high) = (0.0, supply_v);
    for _ in 0..200 {
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
fn metro_m4_mosfet_junction_density_reaches_adc0_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/metro-m4.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Metro M4 profile");
    let analog = SystemManifest::from_file(root("examples/metro-m4/analog-ntc-diode-system.yaml"))
        .expect("load generic NTC divider and Shockley diode");
    let junctions = SystemManifest::from_file(root(
        "examples/metro-m4/analog-mosfet-junction-density.yaml",
    ))
    .expect("load generic NMOS JS bulk-junction branches");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(junctions.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ATSAMD51J19A descriptor");
    for (pad, channel) in [
        ("PA02", 0),
        ("PB08", 2),
        ("PA05", 5),
        ("PA06", 6),
        ("PA04", 4),
    ] {
        assert_eq!(chip.analog_pins[pad].peripheral, "adc0");
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Metro M4 system bus");
    bus.write_u32(MCLK_APBDMASK, 1 << 7)
        .expect("enable ADC0 APB clock");
    bus.write_u32(GCLK_PCHCTRL40, 1 << 6)
        .expect("enable ADC0 generic clock");
    bus.write_u8(ADC0 + 0x08, 3)
        .expect("select VDDANA reference");
    bus.write_u8(ADC0, 1 << 1).expect("enable ADC0");
    enable_function_b(&mut bus, PORTA, 2); // A0 / PA02 / AIN0
    enable_function_b(&mut bus, PORTB, 8); // A4 / PB08 / AIN2
    enable_function_b(&mut bus, PORTA, 5); // A1 / PA05 / AIN5
    enable_function_b(&mut bus, PORTA, 6); // A2 / PA06 / AIN6
    enable_function_b(&mut bus, PORTA, 4); // A3 / PA04 / AIN4

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile carries the analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "A0..A4 must bind through the chip's analog pin map"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("hold the unrelated NTC at nominal temperature");

    // ngspice-45 `.op` at TEMP = TNOM = 26.85 °C (the engine's fixed 300 K)
    // for the same deck at the default 3.3 V supply.
    const NGSPICE_3V3: [f64; 3] = [0.426_057_2, 0.408_297_2, 0.544_026_4];
    // JS·(AD+AS) for the area branches; 2·IS when AS is absent.
    const SATURATION_A: [f64; 3] = [2e-12, 4e-12, 2e-14];
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    for supply_v in [3.3_f64, 2.5] {
        session
            .set_signal_number("ui.mos_js.supply_v", supply_v)
            .expect("set the common bulk-junction supply");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all analog netlists");
        assert_eq!(steps.len(), 3, "all three independent netlists must solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let bulk_v = [
            voltage(&session, "board.analog.pa05_volts"),
            voltage(&session, "board.analog.pa06_volts"),
            voltage(&session, "board.analog.pa04_volts"),
        ];
        for (index, label) in ["A1 symmetric JS", "A2 asymmetric JS", "A3 IS fallback"]
            .into_iter()
            .enumerate()
        {
            let expected = bulk_junction_voltage(supply_v, SATURATION_A[index]);
            assert!(
                (bulk_v[index] - expected).abs() < 2e-5,
                "{label} at {supply_v} V should be {expected} V, got {}",
                bulk_v[index]
            );
            if supply_v == 3.3 {
                assert!(
                    (bulk_v[index] - NGSPICE_3V3[index]).abs() < 1e-4,
                    "{label} should match ngspice {} V, got {}",
                    NGSPICE_3V3[index],
                    bulk_v[index]
                );
            }
        }
        assert!(
            bulk_v[2] > bulk_v[0] + 0.1 && bulk_v[0] > bulk_v[1] + 0.015,
            "JS·AD/AS must separate the three branches: {bulk_v:?}"
        );

        let ntc_v = voltage(&session, "board.analog.pa02_volts");
        let diode_v = voltage(&session, "board.analog.pb08_volts");
        let expected_ntc_v = 3.3 * 2_200.0 / (ntc_resistance_ohms(25.0) + 2_200.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the nominal NTC divider is unchanged"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode load line is unchanged"
        );

        let codes = [
            convert_adc0(&mut bus, 5),
            convert_adc0(&mut bus, 6),
            convert_adc0(&mut bus, 4),
        ];
        for (index, ain) in [5, 6, 4].into_iter().enumerate() {
            assert_eq!(
                codes[index],
                expected_code(bulk_v[index]),
                "firmware-visible ADC0 AIN{ain} must convert its solved bulk voltage"
            );
        }
        assert!(
            codes[2] > codes[0] + 100 && codes[0] > codes[1] + 15,
            "the JS branches must stay distinct in 12-bit counts: {codes:?}"
        );
        assert_eq!(convert_adc0(&mut bus, 0), expected_code(ntc_v));
        assert_eq!(convert_adc0(&mut bus, 2), expected_code(diode_v));
        assert_eq!(convert_adc0(&mut bus, 1), 0, "undriven AIN1 stays at zero");
    }
}
