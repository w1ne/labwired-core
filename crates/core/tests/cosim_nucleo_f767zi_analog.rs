// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices route through the NUCLEO-F767ZI STM32F767 profile into ADC1.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const RCC_BASE: u64 = 0x4002_3800;
const ADC1_BASE: u64 = 0x4001_2000;

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

/// Independent reverse-breakdown load-line reference, including the SPICE
/// breakdown knee fit and the diode's series resistance.
fn zener_breakdown_voltage(supply_v: f64) -> (f64, f64, f64) {
    const LOAD_OHMS: f64 = 1_000.0;
    const IS_A: f64 = 1e-12;
    const BV: f64 = 2.0;
    const IBV_A: f64 = 1e-3;
    const NBV: f64 = 1.5;
    const RS_OHMS: f64 = 20.0;
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;

    // ngspice's fixed-point fit places IBV at the model's specified BV.
    let mut xbv = BV - NBV * THERMAL_V * (1.0 + IBV_A / IS_A).ln();
    for _ in 0..25 {
        xbv = BV - NBV * THERMAL_V * (IBV_A / IS_A + 1.0 - xbv / THERMAL_V).ln();
        let fitted_current =
            IS_A * (((BV - xbv) / (NBV * THERMAL_V)).exp() - 1.0 + xbv / THERMAL_V);
        if (fitted_current - IBV_A).abs() <= 1e-9 * IBV_A {
            break;
        }
    }

    let breakdown_current = |junction_v: f64| {
        IS_A * ((junction_v - xbv) / (NBV * THERMAL_V)).exp() + 1e-12 * junction_v
    };
    let residual = |junction_v: f64| {
        let current = breakdown_current(junction_v);
        (supply_v - junction_v - current * RS_OHMS) / LOAD_OHMS - current
    };
    let (mut low, mut high) = (xbv, supply_v);
    assert!(residual(low) > 0.0 && residual(high) < 0.0);
    for _ in 0..100 {
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    let junction_v = 0.5 * (low + high);
    let current = breakdown_current(junction_v);
    (junction_v + current * RS_OHMS, junction_v, xbv)
}

/// Independent Gummel–Poon NPN load-line reference with forward Early effect.
/// The base-drive and collector-load roots are solved separately from the MNA
/// result so this catches a wrong transistor equation or route value.
fn bjt_collector_voltage(base_drive_v: f64) -> f64 {
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

/// Independent Gummel–Poon reference including forward/reverse Early effect
/// and optional high-injection rolloff.
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

/// Independent level-1 NMOS load-line reference for the board-routed test.
fn mosfet_drain_voltage(gate_v: f64) -> f64 {
    mosfet_body_effect_drain_voltage(gate_v, 0.0)
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

/// Independent SPICE level-1 JFET load-line reference, including the
/// Parker–Skellern B fit and channel-length modulation.
fn jfet_drain_voltage(gate_v: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 1_000.0;
    const VTO: f64 = -2.0;
    const BETA_A_PER_V2: f64 = 500e-6;
    const LAMBDA_PER_V: f64 = 0.025;
    const B: f64 = 0.8;
    const PB: f64 = 1.0;

    let overdrive = gate_v - VTO;
    if overdrive <= 0.0 {
        return SUPPLY_V;
    }
    let bfac = (1.0 - B) / (PB - VTO);
    let drain_current = |vds: f64| {
        let beta_p = BETA_A_PER_V2 * (1.0 + LAMBDA_PER_V * vds);
        if overdrive >= vds {
            beta_p
                * vds
                * (vds * (bfac * vds - B) + overdrive * (2.0 * B + 3.0 * bfac * (overdrive - vds)))
        } else {
            beta_p * overdrive * overdrive * (B + overdrive * bfac)
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

/// Independent forward gate-junction root for a JFET whose drain and source
/// are both grounded: the bias resistor feeds two parallel gate diodes whose
/// `IS` follows the `EG`/`XTI` law at the instance temperature.
fn jfet_gate_thermometer_voltage(bias_v: f64, temperature_c: f64) -> f64 {
    const BIAS_OHMS: f64 = 10_000.0;
    const IS_A: f64 = 10e-15;
    const EG_EV: f64 = 1.11;
    const XTI: f64 = 3.0;
    const TNOM_K: f64 = 25.0 + 273.15;
    let thermal_v = |kelvin: f64| 1.380_649e-23 * kelvin / 1.602_176_634e-19;
    let kelvin = temperature_c + 273.15;
    let vt = thermal_v(kelvin);
    let is_t = IS_A * (EG_EV / thermal_v(TNOM_K) - EG_EV / vt).exp() * (kelvin / TNOM_K).powf(XTI);
    let residual = |gate_v: f64| {
        (bias_v - gate_v) / BIAS_OHMS - 2.0 * (is_t * ((gate_v / vt).exp() - 1.0) + 1e-12 * gate_v)
    };
    let (mut low, mut high) = (0.0, bias_v);
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

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn adc_channel_count(bus: &mut SystemBus, channel: u8) -> u16 {
    let index = bus
        .find_peripheral_index_by_name("adc1")
        .expect("STM32F767 declares ADC1");
    bus.peripherals[index]
        .dev
        .as_any_mut()
        .expect("ADC1 downcasts")
        .downcast_mut::<labwired_core::peripherals::adc::Adc>()
        .expect("ADC1 uses the shared STM32 ADC model")
        .channel_input_count(channel)
}

/// Perform the selected one-shot through the same F4 register interface firmware uses.
fn stm32f4_adc_convert(bus: &mut SystemBus, channel: u8) -> u32 {
    const ADC1EN: u32 = 1 << 8;
    const ADON: u32 = 1;
    const SWSTART: u32 = 1 << 30;
    const EOC: u32 = 1 << 1;

    let apb2enr = bus.read_u32(RCC_BASE + 0x44).expect("read RCC APB2ENR");
    bus.write_u32(RCC_BASE + 0x44, apb2enr | ADC1EN)
        .expect("enable ADC1 clock");
    bus.write_u32(ADC1_BASE + 0x34, u32::from(channel))
        .expect("select SQR3 SQ1 channel");
    bus.write_u32(ADC1_BASE + 0x08, ADON)
        .expect("power up ADC1");
    bus.write_u32(ADC1_BASE + 0x08, ADON | SWSTART)
        .expect("software-start ADC1 conversion");
    for _ in 0..15 {
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

/// Independent SPICE level-1 JFET gate-junction depletion charge below FC·PB.
fn jfet_gate_junction_charge(voltage_v: f64) -> f64 {
    const C0_F: f64 = 1e-9;
    const PB_V: f64 = 1.0;
    const M: f64 = 0.5;
    C0_F * PB_V / (1.0 - M) * (1.0 - (1.0 - voltage_v / PB_V).powf(1.0 - M))
}

/// Independent backward-Euler update for two gate junctions tied to 3.3 V
/// drain/source nodes through their depletion charge.
fn expected_jfet_gate_voltage(previous_v: f64, drive_v: f64, step_s: f64) -> f64 {
    const DRIVE_OHMS: f64 = 1_000.0;
    const JUNCTION_REFERENCE_V: f64 = 3.3;
    let total_charge = |gate_v: f64| 2.0 * jfet_gate_junction_charge(gate_v - JUNCTION_REFERENCE_V);
    let previous_charge = total_charge(previous_v);
    let residual = |gate_v: f64| {
        (gate_v - drive_v) / DRIVE_OHMS + (total_charge(gate_v) - previous_charge) / step_s
    };
    let (mut low, mut high) = (drive_v.min(previous_v), drive_v.max(previous_v));
    for _ in 0..100 {
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            high = mid;
        } else {
            low = mid;
        }
    }
    0.5 * (low + high)
}

#[test]
fn nucleo_f767zi_ntc_and_diode_netlists_reach_pa0_pa1_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    assert_eq!(profile.name, "nucleo-f767zi");
    assert!(profile.external_devices.is_empty());
    assert!(
        profile
            .board_io
            .iter()
            .any(|io| io.id == "led1_pb0" && io.peripheral == "gpiob" && io.pin == 0),
        "the mixed-signal run keeps the NUCLEO-F767ZI's PB0 user LED"
    );
    assert!(
        profile.board_io.iter().any(|io| {
            io.id == "button_user_pc13"
                && io.peripheral == "gpioc"
                && io.pin == 13
                && io.active_high
        }),
        "the mixed-signal run keeps the NUCLEO-F767ZI's PC13 user button"
    );

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and diode netlists");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    assert_eq!(chip.analog_pins["PA0"].channel, 0);
    assert_eq!(chip.analog_pins["PA1"].channel, 1);
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through PA0/PA1 to ADC1 channels 0/1"
    );
    for channel in [0, 1, 2] {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} begins without an analog source"
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
            "the Shockley diode load line should be {expected_diode_v} V, got {diode_v} V"
        );

        for (channel, volts) in [(0, ntc_v), (1, diode_v)] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
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
        "ADC counts track the solved temperature-dependent voltage: {ntc_samples:?}"
    );
    assert_eq!(
        ntc_samples[1].1, 2047,
        "the 25 °C NTC divider is half scale"
    );
}

#[test]
fn nucleo_f767zi_diode_depletion_and_transit_charge_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let charge = SystemManifest::from_file(root("examples/nucleo-f767zi/analog-diode-charge.yaml"))
        .expect("load diode charge comparison netlists");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(charge.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [
        ("PA0", 0),
        ("PA1", 1),
        ("PA2", 2),
        ("PA3", 3),
        ("PA4", 4),
        ("PA5", 5),
    ] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an NTC and four diode charge variants");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic analog netlists must resolve to ADC1 channels 0 through 5"
    );
    for channel in 0..=6 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.diode_charge.supply_v", 0.0)
        .expect("initialize the diode drive at zero volts");

    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle the NTC and diode charge branches before the step");
    assert_eq!(initial_steps.len(), 6, "all six analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );

    session
        .set_signal_number("ui.diode_charge.supply_v", 3.3)
        .expect("apply a supply step to the four diode branches");
    let mut samples = Vec::new();
    for step in 0..800 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the NTC and all four diode branches");
        assert_eq!(steps.len(), 6, "all six analog branches solve at each step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let reference_v = signal_voltage(&session, "board.analog.pa2_volts");
        let depletion_v = signal_voltage(&session, "board.analog.pa3_volts");
        let transit_v = signal_voltage(&session, "board.analog.pa4_volts");
        let combined_v = signal_voltage(&session, "board.analog.pa5_volts");
        assert!(
            (reference_v - diode_load_line_voltage()).abs() < 2e-5,
            "uncharged reference should sit at the DC load line {} V, got {reference_v} V at step {step}",
            diode_load_line_voltage()
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa0_volts") - 1.65).abs() < 1e-9,
            "the unrelated 25 °C NTC stays isolated on PA0/ADC1_IN0"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa1_volts") - diode_load_line_voltage()).abs()
                < 2e-5,
            "the unrelated DC diode stays isolated on PA1/ADC1_IN1"
        );

        for (channel, volts) in [
            (0, 1.65),
            (1, diode_load_line_voltage()),
            (2, reference_v),
            (3, depletion_v),
            (4, transit_v),
            (5, combined_v),
        ] {
            let expected = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 conversion must report the voltage on channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 6),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        samples.push((reference_v, depletion_v, transit_v, combined_v));
        if step == 0 {
            assert!(
                (depletion_v - reference_v).abs() > 0.001,
                "CJO depletion charge must change the first routed transient: {samples:?}"
            );
            assert!(
                (transit_v - reference_v).abs() > 0.01,
                "TT diffusion charge must change the first routed transient: {samples:?}"
            );
            assert!(
                (combined_v - reference_v).abs() > 0.01,
                "combined junction and transit charge must change the first routed transient: {samples:?}"
            );
        }
    }

    let settled = samples.last().expect("transient samples exist");
    for (label, voltage) in [
        ("depletion", settled.1),
        ("transit", settled.2),
        ("combined", settled.3),
    ] {
        assert!(
            (voltage - settled.0).abs() < 2e-4,
            "{label} charge branch must settle to the same DC diode load line, got {voltage} V vs {} V",
            settled.0
        );
    }
}

#[test]
fn nucleo_f767zi_bjt_load_line_reaches_pa2_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    assert_eq!(profile.name, "nucleo-f767zi");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and diode netlists");
    let bjt = SystemManifest::from_file(root("examples/nucleo-f767zi/analog-bjt-load.yaml"))
        .expect("load generic Gummel–Poon NPN netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and BJT devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic NTC, diode, and NPN netlists must resolve to ADC1 channels 0/1/2"
    );
    for channel in 0..=3 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent NTC temperature");

    let mut cycles = 0;
    let mut collector_samples = Vec::new();
    for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt.base_v", base_drive_v)
            .expect("set Gummel–Poon base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all three analog netlists");
        assert_eq!(steps.len(), 3, "all devices solve at the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let collector_v = signal_voltage(&session, "board.analog.pa2_volts");
        let expected_collector_v = bjt_collector_voltage(base_drive_v);
        assert!(
            (collector_v - expected_collector_v).abs() < 5e-5,
            "Gummel–Poon NPN load line at base drive {base_drive_v} V should be {expected_collector_v} V, got {collector_v} V"
        );
        let bjt_count = expected_adc_count(collector_v);
        assert_eq!(
            adc_channel_count(&mut bus, 2),
            bjt_count,
            "the solved BJT collector voltage must seed ADC1 channel 2"
        );
        assert_eq!(
            stm32f4_adc_convert(&mut bus, 2),
            u32::from(bjt_count),
            "firmware-visible ADC1 channel 2 conversion must report the BJT load line"
        );

        for (channel, volts) in [(0, expected_ntc_v), (1, expected_diode_v)] {
            let count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                count,
                "the independent analog source must remain isolated on ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(count),
                "firmware-visible ADC1 channel {channel} conversion remains stable"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 3),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        collector_samples.push((base_drive_v, collector_v));
    }

    assert!(
        collector_samples
            .windows(2)
            .all(|pair| pair[1].1 < pair[0].1),
        "collector voltage falls monotonically as base drive increases: {collector_samples:?}"
    );
}

#[test]
fn nucleo_f767zi_bjt_high_injection_rolloff_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    assert_eq!(profile.name, "nucleo-f767zi");

    let generic =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and Shockley diode netlists");
    let high_injection = SystemManifest::from_file(root(
        "examples/nucleo-f767zi/analog-bjt-high-injection-adc.yaml",
    ))
    .expect("load matched generic Gummel–Poon high-injection branches");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(high_injection.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal co-simulation")
        .expect("profile carries NTC, diode, and matched BJT netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to distinct ADC1 channels 0 through 3"
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
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_high_injection.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    let expected_ntc_v = 1.65;
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut high_drive_samples = None;
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

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        let rolloff_v = signal_voltage(&session, "board.analog.pa2_volts");
        let control_v = signal_voltage(&session, "board.analog.pa3_volts");
        let expected_rolloff_v = bjt_high_injection_collector_voltage(base_drive_v, true);
        let expected_control_v = bjt_high_injection_collector_voltage(base_drive_v, false);
        assert!(
            (rolloff_v - expected_rolloff_v).abs() < 5e-5,
            "IKF/IKR/NKF load line at base drive {base_drive_v} V should be {expected_rolloff_v} V, got {rolloff_v} V"
        );
        assert!(
            (control_v - expected_control_v).abs() < 5e-5,
            "matched no-rolloff load line at base drive {base_drive_v} V should be {expected_control_v} V, got {control_v} V"
        );
        assert!((ntc_v - expected_ntc_v).abs() < 1e-9);
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains at its independent load line"
        );

        for (channel, voltage_v) in [(0, ntc_v), (1, diode_v), (2, rolloff_v), (3, control_v)] {
            let expected_count = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "ADC1 channel {channel} conversion follows its independent netlist voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
        );
        if base_drive_v == 3.3 {
            high_drive_samples = Some((rolloff_v, control_v));
        }
    }

    let (rolloff_v, control_v) = high_drive_samples.expect("recorded high-drive sample");
    assert!(
        rolloff_v - control_v > 0.05,
        "IKF/IKR/NKF rolloff reduces collector current and raises collector voltage: {rolloff_v} V vs {control_v} V"
    );
}

#[test]
fn nucleo_f767zi_bjt_terminal_resistances_reach_adc1_with_explicit_network_references() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    assert_eq!(profile.name, "nucleo-f767zi");

    let generic =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and Shockley diode netlists");
    let terminal_resistances = SystemManifest::from_file(root(
        "examples/nucleo-f767zi/analog-bjt-terminal-resistance-adc.yaml",
    ))
    .expect("load matched generic BJT model-card and resistor-network branches");
    profile.cosim_models = generic.cosim_models;
    profile
        .cosim_models
        .extend(terminal_resistances.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [
        ("PA0", 0),
        ("PA1", 1),
        ("PA2", 2),
        ("PA3", 3),
        ("PA4", 4),
        ("PA5", 5),
        ("PA6", 6),
        ("PA7", 7),
    ] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build mixed-signal co-simulation")
        .expect("profile carries an NTC, diode, and six matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to ADC1 channels zero through seven"
    );
    for channel in 0..=8 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_terminal.base_v", 0.0)
        .expect("initialize matched BJT base drives");

    let mut cycles = 0;
    for base_drive_v in [0.7_f64, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt_terminal.base_v", base_drive_v)
            .expect("step all matched BJT base drives");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and BJT resistance branches");
        assert_eq!(steps.len(), 8, "all eight generic analog branches solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "unrelated NTC remains isolated"
        );
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "unrelated Shockley diode remains on its independent load line"
        );

        let bjt_voltages = [
            signal_voltage(&session, "board.analog.pa2_volts"),
            signal_voltage(&session, "board.analog.pa3_volts"),
            signal_voltage(&session, "board.analog.pa4_volts"),
            signal_voltage(&session, "board.analog.pa5_volts"),
            signal_voltage(&session, "board.analog.pa6_volts"),
            signal_voltage(&session, "board.analog.pa7_volts"),
        ];
        let zero_resistance_v = bjt_collector_voltage(base_drive_v);
        for (label, model_card_v, explicit_network_v) in [
            ("RB", bjt_voltages[0], bjt_voltages[1]),
            ("RC", bjt_voltages[2], bjt_voltages[3]),
            ("RE", bjt_voltages[4], bjt_voltages[5]),
        ] {
            assert!(
                (model_card_v - explicit_network_v).abs() < 1e-7,
                "{label} model-card resistance must match its explicit resistor network: {model_card_v} V vs {explicit_network_v} V"
            );
            if base_drive_v == 0.9 {
                assert!(
                    (model_card_v - zero_resistance_v).abs() > 1e-3,
                    "{label} must change the collector load line: zero-R={zero_resistance_v} V, actual={model_card_v} V"
                );
            }
        }

        for (channel, voltage_v) in [(0, ntc_v), (1, diode_v)]
            .into_iter()
            .chain((2..=7).zip(bjt_voltages))
        {
            let expected_count = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "solved voltage seeds descriptor-routed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1 conversion follows the analog device on channel {channel}"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 8),
            0xFFFF,
            "neighboring ADC1 channel 8 remains undriven"
        );
    }
}

