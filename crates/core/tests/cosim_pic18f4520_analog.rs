// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic analog netlists reach the PIC18F4520 10-bit A/D module, and real
//! gpasm firmware converts them.
//!
//! The A/D module is CPU-internal on the PIC18 core, so the
//! `board.analog.<pad>_volts` routes land on the chip's `pic24_adc_pads`
//! window through the descriptor's `analog_pins`, and the core pulls the
//! levels when they change. The firmware is
//! `examples/pic18f4520/firmware/adc-ntc-diode.asm`: it converts AN0 (RA0)
//! and AN1 (RA1) once each, stores ADRESH:ADRESL and prints both results.
//!
//! Expected codes follow the ideal transfer function `Vin · 1024 / VDD`,
//! truncated; the route carries whole millivolts.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::cpu::Pic18;
use labwired_core::system::node::parse_elf_image;
use labwired_core::{Cpu, SimulationConfig};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// `adc-ntc-diode.asm` RAM: AN0 high/low, AN1 high/low, completion marker.
const NTC_RESULT: u16 = 0x000;
const DIODE_RESULT: u16 = 0x002;
const MARKER: u16 = 0x004;
const VDD_V: f64 = 5.0;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line for the declared 5 V source and diode.
fn diode_load_line_voltage() -> f64 {
    const LIMIT_OHMS: f64 = 1_000.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
    let current_at_junction_voltage =
        |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let current = current_at_junction_voltage(junction_v);
        (VDD_V - junction_v - current * DIODE_RS_OHMS) / LIMIT_OHMS - current
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

/// The 10-bit code for `volts` against VDD, through the route's millivolts.
fn ideal_code(volts: f64) -> u16 {
    let mv = (volts * 1000.0).round() as u32;
    (mv * 1024 / (VDD_V * 1000.0) as u32).min(1023) as u16
}

/// The demo board's bus with the example's NTC divider and diode attached.
fn board_bus() -> (SystemBus, SystemManifest, ChipDescriptor) {
    let system_path = root("configs/systems/pic18f4520-demo.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load PIC18F4520 profile");
    assert_eq!(profile.name, "pic18f4520-demo");
    let analog =
        SystemManifest::from_file(root("examples/pic18f4520/analog-ntc-diode-system.yaml"))
            .expect("load generic NTC divider and Shockley diode models");
    profile.cosim_models = analog.cosim_models;
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load PIC18F4520 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build PIC18F4520 bus");
    (bus, profile, chip)
}

struct Conversions {
    ntc: u16,
    diode: u16,
    console: String,
}

/// Runs the gpasm A/D firmware from reset on `bus`. `bind` is what the
/// session builder does for a chip that declares the pad window. Returns
/// `None` when the firmware never stores its completion marker.
fn run_adc_firmware(chip: &ChipDescriptor, bus: &mut SystemBus, bind: bool) -> Option<Conversions> {
    let elf = std::fs::read(root(
        "examples/pic18f4520/firmware/pic18f4520-gpasm-adc-ntc-diode.elf",
    ))
    .expect("read the gpasm A/D ELF");
    let image = parse_elf_image(&elf).expect("parse ELF");
    let sink: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut cpu = Pic18::for_chip_name(&chip.name);
    if bind {
        cpu.bind_adc_pads(bus);
    }
    cpu.load_program_image(&image);
    cpu.set_serial_sink(sink.clone());
    cpu.reset(bus).expect("reset");
    // The whole program is under 1000 instruction cycles.
    for _ in 0..20_000 {
        cpu.step(bus, &[], &SimulationConfig::default())
            .expect("gpasm firmware instruction must decode and execute");
    }
    if cpu.read_data(MARKER) != 0xA5 {
        return None;
    }
    let pair =
        |addr: u16| (u16::from(cpu.read_data(addr)) << 8) | u16::from(cpu.read_data(addr + 1));
    let console = String::from_utf8(sink.lock().unwrap().clone()).expect("ASCII console");
    Some(Conversions {
        ntc: pair(NTC_RESULT),
        diode: pair(DIODE_RESULT),
        console,
    })
}

#[test]
fn pic18f4520_gpasm_firmware_converts_ntc_and_diode_on_an0_an1() {
    let (mut bus, profile, chip) = board_bus();
    assert!(
        chip.peripherals
            .iter()
            .all(|p| p.r#type != "input_pin_bank"),
        "the descriptor routes to the real A/D inputs, not to an invented bank"
    );
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both generic analog models");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "RA0 is AN0 and RA1 is AN1 in the descriptor's analog_pins"
    );

    let expected_diode_v = diode_load_line_voltage();
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both generic analog netlists");
        assert_eq!(steps.len(), 2, "both unrelated device models solve");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = voltage(&session, "board.analog.RA0_volts");
        let r = ntc_resistance_ohms(temperature_c);
        assert!(
            (ntc_v - VDD_V * r / (10_000.0 + r)).abs() < 1e-9,
            "NTC divider at {temperature_c} °C, got {ntc_v} V"
        );
        let diode_v = voltage(&session, "board.analog.RA1_volts");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let result = run_adc_firmware(&chip, &mut bus, true)
            .expect("the firmware converts both channels and stores its marker");
        assert_eq!(
            result.ntc,
            ideal_code(ntc_v),
            "ADRESH:ADRESL for AN0 at {temperature_c} °C ({ntc_v} V on RA0)"
        );
        assert_eq!(
            result.diode,
            ideal_code(diode_v),
            "ADRESH:ADRESL for AN1 ({diode_v} V on RA1)"
        );
        assert_eq!(
            result.console,
            format!("N={:03X} D={:03X}\r\n", result.ntc, result.diode),
            "the firmware prints what it converted"
        );
        ntc_codes.push(result.ntc);
        // 0.649 V of 5 V: the diode channel does not follow the NTC.
        assert_eq!(result.diode, 132);
    }
    // -10 °C = 4.267 V, 25 °C = 2.500 V, 80 °C = 0.564 V: the codes must track.
    assert_eq!(ntc_codes, [873, 512, 115]);
}

/// Negative controls. Without the pad window bound, ADCON0 is a plain
/// register and GO/DONE never clears, so the firmware cannot finish. With it
/// bound but nothing routed, both inputs convert 0 V.
#[test]
fn pic18f4520_adc_needs_the_pad_window_and_a_route() {
    let (mut bus, _, chip) = board_bus();
    assert!(
        run_adc_firmware(&chip, &mut bus, false).is_none(),
        "no A/D module without the pad window"
    );
    let result = run_adc_firmware(&chip, &mut bus, true).expect("the A/D module converts");
    assert_eq!((result.ntc, result.diode), (0, 0));
    assert_eq!(result.console, "N=000 D=000\r\n");
}
