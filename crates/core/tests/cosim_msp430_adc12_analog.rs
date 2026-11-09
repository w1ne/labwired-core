// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the MSP430 ADC12 through
//! the `analog_pins` map: the NTC on pad `A0` (P6.0) and the diode on `A1`
//! (P6.1), on both register maps (F1611 ADC12 from 0x0080, F5437 ADC12_A at
//! 0x0700). What the explicit `adc.adc12.<ch>_volts` sweep in
//! `cosim_onboarding_analog.rs` does not cover:
//! - one `CONSEQ_3` repeat-sequence started ONCE, which keeps following the
//!   NTC as its temperature moves, with no further `ADC12SC`;
//! - the diode against the internal 2.5 V reference (`SREF_1`, `REFON`,
//!   `REF2_5V`), whose codes differ from the AVCC ones by 3.3 / 2.5;
//! - on ADC12_A, `ADC12RES` = 10 bits on the same inputs.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC12SC: u16 = 1 << 0;
const ENC: u16 = 1 << 1;
const ADC12ON: u16 = 1 << 4;
const REFON: u16 = 1 << 5;
const REF2_5V: u16 = 1 << 6;
const MSC: u16 = 1 << 7;
const SHT0_2: u16 = 0x0200;
const SHP: u16 = 1 << 9;
const CONSEQ_3: u16 = 0x0006;
const BUSY: u16 = 1 << 0;
const EOS: u8 = 1 << 7;
const SREF_1: u8 = 0x10;

/// One register map, as absolute addresses.
struct Map {
    profile: &'static str,
    ctl0: u64,
    ctl1: u64,
    ctl2: Option<u64>,
    ifg: u64,
    mctl: u64,
    mem: u64,
}

/// msp430f1611_symbols.ld: MCTL0 0x0080, MEM0 0x0140, CTL0 0x01A0.
const F1611: Map = Map {
    profile: "mspsim-msp430f1611",
    ctl0: 0x01A0,
    ctl1: 0x01A2,
    ctl2: None,
    ifg: 0x01A4,
    mctl: 0x0080,
    mem: 0x0140,
};

/// msp430f5437_symbols.ld: the ADC12_A block at 0x0700.
const F5437: Map = Map {
    profile: "mspsim-msp430f5437",
    ctl0: 0x0700,
    ctl1: 0x0702,
    ctl2: Some(0x0704),
    ifg: 0x070A,
    mctl: 0x0710,
    mem: 0x0720,
};

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
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

/// `N = full * Vin / VR+`, truncated, clamped (SREF with VR- = AVSS).
fn code(volts: f64, vref_mv: u32, full: u32) -> u32 {
    millivolts(volts).min(vref_mv) * full / vref_mv
}

fn mem(bus: &mut SystemBus, map: &Map, slot: u64) -> u32 {
    u32::from(bus.read_u16(map.mem + 2 * slot).unwrap())
}

fn netlists_on_a0_a1(map: &Map) -> (SystemBus, CosimSession) {
    let system_path = root(&format!("configs/systems/onboarding/{}.yaml", map.profile));
    let mut profile = SystemManifest::from_file(&system_path).expect("load MSP430 profile");
    // Anchor the chip at the system file, as the CLI does, so the ports'
    // declarative `path` resolves next to the chip.
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    profile.chip = chip_path.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(&chip_path).expect("load MSP430 descriptor");
    let mut analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    for model in &mut analog.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", "a0"),
            "diode_load_line" => ("v_diode", "a1"),
            other => panic!("unexpected fixture model {other}"),
        };
        model
            .outputs
            .insert(key.into(), format!("board.analog.{pad}_volts"));
    }
    profile.cosim_models = analog.cosim_models;
    let bus = SystemBus::from_config(&chip, &profile).expect("build MSP430 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{}: A0/A1 must resolve through analog_pins to ADC12 inputs 0/1",
        map.profile
    );
    (bus, session)
}

/// Solve both netlists at `temperature_c` and return (NTC, diode) volts.
fn solve(
    session: &mut CosimSession,
    bus: &mut SystemBus,
    cycles: &mut u64,
    temperature_c: f64,
) -> (f64, f64) {
    session
        .set_signal_number("ui.ntc.temperature_c", temperature_c)
        .expect("set NTC temperature");
    *cycles += session.cycles_until_boundary(*cycles);
    let (steps, errors) = session
        .advance_to(*cycles, bus)
        .expect("solve and route both netlists");
    assert_eq!(steps.len(), 2, "both netlists solve each step");
    assert!(errors.is_empty(), "route errors: {errors:?}");
    let ntc_v = signal_voltage(session, "board.analog.a0_volts");
    let r = ntc_resistance_ohms(temperature_c);
    let expected = 3.3 * r / (10_000.0 + r);
    assert!(
        (ntc_v - expected).abs() < 1e-9,
        "at {temperature_c} °C the divider should be {expected} V, got {ntc_v} V"
    );
    let diode_v = signal_voltage(session, "board.analog.a1_volts");
    assert!(
        (0.5..0.75).contains(&diode_v),
        "a forward-biased silicon diode: {diode_v} V"
    );
    (ntc_v, diode_v)
}

