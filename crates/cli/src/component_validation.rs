// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `labwired asset validate-component <spec.yaml> [--json]`
//!
//! Shape-validates a declarative device descriptor (a `configs/devices/*.yaml`
//! entry, the modern single declarative stack — see
//! `docs/specs/declarative_i2c_devices.md`). Exit code 0 when the file parses
//! as a [`DeviceDescriptor`], 1 when it is unreadable or malformed. `--json`
//! emits `{ "ok": bool, "name": string|null, "diagnostics": [...] }` on stdout
//! for the MCP server.

use clap::Args;
use labwired_config::DeviceDescriptor;
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct ValidateComponentArgs {
    /// Path to the device descriptor YAML.
    pub spec: std::path::PathBuf,
    /// Emit machine-readable JSON on stdout.
    #[arg(long)]
    pub json: bool,
}

/// One validation diagnostic. Kept flat so the MCP server can render it.
#[derive(serde::Serialize)]
struct Diag {
    code: String,
    message: String,
    hint: String,
}

#[derive(serde::Serialize)]
struct JsonReport {
    ok: bool,
    name: Option<String>,
    diagnostics: Vec<Diag>,
}

pub fn run_validate_component(args: ValidateComponentArgs) -> ExitCode {
    let (report, code) = build_report(&args.spec);
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("serialize")
        );
    } else if report.ok {
        println!("OK: {}", report.name.as_deref().unwrap_or("?"));
    } else {
        for d in &report.diagnostics {
            eprintln!("{}: {} (hint: {})", d.code, d.message, d.hint);
        }
    }
    code
}

fn build_report(path: &std::path::Path) -> (JsonReport, ExitCode) {
    let io_diag = |code: &str, message: String| JsonReport {
        ok: false,
        name: None,
        diagnostics: vec![Diag {
            code: code.into(),
            message,
            hint: "Check the file path and YAML syntax".into(),
        }],
    };
    let yaml = match std::fs::read_to_string(path) {
        Ok(y) => y,
        Err(e) => {
            return (
                io_diag("DEVICE_READ_ERROR", e.to_string()),
                ExitCode::from(1),
            )
        }
    };
    let descriptor = match DeviceDescriptor::from_yaml(&yaml) {
        Ok(d) => d,
        Err(e) => {
            return (
                io_diag("DEVICE_PARSE_ERROR", format!("{e:#}")),
                ExitCode::from(1),
            )
        }
    };
    (
        JsonReport {
            ok: true,
            name: Some(descriptor.r#type),
            diagnostics: Vec::new(),
        },
        ExitCode::SUCCESS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_descriptor_reports_ok() {
        let (r, _) = build_report(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../configs/devices/pca9685.yaml"
        )));
        assert!(r.ok);
        assert_eq!(r.name.as_deref(), Some("pca9685"));
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn missing_file_reports_read_error() {
        let (r, _) = build_report(std::path::Path::new("/nonexistent/spec.yaml"));
        assert!(!r.ok);
        assert_eq!(r.diagnostics[0].code, "DEVICE_READ_ERROR");
    }

    #[test]
    fn malformed_descriptor_reports_parse_error() {
        let dir = std::env::temp_dir().join("labwired_devdesc_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bad.yaml");
        // `behavior:` is required and typed; a scalar there fails to parse.
        std::fs::write(&p, "type: Bad\nbehavior: not-a-behavior-map\n").unwrap();
        let (r, _) = build_report(&p);
        assert!(!r.ok);
        assert_eq!(r.diagnostics[0].code, "DEVICE_PARSE_ERROR");
    }
}
