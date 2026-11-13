// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach both RZ/T2M ADC12 units
//! (`ra_adc12` at R_ADC120_BASE 0x9000_4000 and R_ADC121_BASE 0x8004_5000,
//! rzt-fsp R9A07G075.h) on the onboarding RZ/T2M RSK through the
//! descriptor's `analog_pins` map. The AN pins are dedicated inputs: ANunn is
//! unit u, channel nn. The NTC is on AN005 (unit 0) and the diode on AN115
//! (unit 1), read the way rzt-fsp `r_adc.c` does:
//! - `R_ADC_Open`: `ADCSR` = `GBADIE | ADIE` without `ADST`, `ADCER` with
//!   `ADPRC` = 12 bit and right alignment, `ADADC`;
//! - `R_ADC_ScanCfg`: `ADANSA0`, `ADADS0`;
//! - `R_ADC_ScanStart`: `ADST` must read 0, then `ADCSR |= ADST`;
//! - `R_ADC_StatusGet` polls `ADST`, `R_ADC_Read` reads `ADDR[n]`.
//!
//! The reference is 1.8 V (Zephyr r9a07g075.dtsi `vref-mv = <1800>`), so the
//! -10 °C divider (2.79 V) saturates and the 25 °C one (1.65 V) does not.
//! Also: four-time addition on one unit, and the netlists moved to AN100 and
//! AN007 convert there while AN005 and AN115 go back to 0.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC120: u64 = 0x9000_4000;
const ADC121: u64 = 0x8004_5000;
const ADCSR: u64 = 0x00;
const ADANSA0: u64 = 0x04;
const ADADS0: u64 = 0x08;
const ADADC: u64 = 0x0C;
const ADCER: u64 = 0x0E;
const ADGSPCR: u64 = 0x80;
const ADDR0: u64 = 0x20;
const ADST: u16 = 1 << 15;
const ADIE: u16 = 1 << 12;
const GBADIE: u16 = 1 << 6;
/// VREFH (the profile's `vref_mv`).
const VREF_MV: u32 = 1800;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn ntc_volts(temperature_c: f64) -> f64 {
    let r = ntc_resistance_ohms(temperature_c);
    3.3 * r / (10_000.0 + r)
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn millivolts(volts: f64) -> u32 {
    (volts * 1000.0).round() as u32
}

/// 12-bit, (2^12 - 1) / VREFH, truncated, clamped.
fn code(volts: f64) -> u32 {
    (millivolts(volts) * 4095 / VREF_MV).min(4095)
}

/// The onboarding RZ/T2M RSK with the generic NTC and diode netlists routed
/// to the two pads.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/onboarding/renesas-rz_t2m_rsk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load RZ/T2M RSK");
    let analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic Beta NTC and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    for model in &mut profile.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", ntc_pad),
            "diode_load_line" => ("v_diode", diode_pad),
            other => panic!("unexpected model {other}"),
        };
        model
            .outputs
            .insert(key.to_string(), format!("board.analog.{pad}_volts"));
    }
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    profile.chip = chip_path.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(&chip_path).expect("load RZ/T2M descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build RZ/T2M RSK bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to ADC12 channels through analog_pins"
    );
    (bus, session)
}

/// Move the NTC and solve both netlists to the next boundary.
fn settle(session: &mut CosimSession, bus: &mut SystemBus, cycles: &mut u64, temperature_c: f64) {
    session
        .set_signal_number("ui.ntc.temperature_c", temperature_c)
        .expect("set NTC temperature");
    *cycles += session.cycles_until_boundary(*cycles);
    let (steps, errors) = session
        .advance_to(*cycles, bus)
        .expect("solve and route both netlists");
    assert_eq!(steps.len(), 2, "both netlists solve each step");
    assert!(errors.is_empty(), "route errors: {errors:?}");
}

/// `R_ADC_Open` for single scan, 12 bit, right aligned, no addition.
fn adc_open(bus: &mut SystemBus, unit: u64) {
    bus.write_u16(unit + ADCSR, GBADIE | ADIE)
        .expect("ADCSR without ADST");
    bus.write_u16(unit + ADCER, 0).expect("ADCER 12 bit");
    bus.write_u8(unit + ADADC, 0).expect("ADADC");
}

