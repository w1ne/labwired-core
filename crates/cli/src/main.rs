// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

mod artifacts;
mod commands;
mod wifi_frames;
use clap::{Parser, Subcommand};
use serde::Serialize;
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

use artifacts::{AssertionResult, NamedU64, Snapshot, StopReasonDetails, TestConfig, TestResult};
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

/// Parse a `--watch-gpio` ref `peripheral:pin` into `(peripheral, pin)`. The pin
/// is a decimal `u8`; the peripheral is any non-empty name resolved against the
/// bus at run time (`gpio8`, `gpioa`, …). Returns `None` for a malformed ref
/// (missing colon, empty peripheral, or an out-of-range/non-numeric pin) — the
/// caller logs and skips it rather than aborting the whole run.
fn parse_watch_gpio_ref(spec: &str) -> Option<(String, u8)> {
    let (name, pin) = spec.trim().rsplit_once(':')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let pin: u8 = pin.trim().parse().ok()?;
    Some((name.to_string(), pin))
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

    /// Step a manifest-declared co-simulation model through the real
    /// runner/adapter chain and print the routed outputs.
    CosimStep(commands::cosim::CosimStepArgs),

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

    /// Firmware profile to use. Only `arduino-esp32` is supported — installs
    /// the Arduino-ESP32 / ESP32-classic bootstrap (heap-caps thunks, dual-core
    /// handshake, IPI bridge, image header) with thunk PCs resolved from the
    /// ELF symbol table (no hand-curated per-firmware address list). External
    /// peripherals come from the `--system` board manifest.
    #[arg(long, default_value = "arduino-esp32")]
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

    /// Optional path to export the universal I²C/SPI bus trace (logic
    /// analyzer) captured during the run. `.json` writes the raw event list;
    /// any other extension (e.g. `.vcd`) writes a Value Change Dump that
    /// opens directly in GTKWave / PulseView / Saleae / sigrok.
    #[arg(long)]
    pub bus_trace_out: Option<PathBuf>,

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
    #[arg(long)]
    trace_max: Option<usize>,

    /// Collect firmware statement coverage. Writes coverage.info (LCOV) and
    /// coverage.json into --output-dir. Distinct from `labwired coverage`,
    /// which measures chip-model register faithfulness.
    #[arg(long)]
    coverage: bool,

    /// Boot from the real ROM reset vector instead of fast-booting the ELF
    /// (ESP32-C3: mask ROM → 2nd-stage bootloader → app, exactly like
    /// silicon — required for Arduino/IDF images, which cannot fast-boot).
    /// Requires LABWIRED_ESP32C3_FLASH (the merged flash image:
    /// bootloader@0x0 + partition-table@0x8000 + app@0x10000). The boot ROM
    /// auto-provisions from the installed ESP toolchain or the vendored
    /// images; pin via LABWIRED_ESP32C3_ROM[_DATA].
    #[arg(long)]
    rom_boot: bool,

    /// Write a signable, reproducible run-manifest.json into --output-dir
    /// (input hashes, engine version, result subset, coverage summary, and a
    /// wall-clock-free SHA-256 digest).
    #[arg(long)]
    run_manifest: bool,

    /// Faithful rom-boot only: while running the REAL boot (mask ROM →
    /// 2nd-stage bootloader → app), snapshot the machine the instant control
    /// reaches the application and write a `.lwrs` resume snapshot here. The
    /// run then continues to --max-steps as usual, so one cold invocation
    /// yields BOTH the cached snapshot and the normal serial/cycle evidence.
    /// App-entry is `call_start_cpu0`/`app_main` (resolved from the ELF), else
    /// the first PC in the XIP app window [0x4200_0000, 0x4400_0000). The blob
    /// is self-keyed with the chip + firmware SHA-256 (see --resume-snapshot).
    #[arg(long)]
    capture_app_entry: Option<PathBuf>,

    /// Resume from a `.lwrs` snapshot instead of cold-booting: build a fresh
    /// machine for the same chip, load the SAME firmware/flash, validate the
    /// snapshot's self-key (chip + firmware SHA-256) against it, then apply it
    /// and run to --max-steps. Skips the ~150M-step mask-ROM replay entirely.
    /// On a self-key mismatch this errors out so the caller can fall back to a
    /// cold boot. Requires the same LABWIRED_ESP32C3_FLASH as the capture.
    #[arg(long)]
    resume_snapshot: Option<PathBuf>,

    /// Explicitly opt out of sending LABWIRED_API_KEY even if it is set in the environment.
    /// Useful for local development and testing.
    #[arg(long)]
    no_key: bool,

    /// Watch a GPIO pad's output for the deterministic logic-analyzer edge
    /// capture, as `peripheral:pin` (e.g. `gpio8:8`, `gpioa:5`). Repeatable —
    /// each ref is a channel (CH0, CH1, … in argument order). The captured
    /// per-channel edge series lands in `result.json`'s `logic_edges` block, so
    /// the oracle can prove a pad actually toggled / at a given period (the
    /// prove-blink evidence). Edges are drained from the same in-engine tap the
    /// browser logic analyzer uses. No watch → zero overhead, no block emitted.
    #[arg(long = "watch-gpio", value_name = "PERIPHERAL:PIN")]
    watch_gpio: Vec<String>,
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
        Some(Commands::CosimStep(args)) => commands::cosim::run_cosim_step(args),
        Some(Commands::Fuzz(args)) => commands::fuzz::run_fuzz(args),
        None => commands::run::run_interactive(cli),
    }
}

