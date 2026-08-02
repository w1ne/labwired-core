// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Plan 3 Task 11: end-to-end test that builds examples/esp32s3-blinky and
// runs it in the simulator, asserting that GPIO2 toggles at the expected
// cadence via a recording GpioObserver.
//
// Gated on `--features esp32s3-fixtures` so plain `cargo test` (without
// the ESP toolchain) still works.

#![cfg(feature = "esp32s3-fixtures")]

use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::esp32s3::gpio::GpioObserver;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::{Cpu, SimulationError};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Recording observer that captures every (pin, from, to, sim_cycle)
/// transition emitted by the GPIO peripheral.
#[derive(Debug, Default)]
struct RecordingObserver {
    events: Mutex<Vec<(u8, bool, bool, u64)>>,
}

impl GpioObserver for RecordingObserver {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64) {
        self.events.lock().unwrap().push((pin, from, to, sim_cycle));
    }
}

/// Path to the firmware ELF, relative to the workspace root.
fn firmware_path() -> PathBuf {
    PathBuf::from(
        "../../examples/esp32s3-blinky/target/xtensa-esp32s3-none-elf/release/esp32s3-blinky",
    )
}

/// Build the firmware crate via `cargo +esp build --release`.
/// Skips the build if the ELF already exists and is newer than `src/main.rs`.
fn ensure_firmware_built() -> PathBuf {
    let elf = firmware_path();
    let src = PathBuf::from("../../examples/esp32s3-blinky/src/main.rs");
    if elf.exists() {
        if let (Ok(elf_meta), Ok(src_meta)) = (std::fs::metadata(&elf), std::fs::metadata(&src)) {
            if elf_meta.modified().unwrap() >= src_meta.modified().unwrap() {
                return elf;
            }
        }
    }
    let status = Command::new("cargo")
        .args(["+esp", "build", "--release"])
        .current_dir("../../examples/esp32s3-blinky")
        .status()
        .expect("cargo +esp build (is the ESP toolchain installed?)");
    assert!(status.success(), "esp32s3-blinky build failed");
    assert!(elf.exists(), "ELF not found at {:?} after build", elf);
    elf
}

#[test]
fn blinky_toggles_gpio2_at_500ms() {
    let elf_path = ensure_firmware_built();
    let elf_bytes = std::fs::read(&elf_path).expect("read firmware ELF");

    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

    // Install a recording GPIO observer before fast-boot so we capture
    // every transition (including any during early init, though blinky
    // shouldn't toggle GPIO2 until the first alarm fires ~500 ms in).
    let obs = Arc::new(RecordingObserver::default());
    wiring.add_gpio_observer(&mut bus, obs.clone());

    let icache_backing = wiring.icache_backing.clone();
    let dcache_backing = wiring.dcache_backing.clone();
    let mut cpu = wiring.cpu;

    fast_boot(
        &elf_bytes,
        &mut bus,
        &mut cpu,
        &BootOpts {
            stack_top_fallback: 0x3FCD_FFF0,
            icache_backing: Some(icache_backing),
            dcache_backing: Some(dcache_backing),
            factory_flash_base: None,
        },
    )
    .expect("fast_boot");

    // Run for up to 480 M simulated cycles (~6 simulated seconds at 80 MHz).
    // Blinky toggles every 500 ms = 40M cycles, so 6 s should produce ~12
    // transitions. We assert >= 4 to give plenty of margin.
    const MAX_STEPS: u64 = 480_000_000;
    let observers: Vec<std::sync::Arc<dyn labwired_core::SimulationObserver>> = Vec::new();
    let config = labwired_core::SimulationConfig::default();
    for _ in 0..MAX_STEPS {
        match cpu.step(&mut bus, &observers, &config) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(_)) => break,
            Err(e) => panic!("simulator error at pc=0x{:08x}: {e}", cpu.get_pc()),
        }
        // Drain peripheral interrupts so SYSTIMER ticks (just like the CLI does).
        let _ = bus.tick_peripherals_with_costs();

        // Early exit once we've captured enough GPIO2 transitions.
        let events = obs.events.lock().unwrap();
        let pin2_count = events.iter().filter(|&&(p, _, _, _)| p == 2).count();
        if pin2_count >= 4 {
            return;
        }
    }

    let events = obs.events.lock().unwrap();
    let pin2_events: Vec<_> = events.iter().filter(|&&(p, _, _, _)| p == 2).collect();
    panic!(
        "did not see 4+ GPIO2 transitions in {MAX_STEPS} steps; \
         pin2 events: {pin2_events:?}, total events: {}",
        events.len(),
    );
}
