// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists reach the PIC24F16KA102- and PIC24F04KA201-family
//! ADC1 through the same `pic24_adc_pads` seam the dsPIC33F boards use.
//!
//! The family's 10-bit ADC1 has the dsPIC33F register map without AD12B,
//! CHPS and AD1CHS123 (DS39927C Table 4-15), but bonds a different channel
//! set: AN0-AN5 and AN10-AN12, with AN4/AN5 on RA2/RA3 on the 20-pin parts
//! (DS39927C Table 1-2). The dsPIC port-mask layout would put AN4 on RB2 and
//! drop AN10-AN12, so these tests prove the channel set comes from the
//! descriptor's `analog_pins`.
//!
//! The PIC24F04KA201 family (DS30009937C Table 4-15, §19) has the same ADC1
//! map and channel numbers, but its 14-pin PIC24F04KA200 bonds AN2/AN3 on
//! RB4/RA4 and has no AN11/AN12 (Table 1-2, Table 4-15 note 1). No code
//! changes for it: the descriptor's `analog_pins` alone moves the channels.
//!
//! No XC16 toolchain is available to this lane, so the firmware is a
//! hand-encoded PIC24 instruction stream (DS70000157G encodings): it sets
//! AD1PCFG/AD1CHS, runs one manual sample and conversion, polls DONE, and
//! stores ADC1BUF0. Only the routed pad level can reach the converter.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::Pic24;
use labwired_core::system::builder::{pic24_adc_channels, pic24_port_masks};
use labwired_core::{Cpu, SimulationConfig};
use std::path::{Path, PathBuf};

const MARKER: u16 = 0x0E00;
const ADC_RESULT: u16 = 0x0E02;

const AD1CON1: u16 = 0x0320;
const AD1CON2: u16 = 0x0322;
const AD1CON3: u16 = 0x0324;
const AD1CHS: u16 = 0x0328;
const AD1PCFG: u16 = 0x032C;
const AD1CSSL: u16 = 0x0330;
const ADC1BUF0: u16 = 0x0300;

/// AN0-AN5 and AN10-AN12 (all 20/28-pin KA parts).
const KA10X_CHANNELS: u16 = 0x1C3F;
/// AN0-AN5 and AN10 (14-pin PIC24F04KA200).
const KA200_CHANNELS: u16 = 0x043F;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `MOV #lit16, Wn`.
fn mov_lit(val: u16, wn: u32) -> u32 {
    0x0020_0000 | (u32::from(val) << 4) | wn
}

/// `MOV Wns, f` (word).
fn mov_w_to_f(wn: u32, f: u16) -> u32 {
    0x0088_0000 | (u32::from(f >> 1) << 4) | wn
}

/// `MOV f, Wnd` (word).
fn mov_f_to_w(f: u16, wn: u32) -> u32 {
    0x0080_0000 | (u32::from(f >> 1) << 4) | wn
}

/// One manual 10-bit conversion of AN`channel`, unsigned integer format.
fn convert_program(channels: u16, channel: u16) -> Vec<u32> {
    let mut words = Vec::new();
    let write = |words: &mut Vec<u32>, f: u16, val: u16| {
        words.push(mov_lit(val, 0));
        words.push(mov_w_to_f(0, f));
    };
    // PCFG = 1 is digital: leave only AN`channel` analog.
    write(&mut words, AD1PCFG, channels & !(1 << channel));
    write(&mut words, AD1CON1, 0x0000); // SSRC = 000, FORM = 00
    write(&mut words, AD1CON2, 0x0000); // AVDD/AVSS, MUX A, one buffer
    write(&mut words, AD1CON3, 0x0001); // ADCS = 1: TAD = 2 TCY
    write(&mut words, AD1CHS, channel); // CH0SA = AN`channel`, CH0NA = VR-
    write(&mut words, AD1CSSL, 0x0000);
    write(&mut words, AD1CON1, 0x8000); // ADON: sampling starts
    write(&mut words, AD1CON1, 0x8002); // SAMP = 1
    write(&mut words, AD1CON1, 0x8000); // SAMP = 0: convert
                                        // poll: MOV AD1CON1, W1; AND #1, W1; BRA Z, poll
    words.push(mov_f_to_w(AD1CON1, 1));
    words.push(0x00B2_0011);
    words.push(0x0032_FFFD);
    words.push(mov_f_to_w(ADC1BUF0, 2));
    words.push(mov_w_to_f(2, ADC_RESULT));
    words.push(mov_lit(0xBEEF, 0));
    words.push(mov_w_to_f(0, MARKER));
    words.push(0x0037_FFFF); // BRA $-1
    words
}

