// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the EK-RX261 S12AD
//! (12-bit A/D converter, on the unchanged `ra_adc12` model) through the
//! descriptor's `analog_pins` map (R01UH1045EJ0100 Table 1.5: AN000 = P40,
//! AN001 = P41, AN016 = PE0):
//! - on the bus: the solved voltages against an independent divider and
//!   Shockley load-line reference, and the 12-bit `ADDRn` codes of a
//!   software-started single scan at -10/25/50/80 °C;
//! - the netlist moved to PE0 converts in `ADDR16` (`ADANSA1` bit 0), and the
//!   removed input-bank path no longer binds;
//! - in a `Session`: the committed bare-metal firmware
//!   (examples/ek-rx261/firmware/adc-ntc-diode.c) starts the scan, polls
//!   `ADCSR.ADST` and prints both codes on SCI6 while the session solves the
//!   netlists.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::*;
use labwired_core::Bus;
use std::path::Path;
use std::time::Duration;

const S12AD: u64 = 0x0008_9000;
const ADCSR: u64 = S12AD;
const ADANSA0: u64 = S12AD + 0x04;
const ADANSA1: u64 = S12AD + 0x06;
const ADST: u16 = 1 << 15;
/// VREFH0 = AVCC0 (the descriptor's `vref_mv`, an assumed 3.3 V supply).
const VREF_MV: u32 = 3300;
const FIRMWARE: &str = "examples/ek-rx261/firmware/ek-rx261-adc-ntc-diode.elf";
const ANALOG_SYSTEM: &str = "examples/ek-rx261/analog-ntc-diode-system.yaml";

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn ntc_volts(temperature_c: f64) -> f64 {
    let r = ntc_resistance_ohms(temperature_c);
    3.3 * r / (10_000.0 + r)
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

fn voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

/// 12-bit, `ADDRn` = mV * 4095 / VREFH0, truncated, clamped.
fn code(volts: f64) -> u32 {
    let mv = (volts * 1000.0).round() as u32;
    (mv * 4095 / VREF_MV).min(4095)
}

/// The EK-RX261 with the example's netlists, their outputs re-pointed to
/// `ntc_path` and `diode_path`.
fn board(ntc_path: &str, diode_path: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/ek-rx261.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load EK-RX261 profile");
    assert_eq!(profile.name, "ek-rx261");
    let analog = SystemManifest::from_file(root(ANALOG_SYSTEM))
        .expect("load generic NTC divider and Shockley diode");
    profile.cosim_models = analog.cosim_models;
    for model in &mut profile.cosim_models {
        let (key, path) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", ntc_path),
            "diode_load_line" => ("v_diode", diode_path),
            other => panic!("unexpected model {other}"),
        };
        model.outputs.insert(key.to_string(), path.to_string());
    }
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    profile.chip = chip_path.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(&chip_path).expect("load RX261 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build EK-RX261 system bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
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

/// One software-started single scan of the `ADANSA1:ADANSA0` channel set:
/// `ADST` stays set while it runs and clears at scan end.
fn single_scan(bus: &mut SystemBus, channels: &[u8]) -> Vec<u32> {
    let mask = channels.iter().fold(0u32, |m, &ch| m | (1 << ch));
    bus.write_u16(ADANSA0, mask as u16).expect("ADANSA0");
    bus.write_u16(ADANSA1, (mask >> 16) as u16)
        .expect("ADANSA1");
    bus.write_u16(ADCSR, ADST)
        .expect("ADCSR.ADST, ADCS = single scan");
    let start = bus.current_cycle;
    bus.set_current_cycle(start + 1);
    bus.tick_peripherals_fully();
    assert_ne!(bus.read_u16(ADCSR).unwrap() & ADST, 0, "scan running");
    // A nominal 1 us per channel at the 64 MHz ICLK.
    bus.set_current_cycle(start + 64 * channels.len() as u64 + 1);
    bus.tick_peripherals_fully();
    assert_eq!(
        bus.read_u16(ADCSR).unwrap() & ADST,
        0,
        "ADST clears at scan end"
    );
    channels
        .iter()
        .map(|&ch| u32::from(bus.read_u16(S12AD + 0x20 + 2 * u64::from(ch)).unwrap()))
        .collect()
}

#[test]
fn ek_rx261_s12ad_converts_the_ntc_on_p40_and_the_diode_on_p41() {
    let (mut bus, mut session) = board("board.analog.P40_volts", "board.analog.P41_volts");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "P40/P41 must resolve to AN000/AN001 through analog_pins"
    );
    let diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 50.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = voltage(&session, "board.analog.P40_volts");
        assert!(
            (ntc_v - ntc_volts(temperature_c)).abs() < 1e-9,
            "at {temperature_c} °C the divider is {} V, solved {ntc_v} V",
            ntc_volts(temperature_c)
        );
        let solved_diode_v = voltage(&session, "board.analog.P41_volts");
        assert!(
            (solved_diode_v - diode_v).abs() < 2e-5,
            "the Shockley load line is {diode_v} V, solved {solved_diode_v} V"
        );
        let codes = single_scan(&mut bus, &[0, 1, 2]);
        assert_eq!(
            codes,
            vec![code(ntc_v), code(solved_diode_v), 0],
            "AN000/AN001 at {temperature_c} °C; undriven AN002 reads 0"
        );
        assert!(
            codes[0] > 0 && codes[0] < 4095,
            "{temperature_c} °C in range"
        );
        ntc_codes.push(codes[0]);
    }
    // 25 °C: the NTC equals the 10 k top resistor, half of VREFH0.
    assert_eq!(ntc_codes[1], 0x7FF);
    assert!(
        ntc_codes.windows(2).all(|w| w[0] > w[1]),
        "a warmer NTC converts lower: {ntc_codes:?}"
    );
}

