// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A nonlinear diode circuit drives Arduino Uno A0, resolves through PC0's
//! descriptor entry to ADC0, and is converted by the ATmega328P's firmware
//! visible ADC registers. This exercises the shared analog netlist and ADC
//! routing on a board whose ADC is owned by its CPU rather than a normal ADC
//! peripheral.

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
