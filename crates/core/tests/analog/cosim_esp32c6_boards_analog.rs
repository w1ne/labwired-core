// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every ESP32-C6 system profile solves the same generic NTC divider and
//! Shockley diode load line through its descriptor's ADC1 pad map. The deck
//! names only `board.analog.gpio2_volts` / `board.analog.gpio3_volts`; the
//! five profiles below load `chips/esp32c6.yaml` or `chips/esp32c6fh4.yaml`,
//! whose `analog_pins:` tables assign GPIO0..GPIO6 to APB_SARADC ADC1 channels
//! 0..6. Nothing in the solver, router or converter is C6-specific: the decks
//! are the ones the ESP32-C3, classic ESP32, STM32, AVR and RP2040 boards
//! already run.
//!
//! The C6 `apb_saradc` is the C3 `Esp32c3ApbSarAdc` one-shot engine at the C6
//! base address (the register heads are offset-identical, see esp32c6.yaml).
//! The conversion is the path ESP-IDF `adc_oneshot_read` drives: program `ONETIME_SAMPLE` with the SAR1
//! unit bit, the channel and `ONETIME_START`, poll `INT_RAW.APB_SARADC1_DONE`,
//! read `SAR1DATA_STATUS` (channel in [16:13], 12-bit sample in [12:0]) and
//! acknowledge through `INT_CLR`. Each profile is asserted separately, so an
//! uncovered board cannot hide behind a sibling that happens to be wired
//! correctly.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

/// ESP32-C6 APB_SARADC base (esp32c6.yaml `apb_saradc`, reg_base.h
/// DR_REG_APB_SARADC_BASE). The C3 constant is 0x6004_0000.
const APB_SARADC_BASE: u32 = 0x6000_E000;
const ONETIME_SAMPLE: u64 = 0x20;
const SAR1DATA_STATUS: u64 = 0x2C;
const INT_RAW: u64 = 0x44;
const INT_CLR: u64 = 0x4C;
const SAR1_ONETIME_SAMPLE: u32 = 1 << 31;
const ONETIME_START: u32 = 1 << 29;
const SAR1_DONE: u32 = 1 << 31;

const SUPPLY_V: f64 = 3.3;
const LIMIT_OHMS: f64 = 1_000.0;
const DIODE_IS_A: f64 = 2.52e-9;
const DIODE_N: f64 = 1.752;
const DIODE_RS_OHMS: f64 = 0.568;
const THERMAL_V: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

const DECK: &str = "examples/esp32c6-devkitc/analog-ntc-diode-system.yaml";

/// Board profiles that load an ESP32-C6 descriptor (esp32c6 or esp32c6fh4).
const PROFILES: [&str; 5] = [
    "configs/systems/esp32c6-devkitc.yaml",
    "configs/systems/esp32c6-devkitc-bme280.yaml",
    "configs/systems/esp32c6-devkitc-1-n8-bme280.yaml",
    "configs/systems/esp32c6-zero-m-bme280.yaml",
    "configs/systems/seeed-xiao-esp32c6.yaml",
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

/// One SAR1 one-shot conversion of `channel`, as `adc_oneshot_read` runs it.
fn sample_sar1(bus: &mut SystemBus, channel: u8) -> u32 {
    let base = u64::from(APB_SARADC_BASE);
    let word = SAR1_ONETIME_SAMPLE | (u32::from(channel) << 25) | ONETIME_START;
    bus.write_u32(base + ONETIME_SAMPLE, word).unwrap();
    assert_ne!(
        bus.read_u32(base + INT_RAW).unwrap() & SAR1_DONE,
        0,
        "APB_SARADC1_DONE is raised"
    );
    let data = bus.read_u32(base + SAR1DATA_STATUS).unwrap();
    bus.write_u32(base + INT_CLR, SAR1_DONE).unwrap();
    assert_eq!(
        (data >> 13) & 0xF,
        u32::from(channel),
        "SAR1DATA_STATUS reports the converted channel"
    );
    data & 0x1FFF
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

fn load(system: &str) -> (SystemManifest, ChipDescriptor) {
    let system_path = root(system);
    let mut profile = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|error| panic!("load {system}: {error}"));
    let deck = SystemManifest::from_file(root(DECK))
        .expect("load the generic NTC divider and diode netlists");
    profile.cosim_models = deck.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the ESP32-C6 descriptor");
    (profile, chip)
}

/// Load one profile, overlay the generic decks, and build the analog session.
fn bench(system: &str) -> (SystemBus, CosimSession) {
    let (profile, chip) = load(system);
    for channel in 0..7u8 {
        let pad = format!("GPIO{channel}");
        assert_eq!(
            chip.analog_pins[&pad].peripheral, "apb_saradc",
            "{system}: {pad} must resolve to APB_SARADC"
        );
        assert_eq!(
            chip.analog_pins[&pad].channel, channel,
            "{system}: {pad} is ADC1 channel {channel} on the datasheet"
        );
    }
    assert_eq!(
        chip.analog_pins.len(),
        7,
        "{system}: the board map publishes exactly the seven ADC1 pads"
    );

    let bus = SystemBus::from_config(&chip, &profile)
        .unwrap_or_else(|error| panic!("build the bus for {system}: {error}"));
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries the selected analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{system} must resolve GPIO2/GPIO3 to APB_SARADC without naming a channel"
    );
    (bus, session)
}

