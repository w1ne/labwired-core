// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices reach the ATmega4809 ADC0 through the `adc` function on
//! the chip's own pin table. The same two decks proven on the ATmega328P
//! (`cosim_avr_atmega328p_analog`) and on the STM32, WCH, nRF52 and micro:bit
//! profiles run here unchanged: a Beta-parameter NTC divider and a Shockley
//! diode load line name only `board.analog.<pad>_volts`.
//!
//! What is new is the route, not the physics. The ATmega4809 descriptor now
//! records `{ type: adc, peripheral: adc0, channel: N }` on the pads the data
//! sheet bonds to ADC0, so the router resolves `board.analog.pd3_volts` to
//! AIN3 without a duplicate `analog_pins:` table. PD0..PD7 are AIN0..AIN7,
//! PE0..PE3 are AIN8..AIN11 and PF2..PF5 are AIN12..AIN15 (DS40002173
//! "I/O Multiplexing"). On the Arduino Nano Every, A0 is PD3/AIN3 and A1 is
//! PD2/AIN2 (ArduinoCore-megaavr `variants/nona4809`).
//!
//! The converter is the bus-side `megaavr_adc`, not the AVR interpreter's
//! ADMUX/ADCSRA path, so the test drives the same register sequence firmware
//! drives: PRESC, ENABLE, VDD reference, MUXPOS, COMMAND.STCONV, then RES.
//! Three unrelated Nano Every profiles run the decks, including the two with
//! an external sensor attached.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

/// ADC0 in the megaAVR data space; the AVR CPU reaches it through the
/// 0x10000 data-space window.
const ADC0_BASE: u64 = 0x0001_0600;
const ADC_CTRLA: u64 = 0x00;
const ADC_CTRLC: u64 = 0x02;
const ADC_MUXPOS: u64 = 0x06;
const ADC_COMMAND: u64 = 0x08;
const ADC_RES: u64 = 0x10;

/// ArduinoCore-megaavr `init()`: PRESC DIV128, ENABLE (10-bit).
const CTRLA_ENABLE: u8 = 0x01;
const CTRLC_ARD_INIT: u8 = 0x06;
/// CTRLC with REFSEL = VDD, the reference `analogReference(VDD)` selects.
const CTRLC_VDD_REF: u8 = 0x16;
const COMMAND_STCONV: u8 = 0x01;

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

/// The router rounds volts to integer millivolts; `megaavr_adc` then converts
/// `floor(mV * 1024 / VDD)` at 10 bits against the 5 V VDD reference.
fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 1024 / 5000).min(1023) as u16
}

struct Bench {
    bus: SystemBus,
    session: CosimSession,
}

impl Bench {
    /// Load one Nano Every profile, replace its co-simulation models with the
    /// generic NTC/diode deck, and start ADC0 as `init()` does.
    fn new(system: &str) -> Self {
        let system_path = root(system);
        let mut profile = SystemManifest::from_file(&system_path).expect("load Nano Every profile");
        let analog = SystemManifest::from_file(root(
            "examples/arduino-nano-every/analog-ntc-diode-system.yaml",
        ))
        .expect("load generic NTC divider and diode netlists");
        profile.cosim_models = analog.cosim_models;

        let chip_path = system_path.parent().unwrap().join(&profile.chip);
        let chip = ChipDescriptor::from_file(&chip_path).expect("load ATmega4809 descriptor");
        let table = chip.analog_pin_table();
        for (pad, channel) in [
            ("PD0", 0),
            ("PD1", 1),
            ("PD2", 2),
            ("PD3", 3),
            ("PD7", 7),
            ("PE0", 8),
            ("PE3", 11),
            ("PF2", 12),
            ("PF5", 15),
        ] {
            assert_eq!(
                table
                    .get(pad)
                    .map(|pin| (pin.peripheral.as_str(), pin.channel)),
                Some(("adc0", channel)),
                "{system}: {pad} must be ADC0/AIN{channel} from the datasheet pin table"
            );
        }
        assert_eq!(
            chip.analog_pin_conflicts(),
            vec![],
            "{system}: the pin table and any analog_pins block must agree"
        );

        let mut bus = SystemBus::from_config(&chip, &profile).expect("build ATmega4809 bus");
        bus.write_u8(ADC0_BASE + ADC_CTRLC, CTRLC_ARD_INIT)
            .expect("set ADC0 prescaler");
        bus.write_u8(ADC0_BASE + ADC_CTRLA, CTRLA_ENABLE)
            .expect("enable ADC0");
        bus.write_u8(ADC0_BASE + ADC_CTRLC, CTRLC_VDD_REF)
            .expect("select the VDD reference");

        let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
            .expect("build analog co-simulation")
            .expect("fixture carries both analog devices");
        assert_eq!(
            session.binding_errors(),
            &[] as &[RoutingError],
            "{system}: the generic netlists must resolve through PD3/PD2 to ADC0/AIN3 and AIN2"
        );
        Self { bus, session }
    }