/// The board bus as the PIC24 session builder makes it, plus the chip.
fn board_bus(chip: &str, analog: &str) -> (SystemBus, SystemManifest, ChipDescriptor) {
    let system_path = repo_root().join(format!("examples/{chip}/system.yaml"));
    let mut profile = SystemManifest::from_file(&system_path).expect("load the KA example");
    profile.cosim_models = serde_yaml::from_str(analog).expect("parse the analog models");
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the chip descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build the KA bus");
    (bus, profile, chip)
}

/// Runs the conversion program on a CPU set up as the builder sets it up
/// and returns ADC1BUF0.
fn convert(chip: &ChipDescriptor, channel: u16, bus: &mut SystemBus) -> u16 {
    let channels = pic24_adc_channels(chip).expect("KA descriptor declares analog_pins");
    let expected = if chip.name == "pic24f04ka200" {
        KA200_CHANNELS
    } else {
        KA10X_CHANNELS
    };
    assert_eq!(channels, expected, "Table 1-2 channel set of {}", chip.name);
    let mut cpu = Pic24::new();
    cpu.set_port_masks(pic24_port_masks(chip));
    cpu.set_adc_channels(channels);
    cpu.set_adc_pad_window(true);
    cpu.load_words(&convert_program(channels, channel));
    cpu.reset(bus).expect("reset");
    cpu.set_pc(0);
    for _ in 0..10_000 {
        cpu.step(bus, &[], &SimulationConfig::default())
            .expect("conversion program must decode and execute");
        if cpu.data_read(MARKER).unwrap() == 0xBEEF {
            assert_eq!(
                cpu.data_read(AD1PCFG).unwrap(),
                channels & !(1 << channel),
                "AD1PCFG implements only the bonded PCFG bits"
            );
            return cpu.data_read(ADC_RESULT).unwrap();
        }
    }
    panic!("the conversion program never reached its marker");
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// The 10-bit code for `volts` against `vref_v`, through the route's mV.
fn ideal_code(volts: f64, vref_v: f64) -> u16 {
    let mv = (volts * 1000.0).round() as u32;
    let vref_mv = (vref_v * 1000.0).round() as u32;
    (mv * 1024 / vref_mv).min(1023) as u16
}

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn solve_session(session: &mut CosimSession, cycles: &mut u64, bus: &mut SystemBus) {
    *cycles += session.cycles_until_boundary(*cycles);
    let (_, errors) = session.advance_to(*cycles, bus).expect("solve and route");
    assert!(errors.is_empty(), "unexpected route errors: {errors:?}");
}

/// An NTC divider on RB12, which is AN12 on the 28-pin PIC24F16KA102: an
/// input above AN8 that the dsPIC layout does not have.
#[test]
fn pic24f16ka102_adc1_converts_a_routed_ntc_divider_on_an12() {
    const ANALOG: &str = r#"
- id: "ntc_divider"
  adapter: "analog"
  step_ns: 1000
  inputs:
    temperature: "ui.ntc.temperature_c"
  outputs:
    v_sense: "board.analog.RB12_volts"
  config:
    netlist_text: |
      Vcc vcc 0 dc 3.3
      Rpull vcc sense 10k
      Rntc sense 0 10k beta=3950 tnom=25 temp=25
    probes:
      v_sense: "v(sense)"
    sources:
      temperature: "Rntc"
    vdd: 3.3
"#;
    let (mut bus, profile, chip) = board_bus("pic24f16ka102", ANALOG);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an analog model");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "RB12 is AN12 in the KA102 descriptor's analog_pins"
    );

    let mut cycles = 0;
    let mut codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .unwrap();
        solve_session(&mut session, &mut cycles, &mut bus);
        let v = signal_voltage(&session, "board.analog.RB12_volts");
        let r = ntc_resistance_ohms(temperature_c);
        assert!(
            (v - 3.3 * r / (10_000.0 + r)).abs() < 1e-9,
            "NTC divider at {temperature_c} °C"
        );
        let code = convert(&chip, 12, &mut bus);
        assert_eq!(code, ideal_code(v, 3.3), "ADC1BUF0 at {temperature_c} °C");
        codes.push(code);
    }
    // -10 °C = 2.816 V, 25 °C = 1.650 V, 80 °C = 0.372 V.
    assert_eq!(codes, [873, 512, 115]);
}

