use clap::Parser;
use labwired_config::{ChipDescriptor, SystemManifest};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use tracing::error;

#[derive(Parser, Debug)]
pub struct ValidateArgs {
    /// Path to the system manifest (YAML) to validate
    #[arg(long, conflicts_with = "chip")]
    pub system: Option<PathBuf>,

    /// Path to the chip descriptor (YAML) to validate
    #[arg(long, conflicts_with = "system")]
    pub chip: Option<PathBuf>,
}

#[derive(Parser, Debug)]
pub struct ListChipsArgs {
    /// Filter string for chip names
    #[arg(short, long)]
    pub filter: Option<String>,

    /// Output format (text or json)
    #[arg(long, default_value = "text")]
    pub format: String,
}

#[derive(Serialize)]
struct ValidationResult {
    valid: bool,
    errors: Vec<String>,
    warnings: Vec<String>,
    context: String,
}

impl ValidationResult {
    fn new(context: impl Into<String>) -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
            context: context.into(),
        }
    }

    fn error(&mut self, msg: impl Into<String>) {
        self.valid = false;
        self.errors.push(msg.into());
    }

    fn _warning(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }
}

pub fn run_validate(args: ValidateArgs) -> ExitCode {
    if let Some(path) = args.system {
        validate_system(&path)
    } else if let Some(path) = args.chip {
        validate_chip(&path)
    } else {
        error!("Must provide either --system or --chip");
        ExitCode::from(2)
    }
}

fn validate_system(path: &PathBuf) -> ExitCode {
    let mut result = ValidationResult::new(format!("SystemManifest: {:?}", path));

    // 1. Load System Manifest
    let system = match SystemManifest::from_file(path) {
        Ok(s) => s,
        Err(e) => {
            result.error(format!("Failed to parse system manifest: {}", e));
            print_result(&result);
            return ExitCode::from(1);
        }
    };

    // 2. Load Referenced Chip
    // Resolving chip path relative to system file
    let chip_path_resolved = if let Some(parent) = path.parent() {
        parent.join(&system.chip)
    } else {
        PathBuf::from(&system.chip)
    };

    let chip = match ChipDescriptor::from_file(&chip_path_resolved) {
        Ok(c) => c,
        Err(e) => {
            result.error(format!(
                "Failed to load referenced chip '{:?}': {}",
                chip_path_resolved, e
            ));
            print_result(&result);
            return ExitCode::from(1);
        }
    };

    // 3. Validate Connections
    // Every connection in external_devices must map to a valid peripheral ID in the chip
    // OR be a direct GPIO/Pin reference (if supported by schema, currently usually "usart1" etc)
    
    // Build a set of valid connection targets from chip peripherals
    let valid_targets: Vec<String> = chip.peripherals.iter().map(|p| p.id.clone()).collect();
    
    for device in &system.external_devices {
        let conn = &device.connection;
             // For now, simple ID matching.
             // If connection looks like "pc13", check if it's a known pin naming convention or if we have a GPIO peripheral.
             // LabWired config usually maps functionality to Peripheral IDs for now (e.g. "usart1").
             // If it's a pin code (p[a-z][0-9]+), we assume it maps to gpio ports.
             
             let is_pin = conn.starts_with('p') && conn.len() >= 3 && conn.chars().nth(2).unwrap_or(' ').is_numeric();
             let is_peripheral_id = valid_targets.contains(conn);

             if !is_pin && !is_peripheral_id {
                 result.error(format!(
                     "External device '{}' connects to unknown target '{}'. Available peripherals: {:?}",
                     device.id, conn, valid_targets
                 ));
             }
    }

    print_result(&result)
}

fn validate_chip(path: &PathBuf) -> ExitCode {
    let mut result = ValidationResult::new(format!("ChipDescriptor: {:?}", path));

    let chip = match ChipDescriptor::from_file(path) {
        Ok(c) => c,
        Err(e) => {
            result.error(format!("Failed to parse chip descriptor: {}", e));
            print_result(&result);
            return ExitCode::from(1);
        }
    };

    // 1. Check Memory Map Overlaps
    // Flash vs RAM
    let flash_size = labwired_config::parse_size(&chip.flash.size).unwrap_or(0);
    let ram_size = labwired_config::parse_size(&chip.ram.size).unwrap_or(0);
    
    let _flash_end = chip.flash.base + flash_size;
    let _ram_end = chip.ram.base + ram_size;

    // Simple integrity check: Base addresses
    if chip.flash.base == chip.ram.base {
         result.error("Flash and RAM have the same base address");
    }

    // 2. Validate Peripherals
    let mut ids = HashMap::new();
    for p in &chip.peripherals {
        if let Some(_existing) = ids.insert(p.id.clone(), p) {
            result.error(format!("Duplicate peripheral ID '{}' detected.", p.id));
        }

        // Check bounds
        // If size is defined, check constraints.
        // For now, just ensuring base_address is not 0 unless it's a special case? 
        // 0x0 is usually Flash, so Peripherals should be at >= 0x4000_0000 typically for Cortex-M
        if p.base_address < 0x2000_0000 {
             result._warning(format!("Peripheral '{}' has a low base address ({:#x}). Is this intended?", p.id, p.base_address));
        }
    }

    print_result(&result)
}

fn print_result(result: &ValidationResult) -> ExitCode {
    // Always print JSON for agents
    let json = serde_json::to_string_pretty(result).unwrap_or_default();
    println!("{}", json);
    
    if result.valid {
        ExitCode::from(0)
    } else {
        ExitCode::from(1)
    }
}

pub fn run_list_chips(args: ListChipsArgs) -> ExitCode {
     // Scan standard locations
    let roots = [
        PathBuf::from("configs/chips"),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips"),
    ];

    let mut found_chips = Vec::new();

    for root in &roots {
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                 let path = entry.path();
                 if let Some(ext) = path.extension() {
                     if ext == "yaml" || ext == "yml" {
                         // Try to parse name
                         if let Ok(desc) = ChipDescriptor::from_file(&path) {
                             if let Some(filter) = &args.filter {
                                 if !desc.name.contains(filter) {
                                     continue;
                                 }
                             }
                             found_chips.push(desc);
                         }
                     }
                 }
            }
        }
    }

    if args.format == "json" {
        println!("{}", serde_json::to_string_pretty(&found_chips).unwrap());
    } else {
        println!("Available Chips:");
        for chip in found_chips {
            println!("- {} (Arch: {:?})", chip.name, chip.arch);
        }
    }

    ExitCode::from(0)
}
