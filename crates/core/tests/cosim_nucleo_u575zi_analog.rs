// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA netlists resolve through the U575 PA0/PA1 analog-pad map into
//! ADC1 channels 5/6, then convert through ADC1's firmware-visible registers.

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

/// Independent Gummel–Poon collector load line with optional forward Early effect.
fn bjt_early_effect_collector_voltage(base_drive_v: f64, vaf_v: Option<f64>) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const BASE_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 80.0;
    const BR: f64 = 4.0;
    const VAR_V: f64 = 75.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let base_width = 1.0 - vaf_v.map_or(0.0, |vaf| vbc / vaf) - vbe / VAR_V;
        let transport = IS_A * (forward - reverse) * base_width;
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
        if (base_drive_v - vbe) / BASE_OHMS > base_current {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe(0.5 * (low + high))
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
        .expect("U575 profile declares ADC1");
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

/// Independent ngspice level-1 base spreading resistance interpolation.
fn bjt_base_resistance_ohms(base_current_a: f64, rb_ohms: f64, rbm_ohms: f64, irb_a: f64) -> f64 {
    let ratio = (base_current_a / irb_a).max(1e-9);
    let z = ((1.0 + 14.59025 * ratio).sqrt() - 1.0) / (2.4317 * ratio.sqrt());
    let tan_z = z.tan();
    rbm_ohms + 3.0 * (rb_ohms - rbm_ohms) * (tan_z - z) / (z * tan_z * tan_z)
}

/// Independent NPN Gummel–Poon load line with optional RBM/IRB rolloff.
fn bjt_base_resistance_collector_voltage(base_drive_v: f64, rolloff: bool) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const COLLECTOR_OHMS: f64 = 1_000.0;
    const DRIVE_OHMS: f64 = 2_000.0;
    const RB_OHMS: f64 = 2_000.0;
    const RBM_OHMS: f64 = 20.0;
    const IRB_A: f64 = 50e-6;
    const IS_A: f64 = 1e-16;
    const BF: f64 = 20.0;
    const BR: f64 = 4.0;
    const VAF: f64 = 50.0;
    const VAR: f64 = 75.0;
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    let currents = |vbe: f64, collector_v: f64| {
        let vbc = vbe - collector_v;
        let forward = (vbe / VT).exp();
        let reverse = (vbc / VT).exp();
        let base_width = 1.0 - vbc / VAF - vbe / VAR;
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
        let base_resistance = if rolloff {
            bjt_base_resistance_ohms(base_current, RB_OHMS, RBM_OHMS, IRB_A)
        } else {
            RB_OHMS
        };
        let residual = (base_drive_v - vbe) / (DRIVE_OHMS + base_resistance) - base_current;
        if residual > 0.0 {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe(0.5 * (low + high))
}

/// Independent DC Gummel–Poon load line for the BJT charge comparison fixture.
fn bjt_charge_collector_voltage(base_drive_v: f64) -> f64 {
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
fn nucleo_u575zi_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-u575zi.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-U575ZI-Q profile");
    assert_eq!(profile.name, "nucleo-u575zi");
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "led_green_ld1" && io.peripheral == "gpioc" && io.pin == 7));
    assert!(profile
        .board_io
        .iter()
        .any(|io| io.id == "button_user_pc13" && io.peripheral == "gpioc" && io.pin == 13));

    let analog =
        SystemManifest::from_file(root("examples/nucleo-u575zi/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32U575 descriptor");
    assert_eq!(
        chip.analog_pins
            .get("PA0")
            .map(|route| (route.peripheral.as_str(), route.channel)),
        Some(("adc1", 5)),
        "PA0 must resolve to its documented ADC1 input channel 5"
    );
    assert_eq!(
        chip.analog_pins
            .get("PA1")
            .map(|route| (route.peripheral.as_str(), route.channel)),
        Some(("adc1", 6)),
        "PA1 must resolve to its documented ADC1 input channel 6"
    );
    let mut bus =
        SystemBus::from_config(&chip, &profile).expect("build NUCLEO-U575ZI-Q system bus");

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
    bus.write_u32(ADC_PCSEL, (1 << 5) | (1 << 6))
        .expect("select both routed ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both generic analog netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic models must resolve PA0/PA1 to ADC1 channels 5/6"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 7),
        u16::MAX,
        "adjacent ADC1 channel 7 is undriven"
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

        for (channel, voltage_v) in [(5, ntc_v), (6, diode_v)] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_adc12_count(voltage_v) as u16,
                "PA route must seed ADC1 channel {channel} with the solved voltage"
            );
            assert_eq!(
                u5_adc_convert(&mut bus, channel),
                expected_adc12_count(voltage_v),
                "ADC1 channel {channel} DR must match the routed {voltage_v} V"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 7),
            u16::MAX,
            "channel 7 stays undriven"
        );
    }
}

