// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the FRDM-K64F's mapped GPIO inputs.
//! This profile has no ADC model; the endpoint is voltage-thresholded PDIR.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const GPIOA_PDIR: u64 = 0x400F_F000 + 0x10;
const GPIOC_PDIR: u64 = 0x400F_F080 + 0x10;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

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

#[test]
fn frdm_k64f_ntc_and_diode_netlists_reach_gpio_inputs_through_thresholds() {
    let system_path = root("configs/systems/frdm-k64f.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load FRDM-K64F profile");
    assert_eq!(profile.name, "frdm-k64f");
    assert!(profile
        .board_io
        .iter()
        .any(|io| { io.id == "button_sw2" && io.peripheral == "gpioc" && io.pin == 6 }));
    assert!(profile
        .board_io
        .iter()
        .any(|io| { io.id == "button_sw3" && io.peripheral == "gpioa" && io.pin == 4 }));

    let analog = SystemManifest::from_file(root("examples/frdm-k64f/analog-ntc-diode-system.yaml"))
        .expect("load generic Beta NTC and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load MK64F12 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build FRDM-K64F bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic devices must bind to distinct mapped GPIO inputs"
    );
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // retain the cold high level inside the Schmitt band
        (80.0, false),
        (25.0, false), // retain the hot low level inside the Schmitt band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(steps.len(), 2, "both independent netlists solve per step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.ptc6");
        let expected_ntc_v = 3.3 * ntc_resistance_ohms(temperature_c)
            / (10_000.0 + ntc_resistance_ohms(temperature_c));
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "board.gpio_in.pta4");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let gpioc_pdir = bus.read_u32(GPIOC_PDIR).expect("read GPIOC PDIR");
        let gpioa_pdir = bus.read_u32(GPIOA_PDIR).expect("read GPIOA PDIR");
        assert_eq!(
            gpioc_pdir & (1 << 6) != 0,
            expected_ntc_level,
            "PTC6 PDIR must follow the solved NTC voltage and retain its Schmitt state"
        );
        assert_eq!(
            gpioa_pdir & (1 << 4),
            0,
            "PTA4 PDIR must read low for the diode voltage below VIL"
        );
    }
}
