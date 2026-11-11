// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through STM32F103 board profiles into ADC1.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_BASE: u64 = 0x4002_1000;
const ADC1_BASE: u64 = 0x4001_2400;

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

/// Independent level-1 NMOS load line using the ngspice `mos1temp` laws for
/// KP, PHI, and VTO. Source and bulk are both grounded, so this isolates the
/// model's temperature adjustment from body bias.
fn mosfet_drain_voltage_at_temperature(temperature_c: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const GATE_V: f64 = 3.0;
    const VTO: f64 = 1.0;
    const KP_A_PER_V2: f64 = 20e-6;
    const GAMMA: f64 = 0.4;
    const PHI: f64 = 0.65;
    const W_M: f64 = 100e-6;
    const L_M: f64 = 10e-6;
    const LAMBDA_PER_V: f64 = 0.02;
    const TNOM_C: f64 = 25.0;

    let bandgap = |temperature_k: f64| {
        1.16 - 7.02e-4 * temperature_k * temperature_k / (temperature_k + 1108.0)
    };
    let kelvin = temperature_c + 273.15;
    let nominal_kelvin = TNOM_C + 273.15;
    let ratio = kelvin / nominal_kelvin;
    let vt = 1.380_649e-23 * kelvin / 1.602_176_634e-19;
    let nominal_vt = 1.380_649e-23 * nominal_kelvin / 1.602_176_634e-19;
    let phi_t = PHI * ratio
        - vt * (3.0 * ratio.ln() + bandgap(nominal_kelvin) / nominal_vt - bandgap(kelvin) / vt);
    let vto_t = VTO
        + GAMMA * (phi_t.sqrt() - PHI.sqrt())
        + 0.5 * (phi_t - PHI)
        + 0.5 * (bandgap(nominal_kelvin) - bandgap(kelvin));
    let kp_t = KP_A_PER_V2 / ratio.powf(1.5);
    let overdrive = GATE_V - vto_t;
    let beta = kp_t * W_M / L_M;
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

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("STM32F103RB declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<labwired_core::peripherals::adc::Adc>()
        .expect("ADC1 uses the shared STM32 ADC model")
        .channel_input_count(channel)
}

/// Perform the selected one-shot through the STM32F1 register interface firmware uses.
fn stm32f103_adc_convert(bus: &mut SystemBus, channel: u8) -> u32 {
    const ADC1EN: u32 = 1 << 9;
    const ADON: u32 = 1;
    const EXTSEL_SOFTWARE: u32 = 0b111 << 17;
    const EXTTRIG: u32 = 1 << 20;
    const SWSTART: u32 = 1 << 22;
    const EOC: u32 = 1 << 1;

    let apb2enr = bus.read_u32(RCC_BASE + 0x18).expect("read RCC APB2ENR");
    bus.write_u32(RCC_BASE + 0x18, apb2enr | ADC1EN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_BASE + 0x34, u32::from(channel))
        .expect("select SQR3 SQ1 channel");
    let trigger = ADON | EXTSEL_SOFTWARE | EXTTRIG;
    bus.write_u32(ADC1_BASE + 0x08, trigger)
        .expect("power up ADC1 and enable software trigger");
    bus.write_u32(ADC1_BASE + 0x08, trigger | SWSTART)
        .expect("start ADC1 conversion");
    for _ in 0..64 {
        bus.tick_peripherals_fully();
    }
    let status = bus.read_u32(ADC1_BASE).expect("read ADC1 SR");
    assert_ne!(status & EOC, 0, "ADC1 conversion completes with EOC set");
    bus.read_u32(ADC1_BASE + 0x4c)
        .expect("read firmware-visible ADC1 DR")
}

fn expected_adc_count(voltage_v: f64) -> u16 {
    let millivolts = (voltage_v * 1000.0).round() as u32;
    (millivolts * 4095 / 3300).min(4095) as u16
}

/// Independent Ward–Dutton drain terminal charge from the level-1 charge
/// sheet. Simpson integration avoids using the solver's analytic expression
/// as its own oracle.
fn ward_dutton_drain_charge(gate_v: f64, drain_v: f64) -> f64 {
    const WIDTH_M: f64 = 100e-6;
    const LENGTH_M: f64 = 10e-6;
    const TOX_M: f64 = 10e-9;
    const VTO_V: f64 = 1.0;
    const EPSILON_OX_F_PER_M: f64 = 3.453_133e-11;
    const INTERVALS: usize = 4000;

    let overdrive = (gate_v - VTO_V).max(0.0);
    if overdrive == 0.0 {
        return 0.0;
    }
    let channel_drop = drain_v.clamp(0.0, overdrive);
    let denominator = overdrive * channel_drop - 0.5 * channel_drop * channel_drop;
    let mut integral = 0.0;
    for point in 0..=INTERVALS {
        let position = point as f64 / INTERVALS as f64;
        let local_charge = (overdrive * overdrive - 2.0 * position * denominator).sqrt();
        let weight = if point == 0 || point == INTERVALS {
            1.0
        } else if point % 2 == 0 {
            2.0
        } else {
            4.0
        };
        integral += weight * position * local_charge;
    }
    let cox = EPSILON_OX_F_PER_M * WIDTH_M * LENGTH_M / TOX_M;
    -cox * integral / (3.0 * INTERVALS as f64)
}

fn ward_dutton_drain_voltage(
    gate_v: f64,
    previous_gate_v: f64,
    previous_drain_v: f64,
    resistance_ohms: f64,
    step_s: f64,
) -> f64 {
    let previous_charge = ward_dutton_drain_charge(previous_gate_v, previous_drain_v);
    let residual = |drain_v: f64| {
        drain_v / resistance_ohms
            + (ward_dutton_drain_charge(gate_v, drain_v) - previous_charge) / step_s
    };
    let (mut low, mut high) = (0.0, 3.3);
    assert!(residual(low) <= 0.0 && residual(high) >= 0.0);
    for _ in 0..80 {
        let middle = 0.5 * (low + high);
        if residual(middle) < 0.0 {
            low = middle;
        } else {
            high = middle;
        }
    }
    0.5 * (low + high)
}

/// Independent level-1 NMOS load line with external drain/source resistance.
/// It treats model-card RSH × NRD/NRS and explicit resistors as the same series
/// path around the intrinsic channel, including the source-resistor VGS drop.
fn mosfet_drain_voltage(gate_v: f64, drain_ohms: f64, source_ohms: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 10_000.0;
    const VTO: f64 = 1.0;
    const KP_A_PER_V2: f64 = 20e-6;
    const W_M: f64 = 100e-6;
    const L_M: f64 = 10e-6;
    const LAMBDA_PER_V: f64 = 0.02;

    let beta = KP_A_PER_V2 * W_M / L_M;
    let channel_current = |current_a: f64| {
        let vds = SUPPLY_V - current_a * (LOAD_OHMS + drain_ohms + source_ohms);
        let overdrive = gate_v - current_a * source_ohms - VTO;
        if overdrive <= 0.0 || vds <= 0.0 {
            0.0
        } else if vds < overdrive {
            beta * (1.0 + LAMBDA_PER_V * vds) * vds * (overdrive - 0.5 * vds)
        } else {
            0.5 * beta * (1.0 + LAMBDA_PER_V * vds) * overdrive * overdrive
        }
    };

    let (mut low, mut high) = (0.0, SUPPLY_V / (LOAD_OHMS + drain_ohms + source_ohms));
    for _ in 0..100 {
        let current_a = (low + high) / 2.0;
        if channel_current(current_a) > current_a {
            low = current_a;
        } else {
            high = current_a;
        }
    }
    SUPPLY_V - ((low + high) / 2.0) * LOAD_OHMS
}

/// Extend the basic NUCLEO-F103RB NTC route with a Shockley diode and matched
/// Gummel–Poon branches to exercise generic netlist routing and ADC conversion.
#[test]
fn nucleo_f103rb_bjt_early_effect_reaches_pa2_pa3_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-f103rb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F103RB");
    assert_eq!(profile.name, "nucleo-f103rb");
    assert_eq!(
        profile
            .board_io
            .iter()
            .map(|io| (io.id.as_str(), io.peripheral.as_str(), io.pin))
            .collect::<Vec<_>>(),
        [("led_ld2", "gpioa", 5), ("button_b1", "gpioc", 13)],
        "the analog overlay preserves the board's user LED and button"
    );

    let generic = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-ntc-diode-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let bjt = SystemManifest::from_file(root(
        "examples/nucleo-f103rb/analog-bjt-early-effect-system.yaml",
    ))
    .expect("load matched generic Gummel–Poon NPN branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F103RB descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
        assert_eq!(chip.analog_pins[pad].peripheral, "adc1");
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F103RB bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries NTC, diode, and matched BJT netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all generic devices must resolve through PA0..PA3 ADC routes"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC to nominal temperature");
    session
        .set_signal_number("ui.bjt.base_v", 0.0)
        .expect("initialize matched BJT base drives");
    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut early_samples = Vec::new();
    let mut reference_samples = Vec::new();

    for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all generic analog netlists");
        assert_eq!(steps.len(), 4, "all four netlists cross the time boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        let early_v = signal_voltage(&session, "board.analog.pa2_volts");
        let reference_v = signal_voltage(&session, "board.analog.pa3_volts");
        let expected_early_v = bjt_collector_voltage(base_drive_v, Some(50.0));
        let expected_reference_v = bjt_collector_voltage(base_drive_v, None);

        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the unrelated 25 °C NTC divider remains at {expected_ntc_v} V, got {ntc_v} V"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(
            (early_v - expected_early_v).abs() < 5e-5,
            "VAF=50 Gummel–Poon load line at base drive {base_drive_v} V should be {expected_early_v} V, got {early_v} V"
        );
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "no-Early-effect load line at base drive {base_drive_v} V should be {expected_reference_v} V, got {reference_v} V"
        );
        if base_drive_v >= 0.7 {
            assert!(
                early_v < reference_v - 0.005,
                "finite VAF increases collector current and lowers collector voltage: {early_v} V vs {reference_v} V"
            );
        }

        for (channel, volts) in [(0, ntc_v), (1, diode_v), (2, early_v), (3, reference_v)] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f103_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1 channel {channel} must report its solved voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        early_samples.push(early_v);
        reference_samples.push(reference_v);
    }

    assert!(
        early_samples[1] > early_samples[2] && early_samples[2] > early_samples[3],
        "increasing base drive lowers the Early-effect collector voltage: {early_samples:?}"
    );
    assert!(
        reference_samples[1] > reference_samples[2] && reference_samples[2] > reference_samples[3],
        "increasing base drive lowers the reference collector voltage: {reference_samples:?}"
    );
}

