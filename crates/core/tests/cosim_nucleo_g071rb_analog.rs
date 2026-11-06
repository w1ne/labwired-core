// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices drive the NUCLEO-G071RB model's PA0/PA1 GPIO inputs.
//! This proves voltage solving, routing, and firmware-visible GPIO threshold
//! behavior. Threshold ratios are test assumptions; ADC external inputs and
//! silicon behavior are not claimed.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, GpioInputThresholds, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_IOPENR: u64 = 0x4002_1034;
const GPIOA_BASE: u64 = 0x5000_0000;
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

#[test]
fn nucleo_g071rb_ntc_and_diode_netlists_reach_pa0_pa1_gpio_inputs() {
    let system_path = root("configs/systems/nucleo-g071rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-G071RB profile");
    assert_eq!(profile.name, "nucleo-g071rb");
    assert!(profile.external_devices.is_empty());
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "led_ld4" && io.peripheral == "gpioa" && io.pin == 5));
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "button_b1" && io.peripheral == "gpioc" && io.pin == 13));

    let analog =
        SystemManifest::from_file(root("examples/nucleo-g071rb/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("load STM32G071 descriptor");
    // Simulation assumptions only: no G071 threshold values are claimed here.
    chip.io_voltage_v = Some(3.3);
    chip.gpio_input_thresholds = Some(GpioInputThresholds { vil: 0.3, vih: 0.7 });

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-G071RB bus");
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA0"),
        Some((GPIOA_IDR, 0)),
        "PA0 must resolve to the modeled GPIOA IDR bit 0"
    );
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA1"),
        Some((GPIOA_IDR, 1)),
        "PA1 must resolve to the modeled GPIOA IDR bit 1"
    );

    bus.write_u32(RCC_IOPENR, 1).expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_MODER).expect("read GPIOA mode");
    bus.write_u32(GPIOA_MODER, moder & !0xF)
        .expect("configure PA0/PA1 as digital inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to separate modeled PA0/PA1 GPIO inputs"
    );

    let expected_diode_v = diode_load_line_voltage();
    let vil_v = 0.3 * 3.3;
    assert!(expected_diode_v < vil_v, "diode must fall below test VIL");
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // hold high while the input sits inside the assumed band
        (80.0, false),
        (25.0, false), // hold low while it returns to the assumed band
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