#[test]
fn nucleo_f767zi_bjt_junction_and_transit_charge_reach_adc1_end_to_end() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    let generic =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC and diode netlists");
    let charge = SystemManifest::from_file(root("examples/nucleo-f767zi/analog-bjt-charge.yaml"))
        .expect("load matched BJT charge comparison netlists");
    profile.cosim_models = generic.cosim_models;
    profile.cosim_models.extend(charge.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [
        ("PA0", 0),
        ("PA1", 1),
        ("PA2", 2),
        ("PA3", 3),
        ("PA4", 4),
        ("PA5", 5),
        ("PA6", 6),
    ] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and five matched BJT charge cases");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic devices must resolve to ADC1 channels 0 through 6"
    );
    for channel in 0..=7 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated Beta NTC temperature");
    session
        .set_signal_number("ui.bjt_charge.base_v", 0.0)
        .expect("initialize all BJT base-drive sources");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("initialize the independent branches before the base-drive step");
    assert_eq!(initial_steps.len(), 7, "all seven analog branches solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );

    session
        .set_signal_number("ui.bjt_charge.base_v", 0.8)
        .expect("step the matched BJT base-drive sources");
    let mut samples = Vec::new();
    for step in 0..8_000 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route the NTC, diode, and BJT charge branches");
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
        assert!(
            (ntc_v - 1.65).abs() < 1e-9,
            "the unrelated NTC stays isolated"
        );
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated diode stays at its independent DC load line"
        );

        let expected_dc = bjt_collector_voltage(0.8);
        assert!(
            (bjt_voltages[0] - expected_dc).abs() < 5e-5,
            "the uncharged branch must match the independent Gummel–Poon load line {expected_dc} V, got {} V",
            bjt_voltages[0]
        );
        for (label, channel, voltage) in [
            ("NTC", 0, ntc_v),
            ("diode", 1, diode_v),
            ("uncharged BJT", 2, bjt_voltages[0]),
            ("CJE BJT", 3, bjt_voltages[1]),
            ("CJC BJT", 4, bjt_voltages[2]),
            ("TF BJT", 5, bjt_voltages[3]),
            ("combined-charge BJT", 6, bjt_voltages[4]),
        ] {
            let expected = expected_adc_count(voltage);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved {label} voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1 channel {channel} must report the {label} voltage"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 7),
            0xFFFF,
            "the neighboring ADC1 channel remains undriven"
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
fn nucleo_f767zi_mosfet_load_line_reaches_pa3_adc1_alongside_unrelated_devices() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    assert_eq!(profile.name, "nucleo-f767zi");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load the independent generic NTC and diode netlists");
    let mosfet =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-mosfet-switch.yaml"))
            .expect("load the generic low-side MOSFET netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries three unrelated analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "NTC, diode, and NMOS netlists must resolve to distinct ADC1 inputs"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    const NTC_V: f64 = 1.65;
    let diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent NTC temperature");

    let mut cycles = 0;
    let mut drain_samples = Vec::new();
    for gate_v in [0.0_f64, 0.8, 1.0, 2.0, 3.3] {
        session
            .set_signal_number("ui.mosfet.gate_v", gate_v)
            .expect("set generic MOSFET gate voltage");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and MOSFET netlists");
        assert_eq!(
            steps.len(),
            3,
            "all three devices solve at the 1 us boundary"
        );
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let drain_v = signal_voltage(&session, "board.analog.pa3_volts");
        let expected_drain_v = mosfet_drain_voltage(gate_v);
        assert!(
            (drain_v - expected_drain_v).abs() < 5e-5,
            "level-1 NMOS load line at Vgs={gate_v} V should be {expected_drain_v} V, got {drain_v} V"
        );
        for (channel, volts) in [(3, drain_v), (0, NTC_V), (1, diode_v)] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1 channel {channel} conversion follows its analog device"
            );
        }
        assert!(
            (signal_voltage(&session, "board.analog.pa0_volts") - NTC_V).abs() < 1e-9,
            "the independent Beta NTC divider remains on PA0/ADC1 channel 0"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa1_volts") - diode_v).abs() < 2e-5,
            "the independent Shockley diode remains on PA1/ADC1 channel 1"
        );
        for channel in [2, 4] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                0xFFFF,
                "neighboring ADC1 channel {channel} remains undriven"
            );
        }
        drain_samples.push((gate_v, drain_v));
    }

    assert_eq!(
        drain_samples[0].1, drain_samples[2].1,
        "gate voltage at or below threshold leaves the drain at the supply"
    );
    assert!(
        drain_samples[4].1 < drain_samples[3].1 && drain_samples[3].1 < drain_samples[2].1,
        "drain voltage falls as the NMOS conducts more strongly: {drain_samples:?}"
    );
}