#[test]
fn ek_rx261_an016_on_pe0_converts_in_addr16_and_the_old_bank_is_gone() {
    let (mut bus, mut session) = board("board.analog.PE0_volts", "board.analog.P41_volts");
    assert_eq!(session.binding_errors(), &[] as &[RoutingError]);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 50.0);
    let codes = single_scan(&mut bus, &[0, 16]);
    assert_eq!(codes, vec![0, code(ntc_volts(50.0))], "AN000 is idle now");

    let (_, session) = board("board.gpio_in.port.0", "board.analog.P41_volts");
    assert_eq!(
        session.binding_errors().len(),
        1,
        "the input_pin_bank endpoint was removed: {:?}",
        session.binding_errors()
    );
}

fn open_firmware(system: &str) -> Session {
    let (chip, manifest) = common::system(system);
    let blobs = BlobMap::new();
    let fw = common::committed(FIRMWARE);
    Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(&fw),
            boot: BootMode::FastBoot,
            blobs: &blobs,
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .unwrap()
}

/// Nothing is routed on the plain board: both inputs convert 0 V. The
/// firmware prints only after `ADST` clears, so this also proves the scan
/// ends.
#[test]
fn ek_rx261_adc_firmware_converts_through_build_machine() {
    let mut s = open_firmware("examples/ek-rx261/system.yaml");
    let r = s.expect("N=000 D=000", Duration::from_secs(1));
    let transcript = s.uart_transcript();
    r.unwrap_or_else(|e| panic!("{e}\n--- console ---\n{transcript}"));
}

/// The whole loop in one `Session`: the session solves the example's NTC
/// divider and diode netlists, routes them to P40/AN000 and P41/AN001, and
/// the firmware converts and prints them. The NTC moves from 25 °C to 80 °C
/// while the firmware runs, and the printed code follows.
#[test]
fn ek_rx261_adc_firmware_converts_the_solved_netlists_in_a_session() {
    let mut s = open_firmware(ANALOG_SYSTEM);
    let diode = code(diode_load_line_voltage());
    for temperature_c in [25.0, 80.0] {
        s.set_signal("ui.ntc.temperature_c", temperature_c).unwrap();
        let line = format!("N={:03X} D={diode:03X}", code(ntc_volts(temperature_c)));
        let r = s.expect(&line, Duration::from_secs(1));
        let transcript = s.uart_transcript();
        r.unwrap_or_else(|e| panic!("{temperature_c} °C: {e}\n--- console ---\n{transcript}"));
    }
    assert!(s.uart_transcript().contains("N=7FF D="));
}
