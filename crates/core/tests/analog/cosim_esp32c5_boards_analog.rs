// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every ESP32-C5 system profile solves the same generic NTC divider and
//! Shockley diode load line through its descriptor's ADC1 pad map. The deck
//! names only `board.analog.gpio2_volts` / `board.analog.gpio4_volts`; both
//! profiles load `chips/esp32c5.yaml`, whose `analog_pins:` table assigns
//! GPIO1..GPIO5 to APB_SARADC ADC1 channels 0..4 (GPIO6 = ADC1_CH5 waits for a
//! per-chip model channel count). The pad is one above its channel, so a
//! router that assumed channel == GPIO would put GPIO2 on channel 2 and GPIO4
//! on channel 4; the unrouted-channel checks catch both. Nothing in the solver, router or converter is
//! C5-specific: the decks are the ones the ESP32-C3/C6/C61, classic ESP32,
//! STM32, AVR and RP2040 boards already run.
//!
//! The C5 `apb_saradc` is the C3 `Esp32c3ApbSarAdc` one-shot engine at the
//! C5 base address (the register heads are offset-identical, see
//! esp32c5.yaml). The conversion is the path ESP-IDF `adc_oneshot_read`
//! drives: program `ONETIME_SAMPLE` with the SAR1 unit bit, the channel and
//! `ONETIME_START`, poll `INT_RAW.APB_SARADC1_DONE`, read `SAR1DATA_STATUS`
//! (channel in [16:13], 12-bit sample in [12:0]) and acknowledge through
//! `INT_CLR`. Each profile is asserted separately.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

/// ESP32-C5 APB_SARADC base (esp32c5.yaml `apb_saradc`, reg_base.h
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

const DECK: &str = "examples/esp32c5-devkitc-1-n8r8/analog-ntc-diode-system.yaml";

/// Board profiles that load the ESP32-C5 descriptor.
const PROFILES: [&str; 2] = [
    "configs/systems/esp32c5-devkitc-1-n8r8.yaml",
    "configs/systems/esp32c5-devkitc-1-n8r8-bme280.yaml",
];

/// ADC1 channel -> pad, ESP-IDF esp32c5 `adc_channel.h`.
const ADC1_PADS: [u8; 5] = [1, 2, 3, 4, 5];

/// Deck pads and the channels the map must resolve them to.
const NTC_PAD_CHANNEL: u8 = 1; // GPIO2
const DIODE_PAD_CHANNEL: u8 = 3; // GPIO4

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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the ESP32-C5 descriptor");
    (profile, chip)
}

/// Load one profile, overlay the generic decks, and build the analog session.
fn bench(system: &str) -> (SystemBus, CosimSession) {
    let (profile, chip) = load(system);
    for (channel, gpio) in ADC1_PADS.iter().enumerate() {
        let pad = format!("GPIO{gpio}");
        assert_eq!(
            chip.analog_pins[&pad].peripheral, "apb_saradc",
            "{system}: {pad} must resolve to APB_SARADC"
        );
        assert_eq!(
            usize::from(chip.analog_pins[&pad].channel),
            channel,
            "{system}: {pad} is ADC1 channel {channel} on the C5"
        );
    }
    assert_eq!(
        chip.analog_pins.len(),
        5,
        "{system}: the board map publishes the five modelled ADC1 pads"
    );

    let bus = SystemBus::from_config(&chip, &profile)
        .unwrap_or_else(|error| panic!("build the bus for {system}: {error}"));
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries the selected analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{system} must resolve GPIO2/GPIO4 to APB_SARADC without naming a channel"
    );
    (bus, session)
}

