// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every classic-ESP32 system profile solves the same generic NTC divider and
//! Shockley diode load line through its descriptor's ADC1 pad map. The decks
//! name only `board.analog.gpio34_volts` / `board.analog.gpio35_volts`; the
//! six profiles below share `chips/esp32.yaml`, whose new `analog_pins:` table
//! assigns GPIO34/GPIO35 to SENS SAR1 channels 6/7. Nothing in the solver,
//! router or converter is ESP32-specific, so this test proves the pad map is
//! per-part data like the STM32, AVR, WCH, nRF52, micro:bit, RP2040 and RA
//! descriptors already used by the same decks.
//!
//! The conversion is the production `Esp32SarAdc` one-shot path: program
//! `SAR_READ_CTRL` to 12 bits, strobe the channel in `SAR_MEAS_START1`, then
//! read `MEAS1_DATA_SAR`. Each profile is asserted separately, so an uncovered
//! board cannot hide behind a sibling that happens to be wired correctly.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::esp32::sar_adc::SENS_BASE;
use labwired_core::Bus;
use std::path::Path;

const SUPPLY_V: f64 = 3.3;
const LIMIT_OHMS: f64 = 1_000.0;
const DIODE_IS_A: f64 = 2.52e-9;
const DIODE_N: f64 = 1.752;
const DIODE_RS_OHMS: f64 = 0.568;
const THERMAL_V: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

/// Board profiles that load the classic-ESP32 descriptor.
const PROFILES: [&str; 6] = [
    "configs/systems/esp32-devkitm-1.yaml",
    "configs/systems/esp32-devkitm-1-bme280.yaml",
    "configs/systems/esp32-wroom-32.yaml",
    "configs/systems/esp32-wroom-epaper.yaml",
    "configs/systems/adafruit-feather-esp32-v2.yaml",
    "configs/systems/adafruit-feather-esp32-v2-bme280.yaml",
];

/// Beta-parameter NTC resistance, the model card in the fixture.
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

/// One 12-bit software-started SAR1 conversion of `channel`.
fn sample_sar1(bus: &mut SystemBus, channel: u8) -> u32 {
    bus.write_u32(u64::from(SENS_BASE), 3 << 16).unwrap();
    let start = (1u32 << (19 + u32::from(channel))) | (1 << 18) | (1 << 17);
    bus.write_u32(u64::from(SENS_BASE) + 0x54, start).unwrap();
    let result = bus.read_u32(u64::from(SENS_BASE) + 0x54).unwrap();
    assert_ne!(result & (1 << 16), 0, "DONE_SAR is raised");
    result & 0x0FFF
}

/// The count the model reports for a routed millivolt level on 3.3 V Vref.
fn expected_adc_count(voltage_v: f64) -> u32 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    millivolts * 4095 / 3300
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// Load one profile, overlay the generic decks, and build the analog session.
fn bench(system: &str) -> (ChipDescriptor, SystemBus, CosimSession) {
    let system_path = root(system);
    let mut profile = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|error| panic!("load {system}: {error}"));

    let deck = SystemManifest::from_file(root(
        "examples/esp32-devkitm-1/analog-ntc-diode-system.yaml",
    ))
    .expect("load the generic NTC divider and diode netlists");
    profile.cosim_models = deck.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the classic ESP32 descriptor");
    assert_eq!(chip.analog_pins["GPIO34"].peripheral, "sens_sar_adc");
    assert_eq!(chip.analog_pins["GPIO34"].channel, 6);
    assert_eq!(chip.analog_pins["GPIO35"].channel, 7);
    assert_eq!(chip.analog_pins["GPIO32"].channel, 4);
    assert_eq!(chip.analog_pins["GPIO39"].channel, 3);
    assert!(
        !chip.analog_pins.contains_key("GPIO25"),
        "ADC2 stays off the board pad map; it is unusable while WiFi runs"
    );

    let bus = SystemBus::from_config(&chip, &profile)
        .unwrap_or_else(|error| panic!("build the bus for {system}: {error}"));
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries the selected analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{system} must resolve GPIO34/GPIO35 to SENS SAR1 without naming a channel"
    );
    (chip, bus, session)
}

