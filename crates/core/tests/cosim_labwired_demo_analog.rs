// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic demo-board profile carries two reusable MNA devices through
//! its STM32F103 PA0/PA1 analog-pad mappings into firmware-visible ADC1 reads.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

fn voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn adc_readback(bus: &mut SystemBus, channel: u8) -> (u16, f64) {
    let idx = bus
        .find_peripheral_index_by_name("adc1")
        .expect("STM32F103 profile declares adc1");
    let adc = bus.peripherals[idx]
        .dev
        .as_any_mut()
        .expect("adc1 downcasts")
        .downcast_mut::<labwired_core::peripherals::adc::Adc>()
        .expect("adc1 is an Adc");
    (adc.channel_input_count(channel), adc.reference_voltage_v())
}

fn f103_adc_convert(bus: &mut SystemBus, channel: u8) -> u32 {
    const RCC_BASE: u64 = 0x4002_1000;
    const ADC1_BASE: u64 = 0x4001_2400;
    const ADC1EN: u32 = 1 << 9;
    const ADON: u32 = 1;
    const EXTSEL_SOFTWARE: u32 = 0b111 << 17;
    const EXTTRIG: u32 = 1 << 20;
    const SWSTART: u32 = 1 << 22;
    const EOC: u32 = 1 << 1;

    let apb2enr = bus.read_u32(RCC_BASE + 0x18).expect("read APB2ENR");
    bus.write_u32(RCC_BASE + 0x18, apb2enr | ADC1EN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_BASE + 0x34, u32::from(channel))
        .expect("select ADC1 regular channel");
    let trigger = ADON | EXTSEL_SOFTWARE | EXTTRIG;
    bus.write_u32(ADC1_BASE + 0x08, trigger)
        .expect("power up ADC1 and enable software trigger");
    bus.write_u32(ADC1_BASE + 0x08, trigger | SWSTART)
        .expect("start ADC1 conversion");
    for _ in 0..64 {
        bus.tick_peripherals_fully();
    }
    let status = bus.read_u32(ADC1_BASE).expect("read ADC1 SR");
    assert_ne!(status & EOC, 0, "ADC1 conversion completes with EOC set");
    bus.read_u32(ADC1_BASE + 0x4c)
        .expect("read firmware-visible ADC1 DR")
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
fn labwired_demo_board_ntc_and_diode_reach_pa0_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/labwired-demo-board.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load demo-board profile");
    assert_eq!(profile.name, "labwired-demo-board");
    let analog = SystemManifest::from_file(root(
        "examples/labwired-demo-board/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC and Shockley diode overlay");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F103 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build demo-board system bus");
    assert!(
        profile
            .external_devices
            .iter()
            .any(|device| device.id == "temp_sensor" && device.r#type == "tmp102"),
        "analog overlay must preserve the profile's existing TMP102 device"
    );

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("overlay declares two analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "PA0 and PA1 must resolve to distinct ADC1 inputs"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature input");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog devices");
        assert_eq!(steps.len(), 2, "both independent netlists solve");
        assert!(errors.is_empty(), "unexpected routing errors: {errors:?}");

        let ntc_v = voltage(&session, "board.analog.pa0_volts");
        let ntc_ohms = 10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp();
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_ohms + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the Beta NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        for (channel, volts, pad) in [(0, ntc_v, "PA0"), (1, diode_v, "PA1")] {
            let (count, reference_v) = adc_readback(&mut bus, channel);
            let input_mv = (volts * 1000.0 + 0.5).floor();
            let expected_count = (input_mv * 4095.0 / (reference_v * 1000.0)).floor() as u16;
            assert_eq!(
                count, expected_count,
                "{pad} must inject the solved voltage into ADC1 channel {channel}"
            );
            assert_eq!(
                f103_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "ADC1 channel {channel} data-register read must reflect {pad}"
            );
        }
        assert_eq!(
            adc_readback(&mut bus, 2).0,
            0xFFFF,
            "the adjacent ADC1 channel must remain undriven"
        );
    }
}
