// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA netlists drive explicit U545 ADC1 channels and are converted
//! through ADC1's firmware-visible register interface, first on explicit ADC
//! channel routes, then on the PA0/PA1 pads that the chip's `analog_pins`
//! resolve to ADC1_IN5/IN6 (ST open pin data). Silicon behaviour is not
//! claimed.

mod common;
use common::root;
use labwired_config::SystemManifest;
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
const ADC_JSQR: u64 = ADC1_BASE + 0x4C;
const ADC_JDR1: u64 = ADC1_BASE + 0x80;
const ADC_ON: u32 = (1 << 28) | 1; // ADVREGEN | ADEN
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

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("U545 profile declares ADC1");
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

/// Independent level-1 JFET gate-junction depletion charge below FC * PB.
fn jfet_gate_junction_charge(voltage_v: f64) -> f64 {
    const C0_F: f64 = 1e-9;
    const PB_V: f64 = 1.0;
    const M: f64 = 0.5;
    C0_F * PB_V / (1.0 - M) * (1.0 - (1.0 - voltage_v / PB_V).powf(1.0 - M))
}

/// Independent backward-Euler update for two equal gate junctions tied to
/// fixed 3.3 V drain/source nodes through their depletion charges.
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
fn nucleo_u545re_ntc_and_diode_netlists_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-u545re.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-U545RE profile");
    assert_eq!(profile.name, "nucleo-u545re");
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "led_green_ld2" && io.peripheral == "gpioa" && io.pin == 5));
    assert!(profile
        .board_io
        .iter()
        .any(|io| { io.id == "button_user_pc13" && io.peripheral == "gpioc" && io.pin == 13 }));

    let analog =
        SystemManifest::from_file(root("examples/nucleo-u545re/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    let mut bus = SystemBus::from_config(
        &labwired_config::ChipDescriptor::from_file(
            system_path.parent().unwrap().join(&profile.chip),
        )
        .expect("load STM32U545 descriptor"),
        &profile,
    )
    .expect("build NUCLEO-U545RE system bus");

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
    bus.write_u32(ADC_PCSEL, 0b11)
        .expect("select both routed ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both generic analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models must bind to distinct explicit ADC1 channel routes"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 2),
        u16::MAX,
        "channel 2 is undriven"
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

        let ntc_v = signal_voltage(&session, "adc.adc1.0_volts");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "adc.adc1.1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        for (channel, voltage_v) in [(0, ntc_v), (1, diode_v)] {
            assert_eq!(
                u5_adc_convert(&mut bus, channel),
                expected_adc12_count(voltage_v),
                "ADC1 channel {channel} DR must match the routed {voltage_v} V"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 2),
            u16::MAX,
            "channel 2 stays undriven"
        );
    }
}

/// One software-started regular sequence (`SQR1.L` = channels - 1), read by
/// polling `EOC` and `DR`.
fn u5_adc_scan(bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
    const EOC: u32 = 1 << 2;
    const EOS: u32 = 1 << 3;
    let sqr1 = channels
        .iter()
        .enumerate()
        .fold(channels.len() as u32 - 1, |w, (k, &ch)| {
            w | (u32::from(ch) << (6 * (k + 1)))
        });
    bus.write_u32(ADC_SQR1, sqr1).expect("SQR1");
    bus.write_u32(ADC_ISR, EOC | EOS).expect("clear EOC/EOS");
    bus.write_u32(ADC_CR, ADC_ON | (1 << 2)).expect("ADSTART");
    channels
        .iter()
        .enumerate()
        .map(|(k, _)| {
            let isr = bus.read_u32(ADC_ISR).unwrap();
            assert_ne!(isr & EOC, 0, "EOC for conversion {k}");
            assert_eq!(isr & EOS != 0, k + 1 == channels.len(), "EOS at {k}");
            bus.write_u32(ADC_ISR, EOC).expect("clear EOC");
            bus.read_u32(ADC_DR).expect("DR")
        })
        .collect()
}

/// One `JADSTART` over `channels` (JSQ1 first) in the U5 `JSQR` packing:
/// `JL[1:0]`, `JEXTSEL[6:2]`, `JEXTEN[8:7]`, `JSQ1` at bit 9 and six bits per
/// rank (`stm32u575.svd` ADC1). Returns `JDR1..JDRn`.
fn u5_adc_injected(bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
    const JEOC_JEOS: u32 = (1 << 5) | (1 << 6);
    let jsqr = channels
        .iter()
        .enumerate()
        .fold(channels.len() as u32 - 1, |w, (k, &ch)| {
            w | (u32::from(ch) << (9 + 6 * k))
        });
    bus.write_u32(ADC_JSQR, jsqr).expect("JSQR");
    bus.write_u32(ADC_ISR, JEOC_JEOS).expect("clear JEOC/JEOS");
    bus.write_u32(ADC_CR, ADC_ON | (1 << 3)).expect("JADSTART");
    assert_eq!(bus.read_u32(ADC_ISR).unwrap() & JEOC_JEOS, JEOC_JEOS);
    assert_eq!(
        bus.read_u32(ADC_CR).unwrap() & (1 << 3),
        0,
        "JADSTART clears"
    );
    (0..channels.len() as u64)
        .map(|k| bus.read_u32(ADC_JDR1 + 4 * k).expect("JDRx"))
        .collect()
}

