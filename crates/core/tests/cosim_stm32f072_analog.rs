// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the STM32F072B-DISCO GPIO and F0 ADC paths.
//! The tests assert GPIO thresholds and convert descriptor-routed external ADC
//! inputs through the shared STM32F0 CHSELR register map.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_BASE: u64 = 0x4002_1000;
const GPIOA_BASE: u64 = 0x4800_0000;
const GPIOA_IDR: u64 = GPIOA_BASE + 0x10;
const ADC1_BASE: u64 = 0x4001_2400;

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

fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("STM32F072 descriptor declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<labwired_core::peripherals::adc::Adc>()
        .expect("ADC1 uses the shared STM32 ADC model")
        .channel_input_count(channel)
}

/// Run one STM32F0 CHSELR-selected single conversion through ADC1 registers.
fn stm32f0_adc_convert(bus: &mut SystemBus, channel: u8) -> u32 {
    const EOC: u32 = 1 << 2;
    const EOS: u32 = 1 << 3;
    const ADEN: u32 = 1;
    const ADSTART: u32 = 1 << 2;

    bus.write_u32(ADC1_BASE + 0x28, 1 << channel)
        .expect("select F0 CHSELR channel");
    bus.write_u32(ADC1_BASE, EOC | EOS)
        .expect("clear prior F0 conversion flags");
    bus.write_u32(ADC1_BASE + 0x08, ADEN | ADSTART)
        .expect("start F0 ADC conversion");
    let status = bus.read_u32(ADC1_BASE).expect("read F0 ADC ISR");
    assert_ne!(status & EOC, 0, "F0 ADC reports EOC");
    assert_ne!(status & EOS, 0, "single-channel F0 ADC reports EOS");
    bus.read_u32(ADC1_BASE + 0x40)
        .expect("read firmware-visible F0 ADC DR")
}

fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 4095 / 3300).min(4095) as u16
}

#[test]
fn stm32f072b_disco_ntc_and_diode_netlists_reach_pa0_pa9_gpio_inputs() {
    let system_path = root("configs/systems/stm32f072b-disco.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load F072B-DISCO profile");
    assert_eq!(profile.name, "stm32f072b-disco");
    assert!(profile.external_devices.is_empty());
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "button_b1" && io.peripheral == "gpioa" && io.pin == 0));
    for (id, pin) in [
        ("led_ld3", 6),
        ("led_ld4", 8),
        ("led_ld5", 9),
        ("led_ld6", 7),
    ] {
        assert!(
            profile
                .board_io
                .iter()
                .any(|io| io.id == id && io.peripheral == "gpioc" && io.pin == pin),
            "mixed-signal fixture keeps the board mapping for {id}"
        );
    }

    let analog = SystemManifest::from_file(root(
        "examples/stm32f072b-disco/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic Beta NTC and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F072 descriptor");
    assert_eq!(chip.pins["PA0"].gpio, "gpioa");
    assert_eq!(chip.pins["PA9"].gpio, "gpioa");
    let thresholds = chip
        .gpio_input_thresholds
        .expect("F072 declares input thresholds");
    let io_voltage = chip.io_voltage_v.expect("F072 declares GPIO voltage");
    let vil_v = thresholds.vil * io_voltage;
    let vih_v = thresholds.vih * io_voltage;

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build F072B-DISCO bus");
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA0"),
        Some((GPIOA_IDR, 0))
    );
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA9"),
        Some((GPIOA_IDR, 9))
    );

    // STM32F0 GPIOA is clocked from AHBENR bit 17. Keep both routed pins in
    // input mode so the GPIO IDR is the endpoint observed by firmware.
    let ahbenr = bus.read_u32(RCC_BASE + 0x14).expect("read RCC AHBENR");
    bus.write_u32(RCC_BASE + 0x14, ahbenr | (1 << 17))
        .expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_BASE).expect("read GPIOA MODER");
    let input_mode_mask = 0b11 | (0b11 << 18);
    bus.write_u32(GPIOA_BASE, moder & !input_mode_mask)
        .expect("configure PA0 and PA9 as inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both generic analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must bind to distinct modeled GPIOA input pins"
    );

    let expected_diode_v = diode_load_line_voltage();
    assert!(
        expected_diode_v < vil_v,
        "diode load line must be below VIL"
    );
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, false),
        (25.0, false), // retain low in the Schmitt band
        (80.0, true),
        (25.0, true), // retain high in the Schmitt band
        (-10.0, false),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(steps.len(), 2, "both netlists cross the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * 10_000.0 / (resistance + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        if temperature_c == 80.0 {
            assert!(
                ntc_v >= vih_v,
                "high sample must cross VIH: {ntc_v} V < {vih_v} V"
            );
        } else if temperature_c == -10.0 {
            assert!(
                ntc_v <= vil_v,
                "cold sample must cross VIL: {ntc_v} V > {vil_v} V"
            );
        }

        let diode_v = signal_voltage(&session, "board.gpio_in.pa9");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(
            diode_v <= vil_v,
            "diode sample must cross VIL: {diode_v} V > {vil_v} V"
        );

        let gpioa_idr = bus
            .read_u32(GPIOA_IDR)
            .expect("read firmware-visible GPIOA IDR");
        assert_eq!(
            gpioa_idr & 1 != 0,
            expected_ntc_level,
            "PA0 follows the solved NTC voltage and retains threshold state"
        );
        assert_eq!(
            gpioa_idr & (1 << 9),
            0,
            "PA9 reads low for the diode load line"
        );
    }
}