/// The same generic Gummel–Poon Early-effect pair also resolves through the
/// bare STM32F103C8 profile's descriptor-driven ADC routes, alongside unrelated
/// Beta NTC and Shockley diode netlists.
#[test]
fn stm32f103_bare_bjt_early_effect_reaches_pa2_pa3_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/stm32f103-bare.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load bare STM32F103");
    assert_eq!(profile.name, "stm32f103-bare");
    assert!(profile.external_devices.is_empty());
    assert!(profile.board_io.is_empty());

    let generic = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-ntc-diode-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let bjt = SystemManifest::from_file(root(
        "examples/nucleo-f103rb/analog-bjt-early-effect-system.yaml",
    ))
    .expect("load matched generic Gummel–Poon NPN branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F103 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
        assert_eq!(chip.analog_pins[pad].peripheral, "adc1");
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build bare STM32F103 bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries NTC, diode, and matched BJT netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all generic devices must resolve through PA0..PA3 ADC routes"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC to nominal temperature");
    session
        .set_signal_number("ui.bjt.base_v", 0.0)
        .expect("initialize matched BJT base drives");
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut early_samples = Vec::new();
    let mut reference_samples = Vec::new();

    for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all generic analog netlists");
        assert_eq!(steps.len(), 4, "all four netlists cross the time boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        let early_v = signal_voltage(&session, "board.analog.pa2_volts");
        let reference_v = signal_voltage(&session, "board.analog.pa3_volts");
        let expected_early_v = bjt_collector_voltage(base_drive_v, Some(50.0));
        let expected_reference_v = bjt_collector_voltage(base_drive_v, None);

        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the unrelated 25 °C NTC divider remains at 1.65 V, got {ntc_v} V"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );
        assert!(
            (early_v - expected_early_v).abs() < 5e-5,
            "VAF=50 Gummel–Poon load line at base drive {base_drive_v} V should be {expected_early_v} V, got {early_v} V"
        );
        assert!(
            (reference_v - expected_reference_v).abs() < 5e-5,
            "no-Early-effect load line at base drive {base_drive_v} V should be {expected_reference_v} V, got {reference_v} V"
        );
        if base_drive_v >= 0.7 {
            assert!(
                early_v < reference_v - 0.005,
                "finite VAF increases collector current and lowers collector voltage: {early_v} V vs {reference_v} V"
            );
        }

        for (channel, volts) in [(0, ntc_v), (1, diode_v), (2, early_v), (3, reference_v)] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f103_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1 channel {channel} must report its solved voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        early_samples.push(early_v);
        reference_samples.push(reference_v);
    }

    assert!(
        early_samples[1] > early_samples[2] && early_samples[2] > early_samples[3],
        "increasing base drive lowers the Early-effect collector voltage: {early_samples:?}"
    );
    assert!(
        reference_samples[1] > reference_samples[2] && reference_samples[2] > reference_samples[3],
        "increasing base drive lowers the reference collector voltage: {reference_samples:?}"
    );
}

