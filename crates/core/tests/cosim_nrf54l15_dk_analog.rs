// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA netlists drive the nRF54L15-DK application-core SAADC.

mod common;

#[test]
fn nrf54l15_dk_ntc_and_diode_netlists_reach_distinct_saadc_channels() {
    common::nrf54l_analog::assert_ntc_and_diode_saadc_profile(
        "configs/systems/nrf54l15dk.yaml",
        "examples/nrf54l15-dk/analog-ntc-diode-system.yaml",
        "nrf54l15dk",
        &[],
        &[
            ("led0", "gpio2", 9),
            ("led1", "gpio1", 10),
            ("led2", "gpio2", 7),
            ("led3", "gpio1", 14),
            ("button0", "gpio1", 13),
            ("button1", "gpio1", 9),
            ("button2", "gpio1", 8),
            ("button3", "gpio0", 4),
        ],
    );
}

/// Independent depletion charge of a `CJO`/`VJ` junction with `M` = 0.4,
/// valid below `FC·VJ` (the fixture stays reverse-biased).
fn depletion_charge(cjo_f: f64, vj_v: f64, junction_v: f64) -> f64 {
    const M: f64 = 0.4;
    cjo_f * vj_v / (1.0 - M) * (1.0 - (1.0 - junction_v / vj_v).powf(1.0 - M))
}

