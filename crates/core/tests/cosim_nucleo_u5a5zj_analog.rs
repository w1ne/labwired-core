// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA netlists resolve through the U5A5 PA0-PA3 analog-pad map into
//! ADC1 channels 5-8, then convert through ADC1's firmware-visible registers.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::adc::Adc;
use labwired_core::Bus;
use std::path::Path;

const RCC_BASE: u64 = 0x4602_0C00;
const RCC_AHB2ENR1: u64 = RCC_BASE + 0x8C;
const ADC1_BASE: u64 = 0x4202_8000;
const ADC_CR: u64 = ADC1_BASE + 0x08;
const ADC_CFGR1: u64 = ADC1_BASE + 0x0C;
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

/// Independent level-1 NMOS body-effect load line. With the source grounded,
/// a negative bulk bias raises threshold through GAMMA/PHI.
fn mosfet_body_effect_drain_voltage(gate_v: f64, bulk_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const VTO: f64 = 1.0;
    const KP_A_PER_V2: f64 = 20e-6;
    const W_M: f64 = 100e-6;
    const L_M: f64 = 10e-6;
    const LAMBDA_PER_V: f64 = 0.02;
    const GAMMA: f64 = 0.4;
    const PHI: f64 = 0.7;

    let threshold = VTO + GAMMA * ((PHI - bulk_v).sqrt() - PHI.sqrt());
    let overdrive = gate_v - threshold;
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
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
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
        .expect("U5A5 profile declares ADC1");
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

fn adc1_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("U5A5 profile declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<Adc>()
        .expect("ADC1 uses the shared STM32 ADC model")
        .channel_input_count(channel)
}

fn u5_adc_convert(bus: &mut SystemBus, channel: u8) -> u32 {
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
        "U5 conversion must complete with EOC and EOS"
    );
    let result = bus.read_u32(ADC_DR).expect("read firmware-visible ADC1 DR");
    bus.write_u32(ADC_ISR, ISR_EOC_EOS)
        .expect("clear ADC1 completion flags");
    result
}

#[test]
fn nucleo_u5a5zj_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-u5a5zj.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-U5A5ZJ-Q profile");
    assert_eq!(profile.name, "nucleo-u5a5zj");
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "led_ld1" && io.peripheral == "gpioc" && io.pin == 7));
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "button_b1" && io.peripheral == "gpioc" && io.pin == 13));

    let analog =
        SystemManifest::from_file(root("examples/nucleo-u5a5zj/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32U5A5 descriptor");
    assert_eq!(
        chip.analog_pins
            .get("PA0")
            .map(|route| (route.peripheral.as_str(), route.channel)),
        Some(("adc1", 5)),
        "PA0 must resolve to its documented ADC1 input channel 5"
    );
    assert_eq!(
        chip.analog_pins
            .get("PA1")
            .map(|route| (route.peripheral.as_str(), route.channel)),
        Some(("adc1", 6)),
        "PA1 must resolve to its documented ADC1 input channel 6"
    );
    let mut bus =
        SystemBus::from_config(&chip, &profile).expect("build NUCLEO-U5A5ZJ-Q system bus");

    let ahb2enr1 = bus.read_u32(RCC_AHB2ENR1).expect("read RCC AHB2ENR1");
    bus.write_u32(RCC_AHB2ENR1, ahb2enr1 | (1 << 10))
        .expect("enable ADC1 clock");
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
    bus.write_u32(ADC_CFGR1, 0x8000_0000 | (1 << 2))
        .expect("select 12-bit conversion resolution");
    bus.write_u32(ADC_PCSEL, (1 << 5) | (1 << 6))
        .expect("select both routed ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both generic analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models must resolve PA0/PA1 to ADC1 channels 5/6"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 7),
        u16::MAX,
        "adjacent ADC1 channel 7 is undriven"
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
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        for (channel, voltage_v) in [(5, ntc_v), (6, diode_v)] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_adc12_count(voltage_v) as u16,
                "PA route must seed ADC1 channel {channel} with the solved voltage"
            );
            assert_eq!(
                u5_adc_convert(&mut bus, channel),
                expected_adc12_count(voltage_v),
                "ADC1 channel {channel} DR must match the routed {voltage_v} V"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 7),
            u16::MAX,
            "channel 7 stays undriven"
        );
    }
}

