// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the Arduino Nano 33 BLE's nRF52840 SAADC.

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
    const LIMIT_OHMS: f64 = 1_000.0;
    const IS_A: f64 = 2.52e-9;
    const N: f64 = 1.752;
    const RS_OHMS: f64 = 0.568;
    const VT: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
    let current = |junction_v: f64| IS_A * ((junction_v / (N * VT)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let diode_current = current(junction_v);
        (SUPPLY_V - junction_v - diode_current * RS_OHMS) / LIMIT_OHMS - diode_current
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
    junction_v + current(junction_v) * RS_OHMS
}

/// Independent Gummel–Poon collector load line with an optional forward Early term.
fn bjt_collector_voltage(base_drive_v: f64, vaf_v: Option<f64>) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const VAR: f64 = 75.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let base_width = 1.0 - vaf_v.map_or(0.0, |vaf| vbc / vaf) - vbe / VAR;
        let transport = IS_A * (forward - reverse) * base_width;
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
        if (base_drive_v - vbe) / BASE_OHMS > base_current {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe((low + high) / 2.0)
}

fn voltage(session: &CosimSession, channel: usize) -> f64 {
    let path = format!("adc.saadc.{channel}_volts");
    match session.signals().get(&path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn expected_saadc_code(volts: f64) -> u16 {
    let millivolts = (volts * 1000.0).round() as u32;
    ((millivolts * 4096 / 3300).min(4095)) as u16
}

fn configure_saadc(bus: &mut SystemBus) {
    // SAADC channels 0..4 select AIN0..4; AIN4 is left undriven as an isolation check.
    for channel in 0u64..5 {
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
    bus.write_u32(SAADC + 0x630, 5)
        .expect("reserve five channel results");
    bus.write_u32(SAADC + 0x500, 1).expect("enable SAADC");
}

fn scan_saadc(bus: &mut SystemBus) -> [u16; 5] {
    bus.write_u32(SAADC + 0x104, 0)
        .expect("clear prior END event");
    bus.write_u32(SAADC, 1).expect("arm SAADC EasyDMA scan");
    bus.write_u32(SAADC + 4, 1)
        .expect("sample configured SAADC channels");
    bus.tick_peripherals_fully();

    assert_eq!(bus.read_u32(SAADC + 0x104).unwrap(), 1, "scan sets END");
    assert_eq!(
        bus.read_u32(SAADC + 0x634).unwrap(),
        5,
        "scan writes all five results"
    );
    std::array::from_fn(|channel| bus.read_u16(RESULT + channel as u64 * 2).unwrap())
}

#[test]
fn arduino_nano_33_ble_bjt_early_effect_reaches_saadc_with_unrelated_devices() {
    let system_path = root("configs/systems/arduino-nano-33-ble.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Nano 33 BLE profile");
    assert_eq!(profile.name, "arduino-nano-33-ble");
    let analog_path = root("examples/arduino-nano-33-ble/analog-bjt-early-effect-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic analog netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF52840 descriptor");
    assert!(
        chip.analog_pins.is_empty(),
        "the test uses the profile's explicit SAADC channel endpoints"
    );
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Nano 33 BLE bus");
    configure_saadc(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to SAADC channels 0 through 3"
    );

    const NTC_TEMPERATURE_C: f64 = 25.0;
    session
        .set_signal_number("ui.ntc.temperature_c", NTC_TEMPERATURE_C)
        .expect("hold the unrelated NTC at nominal temperature");
    session
        .set_signal_number("ui.bjt_early.base_v", 0.0)
        .expect("initialize both BJT base-drive sources");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize all independent analog branches");
    assert_eq!(initial_steps.len(), 4, "all four device netlists solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );

    let ntc_r = ntc_resistance_ohms(NTC_TEMPERATURE_C);
    let expected_ntc_v = 3.3 * ntc_r / (ntc_r + 10_000.0);
    let expected_diode_v = diode_load_line_voltage();
    let mut early_effect_deltas = Vec::new();
    let mut unrelated_codes = None;
    for base_drive_v in [0.70_f64, 0.78, 0.86] {
        session
            .set_signal_number("ui.bjt_early.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and BJT branches");
        assert_eq!(steps.len(), 4, "all four branches solve at each drive");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let voltages = std::array::from_fn::<_, 4, _>(|channel| voltage(&session, channel));
        assert!(
            (voltages[0] - expected_ntc_v).abs() < 1e-9,
            "unrelated Beta NTC must match its divider reference {expected_ntc_v} V, got {} V",
            voltages[0]
        );
        assert!(
            (voltages[1] - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley diode must match its load line {expected_diode_v} V, got {} V",
            voltages[1]
        );

        let expected_early_v = bjt_collector_voltage(base_drive_v, Some(50.0));
        let expected_control_v = bjt_collector_voltage(base_drive_v, None);
        assert!(
            (voltages[2] - expected_early_v).abs() < 5e-5,
            "VAF=50 branch must match independent load line {expected_early_v} V, got {} V",
            voltages[2]
        );
        assert!(
            (voltages[3] - expected_control_v).abs() < 5e-5,
            "no-forward-Early control must match independent load line {expected_control_v} V, got {} V",
            voltages[3]
        );
        assert!(
            voltages[2] < voltages[3] - 0.001,
            "finite VAF increases collector current: {} V vs {} V",
            voltages[2],
            voltages[3]
        );
        early_effect_deltas.push(voltages[3] - voltages[2]);

        let codes = scan_saadc(&mut bus);
        for (channel, voltage) in voltages.iter().copied().enumerate() {
            assert_eq!(
                codes[channel],
                expected_saadc_code(voltage),
                "SAADC channel {channel} must convert the independently routed voltage"
            );
        }
        assert_eq!(codes[4], 0, "undriven SAADC AIN4 must stay at ground");
        match unrelated_codes {
            Some((ntc_code, diode_code)) => {
                assert_eq!(codes[0], ntc_code, "BJT drive must not disturb NTC routing");
                assert_eq!(
                    codes[1], diode_code,
                    "BJT drive must not disturb diode routing"
                );
            }
            None => unrelated_codes = Some((codes[0], codes[1])),
        }
    }

    assert!(
        early_effect_deltas.iter().any(|delta| *delta > 0.02),
        "finite VAF must measurably shift the collector load line: {early_effect_deltas:?}"
    );
}
