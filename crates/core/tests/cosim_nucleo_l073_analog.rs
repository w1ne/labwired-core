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

/// Independent level-1 NMOS body-effect load-line reference. With the source
/// grounded, `bulk_v` is VBS; reverse bias raises threshold through GAMMA/PHI.
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
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

/// Independent level-1 NJF load line including model-card drain/source
/// resistances. The scalar root solves channel current against both the
/// external load and intrinsic terminal drops, independently of MNA.
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

/// One software-started scan of a `len`-channel CHSELR sequence, read the way
/// a polling firmware reads it: wait for EOC, read DR, repeat. EOS must stay
/// low until the last channel has landed.
fn adc_scan(bus: &mut SystemBus, chselr: u32, cfgr1: u32, len: usize) -> Vec<u32> {
    bus.write_u32(ADC_CFGR1, cfgr1).expect("write CFGR1");
    bus.write_u32(ADC_CHSELR, chselr).expect("select channels");
    bus.write_u32(ADC_ISR, ISR_EOC | ISR_EOS)
        .expect("clear EOC/EOS");
    bus.write_u32(ADC_CR, CR_ADEN | CR_ADSTART)
        .expect("start scan");
    (0..len)
        .map(|slot| {
            let isr = bus.read_u32(ADC_ISR).expect("read ISR");
            assert_ne!(isr & ISR_EOC, 0, "EOC for conversion {slot}");
            assert_eq!(
                isr & ISR_EOS != 0,
                slot == len - 1,
                "EOS only once conversion {} of {len} lands",
                len - 1
            );
            // HAL_ADC_PollForConversion clears EOC before HAL_ADC_GetValue.
            bus.write_u32(ADC_ISR, ISR_EOC).expect("clear EOC");
            bus.read_u32(ADC_DR).expect("read DR")
        })
        .collect()
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
        // Both selected: one ADSTART scans IN0 then IN15 upward, IN15 then
        // IN0 downward, each result landing in DR in turn.
        let both = (1 << 0) | (1 << 15);
        assert_eq!(
            adc_scan(&mut bus, both, 0, 2),
            [ntc_code, diode_code],
            "upward scan IN0, IN15"
        );
        assert_eq!(
            adc_scan(&mut bus, both, CFGR1_SCANDIR, 2),
            [diode_code, ntc_code],
            "SCANDIR scan IN15, IN0"
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

#[test]
fn nucleo_l073rz_mosfet_body_effect_reaches_pa2_pa3_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-l073rz.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L073RZ profile");
    assert_eq!(profile.name, "nucleo-l073rz");

    let analog = SystemManifest::from_file(root("examples/nucleo-l073rz/analog-adc-system.yaml"))
        .expect("load the generic NTC and Shockley diode netlists");
    let mosfet = SystemManifest::from_file(root(
        "examples/nucleo-l073rz/analog-mosfet-body-effect-adc-system.yaml",
    ))
    .expect("load the matched generic NMOS body-bias netlists");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L073 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA2", 2), ("PA3", 3), ("PC5", 15)] {
        assert_eq!(
            chip.analog_pins[pad].channel, channel,
            "{pad} must resolve to ADC1_IN{channel}"
        );
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L073RZ bus");
    adc_power_up(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched NMOS branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all four devices must bind through their distinct board ADC routes"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the unrelated Beta NTC temperature");
    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut body_samples = Vec::new();
    for bulk_v in [0.0_f64, -0.5, -1.0, -1.5] {
        session
            .set_signal_number("ui.mosfet_body.bulk_v", bulk_v)
            .expect("set generic NMOS reverse body bias");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched NMOS netlists");
        assert_eq!(
            steps.len(),
            3,
            "all three analog models solve at each boundary"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let reference_v = signal_voltage(&session, "board.analog.pa2_volts");
        let body_v = signal_voltage(&session, "board.analog.pa3_volts");
        let expected_reference_v = mosfet_body_effect_drain_voltage(2.5, 0.0);
        let expected_body_v = mosfet_body_effect_drain_voltage(2.5, bulk_v);
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "zero-body-bias NMOS should settle at {expected_reference_v} V, got {reference_v} V"
        );
        assert!(
            (body_v - expected_body_v).abs() < 5e-5,
            "NMOS at VBS={bulk_v} V should settle at {expected_body_v} V, got {body_v} V"
        );
        if bulk_v == 0.0 {
            assert!(
                (body_v - reference_v).abs() < 1e-9,
                "matched devices agree at zero body bias"
            );
        }

        for (channel, volts) in [
            (0, expected_ntc_v),
            (2, reference_v),
            (3, body_v),
            (15, expected_diode_v),
        ] {
            assert_eq!(
                adc_convert(&mut bus, 1 << channel, 0),
                code12(volts),
                "firmware-visible ADC1_IN{channel} conversion follows the solved voltage"
            );
        }
        assert_eq!(
            adc_convert(&mut bus, 1 << 4, 0),
            UNDRIVEN_CODE,
            "the neighboring ADC1_IN4 channel remains undriven"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa0_volts") - expected_ntc_v).abs() < 1e-9,
            "the unrelated NTC stays isolated on PA0/ADC1_IN0"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pc5_volts") - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode stays isolated on PC5/ADC1_IN15"
        );
        body_samples.push((bulk_v, body_v));
    }

    assert!(
        body_samples.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "reverse body bias raises Vth and the loaded drain voltage: {body_samples:?}"
    );
}

#[test]
fn nucleo_l073rz_jfet_terminal_resistances_reach_adc1_with_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-l073rz.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L073RZ profile");
    let base = SystemManifest::from_file(root("examples/nucleo-l073rz/analog-adc-system.yaml"))
        .expect("load unrelated generic NTC and Shockley diode routes");
    let jfet = SystemManifest::from_file(root(
        "examples/nucleo-l073rz/analog-jfet-terminal-resistance-adc.yaml",
    ))
    .expect("load matched generic level-1 JFET branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(jfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L073 descriptor");
    for (pad, channel) in [
        ("PA0", 0),
        ("PA2", 2),
        ("PA3", 3),
        ("PA4", 4),
        ("PA5", 5),
        ("PC5", 15),
    ] {
        assert_eq!(
            chip.analog_pins[pad].channel, channel,
            "{pad} must resolve to ADC1_IN{channel}"
        );
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L073RZ bus");
    adc_power_up(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and four matched JFET branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models must resolve through PA0/PA2–PA5/PC5 ADC routes"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the unrelated NTC temperature");
    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let branch_channels = [
        ("control", "pa2", 2, 0.0, 0.0),
        ("drain", "pa3", 3, 200.0, 0.0),
        ("source", "pa4", 4, 0.0, 200.0),
        ("both", "pa5", 5, 200.0, 200.0),
    ];
    let mut cycles = 0;
    let mut settled_jfet_voltages = None;
    for gate_v in [-1.0_f64, 0.0] {
        session
            .set_signal_number("ui.jfet.gate_v", gate_v)
            .expect("set matched JFET gate drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and JFET branches");
        assert_eq!(steps.len(), 3, "NTC, diode, and JFET netlists solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pc5_volts");
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "unrelated NTC stays isolated"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated diode stays on its independent load line"
        );

        let mut jfet_voltages = Vec::new();
        for (name, pad, channel, drain_ohms, source_ohms) in branch_channels {
            let solved_v = signal_voltage(&session, &format!("board.analog.{pad}_volts"));
            let expected_v = jfet_drain_voltage(gate_v, drain_ohms, source_ohms);
            assert!(
                (solved_v - expected_v).abs() < 5e-5,
                "{name} NJF load line at gate={gate_v} V should be {expected_v} V, got {solved_v} V"
            );
            assert_eq!(
                adc_convert(&mut bus, 1 << channel, 0),
                code12(solved_v),
                "firmware-visible ADC1_IN{channel} conversion follows the {name} JFET"
            );
            jfet_voltages.push(solved_v);
        }
        assert_eq!(
            adc_convert(&mut bus, 1 << 0, 0),
            code12(ntc_v),
            "ADC1_IN0 still reads the unrelated NTC"
        );
        assert_eq!(
            adc_convert(&mut bus, 1 << 15, 0),
            code12(diode_v),
            "ADC1_IN15 still reads the unrelated diode"
        );
        assert_eq!(
            adc_convert(&mut bus, 1 << 1, 0),
            UNDRIVEN_CODE,
            "unwired ADC1_IN1 remains on the undriven input source"
        );
        settled_jfet_voltages = Some(jfet_voltages);
    }

    let [control, drain, source, both] = settled_jfet_voltages
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

/// Solve both netlists at `temperature_c` and route them onto IN0/IN15.
fn settle(session: &mut CosimSession, bus: &mut SystemBus, cycles: &mut u64, temperature_c: f64) {
    session
        .set_signal_number("ui.ntc.temperature_c", temperature_c)
        .expect("set NTC temperature");
    *cycles += session.cycles_until_boundary(*cycles);
    let (_, errors) = session
        .advance_to(*cycles, bus)
        .expect("solve and route both analog netlists");
    assert!(errors.is_empty(), "unexpected route errors: {errors:?}");
}

fn expected_ntc_code(temperature_c: f64) -> u32 {
    let resistance = ntc_resistance_ohms(temperature_c);
    code12(3.3 * resistance / (10_000.0 + resistance))
}

#[test]
fn nucleo_l073rz_chselr_sequence_modes_track_the_solved_ntc_and_diode() {
    const CFGR1_CONT: u32 = 1 << 13;
    const CFGR1_DISCEN: u32 = 1 << 16;
    const CR_ADSTP: u32 = 1 << 4;
    let system_path = root("configs/systems/nucleo-l073rz.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L073RZ profile");
    let analog = SystemManifest::from_file(root("examples/nucleo-l073rz/analog-adc-system.yaml"))
        .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L073 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L073RZ bus");
    adc_power_up(&mut bus);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    let mut cycles = 0;
    let diode_code = code12(diode_load_line_voltage());
    let both = (1 << 0) | (1 << 15);

    // CONT: the sequence keeps wrapping, and each result is sampled when it
    // lands, so a temperature step between reads shows up in the next IN0.
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    bus.write_u32(ADC_CFGR1, CFGR1_CONT).unwrap();
    bus.write_u32(ADC_CHSELR, both).unwrap();
    bus.write_u32(ADC_ISR, ISR_EOC | ISR_EOS).unwrap();
    bus.write_u32(ADC_CR, CR_ADEN | CR_ADSTART).unwrap();
    assert_ne!(
        bus.read_u32(ADC_CR).unwrap() & CR_ADSTART,
        0,
        "ADSTART stays set in CONT"
    );
    let cold = expected_ntc_code(-10.0);
    assert_eq!(
        bus.read_u32(ADC_DR).unwrap(),
        cold,
        "CONT 1st: IN0 at -10 °C"
    );
    assert_ne!(
        bus.read_u32(ADC_ISR).unwrap() & ISR_EOS,
        0,
        "EOS once IN15, the last channel, lands"
    );
    bus.write_u32(ADC_ISR, ISR_EOS).unwrap();
    assert_eq!(bus.read_u32(ADC_DR).unwrap(), diode_code, "CONT 2nd: IN15");
    assert_eq!(
        bus.read_u32(ADC_ISR).unwrap() & ISR_EOS,
        0,
        "EOS stays clear while IN0 starts the next pass"
    );
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    // The IN0 that landed before the step still holds the cold sample; the
    // one after it is the first to see 80 °C.
    assert_eq!(
        bus.read_u32(ADC_DR).unwrap(),
        cold,
        "CONT 3rd: landed before the step"
    );
    assert_eq!(bus.read_u32(ADC_DR).unwrap(), diode_code, "CONT 4th: IN15");
    let hot = expected_ntc_code(80.0);
    assert!(hot < cold, "NTC code falls with temperature");
    // A half-word read consumes DR once, not once per byte.
    assert_eq!(
        u32::from(bus.read_u16(ADC_DR).unwrap()),
        hot,
        "CONT 5th: IN0 at 80 °C"
    );
    assert_eq!(
        u32::from(bus.read_u16(ADC_DR).unwrap()),
        diode_code,
        "CONT 6th: IN15"
    );

    // ADSTP stops the sequence: DR keeps its last result, nothing more lands.
    bus.write_u32(ADC_CR, CR_ADEN | CR_ADSTP).unwrap();
    assert_eq!(
        bus.read_u32(ADC_CR).unwrap() & CR_ADSTART,
        0,
        "ADSTP clears ADSTART"
    );
    let held = bus.read_u32(ADC_DR).unwrap();
    assert_eq!(held, hot, "DR keeps the IN0 that had landed");
    assert_eq!(
        bus.read_u32(ADC_DR).unwrap(),
        held,
        "no conversion after ADSTP"
    );

    // DISCEN: each ADSTART converts only the next channel; EOS marks the end.
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let room = expected_ntc_code(25.0);
    bus.write_u32(ADC_CFGR1, CFGR1_DISCEN).unwrap();
    bus.write_u32(ADC_CHSELR, both).unwrap();
    for (expected, last) in [(room, false), (diode_code, true), (room, false)] {
        bus.write_u32(ADC_ISR, ISR_EOC | ISR_EOS).unwrap();
        bus.write_u32(ADC_CR, CR_ADEN | CR_ADSTART).unwrap();
        let isr = bus.read_u32(ADC_ISR).unwrap();
        assert_ne!(isr & ISR_EOC, 0, "DISCEN: EOC per trigger");
        assert_eq!(isr & ISR_EOS != 0, last, "DISCEN: EOS only on IN15");
        assert_eq!(bus.read_u32(ADC_DR).unwrap(), expected, "DISCEN result");
        assert_eq!(
            bus.read_u32(ADC_ISR).unwrap() & ISR_EOS != 0,
            last,
            "DISCEN: reading DR lands nothing more"
        );
        assert_eq!(
            bus.read_u32(ADC_DR).unwrap(),
            expected,
            "DISCEN: one per trigger"
        );
    }
}