#[test]
fn every_esp32c6_profile_solves_generic_netlists_on_adc1_pads() {
    let expected_diode_v = diode_load_line_voltage();

    for system in PROFILES {
        let (mut bus, mut session) = bench(system);
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

            let ntc_v = signal_voltage(&session, "board.analog.gpio2_volts");
            let expected_ntc_v =
                SUPPLY_V * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
            assert!(
                (ntc_v - expected_ntc_v).abs() < 1e-9,
                "{system}: at {temperature_c} C the Beta divider on GPIO2 should be {expected_ntc_v} V, got {ntc_v} V"
            );

            let diode_v = signal_voltage(&session, "board.analog.gpio3_volts");
            assert!(
                (diode_v - expected_diode_v).abs() < 2e-5,
                "{system}: the Shockley load line on GPIO3 should be {expected_diode_v} V, got {diode_v} V"
            );

            let ntc_code = sample_sar1(&mut bus, 2);
            let diode_code = sample_sar1(&mut bus, 3);
            assert_eq!(
                ntc_code,
                expected_adc_count(ntc_v),
                "{system}: ADC1 channel 2 (GPIO2) must report the solved NTC voltage"
            );
            assert_eq!(
                diode_code,
                expected_adc_count(diode_v),
                "{system}: ADC1 channel 3 (GPIO3) must report the solved diode voltage"
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
            samples[1].2, 2047,
            "{system}: the 25 C NTC divider is exactly half scale on 3.3 V Vref"
        );
        // An unrouted ADC1 channel keeps the model's fixed placeholder ramp
        // (0x100 + n * 0x111), so a route that landed on the wrong channel
        // would show up here as well as in the codes above. Channels 4 and 6
        // flank the deck and include the top C6 input, which the C3 lacks.
        assert_eq!(
            sample_sar1(&mut bus, 4),
            0x544,
            "{system}: GPIO4 is not routed by the deck"
        );
        assert_eq!(
            sample_sar1(&mut bus, 6),
            0x766,
            "{system}: GPIO6 is not routed by the deck"
        );
    }
}

/// A pad outside the ADC1 table is refused rather than silently seeded on
/// another channel. GPIO7 is the BME280 profiles' SCL pad and GPIO8 the
/// DevKitC-1 RGB LED data pin; neither has an ADC function on the C6.
#[test]
fn esp32c6_pad_map_refuses_pads_it_does_not_publish() {
    let (mut bus, session) = bench("configs/systems/esp32c6-devkitc.yaml");
    assert!(
        !bus.set_analog_pad_mv("GPIO7", 1_650),
        "GPIO7 is digital only"
    );
    assert!(
        !bus.set_analog_pad_mv("GPIO8", 1_650),
        "GPIO8 is digital only"
    );
    assert_eq!(
        sample_sar1(&mut bus, 0),
        0x100,
        "a refused level must not alias onto ADC1 channel 0 (GPIO0)"
    );
    // The top published pad does accept a level: the C6 map reaches past the
    // C3's five inputs.
    assert!(bus.set_analog_pad_mv("GPIO6", 1_650), "GPIO6 is ADC1_CH6");
    assert_eq!(
        sample_sar1(&mut bus, 6),
        1_650 * 4095 / 3300,
        "GPIO6 converts on ADC1 channel 6"
    );
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the negative probes must not disturb the two valid routes"
    );
}

/// The refusal also happens on the deck route: a model that writes
/// `board.analog.gpio7_volts` gets a binding error at session build time
/// instead of a silently dropped netlist output. The XIAO profile loads the
/// FH4 descriptor, so this also checks the second file.
#[test]
fn esp32c6_router_refuses_a_deck_route_to_a_digital_pad() {
    let (mut profile, chip) = load("configs/systems/seeed-xiao-esp32c6.yaml");
    profile.cosim_models[0].outputs.insert(
        "v_digital_probe".to_string(),
        "board.analog.gpio7_volts".to_string(),
    );
    let bus = SystemBus::from_config(&chip, &profile).expect("build the XIAO ESP32-C6 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries the selected analog devices");
    assert_eq!(
        session.binding_errors(),
        &[RoutingError::NoAdcChannel {
            path: "board.analog.gpio7_volts".to_string(),
            pad: "gpio7".to_string(),
        }],
        "GPIO7 has no ADC function on the ESP32-C6"
    );
}