#[test]
fn nucleo_f767zi_mosfet_body_effect_reaches_adc1_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    assert_eq!(profile.name, "nucleo-f767zi");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load the independent generic NTC and diode netlists");
    let body_effect = SystemManifest::from_file(root(
        "examples/nucleo-f767zi/analog-mosfet-body-effect.yaml",
    ))
    .expect("load matched generic NMOS body-bias netlists");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(body_effect.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched NMOS branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all four generic netlists must resolve to distinct ADC1 inputs"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    const NTC_V: f64 = 1.65;
    let diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the independent Beta NTC temperature");

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

        for (channel, volts) in [(0, NTC_V), (1, diode_v), (2, reference_v), (3, body_v)] {
            let expected = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected),
                "ADC1 channel {channel} conversion must follow its analog device"
            );
        }
        assert_eq!(
            adc_channel_count(&mut bus, 4),
            0xFFFF,
            "neighboring ADC1 channel 4 remains undriven"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa0_volts") - NTC_V).abs() < 1e-9,
            "the unrelated Beta NTC remains isolated on PA0/ADC1 channel 0"
        );
        assert!(
            (signal_voltage(&session, "board.analog.pa1_volts") - diode_v).abs() < 2e-5,
            "the unrelated Shockley diode remains isolated on PA1/ADC1 channel 1"
        );
        body_samples.push((bulk_v, body_v));
    }

    assert!(
        body_samples.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "reverse body bias raises Vth and the loaded drain voltage: {body_samples:?}"
    );
}

