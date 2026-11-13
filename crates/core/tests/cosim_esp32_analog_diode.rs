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

fn adc_channel_input_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let adc_index = bus
        .find_peripheral_index_by_name("sens_sar_adc")
        .expect("the classic ESP32 profile declares the SENS SAR ADC");
    let adc = bus.peripherals[adc_index]
        .dev
        .as_any_mut()
        .expect("SENS SAR downcasts")
        .downcast_mut::<Esp32SarAdc>()
        .expect("sens_sar_adc uses the classic ESP32 SAR model");
    adc.channel_input_count(channel)
}

fn convert_adc1_channel(bus: &mut SystemBus, channel: u8) -> u16 {
    // SENS_SAR_READ_CTRL.SAR1_SAMPLE_BIT = 0b11 selects 12-bit conversion.
    bus.write_u32(u64::from(SENS_BASE), 3 << 16)
        .expect("select 12-bit SAR1 conversion");
    // SAR_MEAS_START1: EN_PAD is one-hot at CHANNEL, force is set, then SAR starts.
    let start = (1u32 << (19 + u32::from(channel))) | (1 << 18) | (1 << 17);
    bus.write_u32(u64::from(SENS_BASE) + 0x54, start)
        .expect("start SAR1 conversion on the routed channel");
    let result = bus
        .read_u32(u64::from(SENS_BASE) + 0x54)
        .expect("read firmware-visible SAR1 result");
    assert_ne!(result & (1 << 16), 0, "DONE_SAR is raised");
    (result & 0x0FFF) as u16
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

/// The e-paper system profile uses the same classic ESP32 ADC routing as the
/// bare WROOM profile. Exercise the profile itself with two unrelated generic
/// circuits so coverage proves both nonlinear device physics and channel
/// isolation through the one-shot conversion registers.
#[test]
fn esp32_wroom_epaper_ntc_and_diode_reach_distinct_sens_adc_channels() {
    let profile_path = root("configs/systems/esp32-wroom-epaper.yaml");
    let mut profile = SystemManifest::from_file(&profile_path).expect("load e-paper profile");
    assert_eq!(profile.name, "esp32-wroom-epaper");
    let analog_path = root("examples/esp32-wroom-epaper/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic analog fixture");
    profile.cosim_models = analog.cosim_models;

    let chip_path = profile_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load classic ESP32 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build e-paper ESP32 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries generic NTC and diode models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the NTC and diode must bind to distinct SENS SAR channels"
    );
    assert_eq!(adc_channel_input_count(&mut bus, 6), 0xFFFF);
    assert_eq!(adc_channel_input_count(&mut bus, 7), 0xFFFF);
    assert_eq!(adc_channel_input_count(&mut bus, 5), 0xFFFF);

    let expected_diode_v = expected_diode_voltage();
    let mut cycles = 0;
    let mut ntc_samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature input");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve both analog circuits and route their outputs");
        assert_eq!(steps.len(), 2, "both devices cross the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = match session.signals().get("adc.sens_sar_adc.6_volts") {
            Some(CosimSignalValue::F64(volts)) => *volts,
            other => panic!("the NTC voltage did not reach ADC1 channel 6: {other:?}"),
        };
        let ntc_ohms = 10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp();
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_ohms + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = match session.signals().get("adc.sens_sar_adc.7_volts") {
            Some(CosimSignalValue::F64(volts)) => *volts,
            other => panic!("the diode voltage did not reach ADC1 channel 7: {other:?}"),
        };
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        for (channel, voltage) in [(6, ntc_v), (7, diode_v)] {
            let expected_mv = (voltage * 1000.0 + 0.5).floor() as u16;
            let expected_count = (u32::from(expected_mv) * 4095 / 3300) as u16;
            assert_eq!(
                adc_channel_input_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed only the selected ADC channel"
            );
            assert_eq!(
                convert_adc1_channel(&mut bus, channel),
                expected_count,
                "the firmware-visible one-shot conversion must report channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_input_count(&mut bus, 5),
            0xFFFF,
            "the adjacent undriven channel remains untouched"
        );
        ntc_samples.push((ntc_v, adc_channel_input_count(&mut bus, 6)));
    }

    assert!(
        ntc_samples[2].0 > ntc_samples[1].0 && ntc_samples[1].0 > ntc_samples[0].0,
        "the NTC divider voltage rises as it heats: {ntc_samples:?}"
    );
    assert!(
        ntc_samples[2].1 > ntc_samples[1].1 && ntc_samples[1].1 > ntc_samples[0].1,
        "the ADC result follows the temperature-dependent divider: {ntc_samples:?}"
    );
    assert_eq!(
        ntc_samples[1].1, 2047,
        "25 °C puts the divider at half scale"
    );
}

/// Reverse pn-junction law of one SPICE device family, as ngspice-42 writes
/// it, with the junction current in amps for a junction voltage `v` < 0.
#[derive(Clone, Copy, Debug)]
enum ReverseLaw {
    /// `dioload.c`/`jfetload.c`: cubic below `−3·N·Vt`.
    Cubic,
    /// `mos1load.c`: `−IS` held at or below `−3·Vt`.
    Hold,
    /// The plain exponential the engine used before, as a control.
    Exponential,
}

