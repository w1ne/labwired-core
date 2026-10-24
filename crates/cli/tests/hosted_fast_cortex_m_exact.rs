// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The Cortex-M bare-ELF fast path of `labwired test` changes no observable
//! result.
//!
//! The fast path (`LABWIRED_HOSTED_FAST`, on by default) takes metrics from
//! the machine's counters instead of a per-step observer and plans CPU windows
//! at the bus's max-safe interval with exact boundaries
//! (`Machine::exact_batch_boundaries`). Each tier-1 fixture runs with it on and
//! off; result.json must be equal except `cycles`, `instructions` and
//! `metrics`, which count work: the observer over-counts Cortex-M cycles (the
//! fast path reports the machine clock) and does not count a step that only
//! enters an exception (the machine's counter does). `max_steps` already
//! counts machine steps on both paths, so where a run stops does not move. The fixtures drive UART, timers and interrupts, which is
//! where a late interrupt or a quantised peripheral cadence would show.

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

fn run(
    name: &str,
    fixture: &str,
    system: &str,
    cycles: u64,
    fast: bool,
) -> (serde_json::Value, String, Vec<u8>) {
    let root = workspace_root();
    let dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-hosted-fast-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: \"{}\"\n  system: \"{}\"\nlimits:\n  max_steps: 50000000\n  max_cycles: {cycles}\n",
            root.join("tests/fixtures/tier1").join(fixture).display(),
            root.join("configs/systems").join(system).display(),
        ),
    )
    .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_labwired"));
    cmd.args(["test", "--script"])
        .arg(&script)
        .arg("--output-dir")
        .arg(&dir)
        .arg("--no-uart-stdout")
        .env("LABWIRED_HOSTED_FAST", if fast { "1" } else { "0" });
    let output = cmd.output().expect("run labwired");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let result = std::fs::read_to_string(dir.join("result.json"))
        .unwrap_or_else(|_| panic!("no result.json for {name}; stderr:\n{stderr}"));
    let uart = std::fs::read(dir.join("uart.log")).unwrap_or_default();
    (serde_json::from_str(&result).unwrap(), stderr, uart)
}

fn observable(mut r: serde_json::Value) -> serde_json::Value {
    let o = r.as_object_mut().unwrap();
    o.remove("cycles");
    o.remove("instructions");
    o.remove("metrics");
    o.get_mut("config")
        .and_then(|c| c.as_object_mut())
        .map(|c| c.remove("script"));
    r
}

#[test]
fn the_cortex_m_fast_path_reports_the_same_run() {
    // `wide`: the bus allows wide windows (its per-cycle tick has no work of
    // its own). nRF52 has the per-cycle Nordic GPIO/GPIOTE service, so it keeps
    // one-cycle windows and only gains the counter-sourced metrics.
    for (fixture, system, wide) in [
        ("stm32f401.elf", "nucleo-f401re.yaml", true),
        ("stm32l476.elf", "nucleo-l476rg.yaml", true),
        ("stm32g071.elf", "nucleo-g071rb.yaml", true),
        ("nrf52840.elf", "nrf52840-dk.yaml", false),
        ("rp2040.elf", "rp2040-pico.yaml", true),
    ] {
        let name = fixture.trim_end_matches(".elf");
        let (fast, fast_err, fast_uart) = run(name, fixture, system, 4_000_000, true);
        let (slow, _, slow_uart) = run(&format!("{name}-ref"), fixture, system, 4_000_000, false);
        assert_eq!(
            fast_err.contains("hosted fast path"),
            wide,
            "{name}: wide windows engaged where they should not, or not where they should:\n{fast_err}"
        );
        assert!(
            !slow_uart.is_empty(),
            "{name}: the fixture printed nothing, so the check proves little"
        );
        assert_eq!(fast_uart, slow_uart, "{name}: UART differs");
        assert_eq!(
            observable(fast),
            observable(slow),
            "{name}: result.json differs"
        );
    }
}
