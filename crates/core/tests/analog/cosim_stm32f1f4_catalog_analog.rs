// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! One generic deck pair drives every other STM32F1 and STM32F4 profile in
//! `configs/systems/` into ADC1: the exact-part catalog smoke systems
//! (`stm32-catalog/*.yaml`, one per orderable STM32F103x4/x6/x8 and
//! STM32F401/STM32F407 density), the F103 bare and blue-pill profiles, the
//! F401/F411 black pills and the ADXL345 sensor lab.
//!
//! Nothing here is per-board code. The Beta NTC divider and the Shockley diode
//! load line name only `board.analog.<pad>_volts`; each profile's chip
//! descriptor maps PA0 -> `adc1`/0 and PA1 -> `adc1`/1, and the shared STM32F1
//! or STM32F4 ADC model performs the conversion. The table below supplies
//! three strings per profile (name, manifest, register family); the solve,
//! route, conversion and assertion path is identical for all of them.
//!
//! The two register families differ only in the addresses firmware uses
//! (RM0008 §11.12 for the F1, RM0368/RM0090 §13 for the F4): ADC1 at
//! 0x40012400 vs 0x40012000, `RCC_APB2ENR` at 0x40021018 bit 9 vs 0x40023844
//! bit 8, `CR2.SWSTART` bit 22 vs bit 30. `Family` holds exactly that, so a new
//! F1 or F4 profile joins the sweep with one table row.
//!
//! This is deliberately the *family* proof, not a re-proof of a single board:
//! the previously covered nucleo/feather profiles stay in their own files.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::Path;

/// RM0008 §8.3.7 / RM0368 §6.3.13: RCC_APB2ENR offset from RCC base, and the
/// ADC1 enable bit.
const RCC_APB2ENR_OFFSET: u64 = 0x18;
const RCC_APB2ENR_OFFSET_F4: u64 = 0x44;

/// RM0008 §11.12.3: EXTSEL[2:0] = 0b111 selects the software trigger.
const F1_CR2_ADON: u32 = 1 << 0;
const F1_CR2_EXTSEL_SW: u32 = 0b111 << 17;
const F1_CR2_SWSTART: u32 = 1 << 22;

/// RM0368 §11.13.2: F4 ADC_CR2.ADON (bit 0) and SWSTART (bit 30).
const F4_CR2_ADON: u32 = 1 << 0;
const F4_CR2_SWSTART: u32 = 1 << 30;

/// ADC_SR.EOC (bit 1) on both register maps.
const SR_EOC: u32 = 1 << 1;
/// ADC_DR, common offset on both register maps.
const DR_OFFSET: u64 = 0x4C;
/// ADC_SQR3, common offset; SQ1 holds the channel to convert.
const SQR3_OFFSET: u64 = 0x34;

/// The generic decks, used as a deck source: only their `cosim_models` are
/// overlaid on each profile, so the deck's own `chip:` is not read.
const GENERIC_DECKS: &str = "examples/nucleo-f103rb/analog-ntc-diode-system.yaml";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Family {
    /// STM32F1: ADC1 0x40012400, APB2ENR +0x18 bit 9, SWSTART bit 22.
    F1,
    /// STM32F4: ADC1 0x40012000, APB2ENR +0x44 bit 8, SWSTART bit 30.
    F4,
}

impl Family {
    fn adc1_base(self) -> u64 {
        match self {
            Family::F1 => 0x4001_2400,
            Family::F4 => 0x4001_2000,
        }
    }

    fn rcc_base(self) -> u64 {
        match self {
            Family::F1 => 0x4002_1000,
            Family::F4 => 0x4002_3800,
        }
    }

    fn apb2enr_offset(self) -> u64 {
        match self {
            Family::F1 => RCC_APB2ENR_OFFSET,
            Family::F4 => RCC_APB2ENR_OFFSET_F4,
        }
    }

    fn adc1en(self) -> u32 {
        match self {
            Family::F1 => 1 << 9,
            Family::F4 => 1 << 8,
        }
    }

    fn cr2_enable(self) -> u32 {
        match self {
            Family::F1 => F1_CR2_ADON | F1_CR2_EXTSEL_SW,
            Family::F4 => F4_CR2_ADON,
        }
    }

    fn cr2_start(self) -> u32 {
        match self {
            Family::F1 => F1_CR2_SWSTART,
            Family::F4 => F4_CR2_SWSTART,
        }
    }
}

struct Profile {
    name: &'static str,
    system: &'static str,
    family: Family,
}

