// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::SystemBus;
use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::system::arch_policy::{machine_family, MachineFamily};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::info;

/// Builds a SystemBus from an already-resolved system.
/// If none is provided, returns a default (empty/default) SystemBus.
pub fn build_system_bus(
    system: Option<&labwired_config::ResolvedSystem>,
) -> anyhow::Result<SystemBus> {
    build_system_bus_with_plugins(system, &[])
}

/// [`build_system_bus`] with out-of-tree chip plugins: the system's chip
/// resolves against the plugins' embedded YAMLs, and each peripheral type is
/// offered to the plugins before the in-tree factories.
pub fn build_system_bus_with_plugins(
    system: Option<&labwired_config::ResolvedSystem>,
    plugins: &[&dyn crate::plugin::ChipPlugin],
) -> anyhow::Result<SystemBus> {
    let bus = if let Some(system) = system {
        info!("Loading chip descriptor: {}", system.manifest.chip);
        let chip =
            system.chip_with_plugins(&|name| plugins.iter().find_map(|p| p.chip_yaml(name)))?;
        let mut manifest = system.manifest.clone();
        // Peripheral descriptor paths inside a chip file are resolved relative
        // to it, so a file-backed chip keeps its full path. A built-in name has
        // no directory and is left as the name — its descriptors are embedded.
        if !labwired_config::is_builtin_chip_spec(&manifest.chip) {
            manifest.chip = system
                .base_dir()
                .join(&manifest.chip)
                .to_string_lossy()
                .into_owned();
        }
        SystemBus::from_config_with_plugins(&chip, &manifest, plugins)?
    } else {
        info!("Using default hardware configuration");
        SystemBus::new()
    };

    Ok(bus)
}

/// Build a complete ESP32-classic (Xtensa LX6) dual-core simulation system
/// from an already-parsed `SystemManifest`.
///
/// This is the manifest-driven counterpart to the WASM path in
/// `WasmSimulator::new_from_config_xtensa_esp32`. It:
///   1. Calls `configure_xtensa_esp32` which registers the full ESP32
///      peripheral bank (IRAM/DRAM/Flash/ROM/UART0/SPI0–SPI3/GPIO/…) on a
///      fresh `SystemBus` — the YAML peripherals list is intentionally
///      bypassed because the YAML only documents the memory map; the Rust
///      code is authoritative.
///   2. Calls `attach_esp32_external_devices` to wire any devices declared
///      in `manifest.external_devices` (e.g. the SSD1680 e-paper panel on
///      SPI3) onto the already-configured bus.
///   3. Constructs a real **APP_CPU** (`XtensaLx7::new_app_cpu`) — PRID
///      0xABAB / core 1, starts **halted** until PRO releases it via the
///      silicon boot path (`ets_set_appcpu_boot_addr` / DPORT unstall →
///      `Machine` drains `APPCPU_BOOT_ADDR` and `unhalt()`s core 1). This is
///      the same dual-core model the wasm playground and
///      `e2e_labwired_ereader` use. Arduino-ESP32 / FreeRTOS need that second
///      core (loopTask is pinned to `CONFIG_ARDUINO_RUNNING_CORE=1`).
///
/// `system_path` is only used to resolve any chip descriptor path that
/// still needs to be verified; pass the directory that contains the manifest.
///
/// Returns `(bus, pro_cpu, app_cpu)` so the caller can
/// `Machine::new(pro, bus).with_secondary_cpu(app)` without re-running
/// `configure_xtensa_esp32` (which would clear the bus).
pub fn build_esp32_system_from_manifest(
    manifest: &labwired_config::SystemManifest,
    system_path: &Path,
) -> anyhow::Result<(SystemBus, XtensaLx7, XtensaLx7)> {
    build_esp32_system_from_manifest_with_plugins(manifest, system_path, &|_| None)
}

