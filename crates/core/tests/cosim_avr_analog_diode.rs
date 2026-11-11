// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists route through Arduino Uno's PC0..PC5 descriptor
//! entries into the ATmega328P's CPU-owned ADC and firmware-visible registers.
//! The circuits include unrelated passive and semiconductor devices, so this
//! exercises shared MNA and pin-routing paths rather than a board-specific
//! analog implementation.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::avr::Avr;
use labwired_core::{Cpu, Machine, SimulationConfig};
use std::path::Path;

const SUPPLY_V: f64 = 3.3;
const LIMIT_OHMS: f64 = 1_000.0;
const DIODE_IS_A: f64 = 2.52e-9;
const DIODE_N: f64 = 1.752;
const DIODE_RS_OHMS: f64 = 0.568;
const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;

/// Solve the circuit's Shockley diode load line, including the model's series
/// resistance. This independent reference makes a default/linearized diode or
/// a wrong ADC channel fail the integration test.
fn expected_diode_voltage() -> f64 {
    let current_at_junction_voltage =
        |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let current = current_at_junction_voltage(junction_v);
        (SUPPLY_V - junction_v - current * DIODE_RS_OHMS) / LIMIT_OHMS - current
    };

    let mut low = 0.0;
    let mut high = 1.2;
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

