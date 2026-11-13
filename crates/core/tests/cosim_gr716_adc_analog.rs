// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the GR716 devboard's
//! on-chip ADC (`gr716_adc`) through the onboarding descriptor's
//! `analog_pins` map (GR716-MINI user manual: ADC0..ADC7 = GPIO37..GPIO44):
//! the NTC on GPIO37 (ADC0) and the diode on GPIO44 (ADC7), read the way the
//! Phoenix-RTOS grlib-multi driver does it (`adc_configure`: ACFG = scaler |
//! AI | AM | AE, PACFG = AB, AMASK = IE; `adc_convert`: ACFG |= AS, wait for
//! ASTS bit 31, read `ASTS & 0x7FFFF`; the ISR write-1-clears AINT.IE) at 11
//! bits over the 0 V to 2.5 V single-ended range (GR716-DS-UM §12.2). Also:
//! - every control unit reaches every pad through the shared MUX, so unit 3
//!   with AI = 0 reads the same NTC code as unit 0;
//! - ASAMPC.AO = 3 accumulates four samples without dividing (§12.2), and
//!   the pre-amplifier at AG = 6 dB (AB = 0) doubles the diode code;
//! - the netlists moved to GPIO41/GPIO42 convert there, and the old pads go
//!   back to 0 V.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC: u64 = 0x8040_0000;
const UNIT: u64 = 0x1000;
const ACFG: u64 = 0x00;
const ASAMPC: u64 = 0x04;
const ASTS: u64 = 0x10;
const AINT: u64 = 0x14;
const AMASK: u64 = 0x18;
const PACFG: u64 = 0x1C;
const AE: u32 = 1 << 15;
const AS: u32 = 1 << 14;
const AM: u32 = 1 << 0;
const AB: u32 = 1 << 2;
const IE: u32 = 1 << 0;
const VALID: u32 = 1 << 31;
/// 50 MHz / (200 kSps * 20), the Phoenix-RTOS default scaler.
const SCALER: u32 = 12;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn ntc_volts(temperature_c: f64) -> f64 {
    let r = ntc_resistance_ohms(temperature_c);
    3.3 * r / (10_000.0 + r)
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn millivolts(volts: f64) -> u32 {
    (volts * 1000.0).round() as u32
}

/// 11 bits, 2^11 / 2.5 V, truncated, clamped.
fn code(volts: f64) -> u32 {
    (millivolts(volts) * 2048 / 2500).min(2047)
}

/// The onboarding GR716 devboard with the generic NTC and diode netlists
/// routed to the two pads.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/onboarding/gr716-devboard.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load GR716 devboard");
    let analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load GR716 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build GR716 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to ADC pads through analog_pins"
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

/// Phoenix-RTOS `adc_configure` on control unit `unit` for pad `pad`
/// (PACFG = `pacfg`), then `adc_convert`. Returns `ASTS & 0x7FFFF`.
fn convert(bus: &mut SystemBus, unit: u64, pad: u32, pacfg: u32) -> u32 {
    let base = ADC + unit * UNIT;
    bus.write_u32(base + ACFG, (SCALER << 16) | (pad << 2) | AM | AE)
        .expect("ACFG");
    bus.write_u32(base + PACFG, pacfg).expect("PACFG");
    bus.write_u32(base + AMASK, IE).expect("AMASK.IE");
    let cfg = bus.read_u32(base + ACFG).unwrap();
    bus.write_u32(base + ACFG, cfg | AS).expect("ACFG.AS");
    assert_eq!(
        bus.read_u32(base + ACFG).unwrap() & AS,
        0,
        "AS drops at end of conversion"
    );
    let sts = bus.read_u32(base + ASTS).unwrap();
    assert_ne!(sts & VALID, 0, "unit {unit}: ASTS valid");
    assert_eq!(bus.read_u32(base + AINT).unwrap(), IE, "unit {unit}: IE");
    bus.write_u32(base + AINT, IE).expect("ISR clears AINT.IE");
    assert_eq!(bus.read_u32(base + AINT).unwrap(), 0);
    sts & 0x7FFFF
}

#[test]
fn gr716_gpio37_and_gpio44_convert_ntc_and_diode_on_their_phoenix_units() {
    let (mut bus, mut session) = board("gpio37", "gpio44");
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.gpio37_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.gpio44_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc_code = convert(&mut bus, 0, 0, AB);
        assert_eq!(ntc_code, code(ntc_v), "ADC0 at {temperature_c} °C");
        assert_eq!(convert(&mut bus, 7, 7, AB), code(diode_v), "ADC7");
        // The shared MUX: unit 3 selects pad ADC0 through AI.
        assert_eq!(convert(&mut bus, 3, 0, AB), ntc_code, "unit 3, AI = 0");
        assert_eq!(convert(&mut bus, 3, 3, AB), 0, "unrouted GPIO40 reads 0");
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    // The -10 °C divider (2.79 V) is above the 2.5 V single-ended range.
    assert_eq!(ntc_codes[0], 2047, "-10 °C saturates");
    // The firmware inverse, code * 2500 / 2048, recovers the 25 °C divider.
    let mv = ntc_codes[1] * 2500 / 2048;
    assert!(
        mv.abs_diff(millivolts(ntc_volts(25.0))) <= 2,
        "code * VREF / 2^11 gives {mv} mV"
    );
}

#[test]
fn gr716_oversampling_and_preamplifier_gain_scale_the_routed_diode() {
    let (mut bus, mut session) = board("gpio37", "gpio44");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let diode_v = signal_voltage(&session, "board.analog.gpio44_volts");
    let single = convert(&mut bus, 7, 7, AB);
    assert_eq!(single, code(diode_v));
    // ASAMPC.AO = 3: four samples accumulated, software divides.
    bus.write_u32(ADC + 7 * UNIT + ASAMPC, 3)
        .expect("ASAMPC.AO");
    let sum = convert(&mut bus, 7, 7, AB);
    assert_eq!(sum, 4 * single, "AO + 1 samples summed, not divided");
    bus.write_u32(ADC + 7 * UNIT + ASAMPC, 0).expect("ASAMPC");
    // AB = 0, AG = b01 (6 dB): the amplifier doubles the grounded-negative
    // single-ended input; 2 x ~0.6 V stays inside 2.5 V.
    let amplified = convert(&mut bus, 7, 7, 1);
    assert_eq!(amplified, code(2.0 * diode_v), "x2 pre-amplifier");
    assert!(amplified > single + single / 2);
}

#[test]
fn gr716_gpio41_and_gpio42_convert_after_the_netlists_move() {
    let (mut bus, mut session) = board("gpio41", "gpio42");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_v = signal_voltage(&session, "board.analog.gpio41_volts");
    let diode_v = signal_voltage(&session, "board.analog.gpio42_volts");
    assert_eq!(convert(&mut bus, 4, 4, AB), code(ntc_v), "ADC4 (GPIO41)");
    assert_eq!(convert(&mut bus, 5, 5, AB), code(diode_v), "ADC5 (GPIO42)");
    assert_eq!(convert(&mut bus, 0, 0, AB), 0, "GPIO37 is not driven");
    assert_eq!(convert(&mut bus, 7, 7, AB), 0, "GPIO44 is not driven");
}
