// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the STM32F411CEU6 Black Pill profile into ADC1.

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
        .expect("STM32F411CEU6 declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<labwired_core::peripherals::adc::Adc>()
        .expect("ADC1 uses the shared STM32 ADC model")
        .channel_input_count(channel)
}

/// Perform a one-shot through the F4 register interface used by firmware.
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

/// Independent backward-Euler update for the two gate junctions tied to 3.3 V.
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

/// Independent Gummel–Poon load line with forward/reverse high-injection
/// rolloff and matched Early voltages in the device and control branches.
fn bjt_high_injection_collector_voltage(base_drive_v: f64, high_injection: bool) -> f64 {
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
        let base_emitter = (IS_A / BF) * (forward - 1.0);
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

/// Independent Gummel–Poon load line with optional forward Early effect.
fn bjt_early_collector_voltage(base_drive_v: f64, forward_early_voltage: Option<f64>) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const VAR: f64 = 75.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let base_width = 1.0 - forward_early_voltage.map_or(0.0, |vaf| vbc / vaf) - vbe / VAR;
        let transport = IS_A * (forward - reverse) * base_width;
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        let base_emitter = (IS_A / BF) * (forward - 1.0);
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

/// Independent Gummel–Poon load line with optional base-emitter recombination.
fn bjt_recombination_collector_voltage(base_drive_v: f64, recombination: bool) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const VAF: f64 = 50.0;
    const VAR: f64 = 75.0;
    const ISE_A: f64 = 2e-12;
    const NE: f64 = 1.5;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let base_width = 1.0 - vbc / VAF - vbe / VAR;
        let transport = IS_A * (forward - reverse) * base_width;
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        let base_emitter = (IS_A / BF) * (forward - 1.0)
            + if recombination {
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

#[test]
fn stm32f411ceu6_blackpill_ntc_and_diode_netlists_reach_pa1_pa4_adc1_end_to_end() {
    let system_path = root("configs/systems/stm32f411ceu6-blackpill.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load STM32F411CEU6 Black Pill profile");
    assert_eq!(profile.name, "stm32f411ceu6-blackpill");
    assert!(profile.external_devices.is_empty());
    assert!(
        profile
            .board_io
            .iter()
            .any(|io| io.id == "led_pc13" && io.peripheral == "gpioc" && io.pin == 13),
        "the analog fixture preserves the configured PC13 LED mapping"
    );
    assert!(
        profile
            .board_io
            .iter()
            .any(|io| io.id == "button_user_pa0" && io.peripheral == "gpioa" && io.pin == 0),
        "the analog fixture preserves the configured PA0 button mapping"
    );

    let analog = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F411CEU6 descriptor");
    assert_eq!(chip.analog_pins["PA1"].channel, 1);
    assert_eq!(chip.analog_pins["PA4"].channel, 4);

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Black Pill bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve through PA1/PA4 to ADC1 channels 1/4"
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
    assert_eq!(ntc_samples[1].1, 2047, "the 25 °C divider is half scale");
}

#[test]
fn stm32f411ceu6_blackpill_mosfet_load_line_reaches_pa0_adc1_end_to_end() {
    let system_path = root("configs/systems/stm32f411ceu6-blackpill.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load STM32F411CEU6 Black Pill profile");
    let analog = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-ntc-diode-system.yaml",
    ))
    .expect("load the existing generic NTC and diode netlists");
    let mosfet = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-mosfet-switch.yaml",
    ))
    .expect("load the generic low-side MOSFET netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F411CEU6 descriptor");
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
    for channel in [0, 1, 2, 3, 4, 5] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} begins without an analog source"
        );
    }

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
            "unrouted neighboring ADC1 inputs remain isolated"
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

#[test]
fn stm32f411ceu6_jfet_gate_depletion_charge_reaches_adc1_with_ntc_and_diode() {
    const STEP_S: f64 = 1e-6;
    const SUBSTEPS: usize = 10;

    let system_path = root("configs/systems/stm32f411ceu6-blackpill.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load STM32F411CEU6 Black Pill profile");
    let base = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-ntc-diode-system.yaml",
    ))
    .expect("load the unrelated generic NTC and Shockley diode netlists");
    let jfet = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-jfet-gate-charge-adc.yaml",
    ))
    .expect("load matched generic JFET gate-charge branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(jfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F411CEU6 descriptor");
    for (pad, channel) in [("PA1", 1), ("PA2", 2), ("PA3", 3), ("PA4", 4)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Black Pill bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries JFET, NTC, and diode netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve PA1..PA4 to ADC1 channels 1..4"
    );
    for channel in 0..=5 {
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

        let ntc_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the unrelated NTC stays at its 80 °C divider voltage"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa4_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode stays at its DC load line"
        );

        for (channel, voltage_v) in [(1, ntc_v), (2, gate_v), (3, control_v), (4, diode_v)] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 channel {channel} conversion reports the routed voltage"
            );
        }
    }
    assert!(
        signal_voltage(&session, "board.analog.pa2_volts") < 1.0,
        "gate depletion capacitance delays the edge but eventually follows the low drive"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 0),
        u16::MAX,
        "neighboring ADC1_IN0 remains undriven"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 5),
        u16::MAX,
        "neighboring ADC1_IN5 remains undriven"
    );
}

