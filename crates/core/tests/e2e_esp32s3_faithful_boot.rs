// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Regression guard for the ESP32-S3 faithful-ROM (zero-thunk) boot path.
//
// Runs the hello-world esp-hal app on the REAL boot ROM (no
// LABWIRED_ESP32S3_FASTBOOT, no thunk harness) and asserts control never
// transfers to a near-null address during early init. Two historical faults
// this locks down, both firing within the first ~400 executed instructions:
//
//   * step ~105: the real ROM cache helpers esp-hal's `pre_init` calls
//     (rom_config_instruction_cache_mode @0x4000_1a1c etc.) dispatch through
//     `rom_cache_internal_table_ptr` @0x3FCEFFC4, which stayed null because
//     fast-boot skipped the ROM reset's `.data` copy → `callx8 a8` with a8==0.
//     Fixed by replicating the ROM `.data` init (see esp32s3_rom.rs).
//   * step ~399: `esp32_init`'s jump table in the app's `.rodata` (flash
//     D-cache window @0x3C00_0000) read 0 because the faithful path translated
//     XIP through an unprogrammed MMU table → `jx a15` with a15==0. Fixed by
//     using per-window identity XIP for fast-boot (see Esp32s3Opts).
//
// A short run is sufficient — both faults are early. On a real toolchain the
// ELF is prebuilt; skip cleanly if it isn't present.
#![cfg(feature = "esp32s3-fixtures")]

use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
use labwired_core::bus::SystemBus;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts};
use labwired_core::Cpu;
use std::path::PathBuf;

fn firmware_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32s3-hello-world/target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world")
}

#[test]
fn faithful_rom_boot_never_jumps_to_null() {
    let Ok(elf_bytes) = std::fs::read(firmware_path()) else {
        eprintln!("skipping: esp32s3-hello-world ELF not built");
        return;
    };

    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    // This guard is only meaningful on the faithful (real-ROM) path; if no ROM
    // blob is available the config falls back to the thunk harness.
    assert_eq!(
        wiring.boot_mode,
        Esp32s3BootMode::Faithful,
        "expected the faithful real-ROM path (vendored ROM should always resolve)"
    );
    let mut cpu = wiring.cpu;

    fast_boot(
        &elf_bytes,
        &mut bus,
        &mut cpu,
        &BootOpts {
            stack_top_fallback: 0x3FCD_FFF0,
            icache_backing: Some(wiring.icache_backing),
            dcache_backing: Some(wiring.dcache_backing),
            factory_flash_base: None,
        },
    )
    .expect("fast_boot");

    let observers: Vec<std::sync::Arc<dyn labwired_core::SimulationObserver>> = Vec::new();
    let config = labwired_core::SimulationConfig::default();

    // Both historical faults fire by step ~400; 200k gives generous margin
    // through the whole esp-hal init sequence without a full app run.
    for i in 0..200_000u64 {
        let pc = cpu.get_pc();
        assert!(
            pc >= 0x1000,
            "faithful boot jumped to near-null pc=0x{pc:08x} at step {i} \
             (a ROM data pointer or XIP read resolved to 0)"
        );
        cpu.step(&mut bus, &observers, &config)
            .unwrap_or_else(|e| panic!("sim error at step {i} pc=0x{pc:08x}: {e}"));
        let _ = bus.tick_peripherals_with_costs();
    }
}
