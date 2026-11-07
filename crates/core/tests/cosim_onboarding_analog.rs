// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through every imported onboarding profile
//! (`configs/systems/onboarding/*.yaml`) whose chip descriptor declares an ADC
//! LabWired can convert, into EVERY such converter instance.
//!
//! Nothing here is per-board. The converter family is read from each
//! descriptor (peripheral type plus `config.profile`), the instance base from
//! its `base_address`, and the same two netlists — a Beta NTC divider and an
//! unrelated Shockley diode load line — are routed to `adc.<id>.<ch>_volts`
//! and read back through the register sequence firmware uses for that family.
//! A new onboarding descriptor whose ADC is convertible is covered the moment
//! it lands; one whose ADC changes family moves between the lists below and
//! fails here until the coverage doc is updated.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::{Path, PathBuf};

/// Channels used on every converter: low enough to exist on all four
/// families (nRF52 SAADC has AIN0..7), clear of channel 0.
const NTC_CHANNEL: u8 = 5;
const DIODE_CHANNEL: u8 = 6;
const IDLE_CHANNEL: u8 = 7;

/// Onboarding profiles whose ADC is converted end to end here.
const CONVERTED: &[&str] = &[
    "imxrt1064",
    "mimxrt1064_evk",
    "nrf52840",
    "nucleo_f767zi",
    "nucleo_h753zi",
    "stm32f401cdu6-blackpill",
    "stm32f777",
    "stm32h7",
    "stm32h743",
    "stm32h747-disco",
    "stm32h747",
    "stm32h753",
];

/// Onboarding profiles that declare an ADC whose IP LabWired does not yet
/// convert from a routed input. The F0/L0/WBA ADC selects channels through
/// `CHSELR`, which no `adc` layout models, and the L4-map engine (L5) converts
/// a fixed internal source rather than the routed channel. Listed so that
/// closing either gap has to move the profile into [`CONVERTED`].
const DECLARED_NOT_CONVERTIBLE: &[&str] = &[
    "nucleo_wba52cg",
    "ramn",
    "stm32f042",
    "stm32f072",
    "stm32f072b_discovery",
    "stm32l071",
    "stm32l072",
    "stm32wba52",
];

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

fn millivolts(volts: f64) -> u32 {
    (volts * 1000.0).round() as u32
}

/// The converter families an onboarding descriptor can name.
#[derive(Debug, Clone, Copy)]
enum Family {
    /// Shared `adc` model, F2/F4/F7 map (RM0090/RM0410).
    Stm32F4,
    /// Shared `adc` model, H7 map (RM0433).
    Stm32H7,
    /// i.MX RT 12-bit SAR ADC (IMXRT1060RM §66).
    Imxrt,
    /// nRF52840 SAADC with EasyDMA (nRF52840 PS §6.23).
    Saadc { ram: u64 },
}

struct Converter {
    id: String,
    base: u64,
    family: Family,
}

/// Every non-stub ADC the descriptor declares, with the family it converts
/// through. `Err` names an ADC this test cannot convert and why.
fn converters(chip: &ChipDescriptor) -> Result<Vec<Converter>, String> {
    let mut found = Vec::new();
    for p in &chip.peripherals {
        let kind = p.r#type.to_ascii_lowercase();
        if kind == "stub" || !p.id.to_ascii_lowercase().contains("adc") {
            continue;
        }
        let family = if kind == "imxrt_adc" {
            Family::Imxrt
        } else if kind.contains("saadc") {
            Family::Saadc { ram: chip.ram.base }
        } else {
            match p.config.get("profile").and_then(|v| v.as_str()) {
                Some("stm32f4") => Family::Stm32F4,
                Some("stm32h7") => Family::Stm32H7,
                other => {
                    return Err(format!(
                        "{}:{} ({kind}, profile {other:?})",
                        chip.name, p.id
                    ))
                }
            }
        };
        found.push(Converter {
            id: p.id.clone(),
            base: p.base_address,
            family,
        });
    }
    Ok(found)
}

