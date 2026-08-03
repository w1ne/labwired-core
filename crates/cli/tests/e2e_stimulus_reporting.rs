// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Regression gate: a declarative input stimulus that never reaches its device
// must be REPORTED, never swallowed.
//
// The runner used to only `error!` a failed stimulus into the log and carry on,
// so a script that asked for a button press, got nothing, and then passed its
// (unrelated) assertions reported `status: "pass"` — a green run that proved
// nothing about the input it was written to exercise. Every consumer of the run
// result (the hosted builder `/run`, `packages/api`, MCP `labwired_run`, the
// studio) saw that pass with no indication anything had been dropped, which
// makes "the stimulus never arrived" indistinguishable from "the stimulus
// arrived and the firmware ignored it" — completely different bugs.
//
// These tests pin all three outcomes end-to-end through the real `labwired
// test` CLI, on the committed KW41Z cow demo (FRDM-KW41Z + FXOS8700):
//
//   • applied     — the stimulus reached the device.
//   • rejected    — the engine refused it; the run is INVALID (status "error",
//                   exit 2) even though its assertions pass.
//   • not_reached — an `after_cycles` threshold the run never got to; reported,
//                   but deliberately not fatal.
//
// The rejected cases are built so that the run would otherwise be a clean pass.
// That is the whole point: if this file ever goes green with `status: "pass"`,
// the false-pass has come back.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

struct Run {
    exit_code: Option<i32>,
    stderr: String,
    result: serde_json::Value,
}

