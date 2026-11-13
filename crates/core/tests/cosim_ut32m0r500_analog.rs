// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the UT32M0R500 ADC
//! through the `analog_pins` map: the NTC on `AIN12` and the diode on `AIN8`.
//! One `ADC_TRIGGER` sweep converts them three ways at once, which the
//! explicit `adc.adc.<ch>_volts` sweep in `cosim_onboarding_analog.rs` does
//! not cover:
//! - the NTC single-ended through PGA 0.5 V/V (3 V range) and 1 V/V
//!   (1.5 V range, saturating except when hot),
//! - the diode single-ended through PGA 2 V/V (0.75 V range),
//! - the diode differentially on pair 4 (AIN8 - AIN9, AIN9 unrouted).
//!
//! The register sequence follows the CAES ADC Users Guide Example 1:
//! `ADC_REGDEF`, single sweep with `ADC_EN`, `SECHAN_CFG` enable and gain,
//! `ADC_TRIGGER`, poll `INT_STATUS`, read `DATA_OUT[11:0]`, flip bit 11.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC: u64 = 0x4000_F000;
const SPB_CFG_0: u64 = ADC;
const SPB_CFG_1: u64 = ADC + 0x04;
const INT_STATUS: u64 = ADC + 0x8C;
const ADC_EN: u32 = 1 << 0;
const EN_PGA: u32 = 1 << 1;
const SINGLESWEEP: u32 = 1 << 13;
const TRIGGER: u32 = 1 << 0;
const REGDEF: u32 = 1 << 31;
const CONV_COMPL_COMB: u32 = 1 << 31;

const GAIN_HALF: u32 = 0;
const GAIN_1: u32 = 1;
const GAIN_2: u32 = 2;

fn sechan_cfg(ain: u64) -> u64 {
    ADC + 0x10 + 4 * ain
}

fn diffchan_cfg(pair: u64) -> u64 {
    ADC + 0x50 + 4 * pair
}

fn data(word: u64) -> u64 {
    ADC + 0x90 + 4 * word
}

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
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

/// Single-ended offset-binary code with the input range `range_mv`.
fn se_code(volts: f64, range_mv: u32) -> u32 {
    millivolts(volts).min(range_mv) * 4095 / range_mv
}

/// One sweep of `channels` (config offset, GAIN), polled to completion.
fn sweep(bus: &mut SystemBus, channels: &[(u64, u32)]) {
    bus.write_u32(SPB_CFG_1, REGDEF).unwrap();
    bus.write_u32(SPB_CFG_0, SINGLESWEEP | EN_PGA | ADC_EN)
        .unwrap();
    for &(cfg, gain) in channels {
        bus.write_u32(cfg, (gain << 4) | 1).unwrap();
    }
    bus.write_u32(SPB_CFG_1, TRIGGER).unwrap();
    let status = bus.read_u32(INT_STATUS).unwrap();
    assert_ne!(
        status & CONV_COMPL_COMB,
        0,
        "the sweep completes: {status:#x}"
    );
}

/// `DATA_OUT[11:0]` with bit 11 flipped back to offset binary (Code 4).
fn read_se(bus: &mut SystemBus, ain: u64) -> u32 {
    (bus.read_u32(data(ain)).unwrap() & 0xFFF) ^ 0x800
}

#[test]
fn ut32m0r500_ntc_and_diode_netlists_convert_through_ain12_and_ain8() {
    let system_path = root("configs/systems/onboarding/ut32m0r500.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load UT32M0R500 profile");
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load UT32M0R500 descriptor");
    let mut analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    for model in &mut analog.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", "ain12"),
            "diode_load_line" => ("v_diode", "ain8"),
            other => panic!("unexpected fixture model {other}"),
        };
        model
            .outputs
            .insert(key.into(), format!("board.analog.{pad}_volts"));
    }
    profile.cosim_models = analog.cosim_models;
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build UT32M0R500 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "AIN12/AIN8 must resolve through analog_pins to ADC inputs 12/8"
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

        let ntc_v = signal_voltage(&session, "board.analog.ain12_volts");
        let r = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * r / (10_000.0 + r);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.ain8_volts");
        assert!(
            (0.5..0.75).contains(&diode_v),
            "a forward-biased silicon diode inside the 2 V/V range: {diode_v} V"
        );

        // NTC at 0.5 V/V and the diode at 2 V/V, plus the unrouted AIN13.
        sweep(
            &mut bus,
            &[
                (sechan_cfg(12), GAIN_HALF),
                (sechan_cfg(8), GAIN_2),
                (sechan_cfg(13), GAIN_1),
            ],
        );
        let ntc_code = read_se(&mut bus, 12);
        assert_eq!(ntc_code, se_code(ntc_v, 3000), "AIN12 converts the NTC");
        assert_eq!(
            read_se(&mut bus, 8),
            (millivolts(diode_v) * 2).min(1500) * 4095 / 1500,
            "AIN8 converts the diode at 2 V/V"
        );
        assert_eq!(read_se(&mut bus, 13), 0, "unrouted AIN13 reads 0 V");
        ntc_codes.push(ntc_code);

        // NTC at 1 V/V: 1.5 V full scale saturates the -10/25 °C dividers.
        // Diode differentially on pair 4 (AIN8 - AIN9): (Vd + 1.5 V) / 3 V.
        sweep(
            &mut bus,
            &[(sechan_cfg(12), GAIN_1), (diffchan_cfg(4), GAIN_1)],
        );
        let narrow = read_se(&mut bus, 12);
        assert_eq!(narrow, se_code(ntc_v, 1500));
        assert_eq!(narrow == 4095, ntc_v >= 1.5, "saturates above 1.5 V");
        assert_eq!(
            bus.read_u32(data(8)).unwrap() & 0xFFF,
            (millivolts(diode_v) + 1500) * 4095 / 3000,
            "differential pair 4 lands in data word 8"
        );
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "AIN12 codes fall as the low-side NTC heats: {ntc_codes:?}"
    );
}