/// Reuse the generic Beta NTC divider and unrelated Shockley diode netlists on
/// each STM32F103 profile, resolving through that profile's own chip routes.
fn assert_stm32f103_generic_analog_profile(
    mut profile: SystemManifest,
    system_path: &Path,
    expected_profile_name: &str,
    required_peripherals: &[&str],
) {
    assert_eq!(profile.name, expected_profile_name);
    let analog = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F103 descriptor");
    assert_eq!(chip.analog_pins["PA0"].channel, 0);
    assert_eq!(chip.analog_pins["PA1"].channel, 1);
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build STM32F103 bus");
    for peripheral in required_peripherals {
        assert!(
            bus.find_peripheral_index_by_name(peripheral).is_some(),
            "the profile-specific {peripheral} peripheral remains attached"
        );
    }
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through this profile's PA0/PA1 ADC routes"
    );
    for channel in [0, 1, 2] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut ntc_samples = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set generic NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both analog netlists");
        assert_eq!(steps.len(), 2, "both netlists cross the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let expected_ntc_v = 3.3 * 10_000.0 / (ntc_resistance_ohms(temperature_c) + 10_000.0);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        for (channel, volts) in [(0, ntc_v), (1, diode_v)] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f103_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1 channel {channel} conversion must report the solved voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 2),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        ntc_samples.push((ntc_v, adc_channel_count(&mut bus, 0)));
    }

    assert!(
        ntc_samples[2].0 > ntc_samples[1].0 && ntc_samples[1].0 > ntc_samples[0].0,
        "the divider voltage rises with temperature: {ntc_samples:?}"
    );
    assert!(
        ntc_samples[2].1 > ntc_samples[1].1 && ntc_samples[1].1 > ntc_samples[0].1,
        "ADC counts track the temperature-dependent voltage: {ntc_samples:?}"
    );
    assert_eq!(
        ntc_samples[1].1, 2047,
        "the 25 °C NTC divider is half scale"
    );
}

