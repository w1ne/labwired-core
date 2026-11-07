// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog devices route through Uno R4 Minima A0/A1 into its ADC14.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Bus, Cpu, Machine};
use std::path::Path;

const ADC_BASE: u64 = 0x4005_C000;
const ADCSR: u64 = ADC_BASE;
const ADANSA0: u64 = ADC_BASE + 0x04;
const ADCER: u64 = ADC_BASE + 0x0E;
const ADDR0: u64 = ADC_BASE + 0x20;
const ADST: u16 = 1 << 15;
const SCAN_CHANNELS: u16 = (1 << 0) | (1 << 1) | (1 << 9);

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 5.0;
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

/// Independent diode-connected level-1 NMOS root: the bias resistor feeds a
/// saturated device whose `VTO`, `KP` and `PHI` follow ngspice's `mos1temp`
/// laws at the instance temperature (source and bulk grounded).
fn mosfet_thermometer_voltage(bias_v: f64, temperature_c: f64) -> f64 {
    const BIAS_OHMS: f64 = 1e6;
    const VTO: f64 = 0.7;
    const KP: f64 = 50e-6;
    const GAMMA: f64 = 0.4;
    const PHI: f64 = 0.65;
    const W_OVER_L: f64 = 10.0;
    const TNOM_K: f64 = 25.0 + 273.15;
    const GMIN: f64 = 1e-12;
    let thermal_v = |kelvin: f64| 1.380_649e-23 * kelvin / 1.602_176_634e-19;
    let bandgap = |kelvin: f64| 1.16 - 7.02e-4 * kelvin * kelvin / (kelvin + 1108.0);
    let kelvin = temperature_c + 273.15;
    let ratio = kelvin / TNOM_K;
    let vt = thermal_v(kelvin);
    let phi_t = PHI * ratio
        - vt * (3.0 * ratio.ln() + bandgap(TNOM_K) / thermal_v(TNOM_K) - bandgap(kelvin) / vt);
    let vto_t = VTO
        + GAMMA * (phi_t.sqrt() - PHI.sqrt())
        + 0.5 * (phi_t - PHI)
        + 0.5 * (bandgap(TNOM_K) - bandgap(kelvin));
    let beta_t = KP / ratio.powf(1.5) * W_OVER_L;
    let residual = |gate_v: f64| {
        let overdrive = (gate_v - vto_t).max(0.0);
        // GMIN drain-source plus the reverse bulk-drain junction's GMIN.
        (bias_v - gate_v) / BIAS_OHMS - 0.5 * beta_t * overdrive * overdrive - 2.0 * GMIN * gate_v
    };
    let (mut low, mut high) = (0.0, bias_v);
    for _ in 0..100 {
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

fn signal_number(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(value)) => *value,
        other => panic!("expected analog voltage at {path}, got {other:?}"),
    }
}

fn scan_adc14<C: Cpu>(
    machine: &mut Machine<C>,
    session: &mut CosimSession,
    adcer: u16,
) -> [u16; 3] {
    run_adc14_scan(machine, session, adcer, SCAN_CHANNELS);
    [
        machine.bus.read_u16(ADDR0).expect("read AN000 result"),
        machine
            .bus
            .read_u16(ADDR0 + 2)
            .expect("read undriven AN001 result"),
        machine.bus.read_u16(ADDR0 + 18).expect("read AN009 result"),
    ]
}

