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
//! the part's bandgap input. The ATmega164/324/644/1284P run on the port-A
//! cores (ADC0..7 on PA0..PA7); there it checks the internal 1.1 V and 2.56 V
//! references and the MUX 0x1E bandgap. The ATmega128RFA1/128RFR2 run on the
//! RF core, whose ADC is powered from the internal 1.8 V AVDD: there the codes
//! are against 1.8 V (the NTC divider clips at 0x3FF when it sits above it),
//! and it checks the internal 1.5 V and 1.6 V references, the 1.2 V bandgap
//! on MUX 0x1E, AVSS on 0x1F, and that firmware polling ADCSRB.AVDDOK/REFOK
//! sees them set once ADEN is. The ATtiny25/45/85 run on the I/O-space ATtiny
//! x5 cores (ADC0..3 on PB5, PB2, PB4, PB3); there it checks REFS2 (ADMUX
//! bit 4) selecting the internal 1.1 V and 2.56 V references and the MUX
//! 0x0C bandgap. The ATtiny24/44/84 run on the I/O-space ATtiny x4 cores
//! (ADC0..7 on PA0..PA7, MUX5:0 in ADMUX); there it checks REFS = 10 selecting
//! the internal 1.1 V, the MUX 0x21 bandgap, MUX 0x20 AGND, and that firmware
//! setting ADLAR in ADCSRB bit 4 (ADMUX bit 5 is MUX5 there) gets a
//! left-adjusted result. The ATmega128 runs on its own I/O-space core (ADC0..7
//! on PF0..PF7, whose PINF and DDRF/PORTF are not adjacent); there it checks
//! the internal 2.56 V reference and the MUX 0x1E 1.23 V bandgap. The ATtiny13
//! runs on its own I/O-space core (ADC0..3 on PB5, PB2, PB4, PB3, MUX1:0
//! only); there it checks REFS0 (ADMUX bit 6, AVcc on the megas) selecting
//! the internal 1.1 V, and that the x5 parts' REFS1 and bandgap code do not
//! exist. The ATmega32U4 runs on its own core (ADC0/1, ADC4..7 on port F and
//! ADC8..13 on PD4, PD6, PD7, PB4..PB6, no ADC2/ADC3); there the highest pad
//! is PB6/ADC13, selected with MUX5 in ADCSRB bit 5, and it checks that the
//! 2560's MUX5 bit (3) does not reach ADC8..13, that MUX 0x02 has no input,
//! the internal 2.56 V reference and the MUX 0x1E 1.1 V bandgap. The
//! ATtiny1634 runs on its own I/O-space core (ADCL/ADCH 0x20/0x21, ADCSRB
//! 0x22, ADCSRA 0x23, ADMUX 0x24; ADC0..11 on PA3..PA7, PB0..PB3, PC0..PC2);
//! there the highest pad is PC2/ADC11, reached with MUX3:0 alone, and it
//! checks REFS = 10 selecting the internal 1.1 V, MUX 0x0F GND, that 0x0C..0x0E
//! have no input, and that ADLAR is ADCSRB bit 3 (not the x4's bit 4, nor
//! ADMUX bit 5, which is REFEN here). The ATmega16M1/64M1 run on the M1
//! cores (ADC0..10 on PE2, PD4..PD6, PB7, PB2, PB5, PB6, PC4..PC6, MUX4:0,
//! no MUX5); there the highest pad is PC6/ADC10 and it checks the internal
//! 2.56 V reference, the MUX 0x11 1.1 V bandgap, MUX 0x0C VCC/4, MUX 0x12
//! GND, that the 328P (0x0E) and mega (0x1E) bandgap codes have no input,
//! and that ADCSRB bits 3 and 5 (MUX5 on the 2560 and 32U4) do not move the
//! channel. The ATmega169P runs on its own core (ports A..G at the 1281's
//! addresses, ADC0..7 on PF0..PF7, MUX4:0 with gain-1 differential codes);
//! there a second session routes the NTC divider to PF0 and the diode to
//! PF1, and firmware converts MUX 0x10 (ADC0 - ADC1) at every temperature:
//! the signed result is NTC minus diode, so it turns negative between 50 °C
//! and 80 °C, and saturates at +511 against 1.1 V when cold. It also checks the internal 1.1 V reference (REFS =
//! 11), the MUX 0x1E bandgap, MUX 0x1F GND, that 0x0E (the 328P's bandgap
//! code) is N/A, and that ADCSRB bit 3 is no MUX5. Every result is read back
//! with LDS ADCL/ADCH.
//!
//! For each covered part the generic Beta NTC divider and the unrelated
//! Shockley diode route through the descriptor's `analog_pins`, to the lowest
//! and highest ADC pad. On the 100-pin megas the highest pad is PK7/ADC15
//! and on the 32U4 PB6/ADC13; both need ADCSRB.MUX5. Both solved voltages are checked against
//! independent references, the 10-bit codes against `mV * 1024 / AVcc` (5 V,
//! or the RF parts' 1.8 V AVDD), and the NTC code must fall monotonically with
//! temperature wherever it is below full scale.
//!
//! The ATtiny2313/2313A/4313 and the AT90USB162 ([`COMPARATOR_COVERED`]) have
//! no ADC; their analog input is the comparator (ACSR 0x28, AIN0 on PB0, AIN1
//! on PB1 on the tinies; ACSR 0x50, AIN0 on PD1, AIN1 on PD2 on the USB162). There
//! the same two netlists are routed to AIN0 and AIN1, both ways round, and
//! firmware reads ACO back with LDS ACSR: it must be set exactly while the
//! positive input is above the negative one, so with the NTC on AIN0 and the
//! diode on AIN1 it flips between 50 °C and 80 °C, and swapped it flips the
//! other way. ACBG (the 1.1 V bandgap on the positive input) and ACD are
//! checked against the same solved voltages.
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
use labwired_core::{Bus, Cpu, SimulationConfig};
use std::path::{Path, PathBuf};

