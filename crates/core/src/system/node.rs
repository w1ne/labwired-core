// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One place that turns (chip, system, firmware) into a runnable machine.
//!
//! A [`crate::world::World`] node and a single-chip run are the same thing —
//! a chip, its system manifest, and an image to execute. Before this module the
//! two were built by different code, and only the Cortex-M half had a
//! multi-node path, so a world of ESP32 or RISC-V nodes was rejected outright
//! even though the engine can run every one of those chips on its own.
//!
//! Everything here is construction only: no stepping, no run loop, no policy.
//! That keeps it callable from the CLI, the hosted runner, and the browser
//! without dragging any of their orchestration along.

use crate::system::arch_policy::{elf_arch, machine_family, MachineFamily};
use crate::world::MachineTrait;
use crate::Machine;
use anyhow::Context;
use labwired_config::{ChipDescriptor, SystemManifest};

/// The image a node executes.
///
/// The distinction is not cosmetic: an ELF is loaded into memory and the CPU
/// starts at its entry point, whereas a flash image is placed *behind the flash
/// controller* so the chip's genuine mask ROM finds and loads it exactly as
/// silicon does. Modelling both keeps ESP32 nodes on the faithful boot path
/// instead of a fast-boot shortcut.
pub enum NodeFirmware {
    /// Raw ELF bytes. Parsed per-architecture — the Xtensa boot path needs the
    /// original bytes, not a pre-digested image.
    Elf(Vec<u8>),
    /// A flash image (`bootloader@0x0` + partition table + app) for ROM boot.
    FlashImage(Vec<u8>),
}

impl NodeFirmware {
    /// Classify bytes read from disk. ELF magic is the discriminator, so a
    /// node declares `firmware: <path>` and the right boot path follows from
    /// the file itself — no second manifest field to keep in sync.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        if bytes.starts_with(&[0x7f, b'E', b'L', b'F']) {
            NodeFirmware::Elf(bytes)
        } else {
            NodeFirmware::FlashImage(bytes)
        }
    }

    /// Read and classify a firmware file.
    pub fn from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("read firmware {path:?}"))?;
        Ok(Self::from_bytes(bytes))
    }
}

/// Build one runnable machine from a chip descriptor, its system manifest, and
/// a firmware image.
///
/// `id` is used only to attribute errors to the node that caused them; a
/// single-chip caller can pass any label.
///
/// The returned machine has been reset and is ready to step. Wiring that
/// belongs to the *environment* rather than the chip — cross-links, capture
/// sinks, stdout prefixes — is deliberately left to the caller, so a node built
/// here behaves identically whether it runs alone or in a world.
pub fn build_node(
    id: &str,
    chip: &ChipDescriptor,
    system: &SystemManifest,
    firmware: NodeFirmware,
) -> anyhow::Result<Box<dyn MachineTrait>> {
    build_node_with_plugins(id, chip, system, firmware, &[])
}

/// [`build_node`] with out-of-tree chip plugins: each peripheral type is
/// offered to `plugins` before the in-tree factories when the node's bus is
/// built.
pub fn build_node_with_plugins(
    id: &str,
    chip: &ChipDescriptor,
    system: &SystemManifest,
    firmware: NodeFirmware,
    plugins: &[&dyn crate::plugin::ChipPlugin],
) -> anyhow::Result<Box<dyn MachineTrait>> {
    match machine_family(chip).with_context(|| format!("node '{id}'"))? {
        MachineFamily::CortexM => build_cortex_m_node(id, chip, system, firmware, plugins),
        MachineFamily::RiscV => build_riscv_node(id, chip, system, firmware, plugins),
        MachineFamily::Xtensa => build_xtensa_node(id, chip, system, firmware),
    }
}

fn is_cortex_m(chip: &ChipDescriptor) -> bool {
    chip.core
        .as_deref()
        .is_some_and(|core| core.trim().to_ascii_lowercase().starts_with("cortex-m"))
}

