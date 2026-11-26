// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The NUCLEO-L476RG solves the same generic NTC divider and Shockley diode
//! load line as every other covered board, through the STM32L476 descriptor's
//! ADC pad map. The deck names only `board.analog.pa0_volts` /
//! `board.analog.pa1_volts`; nothing in the solver, router or converter is
//! L476-specific.
//!
//! The map is the chip's full LQFP64 ADC1 input set from RM0351 Table 18
//! (PC0-PC3 = IN1-IN4, PA0-PA7 = IN5-IN12, PC4/PC5 = IN13/IN14,
//! PB0/PB1 = IN15/IN16), so it serves every board on this die. The
//! NUCLEO-L476RG Arduino header (Zephyr's connector DTS: A0-A5 = PA0, PA1,
//! PA4, PB0, PC1, PC0) is a subset, and the board DTS enables `adc1_in1_pc0`,
//! which cross-checks the PC0 → channel 1 entry.
//!
//! The conversion is the production L4 block: clock gate in `AHB2ENR`, select
//! `SQR1.SQ1`, raise `ADVREGEN` then `ADEN`/`ADRDY`, start, read a 12-bit
//! `DR`. A route that landed on another family's register layout would fail
//! the conversion, not just the binding.
//!
//! `iolink-multiport-demo` is a second board on the same die. It reads an
//! unrelated device (a 24 V IO-Link L+ supply monitor) plus the NTC divider on
//! PC0/PC1 through the same map, with no board-specific code.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::Path;

const ADC1_BASE: u64 = 0x5004_0000;
/// RM0351 §6.4.17: RCC_AHB2ENR, ADCEN bit 13.
const RCC_AHB2ENR: u64 = 0x4002_1000 + 0x4C;
const RCC_ADCEN: u32 = 1 << 13;

const ADC_ISR: u64 = 0x00;
const ADC_CR: u64 = 0x08;
const ADC_SQR1: u64 = 0x30;
const ADC_DR: u64 = 0x40;

const ISR_ADRDY: u32 = 1 << 0;
const ISR_EOC: u32 = 1 << 2;
const CR_ADEN: u32 = 1 << 0;
const CR_ADSTART: u32 = 1 << 2;
const CR_ADVREGEN: u32 = 1 << 28;

const VREF_MV: f64 = 3_300.0;
const SUPPLY_V: f64 = 3.3;
const LIMIT_OHMS: f64 = 1_000.0;
const DIODE_IS_A: f64 = 2.52e-9;
const DIODE_N: f64 = 1.752;
const DIODE_RS_OHMS: f64 = 0.568;
const THERMAL_V: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

