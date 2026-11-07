// SPDX-License-Identifier: MIT

//! Native Cortex-R machines (feature `qemu-arm-system`): the pinned tlib
//! `cortex-r5f` (arm32) or `cortex-r52` (arm64, AArch32) over a bus that owns
//! memory. The R5F also gets the descriptor-owned RPU platform; only the Zynq
//! UltraScale+ XCK26 RPU is modelled.
use crate::bus::SystemBus;
use crate::cpu::{ArmModel, CortexA7System};
use crate::system::node::parse_elf_image;
use crate::{Arch, Cpu, Machine};
use labwired_config::ChipDescriptor;

/// Install the chip's platform on `bus` and return a reset Cortex-R that may
/// fetch from the bus's RAM and declared memory regions.
///
/// Cortex-R5F needs the descriptor-owned RPU platform, modelled only for the
/// Zynq UltraScale+ XCK26. Cortex-R52 attaches to the descriptor's own memory
/// and peripherals; no GIC, generic timer or TCM is installed for it.
pub fn attach(bus: &mut SystemBus, chip: &ChipDescriptor) -> anyhow::Result<CortexA7System> {
    let core = chip.core.as_deref().map(str::trim).unwrap_or_default();
    let model = if core.eq_ignore_ascii_case("cortex-r52") {
        ArmModel::CortexR52
    } else {
        if chip.name != "xck26-rpu" {
            anyhow::bail!(
                "chip '{}': no native Cortex-R5F platform map; only xck26-rpu (Zynq UltraScale+ RPU) is modelled",
                chip.name
            );
        }
        bus.configure_armv7_zynqmp_rpu()?;
        ArmModel::CortexR5F
    };
    let mut cpu = CortexA7System::with_model(model)?;
    for mem in std::iter::once(&bus.ram).chain(bus.extra_mem.iter()) {
        if !mem.data.is_empty() {
            cpu.executable_range(mem.base_addr, mem.data.len() as u64)?;
        }
    }
    Ok(cpu)
}

/// Load an initialized-memory ARM ELF (as the FSBL would place it) and enter
/// it in the reset mode (SVC on Cortex-R5F, Hyp on Cortex-R52; IRQ/FIQ masked). `cpu` comes from [`attach`] on
/// this `bus`.
pub fn load(
    bus: SystemBus,
    cpu: CortexA7System,
    elf: &[u8],
) -> anyhow::Result<Machine<Box<dyn Cpu>>> {
    let image = parse_elf_image(elf)?;
    if image.arch != Arch::Arm || image.entry_point > u64::from(u32::MAX) {
        anyhow::bail!("Cortex-R requires a 32-bit ARM ELF");
    }
    let mut machine = Machine::new(Box::new(cpu) as Box<dyn Cpu>, bus);
    machine.load_firmware(&image)?;
    Ok(machine)
}

/// [`attach`] then [`load`].
pub fn load_elf(
    mut bus: SystemBus,
    chip: &ChipDescriptor,
    elf: &[u8],
) -> anyhow::Result<Machine<Box<dyn Cpu>>> {
    let cpu = attach(&mut bus, chip)?;
    load(bus, cpu, elf)
}
