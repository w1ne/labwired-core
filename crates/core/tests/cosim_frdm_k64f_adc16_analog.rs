// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the FRDM-K64F's two
//! ADC16 instances through the `analog_pins` map: the NTC on Arduino A0
//! (PTB2 = ADC0_SE12) and the diode on A2 (PTB10 = ADC1_SE14), converted the
//! way mcux-sdk `fsl_adc16.c` does it (Init, DoAutoCalibration, SC1A write,
//! poll COCO, read R[0]). Also:
//! - one continuous conversion (`SC3.ADCO`) started ONCE keeps following the
//!   NTC as its temperature moves, with no further `SC1A` write;
//! - the differential pair DAD2 (PTE2 = ADC0_DP2, PTE3 = ADC0_DM2) converts
//!   the signed difference of the two netlists, sign-extended in `R[0]`.
//! - a generic level-1 JFET's gate-junction depletion charge follows a
//!   transient through ADC0_SE13/14 beside the unrelated NTC and diode.
//! - matched level-1 NMOS stages separate by lateral diffusion `LD` and the
//!   `UO`/`TOX`-derived `KP` through ADC0_SE8/9/13.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC0: u64 = 0x4003_B000;
const ADC1: u64 = 0x400B_B000;
const SC1A: u64 = 0x00;
const CFG1: u64 = 0x08;
const RA: u64 = 0x10;
const SC2: u64 = 0x20;
const SC3: u64 = 0x24;
const OFS: u64 = 0x28;
const COCO: u32 = 1 << 7;
const DIFF: u32 = 1 << 5;
const MODE_12: u32 = 1 << 2;
const MODE_16: u32 = 3 << 2;
const ADACT: u32 = 1 << 7;
const ADCO: u32 = 1 << 3;
const CAL: u32 = 1 << 7;
const CALF: u32 = 1 << 6;
const ADCH_DISABLED: u32 = 0x1F;
/// ADC0_SE12 (PTB2), ADC1_SE14 (PTB10), DAD2 (PTE2/PTE3).
const AD12: u32 = 12;
const AD14: u32 = 14;
const DAD2: u32 = 2;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn ntc_volts(temperature_c: f64) -> f64 {
    let r = ntc_resistance_ohms(temperature_c);
    3.3 * r / (10_000.0 + r)
}

/// Independent DC solution for the fixture's non-inverting OPAMP. The finite
/// open-loop gain, output resistance, and feedback divider are all included;
/// this is not the ideal `1 + Rfeedback/Rgain` approximation.
fn noninverting_opamp_output_voltage(input_v: f64) -> f64 {
    const AOL: f64 = 100_000.0;
    const ROUT: f64 = 100.0;
    const RFEEDBACK: f64 = 2_000.0;
    const RGAIN: f64 = 10_000.0;
    let output_load = RFEEDBACK + RGAIN;
    let output_fraction = output_load / (output_load + ROUT);
    let feedback_fraction = RGAIN / output_load;
    AOL * output_fraction * input_v / (1.0 + AOL * output_fraction * feedback_fraction)
}

/// Independent JFET gate-junction depletion charge below `FC * PB`.
fn jfet_gate_junction_charge(voltage_v: f64) -> f64 {
    const C0_F: f64 = 1e-9;
    const PB_V: f64 = 1.0;
    const M: f64 = 0.5;
    C0_F * PB_V / (1.0 - M) * (1.0 - (1.0 - voltage_v / PB_V).powf(1.0 - M))
}

/// Independent backward-Euler update for two equal gate junctions tied to
/// fixed 3.3 V drain/source nodes through their depletion charges.
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
        let mid = 0.5 * (low + high);
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    let junction_v = 0.5 * (low + high);
    junction_v + current_at_junction_voltage(junction_v) * DIODE_RS_OHMS
}