/// Every STM32F1/F4 profile whose descriptor declares PA0/PA1 on `adc1` and
/// that no other analog test already covers.
const PROFILES: &[Profile] = &[
    // ── F103: lab, demo board, bare and the blue pills ─────────────────────
    Profile {
        name: "adxl345-sensor-lab",
        system: "configs/systems/adxl345-sensor-lab.yaml",
        family: Family::F1,
    },
    Profile {
        name: "labwired-demo-board",
        system: "configs/systems/labwired-demo-board.yaml",
        family: Family::F1,
    },
    Profile {
        name: "stm32f103-bare",
        system: "configs/systems/stm32f103-bare.yaml",
        family: Family::F1,
    },
    Profile {
        name: "stm32f103-blue-pill",
        system: "configs/systems/stm32f103-blue-pill.yaml",
        family: Family::F1,
    },
    Profile {
        name: "stm32f103-blue-pill-bme280",
        system: "configs/systems/stm32f103-blue-pill-bme280.yaml",
        family: Family::F1,
    },
    // ── F103 exact-part catalog: x4/x6 low density, x8 medium density ──────
    Profile {
        name: "stm32f103c4-smoke",
        system: "configs/systems/stm32-catalog/stm32f103c4.yaml",
        family: Family::F1,
    },
    Profile {
        name: "stm32f103c6-smoke",
        system: "configs/systems/stm32-catalog/stm32f103c6.yaml",
        family: Family::F1,
    },
    Profile {
        name: "stm32f103r4-smoke",
        system: "configs/systems/stm32-catalog/stm32f103r4.yaml",
        family: Family::F1,
    },
    Profile {
        name: "stm32f103r4h-smoke",
        system: "configs/systems/stm32-catalog/stm32f103r4h.yaml",
        family: Family::F1,
    },
    Profile {
        name: "stm32f103r6-smoke",
        system: "configs/systems/stm32-catalog/stm32f103r6.yaml",
        family: Family::F1,
    },
    Profile {
        name: "stm32f103r6h-smoke",
        system: "configs/systems/stm32-catalog/stm32f103r6h.yaml",
        family: Family::F1,
    },
    Profile {
        name: "stm32f103r8-smoke",
        system: "configs/systems/stm32-catalog/stm32f103r8.yaml",
        family: Family::F1,
    },
    Profile {
        name: "stm32f103t4-smoke",
        system: "configs/systems/stm32-catalog/stm32f103t4.yaml",
        family: Family::F1,
    },
    Profile {
        name: "stm32f103t6-smoke",
        system: "configs/systems/stm32-catalog/stm32f103t6.yaml",
        family: Family::F1,
    },
    // ── F4: the two black pills ───────────────────────────────────────────
    Profile {
        name: "stm32f401cdu6",
        system: "configs/systems/stm32f401cdu6.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f401cdu6-blackpill",
        system: "configs/systems/stm32f401cdu6-blackpill.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f411ceu6-blackpill",
        system: "configs/systems/stm32f411ceu6-blackpill.yaml",
        family: Family::F4,
    },
    // ── F401 exact-part catalog: 48-pin C, 64-pin R, 100-pin V ────────────
    Profile {
        name: "stm32f401cb-smoke",
        system: "configs/systems/stm32-catalog/stm32f401cb.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f401cc-smoke",
        system: "configs/systems/stm32-catalog/stm32f401cc.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f401cd-smoke",
        system: "configs/systems/stm32-catalog/stm32f401cd.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f401ce-smoke",
        system: "configs/systems/stm32-catalog/stm32f401ce.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f401rb-smoke",
        system: "configs/systems/stm32-catalog/stm32f401rb.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f401rc-smoke",
        system: "configs/systems/stm32-catalog/stm32f401rc.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f401rd-smoke",
        system: "configs/systems/stm32-catalog/stm32f401rd.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f401re-smoke",
        system: "configs/systems/stm32-catalog/stm32f401re.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f401vb-smoke",
        system: "configs/systems/stm32-catalog/stm32f401vb.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f401vc-smoke",
        system: "configs/systems/stm32-catalog/stm32f401vc.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f401vd-smoke",
        system: "configs/systems/stm32-catalog/stm32f401vd.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f401ve-smoke",
        system: "configs/systems/stm32-catalog/stm32f401ve.yaml",
        family: Family::F4,
    },
    // ── F407 exact-part catalog: 100/144-pin I, 100/144-pin Z, V ───────────
    Profile {
        name: "stm32f407ie-smoke",
        system: "configs/systems/stm32-catalog/stm32f407ie.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f407ig-smoke",
        system: "configs/systems/stm32-catalog/stm32f407ig.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f407ve-smoke",
        system: "configs/systems/stm32-catalog/stm32f407ve.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f407vg-smoke",
        system: "configs/systems/stm32-catalog/stm32f407vg.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f407ze-smoke",
        system: "configs/systems/stm32-catalog/stm32f407ze.yaml",
        family: Family::F4,
    },
    Profile {
        name: "stm32f407zg-smoke",
        system: "configs/systems/stm32-catalog/stm32f407zg.yaml",
        family: Family::F4,
    },
];

