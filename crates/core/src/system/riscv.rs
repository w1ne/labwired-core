// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::SystemBus;
use crate::cpu::RiscV;

pub fn configure_riscv(bus: &mut SystemBus) -> RiscV {
    // The profile is resolved once, at bus build, from the chip descriptor
    // (`RiscVCoreProfile::for_chip`): a generic `rv32*` core runs the standard
    // profile, an ESP32 part keeps the C3's vendor CSR set, and a descriptor
    // without a `core:` keeps the historical C3 default. Every caller — CLI,
    // browser, debugger, tests — shares this one resolution.
    let mut cpu = RiscV::new_for(bus.riscv_profile);
    // A WCH QingKe chip declares its PFIC (`wch_pfic`). The core takes
    // interrupts from that state, and `bus.nvic` points at the same state so
    // the bus's existing pend and level-reconcile paths drive it: a
    // peripheral's `irq:` line pends the PFIC bit of that number.
    let pfic = bus.peripherals.iter().find_map(|p| p.dev.wch_pfic_state());
    if let Some(state) = pfic {
        bus.nvic = Some(state.clone());
        cpu.set_pfic(state);
    }
    cpu
}
