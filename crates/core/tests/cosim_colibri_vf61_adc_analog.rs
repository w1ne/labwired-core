// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the Colibri VF61's
//! Vybrid ADC0/ADC1 (`imxrt_adc`, `layout: vf610`) through the onboarding
//! descriptor's `analog_pins` map (Linux v6.12 vf610-pinfunc.h
//! `VF610_PAD_<pad>__ADC<n>_SE<ch>`): the NTC on PTC30 (ADC0_SE5) and the
//! diode on PTB2 (ADC1_SE2), read the way the Linux vf610_adc driver does:
//! - probe: CFG = ADLPC | ADHSC | OVWREN, then 12-bit mode; calibration with
//!   HC0 = AIEN | 0x1F and GC |= CAL, wait for COCO0, check GS.CALF;
//! - `vf610_read_sample`: HC0 = ADCHC(ch) | AIEN, the ISR sees HS.COCO0 and
//!   reads R0 masked to the resolution (12 or 10 bits) against VREFH 3.3 V.
//!
//! Also: ADC0 channel 5 and ADC1 channel 5 are different pads (PTC30 and
//! PTC31), so the NTC on ADC0 does not reach ADC1; and the netlists moved to
//! PTA18 (ADC0_SE0) and PTC17 (ADC1_SE7) convert there while the old pads go
//! back to 0.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC0: u64 = 0x4003_B000;
const ADC1: u64 = 0x400B_B000;
// Linux v6.12 drivers/iio/adc/vf610_adc.c.
const HC0: u64 = 0x00;
const HS: u64 = 0x08;
const R0: u64 = 0x0C;
const CFG: u64 = 0x14;
const GC: u64 = 0x18;
const GS: u64 = 0x1C;
const MODE_MASK: u32 = 0x0C;
const MODE_BIT10: u32 = 0x04;
const MODE_BIT12: u32 = 0x08;
const ADLPC_EN: u32 = 0x80;
const ADHSC_EN: u32 = 0x400;
const OVWREN: u32 = 0x10000;
const CAL: u32 = 0x80;
const AIEN: u32 = 1 << 7;
const CONV_DISABLE: u32 = 0x1F;
const COCO0: u32 = 0x1;
const CALF: u32 = 0x2;
/// 10 us at the model's default 600 MHz timebase: past one conversion (3 us).
const CONVERSION_CYCLES: u64 = 6_000;
/// 100 us: past the 50 us calibration.
const CALIBRATION_CYCLES: u64 = 60_000;

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

/// `bits` against VREFH 3.3 V, rounded to nearest, clamped.
fn code(volts: f64, bits: u32) -> u32 {
    let full = (1 << bits) - 1;
    (millivolts(volts).min(3300) * full + 1650) / 3300
}

/// The onboarding Colibri VF61 with the generic NTC and diode netlists
/// routed to the two pads.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/onboarding/colibri-vf61.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Colibri VF61");
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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load Colibri VF61 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Colibri VF61 bus");
    for adc in [ADC0, ADC1] {
        linux_probe(&mut bus, adc);
    }
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to ADC pads through analog_pins"
    );
    (bus, session)
}

fn elapse(bus: &mut SystemBus, cycles: u64) {
    let now = bus.current_cycle;
    bus.set_current_cycle(now + cycles);
}

