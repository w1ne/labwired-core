// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The pin table's `functions:` list is enough to reach an ADC: three parts
//! that transcribe their pinout but carry no `analog_pins:` block drive the
//! generic NTC and diode decks end to end.
//!
//! `nucleo-l432kc` (STM32L432KC, L4 ADC), `nucleo-u575zi` (STM32U575, U5 ADC)
//! and `nucleo-u5a5zj` (STM32U5A5ZJ, U5 ADC) each publish their ADC inputs as
//! `{ type: adc, peripheral: adc1, channel: N }` on the pad in `pins:`. Until
//! the router read that list, the same fact had to be typed a second time in
//! `analog_pins:`, and these three boards could not resolve
//! `board.analog.<pad>_volts` at all.
//!
//! The physics under test is the same generic pair the F1/F4/H7 sweeps use:
//! a Beta-parameter NTC divider at 25 C is half of the 3.3 V rail, and the
//! Shockley load line settles where the 1 kOhm limit resistor and the diode's
//! own series resistance balance. What is new here is the converter:
//!
//! * the L4 block needs `ADVREGEN` before `ADEN` raises `ADRDY` and converts
//!   into a 12-bit `DR`;
//! * the U5 block resets with `DEEPPWD` set and converts 14-bit, so the same
//!   1650 mV node must read 8188 counts, not 2047. A route that landed on a
//!   12-bit code path would be caught by the count, not by the binding.
//!
//! The pad → channel numbers (PA0 = IN5, PA1 = IN6) are the U5 datasheet
//! assignment, not the F1/F4 layout, and the test asserts them from the
//! descriptor so a transcription slip fails here.

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

/// ADC register offsets shared by the L4 and U5 blocks.
const ADC_ISR: u64 = 0x00;
const ADC_CR: u64 = 0x08;
const ADC_SQR1: u64 = 0x30;
const ADC_DR: u64 = 0x40;

/// `ISR.ADRDY` (bit 0) and `ISR.EOC` (bit 2) on both families.
const ISR_ADRDY: u32 = 1 << 0;
const ISR_EOC: u32 = 1 << 2;

/// `CR.ADEN`, `CR.ADSTART`, `CR.ADVREGEN` (bit 28) and `CR.DEEPPWD` (bit 29).
const CR_ADEN: u32 = 1 << 0;
const CR_ADSTART: u32 = 1 << 2;
const CR_ADVREGEN: u32 = 1 << 28;

/// The ADC reference these boards run at, and its 12-bit code for a node.
const VREF_MV: f64 = 3_300.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Family {
    /// STM32L4: ADC1 at 0x5004_0000, RCC at 0x4002_1000, AHB2ENR +0x4C bit 13,
    /// 12-bit `DR`.
    L4,
    /// STM32U5: ADC1 at 0x4202_8000, RCC at 0x4602_0C00, AHB2ENR1 +0x8C bit
    /// 10 (`ADC12EN`), 14-bit `DR`.
    U5,
}

impl Family {
    fn adc1_base(self) -> u64 {
        match self {
            Family::L4 => 0x5004_0000,
            Family::U5 => 0x4202_8000,
        }
    }

    fn rcc_base(self) -> u64 {
        match self {
            Family::L4 => 0x4002_1000,
            Family::U5 => 0x4602_0C00,
        }
    }

    fn adc_clock_offset(self) -> u64 {
        match self {
            Family::L4 => 0x4C, // RM0394 §6.4.17 AHB2ENR
            Family::U5 => 0x8C, // RM0456 §11.8.13 AHB2ENR1
        }
    }

    fn adc_clock_bit(self) -> u32 {
        match self {
            Family::L4 => 1 << 13, // ADCEN
            Family::U5 => 1 << 10, // ADC12EN
        }
    }

    /// Conversion width the reset value of `CFGR`/`CFGR1` selects.
    fn bits(self) -> u32 {
        match self {
            Family::L4 => 12,
            Family::U5 => 14, // RES[3:2] = 00 encodes 14 bits on the U5
        }
    }
}

struct Profile {
    /// The profile the assertion names when it fails.
    name: &'static str,
    system: &'static str,
    deck: &'static str,
    family: Family,
    /// Every pad the deck publishes and the channel the descriptor must give it.
    pads: &'static [(&'static str, u8)],
}