/// Independent Gummel–Poon load line with optional forward Early effect.
fn bjt_early_collector_voltage(base_drive_v: f64, forward_early_voltage: Option<f64>) -> f64 {
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
        let base_width = 1.0 - forward_early_voltage.map_or(0.0, |vaf| vbc / vaf) - vbe / VAR;
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
        if (base_drive_v - vbe) / BASE_OHMS > base_current {
            low = vbe;
        } else {
            high = vbe;
        }
    }
    collector_for_vbe(0.5 * (low + high))
}

/// Independent level-1 NMOS common-source load line: 3.3 V through 10 kΩ
/// into the drain, source and bulk grounded, `VTO = 1`, `LAMBDA = 0`. Drain
/// current is Shichman–Hodges with `beta = KP·W/(L − 2·LD)` plus the
/// engine's drain-source `GMIN`; bisection, not the solver.
fn nmos_drain_voltage(gate_v: f64, beta: f64) -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LOAD_OHMS: f64 = 10_000.0;
    const VTO_V: f64 = 1.0;
    const GMIN_S: f64 = 1e-12;
    let channel = |drain_v: f64| {
        let overdrive = gate_v - VTO_V;
        if overdrive <= 0.0 {
            0.0
        } else if drain_v < overdrive {
            beta * drain_v * (overdrive - 0.5 * drain_v)
        } else {
            0.5 * beta * overdrive * overdrive
        }
    };
    let (mut low, mut high) = (0.0, SUPPLY_V);
    for _ in 0..100 {
        let drain_v = 0.5 * (low + high);
        if (SUPPLY_V - drain_v) / LOAD_OHMS > channel(drain_v) + GMIN_S * drain_v {
            low = drain_v;
        } else {
            high = drain_v;
        }
    }
    0.5 * (low + high)
}

/// ngspice level-1 `KP` from the process when the card gives only `TOX`.
fn process_kp(uo_cm2_per_vs: f64, tox_m: f64) -> f64 {
    uo_cm2_per_vs * 1e-4 * 3.453_133e-11 / tox_m
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn millivolts(volts: f64) -> i64 {
    (volts * 1000.0).round() as i64
}

/// Single-ended N-bit code: 1 LSB = VREFH (3.3 V) / 2^N, truncated, clamped.
fn se_code(volts: f64, bits: u32) -> u32 {
    ((millivolts(volts) << bits) / 3300).clamp(0, (1 << bits) - 1) as u32
}

/// The FRDM-K64F with the generic NTC and diode netlists routed to `pads`.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/frdm-k64f.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load FRDM-K64F profile");
    let analog = SystemManifest::from_file(root("examples/frdm-k64f/analog-ntc-diode-system.yaml"))
        .expect("load generic Beta NTC and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    for model in &mut profile.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", ntc_pad),
            "diode_load_line" => ("v_diode", diode_pad),
            other => panic!("unexpected model {other}"),
        };
        model
            .outputs
            .insert(key.to_string(), format!("board.analog.{pad}_volts"));
    }
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load MK64F12 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build FRDM-K64F bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to ADC16 inputs through analog_pins"
    );
    (bus, session)
}

/// Move the NTC and solve both netlists to the next boundary.
fn settle(session: &mut CosimSession, bus: &mut SystemBus, cycles: &mut u64, temperature_c: f64) {
    session
        .set_signal_number("ui.ntc.temperature_c", temperature_c)
        .expect("set NTC temperature");
    *cycles += session.cycles_until_boundary(*cycles);
    let (steps, errors) = session
        .advance_to(*cycles, bus)
        .expect("solve and route both netlists");
    assert_eq!(steps.len(), 2, "both netlists solve each step");
    assert!(errors.is_empty(), "route errors: {errors:?}");
}

