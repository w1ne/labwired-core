// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices drive GPIO inputs and explicit ADC1 channels on the
//! NUCLEO-L476RG. The GPIO tests assert the solved voltages and firmware-visible
//! input levels through the chip's datasheet thresholds. The ADC test routes
//! the same unrelated Beta NTC and Shockley diode to channels 5/6 and asserts
//! firmware-visible conversions. Matched generic Gummel–Poon BJT branches
//! assert high-injection rolloff through channels 11/12 alongside the same
//! unrelated devices. A generic level-1 JFET terminal-resistance
//! test checks model-card RD/RS against explicit resistor networks on the same
//! ADC while preserving the NTC and diode routes. A separate JFET gate-charge
//! test compares CGS/CGD depletion charge against a zero-charge control through
//! the ADC alongside those unrelated devices. The descriptor has no pad-to-ADC
//! map, so these tests make no PAx alias claim.

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

/// Independent Gummel–Poon collector load line with optional forward and
/// reverse high-injection rolloff.
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

/// Independent level-1 NJF load line, including model-card drain/source
/// resistances. Gate-junction leakage is negligible at the fixture's IS=1e-30 A.
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
        let current_a = (low + high) / 2.0;
        if channel_current(current_a) > current_a {
            low = current_a;
        } else {
            high = current_a;
        }
    }
    SUPPLY_V - ((low + high) / 2.0) * LOAD_OHMS
}

/// Independent SPICE level-1 JFET gate-junction depletion charge below FC·PB.
fn jfet_gate_junction_charge(voltage_v: f64) -> f64 {
    const C0_F: f64 = 1e-9;
    const PB_V: f64 = 1.0;
    const M: f64 = 0.5;
    C0_F * PB_V / (1.0 - M) * (1.0 - (1.0 - voltage_v / PB_V).powf(1.0 - M))
}

/// Independent backward-Euler update for a gate tied through two charged
/// junctions to the fixed 3.3 V drain and source nodes.
fn expected_jfet_gate_voltage(previous_v: f64, drive_v: f64, step_s: f64) -> f64 {
    const DRIVE_OHMS: f64 = 1_000.0;
    const JUNCTION_REFERENCE_V: f64 = 3.3;
    let total_charge = |gate_v: f64| 2.0 * jfet_gate_junction_charge(gate_v - JUNCTION_REFERENCE_V);
    let previous_charge = total_charge(previous_v);
    let residual = |gate_v: f64| {
        (gate_v - drive_v) / DRIVE_OHMS + (total_charge(gate_v) - previous_charge) / step_s
    };
    let (mut low, mut high) = (drive_v.min(previous_v), drive_v.max(previous_v));
    for _ in 0..100 {
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            high = mid;
        } else {
            low = mid;
        }
    }
    0.5 * (low + high)
}

