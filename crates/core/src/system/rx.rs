// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Renesas RX machines: the [`crate::cpu::Rx`] core (with its ICU) over a
//! bus that owns RAM, code flash and the chip's peripherals.

use crate::bus::SystemBus;
use crate::system::node::parse_elf_image;
use crate::{Arch, Bus, Cpu, Machine};

/// The fixed reset vector every RX core fetches its first PC from.
const RESET_VECTOR: u64 = 0xFFFF_FFFC;

/// Load an RX ELF into `bus` and start the core at the reset vector, as the
/// silicon does: the image's `.fvectors` section holds the reset handler's
/// address at 0xFFFFFFFC.
pub fn load_elf(bus: SystemBus, elf: &[u8]) -> anyhow::Result<Machine<Box<dyn Cpu>>> {
    let image = parse_elf_image(elf)?;
    if image.arch != Arch::Rx {
        anyhow::bail!(
            "firmware architecture {:?} is incompatible with an RX chip",
            image.arch
        );
    }
    let mut machine = Machine::new(Box::new(crate::cpu::Rx::new()) as Box<dyn Cpu>, bus);
    machine.load_firmware(&image)?;
    let pc = reset_pc(&machine.bus)?;
    machine.cpu.set_pc(pc);
    Ok(machine)
}

/// The reset handler address the loaded image put in the fixed vector.
pub fn reset_pc(bus: &dyn Bus) -> anyhow::Result<u32> {
    bus.read_u32(RESET_VECTOR)
        .map_err(|e| anyhow::anyhow!("RX reset vector at {RESET_VECTOR:#x}: {e:?}"))
}
