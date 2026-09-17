// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// End-to-end coverage for the SEGGER RTT stream through `labwired test`:
// `--rtt` (or an `rtt_contains` assertion) enables capture, `rtt.log` lands
// beside `uart.log`, and `result.json`'s `rtt` block reports whether the
// control block was found and how many bytes were drained. The firmware is the
// stock-SEGGER_RTT.c `firmware-nrf52840-rtt`, which prints
// "RTT hello from labwired" once.
//
// This is what keeps the CLI's RTT wiring honest end to end: a regression in
// the `--rtt` flag, the `rtt_contains` assertion plumbing, the in-loop RTT
// evaluation under `stop_when_assertions_pass`, or the `rtt.log`/result.json
// artifact writing fails the merge gate here.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

/// Build `firmware-nrf52840-rtt` for thumbv7em-none-eabi. Mirrors
/// `crates/core/tests/e2e_segger_rtt.rs::ensure_firmware_built`: the build runs
/// unconditionally — a stale binary from an older `SEGGER_RTT.c` must not be
/// able to satisfy this test — and scrubs inherited coverage-instrumentation
/// flags, which the no_std cross-build would fail to link under (E0463).
fn ensure_firmware_built(root: &std::path::Path) -> PathBuf {
    let bin = labwired_core::test_support::target_dir()
        .join("thumbv7em-none-eabi/release/firmware-nrf52840-rtt");
    let status = Command::new("cargo")
        .current_dir(root)
        .args([
            "build",
            "-p",
            "firmware-nrf52840-rtt",
            "--release",
            "--target",
            "thumbv7em-none-eabi",
        ])
        // See e2e_epaper_tricolor: clear coverage instrumentation flags so the
        // no_std firmware cross-build doesn't fail with E0463 under llvm-cov.
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .status()
        .expect("execute cargo build");
    assert!(
        status.success(),
        "failed to build firmware-nrf52840-rtt (needs gcc-arm-none-eabi)"
    );
    assert!(bin.exists(), "expected binary at {:?}", bin);
    bin
}

struct RttRun {
    exit_code: Option<i32>,
    /// `result.json`'s `status`, copied out for readable assertions.
    status: String,
    /// Contents of `rtt.log`.
    rtt: String,
    /// Child stdout, kept for failure messages.
    stdout: String,
    /// Parsed `result.json` (`Null` when the run never wrote one).
    result: serde_json::Value,
}

/// Run `labwired test` on a schema-1.0 script composed from `yaml_body` — the
/// `limits:` + `assertions:` block — with absolute `inputs.firmware`/system
/// paths, then return `rtt.log` and `result.json`.
fn run_script(root: &std::path::Path, fw: &std::path::Path, yaml_body: &str) -> RttRun {
    run_script_args(root, fw, yaml_body, &[])
}

