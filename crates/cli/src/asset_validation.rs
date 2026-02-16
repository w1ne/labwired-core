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

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Serialize, Debug, Clone)]
struct ValidationIssue {
    severity: Severity,
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<String>,
}

#[derive(Serialize, Default)]
struct ValidationStatistics {
    total_checks: usize,
    errors: usize,
    warnings: usize,
    infos: usize,
}

#[derive(Serialize)]
struct ValidationResult {
    valid: bool,
    issues: Vec<ValidationIssue>,
    context: String,
    statistics: ValidationStatistics,
}

impl ValidationResult {
    fn new(context: impl Into<String>) -> Self {
        Self {
            valid: true,
            issues: Vec::new(),
            context: context.into(),
            statistics: ValidationStatistics::default(),
        }
    }

    fn add_issue(&mut self, issue: ValidationIssue) {
        self.statistics.total_checks += 1;

        match issue.severity {
            Severity::Error => {
                self.valid = false;
                self.statistics.errors += 1;
            }
            Severity::Warning => {
                self.statistics.warnings += 1;
            }
            Severity::Info => {
                self.statistics.infos += 1;
            }
        }

        self.issues.push(issue);
    }

    fn add_error(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        suggestion: Option<String>,
        location: Option<String>,
    ) {
        self.add_issue(ValidationIssue {
            severity: Severity::Error,
            code: code.into(),
            message: message.into(),
            suggestion,
            location,
        });
    }

    fn add_warning(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        suggestion: Option<String>,
        location: Option<String>,
    ) {
        self.add_issue(ValidationIssue {
            severity: Severity::Warning,
            code: code.into(),
            message: message.into(),
            suggestion,
            location,
        });
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
            result.add_error(
                "SYSTEM_PARSE_ERROR",
                format!("Failed to parse system manifest: {}", e),
                Some("Check YAML syntax and schema compliance".to_string()),
                None,
            );
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
            result.add_error(
                "CHIP_LOAD_ERROR",
                format!(
                    "Failed to load referenced chip '{:?}': {}",
                    chip_path_resolved, e
                ),
                Some(format!(
                    "Ensure chip file exists at {:?}",
                    chip_path_resolved
                )),
                Some("chip".to_string()),
            );
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

        let is_pin = conn.starts_with('p')
            && conn.len() >= 3
            && conn.chars().nth(2).unwrap_or(' ').is_numeric();
        let is_peripheral_id = valid_targets.contains(conn);

        if !is_pin && !is_peripheral_id {
            result.add_error(
                "INVALID_CONNECTION",
                format!(
                    "External device '{}' connects to unknown target '{}'. Available peripherals: {:?}",
                    device.id, conn, valid_targets
                ),
                Some(format!("Use one of the available peripheral IDs: {:?}", valid_targets)),
                Some("external_devices[].connection".to_string()),
            );
        }
    }

    print_result(&result)
}