/// Independent level-1 NMOS body-effect load line for the matched-pair
/// board route. The source is grounded, so `bulk_v` is VBS; reverse bias
/// raises threshold through the GAMMA/PHI relation.
fn mosfet_body_effect_drain_voltage(gate_v: f64, bulk_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const VTO: f64 = 1.0;
    const KP_A_PER_V2: f64 = 20e-6;
    const W_M: f64 = 100e-6;
    const L_M: f64 = 10e-6;
    const LAMBDA_PER_V: f64 = 0.02;
    const GAMMA: f64 = 0.4;
    const PHI: f64 = 0.7;

    let threshold = VTO + GAMMA * ((PHI - bulk_v).sqrt() - PHI.sqrt());
    let overdrive = gate_v - threshold;
    if overdrive <= 0.0 {
        return SUPPLY_V;
    }
    let beta = KP_A_PER_V2 * W_M / L_M;
    let drain_current = |vds: f64| {
        if vds < overdrive {
            beta * (1.0 + LAMBDA_PER_V * vds) * vds * (overdrive - 0.5 * vds)
        } else {
            0.5 * beta * (1.0 + LAMBDA_PER_V * vds) * overdrive * overdrive
        }
    };
    let residual = |vds: f64| SUPPLY_V - vds - LOAD_OHMS * drain_current(vds);
    let (mut low, mut high) = (0.0, SUPPLY_V);
    for _ in 0..100 {
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
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
fn nucleo_l476rg_jfet_terminal_resistances_reach_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-l476rg.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L476RG profile");
    assert_eq!(profile.name, "nucleo-l476rg");
    let base = SystemManifest::from_file(root("examples/nucleo-l476rg/analog-ntc-diode-adc.yaml"))
        .expect("load unrelated generic NTC and diode ADC routes");
    let jfet = SystemManifest::from_file(root(
        "examples/nucleo-l476rg/analog-jfet-terminal-resistance-adc.yaml",
    ))
    .expect("load matched generic level-1 JFET branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(jfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L476 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L476RG bus");
    let rcc_ahb2enr = bus.read_u32(RCC_AHB2ENR).expect("read RCC AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, rcc_ahb2enr | (1 << 13))
        .expect("enable ADC1 clock");
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
        .expect("fixture carries NTC, diode, and matched JFET branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic analog devices bind to distinct explicit ADC1 channels"
    );
    for channel in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] {
        assert_eq!(
            adc1_channel_input_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts undriven"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent Beta NTC temperature");
    session
        .set_signal_number("ui.jfet.gate_v", -1.0)
        .expect("initialize matched JFET gate drive");

    let branch_channels = [
        ("control", 0, 0.0, 0.0),
        ("drain resistance", 1, 200.0, 0.0),
        ("source resistance", 2, 0.0, 200.0),
        ("both terminal resistances", 3, 200.0, 200.0),
    ];
    let explicit_channels = [
        ("explicit drain resistance", 8, 200.0, 0.0),
        ("explicit source resistance", 9, 0.0, 200.0),
        ("explicit drain and source resistances", 10, 200.0, 200.0),
    ];
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut settled_jfet_voltages = None;
    let mut settled_explicit_voltages = None;
    for gate_v in [-1.0_f64, 0.0] {
        session
            .set_signal_number("ui.jfet.gate_v", gate_v)
            .expect("set matched JFET gate drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the NTC, diode, and JFET netlists");
        assert_eq!(steps.len(), 3, "all three unrelated analog models solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "adc.adc1.5_volts");
        assert!((ntc_v - 1.65).abs() < 1e-9, "the 25 °C NTC stays isolated");
        let diode_v = signal_voltage(&session, "adc.adc1.6_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley diode stays on its load line"
        );

        let mut jfet_voltages = Vec::new();
        for (name, channel, drain_ohms, source_ohms) in branch_channels {
            let path = format!("adc.adc1.{channel}_volts");
            let solved_v = signal_voltage(&session, &path);
            let expected_v = jfet_drain_voltage(gate_v, drain_ohms, source_ohms);
            assert!(
                (solved_v - expected_v).abs() < 5e-5,
                "{name} at gate={gate_v} V should solve to {expected_v} V, got {solved_v} V"
            );
            let count = expected_adc_count(solved_v);
            assert_eq!(
                adc1_channel_input_count(&mut bus, channel),
                count,
                "solved {name} voltage reaches ADC1_IN{channel}"
            );
            assert_eq!(
                l476_adc_convert(&mut bus, channel),
                u32::from(count),
                "firmware-visible ADC1_IN{channel} conversion follows {name}"
            );
            jfet_voltages.push(solved_v);
        }

        let mut explicit_voltages = Vec::new();
        for (name, channel, drain_ohms, source_ohms) in explicit_channels {
            let path = format!("adc.adc1.{channel}_volts");
            let solved_v = signal_voltage(&session, &path);
            let expected_v = jfet_drain_voltage(gate_v, drain_ohms, source_ohms);
            assert!(
                (solved_v - expected_v).abs() < 5e-5,
                "{name} reference at gate={gate_v} V should solve to {expected_v} V, got {solved_v} V"
            );
            let count = expected_adc_count(solved_v);
            assert_eq!(adc1_channel_input_count(&mut bus, channel), count);
            assert_eq!(
                l476_adc_convert(&mut bus, channel),
                u32::from(count),
                "firmware-visible ADC1_IN{channel} conversion follows {name}"
            );
            explicit_voltages.push(solved_v);
        }

        for (channel, voltage_v) in [(5, ntc_v), (6, diode_v)] {
            let count = expected_adc_count(voltage_v);
            assert_eq!(adc1_channel_input_count(&mut bus, channel), count);
            assert_eq!(
                l476_adc_convert(&mut bus, channel),
                u32::from(count),
                "the unrelated device remains isolated on ADC1_IN{channel}"
            );
        }
        for channel in [4, 7, 11] {
            assert_eq!(
                adc1_channel_input_count(&mut bus, channel),
                u16::MAX,
                "neighboring ADC1_IN{channel} remains undriven"
            );
        }
        settled_jfet_voltages = Some(jfet_voltages);
        settled_explicit_voltages = Some(explicit_voltages);
    }

    let [control, drain, source, both] = settled_jfet_voltages
        .expect("the final JFET gate-drive sample was recorded")
        .try_into()
        .expect("all four model-card JFET branches were routed");
    let [explicit_drain, explicit_source, explicit_both] = settled_explicit_voltages
        .expect("the final explicit-resistor sample was recorded")
        .try_into()
        .expect("all three explicit-resistor JFET references were routed");
    assert!(
        drain > control + 0.05,
        "model-card RD lowers channel current and raises external drain voltage: {drain} V vs {control} V"
    );
    assert!(
        source > control + 0.1,
        "model-card RS raises the intrinsic source and reduces channel current: {source} V vs {control} V"
    );
    assert!(
        both > drain && both > source,
        "model-card RD and RS combine to increase the external drain voltage: {both} V, {drain} V, {source} V"
    );
    for (model_card, explicit) in [
        (drain, explicit_drain),
        (source, explicit_source),
        (both, explicit_both),
    ] {
        assert!(
            (model_card - explicit).abs() < 5e-5,
            "model-card terminal resistance must match its explicit resistor network"
        );
    }
}

#[test]
fn nucleo_l476rg_jfet_gate_depletion_charge_reaches_adc1_alongside_ntc_and_diode() {
    const STEP_S: f64 = 1e-6;
    const SUBSTEPS: usize = 10;

    let system_path = root("configs/systems/nucleo-l476rg.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L476RG profile");
    assert_eq!(profile.name, "nucleo-l476rg");
    let base = SystemManifest::from_file(root("examples/nucleo-l476rg/analog-ntc-diode-adc.yaml"))
        .expect("load unrelated generic NTC and diode ADC routes");
    let jfet = SystemManifest::from_file(root(
        "examples/nucleo-l476rg/analog-jfet-gate-charge-adc.yaml",
    ))
    .expect("load charged and zero-charge generic JFET branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(jfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L476 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L476RG bus");
    let rcc_ahb2enr = bus.read_u32(RCC_AHB2ENR).expect("read RCC AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, rcc_ahb2enr | (1 << 13))
        .expect("enable ADC1 clock");
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
        .expect("fixture carries JFET charge, NTC, and diode netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic analog devices bind to distinct explicit ADC1 channels"
    );
    for channel in 0..=7 {
        assert_eq!(
            adc1_channel_input_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    let expected_diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.jfet_gate.drive_v", 3.3)
        .expect("precharge both JFET gates to the drain/source reference");
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the unrelated NTC temperature");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle NTC, diode, and both JFET branches");
    assert_eq!(
        initial_steps.len(),
        4,
        "all four generic analog netlists solve"
    );
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );
    assert!(
        (signal_voltage(&session, "adc.adc1.0_volts") - 3.3).abs() < 1e-9,
        "charged JFET gate starts at its drain/source reference"
    );

    session
        .set_signal_number("ui.jfet_gate.drive_v", 0.0)
        .expect("step both JFET gate drives low");
    session
        .set_signal_number("ui.ntc.temperature_c", 80.0)
        .expect("heat the unrelated NTC during the JFET transient");
    let ntc_resistance = ntc_resistance_ohms(80.0);
    let expected_ntc_v = 3.3 * ntc_resistance / (10_000.0 + ntc_resistance);
    let mut expected_gate_v = 3.3;
    for step in 0..8 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route JFET charge alongside NTC and diode");
        assert_eq!(steps.len(), 4, "all four netlists solve at every step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        for _ in 0..SUBSTEPS {
            expected_gate_v =
                expected_jfet_gate_voltage(expected_gate_v, 0.0, STEP_S / SUBSTEPS as f64);
        }
        let gate_v = signal_voltage(&session, "adc.adc1.0_volts");
        let control_v = signal_voltage(&session, "adc.adc1.1_volts");
        assert!(
            (gate_v - expected_gate_v).abs() < 2e-8,
            "JFET CGS/CGD depletion charge at step {step}: expected {expected_gate_v} V, got {gate_v} V"
        );
        assert!(
            control_v.abs() < 1e-8,
            "zero-charge control follows the low drive, got {control_v} V"
        );
        assert!(
            gate_v > control_v,
            "depletion charge must delay the gate edge relative to its zero-charge control"
        );

        let ntc_v = signal_voltage(&session, "adc.adc1.5_volts");
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "unrelated NTC stays at 80 °C divider voltage: expected {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "adc.adc1.6_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley diode stays at its DC load line"
        );

        for (channel, voltage_v) in [(0, gate_v), (1, control_v), (5, ntc_v), (6, diode_v)] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc1_channel_input_count(&mut bus, channel),
                expected,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                l476_adc_convert(&mut bus, channel),
                u32::from(expected),
                "ADC1 data register reports the routed voltage on channel {channel}"
            );
        }
    }
    assert!(
        signal_voltage(&session, "adc.adc1.0_volts") < 1.0,
        "gate depletion capacitance delays the edge but eventually follows the low drive"
    );
    assert_eq!(
        adc1_channel_input_count(&mut bus, 7),
        u16::MAX,
        "neighboring ADC1_IN7 remains undriven"
    );
}