#[test]
fn nucleo_u545re_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-u545re.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-U545RE profile");
    let analog = SystemManifest::from_file(root(
        "examples/nucleo-u545re/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32U545 descriptor");
    for (pad, channel) in [("PA0", 5), ("PA1", 6)] {
        assert_eq!(
            chip.analog_pins
                .get(pad)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pad} is ADC1_IN{channel} (ST open pin data)"
        );
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-U545RE bus");

    let ahb2enr1 = bus.read_u32(RCC_AHB2ENR1).expect("read RCC AHB2ENR1");
    bus.write_u32(RCC_AHB2ENR1, ahb2enr1 | (1 << 10))
        .expect("enable ADC1 clock");
    bus.write_u32(ADC_CR, 0).expect("leave ADC deep power-down");
    bus.write_u32(ADC_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC_CR, (1 << 28) | (1 << 31))
        .expect("calibrate ADC1");
    bus.write_u32(ADC_CR, ADC_ON).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");
    bus.write_u32(ADC_CFGR1, 0x8000_0000 | (1 << 2))
        .expect("select 12-bit conversion resolution");
    bus.write_u32(ADC_PCSEL, (1 << 5) | (1 << 6))
        .expect("preselect the routed pad channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both generic analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models must resolve PA0/PA1 to ADC1 channels 5/6"
    );
    assert_eq!(adc_channel_count(&mut bus, 1), u16::MAX, "IN1 is undriven");

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
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

        let ntc = expected_adc12_count(ntc_v);
        let diode = expected_adc12_count(diode_v);
        assert_eq!(adc_channel_count(&mut bus, 5), ntc as u16, "PA0 seeds IN5");
        assert_eq!(
            adc_channel_count(&mut bus, 6),
            diode as u16,
            "PA1 seeds IN6"
        );
        assert_eq!(u5_adc_scan(&mut bus, &[5]), vec![ntc], "SQ1 = IN5");
        assert_eq!(u5_adc_scan(&mut bus, &[6]), vec![diode], "SQ1 = IN6");
        assert_eq!(
            u5_adc_scan(&mut bus, &[6, 5, 6]),
            vec![diode, ntc, diode],
            "one ADSTART scans the routed sensors in SQx order"
        );
        let dr = bus.read_u32(ADC_DR).unwrap();
        assert_eq!(
            u5_adc_injected(&mut bus, &[5, 6, 5, 6]),
            vec![ntc, diode, ntc, diode],
            "one JADSTART converts JSQ1..JSQ4 into JDR1..JDR4"
        );
        assert_eq!(bus.read_u32(ADC_DR).unwrap(), dr, "injected leaves DR");
        ntc_codes.push(ntc);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC code falls as temperature rises: {ntc_codes:?}"
    );
}

#[test]
fn nucleo_u545re_jfet_gate_depletion_charge_reaches_adc1_with_ntc_and_diode() {
    const STEP_S: f64 = 1e-6;
    const SUBSTEPS: usize = 10;

    let system_path = root("configs/systems/nucleo-u545re.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-U545RE profile");
    let base = SystemManifest::from_file(root(
        "examples/nucleo-u545re/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let jfet = SystemManifest::from_file(root(
        "examples/nucleo-u545re/analog-jfet-gate-charge-adc-system.yaml",
    ))
    .expect("load matched generic JFET gate-charge branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(jfet.cosim_models);

    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32U545 descriptor");
    for (pad, channel) in [("PA0", 5), ("PA1", 6), ("PA2", 7), ("PA3", 8)] {
        assert_eq!(
            chip.analog_pins
                .get(pad)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pad} must resolve to U545 ADC1_IN{channel}"
        );
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-U545RE bus");
    let ahb2enr1 = bus.read_u32(RCC_AHB2ENR1).expect("read RCC AHB2ENR1");
    bus.write_u32(RCC_AHB2ENR1, ahb2enr1 | (1 << 10))
        .expect("enable ADC1 clock");
    bus.write_u32(ADC_CR, 0).expect("leave ADC deep power-down");
    bus.write_u32(ADC_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC_CR, (1 << 28) | (1 << 31))
        .expect("calibrate ADC1");
    bus.write_u32(ADC_CR, ADC_ON).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");
    bus.write_u32(ADC_CFGR1, 0x8000_0000 | (1 << 2))
        .expect("select 12-bit conversion resolution");
    bus.write_u32(ADC_PCSEL, (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8))
        .expect("preselect all four routed ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries JFET, NTC, and diode netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve PA0..PA3 to ADC1 channels 5..8"
    );
    for channel in 5..=8 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts undriven"
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
    let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
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

        for (channel, voltage_v) in [(5, ntc_v), (6, diode_v), (7, gate_v), (8, control_v)] {
            let expected = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected as u16,
                "the solved voltage seeds ADC1_IN{channel}"
            );
            assert_eq!(
                u5_adc_scan(&mut bus, &[channel]),
                vec![expected],
                "firmware-visible ADC1_IN{channel} conversion reports the routed voltage"
            );
        }
    }
    assert!(
        signal_voltage(&session, "board.analog.pa2_volts") < 1.0,
        "gate depletion capacitance delays the edge but eventually follows the low drive"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 9),
        u16::MAX,
        "neighboring ADC1_IN9 remains undriven"
    );
}
