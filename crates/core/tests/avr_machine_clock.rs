// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! An AVR machine's clock is the core's datasheet cycles.
//!
//! The ATmega core charges 1–4 clock cycles per step and its Timer0, which is
//! what Arduino's `millis()` and `delay()` count, advances by exactly those.
//! `Machine::total_cycles` is the clock test triggers, co-simulation and traces
//! convert to time, so on this core it must advance by the same cycles, or
//! `millis()`, an analog circuit and an `after_cycles` stimulus would each run
//! on a different time base.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::avr::{strip_avr_data_bias, Avr, SRAM_START};
use labwired_core::{AdvanceRequest, AdvanceStop, Cpu, Machine};
use std::path::{Path, PathBuf};

const CPU_HZ: u64 = 16_000_000;

fn root(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn nano_elf() -> Vec<u8> {
    std::fs::read(root("tests/fixtures/avr/arduino-nano-blinky.elf")).expect("Nano blink fixture")
}

/// The committed Nano blink sketch (`Serial.println`, `delay(1)` loop) on a bus
/// built from the shipped descriptor and board manifest.
fn nano_machine(tick_interval: u32) -> Machine<Avr> {
    let chip = ChipDescriptor::from_file(root("configs/chips/atmega328p.yaml")).expect("chip");
    let manifest = SystemManifest::from_file(root("configs/systems/arduino-nano.yaml"))
        .expect("Nano manifest");
    let bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let image = labwired_loader::load_elf_bytes(&nano_elf()).expect("load ELF");
    let mut cpu = Avr::new();
    cpu.load_program_image(&image);
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;
    machine
}

/// Arduino core's `timer0_millis`, read from SRAM through the ELF symbol.
fn millis(machine: &Machine<Avr>) -> u32 {
    let address = labwired_loader::resolve_symbol_in_elf(&nano_elf(), "timer0_millis")
        .expect("the sketch links timer0_millis");
    let data = strip_avr_data_bias(u64::from(address)).expect("a data-space address") as usize;
    let at = data - usize::from(SRAM_START);
    u32::from_le_bytes(machine.cpu.sram[at..at + 4].try_into().expect("4 bytes"))
}

fn run_one_second(machine: &mut Machine<Avr>) {
    while machine.total_cycles < CPU_HZ {
        let left = CPU_HZ - machine.total_cycles;
        machine
            .advance(AdvanceRequest::run(None).with_cycle_limit(left.min(100_000)))
            .expect("advance");
    }
}

/// One simulated second by the machine clock is one second of `millis()`,
/// at the CLI's per-instruction tick and at the browser's wider one.
#[test]
fn millis_agrees_with_the_machine_clock() {
    for tick_interval in [1, 64] {
        let mut machine = nano_machine(tick_interval);
        run_one_second(&mut machine);
        let by_machine_ms = machine.total_cycles as f64 * 1000.0 / CPU_HZ as f64;
        let by_firmware_ms = f64::from(millis(&machine));
        let error = (by_firmware_ms - by_machine_ms).abs() / by_machine_ms;
        println!(
            "tick {tick_interval}: {} machine cycles = {by_machine_ms:.1} ms, millis() = \
             {by_firmware_ms} ({:.2} % off)",
            machine.total_cycles,
            error * 100.0
        );
        assert!(
            error < 0.01,
            "tick {tick_interval}: millis() reads {by_firmware_ms} ms after {by_machine_ms:.1} ms \
             of machine time"
        );
        assert_eq!(
            machine.total_cycles, machine.cpu.cycles,
            "tick {tick_interval}: the machine clock is the core's clock"
        );
    }
}

/// `Machine::step` charges what the instruction took, not one cycle.
#[test]
fn a_single_step_charges_the_instructions_cycles() {
    let mut machine = nano_machine(1);
    // CALL (4 cycles), then RET (4), then LDI (1), from a hand-assembled image.
    // 0x0000 CALL 0x0004 (word 2) ; 0x0004 LDI R16,1 ; 0x0006 RET
    machine.cpu.load_words(0, &[0x940E, 0x0002, 0xE001, 0x9508]);
    machine.cpu.set_pc(0);
    for expected in [4, 1, 4] {
        let before = machine.total_cycles;
        machine.step().expect("step");
        assert_eq!(machine.total_cycles - before, expected);
    }
    assert_eq!(machine.total_cycles, machine.cpu.cycles);
}

/// A cycle budget ends at the budget or within one step (4 cycles) past it,
/// never a whole batch past it, at either tick width.
#[test]
fn a_cycle_limit_stops_within_one_step() {
    for tick_interval in [1, 64] {
        let mut machine = nano_machine(tick_interval);
        for budget in [1, 3, 16, 100, 1_000, 16_001] {
            let start = machine.total_cycles;
            let report = machine
                .advance(AdvanceRequest::run(None).with_cycle_limit(budget))
                .expect("advance");
            assert_eq!(report.stop, AdvanceStop::CycleLimit);
            let elapsed = machine.total_cycles - start;
            assert_eq!(elapsed, report.elapsed_cycles);
            assert!(
                (budget..budget + 4).contains(&elapsed),
                "tick {tick_interval}: budget {budget} ran {elapsed} cycles"
            );
        }
    }
}