impl Run {
    fn status(&self) -> &str {
        self.result["status"].as_str().unwrap_or("<missing>")
    }
    fn stimuli(&self) -> &[serde_json::Value] {
        self.result["stimuli"]
            .as_array()
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}

/// Write `script_body` into a temp dir and run it through `labwired test`.
///
/// The script is generated rather than committed so a deliberately broken
/// stimulus never sits in `examples/` where an author might copy it. Firmware
/// and system paths are absolute, so the script's location does not matter.
fn run_script(script_body: &str) -> Run {
    let root = repo_root();
    // Timestamp PLUS a process-wide counter. `as_nanos()` is not actually
    // nanosecond-granular on macOS, so sibling tests starting in the same
    // microsecond used to land in the SAME directory and overwrite each
    // other's script.yaml/result.json — a random subset of this file failed
    // on every parallel run, a different subset each time, while
    // `--test-threads=1` always passed. The counter makes the name unique
    // regardless of what the clock resolves to.
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-stimulus-report");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = dir.join("script.yaml");
    std::fs::write(&script, script_body).expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--no-uart-stdout",
            "--output-dir",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute labwired");

    let result_json = std::fs::read_to_string(dir.join("result.json")).expect("read result.json");
    let result: serde_json::Value = serde_json::from_str(&result_json).expect("parse result.json");

    let _ = std::fs::remove_dir_all(&dir);
    Run {
        exit_code: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        result,
    }
}

/// The cow demo with `stimuli:` spliced in. `max_steps` is small: these tests
/// are about the stimulus bookkeeping, not the firmware's behaviour, and the
/// `MOOD=CALM` assertion passes early.
fn cow_script(stimuli: &str) -> String {
    let root = repo_root();
    format!(
        r#"schema_version: "1.2"
inputs:
  firmware: "{fw}"
  system: "{sys}"
limits:
  max_steps: 250000
  max_uart_bytes: 8000
{stimuli}assertions:
  - uart_regex: "MOOD=CALM"
"#,
        fw = root.join("tests/fixtures/kw41z-lcd-activity.elf").display(),
        sys = root.join("configs/systems/frdm-kw41z-lcd.yaml").display(),
    )
}

/// Control case. A stimulus that DOES resolve is reported as `applied`, and the
/// run still passes. Without this, "make every stimulus fatal" would trivially
/// satisfy the other tests while destroying the feature.
#[test]
fn an_applied_stimulus_is_reported_as_applied_and_the_run_still_passes() {
    let run = run_script(&cow_script(
        "stimuli:\n  - target: { component: fxos8700, channel: x }\n    trigger: !after_cycles { cycles: 50000 }\n    value: 2.0\n",
    ));

    assert_eq!(
        run.exit_code,
        Some(0),
        "a resolvable stimulus must not fail the run; stderr:\n{}",
        run.stderr
    );
    assert_eq!(run.status(), "pass", "result.json: {}", run.result);

    let stimuli = run.stimuli();
    assert_eq!(
        stimuli.len(),
        1,
        "expected one stimulus record: {}",
        run.result
    );
    assert_eq!(stimuli[0]["outcome"], "applied", "{}", run.result);
    assert_eq!(stimuli[0]["channel"], "x");
    assert_eq!(stimuli[0]["component"], "fxos8700");
    assert!(
        stimuli[0]["error"].is_null(),
        "an applied stimulus carries no error: {}",
        run.result
    );
    // An applied stimulus must not invent a run-level failure message.
    assert!(
        run.result["message"].is_null(),
        "clean run must have no message: {}",
        run.result
    );
}

/// An unknown CHANNEL is refused by the engine. The run's assertions still pass,
/// so before this gate existed it reported `status: "pass"` and exit 0.
#[test]
fn an_unknown_channel_is_reported_and_fails_the_run() {
    let run = run_script(&cow_script(
        "stimuli:\n  - target: { channel: definitely_not_a_channel }\n    trigger: at_start\n    value: 1.0\n",
    ));

    // The assertions in this script PASS. The run is non-passing purely because
    // the declared input never reached a device.
    assert_eq!(
        run.exit_code,
        Some(2),
        "a dropped stimulus must exit EXIT_CONFIG_ERROR; stderr:\n{}",
        run.stderr
    );
    assert_eq!(
        run.status(),
        "error",
        "a dropped stimulus must not report pass: {}",
        run.result
    );

    let stimuli = run.stimuli();
    assert_eq!(
        stimuli.len(),
        1,
        "expected one stimulus record: {}",
        run.result
    );
    assert_eq!(stimuli[0]["outcome"], "rejected", "{}", run.result);
    assert_eq!(stimuli[0]["channel"], "definitely_not_a_channel");

    // The reason must name what actually went wrong, not just "failed".
    let err = stimuli[0]["error"].as_str().unwrap_or_default();
    assert!(
        err.contains("no attached input device exposes channel")
            && err.contains("definitely_not_a_channel"),
        "rejection reason must be actionable, got {err:?}: {}",
        run.result
    );

    // Impossible to miss: the failure is also on the top-level message.
    let message = run.result["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("could not be applied") && message.contains("definitely_not_a_channel"),
        "run-level message must carry the rejection, got {message:?}"
    );
}

/// An unknown COMPONENT is the case that actually bit us: a `btn1` button that
/// the running engine has no model for resolves to nothing, and the press is
/// dropped.
#[test]
fn an_unknown_component_is_reported_and_fails_the_run() {
    let run = run_script(&cow_script(
        "stimuli:\n  - target: { component: btn1, channel: pressed }\n    trigger: at_start\n    value: 1.0\n",
    ));

    assert_eq!(
        run.exit_code,
        Some(2),
        "a dropped stimulus must exit EXIT_CONFIG_ERROR; stderr:\n{}",
        run.stderr
    );
    assert_eq!(run.status(), "error", "result.json: {}", run.result);

    let stimuli = run.stimuli();
    assert_eq!(
        stimuli.len(),
        1,
        "expected one stimulus record: {}",
        run.result
    );
    assert_eq!(stimuli[0]["outcome"], "rejected", "{}", run.result);
    assert_eq!(stimuli[0]["component"], "btn1");
    assert_eq!(stimuli[0]["channel"], "pressed");
    assert!(
        stimuli[0]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("btn1/pressed"),
        "the reason must name the unresolved component: {}",
        run.result
    );

    // The stderr line stays too — it is how the hosted builder's captured logs
    // show the same failure.
    assert!(
        run.stderr.contains("could not be applied"),
        "stderr must still carry the rejection\n--- stderr ---\n{}",
        run.stderr
    );
}

/// An `after_cycles` stimulus whose threshold the run never reaches also proved
/// nothing — it is reported as `not_reached`, but deliberately does NOT fail the
/// run (a run may legitimately stop early with a later rung unfired).
#[test]
fn a_stimulus_that_never_fires_is_reported_but_is_not_fatal() {
    let run = run_script(&cow_script(
        "stimuli:\n  - target: { component: fxos8700, channel: x }\n    trigger: !after_cycles { cycles: 999000000 }\n    value: 2.0\n",
    ));

    assert_eq!(
        run.exit_code,
        Some(0),
        "an unfired stimulus is reported, not fatal; stderr:\n{}",
        run.stderr
    );
    assert_eq!(run.status(), "pass", "result.json: {}", run.result);

    let stimuli = run.stimuli();
    assert_eq!(
        stimuli.len(),
        1,
        "expected one stimulus record: {}",
        run.result
    );
    assert_eq!(stimuli[0]["outcome"], "not_reached", "{}", run.result);
    assert!(
        stimuli[0]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("never fired"),
        "the reason must say it never fired: {}",
        run.result
    );
}

/// A script with no stimuli emits no `stimuli` block at all, so `result.json`
/// stays byte-identical for every run that never used the feature (the release
/// runner contract's golden reference and `tests/determinism.rs` depend on it).
#[test]
fn a_script_without_stimuli_emits_no_stimuli_block() {
    let run = run_script(&cow_script(""));
    assert_eq!(run.exit_code, Some(0), "stderr:\n{}", run.stderr);
    assert_eq!(run.status(), "pass");
    assert!(
        run.result.get("stimuli").is_none(),
        "an unused feature must not appear in the result: {}",
        run.result
    );
}
