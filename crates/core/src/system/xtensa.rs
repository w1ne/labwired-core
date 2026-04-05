// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::SystemBus;
use crate::cpu::Xtensa;

pub fn configure_xtensa(_bus: &mut SystemBus) -> Xtensa {
    // For now, no specific peripherals (interrupt controller, etc.) are mandated
    // for the basic Xtensa simulation loop. Future iterations will add the
    // ESP32-S3 interrupt matrix here.
    Xtensa::new()
}
