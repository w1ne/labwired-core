// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog devices route through SAM D21 pads into firmware-visible ADC results.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::sam::adc21::Samd21Adc;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Bus, Cpu, Machine};
use std::path::Path;

const ADC_BASE: u64 = 0x4200_4000;
const CTRLA: u64 = ADC_BASE;
const REFCTRL: u64 = ADC_BASE + 1;
const CTRLB: u64 = ADC_BASE + 4;
const SWTRIG: u64 = ADC_BASE + 0x0c;
const INPUTCTRL: u64 = ADC_BASE + 0x10;
const INTFLAG: u64 = ADC_BASE + 0x18;
const RESULT: u64 = ADC_BASE + 0x1a;
const PORT_A: u64 = 0x4100_4400;
const PORT_B: u64 = 0x4100_4480;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

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

/// Independent Gummel–Poon load-line reference with optional base-emitter recombination.
fn bjt_collector_voltage_with_recombination(base_drive_v: f64, recombination: bool) -> f64 {
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

fn signal_number(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(value)) => *value,
        other => panic!("expected analog voltage at {path}, got {other:?}"),
    }
}

fn advance_to_analog_boundary<C: Cpu>(machine: &mut Machine<C>, session: &mut CosimSession) {
    for _ in 0..256 {
        let advance = session
            .advance(machine, AdvanceRequest::single())
            .expect("advance board and analog devices");
        assert!(
            advance.new_routing_errors.is_empty(),
            "unexpected route errors: {:?}",
            advance.new_routing_errors
        );
        if !advance.routed.is_empty() {
            return;
        }
    }
    panic!("analog devices did not reach a co-simulation boundary");
}

fn convert<C: Cpu>(
    machine: &mut Machine<C>,
    session: &mut CosimSession,
    channel: u8,
    resolution_select: u16,
) -> u16 {
    machine
        .bus
        .write_u32(INPUTCTRL, 0x1800 | u32::from(channel))
        .expect("select one external AIN against GND");
    machine
        .bus
        .write_u16(CTRLB, resolution_select)
        .expect("select right-adjusted ADC resolution");
    machine
        .bus
        .write_u8(SWTRIG, 1 << 1)
        .expect("start software conversion");

    for _ in 0..256 {
        if machine.bus.read_u8(INTFLAG).expect("read INTFLAG") & 1 != 0 {
            return machine.bus.read_u16(RESULT).expect("read RESULT");
        }
        let advance = session
            .advance(machine, AdvanceRequest::single())
            .expect("advance pending ADC conversion");
        assert!(
            advance.new_routing_errors.is_empty(),
            "unexpected ADC routing errors: {:?}",
            advance.new_routing_errors
        );
    }
    panic!("SAMD21 ADC conversion did not complete");
}

fn expected_code(voltage_v: f64, bits: u32, reference_mv: u32) -> u16 {
    let millivolts = (voltage_v * 1000.0 + 0.5).floor() as u32;
    ((millivolts * ((1 << bits) - 1) / reference_mv).min((1 << bits) - 1)) as u16
}

fn enable_pad_b<C: Cpu>(machine: &mut Machine<C>, port: u64, pin: u8) {
    machine
        .bus
        .write_u8(port + 0x30 + u64::from(pin / 2), 1 << (4 * (pin & 1)))
        .expect("select PORT peripheral function B");
    machine
        .bus
        .write_u8(port + 0x40 + u64::from(pin), 1)
        .expect("enable peripheral mux on ADC input pad");
}

fn convert_on_next_tick(bus: &mut SystemBus, channel: u8) -> u16 {
    bus.write_u32(INPUTCTRL, 0x1800 | u32::from(channel))
        .expect("select one external AIN against GND");
    bus.write_u16(CTRLB, 0)
        .expect("select 12-bit ADC resolution");
    bus.write_u8(SWTRIG, 1 << 1)
        .expect("start software conversion");
    Bus::tick_peripherals(bus);
    assert_ne!(bus.read_u8(INTFLAG).expect("read INTFLAG") & 1, 0);
    bus.read_u16(RESULT).expect("read RESULT")
}

fn enable_adc_pad_b(bus: &mut SystemBus, port: u64, pin: u8) {
    let pmux_address = port + 0x30 + u64::from(pin / 2);
    let shift = 4 * u32::from(pin & 1);
    let pmux = bus.read_u8(pmux_address).expect("read PORT PMUX") as u32;
    let updated_pmux = (pmux & !(0x0f << shift)) | (1 << shift);
    bus.write_u8(pmux_address, updated_pmux as u8)
        .expect("select PORT peripheral function B");
    bus.write_u8(port + 0x40 + u64::from(pin), 1)
        .expect("enable peripheral mux on ADC input pad");
}