#[test]
fn nucleo_u5a5zj_mosfet_body_effect_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-u5a5zj.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-U5A5ZJ-Q profile");
    assert_eq!(profile.name, "nucleo-u5a5zj");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-u5a5zj/analog-ntc-diode-system.yaml"))
            .expect("load generic Beta NTC and Shockley diode netlists");
    let mosfet = SystemManifest::from_file(root(
        "examples/nucleo-u5a5zj/analog-mosfet-body-effect-adc-system.yaml",
    ))
    .expect("load matched generic NMOS body-bias netlists");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip = ChipDescriptor::from_file(system_path.parent().unwrap().join(&profile.chip))
        .expect("load STM32U5A5 descriptor");
    for (pad, channel) in [("PA0", 5), ("PA1", 6), ("PA2", 7), ("PA3", 8)] {
        assert_eq!(
            chip.analog_pins[pad].channel, channel,
            "{pad} must resolve to ADC1_IN{channel}"
        );
    }

    let mut bus =
        SystemBus::from_config(&chip, &profile).expect("build NUCLEO-U5A5ZJ-Q system bus");
    let ahb2enr1 = bus.read_u32(RCC_AHB2ENR1).expect("read RCC AHB2ENR1");
    bus.write_u32(RCC_AHB2ENR1, ahb2enr1 | (1 << 10))
        .expect("enable ADC1 clock");
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
    bus.write_u32(ADC_CFGR1, 0x8000_0000 | (1 << 2))
        .expect("select 12-bit conversion resolution");
    bus.write_u32(ADC_PCSEL, (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8))
        .expect("select the NTC, diode, and NMOS ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched NMOS netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all four devices must bind through distinct PA0-PA3 ADC routes"
    );
    assert_eq!(
        adc1_channel_count(&mut bus, 9),
        u16::MAX,
        "adjacent ADC1 channel 9 is undriven"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the unrelated Beta NTC temperature");
    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut body_samples = Vec::new();
    for bulk_v in [0.0_f64, -0.5, -1.0, -1.5] {
        session
            .set_signal_number("ui.mosfet_body.bulk_v", bulk_v)
            .expect("set generic NMOS reverse body bias");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched NMOS netlists");
        assert_eq!(steps.len(), 3, "all three analog models solve per boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let reference_v = signal_voltage(&session, "board.analog.pa2_volts");
        let body_v = signal_voltage(&session, "board.analog.pa3_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "unrelated NTC remains at {expected_ntc_v} V, got {ntc_v} V"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley diode should settle at {expected_diode_v} V, got {diode_v} V"
        );

        let expected_reference_v = mosfet_body_effect_drain_voltage(2.5, 0.0);
        let expected_body_v = mosfet_body_effect_drain_voltage(2.5, bulk_v);
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "zero-body-bias NMOS should settle at {expected_reference_v} V, got {reference_v} V"
        );
        assert!(
            (body_v - expected_body_v).abs() < 5e-5,
            "NMOS at VBS={bulk_v} V should settle at {expected_body_v} V, got {body_v} V"
        );
        if bulk_v == 0.0 {
            assert!(
                (body_v - reference_v).abs() < 1e-9,
                "matched devices agree at zero body bias"
            );
        }

        for (channel, volts) in [(5, ntc_v), (6, diode_v), (7, reference_v), (8, body_v)] {
            assert_eq!(
                adc1_channel_count(&mut bus, channel),
                expected_adc12_count(volts) as u16,
                "PA route must seed ADC1_IN{channel} with the solved voltage"
            );
            assert_eq!(
                u5_adc_convert(&mut bus, channel),
                expected_adc12_count(volts),
                "firmware-visible ADC1_IN{channel} conversion follows the solved voltage"
            );
        }
        assert_eq!(
            adc1_channel_count(&mut bus, 9),
            u16::MAX,
            "neighboring ADC1 channel 9 remains undriven"
        );
        body_samples.push((bulk_v, body_v));
    }

    assert!(
        body_samples.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "reverse body bias raises Vth and the loaded drain voltage: {body_samples:?}"
    );
}