/// Beta-parameter NTC resistance, the model card in the shared deck.
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

/// One software-triggered regular conversion, with the register sequence the
/// family's firmware uses: clock gate, SQ1 = channel, enable, start, EOC, DR.
fn adc1_convert(machine: &mut Machine<CortexM>, family: Family, channel: u8) -> u32 {
    let apb2enr = family.rcc_base() + family.apb2enr_offset();
    let adc1 = family.adc1_base();
    let bus = &mut machine.bus;
    let current = bus.read_u32(apb2enr).expect("read RCC APB2ENR");
    bus.write_u32(apb2enr, current | family.adc1en())
        .expect("enable the ADC1 clock");
    bus.write_u32(adc1 + SQR3_OFFSET, u32::from(channel))
        .expect("select SQR3 SQ1 channel");
    bus.write_u32(adc1 + 0x08, family.cr2_enable())
        .expect("power up ADC1 with the software trigger selected");
    bus.write_u32(adc1 + 0x08, family.cr2_enable() | family.cr2_start())
        .expect("software-start the ADC1 conversion");
    for _ in 0..40 {
        machine.run(Some(1)).expect("advance the guest one cycle");
    }
    let bus = &mut machine.bus;
    let status = bus.read_u32(adc1).expect("read ADC1 SR");
    assert_ne!(status & SR_EOC, 0, "ADC1 conversion completes with EOC set");
    bus.read_u32(adc1 + DR_OFFSET)
        .expect("read firmware-visible ADC1 DR")
}

/// The conversion the model reports: the routed pad voltage in whole
/// millivolts, scaled to the 12-bit range against the 3.3 V reference.
fn expected_adc_count(voltage_v: f64) -> u32 {
    let millivolts = (voltage_v * 1000.0).round();
    (millivolts * 4095.0 / 3300.0).floor().clamp(0.0, 4095.0) as u32
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

    let decks = SystemManifest::from_file(root(GENERIC_DECKS))
        .unwrap_or_else(|e| panic!("load the generic decks for {}: {e}", profile.name));
    assert_eq!(
        decks.cosim_models.len(),
        2,
        "the shared deck must carry the NTC divider and the diode load line"
    );
    manifest.cosim_models = decks.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load {} descriptor: {e}", profile.name));

    // The deck publishes PA0 and PA1, so both must resolve to ADC1 channels
    // 0 and 1 on every part in the sweep, by descriptor data alone.
    for (pad, channel) in [("PA0", 0), ("PA1", 1)] {
        let pin = chip
            .analog_pins
            .get(pad)
            .unwrap_or_else(|| panic!("{} must map {pad} to an ADC input", profile.name));
        assert_eq!(
            (pin.peripheral.as_str(), pin.channel),
            ("adc1", channel),
            "{}: {pad} must resolve to adc1 channel {channel}",
            profile.name
        );
    }
    // Every analog pad on these parts belongs to ADC1 with its own channel; a
    // duplicate channel would make one pad's level shadow another's.
    let mut channels: Vec<u8> = Vec::new();
    for (pad, pin) in &chip.analog_pins {
        assert_eq!(
            pin.peripheral, "adc1",
            "{}: {pad} is not an ADC1 input, so this sweep would not drive it",
            profile.name
        );
        assert!(
            !channels.contains(&pin.channel),
            "{}: two pads claim ADC1 channel {}",
            profile.name,
            pin.channel
        );
        channels.push(pin.channel);
    }

    let bus = SystemBus::from_config(&chip, &manifest)
        .unwrap_or_else(|e| panic!("build {} bus: {e}", profile.name));
    let mut machine = board_machine(bus);
    let mut session = CosimSession::new(&manifest.cosim_models, Path::new("."), &machine.bus)
        .unwrap_or_else(|e| panic!("build {} co-simulation: {e}", profile.name))
        .unwrap_or_else(|| panic!("{} deck carries both analog devices", profile.name));
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{} generic decks must resolve through PA0/PA1 to ADC1 channels 0/1",
        profile.name
    );

    let expected_diode_v = diode_load_line_voltage();
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
            2,
            "{}: both decks cross the 1 us boundary",
            profile.name
        );
        assert!(
            errors.is_empty(),
            "{}: unexpected route errors: {errors:?}",
            profile.name
        );

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "{} at {temperature_c} C: Beta divider should be {expected_ntc_v} V, got {ntc_v} V",
            profile.name
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "{}: Shockley load line should be {expected_diode_v} V, got {diode_v} V",
            profile.name
        );

        let ntc_count = adc1_convert(&mut machine, profile.family, 0);
        assert_eq!(
            ntc_count,
            expected_adc_count(ntc_v),
            "{}: firmware-visible ADC1 channel 0 must report the solved NTC voltage",
            profile.name
        );
        let diode_count = adc1_convert(&mut machine, profile.family, 1);
        assert_eq!(
            diode_count,
            expected_adc_count(diode_v),
            "{}: firmware-visible ADC1 channel 1 must report the solved diode voltage",
            profile.name
        );
        // An unrouted channel reports the code after the previous DR value, so
        // this assertion also proves the neighbour did not take a routed input.
        assert_eq!(
            adc1_convert(&mut machine, profile.family, 2),
            (diode_count + 1) & 0xfff,
            "{}: the undriven neighbouring channel uses its fallback",
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
        samples[1].2, 2047,
        "{}: the 25 C NTC divider is half scale on the 3.3 V reference",
        profile.name
    );
}

