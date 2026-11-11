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

/// Independent Gummel–Poon base-voltage reference with a fixed collector bias.
fn bjt_fixed_collector_base_voltage(base_drive_v: f64, collector_v: f64) -> f64 {
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-15;
    const BF: f64 = 100.0;
    const BR: f64 = 4.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let base_current = |vbe: f64| {
        let vbc = vbe - collector_v;
        IS_A / BF * ((vbe / VT).exp() - 1.0) + IS_A / BR * ((vbc / VT).exp() - 1.0)
    };
    let mut low = 0.0;
    let mut high = base_drive_v;
    for _ in 0..100 {
        let vbe = (low + high) / 2.0;
        let residual = (base_drive_v - vbe) / BASE_OHMS - base_current(vbe);
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    (low + high) / 2.0
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

/// Independent level-1 NJF load line including model-card drain/source
/// resistances. The scalar root solves channel current against the external
/// load and intrinsic terminal drops, independently of MNA.
fn jfet_drain_voltage(gate_v: f64, drain_ohms: f64, source_ohms: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const VTO: f64 = -2.0;
    const BETA_A_PER_V2: f64 = 500e-6;
    const LAMBDA_PER_V: f64 = 0.025;

    let channel_current = |current_a: f64| {
        let external_drain_v = SUPPLY_V - current_a * LOAD_OHMS;
        let internal_drain_v = external_drain_v - current_a * drain_ohms;
        let internal_source_v = current_a * source_ohms;
        let vds = internal_drain_v - internal_source_v;
        let vgst = gate_v - internal_source_v - VTO;
        if vgst <= 0.0 {
            0.0
        } else if vds < vgst {
            BETA_A_PER_V2 * (1.0 + LAMBDA_PER_V * vds) * vds * (2.0 * vgst - vds)
        } else {
            BETA_A_PER_V2 * (1.0 + LAMBDA_PER_V * vds) * vgst * vgst
        }
    };

    let (mut low, mut high) = (0.0, SUPPLY_V / LOAD_OHMS);
    for _ in 0..100 {
        let current_a = 0.5 * (low + high);
        if channel_current(current_a) > current_a {
            low = current_a;
        } else {
            high = current_a;
        }
    }
    SUPPLY_V - 0.5 * (low + high) * LOAD_OHMS
}

/// Independent level-1 NMOS body-effect load line. With its source grounded,
/// the bulk bias changes threshold through the GAMMA/PHI relation.
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
fn nucleo_f401re_jfet_terminal_resistances_reach_adc1_with_unrelated_diode() {
    let system_path = root("configs/systems/nucleo-f401re.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F401RE profile");
    let diode = SystemManifest::from_file(root("examples/nucleo-f401re/analog-diode-system.yaml"))
        .expect("load unrelated generic Shockley diode netlist");
    let jfet = SystemManifest::from_file(root(
        "examples/nucleo-f401re/analog-jfet-terminal-resistance.yaml",
    ))
    .expect("load matched generic level-1 JFET branches");
    profile.cosim_models = diode.cosim_models;
    profile.cosim_models.extend(jfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F401 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA2", 2), ("PA3", 3), ("PA4", 4), ("PA5", 5)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F401RE bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an unrelated diode and four matched JFET branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve through PA0 and PA2–PA5 to ADC1"
    );
    for channel in 0..=6 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} begins without an analog source"
        );
    }

    session
        .set_signal_number("ui.jfet.gate_v", -1.0)
        .expect("initialize the matched JFET gate drives");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize the diode and matched JFET branches");
    assert_eq!(initial_steps.len(), 2, "the diode and JFET netlists solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );

    let branches = [
        ("control", "pa2", 2, 0.0, 0.0),
        ("drain resistance", "pa3", 3, 200.0, 0.0),
        ("source resistance", "pa4", 4, 0.0, 200.0),
        ("both terminal resistances", "pa5", 5, 200.0, 200.0),
    ];
    let mut final_voltages = None;
    for gate_v in [-1.0_f64, 0.0] {
        session
            .set_signal_number("ui.jfet.gate_v", gate_v)
            .expect("step all matched JFET gate drives");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the unrelated diode and JFET branches");
        assert_eq!(
            steps.len(),
            2,
            "both analog netlists solve at each gate bias"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let diode_v = signal_voltage(&session, "board.analog.pa0_volts");
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated Shockley diode stays on its independent load line"
        );
        let mut voltages = Vec::new();
        for (name, pad, channel, drain_ohms, source_ohms) in branches {
            let solved_v = signal_voltage(&session, &format!("board.analog.{pad}_volts"));
            let expected_v = jfet_drain_voltage(gate_v, drain_ohms, source_ohms);
            assert!(
                (solved_v - expected_v).abs() < 5e-5,
                "{name} NJF load line at gate={gate_v} V should be {expected_v} V, got {solved_v} V"
            );
            let expected = expected_adc_count(solved_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "solved {name} voltage seeds descriptor-routed ADC1_IN{channel}"
            );
            assert_eq!(
                f401_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1_IN{channel} conversion follows the {name} JFET"
            );
            voltages.push(solved_v);
        }
        assert_eq!(
            f401_adc_convert(&mut bus, 0),
            u32::from(expected_adc_count(diode_v)),
            "ADC1_IN0 continues to convert the unrelated diode"
        );
        assert_eq!(
            adc_channel_count(&mut bus, 1),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        final_voltages = Some(voltages);
    }

    let [control, drain, source, both] = final_voltages
        .expect("the final JFET gate-drive sample was recorded")
        .try_into()
        .expect("all four JFET branches were routed");
    assert!(
        drain > control + 0.05,
        "drain resistance reduces channel current and raises external drain voltage: {drain} V vs {control} V"
    );
    assert!(
        source > control + 0.1,
        "source resistance raises intrinsic source voltage and reduces channel current: {source} V vs {control} V"
    );
    assert!(
        both > drain && both > source,
        "combined RD and RS exceed either terminal alone: {both} V, {drain} V, {source} V"
    );
}

#[test]
fn nucleo_f401re_mosfet_body_effect_reaches_adc1_with_unrelated_diode() {
    let system_path = root("configs/systems/nucleo-f401re.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F401RE profile");
    let diode = SystemManifest::from_file(root("examples/nucleo-f401re/analog-diode-system.yaml"))
        .expect("load unrelated generic Shockley diode netlist");
    let body_effect = SystemManifest::from_file(root(
        "examples/nucleo-f401re/analog-mosfet-body-effect.yaml",
    ))
    .expect("load matched generic NMOS body-bias netlist");
    profile.cosim_models = diode.cosim_models;
    profile.cosim_models.extend(body_effect.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F401 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F401RE bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries a diode and a matched generic NMOS pair");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve through PA0 and PA2/PA3 to ADC1"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} begins without an analog source"
        );
    }

    session
        .set_signal_number("ui.mosfet_body.bulk_v", 0.0)
        .expect("initialize the matched NMOS bulk bias");
    let expected_diode_v = diode_load_line_voltage();
    let expected_reference_v = mosfet_body_effect_drain_voltage(2.5, 0.0);
    let mut cycles = 0;
    let mut body_samples = Vec::new();
    for bulk_v in [0.0_f64, -0.5, -1.0, -1.5] {
        session
            .set_signal_number("ui.mosfet_body.bulk_v", bulk_v)
            .expect("set generic NMOS reverse body bias");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the diode and matched NMOS branches");
        assert_eq!(
            steps.len(),
            2,
            "both analog netlists solve at the 1 us boundary"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let diode_v = signal_voltage(&session, "board.analog.pa0_volts");
        let reference_v = signal_voltage(&session, "board.analog.pa2_volts");
        let body_v = signal_voltage(&session, "board.analog.pa3_volts");
        let expected_body_v = mosfet_body_effect_drain_voltage(2.5, bulk_v);
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode stays on its independent load line"
        );
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "zero-body-bias NMOS should settle at {expected_reference_v} V, got {reference_v} V"
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
        } else {
            assert!(
                body_v > reference_v,
                "reverse body bias raises threshold, lowers current, and raises the drain voltage"
            );
        }

        for (channel, volts) in [(0, diode_v), (2, reference_v), (3, body_v)] {
            let expected = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "solved voltage seeds descriptor-routed ADC1_IN{channel}"
            );
            assert_eq!(
                f401_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1_IN{channel} conversion reports the routed voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 1),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        body_samples.push((bulk_v, body_v));
    }

    assert!(
        body_samples.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "increasing reverse body bias raises the NMOS drain monotonically: {body_samples:?}"
    );
}

