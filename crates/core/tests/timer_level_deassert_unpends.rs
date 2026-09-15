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
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
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

fn ispr_bit(bus: &SystemBus, irq: u32) -> bool {
    let nvic = bus.nvic.as_ref().expect("configured NVIC");
    (nvic.ispr[(irq / 32) as usize].load(Ordering::SeqCst) & (1 << (irq % 32))) != 0
}

/// Arm TIM2 for an update event every few ticks and run the bus until the
/// pend appears. Returns the bus mid-event: UIF set, ISPR set.
fn bus_with_held_update() -> SystemBus {
    let mut bus = f103_bus();
    bus.write_u32(RCC_APB1ENR, 1).expect("TIM2EN");
    bus.write_u32(TIM2_PSC, 0).expect("psc");
    bus.write_u32(TIM2_ARR, 3).expect("arr");
    bus.write_u32(TIM2_DIER, 1).expect("UIE");
    bus.write_u32(TIM2_CR1, 1).expect("CEN");
    for _ in 0..64 {
        let _ = bus.tick_peripherals_fully();
        if ispr_bit(&bus, TIM2_IRQ) {
            return bus;
        }
    }
    panic!("TIM2 never pended an update in 64 ticks");
}

#[test]
fn clearing_the_flag_drops_the_level_pend() {
    let mut bus = bus_with_held_update();
    assert_ne!(
        bus.read_u32(TIM2_SR).expect("SR") & 1,
        0,
        "UIF latched at the event"
    );

    // The handler's store: clear UIF. This is the line deasserting.
    bus.write_u32(TIM2_SR, 0).expect("clear UIF");

    assert!(
        !ispr_bit(&bus, TIM2_IRQ),
        "a level pend must follow its line down — the stale pend is the \
         second handler entry"
    );

    // And it STAYS down across further quiet ticks (the walk must not
    // resurrect it from stale state).
    for _ in 0..3 {
        let _ = bus.tick_peripherals_fully();
    }
    // (the counter is free again, so a NEW event may legitimately pend later;
    // three ticks is inside the 4-tick period armed above)
    assert!(
        !ispr_bit(&bus, TIM2_IRQ),
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
    let mut bus = bus_with_held_update();
    for _ in 0..8 {
        let _ = bus.tick_peripherals_fully();
    }
    assert!(
        ispr_bit(&bus, TIM2_IRQ),
        "a held line stays pended until the flag is cleared"
    );
}