/// Covered simavr profiles, the core they run on, and the two pads.
const COVERED: &[(&str, AvrModel, &str, &str)] = &[
    ("simavr-atmega128", AvrModel::Atmega128, "pf0", "pf7"),
    ("simavr-atmega1280", AvrModel::Atmega1281, "pf0", "pk7"),
    ("simavr-atmega1281", AvrModel::Atmega1281, "pf0", "pf7"),
    ("simavr-atmega1284p", AvrModel::Atmega1284, "pa0", "pa7"),
    ("simavr-atmega128rfa1", AvrModel::Atmega128Rf, "pf0", "pf7"),
    ("simavr-atmega128rfr2", AvrModel::Atmega128Rf, "pf0", "pf7"),
    ("simavr-atmega16", AvrModel::Atmega16, "pa0", "pa7"),
    ("simavr-atmega164", AvrModel::Atmega164, "pa0", "pa7"),
    ("simavr-atmega169p", AvrModel::Atmega169p, "pf0", "pf7"),
    ("simavr-atmega16m1", AvrModel::Atmega16m1, "pe2", "pc6"),
    ("simavr-atmega168", AvrModel::Atmega328p, "pc0", "pc5"),
    ("simavr-atmega2560", AvrModel::Atmega2560, "pf0", "pk7"),
    ("simavr-atmega32", AvrModel::Atmega32, "pa0", "pa7"),
    ("simavr-atmega324", AvrModel::Atmega324, "pa0", "pa7"),
    ("simavr-atmega328", AvrModel::Atmega328p, "pc0", "pc5"),
    ("simavr-atmega328pb", AvrModel::Atmega328pb, "pe2", "pe3"),
    ("simavr-atmega32u4", AvrModel::Atmega32u4, "pf0", "pb6"),
    ("simavr-atmega48", AvrModel::Atmega328p, "pc0", "pc5"),
    ("simavr-atmega644", AvrModel::Atmega644, "pa0", "pa7"),
    ("simavr-atmega64m1", AvrModel::Atmega64m1, "pe2", "pc6"),
    ("simavr-atmega8", AvrModel::Atmega8, "pc0", "pc5"),
    ("simavr-atmega88", AvrModel::Atmega328p, "pc0", "pc5"),
    ("simavr-attiny13", AvrModel::Attiny13, "pb5", "pb3"),
    ("simavr-attiny1634", AvrModel::Attiny1634, "pa3", "pc2"),
    ("simavr-attiny24", AvrModel::Attiny24, "pa0", "pa7"),
    ("simavr-attiny25", AvrModel::Attiny25, "pb5", "pb3"),
    ("simavr-attiny44", AvrModel::Attiny44, "pa0", "pa7"),
    ("simavr-attiny45", AvrModel::Attiny45, "pb5", "pb3"),
    ("simavr-attiny84", AvrModel::Attiny84, "pa0", "pa7"),
    ("simavr-attiny85", AvrModel::Attiny85, "pb5", "pb3"),
];

/// simavr profiles with no ADC whose comparator AIN0/AIN1 take the netlists:
/// (profile, core, ACSR data address, AIN0 pad, AIN1 pad).
const COMPARATOR_COVERED: &[(&str, AvrModel, u16, &str, &str)] = &[
    (
        "simavr-at90usb162",
        AvrModel::At90usb162,
        0x50,
        "pd1",
        "pd2",
    ),
    (
        "simavr-attiny2313",
        AvrModel::Attiny2313,
        0x28,
        "pb0",
        "pb1",
    ),
    (
        "simavr-attiny2313a",
        AvrModel::Attiny2313,
        0x28,
        "pb0",
        "pb1",
    ),
    (
        "simavr-attiny4313",
        AvrModel::Attiny4313,
        0x28,
        "pb0",
        "pb1",
    ),
];

/// simavr profiles with no analog endpoint, and why (addresses per avr-libc
/// 2.0.0). Covering one has to move it into [`COVERED`]. Empty since the
/// ATmega328PB, checked against avr-libc main's `iom328pb.h`.
const UNCOVERED: &[(&str, &str)] = &[];

const AVCC_MV: u32 = 5000;
/// ATmega128RFA1/128RFR2 ADC supply: the internal AVDD regulator (REFS = 01).
const RF_AVDD_MV: u32 = 1800;

