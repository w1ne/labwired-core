// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Analog circuits drive classic ESP32 SENS SAR ADC channels through the
//! generic named-channel route. The tests check a nonlinear diode on the
//! Feather V2 and a Beta NTC divider on the WROOM-32, including the solved
//! circuit, injected channel and one-shot conversion registers.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::esp32::sar_adc::{Esp32SarAdc, SENS_BASE};
use labwired_core::Bus;
use std::path::Path;

const CHANNEL: u8 = 6;
const SUPPLY_V: f64 = 3.3;
const LIMIT_OHMS: f64 = 1_000.0;
const DIODE_IS_A: f64 = 2.52e-9;
const DIODE_N: f64 = 1.752;
const DIODE_RS_OHMS: f64 = 0.568;
const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;

/// Independent Shockley load-line reference, including junction series
/// resistance. This catches a missing/nonlinear diode model and incorrect
/// probe routing rather than merely checking that an ADC slot changed.
fn expected_diode_voltage() -> f64 {
    let current_at_junction_voltage =
        |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let current = current_at_junction_voltage(junction_v);
        (SUPPLY_V - junction_v - current * DIODE_RS_OHMS) / LIMIT_OHMS - current
    };

    let mut low = 0.0;
    let mut high = 1.2;
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

#[test]
fn feather_esp32_diode_netlist_reaches_sens_sar_channel_and_one_shot_result() {
    let system_path = root("examples/adafruit-feather-esp32-v2/analog-diode-system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load Feather ESP32 fixture");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load classic ESP32 descriptor");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build Feather ESP32 bus");
    let mut session = CosimSession::new(&manifest.cosim_models, Path::new("."), &bus)
        .expect("build analog session")
        .expect("manifest declares a diode netlist");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the diode output must resolve to SENS SAR ADC channel {CHANNEL}"
    );

    let boundary = session.cycles_until_boundary(0);
    let (steps, errors) = session
        .advance_to(boundary, &mut bus)
        .expect("solve the diode circuit");
    assert_eq!(steps.len(), 1, "one 1 us analog boundary is crossed");
    assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

    let routed_voltage = match session.signals().get("adc.sens_sar_adc.6_volts") {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("the diode voltage was not routed to SENS SAR: {other:?}"),
    };
    let expected_voltage = expected_diode_voltage();
    assert!(
        (routed_voltage - expected_voltage).abs() < 2e-5,
        "the solved nonlinear load line should be {expected_voltage} V, got {routed_voltage} V"
    );

    let adc_index = bus
        .find_peripheral_index_by_name("sens_sar_adc")
        .expect("the board declares the classic ESP32 SENS SAR ADC");
    let adc = bus.peripherals[adc_index]
        .dev
        .as_any_mut()
        .expect("SENS SAR downcasts")
        .downcast_mut::<Esp32SarAdc>()
        .expect("sens_sar_adc uses the classic ESP32 SAR model");
    let expected_mv = (expected_voltage * 1000.0).round() as u16;
    let expected_count = (u32::from(expected_mv) * 4095 / 3300) as u16;
    assert_eq!(
        adc.channel_input_count(CHANNEL),
        expected_count,
        "the MNA result must seed the selected SENS SAR input"
    );
    assert_eq!(
        adc.channel_input_count(CHANNEL - 1),
        0xFFFF,
        "the neighboring channel must remain undriven"
    );

    // SENS_SAR_READ_CTRL.SAR1_SAMPLE_BIT = 0b11 selects 12-bit conversion.
    bus.write_u32(u64::from(SENS_BASE), 3 << 16)
        .expect("select 12-bit SAR1 conversion");
    // SAR_MEAS_START1: EN_PAD is one-hot at CHANNEL, force is set, then SAR starts.
    let start = (1u32 << (19 + u32::from(CHANNEL))) | (1 << 18) | (1 << 17);
    bus.write_u32(u64::from(SENS_BASE) + 0x54, start)
        .expect("start SAR1 conversion on the routed channel");
    let result = bus
        .read_u32(u64::from(SENS_BASE) + 0x54)
        .expect("read firmware-visible SAR1 result");
    assert_ne!(result & (1 << 16), 0, "DONE_SAR is raised");
    assert_eq!(
        result & 0x0FFF,
        u32::from(expected_count),
        "SAR1 DATA_SAR must report the diode's routed voltage"
    );
}