fn build_cortex_m_node(
    id: &str,
    chip: &ChipDescriptor,
    system: &SystemManifest,
    firmware: NodeFirmware,
    plugins: &[&dyn crate::plugin::ChipPlugin],
) -> anyhow::Result<Box<dyn MachineTrait>> {
    if !is_cortex_m(chip) {
        anyhow::bail!(
            "node '{id}': chip '{}' declares arch arm but core {:?}; only Cortex-M cores are modelled",
            chip.name,
            chip.core
        );
    }
    let NodeFirmware::Elf(bytes) = firmware else {
        anyhow::bail!(
            "node '{id}': chip '{}' boots from an ELF, but the firmware is not an ELF file",
            chip.name
        );
    };

    let image =
        parse_elf_image(&bytes).with_context(|| format!("node '{id}': parse firmware ELF"))?;
    validate_cortex_m_firmware(id, chip, &image)?;

    let mut bus = crate::bus::SystemBus::from_config_with_plugins(chip, system, plugins)
        .with_context(|| format!("node '{id}': build bus"))?;
    let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine
        .load_firmware(&image)
        .map_err(|e| anyhow::anyhow!("node '{id}': load firmware: {e:?}"))?;
    machine
        .reset()
        .map_err(|e| anyhow::anyhow!("node '{id}': reset: {e:?}"))?;
    Ok(Box::new(machine))
}

fn build_riscv_node(
    id: &str,
    chip: &ChipDescriptor,
    system: &SystemManifest,
    firmware: NodeFirmware,
    plugins: &[&dyn crate::plugin::ChipPlugin],
) -> anyhow::Result<Box<dyn MachineTrait>> {
    let mut bus = crate::bus::SystemBus::from_config_with_plugins(chip, system, plugins)
        .with_context(|| format!("node '{id}': build bus"))?;

    match firmware {
        // Faithful ROM boot: the mask ROM reads the image through the flash
        // controller and jumps to the app itself, so nothing is pre-loaded.
        NodeFirmware::FlashImage(flash_bytes) => {
            use crate::boot::esp32c3_rom as c3rom;
            // ROM boot is per-silicon: the image layout, flash controller, and
            // mask ROM all belong to one specific chip, so this cannot be
            // applied to RISC-V generally. The ESP32-C3 is the RISC-V chip whose
            // mask ROM is modelled (see `boot::esp32c3_rom`); the ESP32-S3 has
            // its own in `boot::esp32s3_rom`, reached through the Xtensa arm.
            if !chip.name.to_ascii_lowercase().contains("esp32c3") {
                anyhow::bail!(
                    "node '{id}': flash-image ROM boot is modelled per chip, and chip '{}' is not \
                     one of them; supply an ELF instead",
                    chip.name
                );
            }
            let images = c3rom::provision_rom_images().with_context(|| {
                format!(
                    "node '{id}': chip '{}' needs the real ESP32-C3 boot ROM to run a flash image; \
                     install an ESP toolchain (esp32c3_rev3_rom.elf) or set \
                     LABWIRED_ESP32C3_ROM / LABWIRED_ESP32C3_ROM_DATA",
                    chip.name
                )
            })?;
            if !c3rom::inject_rom_regions(&mut bus, &images) {
                anyhow::bail!(
                    "node '{id}': chip '{}' has no instruction-ROM window for the boot ROM",
                    chip.name
                );
            }
            let machine = c3rom::build_rom_boot_machine(
                bus,
                flash_bytes,
                c3rom::RomBootOpts::default(),
                |cpu| cpu,
            );
            Ok(Box::new(machine))
        }
        NodeFirmware::Elf(bytes) => {
            let image = parse_elf_image(&bytes)
                .with_context(|| format!("node '{id}': parse firmware ELF"))?;
            use crate::Cpu as _;
            let cpu = crate::system::riscv::configure_riscv(&mut bus);
            let mut machine = Machine::new(cpu, bus);
            machine
                .load_firmware(&image)
                .map_err(|e| anyhow::anyhow!("node '{id}': load firmware: {e:?}"))?;
            // Fast boot skips the ROM/2nd-stage bootloader that would normally
            // set the stack pointer, so seed it at the top of RAM (16-byte
            // aligned, RISC-V ABI) or the first prologue store faults.
            let ram_size = labwired_config::parse_size(&chip.ram.size).unwrap_or(0);
            let sp_top = (chip.ram.base + ram_size) as u32;
            machine.cpu.set_sp(sp_top & !0xF);
            Ok(Box::new(machine))
        }
    }
}

