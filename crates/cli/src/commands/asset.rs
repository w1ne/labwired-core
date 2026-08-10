// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired asset` subcommands: scaffold projects and peripherals.

use crate::*;

pub(crate) fn run_asset_add_peripheral(args: AddPeripheralArgs) -> ExitCode {
    info!("Adding peripheral '{}' to {:?}", args.id, args.chip);

    let mut chip = match labwired_config::ChipDescriptor::from_file(&args.chip) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load chip descriptor: {}", e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Check if peripheral already exists
    if chip.peripherals.iter().any(|p| p.id == args.id) {
        error!("Peripheral '{}' already exists in {:?}", args.id, args.chip);
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let mut config = std::collections::HashMap::new();
    config.insert(
        "path".to_string(),
        serde_yaml::Value::String(args.ir_path.to_string_lossy().to_string()),
    );

    chip.peripherals.push(labwired_config::PeripheralConfig {
        id: args.id,
        r#type: args.r#type,
        base_address: args.base as u64,
        size: Some("4KB".to_string()),
        irq: None,
        clock: None,
        config,
    });

    let yaml = match serde_yaml::to_string(&chip) {
        Ok(y) => y,
        Err(e) => {
            error!("Failed to serialize chip descriptor: {}", e);
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }
    };

    if let Err(e) = std::fs::write(&args.chip, yaml) {
        error!("Failed to write updated chip descriptor: {}", e);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    info!("Successfully added peripheral to {:?}", args.chip);
    ExitCode::from(EXIT_PASS)
}

pub(crate) fn run_asset_init(args: InitArgs) -> ExitCode {
    let output_dir = args.output;
    if output_dir.exists() {
        error!("Output directory already exists: {:?}", output_dir);
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    if let Err(e) = std::fs::create_dir_all(&output_dir) {
        error!("Failed to create directory {:?}: {}", output_dir, e);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    let chip_input = args.chip.unwrap_or_else(|| "stm32f103".to_string());
    let chip_source = match resolve_chip_descriptor_path(&chip_input) {
        Some(path) => path,
        None => {
            error!(
                "Could not resolve chip descriptor '{}'. Pass a valid file path or a known chip in configs/chips.",
                chip_input
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let chip_file_name = match chip_source.file_name() {
        Some(name) => name.to_string_lossy().to_string(),
        None => {
            error!("Invalid chip descriptor path: {:?}", chip_source);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let chip_dest = output_dir.join(&chip_file_name);
    if let Err(e) = std::fs::copy(&chip_source, &chip_dest) {
        error!(
            "Failed to copy chip descriptor from {:?} to {:?}: {}",
            chip_source, chip_dest, e
        );
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    let system_yaml = format!(
        r#"# LabWired System Configuration
name: "my-project"
chip: "{}"
external_devices: []
"#,
        chip_file_name
    );

    let system_path = output_dir.join("system.yaml");
    if let Err(e) = std::fs::write(&system_path, system_yaml) {
        error!("Failed to write system.yaml: {}", e);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    info!("Initialized new project skeleton in {:?}", output_dir);
    info!(
        "Created system.yaml with chip: {} (copied from {:?})",
        chip_file_name, chip_source
    );
    ExitCode::from(EXIT_PASS)
}
