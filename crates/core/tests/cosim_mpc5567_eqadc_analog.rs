// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the eQADC
//! (`mpc55xx_eqadc`, the MPC5567 header's `EQADC` at 0xFFF8_0000) on the
//! onboarding MPC5567 through the descriptor's `analog_pins` map (AN0..AN39
//! are conversion channels 0..39). The NTC is on AN12 and the diode on AN31,
//! read three ways:
//! - the OpenSAR `Adc_eQADC.c` flow: `Adc_ConfigureEQADC` enables ADC0 with
//!   a configuration write through CFIFO0, `Adc_EQADCCalibrationSequence`
//!   converts the 25 % and 75 % references, computes GCC/OCC, writes them
//!   and reads them back; then calibrated conversions (`CAL` = 1) of the NTC
//!   on ADC0 and of the diode on ADC1 (`BN` = 1);
//! - a software continuous scan (`MODE` 9) on CFIFO1 with `EOQIE`, the
//!   results in RFIFO1, while the NTC warms;
//! - the netlists moved to AN0 and AN39, while AN12 and AN31 go back to 0.
//!
//! VRH - VRL is the nominal 5 V, so every divider stays in range: the code
//! is mV * 4096 / 5000 (12 bits) and the RFIFO holds it shifted left 2.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const EQADC: u64 = 0xFFF8_0000;
const CFPR0: u64 = 0x10;
const RFPR0: u64 = 0x30;
const CFCR0: u64 = 0x50;
const IDCR0: u64 = 0x60;
const FISR0: u64 = 0x70;
const CFTCR0: u64 = 0x90;
const CFSR: u64 = 0xAC;
const CFCR_SSE: u16 = 1 << 10;
const CFCR_CFINV: u16 = 1 << 9;
const MODE_SWSS: u16 = 1 << 4;
const MODE_SWCS: u16 = 9 << 4;
const IDCR_EOQIE: u16 = 1 << 12;
const FISR_EOQF: u32 = 1 << 28;
const FISR_SSS: u32 = 1 << 26;
const FISR_RFDF: u32 = 1 << 17;
const CCM_EOQ: u32 = 1 << 31;
const CCM_BN_ADC1: u32 = 1 << 25;
const CCM_CAL: u32 = 1 << 24;
/// Configuration command `R/W` (same bit as `CAL`).
const RW_READ: u32 = 1 << 24;
const LST_128: u32 = 3 << 18;
const ADC_CR: u32 = 1;
const ADC_GCCR: u32 = 4;
const ADC_OCCR: u32 = 5;
const ADC_ENABLE: u32 = 0x8000;
/// OpenSAR `Adc_eQADC.c` `IDEAL_RES25` / `IDEAL_RES75`.
const IDEAL_RES25: u32 = 0x1000;
const IDEAL_RES75: u32 = 0x3000;
/// The profile's `vref_mv` (VRH - VRL).
const VREF_MV: u32 = 5000;

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

/// The 14-bit RFIFO result: 12-bit code (2^12 / VRH, truncated, clamped)
/// shifted left 2.
fn result(volts: f64) -> u32 {
    (millivolts(volts) * 4096 / VREF_MV).min(4095) << 2
}

fn conversion(bn: u32, tag: u32, channel: u32) -> u32 {
    bn | (tag << 20) | LST_128 | (channel << 8)
}

/// The onboarding MPC5567 with the generic NTC and diode netlists routed to
/// the two pads.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/onboarding/mpc5567.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load MPC5567");
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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load MPC5567 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build MPC5567 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to eQADC channels through analog_pins"
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

fn eqadc_irq(bus: &SystemBus) -> bool {
    bus.peripherals
        .iter()
        .find(|p| p.name == "eqadc")
        .expect("eqadc on the bus")
        .dev
        .irq_line_level()
        .expect("mpc55xx_eqadc reports its request")
}

fn fisr(bus: &SystemBus, n: u64) -> u32 {
    bus.read_u32(EQADC + FISR0 + 4 * n).unwrap()
}

