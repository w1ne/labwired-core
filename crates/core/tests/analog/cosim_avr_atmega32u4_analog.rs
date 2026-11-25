// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the ARDUINO LEONARDO ATmega32U4 profile
//! into its 10-bit ADC. The same two decks already proven on the STM32, WCH,
//! nRF52, micro:bit and ATmega328P boards (`cosim_nucleo_f103rb_analog`,
//! `cosim_ch32v003_analog`, `cosim_nrf52840_family_analog`,
//! `cosim_avr_atmega328p_analog`) run here unchanged: a Beta-parameter NTC
//! divider, a Shockley diode load line and a plain resistor divider name only
//! `board.analog.<pad>_volts`, and the ATmega32U4 descriptor's new
//! `analog_pins:` map selects the ADC channel. The divider sits on PD7 (ADC10)
//! so the high, `ADCSRB.MUX5` half of the map is proven by a conversion.
//!
//! This is the second AVR part in the matrix and the first 32U4. Nothing in the
//! deck, solver, router or converter changed: only the chip descriptor gained
//! the pad map, so the test proves the mechanism is per-part data, not a 328P
//! special case. The conversion runs through the interpreter's own
//! ADMUX/ADCSRA/ADCL/ADCH handling (`Avr::adc_convert` in `cpu/avr.rs`, which
//! for this part also decodes ADCSRB.MUX5), the same code path the 328P uses.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::avr::{Avr, AvrModel};
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
/// `u16` millivolts by rounding, then `Avr::adc_convert` scales by 1024/AVcc
/// with an integer divide. Replicate that order so a mismatch is real physics,
/// not rounding.
fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 1024 / 5000).min(1023) as u16
}

/// `LDI R20,<ADCSRB> ; STS ADCSRB,R20 ; LDI R16,<ADMUX> ; STS ADMUX,R16 ;
/// LDI R17,0xC0 ; STS ADCSRA,R17 ; LDS R18,ADCL ; LDS R19,ADCH ; RJMP .` —
/// the Arduino `analogRead` sequence, which writes ADCSRB.MUX5 on every call
/// so a previous high-channel read cannot leak into the next one. The result
/// is already in the register pair firmware would use.
fn convert_and_read_program(channel: u8) -> [u16; 14] {
    // DS7766 Table 24-4: ADCSRB.MUX5 (bit 5) selects ADC8..13, and ADMUX
    // MUX4:0 then holds the channel minus 8.
    let (adcsrb, admux): (u16, u16) = if channel >= 8 {
        assert!(channel <= 13, "ATmega32U4 has no ADC{channel}");
        (0x20, 0x40 | u16::from(channel - 8))
    } else {
        (0x00, 0x40 | u16::from(channel))
    };
    [
        0xE000 | ((adcsrb & 0xF0) << 4) | (4 << 4) | (adcsrb & 0x0F), // LDI R20, ADCSRB
        0x9340,                                                       // STS (R20), k
        0x007B,                                                       //   ADCSRB
        0xE000 | ((admux & 0xF0) << 4) | (admux & 0x0F),              // LDI R16, ADMUX
        0x9300,                                                       // STS (R16), k
        0x007C,                                                       //   ADMUX
        0xEC10,                                                       // LDI R17, 0xC0 (ADEN|ADSC)
        0x9310,                                                       // STS (R17), k
        0x007A,                                                       //   ADCSRA
        0x9120,                                                       // LDS R18, k
        0x0078,                                                       //   ADCL
        0x9130,                                                       // LDS R19, k
        0x0079,                                                       //   ADCH
        0xCFFF,                                                       // RJMP .
    ]
}

struct Bench {
    machine: Machine<Avr>,
    session: CosimSession,
}

impl Bench {
    /// Load the Leonardo profile, overlay the generic decks and build the
    /// co-simulation. The profile's own `board_io` entry must survive.
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
        assert!(
            profile.board_io.iter().any(|io| {
                io.id == "status_led" && io.peripheral == "portc" && io.pin == 7 && io.active_high
            }),
            "the mixed-signal run keeps the Leonardo's D13 = PC7 status LED"
        );

        let analog = SystemManifest::from_file(root(
            "examples/arduino-leonardo/analog-ntc-diode-system.yaml",
        ))
        .expect("load generic NTC divider and diode netlists");
        profile.cosim_models = analog.cosim_models;

