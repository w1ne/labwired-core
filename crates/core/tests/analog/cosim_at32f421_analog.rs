// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the Artery AT32F421G8U7 profile into
//! ADC1. The same two decks already proven on the NUCLEO-F103RB
//! (`cosim_nucleo_f103rb_analog`) run here unchanged: a Beta-parameter NTC
//! divider and a Shockley diode load line name only `board.analog.<pad>_volts`,
//! and the AT32F421 descriptor's `analog_pins:` map selects the ADC channel.
//!
//! The AT32F421 ADC1 block is the STM32F1 register map bit for bit
//! (RM_AT32F421 V2.05 §18.5: ODT at 0x4C, CTRL2 OCSWTRG bit 22, OCTESEL
//! 19:17), so this test drives the same conversion sequence as the F1 board.
//! It proves the board route, not an Artery-specific ADC feature: the shared
//! F1 model does not gate a conversion on calibration or settling time.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::Path;

const RCC_BASE: u64 = 0x4002_1000;
const ADC1_BASE: u64 = 0x4001_2400;

/// RM_AT32F421 V2.05 §7.3: CRM_APB2EN.ADC1EN (bit 9).
const RCC_APB2ENR_ADC1EN: u32 = 1 << 9;
/// RM_AT32F421 §18.5: ADC_CTRL2.ADCON (bit 0), on the F1 CR2 layout.
const F1_CR2_ADON: u32 = 1 << 0;
/// RM_AT32F421 §18.5: OCTESEL[2:0] = 0b111 selects the software trigger.
const F1_CR2_EXTSEL_SW: u32 = 0b111 << 17;
/// RM_AT32F421 §18.5: ADC_CTRL2.OCSWTRG (bit 22), the F1 SWSTART bit.
const F1_CR2_SWSTART: u32 = 1 << 22;
/// RM_AT32F421 §18.5: ADC_STS.EOC (bit 1).
const F1_SR_EOC: u32 = 1 << 1;

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

/// Wrap the board bus in the production `Machine`, parked on a branch-to-self
/// guest, so a conversion completes through the same boundary firmware uses.
fn board_machine(mut bus: SystemBus) -> Machine<CortexM> {
    // Park the guest in SRAM: the AT32F421 flash controller rejects a bare
    // store without an unlocked FPRGM program operation, so the branch-to-self
    // cannot live in flash the way the F1 board allows.
    let spin_pc = 0x2000_0100;
    bus.write_u16(spin_pc, 0xe7fe).expect("write b ."); // b .
    let (mut cpu, _) = configure_cortex_m(&mut bus);
    cpu.pc = spin_pc as u32;
    cpu.sp = 0x2000_4000;
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine
}

/// Perform one AT32F421 software-triggered regular conversion the way firmware
/// does: ADCON | OCTESEL=111, then the OCSWTRG rising edge, then read STS
/// before ODT.
fn at32f421_adc_convert(machine: &mut Machine<CortexM>, channel: u8) -> u32 {
    let bus = &mut machine.bus;
    let apb2enr = bus.read_u32(RCC_BASE + 0x18).expect("read CRM APB2EN");
    bus.write_u32(RCC_BASE + 0x18, apb2enr | RCC_APB2ENR_ADC1EN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_BASE + 0x34, u32::from(channel))
        .expect("select OSQ3 SQ1 channel");
    bus.write_u32(ADC1_BASE + 0x08, F1_CR2_ADON | F1_CR2_EXTSEL_SW)
        .expect("power up ADC1 with the software trigger selected");
    bus.write_u32(
        ADC1_BASE + 0x08,
        F1_CR2_ADON | F1_CR2_EXTSEL_SW | F1_CR2_SWSTART,
    )
    .expect("software-start the ADC1 conversion");
    for _ in 0..40 {
        machine.run(Some(1)).expect("advance the guest one cycle");
    }
    let bus = &mut machine.bus;
    let status = bus.read_u32(ADC1_BASE).expect("read ADC1 STS");
    assert_ne!(
        status & F1_SR_EOC,
        0,
        "ADC1 conversion completes with EOC set"
    );
    bus.read_u32(ADC1_BASE + 0x4c)
        .expect("read firmware-visible ADC1 ODT")
}

/// The conversion the model reports: the routed pad voltage in whole
/// millivolts, scaled to the 12-bit range against the 3.3 V reference.
///
/// The router converts volts to `u16` millivolts by rounding, and the ADC
/// floors the scaled code, so the same rounding has to happen here; a value
/// within a millivolt of a code boundary would otherwise read one code low.
fn expected_adc_count(voltage_v: f64) -> u32 {
    let millivolts = (voltage_v * 1000.0).round();
    (millivolts * 4095.0 / 3300.0).floor().clamp(0.0, 4095.0) as u32
}

#[test]
fn at32f421_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/at32f421g8u7.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load AT32F421G8U7 profile");
    assert_eq!(profile.name, "at32f421g8u7");
    assert!(profile.external_devices.is_empty());

    let analog =
        SystemManifest::from_file(root("examples/at32f421g8u7/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load AT32F421G8U7 descriptor");
    assert_eq!(chip.analog_pins["PA0"].channel, 0);
    assert_eq!(chip.analog_pins["PA1"].channel, 1);
    assert_eq!(chip.analog_pins["PA0"].peripheral, "adc1");
    assert_eq!(chip.analog_pins["PA1"].peripheral, "adc1");

    let bus = SystemBus::from_config(&chip, &profile).expect("build AT32F421G8U7 bus");
    let mut machine = board_machine(bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through PA0/PA1 to ADC1 channels 0/1"
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

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
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

        let ntc_count = at32f421_adc_convert(&mut machine, 0);
        assert_eq!(
            ntc_count,
            expected_adc_count(ntc_v),
            "firmware-visible ADC1 channel 0 must report the solved NTC voltage"
        );
        let diode_count = at32f421_adc_convert(&mut machine, 1);
        assert_eq!(
            diode_count,
            expected_adc_count(diode_v),
            "firmware-visible ADC1 channel 1 must report the solved diode voltage"
        );
        // An unrouted channel reports the code after the previous ODT value, so
        // this assertion also proves the neighbour did not take a routed input.
        assert_eq!(
            at32f421_adc_convert(&mut machine, 2),
            (diode_count + 1) & 0xfff,
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
        samples[1].2, 2047,
        "the 25 C NTC divider is half scale on the 3.3 V reference"
    );
}
