// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the ARDUINO ZERO and ARDUINO NANO 33 IoT
//! (ATSAMD21G18A) profiles into the part's `samd21_adc` converter. The same
//! decks already proven on the STM32, WCH, nRF52, micro:bit, AVR, Renesas RA
//! and RP2040 boards run here unchanged: a Beta-parameter NTC divider, a
//! Shockley diode load line and a plain resistor divider name only
//! `board.analog.<pad>_volts`, and the ATSAMD21G18A descriptor's new
//! `analog_pins:` map selects AIN0 (PA2), AIN4 (PA4) and AIN6 (PA6).
//!
//! This is the first Microchip SAM D21 board in the matrix, and the first part
//! whose converter demands the pad really be handed to the ADC: `samd21_adc`
//! reads `PORT.PINCFG.PMUXEN` and the PMUX function nibble at conversion time,
//! exactly as `pinPeripheral(pin, PIO_ANALOG)` configures them before
//! `analogRead()`. The test therefore proves the pad map AND the port mux, not
//! just the voltage arithmetic.
//!
//! Nothing in the deck, solver, router or converter changed: only the chip
//! descriptor gained the pad map, so the test proves the mechanism is
//! per-part data. The channel numbers are the CMSIS `PIN_PxyyB_ADC_AINn`
//! macros the model already converts against; the test asserts the map against
//! those numbers rather than hardcoding a second copy.
//!
//! The Nano 33 IoT profile is the same part with a BME280 on SERCOM4
//! (PB8/PB9). The decks are kept off those pads so the sensor and the analog
//! front end can run together.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::Path;

/// ADC (`samd21_adc`) base and register offsets, CMSIS `component/adc.h`.
const ADC_BASE: u64 = 0x4200_4000;
const ADC_CTRLA: u64 = ADC_BASE;
const ADC_REFCTRL: u64 = ADC_BASE + 0x01;
const ADC_CTRLB: u64 = ADC_BASE + 0x04;
const ADC_SWTRIG: u64 = ADC_BASE + 0x0C;
const ADC_INPUTCTRL: u64 = ADC_BASE + 0x10;
const ADC_INTFLAG: u64 = ADC_BASE + 0x18;
const ADC_RESULT: u64 = ADC_BASE + 0x1A;

/// `CTRLA.ENABLE`.
const CTRLA_ENABLE: u8 = 1 << 1;
/// `REFCTRL.REFSEL = INTVCC1` (VDDANA / 2), Arduino `AR_DEFAULT`.
const REFSEL_INTVCC1: u8 = 2;
/// `INPUTCTRL.MUXNEG = GND`.
const MUXNEG_GND: u32 = 0x18 << 8;
/// `INPUTCTRL.GAIN = DIV2`, Arduino `AR_DEFAULT`.
const GAIN_DIV2: u32 = 0xF << 24;
/// `SWTRIG.START`.
const SWTRIG_START: u8 = 1 << 1;
/// `INTFLAG.RESRDY`.
const INTFLAG_RESRDY: u8 = 1;

/// PM.APBCMASK; the ADC is bit 16 (CMSIS `PM_APBCMASK_ADC_Pos`).
const PM_APBCMASK: u64 = 0x4000_0420;
const APBC_ADC: u32 = 1 << 16;

/// PORT groups; group B follows group A at +0x80. PMUX at +0x30, PINCFG at +0x40.
const PORT_A: u64 = 0x4100_4400;
const PORT_B: u64 = 0x4100_4480;
const PMUX_OFFSET: u64 = 0x30;
const PINCFG_OFFSET: u64 = 0x40;
/// PMUX function B is the ADC on this family (`PIO_ANALOG`).
const PMUX_FUNCTION_B: u8 = 1;

/// VDDANA on the Arduino Zero and Nano 33 IoT, in millivolts.
const VDDANA_MV: u64 = 3_300;
/// 12-bit full-scale count.
const FULL_SCALE: u64 = 1 << 12;

/// Parked `b .` guest in SRAM. The part executes from SRAM in the simulator,
/// so a conversion can be advanced through the production `Machine` without a
/// firmware image.
const SPIN_PC: u32 = 0x2000_0100;
const STACK_TOP: u32 = 0x2000_4000;

