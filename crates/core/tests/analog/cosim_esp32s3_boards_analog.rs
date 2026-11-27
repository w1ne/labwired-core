// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every ESP32-S3 system profile solves the same generic NTC divider and
//! Shockley diode load line through its descriptor's ADC1 pad map. The deck
//! names only `board.analog.gpio4_volts` / `board.analog.gpio5_volts`; the ten
//! profiles below share `chips/esp32s3.yaml`, whose `analog_pins:` table
//! assigns GPIO1..GPIO10 to SENS SAR ADC1 channels 0..9. Nothing in the
//! solver, router or converter is S3-specific: the decks are the ones the
//! classic ESP32, C3, STM32, AVR and RP2040 boards already run.
//!
//! The conversion is the production `Esp32s3Sens` RTC one-shot path that
//! ESP-IDF `adc_oneshot_read` drives on the S3: write `SAR_MEAS1_CTRL2` with
//! `SAR1_EN_PAD = 1 << channel`, set `MEAS1_START_SAR`, poll `MEAS1_DONE_SAR`
//! and read the 12-bit `MEAS1_DATA_SAR`. The reset `SAR_ATTEN1` is 12 dB on
//! every channel, so the expected code is the S3 V1 curve-fit inverse at
//! ATTEN3, not a linear 3.3 V scale. The same pad level also converts on the
//! digital APB_SARADC controller, which shares the physical ADC1 pad bank.
//! Each profile is asserted separately, so an uncovered board cannot hide
//! behind a sibling that happens to be wired correctly.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::esp_xtensa_common::esp32s3_adc1_mv_to_raw;
use labwired_core::Bus;
use std::path::Path;

const SENS_BASE: u64 = 0x6000_8800;
const SAR_MEAS1_CTRL2: u64 = SENS_BASE + 0x00C;
const SAR1_EN_PAD_S: u32 = 19;
const MEAS_START_SAR: u32 = 1 << 17;
const MEAS_DONE_SAR: u32 = 1 << 16;
/// The SENS model's placeholder for an ADC1 channel no source has set.
const SENS_UNSET_SAMPLE: u32 = 0x800;

const APB_SARADC_BASE: u64 = 0x6004_0000;
const APB_ONETIME_SAMPLE: u64 = APB_SARADC_BASE + 0x80;
const APB_SARADC1_DATA_STATUS: u64 = APB_SARADC_BASE + 0x40;
const APB_INT_RAW: u64 = APB_SARADC_BASE + 0x60;
const APB_INT_CLR: u64 = APB_SARADC_BASE + 0x68;
const APB_ONETIME_SAMPLE1_EN: u32 = 1 << 31;
const APB_ONETIME_START: u32 = 1 << 29;
const APB_ADC1_DONE: u32 = 1 << 31;

const SUPPLY_V: f64 = 3.3;
const LIMIT_OHMS: f64 = 1_000.0;
const DIODE_IS_A: f64 = 2.52e-9;
const DIODE_N: f64 = 1.752;
const DIODE_RS_OHMS: f64 = 0.568;
const THERMAL_V: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

const DECK: &str = "examples/esp32s3-devkitc-1/analog-ntc-diode-system.yaml";

/// Board profiles that load the ESP32-S3 descriptor.
const PROFILES: [&str; 10] = [
    "configs/systems/arduino-nano-esp32-bme280.yaml",
    "configs/systems/arduino-nano-esp32.yaml",
    "configs/systems/esp32s3-devkitc-1-n16r8-bme280.yaml",
    "configs/systems/esp32s3-devkitc-1-v11-rgb.yaml",
    "configs/systems/esp32s3-devkitc-1-v11.yaml",
    "configs/systems/esp32s3-lcd-ev-board-v1-5-touch.yaml",
    "configs/systems/esp32s3-oled-demo.yaml",
    "configs/systems/esp32s3-zero.yaml",
    "configs/systems/esp32s3.yaml",
    "configs/systems/seeed-xiao-esp32s3.yaml",
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

/// One SENS SAR1 one-shot conversion of `channel`, as `adc_oneshot_read` runs
/// it through `adc_oneshot_ll_set_channel` / `_start` / `_get_raw_result`.
fn sample_sens_sar1(bus: &mut SystemBus, channel: u8) -> u32 {
    let en_pad = 1u32 << (SAR1_EN_PAD_S + u32::from(channel));
    bus.write_u32(SAR_MEAS1_CTRL2, en_pad).unwrap();
    bus.write_u32(SAR_MEAS1_CTRL2, en_pad | MEAS_START_SAR)
        .unwrap();
    let word = bus.read_u32(SAR_MEAS1_CTRL2).unwrap();
    assert_ne!(word & MEAS_DONE_SAR, 0, "MEAS1_DONE_SAR is raised");
    word & 0xFFFF
}

/// One APB_SARADC SAR1 one-time conversion of `channel`.
fn sample_apb_sar1(bus: &mut SystemBus, channel: u8) -> u32 {
    let word = APB_ONETIME_SAMPLE1_EN | (u32::from(channel) << 25) | APB_ONETIME_START;
    bus.write_u32(APB_ONETIME_SAMPLE, word).unwrap();
    assert_ne!(
        bus.read_u32(APB_INT_RAW).unwrap() & APB_ADC1_DONE,
        0,
        "APB_SARADC1_DONE is raised"
    );
    let data = bus.read_u32(APB_SARADC1_DATA_STATUS).unwrap();
    bus.write_u32(APB_INT_CLR, APB_ADC1_DONE).unwrap();
    assert_eq!(
        (data >> 13) & 0xF,
        u32::from(channel),
        "SARADC1_DATA_STATUS reports the converted channel"
    );
    data & 0x0FFF
}

/// The count the S3 model reports for a routed millivolt level at ATTEN3.
fn expected_adc_count(voltage_v: f64) -> u32 {
    let millivolts = (voltage_v * 1000.0).round() as u16;
    u32::from(esp32s3_adc1_mv_to_raw(millivolts, 3))
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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the ESP32-S3 descriptor");
    (profile, chip)
}

/// Load one profile, overlay the generic decks, and build the analog session.
fn bench(system: &str) -> (SystemBus, CosimSession) {
    let (profile, chip) = load(system);
    for channel in 0..10u8 {
        let pad = format!("GPIO{}", channel + 1);
        assert_eq!(
            chip.analog_pins[&pad].peripheral, "sens_s3",
            "{system}: {pad} must resolve to the SENS SAR ADC1"
        );
        assert_eq!(
            chip.analog_pins[&pad].channel, channel,
            "{system}: {pad} is ADC1 channel {channel} on the datasheet"
        );
    }
    assert_eq!(
        chip.analog_pins.len(),
        10,
        "{system}: the board map publishes exactly the ten ADC1 pads"
    );

    let bus = SystemBus::from_config(&chip, &profile)
        .unwrap_or_else(|error| panic!("build the bus for {system}: {error}"));
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries the selected analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{system} must resolve GPIO4/GPIO5 to the SENS SAR ADC1 without naming a channel"
    );
    (bus, session)
}

