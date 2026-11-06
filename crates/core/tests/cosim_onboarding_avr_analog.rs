// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices reach the CPU-owned ADC of every imported simavr AVR
//! profile (`configs/systems/onboarding/simavr-*.yaml`) whose register map is
//! one LabWired's AVR cores run.
//!
//! The simavr imports used to carry `peripherals: []`, so no analog device
//! could reach them. The ADC on an AVR is not a bus peripheral: ADMUX/ADCSRA/
//! ADCL/ADCH live on the CPU, and the bus-side `avr_adc` window only holds the
//! pin millivolts. So the bus-register sweep in `cosim_onboarding_analog.rs`
//! cannot convert them, and this test drives the conversion the way firmware
//! does, with STS to ADCSRB/ADMUX/ADCSRA on the CPU core the descriptor
//! selects.
//!
//! The classic ATmega8/16/32 run on their own I/O-space cores (ADMUX 0x27,
//! no ADCSRB); there the test also checks the internal 2.56 V reference and
//! the part's bandgap input. Every result is read back with LDS ADCL/ADCH.
//!
//! For each covered part the generic Beta NTC divider and the unrelated
//! Shockley diode route through the descriptor's `analog_pins`, to the lowest
//! and highest ADC pad. On the 100-pin megas the highest pad is PK7/ADC15,
//! which needs ADCSRB.MUX5. Both solved voltages are checked against
//! independent references, the 10-bit codes against `mV * 1024 / 5000`, and
//! the NTC code must fall monotonically with temperature.
//!
//! [`UNCOVERED`] lists the simavr parts whose register map no LabWired core
//! runs (their ADC or ports sit elsewhere, per avr-libc), so a part cannot
//! silently be given the 328P core.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::avr::{Avr, AvrModel};
use labwired_core::peripherals::avr_adc::AVR_ADC_UNDRIVEN_MV;
use labwired_core::{Cpu, SimulationConfig};
use std::path::{Path, PathBuf};

/// Covered simavr profiles, the core they run on, and the two pads.
const COVERED: &[(&str, AvrModel, &str, &str)] = &[
    ("simavr-atmega1280", AvrModel::Atmega1281, "pf0", "pk7"),
    ("simavr-atmega1281", AvrModel::Atmega1281, "pf0", "pf7"),
    ("simavr-atmega16", AvrModel::Atmega16, "pa0", "pa7"),
    ("simavr-atmega168", AvrModel::Atmega328p, "pc0", "pc5"),
    ("simavr-atmega2560", AvrModel::Atmega2560, "pf0", "pk7"),
    ("simavr-atmega32", AvrModel::Atmega32, "pa0", "pa7"),
    ("simavr-atmega328", AvrModel::Atmega328p, "pc0", "pc5"),
    ("simavr-atmega48", AvrModel::Atmega328p, "pc0", "pc5"),
    ("simavr-atmega8", AvrModel::Atmega8, "pc0", "pc5"),
    ("simavr-atmega88", AvrModel::Atmega328p, "pc0", "pc5"),
];

/// simavr profiles with no analog endpoint, and why (addresses per avr-libc
/// 2.0.0). Covering one has to move it into [`COVERED`].
const UNCOVERED: &[(&str, &str)] = &[
    ("simavr-at90usb162", "no ADC"),
    ("simavr-atmega128", "ADMUX at 0x27, the 328P core's DDRC"),
    (
        "simavr-atmega1284p",
        "ADC on port A at 0x20, not a 328P-core port",
    ),
    ("simavr-atmega128rfa1", "mega map, but no 128RFA1 core"),
    ("simavr-atmega128rfr2", "mega map, but no 128RFR2 core"),
    ("simavr-atmega164", "ADC on port A at 0x20"),
    ("simavr-atmega169p", "LCD-controller part, no 169P core"),
    (
        "simavr-atmega16m1",
        "ADMUX MUX4:0 maps ADC0..10 differently",
    ),
    ("simavr-atmega324", "ADC on port A at 0x20"),
    (
        "simavr-atmega328pb",
        "no avr-libc header to check PE2/PE3 against",
    ),
    ("simavr-atmega32u4", "ADC2/ADC3 do not exist, MUX5 differs"),
    ("simavr-atmega644", "ADC on port A at 0x20"),
    (
        "simavr-atmega64m1",
        "ADMUX MUX4:0 maps ADC0..10 differently",
    ),
    ("simavr-attiny13", "ADMUX at 0x27"),
    ("simavr-attiny1634", "ADMUX at 0x24"),
    ("simavr-attiny2313", "no ADC"),
    ("simavr-attiny2313a", "no ADC"),
    ("simavr-attiny24", "ADMUX at 0x27"),
    ("simavr-attiny25", "ADMUX at 0x27"),
    ("simavr-attiny4313", "no ADC"),
    ("simavr-attiny44", "ADMUX at 0x27"),
    ("simavr-attiny45", "ADMUX at 0x27"),
    ("simavr-attiny84", "ADMUX at 0x27"),
    ("simavr-attiny85", "ADMUX at 0x27"),
];

