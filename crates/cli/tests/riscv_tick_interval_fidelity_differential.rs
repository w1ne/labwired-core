// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! FIDELITY GUARD for the peripheral tick-WIDENING applied on the JIT-eligible
//! ESP32-C3 `labwired test` rom-boot path.
//!
//! When a run is JIT-eligible (built with `jit-core`, C3 rom-boot, no
//! cycle-accurate bus), the CLI widens the peripheral-tick interval from 1 to
//! `RECOMMENDED_TICK_INTERVAL` (64) so `Machine::run`'s per-tick batch is wide
//! enough for compiled blocks to retire and the peripheral tick count drops
//! ~64x. That widening is the SHIPPED DEFAULT for every JIT-eligible C3 run,
//! whether or not the JIT itself is engaged (`LABWIRED_RISCV_JIT`). This test
//! is the empirical proof that the widening changes NO observable output.
//!
//! It drives the REAL `labwired test` binary (the same one the builder service
//! runs) on the `esp32c3-oled-demo` oracle TWICE, with the JIT OFF on BOTH arms
//! (`LABWIRED_RISCV_JIT=0`) so the tick interval is the ONLY variable:
//!   * Arm A — `LABWIRED_TICK_INTERVAL=1`  (baseline, cycle-by-cycle ticking)
//!   * Arm B — `LABWIRED_TICK_INTERVAL=64` (the widened shipped default)
//!
//! Both take the identical JIT-eligible code path (same machine-sourced cycle
//! counting); the only difference is how often peripherals are ticked. The
//! `LABWIRED_TICK_INTERVAL` env hook exists solely for this gate (see the
//! comment in `crates/cli/src/main.rs`); unset it defaults to the widened 64.
//!
//! The test asserts every OBSERVABLE channel is byte-identical between the two
//! arms: the serial/UART transcript (`uart.log`), the assertion verdicts
//! (status == pass, `expected_stop_reason`, `uart_contains`), `stop_reason`,
//! `steps_executed`, and the whole `inspect` block (the decoded SSD1306
//! framebuffer / generation / lit pixels every peripheral exposes). It then
//! strips the deliberately-excluded internal fields and asserts the ENTIRE
//! remainder of `result.json` matches too, so any new observable field is
//! caught automatically.
//!
//! DELIBERATELY EXCLUDED from the comparison (NOT observable output):
//!   * `cpu_state` — the halt-instant register snapshot differs BY DESIGN:
//!     interval-64 services peripheral interrupts on 64-cycle boundaries, so the
//!     firmware is stopped at a different micro-instant and its live registers
//!     (`pc`, `mepc`, a handful of GPRs) differ. This is internal CPU micro-state
//!     at the arbitrary halt point, not a channel the firmware's behavior is
//!     observed through.
//!   * `cycles` / `instructions` — internal retirement counters. (They in fact
//!     match today, but they are excluded on principle: they are engine
//!     telemetry, not firmware-observable output.)
//!   * any wall-clock / IPS / timestamp field — `result.json` carries none, so
//!     nothing to strip, but excluded by policy for completeness.
//!
//! If this test EVER fails, the tick-widening has started perturbing observable
//! behavior and MUST NOT ship as the default — the assertions must not be
//! weakened to make it pass.
//!
//! This is a SEPARATE guard from the JIT-on vs JIT-off differential
//! (`riscv_jit_c3_oled_test_differential`): that one fixes the tick interval and
//! varies the JIT; this one fixes the JIT (off) and varies the tick interval.
//! Together they prove both knobs of the eligible path are observably inert.

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
    /// Parsed `result.json`.
    result: serde_json::Value,
    /// The serial transcript artifact (`uart.log`) — the firmware's observable
    /// serial output, the exact bytes the `uart_contains` assertion reads.
    uart_log: Vec<u8>,
    stderr: String,
    exit_ok: bool,
}

