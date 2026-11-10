// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the NUCLEO-F103RB e-paper profile into ADC1.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_BASE: u64 = 0x4002_1000;
const ADC1_BASE: u64 = 0x4001_2400;

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

/// Independent level-1 NMOS load line using the ngspice `mos1temp` laws for
/// KP, PHI, and VTO. Source and bulk are both grounded, so this isolates the
/// model's temperature adjustment from body bias.
fn mosfet_drain_voltage_at_temperature(temperature_c: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const GATE_V: f64 = 3.0;
    const VTO: f64 = 1.0;
    const KP_A_PER_V2: f64 = 20e-6;
    const GAMMA: f64 = 0.4;
    const PHI: f64 = 0.65;
    const W_M: f64 = 100e-6;
    const L_M: f64 = 10e-6;
    const LAMBDA_PER_V: f64 = 0.02;
    const TNOM_C: f64 = 25.0;

    let bandgap = |temperature_k: f64| {
        1.16 - 7.02e-4 * temperature_k * temperature_k / (temperature_k + 1108.0)
    };
    let kelvin = temperature_c + 273.15;
    let nominal_kelvin = TNOM_C + 273.15;
    let ratio = kelvin / nominal_kelvin;
    let vt = 1.380_649e-23 * kelvin / 1.602_176_634e-19;
    let nominal_vt = 1.380_649e-23 * nominal_kelvin / 1.602_176_634e-19;
    let phi_t = PHI * ratio
        - vt * (3.0 * ratio.ln() + bandgap(nominal_kelvin) / nominal_vt - bandgap(kelvin) / vt);
    let vto_t = VTO
        + GAMMA * (phi_t.sqrt() - PHI.sqrt())
        + 0.5 * (phi_t - PHI)
        + 0.5 * (bandgap(nominal_kelvin) - bandgap(kelvin));
    let kp_t = KP_A_PER_V2 / ratio.powf(1.5);
    let overdrive = GATE_V - vto_t;
    let beta = kp_t * W_M / L_M;
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

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("STM32F103RB declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<labwired_core::peripherals::adc::Adc>()
        .expect("ADC1 uses the shared STM32 ADC model")
        .channel_input_count(channel)
}

/// Perform the selected one-shot through the STM32F1 register interface firmware uses.
fn stm32f103_adc_convert(bus: &mut SystemBus, channel: u8) -> u32 {
    const ADC1EN: u32 = 1 << 9;
    const ADON: u32 = 1;
    const EXTSEL_SOFTWARE: u32 = 0b111 << 17;
    const EXTTRIG: u32 = 1 << 20;
    const SWSTART: u32 = 1 << 22;
    const EOC: u32 = 1 << 1;

    let apb2enr = bus.read_u32(RCC_BASE + 0x18).expect("read RCC APB2ENR");
    bus.write_u32(RCC_BASE + 0x18, apb2enr | ADC1EN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_BASE + 0x34, u32::from(channel))
        .expect("select SQR3 SQ1 channel");
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

fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 4095 / 3300).min(4095) as u16
}

/// Reuse the generic Beta NTC divider and unrelated Shockley diode netlists on
/// each STM32F103 profile, resolving through that profile's own chip routes.
fn assert_stm32f103_generic_analog_profile(
    mut profile: SystemManifest,
    system_path: &Path,
    expected_profile_name: &str,
    required_peripherals: &[&str],
) {
    assert_eq!(profile.name, expected_profile_name);
    let analog = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F103 descriptor");
    assert_eq!(chip.analog_pins["PA0"].channel, 0);
    assert_eq!(chip.analog_pins["PA1"].channel, 1);
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build STM32F103 bus");
    for peripheral in required_peripherals {
        assert!(
            bus.find_peripheral_index_by_name(peripheral).is_some(),
            "the profile-specific {peripheral} peripheral remains attached"
        );
    }
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through this profile's PA0/PA1 ADC routes"
    );
    for channel in [0, 1, 2] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
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

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        for (channel, volts) in [(0, ntc_v), (1, diode_v)] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f103_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1 channel {channel} conversion must report the solved voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 2),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        ntc_samples.push((ntc_v, adc_channel_count(&mut bus, 0)));
    }

    assert!(
        ntc_samples[2].0 > ntc_samples[1].0 && ntc_samples[1].0 > ntc_samples[0].0,
        "the divider voltage rises with temperature: {ntc_samples:?}"
    );
    assert!(
        ntc_samples[2].1 > ntc_samples[1].1 && ntc_samples[1].1 > ntc_samples[0].1,
        "ADC counts track the temperature-dependent voltage: {ntc_samples:?}"
    );
    assert_eq!(
        ntc_samples[1].1, 2047,
        "the 25 °C NTC divider is half scale"
    );
}

#[test]
fn nucleo_f103rb_epaper_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f103rb-epaper.yaml");
    let profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-F103RB e-paper profile");
    assert_eq!(profile.name, "nucleo-f103rb-epaper");
    assert!(profile.board_io.is_empty());
    assert_eq!(profile.external_devices.len(), 1);
    assert_eq!(profile.external_devices[0].id, "epaper");
    assert_eq!(profile.external_devices[0].r#type, "ssd1680_tricolor_290");
    assert_eq!(profile.external_devices[0].connection, "spi1");
    assert_eq!(
        profile.external_devices[0].config["cs_pin"].as_str(),
        Some("PA4"),
        "the analog overlay must retain the panel's chip-select wiring"
    );
    assert_stm32f103_generic_analog_profile(profile, &system_path, "nucleo-f103rb-epaper", &[]);
}

