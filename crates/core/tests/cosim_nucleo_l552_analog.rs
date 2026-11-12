// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists drive PA0/PA1 on the NUCLEO-L552ZE-Q through the
//! STM32L552 input thresholds and into GPIOA_IDR, then, as analog levels,
//! resolve through the chip's `analog_pins` to ADC1_IN5/IN6 and convert
//! through ADC1's regular sequence and injected group. Matched Gummel–Poon BJT
//! branches exercise recombination and Early effect through additional ADC
//! channels. A matched BJT pair also exercises Gummel–Poon high-injection
//! rolloff through ADC1. Silicon behaviour is not claimed.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::adc::Adc;
use labwired_core::Bus;
use std::path::Path;

const RCC_AHB2ENR: u64 = 0x4002_104C;
const GPIOA_IDR: u64 = 0x4202_0010;
// ST CMSIS stm32l552xx.h: ADC1_BASE_NS = AHB2PERIPH_BASE_NS + 0x8000.
const ADC1_BASE: u64 = 0x4202_8000;
const ADC_ISR: u64 = ADC1_BASE;
const ADC_CR: u64 = ADC1_BASE + 0x08;
const ADC_SQR1: u64 = ADC1_BASE + 0x30;
const ADC_DR: u64 = ADC1_BASE + 0x40;
const ADC_JSQR: u64 = ADC1_BASE + 0x4C;
const ADC_JDR1: u64 = ADC1_BASE + 0x80;
const ADC_ON: u32 = (1 << 28) | 1; // ADVREGEN | ADEN
const RCC_AHB2ENR_ADCEN: u32 = 1 << 13;

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

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// Independent Gummel–Poon load line with optional B-E and B-C recombination.
fn bjt_recombination_collector_voltage(
    base_drive_v: f64,
    base_emitter_recombination: bool,
    base_collector_recombination: bool,
) -> f64 {
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
    const ISC_A: f64 = 5e-10;
    const NC: f64 = 2.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let base_width = 1.0 - vbc / VAF - vbe / VAR;
        let transport = IS_A * (forward - reverse) * base_width;
        let base_collector = (IS_A / BR) * (reverse - 1.0)
            + if base_collector_recombination {
                ISC_A * ((vbc / (NC * VT)).exp() - 1.0)
            } else {
                0.0
            };
        let base_emitter = (IS_A / BF) * (forward - 1.0)
            + if base_emitter_recombination {
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
        let residual = (base_drive_v - vbe) / BASE_OHMS - base_current;
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe(0.5 * (low + high))
}

/// Independent Gummel–Poon load line with optional forward Early effect.
fn bjt_early_effect_collector_voltage(base_drive_v: f64, vaf: Option<f64>) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const VAR: f64 = 75.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let base_width = 1.0 - vbe / VAR - vaf.map_or(0.0, |voltage| vbc / voltage);
        let transport = IS_A * (forward - reverse) * base_width;
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        let base_emitter = (IS_A / BF) * (forward - 1.0);
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

/// Independent Gummel–Poon load line with forward/reverse high-injection
/// rolloff and the same Early voltages in both branches.
fn bjt_high_injection_collector_voltage(base_drive_v: f64, high_injection: bool) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const VAF: f64 = 50.0;
    const VAR: f64 = 75.0;
    const IKF_A: f64 = 50e-6;
    const IKR_A: f64 = 500e-6;
    const NKF: f64 = 0.8;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let base_width = 1.0 - vbc / VAF - vbe / VAR;
        let q2 = if high_injection {
            IS_A * (forward - 1.0) / IKF_A + IS_A * (reverse - 1.0) / IKR_A
        } else {
            0.0
        };
        let qb = (1.0 + (1.0 + 4.0 * q2).max(0.0).powf(NKF)) / (2.0 * base_width);
        let transport = IS_A * (forward - reverse) / qb;
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        let base_emitter = (IS_A / BF) * (forward - 1.0);
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

#[test]
fn nucleo_l552ze_q_ntc_and_diode_netlists_reach_gpio_thresholds() {
    let system_path = root("configs/systems/nucleo-l552ze-q.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-L552ZE-Q profile");
    assert_eq!(profile.name, "nucleo-l552ze-q");
    let analog_path = root("examples/nucleo-l552ze-q/analog-ntc-diode-system.yaml");
    let analog = SystemManifest::from_file(&analog_path).expect("load generic NTC and diode");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L552 descriptor");
    let mut bus =
        SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L552ZE-Q system bus");
    bus.write_u32(RCC_AHB2ENR, 1).expect("enable GPIOA clock");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both generic devices must bind to distinct STM32 GPIO input pads"
    );

    let expected_diode_v = diode_load_line_voltage();
    assert!(
        expected_diode_v < 0.99,
        "diode must fall below STM32L552 VIL"
    );
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // retain the cold high level inside the Schmitt band
        (80.0, false),
        (25.0, false), // retain the hot low level inside the Schmitt band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(steps.len(), 2, "both analog netlists cross the boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );

        let diode_v = signal_voltage(&session, "board.gpio_in.pa1");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let gpioa = bus.read_u32(GPIOA_IDR).expect("read GPIOA input data");
        assert_eq!(
            gpioa & 1 != 0,
            expected_ntc_level,
            "PA0 must follow the solved NTC voltage and retain its Schmitt state"
        );
        assert_eq!(gpioa & 2, 0, "PA1 must read low below the configured VIL");
    }
}

fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("L552 profile declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<Adc>()
        .expect("ADC1 uses the shared STM32 ADC model, not a stub")
        .channel_input_count(channel)
}

fn expected_adc12_count(voltage_v: f64) -> u32 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 4095 / 3300).min(4095)
}

/// One software-started regular sequence (`SQR1.L` = channels - 1), read by
/// polling `EOC` and `DR`.
fn l5_adc_scan(bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
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

/// One `JADSTART` over `channels` (JSQ1 first) in the L5 `JSQR` packing,
/// which is the narrow L4 one: `JL[1:0]`, `JEXTSEL[5:2]`, `JEXTEN[7:6]`,
/// `JSQ1` at bit 8 and six bits per rank (ST CMSIS `stm32l552xx.h`
/// `ADC_JSQR_*_Pos`). Returns `JDR1..JDRn`.
fn l5_adc_injected(bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
    const JEOC_JEOS: u32 = (1 << 5) | (1 << 6);
    let jsqr = channels
        .iter()
        .enumerate()
        .fold(channels.len() as u32 - 1, |w, (k, &ch)| {
            w | (u32::from(ch) << (8 + 6 * k))
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
fn nucleo_l552ze_q_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-l552ze-q.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-L552ZE-Q profile");
    let analog = SystemManifest::from_file(root(
        "examples/nucleo-l552ze-q/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip = ChipDescriptor::from_file(system_path.parent().unwrap().join(&profile.chip))
        .expect("load STM32L552 descriptor");
    for (pad, channel) in [("PA0", 5), ("PA1", 6)] {
        assert_eq!(
            chip.analog_pins
                .get(pad)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pad} is ADC1_IN{channel} (ST open pin data)"
        );
    }
    let mut bus =
        SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L552ZE-Q system bus");

    bus.write_u32(ADC_CR, 0)
        .expect("write ADC1 while unclocked");
    bus.write_u32(ADC_CR, 1 << 28)
        .expect("write ADC1 while unclocked");
    bus.write_u32(ADC_CR, ADC_ON)
        .expect("write ADC1 while unclocked");
    assert_eq!(
        bus.read_u32(ADC_ISR).unwrap() & 1,
        0,
        "ADC1 ignores ADEN until RCC_AHB2ENR.ADCEN clocks it"
    );
    let ahb2enr = bus.read_u32(RCC_AHB2ENR).expect("read RCC AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, ahb2enr | RCC_AHB2ENR_ADCEN)
        .expect("enable ADC clock");
    bus.write_u32(ADC_CR, 0).expect("leave ADC deep power-down");
    bus.write_u32(ADC_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC_CR, ADC_ON).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

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
        assert_eq!(l5_adc_scan(&mut bus, &[5]), vec![ntc], "SQ1 = IN5");
        assert_eq!(l5_adc_scan(&mut bus, &[6]), vec![diode], "SQ1 = IN6");
        assert_eq!(
            l5_adc_scan(&mut bus, &[6, 5, 6]),
            vec![diode, ntc, diode],
            "one ADSTART scans the routed sensors in SQx order"
        );
        let dr = bus.read_u32(ADC_DR).unwrap();
        assert_eq!(
            l5_adc_injected(&mut bus, &[5, 6, 5, 6]),
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
fn nucleo_l552ze_q_bjt_base_recombination_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-l552ze-q.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L552ZE-Q");
    let generic = SystemManifest::from_file(root(
        "examples/nucleo-l552ze-q/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode routes");
    let recombination = SystemManifest::from_file(root(
        "examples/nucleo-l552ze-q/analog-bjt-recombination-adc-system.yaml",
    ))
    .expect("load matched generic BJT recombination branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(recombination.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L552 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L552ZE-Q bus");
    bus.write_u32(ADC_CR, 0)
        .expect("write ADC1 while unclocked");
    bus.write_u32(ADC_CR, 1 << 28)
        .expect("write ADC1 while unclocked");
    bus.write_u32(ADC_CR, ADC_ON)
        .expect("write ADC1 while unclocked");
    assert_eq!(
        bus.read_u32(ADC_ISR).unwrap() & 1,
        0,
        "ADC1 ignores ADEN until RCC_AHB2ENR.ADCEN clocks it"
    );
    let ahb2enr = bus.read_u32(RCC_AHB2ENR).expect("read RCC AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, ahb2enr | RCC_AHB2ENR_ADCEN)
        .expect("enable ADC clock");
    bus.write_u32(ADC_CR, 0).expect("leave ADC deep power-down");
    bus.write_u32(ADC_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC_CR, ADC_ON).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixtures carry NTC, diode, and four BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must bind to independent ADC1 channel endpoints"
    );
    for channel in [7, 8, 9, 10, 11] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} begins without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_recombination.base_v", 0.0)
        .expect("initialize matched BJT base-drive sources");

    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut saw_base_emitter_effect = false;
    let mut saw_base_collector_effect = false;
    for base_drive_v in [0.7_f64, 0.9, 1.2] {
        session
            .set_signal_number("ui.bjt_recombination.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and four BJT branches");
        assert_eq!(steps.len(), 6, "all six generic analog branches solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        assert!(
            (signal_voltage(&session, "board.analog.pa0_volts") - ntc_v).abs() < 1e-9,
            "unrelated Beta NTC stays isolated on ADC1 channel 5"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa1_volts") - diode_v).abs() < 2e-5,
            "unrelated Shockley diode stays isolated on ADC1 channel 6"
        );

        let bjt_voltages = [
            signal_voltage(&session, "board.analog.pa2_volts"),
            signal_voltage(&session, "board.analog.pa3_volts"),
            signal_voltage(&session, "board.analog.pa4_volts"),
            signal_voltage(&session, "board.analog.pa5_volts"),
        ];
        let expected = [
            bjt_recombination_collector_voltage(base_drive_v, false, false),
            bjt_recombination_collector_voltage(base_drive_v, true, false),
            bjt_recombination_collector_voltage(base_drive_v, false, true),
            bjt_recombination_collector_voltage(base_drive_v, true, true),
        ];
        for (index, (voltage, expected_v)) in bjt_voltages.iter().zip(expected).enumerate() {
            assert!(
                (voltage - expected_v).abs() < 5e-5,
                "PA{} / ADC1 BJT branch {index} at base drive {base_drive_v} V should match independent load line {expected_v} V, got {voltage} V",
                index + 2
            );
        }
        assert!(
            bjt_voltages[1] > bjt_voltages[0] + 0.005,
            "ISE/NE base-emitter recombination loads the base drive and raises collector voltage"
        );
        saw_base_emitter_effect |= bjt_voltages[1] - bjt_voltages[0] > 0.005;
        saw_base_collector_effect |= bjt_voltages[2] - bjt_voltages[0] > 0.1;

        let samples = [(5, ntc_v), (6, diode_v)]
            .into_iter()
            .chain((7..=10).zip(bjt_voltages));
        let mut scan_channels = Vec::new();
        let mut expected_scan = Vec::new();
        for (channel, voltage_v) in samples {
            let expected_count = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count as u16,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                l5_adc_scan(&mut bus, &[channel]),
                vec![expected_count],
                "firmware-visible ADC1 scan follows the generic device on channel {channel}"
            );
            scan_channels.push(channel);
            expected_scan.push(expected_count);
        }
        assert_eq!(
            l5_adc_scan(&mut bus, &scan_channels[..4]),
            expected_scan[..4],
            "four-rank regular sequence converts NTC, diode, and first two BJT branches"
        );
        assert_eq!(
            l5_adc_scan(&mut bus, &scan_channels[4..]),
            expected_scan[4..],
            "two-rank regular sequence converts the remaining BJT branches"
        );
        assert_eq!(
            adc_channel_count(&mut bus, 11),
            u16::MAX,
            "neighboring ADC1 channel 11 remains isolated"
        );
    }
    assert!(saw_base_emitter_effect, "ISE/NE effect reaches ADC1");
    assert!(
        saw_base_collector_effect,
        "ISC/NC forward base-collector recombination changes the ADC1 load line"
    );
}

#[test]
fn nucleo_l552ze_q_bjt_early_effect_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-l552ze-q.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L552ZE-Q");
    let generic = SystemManifest::from_file(root(
        "examples/nucleo-l552ze-q/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode routes");
    let bjt = SystemManifest::from_file(root(
        "examples/nucleo-l552ze-q/analog-bjt-early-effect-adc-system.yaml",
    ))
    .expect("load matched generic Gummel–Poon NPN branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L552 descriptor");
    for (pad, channel) in [("PA0", 5), ("PA1", 6), ("PA6", 11), ("PA7", 12)] {
        assert_eq!(
            chip.analog_pins
                .get(pad)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pad} resolves through ST-data analog_pins to ADC1_IN{channel}"
        );
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L552ZE-Q bus");
    bus.write_u32(ADC_CR, 0)
        .expect("write ADC1 while unclocked");
    bus.write_u32(ADC_CR, 1 << 28)
        .expect("write ADC1 while unclocked");
    bus.write_u32(ADC_CR, ADC_ON)
        .expect("write ADC1 while unclocked");
    assert_eq!(
        bus.read_u32(ADC_ISR).unwrap() & 1,
        0,
        "ADC1 ignores ADEN until RCC_AHB2ENR.ADCEN clocks it"
    );
    let ahb2enr = bus.read_u32(RCC_AHB2ENR).expect("read RCC AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, ahb2enr | RCC_AHB2ENR_ADCEN)
        .expect("enable ADC clock");
    bus.write_u32(ADC_CR, 0).expect("leave ADC deep power-down");
    bus.write_u32(ADC_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC_CR, ADC_ON).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixtures carry NTC, diode, and both generic BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all generic netlists must bind through their descriptor-mapped ADC endpoints"
    );
    for channel in [10, 11, 12, 13] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_early.base_v", 0.0)
        .expect("initialize matched BJT base-drive sources");

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut early_samples = Vec::new();
    let mut reference_samples = Vec::new();
    for base_drive_v in [0.0_f64, 0.7, 0.9, 1.1] {
        session
            .set_signal_number("ui.bjt_early.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and both BJT branches");
        assert_eq!(
            steps.len(),
            4,
            "all four analog netlists cross the boundary"
        );
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        let early_v = signal_voltage(&session, "board.analog.pa6_volts");
        let reference_v = signal_voltage(&session, "board.analog.pa7_volts");
        let expected_early_v = bjt_early_effect_collector_voltage(base_drive_v, Some(50.0));
        let expected_reference_v = bjt_early_effect_collector_voltage(base_drive_v, None);

        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the unrelated 25 °C NTC remains at 1.65 V, got {ntc_v} V"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(
            (early_v - expected_early_v).abs() < 5e-5,
            "VAF=50 load line at base drive {base_drive_v} V should be {expected_early_v} V, got {early_v} V"
        );
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "no-forward-Early load line at base drive {base_drive_v} V should be {expected_reference_v} V, got {reference_v} V"
        );
        if base_drive_v >= 0.7 {
            assert!(
                early_v < reference_v - 0.001,
                "finite VAF increases collector current and lowers collector voltage: {early_v} V vs {reference_v} V"
            );
        }

        for (channel, voltage_v) in [(5, ntc_v), (6, diode_v), (11, early_v), (12, reference_v)] {
            let expected_count = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count as u16,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                l5_adc_scan(&mut bus, &[channel]),
                vec![expected_count],
                "ADC1 regular conversion reads the routed device on channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 10),
            u16::MAX,
            "the neighboring ADC1 channel 10 remains isolated"
        );
        assert_eq!(
            adc_channel_count(&mut bus, 13),
            u16::MAX,
            "the neighboring ADC1 channel 13 remains isolated"
        );
        early_samples.push(early_v);
        reference_samples.push(reference_v);
    }
    assert!(
        early_samples[1] > early_samples[2] && early_samples[2] > early_samples[3],
        "increasing base drive lowers the finite-VAF collector voltage: {early_samples:?}"
    );
    assert!(
        reference_samples[1] > reference_samples[2]
            && reference_samples[2] > reference_samples[3],
        "increasing base drive lowers the no-forward-Early collector voltage: {reference_samples:?}"
    );
}

/// Route the generic Gummel–Poon high-injection pair through ADC1 beside the
/// unrelated NTC divider and Shockley diode.
#[test]
fn nucleo_l552ze_q_bjt_high_injection_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-l552ze-q.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-L552ZE-Q");
    let generic = SystemManifest::from_file(root(
        "examples/nucleo-l552ze-q/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let high_injection = SystemManifest::from_file(root(
        "examples/nucleo-l552ze-q/analog-bjt-high-injection-adc-system.yaml",
    ))
    .expect("load matched generic Gummel–Poon high-injection branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(high_injection.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32L552 descriptor");
    for (pad, channel) in [("PA0", 5), ("PA1", 6), ("PB0", 15), ("PB1", 16)] {
        assert_eq!(
            chip.analog_pins
                .get(pad)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pad} resolves through ST-data analog_pins to ADC1_IN{channel}"
        );
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-L552ZE-Q bus");
    bus.write_u32(ADC_CR, 0)
        .expect("write ADC1 while unclocked");
    bus.write_u32(ADC_CR, 1 << 28)
        .expect("write ADC1 while unclocked");
    bus.write_u32(ADC_CR, ADC_ON)
        .expect("write ADC1 while unclocked");
    assert_eq!(
        bus.read_u32(ADC_ISR).unwrap() & 1,
        0,
        "ADC1 ignores ADEN until RCC_AHB2ENR.ADCEN clocks it"
    );
    let ahb2enr = bus.read_u32(RCC_AHB2ENR).expect("read RCC AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, ahb2enr | RCC_AHB2ENR_ADCEN)
        .expect("enable ADC clock");
    bus.write_u32(ADC_CR, 0).expect("leave ADC deep power-down");
    bus.write_u32(ADC_CR, 1 << 28)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC_CR, ADC_ON).expect("enable ADC1");
    assert_ne!(bus.read_u32(ADC_ISR).unwrap() & 1, 0, "ADC1 reports ADRDY");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixtures carry NTC, diode, and matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all generic netlists must bind through their descriptor-mapped ADC endpoints"
    );
    for channel in [14, 15, 16, 17] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_high_injection.base_v", 0.0)
        .expect("initialize matched BJT base-drive sources");

    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut high_drive_delta = None;
    for base_drive_v in [0.8_f64, 1.5, 3.3] {
        session
            .set_signal_number("ui.bjt_high_injection.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched BJT branches");
        assert_eq!(steps.len(), 4, "NTC, diode, and both BJT branches solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_sample_v = signal_voltage(&session, "board.analog.pa1_volts");
        let high_v = signal_voltage(&session, "board.analog.pb0_volts");
        let control_v = signal_voltage(&session, "board.analog.pb1_volts");
        let expected_high_v = bjt_high_injection_collector_voltage(base_drive_v, true);
        let expected_control_v = bjt_high_injection_collector_voltage(base_drive_v, false);

        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "unrelated 25 °C NTC remains isolated on ADC1 channel 5"
        );
        assert!(
            (diode_sample_v - diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains isolated on ADC1 channel 6"
        );
        assert!(
            (high_v - expected_high_v).abs() < 5e-5,
            "IKF/IKR/NKF load line at base drive {base_drive_v} V should be {expected_high_v} V, got {high_v} V"
        );
        assert!(
            (control_v - expected_control_v).abs() < 5e-5,
            "matched no-rolloff load line at base drive {base_drive_v} V should be {expected_control_v} V, got {control_v} V"
        );

        for (channel, voltage_v) in [
            (5, ntc_v),
            (6, diode_sample_v),
            (15, high_v),
            (16, control_v),
        ] {
            let expected_count = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count as u16,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                l5_adc_scan(&mut bus, &[channel]),
                vec![expected_count],
                "firmware-visible ADC1 scan follows the generic device on channel {channel}"
            );
        }
        for channel in [14, 17] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                u16::MAX,
                "neighboring ADC1 channel {channel} remains isolated"
            );
        }
        if base_drive_v == 3.3 {
            high_drive_delta = Some(high_v - control_v);
        }
    }

    let delta = high_drive_delta.expect("high-drive sample was recorded");
    assert!(
        delta > 0.05,
        "IKF/NKF rolloff lowers collector current and raises collector voltage: delta {delta} V"
    );
}