#[test]
fn nucleo_f767zi_jfet_load_line_reaches_pa4_adc1_alongside_unrelated_devices() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    assert_eq!(profile.name, "nucleo-f767zi");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load the independent generic NTC and diode netlists");
    let mosfet =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-mosfet-switch.yaml"))
            .expect("load the generic low-side NMOS netlist");
    let jfet = SystemManifest::from_file(root("examples/nucleo-f767zi/analog-jfet-load.yaml"))
        .expect("load the generic level-1 JFET netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(mosfet.cosim_models);
    profile.cosim_models.extend(jfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA3", 3), ("PA4", 4)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, NMOS, and JFET devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "all four generic netlists must resolve to distinct ADC1 inputs"
    );
    for channel in 0..=5 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    const NTC_V: f64 = 1.65;
    const NMOS_GATE_V: f64 = 2.0;
    let diode_v = diode_load_line_voltage();
    let mosfet_v = mosfet_drain_voltage(NMOS_GATE_V);
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set independent NTC temperature");
    session
        .set_signal_number("ui.mosfet.gate_v", NMOS_GATE_V)
        .expect("set independent NMOS gate voltage");

    let mut cycles = 0;
    let mut drain_samples = Vec::new();
    for gate_v in [0.0_f64, -0.5, -1.0, -1.5, -2.2] {
        session
            .set_signal_number("ui.jfet.gate_v", gate_v)
            .expect("set generic JFET gate voltage");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, NMOS, and JFET netlists");
        assert_eq!(steps.len(), 4, "all devices solve at the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let drain_v = signal_voltage(&session, "board.analog.pa4_volts");
        let expected_drain_v = jfet_drain_voltage(gate_v);
        assert!(
            (drain_v - expected_drain_v).abs() < 5e-5,
            "level-1 NJF load line at Vgs={gate_v} V should be {expected_drain_v} V, got {drain_v} V"
        );
        for (channel, volts) in [(4, drain_v), (0, NTC_V), (1, diode_v), (3, mosfet_v)] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1 channel {channel} conversion follows its analog device"
            );
        }
        for channel in [2, 5] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                0xFFFF,
                "neighboring ADC1 channel {channel} remains undriven"
            );
        }
        drain_samples.push((gate_v, drain_v));
    }

    assert!(
        drain_samples[0].1 < 2.0,
        "zero-bias JFET load line exercises the linear channel region: {drain_samples:?}"
    );
    assert!(
        drain_samples[1].1 > 1.5,
        "the reverse-biased gate moves the JFET into saturation: {drain_samples:?}"
    );
    assert!(
        (drain_samples[4].1 - 3.3).abs() < 1e-6,
        "gate voltage beyond pinch-off leaves the JFET drain at the supply"
    );
    assert!(
        drain_samples.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "the drain voltage rises monotonically toward cutoff as Vgs becomes more negative: {drain_samples:?}"
    );
}

