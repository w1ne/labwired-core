// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::*;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "LabWired Simulator",
    long_about = None,
    subcommand_negates_reqs = true
)]
pub(crate) struct Cli {
    /// Path to the firmware ELF file
    #[arg(short, long)]
    pub(crate) firmware: Option<PathBuf>,

    /// Path to the system manifest (YAML)
    #[arg(short, long)]
    pub(crate) system: Option<PathBuf>,

    /// Write a state snapshot (JSON) for interactive runs.
    #[arg(long)]
    pub(crate) snapshot: Option<PathBuf>,

    /// Breakpoint PC address (repeatable). Stops simulation when PC matches.
    #[arg(long, value_parser = parse_u32_addr)]
    pub(crate) breakpoint: Vec<u32>,

    /// Enable instruction-level execution tracing
    #[arg(short, long, global = true)]
    pub(crate) trace: bool,

    /// Maximum number of steps to execute (default: 20000)
    #[arg(long, default_value = "20000")]
    pub(crate) max_steps: usize,

    /// Start a GDB server on the specified port
    #[arg(long)]
    pub(crate) gdb: Option<u16>,

    /// Output errors and diagnostics as structured JSON for agent consumption
    #[arg(long, global = true)]
    pub(crate) json: bool,

    /// Output VCD trace to file
    #[arg(long, global = true)]
    pub(crate) vcd: Option<PathBuf>,

    /// Emit SEGGER RTT output: interactive runs echo drained RTT bytes to
    /// stdout; `test` writes rtt.log and enables `rtt_contains` assertions.
    #[arg(long, global = true)]
    pub(crate) rtt: bool,

    /// Emit ARM semihosting output: interactive runs echo drained bytes to
    /// stdout; `test` writes semihosting.log and enables `semihosting_contains`.
    #[arg(long, global = true)]
    pub(crate) semihosting: bool,

    /// Emit ITM stimulus port 0: interactive runs echo the byte stream to
    /// stdout; `test` writes itm.log and enables `itm_contains` assertions.
    #[arg(long, global = true)]
    pub(crate) itm: bool,

    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Commands {
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
        Some(Commands::Test(args)) => {
            commands::test::run_test(args, plugins, cli.rtt, cli.semihosting, cli.itm)
        }
        Some(Commands::Machine(args)) => run_machine(args, plugins),
        Some(Commands::Asset(args)) => run_asset(args, plugins),
        Some(Commands::Run(args)) => commands::run::run_firmware(args, plugins, cli.json),
        Some(Commands::Snapshot(args)) => commands::snapshot::run_snapshot(args, plugins),
        Some(Commands::Coverage(args)) => commands::coverage::run_coverage(args),
        Some(Commands::Tier1Matrix(args)) => commands::tier1::run_tier1_matrix(args),
        Some(Commands::CosimStep(args)) => commands::cosim::run_cosim_step(args),
        Some(Commands::Fuzz(args)) => commands::fuzz::run_fuzz(args),
        Some(Commands::DebugProbe(args)) => commands::debug_probe::run(args, plugins),
        None => commands::run::run_interactive(cli, plugins),
    }
}
