// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// End-to-end test: load the `esp32-epaper-lab` ELF, run it in the
// simulator wired with our ESP32-classic GPIO + VSPI peripherals, and
// verify the SSD1680 panel received the exact e-reader bitmap the firmware
// streams (`EREADER_BLACK_PLANE` / `EREADER_RED_PLANE`).
//
// This is the side-by-side fidelity check that justifies "same ELF on
// chip and in sim" for the Arduino-ESP32-style ESP32 + tri-color e-paper
// hardware target: the panel content must match the firmware's embedded
// planes byte-for-byte. (The firmware switched from a synthetic three-band
// pattern to the rendered e-reader page in 87d8d45; this test was updated
// to match — the gitignored ELF had let the stale golden go unnoticed.)

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::Ssd1680Tricolor290;
use labwired_core::peripherals::esp32::spi::Esp32Spi;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;
use std::process::Command;

// The exact planes the firmware embeds and streams to the panel. Pointing
// at the firmware's own source makes this golden self-maintaining: if the
// bitmap is regenerated, the expectation follows automatically.
#[path = "../../../examples/esp32-epaper-lab/src/ereader_bitmap.rs"]
mod ereader_bitmap;

fn firmware_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../examples/esp32-epaper-lab/target/xtensa-esp32-none-elf/release/esp32-epaper-lab",
    )
}

fn ensure_firmware_built() -> PathBuf {
    let elf = firmware_path();
    if !elf.exists() {
        // The Xtensa target requires the `+esp` toolchain, which we can't
        // assume the test runner has set up. Skip-warning rather than fail
        // when missing — CI builds the firmware in a separate step.
        eprintln!(
            "[skip] ESP32 ELF not found at {elf:?}; build with: \
             cd core/examples/esp32-epaper-lab && \
             source ~/export-esp.sh && cargo build --release"
        );
        // Try to build it ourselves on the off-chance the env is set up.
        let _ = Command::new("cargo")
            .args(["build", "--release"])
            .current_dir(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/esp32-epaper-lab"),
            )
            .status();
    }
    elf
}

#[test]
#[ignore = "needs +esp-built epaper ELF; NO CI lane builds it yet — manual only"]
fn firmware_drives_panel_to_ereader_bitmap() {
    let elf_path = ensure_firmware_built();
    if !elf_path.exists() {
        // Comment in core-ci.yml claimed CI builds this — it does NOT.
        // The nightly espup lane (core-nightly.yml) is responsible.
        panic!(
            "esp32-epaper-lab ELF not found at {elf_path:?}. \
             Build with: cd core/examples/esp32-epaper-lab && \
             source ~/export-esp.sh && cargo build --release. \
             No CI lane builds this ELF yet (the nightly espup lane covers only the\n             esp32s3-fixtures e2e trio) — run manually after a +esp build."
        );
    }

    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);

    // Attach SSD1680 panel to SPI3 — mirrors what
    // `WasmSimulator::attach_esp32_external_devices` does for the playground.
    bus.attach_spi_device("spi3", Box::new(Ssd1680Tricolor290::new("GPIO5")))
        .expect("spi3 is an Esp32Spi controller");

    bus.refresh_peripheral_index();

    let mut machine = Machine::new(cpu, bus);

    let image = labwired_loader::load_elf(&elf_path).expect("parse ELF");
    machine.load_firmware(&image).expect("load firmware");
    // XtensaLx7::reset() defaults PC to 0x40000400 (BROM reset vector on
    // real silicon). Our sim has no BROM image at that address — for the
    // sim path we skip BROM and start at the ELF's app entry directly,
    // matching what a 2nd-stage bootloader would do post-BROM.
    machine.cpu.set_pc(image.entry_point as u32);

    // Drive the simulator. The firmware does ~600 ms of busy-wait reset
    // pulses + 2 plane streams + refresh — small in instruction count but
    // each spi_write has a busy-poll loop that idles waiting for CMD.USR
    // to clear (synchronous in sim, but we still loop). 100M instructions
    // is a comfortable headroom over the ~10M the firmware actually needs.
    const MAX_STEPS: u64 = 100_000_000;
    let mut wfi_streak = 0u32;
    let mut last_pc = 0u32;
    let mut step_count = 0u64;
    for _ in 0..MAX_STEPS {
        step_count += 1;
        if let Err(e) = machine.step() {
            panic!(
                "CPU step error after {step_count} steps. \
                 Last PC before error: {last_pc:#x}. \
                 Current PC: {:#x}. \
                 Error: {e}",
                machine.cpu.get_pc()
            );
        }
        let pc = machine.cpu.get_pc();
        if pc == last_pc {
            wfi_streak += 1;
            if wfi_streak > 4000 {
                break;
            }
        } else {
            wfi_streak = 0;
            last_pc = pc;
        }
    }

    let spi3_idx = machine
        .bus
        .find_peripheral_index_by_name("spi3")
        .expect("spi3 still registered");
    let any = machine.bus.peripherals[spi3_idx]
        .dev
        .as_any()
        .expect("spi3 supports downcast");
    let spi = any.downcast_ref::<Esp32Spi>().expect("spi3 is Esp32Spi");
    let panel = spi
        .attached_devices
        .iter()
        .find_map(|d| {
            d.as_any()
                .and_then(|a| a.downcast_ref::<Ssd1680Tricolor290>())
        })
        .expect("SSD1680 attached to spi3");

    assert!(
        panel.refresh_generation() >= 1,
        "0x20 master activation must have fired; got generation={}",
        panel.refresh_generation()
    );

    let black = panel.black_plane();
    let red = panel.red_plane();
    assert_eq!(black.len(), 4736);
    assert_eq!(red.len(), 4736);

    // The panel must hold exactly what the firmware streamed: its embedded
    // e-reader bitmap planes, byte-for-byte. This is the digital-twin
    // guarantee — same ELF, same pixels. Report the first divergence with
    // its row/col for a readable failure.
    const WIDTH_BYTES: usize = 16;
    let first_diff = |got: &[u8], want: &[u8]| -> Option<(usize, usize, u8, u8)> {
        got.iter()
            .zip(want)
            .enumerate()
            .find_map(|(idx, (&g, &w))| {
                (g != w).then_some((idx / WIDTH_BYTES, idx % WIDTH_BYTES, g, w))
            })
    };
    if let Some((row, col, got, want)) = first_diff(black, &ereader_bitmap::EREADER_BLACK_PLANE) {
        panic!("black plane[row={row} col={col}] = {got:#04x}, expected {want:#04x} (firmware EREADER_BLACK_PLANE)");
    }
    if let Some((row, col, got, want)) = first_diff(red, &ereader_bitmap::EREADER_RED_PLANE) {
        panic!("red plane[row={row} col={col}] = {got:#04x}, expected {want:#04x} (firmware EREADER_RED_PLANE)");
    }
}