/// Repeat-sequence MCTL0..3: NTC (AVCC), diode (AVCC), diode (internal
/// 2.5 V), the unrouted A2 with EOS. Started once with `MSC`.
fn start_repeat_sequence(bus: &mut SystemBus, map: &Map) {
    let ctl0 = SHT0_2 | ADC12ON | REFON | REF2_5V | MSC;
    bus.write_u16(map.ctl0, ctl0).unwrap();
    bus.write_u16(map.ctl1, SHP | CONSEQ_3).unwrap();
    for (slot, mctl) in [0, 1, SREF_1 | 1, 2 | EOS].into_iter().enumerate() {
        bus.write_u8(map.mctl + slot as u64, mctl).unwrap();
    }
    bus.write_u16(map.ctl0, ctl0 | ENC | ADC12SC).unwrap();
    assert_eq!(
        bus.read_u16(map.ifg).unwrap(),
        0b1111,
        "{}: the first pass converts MCTL0..3",
        map.profile
    );
    assert_ne!(
        bus.read_u16(map.ctl1).unwrap() & BUSY,
        0,
        "{}: the repeat sequence stays busy",
        map.profile
    );
}

fn assert_repeat_follows_ntc(map: &Map) {
    let (mut bus, mut session) = netlists_on_a0_a1(map);
    let mut cycles = 0;
    solve(&mut session, &mut bus, &mut cycles, -10.0);
    start_repeat_sequence(&mut bus, map);
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        let (ntc_v, diode_v) = solve(&mut session, &mut bus, &mut cycles, temperature_c);
        // No ADC12SC: the running CONSEQ_3 pass samples the new levels.
        bus.tick_peripherals_fully();
        let tag = format!("{} at {temperature_c} °C", map.profile);
        let ntc = mem(&mut bus, map, 0);
        assert_eq!(ntc, code(ntc_v, 3300, 4095), "{tag}: A0 NTC against AVCC");
        assert_eq!(
            mem(&mut bus, map, 1),
            code(diode_v, 3300, 4095),
            "{tag}: A1 diode against AVCC"
        );
        let internal = mem(&mut bus, map, 2);
        assert_eq!(
            internal,
            code(diode_v, 2500, 4095),
            "{tag}: A1 diode against the internal 2.5 V reference"
        );
        assert!(
            internal > mem(&mut bus, map, 1),
            "{tag}: the smaller reference gives the larger code"
        );
        assert_eq!(mem(&mut bus, map, 3), 0, "{tag}: unrouted A2 reads 0 V");
        ntc_codes.push(ntc);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "{}: A0 codes fall as the low-side NTC heats: {ntc_codes:?}",
        map.profile
    );

    // Clearing ENC stops the repeat: a new temperature no longer lands.
    let ctl0 = bus.read_u16(map.ctl0).unwrap();
    bus.write_u16(map.ctl0, ctl0 & !ENC).unwrap();
    let held = mem(&mut bus, map, 0);
    solve(&mut session, &mut bus, &mut cycles, -10.0);
    bus.tick_peripherals_fully();
    assert_eq!(
        mem(&mut bus, map, 0),
        held,
        "{}: ENC clear holds MEM0",
        map.profile
    );
    assert_eq!(bus.read_u16(map.ctl1).unwrap() & BUSY, 0);
}

#[test]
fn msp430f1611_adc12_repeat_sequence_follows_the_routed_ntc_and_diode() {
    assert_repeat_follows_ntc(&F1611);
}

#[test]
fn msp430f5437_adc12_a_repeat_sequence_follows_the_routed_ntc_and_diode() {
    assert_repeat_follows_ntc(&F5437);
}

#[test]
fn msp430f5437_adc12_a_resolution_scales_the_routed_codes() {
    let (mut bus, mut session) = netlists_on_a0_a1(&F5437);
    let mut cycles = 0;
    let (ntc_v, diode_v) = solve(&mut session, &mut bus, &mut cycles, 25.0);
    let ctl2 = F5437.ctl2.unwrap();
    assert_eq!(bus.read_u16(ctl2).unwrap(), 0x0020, "ADC12RES_2 at reset");
    for (res, full) in [(0x0000u16, 255u32), (0x0010, 1023), (0x0020, 4095)] {
        let ctl0 = SHT0_2 | ADC12ON | MSC;
        bus.write_u16(F5437.ctl0, ctl0).unwrap();
        bus.write_u16(ctl2, res).unwrap();
        // CONSEQ_1 over A0, A1 (EOS).
        bus.write_u16(F5437.ctl1, SHP | 0x0002).unwrap();
        bus.write_u8(F5437.mctl, 0).unwrap();
        bus.write_u8(F5437.mctl + 1, 1 | EOS).unwrap();
        bus.write_u16(F5437.ctl0, ctl0 | ENC | ADC12SC).unwrap();
        assert_eq!(bus.read_u16(F5437.ifg).unwrap(), 0b11);
        assert_eq!(
            mem(&mut bus, &F5437, 0),
            code(ntc_v, 3300, full),
            "NTC at RES {res:#x}"
        );
        assert_eq!(
            mem(&mut bus, &F5437, 1),
            code(diode_v, 3300, full),
            "diode at RES {res:#x}"
        );
        assert_eq!(bus.read_u16(F5437.ifg).unwrap(), 0, "MEM reads clear IFG");
    }
}