/// Every LQFP64 ADC1 regular input and its RM0351 Table 18 channel.
const PADS: [(&str, u8); 16] = [
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
    ("PB0", 15),
    ("PB1", 16),
];

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line at the board's 3.3 V rail, including the
/// diode's series resistance.
fn diode_load_line_voltage() -> f64 {
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
    cpu.sp = 0x2000_5000;
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

/// One software-triggered regular conversion on the L4 register map: clock
/// gate, `SQR1.SQ1`, regulator, enable, start, `EOC`, `DR`.
fn adc1_convert(machine: &mut Machine<CortexM>, channel: u8) -> u32 {
    let bus = &mut machine.bus;
    let current = bus.read_u32(RCC_AHB2ENR).expect("read RCC_AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, current | RCC_ADCEN)
        .expect("enable the ADC kernel clock");

    bus.write_u32(ADC1_BASE + ADC_SQR1, u32::from(channel) << 6)
        .expect("select SQR1 SQ1 channel");
    // The L4 power-up order: raise ADVREGEN, then ADEN (ADRDY), then ADSTART.
    bus.write_u32(ADC1_BASE + ADC_CR, CR_ADVREGEN)
        .expect("start the ADC regulator");
    bus.write_u32(ADC1_BASE + ADC_CR, CR_ADVREGEN | CR_ADEN)
        .expect("enable the converter");
    bus.write_u32(ADC1_BASE + ADC_CR, CR_ADVREGEN | CR_ADEN | CR_ADSTART)
        .expect("software-start the conversion");
    for _ in 0..40 {
        machine.run(Some(1)).expect("advance the guest one cycle");
    }
    let bus = &mut machine.bus;
    let status = bus.read_u32(ADC1_BASE + ADC_ISR).expect("read ADC ISR");
    assert_ne!(status & ISR_ADRDY, 0, "ADRDY must be set before EOC");
    assert_ne!(status & ISR_EOC, 0, "EOC must follow ADSTART");
    bus.read_u32(ADC1_BASE + ADC_DR)
        .expect("read firmware-visible ADC DR")
}

/// Load `profile` (a `configs/systems` name), replace its co-simulation with
/// the example `deck`, prove the chip map, and bind the deck to the board bus.
fn board_bench(
    profile_name: &str,
    deck: &str,
    reroute: Option<&str>,
) -> (Machine<CortexM>, CosimSession, Vec<RoutingError>) {
    let system_path = root(&format!("configs/systems/{profile_name}.yaml"));
    let mut profile = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|error| panic!("load {profile_name} profile: {error}"));
    assert_eq!(profile.name, profile_name);
    let mut models = SystemManifest::from_file(root(deck))
        .unwrap_or_else(|error| panic!("load {deck}: {error}"))
        .cosim_models;
    if let Some(pad) = reroute {
        models[0].outputs.insert(
            "v_sense".to_string(),
            format!("board.analog.{}_volts", pad.to_ascii_lowercase()),
        );
    }
    profile.cosim_models = models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the STM32L476 descriptor");
    for (pad, channel) in PADS {
        assert_eq!(
            chip.analog_pins[pad].peripheral, "adc1",
            "{pad} must resolve to ADC1"
        );
        assert_eq!(
            chip.analog_pins[pad].channel, channel,
            "{pad} is ADC1 channel {channel} (RM0351 Table 18)"
        );
    }
    assert_eq!(
        chip.analog_pins.len(),
        PADS.len(),
        "the chip map publishes exactly the sixteen LQFP64 ADC1 inputs"
    );
    assert_eq!(
        chip.analog_pin_conflicts(),
        Vec::new(),
        "the pin table and the analog pad map must agree"
    );

    let bus = SystemBus::from_config(&chip, &profile)
        .unwrap_or_else(|error| panic!("build the {profile_name} bus: {error}"));
    let machine = board_machine(bus);
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build analog co-simulation")
        .expect("deck carries the selected analog devices");
    let errors = session.binding_errors().to_vec();
    (machine, session, errors)
}

const NUCLEO_DECK: &str = "examples/nucleo-l476rg/analog-ntc-diode-pa0-pa1-system.yaml";

fn bench() -> (Machine<CortexM>, CosimSession) {
    let (machine, session, errors) = board_bench("nucleo-l476rg", NUCLEO_DECK, None);
    assert_eq!(
        errors,
        Vec::new(),
        "PA0/PA1 must resolve through the board pad map without naming a channel"
    );
    (machine, session)
}

#[test]
fn nucleo_l476rg_solves_generic_decks_and_converts_through_adc1() {
    let expected_diode_v = diode_load_line_voltage();
    let (mut machine, mut session) = bench();
    let mut cycles = 0;
    let mut samples = Vec::new();

    for temperature_c in [-20.0_f64, 25.0, 85.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut machine.bus)
            .unwrap_or_else(|error| panic!("solve the nucleo-l476rg decks: {error}"));
        assert_eq!(steps.len(), 2, "both netlists cross the boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "PA0");
        let expected_ntc_v = SUPPLY_V * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} C the Beta divider on PA0 should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "PA1");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line on PA1 should be {expected_diode_v} V, got {diode_v} V"
        );

        let ntc_code = adc1_convert(&mut machine, 5);
        let diode_code = adc1_convert(&mut machine, 6);
        assert_eq!(
            ntc_code,
            expected_adc_count(ntc_v),
            "ADC1 channel 5 (PA0) must report the solved NTC voltage"
        );
        assert_eq!(
            diode_code,
            expected_adc_count(diode_v),
            "ADC1 channel 6 (PA1) must report the solved diode voltage"
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
        expected_adc_count(SUPPLY_V / 2.0),
        "the 25 C NTC divider is exactly half scale on 3.3 V Vref"
    );
}

/// Build a session whose NTC divider output is routed to `pad` instead of
/// PA0, and return any binding errors with the bench.
fn rerouted_bench(pad: &str) -> (Machine<CortexM>, CosimSession, Vec<RoutingError>) {
    board_bench("nucleo-l476rg", NUCLEO_DECK, Some(pad))
}

