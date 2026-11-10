// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic Gummel–Poon charge devices route through NUCLEO-F401RE into ADC1.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_BASE: u64 = 0x4002_3800;
const ADC1_BASE: u64 = 0x4001_2000;

/// Independent Gummel–Poon load-line reference with forward Early effect.
fn bjt_collector_voltage(base_drive_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-15;
    const BF: f64 = 100.0;
    const BR: f64 = 4.0;
    const VAF: f64 = 50.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let transport = IS_A * (forward - reverse) * (1.0 - vbc / VAF);
        let base_emitter = (IS_A / BF) * (forward - 1.0);
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        (transport - base_collector, base_emitter + base_collector)
    };

    let collector_for_vbe = |vbe: f64| {
        let (mut low, mut high) = (0.0, SUPPLY_V);
        for _ in 0..100 {
            let collector_v = (low + high) / 2.0;
            let (collector_current, _) = currents(vbe, collector_v);
            let residual = (SUPPLY_V - collector_v) / COLLECTOR_OHMS - collector_current;
            if residual > 0.0 {
                low = collector_v;
            } else {
                high = collector_v;
            }
        }
        (low + high) / 2.0
    };

    let (mut low, mut high) = (0.0, base_drive_v);
    for _ in 0..100 {
        let vbe = (low + high) / 2.0;
        let collector_v = collector_for_vbe(vbe);
        let (_, base_current) = currents(vbe, collector_v);
        let residual = (base_drive_v - vbe) / BASE_OHMS - base_current;
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe((low + high) / 2.0)
}

/// Independent Shockley diode load line with series resistance.
fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LIMIT_OHMS: f64 = 1_000.0;
    const IS_A: f64 = 2.52e-9;
    const N: f64 = 1.752;
    const RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
    let current = |junction_v: f64| IS_A * ((junction_v / (N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let amps = current(junction_v);
        (SUPPLY_V - junction_v - amps * RS_OHMS) / LIMIT_OHMS - amps
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
    junction_v + current(junction_v) * RS_OHMS
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
        .expect("STM32F401 declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<labwired_core::peripherals::adc::Adc>()
        .expect("ADC1 uses the shared STM32 ADC model")
        .channel_input_count(channel)
}

fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 4095 / 3300).min(4095) as u16
}

/// Exercise the F401's firmware-visible ADC1 regular one-shot conversion.
fn f401_adc_convert(bus: &mut SystemBus, channel: u8) -> u32 {
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

#[test]
fn nucleo_f401re_bjt_junction_and_transit_charge_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f401re.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F401RE profile");
    let diode = SystemManifest::from_file(root("examples/nucleo-f401re/analog-diode-system.yaml"))
        .expect("load unrelated generic diode netlist");
    let charge = SystemManifest::from_file(root("examples/nucleo-f401re/analog-bjt-charge.yaml"))
        .expect("load matched generic BJT charge netlists");
    profile.cosim_models = diode.cosim_models;
    profile.cosim_models.extend(charge.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F401 descriptor");
    for (pad, channel) in [
        ("PA0", 0),
        ("PA1", 1),
        ("PA2", 2),
        ("PA3", 3),
        ("PA4", 4),
        ("PA5", 5),
    ] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F401RE bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries one diode and five matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to ADC1 channels zero through five"
    );
    for channel in 0..=6 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} begins without an analog source"
        );
    }

    session
        .set_signal_number("ui.bjt_charge.base_v", 0.0)
        .expect("initialize all BJT base-drive sources");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize the diode and matched BJT branches");
    assert_eq!(initial_steps.len(), 6, "all six analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );

    session
        .set_signal_number("ui.bjt_charge.base_v", 0.8)
        .expect("step the matched BJT base-drive sources");
    let mut first_bjt = None;
    let mut settled_bjt = None;
    for step in 0..8_000 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route diode and BJT charge branches");
        assert_eq!(steps.len(), 6, "all six branches solve at each 1 us step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let diode_v = signal_voltage(&session, "board.analog.pa0_volts");
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated Shockley diode remains on its independent load line"
        );
        let bjt_voltages = [
            signal_voltage(&session, "board.analog.pa1_volts"),
            signal_voltage(&session, "board.analog.pa2_volts"),
            signal_voltage(&session, "board.analog.pa3_volts"),
            signal_voltage(&session, "board.analog.pa4_volts"),
            signal_voltage(&session, "board.analog.pa5_volts"),
        ];

        if step == 0 {
            assert!(
                (bjt_voltages[0] - bjt_collector_voltage(0.8)).abs() < 5e-5,
                "uncharged BJT must match independent Gummel–Poon DC load line"
            );
            for (label, voltage) in [
                ("CJE depletion", bjt_voltages[1]),
                ("CJC depletion", bjt_voltages[2]),
                ("TF transit", bjt_voltages[3]),
                ("combined junction and transit", bjt_voltages[4]),
            ] {
                assert!(
                    (voltage - bjt_voltages[0]).abs() > 0.01,
                    "{label} must change the first transient: reference={} V, charged={voltage} V",
                    bjt_voltages[0]
                );
            }
            first_bjt = Some(bjt_voltages);
        }
        if step == 7_999 {
            settled_bjt = Some(bjt_voltages);
        }
    }

    let first = first_bjt.expect("first transient sample exists");
    let settled = settled_bjt.expect("settled transient sample exists");
    let expected_dc = bjt_collector_voltage(0.8);
    assert!((settled[0] - expected_dc).abs() < 5e-5);
    for (label, voltage) in [
        ("CJE depletion", settled[1]),
        ("CJC depletion", settled[2]),
        ("TF transit", settled[3]),
        ("combined junction and transit", settled[4]),
    ] {
        assert!(
            (voltage - expected_dc).abs() < 2e-4,
            "{label} must settle to the independent Gummel–Poon DC load line {expected_dc} V, got {voltage} V"
        );
    }
    assert!(
        first[1..]
            .iter()
            .zip(settled[1..].iter())
            .all(|(early, late)| (early - late).abs() > 0.01),
        "junction and transit charge contributions decay between the first and settled samples"
    );

    let diode_v = signal_voltage(&session, "board.analog.pa0_volts");
    for (channel, voltage) in std::iter::once((0, diode_v)).chain(
        settled
            .iter()
            .enumerate()
            .map(|(idx, voltage)| (idx as u8 + 1, *voltage)),
    ) {
        let expected = expected_adc_count(voltage);
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            expected,
            "the solved voltage must seed descriptor-routed ADC1 channel {channel}"
        );
        assert_eq!(
            f401_adc_convert(&mut bus, channel),
            u32::from(expected),
            "firmware-visible ADC1 channel {channel} must report its analog device"
        );
    }
    assert_eq!(
        adc_channel_count(&mut bus, 6),
        0xFFFF,
        "the neighboring ADC1 channel remains undriven"
    );
}
