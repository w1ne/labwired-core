// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic NTC and diode netlists route through the Apollo4 Plus ADC slots and FIFO.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::apollo4_adc::Apollo4Adc;
use labwired_core::Bus;
use std::path::Path;

const ADC_BASE: u64 = 0x4003_8000;
const SLOT0CFG: u64 = ADC_BASE + 0x0C;
const FIFO_POP_READ: u64 = ADC_BASE + 0x3C;
const ADC_CFG: u64 = ADC_BASE;
const ADC_SWT: u64 = ADC_BASE + 0x08;
const ADC_INTSTAT: u64 = ADC_BASE + 0x204;

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

#[test]
fn apollo4p_evb_ntc_and_diode_netlists_reach_adc_slots_and_scan_fifo() {
    let profile_path = root("configs/systems/apollo4p-evb.yaml");
    let mut profile = SystemManifest::from_file(&profile_path).expect("load Apollo4 EVB profile");
    assert_eq!(profile.name, "apollo4p-evb");
    let analog_path = root("examples/apollo4p-evb/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path)
        .expect("load unrelated generic NTC and diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = profile_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load Apollo4 Plus descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Apollo4 EVB bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both solved devices must resolve to their explicit ADC channels"
    );

    let adc_index = bus
        .find_peripheral_index_by_name("adc")
        .expect("ADC peripheral is present");
    let mut cycles = 0;
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature input");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the NTC divider");
        assert_eq!(steps.len(), 2, "both analog netlists cross the boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let routed_voltage = match session.signals().get("adc.adc.0_volts") {
            Some(CosimSignalValue::F64(volts)) => *volts,
            other => panic!("the solved NTC voltage did not reach ADC SE0: {other:?}"),
        };
        let expected_voltage = 1.19 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (routed_voltage - expected_voltage).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_voltage} V, got {routed_voltage} V"
        );

        let routed_diode_voltage = match session.signals().get("adc.adc.1_volts") {
            Some(CosimSignalValue::F64(volts)) => *volts,
            other => panic!("the solved diode voltage did not reach ADC SE1: {other:?}"),
        };
        let expected_diode_voltage = diode_load_line_voltage();
        assert!(
            (routed_diode_voltage - expected_diode_voltage).abs() < 2e-5,
            "the Shockley diode load line should be {expected_diode_voltage} V, got {routed_diode_voltage} V"
        );

        let adc = bus.peripherals[adc_index]
            .dev
            .as_any_mut()
            .expect("ADC downcasts")
            .downcast_mut::<Apollo4Adc>()
            .expect("the board's ADC uses Apollo4Adc");
        let expected_mv = (expected_voltage * 1000.0 + 0.5).floor() as u16;
        assert_eq!(
            adc.channel_input_mv(0),
            Some(expected_mv),
            "co-simulation must seed the NTC's routed SE0 input"
        );
        assert_eq!(
            adc.channel_input_mv(1),
            Some((routed_diode_voltage * 1000.0 + 0.5).floor() as u16),
            "the independent diode netlist must seed its own SE1 input"
        );
        assert_eq!(
            adc.channel_input_mv(2),
            None,
            "neighboring SE2 remains undriven"
        );

        // Three enabled slots distinguish the unrelated solved inputs from an
        // undriven neighbor. Each emits one FIFO entry with its slot number.
        bus.write_u32(SLOT0CFG, 1)
            .expect("enable slot 0 on SE0 at 12 bits");
        bus.write_u32(SLOT0CFG + 4, 1 | (1 << 8))
            .expect("enable slot 1 on SE1 at 12 bits");
        bus.write_u32(SLOT0CFG + 8, 1 | (2 << 8))
            .expect("enable slot 2 on SE2 at 12 bits");
        bus.write_u32(ADC_CFG, 1 | (1 << 12))
            .expect("enable ADC and destructive FIFO reads");
        bus.write_u32(ADC_SWT, 0x37).expect("start software scan");
        assert_eq!(
            bus.read_u32(ADC_INTSTAT).expect("read conversion status") & 0x3,
            0x3,
            "conversion-complete and scan-complete status are raised"
        );

        let se0 = bus.read_u32(FIFO_POP_READ).expect("pop slot 0 result");
        assert_eq!((se0 >> 28) & 0x7, 0, "first FIFO sample belongs to slot 0");
        assert_eq!((se0 >> 20) & 0xFF, 3, "FIFO reports all scan results");
        let expected_code = u32::from(expected_mv) * 4095 / 1190;
        assert_eq!(
            (se0 & 0xF_FFFF) >> 6,
            expected_code,
            "Apollo4 12-bit FIFO sample must follow the solved NTC voltage"
        );

        let se1 = bus.read_u32(FIFO_POP_READ).expect("pop slot 1 result");
        assert_eq!((se1 >> 28) & 0x7, 1, "second FIFO sample belongs to slot 1");
        assert_eq!(
            (se1 >> 20) & 0xFF,
            2,
            "two FIFO results remain before the pop"
        );
        let diode_mv = (routed_diode_voltage * 1000.0 + 0.5).floor() as u32;
        let expected_diode_code = diode_mv * 4095 / 1190;
        assert_eq!(
            (se1 & 0xF_FFFF) >> 6,
            expected_diode_code,
            "Apollo4 12-bit FIFO sample must convert the independent diode voltage"
        );

        let se2 = bus.read_u32(FIFO_POP_READ).expect("pop slot 2 result");
        assert_eq!((se2 >> 28) & 0x7, 2, "third FIFO sample belongs to slot 2");
        assert_eq!(
            (se2 >> 20) & 0xFF,
            1,
            "one FIFO result remains before the pop"
        );
        assert_eq!((se2 & 0xF_FFFF) >> 6, 0, "undriven SE2 converts to zero");
    }
}
