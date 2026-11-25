// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider reaches the STM32H7 converter for the first time on
//! a board: five H7 part profiles (`stm32h7`, `stm32h743`, `stm32h747`,
//! `stm32h747-disco`, `stm32h753`) that declare exactly one analog pad,
//! PA0 = ADC1_INP16.
//!
//! This is the first end-to-end H7 analog path in the tree. The register model
//! (`AdcRegisterLayout::Stm32H7`) existed and was unit-tested, but no board
//! test drove it, so nothing proved that a solved netlist reaches an H7
//! `DR`. Three H7-only facts are exercised here and none of them are F4
//! behaviour with a different address:
//!
//! * `CR.DEEPPWD` resets SET (RM0433 §25.4.4): firmware must clear it and set
//!   `ADVREGEN`, then wait for `ISR.LDORDY`, before `ADEN` is even accepted.
//! * The converter is 16-bit. A 1650 mV NTC midpoint must read about 32752
//!   counts, not 2047: a 12-bit code would be a routing bug the test catches.
//! * The input is selected by `SQR1.SQ1` and gated by `PCSEL[16]`, the channel
//!   bitmap the F4 does not have. `PA0` is `INP16`, not `IN0`.
//!
//! The deck is the NTC half of the same generic pair the F1/F4 sweep uses; the
//! diode half publishes PA1, which none of these parts bond to an ADC input,
//! so it is left out rather than silently mis-routed.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::Path;

/// RM0433 §9.7.8: RCC_AHB1ENR offset from the H7 RCC base.
const RCC_AHB1ENR_OFFSET: u64 = 0xD8;
/// RM0433 §9.7.8: RCC_AHB1ENR.ADC12EN (bit 5) gates ADC1 and ADC2.
const AHB1ENR_ADC12EN: u32 = 1 << 5;

const ADC_ISR: u64 = 0x00;
const ADC_CR: u64 = 0x08;
const ADC_CFGR: u64 = 0x0C;
const ADC_PCSEL: u64 = 0x1C;
const ADC_SQR1: u64 = 0x30;
const ADC_DR: u64 = 0x40;

/// RM0433 §25.4.4: `CR.DEEPPWD` (bit 29) resets set and blocks everything
/// until firmware clears it.
const CR_DEEPPWD: u32 = 1 << 29;
/// `CR.ADVREGEN` (bit 28): the internal regulator must be up before ADEN.
const CR_ADVREGEN: u32 = 1 << 28;
/// `CR.ADCAL` (bit 31): self-clearing calibration command.
const CR_ADCAL: u32 = 1 << 31;
/// `CR.ADEN` (bit 0).
const CR_ADEN: u32 = 1 << 0;
/// `CR.ADSTART` (bit 2): start one regular conversion.
const CR_ADSTART: u32 = 1 << 2;
/// `ISR.ADRDY` (bit 0) and `ISR.LDORDY` (bit 12).
const ISR_ADRDY: u32 = 1;
const ISR_LDORDY: u32 = 1 << 12;
/// `ISR.EOC` (bit 2).
const ISR_EOC: u32 = 1 << 2;
/// `CFGR.JQDIS` (bit 31) resets set; leaving it set is what firmware does when
/// it writes `RES` and a trigger mode.
const CFGR_JQDIS: u32 = 1 << 31;

/// `CFGR.RES = 0b000` selects the native 16-bit width (RM0433 §25.4.7).
const CFGR_RES_16_BIT: u32 = 0b000 << 2;

/// The H7 converter's full scale, for the physics bound.
const H7_FULL_SCALE: f64 = 65_535.0;
/// The model's reference, the 3.3 V VDDA rail these parts run at.
const VREF_MV: f64 = 3_300.0;

/// The pad these parts bond to an ADC input (DS12110: PA0 = ADC1_INP16).
const PAD: &str = "PA0";
const CHANNEL: u8 = 16;
/// A channel no pad on these parts carries, for the bleed check.
const IDLE_CHANNEL: u8 = 15;

struct Profile {
    name: &'static str,
    system: &'static str,
}

