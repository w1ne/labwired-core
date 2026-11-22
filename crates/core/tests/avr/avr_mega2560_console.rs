// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino Mega 2560 console tap: the board's USB bridge carries USART0 only,
//! so `configs/systems/arduino-mega-2560.yaml` names `usart0` as its
//! `debug_uart`. Without that line `labwired test` fell back to attaching one
//! capture buffer to all four USARTs, and a sketch that wrote Serial and
//! Serial1 together produced byte-interleaved text (`SS01_S_B2LO_...`).
//!
//! Firmware: tests/fixtures/avr/arduino-mega-2560-console.cpp (real
//! ArduinoCore-avr `HardwareSerial`, interrupt-driven TX on every port).

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::Machine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Boot the fixture on the shipped board manifest the way `labwired test`
/// does: USART0 TX leaves through the CPU serial sink, and `attach` decides
/// which bus USART windows also feed `sink`. Returns the capture once it
/// holds `until`.
fn run(
    until: &str,
    console_on_cpu: bool,
    attach: impl FnOnce(&mut SystemBus, &SystemManifest, Arc<Mutex<Vec<u8>>>),
) -> String {
    let yaml = root().join("configs/systems/arduino-mega-2560.yaml");
    let manifest = SystemManifest::from_file(&yaml).expect("load system yaml");
    let chip_path = yaml.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let sink = Arc::new(Mutex::new(Vec::new()));
    attach(&mut bus, &manifest, sink.clone());
    let elf = std::fs::read(root().join("tests/fixtures/avr/arduino-mega-2560-console.elf"))
        .expect("missing fixture; build tests/fixtures/avr/arduino-mega-2560-console.cpp");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse ELF");
    let mut cpu = Avr::for_chip_name(&chip.name);
    assert_eq!(cpu.model, AvrModel::Atmega2560);
    cpu.load_program_image(&image);
    if console_on_cpu {
        cpu.set_serial_sink(sink.clone());
    }
    let mut machine = Machine::new(cpu, bus);
    // 4 lines of ~14 bytes at 115200 baud is under 6 ms = 96k cycles; the
    // budget leaves room for the C runtime and the Arduino init.
    for _ in 0..2_000_000u32 {
        machine.step().expect("step");
        if String::from_utf8_lossy(&sink.lock().unwrap()).contains(until) {
            break;
        }
    }
    let text = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    drop(machine);
    text
}

#[test]
fn mega_manifest_names_usart0_as_the_console() {
    let manifest = SystemManifest::from_file(root().join("configs/systems/arduino-mega-2560.yaml"))
        .expect("load system yaml");
    assert_eq!(manifest.debug_uart.as_deref(), Some("usart0"));
}

/// The CLI path: `debug_uart` resolves to one USART, and only Serial lands in
/// the console capture, line by line.
#[test]
fn console_capture_holds_serial_only() {
    let text = run("S0_DONE\r\n", true, |bus, manifest, sink| {
        let name = manifest.debug_uart.as_deref().expect("debug_uart");
        assert!(bus.attach_uart_tx_sink_named(name, sink, false));
    });
    assert_eq!(text, "S0_BOOT\r\nS0_DONE\r\n");
}

/// Serial1 still transmits: its bytes reach its own window, not the console.
#[test]
fn serial1_bytes_reach_usart1() {
    let text = run("S1_LINK_DATA\r\n", false, |bus, _, sink| {
        assert!(bus.attach_uart_tx_sink_named("usart1", sink.clone(), false));
    });
    assert!(
        text.contains("S1_LINK_DATA\r\n"),
        "usart1 capture: {text:?}"
    );
}

/// Control: the all-UART fallback the board used before `debug_uart` existed
/// splices Serial1/Serial2 into the console, so the line never comes out whole.
#[test]
fn all_uart_fallback_interleaves_serial1_into_the_console() {
    let text = run("S0_DONE\r\n", true, |bus, _, sink| {
        bus.attach_uart_tx_sink(sink, false);
    });
    assert!(
        text.contains('1') && text.contains('2'),
        "capture: {text:?}"
    );
    assert!(!text.contains("S0_BOOT\r\n"), "capture: {text:?}");
}
