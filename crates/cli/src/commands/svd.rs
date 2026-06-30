// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired ingest-svd` / `import-svd` SVD tooling.

use crate::*;

pub(crate) fn run_ingest_svd(args: IngestSvdArgs) -> ExitCode {
    // Keep stdout pure JSON in --json mode (the MCP agent surface parses it);
    // the progress line is only useful for the human table mode.
    if !args.json {
        info!(
            "Ingesting SVD {:?} -> declarative descriptors in {:?}",
            args.input, args.output_dir
        );
    }

    let xml = match std::fs::read_to_string(&args.input) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to read SVD file: {}", e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    let device = match svd_parser::parse(&xml) {
        Ok(d) => d,
        Err(e) => {
            error!("Failed to parse SVD XML: {}", e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    if let Err(e) = std::fs::create_dir_all(&args.output_dir) {
        error!("Failed to create output directory: {}", e);
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let filter: Option<Vec<String>> = args.filter.as_ref().map(|s| {
        s.split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    });

    let mut summary: Vec<serde_json::Value> = Vec::new();
    let mut errors = 0usize;
    for peripheral in &device.peripherals {
        if let Some(ref f) = filter {
            if !f.iter().any(|n| n.eq_ignore_ascii_case(&peripheral.name)) {
                continue;
            }
        }
        match svd_ingestor::process_peripheral(&device, peripheral) {
            Ok(desc) => {
                if let Err(e) = svd_ingestor::save_descriptor(&desc, &args.output_dir) {
                    error!("Failed to save descriptor for {}: {}", peripheral.name, e);
                    errors += 1;
                    continue;
                }
                let path = args
                    .output_dir
                    .join(format!("{}.yaml", desc.peripheral.to_lowercase()));
                summary.push(serde_json::json!({
                    "name": desc.peripheral,
                    "descriptor_path": path.to_string_lossy(),
                    "register_count": desc.registers.len(),
                    "base_address": format!("0x{:08X}", peripheral.base_address),
                }));
            }
            Err(e) => {
                error!("Failed to process peripheral {}: {}", peripheral.name, e);
                errors += 1;
            }
        }
    }

    if args.json {
        let out = serde_json::json!({
            "output_dir": args.output_dir.to_string_lossy(),
            "peripheral_count": summary.len(),
            "peripherals": summary,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&out).unwrap_or_else(|_| "{}".into())
        );
    } else {
        println!(
            "Ingested {} peripheral(s) into {}:",
            summary.len(),
            args.output_dir.display()
        );
        for p in &summary {
            println!(
                "  {:<16} {:>4} registers -> {}",
                p["name"].as_str().unwrap_or("?"),
                p["register_count"].as_u64().unwrap_or(0),
                p["descriptor_path"].as_str().unwrap_or("?")
            );
        }
    }

    if summary.is_empty() {
        error!("No peripherals ingested (check --filter against the SVD's peripheral names)");
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }
    if errors > 0 {
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }
    ExitCode::from(EXIT_PASS)
}

pub(crate) fn run_import_svd(args: ImportSvdArgs) -> ExitCode {
    info!("Importing SVD from {:?}", args.input);

    let xml = match std::fs::read_to_string(&args.input) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to read SVD file: {}", e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let svd = match svd_parser::parse(&xml) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to parse SVD XML: {}", e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let mut device = match labwired_ir::IrDevice::from_svd(&svd) {
        Ok(d) => d,
        Err(e) => {
            error!("Failed to convert to Strict IR: {}", e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    if let (Some(base), Some(size)) = (args.flash_base, &args.flash_size) {
        device.memory_regions.insert(
            "FLASH".to_string(),
            labwired_ir::IrMemoryRegion {
                name: "FLASH".to_string(),
                base: base as u64,
                size: labwired_config::parse_size(size)
                    .map_err(|e| {
                        error!("Invalid flash size '{}': {}", size, e);
                    })
                    .unwrap_or(0),
            },
        );
    }

    if let (Some(base), Some(size)) = (args.ram_base, &args.ram_size) {
        device.memory_regions.insert(
            "RAM".to_string(),
            labwired_ir::IrMemoryRegion {
                name: "RAM".to_string(),
                base: base as u64,
                size: labwired_config::parse_size(size)
                    .map_err(|e| {
                        error!("Invalid ram size '{}': {}", size, e);
                    })
                    .unwrap_or(0),
            },
        );
    }

    let file = match std::fs::File::create(&args.output) {
        Ok(f) => f,
        Err(e) => {
            error!("Failed to create output file: {}", e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    if let Err(e) = serde_json::to_writer_pretty(file, &device) {
        error!("Failed to write JSON: {}", e);
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    info!("Successfully wrote Strict IR to {:?}", args.output);
    ExitCode::from(EXIT_PASS)
}