#[test]
fn arduino_zero_ntc_and_diode_netlists_reach_distinct_samd21_adc_pads() {
    let system_path = root("configs/systems/arduino-zero.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Arduino Zero profile");
    assert_eq!(profile.name, "arduino-zero");
    let analog =
        SystemManifest::from_file(root("examples/arduino-zero/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC and diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ATSAMD21G18A descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Arduino Zero bus");
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    const IDLE: u32 = 0x2000_2000;
    machine
        .bus
        .write_u16(u64::from(IDLE), 0xE7FE)
        .expect("place branch-to-self instruction in SRAM");
    machine.cpu.set_pc(IDLE);
    machine.cpu.set_sp(0x2000_7f00);

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build mixed-signal session")
        .expect("profile contains generic analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "PA02/AIN0 and PB08/AIN2 must resolve through the chip analog pin map"
    );

    machine
        .bus
        .write_u8(REFCTRL, 1)
        .expect("select INTVCC0 reference");
    machine.bus.write_u8(CTRLA, 1 << 1).expect("enable ADC");

    let mut samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature input");
        advance_to_analog_boundary(&mut machine, &mut session);

        let ntc_v = signal_number(&session, "board.analog.pa02_volts");
        let expected_ntc_v = 3.3 * 2_200.0 / (ntc_resistance_ohms(temperature_c) + 2_200.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the Beta NTC divider at {temperature_c} °C should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_number(&session, "board.analog.pb08_volts");
        let expected_diode_v = diode_load_line_voltage();
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the independent Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let adc_index = machine
            .bus
            .find_peripheral_index_by_name("adc")
            .expect("SAMD21 ADC is present");
        let adc = machine.bus.peripherals[adc_index]
            .dev
            .as_any()
            .expect("ADC downcasts")
            .downcast_ref::<Samd21Adc>()
            .expect("the board ADC uses Samd21Adc");
        assert_eq!(
            adc.channel_input_mv(0),
            Some((ntc_v * 1000.0 + 0.5).floor() as u16),
            "the NTC netlist must seed its AIN0 input"
        );
        assert_eq!(
            adc.channel_input_mv(2),
            Some((diode_v * 1000.0 + 0.5).floor() as u16),
            "the unrelated diode netlist must seed its AIN2 input"
        );
        assert_eq!(
            adc.channel_input_mv(3),
            None,
            "neighboring AIN3 is undriven"
        );

        // The circuit has solved and seeded AIN0, but its voltage is not
        // observable while the PA02 pad remains in GPIO mode.
        if samples.is_empty() {
            assert_eq!(
                convert(&mut machine, &mut session, 0, 0),
                0,
                "an unmuxed PA02 pad must not reach the ADC"
            );
        }

        // Correct device input values become visible once the actual pads
        // are muxed to ADC function B and PMUXEN is enabled.
        enable_pad_b(&mut machine, PORT_A, 2); // PA02 / AIN0
        enable_pad_b(&mut machine, PORT_B, 8); // PB08 / AIN2

        for (resolution_select, bits) in [(0, 12), (2 << 4, 10), (3 << 4, 8)] {
            let ntc_code = convert(&mut machine, &mut session, 0, resolution_select);
            let diode_code = convert(&mut machine, &mut session, 2, resolution_select);
            let undriven_code = convert(&mut machine, &mut session, 3, resolution_select);
            let reference_mv = 3300 * 100 / 148;
            assert_eq!(
                ntc_code,
                expected_code(ntc_v, bits, reference_mv),
                "{bits}-bit AIN0 code must follow the solved NTC voltage"
            );
            assert_eq!(
                diode_code,
                expected_code(diode_v, bits, reference_mv),
                "{bits}-bit AIN2 code must follow the independent diode load line"
            );
            assert_eq!(undriven_code, 0, "unwired AIN3 converts to zero");
            samples.push((temperature_c, ntc_v, ntc_code));
        }
    }

    assert!(
        samples[6].1 > samples[3].1 && samples[3].1 > samples[0].1,
        "the NTC divider voltage must rise as temperature rises: {samples:?}"
    );
    assert!(
        samples[6].2 > samples[3].2 && samples[3].2 > samples[0].2,
        "the ADC result must track the temperature-dependent voltage: {samples:?}"
    );
}

#[test]
fn arduino_zero_bjt_junction_and_transit_charge_reach_samd21_adc() {
    let system_path = root("configs/systems/arduino-zero.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Arduino Zero profile");
    let generic =
        SystemManifest::from_file(root("examples/arduino-zero/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let charge =
        SystemManifest::from_file(root("examples/arduino-zero/analog-bjt-charge-system.yaml"))
            .expect("load matched BJT charge comparison netlists");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(charge.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ATSAMD21G18A descriptor");
    for (pad, channel) in [
        ("PA02", 0),
        ("PA03", 1),
        ("PB08", 2),
        ("PB09", 3),
        ("PA04", 4),
        ("PA05", 5),
        ("PA06", 6),
    ] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Arduino Zero bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and BJT charge netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to SAMD21 ADC channels 0 through 6"
    );

    bus.write_u8(REFCTRL, 1).expect("select INTVCC0 reference");
    bus.write_u8(CTRLA, 1 << 1).expect("enable ADC");
    for (port, pin) in [
        (PORT_A, 2),
        (PORT_A, 3),
        (PORT_B, 8),
        (PORT_B, 9),
        (PORT_A, 4),
        (PORT_A, 5),
        (PORT_A, 6),
    ] {
        enable_adc_pad_b(&mut bus, port, pin);
    }

    let adc_index = bus
        .find_peripheral_index_by_name("adc")
        .expect("SAMD21 ADC is present");
    let adc = bus.peripherals[adc_index]
        .dev
        .as_any()
        .expect("ADC downcasts")
        .downcast_ref::<Samd21Adc>()
        .expect("the board ADC uses Samd21Adc");
    for channel in 0..=7 {
        assert_eq!(
            adc.channel_input_mv(channel),
            None,
            "AIN{channel} starts undriven"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");
    session
        .set_signal_number("ui.bjt_charge.base_v", 0.0)
        .expect("initialize all BJT base-drive sources");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize the independent branches before the base-drive step");
    assert_eq!(initial_steps.len(), 7, "all seven analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );

    session
        .set_signal_number("ui.bjt_charge.base_v", 0.8)
        .expect("step the BJT base-drive sources");
    let expected_ntc_v = 3.3 * 2_200.0 / (ntc_resistance_ohms(25.0) + 2_200.0);
    let expected_diode_v = diode_load_line_voltage();
    let expected_dc = bjt_collector_voltage(0.8);
    let mut samples = Vec::new();
    for step in 0..8_000 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and BJT charge branches");
        assert_eq!(
            steps.len(),
            7,
            "all seven branches solve at every 1 us step"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let voltages = [
            signal_number(&session, "board.analog.pa02_volts"),
            signal_number(&session, "board.analog.pa03_volts"),
            signal_number(&session, "board.analog.pb08_volts"),
            signal_number(&session, "board.analog.pb09_volts"),
            signal_number(&session, "board.analog.pa04_volts"),
            signal_number(&session, "board.analog.pa05_volts"),
            signal_number(&session, "board.analog.pa06_volts"),
        ];
        assert!(
            (voltages[0] - expected_ntc_v).abs() < 1e-9,
            "the unrelated NTC remains at its independent divider voltage"
        );
        assert!(
            (voltages[2] - expected_diode_v).abs() < 2e-5,
            "the unrelated diode remains at its independent load line"
        );
        assert!(
            (voltages[1] - expected_dc).abs() < 5e-5,
            "the uncharged BJT must match the independent Gummel–Poon load line {expected_dc} V, got {} V",
            voltages[1]
        );

        if step == 0 || step == 7_999 {
            let reference_mv = 3300 * 100 / 148;
            for (channel, voltage) in voltages.iter().copied().enumerate() {
                let channel = channel as u8;
                let expected_input = (voltage * 1000.0 + 0.5).floor() as u16;
                let adc = bus.peripherals[adc_index]
                    .dev
                    .as_any()
                    .expect("ADC downcasts")
                    .downcast_ref::<Samd21Adc>()
                    .expect("the board ADC uses Samd21Adc");
                assert_eq!(
                    adc.channel_input_mv(channel),
                    Some(expected_input),
                    "the solved voltage must seed SAMD21 AIN{channel}"
                );
                assert_eq!(
                    convert_on_next_tick(&mut bus, channel),
                    expected_code(voltage, 12, reference_mv),
                    "firmware-visible 12-bit AIN{channel} conversion must report {voltage} V"
                );
            }
            assert_eq!(
                convert_on_next_tick(&mut bus, 7),
                0,
                "the neighboring unrouted AIN7 remains at zero"
            );
        }

        if step == 0 {
            for (label, voltage) in [
                ("CJE depletion", voltages[3]),
                ("CJC depletion", voltages[4]),
                ("TF transit", voltages[5]),
                ("combined junction and transit", voltages[6]),
            ] {
                assert!(
                    (voltage - voltages[1]).abs() > 0.01,
                    "{label} must change the first routed transient: reference={} V, charged={voltage} V",
                    voltages[1]
                );
            }
        }
        samples.push([
            voltages[1],
            voltages[3],
            voltages[4],
            voltages[5],
            voltages[6],
        ]);
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

#[test]
fn arduino_zero_bjt_base_recombination_reaches_samd21_adc_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/arduino-zero.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Arduino Zero profile");
    let generic =
        SystemManifest::from_file(root("examples/arduino-zero/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let bjt = SystemManifest::from_file(root(
        "examples/arduino-zero/analog-bjt-recombination-system.yaml",
    ))
    .expect("load matched generic BJT recombination branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ATSAMD21G18A descriptor");
    for (pad, channel) in [("PA02", 0), ("PA03", 1), ("PB08", 2), ("PB09", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build Arduino Zero bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal session")
        .expect("profile contains NTC, diode, and matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to SAMD21 ADC channels 0 through 3"
    );

    bus.write_u8(REFCTRL, 1).expect("select INTVCC0 reference");
    bus.write_u8(CTRLA, 1 << 1).expect("enable ADC");
    for (port, pin) in [(PORT_A, 2), (PORT_A, 3), (PORT_B, 8), (PORT_B, 9)] {
        enable_adc_pad_b(&mut bus, port, pin);
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");
    session
        .set_signal_number("ui.bjt_recombination.base_v", 0.0)
        .expect("initialize both BJT base-drive sources");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize all four independent branches");
    assert_eq!(
        initial_steps.len(),
        4,
        "NTC, diode, and both BJT branches solve"
    );
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );

    let expected_ntc_v = 3.3 * 2_200.0 / (ntc_resistance_ohms(25.0) + 2_200.0);
    let expected_diode_v = diode_load_line_voltage();
    let adc_index = bus
        .find_peripheral_index_by_name("adc")
        .expect("SAMD21 ADC is present");
    let mut recombination_deltas = Vec::new();
    for base_drive_v in [0.70_f64, 0.78, 0.86] {
        session
            .set_signal_number("ui.bjt_recombination.base_v", base_drive_v)
            .expect("set matched BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and BJT recombination branches");
        assert_eq!(steps.len(), 4, "all four branches solve at each drive");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let voltages = [
            signal_number(&session, "board.analog.pa02_volts"),
            signal_number(&session, "board.analog.pa03_volts"),
            signal_number(&session, "board.analog.pb08_volts"),
            signal_number(&session, "board.analog.pb09_volts"),
        ];
        assert!((voltages[0] - expected_ntc_v).abs() < 1e-9);
        assert!((voltages[2] - expected_diode_v).abs() < 2e-5);

        let expected_without = bjt_collector_voltage_with_recombination(base_drive_v, false);
        let expected_with = bjt_collector_voltage_with_recombination(base_drive_v, true);
        assert!(
            (voltages[1] - expected_without).abs() < 5e-5,
            "the reference BJT must match the independent load line {expected_without} V, got {} V",
            voltages[1]
        );
        assert!(
            (voltages[3] - expected_with).abs() < 5e-5,
            "the recombination BJT must match the independent load line {expected_with} V, got {} V",
            voltages[3]
        );
        recombination_deltas.push((voltages[3] - voltages[1]).abs());

        for (channel, voltage) in voltages.iter().copied().enumerate() {
            let channel = channel as u8;
            let expected_input = (voltage * 1000.0 + 0.5).floor() as u16;
            let adc = bus.peripherals[adc_index]
                .dev
                .as_any()
                .expect("ADC downcasts")
                .downcast_ref::<Samd21Adc>()
                .expect("the board ADC uses Samd21Adc");
            assert_eq!(
                adc.channel_input_mv(channel),
                Some(expected_input),
                "solved voltage must seed SAMD21 AIN{channel}"
            );
            assert_eq!(
                convert_on_next_tick(&mut bus, channel),
                expected_code(voltage, 12, 3300 * 100 / 148),
                "firmware-visible 12-bit AIN{channel} conversion must report {voltage} V"
            );
        }
        assert_eq!(
            convert_on_next_tick(&mut bus, 4),
            0,
            "unrouted AIN4 remains zero"
        );
    }
    assert!(
        recombination_deltas.iter().any(|delta| *delta > 0.02),
        "ISE/NE base recombination must measurably change the collector load line: {recombination_deltas:?}"
    );
}