fn build_xtensa_node(
    id: &str,
    chip: &ChipDescriptor,
    system: &SystemManifest,
    firmware: NodeFirmware,
) -> anyhow::Result<Box<dyn MachineTrait>> {
    let core = chip.core.as_deref().unwrap_or("").to_ascii_lowercase();
    if core.contains("lx7") {
        return build_esp32s3_node(id, chip, system, firmware);
    }

    let NodeFirmware::Elf(bytes) = firmware else {
        anyhow::bail!(
            "node '{id}': chip '{}' boots from an ELF, but the firmware is not an ELF file",
            chip.name
        );
    };
    let image =
        parse_elf_image(&bytes).with_context(|| format!("node '{id}': parse firmware ELF"))?;

    // Classic ESP32 (LX6): the Rust peripheral bank is authoritative, and the
    // second core starts halted until PRO releases it — the same construction
    // the single-chip ESP32 path uses.
    let mut bus = crate::bus::SystemBus::new();
    let pro_cpu = crate::system::xtensa::configure_xtensa_esp32(&mut bus);
    crate::system::xtensa::attach_esp32_external_devices(&mut bus, system)
        .with_context(|| format!("node '{id}': attach external devices"))?;
    // Debugger register names still come from the chip YAML even though the
    // peripheral bank is programmatic — see `SystemBus::attach_debug_schemas`.
    bus.attach_debug_schemas(chip, system);
    bus.refresh_peripheral_index();
    let app_cpu = crate::cpu::xtensa_lx7::XtensaLx7::new_app_cpu();

    let mut machine = Machine::new(pro_cpu, bus).with_secondary_cpu(app_cpu);
    machine
        .load_firmware(&image)
        .map_err(|e| anyhow::anyhow!("node '{id}': load firmware: {e:?}"))?;
    machine
        .reset()
        .map_err(|e| anyhow::anyhow!("node '{id}': reset: {e:?}"))?;
    Ok(Box::new(machine))
}

/// Build an ESP32-S3 (Xtensa LX7) node.
///
/// Both boot paths are real: a flash image runs the genuine mask ROM from the
/// reset vector, and an ELF fast-boots into the app. The flash image is passed
/// to `configure_xtensa_esp32s3` rather than read from `LABWIRED_ESP32S3_FLASH`,
/// which is what lets two S3 nodes in one world run *different* firmware.
///
/// Known limitation on the ELF path: the single-chip runner additionally
/// pre-paints the ESP-IDF dual-core handshake flags (`s_cpu_inited` &c.) by
/// looking up firmware symbols. That is a thunk over the boot sequence, it
/// needs the `loader` crate (which depends on core, so core cannot use it), and
/// it is superseded by the chip's SMP model — so it is deliberately not
/// reproduced here. An ESP-IDF ELF node will therefore wait at that handshake;
/// prefer the flash-image path, which boots the real ROM and does not need it.
fn build_esp32s3_node(
    id: &str,
    chip: &ChipDescriptor,
    system: &SystemManifest,
    firmware: NodeFirmware,
) -> anyhow::Result<Box<dyn MachineTrait>> {
    use crate::cpu::xtensa_lx7::XtensaLx7;
    use crate::system::xtensa::{configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts};

    let rom_boot = matches!(firmware, NodeFirmware::FlashImage(_));
    let flash_image = match &firmware {
        NodeFirmware::FlashImage(bytes) => Some(bytes.clone()),
        NodeFirmware::Elf(_) => None,
    };

    let mut bus = crate::bus::SystemBus::new();
    let opts = Esp32s3Opts {
        real_reset_boot: rom_boot,
        flash_image,
        ..Esp32s3Opts::default()
    };
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
    // Debugger register names still come from the chip YAML even though the
    // peripheral bank is programmatic — see `SystemBus::attach_debug_schemas`.
    bus.attach_debug_schemas(chip, system);
    let boot_mode = wiring.boot_mode;
    let mut cpu = wiring.cpu;

    match firmware {
        NodeFirmware::FlashImage(_) => {
            if boot_mode != Esp32s3BootMode::Faithful {
                anyhow::bail!(
                    "node '{id}': chip '{}' needs the real ESP32-S3 boot ROM to run a flash image, \
                     but none was found; install an ESP toolchain (PlatformIO/ESP-IDF) or set \
                     LABWIRED_ESP32S3_ROM_ELF (or pin LABWIRED_ESP32S3_ROM/_DROM)",
                    chip.name
                );
            }
            // The ROM and firmware install the window vectors and build a real
            // stack save chain, so use the genuine per-access overflow / RETW
            // underflow path rather than a simulated shadow stack.
            cpu.faithful_windows = true;
            let mut app_cpu = XtensaLx7::new_app_cpu();
            app_cpu.faithful_windows = true;
            Ok(Box::new(Machine::new(cpu, bus).with_secondary_cpu(app_cpu)))
        }
        NodeFirmware::Elf(bytes) => {
            use crate::boot::esp32s3::{fast_boot, BootOpts};
            fast_boot(
                &bytes,
                &mut bus,
                &mut cpu,
                &BootOpts {
                    stack_top_fallback: 0x3FCD_FFF0,
                    icache_backing: Some(wiring.icache_backing),
                    dcache_backing: Some(wiring.dcache_backing),
                    factory_flash_base: None,
                },
            )
            .map_err(|e| anyhow::anyhow!("node '{id}': fast boot: {e}"))?;
            Ok(Box::new(
                Machine::new(cpu, bus).with_secondary_cpu(XtensaLx7::new_app_cpu()),
            ))
        }
    }
}

