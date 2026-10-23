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
    RiscV::new_for(bus.riscv_profile)
}