#[test]
fn stm32f411ceu6_bjt_high_injection_reaches_adc1_with_ntc_and_diode() {
    let system_path = root("configs/systems/stm32f411ceu6-blackpill.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load STM32F411CEU6 Black Pill profile");
    let base = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-ntc-diode-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let bjt = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-bjt-high-injection-adc.yaml",
    ))
    .expect("load matched generic Gummel–Poon high-injection branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F411CEU6 descriptor");
    for (pad, channel) in [("PA1", 1), ("PA4", 4), ("PA5", 5), ("PA6", 6)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Black Pill bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries BJT, NTC, and diode netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve PA1/PA4/PA5/PA6 to ADC1 channels 1/4/5/6"
    );
    for channel in 0..=7 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt.high_drive_v", 0.0)
        .expect("initialize high-injection BJT base drive");
    session
        .set_signal_number("ui.bjt.control_drive_v", 0.0)
        .expect("initialize control BJT base drive");

    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut high_drive_voltages = None;
    for base_drive_v in [0.8_f64, 1.5, 3.3] {
        session
            .set_signal_number("ui.bjt.high_drive_v", base_drive_v)
            .expect("set high-injection BJT base drive");
        session
            .set_signal_number("ui.bjt.control_drive_v", base_drive_v)
            .expect("set control BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched BJT branches");
        assert_eq!(steps.len(), 3, "NTC, diode, and BJT co-simulations solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa1_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa4_volts");
        let rolloff_v = signal_voltage(&session, "board.analog.pa5_volts");
        let control_v = signal_voltage(&session, "board.analog.pa6_volts");
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "unrelated NTC remains isolated on PA1/ADC1 channel 1"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains isolated on PA4/ADC1 channel 4"
        );

        let expected_rolloff_v = bjt_high_injection_collector_voltage(base_drive_v, true);
        let expected_control_v = bjt_high_injection_collector_voltage(base_drive_v, false);
        assert!(
            (rolloff_v - expected_rolloff_v).abs() < 5e-5,
            "IKF/IKR/NKF load line at base drive {base_drive_v} V should be {expected_rolloff_v} V, got {rolloff_v} V"
        );
        assert!(
            (control_v - expected_control_v).abs() < 5e-5,
            "matched no-rolloff load line at base drive {base_drive_v} V should be {expected_control_v} V, got {control_v} V"
        );

        for (channel, voltage_v) in [(1, ntc_v), (4, diode_v), (5, rolloff_v), (6, control_v)] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 conversion follows channel {channel}"
            );
        }
        assert_eq!(
            [0, 2, 3, 7].map(|channel| adc_channel_count(&mut bus, channel)),
            [u16::MAX; 4],
            "neighboring ADC1 channels remain undriven"
        );

        if base_drive_v == 3.3 {
            high_drive_voltages = Some((rolloff_v, control_v));
        }
    }

    let (rolloff_v, control_v) = high_drive_voltages.expect("high-drive sample was recorded");
    assert!(
        rolloff_v - control_v > 0.05,
        "IKF/IKR/NKF rolloff reduces collector current and raises collector voltage: {rolloff_v} V vs {control_v} V"
    );
}

