// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

mod commands;
mod wifi_frames;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use wifi_frames::*;
// use std::sync::atomic::Ordering; // Removed as unused
use labwired_core::{Bus, Cpu};
use std::sync::{Arc, Mutex};
use tracing::{error, info};

mod api_client;
mod asset_validation;
mod component_validation;
use labwired_cli::coverage;
mod gpio_observer;
mod size_limited_writer;
mod vcd_trace;

use labwired_config::{
    load_test_script, LoadedTestScript, StopReason, TestAssertion, TestLimits, UdsTesterDetails,
};

pub(crate) const EXIT_PASS: u8 = 0;
pub(crate) const EXIT_ASSERT_FAIL: u8 = 1;
pub(crate) const EXIT_CONFIG_ERROR: u8 = 2;
pub(crate) const EXIT_RUNTIME_ERROR: u8 = 3;

const RESULT_SCHEMA_VERSION: &str = "1.0";

fn parse_u32_addr(s: &str) -> Result<u32, String> {
    let trimmed = s.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).map_err(|e| format!("Invalid hex address '{}': {}", s, e))
    } else {
        u32::from_str(trimmed).map_err(|e| format!("Invalid address '{}': {}", s, e))
    }
}

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "LabWired Simulator",
    long_about = None,
    subcommand_negates_reqs = true
)]
struct Cli {
    /// Path to the firmware ELF file
    #[arg(short, long)]
    firmware: Option<PathBuf>,

    /// Path to the system manifest (YAML)
    #[arg(short, long)]
    system: Option<PathBuf>,

    /// Write a state snapshot (JSON) for interactive runs.
    #[arg(long)]
    snapshot: Option<PathBuf>,

    /// Breakpoint PC address (repeatable). Stops simulation when PC matches.
    #[arg(long, value_parser = parse_u32_addr)]
    breakpoint: Vec<u32>,

    /// Enable instruction-level execution tracing
    #[arg(short, long, global = true)]
    trace: bool,

    /// Maximum number of steps to execute (default: 20000)
    #[arg(long, default_value = "20000")]
    max_steps: usize,

    /// Start a GDB server on the specified port
    #[arg(long)]
    gdb: Option<u16>,

    /// Output errors and diagnostics as structured JSON for agent consumption
    #[arg(long, global = true)]
    json: bool,

    /// Output VCD trace to file
    #[arg(long, global = true)]
    vcd: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Deterministic, CI-friendly runner mode driven by a test script (YAML).
    Test(TestArgs),

    /// Machine control operations (load, etc.)
    Machine(MachineArgs),

    /// Utilities for Asset Foundry
    Asset(AssetArgs),

    /// Run a firmware ELF in the simulator using a chip descriptor.
    ///
    /// Loads the chip's peripheral wiring, fast-boots the firmware, and
    /// runs the simulation loop.  Output written to USB_SERIAL_JTAG (for
    /// Xtensa chips) or UART (for ARM chips) appears on stdout in real
    /// time.
    Run(RunArgs),

    /// Capture a binary runtime snapshot of a firmware mid-flight, for
    /// fast-replay in the playground. Produces an `.lwrs` blob that
    /// `WasmSimulator::apply_runtime_snapshot` can restore.
    Snapshot(SnapshotArgs),

    /// Report ESP32-S3 register-level peripheral coverage against the SVD.
    ///
    /// Probes every register in the SVD behaviorally (read/write sentinel) and
    /// classifies each as Modelled / Indeterminate / Unmodelled. Prints a
    /// human-readable table and optionally writes the full matrix as JSON.
    Coverage(CoverageArgs),

    /// Run the Tier-1 chip × peripheral validation matrix and export it.
    Tier1Matrix(Tier1MatrixArgs),

    /// Coverage-guided fuzz a firmware in the silicon-validated simulator.
    ///
    /// Mutates an input byte stream injected into the firmware's RAM buffer,
    /// drives execution with AFL-style edge coverage, and reports crashes. The
    /// target firmware follows a small contract (length+data buffer, a verdict
    /// word with DONE/FAULT markers) so any crash found here is replayable on
    /// real silicon (`--features hw-oracle-stm32` HIL-confirm) — silicon-true
    /// findings, not emulation false positives. Exits non-zero if a crash is
    /// found (CI-friendly).
    Fuzz(FuzzArgs),
}

#[derive(Parser, Debug)]
pub struct FuzzArgs {
    /// Path to the chip descriptor YAML.
    #[arg(long)]
    pub chip: PathBuf,

    /// Path to the system manifest YAML.
    #[arg(long)]
    pub system: PathBuf,

    /// Path to the firmware ELF (must follow the fuzz contract below).
    #[arg(long)]
    pub firmware: PathBuf,

    /// Max fuzzing iterations before giving up.
    #[arg(long, default_value = "200000")]
    pub max_iters: usize,

    /// Max simulator steps per run (a run past this is a hang).
    #[arg(long, default_value = "1000000")]
    pub max_steps: usize,

    /// RNG seed — fuzzing is deterministic for a fixed seed.
    #[arg(long, default_value = "3735928559")]
    pub seed: u64,

    /// Seed input as hex bytes (e.g. `5000` for [0x50,0x00]). Repeatable.
    #[arg(long = "seed-input", value_name = "HEX")]
    pub seed_input: Vec<String>,

    /// Collect up to N distinct crashes instead of stopping at the first.
    #[arg(long)]
    pub collect: Option<usize>,

    /// Write the crashing input(s) as a JSON array of byte arrays to this path.
    #[arg(long = "crashes-out")]
    pub crashes_out: Option<PathBuf>,

    /// Contract: address of the u32 input-length word.
    #[arg(long, value_parser = parse_hex_u32, default_value = "0x20002800")]
    pub input_len_addr: u32,

    /// Contract: address of the input data buffer.
    #[arg(long, value_parser = parse_hex_u32, default_value = "0x20002804")]
    pub input_data_addr: u32,

    /// Contract: address of the u32 verdict word.
    #[arg(long, value_parser = parse_hex_u32, default_value = "0x20003000")]
    pub verdict_addr: u32,

    /// Contract: verdict value the firmware writes on clean completion.
    #[arg(long, value_parser = parse_hex_u32, default_value = "0xC0DEF022")]
    pub done_magic: u32,

    /// Contract: verdict value a fault/panic handler writes on a crash.
    #[arg(long, value_parser = parse_hex_u32, default_value = "0xDEADFA17")]
    pub fault_magic: u32,
}

fn parse_hex_u32(s: &str) -> Result<u32, String> {
    let t = s.trim_start_matches("0x").trim_start_matches("0X");
    u32::from_str_radix(t, 16).map_err(|e| format!("invalid hex u32 `{s}`: {e}"))
}

#[derive(Parser, Debug)]
pub struct Tier1MatrixArgs {
    /// Write the matrix as JSON (the committed snapshot path is
    /// docs/coverage/tier1-matrix.json).
    #[arg(long = "json-out")]
    pub json_out: Option<PathBuf>,

    /// Evidence link stamped into every cell that carries evidence (skips na and unrecorded).
    #[arg(long = "run-url")]
    pub run_url: Option<String>,
}

#[derive(Parser, Debug)]
pub struct CoverageArgs {
    /// Path to the ESP32-S3 SVD (else auto-discovered from PlatformIO or
    /// LABWIRED_ESP32S3_SVD env var).
    #[arg(long)]
    pub svd: Option<PathBuf>,

    /// Write the coverage matrix as JSON to this path.
    #[arg(long = "json-out", id = "coverage_json_out")]
    pub json_out: Option<PathBuf>,
}

#[derive(Parser, Debug)]
pub struct SnapshotArgs {
    #[command(subcommand)]
    pub command: SnapshotCommands,
}

#[derive(Subcommand, Debug)]
pub enum SnapshotCommands {
    /// Boot a firmware, step N times, write a runtime snapshot blob.
    Capture(SnapshotCaptureArgs),
}

#[derive(Parser, Debug)]
pub struct SnapshotCaptureArgs {
    /// Path to the firmware ELF.
    #[arg(long)]
    pub firmware: PathBuf,

    /// Number of cycles to run before taking the snapshot.
    #[arg(long)]
    pub steps: u64,

    /// Output `.lwrs` path.
    #[arg(long)]
    pub output: PathBuf,

    /// Board manifest (SystemManifest YAML) declaring the external peripherals
    /// to attach (panel, sensors, …). Peripherals are NEVER hardcoded; they come
    /// from this manifest via the generic attach_esp32_external_devices factory.
    #[arg(long)]
    pub system: Option<PathBuf>,

    /// Firmware profile to use. Currently only `agentdeck` is supported —
    /// installs the Arduino-ESP32 / ESP32-classic bootstrap (heap-caps
    /// thunks, dual-core handshake fakery, IPI bridge, image header,
    /// SSD1680 tri-color panel attached to spi3 / GPIO5). Each Arduino-ESP32
    /// firmware has a different set of thunk PCs, so the profile name maps
    /// to a hand-curated address list inside the binary.
    #[arg(long, default_value = "agentdeck")]
    pub profile: String,

    /// Print a progress line every N steps. 0 = silent.
    #[arg(long, default_value = "5000000")]
    pub progress_every: u64,
}

#[derive(Parser, Debug)]
pub struct RunArgs {
    /// Path to the chip descriptor YAML.
    #[arg(long)]
    pub chip: PathBuf,

    /// Path to the firmware ELF.
    #[arg(long)]
    pub firmware: PathBuf,

    /// Maximum number of simulator steps before exit (default: unlimited).
    #[arg(long)]
    pub max_steps: Option<u64>,

    /// Optional path to write a JSON-line GPIO transition trace.
    /// Each line is `{"sim_cycle":N, "pin":P, "from":B, "to":B}`.
    #[arg(long)]
    pub gpio_trace: Option<PathBuf>,

    /// Boot from the real ROM reset vector (0x40000400) instead of fast-booting
    /// the ELF. The chip's real boot ROM runs and loads the 2nd-stage bootloader
    /// and app through the SPI-flash controller — the faithful chip-model path.
    /// Requires LABWIRED_ESP32S3_FLASH (the firmware flash image). The boot ROM is
    /// auto-provisioned from the installed ESP toolchain, or pinned via
    /// LABWIRED_ESP32S3_ROM/_DROM (pre-extracted bins) or LABWIRED_ESP32S3_ROM_ELF.
    #[arg(long)]
    pub rom_boot: bool,

    /// Debug: PC address(es) (hex, e.g. `0x4004eacc`) to break on. On the
    /// first time each is reached, dump a0..a15 + PS/window state and any
    /// `--watch-mem` words, then continue. Repeatable. Works on `--rom-boot`.
    #[arg(long = "break-at", value_name = "HEX")]
    pub break_at: Vec<String>,

    /// Debug: memory address(es) (hex) to read as u32 and print whenever a
    /// `--break-at` fires — for tracing ROM pointer chains. Repeatable.
    #[arg(long = "watch-mem", value_name = "HEX")]
    pub watch_mem: Vec<String>,
}

#[derive(Parser, Debug)]
pub struct AssetArgs {
    #[command(subcommand)]
    pub command: AssetCommands,
}

#[derive(Subcommand, Debug)]
pub enum AssetCommands {
    /// Import an SVD file and convert it to Strict IR (JSON).
    ImportSvd(ImportSvdArgs),

