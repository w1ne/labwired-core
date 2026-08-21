// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

// The library is also the `labwired` binary's home: `src/main.rs` is a thin
// shim over [`run_with_plugins`]. `extern crate self` keeps the
// `labwired_cli::...` paths the binary used valid inside the library.
extern crate self as labwired_cli;

pub mod baseline;
pub mod bus_vcd;
pub mod coverage;
pub mod crash_report;
pub mod faults;
pub mod manifest;
pub mod pc_coverage_report;
pub mod regex;
/// What a finished run reports (row 6.11: verdict / report / drive).
mod report;
pub mod test_support;
pub mod tier1;
pub mod verdict;

mod api_client;
mod artifacts;
mod asset_validation;
mod commands;
mod component_validation;
mod gpio_observer;
mod resource_report;
mod size_limited_writer;
mod vcd_trace;
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

use artifacts::{
    AssertionEvidence, AssertionResult, Snapshot, StimulusOutcome, StopReasonDetails, TestConfig,
    TestResult,
};
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

    /// List the chips bundled with this CLI, usable as `inputs.chip` in a test
    /// script or as a manifest's `chip:` field without copying any YAML.
    Chips,

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

    /// One-shot agent debug probe (JSON on stdout or --output-dir/result.json).
    ///
    /// Loads machine + firmware, resolves optional breakpoints (address /
    /// symbol / line), runs until stop or max-steps, and returns stop reason,
    /// PC, location, registers, and serial. Observational only: never claims
    /// oracle proof (`proven` is always false).
    DebugProbe(commands::debug_probe::DebugProbeArgs),
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

    /// Write a JSON instruction trace here. Records the LAST `--trace-last`
    /// retired instructions, which is the window that matters when a run
    /// faults. Attaching a trace forces the interpreter (compiled blocks can't
    /// emit per-step events), so the capture runs slower.
    #[arg(long = "trace-out", value_name = "PATH")]
    pub trace_out: Option<PathBuf>,

    /// How many retired instructions to keep in the trace ring.
    #[arg(long = "trace-last", value_name = "N", default_value = "4096")]
    pub trace_last: usize,
}

#[derive(Parser, Debug)]
pub struct RunArgs {
    /// Path to the chip descriptor YAML.
    #[arg(long)]
    pub chip: PathBuf,

    /// Path to the firmware ELF.
    #[arg(long)]
    pub firmware: PathBuf,

    /// Optional system manifest (YAML) whose `external_devices:` are attached
    /// to the chip before the run — a display, a sensor, anything the board
    /// carries. Without it the chip runs bare and firmware talking to a panel
    /// has nothing on the far side of the bus.
    ///
    /// ESP32-S3 only for now (the path `--rom-boot` uses); other families
    /// build their bus through `SystemBus::from_config` and already take a
    /// manifest via the top-level `--system`.
    #[arg(long)]
    pub system: Option<PathBuf>,

    /// Optional path for an end-of-run dump of every attached parallel panel:
    /// a binary PPM at this path plus a luma ASCII map on stderr. Proves what
    /// the display actually painted, not just that a transaction completed.
    #[arg(long)]
    pub display_out: Option<PathBuf>,

    /// End the run as soon as this text appears on the firmware's console.
    /// Makes end-of-run artifacts frame-exact: "stop right after the firmware
    /// printed X" is reproducible where a hand-tuned `--max-steps` is not.
    /// ESP32-S3 only (needs the USB-Serial-JTAG console).
    #[arg(long)]
    pub stop_on: Option<String>,

    /// Maximum number of simulator steps before exit (default: unlimited).
    #[arg(long)]
    pub max_steps: Option<u64>,

    /// Exit 0 even when the run ends on a simulation fault.
    ///
    /// A fault normally exits 3, the same as every other runtime error. Use
    /// this when the caller owns the verdict and reads it from the output —
    /// the TIER1 matrix, for instance, treats the protocol lines on stdout as
    /// the result and a late fault as noise.
    #[arg(long)]
    pub allow_sim_error: bool,

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

    /// ARM-only: an additional flash piece placed at an explicit absolute
    /// address, `<path>@<hex-offset>`. Repeatable — compose e.g. a Nordic
    /// SoftDevice at `0x0` with an application ELF (`--firmware`) linked to
    /// run above it. Each piece may be an ELF, an Intel HEX (`.hex`), or a
    /// raw binary blob; overlapping pieces (with each other or with
    /// `--firmware`) are a hard error. `--firmware` keeps working exactly as
    /// before when no `--flash-image` is given.
    #[arg(long = "flash-image", value_name = "PATH@HEX")]
    pub flash_image: Vec<String>,

    /// Drive the run through the batched orchestration
    /// (`Machine::advance(AdvanceRequest::run(..))`) that the browser front end
    /// uses, instead of the one-instruction-per-call `Machine::step()` loop.
    ///
    /// This is an assertion, not a hint: the flag fails the run rather than
    /// falling back, on any chip family or option combination that cannot take
    /// the batched path. On ARM it selects the batched loop; on RISC-V the
    /// batched loop is already the default and the flag only refuses the
    /// per-instruction instrumentation (`--break-at`, the WiFi bridge, the DHCP
    /// trace) that would silently turn it back into single-stepping; on Xtensa
    /// there is no `Machine`-driven path at all, so it is rejected outright.
    ///
    /// Exists because the batched path is what users actually run in the
    /// browser while the CLI default is not, which left engine changes to ARM
    /// batch orchestration invisible to `scripts/perf/board_perf.py`. It also
    /// prints a `[batched] ...` summary line to stderr on exit, so a caller can
    /// prove which path executed rather than assume it.
    #[arg(long = "batched")]
    pub batched: bool,
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

/// The chip-YAML lookup [`labwired_config::ChipDescriptor::resolve_with`]
/// expects, across all linked plugins. Built-ins always win inside
/// `resolve_with`; this is only consulted for names the open catalog does
/// not know.
pub(crate) fn plugin_chip_yaml<'a>(
    plugins: &'a [&'a dyn labwired_core::plugin::ChipPlugin],
) -> impl Fn(&str) -> Option<&'static str> + 'a {
    move |name| plugins.iter().find_map(|p| p.chip_yaml(name))
}