/// Independent node voltage of a leakage circuit: `junctions` parallel
/// junctions of saturation current `is_a` (N = 1, engine temperature 300 K)
/// held in reverse by a 1.65 V bias, their leakage flowing into `load_ohms`.
fn expected_leakage_voltage(law: ReverseLaw, is_a: f64, junctions: f64, load_ohms: f64) -> f64 {
    const BIAS_V: f64 = 1.65;
    const GMIN_S: f64 = 1e-12;
    let vt = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;
    // Reverse current, cathode to anode, of one junction at voltage v.
    let reverse_current = |v: f64| {
        let junction = match law {
            ReverseLaw::Cubic if v < -3.0 * vt => {
                let arg = 3.0 * vt / (v * std::f64::consts::E);
                is_a * (1.0 + arg * arg * arg)
            }
            ReverseLaw::Hold if v <= -3.0 * vt => is_a,
            _ => -is_a * ((v / vt).exp() - 1.0),
        };
        junction - GMIN_S * v
    };
    let (mut low, mut high) = (0.0, BIAS_V);
    for _ in 0..200 {
        let mid = 0.5 * (low + high);
        if junctions * reverse_current(mid - BIAS_V) * load_ohms > mid {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

/// Reverse-biased junction leakage of three unrelated device families reaches
/// SENS SAR ADC1 on the Feather V2 profile beside the diode load line. In
/// this region the diode and JFET gate take the cubic reverse form and the
/// MOSFET bulk junction holds `−IS`; each is checked against an independent
/// root, ngspice-42 `.op` (`.temp 26.85`, `tnom=26.85`), and a 12-bit
/// one-shot conversion whose code differs from the plain exponential's.
#[test]
fn feather_esp32_reverse_junction_leakage_reaches_sens_sar_beside_diode() {
    let profile_path = root("configs/systems/adafruit-feather-esp32-v2.yaml");
    let mut profile = SystemManifest::from_file(&profile_path).expect("load Feather V2 profile");
    assert_eq!(profile.name, "adafruit-feather-esp32-v2");
    let analog = SystemManifest::from_file(root(
        "examples/adafruit-feather-esp32-v2/analog-reverse-leakage-adc.yaml",
    ))
    .expect("load generic reverse-leakage fixture");
    profile.cosim_models = analog.cosim_models;

    let chip_path = profile_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load classic ESP32 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Feather V2 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries the diode and three leakage circuits");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "every device output must bind to its SENS SAR channel"
    );
    for channel in [2, 3, 4, 5, 6] {
        assert_eq!(adc_channel_input_count(&mut bus, channel), 0xFFFF);
    }

    let boundary = session.cycles_until_boundary(0);
    let (steps, errors) = session
        .advance_to(boundary, &mut bus)
        .expect("solve all four circuits");
    assert_eq!(steps.len(), 4, "the diode and three leakage circuits solve");
    assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

    let count = |volts: f64| (u32::from((volts * 1000.0).round() as u16) * 4095 / 3300) as u16;
    let routed = |session: &CosimSession, channel: u8| match session
        .signals()
        .get(&format!("adc.sens_sar_adc.{channel}_volts"))
    {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("channel {channel} was not routed: {other:?}"),
    };

    let diode = routed(&session, 6);
    let expected_diode = expected_diode_voltage();
    assert!(
        (diode - expected_diode).abs() < 2e-5,
        "the diode load line should be {expected_diode} V, got {diode} V"
    );
    assert_eq!(convert_adc1_channel(&mut bus, 6), count(diode));

    let exponential = expected_leakage_voltage(ReverseLaw::Exponential, 1e-6, 1.0, 1.5e6);
    // (channel, law, parallel junctions, load, ngspice-42 node voltage)
    let cases = [
        (3, ReverseLaw::Cubic, 1.0, 1.5e6, 1.491_286_820_4),
        (4, ReverseLaw::Cubic, 2.0, 7.5e5, 1.491_286_371_5),
        (5, ReverseLaw::Hold, 2.0, 7.5e5, 1.500_000_225_0),
    ];
    for (channel, law, junctions, load_ohms, ngspice) in cases {
        let got = routed(&session, channel);
        let expected = expected_leakage_voltage(law, 1e-6, junctions, load_ohms);
        assert!(
            (got - expected).abs() < 1e-6,
            "channel {channel} {law:?}: independent root {expected} V, got {got} V"
        );
        assert!(
            (got - ngspice).abs() < 2e-6,
            "channel {channel} {law:?}: ngspice-42 {ngspice} V, got {got} V"
        );
        assert!(
            count(got).abs_diff(count(exponential)) >= 5,
            "channel {channel} {law:?}: the reverse law must move the code away from the \
             exponential's {}, got {}",
            count(exponential),
            count(got)
        );
        assert_eq!(adc_channel_input_count(&mut bus, channel), count(got));
        assert_eq!(
            convert_adc1_channel(&mut bus, channel),
            count(got),
            "the one-shot SAR1 result must report channel {channel}"
        );
    }
    assert_eq!(
        adc_channel_input_count(&mut bus, 2),
        0xFFFF,
        "the adjacent undriven channel stays untouched"
    );
}
