// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The classic ESP32's `analog_pins:` table reaches its SENS SAR ADC end to
//! end, on every profile that loads `esp32.yaml`.
//!
//! `esp32.yaml` transcribes the chip but carried no ADC pad map, so
//! `board.analog.<pad>_volts` could not resolve: the only ESP32 analog test
//! routed an explicit `adc.sens_sar_adc.6_volts`. This slice publishes the
//! eight ADC1 pads (GPIO36/37/38/39/32/33/34/35 -> ADC1_CH0..CH7) in the same
//! `analog_pins:` block the STM32, AVR, nRF52 and RP2040 chips use. The union
//! is already read by `ChipDescriptor::analog_pin_table`, so no board-specific
//! route and no engine change is needed.
//!
//! The generic Beta NTC divider and Shockley load line then load unchanged on
//! five profiles across the ESP32 family. The test solves both nets, compares
//! each node to an independent closed form, and reads the firmware-visible
//! SAR1 `DATA_SAR` register through the production SENS register map.

use crate::common;
use common::root;
use labwired_config::ChipDescriptor;
use labwired_config::SystemManifest;
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::esp32::sar_adc::SENS_BASE;
use labwired_core::Bus;
use std::path::Path;

const VREF_MV: f64 = 3_300.0;

struct Profile {
    /// The profile the assertion names when it fails.
    name: &'static str,
    system: &'static str,
    /// The pads the deck publishes and the channel the descriptor must give
    /// them. Both are ADC1 inputs, so one SAR1 conversion covers each.
    pads: &'static [(&'static str, u8)],
}

const GENERIC_DECK: &str =
    "examples/adafruit-feather-esp32-v2/analog-ntc-diode-gpio34-gpio35-system.yaml";

const PROFILES: &[Profile] = &[
    Profile {
        name: "adafruit-feather-esp32-v2-bme280",
        system: "configs/systems/adafruit-feather-esp32-v2-bme280.yaml",
        pads: &[("GPIO34", 6), ("GPIO35", 7)],
    },
    Profile {
        name: "esp32-devkitm-1-bme280",
        system: "configs/systems/esp32-devkitm-1-bme280.yaml",
        pads: &[("GPIO34", 6), ("GPIO35", 7)],
    },
    Profile {
        name: "esp32-devkitm-1",
        system: "configs/systems/esp32-devkitm-1.yaml",
        pads: &[("GPIO34", 6), ("GPIO35", 7)],
    },
    Profile {
        name: "esp32-wroom-32",
        system: "configs/systems/esp32-wroom-32.yaml",
        pads: &[("GPIO34", 6), ("GPIO35", 7)],
    },
    Profile {
        name: "esp32-wroom-epaper",
        system: "configs/systems/esp32-wroom-epaper.yaml",
        pads: &[("GPIO34", 6), ("GPIO35", 7)],
    },
];

/// Beta-parameter NTC resistance, the model card in the generic deck.
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

/// One software-triggered SAR1 conversion on the classic ESP32 SENS map.
fn sample_sar1(bus: &mut SystemBus, channel: u8) -> u32 {
    bus.write_u32(u64::from(SENS_BASE), 3 << 16).unwrap();
    let start = (1u32 << (19 + u32::from(channel))) | (1 << 18) | (1 << 17);
    bus.write_u32(u64::from(SENS_BASE) + 0x54, start).unwrap();
    let result = bus.read_u32(u64::from(SENS_BASE) + 0x54).unwrap();
    assert_ne!(result & (1 << 16), 0, "DONE_SAR is raised");
    result & 0x0FFF
}

