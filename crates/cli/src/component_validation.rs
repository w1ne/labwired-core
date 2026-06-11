// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `labwired asset validate-component <spec.yaml> [--json]`
//!
//! Validates an IrComponent spec and prints diagnostics. Exit code 0 when
//! clean, 1 when the file is unreadable/unparsable or has diagnostics.
//! `--json` emits `{ "ok": bool, "name": string|null, "diagnostics": [...] }`
//! on stdout for the MCP server.

use clap::Args;
use labwired_ir::component::{IrComponent, IrComponentDiag};
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct ValidateComponentArgs {
    /// Path to the component spec YAML.
    pub spec: std::path::PathBuf,
    /// Emit machine-readable JSON on stdout.
    #[arg(long)]
    pub json: bool,
}

#[derive(serde::Serialize)]
struct JsonReport {
    ok: bool,
    name: Option<String>,
    diagnostics: Vec<IrComponentDiag>,
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
        diagnostics: vec![IrComponentDiag {
            code: code.into(),
            message,
            hint: "Check the file path and YAML syntax".into(),
        }],
    };
    let yaml = match std::fs::read_to_string(path) {
        Ok(y) => y,
        Err(e) => {
            return (
                io_diag("ICOMP_READ_ERROR", e.to_string()),
                ExitCode::from(1),
            )
        }
    };
    let spec: IrComponent = match serde_yaml::from_str(&yaml) {
        Ok(s) => s,
        Err(e) => {
            return (
                io_diag("ICOMP_PARSE_ERROR", e.to_string()),
                ExitCode::from(1),
            )
        }
    };
    let diagnostics = spec.validate();
    let ok = diagnostics.is_empty();
    (
        JsonReport {
            ok,
            name: Some(spec.name),
            diagnostics,
        },
        if ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_spec_reports_ok() {
        let (r, _) = build_report(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../configs/components/pca9685.yaml"
        )));
        assert!(r.ok);
        assert_eq!(r.name.as_deref(), Some("PCA9685"));
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn missing_file_reports_read_error() {
        let (r, _) = build_report(std::path::Path::new("/nonexistent/spec.yaml"));
        assert!(!r.ok);
        assert_eq!(r.diagnostics[0].code, "ICOMP_READ_ERROR");
    }

    #[test]
    fn invalid_spec_reports_diagnostics() {
        let dir = std::env::temp_dir().join("labwired_icomp_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bad.yaml");
        std::fs::write(
            &p,
            "name: Bad\nkind: wasm\ninterface: { i2c: { default_address: 0x40 } }\nregister_file: { size: 256 }\n",
        )
        .unwrap();
        let (r, _) = build_report(&p);
        assert!(!r.ok);
        assert!(r
            .diagnostics
            .iter()
            .any(|d| d.code == "ICOMP_WASM_UNSUPPORTED"));
    }
}