    /// One 10-bit conversion of `channel` through the firmware-visible
    /// registers: MUXPOS, COMMAND.STCONV, one bus tick, then RES.
    fn convert(&mut self, channel: u8) -> u16 {
        self.bus
            .write_u8(ADC0_BASE + ADC_MUXPOS, channel)
            .expect("select AIN");
        self.bus
            .write_u8(ADC0_BASE + ADC_COMMAND, COMMAND_STCONV)
            .expect("start conversion");
        self.bus.tick_peripherals_fully();
        let low = self.bus.read_u8(ADC0_BASE + ADC_RES).expect("read RES low");
        let high = self
            .bus
            .read_u8(ADC0_BASE + ADC_RES + 1)
            .expect("read RES high");
        u16::from(low) | (u16::from(high) << 8)
    }
}

#[test]
fn atmega4809_ntc_and_diode_netlists_reach_pd3_pd2_adc0_end_to_end() {
    let expected_diode_v = diode_load_line_voltage();
    let mut covered = Vec::new();

    for system in [
        "configs/systems/arduino-nano-every.yaml",
        "configs/systems/arduino-nano-every-bme280.yaml",
        "configs/systems/arduino-nano-every-max31855.yaml",
    ] {
        let mut bench = Bench::new(system);
        let mut cycles = 0;
        let mut samples = Vec::new();

        for temperature_c in [-20.0_f64, 25.0, 85.0] {
            bench
                .session
                .set_signal_number("ui.ntc.temperature_c", temperature_c)
                .expect("set generic NTC temperature");
            cycles += bench.session.cycles_until_boundary(cycles);
            let (steps, errors) = bench
                .session
                .advance_to(cycles, &mut bench.bus)
                .expect("solve and route both analog netlists");
            assert_eq!(steps.len(), 2, "both netlists cross the 1 us boundary");
            assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

            let ntc_v = signal_voltage(&bench.session, "board.analog.pd3_volts");
            let expected_ntc_v = 5.0 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
            assert!(
                (ntc_v - expected_ntc_v).abs() < 1e-9,
                "{system}: at {temperature_c} C the Beta divider should be \
                 {expected_ntc_v} V, got {ntc_v} V"
            );
            let diode_v = signal_voltage(&bench.session, "board.analog.pd2_volts");
            assert!(
                (diode_v - expected_diode_v).abs() < 2e-5,
                "{system}: the Shockley diode load line should be {expected_diode_v} V, \
                 got {diode_v} V"
            );

            let ntc_code = bench.convert(3);
            assert_eq!(
                ntc_code,
                expected_adc_count(ntc_v),
                "{system}: ADC0/AIN3 (PD3, Arduino A0) must report the solved NTC voltage"
            );
            let diode_code = bench.convert(2);
            assert_eq!(
                diode_code,
                expected_adc_count(diode_v),
                "{system}: ADC0/AIN2 (PD2, Arduino A1) must report the solved diode voltage"
            );

            // AIN0 has no deck. The megaAVR model's undriven level is 0, so a
            // zero code proves the routed inputs did not bleed sideways.
            assert_eq!(
                bench.convert(0),
                0,
                "{system}: an unrouted AIN0 must stay at the model's zero default"
            );

            samples.push((temperature_c, ntc_v, ntc_code));
        }

        assert!(
            samples[2].1 > samples[1].1 && samples[1].1 > samples[0].1,
            "{system}: the divider voltage rises with NTC temperature: {samples:?}"
        );
        assert!(
            samples[2].2 > samples[1].2 && samples[1].2 > samples[0].2,
            "{system}: ADC counts track the solved temperature-dependent voltage: {samples:?}"
        );
        assert_eq!(
            samples[1].2, 512,
            "{system}: the 25 C NTC divider is exactly half scale on 5 V VDD"
        );
        covered.push(system);
    }

    assert_eq!(
        covered,
        [
            "configs/systems/arduino-nano-every.yaml",
            "configs/systems/arduino-nano-every-bme280.yaml",
            "configs/systems/arduino-nano-every-max31855.yaml",
        ],
        "all three ATmega4809 profiles run the generic decks"
    );
}
