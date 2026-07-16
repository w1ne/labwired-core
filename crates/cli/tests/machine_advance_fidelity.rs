// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos();
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lw-advance-fidelity-{}-{nonce}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create fidelity temp directory");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

struct RunOutput {
    result: Vec<u8>,
    snapshot: Vec<u8>,
    junit: String,
    uart: Vec<u8>,
    exit_ok: bool,
    stderr: String,
}

fn run_fixture(dir: &Path, firmware: &Path, system: &Path, engine: Option<&str>) -> RunOutput {
    std::fs::create_dir_all(dir).expect("create engine directory");
    let script_path = dir.join("script.yaml");
    let output_dir = dir.join("artifacts");
    let script = format!(
        r#"schema_version: "1.0"
inputs:
  firmware: {:?}
  system: {:?}
limits:
  max_steps: 32
assertions:
  - expected_stop_reason: max_steps
"#,
        firmware.display().to_string(),
        system.display().to_string(),
    );
    std::fs::write(&script_path, script).expect("write fidelity script");

    let mut command = Command::new(env!("CARGO_BIN_EXE_labwired"));
    command
        .current_dir(dir)
        .args(["test", "--script", "script.yaml"])
        .args(["--no-uart-stdout", "--output-dir", "artifacts"]);
    if let Some(engine) = engine {
        command.env("LABWIRED_TEST_EXECUTOR", engine);
    } else {
        command.env_remove("LABWIRED_TEST_EXECUTOR");
    }
    let output = command.output().expect("run labwired fidelity fixture");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let read = |name: &str| {
        std::fs::read(output_dir.join(name)).unwrap_or_else(|error| {
            panic!("read {name} for executor={engine:?}: {error}\nstderr:\n{stderr}")
        })
    };

    RunOutput {
        result: read("result.json"),
        snapshot: read("snapshot.json"),
        junit: String::from_utf8(read("junit.xml")).expect("JUnit is UTF-8"),
        uart: read("uart.log"),
        exit_ok: output.status.success(),
        stderr,
    }
}

fn is_numeric_attribute_value(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        index += 1;
    }

    let integer_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    let mut has_digits = index > integer_start;

    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        has_digits |= index > fraction_start;
    }
    if !has_digits {
        return false;
    }

    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }

    index == bytes.len()
}

fn normalize_tag(tag: &str) -> String {
    const PREFIX: &str = "time=\"";
    let mut normalized = String::with_capacity(tag.len());
    let mut cursor = 0;

    while let Some(relative) = tag[cursor..].find(PREFIX) {
        let start = cursor + relative;
        let is_attribute = start > 0 && tag.as_bytes()[start - 1].is_ascii_whitespace();
        let value_start = start + PREFIX.len();
        let Some(relative_end) = tag[value_start..].find('"') else {
            break;
        };
        let value_end = value_start + relative_end;
        normalized.push_str(&tag[cursor..start]);
        if is_attribute && is_numeric_attribute_value(&tag[value_start..value_end]) {
            normalized.push_str("time=\"0\"");
        } else {
            normalized.push_str(&tag[start..=value_end]);
        }
        cursor = value_end + 1;
    }
    normalized.push_str(&tag[cursor..]);
    normalized
}

fn normalize_junit(junit: &str) -> String {
    let mut normalized = String::with_capacity(junit.len());
    let mut cursor = 0;
    while let Some(relative_start) = junit[cursor..].find('<') {
        let start = cursor + relative_start;
        normalized.push_str(&junit[cursor..start]);
        let Some(relative_end) = junit[start..].find('>') else {
            normalized.push_str(&junit[start..]);
            return normalized;
        };
        let end = start + relative_end + 1;
        normalized.push_str(&normalize_tag(&junit[start..end]));
        cursor = end;
    }
    normalized.push_str(&junit[cursor..]);
    normalized
}

fn assert_single_executor_marker(run: &RunOutput, expected: &str) {
    let markers: Vec<_> = run
        .stderr
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("executor="))
        .collect();
    assert_eq!(
        markers,
        vec![expected],
        "unexpected executor markers in stderr:\n{}",
        run.stderr
    );
}

#[test]
fn cli_unified_batch_matches_single_step_artifacts() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize workspace root");
    let firmware = root
        .join("tests/fixtures/uart-ok-thumbv7m.elf")
        .canonicalize()
        .expect("canonicalize UART fixture");
    let system = root
        .join("configs/systems/ci-fixture-uart1.yaml")
        .canonicalize()
        .expect("canonicalize UART system");
    let temp = TempDir::new();

    let legacy = run_fixture(
        &temp.path.join("legacy"),
        &firmware,
        &system,
        Some("legacy"),
    );
    let unified = run_fixture(
        &temp.path.join("unified"),
        &firmware,
        &system,
        Some("unified"),
    );

    assert!(
        legacy.exit_ok && unified.exit_ok,
        "legacy={} unified={}\nlegacy stderr:\n{}\nunified stderr:\n{}",
        legacy.exit_ok,
        unified.exit_ok,
        legacy.stderr,
        unified.stderr
    );
    assert_single_executor_marker(&legacy, "executor=legacy");
    assert_single_executor_marker(&unified, "executor=unified");
    assert_eq!(legacy.uart, unified.uart, "UART artifacts differ");
    assert_eq!(
        legacy.result, unified.result,
        "result.json artifacts differ"
    );
    assert_eq!(
        legacy.snapshot, unified.snapshot,
        "snapshot.json artifacts differ"
    );
    assert_eq!(
        normalize_junit(&legacy.junit),
        normalize_junit(&unified.junit),
        "normalized JUnit artifacts differ"
    );
}

#[test]
fn cli_defaults_absent_and_unknown_executor_to_unified() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize workspace root");
    let firmware = root
        .join("tests/fixtures/uart-ok-thumbv7m.elf")
        .canonicalize()
        .expect("canonicalize UART fixture");
    let system = root
        .join("configs/systems/ci-fixture-uart1.yaml")
        .canonicalize()
        .expect("canonicalize UART system");
    let temp = TempDir::new();

    let absent = run_fixture(&temp.path.join("absent"), &firmware, &system, None);
    let unknown = run_fixture(
        &temp.path.join("unknown"),
        &firmware,
        &system,
        Some("unexpected-value"),
    );

    assert!(absent.exit_ok, "absent executor failed:\n{}", absent.stderr);
    assert!(
        unknown.exit_ok,
        "unknown executor failed:\n{}",
        unknown.stderr
    );
    assert_single_executor_marker(&absent, "executor=unified");
    assert_single_executor_marker(&unknown, "executor=unified");
}

#[test]
fn normalize_junit_replaces_only_numeric_time_attributes() {
    let junit = concat!(
        "<testsuite time=\"12.5\" cycles=\"77\" instructions=\"66\">",
        "<testcase time=\"1e-3\" assertions=\"2\" name=\"firmware\"/>",
        "</testsuite>"
    );

    assert_eq!(
        normalize_junit(junit),
        concat!(
            "<testsuite time=\"0\" cycles=\"77\" instructions=\"66\">",
            "<testcase time=\"0\" assertions=\"2\" name=\"firmware\"/>",
            "</testsuite>"
        )
    );
}

#[test]
fn normalize_junit_preserves_non_time_fields_and_text() {
    let junit = concat!(
        "<testsuite time=\"not-numeric\" runtime=\"9\" cycles=\"12\" instructions=\"8\">",
        "<testcase assertions=\"3\"><failure>time=\"7\" cycles=99; instructions=88</failure>",
        "firmware output and stop details</testcase></testsuite>"
    );

    assert_eq!(normalize_junit(junit), junit);
}
