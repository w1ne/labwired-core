// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the DA14592 GPADC
//! through the `analog_pins` pad map: the NTC on `P1_9` (`GP_ADC_SEL_P` 12)
//! and the diode on `P0_10` (`SEL_P` 3). Pad names resolve to the mux values
//! of the SDK10 `HW_GPADC_INP_Px_y` enum, so this covers the split mux
//! (0..3 and 10..13) that the explicit `adc.gpadc.<ch>_volts` sweep in
//! `cosim_onboarding_analog.rs` does not. Conversions use the SDK register
//! sequence (`hw_gpadc_adc_measure`, `hw_gpadc_get_value`).

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const GPADC: u64 = 0x5004_0900;
const CTRL: u64 = GPADC;
const CTRL2: u64 = GPADC + 0x04;
const SEL: u64 = GPADC + 0x0C;
const CLEAR_INT: u64 = GPADC + 0x1C;
const RESULT: u64 = GPADC + 0x20;
const EN: u32 = 1 << 0;
const START: u32 = 1 << 1;
const INT: u32 = 1 << 4;
const SE: u32 = 1 << 6;

const SEL_P0_10: u32 = 3;
const SEL_P1_9: u32 = 12;
const SEL_P1_11: u32 = 13;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// hw_gpadc_configure: single-ended, `GP_ADC_ATTN`, one sample, enable.
fn configure(bus: &mut SystemBus, attn: u32) {
    bus.write_u32(CTRL, SE).unwrap();
    let ctrl2 = bus.read_u32(CTRL2).unwrap() & !(3 | (7 << 6));
    bus.write_u32(CTRL2, ctrl2 | attn).unwrap();
    bus.write_u32(CTRL, SE | EN).unwrap();
}

/// SEL_P, clear INT, hw_gpadc_adc_measure, hw_gpadc_get_value (n = 0).
fn measure(bus: &mut SystemBus, sel_p: u32) -> u32 {
    bus.write_u32(SEL, sel_p << 4).unwrap();
    bus.write_u32(CLEAR_INT, 1).unwrap();
    let ctrl = bus.read_u32(CTRL).unwrap();
    bus.write_u32(CTRL, ctrl | START).unwrap();
    let ctrl = bus.read_u32(CTRL).unwrap();
    assert_eq!(ctrl & START, 0, "GP_ADC_START clears when done");
    assert_ne!(ctrl & INT, 0, "the conversion sets GP_ADC_INT");
    (bus.read_u32(RESULT).unwrap() & 0xFFFF) >> 6
}

fn code(volts: f64, full_scale_mv: u32) -> u32 {
    let mv = (volts * 1000.0).round() as u32;
    mv.min(full_scale_mv) * 1023 / full_scale_mv
}

#[test]
fn da14592_ntc_and_diode_netlists_convert_through_p1_9_and_p0_10_pads() {
    let system_path = root("configs/systems/onboarding/renesas-da14592.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load DA14592 profile");
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load DA14592 descriptor");
    let mut analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    for model in &mut analog.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", "p1_9"),
            "diode_load_line" => ("v_diode", "p0_10"),
            other => panic!("unexpected fixture model {other}"),
        };
        model
            .outputs
            .insert(key.into(), format!("board.analog.{pad}_volts"));
    }
    profile.cosim_models = analog.cosim_models;
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build DA14592 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "P1_9/P0_10 must resolve through analog_pins to GP_ADC_SEL_P 12/3"
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
        assert_eq!(steps.len(), 2, "both netlists solve each step");
        assert!(errors.is_empty(), "route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.p1_9_volts");
        let r = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * r / (10_000.0 + r);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.p0_10_volts");
        assert!(
            (0.5..0.8).contains(&diode_v),
            "a forward-biased silicon diode: {diode_v} V"
        );

        // ATTN = 3: 3.6 V full scale resolves the 3.3 V divider.
        configure(&mut bus, 3);
        let ntc_code = measure(&mut bus, SEL_P1_9);
        assert_eq!(ntc_code, code(ntc_v, 3600), "P1_9 converts the NTC");
        assert_eq!(
            measure(&mut bus, SEL_P0_10),
            code(diode_v, 3600),
            "P0_10 converts the diode"
        );
        assert_eq!(measure(&mut bus, SEL_P1_11), 0, "unrouted P1_11 reads 0");
        // ATTN = 0: 0.9 V full scale. The diode fits and gains four times
        // the resolution; the divider saturates except when hot.
        configure(&mut bus, 0);
        assert_eq!(measure(&mut bus, SEL_P0_10), code(diode_v, 900));
        let narrow = measure(&mut bus, SEL_P1_9);
        assert_eq!(narrow, code(ntc_v, 900));
        assert_eq!(narrow == 1023, ntc_v >= 0.9, "saturates above 0.9 V");
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "P1_9 codes fall as the low-side NTC heats: {ntc_codes:?}"
    );
}

