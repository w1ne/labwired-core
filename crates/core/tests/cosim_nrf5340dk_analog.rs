// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA netlists drive the nRF5340-DK's application-core SAADC inputs.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const SAADC: u64 = 0x5000_E000;
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
    configure_saadc_channels(bus, 3);
}

fn configure_saadc_channels(bus: &mut SystemBus, channel_count: u32) {
    // Channel k selects AINk. Tests reserve extra channels as undriven controls.
    for channel in 0..channel_count {
        let base = SAADC + 0x510 + u64::from(channel) * 0x10;
        bus.write_u32(base, channel + 1)
            .expect("select SAADC positive input");
        bus.write_u32(base + 8, 0x1200)
            .expect("select gain and VDD/4 reference");
    }
    bus.write_u32(SAADC + 0x5f0, 2)
        .expect("select 12-bit SAADC resolution");
    bus.write_u32(SAADC + 0x62c, RESULT as u32)
        .expect("set SAADC EasyDMA result pointer");
    bus.write_u32(SAADC + 0x630, channel_count)
        .expect("reserve channel results");
    bus.write_u32(SAADC + 0x500, 1).expect("enable SAADC");
}

fn scan(bus: &mut SystemBus) -> [u16; 3] {
    scan_saadc_channels::<3>(bus)
}

fn scan_saadc_channels<const N: usize>(bus: &mut SystemBus) -> [u16; N] {
    bus.write_u32(SAADC + 0x104, 0)
        .expect("clear prior END event");
    bus.write_u32(SAADC, 1).expect("arm SAADC EasyDMA scan");
    bus.write_u32(SAADC + 4, 1)
        .expect("sample configured SAADC channels");
    bus.tick_peripherals_fully();

    assert_eq!(bus.read_u32(SAADC + 0x104).unwrap(), 1, "scan sets END");
    assert_eq!(
        bus.read_u32(SAADC + 0x634).unwrap(),
        N as u32,
        "scan writes every reserved result"
    );
    std::array::from_fn(|channel| bus.read_u16(RESULT + channel as u64 * 2).unwrap())
}

/// Independent Gummel–Poon NPN load-line reference including forward/reverse
/// Early effect and optional high-injection rolloff.
fn bjt_high_injection_collector_voltage(base_drive_v: f64, high_injection: bool) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const VAF: f64 = 50.0;
    const VAR: f64 = 75.0;
    const IKF_A: f64 = 50e-6;
    const IKR_A: f64 = 500e-6;
    const NKF: f64 = 0.8;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let base_width = 1.0 - vbc / VAF - vbe / VAR;
        let q2 = if high_injection {
            IS_A * (forward - 1.0) / IKF_A + IS_A * (reverse - 1.0) / IKR_A
        } else {
            0.0
        };
        let qb = (1.0 + (1.0 + 4.0 * q2).max(0.0).powf(NKF)) / (2.0 * base_width);
        let transport = IS_A * (forward - reverse) / qb;
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        let base_emitter = (IS_A / BF) * (forward - 1.0);
        (transport - base_collector, base_emitter + base_collector)
    };

    let collector_for_vbe = |vbe: f64| {
        let (mut low, mut high) = (0.0, SUPPLY_V);
        for _ in 0..100 {
            let collector_v = 0.5 * (low + high);
            let (collector_current, _) = currents(vbe, collector_v);
            let residual = (SUPPLY_V - collector_v) / COLLECTOR_OHMS - collector_current;
            if residual > 0.0 {
                low = collector_v;
            } else {
                high = collector_v;
            }
        }
        0.5 * (low + high)
    };

    let (mut low, mut high) = (0.0, base_drive_v);
    for _ in 0..100 {
        let vbe = 0.5 * (low + high);
        let collector_v = collector_for_vbe(vbe);
        let (_, base_current) = currents(vbe, collector_v);
        let residual = (base_drive_v - vbe) / BASE_OHMS - base_current;
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe(0.5 * (low + high))
}