#[test]
fn every_classic_esp32_profile_solves_generic_netlists_on_adc1_pads() {
    let expected_diode_v = diode_load_line_voltage();

    for system in PROFILES {
        let (chip, mut bus, mut session) = bench(system);
        let mut cycles = 0;
        let mut samples = Vec::new();

        for temperature_c in [-20.0_f64, 25.0, 85.0] {
            session
                .set_signal_number("ui.ntc.temperature_c", temperature_c)
                .expect("set generic NTC temperature");
            cycles += session.cycles_until_boundary(cycles);
            let (steps, errors) = session
                .advance_to(cycles, &mut bus)
                .unwrap_or_else(|error| panic!("solve {system}: {error}"));
            assert_eq!(steps.len(), 2, "{system}: both netlists cross the boundary");
            assert!(
                errors.is_empty(),
                "{system}: unexpected route errors: {errors:?}"
            );

            let ntc_v = signal_voltage(&session, "board.analog.gpio34_volts");
            let expected_ntc_v =
                SUPPLY_V * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
            assert!(
                (ntc_v - expected_ntc_v).abs() < 1e-9,
                "{system}: at {temperature_c} C the Beta divider on GPIO34 should be {expected_ntc_v} V, got {ntc_v} V"
            );

            let diode_v = signal_voltage(&session, "board.analog.gpio35_volts");
            assert!(
                (diode_v - expected_diode_v).abs() < 2e-5,
                "{system}: the Shockley load line on GPIO35 should be {expected_diode_v} V, got {diode_v} V"
            );

            let ntc_code = sample_sar1(&mut bus, 6);
            let diode_code = sample_sar1(&mut bus, 7);
            assert_eq!(
                ntc_code,
                expected_adc_count(ntc_v),
                "{system}: SENS SAR1 channel 6 (GPIO34) must report the solved NTC voltage"
            );
            assert_eq!(
                diode_code,
                expected_adc_count(diode_v),
                "{system}: SENS SAR1 channel 7 (GPIO35) must report the solved diode voltage"
            );

            samples.push((temperature_c, ntc_v, ntc_code));
        }

        assert!(
            samples[2].1 > samples[1].1 && samples[1].1 > samples[0].1,
            "{system}: the divider voltage rises with NTC temperature: {samples:?}"
        );
        assert!(
            samples[2].2 > samples[1].2 && samples[1].2 > samples[0].2,
            "{system}: ADC counts track the solved temperature-dependent voltage: {samples:?}"
        );
        assert_eq!(
            samples[1].2,
            expected_adc_count(SUPPLY_V / 2.0),
            "{system}: the 25 C NTC divider is exactly half scale on 3.3 V Vref"
        );

        assert!(
            chip.analog_pins.len() == 8,
            "{system}: only the eight ADC1 pads are published, got {}",
            chip.analog_pins.len()
        );
    }
}

/// A pad outside the ADC1 table is refused rather than silently seeded on
/// channel 0, and the pad map never reaches ADC2. GPIO23 is the DevKitM-1
/// header LED and GPIO25 is ADC2_CH8 on silicon, but neither is in the table.
#[test]
fn classic_esp32_pad_map_refuses_pads_it_does_not_publish() {
    let (_chip, mut bus, session) = bench("configs/systems/esp32-devkitm-1.yaml");
    assert!(
        !bus.set_analog_pad_mv("GPIO23", 1_650),
        "GPIO23 is digital only"
    );
    assert!(
        !bus.set_analog_pad_mv("GPIO25", 1_650),
        "ADC2 is not on the board pad map"
    );
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the negative probes must not disturb the two valid routes"
    );
}