/// Independent backward-Euler update of a 1 k drive into the cathode of a
/// grounded-anode diode carrying that charge and the solver's 1 pS GMIN.
/// `IS` = 1e-18 A keeps the reverse current below 1 pV across 1 k.
fn expected_depletion_voltage(
    cjo_f: f64,
    vj_v: f64,
    previous_v: f64,
    drive_v: f64,
    step_s: f64,
) -> f64 {
    const DRIVE_OHMS: f64 = 1_000.0;
    const GMIN_S: f64 = 1e-12;
    let previous_charge = depletion_charge(cjo_f, vj_v, -previous_v);
    let residual = |sense_v: f64| {
        let junction_v = -sense_v;
        (drive_v - sense_v) / DRIVE_OHMS
            + GMIN_S * junction_v
            + (depletion_charge(cjo_f, vj_v, junction_v) - previous_charge) / step_s
    };
    let (mut low, mut high) = (previous_v.min(drive_v), previous_v.max(drive_v));
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

#[test]
fn nrf54l15_dk_diode_tlevc1_depletion_charge_reaches_saadc_beside_ntc_and_diode() {
    use common::nrf54l_analog::{
        configure_saadc_channels, expected_saadc_code, scan_channels, voltage,
    };
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::SystemBus;
    use labwired_core::cosim::{CosimSession, RoutingError};
    use std::path::Path;

    const STEP_S: f64 = 1e-6;
    const SUBSTEPS: usize = 10;
    const DRIVE_V: f64 = 3.0;
    // TLEVC=1 from the fixture card: CJO·(1 + CTA·ΔT), VJ − TPB·ΔT, TNOM 27 °C.
    let linear = |temperature_c: f64| {
        let delta = temperature_c - 27.0;
        (1e-9 * (1.0 + 4e-3 * delta), 0.7 - 2e-3 * delta)
    };
    let (hot_cjo, hot_vj) = linear(85.0);
    let (cold_cjo, cold_vj) = linear(-40.0);

    let system_path = common::root("configs/systems/nrf54l15dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF54L15-DK profile");
    let analog = SystemManifest::from_file(common::root(
        "examples/nrf54l15-dk/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC and diode netlists");
    let tlevc = SystemManifest::from_file(common::root(
        "examples/nrf54l15-dk/analog-diode-tlevc-charge-system.yaml",
    ))
    .expect("load generic TLEVC diode-charge branches");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(tlevc.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF54L15 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF54L15-DK bus");
    configure_saadc_channels(&mut bus, 5);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode and three TLEVC branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all five netlists must bind to SAADC channels 0..4"
    );

    let mut cycles = 0;
    let mut settle = |session: &mut CosimSession, bus: &mut SystemBus| {
        session
            .set_signal_number("ui.ntc.temperature_c", 25.0)
            .expect("hold the NTC at 25 °C");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, bus)
            .expect("solve and route every netlist");
        assert_eq!(steps.len(), 5, "every netlist solves each step");
        assert!(errors.is_empty(), "route errors: {errors:?}");
    };

    session
        .set_signal_number("ui.diode_tlevc.drive_v", 0.0)
        .expect("start every TLEVC branch discharged");
    settle(&mut session, &mut bus);
    assert!(voltage(&session, "adc.saadc.2_volts").abs() < 1e-12);

    session
        .set_signal_number("ui.diode_tlevc.drive_v", DRIVE_V)
        .expect("step the drive into reverse bias");
    let (mut hot_expected, mut cold_expected) = (0.0, 0.0);
    let mut first_step = None;
    for step in 0..8 {
        settle(&mut session, &mut bus);
        for _ in 0..SUBSTEPS {
            let h = STEP_S / SUBSTEPS as f64;
            hot_expected = expected_depletion_voltage(hot_cjo, hot_vj, hot_expected, DRIVE_V, h);
            cold_expected =
                expected_depletion_voltage(cold_cjo, cold_vj, cold_expected, DRIVE_V, h);
        }
        let hot_v = voltage(&session, "adc.saadc.2_volts");
        let cold_v = voltage(&session, "adc.saadc.3_volts");
        let silicon_v = voltage(&session, "adc.saadc.4_volts");
        assert!(
            (hot_v - hot_expected).abs() < 2e-8,
            "TLEVC=1 at 85 °C, step {step}: expected {hot_expected} V, got {hot_v} V"
        );
        assert!(
            (cold_v - cold_expected).abs() < 2e-8,
            "TLEVC=1 at -40 °C, step {step}: expected {cold_expected} V, got {cold_v} V"
        );
        if step == 0 {
            first_step = Some((hot_v, cold_v, silicon_v));
        }

        let ntc_v = voltage(&session, "adc.saadc.0_volts");
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the 25 °C NTC divider stays isolated, got {ntc_v} V"
        );
        let diode_v = voltage(&session, "adc.saadc.1_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let codes = scan_channels(&mut bus, 5);
        for (channel, volts) in [ntc_v, diode_v, hot_v, cold_v, silicon_v]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                codes[channel],
                expected_saadc_code(volts),
                "SAADC channel {channel} at step {step}"
            );
        }
    }

    // The hot junction holds more charge and lags further than the cold one,
    // and the silicon law (TLEVC=0, coefficients ignored) is a third value.
    let (hot_v, cold_v, silicon_v) = first_step.expect("first step recorded");
    assert!(
        cold_v - hot_v > 0.3,
        "85 °C vs -40 °C under TLEVC=1 must differ by > 0.3 V after 1 µs: {hot_v} V vs {cold_v} V"
    );
    assert!(
        (silicon_v - hot_v).abs() > 0.05,
        "TLEVC must change the 85 °C trajectory: {silicon_v} V (TLEVC=0) vs {hot_v} V"
    );
    assert_ne!(
        expected_saadc_code(silicon_v),
        expected_saadc_code(hot_v),
        "the TLEVC switch must be visible in the SAADC code"
    );
}

