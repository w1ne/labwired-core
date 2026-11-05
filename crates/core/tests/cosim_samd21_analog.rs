// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog devices route through SAM D21 pads into firmware-visible ADC results.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::sam::adc21::Samd21Adc;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Bus, Cpu, Machine};
use std::path::Path;

const ADC_BASE: u64 = 0x4200_4000;
const CTRLA: u64 = ADC_BASE;
const REFCTRL: u64 = ADC_BASE + 1;
const CTRLB: u64 = ADC_BASE + 4;
const SWTRIG: u64 = ADC_BASE + 0x0c;
const INPUTCTRL: u64 = ADC_BASE + 0x10;
const INTFLAG: u64 = ADC_BASE + 0x18;
const RESULT: u64 = ADC_BASE + 0x1a;
const PORT_A: u64 = 0x4100_4400;
const PORT_B: u64 = 0x4100_4480;

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

fn signal_number(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(value)) => *value,
        other => panic!("expected analog voltage at {path}, got {other:?}"),
    }
}

fn advance_to_analog_boundary<C: Cpu>(machine: &mut Machine<C>, session: &mut CosimSession) {
    for _ in 0..256 {
        let advance = session
            .advance(machine, AdvanceRequest::single())
            .expect("advance board and analog devices");
        assert!(
            advance.new_routing_errors.is_empty(),
            "unexpected route errors: {:?}",
            advance.new_routing_errors
        );
        if !advance.routed.is_empty() {
            return;
        }
    }
    panic!("analog devices did not reach a co-simulation boundary");
}

fn convert<C: Cpu>(
    machine: &mut Machine<C>,
    session: &mut CosimSession,
    channel: u8,
    resolution_select: u16,
) -> u16 {
    machine
        .bus
        .write_u32(INPUTCTRL, 0x1800 | u32::from(channel))
        .expect("select one external AIN against GND");
    machine
        .bus
        .write_u16(CTRLB, resolution_select)
        .expect("select right-adjusted ADC resolution");
    machine
        .bus
        .write_u8(SWTRIG, 1 << 1)
        .expect("start software conversion");

    for _ in 0..256 {
        if machine.bus.read_u8(INTFLAG).expect("read INTFLAG") & 1 != 0 {
            return machine.bus.read_u16(RESULT).expect("read RESULT");
        }
        let advance = session
            .advance(machine, AdvanceRequest::single())
            .expect("advance pending ADC conversion");
        assert!(
            advance.new_routing_errors.is_empty(),
            "unexpected ADC routing errors: {:?}",
            advance.new_routing_errors
        );
    }
    panic!("SAMD21 ADC conversion did not complete");
}

fn expected_code(voltage_v: f64, bits: u32, reference_mv: u32) -> u16 {
    let millivolts = (voltage_v * 1000.0 + 0.5).floor() as u32;
    ((millivolts * ((1 << bits) - 1) / reference_mv).min((1 << bits) - 1)) as u16
}

fn enable_pad_b<C: Cpu>(machine: &mut Machine<C>, port: u64, pin: u8) {
    machine
        .bus
        .write_u8(port + 0x30 + u64::from(pin / 2), 1 << (4 * (pin & 1)))
        .expect("select PORT peripheral function B");
    machine
        .bus
        .write_u8(port + 0x40 + u64::from(pin), 1)
        .expect("enable peripheral mux on ADC input pad");
}

#[test]
fn arduino_zero_ntc_and_diode_netlists_reach_distinct_samd21_adc_pads() {
    let system_path = root("configs/systems/arduino-zero.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Arduino Zero profile");
    assert_eq!(profile.name, "arduino-zero");
    let analog =
        SystemManifest::from_file(root("examples/arduino-zero/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC and diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ATSAMD21G18A descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Arduino Zero bus");
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    const IDLE: u32 = 0x2000_2000;
    machine
        .bus
        .write_u16(u64::from(IDLE), 0xE7FE)
        .expect("place branch-to-self instruction in SRAM");
    machine.cpu.set_pc(IDLE);
    machine.cpu.set_sp(0x2000_7f00);

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build mixed-signal session")
        .expect("profile contains generic analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "PA02/AIN0 and PB08/AIN2 must resolve through the chip analog pin map"
    );

    machine
        .bus
        .write_u8(REFCTRL, 1)
        .expect("select INTVCC0 reference");
    machine.bus.write_u8(CTRLA, 1 << 1).expect("enable ADC");

    let mut samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature input");
        advance_to_analog_boundary(&mut machine, &mut session);

        let ntc_v = signal_number(&session, "board.analog.pa02_volts");
        let expected_ntc_v = 3.3 * 2_200.0 / (ntc_resistance_ohms(temperature_c) + 2_200.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the Beta NTC divider at {temperature_c} °C should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_number(&session, "board.analog.pb08_volts");
        let expected_diode_v = diode_load_line_voltage();
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let adc_index = machine
            .bus
            .find_peripheral_index_by_name("adc")
            .expect("SAMD21 ADC is present");
        let adc = machine.bus.peripherals[adc_index]
            .dev
            .as_any()
            .expect("ADC downcasts")
            .downcast_ref::<Samd21Adc>()
            .expect("the board ADC uses Samd21Adc");
        assert_eq!(
            adc.channel_input_mv(0),
            Some((ntc_v * 1000.0 + 0.5).floor() as u16),
            "the NTC netlist must seed its AIN0 input"
        );
        assert_eq!(
            adc.channel_input_mv(2),
            Some((diode_v * 1000.0 + 0.5).floor() as u16),
            "the unrelated diode netlist must seed its AIN2 input"
        );
        assert_eq!(
            adc.channel_input_mv(3),
            None,
            "neighboring AIN3 is undriven"
        );

        // The circuit has solved and seeded AIN0, but its voltage is not
        // observable while the PA02 pad remains in GPIO mode.
        if samples.is_empty() {
            assert_eq!(
                convert(&mut machine, &mut session, 0, 0),
                0,
                "an unmuxed PA02 pad must not reach the ADC"
            );
        }

        // Correct device input values become visible once the actual pads
        // are muxed to ADC function B and PMUXEN is enabled.
        enable_pad_b(&mut machine, PORT_A, 2); // PA02 / AIN0
        enable_pad_b(&mut machine, PORT_B, 8); // PB08 / AIN2

        for (resolution_select, bits) in [(0, 12), (2 << 4, 10), (3 << 4, 8)] {
            let ntc_code = convert(&mut machine, &mut session, 0, resolution_select);
            let diode_code = convert(&mut machine, &mut session, 2, resolution_select);
            let undriven_code = convert(&mut machine, &mut session, 3, resolution_select);
            let reference_mv = 3300 * 100 / 148;
            assert_eq!(
                ntc_code,
                expected_code(ntc_v, bits, reference_mv),
                "{bits}-bit AIN0 code must follow the solved NTC voltage"
            );
            assert_eq!(
                diode_code,
                expected_code(diode_v, bits, reference_mv),
                "{bits}-bit AIN2 code must follow the independent diode load line"
            );
            assert_eq!(undriven_code, 0, "unwired AIN3 converts to zero");
            samples.push((temperature_c, ntc_v, ntc_code));
        }
    }

    assert!(
        samples[6].1 > samples[3].1 && samples[3].1 > samples[0].1,
        "the NTC divider voltage must rise as temperature rises: {samples:?}"
    );
    assert!(
        samples[6].2 > samples[3].2 && samples[3].2 > samples[0].2,
        "the ADC result must track the temperature-dependent voltage: {samples:?}"
    );
}
