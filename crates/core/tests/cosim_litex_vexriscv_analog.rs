// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the LiteX GPIOIn `in`
//! CSR of the Zephyr enjoydigital litex_vexriscv board (litex_vexriscv.dts
//! `gpio_in: gpio@e0006000`, the declarative `peripherals/litex/gpio_in.yaml`
//! model that the onboarding LiteX profiles already use):
//! - on the bus: the solved voltages against an independent divider and
//!   Shockley load-line reference, and the `in` bits through the descriptor's
//!   assumed Schmitt thresholds, including the retained level at 25 °C;
//! - the old simulation-only `cosim_inputs` bank is gone and no longer binds;
//! - in a `Session`: the committed RV32IM firmware
//!   (examples/litex-vexriscv/firmware/litex-vexriscv-gpio-in.elf) reads the
//!   CSR and prints `IN=<p3..p0>` on the LiteX UART while the session solves
//!   the netlists.
//!
//! The SoC has no ADC: this is a digital-input route, not a conversion.

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

/// LiteX GPIOIn `in` CSR (litex_vexriscv.dts `gpio_in: gpio@e0006000`).
const GPIO_IN: u64 = 0xE000_6000;
const FIRMWARE: &str = "examples/litex-vexriscv/firmware/litex-vexriscv-gpio-in.elf";
const ANALOG_SYSTEM: &str = "examples/litex-vexriscv/analog-ntc-diode-system.yaml";

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

fn voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn board(ntc_path: &str, diode_path: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/litex-vexriscv.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load LiteX profile");
    assert_eq!(profile.name, "litex-vexriscv");
    let analog = SystemManifest::from_file(root(ANALOG_SYSTEM))
        .expect("load generic NTC divider and Shockley diode");
    profile.cosim_models = analog.cosim_models;
    for model in &mut profile.cosim_models {
        for path in model.outputs.values_mut() {
            *path = match path.as_str() {
                "board.gpio_in.gpio_in.0" => ntc_path.to_string(),
                "board.gpio_in.gpio_in.1" => diode_path.to_string(),
                other => other.to_string(),
            };
        }
    }

    // Absolute chip path: the declarative GPIO descriptors resolve against it.
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    profile.chip = chip_path.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(&chip_path).expect("load LiteX chip descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build LiteX system bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog models");
    (bus, session)
}

#[test]
fn litex_vexriscv_gpio_in_csr_reads_the_solved_ntc_and_diode_through_thresholds() {
    let (mut bus, mut session) = board("board.gpio_in.gpio_in.0", "board.gpio_in.gpio_in.1");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "both unrelated device models must bind to GPIOIn pins 0 and 1"
    );
    assert!(
        bus.find_peripheral_index_by_name("cosim_inputs").is_none(),
        "the simulation-only input bank was removed"
    );
    assert_eq!(bus.read_u32(GPIO_IN).expect("read GPIOIn in"), 0);

    let expected_diode_v = diode_load_line_voltage();
    assert!(expected_diode_v < 0.99, "diode must fall below assumed VIL");
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // retain the cold high level inside the assumed band
        (80.0, false),
        (25.0, false), // retain the hot low level inside the assumed band
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both generic analog netlists");
        assert_eq!(steps.len(), 2, "both device models cross the boundary");
        assert!(errors.is_empty(), "unexpected route errors: {errors:?}");

        let ntc_v = voltage(&session, "board.gpio_in.gpio_in.0");
        let resistance = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * resistance / (10_000.0 + resistance);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "at {temperature_c} °C the NTC divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = voltage(&session, "board.gpio_in.gpio_in.1");
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "the Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        assert_eq!(
            bus.read_u32(GPIO_IN).expect("read GPIOIn in"),
            expected_ntc_level as u32,
            "at {temperature_c} °C: `in` bit 0 follows the NTC through the thresholds \
             and hysteresis, bit 1 reads the diode low"
        );
    }
}

/// Pin 3 is a real GPIOIn pin (ngpios 4); the old bank path no longer binds.
#[test]
fn litex_vexriscv_ntc_on_gpio_in_pin_3_sets_in_bit_3_and_the_old_bank_is_gone() {
    let (mut bus, mut session) = board("board.gpio_in.gpio_in.3", "board.gpio_in.gpio_in.1");
    assert_eq!(session.binding_errors(), &[] as &[RoutingError]);
    session
        .set_signal_number("ui.ntc.temperature_c", -10.0)
        .unwrap();
    let cycles = session.cycles_until_boundary(0);
    let (_, errors) = session.advance_to(cycles, &mut bus).unwrap();
    assert!(errors.is_empty(), "unexpected route errors: {errors:?}");
    assert_eq!(bus.read_u32(GPIO_IN).unwrap(), 0b1000);

    let (_, session) = board(
        "board.gpio_in.cosim_inputs.0",
        "board.gpio_in.cosim_inputs.1",
    );
    assert_eq!(
        session.binding_errors().len(),
        2,
        "the removed input-bank endpoints must not bind: {:?}",
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

/// Nothing is routed on the plain board: every GPIOIn pin reads low.
#[test]
fn litex_vexriscv_gpio_in_firmware_reads_the_csr_on_the_plain_board() {
    let mut s = open_firmware("examples/litex-vexriscv/system.yaml");
    let r = s.expect("LITEX GPIO IN\nIN=0000\n", Duration::from_secs(1));
    let transcript = s.uart_transcript();
    r.unwrap_or_else(|e| panic!("{e}\n--- console ---\n{transcript}"));
}

/// The whole loop in one `Session`: the session solves the example's NTC
/// divider and diode netlists, routes them to GPIOIn pins 0 and 1, and the
/// firmware reads the `in` CSR and prints it. The NTC moves between -10 °C
/// and 80 °C while the firmware runs, and the printed bit 0 follows.
#[test]
fn litex_vexriscv_gpio_in_firmware_reads_the_solved_netlists_in_a_session() {
    let mut s = open_firmware(ANALOG_SYSTEM);
    for (temperature_c, line) in [(-10.0, "IN=0001"), (80.0, "IN=0000"), (-10.0, "IN=0001")] {
        s.set_signal("ui.ntc.temperature_c", temperature_c).unwrap();
        let r = s.expect(line, Duration::from_secs(1));
        let transcript = s.uart_transcript();
        r.unwrap_or_else(|e| panic!("{temperature_c} °C: {e}\n--- console ---\n{transcript}"));
    }
    assert!(
        !s.uart_transcript().contains("IN=0010"),
        "the diode never reads high"
    );
}