#[test]
fn nucleo_f103rb_epaper_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f103rb-epaper.yaml");
    let profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-F103RB e-paper profile");
    assert_eq!(profile.name, "nucleo-f103rb-epaper");
    assert!(profile.board_io.is_empty());
    assert_eq!(profile.external_devices.len(), 1);
    assert_eq!(profile.external_devices[0].id, "epaper");
    assert_eq!(profile.external_devices[0].r#type, "ssd1680_tricolor_290");
    assert_eq!(profile.external_devices[0].connection, "spi1");
    assert_eq!(
        profile.external_devices[0].config["cs_pin"].as_str(),
        Some("PA4"),
        "the analog overlay must retain the panel's chip-select wiring"
    );
    assert_stm32f103_generic_analog_profile(profile, &system_path, "nucleo-f103rb-epaper", &[]);
}

/// The NUCLEO-F103RB e-paper profile routes generic level-1 MOSFET
/// temperature behavior through PA2/PA3 while its unrelated NTC and diode
/// remain on PA0/PA1. The two MOSFETs share one model and bias but carry
/// distinct instance temperatures, so omitting `mos1temp` cannot pass.
#[test]
fn nucleo_f103rb_epaper_mosfet_temperature_reaches_pa2_pa3_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f103rb-epaper.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-F103RB e-paper profile");
    assert_eq!(profile.name, "nucleo-f103rb-epaper");
    assert_eq!(profile.external_devices.len(), 1);
    assert_eq!(profile.external_devices[0].id, "epaper");
    let analog = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-mosfet-temperature-system.yaml",
    ))
    .expect("load generic NTC, diode, and temperature-paired MOSFET netlists");
    profile.cosim_models = analog.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F103 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build e-paper board bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries the generic NTC, diode, and MOSFET models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "each netlist output must resolve through its STM32F103 PA0..PA3 ADC route"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the unrelated NTC to its nominal temperature");
    let cycles = session.cycles_until_boundary(0);
    let (steps, errors) = session
        .advance_to(cycles, &mut bus)
        .expect("solve the generic NTC, diode, and MOSFET netlists");
    assert_eq!(steps.len(), 3, "all three netlists cross the 1 us boundary");
    assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

    let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
    assert!(
        (ntc_v - 1.65).abs() < 1e-9,
        "the NTC divider is half scale: {ntc_v} V"
    );
    let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
    let expected_diode_v = diode_load_line_voltage();
    assert!(
        (diode_v - expected_diode_v).abs() < 2e-5,
        "the unrelated Shockley branch is {expected_diode_v} V, got {diode_v} V"
    );

    let mosfet_samples = [
        (2, 25.0, "board.analog.pa2_volts"),
        (3, 85.0, "board.analog.pa3_volts"),
    ]
    .map(|(channel, temperature_c, path)| {
        let volts = signal_voltage(&session, path);
        let expected = mosfet_drain_voltage_at_temperature(temperature_c);
        assert!(
            (volts - expected).abs() < 1e-3,
            "the {temperature_c} °C MOSFET load line should be {expected} V, got {volts} V"
        );
        let expected_count = expected_adc_count(expected);
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            expected_count,
            "the solved MOSFET drain voltage must seed ADC1 channel {channel}"
        );
        assert_eq!(
            stm32f103_adc_convert(&mut bus, channel),
            u32::from(expected_count),
            "firmware-visible ADC1 channel {channel} must report the MOSFET load line"
        );
        (temperature_c, volts, expected_count)
    });
    assert!(
        mosfet_samples[1].1 > mosfet_samples[0].1 + 0.04,
        "reduced hot-channel mobility must raise the drain voltage: {mosfet_samples:?}"
    );
    assert!(
        mosfet_samples[1].2 > mosfet_samples[0].2 + 40,
        "the hot MOSFET must produce a distinct firmware ADC result: {mosfet_samples:?}"
    );
    for (channel, volts) in [(0, ntc_v), (1, diode_v)] {
        let expected_count = expected_adc_count(volts);
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            expected_count,
            "the unrelated netlist must stay on ADC1 channel {channel}"
        );
        assert_eq!(
            stm32f103_adc_convert(&mut bus, channel),
            u32::from(expected_count),
            "firmware-visible ADC1 channel {channel} must retain its own analog result"
        );
    }
    assert_eq!(
        adc_channel_count(&mut bus, 4),
        0xFFFF,
        "the neighboring ADC1 channel remains undriven"
    );
}

