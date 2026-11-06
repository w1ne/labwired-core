// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through STM32F401CDU6 system profiles into ADC1.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_BASE: u64 = 0x4002_3800;
const ADC1_BASE: u64 = 0x4001_2000;

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
        .expect("STM32F401CDU6 declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<labwired_core::peripherals::adc::Adc>()
        .expect("ADC1 uses the shared STM32 ADC model")
        .channel_input_count(channel)
}

/// Perform the selected one-shot through the same F4 register interface firmware uses.
fn stm32f4_adc_convert(bus: &mut SystemBus, channel: u8) -> u32 {
    const ADC1EN: u32 = 1 << 8;
    const ADON: u32 = 1;
    const SWSTART: u32 = 1 << 30;
    const EOC: u32 = 1 << 1;

    let apb2enr = bus.read_u32(RCC_BASE + 0x44).expect("read RCC APB2ENR");
    bus.write_u32(RCC_BASE + 0x44, apb2enr | ADC1EN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_BASE + 0x34, u32::from(channel))
        .expect("select SQR3 SQ1 channel");
    bus.write_u32(ADC1_BASE + 0x08, ADON)
        .expect("power up ADC1");
    bus.write_u32(ADC1_BASE + 0x08, ADON | SWSTART)
        .expect("software-start ADC1 conversion");
    for _ in 0..15 {
        bus.tick_peripherals_fully();
    }
    let status = bus.read_u32(ADC1_BASE).expect("read ADC1 SR");
    assert_ne!(status & EOC, 0, "ADC1 conversion completes with EOC set");
    bus.read_u32(ADC1_BASE + 0x4c)
        .expect("read firmware-visible ADC1 DR")
}

fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 4095 / 3300).min(4095) as u16
}

/// Independent level-1 NMOS load-line reference for the board-routed test.
fn mosfet_drain_voltage(gate_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const VTO: f64 = 1.0;
    const KP_A_PER_V2: f64 = 20e-6;
    const W_M: f64 = 100e-6;
    const L_M: f64 = 10e-6;
    const LAMBDA_PER_V: f64 = 0.02;

    let overdrive = gate_v - VTO;
    if overdrive <= 0.0 {
        return SUPPLY_V;
    }
    let beta = KP_A_PER_V2 * W_M / L_M;
    let drain_current = |vds: f64| {
        if vds < overdrive {
            beta * (1.0 + LAMBDA_PER_V * vds) * vds * (overdrive - 0.5 * vds)
        } else {
            0.5 * beta * (1.0 + LAMBDA_PER_V * vds) * overdrive * overdrive
        }
    };
    let residual = |vds: f64| SUPPLY_V - vds - LOAD_OHMS * drain_current(vds);
    let (mut low, mut high) = (0.0, SUPPLY_V);
    for _ in 0..100 {
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

#[test]
fn stm32f401cdu6_blackpill_ntc_and_diode_netlists_reach_pa1_pa4_adc1_end_to_end() {
    let system_path = root("configs/systems/stm32f401cdu6-blackpill.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load STM32F401CDU6 Black Pill profile");
    assert_eq!(profile.name, "stm32f401cdu6-blackpill");
    assert!(profile.external_devices.is_empty());
    assert!(
        profile
            .board_io
            .iter()
            .any(|io| io.id == "led_pc13" && io.peripheral == "gpioc" && io.pin == 13),
        "the mixed-signal run keeps the Black Pill's active-low PC13 LED mapping"
    );
    assert!(
        profile
            .board_io
            .iter()
            .any(|io| io.id == "button_user_pa0" && io.peripheral == "gpioa" && io.pin == 0),
        "the mixed-signal run keeps the Black Pill's PA0 user button mapping"
    );

    let analog =
        SystemManifest::from_file(root("examples/stm32f401cdu6/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F401CDU6 descriptor");
    assert_eq!(chip.analog_pins["PA1"].channel, 1);
    assert_eq!(chip.analog_pins["PA4"].channel, 4);
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Black Pill bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through PA1/PA4 to ADC1 channels 1/4"
    );
    for channel in [0, 1, 2, 3, 4, 5] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} begins without an analog source"
        );
    }

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut ntc_samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(steps.len(), 2, "both netlists cross the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa1_volts");
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa4_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley diode load line should be {expected_diode_v} V, got {diode_v} V"
        );

        for (channel, volts) in [(1, ntc_v), (4, diode_v)] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1 channel {channel} conversion must report the solved voltage"
            );
        }
        assert_eq!(
            [0, 2, 3, 5].map(|channel| adc_channel_count(&mut bus, channel)),
            [0xFFFF; 4],
            "unrouted neighboring ADC1 channels remain undriven"
        );
        ntc_samples.push((ntc_v, adc_channel_count(&mut bus, 1)));
    }

    assert!(
        ntc_samples[2].0 > ntc_samples[1].0 && ntc_samples[1].0 > ntc_samples[0].0,
        "the divider voltage rises with temperature: {ntc_samples:?}"
    );
    assert!(
        ntc_samples[2].1 > ntc_samples[1].1 && ntc_samples[1].1 > ntc_samples[0].1,
        "ADC counts track the solved temperature-dependent voltage: {ntc_samples:?}"
    );
    assert_eq!(
        ntc_samples[1].1, 2047,
        "the 25 °C NTC divider is half scale"
    );
}

