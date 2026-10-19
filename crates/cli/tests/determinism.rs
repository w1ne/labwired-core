// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;

fn get_labwired_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_labwired"))
}

fn sha256_bytes(b: &[u8]) -> String {
    let hash = Sha256::digest(b);
    format!("{:x}", hash)
}

fn sha256_file(path: &std::path::Path) -> String {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("Failed to read {:?}: {}", path, e));
    sha256_bytes(&data)
}

#[test]
fn test_determinism_smoke() {
    let runs = 5;
    let temp_dir = std::env::temp_dir().join("labwired-determinism-smoke");
    let _ = std::fs::remove_dir_all(&temp_dir); // clean start
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let firmware_path = workspace_root.join("tests/fixtures/uart-ok-thumbv7m.elf");
    let system_path = workspace_root.join("configs/systems/ci-fixture-uart1.yaml");

    if !firmware_path.exists() {
        panic!("Firmware fixture not found at {:?}", firmware_path);
    }
    if !system_path.exists() {
        panic!("System fixture not found at {:?}", system_path);
    }
    let firmware_path = firmware_path.canonicalize().unwrap();
    let system_path = system_path.canonicalize().unwrap();

    let script_content = format!(
        r#"
schema_version: "1.0"
inputs:
  firmware: "{}"
  system: "{}"
limits:
  max_steps: 1000
assertions: []
"#,
        firmware_path.display(),
        system_path.display()
    );

    let script_path = temp_dir.join("script.yaml");
    std::fs::write(&script_path, script_content).unwrap();

    // One run of the same script; `extra` carries the phase-specific flags.
    // Note: UART stdout echo is deliberately left enabled (no
    // `--no-uart-stdout`) so the stdout hashes below cover real payload bytes
    // (`OK\n` from the fixture), not an empty stream.
    let run_once = |output_dir: &std::path::Path, extra: &[&str]| -> std::process::Output {
        Command::new(get_labwired_bin())
            .arg("test")
            .arg("--script")
            .arg(&script_path)
            .arg("--output-dir")
            .arg(output_dir)
            .arg("--trace")
            .args(extra)
            .output()
            .expect("Failed to run labwired")
    };

    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut trace_hashes: Vec<String> = Vec::new();
    let mut stdout_hashes: Vec<String> = Vec::new();

    for i in 0..runs {
        let output_dir = temp_dir.join(format!("run_{}", i));
        let output = run_once(&output_dir, &[]);

        let result_json_path = output_dir.join("result.json");
        if !result_json_path.exists() {
            panic!(
                "Run {} failed to produce result.json. Exit: {:?}\nStdout: {}\nStderr: {}",
                i,
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // Assert passed status because firmware-ci-fixture is valid and shouldn't crash
        assert!(output.status.success(), "Run {} failed exit code", i);

        let result_json_path = output_dir.join("result.json");
        let result_content =
            std::fs::read_to_string(&result_json_path).expect("Failed to read result.json");
        let json: serde_json::Value =
            serde_json::from_str(&result_content).expect("Failed to parse result.json");
        results.push(json);

        // Collect trace.json hash for determinism comparison
        let trace_json_path = output_dir.join("trace.json");
        if trace_json_path.exists() {
            trace_hashes.push(sha256_file(&trace_json_path));
        } else {
            panic!("Run {} failed to produce trace.json (--trace was set)", i);
        }

        // The fixture firmware writes `OK\n` to UART. stdout must carry that
        // echo so the hash below covers real payload bytes; empty or `OK`-less
        // stdout means `--no-uart-stdout` was silently re-added to the helper
        // and the hash would be covering nothing.
        let stdout_text = String::from_utf8_lossy(&output.stdout);
        assert!(
            !output.stdout.is_empty(),
            "Run {} stdout is empty; expected the UART `OK` echo (was --no-uart-stdout re-added?)",
            i
        );
        assert!(
            stdout_text.contains("OK"),
            "Run {} stdout does not contain `OK` (expected the UART echo). stdout: {:?}",
            i,
            stdout_text
        );
        eprintln!(
            "[determinism] run {} stdout ({} bytes): {:?}",
            i,
            output.stdout.len(),
            stdout_text
        );

        stdout_hashes.push(sha256_bytes(&output.stdout));
    }

    // Compare result.json across all runs
    let first = &results[0];
    for (i, current) in results.iter().enumerate().skip(1) {
        assert_eq!(first, current, "Run {} result.json differs from Run 0", i);
    }

    // Compare trace.json hashes across all runs (SHA-256 for efficiency)
    let first_hash = &trace_hashes[0];
    for (i, hash) in trace_hashes.iter().enumerate().skip(1) {
        assert_eq!(
            first_hash, hash,
            "Run {} trace.json hash differs from Run 0 (non-deterministic instruction trace)",
            i
        );
    }

    // Compare stdout hashes across all runs: stdout must be byte-identical
    // run to run (it carries the fixture's UART `OK` echo and any agent
    // payload).
    let first_stdout_hash = &stdout_hashes[0];
    for (i, hash) in stdout_hashes.iter().enumerate().skip(1) {
        assert_eq!(
            first_stdout_hash, hash,
            "Run {} stdout differs from Run 0; stdout must be byte-identical across runs",
            i
        );
    }

    // Pin specific result fields so any regression in determinism is instantly visible.
    let steps = first["steps_executed"]
        .as_u64()
        .expect("steps_executed must be numeric");
    assert_eq!(
        steps, 1000,
        "steps_executed must equal the configured max_steps=1000 for a deterministic run"
    );

    let firmware_hash = first["firmware_hash"].as_str().unwrap_or("");
    assert!(
        !firmware_hash.is_empty(),
        "firmware_hash must be non-empty in result.json"
    );
    // Hash must be a 64-char hex SHA-256.
    assert_eq!(
        firmware_hash.len(),
        64,
        "firmware_hash should be a 64-char hex SHA-256, got: {firmware_hash:?}"
    );
    eprintln!("[determinism] steps_executed={steps}, firmware_hash={firmware_hash}");

    // ── Phase 2: `--log-timestamps virtual` ────────────────────────────────
    //
    // The flag prefixes human-readable stderr lines only: the instruction
    // trace must stay deterministic under it, and stdout must not move at all
    // (the prefixes go to stderr). Three runs are enough to catch a per-run
    // clock or ordering leak.
    let virtual_runs = 3;
    let mut virtual_trace_hashes: Vec<String> = Vec::new();
    let mut virtual_stdout_hashes: Vec<String> = Vec::new();
    for i in 0..virtual_runs {
        let output_dir = temp_dir.join(format!("virtual_{}", i));
        let output = run_once(&output_dir, &["--log-timestamps", "virtual"]);
        assert!(
            output.status.success(),
            "Virtual run {} failed. Exit: {:?}\nStdout: {}\nStderr: {}",
            i,
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let trace_json_path = output_dir.join("trace.json");
        if !trace_json_path.exists() {
            panic!(
                "Virtual run {} failed to produce trace.json (--trace was set)",
                i
            );
        }

        // Same guard as the default phase: stdout must carry the fixture's
        // UART `OK` echo.
        let virtual_stdout_text = String::from_utf8_lossy(&output.stdout);
        assert!(
            !output.stdout.is_empty(),
            "Virtual run {} stdout is empty; expected the UART `OK` echo (was --no-uart-stdout re-added?)",
            i
        );
        assert!(
            virtual_stdout_text.contains("OK"),
            "Virtual run {} stdout does not contain `OK` (expected the UART echo). stdout: {:?}",
            i,
            virtual_stdout_text
        );
        eprintln!(
            "[determinism] virtual run {} stdout ({} bytes): {:?}",
            i,
            output.stdout.len(),
            virtual_stdout_text
        );

        virtual_trace_hashes.push(sha256_file(&trace_json_path));
        virtual_stdout_hashes.push(sha256_bytes(&output.stdout));
    }

    let first_virtual_trace_hash = &virtual_trace_hashes[0];
    for (i, hash) in virtual_trace_hashes.iter().enumerate().skip(1) {
        assert_eq!(
            first_virtual_trace_hash, hash,
            "Virtual run {} trace.json hash differs from Virtual Run 0 (non-deterministic instruction trace)",
            i
        );
    }

    let first_virtual_stdout_hash = &virtual_stdout_hashes[0];
    for (i, hash) in virtual_stdout_hashes.iter().enumerate().skip(1) {
        assert_eq!(
            first_virtual_stdout_hash, hash,
            "Virtual run {} stdout differs from Virtual Run 0; stdout must be byte-identical across runs",
            i
        );
    }

    assert_eq!(
        first_stdout_hash, first_virtual_stdout_hash,
        "`--log-timestamps virtual` must not move stdout: the `[t=...]` prefixes go to stderr only"
    );
}
