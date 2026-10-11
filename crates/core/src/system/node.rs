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
    build_node_in_fab(id, chip, system, firmware, plugins, None)
}

/// [`build_node_with_plugins`] whose dice take their factory identity (the
/// eFuse MAC, hence the BLE address) from `fab` instead of the process-wide
/// allocator. A world passes its own, so the same world built twice in one
/// process gives its nodes the same addresses — a BLE transcript then
/// replays byte for byte.
pub fn build_node_in_fab(
    id: &str,
    chip: &ChipDescriptor,
    system: &SystemManifest,
    firmware: NodeFirmware,
    plugins: &[&dyn crate::plugin::ChipPlugin],
    fab: Option<&crate::system::efuse::FactoryMacAllocator>,
) -> anyhow::Result<Box<dyn MachineTrait>> {
    build_node_with_options(
        id,
        chip,
        system,
        firmware,
        &NodeBuildOptions {
            plugins,
            fab,
            ..NodeBuildOptions::default()
        },
    )
}

/// A node's boot profile: a named, opt-in departure from the faithful boot,
/// spelled the same as a test script's `inputs.profile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeProfile {
    /// The classic-ESP32 Arduino fast boot: the ELF's entry is entered
    /// directly and the shared `install_arduino_esp32_profile` seeds the boot
    /// state the skipped ROM would have left and installs its flash thunks —
    /// the path `labwired test` takes for `profile: arduino-esp32` and the
    /// debugger takes for an Arduino sketch. An Arduino-ESP32 sketch ELF does
    /// not reach `setup()` on a classic ESP32 without it.
    ArduinoEsp32,
}

impl NodeProfile {
    /// Parse a manifest's `profile:` string.
    pub fn parse(name: &str) -> anyhow::Result<Self> {
        if name == labwired_config::NODE_PROFILE_ARDUINO_ESP32 {
            Ok(Self::ArduinoEsp32)
        } else {
            anyhow::bail!(
                "unknown node profile '{name}'; the only node profile is '{}'",
                labwired_config::NODE_PROFILE_ARDUINO_ESP32
            )
        }
    }
}

/// Everything a node build takes beyond (chip, system, firmware).
#[derive(Default)]
pub struct NodeBuildOptions<'a> {
    /// Out-of-tree chip plugins, offered each peripheral type first.
    pub plugins: &'a [&'a dyn crate::plugin::ChipPlugin],
    /// The world's eFuse fab; `None` takes the process-wide allocator.
    pub fab: Option<&'a crate::system::efuse::FactoryMacAllocator>,
    /// Named binary blobs, under the names the single-chip engine takes them
    /// (`crate::system::builder::BlobMap`): `esp32c3_irom` / `esp32c3_drom` and
    /// `esp32s3_irom` / `esp32s3_drom` for the mask ROMs. A node whose blobs
    /// carry no ROM falls back to the provisioned one (a registered image, env
    /// pins, the installed toolchain or — native only — the vendored copy), so
    /// a hosted world needs none; the browser has no filesystem and passes
    /// them here.
    pub blobs: Option<&'a crate::system::builder::BlobMap>,
    /// Opt-in boot profile; `None` is the faithful path.
    pub profile: Option<NodeProfile>,
}

/// [`build_node`] with every per-node input spelled out.
pub fn build_node_with_options(
    id: &str,
    chip: &ChipDescriptor,
    system: &SystemManifest,
    firmware: NodeFirmware,
    opts: &NodeBuildOptions<'_>,
) -> anyhow::Result<Box<dyn MachineTrait>> {
    let no_blobs = crate::system::builder::BlobMap::new();
    let blobs = opts.blobs.unwrap_or(&no_blobs);
    let family = machine_family(chip).with_context(|| format!("node '{id}'"))?;
    if let Some(profile) = opts.profile {
        check_profile(id, chip, family, profile)?;
    }
    match family {
        MachineFamily::CortexM => build_cortex_m_node(id, chip, system, firmware, opts.plugins),
        MachineFamily::RiscV => {
            build_riscv_node(id, chip, system, firmware, opts.plugins, opts.fab, blobs)
        }
        MachineFamily::Xtensa => build_xtensa_node(id, chip, system, firmware, blobs, opts.profile),
        MachineFamily::Avr => build_avr_node(id, chip, system, firmware, opts.plugins),
    }
}