/// [`build_esp32_system_from_manifest`] with plugin chip lookup: bare chip
/// names not found among the built-ins are offered to `plugin_chips` (chip
/// name → embedded YAML) before giving up.
pub fn build_esp32_system_from_manifest_with_plugins(
    manifest: &labwired_config::SystemManifest,
    system_path: &Path,
    plugin_chips: &dyn Fn(&str) -> Option<&'static str>,
) -> anyhow::Result<(SystemBus, XtensaLx7, XtensaLx7)> {
    let chip_dir = system_path.parent().unwrap_or_else(|| Path::new("."));
    info!("Loading chip descriptor: {}", manifest.chip);
    let chip =
        labwired_config::ChipDescriptor::resolve_with(&manifest.chip, chip_dir, plugin_chips)?;

    let mut bus = SystemBus::new();
    let pro_cpu = crate::system::xtensa::configure_xtensa_esp32(&mut bus);
    crate::system::xtensa::attach_esp32_external_devices(&mut bus, manifest)?;
    // The peripheral BANK is programmatic (above), but the chip YAML is still
    // where a peripheral's debugger register schema is declared. Without this,
    // every `debug_schema:` on an Xtensa chip is inert and the whole bank
    // inspects as `registers: []`.
    bus.attach_debug_schemas(&chip, &anchor_chip_path(manifest, chip_dir));
    bus.refresh_peripheral_index();
    let app_cpu = XtensaLx7::new_app_cpu();

    Ok((bus, pro_cpu, app_cpu))
}

/// A copy of `manifest` whose `chip:` is anchored to `chip_dir`, mirroring what
/// [`build_system_bus`] does before `from_config`.
///
/// `SystemBus::resolve_peripheral_path` resolves a chip-relative descriptor path
/// against `manifest.chip`'s directory, so a manifest carrying a chip path
/// relative to ITSELF would resolve descriptors against the process CWD instead.
/// A built-in chip name has no directory and is left as the name — its
/// descriptors come from the embedded registry, not the filesystem.
pub fn anchor_chip_path(
    manifest: &labwired_config::SystemManifest,
    chip_dir: &Path,
) -> labwired_config::SystemManifest {
    let mut anchored = manifest.clone();
    if !labwired_config::is_builtin_chip_spec(&anchored.chip) {
        anchored.chip = chip_dir.join(&anchored.chip).to_string_lossy().into_owned();
    }
    anchored
}

/// Thin wrapper around [`build_esp32_system_from_manifest`] for callers that
/// only have a path.  Parses the manifest from disk and delegates.
pub fn build_esp32_system(system_path: &Path) -> anyhow::Result<(SystemBus, XtensaLx7, XtensaLx7)> {
    info!("Loading ESP32 system manifest: {:?}", system_path);
    let manifest = labwired_config::SystemManifest::from_file(system_path)?;
    build_esp32_system_from_manifest(&manifest, system_path)
}

// ── Shared machine builder ──────────────────────────────────────────────────
//
// One constructor for (chip, system, firmware) → runnable machine plus the
// UART wires a frontend needs. The per-architecture bodies used to be
// copy-pasted into the browser crate and the CLI; they move here one family at
// a time, each ported verbatim from the browser constructor.

mod arm;
mod avr;
mod riscv;
mod xtensa;

/// Named binary blobs a board references (mask ROM images, merged flash, ...).
pub type BlobMap = HashMap<String, Vec<u8>>;

/// The image a machine runs.
pub enum FirmwareSource<'a> {
    /// ELF bytes.
    Elf(&'a [u8]),
    /// Raw flash image (ESP fast-boot / rom-boot); `symbols` is an optional
    /// companion ELF used only for symbol resolution.
    FlashImage {
        image: &'a [u8],
        symbols: Option<&'a [u8]>,
    },
}

/// How the machine reaches the application.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BootMode {
    /// Load the image and start at its entry point.
    #[default]
    FastBoot,
    /// Run the chip's real mask ROM from the reset vector.
    RomBoot,
}

/// Frontend-facing knobs that do not change what the machine is.
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// Also echo console bytes to the host's stdout (the CLI's default).
    pub echo_uart_stdout: bool,
    /// Restrict host serial input to this named UART. `None` preserves the
    /// legacy broadcast to all UART RX queues. TX is selected independently
    /// by the manifest's `debug_uart` declaration.
    pub uart_rx: Option<String>,
}