    /// Generate Rust code from Strict IR (JSON).
    Codegen(CodegenArgs),

    /// Initialize a new project skeleton.
    Init(InitArgs),

    /// Add a peripheral to the current chip descriptor.
    AddPeripheral(AddPeripheralArgs),

    /// Validate a System Manifest and its referenced Chip.
    Validate(asset_validation::ValidateArgs),

    /// List available chip descriptors.
    ListChips(asset_validation::ListChipsArgs),

    /// Create a new peripheral asset from a PDF datasheet using AI.
    Create(CreateArgs),

    /// Verify an AI-generated peripheral model using a simulator loopback.
    Verify(VerifyArgs),

    /// Validate an off-chip component IR spec (YAML).
    ValidateComponent(component_validation::ValidateComponentArgs),

    /// Ingest an SVD into runnable declarative PeripheralDescriptor YAML.
    ///
    /// Unlike `import-svd` (Strict IR → codegen → Rust, needs a rebuild), this
    /// emits descriptors the simulator runs directly as `type: declarative`
    /// peripherals — no codegen, no recompile. The one-step path from a vendor
    /// SVD to a working chip.
    IngestSvd(IngestSvdArgs),
}

#[derive(Parser, Debug)]
pub struct IngestSvdArgs {
    /// Path to the input SVD file.
    #[arg(short, long)]
    pub input: PathBuf,

    /// Directory to write `<peripheral>.yaml` descriptors into.
    #[arg(short, long)]
    pub output_dir: PathBuf,

    /// Only ingest these peripherals (comma-separated names). Default: all.
    #[arg(long)]
    pub filter: Option<String>,

    /// Emit a machine-readable JSON summary on stdout (paths + register counts)
    /// instead of a human table. Used by the MCP agent surface.
    #[arg(long)]
    pub json: bool,
}

#[derive(Parser, Debug)]
pub struct CodegenArgs {
    /// Path to the input Strict IR (JSON) file
    #[arg(short, long)]
    pub input: PathBuf,

    /// Path to the output Rust file
    #[arg(short, long)]
    pub output: PathBuf,
}

#[derive(Parser, Debug)]
pub struct CreateArgs {
    /// Path to the input PDF datasheet
    #[arg(short = 'd', long)]
    pub pdf: PathBuf,

    /// Comma-separated list of pages to analyze (e.g. "12,15,20")
    #[arg(short, long)]
    pub pages: String,

    /// Name of the peripheral to extract (e.g. "USART1")
    #[arg(short, long)]
    pub name: String,

    /// Path to the output YAML file
    #[arg(short, long)]
    pub output: PathBuf,

    /// Path to the output Strict IR (JSON) file
    #[arg(short, long)]
    pub strict_ir: Option<PathBuf>,

    /// Optional path to a python virtual environment
    #[arg(long)]
    pub venv: Option<PathBuf>,
}

#[derive(Parser, Debug)]
pub struct VerifyArgs {
    /// Path to the peripheral IR (JSON) to verify
    #[arg(short, long)]
    pub ir: PathBuf,

    /// Optional peripheral ID (defaults to name in IR)
    #[arg(short = 'n', long)]
    pub id: Option<String>,

    /// Optional path to a python virtual environment
    #[arg(long)]
    pub venv: Option<PathBuf>,
}

#[derive(Parser, Debug)]
pub struct InitArgs {
    /// Path to the output directory
    #[arg(short, long)]
    pub output: PathBuf,

    /// Chip name or path to chip descriptor
    #[arg(short, long)]
    pub chip: Option<String>,
}

#[derive(Parser, Debug)]
pub struct AddPeripheralArgs {
    /// Path to the chip descriptor YAML to modify
    #[arg(short, long)]
    pub chip: PathBuf,

    /// New peripheral ID
    #[arg(short, long)]
    pub id: String,

    /// Peripheral type (e.g., "strict_ir")
    #[arg(long, default_value = "strict_ir")]
    pub r#type: String,

    /// Base memory address
    #[arg(short, long, value_parser = parse_u32_addr)]
    pub base: u32,

    /// Path to the IR descriptor (JSON)
    #[arg(long)]
    pub ir_path: PathBuf,
}

#[derive(Parser, Debug)]
pub struct ImportSvdArgs {
    /// Path to the input SVD file
    #[arg(short, long)]
    pub input: PathBuf,

    /// Path to the output JSON file
    #[arg(short, long)]
    pub output: PathBuf,

    /// Optional Flash base address
    #[arg(long, value_parser = parse_u32_addr)]
    pub flash_base: Option<u32>,

    /// Optional Flash size (e.g. "512KB")
    #[arg(long)]
    pub flash_size: Option<String>,

    /// Optional RAM base address
    #[arg(long, value_parser = parse_u32_addr)]
    pub ram_base: Option<u32>,

    /// Optional RAM size (e.g. "128KB")
    #[arg(long)]
    pub ram_size: Option<String>,
}

#[derive(Parser, Debug)]
pub struct MachineArgs {
    #[command(subcommand)]
    pub command: MachineCommands,
}

#[derive(Subcommand, Debug)]
pub enum MachineCommands {
    /// Load a machine state from a snapshot and resume simulation.
    Load(LoadArgs),
}

#[derive(Parser, Debug)]
pub struct LoadArgs {
    /// Path to the snapshot JSON file
    #[arg(short, long)]
    pub snapshot: PathBuf,

    /// Override maximum number of steps to execute
    #[arg(long)]
    pub max_steps: Option<usize>,

    /// Enable instruction-level execution tracing
    #[arg(short, long)]
    pub trace: bool,
}

#[derive(Parser, Debug)]
struct TestArgs {
    /// Path to the firmware ELF file
    #[arg(short = 'f', long)]
    firmware: Option<PathBuf>,

    /// Path to the system manifest (YAML)
    #[arg(short = 's', long)]
    system: Option<PathBuf>,

    /// Path to the test script (YAML)
    #[arg(short = 'c', long)]
    script: PathBuf,

    /// Override max steps (takes precedence over script)
    #[arg(long)]
    max_steps: Option<u64>,

    /// Breakpoint PC address (repeatable). Stops simulation when PC matches.
    #[arg(long, value_parser = parse_u32_addr)]
    breakpoint: Vec<u32>,

    /// Disable UART stdout echo (still captured for assertions/artifacts)
    #[arg(long)]
    no_uart_stdout: bool,

    /// Directory to write test artifacts (result.json, uart.log)
    #[arg(long)]
    output_dir: Option<PathBuf>,

    /// Optional path to write a JUnit XML report for CI systems
    #[arg(long)]
    junit: Option<PathBuf>,

    /// Override max cycles limit
    #[arg(long)]
    max_cycles: Option<u64>,

    /// Override max UART bytes limit
    #[arg(long)]
    max_uart_bytes: Option<u64>,

    /// Number of steps with no PC change to detect stuck state (default: None)
    #[arg(long, alias = "no-progress")]
    detect_stuck: Option<u64>,

    /// Override max VCD file size limit (bytes)
    #[arg(long)]
    max_vcd_bytes: Option<u64>,

    /// Enable instruction tracing (saved to trace.json)
    #[arg(long)]
    trace: bool,

    /// Output VCD trace to file
    #[arg(long)]
    vcd: Option<PathBuf>,

    /// Maximum number of instructions to trace
    #[arg(long, default_value = "100000")]
    trace_max: usize,

    /// Collect firmware statement coverage. Writes coverage.info (LCOV) and
    /// coverage.json into --output-dir. Distinct from `labwired coverage`,
    /// which measures chip-model register faithfulness.
    #[arg(long)]
    coverage: bool,

    /// Write a signable, reproducible run-manifest.json into --output-dir
    /// (input hashes, engine version, result subset, coverage summary, and a
    /// wall-clock-free SHA-256 digest).
    #[arg(long)]
    run_manifest: bool,

