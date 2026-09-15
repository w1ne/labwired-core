// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// The ESP32-S3 boots from a flash image with NO ELF.
//
// This is the assembly every HOSTED S3 run needs and the one that did not
// exist: the hosted compiler ships `bootloader@0x0 + partition-table@0x8000 +
// app@0x10000` and no ELF, while the only S3 constructor took an ELF and
// `fast_boot`. A run with no ELF to load looked exactly like a hang — the mask
// ROM printed
//
//     ESP-ROM:esp32s3-20210327 … entry 0x403c98d0
//
// and then nothing, at 20 M and at 200 M steps, with the fault PC in ROM space.
// The C3 has had the ELF-less equivalent since its own rom-boot path landed.
//
// What this locks down is the assembly, not the wasm wrapper (which needs a JS
// context): flash bytes in through `Esp32s3Opts::flash_image`, `real_reset_boot`
// for the MMU XIP model, the real ROM, and the CPU left at the BROM reset
// vector. If any one of those is dropped the boot dies before the app: identity
// XIP in particular reads the wrong dcache page and returns zeros, which is a
// silent wrong answer rather than an error.
#![cfg(feature = "esp32s3-fixtures")]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::xtensa_lx7::XtensaLx7;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts};
use labwired_core::{AdvanceRequest, BreakpointPolicy, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn flash_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tier1/esp32s3-flash.bin")
}

#[test]
fn flash_image_boots_the_app_without_an_elf() {
    let Ok(flash) = std::fs::read(flash_path()) else {
        eprintln!("skipping: tier1 esp32s3-flash.bin fixture not present");
        return;
    };

    let mut bus = SystemBus::new();
    let opts = Esp32s3Opts {
        real_reset_boot: true,
        flash_image: Some(flash.clone()),
        flash_size: (flash.len() as u32).max(4 * 1024 * 1024),
        ..Esp32s3Opts::default()
    };
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
    assert_eq!(
        wiring.boot_mode,
        Esp32s3BootMode::Faithful,
        "this path IS the ROM; without a real one there is nothing to boot"
    );
    let cpu = wiring.cpu;

    // Both consoles: the mask ROM and the 2nd-stage bootloader talk on UART0,
    // an Arduino sketch built CDC-on-boot talks on USB-Serial-JTAG. A tap on
    // one of them is how a run that IS working reads as silent.
    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    bus.attach_usb_serial_jtag_sink(sink.clone());
    bus.refresh_peripheral_index();

    // The ESP32-S3 is a DUAL-core chip and this test has to boot it as one.
    // ESP-IDF's `start_other_core` spins `while (!s_cpu_up[1])
    // esp_rom_delay_us(100)` before app_main, so a single-core S3 never leaves
    // the mask ROM's `ets_delay_us` — the console shows the banner and stops.
    let mut app_cpu = XtensaLx7::new_app_cpu();
    app_cpu.faithful_windows = true;
    let mut machine = Machine::new(cpu, bus).with_secondary_cpu(app_cpu);

    // Drive it through `advance` with a WIDE batch, not `step`, because that is
    // what the hosted CLI (`execute_test_loop`) and the browser both issue and
    // it is the only path that plans multi-instruction windows. Stepping one
    // instruction at a time hides every batch-planning defect: this test passed
    // as a `step` loop while every real S3 run hung.
    let mut retired = 0u64;
    while retired < 20_000_000 {
        let request = AdvanceRequest::run(Some(20_000_000 - retired))
            .with_batch_cap(std::num::NonZeroU32::new(10_000).expect("non-zero"))
            .with_breakpoints(BreakpointPolicy::Ignore);
        let Ok(report) = machine.advance(request) else {
            break;
        };
        if report.primary_steps == 0 {
            break;
        }
        retired += report.primary_steps;
    }

    let out = String::from_utf8_lossy(&sink.lock().unwrap().clone()).to_string();
    assert!(
        out.contains("ESP-ROM:esp32s3"),
        "no mask-ROM banner — the ROM never ran at all. Got: {out:?}"
    );
    // NOT an assertion on "boot:" — the mask ROM's own reset line
    // (`rst:0xc (RTC_SW_CPU_RST),boot:0x8 (SPI_FAST_FLASH_BOOT)`) already
    // contains it, so such a check passes on a run that loads nothing. The
    // distinguishing evidence is below: only a 2nd-stage bootloader served real
    // bytes by the flash controller and the MMU prints it.
    assert!(
        out.contains("Loaded app from partition"),
        "bootloader ran but never loaded the app image. Got: {out:?}"
    );
    // And the APPLICATION actually ran. "Loaded app from partition" is the
    // bootloader's own claim about what it copied; it is printed before a
    // single application instruction retires, so it passes on a run that loads
    // the image and then hangs — which is exactly what every hosted S3 run did.
    // The fixture's last line is the one that needs both cores alive and the
    // scheduler intact.
    assert!(
        out.contains("TIER1 done"),
        "the app never ran to completion — the image loaded but the firmware \
         did not finish. Got: {out:?}"
    );
}

