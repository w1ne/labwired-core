// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA netlists drive the Seeed XIAO nRF52840 Sense's SAADC channels.

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
    // PSELP 1..7 select AIN0..6. Channels 0..3 read the JFET branches,
    // channels 4..5 read the unrelated NTC and diode, and channel 6 stays idle.
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

/// Independent level-1 NJF load line including model-card drain/source
/// resistances. The current root is solved outside the MNA engine.
fn jfet_drain_voltage(gate_v: f64, drain_ohms: f64, source_ohms: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const VTO: f64 = -2.0;
    const BETA_A_PER_V2: f64 = 500e-6;
    const LAMBDA_PER_V: f64 = 0.025;

    let channel_current = |current_a: f64| {
        let external_drain_v = SUPPLY_V - current_a * LOAD_OHMS;
        let internal_drain_v = external_drain_v - current_a * drain_ohms;
        let internal_source_v = current_a * source_ohms;
        let vds = internal_drain_v - internal_source_v;
        let vgst = gate_v - internal_source_v - VTO;
        if vgst <= 0.0 {
            0.0
        } else if vds < vgst {
            BETA_A_PER_V2 * (1.0 + LAMBDA_PER_V * vds) * vds * (2.0 * vgst - vds)
        } else {
            BETA_A_PER_V2 * (1.0 + LAMBDA_PER_V * vds) * vgst * vgst
        }
    };

    let (mut low, mut high) = (0.0, SUPPLY_V / LOAD_OHMS);
    for _ in 0..100 {
        let current_a = 0.5 * (low + high);
        if channel_current(current_a) > current_a {
            low = current_a;
        } else {
            high = current_a;
        }
    }
    SUPPLY_V - 0.5 * (low + high) * LOAD_OHMS
}

#[test]
fn seeed_xiao_nrf52840_ntc_and_diode_netlists_reach_distinct_saadc_channels() {
    let system_path = root("configs/systems/seeed-xiao-nrf52840-sense.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load Seeed XIAO nRF52840 Sense profile");
    assert_eq!(profile.name, "seeed-xiao-nrf52840-sense");
    assert!(profile.external_devices.is_empty());
    assert_eq!(
        profile
            .board_io
            .iter()
            .map(|io| (io.id.as_str(), io.pin))
            .collect::<Vec<_>>(),
        [("led_red", 26), ("led_green", 30), ("led_blue", 6)],
        "the analog test must retain the XIAO's normal RGB LED wiring"
    );

    // Reuse the generic NTC divider and unrelated Shockley diode netlists.
    // They target explicit SAADC channels; this test does not claim physical
    // XIAO pad-to-AIN aliases, which are not declared in the shared chip model.
    let analog_path = root("examples/nrf52840-dk/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF52840 descriptor");
    assert!(
        chip.analog_pins.is_empty(),
        "use the existing explicit ADC channel route without changing the shared silicon profile"
    );
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build XIAO nRF52840 bus");
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

#[test]
fn seeed_xiao_nrf52840_jfet_terminal_resistances_reach_saadc_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/seeed-xiao-nrf52840-sense.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Seeed XIAO profile");
    let mut generic =
        SystemManifest::from_file(root("examples/nrf52840-dk/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and Shockley diode netlists");
    generic.cosim_models[0]
        .outputs
        .insert("v_sense".to_string(), "adc.saadc.4_volts".to_string());
    generic.cosim_models[1]
        .outputs
        .insert("v_diode".to_string(), "adc.saadc.5_volts".to_string());
    let jfet = SystemManifest::from_file(root(
        "examples/seeed-xiao-nrf52840-sense/analog-jfet-terminal-resistance.yaml",
    ))
    .expect("load matched generic level-1 JFET branches");
    profile.cosim_models = jfet.cosim_models;
    profile.cosim_models.extend(generic.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF52840 descriptor");
    assert!(
        chip.analog_pins.is_empty(),
        "this proof uses explicit SAADC input channels, not XIAO pad aliases"
    );
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build XIAO system bus");
    configure_seven_channel_saadc(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal co-simulation")
        .expect("profile carries matched JFETs, NTC, and diode");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all six generic devices must bind to distinct SAADC channels"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut final_jfet_voltages = None;
    for gate_v in [-3.0_f64, -1.0, 0.0] {
        session
            .set_signal_number("ui.jfet.gate_v", gate_v)
            .expect("set the four matched JFET gate drives");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route JFET, NTC, and diode netlists");
        assert_eq!(steps.len(), 3, "all three independent netlists solve");
        assert!(
            errors.is_empty(),
            "unexpected SAADC route errors: {errors:?}"
        );

        let actual = [
            voltage(&session, "adc.saadc.0_volts"),
            voltage(&session, "adc.saadc.1_volts"),
            voltage(&session, "adc.saadc.2_volts"),
            voltage(&session, "adc.saadc.3_volts"),
        ];
        let cases = [
            ("control", 0.0, 0.0),
            ("RD", 200.0, 0.0),
            ("RS", 0.0, 200.0),
            ("RD+RS", 200.0, 200.0),
        ];
        let codes = scan_seven_channels(&mut bus);
        if gate_v == -3.0 {
            assert!(
                actual.iter().all(|volts| (volts - 3.3).abs() < 5e-5),
                "all matched JFETs remain cut off below threshold: {actual:?}"
            );
        }
        for (channel, (name, rd, rs)) in cases.into_iter().enumerate() {
            let expected_v = jfet_drain_voltage(gate_v, rd, rs);
            assert!(
                (actual[channel] - expected_v).abs() < 5e-5,
                "level-1 NJF {name} load line at gate={gate_v} V should be {expected_v} V, got {} V",
                actual[channel]
            );
            assert_eq!(
                codes[channel],
                expected_saadc_code(actual[channel]),
                "firmware-visible SAADC channel {channel} follows the solved {name} JFET voltage"
            );
        }

        let ntc_v = voltage(&session, "adc.saadc.4_volts");
        let diode_v = voltage(&session, "adc.saadc.5_volts");
        assert!((ntc_v - expected_ntc_v).abs() < 1e-9);
        assert!((diode_v - expected_diode_v).abs() < 2e-5);
        assert_eq!(
            codes[4],
            expected_saadc_code(ntc_v),
            "SAADC channel 4 tracks NTC"
        );
        assert_eq!(
            codes[5],
            expected_saadc_code(diode_v),
            "SAADC channel 5 tracks diode"
        );
        assert_eq!(codes[6], 0, "undriven SAADC channel 6 remains at ground");
        final_jfet_voltages = Some(actual);
    }

    let final_voltages = final_jfet_voltages.expect("final JFET sample was recorded");
    for (name, volts) in [
        ("RD", final_voltages[1]),
        ("RS", final_voltages[2]),
        ("RD+RS", final_voltages[3]),
    ] {
        assert!(
            volts > final_voltages[0] + 0.05,
            "{name} must reduce channel current and raise the external drain voltage: {volts} V vs {} V",
            final_voltages[0]
        );
    }
}
