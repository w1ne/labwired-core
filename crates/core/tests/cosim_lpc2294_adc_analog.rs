// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the LPC2294's A/D
//! converter (`lpc2000_adc`) through the onboarding descriptor's
//! `analog_pins` map (Philips LPC2119/2129/2194/2292/2294 User Manual
//! 2004-05-03, 144-pin pin description: Ain0..3 = P0.27..P0.30, Ain4/5 =
//! P2.30/P2.31, Ain6/7 = P3.29/P3.28): the NTC on P2.31 (Ain5) and the diode
//! on P3.29 (Ain6), read the way LPC21xx/22xx firmware does:
//! - software read: `ADCR = PDN | CLKDIV`, set the `SEL` bit,
//!   `ADCR |= START` (001), poll `ADDR.DONE`, `ADCR ^= START`, result
//!   `(ADDR >> 6) & 0x3FF`, the fraction of V3A (3.3 V);
//! - burst: `ADCR = PDN | BURST | SEL`, every conversion lands in `ADDR`
//!   with its channel in `CHN`, lowest `SEL` bit first.
//!
//! Also: the netlists moved to P0.27 (Ain0) and P3.28 (Ain7) convert there
//! while the old pads go back to 0, and the A/D request (VIC channel 18)
//! follows `DONE`.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC: u64 = 0xE003_4000;
const ADCR: u64 = ADC;
const ADDR: u64 = ADC + 0x04;
const PDN: u32 = 1 << 21;
const BURST: u32 = 1 << 16;
const START_NOW: u32 = 1 << 24;
const DONE: u32 = 1 << 31;
const OVERUN: u32 = 1 << 30;
/// CLKDIV 4: PCLK / 5, as the LPC2129 driver programs it.
const CLKDIV: u32 = 4 << 8;

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

/// V/V3A as a 10-bit binary fraction of 3.3 V, truncated, clamped.
fn code(volts: f64) -> u32 {
    (millivolts(volts) * 1024 / 3300).min(1023)
}

/// The onboarding LPC2294 with the generic NTC and diode netlists routed to
/// the two pads.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/onboarding/lpc2294.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load LPC2294");
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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load LPC2294 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build LPC2294 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to Ain pads through analog_pins"
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

fn adc_irq(bus: &SystemBus) -> bool {
    let adc = bus
        .peripherals
        .iter()
        .find(|p| p.name == "adc")
        .expect("adc on the bus");
    adc.dev
        .irq_line_level()
        .expect("lpc2000_adc reports its request")
}

/// LPC2129 `adc_driver.c` `adc_init` + `adc_read`.
fn adc_read(bus: &mut SystemBus, channel: u32) -> u32 {
    bus.write_u32(ADCR, PDN | CLKDIV).expect("adc_init");
    let cr = bus.read_u32(ADCR).unwrap();
    bus.write_u32(ADCR, cr | (1 << channel)).expect("SEL");
    let cr = bus.read_u32(ADCR).unwrap();
    bus.write_u32(ADCR, cr | START_NOW).expect("START now");
    assert!(adc_irq(bus), "Ain{channel}: DONE requests VIC channel 18");
    let polled = bus.read_u32(ADDR).unwrap();
    assert_ne!(polled & DONE, 0, "Ain{channel}: DONE");
    assert_eq!((polled >> 24) & 7, channel, "ADDR.CHN");
    assert!(!adc_irq(bus), "reading ADDR drops the request");
    let cr = bus.read_u32(ADCR).unwrap();
    bus.write_u32(ADCR, cr ^ START_NOW).expect("ADCR ^= START");
    let cr = bus.read_u32(ADCR).unwrap();
    bus.write_u32(ADCR, cr ^ (1 << channel)).expect("deselect");
    (bus.read_u32(ADDR).unwrap() >> 6) & 0x3FF
}

#[test]
fn lpc2294_p2_31_and_p3_29_convert_ntc_and_diode_on_ain5_and_ain6() {
    let (mut bus, mut session) = board("P2.31", "P3.29");
    assert_eq!(bus.read_u32(ADCR).unwrap(), 0x01, "ADCR reset");
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.P2.31_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.P3.29_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc_code = adc_read(&mut bus, 5);
        assert_eq!(ntc_code, code(ntc_v), "Ain5 at {temperature_c} °C");
        assert_eq!(adc_read(&mut bus, 6), code(diode_v), "Ain6");
        for idle in [0, 4, 7] {
            assert_eq!(adc_read(&mut bus, idle), 0, "Ain{idle} is not driven");
        }
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    // Firmware scales the fraction back: code * V3A / 1024.
    let mv = ntc_codes[1] * 3300 / 1024;
    assert!(
        mv.abs_diff(millivolts(ntc_volts(25.0))) <= 4,
        "code * V3A / 1024 gives {mv} mV"
    );
}

#[test]
fn lpc2294_burst_scans_ain5_and_ain6_lowest_first() {
    let (mut bus, mut session) = board("P2.31", "P3.29");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let ntc = code(signal_voltage(&session, "board.analog.P2.31_volts"));
    let diode = code(signal_voltage(&session, "board.analog.P3.29_volts"));
    bus.write_u32(ADCR, PDN | CLKDIV | BURST | (1 << 5) | (1 << 6))
        .expect("burst over Ain5, Ain6");
    let mut seen = Vec::new();
    for _ in 0..4 {
        bus.tick_peripherals();
        let addr = bus.read_u32(ADDR).unwrap();
        assert_eq!(addr & (DONE | OVERUN), DONE, "one fresh result per read");
        seen.push(((addr >> 24) & 7, (addr >> 6) & 0x3FF));
    }
    assert_eq!(seen, [(5, ntc), (6, diode), (5, ntc), (6, diode)]);
    // An unread result overwritten by the next one sets OVERUN.
    bus.tick_peripherals();
    bus.tick_peripherals();
    let addr = bus.read_u32(ADDR).unwrap();
    assert_eq!(addr & (DONE | OVERUN), DONE | OVERUN);
    bus.write_u32(ADCR, PDN | CLKDIV | (1 << 5))
        .expect("BURST off");
    bus.tick_peripherals();
    assert_eq!(bus.read_u32(ADDR).unwrap() & DONE, 0, "the scan stopped");
}

#[test]
fn lpc2294_p0_27_and_p3_28_convert_after_the_netlists_move() {
    let (mut bus, mut session) = board("P0.27", "P3.28");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_v = signal_voltage(&session, "board.analog.P0.27_volts");
    let diode_v = signal_voltage(&session, "board.analog.P3.28_volts");
    assert_eq!(adc_read(&mut bus, 0), code(ntc_v), "Ain0");
    assert_eq!(adc_read(&mut bus, 7), code(diode_v), "Ain7");
    assert_eq!(adc_read(&mut bus, 5), 0, "P2.31 is not driven");
    assert_eq!(adc_read(&mut bus, 6), 0, "P3.29 is not driven");
}