/// A profile names one chip's boot path; on any other chip it would be
/// silently meaningless, so it is refused.
fn check_profile(
    id: &str,
    chip: &ChipDescriptor,
    family: MachineFamily,
    profile: NodeProfile,
) -> anyhow::Result<()> {
    match profile {
        NodeProfile::ArduinoEsp32 => {
            if family != MachineFamily::Xtensa || is_esp32s3(chip) {
                anyhow::bail!(
                    "node '{id}': profile '{}' is the classic-ESP32 (Xtensa LX6) Arduino fast \
                     boot, and chip '{}' is not a classic ESP32",
                    labwired_config::NODE_PROFILE_ARDUINO_ESP32,
                    chip.name
                );
            }
        }
    }
    Ok(())
}

/// An ESP32-S3 (Xtensa LX7). Recognised by the predicate the single-chip
/// engine dispatches on ([`ChipDescriptor::is_esp32s3`]): the in-tree S3 chip
/// YAMLs spell the core in `arch: xtensa-lx7`, which parses to plain
/// `Arch::Xtensa` with no `core:`, so a `core`-only test sent every S3 node
/// down the classic-ESP32 (LX6) path. A descriptor that does declare an LX7
/// `core:` is accepted too.
fn is_esp32s3(chip: &ChipDescriptor) -> bool {
    chip.is_esp32s3()
        || chip
            .core
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase()
            .contains("lx7")
}

/// The ESP32-C3 is the RISC-V chip whose mask ROM is modelled (see
/// `boot::esp32c3_rom`).
fn is_esp32c3(chip: &ChipDescriptor) -> bool {
    chip.name.to_ascii_lowercase().contains("esp32c3")
}

