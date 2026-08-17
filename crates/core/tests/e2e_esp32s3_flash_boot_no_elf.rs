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
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts};
use labwired_core::Machine;
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

    let mut machine = Machine::new(cpu, bus);

    // The banner arrives early; the fixture's own app output follows. 20 M
    // steps is what the native `--rom-boot` CLI needs for both on this image.
    for _ in 0..20_000_000u64 {
        let _ = machine.step();
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
}
