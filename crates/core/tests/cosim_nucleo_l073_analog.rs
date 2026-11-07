// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists drive the NUCLEO-L073RZ, through two paths:
//!
//! * PA0/PA1 GPIO inputs, reaching GPIOA_IDR. Thresholds are simulation
//!   assumptions; silicon threshold behaviour is not claimed.
//! * ADC1, the F0/L0 "v2" block (`profile: stm32l0`). The same Beta NTC
//!   divider and unrelated Shockley diode solve onto PA0 (ADC_IN0) and PC5
//!   (ADC_IN15) through the descriptor's `analog_pins:`, and the register
//!   sequence the L0 HAL runs (ADCAL, ADEN/ADRDY, CHSELR, ADSTART/EOC) reads
//!   them back from ADC_DR. On the L4 map this board used before, CHSELR
//!   was not a register and every conversion returned one fixed code.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, GpioInputThresholds, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_AHBENR: u64 = 0x4002_1030;
const GPIOA_BASE: u64 = 0x5000_0000;
const GPIOA_MODER: u64 = GPIOA_BASE;
const GPIOA_IDR: u64 = GPIOA_BASE + 0x10;

const RCC_APB2ENR: u64 = 0x4002_1034;
const APB2ENR_ADCEN: u32 = 1 << 9;
const ADC1_BASE: u64 = 0x4001_2400;
const ADC_ISR: u64 = ADC1_BASE;
const ADC_CR: u64 = ADC1_BASE + 0x08;
const ADC_CFGR1: u64 = ADC1_BASE + 0x0C;
const ADC_CHSELR: u64 = ADC1_BASE + 0x28;
const ADC_DR: u64 = ADC1_BASE + 0x40;
const ISR_ADRDY: u32 = 1 << 0;
const ISR_EOC: u32 = 1 << 2;
const ISR_EOS: u32 = 1 << 3;
const ISR_EOCAL: u32 = 1 << 11;
const CR_ADEN: u32 = 1 << 0;
const CR_ADDIS: u32 = 1 << 1;
const CR_ADSTART: u32 = 1 << 2;
const CR_ADCAL: u32 = 1 << 31;
const CFGR1_SCANDIR: u32 = 1 << 2;
const CFGR1_RES_10BIT: u32 = 0b01 << 3;
const CFGR1_ALIGN: u32 = 1 << 5;
/// The fixed internal source an undriven channel converts (3.0 V of 3.3 V).
const UNDRIVEN_CODE: u32 = 3000 * 4096 / 3300;

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

#[test]
fn nucleo_l073rz_ntc_and_diode_netlists_reach_pa0_pa1_gpio_inputs() {
    let system_path = root("configs/systems/nucleo-l073rz.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L073RZ profile");
    assert_eq!(profile.name, "nucleo-l073rz");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-l073rz/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L073 descriptor");
    // Test thresholds only; these values are not asserted as L073 silicon data.
    chip.io_voltage_v = Some(3.3);
    chip.gpio_input_thresholds = Some(GpioInputThresholds { vil: 0.3, vih: 0.7 });

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L073RZ bus");
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA0"),
        Some((GPIOA_IDR, 0)),
        "PA0 must resolve to GPIOA IDR bit 0"
    );
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA1"),
        Some((GPIOA_IDR, 1)),
        "PA1 must resolve to GPIOA IDR bit 1"
    );

    bus.write_u32(RCC_AHBENR, 1).expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_MODER).expect("read GPIOA mode");
    bus.write_u32(GPIOA_MODER, moder & !0xF)
        .expect("configure PA0/PA1 as digital inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must bind to distinct modeled PA0/PA1 inputs"
    );

    let expected_diode_v = diode_load_line_voltage();
    let vil_v = 0.3 * 3.3;
    assert!(
        expected_diode_v < vil_v,
        "diode must fall below assumed VIL"
    );
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // retain the cold high level inside the assumed Schmitt band
        (80.0, false),
        (25.0, false), // retain the hot low level inside the assumed Schmitt band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(steps.len(), 2, "both generic netlists cross the boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "board.gpio_in.pa1");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let gpioa_idr = bus.read_u32(GPIOA_IDR).expect("read GPIOA input data");
        assert_eq!(
            gpioa_idr & 1 != 0,
            expected_ntc_level,
            "PA0 must follow the solved NTC voltage and retain threshold state"
        );
        assert_eq!(gpioa_idr & 2, 0, "PA1 must read low below assumed VIL");
    }
}

/// 12-bit code for a routed voltage: the bus carries whole millivolts.
fn code12(volts: f64) -> u32 {
    let millivolts = (volts * 1000.0).round() as u32;
    millivolts * 4095 / 3300
}

/// L0 HAL bring-up: clock ADC1, calibrate, enable and wait for ADRDY.
fn adc_power_up(bus: &mut SystemBus) {
    let apb2enr = bus.read_u32(RCC_APB2ENR).expect("read APB2ENR");
    bus.write_u32(RCC_APB2ENR, apb2enr | APB2ENR_ADCEN)
        .expect("clock ADC1");
    assert_eq!(
        bus.read_u32(ADC_CR).unwrap(),
        0,
        "v2 CR resets to 0, no DEEPPWD"
    );
    bus.write_u32(ADC_CR, CR_ADCAL).expect("start calibration");
    assert_eq!(
        bus.read_u32(ADC_CR).unwrap() & CR_ADCAL,
        0,
        "ADCAL self-clears"
    );
    assert_ne!(
        bus.read_u32(ADC_ISR).unwrap() & ISR_EOCAL,
        0,
        "EOCAL raised"
    );
    bus.write_u32(ADC_ISR, ISR_EOCAL).expect("clear EOCAL");
    bus.write_u32(ADC_CR, CR_ADEN).expect("enable ADC");
    assert_ne!(
        bus.read_u32(ADC_ISR).unwrap() & ISR_ADRDY,
        0,
        "ADEN raises ADRDY"
    );
}

