// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the NUCLEO-F407 STM32F407 profile into ADC1.

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

/// Independent level-1 NMOS body-effect load-line reference for the
/// board-routed matched-pair test. `bulk_v` is VBS because the source is
/// grounded; reverse body bias raises the threshold through GAMMA/PHI.
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
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

/// Independent SPICE level-1 JFET load-line reference, including the
/// Parker–Skellern B fit and channel-length modulation.
fn jfet_drain_voltage(gate_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const VTO: f64 = -2.0;
    const BETA_A_PER_V2: f64 = 500e-6;
    const LAMBDA_PER_V: f64 = 0.025;
    const B: f64 = 0.8;
    const PB: f64 = 1.0;

    let overdrive = gate_v - VTO;
    if overdrive <= 0.0 {
        return SUPPLY_V;
    }
    let bfac = (1.0 - B) / (PB - VTO);
    let drain_current = |vds: f64| {
        let beta_p = BETA_A_PER_V2 * (1.0 + LAMBDA_PER_V * vds);
        if overdrive >= vds {
            beta_p
                * vds
                * (vds * (bfac * vds - B) + overdrive * (2.0 * B + 3.0 * bfac * (overdrive - vds)))
        } else {
            beta_p * overdrive * overdrive * (B + overdrive * bfac)
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

/// Independent Gummel–Poon NPN load-line reference with forward Early effect.
/// The two scalar roots solve base-drive current and collector-load current;
/// neither reads a voltage from the analog solver.
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

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("STM32F407 declares ADC1");
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

/// Independent SPICE level-1 JFET gate-junction depletion charge below FC·PB.
fn jfet_gate_junction_charge(voltage_v: f64) -> f64 {
    const C0_F: f64 = 1e-9;
    const PB_V: f64 = 1.0;
    const M: f64 = 0.5;
    C0_F * PB_V / (1.0 - M) * (1.0 - (1.0 - voltage_v / PB_V).powf(1.0 - M))
}

/// Independent backward-Euler update for two gate junctions tied to 3.3 V
/// drain/source nodes through their depletion charge.
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

#[test]
fn nucleo_f407_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f407.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F407 profile");
    assert_eq!(profile.name, "nucleo-f407");
    assert!(profile.external_devices.is_empty());
    assert!(
        profile
            .board_io
            .iter()
            .any(|io| io.id == "led_ld2" && io.pin == 5),
        "the mixed-signal run keeps the NUCLEO-F407's PA5 user LED"
    );
    assert!(
        profile
            .board_io
            .iter()
            .any(|io| io.id == "button_b1" && io.peripheral == "gpioc" && io.pin == 13),
        "the mixed-signal run keeps the NUCLEO-F407's PC13 user button"
    );

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f407/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F407 descriptor");
    assert_eq!(chip.analog_pins["PA0"].channel, 0);
    assert_eq!(chip.analog_pins["PA1"].channel, 1);
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F407 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through PA0/PA1 to ADC1 channels 0/1"
    );
    for channel in [0, 1, 2] {
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

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley diode load line should be {expected_diode_v} V, got {diode_v} V"
        );

        for (channel, volts) in [(0, ntc_v), (1, diode_v)] {
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
        "ADC counts track the solved temperature-dependent voltage: {ntc_samples:?}"
    );
    assert_eq!(
        ntc_samples[1].1, 2047,
        "the 25 °C NTC divider is half scale"
    );
}

#[test]
fn nucleo_f407_diode_depletion_and_transit_charge_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f407.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F407 profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-f407/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let charge = SystemManifest::from_file(root("examples/nucleo-f407/analog-diode-charge.yaml"))
        .expect("load diode charge comparison netlists");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(charge.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F407 descriptor");
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

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F407 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an NTC and four diode charge variants");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic analog netlists must resolve to ADC1 channels 0 through 5"
    );
    for channel in 0..=6 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.diode_charge.supply_v", 0.0)
        .expect("initialize the diode drive at zero volts");

    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle the NTC and diode charge branches before the step");
    assert_eq!(initial_steps.len(), 6, "all six analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );

    session
        .set_signal_number("ui.diode_charge.supply_v", 3.3)
        .expect("apply a supply step to the four diode branches");
    let mut samples = Vec::new();
    for step in 0..800 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the NTC and all four diode branches");
        assert_eq!(steps.len(), 6, "all six analog branches solve at each step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let reference_v = signal_voltage(&session, "board.analog.pa2_volts");
        let depletion_v = signal_voltage(&session, "board.analog.pa3_volts");
        let transit_v = signal_voltage(&session, "board.analog.pa4_volts");
        let combined_v = signal_voltage(&session, "board.analog.pa5_volts");
        assert!(
            (reference_v - diode_load_line_voltage()).abs() < 2e-5,
            "uncharged reference should sit at the DC load line {} V, got {reference_v} V at step {step}",
            diode_load_line_voltage()
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa0_volts") - 1.65).abs() < 1e-9,
            "the unrelated 25 °C NTC stays isolated on PA0/ADC1_IN0"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa1_volts") - diode_load_line_voltage()).abs()
                < 2e-5,
            "the unrelated DC diode stays isolated on PA1/ADC1_IN1"
        );

        for (channel, volts) in [
            (0, 1.65),
            (1, diode_load_line_voltage()),
            (2, reference_v),
            (3, depletion_v),
            (4, transit_v),
            (5, combined_v),
        ] {
            let expected = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 conversion must report the voltage on channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 6),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
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

#[test]
fn nucleo_f407_mosfet_overlap_charge_reaches_pa3_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f407.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F407 profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-f407/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let mosfet = SystemManifest::from_file(root("examples/nucleo-f407/analog-mosfet-charge.yaml"))
        .expect("load MOSFET gate-charge comparison netlists");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F407 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F407 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an NTC, diode, and MOSFET charge pair");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic NTC, diode, and MOSFET branches must resolve to ADC1 channels 0 through 3"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.mosfet_charge.gate_v", 0.0)
        .expect("initialize the MOSFET gate at zero volts");

    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle the NTC, diode, and MOSFET gate branches before the step");
    assert_eq!(initial_steps.len(), 4, "all four analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );
    assert!((signal_voltage(&session, "board.analog.pa2_volts")).abs() < 1e-12);
    assert!((signal_voltage(&session, "board.analog.pa3_volts")).abs() < 1e-12);

    session
        .set_signal_number("ui.mosfet_charge.gate_v", 3.3)
        .expect("step the generic NMOS gate");

    // CGSO=10 uF/m times W=100 um gives 1 nF. With the 1 kOhm source
    // resistor and a 1 us backward-Euler step, the source node receives half
    // of the 3.3 V gate step, then discharges by half on each held-input step.
    let mut expected_source_v = 1.65;
    for step in 0..12 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the MOSFET overlap-charge transient");
        assert_eq!(
            steps.len(),
            4,
            "all four analog branches solve at each step"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        let reference_v = signal_voltage(&session, "board.analog.pa2_volts");
        let charged_v = signal_voltage(&session, "board.analog.pa3_volts");
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the unrelated NTC remains isolated"
        );
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated Shockley diode remains at its DC load line"
        );
        assert!(
            reference_v.abs() < 1e-12,
            "the uncharged MOSFET is the control branch"
        );
        assert!(
            (charged_v - expected_source_v).abs() < 2e-8,
            "CGSO RC recurrence at step {step}: expected {expected_source_v} V, got {charged_v} V"
        );

        for (channel, volts) in [(0, ntc_v), (1, diode_v), (2, reference_v), (3, charged_v)] {
            let expected = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 conversion must report channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        if step == 0 {
            assert_eq!(
                adc_channel_count(&mut bus, 3),
                2047,
                "the first overlap-charge transient reaches the PA3 ADC as half scale"
            );
        }
        expected_source_v *= 0.5;
    }

    assert!(
        signal_voltage(&session, "board.analog.pa3_volts") < 1e-3,
        "the overlap capacitance changes the transient but not the settled DC operating point"
    );
}

#[test]
fn nucleo_f407_mosfet_load_line_reaches_pa2_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-f407.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F407 profile");
    assert_eq!(profile.name, "nucleo-f407");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f407/analog-ntc-diode-system.yaml"))
            .expect("load the independent generic NTC and diode netlists");
    let mosfet = SystemManifest::from_file(root("examples/nucleo-f407/analog-mosfet-switch.yaml"))
        .expect("load the generic low-side MOSFET netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F407 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F407 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries three unrelated analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "NTC, diode, and NMOS netlists must resolve to distinct ADC1 inputs"
    );
    for channel in 0..=3 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the independent NTC temperature");
    let mut cycles = 0;
    let mut drain_samples = Vec::new();
    for gate_v in [0.0_f64, 0.8, 1.0, 2.0, 3.3] {
        session
            .set_signal_number("ui.mosfet.gate_v", gate_v)
            .expect("set generic MOSFET gate voltage");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and MOSFET netlists");
        assert_eq!(
            steps.len(),
            3,
            "all three devices solve at the 1 us boundary"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let drain_v = signal_voltage(&session, "board.analog.pa2_volts");
        let expected_drain_v = mosfet_drain_voltage(gate_v);
        assert!(
            (drain_v - expected_drain_v).abs() < 5e-5,
            "level-1 NMOS load line at Vgs={gate_v} V should be {expected_drain_v} V, got {drain_v} V"
        );
        for (channel, volts) in [(2, drain_v), (0, expected_ntc_v), (1, expected_diode_v)] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1 channel {channel} conversion follows its analog device"
            );
        }
        assert!(
            (signal_voltage(&session, "board.analog.pa0_volts") - expected_ntc_v).abs() < 1e-9,
            "the independent Beta NTC divider remains on PA0/ADC1 channel 0"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa1_volts") - expected_diode_v).abs() < 2e-5,
            "the independent Shockley diode remains on PA1/ADC1 channel 1"
        );
        assert_eq!(
            adc_channel_count(&mut bus, 3),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        drain_samples.push((gate_v, drain_v));
    }

    assert_eq!(
        drain_samples[0].1, drain_samples[2].1,
        "gate voltage at or below threshold leaves the drain at the supply"
    );
    assert!(
        drain_samples[4].1 < drain_samples[3].1 && drain_samples[3].1 < drain_samples[2].1,
        "drain voltage falls as the NMOS conducts more strongly: {drain_samples:?}"
    );
}

#[test]
fn nucleo_f407_mosfet_body_effect_reaches_pa2_pa3_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f407.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F407 profile");
    assert_eq!(profile.name, "nucleo-f407");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f407/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let body_effect =
        SystemManifest::from_file(root("examples/nucleo-f407/analog-mosfet-body-effect.yaml"))
            .expect("load generic matched NMOS body-bias netlists");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(body_effect.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F407 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F407 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched NMOS branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the NTC, diode, and matched NMOS drains must resolve to ADC1 channels 0 through 3"
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
        .expect("set the unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.mosfet_body.bulk_v", 0.0)
        .expect("initialize the matched NMOS bulk bias");

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
        assert_eq!(
            steps.len(),
            3,
            "all three analog models solve at the 1 us boundary"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let reference_v = signal_voltage(&session, "board.analog.pa2_volts");
        let body_v = signal_voltage(&session, "board.analog.pa3_volts");
        let expected_reference_v = mosfet_body_effect_drain_voltage(2.5, 0.0);
        let expected_body_v = mosfet_body_effect_drain_voltage(2.5, bulk_v);
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "zero-body-bias level-1 NMOS should settle at {expected_reference_v} V, got {reference_v} V"
        );
        assert!(
            (body_v - expected_body_v).abs() < 5e-5,
            "level-1 NMOS at VBS={bulk_v} V should settle at {expected_body_v} V, got {body_v} V"
        );
        if bulk_v == 0.0 {
            assert!(
                (body_v - reference_v).abs() < 1e-9,
                "matched devices agree with zero body bias"
            );
        }

        for (channel, volts) in [
            (0, expected_ntc_v),
            (1, expected_diode_v),
            (2, reference_v),
            (3, body_v),
        ] {
            let count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(count),
                "firmware-visible ADC1 channel {channel} conversion must report its analog voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa0_volts") - expected_ntc_v).abs() < 1e-9,
            "the unrelated NTC remains isolated on PA0/ADC1_IN0"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa1_volts") - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode remains isolated on PA1/ADC1_IN1"
        );
        body_samples.push((bulk_v, body_v));
    }

    assert!(
        body_samples.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "reverse body bias raises Vth and the loaded drain voltage: {body_samples:?}"
    );
}