#[test]
fn nucleo_f767zi_zener_breakdown_reaches_pa5_adc1_alongside_unrelated_devices() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    assert_eq!(profile.name, "nucleo-f767zi");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load the independent generic NTC and forward diode netlists");
    let zener =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-zener-breakdown.yaml"))
            .expect("load the generic reverse-breakdown diode netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(zener.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA5", 5)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an NTC, forward diode, and reverse-breakdown diode");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through PA0/PA1/PA5 to ADC1 channels 0/1/5"
    );
    for channel in 0..=6 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            0xFFFF,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the independent NTC temperature");
    let mut cycles = 0;
    let mut zener_samples = Vec::new();
    for supply_v in [2.6_f64, 3.0, 3.3] {
        session
            .set_signal_number("ui.zener.supply_v", supply_v)
            .expect("drive the generic zener load line");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all three analog netlists");
        assert_eq!(steps.len(), 3, "all devices solve at the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let zener_v = signal_voltage(&session, "board.analog.pa5_volts");
        let (expected_v, junction_v, xbv) = zener_breakdown_voltage(supply_v);
        assert!(
            junction_v > xbv,
            "the load line must enter the reverse-breakdown region: {junction_v} V <= knee {xbv} V"
        );
        assert!(
            (zener_v - expected_v).abs() < 2e-5,
            "reverse-breakdown load line at {supply_v} V should be {expected_v} V, got {zener_v} V"
        );

        for (channel, volts) in [(0, 1.65), (1, diode_load_line_voltage()), (5, zener_v)] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1 channel {channel} conversion must report its analog voltage"
            );
        }
        for channel in [2, 3, 4, 6] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                0xFFFF,
                "unconnected ADC1 channel {channel} remains undriven"
            );
        }
        zener_samples.push((supply_v, zener_v));
    }
    assert!(
        zener_samples.windows(2).all(|pair| pair[1].1 > pair[0].1),
        "the breakdown voltage follows the driven load line: {zener_samples:?}"
    );
}