#[test]
fn stm32f401cdu6_ntc_and_diode_netlists_reach_pa1_pa4_adc1_end_to_end() {
    let system_path = root("configs/systems/stm32f401cdu6.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load STM32F401CDU6 profile");
    assert_eq!(profile.name, "stm32f401cdu6");
    assert!(profile.external_devices.is_empty());
    assert!(profile.board_io.is_empty());

    let analog =
        SystemManifest::from_file(root("examples/stm32f401cdu6/analog-ntc-diode-system.yaml"))
            .expect("load shared generic NTC divider and diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F401CDU6 descriptor");
    assert_eq!(chip.analog_pins["PA1"].channel, 1);
    assert_eq!(chip.analog_pins["PA4"].channel, 4);

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build STM32F401CDU6 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through PA1/PA4 to ADC1 channels 1/4"
    );
    for channel in [0, 1, 2, 3, 4, 5] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} begins without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set generic NTC temperature");
    let cycles = session.cycles_until_boundary(0);
    let (steps, errors) = session
        .advance_to(cycles, &mut bus)
        .expect("solve and route both analog netlists");
    assert_eq!(steps.len(), 2, "both netlists cross the 1 us boundary");
    assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

    let ntc_v = signal_voltage(&session, "board.analog.pa1_volts");
    assert!(
        (ntc_v - 1.65).abs() < 1e-9,
        "the 25 °C Beta divider should solve to 1.65 V, got {ntc_v} V"
    );
    let diode_v = signal_voltage(&session, "board.analog.pa4_volts");
    let expected_diode_v = diode_load_line_voltage();
    assert!(
        (diode_v - expected_diode_v).abs() < 2e-5,
        "the Shockley diode load line should be {expected_diode_v} V, got {diode_v} V"
    );

    for (channel, volts) in [(1, ntc_v), (4, diode_v)] {
        let expected_count = expected_adc_count(volts);
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            expected_count,
            "the solved voltage must seed ADC1 channel {channel}"
        );
        assert_eq!(
            stm32f4_adc_convert(&mut bus, channel),
            u32::from(expected_count),
            "firmware-visible ADC1 channel {channel} conversion must report the solved voltage"
        );
    }
    assert_eq!(
        [0, 2, 3, 5].map(|channel| adc_channel_count(&mut bus, channel)),
        [0xFFFF; 4],
        "unrouted neighboring ADC1 channels remain undriven"
    );
}

#[test]
fn stm32f401_blackpill_mosfet_load_line_reaches_pa0_adc1_end_to_end() {
    let system_path = root("configs/systems/stm32f401cdu6-blackpill.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load STM32F401CDU6 Black Pill profile");
    let analog =
        SystemManifest::from_file(root("examples/stm32f401cdu6/analog-ntc-diode-system.yaml"))
            .expect("load the existing generic NTC and diode netlists");
    let mosfet =
        SystemManifest::from_file(root("examples/stm32f401cdu6/analog-mosfet-switch.yaml"))
            .expect("load the generic low-side MOSFET netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F401CDU6 descriptor");
    assert_eq!(chip.analog_pins["PA0"].channel, 0);
    assert_eq!(chip.analog_pins["PA1"].channel, 1);
    assert_eq!(chip.analog_pins["PA4"].channel, 4);

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Black Pill bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and MOSFET netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all three unrelated devices must resolve through PA0/PA1/PA4 to ADC1"
    );

    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the independent generic NTC temperature");
    let mut cycles = 0;
    let mut drain_samples = Vec::new();
    for gate_v in [0.0_f64, 0.8, 1.0, 2.0, 3.3] {
        session
            .set_signal_number("ui.mosfet.gate_v", gate_v)
            .expect("set generic MOSFET gate voltage");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the NTC, diode, and MOSFET netlists");
        assert_eq!(steps.len(), 3, "all three unrelated netlists solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let drain_v = signal_voltage(&session, "board.analog.pa0_volts");
        let expected_drain_v = mosfet_drain_voltage(gate_v);
        assert!(
            (drain_v - expected_drain_v).abs() < 5e-5,
            "level-1 NMOS load line at Vgs={gate_v} V should be {expected_drain_v} V, got {drain_v} V"
        );
        let expected_count = expected_adc_count(drain_v);
        assert_eq!(
            adc_channel_count(&mut bus, 0),
            expected_count,
            "solved MOSFET drain voltage seeds ADC1 channel 0"
        );
        assert_eq!(
            stm32f4_adc_convert(&mut bus, 0),
            u32::from(expected_count),
            "firmware-visible ADC1 channel 0 conversion follows the MOSFET load line"
        );

        assert!(
            (signal_voltage(&session, "board.analog.pa1_volts") - expected_ntc_v).abs() < 1e-9,
            "the independent NTC divider remains on PA1/ADC1 channel 1"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa4_volts") - expected_diode_v).abs() < 2e-5,
            "the independent Shockley diode remains on PA4/ADC1 channel 4"
        );
        assert_eq!(
            adc_channel_count(&mut bus, 1),
            expected_adc_count(expected_ntc_v)
        );
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            expected_adc_count(expected_diode_v)
        );
        assert_eq!(
            [2, 3, 5].map(|channel| adc_channel_count(&mut bus, channel)),
            [0xFFFF; 3],
            "undriven neighboring ADC1 inputs remain isolated"
        );
        drain_samples.push((gate_v, drain_v));
    }

    assert_eq!(
        drain_samples[0].1, drain_samples[2].1,
        "Vgs below threshold leaves the drain at the supply"
    );
    assert!(
        drain_samples[4].1 < drain_samples[3].1 && drain_samples[3].1 < drain_samples[2].1,
        "drain voltage falls as the NMOS conducts more strongly: {drain_samples:?}"
    );
}
