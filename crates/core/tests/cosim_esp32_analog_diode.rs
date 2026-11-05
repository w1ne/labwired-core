// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A nonlinear diode circuit drives the Adafruit Feather ESP32 V2's classic
//! ESP32 SENS SAR ADC through the generic named-channel route. The test checks
//! the solved load line, the injected channel, and the firmware-visible
//! one-shot conversion registers.

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