/// Run one firmware-visible ADC14 group-A software scan over `channels`.
fn run_adc14_scan<C: Cpu>(
    machine: &mut Machine<C>,
    session: &mut CosimSession,
    adcer: u16,
    channels: u16,
) {
    machine
        .bus
        .write_u16(ADANSA0, channels)
        .expect("select the ADC14 scan channels");
    machine
        .bus
        .write_u16(ADCER, adcer)
        .expect("select ADC14 conversion accuracy");
    machine
        .bus
        .write_u16(ADCSR, ADST)
        .expect("start firmware-visible ADC14 software scan");

    let mut completed = false;
    for _ in 0..2048 {
        if machine.bus.read_u16(ADCSR).expect("read ADC14 ADCSR") & ADST == 0 {
            completed = true;
            break;
        }
        let advance = session
            .advance(machine, AdvanceRequest::single())
            .expect("advance CPU and ADC14 in lockstep");
        assert!(
            advance.new_routing_errors.is_empty(),
            "unexpected route errors: {:?}",
            advance.new_routing_errors
        );
    }
    assert!(completed, "ADC14 group-A scan did not complete");
}

#[test]
fn uno_r4_minima_ntc_and_diode_netlists_reach_a0_a1_and_adc14() {
    let system_path = root("configs/systems/arduino-uno-r4-minima.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Uno R4 profile");
    assert_eq!(profile.name, "arduino-uno-r4-minima");
    let analog = SystemManifest::from_file(root(
        "examples/arduino-uno-r4-minima/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC and diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load RA4M1 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Uno R4 bus");
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    const IDLE: u32 = 0x2000_2000;
    machine
        .bus
        .write_u16(u64::from(IDLE), 0xE7FE)
        .expect("place a branch-to-self instruction in RAM");
    machine.cpu.set_pc(IDLE);
    machine.cpu.set_sp(0x2000_7F00);

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build analog co-simulation")
        .expect("profile contains analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "A0 and A1 must resolve through descriptor-backed ADC14 analog pins"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC model input");

        let mut reached_boundary = false;
        for _ in 0..256 {
            let advance = session
                .advance(&mut machine, AdvanceRequest::single())
                .expect("advance the board and analog models in lockstep");
            assert!(
                advance.new_routing_errors.is_empty(),
                "unexpected route errors: {:?}",
                advance.new_routing_errors
            );
            if !advance.routed.is_empty() {
                reached_boundary = true;
                break;
            }
        }
        assert!(
            reached_boundary,
            "analog models did not reach their boundary"
        );

        let ntc_v = signal_number(&session, "board.analog.p014_volts");
        let diode_v = signal_number(&session, "board.analog.p000_volts");
        let ntc_ohms = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 5.0 * 10_000.0 / (ntc_ohms + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the Beta NTC divider at {temperature_c} °C should be {expected_ntc_v} V, got {ntc_v} V"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        for (adcer, full_scale, resolution) in [(0, 4095_u32, 12), (3 << 1, 16383, 14)] {
            let [diode_code, neighbor_code, ntc_code] =
                scan_adc14(&mut machine, &mut session, adcer);
            let diode_mv = (diode_v * 1000.0 + 0.5).floor() as u32;
            let ntc_mv = (ntc_v * 1000.0 + 0.5).floor() as u32;
            assert_eq!(
                diode_code,
                (diode_mv * full_scale / 5000) as u16,
                "AN000 ADC14 {resolution}-bit result must follow the diode load line"
            );
            assert_eq!(
                ntc_code,
                (ntc_mv * full_scale / 5000) as u16,
                "AN009 ADC14 {resolution}-bit result must follow the temperature-dependent divider"
            );
            assert_eq!(neighbor_code, 0, "unconnected AN001 remains at zero");
            samples.push((temperature_c, ntc_v, ntc_code));
        }
    }

    assert!(
        samples[4].1 > samples[2].1 && samples[2].1 > samples[0].1,
        "an NTC divider rises as it heats: {samples:?}"
    );
    assert!(
        samples[5].2 > samples[3].2 && samples[3].2 > samples[1].2,
        "the 14-bit ADC result follows the temperature-dependent voltage: {samples:?}"
    );
    assert_eq!(samples[3].2, 8191, "25 °C yields half scale at 14 bits");
}