#[test]
fn nucleo_f407_jfet_load_line_reaches_pa3_adc1_alongside_unrelated_devices() {
    let system_path = root("configs/systems/nucleo-f407.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F407 profile");
    assert_eq!(profile.name, "nucleo-f407");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f407/analog-ntc-diode-system.yaml"))
            .expect("load the independent generic NTC and diode netlists");
    let mosfet = SystemManifest::from_file(root("examples/nucleo-f407/analog-mosfet-switch.yaml"))
        .expect("load the generic low-side MOSFET netlist");
    let jfet = SystemManifest::from_file(root("examples/nucleo-f407/analog-jfet-load.yaml"))
        .expect("load the generic level-1 JFET netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);
    profile.cosim_models.extend(jfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F407 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F407 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries four independent analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "NTC, diode, NMOS, and JFET routes must resolve to distinct ADC1 inputs"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the independent NTC temperature");
    session
        .set_signal_number("ui.mosfet.gate_v", 2.0)
        .expect("set the independent MOSFET gate voltage");

    let mut cycles = 0;
    let mut drain_samples = Vec::new();
    for gate_v in [0.0_f64, -0.5, -1.0, -1.5, -2.2] {
        session
            .set_signal_number("ui.jfet.gate_v", gate_v)
            .expect("set generic JFET gate voltage");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, MOSFET, and JFET netlists");
        assert_eq!(
            steps.len(),
            4,
            "all four devices solve at the 1 us boundary"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let drain_v = signal_voltage(&session, "board.analog.pa3_volts");
        let expected_drain_v = jfet_drain_voltage(gate_v);
        assert!(
            (drain_v - expected_drain_v).abs() < 5e-5,
            "level-1 NJF load line at Vgs={gate_v} V should be {expected_drain_v} V, got {drain_v} V"
        );
        let jfet_count = expected_adc_count(drain_v);
        assert_eq!(
            adc_channel_count(&mut bus, 3),
            jfet_count,
            "the solved JFET drain voltage must seed ADC1 channel 3"
        );
        assert_eq!(
            stm32f4_adc_convert(&mut bus, 3),
            u32::from(jfet_count),
            "firmware-visible ADC1 channel 3 conversion must report the JFET load line"
        );

        for (channel, volts) in [
            (0, expected_ntc_v),
            (1, expected_diode_v),
            (2, mosfet_drain_voltage(2.0)),
        ] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the unrelated analog source must remain isolated on ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1 channel {channel} conversion remains stable"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        drain_samples.push((gate_v, drain_v));
    }

    assert!(
        drain_samples[0].1 < 2.0,
        "zero-bias JFET load line exercises the linear channel region: {drain_samples:?}"
    );
    assert!(
        drain_samples[1].1 > 1.5,
        "the reverse-biased gate moves the JFET into saturation: {drain_samples:?}"
    );
    assert!(
        (drain_samples[4].1 - 3.3).abs() < 1e-6,
        "gate voltage beyond pinch-off leaves the JFET drain at the supply"
    );
    assert!(
        drain_samples.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "the drain voltage rises monotonically toward cutoff as Vgs becomes more negative: {drain_samples:?}"
    );
}

#[test]
fn nucleo_f407_jfet_gate_depletion_charge_reaches_adc1_with_ntc_and_diode() {
    const STEP_S: f64 = 1e-6;
    const SUBSTEPS: usize = 10;

    let system_path = root("configs/systems/nucleo-f407.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F407 profile");
    let base = SystemManifest::from_file(root("examples/nucleo-f407/analog-ntc-diode-system.yaml"))
        .expect("load the unrelated generic NTC and Shockley diode netlists");
    let jfet = SystemManifest::from_file(root(
        "examples/nucleo-f407/analog-jfet-gate-charge-adc.yaml",
    ))
    .expect("load matched generic JFET gate-charge branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(jfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F407 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F407 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries JFET, NTC, and diode netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve PA0..PA3 to ADC1 channels 0..3"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    let expected_diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.jfet_gate.drive_v", 3.3)
        .expect("precharge both JFET gates at the drain/source reference");
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the unrelated NTC temperature");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle NTC, diode, and both JFET branches");
    assert_eq!(initial_steps.len(), 4, "all four analog netlists solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );
    assert!(
        (signal_voltage(&session, "board.analog.pa2_volts") - 3.3).abs() < 1e-9,
        "the charged JFET gate starts at the drain/source reference"
    );

    session
        .set_signal_number("ui.jfet_gate.drive_v", 0.0)
        .expect("step both JFET gate drives low");
    session
        .set_signal_number("ui.ntc.temperature_c", 80.0)
        .expect("heat the unrelated NTC while the JFET gates discharge");
    let resistance = ntc_resistance_ohms(80.0);
    let expected_ntc_v = 3.3 * 10_000.0 / (resistance + 10_000.0);
    let mut expected_gate_v = 3.3;
    for step in 0..8 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route JFET charge alongside NTC and diode");
        assert_eq!(steps.len(), 4, "all four netlists solve at every step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        for _ in 0..SUBSTEPS {
            expected_gate_v =
                expected_jfet_gate_voltage(expected_gate_v, 0.0, STEP_S / SUBSTEPS as f64);
        }
        let gate_v = signal_voltage(&session, "board.analog.pa2_volts");
        let control_v = signal_voltage(&session, "board.analog.pa3_volts");
        assert!(
            (gate_v - expected_gate_v).abs() < 2e-8,
            "JFET CGS/CGD depletion charge at step {step}: expected {expected_gate_v} V, got {gate_v} V"
        );
        assert!(
            control_v.abs() < 1e-8,
            "zero-charge control follows the low drive within the leakage floor, got {control_v} V"
        );
        assert!(
            gate_v > control_v,
            "depletion charge must delay the gate edge relative to its zero-charge control"
        );

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the unrelated NTC stays at its 80 °C divider voltage"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode stays at its DC load line"
        );

        for (channel, voltage_v) in [(0, ntc_v), (1, diode_v), (2, gate_v), (3, control_v)] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage seeds ADC1_IN{channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1_IN{channel} conversion reports the routed voltage"
            );
        }
    }
    assert!(
        signal_voltage(&session, "board.analog.pa2_volts") < 1.0,
        "gate depletion capacitance delays the edge but eventually follows the low drive"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 4),
        u16::MAX,
        "neighboring ADC1_IN4 remains undriven"
    );
}