/// fsl_adc16 ADC16_Init (12-bit, VREFH, single conversion) followed by
/// ADC16_DoAutoCalibration.
fn init_and_calibrate(bus: &mut SystemBus, base: u64, mode: u32) {
    bus.write_u32(base + CFG1, mode).expect("CFG1");
    bus.write_u32(base + SC2, 0).expect("SC2.REFSEL = VREFH");
    bus.write_u32(base + SC3, 0).expect("SC3");
    bus.write_u32(base + SC3, CAL | CALF).expect("SC3.CAL");
    assert_ne!(
        bus.read_u32(base + SC1A).unwrap() & COCO,
        0,
        "calibration COCO"
    );
    assert_eq!(bus.read_u32(base + SC3).unwrap() & (CAL | CALF), 0, "CALF");
    let _ = bus.read_u32(base + RA).expect("dummy R[0]");
    assert_eq!(bus.read_u32(base + OFS).unwrap(), 0, "ideal calibration");
}

/// ADC16_SetChannelConfig(SC1A) + poll COCO + ADC16_GetChannelConversionValue.
fn convert(bus: &mut SystemBus, base: u64, sc1: u32) -> u32 {
    bus.write_u32(base + SC1A, sc1).expect("SC1A");
    assert_ne!(bus.read_u32(base + SC1A).unwrap() & COCO, 0, "COCO");
    let code = bus.read_u32(base + RA).expect("R[0]");
    assert_eq!(
        bus.read_u32(base + SC1A).unwrap() & COCO,
        0,
        "R[0] read clears COCO"
    );
    code
}

#[test]
fn frdm_k64f_arduino_a0_and_a2_convert_ntc_and_diode_on_adc0_and_adc1() {
    let (mut bus, mut session) = board("ptb2", "ptb10");
    init_and_calibrate(&mut bus, ADC0, MODE_12);
    init_and_calibrate(&mut bus, ADC1, MODE_12);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.ptb2_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.ptb10_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc_code = convert(&mut bus, ADC0, AD12);
        assert_eq!(
            ntc_code,
            se_code(ntc_v, 12),
            "ADC0_SE12 at {temperature_c} °C"
        );
        assert_eq!(
            convert(&mut bus, ADC1, AD14),
            se_code(diode_v, 12),
            "ADC1_SE14"
        );
        // The pads are on their own instance only: ADC1 AD12 and ADC0 AD14
        // (PTC0) carry nothing.
        assert_eq!(convert(&mut bus, ADC1, AD12), 0);
        assert_eq!(convert(&mut bus, ADC0, AD14), 0);
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
}

