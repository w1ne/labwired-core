// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The STM32G0B1RE pin table's `functions:` list reaches its ADC end to end.
//!
//! `nucleo-g0b1re` (NUCLEO-G0B1RE carrier) and `iolinki-g0-startup` (the
//! customer startup fixture) both load `stm32g0b1re.yaml`. That descriptor
//! transcribes the LQFP64 pinout but carried no ADC mapping, so neither board
//! could resolve `board.analog.<pad>_volts` even though the `stm32g0` ADC
//! engine has been in the core since the G0/C0 register map landed.
//!
//! This slice adds the ADC inputs to the pin table as
//! `{ type: adc, peripheral: adc1, channel: N }`; the analog router already
//! reads that union, so no `analog_pins:` duplicate and no board-specific
//! route is needed. The same generic NTC/diode deck that drives the L4, U5,
//! F1/F4/H7 and AVR parts loads here unchanged.
//!
//! What is new is the converter register map: the G0 has no `SQR1` and no
//! `DEEPPWD`. The regular group is the `CHSELR` bitmap at 0x28, `CR` resets to
//! 0, and `ADVREGEN` must precede `ADEN` before `ADSTART` converts into a
//! 12-bit `DR` at 0x40. A route that assumed the L4 sequencer would write
//! `SQR1` and never select a channel, so the conversion count, not just the
//! binding, is what proves the pad map.

use crate::common;
use common::root;
use labwired_config::ChipDescriptor;
use labwired_config::SystemManifest;
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::Path;

/// STM32G0 ADC1 register offsets (ST `stm32g0b1xx.h`).
const ADC_ISR: u64 = 0x00;
const ADC_CR: u64 = 0x08;
const ADC_CHSELR: u64 = 0x28;
const ADC_DR: u64 = 0x40;

/// `ISR.ADRDY` (bit 0), `ISR.EOC` (bit 2) and `ISR.EOS` (bit 3).
const ISR_ADRDY: u32 = 1 << 0;
const ISR_EOC: u32 = 1 << 2;
const ISR_EOS: u32 = 1 << 3;

/// `CR.ADEN` (bit 0), `CR.ADSTART` (bit 2) and `CR.ADVREGEN` (bit 28). The G0
/// CR resets to 0; it has no `DEEPPWD` bit.
const CR_ADEN: u32 = 1 << 0;
const CR_ADSTART: u32 = 1 << 2;
const CR_ADVREGEN: u32 = 1 << 28;

/// ADC1 base and the RCC APBENR2 gate (bit 20, `ADCEN`).
const ADC1_BASE: u64 = 0x4001_2400;
const RCC_BASE: u64 = 0x4002_1000;
const RCC_APBENR2: u64 = 0x40;
const RCC_ADCEN: u32 = 1 << 20;

const VREF_MV: f64 = 3_300.0;

struct Profile {
    /// The profile the assertion names when it fails.
    name: &'static str,
    system: &'static str,
    deck: &'static str,
    /// Every pad the deck publishes and the channel the descriptor must give it.
    pads: &'static [(&'static str, u8)],
}

