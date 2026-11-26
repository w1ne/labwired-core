// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the ARDUINO UNO R4 MINIMA (R7FA4M1AB)
//! profile into its 14-bit ADC14. The same decks already proven on the STM32,
//! WCH, nRF52, micro:bit and AVR boards run here unchanged: a Beta-parameter
//! NTC divider, a Shockley diode load line and a plain resistor divider name
//! only `board.analog.<pad>_volts`, and the RA4M1 descriptor's new
//! `analog_pins:` map selects the ADC channel. The divider sits on P101 (A4),
//! which is channel 21, so the high, ADANSA1 half of the map is proven by a
//! conversion.
//!
//! This is the first Renesas RA part in the matrix. Nothing in the deck,
//! solver, router or converter changed: only the chip descriptor gained the
//! pad map, so the test proves the mechanism is per-part data, not an STM32 or
//! AVR special case. The conversion runs through `ra_adc12`, whose channel
//! numbers are the ADANSA0/ADANSA1 bit positions ArduinoCore-renesas' FSP
//! writes (`variants/MINIMA/pinmux.inc` `PIN_ANALOG|CHANNEL_n`).
//!
//! The BME280 profile is the same board with a sensor on IIC1; it runs the NTC
//! and diode models only, because A4/A5 (P101/P100) are that bus.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::Path;

/// ADC0 window, FSP `R_ADC0_BASE` on the RA4M1.
const ADC0_BASE: u64 = 0x4005_C000;
const ADCSR: u64 = ADC0_BASE;
const ADANSA0: u64 = ADC0_BASE + 0x04;
const ADANSA1: u64 = ADC0_BASE + 0x06;
const ADCER: u64 = ADC0_BASE + 0x0E;
const ADDR0: u64 = ADC0_BASE + 0x20;

/// `ADCSR.ADST`.
const ADST: u16 = 1 << 15;
/// `ADCER.ADPRC` = 3 on an ADC14 unit is 14-bit resolution.
const ADPRC_14BIT: u16 = 3 << 1;

/// Beta-parameter NTC resistance, the model card in the fixture.
fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line at the board's 5 V rail, including the
/// diode's series resistance.
fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 5.0;
    const LIMIT_OHMS: f64 = 1_000.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;
    let current_at_junction_voltage =
        |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let current = current_at_junction_voltage(junction_v);
        (SUPPLY_V - junction_v - current * DIODE_RS_OHMS) / LIMIT_OHMS - current
    };
    let (mut low, mut high) = (0.0, 1.2);
    for _ in 0..100 {
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    let junction_v = (low + high) / 2.0;
    junction_v + current_at_junction_voltage(junction_v) * DIODE_RS_OHMS
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// Wrap the board bus in the production `Machine`, parked on a branch-to-self
/// guest, so a conversion completes through the same boundary firmware uses.
fn board_machine(mut bus: SystemBus) -> Machine<CortexM> {
    let spin_pc = 0x0000_5000;
    bus.write_u16(spin_pc, 0xe7fe).expect("write b ."); // b .
    let (mut cpu, _) = configure_cortex_m(&mut bus);
    cpu.pc = spin_pc as u32;
    cpu.sp = 0x2000_5000;
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine
}

/// One software-started single scan of `channel`, the way FSP's
/// `R_ADC_ScanStart` does it: select the channel in ADANSA0 (0-15) or ADANSA1
/// (16-28), write ADST, then read ADDRn once the scan clears it.
fn ra_adc_convert(machine: &mut Machine<CortexM>, channel: u8) -> u16 {
    let bus = &mut machine.bus;
    // ArduinoCore-renesas always converts at 14 bits (`analogReadResolution`
    // shifts down afterwards), so ADPRC = 3 is what firmware sets.
    bus.write_u16(ADCER, ADPRC_14BIT).expect("select 14 bits");
    let (select_low, select_high) = if channel < 16 {
        (1u16 << channel, 0)
    } else {
        (0, 1u16 << (channel - 16))
    };
    bus.write_u16(ADANSA0, select_low).expect("ADANSA0");
    bus.write_u16(ADANSA1, select_high).expect("ADANSA1");
    bus.write_u16(ADCSR, ADST).expect("software-start the scan");

    // One channel at the model's nominal 1 us at 48 MHz is 48 cycles; the loop
    // allows for the scan never finishing rather than hanging the suite.
    for _ in 0..400 {
        if machine.bus.read_u16(ADCSR).expect("read ADCSR") & ADST == 0 {
            break;
        }
        machine.run(Some(1)).expect("advance the guest one cycle");
    }
    assert_eq!(
        machine.bus.read_u16(ADCSR).expect("read ADCSR") & ADST,
        0,
        "a single scan clears ADST when it finishes"
    );
    machine
        .bus
        .read_u16(ADDR0 + 2 * u64::from(channel))
        .expect("read ADDRn")
}

/// The conversion the model reports: the routed pad voltage in whole
/// millivolts, scaled to 14 bits against the board's 5 V AVCC0.
fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round();
    (millivolts * 16_383.0 / 5_000.0)
        .floor()
        .clamp(0.0, 16_383.0) as u16
}