impl Converter {
    fn expected_code(&self, volts: f64) -> u32 {
        let mv = millivolts(volts);
        match self.family {
            // 12-bit, mV * 4095 / VREF (3.3 V) truncated.
            Family::Stm32F4 | Family::Stm32H7 => (mv * 4095 / 3300).min(4095),
            // 12-bit, rounded to nearest.
            Family::Imxrt => (mv.min(3300) * 4095 + 1650) / 3300,
            // Gain 1/4 against VDD/4: full scale is VDD, 12-bit, 2^12 / VDD.
            Family::Saadc { .. } => (mv * 4096 / 3300).min(4095),
        }
    }

    /// Bring the converter up the way firmware does, before any conversion.
    fn power_up(&self, bus: &mut SystemBus) {
        match self.family {
            Family::Stm32F4 => {}
            Family::Stm32H7 => {
                let cr = self.base + 0x08;
                bus.write_u32(cr, 0).expect("leave H7 ADC deep power-down");
                bus.write_u32(cr, 1 << 28).expect("enable ADC regulator");
                bus.write_u32(cr, (1 << 28) | (1 << 31))
                    .expect("calibrate ADC");
                bus.write_u32(cr, (1 << 28) | 1).expect("enable ADC");
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & 1,
                    0,
                    "{}: H7 ADC reports ADRDY",
                    self.id
                );
                bus.write_u32(self.base + 0x0C, 2 << 2)
                    .expect("select 12-bit H7 resolution");
                bus.write_u32(self.base + 0x1C, (1 << NTC_CHANNEL) | (1 << DIODE_CHANNEL))
                    .expect("preselect routed channels");
            }
            Family::Imxrt => {
                let cfg = bus.read_u32(self.base + 0x44).unwrap();
                bus.write_u32(self.base + 0x44, (cfg & !(3 << 2)) | (2 << 2))
                    .expect("select 12-bit i.MX RT ADC mode");
            }
            Family::Saadc { ram } => {
                for (slot, channel) in [(0u64, NTC_CHANNEL), (1, DIODE_CHANNEL), (2, IDLE_CHANNEL)]
                {
                    let ch = self.base + 0x510 + slot * 0x10;
                    bus.write_u32(ch, u32::from(channel) + 1)
                        .expect("select SAADC PSELP = AIN<channel>");
                    bus.write_u32(ch + 8, 0x1200)
                        .expect("select gain 1/4 and VDD/4 reference");
                }
                bus.write_u32(self.base + 0x5f0, 2)
                    .expect("select 12-bit SAADC resolution");
                bus.write_u32(self.base + 0x62c, ram as u32)
                    .expect("set EasyDMA result pointer");
                bus.write_u32(self.base + 0x630, 3)
                    .expect("reserve three results");
                bus.write_u32(self.base + 0x500, 1).expect("enable SAADC");
            }
        }
    }

    /// One firmware-style conversion per routed channel: `[ntc, diode]`.
    fn convert_routed(&self, bus: &mut SystemBus) -> [u32; 2] {
        match self.family {
            Family::Saadc { ram } => {
                bus.write_u32(self.base + 0x104, 0).expect("clear END");
                bus.write_u32(self.base, 1).expect("TASKS_START");
                bus.write_u32(self.base + 4, 1).expect("TASKS_SAMPLE");
                bus.tick_peripherals_fully();
                assert_eq!(bus.read_u32(self.base + 0x104).unwrap(), 1, "scan sets END");
                assert_eq!(bus.read_u32(self.base + 0x634).unwrap(), 3, "AMOUNT");
                assert_eq!(
                    bus.read_u16(ram + 4).unwrap(),
                    0,
                    "{}: unrouted AIN{IDLE_CHANNEL} converts to zero",
                    self.id
                );
                [
                    u32::from(bus.read_u16(ram).unwrap()),
                    u32::from(bus.read_u16(ram + 2).unwrap()),
                ]
            }
            _ => [
                self.convert(bus, NTC_CHANNEL),
                self.convert(bus, DIODE_CHANNEL),
            ],
        }
    }

    fn convert(&self, bus: &mut SystemBus, channel: u8) -> u32 {
        match self.family {
            Family::Stm32F4 => {
                const ADON: u32 = 1;
                bus.write_u32(self.base + 0x34, u32::from(channel))
                    .expect("select SQR3 SQ1");
                bus.write_u32(self.base + 0x08, ADON).expect("ADON");
                bus.write_u32(self.base + 0x08, ADON | (1 << 30))
                    .expect("SWSTART");
                for _ in 0..64 {
                    bus.tick_peripherals_fully();
                }
                assert_ne!(
                    bus.read_u32(self.base).unwrap() & (1 << 1),
                    0,
                    "{}: conversion sets EOC",
                    self.id
                );
                bus.read_u32(self.base + 0x4c).expect("read DR")
            }
            Family::Stm32H7 => {
                const EOC_EOS: u32 = (1 << 2) | (1 << 3);
                bus.write_u32(self.base + 0x30, u32::from(channel) << 6)
                    .expect("select SQR1 SQ1");
                bus.write_u32(self.base + 0x08, (1 << 28) | 1 | (1 << 2))
                    .expect("ADSTART");
                assert_eq!(
                    bus.read_u32(self.base).unwrap() & EOC_EOS,
                    EOC_EOS,
                    "{}: H7 conversion sets EOC and EOS",
                    self.id
                );
                let code = bus.read_u32(self.base + 0x40).expect("read DR");
                bus.write_u32(self.base, EOC_EOS).expect("clear EOC/EOS");
                code
            }
            Family::Imxrt => {
                bus.write_u32(self.base, u32::from(channel))
                    .expect("HC0 software trigger");
                assert_eq!(
                    bus.read_u32(self.base + 0x20).unwrap() & 1,
                    0,
                    "{}: HS.COCO0 clears when HC0 starts a conversion",
                    self.id
                );
                // A conversion takes 3 us; publish 10 us at the RT1064's
                // 600 MHz core (the model's default without a Machine) on
                // the bus clock the model reads.
                let now = bus.current_cycle;
                bus.set_current_cycle(now + 6_000);
                assert_ne!(
                    bus.read_u32(self.base + 0x20).unwrap() & 1,
                    0,
                    "{}: HS.COCO0 sets once the conversion time has elapsed",
                    self.id
                );
                bus.read_u32(self.base + 0x24).expect("read R0")
            }
            Family::Saadc { .. } => unreachable!("SAADC converts as a scan"),
        }
    }

    /// The shared STM32 model also exposes what it holds per channel, so an
    /// unrouted channel can be shown undriven rather than merely "some code".
    fn held_count(&self, bus: &mut SystemBus, channel: u8) -> Option<u16> {
        if !matches!(self.family, Family::Stm32F4 | Family::Stm32H7) {
            return None;
        }
        let index = bus.find_peripheral_index_by_name(&self.id)?;
        Some(
            bus.peripherals[index]
                .dev
                .as_any_mut()?
                .downcast_mut::<labwired_core::peripherals::adc::Adc>()
                .expect("STM32 ADC uses the shared model")
                .channel_input_count(channel),
        )
    }
}