#[test]
fn nucleo_u575zi_bjt_high_injection_reaches_pa2_pa3_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-u575zi.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-U575ZI-Q profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-u575zi/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and Shockley diode routes");
    let high_injection = SystemManifest::from_file(root(
        "examples/nucleo-u575zi/analog-bjt-high-injection-system.yaml",
    ))
    .expect("load matched generic Gummel–Poon high-injection branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(high_injection.cosim_models);

    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32U575 descriptor");
    for (pin, channel) in [("PA2", 7), ("PA3", 8)] {
        assert_eq!(
            chip.analog_pins
                .get(pin)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pin} must resolve to its documented ADC1 input channel {channel}"
        );
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-U575ZI-Q bus");

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
    bus.write_u32(ADC_PCSEL, (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8))
        .expect("select the NTC, diode, and BJT ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixtures carry NTC, diode, and matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve PA0–PA3 to ADC1 channels 5–8"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 9),
        u16::MAX,
        "channel 9 is undriven"
    );

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
            (signal_voltage(&session, "board.analog.pa0_volts") - ntc_v).abs() < 1e-9,
            "unrelated Beta NTC remains isolated on ADC1 channel 5"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa1_volts") - diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains isolated on ADC1 channel 6"
        );

        let rolloff_v = signal_voltage(&session, "board.analog.pa2_volts");
        let control_v = signal_voltage(&session, "board.analog.pa3_volts");
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

        for (channel, voltage_v) in [(5, ntc_v), (6, diode_v), (7, rolloff_v), (8, control_v)] {
            let expected_count = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count as u16,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                u5_adc_convert(&mut bus, channel),
                expected_count,
                "firmware-visible ADC1 data register follows channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 9),
            u16::MAX,
            "channel 9 stays isolated"
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

#[test]
fn nucleo_u575zi_bjt_base_recombination_reaches_pa4_pa7_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-u575zi.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-U575ZI-Q profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-u575zi/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and Shockley diode routes");
    let recombination = SystemManifest::from_file(root(
        "examples/nucleo-u575zi/analog-bjt-recombination-system.yaml",
    ))
    .expect("load matched generic Gummel–Poon recombination branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(recombination.cosim_models);

    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32U575 descriptor");
    for (pin, channel) in [("PA4", 9), ("PA5", 10), ("PA6", 11), ("PA7", 12)] {
        assert_eq!(
            chip.analog_pins
                .get(pin)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pin} must resolve to its documented ADC1 input channel {channel}"
        );
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-U575ZI-Q bus");

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
    bus.write_u32(
        ADC_PCSEL,
        (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11) | (1 << 12),
    )
    .expect("select the NTC, diode, and four BJT ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixtures carry NTC, diode, and four matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve PA0–PA7 to ADC1 channels 5–12"
    );
    for channel in 9..=13 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without a BJT source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
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

        let routed_ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let routed_diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (routed_ntc_v - ntc_v).abs() < 1e-9,
            "the unrelated NTC stays isolated on ADC1 channel 5"
        );
        assert!(
            (routed_diode_v - diode_v).abs() < 2e-5,
            "the unrelated Shockley diode stays on its independent load line"
        );

        let bjt_voltages = [
            signal_voltage(&session, "board.analog.pa4_volts"),
            signal_voltage(&session, "board.analog.pa5_volts"),
            signal_voltage(&session, "board.analog.pa6_volts"),
            signal_voltage(&session, "board.analog.pa7_volts"),
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

        for (channel, voltage_v) in [(5, routed_ntc_v), (6, routed_diode_v)]
            .into_iter()
            .chain((9..=12).zip(bjt_voltages))
        {
            let expected_count = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count as u16,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                u5_adc_convert(&mut bus, channel),
                expected_count,
                "firmware-visible ADC1 conversion follows the generic device on channel {channel}"
            );
        }
        for channel in [7, 8, 13] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                u16::MAX,
                "unrelated ADC1 channel {channel} remains undriven"
            );
        }
    }
    assert!(
        saw_base_emitter_effect,
        "ISE/NE effect is resolved at the ADC"
    );
    assert!(
        saw_base_collector_effect,
        "forward-biased ISC/NC recombination changes the collector load line"
    );
}

