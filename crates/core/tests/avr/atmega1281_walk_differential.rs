// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The committed ATmega1281 image must produce the same machine state when
//! advanced one instruction at a time and in scheduler batches. This exercises
//! the exact 128 KB/two-byte-PC chip descriptor, startup code, USART0, and PB5.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::{DebugControl, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const STEPS: u32 = 50_000;
const BATCH: u32 = 97;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn machine() -> (Machine<Avr>, Arc<Mutex<Vec<u8>>>) {
    let system_path = root().join("examples/atmega1281-io/system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load system.yaml");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let elf = std::fs::read(root().join("tests/fixtures/avr/atmega1281-io.elf"))
        .expect("missing compiled ATmega1281 firmware fixture");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse AVR-GCC ELF");
    let mut cpu = Avr::for_chip_name(&chip.name);
    cpu.load_program_image(&image);
    let serial = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(serial.clone());
    (Machine::new(cpu, bus), serial)
}

#[test]
fn committed_firmware_matches_instruction_walk_and_scheduler_batches() {
    let (mut walk, walk_serial) = machine();
    let (mut scheduled, scheduled_serial) = machine();
    let portb = walk
        .bus
        .find_peripheral_index_by_name("portb")
        .expect("portb");
    let scheduled_portb = scheduled
        .bus
        .find_peripheral_index_by_name("portb")
        .expect("portb");

    for _ in 0..STEPS {
        walk.step().expect("single-instruction reference walk");
    }
    let mut remaining = STEPS;
    while remaining > 0 {
        let batch = remaining.min(BATCH);
        scheduled.run(Some(batch)).expect("scheduler batch");
        remaining -= batch;
    }

    assert_eq!(
        walk.total_cycles, scheduled.total_cycles,
        "simulated cycles"
    );
    assert_eq!(walk.cpu.model, scheduled.cpu.model);
    assert_eq!(walk.cpu.pc, scheduled.cpu.pc, "program counter");
    assert_eq!(walk.cpu.r, scheduled.cpu.r, "AVR registers");
    assert_eq!(walk.cpu.sp, scheduled.cpu.sp, "stack pointer");
    assert_eq!(walk.cpu.sreg, scheduled.cpu.sreg, "status register");
    assert_eq!(walk.cpu.sram, scheduled.cpu.sram, "SRAM");
    assert_eq!(walk.cpu.io, scheduled.cpu.io, "I/O registers");
    assert_eq!(
        walk.cpu.pending_irq, scheduled.cpu.pending_irq,
        "pending IRQs"
    );
    assert_eq!(walk.cpu.cycles, scheduled.cpu.cycles, "CPU cycles");
    assert_eq!(walk.cpu.tcnt0, scheduled.cpu.tcnt0, "Timer0 counter");
    assert_eq!(walk.cpu.tcnt2, scheduled.cpu.tcnt2, "Timer2 counter");
    assert_eq!(walk.cpu.serial_tx, scheduled.cpu.serial_tx, "USART0 bytes");
    assert_eq!(
        *walk_serial.lock().unwrap(),
        *scheduled_serial.lock().unwrap()
    );
    assert_eq!(
        walk.bus.peripherals[portb].dev.read_gpio_output(5),
        scheduled.bus.peripherals[scheduled_portb]
            .dev
            .read_gpio_output(5),
        "PB5 package output"
    );
    assert!(
        walk_serial
            .lock()
            .unwrap()
            .windows(8)
            .any(|window| window == b"1281 OK\n"),
        "both lanes must execute the committed firmware and print its marker"
    );
    assert_eq!(
        walk.bus.peripherals[portb].dev.read_gpio_output(5),
        Some(true),
        "firmware must drive PB5 high"
    );
}