const PROFILES: &[Profile] = &[
    Profile {
        name: "nucleo-g0b1re",
        system: "configs/systems/nucleo-g0b1re.yaml",
        deck: "examples/nucleo-g0b1re/analog-ntc-diode-pa0-pa1-system.yaml",
        pads: &[("PA0", 0), ("PA1", 1)],
    },
    // The startup fixture loads the same chip descriptor and needs no second
    // deck: the router resolves the pads from the chip, not the board name.
    Profile {
        name: "iolinki-g0-startup",
        system: "configs/systems/iolinki-g0-startup.yaml",
        deck: "examples/nucleo-g0b1re/analog-ntc-diode-pa0-pa1-system.yaml",
        pads: &[("PA0", 0), ("PA1", 1)],
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

/// The 12-bit code the model derives for a routed node.
fn expected_adc_count(voltage_v: f64) -> u32 {
    let millivolts = (voltage_v * 1000.0).round();
    (millivolts * 4095.0 / VREF_MV).floor().clamp(0.0, 4095.0) as u32
}

/// One software-triggered regular conversion on the G0 register map: clock
/// gate, `CHSELR` channel, regulator, enable, start, `EOC`/`EOS`, `DR`.
fn g0_convert(machine: &mut Machine<CortexM>, channel: u8) -> u32 {
    let bus = &mut machine.bus;
    let current = bus.read_u32(RCC_BASE + RCC_APBENR2).expect("read APBENR2");
    bus.write_u32(RCC_BASE + RCC_APBENR2, current | RCC_ADCEN)
        .expect("enable the ADC kernel clock");

    // The G0 regular group is a channel bitmap, not an L4 rank field.
    bus.write_u32(ADC1_BASE + ADC_CHSELR, 1 << channel)
        .expect("select the CHSELR channel");
    bus.write_u32(ADC1_BASE + ADC_CR, CR_ADVREGEN)
        .expect("start the regulator");
    bus.write_u32(ADC1_BASE + ADC_CR, CR_ADVREGEN | CR_ADEN)
        .expect("enable the converter");
    bus.write_u32(ADC1_BASE + ADC_CR, CR_ADVREGEN | CR_ADEN | CR_ADSTART)
        .expect("software-start the conversion");
    for _ in 0..4 {
        machine.run(Some(1)).expect("advance the guest one cycle");
    }
    let bus = &mut machine.bus;
    let status = bus.read_u32(ADC1_BASE + ADC_ISR).expect("read ADC ISR");
    assert_ne!(status & ISR_EOC, 0, "EOC must follow ADSTART");
    assert_ne!(status & ISR_EOS, 0, "one-channel sequence must reach EOS");
    assert_ne!(status & ISR_ADRDY, 0, "ADRDY must precede a conversion");
    bus.read_u32(ADC1_BASE + ADC_DR)
        .expect("read firmware-visible ADC DR")
}

fn run_profile(profile: &Profile) {
    let system_path = root(profile.system);
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load {} profile: {e}", profile.name));
    assert_eq!(
        manifest.name, profile.name,
        "{} must stay the profile its manifest declares",
        profile.system
    );

    let decks = SystemManifest::from_file(root(profile.deck))
        .unwrap_or_else(|e| panic!("load the generic deck for {}: {e}", profile.name));
    assert_eq!(
        decks.cosim_models.len(),
        profile.pads.len(),
        "{}: the deck must carry one model per published pad",
        profile.name
    );
    manifest.cosim_models = decks.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load {} descriptor: {e}", profile.name));

    // This part names its ADC inputs in `pins:` only. If a later edit
    // duplicates the map in `analog_pins:`, this test still proves the board
    // and the duplicate must not disagree.
    assert!(
        chip.analog_pins.is_empty(),
        "{}: this test covers descriptor pin functions; move it if `analog_pins:` returns",
        profile.name
    );
    assert_eq!(
        chip.analog_pin_conflicts(),
        Vec::new(),
        "{}: the two analog pad tables disagree",
        profile.name
    );
    let table = chip.analog_pin_table();
    assert!(
        !table.is_empty(),
        "{}: the pin table must name at least one ADC input",
        profile.name
    );
    for (pad, channel) in profile.pads {
        let pin = table
            .get(*pad)
            .unwrap_or_else(|| panic!("{} must map {pad} to an ADC input", profile.name));
        assert_eq!(
            (pin.peripheral.as_str(), pin.channel),
            ("adc1", *channel),
            "{}: {pad} must resolve to adc1 channel {channel}",
            profile.name
        );
    }

    let bus = SystemBus::from_config(&chip, &manifest)
        .unwrap_or_else(|e| panic!("build {} bus: {e}", profile.name));
    let mut machine = board_machine(bus);
    let mut session = CosimSession::new(&manifest.cosim_models, Path::new("."), &machine.bus)
        .unwrap_or_else(|e| panic!("build {} co-simulation: {e}", profile.name))
        .unwrap_or_else(|| panic!("{} deck carries its analog devices", profile.name));
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{} generic deck must resolve through the pin functions to ADC1",
        profile.name
    );

    let mut cycles = 0;
    let mut samples = Vec::new();
    for temperature_c in [-20.0_f64, 25.0, 85.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .unwrap_or_else(|e| panic!("set {} NTC temperature: {e}", profile.name));
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut machine.bus)
            .unwrap_or_else(|e| panic!("solve {} analog decks: {e}", profile.name));
        assert_eq!(
            steps.len(),
            profile.pads.len(),
            "{}: every deck crosses the 1 us boundary",
            profile.name
        );
        assert!(
            errors.is_empty(),
            "{}: unexpected route errors: {errors:?}",
            profile.name
        );

        let (ntc_pad, ntc_channel) = profile.pads[0];
        let ntc_v = signal_voltage(&session, ntc_pad);
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "{} at {temperature_c} C: Beta divider on {ntc_pad} should be {expected_ntc_v} V, got {ntc_v} V",
            profile.name
        );
        let ntc_count = g0_convert(&mut machine, ntc_channel);
        assert_eq!(
            ntc_count,
            expected_adc_count(ntc_v),
            "{}: a 12-bit conversion of channel {ntc_channel} must report the solved NTC voltage",
            profile.name
        );

        let (diode_pad, diode_channel) = profile.pads[1];
        let diode_v = signal_voltage(&session, diode_pad);
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "{}: Shockley load line on {diode_pad} should be {} V, got {diode_v} V",
            profile.name,
            diode_load_line_voltage()
        );
        assert_eq!(
            g0_convert(&mut machine, diode_channel),
            expected_adc_count(diode_v),
            "{}: a 12-bit conversion of channel {diode_channel} must report the solved diode voltage",
            profile.name
        );

        samples.push((temperature_c, ntc_v, ntc_count));
    }

    assert!(
        samples[2].1 > samples[1].1 && samples[1].1 > samples[0].1,
        "{}: the divider voltage rises with NTC temperature: {samples:?}",
        profile.name
    );
    assert!(
        samples[2].2 > samples[1].2 && samples[1].2 > samples[0].2,
        "{}: ADC counts track the solved temperature-dependent voltage: {samples:?}",
        profile.name
    );
    assert_eq!(
        samples[1].2,
        expected_adc_count(3.3 / 2.0),
        "{}: the 25 C NTC divider is half scale",
        profile.name
    );
}

