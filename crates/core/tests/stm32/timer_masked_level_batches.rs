// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A held timer level the core cannot take yet (PRIMASK set, as firmware
//! does around a timing-critical section) must not pin the CPU quantum to one
//! instruction.
//!
//! The timer's event chain re-pends its held line every cycle so the NVIC
//! sees a level. While that pend is still standing (enabled, pending,
//! level-marked) a re-pend changes nothing, but the event at `now + 1` bound
//! every batch to width 1: the real AM32 ESC firmware ran its whole
//! ~72 M-instruction startup tune (`__disable_irq()` around `delayMillis`)
//! at 1.00 instructions per batch. The chain now parks while its pend
//! stands, and the bus re-arms it at the cycle after exception entry
//! consumes the pend: the same cycle the per-cycle chain re-pended on.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Bus, Machine};
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
const NVIC_ISER0: u64 = 0xE000_E100;
/// Thread code: `b .` at 0x200. Handler: `b .` at 0x300, never clears UIF.
const THREAD_PC: u32 = 0x200;
const HANDLER_PC: u32 = 0x300;
const SPIN: u16 = 0xE7FE;
const INITIAL_SP: u32 = 0x2000_5000;

fn machine() -> Machine<CortexM> {
    let chip_path = root("configs/chips/stm32f103.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("chip yaml");
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "schema_version: \"1.0\"\nname: masked-level\nchip: \"{}\"\n",
        chip_path.display()
    ))
    .expect("minimal manifest");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build f103 bus");
    bus.write_u16(u64::from(THREAD_PC), SPIN).unwrap();
    bus.write_u16(u64::from(HANDLER_PC), SPIN).unwrap();
    bus.write_u32(u64::from(16 + TIM2_IRQ) * 4, HANDLER_PC | 1)
        .unwrap();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    m.cpu.pc = THREAD_PC;
    m.cpu.sp = INITIAL_SP;
    let interval = m.bus.max_safe_tick_interval();
    m.config.peripheral_tick_interval = interval;
    m.bus.config.peripheral_tick_interval = interval;
    m
}

fn ispr_bit(bus: &SystemBus) -> bool {
    let nvic = bus.nvic.as_ref().expect("configured NVIC");
    nvic.ispr[(TIM2_IRQ / 32) as usize].load(Ordering::SeqCst) & (1 << (TIM2_IRQ % 32)) != 0
}

/// TIM2 update every 4 cycles, enabled in the NVIC, core masked by PRIMASK.
fn masked_machine_with_held_update() -> Machine<CortexM> {
    let mut m = machine();
    m.cpu.primask = true;
    m.bus.write_u32(RCC_APB1ENR, 1).unwrap();
    m.bus.write_u32(NVIC_ISER0, 1 << TIM2_IRQ).unwrap();
    m.bus.write_u32(TIM2_PSC, 0).unwrap();
    m.bus.write_u32(TIM2_ARR, 3).unwrap();
    m.bus.write_u32(TIM2_DIER, 1).unwrap();
    m.bus.write_u32(TIM2_CR1, 1).unwrap();
    m.advance(AdvanceRequest::run(Some(64))).unwrap();
    assert!(ispr_bit(&m.bus), "TIM2 update pended");
    assert_ne!(m.bus.read_u32(TIM2_SR).unwrap() & 1, 0, "UIF held");
    m
}

/// The parked chain is a scheduler concept; the walk lane re-pends per tick.
#[cfg(feature = "event-scheduler")]
#[test]
fn a_masked_held_level_does_not_pin_the_quantum_to_one_instruction() {
    let mut m = masked_machine_with_held_update();
    let report = m.advance(AdvanceRequest::run(Some(100_000))).unwrap();
    assert_eq!(report.fuel_consumed, 100_000);
    let width = report.fuel_consumed as f64 / report.cpu_batches.max(1) as f64;
    assert!(
        width > 100.0,
        "a standing level pend kept a per-cycle event alive: {} batches for {} \
         instructions ({width:.2} per batch)",
        report.cpu_batches,
        report.fuel_consumed
    );
    assert!(ispr_bit(&m.bus), "the level stays pending while masked");
    assert_eq!(
        m.cpu.active_exception, 0,
        "PRIMASK holds the core in thread mode"
    );
}

#[test]
fn unmasking_takes_the_held_level_and_it_pends_again_while_active() {
    let mut m = masked_machine_with_held_update();
    m.advance(AdvanceRequest::run(Some(10_000))).unwrap();
    m.cpu.primask = false;
    m.advance(AdvanceRequest::run(Some(16))).unwrap();
    assert_eq!(
        m.cpu.active_exception,
        16 + TIM2_IRQ,
        "the held update is taken once PRIMASK clears"
    );
    // The handler never clears UIF: the line is still high, so the NVIC holds
    // it pending while active, as on silicon.
    m.advance(AdvanceRequest::run(Some(1_000))).unwrap();
    assert!(
        ispr_bit(&m.bus),
        "a held line re-pends after exception entry"
    );
    // Clearing the flag drops the pend (level follows the line down).
    m.bus.write_u32(TIM2_SR, 0).unwrap();
    assert!(!ispr_bit(&m.bus));
}