const AVCC_MV: u32 = 5000;

/// ADCSRB/ADMUX/ADCSRA/ADCL data addresses on `core` (ADCH = ADCL + 1). The
/// classic ATmega8/16/32 keep the ADC in I/O space and have no ADCSRB (MUX5
/// does not exist).
fn adc_registers(core: AvrModel) -> (Option<u16>, u16, u16, u16) {
    if core.is_classic() {
        (None, 0x27, 0x26, 0x24)
    } else {
        (Some(0x7B), 0x7C, 0x7A, 0x78)
    }
}

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line, including the diode's series resistance.
fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LIMIT_OHMS: f64 = 1_000.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
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

/// `LDI Rd, K` for r16..r31.
fn ldi(rd: u16, k: u8) -> u16 {
    0xE000 | (u16::from(k & 0xF0) << 4) | ((rd - 16) << 4) | u16::from(k & 0x0F)
}

/// `STS k, Rr` (two words).
fn sts(addr: u16, rr: u16) -> [u16; 2] {
    [0x9200 | (rr << 4), addr]
}

/// `LDS Rd, k` (two words).
fn lds(rd: u16, addr: u16) -> [u16; 2] {
    [0x9000 | (rd << 4), addr]
}

/// One firmware conversion of ADC `channel` against AVcc.
fn convert(chip: &ChipDescriptor, bus: &mut SystemBus, channel: u8) -> u16 {
    convert_mux(chip, bus, 0x40, channel)
}

/// One firmware conversion: ADCSRB.MUX5 for ADC8..15 (where ADCSRB exists),
/// ADMUX = `refs` | MUX, then ADCSRA = ADEN | ADSC, and the result read back
/// with LDS ADCL/ADCH into r24/r25.
fn convert_mux(chip: &ChipDescriptor, bus: &mut SystemBus, refs: u8, mux: u8) -> u16 {
    let mut cpu = Avr::for_chip_name(&chip.name);
    let (adcsrb, admux, adcsra, adcl) = adc_registers(cpu.model);
    let mut words = Vec::new();
    if let Some(adcsrb) = adcsrb {
        words.push(ldi(16, if mux >= 8 { 1 << 3 } else { 0 }));
        words.extend(sts(adcsrb, 16));
    }
    words.push(ldi(
        16,
        refs | if adcsrb.is_some() { mux & 0x07 } else { mux },
    ));
    words.extend(sts(admux, 16));
    words.push(ldi(16, 0xC0));
    words.extend(sts(adcsra, 16));
    words.extend(lds(24, adcl));
    words.extend(lds(25, adcl + 1));
    let instructions = (words.len() - 4) / 3 * 2 + 2;
    cpu.load_words(0, &words);
    let config = SimulationConfig::default();
    for _ in 0..instructions {
        cpu.step(bus, &[], &config)
            .expect("execute ADC setup and conversion");
    }
    assert_eq!(cpu.adcsra & (1 << 6), 0, "ADSC clears at completion");
    assert_ne!(cpu.adcsra & (1 << 4), 0, "ADIF records the conversion");
    let code = u16::from(cpu.r[25]) << 8 | u16::from(cpu.r[24]);
    assert_eq!(
        code,
        u16::from(cpu.adch) << 8 | u16::from(cpu.adcl),
        "{}: LDS ADCL/ADCH reads the converter's result",
        chip.name
    );
    code
}

fn expected_code(volts: f64) -> u16 {
    let mv = (volts * 1000.0).round() as u32;
    (mv * 1024 / AVCC_MV).min(1023) as u16
}

fn generic_netlists(ntc_pad: &str, diode_pad: &str) -> SystemManifest {
    let mut analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    for model in &mut analog.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", ntc_pad),
            "diode_load_line" => ("v_diode", diode_pad),
            other => panic!("unexpected fixture model {other}"),
        };
        model
            .outputs
            .insert(key.into(), format!("board.analog.{pad}_volts"));
    }
    analog
}

fn simavr_profiles() -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(root("configs/systems/onboarding"))
        .expect("read onboarding systems")
        .map(|entry| entry.expect("onboarding entry").path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "yaml")
                && path
                    .file_stem()
                    .is_some_and(|s| s.to_string_lossy().starts_with("simavr-"))
        })
        .collect();
    paths.sort();
    paths
}

fn analog_channel(chip: &ChipDescriptor, pad: &str) -> u8 {
    chip.analog_pins
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(pad))
        .map(|(_, route)| route.channel)
        .unwrap_or_else(|| panic!("{}: no analog pad {pad}", chip.name))
}

