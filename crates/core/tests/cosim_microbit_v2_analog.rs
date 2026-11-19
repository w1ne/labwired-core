// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA netlists drive the micro:bit V2's physical SAADC pads.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const SAADC: u64 = 0x4000_7000;
const RESULT: u64 = 0x2000_0000;
const BOARD_PULLUP_OHMS: f64 = 10_000_000.0;

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
    let limit_ohms = 1.0 / (1.0 / SERIES_LIMIT_OHMS + 1.0 / BOARD_PULLUP_OHMS);
    let current_at_junction_voltage =
        |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let current = current_at_junction_voltage(junction_v);
        (SUPPLY_V - junction_v - current * DIODE_RS_OHMS) / limit_ohms - current
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

/// Independent Gummel–Poon NPN load line, including the micro:bit edge-ring
/// 10 MΩ pull-up in parallel with the 1 kΩ collector load.
fn bjt_collector_voltage(base_drive_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BOARD_PULLUP_OHMS: f64 = 10_000_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-15;
    const BF: f64 = 100.0;
    const BR: f64 = 4.0;
    const VAF: f64 = 50.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let transport = IS_A * (forward - reverse) * (1.0 - vbc / VAF);
        let base_emitter = (IS_A / BF) * (forward - 1.0);
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        (transport - base_collector, base_emitter + base_collector)
    };

    let collector_for_vbe = |vbe: f64| {
        let (mut low, mut high) = (0.0, SUPPLY_V);
        for _ in 0..100 {
            let collector_v = 0.5 * (low + high);
            let (collector_current, _) = currents(vbe, collector_v);
            let load_current = (SUPPLY_V - collector_v) / COLLECTOR_OHMS
                + (SUPPLY_V - collector_v) / BOARD_PULLUP_OHMS;
            if load_current > collector_current {
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
        if (base_drive_v - vbe) / BASE_OHMS > base_current {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe(0.5 * (low + high))
}

fn configure_saadc(bus: &mut SystemBus) {
    // CH[0..2] sample physical AIN0..2. Gain 1/4 with VDD/4 reference gives
    // a 0..3.3 V input range; AIN2 is intentionally not driven by a netlist.
    for (channel, ain) in [(0u64, 1u32), (1, 2), (2, 3)] {
        let base = SAADC + 0x510 + channel * 0x10;
        bus.write_u32(base, ain)
            .expect("select SAADC positive input");
        bus.write_u32(base + 8, 0x1200)
            .expect("select SAADC gain and VDD/4 reference");
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
fn microbit_v2_ntc_and_diode_netlists_reach_distinct_saadc_pads() {
    let system_path = root("configs/systems/microbit-v2.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load micro:bit V2 profile");
    assert_eq!(profile.name, "microbit-v2");
    assert_eq!(
        profile
            .external_devices
            .iter()
            .map(|device| device.id.as_str())
            .collect::<Vec<_>>(),
        ["accelerometer", "magnetometer", "led_matrix"],
        "mixed-signal coverage must retain the board's normal attached devices"
    );

    let analog_path = root("examples/microbit-v2/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF52833 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build micro:bit V2 bus");
    configure_saadc(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile carries two analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "P0.02/AIN0 and P0.03/AIN1 must bind from the chip's analog pin map"
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
        let effective_pullup_ohms = 1.0 / (1.0 / 10_000.0 + 1.0 / BOARD_PULLUP_OHMS);
        let expected_ntc_v = 3.3 * ntc_resistance_ohms(temperature_c)
            / (ntc_resistance_ohms(temperature_c) + effective_pullup_ohms);
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
            "P0.02/AIN0 must convert the solved temperature-dependent NTC voltage"
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

fn scan_all(bus: &mut SystemBus) -> [u16; 8] {
    // CH[n] samples AIN n with gain 1/4 and the VDD/4 reference, as above.
    for channel in 0..8u64 {
        let base = SAADC + 0x510 + channel * 0x10;
        bus.write_u32(base, channel as u32 + 1)
            .expect("select SAADC positive input");
        bus.write_u32(base + 8, 0x1200)
            .expect("select SAADC gain and VDD/4 reference");
    }
    bus.write_u32(SAADC + 0x630, 8)
        .expect("reserve eight channel results");
    bus.write_u32(SAADC + 0x104, 0)
        .expect("clear prior END event");
    bus.write_u32(SAADC, 1).expect("arm SAADC EasyDMA scan");
    bus.write_u32(SAADC + 4, 1)
        .expect("sample configured SAADC channels");
    bus.tick_peripherals_fully();
    assert_eq!(bus.read_u32(SAADC + 0x104).unwrap(), 1, "scan sets END");
    assert_eq!(
        bus.read_u32(SAADC + 0x634).unwrap(),
        8,
        "scan writes all eight results"
    );
    std::array::from_fn(|index| bus.read_u16(RESULT + 2 * index as u64).unwrap())
}

/// SPICE `AREA` on three unrelated device kinds reaches the SAADC. Each
/// area instance must equal the same circuit built from a hand-scaled card,
/// and both must match ngspice-45 `.op` on the area netlist (`.temp 26.85`,
/// `tnom=26.85`). The area-1 ngspice values show how far each node would sit
/// if the parser dropped the area.
#[test]
fn microbit_v2_device_area_on_d_q_j_reaches_ain2_to_ain7() {
    let system_path = root("configs/systems/microbit-v2.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load micro:bit V2 profile");
    let generic =
        SystemManifest::from_file(root("examples/microbit-v2/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode routes");
    let area =
        SystemManifest::from_file(root("examples/microbit-v2/analog-device-area-system.yaml"))
            .expect("load area and hand-scaled-card netlists");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(area.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF52833 descriptor");
    for (pin, channel) in [
        ("P0.04", 2),
        ("P0.05", 3),
        ("P0.28", 4),
        ("P0.29", 5),
        ("P0.30", 6),
        ("P0.31", 7),
    ] {
        assert_eq!(
            chip.analog_pins
                .get(pin)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("saadc", channel)),
            "{pin} must resolve to SAADC AIN{channel}"
        );
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build micro:bit V2 bus");
    configure_saadc(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile carries NTC, diode and both area netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "P0.02..P0.05 and P0.28..P0.31 must bind from the chip's analog pin map"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");

    // ngspice-45 V(d), V(c), V(dj): (supply, with AREA, with area 1).
    let ngspice = [
        (
            2.0,
            [0.681_526_1, 0.758_511_1, 0.363_045_4],
            [0.763_578_8, 0.800_898_5, 0.659_417_6],
        ),
        (
            2.5,
            [0.696_701_4, 0.770_154_1, 0.466_817_9],
            [0.794_499_6, 0.818_995_8, 0.870_890_0],
        ),
        (
            3.3,
            [0.716_716_4, 0.784_774_5, 0.649_472_1],
            [0.839_165_2, 0.844_425_2, 1.307_921],
        ),
    ];
    let area_paths = [
        "board.analog.p0.04_volts",
        "board.analog.p0.05_volts",
        "board.analog.p0.28_volts",
    ];
    let scaled_paths = [
        "board.analog.p0.29_volts",
        "board.analog.p0.30_volts",
        "board.analog.p0.31_volts",
    ];
    let devices = ["diode area=4", "NPN area 3", "NJF area=2"];
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    for (supply_v, with_area, area_one) in ngspice {
        session
            .set_signal_number("ui.device_area.supply_v", supply_v)
            .expect("set shared area supply");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all four netlists");
        assert_eq!(steps.len(), 4, "NTC, diode and both area netlists solve");
        assert!(
            errors.is_empty(),
            "unexpected analog route errors: {errors:?}"
        );

        let mut expected = [0u16; 8];
        expected[0] = expected_saadc_code(voltage(&session, "board.analog.p0.02_volts"));
        let diode_v = voltage(&session, "board.analog.p0.03_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated diode load line stays at {expected_diode_v} V, got {diode_v} V"
        );
        expected[1] = expected_saadc_code(diode_v);
        for index in 0..3 {
            let area_v = voltage(&session, area_paths[index]);
            let scaled_v = voltage(&session, scaled_paths[index]);
            assert!(
                (area_v - scaled_v).abs() < 1e-9,
                "{} at {supply_v} V: area instance {area_v} V, hand-scaled card {scaled_v} V",
                devices[index]
            );
            assert!(
                (area_v - with_area[index]).abs() < 1e-4,
                "{} at {supply_v} V: ngspice {}, got {area_v}",
                devices[index],
                with_area[index]
            );
            assert!(
                (area_v - area_one[index]).abs() > 0.04,
                "{} at {supply_v} V must sit away from the area-1 node {}",
                devices[index],
                area_one[index]
            );
            expected[2 + index] = expected_saadc_code(area_v);
            expected[5 + index] = expected_saadc_code(scaled_v);
            assert!(
                expected_saadc_code(area_one[index]).abs_diff(expected[2 + index]) > 40,
                "{}: the SAADC code must show the area",
                devices[index]
            );
        }
        assert_eq!(
            scan_all(&mut bus),
            expected,
            "SAADC AIN0..AIN7 at {supply_v} V supply"
        );
    }
}

/// Generic Gummel–Poon junction and transit charge resolve through the
/// micro:bit V2 SAADC. Matched branches isolate CJE, CJC, and TF while the
/// unrelated NTC and Shockley diode remain on their own board analog pads.
#[test]
fn microbit_v2_bjt_junction_and_transit_charge_reach_saadc_with_ntc_and_diode() {
    const BASE_DRIVE_V: f64 = 0.8;

    let system_path = root("configs/systems/microbit-v2.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load micro:bit V2 profile");
    let generic =
        SystemManifest::from_file(root("examples/microbit-v2/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let charge =
        SystemManifest::from_file(root("examples/microbit-v2/analog-bjt-charge-system.yaml"))
            .expect("load matched generic Gummel–Poon charge branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(charge.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF52833 descriptor");
    for (pin, channel) in [
        ("P0.04", 2),
        ("P0.05", 3),
        ("P0.28", 4),
        ("P0.29", 5),
        ("P0.30", 6),
    ] {
        assert_eq!(
            chip.analog_pins
                .get(pin)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("saadc", channel)),
            "{pin} must resolve to SAADC AIN{channel}"
        );
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build micro:bit V2 bus");
    configure_saadc(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal co-simulation")
        .expect("profile carries the NTC, diode, and five matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all seven generic devices must resolve through P0.02..P0.30 to SAADC"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("hold the unrelated NTC at nominal temperature");
    session
        .set_signal_number("ui.bjt_charge.base_v", 0.0)
        .expect("initialize all matched BJT base-drive sources");

    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize the independent analog branches");
    assert_eq!(initial_steps.len(), 7, "all seven netlists solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );

    session
        .set_signal_number("ui.bjt_charge.base_v", BASE_DRIVE_V)
        .expect("step the matched BJT base-drive sources");
    let ntc_resistance = ntc_resistance_ohms(25.0);
    let effective_ntc_pullup = 1.0 / (1.0 / 10_000.0 + 1.0 / BOARD_PULLUP_OHMS);
    let expected_ntc_v = 3.3 * ntc_resistance / (ntc_resistance + effective_ntc_pullup);
    let expected_diode_v = diode_load_line_voltage();
    let expected_dc = bjt_collector_voltage(BASE_DRIVE_V);
    let mut first_bjt = None;
    let mut first_codes = None;
    let mut settled_bjt = None;

    for step in 0..8_000 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and BJT charge branches");
        assert_eq!(steps.len(), 7, "all seven netlists solve at step {step}");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = voltage(&session, "board.analog.p0.02_volts");
        let diode_v = voltage(&session, "board.analog.p0.03_volts");
        let bjt_voltages = [
            voltage(&session, "board.analog.p0.04_volts"),
            voltage(&session, "board.analog.p0.05_volts"),
            voltage(&session, "board.analog.p0.28_volts"),
            voltage(&session, "board.analog.p0.29_volts"),
            voltage(&session, "board.analog.p0.30_volts"),
        ];
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the unrelated NTC remains at {expected_ntc_v} V, got {ntc_v} V"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode remains at {expected_diode_v} V, got {diode_v} V"
        );
        assert!(
            (bjt_voltages[0] - expected_dc).abs() < 5e-5,
            "the uncharged NPN must match the independent Gummel–Poon load line {expected_dc} V, got {} V",
            bjt_voltages[0]
        );

        if step == 0 {
            for (label, charged_v) in [
                ("CJE depletion", bjt_voltages[1]),
                ("CJC depletion", bjt_voltages[2]),
                ("TF transit", bjt_voltages[3]),
                ("combined junction and transit", bjt_voltages[4]),
            ] {
                assert!(
                    (charged_v - bjt_voltages[0]).abs() > 0.01,
                    "{label} must change the first transient: control={} V, charged={charged_v} V",
                    bjt_voltages[0]
                );
            }
            let codes = scan_all(&mut bus);
            for (channel, voltage_v) in std::iter::once((0, ntc_v))
                .chain(std::iter::once((1, diode_v)))
                .chain(
                    bjt_voltages
                        .iter()
                        .enumerate()
                        .map(|(i, v)| (i as u8 + 2, *v)),
                )
            {
                assert_eq!(
                    codes[channel as usize],
                    expected_saadc_code(voltage_v),
                    "SAADC AIN{channel} must convert the first-step solved voltage"
                );
            }
            assert!(
                codes[3..7].iter().all(|code| code.abs_diff(codes[2]) > 5),
                "each BJT charge branch must remain distinguishable in SAADC codes: {codes:?}"
            );
            first_bjt = Some(bjt_voltages);
            first_codes = Some(codes);
        }

        if step == 7_999 {
            let codes = scan_all(&mut bus);
            for (channel, voltage_v) in std::iter::once((0, ntc_v))
                .chain(std::iter::once((1, diode_v)))
                .chain(
                    bjt_voltages
                        .iter()
                        .enumerate()
                        .map(|(i, v)| (i as u8 + 2, *v)),
                )
            {
                assert_eq!(
                    codes[channel as usize],
                    expected_saadc_code(voltage_v),
                    "settled SAADC AIN{channel} must convert its solved voltage"
                );
            }
            settled_bjt = Some(bjt_voltages);
        }
    }

    let first = first_bjt.expect("first transient sample exists");
    let first_codes = first_codes.expect("first EasyDMA scan exists");
    let settled = settled_bjt.expect("settled transient sample exists");
    for (index, label) in [
        "CJE depletion",
        "CJC depletion",
        "TF transit",
        "combined junction and transit",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            (settled[index + 1] - expected_dc).abs() < 2e-4,
            "{label} must settle to the independent DC load line {expected_dc} V, got {} V",
            settled[index + 1]
        );
        assert!(
            (first[index + 1] - settled[index + 1]).abs() > 0.01,
            "{label} transient must decay to the same DC operating point"
        );
        assert_ne!(
            first_codes[index + 1],
            first_codes[0],
            "{label} transient must remain visible through the SAADC"
        );
    }
}
