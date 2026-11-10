// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices drive GPIO and explicit ADC1 co-simulation endpoints on
//! the NUCLEO-H563ZI. GPIO threshold ratios are test assumptions, and ADC
//! channel routes do not claim physical pad aliases or silicon parity.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, GpioInputThresholds, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::peripherals::adc::Adc;
use labwired_core::Bus;
use std::path::Path;

const RCC_AHB2ENR: u64 = 0x4402_0C8C;
const GPIOA_BASE: u64 = 0x4202_0000;
const GPIOA_MODER: u64 = GPIOA_BASE;
const GPIOA_IDR: u64 = GPIOA_BASE + 0x10;
const ADC1_BASE: u64 = 0x4202_8000;
const ADC_ISR: u64 = ADC1_BASE;
const ADC_CR: u64 = ADC1_BASE + 0x08;
const ADC_CFGR: u64 = ADC1_BASE + 0x0C;
const ADC_SQR1: u64 = ADC1_BASE + 0x30;
const ADC_DR: u64 = ADC1_BASE + 0x40;
const ADCEN: u32 = 1 << 10;
const ADC_ADVREGEN: u32 = 1 << 28;
const ADC_ADEN: u32 = 1;
const ADC_ADSTART: u32 = 1 << 2;
const ADC_ADCAL: u32 = 1 << 31;
const ADC_ISR_ADRDY: u32 = 1;
const ADC_ISR_EOC_EOS: u32 = (1 << 2) | (1 << 3);

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

/// Independent forward-active Gummel–Poon load line, with optional Early effect.
fn bjt_collector_voltage(base_drive_v: f64, early_voltage_v: Option<f64>) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-15;
    const BF: f64 = 100.0;
    const BR: f64 = 4.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let early_factor = early_voltage_v.map_or(1.0, |vaf| 1.0 - vbc / vaf);
        let transport = IS_A * (forward - reverse) * early_factor;
        let base_emitter = (IS_A / BF) * (forward - 1.0);
        let base_collector = (IS_A / BR) * (reverse - 1.0);
        (transport - base_collector, base_emitter + base_collector)
    };

    let collector_for_vbe = |vbe: f64| {
        let (mut low, mut high) = (0.0, SUPPLY_V);
        for _ in 0..100 {
            let collector_v = (low + high) / 2.0;
            let (collector_current, _) = currents(vbe, collector_v);
            let residual = (SUPPLY_V - collector_v) / COLLECTOR_OHMS - collector_current;
            if residual > 0.0 {
                low = collector_v;
            } else {
                high = collector_v;
            }
        }
        (low + high) / 2.0
    };

    let (mut low, mut high) = (0.0, base_drive_v);
    for _ in 0..100 {
        let vbe = (low + high) / 2.0;
        let collector_v = collector_for_vbe(vbe);
        let (_, base_current) = currents(vbe, collector_v);
        let residual = (base_drive_v - vbe) / BASE_OHMS - base_current;
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe((low + high) / 2.0)
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

/// Independent level-1 NMOS load-line reference for the explicit ADC route.
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
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("H563 profile declares ADC1");
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

fn adc_power_up(bus: &mut SystemBus) {
    let ahb2enr = bus.read_u32(RCC_AHB2ENR).expect("read RCC AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, ahb2enr | ADCEN)
        .expect("clock ADC1");
    assert_eq!(
        bus.read_u32(ADC_CR).unwrap(),
        1 << 29,
        "clocked ADC CR reset has DEEPPWD set"
    );
    bus.write_u32(ADC_CR, 0).expect("leave deep power-down");
    bus.write_u32(ADC_CR, ADC_ADVREGEN)
        .expect("enable ADC voltage regulator");
    bus.write_u32(ADC_CR, ADC_ADVREGEN | ADC_ADCAL)
        .expect("calibrate ADC");
    assert_eq!(bus.read_u32(ADC_CR).unwrap() & ADC_ADCAL, 0, "ADCAL clears");
    bus.write_u32(ADC_CR, ADC_ADVREGEN | ADC_ADEN)
        .expect("enable ADC");
    assert_ne!(
        bus.read_u32(ADC_ISR).unwrap() & ADC_ISR_ADRDY,
        0,
        "ADEN raises ADRDY"
    );
    bus.write_u32(ADC_CFGR, 0)
        .expect("select 12-bit conversion");
}

fn adc_convert(bus: &mut SystemBus, channel: u8) -> u32 {
    bus.write_u32(ADC_SQR1, u32::from(channel) << 6)
        .expect("select ADC1 regular sequence channel");
    bus.write_u32(ADC_ISR, ADC_ISR_EOC_EOS)
        .expect("clear ADC1 completion flags");
    bus.write_u32(ADC_CR, ADC_ADVREGEN | ADC_ADEN | ADC_ADSTART)
        .expect("start ADC1 conversion");
    assert_eq!(
        bus.read_u32(ADC_ISR).unwrap() & ADC_ISR_EOC_EOS,
        ADC_ISR_EOC_EOS,
        "ADC conversion completes with EOC and EOS"
    );
    assert_eq!(
        bus.read_u32(ADC_CR).unwrap() & ADC_ADSTART,
        0,
        "ADSTART self-clears"
    );
    bus.read_u32(ADC_DR).expect("read firmware-visible ADC1 DR")
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

#[test]
fn nucleo_h563zi_ntc_and_diode_netlists_reach_pa0_pa1_gpio_inputs() {
    let system_path = root("configs/systems/nucleo-h563zi-demo.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-H563ZI profile");
    assert_eq!(profile.name, "nucleo-h563zi-demo");
    assert!(profile.external_devices.is_empty());
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "led_green_pb0" && io.peripheral == "gpiob" && io.pin == 0));
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "button_blue_pc13" && io.peripheral == "gpioc" && io.pin == 13));

    let analog =
        SystemManifest::from_file(root("examples/nucleo-h563zi/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("load STM32H563 descriptor");
    // Simulation assumptions only; no H563 input-threshold silicon claim.
    chip.io_voltage_v = Some(3.3);
    chip.gpio_input_thresholds = Some(GpioInputThresholds { vil: 0.3, vih: 0.7 });

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-H563ZI bus");
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA0"),
        Some((GPIOA_IDR, 0)),
        "PA0 must resolve to the modeled GPIOA IDR bit 0"
    );
    assert_eq!(
        SystemBus::resolve_pin_idr_pub(&bus, "PA1"),
        Some((GPIOA_IDR, 1)),
        "PA1 must resolve to the modeled GPIOA IDR bit 1"
    );

    let ahb2enr = bus.read_u32(RCC_AHB2ENR).expect("read RCC AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, ahb2enr | 1)
        .expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_MODER).expect("read GPIOA mode");
    bus.write_u32(GPIOA_MODER, moder & !0xF)
        .expect("configure PA0/PA1 as digital inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to separate modeled PA0/PA1 GPIO inputs"
    );

    let expected_diode_v = diode_load_line_voltage();
    let vil_v = 0.3 * 3.3;
    assert!(expected_diode_v < vil_v, "diode must fall below test VIL");
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // hold high while the input sits inside the assumed band
        (80.0, false),
        (25.0, false), // hold low while it returns to the assumed band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(
            steps.len(),
            2,
            "both netlists cross the co-simulation boundary"
        );
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
        assert_eq!(gpioa_idr & 2, 0, "PA1 must read low below the assumed VIL");
    }
}

