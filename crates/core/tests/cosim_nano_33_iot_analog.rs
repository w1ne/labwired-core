// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists route through the Nano 33 IoT SAM D21 pads to ADC.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::sam::adc21::Samd21Adc;
use labwired_core::Bus;
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

fn voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(value)) => *value,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn expected_code(voltage_v: f64, bits: u32, reference_mv: u32) -> u16 {
    let millivolts = (voltage_v * 1000.0 + 0.5).floor() as u32;
    ((millivolts * ((1 << bits) - 1) / reference_mv).min((1 << bits) - 1)) as u16
}

fn enable_pad_b(bus: &mut SystemBus, port: u64, pin: u8) {
    bus.write_u8(port + 0x30 + u64::from(pin / 2), 1 << (4 * (pin & 1)))
        .expect("select PORT peripheral function B");
    bus.write_u8(port + 0x40 + u64::from(pin), 1)
        .expect("enable peripheral mux on ADC input pad");
}

fn convert(bus: &mut SystemBus, channel: u8, resolution_select: u16) -> u16 {
    bus.write_u32(INPUTCTRL, 0x1800 | u32::from(channel))
        .expect("select one external AIN against GND");
    bus.write_u16(CTRLB, resolution_select)
        .expect("select right-adjusted ADC resolution");
    bus.write_u8(SWTRIG, 1 << 1)
        .expect("start software conversion");
    bus.tick_peripherals_fully();

    assert_ne!(
        bus.read_u8(INTFLAG).expect("read INTFLAG") & 1,
        0,
        "conversion sets RESRDY"
    );
    bus.read_u16(RESULT).expect("read RESULT")
}

#[test]
fn nano_33_iot_ntc_and_diode_netlists_reach_distinct_adc_pads() {
    let system_path = root("configs/systems/nano-33-iot.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Nano 33 IoT profile");
    assert_eq!(profile.name, "nano-33-iot");
    assert!(
        profile.external_devices.is_empty(),
        "the analog test retains the board profile's declared devices"
    );
    let analog =
        SystemManifest::from_file(root("examples/nano-33-iot/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC and diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load Nano 33 IoT SAMD21 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Nano 33 IoT bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains generic analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "PA02/AIN0 and PB08/AIN2 must resolve through the chip analog pin map"
    );

    bus.write_u8(REFCTRL, 1).expect("select INTVCC0 reference");
    bus.write_u8(CTRLA, 1 << 1).expect("enable ADC");

    let mut cycles = 0;
    let mut samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature input");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both device netlists");
        assert_eq!(steps.len(), 2, "both independent analog devices solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = voltage(&session, "board.analog.pa02_volts");
        let expected_ntc_v = 3.3 * 2_200.0 / (ntc_resistance_ohms(temperature_c) + 2_200.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the Beta NTC divider at {temperature_c} °C should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = voltage(&session, "board.analog.pb08_volts");
        let expected_diode_v = diode_load_line_voltage();
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let adc_index = bus
            .find_peripheral_index_by_name("adc")
            .expect("SAMD21 ADC is present");
        let adc = bus.peripherals[adc_index]
            .dev
            .as_any()
            .expect("ADC downcasts")
            .downcast_ref::<Samd21Adc>()
            .expect("the board ADC uses the shared Samd21Adc model");
        assert_eq!(
            adc.channel_input_mv(0),
            Some((ntc_v * 1000.0 + 0.5).floor() as u16),
            "the NTC netlist seeds AIN0"
        );
        assert_eq!(
            adc.channel_input_mv(2),
            Some((diode_v * 1000.0 + 0.5).floor() as u16),
            "the unrelated diode netlist seeds AIN2"
        );
        assert_eq!(
            adc.channel_input_mv(3),
            None,
            "neighboring AIN3 is undriven"
        );

        if samples.is_empty() {
            assert_eq!(
                convert(&mut bus, 0, 0),
                0,
                "an unmuxed PA02 pad must not reach the ADC"
            );
        }
        enable_pad_b(&mut bus, PORT_A, 2); // PA02 / AIN0
        enable_pad_b(&mut bus, PORT_B, 8); // PB08 / AIN2

        for (resolution_select, bits) in [(0, 12), (2 << 4, 10), (3 << 4, 8)] {
            let ntc_code = convert(&mut bus, 0, resolution_select);
            let diode_code = convert(&mut bus, 2, resolution_select);
            let undriven_code = convert(&mut bus, 3, resolution_select);
            let reference_mv = 3300 * 100 / 148;
            assert_eq!(
                ntc_code,
                expected_code(ntc_v, bits, reference_mv),
                "{bits}-bit AIN0 code follows the solved NTC voltage"
            );
            assert_eq!(
                diode_code,
                expected_code(diode_v, bits, reference_mv),
                "{bits}-bit AIN2 code follows the independent diode load line"
            );
            assert_eq!(undriven_code, 0, "unwired AIN3 converts to zero");
            samples.push((temperature_c, ntc_v, ntc_code));
        }
    }

    assert!(
        samples[6].1 > samples[3].1 && samples[3].1 > samples[0].1,
        "the NTC divider voltage rises as temperature rises: {samples:?}"
    );
    assert!(
        samples[6].2 > samples[3].2 && samples[3].2 > samples[0].2,
        "the ADC result tracks the temperature-dependent voltage: {samples:?}"
    );
}