#[test]
fn nucleo_f401re_bjt_terminal_resistances_reach_adc1_with_explicit_network_reference() {
    let system_path = root("configs/systems/nucleo-f401re.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F401RE profile");
    let diode = SystemManifest::from_file(root("examples/nucleo-f401re/analog-diode-system.yaml"))
        .expect("load unrelated generic diode netlist");
    let terminal_resistances = SystemManifest::from_file(root(
        "examples/nucleo-f401re/analog-bjt-terminal-resistance.yaml",
    ))
    .expect("load matched generic BJT terminal-resistance netlists");
    profile.cosim_models = diode.cosim_models;
    profile
        .cosim_models
        .extend(terminal_resistances.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F401 descriptor");
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

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F401RE bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries one diode and six matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to ADC1 channels zero through six"
    );
    for channel in 0..=7 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} begins without an analog source"
        );
    }

    session
        .set_signal_number("ui.bjt_terminal.base_v", 0.0)
        .expect("initialize all matched BJT base drives");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize the diode and matched BJT branches");
    assert_eq!(initial_steps.len(), 7, "all seven analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );

    let mut settled = None;
    for base_drive_v in [0.7_f64, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt_terminal.base_v", base_drive_v)
            .expect("step all matched BJT base drives");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route diode and BJT terminal-resistance branches");
        assert_eq!(
            steps.len(),
            7,
            "all seven branches solve at each base drive"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let diode_v = signal_voltage(&session, "board.analog.pa0_volts");
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated Shockley diode stays on its independent load line"
        );
        let voltages = [
            signal_voltage(&session, "board.analog.pa1_volts"),
            signal_voltage(&session, "board.analog.pa2_volts"),
            signal_voltage(&session, "board.analog.pa3_volts"),
            signal_voltage(&session, "board.analog.pa4_volts"),
            signal_voltage(&session, "board.analog.pa5_volts"),
            signal_voltage(&session, "board.analog.pa6_volts"),
        ];
        assert!(
            (voltages[0] - bjt_collector_voltage(base_drive_v)).abs() < 5e-5,
            "zero-resistance Gummel–Poon branch must match the independent load line"
        );
        assert!(
            (voltages[4] - voltages[5]).abs() < 1e-7,
            "model-card RB/RC/RE must match explicit series resistors: {} V vs {} V",
            voltages[4],
            voltages[5]
        );

        if base_drive_v == 0.9 {
            for (label, voltage) in [
                ("RB", voltages[1]),
                ("RC", voltages[2]),
                ("RE", voltages[3]),
                ("RB+RC+RE", voltages[4]),
            ] {
                assert!(
                    (voltage - voltages[0]).abs() > 1e-3,
                    "{label} must change the loaded collector voltage, zero-R={} V, actual={voltage} V",
                    voltages[0]
                );
            }
        }

        settled = Some((diode_v, voltages));
    }

    let (diode_v, bjt_voltages) = settled.expect("final base-drive sample exists");
    for (channel, voltage) in std::iter::once((0, diode_v)).chain(
        bjt_voltages
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
        adc_channel_count(&mut bus, 7),
        0xFFFF,
        "the neighboring ADC1 channel remains undriven"
    );
}

