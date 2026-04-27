// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::SystemBus;
use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::Cpu;

/// Stub. Phase D will replace with the full configure_xtensa_esp32s3
/// (registering all peripherals, returning Esp32s3Wiring).
pub fn configure_xtensa(bus: &mut SystemBus) -> XtensaLx7 {
    let mut cpu = XtensaLx7::new();
    cpu.reset(bus).expect("xtensa reset");
    cpu
}