#[test]
fn nucleo_h563zi_bjt_early_effect_reaches_pa2_pa3_gpio_inputs() {
    let system_path = root("configs/systems/nucleo-h563zi-demo.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-H563ZI profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-h563zi/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC divider and Shockley diode");
    let bjt = SystemManifest::from_file(root(
        "examples/nucleo-h563zi/analog-bjt-early-effect-system.yaml",
    ))
    .expect("load matched generic Gummel–Poon NPN branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("load STM32H563 descriptor");
    // Simulation assumptions only; no H563 input-threshold silicon claim.
    chip.io_voltage_v = Some(3.3);
    chip.gpio_input_thresholds = Some(GpioInputThresholds { vil: 0.3, vih: 0.7 });

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-H563ZI bus");
    for (pad, bit) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(
            SystemBus::resolve_pin_idr_pub(&bus, pad),
            Some((GPIOA_IDR, bit)),
            "{pad} must resolve to its modeled GPIOA IDR bit"
        );
    }
    let ahb2enr = bus.read_u32(RCC_AHB2ENR).expect("read RCC AHB2ENR");
    bus.write_u32(RCC_AHB2ENR, ahb2enr | 1)
        .expect("enable GPIOA clock");
    let moder = bus.read_u32(GPIOA_MODER).expect("read GPIOA mode");
    bus.write_u32(GPIOA_MODER, moder & !0xFF)
        .expect("configure PA0-PA3 as digital inputs");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched BJT models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must route independently to PA0 through PA3"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", -10.0)
        .expect("set independent NTC source");
    session
        .set_signal_number("ui.bjt.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    let vil_v = 0.3 * 3.3;
    let vih_v = 0.7 * 3.3;
    let mut early_high = false;
    let mut reference_high = false;
    let mut cycles = 0;
    for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the unrelated and matched device netlists");
        assert_eq!(steps.len(), 4, "all four analog branches solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.gpio_in.pa0");
        let diode_v = signal_voltage(&session, "board.gpio_in.pa1");
        let early_v = signal_voltage(&session, "board.gpio_in.pa2");
        let reference_v = signal_voltage(&session, "board.gpio_in.pa3");
        let ntc_resistance = ntc_resistance_ohms(-10.0);
        let expected_ntc_v = 3.3 * ntc_resistance / (10_000.0 + ntc_resistance);
        let expected_diode_v = diode_load_line_voltage();
        let expected_early_v = bjt_collector_voltage(base_drive_v, Some(50.0));
        let expected_reference_v = bjt_collector_voltage(base_drive_v, None);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "unrelated NTC voltage is stable"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated diode load line is stable"
        );
        assert!(
            (early_v - expected_early_v).abs() < 5e-5,
            "VAF=50 Gummel–Poon load line at base drive {base_drive_v} V should be {expected_early_v} V, got {early_v} V"
        );
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "no-Early-effect Gummel–Poon load line at base drive {base_drive_v} V should be {expected_reference_v} V, got {reference_v} V"
        );
        if base_drive_v >= 0.7 {
            assert!(
                early_v < reference_v - 0.005,
                "finite VAF increases forward collector current and lowers collector voltage: {early_v} V vs {reference_v} V"
            );
        }

        if early_v >= vih_v {
            early_high = true;
        } else if early_v <= vil_v {
            early_high = false;
        }
        if reference_v >= vih_v {
            reference_high = true;
        } else if reference_v <= vil_v {
            reference_high = false;
        }
        let idr = bus.read_u32(GPIOA_IDR).expect("read GPIOA input levels");
        assert_ne!(idr & 1, 0, "the cold NTC remains above assumed VIH on PA0");
        assert_eq!(
            idr & 2,
            0,
            "the Shockley diode remains below assumed VIL on PA1"
        );
        assert_eq!(
            idr & 4 != 0,
            early_high,
            "PA2 follows the Early-effect collector voltage"
        );
        assert_eq!(
            idr & 8 != 0,
            reference_high,
            "PA3 follows the no-Early-effect collector voltage"
        );
    }
}