fn build_avr_node(
    id: &str,
    chip: &ChipDescriptor,
    system: &SystemManifest,
    firmware: NodeFirmware,
    plugins: &[&dyn crate::plugin::ChipPlugin],
) -> anyhow::Result<Box<dyn MachineTrait>> {
    let NodeFirmware::Elf(bytes) = firmware else {
        anyhow::bail!(
            "node '{id}': chip '{}' boots from an ELF, but the firmware is not an ELF file",
            chip.name
        );
    };
    let image =
        parse_elf_image(&bytes).with_context(|| format!("node '{id}': parse firmware ELF"))?;
    let mut bus = crate::bus::SystemBus::from_config_with_plugins(chip, system, plugins)
        .with_context(|| format!("node '{id}': build bus"))?;
    let mut cpu = crate::cpu::Avr::new();
    cpu.load_program_image(&image);
    // SPI/I2C kits park on bus controllers; the AVR interpreter owns
    // transfers via SPDR / TWCR, so move slaves onto the CPU.
    for name in ["spi", "spi0", "spi1"] {
        for dev in bus.take_spi_devices(name) {
            cpu.push_spi_device(dev);
        }
    }
    for name in ["i2c", "i2c0", "twi"] {
        for dev in bus.take_i2c_slaves(name) {
            cpu.push_i2c_slave(dev);
        }
    }
    let machine = Machine::new(cpu, bus);
    Ok(Box::new(machine))
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
    fab: Option<&crate::system::efuse::FactoryMacAllocator>,
    blobs: &crate::system::builder::BlobMap,
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
            if !is_esp32c3(chip) {
                anyhow::bail!(
                    "node '{id}': flash-image ROM boot is modelled per chip, and chip '{}' is not \
                     one of them; supply an ELF instead",
                    chip.name
                );
            }
            let images = c3rom::rom_images_from_blobs_or_provisioned(blobs).with_context(|| {
                format!(
                    "node '{id}': chip '{}' needs the real ESP32-C3 boot ROM to run a flash image; \
                     pass the node esp32c3_irom + esp32c3_drom blobs, install an ESP toolchain \
                     (esp32c3_rev3_rom.elf) or set LABWIRED_ESP32C3_ROM / LABWIRED_ESP32C3_ROM_DATA",
                    chip.name
                )
            })?;
            if !c3rom::inject_rom_regions(&mut bus, &images) {
                anyhow::bail!(
                    "node '{id}': chip '{}' has no instruction-ROM window for the boot ROM",
                    chip.name
                );
            }
            let mut machine = c3rom::build_rom_boot_machine(
                bus,
                flash_bytes,
                c3rom::RomBootOpts {
                    pinned_efuse_mac: fab.map(|f| f.next_mac()),
                    ..Default::default()
                },
                |cpu| cpu,
            );
            // The same run settings the single-chip front ends give a C3
            // (CLI `execute.rs`, the browser's heavy-chip path): the widest
            // tick batch the bus declares safe, and idle fast-forward. A
            // world node without them runs a FreeRTOS idle loop instruction
            // by instruction — ~20x slower for the same simulated time.
            let interval = machine.max_safe_tick_interval();
            machine.config.peripheral_tick_interval = interval;
            machine.bus.config.peripheral_tick_interval = interval;
            machine.config.idle_fast_forward_enabled = true;
            Ok(Box::new(machine))
        }
        NodeFirmware::Elf(bytes) => {
            let image = parse_elf_image(&bytes)
                .with_context(|| format!("node '{id}': parse firmware ELF"))?;
            use crate::Cpu as _;
            // The single-chip engine's C3 bare-ELF path, step for step
            // (`builder::riscv`, the browser's `new_from_config_riscv`): the
            // real mask ROM in its windows, its reset-time `.data` copy
            // replayed, and the analog-I2C and USB-Serial-JTAG models an
            // esp-hal app's ROM calls and console need. Without them an app
            // that calls into the ROM jumps through a zeroed function table.
            // No ROM resolvable (the browser with no blobs) leaves the windows
            // zero, exactly as the single-chip engine does then.
            if is_esp32c3(chip) {
                if let Some(images) =
                    crate::boot::esp32c3_rom::rom_images_from_blobs_or_provisioned(blobs)
                {
                    crate::boot::esp32c3_rom::install_fast_boot_rom(&mut bus, &images);
                }
            }
            let cpu = crate::system::riscv::configure_riscv(&mut bus);
            let mut machine = Machine::new(cpu, bus);
            machine
                .load_firmware(&image)
                .map_err(|e| anyhow::anyhow!("node '{id}': load firmware: {e:?}"))?;
            // Fast boot skips the ROM/2nd-stage bootloader that would normally
            // set the stack pointer, so seed it at the top of RAM (16-byte
            // aligned, RISC-V ABI) or the first prologue store faults.
            let ram_size = chip.ram.size;
            let sp_top = (chip.ram.base + ram_size) as u32;
            machine.cpu.set_sp(sp_top & !0xF);
            machine.cpu.set_pc(image.entry_point as u32);
            Ok(Box::new(machine))
        }
    }
}