/// Load a UNO R4 profile, overlay the decks' `cosim_models` and build the bus
/// and co-simulation. `keep` selects which deck models the profile runs.
fn bench(system: &str, keep: &[&str]) -> (Machine<CortexM>, CosimSession) {
    let system_path = root(system);
    let mut profile = SystemManifest::from_file(&system_path).expect("load UNO R4 profile");
    assert!(
        profile
            .board_io
            .iter()
            .any(|io| io.id == "led_d13" && io.peripheral == "port1" && io.pin == 11),
        "{system} keeps its D13 LED binding"
    );

    let deck = SystemManifest::from_file(root(
        "examples/arduino-uno-r4-minima/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    profile.cosim_models = deck
        .cosim_models
        .into_iter()
        .filter(|model| keep.contains(&model.id.as_str()))
        .collect();

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load RA4M1 descriptor");
    assert_eq!(chip.analog_pins["P014"].channel, 9);
    assert_eq!(chip.analog_pins["P000"].channel, 0);
    assert_eq!(chip.analog_pins["P101"].channel, 21);
    assert_eq!(chip.analog_pins["P100"].channel, 22);
    assert_eq!(chip.analog_pins["P014"].peripheral, "adc0");

    let bus = SystemBus::from_config(&chip, &profile).expect("build RA4M1 bus");
    let machine = board_machine(bus);
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build analog co-simulation")
        .expect("fixture carries the selected analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through P014/P000/P101 to ADC0 channels 9/0/21"
    );
    (machine, session)
}

#[test]
fn uno_r4_minima_ntc_and_diode_netlists_reach_a0_a1_a4_adc_end_to_end() {
    let (mut machine, mut session) = bench(
        "configs/systems/arduino-uno-r4-minima.yaml",
        &["ntc_divider", "diode_load_line", "p101_divider"],
    );
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut samples = Vec::new();

    for temperature_c in [-20.0_f64, 25.0, 85.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut machine.bus)
            .expect("solve and route all three analog netlists");
        assert_eq!(steps.len(), 3, "all three netlists cross the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.p014_volts");
        let expected_ntc_v = 5.0 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} C the Beta divider on A0 should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.p000_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley diode load line on A1 should be {expected_diode_v} V, got {diode_v} V"
        );
        let divider_v = signal_voltage(&session, "board.analog.p101_volts");
        let expected_divider_v = 5.0 * 4_700.0 / (10_000.0 + 4_700.0);
        assert!(
            (divider_v - expected_divider_v).abs() < 1e-9,
            "the A4 divider should be {expected_divider_v} V, got {divider_v} V"
        );

        let (ntc_code, diode_code, divider_code) = (
            ra_adc_convert(&mut machine, 9),
            ra_adc_convert(&mut machine, 0),
            ra_adc_convert(&mut machine, 21),
        );
        assert_eq!(
            ntc_code,
            expected_adc_count(ntc_v),
            "firmware-visible ADDR9 (A0) must report the solved NTC voltage"
        );
        assert_eq!(
            diode_code,
            expected_adc_count(diode_v),
            "firmware-visible ADDR0 (A1) must report the solved diode voltage"
        );
        assert_eq!(
            divider_code,
            expected_adc_count(divider_v),
            "firmware-visible ADDR21 (A4) must report the solved divider voltage"
        );

        // Channel 22 (A5, P100) has no deck. `ra_adc12`'s documented undriven
        // level is 0 mV, so a nonzero neighbour code proves the routed inputs
        // did not bleed sideways, and a zero one proves the selector really
        // moved between ADANSA0 and ADANSA1.
        assert_eq!(
            ra_adc_convert(&mut machine, 22),
            0,
            "an unrouted ADANSA1 channel must keep the model's 0 mV default"
        );

        samples.push((temperature_c, ntc_v, ntc_code));
    }

    assert!(
        samples[2].1 > samples[1].1 && samples[1].1 > samples[0].1,
        "the divider voltage rises with NTC temperature: {samples:?}"
    );
    assert!(
        samples[2].2 > samples[1].2 && samples[1].2 > samples[0].2,
        "ADC counts track the solved temperature-dependent voltage: {samples:?}"
    );
    assert_eq!(
        samples[1].2,
        expected_adc_count(2.5),
        "the 25 C NTC divider is exactly half scale on 5 V AVCC0"
    );
}

/// The BME280 profile is the same board with IIC1 populated at A4/A5, so it runs
/// only the NTC and diode decks; a solved A0/A1 conversion must still work with
/// the sensor attached, and the sensor must still be on the bus.
#[test]
fn uno_r4_minima_bme280_profile_reads_a0_a1_with_the_sensor_attached() {
    let system_path = root("configs/systems/arduino-uno-r4-minima-bme280.yaml");
    let profile = SystemManifest::from_file(&system_path).expect("load BME280 profile");
    assert!(
        profile
            .external_devices
            .iter()
            .any(|dev| dev.id == "bme280" && dev.connection == "iic1"),
        "the mixed-signal run keeps the BME280 on IIC1"
    );

    let (mut machine, mut session) = bench(
        "configs/systems/arduino-uno-r4-minima-bme280.yaml",
        &["ntc_divider", "diode_load_line"],
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set generic NTC temperature");
    let cycles = session.cycles_until_boundary(0);
    let (steps, errors) = session
        .advance_to(cycles, &mut machine.bus)
        .expect("solve and route both analog netlists");
    assert_eq!(steps.len(), 2, "both netlists cross the 1 us boundary");
    assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

    let ntc_v = signal_voltage(&session, "board.analog.p014_volts");
    assert_eq!(
        ra_adc_convert(&mut machine, 9),
        expected_adc_count(ntc_v),
        "A0 must convert the solved NTC voltage while the BME280 owns A4/A5"
    );
    let diode_v = signal_voltage(&session, "board.analog.p000_volts");
    assert_eq!(
        ra_adc_convert(&mut machine, 0),
        expected_adc_count(diode_v),
        "A1 must convert the solved diode voltage while the BME280 owns A4/A5"
    );
}
