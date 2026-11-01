// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! MIPS32 (PIC32MZ) machines: the [`crate::cpu::Mips32`] core (with its
//! EVIC) over a bus that owns data RAM, program flash, boot flash and the
//! chip's peripherals at their **physical** addresses.

use crate::bus::SystemBus;
use crate::system::node::parse_elf_image;
use crate::{Arch, Cpu, Machine};

/// The fixed KSEG0/KSEG1 windows map virtual `va` to physical `va & 0x1FFFFFFF`.
pub const fn kseg_to_physical(va: u64) -> u64 {
    va & 0x1FFF_FFFF
}

/// Load a MIPS ELF into `bus` and start the core at the reset vector
/// (0xBFC00000), as the silicon does.
///
/// A MIPS ELF names load addresses in the KSEG0/KSEG1 windows (program flash
/// at 0x9D000000, boot flash at 0xBFC00000, initialised data's load image in
/// flash); the bus is physical, so [`parse_elf_image`] puts each segment at
/// its physical address. The image is refused when DEVCFG0.BOOTISA selects
/// microMIPS.
pub fn load_elf(bus: SystemBus, elf: &[u8]) -> anyhow::Result<Machine<Box<dyn Cpu>>> {
    let image = parse_elf_image(elf)?;
    if image.arch != Arch::Mips {
        anyhow::bail!(
            "firmware architecture {:?} is incompatible with a MIPS chip",
            image.arch
        );
    }
    let mut machine = Machine::new(Box::new(crate::cpu::Mips32::new()) as Box<dyn Cpu>, bus);
    machine.load_firmware(&image)?;
    machine
        .cpu
        .reset(&mut machine.bus)
        .map_err(|e| anyhow::anyhow!("MIPS reset: {e}"))?;
    machine.cpu.set_pc(crate::cpu::mips::RESET_VECTOR);
    Ok(machine)
}