/// The level-1 source-follower output: `KP/2 · (W/L) · (Vg - Vs - VTO)^2 ·
/// (1 + LAMBDA · Vds) = Vs / Rs`, bulk tied to source.
fn source_follower_volts(vdd: f64, vg: f64, rs: f64) -> f64 {
    const KP: f64 = 50e-6;
    const VTO: f64 = 0.7;
    const LAMBDA: f64 = 0.02;
    const W_OVER_L: f64 = 4.0;
    let residual = |vs: f64| {
        let vov = vg - vs - VTO;
        KP / 2.0 * W_OVER_L * vov * vov * (1.0 + LAMBDA * (vdd - vs)) - vs / rs
    };
    let (mut low, mut high) = (0.0, vg - VTO);
    for _ in 0..200 {
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

/// A second, unrelated device on the 20-pin PIC24F08KA101: an NMOS source
/// follower on RA2, which is AN4 on this package (RB2 on the dsPIC layout,
/// and RB3/AN5 is not bonded here). The converter reference is routed from
/// the circuit (`board.power.vdda_volts` = 3.0 V).
#[test]
fn pic24f08ka101_adc1_converts_an_nmos_follower_on_ra2_against_a_routed_reference() {
    const ANALOG: &str = r#"
- id: "nmos_follower"
  adapter: "analog"
  step_ns: 1000
  outputs:
    v_out: "board.analog.RA2_volts"
    v_ref: "board.power.vdda_volts"
  config:
    netlist_text: |
      Vdd vdd 0 dc 3.0
      Vg gate 0 dc 2.5
      M1 vdd gate out out NM W=20u L=5u
      Rs out 0 10k
      .model NM NMOS(LEVEL=1 VTO=0.7 KP=50u LAMBDA=0.02)
    probes:
      v_out: "v(out)"
      v_ref: "v(vdd)"
    vdd: 3.0
"#;
    let (mut bus, profile, chip) = board_bus("pic24f08ka101", ANALOG);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an analog model");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "RA2 is AN4 on the 20-pin KA101"
    );
    let mut cycles = 0;
    solve_session(&mut session, &mut cycles, &mut bus);

    let v = signal_voltage(&session, "board.analog.RA2_volts");
    let expected_v = source_follower_volts(3.0, 2.5, 10_000.0);
    assert!(
        (v - expected_v).abs() < 1e-6,
        "level-1 follower {expected_v} V, got {v} V"
    );
    let code = convert(&chip, 4, &mut bus);
    assert_eq!(code, ideal_code(v, 3.0), "ADC1BUF0 for {v} V against 3.0 V");
    assert_ne!(
        code,
        ideal_code(v, 3.3),
        "the routed reference must change the code"
    );
    // A different channel on the same window stays at the harness code.
    assert_eq!(convert(&chip, 5, &mut bus), 0, "AN5 (RA3) carries no route");
}

/// Routes the family does not bond out never reach the converter. A pad with
/// no AN function (RB15) is refused when the session is built. AN9 sits
/// inside the AN0-AN12 count the generic seam checks at build time
/// (`Peripheral::adc_channel_count`), so the window refuses it on every
/// solve instead and the step reports a route error.
#[test]
fn pic24f_ka10x_refuses_adc_routes_outside_its_channel_set() {
    const ANALOG: &str = r#"
- id: "divider"
  adapter: "analog"
  step_ns: 1000
  outputs:
    an9: "adc.adc1.9_volts"
    rb15: "board.analog.RB15_volts"
  config:
    netlist_text: |
      V1 a 0 dc 1.0
      R1 a 0 1k
    probes:
      an9: "v(a)"
      rb15: "v(a)"
    vdd: 3.3
"#;
    let (mut bus, profile, _) = board_bus("pic24f16ka102", ANALOG);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an analog model");
    assert_eq!(
        session.binding_errors().len(),
        1,
        "RB15 has no AN function: {:?}",
        session.binding_errors()
    );
    let cycles = session.cycles_until_boundary(0);
    let (_, errors) = session.advance_to(cycles, &mut bus).expect("solve");
    assert!(
        !errors.is_empty(),
        "the AN9 route must not land silently: {errors:?}"
    );

    const RB2_ON_KA101: &str = r#"
- id: "divider"
  adapter: "analog"
  step_ns: 1000
  outputs:
    rb2: "board.analog.RB2_volts"
  config:
    netlist_text: |
      V1 a 0 dc 1.0
      R1 a 0 1k
    probes:
      rb2: "v(a)"
    vdd: 3.3
"#;
    let (bus, profile, _) = board_bus("pic24f08ka101", RB2_ON_KA101);
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an analog model");
    assert_eq!(
        session.binding_errors().len(),
        1,
        "RB2 has no AN function on the 20-pin package: {:?}",
        session.binding_errors()
    );
}

/// Gummel-Poon emitter follower with only IS, BF and BR set (qb = 1): the
/// emitter current `Ve / Re` is `Ibe - Ibc + Ibe / BF`. The `Ibc / BR` terms
/// of the collector and base currents cancel in the emitter current.
fn npn_follower_volts(vcc: f64, vb: f64, re: f64) -> f64 {
    const IS: f64 = 1e-15;
    const BF: f64 = 100.0;
    let vt = labwired_core::analog::device::THERMAL_VOLTAGE;
    let residual = |ve: f64| {
        let i_be = IS * (((vb - ve) / vt).exp() - 1.0);
        let i_bc = IS * (((vb - vcc) / vt).exp() - 1.0);
        i_be - i_bc + i_be / BF - ve / re
    };
    let (mut low, mut high) = (0.0, vb);
    for _ in 0..200 {
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

/// The 14-pin PIC24F04KA200 bonds AN2 on RB4 and AN3 on RA4 (DS30009937C
/// Table 1-2). An NPN emitter follower on RB4 converts as AN2; the 20-pin
/// layout would have put AN2 on RB0, which this package does not bond.
#[test]
fn pic24f04ka200_adc1_converts_an_npn_follower_on_rb4_as_an2() {
    const ANALOG: &str = r#"
- id: "npn_follower"
  adapter: "analog"
  step_ns: 1000
  inputs:
    base: "ui.npn.base_v"
  outputs:
    v_out: "board.analog.RB4_volts"
  config:
    netlist_text: |
      Vcc vcc 0 dc 3.3
      Vb base 0 dc 1.5
      Q1 vcc base out QN
      Re out 0 10k
      .model QN NPN(IS=1e-15 BF=100 BR=1)
    probes:
      v_out: "v(out)"
    sources:
      base: "Vb"
    vdd: 3.3
"#;
    let (mut bus, profile, chip) = board_bus("pic24f04ka200", ANALOG);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an analog model");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "RB4 is AN2 on the 14-pin KA200"
    );

    let mut cycles = 0;
    let mut codes = Vec::new();
    for base_v in [1.2_f64, 1.8, 2.6] {
        session.set_signal_number("ui.npn.base_v", base_v).unwrap();
        solve_session(&mut session, &mut cycles, &mut bus);
        let v = signal_voltage(&session, "board.analog.RB4_volts");
        let expected_v = npn_follower_volts(3.3, base_v, 10_000.0);
        assert!(
            (v - expected_v).abs() < 1e-5,
            "NPN follower at Vb = {base_v} V: expected {expected_v} V, got {v} V"
        );
        let code = convert(&chip, 2, &mut bus);
        assert_eq!(code, ideal_code(v, 3.3), "ADC1BUF0 at Vb = {base_v} V");
        codes.push(code);
    }
    assert!(
        codes.windows(2).all(|w| w[0] < w[1]),
        "AN2 codes follow the base drive: {codes:?}"
    );
    // RA4 (AN3) carries no route and stays at the harness code.
    assert_eq!(convert(&chip, 3, &mut bus), 0, "AN3 (RA4) carries no route");
}

/// The 14-pin KA200 refuses the 20-pin AN pads (RB0/AN2 and RB12/AN12 are
/// not bonded) and AN11 (PCFG11 not implemented: Table 4-15 note 1) when the
/// session is built: the window's channel count ends at AN10. AN6 sits inside
/// that count, so the window refuses it on every solve instead.
#[test]
fn pic24f04ka200_refuses_the_20_pin_analog_inputs() {
    const ANALOG: &str = r#"
- id: "divider"
  adapter: "analog"
  step_ns: 1000
  outputs:
    rb0: "board.analog.RB0_volts"
    rb12: "board.analog.RB12_volts"
    an11: "adc.adc1.11_volts"
  config:
    netlist_text: |
      V1 a 0 dc 1.0
      R1 a 0 1k
    probes:
      rb0: "v(a)"
      rb12: "v(a)"
      an11: "v(a)"
    vdd: 3.3
"#;
    let (bus, profile, _) = board_bus("pic24f04ka200", ANALOG);
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an analog model");
    assert_eq!(
        session.binding_errors().len(),
        3,
        "RB0, RB12 and AN11 do not exist on the 14-pin package: {:?}",
        session.binding_errors()
    );

    const AN6: &str = r#"
- id: "divider"
  adapter: "analog"
  step_ns: 1000
  outputs:
    an6: "adc.adc1.6_volts"
  config:
    netlist_text: |
      V1 a 0 dc 1.0
      R1 a 0 1k
    probes:
      an6: "v(a)"
    vdd: 3.3
"#;
    let (mut bus, profile, _) = board_bus("pic24f04ka200", AN6);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an analog model");
    assert_eq!(session.binding_errors(), &[] as &[RoutingError]);
    let cycles = session.cycles_until_boundary(0);
    let (_, errors) = session.advance_to(cycles, &mut bus).expect("solve");
    assert!(
        !errors.is_empty(),
        "the AN6 route must not land silently: {errors:?}"
    );
}

/// Level-1 NJF drain voltage against a 1 kΩ load (no terminal resistances).
fn njf_drain_volts(vdd: f64, vgs: f64) -> f64 {
    const VTO: f64 = -2.0;
    const BETA: f64 = 500e-6;
    const LAMBDA: f64 = 0.025;
    const RLOAD: f64 = 1_000.0;
    let channel = |vds: f64| {
        let vgst = vgs - VTO;
        if vgst <= 0.0 {
            0.0
        } else if vds < vgst {
            BETA * (1.0 + LAMBDA * vds) * vds * (2.0 * vgst - vds)
        } else {
            BETA * (1.0 + LAMBDA * vds) * vgst * vgst
        }
    };
    let (mut low, mut high) = (0.0, vdd);
    for _ in 0..200 {
        let vds = (low + high) / 2.0;
        if channel(vds) > (vdd - vds) / RLOAD {
            high = vds;
        } else {
            low = vds;
        }
    }
    (low + high) / 2.0
}

/// A third device class on the 20-pin PIC24F04KA201: a level-1 N-JFET load
/// line on RB13, which is AN11 on this family. The gate sweep moves the
/// drain through saturation into the linear region.
#[test]
fn pic24f04ka201_adc1_converts_a_jfet_load_line_on_an11() {
    const ANALOG: &str = r#"
- id: "jfet_load_line"
  adapter: "analog"
  step_ns: 1000
  inputs:
    gate: "ui.jfet.gate_v"
  outputs:
    v_drain: "board.analog.RB13_volts"
  config:
    netlist_text: |
      Vdd vdd 0 dc 3.3
      Vg gate 0 dc 0
      Rload vdd drain 1k
      J1 drain gate 0 NJ
      .model NJ NJF(VTO=-2 BETA=500u LAMBDA=0.025 B=1 PB=1 IS=1e-30)
    probes:
      v_drain: "v(drain)"
    sources:
      gate: "Vg"
    vdd: 3.3
"#;
    let (mut bus, profile, chip) = board_bus("pic24f04ka201", ANALOG);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an analog model");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "RB13 is AN11 on the 20-pin KA201"
    );

    let mut cycles = 0;
    let mut codes = Vec::new();
    for gate_v in [-1.8_f64, -1.2, 0.0] {
        session.set_signal_number("ui.jfet.gate_v", gate_v).unwrap();
        solve_session(&mut session, &mut cycles, &mut bus);
        let v = signal_voltage(&session, "board.analog.RB13_volts");
        let expected_v = njf_drain_volts(3.3, gate_v);
        assert!(
            (v - expected_v).abs() < 1e-6,
            "NJF load line at Vgs = {gate_v} V: expected {expected_v} V, got {v} V"
        );
        let code = convert(&chip, 11, &mut bus);
        assert_eq!(code, ideal_code(v, 3.3), "ADC1BUF0 at Vgs = {gate_v} V");
        codes.push(code);
    }
    assert!(
        codes.windows(2).all(|w| w[0] > w[1]),
        "AN11 codes fall as the JFET opens: {codes:?}"
    );
}