const PROFILES: &[Profile] = &[
    Profile {
        name: "stm32h7",
        system: "configs/systems/onboarding/stm32h7.yaml",
    },
    Profile {
        name: "stm32h743",
        system: "configs/systems/onboarding/stm32h743.yaml",
    },
    Profile {
        name: "stm32h747",
        system: "configs/systems/onboarding/stm32h747.yaml",
    },
    Profile {
        name: "stm32h747-disco",
        system: "configs/systems/onboarding/stm32h747-disco.yaml",
    },
    Profile {
        name: "stm32h753",
        system: "configs/systems/onboarding/stm32h753.yaml",
    },
];

/// Beta-parameter NTC resistance, the model card in the shared deck.
fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// The count the model reports: the routed level becomes a 12-bit reference
/// count in whole millivolts and is widened to the configured 16-bit width.
/// The widening is the model's documented pipeline; [`ideal_16_bit_count`] is
/// the physical value the same voltage would give on silicon, and the test
/// bounds the difference rather than pretending they are identical.
fn expected_code(volts: f64) -> u32 {
    let millivolts = (volts * 1000.0).round();
    let count12 = (millivolts * 4095.0 / VREF_MV).floor().clamp(0.0, 4095.0) as u32;
    count12 << 4
}

/// What a 16-bit SAR reports for `volts` against the 3.3 V reference.
fn ideal_16_bit_count(volts: f64) -> f64 {
    volts * H7_FULL_SCALE / 3.3
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

fn peripheral_base(chip: &ChipDescriptor, id: &str) -> u64 {
    chip.peripherals
        .iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("the descriptor must declare {id}"))
        .base_address
}

/// Power the converter the way firmware does: gate its clock, then leave deep
/// power-down and start the regulator. Returns the `CR` reset state observed
/// after the clock is on, which is what proves `DEEPPWD` really resets set.
fn h7_power_up(machine: &mut Machine<CortexM>, chip: &ChipDescriptor) -> u32 {
    let rcc = peripheral_base(chip, "rcc");
    let adc = peripheral_base(chip, "adc1");
    let bus = &mut machine.bus;
    let ahb1enr = bus
        .read_u32(rcc + RCC_AHB1ENR_OFFSET)
        .expect("read AHB1ENR");
    bus.write_u32(rcc + RCC_AHB1ENR_OFFSET, ahb1enr | AHB1ENR_ADC12EN)
        .expect("enable the ADC12 clock");
    // Until the gate opens every read of the block is the bus's zero, so the
    // reset value can only be observed after the write above.
    let cr = bus.read_u32(adc + ADC_CR).expect("read CR");
    bus.write_u32(adc + ADC_CR, (cr & !CR_DEEPPWD) | CR_ADVREGEN)
        .expect("leave deep power-down and start the regulator");
    assert_ne!(
        bus.read_u32(adc + ADC_ISR).expect("read ISR") & ISR_LDORDY,
        0,
        "the regulator must report LDORDY before a calibration"
    );
    cr
}

/// One H7 regular conversion with the bring-up order firmware uses:
/// calibration, `ADEN`, channel preselection, `SQ1`, `RES`, then `ADSTART`.
/// Every `CR` write is a read-modify-write because `CR` mixes `rs` command
/// bits with latched configuration (RM0433 §25.4.4): writing a bare command
/// word would drop `ADVREGEN` and power the converter back down.
fn h7_convert(machine: &mut Machine<CortexM>, chip: &ChipDescriptor, channel: u8) -> u32 {
    let adc = peripheral_base(chip, "adc1");

    {
        let bus = &mut machine.bus;
        let cr = bus.read_u32(adc + ADC_CR).expect("read CR");
        assert_ne!(
            cr & CR_ADVREGEN,
            0,
            "the regulator must still be up from power-up"
        );
        assert_eq!(
            cr & CR_DEEPPWD,
            0,
            "power-up must have left deep power-down"
        );
        bus.write_u32(adc + ADC_CR, cr | CR_ADCAL)
            .expect("calibrate");
        let cr = bus.read_u32(adc + ADC_CR).expect("read CR");
        bus.write_u32(adc + ADC_CR, cr | CR_ADEN).expect("enable");
        assert_ne!(
            bus.read_u32(adc + ADC_ISR).expect("read ISR") & ISR_ADRDY,
            0,
            "ADEN must raise ADRDY once the regulator is up"
        );

        bus.write_u32(adc + ADC_CFGR, CFGR_JQDIS | CFGR_RES_16_BIT)
            .expect("select the 16-bit width");
        bus.write_u32(adc + ADC_PCSEL, 1 << channel)
            .expect("preselect the channel");
        bus.write_u32(adc + ADC_SQR1, u32::from(channel) << 6)
            .expect("select SQ1");
        let cr = bus.read_u32(adc + ADC_CR).expect("read CR");
        bus.write_u32(adc + ADC_CR, cr | CR_ADSTART)
            .expect("start one regular conversion");
    }

    for _ in 0..16 {
        machine.run(Some(1)).expect("advance the guest one cycle");
    }
    let bus = &mut machine.bus;
    assert_ne!(
        bus.read_u32(adc + ADC_ISR).expect("read ISR") & ISR_EOC,
        0,
        "the conversion completes with EOC set"
    );
    bus.read_u32(adc + ADC_DR).expect("read DR") & 0xFFFF
}

