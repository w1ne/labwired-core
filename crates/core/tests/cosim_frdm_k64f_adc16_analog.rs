// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the FRDM-K64F's two
//! ADC16 instances through the `analog_pins` map: the NTC on Arduino A0
//! (PTB2 = ADC0_SE12) and the diode on A2 (PTB10 = ADC1_SE14), converted the
//! way mcux-sdk `fsl_adc16.c` does it (Init, DoAutoCalibration, SC1A write,
//! poll COCO, read R[0]). Also:
//! - one continuous conversion (`SC3.ADCO`) started ONCE keeps following the
//!   NTC as its temperature moves, with no further `SC1A` write;
//! - the differential pair DAD2 (PTE2 = ADC0_DP2, PTE3 = ADC0_DM2) converts
//!   the signed difference of the two netlists, sign-extended in `R[0]`.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC0: u64 = 0x4003_B000;
const ADC1: u64 = 0x400B_B000;
const SC1A: u64 = 0x00;
const CFG1: u64 = 0x08;
const RA: u64 = 0x10;
const SC2: u64 = 0x20;
const SC3: u64 = 0x24;
const OFS: u64 = 0x28;
const COCO: u32 = 1 << 7;
const DIFF: u32 = 1 << 5;
const MODE_12: u32 = 1 << 2;
const MODE_16: u32 = 3 << 2;
const ADACT: u32 = 1 << 7;
const ADCO: u32 = 1 << 3;
const CAL: u32 = 1 << 7;
const CALF: u32 = 1 << 6;
const ADCH_DISABLED: u32 = 0x1F;
/// ADC0_SE12 (PTB2), ADC1_SE14 (PTB10), DAD2 (PTE2/PTE3).
const AD12: u32 = 12;
const AD14: u32 = 14;
const DAD2: u32 = 2;

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

fn millivolts(volts: f64) -> i64 {
    (volts * 1000.0).round() as i64
}

/// Single-ended N-bit code: 1 LSB = VREFH (3.3 V) / 2^N, truncated, clamped.
fn se_code(volts: f64, bits: u32) -> u32 {
    ((millivolts(volts) << bits) / 3300).clamp(0, (1 << bits) - 1) as u32
}

/// The FRDM-K64F with the generic NTC and diode netlists routed to `pads`.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/frdm-k64f.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load FRDM-K64F profile");
    let analog = SystemManifest::from_file(root("examples/frdm-k64f/analog-ntc-diode-system.yaml"))
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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load MK64F12 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build FRDM-K64F bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to ADC16 inputs through analog_pins"
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

/// fsl_adc16 ADC16_Init (12-bit, VREFH, single conversion) followed by
/// ADC16_DoAutoCalibration.
fn init_and_calibrate(bus: &mut SystemBus, base: u64, mode: u32) {
    bus.write_u32(base + CFG1, mode).expect("CFG1");
    bus.write_u32(base + SC2, 0).expect("SC2.REFSEL = VREFH");
    bus.write_u32(base + SC3, 0).expect("SC3");
    bus.write_u32(base + SC3, CAL | CALF).expect("SC3.CAL");
    assert_ne!(
        bus.read_u32(base + SC1A).unwrap() & COCO,
        0,
        "calibration COCO"
    );
    assert_eq!(bus.read_u32(base + SC3).unwrap() & (CAL | CALF), 0, "CALF");
    let _ = bus.read_u32(base + RA).expect("dummy R[0]");
    assert_eq!(bus.read_u32(base + OFS).unwrap(), 0, "ideal calibration");
}

/// ADC16_SetChannelConfig(SC1A) + poll COCO + ADC16_GetChannelConversionValue.
fn convert(bus: &mut SystemBus, base: u64, sc1: u32) -> u32 {
    bus.write_u32(base + SC1A, sc1).expect("SC1A");
    assert_ne!(bus.read_u32(base + SC1A).unwrap() & COCO, 0, "COCO");
    let code = bus.read_u32(base + RA).expect("R[0]");
    assert_eq!(
        bus.read_u32(base + SC1A).unwrap() & COCO,
        0,
        "R[0] read clears COCO"
    );
    code
}