/// Parse ELF bytes into a [`crate::memory::ProgramImage`].
///
/// Core cannot depend on the `loader` crate (that crate depends on core), so
/// this is the in-core equivalent. PT_LOAD segments are placed at their load
/// address (`p_paddr`), which is what makes the `.data`-LMA-in-flash convention
/// work on Cortex-M.
pub fn parse_elf_image(bytes: &[u8]) -> anyhow::Result<crate::memory::ProgramImage> {
    use goblin::elf::program_header::PT_LOAD;
    use goblin::elf::Elf;

    let elf = Elf::parse(bytes).context("parse ELF")?;
    let machine = elf.header.e_machine;
    let arch = elf_arch(machine)
        .ok_or_else(|| anyhow::anyhow!("unsupported ELF machine type {machine}"))?;
    let mut image = crate::memory::ProgramImage::new(elf.entry, arch);
    for ph in &elf.program_headers {
        if ph.p_type != PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        let off = ph.p_offset as usize;
        let n = ph.p_filesz as usize;
        if off + n <= bytes.len() {
            image.add_segment(ph.p_paddr, bytes[off..off + n].to_vec());
        }
    }
    Ok(image)
}

/// A Cortex-M image must carry a usable reset vector, or the machine boots to
/// a garbage PC and fails far from the real cause.
fn validate_cortex_m_firmware(
    node_id: &str,
    chip: &ChipDescriptor,
    image: &crate::memory::ProgramImage,
) -> anyhow::Result<()> {
    if image.arch != crate::Arch::Arm {
        anyhow::bail!(
            "node '{node_id}': firmware architecture {:?} is incompatible with Cortex-M chip '{}'",
            image.arch,
            chip.name
        );
    }

    let flash_size = labwired_config::parse_size(&chip.flash.size).with_context(|| {
        format!(
            "node '{node_id}': invalid flash size for chip '{}'",
            chip.name
        )
    })?;
    let ram_size = labwired_config::parse_size(&chip.ram.size).with_context(|| {
        format!(
            "node '{node_id}': invalid RAM size for chip '{}'",
            chip.name
        )
    })?;
    let vector_base = chip
        .flash
        .base
        .checked_add(chip.reset_vector_offset)
        .context("Cortex-M reset vector address overflow")?;
    let stack_pointer = image_u32_at(image, vector_base);
    let reset_handler = image_u32_at(image, vector_base.saturating_add(4));
    let reset_target = reset_handler.map(|handler| u64::from(handler & !1));
    let valid_stack = stack_pointer.is_some_and(|stack| {
        let stack = u64::from(stack);
        stack >= chip.ram.base && stack <= chip.ram.base.saturating_add(ram_size)
    });
    let valid_reset = reset_handler.is_some_and(|handler| handler & 1 == 1)
        && reset_target.is_some_and(|target| {
            target >= chip.flash.base && target < chip.flash.base.saturating_add(flash_size)
        });
    if !valid_stack || !valid_reset {
        anyhow::bail!(
            "node '{node_id}': firmware does not contain a valid Cortex-M Thumb reset vector for chip '{}'",
            chip.name
        );
    }
    Ok(())
}

fn image_u32_at(image: &crate::memory::ProgramImage, address: u64) -> Option<u32> {
    let mut bytes = [0_u8; 4];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let byte_address = address.checked_add(index as u64)?;
        *byte = image.segments.iter().find_map(|segment| {
            let offset = usize::try_from(byte_address.checked_sub(segment.start_addr)?).ok()?;
            segment.data.get(offset).copied()
        })?;
    }
    Some(u32::from_le_bytes(bytes))
}
