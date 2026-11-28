// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Four more STM32 boards solve the generic NTC divider and Shockley diode
//! load line through their chip descriptor's `analog_pins:` map:
//!
//! | Profile         | Chip        | Converter layout | PA0 / PA1    |
//! | --------------- | ----------- | ---------------- | ------------ |
//! | `mb1355c`       | STM32WB55   | L4 (`SQR1`)      | IN5 / IN6    |
//! | `nucleo-g431rb` | STM32G431RB | L4 (`SQR1`)      | IN1 / IN2    |
//! | `nucleo_g474re` | STM32G474RE | L4 (`SQR1`)      | IN1 / IN2    |
//! | `nucleo-l031k6` | STM32L031K6 | L0 (`CHSELR`)    | IN0 / IN1    |
//!
//! The decks are byte-identical circuits to the NUCLEO-L476RG deck and name
//! only `board.analog.pa0_volts` / `board.analog.pa1_volts`. The same two pads
//! are three different channel pairs on these parts (ST STM32_open_pin_data),
//! so a map copied from another family converts an empty channel and the
//! count assertion fails.
//!
//! The conversion uses each model's register map: the WB55 and G4 select
//! `SQR1.SQ1` behind `RCC_AHB2ENR` bit 13, the L031 sets a `CHSELR` bit behind
//! `RCC_APB2ENR` bit 9. It is not a silicon bring-up sequence (it skips the
//! ADC clock source, regulator delay and calibration).

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::Path;

const ADC_ISR: u64 = 0x00;
const ADC_CR: u64 = 0x08;
const ADC_CHSELR: u64 = 0x28;
const ADC_SQR1: u64 = 0x30;
const ADC_DR: u64 = 0x40;

const ISR_ADRDY: u32 = 1 << 0;
const ISR_EOC: u32 = 1 << 2;
const CR_ADEN: u32 = 1 << 0;
const CR_ADSTART: u32 = 1 << 2;
const CR_ADVREGEN: u32 = 1 << 28;

const VREF_MV: f64 = 3_300.0;
const SUPPLY_V: f64 = 3.3;

/// How the regular group names its one channel.
#[derive(Clone, Copy, Debug)]
enum Select {
    /// L4-layout rank field: `SQR1.SQ1` at bits 10:6.
    Sqr1,
    /// L0-layout channel bitmap: one `CHSELR` bit per input.
    Chselr,
}

struct Board {
    profile: &'static str,
    deck: &'static str,
    adc_base: u64,
    /// Address of the RCC enable register that gates the ADC, and its bit.
    rcc_enr: u64,
    rcc_adcen: u32,
    select: Select,
    /// The descriptor's full `analog_pins:` map (ST STM32_open_pin_data).
    pads: &'static [(&'static str, u8)],
    /// An ADC pad the generic deck does not use, for the reroute case.
    other_pad: (&'static str, u8),
    /// A pad with no input on the instantiated converter.
    refused_pad: &'static str,
}

/// LQFP64 ADC1 inputs shared by the STM32G431RB and STM32G474RE.
const G4_PADS: &[(&str, u8)] = &[
    ("PA0", 1),
    ("PA1", 2),
    ("PA2", 3),
    ("PA3", 4),
    ("PB14", 5),
    ("PC0", 6),
    ("PC1", 7),
    ("PC2", 8),
    ("PC3", 9),
    ("PB12", 11),
    ("PB1", 12),
    ("PB11", 14),
    ("PB0", 15),
];