/// Two cross-coupled latches, each with one device flagged SPICE `off`. The
/// NPN latch has `Q1 off`, so `Q1` starts cut off and `Q2` saturates; the
/// NMOS latch has `M2 off`, the opposite side. Without the flag both latches
/// sit at their symmetric metastable point.
const LATCHES: &str = r#"
cosim_models:
  - id: "bjt_latch"
    adapter: "analog"
    step_ns: 1000
    outputs:
      v_c1: "board.analog.p1_0_volts"
      v_c2: "board.analog.p1_1_volts"
    config:
      netlist_text: |
        Vcc vcc 0 dc 3.3
        Rc1 vcc c1 1k
        Rc2 vcc c2 1k
        Rb1 c2 b1 10k
        Rb2 c1 b2 10k
        Q1 c1 b1 0 QN off
        Q2 c2 b2 0 QN
        .model QN NPN(IS=1e-15 BF=100 BR=1)
      probes:
        v_c1: "v(c1)"
        v_c2: "v(c2)"
      vdd: 3.3
  - id: "nmos_latch"
    adapter: "analog"
    step_ns: 1000
    outputs:
      v_d1: "board.analog.p1_5_volts"
      v_d2: "board.analog.p1_6_volts"
    config:
      netlist_text: |
        Vdd vdd 0 dc 3.3
        Rd1 vdd d1 10k
        Rd2 vdd d2 10k
        M1 d1 d2 0 0 NM w=10u l=1u
        M2 d2 d1 0 0 NM w=10u l=1u off
        .model NM NMOS(LEVEL=1 VTO=0.7 KP=50u LAMBDA=0.02)
      probes:
        v_d1: "v(d1)"
        v_d2: "v(d2)"
      vdd: 3.3
"#;

#[test]
fn da14592_off_flag_latches_reach_gpadc_beside_ntc_and_diode() {
    let system_path = root("configs/systems/onboarding/renesas-da14592.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load DA14592 profile");
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load DA14592 descriptor");
    let mut analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    for model in &mut analog.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", "p1_9"),
            "diode_load_line" => ("v_diode", "p0_10"),
            other => panic!("unexpected fixture model {other}"),
        };
        model
            .outputs
            .insert(key.into(), format!("board.analog.{pad}_volts"));
    }
    let latches: SystemManifest = serde_yaml::from_str(&format!(
        "name: \"da14592-latches\"\nchip: \"{}\"\nexternal_devices: []\n{LATCHES}",
        chip_path.display()
    ))
    .expect("the latch block parses as a system manifest");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(latches.cosim_models);
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build DA14592 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries four netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "P1_0/P1_1/P1_5/P1_6 must resolve through analog_pins to SEL_P 0/1/10/11"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("hold the NTC at 25 °C");
    let cycles = session.cycles_until_boundary(0);
    let (steps, errors) = session
        .advance_to(cycles, &mut bus)
        .expect("solve and route every netlist");
    assert_eq!(steps.len(), 4, "NTC, diode and both latches solve");
    assert!(errors.is_empty(), "route errors: {errors:?}");

    // ngspice-42 `.op` of the same netlists (`.temp 26.85`, `reltol=1e-7`).
    // Without `off` both latches sit at their symmetric point instead: both
    // BJT collectors at 0.967 V and both NMOS drains at 1.529 V.
    configure(&mut bus, 3);
    let mut codes = Vec::new();
    for (pad, sel_p, ngspice_v) in [
        ("p1_0", 0, 3.067_958_989),
        ("p1_1", 1, 0.075_377_679),
        ("p1_5", 10, 0.245_402_665),
        ("p1_6", 11, 3.299_999_967),
    ] {
        let solved = signal_voltage(&session, &format!("board.analog.{pad}_volts"));
        assert!(
            (solved - ngspice_v).abs() < 1e-4,
            "{pad}: ngspice-42 {ngspice_v} V, got {solved} V"
        );
        let converted = measure(&mut bus, sel_p);
        assert_eq!(
            converted,
            code(solved, 3600),
            "{pad} converts the latch node"
        );
        codes.push(converted);
    }
    assert!(
        codes[0] > 800 && codes[1] < 30,
        "Q1 off: c1 high, c2 saturated low: {codes:?}"
    );
    assert!(
        codes[2] < 80 && codes[3] > 900,
        "M2 off: d2 high, d1 pulled low: {codes:?}"
    );

    // The NTC and diode channels still convert beside the latches.
    let ntc_v = signal_voltage(&session, "board.analog.p1_9_volts");
    assert!(
        (ntc_v - 1.65).abs() < 1e-9,
        "NTC divider at 25 °C is mid-rail"
    );
    assert_eq!(measure(&mut bus, SEL_P1_9), code(ntc_v, 3600));
    let diode_v = signal_voltage(&session, "board.analog.p0_10_volts");
    assert!((0.5..0.8).contains(&diode_v), "forward diode: {diode_v} V");
    assert_eq!(measure(&mut bus, SEL_P0_10), code(diode_v, 3600));
}