/// Beta-parameter NTC resistance, the model card in the fixture.
fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line on the deck's 3.3 V rail, including the
/// diode's series resistance.
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

/// The count the model reports: the routed level in whole millivolts, DIV2
/// gain against INTVCC1 (VDDANA / 2), so the full scale is VDDANA.
fn expected_code(volts: f64) -> u16 {
    let millivolts = (volts * 1000.0).round() as u64;
    ((millivolts * FULL_SCALE / VDDANA_MV).min(FULL_SCALE - 1)) as u16
}

/// A bus built from the profile, plus a Cortex-M0+ parked on `b .` so the
/// peripheral bus-tick phase runs through the production machine.
fn board_machine(chip: &ChipDescriptor, manifest: &SystemManifest) -> Machine<CortexM> {
    let mut bus = SystemBus::from_config(chip, manifest).expect("build the board bus");
    bus.write_u16(u64::from(SPIN_PC), 0xe7fe)
        .expect("write b .");
    let (mut cpu, _nvic) = configure_cortex_m(&mut bus);
    cpu.pc = SPIN_PC;
    cpu.sp = STACK_TOP;
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine
}

/// `pinPeripheral(pin, PIO_ANALOG)`: hand the pad to the ADC (function B).
fn route_pad_to_adc(machine: &mut Machine<CortexM>, port: u64, pin: u8) {
    let bus = &mut machine.bus;
    let pincfg = port + PINCFG_OFFSET + u64::from(pin);
    let pmux = port + PMUX_OFFSET + u64::from(pin / 2);
    let current = bus.read_u8(pmux).expect("read PMUX");
    let shift = 4 * (pin % 2);
    let value = (current & !(0xF << shift)) | (PMUX_FUNCTION_B << shift);
    bus.write_u8(pmux, value).expect("write PMUX function B");
    bus.write_u8(pincfg, 1).expect("set PINCFG.PMUXEN");
}

/// `analogRead()` on the SAMD21: enable the ADC and its APB clock, set
/// AR_DEFAULT (12-bit, MUXNEG GND, GAIN DIV2, INTVCC1), select the AIN, start
/// one conversion and read `RESULT`.
fn samd21_adc_read(machine: &mut Machine<CortexM>, port: u64, pin: u8, ain: u8) -> u16 {
    route_pad_to_adc(machine, port, pin);
    let bus = &mut machine.bus;
    let mask = bus.read_u32(PM_APBCMASK).expect("read PM.APBCMASK");
    bus.write_u32(PM_APBCMASK, mask | APBC_ADC)
        .expect("clock the ADC");
    bus.write_u8(ADC_CTRLA, CTRLA_ENABLE).expect("enable ADC");
    bus.write_u8(ADC_REFCTRL, REFSEL_INTVCC1)
        .expect("select INTVCC1");
    // RESSEL = 12-bit.
    bus.write_u16(ADC_CTRLB, 0).expect("select 12 bits");
    bus.write_u32(ADC_INPUTCTRL, u32::from(ain) | MUXNEG_GND | GAIN_DIV2)
        .expect("select the AIN");
    bus.write_u8(ADC_SWTRIG, SWTRIG_START)
        .expect("software-start the conversion");

    // The conversion completes on the next bus tick.
    for _ in 0..64 {
        if machine.bus.read_u8(ADC_INTFLAG).expect("read INTFLAG") & INTFLAG_RESRDY != 0 {
            break;
        }
        machine.run(Some(1)).expect("advance the guest one cycle");
    }
    assert!(
        machine.bus.read_u8(ADC_INTFLAG).expect("read INTFLAG") & INTFLAG_RESRDY != 0,
        "a software-started SAMD21 conversion must set RESRDY"
    );
    machine.bus.read_u16(ADC_RESULT).expect("read RESULT") & 0x0FFF
}