fn generic_netlists(adc_id: &str) -> SystemManifest {
    let mut analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    for model in &mut analog.cosim_models {
        let (key, channel) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", NTC_CHANNEL),
            "diode_load_line" => ("v_diode", DIODE_CHANNEL),
            other => panic!("unexpected fixture model {other}"),
        };
        model
            .outputs
            .insert(key.into(), format!("adc.{adc_id}.{channel}_volts"));
    }
    analog
}

fn onboarding_profiles() -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(root("configs/systems/onboarding"))
        .expect("read onboarding systems")
        .map(|entry| entry.expect("onboarding entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    paths.sort();
    paths
}

fn assert_converter_routes_generic_analog(
    board: &str,
    chip: &ChipDescriptor,
    base_profile: &SystemManifest,
    converter: &Converter,
) {
    let tag = format!("{board}:{}", converter.id);
    let mut profile = base_profile.clone();
    profile.cosim_models = generic_netlists(&converter.id).cosim_models;
    let mut bus = SystemBus::from_config(chip, &profile).expect("build onboarding bus");
    converter.power_up(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{tag}: both netlists bind to ADC channels {NTC_CHANNEL}/{DIODE_CHANNEL}"
    );

    let expected_diode_v = diode_load_line_voltage();
    let ntc_path = format!("adc.{}.{NTC_CHANNEL}_volts", converter.id);
    let diode_path = format!("adc.{}.{DIODE_CHANNEL}_volts", converter.id);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both netlists");
        assert_eq!(steps.len(), 2, "{tag}: both netlists solve each step");
        assert!(errors.is_empty(), "{tag}: route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, &ntc_path);
        let r = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * r / (10_000.0 + r);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "{tag}: at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, &diode_path);
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "{tag}: Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let [ntc_code, diode_code] = converter.convert_routed(&mut bus);
        assert_eq!(
            ntc_code,
            converter.expected_code(ntc_v),
            "{tag}: firmware-visible NTC conversion at {temperature_c} °C"
        );
        assert_eq!(
            diode_code,
            converter.expected_code(diode_v),
            "{tag}: firmware-visible diode conversion"
        );
        if let Some(idle) = converter.held_count(&mut bus, IDLE_CHANNEL) {
            assert_eq!(idle, 0xFFFF, "{tag}: unrouted channel stays undriven");
        }
        if matches!(converter.family, Family::Imxrt) {
            assert_eq!(
                converter.convert(&mut bus, IDLE_CHANNEL),
                0,
                "{tag}: unrouted channel converts to zero"
            );
        }
        ntc_codes.push(ntc_code);
    }
    // The NTC sits on the low side: hotter means lower resistance, lower volts.
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "{tag}: codes track the NTC temperature: {ntc_codes:?}"
    );
}