/// OpenSAR `Adc_WriteEQADCRegister` / `Adc_ReadEQADCRegister` on CFIFO0:
/// `CFINV`, push the queue (`EOQ` on the last), `MODE` 1, `SSE`,
/// `WAIT_FOR_QUEUE_TO_FINISH`, then pop every result and clear `EOQF`.
fn run_queue(bus: &mut SystemBus, cmds: &[u32]) -> Vec<u32> {
    let cfcr = EQADC + CFCR0;
    bus.write_u16(cfcr, CFCR_CFINV).unwrap();
    for (i, &cmd) in cmds.iter().enumerate() {
        let eoq = if i + 1 == cmds.len() { CCM_EOQ } else { 0 };
        bus.write_u32(EQADC + CFPR0, cmd | eoq).unwrap();
    }
    let old_mode = bus.read_u16(cfcr).unwrap();
    bus.write_u16(cfcr, MODE_SWSS).unwrap();
    let mode = bus.read_u16(cfcr).unwrap();
    bus.write_u16(cfcr, mode | CFCR_SSE).unwrap();
    let mut polls = 0;
    while fisr(bus, 0) & FISR_SSS != 0 {
        bus.tick_peripherals();
        polls += 1;
        assert!(polls <= cmds.len(), "one command per tick");
    }
    assert_ne!(fisr(bus, 0) & FISR_EOQF, 0, "EOQF");
    let count = (fisr(bus, 0) >> 4) & 0xF;
    let results = (0..count)
        .map(|_| u32::from(bus.read_u16(EQADC + RFPR0 + 2).unwrap()))
        .collect();
    let status = fisr(bus, 0);
    bus.write_u32(EQADC + FISR0, status).unwrap();
    bus.write_u16(cfcr, old_mode).unwrap();
    results
}

/// `Adc_ConfigureEQADC` + `Adc_EQADCCalibrationSequence` for one converter:
/// enable, four samples each of 25 % (channel 44) and 75 % (channel 43),
/// GCC/OCC, write, read back. Returns `(gcc, occ)`.
fn opensar_calibrate(bus: &mut SystemBus, bn: u32) -> (u32, u32) {
    assert!(run_queue(bus, &[bn | ((ADC_ENABLE | 0x09) << 8) | ADC_CR]).is_empty());
    let r25 = run_queue(bus, &[conversion(bn, 0, 44); 4]);
    let r75 = run_queue(bus, &[conversion(bn, 0, 43); 4]);
    assert_eq!((r25.len(), r75.len()), (4, 4), "four samples each");
    let p25 = r25.iter().sum::<u32>() / 4;
    let p75 = r75.iter().sum::<u32>() / 4;
    let gcc = (((IDEAL_RES75 - IDEAL_RES25) << 14) / (p75 - p25)) & !(1 << 15);
    let occ = (IDEAL_RES75 as i32 - ((gcc * p75) >> 14) as i32 - 2) as u32 & 0x3FFF;
    assert!(run_queue(
        bus,
        &[bn | (gcc << 8) | ADC_GCCR, bn | (occ << 8) | ADC_OCCR,]
    )
    .is_empty());
    let back = run_queue(bus, &[bn | RW_READ | ADC_GCCR, bn | RW_READ | ADC_OCCR]);
    assert_eq!(back, vec![gcc, occ], "Adc_ReadEQADCRegister read-back");
    (gcc, occ)
}

#[test]
fn mpc5567_an12_and_an31_convert_ntc_and_diode_with_opensar_calibration() {
    let (mut bus, mut session) = board("AN12", "AN31");
    assert_eq!(fisr(&bus, 0), 1 << 25, "reset: only CFFF");

    // Before Adc_ConfigureEQADC, ADC0_CR.EN = 0: the queue ends, no result.
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    assert!(run_queue(&mut bus, &[conversion(0, 0, 12)]).is_empty());

    // On the ideal references the calibration is the identity.
    assert_eq!(opensar_calibrate(&mut bus, 0), (0x4000, 0x3FFE));
    assert_eq!(opensar_calibrate(&mut bus, CCM_BN_ADC1), (0x4000, 0x3FFE));

    let mut ntc_results = Vec::new();
    for temperature_c in [25.0_f64, 50.0, 80.0, -10.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.AN12_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.AN31_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let r = run_queue(
            &mut bus,
            &[
                conversion(0, 0, 12) | CCM_CAL,
                conversion(CCM_BN_ADC1, 0, 31) | CCM_CAL,
                conversion(0, 0, 13) | CCM_CAL,
                conversion(CCM_BN_ADC1, 0, 30),
            ],
        );
        assert_eq!(
            r,
            vec![result(ntc_v), result(diode_v), 0, 0],
            "AN12, AN31, idle AN13 and AN30 at {temperature_c} °C"
        );
        assert!(
            r[0] < 0x3FFC,
            "5 V reference: in range at {temperature_c} °C"
        );
        ntc_results.push(r[0]);
    }
    assert!(
        ntc_results[3] > ntc_results[0]
            && ntc_results[0] > ntc_results[1]
            && ntc_results[1] > ntc_results[2],
        "the NTC results fall as it warms: {ntc_results:?}"
    );
    let mv = (ntc_results[0] >> 2) * VREF_MV / 4096;
    assert!(
        mv.abs_diff(millivolts(ntc_volts(25.0))) <= 2,
        "code * VRH / 4096 gives {mv} mV"
    );

    // A gain error: GCC 0x4800 (x 1.125) and OCC 0 scale the calibrated
    // NTC result; the uncalibrated one is unchanged.
    run_queue(&mut bus, &[(0x4800 << 8) | ADC_GCCR, ADC_OCCR]);
    let ntc_v = signal_voltage(&session, "board.analog.AN12_volts");
    let r = run_queue(
        &mut bus,
        &[conversion(0, 0, 12) | CCM_CAL, conversion(0, 0, 12)],
    );
    assert_eq!(r, vec![((result(ntc_v) * 9) >> 3) + 2, result(ntc_v)]);
}

