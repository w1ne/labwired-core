// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Texas Instruments MSP430 machines: the [`crate::cpu::Msp430`] core over a
//! bus that owns RAM, flash and the chip's modules, with the interrupt
//! request lines shared between them.

use crate::bus::SystemBus;
use crate::cpu::Msp430;
use crate::peripherals::msp430::Msp430Irq;
use crate::system::node::parse_elf_image;
use crate::{Arch, Cpu, Machine};
use labwired_config::ChipDescriptor;

/// The core `chip` names (the 16-bit CPU when it is unknown), sharing one set
/// of interrupt sources with every module on `bus`.
pub fn cpu_for(bus: &mut SystemBus, chip: Option<&ChipDescriptor>) -> Msp430 {
    let irq = Msp430Irq::new();
    for p in &mut bus.peripherals {
        p.dev.attach_msp430_irq(&irq);
    }
    let mut cpu = chip.map_or_else(Msp430::new, Msp430::for_chip);
    cpu.attach_irq(irq);
    cpu
}

/// Load an MSP430 ELF into `bus` and start the core at the reset vector
/// (0xFFFE), as the silicon does.
pub fn load_elf(
    mut bus: SystemBus,
    chip: &ChipDescriptor,
    elf: &[u8],
) -> anyhow::Result<Machine<Box<dyn Cpu>>> {
    let image = parse_elf_image(elf)?;
    if image.arch != Arch::Msp430 {
        anyhow::bail!(
            "firmware architecture {:?} is incompatible with an MSP430 chip",
            image.arch
        );
    }
    let cpu = cpu_for(&mut bus, Some(chip));
    let mut machine = Machine::new(Box::new(cpu) as Box<dyn Cpu>, bus);
    machine.load_firmware(&image)?;
    machine
        .cpu
        .reset(&mut machine.bus)
        .map_err(|e| anyhow::anyhow!("MSP430 reset vector: {e}"))?;
    Ok(machine)
}
