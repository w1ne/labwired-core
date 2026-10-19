//! `run --stop-on` ends the run at the user's request, not at the step
//! budget. The epilogue used to print `reached --max-steps {limit}`
//! unconditionally, so a deliberate stop told the user the budget ran out.
//!
//! Drives the same S3 rom-boot fixture the hooks receipt used: the real boot
//! ROM prints its banner early, `--stop-on "ESP-ROM"` matches it at the first
//! 100k-step scan boundary, and the run exits long before `--max-steps`.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

/// The S3 rom-boot command shape: real ROM + flash image via env pin, app ELF
/// for symbol context. `stop_on` is added when present.
fn s3_rom_boot(stop_on: Option<&str>, max_steps: u64) -> std::process::Output {
    let root = repo_root();
    let flash = root.join("tests/fixtures/source-debug/esp32s3-arduino-flash.bin");
    let elf = root.join("tests/fixtures/source-debug/esp32s3-arduino.elf");
    assert!(flash.exists(), "missing fixture: {}", flash.display());
    assert!(elf.exists(), "missing fixture: {}", elf.display());

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_labwired"));
    cmd.current_dir(&root)
        .env("LABWIRED_ESP32S3_FLASH", &flash)
        .env_remove("LABWIRED_ESP32S3_FASTBOOT")
        .args([
            "run",
            "--chip",
            "configs/chips/esp32s3.yaml",
            "--firmware",
            elf.to_str().unwrap(),
            "--rom-boot",
            "--max-steps",
            &max_steps.to_string(),
        ]);
    if let Some(pat) = stop_on {
        cmd.args(["--stop-on", pat]);
    }
    cmd.output().expect("spawn labwired")
}

#[test]
fn stop_on_match_does_not_claim_the_step_budget_ran_out() {
    let out = s3_rom_boot(Some("ESP-ROM"), 2_000_000);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a stop-on run is not a failure; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("--stop-on") && stderr.contains("matched at step"),
        "expected the stop-on match to be reported; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("reached --max-steps"),
        "a run stopped by --stop-on must not claim it exhausted --max-steps; stderr:\n{stderr}"
    );
}

/// Negative control: without `--stop-on` the budget epilogue must still be
/// printed when the run really reaches `--max-steps`.
#[test]
fn max_steps_epilogue_still_prints_without_stop_on() {
    let out = s3_rom_boot(None, 200_000);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "stderr:\n{stderr}");
    assert!(
        stderr.contains("reached --max-steps 200000"),
        "expected the budget epilogue; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("--stop-on"),
        "no stop-on report without the flag; stderr:\n{stderr}"
    );
}
