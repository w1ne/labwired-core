// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! One generic deck pair drives four unrelated STM32F4 board profiles into
//! ADC1. The Beta NTC divider and the Shockley diode load line name only
//! `board.analog.<pad>_volts`; each profile's descriptor maps PA0 and PA1 to
//! `adc1` channels 0 and 1, and the shared STM32F4 ADC model performs the
//! conversion.
//!
//! The profiles differ in chip package, flash size, extra peripherals and
//! board I/O, but share the F4 ADC register interface (RM0368/RM0393/RM0383/
//! RM0090: ADC1 at 0x40012000, `RCC_APB2ENR` at 0x40023844, ADC1EN bit 8,
//! SWSTART bit 30). A single table selects the profile, deck and descriptor;
//! the same solve, routing and conversion assertions run for every entry.
//! This proves the mechanism is not board-specific: a new F4 profile needs a
//! descriptor map and an example deck, not new engine code.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::Path;

const RCC_BASE: u64 = 0x4002_3800;
const ADC1_BASE: u64 = 0x4001_2000;

/// RM0368 §6.3.13: RCC_APB2ENR.ADC1EN (bit 8).
const RCC_APB2ENR_ADC1EN: u32 = 1 << 8;
/// RM0368 §11.13.2: ADC_CR2.ADON (bit 0).
const F4_CR2_ADON: u32 = 1 << 0;
/// RM0368 §11.13.2: ADC_CR2.SWSTART (bit 30).
const F4_CR2_SWSTART: u32 = 1 << 30;
/// RM0368 §11.13.1: ADC_SR.EOC (bit 1).
const F4_SR_EOC: u32 = 1 << 1;

struct Board {
    profile: &'static str,
    system: &'static str,
    deck: &'static str,
    chip: &'static str,
}

const BOARDS: &[Board] = &[
    Board {
        profile: "nucleo-f407",
        system: "configs/systems/nucleo-f407.yaml",
        deck: "examples/nucleo-f407/analog-ntc-diode-system.yaml",
        chip: "STM32F407",
    },
    Board {
        profile: "nucleo-f411re",
        system: "configs/systems/nucleo-f411re.yaml",
        deck: "examples/nucleo-f411re/analog-ntc-diode-system.yaml",
        chip: "STM32F411",
    },
    Board {
        profile: "nucleo-f446re",
        system: "configs/systems/nucleo-f446re.yaml",
        deck: "examples/nucleo-f446re/analog-ntc-diode-system.yaml",
        chip: "STM32F446",
    },
    Board {
        profile: "feather-f405",
        system: "configs/systems/feather-f405.yaml",
        deck: "examples/feather-f405/analog-ntc-diode-system.yaml",
        chip: "STM32F405",
    },
];

/// Beta-parameter NTC resistance, the model card in every deck.
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
    let spin_pc = 0x0800_0040;
    bus.write_u16(spin_pc, 0xe7fe).expect("write b ."); // b .
    let (mut cpu, _) = configure_cortex_m(&mut bus);
    cpu.pc = spin_pc as u32;
    cpu.sp = 0x2000_5000;
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine
}

/// Perform one F4 software-triggered regular conversion the way firmware does:
/// ADON, then the SWSTART rising edge, then read SR before DR.
fn stm32f4_adc_convert(machine: &mut Machine<CortexM>, channel: u8) -> u32 {
    let bus = &mut machine.bus;
    let apb2enr = bus.read_u32(RCC_BASE + 0x44).expect("read RCC APB2ENR");
    bus.write_u32(RCC_BASE + 0x44, apb2enr | RCC_APB2ENR_ADC1EN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_BASE + 0x34, u32::from(channel))
        .expect("select SQR3 SQ1 channel");
    bus.write_u32(ADC1_BASE + 0x08, F4_CR2_ADON)
        .expect("power up ADC1");
    bus.write_u32(ADC1_BASE + 0x08, F4_CR2_ADON | F4_CR2_SWSTART)
        .expect("software-start the ADC1 conversion");
    for _ in 0..15 {
        machine.run(Some(1)).expect("advance the guest one cycle");
    }
    let bus = &mut machine.bus;
    let status = bus.read_u32(ADC1_BASE).expect("read ADC1 SR");
    assert_ne!(
        status & F4_SR_EOC,
        0,
        "ADC1 conversion completes with EOC set"
    );
    bus.read_u32(ADC1_BASE + 0x4c)
        .expect("read firmware-visible ADC1 DR")
}

