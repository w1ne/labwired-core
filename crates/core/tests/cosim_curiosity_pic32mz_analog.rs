// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists reach the bounded Curiosity PIC32MZ GPIOB endpoint.
//! The configured thresholds are simulation assumptions, not silicon data.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::plugin::ChipPlugin;
use labwired_core::system::mips::Pic32GpioPlugin;
use labwired_core::Bus;
use std::path::Path;

const GPIOB_BASE: u64 = 0x1F86_0100;
const GPIOB_ANSEL: u64 = GPIOB_BASE;
const GPIOB_TRIS: u64 = GPIOB_BASE + 0x10;
const GPIOB_PORT: u64 = GPIOB_BASE + 0x20;

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
fn curiosity_pic32mz_generic_ntc_and_diode_reach_gpio_thresholds() {
    let system_path = root("configs/systems/curiosity-pic32mz-ef.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Curiosity profile");
    assert_eq!(profile.name, "curiosity-pic32mz-ef");
    let analog_path = root("examples/curiosity-pic32mz-ef/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load PIC32MZ descriptor");
    let gpio_plugin = Pic32GpioPlugin;
    let plugins: [&dyn ChipPlugin; 1] = [&gpio_plugin];
    let mut bus = SystemBus::from_config_with_plugins(&chip, &profile, &plugins)
        .expect("build Curiosity system bus");
    bus.write_u32(GPIOB_TRIS, 0xFFFF)
        .expect("configure PORTB as inputs");
    assert_eq!(bus.read_u32(GPIOB_ANSEL).unwrap(), 0xFFFF);
    assert_eq!(bus.read_u32(GPIOB_TRIS).unwrap(), 0xFFFF);

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both generic devices must bind to distinct GPIOB inputs"
    );

    // Reset selects analog mode on PORTB. PORT reads low until ANSEL is
    // cleared for digital input use, as required by the silicon register map.
    session
        .set_signal_number("ui.ntc.temperature_c", -10.0)
        .expect("set NTC temperature");
    let mut cycles = session.cycles_until_boundary(0);
    let (steps, errors) = session
        .advance_to(cycles, &mut bus)
        .expect("solve analog inputs in reset mode");
    assert_eq!(steps.len(), 2);
    assert!(errors.is_empty());
    assert_eq!(bus.read_u32(GPIOB_PORT).unwrap() & 3, 0);
    bus.write_u32(GPIOB_ANSEL, 0xFFFF_FFFC)
        .expect("enable digital input buffers on RB0 and RB1");
    assert_eq!(bus.read_u32(GPIOB_ANSEL).unwrap(), 0xFFFC);

    let expected_diode_v = diode_load_line_voltage();
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // retain the cold high level inside the model's band
        (80.0, false),
        (25.0, false), // retain the hot low level inside the model's band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both generic analog netlists");
        assert_eq!(steps.len(), 2, "both independent netlists solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.gpiob.0");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "board.gpio_in.gpiob.1");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(diode_v < 0.99, "diode voltage crosses the declared VIL");

        let portb = bus.read_u32(GPIOB_PORT).expect("read GPIOB input register");
        assert_eq!(
            portb & 1 != 0,
            expected_ntc_level,
            "GPIOB.0 follows the NTC voltage and retains its Schmitt state"
        );
        assert_eq!(portb & 2, 0, "GPIOB.1 reads low below the declared VIL");
    }
}