#[test]
fn nucleo_u575zi_bjt_base_resistance_rolloff_reaches_pa2_pa3_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-u575zi.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-U575ZI-Q profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-u575zi/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and Shockley diode routes");
    let base_resistance = SystemManifest::from_file(root(
        "examples/nucleo-u575zi/analog-bjt-base-resistance-system.yaml",
    ))
    .expect("load matched generic BJT base-resistance branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(base_resistance.cosim_models);

    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32U575 descriptor");
    for (pin, channel) in [("PA2", 7), ("PA3", 8)] {
        assert_eq!(
            chip.analog_pins
                .get(pin)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pin} must resolve to its documented ADC1 input channel {channel}"
        );
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-U575ZI-Q bus");

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
    bus.write_u32(ADC_PCSEL, (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8))
        .expect("select NTC, diode, and both BJT ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched BJT base-resistance branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve PA0–PA3 to ADC1 channels 5–8"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 9),
        u16::MAX,
        "ADC1 channel 9 starts undriven"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_base_resistance.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut rolloff_at_high_drive = None;
    let mut control_at_high_drive = None;
    for base_drive_v in [0.8_f64, 1.0, 1.2] {
        session
            .set_signal_number("ui.bjt_base_resistance.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched BJT branches");
        assert_eq!(steps.len(), 4, "all four generic analog branches solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let routed_ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let routed_diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (routed_ntc_v - ntc_v).abs() < 1e-9,
            "the unrelated NTC remains isolated on ADC1 channel 5"
        );
        assert!(
            (routed_diode_v - diode_v).abs() < 2e-5,
            "the unrelated Shockley diode remains isolated on ADC1 channel 6"
        );

        let rolloff_v = signal_voltage(&session, "board.analog.pa2_volts");
        let control_v = signal_voltage(&session, "board.analog.pa3_volts");
        let expected_rolloff_v = bjt_base_resistance_collector_voltage(base_drive_v, true);
        let expected_control_v = bjt_base_resistance_collector_voltage(base_drive_v, false);
        assert!(
            (rolloff_v - expected_rolloff_v).abs() < 5e-5,
            "RBM/IRB load line at base drive {base_drive_v} V should be {expected_rolloff_v} V, got {rolloff_v} V"
        );
        assert!(
            (control_v - expected_control_v).abs() < 5e-5,
            "matched fixed-RB load line at base drive {base_drive_v} V should be {expected_control_v} V, got {control_v} V"
        );

        for (channel, voltage_v) in [
            (5, routed_ntc_v),
            (6, routed_diode_v),
            (7, rolloff_v),
            (8, control_v),
        ] {
            let expected_count = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count as u16,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                u5_adc_convert(&mut bus, channel),
                expected_count,
                "firmware-visible ADC1 conversion follows channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 9),
            u16::MAX,
            "adjacent ADC1 channel 9 stays isolated"
        );

        if base_drive_v == 1.2 {
            rolloff_at_high_drive = Some(rolloff_v);
            control_at_high_drive = Some(control_v);
        }
    }

    let rolloff_v = rolloff_at_high_drive.expect("high-drive rolloff sample was recorded");
    let control_v = control_at_high_drive.expect("high-drive control sample was recorded");
    assert!(
        control_v - rolloff_v > 0.5,
        "the falling RBM/IRB resistance increases base drive and lowers collector voltage: {rolloff_v} V vs {control_v} V"
    );
}

