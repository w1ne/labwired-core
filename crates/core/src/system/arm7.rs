// SPDX-License-Identifier: MIT

//! The bounded ARM7TDMI execution path for the imported LPC2294 profile.
//! Device claims remain limited to the peripherals installed by its chip map.
use crate::bus::SystemBus;
use crate::cpu::{ArmModel, ArmSystem};
use crate::system::node::parse_elf_image;
use crate::{Arch, Cpu, Machine};
use labwired_config::ChipDescriptor;

/// Attach the native ARM7TDMI model and the LPC2294 PL190 IRQ/FIQ output.
pub fn attach(bus: &mut SystemBus, chip: &ChipDescriptor) -> anyhow::Result<ArmSystem> {
    let core = chip
        .core
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase());
    if chip.name != "lpc2294" || !matches!(core.as_deref(), Some("arm7tdmi" | "arm7tdmi-s")) {
        anyhow::bail!(
            "chip '{}': the ARM7TDMI native profile is limited to the imported LPC2294 descriptor",
            chip.name
        );
    }

    let vic_index = bus
        .find_peripheral_index_by_name("vic")
        .ok_or_else(|| anyhow::anyhow!("LPC2294 profile is missing its PL190 VIC"))?;
    if bus.peripherals[vic_index]
        .dev
        .arm_irq_fiq_levels()
        .is_none()
    {
        anyhow::bail!("LPC2294 'vic' peripheral does not expose ARM IRQ/FIQ outputs");
    }
    bus.irq_fabric.lpc2294_vic = Some(vic_index);

    let mut cpu = ArmSystem::with_model(ArmModel::Arm7Tdmi)?;
    for memory in std::iter::once(&bus.flash)
        .chain(std::iter::once(&bus.ram))
        .chain(bus.extra_mem.iter())
    {
        if !memory.data.is_empty() {
            cpu.executable_range(memory.base_addr, memory.data.len() as u64)?;
        }
    }
    Ok(cpu)
}

/// Load a bare-metal ARM ELF and start at its entry point.
pub fn load_elf(
    mut bus: SystemBus,
    chip: &ChipDescriptor,
    elf: &[u8],
) -> anyhow::Result<Machine<Box<dyn Cpu>>> {
    let image = parse_elf_image(elf)?;
    if image.arch != Arch::Arm || image.entry_point > u64::from(u32::MAX) {
        anyhow::bail!("ARM7TDMI requires a 32-bit ARM ELF");
    }
    let cpu = attach(&mut bus, chip)?;
    let mut machine = Machine::new(Box::new(cpu) as Box<dyn Cpu>, bus);
    machine.load_firmware(&image)?;
    Ok(machine)
}