/// The generic level-1 NMOS sheet-resistance path resolves through the e-paper
/// profile's ADC1 pins. Matched explicit resistor branches independently check
/// RSH×NRD and RSH×NRS, while the unrelated NTC and diode stay isolated.
#[test]
fn nucleo_f103rb_epaper_mosfet_sheet_resistance_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-f103rb-epaper.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-F103RB e-paper profile");
    assert_eq!(profile.name, "nucleo-f103rb-epaper");
    assert_eq!(profile.external_devices.len(), 1);
    assert_eq!(profile.external_devices[0].id, "epaper");

    let generic = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC and Shockley diode netlists");
    let sheet = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-mosfet-sheet-resistance-system.yaml",
    ))
    .expect("load generic MOSFET sheet-resistance branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(sheet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F103 descriptor");
    for (pad, channel) in [
        ("PA0", 0),
        ("PA1", 1),
        ("PB0", 8),
        ("PB1", 9),
        ("PC0", 10),
        ("PC1", 11),
        ("PC2", 12),
    ] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
        assert_eq!(chip.analog_pins[pad].peripheral, "adc1");
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build e-paper board bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal co-simulation")
        .expect("profile carries NTC, diode, and five MOSFET branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "each generic device output must resolve through PA0/PA1 and PB0..PC2 ADC1 routes"
    );
    for channel in 0..=13 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("hold the unrelated NTC at nominal temperature");
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    for gate_v in [0.5_f64, 2.5] {
        session
            .set_signal_number("ui.mosfet.gate_v", gate_v)
            .expect("set shared MOSFET gate voltage");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and MOSFET netlists");
        assert_eq!(steps.len(), 3, "all three unrelated netlists solve");
        assert!(
            errors.is_empty(),
            "unexpected ADC1 routing errors: {errors:?}"
        );

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!((ntc_v - 1.65).abs() < 1e-9);
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated diode load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let branches = [
            (8, "board.analog.pb0_volts", 0.0, 0.0),
            (9, "board.analog.pb1_volts", 1_000.0, 0.0),
            (10, "board.analog.pc0_volts", 0.0, 2_000.0),
            (11, "board.analog.pc1_volts", 1_000.0, 0.0),
            (12, "board.analog.pc2_volts", 0.0, 2_000.0),
        ];
        let mut voltages = Vec::new();
        for (channel, path, drain_ohms, source_ohms) in branches {
            let volts = signal_voltage(&session, path);
            let expected = mosfet_drain_voltage(gate_v, drain_ohms, source_ohms);
            assert!(
                (volts - expected).abs() < 5e-5,
                "at VGS={gate_v} V with RD={drain_ohms} Ω and RS={source_ohms} Ω, expected {expected} V on ADC1_IN{channel}, got {volts} V"
            );
            let count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                count,
                "the solved MOSFET voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f103_adc_convert(&mut bus, channel),
                u32::from(count),
                "firmware-visible ADC1 channel {channel} must follow the solved load line"
            );
            voltages.push(volts);
        }
        assert!(
            (voltages[1] - voltages[3]).abs() < 5e-5,
            "drain RSH×NRD must match the equivalent external resistor: {voltages:?}"
        );
        assert!(
            (voltages[2] - voltages[4]).abs() < 5e-5,
            "source RSH×NRS must match the equivalent external resistor: {voltages:?}"
        );
        if gate_v < 1.0 {
            assert!(
                voltages.iter().all(|volts| (volts - 3.3).abs() < 5e-5),
                "all matched MOSFETs remain in cutoff below threshold: {voltages:?}"
            );
        } else {
            assert!(
                (voltages[1] - voltages[0]).abs() > 0.02,
                "drain sheet resistance changes the solved load line: {voltages:?}"
            );
            assert!(
                (voltages[2] - voltages[0]).abs() > 0.02,
                "source sheet resistance changes the solved load line: {voltages:?}"
            );
        }

        for (channel, volts) in [(0, ntc_v), (1, diode_v)] {
            let count = expected_adc_count(volts);
            assert_eq!(adc_channel_count(&mut bus, channel), count);
            assert_eq!(
                stm32f103_adc_convert(&mut bus, channel),
                u32::from(count),
                "the unrelated device remains isolated on ADC1 channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 13),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
    }
}

