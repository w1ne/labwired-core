// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the STM32F072B-DISCO GPIO input path.
//! The ADC peripheral uses the shared L4-style fixed-source model, so this
//! board proof uses the descriptor's voltage thresholds and asserts GPIOA IDR.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_BASE: u64 = 0x4002_1000;
const GPIOA_BASE: u64 = 0x4800_0000;
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
fn stm32f072b_disco_ntc_and_diode_netlists_reach_pa0_pa9_gpio_inputs() {
    let system_path = root("configs/systems/stm32f072b-disco.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load F072B-DISCO profile");
    assert_eq!(profile.name, "stm32f072b-disco");
    assert!(profile.external_devices.is_empty());
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "button_b1" && io.peripheral == "gpioa" && io.pin == 0));
    for (id, pin) in [
        ("led_ld3", 6),
        ("led_ld4", 8),
        ("led_ld5", 9),
        ("led_ld6", 7),
    ] {
        assert!(
            profile
                .board_io
                .iter()
                .any(|io| io.id == id && io.peripheral == "gpioc" && io.pin == pin),
            "mixed-signal fixture keeps the board mapping for {id}"
        );
    }

    let analog = SystemManifest::from_file(root(
        "examples/stm32f072b-disco/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic Beta NTC and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F072 descriptor");
    assert_eq!(chip.pins["PA0"].gpio, "gpioa");
    assert_eq!(chip.pins["PA9"].gpio, "gpioa");
    let thresholds = chip
        .gpio_input_thresholds
        .expect("F072 declares input thresholds");
    let io_voltage = chip.io_voltage_v.expect("F072 declares GPIO voltage");
    let vil_v = thresholds.vil * io_voltage;
    let vih_v = thresholds.vih * io_voltage;

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build F072B-DISCO bus");
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA0"),
        Some((GPIOA_IDR, 0))
    );
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA9"),
        Some((GPIOA_IDR, 9))
    );

    // STM32F0 GPIOA is clocked from AHBENR bit 17. Keep both routed pins in
    // input mode so the GPIO IDR is the endpoint observed by firmware.
    let ahbenr = bus.read_u32(RCC_BASE + 0x14).expect("read RCC AHBENR");
    bus.write_u32(RCC_BASE + 0x14, ahbenr | (1 << 17))
        .expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_BASE).expect("read GPIOA MODER");
    let input_mode_mask = 0b11 | (0b11 << 18);
    bus.write_u32(GPIOA_BASE, moder & !input_mode_mask)
        .expect("configure PA0 and PA9 as inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both generic analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must bind to distinct modeled GPIOA input pins"
    );

    let expected_diode_v = diode_load_line_voltage();
    assert!(
        expected_diode_v < vil_v,
        "diode load line must be below VIL"
    );
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, false),
        (25.0, false), // retain low in the Schmitt band
        (80.0, true),
        (25.0, true), // retain high in the Schmitt band
        (-10.0, false),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(steps.len(), 2, "both netlists cross the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * 10_000.0 / (resistance + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        if temperature_c == 80.0 {
            assert!(
                ntc_v >= vih_v,
                "high sample must cross VIH: {ntc_v} V < {vih_v} V"
            );
        } else if temperature_c == -10.0 {
            assert!(
                ntc_v <= vil_v,
                "cold sample must cross VIL: {ntc_v} V > {vil_v} V"
            );
        }

        let diode_v = signal_voltage(&session, "board.gpio_in.pa9");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(
            diode_v <= vil_v,
            "diode sample must cross VIL: {diode_v} V > {vil_v} V"
        );

        let gpioa_idr = bus
            .read_u32(GPIOA_IDR)
            .expect("read firmware-visible GPIOA IDR");
        assert_eq!(
            gpioa_idr & 1 != 0,
            expected_ntc_level,
            "PA0 follows the solved NTC voltage and retains threshold state"
        );
        assert_eq!(
            gpioa_idr & (1 << 9),
            0,
            "PA9 reads low for the diode load line"
        );
    }
}