/// `R_ADC_ScanCfg`, `R_ADC_ScanStart`, poll `R_ADC_StatusGet`, then
/// `R_ADC_Read` for each channel.
fn adc_scan(bus: &mut SystemBus, unit: u64, channels: &[u8]) -> Vec<u32> {
    let mask = channels.iter().fold(0u16, |m, &ch| m | (1 << ch));
    bus.write_u16(unit + ADANSA0, mask).expect("ADANSA0");
    assert_eq!(bus.read_u16(unit + ADGSPCR).unwrap(), 0, "ADGSPCR reset");
    let adcsr = bus.read_u16(unit + ADCSR).unwrap();
    assert_eq!(adcsr & ADST, 0, "R_ADC_ScanStart: not in use");
    bus.write_u16(unit + ADCSR, adcsr | ADST)
        .expect("ADCSR.ADST");
    let mut polls = 0;
    while bus.read_u16(unit + ADCSR).unwrap() & ADST != 0 {
        polls += 1;
        assert!(polls < 1_000, "scan never ends");
        bus.set_current_cycle(bus.current_cycle + 100);
        bus.tick_peripherals_fully();
    }
    // 1 us per channel at the 800 MHz clock_hz.
    assert!(
        polls >= 8 * channels.len() as u32 - 1,
        "the scan takes time: {polls} polls"
    );
    channels
        .iter()
        .map(|&ch| u32::from(bus.read_u16(unit + ADDR0 + 2 * u64::from(ch)).unwrap()))
        .collect()
}

#[test]
fn rzt2m_an005_and_an115_convert_ntc_and_diode_on_both_adc12_units() {
    let (mut bus, mut session) = board("AN005", "AN115");
    adc_open(&mut bus, ADC120);
    adc_open(&mut bus, ADC121);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [25.0_f64, 50.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.AN005_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.AN115_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let unit0 = adc_scan(&mut bus, ADC120, &[5, 0, 7]);
        assert_eq!(unit0[0], code(ntc_v), "AN005 at {temperature_c} °C");
        assert!(unit0[0] < 4095, "below full scale at {temperature_c} °C");
        assert_eq!(unit0[1..], [0, 0], "AN000 and AN007 are not driven");
        let unit1 = adc_scan(&mut bus, ADC121, &[15, 5]);
        assert_eq!(unit1[0], code(diode_v), "AN115");
        assert_eq!(unit1[1], 0, "AN105 is not driven: units do not alias");
        ntc_codes.push(unit0[0]);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    // Firmware scales the result back: code * VREFH / 4095.
    let mv = ntc_codes[0] * VREF_MV / 4095;
    assert!(
        mv.abs_diff(millivolts(ntc_volts(25.0))) <= 1,
        "code * VREFH / 4095 gives {mv} mV"
    );
    // At -10 °C the divider (about 2.8 V) is above the 1.8 V VREFH.
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    assert_eq!(
        adc_scan(&mut bus, ADC120, &[5]),
        [4095],
        "over-range clamps"
    );
}

#[test]
fn rzt2m_four_time_addition_sums_the_diode_samples() {
    let (mut bus, mut session) = board("AN005", "AN115");
    adc_open(&mut bus, ADC121);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let diode = code(signal_voltage(&session, "board.analog.AN115_volts"));
    // ADC_ADD_FOUR_TIME: ADADC.ADC = 3, AVEE = 0, channel in ADADS0.
    bus.write_u8(ADC121 + ADADC, 3).expect("ADADC x4");
    bus.write_u16(ADC121 + ADADS0, 1 << 15)
        .expect("ADADS0 AN115");
    assert_eq!(adc_scan(&mut bus, ADC121, &[15]), [4 * diode], "sum");
    // ADC_ADD_AVERAGE_FOUR: AVEE = 1 returns the single-sample code.
    bus.write_u8(ADC121 + ADADC, 0x80 | 3)
        .expect("ADADC average");
    assert_eq!(adc_scan(&mut bus, ADC121, &[15]), [diode], "average");
}

#[test]
fn rzt2m_an100_and_an007_convert_after_the_netlists_move() {
    let (mut bus, mut session) = board("AN100", "AN007");
    adc_open(&mut bus, ADC120);
    adc_open(&mut bus, ADC121);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_v = signal_voltage(&session, "board.analog.AN100_volts");
    let diode_v = signal_voltage(&session, "board.analog.AN007_volts");
    assert_eq!(
        adc_scan(&mut bus, ADC121, &[0, 15]),
        [code(ntc_v), 0],
        "AN100 driven, AN115 not"
    );
    assert_eq!(
        adc_scan(&mut bus, ADC120, &[7, 5]),
        [code(diode_v), 0],
        "AN007 driven, AN005 not"
    );
}
