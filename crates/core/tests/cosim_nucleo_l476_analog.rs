// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices drive GPIO inputs and explicit ADC1 channels on the
//! NUCLEO-L476RG. The GPIO tests assert the solved voltages and firmware-visible
//! input levels through the chip's datasheet thresholds. The ADC test routes
//! the same unrelated Beta NTC and Shockley diode to channels 5/6 and asserts
//! firmware-visible conversions; the descriptor has no pad-to-ADC map, so it
//! makes no PAx alias claim.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::adc::Adc;
use labwired_core::Bus;
use std::path::Path;

const RCC_AHB2ENR: u64 = 0x4002_104C;
const GPIOA_MODER: u64 = 0x4800_0000;
const GPIOA_IDR: u64 = 0x4800_0010;
const ADC1_BASE: u64 = 0x5004_0000;
const ADC_ISR: u64 = ADC1_BASE;
const ADC_CR: u64 = ADC1_BASE + 0x08;
const ADC_CFGR: u64 = ADC1_BASE + 0x0C;
const ADC_SQR1: u64 = ADC1_BASE + 0x30;
const ADC_DR: u64 = ADC1_BASE + 0x40;
const ADC_ADVREGEN: u32 = 1 << 28;
const ADC_ADEN: u32 = 1;
const ADC_ADSTART: u32 = 1 << 2;
const ADC_ADRDY: u32 = 1;
const ADC_EOC: u32 = 1 << 2;
const ADC_EOS: u32 = 1 << 3;

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

/// Independent forward-active Gummel–Poon load line, with optional Early effect.
fn bjt_collector_voltage(base_drive_v: f64, early_voltage_v: Option<f64>) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-15;
    const BF: f64 = 100.0;
    const BR: f64 = 4.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let early_factor = early_voltage_v.map_or(1.0, |vaf| 1.0 - vbc / vaf);
        let transport = IS_A * (forward - reverse) * early_factor;
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

fn adc1_channel_input_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("STM32L476 declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<Adc>()
        .expect("ADC1 uses the shared STM32 ADC model")
        .channel_input_count(channel)
}

fn l476_adc_convert(bus: &mut SystemBus, channel: u8) -> u32 {
    // The L476 profile captures ADCAL remaining set when no ADC kernel clock
    // is configured. Power up the modeled converter and run the regular
    // software-triggered sequence without claiming calibration behavior.
    bus.write_u32(ADC_SQR1, u32::from(channel) << 6)
        .expect("select the ADC1 regular channel");
    bus.write_u32(ADC_ISR, ADC_EOC | ADC_EOS)
        .expect("clear prior conversion flags");
    bus.write_u32(ADC_CR, ADC_ADVREGEN | ADC_ADEN | ADC_ADSTART)
        .expect("start the ADC1 conversion");
    let isr = bus.read_u32(ADC_ISR).expect("read ADC1 status");
    assert_ne!(isr & ADC_EOC, 0, "ADC1 conversion completes with EOC");
    assert_ne!(isr & ADC_EOS, 0, "ADC1 conversion completes with EOS");
    bus.read_u32(ADC_DR).expect("read firmware-visible ADC1 DR")
}

fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 4095 / 3300).min(4095) as u16
}

fn assert_profile_routes_generic_analog_inputs(
    system_rel: &str,
    expected_name: &str,
    expected_external_devices: &[&str],
) {
    let system_path = root(system_rel);
    let mut profile = SystemManifest::from_file(&system_path).expect("load L476 profile");
    assert_eq!(profile.name, expected_name);
    let external_device_ids: Vec<_> = profile
        .external_devices
        .iter()
        .map(|device| device.id.as_str())
        .collect();
    assert_eq!(
        external_device_ids, expected_external_devices,
        "the analog fixture must preserve the profile's external devices"
    );
    let analog_path = root("examples/nucleo-l476rg/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L476 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build L476 system bus");
    bus.write_u32(RCC_AHB2ENR, 1).expect("enable GPIOA clock");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both generic devices must bind to distinct L476 GPIO input pads"
    );

    let expected_diode_v = diode_load_line_voltage();
    assert!(expected_diode_v < 0.99, "diode must fall below L476 VIL");
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // retain the cold high level while inside the Schmitt band
        (80.0, false),
        (25.0, false), // retain the hot low level while inside the Schmitt band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(steps.len(), 2, "both analog netlists cross the boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "board.gpio_in.pa1");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let gpioa = bus.read_u32(GPIOA_IDR).expect("read GPIOA input data");
        assert_eq!(
            gpioa & 1 != 0,
            expected_ntc_level,
            "PA0 must follow the solved NTC voltage and retain its Schmitt state"
        );
        assert_eq!(
            gpioa & 2,
            0,
            "PA1 must read low for the diode voltage below VIL"
        );
    }
}