fn assert_avr_converts_generic_analog(
    board: &str,
    chip: &ChipDescriptor,
    base_profile: &SystemManifest,
    core: AvrModel,
    ntc_pad: &str,
    diode_pad: &str,
) {
    assert_eq!(
        Avr::for_chip_name(&chip.name).model,
        core,
        "{board}: descriptor name selects the core whose map it was checked against"
    );
    let ntc_channel = analog_channel(chip, ntc_pad);
    let diode_channel = analog_channel(chip, diode_pad);
    assert_ne!(ntc_channel, diode_channel, "{board}: distinct ADC channels");

    let mut profile = base_profile.clone();
    profile.cosim_models = generic_netlists(ntc_pad, diode_pad).cosim_models;
    let mut bus = SystemBus::from_config(chip, &profile).expect("build simavr bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{board}: {ntc_pad}/{diode_pad} resolve through analog_pins"
    );

    let expected_diode_v = diode_load_line_voltage();
    let ntc_path = format!("board.analog.{ntc_pad}_volts");
    let diode_path = format!("board.analog.{diode_pad}_volts");
    let mut cycles = 0;
    let mut last_ntc_code = u16::MAX;
    for temperature_c in [-10.0_f64, 0.0, 25.0, 50.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both netlists");
        assert_eq!(steps.len(), 2, "{board}: both netlists solve each step");
        assert!(errors.is_empty(), "{board}: route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, &ntc_path);
        let r = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * r / (10_000.0 + r);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "{board}: at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, &diode_path);
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "{board}: Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let ntc_code = convert(chip, &mut bus, ntc_channel);
        assert_eq!(
            ntc_code,
            expected_code(expected_ntc_v),
            "{board}: ADC{ntc_channel} ({ntc_pad}) converts the NTC divider at {temperature_c} °C"
        );
        assert!(
            ntc_code < last_ntc_code,
            "{board}: NTC code falls as temperature rises ({last_ntc_code} -> {ntc_code})"
        );
        last_ntc_code = ntc_code;
        assert_eq!(
            convert(chip, &mut bus, diode_channel),
            expected_code(expected_diode_v),
            "{board}: ADC{diode_channel} ({diode_pad}) converts the diode load line"
        );
    }
    // A channel neither netlist drives keeps the undriven mid-rail level.
    let idle = (0..8)
        .find(|ch| *ch != ntc_channel && *ch != diode_channel)
        .unwrap();
    assert_eq!(
        convert(chip, &mut bus, idle),
        expected_code(f64::from(AVR_ADC_UNDRIVEN_MV) / 1000.0),
        "{board}: idle ADC{idle} is not written by either route"
    );

    if core.is_classic() {
        // REFS = 11 is the classic parts' internal 2.56 V, not the 328P's 1.1 V.
        let mv = (expected_diode_v * 1000.0).round() as u32;
        assert_eq!(
            convert_mux(chip, &mut bus, 0xC0, diode_channel),
            (mv * 1024 / 2560) as u16,
            "{board}: the diode converts against the internal 2.56 V reference"
        );
        // The bandgap input: 1.30 V on MUX 0x0E (ATmega8), 1.22 V on 0x1E (16/32).
        let (code, bandgap_mv) = if core == AvrModel::Atmega8 {
            (0x0E, 1300)
        } else {
            (0x1E, 1220)
        };
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, code),
            (bandgap_mv * 1024 / AVCC_MV) as u16,
            "{board}: MUX {code:#04x} converts the {bandgap_mv} mV bandgap"
        );
    }
}

#[test]
fn every_simavr_profile_on_a_modelled_avr_core_converts_ntc_and_diode_in_firmware() {
    let mut covered = Vec::new();
    let mut uncovered = Vec::new();
    for system_path in simavr_profiles() {
        let board = system_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let profile =
            SystemManifest::from_file(&system_path).unwrap_or_else(|e| panic!("load {board}: {e}"));
        let chip = ChipDescriptor::from_file(system_path.parent().unwrap().join(&profile.chip))
            .unwrap_or_else(|e| panic!("load {board} descriptor: {e}"));
        let has_adc = chip.peripherals.iter().any(|p| p.r#type == "avr_adc");
        match COVERED.iter().find(|(b, ..)| *b == board) {
            Some((_, core, ntc_pad, diode_pad)) => {
                assert!(has_adc, "{board}: covered parts declare avr_adc");
                assert_avr_converts_generic_analog(
                    &board, &chip, &profile, *core, ntc_pad, diode_pad,
                );
                covered.push(board);
            }
            None => {
                assert!(
                    chip.peripherals.is_empty() && chip.analog_pins.is_empty(),
                    "{board}: a part outside COVERED declares no analog endpoint"
                );
                uncovered.push(board);
            }
        }
    }
    let expected_covered: Vec<_> = COVERED.iter().map(|(b, ..)| b.to_string()).collect();
    let expected_uncovered: Vec<_> = UNCOVERED.iter().map(|(b, _)| b.to_string()).collect();
    assert_eq!(
        covered, expected_covered,
        "simavr profiles converted end to end; update docs/mixed_signal_board_coverage.md"
    );
    assert_eq!(
        uncovered, expected_uncovered,
        "simavr profiles with no analog endpoint"
    );
}