#[test]
fn nucleo_f401re_bjt_bias_dependent_transit_charge_reaches_adc1_with_diode() {
    let system_path = root("configs/systems/nucleo-f401re.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F401RE profile");
    let diode = SystemManifest::from_file(root("examples/nucleo-f401re/analog-diode-system.yaml"))
        .expect("load unrelated generic Shockley diode netlist");
    let transit = SystemManifest::from_file(root(
        "examples/nucleo-f401re/analog-bjt-bias-transit-charge.yaml",
    ))
    .expect("load matched generic BJT transit-charge branches");
    profile.cosim_models = diode.cosim_models;
    profile.cosim_models.extend(transit.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F401 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3), ("PA4", 4)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F401RE bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an unrelated diode and four matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to ADC1 channels zero through four"
    );
    for channel in 0..=5 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} begins without an analog source"
        );
    }

    session
        .set_signal_number("ui.bjt_transit.base_v", 0.0)
        .expect("initialize all matched BJT branches");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize the diode and matched BJT branches");
    assert_eq!(initial_steps.len(), 5, "all five analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );

    session
        .set_signal_number("ui.bjt_transit.base_v", 0.8)
        .expect("step the matched BJT base drives");
    let mut samples = Vec::new();
    for step in 0..10_000 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route diode and BJT transit-charge branches");
        assert_eq!(steps.len(), 5, "all five branches solve at each 1 us step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let diode_v = signal_voltage(&session, "board.analog.pa0_volts");
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated Shockley diode stays on its independent load line"
        );
        let base_voltages = [
            signal_voltage(&session, "board.analog.pa1_volts"),
            signal_voltage(&session, "board.analog.pa2_volts"),
            signal_voltage(&session, "board.analog.pa3_volts"),
            signal_voltage(&session, "board.analog.pa4_volts"),
        ];
        for (channel, voltage) in (1..=4).zip(base_voltages) {
            let expected = expected_adc_count(voltage);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved BJT base voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                f401_adc_convert(&mut bus, channel),
                u32::from(expected),
                "ADC1 channel {channel} must convert its routed BJT base voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 5),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );

        if step == 0 {
            let [tf_only, bias_dependent, no_vtf, no_itf] = base_voltages;
            assert!(
                tf_only > 0.1 && bias_dependent > 0.1,
                "the fixed 0.1 V collector leaves both transit branches with positive VBC"
            );
            assert!(
                bias_dependent < tf_only - 1e-3,
                "positive VBC with XTF/VTF/ITF must increase stored charge and lower the first-step base voltage: TF={tf_only} V, bias-dependent={bias_dependent} V"
            );
            assert!(
                bias_dependent < no_vtf - 1e-3 && no_vtf < tf_only - 1e-3,
                "VTF's positive-VBC exponential must increase transit charge beyond VTF=0: active={bias_dependent} V, VTF=0={no_vtf} V, control={tf_only} V"
            );
            assert!(
                no_itf < bias_dependent - 1e-3,
                "ITF=0 must remove current suppression and increase stored charge: ITF=20u={bias_dependent} V, ITF=0={no_itf} V"
            );
        }
        samples.push(base_voltages);
    }

    let settled = samples.last().expect("transient samples exist");
    for (label, voltage) in [
        ("TF control", settled[0]),
        ("XTF/VTF/ITF", settled[1]),
        ("VTF=0", settled[2]),
        ("ITF=0", settled[3]),
    ] {
        assert!(
            (voltage - bjt_fixed_collector_base_voltage(0.8, 0.1)).abs() < 5e-3,
            "{label} must converge to the independent DC base voltage, expected {} V, got {voltage} V",
            bjt_fixed_collector_base_voltage(0.8, 0.1)
        );
    }
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
