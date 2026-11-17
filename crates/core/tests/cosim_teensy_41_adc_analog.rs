// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists reach Teensy 4.1's RT1062 ADCs through the physical
//! A0/ADC1_IN7 and A12/ADC2_IN3 pad routes. This upgrades the former synthetic
//! input-bank example to firmware-visible conversions on both ADC instances.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC1: u64 = 0x400C_4000;
const ADC2: u64 = 0x400C_8000;
const HC0: u64 = 0x00;
const HS: u64 = 0x20;
const R0: u64 = 0x24;
const CFG: u64 = 0x44;
const CONVERSION_CYCLES: u64 = 6_000;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn ntc_volts(temperature_c: f64) -> f64 {
    let resistance = ntc_resistance_ohms(temperature_c);
    3.3 * resistance / (10_000.0 + resistance)
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

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn adc_code(volts: f64) -> u32 {
    let millivolts = (volts * 1000.0).round() as u32;
    (millivolts.min(3300) * 4095 + 1650) / 3300
}

fn set_12_bit_mode(bus: &mut SystemBus, base: u64) {
    let cfg = bus.read_u32(base + CFG).expect("read ADC CFG");
    assert_eq!(cfg, 0x200, "RT106x ADC CFG reset value");
    bus.write_u32(base + CFG, (cfg & !(3 << 2)) | (2 << 2))
        .expect("select 12-bit conversion");
}

fn read_sample(bus: &mut SystemBus, base: u64, channel: u32) -> u32 {
    bus.write_u32(base + HC0, channel)
        .expect("software-trigger HC0 conversion");
    assert_eq!(bus.read_u32(base + HS).unwrap() & 1, 0, "COCO clears");
    bus.set_current_cycle(bus.current_cycle + CONVERSION_CYCLES);
    assert_eq!(bus.read_u32(base + HS).unwrap() & 1, 1, "COCO sets");
    bus.read_u32(base + R0).expect("read result and clear COCO")
}

#[test]
fn teensy_41_a0_and_a12_route_ntc_and_diode_to_both_adcs() {
    let system_path = root("configs/systems/teensy-41.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Teensy 4.1");
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "led_d13" && io.peripheral == "gpio2" && io.pin == 3));

    let analog = SystemManifest::from_file(root("examples/teensy-41/analog-ntc-diode-system.yaml"))
        .expect("load generic NTC and diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    profile.chip = chip_path.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(&chip_path).expect("load RT1064-class descriptor");
    assert_eq!(chip.analog_pins["AD_B1_02"].peripheral, "adc1");
    assert_eq!(chip.analog_pins["AD_B1_02"].channel, 7, "A0 = ADC1_IN7");
    assert_eq!(chip.analog_pins["AD_B1_14"].peripheral, "adc2");
    assert_eq!(chip.analog_pins["AD_B1_14"].channel, 3, "A12 = ADC2_IN3");

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Teensy bus");
    assert!(bus.find_peripheral_index_by_name("adc1").is_some());
    assert!(bus.find_peripheral_index_by_name("adc2").is_some());
    set_12_bit_mode(&mut bus, ADC1);
    set_12_bit_mode(&mut bus, ADC2);

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile has two analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "A0 and A12 must resolve through the RT106x pad map"
    );

    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both netlists");
        assert!(
            steps.iter().any(|step| step.model_id == "ntc_divider")
                && steps.iter().any(|step| step.model_id == "diode_load_line"),
            "both unrelated devices solve: {steps:?}"
        );
        assert!(errors.is_empty(), "route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.AD_B1_02_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.AD_B1_14_volts");
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "generic Shockley diode load line: {diode_v} V"
        );

        let ntc_code = read_sample(&mut bus, ADC1, 7);
        assert_eq!(
            ntc_code,
            adc_code(ntc_v),
            "A0 / ADC1_IN7 at {temperature_c} °C"
        );
        assert_eq!(
            read_sample(&mut bus, ADC2, 3),
            adc_code(diode_v),
            "A12 / ADC2_IN3"
        );
        assert_eq!(read_sample(&mut bus, ADC1, 6), 0, "undriven ADC1 channel");
        assert_eq!(read_sample(&mut bus, ADC2, 4), 0, "undriven ADC2 channel");
        ntc_codes.push(ntc_code);
        cycles = bus.current_cycle;
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC ADC code falls as it warms: {ntc_codes:?}"
    );
}