#[test]
fn frdm_k64f_generic_opamp_amplifies_ntc_and_reaches_adc16_with_diode() {
    let system_path = root("configs/systems/frdm-k64f.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load FRDM-K64F profile");
    let fixture =
        SystemManifest::from_file(root("examples/frdm-k64f/analog-opamp-ntc-adc-system.yaml"))
            .expect("load generic NTC, OPAMP, and diode netlist");
    profile.cosim_models = fixture.cosim_models;

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load MK64F12 descriptor");
    for (pad, peripheral, channel) in [
        ("PTB0", "adc0", 8),
        ("PTB2", "adc0", 12),
        ("PTB10", "adc1", 14),
    ] {
        let route = &chip.analog_pins[pad];
        assert_eq!(route.peripheral, peripheral, "{pad} ADC instance");
        assert_eq!(route.channel, channel, "{pad} ADC channel");
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build FRDM-K64F bus");
    init_and_calibrate(&mut bus, ADC0, MODE_12);
    init_and_calibrate(&mut bus, ADC1, MODE_12);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build OPAMP co-simulation")
        .expect("fixture carries a generic OPAMP and two unrelated device models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "NTC, amplifier, and diode outputs must resolve to ADC16 inputs"
    );

    let mut cycles = 0;
    let mut sensor_codes = Vec::new();
    let mut amplifier_codes = Vec::new();
    for temperature_c in [25.0_f64, 50.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        // Let the OPAMP's compensated gain node settle after the resistor
        // divider moves. Six 1 µs boundaries span many closed-loop time
        // constants at this fixture's 1 MHz GBW.
        for _ in 0..6 {
            cycles += session.cycles_until_boundary(cycles);
            let (steps, errors) = session
                .advance_to(cycles, &mut bus)
                .expect("solve and route NTC, OPAMP, and diode");
            assert_eq!(steps.len(), 1, "the complete analog netlist solves");
            assert!(errors.is_empty(), "ADC16 route errors: {errors:?}");
        }

        let sensor_v = signal_voltage(&session, "board.analog.ptb2_volts");
        let amplified_v = signal_voltage(&session, "board.analog.ptb0_volts");
        let diode_v = signal_voltage(&session, "board.analog.ptb10_volts");
        let expected_sensor_v = ntc_volts(temperature_c);
        let expected_amplified_v = noninverting_opamp_output_voltage(expected_sensor_v);
        assert!(
            (sensor_v - expected_sensor_v).abs() < 1e-9,
            "Beta NTC at {temperature_c} °C: expected {expected_sensor_v} V, got {sensor_v} V"
        );
        assert!(
            (amplified_v - expected_amplified_v).abs() < 2e-4,
            "finite-gain OPAMP at {temperature_c} °C: expected {expected_amplified_v} V, got {amplified_v} V"
        );
        assert!(
            diode_v > 0.5 && diode_v < 0.8,
            "unrelated Shockley diode remains at its loaded forward drop: {diode_v} V"
        );

        let sensor_code = convert(&mut bus, ADC0, AD12);
        let amplified_code = convert(&mut bus, ADC0, 8);
        let diode_code = convert(&mut bus, ADC1, AD14);
        assert_eq!(sensor_code, se_code(sensor_v, 12), "ADC0_SE12 sensor");
        assert_eq!(
            amplified_code,
            se_code(amplified_v, 12),
            "ADC0_SE8 amplified NTC"
        );
        assert_eq!(diode_code, se_code(diode_v, 12), "ADC1_SE14 diode");
        assert_eq!(convert(&mut bus, ADC1, AD12), 0, "NTC stays on ADC0");
        assert_eq!(convert(&mut bus, ADC0, AD14), 0, "diode stays on ADC1");
        sensor_codes.push(sensor_code);
        amplifier_codes.push(amplified_code);
    }
    assert!(
        sensor_codes[0] > sensor_codes[1] && sensor_codes[1] > sensor_codes[2],
        "the NTC sensor voltage falls as it warms: {sensor_codes:?}"
    );
    assert!(
        amplifier_codes[0] > amplifier_codes[1] && amplifier_codes[1] > amplifier_codes[2],
        "the closed-loop amplified output follows the NTC: {amplifier_codes:?}"
    );
}

#[test]
fn frdm_k64f_continuous_conversion_started_once_follows_the_ntc() {
    let (mut bus, mut session) = board("ptb2", "ptb10");
    init_and_calibrate(&mut bus, ADC0, MODE_16);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    bus.write_u32(ADC0 + SC3, ADCO).expect("SC3.ADCO");
    bus.write_u32(ADC0 + SC1A, AD12).expect("start once");
    assert_ne!(bus.read_u32(ADC0 + SC2).unwrap() & ADACT, 0, "ADACT");
    for temperature_c in [-10.0_f64, 25.0, 80.0, 25.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.ptb2_volts");
        bus.tick_peripherals_fully();
        assert_ne!(
            bus.read_u32(ADC0 + SC1A).unwrap() & COCO,
            0,
            "the chain completes a new conversion"
        );
        assert_eq!(
            bus.read_u32(ADC0 + RA).unwrap(),
            se_code(ntc_v, 16),
            "16-bit continuous result at {temperature_c} °C"
        );
    }
    // ADCH = 11111 ends the chain: the result stays at the last 25 °C value.
    let held = se_code(ntc_volts(25.0), 16);
    bus.write_u32(ADC0 + SC1A, ADCH_DISABLED).expect("disable");
    assert_eq!(bus.read_u32(ADC0 + SC2).unwrap() & ADACT, 0);
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    bus.tick_peripherals_fully();
    assert_eq!(bus.read_u32(ADC0 + SC1A).unwrap() & COCO, 0);
    assert_eq!(bus.read_u32(ADC0 + RA).unwrap(), held);
}

#[test]
fn frdm_k64f_dad2_converts_the_signed_difference_of_two_netlists() {
    // NTC on ADC0_DP2 (PTE2), diode on ADC0_DM2 (PTE3).
    let (mut bus, mut session) = board("pte2", "pte3");
    init_and_calibrate(&mut bus, ADC0, MODE_12);
    let mut cycles = 0;
    let mut signs = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let plus = millivolts(signal_voltage(&session, "board.analog.pte2_volts"));
        let minus = millivolts(signal_voltage(&session, "board.analog.pte3_volts"));
        // 13-bit differential, 1 LSB = VREFH / 2^12, sign bit 12 extended
        // through bit 15.
        let expected = ((plus - minus) << 12) / 3300;
        let raw = convert(&mut bus, ADC0, DAD2 | DIFF);
        assert_eq!(raw >> 16, 0);
        assert_eq!(
            i64::from(raw as u16 as i16),
            expected,
            "DAD2 at {temperature_c} °C ({plus} - {minus} mV)"
        );
        // Single-ended ADCH 2 reads DADP2 alone.
        assert_eq!(
            convert(&mut bus, ADC0, DAD2),
            ((plus << 12) / 3300) as u32,
            "DADP2"
        );
        signs.push(expected.signum());
    }
    // The NTC divider (2.79 V, 1.65 V, 0.37 V) crosses the ~0.7 V diode
    // between 25 and 80 °C, so the difference changes sign.
    assert_eq!(signs, [1, 1, -1]);
}