/// Independent level-1 NMOS load line, including reverse-bias body effect.
fn expected_mosfet_body_effect_voltage(bulk_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const VTO: f64 = 1.0;
    const KP_A_PER_V2: f64 = 20e-6;
    const W_OVER_L: f64 = 10.0;
    const LAMBDA_PER_V: f64 = 0.02;
    const GAMMA: f64 = 0.4;
    const PHI: f64 = 0.7;

    let threshold = VTO + GAMMA * ((PHI - bulk_v).sqrt() - PHI.sqrt());
    let overdrive = 2.5 - threshold;
    if overdrive <= 0.0 {
        return SUPPLY_V;
    }
    let beta = KP_A_PER_V2 * W_OVER_L;
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
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent forward-active Gummel–Poon collector load line, including the
/// base resistor and the model card's reverse Early voltage. Only VAF varies
/// between the matched branches.
fn expected_bjt_collector_voltage(base_drive_v: f64, forward_early_v: Option<f64>) -> f64 {
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
        let base_width = 1.0 - forward_early_v.map_or(0.0, |vaf| vbc / vaf) - vbe / VAR;
        let transport = IS_A * (forward - reverse) * base_width;
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
        if (base_drive_v - vbe) / BASE_OHMS > base_current {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe(0.5 * (low + high))
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn firmware_adc_count(machine: &mut Machine<Avr>, channel: u8) -> u16 {
    assert!(channel <= 5, "the Uno bonds ADC0 through ADC5");
    // LDI R16, 0x40|channel; STS ADMUX, R16; LDI R17, 0xC0;
    // STS ADCSRA, R17. This selects AVcc and starts one CPU-owned conversion.
    machine.cpu.load_words(
        0,
        &[
            0xE400 | u16::from(channel),
            0x9300,
            0x007C,
            0xEC10,
            0x9310,
            0x007A,
        ],
    );
    machine.cpu.set_pc(0);
    let config = SimulationConfig::default();
    for _ in 0..4 {
        machine
            .cpu
            .step(&mut machine.bus, &[], &config)
            .expect("execute ADC setup and conversion instructions");
    }
    assert_eq!(
        machine.cpu.admux & 0x0f,
        channel,
        "the test selects ADC{channel}"
    );
    assert_eq!(
        machine.cpu.adcsra & (1 << 6),
        0,
        "ADSC clears at completion"
    );
    assert_ne!(
        machine.cpu.adcsra & (1 << 4),
        0,
        "ADIF records the conversion"
    );
    u16::from(machine.cpu.adch) << 8 | u16::from(machine.cpu.adcl)
}

#[test]
fn arduino_uno_diode_netlist_reaches_a0_and_firmware_adc0() {
    let system_path = root("examples/arduino-uno-io/analog-diode-system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load Uno board fixture");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ATmega328P descriptor");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build Uno bus");
    let mut session = CosimSession::new(&manifest.cosim_models, Path::new("."), &bus)
        .expect("build analog session")
        .expect("manifest declares a diode netlist");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the diode output must resolve through the Uno's PC0 → ADC0 mapping"
    );

    let (steps, errors) = session
        .advance_to(16, &mut bus)
        .expect("solve the diode circuit");
    assert_eq!(steps.len(), 1, "one 1 us analog boundary is crossed");
    assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

    let routed_voltage = match session.signals().get("board.analog.pc0_volts") {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("the diode voltage was not routed to A0: {other:?}"),
    };
    let expected_voltage = expected_diode_voltage();
    assert!(
        (routed_voltage - expected_voltage).abs() < 2e-5,
        "the solved nonlinear load line should be {expected_voltage} V, got {routed_voltage} V"
    );

    // LDI R16, 0x40; STS ADMUX, R16; LDI R17, 0xC0; STS ADCSRA, R17.
    // With AVcc as reference and MUX=0, the CPU converts the routed ADC0 input.
    let mut cpu = Avr::new();
    cpu.load_words(0, &[0xE400, 0x9300, 0x007C, 0xEC10, 0x9310, 0x007A]);
    let mut machine = Machine::new(cpu, bus);
    let config = SimulationConfig::default();
    for _ in 0..4 {
        machine
            .cpu
            .step(&mut machine.bus, &[], &config)
            .expect("execute ADC setup and conversion instructions");
    }

    let routed_mv = (expected_voltage * 1000.0).round() as u32;
    let expected_count = (routed_mv * 1024 / 5000).min(1023) as u16;
    let firmware_count = u16::from(machine.cpu.adch) << 8 | u16::from(machine.cpu.adcl);
    assert_eq!(
        firmware_count, expected_count,
        "ATmega328P ADC0 should convert the diode's routed voltage"
    );
    assert_eq!(machine.cpu.admux & 0x0f, 0, "the test selects ADC0 / A0");
    assert_eq!(
        machine.cpu.adcsra & (1 << 6),
        0,
        "ADSC clears at completion"
    );
    assert_ne!(
        machine.cpu.adcsra & (1 << 4),
        0,
        "ADIF records the conversion"
    );
}

#[test]
fn arduino_uno_mosfet_body_effect_pair_reaches_a1_a2_and_firmware_adc() {
    let system_path = root("examples/arduino-uno-io/analog-mosfet-body-effect-system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load Uno mixed-signal fixture");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ATmega328P descriptor");
    let expected_diode_v = expected_diode_voltage();
    let expected_reference_v = expected_mosfet_body_effect_voltage(0.0);
    let mut body_samples = Vec::new();
    for bulk_v in [0.0_f64, -0.5, -1.0, -2.0, -3.0] {
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("build Uno bus");
        let mut session = CosimSession::new(&manifest.cosim_models, Path::new("."), &bus)
            .expect("build analog session")
            .expect("manifest declares a diode and matched MOSFET pair");
        assert_eq!(
            session.binding_errors(),
            &[] as &[RoutingError],
            "the diode and MOSFET drains must resolve through PC0..PC2 to ADC0..ADC2"
        );
        session
            .set_signal_number("ui.mosfet_body.bulk_v", bulk_v)
            .expect("set the NMOS body voltage");
        let (steps, errors) = session
            .advance_to(16, &mut bus)
            .expect("solve diode and matched MOSFET load lines");
        assert_eq!(steps.len(), 2, "both analog models solve at each boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let diode_v = match session.signals().get("board.analog.pc0_volts") {
            Some(CosimSignalValue::F64(volts)) => *volts,
            other => panic!("the diode voltage was not routed to A0: {other:?}"),
        };
        let reference_v = match session.signals().get("board.analog.pc1_volts") {
            Some(CosimSignalValue::F64(volts)) => *volts,
            other => panic!("the zero-bias NMOS voltage was not routed to A1: {other:?}"),
        };
        let body_v = match session.signals().get("board.analog.pc2_volts") {
            Some(CosimSignalValue::F64(volts)) => *volts,
            other => panic!("the body-biased NMOS voltage was not routed to A2: {other:?}"),
        };
        let expected_body_v = expected_mosfet_body_effect_voltage(bulk_v);
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode should remain on its load line"
        );
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

        let mut machine = Machine::new(Avr::new(), bus);
        for (channel, voltage_v) in [
            (0, expected_diode_v),
            (1, expected_reference_v),
            (2, expected_body_v),
        ] {
            let millivolts = (voltage_v * 1000.0).round() as u32;
            let expected_count = (millivolts * 1024 / 5000).min(1023) as u16;
            assert_eq!(
                firmware_adc_count(&mut machine, channel),
                expected_count,
                "ATmega328P ADC{channel} should convert its independent analog load-line result"
            );
        }
        body_samples.push((bulk_v, body_v));
    }
    assert!(
        body_samples.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "reverse body bias raises threshold and drain voltage: {body_samples:?}"
    );
}

#[test]
fn arduino_uno_bjt_early_effect_reaches_pc2_pc3_and_firmware_adc_alongside_ntc_and_diode() {
    atmega328p_bjt_early_effect_reaches_pc2_pc3_and_firmware_adc_alongside_ntc_and_diode(
        "arduino-uno",
    );
}

#[test]
fn arduino_nano_bjt_early_effect_reaches_pc2_pc3_and_firmware_adc_alongside_ntc_and_diode() {
    atmega328p_bjt_early_effect_reaches_pc2_pc3_and_firmware_adc_alongside_ntc_and_diode(
        "arduino-nano",
    );
}