type UartRxSources = Vec<Arc<Mutex<VecDeque<u8>>>>;

/// Resolve serial input before constructing the CPU so an unavailable port
/// cannot silently swallow input or redirect it to another peripheral.
fn uart_rx_sources(bus: &SystemBus, options: &BuildOptions) -> anyhow::Result<UartRxSources> {
    match options.uart_rx.as_deref() {
        None => Ok(bus.attach_uart_rx_source()),
        Some(name) => bus
            .attach_uart_rx_source_named(name)
            .map(|source| vec![source])
            .ok_or_else(|| anyhow::anyhow!("UART receive source {name:?} is unavailable")),
    }
}

/// Everything [`build_machine`] needs.
pub struct BuildRequest<'a> {
    pub chip: &'a labwired_config::ChipDescriptor,
    /// The board manifest. `chip` inside it must already be the absolute
    /// descriptor path (as [`build_system_bus`] rewrites it), because peripheral
    /// descriptor paths resolve relative to it.
    pub system: &'a labwired_config::SystemManifest,
    pub firmware: FirmwareSource<'a>,
    pub boot: BootMode,
    /// Mask ROM images, under the names the browser passes them:
    /// `esp32c3_irom` / `esp32c3_drom` (ESP32-C3) and `esp32s3_irom` /
    /// `esp32s3_drom` (ESP32-S3). Required by the flash-image boots, optional
    /// on an ESP ELF fast boot, ignored elsewhere.
    pub blobs: &'a BlobMap,
    pub options: BuildOptions,
}

/// The console capture and RX feeders attached at construction.
pub struct UartWires {
    /// Console TX bytes (the board console the host is plugged into).
    pub sink: Arc<Mutex<Vec<u8>>>,
    /// Selected UART RX queues; bytes pushed here reach firmware.
    pub rx: Vec<Arc<Mutex<VecDeque<u8>>>>,
}

/// A constructed, loaded machine and its wiring.
pub struct BuiltMachine {
    pub machine: Box<dyn crate::session::machine::SessionMachine>,
    pub uart: UartWires,
    pub board_io: Vec<labwired_config::BoardIoBinding>,
    pub arch: labwired_config::Arch,
    /// The ELF (or the companion symbols ELF) for symbol resolution; empty when
    /// the image carries none.
    pub firmware_bytes: Vec<u8>,
}

/// Build a runnable machine from a chip, its board manifest, and firmware.
///
/// Dispatch goes through [`machine_family`], the one architecture policy: a
/// chip declaring no architecture is refused rather than guessed as Cortex-M.
///
/// | family | firmware + boot | path |
/// |---|---|---|
/// | Cortex-M | `Elf` + `FastBoot` | ELF load, no reset |
/// | RISC-V | `Elf` + `FastBoot` | ELF at its entry, SP at top of RAM |
/// | RISC-V (C3) | `FlashImage` + `FastBoot` | 2nd-stage bootloader entered directly |
/// | RISC-V (C3) | `FlashImage` + `RomBoot` | mask ROM from the reset vector |
/// | ESP32-S3 | `Elf` + `FastBoot` | `boot::esp32s3::fast_boot` |
/// | ESP32-S3 | `FlashImage` + `RomBoot` | mask ROM, MMU XIP, dual core |
/// | ESP32 | `Elf` + `FastBoot` | ELF at its entry, dual core |
/// | AVR | `Elf` + `FastBoot` | ELF into the CPU's own flash |
///
/// Any other combination is an error saying so ("not supported: ..." for a
/// boot path the engine has no constructor for); nothing falls back to a
/// different path.
pub fn build_machine(req: BuildRequest<'_>) -> anyhow::Result<BuiltMachine> {
    match machine_family(req.chip)? {
        MachineFamily::CortexM => arm::build(req),
        MachineFamily::RiscV => riscv::build(req),
        MachineFamily::Xtensa => xtensa::build(req),
        MachineFamily::Avr => avr::build(req),
    }
}
