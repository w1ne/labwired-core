// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the FRDM-KW41Z's ADC0
//! (the same Kinetis ADC16 model as the FRDM-K64F) through the MKW41Z4
//! `analog_pins` map: the NTC on PTB2 (ADC0_SE3) and the diode on PTB3
//! (ADC0_SE2), converted the way mcux-sdk `fsl_adc16.c` does it (Init,
//! DoAutoCalibration, SC1A write, poll COCO, read R[0]). Also:
//! - the same two netlists moved to PTB18 (SE4) and PTA19 (SE5) convert at
//!   16 bits, and the old pads go back to 0 V;
//! - `SC1A.AIEN` raises ADC0_IRQn (15) in the NVIC on conversion complete;
//! - one continuous conversion (`SC3.ADCO`) started once follows the NTC.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;
use std::sync::atomic::Ordering;

const ADC0: u64 = 0x4003_B000;
const SC1A: u64 = 0x00;
const CFG1: u64 = 0x08;
const RA: u64 = 0x10;
const SC2: u64 = 0x20;
const SC3: u64 = 0x24;
const OFS: u64 = 0x28;
const COCO: u32 = 1 << 7;
const MODE_12: u32 = 1 << 2;
const MODE_16: u32 = 3 << 2;
const ADACT: u32 = 1 << 7;
const ADCO: u32 = 1 << 3;
const CAL: u32 = 1 << 7;
const CALF: u32 = 1 << 6;
const ADCH_DISABLED: u32 = 0x1F;
/// MKW41Z512VHT4 ADC0 pads: SE1 = PTB1, SE2 = PTB3, SE3 = PTB2, SE4 = PTB18,
/// SE5 = PTA19.
const SE1: u32 = 1;
const SE2: u32 = 2;
const SE3: u32 = 3;
const SE4: u32 = 4;
const SE5: u32 = 5;
const AIEN: u32 = 1 << 6;

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

/// The FRDM-KW41Z with the generic NTC and diode netlists routed to `pads`.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/frdm-kw41z.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load FRDM-KW41Z profile");
    let analog =
        SystemManifest::from_file(root("examples/frdm-kw41z/analog-ntc-diode-system.yaml"))
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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load MKW41Z4 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build FRDM-KW41Z bus");
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
fn frdm_kw41z_ptb2_and_ptb3_convert_ntc_and_diode_on_adc0() {
    let (mut bus, mut session) = board("ptb2", "ptb3");
    init_and_calibrate(&mut bus, ADC0, MODE_12);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.ptb2_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.ptb3_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc_code = convert(&mut bus, ADC0, SE3);
        assert_eq!(
            ntc_code,
            se_code(ntc_v, 12),
            "ADC0_SE3 at {temperature_c} °C"
        );
        assert_eq!(
            convert(&mut bus, ADC0, SE2),
            se_code(diode_v, 12),
            "ADC0_SE2"
        );
        // The unrouted pads (PTB1, PTB18, PTA19) carry nothing.
        for idle in [SE1, SE4, SE5] {
            assert_eq!(convert(&mut bus, ADC0, idle), 0, "ADCH {idle}");
        }
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
}

#[test]
fn frdm_kw41z_ptb18_and_pta19_convert_at_16_bits_and_raise_adc0_irq() {
    let (mut bus, mut session) = board("ptb18", "pta19");
    init_and_calibrate(&mut bus, ADC0, MODE_16);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let ntc_v = signal_voltage(&session, "board.analog.ptb18_volts");
    let diode_v = signal_voltage(&session, "board.analog.pta19_volts");
    assert_eq!(convert(&mut bus, ADC0, SE4), se_code(ntc_v, 16), "ADC0_SE4");
    assert_eq!(
        convert(&mut bus, ADC0, SE5),
        se_code(diode_v, 16),
        "ADC0_SE5"
    );
    assert_eq!(convert(&mut bus, ADC0, SE3), 0, "PTB2 is no longer driven");

    // AIEN: COCO drives ADC0_IRQn = 15 into the NVIC pending state.
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let pending = |bus: &SystemBus| {
        bus.nvic
            .as_ref()
            .map(|n| n.ispr[0].load(Ordering::SeqCst) & (1 << 15) != 0)
    };
    assert_eq!(pending(&bus), Some(false), "no IRQ before the conversion");
    bus.write_u32(ADC0 + SC1A, SE4 | AIEN)
        .expect("SC1A with AIEN");
    bus.tick_peripherals_fully();
    assert_eq!(pending(&bus), Some(true), "ADC0_IRQn pends on COCO");
    assert_eq!(bus.read_u32(ADC0 + RA).unwrap(), se_code(ntc_v, 16));
}

#[test]
fn frdm_kw41z_continuous_conversion_started_once_follows_the_ntc() {
    let (mut bus, mut session) = board("ptb2", "ptb3");
    init_and_calibrate(&mut bus, ADC0, MODE_12);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    bus.write_u32(ADC0 + SC3, ADCO).expect("SC3.ADCO");
    bus.write_u32(ADC0 + SC1A, SE3).expect("start once");
    assert_ne!(bus.read_u32(ADC0 + SC2).unwrap() & ADACT, 0, "ADACT");
    for temperature_c in [-10.0_f64, 25.0, 80.0, 25.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.ptb2_volts");
        bus.tick_peripherals_fully();
        assert_ne!(
            bus.read_u32(ADC0 + SC1A).unwrap() & COCO,
            0,
            "new conversion"
        );
        assert_eq!(
            bus.read_u32(ADC0 + RA).unwrap(),
            se_code(ntc_v, 12),
            "continuous result at {temperature_c} °C"
        );
    }
    bus.write_u32(ADC0 + SC1A, ADCH_DISABLED).expect("disable");
    assert_eq!(bus.read_u32(ADC0 + SC2).unwrap() & ADACT, 0);
}
