// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A CONSOLE THE BUS CANNOT PROVIDE MUST NOT BE ANSWERED WITH A DIFFERENT ONE.
//!
//! Which console carries `Serial` to the developer's cable is a board fact: an
//! ESP32-C3 SuperMini's USB-C socket is the chip's own USB-Serial-JTAG, while a
//! classic ESP32 devkit's socket is a CP210x sitting on UART0. The run manifest
//! states it (`debug_uart:`) and [`SystemBus::attach_host_console`] honours it.
//!
//! Every call site used to do this instead:
//!
//! ```ignore
//! if !bus.attach_uart_tx_sink_named(name, sink, false) {
//!     bus.attach_uart_tx_sink(sink, false);   // ... any UART at all
//! }
//! ```
//!
//! so a manifest naming a console the bus does not have quietly got a DIFFERENT
//! console. For a twin that is the worst available answer: the Serial pane fills
//! with plausible text while asserting `Serial` is on pins the board never uses,
//! and the developer only finds out when the real board is silent. These tests
//! pin the refusal, in both the "wrong UART" and "no USB block at all" shapes.
//!
//! Deliberately NOT `#![cfg(feature = "event-scheduler")]`: this must run in
//! every lane, including plain `cargo test -p labwired-core`.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::console::HostConsole;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn configs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs")
}

fn bus_for(chip_file: &str, system_yaml: &str) -> SystemBus {
    let chip: ChipDescriptor = serde_yaml::from_str(
        &std::fs::read_to_string(configs().join("chips").join(chip_file))
            .unwrap_or_else(|e| panic!("read {chip_file}: {e}")),
    )
    .expect("parse chip yaml");
    let manifest: SystemManifest = serde_yaml::from_str(system_yaml).expect("parse system yaml");
    SystemBus::from_config(&chip, &manifest).expect("build bus")
}

fn sink() -> Arc<Mutex<Vec<u8>>> {
    Arc::new(Mutex::new(Vec::new()))
}

const STM32_SYSTEM: &str = "name: \"t\"\nchip: \"../chips/stm32l476.yaml\"\n";
const C3_SYSTEM: &str = "name: \"t\"\nchip: \"../chips/esp32c3.yaml\"\n";

/// A Cortex-M board has no USB-Serial-JTAG block. Declaring one is a board
/// mapping error, and it must be reported as one.
#[test]
fn a_chip_without_a_usb_serial_jtag_block_refuses_that_console() {
    let mut bus = bus_for("stm32l476.yaml", STM32_SYSTEM);

    let s = sink();
    let err = bus
        .attach_host_console(&HostConsole::UsbSerialJtag, s.clone())
        .expect_err("an STM32L476 has no USB-Serial-JTAG block");

    assert!(
        err.contains("usb_serial_jtag"),
        "refusal must name the console it could not provide: {err}"
    );
    // Decisive: the sink was not handed to some other console on the way out.
    // The old fallback stored it in every UART on the bus, so the pane filled
    // with UART text while the manifest said USB.
    assert_eq!(
        Arc::strong_count(&s),
        1,
        "a refused console must leave the sink attached to NOTHING"
    );
}

/// A UART the bus does not carry is refused rather than swapped for another.
#[test]
fn a_uart_this_bus_does_not_have_is_refused() {
    let mut bus = bus_for("stm32l476.yaml", STM32_SYSTEM);

    let s = sink();
    let err = bus
        .attach_host_console(&HostConsole::Uart("uart9".into()), s.clone())
        .expect_err("there is no uart9 on an STM32L476");

    assert!(
        err.contains("uart9"),
        "refusal must name the console: {err}"
    );
    assert_eq!(
        Arc::strong_count(&s),
        1,
        "a refused console must leave the sink attached to NOTHING"
    );
}

/// The console a board really has still attaches — the refusal is narrow.
#[test]
fn a_uart_this_bus_does_have_still_attaches() {
    let mut bus = bus_for("stm32l476.yaml", STM32_SYSTEM);

    bus.attach_host_console(&HostConsole::Uart("uart2".into()), sink())
        .expect("uart2 exists on an STM32L476");
}

/// An undeclared manifest keeps the historical default (every console-capable
/// UART on the bus), so no shipped lab changes behaviour.
#[test]
fn an_undeclared_console_attaches_the_uarts_as_before() {
    let mut bus = bus_for("stm32l476.yaml", STM32_SYSTEM);

    bus.attach_host_console(&HostConsole::Undeclared, sink())
        .expect("an undeclared console is not an error");
}

/// The C3 chip descriptor declares a REGISTER STUB at 0x6004_3000, not the
/// behavioral USB-Serial-JTAG model — it answers reads and never drains a byte.
/// Counting that as "console attached" would be the same silent lie: the pane
/// would stay empty with the twin reporting success. Only the real model counts,
/// and it is installed by the ROM-boot builder on the paths that run firmware.
#[test]
fn a_declarative_register_stub_does_not_count_as_the_usb_console() {
    let mut bus = bus_for("esp32c3.yaml", C3_SYSTEM);

    assert!(
        !bus.attach_usb_serial_jtag_sink(sink()),
        "a register stub at 0x60043000 is not a console"
    );
}

/// UART0 is where every C3 lab shipped so far prints, and it must keep working.
#[test]
fn the_c3_keeps_its_uart0_console() {
    let mut bus = bus_for("esp32c3.yaml", C3_SYSTEM);

    bus.attach_host_console(&HostConsole::Uart("uart0".into()), sink())
        .expect("uart0 exists on an ESP32-C3");
}