/// `vf610_adc_hw_init`: feature set, 12-bit sample set, calibration.
fn linux_probe(bus: &mut SystemBus, adc: u64) {
    assert_eq!(
        bus.read_u32(adc + CFG).unwrap(),
        0x200,
        "{adc:#x}: CFG reset"
    );
    bus.write_u32(adc + CFG, ADLPC_EN | ADHSC_EN | OVWREN)
        .expect("cfg_post_set");
    bus.write_u32(adc + GC, 0).expect("GC");
    let cfg = bus.read_u32(adc + CFG).unwrap();
    bus.write_u32(adc + CFG, (cfg & !MODE_MASK) | MODE_BIT12)
        .expect("12-bit");
    bus.write_u32(adc + HC0, AIEN | CONV_DISABLE)
        .expect("calibration interrupt");
    let gc = bus.read_u32(adc + GC).unwrap();
    bus.write_u32(adc + GC, gc | CAL).expect("GC.CAL");
    assert_eq!(bus.read_u32(adc + HS).unwrap() & COCO0, 0, "calibrating");
    elapse(bus, CALIBRATION_CYCLES);
    assert_eq!(
        bus.read_u32(adc + HS).unwrap() & COCO0,
        COCO0,
        "{adc:#x}: calibration ends with COCO0"
    );
    assert_eq!(
        bus.read_u32(adc + GC).unwrap() & CAL,
        0,
        "GC.CAL self-clears"
    );
    assert_eq!(bus.read_u32(adc + GS).unwrap() & CALF, 0, "no CALF");
    bus.read_u32(adc + R0).expect("ISR reads R0");
    assert_eq!(bus.read_u32(adc + HS).unwrap() & COCO0, 0);
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

/// `vf610_read_sample` + `vf610_adc_isr` + `vf610_adc_read_data`.
fn read_sample(bus: &mut SystemBus, adc: u64, channel: u32, bits: u32) -> u32 {
    bus.write_u32(adc + HC0, channel | AIEN)
        .expect("HC0 = ADCHC | AIEN");
    assert_eq!(
        bus.read_u32(adc + HS).unwrap() & COCO0,
        0,
        "HC0 clears COCO0"
    );
    elapse(bus, CONVERSION_CYCLES);
    assert_eq!(
        bus.read_u32(adc + HS).unwrap() & COCO0,
        COCO0,
        "{adc:#x} CH{channel}: COCO0"
    );
    bus.read_u32(adc + R0).unwrap() & ((1 << bits) - 1)
}

fn set_resolution(bus: &mut SystemBus, adc: u64, mode: u32) {
    let cfg = bus.read_u32(adc + CFG).unwrap();
    bus.write_u32(adc + CFG, (cfg & !MODE_MASK) | mode)
        .expect("CFG.MODE");
}

#[test]
fn colibri_vf61_ptc30_and_ptb2_convert_ntc_and_diode_on_adc0_and_adc1() {
    let (mut bus, mut session) = board("PTC30", "PTB2");
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.PTC30_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.PTB2_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc_code = read_sample(&mut bus, ADC0, 5, 12);
        assert_eq!(ntc_code, code(ntc_v, 12), "ADC0_SE5 at {temperature_c} °C");
        assert_eq!(
            read_sample(&mut bus, ADC1, 2, 12),
            code(diode_v, 12),
            "ADC1_SE2"
        );
        // Channel 5 of ADC1 is PTC31, not PTC30.
        assert_eq!(read_sample(&mut bus, ADC1, 5, 12), 0, "PTC31 is not driven");
        assert_eq!(read_sample(&mut bus, ADC0, 2, 12), 0, "PTB0 is not driven");
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    // The IIO scale is vref_uv / 1000 / 2^12: code * 3300 / 4096 mV.
    let mv = ntc_codes[1] * 3300 / 4096;
    assert!(
        mv.abs_diff(millivolts(ntc_volts(25.0))) <= 2,
        "code * VREF / 2^12 gives {mv} mV"
    );
}

#[test]
fn colibri_vf61_ten_bit_mode_rescales_the_routed_diode() {
    let (mut bus, mut session) = board("PTC30", "PTB2");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let diode_v = signal_voltage(&session, "board.analog.PTB2_volts");
    let twelve = read_sample(&mut bus, ADC1, 2, 12);
    set_resolution(&mut bus, ADC1, MODE_BIT10);
    let ten = read_sample(&mut bus, ADC1, 2, 10);
    assert_eq!(ten, code(diode_v, 10), "10-bit ADC1_SE2");
    assert!(
        ten.abs_diff(twelve / 4) <= 1,
        "10-bit {ten} vs 12-bit {twelve}"
    );
}

#[test]
fn colibri_vf61_pta18_and_ptc17_convert_after_the_netlists_move() {
    let (mut bus, mut session) = board("PTA18", "PTC17");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_v = signal_voltage(&session, "board.analog.PTA18_volts");
    let diode_v = signal_voltage(&session, "board.analog.PTC17_volts");
    assert_eq!(
        read_sample(&mut bus, ADC0, 0, 12),
        code(ntc_v, 12),
        "ADC0_SE0"
    );
    assert_eq!(
        read_sample(&mut bus, ADC1, 7, 12),
        code(diode_v, 12),
        "ADC1_SE7"
    );
    assert_eq!(read_sample(&mut bus, ADC0, 5, 12), 0, "PTC30 is not driven");
    assert_eq!(read_sample(&mut bus, ADC1, 2, 12), 0, "PTB2 is not driven");
}
