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
use labwired_core::Cpu;
use std::sync::{Arc, Mutex};
use tracing::{error, info};

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

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Deterministic, CI-friendly runner mode driven by a test script (YAML).
    Test(TestArgs),

    /// Machine control operations (load, etc.)
    Machine(MachineArgs),
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
        None => run_interactive(cli),
    }
}

fn run_interactive(cli: Cli) -> ExitCode {
    info!("Starting LabWired Simulator");

    let Some(firmware) = &cli.firmware else {
        tracing::error!("Missing required --firmware argument");
        return ExitCode::from(EXIT_CONFIG_ERROR);
    };

    let system_path = cli.system.clone();
    let bus = match build_bus(system_path.clone()) {
        Ok(bus) => bus,
        Err(e) => {
            tracing::error!("{:#}", e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    info!("Loading firmware: {:?}", firmware);
    let program = match labwired_loader::load_elf(firmware) {
        Ok(program) => program,
        Err(e) => {
            tracing::error!("{:#}", e);
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
                        tracing::error!("Failed to parse chip descriptor: {:#}", e);
                        return ExitCode::from(EXIT_CONFIG_ERROR);
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to parse system manifest: {:#}", e);
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
        _ => {
            error!("Unsupported architecture: {:?}", cpu_arch);
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
    let mut bus = match build_bus(config.system.clone()) {
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

fn run_interactive_arm(
    cli: Cli,
    mut bus: labwired_core::bus::SystemBus,
    program: labwired_core::memory::ProgramImage,
    metrics: Arc<labwired_core::metrics::PerformanceMetrics>,
) -> ExitCode {
    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = labwired_core::Machine::new(cpu, bus);
    machine.observers.push(metrics.clone());

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

    report_metrics(&machine.cpu, &metrics);
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

    report_metrics(&machine.cpu, &metrics);
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
    cpu: &C,
    metrics: &labwired_core::metrics::PerformanceMetrics,
) {
    info!("Simulation loop finished.");
    info!("Final PC: {:#x}", cpu.get_pc());
    info!("Total Instructions: {}", metrics.get_instructions());
    info!("Total Cycles: {}", metrics.get_cycles());
    info!("Average IPS: {:.2}", metrics.get_ips());
}

fn build_stop_reason_details(
    stop_reason: &StopReason,
    limits: &TestLimits,
    steps_executed: u64,
    cycles: u64,
    uart_bytes: u64,
    stuck_steps: u64,
    duration: std::time::Duration,
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
        StopReason::MemoryViolation
        | StopReason::DecodeError
        | StopReason::Halt
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
                script.assertions,
            )
        }
    };

    let max_steps = args.max_steps.unwrap_or(script_max_steps);
    let max_cycles = args.max_cycles.or(script_max_cycles);
    let max_uart_bytes = args.max_uart_bytes.or(script_max_uart_bytes);
    let detect_stuck = args.detect_stuck.or(script_no_progress_steps);
    let resolved_limits = TestLimits {
        max_steps,
        max_cycles,
        max_uart_bytes,
        no_progress_steps: detect_stuck,
        wall_time_ms: script_wall_time_ms,
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

    let mut bus = match build_bus(system_path.clone()) {
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
    let (_cpu_configured, machine_arm, machine_riscv) = match program.arch {
        labwired_core::Arch::Arm => {
            let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
            let mut machine = labwired_core::Machine::new(cpu, bus);
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
            (true, Some(machine), None)
        }
        labwired_core::Arch::RiscV => {
            let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
            let mut machine = labwired_core::Machine::new(cpu, bus);
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
            (true, None, Some(machine))
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
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    if let Some(mut machine) = machine_arm {
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
    } else if let Some(mut machine) = machine_riscv {
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
    } else {
        unreachable!()
    }
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
    let mut sim_error_happened = false;
    let mut prev_pc = machine.cpu.get_pc();
    let mut stuck_counter: u64 = 0;

    for step in 0..max_steps {
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

        steps_executed = step + 1;
        if let Err(e) = machine.step() {
            sim_error_happened = true;
            stop_reason = match e {
                labwired_core::SimulationError::MemoryViolation(_) => StopReason::MemoryViolation,
                labwired_core::SimulationError::DecodeError(_) => StopReason::DecodeError,
            };
            error!("Simulation error at step {}: {}", step, e);
            break;
        }

        // Check no_progress (PC stuck)
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

fn build_bus(system_path: Option<PathBuf>) -> anyhow::Result<labwired_core::bus::SystemBus> {
    let bus = if let Some(sys_path) = system_path {
        info!("Loading system manifest: {:?}", sys_path);
        let manifest = labwired_config::SystemManifest::from_file(&sys_path)?;
        let chip_path = sys_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(&manifest.chip);
        info!("Loading chip descriptor: {:?}", chip_path);
        let chip = labwired_config::ChipDescriptor::from_file(&chip_path)?;
        labwired_core::bus::SystemBus::from_config(&chip, &manifest)?
    } else {
        info!("Using default hardware configuration");
        labwired_core::bus::SystemBus::new()
    };

    Ok(bus)
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