fn atmega328p_bjt_early_effect_reaches_pc2_pc3_and_firmware_adc_alongside_ntc_and_diode(
    system_name: &str,
) {
    let system_path = root(&format!("configs/systems/{system_name}.yaml"));
    let mut profile = SystemManifest::from_file(&system_path).expect("load ATmega328P profile");
    assert_eq!(profile.name, system_name);

    // Reuse the generic NTC, diode, and Gummel–Poon assets. Only their
    // descriptor endpoints change from the STM32 example's PA pins to the
    // ATmega328P's bonded PC0..PC3 / ADC0..ADC3 inputs.
    let mut analog = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic Beta NTC and Shockley diode netlists");
    analog
        .cosim_models
        .iter_mut()
        .find(|model| model.id == "ntc_divider")
        .expect("fixture carries the generic NTC divider")
        .outputs
        .insert("v_sense".into(), "board.analog.pc0_volts".into());
    analog
        .cosim_models
        .iter_mut()
        .find(|model| model.id == "diode_load_line")
        .expect("fixture carries the generic Shockley diode")
        .outputs
        .insert("v_diode".into(), "board.analog.pc1_volts".into());

    let mut bjt =
        SystemManifest::from_file(root("examples/stm32-catalog/analog-bjt-early-effect.yaml"))
            .expect("load the generic matched Gummel–Poon NPN branches");
    bjt.cosim_models
        .iter_mut()
        .find(|model| model.id == "bjt_with_forward_early_effect")
        .expect("fixture carries the finite-VAF branch")
        .outputs
        .insert("collector_voltage".into(), "board.analog.pc2_volts".into());
    bjt.cosim_models
        .iter_mut()
        .find(|model| model.id == "bjt_without_forward_early_effect")
        .expect("fixture carries the no-forward-Early control")
        .outputs
        .insert("collector_voltage".into(), "board.analog.pc3_volts".into());
    analog.cosim_models.extend(bjt.cosim_models);
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ATmega328P descriptor");
    for (pad, channel) in [("PC0", 0), ("PC1", 1), ("PC2", 2), ("PC3", 3)] {
        assert_eq!(
            chip.analog_pins[pad].channel, channel,
            "{pad} is ADC{channel}"
        );
        assert_eq!(chip.analog_pins[pad].peripheral, "adc");
    }

    let bus = SystemBus::from_config(&chip, &profile).expect("build ATmega328P board bus");
    let mut machine = Machine::new(Avr::new(), bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build analog co-simulation")
        .expect("profile carries NTC, diode, and matched BJT netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all four generic branches must resolve through PC0..PC3 to ADC0..ADC3"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("hold the independent NTC at nominal temperature");
    session
        .set_signal_number("ui.bjt_early.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    let expected_ntc_v = 3.3 * ntc_resistance_ohms(25.0) / 20_000.0;
    let expected_diode_v = expected_diode_voltage();
    let mut cycles = 0;
    let mut early_samples = Vec::new();
    let mut control_samples = Vec::new();
    for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt_early.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut machine.bus)
            .expect("solve and route NTC, diode, and matched BJT branches");
        assert_eq!(
            steps.len(),
            4,
            "all four analog branches cross the boundary"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pc0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pc1_volts");
        let early_v = signal_voltage(&session, "board.analog.pc2_volts");
        let control_v = signal_voltage(&session, "board.analog.pc3_volts");
        let expected_early_v = expected_bjt_collector_voltage(base_drive_v, Some(50.0));
        let expected_control_v = expected_bjt_collector_voltage(base_drive_v, None);

        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the unrelated nominal NTC remains at {expected_ntc_v} V, got {ntc_v} V"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(
            (early_v - expected_early_v).abs() < 5e-5,
            "VAF=50 Gummel–Poon branch at {base_drive_v} V should be {expected_early_v} V, got {early_v} V"
        );
        assert!(
            (control_v - expected_control_v).abs() < 5e-5,
            "no-forward-Early branch at {base_drive_v} V should be {expected_control_v} V, got {control_v} V"
        );
        if base_drive_v >= 0.7 {
            assert!(
                early_v < control_v - 0.001,
                "finite VAF increases collector current: {early_v} V vs {control_v} V"
            );
        }

        for (channel, voltage_v) in [(0, ntc_v), (1, diode_v), (2, early_v), (3, control_v)] {
            let millivolts = (voltage_v * 1000.0).round() as u32;
            let expected_count = (millivolts * 1024 / 5000).min(1023) as u16;
            assert_eq!(
                firmware_adc_count(&mut machine, channel),
                expected_count,
                "ATmega328P ADC{channel} must convert its independently solved analog voltage"
            );
        }
        early_samples.push(early_v);
        control_samples.push(control_v);
    }
    assert!(
        early_samples[1] > early_samples[2] && early_samples[2] > early_samples[3],
        "increasing base drive lowers the finite-VAF collector voltage: {early_samples:?}"
    );
    assert!(
        control_samples[1] > control_samples[2] && control_samples[2] > control_samples[3],
        "increasing base drive lowers the control collector voltage: {control_samples:?}"
    );
}