#[test]
fn frdm_k64f_jfet_gate_depletion_charge_reaches_adc16_alongside_ntc_and_diode() {
    const STEP_S: f64 = 1e-6;
    const SUBSTEPS: usize = 10;
    let system_path = root("configs/systems/frdm-k64f.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load FRDM-K64F profile");
    let analog = SystemManifest::from_file(root("examples/frdm-k64f/analog-ntc-diode-system.yaml"))
        .expect("load unrelated generic NTC and Shockley diode routes");
    let jfet = SystemManifest::from_file(root(
        "examples/frdm-k64f/analog-jfet-gate-charge-adc-system.yaml",
    ))
    .expect("load matched generic JFET gate-charge branches");
    profile.cosim_models = analog.cosim_models;
    for model in &mut profile.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", "ptb2"),
            "diode_load_line" => ("v_diode", "ptb10"),
            other => panic!("unexpected base model {other}"),
        };
        model
            .outputs
            .insert(key.to_string(), format!("board.analog.{pad}_volts"));
    }
    profile.cosim_models.extend(jfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load MK64F12 descriptor");
    for (pad, peripheral, channel) in [
        ("PTB2", "adc0", 12),
        ("PTB3", "adc0", 13),
        ("PTC0", "adc0", 14),
        ("PTB10", "adc1", 14),
    ] {
        let route = &chip.analog_pins[pad];
        assert_eq!(route.peripheral, peripheral, "{pad} ADC instance");
        assert_eq!(route.channel, channel, "{pad} channel");
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build FRDM-K64F bus");
    init_and_calibrate(&mut bus, ADC0, MODE_12);
    init_and_calibrate(&mut bus, ADC1, MODE_12);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries NTC, diode, and matched generic JFET charge branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "the generic devices must resolve through PTB2/PTB3/PTC0/PTB10 to ADC16"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");
    session
        .set_signal_number("ui.jfet_gate.drive_v", 3.3)
        .expect("precharge both JFET gates at their drain/source reference");
    let mut cycles = session.cycles_until_boundary(0);
    let (initial_steps, initial_errors) = session
        .advance_to(cycles, &mut bus)
        .expect("settle NTC, diode, and both JFET branches");
    assert_eq!(initial_steps.len(), 4, "all four analog netlists solve");
    assert!(
        initial_errors.is_empty(),
        "route errors: {initial_errors:?}"
    );
    assert!(
        (signal_voltage(&session, "board.analog.ptb3_volts") - 3.3).abs() < 1e-9,
        "the precharged JFET gate starts at the drain/source reference"
    );

    session
        .set_signal_number("ui.jfet_gate.drive_v", 0.0)
        .expect("step both matched JFET gate drives low");
    let mut expected_gate_v = 3.3;
    let mut final_gate_v = 3.3;
    for step in 0..8 {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route JFET charge alongside NTC and diode");
        assert_eq!(
            steps.len(),
            4,
            "all analog netlists solve on each transient step"
        );
        assert!(
            errors.is_empty(),
            "unexpected ADC16 route errors: {errors:?}"
        );

        for _ in 0..SUBSTEPS {
            expected_gate_v =
                expected_jfet_gate_voltage(expected_gate_v, 0.0, STEP_S / SUBSTEPS as f64);
        }
        let gate_v = signal_voltage(&session, "board.analog.ptb3_volts");
        let control_v = signal_voltage(&session, "board.analog.ptc0_volts");
        assert!(
            (gate_v - expected_gate_v).abs() < 2e-8,
            "JFET CGS/CGD charge at step {step}: expected {expected_gate_v} V, got {gate_v} V"
        );
        assert!(
            control_v.abs() < 1e-8,
            "zero-charge control follows the low drive within the leakage floor, got {control_v} V"
        );
        assert!(
            gate_v > control_v,
            "depletion charge must delay the gate edge relative to its zero-charge control"
        );

        let ntc_v = signal_voltage(&session, "board.analog.ptb2_volts");
        assert!(
            (ntc_v - ntc_volts(25.0)).abs() < 1e-9,
            "the NTC remains independent"
        );
        let diode_v = signal_voltage(&session, "board.analog.ptb10_volts");
        assert!(
            (diode_v - diode_load_line_voltage()).abs() < 2e-5,
            "the unrelated Shockley diode remains at its independent load line"
        );

        for (adc, channel, voltage_v) in [
            (ADC0, AD12, ntc_v),
            (ADC0, 13, gate_v),
            (ADC0, AD14, control_v),
            (ADC1, AD14, diode_v),
        ] {
            assert_eq!(
                convert(&mut bus, adc, channel),
                se_code(voltage_v, 12),
                "ADC16 {adc:#x} channel {channel} follows its routed analog device"
            );
        }
        final_gate_v = gate_v;
    }
    assert!(
        final_gate_v < 3e-3,
        "the charged gate converges within the MNA leakage/convergence floor of its zero-volt control: {final_gate_v} V"
    );
}

#[test]
fn frdm_k64f_bjt_early_effect_reaches_adc16_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/frdm-k64f.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load FRDM-K64F profile");
    let unrelated =
        SystemManifest::from_file(root("examples/frdm-k64f/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC divider and Shockley diode");
    let bjt =
        SystemManifest::from_file(root("examples/frdm-k64f/analog-bjt-early-adc-system.yaml"))
            .expect("load matched generic Gummel–Poon Early-effect branches");
    profile.cosim_models = unrelated.cosim_models;
    for model in &mut profile.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", "ptb2"),
            "diode_load_line" => ("v_diode", "ptb10"),
            other => panic!("unexpected base model {other}"),
        };
        model
            .outputs
            .insert(key.to_string(), format!("board.analog.{pad}_volts"));
    }
    profile.cosim_models.extend(bjt.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load MK64F12 descriptor");
    for (pad, channel) in [("PTB0", 8), ("PTB1", 9), ("PTB2", 12)] {
        assert_eq!(
            chip.analog_pins[pad].peripheral, "adc0",
            "{pad} ADC instance"
        );
        assert_eq!(chip.analog_pins[pad].channel, channel, "{pad} ADC channel");
    }
    assert_eq!(chip.analog_pins["PTB10"].peripheral, "adc1");
    assert_eq!(chip.analog_pins["PTB10"].channel, 14);

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build FRDM-K64F bus");
    init_and_calibrate(&mut bus, ADC0, MODE_12);
    init_and_calibrate(&mut bus, ADC1, MODE_12);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an NTC, diode, and matched BJT branches");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic BJT, NTC, and diode routes must resolve through K64F ADC16 inputs"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");
    session
        .set_signal_number("ui.bjt_early.base_v", 0.0)
        .expect("initialize both matched BJT base drives");
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut early_samples = Vec::new();
    let mut control_samples = Vec::new();

    for base_drive_v in [0.0_f64, 0.7, 0.8, 0.9] {
        session
            .set_signal_number("ui.bjt_early.base_v", base_drive_v)
            .expect("set common BJT base drive");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route NTC, diode, and matched BJT branches");
        assert_eq!(steps.len(), 4, "all four analog branches solve");
        assert!(
            errors.is_empty(),
            "unexpected ADC16 route errors: {errors:?}"
        );

        let early_v = signal_voltage(&session, "board.analog.ptb0_volts");
        let control_v = signal_voltage(&session, "board.analog.ptb1_volts");
        let ntc_v = signal_voltage(&session, "board.analog.ptb2_volts");
        let diode_v = signal_voltage(&session, "board.analog.ptb10_volts");
        let expected_early_v = bjt_early_collector_voltage(base_drive_v, Some(50.0));
        let expected_control_v = bjt_early_collector_voltage(base_drive_v, None);
        assert!((early_v - expected_early_v).abs() < 5e-5,
            "VAF=50 load line should be {expected_early_v} V at {base_drive_v} V drive, got {early_v} V");
        assert!((control_v - expected_control_v).abs() < 5e-5,
            "matched no-forward-Early load line should be {expected_control_v} V at {base_drive_v} V drive, got {control_v} V");
        assert!(
            (ntc_v - ntc_volts(25.0)).abs() < 1e-9,
            "unrelated 25 °C NTC stays isolated"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains at its independent load line"
        );
        if base_drive_v >= 0.7 {
            assert!(early_v < control_v - 0.001,
                "finite VAF increases collector current and lowers collector voltage: {early_v} V vs {control_v} V");
        }

        for (adc, channel, voltage_v) in [
            (ADC0, 8, early_v),
            (ADC0, 9, control_v),
            (ADC0, AD12, ntc_v),
            (ADC1, AD14, diode_v),
        ] {
            assert_eq!(
                convert(&mut bus, adc, channel),
                se_code(voltage_v, 12),
                "ADC16 {adc:#x} channel {channel} follows its independently checked analog route"
            );
        }
        early_samples.push(early_v);
        control_samples.push(control_v);
    }

    assert!(
        early_samples[1] > early_samples[2] && early_samples[2] > early_samples[3],
        "increasing base drive lowers the finite-VAF collector voltage: {early_samples:?}"
    );
    assert!(
        control_samples[1] > control_samples[2] && control_samples[2] > control_samples[3],
        "increasing base drive lowers the no-forward-Early collector voltage: {control_samples:?}"
    );
}