#[test]
fn nucleo_l476rg_profiles_route_ntc_and_diode_to_gpio_thresholds() {
    assert_profile_routes_generic_analog_inputs(
        "configs/systems/nucleo-l476rg.yaml",
        "nucleo-l476rg",
        &[],
    );
    assert_profile_routes_generic_analog_inputs(
        "configs/systems/ci-fixture-l476-readonly.yaml",
        "ci-fixture-l476-readonly",
        &[],
    );
}

#[test]
fn nucleo_l476rg_ntc_and_diode_netlists_convert_on_adc1_channels_5_6() {
    let system_path = root("configs/systems/nucleo-l476rg.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L476RG");
    assert_eq!(profile.name, "nucleo-l476rg");
    let analog =
        SystemManifest::from_file(root("examples/nucleo-l476rg/analog-ntc-diode-adc.yaml"))
            .expect("load generic NTC and diode with explicit ADC routes");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L476 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L476RG bus");
    let rcc_ahb2enr = bus.read_u32(RCC_AHB2ENR).expect("read RCC AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, rcc_ahb2enr | (1 << 13))
        .expect("enable ADC1 clock");

    assert_eq!(bus.read_u32(ADC_CR).unwrap(), 1 << 29, "DEEPPWD reset");
    bus.write_u32(ADC_CR, 0).expect("clear ADC deep power-down");
    bus.write_u32(ADC_CR, ADC_ADVREGEN)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC_CR, ADC_ADVREGEN | ADC_ADEN)
        .expect("enable ADC1");
    assert_ne!(
        bus.read_u32(ADC_ISR).unwrap() & ADC_ADRDY,
        0,
        "ADC1 reports ready"
    );
    bus.write_u32(ADC_CFGR, 0)
        .expect("select 12-bit right-aligned conversion");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists bind to explicit ADC1 channels 5 and 6"
    );
    for channel in [5, 6] {
        assert_eq!(
            adc1_channel_input_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts undriven"
        );
    }
    assert_eq!(
        adc1_channel_input_count(&mut bus, 7),
        u16::MAX,
        "neighboring ADC1 channel 7 starts undriven"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the NTC and diode netlists");
        assert_eq!(steps.len(), 2, "both unrelated netlists solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "adc.adc1.5_volts");
        let ntc_resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * ntc_resistance / (10_000.0 + ntc_resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should solve to {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "adc.adc1.6_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley load line should solve to {expected_diode_v} V, got {diode_v} V"
        );
        assert_eq!(
            adc1_channel_input_count(&mut bus, 5),
            expected_adc_count(ntc_v),
            "the solved NTC level reaches ADC1_IN5"
        );
        assert_eq!(
            adc1_channel_input_count(&mut bus, 6),
            expected_adc_count(diode_v),
            "the solved diode level reaches ADC1_IN6"
        );
        assert_eq!(
            adc1_channel_input_count(&mut bus, 7),
            u16::MAX,
            "the independent routes leave ADC1_IN7 undriven"
        );

        let ntc_code = l476_adc_convert(&mut bus, 5);
        assert_eq!(ntc_code, u32::from(expected_adc_count(ntc_v)));
        let diode_code = l476_adc_convert(&mut bus, 6);
        assert_eq!(diode_code, u32::from(expected_adc_count(diode_v)));
        assert_eq!(
            l476_adc_convert(&mut bus, 7),
            3723,
            "undriven IN7 retains the modeled fixed 3.0 V internal input"
        );
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "NTC ADC counts fall as it warms: {ntc_codes:?}"
    );
}

#[test]
fn iolink_multiport_demo_routes_generic_ntc_and_diode_to_gpio_thresholds() {
    assert_profile_routes_generic_analog_inputs(
        "configs/systems/iolink-multiport-demo.yaml",
        "iolink-multiport-demo",
        &["iolink_master_4p"],
    );
}

#[test]
fn nucleo_l476rg_bjt_early_effect_reaches_pa2_pa3_gpio_inputs() {
    let system_path = root("configs/systems/nucleo-l476rg.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L476RG profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-l476rg/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC divider and Shockley diode");
    let bjt =
        SystemManifest::from_file(root("examples/nucleo-l476rg/analog-bjt-early-effect.yaml"))
            .expect("load matched generic Gummel–Poon NPN branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L476 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L476RG bus");
    for (pad, bit) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(
            SystemBus::resolve_pin_idr_pub(&bus, pad),
            Some((GPIOA_IDR, bit)),
            "{pad} must resolve to its modeled GPIOA IDR bit"
        );
    }
    bus.write_u32(RCC_AHB2ENR, 1).expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_MODER).expect("read GPIOA mode");
    bus.write_u32(GPIOA_MODER, moder & !0xFF)
        .expect("configure PA0-PA3 as digital inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched BJT models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must route independently to PA0 through PA3"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", -10.0)
        .expect("set independent NTC source");
    session
        .set_signal_number("ui.bjt.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    let vil_v = chip.io_voltage_v.unwrap() * chip.gpio_input_thresholds.as_ref().unwrap().vil;
    let vih_v = chip.io_voltage_v.unwrap() * chip.gpio_input_thresholds.as_ref().unwrap().vih;
    let expected_ntc_resistance = ntc_resistance_ohms(-10.0);
    let expected_ntc_v = 3.3 * expected_ntc_resistance / (10_000.0 + expected_ntc_resistance);
    let expected_diode_v = diode_load_line_voltage();
    let mut early_high = false;
    let mut reference_high = false;
    let mut cycles = 0;
    for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route unrelated devices and matched BJT branches");
        assert_eq!(steps.len(), 4, "all four analog branches solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let diode_v = signal_voltage(&session, "board.gpio_in.pa1");
        let early_v = signal_voltage(&session, "board.gpio_in.pa2");
        let reference_v = signal_voltage(&session, "board.gpio_in.pa3");
        let expected_early_v = bjt_collector_voltage(base_drive_v, Some(50.0));
        let expected_reference_v = bjt_collector_voltage(base_drive_v, None);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "unrelated NTC voltage should remain {expected_ntc_v} V, got {ntc_v} V"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(
            (early_v - expected_early_v).abs() < 5e-5,
            "VAF=50 collector voltage at base drive {base_drive_v} V should be {expected_early_v} V, got {early_v} V"
        );
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "collector voltage without Early effect at base drive {base_drive_v} V should be {expected_reference_v} V, got {reference_v} V"
        );
        if base_drive_v >= 0.7 {
            assert!(
                early_v < reference_v - 0.005,
                "finite VAF increases collector current and lowers collector voltage: {early_v} V vs {reference_v} V"
            );
        }

        if early_v >= vih_v {
            early_high = true;
        } else if early_v <= vil_v {
            early_high = false;
        }
        if reference_v >= vih_v {
            reference_high = true;
        } else if reference_v <= vil_v {
            reference_high = false;
        }
        let idr = bus.read_u32(GPIOA_IDR).expect("read GPIOA input levels");
        assert_ne!(idr & 1, 0, "the cold NTC remains above VIH on PA0");
        assert_eq!(idr & 2, 0, "the diode remains below VIL on PA1");
        assert_eq!(
            idr & 4 != 0,
            early_high,
            "PA2 follows the finite-VAF voltage"
        );
        assert_eq!(
            idr & 8 != 0,
            reference_high,
            "PA3 follows the no-Early-effect voltage"
        );
    }
}