fn build_xtensa_node(
    id: &str,
    chip: &ChipDescriptor,
    system: &SystemManifest,
    firmware: NodeFirmware,
    blobs: &crate::system::builder::BlobMap,
    profile: Option<NodeProfile>,
) -> anyhow::Result<Box<dyn MachineTrait>> {
    if is_esp32s3(chip) {
        return build_esp32s3_node(id, chip, system, firmware, blobs);
    }

    let NodeFirmware::Elf(bytes) = firmware else {
        anyhow::bail!(
            "node '{id}': chip '{}' boots from an ELF, but the firmware is not an ELF file",
            chip.name
        );
    };
    let image =
        parse_elf_image(&bytes).with_context(|| format!("node '{id}': parse firmware ELF"))?;

    if profile == Some(NodeProfile::ArduinoEsp32) {
        return build_esp32_arduino_node(id, chip, system, &bytes, &image);
    }

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

/// A classic-ESP32 node on the Arduino fast boot ([`NodeProfile::ArduinoEsp32`]).
///
/// Built by `boot::esp32_arduino::build_arduino_elf_machine` — the one home of
/// that recipe, shared with the debugger and the end-to-end tests — with the
/// symbols resolved by `arduino_esp32_symbols`, the same set the CLI's
/// `profile: arduino-esp32` resolves. Real dual core: Arduino's `loopTask` is
/// pinned to core 1, and the firmware drives the APP_CPU rendezvous itself.
///
/// The profile's helpers keep a little state per thread (the APP_CPU boot
/// mailbox, `pxCurrentTCB`), and a world steps every node on one thread, so a
/// world admits one node on this profile; `World::from_resolved` enforces it.
fn build_esp32_arduino_node(
    id: &str,
    chip: &ChipDescriptor,
    system: &SystemManifest,
    elf: &[u8],
    image: &crate::memory::ProgramImage,
) -> anyhow::Result<Box<dyn MachineTrait>> {
    use crate::boot::esp32_arduino::{
        arduino_esp32_symbols, build_arduino_elf_machine, ArduinoElfBootOpts,
    };
    // Start from a clean slate of the profile's per-thread state, exactly as
    // the single-chip engines do before installing it.
    crate::peripherals::esp_xtensa_common::rom_thunks::reset_esp32_session_state();
    let mut built = build_arduino_elf_machine(
        image,
        arduino_esp32_symbols(elf),
        system,
        &ArduinoElfBootOpts::default(),
    )
    .map_err(|e| anyhow::anyhow!("node '{id}': Arduino-ESP32 boot: {e}"))?;
    built.machine.bus.attach_debug_schemas(chip, system);
    built.machine.bus.refresh_peripheral_index();
    Ok(Box::new(built.machine))
}

/// Build an ESP32-S3 (Xtensa LX7) node.
///
/// Both boot paths are real: a flash image runs the genuine mask ROM from the
/// reset vector, and an ELF fast-boots into the app. The flash image is passed
/// to `configure_xtensa_esp32s3` rather than read from `LABWIRED_ESP32S3_FLASH`,
/// which is what lets two S3 nodes in one world run *different* firmware.
///
/// Both paths are dual-core, and the single-chip runner (`labwired run`) now
/// builds the same shape. The ESP-IDF handshake flags (`s_cpu_inited`,
/// `s_other_cpu_startup_done`, …) are written by the firmware running on core 1,
/// not pre-painted from firmware symbols: core 1 is released by the real
/// hardware edge (`SYSTEM_CORE_1_CONTROL_0.RESETING` 1→0 on the flash path, the
/// `ets_set_appcpu_boot_addr` handover on the ELF path) and then executes the
/// real bring-up. `s_other_cpu_startup_done` in particular is set by core 1's
/// FreeRTOS idle hook, which only runs once a systimer tick wakes core 1 out of
/// `WAITI` — see the `Waiti` arm in `cpu::xtensa_lx7`.
fn build_esp32s3_node(
    id: &str,
    chip: &ChipDescriptor,
    system: &SystemManifest,
    firmware: NodeFirmware,
    blobs: &crate::system::builder::BlobMap,
) -> anyhow::Result<Box<dyn MachineTrait>> {
    use crate::cpu::xtensa_lx7::XtensaLx7;
    use crate::system::xtensa::{configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts};

    let flash_image = match &firmware {
        NodeFirmware::FlashImage(bytes) => Some(bytes.clone()),
        NodeFirmware::Elf(_) => None,
    };
    // The chip descriptor is authoritative, exactly as on the single-chip
    // engine (`builder::xtensa`, the browser's S3 constructors): its `cpu_hz`
    // clocks the SYSTIMER, and on the flash path its flash size is the part's
    // capacity the model reports over RDID — a 16 MiB module booted on a
    // 4 MiB backing fails `esp_flash`'s size check before `app_main`.
    let base = Esp32s3Opts::for_chip(chip);
    let opts = match &flash_image {
        Some(image) => Esp32s3Opts {
            real_reset_boot: true,
            flash_size: crate::system::builder::esp32s3_flash_backing_size(
                chip.flash.size,
                image.len(),
            ),
            flash_image: flash_image.clone(),
            rom_images: crate::boot::esp32s3_rom::rom_images_from_blobs_or_provisioned(blobs),
            ..base
        },
        // Fast boot keeps the single-chip engine's default backing (identity
        // XIP, filled from the ELF's segments). A ROM the caller supplied wins;
        // otherwise `configure_xtensa_esp32s3` provisions one itself.
        None => Esp32s3Opts {
            rom_images: match (blobs.get("esp32s3_irom"), blobs.get("esp32s3_drom")) {
                (Some(irom), Some(drom)) => Some(crate::boot::esp32s3_rom::RomImages {
                    irom: irom.clone(),
                    drom: drom.clone(),
                }),
                _ => None,
            },
            ..base
        },
    };

    let mut bus = crate::bus::SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
    // Devices the board manifest wires (an OLED on i2c0, a panel on SPI) — the
    // same factory the single-chip S3 paths call; a node without it silently
    // dropped every `external_devices` entry.
    crate::system::xtensa::attach_esp32_external_devices(&mut bus, system)
        .with_context(|| format!("node '{id}': attach external devices"))?;
    // Debugger register names still come from the chip YAML even though the
    // peripheral bank is programmatic — see `SystemBus::attach_debug_schemas`.
    bus.attach_debug_schemas(chip, system);
    bus.refresh_peripheral_index();
    let boot_mode = wiring.boot_mode;
    let mut cpu = wiring.cpu;

    match firmware {
        NodeFirmware::FlashImage(_) => {
            if boot_mode != Esp32s3BootMode::Faithful {
                anyhow::bail!(
                    "node '{id}': chip '{}' needs the real ESP32-S3 boot ROM to run a flash image, \
                     but none was found; pass the node esp32s3_irom + esp32s3_drom blobs, install \
                     an ESP toolchain (PlatformIO/ESP-IDF) or set LABWIRED_ESP32S3_ROM_ELF (or pin \
                     LABWIRED_ESP32S3_ROM/_DROM)",
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

/// Parse an avr-gcc ELF into the [`crate::memory::ProgramImage`]
/// `labwired_loader::load_elf_bytes` produces for it.
///
/// avr-gcc links `.text` at a low VMA and `.data` at a biased data-space VMA
/// (`0x80_0000 + addr`) with its LMA in flash, so the CRT can copy it. The
/// loader emits BOTH for a data-space segment — the flash LMA copy (for LPM /
/// `__do_copy_data`) and the biased VMA copy (which
/// [`crate::cpu::Avr::load_program_image`] uses to preload SRAM) — and this is
/// that mapping, segment for segment. [`parse_elf_image`] places every segment
/// at `p_paddr` only, which drops the data-space copies.
///
/// Unlike the loader, an ELF whose `e_machine` is not AVR is refused: loading a
/// foreign image into the AVR interpreter would decode garbage.
pub fn parse_avr_elf_image(bytes: &[u8]) -> anyhow::Result<crate::memory::ProgramImage> {
    use crate::cpu::avr::{classify_avr_vma, AvrLoadSpace};
    use goblin::elf::program_header::PT_LOAD;
    use goblin::elf::Elf;

    let elf = Elf::parse(bytes).context("Failed to parse ELF binary")?;
    let machine = elf.header.e_machine;
    if elf_arch(machine) != Some(crate::Arch::Avr) {
        anyhow::bail!("firmware is not an AVR ELF (e_machine {machine})");
    }
    let mut image = crate::memory::ProgramImage::new(elf.entry, crate::Arch::Avr);
    for ph in &elf.program_headers {
        if ph.p_type != PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        let (off, n) = (ph.p_offset as usize, ph.p_filesz as usize);
        if off + n > bytes.len() {
            anyhow::bail!("Segment out of bounds in ELF file");
        }
        let data = bytes[off..off + n].to_vec();
        let vma = if ph.p_vaddr != 0 {
            ph.p_vaddr
        } else {
            ph.p_paddr
        };
        let (space, data_addr) = classify_avr_vma(vma);
        match space {
            AvrLoadSpace::Flash => {
                let flash_addr = if ph.p_paddr != 0 {
                    ph.p_paddr
                } else {
                    data_addr
                };
                image.add_segment(flash_addr, data);
            }
            AvrLoadSpace::Data | AvrLoadSpace::Eeprom => {
                // Flash LMA holds the initializer image.
                if ph.p_paddr < 0x8000 {
                    image.add_segment(ph.p_paddr, data.clone());
                }
                // Keep the biased VMA so `load_program_image` can tell data
                // space from program space.
                image.add_segment(vma, data);
            }
        }
    }
    Ok(image)
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

    // No parse and no error path: the sizes were validated when the chip
    // deserialised, so "invalid flash size" is no longer reachable here.
    let flash_size = chip.flash.size;
    let ram_size = chip.ram.size;
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
