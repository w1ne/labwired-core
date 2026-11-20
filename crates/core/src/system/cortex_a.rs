// SPDX-License-Identifier: MIT

use crate::bus::SystemBus;
use crate::cpu::CortexA7;
use crate::system::arch_policy::CortexACore;
use crate::system::node::parse_elf_image;
use crate::{Arch, Cpu, Machine};

/// Load a bare-metal ARM ELF with the embedded native core. This installs no
/// Cortex-M NVIC/SysTick/SCB and makes no claim of Linux or RV1103 support.
pub fn load_elf(bus: SystemBus, elf: &[u8]) -> anyhow::Result<Machine<Box<dyn Cpu>>> {
    load_elf_for_core(bus, elf, CortexACore::A7)
}

/// [`load_elf`] on the chip's declared Cortex-A core.
pub fn load_elf_for_core(
    bus: SystemBus,
    elf: &[u8],
    core: CortexACore,
) -> anyhow::Result<Machine<Box<dyn Cpu>>> {
    let image = parse_elf_image(elf)?;
    if image.arch != Arch::Arm || image.entry_point > u64::from(u32::MAX) {
        anyhow::bail!("{} requires a 32-bit ARM ELF", core.name());
    }
    let cpu: Box<dyn Cpu> = Box::new(CortexA7::with_core(core)?);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image)?;
    Ok(machine)
}