#[test]
fn every_esp32c5_profile_solves_generic_netlists_on_adc1_pads() {
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

            let diode_v = signal_voltage(&session, "board.analog.gpio4_volts");
            assert!(
                (diode_v - expected_diode_v).abs() < 2e-5,
                "{system}: the Shockley load line on GPIO4 should be {expected_diode_v} V, got {diode_v} V"
            );

            let ntc_code = sample_sar1(&mut bus, NTC_PAD_CHANNEL);
            let diode_code = sample_sar1(&mut bus, DIODE_PAD_CHANNEL);
            assert_eq!(
                ntc_code,
                expected_adc_count(ntc_v),
                "{system}: ADC1 channel 1 (GPIO2) must report the solved NTC voltage"
            );
            assert_eq!(
                diode_code,
                expected_adc_count(diode_v),
                "{system}: ADC1 channel 3 (GPIO4) must report the solved diode voltage"
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
        // (0x100 + n * 0x111). Channels 2 (GPIO3) and 4 (GPIO5) are where a
        // channel == GPIO router would put GPIO2 and GPIO4.
        for (channel, pad) in [(0u8, "GPIO1"), (2, "GPIO3"), (4, "GPIO5")] {
            assert_eq!(
                sample_sar1(&mut bus, channel),
                0x100 + u32::from(channel) * 0x111,
                "{system}: {pad} is not routed by the deck"
            );
        }
    }
}

/// A pad outside the ADC1 table is refused rather than silently seeded on
/// another channel. GPIO0 and GPIO7 have no ADC function on the C5; GPIO8 is
/// the BME280 profile's SDA pad. A channel == GPIO router would accept GPIO0
/// on channel 0.
#[test]
fn esp32c5_pad_map_refuses_pads_it_does_not_publish() {
    let (mut bus, session) = bench("configs/systems/esp32c5-devkitc-1-n8r8.yaml");
    for pad in ["GPIO0", "GPIO7", "GPIO8"] {
        assert!(
            !bus.set_analog_pad_mv(pad, 1_650),
            "{pad} is not on the C5 board pad map"
        );
    }
    assert_eq!(
        sample_sar1(&mut bus, 0),
        0x100,
        "a refused GPIO0 level must not alias onto ADC1 channel 0 (GPIO1)"
    );
    assert!(
        bus.set_analog_pad_mv("GPIO3", 1_650),
        "GPIO3 is ADC1_CH2 on the C5"
    );
    assert_eq!(
        sample_sar1(&mut bus, 2),
        expected_adc_count(1.65),
        "a published pad seeds its own channel"
    );
    assert_eq!(
        sample_sar1(&mut bus, 4),
        0x544,
        "the GPIO3 level must not land on channel 4 (GPIO5)"
    );
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the negative probes must not disturb the two valid routes"
    );
}

/// The lowest and highest published pads bind through the router, and an
/// unpublished pad gets a binding error at session build time instead of a
/// silently dropped netlist output. The BME280 profile is the second C5
/// board.
#[test]
fn esp32c5_router_binds_published_pads_and_refuses_unpublished_ones() {
    let (mut profile, chip) = load("configs/systems/esp32c5-devkitc-1-n8r8-bme280.yaml");
    let outputs = &mut profile.cosim_models[0].outputs;
    outputs.insert("v_low".to_string(), "board.analog.gpio1_volts".to_string());
    outputs.insert("v_high".to_string(), "board.analog.gpio5_volts".to_string());
    outputs.insert(
        "v_gpio0".to_string(),
        "board.analog.gpio0_volts".to_string(),
    );
    outputs.insert(
        "v_gpio7".to_string(),
        "board.analog.gpio7_volts".to_string(),
    );
    let bus = SystemBus::from_config(&chip, &profile).expect("build the C5 BME280 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries the selected analog devices");
    assert_eq!(
        session.binding_errors(),
        &[
            RoutingError::NoAdcChannel {
                path: "board.analog.gpio0_volts".to_string(),
                pad: "gpio0".to_string(),
            },
            RoutingError::NoAdcChannel {
                path: "board.analog.gpio7_volts".to_string(),
                pad: "gpio7".to_string(),
            },
        ],
        "GPIO1 (ADC1_CH0) and GPIO5 (ADC1_CH4) bind; GPIO0 and GPIO7 have no C5 ADC input"
    );
}