const BOARDS: &[Board] = &[
    Board {
        profile: "mb1355c",
        deck: "examples/mb1355c/analog-ntc-diode-pa0-pa1-system.yaml",
        adc_base: 0x5004_0000,
        // RM0434: RCC at 0x5800_0000, AHB2ENR at 0x4C, ADCEN bit 13.
        rcc_enr: 0x5800_0000 + 0x4C,
        rcc_adcen: 1 << 13,
        select: Select::Sqr1,
        pads: &[
            ("PC0", 1),
            ("PC1", 2),
            ("PC2", 3),
            ("PC3", 4),
            ("PA0", 5),
            ("PA1", 6),
            ("PA2", 7),
            ("PA3", 8),
            ("PA4", 9),
            ("PA5", 10),
            ("PA6", 11),
            ("PA7", 12),
            ("PC4", 13),
            ("PC5", 14),
            ("PA8", 15),
            ("PA9", 16),
        ],
        other_pad: ("PA8", 15),
        // The WB55 has no ADC input on port B.
        refused_pad: "PB0",
    },
    Board {
        profile: "nucleo-g431rb",
        deck: "examples/nucleo-g431rb/analog-ntc-diode-pa0-pa1-system.yaml",
        adc_base: 0x5000_0000,
        // RM0440: RCC at 0x4002_1000, AHB2ENR at 0x4C, ADC12EN bit 13.
        rcc_enr: 0x4002_1000 + 0x4C,
        rcc_adcen: 1 << 13,
        select: Select::Sqr1,
        pads: G4_PADS,
        other_pad: ("PB0", 15),
        // PA4 is ADC2_IN17 only; the descriptor instantiates no ADC2.
        refused_pad: "PA4",
    },
    Board {
        profile: "nucleo_g474re",
        deck: "examples/nucleo_g474re/analog-ntc-diode-pa0-pa1-system.yaml",
        adc_base: 0x5000_0000,
        rcc_enr: 0x4002_1000 + 0x4C,
        rcc_adcen: 1 << 13,
        select: Select::Sqr1,
        pads: G4_PADS,
        other_pad: ("PB14", 5),
        // PA8 is ADC5_IN1 only; the descriptor instantiates no ADC5.
        refused_pad: "PA8",
    },
    Board {
        profile: "nucleo-l031k6",
        deck: "examples/nucleo-l031k6/analog-ntc-diode-pa0-pa1-system.yaml",
        adc_base: 0x4001_2400,
        // RM0377: RCC at 0x4002_1000, APB2ENR at 0x34, ADCEN bit 9.
        rcc_enr: 0x4002_1000 + 0x34,
        rcc_adcen: 1 << 9,
        select: Select::Chselr,
        pads: &[
            ("PA0", 0),
            ("PA1", 1),
            ("PA2", 2),
            ("PA3", 3),
            ("PA4", 4),
            ("PA5", 5),
            ("PA6", 6),
            ("PA7", 7),
            ("PB0", 8),
            ("PB1", 9),
        ],
        other_pad: ("PB0", 8),
        refused_pad: "PA9",
    },
];

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line at the 3.3 V rail, including the diode's
/// series resistance.
fn diode_load_line_voltage() -> f64 {
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

fn signal_voltage(session: &CosimSession, pad: &str) -> f64 {
    let path = format!("board.analog.{}_volts", pad.to_ascii_lowercase());
    match session.signals().get(&path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// Wrap the board bus in the production `Machine`, parked on a branch-to-self
/// guest, so a conversion completes through the same boundary firmware uses.
fn board_machine(mut bus: SystemBus) -> Machine<CortexM> {
    let spin_pc = 0x0800_0040;
    bus.write_u16(spin_pc, 0xe7fe).expect("write b ."); // b .
    let (mut cpu, _) = configure_cortex_m(&mut bus);
    cpu.pc = spin_pc as u32;
    cpu.sp = 0x2000_1000;
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine
}

/// The 12-bit code the model derives for a routed node on a 3.3 V Vref.
fn expected_adc_count(voltage_v: f64) -> u32 {
    let millivolts = (voltage_v * 1000.0).round();
    (millivolts * 4095.0 / VREF_MV).floor().clamp(0.0, 4095.0) as u32
}

/// One software-triggered regular conversion: clock gate, channel select on
/// the board's layout, regulator, enable, start, `EOC`, `DR`.
fn convert(board: &Board, machine: &mut Machine<CortexM>, channel: u8) -> u32 {
    let name = board.profile;
    let base = board.adc_base;
    let bus = &mut machine.bus;
    let current = bus.read_u32(board.rcc_enr).expect("read the RCC gate");
    bus.write_u32(board.rcc_enr, current | board.rcc_adcen)
        .expect("enable the ADC clock");
    match board.select {
        Select::Sqr1 => bus.write_u32(base + ADC_SQR1, u32::from(channel) << 6),
        Select::Chselr => bus.write_u32(base + ADC_CHSELR, 1 << channel),
    }
    .expect("select the regular channel");
    bus.write_u32(base + ADC_CR, CR_ADVREGEN)
        .expect("start the ADC regulator");
    bus.write_u32(base + ADC_CR, CR_ADVREGEN | CR_ADEN)
        .expect("enable the converter");
    bus.write_u32(base + ADC_CR, CR_ADVREGEN | CR_ADEN | CR_ADSTART)
        .expect("software-start the conversion");
    for _ in 0..40 {
        machine.run(Some(1)).expect("advance the guest one cycle");
    }
    let bus = &mut machine.bus;
    let status = bus.read_u32(base + ADC_ISR).expect("read ADC ISR");
    assert_ne!(status & ISR_ADRDY, 0, "{name}: ADRDY must be set");
    assert_ne!(status & ISR_EOC, 0, "{name}: EOC must follow ADSTART");
    bus.read_u32(base + ADC_DR).expect("read ADC DR")
}

/// Load the board profile, replace its co-simulation with the example deck
/// (optionally moving the diode probe to `diode_pad`), and bind it to the bus.
fn bench(
    board: &Board,
    diode_pad: Option<&str>,
) -> (ChipDescriptor, Machine<CortexM>, CosimSession) {
    let name = board.profile;
    let system_path = root(&format!("configs/systems/{name}.yaml"));
    let mut profile = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|error| panic!("load {name} profile: {error}"));
    assert_eq!(profile.name, name);
    let mut models = SystemManifest::from_file(root(board.deck))
        .unwrap_or_else(|error| panic!("load {}: {error}", board.deck))
        .cosim_models;
    if let Some(pad) = diode_pad {
        models[1].outputs.insert(
            "v_diode".to_string(),
            format!("board.analog.{}_volts", pad.to_ascii_lowercase()),
        );
    }
    profile.cosim_models = models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|error| panic!("load {name} descriptor: {error}"));
    let bus = SystemBus::from_config(&chip, &profile)
        .unwrap_or_else(|error| panic!("build the {name} bus: {error}"));
    let machine = board_machine(bus);
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .unwrap_or_else(|error| panic!("build {name} co-simulation: {error}"))
        .unwrap_or_else(|| panic!("{name} deck carries its analog devices"));
    (chip, machine, session)
}

#[test]
fn wb55_g4_and_l031_boards_solve_generic_decks_and_convert() {
    let expected_diode_v = diode_load_line_voltage();
    for board in BOARDS {
        let name = board.profile;
        let (chip, mut machine, mut session) = bench(board, None);

        assert_eq!(
            chip.analog_pin_conflicts(),
            Vec::new(),
            "{name}: the pin table and the analog pad map must agree"
        );
        assert_eq!(
            chip.analog_pins.len(),
            board.pads.len(),
            "{name}: the map publishes exactly the package ADC inputs"
        );
        for (pad, channel) in board.pads {
            let pin = &chip.analog_pins[*pad];
            assert_eq!(
                (pin.peripheral.as_str(), pin.channel),
                ("adc1", *channel),
                "{name}: {pad} must be adc1 channel {channel}"
            );
        }
        assert_eq!(
            session.binding_errors(),
            &[] as &[RoutingError],
            "{name}: PA0/PA1 must resolve through the chip pad map"
        );
        let channel_of = |pad: &str| chip.analog_pins[pad].channel;
        let (ntc_channel, diode_channel) = (channel_of("PA0"), channel_of("PA1"));

        let mut cycles = 0;
        let mut samples = Vec::new();
        for temperature_c in [-20.0_f64, 25.0, 85.0] {
            session
                .set_signal_number("ui.ntc.temperature_c", temperature_c)
                .unwrap_or_else(|error| panic!("set {name} NTC temperature: {error}"));
            cycles += session.cycles_until_boundary(cycles);
            let (steps, errors) = session
                .advance_to(cycles, &mut machine.bus)
                .unwrap_or_else(|error| panic!("solve the {name} decks: {error}"));
            assert_eq!(steps.len(), 2, "{name}: both netlists cross the boundary");
            assert!(errors.is_empty(), "{name}: route errors: {errors:?}");

            let ntc_v = signal_voltage(&session, "PA0");
            let expected_ntc_v =
                SUPPLY_V * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
            assert!(
                (ntc_v - expected_ntc_v).abs() < 1e-9,
                "{name} at {temperature_c} C: the Beta divider on PA0 should be {expected_ntc_v} V, got {ntc_v} V"
            );
            let diode_v = signal_voltage(&session, "PA1");
            assert!(
                (diode_v - expected_diode_v).abs() < 2e-5,
                "{name}: the Shockley load line on PA1 should be {expected_diode_v} V, got {diode_v} V"
            );

            let ntc_code = convert(board, &mut machine, ntc_channel);
            assert_eq!(
                ntc_code,
                expected_adc_count(ntc_v),
                "{name}: channel {ntc_channel} (PA0) must report the solved NTC voltage"
            );
            assert_eq!(
                convert(board, &mut machine, diode_channel),
                expected_adc_count(diode_v),
                "{name}: channel {diode_channel} (PA1) must report the solved diode voltage"
            );
            samples.push((temperature_c, ntc_v, ntc_code));
        }

        assert!(
            samples[2].2 > samples[1].2 && samples[1].2 > samples[0].2,
            "{name}: ADC counts rise with NTC temperature: {samples:?}"
        );
        assert_eq!(
            samples[1].2, 2047,
            "{name}: the 25 C divider is half scale: 1650 mV * 4095 / 3300 = 2047"
        );
    }
}

/// The diode moves to a pad the default deck does not use. PB0 is ADC1_IN15
/// on the G431 and ADC_IN8 on the L031; PA8 is ADC1_IN15 on the WB55; PB14 is
/// ADC1_IN5 on the G474. The solved voltage must arrive on that channel, and
/// the channel PA1 used before must no longer carry it.
#[test]
fn a_rerouted_diode_converts_on_the_channel_each_descriptor_names() {
    let expected_diode_v = diode_load_line_voltage();
    for board in BOARDS {
        let name = board.profile;
        let (pad, channel) = board.other_pad;
        let (chip, mut machine, mut session) = bench(board, Some(pad));
        assert_eq!(
            chip.analog_pins[pad].channel, channel,
            "{name}: {pad} must be channel {channel}"
        );
        assert_eq!(
            session.binding_errors(),
            &[] as &[RoutingError],
            "{name}: {pad} must bind"
        );
        let cycles = session.cycles_until_boundary(0);
        let (_, errors) = session
            .advance_to(cycles, &mut machine.bus)
            .unwrap_or_else(|error| panic!("solve the rerouted {name} decks: {error}"));
        assert!(errors.is_empty(), "{name}: route errors: {errors:?}");

        let diode_v = signal_voltage(&session, pad);
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "{name}: the Shockley load line on {pad} should be {expected_diode_v} V, got {diode_v} V"
        );
        let diode_code = expected_adc_count(diode_v);
        assert_eq!(
            convert(board, &mut machine, channel),
            diode_code,
            "{name}: channel {channel} ({pad}) must report the solved diode voltage"
        );
        assert_ne!(
            convert(board, &mut machine, chip.analog_pins["PA1"].channel),
            diode_code,
            "{name}: PA1 is no longer driven and must not report the diode voltage"
        );
    }
}

/// A pad with no input on the instantiated converter is refused at binding
/// time. On the G4 boards the refused pad is a real analog pad of ADC2 or
/// ADC5: the router must not put it on an ADC1 channel.
#[test]
fn a_pad_without_an_adc1_input_is_refused() {
    for board in BOARDS {
        let name = board.profile;
        let pad = board.refused_pad;
        let (chip, _machine, session) = bench(board, Some(pad));
        assert!(
            !chip.analog_pin_table().contains_key(pad),
            "{name}: {pad} must not be published as an ADC input"
        );
        let lower = pad.to_ascii_lowercase();
        assert_eq!(
            session.binding_errors(),
            &[RoutingError::NoAdcChannel {
                path: format!("board.analog.{lower}_volts"),
                pad: lower,
            }],
            "{name}: {pad} has no ADC1 input: the router must say so"
        );
    }
}
