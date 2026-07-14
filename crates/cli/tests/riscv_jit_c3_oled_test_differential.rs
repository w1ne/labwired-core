// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! HARD GATE for engaging the RV32IMC wasm-JIT on the DEPLOYED `labwired test`
//! ESP32-C3 firmware-verify oracle.
//!
//! Drives the REAL `labwired` binary (the same one the builder service runs) on
//! the `esp32c3-oled-demo` rom-boot lab TWICE — JIT-on (`LABWIRED_RISCV_JIT=1`)
//! vs JIT-off (`LABWIRED_RISCV_JIT=0`) — and asserts the emitted `result.json`
//! is byte-identical. `result.json` carries no wall-time / IPS / timestamp field
//! (only deterministic `cycles`/`instructions` counters, the decoded register
//! and framebuffer inspect block, and serial-derived assertions), so no
//! host-variant normalization is needed — the comparison is a literal byte diff.
//!
//! Both arms take the JIT-eligible path (`riscv_jit_test_eligible` →
//! `Machine::run` at the bus max-safe tick interval, sourcing cycles from
//! `machine.total_cycles` instead of the metrics step observer). The ONLY thing
//! `LABWIRED_RISCV_JIT` changes is whether hot basic blocks are dispatched
//! through the compiled engine; because cycles are machine-sourced (compiled
//! blocks retire without firing `on_step_end`), the two arms match to the byte.
//!
//! Two scenarios, per the gate. The `max_steps` scenario runs to a step budget:
//! the OLED paints ("OLED painted: LabWired" on serial, framebuffer in the
//! inspect block), proving the verdict and telemetry match. The `max_cycles`
//! scenario stops the run mid-flight on a cycle LIMIT, proving the cycle-sourcing
//! swap fires `max_cycles` at the exact same point on both arms (the crux).
//!
//! Under a `jit-core` build the JIT-on arm is additionally asserted NON-VACUOUS
//! (compiled blocks actually ran) by parsing the `LABWIRED_JIT_STATS=1` stderr.
//! Without `jit-core` the eligible path is compiled out (`cfg!` false) so both
//! arms run the identical interpreter and byte-identity holds trivially.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Repo root = crates/cli/../.. (matches the other CLI integration tests).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn require(path: PathBuf) -> PathBuf {
    assert!(path.exists(), "missing fixture: {}", path.display());
    path
}

struct Fixtures {
    flash: PathBuf,
    elf: PathBuf,
    system: PathBuf,
}

fn fixtures() -> Fixtures {
    let root = repo_root();
    Fixtures {
        // The curated OLED-demo flash image (bootloader + partition table + app)
        // the browser fast-start and the core differential gate both boot.
        flash: require(root.join("crates/wasm/tests/fixtures/esp32c3-oled-demo-flash.bin")),
        // rom-boot executes the FLASH; the ELF only feeds symbols/diagnostics, so
        // any RISC-V C3 ELF drives the same run (both arms use the identical one).
        elf: require(root.join("tests/fixtures/esp32c3-demo.elf")),
        system: require(root.join("configs/systems/esp32c3-oled-demo.yaml")),
    }
}

/// Write a `labwired test` YAML script to `dir/name` and return its path.
fn write_script(dir: &Path, name: &str, fx: &Fixtures, limits: &str, assertion: &str) -> PathBuf {
    let script = format!(
        "schema_version: \"1.0\"\n\
         inputs:\n  \
           firmware: \"{}\"\n  \
           system: \"{}\"\n\
         limits:\n{}\
         assertions:\n{}\n",
        fx.elf.display(),
        fx.system.display(),
        limits,
        assertion,
    );
    let path = dir.join(name);
    std::fs::write(&path, script).expect("write test script");
    path
}

struct RunOutput {
    result_json: Vec<u8>,
    stderr: String,
    exit_ok: bool,
}