#[test]
fn stm32f072b_disco_bjt_early_effect_reaches_pa2_pa3_gpio_inputs() {
    let system_path = root("configs/systems/stm32f072b-disco.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load F072B-DISCO profile");
    let generic = SystemManifest::from_file(root(
        "examples/stm32f072b-disco/analog-ntc-diode-system.yaml",
    ))
    .expect("load unrelated generic NTC divider and Shockley diode");
    let bjt = SystemManifest::from_file(root(
        "examples/stm32f072b-disco/analog-bjt-early-effect-system.yaml",
    ))
    .expect("load matched generic Gummel–Poon NPN branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F072 descriptor");
    let thresholds = chip
        .gpio_input_thresholds
        .expect("F072 declares input thresholds");
    let io_voltage = chip.io_voltage_v.expect("F072 declares GPIO voltage");
    let vil_v = thresholds.vil * io_voltage;
    let vih_v = thresholds.vih * io_voltage;

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build F072B-DISCO bus");
    for (pad, bit) in [("PA0", 0), ("PA2", 2), ("PA3", 3), ("PA9", 9)] {
        assert_eq!(
            SystemBus::resolve_pin_idr_pub(&bus, pad),
            Some((GPIOA_IDR, bit)),
            "{pad} must resolve to the modeled GPIOA IDR bit"
        );
    }

    // STM32F0 GPIOA is clocked from AHBENR bit 17. Keep all analog endpoints
    // in input mode so IDR is the observed board endpoint.
    let ahbenr = bus.read_u32(RCC_BASE + 0x14).expect("read RCC AHBENR");
    bus.write_u32(RCC_BASE + 0x14, ahbenr | (1 << 17))
        .expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_BASE).expect("read GPIOA MODER");
    let input_mode_mask = 0b11 | (0b11 << 4) | (0b11 << 6) | (0b11 << 18);
    bus.write_u32(GPIOA_BASE, moder & !input_mode_mask)
        .expect("configure PA0, PA2, PA3, and PA9 as inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched BJT models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must bind to four distinct modeled GPIOA input pins"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", -10.0)
        .expect("set independent NTC stimulus");
    session
        .set_signal_number("ui.bjt.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    let expected_ntc_v = {
        let resistance = ntc_resistance_ohms(-10.0);
        3.3 * 10_000.0 / (resistance + 10_000.0)
    };
    let expected_diode_v = diode_load_line_voltage();
    assert!(expected_ntc_v <= vil_v, "cold NTC must be below VIL");
    assert!(
        expected_diode_v <= vil_v,
        "diode load line must be below VIL"
    );

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
            .expect("solve and route unrelated and matched-device netlists");
        assert_eq!(steps.len(), 4, "all four analog branches solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let diode_v = signal_voltage(&session, "board.gpio_in.pa9");
        let early_v = signal_voltage(&session, "board.gpio_in.pa2");
        let reference_v = signal_voltage(&session, "board.gpio_in.pa3");
        let expected_early_v = bjt_collector_voltage(base_drive_v, Some(50.0));
        let expected_reference_v = bjt_collector_voltage(base_drive_v, None);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "unrelated cold NTC remains at its independent divider voltage"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains at its independent load line"
        );
        assert!(
            (early_v - expected_early_v).abs() < 5e-5,
            "VAF=50 Gummel–Poon load line at base drive {base_drive_v} V should be {expected_early_v} V, got {early_v} V"
        );
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "no-Early-effect Gummel–Poon load line at base drive {base_drive_v} V should be {expected_reference_v} V, got {reference_v} V"
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
        let idr = bus
            .read_u32(GPIOA_IDR)
            .expect("read firmware-visible GPIOA IDR");
        assert_eq!(idr & 1, 0, "cold NTC reads low at PA0");
        assert_eq!(idr & (1 << 9), 0, "the diode reads low at PA9");
        assert_eq!(
            idr & (1 << 2) != 0,
            early_high,
            "PA2 follows the Early-effect collector voltage"
        );
        assert_eq!(
            idr & (1 << 3) != 0,
            reference_high,
            "PA3 follows the no-Early-effect collector voltage"
        );
    }
}