/// One software-started conversion of the CHSELR selection.
fn adc_convert(bus: &mut SystemBus, chselr: u32, cfgr1: u32) -> u32 {
    bus.write_u32(ADC_CFGR1, cfgr1).expect("write CFGR1");
    bus.write_u32(ADC_CHSELR, chselr).expect("select channels");
    bus.write_u32(ADC_ISR, ISR_EOC | ISR_EOS)
        .expect("clear EOC/EOS");
    bus.write_u32(ADC_CR, CR_ADEN | CR_ADSTART)
        .expect("start conversion");
    let isr = bus.read_u32(ADC_ISR).expect("read ISR");
    assert_ne!(isr & ISR_EOC, 0, "EOC after ADSTART");
    assert_ne!(isr & ISR_EOS, 0, "EOS after the single-channel sequence");
    assert_eq!(
        bus.read_u32(ADC_CR).unwrap() & CR_ADSTART,
        0,
        "ADSTART self-clears"
    );
    bus.read_u32(ADC_DR).expect("read DR")
}

#[test]
fn nucleo_l073rz_ntc_and_diode_netlists_convert_through_chselr_adc() {
    let system_path = root("configs/systems/nucleo-l073rz.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L073RZ profile");
    let analog = SystemManifest::from_file(root("examples/nucleo-l073rz/analog-adc-system.yaml"))
        .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L073 descriptor");
    let pads: Vec<_> = ["PA0", "PA7", "PB0", "PB1", "PC0", "PC5"]
        .iter()
        .map(|pad| {
            let adc = &chip.analog_pins[*pad];
            (adc.peripheral.as_str(), adc.channel)
        })
        .collect();
    assert_eq!(
        pads,
        [
            ("adc1", 0),
            ("adc1", 7),
            ("adc1", 8),
            ("adc1", 9),
            ("adc1", 10),
            ("adc1", 15)
        ],
        "DS10685 LQFP64 ADC_IN assignment"
    );
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L073RZ bus");

    // ADSTART before ADEN converts nothing.
    let apb2enr = bus.read_u32(RCC_APB2ENR).unwrap();
    bus.write_u32(RCC_APB2ENR, apb2enr | APB2ENR_ADCEN).unwrap();
    bus.write_u32(ADC_CHSELR, 1).unwrap();
    bus.write_u32(ADC_CR, CR_ADSTART).unwrap();
    assert_eq!(
        bus.read_u32(ADC_ISR).unwrap() & ISR_EOC,
        0,
        "no EOC while disabled"
    );
    assert_eq!(
        bus.read_u32(ADC_DR).unwrap(),
        0,
        "no conversion while disabled"
    );
    bus.write_u32(ADC_CR, 0).unwrap();
    adc_power_up(&mut bus);

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "PA0 and PC5 must resolve to ADC1 channels"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut previous_ntc_code = u32::MAX;
    for temperature_c in [-10.0_f64, 25.0, 50.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(steps.len(), 2, "both generic netlists cross the boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pc5_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let ntc_code = adc_convert(&mut bus, 1 << 0, 0);
        assert_eq!(
            ntc_code,
            code12(expected_ntc_v),
            "IN0 at {temperature_c} °C"
        );
        assert!(
            ntc_code < previous_ntc_code,
            "NTC code must fall as temperature rises: {ntc_code} at {temperature_c} °C"
        );
        previous_ntc_code = ntc_code;

        let diode_code = code12(expected_diode_v);
        assert_eq!(adc_convert(&mut bus, 1 << 15, 0), diode_code, "IN15 diode");
        // CFGR1.RES = 10 bit drops two LSBs; ALIGN left-justifies to bit 15.
        assert_eq!(
            adc_convert(&mut bus, 1 << 15, CFGR1_RES_10BIT),
            diode_code >> 2,
            "10-bit diode code"
        );
        assert_eq!(
            adc_convert(&mut bus, 1 << 15, CFGR1_ALIGN),
            diode_code << 4,
            "left-aligned 12-bit diode code"
        );
        // Both selected: the scan starts at IN0 upward, at IN15 downward.
        let both = (1 << 0) | (1 << 15);
        assert_eq!(
            adc_convert(&mut bus, both, 0),
            ntc_code,
            "upward scan starts at IN0"
        );
        assert_eq!(
            adc_convert(&mut bus, both, CFGR1_SCANDIR),
            diode_code,
            "SCANDIR scan starts at IN15"
        );
        // A pad nothing drives keeps the fixed internal source.
        assert_eq!(
            adc_convert(&mut bus, 1 << 7, 0),
            UNDRIVEN_CODE,
            "undriven IN7"
        );
    }

    // ADDIS powers the converter down: ADRDY drops and ADSTART is ignored.
    bus.write_u32(ADC_CR, CR_ADEN | CR_ADDIS).unwrap();
    assert_eq!(
        bus.read_u32(ADC_ISR).unwrap() & ISR_ADRDY,
        0,
        "ADDIS clears ADRDY"
    );
    bus.write_u32(ADC_ISR, ISR_EOC).unwrap();
    bus.write_u32(ADC_CR, CR_ADSTART).unwrap();
    assert_eq!(
        bus.read_u32(ADC_ISR).unwrap() & ISR_EOC,
        0,
        "no conversion after ADDIS"
    );
}
