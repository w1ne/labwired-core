// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

// The library is also the `labwired` binary's home: `src/main.rs` is a thin
// shim over [`run_with_plugins`]. `extern crate self` keeps the
// `labwired_cli::...` paths the binary used valid inside the library.
extern crate self as labwired_cli;

/// Analog-waveform export for `--analog-trace` (CSV / VCD).
pub mod analog_trace;
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
mod rtt_stdin;
pub mod test_support;
pub mod tier1;
pub mod verdict;

mod api_client;
mod artifacts;
mod asset_validation;
mod commands;
mod component_validation;
/// Top-level `Commands` enum and CLI entry-point dispatch (`run_with_plugins`).
mod dispatch;
/// `execute_test_loop` and the `TestExecutionContext` it takes.
mod execute;
mod gpio_observer;
/// The three run-outcome writers (`write_outputs`, `write_config_error_outputs`,
/// `write_junit_xml`), all deriving from `artifacts::TestOutcome`.
mod outputs;
mod resource_report;
mod size_limited_writer;
mod stimuli;
mod vcd_trace;
mod wifi_frames;

pub use dispatch::{check_plugin_versions, run_with_plugins};
pub(crate) use dispatch::{plugin_chip_yaml, Cli};
pub(crate) use execute::{execute_test_loop, TestExecutionContext};
pub(crate) use outputs::{
    resolve_script_path, write_config_error_outputs, write_outputs, xml_escape,
};

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
use tracing::{debug, error, info, warn};

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
    /// Path to the chip descriptor YAML. Required unless --system is given
    /// (the manifest then names the chip: the system-aware driver).
    #[arg(long, required_unless_present = "system")]
    pub chip: Option<PathBuf>,

    /// Path to the firmware ELF.
    #[arg(long)]
    pub firmware: PathBuf,

    /// Board manifest (SystemManifest YAML). Two shapes:
    ///
    /// * With `--chip`: attach the manifest's `external_devices:` to the chip
    ///   before the run — a display, a sensor, anything the board carries.
    ///   ESP32-S3 only for now (the path `--rom-boot` uses); other families
    ///   build their bus through `SystemBus::from_config` and take a manifest
    ///   via the top-level `--system`.
    /// * Without `--chip`: select the system-aware driver, which resolves the
    ///   chip from the manifest, attaches its external devices, and (ARM only
    ///   in this phase) applies `--stimulus` entries. An explicit `--max-steps`
    ///   budget is required in this shape.
    #[arg(long)]
    pub system: Option<PathBuf>,

    /// Declarative input stimulus as JSON (repeatable), agent MCP shape:
    /// {"channel":"x","value":2.0,"after_cycles":3000000,"component":"fxos8700"}.
    /// Requires the system-aware driver (`--system` without `--chip`);
    /// after_cycles omitted or 0 applies at start.
    #[arg(long = "stimulus", value_name = "JSON")]
    pub stimulus: Vec<String>,

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

    /// Maximum number of simulator steps before exit (default: unlimited;
    /// required when --system is given without --chip). In accelerated ARM
    /// runs, each coalesced idle cycle also consumes one step of this budget.
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

    /// Optional path for the in-core analog engine's waveform trace — the node
    /// voltages and branch currents an `adapter: analog` co-simulation model
    /// solved. `.csv` writes `time_ns,<channel>...`; any other extension writes
    /// a VCD with one `real` variable per channel, so the analog curve opens
    /// beside the digital capture in GTKWave / PulseView.
    ///
    /// Empty unless a co-simulation runner is attached to the run; see
    /// `docs/cosimulation_plugins.md`.
    #[arg(long = "analog-trace", value_name = "PATH")]
    pub analog_trace: Option<PathBuf>,

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
    /// the batched path. ARM and RISC-V use batching by default for ordinary
    /// runs. On RISC-V the flag refuses the
    /// per-instruction instrumentation (`--break-at`, the WiFi bridge, the DHCP
    /// trace) that would silently turn it back into single-stepping; on Xtensa
    /// this flag is not supported, so it is rejected outright.
    ///
    /// Keeps throughput measurements explicit even when instrumentation would
    /// otherwise select single-stepping. It also prints a `[batched] ...`
    /// summary line to stderr on exit, so a caller can
    /// prove which path executed rather than assume it.
    #[arg(long = "batched")]
    pub batched: bool,

    /// Host wall-clock policy. `max-speed` (default) never sleeps; `realtime`
    /// sleeps when virtual time (`cycles/cpu_hz`) is at least 1 ms ahead of wall.
    #[arg(long = "time-mode", value_name = "MODE", default_value_t = labwired_core::HostTimeMode::MaxSpeed)]
    pub time_mode: labwired_core::HostTimeMode,
}

