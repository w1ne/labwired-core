// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through AVR profiles into the ATmega328P's 10-bit
//! ADC. The same two decks already proven on the STM32, WCH, nRF52 and
//! micro:bit boards (`cosim_nucleo_f103rb_analog`, `cosim_ch32v003_analog`,
//! `cosim_nrf52840_family_analog`) run here unchanged: a Beta-parameter NTC
//! divider and a Shockley diode load line name only `board.analog.<pad>_volts`,
//! and the ATmega328P descriptor's `analog_pins:` map selects the ADC channel.
//!
//! This is the first mixed-signal coverage on an AVR core and the first at a
//! 5 V AVcc reference. The conversion still runs through the interpreter's own
//! ADMUX/ADCSRA/ADCL/ADCH handling (`cpu/avr.rs`), so the test executes the
//! register sequence firmware executes rather than poking a model setter.
//! Three unrelated profiles share the chip map: the Arduino Uno (DIP-28, A0/A1
//! only), the Arduino Nano (TQFP, A0..A7) and the CONTROLLINO MINI.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::avr::Avr;
use labwired_core::Machine;
use std::path::Path;

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

/// The AVR conversion the interpreter reports: the router turns volts into
/// `u16` millivolts by rounding, then `cpu/avr.rs` scales by 1024/AVcc with an
/// integer divide. Replicate that order so a mismatch is real physics, not
/// rounding.
fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 1024 / 5000).min(1023) as u16
}

/// `LDI R16,<ADMUX> ; STS ADMUX,R16 ; LDI R17,0xC0 ; STS ADCSRA,R17 ;
/// LDS R18,ADCL ; LDS R19,ADCH ; RJMP .` — the analogRead sequence, with the
/// result already in the register pair firmware would use.
fn convert_and_read_program(admux: u8) -> [u16; 11] {
    [
        0xE000 | (u16::from(admux & 0xF0) << 4) | u16::from(admux & 0x0F), // LDI R16, ADMUX
        0x9300,                                                            // STS (R16), k
        0x007C,                                                            //   ADMUX
        0xEC10, // LDI R17, 0xC0 (ADEN|ADSC)
        0x9310, // STS (R17), k
        0x007A, //   ADCSRA
        0x9120, // LDS R18, k
        0x0078, //   ADCL
        0x9130, // LDS R19, k
        0x0079, //   ADCH
        0xCFFF, // RJMP .
    ]
}

struct Bench {
    machine: Machine<Avr>,
    session: CosimSession,
}

impl Bench {
    /// Load one board profile, overlay the generic decks and run the NTC and
    /// diode circuits once per temperature. `first_io` names the profile's
    /// own first `board_io` entry, so the overload cannot silently drop it.
    fn new(system: &str, first_io: &str) -> Self {
        let system_path = root(system);
        let mut profile = SystemManifest::from_file(&system_path).expect("load AVR profile");
        assert!(
            profile.external_devices.is_empty(),
            "the generic decks are the only stimulus"
        );
        assert_eq!(
            profile.board_io.first().map(|io| io.id.as_str()),
            Some(first_io),
            "{system} keeps its own board_io bindings"
        );

        let analog =
            SystemManifest::from_file(root("examples/arduino-uno-io/analog-ntc-diode-system.yaml"))
                .expect("load generic NTC divider and diode netlists");
        profile.cosim_models = analog.cosim_models;

        let chip_path = system_path.parent().unwrap().join(&profile.chip);
        let chip = ChipDescriptor::from_file(&chip_path).expect("load ATmega328P descriptor");
        assert_eq!(chip.analog_pins["PC0"].channel, 0);
        assert_eq!(chip.analog_pins["PC1"].channel, 1);
        assert_eq!(chip.analog_pins["ADC7"].channel, 7);
        assert_eq!(chip.analog_pins["PC0"].peripheral, "adc");

        let bus = SystemBus::from_config(&chip, &profile).expect("build AVR bus");
        let mut cpu = Avr::new();
        cpu.load_words(0, &convert_and_read_program(0x40));
        let machine = Machine::new(cpu, bus);
        let session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
            .expect("build analog co-simulation")
            .expect("fixture carries both analog devices");
        assert_eq!(
            session.binding_errors(),
            &[] as &[RoutingError],
            "the generic netlists must resolve through PC0/PC1 to ADC0/ADC1"
        );
        Self { machine, session }
    }

