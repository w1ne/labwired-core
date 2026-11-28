// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the WCH CH32V003EVT RISC-V profiles into
//! its 10-bit ADC. The same two decks already proven on the STM32F1/F4 boards
//! (`cosim_nucleo_f103rb_analog`, `cosim_stm32f4_family_analog`) run here
//! unchanged: a Beta-parameter NTC divider and a Shockley diode load line name
//! only `board.analog.<pad>_volts`, and the CH32V003 descriptor's
//! `analog_pins:` map selects the ADC channel.
//!
//! Three profiles share the run: the bare CH32V003EVT, the same board with a
//! BME280 on I2C1, and the same board with a MAX31855 on SPI1. The BME280 owns
//! PC1/PC2, the MAX31855 owns PC4/PC5/PC6/PC7, and the LED jumper owns PC0;
//! the generic decks drive PA1/PA2, so no attached device disturbs the analog
//! front end and no engine code changes per board.
//!
//! This is the first mixed-signal board on a non-ARM core. The deck, the
//! netlist solver, and the routing layer are unchanged; only the descriptor
//! and the board profile differ. The conversion crosses the WCH register file
//! (ch32v00x.h: ADC1 at 0x40012400, RCC_APB2PCENR.ADC1EN bit 9, SWSTART bit
//! 22, RDATAR full scale 1023), so the test also proves the endpoint is a
//! second ADC width, not another 12-bit ST part.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::RiscV;
use labwired_core::system::riscv::configure_riscv;
use labwired_core::{Bus, Machine};
use std::path::Path;

/// ch32v00x.h: RCC_TypeDef base and APB2PCENR offset.
const RCC_BASE: u64 = 0x4002_1000;
/// ch32v00x.h: ADC1_BASE = APB2PERIPH_BASE + 0x2400.
const ADC1_BASE: u64 = 0x4001_2400;

/// ch32v00x_rcc.h: RCC_APB2PCENR.ADC1EN (bit 9).
const RCC_APB2PCENR_ADC1EN: u32 = 1 << 9;
/// ch32v00x.h: ADC_CTLR2.ADON (bit 0).
const WCH_CTLR2_ADON: u32 = 1 << 0;
/// ch32v00x.h: ADC_CTLR2.SWSTART (bit 22), the F1 layout's software trigger.
const WCH_CTLR2_SWSTART: u32 = 1 << 22;
/// ch32v00x.h: ADC_STATR.EOC (bit 1).
const WCH_STATR_EOC: u32 = 1 << 1;
/// ch32v00x.h: ADC_CTLR2.EXTSEL[2:0] = 0b111 selects the software trigger.
const WCH_CTLR2_EXTSEL_SW: u32 = 0b111 << 17;

/// Beta-parameter NTC resistance, the model card in the fixture.
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
    const THERMAL_V: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;
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

/// Wrap the board bus in the production `Machine`, parked on a RISC-V
/// branch-to-self guest, so a conversion completes through the same boundary
/// firmware uses on this core.
fn board_machine(mut bus: SystemBus) -> Machine<RiscV> {
    let spin_pc = 0x0800_0000;
    // `jal x0, 0` (C.J offset 0 encoded as JAL): the RV32 unconditional
    // branch to itself.
    bus.write_u32(spin_pc, 0x0000_006f)
        .expect("write jal x0, 0");
    let mut cpu = configure_riscv(&mut bus);
    cpu.pc = spin_pc as u32;
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine
}

/// Perform one WCH software-triggered regular conversion the way firmware
/// does: enable ADC1's APB2 clock, select the channel, ADON | EXTSEL=111,
/// then the SWSTART rising edge, then read STATR before RDATAR.
fn ch32v003_adc_convert(machine: &mut Machine<RiscV>, channel: u8) -> u32 {
    let bus = &mut machine.bus;
    let apb2pcenr = bus.read_u32(RCC_BASE + 0x18).expect("read RCC APB2PCENR");
    bus.write_u32(RCC_BASE + 0x18, apb2pcenr | RCC_APB2PCENR_ADC1EN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_BASE + 0x34, u32::from(channel))
        .expect("select RSQR3 SQ1 channel");
    bus.write_u32(ADC1_BASE + 0x08, WCH_CTLR2_ADON | WCH_CTLR2_EXTSEL_SW)
        .expect("power up ADC1 with the software trigger selected");
    bus.write_u32(
        ADC1_BASE + 0x08,
        WCH_CTLR2_ADON | WCH_CTLR2_EXTSEL_SW | WCH_CTLR2_SWSTART,
    )
    .expect("software-start the ADC1 conversion");
    for _ in 0..40 {
        machine.step().expect("advance the RV32EC guest one cycle");
    }
    let bus = &mut machine.bus;
    let status = bus.read_u32(ADC1_BASE).expect("read ADC1 STATR");
    assert_ne!(
        status & WCH_STATR_EOC,
        0,
        "ADC1 conversion completes with EOC set"
    );
    bus.read_u32(ADC1_BASE + 0x4c)
        .expect("read firmware-visible ADC1 RDATAR")
}

/// The conversion the WCH model reports. The router converts volts to `u16`
/// millivolts by rounding; the ADC floors the 12-bit-equivalent code and then
/// drops the two low bits for the 10-bit SAR, so replicate that order.
fn expected_adc_count(voltage_v: f64) -> u32 {
    let millivolts = (voltage_v * 1000.0).round();
    ((millivolts * 4095.0 / 3300.0).floor().clamp(0.0, 4095.0) as u32) >> 2
}