/// Resolve the rom-boot self-key — the chip name and the SHA-256 of the flash
/// image the faithful boot runs — from whichever `LABWIRED_ESP32*_FLASH` env
/// pin is set. This is the same firmware the resume snapshot must match; it is
/// stamped into a captured `.lwrs` and re-validated on resume so a snapshot
/// can never be applied on top of a different chip or firmware. Returns `None`
/// (so capture/resume are no-ops that fall back to a cold boot) when no flash
/// image is set — snapshot capture/resume only make sense on `--rom-boot`.
fn rom_boot_flash_self_key() -> Option<(&'static str, [u8; 32])> {
    use sha2::{Digest, Sha256};
    let (chip, path) = if let Ok(p) = std::env::var("LABWIRED_ESP32C3_FLASH") {
        ("esp32c3", p)
    } else if let Ok(p) = std::env::var("LABWIRED_ESP32S3_FLASH") {
        ("esp32s3", p)
    } else {
        return None;
    };
    let bytes = std::fs::read(&path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&hasher.finalize());
    Some((chip, out))
}

/// Build an ESP32-C3 ROM-boot machine; `efuse_mac` programs a distinct factory
/// MAC so multiple instances are distinguishable on the shared VirtualWifi air.
pub(crate) fn build_c3_rom_boot_machine(
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
    // ROM images: from_config already loaded them into the chip's rom regions
    // when the LABWIRED_ESP32C3_ROM[_DATA] env pins are set. Otherwise
    // auto-provision (toolchain ROM ELF, else the vendored images) and write
    // them into the still-zeroed regions, so --rom-boot works out of the box.
    if std::env::var("LABWIRED_ESP32C3_ROM").is_err() {
        use labwired_core::boot::esp32c3_rom as c3rom;
        let Some(images) = c3rom::provision_rom_images() else {
            eprintln!(
                "error: --rom-boot needs the real ESP32-C3 boot ROM, but none was found. \
                 Install an ESP toolchain (esp32c3_rev3_rom.elf) or set \
                 LABWIRED_ESP32C3_ROM / LABWIRED_ESP32C3_ROM_DATA."
            );
            return Err(ExitCode::from(EXIT_CONFIG_ERROR));
        };
        for mem in bus.extra_mem.iter_mut() {
            let (src, base) = if mem.base_addr == c3rom::IROM_BASE as u64 {
                (&images.irom, c3rom::IROM_BASE)
            } else if mem.base_addr == c3rom::DROM_BASE as u64 {
                (&images.drom, c3rom::DROM_BASE)
            } else {
                continue;
            };
            let n = src.len().min(mem.data.len());
            mem.data[..n].copy_from_slice(&src[..n]);
            tracing::info!("provisioned {n} bytes of C3 boot ROM @ {base:#010x}");
        }
    }
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
    // All the faithful peripheral wiring + reset-vector boot lives in the
    // shared core builder so the wasm browser path reuses it byte-for-byte.
    Ok(labwired_core::boot::esp32c3_rom::build_rom_boot_machine(
        bus,
        flash_bytes,
        labwired_core::boot::esp32c3_rom::RomBootOpts {
            efuse_mac,
            ..Default::default()
        },
        // Native keeps the concrete RiscV CPU (the wasm path boxes it).
        |c| c,
    ))
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
        // `attach_to_medium` flips the MAC's `needs_bus_tick()` on (medium
        // stations poll their inbox + beacon each tick) but is a non-MMIO
        // toggle, so rebuild the bus tick-index once to make the MAC resident.
        m.bus.refresh_peripheral_index();
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
        None,
        None,
    );
    ExitCode::from(EXIT_RUNTIME_ERROR)
}