        let chip_path = system_path.parent().unwrap().join(&profile.chip);
        let chip = ChipDescriptor::from_file(&chip_path).expect("load ATmega32U4 descriptor");
        assert_eq!(chip.analog_pins["PF7"].channel, 7);
        assert_eq!(chip.analog_pins["PF6"].channel, 6);
        assert_eq!(chip.analog_pins["PD7"].channel, 10);
        assert_eq!(chip.analog_pins["PB6"].channel, 13);
        assert_eq!(chip.analog_pins["PF7"].peripheral, "adc");

        let bus = SystemBus::from_config(&chip, &profile).expect("build AVR bus");
        // The 32U4 keeps its own data map, vector table and flash geometry, so
        // the model comes from the descriptor name, not the 328P default.
        let mut cpu = Avr::for_chip_name(&chip.name);
        assert_eq!(cpu.model, AvrModel::Atmega32u4);
        cpu.load_words(0, &convert_and_read_program(7));
        let machine = Machine::new(cpu, bus);
        let session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
            .expect("build analog co-simulation")
            .expect("fixture carries both analog devices");
        assert_eq!(
            session.binding_errors(),
            &[] as &[RoutingError],
            "the generic netlists must resolve through PF7/PF6/PD7 to ADC7/ADC6/ADC10"
        );
        Self { machine, session }
    }

    /// One firmware conversion of `channel`: load the `analogRead` sequence
    /// for that channel and execute it from reset.
    fn convert(&mut self, channel: u8) -> (u16, u8) {
        self.machine
            .cpu
            .load_words(0, &convert_and_read_program(channel));
        self.machine.cpu.pc = 0;
        for _ in 0..8 {
            self.machine.step().expect("execute one AVR instruction");
        }
        assert_eq!(
            self.machine.cpu.pc, 0x1A,
            "the analogRead program ran to its RJMP"
        );
        let cpu = &self.machine.cpu;
        let code = u16::from(cpu.r[18]) | (u16::from(cpu.r[19]) << 8);
        (code, cpu.adcsra)
    }
}

#[test]
fn atmega32u4_netlists_reach_pf7_pf6_pd7_adc_end_to_end() {
    let system = "configs/systems/arduino-leonardo.yaml";
    let mut bench = Bench::new(system, "status_led");
    let expected_diode_v = diode_load_line_voltage();
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
            .expect("solve and route all three analog netlists");
        assert_eq!(steps.len(), 3, "all three netlists cross the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        // PF7 is Arduino A0, PF6 is A1 and PD7 is A7 on the Leonardo pinout.
        let ntc_v = signal_voltage(&bench.session, "board.analog.pf7_volts");
        let expected_ntc_v = 5.0 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&bench.session, "board.analog.pf6_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley diode load line should be {expected_diode_v} V, got {diode_v} V"
        );
        let divider_v = signal_voltage(&bench.session, "board.analog.pd7_volts");
        let expected_divider_v = 5.0 * 4_700.0 / (10_000.0 + 4_700.0);
        assert!(
            (divider_v - expected_divider_v).abs() < 1e-9,
            "the PD7 divider should be {expected_divider_v} V, got {divider_v} V"
        );

        let (ntc_code, ntc_status) = bench.convert(7);
        assert_eq!(
            ntc_code,
            expected_adc_count(ntc_v),
            "firmware ADCL/ADCH on ADC7 must report the solved NTC voltage"
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

        let (diode_code, _) = bench.convert(6);
        assert_eq!(
            diode_code,
            expected_adc_count(diode_v),
            "firmware ADCL/ADCH on ADC6 must report the solved diode voltage"
        );

        // ADC10 sits above the low mux window: the same register read finds it
        // only when ADCSRB.MUX5 is set, which is the 32U4's bit 5. PD7 -> ADC10
        // is one of the assignments the first draft of the map got wrong.
        let (divider_code, _) = bench.convert(10);
        assert_eq!(
            divider_code,
            expected_adc_count(divider_v),
            "firmware ADCL/ADCH on ADC10 must report the solved PD7 divider voltage"
        );

        // ADC5 has no deck. The `avr_adc` model's documented undriven level is
        // mid-rail, 2500 mV against 5 V AVcc -> 512, so a nonzero neighbour
        // code proves the routed inputs did not bleed sideways.
        let (floating, _) = bench.convert(5);
        assert_eq!(
            floating, 512,
            "an unrouted ADC5 must keep the model's mid-rail default"
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
        samples[1].2, 512,
        "the 25 C NTC divider is exactly half scale on 5 V AVcc"
    );
}
