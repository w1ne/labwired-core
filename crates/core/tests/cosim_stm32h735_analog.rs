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

/// Independent Gummel–Poon collector load line, including forward Early
/// effect, high-injection rolloff, and base-emitter recombination. This scalar
/// reference does not read voltages or currents from the MNA solver.
fn bjt_collector_voltage(base_drive_v: f64, high_injection: bool, base_recombination: bool) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const VAF: f64 = 50.0;
    const VAR: f64 = 75.0;
    const IKF_A: f64 = 50e-6;
    const IKR_A: f64 = 500e-6;
    const NKF: f64 = 0.8;
    const ISE_A: f64 = 2e-12;
    const NE: f64 = 1.5;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let base_width = 1.0 - vbc / VAF - vbe / VAR;
        let q2 = if high_injection {
            IS_A * (forward - 1.0) / IKF_A + IS_A * (reverse - 1.0) / IKR_A
        } else {
            0.0
        };
        let qb = (1.0 + (1.0 + 4.0 * q2).max(0.0).powf(NKF)) / (2.0 * base_width);
        let transport = IS_A * (forward - reverse) / qb;
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        let base_emitter = (IS_A / BF) * (forward - 1.0)
            + if base_recombination {
                ISE_A * ((vbe / (NE * VT)).exp() - 1.0)
            } else {
                0.0
            };
        (transport - base_collector, base_emitter + base_collector)
    };

    let collector_for_vbe = |vbe: f64| {
        let (mut low, mut high) = (0.0, SUPPLY_V);
        for _ in 0..100 {
            let collector_v = 0.5 * (low + high);
            let (collector_current, _) = currents(vbe, collector_v);
            let residual = (SUPPLY_V - collector_v) / COLLECTOR_OHMS - collector_current;
            if residual > 0.0 {
                low = collector_v;
            } else {
                high = collector_v;
            }
        }
        0.5 * (low + high)
    };

    let (mut low, mut high) = (0.0, base_drive_v);
    for _ in 0..100 {
        let vbe = 0.5 * (low + high);
        let collector_v = collector_for_vbe(vbe);
        let (_, base_current) = currents(vbe, collector_v);
        if (base_drive_v - vbe) / BASE_OHMS > base_current {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe(0.5 * (low + high))
}

/// Independent level-1 JFET gate depletion charge for one junction. The
/// fixture uses reverse bias below `FC * PB`, where the square-root law applies.
fn jfet_gate_junction_charge(voltage_v: f64) -> f64 {
    const C0_F: f64 = 1e-9;
    const PB_V: f64 = 1.0;
    const M: f64 = 0.5;
    C0_F * PB_V / (1.0 - M) * (1.0 - (1.0 - voltage_v / PB_V).powf(1.0 - M))
}

/// Solve one backward-Euler step for the two JFET gate junctions, each tied
/// from the gate to a fixed 3.3 V drain/source node through its depletion charge.
fn expected_jfet_gate_voltage(previous_v: f64, drive_v: f64, step_s: f64) -> f64 {
    const DRIVE_OHMS: f64 = 1_000.0;
    const JUNCTION_REFERENCE_V: f64 = 3.3;
    let total_charge = |gate_v: f64| 2.0 * jfet_gate_junction_charge(gate_v - JUNCTION_REFERENCE_V);
    let previous_charge = total_charge(previous_v);
    let residual = |gate_v: f64| {
        (gate_v - drive_v) / DRIVE_OHMS + (total_charge(gate_v) - previous_charge) / step_s
    };
    let (mut low, mut high) = (drive_v.min(previous_v), drive_v.max(previous_v));
    for _ in 0..100 {
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            high = mid;
        } else {
            low = mid;
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

#[test]
fn stm32h735_bjt_high_injection_and_recombination_reach_adc1_with_an_unrelated_diode() {
    let system_path = root("configs/systems/stm32h735-smoke.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load STM32H735 profile");
    let analog = SystemManifest::from_file(root(
        "examples/stm32h735-smoke/analog-bjt-high-injection.yaml",
    ))
    .expect("load generic BJT and diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32H735 descriptor");
    for (pad, channel) in [("PA0", 16), ("PA1", 17)] {
        assert_eq!(
            chip.analog_pins
                .get(pad)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pad} must resolve to STM32H735 ADC1_INP{channel}"
        );
    }

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
        .expect("select BJT and diode ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries BJT and diode netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve PA0/PA1 to ADC1 channels 16/17"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut high_drive_actual_v = None;
    let mut high_drive_no_rolloff_v = None;
    let mut high_drive_no_recombination_v = None;
    for base_drive_v in [0.8, 1.5, 3.3] {
        session
            .set_signal_number("ui.bjt.base_v", base_drive_v)
            .expect("set independent BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the BJT and diode netlists");
        assert_eq!(steps.len(), 2, "BJT and diode solve at every step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let collector_v = signal_voltage(&session, "board.analog.pa0_volts");
        let expected_collector_v = bjt_collector_voltage(base_drive_v, true, true);
        assert!(
            (collector_v - expected_collector_v).abs() < 5e-5,
            "BJT load line at base drive {base_drive_v} V should be {expected_collector_v} V with IKF/NKF rolloff and ISE recombination, got {collector_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode remains at its DC load line"
        );

        for (channel, voltage_v) in [(16, collector_v), (17, diode_v)] {
            let expected = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected as u16,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                h7_adc_convert(&mut bus, channel),
                expected,
                "ADC1 channel {channel} must report the routed {voltage_v} V"
            );
        }
        assert_eq!(adc_channel_count(&mut bus, 18), u16::MAX);

        if base_drive_v == 3.3 {
            high_drive_actual_v = Some(collector_v);
            high_drive_no_rolloff_v = Some(bjt_collector_voltage(base_drive_v, false, true));
            high_drive_no_recombination_v = Some(bjt_collector_voltage(base_drive_v, true, false));
        }
    }

    let actual_v = high_drive_actual_v.expect("the high-drive sample was recorded");
    let no_rolloff_v = high_drive_no_rolloff_v.expect("the no-rolloff reference was recorded");
    let no_recombination_v =
        high_drive_no_recombination_v.expect("the no-recombination reference was recorded");
    assert!(
        actual_v - no_rolloff_v > 0.05,
        "IKF/NKF high-injection rolloff must measurably raise collector voltage: actual={actual_v} V, no rolloff={no_rolloff_v} V"
    );
    assert!(
        actual_v - no_recombination_v > 0.05,
        "base-emitter recombination must measurably load the base drive"
    );
}

#[test]
fn stm32h735_mosfet_drain_bulk_charge_reaches_adc1_with_an_unrelated_diode() {
    let system_path = root("configs/systems/stm32h735-smoke.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load STM32H735 profile");
    let analog = SystemManifest::from_file(root(
        "examples/stm32h735-smoke/analog-mosfet-junction-charge.yaml",
    ))
    .expect("load generic MOSFET junction-charge and diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32H735 descriptor");
    for (pad, channel) in [("PA0", 16), ("PA1", 17)] {
        assert_eq!(
            chip.analog_pins
                .get(pad)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pad} must resolve to STM32H735 ADC1_INP{channel}"
        );
    }
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
        .expect("select MOSFET and diode ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries MOSFET and diode netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve PA0/PA1 to ADC1 channels 16/17"
    );
    let expected_diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.mosfet_bulk.drain_current_a", 0.0)
        .expect("initialize the MOSFET drain current");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle the zero-current MOSFET and diode branches");
    assert_eq!(initial_steps.len(), 2, "both analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );
    assert!(
        signal_voltage(&session, "board.analog.pa0_volts").abs() < 1e-12,
        "the zero-current drain starts at ground"
    );

    session
        .set_signal_number("ui.mosfet_bulk.drain_current_a", 1e-3)
        .expect("step 1 mA through the MOSFET drain-bulk junction");
    let mut expected_drain_v = 0.0;
    for step in 0..12 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the MOSFET junction-charge transient");
        assert_eq!(steps.len(), 2, "MOSFET and diode solve at every step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        // CBD=1 nF, Rload=1 kΩ, and dt=1 us give a 1/2 backward-Euler
        // recurrence toward the 1 V DC load line after the 1 mA current step.
        expected_drain_v = 0.5 * expected_drain_v + 0.5;
        let drain_v = signal_voltage(&session, "board.analog.pa0_volts");
        assert!(
            (drain_v - expected_drain_v).abs() < 2e-8,
            "CBD drain transient at step {step}: expected {expected_drain_v} V, got {drain_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode remains at its DC load line"
        );

        for (channel, voltage_v) in [(16, drain_v), (17, diode_v)] {
            let expected = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected as u16,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                h7_adc_convert(&mut bus, channel),
                expected,
                "ADC1 channel {channel} must report the routed {voltage_v} V"
            );
        }
        assert_eq!(adc_channel_count(&mut bus, 18), u16::MAX);
    }
    assert!(
        (signal_voltage(&session, "board.analog.pa0_volts") - 1.0).abs() < 3e-4,
        "the junction capacitance changes the transient but not the 1 V DC load line"
    );
}

#[test]
fn stm32h735_jfet_gate_depletion_charge_reaches_pa0_adc1_with_an_unrelated_diode() {
    const STEP_S: f64 = 1e-6;
    const SUBSTEPS: usize = 10;
    let system_path = root("configs/systems/stm32h735-smoke.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load STM32H735 profile");
    let analog = SystemManifest::from_file(root(
        "examples/stm32h735-smoke/analog-jfet-gate-charge.yaml",
    ))
    .expect("load generic JFET gate-charge and diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32H735 descriptor");
    for (pad, channel) in [("PA0", 16), ("PA1", 17)] {
        assert_eq!(
            chip.analog_pins
                .get(pad)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pad} must resolve to STM32H735 ADC1_INP{channel}"
        );
    }
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
        .expect("select JFET gate and diode ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries JFET and diode netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve PA0/PA1 to ADC1 channels 16/17"
    );

    let expected_diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.jfet_gate.drive_v", 3.3)
        .expect("precharge the JFET gate at the drain/source reference");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle the JFET gate and diode branches");
    assert_eq!(initial_steps.len(), 2, "both analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );
    assert!(
        (signal_voltage(&session, "board.analog.pa0_volts") - 3.3).abs() < 1e-9,
        "the precharged gate should begin at the 3.3 V drain/source reference"
    );

    session
        .set_signal_number("ui.jfet_gate.drive_v", 0.0)
        .expect("step the JFET gate drive low");
    let mut expected_gate_v = 3.3;
    for step in 0..8 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the JFET depletion-charge transient");
        assert_eq!(steps.len(), 2, "JFET and diode solve at every step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        for _ in 0..SUBSTEPS {
            expected_gate_v =
                expected_jfet_gate_voltage(expected_gate_v, 0.0, STEP_S / SUBSTEPS as f64);
        }
        let gate_v = signal_voltage(&session, "board.analog.pa0_volts");
        assert!(
            (gate_v - expected_gate_v).abs() < 2e-8,
            "JFET CGS/CGD depletion charge at step {step}: expected {expected_gate_v} V, got {gate_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode remains at its DC load line: expected {expected_diode_v} V, got {diode_v} V"
        );

        for (channel, voltage_v) in [(16, gate_v), (17, diode_v)] {
            let expected = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected as u16,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                h7_adc_convert(&mut bus, channel),
                expected,
                "ADC1 channel {channel} must report the routed {voltage_v} V"
            );
        }
        assert_eq!(adc_channel_count(&mut bus, 18), u16::MAX);
    }
    assert!(
        signal_voltage(&session, "board.analog.pa0_volts") < 1.0,
        "the gate depletion capacitance delays the edge but eventually follows the low drive"
    );
}