/// Level-1 MOSFET lateral diffusion `LD` and the `UO`/`TOX`-derived `KP` on
/// the K64F ADC16 path. The three matched stages differ only in how
/// `KP·W/Leff` is formed; an engine that ignored `LD` or `UO` would put two
/// of them on the reference load line and fail the independent check.
#[test]
fn frdm_k64f_mosfet_lateral_diffusion_and_process_kp_reach_adc16_alongside_ntc_and_diode() {
    let system_path = root("configs/systems/frdm-k64f.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load FRDM-K64F profile");
    let unrelated =
        SystemManifest::from_file(root("examples/frdm-k64f/analog-ntc-diode-system.yaml"))
            .expect("load unrelated generic NTC divider and Shockley diode");
    let mosfet = SystemManifest::from_file(root(
        "examples/frdm-k64f/analog-mosfet-ld-uo-adc-system.yaml",
    ))
    .expect("load matched generic level-1 NMOS stages");
    profile.cosim_models = unrelated.cosim_models;
    for model in &mut profile.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", "ptb2"),
            "diode_load_line" => ("v_diode", "ptb10"),
            other => panic!("unexpected base model {other}"),
        };
        model
            .outputs
            .insert(key.to_string(), format!("board.analog.{pad}_volts"));
    }
    profile.cosim_models.extend(mosfet.cosim_models);

    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load MK64F12 descriptor");
    for (pad, channel) in [("PTB0", 8), ("PTB1", 9), ("PTB2", 12), ("PTB3", 13)] {
        assert_eq!(
            chip.analog_pins[pad].peripheral, "adc0",
            "{pad} ADC instance"
        );
        assert_eq!(chip.analog_pins[pad].channel, channel, "{pad} ADC channel");
    }

    let mut bus = SystemBus::from_config(&chip, &profile).expect("build FRDM-K64F bus");
    init_and_calibrate(&mut bus, ADC0, MODE_12);
    init_and_calibrate(&mut bus, ADC1, MODE_12);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries an NTC, diode, and matched NMOS stages");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "generic NMOS, NTC, and diode routes must resolve through K64F ADC16 inputs"
    );
    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set unrelated NTC temperature");

    let (w, l) = (10e-6, 2e-6);
    let beta_ld = 50e-6 * w / (l - 2.0 * 0.5e-6);
    let beta_uo = process_kp(400.0, 20e-9) * w / l;
    let beta_ref = 50e-6 * w / l;
    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    for gate_v in [0.0_f64, 1.5, 2.0, 2.5] {
        session
            .set_signal_number("ui.mosfet_ld_uo.gate_v", gate_v)
            .expect("set common NMOS gate drive");
        // Two boundaries: the TOX stage's Meyer charge settles in the first.
        for _ in 0..2 {
            cycles += session.cycles_until_boundary(cycles);
            let (steps, errors) = session
                .advance_to(cycles, &mut bus)
                .expect("solve and route NTC, diode, and NMOS stages");
            assert_eq!(steps.len(), 3, "all three analog netlists solve");
            assert!(
                errors.is_empty(),
                "unexpected ADC16 route errors: {errors:?}"
            );
        }

        let ld_v = signal_voltage(&session, "board.analog.ptb0_volts");
        let uo_v = signal_voltage(&session, "board.analog.ptb1_volts");
        let ref_v = signal_voltage(&session, "board.analog.ptb3_volts");
        for (label, actual_v, beta) in [
            ("LD=0.5u", ld_v, beta_ld),
            ("UO=400 TOX=20n", uo_v, beta_uo),
            ("reference", ref_v, beta_ref),
        ] {
            let expected_v = nmos_drain_voltage(gate_v, beta);
            assert!(
                (actual_v - expected_v).abs() < 5e-5,
                "{label} drain at {gate_v} V gate should be {expected_v} V, got {actual_v} V"
            );
        }
        if gate_v > 1.0 {
            assert!(
                ld_v < uo_v - 0.05 && uo_v < ref_v - 0.05,
                "LD and process KP raise drain current above the reference: {ld_v} / {uo_v} / {ref_v} V"
            );
        }
        let ntc_v = signal_voltage(&session, "board.analog.ptb2_volts");
        let diode_v = signal_voltage(&session, "board.analog.ptb10_volts");
        assert!(
            (ntc_v - ntc_volts(25.0)).abs() < 1e-9,
            "unrelated NTC stays isolated"
        );
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "unrelated Shockley diode remains at its independent load line"
        );

        for (adc, channel, voltage_v) in [
            (ADC0, 8, ld_v),
            (ADC0, 9, uo_v),
            (ADC0, AD12, ntc_v),
            (ADC0, 13, ref_v),
            (ADC1, AD14, diode_v),
        ] {
            assert_eq!(
                convert(&mut bus, adc, channel),
                se_code(voltage_v, 12),
                "ADC16 {adc:#x} channel {channel} follows its independently checked analog route"
            );
        }
    }
}