/// Load a SAMD21 profile, overlay the deck's `cosim_models`, and build the
/// machine and co-simulation.
fn bench(system: &str) -> (ChipDescriptor, Machine<CortexM>, CosimSession) {
    let system_path = root(system);
    let mut profile = SystemManifest::from_file(&system_path).expect("load SAMD21 profile");
    let deck =
        SystemManifest::from_file(root("examples/arduino-zero/analog-ntc-diode-system.yaml"))
            .expect("load the generic analog deck");
    profile.cosim_models = deck.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load the ATSAMD21G18A descriptor");

    // The descriptor map must be exactly the CMSIS `PIN_PxyyB_ADC_AINn` table
    // the converter converts against (`SAMD21G18A_AIN_PADS`), so a typo cannot
    // send a solved node to a channel silicon does not have.
    for &(ain, port, pin) in labwired_core::peripherals::sam::adc21::SAMD21G18A_AIN_PADS {
        let label = format!("P{}{}", if port == 0 { 'A' } else { 'B' }, pin);
        assert_eq!(
            chip.analog_pins[&label].channel, ain,
            "{label} must resolve to AIN{ain}"
        );
        assert_eq!(chip.analog_pins[&label].peripheral, "adc");
    }
    assert_eq!(
        chip.analog_pins.len(),
        labwired_core::peripherals::sam::adc21::SAMD21G18A_AIN_PADS.len(),
        "no extra pad may claim an AIN input"
    );
    // AIN8/AIN9 are not bonded on the 48-pin G package, so no pad may claim
    // them: a board net there would bias an input silicon does not bring out.
    assert!(
        chip.analog_pins
            .values()
            .all(|pin| pin.channel != 8 && pin.channel != 9),
        "the G package does not bond AIN8/AIN9"
    );

    let machine = board_machine(&chip, &profile);
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &machine.bus)
        .expect("build analog co-simulation")
        .expect("fixture carries the analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through PA2/PA4/PA6 to AIN0/AIN4/AIN6"
    );
    (chip, machine, session)
}

#[test]
fn arduino_zero_ntc_diode_divider_reach_a0_a3_a6_adc_end_to_end() {
    let (_chip, mut machine, mut session) = bench("configs/systems/arduino-zero.yaml");
    let expected_diode_v = diode_load_line_voltage();
    let mut samples = Vec::new();

    for temperature_c in [-20.0_f64, 25.0, 85.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        let cycles = session.cycles_until_boundary(0);
        let (steps, errors) = session
            .advance_to(cycles, &mut machine.bus)
            .expect("solve and route all three analog netlists");
        assert_eq!(steps.len(), 3, "all three netlists cross the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa2_volts");
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} C the Beta divider on A0 should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa4_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley diode load line on A1 should be {expected_diode_v} V, got {diode_v} V"
        );
        let divider_v = signal_voltage(&session, "board.analog.pa6_volts");
        let expected_divider_v = 3.3 * 4_700.0 / (10_000.0 + 4_700.0);
        assert!(
            (divider_v - expected_divider_v).abs() < 1e-9,
            "the divider should be {expected_divider_v} V, got {divider_v} V"
        );

        // PA2 = AIN0, PA4 = AIN4, PA6 = AIN6.
        let (ntc_code, diode_code, divider_code) = (
            samd21_adc_read(&mut machine, PORT_A, 2, 0),
            samd21_adc_read(&mut machine, PORT_A, 4, 4),
            samd21_adc_read(&mut machine, PORT_A, 6, 6),
        );
        assert_eq!(
            ntc_code,
            expected_code(ntc_v),
            "firmware-visible RESULT for AIN0 (A0) must report the solved NTC voltage"
        );
        assert_eq!(
            diode_code,
            expected_code(diode_v),
            "firmware-visible RESULT for AIN4 must report the solved diode voltage"
        );
        assert_eq!(
            divider_code,
            expected_code(divider_v),
            "firmware-visible RESULT for AIN6 must report the solved divider voltage"
        );

        // PB2 (AIN10) has no deck. It is a port-B pad, so a zero code proves
        // the routed port-A levels did not bleed sideways and the map really
        // carries the port bit as well as the channel.
        assert_eq!(
            samd21_adc_read(&mut machine, PORT_B, 2, 10),
            0,
            "an unrouted port-B input must keep the model's 0 mV default"
        );

        samples.push((temperature_c, ntc_v, ntc_code));
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
        samples[1].2,
        expected_code(1.65),
        "the 25 C NTC divider is exactly half scale on 3.3 V VDDANA"
    );
}