const PROFILES: &[Profile] = &[
    // The NUCLEO-32 part bonds one ADC input; the deck publishes one pad.
    Profile {
        name: "nucleo-l432kc",
        system: "configs/systems/nucleo-l432kc.yaml",
        deck: "examples/nucleo-l432kc/analog-ntc-pa0-system.yaml",
        family: Family::L4,
        pads: &[("PA0", 5)],
    },
    Profile {
        name: "nucleo-u575zi",
        system: "configs/systems/nucleo-u575zi.yaml",
        deck: "examples/nucleo-u575zi/analog-ntc-diode-pa0-pa1-system.yaml",
        family: Family::U5,
        pads: &[("PA0", 5), ("PA1", 6)],
    },
    // Same U5 pad set; the deck names no part, so it loads unchanged.
    Profile {
        name: "nucleo-u5a5zj",
        system: "configs/systems/nucleo-u5a5zj.yaml",
        deck: "examples/nucleo-u575zi/analog-ntc-diode-pa0-pa1-system.yaml",
        family: Family::U5,
        pads: &[("PA0", 5), ("PA1", 6)],
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

/// The 12-bit code the model derives for a routed node, then widened to the
/// converter's own width.
fn expected_adc_count(voltage_v: f64, bits: u32) -> u32 {
    let millivolts = (voltage_v * 1000.0).round();
    let count12 = (millivolts * 4095.0 / VREF_MV).floor().clamp(0.0, 4095.0) as u32;
    if bits >= 12 {
        (count12 << (bits - 12)) & ((1 << bits) - 1)
    } else {
        count12 >> (12 - bits)
    }
}

/// One software-triggered regular conversion on the family's register map:
/// clock gate, `SQR1.SQ1`, regulator, enable, start, `EOC`, `DR`.
fn adc1_convert(machine: &mut Machine<CortexM>, family: Family, channel: u8) -> u32 {
    let rcc = family.rcc_base() + family.adc_clock_offset();
    let adc1 = family.adc1_base();
    let bus = &mut machine.bus;
    let current = bus.read_u32(rcc).expect("read the RCC enable register");
    bus.write_u32(rcc, current | family.adc_clock_bit())
        .expect("enable the ADC kernel clock");

    bus.write_u32(adc1 + ADC_SQR1, u32::from(channel) << 6)
        .expect("select SQR1 SQ1 channel");
    // Power-up order both families impose: clear DEEPPWD and raise ADVREGEN,
    // then ADEN (ADRDY), then ADSTART. The L4's `rs` bits keep ADEN across a
    // write that carries zero, the U5 stores the word as written.
    bus.write_u32(adc1 + ADC_CR, CR_ADVREGEN)
        .expect("leave deep power-down and start the regulator");
    bus.write_u32(adc1 + ADC_CR, CR_ADVREGEN | CR_ADEN)
        .expect("enable the converter");
    bus.write_u32(adc1 + ADC_CR, CR_ADVREGEN | CR_ADEN | CR_ADSTART)
        .expect("software-start the conversion");
    for _ in 0..40 {
        machine.run(Some(1)).expect("advance the guest one cycle");
    }
    let bus = &mut machine.bus;
    let status = bus.read_u32(adc1 + ADC_ISR).expect("read ADC ISR");
    assert_ne!(status & ISR_EOC, 0, "{:?}: EOC must follow ADSTART", family);
    assert_ne!(
        status & ISR_ADRDY,
        0,
        "{:?}: ADRDY must be set before a conversion starts",
        family
    );
    bus.read_u32(adc1 + ADC_DR)
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

    // These parts name their ADC inputs in `pins:` only. If a later edit
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

    let expected_diode_v = if profile.pads.len() > 1 {
        Some(diode_load_line_voltage())
    } else {
        None
    };
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
        let ntc_count = adc1_convert(&mut machine, profile.family, ntc_channel);
        assert_eq!(
            ntc_count,
            expected_adc_count(ntc_v, profile.family.bits()),
            "{}: a {}-bit conversion of channel {ntc_channel} must report the solved NTC voltage",
            profile.name,
            profile.family.bits()
        );

        if let (Some(expected_diode_v), Some((diode_pad, diode_channel))) =
            (expected_diode_v, profile.pads.get(1))
        {
            let diode_v = signal_voltage(&session, diode_pad);
            assert!(
                (diode_v - expected_diode_v).abs() < 2e-5,
                "{}: Shockley load line on {diode_pad} should be {expected_diode_v} V, got {diode_v} V",
                profile.name
            );
            assert_eq!(
                adc1_convert(&mut machine, profile.family, *diode_channel),
                expected_adc_count(diode_v, profile.family.bits()),
                "{}: a {}-bit conversion of channel {diode_channel} must report the solved diode voltage",
                profile.name,
                profile.family.bits()
            );
        }
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
        expected_adc_count(3.3 / 2.0, profile.family.bits()),
        "{}: the 25 C NTC divider is half scale, widened to the converter's {} bits",
        profile.name,
        profile.family.bits()
    );
}

#[test]
fn pin_table_adc_functions_reach_adc1_on_l4_and_u5_profiles_end_to_end() {
    for profile in PROFILES {
        run_profile(profile);
    }
}

/// A pad the pin table does not give an ADC function stays refused at binding
/// time: the union must not invent a channel for a digital pin.
#[test]
fn a_pad_without_an_adc_pin_function_is_still_refused() {
    let system_path = root("configs/systems/nucleo-l432kc.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load the nucleo-l432kc profile");
    let deck = SystemManifest::from_file(root("examples/nucleo-l432kc/analog-ntc-pa0-system.yaml"))
        .expect("load the generic analog deck");
    let mut models = deck.cosim_models;
    models[0]
        .outputs
        .insert("v_sense".to_string(), "board.analog.pa1_volts".to_string());
    profile.cosim_models = models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the chip descriptor");
    assert!(
        !chip.analog_pin_table().contains_key("PA1"),
        "PA1 on this part is not an ADC input, so the union must leave it out"
    );
    let bus = SystemBus::from_config(&chip, &profile).expect("build the board bus");
    let machine = board_machine(bus);
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build the analog co-simulation")
        .expect("the deck carries an analog device");
    assert_eq!(
        session.binding_errors(),
        &[RoutingError::NoAdcChannel {
            path: "board.analog.pa1_volts".to_string(),
            pad: "pa1".to_string(),
        }],
        "PA1 is not an ADC input on this part: the router must say so"
    );
}

/// No descriptor in the tree may answer "which channel samples this pad?" two
/// ways. `analog_pins:` wins when both tables name a pad, so a silent
/// disagreement would convert a node on a channel the datasheet does not give.
#[test]
fn no_chip_descriptor_assigns_a_pad_two_different_adc_channels() {
    let mut conflicts = Vec::new();
    for entry in std::fs::read_dir(root("configs/chips")).expect("list the chip descriptors") {
        let path = entry.expect("read a chip entry").path();
        if path.extension().is_some_and(|ext| ext == "yaml") {
            let Ok(chip) = ChipDescriptor::from_file(&path) else {
                continue;
            };
            for (pad, explicit, derived) in chip.analog_pin_conflicts() {
                conflicts.push(format!(
                    "{}: {pad} is {}:{} in `analog_pins:` and {}:{} in `pins:`",
                    path.display(),
                    explicit.peripheral,
                    explicit.channel,
                    derived.peripheral,
                    derived.channel
                ));
            }
        }
    }
    assert!(
        conflicts.is_empty(),
        "chip descriptors disagree with themselves:\n{}",
        conflicts.join("\n")
    );
}

/// Every profile whose chip publishes ADC inputs through its pin table is in
/// the sweep above. A new such part in `configs/systems/` cannot be left
/// uncovered by adding a chip descriptor alone.
#[test]
fn sweep_covers_every_profile_that_reaches_an_adc_through_pin_functions() {
    let mut found = Vec::new();
    for directory in ["configs/systems", "configs/systems/onboarding"] {
        let Ok(entries) = std::fs::read_dir(root(directory)) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("read manifest entry").path();
            if path.extension().is_none_or(|ext| ext != "yaml") {
                continue;
            }
            let Ok(manifest) = SystemManifest::from_file(&path) else {
                continue;
            };
            if PROFILES.iter().any(|profile| profile.name == manifest.name) {
                continue;
            }
            let chip_path = path.parent().unwrap().join(&manifest.chip);
            let Ok(chip) = ChipDescriptor::from_file(&chip_path) else {
                continue;
            };
            if chip.analog_pins.is_empty() && !chip.analog_pin_table().is_empty() {
                found.push(manifest.name);
            }
        }
    }
    assert!(
        found.is_empty(),
        "these profiles publish ADC inputs through their pin table but no analog test loads them: {found:?}"
    );
}