#[test]
fn nucleo_h563zi_bjt_early_effect_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-h563zi-demo.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-H563ZI profile");
    let generic = SystemManifest::from_file(root(
        "examples/nucleo-h563zi/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode ADC routes");
    let bjt = SystemManifest::from_file(root(
        "examples/nucleo-h563zi/analog-bjt-early-effect-adc-system.yaml",
    ))
    .expect("load matched generic BJT Early-effect branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32H563 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-H563ZI bus");
    adc_power_up(&mut bus);

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixtures carry NTC, diode, and matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must bind to independent ADC1 channel endpoints"
    );
    for channel in 0..=3 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an injected analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_early.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut saw_early_effect = false;
    for base_drive_v in [0.7_f64, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt_early.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched BJT branches");
        assert_eq!(steps.len(), 4, "all four generic analog branches solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        assert!(
            (signal_voltage(&session, "adc.adc1.0_volts") - ntc_v).abs() < 1e-9,
            "unrelated Beta NTC remains isolated on ADC1 channel 0"
        );
        assert!(
            (signal_voltage(&session, "adc.adc1.1_volts") - diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains isolated on ADC1 channel 1"
        );

        let early_v = signal_voltage(&session, "adc.adc1.2_volts");
        let reference_v = signal_voltage(&session, "adc.adc1.3_volts");
        let expected_early_v = bjt_collector_voltage(base_drive_v, Some(50.0));
        let expected_reference_v = bjt_collector_voltage(base_drive_v, None);
        assert!(
            (early_v - expected_early_v).abs() < 5e-5,
            "VAF=50 load line at base drive {base_drive_v} V should be {expected_early_v} V, got {early_v} V"
        );
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "no-Early-effect load line at base drive {base_drive_v} V should be {expected_reference_v} V, got {reference_v} V"
        );
        assert!(
            early_v < reference_v - 0.005,
            "finite VAF increases forward collector current and lowers collector voltage: {early_v} V vs {reference_v} V"
        );
        saw_early_effect |= reference_v - early_v > 0.005;

        for (channel, voltage_v) in [(0, ntc_v), (1, diode_v), (2, early_v), (3, reference_v)] {
            let expected = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected as u16,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                adc_convert(&mut bus, channel),
                expected,
                "firmware-visible ADC1 conversion follows the device on channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            u16::MAX,
            "neighboring ADC1 channel 4 remains isolated"
        );
    }
    assert!(
        saw_early_effect,
        "finite VAF's collector-current effect reaches ADC1"
    );
}

