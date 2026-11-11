// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the STM32F7 Discovery STM32F746 profile into ADC1.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_BASE: u64 = 0x4002_3800;
const ADC1_BASE: u64 = 0x4001_2000;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line, including the diode's series resistance.
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

/// Independent Gummel–Poon NPN load-line reference with forward Early effect.
/// The two scalar roots solve base-drive current and collector-load current;
/// neither reads a voltage from the analog solver.
fn bjt_collector_voltage(base_drive_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
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
        let residual = (base_drive_v - vbe) / BASE_OHMS - base_current;
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe((low + high) / 2.0)
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("STM32F746 declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<labwired_core::peripherals::adc::Adc>()
        .expect("ADC1 uses the shared STM32 ADC model")
        .channel_input_count(channel)
}

/// Perform the selected one-shot through the same F4 register interface firmware uses.
fn stm32f4_adc_convert(bus: &mut SystemBus, channel: u8) -> u32 {
    const ADC1EN: u32 = 1 << 8;
    const ADON: u32 = 1;
    const SWSTART: u32 = 1 << 30;
    const EOC: u32 = 1 << 1;

    let apb2enr = bus.read_u32(RCC_BASE + 0x44).expect("read RCC APB2ENR");
    bus.write_u32(RCC_BASE + 0x44, apb2enr | ADC1EN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_BASE + 0x34, u32::from(channel))
        .expect("select SQR3 SQ1 channel");
    bus.write_u32(ADC1_BASE + 0x08, ADON)
        .expect("power up ADC1");
    bus.write_u32(ADC1_BASE + 0x08, ADON | SWSTART)
        .expect("software-start ADC1 conversion");
    for _ in 0..15 {
        bus.tick_peripherals_fully();
    }
    let status = bus.read_u32(ADC1_BASE).expect("read ADC1 SR");
    assert_ne!(status & EOC, 0, "ADC1 conversion completes with EOC set");
    bus.read_u32(ADC1_BASE + 0x4c)
        .expect("read firmware-visible ADC1 DR")
}

fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 4095 / 3300).min(4095) as u16
}

#[test]
fn stm32f746_discovery_ntc_diode_and_bjt_netlists_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/stm32f7-discovery.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load STM32F7 Discovery profile");
    assert_eq!(profile.name, "stm32f7-discovery");
    assert!(profile.external_devices.is_empty());
    assert!(
        profile
            .board_io
            .iter()
            .any(|io| io.id == "led_ld1" && io.peripheral == "gpioi" && io.pin == 1),
        "the mixed-signal run keeps the Discovery's LD1/PI1 mapping"
    );

    let analog = SystemManifest::from_file(root(
        "examples/stm32f7-discovery/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    let bjt = SystemManifest::from_file(root("examples/stm32f7-discovery/analog-bjt-load.yaml"))
        .expect("load generic Gummel–Poon NPN netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F746 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build STM32F7 Discovery bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and BJT devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic NTC, diode, and NPN netlists must resolve to ADC1 channels 0/1/2"
    );
    for channel in 0..=3 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} begins without an analog source"
        );
    }

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut ntc_samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        let mut collector_samples = Vec::new();
        for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
            session
                .set_signal_number("ui.bjt.base_v", base_drive_v)
                .expect("set Gummel–Poon base drive");
            cycles += session.cycles_until_boundary(cycles);
            let (steps, errors) = session
                .advance_to(cycles, &mut bus)
                .expect("solve and route all three analog netlists");
            assert_eq!(steps.len(), 3, "all devices solve at the 1 us boundary");
            assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

            let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
            let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
            assert!(
                (ntc_v - expected_ntc_v).abs() < 1e-9,
                "at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
            );
            let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
            assert!(
                (diode_v - expected_diode_v).abs() < 2e-5,
                "the Shockley diode load line should be {expected_diode_v} V, got {diode_v} V"
            );

            let collector_v = signal_voltage(&session, "board.analog.pa2_volts");
            let expected_collector_v = bjt_collector_voltage(base_drive_v);
            assert!(
                (collector_v - expected_collector_v).abs() < 5e-5,
                "Gummel–Poon NPN load line at base drive {base_drive_v} V should be \
                 {expected_collector_v} V, got {collector_v} V"
            );

            for (channel, volts) in [(0, ntc_v), (1, diode_v), (2, collector_v)] {
                let expected_count = expected_adc_count(volts);
                assert_eq!(
                    adc_channel_count(&mut bus, channel),
                    expected_count,
                    "the solved voltage must seed ADC1 channel {channel}"
                );
                assert_eq!(
                    stm32f4_adc_convert(&mut bus, channel),
                    u32::from(expected_count),
                    "firmware-visible ADC1 channel {channel} conversion must report its solved voltage"
                );
            }
            assert_eq!(
                adc_channel_count(&mut bus, 3),
                0xFFFF,
                "the neighboring ADC1 channel remains undriven"
            );
            collector_samples.push((base_drive_v, collector_v));
            ntc_samples.push((ntc_v, adc_channel_count(&mut bus, 0)));
        }

        assert!(
            collector_samples
                .windows(2)
                .all(|pair| pair[1].1 < pair[0].1),
            "increasing base drive must lower the BJT collector voltage: {collector_samples:?}"
        );
    }

    assert!(
        ntc_samples[8].0 > ntc_samples[4].0 && ntc_samples[4].0 > ntc_samples[0].0,
        "the divider voltage rises with temperature: {ntc_samples:?}"
    );
    assert!(
        ntc_samples[8].1 > ntc_samples[4].1 && ntc_samples[4].1 > ntc_samples[0].1,
        "ADC counts track the solved temperature-dependent voltage: {ntc_samples:?}"
    );
    assert_eq!(
        ntc_samples[4].1, 2047,
        "the 25 °C NTC divider remains half scale throughout the BJT sweep"
    );
}

