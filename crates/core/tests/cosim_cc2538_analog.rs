// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists reach the CC2538DK twice: through the mapped GPIO
//! inputs' voltage thresholds, and through the `analog_pins` pad map into the
//! SoC ADC, converted by Contiki-NG's `adc_get()` register sequence (ADCCON3
//! write, poll ADCCON1.EOC, read ADCL then ADCH).

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const GPIOC_DATA: u64 = 0x400D_B3FC;

const ADC_BASE: u64 = 0x400D_7000;
const ADCCON1: u64 = ADC_BASE;
const ADCCON3: u64 = ADC_BASE + 0x08;
const ADCL: u64 = ADC_BASE + 0x0C;
const ADCH: u64 = ADC_BASE + 0x10;
const ADCCON1_EOC: u32 = 0x80;
const ADCCON1_STSEL: u32 = 0x30;
const REF_INT: u32 = 0 << 6;
const REF_AVDD5: u32 = 2 << 6;
const DIV_128: u32 = 1 << 4;
const DIV_512: u32 = 3 << 4;
const CH_AIN4_AIN5: u32 = 0xA;
const CH_VDD_3: u32 = 0xF;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn diode_load_line_voltage(limit_ohms: f64) -> f64 {
    const SUPPLY_V: f64 = 3.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
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

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

#[test]
fn cc2538dk_ntc_and_diode_netlists_drive_gpio_inputs_through_thresholds() {
    let system_path = root("configs/systems/cc2538dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load CC2538DK profile");
    assert_eq!(profile.name, "cc2538dk");
    let analog_path = root("examples/cc2538dk/analog-ntc-diode-system.yaml");
    let analog =
        SystemManifest::from_file(&analog_path).expect("load generic NTC and diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load CC2538SF53 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build CC2538DK system bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both solved devices must bind to distinct CC2538 GPIO inputs"
    );

    let expected_diode_v = diode_load_line_voltage(10_000_000.0);
    assert!(
        expected_diode_v < 0.5,
        "diode must cross the configured VIL"
    );
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // in the hysteresis band; retain the cold high level
        (80.0, false),
        (25.0, false), // in the hysteresis band; retain the hot low level
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

        let ntc_v = signal_voltage(&session, "board.gpio_in.gpioc.4");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.0 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "board.gpio_in.gpioc.5");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let port_c = bus.read_u32(GPIOC_DATA).expect("read CC2538 GPIOC inputs");
        assert_eq!(
            port_c & (1 << 4) != 0,
            expected_ntc_level,
            "PC4 must follow the NTC voltage threshold and retain its Schmitt state at 25 °C"
        );
        assert_eq!(
            port_c & (1 << 5),
            0,
            "PC5 must read low for the diode voltage below VIL"
        );
    }
}

/// Contiki-NG `adc_get()`: start an extra conversion, poll EOC, read ADCL then
/// ADCH (the ADCH read clears EOC). Returns the signed, MSB-aligned result.
fn adc_get(bus: &mut SystemBus, channel: u32, reference: u32, div: u32) -> i16 {
    let con3 = bus.read_u32(ADCCON3).unwrap() & !0xFF;
    bus.write_u32(ADCCON3, con3 | reference | div | channel)
        .expect("start extra conversion");
    assert_ne!(
        bus.read_u32(ADCCON1).unwrap() & ADCCON1_EOC,
        0,
        "the extra conversion raises EOC"
    );
    let low = bus.read_u32(ADCL).unwrap() & 0xFC;
    let high = bus.read_u32(ADCH).unwrap();
    assert_eq!(
        bus.read_u32(ADCCON1).unwrap() & ADCCON1_EOC,
        0,
        "reading ADCH clears EOC"
    );
    (low | (high << 8)) as u16 as i16
}

/// The expected code: the routed level in whole millivolts, scaled to the
/// signed full scale and truncated toward zero, as the model's converter.
fn code(mv: i64, reference_mv: i64, bits: u32) -> i16 {
    let max = (1i64 << (bits - 1)) - 1;
    let code = (mv * max / reference_mv).clamp(-max - 1, max);
    ((code << (16 - bits)) & 0xFFFF) as u16 as i16
}

#[test]
fn cc2538dk_ntc_and_diode_netlists_convert_through_pa4_pa5_soc_adc() {
    let system_path = root("configs/systems/cc2538dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load CC2538DK profile");
    let analog =
        SystemManifest::from_file(root("examples/cc2538dk/analog-ntc-diode-adc-system.yaml"))
            .expect("load generic NTC and diode netlists routed to analog pads");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load CC2538SF53 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build CC2538DK system bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "PA4/PA5 must resolve through analog_pins to AIN4/AIN5"
    );

    // adc_init(): manual start only.
    bus.write_u32(ADCCON1, ADCCON1_STSEL).unwrap();
    assert_eq!(bus.read_u32(ADCCON1).unwrap(), ADCCON1_STSEL);

    let expected_diode_v = diode_load_line_voltage(1_000.0);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both generic analog netlists");
        assert_eq!(steps.len(), 2, "both unrelated netlists solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa4_volts");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.0 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa5_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(
            diode_v < 1.19,
            "the diode sits inside the internal reference"
        );
        let ntc_mv = (ntc_v * 1000.0).round() as i64;
        let diode_mv = (diode_v * 1000.0).round() as i64;

        // NTC on AIN4 against AVDD5 (3.0 V), 12 bits (decimation 512).
        let ntc_code = adc_get(&mut bus, 4, REF_AVDD5, DIV_512);
        assert_eq!(
            ntc_code,
            code(ntc_mv, 3000, 12),
            "AIN4 converts the NTC divider"
        );
        // Diode on AIN5 against the 1.19 V internal reference, 12 and 9 bits.
        assert_eq!(
            adc_get(&mut bus, 5, REF_INT, DIV_512),
            code(diode_mv, 1190, 12),
            "AIN5 converts the diode load line"
        );
        let coarse = adc_get(&mut bus, 5, REF_INT, DIV_128);
        assert_eq!(coarse, code(diode_mv, 1190, 9));
        assert_eq!(coarse & 0x7F, 0, "decimation 128 leaves 9 significant bits");
        // The AIN4-AIN5 pair is the signed difference of the two netlists.
        let pair = adc_get(&mut bus, CH_AIN4_AIN5, REF_AVDD5, DIV_512);
        assert_eq!(pair, code(ntc_mv - diode_mv, 3000, 12));
        assert_eq!(pair < 0, ntc_mv < diode_mv, "the pair carries its sign");
        // The unrouted AIN2 reads zero.
        assert_eq!(adc_get(&mut bus, 2, REF_AVDD5, DIV_512), 0);
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "AIN4 codes fall as the low-side NTC heats: {ntc_codes:?}"
    );
    // At 80 °C the NTC (~0.37 V) sits below the diode (~0.6 V).
    assert!(adc_get(&mut bus, CH_AIN4_AIN5, REF_AVDD5, DIV_512) < 0);

    // VDD/3 against the internal reference reads back the 3.0 V AVDD5 rail
    // through cc2538-vdd3-sensor.c's conversion.
    let raw = i32::from(adc_get(&mut bus, CH_VDD_3, REF_INT, DIV_512));
    let vdd_mv = raw * (3 * 1190) / (2047 << 4);
    assert!((2990..=3000).contains(&vdd_mv), "VDD reads {vdd_mv} mV");
}