#[test]
fn nrf5340dk_bjt_high_injection_reaches_saadc_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    assert_eq!(profile.name, "nrf5340dk");
    let generic =
        SystemManifest::from_file(root("examples/nrf5340dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let high_injection = SystemManifest::from_file(root(
        "examples/nrf5340dk/analog-bjt-high-injection-system.yaml",
    ))
    .expect("load generic matched BJT high-injection branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(high_injection.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    assert!(chip.analog_pins.is_empty());
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc_channels(&mut bus, 5);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and matched BJT netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to SAADC channels 0 through 3"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");
    session
        .set_signal_number("ui.bjt_high_injection.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut high_drive_samples = None;
    for base_drive_v in [0.8_f64, 1.5, 3.3] {
        session
            .set_signal_number("ui.bjt_high_injection.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched BJT branches");
        assert_eq!(steps.len(), 4, "all four generic analog branches solve");
        assert!(
            errors.is_empty(),
            "unexpected analog route errors: {errors:?}"
        );

        let ntc_v = voltage(&session, "adc.saadc.0_volts");
        let diode_v = voltage(&session, "adc.saadc.1_volts");
        let rolloff_v = voltage(&session, "adc.saadc.2_volts");
        let control_v = voltage(&session, "adc.saadc.3_volts");
        assert!((ntc_v - expected_ntc_v).abs() < 1e-9);
        assert!((diode_v - expected_diode_v).abs() < 2e-5);
        assert!(
            (rolloff_v - bjt_high_injection_collector_voltage(base_drive_v, true)).abs() < 5e-5,
            "IKF/IKR/NKF load line at {base_drive_v} V must match the independent reference, got {rolloff_v} V"
        );
        assert!(
            (control_v - bjt_high_injection_collector_voltage(base_drive_v, false)).abs() < 5e-5,
            "matched no-rolloff load line at {base_drive_v} V must match the independent reference, got {control_v} V"
        );

        let [ntc_code, diode_code, rolloff_code, control_code, undriven_code] =
            scan_saadc_channels::<5>(&mut bus);
        for (channel, voltage_v, code) in [
            (0, ntc_v, ntc_code),
            (1, diode_v, diode_code),
            (2, rolloff_v, rolloff_code),
            (3, control_v, control_code),
        ] {
            assert_eq!(
                code,
                expected_saadc_code(voltage_v),
                "SAADC channel {channel} converts its routed solved voltage"
            );
        }
        assert_eq!(undriven_code, 0, "neighboring AIN4 stays at ground");
        if base_drive_v == 3.3 {
            high_drive_samples = Some((rolloff_v, control_v));
        }
    }

    let (rolloff_v, control_v) = high_drive_samples.expect("recorded high-drive sample");
    assert!(
        rolloff_v - control_v > 0.05,
        "high-injection rolloff lowers collector current and raises collector voltage: {rolloff_v} V vs {control_v} V"
    );
}

#[test]
fn nrf5340dk_ntc_and_diode_netlists_reach_distinct_saadc_channels() {
    let system_path = root("configs/systems/nrf5340dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF5340-DK profile");
    assert_eq!(profile.name, "nrf5340dk");
    assert!(profile.external_devices.is_empty());
    assert_eq!(
        profile
            .board_io
            .iter()
            .map(|io| (io.id.as_str(), io.pin))
            .collect::<Vec<_>>(),
        [
            ("led1", 28),
            ("led2", 29),
            ("led3", 30),
            ("led4", 31),
            ("button1", 23),
            ("button2", 24),
            ("button3", 8),
            ("button4", 9),
        ],
        "the analog test must retain the DK's normal LED and button wiring"
    );

    let analog_path = root("examples/nrf5340dk/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF5340 descriptor");
    assert!(
        chip.analog_pins.is_empty(),
        "use explicit SAADC AIN routes rather than claiming unverified pad aliases"
    );
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF5340-DK bus");
    configure_saadc(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile carries two analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic analog device outputs must bind to SAADC AIN0 and AIN1"
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
            "SAADC AIN0 must convert the solved temperature-dependent voltage"
        );
        assert_eq!(
            diode_code,
            expected_saadc_code(diode_v),
            "SAADC AIN1 must independently convert the Shockley load line"
        );
        assert_eq!(
            undriven_code, 0,
            "the routed but undriven SAADC AIN2 channel stays at ground"
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