#[test]
fn nucleo_f407_bjt_load_line_reaches_pa4_adc1_alongside_unrelated_devices() {
    let system_path = root("configs/systems/nucleo-f407.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F407 profile");
    assert_eq!(profile.name, "nucleo-f407");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f407/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and diode netlists");
    let mosfet = SystemManifest::from_file(root("examples/nucleo-f407/analog-mosfet-switch.yaml"))
        .expect("load generic level-1 NMOS netlist");
    let jfet = SystemManifest::from_file(root("examples/nucleo-f407/analog-jfet-load.yaml"))
        .expect("load generic level-1 JFET netlist");
    let bjt = SystemManifest::from_file(root("examples/nucleo-f407/analog-bjt-load.yaml"))
        .expect("load generic Gummel–Poon NPN netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);
    profile.cosim_models.extend(jfet.cosim_models);
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F407 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3), ("PA4", 4)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F407 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries five unrelated analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "NTC, diode, NMOS, JFET and NPN routes must resolve to distinct ADC1 inputs"
    );
    for channel in 0..=5 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let expected_mosfet_v = mosfet_drain_voltage(2.0);
    let expected_jfet_v = jfet_drain_voltage(-1.0);
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent NTC temperature");
    session
        .set_signal_number("ui.mosfet.gate_v", 2.0)
        .expect("set independent MOSFET gate voltage");
    session
        .set_signal_number("ui.jfet.gate_v", -1.0)
        .expect("set independent JFET gate voltage");

    let mut cycles = 0;
    let mut collector_samples = Vec::new();
    for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt.base_v", base_drive_v)
            .expect("set Gummel–Poon base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all five analog netlists");
        assert_eq!(
            steps.len(),
            5,
            "all five devices solve at the 1 us boundary"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let collector_v = signal_voltage(&session, "board.analog.pa4_volts");
        let expected_collector_v = bjt_collector_voltage(base_drive_v);
        assert!(
            (collector_v - expected_collector_v).abs() < 5e-5,
            "Gummel–Poon NPN load line at base drive {base_drive_v} V should be {expected_collector_v} V, got {collector_v} V"
        );
        let bjt_count = expected_adc_count(collector_v);
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            bjt_count,
            "the solved BJT collector voltage must seed ADC1 channel 4"
        );
        assert_eq!(
            stm32f4_adc_convert(&mut bus, 4),
            u32::from(bjt_count),
            "firmware-visible ADC1 channel 4 conversion must report the BJT load line"
        );

        for (channel, volts) in [
            (0, expected_ntc_v),
            (1, expected_diode_v),
            (2, expected_mosfet_v),
            (3, expected_jfet_v),
        ] {
            let count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                count,
                "the independent analog source must remain isolated on ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(count),
                "firmware-visible ADC1 channel {channel} conversion remains stable"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 5),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        collector_samples.push((base_drive_v, collector_v));
    }

    assert!(
        collector_samples
            .windows(2)
            .all(|pair| pair[1].1 < pair[0].1),
        "collector voltage falls monotonically as base drive increases: {collector_samples:?}"
    );
}

