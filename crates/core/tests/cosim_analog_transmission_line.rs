// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A `T` transmission line between a sensor and an ADC pad, end to end
//! through the co-simulation router and the ADC registers: a remote NTC
//! hangs on 1 km of 100 Ω pair (5 us one way) and the pull-up sits on the
//! board, at the pad. After a temperature step at the far end the pad must
//! not move for one line delay, and must then climb one stair per round trip
//! as the reflections charge the cable through the 10 kΩ pull-up, on the
//! closed-form bounce diagram. A wire in place of the line would put the new
//! divider voltage on the pad in the first step. The same netlist runs on two
//! unrelated chips' ADCs.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const VCC: f64 = 3.3;
const PULL_UP_OHMS: f64 = 10_000.0;
const Z0_OHMS: f64 = 100.0;
/// One-way delay in co-sim steps of 1 us.
const DELAY_STEPS: u32 = 5;

fn ntc_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn remote_ntc_on(pad: &str) -> String {
    format!(
        r#"
- id: "remote_ntc"
  adapter: "analog"
  step_ns: 1000
  inputs:
    temperature: "ui.ntc.temperature_c"
  outputs:
    v_adc: "board.analog.{pad}_volts"
  config:
    substeps: 10
    netlist_text: |
      Vcc vcc 0 dc {VCC}
      Rpull vcc pad {PULL_UP_OHMS}
      Tcable pad 0 remote 0 Z0={Z0_OHMS} TD={DELAY_STEPS}u
      Rntc remote 0 10k beta=3950 tnom=25 temp=25
    probes:
      v_adc: "v(pad)"
    sources:
      temperature: "Rntc"
    vdd: 3.3
"#
    )
}

/// The bounce diagram at the pad, `steps` co-sim steps after the NTC moved
/// from `from_c` to `to_c`, independent of the engine.
///
/// `a` is the wave travelling to the NTC and `b` the one coming back. The
/// pull-up end launches `a = Vcc·Z0/(Rp + Z0) + gamma_pad·b`, the NTC end
/// answers `b = gamma_ntc·a`, and the pad sits at `a + b`. Before the step
/// both are the settled pair. The NTC answers the old `a` with its new
/// reflection at once; that `b` reaches the pad one delay later and every
/// later one two delays after the last.
fn pad_volts(from_c: f64, to_c: f64, steps: u32) -> f64 {
    let gamma = |ohms: f64| (ohms - Z0_OHMS) / (ohms + Z0_OHMS);
    let launch = VCC * Z0_OHMS / (PULL_UP_OHMS + Z0_OHMS);
    let gamma_pad = gamma(PULL_UP_OHMS);
    let mut a = launch / (1.0 - gamma_pad * gamma(ntc_ohms(from_c)));
    let mut b = gamma(ntc_ohms(from_c)) * a;
    // The engine moves the resistor in the first internal step, so the first
    // pad sample that holds a new stair is the one after the delay.
    if steps > DELAY_STEPS {
        for _ in 0..=(steps - DELAY_STEPS - 1) / (2 * DELAY_STEPS) {
            b = gamma(ntc_ohms(to_c)) * a;
            a = launch + gamma_pad * b;
        }
    }
    a + b
}

fn divider_volts(temperature_c: f64) -> f64 {
    let ntc = ntc_ohms(temperature_c);
    VCC * ntc / (PULL_UP_OHMS + ntc)
}

/// 12-bit single-ended code against a 3.3 V reference, as both ADC models
/// quantise the routed millivolts.
fn expected_code(volts: f64) -> u32 {
    let millivolts = (volts * 1000.0).round() as u32;
    (millivolts * (1 << 12) / 3300).min((1 << 12) - 1)
}