#[test]
fn nucleo_u5a5zj_diode_depletion_and_transit_charge_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-u5a5zj.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-U5A5ZJ-Q profile");
    assert_eq!(profile.name, "nucleo-u5a5zj");

    let generic =
        SystemManifest::from_file(root("examples/nucleo-u5a5zj/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and Shockley diode netlists");
    let charge = SystemManifest::from_file(root(
        "examples/nucleo-u5a5zj/analog-diode-charge-adc-system.yaml",
    ))
    .expect("load four matched generic diode charge variants");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(charge.cosim_models);

    let chip = ChipDescriptor::from_file(system_path.parent().unwrap().join(&profile.chip))
        .expect("load STM32U5A5 descriptor");
    for (pad, channel) in [
        ("PA0", 5),
        ("PA1", 6),
        ("PA2", 7),
        ("PA3", 8),
        ("PA4", 9),
        ("PA5", 10),
    ] {
        assert_eq!(
            chip.analog_pins[pad].channel, channel,
            "{pad} must resolve to ADC1_IN{channel}"
        );
    }

    let mut bus =
        SystemBus::from_config(&chip, &profile).expect("build NUCLEO-U5A5ZJ-Q system bus");
    let ahb2enr1 = bus.read_u32(RCC_AHB2ENR1).expect("read RCC AHB2ENR1");
    bus.write_u32(RCC_AHB2ENR1, ahb2enr1 | (1 << 10))
        .expect("enable ADC1 clock");
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
    bus.write_u32(ADC_CFGR1, 0x8000_0000 | (1 << 2))
        .expect("select 12-bit conversion resolution");
    bus.write_u32(
        ADC_PCSEL,
        (5..=10).fold(0, |mask, channel| mask | (1 << channel)),
    )
    .expect("select all six routed ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an NTC, an unrelated diode, and four charge variants");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all six netlists must route through PA0-PA5 to ADC1_IN5-10"
    );
    assert_eq!(
        adc1_channel_count(&mut bus, 11),
        u16::MAX,
        "adjacent ADC1 channel 11 remains undriven"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.diode_charge.supply_v", 0.0)
        .expect("initialize the four diode branches at zero volts");

    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle the NTC and four diode branches before the supply step");
    assert_eq!(initial_steps.len(), 6, "all six analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );

    session
        .set_signal_number("ui.diode_charge.supply_v", 3.3)
        .expect("step the supply on all four diode branches");
    let mut samples = Vec::new();
    for step in 0..800 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the NTC, diode, and four charge branches");
        assert_eq!(steps.len(), 6, "all six analog branches solve per step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        let reference_v = signal_voltage(&session, "board.analog.pa2_volts");
        let depletion_v = signal_voltage(&session, "board.analog.pa3_volts");
        let transit_v = signal_voltage(&session, "board.analog.pa4_volts");
        let combined_v = signal_voltage(&session, "board.analog.pa5_volts");
        let dc_v = diode_load_line_voltage();
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the unrelated 25 °C NTC stays isolated on PA0/ADC1_IN5, got {ntc_v} V"
        );
        assert!(
            (diode_v - dc_v).abs() < 2e-5,
            "the unrelated Shockley diode stays at its DC load line on PA1/ADC1_IN6, got {diode_v} V"
        );
        assert!(
            (reference_v - dc_v).abs() < 2e-5,
            "the uncharged control stays at the DC load line {dc_v} V, got {reference_v} V"
        );

        for (channel, volts) in [
            (5, ntc_v),
            (6, diode_v),
            (7, reference_v),
            (8, depletion_v),
            (9, transit_v),
            (10, combined_v),
        ] {
            let expected = expected_adc12_count(volts);
            assert_eq!(
                adc1_channel_count(&mut bus, channel),
                expected as u16,
                "the solved voltage must seed ADC1_IN{channel}"
            );
            assert_eq!(
                u5_adc_convert(&mut bus, channel),
                expected,
                "firmware-visible ADC1_IN{channel} conversion must report {volts} V"
            );
        }
        assert_eq!(
            adc1_channel_count(&mut bus, 11),
            u16::MAX,
            "adjacent ADC1 channel 11 remains undriven"
        );

        samples.push((reference_v, depletion_v, transit_v, combined_v));
        if step == 0 {
            assert!(
                (depletion_v - reference_v).abs() > 0.001,
                "CJO depletion charge must change the first routed transient: {samples:?}"
            );
            assert!(
                (transit_v - reference_v).abs() > 0.01,
                "TT diffusion charge must change the first routed transient: {samples:?}"
            );
            assert!(
                (combined_v - reference_v).abs() > 0.01,
                "combined junction and transit charge must change the first routed transient: {samples:?}"
            );
        }
    }

    let settled = samples.last().expect("transient samples exist");
    for (label, voltage) in [
        ("depletion", settled.1),
        ("transit", settled.2),
        ("combined", settled.3),
    ] {
        assert!(
            (voltage - settled.0).abs() < 2e-4,
            "{label} charge branch must settle to the same DC diode load line, got {voltage} V vs {} V",
            settled.0
        );
    }
}
