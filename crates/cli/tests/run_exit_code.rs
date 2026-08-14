//! `labwired run` reports a fault in its exit status.
//!
//! It used to print
//!
//! ```text
//! labwired run (arm): simulation error: Memory access violation at 0x8000400
//! ```
//!
//! and exit 0, so `labwired run … && echo ok` printed `ok` for firmware that
//! died on its second instruction, and any harness judging by exit status read
//! a memory access violation as a pass. Xtensa already exited non-zero on the
//! same class of failure; ARM did not.
//!
//! The escape hatch stays, because one caller genuinely owns the verdict: the
//! TIER1 matrix reads protocol lines from stdout and a late fault is noise to
//! it. That is what `--allow-sim-error` is for — an explicit opt-in, rather
//! than the default that hid every other caller's failures.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Exit 3 — `EXIT_RUNTIME_ERROR`, shared with every other runtime failure.
const EXIT_RUNTIME_ERROR: i32 = 3;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn labwired_bin() -> PathBuf {
    // The integration-test binary sits next to the CLI it tests.
    let mut path = std::env::current_exe().expect("test binary path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("labwired")
}

/// An ARM chip and an ELF built for something else: it loads, then faults.
fn faulting_run(extra: &[&str]) -> std::process::Output {
    let root = workspace_root();
    let mut cmd = Command::new(labwired_bin());
    cmd.arg("run")
        .arg("--chip")
        .arg(root.join("configs/chips/nrf52832.yaml"))
        .arg("--firmware")
        .arg(root.join("crates/core/tests/fixtures/esp32_brom.elf"))
        .arg("--max-steps")
        .arg("100000");
    for arg in extra {
        cmd.arg(arg);
    }
    cmd.output().expect("spawn labwired")
}

#[test]
fn a_run_that_ends_on_a_fault_exits_non_zero() {
    let out = faulting_run(&[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("simulation error"),
        "expected the fault to be reported on stderr, got: {stderr}"
    );
    assert_eq!(
        out.status.code(),
        Some(EXIT_RUNTIME_ERROR),
        "a fault must not report success; stderr was: {stderr}"
    );
}

#[test]
fn the_batched_loop_reports_a_fault_the_same_way() {
    let out = faulting_run(&["--batched"]);
    assert_eq!(
        out.status.code(),
        Some(EXIT_RUNTIME_ERROR),
        "--batched must not have a different verdict from the default loop"
    );
}

#[test]
fn allow_sim_error_restores_exit_zero_for_callers_that_own_the_verdict() {
    let out = faulting_run(&["--allow-sim-error"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("simulation error"),
        "the fault is still reported, only the exit status changes: {stderr}"
    );
    assert_eq!(out.status.code(), Some(0), "explicit opt-in must exit 0");
}

#[test]
fn a_clean_run_still_exits_zero() {
    let root = workspace_root();
    let out = Command::new(labwired_bin())
        .arg("run")
        .arg("--chip")
        .arg(root.join("configs/chips/nrf54l15.yaml"))
        .arg("--firmware")
        .arg(root.join("tests/fixtures/nrf54l15-smoke.elf"))
        .arg("--max-steps")
        .arg("2000000")
        .output()
        .expect("spawn labwired");
    assert_eq!(
        out.status.code(),
        Some(0),
        "a run with no fault must stay a pass; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