#[test]
fn nucleo_l476rg_mosfet_body_effect_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-l476rg.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L476RG profile");
    assert_eq!(profile.name, "nucleo-l476rg");
    let base = SystemManifest::from_file(root("examples/nucleo-l476rg/analog-ntc-diode-adc.yaml"))
        .expect("load unrelated generic NTC and diode ADC routes");
    let body_effect = SystemManifest::from_file(root(
        "examples/nucleo-l476rg/analog-mosfet-body-effect-adc.yaml",
    ))
    .expect("load matched generic level-1 NMOS body-bias branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(body_effect.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L476 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L476RG bus");
    let rcc_ahb2enr = bus.read_u32(RCC_AHB2ENR).expect("read RCC AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, rcc_ahb2enr | (1 << 13))
        .expect("enable ADC1 clock");
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
        .expect("fixture carries NTC, diode, and matched NMOS branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic analog netlists bind to explicit ADC1 channels 5, 6, 11, and 12"
    );
    for channel in [5, 6, 11, 12, 13] {
        assert_eq!(
            adc1_channel_input_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts undriven"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.mosfet_body.bulk_v", 0.0)
        .expect("initialize matched NMOS body bias");

    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut body_samples = Vec::new();
    for bulk_v in [0.0_f64, -0.5, -1.0, -1.5] {
        session
            .set_signal_number("ui.mosfet_body.bulk_v", bulk_v)
            .expect("set generic NMOS reverse body bias");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the NTC, diode, and matched NMOS netlists");
        assert_eq!(steps.len(), 3, "all three independent analog models solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "adc.adc1.5_volts");
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the unrelated 25 °C NTC stays at {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "adc.adc1.6_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode stays on its load line"
        );
        let reference_v = signal_voltage(&session, "adc.adc1.11_volts");
        let body_v = signal_voltage(&session, "adc.adc1.12_volts");
        let expected_reference_v = mosfet_body_effect_drain_voltage(2.5, 0.0);
        let expected_body_v = mosfet_body_effect_drain_voltage(2.5, bulk_v);
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "zero-body-bias NMOS should settle at {expected_reference_v} V, got {reference_v} V"
        );
        assert!(
            (body_v - expected_body_v).abs() < 5e-5,
            "NMOS at VBS={bulk_v} V should settle at {expected_body_v} V, got {body_v} V"
        );
        if bulk_v == 0.0 {
            assert!(
                (body_v - reference_v).abs() < 1e-9,
                "matched devices agree at zero body bias"
            );
        }

        for (channel, voltage_v) in [(5, ntc_v), (6, diode_v), (11, reference_v), (12, body_v)] {
            let expected_count = expected_adc_count(voltage_v);
            assert_eq!(
                adc1_channel_input_count(&mut bus, channel),
                expected_count,
                "solved voltage reaches ADC1_IN{channel}"
            );
            assert_eq!(
                l476_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1_IN{channel} conversion follows its analog device"
            );
        }
        assert_eq!(
            adc1_channel_input_count(&mut bus, 13),
            u16::MAX,
            "neighboring ADC1_IN13 remains undriven"
        );
        body_samples.push((bulk_v, body_v));
    }
    assert!(
        body_samples.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "reverse body bias raises threshold and drain voltage: {body_samples:?}"
    );
}

#[test]
fn nucleo_l476rg_mosfet_tox_gate_charge_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-l476rg.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L476RG profile");
    assert_eq!(profile.name, "nucleo-l476rg");
    let base = SystemManifest::from_file(root("examples/nucleo-l476rg/analog-ntc-diode-adc.yaml"))
        .expect("load unrelated generic NTC and diode ADC routes");
    let tox_charge = SystemManifest::from_file(root(
        "examples/nucleo-l476rg/analog-mosfet-tox-charge-adc.yaml",
    ))
    .expect("load matched generic NMOS TOX charge branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(tox_charge.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L476 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L476RG bus");
    let rcc_ahb2enr = bus.read_u32(RCC_AHB2ENR).expect("read RCC AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, rcc_ahb2enr | (1 << 13))
        .expect("enable ADC1 clock");
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
        .expect("fixture carries NTC, diode, and a matched NMOS TOX pair");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic analog netlists bind to explicit ADC1 channels 5, 6, 11, and 12"
    );
    for channel in [5, 6, 11, 12, 13] {
        assert_eq!(
            adc1_channel_input_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts undriven"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.mosfet_tox.gate_v", 0.0)
        .expect("initialize matched NMOS gate drives");

    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let cox_f = 3.453_133e-11 * 100e-6 * 10e-6 / 10e-9;
    // In the selected accumulation region (VGS - VTO <= -PHI), the two
    // level-1 Meyer half-capacitances combine to the full oxide capacitance
    // between gate and bulk.
    let gate_bulk_capacitance_f = cox_f;
    let resistance_ohms = 1e6;
    let step_s = 1e-6;
    let drive_v = 0.2; // remains below threshold and in the same Meyer region
    let mut expected_tox_gate_v = 0.0;
    let mut cycles = 0;

    cycles += session.cycles_until_boundary(cycles);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle the NTC, diode, and NMOS gate branches before the step");
    assert_eq!(initial_steps.len(), 3, "all three analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );
    assert!((signal_voltage(&session, "adc.adc1.11_volts")).abs() < 1e-12);
    assert!((signal_voltage(&session, "adc.adc1.12_volts")).abs() < 1e-12);

    session
        .set_signal_number("ui.mosfet_tox.gate_v", drive_v)
        .expect("step both generic NMOS gate drives");
    for step in 0..24 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the MOSFET TOX charge transient");
        assert_eq!(
            steps.len(),
            3,
            "all three analog branches solve at each step"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        expected_tox_gate_v = (resistance_ohms * gate_bulk_capacitance_f * expected_tox_gate_v
            + step_s * drive_v)
            / (resistance_ohms * gate_bulk_capacitance_f + step_s);
        let ntc_v = signal_voltage(&session, "adc.adc1.5_volts");
        let diode_v = signal_voltage(&session, "adc.adc1.6_volts");
        let no_tox_gate_v = signal_voltage(&session, "adc.adc1.11_volts");
        let tox_gate_v = signal_voltage(&session, "adc.adc1.12_volts");
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "unrelated NTC stays isolated"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley diode stays at its DC load line"
        );
        assert!(
            (no_tox_gate_v - drive_v).abs() < 1e-9,
            "without TOX the isolated gate follows its drive immediately"
        );
        assert!(
            (tox_gate_v - expected_tox_gate_v).abs() < 2e-7,
            "Meyer TOX gate-bulk RC recurrence at step {step}: expected {expected_tox_gate_v} V, got {tox_gate_v} V"
        );
        assert!(
            no_tox_gate_v > tox_gate_v,
            "TOX charge must slow the gate response relative to the no-charge control"
        );

        for (channel, volts) in [
            (5, ntc_v),
            (6, diode_v),
            (11, no_tox_gate_v),
            (12, tox_gate_v),
        ] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc1_channel_input_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                l476_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1 conversion must report channel {channel}"
            );
        }
        assert_eq!(
            adc1_channel_input_count(&mut bus, 13),
            u16::MAX,
            "neighboring ADC1_IN13 remains undriven"
        );
    }
    assert!(
        (drive_v - expected_tox_gate_v).abs() < 1e-3,
        "the TOX branch should approach its DC gate drive after repeated steps"
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

#[test]
fn nucleo_l476rg_bjt_high_injection_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-l476rg.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L476RG");
    assert_eq!(profile.name, "nucleo-l476rg");
    let base = SystemManifest::from_file(root("examples/nucleo-l476rg/analog-ntc-diode-adc.yaml"))
        .expect("load unrelated generic NTC divider and Shockley diode ADC routes");
    let high_injection = SystemManifest::from_file(root(
        "examples/nucleo-l476rg/analog-bjt-high-injection-adc.yaml",
    ))
    .expect("load matched generic Gummel–Poon high-injection branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(high_injection.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L476 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L476RG bus");
    let rcc_ahb2enr = bus.read_u32(RCC_AHB2ENR).expect("read RCC AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, rcc_ahb2enr | (1 << 13))
        .expect("enable ADC1 clock");
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
        .expect("fixture carries the unrelated NTC, diode, and matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists bind to explicit ADC1 channels 5, 6, 11, and 12"
    );
    for channel in [5, 6, 11, 12] {
        assert_eq!(
            adc1_channel_input_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts undriven"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_high_injection.base_v", 0.0)
        .expect("initialize matched BJT base-drive source");

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut high_drive_delta = None;
    for base_drive_v in [0.8_f64, 1.5, 3.3] {
        session
            .set_signal_number("ui.bjt_high_injection.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the NTC, diode, and matched BJT branches");
        assert_eq!(steps.len(), 4, "all four independent analog models solve");
        assert!(
            errors.is_empty(),
            "unexpected ADC routing errors: {errors:?}"
        );

        let ntc_v = signal_voltage(&session, "adc.adc1.5_volts");
        let diode_v = signal_voltage(&session, "adc.adc1.6_volts");
        let high_v = signal_voltage(&session, "adc.adc1.11_volts");
        let control_v = signal_voltage(&session, "adc.adc1.12_volts");
        let expected_high_v = bjt_high_injection_collector_voltage(base_drive_v, true);
        let expected_control_v = bjt_high_injection_collector_voltage(base_drive_v, false);
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the unrelated 25 °C NTC stays isolated"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode stays on its independent load line"
        );
        assert!(
            (high_v - expected_high_v).abs() < 5e-5,
            "IKF/IKR/NKF load line at base drive {base_drive_v} V should be {expected_high_v} V, got {high_v} V"
        );
        assert!(
            (control_v - expected_control_v).abs() < 5e-5,
            "matched no-rolloff load line at base drive {base_drive_v} V should be {expected_control_v} V, got {control_v} V"
        );

        for (channel, voltage_v) in [(5, ntc_v), (6, diode_v), (11, high_v), (12, control_v)] {
            let expected_count = expected_adc_count(voltage_v);
            assert_eq!(
                adc1_channel_input_count(&mut bus, channel),
                expected_count,
                "solved voltage seeds ADC1_IN{channel}"
            );
            assert_eq!(
                l476_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1_IN{channel} conversion follows its generic device"
            );
        }
        for channel in [7, 10, 13] {
            assert_eq!(
                adc1_channel_input_count(&mut bus, channel),
                u16::MAX,
                "unconnected ADC1_IN{channel} remains undriven"
            );
        }
        if base_drive_v == 3.3 {
            high_drive_delta = Some(high_v - control_v);
        }
    }

    let delta = high_drive_delta.expect("high-drive sample was recorded");
    assert!(
        delta > 0.05,
        "IKF/NKF high-injection rolloff raises collector voltage by {delta} V at high drive"
    );
}