    /// Explicitly opt out of sending LABWIRED_API_KEY even if it is set in the environment.
    /// Useful for local development and testing.
    #[arg(long)]
    no_key: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct TestResult {
    result_schema_version: String,
    status: String,
    steps_executed: u64,
    cycles: u64,
    instructions: u64,
    stop_reason: StopReason,
    stop_reason_details: StopReasonDetails,
    limits: TestLimits,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    assertions: Vec<AssertionResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_state: Option<labwired_core::snapshot::CpuSnapshot>,
    firmware_hash: String,
    config: TestConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct StopReasonDetails {
    triggered_stop_condition: StopReason,
    triggered_limit: Option<NamedU64>,
    observed: Option<NamedU64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct NamedU64 {
    name: String,
    value: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AssertionResult {
    assertion: TestAssertion,
    passed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct TestConfig {
    firmware: PathBuf,
    system: Option<PathBuf>,
    script: PathBuf,
}

use labwired_core::snapshot::CpuSnapshot;

#[derive(Debug, Serialize, Deserialize)]
struct PeripheralSnapshot {
    name: String,
    base: u64,
    size: u64,
    irq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct InteractiveSnapshotConfig {
    firmware: PathBuf,
    system: Option<PathBuf>,
    max_steps: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Snapshot {
    Standard {
        cpu: CpuSnapshot,
        steps_executed: u64,
        cycles: u64,
        instructions: u64,
        stop_reason: StopReason,
        stop_reason_details: StopReasonDetails,
        limits: TestLimits,
        firmware_hash: String,
        config: TestConfig,
    },
    ConfigError {
        message: String,
        stop_reason_details: StopReasonDetails,
        limits: TestLimits,
        config: TestConfig,
    },
    Interactive {
        snapshot_schema_version: String,
        status: String,
        steps_executed: u64,
        cycles: u64,
        instructions: u64,
        stop_reason: StopReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        firmware_hash: String,
        cpu: CpuSnapshot,
        peripherals: Vec<PeripheralSnapshot>,
        config: InteractiveSnapshotConfig,
    },
}

// snapshot_cortexm_cpu removed, use cpu.snapshot() directly

struct InteractiveSnapshotInputs<'a> {
    firmware_path: &'a Path,
    system_path: Option<&'a PathBuf>,
    max_steps: usize,
    steps_executed: u64,
    stop_reason: StopReason,
    message: Option<String>,
}

/// Unified error response for agent consumption
#[derive(Debug, Serialize)]
struct ErrorResponse {
    error_type: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
    exit_code: u8,
}

/// Emit an error message, respecting the --json flag for structured output
pub(crate) fn emit_error(
    json_mode: bool,
    error_type: &str,
    message: String,
    details: Option<serde_json::Value>,
    exit_code: u8,
) {
    if json_mode {
        let response = ErrorResponse {
            error_type: error_type.to_string(),
            message: message.clone(),
            details,
            exit_code,
        };
        if let Ok(json) = serde_json::to_string_pretty(&response) {
            println!("{}", json);
        } else {
            // Fallback if JSON serialization fails
            eprintln!(
                "{{\"error_type\":\"{}\",\"message\":\"{}\",\"exit_code\":{}}}",
                error_type,
                message.replace('"', "\\\""),
                exit_code
            );
        }
    } else {
        error!("{}", message);
    }
}

fn write_interactive_snapshot<C: labwired_core::Cpu>(
    path: &Path,
    metrics: &labwired_core::metrics::PerformanceMetrics,
    machine: &labwired_core::Machine<C>,
    inputs: InteractiveSnapshotInputs<'_>,
) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            error!("Failed to create snapshot parent dir {:?}: {}", parent, e);
            return;
        }
    }

    let firmware_hash = match std::fs::read(inputs.firmware_path) {
        Ok(bytes) => {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            format!("{:x}", hasher.finalize())
        }
        Err(e) => {
            error!(
                "Failed to read firmware for snapshot hash {:?}: {}",
                inputs.firmware_path, e
            );
            String::new()
        }
    };

    let machine_snapshot = machine.snapshot();
    let peripherals = machine
        .bus
        .peripherals
        .iter()
        .map(|p| {
            let state = machine_snapshot.peripherals.get(&p.name).cloned();
            PeripheralSnapshot {
                name: p.name.clone(),
                base: p.base,
                size: p.size,
                irq: p.irq,
                state,
            }
        })
        .collect::<Vec<_>>();

    let cpu_snapshot = machine.cpu.snapshot();

    let snapshot = Snapshot::Interactive {
        snapshot_schema_version: "1.0".to_string(),
        status: if matches!(
            inputs.stop_reason,
            StopReason::MemoryViolation | StopReason::DecodeError
        ) {
            "error".to_string()
        } else {
            "ok".to_string()
        },
        steps_executed: inputs.steps_executed,
        cycles: metrics.get_cycles(),
        instructions: metrics.get_instructions(),
        stop_reason: inputs.stop_reason,
        message: inputs.message,
        firmware_hash,
        cpu: cpu_snapshot,
        peripherals,
        config: InteractiveSnapshotConfig {
            firmware: inputs.firmware_path.to_path_buf(),
            system: inputs.system_path.cloned(),
            max_steps: inputs.max_steps,
        },
    };

    match std::fs::File::create(path) {
        Ok(f) => {
            if let Err(e) = serde_json::to_writer_pretty(f, &snapshot) {
                error!("Failed to write snapshot {:?}: {}", path, e);
            }
        }
        Err(e) => error!("Failed to create snapshot {:?}: {}", path, e),
    }
}
fn main() -> ExitCode {
    let cli = Cli::parse();

    // Initialize tracing with appropriate level based on --trace flag
    if cli.trace {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_max_level(tracing::Level::DEBUG)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_max_level(tracing::Level::INFO)
            .init();
    }

    match cli.command {
        Some(Commands::Test(args)) => commands::test::run_test(args),
        Some(Commands::Machine(args)) => run_machine(args),
        Some(Commands::Asset(args)) => run_asset(args),
        Some(Commands::Run(args)) => commands::run::run_firmware(args),
        Some(Commands::Snapshot(args)) => commands::snapshot::run_snapshot(args),
        Some(Commands::Coverage(args)) => commands::coverage::run_coverage(args),
        Some(Commands::Tier1Matrix(args)) => commands::tier1::run_tier1_matrix(args),
        Some(Commands::Fuzz(args)) => commands::fuzz::run_fuzz(args),
        None => commands::run::run_interactive(cli),
    }
}

/// Build an ESP32-C3 ROM-boot machine; `efuse_mac` programs a distinct factory
/// MAC so multiple instances are distinguishable on the shared VirtualWifi air.
fn build_c3_rom_boot_machine(
    mut bus: labwired_core::bus::SystemBus,
    efuse_mac: Option<[u8; 6]>,
) -> Result<labwired_core::Machine<labwired_core::cpu::RiscV>, ExitCode> {
    // ── Faithful RISC-V ROM boot (ESP32-C3) ──────────────────────────
    // Reset to the BROM vector 0x4000_0000 (RISC-V `_start`, which jumps to
    // the BROM startup at 0x40001e90) and let the real mask ROM run:
    // it initializes the ROM's own DRAM globals (rom_phyFuns &c.) — which
    // fast-boot skips, causing the rom_i2c_writeReg_Mask indirect-call
    // crash — then loads the 2nd-stage bootloader + app from the flash
    // image through the SPI-flash controller and jumps to app_main, exactly
    // like silicon. "Run the binary, don't thunk it." Requires the real ROM
    // (LABWIRED_ESP32C3_ROM[_DATA], loaded into the chip's rom regions by
    // from_config) and the flash image (LABWIRED_ESP32C3_FLASH).
    use std::sync::{Arc, Mutex};
    let flash_path = match std::env::var("LABWIRED_ESP32C3_FLASH") {
        Ok(p) => p,
        Err(_) => {
            eprintln!(
                "error: --rom-boot needs LABWIRED_ESP32C3_FLASH set (the flash image: \
                     bootloader@0x0 + partition-table@0x8000 + app@0x10000)"
            );
            return Err(ExitCode::from(EXIT_CONFIG_ERROR));
        }
    };
    let flash_bytes = match std::fs::read(&flash_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read flash image {flash_path}: {e}");
            return Err(ExitCode::from(EXIT_RUNTIME_ERROR));
        }
    };
    eprintln!(
        "labwired-riscv: rom-boot from reset vector 0x40000000 (flash image {} bytes from {})",
        flash_bytes.len(),
        flash_path
    );
    let backing = Arc::new(Mutex::new(flash_bytes));
    // Route reads through peripherals first: the fast path checks the
    // chip's `flash`/`drom` memory-regions (zero-filled in rom-boot) before
    // peripherals, which would shadow the FlashXip windows we install at the
    // same XIP addresses. Disabling it lets the MMU-translating FlashXip
    // serve 0x4200_0000 / 0x3C00_0000 from the real flash image.
    bus.config.optimized_bus_access = false;
    // SPIMEM1 flash-command controller (0x6000_2000) backed by the real
    // image, overriding the declarative stub — a narrower, later-registered
    // window wins, so the BROM's READ/RDID/RDSR commands return real bytes.
    // The C3's SPI1 shares the S3's SPIMEM register layout, so the S3 model
    // drops in unchanged.
    bus.add_peripheral(
        "spimem1_flash",
        0x6000_2000,
        0x100,
        None,
        Box::new(
            labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(backing.clone()),
        ),
    );
    // SPIMEM0 (0x6000_3000) — the cache's auto-fetch MSPI controller. Back
    // it with the same flash image too, in case the BROM's bootloader load
    // path issues commands here rather than on SPIMEM1.
    bus.add_peripheral(
        "spimem0_flash",
        0x6000_3000,
        0x100,
        None,
        Box::new(
            labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(backing.clone()),
        ),
    );
    // Flash cache MMU: the 2nd-stage bootloader programs the virtual→flash
    // page table at 0x600C_5000, then runs the app from the XIP windows
    // (IROM 0x4200_0000, DROM 0x3C00_0000). Model the real MMU table shared
    // with two FlashXip windows that translate through it (C3 entry format:
    // invalid=BIT(8), 0xFF page field, 8 MiB span) over the flash image —
    // so the app executes from flash exactly like silicon.
    use labwired_core::peripherals::esp32s3::flash_xip::{
        new_mmu_table, Esp32s3MmuTable, FlashXipPeripheral, MMU_FMT_C3,
    };
    let mmu_table = new_mmu_table();
    bus.add_peripheral(
        "mmu_table",
        0x600C_5000,
        0x800,
        None,
        Box::new(Esp32s3MmuTable::new(mmu_table.clone())),
    );
    // EXTMEM cache controller (0x600C_4000): auto-completes the cache
    // invalidate/sync launch→done handshake the ROM busy-polls (offset 0x28,
    // launch bit0 / done bit1). Overrides the declarative stub, which never
    // asserts done and spins Cache_Invalidate_ICache_Items forever.
    bus.add_peripheral(
        "extmem_cache",
        0x600C_4000,
        0x400,
        None,
        Box::new(labwired_core::peripherals::esp32c3::cache::Esp32c3Cache::new()),
    );
    // Analog I²C master / ANA_CONFIG block (0x6000_E000, DR_REG_I2C_ANA_MST_BASE):
    // rom_i2c_writeReg drives it (read-modify-write of ANA_CONFIG regs) during
    // PHY/clock bring-up; the libphy full RF calibration also touches regs up
    // past 0x6000_E130, so the window spans 0x400. The model reports the
    // master FSM status (0x50 bits[26:24]=7, idle/done) so the ROM's
    // transaction busy-poll exits; all other regs are register-backed.
    bus.add_peripheral(
        "rtc_i2c_ana",
        0x6000_E000,
        0x400,
        None,
        Box::new(labwired_core::peripherals::esp32c3::ana_i2c::Esp32c3AnaI2c::new()),
    );
    // FE/PHY register block (0x6001_1000): libphy's set_rx_gain_table also
    // writes gain/FE config into the gap between uart1 (0x6001_0000) and
    // i2c0 (0x6001_3000). Register-backed storage for those RF tables.
    bus.add_peripheral(
        "wifi_fe",
        0x6001_1000,
        0x2000,
        None,
        Box::new(labwired_core::peripherals::esp32c3::reg_block::Esp32c3RegBlock::new(0x2000)),
    );
    // Baseband/RF register block (0x6001_C000): libphy writes the RX gain
    // table and other BB/RF config here (set_rx_gain_table). Unmapped, the
    // gain-table store faults. Register-backed window up to the declarative
    // peripheral at 0x6001_CC00. (RF air-gap: storage is enough — there's
    // no real RF that would act on these values.)
    bus.add_peripheral(
        "wifi_bb",
        0x6001_C000,
        0xC00,
        None,
        Box::new(labwired_core::peripherals::esp32c3::reg_block::Esp32c3RegBlock::new(0xC00)),
    );
    // Radio front-end PLL-lock status (RADIO_FE 0x6000_6000 + 0x174, bit16):
    // the libphy pll_cal launches the BBPLL/RF PLL then busy-polls this bit
    // for lock; without real RF it never sets and pll_cal spins/retries
    // ("pll_cal exceeds 2ms!!!"). Force-assert it (RF air-gap cut) over just
    // that one word, leaving the declarative radio_fe descriptors intact.
    bus.add_peripheral(
        "radio_fe_pll_lock",
        0x6000_6174,
        0x4,
        None,
        Box::new(
            labwired_core::peripherals::esp32c3::forced_status::Esp32c3ForcedStatus::new(
                0x4,
                vec![(0x0, 1 << 16)],
            ),
        ),
    );
    // WiFi MAC (WIFI_MAC 0x6003_3000, 12 KiB) — behavioral model for the
    // MAC <-> SimNet bridge: register-backed bring-up, MAC-ready bit (0xD14
    // b0, polled by hal_init), RX descriptor-ring DMA + RX-frame injection,
    // and MAC interrupt (matrix source 0) on RX-done. Overrides the
    // declarative wifi_mac window. See docs/esp32c3_wifi_mac_bridge.md.
    bus.add_peripheral(
        "wifi_mac",
        0x6003_3000,
        0x3000,
        None,
        Box::new(labwired_core::peripherals::esp32c3::wifi_mac::Esp32c3WifiMac::new()),
    );
    // Hardware RNG data register (WDEV_RND_REG, 0x6002_60B0): yields a fresh
    // word per read. bootloader_fill_random XORs successive reads and
    // process_segments refills ram_obfs_value until non-zero — a constant
    // RNG gives 0 and spins forever. Override the SYSCON stub at this word.
    bus.add_peripheral(
        "wdev_rnd",
        0x6002_60B0,
        0x4,
        None,
        Box::new(labwired_core::peripherals::esp32c3::rng::Esp32c3Rng::new()),
    );
    // SHA accelerator (0x6003_B000): the 2nd-stage bootloader verifies the
    // app image's appended SHA-256 with it; an unmodelled (zero) digest
    // makes it reject the image. Real SHA-256 block compression here.
    bus.add_peripheral(
        "sha",
        0x6003_B000,
        0x100,
        None,
        Box::new(labwired_core::peripherals::esp32c3::sha::Esp32c3Sha::new()),
    );
    bus.add_peripheral(
        "flash_irom_xip",
        0x4200_0000,
        0x80_0000, // 8 MiB I-cache window
        None,
        Box::new(FlashXipPeripheral::new_mmu_fmt(
            backing.clone(),
            0x4200_0000,
            mmu_table.clone(),
            MMU_FMT_C3,
        )),
    );
    bus.add_peripheral(
        "flash_drom_xip",
        0x3C00_0000,
        0x80_0000, // 8 MiB D-cache window
        None,
        Box::new(FlashXipPeripheral::new_mmu_fmt(
            backing.clone(),
            0x3C00_0000,
            mmu_table.clone(),
            MMU_FMT_C3,
        )),
    );
    // SAR ADC (APB_SARADC, 0x6004_0000): the IDF's adc_hal_self_calibration
    // triggers single conversions and polls a data-valid flag (0x44 bit31/
    // bit30) before reading the result; the declarative stub never asserts
    // it, so read_cal_channel spins forever after spi_flash init. Model
    // conversions as instant (valid flags set, mid-scale sample) so the
    // bounded cal search converges and boot continues.
    bus.add_peripheral(
        "apb_saradc",
        0x6004_0000,
        0x100,
        None,
        Box::new(labwired_core::peripherals::esp32c3::sar_adc::Esp32c3SarAdc::new()),
    );
    // SYSTIMER (0x6002_3000): the 16 MHz free-running counter behind
    // esp_timer and the FreeRTOS tick. systimer_hal_get_counter_value sets
    // UNITx_OP bit30 (UPDATE) then polls bit29 (VALUE_VALID) before reading
    // the snapshot; the declarative stub never asserts VALUE_VALID, so the
    // counter read spins forever right after heap_init. The C3 SYSTIMER is
    // the same IP as the S3 (identical register layout), so the S3 model
    // drops in: it asserts VALUE_VALID, advances the counter, and supports
    // the alarm/IRQ path FreeRTOS needs. Clocked relative to the 160 MHz
    // CPU (10 CPU cycles per 16 MHz tick).
    bus.add_peripheral(
        "systimer",
        0x6002_3000,
        0x100,
        None,
        // C3 SYSTIMER_TARGET0 routes through the interrupt matrix on source
        // 37 (TARGET1/2 at 38/39), unlike the S3's 57; the FreeRTOS tick
        // alarm fires on that source.
        Box::new(
            labwired_core::peripherals::esp32s3::systimer::Systimer::new_with_source(
                160_000_000,
                37,
            ),
        ),
    );
    // RTC_CNTL main timer (0x6000_8000): the free-running slow-clock counter
    // the IDF reads via rtc_time_get (set TIME_UPDATE @0x0C bit31 to latch,
    // read TIME0 @0x10 / TIME1 @0x14). A frozen counter makes every
    // RTC-deadline wait spin forever — most notably calibrate_ocode, which
    // polls a regi2c comparator that never settles without real RF and
    // relies on a ~10 ms RTC timeout to give up and continue. A real
    // advancing timer lets that loop (and other RTC delays) reach the
    // timeout exactly as silicon does. Overrides the declarative RTC_CNTL
    // stub for this window; non-timer regs stay register-backed so the
    // reset-cause seed at 0x38 below still reads back.
    bus.add_peripheral(
        "rtc_cntl_timer",
        0x6000_8000,
        0x100,
        None,
        Box::new(labwired_core::peripherals::esp32c3::rtc_timer::Esp32c3RtcTimer::new()),
    );
    // Seed the power-on hardware reset state the BROM reads to decide it's a
    // normal flash boot (silicon has this at reset; the sim starts zeroed):
    //   * RTC_CNTL reset-cause (0x6000_8038, bits[5:0]) = 1 (POWERON_RESET).
    //     rtc_get_reset_reason returns this; BROM main treats reset_reason 0
    //     as an error and bails (ret to 0) — 1 lets it continue to flash.
    //   * GPIO_STRAP (0x6000_4038) bit3 = SPI fast-flash-boot (matches the
    //     Xtensa rom-boot strap).
    let _ = bus.write_u32(0x6000_8038, 0x0000_0001);
    let _ = bus.write_u32(0x6000_4038, 0x0000_0008);
    //   * eFuse wafer version (EFUSE_RD_MAC_SPI_SYS_3 @ 0x6000_8850,
    //     WAFER_VERSION_MINOR_LO bits[20:18]) = 4 → chip rev v0.4. The real
    //     C3 is v0.4; without it eFuse reads v0.0 and the 2nd-stage
    //     bootloader rejects the app ("requires chip rev >= v0.3").
    let _ = bus.write_u32(0x6000_8850, 0x0010_0000);
    // Enable C3 RISC-V interrupt routing: the bus routes asserted peripheral
    // sources + the SYSTEM FROM_CPU IPI registers through the INTERRUPT_CORE0
    // matrix into the CPU's external interrupt lines. FreeRTOS's first
    // context switch (vPortYield → FROM_CPU SW interrupt) depends on this.
    bus.esp32c3_irq_routing = true;
    if let Some(mac) = efuse_mac {
        let lo =
            mac[5] as u32 | (mac[4] as u32) << 8 | (mac[3] as u32) << 16 | (mac[2] as u32) << 24;
        let hi = mac[1] as u32 | (mac[0] as u32) << 8;
        let _ = bus.write_u32(0x6000_8844, lo);
        let _ = bus.write_u32(0x6000_8848, hi);
    }
    let mut cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
    cpu.set_pc(0x4000_0000);
    // Disable the internal CLINT timer: the C3 has no standard MTIP — its
    // 31 interrupt lines (incl. line 7) are ESP matrix lines, so a
    // self-pending MTIP would collide. mtimecmp=MAX keeps mip bit7 clear.
    cpu.mtimecmp = u64::MAX;
    Ok(labwired_core::Machine::new(cpu, bus))
}

/// Two-station WiFi run: boot two ESP32-C3 instances with distinct factory MACs
/// onto the shared [`virtual_wifi`] medium. Each is a full real firmware over its
/// own real MAC; the medium is the AP + the air between them. They associate, get
/// distinct DHCP leases (192.168.4.2 / .3), and exchange routed IP traffic.
fn run_two_c3_wifi(
    args: &RunArgs,
    chip: &labwired_config::ChipDescriptor,
    manifest: &labwired_config::SystemManifest,
) -> ExitCode {
    use labwired_core::bus::SystemBus;
    use labwired_core::peripherals::esp32c3::{virtual_wifi, wifi_mac::Esp32c3WifiMac};

    virtual_wifi::reset();
    eprintln!(
        "[dual] two-C3 WiFi over shared VirtualWifi: A=02:00:00:00:00:02, B=02:00:00:00:00:03"
    );

    let build =
        |mac: [u8; 6]| -> Result<labwired_core::Machine<labwired_core::cpu::RiscV>, ExitCode> {
            let bus = match SystemBus::from_config(chip, manifest) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("error: failed to build system bus: {e:#}");
                    return Err(ExitCode::from(EXIT_CONFIG_ERROR));
                }
            };
            build_c3_rom_boot_machine(bus, Some(mac))
        };
    let mut a = match build([0x02, 0, 0, 0, 0, 0x02]) {
        Ok(m) => m,
        Err(c) => return c,
    };
    let mut b = match build([0x02, 0, 0, 0, 0, 0x03]) {
        Ok(m) => m,
        Err(c) => return c,
    };
    // Attach each station's WiFi MAC to the medium (medium mode), and label each
    // station's UART output so the shared stdout is readable.
    for (m, label) in [(&mut a, "[A] "), (&mut b, "[B] ")] {
        for p in m.bus.peripherals.iter_mut() {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            if let Some(mac) = any.downcast_mut::<Esp32c3WifiMac>() {
                mac.attach_to_medium();
            } else if let Some(uart) = any.downcast_mut::<labwired_core::peripherals::uart::Uart>()
            {
                uart.set_stdout_prefix(label);
            }
        }
    }

