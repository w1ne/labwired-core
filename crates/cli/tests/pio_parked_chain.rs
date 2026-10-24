// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A parked PIO state machine that stops its step chain changes no result.
//!
//! `fixtures/pio_parked_chain` parks RP2040 PIO0 SM0 on `jmp 0`, writes the
//! TX FIFO while it is parked, runs a `jmp 1` / `jmp 0` ping-pong at clock
//! divider 1 and 2, and parks again, sending FSTAT and SM0_ADDR to UART0 at
//! fixed cycle offsets. The run with the park must report exactly what the
//! run with a per-cycle step chain (`LABWIRED_PIO_PARK=0`) reports, except
//! `metrics.pc_samples` (sampled per retired primary step; a stopped chain
//! shifts the sampling grid without retiring a different instruction). The
//! ping-pong is the negative control's target: a park test that ignored the
//! jump target would freeze SM0_ADDR there.

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
    let dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-pio-park-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: \"{}\"\n  system: \"{}\"\nlimits:\n  max_steps: 10000000\n  max_cycles: 400000\n",
            root.join("crates/cli/tests/fixtures/pio_parked_chain/pio_park.elf").display(),
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
fn a_parked_pio_chain_reports_the_same_run() {
    let (parked, parked_uart) = run("park", true);
    let (reference, reference_uart) = run("ref", false);
    // 2 + 2 + 2 + 8 + 2 + 8 + 2 report bytes.
    assert_eq!(parked_uart.len(), 26, "uart: {parked_uart:02x?}");
    assert!(
        parked_uart[16..26].contains(&1) && parked_uart[6..14].contains(&1),
        "SM0 never left pc 0 in a ping-pong: {parked_uart:02x?}"
    );
    assert_eq!(
        parked_uart, reference_uart,
        "the park changed the UART bytes"
    );
    assert_eq!(parked, reference, "the park changed the run");
}