/// Independent level-1 NMOS body-effect load line, with source at ground.
fn mosfet_body_effect_drain_voltage(gate_v: f64, bulk_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const VTO: f64 = 1.0;
    const KP_A_PER_V2: f64 = 100e-6;
    const W_M: f64 = 100e-6;
    const L_M: f64 = 10e-6;
    const LAMBDA_PER_V: f64 = 0.02;
    const GAMMA: f64 = 0.4;
    const PHI: f64 = 0.7;

    let threshold = VTO + GAMMA * ((PHI - bulk_v).sqrt() - PHI.sqrt());
    let overdrive = gate_v - threshold;
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
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

/// Independent Shockley load line for the unrelated diode retained in the fixture.
fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const SERIES_LIMIT_OHMS: f64 = 1_000.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
    let current_at_junction_voltage =
        |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let current = current_at_junction_voltage(junction_v);
        (SUPPLY_V - junction_v - current * DIODE_RS_OHMS) / SERIES_LIMIT_OHMS - current
    };
    let (mut low, mut high) = (0.0, 1.2);
    for _ in 0..100 {
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    let junction_v = 0.5 * (low + high);
    junction_v + current_at_junction_voltage(junction_v) * DIODE_RS_OHMS
}

#[test]
fn nrf54l15_dk_mosfet_body_effect_reaches_saadc_beside_ntc_and_diode() {
    use common::nrf54l_analog::{
        configure_saadc_channels, expected_saadc_code, scan_channels, voltage,
    };
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::SystemBus;
    use labwired_core::cosim::{CosimSession, RoutingError};
    use std::path::Path;

    let system_path = common::root("configs/systems/nrf54l15dk.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load nRF54L15-DK profile");
    let analog = SystemManifest::from_file(common::root(
        "examples/nrf54l15-dk/analog-ntc-diode-system.yaml",
    ))
    .expect("load unrelated generic NTC and diode netlists");
    let mosfet = SystemManifest::from_file(common::root(
        "examples/nrf54l15-dk/analog-mosfet-body-effect-system.yaml",
    ))
    .expect("load generic NMOS body-effect branches");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nRF54L15 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build nRF54L15-DK bus");
    configure_saadc_channels(&mut bus, 4);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and NMOS branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all four netlist outputs must bind to SAADC channels 0..3"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("hold the unrelated NTC at 25 °C");
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut body_samples = Vec::new();
    for bulk_v in [0.0_f64, -0.5, -1.0, -1.5] {
        session
            .set_signal_number("ui.mosfet_body.bulk_v", bulk_v)
            .expect("set NMOS reverse body bias");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and NMOS netlists");
        assert_eq!(steps.len(), 3, "NTC, diode, and NMOS models solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let reference_v = voltage(&session, "adc.saadc.2_volts");
        let body_v = voltage(&session, "adc.saadc.3_volts");
        let expected_reference_v = mosfet_body_effect_drain_voltage(2.5, 0.0);
        let expected_body_v = mosfet_body_effect_drain_voltage(2.5, bulk_v);
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "zero-bias NMOS load line: expected {expected_reference_v} V, got {reference_v} V"
        );
        assert!(
            (body_v - expected_body_v).abs() < 5e-5,
            "GAMMA/PHI load line at VBS={bulk_v} V: expected {expected_body_v} V, got {body_v} V"
        );
        if bulk_v == 0.0 {
            assert!(
                (reference_v - body_v).abs() < 1e-9,
                "matched NMOS devices agree at zero body bias"
            );
        } else {
            assert!(
                body_v > reference_v,
                "reverse body bias raises Vth and the loaded drain voltage: {body_v} V vs {reference_v} V"
            );
        }

        let ntc_v = voltage(&session, "adc.saadc.0_volts");
        let diode_v = voltage(&session, "adc.saadc.1_volts");
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "unrelated 25 °C NTC divider remains isolated"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains isolated"
        );

        let codes = scan_channels(&mut bus, 4);
        for (channel, volts) in [ntc_v, diode_v, reference_v, body_v]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                codes[channel],
                expected_saadc_code(volts),
                "firmware-visible SAADC channel {channel} at VBS={bulk_v} V"
            );
        }
        body_samples.push((bulk_v, body_v, codes[3]));
    }

    assert!(
        body_samples.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "increasing reverse body bias raises the drain voltage: {body_samples:?}"
    );
    assert_ne!(
        body_samples[0].2, body_samples[3].2,
        "body effect must be visible in the 12-bit SAADC result"
    );
}
