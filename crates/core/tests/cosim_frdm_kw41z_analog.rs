// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists reach the KW41Z's behavioral GPIOC PDIR through PB2.
//! Input thresholds are simulation assumptions, not silicon electrical claims.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, CosimModelConfig, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const GPIOC_PDIR: u64 = 0x400F_F080 + 0x10;
const PB2_GPIOC_BIT: u8 = 1;

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

fn profile_with_model(system_rel: &str, model: CosimModelConfig) -> (SystemManifest, SystemBus) {
    let system_path = root(system_rel);
    let mut profile = SystemManifest::from_file(&system_path).expect("load FRDM-KW41Z profile");
    profile.cosim_models = vec![model];
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load MKW41Z4 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build FRDM-KW41Z bus");
    (profile, bus)
}

fn assert_profile_routes_generic_analog_inputs(system_rel: &str, expected_name: &str) {
    let fixture =
        SystemManifest::from_file(root("examples/frdm-kw41z/analog-ntc-diode-system.yaml"))
            .expect("load generic Beta NTC and Shockley diode netlists");
    assert_eq!(fixture.cosim_models.len(), 2);

    for model in &fixture.cosim_models {
        let (profile, mut bus) = profile_with_model(system_rel, model.clone());
        assert_eq!(profile.name, expected_name);
        let (idr_address, idr_bit) =
            SystemBus::resolve_pin_idr_pub(&bus, "PB2").expect("PB2 resolves to GPIOC PDIR");
        assert_eq!(idr_address, GPIOC_PDIR);
        assert_eq!(idr_bit, PB2_GPIOC_BIT);

        let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
            .expect("build analog co-simulation")
            .expect("fixture carries one analog device");
        assert_eq!(
            session.binding_errors(),
            &[] as &[RoutingError],
            "the generic analog device must bind through the PB2 pin map"
        );

        if model.id == "ntc_divider" {
            let mut cycles = 0;
            for (temperature_c, expected_high) in [
                (-20.0_f64, true),
                (25.0, true), // retain the cold high level within the assumed Schmitt band
                (80.0, false),
                (25.0, false), // retain the hot low level within the assumed Schmitt band
                (-20.0, true),
            ] {
                session
                    .set_signal_number("ui.ntc.temperature_c", temperature_c)
                    .expect("set NTC temperature");
                cycles += session.cycles_until_boundary(cycles);
                let (steps, errors) = session
                    .advance_to(cycles, &mut bus)
                    .expect("solve and route NTC netlist");
                assert_eq!(steps.len(), 1, "one NTC netlist solves per boundary");
                assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

                let voltage = signal_voltage(&session, "board.gpio_in.pb2");
                let resistance = ntc_resistance_ohms(temperature_c);
                let expected_voltage = 3.3 * resistance / (10_000.0 + resistance);
                assert!(
                    (voltage - expected_voltage).abs() < 1e-9,
                    "at {temperature_c} °C the Beta divider should be {expected_voltage} V, got {voltage} V"
                );
                assert_eq!(
                    bus.read_u32(GPIOC_PDIR).expect("read GPIOC PDIR") & (1 << PB2_GPIOC_BIT) != 0,
                    expected_high,
                    "PB2 PDIR follows the solved NTC node and its retained Schmitt state"
                );
            }
        } else {
            assert_eq!(model.id, "diode_load_line");
            let cycles = session.cycles_until_boundary(0);
            let (steps, errors) = session
                .advance_to(cycles, &mut bus)
                .expect("solve and route Shockley diode netlist");
            assert_eq!(steps.len(), 1);
            assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

            let voltage = signal_voltage(&session, "board.gpio_in.pb2");
            let expected_voltage = diode_load_line_voltage();
            assert!(
                (voltage - expected_voltage).abs() < 2e-5,
                "Shockley load line should be {expected_voltage} V, got {voltage} V"
            );
            assert_eq!(
                bus.read_u32(GPIOC_PDIR).expect("read GPIOC PDIR") & (1 << PB2_GPIOC_BIT),
                0,
                "PB2 PDIR reads low for the diode voltage below assumed VIL"
            );
        }
    }
}

#[test]
fn frdm_kw41z_profiles_route_generic_ntc_and_diode_to_pb2_gpio_thresholds() {
    assert_profile_routes_generic_analog_inputs(
        "configs/systems/frdm-kw41z-lcd.yaml",
        "frdm-kw41z-lcd",
    );
    assert_profile_routes_generic_analog_inputs("configs/systems/frdm-kw41z.yaml", "frdm-kw41z");
}