#[test]
fn nucleo_u575zi_bjt_terminal_resistances_match_external_network_through_adc1() {
    let system_path = root("configs/systems/nucleo-u575zi.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-U575ZI-Q profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-u575zi/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and Shockley diode routes");
    let terminal_resistance = SystemManifest::from_file(root(
        "examples/nucleo-u575zi/analog-bjt-terminal-resistance-system.yaml",
    ))
    .expect("load BJT model-card and explicit terminal-resistance branches");
    profile.cosim_models = generic.cosim_models;
    profile
        .cosim_models
        .extend(terminal_resistance.cosim_models);

    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32U575 descriptor");
    for (pin, channel) in [
        ("PA2", 7),
        ("PA3", 8),
        ("PA4", 9),
        ("PA5", 10),
        ("PA6", 11),
        ("PA7", 12),
    ] {
        assert_eq!(
            chip.analog_pins
                .get(pin)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pin} must resolve to documented ADC1 input channel {channel}"
        );
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-U575ZI-Q bus");

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
    bus.write_u32(
        ADC_PCSEL,
        (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11) | (1 << 12),
    )
    .expect("select NTC, diode, and all six BJT terminal channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched BJT terminal branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve PA0-PA7 to ADC1 channels 5-12"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 13),
        u16::MAX,
        "adjacent ADC1 channel 13 starts undriven"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_terminal_resistance.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let terminal_routes = [
        (
            "intrinsic collector",
            "board.analog.pa2_volts",
            7,
            "board.analog.pa5_volts",
            10,
        ),
        (
            "intrinsic base",
            "board.analog.pa3_volts",
            8,
            "board.analog.pa6_volts",
            11,
        ),
        (
            "intrinsic emitter",
            "board.analog.pa4_volts",
            9,
            "board.analog.pa7_volts",
            12,
        ),
    ];
    let mut cycles = 0;
    for base_drive_v in [0.8_f64, 1.0, 1.2] {
        session
            .set_signal_number("ui.bjt_terminal_resistance.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and both terminal-resistance branches");
        assert_eq!(steps.len(), 4, "all four generic analog branches solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let routed_ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let routed_diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (routed_ntc_v - ntc_v).abs() < 1e-9,
            "the unrelated NTC remains isolated on ADC1 channel 5"
        );
        assert!(
            (routed_diode_v - diode_v).abs() < 2e-5,
            "the unrelated Shockley diode remains isolated on ADC1 channel 6"
        );

        let mut channel_voltages = vec![(5, routed_ntc_v), (6, routed_diode_v)];
        for (terminal, card_path, card_channel, external_path, external_channel) in terminal_routes
        {
            let card_voltage = signal_voltage(&session, card_path);
            let external_voltage = signal_voltage(&session, external_path);
            assert!(
                (card_voltage - external_voltage).abs() < 1e-7,
                "model-card RB/RC/RE and equivalent explicit resistors must agree at the intrinsic {terminal}: {card_voltage} V vs {external_voltage} V"
            );
            channel_voltages.push((card_channel, card_voltage));
            channel_voltages.push((external_channel, external_voltage));
        }

        for (channel, voltage_v) in channel_voltages {
            let expected_count = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count as u16,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                u5_adc_convert(&mut bus, channel),
                expected_count,
                "firmware-visible ADC1 conversion follows channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 13),
            u16::MAX,
            "adjacent ADC1 channel 13 stays isolated"
        );
    }
}

