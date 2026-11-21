// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the onboarding
//! STM32L151's ADC1 (`stm32l1_adc`) through the descriptor's `analog_pins`
//! map (STM32duino L151ZCT `PinMap_ADC`), read the way the STM32CubeL1 HAL
//! does it: `ADC_Enable` (`CR2.ADON`, poll `SR.ADONS`), `ConfigChannel`
//! (rank 1 in `SQR5`, `CR2.CFG` for bank B), `Start` (`SWSTART`),
//! `PollForConversion` (`EOC`, cleared by writing its complement), then
//! `GetValue` (`DR`). 12 bits against VREF+ 3.3 V. Also:
//! - the diode on PB2, which is `ADC_IN0b`: channel 0 converts it only with
//!   bank B selected, and PA0 (`ADC_IN0`, bank A) with bank A;
//! - a two-rank scan with `EOCS` over pads in different SQR registers' worth
//!   of channel numbers (IN31 on PF10, IN22 on PE7), and the analog
//!   watchdog on the NTC channel raising `AWD` and IRQ 18 once the NTC warms
//!   below the `LTR` threshold.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC: u64 = 0x4001_2400;
const SR: u64 = 0x00;
const CR1: u64 = 0x04;
const CR2: u64 = 0x08;
const LTR: u64 = 0x2C;
const SQR1: u64 = 0x30;
const SQR5: u64 = 0x40;
const DR: u64 = 0x58;
const AWD: u32 = 1 << 0;
const EOC: u32 = 1 << 1;
const ADONS: u32 = 1 << 6;
const SCAN: u32 = 1 << 8;
const AWDSGL: u32 = 1 << 9;
const AWDIE: u32 = 1 << 6;
const AWDEN: u32 = 1 << 23;
const ADON: u32 = 1 << 0;
const CFG: u32 = 1 << 2;
const EOCS: u32 = 1 << 10;
const SWSTART: u32 = 1 << 30;

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

/// 12 bits, mV * 4095 / 3300, truncated, clamped.
fn code(volts: f64) -> u32 {
    (millivolts(volts) * 4095 / 3300).min(4095)
}

/// The onboarding STM32L151 with the generic NTC and diode netlists routed
/// to the two pads.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/onboarding/stm32l151.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load STM32L151");
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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L151 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build STM32L151 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to ADC1 inputs through analog_pins"
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

/// The HAL single-conversion flow on `channel`, bank B when `bank_b`.
fn hal_read(bus: &mut SystemBus, channel: u32, bank_b: bool) -> u32 {
    let cr2 = ADON | if bank_b { CFG } else { 0 };
    bus.write_u32(ADC + CR2, cr2).expect("ADON, CFG");
    assert_ne!(bus.read_u32(ADC + SR).unwrap() & ADONS, 0, "ADONS");
    bus.write_u32(ADC + SQR5, channel).expect("SQR5 SQ1");
    bus.write_u32(ADC + SR, !EOC).expect("clear EOC");
    bus.write_u32(ADC + CR2, cr2 | SWSTART).expect("SWSTART");
    assert_ne!(bus.read_u32(ADC + SR).unwrap() & EOC, 0, "IN{channel}: EOC");
    bus.write_u32(ADC + SR, !EOC)
        .expect("PollForConversion clears EOC");
    bus.read_u32(ADC + DR).expect("DR")
}

#[test]
fn stm32l151_pa5_and_bank_b_pb2_convert_ntc_and_diode() {
    let (mut bus, mut session) = board("pa5", "pb2");
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.pa5_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.pb2_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc_code = hal_read(&mut bus, 5, false);
        assert_eq!(ntc_code, code(ntc_v), "IN5 (PA5) at {temperature_c} °C");
        // IN5 has no bank-B pad, so CFG does not move it.
        assert_eq!(hal_read(&mut bus, 5, true), ntc_code, "IN5 is common");
        assert_eq!(hal_read(&mut bus, 0, true), code(diode_v), "IN0b (PB2)");
        assert_eq!(hal_read(&mut bus, 0, false), 0, "IN0 (PA0) is not driven");
        assert_eq!(hal_read(&mut bus, 7, false), 0, "IN7 (PA7) is not driven");
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    // The firmware inverse, code * 3300 / 4095, recovers the 25 °C divider.
    let mv = ntc_codes[1] * 3300 / 4095;
    assert!(
        mv.abs_diff(millivolts(ntc_volts(25.0))) <= 1,
        "code * VREF / 4095 gives {mv} mV"
    );
}

#[test]
fn stm32l151_scan_and_watchdog_follow_the_ntc_on_pf10_and_diode_on_pe7() {
    let (mut bus, mut session) = board("pf10", "pe7");
    let mut cycles = 0;
    // Watchdog on IN31 alone: low threshold at the 25 °C divider (1.65 V).
    let ltr = code(ntc_volts(25.0)) - 50;
    bus.write_u32(ADC + LTR, ltr).expect("LTR");
    bus.write_u32(ADC + CR1, SCAN | AWDEN | AWDSGL | AWDIE | 31)
        .expect("CR1");
    // SQ1 = IN31 (PF10), SQ2 = IN22 (PE7), L = 1.
    bus.write_u32(ADC + SQR5, 31 | (22 << 5)).expect("SQR5");
    bus.write_u32(ADC + SQR1, 1 << 20).expect("SQR1.L");
    for (temperature_c, alarm) in [(-10.0_f64, false), (25.0, false), (80.0, true)] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.pf10_volts");
        let diode_v = signal_voltage(&session, "board.analog.pe7_volts");
        bus.write_u32(ADC + SR, 0).expect("clear flags");
        bus.write_u32(ADC + CR2, ADON | EOCS | SWSTART)
            .expect("SWSTART");
        let mut codes = Vec::new();
        while bus.read_u32(ADC + SR).unwrap() & EOC != 0 {
            codes.push(bus.read_u32(ADC + DR).expect("DR"));
        }
        assert_eq!(
            codes,
            [code(ntc_v), code(diode_v)],
            "SQ1/SQ2 at {temperature_c} °C"
        );
        let sr = bus.read_u32(ADC + SR).unwrap();
        assert_eq!(sr & AWD != 0, alarm, "AWD at {temperature_c} °C");
        let irq = bus.peripherals[bus.find_peripheral_index_by_name("adc1").unwrap()]
            .dev
            .irq_line_level();
        assert_eq!(irq, Some(alarm), "IRQ 18 follows AWD & AWDIE");
    }
}
