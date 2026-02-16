// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::SystemBus;
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
