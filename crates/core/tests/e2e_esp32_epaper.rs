// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// End-to-end test: load the `esp32-epaper-lab` ELF, run it in the
// simulator wired with our ESP32-classic GPIO + VSPI peripherals, and
// verify the SSD1680 panel received the expected three-band pattern.
//
// This is the side-by-side fidelity check that justifies "same ELF on
// chip and in sim" for the AgentDeck-style ESP32 + tri-color e-paper
// hardware target.

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::Ssd1680Tricolor290;
use labwired_core::peripherals::esp32::spi::Esp32Spi;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;
use std::process::Command;

fn firmware_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-epaper-lab/target/xtensa-esp32-none-elf/release/esp32-epaper-lab")
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
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../examples/esp32-epaper-lab"),
            )
            .status();
    }
    elf
}

#[test]
#[ignore = "v0.6 WIP: firmware traps in esp-hal __pre_init → esp32_init before reaching main; needs DPORT/IO_MUX peripheral stubs or a weak-symbol override of esp-hal's pre-init hook."]
fn firmware_drives_panel_to_three_band_pattern() {
    let elf_path = ensure_firmware_built();
    if !elf_path.exists() {
        eprintln!("[skip] esp32-epaper-lab ELF unavailable; skipping");
        return;
    }

    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);

    // Attach SSD1680 panel to SPI3 — mirrors what
    // `WasmSimulator::attach_esp32_external_devices` does for the playground.
    let spi3_idx = bus
        .find_peripheral_index_by_name("spi3")
        .expect("spi3 must be registered by configure_xtensa_esp32");
    let any = bus.peripherals[spi3_idx]
        .dev
        .as_any_mut()
        .expect("spi3 supports downcast");
    let spi = any
        .downcast_mut::<Esp32Spi>()
        .expect("spi3 is Esp32Spi");
    spi.attach(Box::new(Ssd1680Tricolor290::new("GPIO5")));

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
    for _ in 0..MAX_STEPS {
        if machine.step().is_err() {
            // Many Xtensa instructions our LX6/LX7 decoder doesn't
            // implement will error; we treat that as the test failing
            // (we want the demo's hot path to be fully decodable).
            panic!("CPU step error at PC {:#x}", machine.cpu.get_pc());
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
        .find_map(|d| d.as_any().and_then(|a| a.downcast_ref::<Ssd1680Tricolor290>()))
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
                "black plane[row={row} col={col_byte}] expected {exp_b:#04x}, got {:#04x}",
                black[idx]
            );
            assert_eq!(
                red[idx], exp_r,
                "red plane[row={row} col={col_byte}] expected {exp_r:#04x}, got {:#04x}",
                red[idx]
            );
        }
    }
}
