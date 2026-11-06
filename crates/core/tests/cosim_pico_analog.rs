// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog devices route through separate ADC input channels on the
//! Raspberry Pi Pico system profiles, without simulation-only peripherals.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RP2040_ADC_BASE: u64 = 0x4004_C000;
const RP2350_ADC_BASE: u64 = 0x400A_0000;
const CS_EN: u32 = 1;
const CS_START_ONCE: u32 = 1 << 2;
const CS_AINSEL_SHIFT: u32 = 12;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line for the 3.3 V source and series diode.
fn diode_load_line_voltage() -> f64 {
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

/// Exercise the RP2040's firmware-visible AINSEL / START_ONCE / RESULT path.
fn convert_channel(bus: &mut SystemBus, adc_base: u64, channel: u8) -> u16 {
    bus.write_u32(
        adc_base,
        CS_EN | (u32::from(channel) << CS_AINSEL_SHIFT) | CS_START_ONCE,
    )
    .expect("select ADC input and start one conversion");
    bus.read_u32(adc_base + 0x04)
        .expect("read the converted ADC result") as u16
}

fn expected_adc_code(volts: f64) -> u16 {
    let millivolts = (volts * 1000.0).round() as u64;
    ((millivolts * 4096 / 3300).min(4095)) as u16
}

/// Independent level-1 NMOS load-line reference for the board-routed test.
fn mosfet_drain_voltage(gate_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const VTO: f64 = 1.0;
    const KP_A_PER_V2: f64 = 20e-6;
    const W_M: f64 = 100e-6;
    const L_M: f64 = 10e-6;
    const LAMBDA_PER_V: f64 = 0.02;

    let overdrive = gate_v - VTO;
    if overdrive <= 0.0 {
        return SUPPLY_V;
    }
    let beta = KP_A_PER_V2 * W_M / L_M;
    let drain_current = |vds: f64| {
        if vds < overdrive {
            beta * (1.0 + LAMBDA_PER_V * vds) * vds * (overdrive - 0.5 * vds)
        } else {
            0.5 * beta * (1.0 + LAMBDA_PER_V * vds) * overdrive * overdrive
        }
    };
    let residual = |vds: f64| SUPPLY_V - vds - LOAD_OHMS * drain_current(vds);
    let (mut low, mut high) = (0.0, SUPPLY_V);
    for _ in 0..100 {
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

fn generic_analog_devices_reach_adc_profile(
    system_name: &str,
    adc_base: u64,
    ntc_output: &str,
    diode_output: &str,
) {
    let system_path = root(&format!("configs/systems/{system_name}.yaml"));
    let mut profile = SystemManifest::from_file(&system_path).expect("load board system profile");
    assert_eq!(profile.name, system_name);

    let analog_path = root("examples/pico/analog-ntc-diode-system.yaml");
    let mut analog = SystemManifest::from_file(&analog_path)
        .expect("load generic NTC and Shockley diode netlists");
    analog.cosim_models[0]
        .outputs
        .insert("v_sense".to_string(), ntc_output.to_string());
    analog.cosim_models[1]
        .outputs
        .insert("v_diode".to_string(), diode_output.to_string());
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build board system bus");
    assert!(bus.find_peripheral_index_by_name("adc").is_some());

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal co-simulation")
        .expect("fixture carries both generic analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both devices must resolve to distinct ADC input channels"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature input");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve both netlists and route the ADC pad voltages");
        assert_eq!(
            steps.len(),
            2,
            "both unrelated devices solve at the boundary"
        );
        assert!(
            errors.is_empty(),
            "unexpected ADC routing errors: {errors:?}"
        );

        let ntc_v = signal_voltage(&session, ntc_output);
        let expected_ntc_v = 3.3 * ntc_resistance_ohms(temperature_c)
            / (10_000.0 + ntc_resistance_ohms(temperature_c));
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, diode_output);
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let ntc_code = convert_channel(&mut bus, adc_base, 0);
        let diode_code = convert_channel(&mut bus, adc_base, 1);
        assert_eq!(
            ntc_code,
            expected_adc_code(ntc_v),
            "the register-visible ADC0 result must follow the solved NTC voltage"
        );
        assert_eq!(
            diode_code,
            expected_adc_code(diode_v),
            "the independent ADC1 result must follow the Shockley load line"
        );
        assert_eq!(
            convert_channel(&mut bus, adc_base, 2),
            0,
            "undriven ADC2 remains at ground"
        );
        samples.push((temperature_c, ntc_v, ntc_code, diode_code));
    }

    assert!(
        samples[0].1 > samples[1].1 && samples[1].1 > samples[2].1,
        "the NTC divider voltage should fall as it heats: {samples:?}"
    );
    assert!(
        samples[0].2 > samples[1].2 && samples[1].2 > samples[2].2,
        "the 12-bit ADC result should track the NTC voltage: {samples:?}"
    );
    assert!(
        samples.iter().all(|sample| sample.3 == samples[0].3),
        "the independent diode channel stays constant as the NTC changes: {samples:?}"
    );
}

#[test]
fn pico_ntc_and_diode_netlists_reach_adc_channels_end_to_end() {
    generic_analog_devices_reach_adc_profile(
        "pico",
        RP2040_ADC_BASE,
        "adc.adc.0_volts",
        "adc.adc.1_volts",
    );
}

#[test]
fn rp2040_pico_ntc_and_diode_netlists_reach_adc_channels_end_to_end() {
    generic_analog_devices_reach_adc_profile(
        "rp2040-pico",
        RP2040_ADC_BASE,
        "adc.adc.0_volts",
        "adc.adc.1_volts",
    );
}

#[test]
fn rp2350_zero_ntc_and_diode_netlists_reach_gp26_gp27_adc_inputs() {
    let chip = ChipDescriptor::from_file(root("configs/chips/rp2350.yaml"))
        .expect("load RP2350 descriptor");
    for (pad, channel) in [("GP26", 0), ("GP27", 1), ("GP28", 2), ("GP29", 3)] {
        assert_eq!(
            chip.analog_pins[pad].channel, channel,
            "RP2350A {pad} must resolve to ADC input {channel}"
        );
        assert_eq!(chip.analog_pins[pad].peripheral, "adc");
    }
    generic_analog_devices_reach_adc_profile(
        "rp2350-zero",
        RP2350_ADC_BASE,
        "board.analog.gp26_volts",
        "board.analog.gp27_volts",
    );
}

#[test]
fn pico_mosfet_load_line_reaches_adc0_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/pico.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Pico system profile");
    let mut analog = SystemManifest::from_file(root("examples/pico/analog-ntc-diode-system.yaml"))
        .expect("load the generic NTC and Shockley diode netlists");
    analog.cosim_models[0]
        .outputs
        .insert("v_sense".to_string(), "adc.adc.1_volts".to_string());
    analog.cosim_models[1]
        .outputs
        .insert("v_diode".to_string(), "adc.adc.2_volts".to_string());
    let mosfet = SystemManifest::from_file(root("examples/pico/analog-mosfet-switch.yaml"))
        .expect("load the generic low-side MOSFET netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load RP2040 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Pico system bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal co-simulation")
        .expect("profile carries NTC, diode, and MOSFET netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "three unrelated devices must resolve to ADC channels 0/1/2"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set NTC temperature");
    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut drain_samples = Vec::new();
    for gate_v in [0.0_f64, 0.8, 1.0, 2.0, 3.3] {
        session
            .set_signal_number("ui.mosfet.gate_v", gate_v)
            .expect("set MOSFET gate voltage");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all three netlists");
        assert_eq!(steps.len(), 3, "all three unrelated netlists solve");
        assert!(
            errors.is_empty(),
            "unexpected ADC routing errors: {errors:?}"
        );

        let drain_v = signal_voltage(&session, "adc.adc.0_volts");
        let expected_drain_v = mosfet_drain_voltage(gate_v);
        assert!(
            (drain_v - expected_drain_v).abs() < 5e-5,
            "level-1 NMOS load line at Vgs={gate_v} V should be {expected_drain_v} V, got {drain_v} V"
        );
        assert_eq!(
            convert_channel(&mut bus, RP2040_ADC_BASE, 0),
            expected_adc_code(drain_v),
            "firmware-visible ADC0 result follows the solved MOSFET drain voltage"
        );

        let ntc_v = signal_voltage(&session, "adc.adc.1_volts");
        let diode_v = signal_voltage(&session, "adc.adc.2_volts");
        assert!((ntc_v - expected_ntc_v).abs() < 1e-9);
        assert!((diode_v - expected_diode_v).abs() < 2e-5);
        assert_eq!(
            convert_channel(&mut bus, RP2040_ADC_BASE, 1),
            expected_adc_code(ntc_v)
        );
        assert_eq!(
            convert_channel(&mut bus, RP2040_ADC_BASE, 2),
            expected_adc_code(diode_v)
        );
        assert_eq!(
            convert_channel(&mut bus, RP2040_ADC_BASE, 3),
            0,
            "undriven ADC3 remains at ground"
        );
        drain_samples.push((gate_v, drain_v));
    }

    assert_eq!(drain_samples[0].1, drain_samples[2].1);
    assert!(
        drain_samples[4].1 < drain_samples[3].1 && drain_samples[3].1 < drain_samples[2].1,
        "drain voltage falls as the NMOS conducts more strongly: {drain_samples:?}"
    );
}