/// PA3 is off the Arduino header (the board wires it to the ST-LINK VCP), but
/// the silicon has ADC1_IN8 there. The chip-level map must route it, so a
/// different board on this die can use the pad. A board-specific map would
/// refuse it and fail here.
#[test]
fn stm32l476_routes_an_off_header_adc_pad() {
    let (mut machine, mut session, errors) = rerouted_bench("PA3");
    assert_eq!(errors, Vec::new(), "PA3 is ADC1_IN8 and must bind");

    session
        .set_signal_number("ui.ntc.temperature_c", 85.0)
        .expect("set generic NTC temperature");
    let cycles = session.cycles_until_boundary(0);
    session
        .advance_to(cycles, &mut machine.bus)
        .unwrap_or_else(|error| panic!("solve the rerouted deck: {error}"));
    let ntc_v = signal_voltage(&session, "PA3");
    let expected_ntc_v = SUPPLY_V * 10_000.0 / (ntc_resistance_ohms(85.0) + 10_000.0);
    assert!(
        (ntc_v - expected_ntc_v).abs() < 1e-9,
        "the Beta divider on PA3 should be {expected_ntc_v} V, got {ntc_v} V"
    );
    assert_eq!(
        adc1_convert(&mut machine, 8),
        expected_adc_count(ntc_v),
        "ADC1 channel 8 (PA3) must report the solved NTC voltage"
    );
}

/// PA9 (USART1_TX) has no ADC input on the STM32L476. A deck that names it
/// must be refused at binding time instead of reading a channel nobody routed.
#[test]
fn nucleo_l476rg_router_refuses_a_non_adc_pad() {
    let (_machine, _session, errors) = rerouted_bench("PA9");
    assert_eq!(
        errors,
        vec![RoutingError::NoAdcChannel {
            path: "board.analog.pa9_volts".to_string(),
            pad: "pa9".to_string(),
        }],
        "PA9 is not an ADC input: the router must say so"
    );
}

/// Divider ratio of the IO-Link L+ monitor (100k over 12k).
const LPLUS_RATIO: f64 = 12_000.0 / 112_000.0;

/// The second board on this die, `iolink-multiport-demo`, reads a different
/// device (a 24 V L+ supply monitor) and the generic NTC divider on PC0/PC1,
/// pads the NUCLEO deck never uses. The same chip map, router and L4 ADC
/// serve it with the board's IO-Link master still attached to USART1. The
/// L+ sweep covers the IEC 61131-9 supply range (18 V to 30 V), so a wrong
/// source binding, divider or channel moves a code.
#[test]
fn iolink_multiport_demo_monitors_lplus_and_ntc_through_adc1() {
    let (mut machine, mut session, errors) = board_bench(
        "iolink-multiport-demo",
        "examples/iolink-multiport-demo/analog-lplus-ntc-system.yaml",
        None,
    );
    assert_eq!(
        errors,
        Vec::new(),
        "PC0/PC1 must resolve through the chip pad map without naming a channel"
    );

    let mut cycles = 0;
    let mut codes = Vec::new();
    for (l_plus_v, temperature_c) in [(18.0_f64, -20.0_f64), (24.0, 25.0), (30.0, 85.0)] {
        session
            .set_signal_number("ui.iolink.l_plus_volts", l_plus_v)
            .expect("set the L+ rail");
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut machine.bus)
            .unwrap_or_else(|error| panic!("solve the iolink-multiport-demo decks: {error}"));
        assert_eq!(steps.len(), 2, "both netlists cross the boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let lplus_sense_v = signal_voltage(&session, "PC0");
        assert!(
            (lplus_sense_v - l_plus_v * LPLUS_RATIO).abs() < 1e-9,
            "L+ = {l_plus_v} V must divide to {} V on PC0, got {lplus_sense_v} V",
            l_plus_v * LPLUS_RATIO
        );
        let ntc_v = signal_voltage(&session, "PC1");
        let expected_ntc_v = SUPPLY_V * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} C the Beta divider on PC1 should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let lplus_code = adc1_convert(&mut machine, 1);
        let ntc_code = adc1_convert(&mut machine, 2);
        assert_eq!(
            lplus_code,
            expected_adc_count(lplus_sense_v),
            "ADC1 channel 1 (PC0) must report the divided L+ rail"
        );
        assert_eq!(
            ntc_code,
            expected_adc_count(ntc_v),
            "ADC1 channel 2 (PC1) must report the solved NTC voltage"
        );
        codes.push((l_plus_v, lplus_code, ntc_code));
    }

    // 24 V divides to 2.5714 V: 2571 mV * 4095 / 3300 = 3190.
    assert_eq!(codes[1].1, 3190, "nominal 24 V L+ code: {codes:?}");
    assert!(
        codes[2].1 > codes[1].1 && codes[1].1 > codes[0].1,
        "L+ codes rise with the rail: {codes:?}"
    );
    assert!(
        codes[2].2 > codes[1].2 && codes[1].2 > codes[0].2,
        "NTC codes rise with temperature: {codes:?}"
    );
}