/// A pad handed to the ADC by the descriptor map is not enough: the converter
/// reads `PINCFG.PMUXEN` and the PMUX nibble. Leaving the pad as a plain GPIO
/// must convert 0, and only then setting function B must expose the routed
/// level. This is the port-mux half of the route, proven rather than assumed.
#[test]
fn samd21_conversion_requires_the_pad_to_be_muxed_to_the_adc() {
    let (_chip, mut machine, mut session) = bench("configs/systems/arduino-zero.yaml");
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set generic NTC temperature");
    let cycles = session.cycles_until_boundary(0);
    let (_, errors) = session
        .advance_to(cycles, &mut machine.bus)
        .expect("solve and route the NTC netlist");
    assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

    // Configure the ADC exactly as analogRead() does, but leave PA2 a GPIO.
    {
        let bus = &mut machine.bus;
        let mask = bus.read_u32(PM_APBCMASK).expect("read PM.APBCMASK");
        bus.write_u32(PM_APBCMASK, mask | APBC_ADC)
            .expect("clock the ADC");
        bus.write_u8(ADC_CTRLA, CTRLA_ENABLE).expect("enable ADC");
        bus.write_u8(ADC_REFCTRL, REFSEL_INTVCC1)
            .expect("select INTVCC1");
        bus.write_u16(ADC_CTRLB, 0).expect("select 12 bits");
        bus.write_u32(ADC_INPUTCTRL, u32::from(0u8) | MUXNEG_GND | GAIN_DIV2)
            .expect("select AIN0");
        bus.write_u8(ADC_SWTRIG, SWTRIG_START)
            .expect("software-start the conversion");
    }
    for _ in 0..64 {
        if machine.bus.read_u8(ADC_INTFLAG).expect("read INTFLAG") & INTFLAG_RESRDY != 0 {
            break;
        }
        machine.run(Some(1)).expect("advance the guest one cycle");
    }
    assert_eq!(
        machine.bus.read_u16(ADC_RESULT).expect("read RESULT") & 0x0FFF,
        0,
        "an unmuxed pad must not carry its routed level into the converter"
    );

    let routed = signal_voltage(&session, "board.analog.pa2_volts");
    assert!(
        routed > 0.5,
        "the NTC divider at 25 C is a mid-scale level, got {routed} V"
    );
    assert_eq!(
        samd21_adc_read(&mut machine, PORT_A, 2, 0),
        expected_code(routed),
        "muxing PA2 to function B must expose the same routed level"
    );
}

/// The Nano 33 IoT is the same part with a BME280 on SERCOM4 (PB8/PB9). The
/// analog front end must still convert while the sensor stays on the bus.
#[test]
fn nano_33_iot_keeps_the_bme280_and_converts_the_analog_deck() {
    let system_path = root("configs/systems/arduino-nano-33-iot-bme280.yaml");
    let profile = SystemManifest::from_file(&system_path).expect("load Nano 33 IoT profile");
    assert!(
        profile
            .external_devices
            .iter()
            .any(|dev| dev.id == "bme280" && dev.connection == "sercom4"),
        "the mixed-signal run keeps the BME280 on SERCOM4"
    );

    let (_chip, mut machine, mut session) =
        bench("configs/systems/arduino-nano-33-iot-bme280.yaml");
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set generic NTC temperature");
    let cycles = session.cycles_until_boundary(0);
    let (steps, errors) = session
        .advance_to(cycles, &mut machine.bus)
        .expect("solve and route the analog netlists");
    assert_eq!(steps.len(), 3, "all three netlists cross the 1 us boundary");
    assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

    let ntc_v = signal_voltage(&session, "board.analog.pa2_volts");
    assert_eq!(
        samd21_adc_read(&mut machine, PORT_A, 2, 0),
        expected_code(ntc_v),
        "AIN0 must convert the solved NTC voltage while the BME280 owns PB8/PB9"
    );
}