#[test]
fn nucleo_f767zi_heated_jfet_gate_thermometer_reaches_pa6_pa7_adc1() {
    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    assert_eq!(profile.name, "nucleo-f767zi");

    let analog =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load the independent generic NTC and forward diode netlists");
    let thermometer = SystemManifest::from_file(root(
        "examples/nucleo-f767zi/analog-jfet-gate-thermometer.yaml",
    ))
    .expect("load the generic JFET gate-junction thermometer netlist");
    profile.cosim_models = analog.cosim_models;
    profile.cosim_models.extend(thermometer.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA6", 6), ("PA7", 7)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an NTC, forward diode, and JFET thermometer pair");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic netlists must resolve through PA0/PA1/PA6/PA7 to ADC1"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the independent NTC temperature");
    let mut cycles = 0;
    for bias_v in [1.5_f64, 3.3] {
        session
            .set_signal_number("ui.jfet_thermo.bias_v", bias_v)
            .expect("drive the shared gate bias");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route all three analog netlists");
        assert_eq!(steps.len(), 3, "all devices solve at the 1 us boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let reference_v = signal_voltage(&session, "board.analog.pa6_volts");
        let heated_v = signal_voltage(&session, "board.analog.pa7_volts");
        for (label, measured, temperature_c) in
            [("TNOM", reference_v, 25.0), ("85 C", heated_v, 85.0)]
        {
            let expected = jfet_gate_thermometer_voltage(bias_v, temperature_c);
            assert!(
                (measured - expected).abs() < 2e-5,
                "{label} gate junction at {bias_v} V bias should sit at {expected} V, got {measured} V"
            );
        }
        // About -2 mV/K over 60 K: a missing IS(T) or kT/q law collapses this.
        let drop = reference_v - heated_v;
        assert!(
            (0.09..0.15).contains(&drop),
            "the heated gate must read ~120 mV lower at {bias_v} V bias, got {drop} V"
        );

        for (channel, volts) in [
            (0, 1.65),
            (1, diode_load_line_voltage()),
            (6, reference_v),
            (7, heated_v),
        ] {
            let expected_count = expected_adc_count(volts);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected_count,
                "the solved voltage must seed ADC1 channel {channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected_count),
                "firmware-visible ADC1 channel {channel} conversion must report its analog voltage"
            );
        }
        assert!(
            u32::from(expected_adc_count(reference_v))
                > u32::from(expected_adc_count(heated_v)) + 100,
            "firmware must resolve the temperature difference in ADC counts"
        );
        for channel in [2, 3, 4, 5] {
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                0xFFFF,
                "unconnected ADC1 channel {channel} remains undriven"
            );
        }
    }
}

