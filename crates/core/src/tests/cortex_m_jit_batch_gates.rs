// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The batch gates a compiled backend consults before running a block.
//!
//! [`CortexM::jit_takeable_exception`], [`CortexM::block_would_cross_irq`] and
//! [`CortexM::sysreset_latched`] are compiled for `jit` *and* `jit-framework`
//! because the browser adapter owns its own dispatcher: it compiles Thumb
//! blocks to wasm in the client and must refuse exactly the windows the
//! in-tree JIT refuses. These tests pin the gate semantics on the feature set
//! that has no in-tree dispatcher to exercise them.

use crate::bus::SystemBus;
use crate::cpu::CortexM;
use crate::{Cpu, Peripheral, SimResult};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug)]
struct SystickDeadline(u64);

impl Peripheral for SystickDeadline {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn systick_ticks_until_fire(&self) -> Option<u64> {
        Some(self.0)
    }
}

#[test]
fn takeable_exception_gate_matches_mask_state() {
    let mut cpu = CortexM::new();
    assert!(
        !cpu.jit_takeable_exception(),
        "nothing pending, nothing to take"
    );

    cpu.set_exception_pending(15);
    assert!(
        cpu.jit_takeable_exception(),
        "an unmasked pending IRQ is takeable"
    );

    cpu.primask = true;
    assert!(
        !cpu.jit_takeable_exception(),
        "PRIMASK masks the exception, so a compiled block may run"
    );
}

#[test]
fn systick_edge_inside_a_block_refuses_it() {
    let mut bus = SystemBus::new();
    let cpu = CortexM::new();
    assert!(
        !cpu.block_would_cross_irq(&bus, 8),
        "no SysTick on the bus, no edge to cross"
    );

    bus.add_peripheral(
        "fake-systick",
        0xE000_E010,
        0x1000,
        None,
        Box::new(SystickDeadline(4)),
    );
    assert!(
        !cpu.block_would_cross_irq(&bus, 3),
        "a block ending before the edge may compile"
    );
    assert!(
        cpu.block_would_cross_irq(&bus, 4),
        "a block landing on the edge must interpret"
    );
}

#[test]
fn sysreset_latch_is_visible_to_compiled_backends() {
    let mut cpu = CortexM::new();
    assert!(
        !cpu.sysreset_latched(),
        "no SCB wired means no latch to observe"
    );

    let signal = Arc::new(AtomicBool::new(true));
    cpu.set_shared_sysreset_signal(signal);
    assert!(
        cpu.sysreset_latched(),
        "a compiled window must be able to end on the AIRCR store"
    );
}