/// The same in-core analog solver and SENS SAR route also carry a different
/// device model on the ESP32-WROOM-32 system profile. The independent Beta
/// equation catches a route that merely changes the channel slot without
/// solving the NTC divider's temperature-dependent resistance.
#[test]
fn esp32_wroom_32_ntc_netlist_reaches_sens_sar_channel_and_one_shot_result() {
    let profile_path = root("configs/systems/esp32-wroom-32.yaml");
    let mut profile = SystemManifest::from_file(&profile_path).expect("load WROOM-32 profile");
    assert_eq!(profile.name, "esp32-wroom-32");
    let analog_path = root("examples/esp32-wroom-32/analog-ntc-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load WROOM-32 NTC fixture");
    profile.cosim_models = analog.cosim_models;
    let chip_path = profile_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load classic ESP32 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build WROOM-32 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build NTC analog session")
        .expect("profile carries the NTC model");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the NTC output must resolve to classic ESP32 SENS SAR channel 6"
    );

    let adc_index = bus
        .find_peripheral_index_by_name("sens_sar_adc")
        .expect("the WROOM profile declares the classic ESP32 SENS SAR ADC");
    let adc = bus.peripherals[adc_index]
        .dev
        .as_any_mut()
        .expect("SENS SAR downcasts")
        .downcast_mut::<Esp32SarAdc>()
        .expect("sens_sar_adc uses the classic ESP32 SAR model");
    assert_eq!(
        adc.channel_input_count(6),
        0xFFFF,
        "channel 6 starts undriven"
    );
    assert_eq!(
        adc.channel_input_count(7),
        0xFFFF,
        "neighbor channel stays undriven"
    );

    let mut cycles = 0;
    let mut samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("the NTC resistor reads the temperature input");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the NTC divider");
        assert_eq!(steps.len(), 1, "one 1 us analog boundary is crossed");
        assert!(errors.is_empty(), "unexpected routing errors: {errors:?}");

        let routed_voltage = match session.signals().get("adc.sens_sar_adc.6_volts") {
            Some(CosimSignalValue::F64(volts)) => *volts,
            other => panic!("the NTC voltage did not reach SENS SAR channel 6: {other:?}"),
        };
        let ntc_ohms = 10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp();
        let expected_voltage = 3.3 * 10_000.0 / (ntc_ohms + 10_000.0);
        assert!(
            (routed_voltage - expected_voltage).abs() < 1e-9,
            "at {temperature_c} °C the solved divider should be {expected_voltage} V, got {routed_voltage} V"
        );

        let expected_mv = (expected_voltage * 1000.0 + 0.5).floor() as u16;
        let expected_count = (u32::from(expected_mv) * 4095 / 3300) as u16;
        let adc = bus.peripherals[adc_index]
            .dev
            .as_any_mut()
            .expect("SENS SAR downcasts")
            .downcast_mut::<Esp32SarAdc>()
            .expect("sens_sar_adc uses the classic ESP32 SAR model");
        assert_eq!(
            adc.channel_input_count(6),
            expected_count,
            "the solved voltage must seed only the selected SENS SAR input"
        );
        assert_eq!(
            adc.channel_input_count(7),
            0xFFFF,
            "channel 7 remains undriven"
        );

        // Select 12-bit SAR1 conversion, then start the one-hot channel-6 input.
        bus.write_u32(u64::from(SENS_BASE), 3 << 16)
            .expect("select 12-bit SAR1 conversion");
        let start = (1u32 << (19 + 6)) | (1 << 18) | (1 << 17);
        bus.write_u32(u64::from(SENS_BASE) + 0x54, start)
            .expect("start SAR1 conversion on channel 6");
        let result = bus
            .read_u32(u64::from(SENS_BASE) + 0x54)
            .expect("read firmware-visible SAR1 result");
        assert_ne!(result & (1 << 16), 0, "DONE_SAR is raised");
        assert_eq!(
            result & 0x0FFF,
            u32::from(expected_count),
            "SAR1 DATA_SAR must report the routed NTC voltage at {temperature_c} °C"
        );
        samples.push((temperature_c, routed_voltage, expected_count));
    }

    assert!(
        samples[2].1 > samples[1].1 && samples[1].1 > samples[0].1,
        "the NTC divider voltage rises as it heats: {samples:?}"
    );
    assert!(
        samples[2].2 > samples[1].2 && samples[1].2 > samples[0].2,
        "the firmware-visible SAR result follows the NTC voltage: {samples:?}"
    );
    assert_eq!(
        samples[1].2, 2047,
        "25 °C puts the matched divider at half scale"
    );
}
