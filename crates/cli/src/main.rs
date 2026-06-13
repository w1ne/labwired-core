// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
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

use labwired_config::{load_test_script, LoadedTestScript, StopReason, TestAssertion, TestLimits};

const EXIT_PASS: u8 = 0;
const EXIT_ASSERT_FAIL: u8 = 1;
const EXIT_CONFIG_ERROR: u8 = 2;
const EXIT_RUNTIME_ERROR: u8 = 3;

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
fn emit_error(
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
            .with_max_level(tracing::Level::DEBUG)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .init();
    }

    match cli.command {
        Some(Commands::Test(args)) => run_test(args),
        Some(Commands::Machine(args)) => run_machine(args),
        Some(Commands::Asset(args)) => run_asset(args),
        Some(Commands::Run(args)) => run_firmware(args),
        Some(Commands::Snapshot(args)) => run_snapshot(args),
        Some(Commands::Coverage(args)) => run_coverage(args),
        Some(Commands::Tier1Matrix(args)) => run_tier1_matrix(args),
        Some(Commands::Fuzz(args)) => run_fuzz(args),
        None => run_interactive(cli),
    }
}

/// Fast-boot path for RISC-V chip descriptors (e.g. ESP32-C3).
///
/// Loads the chip's declarative peripherals from the YAML via
/// `SystemBus::from_config`, creates a RISC-V CPU, loads the ELF at its entry
/// point, and runs the step loop up to `max_steps`. UART output is echoed to
/// stdout, which is how the Tier-1 harness reads protocol lines
/// (`TIER1 <class> PASS|FAIL` / `TIER1 done`).
fn run_firmware_riscv(args: RunArgs, _chip_yaml: String) -> ExitCode {
    use labwired_core::bus::SystemBus;

    let chip = match labwired_config::ChipDescriptor::from_file(&args.chip) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: cannot parse chip YAML {:?}: {e}", args.chip);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Minimal system manifest: no external devices, no extra peripherals.
    // All peripherals come from the chip descriptor.
    let manifest = labwired_config::SystemManifest {
        schema_version: "1.0".to_string(),
        name: chip.name.clone(),
        chip: args.chip.to_string_lossy().into_owned(),
        memory_overrides: Default::default(),
        external_devices: vec![],
        board_io: vec![],
        peripherals: vec![],
        walk_deleted: false,
    };

    let mut bus = match SystemBus::from_config(&chip, &manifest) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: failed to build system bus: {e:#}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let program = match labwired_loader::load_elf(&args.firmware) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: cannot load ELF {:?}: {e}", args.firmware);
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }
    };

    let mut machine = if args.rom_boot {
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
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        };
        let flash_bytes = match std::fs::read(&flash_path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("error: cannot read flash image {flash_path}: {e}");
                return ExitCode::from(EXIT_RUNTIME_ERROR);
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
                labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(
                    backing.clone(),
                ),
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
                labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(
                    backing.clone(),
                ),
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
        let mut cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
        cpu.set_pc(0x4000_0000);
        // Disable the internal CLINT timer: the C3 has no standard MTIP — its
        // 31 interrupt lines (incl. line 7) are ESP matrix lines, so a
        // self-pending MTIP would collide. mtimecmp=MAX keeps mip bit7 clear.
        cpu.mtimecmp = u64::MAX;
        labwired_core::Machine::new(cpu, bus)
    } else {
        let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
        let mut machine = labwired_core::Machine::new(cpu, bus);
        if let Err(e) = machine.load_firmware(&program) {
            eprintln!("error: firmware load failed: {e}");
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }

        // Fast-boot skips the ROM/2nd-stage bootloader that normally sets the
        // stack pointer before jumping to the app, so SP=0 and the app's first
        // prologue store faults near 0xffffffff. Seed SP at the top of DRAM
        // (16-byte aligned, RISC-V ABI) so real IDF apps can boot.
        let sp_top =
            (chip.ram.base + labwired_config::parse_size(&chip.ram.size).unwrap_or(0)) as u32;
        machine.cpu.set_sp(sp_top & !0xF);
        machine
    };

    let break_at: Vec<u32> = args
        .break_at
        .iter()
        .filter_map(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .collect();
    let mut break_hit = vec![false; break_at.len()];
    let limit = args.max_steps.unwrap_or(u64::MAX);
    // Recent-PC trail for boot debugging — only maintained when --break-at is in
    // use, so the normal hot loop pays nothing.
    let debug = !break_at.is_empty();
    // Executable address windows for C3 (ROM, IRAM, flash IROM XIP). A PC
    // outside all of these means a bad jump (truncated pointer, garbage return
    // address); trap it immediately so the trail still shows the jumper instead
    // of 64 instructions of slide through unmapped memory.
    let is_exec = |pc: u32| -> bool {
        (0x4000_0000..0x4006_0000).contains(&pc)      // mask ROM
            || (0x4037_0000..0x403E_0000).contains(&pc) // IRAM
            || (0x4200_0000..0x4400_0000).contains(&pc) // flash IROM (XIP)
    };
    let trail_cap = 600;
    let mut recent = std::collections::VecDeque::with_capacity(trail_cap + 1);
    for i in 0..limit {
        let pc = machine.cpu.get_pc();
        if debug {
            if recent.len() == trail_cap {
                recent.pop_front();
            }
            recent.push_back(pc);
            if i > 0 && !is_exec(pc) {
                let c = &machine.cpu;
                eprintln!(
                    "[badjump] step {i}: PC entered non-exec region {pc:#010x} \
                     ra={:#010x} sp={:#010x} a0={:#010x}",
                    c.x[1], c.x[2], c.x[10]
                );
                let trail: Vec<String> = recent.iter().map(|p| format!("{p:#010x}")).collect();
                eprintln!("[trail] {}", trail.join(" -> "));
                break;
            }
        }
        if let Some(bi) = break_at.iter().position(|&b| b == pc) {
            if !break_hit[bi] {
                break_hit[bi] = true;
                let c = &machine.cpu;
                eprintln!(
                    "[break] step {i} pc={pc:#010x} ra={:#010x} sp={:#010x} a0={:#010x}",
                    c.x[1], c.x[2], c.x[10]
                );
            }
        }
        if debug && i > 0 && i % 20_000_000 == 0 {
            eprintln!("[progress] step {i} pc={pc:#010x}");
        }
        if let Err(e) = machine.step() {
            // Surface the halt (was a silent debug log): the fault PC + reason is
            // the key signal when bringing real firmware up on the sim.
            tracing::debug!("labwired-riscv: step {i} pc={pc:#010x} halt: {e}");
            if !break_at.is_empty() {
                eprintln!("[halt] step {i} pc={pc:#010x} err={e}");
                let trail: Vec<String> = recent.iter().map(|p| format!("{p:#010x}")).collect();
                eprintln!("[trail] {}", trail.join(" -> "));
            }
            break;
        }
    }

    ExitCode::from(EXIT_PASS)
}