    let limit = args.max_steps.unwrap_or(u64::MAX);
    for i in 0..limit {
        if let Err(e) = a.step() {
            eprintln!("[dual] station A halted at step {i}: {e}");
            break;
        }
        if let Err(e) = b.step() {
            eprintln!("[dual] station B halted at step {i}: {e}");
            break;
        }
    }
    eprintln!("[dual] run complete");
    ExitCode::SUCCESS
}

fn run_asset(args: AssetArgs) -> ExitCode {
    match args.command {
        AssetCommands::ImportSvd(a) => commands::svd::run_import_svd(a),
        AssetCommands::Codegen(a) => commands::codegen::run_codegen(a),
        AssetCommands::Init(a) => commands::asset::run_asset_init(a),
        AssetCommands::AddPeripheral(a) => commands::asset::run_asset_add_peripheral(a),
        AssetCommands::Validate(a) => asset_validation::run_validate(a),
        AssetCommands::ListChips(a) => asset_validation::run_list_chips(a),
        AssetCommands::Create(a) => commands::asset::run_asset_create(a),
        AssetCommands::Verify(a) => commands::asset::run_asset_verify(a),
        AssetCommands::ValidateComponent(a) => component_validation::run_validate_component(a),
        AssetCommands::IngestSvd(a) => commands::svd::run_ingest_svd(a),
    }
}

