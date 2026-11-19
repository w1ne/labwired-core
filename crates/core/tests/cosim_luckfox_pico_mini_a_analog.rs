// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the Luckfox Pico Mini A
//! SARADC (`rockchip_saradc_v2` at 0xff3c0000, Luckfox SDK rv1106.dtsi
//! `saradc@ff3c0000`). SARADC_IN0 (header pad 19) is channel 0 and
//! SARADC_IN1 (header pad 20) is channel 1; both pads are in the 1.8 V
//! domain, so the netlists run from 1.8 V.
//!
//! The RV1103 descriptor stays declared-only (CPU and memory). The example
//! system adds the converter with a system-level `peripherals:` entry, and
//! the netlists route by channel (`adc.saradc.<n>_volts`). The conversions
//! use the Linux v6.12 driver sequence (`rockchip_saradc_start_v2` and
//! `rockchip_saradc_read_v2`), whose `CONV_CON` write enable covers the
//! channel field. The vendor kernel's narrower write enable needs a
//! controller reset between channels; the last test shows both. This does
//! not model RV1103 GPIO, pinmux or the CRU, and the native Linux platform
//! does not instantiate the converter.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const SARADC: u64 = 0xff3c_0000;
const CONV_CON: u64 = SARADC;
const T_PD_SOC: u64 = SARADC + 0x004;
const T_DAS_SOC: u64 = SARADC + 0x00C;
const END_INT_EN: u64 = SARADC + 0x104;
const END_INT_ST: u64 = SARADC + 0x110;
const DATA_BASE: u64 = SARADC + 0x120;
const START: u32 = 1 << 4;
const SINGLE_MODE: u32 = 1 << 5;
const CHANNEL_MASK: u32 = 0xF;
const SUPPLY_V: f64 = 1.8;
/// `vref-supply = <&vcc_1v8>`.
const VREF_MV: u32 = 1800;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn ntc_volts(temperature_c: f64) -> f64 {
    let r = ntc_resistance_ohms(temperature_c);
    SUPPLY_V * r / (10_000.0 + r)
}

fn diode_load_line_voltage() -> f64 {
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

fn millivolts(volts: f64) -> u32 {
    (volts * 1000.0).round() as u32
}

/// 10 bits, `2^10 / VREF`, truncated, clamped (driver scale `vref / 2^10`).
fn code(volts: f64) -> u32 {
    (millivolts(volts) * 1024 / VREF_MV).min(1023)
}

fn analog_system() -> SystemManifest {
    SystemManifest::from_file(root(
        "examples/luckfox-pico-mini-a/analog-ntc-diode-system.yaml",
    ))
    .expect("load the Mini A SARADC system")
}

/// The Mini A analog system with the NTC and diode on the given channels.
fn board(ntc_channel: u8, diode_channel: u8) -> (SystemBus, CosimSession) {
    let mut system = analog_system();
    for model in &mut system.cosim_models {
        let (key, channel) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", ntc_channel),
            "diode_load_line" => ("v_diode", diode_channel),
            other => panic!("unexpected model {other}"),
        };
        model
            .outputs
            .insert(key.to_string(), format!("adc.saradc.{channel}_volts"));
    }
    let chip = ChipDescriptor::from_file(root("configs/chips/rv1103.yaml"))
        .expect("load RV1103 descriptor");
    assert!(
        chip.peripherals.is_empty(),
        "the descriptor stays declared-only; the system adds the SARADC"
    );
    let bus = SystemBus::from_config(&chip, &system).expect("build Mini A SystemBus");
    let session = CosimSession::new(&system.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both netlists must bind to SARADC channels"
    );
    (bus, session)
}

/// Move the NTC and solve both netlists to the next boundary.
fn settle(session: &mut CosimSession, bus: &mut SystemBus, cycles: &mut u64, temperature_c: f64) {
    session
        .set_signal_number("ui.ntc.temperature_c", temperature_c)
        .expect("set NTC temperature");
    *cycles += session.cycles_until_boundary(*cycles);
    let (steps, errors) = session
        .advance_to(*cycles, bus)
        .expect("solve and route both netlists");
    assert_eq!(steps.len(), 2, "both netlists solve each step");
    assert!(errors.is_empty(), "route errors: {errors:?}");
}

fn saradc_irq(bus: &SystemBus) -> bool {
    let saradc = bus
        .peripherals
        .iter()
        .find(|p| p.name == "saradc")
        .expect("saradc on the bus");
    saradc
        .dev
        .irq_line_level()
        .expect("rockchip_saradc_v2 reports its request")
}

/// Linux v6.12 `rockchip_saradc_start_v2`: the write enable is
/// `(START | SINGLE_MODE | CONV_CHANNELS) << 16`.
fn driver_start(bus: &mut SystemBus, channel: u32) {
    bus.write_u32(T_DAS_SOC, 0xC).expect("T_DAS_SOC");
    bus.write_u32(T_PD_SOC, 0x20).expect("T_PD_SOC");
    bus.write_u32(END_INT_EN, 1 << 16 | 1).expect("END_INT_EN");
    let val = START | SINGLE_MODE | channel;
    let enable = START | SINGLE_MODE | CHANNEL_MASK;
    bus.write_u32(CONV_CON, enable << 16 | val)
        .expect("CONV_CON");
}

/// Luckfox SDK kernel `rockchip_saradc_start_v2`: `val << 16 | val`.
fn vendor_start(bus: &mut SystemBus, channel: u32) {
    bus.write_u32(END_INT_EN, 1 << 16 | 1).expect("END_INT_EN");
    let val = START | SINGLE_MODE | channel;
    bus.write_u32(CONV_CON, val << 16 | val).expect("CONV_CON");
}

