// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The EK-RA6M5 and EK-RA8M1 boards solve the same generic Beta-parameter NTC
//! divider through their descriptor's ADC0 pad map. RA4M1 proved the RA
//! `ra_adc12` converter; this test proves the two EK boards are reachable from
//! a deck that names only `board.analog.<pad>_volts`.
//!
//! Renesas' FSP pin data and the boards' `adc0_default` pinctrl entry place
//! AN000 on P000 (RA6M5) and P004 (RA8M1); the existing DAC loopback fixtures
//! already wire DA0 to exactly those pads. Both descriptors now publish that
//! one verified input in `analog_pins:`. Nothing in the solver, router or
//! converter changed, so the test is per-part data on the generic mechanism.
//!
//! The conversion is the production `ra_adc12` single-scan path: select the
//! channel in ADANSA0, write ADST, wait for the model to clear it, read ADDR0.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::Path;

const ADCSR: u64 = 0x00;
const ADANSA0: u64 = 0x04;
const ADDR0: u64 = 0x20;
const ADST: u16 = 1 << 15;
const VREF_MV: f64 = 3_300.0;

struct Board {
    chip: &'static str,
    system: &'static str,
    deck: &'static str,
    pad: &'static str,
    adc0: u64,
    /// A branch-to-self in on-chip SRAM to park the core on.
    idle: u32,
}

const RA6M5: Board = Board {
    chip: "configs/chips/r7fa6m5bh.yaml",
    system: "configs/systems/ek-ra6m5.yaml",
    deck: "examples/ek-ra6m5/analog-ntc-system.yaml",
    pad: "P000",
    adc0: 0x4017_0000,
    idle: 0x2000_2000,
};

const RA8M1: Board = Board {
    chip: "configs/chips/r7fa8m1ah.yaml",
    system: "configs/systems/ek-ra8m1.yaml",
    deck: "examples/ek-ra8m1/analog-ntc-system.yaml",
    pad: "P004",
    adc0: 0x4033_2000,
    idle: 0x2200_2000,
};

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// Load the board, overlay the deck, and park a CPU on a branch-to-self.
fn bench(board: &Board) -> (ChipDescriptor, Machine<CortexM>, CosimSession) {
    let chip_path = root(board.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the RA descriptor");
    assert_eq!(chip.analog_pins[board.pad].peripheral, "adc0");
    assert_eq!(chip.analog_pins[board.pad].channel, 0);

    let system_path = root(board.system);
    let mut profile = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|error| panic!("load {}: {error}", board.system));
    let deck = SystemManifest::from_file(root(board.deck))
        .unwrap_or_else(|error| panic!("load {}: {error}", board.deck));
    profile.cosim_models = deck.cosim_models;

    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &profile)
        .unwrap_or_else(|error| panic!("build the bus for {}: {error}", board.system));
    bus.write_u16(u64::from(board.idle), 0xe7fe)
        .expect("write b .");
    let (mut cpu, _nvic) = configure_cortex_m(&mut bus);
    cpu.pc = board.idle;
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;

    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build analog co-simulation")
        .expect("fixture carries the NTC model");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{} must resolve {} to ADC0 channel 0 without naming a channel",
        board.system,
        board.pad
    );
    (chip, machine, session)
}

/// One software-started single scan of `channel`; the code it converts.
fn ra_adc_convert(machine: &mut Machine<CortexM>, adc0: u64, channel: u8) -> u16 {
    machine
        .bus
        .write_u16(adc0 + ADANSA0, 1u16 << channel)
        .expect("ADANSA0");
    machine.bus.write_u16(adc0 + ADCSR, ADST).expect("ADST");
    for _ in 0..10_000 {
        if machine.bus.read_u16(adc0 + ADCSR).expect("ADCSR") & ADST == 0 {
            return machine
                .bus
                .read_u16(adc0 + ADDR0 + 2 * u64::from(channel))
                .expect("ADDRn");
        }
        machine.run(Some(1)).expect("advance the guest one cycle");
    }
    panic!("ADC0 scan on channel {channel} never completed");
}

fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 4095 / VREF_MV as u32) as u16
}

fn board_solves_ntc_and_converts(board: &Board) {
    let (_chip, mut machine, mut session) = bench(board);
    let signal = format!("board.analog.{}_volts", board.pad.to_ascii_lowercase());
    let mut cycles = 0;
    let mut samples = Vec::new();

    for temperature_c in [-20.0_f64, 25.0, 85.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut machine.bus)
            .unwrap_or_else(|error| panic!("solve {}: {error}", board.system));
        assert_eq!(
            steps.len(),
            1,
            "{}: one netlist crosses the boundary",
            board.system
        );
        assert!(
            errors.is_empty(),
            "{}: unexpected route errors: {errors:?}",
            board.system
        );

        let ntc_v = signal_voltage(&session, &signal);
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "{}: at {temperature_c} C the Beta divider on {} should be {expected_ntc_v} V, got {ntc_v} V",
            board.system,
            board.pad
        );

        let code = ra_adc_convert(&mut machine, board.adc0, 0);
        assert_eq!(
            code,
            expected_adc_count(ntc_v),
            "{}: firmware-visible ADDR0 (AN000, {}) must report the solved NTC voltage",
            board.system,
            board.pad
        );
        samples.push((temperature_c, ntc_v, code));
    }

    assert!(
        samples[2].1 > samples[1].1 && samples[1].1 > samples[0].1,
        "{}: the divider voltage rises with NTC temperature: {samples:?}",
        board.system
    );
    assert!(
        samples[2].2 > samples[1].2 && samples[1].2 > samples[0].2,
        "{}: ADC counts track the solved temperature-dependent voltage: {samples:?}",
        board.system
    );
    assert_eq!(
        samples[1].2,
        expected_adc_count(3.3 / 2.0),
        "{}: the 25 C NTC divider is exactly half scale on the 3.3 V Vref",
        board.system
    );

    // AN001 is not published by the descriptor and no deck drives it; the
    // model's documented undriven level is 0 mV, so a zero here proves the
    // routed input did not bleed sideways.
    assert_eq!(
        ra_adc_convert(&mut machine, board.adc0, 1),
        0,
        "{}: an unrouted ADC0 channel must keep the model's 0 mV default",
        board.system
    );
}

#[test]
fn ek_ra6m5_ntc_divider_reaches_an000_and_adc0() {
    board_solves_ntc_and_converts(&RA6M5);
}

#[test]
fn ek_ra8m1_ntc_divider_reaches_an000_and_adc0() {
    board_solves_ntc_and_converts(&RA8M1);
}