/// The 12-bit code the SENS model derives for a routed node.
fn expected_adc_count(voltage_v: f64) -> u32 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 4095 / VREF_MV as u32).min(4095)
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

    let deck = SystemManifest::from_file(root(GENERIC_DECK))
        .unwrap_or_else(|e| panic!("load the generic deck for {}: {e}", profile.name));
    assert_eq!(
        deck.cosim_models.len(),
        profile.pads.len(),
        "{}: the deck must carry one model per published pad",
        profile.name
    );
    manifest.cosim_models = deck.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load {} descriptor: {e}", profile.name));
    assert!(
        !chip.analog_pins.is_empty(),
        "{}: the classic ESP32 descriptor must publish ADC pads",
        profile.name
    );
    assert_eq!(
        chip.analog_pin_conflicts(),
        Vec::new(),
        "{}: the analog pad tables disagree",
        profile.name
    );
    let table = chip.analog_pin_table();
    for (pad, channel) in profile.pads {
        let pin = table
            .get(*pad)
            .unwrap_or_else(|| panic!("{} must map {pad} to an ADC input", profile.name));
        assert_eq!(
            (pin.peripheral.as_str(), pin.channel),
            ("sens_sar_adc", *channel),
            "{}: {pad} must resolve to sens_sar_adc channel {channel}",
            profile.name
        );
    }

    let bus = SystemBus::from_config(&chip, &manifest)
        .unwrap_or_else(|e| panic!("build {} bus: {e}", profile.name));
    let mut session = CosimSession::new(&manifest.cosim_models, Path::new("."), &bus)
        .unwrap_or_else(|e| panic!("build {} co-simulation: {e}", profile.name))
        .unwrap_or_else(|| panic!("{} deck carries its analog devices", profile.name));
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{}: the generic deck must resolve through `analog_pins:`",
        profile.name
    );

    let mut bus = bus;
    let (ntc_pad, ntc_channel) = profile.pads[0];
    let (diode_pad, diode_channel) = profile.pads[1];
    let mut cycles = 0;
    let mut samples = Vec::new();
    for temperature_c in [-20.0_f64, 25.0, 85.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .unwrap_or_else(|e| panic!("set {} NTC temperature: {e}", profile.name));
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
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

        let ntc_v = signal_voltage(&session, ntc_pad);
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "{} at {temperature_c} C: Beta divider on {ntc_pad} should be {expected_ntc_v} V, got {ntc_v} V",
            profile.name
        );
        let ntc_count = sample_sar1(&mut bus, ntc_channel);
        assert_eq!(
            ntc_count,
            expected_adc_count(ntc_v),
            "{}: SAR1 channel {ntc_channel} must report the solved NTC voltage",
            profile.name
        );

        let diode_v = signal_voltage(&session, diode_pad);
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "{}: Shockley load line on {diode_pad} should be {} V, got {diode_v} V",
            profile.name,
            diode_load_line_voltage()
        );
        assert_eq!(
            sample_sar1(&mut bus, diode_channel),
            expected_adc_count(diode_v),
            "{}: SAR1 channel {diode_channel} must report the solved diode voltage",
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
        "{}: SAR counts track the solved temperature-dependent voltage: {samples:?}",
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
fn esp32_analog_pin_table_reaches_sens_sar_on_every_classic_profile() {
    for profile in PROFILES {
        run_profile(profile);
    }
}

/// A pad the descriptor does not give an ADC input stays refused at binding
/// time. The table must not invent a channel for a digital pin.
#[test]
fn an_esp32_digital_pad_without_an_adc_pad_stays_refused() {
    let system_path = root("configs/systems/esp32-devkitm-1.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load the esp32-devkitm-1 profile");
    let deck = SystemManifest::from_file(root(GENERIC_DECK)).expect("load the generic analog deck");
    let mut models = deck.cosim_models;
    models[0].outputs.insert(
        "v_sense".to_string(),
        "board.analog.gpio23_volts".to_string(),
    );
    profile.cosim_models = models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the classic ESP32 descriptor");
    assert!(
        !chip.analog_pin_table().contains_key("GPIO23"),
        "GPIO23 is a digital pad; the map must not claim an ADC input"
    );

    let bus = SystemBus::from_config(&chip, &profile).expect("build the esp32-devkitm-1 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build the co-simulation")
        .expect("deck carries its analog device");
    assert_eq!(
        session.binding_errors(),
        &[RoutingError::NoAdcChannel {
            path: "board.analog.gpio23_volts".to_string(),
            pad: "gpio23".to_string(),
        }],
        "GPIO23 is not an ADC input on this part: the router must say so"
    );
}
