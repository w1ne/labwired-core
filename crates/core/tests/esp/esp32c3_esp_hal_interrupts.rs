// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! esp-hal 1.2 interrupts and Embassy async on the ESP32-C3 bare-ELF fast boot.
//!
//! The fixtures are the `examples/esp32c2/esp-hal-*` interrupt crates built
//! for the C3 (see `tests/fixtures/esp32c3-esp-hal-interrupts.md`). Each prints
//! a `PASS`/`FAIL` verdict on the ROM console.
//!
//! The fast boot skips the mask ROM's `ets_intr_unlock`, the only code that
//! sets `mstatus.MIE` before the app runs, and esp-hal never sets it from
//! zero. `boot::esp32c3_rom::prepare_fast_boot_cpu` hands the app the CPU
//! state the ROM leaves; without it no handler runs and no task wakes.

use crate::common;
use labwired_core::boot::esp32c3_rom::{install_fast_boot_rom, prepare_fast_boot_cpu};
use labwired_core::boot::esp32s3_rom::RomImages;
use labwired_core::bus::SystemBus;
use labwired_core::system::riscv::configure_riscv;
use labwired_core::{Cpu as _, Machine};

/// Runs a C3 esp-hal fixture on the fast boot the builder uses
/// (`install_fast_boot_rom`), with or without the ROM's CPU hand-over.
/// Returns the console text once the app prints its verdict or `steps` run
/// out. `Machine::step` never idle-fast-forwards.
fn run_fixture(fixture: &str, hand_over_cpu: bool, steps: usize) -> String {
    let (chip, manifest) = common::system("configs/systems/esp32c3-devkit.yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("ESP32-C3 peripherals");
    assert!(install_fast_boot_rom(
        &mut bus,
        &RomImages {
            irom: common::committed("crates/core/roms/esp32c3/esp32c3_rom.bin"),
            drom: common::committed("crates/core/roms/esp32c3/esp32c3_drom.bin"),
        },
    ));
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let image = labwired_loader::load_elf(&common::root(fixture)).expect("fixture ELF");
    let mut cpu = configure_riscv(&mut bus);
    if hand_over_cpu {
        prepare_fast_boot_cpu(&mut cpu);
    }
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).expect("load fixture");
    let sp_top = (chip.ram.base + chip.ram.size) as u32;
    machine.cpu.set_sp(sp_top & !0xF);
    machine.cpu.set_pc(image.entry_point as u32);
    let uart = || String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    for chunk in 0..steps / 10_000 {
        for _ in 0..10_000 {
            machine
                .step()
                .unwrap_or_else(|e| panic!("step {chunk}: {e:?}; UART {:?}", uart()));
        }
        let text = uart();
        if text.contains(" PASS\n") || text.contains(" FAIL\n") {
            return text;
        }
    }
    uart()
}

fn count(uart: &str, prefix: &str) -> u32 {
    uart.lines()
        .find_map(|l| l.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("no {prefix:?} line in {uart:?}"))
        .trim()
        .parse()
        .expect("count")
}

#[test]
fn esp32c3_esp_hal_software_interrupt_and_systimer_alarm_handlers_run() {
    // Three FROM_CPU raises run the handler three times; a 1 ms periodic
    // SYSTIMER alarm fires 4..6 times during a 5 ms delay.
    let uart = run_fixture("tests/fixtures/esp32c3-esp-hal-irq.elf", true, 6_000_000);
    assert_eq!(count(&uart, "LW-C3 swi hits "), 3, "{uart:?}");
    let alarms = count(&uart, "LW-C3 alarm hits ");
    assert!((4..=6).contains(&alarms), "{alarms} alarm runs: {uart:?}");
    assert!(uart.contains("LW-C3 irq PASS\n"), "{uart:?}");
}

#[test]
fn esp32c3_esp_hal_handlers_never_run_without_the_rom_cpu_hand_over() {
    // Negative control: same bus, MIE left clear as reset leaves it.
    let uart = run_fixture("tests/fixtures/esp32c3-esp-hal-irq.elf", false, 6_000_000);
    assert!(uart.contains("LW-C3 swi hits 0\n"), "{uart:?}");
    assert!(uart.contains("LW-C3 alarm hits 0\n"), "{uart:?}");
    assert!(uart.contains("LW-C3 irq FAIL\n"), "{uart:?}");
}

#[test]
fn esp32c3_esp_hal_gpio_edge_interrupts_run() {
    let uart = run_fixture(
        "tests/fixtures/esp32c3-esp-hal-gpio-irq.elf",
        true,
        6_000_000,
    );
    assert!(uart.contains("LW-C3 gpio-irq PASS\n"), "{uart:?}");
}

#[test]
fn esp32c3_embassy_tasks_run_on_esp_rtos() {
    let uart = run_fixture(
        "tests/fixtures/esp32c3-esp-hal-embassy.elf",
        true,
        6_000_000,
    );
    assert!(uart.contains("LW-C3 embassy PASS\n"), "{uart:?}");
}

#[test]
fn esp32c3_embassy_async_gpio_wakes_on_an_edge() {
    let uart = run_fixture(
        "tests/fixtures/esp32c3-esp-hal-gpio-async.elf",
        true,
        6_000_000,
    );
    assert!(uart.contains("LW-C3 gpio-async PASS\n"), "{uart:?}");
}

#[test]
fn esp32c3_embassy_async_uart_loopback_frames_arrive() {
    let uart = run_fixture(
        "tests/fixtures/esp32c3-esp-hal-uart-async.elf",
        true,
        16_000_000,
    );
    assert!(uart.contains("LW-C3 uart-async PASS\n"), "{uart:?}");
}
