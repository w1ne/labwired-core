// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the 64-pin megaAVR profiles into the
//! ATmega1281 / ATmega2561 10-bit ADC. The same two decks already proven on the
//! ATmega328P (`cosim_avr_atmega328p_analog`), the STM32, WCH, nRF52 and
//! micro:bit boards run here unchanged: a Beta-parameter NTC divider and a
//! Shockley diode load line name only `board.analog.<pad>_volts`, and each
//! descriptor's Port F `adc` pin function selects the converter channel.
//!
//! Two things are new on these parts. The pad-to-channel mapping lives in the
//! `pins:` table (`{ type: adc, peripheral: adc, channel: N }` on PF0..PF7)
//! instead of a separate `analog_pins:` block, so the test resolves the channel
//! through `ChipDescriptor::analog_pin_table` — the same table the router reads.
//! And the converter is the mega core's time-based ADC
//! (`cpu/avr/mega_adc.rs`), so the test polls `ADIF` after starting a
//! conversion instead of assuming the data register is ready when the store
//! retires. The ATmega2561 selects the three-byte-PC mega core, so the same
//! instruction sequence also proves the larger part's data map.
//!
//! Both profiles are the bare 64-pin parts at their 16 MHz maximum with a 5 V
//! VCC rail, which is also the AVcc reference, so a full-scale input reads 1023.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::avr::Avr;
use labwired_core::Machine;
use std::path::Path;

/// `ADCSRA.ADSC` (bit 6) and `ADIF` (bit 4), DS40002211A §24.9.2. `ADIF` is
/// write-one-to-clear, so the same store that starts a conversion also drops
/// the flag the previous one raised.
const ADCSRA_ADSC: u8 = 1 << 6;
const ADCSRA_ADIF: u8 = 1 << 4;

/// Beta-parameter NTC resistance, the model card in the fixture.
fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line at the part's 5 V rail, including the
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
/// `u16` millivolts by rounding, then `cpu/avr/mega_adc.rs` scales by
/// 1024 / AVcc with an integer multiply and divide.
fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 1024 / 5000).min(1023) as u16
}

/// `LDI R16,<ADMUX> ; STS ADMUX,R16 ; LDI R17,0xD0 (ADEN|ADSC|ADIF) ;
/// STS ADCSRA,R17 ; RJMP .` — the analogRead start sequence, then the spin a
/// sketch's `analogRead` enters while it waits for the conversion. The `ADIF`
/// bit clears the flag left by the previous conversion on the same store.
fn start_program(admux: u8) -> [u16; 7] {
    [
        0xE000 | (u16::from(admux & 0xF0) << 4) | u16::from(admux & 0x0F), // LDI R16, ADMUX
        0x9300,                                                            // STS (R16), k
        0x007C,                                                            //   ADMUX
        0xED10, // LDI R17, 0xD0 (ADEN|ADSC|ADIF)
        0x9310, // STS (R17), k
        0x007A, //   ADCSRA
        0xCFFF, // RJMP .
    ]
}

/// `LDS R18,ADCL ; LDS R19,ADCH ; RJMP .` — read the result the way firmware
/// does, low byte first so the ADCL read does not lock ADCH out.
const READ_PROGRAM: [u16; 5] = [
    0x9120, // LDS R18, k
    0x0078, //   ADCL
    0x9130, // LDS R19, k
    0x0079, //   ADCH
    0xCFFF, // RJMP .
];

struct Bench {
    machine: Machine<Avr>,
    session: CosimSession,
}

impl Bench {
    /// Load one 64-pin mega profile, overlay the generic decks and run the NTC
    /// and diode circuits once per temperature. `first_io` names the profile's
    /// own first `board_io` entry, so the overlay cannot silently drop it.
    fn new(system: &str, deck: &str, first_io: &str) -> Self {
        let system_path = root(system);
        let mut profile = SystemManifest::from_file(&system_path)
            .unwrap_or_else(|e| panic!("load {system} profile: {e}"));
        assert_eq!(
            profile.board_io.first().map(|io| io.id.as_str()),
            Some(first_io),
            "{system} keeps its own board_io bindings"
        );

        let analog = SystemManifest::from_file(root(deck))
            .unwrap_or_else(|e| panic!("load generic decks {deck}: {e}"));
        assert!(
            analog.external_devices.is_empty(),
            "the generic decks add no external device"
        );
        profile.cosim_models = analog.cosim_models;

        let chip_path = system_path.parent().unwrap().join(&profile.chip);
        let chip = ChipDescriptor::from_file(&chip_path)
            .unwrap_or_else(|e| panic!("load {system} descriptor: {e}"));
        let pads = chip.analog_pin_table();
        assert_eq!(
            pads.len(),
            8,
            "{system}: Port F is the eight bonded analog inputs"
        );
        for (pad, channel) in [
            ("PF0", 0),
            ("PF1", 1),
            ("PF2", 2),
            ("PF3", 3),
            ("PF4", 4),
            ("PF5", 5),
            ("PF6", 6),
            ("PF7", 7),
        ] {
            let entry = pads
                .get(pad)
                .unwrap_or_else(|| panic!("{system}: {pad} must reach an ADC"));
            assert_eq!(entry.channel, channel, "{system}: {pad} channel");
            assert_eq!(entry.peripheral, "adc", "{system}: {pad} peripheral");
        }

        let bus = SystemBus::from_config(&chip, &profile)
            .unwrap_or_else(|e| panic!("build {system} bus: {e}"));
        let mut cpu = Avr::for_chip_name(&chip.name);
        cpu.load_words(0, &start_program(0x40));
        let machine = Machine::new(cpu, bus);
        let session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
            .unwrap_or_else(|e| panic!("build {system} co-simulation: {e}"))
            .unwrap_or_else(|| panic!("{system} deck carries both analog devices"));
        assert_eq!(
            session.binding_errors(),
            &[] as &[RoutingError],
            "{system} generic netlists must resolve through PF0/PF1 to ADC0/ADC1"
        );
        Self { machine, session }
    }