/// The same unrelated analog devices and F1 ADC route must work on the bare
/// STM32F103C8 profile, which has no board-attached devices to supply a route.
#[test]
fn stm32f103_bare_ntc_and_diode_netlists_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/stm32f103-bare.yaml");
    let profile = SystemManifest::from_file(&system_path).expect("load bare STM32F103");
    assert!(profile.external_devices.is_empty());
    assert!(profile.board_io.is_empty());
    assert_stm32f103_generic_analog_profile(profile, &system_path, "stm32f103-bare", &[]);
}

/// The integrated-test system extends the common STM32F103 chip with DMA1 and
/// a DMA-wired UART1. Its profile must retain those peripherals while using the
/// same descriptor-driven analog routing as the bare F103 target.
#[test]
fn stm32f103_integrated_test_ntc_and_diode_netlists_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/stm32f103-integrated-test.yaml");
    let profile =
        SystemManifest::from_file(&system_path).expect("load integrated STM32F103 test profile");
    assert_eq!(profile.external_devices.len(), 0);
    assert_eq!(profile.board_io.len(), 0);
    assert_eq!(
        profile
            .peripherals
            .iter()
            .map(|peripheral| peripheral.id.as_str())
            .collect::<Vec<_>>(),
        ["dma1", "uart1"],
        "the integrated-test profile keeps its DMA and UART additions"
    );
    assert_stm32f103_generic_analog_profile(
        profile,
        &system_path,
        "stm32f103-integrated-test",
        &["dma1", "uart1"],
    );
}

