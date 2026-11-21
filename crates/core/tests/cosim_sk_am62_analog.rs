// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog devices reach MAIN_GPIO0 on the SK-AM62 through two pads
//! of the 40-pin user expansion connector (SPRUJ40F, Table 2-28): pin 15
//! EXP_GPIO0_32 (ball L23) and pin 13 EXP_GPIO0_42 (ball L21), both LVCMOS
//! in the 3.3 V GPMC group. Firmware reads the levels in IN_DATA of the
//! second register bank and the edges in its INTSTAT.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

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

fn voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// MAIN_GPIO0 (`ti,am64-gpio`, Linux v6.12 k3-am62-main.dtsi). GPIO 32..63
/// are the second register bank: `offset_array[1] = 0x38` in gpio-davinci.c.
const GPIO0_BASE: u64 = 0x0060_0000;
const DIR23: u64 = GPIO0_BASE + 0x38;
const IN_DATA23: u64 = GPIO0_BASE + 0x48;
const SET_FAL_TRIG23: u64 = GPIO0_BASE + 0x54;
const INTSTAT23: u64 = GPIO0_BASE + 0x5C;
/// GPIO0_32 carries the NTC, GPIO0_42 the diode.
const NTC_GPIO: u8 = 32;
const DIODE_GPIO: u8 = 42;
/// LVCMOS 3.3 V mode limits, AM62x data sheet SPRSP58C section 6.8.6.
const VIL_V: f64 = 0.8;
const VIH_V: f64 = 2.0;

#[test]
fn sk_am62_ntc_and_diode_netlists_reach_main_gpio0_through_expansion_header_pads() {
    let system_path = root("configs/systems/sk-am62.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load SK-AM62 profile");
    assert_eq!(profile.name, "sk-am62");
    let analog_path = root("examples/sk-am62/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path)
        .expect("load generic NTC divider and Shockley diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load AM6234 descriptor");
    assert!(
        chip.peripherals
            .iter()
            .all(|p| p.r#type != "input_pin_bank"),
        "the chip must not declare a simulation-only input bank"
    );
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build SK-AM62 SystemBus");
    let gpio0 = bus
        .find_peripheral_index_by_name("main_gpio0")
        .expect("MAIN_GPIO0 is modelled");
    assert_eq!(bus.peripherals[gpio0].base, GPIO0_BASE);
    // MAIN_GPIO1 is the same model with `ti,ngpio = <52>`: its second
    // register bank holds GPIO 32..51 only.
    assert_eq!(
        bus.read_u32(0x0060_1038).expect("MAIN_GPIO1 DIR23"),
        0x000F_FFFF
    );

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both unrelated device models must bind to distinct MAIN_GPIO0 pads"
    );

    let expected_diode_v = diode_load_line_voltage();
    assert!(expected_diode_v < VIL_V, "diode must fall below VIL");
    let ntc_bit = 1u32 << (NTC_GPIO - 32);
    let diode_bit = 1u32 << (DIODE_GPIO - 32);
    assert_eq!(
        bus.read_u32(DIR23).expect("DIR23"),
        u32::MAX,
        "reset leaves every GPIO an input"
    );
    // Firmware arms falling-edge detection on the NTC pad.
    bus.write_u32(SET_FAL_TRIG23, ntc_bit)
        .expect("SET_FAL_TRIG23");

    let mut cycles = 0;
    // 50 °C (0.87 V) and 10 °C (2.21 V) sit between the LVCMOS limits and
    // the 0.3 / 0.7 x VDD (0.99 V / 2.31 V) pair the removed input bank
    // assumed: under those the 50 °C step reads low and the 10 °C step stays
    // low.
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // inside the band: keeps the cold high level
        (50.0, true), // 0.87 V is still above VIL 0.8 V
        (80.0, false),
        (25.0, false), // inside the band: keeps the hot low level
        (10.0, true),  // 2.21 V is above VIH 2.0 V
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both generic analog netlists");
        assert_eq!(steps.len(), 2, "both device models cross the boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = voltage(&session, "board.gpio_in.main_gpio0.32");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let in_band = VIL_V < expected_ntc_v && expected_ntc_v < VIH_V;
        assert_eq!(
            in_band,
            matches!(temperature_c as i32, 25 | 50),
            "{temperature_c} °C gives {expected_ntc_v} V"
        );
        if !in_band {
            assert_eq!(expected_ntc_level, expected_ntc_v >= VIH_V);
        }

        let diode_v = voltage(&session, "board.gpio_in.main_gpio0.42");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let in_data = bus.read_u32(IN_DATA23).expect("IN_DATA23");
        assert_eq!(
            in_data & ntc_bit != 0,
            expected_ntc_level,
            "IN_DATA23 bit 0 at {temperature_c} °C ({ntc_v} V), LVCMOS limits with hysteresis"
        );
        assert_eq!(
            in_data & diode_bit,
            0,
            "IN_DATA23 bit 10 reads low from the solved diode voltage"
        );
        assert_eq!(
            in_data & !(ntc_bit | diode_bit),
            0,
            "no other MAIN_GPIO0 pad moves"
        );

        // The one falling edge is the 50 °C -> 80 °C step.
        let intstat = bus.read_u32(INTSTAT23).expect("INTSTAT23");
        if temperature_c == 80.0 {
            assert_eq!(intstat, ntc_bit, "INTSTAT23 latches the NTC falling edge");
            bus.write_u32(INTSTAT23, ntc_bit).expect("clear INTSTAT23");
        } else {
            assert_eq!(intstat, 0, "no falling edge at {temperature_c} °C");
        }
    }
    assert_eq!(
        bus.read_u32(GPIO0_BASE + 0x20).expect("IN_DATA01"),
        0,
        "the first register bank carries no routed pad"
    );
}