#[test]
fn stm32f411ceu6_bjt_early_effect_reaches_adc1_with_ntc_and_diode() {
    let system_path = root("configs/systems/stm32f411ceu6-blackpill.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load STM32F411CEU6 Black Pill profile");
    let unrelated = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-ntc-diode-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let bjt = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-bjt-early-effect-adc.yaml",
    ))
    .expect("load matched generic Gummel–Poon Early-effect branches");
    profile.cosim_models = unrelated.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F411CEU6 descriptor");
    for (pad, channel) in [("PA1", 1), ("PA2", 2), ("PA3", 3), ("PA4", 4)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
        assert_eq!(chip.analog_pins[pad].peripheral, "adc1");
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Black Pill bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve PA1/PA2/PA3/PA4 to ADC1 channels 1/2/3/4"
    );
    for channel in 0..=5 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_early.base_v", 0.0)
        .expect("initialize matched BJT base drives");
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut early_samples = Vec::new();
    let mut reference_samples = Vec::new();
    for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt_early.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched BJT branches");
        assert_eq!(steps.len(), 4, "all four analog branches solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa1_volts");
        let early_v = signal_voltage(&session, "board.analog.pa2_volts");
        let reference_v = signal_voltage(&session, "board.analog.pa3_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa4_volts");
        let expected_early_v = bjt_early_collector_voltage(base_drive_v, Some(50.0));
        let expected_reference_v = bjt_early_collector_voltage(base_drive_v, None);
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "unrelated 25 °C NTC stays isolated"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains on its independent load line"
        );
        assert!(
            (early_v - expected_early_v).abs() < 5e-5,
            "VAF=50 load line should be {expected_early_v} V at {base_drive_v} V drive, got {early_v} V"
        );
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "matched no-forward-Early load line should be {expected_reference_v} V at {base_drive_v} V drive, got {reference_v} V"
        );
        if base_drive_v >= 0.7 {
            assert!(
                early_v < reference_v - 0.001,
                "finite VAF increases collector current and lowers collector voltage: {early_v} V vs {reference_v} V"
            );
        }

        for (channel, voltage_v) in [(1, ntc_v), (2, early_v), (3, reference_v), (4, diode_v)] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 conversion follows channel {channel}"
            );
        }
        assert_eq!(
            [0, 5].map(|channel| adc_channel_count(&mut bus, channel)),
            [u16::MAX; 2],
            "neighboring ADC1 channels remain undriven"
        );
        early_samples.push(early_v);
        reference_samples.push(reference_v);
    }
    assert!(
        early_samples[1] > early_samples[2] && early_samples[2] > early_samples[3],
        "increasing base drive lowers the finite-VAF collector voltage: {early_samples:?}"
    );
    assert!(
        reference_samples[1] > reference_samples[2] && reference_samples[2] > reference_samples[3],
        "increasing base drive lowers the no-forward-Early collector voltage: {reference_samples:?}"
    );
}

#[test]
fn stm32f411ceu6_bjt_base_recombination_reaches_adc1_with_ntc_and_diode() {
    let system_path = root("configs/systems/stm32f411ceu6-blackpill.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load STM32F411CEU6 Black Pill profile");
    let base = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-ntc-diode-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let bjt = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-bjt-base-recombination-adc.yaml",
    ))
    .expect("load matched generic Gummel–Poon recombination branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F411CEU6 descriptor");
    for (pad, channel) in [("PA1", 1), ("PA4", 4), ("PB0", 8), ("PB1", 9)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Black Pill bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve PA1/PA4/PB0/PB1 to ADC1 channels 1/4/8/9"
    );
    for channel in 0..=9 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} begins without an analog source"
        );
    }

    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_recombination.base_v", 0.0)
        .expect("initialize matched BJT base drives");
    let mut cycles = 0;
    let mut recombination_deltas = Vec::new();
    for base_drive_v in [0.70_f64, 0.78, 0.86] {
        session
            .set_signal_number("ui.bjt_recombination.base_v", base_drive_v)
            .expect("set matched BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and BJT recombination branches");
        assert_eq!(steps.len(), 4, "all four unrelated netlists solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa1_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa4_volts");
        let reference_v = signal_voltage(&session, "board.analog.pb0_volts");
        let recombination_v = signal_voltage(&session, "board.analog.pb1_volts");
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the independent NTC remains isolated on PA1/ADC1 channel 1"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley diode remains isolated on PA4/ADC1 channel 4"
        );

        let expected_reference_v = bjt_recombination_collector_voltage(base_drive_v, false);
        let expected_recombination_v = bjt_recombination_collector_voltage(base_drive_v, true);
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "the control BJT must match the independent load line {expected_reference_v} V, got {reference_v} V"
        );
        assert!(
            (recombination_v - expected_recombination_v).abs() < 5e-5,
            "the ISE/NE BJT must match the independent load line {expected_recombination_v} V, got {recombination_v} V"
        );
        recombination_deltas.push((recombination_v - reference_v).abs());

        for (channel, voltage_v) in [
            (1, ntc_v),
            (4, diode_v),
            (8, reference_v),
            (9, recombination_v),
        ] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 conversion follows channel {channel}"
            );
        }
        assert_eq!(
            [0, 2, 3, 5, 6, 7].map(|channel| adc_channel_count(&mut bus, channel)),
            [u16::MAX; 6],
            "neighboring ADC1 channels remain undriven"
        );
    }
    assert!(
        recombination_deltas.iter().any(|delta| *delta > 0.02),
        "ISE/NE base-emitter recombination must measurably change the collector load line: {recombination_deltas:?}"
    );
}