fn run_board(profile_name: &str, system: &str) {
    let system_path = root(system);
    let mut profile = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load {profile_name} profile: {e}"));
    assert_eq!(profile.name, profile_name);

    let analog = SystemManifest::from_file(root(
        "examples/wch-ch32v003evt/analog-ntc-diode-system.yaml",
    ))
    .unwrap_or_else(|e| panic!("load generic decks for {profile_name}: {e}"));
    assert!(
        analog.external_devices.is_empty(),
        "{profile_name}: the generic decks add no external device"
    );
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load {profile_name} descriptor: {e}"));
    assert_eq!(chip.analog_pins["PA2"].channel, 0);
    assert_eq!(chip.analog_pins["PA1"].channel, 1);
    assert_eq!(chip.analog_pins["PA2"].peripheral, "adc1");
    assert_eq!(chip.analog_pins["PA1"].peripheral, "adc1");

    let bus = SystemBus::from_config(&chip, &profile)
        .unwrap_or_else(|e| panic!("build {profile_name} bus: {e}"));
    let mut machine = board_machine(bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .unwrap_or_else(|e| panic!("build {profile_name} co-simulation: {e}"))
        .unwrap_or_else(|| panic!("{profile_name} deck carries both analog devices"));
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{profile_name} generic netlists must resolve through PA2/PA1 to ADC1 channels 0/1"
    );
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut samples = Vec::new();
    for temperature_c in [-20.0_f64, 25.0, 85.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut machine.bus)
            .expect("solve and route both analog netlists");
        assert_eq!(steps.len(), 2, "both netlists cross the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa2_volts");
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley diode load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let ntc_count = ch32v003_adc_convert(&mut machine, 0);
        assert_eq!(
            ntc_count,
            expected_adc_count(ntc_v),
            "firmware-visible ADC1 channel 0 must report the solved NTC voltage"
        );
        let diode_count = ch32v003_adc_convert(&mut machine, 1);
        assert_eq!(
            diode_count,
            expected_adc_count(diode_v),
            "firmware-visible ADC1 channel 1 must report the solved diode voltage"
        );
        // An unrouted channel reports the code after the previous RDATAR value,
        // masked to the 10-bit SAR, so this also proves the neighbour took no
        // routed input.
        assert_eq!(
            ch32v003_adc_convert(&mut machine, 2),
            (diode_count + 1) & 0x3ff,
            "the undriven neighbouring channel uses its fallback, not a routed input"
        );
        samples.push((temperature_c, ntc_v, ntc_count));
    }

    assert!(
        samples[2].1 > samples[1].1 && samples[1].1 > samples[0].1,
        "the divider voltage rises with NTC temperature: {samples:?}"
    );
    assert!(
        samples[2].2 > samples[1].2 && samples[1].2 > samples[0].2,
        "ADC counts track the solved temperature-dependent voltage: {samples:?}"
    );
    assert_eq!(
        samples[1].2, 511,
        "the 25 C NTC divider is half scale on the 3.3 V reference and 10-bit SAR"
    );
}

#[test]
fn ch32v003_ntc_and_diode_netlists_reach_pa2_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/wch-ch32v003evt.yaml");
    let profile = SystemManifest::from_file(&system_path).expect("load CH32V003EVT profile");
    assert_eq!(profile.name, "wch-ch32v003evt");
    assert!(profile.external_devices.is_empty());
    assert!(
        profile
            .board_io
            .iter()
            .any(|io| io.id == "led1_pc0" && io.peripheral == "gpioc" && io.pin == 0),
        "the mixed-signal run keeps the CH32V003EVT LED jumper"
    );
    run_board("wch-ch32v003evt", "configs/systems/wch-ch32v003evt.yaml");
}

/// The CH32V003EVT with an Adafruit MAX31855 K-type thermocouple breakout on
/// SPI1. The breakout owns PC4..PC7 (CS, SCK, DIN, DO) and the LED jumper owns
/// PC0; the generic decks drive PA1/PA2, which the SPI1 wiring leaves free, so
/// the attached converter does not disturb the analog front end.
#[test]
fn ch32v003_max31855_ntc_and_diode_netlists_reach_pa2_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/wch-ch32v003evt-max31855.yaml");
    let profile =
        SystemManifest::from_file(&system_path).expect("load CH32V003EVT-MAX31855 profile");
    assert_eq!(profile.name, "wch-ch32v003evt-max31855");
    assert!(
        profile
            .external_devices
            .iter()
            .any(|device| device.id == "tc1"),
        "the mixed-signal run keeps the MAX31855 on SPI1"
    );
    run_board(
        "wch-ch32v003evt-max31855",
        "configs/systems/wch-ch32v003evt-max31855.yaml",
    );
}

/// The CH32V003EVT with the BME280 breakout on I2C1. The sensor owns PC1/PC2
/// (SDA/SCL) and the LED jumper owns PC0; the generic decks drive PA1/PA2,
/// which the I2C wiring leaves free, so the attached sensor does not disturb
/// the analog front end.
#[test]
fn ch32v003_bme280_ntc_and_diode_netlists_reach_pa2_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/wch-ch32v003evt-bme280.yaml");
    let profile = SystemManifest::from_file(&system_path).expect("load CH32V003EVT-BME280 profile");
    assert_eq!(profile.name, "wch-ch32v003evt-bme280");
    assert!(
        profile
            .external_devices
            .iter()
            .any(|device| device.id == "bme280"),
        "the mixed-signal run keeps the BME280 on I2C1"
    );
    run_board(
        "wch-ch32v003evt-bme280",
        "configs/systems/wch-ch32v003evt-bme280.yaml",
    );
}