/// The conversion the model reports: the routed pad voltage in whole
/// millivolts, scaled to the 12-bit range against the 3.3 V reference.
fn expected_adc_count(voltage_v: f64) -> u32 {
    let millivolts = (voltage_v * 1000.0).round();
    (millivolts * 4095.0 / 3300.0).floor().clamp(0.0, 4095.0) as u32
}

fn run_board(board: &Board) {
    let system_path = root(board.system);
    let mut profile = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load {} profile: {e}", board.profile));
    assert_eq!(profile.name, board.profile);

    let analog = SystemManifest::from_file(root(board.deck))
        .unwrap_or_else(|e| panic!("load generic decks for {}: {e}", board.profile));
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load {} descriptor: {e}", board.chip));
    assert_eq!(chip.analog_pins["PA0"].channel, 0, "{}", board.profile);
    assert_eq!(chip.analog_pins["PA1"].channel, 1, "{}", board.profile);
    assert_eq!(
        chip.analog_pins["PA0"].peripheral, "adc1",
        "{}",
        board.profile
    );
    assert_eq!(
        chip.analog_pins["PA1"].peripheral, "adc1",
        "{}",
        board.profile
    );

    let bus = SystemBus::from_config(&chip, &profile)
        .unwrap_or_else(|e| panic!("build {} bus: {e}", board.profile));
    let mut machine = board_machine(bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .unwrap_or_else(|e| panic!("build {} co-simulation: {e}", board.profile))
        .unwrap_or_else(|| panic!("{} deck carries both analog devices", board.profile));
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{} generic decks must resolve through PA0/PA1 to ADC1 channels 0/1",
        board.profile
    );
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut samples = Vec::new();
    for temperature_c in [-20.0_f64, 25.0, 85.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .unwrap_or_else(|e| panic!("set {} NTC temperature: {e}", board.profile));
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut machine.bus)
            .unwrap_or_else(|e| panic!("solve {} analog decks: {e}", board.profile));
        assert_eq!(
            steps.len(),
            2,
            "{}: both decks cross the 1 us boundary",
            board.profile
        );
        assert!(
            errors.is_empty(),
            "{}: unexpected route errors: {errors:?}",
            board.profile
        );

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "{} at {temperature_c} C: Beta divider should be {expected_ntc_v} V, got {ntc_v} V",
            board.profile
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "{}: Shockley load line should be {expected_diode_v} V, got {diode_v} V",
            board.profile
        );

        let ntc_count = stm32f4_adc_convert(&mut machine, 0);
        assert_eq!(
            ntc_count,
            expected_adc_count(ntc_v),
            "{}: ADC1 channel 0 must report the solved NTC voltage",
            board.profile
        );
        let diode_count = stm32f4_adc_convert(&mut machine, 1);
        assert_eq!(
            diode_count,
            expected_adc_count(diode_v),
            "{}: ADC1 channel 1 must report the solved diode voltage",
            board.profile
        );
        // An unrouted channel reports the code after the previous DR value, so
        // this assertion also proves the neighbour did not take a routed input.
        assert_eq!(
            stm32f4_adc_convert(&mut machine, 2),
            (diode_count + 1) & 0xfff,
            "{}: the undriven neighbouring channel uses its fallback",
            board.profile
        );
        samples.push((temperature_c, ntc_v, ntc_count));
    }

    assert!(
        samples[2].1 > samples[1].1 && samples[1].1 > samples[0].1,
        "{}: the divider voltage rises with NTC temperature: {samples:?}",
        board.profile
    );
    assert!(
        samples[2].2 > samples[1].2 && samples[1].2 > samples[0].2,
        "{}: ADC counts track the solved temperature-dependent voltage: {samples:?}",
        board.profile
    );
    assert_eq!(
        samples[1].2, 2047,
        "{}: the 25 C NTC divider is half scale on the 3.3 V reference",
        board.profile
    );
}

#[test]
fn stm32f4_family_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end() {
    for board in BOARDS {
        run_board(board);
    }
}
