// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::{PeripheralEntry, SystemBus};
use crate::cpu::CortexM;
use crate::peripherals::dwt::Dwt;
use crate::peripherals::nvic::{Nvic, NvicState};
use crate::peripherals::scb::{Scb, SharedScbState};
use crate::Peripheral;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;

pub fn configure_cortex_m(bus: &mut SystemBus) -> (CortexM, Arc<NvicState>) {
    let vtor = Arc::new(AtomicU32::new(0));
    let vectactive = Arc::new(AtomicU32::new(0));
    let shpr1 = Arc::new(AtomicU32::new(0));
    let shpr2 = Arc::new(AtomicU32::new(0));
    let shpr3 = Arc::new(AtomicU32::new(0));
    let nvic_state = Arc::new(NvicState::default());

    let mut cpu = CortexM::default();
    cpu.set_shared_vtor(vtor.clone());
    cpu.set_shared_vectactive(vectactive.clone());
    cpu.set_shared_shpr(shpr1.clone(), shpr2.clone(), shpr3.clone());
    cpu.set_shared_nvic_state(nvic_state.clone());

    bus.nvic = Some(nvic_state.clone());

    // Ensure SCB exists (VTOR relocation, ICSR.VECTACTIVE mirror, SHPR1/2/3).
    let mut scb = Scb::with_shared(SharedScbState {
        vtor,
        vectactive,
        shpr1,
        shpr2,
        shpr3,
    });
    // Walk-free plan batch B1: this install path replaces the placeholder dev
    // (or pushes directly) and so bypasses the `add_peripheral`/`push_peripheral`
    // attach chokes — attach the bus cycle clock here explicitly, flipping the
    // SCB's ICSR pend-drain onto the event scheduler (event-scheduler builds).
    crate::Peripheral::attach_cycle_clock(&mut scb, bus.cycle_clock.clone());
    if let Some(p) = bus
        .peripherals
        .iter_mut()
        .find(|p| p.name == "scb" || p.base == 0xE000_ED00)
    {
        p.name = "scb".to_string();
        p.base = 0xE000_ED00;
        // 0xC8 so the MPU block is served by the SCB/SCS model, not unmapped
        // space: TYPE/CTRL/RNR/RBAR/RASR at 0x90..0xA0 plus the ARMv8-M
        // (Cortex-M33) MAIR0/MAIR1 attribute registers at 0xC0/0xC4.
        p.size = 0xC8;
        p.irq = None;
        p.dev = Box::new(scb);
    } else {
        bus.peripherals.push(PeripheralEntry {
            name: "scb".to_string(),
            base: 0xE000_ED00,
            // 0xC8 to include the MPU block (0x90..0xA0) plus the ARMv8-M
            // MAIR0/MAIR1 at 0xC0/0xC4; see above.
            size: 0xC8,
            irq: None,
            dev: Box::new(scb),
            ticks_remaining: 0,
            clock_gate: None,
        });
    }

    // Ensure NVIC exists (shared pending/enabled state)
    let nvic = Nvic::new(nvic_state.clone());
    if let Some(p) = bus
        .peripherals
        .iter_mut()
        .find(|p| p.name == "nvic" || p.base == 0xE000_E100)
    {
        p.name = "nvic".to_string();
        p.base = 0xE000_E100;
        p.size = 0x400;
        p.irq = None;
        p.dev = Box::new(nvic);
    } else {
        bus.peripherals.push(PeripheralEntry {
            name: "nvic".to_string(),
            base: 0xE000_E100,
            size: 0x400,
            irq: None,
            dev: Box::new(nvic),
            ticks_remaining: 0,
            clock_gate: None,
        });
    }

    // Ensure DWT exists. Size 0x1000 covers the full CoreSight DWT register space,
    // including the CYCCNT enable bit at offset 0 and CYCCNT at offset 4, as well as
    // extended offsets accessed by some HAL dwt_init routines (e.g. offset 0xfc).
    // Attach the bus cycle clock so CYCCNT can be derived lazily (walk-free
    // plan Part 1). DWT is registered by directly manipulating `bus.peripherals`
    // (not `add_peripheral`), so the attach choke is replicated here — without
    // it the model stays on the legacy walk. The clone happens before the
    // `iter_mut` borrow below.
    let dwt_clock = bus.cycle_clock.clone();
    let mut dwt = Dwt::new();
    dwt.attach_cycle_clock(dwt_clock);
    if let Some(p) = bus
        .peripherals
        .iter_mut()
        .find(|p| p.name == "dwt" || p.base == 0xE000_1000)
    {
        p.name = "dwt".to_string();
        p.base = 0xE000_1000;
        p.size = 0x1000;
        p.irq = None;
        p.dev = Box::new(dwt);
    } else {
        bus.peripherals.push(PeripheralEntry {
            name: "dwt".to_string(),
            base: 0xE000_1000,
            size: 0x1000,
            irq: None,
            dev: Box::new(dwt),
            ticks_remaining: 0,
            clock_gate: None,
        });
    }

    bus.refresh_peripheral_index();

    (cpu, nvic_state)
}