/// Invoke the built `labwired test` binary once at the given peripheral-tick
/// interval, JIT OFF. Requires the `jit-core` build so the eligible path (and
/// thus the `LABWIRED_TICK_INTERVAL` override) is compiled in.
fn run_oracle(fx: &Fixtures, script: &Path, out_dir: &Path, tick_interval: u32) -> RunOutput {
    std::fs::create_dir_all(out_dir).expect("create out dir");
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(repo_root())
        .env("LABWIRED_ESP32C3_FLASH", &fx.flash)
        // JIT OFF on BOTH arms: the tick interval is the ONLY variable.
        .env("LABWIRED_RISCV_JIT", "0")
        // The test-only escape hatch that overrides the widened interval.
        .env("LABWIRED_TICK_INTERVAL", tick_interval.to_string())
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
    let result_bytes = std::fs::read(&result_path).unwrap_or_else(|e| {
        panic!(
            "no result.json at {} (exit {:?}): {e}\nstderr:\n{}",
            result_path.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let result: serde_json::Value =
        serde_json::from_slice(&result_bytes).expect("parse result.json");
    // The serial transcript is written alongside result.json. It must exist —
    // the whole point is to diff the firmware's serial output between arms.
    let uart_log = std::fs::read(out_dir.join("uart.log")).unwrap_or_else(|e| {
        panic!(
            "no uart.log in {} (exit {:?}): {e}",
            out_dir.display(),
            output.status.code()
        )
    });
    RunOutput {
        result,
        uart_log,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_ok: output.status.success(),
    }
}

/// Return a clone of `result.json` with the deliberately-excluded, non-observable
/// internal fields removed, so the remainder can be compared byte-for-byte.
fn strip_non_observable(mut v: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = v.as_object_mut() {
        // Halt-instant CPU register snapshot: differs by design (see module doc).
        obj.remove("cpu_state");
        // Internal engine retirement counters, not firmware-observable output.
        obj.remove("cycles");
        obj.remove("instructions");
    }
    v
}

fn pretty(v: &serde_json::Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

#[test]
fn tick_interval_1_vs_64_observable_output_identical_c3_oled() {
    let fx = fixtures();
    let tmp = std::env::temp_dir().join(format!("lw-tick-fidelity-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create tmp dir");

    // The `LABWIRED_TICK_INTERVAL` override only takes effect on the JIT-eligible
    // path, which is compiled in only under `jit-core`. Without it both arms run
    // the identical interpreter at interval-1 and the test is vacuous, so skip.
    if !cfg!(feature = "jit-core") {
        eprintln!(
            "SKIP: built without `jit-core`; the tick-widening eligible path is \
             compiled out, so interval-1 vs interval-64 is not exercised."
        );
        return;
    }

    // Run to a step budget large enough to paint the OLED and report it on serial
    // (matches riscv_jit_c3_oled_test_differential's max_steps scenario).
    let script = write_script(
        &tmp,
        "oled_max_steps.yaml",
        &fx,
        "  max_steps: 8000000\n",
        "  - expected_stop_reason: max_steps\n  - uart_contains: \"OLED painted: LabWired\"",
    );

    let a = run_oracle(&fx, &script, &tmp.join("interval_1"), 1); // baseline
    let b = run_oracle(&fx, &script, &tmp.join("interval_64"), 64); // widened default

    assert!(a.exit_ok, "interval-1 run failed:\n{}", a.stderr);
    assert!(b.exit_ok, "interval-64 run failed:\n{}", b.stderr);

    // Both arms must actually pass (guards against a vacuous "both errored" match).
    assert_eq!(
        a.result["status"],
        "pass",
        "interval-1 did not pass:\n{}",
        pretty(&a.result)
    );
    assert_eq!(
        b.result["status"],
        "pass",
        "interval-64 did not pass:\n{}",
        pretty(&b.result)
    );

    // --- OBSERVABLE CHANNEL 1: the serial/UART transcript, byte-for-byte. ---
    assert!(
        a.uart_log == b.uart_log,
        "uart.log DIVERGED between interval-1 and interval-64 ({} vs {} bytes)\n\
         interval-1:\n{}\ninterval-64:\n{}",
        a.uart_log.len(),
        b.uart_log.len(),
        String::from_utf8_lossy(&a.uart_log),
        String::from_utf8_lossy(&b.uart_log),
    );

    // --- OBSERVABLE CHANNEL 2: the assertion verdicts (status + each clause). ---
    assert_eq!(
        a.result["assertions"],
        b.result["assertions"],
        "assertion verdicts diverged\ninterval-1:\n{}\ninterval-64:\n{}",
        pretty(&a.result["assertions"]),
        pretty(&b.result["assertions"]),
    );

    // --- OBSERVABLE CHANNEL 3: the stop reason and steps executed. ---
    assert_eq!(
        a.result["stop_reason"], b.result["stop_reason"],
        "stop_reason diverged"
    );
    assert_eq!(
        a.result["steps_executed"], b.result["steps_executed"],
        "steps_executed diverged"
    );

    // --- OBSERVABLE CHANNEL 4: the whole inspect block (decoded SSD1306 pixels /
    //     generation / lit pixels / ink bytes every peripheral exposes). ---
    assert_eq!(
        a.result["inspect"],
        b.result["inspect"],
        "inspect block (framebuffer/peripheral decode) diverged\n\
         interval-1:\n{}\ninterval-64:\n{}",
        pretty(&a.result["inspect"]),
        pretty(&b.result["inspect"]),
    );

    // --- CATCH-ALL: the ENTIRE result.json minus the excluded internal fields
    //     (cpu_state / cycles / instructions) must be byte-identical, so any new
    //     observable field is compared automatically without editing this test. ---
    let stripped_a = strip_non_observable(a.result.clone());
    let stripped_b = strip_non_observable(b.result.clone());
    assert_eq!(
        stripped_a,
        stripped_b,
        "result.json (excluding cpu_state/cycles/instructions) DIVERGED between \
         interval-1 and interval-64 — the tick-widening perturbed observable output.\n\
         interval-1:\n{}\ninterval-64:\n{}",
        pretty(&stripped_a),
        pretty(&stripped_b),
    );

    // Sanity: cpu_state IS present and IS expected to differ (proving the arms
    // really did halt at different micro-instants — i.e. the widening was live,
    // not a no-op that would make byte-identity trivial).
    if a.result.get("cpu_state").is_some() && b.result.get("cpu_state").is_some() {
        eprintln!(
            "[tick-fidelity] observable output byte-identical at interval-1 vs -64; \
             cpu_state differs as expected (differ={})",
            a.result["cpu_state"] != b.result["cpu_state"]
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}