#[test]
fn every_stm32f1f4_profile_routes_the_generic_decks_to_adc1_end_to_end() {
    for profile in PROFILES {
        run_profile(profile);
    }
}

/// The sweep must stay honest: a profile that gains an analog deck of its own,
/// or a new STM32F1/F4 profile that appears in `configs/systems/`, must be
/// added here rather than silently missing from the matrix.
#[test]
fn sweep_covers_every_uncovered_stm32f1f4_profile_with_an_adc1_pad_map() {
    // Profiles a dedicated file already drives end to end. Adding one here
    // without removing it below only costs a duplicate run, never a false
    // claim; leaving it out of both lists is what this test refuses.
    let covered_elsewhere = [
        "nucleo-f103rb",
        "nucleo-f103rb-epaper",
        "nucleo-f401re",
        "nucleo-f407",
        "nucleo-f411re",
        "nucleo-f446re",
        "nucleo-f767zi",
        "feather-f405",
        "stm32f4-discovery",
    ];
    let mut found = Vec::new();
    for name in config_files() {
        let path = root(&format!("configs/systems/{name}"));
        let Ok(manifest) = SystemManifest::from_file(&path) else {
            continue;
        };
        if covered_elsewhere.contains(&manifest.name.as_str())
            || PROFILES.iter().any(|p| p.name == manifest.name)
        {
            continue;
        }
        let chip_path = path.parent().unwrap().join(&manifest.chip);
        let Ok(chip) = ChipDescriptor::from_file(&chip_path) else {
            continue;
        };
        let is_f1f4 = matches!(
            chip.name.as_str(),
            n if n.starts_with("stm32f1") || n.starts_with("stm32f4")
        );
        let has_pa0 = chip
            .analog_pins
            .get("PA0")
            .is_some_and(|pin| pin.peripheral == "adc1" && pin.channel == 0);
        if is_f1f4 && has_pa0 {
            found.push(manifest.name);
        }
    }
    assert!(
        found.is_empty(),
        "these STM32F1/F4 profiles declare PA0 on ADC1 but no analog test loads them: {found:?}"
    );
}

/// Every system manifest at the top level and in `stm32-catalog/`, as paths
/// relative to `configs/systems/`.
fn config_files() -> Vec<String> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root("configs/systems")).expect("list configs/systems") {
        let path = entry.expect("read configs/systems entry").path();
        if path.extension().is_some_and(|ext| ext == "yaml") {
            files.push(
                path.file_name()
                    .expect("manifest has a file name")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    let catalog = root("configs/systems/stm32-catalog");
    for entry in std::fs::read_dir(&catalog).expect("list configs/systems/stm32-catalog") {
        let path = entry.expect("read stm32-catalog entry").path();
        if path.extension().is_some_and(|ext| ext == "yaml") {
            files.push(format!(
                "stm32-catalog/{}",
                path.file_name()
                    .expect("catalog manifest has a file name")
                    .to_string_lossy()
            ));
        }
    }
    files
}
