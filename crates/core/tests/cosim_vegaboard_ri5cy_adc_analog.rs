// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the RV32M1 LPADC0
//! (`nxp_lpadc`, `layout: single_fifo`) on the onboarding VEGAboard through
//! the descriptor's `analog_pins` map (rv32m1_sdk_riscv pin_mux.c signal
//! lists: ADC0_SE2 = PTB7, ADC0_SE4 = PTC9): the NTC on PTB7 and the diode
//! on PTC9, read the way the SDK does:
//! - `LPADC_Init` (default config) and `LPADC_DoAutoCalibration`
//!   (`CFG.CALOFS`, one conversion into `OFSTRIM`);
//! - lpadc/polling: CMD1 on the channel, trigger 0 -> CMD1, `SWTRIG`,
//!   poll `RESFIFO.VALID`, `D >> 3` against VREFH / 4096;
//! - a `CMDH.NEXT` chain that converts both pads from one trigger.
//!
//! Also: the netlists moved to PTB3 (SE0) and PTD9 (SE12) convert there
//! while the old pads go back to 0.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC: u64 = 0x4004_A000;
const CTRL: u64 = ADC + 0x10;
const STAT: u64 = ADC + 0x14;
const CFG: u64 = ADC + 0x20;
const FCTRL: u64 = ADC + 0x30;
const SWTRIG: u64 = ADC + 0x34;
const OFSTRIM: u64 = ADC + 0x40;
const TCTRL0: u64 = ADC + 0xC0;
const CMDL1: u64 = ADC + 0x100;
const RESFIFO: u64 = ADC + 0x300;
const ADCEN: u32 = 1 << 0;
const RST: u32 = 1 << 1;
const DOZEN: u32 = 1 << 2;
const RSTFIFO: u32 = 1 << 8;
const CALOFS: u32 = 1 << 15;
const RDY: u32 = 1 << 0;
const VALID: u32 = 1 << 31;
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

/// 12-bit single-ended, 1 LSB = VREFH / 4096, truncated, clamped.
fn code(volts: f64) -> u32 {
    (millivolts(volts) * 4096 / VREF_MV).min(4095)
}

/// The onboarding VEGAboard with the generic NTC and diode netlists routed
/// to the two pads.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/onboarding/vegaboard_ri5cy.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load VEGAboard");
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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load RV32M1 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build VEGAboard bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to ADC0_SE pads through analog_pins"
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

fn command(bus: &mut SystemBus, id: u64, cmdl: u32, cmdh: u32) {
    bus.write_u32(CMDL1 + 8 * (id - 1), cmdl).expect("CMDL");
    bus.write_u32(CMDL1 + 8 * (id - 1) + 4, cmdh).expect("CMDH");
}

/// rv32m1_sdk fsl_lpadc.c `LPADC_Init` with `LPADC_GetDefaultConfig`, then
/// `LPADC_DoAutoCalibration`.
fn lpadc_init(bus: &mut SystemBus) {
    bus.write_u32(CTRL, RST).expect("CTRL.RST");
    bus.write_u32(CTRL, 0).expect("release");
    bus.write_u32(CTRL, RSTFIFO).expect("RSTFIFO");
    bus.write_u32(CTRL, DOZEN).expect("DOZEN");
    let cfg = 0x80 << 16;
    bus.write_u32(CFG, cfg).expect("CFG.PUDLY");
    bus.write_u32(FCTRL, 0).expect("FWMARK 0");
    // Nothing converts before ADCEN.
    command(bus, 1, 2, 0);
    bus.write_u32(TCTRL0, 1 << 24).expect("TCTRL0");
    bus.write_u32(SWTRIG, 1).expect("SWT0");
    assert_eq!(bus.read_u32(FCTRL).unwrap() & 0x1F, 0, "ADCEN clear");
    bus.write_u32(CTRL, DOZEN | ADCEN).expect("ADCEN");
    // LPADC_DoAutoCalibration: 128 averages, the result goes to OFSTRIM.
    bus.write_u32(CFG, cfg | CALOFS).expect("CALOFS");
    command(bus, 1, 0, 7 << 12);
    bus.write_u32(SWTRIG, 1).expect("SWT0");
    let entry = bus.read_u32(RESFIFO).unwrap();
    assert_ne!(entry & VALID, 0, "calibration conversion");
    bus.write_u32(OFSTRIM, (entry & 0xFFFF) >> 3)
        .expect("OFSTRIM");
    assert_eq!(bus.read_u32(OFSTRIM).unwrap(), 0, "ideal offset");
    bus.write_u32(CFG, cfg).expect("CALOFS off");
    command(bus, 1, 0, 0);
    bus.write_u32(TCTRL0, 0).expect("restore TCTRL0");
}