#[test]
fn mpc5567_continuous_scan_on_cfifo1_follows_the_warming_ntc() {
    let (mut bus, mut session) = board("AN12", "AN31");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    run_queue(&mut bus, &[((ADC_ENABLE | 0x09) << 8) | ADC_CR]);
    let ntc_25 = result(signal_voltage(&session, "board.analog.AN12_volts"));
    let diode = result(signal_voltage(&session, "board.analog.AN31_volts"));

    // CFIFO1, software continuous scan, EOQ interrupt; results to RFIFO1.
    bus.write_u16(EQADC + IDCR0 + 2, IDCR_EOQIE).unwrap();
    bus.write_u32(EQADC + CFPR0 + 4, conversion(0, 1, 12))
        .unwrap();
    bus.write_u32(EQADC + CFPR0 + 4, conversion(0, 1, 31) | CCM_EOQ)
        .unwrap();
    // AN31 is an ADC0 channel too: either converter reaches every pad.
    bus.write_u16(EQADC + CFCR0 + 2, MODE_SWCS).unwrap();
    assert_eq!(
        (bus.read_u32(EQADC + CFSR).unwrap() >> 28) & 0b11,
        0b11,
        "CFIFO1 triggered"
    );
    bus.tick_peripherals();
    assert!(!eqadc_irq(&bus), "no EOQ yet");
    bus.tick_peripherals();
    assert!(eqadc_irq(&bus), "EOQF with EOQIE");
    assert_eq!(fisr(&bus, 1) & FISR_RFDF, FISR_RFDF);
    assert_eq!(bus.read_u32(EQADC + RFPR0 + 4).unwrap(), ntc_25);
    assert_eq!(bus.read_u32(EQADC + RFPR0 + 4).unwrap(), diode);
    bus.write_u32(EQADC + FISR0 + 4, FISR_EOQF).unwrap();
    assert!(!eqadc_irq(&bus));

    // The ISR refills the queue; the NTC has warmed in the meantime.
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_80 = result(signal_voltage(&session, "board.analog.AN12_volts"));
    assert!(ntc_80 < ntc_25);
    bus.write_u32(EQADC + CFPR0 + 4, conversion(0, 1, 12) | CCM_EOQ)
        .unwrap();
    bus.tick_peripherals();
    assert!(eqadc_irq(&bus));
    assert_eq!(
        bus.read_u32(EQADC + RFPR0 + 4).unwrap(),
        ntc_80,
        "continuous scan follows the NTC"
    );
    assert_eq!(
        bus.read_u16(EQADC + CFTCR0 + 2).unwrap(),
        3,
        "three transfers"
    );

    // MODE 0 stops the scan: a pushed command waits.
    bus.write_u32(EQADC + FISR0 + 4, FISR_EOQF).unwrap();
    bus.write_u16(EQADC + CFCR0 + 2, 0).unwrap();
    bus.write_u32(EQADC + CFPR0 + 4, conversion(0, 1, 12) | CCM_EOQ)
        .unwrap();
    bus.tick_peripherals();
    assert_eq!((fisr(&bus, 1) >> 12) & 0xF, 1, "the command waits");
    assert_eq!((fisr(&bus, 1) >> 4) & 0xF, 0, "no result");
}

#[test]
fn mpc5567_an0_and_an39_convert_after_the_netlists_move() {
    let (mut bus, mut session) = board("AN0", "AN39");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 50.0);
    run_queue(&mut bus, &[((ADC_ENABLE | 0x09) << 8) | ADC_CR]);
    let ntc_v = signal_voltage(&session, "board.analog.AN0_volts");
    let diode_v = signal_voltage(&session, "board.analog.AN39_volts");
    assert_eq!(
        run_queue(
            &mut bus,
            &[
                conversion(0, 0, 0),
                conversion(0, 0, 39),
                conversion(0, 0, 12),
                conversion(0, 0, 31),
            ]
        ),
        vec![result(ntc_v), result(diode_v), 0, 0],
        "AN0, AN39; AN12 and AN31 are not driven"
    );
}