#[test]
fn every_esp32s3_profile_solves_generic_netlists_on_adc1_pads() {
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

            let ntc_v = signal_voltage(&session, "board.analog.gpio4_volts");
            let expected_ntc_v =
                SUPPLY_V * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
            assert!(
                (ntc_v - expected_ntc_v).abs() < 1e-9,
                "{system}: at {temperature_c} C the Beta divider on GPIO4 should be {expected_ntc_v} V, got {ntc_v} V"
            );

            let diode_v = signal_voltage(&session, "board.analog.gpio5_volts");
            assert!(
                (diode_v - expected_diode_v).abs() < 2e-5,
                "{system}: the Shockley load line on GPIO5 should be {expected_diode_v} V, got {diode_v} V"
            );

            let ntc_code = sample_sens_sar1(&mut bus, 3);
            let diode_code = sample_sens_sar1(&mut bus, 4);
            assert_eq!(
                ntc_code,
                expected_adc_count(ntc_v),
                "{system}: SENS ADC1 channel 3 (GPIO4) must report the solved NTC voltage"
            );
            assert_eq!(
                diode_code,
                expected_adc_count(diode_v),
                "{system}: SENS ADC1 channel 4 (GPIO5) must report the solved diode voltage"
            );
            assert_eq!(
                sample_apb_sar1(&mut bus, 3),
                ntc_code,
                "{system}: APB_SARADC reads the same physical GPIO4 pad level"
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
            u32::from(esp32s3_adc1_mv_to_raw(1_650, 3)),
            "{system}: the 25 C NTC divider is the 1650 mV ATTEN3 code"
        );
        // An unrouted ADC1 channel keeps the SENS mid-scale placeholder, so a
        // route that landed on the wrong channel would show up here as well
        // as in the codes above.
        assert_eq!(
            sample_sens_sar1(&mut bus, 2),
            SENS_UNSET_SAMPLE,
            "{system}: GPIO3 is not routed by the deck"
        );
    }
}

/// A pad outside the ADC1 table is refused rather than silently seeded on
/// another channel. GPIO48 (the DevKitC-1 v1.0 RGB LED pin) has no ADC
/// function and GPIO11 is ADC2_CH0 on silicon; neither is in the table.
#[test]
fn esp32s3_pad_map_refuses_pads_it_does_not_publish() {
    let (mut bus, session) = bench("configs/systems/esp32s3-devkitc-1-v11.yaml");
    assert!(
        !bus.set_analog_pad_mv("GPIO48", 1_650),
        "GPIO48 is digital only"
    );
    assert!(
        !bus.set_analog_pad_mv("GPIO11", 1_650),
        "ADC2 is not on the S3 board pad map"
    );
    assert!(
        !bus.set_analog_pad_mv("GPIO0", 1_650),
        "GPIO0 is a strapping pin with no ADC function"
    );
    assert_eq!(
        sample_sens_sar1(&mut bus, 0),
        SENS_UNSET_SAMPLE,
        "the refused GPIO11 level must not alias onto ADC1 channel 0 (GPIO1)"
    );
    assert!(bus.set_analog_pad_mv("GPIO1", 1_000), "GPIO1 is ADC1_CH0");
    assert_eq!(
        sample_sens_sar1(&mut bus, 0),
        u32::from(esp32s3_adc1_mv_to_raw(1_000, 3)),
        "GPIO1 is channel 0, the off-by-one pad the S3 map exists to get right"
    );
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the negative probes must not disturb the two valid routes"
    );
}

/// The refusal also happens on the deck route: a model that writes
/// `board.analog.gpio11_volts` gets a binding error at session build time
/// instead of a silently dropped netlist output.
#[test]
fn esp32s3_router_refuses_a_deck_route_to_an_adc2_pad() {
    let (mut profile, chip) = load("configs/systems/esp32s3-devkitc-1-v11.yaml");
    profile.cosim_models[0].outputs.insert(
        "v_adc2_probe".to_string(),
        "board.analog.gpio11_volts".to_string(),
    );
    let bus = SystemBus::from_config(&chip, &profile).expect("build the DevKitC-1 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries the selected analog devices");
    assert_eq!(
        session.binding_errors(),
        &[RoutingError::NoAdcChannel {
            path: "board.analog.gpio11_volts".to_string(),
            pad: "gpio11".to_string(),
        }],
        "GPIO11 is ADC2_CH0 on silicon but not on the board's ADC1 map"
    );
}
