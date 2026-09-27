// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired fault-inject`: run a scheduled-fault experiment and print the
//! lockstep verdict as JSON.
//!
//! The firmware is built twice from the same inputs, a golden and a faulted
//! copy. The plan's faults fire on the faulted copy at their cycles and the
//! two step in lockstep, compared after every instruction. See
//! `labwired_core::vfi::lockstep` for the report and what each verdict means.

use clap::Args;
use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::{
    BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
};
use labwired_core::vfi::{FaultPlan, ScheduledFault};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct FaultInjectArgs {
    /// Firmware ELF.
    #[arg(short = 'f', long)]
    pub firmware: PathBuf,

    /// System manifest (its `chip:` is a built-in name or a path).
    #[arg(short = 's', long)]
    pub system: PathBuf,

    /// The plan as a JSON file: `{"until_cycle": N, "faults": [...]}`.
    #[arg(long, conflicts_with_all = ["faults_json", "until_cycle"])]
    pub plan: Option<PathBuf>,

    /// The faults as a JSON array, e.g.
    /// `[{"at_cycle":500,"kind":"register_bit_flip","register":"R0","bit":3}]`.
    /// Kinds: register_bit_flip (register, bit), memory_bit_flip (address,
    /// bit), instruction_skip.
    #[arg(long, requires = "until_cycle")]
    pub faults_json: Option<String>,

    /// Stop when the golden machine reaches this cycle.
    #[arg(long)]
    pub until_cycle: Option<u64>,

    /// Also write the report to `<dir>/fault-report.json`.
    #[arg(long)]
    pub output_dir: Option<PathBuf>,
}

fn load_plan(args: &FaultInjectArgs) -> Result<FaultPlan, String> {
    if let Some(path) = &args.plan {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        return serde_json::from_str(&text).map_err(|e| format!("plan {}: {e}", path.display()));
    }
    let faults: Vec<ScheduledFault> = serde_json::from_str(
        args.faults_json
            .as_deref()
            .ok_or("pass --plan, or --faults-json with --until-cycle")?,
    )
    .map_err(|e| format!("--faults-json: {e}"))?;
    Ok(FaultPlan {
        faults,
        until_cycle: args.until_cycle.unwrap_or(0),
    })
}

fn open(
    args: &FaultInjectArgs,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> Result<Session, String> {
    let mut manifest = labwired_config::SystemManifest::from_file(&args.system)
        .map_err(|e| format!("system {}: {e:#}", args.system.display()))?;
    let chip_dir = args
        .system
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let chip = labwired_config::ChipDescriptor::resolve_with(
        &manifest.chip,
        chip_dir,
        &crate::plugin_chip_yaml(plugins),
    )
    .map_err(|e| format!("chip '{}': {e:#}", manifest.chip))?;
    if !labwired_config::is_builtin_chip_spec(&manifest.chip) {
        manifest.chip = chip_dir.join(&manifest.chip).to_string_lossy().into_owned();
    }
    let firmware = std::fs::read(&args.firmware)
        .map_err(|e| format!("firmware {}: {e}", args.firmware.display()))?;
    Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(&firmware),
            boot: BootMode::FastBoot,
            blobs: &BlobMap::new(),
            options: BuildOptions {
                uart_rx: manifest.debug_uart.clone(),
                ..Default::default()
            },
        },
        OpenOptions::default(),
    )
    .map_err(|e| format!("building the machine: {e:#}"))
}

pub fn run_fault_inject(
    args: FaultInjectArgs,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> ExitCode {
    let result = load_plan(&args).and_then(|plan| {
        let session = open(&args, plugins)?;
        session.fault_experiment(&plan).map_err(|e| format!("{e}"))
    });
    let report = match result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("fault-inject: {e}");
            return ExitCode::from(2);
        }
    };
    let json = serde_json::to_string_pretty(&report).expect("a report serialises");
    if let Some(dir) = &args.output_dir {
        if let Err(e) = std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(dir.join("fault-report.json"), &json))
        {
            eprintln!("fault-inject: writing {}: {e}", dir.display());
            return ExitCode::from(2);
        }
    }
    println!("{json}");
    ExitCode::SUCCESS
}
