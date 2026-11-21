// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists drive the NUCLEO-WBA52CG model's PA0..PA3 GPIO
//! inputs. This proves voltage solving, pad routing, device physics, and GPIO
//! threshold behavior; thresholds are test assumptions and ADC4 is not modeled
//! here.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, GpioInputThresholds, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_AHB2ENR: u64 = 0x4602_0C8C;
const GPIOA_BASE: u64 = 0x4202_0000;
const GPIOA_MODER: u64 = GPIOA_BASE;
const GPIOA_IDR: u64 = GPIOA_BASE + 0x10;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line, including the diode's series resistance.
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

/// Independent level-1 NMOS load line including `GAMMA`/`PHI` body effect.
fn mosfet_body_effect_drain_voltage(gate_v: f64, bulk_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 100_000.0;
    const VTO: f64 = 1.0;
    const KP_A_PER_V2: f64 = 20e-6;
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

#[test]
fn nucleo_wba52cg_ntc_and_diode_netlists_reach_pa0_pa1_gpio_inputs() {
    let system_path = root("configs/systems/nucleo_wba52cg.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-WBA52CG profile");
    assert_eq!(profile.name, "nucleo_wba52cg");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-wba52cg/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("load STM32WBA52 descriptor");
    // Test-only assumptions; these are not asserted as WBA52 silicon thresholds.
    chip.io_voltage_v = Some(3.3);
    chip.gpio_input_thresholds = Some(GpioInputThresholds::uniform(0.3, 0.7));

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-WBA52CG bus");
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA0"),
        Some((GPIOA_IDR, 0)),
        "PA0 must resolve to modeled GPIOA IDR bit 0"
    );
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA1"),
        Some((GPIOA_IDR, 1)),
        "PA1 must resolve to modeled GPIOA IDR bit 1"
    );

    bus.write_u32(RCC_AHB2ENR, 1).expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_MODER).expect("read GPIOA mode");
    bus.write_u32(GPIOA_MODER, moder & !0xF)
        .expect("configure PA0/PA1 as digital inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both generic analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must bind to distinct modeled PA0/PA1 GPIO inputs"
    );

    let expected_diode_v = diode_load_line_voltage();
    let vil_v = 0.3 * 3.3;
    assert!(
        expected_diode_v < vil_v,
        "diode must fall below assumed VIL"
    );
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // retain the cold high level inside the assumed Schmitt band
        (80.0, false),
        (25.0, false), // retain the hot low level inside the assumed Schmitt band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(
            steps.len(),
            2,
            "both netlists cross the co-simulation boundary"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "board.gpio_in.pa1");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let gpioa_idr = bus.read_u32(GPIOA_IDR).expect("read GPIOA input data");
        assert_eq!(
            gpioa_idr & 1 != 0,
            expected_ntc_level,
            "PA0 must follow the solved NTC voltage and retain threshold state"
        );
        assert_eq!(gpioa_idr & 2, 0, "PA1 must read low below the assumed VIL");
    }
}

#[test]
fn nucleo_wba52cg_mosfet_body_effect_reaches_pa2_pa3_gpio_beside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo_wba52cg.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-WBA52CG profile");
    let analog =
        SystemManifest::from_file(root("examples/nucleo-wba52cg/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC divider and Shockley diode netlists");
    let mosfet = SystemManifest::from_file(root(
        "examples/nucleo-wba52cg/analog-mosfet-body-effect-system.yaml",
    ))
    .expect("load matched generic NMOS body-effect branches");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("load STM32WBA52 descriptor");
    // Assumed thresholds for the digital endpoint; no WBA52 silicon limits are claimed.
    chip.io_voltage_v = Some(3.3);
    chip.gpio_input_thresholds = Some(GpioInputThresholds::uniform(0.3, 0.7));

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-WBA52CG bus");
    for (pin, bit) in [("PA2", 2), ("PA3", 3)] {
        assert_eq!(
            SystemBus::resolve_pin_idr_pub(&bus, pin),
            Some((GPIOA_IDR, bit)),
            "{pin} must resolve to modeled GPIOA IDR bit {bit}"
        );
    }
    bus.write_u32(RCC_AHB2ENR, 1).expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_MODER).expect("read GPIO mode");
    bus.write_u32(GPIOA_MODER, moder & !0xFF)
        .expect("configure PA0..PA3 as digital inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and generic NMOS branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must route to distinct modeled GPIO inputs"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", -10.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.mosfet_body.gate_v", 1.5)
        .expect("set shared generic NMOS gate drive");

    const VIL_V: f64 = 0.3 * 3.3;
    const VIH_V: f64 = 0.7 * 3.3;
    let expected_ntc_v = 3.3 * ntc_resistance_ohms(-10.0) / (10_000.0 + ntc_resistance_ohms(-10.0));
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
            .expect("solve and route NTC, diode, and matched NMOS netlists");
        assert_eq!(steps.len(), 3, "NTC, diode, and NMOS models solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let reference_v = signal_voltage(&session, "board.gpio_in.pa2");
        let body_v = signal_voltage(&session, "board.gpio_in.pa3");
        let expected_reference_v = mosfet_body_effect_drain_voltage(1.5, 0.0);
        let expected_body_v = mosfet_body_effect_drain_voltage(1.5, bulk_v);
        assert!((reference_v - expected_reference_v).abs() < 5e-5);
        assert!((body_v - expected_body_v).abs() < 5e-5);
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
        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let diode_v = signal_voltage(&session, "board.gpio_in.pa1");
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "NTC route is isolated"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "diode route is isolated"
        );

        let gpioa_idr = bus.read_u32(GPIOA_IDR).expect("read GPIOA input data");
        assert_ne!(gpioa_idr & 1, 0, "cold NTC drives PA0 above assumed VIH");
        assert_eq!(
            gpioa_idr & 2,
            0,
            "Shockley diode drives PA1 below assumed VIL"
        );
        assert_eq!(gpioa_idr & (1 << 2), 0, "zero-bias NMOS keeps PA2 low");
        assert_eq!(
            gpioa_idr & (1 << 3) != 0,
            body_v > VIH_V,
            "PA3 follows the solved NMOS drain after the assumed Schmitt threshold"
        );
        body_samples.push((bulk_v, body_v));
    }
    assert!(
        body_samples.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "increasing reverse body bias raises the drain voltage: {body_samples:?}"
    );
    assert!(
        body_samples.last().unwrap().1 > VIH_V && body_samples[0].1 < VIL_V,
        "body effect must move the routed endpoint across the assumed GPIO thresholds: {body_samples:?}"
    );
}