/// Full scale of a REFS = 01 conversion on `core`.
fn avcc_mv(core: AvrModel) -> u32 {
    if core.is_rf() {
        RF_AVDD_MV
    } else {
        AVCC_MV
    }
}

/// ADCSRB/ADMUX/ADCSRA/ADCL data addresses on `core` (ADCH = ADCL + 1). The
/// classic ATmega8/16/32 and the ATtinys keep the ADC in I/O space; ADCSRB
/// is driven only on the ATtiny24/44/84 and the ATtiny1634, where it holds
/// ADLAR.
fn adc_registers(core: AvrModel) -> (Option<u16>, u16, u16, u16) {
    if core.is_tiny1634() {
        (Some(0x22), 0x24, 0x23, 0x20)
    } else if core.is_tiny_x4() {
        (Some(0x23), 0x27, 0x26, 0x24)
    } else if core.is_io_space() {
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

/// ADMUX REFS bits selecting AVcc/VCC: REFS = 01 on the megas, 00 on the
/// ATtinys (where 01 is the external AREF pin).
fn vcc_refs(core: AvrModel) -> u8 {
    if core.is_tiny_x4() || core.is_tiny_x5() || core.is_tiny13() || core.is_tiny1634() {
        0x00
    } else {
        0x40
    }
}

/// One firmware conversion of ADC `channel` against AVcc.
fn convert(chip: &ChipDescriptor, bus: &mut SystemBus, channel: u8) -> u16 {
    let refs = vcc_refs(AvrModel::from_chip_name(&chip.name));
    convert_mux(chip, bus, refs, channel)
}

/// ATtiny24/44/84 firmware conversion with ADCSRB.ADLAR (bit 4) set:
/// returns ADCH/ADCL as read back with LDS.
fn convert_left_adjusted_tiny_x4(
    chip: &ChipDescriptor,
    bus: &mut SystemBus,
    channel: u8,
) -> (u8, u8) {
    let mut cpu = Avr::for_chip_name(&chip.name);
    let mut words = vec![ldi(16, 1 << 4)];
    words.extend(sts(0x23, 16));
    words.push(ldi(16, channel));
    words.extend(sts(0x27, 16));
    words.push(ldi(16, 0xC0));
    words.extend(sts(0x26, 16));
    words.extend(lds(24, 0x24));
    words.extend(lds(25, 0x25));
    cpu.load_words(0, &words);
    let config = SimulationConfig::default();
    for _ in 0..8 {
        cpu.step(bus, &[], &config)
            .expect("left-adjusted conversion");
    }
    (cpu.r[25], cpu.r[24])
}

/// ADCSRB.MUX5 on the cores whose ADC8..ADC15 the test reaches through it:
/// bit 3 on the 1281/2560, bit 5 on the 32U4.
fn adcsrb_mux5(core: AvrModel) -> Option<u8> {
    match core {
        AvrModel::Atmega1281 | AvrModel::Atmega2560 => Some(1 << 3),
        AvrModel::Atmega32u4 => Some(1 << 5),
        _ => None,
    }
}

/// One firmware conversion: ADCSRB.MUX5 for ADC8..15 (where ADCSRB exists),
/// ADMUX = `refs` | MUX, then ADCSRA = ADEN | ADSC, and the result read back
/// with LDS ADCL/ADCH into r24/r25.
fn convert_mux(chip: &ChipDescriptor, bus: &mut SystemBus, refs: u8, mux: u8) -> u16 {
    let core = AvrModel::from_chip_name(&chip.name);
    let adcsrb = match adcsrb_mux5(core) {
        Some(mux5) if mux >= 8 => mux5,
        _ => 0,
    };
    let admux = if adcsrb_mux5(core).is_some() {
        mux & 0x07
    } else {
        mux
    };
    convert_raw(chip, bus, adcsrb, refs | admux)
}

/// One firmware conversion with ADCSRB (where it exists) and ADMUX written
/// as given.
fn convert_raw(
    chip: &ChipDescriptor,
    bus: &mut SystemBus,
    adcsrb_value: u8,
    admux_value: u8,
) -> u16 {
    let mut cpu = Avr::for_chip_name(&chip.name);
    let (adcsrb, admux, adcsra, adcl) = adc_registers(cpu.model);
    let mut words = Vec::new();
    if let Some(adcsrb) = adcsrb {
        words.push(ldi(16, adcsrb_value));
        words.extend(sts(adcsrb, 16));
    }
    words.push(ldi(16, admux_value));
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

/// ATmega128RFA1/128RFR2 firmware poll: LDS ADCSRB, set ADEN, LDS ADCSRB
/// again. Returns the AVDDOK|REFOK bits before and after ADEN.
fn poll_adcsrb_status(chip: &ChipDescriptor, bus: &mut SystemBus) -> (u8, u8) {
    let mut cpu = Avr::for_chip_name(&chip.name);
    let mut words = Vec::new();
    words.extend(lds(20, 0x7B));
    words.push(ldi(16, 0x80));
    words.extend(sts(0x7A, 16));
    words.extend(lds(21, 0x7B));
    cpu.load_words(0, &words);
    let config = SimulationConfig::default();
    for _ in 0..4 {
        cpu.step(bus, &[], &config).expect("poll ADCSRB");
    }
    (cpu.r[20] & 0xA0, cpu.r[21] & 0xA0)
}

fn expected_code(core: AvrModel, volts: f64) -> u16 {
    let mv = (volts * 1000.0).round() as u32;
    (mv * 1024 / avcc_mv(core)).min(1023) as u16
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
            expected_code(core, expected_ntc_v),
            "{board}: ADC{ntc_channel} ({ntc_pad}) converts the NTC divider at {temperature_c} °C"
        );
        assert!(
            ntc_code < last_ntc_code || (ntc_code == 1023 && last_ntc_code == 1023),
            "{board}: NTC code falls as temperature rises ({last_ntc_code} -> {ntc_code})"
        );
        if core.is_rf() && temperature_c < 0.0 {
            // The divider sits above the 1.8 V AVDD the RF ADC converts against.
            assert_eq!(ntc_code, 1023, "{board}: {expected_ntc_v} V clips at 0x3FF");
        }
        last_ntc_code = ntc_code;
        assert_eq!(
            convert(chip, &mut bus, diode_channel),
            expected_code(core, expected_diode_v),
            "{board}: ADC{diode_channel} ({diode_pad}) converts the diode load line"
        );
    }
    // A channel neither netlist drives keeps the undriven mid-rail level.
    let idle = (0..8)
        .find(|ch| *ch != ntc_channel && *ch != diode_channel)
        .unwrap();
    assert_eq!(
        convert(chip, &mut bus, idle),
        expected_code(core, f64::from(AVR_ADC_UNDRIVEN_MV) / 1000.0),
        "{board}: idle ADC{idle} is not written by either route"
    );

    if core.is_xx4() {
        // REFS = 10 is the internal 1.1 V and 11 the internal 2.56 V; MUX 0x1E
        // is the 1.1 V bandgap (not the 328P's 0x0E, a differential code here).
        let mv = (expected_diode_v * 1000.0).round() as u32;
        assert_eq!(
            convert_mux(chip, &mut bus, 0x80, diode_channel),
            (mv * 1024 / 1100).min(1023) as u16,
            "{board}: the diode converts against the internal 1.1 V reference"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0xC0, diode_channel),
            (mv * 1024 / 2560) as u16,
            "{board}: the diode converts against the internal 2.56 V reference"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, 0x1E),
            (1100 * 1024 / AVCC_MV) as u16,
            "{board}: MUX 0x1E converts the 1.1 V bandgap"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, 0x0E),
            0,
            "{board}: MUX 0x0E is a differential code, not the bandgap"
        );
    }
    if core.is_rf() {
        // REFS = 10 is the internal 1.5 V and 11 the internal 1.6 V (not the
        // 1281's 1.1 V / 2.56 V); MUX 0x1E is the 1.2 V VBG and 0x1F AVSS.
        let mv = (expected_diode_v * 1000.0).round() as u32;
        assert_eq!(
            convert_mux(chip, &mut bus, 0x80, diode_channel),
            (mv * 1024 / 1500) as u16,
            "{board}: the diode converts against the internal 1.5 V reference"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0xC0, diode_channel),
            (mv * 1024 / 1600) as u16,
            "{board}: the diode converts against the internal 1.6 V reference"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, 0x1E),
            (1200 * 1024 / RF_AVDD_MV) as u16,
            "{board}: MUX 0x1E converts the 1.2 V bandgap against AVDD"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, 0x1F),
            0,
            "{board}: MUX 0x1F is AVSS"
        );
        assert_eq!(
            poll_adcsrb_status(chip, &mut bus),
            (0, 0xA0),
            "{board}: AVDDOK/REFOK read 0 before ADEN and 1 after"
        );
    }
    if core.is_tiny_x4() {
        // REFS = 10 is the internal 1.1 V. MUX5:0 is six bits: 0x21 is the
        // 1.1 V bandgap, 0x20 AGND and 0x22 the unmodelled temperature
        // sensor; 0x0C (the x5 bandgap), 0x1E (the megas') and 0x08
        // (differential, not ADC8) have no input.
        let mv = (expected_diode_v * 1000.0).round() as u32;
        assert_eq!(
            convert_mux(chip, &mut bus, 0x80, diode_channel),
            (mv * 1024 / 1100).min(1023) as u16,
            "{board}: the diode converts against the internal 1.1 V reference"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x00, 0x21),
            (1100 * 1024 / AVCC_MV) as u16,
            "{board}: MUX 0x21 converts the 1.1 V bandgap"
        );
        for code in [0x08, 0x0C, 0x1E, 0x20, 0x22] {
            assert_eq!(
                convert_mux(chip, &mut bus, 0x00, code),
                0,
                "{board}: MUX {code:#04x} is not a modelled input"
            );
        }
        // ADLAR lives in ADCSRB bit 4: the diode code comes back left-adjusted.
        let code = expected_code(core, expected_diode_v);
        assert_eq!(
            convert_left_adjusted_tiny_x4(chip, &mut bus, diode_channel),
            ((code >> 2) as u8, ((code & 0x03) << 6) as u8),
            "{board}: ADCSRB.ADLAR left-adjusts the diode result"
        );
    }
    if core.is_tiny_x5() {
        // REFS2 is ADMUX bit 4 here (MUX4 on the ATmega16/32): REFS2:0 = 010
        // is the internal 1.1 V and 110 the internal 2.56 V. MUX 0x0C is the
        // 1.1 V bandgap and 0x0D GND; 0x0E (the ATmega8's bandgap) has no
        // input and 0x0F is the unmodelled temperature sensor.
        let mv = (expected_diode_v * 1000.0).round() as u32;
        assert_eq!(
            convert_mux(chip, &mut bus, 0x80, diode_channel),
            (mv * 1024 / 1100).min(1023) as u16,
            "{board}: the diode converts against the internal 1.1 V reference"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x90, diode_channel),
            (mv * 1024 / 2560) as u16,
            "{board}: REFS2:0 = 110 converts against the internal 2.56 V"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, 0x0C),
            (1100 * 1024 / AVCC_MV) as u16,
            "{board}: MUX 0x0C converts the 1.1 V bandgap"
        );
        for code in [0x0D, 0x0E, 0x0F] {
            assert_eq!(
                convert_mux(chip, &mut bus, 0x40, code),
                0,
                "{board}: MUX {code:#04x} is not a modelled input"
            );
        }
        // Only four channels: MUX 0x04 is a differential code, not ADC4.
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, 0x04),
            0,
            "{board}: MUX 0x04 is differential"
        );
    }
    if core.is_tiny13() {
        // REFS0 is the only reference bit: ADMUX bit 6 (AVcc on the megas,
        // AREF on the other tinies) is the internal 1.1 V here, and bit 7
        // (the x5 parts' REFS1) is reserved, so 0x80 is still VCC.
        let mv = (expected_diode_v * 1000.0).round() as u32;
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, diode_channel),
            (mv * 1024 / 1100).min(1023) as u16,
            "{board}: REFS0 converts the diode against the internal 1.1 V"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x80, diode_channel),
            (mv * 1024 / AVCC_MV) as u16,
            "{board}: ADMUX bit 7 is reserved, VCC stays the reference"
        );
        // MUX3:2 are reserved: the x5 bandgap code 0x0C selects ADC0, the
        // NTC pad, not a 1.1 V input.
        assert_eq!(
            convert_mux(chip, &mut bus, 0x00, 0x0C),
            convert_mux(chip, &mut bus, 0x00, 0x00),
            "{board}: MUX 0x0C is ADC0 (no bandgap input)"
        );
        assert_ne!(
            convert_mux(chip, &mut bus, 0x00, 0x0C),
            (1100 * 1024 / AVCC_MV) as u16,
            "{board}: MUX 0x0C is not the bandgap"
        );
    }
    if core.is_tiny1634() {
        // REFS = 10 is the internal 1.1 V (01 is AREF, taken as VCC). MUX3:0
        // reaches ADC11 directly; 0x0C/0x0D are reserved, 0x0E is the
        // unmodelled temperature sensor and 0x0F is GND.
        let mv = (expected_diode_v * 1000.0).round() as u32;
        let code = expected_code(core, expected_diode_v);
        assert_eq!(
            convert_mux(chip, &mut bus, 0x80, diode_channel),
            (mv * 1024 / 1100).min(1023) as u16,
            "{board}: the diode converts against the internal 1.1 V reference"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, diode_channel),
            code,
            "{board}: REFS = 01 (AREF) is taken as VCC"
        );
        for mux in 0x0C..=0x0F {
            assert_eq!(
                convert_mux(chip, &mut bus, 0x00, mux),
                0,
                "{board}: MUX {mux:#04x} is not a modelled input"
            );
        }
        // ADLAR is ADCSRB bit 3: ADCH:ADCL is the code shifted left by 6.
        assert_eq!(
            convert_raw(chip, &mut bus, 1 << 3, diode_channel),
            code << 6,
            "{board}: ADCSRB.ADLAR (bit 3) left-adjusts the diode result"
        );
        assert_eq!(
            convert_raw(chip, &mut bus, 1 << 4, diode_channel),
            code,
            "{board}: ADCSRB bit 4 (the x4's ADLAR) leaves it right-adjusted"
        );
        assert_eq!(
            convert_raw(chip, &mut bus, 0, 0x20 | diode_channel),
            code,
            "{board}: ADMUX bit 5 is REFEN here, not ADLAR"
        );
    }
    if core.is_u4() {
        // MUX5 is ADCSRB bit 5: with bit 3 (the 2560's MUX5) instead, MUX4:0 =
        // 5 stays ADC5 (PF5, undriven), not ADC13 (PB6, the diode).
        let mv = (expected_diode_v * 1000.0).round() as u32;
        assert_eq!(
            convert_raw(chip, &mut bus, 1 << 3, 0x45),
            expected_code(core, f64::from(AVR_ADC_UNDRIVEN_MV) / 1000.0),
            "{board}: ADCSRB bit 3 is not MUX5 on the 32U4"
        );
        assert_eq!(
            convert_raw(chip, &mut bus, 1 << 5, 0x45),
            (mv * 1024 / AVCC_MV) as u16,
            "{board}: ADCSRB bit 5 with MUX 5 is ADC13 (PB6)"
        );
        // There is no ADC2/ADC3: MUX 0x02 has no input even though the window
        // slot exists.
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, 0x02),
            0,
            "{board}: no ADC2"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0xC0, diode_channel),
            (mv * 1024 / 2560) as u16,
            "{board}: the diode converts against the internal 2.56 V reference"
        );
        assert_eq!(
            convert_raw(chip, &mut bus, 0, 0x5E),
            (1100 * 1024 / AVCC_MV) as u16,
            "{board}: MUX 0x1E converts the 1.1 V bandgap"
        );
    }
    if core.is_m1() {
        // MUX4:0 reaches ADC10 directly and ADCSRB has no MUX5: bit 3 (the
        // 2560's MUX5) and bit 5 (the 32U4's; AREFEN here) leave ADC10.
        let mv = (expected_diode_v * 1000.0).round() as u32;
        let code = expected_code(core, expected_diode_v);
        for adcsrb in [1 << 3, 1 << 5] {
            assert_eq!(
                convert_raw(chip, &mut bus, adcsrb, 0x40 | diode_channel),
                code,
                "{board}: ADCSRB {adcsrb:#04x} is not MUX5 on the M1"
            );
        }
        assert_eq!(
            convert_mux(chip, &mut bus, 0xC0, diode_channel),
            (mv * 1024 / 2560) as u16,
            "{board}: the diode converts against the internal 2.56 V reference"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, 0x11),
            (1100 * 1024 / AVCC_MV) as u16,
            "{board}: MUX 0x11 converts the 1.1 V bandgap"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, 0x0C),
            (AVCC_MV / 4 * 1024 / AVCC_MV) as u16,
            "{board}: MUX 0x0C converts VCC/4"
        );
        // 0x12 is GND; 0x0E (the 328P's bandgap code) is AMP0 and 0x1E (the
        // megas') is reserved here.
        for mux in [0x12, 0x0E, 0x1E] {
            assert_eq!(
                convert_mux(chip, &mut bus, 0x40, mux),
                0,
                "{board}: MUX {mux:#04x} has no modelled input"
            );
        }
    }
    if core.is_m169p() {
        assert_m169p_differential_measures_ntc_against_diode(board, chip, base_profile);
        // REFS = 11 is the internal 1.1 V; ADCSRB bit 3 is not MUX5.
        let mv = (expected_diode_v * 1000.0).round() as u32;
        assert_eq!(
            convert_mux(chip, &mut bus, 0xC0, diode_channel),
            (mv * 1024 / 1100).min(1023) as u16,
            "{board}: the diode converts against the internal 1.1 V reference"
        );
        assert_eq!(
            convert_raw(chip, &mut bus, 1 << 3, 0x40 | diode_channel),
            expected_code(core, expected_diode_v),
            "{board}: ADCSRB bit 3 is not MUX5 on the 169P"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, 0x1E),
            (1100 * 1024 / AVCC_MV) as u16,
            "{board}: MUX 0x1E converts the 1.1 V bandgap"
        );
        for mux in [0x1F, 0x0E, 0x08] {
            assert_eq!(
                convert_mux(chip, &mut bus, 0x40, mux),
                0,
                "{board}: MUX {mux:#04x} has no modelled input"
            );
        }
    }
    if core == AvrModel::Atmega328pb {
        assert_m328pb_port_e_carries_the_analog_pads(board, chip, base_profile);
        // The 328P's mux table (sim_mega328pb.c muxmode): REFS = 11 is the
        // internal 1.1 V, 0x0E the 1.1 V bandgap, 0x0F GND.
        let mv = (expected_diode_v * 1000.0).round() as u32;
        assert_eq!(
            convert_mux(chip, &mut bus, 0xC0, diode_channel),
            (mv * 1024 / 1100).min(1023) as u16,
            "{board}: the PE3 diode converts against the internal 1.1 V reference"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, 0x0E),
            (1100 * 1024 / AVCC_MV) as u16,
            "{board}: MUX 0x0E converts the 1.1 V bandgap"
        );
        assert_eq!(
            convert_mux(chip, &mut bus, 0x40, 0x0F),
            0,
            "{board}: MUX 0x0F is GND"
        );
    }
    if core.is_classic() || core.is_mega128() {
        // REFS = 11 is the classic parts' internal 2.56 V, not the 328P's 1.1 V.
        let mv = (expected_diode_v * 1000.0).round() as u32;
        assert_eq!(
            convert_mux(chip, &mut bus, 0xC0, diode_channel),
            (mv * 1024 / 2560) as u16,
            "{board}: the diode converts against the internal 2.56 V reference"
        );
        // The bandgap input: 1.30 V on MUX 0x0E (ATmega8), 1.22 V on 0x1E
        // (16/32), 1.23 V on 0x1E (128).
        let (code, bandgap_mv) = if core == AvrModel::Atmega8 {
            (0x0E, 1300)
        } else if core.is_mega128() {
            (0x1E, 1230)
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

/// ATmega328PB: ADC6/ADC7 are bonded to PE2/PE3, so the routed pads must be
/// port E's and firmware must reach that port at PINE/DDRE/PORTE
/// (0x2C..0x2E). Firmware writes DDRE/PORTE with STS, and the bus-side port E
/// window must hold them: the 328P core keeps those addresses as plain
/// storage and would leave the window untouched.
fn assert_m328pb_port_e_carries_the_analog_pads(
    board: &str,
    chip: &ChipDescriptor,
    base_profile: &SystemManifest,
) {
    for (pad, channel) in [("PE2", 6), ("PE3", 7)] {
        assert_eq!(
            analog_channel(chip, pad),
            channel,
            "{board}: {pad} is ADC{channel}"
        );
    }
    assert!(
        !chip
            .analog_pins
            .keys()
            .any(|pad| pad.eq_ignore_ascii_case("PC6") || pad.eq_ignore_ascii_case("PC7")),
        "{board}: ADC6/ADC7 have no dedicated pads on the 328PB"
    );
    let mut bus = SystemBus::from_config(chip, base_profile).expect("build 328PB bus");
    let porte = bus
        .peripherals
        .iter()
        .find(|p| p.name == "porte")
        .unwrap_or_else(|| panic!("{board}: port E window"));
    assert_eq!(porte.base, 0x1_002C, "{board}: port E mirrors PINE at 0x2C");

    let mut cpu = Avr::for_chip_name(&chip.name);
    let mut words = Vec::new();
    // PE0/PE1 outputs driven 1/0; PE2/PE3 inputs, pull-ups off.
    words.push(ldi(16, 0x03));
    words.extend(sts(0x2D, 16));
    words.push(ldi(16, 0x01));
    words.extend(sts(0x2E, 16));
    words.extend(lds(24, 0x2D));
    cpu.load_words(0, &words);
    let config = SimulationConfig::default();
    for _ in 0..5 {
        cpu.step(&mut bus, &[], &config).expect("port E setup");
    }
    assert_eq!(cpu.r[24], 0x03, "{board}: LDS DDRE reads back");
    assert_eq!(
        bus.read_u8(0x1_002D).expect("DDRE mirror"),
        0x03,
        "{board}: DDRE reaches the port E window"
    );
    assert_eq!(
        bus.read_u8(0x1_002E).expect("PORTE mirror") & 0x0F,
        0x01,
        "{board}: PORTE drives PE0 high and PE1 low, PE2/PE3 undriven"
    );
}

/// ATmega169P gain-1 differential channels (datasheet 8018P §22.7, Table
/// 22-4) measuring one device against the other: the NTC divider on PF0/ADC0
/// and the diode on PF1/ADC1. MUX 0x10 (ADC0 - ADC1) must equal
/// `(V_ntc - V_diode) * 512 / Vref` as a signed 10-bit result, so it is
/// positive while the divider is above the diode's forward voltage and turns
/// negative between 50 °C and 80 °C. MUX 0x19 (ADC1 - ADC2) puts the diode
/// against the undriven ADC2, and at the 1.1 V reference ADC0 - ADC1
/// saturates at +511 when cold.
fn assert_m169p_differential_measures_ntc_against_diode(
    board: &str,
    chip: &ChipDescriptor,
    base_profile: &SystemManifest,
) {
    let mut profile = base_profile.clone();
    profile.cosim_models = generic_netlists("pf0", "pf1").cosim_models;
    let mut bus = SystemBus::from_config(chip, &profile).expect("build simavr bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(session.binding_errors(), &[] as &[RoutingError]);
    let signed = |delta_mv: i32, reference_mv: i32| {
        ((delta_mv * 512 / reference_mv).clamp(-512, 511) & 0x3FF) as u16
    };
    let diode_mv = (diode_load_line_voltage() * 1000.0).round() as i32;
    let mut cycles = 0;
    for temperature_c in [-10.0_f64, 0.0, 25.0, 50.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (_, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both netlists");
        assert!(errors.is_empty(), "{board}: route errors: {errors:?}");
        let r = ntc_resistance_ohms(temperature_c);
        let ntc_mv = (3.3 * r / (10_000.0 + r) * 1000.0).round() as i32;
        let code = convert_mux(chip, &mut bus, 0x40, 0x10);
        assert_eq!(
            code,
            signed(ntc_mv - diode_mv, AVCC_MV as i32),
            "{board}: ADC0 - ADC1 is NTC minus diode at {temperature_c} °C"
        );
        assert_eq!(
            code & 0x200 != 0,
            temperature_c >= 80.0,
            "{board}: the divider drops below the diode between 50 and 80 °C"
        );
        if temperature_c < 0.0 {
            assert_eq!(
                convert_mux(chip, &mut bus, 0xC0, 0x10),
                0x1FF,
                "{board}: ADC0 - ADC1 saturates at +511 against 1.1 V"
            );
        }
    }
    assert_eq!(
        convert_mux(chip, &mut bus, 0x40, 0x19),
        signed(diode_mv - i32::from(AVR_ADC_UNDRIVEN_MV), AVCC_MV as i32),
        "{board}: MUX 0x19 is ADC1 - ADC2, the diode against the undriven pad"
    );
}

/// `ACSR` bits, the same on the ATtiny2313/4313 (avr-libc `iotn2313.h`) and
/// the AT90USB162 (`iousb162.h`).
const ACSR_ACD: u8 = 1 << 7;
const ACSR_ACBG: u8 = 1 << 6;
const ACSR_ACO: u8 = 1 << 5;
/// The comparator's internal bandgap with ACBG set (1.1 V typical).
const BANDGAP_V: f64 = 1.1;

/// Firmware writes `acsr_value` to ACSR at `acsr` and loads ACSR back:
/// `LDI r16, K; STS ACSR, r16; LDS r24, ACSR`.
fn read_acsr(chip: &ChipDescriptor, bus: &mut SystemBus, acsr: u16, acsr_value: u8) -> u8 {
    let mut cpu = Avr::for_chip_name(&chip.name);
    let mut words = vec![ldi(16, acsr_value)];
    words.extend(sts(acsr, 16));
    words.extend(lds(24, acsr));
    cpu.load_words(0, &words);
    let config = SimulationConfig::default();
    for _ in 0..3 {
        cpu.step(bus, &[], &config)
            .expect("execute ACSR write and read");
    }
    cpu.r[24]
}

/// The generic NTC divider and Shockley diode on AIN0/AIN1 of a part whose
/// only analog input is the comparator. `ntc_on_ain0` picks which way round.
fn assert_avr_comparator_compares_generic_analog(
    board: &str,
    chip: &ChipDescriptor,
    base_profile: &SystemManifest,
    (core, acsr, ain0, ain1): (AvrModel, u16, &str, &str),
    ntc_on_ain0: bool,
) {
    assert_eq!(
        Avr::for_chip_name(&chip.name).model,
        core,
        "{board}: descriptor name selects the core whose map it was checked against"
    );
    assert_eq!(
        core.comparator_acsr(),
        Some(acsr),
        "{board}: comparator-only core with ACSR at {acsr:#x}"
    );
    let (ntc_pad, diode_pad) = if ntc_on_ain0 {
        (ain0, ain1)
    } else {
        (ain1, ain0)
    };
    assert_eq!(analog_channel(chip, ain0), 0, "{board}: AIN0 is channel 0");
    assert_eq!(analog_channel(chip, ain1), 1, "{board}: AIN1 is channel 1");

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

    // Before any solve both pins sit at the undriven level: equal, so ACO = 0.
    assert_eq!(
        read_acsr(chip, &mut bus, acsr, 0) & ACSR_ACO,
        0,
        "{board}: equal inputs"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut outputs = Vec::new();
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

        let ntc_v = signal_voltage(&session, &format!("board.analog.{ntc_pad}_volts"));
        let r = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * r / (10_000.0 + r);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "{board}: at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, &format!("board.analog.{diode_pad}_volts"));
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "{board}: Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let (ain0_v, ain1_v) = if ntc_on_ain0 {
            (expected_ntc_v, expected_diode_v)
        } else {
            (expected_diode_v, expected_ntc_v)
        };
        assert!(
            (ain0_v - ain1_v).abs() > 0.05 && (BANDGAP_V - ain1_v).abs() > 0.05,
            "{board}: the fixture keeps every comparison clear of millivolt rounding"
        );
        let aco = read_acsr(chip, &mut bus, acsr, 0) & ACSR_ACO != 0;
        assert_eq!(
            aco,
            ain0_v > ain1_v,
            "{board}: at {temperature_c} °C ACO compares AIN0 {ain0_v} V with AIN1 {ain1_v} V"
        );
        outputs.push(aco);
        let with_bg = read_acsr(chip, &mut bus, acsr, ACSR_ACBG);
        assert_eq!(with_bg & ACSR_ACBG, ACSR_ACBG, "{board}: ACBG reads back");
        assert_eq!(
            with_bg & ACSR_ACO != 0,
            BANDGAP_V > ain1_v,
            "{board}: at {temperature_c} °C ACBG compares the 1.1 V bandgap with AIN1 {ain1_v} V"
        );
        assert_eq!(
            read_acsr(chip, &mut bus, acsr, ACSR_ACD) & ACSR_ACO,
            0,
            "{board}: ACD switches the comparator off"
        );
    }
    // The NTC divider crosses the diode's forward voltage between 50 and 80 °C.
    let expected: &[bool] = if ntc_on_ain0 {
        &[true, true, true, true, false]
    } else {
        &[false, false, false, false, true]
    };
    assert_eq!(outputs, expected, "{board}: ACO over -10/0/25/50/80 °C");
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
        if let Some((_, core, acsr, ain0, ain1)) =
            COMPARATOR_COVERED.iter().find(|(b, ..)| *b == board)
        {
            for ntc_on_ain0 in [true, false] {
                assert_avr_comparator_compares_generic_analog(
                    &board,
                    &chip,
                    &profile,
                    (*core, *acsr, ain0, ain1),
                    ntc_on_ain0,
                );
            }
            covered.push(board);
            continue;
        }
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
    let mut expected_covered: Vec<_> = COVERED
        .iter()
        .map(|(b, ..)| b.to_string())
        .chain(COMPARATOR_COVERED.iter().map(|(b, ..)| b.to_string()))
        .collect();
    expected_covered.sort();
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