/// Invoke the built `labwired test` binary once with the given JIT toggle.
fn run_oracle(fx: &Fixtures, script: &Path, out_dir: &Path, jit_env: &str) -> RunOutput {
    std::fs::create_dir_all(out_dir).expect("create out dir");
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(repo_root())
        .env("LABWIRED_ESP32C3_FLASH", &fx.flash)
        .env("LABWIRED_RISCV_JIT", jit_env)
        .env("LABWIRED_JIT_STATS", "1")
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--rom-boot",
            "--no-uart-stdout",
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("spawn labwired");
    let result_path = out_dir.join("result.json");
    let result_json = std::fs::read(&result_path).unwrap_or_else(|e| {
        panic!(
            "no result.json at {} (exit {:?}): {e}\nstderr:\n{}",
            result_path.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    RunOutput {
        result_json,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_ok: output.status.success(),
    }
}

/// Parse the `[jit-stats] compiled=N block_runs=M ...` diagnostic line, if any.
fn jit_compiled_blocks(stderr: &str) -> Option<u64> {
    let line = stderr
        .lines()
        .find(|l| l.contains("[jit-stats] compiled="))?;
    let tok = line
        .split_whitespace()
        .find(|t| t.starts_with("compiled="))?;
    tok.trim_start_matches("compiled=").parse().ok()
}

/// Core assertion: run both arms, diff `result.json`, and (under `jit-core`)
/// require the JIT-on arm to be non-vacuous.
fn assert_arms_byte_identical(scenario: &str, fx: &Fixtures, script: &Path, tmp: &Path) {
    let on = run_oracle(fx, script, &tmp.join(format!("{scenario}_on")), "1");
    let off = run_oracle(fx, script, &tmp.join(format!("{scenario}_off")), "0");

    assert!(on.exit_ok, "[{scenario}] JIT-on run failed:\n{}", on.stderr);
    assert!(
        off.exit_ok,
        "[{scenario}] JIT-off run failed:\n{}",
        off.stderr
    );

    assert!(
        on.result_json == off.result_json,
        "[{scenario}] result.json DIVERGED between JIT-on and JIT-off ({} vs {} bytes)\n\
         JIT-on:\n{}\nJIT-off:\n{}",
        on.result_json.len(),
        off.result_json.len(),
        String::from_utf8_lossy(&on.result_json),
        String::from_utf8_lossy(&off.result_json),
    );

    // Non-vacuity: only meaningful when the binary was built WITH the JIT
    // (`jit-core`). The test crate and the binary share the cargo feature set,
    // so `cfg!(feature = "jit-core")` reflects the binary's capability.
    if cfg!(feature = "jit-core") {
        let compiled = jit_compiled_blocks(&on.stderr).unwrap_or(0);
        assert!(
            compiled > 0,
            "[{scenario}] JIT-on arm was VACUOUS (compiled={compiled}); \
             the eligible batch never ran through the compiled engine.\nstderr:\n{}",
            on.stderr
        );
        // The JIT-off arm must NOT have created a JIT engine.
        assert!(
            jit_compiled_blocks(&off.stderr).is_none(),
            "[{scenario}] JIT-off arm unexpectedly compiled blocks:\n{}",
            off.stderr
        );
        eprintln!("[{scenario}] byte-identical; JIT-on compiled {compiled} blocks (non-vacuous)");
    } else {
        eprintln!("[{scenario}] byte-identical (no jit-core: eligible path compiled out)");
    }
}

#[test]
fn jit_on_vs_off_result_json_byte_identical_c3_oled() {
    let fx = fixtures();
    let tmp = std::env::temp_dir().join(format!("lw-jit-oled-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create tmp dir");

    // Scenario 1: run to a step budget — the OLED paints and serial reports it.
    // 8M steps is comfortably past the "OLED painted: LabWired" milestone.
    let s_steps = write_script(
        &tmp,
        "oled_max_steps.yaml",
        &fx,
        "  max_steps: 8000000\n",
        "  - expected_stop_reason: max_steps\n  - uart_contains: \"OLED painted: LabWired\"",
    );
    assert_arms_byte_identical("max_steps", &fx, &s_steps, &tmp);

    // Scenario 2: a CYCLE limit that stops the run mid-flight. This is what
    // proves the cycle-sourcing swap (metrics.cycles ← machine.total_cycles)
    // fires `max_cycles` at the identical point on both arms.
    let s_cycles = write_script(
        &tmp,
        "oled_max_cycles.yaml",
        &fx,
        "  max_steps: 20000000\n  max_cycles: 5000000\n",
        "  - expected_stop_reason: max_cycles",
    );
    assert_arms_byte_identical("max_cycles", &fx, &s_cycles, &tmp);

    let _ = std::fs::remove_dir_all(&tmp);
}