fn assertion_currently_passes(
    assertion: &TestAssertion,
    uart_text: &str,
    machine: &labwired_core::Machine<impl labwired_core::Cpu>,
) -> bool {
    match assertion {
        TestAssertion::UartContains(a) => uart_text.contains(&a.uart_contains),
        TestAssertion::UartRegex(a) => simple_regex_is_match(&a.uart_regex, uart_text),
        TestAssertion::ExpectedStopReason(_) => true,
        TestAssertion::MemoryValue(a) => {
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
                _ => return false,
            };
            result.is_ok_and(|val| {
                let mask = a.memory_value.mask.unwrap_or(0xFFFFFFFF) as u32;
                let expected = a.memory_value.expected_value as u32;
                (val & mask) == (expected & mask)
            })
        }
        TestAssertion::UdsTester(a) => {
            evaluate_uds_tester(&machine.bus.can_uds_testers, &a.uds_tester).is_ok()
        }
    }
}

/// Does this `labwired test` run qualify for the RV32IMC wasm-JIT fast path?
///
/// True ⇔ the target is RISC-V (ESP32-C3), batch mode is on, and NONE of the
/// per-instruction-visibility features that force the JIT's correctness gate
/// shut is active. This is the SAME set of conditions that would otherwise pin
/// the CLI batch to one instruction (`batch_size` in `execute_test_loop`) or
/// make `RiscV::jit_gate_allows` refuse to run — folded into one predicate the
/// caller evaluates BEFORE installing observers, so the eligible path can skip
/// the metrics step observer entirely (its presence gates the JIT off) and
/// source cycles/instructions from the machine's own counters instead.
///
/// Deliberately conservative: any `--trace`/`--coverage`/`--vcd`/`--breakpoint`/
/// `--detect-stuck`/`--watch-gpio`, a `stop_when_assertions_pass` early-stop, or
/// a cycle-accurate/poll-mode peripheral drops the run onto the exact current
/// observer-based path (`jit_eligible == false`).
fn riscv_jit_test_eligible<C: labwired_core::Cpu>(
    args: &TestArgs,
    limits: &TestLimits,
    machine: &labwired_core::Machine<C>,
    arch: labwired_core::Arch,
) -> bool {
    // NOTE: `batch_mode_enabled` is deliberately NOT required. The eligible path
    // drives `Machine::advance`, which batches to the peripheral-tick cadence
    // regardless of that flag — indeed the C3 rom-boot machine turns it OFF (its
    // fixed-width step_batch loop freezes FreeRTOS), which is exactly the case we
    // want to accelerate.
    matches!(arch, labwired_core::Arch::RiscV)
        && !args.trace
        && !args.coverage
        && args.vcd.is_none()
        && args.breakpoint.is_empty()
        && args.watch_gpio.is_empty()
        && args.capture_app_entry.is_none()
        && limits.no_progress_steps.is_none()
        && !limits.stop_when_assertions_pass
        && !machine.bus.requires_cycle_accurate()
        && !machine.logic_poll_active()
}

/// Map a core `SimulationError` to the CLI `StopReason` so a halt or fault from
/// `Machine::advance` ends the run with the CLI's established reason.
fn map_sim_error_to_stop_reason(e: &labwired_core::SimulationError) -> StopReason {
    use labwired_core::SimulationError as E;
    match e {
        E::MemoryViolation(_) => StopReason::MemoryViolation,
        E::DecodeError(_) => StopReason::DecodeError,
        E::Halt => StopReason::Halt,
        E::SnapshotSchemaMismatch { .. } => StopReason::Exception,
        E::Other(_) => StopReason::Exception,
        E::NotImplemented(_) => StopReason::Exception,
        E::BreakpointHit(_) => StopReason::Halt,
        E::ExceptionRaised { .. } => StopReason::Exception,
    }
}