/// The F103 e-paper profile routes generic MOSFET drain-bulk depletion charge
/// through PA2/PA3 to ADC1. A matched zero-capacitance branch checks the
/// transient independently while the unrelated NTC and diode remain isolated.
#[test]
fn nucleo_f103rb_epaper_mosfet_drain_bulk_charge_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-f103rb-epaper.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-F103RB e-paper profile");
    assert_eq!(profile.external_devices.len(), 1);
    assert_eq!(profile.external_devices[0].id, "epaper");

    let generic = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-ntc-diode-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let mosfet = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-mosfet-junction-charge-system.yaml",
    ))
    .expect("load generic MOSFET bulk-junction charge branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F103 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
        assert_eq!(chip.analog_pins[pad].peripheral, "adc1");
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build e-paper board bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal co-simulation")
        .expect("profile carries NTC, diode, and MOSFET junction-charge netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through PA0..PA3 ADC1 routes"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("hold the unrelated NTC at nominal temperature");
    session
        .set_signal_number("ui.mosfet_bulk.drain_current_a", 0.0)
        .expect("initialize the charged MOSFET branch");
    session
        .set_signal_number("ui.mosfet_bulk.reference_current_a", 0.0)
        .expect("initialize the zero-capacitance reference branch");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle the zero-current MOSFET branches and unrelated devices");
    assert_eq!(initial_steps.len(), 3, "all three analog netlists solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );
    assert!(
        signal_voltage(&session, "board.analog.pa2_volts").abs() < 1e-12
            && signal_voltage(&session, "board.analog.pa3_volts").abs() < 1e-12,
        "both drain nodes start at ground"
    );

    session
        .set_signal_number("ui.mosfet_bulk.drain_current_a", 1e-3)
        .expect("step 1 mA through the drain-bulk junction");
    session
        .set_signal_number("ui.mosfet_bulk.reference_current_a", 1e-3)
        .expect("apply the same current step to the zero-charge reference");
    let expected_diode_v = diode_load_line_voltage();
    let mut expected_charge_v = 0.0;
    for step in 0..12 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both MOSFET transients, NTC, and diode");
        assert_eq!(
            steps.len(),
            3,
            "all three analog netlists solve at each step"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        // CBD=1 nF and Rload=1 kΩ give a 1/2 backward-Euler recurrence at
        // dt=1 us toward the 1 V DC load line. The matched reference has no
        // depletion capacitance and therefore reaches that load line at once.
        expected_charge_v = 0.5 * expected_charge_v + 0.5;
        let charged_v = signal_voltage(&session, "board.analog.pa2_volts");
        let reference_v = signal_voltage(&session, "board.analog.pa3_volts");
        assert!(
            (charged_v - expected_charge_v).abs() < 2e-8,
            "CBD drain transient at step {step}: expected {expected_charge_v} V, got {charged_v} V"
        );
        assert!(
            (reference_v - 1.0).abs() < 2e-8,
            "the zero-charge reference reaches its 1 V DC load line, got {reference_v} V"
        );
        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!((ntc_v - 1.65).abs() < 1e-9);
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated diode remains at its DC load line"
        );

        for (channel, voltage_v) in [(0, ntc_v), (1, diode_v), (2, charged_v), (3, reference_v)] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f103_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 channel {channel} must report {voltage_v} V"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            0xFFFF,
            "the neighboring ADC1 channel stays undriven"
        );
    }
    assert!(
        (signal_voltage(&session, "board.analog.pa2_volts") - 1.0).abs() < 3e-4,
        "drain-bulk depletion charge changes the transient, not the DC load line"
    );
}

/// The board ADC proves the generic Ward–Dutton channel-charge formulation
/// through the same netlist, MNA, and route path used by unrelated devices.
#[test]
fn nucleo_f103rb_epaper_mosfet_ward_dutton_charge_reaches_adc1_alongside_ntc_and_diode() {
    const GATE_DRIVE_V: f64 = 1.5;
    const LOAD_OHMS: f64 = 5e6;
    const STEP_S: f64 = 1e-6;

    let system_path = root("configs/systems/nucleo-f103rb-epaper.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-F103RB e-paper profile");
    let generic = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-ntc-diode-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let mosfet = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-mosfet-ward-dutton-adc-system.yaml",
    ))
    .expect("load matched generic MOSFET channel-charge netlists");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F103 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
        assert_eq!(chip.analog_pins[pad].peripheral, "adc1");
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build e-paper board bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal co-simulation")
        .expect("profile carries NTC, diode, and both MOSFET channel-charge branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through PA0..PA3 ADC1 routes"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("hold unrelated NTC at nominal temperature");
    session
        .set_signal_number("ui.mosfet_ward_dutton.gate_v", 0.0)
        .expect("initialize matched MOSFET gate drive");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle NTC, diode, and zero-charge MOSFET branches");
    assert_eq!(initial_steps.len(), 3, "all three analog netlists solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );
    assert!(signal_voltage(&session, "board.analog.pa2_volts").abs() < 1e-12);
    assert!(signal_voltage(&session, "board.analog.pa3_volts").abs() < 1e-12);

    session
        .set_signal_number("ui.mosfet_ward_dutton.gate_v", GATE_DRIVE_V)
        .expect("step the common MOSFET gate drive into inversion");
    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let mut previous_gate_v = 0.0;
    let mut previous_charged_v = 0.0;
    for step in 0..8 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the Ward–Dutton MOSFET transient");
        assert_eq!(
            steps.len(),
            3,
            "all three analog netlists solve at each step"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        let charged_v = signal_voltage(&session, "board.analog.pa2_volts");
        let reference_v = signal_voltage(&session, "board.analog.pa3_volts");
        let expected_charged_v = ward_dutton_drain_voltage(
            GATE_DRIVE_V,
            previous_gate_v,
            previous_charged_v,
            LOAD_OHMS,
            STEP_S,
        );

        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "unrelated NTC stays isolated"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley diode stays at its DC load line"
        );
        assert!(
            (charged_v - expected_charged_v).abs() < 5e-5,
            "independent Ward–Dutton charge-sheet recurrence at step {step}: expected {expected_charged_v} V, got {charged_v} V"
        );
        assert!(
            reference_v.abs() < 1e-9,
            "matched no-channel-charge MOSFET stays at the zero-current load line"
        );

        for (channel, volts) in [(0, ntc_v), (1, diode_v), (2, charged_v), (3, reference_v)] {
            let expected = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f103_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 channel {channel} must report its solved voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            0xFFFF,
            "neighboring ADC1 channel 4 remains undriven"
        );
        if step == 0 {
            assert!(
                charged_v > 0.5,
                "Ward–Dutton drain partition must produce a measurable transient: {charged_v} V"
            );
            assert!(
                (charged_v - reference_v).abs() > 0.5,
                "the no-charge branch must distinguish the channel-charge physics"
            );
        }
        previous_gate_v = GATE_DRIVE_V;
        previous_charged_v = charged_v;
    }
    assert!(
        previous_charged_v < 0.2,
        "Ward–Dutton terminal charge discharges toward the zero-current DC solution"
    );
}