pub(crate) fn resolve_chip_descriptor_path(chip: &str) -> Option<PathBuf> {
    let input = PathBuf::from(chip);
    if input.exists() {
        return Some(input);
    }

    // If the input looks like a custom path and does not exist, do not guess.
    if input.components().count() != 1 {
        return None;
    }

    let names = if input.extension().is_some() {
        vec![input]
    } else {
        vec![
            PathBuf::from(format!("{}.yaml", chip)),
            PathBuf::from(format!("{}.yml", chip)),
        ]
    };

    let fallback_roots = [
        PathBuf::from("configs/chips"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips"),
    ];

    for root in &fallback_roots {
        for name in &names {
            let candidate = root.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

fn run_machine(args: MachineArgs) -> ExitCode {
    match args.command {
        MachineCommands::Load(load_args) => commands::machine::run_machine_load(load_args),
    }
}

struct LoopResult {
    stop_reason: StopReason,
    steps_executed: u64,
    stop_message: Option<String>,
}

fn run_simulation_loop<C: labwired_core::Cpu>(
    cli: &Cli,
    machine: &mut labwired_core::Machine<C>,
    metrics: &labwired_core::metrics::PerformanceMetrics,
) -> LoopResult {
    let mut stop_reason = StopReason::MaxSteps;
    let mut steps_executed: u64 = 0;
    let mut stop_message: Option<String> = None;

    info!("Running for {} steps...", cli.max_steps);
    for step in 0..cli.max_steps {
        if !cli.breakpoint.is_empty() && cli.breakpoint.contains(&machine.cpu.get_pc()) {
            info!(
                "Breakpoint hit at PC={:#x} (step={})",
                machine.cpu.get_pc(),
                step
            );
            stop_reason = StopReason::Halt;
            steps_executed = step as u64;
            break;
        }
        match machine.step() {
            Ok(_) => {
                steps_executed = (step + 1) as u64;
                if !cli.trace && step > 0 && step % 10000 == 0 {
                    info!(
                        "Progress: {} steps, current IPS: {:.2}",
                        step,
                        metrics.get_ips()
                    );
                }
            }
            Err(e) => {
                info!("Simulation Error at step {}: {}", step, e);
                stop_reason = match e {
                    labwired_core::SimulationError::MemoryViolation(_) => {
                        StopReason::MemoryViolation
                    }
                    labwired_core::SimulationError::DecodeError(_) => StopReason::DecodeError,
                    labwired_core::SimulationError::Halt => StopReason::Halt,
                    labwired_core::SimulationError::SnapshotSchemaMismatch { .. } => {
                        StopReason::Exception
                    }
                    labwired_core::SimulationError::Other(_) => StopReason::Exception,
                    labwired_core::SimulationError::NotImplemented(_) => StopReason::Exception,
                    labwired_core::SimulationError::BreakpointHit(_) => StopReason::Halt,
                    labwired_core::SimulationError::ExceptionRaised { .. } => StopReason::Exception,
                };
                stop_message = Some(e.to_string());
                break;
            }
        }
    }

    LoopResult {
        stop_reason,
        steps_executed,
        stop_message,
    }
}

fn report_metrics<C: labwired_core::Cpu>(
    cli: &Cli,
    cpu: &C,
    metrics: &labwired_core::metrics::PerformanceMetrics,
) {
    if cli.json {
        let report = serde_json::json!({
            "status": "finished",
            "final_pc": cpu.get_pc(),
            "total_instructions": metrics.get_instructions(),
            "total_cycles": metrics.get_cycles(),
            "average_ips": metrics.get_ips(),
        });
        println!("{}", serde_json::to_string(&report).unwrap());
    } else {
        info!("Simulation loop finished.");
        info!("Final PC: {:#x}", cpu.get_pc());
        info!("Total Instructions: {}", metrics.get_instructions());
        info!("Total Cycles: {}", metrics.get_cycles());
        info!("Average IPS: {:.2}", metrics.get_ips());
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_stop_reason_details(
    stop_reason: &StopReason,
    limits: &TestLimits,
    steps_executed: u64,
    cycles: u64,
    uart_bytes: u64,
    stuck_steps: u64,
    duration: std::time::Duration,
    vcd_bytes: u64,
) -> StopReasonDetails {
    let (triggered_limit, observed) = match stop_reason {
        StopReason::MaxSteps => (
            Some(NamedU64 {
                name: "max_steps".to_string(),
                value: limits.max_steps,
            }),
            Some(NamedU64 {
                name: "steps_executed".to_string(),
                value: steps_executed,
            }),
        ),
        StopReason::MaxCycles => (
            limits.max_cycles.map(|v| NamedU64 {
                name: "max_cycles".to_string(),
                value: v,
            }),
            Some(NamedU64 {
                name: "cycles".to_string(),
                value: cycles,
            }),
        ),
        StopReason::MaxUartBytes => (
            limits.max_uart_bytes.map(|v| NamedU64 {
                name: "max_uart_bytes".to_string(),
                value: v,
            }),
            Some(NamedU64 {
                name: "uart_bytes".to_string(),
                value: uart_bytes,
            }),
        ),
        StopReason::NoProgress => (
            limits.no_progress_steps.map(|v| NamedU64 {
                name: "no_progress_steps".to_string(),
                value: v,
            }),
            Some(NamedU64 {
                name: "stuck_steps".to_string(),
                value: stuck_steps,
            }),
        ),
        StopReason::WallTime => (
            limits.wall_time_ms.map(|v| NamedU64 {
                name: "wall_time_ms".to_string(),
                value: v,
            }),
            Some(NamedU64 {
                name: "elapsed_wall_time_ms".to_string(),
                value: duration.as_millis().min(u128::from(u64::MAX)) as u64,
            }),
        ),
        StopReason::MaxVcdBytes => (
            limits.max_vcd_bytes.map(|v| NamedU64 {
                name: "max_vcd_bytes".to_string(),
                value: v,
            }),
            Some(NamedU64 {
                name: "vcd_bytes".to_string(),
                value: vcd_bytes,
            }),
        ),
        StopReason::AssertionsPassed => (None, None),
        StopReason::MemoryViolation
        | StopReason::DecodeError
        | StopReason::Halt
        | StopReason::Exception
        | StopReason::ConfigError => (None, None),
    };

    StopReasonDetails {
        triggered_stop_condition: stop_reason.clone(),
        triggered_limit,
        observed,
    }
}

#[allow(clippy::if_same_then_else)]
#[allow(clippy::too_many_arguments)]
fn handle_load_error<C: labwired_core::Cpu>(
    args: &TestArgs,
    metrics: &Arc<labwired_core::metrics::PerformanceMetrics>,
    resolved_limits: &TestLimits,
    firmware_bytes: &[u8],
    uart_tx: &Arc<Mutex<Vec<u8>>>,
    cpu: &C,
    firmware_path: &Path,
    system_path: Option<&PathBuf>,
    e: labwired_core::SimulationError,
) -> ExitCode {
    let err_msg = format!("Simulation error during load/reset: {}", e);
    error!("{}", err_msg);
    let stop_reason_details = build_stop_reason_details(
        &StopReason::Halt,
        resolved_limits,
        0,
        metrics.get_cycles(),
        0,
        0,
        std::time::Duration::from_secs(0),
        0, // vcd_bytes
    );
    write_outputs(
        args,
        "error",
        0,
        metrics,
        StopReason::Halt,
        stop_reason_details,
        resolved_limits.clone(),
        vec![],
        firmware_bytes,
        uart_tx,
        cpu,
        firmware_path,
        system_path,
        std::time::Duration::from_secs(0),
        &None,
        &None,
        &[],
    );
    ExitCode::from(EXIT_RUNTIME_ERROR)
}

#[allow(clippy::too_many_arguments)]
fn execute_test_loop<C: labwired_core::Cpu>(
    args: &TestArgs,
    machine: &mut labwired_core::Machine<C>,
    resolved_limits: &TestLimits,
    assertions: &[TestAssertion],
    firmware_bytes: &[u8],
    uart_tx: &Arc<Mutex<Vec<u8>>>,
    metrics: &Arc<labwired_core::metrics::PerformanceMetrics>,
    firmware_path: &Path,
    system_path: Option<&PathBuf>,
    faults: &[labwired_config::FaultSpec],
    require_fault_fired: bool,
    mut fault_evidence: Vec<labwired_cli::faults::FaultEvidence>,
) -> ExitCode {
    let max_steps = resolved_limits.max_steps;
    let max_cycles = resolved_limits.max_cycles;
    let max_uart_bytes = resolved_limits.max_uart_bytes;
    let detect_stuck = resolved_limits.no_progress_steps;
    let script_wall_time_ms = resolved_limits.wall_time_ms;

    let start = std::time::Instant::now();
    let mut stop_reason = StopReason::MaxSteps;
    let mut steps_executed: u64 = 0;

    let trace_observer = if args.trace {
        let obs = Arc::new(labwired_core::trace::TraceObserver::new(args.trace_max));
        machine.observers.push(obs.clone());
        Some(obs)
    } else {
        None
    };

    let coverage_observer = if args.coverage {
        let obs = Arc::new(labwired_core::pc_coverage::PcCoverageObserver::new());
        machine.observers.push(obs.clone());
        Some(obs)
    } else {
        None
    };

    if let Some(vcd_path) = &args.vcd {
        let file = std::fs::File::create(vcd_path).expect("Failed to create VCD file");
        let observer = std::sync::Arc::new(vcd_trace::VcdObserver::new(file));
        machine.observers.push(observer);
    }

    let mut sim_error_happened = false;
    let mut prev_pc = machine.cpu.get_pc();
    let mut stuck_counter: u64 = 0;

    let batch_size = if machine.config.batch_mode_enabled
        && args.breakpoint.is_empty()
        && detect_stuck.is_none()
        // Cycle-tight GPIO-timing devices (e.g. HC-SR04 ECHO pulse) only behave
        // correctly when peripherals tick between every instruction; instruction
        // batching freezes them across the batch and the firmware measures 0.
        && !machine.bus.requires_cycle_accurate()
    {
        10000.min(max_steps)
    } else {
        1
    };

    let mut step = 0;
    while step < max_steps {
        if !args.breakpoint.is_empty() && args.breakpoint.contains(&machine.cpu.get_pc()) {
            stop_reason = StopReason::Halt;
            steps_executed = step;
            break;
        }
        if let Some(wall_time_ms) = script_wall_time_ms {
            if start.elapsed().as_millis() >= wall_time_ms as u128 {
                stop_reason = StopReason::WallTime;
                break;
            }
        }

        // Check max_cycles
        if let Some(limit) = max_cycles {
            if metrics.get_cycles() >= limit {
                stop_reason = StopReason::MaxCycles;
                break;
            }
        }

        // Check max_uart_bytes
        if let Some(limit) = max_uart_bytes {
            let current_len = uart_tx.lock().map(|g| g.len() as u64).unwrap_or(0);
            if current_len >= limit {
                stop_reason = StopReason::MaxUartBytes;
                break;
            }
        }

        let remaining = (max_steps - step) as u32;
        let current_batch = batch_size as u32;
        let to_execute = current_batch.min(remaining);

        if to_execute > 1 {
            match machine.cpu.step_batch(
                &mut machine.bus,
                &machine.observers,
                &machine.config,
                to_execute,
            ) {
                Ok(executed) => {
                    let prev_cycles = machine.total_cycles;
                    step += executed as u64;
                    steps_executed = step;
                    machine.total_cycles += executed as u64;

                    // Cycle-accurate tick propagation for batch runs
                    let tick_interval = machine.config.peripheral_tick_interval as u64;
                    let ticks_before = prev_cycles / tick_interval;
                    let ticks_after = machine.total_cycles / tick_interval;

                    for _ in ticks_before..ticks_after {
                        let (interrupts, costs) = machine.bus.tick_peripherals_fully();
                        for c in costs {
                            machine.total_cycles += c.cycles as u64;
                            if let Some(p) = machine.bus.peripherals.get(c.index) {
                                for observer in &machine.observers {
                                    observer.on_peripheral_tick(&p.name, c.cycles);
                                }
                            }
                        }
                        for irq in interrupts {
                            machine.cpu.set_exception_pending(irq);
                        }
                    }

                    // Honor a firmware-requested system reset (AIRCR
                    // SYSRESETREQ with VECTKEY) latched by the batch just
                    // executed. The single-step `else` branch gets this via
                    // `machine.step()`; the batched path must drain it
                    // explicitly or the reboot never fires.
                    if machine.drain_scb_reset_request() {
                        if let Err(e) = machine.reset() {
                            sim_error_happened = true;
                            stop_reason = match e {
                                labwired_core::SimulationError::MemoryViolation(_) => {
                                    StopReason::MemoryViolation
                                }
                                labwired_core::SimulationError::DecodeError(_) => {
                                    StopReason::DecodeError
                                }
                                labwired_core::SimulationError::Halt => StopReason::Halt,
                                labwired_core::SimulationError::SnapshotSchemaMismatch {
                                    ..
                                } => StopReason::Exception,
                                labwired_core::SimulationError::Other(_) => StopReason::Exception,
                                labwired_core::SimulationError::NotImplemented(_) => {
                                    StopReason::Exception
                                }
                                labwired_core::SimulationError::BreakpointHit(_) => {
                                    StopReason::Halt
                                }
                                labwired_core::SimulationError::ExceptionRaised { .. } => {
                                    StopReason::Exception
                                }
                            };
                            error!("Reset error at step {}: {}", step, e);
                            break;
                        }
                    }

                    if executed < to_execute {
                        // Bailed out early (e.g. exception/branch)
                        continue;
                    }
                }
                Err(e) => {
                    sim_error_happened = true;
                    stop_reason = match e {
                        labwired_core::SimulationError::MemoryViolation(_) => {
                            StopReason::MemoryViolation
                        }
                        labwired_core::SimulationError::DecodeError(_) => StopReason::DecodeError,
                        labwired_core::SimulationError::Halt => StopReason::Halt,
                        labwired_core::SimulationError::SnapshotSchemaMismatch { .. } => {
                            StopReason::Exception
                        }
                        labwired_core::SimulationError::Other(_) => StopReason::Exception,
                        labwired_core::SimulationError::NotImplemented(_) => StopReason::Exception,
                        labwired_core::SimulationError::BreakpointHit(_) => StopReason::Halt,
                        labwired_core::SimulationError::ExceptionRaised { .. } => {
                            StopReason::Exception
                        }
                    };
                    if stop_reason != StopReason::Halt {
                        error!("Simulation error at step {}: {}", step, e);
                    }
                    break;
                }
            }
        } else {
            steps_executed = step + 1;
            if let Err(e) = machine.step() {
                sim_error_happened = true;
                stop_reason = match e {
                    labwired_core::SimulationError::MemoryViolation(_) => {
                        StopReason::MemoryViolation
                    }
                    labwired_core::SimulationError::DecodeError(_) => StopReason::DecodeError,
                    labwired_core::SimulationError::Halt => StopReason::Halt,
                    labwired_core::SimulationError::SnapshotSchemaMismatch { .. } => {
                        StopReason::Exception
                    }
                    labwired_core::SimulationError::Other(_) => StopReason::Exception,
                    labwired_core::SimulationError::NotImplemented(_) => StopReason::Exception,
                    labwired_core::SimulationError::BreakpointHit(_) => StopReason::Halt,
                    labwired_core::SimulationError::ExceptionRaised { .. } => StopReason::Exception,
                };
                if stop_reason != StopReason::Halt {
                    error!("Simulation error at step {}: {}", step, e);
                }
                break;
            }
            step += 1;
        }

        // Check no_progress (PC stuck) - only if batching disabled or not possible
        if let Some(limit) = detect_stuck {
            let current_pc = machine.cpu.get_pc();
            if current_pc == prev_pc {
                stuck_counter += 1;
                if stuck_counter >= limit {
                    stop_reason = StopReason::NoProgress;
                    error!(
                        "No progress (PC stuck at {:#x}) for {} steps",
                        prev_pc, limit
                    );
                    break;
                }
            } else {
                stuck_counter = 0;
                prev_pc = current_pc;
            }
        }
    }

    let uart_text = {
        let bytes = uart_tx.lock().map(|g| g.clone()).unwrap_or_default();
        String::from_utf8_lossy(&bytes).to_string()
    };

    let mut assertion_results = Vec::new();
    let mut all_passed = true;
    let mut expected_stop_reason_matched = false;

    for assertion in assertions {
        let passed = match &assertion {
            TestAssertion::UartContains(a) => uart_text.contains(&a.uart_contains),
            TestAssertion::UartRegex(a) => simple_regex_is_match(&a.uart_regex, &uart_text),
            TestAssertion::ExpectedStopReason(a) => a.expected_stop_reason == stop_reason,
            TestAssertion::MemoryValue(a) => {
                // `size` is the value width. Accept either bytes (1/2/4) or
                // bits (8/16/32) — both name the same u8/u16/u32 reads — so a
                // natural "4 bytes" guess for a u32 RAM word works as well as
                // the historical bit-width form. Defaults to a 32-bit (u32) word.
                let size = a.memory_value.size.unwrap_or(32);
                let result = match size {
                    1 | 8 => machine
                        .bus
                        .read_u8(a.memory_value.address)
                        .map(|v| v as u32),
                    2 | 16 => machine
                        .bus
                        .read_u16(a.memory_value.address)
                        .map(|v| v as u32),
                    4 | 32 => machine.bus.read_u32(a.memory_value.address),
                    _ => {
                        error!(
                            "Unsupported memory assertion size: {} — use 1/2/4 (bytes) or 8/16/32 (bits)",
                            size
                        );
                        Err(labwired_core::SimulationError::Other("Invalid size".into()))
                    }
                };

                match result {
                    Ok(val) => {
                        let mask = a.memory_value.mask.unwrap_or(0xFFFFFFFF) as u32;
                        let expected = a.memory_value.expected_value as u32;
                        let matched = (val & mask) == (expected & mask);
                        if !matched {
                            error!(
                                "Memory assertion failed at {:#x} (size {}): expected {:#x}, got {:#x} (mask {:#x})",
                                a.memory_value.address, size, expected, val, mask
                            );
                        }
                        matched
                    }
                    Err(e) => {
                        error!(
                            "Memory assertion failed to read address {:#x} (size {}): {}",
                            a.memory_value.address, size, e
                        );
                        false
                    }
                }
            }
            TestAssertion::UdsTester(a) => {
                match evaluate_uds_tester(&machine.bus.can_uds_testers, &a.uds_tester) {
                    Ok(()) => true,
                    Err(msg) => {
                        error!("Assertion failed: {}", msg);
                        false
                    }
                }
            }
        };

        if matches!(assertion, TestAssertion::ExpectedStopReason(_)) && passed {
            expected_stop_reason_matched = true;
        }

        if !passed {
            all_passed = false;
            error!(
                "Assertion failed: {:?} (captured len={})",
                assertion,
                uart_text.len()
            );
        }

        assertion_results.push(AssertionResult {
            assertion: assertion.clone(),
            passed,
        });
    }

    let stop_requires_assertion = matches!(
        stop_reason,
        StopReason::WallTime | StopReason::MaxUartBytes | StopReason::NoProgress
    );

    let status = if !all_passed || (stop_requires_assertion && !expected_stop_reason_matched) {
        "fail"
    } else if sim_error_happened && !expected_stop_reason_matched {
        "error"
    } else {
        "pass"
    };

    let duration = start.elapsed();
    let uart_bytes = uart_tx.lock().map(|g| g.len() as u64).unwrap_or(0);
    let stop_reason_details = build_stop_reason_details(
        &stop_reason,
        resolved_limits,
        steps_executed,
        metrics.get_cycles(),
        uart_bytes,
        stuck_counter,
        duration,
        0, // vcd_bytes - will be updated below
    );
    // Finalise runtime-observed fault outcomes (e.g. missing_clock fires only
    // when the firmware actually accessed the unclocked peripheral) and enforce
    // the require_fault_fired gate: a fault that never took effect makes the run
    // invalid, not a firmware pass.
    labwired_cli::faults::finalize_fault_evidence(&machine.bus, faults, &mut fault_evidence);
    let fault_gate_failed = require_fault_fired && fault_evidence.iter().any(|e| !e.fired);
    if fault_gate_failed {
        let n = fault_evidence.iter().filter(|e| !e.fired).count();
        error!("require_fault_fired: {n} fault(s) did not fire; run is invalid");
    }

    write_outputs(
        args,
        status,
        steps_executed,
        metrics,
        stop_reason.clone(),
        stop_reason_details,
        resolved_limits.clone(),
        assertion_results,
        firmware_bytes,
        uart_tx,
        &machine.cpu,
        firmware_path,
        system_path,
        duration,
        &trace_observer,
        &coverage_observer,
        &fault_evidence,
    );

    if !all_passed
        || fault_gate_failed
        || (stop_requires_assertion && !expected_stop_reason_matched)
    {
        ExitCode::from(EXIT_ASSERT_FAIL)
    } else if sim_error_happened && !expected_stop_reason_matched {
        ExitCode::from(EXIT_RUNTIME_ERROR)
    } else {
        ExitCode::from(EXIT_PASS)
    }
}

#[allow(clippy::too_many_arguments, clippy::if_same_then_else)]
fn write_outputs<C: labwired_core::Cpu>(
    args: &TestArgs,
    status: &str,
    steps_executed: u64,
    metrics: &labwired_core::metrics::PerformanceMetrics,
    stop_reason: StopReason,
    stop_reason_details: StopReasonDetails,
    limits: TestLimits,
    assertions: Vec<AssertionResult>,
    firmware_bytes: &[u8],
    uart_tx: &Arc<Mutex<Vec<u8>>>,
    cpu: &C,
    firmware_path: &Path,
    system_path: Option<&PathBuf>,
    duration: std::time::Duration,
    trace_observer: &Option<Arc<labwired_core::trace::TraceObserver>>,
    coverage_observer: &Option<Arc<labwired_core::pc_coverage::PcCoverageObserver>>,
    fault_evidence: &[labwired_cli::faults::FaultEvidence],
) {
    let mut hasher = Sha256::new();
    hasher.update(firmware_bytes);
    let firmware_hash = format!("{:x}", hasher.finalize());

    let assertions_for_junit = assertions.clone();
    let result = TestResult {
        result_schema_version: RESULT_SCHEMA_VERSION.to_string(),
        status: status.to_string(),
        steps_executed,
        cycles: metrics.get_cycles(),
        instructions: metrics.get_instructions(),
        stop_reason,
        stop_reason_details: stop_reason_details.clone(),
        limits: limits.clone(),
        message: None,
        assertions,
        cpu_state: Some(cpu.snapshot()),
        firmware_hash,
        config: TestConfig {
            firmware: firmware_path.to_path_buf(),
            system: system_path.cloned(),
            script: args.script.clone(),
        },
    };

    if let Some(output_dir) = &args.output_dir {
        if let Err(e) = std::fs::create_dir_all(output_dir) {
            error!("Failed to create output directory {:?}: {}", output_dir, e);
        } else {
            // result.json
            let result_path = output_dir.join("result.json");
            match std::fs::File::create(&result_path) {
                Ok(f) => {
                    if let Err(e) = serde_json::to_writer_pretty(f, &result) {
                        error!("Failed to write result.json: {}", e);
                    }
                }
                Err(e) => error!("Failed to create result.json: {}", e),
            }

            // trace.json
            if let Some(obs) = trace_observer {
                let trace_path = output_dir.join("trace.json");
                let traces = obs.take_traces();
                match std::fs::File::create(&trace_path) {
                    Ok(f) => {
                        if let Err(e) = serde_json::to_writer_pretty(f, &traces) {
                            error!("Failed to write trace.json: {}", e);
                        }
                    }
                    Err(e) => error!("Failed to create trace.json: {}", e),
                }
            }

            // fault-evidence.json (per-fault verdicts; also folded into the manifest)
            if !fault_evidence.is_empty() {
                let fault_path = output_dir.join("fault-evidence.json");
                match std::fs::File::create(&fault_path) {
                    Ok(f) => {
                        if let Err(e) = serde_json::to_writer_pretty(f, fault_evidence) {
                            error!("Failed to write fault-evidence.json: {}", e);
                        }
                    }
                    Err(e) => error!("Failed to create fault-evidence.json: {}", e),
                }
            }

            // coverage.info (LCOV) + coverage.json
            let mut coverage_summary: Option<labwired_cli::manifest::CoverageSummary> = None;
            if let Some(cov) = coverage_observer {
                match labwired_loader::SymbolProvider::new(firmware_path) {
                    Ok(symbols) => {
                        let mut report = labwired_cli::pc_coverage_report::CoverageReport::build(
                            symbols.statement_rows(),
                            |addr| cov.was_executed(addr as u32),
                        );
                        // Resolve each observed branch site to its source line.
                        let branch_cov = cov
                            .branch_sites()
                            .into_iter()
                            .filter_map(|(src, counts)| {
                                symbols.lookup(src as u64).and_then(|loc| {
                                    loc.line.map(|line| {
                                        // statement_rows uses the line-program
                                        // file basename; lookup() returns the
                                        // full path. Normalise to the basename
                                        // so branches attach to the right SF.
                                        let file = loc
                                            .file
                                            .rsplit('/')
                                            .next()
                                            .unwrap_or(&loc.file)
                                            .to_string();
                                        labwired_cli::pc_coverage_report::BranchCoverage {
                                            file,
                                            line,
                                            taken: counts.taken,
                                            not_taken: counts.not_taken,
                                        }
                                    })
                                })
                            })
                            .collect();
                        report.set_branches(branch_cov);
                        let info_path = output_dir.join("coverage.info");
                        if let Err(e) = std::fs::write(&info_path, report.to_lcov()) {
                            error!("Failed to write coverage.info: {}", e);
                        }
                        let cov_json_path = output_dir.join("coverage.json");
                        match std::fs::File::create(&cov_json_path) {
                            Ok(f) => {
                                if let Err(e) = serde_json::to_writer_pretty(f, &report) {
                                    error!("Failed to write coverage.json: {}", e);
                                }
                            }
                            Err(e) => error!("Failed to create coverage.json: {}", e),
                        }
                        info!(
                            "Coverage: {}/{} statements ({:.1}%), {}/{} branches ({:.1}%)",
                            report.covered_statements,
                            report.total_statements,
                            report.statement_percent(),
                            report.covered_branches,
                            report.total_branches,
                            report.branch_percent()
                        );
                        coverage_summary = Some(labwired_cli::manifest::CoverageSummary {
                            statements_total: report.total_statements,
                            statements_covered: report.covered_statements,
                            branches_total: report.total_branches,
                            branches_covered: report.covered_branches,
                        });
                    }
                    Err(e) => error!("Failed to load symbols for coverage: {}", e),
                }
            }

            // run-manifest.json (signable, reproducible)
            if args.run_manifest {
                use labwired_cli::manifest;
                // Use the file basename, not the absolute path, so the digest
                // depends only on file contents and is reproducible across
                // machines with different checkout locations.
                let basename = |p: &Path| -> String {
                    p.file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.display().to_string())
                };
                let hash_file = |p: &Path| -> manifest::HashedFile {
                    let sha256 = std::fs::read(p)
                        .map(|b| manifest::sha256_hex(&b))
                        .unwrap_or_default();
                    manifest::HashedFile {
                        path: basename(p),
                        sha256,
                    }
                };
                let mut configs = vec![hash_file(&args.script)];
                if let Some(sys) = system_path {
                    configs.push(hash_file(sys));
                }
                let mut man = manifest::RunManifest {
                    manifest_schema_version: manifest::MANIFEST_SCHEMA_VERSION.to_string(),
                    engine_version: env!("CARGO_PKG_VERSION").to_string(),
                    seed: 0,
                    nondeterminism: "none".to_string(),
                    firmware: manifest::HashedFile {
                        path: basename(firmware_path),
                        sha256: result.firmware_hash.clone(),
                    },
                    configs,
                    results: manifest::ManifestResults {
                        status: status.to_string(),
                        stop_reason: format!("{:?}", result.stop_reason),
                        steps_executed: result.steps_executed,
                        cycles: result.cycles,
                        instructions: result.instructions,
                        assertions: assertions_for_junit
                            .iter()
                            .map(|a| manifest::AssertionOutcome {
                                assertion: format!("{:?}", a.assertion),
                                passed: a.passed,
                            })
                            .collect(),
                        cpu_state_digest: manifest::digest_value(&cpu.snapshot()),
                    },
                    coverage: coverage_summary.clone(),
                    fault_injections: fault_evidence.to_vec(),
                    digest: String::new(),
                };
                man.finalize_digest();
                let manifest_path = output_dir.join("run-manifest.json");
                match std::fs::File::create(&manifest_path) {
                    Ok(f) => {
                        if let Err(e) = serde_json::to_writer_pretty(f, &man) {
                            error!("Failed to write run-manifest.json: {}", e);
                        }
                    }
                    Err(e) => error!("Failed to create run-manifest.json: {}", e),
                }
                info!("Run manifest digest: {}", man.digest);
            }

            // result.json handles cpu generically now
            let snapshot_path = output_dir.join("snapshot.json");
            let snapshot = Snapshot::Standard {
                cpu: cpu.snapshot(),
                steps_executed,
                cycles: result.cycles,
                instructions: result.instructions,
                stop_reason: result.stop_reason.clone(),
                stop_reason_details: result.stop_reason_details.clone(),
                limits: result.limits.clone(),
                firmware_hash: result.firmware_hash.clone(),
                config: TestConfig {
                    firmware: result.config.firmware.clone(),
                    system: result.config.system.clone(),
                    script: result.config.script.clone(),
                },
            };
            match std::fs::File::create(&snapshot_path) {
                Ok(f) => {
                    if let Err(e) = serde_json::to_writer_pretty(f, &snapshot) {
                        error!("Failed to write snapshot.json: {}", e);
                    }
                }
                Err(e) => error!("Failed to create snapshot.json: {}", e),
            }

            // uart.log
            let uart_path = output_dir.join("uart.log");
            let bytes = uart_tx.lock().map(|g| g.clone()).unwrap_or_default();
            if let Err(e) = std::fs::write(&uart_path, bytes) {
                error!("Failed to write uart.log: {}", e);
            }

            // junit.xml
            let junit_path = output_dir.join("junit.xml");
            if let Err(e) = write_junit_xml(
                &junit_path,
                status,
                duration,
                &result.stop_reason,
                &assertions_for_junit,
                &result.firmware_hash,
                &result.config,
                result.message.as_deref(),
                result.steps_executed,
                result.cycles,
                result.instructions,
                &result.limits,
                &result.stop_reason_details,
            ) {
                error!("Failed to write junit.xml: {}", e);
            }
        }
    }

    if let Some(junit_path) = &args.junit {
        if let Some(parent) = junit_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = write_junit_xml(
            junit_path,
            status,
            duration,
            &result.stop_reason,
            &assertions_for_junit,
            &result.firmware_hash,
            &result.config,
            result.message.as_deref(),
            result.steps_executed,
            result.cycles,
            result.instructions,
            &result.limits,
            &result.stop_reason_details,
        ) {
            error!("Failed to write JUnit report {:?}: {}", junit_path, e);
        }
    }
}

pub(crate) fn write_config_error_outputs(
    args: &TestArgs,
    firmware_path: Option<&PathBuf>,
    system_path: Option<&PathBuf>,
    firmware_bytes: Option<&[u8]>,
    limits: Option<&TestLimits>,
    message: String,
) {
    // Best-effort: the caller requests artifacts, but directory creation / writes may fail.
    let firmware_hash = match firmware_bytes {
        Some(bytes) => {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            format!("{:x}", hasher.finalize())
        }
        None => String::new(),
    };

    let resolved_limits = limits.cloned().unwrap_or(TestLimits {
        max_steps: 0,
        max_cycles: None,
        max_uart_bytes: None,
        no_progress_steps: None,
        wall_time_ms: None,
        max_vcd_bytes: None,
        stop_when_assertions_pass: false,
    });

    let stop_reason = StopReason::ConfigError;
    let stop_reason_details = build_stop_reason_details(
        &stop_reason,
        &resolved_limits,
        0,
        0,
        0,
        0,
        std::time::Duration::from_secs(0),
        0, // vcd_bytes
    );

    let result = TestResult {
        result_schema_version: RESULT_SCHEMA_VERSION.to_string(),
        status: "error".to_string(),
        steps_executed: 0,
        cycles: 0,
        instructions: 0,
        stop_reason,
        stop_reason_details: stop_reason_details.clone(),
        limits: resolved_limits.clone(),
        message: Some(message.clone()),
        assertions: vec![],
        cpu_state: None,
        firmware_hash,
        config: TestConfig {
            firmware: firmware_path.cloned().unwrap_or_default(),
            system: system_path.cloned(),
            script: args.script.clone(),
        },
    };

    if let Some(output_dir) = &args.output_dir {
        if let Err(e) = std::fs::create_dir_all(output_dir) {
            error!("Failed to create output directory {:?}: {}", output_dir, e);
        } else {
            let result_path = output_dir.join("result.json");
            match std::fs::File::create(&result_path) {
                Ok(f) => {
                    if let Err(e) = serde_json::to_writer_pretty(f, &result) {
                        error!("Failed to write result.json: {}", e);
                    }
                }
                Err(e) => error!("Failed to create result.json: {}", e),
            }

            let snapshot_path = output_dir.join("snapshot.json");
            let snapshot = Snapshot::ConfigError {
                message: message.clone(),
                stop_reason_details: result.stop_reason_details.clone(),
                limits: result.limits.clone(),
                config: TestConfig {
                    firmware: result.config.firmware.clone(),
                    system: result.config.system.clone(),
                    script: result.config.script.clone(),
                },
            };
            match std::fs::File::create(&snapshot_path) {
                Ok(f) => {
                    if let Err(e) = serde_json::to_writer_pretty(f, &snapshot) {
                        error!("Failed to write snapshot.json: {}", e);
                    }
                }
                Err(e) => error!("Failed to create snapshot.json: {}", e),
            }

            let uart_path = output_dir.join("uart.log");
            if let Err(e) = std::fs::write(&uart_path, b"") {
                error!("Failed to write uart.log: {}", e);
            }

            let junit_path = output_dir.join("junit.xml");
            if let Err(e) = write_junit_xml(
                &junit_path,
                "error",
                std::time::Duration::from_secs(0),
                &result.stop_reason,
                &[],
                &result.firmware_hash,
                &result.config,
                result.message.as_deref(),
                0,
                0,
                0,
                &result.limits,
                &result.stop_reason_details,
            ) {
                error!("Failed to write junit.xml: {}", e);
            }
        }
    }

    if let Some(junit_path) = &args.junit {
        if let Some(parent) = junit_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = write_junit_xml(
            junit_path,
            "error",
            std::time::Duration::from_secs(0),
            &result.stop_reason,
            &[],
            &result.firmware_hash,
            &result.config,
            result.message.as_deref(),
            0,
            0,
            0,
            &result.limits,
            &result.stop_reason_details,
        ) {
            error!("Failed to write JUnit report {:?}: {}", junit_path, e);
        }
    }
}

fn resolve_script_path(script_path: &Path, value: &str) -> PathBuf {
    let p = PathBuf::from(value);
    if p.is_absolute() {
        return p;
    }
    script_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(p)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn write_junit_xml(
    path: &Path,
    status: &str,
    duration: std::time::Duration,
    stop_reason: &StopReason,
    assertions: &[AssertionResult],
    firmware_hash: &str,
    config: &TestConfig,
    message: Option<&str>,
    steps_executed: u64,
    cycles: u64,
    instructions: u64,
    limits: &TestLimits,
    stop_reason_details: &StopReasonDetails,
) -> std::io::Result<()> {
    let any_assertion_failed = assertions.iter().any(|a| !a.passed);
    let any_expected_stop_reason_matched = assertions
        .iter()
        .any(|a| matches!(a.assertion, TestAssertion::ExpectedStopReason(_)) && a.passed);
    let stop_requires_assertion = matches!(
        stop_reason,
        StopReason::WallTime | StopReason::MaxUartBytes | StopReason::NoProgress
    );

    let mut details = String::new();
    details.push_str(&format!(
        "result_schema_version={}\n",
        RESULT_SCHEMA_VERSION
    ));
    details.push_str(&format!("stop_reason={:?}\n", stop_reason));
    if let Some(msg) = message {
        details.push_str(&format!("message={}\n", msg));
    }
    details.push_str(&format!(
        "stop_reason_details.triggered_stop_condition={:?}\n",
        stop_reason_details.triggered_stop_condition
    ));
    if let Some(t) = &stop_reason_details.triggered_limit {
        details.push_str(&format!(
            "stop_reason_details.triggered_limit.{}={}\n",
            t.name, t.value
        ));
    }
    if let Some(o) = &stop_reason_details.observed {
        details.push_str(&format!(
            "stop_reason_details.observed.{}={}\n",
            o.name, o.value
        ));
    }
    details.push_str(&format!("steps_executed={}\n", steps_executed));
    details.push_str(&format!("cycles={}\n", cycles));
    details.push_str(&format!("instructions={}\n", instructions));
    details.push_str("limits:\n");
    details.push_str(&format!("  - max_steps={}\n", limits.max_steps));
    if let Some(v) = limits.max_cycles {
        details.push_str(&format!("  - max_cycles={}\n", v));
    }
    if let Some(v) = limits.max_uart_bytes {
        details.push_str(&format!("  - max_uart_bytes={}\n", v));
    }
    if let Some(v) = limits.no_progress_steps {
        details.push_str(&format!("  - no_progress_steps={}\n", v));
    }
    if let Some(v) = limits.wall_time_ms {
        details.push_str(&format!("  - wall_time_ms={}\n", v));
    }
    details.push_str(&format!("firmware_hash={}\n", firmware_hash));
    details.push_str(&format!("firmware={}\n", config.firmware.display()));
    if let Some(sys) = &config.system {
        details.push_str(&format!("system={}\n", sys.display()));
    }
    details.push_str(&format!("script={}\n", config.script.display()));
    if !assertions.is_empty() {
        details.push_str("assertions:\n");
        for a in assertions {
            details.push_str(&format!("  - {:?}: {}\n", a.assertion, a.passed));
        }
    }

    let time_secs = duration.as_secs_f64();

    let mut xml = String::new();
    xml.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    xml.push('\n');
    let mut tests: u64 = 0;
    let mut failures: u64 = 0;
    let mut errors: u64 = 0;

    let mut testcases = String::new();

    // A top-level "run" testcase captures non-assertion failures (e.g. stop condition without expected_stop_reason)
    // and runtime errors.
    tests += 1;
    testcases.push_str(&format!(
        "  <testcase classname=\"labwired\" name=\"run\" time=\"{:.6}\">\n",
        time_secs
    ));
    if status == "error" {
        let err_type = if *stop_reason == StopReason::ConfigError {
            "config error"
        } else {
            "runtime error"
        };
        errors += 1;
        testcases.push_str(&format!(
            "    <error message=\"{}\">{}</error>\n",
            xml_escape(err_type),
            xml_escape(&details)
        ));
    } else if status == "fail" && stop_requires_assertion && !any_expected_stop_reason_matched {
        failures += 1;
        testcases.push_str(&format!(
            "    <failure message=\"{}\">{}</failure>\n",
            xml_escape("stop condition requires expected_stop_reason assertion"),
            xml_escape(&details)
        ));
    } else if status == "fail" && (!any_assertion_failed) {
        failures += 1;
        testcases.push_str(&format!(
            "    <failure message=\"{}\">{}</failure>\n",
            xml_escape("failure"),
            xml_escape(&details)
        ));
    }
    testcases.push_str("  </testcase>\n");

    // One testcase per assertion so CI UIs show exactly which assertion failed.
    for (idx, a) in assertions.iter().enumerate() {
        tests += 1;
        let name = format!(
            "assertion {}: {}",
            idx + 1,
            assertion_short_name(&a.assertion)
        );
        testcases.push_str(&format!(
            "  <testcase classname=\"labwired\" name=\"{}\" time=\"0.000000\">\n",
            xml_escape(&name)
        ));
        if !a.passed {
            failures += 1;
            testcases.push_str(&format!(
                "    <failure message=\"assertion failed\">{}</failure>\n",
                xml_escape(&format!("{}\n\n{}", name, details))
            ));
        }
        testcases.push_str("  </testcase>\n");
    }

    xml.push_str(&format!(
        r#"<testsuite name="labwired" tests="{}" failures="{}" errors="{}" time="{:.6}">"#,
        tests, failures, errors, time_secs
    ));
    xml.push('\n');
    xml.push_str("  <properties>\n");
    xml.push_str(&format!(
        "    <property name=\"result_schema_version\" value=\"{}\"/>\n",
        xml_escape(RESULT_SCHEMA_VERSION)
    ));
    xml.push_str(&format!(
        "    <property name=\"stop_reason\" value=\"{}\"/>\n",
        xml_escape(&format!("{:?}", stop_reason))
    ));
    xml.push_str(&format!(
        "    <property name=\"firmware_hash\" value=\"{}\"/>\n",
        xml_escape(firmware_hash)
    ));
    xml.push_str("  </properties>\n");
    xml.push_str(&testcases);
    xml.push_str("</testsuite>\n");

    std::fs::write(path, xml)
}

fn assertion_short_name(assertion: &TestAssertion) -> String {
    const MAX_LEN: usize = 120;
    let s = match assertion {
        TestAssertion::UartContains(a) => format!("uart_contains: {}", a.uart_contains),
        TestAssertion::UartRegex(a) => format!("uart_regex: {}", a.uart_regex),
        TestAssertion::ExpectedStopReason(a) => {
            format!("expected_stop_reason: {:?}", a.expected_stop_reason)
        }
        TestAssertion::MemoryValue(a) => format!(
            "memory_value: @{:#x}={:#x}",
            a.memory_value.address, a.memory_value.expected_value
        ),
        TestAssertion::UdsTester(a) => {
            format!(
                "uds_tester: {} result={:?}",
                a.uds_tester.id, a.uds_tester.result
            )
        }
    };

    if s.len() <= MAX_LEN {
        return s;
    }

    let mut truncated = s.chars().take(MAX_LEN - 1).collect::<String>();
    truncated.push('…');
    truncated
}

/// Returns `Ok(())` if the named tester ended in `Done`; `Err(message)` otherwise.
pub(crate) fn evaluate_uds_tester(
    testers: &[labwired_core::bus::CanUdsTester],
    details: &UdsTesterDetails,
) -> Result<(), String> {
    match testers.iter().find(|t| t.id == details.id) {
        None => Err(format!("tester '{}': not found", details.id)),
        Some(t) => {
            if t.state == labwired_core::bus::CanUdsTesterState::Done {
                Ok(())
            } else {
                let reason = t.failure.as_deref().unwrap_or("not completed").to_string();
                Err(format!("tester '{}': {}", details.id, reason))
            }
        }
    }
}

// Minimal regex matcher supporting: '^' anchor, '$' anchor, '.' and '*' (Kleene star).
// This is intentionally small to avoid introducing new deps; it does not implement full PCRE/Rust regex.
pub(crate) fn simple_regex_is_match(pattern: &str, text: &str) -> bool {
    fn char_eq(pat: char, ch: char) -> bool {
        pat == '.' || pat == ch
    }

    fn match_here(pat: &[char], text: &[char]) -> bool {
        if pat.is_empty() {
            return true;
        }
        if pat.len() >= 2 && pat[1] == '*' {
            return match_star(pat[0], &pat[2..], text);
        }
        if pat[0] == '$' && pat.len() == 1 {
            return text.is_empty();
        }
        if !text.is_empty() && char_eq(pat[0], text[0]) {
            return match_here(&pat[1..], &text[1..]);
        }
        false
    }

    fn match_star(ch: char, pat: &[char], text: &[char]) -> bool {
        let mut i = 0;
        loop {
            if match_here(pat, &text[i..]) {
                return true;
            }
            if i >= text.len() {
                return false;
            }
            if !char_eq(ch, text[i]) {
                return false;
            }
            i += 1;
        }
    }

    let pat_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();

    if pat_chars.first().copied() == Some('^') {
        return match_here(&pat_chars[1..], &text_chars);
    }

    for start in 0..=text_chars.len() {
        if match_here(&pat_chars, &text_chars[start..]) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::UdsTesterDetails;
    use labwired_config::UdsTesterResult;
    use labwired_core::bus::{CanUdsTester, CanUdsTesterState};

    fn make_tester(id: &str, state: CanUdsTesterState, failure: Option<&str>) -> CanUdsTester {
        let mut t = CanUdsTester::new(id.to_string(), "bxcan1".to_string());
        t.state = state;
        t.failure = failure.map(|s| s.to_string());
        t
    }

    #[test]
    fn evaluate_uds_tester_done_passes() {
        let testers = vec![make_tester("my-tester", CanUdsTesterState::Done, None)];
        let details = UdsTesterDetails {
            id: "my-tester".to_string(),
            result: UdsTesterResult::Done,
        };
        assert!(evaluate_uds_tester(&testers, &details).is_ok());
    }

    #[test]
    fn evaluate_uds_tester_failed_returns_err_with_failure_text() {
        let testers = vec![make_tester(
            "my-tester",
            CanUdsTesterState::Failed,
            Some("step 0: unexpected response 0x7F"),
        )];
        let details = UdsTesterDetails {
            id: "my-tester".to_string(),
            result: UdsTesterResult::Done,
        };
        let err = evaluate_uds_tester(&testers, &details).unwrap_err();
        assert!(err.contains("my-tester"), "missing id in: {err}");
        assert!(
            err.contains("step 0: unexpected response 0x7F"),
            "missing failure text in: {err}"
        );
    }

    #[test]
    fn evaluate_uds_tester_unknown_id_returns_err() {
        let testers = vec![make_tester("other", CanUdsTesterState::Done, None)];
        let details = UdsTesterDetails {
            id: "ghost-tester".to_string(),
            result: UdsTesterResult::Done,
        };
        let err = evaluate_uds_tester(&testers, &details).unwrap_err();
        assert!(err.contains("ghost-tester"), "missing id in: {err}");
    }
}