fn run_profile(profile: &Profile) {
    let system_path = root(profile.system);
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load {} profile: {e}", profile.name));
    assert_eq!(manifest.name, profile.name);

    // The NTC half of the generic pair. Its output already names
    // `board.analog.<pad>_volts`; nothing in the deck knows the part.
    let decks =
        SystemManifest::from_file(root("examples/nucleo-f103rb/analog-ntc-diode-system.yaml"))
            .unwrap_or_else(|e| panic!("load the generic deck pair for {}: {e}", profile.name));
    let ntc = decks
        .cosim_models
        .into_iter()
        .find(|model| model.id == "ntc_divider")
        .unwrap_or_else(|| panic!("the shared deck must carry the generic NTC divider"));
    manifest.cosim_models = vec![ntc];

    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load {} descriptor: {e}", profile.name));

    // One analog pad, on INP16. A descriptor that quietly grew a second pad the
    // deck does not drive would hide a routing gap, so pin the map exactly.
    assert_eq!(
        chip.analog_pins.len(),
        1,
        "{}: these parts bond one ADC input, so the map must stay one pad",
        profile.name
    );
    let pin = chip
        .analog_pins
        .get(PAD)
        .unwrap_or_else(|| panic!("{} must map {PAD} to an ADC input", profile.name));
    assert_eq!(
        (pin.peripheral.as_str(), pin.channel),
        ("adc1", CHANNEL),
        "{}: {PAD} must resolve to adc1 channel {CHANNEL} (INP16)",
        profile.name
    );

    let bus = SystemBus::from_config(&chip, &manifest)
        .unwrap_or_else(|e| panic!("build {} bus: {e}", profile.name));
    let mut machine = board_machine(bus);
    let mut session = CosimSession::new(&manifest.cosim_models, Path::new("."), &machine.bus)
        .unwrap_or_else(|e| panic!("build {} co-simulation: {e}", profile.name))
        .unwrap_or_else(|| panic!("{} deck carries the NTC divider", profile.name));
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{} generic NTC deck must resolve through {PAD} to ADC1 INP{CHANNEL}",
        profile.name
    );

    // The H7 converter is in deep power-down out of reset; that is a property
    // of the block, observed once through the production bus.
    let reset_cr = h7_power_up(&mut machine, &chip);
    assert_ne!(
        reset_cr & CR_DEEPPWD,
        0,
        "{}: the H7 converter resets in deep power-down (RM0433 §25.4.4)",
        profile.name
    );

    let mut cycles = 0;
    let mut samples = Vec::new();
    let mut idle_codes = Vec::new();
    for temperature_c in [-20.0_f64, 25.0, 85.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .unwrap_or_else(|e| panic!("set {} NTC temperature: {e}", profile.name));
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut machine.bus)
            .unwrap_or_else(|e| panic!("solve {} analog deck: {e}", profile.name));
        assert_eq!(
            steps.len(),
            1,
            "{}: the NTC deck crosses the 1 us boundary",
            profile.name
        );
        assert!(
            errors.is_empty(),
            "{}: unexpected route errors: {errors:?}",
            profile.name
        );

        let routed_v = signal_voltage(&session, "board.analog.pa0_volts");
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (routed_v - expected_ntc_v).abs() < 1e-9,
            "{} at {temperature_c} C: Beta divider should be {expected_ntc_v} V, got {routed_v} V",
            profile.name
        );

        let code = h7_convert(&mut machine, &chip, CHANNEL);
        assert_eq!(
            code,
            expected_code(routed_v),
            "{}: DR must report the solved NTC voltage at 16 bits",
            profile.name
        );
        // The physics claim, independent of the model's pipeline: the count is
        // the 16-bit ratio of the routed level to VDDA within a 12-bit LSB.
        let ideal = ideal_16_bit_count(routed_v);
        assert!(
            (f64::from(code) - ideal).abs() <= 32.0,
            "{}: {code} counts must be within a 12-bit LSB of the ideal 16-bit count {ideal}",
            profile.name
        );
        idle_codes.push(h7_convert(&mut machine, &chip, IDLE_CHANNEL));
        samples.push((temperature_c, routed_v, code));
    }

    // 1650 mV is mid-scale on a 16-bit converter: ~32752 counts. A 12-bit
    // result would mean the routed level never reached the wide path.
    assert!(
        samples[1].2 > 4095,
        "{}: the 25 C midpoint must exceed the 12-bit range, got {}",
        profile.name,
        samples[1].2
    );
    assert!(
        samples[2].1 > samples[1].1 && samples[1].1 > samples[0].1,
        "{}: the divider voltage rises with NTC temperature: {samples:?}",
        profile.name
    );
    assert!(
        samples[2].2 > samples[1].2 && samples[1].2 > samples[0].2,
        "{}: DR tracks the solved temperature-dependent voltage: {samples:?}",
        profile.name
    );
    // A channel with no routed pad must read the same at every temperature: the
    // routed level did not bleed sideways, and the channel really is selected
    // by SQ1 rather than by whatever the last conversion left behind.
    assert_eq!(
        idle_codes[0], idle_codes[1],
        "{}: an unrouted channel must not follow the routed level",
        profile.name
    );
    assert_eq!(
        idle_codes[1], idle_codes[2],
        "{}: an unrouted channel must not follow the routed level",
        profile.name
    );
    assert_ne!(
        idle_codes[1], samples[1].2,
        "{}: the unrouted channel must not report the routed code",
        profile.name
    );
}

