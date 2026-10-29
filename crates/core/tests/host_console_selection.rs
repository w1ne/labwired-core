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

/// The ESP32-S3 bus every firmware path runs on: the one
/// `configure_xtensa_esp32s3` builds for the wasm flash/ROM boot (the Doom lab)
/// and the native `--rom-boot` runner. It names its UARTs `uart0_s3`..; the
/// manifest says `uart0`.
fn s3_firmware_bus() -> SystemBus {
    let mut bus = SystemBus::new();
    let _ = labwired_core::system::xtensa::configure_xtensa_esp32s3(
        &mut bus,
        &labwired_core::system::xtensa::Esp32s3Opts::default(),
    );
    bus
}

/// `debug_uart: uart0` on an ESP-IDF S3 image (its log is on UART0, the
/// DevKitC's CP2102) must reach UART0 on the bus the firmware actually runs on.
/// Before the alias, this refused with "this bus has no such UART" and the
/// bundled `esp32s3-doom-lab` went to ERROR the moment Run was pressed.
#[test]
fn the_s3_firmware_bus_accepts_the_manifest_name_uart0() {
    let mut bus = s3_firmware_bus();
    assert!(
        bus.find_peripheral_index_by_name("uart0").is_none()
            && bus.find_peripheral_index_by_name("uart0_s3").is_some(),
        "precondition: this bus spells UART0 `uart0_s3` and has no plain `uart0`"
    );

    let s = sink();
    bus.attach_host_console(&HostConsole::Uart("uart0".into()), s.clone())
        .expect("UART0 exists on an ESP32-S3; `uart0` must resolve to `uart0_s3`");
    assert_eq!(
        Arc::strong_count(&s),
        2,
        "the sink must be held by exactly one console: UART0"
    );
}

/// The alias is narrow: the programmatic name still works, and a UART the S3
/// does not have is still refused, not swapped for another one.
#[test]
fn the_s3_firmware_bus_still_refuses_a_uart_it_does_not_have() {
    let mut bus = s3_firmware_bus();
    bus.attach_host_console(&HostConsole::Uart("uart0_s3".into()), sink())
        .expect("the programmatic name keeps working");

    let s = sink();
    let err = bus
        .attach_host_console(&HostConsole::Uart("uart9".into()), s.clone())
        .expect_err("there is no uart9 on an ESP32-S3");
    assert!(
        err.contains("uart9"),
        "refusal must name the console: {err}"
    );
    assert_eq!(Arc::strong_count(&s), 1, "a refused console holds no sink");
}

const C6_SYSTEM: &str = "name: \"t\"\nchip: \"../chips/esp32c6.yaml\"\n";

/// The C6 chip yaml registers the behavioural `UsbSerialJtag` (not a declarative
/// stub) at `0x6000_F000`, so the shared console tap can hear CDC-ACM bytes.
/// This is the contract a CDC-on-boot twin needs before the Arduino boot path
/// exists: the window must not fault, and `attach_usb_serial_jtag_sink` must
/// succeed.
#[test]
fn esp32c6_usb_serial_jtag_is_the_behavioural_console() {
    use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
    use labwired_core::Bus;

    let mut bus = bus_for("esp32c6.yaml", C6_SYSTEM);

    assert!(
        bus.attach_usb_serial_jtag_sink(sink()),
        "C6 chip yaml must register the behavioural UsbSerialJtag as usb_serial_jtag"
    );

    // Window is live: EP1_CONF SERIAL_IN_EP_DATA_FREE reads 1 out of reset.
    let conf = bus
        .read_u32(0x6000_F000 + 0x04)
        .expect("USB_DEVICE EP1_CONF must not fault");
    assert_ne!(conf & (1 << 1), 0, "SERIAL_IN_EP_DATA_FREE reset level");

    // Matrix source is the C6's, not a silent C3/S3 default.
    let idx = bus
        .find_peripheral_index_by_name("usb_serial_jtag")
        .expect("usb_serial_jtag present");
    let any = bus.peripherals[idx]
        .dev
        .as_any_mut()
        .expect("downcastable");
    let jtag = any
        .downcast_mut::<UsbSerialJtag>()
        .expect("behavioural model");
    assert_eq!(jtag.irq_source(), Some(48));
}

/// Declaring `debug_uart: usb_serial_jtag` on a C6 bus must attach, not refuse.
#[test]
fn esp32c6_host_console_accepts_usb_serial_jtag() {
    let mut bus = bus_for("esp32c6.yaml", C6_SYSTEM);
    bus.attach_host_console(&HostConsole::UsbSerialJtag, sink())
        .expect("C6 chip yaml carries usb_serial_jtag");
}