#[test]
fn nucleo_f407_bjt_junction_and_transit_charge_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f407.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F407 profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-f407/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let charge = SystemManifest::from_file(root("examples/nucleo-f407/analog-bjt-charge.yaml"))
        .expect("load matched BJT charge comparison netlists");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(charge.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F407 descriptor");
    for (pad, channel) in [
        ("PA0", 0),
        ("PA1", 1),
        ("PA2", 2),
        ("PA3", 3),
        ("PA4", 4),
        ("PA5", 5),
        ("PA6", 6),
    ] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F407 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and five matched BJT charge cases");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to ADC1 channels 0 through 6"
    );
    for channel in 0..=7 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_charge.base_v", 0.0)
        .expect("initialize all BJT base-drive sources");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize independent branches before the base-drive step");
    assert_eq!(initial_steps.len(), 7, "all seven analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );

    session
        .set_signal_number("ui.bjt_charge.base_v", 0.8)
        .expect("step the matched BJT base-drive sources");
    let mut samples = Vec::new();
    for step in 0..8_000 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the NTC, diode, and BJT charge branches");
        assert_eq!(steps.len(), 7, "all seven branches solve at each 1 us step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        let bjt_voltages = [
            signal_voltage(&session, "board.analog.pa2_volts"),
            signal_voltage(&session, "board.analog.pa3_volts"),
            signal_voltage(&session, "board.analog.pa4_volts"),
            signal_voltage(&session, "board.analog.pa5_volts"),
            signal_voltage(&session, "board.analog.pa6_volts"),
        ];
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the unrelated 25 °C NTC stays isolated"
        );
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated diode stays at its independent DC load line"
        );

        let expected_dc = bjt_collector_voltage(0.8);
        assert!(
            (bjt_voltages[0] - expected_dc).abs() < 5e-5,
            "the uncharged branch must match the independent Gummel–Poon load line {expected_dc} V, got {} V",
            bjt_voltages[0]
        );
        for (label, channel, voltage) in [
            ("NTC", 0, ntc_v),
            ("diode", 1, diode_v),
            ("uncharged BJT", 2, bjt_voltages[0]),
            ("CJE BJT", 3, bjt_voltages[1]),
            ("CJC BJT", 4, bjt_voltages[2]),
            ("TF BJT", 5, bjt_voltages[3]),
            ("combined-charge BJT", 6, bjt_voltages[4]),
        ] {
            let expected = expected_adc_count(voltage);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved {label} voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 channel {channel} must report the {label} voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 7),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );

        if step == 0 {
            for (label, voltage) in [
                ("CJE depletion", bjt_voltages[1]),
                ("CJC depletion", bjt_voltages[2]),
                ("TF transit", bjt_voltages[3]),
                ("combined junction and transit", bjt_voltages[4]),
            ] {
                assert!(
                    (voltage - bjt_voltages[0]).abs() > 0.01,
                    "{label} must change the first routed transient: reference={} V, charged={voltage} V",
                    bjt_voltages[0]
                );
            }
        }
        samples.push(bjt_voltages);
    }

    let settled = samples.last().expect("transient samples exist");
    for (label, voltage) in [
        ("CJE depletion", settled[1]),
        ("CJC depletion", settled[2]),
        ("TF transit", settled[3]),
        ("combined junction and transit", settled[4]),
    ] {
        assert!(
            (voltage - settled[0]).abs() < 2e-4,
            "{label} must converge to the same DC Gummel–Poon load line, got {voltage} V vs {} V",
            settled[0]
        );
    }
}
