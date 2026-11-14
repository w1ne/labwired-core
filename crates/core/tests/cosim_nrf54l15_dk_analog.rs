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