/// [`run_script`] with extra argv injected before the `test` subcommand (the
/// `--rtt` flag is `global = true`, so this placement is stable).
fn run_script_args(
    root: &std::path::Path,
    fw: &std::path::Path,
    yaml_body: &str,
    extra_args: &[&str],
) -> RttRun {
    let system = root.join("examples/nrf52840-rtt-lab/system.yaml");
    let script_yaml = format!(
        r#"schema_version: "1.0"
inputs:
  firmware: "{}"
  system: "{}"
{}"#,
        fw.display(),
        system.display(),
        yaml_body
    );

    let out_dir = labwired_cli::test_support::unique_temp_dir("labwired-rtt-e2e");
    std::fs::create_dir_all(&out_dir).expect("create out dir");
    let script_path = out_dir.join("script.yaml");
    std::fs::write(&script_path, script_yaml).expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(root)
        .args(extra_args)
        .args([
            "test",
            "--script",
            script_path.to_str().unwrap(),
            "--no-uart-stdout",
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute labwired");

    let rtt = std::fs::read_to_string(out_dir.join("rtt.log")).unwrap_or_default();
    let result = std::fs::read_to_string(out_dir.join("result.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or(serde_json::Value::Null);
    let status = result["status"].as_str().unwrap_or_default().to_string();

    let _ = std::fs::remove_dir_all(&out_dir);
    RttRun {
        exit_code: output.status.code(),
        status,
        rtt,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        result,
    }
}

#[test]
fn rtt_banner_is_captured_and_assertion_passes() {
    let root = repo_root();
    let fw = ensure_firmware_built(&root);
    let run = run_script(
        &root,
        &fw,
        r#"limits:
  max_steps: 200000
assertions:
  - rtt_contains: "RTT hello from labwired"
  - expected_stop_reason: max_steps
"#,
    );
    assert_eq!(
        run.exit_code,
        Some(0),
        "expected exit 0 (rtt_contains passes); stdout: {}\nrtt.log: {}",
        run.stdout,
        run.rtt
    );
    assert_eq!(
        run.status, "pass",
        "expected status pass; stdout: {}\nrtt.log: {}",
        run.stdout, run.rtt
    );
    assert!(
        run.rtt.contains("RTT hello from labwired"),
        "expected the banner in rtt.log\n--- rtt.log ---\n{}",
        run.rtt
    );
    assert_eq!(
        run.result["rtt"]["control_block_found"].as_bool(),
        Some(true),
        "expected result.json to report the RTT control block as found\n{}",
        run.result
    );
    assert!(
        run.result["rtt"]["bytes_drained"].as_u64().unwrap_or(0) > 0,
        "expected bytes_drained > 0\n{}",
        run.result
    );
}

/// Discriminating half of the above: a token that is NEVER printed by the
/// firmware must fail the exact same assertion the banner passes — proves the
/// RTT capture is real and the assertion is not vacuously true.
#[test]
fn wrong_rtt_assertion_fails() {
    let root = repo_root();
    let fw = ensure_firmware_built(&root);
    let run = run_script(
        &root,
        &fw,
        r#"limits:
  max_steps: 200000
assertions:
  - rtt_contains: "this string is never printed"
  - expected_stop_reason: max_steps
"#,
    );
    assert_eq!(
        run.exit_code,
        Some(1),
        "expected exit 1 (rtt_contains 'this string is never printed' fails); \
         stdout: {}\nrtt.log: {}",
        run.stdout,
        run.rtt
    );
    assert_eq!(
        run.status, "fail",
        "expected status fail; stdout: {}\nrtt.log: {}",
        run.stdout, run.rtt
    );
    assert!(
        run.rtt.contains("RTT hello from labwired"),
        "the capture must still contain the real banner — the failure has to \
         come from the assertion, not from RTT being off\n--- rtt.log ---\n{}",
        run.rtt
    );
}

/// The review-flagged branch: with an `rtt_contains` present the UART-only
/// assertion cache is disabled and RTT is re-read in-loop, so
/// `stop_when_assertions_pass` must still see the token pass and stop the run
/// early with `stop_reason: assertions_passed` (and a pass verdict).
#[test]
fn rtt_assertion_stops_early_without_uart_cache() {
    let root = repo_root();
    let fw = ensure_firmware_built(&root);
    let run = run_script(
        &root,
        &fw,
        r#"limits:
  max_steps: 200000
  stop_when_assertions_pass: true
assertions:
  - rtt_contains: "RTT hello from labwired"
  - expected_stop_reason: assertions_passed
"#,
    );
    assert_eq!(
        run.exit_code,
        Some(0),
        "expected exit 0 (assertions pass, then early-stop); stdout: {}\nrtt.log: {}",
        run.stdout,
        run.rtt
    );
    assert_eq!(
        run.status, "pass",
        "expected status pass; stdout: {}\nrtt.log: {}",
        run.stdout, run.rtt
    );
    assert_eq!(
        run.result["stop_reason"], "assertions_passed",
        "expected the in-loop RTT assertion to stop the run with \
         `assertions_passed`\n{}",
        run.result
    );
    let steps = run.result["steps_executed"].as_u64().unwrap_or(u64::MAX);
    assert!(
        steps < 200_000,
        "expected a stop before max_steps once the RTT assertion passed, \
         got {steps} steps\n{}",
        run.result
    );
    assert!(
        run.rtt.contains("RTT hello from labwired"),
        "the early stop must not drop the captured banner\n--- rtt.log ---\n{}",
        run.rtt
    );
}

/// `--rtt` alone, with no `rtt_contains` assertion, must enable the dedicated
/// capture stream: banner in `rtt.log`, populated `rtt` block in result.json.
/// The same script WITHOUT `--rtt` is the negative control: empty `rtt.log`
/// and no `rtt` key at all (so "capture was never on" stays distinguishable
/// from "capture was on and silent").
#[test]
fn rtt_flag_alone_enables_capture() {
    let root = repo_root();
    let fw = ensure_firmware_built(&root);
    let body = r#"limits:
  max_steps: 200000
assertions:
  - expected_stop_reason: max_steps
"#;

    let with_flag = run_script_args(&root, &fw, body, &["--rtt"]);
    assert_eq!(
        with_flag.exit_code,
        Some(0),
        "expected exit 0 with --rtt; stdout: {}\nrtt.log: {}",
        with_flag.stdout,
        with_flag.rtt
    );
    assert_eq!(
        with_flag.status, "pass",
        "stdout: {}\nrtt.log: {}",
        with_flag.stdout, with_flag.rtt
    );
    assert!(
        with_flag.rtt.contains("RTT hello from labwired"),
        "--rtt must capture the banner into rtt.log\n--- rtt.log ---\n{}",
        with_flag.rtt
    );
    assert_eq!(
        with_flag.result["rtt"]["control_block_found"].as_bool(),
        Some(true),
        "expected a populated result.json rtt block\n{}",
        with_flag.result
    );
    assert!(
        with_flag.result["rtt"]["bytes_drained"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "expected bytes_drained > 0\n{}",
        with_flag.result
    );

    let without_flag = run_script(&root, &fw, body);
    assert_eq!(
        without_flag.exit_code,
        Some(0),
        "expected exit 0 without --rtt; stdout: {}\nrtt.log: {}",
        without_flag.stdout,
        without_flag.rtt
    );
    assert!(
        without_flag.rtt.is_empty(),
        "without --rtt (and without rtt_contains) rtt.log must stay empty\n\
         --- rtt.log ---\n{}",
        without_flag.rtt
    );
    assert!(
        without_flag.result.get("rtt").is_none(),
        "without RTT enabled result.json must not carry an `rtt` key\n{}",
        without_flag.result
    );
}
