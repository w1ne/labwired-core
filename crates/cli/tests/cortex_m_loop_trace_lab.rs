// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Loop traces change what a hot ALU loop costs, not what it does.
//!
//! `fixtures/rp2040_loop_lab` is the shipped demo's hot shape: 200 rounds
//! of `bl leaf; adds r0, #1; subs r4, #1; bne head` (leaf = `adds; bx lr`),
//! a UART report of the results, then a park. The same firmware runs three
//! ways — pure interpreter, Cortex-M JIT without traces (one basic block per
//! dispatch) and JIT with loop traces (one counted loop per dispatch) — and
//! all three must report exactly the same result, UART output and machine
//! state, modulo `metrics.pc_samples` (sampled per retired step; a trace
//! retires many instructions per dispatch, so its sampling grid moves) and
//! the script path (each run writes its own).
//!
//! The trace lane is the interesting one: it is the only path where an
//! emitted wasm loop runs the body and its epilogue decides where the
//! machine resumes, so a divergence shows up as a different pc, register or
//! UART byte rather than a crash.

#![cfg(feature = "jit-core")]

use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Copy)]
enum Lane {
    Interpreter,
    BlockJit,
    TraceJit,
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run(name: &str, lane: Lane) -> (serde_json::Value, Vec<u8>) {
    let root = workspace_root();
    let dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-loop-lab-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: \"{}\"\n  system: \"{}\"\nlimits:\n  max_steps: 1000000\n  max_cycles: 400000\n",
            root.join("crates/cli/tests/fixtures/rp2040_loop_lab/loop_lab.elf")
                .display(),
            root.join("configs/systems/rp2040-pico.yaml").display(),
        ),
    )
    .unwrap();
    let (jit, traces) = match lane {
        Lane::Interpreter => ("0", "0"),
        Lane::BlockJit => ("1", "0"),
        Lane::TraceJit => ("1", "1"),
    };
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args(["test", "--script"])
        .arg(&script)
        .arg("--output-dir")
        .arg(&dir)
        .arg("--no-uart-stdout")
        .env("LABWIRED_CORTEX_M_JIT", jit)
        .env("LABWIRED_CORTEX_M_JIT_TRACES", traces)
        .output()
        .expect("run labwired");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let result = std::fs::read_to_string(dir.join("result.json"))
        .unwrap_or_else(|_| panic!("no result.json; stderr:\n{stderr}"));
    let mut result: serde_json::Value = serde_json::from_str(&result).unwrap();
    without_bookkeeping(&mut result);
    (result, std::fs::read(dir.join("uart.log")).unwrap())
}

/// Each run writes its script in its own temp dir, and the PC histogram is
/// sampled per retired primary step, so neither says anything about behaviour.
fn without_bookkeeping(result: &mut serde_json::Value) {
    result["config"]
        .as_object_mut()
        .expect("config")
        .remove("script");
    if let Some(metrics) = result["metrics"].as_object_mut() {
        metrics.remove("pc_samples");
    }
}

#[test]
fn a_traced_loop_lab_reports_the_same_run() {
    let (interp, interp_uart) = run("interp", Lane::Interpreter);
    let (block, block_uart) = run("block", Lane::BlockJit);
    let (trace, trace_uart) = run("trace", Lane::TraceJit);

    assert_eq!(
        trace, block,
        "the loop trace changed the run relative to the basic-block JIT"
    );
    assert_eq!(
        trace, interp,
        "the loop trace changed the run relative to the interpreter"
    );
    assert_eq!(
        trace_uart,
        vec![0x90, 0x01, 0x00],
        "the fixture's UART report: r0 = 400 (0x190, low byte), r1 = 1, r4 = 0"
    );
    assert_eq!(interp_uart, trace_uart, "UART output differs by lane");
    assert_eq!(block_uart, trace_uart, "UART output differs by lane");

    // The fixture's own meaning, so a run that silently did nothing cannot
    // pass by agreeing with itself: 200 rounds, two `adds r0` each.
    let registers = &trace["cpu_state"]["registers"];
    assert_eq!(registers[0], 400, "r0 accumulates 2 per round");
    assert_eq!(registers[1], 1, "the loop exited and set the marker");
    assert_eq!(registers[4], 0, "the counter reached zero");
    assert_eq!(trace["cycles"], 400000);
}
