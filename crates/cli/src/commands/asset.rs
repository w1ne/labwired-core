// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired asset` subcommands: scaffold projects, peripherals, and verify.

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

pub(crate) fn run_asset_verify(args: VerifyArgs) -> ExitCode {
    info!("Verifying asset from {:?}", args.ir);

    // 1. Locate the 'ai' directory
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ai_dir = manifest_dir.join("../../../ai");

    if !ai_dir.exists() {
        error!("Could not find 'ai' directory at {:?}", ai_dir);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    // 2. Determine Python Command
    let python_cmd = if let Some(venv) = args.venv {
        venv.join("bin/python3")
    } else {
        let local_venv = ai_dir.join(".venv/bin/python3");
        if local_venv.exists() {
            local_venv
        } else {
            PathBuf::from("python3")
        }
    };

    info!("Using Python: {:?}", python_cmd);

    // 3. Construct the command
    let ir_path = args.ir.canonicalize().unwrap_or(args.ir);

    let mut cmd = std::process::Command::new(python_cmd);
    cmd.current_dir(&ai_dir)
        .arg("-m")
        .arg("labwired_ai.verify_harness")
        .arg("--ir")
        .arg(&ir_path);

    if let Some(id) = args.id {
        cmd.arg("--id").arg(id);
    }

    // Redirect stdout/stderr to inheritance
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    info!("Running AI Verification harness...");
    match cmd.status() {
        Ok(status) if status.success() => {
            info!("Verification PASSED for {:?}", ir_path);
            ExitCode::from(EXIT_PASS)
        }
        Ok(status) => {
            error!("Verification FAILED with status: {}", status);
            ExitCode::from(EXIT_RUNTIME_ERROR)
        }
        Err(e) => {
            error!("Failed to execute Verification: {}", e);
            ExitCode::from(EXIT_RUNTIME_ERROR)
        }
    }
}

pub(crate) fn run_asset_create(args: CreateArgs) -> ExitCode {
    info!(
        "Creating asset for '{}' from {:?} (Pages: {})",
        args.name, args.pdf, args.pages
    );

    // 1. Locate the 'ai' directory
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ai_dir = manifest_dir.join("../../../ai");

    if !ai_dir.exists() {
        error!("Could not find 'ai' directory at {:?}", ai_dir);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    // 2. Determine Python Command
    let python_cmd = if let Some(venv) = args.venv {
        venv.join("bin/python3")
    } else {
        let local_venv = ai_dir.join(".venv/bin/python3");
        if local_venv.exists() {
            local_venv
        } else {
            PathBuf::from("python3")
        }
    };

    info!("Using Python: {:?}", python_cmd);

    // 3. Construct the command
    let pdf_path = args.pdf.canonicalize().unwrap_or(args.pdf);
    let output_path = args.output.canonicalize().unwrap_or(args.output);
    let strict_ir_path = args.strict_ir.map(|p| p.canonicalize().unwrap_or(p));

    let mut cmd = std::process::Command::new(python_cmd);
    cmd.current_dir(&ai_dir)
        .arg("-m")
        .arg("labwired_ai")
        .arg("ingest-datasheet")
        .arg("--pdf")
        .arg(&pdf_path)
        .arg("--pages")
        .arg(args.pages)
        .arg("--name")
        .arg(args.name)
        .arg("--output")
        .arg(&output_path);

    if let Some(ref strict_ir) = strict_ir_path {
        cmd.arg("--strict-ir").arg(strict_ir);
    }

    // Redirect stdout/stderr to inheritance so the user sees LLM progress
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    info!("Running AI Ingestion pipeline...");
    match cmd.status() {
        Ok(status) if status.success() => {
            info!("Successfully created YAML asset at {:?}", output_path);
            if let Some(ref strict_ir) = strict_ir_path {
                info!("Successfully created Strict IR at {:?}", strict_ir);
            }
            ExitCode::from(EXIT_PASS)
        }
        Ok(status) => {
            error!("AI Ingestion failed with status: {}", status);
            ExitCode::from(EXIT_RUNTIME_ERROR)
        }
        Err(e) => {
            error!("Failed to execute AI Ingestion: {}", e);
            ExitCode::from(EXIT_RUNTIME_ERROR)
        }
    }
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