#[test]
fn stm32f072b_disco_ntc_diode_and_bjt_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/stm32f072b-disco.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load F072B-DISCO profile");
    let generic = SystemManifest::from_file(root(
        "examples/stm32f072b-disco/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC and diode netlists");
    let bjt = SystemManifest::from_file(root(
        "examples/stm32f072b-disco/analog-bjt-early-effect-system.yaml",
    ))
    .expect("load matched generic Gummel–Poon NPN branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F072 descriptor");
    assert_eq!(
        chip.analog_pins.len(),
        16,
        "all external ADC1 pads are routed"
    );
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3), ("PC5", 15)] {
        let route = chip
            .analog_pins
            .get(pad)
            .unwrap_or_else(|| panic!("descriptor must route {pad} to ADC1"));
        assert_eq!(route.peripheral, "adc1", "{pad} resolves to ADC1");
        assert_eq!(route.channel, channel, "{pad} resolves to ADC1_IN{channel}");
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build F072B-DISCO bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched BJT models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices resolve through descriptor-routed ADC pads"
    );

    // F0 ADC register map: ISR/IER @0x00/0x04, CR @0x08, CFGR1 @0x0C,
    // CHSELR @0x28, DR @0x40. Calibrate and enable ADC1 as firmware does.
    let apb2enr = bus.read_u32(RCC_BASE + 0x18).expect("read RCC APB2ENR");
    bus.write_u32(RCC_BASE + 0x18, apb2enr | (1 << 9))
        .expect("enable ADC1 clock");
    const EOCAL: u32 = 1 << 11;
    const ADCAL: u32 = 1 << 31;
    const ADEN: u32 = 1;
    assert_eq!(bus.read_u32(ADC1_BASE + 0x08).unwrap(), 0, "F0 CR reset");
    bus.write_u32(ADC1_BASE + 0x08, ADCAL)
        .expect("calibrate F0 ADC");
    assert_eq!(
        bus.read_u32(ADC1_BASE + 0x08).unwrap() & ADCAL,
        0,
        "ADCAL self-clears"
    );
    assert_ne!(
        bus.read_u32(ADC1_BASE).unwrap() & EOCAL,
        0,
        "calibration raises EOCAL"
    );
    bus.write_u32(ADC1_BASE, EOCAL)
        .expect("clear calibration flag");
    bus.write_u32(ADC1_BASE + 0x08, ADEN)
        .expect("enable F0 ADC");
    assert_ne!(bus.read_u32(ADC1_BASE).unwrap() & 1, 0, "ADC raises ADRDY");
    bus.write_u32(ADC1_BASE + 0x0C, 0)
        .expect("select 12-bit F0 resolution");

    let ntc_voltage = 1.65;
    let diode_voltage = diode_load_line_voltage();
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set generic NTC temperature");
    session
        .set_signal_number("ui.bjt.base_v", 0.0)
        .expect("initialize matched BJT base drives");
    let mut cycles = 0;
    for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and BJT netlists");
        assert_eq!(steps.len(), 4, "all four generic netlists solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let early_voltage = bjt_collector_voltage(base_drive_v, Some(50.0));
        let reference_voltage = bjt_collector_voltage(base_drive_v, None);
        for (path, expected_v) in [
            ("board.analog.pa0_volts", ntc_voltage),
            ("board.analog.pa1_volts", diode_voltage),
            ("board.analog.pa2_volts", early_voltage),
            ("board.analog.pa3_volts", reference_voltage),
        ] {
            let solved_v = signal_voltage(&session, path);
            assert!(
                (solved_v - expected_v).abs() < 5e-5,
                "{path} should resolve to {expected_v} V, got {solved_v} V"
            );
        }
        if base_drive_v >= 0.7 {
            assert!(
                early_voltage < reference_voltage - 0.005,
                "finite VAF must lower the collector voltage: {early_voltage} V vs {reference_voltage} V"
            );
        }

        for (channel, voltage) in [
            (0, ntc_voltage),
            (1, diode_voltage),
            (2, early_voltage),
            (3, reference_voltage),
        ] {
            let expected = expected_adc_count(voltage);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "solved analog voltage seeds ADC1_IN{channel}"
            );
            assert_eq!(
                stm32f0_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1_IN{channel} conversion follows the solved device voltage"
            );
        }
        assert_eq!(
            (4..=18)
                .map(|ch| adc_channel_count(&mut bus, ch))
                .collect::<Vec<_>>(),
            vec![0xFFFF; 15],
            "undriven ADC channels stay isolated"
        );
    }
}