/// Generic level-1 NMOS CGSO charge resolves through the e-paper profile's
/// ADC1 pins; matched zero-overlap, NTC, and diode branches remain isolated.
#[test]
fn nucleo_f103rb_epaper_mosfet_overlap_charge_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-f103rb-epaper.yaml");
    let mut profile =
        SystemManifest::from_file(&system_path).expect("load NUCLEO-F103RB e-paper profile");
    assert_eq!(profile.name, "nucleo-f103rb-epaper");
    assert_eq!(profile.external_devices.len(), 1);
    assert_eq!(profile.external_devices[0].id, "epaper");

    let generic = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-ntc-diode-system.yaml",
    ))
    .expect("load unrelated generic NTC and Shockley diode netlists");
    let overlap = SystemManifest::from_file(root(
        "examples/nucleo-f103rb-epaper/analog-mosfet-overlap-charge-system.yaml",
    ))
    .expect("load generic MOSFET overlap-charge comparison netlists");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(overlap.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F103 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
        assert_eq!(chip.analog_pins[pad].peripheral, "adc1");
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build e-paper board bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal co-simulation")
        .expect("profile carries NTC, diode, and both MOSFET overlap branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve through PA0..PA3 ADC1 routes"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("hold unrelated NTC at nominal temperature");
    session
        .set_signal_number("ui.mosfet_overlap.gate_v", 0.0)
        .expect("initialize matched MOSFET gate drive");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle NTC, diode, and zero-charge MOSFET branches");
    assert_eq!(initial_steps.len(), 4, "all four analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected ADC1 route errors: {initial_errors:?}"
    );
    assert!((signal_voltage(&session, "board.analog.pa2_volts")).abs() < 1e-12);
    assert!((signal_voltage(&session, "board.analog.pa3_volts")).abs() < 1e-12);

    session
        .set_signal_number("ui.mosfet_overlap.gate_v", 3.3)
        .expect("step the generic NMOS gate");

    // CGSO=10 uF/m times W=100 um gives 1 nF. With the 1 kOhm
    // source resistor and a 1 us backward-Euler step, the source node first
    // reaches half the gate step, then decays by half each held-input step.
    let expected_diode_v = diode_load_line_voltage();
    let mut expected_source_v = 1.65;
    for step in 0..12 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route MOSFET overlap-charge transient");
        assert_eq!(steps.len(), 4, "all four branches solve at every step");
        assert!(
            errors.is_empty(),
            "unexpected ADC1 route errors: {errors:?}"
        );

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        let reference_v = signal_voltage(&session, "board.analog.pa2_volts");
        let charged_v = signal_voltage(&session, "board.analog.pa3_volts");
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "unrelated NTC remains isolated"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated diode stays at its DC load line"
        );
        assert!(
            reference_v.abs() < 1e-12,
            "zero-overlap MOSFET is the transient control branch"
        );
        assert!(
            (charged_v - expected_source_v).abs() < 2e-8,
            "CGSO RC recurrence at step {step}: expected {expected_source_v} V, got {charged_v} V"
        );

        for (channel, volts) in [(0, ntc_v), (1, diode_v), (2, reference_v), (3, charged_v)] {
            let expected = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f103_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 conversion must report channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            0xFFFF,
            "neighboring ADC1 channel 4 remains undriven"
        );
        if step == 0 {
            assert_eq!(
                adc_channel_count(&mut bus, 3),
                2047,
                "the first overlap transient reaches half scale on PA3/ADC1_IN3"
            );
        }
        expected_source_v *= 0.5;
    }

    assert!(
        signal_voltage(&session, "board.analog.pa3_volts") < 1e-3,
        "overlap charge changes the transient, not the settled DC operating point"
    );
}
