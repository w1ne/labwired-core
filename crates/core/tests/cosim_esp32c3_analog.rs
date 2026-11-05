// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists drive ESP32-C3 profiles through explicit APB_SARADC
//! channel routes and the firmware-visible one-shot path. The BLE Pong profile's
//! existing GP2Y0A21 sensor stays routed to ADC1_CH1.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::esp32c3::apb_saradc::Esp32c3ApbSarAdc;
use labwired_core::Bus;
use std::path::Path;

const APB_SARADC_BASE: u64 = 0x6004_0000;
const CHANNEL_SHIFT: u32 = 25;
const ONETIME_START: u32 = 1 << 29;
const SAR1_ONETIME_SAMPLE: u32 = 1 << 31;
const SAR1_DONE: u32 = 1 << 31;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load-line reference, including the junction's series
/// resistance. This catches wrong diode physics as well as misrouting.
fn diode_voltage() -> f64 {
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

fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("apb_saradc")
        .expect("the C3 profile declares APB_SARADC");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("APB_SARADC downcasts")
        .downcast_mut::<Esp32c3ApbSarAdc>()
        .expect("apb_saradc uses the C3 one-shot ADC model")
        .channel_input_count(channel)
}

/// Drive the model's ONETIME_SAMPLE register like IDF's ADC oneshot driver,
/// then check both the DONE handshake and packed channel/data result.
fn convert_channel(bus: &mut SystemBus, channel: u8) -> u16 {
    let sample = SAR1_ONETIME_SAMPLE | ONETIME_START | (u32::from(channel) << CHANNEL_SHIFT);
    bus.write_u32(APB_SARADC_BASE + 0x20, sample)
        .expect("start SAR1 one-shot conversion");
    let raw = bus
        .read_u32(APB_SARADC_BASE + 0x44)
        .expect("read APB_SARADC INT_RAW");
    assert_ne!(raw & SAR1_DONE, 0, "SAR1_DONE is latched");
    let data = bus
        .read_u32(APB_SARADC_BASE + 0x2C)
        .expect("read firmware-visible SAR1DATA_STATUS");
    assert_eq!(
        (data >> 13) & 0xF,
        u32::from(channel),
        "result names its ADC input"
    );
    (data & 0x0FFF) as u16
}

fn assert_generic_c3_analog_inputs(
    system_rel: &str,
    analog_rel: &str,
    expected_profile: &str,
    channel_one_has_board_source: bool,
) {
    let system_path = root(system_rel);
    let mut profile = SystemManifest::from_file(&system_path).expect("load ESP32-C3 profile");
    assert_eq!(profile.name, expected_profile);
    let analog_path = root(analog_rel);
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ESP32-C3 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build ESP32-C3 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries two analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both netlists must resolve to distinct APB_SARADC channels"
    );
    for channel in [0, 2, 3] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without a co-sim injection"
        );
    }
    let channel_one_count = adc_channel_count(&mut bus, 1);
    if channel_one_has_board_source {
        assert_ne!(
            channel_one_count, 0xFFFF,
            "the BLE Pong profile's GP2Y0A21 source seeds GPIO1 / ADC1_CH1"
        );
        assert_eq!(
            convert_channel(&mut bus, 1),
            channel_one_count,
            "APB_SARADC one-shot must read the onboard IR sensor input"
        );
    } else {
        assert_eq!(
            channel_one_count, 0xFFFF,
            "the DevKit profile has no source connected to ADC1_CH1"
        );
    }

    let expected_diode_v = diode_voltage();
    let mut cycles = 0;
    let mut ntc_samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("drive generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both device netlists");
        assert_eq!(steps.len(), 2, "both netlists cross their 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "adc.apb_saradc.0_volts");
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "adc.apb_saradc.2_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        for (channel, volts) in [(0, ntc_v), (2, diode_v)] {
            let millivolts = (volts * 1000.0 + 0.5).floor() as u32;
            let expected_count = (millivolts * 4095 / 3300) as u16;
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                convert_channel(&mut bus, channel),
                expected_count,
                "the C3 one-shot result must report the solved channel {channel} voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 1),
            channel_one_count,
            "generic netlists on ADC1_CH0/CH2 leave ADC1_CH1 unchanged"
        );
        assert_eq!(
            adc_channel_count(&mut bus, 3),
            0xFFFF,
            "adjacent GPIO3 / ADC1_CH3 remains undriven"
        );
        ntc_samples.push((ntc_v, adc_channel_count(&mut bus, 0)));
    }

    assert!(
        ntc_samples[2].0 > ntc_samples[1].0 && ntc_samples[1].0 > ntc_samples[0].0,
        "the NTC divider voltage rises with temperature: {ntc_samples:?}"
    );
    assert!(
        ntc_samples[2].1 > ntc_samples[1].1 && ntc_samples[1].1 > ntc_samples[0].1,
        "ADC result counts follow the solved NTC voltage: {ntc_samples:?}"
    );
    assert_eq!(
        ntc_samples[1].1, 2047,
        "25 °C produces the matched half-scale divider"
    );
}

#[test]
fn esp32c3_ble_pong_ntc_and_diode_netlists_reach_adc1_channels() {
    assert_generic_c3_analog_inputs(
        "configs/systems/esp32c3-ble-pong.yaml",
        "examples/esp32c3-ble-pong/analog-ntc-diode-system.yaml",
        "esp32c3-ble-pong",
        true,
    );
}

#[test]
fn esp32c3_devkit_ntc_and_diode_netlists_reach_adc1_channels() {
    assert_generic_c3_analog_inputs(
        "configs/systems/esp32c3-devkit.yaml",
        "examples/esp32c3-devkit/analog-ntc-diode-system.yaml",
        "esp32c3-devkit",
        false,
    );
}

#[test]
fn esp32c3_epaper_workshop_ntc_and_diode_netlists_reach_adc1_channels() {
    assert_generic_c3_analog_inputs(
        "configs/systems/esp32c3-epaper-workshop.yaml",
        "examples/esp32c3-devkit/analog-ntc-diode-system.yaml",
        "esp32c3-epaper-workshop",
        false,
    );
}

#[test]
fn esp32c3_jtag_watch_ntc_and_diode_netlists_reach_adc1_channels() {
    assert_generic_c3_analog_inputs(
        "configs/systems/esp32c3-jtag-watch.yaml",
        "examples/esp32c3-devkit/analog-ntc-diode-system.yaml",
        "esp32c3-jtag-watch",
        false,
    );
}

#[test]
fn esp32c3_mlx90640_thermal_ntc_and_diode_netlists_reach_adc1_channels() {
    assert_generic_c3_analog_inputs(
        "configs/systems/esp32c3-mlx90640-thermal.yaml",
        "examples/esp32c3-devkit/analog-ntc-diode-system.yaml",
        "esp32c3-mlx90640-thermal",
        false,
    );
}
