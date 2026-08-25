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
//! RISC-V did not either, and was worse: both of its loops logged the fault
//! through `tracing::debug!`, which prints nothing at the default level, so an
//! ESP32-C3 run that died on a `DecodeError` produced no stderr at all and exit
//! 0. The RISC-V half of this file repeats the same cases, and covers each of
//! its two run loops separately, because the two carried their own copies of
//! the decision and had already drifted apart from ARM together.
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

// ── RISC-V ──────────────────────────────────────────────────────────────────

/// A RISC-V chip and an ELF built for something else: it loads, then the
/// RV32IMC decoder meets Thumb bytes and raises `DecodeError` four
/// instructions in.
///
/// `envs` selects the loop. With none set, `run_firmware_riscv` hands the run
/// to `run_firmware_riscv_batched` — the default, and the only path a normal
/// invocation takes. `LABWIRED_DHCP_TRACE=1` turns on per-instruction
/// instrumentation, which pins the run to the single-step loop instead. Both
/// had their own `Err` arm and both swallowed the fault, so both are covered.
fn faulting_riscv_run(extra: &[&str], envs: &[(&str, &str)]) -> std::process::Output {
    let root = workspace_root();
    let mut cmd = Command::new(labwired_bin());
    cmd.arg("run")
        .arg("--chip")
        .arg(root.join("configs/chips/esp32c3.yaml"))
        .arg("--firmware")
        .arg(root.join("tests/fixtures/nrf52832-demo.elf"))
        .arg("--max-steps")
        .arg("100000");
    for arg in extra {
        cmd.arg(arg);
    }
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.output().expect("spawn labwired")
}

#[test]
fn a_riscv_run_that_ends_on_a_fault_exits_non_zero() {
    let out = faulting_riscv_run(&[], &[]);
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

/// The regression this file exists for, in its RISC-V form: the fault used to
/// be a `tracing::debug!`, which prints nothing at the default level and
/// nothing at all with logging off. `RUST_LOG=off` is the honest test of
/// whether the diagnostic is a log line or a report — before this fix the
/// whole run produced zero bytes of stderr and exit 0.
#[test]
fn the_riscv_fault_is_reported_even_with_logging_off() {
    let out = faulting_riscv_run(&[], &[("RUST_LOG", "off")]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("simulation error"),
        "the fault must be reported even with logging off, got {} byte(s): {stderr}",
        out.stderr.len()
    );
    assert_eq!(out.status.code(), Some(EXIT_RUNTIME_ERROR));
}

/// The instrumented single-step loop is a second copy of the same decision.
/// It kept its own `Err` arm, so it needs its own test.
#[test]
fn the_riscv_single_step_loop_reports_a_fault_the_same_way() {
    let out = faulting_riscv_run(&[], &[("LABWIRED_DHCP_TRACE", "1"), ("RUST_LOG", "off")]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(EXIT_RUNTIME_ERROR),
        "the single-step loop must not have a different verdict from the \
         batched one; stderr was: {stderr}"
    );
}

/// `--batched` is an assertion about which loop ran. It must not change the
/// verdict — on RISC-V it selects the same loop a default run already takes.
#[test]
fn the_riscv_batched_flag_reports_a_fault_the_same_way() {
    let out = faulting_riscv_run(&["--batched"], &[("RUST_LOG", "off")]);
    assert_eq!(out.status.code(), Some(EXIT_RUNTIME_ERROR));
}

#[test]
fn allow_sim_error_restores_exit_zero_on_riscv_too() {
    let out = faulting_riscv_run(&["--allow-sim-error"], &[("RUST_LOG", "off")]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("simulation error"),
        "the fault is still reported, only the exit status changes: {stderr}"
    );
    assert_eq!(out.status.code(), Some(0), "explicit opt-in must exit 0");
}

/// The other half of the blast radius: every RISC-V run that ends without a
/// fault has to stay a pass. This one runs to `--max-steps` after its firmware
/// has printed, which is the shape of every TIER1 C3 fixture.
#[test]
fn a_clean_riscv_run_still_exits_zero() {
    let root = workspace_root();
    let out = Command::new(labwired_bin())
        .arg("run")
        .arg("--chip")
        .arg(root.join("configs/chips/esp32c3.yaml"))
        .arg("--firmware")
        .arg(root.join("tests/fixtures/esp32c3-demo.elf"))
        .arg("--max-steps")
        .arg("200000")
        .env("RUST_LOG", "off")
        .output()
        .expect("spawn labwired");
    assert_eq!(
        out.status.code(),
        Some(0),
        "a RISC-V run with no fault must stay a pass; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("ESP OK"),
        "the fixture must actually have run: stdout was {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}
