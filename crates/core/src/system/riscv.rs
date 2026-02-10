// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::SystemBus;
use crate::cpu::RiscV;

pub fn configure_riscv(_bus: &mut SystemBus) -> RiscV {
    // For now, no specific peripherals (PLIC/CLINT) are mandated for the basic RV32I simulation loop.
    // Future iterations will add them here.
    RiscV::new()
}