fn validate_chip(path: &PathBuf) -> ExitCode {
    let mut result = ValidationResult::new(format!("ChipDescriptor: {:?}", path));

    let chip = match ChipDescriptor::from_file(path) {
        Ok(c) => c,
        Err(e) => {
            result.add_error(
                "CHIP_PARSE_ERROR",
                format!("Failed to parse chip descriptor: {}", e),
                Some("Check YAML syntax and schema compliance".to_string()),
                None,
            );
            print_result(&result);
            return ExitCode::from(1);
        }
    };

    // 1. Schema Version Validation
    if chip.schema_version != "1.0" {
        result.add_error(
            "SCHEMA_VERSION_UNSUPPORTED",
            format!(
                "Unsupported schema version '{}'. Supported: '1.0'",
                chip.schema_version
            ),
            Some("Update schema_version field to '1.0' or migrate configuration".to_string()),
            Some("schema_version".to_string()),
        );
    }

    // Validate version format (should be semver-like)
    if !chip.schema_version.contains('.') {
        result.add_warning(
            "SCHEMA_VERSION_FORMAT",
            format!(
                "Schema version '{}' should use semver format (e.g., '1.0')",
                chip.schema_version
            ),
            Some("Use format 'MAJOR.MINOR' for schema_version".to_string()),
            Some("schema_version".to_string()),
        );
    }

    // 2. Memory Region Validation
    let flash_size = labwired_config::parse_size(&chip.flash.size).unwrap_or(0);
    let ram_size = labwired_config::parse_size(&chip.ram.size).unwrap_or(0);

    if flash_size == 0 {
        result.add_error(
            "INVALID_FLASH_SIZE",
            "Flash size is zero or invalid".to_string(),
            Some("Set flash.size to a valid size (e.g., '64KB', '128KB')".to_string()),
            Some("flash.size".to_string()),
        );
    }

    if ram_size == 0 {
        result.add_error(
            "INVALID_RAM_SIZE",
            "RAM size is zero or invalid".to_string(),
            Some("Set ram.size to a valid size (e.g., '20KB', '64KB')".to_string()),
            Some("ram.size".to_string()),
        );
    }

    // Check Flash/RAM overlap
    let flash_end = chip.flash.base + flash_size;
    let ram_end = chip.ram.base + ram_size;

    if chip.flash.base == chip.ram.base {
        result.add_error(
            "MEMORY_SAME_BASE",
            "Flash and RAM have the same base address".to_string(),
            Some("Adjust flash.base or ram.base to different addresses".to_string()),
            Some("flash.base, ram.base".to_string()),
        );
    }

    if chip.flash.base < ram_end && chip.ram.base < flash_end {
        result.add_error(
            "MEMORY_OVERLAP",
            format!(
                "Flash ({:#x}-{:#x}) overlaps with RAM ({:#x}-{:#x})",
                chip.flash.base, flash_end, chip.ram.base, ram_end
            ),
            Some("Adjust flash.base or ram.base to avoid overlap".to_string()),
            Some("flash.base, ram.base".to_string()),
        );
    }

    // 3. Peripheral Validation
    let mut ids = HashMap::new();
    let mut peripheral_ranges: Vec<(usize, String, u64, u64)> = Vec::new();
    let mut irq_map: HashMap<u32, Vec<String>> = HashMap::new();

    for (idx, p) in chip.peripherals.iter().enumerate() {
        // Check for duplicate IDs
        if let Some(_existing) = ids.insert(p.id.clone(), p) {
            result.add_error(
                "DUPLICATE_PERIPHERAL_ID",
                format!("Duplicate peripheral ID '{}' detected", p.id),
                Some("Ensure each peripheral has a unique ID".to_string()),
                Some(format!("peripherals[{}].id", idx)),
            );
        }

        // Build peripheral address ranges
        let size = if let Some(size_str) = &p.size {
            labwired_config::parse_size(size_str).unwrap_or(0x1000)
        } else {
            0x1000 // Default 4KB
        };

        peripheral_ranges.push((idx, p.id.clone(), p.base_address, p.base_address + size));

        // Collect IRQ assignments
        if let Some(irq) = p.irq {
            irq_map.entry(irq).or_default().push(p.id.clone());
        }

        // Check if peripheral overlaps with Flash
        if p.base_address >= chip.flash.base && p.base_address < flash_end {
            result.add_warning(
                "PERIPHERAL_IN_FLASH",
                format!(
                    "Peripheral '{}' at {:#x} is in Flash region",
                    p.id, p.base_address
                ),
                Some(
                    "Peripherals should typically be in peripheral address space (0x40000000+)"
                        .to_string(),
                ),
                Some(format!("peripherals[{}].base_address", idx)),
            );
        }

        // Check if peripheral overlaps with RAM
        if p.base_address >= chip.ram.base && p.base_address < ram_end {
            result.add_warning(
                "PERIPHERAL_IN_RAM",
                format!(
                    "Peripheral '{}' at {:#x} is in RAM region",
                    p.id, p.base_address
                ),
                Some(
                    "Peripherals should typically be in peripheral address space (0x40000000+)"
                        .to_string(),
                ),
                Some(format!("peripherals[{}].base_address", idx)),
            );
        }
    }

    // 4. Peripheral Overlap Detection
    for i in 0..peripheral_ranges.len() {
        for j in (i + 1)..peripheral_ranges.len() {
            let (idx1, id1, start1, end1) = &peripheral_ranges[i];
            let (_idx2, id2, start2, end2) = &peripheral_ranges[j];

            // Check if ranges overlap
            if start1 < end2 && start2 < end1 {
                result.add_error(
                    "PERIPHERAL_OVERLAP",
                    format!(
                        "Peripheral '{}' ({:#x}-{:#x}) overlaps with '{}' ({:#x}-{:#x})",
                        id1, start1, end1, id2, start2, end2
                    ),
                    Some(format!(
                        "Adjust base_address or size of '{}' or '{}'",
                        id1, id2
                    )),
                    Some(format!("peripherals[{}].base_address", idx1)),
                );
            }
        }
    }

    // 5. IRQ Conflict Detection
    for (irq_num, peripherals) in &irq_map {
        if peripherals.len() > 1 {
            result.add_error(
                "IRQ_CONFLICT",
                format!(
                    "IRQ {} is assigned to multiple peripherals: {}",
                    irq_num,
                    peripherals.join(", ")
                ),
                Some("Assign unique IRQ numbers to each peripheral".to_string()),
                Some("peripherals[].irq".to_string()),
            );
        }

        // Validate IRQ range (Cortex-M typically 0-239)
        if *irq_num > 239 {
            result.add_warning(
                "IRQ_OUT_OF_RANGE",
                format!("IRQ {} exceeds typical Cortex-M range (0-239)", irq_num),
                Some("Verify IRQ number against target architecture".to_string()),
                None,
            );
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