#[test]
fn pin_table_adc_functions_reach_adc1_on_stm32g0b1_profiles_end_to_end() {
    for profile in PROFILES {
        run_profile(profile);
    }
}

/// A pad the pin table does not give an ADC function stays refused at binding
/// time: the union must not invent a channel for a digital pin.
#[test]
fn a_g0_pad_without_an_adc_pin_function_is_still_refused() {
    let system_path = root("configs/systems/nucleo-g0b1re.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load the nucleo-g0b1re profile");
    let deck = SystemManifest::from_file(root(
        "examples/nucleo-g0b1re/analog-ntc-diode-pa0-pa1-system.yaml",
    ))
    .expect("load the generic analog deck");
    let mut models = deck.cosim_models;
    models[0]
        .outputs
        .insert("v_sense".to_string(), "board.analog.pa8_volts".to_string());
    profile.cosim_models = models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the G0 descriptor");
    assert!(
        !chip.analog_pin_table().contains_key("PA8"),
        "PA8 is a digital pad; the map must not claim an ADC input"
    );

    let bus = SystemBus::from_config(&chip, &profile).expect("build the nucleo-g0b1re bus");
    let machine = board_machine(bus);
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build the co-simulation")
        .expect("deck carries its analog device");
    assert_eq!(
        session.binding_errors(),
        &[RoutingError::NoAdcChannel {
            path: "board.analog.pa8_volts".to_string(),
            pad: "pa8".to_string(),
        }],
        "PA8 is not an ADC input on this part: the router must say so"
    );
}
