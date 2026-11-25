// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! One generic deck pair drives every STM32F1/F4 profile in the tree into
//! ADC1: the exact-part catalog smoke systems (`stm32-catalog/*.yaml`, one per
//! orderable STM32F103x4/x6/x8 and STM32F401/STM32F407 density), the F103
//! bare, blue-pill and demo profiles, the F401/F411 black pills, the ADXL345
//! sensor lab and the onboarding part profiles (`onboarding/*.yaml`).
//!
//! Nothing here is per-board code. The Beta NTC divider and the Shockley diode
//! load line name only `board.analog.<pad>_volts`; each profile's chip
//! descriptor maps the published pad to an `adc1` channel, and the shared
//! STM32F1 or STM32F4 ADC model performs the conversion.
//!
//! The table supplies what actually differs between profiles:
//!
//! * the manifest,
//! * the register family (`Family`: F1 0x40012400 / `APB2ENR` +0x18 bit 9 /
//!   `SWSTART` bit 22 from RM0008 §11.12; F4 0x40012000 / `APB2ENR` +0x44
//!   bit 8 / `SWSTART` bit 30 from RM0368 and RM0090 §13),
//! * which pads the deck publishes — PA0/PA1 on the nucleo, catalog and
//!   black-pill boards, PA1/PA2 or PA1/PB1 or PA1 alone on the onboarding
//!   parts, whose descriptors bond a different pad set.
//!
//! The onboarding `stm32f4` profile bonds one analog pad, so it runs the
//! single-probe deck; that is the point of a generic deck — the part says what
//! it can convert, the deck only publishes a level.

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