#[test]
fn stm32f746_discovery_bjt_junction_and_transit_charge_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/stm32f7-discovery.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load STM32F7 Discovery profile");
    let generic = SystemManifest::from_file(root(
        "examples/stm32f7-discovery/analog-ntc-diode-system.yaml",
    ))
    .expect("load independent generic NTC and diode netlists");
    let charge =
        SystemManifest::from_file(root("examples/stm32f7-discovery/analog-bjt-charge.yaml"))
            .expect("load generic Gummel–Poon charge netlist");
    let charge_model = charge
        .cosim_models
        .into_iter()
        .next()
        .expect("fixture has one parameterized BJT branch");
    let base_netlist = charge_model.config["netlist_text"]
        .as_str()
        .expect("BJT fixture has inline netlist text")
        .to_owned();

    let system_chip = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&system_chip).expect("load STM32F746 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build STM32F7 Discovery bus");
    for channel in 0..=3 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    let expected_diode_v = diode_load_line_voltage();
    let mut results = Vec::new();
    for (case, parameters) in [
        ("uncharged", ""),
        ("base_emitter", "CJE=10n VJE=0.7 MJE=0.33 FC=0.5"),
        ("base_collector", "CJC=10n VJC=0.7 MJC=0.33 FC=0.5"),
        ("transit", "TF=100n"),
        (
            "combined",
            "CJE=10n VJE=0.7 MJE=0.33 CJC=10n VJC=0.7 MJC=0.33 FC=0.5 TF=100n",
        ),
    ] {
        let mut model = charge_model.clone();
        model.id = format!("bjt_charge_{case}");
        let netlist = if parameters.is_empty() {
            base_netlist.clone()
        } else {
            base_netlist.replace("VAF=50)", &format!("VAF=50 {parameters})"))
        };
        model.config.insert(
            "netlist_text".to_string(),
            serde_yaml::Value::String(netlist),
        );
        profile.cosim_models = generic.cosim_models.clone();
        profile.cosim_models.push(model);

        let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
            .expect("build analog co-simulation")
            .expect("fixture carries NTC, diode, and one BJT charge case");
        assert_eq!(
            session.binding_errors(),
            &[] as &[RoutingError],
            "{case}: NTC, diode, and BJT resolve to PA0/PA1/PA2"
        );
        session
            .set_signal_number("ui.ntc.temperature_c", 25.0)
            .expect("hold unrelated NTC at nominal temperature");
        session
            .set_signal_number("ui.bjt_charge.base_v", 0.0)
            .expect("initialize BJT base drive");
        let mut cycles = session.cycles_until_boundary(0);
        let (initial_steps, initial_errors) = session
            .advance_to(cycles, &mut bus)
            .expect("initialize NTC, diode, and BJT before the base-drive step");
        assert_eq!(initial_steps.len(), 3, "all three generic branches solve");
        assert!(
            initial_errors.is_empty(),
            "unexpected route errors: {initial_errors:?}"
        );

        session
            .set_signal_number("ui.bjt_charge.base_v", 0.8)
            .expect("step the generic BJT base drive");
        let mut first_collector_v = None;
        let mut settled_collector_v = None;
        for step in 0..8_000 {
            cycles += session.cycles_until_boundary(cycles);
            let (steps, errors) = session
                .advance_to(cycles, &mut bus)
                .expect("solve and route NTC, diode, and BJT charge branch");
            assert_eq!(steps.len(), 3, "all three branches solve at each 1 us step");
            assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

            let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
            let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
            let collector_v = signal_voltage(&session, "board.analog.pa2_volts");
            assert!(
                (ntc_v - 1.65).abs() < 1e-9,
                "the unrelated NTC stays isolated"
            );
            assert!(
                (diode_v - expected_diode_v).abs() < 2e-5,
                "the unrelated Shockley diode stays at its independent load line"
            );

            let expected_count = expected_adc_count(collector_v);
            assert_eq!(
                adc_channel_count(&mut bus, 2),
                expected_count,
                "{case}: solved collector voltage seeds ADC1_IN2"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, 2),
                u32::from(expected_count),
                "{case}: firmware-visible ADC1_IN2 follows the BJT charge response"
            );
            assert_eq!(adc_channel_count(&mut bus, 0), expected_adc_count(ntc_v));
            assert_eq!(adc_channel_count(&mut bus, 1), expected_adc_count(diode_v));
            assert_eq!(
                adc_channel_count(&mut bus, 3),
                0xFFFF,
                "neighboring ADC1_IN3 remains undriven"
            );
            if step == 0 {
                first_collector_v = Some(collector_v);
            }
            if step == 7_999 {
                settled_collector_v = Some(collector_v);
            }
        }
        results.push((
            case,
            first_collector_v.expect("first transient sample was recorded"),
            settled_collector_v.expect("settled sample was recorded"),
        ));
    }

    let dc_reference = bjt_collector_voltage(0.8);
    for (case, _, settled) in &results {
        assert!(
            (settled - dc_reference).abs() < 2e-4,
            "{case}: charge must settle to the independent Gummel–Poon DC load line {dc_reference} V, got {settled} V"
        );
    }
    let control_first = results[0].1;
    for (case, first, _) in results.iter().skip(1) {
        assert!(
            (first - control_first).abs() > 0.01,
            "{case}: junction/transit charge must change the first routed transient, control={control_first} V, charged={first} V"
        );
    }
}
