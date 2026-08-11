// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// End-to-end coverage for P0 resource metrics via examples/metrics/stm32f103-blinky.
//
// Exercises the path an author (or CI gate) runs:
//
//   labwired test --script examples/metrics/stm32f103-blinky/test-pass.yaml
//   labwired test --script examples/metrics/stm32f103-blinky/test-fail-stack.yaml
//
// Pass asserts ELF footprint totals + stack paint method; fail asserts a
// resource_budget failure with limit evidence for max_main_stack_bytes: 1.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

struct MetricsRun {
    exit_code: Option<i32>,
    stderr: String,
    result: serde_json::Value,
}

/// Run a metrics scenario script (relative to the repo root) through
/// `labwired test` and return exit code + parsed result.json.
fn run_metrics(script_rel: &str) -> MetricsRun {
    let root = repo_root();
    let out_dir = labwired_cli::test_support::unique_temp_dir("labwired-resource-metrics");
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .args([
            "test",
            "--script",
            script_rel,
            "--no-uart-stdout",
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute labwired");

    let result_path = out_dir.join("result.json");
    if !result_path.exists() {
        let _ = std::fs::remove_dir_all(&out_dir);
        panic!(
            "result.json missing after running {script_rel}\nexit: {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    let result_json = std::fs::read_to_string(&result_path).expect("read result.json");
    let result: serde_json::Value = serde_json::from_str(&result_json).expect("parse result.json");

    let run = MetricsRun {
        exit_code: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        result,
    };

    let _ = std::fs::remove_dir_all(&out_dir);
    run
}

#[test]
fn resource_metrics_pass_footprint_and_stack_paint() {
    let run = run_metrics("examples/metrics/stm32f103-blinky/test-pass.yaml");

    assert_eq!(
        run.exit_code,
        Some(0),
        "expected exit 0 (assertions pass); stderr: {}",
        run.stderr
    );
    assert_eq!(
        run.result["status"].as_str().unwrap_or(""),
        "pass",
        "expected result.json status=pass; result: {}",
        run.result
    );

    // Berkeley-style ELF section totals for tests/fixtures/stm32f103-blinky.elf
    // (locked by crates/loader footprint unit tests).
    let footprint = &run.result["footprint"];
    assert_eq!(
        footprint["text_bytes"].as_u64(),
        Some(12760),
        "footprint.text_bytes; footprint: {footprint}"
    );
    assert_eq!(
        footprint["data_bytes"].as_u64(),
        Some(124),
        "footprint.data_bytes; footprint: {footprint}"
    );
    assert_eq!(
        footprint["bss_bytes"].as_u64(),
        Some(2548),
        "footprint.bss_bytes; footprint: {footprint}"
    );

    let method = run.result["memory"]["main_stack_method"]
        .as_str()
        .unwrap_or("");
    assert_eq!(
        method, "paint",
        "expected memory.main_stack_method == paint; memory: {}",
        run.result["memory"]
    );
}

#[test]
fn resource_metrics_fail_stack_budget_evidence() {
    let run = run_metrics("examples/metrics/stm32f103-blinky/test-fail-stack.yaml");

    assert_ne!(
        run.exit_code,
        Some(0),
        "expected non-zero exit (stack budget fail); stderr: {}",
        run.stderr
    );
    assert_eq!(
        run.result["status"].as_str().unwrap_or(""),
        "fail",
        "expected result.json status=fail; result: {}",
        run.result
    );

    let assertions = run.result["assertions"]
        .as_array()
        .expect("result.assertions array");

    let failed_stack = assertions.iter().find(|a| {
        a["passed"] == false
            && a["evidence"]["type"] == "resource_budget"
            && a["evidence"]["name"] == "max_main_stack_bytes"
    });

    let failed = failed_stack.unwrap_or_else(|| {
        panic!(
            "expected a failed resource_budget max_main_stack_bytes assertion; assertions: {assertions:?}"
        )
    });

    assert_eq!(
        failed["evidence"]["limit"].as_u64(),
        Some(1),
        "expected evidence.limit == 1; evidence: {}",
        failed["evidence"]
    );
    // Script sets max_main_stack_bytes: 1 under resource_budget.
    assert_eq!(
        failed["assertion"]["resource_budget"]["max_main_stack_bytes"].as_u64(),
        Some(1),
        "expected assertion resource_budget max_main_stack_bytes == 1; assertion: {}",
        failed["assertion"]
    );
}
