// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The dogfood gate: the checked-in `simctl` self-test must actually pass.
//!
//! Everything else about this feature is proved with firmware assembled inside
//! a test. This one runs `tests/fixtures/simctl-selftest-thumbv7m.elf`, which
//! `arm-none-eabi-gcc` built from `examples/simctl-selftest/main.c` against the
//! **generated** header — so it exercises the whole chain a user touches: the
//! header's macros, a checked-in board declaring the device, a checked-in
//! script asserting `firmware_exit: 0`, and the real `labwired` binary.
//!
//! The ELF is committed (like every other firmware fixture here) so this gate
//! needs no embedded toolchain. Rebuild it with
//! `examples/simctl-selftest/build.sh` after changing the firmware or header.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize workspace root")
}

const SCRIPT: &str = "examples/simctl-selftest/simctl-selftest.yaml";

fn run(script: &Path, out: &Path) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--no-uart-stdout",
            "--output-dir",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run the labwired binary");

    let path = out.join("result.json");
    let body = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "no result.json at {}: {e}\nstdout:\n{}\nstderr:\n{}",
            path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    serde_json::from_str(&body).expect("result.json is valid JSON")
}

#[test]
fn the_checked_in_selftest_passes() {
    let out = labwired_cli::test_support::unique_temp_dir("labwired-simctl-dogfood");
    let _ = std::fs::remove_dir_all(&out);
    let result = run(&workspace_root().join(SCRIPT), &out);

    assert_eq!(
        result["stop_reason"], "firmware_exit",
        "the run must end because the firmware said so; result: {result}"
    );
    assert_eq!(result["firmware_exit_code"], 0);
    assert_eq!(result["status"], "pass", "result: {result}");

    // The assertion in the script is the `firmware_exit` one, and it passed.
    let assertions = result["assertions"].as_array().expect("assertions array");
    assert_eq!(assertions.len(), 1, "result: {result}");
    assert_eq!(assertions[0]["passed"], true);
}

#[test]
fn the_selftests_assertion_actually_gates() {
    // ANTI-VACUITY CONTROL. Same firmware, same board, but the script demands a
    // code the firmware does not produce. If this still passes, the assertion
    // above is decorative.
    let root = workspace_root();
    let original = std::fs::read_to_string(root.join(SCRIPT)).expect("read the script");
    assert!(
        original.contains("firmware_exit: 0"),
        "the checked-in script no longer asserts firmware_exit: 0"
    );

    let dir = std::env::temp_dir().join("labwired-tests");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let mutated_path = dir.join(format!(
        "{}.yaml",
        labwired_cli::test_support::unique_name("simctl-dogfood-negative")
    ));

    // Rewrite the relative input paths so the copy resolves from its new home.
    let script_dir = root.join(SCRIPT);
    let script_dir = script_dir.parent().unwrap();
    let mutated = original
        .replace("firmware_exit: 0", "firmware_exit: 99")
        .replace(
            "../../tests/fixtures/",
            &format!("{}/tests/fixtures/", root.display()),
        )
        .replace(
            "../../configs/systems/",
            &format!("{}/configs/systems/", root.display()),
        );
    assert!(
        mutated.contains("firmware_exit: 99"),
        "the negative control did not actually change the expected code"
    );
    let _ = script_dir;
    std::fs::write(&mutated_path, mutated).expect("write mutated script");

    let out = labwired_cli::test_support::unique_temp_dir("labwired-simctl-dogfood-neg");
    let _ = std::fs::remove_dir_all(&out);
    let result = run(&mutated_path, &out);

    assert_eq!(
        result["firmware_exit_code"], 0,
        "the firmware still exits 0; only the expectation changed"
    );
    assert_eq!(
        result["status"], "fail",
        "asserting exit 99 against firmware that exits 0 must fail; result: {result}"
    );
}