    /// One firmware conversion of `channel`: start it, poll `ADIF` the way a
    /// sketch's `analogRead` busy-wait does, then read ADCL/ADCH.
    fn convert(&mut self, channel: u8) -> (u16, u8) {
        self.machine
            .cpu
            .load_words(0, &start_program(0x40 | channel));
        self.machine.cpu.pc = 0;
        // The previous conversion left ADIF set, so the wait is two-phase: the
        // start store drops ADIF and raises ADSC, then the conversion clears
        // ADSC and raises ADIF. Polling ADIF alone would read the last result.
        let mut started = false;
        let mut completed = false;
        for _ in 0..500 {
            self.machine.step().expect("execute one AVR instruction");
            let adcsra = self.machine.cpu.adcsra;
            started |= adcsra & ADCSRA_ADSC != 0;
            if started && adcsra & ADCSRA_ADIF != 0 {
                completed = true;
                break;
            }
        }
        assert!(
            completed,
            "ADC{channel}: the conversion must raise ADIF within 500 instructions"
        );
        assert_eq!(
            self.machine.cpu.adcsra & ADCSRA_ADSC,
            0,
            "ADC{channel}: ADSC clears when the conversion ends"
        );

        self.machine.cpu.load_words(0, &READ_PROGRAM);
        self.machine.cpu.pc = 0;
        for _ in 0..2 {
            self.machine.step().expect("execute one AVR instruction");
        }
        let cpu = &self.machine.cpu;
        let code = u16::from(cpu.r[18]) | (u16::from(cpu.r[19]) << 8);
        (code, cpu.adcsra)
    }
}

#[test]
fn mega64_ntc_and_diode_netlists_reach_pf0_pf1_adc_end_to_end() {
    let expected_diode_v = diode_load_line_voltage();
    let mut covered = Vec::new();

    for (system, deck) in [
        (
            "configs/systems/atmega1281.yaml",
            "examples/atmega1281-io/analog-ntc-diode-system.yaml",
        ),
        (
            "configs/systems/atmega2561.yaml",
            "examples/atmega2561-io/analog-ntc-diode-system.yaml",
        ),
    ] {
        let mut bench = Bench::new(system, deck, "status_led");
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

            let ntc_v = signal_voltage(&bench.session, "board.analog.pf0_volts");
            let expected_ntc_v = 5.0 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
            assert!(
                (ntc_v - expected_ntc_v).abs() < 1e-9,
                "{system}: at {temperature_c} C the Beta divider should be \
                 {expected_ntc_v} V, got {ntc_v} V"
            );
            let diode_v = signal_voltage(&bench.session, "board.analog.pf1_volts");
            assert!(
                (diode_v - expected_diode_v).abs() < 2e-5,
                "{system}: the Shockley diode load line should be {expected_diode_v} V, \
                 got {diode_v} V"
            );

            let (ntc_code, status) = bench.convert(0);
            assert_eq!(
                ntc_code,
                expected_adc_count(ntc_v),
                "{system}: firmware ADCL/ADCH on ADC0 must report the solved NTC voltage"
            );
            assert_ne!(
                status & ADCSRA_ADIF,
                0,
                "{system}: ADIF stays readable after the result is latched"
            );
            let (diode_code, _) = bench.convert(1);
            assert_eq!(
                diode_code,
                expected_adc_count(diode_v),
                "{system}: firmware ADCL/ADCH on ADC1 must report the solved diode voltage"
            );

            // ADC2 has no deck. The `avr_adc` model's documented undriven level
            // is mid-rail, 2500 mV against the 5 V AVcc -> 512, so a nonzero
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
            "configs/systems/atmega1281.yaml",
            "configs/systems/atmega2561.yaml",
        ],
        "both 64-pin mega profiles run the generic decks"
    );
}