/// The ARDUINO shape of the same boot — and the one that gates batch planning.
///
/// The tier1 fixture above is a bare ESP-IDF app; it survives a wide primary
/// window, so it cannot catch a scheduler-timing defect. Every hosted S3 run is
/// this shape instead: Arduino core, `ARDUINO_USB_CDC_ON_BOOT=1` (so `Serial`
/// is USB-Serial-JTAG, not UART0), FreeRTOS with both cores live, and the
/// sketch's own output as the only evidence the run worked.
///
/// That combination did not survive the coalesced dual-core idle window. With a
/// flat 1024-instruction clamp the boot reaches app IRAM and then wedges:
/// `vListInsert` walks onto a node whose `pxNext` is itself and spins at
/// 0x4037cdb6 forever, while core 1 sits at 0x4037cf41 on the spinlock PRO_CPU
/// can no longer release. The console shows the mask-ROM banner and nothing
/// else — indistinguishable from a hung sketch.
///
/// Restoring that flat clamp in `plan.rs` must fail this test.
#[test]
fn arduino_flash_image_runs_the_sketch_under_a_wide_batch() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/tier1/esp32s3-arduino-flash.bin");
    let Ok(mut flash) = std::fs::read(&path) else {
        eprintln!("skipping: esp32s3-arduino-flash.bin fixture not present");
        return;
    };
    // Stored trimmed to its used extent; the part is a 4 MB device and the
    // bootloader reads the partition table by absolute offset, so pad it back.
    flash.resize(4 * 1024 * 1024, 0xFF);

    let mut bus = SystemBus::new();
    let opts = Esp32s3Opts {
        real_reset_boot: true,
        flash_image: Some(flash.clone()),
        flash_size: flash.len() as u32,
        ..Esp32s3Opts::default()
    };
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
    assert_eq!(wiring.boot_mode, Esp32s3BootMode::Faithful);
    let mut cpu = wiring.cpu;
    cpu.faithful_windows = true;

    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    bus.attach_usb_serial_jtag_sink(sink.clone());
    bus.refresh_peripheral_index();

    let mut app_cpu = XtensaLx7::new_app_cpu();
    app_cpu.faithful_windows = true;
    let mut machine = Machine::new(cpu, bus).with_secondary_cpu(app_cpu);

    // The 10_000 batch cap is what `execute_test_loop` issues for a
    // `uart_contains` assertion — i.e. the hosted run, not a narrowed one.
    let mut retired = 0u64;
    while retired < 20_000_000 {
        let request = AdvanceRequest::run(Some(20_000_000 - retired))
            .with_batch_cap(std::num::NonZeroU32::new(10_000).expect("non-zero"))
            .with_breakpoints(BreakpointPolicy::Ignore);
        let Ok(report) = machine.advance(request) else {
            break;
        };
        if report.primary_steps == 0 {
            break;
        }
        retired += report.primary_steps;
    }

    let out = String::from_utf8_lossy(&sink.lock().unwrap().clone()).to_string();
    assert!(
        out.contains("ESP-ROM:esp32s3"),
        "no mask-ROM banner — the ROM never ran at all. Got: {out:?}"
    );
    assert!(
        out.contains("SMOKE_OK"),
        "the sketch never printed. The image booted but setup() output never \
         reached the console. Got: {out:?}"
    );
}
