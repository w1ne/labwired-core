// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog device netlists drive ESP32-S3 profiles through their SENS
//! ADC1 channels and firmware-visible one-shot conversion registers.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::esp32s3::sens::Esp32s3Sens;
use labwired_core::system::xtensa::{
    attach_esp32_external_devices, configure_xtensa_esp32s3, Esp32s3Opts,
};
use labwired_core::Bus;
use std::path::Path;

const SENS_MEAS1_CTRL2: u64 = 0x0C;
const MEAS_START_SAR: u32 = 1 << 17;
const MEAS_DONE_SAR: u32 = 1 << 16;
const SAR1_EN_PAD_SHIFT: u32 = 19;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load-line reference, including junction series resistance.
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
        .find_peripheral_index_by_name("sens_s3")
        .expect("the S3 profile declares SENS");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("SENS downcasts")
        .downcast_mut::<Esp32s3Sens>()
        .expect("sens_s3 uses the ESP32-S3 SENS model")
        .channel_input_count(channel)
}

/// Run the SENS ADC1 one-shot for a selected GPIO1..GPIO10 channel and return
/// its low 12-bit firmware-visible result.
fn convert_channel(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("sens_s3")
        .expect("the S3 profile declares SENS");
    let base = bus.peripherals[index].base;
    let selection = 1u32 << (SAR1_EN_PAD_SHIFT + u32::from(channel));
    bus.write_u32(base + SENS_MEAS1_CTRL2, selection)
        .expect("clear SENS SAR1 one-shot");
    bus.write_u32(base + SENS_MEAS1_CTRL2, selection | MEAS_START_SAR)
        .expect("start SENS SAR1 one-shot conversion");
    let result = bus
        .read_u32(base + SENS_MEAS1_CTRL2)
        .expect("read SENS SAR1 one-shot result");
    assert_ne!(result & MEAS_DONE_SAR, 0, "SENS SAR1_DONE is latched");
    (result & 0x0FFF) as u16
}

fn expected_adc_count(volts: f64) -> u16 {
    let millivolts = (volts * 1000.0).round() as u32;
    (millivolts * 4095 / 3300).min(4095) as u16
}

fn assert_s3_profile_analog(
    profile_name: &str,
    analog_fixture: &str,
    expected_external_device: Option<&str>,
    expect_zero_status_led: bool,
) {
    let system_path = root(&format!("configs/systems/{profile_name}.yaml"));
    let mut profile = SystemManifest::from_file(&system_path).expect("load ESP32-S3 profile");
    assert_eq!(profile.name, profile_name);
    if let Some(device_id) = expected_external_device {
        assert!(
            profile
                .external_devices
                .iter()
                .any(|device| device.id == device_id),
            "analog coverage must retain external device {device_id}"
        );
    }
    if expect_zero_status_led {
        assert!(
            profile
                .board_io
                .iter()
                .any(|io| io.id == "status_led" && io.pin == 48),
            "analog coverage must retain the ESP32-S3-Zero GPIO48 status LED"
        );
    }
    let analog_path = root(analog_fixture);
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ESP32-S3 descriptor");
    let mut bus = SystemBus::new();
    configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::for_chip(&chip));
    attach_esp32_external_devices(&mut bus, &profile)
        .expect("attach the profile's external devices to the S3 controllers");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries two analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both netlists must resolve to distinct SENS ADC1 channels"
    );
    for channel in [0, 1, 2, 3] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "SENS ADC1 channel {channel} starts without co-sim injection"
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

        let ntc_v = signal_voltage(&session, "adc.sens_s3.0_volts");
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "adc.sens_s3.2_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        for (channel, volts) in [(0, ntc_v), (2, diode_v)] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed SENS ADC1 channel {channel}"
            );
            assert_eq!(
                convert_channel(&mut bus, channel),
                expected_count,
                "the SENS one-shot result must report the solved channel {channel} voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 1),
            0xFFFF,
            "the generic netlists leave adjacent SENS ADC1 channel 1 undriven"
        );
        assert_eq!(
            adc_channel_count(&mut bus, 3),
            0xFFFF,
            "the generic netlists leave adjacent SENS ADC1 channel 3 undriven"
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
fn esp32s3_oled_demo_ntc_and_diode_netlists_reach_sens_adc1() {
    assert_s3_profile_analog(
        "esp32s3-oled-demo",
        "examples/esp32s3-oled-demo/analog-ntc-diode-system.yaml",
        Some("oled"),
        false,
    );
}

#[test]
fn esp32s3_zero_ntc_and_diode_netlists_reach_sens_adc1() {
    assert_s3_profile_analog(
        "esp32s3-zero",
        "examples/esp32s3-zero/analog-ntc-diode-system.yaml",
        None,
        true,
    );
}

#[test]
fn esp32s3_ntc_and_diode_netlists_reach_sens_adc1() {
    assert_s3_profile_analog(
        "esp32s3",
        "examples/esp32s3-oled-demo/analog-ntc-diode-system.yaml",
        None,
        false,
    );
}
