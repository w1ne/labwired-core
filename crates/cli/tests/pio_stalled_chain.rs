// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A PIO state machine blocked on an empty TX FIFO sleeps without changing
//! any result.
//!
//! `fixtures/pio_stalled_chain` runs `pull block; out pins, 8; jmp 0` on
//! RP2040 PIO0 SM0, pads an unaligned number of cycles, writes a word to the
//! TX FIFO and counts how many MMIO polls pass before PINS shows it, for
//! clock dividers 1.0, 3.5 and 135.25. The count is the write-to-pins latency,
//! so it shifts when the sleeping chain replays the divider phase wrongly.
//! The run that sleeps must report exactly what the run with a per-cycle step
//! chain (`LABWIRED_PIO_PARK=0`) reports, except `metrics.pc_samples`.

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run(name: &str, park: bool) -> (serde_json::Value, Vec<u8>) {
    let root = workspace_root();
    let dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-pio-stall-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: \"{}\"\n  system: \"{}\"\nlimits:\n  max_steps: 10000000\n  max_cycles: 300000\n",
            root.join("crates/cli/tests/fixtures/pio_stalled_chain/pio_stall.elf").display(),
            root.join("configs/systems/rp2040-pico.yaml").display(),
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args(["test", "--script"])
        .arg(&script)
        .arg("--output-dir")
        .arg(&dir)
        .arg("--no-uart-stdout")
        .env("LABWIRED_PIO_PARK", if park { "1" } else { "0" })
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
    result["metrics"]
        .as_object_mut()
        .expect("metrics")
        .remove("pc_samples");
}

#[test]
fn a_stalled_pio_chain_reports_the_same_run() {
    let (slept, slept_uart) = run("sleep", true);
    let (reference, reference_uart) = run("ref", false);
    // 8 trials at each of 3 dividers.
    assert_eq!(slept_uart.len(), 24, "uart: {slept_uart:02x?}");
    // Negative control: the poll counts must actually probe the divider phase.
    // At 135.25 the latency spans many poll iterations and moves with the pad.
    assert!(
        slept_uart[16..24].iter().any(|b| *b != slept_uart[16]),
        "the divided trials never shifted: {slept_uart:02x?}"
    );
    assert_eq!(slept_uart, reference_uart, "sleeping changed the UART bytes");
    assert_eq!(slept, reference, "sleeping changed the run");
}
