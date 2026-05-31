// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::SystemBus;
use crate::cpu::xtensa_lx7::XtensaLx7;
use std::path::Path;
use tracing::info;

/// Builds a SystemBus from a given system manifest path.
/// If no path is provided, returns a default (empty/default) SystemBus.
pub fn build_system_bus(system_path: Option<&Path>) -> anyhow::Result<SystemBus> {
    let bus = if let Some(sys_path) = system_path {
        info!("Loading system manifest: {:?}", sys_path);
        let mut manifest = labwired_config::SystemManifest::from_file(sys_path)?;
        let chip_path = sys_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&manifest.chip);
        manifest.chip = chip_path.to_string_lossy().into_owned();
        info!("Loading chip descriptor: {:?}", chip_path);
        let chip = labwired_config::ChipDescriptor::from_file(&chip_path)?;
        SystemBus::from_config(&chip, &manifest)?
    } else {
        info!("Using default hardware configuration");
        SystemBus::new()
    };

    Ok(bus)
}

/// Build a complete ESP32-classic (Xtensa LX6) simulation system from a
/// system manifest YAML.
///
/// This is the manifest-driven counterpart to the WASM path in
/// `WasmSimulator::new_from_config_xtensa_esp32`. It:
///   1. Parses the system manifest and chip descriptor at `system_path`.
///   2. Calls `configure_xtensa_esp32` which registers the full ESP32
///      peripheral bank (IRAM/DRAM/Flash/ROM/UART0/SPI0–SPI3/GPIO/…) on a
///      fresh `SystemBus` — the YAML peripherals list is intentionally
///      bypassed because the YAML only documents the memory map; the Rust
///      code is authoritative.
///   3. Calls `attach_esp32_external_devices` to wire any devices declared
///      in `manifest.external_devices` (e.g. the SSD1680 e-paper panel on
///      SPI3) onto the already-configured bus.
///
/// Returns `(bus, cpu)` so the caller can pass them to `Machine::new`
/// without needing to call `configure_xtensa_esp32` again (which would
/// clear the bus and lose the attached external devices).
pub fn build_esp32_system(system_path: &Path) -> anyhow::Result<(SystemBus, XtensaLx7)> {
    info!("Loading ESP32 system manifest: {:?}", system_path);
    let manifest = labwired_config::SystemManifest::from_file(system_path)?;
    let chip_path = system_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&manifest.chip);
    info!("Loading chip descriptor: {:?}", chip_path);
    let _chip = labwired_config::ChipDescriptor::from_file(&chip_path)?;

    let mut bus = SystemBus::new();
    let cpu = crate::system::xtensa::configure_xtensa_esp32(&mut bus);
    crate::system::xtensa::attach_esp32_external_devices(&mut bus, &manifest)?;
    bus.refresh_peripheral_index();

    Ok((bus, cpu))
}