/// Refuse plugins whose [`labwired_core::plugin::ChipPlugin::api_version`]
/// does not match the core this CLI was built against.
///
/// Extracted so tests can exercise the gate without running the full CLI.
pub fn check_plugin_versions(
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> Result<(), String> {
    for p in plugins {
        if p.api_version() != labwired_core::plugin::PLUGIN_API_VERSION {
            return Err(format!(
                "plugin API mismatch: plugin built against v{}, CLI core is v{}",
                p.api_version(),
                labwired_core::plugin::PLUGIN_API_VERSION
            ));
        }
    }
    Ok(())
}

/// The `labwired` binary with extra chip plugins linked in.
/// Pass `&[]` for the stock open-catalog CLI.
pub fn run_with_plugins(plugins: &[&dyn labwired_core::plugin::ChipPlugin]) -> ExitCode {
    // A panic used to print a backtrace on the user's terminal and reach
    // nobody else. Chains to the default hook, so what they see is unchanged.
    crash_report::install();

    if let Err(msg) = check_plugin_versions(plugins) {
        eprintln!("{msg}");
        return ExitCode::FAILURE;
    }

    let cli = Cli::parse();

    // RUST_LOG used to be silently ignored. `with_max_level` is a hard ceiling
    // compiled into the binary — no environment variable can raise or lower it —
    // so `RUST_LOG=error labwired test ...` still printed every INFO line and
    // there was no way to quiet the runner at all.
    //
    // `EnvFilter` honours RUST_LOG (including per-module directives such as
    // `RUST_LOG=warn,labwired_core=debug`) and falls back to the previous
    // default when it is unset or unparseable, so behaviour with no RUST_LOG in
    // the environment is unchanged: DEBUG under `--trace`, INFO otherwise.
    let default_level = if cli.trace { "debug" } else { "info" };
    let log_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level));
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(log_filter)
        .init();

    match cli.command {
        Some(Commands::Chips) => {
            for name in labwired_config::BUILTIN_CHIP_NAMES {
                println!("{name}");
            }
            for p in plugins {
                for name in p.chip_names() {
                    println!("{name}");
                }
            }
            ExitCode::SUCCESS
        }
        Some(Commands::Test(args)) => commands::test::run_test(args, plugins),
        Some(Commands::Machine(args)) => run_machine(args, plugins),
        Some(Commands::Asset(args)) => run_asset(args, plugins),
        Some(Commands::Run(args)) => commands::run::run_firmware(args, plugins),
        Some(Commands::Snapshot(args)) => commands::snapshot::run_snapshot(args, plugins),
        Some(Commands::Coverage(args)) => commands::coverage::run_coverage(args),
        Some(Commands::Tier1Matrix(args)) => commands::tier1::run_tier1_matrix(args),
        Some(Commands::CosimStep(args)) => commands::cosim::run_cosim_step(args),
        Some(Commands::Fuzz(args)) => commands::fuzz::run_fuzz(args),
        Some(Commands::DebugProbe(args)) => commands::debug_probe::run(args, plugins),
        None => commands::run::run_interactive(cli, plugins),
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

/// The factory MAC a built C3 die actually carries, read back from its eFuse
/// MAC words (`EFUSE_RD_MAC_SPI_SYS_0/1`). Reported rather than assumed, so a
/// dual-node banner cannot claim an address the die does not have.
fn format_efuse_mac(m: &labwired_core::Machine<labwired_core::cpu::RiscV>) -> String {
    use labwired_core::Bus;
    let lo = m.bus.read_u32(0x6000_8844).unwrap_or(0);
    let hi = m.bus.read_u32(0x6000_8848).unwrap_or(0);
    let mac = [
        (hi >> 8) as u8,
        hi as u8,
        (lo >> 24) as u8,
        (lo >> 16) as u8,
        (lo >> 8) as u8,
        lo as u8,
    ];
    mac.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Build an ESP32-C3 ROM-boot machine. `pinned_efuse_mac` fixes this die's
/// factory MAC; `None` mints a new one, so multiple instances are
/// distinguishable on the shared VirtualWifi/BLE air without the caller
/// arranging it.
pub(crate) fn build_c3_rom_boot_machine(
    bus: labwired_core::bus::SystemBus,
    pinned_efuse_mac: Option<[u8; 6]>,
) -> Result<labwired_core::Machine<labwired_core::cpu::RiscV>, ExitCode> {
    build_c3_rom_boot_machine_from(bus, pinned_efuse_mac, "LABWIRED_ESP32C3_FLASH")
}

/// As [`build_c3_rom_boot_machine`], but the flash image comes from the named
/// environment variable. Multi-node runs boot two different firmwares in one
/// process (e.g. a BLE advertiser and a BLE scanner), which one fixed variable
/// cannot express.
pub(crate) fn build_c3_rom_boot_machine_from(
    mut bus: labwired_core::bus::SystemBus,
    pinned_efuse_mac: Option<[u8; 6]>,
    flash_env: &str,
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
    let flash_path = match std::env::var(flash_env) {
        Ok(p) => p,
        Err(_) => {
            eprintln!(
                "error: --rom-boot needs {flash_env} set (the flash image: \
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
            pinned_efuse_mac,
            ..Default::default()
        },
        // Native keeps the concrete RiscV CPU (the wasm path boxes it).
        |c| c,
    ))
}

/// Two-node BLE run: boot two ESP32-C3 instances with distinct factory MACs and
/// **different firmware** onto the shared BLE air, so one can advertise while
/// the other scans. `LABWIRED_ESP32C3_FLASH` is node A, `LABWIRED_ESP32C3_FLASH_B`
/// is node B; both models take the process-global
/// [`ble_air`](labwired_core::peripherals::ble_air) bus, so the medium between
/// them is the same one the single-node run already transmits into.
fn run_two_c3_ble(
    args: &RunArgs,
    chip: &labwired_config::ChipDescriptor,
    manifest: &labwired_config::SystemManifest,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> ExitCode {
    use labwired_core::bus::SystemBus;

    // Two nodes = two dies. Nothing is arranged here: leaving the factory MAC
    // unpinned makes the builder mint one identity per node, which is the same
    // thing that separates two MCUs on a browser canvas.
    let build = |env: &str| -> Result<labwired_core::Machine<labwired_core::cpu::RiscV>, ExitCode> {
        let bus = match SystemBus::from_config_with_plugins(chip, manifest, plugins) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("error: failed to build system bus: {e:#}");
                return Err(ExitCode::from(EXIT_CONFIG_ERROR));
            }
        };
        build_c3_rom_boot_machine_from(bus, None, env)
    };
    let mut a = match build("LABWIRED_ESP32C3_FLASH") {
        Ok(m) => m,
        Err(c) => return c,
    };
    let mut b = match build("LABWIRED_ESP32C3_FLASH_B") {
        Ok(m) => m,
        Err(c) => return c,
    };
    eprintln!(
        "[ble] two-C3 BLE over the shared air: A={} (LABWIRED_ESP32C3_FLASH), \
         B={} (LABWIRED_ESP32C3_FLASH_B)",
        format_efuse_mac(&a),
        format_efuse_mac(&b)
    );
    // Give each node its own serial capture and silence the shared console
    // echo: two machines writing stdout byte-by-byte interleave into an
    // unreadable mess, and the whole point of this run is reading both.
    let sinks: Vec<std::sync::Arc<std::sync::Mutex<Vec<u8>>>> = (0..2)
        .map(|_| std::sync::Arc::new(std::sync::Mutex::new(Vec::new())))
        .collect();
    for (m, sink) in [(&mut a, &sinks[0]), (&mut b, &sinks[1])] {
        for p in m.bus.peripherals.iter_mut() {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            if let Some(uart) = any.downcast_mut::<labwired_core::peripherals::esp_uart::EspUart>()
            {
                uart.set_sink(Some(sink.clone()));
                uart.silence_stdout_echo_if(false);
            }
        }
    }

    // Acceptance stop, the dual-run twin of a test script's
    // `stop_when_assertions_pass`: `LABWIRED_BLE_DUAL_STOP_ON=<substring>` ends
    // the run as soon as BOTH nodes' serial contains that substring. Without it
    // a two-node gate has to burn its whole step ceiling every time, because
    // there is nothing else that can know the run has proved its point. The
    // budget stays a CEILING — it only bounds how long a broken model flails.
    // Polled every 1M steps: the check copies both sinks, so doing it per step
    // would dominate the run.
    let stop_on = std::env::var("LABWIRED_BLE_DUAL_STOP_ON").ok();
    const STOP_POLL_STEPS: u64 = 1_000_000;
    let seen = |sink: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>, needle: &str| -> bool {
        let bytes = sink.lock().map(|g| g.clone()).unwrap_or_default();
        String::from_utf8_lossy(&bytes).contains(needle)
    };

    let limit = args.max_steps.unwrap_or(u64::MAX);
    for i in 0..limit {
        if let Err(e) = a.step() {
            eprintln!("[ble] node A halted at step {i}: {e}");
            break;
        }
        if let Err(e) = b.step() {
            eprintln!("[ble] node B halted at step {i}: {e}");
            break;
        }
        if let Some(needle) = &stop_on {
            if i % STOP_POLL_STEPS == 0
                && i > 0
                && seen(&sinks[0], needle)
                && seen(&sinks[1], needle)
            {
                eprintln!("[ble] both nodes printed {needle:?} by step {i} — stopping");
                break;
            }
        }
    }
    for (label, sink) in [("[A]", &sinks[0]), ("[B]", &sinks[1])] {
        let bytes = sink.lock().map(|g| g.clone()).unwrap_or_default();
        for line in String::from_utf8_lossy(&bytes).lines() {
            println!("{label} {line}");
        }
    }
    eprintln!("[ble] run complete");
    ExitCode::SUCCESS
}

/// Two-station WiFi run: boot two ESP32-C3 instances with distinct factory MACs
/// onto the shared [`virtual_wifi`] medium. Each is a full real firmware over its
/// own real MAC; the medium is the AP + the air between them. They associate, get
/// distinct DHCP leases (192.168.4.2 / .3), and exchange routed IP traffic.
fn run_two_c3_wifi(
    args: &RunArgs,
    chip: &labwired_config::ChipDescriptor,
    manifest: &labwired_config::SystemManifest,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> ExitCode {
    use labwired_core::bus::SystemBus;
    use labwired_core::peripherals::esp32c3::{virtual_wifi, wifi_mac::Esp32c3WifiMac};

    virtual_wifi::reset();

    // Two stations = two dies; the builder mints an identity for each.
    let build = || -> Result<labwired_core::Machine<labwired_core::cpu::RiscV>, ExitCode> {
        let bus = match SystemBus::from_config_with_plugins(chip, manifest, plugins) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("error: failed to build system bus: {e:#}");
                return Err(ExitCode::from(EXIT_CONFIG_ERROR));
            }
        };
        build_c3_rom_boot_machine(bus, None)
    };
    let mut a = match build() {
        Ok(m) => m,
        Err(c) => return c,
    };
    let mut b = match build() {
        Ok(m) => m,
        Err(c) => return c,
    };
    eprintln!(
        "[dual] two-C3 WiFi over shared VirtualWifi: A={}, B={}",
        format_efuse_mac(&a),
        format_efuse_mac(&b)
    );
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

/// Single-station WiFi run: one ESP32-C3 on the shared [`virtual_wifi`] medium.
/// It associates with the virtual AP, gets a DHCP lease, and reaches the AP's
/// DHCP + HTTP servers — the LBC3.1 stats-device demo path. Mirrors the dual
/// harness (own minimal step loop, non-zero factory MAC, UART echo) rather than
/// bolting medium mode onto the standard run loop, which does not keep the MAC
/// resident (auth never completes).
pub(crate) fn run_one_c3_wifi(
    args: &RunArgs,
    chip: &labwired_config::ChipDescriptor,
    manifest: &labwired_config::SystemManifest,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> ExitCode {
    use labwired_core::bus::SystemBus;
    use labwired_core::peripherals::esp32c3::virtual_wifi::{ApConfig, VirtualWifiBus};
    use labwired_core::peripherals::esp32c3::{virtual_wifi, wifi_mac::Esp32c3WifiMac};

    virtual_wifi::reset();

    // If the manifest declares a `wifi_ap`, host a medium with that config;
    // otherwise the MACs bind the process-global default AP (byte-identical to
    // the former hardcoded behaviour).
    let configured_bus = manifest.wifi_ap.as_ref().map(|ap| {
        let ip = {
            let octets: Vec<u8> = ap
                .ip
                .split('.')
                .filter_map(|o| o.parse::<u8>().ok())
                .collect();
            (octets.len() == 4).then(|| [octets[0], octets[1], octets[2], octets[3]])
        };
        VirtualWifiBus::with_config(ApConfig::from_parts(
            Some(ap.ssid.clone()),
            ip,
            Some(&ap.serves),
        ))
    });

    let bus = match SystemBus::from_config_with_plugins(chip, manifest, plugins) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: failed to build system bus: {e:#}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    let mut m = match build_c3_rom_boot_machine(bus, None) {
        Ok(m) => m,
        Err(c) => return c,
    };
    eprintln!(
        "[solo] one C3 on VirtualWifi: STA={} (AP hosts DHCP + HTTP)",
        format_efuse_mac(&m)
    );
    for p in m.bus.peripherals.iter_mut() {
        let Some(any) = p.dev.as_any_mut() else {
            continue;
        };
        if let Some(mac) = any.downcast_mut::<Esp32c3WifiMac>() {
            if let Some(bus) = configured_bus.as_ref() {
                mac.set_wifi_bus(bus.clone());
            }
            mac.attach_to_medium();
        } else if let Some(uart) = any.downcast_mut::<labwired_core::peripherals::uart::Uart>() {
            uart.set_stdout_prefix("");
        }
    }
    m.bus.refresh_peripheral_index();

    let limit = args.max_steps.unwrap_or(u64::MAX);
    // Measurement/parity path (env LABWIRED_WIFI_FF=1): drive the station via
    // the authoritative `advance(run)` loop with scheduler-safe idle
    // fast-forward enabled — exactly what the browser bridge does for a heavy
    // C3 chip (`isHeavyBrowserChip` → `set_idle_fast_forward_enabled(true)`).
    // The default `step()` loop stays the faithful per-instruction reference so
    // the two can be diffed for byte-identical WiFi output.
    if std::env::var("LABWIRED_WIFI_FF").is_ok() {
        use labwired_core::AdvanceRequest;
        m.config.idle_fast_forward_enabled = true;
        eprintln!("[solo] idle fast-forward ENABLED (advance/run path)");
        let mut done: u64 = 0;
        while done < limit {
            let chunk = (limit - done).min(2_000_000);
            match m.advance(AdvanceRequest::run(Some(chunk))) {
                Ok(_report) => {}
                Err(e) => {
                    eprintln!("[solo] station halted after {done} cycles: {e}");
                    break;
                }
            }
            done = done.saturating_add(chunk);
            if m.total_cycles == 0 {
                break;
            }
        }
        eprintln!(
            "[solo] run complete — total_cycles={} idle_ff_cycles_skipped={}",
            m.total_cycles, m.idle_fast_forward_cycles_skipped
        );
        return ExitCode::SUCCESS;
    }
    for i in 0..limit {
        if let Err(e) = m.step() {
            eprintln!("[solo] station halted at step {i}: {e}");
            break;
        }
    }
    eprintln!(
        "[solo] run complete — total_cycles={} (no idle-ff)",
        m.total_cycles
    );
    ExitCode::SUCCESS
}

fn run_asset(args: AssetArgs, plugins: &[&dyn labwired_core::plugin::ChipPlugin]) -> ExitCode {
    match args.command {
        AssetCommands::ImportSvd(a) => commands::svd::run_import_svd(a),
        AssetCommands::Codegen(a) => commands::codegen::run_codegen(a),
        AssetCommands::Init(a) => commands::asset::run_asset_init(a),
        AssetCommands::AddPeripheral(a) => commands::asset::run_asset_add_peripheral(a),
        AssetCommands::Validate(a) => asset_validation::run_validate(a, plugins),
        AssetCommands::ListChips(a) => asset_validation::run_list_chips(a),
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

fn run_machine(args: MachineArgs, plugins: &[&dyn labwired_core::plugin::ChipPlugin]) -> ExitCode {
    match args.command {
        MachineCommands::Load(load_args) => commands::machine::run_machine_load(load_args, plugins),
    }
}

/// The ONE message a firmware exit produces, so `simctl` cannot read one way on
/// `labwired run` and another under a test script.
pub(crate) fn firmware_exit_message(code: u32) -> String {
    format!("Firmware ended the run with exit code {code}")
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
        // `advance` rather than `step`: `step` discards the AdvanceReport, and
        // the report is the only place a firmware-authored verdict appears.
        // `AdvanceRequest::single()` is exactly what `step` issues, so the
        // stepping behaviour is unchanged — we simply stop throwing the result
        // away.
        match machine.advance(labwired_core::AdvanceRequest::single()) {
            Ok(report) => {
                steps_executed = (step + 1) as u64;
                if let labwired_core::AdvanceStop::FirmwareExit { code } = report.stop {
                    // The message names the exit code, which is all three
                    // consumers of this LoopResult forward. The structured
                    // `firmware_exit_code` lives on TestResult, the run-result
                    // contract, and is set by the test loop below.
                    let message = firmware_exit_message(code);
                    info!("{} (step={})", message, step);
                    stop_reason = StopReason::FirmwareExit;
                    stop_message = Some(message);
                    break;
                }
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
    let stop_reason_details = crate::report::build_stop_reason_details(
        &StopReason::Halt,
        resolved_limits,
        0,
        metrics.get_cycles(),
        0,
        0,
        std::time::Duration::from_secs(0),
        0, // vcd_bytes
    );
    // The pre-run bail-out. Even here the two views come from one verdict, so
    // this path cannot drift the way the run path did.
    let verdict = crate::verdict::Verdict::RuntimeError;
    write_outputs(
        args,
        verdict,
        0,
        metrics,
        StopReason::Halt,
        stop_reason_details,
        // This is the pre-run bail-out path: no firmware verdict exists.
        None,
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
        // Load/reset failed before the run loop, so no stimulus was attempted.
        Vec::new(),
        // Load failed: no successful machine run for footprint/paint/metrics.
        None,
        None,
        None,
    );
    verdict.exit_code()
}

/// The assertions decided by captured UART text alone, and nothing else.
///
/// Returns `None` for any assertion that needs the machine — that is the
/// caller's signal to keep matching, not a failure.
///
/// Shared deliberately: the single-machine runner and the multi-MCU world
/// runner must agree on what `uart_contains` means, and the world runner has
/// no `Machine<impl Cpu>` to hand to [`assertion_currently_passes`]. Two
/// copies of `uart_text.contains(..)` is exactly how one of them drifts.
pub(crate) fn uart_assertion_passes(assertion: &TestAssertion, uart_text: &str) -> Option<bool> {
    Some(match assertion {
        TestAssertion::UartContains(a) => uart_text.contains(&a.uart_contains),
        TestAssertion::UartRegex(a) => simple_regex_is_match(&a.uart_regex, uart_text),
        TestAssertion::UartOrdered(a) => {
            let mut offset = 0;
            a.uart_ordered.iter().all(|token| {
                let Some(found) = uart_text[offset..].find(token) else {
                    return false;
                };
                offset += found + token.len();
                true
            })
        }
        _ => return None,
    })
}

fn assertion_currently_passes(
    assertion: &TestAssertion,
    uart_text: &str,
    machine: &labwired_core::Machine<impl labwired_core::Cpu>,
) -> bool {
    if let Some(passed) = uart_assertion_passes(assertion, uart_text) {
        return passed;
    }
    match assertion {
        // Handled above by `uart_assertion_passes`.
        TestAssertion::UartContains(_)
        | TestAssertion::UartRegex(_)
        | TestAssertion::UartOrdered(_) => unreachable!("decided by uart_assertion_passes"),
        TestAssertion::MotorSpeedReached(a) => machine.bus.motor_snapshots().iter().any(|motor| {
            let speed = motor.speed_rpm.abs();
            motor.id == a.motor_speed_reached.id
                && speed >= a.motor_speed_reached.min_abs_rpm
                && speed <= a.motor_speed_reached.max_abs_rpm
        }),
        TestAssertion::MotorState(a) => machine.bus.motor_snapshots().iter().any(|motor| {
            motor.id == a.motor_state.id
                && motor.control_state == a.motor_state.control_state
                && a.motor_state
                    .fault_contains
                    .as_ref()
                    .is_none_or(|fault| motor.faults.contains(fault))
        }),
        TestAssertion::MqttFabric(a) => machine.bus.mqtt_fabric_matches(
            &a.mqtt_fabric.topic,
            a.mqtt_fabric.payload_contains.as_deref(),
        ),
        // This assertion requires immutable event-cycle evidence collected by
        // the runner; accumulated text alone is deliberately insufficient.
        TestAssertion::ShutdownLatency(_) => false,
        TestAssertion::ExpectedStopReason(_) => true,
        // Terminal, like ExpectedStopReason: decided by how the run ENDED, so
        // it is not a runtime condition the early-stop logic can wait on.
        TestAssertion::FirmwareExit(_) => true,
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
        TestAssertion::DisplayRegion(a) => {
            evaluate_display_region(&machine.bus, &a.display_region).is_ok()
        }
        // Post-run only (footprint / stack paint). Terminal like FirmwareExit:
        // does not block `stop_when_assertions_pass` early-stop of live checks.
        TestAssertion::ResourceBudget(_) => true,
    }
}

/// Evaluate a `resource_budget` assertion against post-run footprint / memory.
///
/// Exactly one of the three limits is set (validated at script load).
/// Evidence is attached **only on failure**.
fn evaluate_resource_budget(
    details: &labwired_config::ResourceBudgetDetails,
    footprint: Option<&artifacts::FootprintReport>,
    memory: Option<&labwired_core::stack_paint::MainStackReport>,
) -> (bool, Option<AssertionEvidence>) {
    use labwired_core::stack_paint::MainStackMethod;

    let (name, measured, limit, method) = if let Some(limit) = details.max_flash_bytes {
        let (measured, method) = match footprint {
            Some(f) => (Some(f.flash_used_bytes), f.method.clone()),
            None => (None, "footprint_unavailable".to_string()),
        };
        ("max_flash_bytes", measured, limit, method)
    } else if let Some(limit) = details.max_ram_static_bytes {
        let (measured, method) = match footprint {
            Some(f) => (Some(f.ram_static_bytes), f.method.clone()),
            None => (None, "footprint_unavailable".to_string()),
        };
        ("max_ram_static_bytes", measured, limit, method)
    } else if let Some(limit) = details.max_main_stack_bytes {
        let (measured, method) = match memory {
            Some(m) => {
                let method = match m.main_stack_method {
                    MainStackMethod::Paint => "paint",
                    MainStackMethod::Disabled => "disabled",
                    MainStackMethod::Unsupported => "unsupported",
                };
                (m.main_stack_high_water_bytes, method.to_string())
            }
            None => (None, "unsupported".to_string()),
        };
        ("max_main_stack_bytes", measured, limit, method)
    } else {
        // validate() should reject this; fail closed if it ever reaches here.
        return (
            false,
            Some(AssertionEvidence::ResourceBudget {
                name: "resource_budget".to_string(),
                measured: None,
                limit: 0,
                method: "invalid".to_string(),
            }),
        );
    };

    let passed = measured.is_some_and(|m| m <= limit);
    let evidence = if !passed {
        Some(AssertionEvidence::ResourceBudget {
            name: name.to_string(),
            measured,
            limit,
            method,
        })
    } else {
        None
    };
    (passed, evidence)
}

/// Measure one `display_region` assertion against the live panel.
///
/// Reads the display through `SystemBus::display_artifact` — the same single
/// door the browser renderer and `inspect` use — so the assertion sees exactly
/// the pixels the product shows, for any panel on any transport, keyed only by
/// its `external_devices:` id.
///
/// Every way of not-measuring is an `Err`, never a pass: no such device, a
/// device with no display artifact, an artifact whose bytes were withheld, a
/// format with no decoder. A panel that genuinely painted nothing is a
/// measurement, and fails on `min_ink` like anything else.
pub(crate) fn evaluate_display_region(
    bus: &labwired_core::bus::SystemBus,
    d: &labwired_config::DisplayRegionDetails,
) -> Result<(), String> {
    use labwired_core::inspect::{artifact_region_ink, InspectOpts, PixelRegion};

    let artifact = bus
        .display_artifact(
            &d.id,
            &InspectOpts {
                include_bytes: true,
                peripheral: None,
            },
        )
        .ok_or_else(|| {
            format!(
                "display_region '{}': no display device with that id is attached to this machine \
                 (check the `external_devices:` id in the system manifest)",
                d.id
            )
        })?;

    let bytes = artifact.bytes.as_deref().ok_or_else(|| {
        format!(
            "display_region '{}': the device published a '{}' artifact with no byte payload",
            d.id, artifact.kind
        )
    })?;
    let format = artifact
        .meta
        .get("format")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("display_region '{}': artifact has no `meta.format`", d.id))?;
    let panel_w = artifact.meta.get("w").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let panel_h = artifact.meta.get("h").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    let region = PixelRegion {
        x: d.x,
        y: d.y,
        w: d.w.unwrap_or_else(|| panel_w.saturating_sub(d.x)),
        h: d.h.unwrap_or_else(|| panel_h.saturating_sub(d.y)),
    };
    let (ink, total) = artifact_region_ink(format, &artifact.meta, bytes, region)
        .map_err(|e| format!("display_region '{}': {e}", d.id))?;

    let fraction = ink as f64 / total as f64;
    let max = d.max_ink.unwrap_or(1.0);
    if fraction < d.min_ink || fraction > max {
        return Err(format!(
            "display_region '{}': region ({},{}) {}x{} is {:.1}% inked ({ink}/{total} pixels), \
             outside the required {:.1}%..={:.1}%",
            d.id,
            region.x,
            region.y,
            region.w,
            region.h,
            fraction * 100.0,
            d.min_ink * 100.0,
            max * 100.0,
        ));
    }
    Ok(())
}

fn requires_fine_grained_observation(assertions: &[TestAssertion]) -> bool {
    assertions
        .iter()
        .any(|assertion| matches!(assertion, TestAssertion::ShutdownLatency(_)))
}

/// How often `stop_when_assertions_pass` may re-measure a display.
///
/// Every other assertion reads something already sitting in memory. A
/// `display_region` unpacks the panel's whole framebuffer — 153,600 bytes for an
/// ILI9341 — and counts pixels. At the step-granular poll rate that runs once
/// per instruction, which turns a sub-second run into an unfinishable one. A
/// screen does not need instruction-exact stop timing, so it is polled on the
/// batch grid instead; the only cost is stopping up to this many steps after the
/// paint, which `stop_when_assertions_pass_settle_steps` already tolerates.
const DISPLAY_POLL_BATCH: u64 = 10_000;

fn assertion_observation_batch_size(
    otherwise_batch_eligible: bool,
    stop_when_assertions_pass: bool,
    assertions: &[TestAssertion],
    max_steps: u64,
) -> u64 {
    if !otherwise_batch_eligible || requires_fine_grained_observation(assertions) {
        return 1;
    }
    if !stop_when_assertions_pass {
        return 10_000.min(max_steps);
    }
    if assertions
        .iter()
        .any(|a| matches!(a, TestAssertion::DisplayRegion(_)))
    {
        return DISPLAY_POLL_BATCH.min(max_steps);
    }
    1
}

fn assertion_compatible_jit_eligibility(
    otherwise_jit_eligible: bool,
    assertions: &[TestAssertion],
) -> bool {
    otherwise_jit_eligible && !requires_fine_grained_observation(assertions)
}

#[derive(Debug)]
struct UartMilestoneCycles {
    occurrences: std::collections::HashMap<String, Vec<(usize, u64)>>,
}

impl UartMilestoneCycles {
    fn new(tokens: impl IntoIterator<Item = String>) -> Self {
        Self {
            occurrences: tokens
                .into_iter()
                .map(|token| (token, Vec::new()))
                .collect(),
        }
    }

    fn observe(&mut self, accumulated_uart: &[u8], cycle: u64) {
        for (token, occurrences) in &mut self.occurrences {
            let bytes = token.as_bytes();
            if bytes.is_empty() || accumulated_uart.len() < bytes.len() {
                continue;
            }
            for (start, window) in accumulated_uart.windows(bytes.len()).enumerate() {
                if window == bytes && !occurrences.iter().any(|(seen, _)| *seen == start) {
                    occurrences.push((start, cycle));
                }
            }
        }
    }

    fn cycles(&self, token: &str) -> impl Iterator<Item = u64> + '_ {
        self.occurrences
            .get(token)
            .into_iter()
            .flatten()
            .map(|(_, cycle)| *cycle)
    }
}

#[derive(Debug, Clone, Copy)]
struct StimulusApplication {
    cycle: u64,
    value: f64,
    sequence: u64,
}

type StimulusCycles = std::collections::HashMap<(Option<String>, String), Vec<StimulusApplication>>;

fn stimulus_key(target: &labwired_config::StimulusTarget) -> (Option<String>, String) {
    (target.component.clone(), target.channel.clone())
}

fn shutdown_latency_passes(
    details: &labwired_config::ShutdownLatencyDetails,
    stimulus_cycles: &StimulusCycles,
    uart_cycles: &UartMilestoneCycles,
) -> bool {
    shutdown_latency_cycles(details, stimulus_cycles, uart_cycles)
        .is_some_and(|(_, _, latency)| latency <= details.max_cycles)
}

fn shutdown_latency_cycles(
    details: &labwired_config::ShutdownLatencyDetails,
    stimulus_cycles: &StimulusCycles,
    uart_cycles: &UartMilestoneCycles,
) -> Option<(u64, u64, u64)> {
    let stimulus_index = usize::try_from(details.stimulus_occurrence.checked_sub(1)?).ok()?;
    let uart_index = usize::try_from(details.uart_occurrence.checked_sub(1)?).ok()?;
    let stimulus = stimulus_cycles
        .get(&stimulus_key(&details.from_stimulus))?
        .get(stimulus_index)?;
    // Preserve value and global application sequence in the retained event
    // record even though latency pairing is selected by target occurrence.
    let _application_identity = (stimulus.value, stimulus.sequence);
    let token_cycle = uart_cycles
        .cycles(&details.to_uart)
        .filter(|cycle| *cycle >= stimulus.cycle)
        .nth(uart_index)?;
    Some((stimulus.cycle, token_cycle, token_cycle - stimulus.cycle))
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
    assertions: &[TestAssertion],
    machine: &labwired_core::Machine<C>,
    arch: labwired_core::Arch,
) -> bool {
    // NOTE: `batch_mode_enabled` is deliberately NOT required. The eligible path
    // drives `Machine::advance`, which batches to the peripheral-tick cadence
    // regardless of that flag — indeed the C3 rom-boot machine turns it OFF (its
    // fixed-width step_batch loop freezes FreeRTOS), which is exactly the case we
    // want to accelerate.
    let otherwise_jit_eligible = matches!(arch, labwired_core::Arch::RiscV)
        && !args.trace
        && !args.coverage
        && args.vcd.is_none()
        && args.breakpoint.is_empty()
        && args.watch_gpio.is_empty()
        && args.capture_app_entry.is_none()
        && limits.no_progress_steps.is_none()
        && !limits.stop_when_assertions_pass
        && !machine.bus.requires_cycle_accurate()
        && !machine.logic_poll_active();
    assertion_compatible_jit_eligibility(otherwise_jit_eligible, assertions)
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

/// Fuel budget per `Machine::advance` call on an idle-fast-forward run whose
/// stop conditions can all be checked at that granularity
/// (`idle_ff_wide_observation` in `execute_test_loop`).
///
/// This is a FUEL budget, not a CPU batch width — the batch cap is passed
/// separately and is unchanged. It bounds how far one idle skip may reach, so
/// it has to comfortably exceed a FreeRTOS tick window: an ESP32-C3 at 160 MHz
/// idles ~160k cycles per millisecond, so `vTaskDelay(200)` is ~32M cycles.
/// Same value as [`JIT_RUN_CHUNK`], which already bounds this loop's
/// observation granularity on the JIT-eligible path.
const IDLE_FF_RUN_CHUNK: u32 = 1_000_000;

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
    uart_injections: &[labwired_config::UartInjectionSpec],
    // True when this run qualifies for the RV32IMC wasm-JIT fast path (decided
    // by `riscv_jit_test_eligible` in the caller): RiscV arch, batch mode, and
    // NONE of the per-instruction-visibility features that gate the JIT off.
    // In this mode `metrics` was NOT installed as a step observer (its presence
    // forces the JIT's correctness gate shut), so the loop mirrors the machine's
    // own counters into `metrics` before each cycle-sensitive check.
    jit_eligible: bool,
    // Architecture of the loaded image (paint is ARM-only in P0).
    arch: labwired_core::Arch,
    // Script + env kill switch for main-stack paint.
    stack_paint: bool,
    // Chip flash/RAM map for footprint totals and paint RAM bounds.
    chip_mem: Option<resource_report::ChipMemoryMap>,
) -> ExitCode {
    // ── Resource metrics: footprint + main-stack paint (load/reset-time) ────
    // Paint is not a SimulationObserver: fill unused stack RAM now, scan after
    // the run. Footprint is pure ELF section math and does not touch the bus.
    let footprint = resource_report::compute_footprint(firmware_bytes, chip_mem.as_ref());
    let sp_top = resource_report::arm_sp(&machine.cpu);
    let (memory_pre, paint_session) = resource_report::apply_stack_paint(
        &mut machine.bus,
        sp_top,
        arch,
        stack_paint,
        firmware_bytes,
        chip_mem.as_ref(),
    );
    // Drop load/paint bus traffic so `metrics.memory_*` reflect the run only.
    let _ = machine.bus.take_access_counts();

    // Cheap statistical PC histogram (no SimulationObserver — JIT-safe).
    let mut pc_hist: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
    let mut pc_sample_budget: u64 = 0;
    // Best-effort exception count: SimulationError::ExceptionRaised only in P1.
    let mut exception_count: u64 = 0;

    let max_steps = resolved_limits.max_steps;
    let max_cycles = resolved_limits.max_cycles;
    let max_uart_bytes = resolved_limits.max_uart_bytes;
    let detect_stuck = resolved_limits.no_progress_steps;
    let script_wall_time_ms = resolved_limits.wall_time_ms;

    let start = std::time::Instant::now();
    let mut stop_reason = StopReason::MaxSteps;
    let mut steps_executed: u64 = 0;
    // Set only when the firmware ends its own run via `simctl`; read by the
    // `firmware_exit` assertion below.
    let mut firmware_exit_code: Option<u32> = None;

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
            let resolved: Vec<Option<labwired_core::logic_capture::LogicSource>> = refs
                .iter()
                .map(|(name, pin)| {
                    machine
                        .bus
                        .find_peripheral_index_by_name(name)
                        .map(|idx| labwired_core::logic_capture::LogicSource::pad(idx, *pin))
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

    let otherwise_batch_eligible = machine.config.batch_mode_enabled
        && args.breakpoint.is_empty()
        && detect_stuck.is_none()
        // Cycle-tight GPIO-timing devices (e.g. HC-SR04 ECHO pulse) only behave
        // correctly when peripherals tick between every instruction; instruction
        // batching freezes them across the batch and the firmware measures 0.
        // Push-mode channels report their own edges from the write sites and keep
        // the full batch width.
        && !machine.logic_poll_active();
    let batch_size = assertion_observation_batch_size(
        otherwise_batch_eligible,
        resolved_limits.stop_when_assertions_pass,
        assertions,
        max_steps,
    );

    // ── Fuel budget vs CPU batch width ──────────────────────────────────────
    // These are two different knobs that this loop used to collapse into one
    // number, and the collapse silently disarmed idle fast-forward.
    //
    // `batch_cap` is how many instructions the CPU may retire per window. It
    // stays `batch_size`, which is 1 whenever batch mode is off (the ESP32-C3
    // rom-boot path turns batching off because a fixed-width batch freezes
    // interrupt delivery and FreeRTOS never runs). That is a FIDELITY setting
    // and is not touched here or below.
    //
    // The advance call's `fuel` is a different thing: how much this call may
    // consume before returning to the checks at the top of this loop. And
    // `Machine::try_idle_fast_forward` clamps its skip to the fuel remaining —
    // so with fuel pinned to 1, an idle FreeRTOS window was fast-forwarded ONE
    // cycle at a time. Measured on a hosted-shaped ESP32-C3 BLE rom-boot run,
    // turning the flag on that way bought 3% of the steps and ran ~2x SLOWER in
    // wall clock than not skipping at all, because every skipped cycle paid a
    // full plan/commit round trip. A `vTaskDelay(200)` window is ~32M cycles;
    // it wants to go in a few thousand skips, not 32M of them.
    //
    // The widened fuel below is applied ONLY on an iteration where the CPU is
    // already parked waiting for an interrupt (`idle_fast_forward_budget` is
    // `Some`). That is the tight guard: while the CPU is parked the advance
    // call retires no instructions at all, so nothing this loop observes
    // between calls — PC, retired-step counts, assertion settling — can move
    // inside the widened window. On every instruction-retiring iteration the
    // fuel is exactly what it was before this change, so a busy run is
    // unchanged instruction for instruction.
    //
    // `idle_ff_wide_observation` is the standing half of the condition. It
    // excludes the features this loop — not `advance` — implements per
    // iteration and which a parked CPU does not exempt:
    //   * `--breakpoint` / `--detect-stuck` re-read the PC between calls, and
    //     a WFI spin is exactly a stuck PC,
    //   * `--capture-app-entry` watches for the app-entry PC between calls,
    //   * poll-mode logic capture and ShutdownLatency assertions need
    //     cycle-accurate attribution of events inside the window.
    // Time-triggered stimuli, UART injections and `max_cycles` are NOT in that
    // list: the per-iteration clamp below already shortens `limit` to land
    // exactly on the next threshold.
    //
    // With idle fast-forward off — including via `LABWIRED_IDLE_FAST_FORWARD=0`
    // — this is `false` and the loop is byte-identical to before.
    //
    // ⚠️ A run with `after_cycles` stimuli or UART injections turns idle
    // fast-forward OFF outright, not just the widened fuel. Those thresholds
    // are compared against `metrics.get_cycles()`, which is accumulated by a
    // per-STEP observer: an idle skip retires no instructions, so it advances
    // the machine's device clock without advancing that counter. Under
    // fast-forward the two clocks separate, and a stimulus whose threshold is
    // expressed in cycles would land late in device time — or, if the run ends
    // first, never fire at all while still reporting a pass. A run that says
    // when its input arrives gets instruction-for-instruction timing; the
    // acceleration is not worth silently moving someone's stimulus.
    let has_time_triggered_inputs = stimuli
        .iter()
        .any(|s| matches!(s.trigger, labwired_config::FaultTrigger::AfterCycles { .. }))
        || uart_injections
            .iter()
            .any(|u| matches!(u.trigger, labwired_config::FaultTrigger::AfterCycles { .. }));
    if has_time_triggered_inputs && machine.config.idle_fast_forward_enabled {
        machine.config.idle_fast_forward_enabled = false;
        eprintln!(
            "labwired-cli test: idle_ff disabled for this run — it declares \
             after_cycles stimuli/uart injections, whose thresholds idle skips \
             do not advance"
        );
    }

    // The `event-scheduler` clause is load-bearing, not belt-and-braces. Without
    // that feature `Machine::try_idle_fast_forward` is compiled to `0`, so there
    // is no skip for the wider fuel to fund — but `idle_fast_forward_budget`
    // still reports the parked CPU, and widening on that would hand `advance` a
    // million instructions of WFI spin to retire in one call. The outer loop's
    // per-iteration checks (`stop_when_assertions_pass` settling,
    // `max_uart_bytes`, `wall_time_ms`) would then run a million steps apart in
    // a DEFAULT build. Gated here, a build without the feature takes the
    // `else` arm exactly as it does today.
    let idle_ff_wide_observation = cfg!(feature = "event-scheduler")
        && machine.config.idle_fast_forward_enabled
        && args.breakpoint.is_empty()
        && detect_stuck.is_none()
        && args.capture_app_entry.is_none()
        && !machine.logic_poll_active()
        && !requires_fine_grained_observation(assertions);

    // Declarative input stimuli (schema_version 1.2). Applied via the generic
    // `Machine::set_input` path (see `labwired_core::sim_input`), so no per-type
    // wiring. `at_start` fires now; `after_cycles` fires the first loop
    // iteration at or past its cycle threshold. The closure takes `machine` as
    // an argument (captures nothing) so it can be called both here and mid-loop.
    //
    // The closure RETURNS the outcome rather than swallowing it. This used to
    // only `error!` a rejection into the log and carry on, so a run whose input
    // never reached the device still reported `status: "pass"` — a surface that
    // claims success having proved nothing. Every outcome is now recorded in
    // `stimulus_outcomes`, surfaced in `result.json`'s `stimuli` block, and a
    // rejection fails the run (see the verdict below).
    let mut stimulus_outcomes: Vec<StimulusOutcome> = Vec::new();
    let apply_stimulus = |machine: &mut labwired_core::Machine<C>,
                          s: &labwired_config::StimulusSpec| {
        let result = match s.target.component.as_deref() {
            Some(component) => machine.set_input_on(component, &s.target.channel, s.value),
            None => machine.set_input(&s.target.channel, s.value),
        };
        let (outcome, error) = match result {
            Ok(()) => {
                info!("stimulus: {} = {} applied", s.target.channel, s.value);
                (artifacts::STIMULUS_APPLIED, None)
            }
            // `SimInputError`'s Display is the author-facing sentence ("no
            // attached input device exposes channel 'pressed'"); the old `{:?}`
            // Debug form leaked Rust variant names into the log.
            Err(e) => {
                error!(
                    "stimulus '{}' = {} could not be applied: {e}",
                    s.target.channel, s.value
                );
                (artifacts::STIMULUS_REJECTED, Some(e.to_string()))
            }
        };
        StimulusOutcome {
            channel: s.target.channel.clone(),
            component: s.target.component.clone(),
            value: s.value,
            trigger: s.trigger.clone(),
            outcome: outcome.to_string(),
            at_cycle: machine.total_cycles,
            error,
        }
    };
    let mut stimulus_cycles: StimulusCycles = std::collections::HashMap::new();
    let mut stimulus_sequence = 0u64;
    let mut uart_milestone_cycles = UartMilestoneCycles::new(assertions.iter().filter_map(|a| {
        if let TestAssertion::ShutdownLatency(a) = a {
            Some(a.shutdown_latency.to_uart.clone())
        } else {
            None
        }
    }));
    for s in stimuli {
        if matches!(s.trigger, labwired_config::FaultTrigger::AtStart) {
            let outcome = apply_stimulus(machine, s);
            if outcome.error.is_none() {
                stimulus_sequence += 1;
                stimulus_cycles
                    .entry(stimulus_key(&s.target))
                    .or_default()
                    .push(StimulusApplication {
                        cycle: machine.total_cycles,
                        value: s.value,
                        sequence: stimulus_sequence,
                    });
            }
            stimulus_outcomes.push(outcome);
        }
    }
    // Time-triggered stimuli, each tagged with whether it has fired yet.
    let mut pending_stimuli: Vec<(&labwired_config::StimulusSpec, bool)> = stimuli
        .iter()
        .filter(|s| matches!(s.trigger, labwired_config::FaultTrigger::AfterCycles { .. }))
        .map(|s| (s, false))
        .collect();

    // Declarative UART RX injections (schema_version 1.2). Resolved against
    // the built bus by peripheral name via the same `attach_uart_rx_source`
    // family used by the wasm bridge (`attach_uart_rx_source_named`), then
    // pushed straight into the UART's RX `VecDeque` — the shared mechanism
    // that already backs interactive serial input. A byte pushed before the
    // firmware configures/reads the UART is buffered, not dropped: RX
    // presence is derived from the queue being non-empty (see
    // `Uart::read`), with no enable-bit gating. `at_start` delivers
    // immediately (before the firmware executes its first instruction);
    // `after_cycles` delivers the first loop iteration at or past its cycle
    // threshold, mirroring `apply_stimulus` above. A named UART that isn't
    // found on the bus is a hard config error — silently dropping serial
    // input a script depends on would be a false pass.
    let mut uart_injection_error = false;
    let apply_uart_injection =
        |machine: &mut labwired_core::Machine<C>, u: &labwired_config::UartInjectionSpec| {
            match machine.bus.attach_uart_rx_source_named(&u.uart) {
                Some(rx) => {
                    let bytes = u.bytes.as_bytes();
                    match rx.lock() {
                        Ok(mut guard) => {
                            guard.extend(bytes.iter().copied());
                            info!(
                                "uart_injection: {} byte(s) delivered to '{}'",
                                bytes.len(),
                                u.uart
                            );
                        }
                        Err(e) => error!("uart_injection '{}': RX buffer poisoned: {e}", u.uart),
                    }
                    None
                }
                None => Some(format!(
                    "uart_injection: UART peripheral '{}' not found on the bus",
                    u.uart
                )),
            }
        };
    for u in uart_injections {
        if matches!(u.trigger, labwired_config::FaultTrigger::AtStart) {
            if let Some(err) = apply_uart_injection(machine, u) {
                error!("{err}");
                uart_injection_error = true;
            }
        }
    }
    if uart_injection_error {
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }
    let mut pending_uart_injections: Vec<(&labwired_config::UartInjectionSpec, bool)> =
        uart_injections
            .iter()
            .filter(|u| matches!(u.trigger, labwired_config::FaultTrigger::AfterCycles { .. }))
            .map(|u| (u, false))
            .collect();

    // Tracks the step at which all runtime assertions first passed. The
    // `stop_when_assertions_pass` early-stop is only accepted after the machine
    // keeps executing for a settling window past this point WITHOUT faulting —
    // print-then-crash firmware breaks with its fault reason during the window
    // instead of certifying as passed. A regression (assertions stop passing)
    // resets it, so the pass must be durable.
    let mut assertions_first_passed_at: Option<u64> = None;
    // Milestone assertions describe state observed before a later injected
    // fault. Once the requested speed band has genuinely been observed in the
    // plant, retain that evidence through the shutdown phase.
    let mut assertion_latched = vec![false; assertions.len()];

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

    // ── stop_when_assertions_pass: per-step evaluation, made cheap ──────────
    //
    // The early-stop pins the CLI batch to one instruction (`batch_size`
    // above), so the block at the bottom of this loop runs once per RETIRED
    // GUEST INSTRUCTION. It used to copy the entire UART capture TWICE every
    // time — `Vec::clone`, then `String::from_utf8_lossy(..).to_string()` —
    // and re-scan the result for every assertion. MEASURED on the pinned C3
    // Arduino-BLE image (`e2e_esp32c3_ble_arduino`), 20 M steps: 5.46 s user
    // CPU with the scan, 2.16 s with the identical run and nothing to scan.
    // 60 % of the run was re-reading a 235-byte buffer that only changes a few
    // hundred times in 362 M steps, and `from_utf8_lossy(..).to_string()`
    // alone was 37 % of process samples.
    //
    // The verdict is a pure function of (uart_text, machine state), so it is
    // recomputed EXACTLY when one of those can have changed:
    //
    //   * `uart_text` changes only when the sink grows — a UART TX sink is
    //     append-only (nothing in this file or the core UART models truncates
    //     it; it is drained once, after the loop, to write `uart.log`), so its
    //     length is an exact change detector;
    //   * machine state changes every step, so any assertion that READS the
    //     machine (`memory_value`, `uds_tester`) still forces a recompute
    //     every step — those keep their old cost, which is the honest price of
    //     asking a question about live machine state.
    //
    // When every runtime assertion is UART-only (what every `uart_contains` /
    // `uart_regex` script is, including both BLE gates), an unchanged length
    // means an unchanged verdict and the cached one is reused. The verdict is
    // therefore identical on every step, so `assertions_first_passed_at`
    // latches on the same step and the run stops at the same step count.
    //
    // The cache is DELIBERATELY conservative about which assertion kinds count
    // as UART-only: `MotorSpeedReached` latches milestones and
    // `ShutdownLatency` reads `stimulus_cycles`/`uart_milestone_cycles`, both
    // of which move without the capture growing, so their presence disables
    // the cache and restores the original every-step evaluation.
    let has_runtime_assertions = assertions.iter().any(|a| {
        !matches!(
            a,
            TestAssertion::ExpectedStopReason(_)
                | TestAssertion::FirmwareExit(_)
                | TestAssertion::ResourceBudget(_)
        )
    });
    let assertions_are_uart_only = assertions
        .iter()
        .filter(|a| {
            !matches!(
                a,
                TestAssertion::ExpectedStopReason(_)
                    | TestAssertion::FirmwareExit(_)
                    | TestAssertion::ResourceBudget(_)
            )
        })
        .all(|a| {
            matches!(
                a,
                TestAssertion::UartContains(_) | TestAssertion::UartRegex(_)
            )
        });
    let mut cached_uart_text = String::new();
    // `usize::MAX` (not 0) so the first iteration always counts as a change and
    // evaluates, even when the capture is still empty.
    let mut cached_uart_len = usize::MAX;
    let mut cached_all_pass = false;

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
                // Same reason as `snapshot capture`: a CPU that models no
                // runtime snapshot answers `None`, and writing a resume file
                // without a CPU half would produce something that still looks
                // valid. Say so and write nothing. NOT a `continue` — the rest
                // of this loop body is what actually advances the machine, so
                // skipping it would hang the run instead of just declining the
                // capture.
                if let Some(mut snap) = machine.take_runtime_snapshot() {
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
                } else {
                    error!(
                        "capture-app-entry: this CPU has no runtime-snapshot implementation \
                         (supported: RISC-V, Xtensa LX7) — no snapshot written to {:?}",
                        cap.path
                    );
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
                        let outcome = apply_stimulus(machine, s);
                        if outcome.error.is_none() {
                            stimulus_sequence += 1;
                            stimulus_cycles
                                .entry(stimulus_key(&s.target))
                                .or_default()
                                .push(StimulusApplication {
                                    cycle: machine.total_cycles,
                                    value: s.value,
                                    sequence: stimulus_sequence,
                                });
                        }
                        stimulus_outcomes.push(outcome);
                        *fired = true;
                    }
                }
            }
        }
        // Fire any `after_cycles` UART injection whose threshold has been reached.
        if !pending_uart_injections.is_empty() {
            let cycles = metrics.get_cycles();
            for (u, fired) in pending_uart_injections.iter_mut() {
                if *fired {
                    continue;
                }
                if let labwired_config::FaultTrigger::AfterCycles { cycles: threshold } = u.trigger
                {
                    if cycles >= threshold {
                        if let Some(err) = apply_uart_injection(machine, u) {
                            error!("{err}");
                        }
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
        } else if idle_ff_wide_observation
            && machine
                .cpu
                .idle_fast_forward_budget(&machine.bus as &dyn labwired_core::Bus)
                .is_some()
        {
            // CPU is parked on WFI right now: give the skip real fuel so the
            // idle window goes in a few thousand skips instead of one cycle at
            // a time. The CPU batch width is still `current_batch` — see the
            // note beside `idle_ff_wide_observation`.
            (u64::from(remaining.min(IDLE_FF_RUN_CHUNK)), current_batch)
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
        for (injection, fired) in &pending_uart_injections {
            if !*fired {
                if let labwired_config::FaultTrigger::AfterCycles { cycles } = injection.trigger {
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
                // Statistical PC sampling: one histogram hit every
                // PC_SAMPLE_EVERY retired primary steps, using the post-batch
                // PC (no per-instruction observer — keeps JIT eligible).
                if report.primary_steps > 0 {
                    pc_sample_budget = pc_sample_budget.saturating_add(report.primary_steps);
                    while pc_sample_budget >= resource_report::PC_SAMPLE_EVERY {
                        pc_sample_budget -= resource_report::PC_SAMPLE_EVERY;
                        resource_report::note_pc_sample(&mut pc_hist, machine.cpu.get_pc());
                    }
                }
                // A firmware-authored verdict ends the run immediately: the
                // firmware has stated the result, so continuing would only let
                // a later timeout overwrite it.
                if let labwired_core::AdvanceStop::FirmwareExit { code } = report.stop {
                    info!("{} (step={})", firmware_exit_message(code), step);
                    stop_reason = StopReason::FirmwareExit;
                    firmware_exit_code = Some(code);
                    break;
                }
                if report.primary_steps == 0 && report.idle_cycles == 0 {
                    stop_reason = StopReason::Halt;
                    break;
                }
            }
            Err(error) => {
                sim_error_happened = true;
                if matches!(
                    error,
                    labwired_core::SimulationError::ExceptionRaised { .. }
                ) {
                    exception_count = exception_count.saturating_add(1);
                }
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

        if !uart_milestone_cycles.occurrences.is_empty() {
            let uart_bytes = uart_tx.lock().map(|g| g.clone()).unwrap_or_default();
            uart_milestone_cycles.observe(&uart_bytes, machine.total_cycles);
        }

        if resolved_limits.stop_when_assertions_pass && has_runtime_assertions {
            // Refresh the cached capture only when the sink actually grew.
            // One uncontended lock + a length compare on the common step.
            let uart_changed = match uart_tx.lock() {
                Ok(g) => {
                    if g.len() != cached_uart_len {
                        cached_uart_len = g.len();
                        cached_uart_text.clear();
                        cached_uart_text.push_str(&String::from_utf8_lossy(&g[..]));
                        true
                    } else {
                        false
                    }
                }
                // Poisoned mutex: the old code read this as an empty
                // capture (`unwrap_or_default`). Reproduce that, once.
                Err(_) => {
                    if cached_uart_len != 0 {
                        cached_uart_len = 0;
                        cached_uart_text.clear();
                        true
                    } else {
                        false
                    }
                }
            };
            if uart_changed || !assertions_are_uart_only {
                let uart_text = &cached_uart_text;
                for (index, assertion) in assertions.iter().enumerate() {
                    let milestone_observed = match assertion {
                        TestAssertion::MotorSpeedReached(_) => {
                            assertion_currently_passes(assertion, uart_text, machine)
                        }
                        _ => false,
                    };
                    if milestone_observed {
                        assertion_latched[index] = true;
                    }
                }
                cached_all_pass = assertions.iter().enumerate().all(|(index, assertion)| {
                    matches!(
                        assertion,
                        TestAssertion::ExpectedStopReason(_)
                            | TestAssertion::FirmwareExit(_)
                            | TestAssertion::ResourceBudget(_)
                    ) || (matches!(assertion, TestAssertion::MotorSpeedReached(_))
                        && assertion_latched[index])
                        || matches!(assertion, TestAssertion::ShutdownLatency(a)
                        if shutdown_latency_passes(
                            &a.shutdown_latency,
                            &stimulus_cycles,
                            &uart_milestone_cycles,
                        ))
                        || assertion_currently_passes(assertion, uart_text, machine)
                });
            }
            let all_pass = cached_all_pass;
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

    // How much of this run's device time the CPU spent parked and skipped
    // rather than interpreted. Printed whenever it is non-zero so a hosted run
    // can be shown to have actually fast-forwarded — the failure mode this
    // guards is a build or a run path where the flag is on and the skip is
    // clamped to nothing, which is indistinguishable from working unless the
    // number is visible. `steps_executed + skipped == machine.total_cycles`.
    if machine.idle_fast_forward_cycles_skipped > 0 {
        eprintln!(
            "labwired-cli test: idle_ff skipped {} of {} device cycles ({} interpreted)",
            machine.idle_fast_forward_cycles_skipped, machine.total_cycles, steps_executed
        );
    }

    let uart_text = {
        let bytes = uart_tx.lock().map(|g| g.clone()).unwrap_or_default();
        String::from_utf8_lossy(&bytes).to_string()
    };

    // Finalize main-stack report before assertion evaluation so
    // `resource_budget` can compare against high-water / footprint.
    // Snapshot bus access counts before paint scan / memory assertions pollute
    // the run-lifetime counters.
    let (memory_reads, memory_writes, peripheral_accesses) = machine.bus.access_counts();
    let top_pcs = resource_report::top_pc_samples(&pc_hist, resource_report::PC_SAMPLE_TOP_N);
    let pc_samples = resource_report::resolve_pc_sample_symbols(&top_pcs, firmware_path);
    let execution_metrics = artifacts::ExecutionMetrics {
        cycles: metrics.get_cycles(),
        instructions: metrics.get_instructions(),
        steps_executed,
        memory_reads,
        memory_writes,
        peripheral_accesses,
        exceptions: exception_count,
        pc_samples,
    };

    let memory = if let Some(session) = paint_session {
        let final_sp = resource_report::arm_sp(&machine.cpu);
        resource_report::finalize_paint_report(&machine.bus, final_sp, session)
    } else {
        memory_pre
    };

    let mut assertion_results = Vec::new();
    let mut all_passed = true;
    let mut expected_stop_reason_matched = false;

    for (assertion_index, assertion) in assertions.iter().enumerate() {
        let (passed, evidence) = match assertion {
            TestAssertion::UartContains(_)
            | TestAssertion::UartRegex(_)
            | TestAssertion::UartOrdered(_)
            | TestAssertion::MotorState(_)
            | TestAssertion::MqttFabric(_) => (
                assertion_currently_passes(assertion, &uart_text, machine),
                None,
            ),
            TestAssertion::MotorSpeedReached(_) => (
                assertion_latched[assertion_index]
                    || assertion_currently_passes(assertion, &uart_text, machine),
                None,
            ),
            TestAssertion::ShutdownLatency(a) => {
                let passed = shutdown_latency_passes(
                    &a.shutdown_latency,
                    &stimulus_cycles,
                    &uart_milestone_cycles,
                );
                let evidence = shutdown_latency_cycles(
                    &a.shutdown_latency,
                    &stimulus_cycles,
                    &uart_milestone_cycles,
                )
                .map(|(stimulus_cycle, token_cycle, latency_cycles)| {
                    AssertionEvidence::ShutdownLatency {
                        stimulus_cycle,
                        token_cycle,
                        latency_cycles,
                        configured_max_cycles: a.shutdown_latency.max_cycles,
                    }
                });
                (passed, evidence)
            }
            TestAssertion::ExpectedStopReason(a) => (a.expected_stop_reason == stop_reason, None),
            // Passes only if the FIRMWARE ended the run with exactly this code.
            // A timeout, halt or fault leaves `firmware_exit_code` None, so a
            // run that never reached its own success path fails rather than
            // passing by silence.
            TestAssertion::FirmwareExit(a) => (firmware_exit_code == Some(a.firmware_exit), None),
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

                let passed = match result {
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
                };
                (passed, None)
            }
            TestAssertion::UdsTester(a) => {
                let passed = match evaluate_uds_tester(&machine.bus.can_uds_testers, &a.uds_tester)
                {
                    Ok(()) => true,
                    Err(msg) => {
                        error!("Assertion failed: {}", msg);
                        false
                    }
                };
                (passed, None)
            }
            // The measurement itself carries the diagnosis (which region, how
            // much ink, what was required), so it is logged rather than
            // reduced to a bare `false`.
            TestAssertion::DisplayRegion(a) => {
                let passed = match evaluate_display_region(&machine.bus, &a.display_region) {
                    Ok(()) => true,
                    Err(msg) => {
                        error!("Assertion failed: {}", msg);
                        false
                    }
                };
                (passed, None)
            }
            TestAssertion::ResourceBudget(a) => {
                evaluate_resource_budget(&a.resource_budget, footprint.as_ref(), Some(&memory))
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
            evidence,
        });
    }

    let stop_requires_assertion = matches!(
        stop_reason,
        StopReason::WallTime | StopReason::MaxUartBytes | StopReason::NoProgress
    );

    // Any `after_cycles` stimulus whose threshold the run never reached also
    // proved nothing about that input, so it is recorded rather than dropped.
    // Unlike a rejection this is NOT fatal: a run can legitimately stop early
    // (`stop_when_assertions_pass`) with a later rung of a stimulus ladder
    // unfired, and failing those would flip existing green runs red on a pacing
    // judgement call. It is reported so the reader can see it.
    {
        let end_cycle = metrics.get_cycles();
        for (s, fired) in &pending_stimuli {
            if *fired {
                continue;
            }
            let threshold = match s.trigger {
                labwired_config::FaultTrigger::AfterCycles { cycles } => cycles,
                _ => continue,
            };
            error!(
                "stimulus '{}' = {} never fired: the run ended at cycle {end_cycle}, before its \
                 after_cycles threshold {threshold}",
                s.target.channel, s.value
            );
            stimulus_outcomes.push(StimulusOutcome {
                channel: s.target.channel.clone(),
                component: s.target.component.clone(),
                value: s.value,
                trigger: s.trigger.clone(),
                outcome: artifacts::STIMULUS_NOT_REACHED.to_string(),
                at_cycle: end_cycle,
                error: Some(format!(
                    "never fired: the run ended at cycle {end_cycle}, before the after_cycles \
                     threshold {threshold}"
                )),
            });
        }
    }

    // A stimulus the engine REFUSED never reached the device, so nothing the
    // run observed can be attributed to it — a "pass" here would be a run that
    // proved nothing, which is the single most expensive failure mode in this
    // codebase. It is therefore an invalid run, not a firmware verdict:
    // `status: "error"` + `EXIT_CONFIG_ERROR`, exactly like the `uart_injection`
    // peripheral-not-found gate above, which is the same class of failure
    // (declared input never delivered) and already hard-fails.
    let stimuli_rejected = stimulus_outcomes.iter().filter(|o| o.is_rejected()).count();
    if stimuli_rejected > 0 {
        error!(
            "{stimuli_rejected} stimulus/stimuli could not be applied; the run is invalid \
             (nothing it observed can be attributed to them)"
        );
    }

    // `rejected` dominates the other verdicts on purpose: a "fail" produced by a
    // run whose inputs were never delivered is not a trustworthy fail either.
    // A firmware that declared its own failure fails the run, whether or not
    // the script asserted anything. Without this a run with no assertions would
    // report `status: "pass"` for firmware that explicitly said `EXIT 5` — the
    // proved-nothing failure mode again, and the worse for being self-inflicted:
    // the run has an unambiguous verdict from the firmware itself and would be
    // ignoring it. `None` (a bare `STOP`, or any non-simctl stop) is not a
    // failure claim and does not trigger this.
    let firmware_declared_failure = firmware_exit_code.is_some_and(|code| code != 0);
    if firmware_declared_failure {
        error!(
            "firmware ended the run with a non-zero exit code ({}); the run fails",
            firmware_exit_code.unwrap_or_default()
        );
    }

    // Finalise runtime-observed fault outcomes (e.g. missing_clock fires only
    // when the firmware actually accessed the unclocked peripheral) and enforce
    // the require_fault_fired gate: a fault that never took effect makes the run
    // invalid, not a firmware pass.
    //
    // This must happen BEFORE the verdict, not after it. It used to sit thirty
    // lines below, which is precisely why `fault_gate_failed` could only reach
    // the exit code and never the `status` — the artifact certified a pass for
    // a run the exit code called invalid. See `crate::verdict`.
    labwired_cli::faults::finalize_fault_evidence(&machine.bus, faults, &mut fault_evidence);
    let fault_gate_failed = require_fault_fired && fault_evidence.iter().any(|e| !e.fired);
    if fault_gate_failed {
        let n = fault_evidence.iter().filter(|e| !e.fired).count();
        error!("require_fault_fired: {n} fault(s) did not fire; run is invalid");
    }

    // THE verdict. One decision, from which both `status` and the exit code are
    // read below — they cannot disagree because there is nothing left to
    // disagree with. Do not reintroduce a second chain here.
    let verdict = crate::verdict::RunFacts {
        stimuli_rejected: stimuli_rejected > 0,
        firmware_declared_failure,
        assertions_failed: !all_passed,
        fault_gate_failed,
        unexpected_safety_stop: stop_requires_assertion && !expected_stop_reason_matched,
        unrescued_runtime_error: sim_error_happened && !expected_stop_reason_matched,
    }
    .verdict();

    let duration = start.elapsed();
    let uart_bytes = uart_tx.lock().map(|g| g.len() as u64).unwrap_or(0);
    let stop_reason_details = crate::report::build_stop_reason_details(
        &stop_reason,
        resolved_limits,
        steps_executed,
        metrics.get_cycles(),
        uart_bytes,
        stuck_counter,
        duration,
        0, // vcd_bytes - will be updated below
    );
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

    // ── THE VERDICT ──────────────────────────────────────────────────────────
    //
    // `labwired test` is the deterministic gate, and until now a PASSING run
    // said nothing at all about what it had verified: failures went out through
    // `error!`, passes were silent, and the only machine-readable answer lived
    // in `result.json` / JUnit. A human running the gate in a terminal saw the
    // firmware's own UART output and had to infer the verdict from `$?`.
    //
    // One line, on STDERR. That is deliberate and it is what makes this safe to
    // print unconditionally: firmware UART echo and the `--json` agent payload
    // both go to stdout, so a human-facing line on stderr can never corrupt a
    // piped capture or a JSON parse. No new parameter threaded through this
    // already twenty-argument signature to decide whether to speak.
    {
        let checked = assertion_results.len();
        let passed = assertion_results.iter().filter(|a| a.passed).count();
        let label = verdict.banner_label();
        // The SCRIPT, not the system manifest: nearly every board ships its
        // manifest as `system.yaml`, so naming that would print the same
        // uninformative "system" for every board in the repo. The script stem
        // is what the caller actually typed and what a CI log needs to
        // identify. Firmware stem is the fallback for a scriptless run.
        let subject = args
            .script
            .file_stem()
            .or_else(|| firmware_path.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "run".to_string());
        eprintln!(
            "{label}  {passed}/{checked} checks · {subject} · {steps_executed} steps · {:.2}s",
            duration.as_secs_f64()
        );
    }

    write_outputs(
        args,
        verdict,
        steps_executed,
        metrics,
        stop_reason.clone(),
        stop_reason_details,
        firmware_exit_code,
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
        stimulus_outcomes,
        footprint,
        Some(memory),
        Some(execution_metrics),
    );

    // The same `verdict` the artifact above was written from. Not a second
    // chain — that is the whole point of `crate::verdict`.
    verdict.exit_code()
}

#[allow(clippy::too_many_arguments, clippy::if_same_then_else)]
fn write_outputs<C: labwired_core::Cpu>(
    args: &TestArgs,
    // The run's ONE verdict, not a status string. Taking `&str` here meant a
    // caller could invent a status that disagreed with the exit code it went on
    // to return; that is the drift `crate::verdict` exists to make
    // unrepresentable, so the artifact writer is handed the verdict itself.
    verdict: crate::verdict::Verdict,
    steps_executed: u64,
    metrics: &labwired_core::metrics::PerformanceMetrics,
    stop_reason: StopReason,
    stop_reason_details: StopReasonDetails,
    // Set only when the firmware ended its own run through `simctl`.
    firmware_exit_code: Option<u32>,
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
    stimuli: Vec<StimulusOutcome>,
    footprint: Option<artifacts::FootprintReport>,
    memory: Option<labwired_core::stack_paint::MainStackReport>,
    metrics_block: Option<artifacts::ExecutionMetrics>,
) {
    let status = verdict.status();

    let mut hasher = Sha256::new();
    hasher.update(firmware_bytes);
    let firmware_hash = format!("{:x}", hasher.finalize());

    // Drain the coverage-gap log for THIS run. `write_outputs` is called
    // synchronously at the tail of `execute_test_loop`, on the very thread that
    // ran the sim loop, so this reads the same thread-local the `record_*` calls
    // populated. `take()` resets it, so it must run exactly once per run — this
    // is the sole call site on the run path.
    let fidelity = labwired_core::fidelity::take().to_gaps();

    // Silent-path census (measurement only). Compiled to an empty function
    // unless `--features silent-census`, and even then writes nothing unless
    // LABWIRED_CENSUS_OUT names a path. Sits here because `write_outputs` is
    // the sole call site on the run path, reached synchronously at the tail of
    // `execute_test_loop` on the thread that ran the sim.
    labwired_core::census::dump_if_requested();

    // Derive the top-level `message` from the stimulus block rather than taking
    // it as a second parameter, so the human sentence and the structured
    // evidence cannot drift apart — one source of truth. A rejected stimulus is
    // fatal, so a reader who only ever looks at `status` + `message` still
    // cannot miss it.
    let rejected: Vec<String> = stimuli
        .iter()
        .filter(|o| o.is_rejected())
        .map(|o| o.describe())
        .collect();
    let message = (!rejected.is_empty()).then(|| {
        format!(
            "{} stimulus/stimuli could not be applied, so the run proved nothing about them: {}",
            rejected.len(),
            rejected.join("; ")
        )
    });

    let assertions_for_junit = assertions.clone();
    let result = TestResult {
        result_schema_version: RESULT_SCHEMA_VERSION.to_string(),
        status: status.to_string(),
        steps_executed,
        cycles: metrics.get_cycles(),
        instructions: metrics.get_instructions(),
        stop_reason,
        stop_reason_details: stop_reason_details.clone(),
        firmware_exit_code,
        limits: limits.clone(),
        message,
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
        stimuli,
        footprint,
        memory,
        metrics: metrics_block,
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
                // Stamp honestly: `any_noise_enabled` matches kit config keys
                // and declarative noise inputs. Seeded noise is bit-identical
                // across runs, but it is not the absence of variation.
                let nondeterminism = system_path
                    .and_then(|sys| std::fs::read_to_string(sys).ok())
                    .filter(|yaml| manifest::any_noise_enabled(yaml))
                    .map(|_| "seeded(sensor-noise)".to_string())
                    .unwrap_or_else(|| "none".to_string());
                let mut man = manifest::RunManifest {
                    manifest_schema_version: manifest::MANIFEST_SCHEMA_VERSION.to_string(),
                    engine_version: env!("CARGO_PKG_VERSION").to_string(),
                    seed: 0,
                    nondeterminism,
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
    let stop_reason_details = crate::report::build_stop_reason_details(
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
        // A config error never ran firmware, so there is no verdict.
        firmware_exit_code: None,
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
        // Nor any stimulus outcomes: the run was rejected before a machine
        // existed, so no stimulus was ever attempted.
        stimuli: Vec::new(),
        // Config error: no firmware footprint or stack paint collected.
        footprint: None,
        memory: None,
        metrics: None,
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
        TestAssertion::UartOrdered(a) => format!("uart_ordered: {:?}", a.uart_ordered),
        TestAssertion::MotorSpeedReached(a) => format!(
            "motor_speed_reached: {} {}..={} rpm",
            a.motor_speed_reached.id,
            a.motor_speed_reached.min_abs_rpm,
            a.motor_speed_reached.max_abs_rpm
        ),
        TestAssertion::MotorState(a) => format!(
            "motor_state: {} state={}",
            a.motor_state.id, a.motor_state.control_state
        ),
        TestAssertion::ShutdownLatency(a) => format!(
            "shutdown_latency: {} <= {} cycles",
            a.shutdown_latency.to_uart, a.shutdown_latency.max_cycles
        ),
        TestAssertion::ExpectedStopReason(a) => {
            format!("expected_stop_reason: {:?}", a.expected_stop_reason)
        }
        TestAssertion::FirmwareExit(a) => format!("firmware_exit: {}", a.firmware_exit),
        TestAssertion::MemoryValue(a) => format!(
            "memory_value: @{:#x}={:#x}",
            a.memory_value.address, a.memory_value.expected_value
        ),
        TestAssertion::MqttFabric(a) => {
            let mut s = format!("mqtt_fabric: topic={}", a.mqtt_fabric.topic);
            if let Some(p) = &a.mqtt_fabric.payload_contains {
                s.push_str(&format!(" payload_contains={p}"));
            }
            s
        }
        TestAssertion::UdsTester(a) => {
            format!(
                "uds_tester: {} result={:?}",
                a.uds_tester.id, a.uds_tester.result
            )
        }
        TestAssertion::DisplayRegion(a) => {
            let d = &a.display_region;
            let dim = |v: Option<usize>| v.map(|n| n.to_string()).unwrap_or_else(|| "*".into());
            format!(
                "display_region: {} ({},{}) {}x{} ink {:.2}..={:.2}",
                d.id,
                d.x,
                d.y,
                dim(d.w),
                dim(d.h),
                d.min_ink,
                d.max_ink.unwrap_or(1.0)
            )
        }
        TestAssertion::ResourceBudget(a) => {
            let b = &a.resource_budget;
            if let Some(n) = b.max_flash_bytes {
                format!("resource_budget: max_flash_bytes={n}")
            } else if let Some(n) = b.max_ram_static_bytes {
                format!("resource_budget: max_ram_static_bytes={n}")
            } else if let Some(n) = b.max_main_stack_bytes {
                format!("resource_budget: max_main_stack_bytes={n}")
            } else {
                "resource_budget".to_string()
            }
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
/// Does `pattern` match anywhere in `text`?
///
/// Thin wrapper over [`crate::regex`], which replaced a `^ $ . *`-only matcher.
/// The call sites want a plain `bool`, so a pattern that cannot be evaluated is
/// logged and reported as "did not match" — which makes the assertion fail. A
/// typo therefore fails the test loudly instead of being mistaken for a
/// firmware bug that never printed the expected line.
pub(crate) fn simple_regex_is_match(pattern: &str, text: &str) -> bool {
    match crate::regex::is_match(pattern, text) {
        Ok(hit) => hit,
        Err(e) => {
            error!("uart_regex `{pattern}`: {e}");
            false
        }
    }
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

    fn shutdown_details(max_cycles: u64) -> labwired_config::ShutdownLatencyDetails {
        labwired_config::ShutdownLatencyDetails {
            from_stimulus: labwired_config::StimulusTarget {
                component: Some("drive_motor".to_owned()),
                channel: "stall".to_owned(),
            },
            stimulus_occurrence: 1,
            to_uart: "INVERTER OFF".to_owned(),
            uart_occurrence: 1,
            max_cycles,
        }
    }

    fn application(cycle: u64, value: f64, sequence: u64) -> StimulusApplication {
        StimulusApplication {
            cycle,
            value,
            sequence,
        }
    }

    #[test]
    fn shutdown_latency_ignores_pre_trigger_token_and_selects_post_trigger_occurrence() {
        let details = shutdown_details(100);
        let mut uart = UartMilestoneCycles::new([details.to_uart.clone()]);
        uart.observe(b"INVERTER OFF", 90);
        uart.observe(b"INVERTER OFF then INVERTER OFF", 220);
        let stimuli = std::collections::HashMap::from([(
            stimulus_key(&details.from_stimulus),
            vec![application(200, 1.0, 1)],
        )]);

        assert_eq!(uart.cycles("INVERTER OFF").collect::<Vec<_>>(), [90, 220]);
        assert!(shutdown_latency_passes(&details, &stimuli, &uart));
    }

    #[test]
    fn shutdown_latency_records_split_token_once_at_completion_cycle() {
        let mut uart = UartMilestoneCycles::new(["INVERTER OFF".to_owned()]);
        uart.observe(b"INVERTER O", 120);
        assert_eq!(uart.cycles("INVERTER OFF").next(), None);
        uart.observe(b"INVERTER OFF", 130);
        uart.observe(b"INVERTER OFF later", 180);

        assert_eq!(uart.cycles("INVERTER OFF").collect::<Vec<_>>(), [130]);
    }

    #[test]
    fn shutdown_latency_accepts_within_bound_and_rejects_beyond_bound() {
        let details = shutdown_details(100);
        let stimuli = std::collections::HashMap::from([(
            stimulus_key(&details.from_stimulus),
            vec![application(200, 1.0, 1)],
        )]);
        let mut within = UartMilestoneCycles::new([details.to_uart.clone()]);
        within.observe(b"INVERTER OFF", 300);
        assert!(shutdown_latency_passes(&details, &stimuli, &within));

        let mut beyond = UartMilestoneCycles::new([details.to_uart.clone()]);
        beyond.observe(b"INVERTER OFF", 301);
        assert!(!shutdown_latency_passes(&details, &stimuli, &beyond));
    }

    #[test]
    fn shutdown_latency_uses_actual_matching_stimulus_application_cycle() {
        let details = shutdown_details(50);
        let stimuli = std::collections::HashMap::from([
            (
                stimulus_key(&details.from_stimulus),
                vec![application(1_025, 1.0, 2)],
            ),
            (
                (Some("other".to_owned()), "stall".to_owned()),
                vec![application(900, 1.0, 1)],
            ),
        ]);
        let mut uart = UartMilestoneCycles::new([details.to_uart.clone()]);
        uart.observe(b"INVERTER OFF", 1_070);

        assert!(shutdown_latency_passes(&details, &stimuli, &uart));
    }

    #[test]
    fn shutdown_latency_selects_repeated_stimulus_and_uart_occurrences() {
        let mut details = shutdown_details(50);
        details.stimulus_occurrence = 2;
        details.uart_occurrence = 2;
        let stimuli = std::collections::HashMap::from([(
            stimulus_key(&details.from_stimulus),
            vec![application(100, 0.0, 1), application(300, 1.0, 2)],
        )]);
        let mut uart = UartMilestoneCycles::new([details.to_uart.clone()]);
        uart.observe(b"INVERTER OFF", 90);
        uart.observe(b"INVERTER OFF x INVERTER OFF", 330);
        uart.observe(b"INVERTER OFF x INVERTER OFF y INVERTER OFF", 340);

        assert_eq!(
            shutdown_latency_cycles(&details, &stimuli, &uart),
            Some((300, 340, 40))
        );
        assert_eq!(stimuli.values().next().unwrap()[1].value, 1.0);
        assert_eq!(stimuli.values().next().unwrap()[1].sequence, 2);
    }

    #[test]
    fn shutdown_latency_missing_selected_occurrence_fails() {
        let mut details = shutdown_details(100);
        details.stimulus_occurrence = 2;
        let stimuli = std::collections::HashMap::from([(
            stimulus_key(&details.from_stimulus),
            vec![application(100, 1.0, 1)],
        )]);
        let mut uart = UartMilestoneCycles::new([details.to_uart.clone()]);
        uart.observe(b"INVERTER OFF", 120);
        assert!(!shutdown_latency_passes(&details, &stimuli, &uart));
    }

    #[test]
    fn shutdown_latency_final_evaluation_does_not_depend_on_early_stop() {
        let details = shutdown_details(25);
        let stimuli = std::collections::HashMap::from([(
            stimulus_key(&details.from_stimulus),
            vec![application(500, 1.0, 1)],
        )]);
        let mut uart = UartMilestoneCycles::new([details.to_uart.clone()]);
        uart.observe(b"INVERTER OFF", 520);

        // This is the same direct evaluator used by final result construction;
        // no assertions-pass latch or early-stop state participates.
        assert!(shutdown_latency_passes(&details, &stimuli, &uart));
        uart.observe(b"INVERTER OFF then INVERTER OFF", 600);
        let mut second = details.clone();
        second.uart_occurrence = 2;
        assert!(!shutdown_latency_passes(&second, &stimuli, &uart));
    }

    #[test]
    fn shutdown_latency_evidence_serializes_into_assertion_result() {
        let result = AssertionResult {
            assertion: TestAssertion::ShutdownLatency(labwired_config::ShutdownLatencyAssertion {
                shutdown_latency: shutdown_details(25),
            }),
            passed: true,
            evidence: Some(AssertionEvidence::ShutdownLatency {
                stimulus_cycle: 500,
                token_cycle: 520,
                latency_cycles: 20,
                configured_max_cycles: 25,
            }),
        };
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["evidence"]["type"], "shutdown_latency");
        assert_eq!(json["evidence"]["stimulus_cycle"], 500);
        assert_eq!(json["evidence"]["token_cycle"], 520);
        assert_eq!(json["evidence"]["latency_cycles"], 20);
        assert_eq!(json["evidence"]["configured_max_cycles"], 25);
    }

    #[test]
    fn shutdown_latency_forces_instruction_boundary_run_loop_observation() {
        let assertion = TestAssertion::ShutdownLatency(labwired_config::ShutdownLatencyAssertion {
            shutdown_latency: shutdown_details(3),
        });
        let assertions = [assertion];
        assert!(requires_fine_grained_observation(&assertions));
        assert_eq!(
            assertion_observation_batch_size(true, false, &assertions, 50_000),
            1
        );

        // Model two runner observations after separate retired instructions.
        // Under the old 10k batch both would have been stamped at batch end.
        let mut uart = UartMilestoneCycles::new(["INVERTER OFF".to_owned()]);
        uart.observe(b"INVERTER OFF", 101);
        uart.observe(b"INVERTER OFF x INVERTER OFF", 103);
        assert_eq!(uart.cycles("INVERTER OFF").collect::<Vec<_>>(), [101, 103]);
        let details = shutdown_details(3);
        let stimuli = std::collections::HashMap::from([(
            stimulus_key(&details.from_stimulus),
            vec![application(100, 1.0, 1)],
        )]);
        assert!(shutdown_latency_passes(&details, &stimuli, &uart));
    }

    /// A display is polled on the batch grid rather than per instruction, and
    /// ONLY a display changes that: every script that existed before this
    /// assertion must keep the batch width it had, or a latched observation
    /// somewhere else silently moves.
    #[test]
    fn display_region_relaxes_the_poll_grid_without_touching_other_scripts() {
        let display = [TestAssertion::DisplayRegion(
            labwired_config::DisplayRegionAssertion {
                display_region: labwired_config::DisplayRegionDetails {
                    id: "tft".into(),
                    x: 0,
                    y: 0,
                    w: None,
                    h: None,
                    min_ink: 1.0,
                    max_ink: None,
                },
            },
        )];
        let uart = [TestAssertion::UartContains(
            labwired_config::UartContainsAssertion {
                uart_contains: "ready".into(),
            },
        )];

        // The new branch: stop-when-assertions-pass + a display => batch grid.
        assert_eq!(
            assertion_observation_batch_size(true, true, &display, 50_000_000),
            DISPLAY_POLL_BATCH
        );
        // Unchanged: the same script shape without a display still polls per step.
        assert_eq!(
            assertion_observation_batch_size(true, true, &uart, 50_000),
            1
        );
        // Unchanged: no early stop => the ordinary 10k batch, display or not.
        assert_eq!(
            assertion_observation_batch_size(true, false, &display, 50_000),
            10_000
        );
        // Unchanged: batching off wins over everything.
        assert_eq!(
            assertion_observation_batch_size(false, true, &display, 50_000),
            1
        );
        // A short run never batches past its own budget.
        assert_eq!(
            assertion_observation_batch_size(true, true, &display, 500),
            500
        );
    }

    #[test]
    fn jit_request_selection_respects_latency_observation_policy() {
        let latency = [TestAssertion::ShutdownLatency(
            labwired_config::ShutdownLatencyAssertion {
                shutdown_latency: shutdown_details(3),
            },
        )];
        assert!(!assertion_compatible_jit_eligibility(true, &latency));
        assert_eq!(
            assertion_observation_batch_size(true, false, &latency, 1_000_000),
            1
        );

        let ordinary = [TestAssertion::UartContains(
            labwired_config::UartContainsAssertion {
                uart_contains: "OK".to_owned(),
            },
        )];
        assert!(assertion_compatible_jit_eligibility(true, &ordinary));
        assert_eq!(
            assertion_observation_batch_size(true, false, &ordinary, 1_000_000),
            10_000
        );
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

#[cfg(test)]
mod resource_budget_tests {
    use super::*;
    use labwired_core::stack_paint::{MainStackMethod, MainStackReport};

    fn flash_details(limit: u64) -> labwired_config::ResourceBudgetDetails {
        labwired_config::ResourceBudgetDetails {
            max_flash_bytes: Some(limit),
            max_ram_static_bytes: None,
            max_main_stack_bytes: None,
        }
    }

    fn stack_details(limit: u64) -> labwired_config::ResourceBudgetDetails {
        labwired_config::ResourceBudgetDetails {
            max_flash_bytes: None,
            max_ram_static_bytes: None,
            max_main_stack_bytes: Some(limit),
        }
    }

    fn sample_footprint(flash: u64, ram: u64) -> artifacts::FootprintReport {
        artifacts::FootprintReport {
            method: "elf_section_totals_v1".to_string(),
            text_bytes: flash,
            data_bytes: 0,
            bss_bytes: ram,
            flash_used_bytes: flash,
            ram_static_bytes: ram,
            flash_total_bytes: None,
            ram_total_bytes: None,
            flash_used_pct: None,
            ram_static_pct: None,
            notes: vec![],
        }
    }

    #[test]
    fn flash_budget_passes_when_measured_within_limit() {
        let fp = sample_footprint(1000, 200);
        let (passed, evidence) = evaluate_resource_budget(&flash_details(1000), Some(&fp), None);
        assert!(passed);
        assert!(evidence.is_none());
    }

    #[test]
    fn flash_budget_fails_with_evidence_when_over_limit() {
        let fp = sample_footprint(1001, 200);
        let (passed, evidence) = evaluate_resource_budget(&flash_details(1000), Some(&fp), None);
        assert!(!passed);
        let Some(AssertionEvidence::ResourceBudget {
            name,
            measured,
            limit,
            method,
        }) = evidence
        else {
            panic!("expected ResourceBudget evidence");
        };
        assert_eq!(name, "max_flash_bytes");
        assert_eq!(measured, Some(1001));
        assert_eq!(limit, 1000);
        assert_eq!(method, "elf_section_totals_v1");
    }

    #[test]
    fn flash_budget_fails_when_footprint_unavailable() {
        let (passed, evidence) = evaluate_resource_budget(&flash_details(1000), None, None);
        assert!(!passed);
        let Some(AssertionEvidence::ResourceBudget {
            measured, method, ..
        }) = evidence
        else {
            panic!("expected ResourceBudget evidence");
        };
        assert_eq!(measured, None);
        assert_eq!(method, "footprint_unavailable");
    }

    #[test]
    fn main_stack_budget_uses_high_water_and_paint_method() {
        let mem = MainStackReport {
            main_stack_method: MainStackMethod::Paint,
            main_stack_limit_bytes: Some(2048),
            main_stack_high_water_bytes: Some(512),
            main_stack_free_min_bytes: Some(1536),
            main_stack_base: Some(0x2000_0000),
            main_stack_top: Some(0x2000_0800),
            main_stack_overflow_suspected: Some(false),
            main_stack_unsupported_reason: None,
            heap_method: Some("paint".to_string()),
            heap_limit_bytes: Some(2048),
            heap_high_water_bytes: Some(0),
            heap_free_min_bytes: Some(1536),
            heap_base: Some(0x2000_0000),
            heap_top: Some(0x2000_0800),
        };
        let (passed, evidence) = evaluate_resource_budget(&stack_details(512), None, Some(&mem));
        assert!(passed);
        assert!(evidence.is_none());

        let (passed, evidence) = evaluate_resource_budget(&stack_details(511), None, Some(&mem));
        assert!(!passed);
        match evidence {
            Some(AssertionEvidence::ResourceBudget {
                name,
                measured,
                limit,
                method,
            }) => {
                assert_eq!(name, "max_main_stack_bytes");
                assert_eq!(measured, Some(512));
                assert_eq!(limit, 511);
                assert_eq!(method, "paint");
            }
            other => panic!("unexpected evidence: {other:?}"),
        }
    }

    #[test]
    fn main_stack_budget_fails_when_high_water_missing() {
        let mem = MainStackReport::disabled();
        let (passed, evidence) = evaluate_resource_budget(&stack_details(512), None, Some(&mem));
        assert!(!passed);
        match evidence {
            Some(AssertionEvidence::ResourceBudget {
                measured, method, ..
            }) => {
                assert_eq!(measured, None);
                assert_eq!(method, "disabled");
            }
            other => panic!("unexpected evidence: {other:?}"),
        }
    }

    #[test]
    fn resource_budget_fail_evidence_serializes() {
        let result = AssertionResult {
            assertion: TestAssertion::ResourceBudget(labwired_config::ResourceBudgetAssertion {
                resource_budget: flash_details(100),
            }),
            passed: false,
            evidence: Some(AssertionEvidence::ResourceBudget {
                name: "max_flash_bytes".to_string(),
                measured: Some(150),
                limit: 100,
                method: "elf_section_totals_v1".to_string(),
            }),
        };
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["evidence"]["type"], "resource_budget");
        assert_eq!(json["evidence"]["name"], "max_flash_bytes");
        assert_eq!(json["evidence"]["measured"], 150);
        assert_eq!(json["evidence"]["limit"], 100);
        assert_eq!(json["evidence"]["method"], "elf_section_totals_v1");
        assert_eq!(json["passed"], false);
    }
}

#[cfg(test)]
mod simctl_exit_tests {
    use super::*;

    #[test]
    fn the_message_names_the_code() {
        assert!(firmware_exit_message(42).contains("42"));
    }

    #[test]
    fn the_stop_reason_serialises_as_snake_case_for_the_json_contract() {
        let json = serde_json::to_string(&StopReason::FirmwareExit).unwrap();
        assert_eq!(json, "\"firmware_exit\"");
        let back: StopReason = serde_json::from_str("\"firmware_exit\"").unwrap();
        assert_eq!(back, StopReason::FirmwareExit);
    }

    #[test]
    fn the_assertion_parses_from_a_test_script() {
        let assertion: labwired_config::TestAssertion =
            serde_yaml::from_str("firmware_exit: 0").expect("firmware_exit should parse");
        assert!(matches!(
            assertion,
            labwired_config::TestAssertion::FirmwareExit(ref a) if a.firmware_exit == 0
        ));
    }

    #[test]
    fn the_assertion_does_not_swallow_other_assertion_shapes() {
        // TestAssertion is `untagged`, so a new arm can hijack neighbouring
        // shapes if its fields are not distinctive. Prove it does not.
        let uart: labwired_config::TestAssertion =
            serde_yaml::from_str("uart_contains: \"PASS\"").unwrap();
        assert!(matches!(
            uart,
            labwired_config::TestAssertion::UartContains(_)
        ));
        let stop: labwired_config::TestAssertion =
            serde_yaml::from_str("expected_stop_reason: firmware_exit").unwrap();
        assert!(matches!(
            stop,
            labwired_config::TestAssertion::ExpectedStopReason(_)
        ));
    }

    /// The run-result JSON must stay readable by consumers written before this
    /// field existed — and must not sprout the field on runs that never used
    /// the device.
    #[test]
    fn a_pre_change_result_json_still_deserialises() {
        let legacy = serde_json::json!({
            "result_schema_version": "1.0",
            "status": "pass",
            "steps_executed": 10,
            "cycles": 10,
            "instructions": 10,
            "stop_reason": "max_steps",
            "stop_reason_details": {
                "triggered_stop_condition": "max_steps",
                "triggered_limit": null,
                "observed": null
            },
            "limits": serde_json::to_value(TestLimits {
                max_steps: 1,
                max_cycles: None,
                max_uart_bytes: None,
                no_progress_steps: None,
                wall_time_ms: None,
                max_vcd_bytes: None,
                stop_when_assertions_pass: false,
                stop_when_assertions_pass_settle_steps: 0,
                stop_when_assertions_pass_min_steps: 0,
            })
            .unwrap(),
            "assertions": [],
            "firmware_hash": "abc",
            "config": {"firmware": "f.elf", "system": null, "script": "t.yaml"},
        });
        let parsed: Result<crate::artifacts::TestResult, _> = serde_json::from_value(legacy);
        assert!(
            parsed.is_ok(),
            "adding firmware_exit_code broke the existing result contract: {:?}",
            parsed.err()
        );
        assert_eq!(parsed.unwrap().firmware_exit_code, None);
    }
}