    /// One firmware conversion of `channel`: rewrite the program's ADMUX
    /// immediate and execute the register sequence from reset.
    fn convert(&mut self, channel: u8) -> (u16, u8) {
        self.machine
            .cpu
            .load_words(0, &convert_and_read_program(0x40 | channel));
        self.machine.cpu.pc = 0;
        for _ in 0..6 {
            self.machine.step().expect("execute one AVR instruction");
        }
        let cpu = &self.machine.cpu;
        let code = u16::from(cpu.r[18]) | (u16::from(cpu.r[19]) << 8);
        (code, cpu.adcsra)
    }
}

#[test]
fn atmega328p_ntc_and_diode_netlists_reach_pc0_pc1_adc_end_to_end() {
    let expected_diode_v = diode_load_line_voltage();
    let mut covered = Vec::new();

    for (system, first_io) in [
        ("configs/systems/arduino-uno.yaml", "status_led"),
        ("configs/systems/arduino-nano.yaml", "status_led"),
        ("configs/systems/controllino-mini.yaml", "in0"),
    ] {
        let mut bench = Bench::new(system, first_io);
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
                .advance_to(cycles, &mut bench.machine.bus)
                .expect("solve and route both analog netlists");
            assert_eq!(steps.len(), 2, "both netlists cross the 1 us boundary");
            assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

            let ntc_v = signal_voltage(&bench.session, "board.analog.pc0_volts");
            let expected_ntc_v = 5.0 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
            assert!(
                (ntc_v - expected_ntc_v).abs() < 1e-9,
                "{system}: at {temperature_c} C the Beta divider should be \
                 {expected_ntc_v} V, got {ntc_v} V"
            );
            let diode_v = signal_voltage(&bench.session, "board.analog.pc1_volts");
            assert!(
                (diode_v - expected_diode_v).abs() < 2e-5,
                "{system}: the Shockley diode load line should be {expected_diode_v} V, \
                 got {diode_v} V"
            );

            let (ntc_code, ntc_status) = bench.convert(0);
            assert_eq!(
                ntc_code,
                expected_adc_count(ntc_v),
                "{system}: firmware ADCL/ADCH on ADC0 must report the solved NTC voltage"
            );
            assert_ne!(
                ntc_status & (1 << 4),
                0,
                "ADIF sets when the conversion ends"
            );
            assert_eq!(
                ntc_status & (1 << 6),
                0,
                "ADSC clears when the conversion ends"
            );

            let (diode_code, _) = bench.convert(1);
            assert_eq!(
                diode_code,
                expected_adc_count(diode_v),
                "{system}: firmware ADCL/ADCH on ADC1 must report the solved diode voltage"
            );

            // ADC2 has no deck. The `avr_adc` model's documented undriven level
            // is mid-rail, 2500 mV against 5 V AVcc -> 512, so a nonzero
            // neighbour code proves the routed inputs did not bleed sideways.
            let (floating, _) = bench.convert(2);
            assert_eq!(
                floating, 512,
                "{system}: an unrouted ADC2 must keep the model's mid-rail default"
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
            "{system}: the 25 C NTC divider is exactly half scale on 5 V AVcc"
        );
        covered.push(system);
    }

    assert_eq!(
        covered,
        [
            "configs/systems/arduino-uno.yaml",
            "configs/systems/arduino-nano.yaml",
            "configs/systems/controllino-mini.yaml",
        ],
        "all three ATmega328P profiles run the generic decks"
    );
}