#[test]
fn nucleo_f767zi_jfet_gate_depletion_charge_reaches_adc1_with_ntc_and_diode() {
    const STEP_S: f64 = 1e-6;
    const SUBSTEPS: usize = 10;

    let system_path = root("configs/systems/nucleo-f767zi.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NUCLEO-F767ZI profile");
    let base =
        SystemManifest::from_file(root("examples/nucleo-f767zi/analog-ntc-diode-system.yaml"))
            .expect("load the unrelated generic NTC and Shockley diode netlists");
    let jfet = SystemManifest::from_file(root(
        "examples/nucleo-f767zi/analog-jfet-gate-charge-adc.yaml",
    ))
    .expect("load matched generic JFET gate-charge branches");
    profile.cosim_models = base.cosim_models;
    profile.cosim_models.extend(jfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load STM32F767 descriptor");
    for (pad, channel) in [("PA0", 0), ("PA1", 1), ("PA2", 2), ("PA3", 3)] {
        assert_eq!(chip.analog_pins[pad].channel, channel);
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build NUCLEO-F767ZI bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries JFET, NTC, and diode netlists");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic netlists must resolve PA0..PA3 to ADC1 channels 0..3"
    );
    for channel in 0..=4 {
        assert_eq!(
            adc_channel_count(&mut bus, channel),
            u16::MAX,
            "ADC1 channel {channel} starts without an analog source"
        );
    }

    let expected_diode_v = diode_load_line_voltage();
    session
        .set_signal_number("ui.jfet_gate.drive_v", 3.3)
        .expect("precharge both JFET gates at the drain/source reference");
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set the unrelated NTC temperature");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle NTC, diode, and both JFET branches");
    assert_eq!(initial_steps.len(), 4, "all four analog netlists solve");
    assert!(
        initial_errors.is_empty(),
        "unexpected route errors: {initial_errors:?}"
    );
    assert!(
        (signal_voltage(&session, "board.analog.pa2_volts") - 3.3).abs() < 1e-9,
        "the charged JFET gate starts at the drain/source reference"
    );

    session
        .set_signal_number("ui.jfet_gate.drive_v", 0.0)
        .expect("step both JFET gate drives low");
    session
        .set_signal_number("ui.ntc.temperature_c", 80.0)
        .expect("heat the unrelated NTC while the JFET gates discharge");
    let resistance = ntc_resistance_ohms(80.0);
    let expected_ntc_v = 3.3 * 10_000.0 / (resistance + 10_000.0);
    let mut expected_gate_v = 3.3;
    for step in 0..8 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route JFET charge alongside NTC and diode");
        assert_eq!(steps.len(), 4, "all four netlists solve at every step");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        for _ in 0..SUBSTEPS {
            expected_gate_v =
                expected_jfet_gate_voltage(expected_gate_v, 0.0, STEP_S / SUBSTEPS as f64);
        }
        let gate_v = signal_voltage(&session, "board.analog.pa2_volts");
        let control_v = signal_voltage(&session, "board.analog.pa3_volts");
        assert!(
            (gate_v - expected_gate_v).abs() < 2e-8,
            "JFET CGS/CGD depletion charge at step {step}: expected {expected_gate_v} V, got {gate_v} V"
        );
        assert!(
            control_v.abs() < 1e-8,
            "zero-charge control follows the low drive within the leakage floor, got {control_v} V"
        );
        assert!(
            gate_v > control_v,
            "depletion charge must delay the gate edge relative to its zero-charge control"
        );

        let ntc_v = signal_voltage(&session, "board.analog.pa0_volts");
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "the unrelated NTC stays at its 80 °C divider voltage"
        );
        let diode_v = signal_voltage(&session, "board.analog.pa1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the unrelated Shockley diode stays at its DC load line"
        );

        for (channel, voltage_v) in [(0, ntc_v), (1, diode_v), (2, gate_v), (3, control_v)] {
            let expected = expected_adc_count(voltage_v);
            assert_eq!(
                adc_channel_count(&mut bus, channel),
                expected,
                "the solved voltage seeds ADC1_IN{channel}"
            );
            assert_eq!(
                stm32f4_adc_convert(&mut bus, channel),
                u32::from(expected),
                "firmware-visible ADC1_IN{channel} conversion reports the routed voltage"
            );
        }
    }
    assert!(
        signal_voltage(&session, "board.analog.pa2_volts") < 1.0,
        "gate depletion capacitance delays the edge but eventually follows the low drive"
    );
    assert_eq!(
        adc_channel_count(&mut bus, 4),
        u16::MAX,
        "neighboring ADC1_IN4 remains undriven"
    );
}
