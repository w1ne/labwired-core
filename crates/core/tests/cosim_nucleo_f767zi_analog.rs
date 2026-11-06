// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the NUCLEO-F767ZI STM32F767 profile into ADC1.

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

/// Independent Gummel–Poon NPN load-line reference with forward Early effect.
/// The base-drive and collector-load roots are solved separately from the MNA
/// result so this catches a wrong transistor equation or route value.
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
        let residual = (base_drive_v - vbe) / BASE_OHMS - base_current;
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe(0.5 * (low + high))
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
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
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
        .expect("STM32F767 declares ADC1");
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

#[test]
fn nucleo_f767zi_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    assert_eq!(profile.name, "nucleo-f767zi");
    assert!(profile.external_devices.is_empty());
    assert!(
        profile
            .board_io
            .iter()
            .any(|io| io.id == "led1_pb0" && io.peripheral == "gpiob" && io.pin == 0),
        "the mixed-signal run keeps the NUCLEO-F767ZI's PB0 user LED"
    );
    assert!(
        profile.board_io.iter().any(|io| {
            io.id == "button_user_pc13"
                && io.peripheral == "gpioc"
                && io.pin == 13
                && io.active_high
        }),
        "the mixed-signal run keeps the NUCLEO-F767ZI's PC13 user button"
    );

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    assert_eq!(chip.analog_pins["PA0"].channel, 0);
    assert_eq!(chip.analog_pins["PA1"].channel, 1);
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
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
fn nucleo_f767zi_bjt_load_line_reaches_pa2_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    assert_eq!(profile.name, "nucleo-f767zi");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and diode netlists");
    let bjt = SystemManifest::from_file(root("examples/nucleo-f767zi/analog-bjt-load.yaml"))
        .expect("load generic Gummel–Poon NPN netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and BJT devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic NTC, diode, and NPN netlists must resolve to ADC1 channels 0/1/2"
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
        .expect("set independent NTC temperature");

    let mut cycles = 0;
    let mut collector_samples = Vec::new();
    for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt.base_v", base_drive_v)
            .expect("set Gummel–Poon base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all three analog netlists");
        assert_eq!(steps.len(), 3, "all devices solve at the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let collector_v = signal_voltage(&session, "board.analog.pa2_volts");
        let expected_collector_v = bjt_collector_voltage(base_drive_v);
        assert!(
            (collector_v - expected_collector_v).abs() < 5e-5,
            "Gummel–Poon NPN load line at base drive {base_drive_v} V should be {expected_collector_v} V, got {collector_v} V"
        );
        let bjt_count = expected_adc_count(collector_v);
        assert_eq!(
            adc_channel_count(&mut bus, 2),
            bjt_count,
            "the solved BJT collector voltage must seed ADC1 channel 2"
        );
        assert_eq!(
            stm32f4_adc_convert(&mut bus, 2),
            u32::from(bjt_count),
            "firmware-visible ADC1 channel 2 conversion must report the BJT load line"
        );

        for (channel, volts) in [(0, expected_ntc_v), (1, expected_diode_v)] {
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
            adc_channel_count(&mut bus, 3),
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
fn nucleo_f767zi_mosfet_load_line_reaches_pa3_adc1_alongside_unrelated_devices() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    assert_eq!(profile.name, "nucleo-f767zi");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load the independent generic NTC and diode netlists");
    let mosfet =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-mosfet-switch.yaml"))
            .expect("load the generic low-side MOSFET netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries three unrelated analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "NTC, diode, and NMOS netlists must resolve to distinct ADC1 inputs"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    const NTC_V: f64 = 1.65;
    let diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent NTC temperature");

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

        let drain_v = signal_voltage(&session, "board.analog.pa3_volts");
        let expected_drain_v = mosfet_drain_voltage(gate_v);
        assert!(
            (drain_v - expected_drain_v).abs() < 5e-5,
            "level-1 NMOS load line at Vgs={gate_v} V should be {expected_drain_v} V, got {drain_v} V"
        );
        for (channel, volts) in [(3, drain_v), (0, NTC_V), (1, diode_v)] {
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
            (signal_voltage(&session, "board.analog.pa0_volts") - NTC_V).abs() < 1e-9,
            "the independent Beta NTC divider remains on PA0/ADC1 channel 0"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa1_volts") - diode_v).abs() < 2e-5,
            "the independent Shockley diode remains on PA1/ADC1 channel 1"
        );
        for channel in [2, 4] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                0xFFFF,
                "neighboring ADC1 channel {channel} remains undriven"
            );
        }
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
fn nucleo_f767zi_jfet_load_line_reaches_pa4_adc1_alongside_unrelated_devices() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    assert_eq!(profile.name, "nucleo-f767zi");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load the independent generic NTC and diode netlists");
    let mosfet =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-mosfet-switch.yaml"))
            .expect("load the generic low-side NMOS netlist");
    let jfet = SystemManifest::from_file(root("examples/nucleo-f767zi/analog-jfet-load.yaml"))
        .expect("load the generic level-1 JFET netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);
    profile.cosim_models.extend(jfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA3", 3), ("PA4", 4)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, NMOS, and JFET devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all four generic netlists must resolve to distinct ADC1 inputs"
    );
    for channel in 0..=5 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    const NTC_V: f64 = 1.65;
    const NMOS_GATE_V: f64 = 2.0;
    let diode_v = diode_load_line_voltage();
    let mosfet_v = mosfet_drain_voltage(NMOS_GATE_V);
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent NTC temperature");
    session
        .set_signal_number("ui.mosfet.gate_v", NMOS_GATE_V)
        .expect("set independent NMOS gate voltage");

    let mut cycles = 0;
    let mut drain_samples = Vec::new();
    for gate_v in [0.0_f64, -0.5, -1.0, -1.5, -2.2] {
        session
            .set_signal_number("ui.jfet.gate_v", gate_v)
            .expect("set generic JFET gate voltage");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, NMOS, and JFET netlists");
        assert_eq!(steps.len(), 4, "all devices solve at the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let drain_v = signal_voltage(&session, "board.analog.pa4_volts");
        let expected_drain_v = jfet_drain_voltage(gate_v);
        assert!(
            (drain_v - expected_drain_v).abs() < 5e-5,
            "level-1 NJF load line at Vgs={gate_v} V should be {expected_drain_v} V, got {drain_v} V"
        );
        for (channel, volts) in [(4, drain_v), (0, NTC_V), (1, diode_v), (3, mosfet_v)] {
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
        for channel in [2, 5] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                0xFFFF,
                "neighboring ADC1 channel {channel} remains undriven"
            );
        }
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