/// Instruction budget per `Machine::advance` call on the JIT-eligible C3 path. The
/// stimulus/limit checks at the top of `execute_test_loop`'s run loop run once
/// per chunk, so this bounds their granularity; the chunk is further clamped so
/// a run never steps PAST the nearest pending cycle threshold (time-triggered
/// stimulus or `max_cycles`), keeping those firing points cycle-tight and
/// identical between the JIT-on and JIT-off arms.
const JIT_RUN_CHUNK: u32 = 1_000_000;

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
    stimuli: &[labwired_config::StimulusSpec],
    // True when this run qualifies for the RV32IMC wasm-JIT fast path (decided
    // by `riscv_jit_test_eligible` in the caller): RiscV arch, batch mode, and
    // NONE of the per-instruction-visibility features that gate the JIT off.
    // In this mode `metrics` was NOT installed as a step observer (its presence
    // forces the JIT's correctness gate shut), so the loop mirrors the machine's
    // own counters into `metrics` before each cycle-sensitive check.
    jit_eligible: bool,
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
        let obs = Arc::new(labwired_core::trace::TraceObserver::new(
            args.trace_max.unwrap_or(100_000),
        ));
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

    // ── --watch-gpio: arm the deterministic logic-analyzer edge capture ──────
    // Resolve each `peripheral:pin` ref ONCE (to a peripheral index + pin),
    // exactly as the wasm `watch_logic_signals` accessor does, arm the in-engine
    // tap, and keep the per-channel identity so the drained edges can be shaped
    // into `result.json`'s `logic_edges` block after the run. An empty watch set
    // is a no-op (no channels installed → zero-overhead capture path).
    let logic_watch_meta: Vec<labwired_core::logic_capture::LogicChannelMeta> = {
        let refs: Vec<(String, u8)> = args
            .watch_gpio
            .iter()
            .filter_map(|spec| parse_watch_gpio_ref(spec))
            .collect();
        if refs.len() != args.watch_gpio.len() {
            for spec in &args.watch_gpio {
                if parse_watch_gpio_ref(spec).is_none() {
                    error!("--watch-gpio: ignoring malformed ref {spec:?} (want `peripheral:pin`)");
                }
            }
        }
        if refs.is_empty() {
            Vec::new()
        } else {
            let resolved: Vec<Option<(usize, u8)>> = refs
                .iter()
                .map(|(name, pin)| {
                    machine
                        .bus
                        .find_peripheral_index_by_name(name)
                        .map(|idx| (idx, *pin))
                })
                .collect();
            for ((name, _), r) in refs.iter().zip(resolved.iter()) {
                if r.is_none() {
                    error!("--watch-gpio: peripheral {name:?} not found on the bus; channel will stay flat");
                }
            }
            let initial = machine.logic_watch(&resolved);
            refs.iter()
                .zip(initial)
                .enumerate()
                .map(
                    |(ch, ((name, pin), value))| labwired_core::logic_capture::LogicChannelMeta {
                        ch: ch as u32,
                        peripheral: name.clone(),
                        pin: *pin,
                        initial: value,
                    },
                )
                .collect()
        }
    };
    let logic_capture_armed = !logic_watch_meta.is_empty();

    // ── JIT-eligible cycle/instruction sourcing (RISC-V / ESP32-C3) ──────────
    // When eligible, engage the RV32IMC wasm-JIT for this run and source the
    // metrics counters from the machine's own state (no step observer). Sourcing
    // cycles from `machine.total_cycles` (not the observer's per-step
    // `on_step_end` tap) is what makes JIT-on and JIT-off byte-identical:
    // compiled blocks retire WITHOUT firing `on_step_end`, so an observer would
    // undercount them. Both JIT arms (`LABWIRED_RISCV_JIT=1` on, default off)
    // STAY in this same machine-sourced regime, so they are byte-identical
    // (proven by tests/riscv_jit_c3_oled_test_differential); the metrics numbers
    // never depend on whether a batch was interpreted or compiled.
    if jit_eligible {
        // JIT is OPT-IN (LABWIRED_RISCV_JIT=1), NOT default-on. Measured on the
        // esp32c3-oled-demo oracle lab, the wasmtime RV32IMC JIT is ~18× SLOWER
        // than the interpreter here: the hot path is tight FreeRTOS/idle loops
        // (~1.9 guest instr per compiled-block run), so the per-block-dispatch
        // FFI overhead dwarfs the interpreted cost and ~⅔ of instructions still
        // fall back to the interpreter. The genuine speedup on this path is the
        // tick-interval widening below (`Machine::advance` at the bus max-safe
        // interval: ~2.6× faster than the pre-change single-step tick-1 oracle),
        // which is applied UNCONDITIONALLY when eligible. The JIT stays wired,
        // proven byte-identical, and one env var away for compute-heavy firmware
        // where straight-line blocks amortize the dispatch cost. See the report.
        let jit_on = std::env::var("LABWIRED_RISCV_JIT").as_deref() == Ok("1");
        machine.config.riscv_jit_enabled = jit_on;
        machine.bus.config.riscv_jit_enabled = jit_on;
        // Widen the peripheral-tick interval to RECOMMENDED_TICK_INTERVAL so
        // `Machine::advance`'s per-tick batch is wide enough
        // for compiled blocks to retire, and the peripheral tick count drops
        // ~64×. The C3 rom-boot peripherals are walk-deletable, so this is
        // observably identical to interval-1 (esp32c3_walk_differential); the
        // eligibility gate already excludes any `requires_cycle_accurate` bus.
        // `max_safe_tick_interval` is NOT used here because it only returns the
        // wide interval under the `event-scheduler` feature, which the CLI does
        // not enable (see crates/cli/Cargo.toml). Crucially this is applied to
        // BOTH JIT arms, so it never perturbs the JIT-on vs JIT-off differential.
        // TEST-ONLY escape hatch (regression gate riscv_jit_c3_oled_test_differential):
        // override the widened interval with LABWIRED_TICK_INTERVAL so the
        // interval-64 (widened) vs interval-1 (baseline) fidelity gate can be
        // proven empirically with EVERYTHING else identical (same machine-sourced
        // cycle counting, same eligible code path) — the tick interval is the ONLY
        // variable. Unset = default (RECOMMENDED_TICK_INTERVAL).
        let interval = std::env::var("LABWIRED_TICK_INTERVAL")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(labwired_core::bus::RECOMMENDED_TICK_INTERVAL);
        machine.config.peripheral_tick_interval = interval;
        machine.bus.config.peripheral_tick_interval = interval;
    }

    let batch_size = if machine.config.batch_mode_enabled
        && args.breakpoint.is_empty()
        && detect_stuck.is_none()
        && !resolved_limits.stop_when_assertions_pass
        // Cycle-tight GPIO-timing devices (e.g. HC-SR04 ECHO pulse) only behave
        // correctly when peripherals tick between every instruction; instruction
        // batching freezes them across the batch and the firmware measures 0.
        && !machine.bus.requires_cycle_accurate()
        // A logic-analyzer POLL-mode channel must be sampled at EVERY cycle
        // boundary, so clamp the batch to one instruction while one is armed
        // (mirrors `Machine::advance`). Push-mode channels report their own edges
        // from the write sites and keep the full batch width.
        && !machine.logic_poll_active()
    {
        10000.min(max_steps)
    } else {
        1
    };

    // Declarative input stimuli (schema_version 1.2). Applied via the generic
    // `Machine::set_input` path (see `labwired_core::sim_input`), so no per-type
    // wiring. `at_start` fires now; `after_cycles` fires the first loop
    // iteration at or past its cycle threshold. The closure takes `machine` as
    // an argument (captures nothing) so it can be called both here and mid-loop.
    let apply_stimulus = |machine: &mut labwired_core::Machine<C>,
                          s: &labwired_config::StimulusSpec| {
        let result = match s.target.component.as_deref() {
            Some(component) => machine.set_input_on(component, &s.target.channel, s.value),
            None => machine.set_input(&s.target.channel, s.value),
        };
        match result {
            Ok(()) => info!("stimulus: {} = {} applied", s.target.channel, s.value),
            Err(e) => error!(
                "stimulus '{}' = {} could not be applied: {:?}",
                s.target.channel, s.value, e
            ),
        }
    };
    for s in stimuli {
        if matches!(s.trigger, labwired_config::FaultTrigger::AtStart) {
            apply_stimulus(machine, s);
        }
    }
    // Time-triggered stimuli, each tagged with whether it has fired yet.
    let mut pending_stimuli: Vec<(&labwired_config::StimulusSpec, bool)> = stimuli
        .iter()
        .filter(|s| matches!(s.trigger, labwired_config::FaultTrigger::AfterCycles { .. }))
        .map(|s| (s, false))
        .collect();

    // Tracks the step at which all runtime assertions first passed. The
    // `stop_when_assertions_pass` early-stop is only accepted after the machine
    // keeps executing for a settling window past this point WITHOUT faulting —
    // print-then-crash firmware breaks with its fault reason during the window
    // instead of certifying as passed. A regression (assertions stop passing)
    // resets it, so the pass must be durable.
    let mut assertions_first_passed_at: Option<u64> = None;

    // ── --capture-app-entry: cache a genuine faithful-boot state ─────────
    // While the REAL rom-boot runs, snapshot the machine the instant control
    // first reaches the application, write the `.lwrs`, then keep running so
    // this same cold invocation still emits the normal evidence. The capture
    // point is a real mid-flight boot state — NOT a hand-modeled handoff.
    struct AppEntryCapture {
        path: PathBuf,
        chip: &'static str,
        fw_sha: [u8; 32],
        // App-entry PC resolved from the ELF (`call_start_cpu0`, else
        // `app_main`); `None` falls back to the XIP app-window detector.
        target_pc: Option<u32>,
    }
    let mut app_entry_capture: Option<AppEntryCapture> =
        args.capture_app_entry.as_ref().and_then(|path| {
            let Some((chip, fw_sha)) = rom_boot_flash_self_key() else {
                error!(
                    "--capture-app-entry needs a faithful rom-boot (set LABWIRED_ESP32C3_FLASH \
                     or LABWIRED_ESP32S3_FLASH); skipping capture"
                );
                return None;
            };
            let target_pc =
                labwired_loader::resolve_symbol_in_elf(firmware_bytes, "call_start_cpu0")
                    .or_else(|| labwired_loader::resolve_symbol_in_elf(firmware_bytes, "app_main"));
            match target_pc {
                Some(pc) => {
                    info!("capture-app-entry: chip={chip} app-entry PC 0x{pc:08x} (ELF symbol)")
                }
                None => info!(
                    "capture-app-entry: chip={chip} no call_start_cpu0/app_main symbol; \
                     using first PC in XIP app window [0x42000000,0x44000000)"
                ),
            }
            Some(AppEntryCapture {
                path: path.clone(),
                chip,
                fw_sha,
                target_pc,
            })
        });

    let mut step = 0;
    while step < max_steps {
        // JIT-eligible path: mirror the machine's authoritative counters into
        // `metrics` BEFORE the cycle-sensitive checks below (stimulus
        // `after_cycles`, `max_cycles`), so they fire at exactly the same batch
        // boundary the observer path would. `step` is the retired-instruction
        // count (accumulated from `step_batch` return values); `total_cycles`
        // is the machine's canonical cycle counter. No-op for the non-eligible
        // path, where `metrics` IS the live step observer.
        if jit_eligible {
            metrics.set_cycles(machine.total_cycles);
            metrics.set_instructions(step);
        }
        // --capture-app-entry: detect the first instant execution reaches the
        // application, snapshot the live machine, and write the resume blob.
        if let Some(cap) = &app_entry_capture {
            let pc = machine.cpu.get_pc();
            let reached = cap.target_pc == Some(pc) || (0x4200_0000..0x4400_0000).contains(&pc);
            if reached {
                let mut snap = machine.take_runtime_snapshot();
                snap.set_self_key(cap.chip, cap.fw_sha);
                if let Some(parent) = cap.path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&cap.path, snap.to_bytes()) {
                    Ok(()) => info!(
                        "capture-app-entry: snapshot written to {:?} at app-entry pc=0x{pc:08x} \
                         (cold-boot step {step})",
                        cap.path
                    ),
                    Err(e) => error!("capture-app-entry: failed to write {:?}: {e}", cap.path),
                }
                // Capture once; keep running so the cold invocation still
                // produces the normal serial/cycle evidence.
                app_entry_capture = None;
            }
        }
        // Fire any `after_cycles` stimulus whose threshold the run has reached.
        if !pending_stimuli.is_empty() {
            let cycles = metrics.get_cycles();
            for (s, fired) in pending_stimuli.iter_mut() {
                if *fired {
                    continue;
                }
                if let labwired_config::FaultTrigger::AfterCycles { cycles: threshold } = s.trigger
                {
                    if cycles >= threshold {
                        apply_stimulus(machine, s);
                        *fired = true;
                    }
                }
            }
        }
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

        let (mut limit, batch_cap) = if jit_eligible {
            let chunk = remaining.min(JIT_RUN_CHUNK);
            (u64::from(chunk), chunk)
        } else {
            (u64::from(to_execute), current_batch)
        };
        let current_cycle = machine.total_cycles;
        for (stimulus, fired) in &pending_stimuli {
            if !*fired {
                if let labwired_config::FaultTrigger::AfterCycles { cycles } = stimulus.trigger {
                    if cycles > current_cycle {
                        limit = limit.min(cycles - current_cycle);
                    }
                }
            }
        }
        if let Some(cycle_limit) = max_cycles {
            if cycle_limit > current_cycle {
                limit = limit.min(cycle_limit - current_cycle);
            }
        }
        let request = labwired_core::AdvanceRequest::run(Some(limit.max(1)))
            .with_batch_cap(
                std::num::NonZeroU32::new(batch_cap.max(1)).expect("advance batch cap is non-zero"),
            )
            .with_breakpoints(labwired_core::BreakpointPolicy::Ignore);
        match machine.advance(request) {
            Ok(report) => {
                step += report.primary_steps;
                steps_executed = step;
                if report.primary_steps == 0 && report.idle_cycles == 0 {
                    stop_reason = StopReason::Halt;
                    break;
                }
            }
            Err(error) => {
                sim_error_happened = true;
                stop_reason = map_sim_error_to_stop_reason(&error);
                if stop_reason != StopReason::Halt {
                    error!("Simulation error at step {}: {}", step, error);
                }
                break;
            }
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

        if resolved_limits.stop_when_assertions_pass {
            let has_runtime_assertions = assertions
                .iter()
                .any(|a| !matches!(a, TestAssertion::ExpectedStopReason(_)));
            if has_runtime_assertions {
                let uart_text = {
                    let bytes = uart_tx.lock().map(|g| g.clone()).unwrap_or_default();
                    String::from_utf8_lossy(&bytes).to_string()
                };
                let all_pass = assertions
                    .iter()
                    .filter(|a| !matches!(a, TestAssertion::ExpectedStopReason(_)))
                    .all(|a| assertion_currently_passes(a, &uart_text, machine));
                if all_pass {
                    // Latch the first all-pass step, but not before the absolute
                    // minimum-steps floor: assertions that satisfy trivially early
                    // (e.g. a token already present at reset) don't short-circuit
                    // the run before real execution has happened.
                    if assertions_first_passed_at.is_none()
                        && step >= resolved_limits.stop_when_assertions_pass_min_steps
                    {
                        assertions_first_passed_at = Some(step);
                    }
                } else {
                    // A regression means the pass was not durable — restart the
                    // settling window from scratch.
                    assertions_first_passed_at = None;
                }
                if let Some(first) = assertions_first_passed_at {
                    if step.saturating_sub(first)
                        >= resolved_limits.stop_when_assertions_pass_settle_steps
                    {
                        stop_reason = StopReason::AssertionsPassed;
                        break;
                    }
                }
            }
        }
    }

    // Final counter mirror for the JIT-eligible path: the loop-top sync runs
    // before the LAST batch, so capture that batch's retired cycles/instructions
    // here — `result.json` (`cycles`/`instructions`) and `stop_reason_details`
    // read `metrics` below and must report the true totals.
    if jit_eligible {
        metrics.set_cycles(machine.total_cycles);
        metrics.set_instructions(steps_executed);
    }

    // Opt-in JIT non-vacuity / diagnostic: prove hot blocks actually compiled
    // and ran on this oracle run (LABWIRED_JIT_STATS=1). `jit_engine_stats` is a
    // feature-agnostic Cpu-trait accessor: `Some(..)` only in a `jit-core` build
    // whose JIT engine was created, `None` otherwise (interpreter-only).
    if jit_eligible && std::env::var("LABWIRED_JIT_STATS").is_ok() {
        match machine.cpu.jit_engine_stats() {
            Some(s) => eprintln!(
                "[jit-stats] compiled={} block_runs={} block_instrs={} interpreted={}",
                s.compiled, s.block_runs, s.block_instrs, s.interpreted
            ),
            None => eprintln!("[jit-stats] JIT engine never created (interpreter-only run)"),
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

    // Final-state universal inspect block (summary mode: decoded registers +
    // artifact metadata, framebuffer bytes omitted/hashed). This is the
    // agent-facing oracle payload — after a run the caller sees the decoded
    // final register state and which artifacts exist.
    let inspect_block = machine.inspect(
        None,
        &labwired_core::inspect::InspectOpts {
            include_bytes: false,
            peripheral: None,
        },
    );

    // Drain the deterministic logic-analyzer edge capture for THIS run and shape
    // it into the shared per-channel series form. Reading from cursor 0 returns
    // every retained edge; `dropped` (surfaced in the block) is non-zero only if
    // the 64k ring overflowed, which the oracle treats as fail-loud. This is the
    // SAME `logic_read_edges` drain the wasm `read_logic_edges` accessor uses, so
    // the CLI `result.json` edges and the browser edges are edge-for-edge equal.
    let logic_edges = if logic_capture_armed {
        let now_cycle = machine.logic_now_cycle();
        let batch = machine.logic_read_edges(0);
        Some(labwired_core::logic_capture::build_logic_edges_result(
            &logic_watch_meta,
            &batch,
            now_cycle,
        ))
    } else {
        None
    };

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
        Some(inspect_block),
        logic_edges,
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
    inspect: Option<labwired_core::inspect::MachineInspect>,
    logic_edges: Option<labwired_core::logic_capture::LogicEdgesResult>,
) {
    let mut hasher = Sha256::new();
    hasher.update(firmware_bytes);
    let firmware_hash = format!("{:x}", hasher.finalize());

    // Drain the coverage-gap log for THIS run. `write_outputs` is called
    // synchronously at the tail of `execute_test_loop`, on the very thread that
    // ran the sim loop, so this reads the same thread-local the `record_*` calls
    // populated. `take()` resets it, so it must run exactly once per run — this
    // is the sole call site on the run path.
    let fidelity = labwired_core::fidelity::take().to_gaps();

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
        inspect,
        fidelity,
        logic_edges,
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
        stop_when_assertions_pass_settle_steps: 0,
        stop_when_assertions_pass_min_steps: 0,
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
        inspect: None,
        // Config error: the sim never ran, so there are no coverage gaps to report.
        fidelity: Vec::new(),
        // Nor any logic-analyzer edges — capture never armed.
        logic_edges: None,
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

    #[test]
    fn config_error_snapshot_keeps_serde_tag() {
        let snapshot = crate::artifacts::Snapshot::ConfigError {
            message: "invalid test config".to_string(),
            stop_reason_details: crate::artifacts::StopReasonDetails {
                triggered_stop_condition: StopReason::ConfigError,
                triggered_limit: None,
                observed: None,
            },
            limits: TestLimits {
                max_steps: 1,
                max_cycles: None,
                max_uart_bytes: None,
                no_progress_steps: None,
                wall_time_ms: None,
                max_vcd_bytes: None,
                stop_when_assertions_pass: false,
                stop_when_assertions_pass_settle_steps: 0,
                stop_when_assertions_pass_min_steps: 0,
            },
            config: crate::artifacts::TestConfig {
                firmware: std::path::PathBuf::from("firmware.elf"),
                system: None,
                script: std::path::PathBuf::from("test.yaml"),
            },
        };

        let json = serde_json::to_value(snapshot).expect("snapshot should serialize");
        assert_eq!(json["type"], "config_error");
    }
}