/// lpadc/polling: CMD1 = CHnA, trigger 0 -> CMD1, poll VALID, D >> 3.
fn adc_read(bus: &mut SystemBus, channel: u32) -> u32 {
    command(bus, 1, channel, 0);
    bus.write_u32(TCTRL0, 1 << 24).expect("TCTRL0");
    bus.write_u32(SWTRIG, 1).expect("SWT0");
    assert_ne!(bus.read_u32(STAT).unwrap() & RDY, 0, "SE{channel}: RDY");
    let entry = bus.read_u32(RESFIFO).unwrap();
    assert_ne!(entry & VALID, 0, "SE{channel}: VALID");
    assert_eq!((entry >> 24) & 0xF, 1, "CMDSRC 1");
    (entry & 0xFFFF) >> 3
}

#[test]
fn vegaboard_ptb7_and_ptc9_convert_ntc_and_diode_on_adc0_se2_and_se4() {
    let (mut bus, mut session) = board("PTB7", "PTC9");
    assert_eq!(bus.read_u32(ADC).unwrap(), 0x0100_0708, "VERID");
    lpadc_init(&mut bus);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [25.0_f64, 50.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.PTB7_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.PTC9_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc_code = adc_read(&mut bus, 2);
        assert_eq!(ntc_code, code(ntc_v), "SE2 at {temperature_c} °C");
        assert!(ntc_code < 4095, "below full scale at {temperature_c} °C");
        assert_eq!(adc_read(&mut bus, 4), code(diode_v), "SE4");
        for idle in [0, 5, 12] {
            assert_eq!(adc_read(&mut bus, idle), 0, "SE{idle} is not driven");
        }
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    // Firmware scales the result back: code * VREFH / 4096.
    let mv = ntc_codes[0] * VREF_MV / 4096;
    assert!(
        mv.abs_diff(millivolts(ntc_volts(25.0))) <= 1,
        "code * VREFH / 4096 gives {mv} mV"
    );
    // At -10 °C the divider (about 2.8 V) is above VREFH: full scale.
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    assert_eq!(adc_read(&mut bus, 2), 4095, "over-range clamps");
}

#[test]
fn vegaboard_next_chain_converts_both_pads_from_one_trigger() {
    let (mut bus, mut session) = board("PTB7", "PTC9");
    lpadc_init(&mut bus);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let ntc = code(signal_voltage(&session, "board.analog.PTB7_volts"));
    let diode = code(signal_voltage(&session, "board.analog.PTC9_volts"));
    // CMD1 (SE2) -> CMD2 (SE4), started by trigger 2.
    command(&mut bus, 1, 2, 2 << 24);
    command(&mut bus, 2, 4, 0);
    bus.write_u32(TCTRL0 + 8, 1 << 24).expect("TCTRL2");
    bus.write_u32(SWTRIG, 1 << 2).expect("SWT2");
    assert_eq!(bus.read_u32(FCTRL).unwrap() & 0x1F, 2, "FCOUNT");
    let entries: Vec<u32> = (0..2).map(|_| bus.read_u32(RESFIFO).unwrap()).collect();
    let got: Vec<(u32, u32, u32)> = entries
        .iter()
        .map(|e| ((e >> 16) & 3, (e >> 24) & 0xF, (e & 0xFFFF) >> 3))
        .collect();
    assert_eq!(got, [(2, 1, ntc), (2, 2, diode)], "(TSRC, CMDSRC, code)");
    assert_eq!(bus.read_u32(RESFIFO).unwrap(), 0, "drained: VALID 0");
}

#[test]
fn vegaboard_ptb3_and_ptd9_convert_after_the_netlists_move() {
    let (mut bus, mut session) = board("PTB3", "PTD9");
    lpadc_init(&mut bus);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_v = signal_voltage(&session, "board.analog.PTB3_volts");
    let diode_v = signal_voltage(&session, "board.analog.PTD9_volts");
    assert_eq!(adc_read(&mut bus, 0), code(ntc_v), "SE0");
    assert_eq!(adc_read(&mut bus, 12), code(diode_v), "SE12");
    assert_eq!(adc_read(&mut bus, 2), 0, "PTB7 is not driven");
    assert_eq!(adc_read(&mut bus, 4), 0, "PTC9 is not driven");
}
