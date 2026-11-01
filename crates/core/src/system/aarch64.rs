// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! AArch64 machines: the [`crate::cpu::Aarch64`] PE (with its GICv3 and
//! generic timer) over a bus that owns RAM and the chip's peripherals.

use crate::bus::SystemBus;
use crate::system::node::parse_elf_image;
use crate::{Arch, Cpu, Machine};

/// Load an AArch64 ELF into `bus` and start the PE at the ELF entry, at EL3
/// with the MMU off: the A53 reset state, with the image standing where the
/// reset vector (RVBAR) points.
pub fn load_elf(
    bus: SystemBus,
    chip: &labwired_config::ChipDescriptor,
    elf: &[u8],
) -> anyhow::Result<Machine<Box<dyn Cpu>>> {
    let image = parse_elf_image(elf)?;
    if image.arch != Arch::Aarch64 {
        anyhow::bail!(
            "firmware architecture {:?} is incompatible with an AArch64 chip",
            image.arch
        );
    }
    let cpu = crate::cpu::Aarch64::for_chip(chip);
    let mut machine = Machine::new(Box::new(cpu) as Box<dyn Cpu>, bus);
    machine.load_firmware(&image)?;
    machine.cpu.set_pc(image.entry_point as u32);
    Ok(machine)
}