/// The NUCLEO-F103RB e-paper profile routes generic level-1 MOSFET
/// temperature behavior through PA2/PA3 while its unrelated NTC and diode
/// remain on PA0/PA1. The two MOSFETs share one model and bias but carry
/// distinct instance temperatures, so omitting `mos1temp` cannot pass.
#[test]
fn nucleo_f103rb_epaper_mosfet_temperature_reaches_pa2_pa3_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f103rb-epaper.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-F103RB e-paper profile");
    assert_eq!(profile.name, "nucleo-f103rb-epaper");
    assert_eq!(profile.external_devices.len(), 1);
    assert_eq!(profile.external_devices[0].id, "epaper");
    let analog = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-mosfet-temperature-system.yaml",
    ))
    .expect("load generic NTC, diode, and temperature-paired MOSFET netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F103 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build e-paper board bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries the generic NTC, diode, and MOSFET models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "each netlist output must resolve through its STM32F103 PA0..PA3 ADC route"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the unrelated NTC to its nominal temperature");
    let cycles = session.cycles_until_boundary(0);
    let (steps, errors) = session
        .advance_to(cycles, &mut bus)
        .expect("solve the generic NTC, diode, and MOSFET netlists");
    assert_eq!(steps.len(), 3, "all three netlists cross the 1 us boundary");
    assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

    let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
    assert!(
        (ntc_v - 1.65).abs() < 1e-9,
        "the NTC divider is half scale: {ntc_v} V"
    );
    let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
    let expected_diode_v = diode_load_line_voltage();
    assert!(
        (diode_v - expected_diode_v).abs() < 2e-5,
        "the unrelated Shockley branch is {expected_diode_v} V, got {diode_v} V"
    );

    let mosfet_samples = [
        (2, 25.0, "board.analog.pa2_volts"),
        (3, 85.0, "board.analog.pa3_volts"),
    ]
    .map(|(channel, temperature_c, path)| {
        let volts = signal_voltage(&session, path);
        let expected = mosfet_drain_voltage_at_temperature(temperature_c);
        assert!(
            (volts - expected).abs() < 1e-3,
            "the {temperature_c} °C MOSFET load line should be {expected} V, got {volts} V"
        );
        let expected_count = expected_adc_count(expected);
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            expected_count,
            "the solved MOSFET drain voltage must seed ADC1 channel {channel}"
        );
        assert_eq!(
            stm32f103_adc_convert(&mut bus, channel),
            u32::from(expected_count),
            "firmware-visible ADC1 channel {channel} must report the MOSFET load line"
        );
        (temperature_c, volts, expected_count)
    });
    assert!(
        mosfet_samples[1].1 > mosfet_samples[0].1 + 0.04,
        "reduced hot-channel mobility must raise the drain voltage: {mosfet_samples:?}"
    );
    assert!(
        mosfet_samples[1].2 > mosfet_samples[0].2 + 40,
        "the hot MOSFET must produce a distinct firmware ADC result: {mosfet_samples:?}"
    );
    for (channel, volts) in [(0, ntc_v), (1, diode_v)] {
        let expected_count = expected_adc_count(volts);
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            expected_count,
            "the unrelated netlist must stay on ADC1 channel {channel}"
        );
        assert_eq!(
            stm32f103_adc_convert(&mut bus, channel),
            u32::from(expected_count),
            "firmware-visible ADC1 channel {channel} must retain its own analog result"
        );
    }
    assert_eq!(
        adc_channel_count(&mut bus, 4),
        0xFFFF,
        "the neighboring ADC1 channel remains undriven"
    );
}

/// The same unrelated analog devices and F1 ADC route must work on the bare
/// STM32F103C8 profile, which has no board-attached devices to supply a route.
#[test]
fn stm32f103_bare_ntc_and_diode_netlists_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/stm32f103-bare.yaml");
    let profile = SystemManifest::from_file(&system_path).expect("load bare STM32F103");
    assert!(profile.external_devices.is_empty());
    assert!(profile.board_io.is_empty());
    assert_stm32f103_generic_analog_profile(profile, &system_path, "stm32f103-bare", &[]);
}

/// The integrated-test system extends the common STM32F103 chip with DMA1 and
/// a DMA-wired UART1. Its profile must retain those peripherals while using the
/// same descriptor-driven analog routing as the bare F103 target.
#[test]
fn stm32f103_integrated_test_ntc_and_diode_netlists_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/stm32f103-integrated-test.yaml");
    let profile =
        SystemManifest::from_file(&system_path).expect("load integrated STM32F103 test profile");
    assert_eq!(profile.external_devices.len(), 0);
    assert_eq!(profile.board_io.len(), 0);
    assert_eq!(
        profile
            .peripherals
            .iter()
            .map(|peripheral| peripheral.id.as_str())
            .collect::<Vec<_>>(),
        ["dma1", "uart1"],
        "the integrated-test profile keeps its DMA and UART additions"
    );
    assert_stm32f103_generic_analog_profile(
        profile,
        &system_path,
        "stm32f103-integrated-test",
        &["dma1", "uart1"],
    );
}