#[test]
fn nucleo_u575zi_bjt_junction_and_transit_charge_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-u575zi.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-U575ZI-Q profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-u575zi/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let charge =
        SystemManifest::from_file(root("examples/nucleo-u575zi/analog-bjt-charge-system.yaml"))
            .expect("load matched BJT junction and transit charge netlists");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(charge.cosim_models);

    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32U575 descriptor");
    for (pin, channel) in [
        ("PA0", 5),
        ("PA1", 6),
        ("PA2", 7),
        ("PA3", 8),
        ("PA4", 9),
        ("PA5", 10),
        ("PA6", 11),
    ] {
        assert_eq!(
            chip.analog_pins
                .get(pin)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pin} must resolve to its documented ADC1 input channel {channel}"
        );
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-U575ZI-Q bus");

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
    bus.write_u32(
        ADC_PCSEL,
        (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11),
    )
    .expect("select NTC, diode, and five BJT ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and five matched BJT charge branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve PA0–PA6 to ADC1 channels 5–11"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 12),
        u16::MAX,
        "neighboring ADC1 channel 12 starts undriven"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_charge.base_v", 0.0)
        .expect("initialize matched BJT base-drive sources");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize independent branches before the base-drive step");
    assert_eq!(initial_steps.len(), 7, "all seven generic branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected initial route errors: {initial_errors:?}"
    );

    session
        .set_signal_number("ui.bjt_charge.base_v", 0.8)
        .expect("step the matched BJT base-drive sources");
    let mut samples = Vec::new();
    for step in 0..8_000 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and BJT charge branches");
        assert_eq!(steps.len(), 7, "all seven branches solve at each 1 us step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        let bjt_voltages = [
            signal_voltage(&session, "board.analog.pa2_volts"),
            signal_voltage(&session, "board.analog.pa3_volts"),
            signal_voltage(&session, "board.analog.pa4_volts"),
            signal_voltage(&session, "board.analog.pa5_volts"),
            signal_voltage(&session, "board.analog.pa6_volts"),
        ];
        let expected_dc = bjt_charge_collector_voltage(0.8);
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "unrelated NTC remains isolated"
        );
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "unrelated diode remains at its independent DC load line"
        );
        assert!(
            (bjt_voltages[0] - expected_dc).abs() < 5e-5,
            "uncharged branch must match the independent Gummel–Poon load line {expected_dc} V, got {} V",
            bjt_voltages[0]
        );

        for (label, channel, voltage) in [
            ("NTC", 5, ntc_v),
            ("diode", 6, diode_v),
            ("uncharged BJT", 7, bjt_voltages[0]),
            ("CJE BJT", 8, bjt_voltages[1]),
            ("CJC BJT", 9, bjt_voltages[2]),
            ("TF BJT", 10, bjt_voltages[3]),
            ("combined-charge BJT", 11, bjt_voltages[4]),
        ] {
            let expected = expected_adc12_count(voltage);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected as u16,
                "solved {label} voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                u5_adc_convert(&mut bus, channel),
                expected,
                "firmware-visible ADC1 channel {channel} must report the {label} voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 12),
            u16::MAX,
            "neighboring ADC1 channel 12 remains undriven"
        );

        if step == 0 {
            for (label, voltage) in [
                ("CJE depletion", bjt_voltages[1]),
                ("CJC depletion", bjt_voltages[2]),
                ("TF transit", bjt_voltages[3]),
                ("combined junction and transit", bjt_voltages[4]),
            ] {
                assert!(
                    (voltage - bjt_voltages[0]).abs() > 0.01,
                    "{label} must change the first routed transient: reference={} V, charged={voltage} V",
                    bjt_voltages[0]
                );
            }
        }
        samples.push(bjt_voltages);
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
fn nucleo_u575zi_bjt_early_effect_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-u575zi.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-U575ZI-Q profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-u575zi/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC divider and Shockley diode");
    let early_effect = SystemManifest::from_file(root(
        "examples/nucleo-u575zi/analog-bjt-early-effect-system.yaml",
    ))
    .expect("load matched generic Gummel–Poon Early-effect branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(early_effect.cosim_models);

    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&profile.chip),
    )
    .expect("load STM32U575 descriptor");
    for (pin, channel) in [("PA2", 7), ("PA3", 8)] {
        assert_eq!(
            chip.analog_pins
                .get(pin)
                .map(|route| (route.peripheral.as_str(), route.channel)),
            Some(("adc1", channel)),
            "{pin} must resolve to its documented ADC1 input channel {channel}"
        );
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-U575ZI-Q bus");

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
    bus.write_u32(ADC_PCSEL, (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8))
        .expect("select NTC, diode, and both BJT ADC channels");

    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve PA0–PA3 to ADC1 channels 5–8"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 9),
        u16::MAX,
        "ADC1 channel 9 starts undriven"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_early_effect.base_v", 0.0)
        .expect("initialize matched BJT base-drive sources");

    let ntc_v = 1.65;
    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut early_effect_deltas = Vec::new();
    for base_drive_v in [0.70_f64, 0.78, 0.86] {
        session
            .set_signal_number("ui.bjt_early_effect.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched BJT branches");
        assert_eq!(steps.len(), 4, "all four generic analog branches solve");
        assert!(errors.is_empty(), "unexpected ADC route errors: {errors:?}");

        let routed_ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let routed_diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (routed_ntc_v - ntc_v).abs() < 1e-9,
            "unrelated Beta NTC remains isolated on ADC1 channel 5"
        );
        assert!(
            (routed_diode_v - diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains isolated on ADC1 channel 6"
        );

        let finite_vaf_v = signal_voltage(&session, "board.analog.pa2_volts");
        let control_v = signal_voltage(&session, "board.analog.pa3_volts");
        let expected_finite_vaf_v = bjt_early_effect_collector_voltage(base_drive_v, Some(50.0));
        let expected_control_v = bjt_early_effect_collector_voltage(base_drive_v, None);
        assert!(
            (finite_vaf_v - expected_finite_vaf_v).abs() < 5e-5,
            "VAF=50 load line at base drive {base_drive_v} V should be {expected_finite_vaf_v} V, got {finite_vaf_v} V"
        );
        assert!(
            (control_v - expected_control_v).abs() < 5e-5,
            "matched no-forward-Early load line at base drive {base_drive_v} V should be {expected_control_v} V, got {control_v} V"
        );
        assert!(
            control_v - finite_vaf_v > 0.001,
            "finite forward Early voltage increases collector current and lowers collector voltage: {finite_vaf_v} V vs {control_v} V"
        );
        early_effect_deltas.push(control_v - finite_vaf_v);

        for (channel, voltage_v) in [
            (5, routed_ntc_v),
            (6, routed_diode_v),
            (7, finite_vaf_v),
            (8, control_v),
        ] {
            let expected_count = expected_adc12_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count as u16,
                "solved voltage seeds ADC1 channel {channel}"
            );
            assert_eq!(
                u5_adc_convert(&mut bus, channel),
                expected_count,
                "firmware-visible ADC1 data register follows channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 9),
            u16::MAX,
            "adjacent ADC1 channel 9 stays isolated"
        );
    }

    assert!(
        early_effect_deltas.iter().any(|delta| *delta > 0.02),
        "the finite-VAF branch must show a measurable collector-current increase: {early_effect_deltas:?}"
    );
}