#[test]
fn uno_r4_minima_heated_mosfet_thermometer_reaches_a2_a3_adc14_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/arduino-uno-r4-minima.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Uno R4 profile");
    let analog = SystemManifest::from_file(root(
        "examples/arduino-uno-r4-minima/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC and diode netlists");
    let thermometer = SystemManifest::from_file(root(
        "examples/arduino-uno-r4-minima/analog-mosfet-thermometer.yaml",
    ))
    .expect("load the generic NMOS threshold thermometer netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(thermometer.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load RA4M1 descriptor");
    for (pad, channel) in [("P014", 9), ("P000", 0), ("P001", 1), ("P002", 2)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Uno R4 bus");
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    const IDLE: u32 = 0x2000_2000;
    machine
        .bus
        .write_u16(u64::from(IDLE), 0xE7FE)
        .expect("place a branch-to-self instruction in RAM");
    machine.cpu.set_pc(IDLE);
    machine.cpu.set_sp(0x2000_7F00);

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build analog co-simulation")
        .expect("profile contains analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "A0..A3 must resolve through descriptor-backed ADC14 analog pins"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the independent NTC temperature");

    for bias_v in [2.5_f64, 5.0] {
        session
            .set_signal_number("ui.mos_thermo.bias_v", bias_v)
            .expect("drive the shared NMOS bias");
        let mut reached_boundary = false;
        for _ in 0..256 {
            let advance = session
                .advance(&mut machine, AdvanceRequest::single())
                .expect("advance the board and analog models in lockstep");
            assert!(
                advance.new_routing_errors.is_empty(),
                "unexpected route errors: {:?}",
                advance.new_routing_errors
            );
            if !advance.routed.is_empty() {
                reached_boundary = true;
                break;
            }
        }
        assert!(
            reached_boundary,
            "analog models did not reach their boundary"
        );

        let reference_v = signal_number(&session, "board.analog.p001_volts");
        let heated_v = signal_number(&session, "board.analog.p002_volts");
        for (label, measured, temperature_c) in
            [("TNOM", reference_v, 25.0), ("85 C", heated_v, 85.0)]
        {
            let expected = mosfet_thermometer_voltage(bias_v, temperature_c);
            assert!(
                (measured - expected).abs() < 2e-5,
                "{label} NMOS at {bias_v} V bias should sit at {expected} V, got {measured} V"
            );
        }
        // VTO(T) falls ~1.5 mV/K, partly offset by KP(T): a missing law
        // either collapses the drop or flips its sign.
        let drop = reference_v - heated_v;
        assert!(
            (0.04..0.12).contains(&drop),
            "the heated NMOS must read lower at {bias_v} V bias, got {drop} V"
        );

        let ntc_v = signal_number(&session, "board.analog.p014_volts");
        let diode_v = signal_number(&session, "board.analog.p000_volts");
        assert!(
            (ntc_v - 2.5).abs() < 1e-9,
            "the 25 C NTC divider sits at half supply"
        );
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated diode load line is unaffected"
        );

        for (adcer, full_scale) in [(0, 4095_u32), (3 << 1, 16383)] {
            run_adc14_scan(
                &mut machine,
                &mut session,
                adcer,
                (1 << 0) | (1 << 1) | (1 << 2) | (1 << 9),
            );
            let code =
                |volts: f64| ((volts * 1000.0 + 0.5).floor() as u32 * full_scale / 5000) as u16;
            for (offset, channel, volts) in [
                (0, "AN000", diode_v),
                (2, "AN001", reference_v),
                (4, "AN002", heated_v),
                (18, "AN009", ntc_v),
            ] {
                assert_eq!(
                    machine
                        .bus
                        .read_u16(ADDR0 + offset)
                        .expect("read ADC14 result"),
                    code(volts),
                    "firmware-visible {channel} result must report its solved voltage"
                );
            }
            assert!(
                u32::from(code(reference_v)) > u32::from(code(heated_v)) + 30,
                "firmware must resolve the temperature difference in ADC14 counts"
            );
        }
    }
}