#[test]
fn every_onboarding_profile_with_a_convertible_adc_routes_ntc_and_diode_into_each_instance() {
    let mut converted = Vec::new();
    let mut not_convertible = Vec::new();
    let mut without_adc = 0;
    let mut cpu_owned = 0;
    let mut instances = 0;
    for system_path in onboarding_profiles() {
        let board = system_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let profile =
            SystemManifest::from_file(&system_path).unwrap_or_else(|e| panic!("load {board}: {e}"));
        let chip = ChipDescriptor::from_file(system_path.parent().unwrap().join(&profile.chip))
            .unwrap_or_else(|e| panic!("load {board} descriptor: {e}"));
        // The AVR ADC registers live on the CPU, not the bus; firmware
        // conversions for these are in `cosim_onboarding_avr_analog.rs`.
        if chip.peripherals.iter().any(|p| p.r#type == "avr_adc") {
            cpu_owned += 1;
            continue;
        }
        match converters(&chip) {
            Ok(list) if list.is_empty() => without_adc += 1,
            Ok(list) => {
                for converter in &list {
                    assert_converter_routes_generic_analog(&board, &chip, &profile, converter);
                }
                instances += list.len();
                converted.push(board);
            }
            Err(_) => not_convertible.push(board),
        }
    }
    assert_eq!(
        converted, CONVERTED,
        "onboarding profiles converted end to end"
    );
    assert_eq!(
        not_convertible, DECLARED_NOT_CONVERTIBLE,
        "onboarding profiles with an ADC LabWired cannot convert yet"
    );
    assert_eq!(
        instances, 24,
        "ADC instances exercised; update docs/mixed_signal_board_coverage.md"
    );
    assert_eq!(
        cpu_owned, 25,
        "simavr profiles with a CPU-owned AVR ADC, converted in cosim_onboarding_avr_analog.rs"
    );
    assert_eq!(
        without_adc, 200,
        "onboarding profiles that declare no modelled ADC"
    );
}