/// The generic pair as first written for the NUCLEO-F103RB: PA0 and PA1.
const GENERIC_PAIR: &str = "examples/nucleo-f103rb/analog-ntc-diode-system.yaml";
/// The same NTC divider, published on PA1 only, for parts that bond one pad.
const NTC_PA1: &str = "examples/stm32-onboarding-analog/analog-ntc-pa1-system.yaml";
/// The same pair, published on PA1 and PA2.
const PA1_PA2: &str = "examples/stm32-onboarding-analog/analog-ntc-diode-pa1-pa2-system.yaml";
/// The same pair, published on PA1 and PB1.
const PA1_PB1: &str = "examples/stm32-onboarding-analog/analog-ntc-diode-pa1-pb1-system.yaml";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Family {
    /// STM32F1: ADC1 0x40012400, APB2ENR +0x18 bit 9, SWSTART bit 22.
    F1,
    /// STM32F4/F7: ADC1 0x40012000, APB2ENR +0x44 bit 8, SWSTART bit 30.
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
    deck: &'static str,
    /// The pad the NTC divider publishes, and the channel it must select.
    ntc: (&'static str, u8),
    /// The pad the diode load line publishes, when the part bonds a second one.
    diode: Option<(&'static str, u8)>,
}

/// A profile whose descriptor bonds PA0/PA1, driven by the shared pair.
const fn pa0_pa1(name: &'static str, system: &'static str, family: Family) -> Profile {
    Profile {
        name,
        system,
        family,
        deck: GENERIC_PAIR,
        ntc: ("PA0", 0),
        diode: Some(("PA1", 1)),
    }
}

/// Every STM32F1/F4 profile that no dedicated test already covers.
const PROFILES: &[Profile] = &[
    // ── F103: lab, demo board, bare and the blue pills ─────────────────────
    pa0_pa1(
        "adxl345-sensor-lab",
        "configs/systems/adxl345-sensor-lab.yaml",
        Family::F1,
    ),
    pa0_pa1(
        "labwired-demo-board",
        "configs/systems/labwired-demo-board.yaml",
        Family::F1,
    ),
    pa0_pa1(
        "stm32f103-bare",
        "configs/systems/stm32f103-bare.yaml",
        Family::F1,
    ),
    pa0_pa1(
        "stm32f103-blue-pill",
        "configs/systems/stm32f103-blue-pill.yaml",
        Family::F1,
    ),
    pa0_pa1(
        "stm32f103-blue-pill-bme280",
        "configs/systems/stm32f103-blue-pill-bme280.yaml",
        Family::F1,
    ),
    // ── F103 exact-part catalog: x4/x6 low density, x8 medium density ──────
    pa0_pa1(
        "stm32f103c4-smoke",
        "configs/systems/stm32-catalog/stm32f103c4.yaml",
        Family::F1,
    ),
    pa0_pa1(
        "stm32f103c6-smoke",
        "configs/systems/stm32-catalog/stm32f103c6.yaml",
        Family::F1,
    ),
    pa0_pa1(
        "stm32f103r4-smoke",
        "configs/systems/stm32-catalog/stm32f103r4.yaml",
        Family::F1,
    ),
    pa0_pa1(
        "stm32f103r4h-smoke",
        "configs/systems/stm32-catalog/stm32f103r4h.yaml",
        Family::F1,
    ),
    pa0_pa1(
        "stm32f103r6-smoke",
        "configs/systems/stm32-catalog/stm32f103r6.yaml",
        Family::F1,
    ),
    pa0_pa1(
        "stm32f103r6h-smoke",
        "configs/systems/stm32-catalog/stm32f103r6h.yaml",
        Family::F1,
    ),
    pa0_pa1(
        "stm32f103r8-smoke",
        "configs/systems/stm32-catalog/stm32f103r8.yaml",
        Family::F1,
    ),
    pa0_pa1(
        "stm32f103t4-smoke",
        "configs/systems/stm32-catalog/stm32f103t4.yaml",
        Family::F1,
    ),
    pa0_pa1(
        "stm32f103t6-smoke",
        "configs/systems/stm32-catalog/stm32f103t6.yaml",
        Family::F1,
    ),
    // ── F4: the two black pills ───────────────────────────────────────────
    pa0_pa1(
        "stm32f401cdu6",
        "configs/systems/stm32f401cdu6.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f401cdu6-blackpill",
        "configs/systems/stm32f401cdu6-blackpill.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f411ceu6-blackpill",
        "configs/systems/stm32f411ceu6-blackpill.yaml",
        Family::F4,
    ),
    // ── F401 exact-part catalog: 48-pin C, 64-pin R, 100-pin V ────────────
    pa0_pa1(
        "stm32f401cb-smoke",
        "configs/systems/stm32-catalog/stm32f401cb.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f401cc-smoke",
        "configs/systems/stm32-catalog/stm32f401cc.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f401cd-smoke",
        "configs/systems/stm32-catalog/stm32f401cd.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f401ce-smoke",
        "configs/systems/stm32-catalog/stm32f401ce.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f401rb-smoke",
        "configs/systems/stm32-catalog/stm32f401rb.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f401rc-smoke",
        "configs/systems/stm32-catalog/stm32f401rc.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f401rd-smoke",
        "configs/systems/stm32-catalog/stm32f401rd.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f401re-smoke",
        "configs/systems/stm32-catalog/stm32f401re.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f401vb-smoke",
        "configs/systems/stm32-catalog/stm32f401vb.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f401vc-smoke",
        "configs/systems/stm32-catalog/stm32f401vc.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f401vd-smoke",
        "configs/systems/stm32-catalog/stm32f401vd.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f401ve-smoke",
        "configs/systems/stm32-catalog/stm32f401ve.yaml",
        Family::F4,
    ),
    // ── F407 exact-part catalog: 100/144-pin I, 100/144-pin Z, V ───────────
    pa0_pa1(
        "stm32f407ie-smoke",
        "configs/systems/stm32-catalog/stm32f407ie.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f407ig-smoke",
        "configs/systems/stm32-catalog/stm32f407ig.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f407ve-smoke",
        "configs/systems/stm32-catalog/stm32f407ve.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f407vg-smoke",
        "configs/systems/stm32-catalog/stm32f407vg.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f407ze-smoke",
        "configs/systems/stm32-catalog/stm32f407ze.yaml",
        Family::F4,
    ),
    pa0_pa1(
        "stm32f407zg-smoke",
        "configs/systems/stm32-catalog/stm32f407zg.yaml",
        Family::F4,
    ),
    // ── Onboarding part profiles: the pad sets these parts bond ────────────
    pa0_pa1(
        "stm32f103",
        "configs/systems/onboarding/stm32f103.yaml",
        Family::F1,
    ),
    Profile {
        // One analog pad only (PA1), so the NTC runs alone.
        name: "stm32f4",
        system: "configs/systems/onboarding/stm32f4.yaml",
        family: Family::F4,
        deck: NTC_PA1,
        ntc: ("PA1", 1),
        diode: None,
    },
    Profile {
        name: "stm32f412",
        system: "configs/systems/onboarding/stm32f412.yaml",
        family: Family::F4,
        deck: PA1_PA2,
        ntc: ("PA1", 1),
        diode: Some(("PA2", 2)),
    },
    Profile {
        name: "stm32f429",
        system: "configs/systems/onboarding/stm32f429.yaml",
        family: Family::F4,
        deck: PA1_PA2,
        ntc: ("PA1", 1),
        diode: Some(("PA2", 2)),
    },
    Profile {
        name: "stm32f746",
        system: "configs/systems/onboarding/stm32f746.yaml",
        family: Family::F4,
        deck: PA1_PA2,
        ntc: ("PA1", 1),
        diode: Some(("PA2", 2)),
    },
    Profile {
        name: "stm32f4_discovery",
        system: "configs/systems/onboarding/stm32f4_discovery.yaml",
        family: Family::F4,
        deck: PA1_PB1,
        ntc: ("PA1", 1),
        diode: Some(("PB1", 9)),
    },
    Profile {
        name: "stm32f4_discovery-kit",
        system: "configs/systems/onboarding/stm32f4_discovery-kit.yaml",
        family: Family::F4,
        deck: PA1_PB1,
        ntc: ("PA1", 1),
        diode: Some(("PB1", 9)),
    },
    // The F7 part profiles include the STM32F746 silicon surface (ADC1 on
    // PA1/IN1 and PA2/IN2); the F76x/F77x leaves only add memory and
    // peripherals this deck does not touch.
    Profile {
        name: "nucleo_f767zi",
        system: "configs/systems/onboarding/nucleo_f767zi.yaml",
        family: Family::F4,
        deck: PA1_PA2,
        ntc: ("PA1", 1),
        diode: Some(("PA2", 2)),
    },
    Profile {
        name: "stm32f7_discovery-bb",
        system: "configs/systems/onboarding/stm32f7_discovery-bb.yaml",
        family: Family::F4,
        deck: PA1_PA2,
        ntc: ("PA1", 1),
        diode: Some(("PA2", 2)),
    },
    Profile {
        name: "stm32f777",
        system: "configs/systems/onboarding/stm32f777.yaml",
        family: Family::F4,
        deck: PA1_PA2,
        ntc: ("PA1", 1),
        diode: Some(("PA2", 2)),
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

/// The lowest channel no deck publishes: a conversion there must show the
/// model's fallback, never a routed level.
fn idle_channel(profile: &Profile) -> u8 {
    let published = [profile.ntc.1];
    let published = match profile.diode {
        Some((_, channel)) => [published[0], channel],
        None => [published[0], published[0]],
    };
    (0u8..16)
        .find(|channel| !published.contains(channel))
        .expect("a 16-channel converter leaves one channel free")
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
        .unwrap_or_else(|e| panic!("load the generic decks for {}: {e}", profile.name));
    assert_eq!(
        decks.cosim_models.len(),
        if profile.diode.is_some() { 2 } else { 1 },
        "{}: the deck must carry one model per published pad",
        profile.name
    );
    manifest.cosim_models = decks.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load {} descriptor: {e}", profile.name));

    // The deck publishes these pads, so each must resolve to ADC1 by
    // descriptor data alone.
    let mut expected = vec![profile.ntc];
    if let Some(diode) = profile.diode {
        expected.push(diode);
    }
    for (pad, channel) in &expected {
        let pin = chip
            .analog_pins
            .get(*pad)
            .unwrap_or_else(|| panic!("{} must map {pad} to an ADC input", profile.name));
        assert_eq!(
            (pin.peripheral.as_str(), pin.channel),
            ("adc1", *channel),
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
        .unwrap_or_else(|| panic!("{} deck carries its analog devices", profile.name));
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{} generic decks must resolve through {:?} to ADC1",
        profile.name,
        expected
    );

    let expected_diode_v = diode_load_line_voltage();
    let idle = idle_channel(profile);
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
            expected.len(),
            "{}: every deck crosses the 1 us boundary",
            profile.name
        );
        assert!(
            errors.is_empty(),
            "{}: unexpected route errors: {errors:?}",
            profile.name
        );

        let ntc_v = signal_voltage(&session, profile.ntc.0);
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "{} at {temperature_c} C: Beta divider on {} should be {expected_ntc_v} V, got {ntc_v} V",
            profile.name,
            profile.ntc.0
        );

        let ntc_count = adc1_convert(&mut machine, profile.family, profile.ntc.1);
        assert_eq!(
            ntc_count,
            expected_adc_count(ntc_v),
            "{}: firmware-visible ADC1 channel {} must report the solved NTC voltage",
            profile.name,
            profile.ntc.1
        );
        let mut last_count = ntc_count;
        if let Some((pad, channel)) = profile.diode {
            let diode_v = signal_voltage(&session, pad);
            assert!(
                (diode_v - expected_diode_v).abs() < 2e-5,
                "{}: Shockley load line on {pad} should be {expected_diode_v} V, got {diode_v} V",
                profile.name
            );
            last_count = adc1_convert(&mut machine, profile.family, channel);
            assert_eq!(
                last_count,
                expected_adc_count(diode_v),
                "{}: firmware-visible ADC1 channel {channel} must report the solved diode voltage",
                profile.name
            );
        }
        // An unrouted channel reports the code after the previous DR value, so
        // this assertion also proves the neighbour did not take a routed input.
        assert_eq!(
            adc1_convert(&mut machine, profile.family, idle),
            (last_count + 1) & 0xfff,
            "{}: the undriven channel {idle} uses its fallback",
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

/// The ADC layout the descriptor asks for, from its `adc1` `config.profile`.
fn adc1_layout(chip: &ChipDescriptor) -> Option<&str> {
    chip.peripherals
        .iter()
        .find(|peripheral| peripheral.id == "adc1")
        .and_then(|peripheral| peripheral.config.get("profile"))
        .and_then(|value| value.as_str())
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
        "at32f421g8u7",
        "feather-f405",
        "nucleo-f103rb",
        "nucleo-f103rb-epaper",
        "nucleo-f401re",
        "nucleo-f407",
        "nucleo-f411re",
        "nucleo-f446re",
        "nucleo-f767zi",
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
        // The layout the descriptor requests, not the part's name: the
        // STM32F746 asks for `stm32f4` registers, and a name test would miss
        // it while catching parts named `stm32f4...` that use another block.
        if !matches!(adc1_layout(&chip), Some("stm32f1" | "stm32f4")) {
            continue;
        }
        let publishes_a_bonded_pad = ["PA0", "PA1"]
            .iter()
            .any(|pad| chip.analog_pins.contains_key(*pad));
        if publishes_a_bonded_pad {
            found.push(manifest.name);
        }
    }
    assert!(
        found.is_empty(),
        "these STM32F1/F4 profiles declare an ADC1 pad but no analog test loads them: {found:?}"
    );
}

/// Every system manifest the sweep can reach: the top level, the exact-part
/// catalog and the onboarding tree.
fn config_files() -> Vec<String> {
    let mut files = Vec::new();
    for (directory, prefix) in [
        ("configs/systems", ""),
        ("configs/systems/stm32-catalog", "stm32-catalog/"),
        ("configs/systems/onboarding", "onboarding/"),
    ] {
        for entry in
            std::fs::read_dir(root(directory)).unwrap_or_else(|e| panic!("list {directory}: {e}"))
        {
            let path = entry.expect("read manifest entry").path();
            if path.extension().is_some_and(|ext| ext == "yaml") {
                files.push(format!(
                    "{prefix}{}",
                    path.file_name()
                        .expect("manifest has a file name")
                        .to_string_lossy()
                ));
            }
        }
    }
    files
}
