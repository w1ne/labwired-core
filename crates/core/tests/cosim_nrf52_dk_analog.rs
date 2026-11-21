// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA netlists drive the nRF52-DK's nRF52832 SAADC physical inputs.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const SAADC: u64 = 0x4000_7000;
const RESULT: u64 = 0x2000_0000;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const SERIES_LIMIT_OHMS: f64 = 1_000.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
    let current_at_junction_voltage =
        |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let current = current_at_junction_voltage(junction_v);
        (SUPPLY_V - junction_v - current * DIODE_RS_OHMS) / SERIES_LIMIT_OHMS - current
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

fn expected_saadc_code(volts: f64) -> u16 {
    let millivolts = (volts * 1000.0).round() as u32;
    ((millivolts * 4096 / 3300).min(4095)) as u16
}

fn configure_saadc(bus: &mut SystemBus) {
    // CH[0..2] select physical AIN0..2. AIN2 is intentionally left undriven.
    for (channel, ain) in [(0u64, 1u32), (1, 2), (2, 3)] {
        let base = SAADC + 0x510 + channel * 0x10;
        bus.write_u32(base, ain)
            .expect("select SAADC positive input");
        bus.write_u32(base + 8, 0x1200)
            .expect("select gain and VDD/4 reference");
    }
    bus.write_u32(SAADC + 0x5f0, 2)
        .expect("select 12-bit SAADC resolution");
    bus.write_u32(SAADC + 0x62c, RESULT as u32)
        .expect("set SAADC EasyDMA result pointer");
    bus.write_u32(SAADC + 0x630, 3)
        .expect("reserve three channel results");
    bus.write_u32(SAADC + 0x500, 1).expect("enable SAADC");
}

fn scan(bus: &mut SystemBus) -> [u16; 3] {
    bus.write_u32(SAADC + 0x104, 0)
        .expect("clear prior END event");
    bus.write_u32(SAADC, 1).expect("arm SAADC EasyDMA scan");
    bus.write_u32(SAADC + 4, 1)
        .expect("sample configured SAADC channels");
    bus.tick_peripherals_fully();

    assert_eq!(bus.read_u32(SAADC + 0x104).unwrap(), 1, "scan sets END");
    assert_eq!(
        bus.read_u32(SAADC + 0x634).unwrap(),
        3,
        "scan writes all three results"
    );
    [
        bus.read_u16(RESULT).unwrap(),
        bus.read_u16(RESULT + 2).unwrap(),
        bus.read_u16(RESULT + 4).unwrap(),
    ]
}

fn configure_seven_channel_saadc(bus: &mut SystemBus) {
    // PSELP 1..7 select AIN0..AIN6. AIN6 is intentionally left undriven.
    for channel in 0u64..7 {
        let base = SAADC + 0x510 + channel * 0x10;
        bus.write_u32(base, channel as u32 + 1)
            .expect("select SAADC positive input");
        bus.write_u32(base + 8, 0x1200)
            .expect("select gain and VDD/4 reference");
    }
    bus.write_u32(SAADC + 0x5f0, 2)
        .expect("select 12-bit SAADC resolution");
    bus.write_u32(SAADC + 0x62c, RESULT as u32)
        .expect("set SAADC EasyDMA result pointer");
    bus.write_u32(SAADC + 0x630, 7)
        .expect("reserve seven channel results");
    bus.write_u32(SAADC + 0x500, 1).expect("enable SAADC");
}

fn scan_seven_channels(bus: &mut SystemBus) -> [u16; 7] {
    bus.write_u32(SAADC + 0x104, 0)
        .expect("clear prior END event");
    bus.write_u32(SAADC, 1).expect("arm SAADC EasyDMA scan");
    bus.write_u32(SAADC + 4, 1)
        .expect("sample configured SAADC channels");
    bus.tick_peripherals_fully();

    assert_eq!(bus.read_u32(SAADC + 0x104).unwrap(), 1, "scan sets END");
    assert_eq!(
        bus.read_u32(SAADC + 0x634).unwrap(),
        7,
        "scan writes all seven results"
    );
    std::array::from_fn(|channel| bus.read_u16(RESULT + channel as u64 * 2).unwrap())
}

