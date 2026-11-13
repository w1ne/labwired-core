// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the S32K118's ADC0
//! (`s32k1_adc`) through the onboarding descriptor's `analog_pins` map: the
//! NTC on PTC14 (ADC0_SE12) and the diode on PTA7 (ADC0_SE3, the S32K118EVB
//! potentiometer channel the NXP cookbook converts), read the way the
//! s32k118_cookbook `ADC.c` does it (`ADC_init`, `SC3.CAL`, then `SC1[0]`
//! write, poll `COCO`, read `R[0]`) against VREFH 5 V. Also:
//! - the same two netlists moved to PTC16 (SE14) and PTC17 (SE15) convert at
//!   10 bits, and the old pads go back to 0 V;
//! - `SC1[0].AIEN` raises ADC0_IRQn (28) in the NVIC on conversion complete;
//! - one continuous conversion (`SC3.ADCO`) started once follows the NTC;
//! - VREFSH (`ADCH` 29), the cookbook's second channel, reads full scale.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;
use std::sync::atomic::Ordering;

const ADC0: u64 = 0x4003_B000;
const SC1_0: u64 = 0x00;
const CFG1: u64 = 0x40;
const CFG2: u64 = 0x44;
const R0: u64 = 0x48;
const SC2: u64 = 0x90;
const SC3: u64 = 0x94;
const COCO: u32 = 1 << 7;
const AIEN: u32 = 1 << 6;
const ADIV: u32 = 3 << 5;
const MODE_12: u32 = 1 << 2;
const MODE_10: u32 = 2 << 2;
const ADACT: u32 = 1 << 7;
const ADCO: u32 = 1 << 3;
const CAL: u32 = 1 << 7;
const ADCH_DISABLED: u32 = 0x1F;
const VREFSH: u32 = 29;
/// S32K118 LQFP64 ADC0 pads (hal_nxp S32K118_LQFP64-pinctrl.h): SE3 = PTA7,
/// SE12 = PTC14, SE13 = PTC15, SE14 = PTC16, SE15 = PTC17.
const SE3: u32 = 3;
const SE12: u32 = 12;
const SE13: u32 = 13;
const SE14: u32 = 14;
const SE15: u32 = 15;
const ADC0_IRQ: u32 = 28;

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

/// N-bit code: 1 LSB = VREFH (5 V) / 2^N, truncated, clamped.
fn code(volts: f64, bits: u32) -> u32 {
    ((millivolts(volts) << bits) / 5000).clamp(0, (1 << bits) - 1) as u32
}

/// The onboarding S32K118 with the generic NTC and diode netlists routed to
/// the two pads.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/onboarding/s32k118.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load S32K118 profile");
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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load S32K118 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build S32K118 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to ADC0 inputs through analog_pins"
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

/// Cookbook `ADC_init` (ADCH off, ADIV, MODE, SMPLTS 12, SW trigger) and the
/// `ADC_calibration_init` CAL handshake.
fn init_and_calibrate(bus: &mut SystemBus, mode: u32) {
    bus.write_u32(ADC0 + SC1_0, ADCH_DISABLED)
        .expect("SC1[0] off");
    bus.write_u32(ADC0 + CFG1, ADIV | mode).expect("CFG1");
    bus.write_u32(ADC0 + CFG2, 12).expect("CFG2.SMPLTS");
    bus.write_u32(ADC0 + SC2, 0)
        .expect("SC2: SW trigger, VREFH");
    bus.write_u32(ADC0 + SC3, CAL).expect("SC3.CAL");
    assert_ne!(
        bus.read_u32(ADC0 + SC1_0).unwrap() & COCO,
        0,
        "calibration COCO"
    );
    assert_eq!(bus.read_u32(ADC0 + SC3).unwrap() & CAL, 0, "CAL done");
    bus.write_u32(ADC0 + SC3, 0).expect("SC3");
}

