// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog devices reach MSS GPIO_2 on the MPFS-DISCO-KIT through the
//! Raspberry Pi header pads of Microchip's reference design (GPIO_2_0 =
//! RPI_GPIO4, GPIO_2_1 = RPI_GPIO17, Bank 1 LVCMOS33). Firmware reads the
//! levels in GPIO_IN once GPIO_CFG[n].INREG is set.

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

/// MSS GPIO_2 (`mss_gpio.h` `GPIO_TypeDef`): GPIO_CFG[n] at 4n, GPIO_IN.
const GPIO2_BASE: u64 = 0x2012_2000;
const GPIO_IN: u64 = GPIO2_BASE + 0x84;
/// `MSS_GPIO_INPUT_MODE`: GPIO_CFG.INREG (bit 1).
const INPUT_MODE: u32 = 0x02;
/// GPIO_2_0 = RPI_GPIO4 carries the NTC, GPIO_2_1 = RPI_GPIO17 the diode.
const NTC_PORT: u8 = 0;
const DIODE_PORT: u8 = 1;
/// LVCMOS33 limits, PolarFire SoC Data Sheet DS00004248E Table 4-15.
const VIL_V: f64 = 0.8;
const VIH_V: f64 = 2.0;

#[test]
fn mpfs_disco_kit_ntc_and_diode_netlists_reach_mss_gpio2_through_rpi_header_pads() {
    let system_path = root("configs/systems/mpfs-disco-kit.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load MPFS profile");
    assert_eq!(profile.name, "mpfs-disco-kit");
    let analog_path = root("examples/mpfs-disco-kit/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path)
        .expect("load generic NTC divider and Shockley diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load MPFS095T descriptor");
    assert!(
        chip.peripherals
            .iter()
            .all(|p| p.r#type != "input_pin_bank"),
        "the chip must not declare a simulation-only input bank"
    );
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build MPFS SystemBus");
    let gpio2 = bus
        .find_peripheral_index_by_name("mss_gpio2")
        .expect("MSS GPIO_2 is modelled");
    assert_eq!(bus.peripherals[gpio2].base, GPIO2_BASE);

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both unrelated device models must bind to distinct GPIO_2 ports"
    );

    let expected_diode_v = diode_load_line_voltage();
    assert!(expected_diode_v < VIL_V, "diode must fall below VIL");
    let ntc_bit = 1u32 << NTC_PORT;
    let diode_bit = 1u32 << DIODE_PORT;
    let mut cycles = 0;
    let mut inputs_enabled = false;
    // 50 °C (0.87 V) and 10 °C (2.21 V) sit between the LVCMOS33 limits and
    // the 0.3 / 0.7 x VDDI (0.99 V / 2.31 V) pair a CMOS-ratio pad would
    // have: under those the 50 °C step reads low and the 10 °C step stays low.
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

        let ntc_v = voltage(&session, "board.gpio_in.mss_gpio2.0");
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

        let diode_v = voltage(&session, "board.gpio_in.mss_gpio2.1");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        if !inputs_enabled {
            // Reset leaves GPIO_CFG.INREG clear: the pad is high, GPIO_IN is not.
            assert_eq!(
                bus.peripherals[gpio2].dev.read_gpio_input(NTC_PORT),
                Some(true)
            );
            assert_eq!(
                bus.read_u32(GPIO_IN).expect("GPIO_IN"),
                0,
                "GPIO_IN masks ports whose input is not enabled"
            );
            for port in [NTC_PORT, DIODE_PORT] {
                bus.write_u32(GPIO2_BASE + 4 * u64::from(port), INPUT_MODE)
                    .expect("GPIO_CFG[n] = MSS_GPIO_INPUT_MODE");
            }
            inputs_enabled = true;
        }
        let gpio_in = bus.read_u32(GPIO_IN).expect("GPIO_IN");
        assert_eq!(
            gpio_in & ntc_bit != 0,
            expected_ntc_level,
            "GPIO_IN bit 0 at {temperature_c} °C ({ntc_v} V), LVCMOS33 limits with hysteresis"
        );
        assert_eq!(
            gpio_in & diode_bit,
            0,
            "GPIO_IN bit 1 reads low from the solved diode voltage"
        );
        assert_eq!(
            gpio_in & !(ntc_bit | diode_bit),
            0,
            "no other GPIO_2 port moves"
        );
    }
}