/// Independent backward-Euler load line for reverse-biased NMOS bulk
/// junctions, integrating bottom and sidewall depletion charge separately.
fn mosfet_junction_geometry_voltage(
    previous_v: f64,
    current_a: f64,
    area_m2: f64,
    perimeter_m: f64,
) -> f64 {
    const LOAD_OHMS: f64 = 1_000.0;
    const STEP_S: f64 = 1e-6;
    const PB_V: f64 = 1.0;
    const MJ: f64 = 0.5;
    const MJSW: f64 = 0.33;
    const CJ_F_PER_M2: f64 = 2e-3;
    const CJSW_F_PER_M: f64 = 500e-9;

    let reverse_charge = |voltage_v: f64, c0_f: f64, grading: f64| {
        c0_f * PB_V / (1.0 - grading) * ((1.0 + voltage_v / PB_V).powf(1.0 - grading) - 1.0)
    };
    let terminal_charge = |voltage_v: f64| {
        reverse_charge(voltage_v, CJ_F_PER_M2 * area_m2, MJ)
            + reverse_charge(voltage_v, CJSW_F_PER_M * perimeter_m, MJSW)
    };
    let previous_charge = terminal_charge(previous_v);
    let residual = |voltage_v: f64| {
        voltage_v / LOAD_OHMS + (terminal_charge(voltage_v) - previous_charge) / STEP_S - current_a
    };

    let (mut low, mut high) = (0.0, current_a * LOAD_OHMS);
    for _ in 0..100 {
        let mid = 0.5 * (low + high);
        if residual(mid) < 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

#[test]
fn nrf52_dk_ntc_and_diode_netlists_reach_distinct_saadc_pads() {
    let system_path = root("configs/systems/nrf52-dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF52-DK profile");
    assert_eq!(profile.name, "nrf52-dk");
    assert!(profile.external_devices.is_empty());
    assert_eq!(
        profile
            .board_io
            .iter()
            .map(|io| (io.id.as_str(), io.pin))
            .collect::<Vec<_>>(),
        [
            ("led1", 17),
            ("led2", 18),
            ("led3", 19),
            ("led4", 20),
            ("button1", 13),
            ("button2", 14),
            ("button3", 15),
            ("button4", 16),
        ],
        "the analog test must retain the DK's normal LED and button wiring"
    );

    let analog_path = root("examples/nrf52-dk/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF52832 descriptor");
    assert_eq!(chip.analog_pins["P0.02"].channel, 0);
    assert_eq!(chip.analog_pins["P0.03"].channel, 1);
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF52-DK bus");
    configure_saadc(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile carries two analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "P0.02/AIN0 and P0.03/AIN1 must bind through the descriptor's physical pad map"
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

        let ntc_v = voltage(&session, "board.analog.p0.02_volts");
        let ntc_r = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * ntc_r / (ntc_r + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the Beta NTC divider at {temperature_c} °C should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = voltage(&session, "board.analog.p0.03_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let [ntc_code, diode_code, undriven_code] = scan(&mut bus);
        assert_eq!(
            ntc_code,
            expected_saadc_code(ntc_v),
            "P0.02/AIN0 must convert the solved temperature-dependent voltage"
        );
        assert_eq!(
            diode_code,
            expected_saadc_code(diode_v),
            "P0.03/AIN1 must independently convert the Shockley load line"
        );
        assert_eq!(
            undriven_code, 0,
            "the routed but undriven P0.04/AIN2 channel stays at ground"
        );
        samples.push((temperature_c, ntc_v, ntc_code, diode_code));
    }

    assert!(
        samples[0].1 > samples[1].1 && samples[1].1 > samples[2].1,
        "the NTC divider voltage should fall as the NTC heats: {samples:?}"
    );
    assert!(
        samples[0].2 > samples[1].2 && samples[1].2 > samples[2].2,
        "the SAADC result should track the temperature-dependent voltage: {samples:?}"
    );
    assert!(
        samples.iter().all(|sample| sample.3 == samples[0].3),
        "the independent diode channel should stay constant while the NTC changes: {samples:?}"
    );
}

#[test]
fn nrf52_dk_mosfet_junction_geometry_charge_reaches_saadc_with_ntc_and_diode() {
    const CURRENT_A: f64 = 1e-3;
    const DRAIN_AREA_M2: f64 = 2e-6;
    const DRAIN_PERIMETER_M: f64 = 4e-3;
    const SOURCE_AREA_M2: f64 = 3e-6;
    const SOURCE_PERIMETER_M: f64 = 5e-3;

    let system_path = root("configs/systems/nrf52-dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF52-DK profile");
    assert_eq!(profile.name, "nrf52-dk");
    let generic = SystemManifest::from_file(root("examples/nrf52-dk/analog-ntc-diode-system.yaml"))
        .expect("load unrelated generic NTC and diode netlists");
    let junction_geometry = SystemManifest::from_file(root(
        "examples/nrf52-dk/analog-mosfet-junction-geometry-system.yaml",
    ))
    .expect("load generic NMOS junction geometry and zero-charge controls");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(junction_geometry.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF52832 descriptor");
    for (pad, channel) in [
        ("P0.02", 0),
        ("P0.03", 1),
        ("P0.04", 2),
        ("P0.05", 3),
        ("P0.28", 4),
        ("P0.29", 5),
    ] {
        assert_eq!(chip.analog_pins[pad].channel, channel, "{pad} AIN channel");
        assert_eq!(
            chip.analog_pins[pad].peripheral, "saadc",
            "{pad} peripheral"
        );
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF52-DK bus");
    configure_seven_channel_saadc(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries NTC, diode, and MOSFET geometry branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve physical P0.02..P0.29 inputs to SAADC"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    for input in [
        "ui.mosfet_junction.drain_current_a",
        "ui.mosfet_junction.drain_reference_current_a",
        "ui.mosfet_junction.source_current_a",
        "ui.mosfet_junction.source_reference_current_a",
    ] {
        session
            .set_signal_number(input, 0.0)
            .expect("initialize matched NMOS junction-current branches");
    }

    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle NTC, diode, and zero-current junction branches");
    assert_eq!(
        initial_steps.len(),
        3,
        "all three generic analog adapters solve"
    );
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );
    for path in [
        "board.analog.p0.04_volts",
        "board.analog.p0.05_volts",
        "board.analog.p0.28_volts",
        "board.analog.p0.29_volts",
    ] {
        assert!(
            voltage(&session, path).abs() < 1e-12,
            "{path} starts at ground"
        );
    }

    for input in [
        "ui.mosfet_junction.drain_current_a",
        "ui.mosfet_junction.drain_reference_current_a",
        "ui.mosfet_junction.source_current_a",
        "ui.mosfet_junction.source_reference_current_a",
    ] {
        session
            .set_signal_number(input, CURRENT_A)
            .expect("step the matched NMOS junction branches");
    }

    let mut previous_drain_v = 0.0;
    let mut previous_source_v = 0.0;
    let expected_diode_v = diode_load_line_voltage();
    for step in 0..32 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NMOS junction transients with NTC and diode");
        assert_eq!(steps.len(), 3, "all three adapters solve at each 1 us step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let expected_drain_v = mosfet_junction_geometry_voltage(
            previous_drain_v,
            CURRENT_A,
            DRAIN_AREA_M2,
            DRAIN_PERIMETER_M,
        );
        let expected_source_v = mosfet_junction_geometry_voltage(
            previous_source_v,
            CURRENT_A,
            SOURCE_AREA_M2,
            SOURCE_PERIMETER_M,
        );
        previous_drain_v = expected_drain_v;
        previous_source_v = expected_source_v;

        let ntc_v = voltage(&session, "board.analog.p0.02_volts");
        let diode_v = voltage(&session, "board.analog.p0.03_volts");
        let drain_v = voltage(&session, "board.analog.p0.04_volts");
        let drain_reference_v = voltage(&session, "board.analog.p0.05_volts");
        let source_v = voltage(&session, "board.analog.p0.28_volts");
        let source_reference_v = voltage(&session, "board.analog.p0.29_volts");
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "unrelated NTC remains isolated"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains at its independent load line"
        );
        assert!(
            (drain_v - expected_drain_v).abs() < 3e-8,
            "CJ·AD + CJSW·PD drain charge at step {step}: expected {expected_drain_v} V, got {drain_v} V"
        );
        assert!(
            (source_v - expected_source_v).abs() < 3e-8,
            "CJ·AS + CJSW·PS source charge at step {step}: expected {expected_source_v} V, got {source_v} V"
        );
        assert!(
            (drain_reference_v - 1.0).abs() < 2e-8 && (source_reference_v - 1.0).abs() < 2e-8,
            "zero-charge controls reach the 1 V DC load line"
        );
        assert!(
            (drain_v - source_v).abs() > 1e-3,
            "distinct AD/PD and AS/PS values produce distinct junction transients"
        );

        let codes = scan_seven_channels(&mut bus);
        for (channel, voltage_v) in [
            (0, ntc_v),
            (1, diode_v),
            (2, drain_v),
            (3, drain_reference_v),
            (4, source_v),
            (5, source_reference_v),
        ] {
            assert_eq!(
                codes[channel],
                expected_saadc_code(voltage_v),
                "SAADC AIN{channel} converts its independently checked analog route"
            );
        }
        assert_eq!(codes[6], 0, "undriven P0.30/AIN6 remains at ground");
    }
}
