// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// End-to-end test: build esp-hal hello-world, run it in the simulator,
// confirm "Hello world!" is captured from the USB_SERIAL_JTAG sink.
//
// Gated on `--features esp32s3-fixtures` so plain `cargo test` (without
// the ESP toolchain) still works.

#![cfg(feature = "esp32s3-fixtures")]

use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::{Cpu, SimulationError};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Path to the firmware ELF, relative to the workspace root.
fn firmware_path() -> PathBuf {
    PathBuf::from("../../examples/esp32s3-hello-world/target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world")
}

/// Build the firmware crate via `cargo +esp build --release`.
/// Skips the build if the ELF already exists and is newer than `src/main.rs`.
fn ensure_firmware_built() -> PathBuf {
    let elf = firmware_path();
    let src = PathBuf::from("../../examples/esp32s3-hello-world/src/main.rs");
    if elf.exists() {
        if let (Ok(elf_meta), Ok(src_meta)) = (std::fs::metadata(&elf), std::fs::metadata(&src)) {
            if elf_meta.modified().unwrap() >= src_meta.modified().unwrap() {
                return elf;
            }
        }
    }
    let status = Command::new("cargo")
        .args(["+esp", "build", "--release"])
        .current_dir("../../examples/esp32s3-hello-world")
        .status()
        .expect("cargo +esp build (is the ESP toolchain installed?)");
    assert!(status.success(), "esp32s3-hello-world build failed");
    assert!(elf.exists(), "ELF not found at {:?} after build", elf);
    elf
}

#[test]
fn hello_world_prints_at_least_twice() {
    let elf_path = ensure_firmware_built();
    let elf_bytes = std::fs::read(&elf_path).expect("read firmware ELF");

    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    let mut cpu = wiring.cpu;

    // Replace the default UsbSerialJtag with one that captures into a buffer.
    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    if let Some(p) = bus
        .peripherals
        .iter_mut()
        .find(|p| p.name == "usb_serial_jtag")
    {
        if let Some(any_mut) = p.dev.as_any_mut() {
            if let Some(jtag) = any_mut.downcast_mut::<UsbSerialJtag>() {
                jtag.set_sink(Some(sink.clone()), false);
            }
        }
    }

    fast_boot(
        &elf_bytes,
        &mut bus,
        &mut cpu,
        &BootOpts {
            stack_top_fallback: 0x3FCD_FFF0,
            icache_backing: Some(wiring.icache_backing),
            dcache_backing: Some(wiring.dcache_backing),
        },
    )
    .expect("fast_boot");

    // Run for up to 500 M simulated cycles. Plan-2 verified this fits 2+
    // "Hello world!" lines paced by SYSTIMER through `Delay::delay_millis`.
    const MAX_STEPS: u64 = 500_000_000;
    let observers: Vec<std::sync::Arc<dyn labwired_core::SimulationObserver>> = Vec::new();
    for _ in 0..MAX_STEPS {
        match cpu.step(&mut bus, &observers) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(_)) => break,
            Err(e) => panic!("simulator error at pc=0x{:08x}: {e}", cpu.get_pc()),
        }
        // Drain peripheral interrupts so SYSTIMER ticks (just like the CLI does).
        let _ = bus.tick_peripherals_with_costs();

        // Early exit once we have two Hello-world lines.
        let captured = sink.lock().unwrap();
        let s = String::from_utf8_lossy(&captured);
        if s.matches("Hello world!").count() >= 2 {
            return;
        }
    }
    let captured = sink.lock().unwrap();
    let s = String::from_utf8_lossy(&captured);
    panic!(
        "did not see 2+ 'Hello world!' lines in {MAX_STEPS} steps; captured: {:?}",
        s
    );
}