#[test]
fn nucleo_h563zi_ntc_diode_and_mosfet_netlists_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-h563zi-demo.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-H563ZI profile");
    let adc_analog = SystemManifest::from_file(root(
        "examples/nucleo-h563zi/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load generic NTC and diode ADC routes");
    let mosfet =
        SystemManifest::from_file(root("examples/nucleo-h563zi/analog-mosfet-adc-system.yaml"))
            .expect("load generic level-1 NMOS route");
    profile.cosim_models = adc_analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32H563 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-H563ZI bus");
    adc_power_up(&mut bus);

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixtures carry NTC, diode, and MOSFET models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must bind to three explicit ADC1 channel endpoints"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 3),
        u16::MAX,
        "the neighboring ADC1 channel begins undriven"
    );

    let expected_diode_v = diode_load_line_voltage();
    let expected_ntc_v = 1.65;
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent NTC temperature");
    let mut cycles = 0;
    let mut mosfet_samples = Vec::new();
    for gate_v in [0.0_f64, 0.8, 1.0, 2.0, 3.3] {
        session
            .set_signal_number("ui.mosfet.gate_v", gate_v)
            .expect("set MOSFET gate voltage");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all three analog devices");
        assert_eq!(steps.len(), 3, "all three netlists solve at each boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let drain_v = signal_voltage(&session, "adc.adc1.2_volts");
        let expected_drain_v = mosfet_drain_voltage(gate_v);
        assert!(
            (drain_v - expected_drain_v).abs() < 5e-5,
            "level-1 NMOS load line at Vgs={gate_v} V should be {expected_drain_v} V, got {drain_v} V"
        );
        assert!(
            (signal_voltage(&session, "adc.adc1.0_volts") - expected_ntc_v).abs() < 1e-9,
            "independent Beta NTC remains on ADC1 channel 0"
        );
        assert!(
            (signal_voltage(&session, "adc.adc1.1_volts") - expected_diode_v).abs() < 2e-5,
            "independent Shockley diode remains on ADC1 channel 1"
        );

        for (channel, volts) in [(0, expected_ntc_v), (1, expected_diode_v), (2, drain_v)] {
            let expected = expected_adc12_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected as u16,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                adc_convert(&mut bus, channel),
                expected,
                "modeled ADC1 data-register conversion follows its analog device on channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 3),
            u16::MAX,
            "adjacent ADC1 channel remains isolated"
        );
        mosfet_samples.push(drain_v);
    }
    assert!(
        mosfet_samples
            .windows(2)
            .all(|pair| pair[1] <= pair[0] + 1e-9),
        "MOSFET drain voltage cannot rise as gate drive increases"
    );
    assert!(
        *mosfet_samples.last().unwrap() < mosfet_samples[0] - 0.1,
        "the conducting MOSFET must pull its drain below the cutoff level"
    );
}