/// `convertAdcChan` + `adc_complete` + `read_adc_chx` (the raw `R[0]`).
fn convert(bus: &mut SystemBus, sc1: u32) -> u32 {
    bus.write_u32(ADC0 + SC1_0, sc1).expect("SC1[0]");
    assert_ne!(bus.read_u32(ADC0 + SC1_0).unwrap() & COCO, 0, "COCO");
    let value = bus.read_u32(ADC0 + R0).expect("R[0]");
    assert_eq!(
        bus.read_u32(ADC0 + SC1_0).unwrap() & COCO,
        0,
        "R[0] read clears COCO"
    );
    value
}

#[test]
fn s32k118_ptc14_and_pta7_convert_ntc_and_diode_on_adc0() {
    let (mut bus, mut session) = board("ptc14", "pta7");
    init_and_calibrate(&mut bus, MODE_12);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.ptc14_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.pta7_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc_code = convert(&mut bus, SE12);
        assert_eq!(ntc_code, code(ntc_v, 12), "ADC0_SE12 at {temperature_c} °C");
        assert_eq!(convert(&mut bus, SE3), code(diode_v, 12), "ADC0_SE3");
        // The unrouted pads (PTC15, PTC16, PTC17) carry nothing.
        for idle in [SE13, SE14, SE15] {
            assert_eq!(convert(&mut bus, idle), 0, "ADCH {idle}");
        }
        assert_eq!(convert(&mut bus, VREFSH), 4095, "VREFSH is full scale");
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    // The cookbook's millivolt scaling recovers the solved NTC voltage.
    let mv = 5000 * ntc_codes[1] / 0xFFF;
    assert!(
        (i64::from(mv) - millivolts(ntc_volts(25.0))).abs() <= 2,
        "read_adc_chx gives {mv} mV"
    );
}

#[test]
fn s32k118_ptc16_and_ptc17_convert_at_10_bits_and_raise_adc0_irq() {
    let (mut bus, mut session) = board("ptc16", "ptc17");
    init_and_calibrate(&mut bus, MODE_10);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let ntc_v = signal_voltage(&session, "board.analog.ptc16_volts");
    let diode_v = signal_voltage(&session, "board.analog.ptc17_volts");
    assert_eq!(convert(&mut bus, SE14), code(ntc_v, 10), "ADC0_SE14");
    assert_eq!(convert(&mut bus, SE15), code(diode_v, 10), "ADC0_SE15");
    assert_eq!(convert(&mut bus, SE12), 0, "PTC14 is no longer driven");

    // AIEN: COCO drives ADC0_IRQn = 28 into the NVIC pending state.
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let pending = |bus: &SystemBus| {
        bus.nvic
            .as_ref()
            .map(|n| n.ispr[0].load(Ordering::SeqCst) & (1 << ADC0_IRQ) != 0)
    };
    assert_eq!(pending(&bus), Some(false), "no IRQ before the conversion");
    bus.write_u32(ADC0 + SC1_0, SE14 | AIEN)
        .expect("SC1[0] with AIEN");
    bus.tick_peripherals_fully();
    assert_eq!(pending(&bus), Some(true), "ADC0_IRQn pends on COCO");
    assert_eq!(bus.read_u32(ADC0 + R0).unwrap(), code(ntc_v, 10));
}

#[test]
fn s32k118_continuous_conversion_started_once_follows_the_ntc() {
    let (mut bus, mut session) = board("ptc14", "pta7");
    init_and_calibrate(&mut bus, MODE_12);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    bus.write_u32(ADC0 + SC3, ADCO).expect("SC3.ADCO");
    bus.write_u32(ADC0 + SC1_0, SE12).expect("start once");
    assert_ne!(bus.read_u32(ADC0 + SC2).unwrap() & ADACT, 0, "ADACT");
    for temperature_c in [-10.0_f64, 25.0, 80.0, 25.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.ptc14_volts");
        bus.tick_peripherals_fully();
        assert_ne!(
            bus.read_u32(ADC0 + SC1_0).unwrap() & COCO,
            0,
            "new conversion"
        );
        assert_eq!(
            bus.read_u32(ADC0 + R0).unwrap(),
            code(ntc_v, 12),
            "continuous result at {temperature_c} °C"
        );
    }
    bus.write_u32(ADC0 + SC1_0, ADCH_DISABLED).expect("disable");
    assert_eq!(bus.read_u32(ADC0 + SC2).unwrap() & ADACT, 0);
}