#[test]
fn stm32h7_part_profiles_route_the_generic_ntc_deck_into_adc1_end_to_end() {
    for profile in PROFILES {
        run_profile(profile);
    }
}

/// The sweep must stay honest: every H7 profile in the tree with a PA0 analog
/// pad belongs here, or the matrix silently loses a board.
#[test]
fn sweep_covers_every_stm32h7_profile_with_an_adc_pad() {
    let mut found = Vec::new();
    let mut manifests: Vec<std::path::PathBuf> = Vec::new();
    for directory in ["configs/systems", "configs/systems/onboarding"] {
        for entry in
            std::fs::read_dir(root(directory)).unwrap_or_else(|e| panic!("list {directory}: {e}"))
        {
            let path = entry.expect("read manifest entry").path();
            if path.extension().is_some_and(|ext| ext == "yaml") {
                manifests.push(path);
            }
        }
    }
    manifests.sort();
    for path in &manifests {
        let Ok(manifest) = SystemManifest::from_file(path) else {
            continue;
        };
        let chip_path = path.parent().unwrap().join(&manifest.chip);
        let Ok(chip) = ChipDescriptor::from_file(&chip_path) else {
            continue;
        };
        if !chip.name.starts_with("stm32h7") {
            continue;
        }
        if chip.analog_pins.contains_key(PAD) && !PROFILES.iter().any(|p| p.name == manifest.name) {
            found.push(manifest.name);
        }
    }
    assert!(
        found.is_empty(),
        "these STM32H7 profiles declare {PAD} as an analog pad but no analog test loads them: {found:?}"
    );
}