impl RunArgs {
    /// The chip descriptor path. Clap guarantees `--chip` whenever the
    /// system-only driver was not selected; the system driver resolves the
    /// chip from the manifest instead.
    pub(crate) fn chip_path(&self) -> &Path {
        self.chip
            .as_deref()
            .expect("clap enforces --chip unless --system replaces it")
    }
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

    /// Optional path for the in-core analog engine's waveform trace (`.csv`
    /// writes `time_ns,<channel>...`, any other extension writes VCD `real`
    /// vars). Empty unless a co-simulation runner is attached to the run.
    #[arg(long = "analog-trace", value_name = "PATH")]
    analog_trace: Option<PathBuf>,

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
    a.config.host_time_mode = args.time_mode;
    b.config.host_time_mode = args.time_mode;
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
    a.config.host_time_mode = args.time_mode;
    b.config.host_time_mode = args.time_mode;
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
    m.config.host_time_mode = args.time_mode;
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
        if cli.rtt && step % 1024 == 0 {
            let pending = crate::rtt_stdin::drain_rtt_stdin();
            if !pending.is_empty() {
                let _ = machine.bus.write_rtt_input(&pending);
            }
        }
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
    rtt_tx: &Arc<Mutex<Vec<u8>>>,
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
        rtt_tx,
        // No bus exists on the load-error path, so RTT status is unavailable.
        None,
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

/// The assertions decided by captured RTT text alone, and nothing else.
///
/// Same contract as [`uart_assertion_passes`]: `None` means "not decided by
/// this stream" and sends the caller on to the machine. The RTT capture is a
/// separate buffer from UART on purpose — mixing them would let an
/// `rtt_contains` token match a UART banner and vice versa.
fn rtt_assertion_passes(assertion: &TestAssertion, rtt_text: &str) -> Option<bool> {
    Some(match assertion {
        TestAssertion::RttContains(a) => rtt_text.contains(&a.rtt_contains),
        _ => return None,
    })
}

fn assertion_currently_passes(
    assertion: &TestAssertion,
    uart_text: &str,
    rtt_text: &str,
    machine: &labwired_core::Machine<impl labwired_core::Cpu>,
) -> bool {
    if let Some(passed) = uart_assertion_passes(assertion, uart_text) {
        return passed;
    }
    if let Some(passed) = rtt_assertion_passes(assertion, rtt_text) {
        return passed;
    }
    match assertion {
        // Handled above by `uart_assertion_passes` / `rtt_assertion_passes`.
        TestAssertion::UartContains(_)
        | TestAssertion::UartRegex(_)
        | TestAssertion::UartOrdered(_)
        | TestAssertion::RttContains(_) => {
            unreachable!("decided by uart_assertion_passes/rtt_assertion_passes")
        }
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

    // `lit` is checked BEFORE the pixels, because it answers a different
    // question and a failure here explains a passing ink measurement rather
    // than contradicting it: the frame really was painted, onto a panel that
    // cannot show it.
    if let Some(want_lit) = d.lit {
        let got = artifact
            .meta
            .get("lit")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| {
                format!(
                    "display_region '{}': `lit` was asserted but this panel publishes no \
                     `meta.lit` -- it has no emissive state to report, so the assertion \
                     cannot be measured (do not ask it of a backlit panel)",
                    d.id
                )
            })?;
        if got != want_lit {
            let brightness = artifact
                .meta
                .get("brightness")
                .and_then(|v| v.as_u64())
                .map(|b| format!(", brightness={b}"))
                .unwrap_or_default();
            return Err(format!(
                "display_region '{}': lit is {got}, expected {want_lit}{brightness}. An \
                 emissive panel shows nothing at brightness 0 however much was painted \
                 into frame memory -- check that the firmware writes WRDISBV and leaves \
                 sleep",
                d.id
            ));
        }
    }

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

/// Write `--analog-trace <path>` if it was given.
///
/// Non-fatal on I/O error, like the bus-trace export: the simulation already
/// finished and its verdict does not depend on a waveform file. `labwired test`
/// attaches its co-simulation session's ring, so a manifest with an
/// `adapter: analog` model gets the real waveform. A run with no runner, or
/// whose models record none, writes the header and no rows and says which — a
/// silent empty file would read as "the circuit stayed at zero".
pub(crate) fn export_analog_trace_if_requested<C: labwired_core::Cpu>(
    analog_trace: &Option<PathBuf>,
    machine: &labwired_core::Machine<C>,
) {
    let Some(path) = analog_trace else {
        return;
    };
    let batch = machine.analog_trace_snapshot(0);
    if batch.channels.is_empty() {
        if machine.analog_trace_attached() {
            eprintln!(
                "labwired: --analog-trace {path:?}: this run's co-simulation models record no \
                 waveform (only `adapter: analog` does), so the trace has no channels"
            );
        } else {
            eprintln!(
                "labwired: --analog-trace {path:?}: no co-simulation runner drives this run, so \
                 the trace has no channels"
            );
        }
    }
    match analog_trace::write_analog_trace(&batch, path) {
        Ok(()) => eprintln!(
            "labwired: analog trace ({} channels, {} samples) -> {path:?}",
            batch.channels.len(),
            batch.samples.len()
        ),
        Err(err) => eprintln!("error: cannot write --analog-trace {path:?}: {err}"),
    }
}

fn assertion_short_name(assertion: &TestAssertion) -> String {
    const MAX_LEN: usize = 120;
    let s = match assertion {
        TestAssertion::UartContains(a) => format!("uart_contains: {}", a.uart_contains),
        TestAssertion::RttContains(a) => format!("rtt_contains: {}", a.rtt_contains),
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
                    lit: None,
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

    #[test]
    fn rtt_assertion_passes_only_decides_rtt_contains() {
        let rtt = TestAssertion::RttContains(labwired_config::RttContainsAssertion {
            rtt_contains: "RTT hello".to_owned(),
        });
        assert_eq!(
            rtt_assertion_passes(&rtt, "RTT hello from labwired"),
            Some(true)
        );
        assert_eq!(rtt_assertion_passes(&rtt, "nothing here"), Some(false));

        // A UART assertion is not decided by the RTT stream; it must return
        // `None` so the caller keeps matching against the other stream.
        let uart = TestAssertion::UartContains(labwired_config::UartContainsAssertion {
            uart_contains: "hello".to_owned(),
        });
        assert_eq!(rtt_assertion_passes(&uart, "hello"), None);
    }
}

/// Golden coverage for the single `TestOutcome` (`artifacts::TestResult`)
/// shape that `write_outputs`, `write_config_error_outputs` and
/// `write_junit_xml` all now derive from: a fixed synthetic outcome must keep
/// producing the exact same `result.json` fields and the exact same
/// `junit.xml` text. A change to either snapshot means the on-disk formats
/// moved, which is exactly what this test exists to catch.
#[cfg(test)]
mod test_outcome_golden_tests {
    use super::*;
    use crate::artifacts::TestOutcome;

    fn synthetic_outcome() -> TestOutcome {
        TestOutcome {
            result_schema_version: RESULT_SCHEMA_VERSION.to_string(),
            status: "fail".to_string(),
            steps_executed: 42,
            cycles: 1000,
            instructions: 900,
            stop_reason: StopReason::MaxSteps,
            stop_reason_details: crate::artifacts::StopReasonDetails {
                triggered_stop_condition: StopReason::MaxSteps,
                triggered_limit: Some(crate::artifacts::NamedU64 {
                    name: "max_steps".to_string(),
                    value: 42,
                }),
                observed: None,
            },
            firmware_exit_code: None,
            limits: TestLimits {
                max_steps: 42,
                max_cycles: None,
                max_uart_bytes: None,
                no_progress_steps: None,
                wall_time_ms: None,
                max_vcd_bytes: None,
                stop_when_assertions_pass: false,
                stop_when_assertions_pass_settle_steps: 0,
                stop_when_assertions_pass_min_steps: 0,
            },
            message: None,
            assertions: vec![AssertionResult {
                assertion: TestAssertion::UartContains(labwired_config::UartContainsAssertion {
                    uart_contains: "READY".to_string(),
                }),
                passed: false,
                evidence: None,
            }],
            cpu_state: None,
            firmware_hash: "deadbeef".to_string(),
            config: TestConfig {
                firmware: std::path::PathBuf::from("firmware.elf"),
                system: None,
                script: std::path::PathBuf::from("test.yaml"),
            },
            inspect: None,
            fidelity: Vec::new(),
            logic_edges: None,
            stimuli: Vec::new(),
            footprint: None,
            memory: None,
            metrics: None,
            rtt: None,
        }
    }

    #[test]
    fn golden_result_json_summary() {
        let outcome = synthetic_outcome();
        let json = serde_json::to_value(&outcome).expect("outcome should serialize");
        let expected = serde_json::json!({
            "result_schema_version": "1.0",
            "status": "fail",
            "steps_executed": 42,
            "cycles": 1000,
            "instructions": 900,
            "stop_reason": "max_steps",
            "stop_reason_details": {
                "triggered_stop_condition": "max_steps",
                "triggered_limit": {"name": "max_steps", "value": 42},
                "observed": null
            },
            "limits": {
                "max_steps": 42,
                "max_cycles": null,
                "max_uart_bytes": null,
                "no_progress_steps": null,
                "wall_time_ms": null,
                "max_vcd_bytes": null,
                "stop_when_assertions_pass": false,
                "stop_when_assertions_pass_settle_steps": 0,
                "stop_when_assertions_pass_min_steps": 0
            },
            "assertions": [
                {
                    "assertion": {"uart_contains": "READY"},
                    "passed": false
                }
            ],
            "firmware_hash": "deadbeef",
            "config": {
                "firmware": "firmware.elf",
                "system": null,
                "script": "test.yaml"
            }
        });
        assert_eq!(json, expected);
    }

    #[test]
    fn golden_junit_xml() {
        let outcome = synthetic_outcome();
        let dir = crate::test_support::unique_temp_dir("labwired-junit-golden");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("junit.xml");
        crate::outputs::write_junit_xml(
            &path,
            &outcome.status,
            std::time::Duration::from_secs_f64(1.5),
            &outcome,
        )
        .expect("write_junit_xml should succeed");
        let xml = std::fs::read_to_string(&path).expect("junit.xml should exist");

        assert!(xml.starts_with(r#"<?xml version="1.0" encoding="UTF-8"?>"#));
        assert!(xml.contains(
            r#"<testsuite name="labwired" tests="2" failures="1" errors="0" time="1.500000">"#
        ));
        assert!(xml.contains(r#"<property name="stop_reason" value="MaxSteps"/>"#));
        assert!(xml.contains(r#"<property name="firmware_hash" value="deadbeef"/>"#));
        assert!(xml.contains(r#"<testcase classname="labwired" name="run" time="1.500000">"#));
        assert!(xml.contains("assertion 1: uart_contains"));
        assert!(xml.contains("<failure message=\"assertion failed\">"));

        let _ = std::fs::remove_dir_all(&dir);
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

#[cfg(test)]
mod time_mode_cli {
    use super::RunArgs;
    use clap::Parser;
    use labwired_core::HostTimeMode;

    fn parse(args: &[&str]) -> RunArgs {
        RunArgs::try_parse_from(args).expect("RunArgs should parse")
    }

    #[test]
    fn time_mode_defaults_to_max_speed() {
        let args = parse(&["labwired", "--chip", "c.yaml", "--firmware", "f.elf"]);
        assert_eq!(args.time_mode, HostTimeMode::MaxSpeed);
    }

    #[test]
    fn time_mode_accepts_realtime() {
        let args = parse(&[
            "labwired",
            "--chip",
            "c.yaml",
            "--firmware",
            "f.elf",
            "--time-mode",
            "realtime",
        ]);
        assert_eq!(args.time_mode, HostTimeMode::Realtime);
    }

    #[test]
    fn time_mode_accepts_max_speed() {
        let args = parse(&[
            "labwired",
            "--chip",
            "c.yaml",
            "--firmware",
            "f.elf",
            "--time-mode",
            "max-speed",
        ]);
        assert_eq!(args.time_mode, HostTimeMode::MaxSpeed);
    }

    #[test]
    fn time_mode_rejects_unknown() {
        let err = RunArgs::try_parse_from([
            "labwired",
            "--chip",
            "c.yaml",
            "--firmware",
            "f.elf",
            "--time-mode",
            "turbo",
        ])
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("turbo") || msg.contains("time-mode") || msg.contains("invalid"),
            "unexpected clap error: {msg}"
        );
    }
}
