// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA netlists drive the nRF52840-DK's SAADC channels.

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

#[test]
fn nrf52840_dk_ntc_and_diode_netlists_reach_distinct_saadc_channels() {
    let system_path = root("configs/systems/nrf52840-dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF52840-DK profile");
    assert_eq!(profile.name, "nrf52840-dk");
    assert!(profile.external_devices.is_empty());
    assert_eq!(
        profile
            .board_io
            .iter()
            .map(|io| (io.id.as_str(), io.pin))
            .collect::<Vec<_>>(),
        [
            ("led0", 13),
            ("led1", 14),
            ("led2", 15),
            ("led3", 16),
            ("button0", 11),
            ("button1", 12),
            ("button2", 24),
            ("button3", 25),
        ],
        "the analog test must retain the DK's normal LED and button wiring"
    );

    let analog_path = root("examples/nrf52840-dk/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF52840 descriptor");
    assert!(
        chip.analog_pins.is_empty(),
        "use the existing explicit ADC channel route without changing the shared silicon profile"
    );
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF52840-DK bus");
    configure_saadc(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile carries two analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic analog device outputs must bind to SAADC channels 0 and 1"
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

        let ntc_v = voltage(&session, "adc.saadc.0_volts");
        let ntc_r = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * ntc_r / (ntc_r + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the Beta NTC divider at {temperature_c} °C should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = voltage(&session, "adc.saadc.1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let [ntc_code, diode_code, undriven_code] = scan(&mut bus);
        assert_eq!(
            ntc_code,
            expected_saadc_code(ntc_v),
            "SAADC channel 0 must convert the solved temperature-dependent voltage"
        );
        assert_eq!(
            diode_code,
            expected_saadc_code(diode_v),
            "SAADC channel 1 must independently convert the Shockley load line"
        );
        assert_eq!(
            undriven_code, 0,
            "the routed but undriven SAADC channel 2 stays at ground"
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
