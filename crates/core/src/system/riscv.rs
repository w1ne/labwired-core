// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::SystemBus;
use crate::cpu::RiscV;

pub fn configure_riscv(bus: &mut SystemBus) -> RiscV {
    // The profile is resolved once, at bus build, from the chip descriptor
    // (`RiscVCoreProfile::for_chip`): `rv32e*` gets the 16-register E profile,
    // other generic `rv32*` gets the standard profile, an ESP32 part keeps the
    // C3's vendor CSR set, and a descriptor without `core:` keeps the
    // historical C3 default. Every caller — CLI,
    // browser, debugger, tests — shares this one resolution.
    let mut cpu = RiscV::new_for(bus.riscv_profile);
    attach_qingke_pfic(bus, &mut cpu);
    cpu
}

/// QingKe V2 interrupts: share a `qingke_pfic` peripheral's state with the
/// core, which arbitrates and takes them, and with every QingKe SysTick,
/// whose STIE/SWIE requests are PFIC level sources. A chip without a PFIC
/// keeps the earlier behaviour: no interrupt is delivered.
fn attach_qingke_pfic(bus: &mut SystemBus, cpu: &mut RiscV) {
    let Some(state) = bus
        .peripherals
        .iter()
        .find_map(|p| p.dev.qingke_pfic_state())
    else {
        return;
    };
    cpu.attach_qingke_pfic(state.clone());
    bus.qingke_pfic = Some(state.clone());
    for p in bus.peripherals.iter_mut() {
        p.dev.attach_qingke_pfic(state.clone());
    }
}
