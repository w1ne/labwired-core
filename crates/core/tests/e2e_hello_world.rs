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

mod common;
use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::{Cpu, SimulationError};
use std::sync::{Arc, Mutex};

#[test]
fn hello_world_prints_at_least_twice() {
    let elf_path = common::ensure_esp_firmware_built(
        "esp32s3-hello-world",
        "target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world",
    );
    let elf_bytes = std::fs::read(&elf_path).expect("read firmware ELF");

    let mut bus = SystemBus::new();
    // Pin the modelled core clock to the 80 MHz operating point these tests
    // were written for. `Systimer::cpu_per_systimer` is an integer division
    // (80 MHz / 16 MHz = 5 cycles per SYSTIMER tick exactly), so guest time
    // stays faithful and the budgets below keep their documented meaning.
    // #1026 moved the model default to the chip descriptor's 240 MHz; these
    // end-to-end behaviour tests assert guest-time events only, so paying 3x
    // host time for the higher clock buys no coverage.
    let opts = Esp32s3Opts {
        cpu_clock_hz: 80_000_000,
        ..Esp32s3Opts::default()
    };
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
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
            factory_flash_base: None,
        },
    )
    .expect("fast_boot");

    // Run for up to 500 M simulated cycles (~6 simulated seconds at 80 MHz).
    // Plan-2 verified this fits 2+ "Hello world!" lines paced by SYSTIMER
    // through `Delay::delay_millis` (one per second).
    const MAX_STEPS: u64 = 500_000_000;
    let observers: Vec<std::sync::Arc<dyn labwired_core::SimulationObserver>> = Vec::new();
    let config = labwired_core::SimulationConfig::default();
    for step in 0..MAX_STEPS {
        match cpu.step(&mut bus, &observers, &config) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(_)) => break,
            Err(e) => panic!("simulator error at pc=0x{:08x}: {e}", cpu.get_pc()),
        }
        // Drain peripheral interrupts so SYSTIMER ticks (just like the CLI does).
        let _ = bus.tick_peripherals_with_costs();
        // Publish the cycle so clock-driven peripherals see time advance. The
        // CLI gets this from `Machine::advance`; a raw step loop must do it by
        // hand. Without it the USB_SERIAL_JTAG's measured host-pickup window
        // (235 us) never expires, `DATA_FREE` stays 0, and every byte after
        // the first IN packet is dropped by the (silicon-faithful) model —
        // so this test could never see a second "Hello world!".
        bus.set_current_cycle(step + 1);

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