fn cable_staircase_reaches_adc(
    system: &str,
    pad: &str,
    prepare: fn(&mut SystemBus),
    convert: fn(&mut SystemBus) -> u32,
) {
    let system_path = root(system);
    let mut profile = SystemManifest::from_file(&system_path).expect("load board profile");
    profile.cosim_models = serde_yaml::from_str(&remote_ntc_on(pad)).expect("parse model");
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build bus");
    prepare(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("one model");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{system}: {pad} must bind as an ADC input"
    );
    let signal = format!("board.analog.{pad}_volts");
    let mut cycles = 0;
    let mut step = |session: &mut CosimSession, bus: &mut SystemBus| {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session.advance_to(cycles, bus).expect("solve");
        assert_eq!(steps.len(), 1);
        assert!(errors.is_empty(), "{system}: route errors {errors:?}");
        match session.signals().get(&signal) {
            Some(CosimSignalValue::F64(volts)) => *volts,
            other => panic!("{system}: no solved voltage on {pad}: {other:?}"),
        }
    };

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .unwrap();
    let start = step(&mut session, &mut bus);
    assert!(
        (start - divider_volts(25.0)).abs() < 1e-9,
        "{system}: at rest the cable is two wires and the pad sits on the divider, got {start} V"
    );
    assert!((pad_volts(25.0, -10.0, 0) - divider_volts(25.0)).abs() < 1e-12);
    let start_code = convert(&mut bus);
    assert_eq!(start_code, expected_code(start), "{system}: code at rest");

    session
        .set_signal_number("ui.ntc.temperature_c", -10.0)
        .unwrap();
    let mut stair_codes = Vec::new();
    for n in 1..=100u32 {
        let got = step(&mut session, &mut bus);
        let expected = pad_volts(25.0, -10.0, n);
        assert!(
            (got - expected).abs() < 1e-9,
            "{system} {pad}, {n} us after the step: bounce diagram {expected} V, solved {got} V"
        );
        let code = convert(&mut bus);
        assert_eq!(code, expected_code(got), "{system}: {pad} code at {n} us");
        if n <= DELAY_STEPS {
            assert_eq!(
                code, start_code,
                "{system}: the pad cannot know about the far end for one line delay"
            );
        }
        // The last sample of each stair.
        if n >= DELAY_STEPS && (n - DELAY_STEPS).is_multiple_of(2 * DELAY_STEPS) {
            stair_codes.push(code);
        }
    }

    assert_eq!(stair_codes.len(), 10);
    assert!(
        stair_codes.windows(2).all(|pair| pair[1] >= pair[0] + 20),
        "{system}: each round trip lifts the pad by more than 20 codes: {stair_codes:?}"
    );
    let settled_code = expected_code(divider_volts(-10.0));
    assert!(
        stair_codes[9] + 800 < settled_code,
        "{system}: 100 us in, the cable is still charging towards {settled_code}: {stair_codes:?}"
    );
}

const K64F_ADC0: u64 = 0x4003_B000;

/// fsl_adc16 `ADC16_Init`: 12-bit, VREFH, software trigger.
fn k64f_prepare(bus: &mut SystemBus) {
    bus.write_u32(K64F_ADC0 + 0x08, 1 << 2).expect("CFG1.MODE");
    bus.write_u32(K64F_ADC0 + 0x20, 0).expect("SC2.REFSEL");
    bus.write_u32(K64F_ADC0 + 0x24, 0).expect("SC3");
}

/// ADC0_SE13 (PTB3, Arduino A1): write SC1A, poll COCO, read R[0].
fn k64f_convert(bus: &mut SystemBus) -> u32 {
    bus.write_u32(K64F_ADC0, 13).expect("SC1A");
    assert_ne!(bus.read_u32(K64F_ADC0).unwrap() & (1 << 7), 0, "COCO");
    bus.read_u32(K64F_ADC0 + 0x10).expect("R[0]")
}

const M4_ADC0: u64 = 0x4300_1c00;
const M4_PORTA: u64 = 0x4100_8000;

/// Clock ADC0, take VDDANA as the reference and hand PA04 to function B.
fn metro_m4_prepare(bus: &mut SystemBus) {
    bus.write_u32(0x4000_0820, 1 << 7).expect("MCLK APBDMASK");
    bus.write_u32(0x4000_1d20, 1 << 6).expect("GCLK PCHCTRL40");
    bus.write_u8(M4_ADC0 + 0x08, 3).expect("REFCTRL");
    bus.write_u8(M4_ADC0, 1 << 1).expect("CTRLA.ENABLE");
    let pmux = bus.read_u8(M4_PORTA + 0x30 + 2).expect("read PMUX");
    bus.write_u8(M4_PORTA + 0x30 + 2, (pmux & 0xf0) | 1)
        .expect("PA04 function B");
    bus.write_u8(M4_PORTA + 0x40 + 4, 1).expect("PA04 PMUXEN");
}

/// ADC0 AIN4 (PA04, Arduino A3): select, start, wait RESRDY, read RESULT.
fn metro_m4_convert(bus: &mut SystemBus) -> u32 {
    bus.write_u16(M4_ADC0 + 0x04, 0x1800 | 4)
        .expect("INPUTCTRL");
    bus.write_u8(M4_ADC0 + 0x14, 1 << 1).expect("SWTRIG.START");
    bus.tick_peripherals();
    assert_ne!(bus.read_u8(M4_ADC0 + 0x2e).unwrap() & 1, 0, "RESRDY");
    u32::from(bus.read_u16(M4_ADC0 + 0x40).expect("RESULT"))
}

#[test]
fn frdm_k64f_adc16_sees_a_remote_ntc_through_a_transmission_line() {
    cable_staircase_reaches_adc(
        "configs/systems/frdm-k64f.yaml",
        "PTB3",
        k64f_prepare,
        k64f_convert,
    );
}

#[test]
fn metro_m4_adc0_sees_a_remote_ntc_through_a_transmission_line() {
    cable_staircase_reaches_adc(
        "configs/systems/metro-m4.yaml",
        "PA04",
        metro_m4_prepare,
        metro_m4_convert,
    );
}