/// One driver read on `channel`: start, wait for the end event, then
/// `rockchip_saradc_read_v2` (clear the event, read `DATAn`, mask 10 bits).
fn read_raw(bus: &mut SystemBus, channel: u32) -> u32 {
    driver_start(bus, channel);
    assert!(!saradc_irq(bus), "no end event before the conversion");
    bus.tick_peripherals();
    assert!(saradc_irq(bus), "END_INT raises the request");
    bus.write_u32(END_INT_ST, 1).expect("clear irq");
    assert!(!saradc_irq(bus), "the driver's clear drops the request");
    bus.read_u32(DATA_BASE + 4 * u64::from(channel)).unwrap() & 0x3FF
}

#[test]
fn luckfox_pico_mini_a_saradc_converts_ntc_and_diode() {
    let (mut bus, mut session) = board(0, 1);
    assert_eq!(bus.read_u32(CONV_CON).unwrap(), 0, "CONV_CON reset");
    bus.tick_peripherals();
    assert_eq!(bus.read_u32(END_INT_ST).unwrap(), 0, "idle: no event");

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 50.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "adc.saradc.0_volts");
        assert!(
            (ntc_v - ntc_volts(temperature_c)).abs() < 1e-9,
            "at {temperature_c} °C the divider should be {} V, got {ntc_v} V",
            ntc_volts(temperature_c)
        );
        let diode_v = signal_voltage(&session, "adc.saradc.1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let ntc_code = read_raw(&mut bus, 0);
        assert_eq!(ntc_code, code(ntc_v), "SARADC_IN0 at {temperature_c} °C");
        assert!(ntc_code > 0 && ntc_code < 1023, "inside the 1.8 V range");
        let diode_code = read_raw(&mut bus, 1);
        assert_eq!(diode_code, code(diode_v), "SARADC_IN1");
        assert_eq!(
            bus.read_u32(DATA_BASE).unwrap(),
            ntc_code,
            "DATA0 keeps its sample while channel 1 converts"
        );
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes.windows(2).all(|pair| pair[0] > pair[1]),
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    // The 25 °C divider is half the rail: mid-scale.
    assert_eq!(ntc_codes[1], 512);
    // The driver scale is vref / 1024: within one 1.76 mV step of the node.
    let mv = ntc_codes[0] * VREF_MV / 1024;
    let true_mv = millivolts(ntc_volts(-10.0));
    assert!(
        mv <= true_mv && true_mv - mv <= 2,
        "{mv} mV vs {true_mv} mV"
    );
}

#[test]
fn luckfox_pico_mini_a_saradc_converts_after_the_netlists_swap() {
    let (mut bus, mut session) = board(1, 0);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_v = signal_voltage(&session, "adc.saradc.1_volts");
    let diode_v = signal_voltage(&session, "adc.saradc.0_volts");
    assert_eq!(read_raw(&mut bus, 1), code(ntc_v), "SARADC_IN1");
    assert_eq!(read_raw(&mut bus, 0), code(diode_v), "SARADC_IN0");
    assert_ne!(code(ntc_v), code(diode_v), "the swap is observable");
}

#[test]
fn luckfox_pico_mini_a_saradc_has_two_channels_only() {
    let mut system = analog_system();
    system.cosim_models[0]
        .outputs
        .insert("v_sense".to_string(), "adc.saradc.2_volts".to_string());
    let chip = ChipDescriptor::from_file(root("configs/chips/rv1103.yaml")).unwrap();
    let bus = SystemBus::from_config(&chip, &system).expect("build Mini A SystemBus");
    let session = CosimSession::new(&system.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors().len(),
        1,
        "RV1103 has SARADC_IN0 and SARADC_IN1 only: {:?}",
        session.binding_errors()
    );
}

#[test]
fn declared_only_mini_a_profile_refuses_the_adc_route() {
    let system_path = root("configs/systems/luckfox-pico-mini-a.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load Mini A profile");
    assert!(profile.peripherals.is_empty());
    profile.cosim_models = analog_system().cosim_models;
    let chip = ChipDescriptor::from_file(system_path.parent().unwrap().join(&profile.chip))
        .expect("load RV1103 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build Mini A SystemBus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors().len(),
        2,
        "without the SARADC both routes are errors, not guesses: {:?}",
        session.binding_errors()
    );
}

#[test]
fn luckfox_pico_mini_a_vendor_start_needs_the_saradc_reset_between_channels() {
    let (mut bus, mut session) = board(0, 1);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let ntc_code = code(signal_voltage(&session, "adc.saradc.0_volts"));
    let diode_code = code(signal_voltage(&session, "adc.saradc.1_volts"));

    vendor_start(&mut bus, 1);
    bus.tick_peripherals();
    bus.write_u32(END_INT_ST, 1).expect("clear irq");
    assert_eq!(bus.read_u32(DATA_BASE + 4).unwrap(), diode_code);

    // Without the reset the vendor write leaves channel 1 selected.
    vendor_start(&mut bus, 0);
    bus.tick_peripherals();
    bus.write_u32(END_INT_ST, 1).expect("clear irq");
    assert_eq!(bus.read_u32(CONV_CON).unwrap(), SINGLE_MODE | 1);
    assert_eq!(bus.read_u32(DATA_BASE).unwrap(), 0, "SARADC_IN0 not read");

    // `rockchip_saradc_reset_controller`, then the same start.
    let saradc = bus
        .peripherals
        .iter_mut()
        .find(|p| p.name == "saradc")
        .expect("saradc on the bus");
    saradc.dev.on_system_reset();
    vendor_start(&mut bus, 0);
    bus.tick_peripherals();
    assert!(saradc_irq(&bus));
    assert_eq!(bus.read_u32(DATA_BASE).unwrap(), ntc_code);
    assert_eq!(ntc_code, 512, "the 25 °C divider is half the rail");
}