#[test]
fn nucleo_h563zi_bjt_base_recombination_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-h563zi-demo.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-H563ZI profile");
    let generic = SystemManifest::from_file(root(
        "examples/nucleo-h563zi/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode ADC routes");
    let recombination = SystemManifest::from_file(root(
        "examples/nucleo-h563zi/analog-bjt-recombination-adc-system.yaml",
    ))
    .expect("load matched generic BJT recombination branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(recombination.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32H563 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-H563ZI bus");
    adc_power_up(&mut bus);

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixtures carry NTC, diode, and four BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must bind to independent ADC1 channel endpoints"
    );
    for channel in 0..=5 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an injected analog source"
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
            (signal_voltage(&session, "adc.adc1.0_volts") - ntc_v).abs() < 1e-9,
            "unrelated Beta NTC remains isolated on ADC1 channel 0"
        );
        assert!(
            (signal_voltage(&session, "adc.adc1.1_volts") - diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains isolated on ADC1 channel 1"
        );

        let bjt_voltages = [
            signal_voltage(&session, "adc.adc1.2_volts"),
            signal_voltage(&session, "adc.adc1.3_volts"),
            signal_voltage(&session, "adc.adc1.4_volts"),
            signal_voltage(&session, "adc.adc1.5_volts"),
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
                "ADC1 BJT branch {index} at base drive {base_drive_v} V should match independent load line {expected_v} V, got {voltage} V"
            );
        }
        assert!(
            bjt_voltages[1] > bjt_voltages[0] + 0.005,
            "ISE/NE base-emitter recombination loads the base drive and raises collector voltage"
        );
        saw_base_emitter_effect |= bjt_voltages[1] - bjt_voltages[0] > 0.005;
        saw_base_collector_effect |= bjt_voltages[2] - bjt_voltages[0] > 0.1;

        for (channel, voltage_v) in [(0, ntc_v), (1, diode_v)]
            .into_iter()
            .chain((2..=5).zip(bjt_voltages))
        {
            let expected_count = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count as u16,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                adc_convert(&mut bus, channel),
                expected_count,
                "modeled ADC1 data register follows the generic device on channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 6),
            u16::MAX,
            "neighboring ADC1 channel 6 remains isolated"
        );
    }
    assert!(saw_base_emitter_effect, "ISE/NE effect reaches ADC1");
    assert!(
        saw_base_collector_effect,
        "ISC/NC forward base-collector recombination changes the ADC1 load line"
    );
}

#[test]
fn nucleo_h563zi_bjt_high_injection_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-h563zi-demo.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-H563ZI profile");
    let generic = SystemManifest::from_file(root(
        "examples/nucleo-h563zi/analog-ntc-diode-adc-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode ADC routes");
    let high_injection = SystemManifest::from_file(root(
        "examples/nucleo-h563zi/analog-bjt-high-injection-adc-system.yaml",
    ))
    .expect("load matched generic Gummel–Poon high-injection and control branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(high_injection.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32H563 descriptor");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-H563ZI bus");
    adc_power_up(&mut bus);

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixtures carry NTC, diode, and matched BJT high-injection branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must bind to independent ADC1 channel endpoints"
    );
    for channel in 0..=3 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an injected analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_high_injection.base_v", 0.0)
        .expect("initialize matched BJT base-drive sources");

    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut rolloff_at_high_drive = None;
    let mut control_at_high_drive = None;
    for base_drive_v in [0.8_f64, 1.5, 3.3] {
        session
            .set_signal_number("ui.bjt_high_injection.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched BJT branches");
        assert_eq!(steps.len(), 4, "all four generic analog branches solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        assert!(
            (signal_voltage(&session, "adc.adc1.0_volts") - ntc_v).abs() < 1e-9,
            "unrelated Beta NTC remains isolated on ADC1 channel 0"
        );
        assert!(
            (signal_voltage(&session, "adc.adc1.1_volts") - diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains isolated on ADC1 channel 1"
        );

        let rolloff_v = signal_voltage(&session, "adc.adc1.2_volts");
        let control_v = signal_voltage(&session, "adc.adc1.3_volts");
        let expected_rolloff_v = bjt_high_injection_collector_voltage(base_drive_v, true);
        let expected_control_v = bjt_high_injection_collector_voltage(base_drive_v, false);
        assert!(
            (rolloff_v - expected_rolloff_v).abs() < 5e-5,
            "IKF/NKF load line at base drive {base_drive_v} V should be {expected_rolloff_v} V, got {rolloff_v} V"
        );
        assert!(
            (control_v - expected_control_v).abs() < 5e-5,
            "matched no-rolloff load line at base drive {base_drive_v} V should be {expected_control_v} V, got {control_v} V"
        );

        for (channel, voltage_v) in [(0, ntc_v), (1, diode_v), (2, rolloff_v), (3, control_v)] {
            let expected_count = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count as u16,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                adc_convert(&mut bus, channel),
                expected_count,
                "firmware-visible ADC1 data register follows the generic device on channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            u16::MAX,
            "neighboring ADC1 channel 4 remains isolated"
        );

        if base_drive_v == 3.3 {
            rolloff_at_high_drive = Some(rolloff_v);
            control_at_high_drive = Some(control_v);
        }
    }

    let rolloff_v = rolloff_at_high_drive.expect("high-drive rolloff sample was recorded");
    let control_v = control_at_high_drive.expect("high-drive control sample was recorded");
    assert!(
        rolloff_v - control_v > 0.05,
        "IKF/NKF rolloff lowers collector current and raises collector voltage: {rolloff_v} V vs {control_v} V"
    );
}
