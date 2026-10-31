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

// ── PIC18 interactive run ───────────────────────────────────────────────────
//
// The PIC24/PIC18 arms of the interactive runner (`labwired --firmware …
// --system …`) returned 0 for every run, faulted or not. The PIC18F4520
// unsupported-instruction audit judges by that exit status, so an
// undecodable opcode read as "0 unsupported".

/// A one-segment ELF32 stamped EM_MCHP_PIC (204), loaded at byte address 0,
/// as `examples/pic18f4520/firmware/hex2elf.py` writes them.
fn pic18_elf(code: &[u8]) -> Vec<u8> {
    let (ehsize, phentsize) = (52u32, 32u32);
    let offset = ehsize + phentsize;
    let mut elf = b"\x7fELF\x01\x01\x01\x00".to_vec();
    elf.extend_from_slice(&[0; 8]);
    for half in [2u16, 204] {
        elf.extend_from_slice(&half.to_le_bytes());
    }
    for word in [1u32, 0, ehsize, 0, 0] {
        elf.extend_from_slice(&word.to_le_bytes());
    }
    for half in [ehsize as u16, phentsize as u16, 1, 40, 0, 0] {
        elf.extend_from_slice(&half.to_le_bytes());
    }
    let len = code.len() as u32;
    for word in [1u32, offset, 0, 0, len, len, 5, 1] {
        elf.extend_from_slice(&word.to_le_bytes());
    }
    elf.extend_from_slice(code);
    elf
}

fn pic18_run(code: &[u8], name: &str) -> std::process::Output {
    let root = workspace_root();
    let dir = std::env::temp_dir().join(format!("lw-pic18-exit-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let fw = dir.join("fw.elf");
    std::fs::write(&fw, pic18_elf(code)).expect("write elf");
    let out = Command::new(labwired_bin())
        .arg("--firmware")
        .arg(&fw)
        .arg("--system")
        .arg(root.join("configs/systems/pic18f4520-demo.yaml"))
        .arg("--max-steps")
        .arg("1000")
        .output()
        .expect("spawn labwired");
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn a_pic18_run_that_ends_on_a_decode_error_exits_non_zero() {
    // 0x0001 is unassigned in the standard PIC18 instruction set.
    let out = pic18_run(&[0x01, 0x00], "decode");
    assert_eq!(
        out.status.code(),
        Some(EXIT_RUNTIME_ERROR),
        "an undecodable opcode must not report success; stderr was: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_clean_pic18_run_still_exits_zero() {
    // `BRA $` (0xD7FF): spins on itself until max-steps.
    let out = pic18_run(&[0xFF, 0xD7], "clean");
    assert_eq!(
        out.status.code(),
        Some(0),
        "a max-steps run is not a fault; stderr was: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── PIC16 interactive run ───────────────────────────────────────────────────
//
// Same contract on the PIC16 arm. The image is the PIC18 container (both
// 8-bit PIC families are EM_MCHP_PIC); the pic16 chip retargets it.

fn pic16_run(code: &[u8], name: &str) -> std::process::Output {
    let root = workspace_root();
    let dir = std::env::temp_dir().join(format!("lw-pic16-exit-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let fw = dir.join("fw.elf");
    std::fs::write(&fw, pic18_elf(code)).expect("write elf");
    let out = Command::new(labwired_bin())
        .arg("--firmware")
        .arg(&fw)
        .arg("--system")
        .arg(root.join("configs/systems/pic16f877a-demo.yaml"))
        .arg("--max-steps")
        .arg("1000")
        .output()
        .expect("spawn labwired");
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn a_pic16_run_that_ends_on_a_decode_error_exits_non_zero() {
    // 0x3B00 is unassigned on the PIC16 mid-range (11 1011 xxxx xxxx) but a
    // valid SWAPF on PIC18, so this exits 3 only if the run really used the
    // PIC16 core.
    let out = pic16_run(&[0x00, 0x3B], "decode");
    assert_eq!(
        out.status.code(),
        Some(EXIT_RUNTIME_ERROR),
        "an undecodable opcode must not report success; stderr was: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_clean_pic16_run_still_exits_zero() {
    // `GOTO 0` (0x2800): spins on itself until max-steps.
    let out = pic16_run(&[0x00, 0x28], "clean");
    assert_eq!(
        out.status.code(),
        Some(0),
        "a max-steps run is not a fault; stderr was: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
