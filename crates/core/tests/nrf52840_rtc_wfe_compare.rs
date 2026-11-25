// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! nRF52840 RTC1 compare must advance, pend, and wake WFE while the IRQ is masked.
//!
//! Product Specification RTC chapter: COUNTER increments at
//! 32.768 kHz / (PRESCALER + 1). A CC[0] match (COUNTER transitioning to N)
//! sets EVENTS_COMPARE[0]. With INTENSET.COMPARE0 set, that pends IRQ 17 in
//! ISPR even if NVIC ISER bit 17 is clear. FreeRTOS tickless idle parks in
//! WFE and polls ISPR; Nordic's port relies on SCR.SEVONPEND so the masked
//! pending bit wakes WFE.

mod common;

use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::machine::AdvanceStop;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Bus, Cpu, Machine};
use std::num::NonZeroU32;

const RTC1: u64 = 0x4001_1000;
const CLOCK: u64 = 0x4000_0000;
const IRQ17: u32 = 1 << 17;
const CODE: u32 = 0x2000_1000;
const SEVONPEND: u32 = 1 << 4;

fn nrf52840_machine() -> Machine<CortexM> {
    let chip =
        ChipDescriptor::from_file(root("configs/chips/nrf52840.yaml")).expect("nrf52840 chip yaml");
    let system_path = root("configs/systems/nrf52840-dk.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("nrf52840-dk system yaml");
    let anchored = system_path
        .parent()
        .expect("system parent")
        .join(&manifest.chip);
    manifest.chip = anchored.to_string_lossy().into_owned();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("nrf52840 bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    Machine::new(cpu, bus)
}

#[test]
fn rtc1_compare_advances_during_wfe_and_pends_masked_irq17() {
    let mut machine = nrf52840_machine();

    // PS CLOCK chapter: software must start LFCLK before using the RTC.
    machine.bus.write_u32(CLOCK + 0x008, 1).unwrap();

    // PRESCALER=0 => 32.768 kHz. CC[0]=4 fires on the 4th increment (~7812.5 CPU cycles).
    machine.bus.write_u32(RTC1 + 0x508, 0).unwrap();
    machine.bus.write_u32(RTC1 + 0x540, 4).unwrap();
    machine.bus.write_u32(RTC1 + 0x304, 1 << 16).unwrap(); // INTENSET COMPARE0
    machine.bus.write_u32(RTC1, 1).unwrap(); // TASKS_START
                                             // Leave ISER bit 17 clear: the IRQ stays masked.

    // Nordic/FreeRTOS tickless idle: SEVONPEND so a masked pending IRQ wakes WFE.
    machine.bus.write_u32(0xE000_ED10, SEVONPEND).unwrap();

    // WFE; WFE; b .  — first WFE consumes any leftover event, second parks.
    machine.bus.write_u16(CODE as u64, 0xBF20).unwrap();
    machine.bus.write_u16(CODE as u64 + 2, 0xBF20).unwrap();
    machine.bus.write_u16(CODE as u64 + 4, 0xE7FE).unwrap();
    machine.cpu.set_sp(0x2000_8000);
    machine.cpu.set_pc(CODE);

    let report = machine
        .advance(
            AdvanceRequest::run(Some(20_000))
                .with_cycle_limit(20_000)
                .with_batch_cap(NonZeroU32::new(512).expect("batch")),
        )
        .expect("advance");

    let counter = machine.bus.read_u32(RTC1 + 0x504).unwrap();
    let ev_cmp0 = machine.bus.read_u32(RTC1 + 0x140).unwrap();
    let ispr = machine.bus.read_u32(0xE000_E200).unwrap();
    let iser = machine.bus.read_u32(0xE000_E100).unwrap();
    let pc = machine.cpu.get_pc();

    assert_eq!(iser & IRQ17, 0, "IRQ 17 must stay masked");
    assert!(
        counter >= 4,
        "COUNTER must advance at 32.768 kHz during WFE; COUNTER={counter:#x} ev={ev_cmp0} ispr={ispr:#x} pc={pc:#x} stop={:?}",
        report.stop
    );
    assert_eq!(
        ev_cmp0, 1,
        "EVENTS_COMPARE[0] must be set when COUNTER reaches CC0"
    );
    assert_eq!(
        ispr & IRQ17,
        IRQ17,
        "IRQ 17 must pend in ISPR even while masked; ISPR={ispr:#x}"
    );
    assert!(
        pc >= CODE + 4,
        "WFE must wake on the masked pending compare; pc={pc:#x} stop={:?}",
        report.stop
    );
    assert!(
        !matches!(report.stop, AdvanceStop::NoProgress),
        "WFE sleep must keep time moving until the compare pends"
    );
}