/// Fast-boot an ESP32-classic (LX6) ELF and run the step loop.
///
/// Mirrors the pattern in `crates/core/tests/e2e_esp32_epaper.rs`:
/// `configure_xtensa_esp32` + ELF load + set_pc(entry) + set_sp + step loop.
/// UART0 (0x3FF4_0000, STM32F1 layout, echo_stdout=true) carries the TIER1
/// protocol lines to the tier1 harness via stdout.
fn run_firmware_esp32(args: &RunArgs) -> ExitCode {
    use labwired_core::bus::SystemBus;
    use labwired_core::system::xtensa::configure_xtensa_esp32;
    use labwired_core::SimulationError;

    // Read the firmware ELF.
    let elf_bytes = match std::fs::read(&args.firmware) {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "error: cannot read firmware ELF at {:?}: {e}",
                args.firmware
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let image = match labwired_loader::load_elf_bytes(&elf_bytes) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("error: failed to parse ELF: {e}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let mut bus = SystemBus::new();
    let mut cpu = configure_xtensa_esp32(&mut bus);

    // Load ELF segments into bus memory (IRAM/DRAM/flash windows).
    for segment in &image.segments {
        for (i, &byte) in segment.data.iter().enumerate() {
            let addr = segment.start_addr + i as u64;
            let _ = bus.write_u8(addr, byte);
        }
    }

    // Set PC to ELF entry and seed SP at top of SRAM1 (post-BROM default on
    // real silicon; see e2e_external_arduino_esp32_in_sim for the rationale).
    // CHEAT(SKIP): bypasses the boot ROM and hand-seeds PC/SP. See FIDELITY.md §C.
    cpu.set_pc(image.entry_point as u32);
    cpu.set_sp(0x3FFE_0000);
    // Post-bootloader PS state: WOE=1 (windowed ABI), INTLEVEL=0, EXCM=0.
    cpu.ps = labwired_core::cpu::xtensa_regs::Ps::from_raw(1 << 18);

    let limit = args.max_steps.unwrap_or(u64::MAX);
    let observers: Vec<std::sync::Arc<dyn labwired_core::SimulationObserver>> = Vec::new();
    let config = labwired_core::SimulationConfig::default();
    let mut steps = 0u64;

    while steps < limit {
        match cpu.step(&mut bus, &observers, &config) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(_)) => break,
            Err(SimulationError::ExceptionRaised { cause, pc }) => {
                eprintln!("labwired-cli run (esp32): ExceptionRaised cause={cause} at 0x{pc:08x}");
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
            Err(e) => {
                eprintln!(
                    "labwired-cli run (esp32): simulator error at pc=0x{:08x}: {e}",
                    cpu.get_pc(),
                );
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
        }
        bus.tick_peripherals_with_costs();
        steps += 1;
    }
    eprintln!(
        "labwired-cli run (esp32): reached --max-steps {limit}; pc=0x{:08x}",
        cpu.get_pc(),
    );
    ExitCode::from(EXIT_PASS)
}

fn run_firmware(args: RunArgs) -> ExitCode {
    use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
    use labwired_core::bus::SystemBus;
    use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts};
    use labwired_core::SimulationError;

    // Read the chip YAML to validate the chip family.
    let chip_yaml = match std::fs::read_to_string(&args.chip) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read chip YAML at {:?}: {e}", args.chip);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // ARM fast-boot path: parse the chip YAML, build the bus, run the firmware
    // through a Cortex-M machine, and stream UART bytes to stdout so the
    // TIER1 protocol lines are visible to the caller.
    if chip_yaml.contains("arch: \"arm\"") || chip_yaml.contains("arch: arm") {
        return run_firmware_arm(&args, &chip_yaml);
    }

    // RISC-V fast-boot path: load peripherals from the chip YAML and run the
    // RV32I core. This is the path used by Tier-1 fixtures for RISC-V chips
    // (e.g. ESP32-C3) which cannot go through the Xtensa boot sequence.
    if chip_yaml.contains("arch: \"riscv\"") || chip_yaml.contains("arch: riscv") {
        return run_firmware_riscv(args, chip_yaml);
    }

    // Classic ESP32 (Xtensa LX6) fast-boot path.
    if chip_yaml.contains("xtensa-lx6") {
        return run_firmware_esp32(&args);
    }

    if !chip_yaml.contains("xtensa-lx7") {
        eprintln!(
            "error: chip {:?} does not look like an Xtensa LX7 chip; \
             only ESP32-S3 is supported by `labwired run`",
            args.chip,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    // Read the firmware ELF.
    let elf_bytes = match std::fs::read(&args.firmware) {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "error: cannot read firmware ELF at {:?}: {e}",
                args.firmware
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Wire the bus + CPU.
    let mut bus = SystemBus::new();
    let opts = Esp32s3Opts::default();
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
    let boot_mode = wiring.boot_mode; // Copy before cpu is moved out of wiring

    // Install default tracing GPIO observer.
    wiring.add_gpio_observer(
        &mut bus,
        std::sync::Arc::new(crate::gpio_observer::TracingGpioObserver::new()),
    );

    // Optional JSON-line GPIO trace.
    if let Some(path) = &args.gpio_trace {
        match crate::gpio_observer::JsonGpioObserver::new(path) {
            Ok(obs) => {
                wiring.add_gpio_observer(&mut bus, std::sync::Arc::new(obs));
                eprintln!("labwired-cli run: gpio trace -> {:?}", path);
            }
            Err(e) => {
                eprintln!("error: cannot open gpio-trace file {:?}: {e}", path);
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        }
    }

    let mut cpu = wiring.cpu;

    // Dual-core (SMP): the APP_CPU (core 1). Created halted at the ROM reset
    // vector; released when the PRO_CPU clears CORE_1_RESETING (real hardware
    // edge, signalled via APPCPU_RESET_RELEASED). The APP_CPU then boots the
    // real ROM exactly like silicon — no firmware-symbol hooks. --rom-boot only.
    let mut cpu1: Option<labwired_core::cpu::xtensa_lx7::XtensaLx7> = None;
    let mut appcpu_started = false;

    if args.rom_boot {
        // ── Faithful boot: run the real ROM from the reset vector ──────────
        // The CPU resets to 0x40000400 (BROM reset vector). With the real ROM
        // (auto-provisioned, or pinned via LABWIRED_ESP32S3_ROM) and the flash image behind the SPI-flash
        // controller (LABWIRED_ESP32S3_FLASH), the chip's own boot ROM loads
        // the 2nd-stage bootloader + app and jumps to it — same path as
        // silicon. No fast_boot, no ELF pre-load, no handshake pre-paint.
        let _ = &elf_bytes; // ELF used only for symbol/diagnostic context
                            // --rom-boot runs the genuine boot ROM. The ROM is auto-provisioned from
                            // the installed toolchain by configure_xtensa_esp32s3 (or pinned via
                            // LABWIRED_ESP32S3_ROM/_DROM); we only need the flash image here. If no
                            // real ROM was resolved we are in harness mode, where --rom-boot is
                            // meaningless — fail clearly.
        if boot_mode != Esp32s3BootMode::Faithful {
            eprintln!(
                "error: --rom-boot needs the real ESP32-S3 boot ROM, but none was found. \
                 Install the ESP toolchain (PlatformIO/ESP-IDF) or set LABWIRED_ESP32S3_ROM_ELF \
                 (or pin LABWIRED_ESP32S3_ROM/_DROM)."
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
        if std::env::var("LABWIRED_ESP32S3_FLASH").is_err() {
            eprintln!(
                "error: --rom-boot needs LABWIRED_ESP32S3_FLASH set (the firmware flash image)"
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
        eprintln!(
            "labwired-cli run: ROM-boot from reset vector 0x{:08x} (real ROM + flash controller)",
            cpu.get_pc(),
        );
        // Faithful windowed-register machinery: rom-boot runs the real ROM +
        // firmware, which install the OF/UF window vectors and build a proper
        // stack save chain — so use the real per-access overflow / RETW
        // underflow path (no sim shadow stack).
        cpu.faithful_windows = true;
        // Bring up the APP_CPU (halted at the ROM reset vector 0x40000400).
        let mut c1 = labwired_core::cpu::xtensa_lx7::XtensaLx7::new_app_cpu();
        c1.faithful_windows = true;
        eprintln!(
            "labwired-cli run: APP_CPU created (halted at reset vector 0x{:08x})",
            c1.get_pc(),
        );
        cpu1 = Some(c1);
    } else {
        // Fast-boot.
        let boot = match fast_boot(
            &elf_bytes,
            &mut bus,
            &mut cpu,
            &BootOpts {
                stack_top_fallback: 0x3FCD_FFF0,
                icache_backing: Some(wiring.icache_backing),
                dcache_backing: Some(wiring.dcache_backing),
            },
        ) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("error: fast_boot failed: {e}");
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
        };
        eprintln!(
            "labwired-cli run: entry=0x{:08x} stack=0x{:08x} segments={}",
            boot.entry, boot.stack, boot.segments_loaded,
        );

        // ESP-IDF dual-core handshake (legacy thunk-path stopgap). system_early_init
        // busy-waits until both per-core init flags are set; the single-CPU run
        // path pre-paints them. Superseded by the SMP phase of the chip model.
        let symbol_addrs = labwired_loader::extract_arduino_esp32_thunks(&elf_bytes);
        for (sym, span) in [
            ("s_cpu_inited", 2u32),
            ("s_cpu_up", 2),
            ("s_system_inited", 2),
            ("s_resume_cores", 1),
            ("s_other_cpu_startup_done", 1),
        ] {
            if let Some(&addr) = symbol_addrs.get(sym) {
                for off in 0..span {
                    let _ = bus.write_u8(addr as u64 + off as u64, 0x01);
                }
                eprintln!("labwired-cli run: handshake {sym} @0x{addr:08x} = 1");
            }
        }
    }

    // Run the step loop.
    let limit = args.max_steps.unwrap_or(u64::MAX);
    let observers: Vec<std::sync::Arc<dyn labwired_core::SimulationObserver>> = Vec::new();
    let config = labwired_core::SimulationConfig::default();
    let mut steps = 0u64;
    // Ring buffer of recent PCs for post-mortem on exceptions.
    const RING_LEN: usize = 1024;
    let mut pc_ring: [u32; RING_LEN] = [0; RING_LEN];
    let mut ring_head: usize = 0;
    let smp_trace = std::env::var("LABWIRED_SMP_TRACE").is_ok();
    let dense_from: u64 = std::env::var("LABWIRED_DENSE_FROM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(u64::MAX);
    let dense_len: u64 = std::env::var("LABWIRED_DENSE_LEN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(800);
    // First-hit watchpoints for the SMP startup → first-task-dispatch path
    // (addresses from firmware.elf for this Unity demo). Each tracks whether
    // it's been reported on core 0 / core 1 yet.
    let mut watch: [(u32, &str, [bool; 2]); 11] = [
        (0x4037ec3c, "xPortStartScheduler", [false; 2]),
        (0x4037f064, "_frxt_dispatch", [false; 2]),
        (0x4037f067, "dispatch:post-switchctx", [false; 2]),
        (0x4037f08f, "dispatch:retw-into-task", [false; 2]),
        (0x4037fd64, "vTaskSwitchContext", [false; 2]),
        (0x4037f960, "prvIdleTask", [false; 2]),
        (0x4202240c, "esp_startup_start_app", [false; 2]),
        (0x4202239c, "main_task", [false; 2]),
        (0x420047c0, "app_main", [false; 2]),
        (0x42002040, "setup()", [false; 2]),
        (0x42001f90, "UnityBegin", [false; 2]),
    ];
    // Debug breakpoints / memory watches (parse hex; ignore unparseable).
    let parse_hex = |s: &str| -> Option<u32> {
        u32::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
    };
    let break_at: Vec<u32> = args.break_at.iter().filter_map(|s| parse_hex(s)).collect();
    let watch_mem: Vec<u32> = args.watch_mem.iter().filter_map(|s| parse_hex(s)).collect();
    let mut break_hit = vec![false; break_at.len()]; // PRO_CPU first-hit flags
    let mut break_hit1 = vec![false; break_at.len()]; // APP_CPU first-hit flags
                                                      // On the first time a core's PC reaches a --break-at address, dump its
                                                      // a0..a15 + window state and the --watch-mem words. Covers both cores so an
                                                      // APP_CPU fault is observable too.
    macro_rules! check_break {
        ($c:expr, $pc:expr, $hits:expr) => {
            if let Some(bi) = break_at.iter().position(|&b| b == $pc) {
                if !$hits[bi] {
                    $hits[bi] = true;
                    eprintln!(
                        "labwired-cli run: BREAK-AT 0x{:08x} (step {steps}, core {})",
                        $pc,
                        if $c.app_cpu { 1 } else { 0 }
                    );
                    for r in 0..16u8 {
                        eprintln!("    a{:<2} = 0x{:08x}", r, $c.regs.read_logical(r));
                    }
                    eprintln!(
                        "    PS=0x{:08x} WB={} WS=0x{:04x}",
                        $c.ps.as_raw(),
                        $c.regs.windowbase(),
                        $c.regs.windowstart()
                    );
                    for &m in &watch_mem {
                        match bus.read_u32(m as u64) {
                            Ok(v) => eprintln!("    mem[0x{m:08x}] = 0x{v:08x}"),
                            Err(e) => eprintln!("    mem[0x{m:08x}] = <unmapped: {e}>"),
                        }
                    }
                }
            }
        };
    }
    if !break_at.is_empty() {
        eprintln!(
            "labwired-cli run: breakpoints {:?} watch-mem {:?}",
            break_at
                .iter()
                .map(|a| format!("0x{a:08x}"))
                .collect::<Vec<_>>(),
            watch_mem
                .iter()
                .map(|a| format!("0x{a:08x}"))
                .collect::<Vec<_>>(),
        );
    }

    while steps < limit {
        let pc_before = cpu.get_pc();
        pc_ring[ring_head] = pc_before;
        ring_head = (ring_head + 1) % RING_LEN;

        // Debug breakpoint (PRO_CPU): dump on first hit.
        check_break!(cpu, pc_before, break_hit);

        // Capture the APP_CPU entry when PRO_CPU programs it. The ROM also
        // points the APP_CPU at early DRAM stubs during its own bring-up; only
        // a real code entry (app IRAM/XIP, >= 0x4037_0000 — excludes ROM and
        // DRAM) is the application's `call_start_cpu1`.
        // Release the APP_CPU on the real hardware edge: the PRO_CPU clearing
        // CORE_1_RESETING (signalled by the SYSTEM_CORE_1_CONTROL peripheral).
        // The APP_CPU then boots the real ROM from its reset vector — exactly
        // like silicon, no firmware-symbol hooks.
        if !appcpu_started
            && labwired_core::peripherals::esp32s3::rom_thunks::APPCPU_RESET_RELEASED
                .with(|s| s.take())
        {
            appcpu_started = true;
            if let Some(c1) = cpu1.as_mut() {
                c1.halted = false;
            }
            eprintln!(
                "labwired-cli run: APP_CPU released from reset → booting real ROM (step {steps})"
            );
        }

        match cpu.step(&mut bus, &observers, &config) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(pc)) => {
                eprintln!("labwired-cli run: BREAK at 0x{pc:08x}");
                return ExitCode::from(EXIT_PASS);
            }
            Err(SimulationError::ExceptionRaised { cause, pc }) => {
                eprintln!("labwired-cli run: ExceptionRaised cause={cause} at 0x{pc:08x}");
                eprintln!(
                    "labwired-cli run: PS=0x{:08x} (excm={} intlevel={}) WB={} WS=0x{:04x}",
                    cpu.ps.as_raw(),
                    cpu.ps.excm(),
                    cpu.ps.intlevel(),
                    cpu.regs.windowbase(),
                    cpu.regs.windowstart(),
                );
                eprintln!("labwired-cli run: recent PCs (oldest first):");
                for i in 0..RING_LEN {
                    let idx = (ring_head + i) % RING_LEN;
                    if pc_ring[idx] != 0 {
                        eprintln!("  [{:2}] 0x{:08x}", i, pc_ring[idx]);
                    }
                }
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
            Err(e) => {
                eprintln!(
                    "labwired-cli run: simulator error at pc=0x{:08x}: {e}",
                    cpu.get_pc(),
                );
                eprintln!("labwired-cli run: a0..a15 at fault:");
                for r in 0..16u8 {
                    eprintln!("  a{:<2} = 0x{:08x}", r, cpu.regs.read_logical(r));
                }
                eprintln!(
                    "  WB=0x{:x} WS=0x{:04x}",
                    cpu.regs.windowbase(),
                    cpu.regs.windowstart(),
                );
                eprintln!("labwired-cli run: recent PCs (oldest first):");
                for i in 0..RING_LEN {
                    let idx = (ring_head + i) % RING_LEN;
                    if pc_ring[idx] != 0 {
                        eprintln!("  [{:2}] 0x{:08x}", i, pc_ring[idx]);
                    }
                }
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
        }
        // panic_abort(details) reason printer (gated): the ESP-IDF panic path
        // stores the assert/abort string ptr in a2 just before the trap. Helps
        // pinpoint firmware-level aborts during bring-up.
        if std::env::var("LABWIRED_CCDBG").is_ok() {
            for c in [Some(&cpu), cpu1.as_ref()].into_iter().flatten() {
                if c.get_pc() == 0x4037_e0a3 {
                    let p = c.regs.read_logical(2);
                    let mut s = String::new();
                    for i in 0..160u32 {
                        match bus.read_u8(p as u64 + i as u64) {
                            Ok(0) | Err(_) => break,
                            Ok(b) => s.push(b as char),
                        }
                    }
                    eprintln!("CCDBG: panic \"{s}\" step={steps}");
                }
            }
        }
        // Step the APP_CPU round-robin (one instruction per PRO_CPU step).
        // A halted APP_CPU returns immediately from step(). S32C1I is atomic
        // within step(), so spinlocks between the cores behave correctly.
        if let Some(c1) = cpu1.as_mut() {
            // Debug breakpoint (APP_CPU): dump on first hit.
            check_break!(c1, c1.get_pc(), break_hit1);
            match c1.step(&mut bus, &observers, &config) {
                Ok(()) | Err(SimulationError::BreakpointHit(_)) => {}
                Err(e) => {
                    eprintln!(
                        "labwired-cli run: APP_CPU error at pc=0x{:08x}: {e}",
                        c1.get_pc()
                    );
                    return ExitCode::from(EXIT_RUNTIME_ERROR);
                }
            }
        }
        bus.tick_peripherals_with_costs();
        steps += 1;

        // SMP bring-up tracer (gated). Prints both cores' PCs periodically and
        // flags the first time each core enters app XIP code (>= 0x4200_0000,
        // where setup()/loop()/Unity live) — the signal that the FreeRTOS SMP
        // scheduler finally dispatched the pinned loopTask.
        if smp_trace {
            for (core, pc) in [
                (0usize, cpu.get_pc()),
                (1usize, cpu1.as_ref().map(|c| c.get_pc()).unwrap_or(0)),
            ] {
                for w in watch.iter_mut() {
                    if w.0 == pc && !w.2[core] {
                        w.2[core] = true;
                        eprintln!("SMP: core {core} reached {} (0x{pc:08x}) step {steps}", w.1);
                    }
                }
            }
            if steps.is_multiple_of(10_000_000) {
                eprintln!(
                    "SMP: step {steps:>11}  pro=0x{:08x}  app=0x{:08x}",
                    cpu.get_pc(),
                    cpu1.as_ref().map(|c| c.get_pc()).unwrap_or(0),
                );
            }
            // Dense single-step trace window (env LABWIRED_DENSE_FROM / _LEN)
            // for following a context switch instruction-by-instruction.
            if steps >= dense_from && steps < dense_from + dense_len {
                eprintln!(
                    "D {steps} pro=0x{:08x} ps={:x} wb={} ws=0x{:04x} exc={} epc1=0x{:08x} | app=0x{:08x}",
                    cpu.get_pc(),
                    cpu.ps.as_raw(),
                    cpu.regs.windowbase(),
                    cpu.regs.windowstart(),
                    cpu.sr.read(232),
                    cpu.sr.read(177),
                    cpu1.as_ref().map(|c| c.get_pc()).unwrap_or(0),
                );
            }
        }
    }
    // Optional end-of-run dump of the Unity result struct (env
    // LABWIRED_UNITY_ADDR=<hex base of the `Unity` UNITY_STORAGE_T global>).
    // Mirrors the hardware oracle (`mdw <addr> 10`): NumberOfTests at +20,
    // TestFailures at +24, TestIgnores at +28 — the authoritative pass/fail
    // since Unity's text output goes out USB_SERIAL_JTAG, not stdout.
    if let Ok(s) = std::env::var("LABWIRED_UNITY_ADDR") {
        if let Ok(base) = u32::from_str_radix(s.trim_start_matches("0x"), 16) {
            let mut words = [0u32; 10];
            for (i, w) in words.iter_mut().enumerate() {
                *w = bus.read_u32(base as u64 + (i * 4) as u64).unwrap_or(0);
            }
            eprint!("labwired-cli run: Unity@0x{base:08x}:");
            for w in &words {
                eprint!(" {w:08x}");
            }
            eprintln!();
            eprintln!(
                "labwired-cli run: Unity NumberOfTests={} TestFailures={} TestIgnores={}",
                words[5], words[6], words[7],
            );
        }
    }
    let cpu1_pc = cpu1
        .as_ref()
        .map(|c| format!(" appcpu_pc=0x{:08x}", c.get_pc()))
        .unwrap_or_default();
    eprintln!(
        "labwired-cli run: reached --max-steps {limit}; pc=0x{:08x}{cpu1_pc}",
        cpu.get_pc(),
    );
    ExitCode::from(EXIT_PASS)
}

fn run_snapshot(args: SnapshotArgs) -> ExitCode {
    match args.command {
        SnapshotCommands::Capture(a) => run_snapshot_capture(a),
    }
}

fn run_coverage(args: CoverageArgs) -> ExitCode {
    if let Some(p) = &args.svd {
        std::env::set_var("LABWIRED_ESP32S3_SVD", p);
    }
    match coverage::run() {
        Some((matrix, text)) => {
            print!("{text}");
            if let Some(out) = &args.json_out {
                let json = serde_json::to_string_pretty(&matrix).expect("serialize matrix");
                std::fs::write(out, &json).expect("write json");
                eprintln!("wrote {}", out.display());
            }
            ExitCode::SUCCESS
        }
        None => {
            eprintln!(
                "error: ESP32-S3 SVD not found; set --svd or LABWIRED_ESP32S3_SVD, \
                 or install the espressif32 PlatformIO platform"
            );
            ExitCode::from(EXIT_CONFIG_ERROR)
        }
    }
}

fn run_fuzz(args: FuzzArgs) -> ExitCode {
    use labwired_fuzz::{fuzz, fuzz_collect, Contract, Target, Verdict};

    let contract = Contract {
        input_len: args.input_len_addr,
        input_data: args.input_data_addr,
        verdict: args.verdict_addr,
        done_magic: args.done_magic,
        fault_magic: args.fault_magic,
    };

    // Seeds: parse `--seed-input` hex bytes; empty means the engine self-seeds.
    let mut seeds: Vec<Vec<u8>> = Vec::new();
    for s in &args.seed_input {
        let t = s.trim_start_matches("0x");
        if t.len() % 2 != 0 {
            eprintln!("error: --seed-input `{s}` must be an even number of hex digits");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
        let mut bytes = Vec::with_capacity(t.len() / 2);
        for i in (0..t.len()).step_by(2) {
            match u8::from_str_radix(&t[i..i + 2], 16) {
                Ok(b) => bytes.push(b),
                Err(e) => {
                    eprintln!("error: --seed-input `{s}`: {e}");
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
            }
        }
        seeds.push(bytes);
    }

    let target = match Target::from_elf(
        &args.chip,
        &args.system,
        &args.firmware,
        contract,
        args.max_steps,
    ) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let engine = if cfg!(feature = "fuzz-libafl") {
        "LibAFL"
    } else {
        "built-in"
    };
    eprintln!(
        "fuzzing {} with the {engine} engine (max_iters={}, seed={:#x}) ...",
        args.firmware.display(),
        args.max_iters,
        args.seed
    );

    // Collect-N mode gathers distinct crashes (feeds HIL-confirm); default mode
    // stops at the first crash.
    let crashes: Vec<Vec<u8>> = if let Some(n) = args.collect {
        fuzz_collect(&target, seeds, args.max_iters, args.seed, n)
    } else {
        match fuzz(&target, seeds, args.max_iters, args.seed) {
            r @ labwired_fuzz::FuzzReport { crash: None, .. } => {
                println!(
                    "no crash in {} iters (corpus {}, {} edges)",
                    r.iterations, r.corpus_size, r.edges_hit
                );
                return ExitCode::SUCCESS;
            }
            labwired_fuzz::FuzzReport {
                crash: Some(c),
                iterations,
                corpus_size,
                edges_hit,
            } => {
                println!(
                    "CRASH in {iterations} iters (corpus {corpus_size}, {edges_hit} edges): {:02X?}",
                    c
                );
                vec![c]
            }
        }
    };

    if crashes.is_empty() {
        println!("no crash found in {} iters", args.max_iters);
        return ExitCode::SUCCESS;
    }

    if args.collect.is_some() {
        println!("found {} distinct crash(es):", crashes.len());
        for c in &crashes {
            println!("  {c:02X?}");
        }
    }

    // Reproduce + report the first crash's verdict for clarity.
    let mut cov = labwired_fuzz::CovMap::new();
    let verdict = target.run(&crashes[0], &mut cov);
    let label = match verdict {
        Verdict::Crash => "crash (fault/panic marker)",
        Verdict::Hang => "hang (step budget exhausted)",
        Verdict::Clean => "clean (non-deterministic?)",
    };
    eprintln!("first crash reproduces as: {label}");

    if let Some(out) = &args.crashes_out {
        match serde_json::to_string_pretty(&crashes) {
            Ok(json) => {
                if let Err(e) = std::fs::write(out, json) {
                    eprintln!("error: write {}: {e}", out.display());
                    return ExitCode::FAILURE;
                }
                eprintln!(
                    "wrote {} crash input(s) to {}",
                    crashes.len(),
                    out.display()
                );
            }
            Err(e) => {
                eprintln!("error: serialize crashes: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    // A crash is a finding — non-zero exit so CI fails the build.
    ExitCode::FAILURE
}

fn run_tier1_matrix(args: Tier1MatrixArgs) -> ExitCode {
    let self_bin = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: cannot resolve current executable: {e}");
            return ExitCode::FAILURE;
        }
    };
    match labwired_cli::tier1::run_all(&self_bin) {
        Ok((mut matrix, skipped)) => {
            for chip in &skipped {
                eprintln!("SKIP: {chip} (fixture not present)");
            }
            // --run-url given but nothing was actually exercised → vacuous green
            // is not permitted; fail loudly so CI notices the misconfiguration.
            // (Skipped targets still emit unrecorded rows, so key on the count
            // of EXERCISED chips, not on matrix emptiness.)
            if args.run_url.is_some() && matrix.0.len() == skipped.len() {
                eprintln!("error: --run-url given but no fixtures were exercised");
                return ExitCode::FAILURE;
            }
            if let Some(url) = &args.run_url {
                use labwired_cli::tier1::CellStatus;
                for row in matrix.0.values_mut() {
                    for cell in row.values_mut() {
                        if cell.status != CellStatus::Unrecorded && cell.status != CellStatus::Na {
                            cell.run_url = Some(url.clone());
                        }
                    }
                }
            }
            // Text grid for humans.
            for (chip, row) in &matrix.0 {
                let cells: Vec<String> = row
                    .iter()
                    .map(|(class, cell)| format!("{class}={}", cell.status.as_str()))
                    .collect();
                println!("{chip}: {}", cells.join(" "));
            }
            if let Some(out) = &args.json_out {
                let json = match serde_json::to_string_pretty(&matrix) {
                    Ok(j) => j,
                    Err(e) => {
                        eprintln!("error: failed to serialize tier1 matrix: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                if let Err(e) = std::fs::write(out, json.as_bytes()) {
                    eprintln!("error: failed to write {}: {e}", out.display());
                    return ExitCode::FAILURE;
                }
                eprintln!("wrote {}", out.display());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("tier1-matrix failed: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Drive a firmware mid-flight in a headless sim and write a runtime
/// snapshot blob. The playground reads the same blob to skip cold boot.
///
/// The `agentdeck` profile mirrors what
/// `WasmSimulator::install_esp32_arduino_quirks` plus `step_with_esp32_aids`
/// do on the web side — same configure_xtensa_esp32 bus, same handshake,
/// same thunk setup, same IPI bridge cadence — so the captured state will
/// resume bit-identically inside the browser.
fn run_snapshot_capture(args: SnapshotCaptureArgs) -> ExitCode {
    use labwired_core::bus::SystemBus;
    use labwired_core::peripherals::components::{Ssd1680Tricolor290, Uc8151dTricolor290};
    use labwired_core::peripherals::esp32::spi::Esp32Spi;
    use labwired_core::peripherals::esp32s3::rom_thunks;
    use labwired_core::system::xtensa::configure_xtensa_esp32;
    use labwired_core::{Machine, SimulationError};
    use labwired_loader::{extract_arduino_esp32_thunks, load_elf_bytes};

    if args.profile != "agentdeck" && args.profile != "arduino-esp32" {
        eprintln!(
            "error: unknown profile '{p}' — supported: 'agentdeck' (preset-PC profile — hardcoded thunk addresses for a specific firmware build), 'arduino-esp32' (any Arduino-ESP32 ELF with symbols intact, auto-discovers thunk PCs)",
            p = args.profile
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let elf_bytes = match std::fs::read(&args.firmware) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read firmware ELF {:?}: {e}", args.firmware);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Bus + CPU — same configure_xtensa_esp32 that the WASM uses.
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);

    // Peripherals come from the board manifest, never hardcoded here. The
    // generic attach_esp32_external_devices factory wires every declared
    // external device (panel, etc.) onto its bus with the right model, CS and
    // DC pins. --system points at the board manifest (e.g. the ereader's
    // board.yaml declaring the SSD1680 e-paper on spi3, CS=GPIO5, DC=GPIO17).
    if let Some(sys_path) = &args.system {
        match labwired_config::SystemManifest::from_file(sys_path) {
            Ok(manifest) => {
                if let Err(e) = labwired_core::system::xtensa::attach_esp32_external_devices(
                    &mut bus, &manifest,
                ) {
                    eprintln!("error: attaching external devices from {sys_path:?}: {e}");
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
            }
            Err(e) => {
                eprintln!("error: cannot load system manifest {sys_path:?}: {e}");
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        }
    } else {
        eprintln!(
            "warning: no --system manifest; no external peripherals attached \
             (firmware that drives a panel will not render)"
        );
    }
    // Enable wire-byte capture on spi3 for snapshot diagnostics (a capture
    // concern, not a device wiring concern).
    if let Some(spi3_idx) = bus.find_peripheral_index_by_name("spi3") {
        if let Some(any) = bus.peripherals[spi3_idx].dev.as_any_mut() {
            if let Some(spi3) = any.downcast_mut::<Esp32Spi>() {
                spi3.enable_byte_capture(65536);
            }
        }
    }
    bus.refresh_peripheral_index();

    let boxed: Box<dyn Cpu> = Box::new(cpu);
    let mut machine = Machine::new(boxed, bus);
    // Arduino-ESP32 sketches reach `xTaskCreatePinnedToCore(..., 1)`
    // for `loopTask` and others — without an APP_CPU to schedule onto,
    // FreeRTOS spins in `vListInsert` forever. Attach a secondary CPU
    // (PRID=0xABAB, halted at construction, released by
    // `ets_set_appcpu_boot_addr` during PRO_CPU boot).
    if args.profile == "arduino-esp32" {
        let cpu1 = labwired_core::cpu::xtensa_lx7::XtensaLx7::new_app_cpu();
        machine.cpu_secondary = Some(Box::new(cpu1));
    }

    // Load firmware FIRST — load_firmware writes ELF segments into bus
    // memory, so any bytes we write before this risk being clobbered.
    // The handshake/header writes and `install_flash_thunk` (which patches
    // BREAK bytes into flash) must happen AFTER the ELF is in place.
    let program_image = match load_elf_bytes(&elf_bytes) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: load_elf_bytes: {e}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    if let Err(e) = machine.load_firmware(&program_image) {
        eprintln!("error: load_firmware: {e}");
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }
    // XtensaLx7::reset() leaves PC at the 0x40000400 BROM reset vector.
    // Skip BROM and jump straight to the ELF's app entry — same as WASM.
    // CHEAT(SKIP): bypasses the boot ROM and hand-seeds PC (SP seeded below).
    // See FIDELITY.md §C.
    machine.cpu.set_pc(program_image.entry_point as u32);

    // Resolve every Arduino-ESP32 symbol we know how to patch / thunk.
    // Empty for the reference firmware (stripped) — those fall back to hardcoded PCs.
    let symbol_addrs = extract_arduino_esp32_thunks(&elf_bytes);
    let resolve_data =
        |sym: &str, fallback: u32| -> u32 { symbol_addrs.get(sym).copied().unwrap_or(fallback) };
    // APP_CPU initial stack — read once, used on cpu1 unhalt.
    // ESP-IDF puts the boot stack at `port_IntStackTop`; if the symbol
    // is missing (stripped ELF), fall back to a safe high-DRAM addr.
    let appcpu_initial_sp: u32 = symbol_addrs
        .get("port_IntStackTop")
        .copied()
        .unwrap_or(0x3FFB_F3A0);

    // loopTask xCoreID repin: Arduino-ESP32's app_main calls
    // xTaskCreateUniversal(loopTask, ..., xCoreID=1), pinning loopTask to
    // APP_CPU. We model only PRO_CPU, so rewrite the xCoreID immediate to 0.
    // Handles both legacy and IDF-5.x app_main layouts. See
    // rom_thunks::repin_loop_task.
    if let Some(&app_main_addr) = symbol_addrs.get("app_main") {
        if let Some((addr, shape)) = rom_thunks::repin_loop_task(&mut machine.bus, app_main_addr) {
            eprintln!(
                "labwired-cli snapshot: repinned loopTask xCoreID at 0x{addr:08x} (1→0, {shape}; runs on PRO_CPU)"
            );
        }
    }

    // Arduino-ESP32 bootstrap — keep in sync with
    // `wasm/src/lib.rs::install_esp32_arduino_quirks` and the e2e test.
    machine.cpu.set_sp(0x3FFE_0000);
    // Handshake-byte pre-paint. Two paths:
    //   * `agentdeck` profile — use the exact byte addresses the original
    //     install_esp32_arduino_quirks wrote, byte-for-byte. the reference firmware's
    //     ELF is stripped, so symbol auto-discovery returns nothing.
    //   * `arduino-esp32` profile — resolve s_resume_cores / s_cpu_up /
    //     s_cpu_inited / s_system_inited / s_other_cpu_startup_done from
    //     the ELF symbol table and write 0x01 to both bytes of each.
    let (s_resume_cores, s_cpu_up, s_cpu_inited, s_system_inited, s_other_cpu_startup_done);
    if args.profile == "agentdeck" {
        s_resume_cores = 0;
        s_cpu_up = 0;
        s_cpu_inited = 0;
        s_system_inited = 0;
        s_other_cpu_startup_done = 0;
        let _ = machine.bus.write_u8(0x3FFC_6F04, 0x01); // s_cpu_up[1]
        let _ = machine.bus.write_u8(0x3FFC_6F01, 0x01); // s_cpu_inited[0]
        let _ = machine.bus.write_u8(0x3FFC_6F02, 0x01); // s_cpu_inited[1]
        let _ = machine.bus.write_u8(0x3FFC_6FFD, 0x01); // s_system_inited[0]
        let _ = machine.bus.write_u8(0x3FFC_6FFE, 0x01); // s_system_inited[1]
        let _ = machine.bus.write_u8(0x3FFC_7190, 0x01); // s_other_cpu_startup_done
        let _ = machine.bus.write_u8(0x400E_90DE, 0x08); // loopTask -> PRO_CPU
                                                         // Re-assert the same flags the instant PRO_CPU releases APP_CPU
                                                         // (models APP_CPU bring-up; see rom_thunks::ets_set_appcpu_boot_addr).
        rom_thunks::set_appcpu_up_flags(vec![
            0x3FFC_6F04,
            0x3FFC_6F01,
            0x3FFC_6F02,
            0x3FFC_6FFD,
            0x3FFC_6FFE,
            0x3FFC_7190,
        ]);
    } else {
        s_resume_cores = resolve_data("s_resume_cores", 0);
        s_cpu_up = resolve_data("s_cpu_up", 0);
        s_cpu_inited = resolve_data("s_cpu_inited", 0);
        s_system_inited = resolve_data("s_system_inited", 0);
        s_other_cpu_startup_done = resolve_data("s_other_cpu_startup_done", 0);
        if s_resume_cores != 0 {
            let _ = machine.bus.write_u8(s_resume_cores as u64, 0x01);
        }
        if s_cpu_up != 0 {
            let _ = machine.bus.write_u8(s_cpu_up as u64, 0x01);
            let _ = machine.bus.write_u8(s_cpu_up as u64 + 1, 0x01);
        }
        if s_cpu_inited != 0 {
            let _ = machine.bus.write_u8(s_cpu_inited as u64, 0x01);
            let _ = machine.bus.write_u8(s_cpu_inited as u64 + 1, 0x01);
        }
        if s_system_inited != 0 {
            let _ = machine.bus.write_u8(s_system_inited as u64, 0x01);
            let _ = machine.bus.write_u8(s_system_inited as u64 + 1, 0x01);
        }
        if s_other_cpu_startup_done != 0 {
            let _ = machine.bus.write_u8(s_other_cpu_startup_done as u64, 0x01);
        }
        // Re-assert these flags the instant PRO_CPU releases APP_CPU, so
        // newer arduino-esp32 cores (whose `start_other_core` spin-waits
        // with a tight timeout) see APP_CPU "up" without depending on the
        // coarse 10k-cycle keep-alive below. Models APP_CPU bring-up; see
        // rom_thunks::ets_set_appcpu_boot_addr.
        let mut appcpu_up_flags: Vec<u32> = Vec::new();
        for (base, two_byte) in [
            (s_cpu_up, true),
            (s_cpu_inited, true),
            (s_system_inited, true),
            (s_resume_cores, false),
            (s_other_cpu_startup_done, false),
        ] {
            if base != 0 {
                appcpu_up_flags.push(base);
                if two_byte {
                    appcpu_up_flags.push(base + 1);
                }
            }
        }
        rom_thunks::set_appcpu_up_flags(appcpu_up_flags);
    }
    // RTC XTAL-freq probe = 40 MHz.
    let _ = machine.bus.write_u32(0x3FF4_80B0, 0x0050_0050);

    // Build the thunk address list. Each entry maps a flash PC to a
    // sim-side rom_thunks function. For unstripped ELFs we use the
    // already-parsed symbol map above; the reference firmware's fully stripped ELF
    // falls back to the hand-curated address list.
    let resolve =
        |sym: &str, fallback: u32| -> u32 { symbol_addrs.get(sym).copied().unwrap_or(fallback) };
    let mut thunks: Vec<(u32, rom_thunks::RomThunkFn)> = vec![
        (
            resolve("heap_caps_init", 0x400e_e3b0),
            rom_thunks::esp_idf_heap_caps_init,
        ),
        (
            resolve("heap_caps_malloc", 0x4008_2904),
            rom_thunks::esp_idf_heap_caps_malloc,
        ),
        (
            resolve("heap_caps_calloc", 0x4008_2a70),
            rom_thunks::esp_idf_heap_caps_calloc,
        ),
        (
            resolve("heap_caps_free", 0x4008_25dc),
            rom_thunks::esp_idf_heap_caps_free,
        ),
        (
            resolve("heap_caps_realloc", 0x4008_29f0),
            rom_thunks::esp_idf_heap_caps_realloc,
        ),
        (
            resolve("esp_timer_init", 0x4012_9034),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve(
                "spi_flash_disable_interrupts_caches_and_other_cpu",
                0x4008_17dc,
            ),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve(
                "spi_flash_enable_interrupts_caches_and_other_cpu",
                0x4008_188c,
            ),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("__retarget_lock_init_recursive", 0x4008_3384),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("__retarget_lock_close_recursive", 0x4008_339c),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("__retarget_lock_acquire_recursive", 0x4008_33b0),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("__retarget_lock_release_recursive", 0x4008_33cc),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("_esp_error_check_failed", 0x4008_bbd0),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("setCpuFrequencyMhz", 0x400e_99dc),
            rom_thunks::nop_return_zero,
        ),
        (
            resolve("esp_ota_get_running_partition", 0x400e_ae18),
            rom_thunks::nop_return_fake_ptr,
        ),
        (resolve("delay", 0x400e_5c28), rom_thunks::nop_return_zero),
    ];
    // HardwareSerial::begin only exists when the sketch called Serial.begin().
    if let Some(&pc) = symbol_addrs.get("HardwareSerial::begin(unsigned long, unsigned int, signed char, signed char, bool, unsigned long, unsigned char)") {
        thunks.push((pc, rom_thunks::nop_return_zero));
    } else if args.profile == "agentdeck" {
        thunks.push((0x400e_2280, rom_thunks::nop_return_zero));
    }
    // Real-silicon noreturn functions — abort_halt prints diagnostics and
    // halts the CPU instead of returning. Without this, stubbing them as
    // nop_return_zero creates tight `assert → return → re-check → assert`
    // loops in xQueueGenericSend's parameter-validation path.
    for sym in &[
        "panic_abort",
        "__assert_func",
        "abort",
        "__assert",
        "__cxa_pure_virtual",
        "__cxa_throw",
    ] {
        if let Some(&pc) = symbol_addrs.get(*sym) {
            thunks.push((pc, rom_thunks::abort_halt));
        }
    }
    // ESP-IDF clock/efuse/cache/dport bring-up — the sim has no silicon
    // behind these so we stub them to return-0. Only installed when the
    // symbol is present in the ELF (Arduino-ESP32 profile).
    for sym in &[
        // newlib stdio init — sketch doesn't use stdio on render path
        "__sinit",
        "__sfp",
        "__sfp_lock_acquire",
        "__sfp_lock_release",
        "__sflags",
        "__swsetup_r",
        "__srefill_r",
        "__sread",
        "__swrite",
        "__seek",
        "__sclose",
        "esp_reent_init",
        "_fflush_r",
        "_fclose_r",
        "_fwrite_r",
        "esp_panic_handler",
        "esp_panic_handler_reconfigure_wdts",
        // xTaskGetCurrentTaskHandle gets a proper thunk below — returning
        // 0 breaks vTaskDelete(NULL) by passing NULL into prvDeleteTLS.
        "pthread_key_create",
        "pthread_setspecific",
        "pthread_getspecific",
        "pthread_mutex_init",
        "pthread_mutex_lock",
        "pthread_mutex_unlock",
        // Dual-core sim: with cpu_secondary actually running, FreeRTOS
        // primitives can use their real implementations — stubbing them
        // would defeat the purpose. Only esp_pthread_init stays stubbed
        // (it depends on per-task TLS we don't model).
        "esp_pthread_init",
        "esp_task_wdt_reset",
        "esp_task_wdt_init",
        "esp_task_wdt_add",
        "esp_task_wdt_delete",
        "esp_clk_init",
        "esp_perip_clk_init",
        "core_intr_matrix_clear",
        "esp_efuse_check_errors",
        "esp_dport_access_stall_other_cpu_start",
        "esp_dport_access_stall_other_cpu_end",
        "esp_cpu_unstall",
        "bootloader_flash_update_id",
        "bootloader_init_mem",
        "esp_mspi_pin_init",
        "spi_flash_init_chip_state",
        "esp_log_timestamp",
        // SPI-flash HAL — see loader::extract_arduino_esp32_thunks for why.
        "spi_flash_hal_configure_host_io_mode",
        "spi_flash_chip_generic_config_host_io_mode",
        "spi_flash_chip_generic_get_io_mode",
        "spi_flash_chip_generic_set_io_mode",
        "spi_flash_chip_generic_probe",
        "spi_flash_chip_generic_detect_size",
        "spi_flash_chip_generic_read",
        "spi_flash_chip_generic_yield",
        "spi_flash_chip_gd_probe",
        "spi_flash_chip_gd_detect_size",
        "spi_flash_chip_gd_get_io_mode",
        "spi_flash_chip_gd_set_io_mode",
        "spi_flash_init",
        "spi_flash_hal_init",
        "spi_flash_hal_supports_direct_write",
        "spi_flash_hal_supports_direct_read",
        "esp_flash_app_enable_os_functions",
        "esp_flash_app_disable_os_functions",
        "esp_flash_app_init",
        "esp_flash_init_main",
        "esp_flash_init_default_chip",
        "esp_flash_init",
        "esp_random",
        "esp_fill_random",
        "esp_log_early_timestamp",
        "esp_log_writev",
        "esp_log_write",
        "esp_log_buffer_hex_internal",
        "esp_log_buffer_char_internal",
        "esp_log_buffer_hexdump_internal",
        // log mutex (esp_log_impl_lock/unlock) — sim doesn't model the log
        // mutex queue, and the real impl calls xQueueGenericSend on an
        // uninitialized queue, tripping a NULL-pcHead assertion.
        "esp_log_impl_lock",
        "esp_log_impl_lock_timeout",
        "esp_log_impl_unlock",
        // esp_ipc_init/isr_init create the IPC task per core. Its
        // semaphore-wait turns into a tight loop in the sim (xQueueSemaphoreTake
        // is stubbed to pdTRUE), starving loopTask. Stub the init so the
        // task is never created — cross-core IPC isn't used on the
        // single-CPU render path.
        "esp_ipc_init",
        "esp_ipc_isr_init",
        // HardwareSerial / UART layer only — leave Print/Stream alone so
        // virtual dispatch through Print::print → Adafruit_GFX::write →
        // drawPixel (the display.print render path) keeps working. The
        // original spin was in HardwareSerial::write's buffer-available
        // wait, not in Print or Stream.
        "_ZN14HardwareSerial5writeEh",
        "_ZN14HardwareSerial5writeEPKhj",
        "_ZN14HardwareSerial9availableEv",
        "_ZN14HardwareSerial5flushEv",
        "_ZN14HardwareSerial9readBytesEPcj",
        "_ZN14HardwareSerial9readBytesEPhj",
        "uartAvailable",
        "uartAvailableForWrite",
        "uartWrite",
        "uartWriteBuf",
        "_Z14serialEventRunv",
        // FreeRTOS recursive mutexes used by newlib stdio locks — same
        // null-queue assertion problem. Stub since sim is effectively
        // single-threaded on the panel-render path. xQueueCreateMutexStatic
        // gets a separate echo_arg0 thunk below (callers assert the returned
        // handle equals the static buffer they passed in).
        "xQueueGiveMutexRecursive",
        "xQueueTakeMutexRecursive",
        "xQueueCreateMutex",
        "__sfvwrite_r",
        "__swsetup_r",
        "__sflush_r",
        "_printf_r",
        "_fprintf_r",
        "_vfprintf_r",
        "_vprintf_r",
        "printf",
        "fprintf",
        "vfprintf",
        "vprintf",
        "puts",
        "fputs",
        "fputc",
        "putchar",
        "_puts_r",
        "_fputs_r",
        "_putchar_r",
        "_write_r",
        "write",
    ] {
        if let Some(&pc) = symbol_addrs.get(*sym) {
            thunks.push((pc, rom_thunks::nop_return_zero));
        }
    }
    // esp_chip_info needs more than nop — has to fill the output struct
    // with a plausible revision so the firmware's chip_revision >= min
    // assert passes.
    if let Some(&pc) = symbol_addrs.get("esp_chip_info") {
        thunks.push((pc, rom_thunks::esp_chip_info_stub));
    }
    // __getreent must return a non-NULL pointer to a zeroed reent struct.
    // Real silicon's per-task reent is set up by FreeRTOS task local
    // storage which we don't model — return a fixed pointer into DRAM
    // (always zeroed by RamPeripheral::new). ESP32-classic-specific
    // address; an `esp32s3` profile (if/when added) would need its own
    // version of this thunk pointing at S3's DRAM range.
    if let Some(&pc) = symbol_addrs.get("__getreent") {
        thunks.push((pc, rom_thunks::getreent_dram_fake_ptr));
    }
    // esp_timer_impl_get_counter_reg must return a monotonically increasing
    // value, otherwise polling-loop callers (esp-idf flash HAL, FreeRTOS
    // timeout helpers) spin forever.
    if let Some(&pc) = symbol_addrs.get("esp_timer_impl_get_counter_reg") {
        thunks.push((pc, rom_thunks::monotonic_counter_32));
    }
    // esp_clk_cpu_freq() — FreeRTOS divides CPU freq by tick rate to set
    // _xt_tick_divisor; without a meaningful value, divisor is 0 and the
    // timer ISR re-fires every CCOUNT cycle, pinning CPU 0 in the tick hook.
    if let Some(&pc) = symbol_addrs.get("esp_clk_cpu_freq") {
        thunks.push((pc, rom_thunks::esp_clk_cpu_freq_240mhz));
    }
    // Xtensa HAL register-window-file spill. The HAL impl walks WS bits
    // and spills each live slot's a0..a3 to its stack save area — but
    // our sim's transparent shadow-spill on CALL{n} leaves WS=1 on
    // displaced slots while the AR file has the callee's data, so the
    // HAL walk reads garbage (callee's a1 is often 0 → store to
    // 0xfffffff0 traps). The custom thunk emulates the spill using
    // shadow-stack snapshots when available. Only wired for
    // arduino-esp32 profile — agentdeck's call path doesn't hit the
    // crash, and replacing the real function with the thunk regresses
    // a working flow (spill writes valid save-area data the callee
    // later reads).
    if args.profile == "arduino-esp32" {
        // Only the `_nw` leaf (the spill loop that would trap) is thunked;
        // the `xthal_window_spill` wrapper is a thin CALL{n}-entered
        // PS-save shell that must run natively (its real ENTRY/RETW manage
        // the window). Thunking the wrapper returns via a0 = the caller's
        // return address, corrupting the first-task dispatch.
        if let Some(&pc) = symbol_addrs.get("xthal_window_spill_nw") {
            thunks.push((pc, rom_thunks::xthal_window_spill_thunk));
        }
    }
    // xQueueCreateMutexStatic returns the caller's static buffer as the
    // handle. Callers (esp_newlib_locks_init in particular) assert that the
    // returned handle equals the buffer they passed in — a nop_return_zero
    // stub fails that check.
    if let Some(&pc) = symbol_addrs.get("xQueueCreateMutexStatic") {
        thunks.push((pc, rom_thunks::x_queue_create_mutex_static_echo));
    }
    // pxCurrentTCB symbol → feed into the rom_thunks side so the
    // xTaskGetCurrentTaskHandle thunk can read it. Arduino-ESP32's
    // main_task self-deletes after app_main returns via vTaskDelete(NULL),
    // which depends on this getter.
    if let Some(&addr) = symbol_addrs.get("pxCurrentTCB") {
        rom_thunks::PX_CURRENT_TCB_ADDR.with(|s| s.set(Some(addr)));
    }
    if let Some(&pc) = symbol_addrs.get("xTaskGetCurrentTaskHandle") {
        thunks.push((pc, rom_thunks::x_task_get_current_task_handle));
    }
    // xQueueSemaphoreTake on the NULL mutex returned by our stubbed
    // xQueueCreateMutex would assert. Force pdTRUE so SPIClass /
    // beginTransaction etc. proceed as if they got the lock.
    if let Some(&pc) = symbol_addrs.get("xQueueSemaphoreTake") {
        thunks.push((pc, rom_thunks::return_pd_true));
    }
    if let Some(&pc) = symbol_addrs.get("xQueueGenericSend") {
        thunks.push((pc, rom_thunks::return_pd_true));
    }
    // ulTaskGenericNotifyTake — gated to arduino-esp32 only.
    // the reference firmware has its own well-modeled lock-acquire path that
    // expects a proper "block-then-wake" semantic; stubbing it to
    // return pdTRUE causes the lock-acquire to skip its setup and
    // later trip __assert_func inside lock_acquire_generic.
    if args.profile == "arduino-esp32" {
        if let Some(&pc) = symbol_addrs.get("ulTaskGenericNotifyTake") {
            thunks.push((pc, rom_thunks::return_pd_true));
        }
    }
    if let Some(&pc) = symbol_addrs.get("spiStartBus") {
        thunks.push((pc, rom_thunks::spi_start_bus_fake));
    }
    // Pre-initialize the Arduino global SPI object's _spi field. The
    // sketch never calls SPI.begin() — GxEPD2 just assumes SPI is up.
    // SPIClass layout: offset 0 = _spi_num (u8), offset 4 = _spi (spi_t*).
    // Our fake spi_t lives at 0x3FFDF020 with dev=0x3FF65000 (SPI3 base);
    // see rom_thunks::spi_start_bus_fake.
    // SPIClass::beginTransaction lazy-init: the sketch never calls
    // SPI.begin() so SPI._spi is NULL at first use. The thunk replaces
    // beginTransaction with one that lazy-allocates a fake spi_t pointing
    // at the correct SPI peripheral base, then returns pdTRUE.
    if let Some(&pc) = symbol_addrs.get("_ZN8SPIClass16beginTransactionE11SPISettings") {
        thunks.push((pc, rom_thunks::spi_class_begin_transaction));
    }
    // No GxEPD2 _writeCommand / _writeData bypass. The real compiled
    // GxEPD2_EPD::_writeCommand/_writeData run: digitalWrite(DC=GPIO17) →
    // SPI.transfer(byte) → spiTransferByteNL writes the SPI3 FIFO/MOSI_DLEN/
    // CMD.USR registers, and the Esp32Spi peripheral drains the byte to the
    // panel framed by the latched DC GPIO. Verified end-to-end against the real
    // PlatformIO firmware.elf (431 real SPI3 transactions → panel refresh) by
    // tests/e2e_labwired_ereader.rs. The arduino-esp32 panel attach above sets
    // the panel's DC source to GPIO17 so the framing is real.
    // Optional debug: install vListInsert short-circuit thunk that dumps
    // list state for first 20 calls. Used to diagnose SMP race issues in
    // the FreeRTOS scheduler. Enable with `LABWIRED_DEBUG_VLIST=1`.
    if std::env::var("LABWIRED_DEBUG_VLIST").is_ok() {
        if let Some(&pc) = symbol_addrs.get("vListInsert") {
            thunks.push((pc, rom_thunks::vlist_insert_debug));
        }
    }
    // the reference firmware-only WiFi + sendHello thunks. Only install for that profile
    // — sketches without those symbols wouldn't trip them anyway.
    if args.profile == "agentdeck" {
        thunks.extend_from_slice(&[
            (0x400d_de98, rom_thunks::nop_return_zero), // WifiWsLink::begin
            (0x400d_dccc, rom_thunks::nop_return_zero), // WifiWsLink::loop
            (0x400e_0034, rom_thunks::nop_return_zero), // anon-ns sendHello
        ]);
    }
    eprintln!(
        "labwired-cli snapshot: installing {} thunks ({} resolved from ELF symbols)",
        thunks.len(),
        symbol_addrs.len(),
    );
    for &(pc, f) in &thunks {
        if let Err(e) = machine.bus.install_flash_thunk(pc, f) {
            eprintln!("error: install_flash_thunk @ {:#x}: {e}", pc);
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }
    }

    // Fake esp_image_header_t (24 bytes) at 0x3F40_0000, entry = ELF entry.
    let entry: u32 = program_image.entry_point as u32;
    let header: [u8; 24] = [
        0xE9,
        0x01,
        0x00,
        0x00,
        (entry & 0xFF) as u8,
        ((entry >> 8) & 0xFF) as u8,
        ((entry >> 16) & 0xFF) as u8,
        ((entry >> 24) & 0xFF) as u8,
        0xEE,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    for (i, &b) in header.iter().enumerate() {
        let _ = machine.bus.write_u8(0x3F40_0000 + i as u64, b);
    }

    // IPI bridge state — DPORT FROM_CPU intmatrix mapping observed each
    // cycle, raised on the CPU as an internal interrupt edge.
    let mut from_cpu_bit0: Option<u8> = None;
    let mut from_cpu_bit1: Option<u8> = None;
    let mut last_from_cpu0_val: u32 = 0;
    let mut last_from_cpu1_val: u32 = 0;

    eprintln!(
        "labwired-cli snapshot: stepping firmware to cycle {}",
        args.steps
    );
    let observers: Vec<std::sync::Arc<dyn labwired_core::SimulationObserver>> = Vec::new();
    let config = labwired_core::SimulationConfig::default();
    let mut i: u64 = 0;
    let progress = args.progress_every;
    while i < args.steps {
        if let Ok(v) = machine.bus.read_u32(0x3FF0_0164) {
            let bit = (v & 0x1F) as u8;
            if v != 0 && bit < 32 {
                from_cpu_bit0 = Some(bit);
            }
        }
        if let Ok(v) = machine.bus.read_u32(0x3FF0_0168) {
            let bit = (v & 0x1F) as u8;
            if v != 0 && bit < 32 {
                from_cpu_bit1 = Some(bit);
            }
        }
        if let Ok(v0) = machine.bus.read_u32(0x3FF0_00DC) {
            if v0 != 0 && v0 != last_from_cpu0_val {
                if let Some(bit) = from_cpu_bit0 {
                    machine.cpu.raise_interrupt_bits(1u32 << bit);
                }
                let _ = machine.bus.write_u32(0x3FF0_00DC, 0);
            }
            last_from_cpu0_val = 0;
        }
        if let Ok(v1) = machine.bus.read_u32(0x3FF0_00E0) {
            if v1 != 0 && v1 != last_from_cpu1_val {
                if let Some(bit) = from_cpu_bit1 {
                    machine.cpu.raise_interrupt_bits(1u32 << bit);
                }
                let _ = machine.bus.write_u32(0x3FF0_00E0, 0);
            }
            last_from_cpu1_val = 0;
        }
        // Re-stamp the dual-core handshake bytes every 10k cycles so
        // start_other_core / do_other_cpu_settings keep seeing them as
        // "up." preset-PC path: byte-for-byte mirror of the original
        // install_esp32_arduino_quirks. Auto-discovery path: write to
        // each resolved symbol's [0]+[1] slots.
        if i.is_multiple_of(10_000) {
            if args.profile == "agentdeck" {
                let _ = machine.bus.write_u8(0x3FFC_6F04, 0x01);
                let _ = machine.bus.write_u8(0x3FFC_6F01, 0x01);
                let _ = machine.bus.write_u8(0x3FFC_6F02, 0x01);
                let _ = machine.bus.write_u8(0x3FFC_6FFD, 0x01);
                let _ = machine.bus.write_u8(0x3FFC_6FFE, 0x01);
                let _ = machine.bus.write_u8(0x3FFC_7190, 0x01);
            } else {
                if s_resume_cores != 0 {
                    let _ = machine.bus.write_u8(s_resume_cores as u64, 0x01);
                }
                if s_cpu_up != 0 {
                    let _ = machine.bus.write_u8(s_cpu_up as u64, 0x01);
                    let _ = machine.bus.write_u8(s_cpu_up as u64 + 1, 0x01);
                }
                if s_cpu_inited != 0 {
                    let _ = machine.bus.write_u8(s_cpu_inited as u64, 0x01);
                    let _ = machine.bus.write_u8(s_cpu_inited as u64 + 1, 0x01);
                }
                if s_system_inited != 0 {
                    let _ = machine.bus.write_u8(s_system_inited as u64, 0x01);
                    let _ = machine.bus.write_u8(s_system_inited as u64 + 1, 0x01);
                }
                if s_other_cpu_startup_done != 0 {
                    let _ = machine.bus.write_u8(s_other_cpu_startup_done as u64, 0x01);
                }
            }
        }
        match machine.cpu.step(&mut machine.bus, &observers, &config) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(_)) => {}
            Err(e) => {
                eprintln!(
                    "error: sim step at cycle {i} pc=0x{:08x}: {e}",
                    machine.cpu.get_pc()
                );
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
        }
        // Dual-core: snapshot capture bypasses Machine::step, so the
        // appcpu-release + cpu1.step loop has to live here too. Plain
        // round-robin per-instruction interleaving — matches the chip's
        // true parallelism within the granularity of one instruction.
        // S32C1I is atomic within step() so spinlocks work correctly.
        if let Some(cpu1) = machine.cpu_secondary.as_mut() {
            if let Some(boot_addr) =
                labwired_core::peripherals::esp32s3::rom_thunks::APPCPU_BOOT_ADDR.with(|s| s.take())
            {
                cpu1.set_pc(boot_addr);
                cpu1.set_sp(appcpu_initial_sp);
                // Keep cpu1 halted while loopTask is patched to PRO_CPU
                // (avoids SMP race on shared FreeRTOS lists until DC7).
                // Set LABWIRED_DUALCORE_RUN=1 to also run cpu1.
                if std::env::var("LABWIRED_DUALCORE_RUN").is_ok() {
                    cpu1.unhalt();
                }
            }
            match cpu1.step(&mut machine.bus, &observers, &config) {
                Ok(()) => {}
                Err(SimulationError::BreakpointHit(_)) => {}
                Err(e) => {
                    eprintln!(
                        "error: sim step cpu1 at cycle {i} pc=0x{:08x}: {e}",
                        cpu1.get_pc()
                    );
                    return ExitCode::from(EXIT_RUNTIME_ERROR);
                }
            }
        }
        machine.bus.tick_peripherals_with_costs();
        i += 1;
        if progress > 0 && i.is_multiple_of(progress) {
            let cpu1_state = match machine.cpu_secondary.as_ref() {
                Some(cpu1) => format!("  cpu1=0x{:08x}", cpu1.get_pc()),
                None => String::new(),
            };
            eprintln!(
                "  step {i:>10}  pc=0x{:08x}{cpu1_state}",
                machine.cpu.get_pc()
            );
            // Optional DC7 debug: dump vListInsert state on spin. Set
            // LABWIRED_DEBUG_LIST=1 to enable. Shows cpu intlevel,
            // xTaskQueueMutex state, pxList walk, and newItem state.
            if std::env::var("LABWIRED_DEBUG_LIST").is_ok() {
                eprintln!(
                    "    cpu0 intlevel={} a0=0x{:08x} a1=0x{:08x}",
                    machine.cpu.intlevel(),
                    machine.cpu.get_register(0),
                    machine.cpu.get_register(1)
                );
                let mux_owner = machine.bus.read_u32(0x3ffbf3b8).unwrap_or(0xDEAD);
                let mux_count = machine.bus.read_u32(0x3ffbf3bc).unwrap_or(0xDEAD);
                eprintln!("    xTaskQueueMutex.owner=0x{mux_owner:08x} .count={mux_count}");
                if let Some(cpu1) = machine.cpu_secondary.as_ref() {
                    eprintln!(
                        "    cpu1 intlevel={} a0=0x{:08x} a1=0x{:08x}",
                        cpu1.intlevel(),
                        cpu1.get_register(0),
                        cpu1.get_register(1)
                    );
                }
                let px_list = machine.cpu.get_register(2);
                let r = |off: u32| {
                    machine
                        .bus
                        .read_u32((px_list + off) as u64)
                        .unwrap_or(0xDEAD)
                };
                eprintln!(
                    "    cpu0 pxList=0x{px_list:08x} num={} idx=0x{:08x} end.val=0x{:08x} end.next=0x{:08x} end.prev=0x{:08x}",
                    r(0), r(4), r(8), r(12), r(16)
                );
                if let Some(cpu1) = machine.cpu_secondary.as_ref() {
                    let px_list1 = cpu1.get_register(2);
                    let r1 = |off: u32| {
                        machine
                            .bus
                            .read_u32((px_list1 + off) as u64)
                            .unwrap_or(0xDEAD)
                    };
                    eprintln!(
                        "    cpu1 pxList=0x{px_list1:08x} num={} idx=0x{:08x} end.val=0x{:08x} end.next=0x{:08x} end.prev=0x{:08x}",
                        r1(0), r1(4), r1(8), r1(12), r1(16)
                    );
                }
                let mut iter = r(12);
                let end_addr = px_list + 8;
                for hop in 0..6 {
                    if iter == end_addr {
                        eprintln!("      [hop {hop}] -> xListEnd (terminator)");
                        break;
                    }
                    let item_next = machine.bus.read_u32((iter + 4) as u64).unwrap_or(0xDEAD);
                    let item_val = machine.bus.read_u32(iter as u64).unwrap_or(0xDEAD);
                    eprintln!("      [hop {hop}] item=0x{iter:08x} val=0x{item_val:08x} next=0x{item_next:08x}");
                    iter = item_next;
                }
                let new_item = machine.cpu.get_register(3);
                let ri = |off: u32| {
                    machine
                        .bus
                        .read_u32((new_item + off) as u64)
                        .unwrap_or(0xDEAD)
                };
                eprintln!(
                    "    cpu0 newItem=0x{new_item:08x} item.val=0x{:08x} item.next=0x{:08x} item.prev=0x{:08x} item.owner=0x{:08x}",
                    ri(0), ri(4), ri(8), ri(12)
                );
            }
        }
    }

    // Sanity-check the captured state — for an `agentdeck` profile we
    // expect the SSD1680 panel to have been driven through at least one
    // refresh cycle by the time the snapshot lands. Print this so the
    // operator can tell "yes, this snapshot is post-paint" without
    // re-running the playground.
    if let Some(idx) = machine.bus.find_peripheral_index_by_name("spi3") {
        if let Some(any) = machine.bus.peripherals[idx].dev.as_any() {
            if let Some(spi3) = any.downcast_ref::<Esp32Spi>() {
                // Diagnostic: dump the full captured wire stream when asked, so
                // we can inspect the 0x24/0x26 RAM-write payloads end-to-end.
                if let Ok(path) = std::env::var("LABWIRED_DUMP_SPI") {
                    let _ = std::fs::write(&path, spi3.captured_bytes());
                    eprintln!(
                        "labwired-cli snapshot: dumped {} captured spi3 bytes to {path}",
                        spi3.captured_bytes().len()
                    );
                }
                eprintln!(
                    "labwired-cli snapshot: spi3 transactions={}",
                    spi3.transactions(),
                );
                let cap = spi3.captured_bytes();
                if !cap.is_empty() {
                    let head_n = cap.len().min(120);
                    let head_hex: Vec<String> =
                        cap[..head_n].iter().map(|b| format!("{b:02x}")).collect();
                    eprintln!(
                        "labwired-cli snapshot: first {head_n} spi3 bytes: {}",
                        head_hex.join(" ")
                    );
                    if cap.len() > 240 {
                        let tail = &cap[cap.len() - 120..];
                        let tail_hex: Vec<String> =
                            tail.iter().map(|b| format!("{b:02x}")).collect();
                        eprintln!(
                            "labwired-cli snapshot: last 120 spi3 bytes: {}",
                            tail_hex.join(" ")
                        );
                    }
                }
                for attached in &spi3.attached_devices {
                    if let Some(panel_any) = attached.as_any() {
                        if let Some(panel) = panel_any.downcast_ref::<Ssd1680Tricolor290>() {
                            let bp = panel.black_plane();
                            let non_ff = bp.iter().filter(|&&b| b != 0xFF).count();
                            eprintln!(
                                "labwired-cli snapshot: panel (ssd1680) state — refresh_generation={}, power_on={}, black-plane non-FF bytes={}/{}",
                                panel.refresh_generation(),
                                panel.power_on(),
                                non_ff,
                                bp.len(),
                            );
                        } else if let Some(panel) = panel_any.downcast_ref::<Uc8151dTricolor290>() {
                            let bp = panel.black_plane();
                            let non_ff = bp.iter().filter(|&&b| b != 0xFF).count();
                            let rp = panel.red_plane();
                            let non_ff_red = rp.iter().filter(|&&b| b != 0xFF).count();
                            eprintln!(
                                "labwired-cli snapshot: panel (uc8151d) state — refresh_generation={}, power_on={}, black-plane non-FF bytes={}/{}, red-plane non-FF bytes={}/{}",
                                panel.refresh_generation(),
                                panel.power_on(),
                                non_ff,
                                bp.len(),
                                non_ff_red,
                                rp.len(),
                            );
                            // Render the panel as a PPM next to the
                            // snapshot output so an operator can visually
                            // confirm "yes, this looks like the real-HW
                            // panel image" before shipping the snapshot.
                            let (w, h) = panel.dimensions();
                            let stride = w / 8;
                            let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
                            for y in 0..h {
                                for x in 0..w {
                                    let idx = y * stride + x / 8;
                                    let bit = 7 - (x % 8);
                                    let black_bit = (bp[idx] >> bit) & 1;
                                    let red_bit = (rp[idx] >> bit) & 1;
                                    let (r, g, b) = if red_bit == 0 {
                                        (220u8, 30u8, 40u8)
                                    } else if black_bit == 0 {
                                        (0u8, 0u8, 0u8)
                                    } else {
                                        (245u8, 245u8, 240u8)
                                    };
                                    ppm.extend_from_slice(&[r, g, b]);
                                }
                            }
                            let ppm_path = args.output.with_extension("ppm");
                            if std::fs::write(&ppm_path, &ppm).is_ok() {
                                eprintln!(
                                    "labwired-cli snapshot: panel PPM written to {}",
                                    ppm_path.display()
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    let snap = machine.take_runtime_snapshot();
    let bytes = snap.to_bytes();

    if let Some(parent) = args.output.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&args.output, &bytes) {
        eprintln!("error: write {:?}: {e}", args.output);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    eprintln!(
        "labwired-cli snapshot: wrote {} bytes to {:?} (pc=0x{:08x} after {} cycles)",
        bytes.len(),
        args.output,
        machine.cpu.get_pc(),
        args.steps,
    );
    // Phase 3.2 JIT pilot (issue #124): report block hit count if the
    // build was compiled with `--features jit-core`. Without the feature
    // the trait default returns 0 and this line is harmless.
    let jit_hits = machine.cpu.jit_hit_count();
    if jit_hits > 0 {
        eprintln!("labwired-cli snapshot: jit block hits: {jit_hits}");
    }
    ExitCode::from(EXIT_PASS)
}

fn run_asset(args: AssetArgs) -> ExitCode {
    match args.command {
        AssetCommands::ImportSvd(a) => run_import_svd(a),
        AssetCommands::Codegen(a) => run_codegen(a),
        AssetCommands::Init(a) => run_asset_init(a),
        AssetCommands::AddPeripheral(a) => run_asset_add_peripheral(a),
        AssetCommands::Validate(a) => asset_validation::run_validate(a),
        AssetCommands::ListChips(a) => asset_validation::run_list_chips(a),
        AssetCommands::Create(a) => run_asset_create(a),
        AssetCommands::Verify(a) => run_asset_verify(a),
        AssetCommands::ValidateComponent(a) => component_validation::run_validate_component(a),
        AssetCommands::IngestSvd(a) => run_ingest_svd(a),
    }
}

fn run_asset_add_peripheral(args: AddPeripheralArgs) -> ExitCode {
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

fn resolve_chip_descriptor_path(chip: &str) -> Option<PathBuf> {
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

fn run_asset_verify(args: VerifyArgs) -> ExitCode {
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

fn run_asset_create(args: CreateArgs) -> ExitCode {
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

fn run_asset_init(args: InitArgs) -> ExitCode {
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

fn run_codegen(args: CodegenArgs) -> ExitCode {
    info!("Generating Rust code from IR: {:?}", args.input);

    let file = match std::fs::File::open(&args.input) {
        Ok(f) => f,
        Err(e) => {
            error!("Failed to open IR file: {}", e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let device: labwired_ir::IrDevice = match serde_json::from_reader(file) {
        Ok(d) => d,
        Err(e) => {
            error!("Failed to parse IR JSON: {}", e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let mut output_code = String::new();
    output_code.push_str("// Generated by LabWired Codegen\n");
    output_code.push_str("#![allow(non_camel_case_types)]\n");
    output_code.push_str("#![allow(non_snake_case)]\n");
    // output_code.push_str("use labwired_core::Peripheral;\n"); // Not strictly needed yet as we generate structs
    // output_code.push_str("use labwired_core::SimResult;\n\n");

    for (name, peripheral) in &device.peripherals {
        match labwired_codegen::PeripheralGenerator::generate(peripheral) {
            Ok(code) => {
                output_code.push_str(&code);
                output_code.push_str("\n\n");
            }
            Err(e) => {
                error!("Failed to generate code for peripheral {}: {}", name, e);
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        }
    }

    if let Err(e) = std::fs::write(&args.output, output_code) {
        error!("Failed to write output file: {}", e);
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    info!("Successfully wrote Rust code to {:?}", args.output);
    ExitCode::from(EXIT_PASS)
}

fn run_import_svd(args: ImportSvdArgs) -> ExitCode {
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

fn run_ingest_svd(args: IngestSvdArgs) -> ExitCode {
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

fn run_interactive(cli: Cli) -> ExitCode {
    info!("Starting LabWired Simulator");

    let Some(firmware) = &cli.firmware else {
        emit_error(
            cli.json,
            "ConfigError",
            "Missing required --firmware argument".to_string(),
            None,
            EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    };

    let system_path = cli.system.clone();
    let bus = match labwired_core::system::builder::build_system_bus(system_path.as_deref()) {
        Ok(bus) => bus,
        Err(e) => {
            emit_error(
                cli.json,
                "ConfigError",
                format!("{:#}", e),
                Some(serde_json::json!({
                    "system_path": system_path.as_ref().map(|p| p.display().to_string()),
                })),
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    info!("Loading firmware: {:?}", firmware);
    let program = match labwired_loader::load_elf(firmware) {
        Ok(program) => program,
        Err(e) => {
            emit_error(
                cli.json,
                "LoadError",
                format!("{:#}", e),
                Some(serde_json::json!({
                    "firmware_path": firmware.display().to_string(),
                })),
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    info!("Firmware Loaded Successfully!");
    info!("Entry Point: {:#x}", program.entry_point);

    let metrics = std::sync::Arc::new(labwired_core::metrics::PerformanceMetrics::new());

    let cpu_arch = if let Some(sys_path) = &system_path {
        match labwired_config::SystemManifest::from_file(sys_path) {
            Ok(manifest) => {
                let chip_path = sys_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(&manifest.chip);
                match labwired_config::ChipDescriptor::from_file(&chip_path) {
                    Ok(c) => c.arch,
                    Err(e) => {
                        emit_error(
                            cli.json,
                            "ConfigError",
                            format!("Failed to parse chip descriptor: {:#}", e),
                            Some(serde_json::json!({
                                "chip_path": chip_path.display().to_string(),
                            })),
                            EXIT_CONFIG_ERROR,
                        );
                        return ExitCode::from(EXIT_CONFIG_ERROR);
                    }
                }
            }
            Err(e) => {
                emit_error(
                    cli.json,
                    "ConfigError",
                    format!("Failed to parse system manifest: {:#}", e),
                    Some(serde_json::json!({
                        "system_path": sys_path.display().to_string(),
                    })),
                    EXIT_CONFIG_ERROR,
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        }
    } else {
        // Default to Arm if no system config provided (backward compatibility)
        labwired_config::Arch::Arm
    };

    if program.arch != labwired_core::Arch::Unknown {
        // Map core::Arch to config::Arch for comparison
        let prog_arch = match program.arch {
            labwired_core::Arch::Arm => labwired_config::Arch::Arm,
            labwired_core::Arch::RiscV => labwired_config::Arch::RiscV,
            labwired_core::Arch::XtensaLx7 => labwired_config::Arch::Xtensa,
            _ => labwired_config::Arch::Unknown,
        };

        if prog_arch != cpu_arch {
            tracing::warn!(
                "Architecture Mismatch! Config expects {:?}, but ELF is {:?}",
                cpu_arch,
                prog_arch
            );
        }
    }

    match cpu_arch {
        labwired_config::Arch::Arm => run_interactive_arm(cli, bus, program, metrics),
        labwired_config::Arch::RiscV => run_interactive_riscv(cli, bus, program, metrics),
        labwired_config::Arch::Xtensa => run_interactive_xtensa(cli, bus, program, metrics),
        _ => {
            emit_error(
                cli.json,
                "ConfigError",
                format!("Unsupported architecture: {:?}", cpu_arch),
                Some(serde_json::json!({
                    "architecture": format!("{:?}", cpu_arch),
                })),
                EXIT_CONFIG_ERROR,
            );
            ExitCode::from(EXIT_CONFIG_ERROR)
        }
    }
}

fn run_machine(args: MachineArgs) -> ExitCode {
    match args.command {
        MachineCommands::Load(load_args) => run_machine_load(load_args),
    }
}

fn run_machine_load(args: LoadArgs) -> ExitCode {
    info!("Loading machine from snapshot: {:?}", args.snapshot);

    let f = match std::fs::File::open(&args.snapshot) {
        Ok(f) => f,
        Err(e) => {
            error!("Failed to open snapshot {:?}: {}", args.snapshot, e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let snapshot_data: Snapshot = match serde_json::from_reader(f) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to parse snapshot {:?}: {}", args.snapshot, e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let (cpu_snapshot, peripherals_snapshot, config) = match snapshot_data {
        Snapshot::Interactive {
            cpu,
            peripherals,
            config,
            ..
        } => (cpu, peripherals, config),
        _ => {
            error!("Unsupported snapshot type for loading");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Reconstruct bus
    let mut bus = match labwired_core::system::builder::build_system_bus(config.system.as_deref()) {
        Ok(bus) => bus,
        Err(e) => {
            error!("Failed to reconstruct bus: {:#}", e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Load original firmware (required for memory content that isn't in snapshot yet)
    // Note: Our snapshot currently doesn't include full RAM/Flash dumps to keep it small.
    // So we MUST load the firmware first.
    let program = match labwired_loader::load_elf(&config.firmware) {
        Ok(p) => p,
        Err(e) => {
            error!(
                "Failed to load original firmware {:?}: {:#}",
                config.firmware, e
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let arch = match program.arch {
        labwired_core::Arch::Arm => labwired_config::Arch::Arm,
        labwired_core::Arch::RiscV => labwired_config::Arch::RiscV,
        labwired_core::Arch::XtensaLx7 => labwired_config::Arch::Xtensa,
        _ => {
            error!("Unknown architecture in firmware");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let metrics = Arc::new(labwired_core::metrics::PerformanceMetrics::new());

    match arch {
        labwired_config::Arch::Arm => {
            let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
            let mut machine = labwired_core::Machine::new(cpu, bus);
            machine.observers.push(metrics.clone());
            if let Err(e) = machine.load_firmware(&program) {
                error!("Failed to load firmware: {}", e);
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }

            // Apply snapshot
            machine.cpu.apply_snapshot(&cpu_snapshot);
            for ps in peripherals_snapshot {
                if let Some(state) = ps.state {
                    // Find peripheral by name and restore
                    for p in &mut machine.bus.peripherals {
                        if p.name == ps.name {
                            if let Err(e) = p.dev.restore(state.clone()) {
                                error!("Failed to restore peripheral {}: {}", p.name, e);
                            }
                            break;
                        }
                    }
                }
            }

            info!("Resuming simulation (ARM)...");
            let cli = Cli {
                firmware: Some(config.firmware),
                system: config.system,
                snapshot: None,
                breakpoint: vec![],
                trace: args.trace,
                max_steps: args.max_steps.unwrap_or(config.max_steps),
                gdb: None,
                command: None,
                json: false,
                vcd: None,
            };
            run_simulation_loop(&cli, &mut machine, &metrics);
            ExitCode::from(EXIT_PASS)
        }
        labwired_config::Arch::RiscV => {
            let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
            let mut machine = labwired_core::Machine::new(cpu, bus);
            machine.observers.push(metrics.clone());
            if let Err(e) = machine.load_firmware(&program) {
                error!("Failed to load firmware: {}", e);
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }

            // Apply snapshot
            machine.cpu.apply_snapshot(&cpu_snapshot);
            for ps in peripherals_snapshot {
                if let Some(state) = ps.state {
                    for p in &mut machine.bus.peripherals {
                        if p.name == ps.name {
                            if let Err(e) = p.dev.restore(state.clone()) {
                                error!("Failed to restore peripheral {}: {}", p.name, e);
                            }
                            break;
                        }
                    }
                }
            }

            info!("Resuming simulation (RISC-V)...");
            let cli = Cli {
                firmware: Some(config.firmware),
                system: config.system,
                snapshot: None,
                breakpoint: vec![],
                trace: args.trace,
                max_steps: args.max_steps.unwrap_or(config.max_steps),
                gdb: None,
                command: None,
                json: false,
                vcd: None,
            };
            run_simulation_loop(&cli, &mut machine, &metrics);
            ExitCode::from(EXIT_PASS)
        }
        labwired_config::Arch::Xtensa => {
            let cpu = labwired_core::system::xtensa::configure_xtensa(&mut bus);
            let mut machine = labwired_core::Machine::new(cpu, bus);
            machine.observers.push(metrics.clone());
            if let Err(e) = machine.load_firmware(&program) {
                error!("Failed to load firmware: {}", e);
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }

            machine.cpu.apply_snapshot(&cpu_snapshot);
            for ps in peripherals_snapshot {
                if let Some(state) = ps.state {
                    for p in &mut machine.bus.peripherals {
                        if p.name == ps.name {
                            if let Err(e) = p.dev.restore(state.clone()) {
                                error!("Failed to restore peripheral {}: {}", p.name, e);
                            }
                            break;
                        }
                    }
                }
            }

            info!("Resuming simulation (Xtensa)...");
            let cli = Cli {
                firmware: Some(config.firmware),
                system: config.system,
                snapshot: None,
                breakpoint: vec![],
                trace: args.trace,
                max_steps: args.max_steps.unwrap_or(config.max_steps),
                gdb: None,
                command: None,
                json: false,
                vcd: None,
            };
            run_simulation_loop(&cli, &mut machine, &metrics);
            ExitCode::from(EXIT_PASS)
        }
        _ => {
            error!("Unsupported architecture for snapshot load");
            ExitCode::from(EXIT_CONFIG_ERROR)
        }
    }
}

/// Fast-boot an ARM Cortex-M firmware from a chip YAML and ELF path.
///
/// Builds the bus directly from the chip descriptor (no system manifest
/// required — the chip YAML's `peripherals` list is sufficient for raw-register
/// fixture firmware).  UART bytes are streamed to stdout so the TIER1 protocol
/// lines are visible to callers that pipe stdout.  Exits when the step limit
/// is reached or the firmware halts.
fn run_firmware_arm(args: &RunArgs, chip_yaml: &str) -> ExitCode {
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::SystemBus;
    use labwired_core::system::cortex_m::configure_cortex_m;
    use labwired_core::Machine;
    use std::io::Write;

    // Parse the chip descriptor.
    let chip = match serde_yaml::from_str::<ChipDescriptor>(chip_yaml) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: cannot parse chip YAML: {e}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Synthesise a minimal system manifest (no external devices) so the bus
    // builder has something to work with.  The chip path is already absolute
    // because `chip_yaml` was read from `args.chip`.
    let manifest_yaml = format!(
        "name: \"tier1-run\"\nchip: \"{}\"\nexternal_devices: []\n",
        args.chip.display()
    );
    let mut manifest = match serde_yaml::from_str::<SystemManifest>(&manifest_yaml) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: cannot build minimal manifest: {e}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    // Chip field must be an absolute path string; already is (args.chip is absolute
    // relative to the caller's cwd, which is the workspace root per run_target).
    manifest.chip = args.chip.to_string_lossy().into_owned();

    // Build the bus.
    let mut bus = match SystemBus::from_config(&chip, &manifest) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot build bus from chip config: {e}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Attach stdout echo to every UART so protocol lines flow through.
    // `echo_stdout = true` prints each byte as it arrives.
    let uart_sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), true);

    // Configure Cortex-M CPU.
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    // Load ELF.
    let image = match labwired_loader::load_elf(&args.firmware) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("error: cannot load firmware ELF {:?}: {e}", args.firmware);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    if let Err(e) = machine.load_firmware(&image) {
        eprintln!("error: cannot map firmware into bus: {e}");
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    // Run the step loop.
    let limit = args.max_steps.unwrap_or(u64::MAX);
    for _ in 0..limit {
        match machine.step() {
            Ok(()) => {}
            Err(e) => {
                eprintln!("labwired run (arm): simulation error: {e}");
                // Non-fatal for TIER1: the protocol may already be complete.
                break;
            }
        }
    }

    // Flush stdout.
    let _ = std::io::stdout().flush();
    ExitCode::from(EXIT_PASS)
}

fn run_interactive_arm(
    cli: Cli,
    mut bus: labwired_core::bus::SystemBus,
    program: labwired_core::memory::ProgramImage,
    metrics: Arc<labwired_core::metrics::PerformanceMetrics>,
) -> ExitCode {
    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = labwired_core::Machine::new(cpu, bus);
    machine.observers.push(metrics.clone());

    if let Some(vcd_path) = &cli.vcd {
        let file = std::fs::File::create(vcd_path).expect("Failed to create VCD file");
        let observer = std::sync::Arc::new(vcd_trace::VcdObserver::new(file));
        machine.observers.push(observer);
    }

    if let Err(e) = machine.load_firmware(&program) {
        tracing::error!("Failed to load firmware into memory: {}", e);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    info!("Starting Simulation (ARM Cortex-M)...");
    info!(
        "Initial PC: {:#x}, SP: {:#x}",
        machine.cpu.pc, machine.cpu.sp
    );

    // Check if GDB server is requested
    if let Some(port) = cli.gdb {
        let server = labwired_gdbstub::GdbServer::new(port);
        if let Err(e) = server.run(machine) {
            error!("GDB server failed: {}", e);
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }
        return ExitCode::from(EXIT_PASS);
    }

    let result = run_simulation_loop(&cli, &mut machine, &metrics);

    if let Some(path) = &cli.snapshot {
        // Need to reconstruct full paths or pass them?
        // cli.firmware is Option<PathBuf>, but checking run_interactive, it ensures firmware is set.
        // But run_interactive passed `program` not paths.
        // Creating cli passes ownership. `cli` has `firmware`.
        // `cli.system` is `Option<PathBuf>`.

        let firmware_path = cli.firmware.as_ref().expect("Firmware path required");
        let system_path = cli.system.as_ref();

        write_interactive_snapshot(
            path,
            &metrics,
            &machine,
            InteractiveSnapshotInputs {
                firmware_path,
                system_path,
                max_steps: cli.max_steps,
                steps_executed: result.steps_executed,
                stop_reason: result.stop_reason,
                message: result.stop_message,
            },
        );
    }

    report_metrics(&cli, &machine.cpu, &metrics);
    ExitCode::from(EXIT_PASS)
}

fn run_interactive_riscv(
    cli: Cli,
    mut bus: labwired_core::bus::SystemBus,
    program: labwired_core::memory::ProgramImage,
    metrics: Arc<labwired_core::metrics::PerformanceMetrics>,
) -> ExitCode {
    let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
    let mut machine = labwired_core::Machine::new(cpu, bus);
    machine.observers.push(metrics.clone());

    if let Some(vcd_path) = &cli.vcd {
        let file = std::fs::File::create(vcd_path).expect("Failed to create VCD file");
        let observer = std::sync::Arc::new(vcd_trace::VcdObserver::new(file));
        machine.observers.push(observer);
    }

    if let Err(e) = machine.load_firmware(&program) {
        tracing::error!("Failed to load firmware into memory: {}", e);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    info!("Starting Simulation (RISC-V)...");
    info!(
        "Initial PC: {:#x}, SP: {:#x}",
        machine.cpu.pc,
        machine.cpu.x[2] // SP is x2 in RISC-V convention
    );

    // Check if GDB server is requested
    if let Some(port) = cli.gdb {
        let server = labwired_gdbstub::GdbServer::new(port);
        if let Err(e) = server.run(machine) {
            error!("GDB server failed: {}", e);
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }
        return ExitCode::from(EXIT_PASS);
    }

    let result = run_simulation_loop(&cli, &mut machine, &metrics);

    if let Some(path) = &cli.snapshot {
        let firmware_path = cli.firmware.as_ref().expect("Firmware path required");
        let system_path = cli.system.as_ref();

        write_interactive_snapshot(
            path,
            &metrics,
            &machine,
            InteractiveSnapshotInputs {
                firmware_path,
                system_path,
                max_steps: cli.max_steps,
                steps_executed: result.steps_executed,
                stop_reason: result.stop_reason,
                message: result.stop_message,
            },
        );
    }

    report_metrics(&cli, &machine.cpu, &metrics);
    ExitCode::from(EXIT_PASS)
}

fn run_interactive_xtensa(
    cli: Cli,
    mut bus: labwired_core::bus::SystemBus,
    program: labwired_core::memory::ProgramImage,
    metrics: Arc<labwired_core::metrics::PerformanceMetrics>,
) -> ExitCode {
    let cpu = labwired_core::system::xtensa::configure_xtensa(&mut bus);
    let mut machine = labwired_core::Machine::new(cpu, bus);
    machine.observers.push(metrics.clone());

    if let Some(vcd_path) = &cli.vcd {
        let file = std::fs::File::create(vcd_path).expect("Failed to create VCD file");
        let observer = std::sync::Arc::new(vcd_trace::VcdObserver::new(file));
        machine.observers.push(observer);
    }

    if let Err(e) = machine.load_firmware(&program) {
        tracing::error!("Failed to load firmware into memory: {}", e);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    info!("Starting Simulation (Xtensa LX7)...");
    info!(
        "Initial PC: {:#x}, SP: {:#x}",
        machine.cpu.pc,
        machine.cpu.regs.read_logical(1) // SP is a1 in Xtensa
    );

    if cli.gdb.is_some() {
        error!("GDB server is not yet supported for Xtensa architecture");
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let result = run_simulation_loop(&cli, &mut machine, &metrics);

    if let Some(path) = &cli.snapshot {
        let firmware_path = cli.firmware.as_ref().expect("Firmware path required");
        let system_path = cli.system.as_ref();

        write_interactive_snapshot(
            path,
            &metrics,
            &machine,
            InteractiveSnapshotInputs {
                firmware_path,
                system_path,
                max_steps: cli.max_steps,
                steps_executed: result.steps_executed,
                stop_reason: result.stop_reason,
                message: result.stop_message,
            },
        );
    }

    report_metrics(&cli, &machine.cpu, &metrics);
    ExitCode::from(EXIT_PASS)
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
fn build_stop_reason_details(
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
fn run_test(args: TestArgs) -> ExitCode {
    // ── API key validation (Pro tier gate) ──────────────────────────────
    // If LABWIRED_API_KEY is set and --no-key is not passed, validate before
    // starting the simulation so we fail fast with a clear message.
    let api_key_opt: Option<String> = if args.no_key {
        None
    } else {
        std::env::var("LABWIRED_API_KEY").ok()
    };

    let run_start = std::time::Instant::now();

    if let Some(ref key) = api_key_opt {
        match api_client::validate_key(key) {
            api_client::ValidateOutcome::Valid {
                workspace_id,
                plan,
                cycles_quota,
                cycles_used_mtd,
            } => {
                info!(
                    "LabWired Pro — workspace={} plan={} cycles_used={}/{} this month",
                    workspace_id, plan, cycles_used_mtd, cycles_quota
                );
            }
            api_client::ValidateOutcome::Invalid => {
                eprintln!(
                    "❌ LABWIRED_API_KEY is invalid. Check your dashboard or unset to use the free tier."
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
            api_client::ValidateOutcome::QuotaExceeded => {
                eprintln!(
                    "⚠️  Monthly cycle quota exceeded. Upgrade your plan or wait until next billing period."
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
            api_client::ValidateOutcome::NetworkError(e) => {
                // Network errors are non-fatal — fall through to run in free-tier mode
                // to avoid blocking CI when the API is temporarily unreachable.
                tracing::warn!(
                    "LabWired API unreachable ({}); continuing in free-tier mode",
                    e
                );
            }
        }
    }

    let loaded = match load_test_script(&args.script) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("{:#}", e);
            error!("{}", msg);
            write_config_error_outputs(&args, None, args.system.as_ref(), None, None, msg);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let (
        script_firmware,
        script_system,
        script_max_steps,
        script_max_cycles,
        script_max_uart_bytes,
        script_no_progress_steps,
        script_wall_time_ms,
        script_max_vcd_bytes,
        assertions,
    ) = match loaded {
        LoadedTestScript::V1_0(script) => (
            Some(script.inputs.firmware),
            script.inputs.system,
            script.limits.max_steps,
            script.limits.max_cycles,
            script.limits.max_uart_bytes,
            script.limits.no_progress_steps,
            script.limits.wall_time_ms,
            script.limits.max_vcd_bytes,
            script.assertions,
        ),
        LoadedTestScript::LegacyV1(script) => {
            tracing::warn!(
                "Deprecated test script format detected (schema_version: 1). Please migrate to schema_version: \"1.0\" with inputs/limits nesting."
            );
            (
                script.firmware,
                script.system,
                script.max_steps,
                None,
                None,
                None,
                script.wall_time_ms,
                None,
                script.assertions,
            )
        }
    };

    let max_steps = args.max_steps.unwrap_or(script_max_steps);
    let max_cycles = args.max_cycles.or(script_max_cycles);
    let max_uart_bytes = args.max_uart_bytes.or(script_max_uart_bytes);
    let max_vcd_bytes = args.max_vcd_bytes.or(script_max_vcd_bytes);
    let detect_stuck = args.detect_stuck.or(script_no_progress_steps);
    let resolved_limits = TestLimits {
        max_steps,
        max_cycles,
        max_uart_bytes,
        no_progress_steps: detect_stuck,
        wall_time_ms: script_wall_time_ms,
        max_vcd_bytes,
    };

    // Guard against accidentally huge runs from CI misconfiguration.
    const MAX_ALLOWED_STEPS: u64 = 50_000_000;
    if max_steps > MAX_ALLOWED_STEPS {
        let msg = format!(
            "max_steps {} exceeds MAX_ALLOWED_STEPS {}",
            max_steps, MAX_ALLOWED_STEPS
        );
        error!("{}", msg);
        write_config_error_outputs(
            &args,
            None,
            args.system.as_ref(),
            None,
            Some(&resolved_limits),
            msg,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let firmware_path = match args.firmware.clone() {
        Some(p) => p,
        None => match script_firmware
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| resolve_script_path(&args.script, s))
        {
            Some(p) => p,
            None => {
                let msg =
                    "Missing firmware path (provide --firmware or set inputs.firmware in script)"
                        .to_string();
                error!("{}", msg);
                write_config_error_outputs(
                    &args,
                    None,
                    args.system.as_ref(),
                    None,
                    Some(&resolved_limits),
                    msg,
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        },
    };

    let system_path = args.system.clone().or_else(|| {
        script_system
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| resolve_script_path(&args.script, s))
    });

    let firmware_bytes = match std::fs::read(&firmware_path) {
        Ok(b) => b,
        Err(e) => {
            let msg = format!("Failed to read firmware {:?}: {}", firmware_path, e);
            error!("{}", msg);
            write_config_error_outputs(
                &args,
                Some(&firmware_path),
                system_path.as_ref(),
                None,
                Some(&resolved_limits),
                msg,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // For Xtensa/ESP32 system manifests, `SystemBus::from_config` (called
    // inside `build_system_bus`) will fail: it tries to attach external devices
    // (e.g. the SSD1680 e-paper panel) to `spi3`, but `spi3` is not in the
    // chip YAML — it is installed in code by `configure_xtensa_esp32`. Detect
    // the Xtensa arch early by parsing the manifest once, and take the dedicated
    // `build_esp32_system_from_manifest` path that calls configure + attach
    // together, before falling through to `build_system_bus` for all other
    // architectures. The parsed manifest is reused so the file is read only once.
    let esp32_manifest: Option<labwired_config::SystemManifest> =
        if let Some(sys_path) = system_path.as_deref() {
            labwired_config::SystemManifest::from_file(sys_path)
                .ok()
                .filter(|m| {
                    let chip_path = sys_path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .join(&m.chip);
                    labwired_config::ChipDescriptor::from_file(&chip_path)
                        .map(|c| c.arch == labwired_config::Arch::Xtensa)
                        .unwrap_or(false)
                })
        } else {
            None
        };
    let is_xtensa = esp32_manifest.is_some();

    // For Xtensa, short-circuit: build bus + CPU together via build_esp32_system_from_manifest.
    if is_xtensa {
        if let (Some(sys_path), Some(manifest)) = (system_path.as_ref(), esp32_manifest.as_ref()) {
            let uart_tx = Arc::new(Mutex::new(Vec::new()));
            let (mut esp_bus, esp_cpu) =
                match labwired_core::system::builder::build_esp32_system_from_manifest(
                    manifest, sys_path,
                ) {
                    Ok(pair) => pair,
                    Err(e) => {
                        let msg = format!("{:#}", e);
                        error!("{}", msg);
                        write_config_error_outputs(
                            &args,
                            Some(&firmware_path),
                            system_path.as_ref(),
                            Some(&firmware_bytes),
                            Some(&resolved_limits),
                            msg,
                        );
                        return ExitCode::from(EXIT_CONFIG_ERROR);
                    }
                };
            esp_bus.attach_uart_tx_sink(uart_tx.clone(), !args.no_uart_stdout);

            let program = match labwired_loader::load_elf(&firmware_path) {
                Ok(program) => program,
                Err(e) => {
                    let msg = format!("{:#}", e);
                    error!("{}", msg);
                    write_config_error_outputs(
                        &args,
                        Some(&firmware_path),
                        system_path.as_ref(),
                        Some(&firmware_bytes),
                        Some(&resolved_limits),
                        msg,
                    );
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
            };

            let metrics = std::sync::Arc::new(labwired_core::metrics::PerformanceMetrics::new());
            let mut machine = labwired_core::Machine::new(esp_cpu, esp_bus);
            machine.observers.push(metrics.clone());
            if let Err(e) = machine.load_firmware(&program) {
                return handle_load_error(
                    &args,
                    &metrics,
                    &resolved_limits,
                    &firmware_bytes,
                    &uart_tx,
                    &machine.cpu,
                    &firmware_path,
                    system_path.as_ref(),
                    e,
                );
            }
            // ESP32 manifest path: skip BROM emulation and jump directly to
            // the ELF entry point — matches the wasm/playground path
            // (`new_from_config_xtensa_esp32`) and the e2e test
            // (`e2e_esp32_epaper.rs`). The BROM reset vector (0x4000_0400)
            // is fine for firmware compiled to boot from BROM, but playground
            // ELFs are pre-linked to start at the app entry.
            //
            // Seed SP to the top of DRAM (0x3FFE_0000): Arduino-ESP32 firmware
            // (call_start_cpu0) expects BROM to have placed SP here before
            // jumping to the app entry. We skip BROM, so do it ourselves —
            // matching `install_esp32_arduino_quirks` in the WASM path.
            // Native Xtensa firmware that sets its own SP will overwrite this
            // immediately.
            // CHEAT(SKIP): bypasses the boot ROM and hand-seeds PC/SP — real: the
            // CPU resets at 0x4000_0400 and the mapped BROM sets up SP and jumps
            // to the app. See FIDELITY.md §C.
            machine.cpu.set_pc(program.entry_point as u32);
            machine.cpu.set_sp(0x3FFE_0000);
            let exit_code = execute_test_loop(
                &args,
                &mut machine,
                &resolved_limits,
                &assertions,
                &firmware_bytes,
                &uart_tx,
                &metrics,
                &firmware_path,
                system_path.as_ref(),
            );
            // Device-block render readout. Surfaces the attached panel block's
            // REAL render state — refresh_gen AND black-plane ink — so a generic
            // verify (e.g. proto.cat's device loop) can judge whether the
            // device-block actually PAINTED, not merely refreshed. A refresh with
            // a blank plane is a false positive (the DC-latch class of bug; see
            // FIDELITY.md §E2). Emitted to stderr alongside the boot logs.
            {
                use labwired_core::peripherals::components::{
                    Ssd1680Tricolor290, Uc8151dTricolor290,
                };
                use labwired_core::peripherals::esp32::spi::Esp32Spi;
                if let Some(idx) = machine.bus.find_peripheral_index_by_name("spi3") {
                    if let Some(any) = machine.bus.peripherals[idx].dev.as_any() {
                        if let Some(spi3) = any.downcast_ref::<Esp32Spi>() {
                            for dev in &spi3.attached_devices {
                                let Some(a) = dev.as_any() else { continue };
                                if let Some(p) = a.downcast_ref::<Ssd1680Tricolor290>() {
                                    let ink =
                                        p.black_plane().iter().filter(|&&b| b != 0xFF).count();
                                    eprintln!(
                                        "[device-block] ssd1680_tricolor_290 refresh_gen={} black_ink={}",
                                        p.refresh_generation(),
                                        ink
                                    );
                                } else if let Some(p) = a.downcast_ref::<Uc8151dTricolor290>() {
                                    let ink =
                                        p.black_plane().iter().filter(|&&b| b != 0xFF).count();
                                    eprintln!(
                                        "[device-block] uc8151d_tricolor_290 refresh_gen={} black_ink={}",
                                        p.refresh_generation(),
                                        ink
                                    );
                                }
                            }
                        }
                    }
                }
            }
            if let Some(ref key) = api_key_opt {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(&firmware_bytes);
                let firmware_hash = format!("{:x}", hasher.finalize());
                let duration_ms = run_start.elapsed().as_millis() as u64;
                let cycles = metrics.get_cycles();
                let exit_val: i32 = if exit_code == ExitCode::from(EXIT_PASS) {
                    0
                } else if exit_code == ExitCode::from(EXIT_ASSERT_FAIL) {
                    1
                } else if exit_code == ExitCode::from(EXIT_RUNTIME_ERROR) {
                    3
                } else {
                    2
                };
                api_client::record_run(key, &firmware_hash, cycles, duration_ms, exit_val);
            }
            return exit_code;
        }
    }

    let mut bus = match labwired_core::system::builder::build_system_bus(system_path.as_deref()) {
        Ok(bus) => bus,
        Err(e) => {
            let msg = format!("{:#}", e);
            error!("{}", msg);
            write_config_error_outputs(
                &args,
                Some(&firmware_path),
                system_path.as_ref(),
                Some(&firmware_bytes),
                Some(&resolved_limits),
                msg,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let uart_tx = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_tx.clone(), !args.no_uart_stdout);

    let program = match labwired_loader::load_elf(&firmware_path) {
        Ok(program) => program,
        Err(e) => {
            let msg = format!("{:#}", e);
            error!("{}", msg);
            write_config_error_outputs(
                &args,
                Some(&firmware_path),
                system_path.as_ref(),
                Some(&firmware_bytes),
                Some(&resolved_limits),
                msg,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let metrics = std::sync::Arc::new(labwired_core::metrics::PerformanceMetrics::new());

    macro_rules! setup_and_run {
        ($cpu:expr) => {{
            let mut machine = labwired_core::Machine::new($cpu, bus);
            machine.observers.push(metrics.clone());
            if let Err(e) = machine.load_firmware(&program) {
                return handle_load_error(
                    &args,
                    &metrics,
                    &resolved_limits,
                    &firmware_bytes,
                    &uart_tx,
                    &machine.cpu,
                    &firmware_path,
                    system_path.as_ref(),
                    e,
                );
            }
            execute_test_loop(
                &args,
                &mut machine,
                &resolved_limits,
                &assertions,
                &firmware_bytes,
                &uart_tx,
                &metrics,
                &firmware_path,
                system_path.as_ref(),
            )
        }};
    }

    let exit_code = match program.arch {
        labwired_core::Arch::Arm => {
            let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
            setup_and_run!(cpu)
        }
        labwired_core::Arch::RiscV => {
            let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
            setup_and_run!(cpu)
        }
        labwired_core::Arch::XtensaLx7 => {
            // No system manifest present: plain configure path (no external devices).
            let cpu = labwired_core::system::xtensa::configure_xtensa(&mut bus);
            setup_and_run!(cpu)
        }
        _ => {
            let msg = format!("Unsupported architecture: {:?}", program.arch);
            error!("{}", msg);
            write_config_error_outputs(
                &args,
                Some(&firmware_path),
                system_path.as_ref(),
                Some(&firmware_bytes),
                Some(&resolved_limits),
                msg,
            );
            ExitCode::from(EXIT_CONFIG_ERROR)
        }
    };

    // ── Best-effort run metering (Pro tier) ──────────────────────────────
    if let Some(ref key) = api_key_opt {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&firmware_bytes);
        let firmware_hash = format!("{:x}", hasher.finalize());

        let duration_ms = run_start.elapsed().as_millis() as u64;
        let cycles = metrics.get_cycles();
        // Encode exit code as an integer for the API payload.
        // EXIT_PASS=0, EXIT_ASSERT_FAIL=1, EXIT_CONFIG_ERROR=2, EXIT_RUNTIME_ERROR=3
        let exit_val: i32 = if exit_code == ExitCode::from(EXIT_PASS) {
            0
        } else if exit_code == ExitCode::from(EXIT_ASSERT_FAIL) {
            1
        } else if exit_code == ExitCode::from(EXIT_RUNTIME_ERROR) {
            3
        } else {
            2
        };

        // best-effort — don't block on failure
        api_client::record_run(key, &firmware_hash, cycles, duration_ms, exit_val);
    }

    exit_code
}

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
    );

    if !all_passed || (stop_requires_assertion && !expected_stop_reason_matched) {
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

fn write_config_error_outputs(
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
fn write_junit_xml(
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
    };

    if s.len() <= MAX_LEN {
        return s;
    }

    let mut truncated = s.chars().take(MAX_LEN - 1).collect::<String>();
    truncated.push('…');
    truncated
}

// Minimal regex matcher supporting: '^' anchor, '$' anchor, '.' and '*' (Kleene star).
// This is intentionally small to avoid introducing new deps; it does not implement full PCRE/Rust regex.
fn simple_regex_is_match(pattern: &str, text: &str) -> bool {
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
