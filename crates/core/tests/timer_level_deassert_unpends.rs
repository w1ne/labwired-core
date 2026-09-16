// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A LEVEL interrupt's pending bit follows its line DOWN, not only up.
//!
//! ## The defect this pins
//!
//! The timer holds its IRQ line as a level (`SR & DIER != 0`) and the walk
//! re-pends ISPR on every held tick — but nothing ever DROPPED the pend when
//! firmware cleared the flag, so a handler that cleared UIF still returned
//! into a stale pend and ran a second time. Measured cross-simulator on the
//! same F0 ELF: 1.95 handler entries per update event against the reference
//! tier's 0.97, while the update GRID itself was cycle-exact — the events
//! were right and each was delivered twice.
//!
//! ## What silicon does (ARM GIC/NVIC level semantics)
//!
//! For a level-sensitive interrupt, pending tracks the line while the
//! exception has not been taken: deassert the line (clear the status flag)
//! and the pend evaporates. A SOFTWARE pend (ISPR write) is different — it
//! fires once even on a low line. Both directions are asserted here.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

const RCC_APB1ENR: u64 = 0x4002_1000 + 0x1C;
const TIM2: u64 = 0x4000_0000;
const TIM2_CR1: u64 = TIM2;
const TIM2_DIER: u64 = TIM2 + 0x0C;
const TIM2_SR: u64 = TIM2 + 0x10;
const TIM2_PSC: u64 = TIM2 + 0x28;
const TIM2_ARR: u64 = TIM2 + 0x2C;
/// tim2's NVIC position in configs/chips/stm32f103.yaml.
const TIM2_IRQ: u32 = 28;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn f103_bus() -> SystemBus {
    let chip_path = root("configs/chips/stm32f103.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("chip yaml");
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "schema_version: \"1.0\"\nname: level-deassert\nchip: \"{}\"\n",
        chip_path.display()
    ))
    .expect("minimal manifest");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build f103 bus");
    let _ = configure_cortex_m(&mut bus);
    bus
}

/// Spin loop at 0x40: `b .` (0xE7FE). Lets the batched `Machine::run` path —
/// the one that ticks peripherals and drains the event scheduler — advance
/// cycles without executing firmware that touches anything. The IRQ is never
/// enabled in the NVIC, so a pend is observed in ISPR and never taken.
const SPIN_PC: u32 = 0x40;
const SPIN: u16 = 0xE7FE;
const INITIAL_SP: u32 = 0x2000_8000;

fn f103_machine() -> Machine<CortexM> {
    let chip_path = root("configs/chips/stm32f103.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("chip yaml");
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "schema_version: \"1.0\"\nname: level-deassert\nchip: \"{}\"\n",
        chip_path.display()
    ))
    .expect("minimal manifest");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build f103 bus");
    bus.write_u16(u64::from(SPIN_PC), SPIN).expect("spin loop");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.cpu.pc = SPIN_PC;
    machine.cpu.sp = INITIAL_SP;
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine
}

fn ispr_bit(bus: &SystemBus, irq: u32) -> bool {
    let nvic = bus.nvic.as_ref().expect("configured NVIC");
    (nvic.ispr[(irq / 32) as usize].load(Ordering::SeqCst) & (1 << (irq % 32))) != 0
}

/// Arm TIM2 for an update event every few ticks and run the machine until the
/// pend appears. Returns mid-event: UIF set, ISPR set.
///
/// Driven through `Machine::run`, not a bare `tick_peripherals_fully` loop:
/// under `event-scheduler` the timer is scheduler-driven (`uses_scheduler()`
/// → its `tick()` is inert), and only the machine boundary drains the
/// scheduled event that latches UIF and pends the NVIC. With the feature off
/// the same loop walks the timer exactly as before, so both lanes exercise
/// the semantics this file pins.
fn machine_with_held_update() -> Machine<CortexM> {
    let mut machine = f103_machine();
    machine.bus.write_u32(RCC_APB1ENR, 1).expect("TIM2EN");
    machine.bus.write_u32(TIM2_PSC, 0).expect("psc");
    machine.bus.write_u32(TIM2_ARR, 3).expect("arr");
    machine.bus.write_u32(TIM2_DIER, 1).expect("UIE");
    machine.bus.write_u32(TIM2_CR1, 1).expect("CEN");
    for _ in 0..64 {
        machine.run(Some(1)).expect("spin loop");
        if ispr_bit(&machine.bus, TIM2_IRQ) {
            return machine;
        }
    }
    panic!("TIM2 never pended an update in 64 ticks");
}

#[test]
fn clearing_the_flag_drops_the_level_pend() {
    let mut machine = machine_with_held_update();
    assert_ne!(
        machine.bus.read_u32(TIM2_SR).expect("SR") & 1,
        0,
        "UIF latched at the event"
    );

    // The handler's store: clear UIF. This is the line deasserting.
    machine.bus.write_u32(TIM2_SR, 0).expect("clear UIF");

    assert!(
        !ispr_bit(&machine.bus, TIM2_IRQ),
        "a level pend must follow its line down — the stale pend is the \
         second handler entry"
    );

    // And it STAYS down across further quiet ticks (the walk must not
    // resurrect it from stale state).
    for _ in 0..3 {
        machine.run(Some(1)).expect("spin loop");
    }
    // (the counter is free again, so a NEW event may legitimately pend later;
    // three ticks is inside the 4-tick period armed above)
    assert!(
        !ispr_bit(&machine.bus, TIM2_IRQ),
        "no resurrection before the next real event"
    );
}

#[test]
fn a_software_pend_survives_a_low_line() {
    let mut bus = f103_bus();
    bus.write_u32(RCC_APB1ENR, 1).expect("TIM2EN");
    // Line is low: timer disabled, SR clear. Software-pend TIM2 via ISPR.
    let nvic = bus.nvic.clone().expect("nvic");
    nvic.ispr[(TIM2_IRQ / 32) as usize].fetch_or(1 << (TIM2_IRQ % 32), Ordering::SeqCst);

    // An unrelated MMIO write to the timer runs the write-choke reconcile
    // with level == false. The software pend is NOT level-marked and must
    // survive — on silicon it fires once even with the line low.
    bus.write_u32(TIM2_ARR, 100).expect("arr");
    for _ in 0..4 {
        let _ = bus.tick_peripherals_fully();
    }
    assert!(
        ispr_bit(&bus, TIM2_IRQ),
        "a software ISPR pend of a low line must not be auto-cleared"
    );
}

#[test]
fn an_unserviced_level_keeps_pending() {
    // The other direction: firmware that never clears UIF keeps the pend —
    // level semantics, not one-shot.
    let mut machine = machine_with_held_update();
    for _ in 0..8 {
        machine.run(Some(1)).expect("spin loop");
    }
    assert!(
        ispr_bit(&machine.bus, TIM2_IRQ),
        "a held line stays pended until the flag is cleared"
    );
}
