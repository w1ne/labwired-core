// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::{PeripheralEntry, SystemBus};
use crate::cpu::CortexM;
use crate::peripherals::nvic::{Nvic, NvicState};
use crate::peripherals::scb::Scb;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;

pub fn configure_cortex_m(bus: &mut SystemBus) -> (CortexM, Arc<NvicState>) {
    let vtor = Arc::new(AtomicU32::new(0));
    let nvic_state = Arc::new(NvicState::default());

    let mut cpu = CortexM::default();
    cpu.set_shared_vtor(vtor.clone());

    bus.nvic = Some(nvic_state.clone());

    // Ensure SCB exists (VTOR relocation)
    let scb = Scb::new(vtor);
    if let Some(p) = bus
        .peripherals
        .iter_mut()
        .find(|p| p.name == "scb" || p.base == 0xE000_ED00)
    {
        p.name = "scb".to_string();
        p.base = 0xE000_ED00;
        p.size = 0x40;
        p.irq = None;
        p.dev = Box::new(scb);
    } else {
        bus.peripherals.push(PeripheralEntry {
            name: "scb".to_string(),
            base: 0xE000_ED00,
            size: 0x40,
            irq: None,
            dev: Box::new(scb),
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
        });
    }

    (cpu, nvic_state)
}
