//! `labwired cosim-step`: drive a manifest-declared co-simulation model
//! through the real runner/adapter chain and print the routed outputs.
//!
//! This is the command-line proof that a co-sim model runs *through*
//! LabWired's manifest routing, not just next to it:
//!
//! ```text
//! labwired cosim-step examples/cosim-plant-demo/system.yaml \
//!     --set plant.channels.11.enabled=false --json
//! ```

use clap::Args;
use labwired_config::SystemManifest;
use labwired_core::cosim::{CosimRunner, CosimSignalValue, CosimSignals};
use std::path::PathBuf;
use std::process::ExitCode;

use crate::{EXIT_CONFIG_ERROR, EXIT_RUNTIME_ERROR};

#[derive(Args, Debug)]
pub struct CosimStepArgs {
    /// Path to the system manifest declaring `cosim_models`.
    pub system: PathBuf,

    /// Seed a signal-store path before stepping, e.g.
    /// `--set plant.channels.11.enabled=false`. Values parse as bool,
    /// integer, then float, falling back to text. Repeatable.
    #[arg(long = "set", value_name = "PATH=VALUE")]
    pub sets: Vec<String>,

    /// Number of model steps to advance (time advances to
    /// `steps * max(step_ns)`).
    #[arg(long, default_value_t = 1)]
    pub steps: u64,

    /// Print routed outputs as JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

fn parse_signal_value(raw: &str) -> CosimSignalValue {
    match raw {
        "true" => CosimSignalValue::Bool(true),
        "false" => CosimSignalValue::Bool(false),
        _ => {
            if let Ok(value) = raw.parse::<i64>() {
                CosimSignalValue::I64(value)
            } else if let Ok(value) = raw.parse::<f64>() {
                CosimSignalValue::F64(value)
            } else {
                CosimSignalValue::Text(raw.to_string())
            }
        }
    }
}

fn signal_to_display(value: &CosimSignalValue) -> String {
    match value {
        CosimSignalValue::Bool(value) => value.to_string(),
        CosimSignalValue::I64(value) => value.to_string(),
        CosimSignalValue::F64(value) => format!("{value}"),
        CosimSignalValue::Text(value) => value.clone(),
    }
}

fn signal_to_json(value: &CosimSignalValue) -> serde_json::Value {
    match value {
        CosimSignalValue::Bool(value) => serde_json::Value::Bool(*value),
        CosimSignalValue::I64(value) => serde_json::json!(value),
        CosimSignalValue::F64(value) => serde_json::json!(value),
        CosimSignalValue::Text(value) => serde_json::Value::String(value.clone()),
    }
}

pub fn run_cosim_step(args: CosimStepArgs) -> ExitCode {
    let manifest = match SystemManifest::from_file(&args.system) {
        Ok(manifest) => manifest,
        Err(err) => {
            eprintln!(
                "error: failed to load manifest {}: {err:#}",
                args.system.display()
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let issues = manifest.validate_cosim_models();
    if !issues.is_empty() {
        for issue in issues {
            eprintln!("error: {issue}");
        }
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }
    if manifest.cosim_models.is_empty() {
        eprintln!(
            "error: manifest {} declares no cosim_models",
            args.system.display()
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let base_dir = args
        .system
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut runner = match CosimRunner::from_configs_with_base(&manifest.cosim_models, &base_dir) {
        Ok(runner) => runner,
        Err(err) => {
            eprintln!("error: failed to build co-sim runner: {err}");
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }
    };

    let mut signals = CosimSignals::new();
    for set in &args.sets {
        let Some((path, raw)) = set.split_once('=') else {
            eprintln!("error: --set expects PATH=VALUE, got '{set}'");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        };
        signals.insert(path.trim().to_string(), parse_signal_value(raw.trim()));
    }

    let max_step_ns = manifest
        .cosim_models
        .iter()
        .map(|model| model.step_ns)
        .max()
        .unwrap_or(1);
    let target_ns = args.steps.saturating_mul(max_step_ns);

    let routed = match runner.step_until_with_signals(target_ns, &mut signals) {
        Ok(routed) => routed,
        Err(err) => {
            eprintln!("error: co-sim step failed: {err}");
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }
    };

    if args.json {
        let steps: Vec<serde_json::Value> = routed
            .iter()
            .map(|step| {
                serde_json::json!({
                    "model": step.model_id,
                    "outputs": step
                        .outputs
                        .iter()
                        .map(|(path, value)| (path.clone(), signal_to_json(value)))
                        .collect::<serde_json::Map<String, serde_json::Value>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&steps).unwrap());
    } else {
        for step in &routed {
            println!("model '{}' routed outputs:", step.model_id);
            for (path, value) in &step.outputs {
                println!("  {path} = {}", signal_to_display(value));
            }
        }
        if routed.is_empty() {
            println!("no model reached its step boundary (try --steps > 0)");
        }
    }

    ExitCode::SUCCESS
}
