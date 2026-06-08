// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// End-to-end test: build the `epaper-tricolor-lab` example, run it in the
// simulator, and verify the SSD1680 panel digital twin received the expected
// three-band pattern (white / black / red, 99/99/98 rows each).
//
// This is the fidelity gate that justifies the side-by-side demo: the same
// firmware ELF that flashes to a real NUCLEO-F103RB + Waveshare panel must
// produce identical plane bytes in the simulator.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::Ssd1680Tricolor290;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;
use std::process::Command;

fn firmware_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/thumbv7m-none-eabi/release/epaper-tricolor-lab")
}

fn ensure_firmware_built() -> PathBuf {
    let elf = firmware_path();
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/epaper-tricolor-lab/src/main.rs");
    if elf.exists() {
        if let (Ok(elf_meta), Ok(src_meta)) = (std::fs::metadata(&elf), std::fs::metadata(&src)) {
            if elf_meta.modified().unwrap() >= src_meta.modified().unwrap() {
                return elf;
            }
        }
    }
    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            "epaper-tricolor-lab",
            "--target",
            "thumbv7m-none-eabi",
            "--release",
        ])
        // Under `cargo llvm-cov` the parent process exports
        // RUSTFLAGS=-C instrument-coverage, which injects a profiler_builtins
        // dependency that has no bare-metal target → the no_std firmware build
        // fails with E0463. Strip the coverage flags from this child build.
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .status()
        .expect("cargo build epaper-tricolor-lab");
    assert!(status.success(), "epaper-tricolor-lab build failed");
    assert!(elf.exists(), "ELF not found at {elf:?} after build");
    elf
}

#[test]
fn firmware_drives_panel_to_three_band_pattern() {
    let elf_path = ensure_firmware_built();

    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/epaper-tricolor-lab/system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load system manifest");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    let image = labwired_loader::load_elf(&elf_path).expect("parse ELF");
    machine.load_firmware(&image).expect("load firmware");

    // 50M instructions is comfortably above what the firmware needs (~25M for
    // the full init + 2 planes + refresh sequence) but bounded so a hang in
    // the firmware fails the test instead of running forever.
    const MAX_STEPS: u64 = 50_000_000;
    let mut last_pc = 0u32;
    let mut wfi_streak = 0u32;
    for _ in 0..MAX_STEPS {
        machine.step().expect("step");
        let pc = machine.cpu.get_pc();
        // Early-exit when the firmware sits in the wfi loop for a while.
        if pc == last_pc {
            wfi_streak += 1;
            if wfi_streak > 1000 {
                break;
            }
        } else {
            wfi_streak = 0;
            last_pc = pc;
        }
    }

    // Reach into the bus, find SPI1, find the attached SSD1680, snapshot it.
    let spi_idx = machine
        .bus
        .find_peripheral_index_by_name("spi1")
        .expect("spi1 not registered");
    let any = machine.bus.peripherals[spi_idx]
        .dev
        .as_any()
        .expect("spi peripheral supports downcast");
    let spi = any
        .downcast_ref::<labwired_core::peripherals::spi::Spi>()
        .expect("spi1 is the Spi peripheral");
    let panel = spi
        .attached_devices
        .iter()
        .find_map(|d| {
            d.as_any()
                .and_then(|a| a.downcast_ref::<Ssd1680Tricolor290>())
        })
        .expect("SSD1680 panel attached to spi1");

    assert!(
        panel.refresh_generation() >= 1,
        "0x20 master activation must have fired at least once; got generation={}",
        panel.refresh_generation()
    );

    let black = panel.black_plane();
    let red = panel.red_plane();
    assert_eq!(black.len(), 4736, "black plane size");
    assert_eq!(red.len(), 4736, "red plane size");

    // 128 px wide × 296 px tall, 16 bytes per row. Pattern:
    //   rows  0..=98  WHITE  → black=0xFF, red=0xFF
    //   rows 99..=197 BLACK  → black=0x00, red=0xFF
    //   rows 198..=295 RED   → black=0xFF, red=0x00
    const WIDTH_BYTES: usize = 16;
    let band_byte = |row: usize| -> (u8, u8) {
        match row {
            0..=98 => (0xFF, 0xFF),
            99..=197 => (0x00, 0xFF),
            _ => (0xFF, 0x00),
        }
    };
    for row in 0..296usize {
        let (exp_b, exp_r) = band_byte(row);
        for col_byte in 0..WIDTH_BYTES {
            let idx = row * WIDTH_BYTES + col_byte;
            assert_eq!(
                black[idx], exp_b,
                "black plane[row={row} col_byte={col_byte}] expected {exp_b:#04x}, got {:#04x}",
                black[idx]
            );
            assert_eq!(
                red[idx], exp_r,
                "red plane[row={row} col_byte={col_byte}] expected {exp_r:#04x}, got {:#04x}",
                red[idx]
            );
        }
    }
}