#[test]
fn stm32f411ceu6_bjt_junction_and_transit_charge_reach_adc1_with_ntc_and_diode() {
    let system_path = root("configs/systems/stm32f411ceu6-blackpill.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load STM32F411CEU6 Black Pill profile");
    let unrelated = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-ntc-diode-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let charge = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-bjt-charge-adc.yaml",
    ))
    .expect("load matched generic Gummel–Poon charge branches");
    profile.cosim_models = unrelated.cosim_models;
    profile.cosim_models.extend(charge.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F411CEU6 descriptor");
    for (pad, channel) in [
        ("PA1", 1),
        ("PA2", 2),
        ("PA3", 3),
        ("PA4", 4),
        ("PA5", 5),
        ("PA6", 6),
        ("PB0", 8),
    ] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Black Pill bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and five BJT charge branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve NTC, diode, and BJT branches to ADC1 channels"
    );
    for channel in 0..=9 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} begins without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_charge.base_v", 0.0)
        .expect("initialize matched BJT base-drive sources");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize NTC, diode, and matched BJT branches");
    assert_eq!(initial_steps.len(), 7, "all seven analog branches solve");
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
            .expect("solve and route NTC, diode, and BJT charge branches");
        assert_eq!(
            steps.len(),
            7,
            "all seven analog branches solve at every step"
        );
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa1_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa4_volts");
        assert!((ntc_v - 1.65).abs() < 1e-9, "unrelated NTC stays isolated");
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "unrelated Shockley diode stays on its independent load line"
        );
        let bjt_voltages = [
            signal_voltage(&session, "board.analog.pa2_volts"),
            signal_voltage(&session, "board.analog.pa3_volts"),
            signal_voltage(&session, "board.analog.pa5_volts"),
            signal_voltage(&session, "board.analog.pa6_volts"),
            signal_voltage(&session, "board.analog.pb0_volts"),
        ];

        if step == 0 {
            let expected_dc = bjt_recombination_collector_voltage(0.8, false);
            assert!(
                (bjt_voltages[0] - expected_dc).abs() < 5e-5,
                "uncharged generic BJT must match the independent Gummel–Poon load line"
            );
            for (label, voltage_v) in [
                ("CJE depletion", bjt_voltages[1]),
                ("CJC depletion", bjt_voltages[2]),
                ("TF transit", bjt_voltages[3]),
                ("combined junction and transit", bjt_voltages[4]),
            ] {
                assert!(
                    (voltage_v - bjt_voltages[0]).abs() > 0.01,
                    "{label} must change the first transient: reference={} V, charged={voltage_v} V",
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
    let expected_dc = bjt_recombination_collector_voltage(0.8, false);
    for (label, voltage_v) in [
        ("control", settled[0]),
        ("CJE depletion", settled[1]),
        ("CJC depletion", settled[2]),
        ("TF transit", settled[3]),
        ("combined junction and transit", settled[4]),
    ] {
        assert!(
            (voltage_v - expected_dc).abs() < 2e-4,
            "{label} must settle to the independent DC load line {expected_dc} V, got {voltage_v} V"
        );
    }
    assert!(
        first[1..]
            .iter()
            .zip(settled[1..].iter())
            .all(|(early, late)| (early - late).abs() > 0.01),
        "junction and transit charge contributions must decay between early and settled samples"
    );

    let ntc_v = signal_voltage(&session, "board.analog.pa1_volts");
    let diode_v = signal_voltage(&session, "board.analog.pa4_volts");
    for (channel, voltage_v) in std::iter::once((1, ntc_v))
        .chain(std::iter::once((4, diode_v)))
        .chain(
            settled
                .iter()
                .enumerate()
                .map(|(idx, voltage_v)| ([2, 3, 5, 6, 8][idx], *voltage_v)),
        )
    {
        let expected = expected_adc_count(voltage_v);
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            expected,
            "solved voltage seeds descriptor-routed ADC1 channel {channel}"
        );
        assert_eq!(
            stm32f4_adc_convert(&mut bus, channel),
            u32::from(expected),
            "firmware-visible ADC1 conversion follows channel {channel}"
        );
    }
    assert_eq!(
        [0, 7, 9].map(|channel| adc_channel_count(&mut bus, channel)),
        [u16::MAX; 3],
        "neighboring ADC1 channels remain undriven"
    );
}

#[test]
fn stm32f411ceu6_bjt_terminal_resistances_reach_adc1_with_explicit_network_reference() {
    let system_path = root("configs/systems/stm32f411ceu6-blackpill.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load STM32F411CEU6 Black Pill profile");
    let generic = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-ntc-diode-system.yaml",
    ))
    .expect("load unrelated generic NTC and diode routes");
    let terminal_resistances = SystemManifest::from_file(root(
        "examples/stm32f411ceu6-blackpill/analog-bjt-terminal-resistance-adc.yaml",
    ))
    .expect("load matched generic Gummel–Poon terminal-resistance branches");
    profile.cosim_models = generic.cosim_models;
    profile
        .cosim_models
        .extend(terminal_resistances.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F411CEU6 descriptor");
    for (pad, channel) in [
        ("PA1", 1),
        ("PA2", 2),
        ("PA3", 3),
        ("PA4", 4),
        ("PA5", 5),
        ("PA6", 6),
        ("PB0", 8),
        ("PB1", 9),
    ] {
        assert_eq!(
            chip.analog_pins[pad].channel, channel,
            "{pad} must resolve to ADC1 input {channel}"
        );
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Black Pill bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and six matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to ADC1 channels 1–6, 8, and 9"
    );
    for channel in 0..=9 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} begins without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_terminal.base_v", 0.0)
        .expect("initialize all matched BJT base drives");

    let mut cycles = 0;
    let mut final_voltages = None;
    for base_drive_v in [0.7_f64, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt_terminal.base_v", base_drive_v)
            .expect("step all matched BJT base drives");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and BJT terminal-resistance branches");
        assert_eq!(steps.len(), 8, "all eight generic analog branches solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa1_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa4_volts");
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the unrelated NTC remains isolated"
        );
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated Shockley diode remains on its independent load line"
        );

        let bjt_voltages = [
            signal_voltage(&session, "board.analog.pa2_volts"),
            signal_voltage(&session, "board.analog.pa3_volts"),
            signal_voltage(&session, "board.analog.pa5_volts"),
            signal_voltage(&session, "board.analog.pa6_volts"),
            signal_voltage(&session, "board.analog.pb0_volts"),
            signal_voltage(&session, "board.analog.pb1_volts"),
        ];
        let control_v = bjt_voltages[0];
        let expected_control_v = bjt_early_collector_voltage(base_drive_v, Some(50.0));
        assert!(
            (control_v - expected_control_v).abs() < 5e-5,
            "zero-resistance Gummel–Poon branch at {base_drive_v} V should match independent load line {expected_control_v} V, got {control_v} V"
        );
        assert!(
            (bjt_voltages[4] - bjt_voltages[5]).abs() < 1e-7,
            "model-card RB/RC/RE must match explicit external resistors: {} V vs {} V",
            bjt_voltages[4],
            bjt_voltages[5]
        );

        for (channel, voltage_v) in std::iter::once((1, ntc_v))
            .chain(std::iter::once((4, diode_v)))
            .chain([2, 3, 5, 6, 8, 9].into_iter().zip(bjt_voltages))
        {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 conversion follows channel {channel}"
            );
        }

        if base_drive_v == 0.9 {
            for (label, voltage_v) in [
                ("RB", bjt_voltages[1]),
                ("RC", bjt_voltages[2]),
                ("RE", bjt_voltages[3]),
                ("RB+RC+RE", bjt_voltages[4]),
            ] {
                assert!(
                    (voltage_v - control_v).abs() > 1e-3,
                    "{label} must change the collector load line: no-resistance={control_v} V, actual={voltage_v} V"
                );
            }
        }
        final_voltages = Some(bjt_voltages);
    }

    let bjt_voltages = final_voltages.expect("final base-drive sample exists");
    for (channel, voltage_v) in [2, 3, 5, 6, 8, 9].into_iter().zip(bjt_voltages) {
        let expected = expected_adc_count(voltage_v);
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            expected,
            "final BJT load line remains routed to ADC1 channel {channel}"
        );
    }
    assert_eq!(
        [0, 7].map(|channel| adc_channel_count(&mut bus, channel)),
        [u16::MAX; 2],
        "unrouted ADC1 neighbors remain undriven"
    );
}