#[test]
fn frdm_k64f_arduino_a0_and_a2_convert_ntc_and_diode_on_adc0_and_adc1() {
    let (mut bus, mut session) = board("ptb2", "ptb10");
    init_and_calibrate(&mut bus, ADC0, MODE_12);
    init_and_calibrate(&mut bus, ADC1, MODE_12);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.ptb2_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.ptb10_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc_code = convert(&mut bus, ADC0, AD12);
        assert_eq!(
            ntc_code,
            se_code(ntc_v, 12),
            "ADC0_SE12 at {temperature_c} °C"
        );
        assert_eq!(
            convert(&mut bus, ADC1, AD14),
            se_code(diode_v, 12),
            "ADC1_SE14"
        );
        // The pads are on their own instance only: ADC1 AD12 and ADC0 AD14
        // (PTC0) carry nothing.
        assert_eq!(convert(&mut bus, ADC1, AD12), 0);
        assert_eq!(convert(&mut bus, ADC0, AD14), 0);
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
}

#[test]
fn frdm_k64f_continuous_conversion_started_once_follows_the_ntc() {
    let (mut bus, mut session) = board("ptb2", "ptb10");
    init_and_calibrate(&mut bus, ADC0, MODE_16);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    bus.write_u32(ADC0 + SC3, ADCO).expect("SC3.ADCO");
    bus.write_u32(ADC0 + SC1A, AD12).expect("start once");
    assert_ne!(bus.read_u32(ADC0 + SC2).unwrap() & ADACT, 0, "ADACT");
    for temperature_c in [-10.0_f64, 25.0, 80.0, 25.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.ptb2_volts");
        bus.tick_peripherals_fully();
        assert_ne!(
            bus.read_u32(ADC0 + SC1A).unwrap() & COCO,
            0,
            "the chain completes a new conversion"
        );
        assert_eq!(
            bus.read_u32(ADC0 + RA).unwrap(),
            se_code(ntc_v, 16),
            "16-bit continuous result at {temperature_c} °C"
        );
    }
    // ADCH = 11111 ends the chain: the result stays at the last 25 °C value.
    let held = se_code(ntc_volts(25.0), 16);
    bus.write_u32(ADC0 + SC1A, ADCH_DISABLED).expect("disable");
    assert_eq!(bus.read_u32(ADC0 + SC2).unwrap() & ADACT, 0);
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    bus.tick_peripherals_fully();
    assert_eq!(bus.read_u32(ADC0 + SC1A).unwrap() & COCO, 0);
    assert_eq!(bus.read_u32(ADC0 + RA).unwrap(), held);
}

#[test]
fn frdm_k64f_dad2_converts_the_signed_difference_of_two_netlists() {
    // NTC on ADC0_DP2 (PTE2), diode on ADC0_DM2 (PTE3).
    let (mut bus, mut session) = board("pte2", "pte3");
    init_and_calibrate(&mut bus, ADC0, MODE_12);
    let mut cycles = 0;
    let mut signs = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let plus = millivolts(signal_voltage(&session, "board.analog.pte2_volts"));
        let minus = millivolts(signal_voltage(&session, "board.analog.pte3_volts"));
        // 13-bit differential, 1 LSB = VREFH / 2^12, sign bit 12 extended
        // through bit 15.
        let expected = ((plus - minus) << 12) / 3300;
        let raw = convert(&mut bus, ADC0, DAD2 | DIFF);
        assert_eq!(raw >> 16, 0);
        assert_eq!(
            i64::from(raw as u16 as i16),
            expected,
            "DAD2 at {temperature_c} °C ({plus} - {minus} mV)"
        );
        // Single-ended ADCH 2 reads DADP2 alone.
        assert_eq!(
            convert(&mut bus, ADC0, DAD2),
            ((plus << 12) / 3300) as u32,
            "DADP2"
        );
        signs.push(expected.signum());
    }
    // The NTC divider (2.79 V, 1.65 V, 0.37 V) crosses the ~0.7 V diode
    // between 25 and 80 °C, so the difference changes sign.
    assert_eq!(signs, [1, 1, -1]);
}
