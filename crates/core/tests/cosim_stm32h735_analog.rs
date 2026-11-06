// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA netlists resolve through the H735 PA0/PA1 analog-pad map into
//! ADC1 channels 16/17, then convert through ADC1's firmware-visible registers.

mod common;
use common::root;
use labwired_config::SystemManifest;
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::adc::Adc;
use labwired_core::Bus;
use std::path::Path;

const RCC_BASE: u64 = 0x5802_4400;
const RCC_AHB1ENR: u64 = RCC_BASE + 0xD8;
const ADC1_BASE: u64 = 0x4002_2000;
const ADC_CR: u64 = ADC1_BASE + 0x08;
const ADC_CFGR: u64 = ADC1_BASE + 0x0C;
const ADC_PCSEL: u64 = ADC1_BASE + 0x1C;
const ADC_SQR1: u64 = ADC1_BASE + 0x30;
const ADC_DR: u64 = ADC1_BASE + 0x40;
const ADC_ISR: u64 = ADC1_BASE;

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

fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("H735 profile declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<Adc>()
        .expect("ADC1 uses the shared STM32 ADC model")
        .channel_input_count(channel)
}

fn expected_adc12_count(voltage_v: f64) -> u32 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 4095 / 3300).min(4095)
}

fn h7_adc_convert(bus: &mut SystemBus, channel: u8) -> u32 {
    const ADC_ADVREGEN: u32 = 1 << 28;
    const ADC_ADEN: u32 = 1;
    const ADC_ADSTART: u32 = 1 << 2;
    const ISR_EOC_EOS: u32 = (1 << 2) | (1 << 3);

    bus.write_u32(ADC_SQR1, u32::from(channel) << 6)
        .expect("select ADC1 regular sequence channel");
    bus.write_u32(ADC_CR, ADC_ADVREGEN | ADC_ADEN | ADC_ADSTART)
        .expect("start ADC1 one-shot conversion");
    let status = bus.read_u32(ADC_ISR).expect("read ADC1 status");
    assert_eq!(
        status & ISR_EOC_EOS,
        ISR_EOC_EOS,
        "H7 conversion must complete with EOC and EOS"
    );
    let result = bus.read_u32(ADC_DR).expect("read firmware-visible ADC1 DR");
    bus.write_u32(ADC_ISR, ISR_EOC_EOS)
        .expect("clear ADC1 completion flags");
    result
}

#[test]
fn stm32h735_smoke_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/stm32h735-smoke.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load STM32H735 profile");
    assert_eq!(profile.name, "stm32h735-smoke");
    assert!(profile
        .board_io
        .iter()
        .any(|io| { io.id == "led_pa5" && io.peripheral == "gpioa" && io.pin == 5 }));

    let analog = SystemManifest::from_file(root(
        "examples/stm32h735-smoke/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32H735 descriptor");
    assert_eq!(
        chip.analog_pins
            .get("PA0")
            .map(|route| (route.peripheral.as_str(), route.channel)),
        Some(("adc1", 16)),
        "PA0 must resolve to STM32H735 ADC1_INP16"
    );
    assert_eq!(
        chip.analog_pins
            .get("PA1")
            .map(|route| (route.peripheral.as_str(), route.channel)),
        Some(("adc1", 17)),
        "PA1 must resolve to STM32H735 ADC1_INP17"
    );
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build STM32H735 system bus");

    let ahb1enr = bus.read_u32(RCC_AHB1ENR).expect("read RCC AHB1ENR");
    bus.write_u32(RCC_AHB1ENR, ahb1enr | (1 << 5))
        .expect("enable ADC1/ADC2 clock");
    bus.write_u32(ADC_CR, 0).expect("leave ADC deep power-down");
    bus.write_u32(ADC_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC_CR, (1 << 28) | (1 << 31))
        .expect("calibrate ADC1");
    bus.write_u32(ADC_CR, (1 << 28) | 1).expect("enable ADC1");
    assert_ne!(
        bus.read_u32(ADC_ISR).expect("read ADC1 ready status") & 1,
        0,
        "ADC1 must report ADRDY before conversion"
    );
    bus.write_u32(ADC_CFGR, 2 << 2)
        .expect("select 12-bit H7 conversion resolution");
    bus.write_u32(ADC_PCSEL, (1 << 16) | (1 << 17))
        .expect("select both routed ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both generic analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models must resolve PA0/PA1 to ADC1 channels 16/17"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 18),
        u16::MAX,
        "adjacent ADC1 channel 18 is undriven"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve both netlists and route their ADC inputs");
        assert_eq!(steps.len(), 2, "both netlists cross the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * 10_000.0 / (resistance + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        for (channel, voltage_v) in [(16, ntc_v), (17, diode_v)] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_adc12_count(voltage_v) as u16,
                "PA route must seed ADC1 channel {channel} with the solved voltage"
            );
            assert_eq!(
                h7_adc_convert(&mut bus, channel),
                expected_adc12_count(voltage_v),
                "ADC1 channel {channel} DR must match the routed {voltage_v} V"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 18),
            u16::MAX,
            "channel 18 stays undriven"
        );
    }
}
