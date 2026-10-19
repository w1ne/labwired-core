// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! A CPU store to flash on a chip that declares `flash.cpu_writes: fault`,
//! end to end through `labwired test`.
//!
//! The fixture chip (`configs/chips/ci-fixture-stm32l476-readonly.yaml`) is a
//! faithful copy of the STM32L476 descriptor with `cpu_writes: fault` on
//! flash. The firmware (`fault-readonly-flash.elf`, FAULT_KIND 6) stores to
//! 0x0800_1000, inside flash, so the bus gate must raise a precise BusFault
//! whose verdict names the read-only region, and `result.json` must carry the
//! `memory_policy` block with the violation.

use std::path::{Path, PathBuf};
use std::process::Command;

const SYSTEM: &str = "../../configs/systems/ci-fixture-l476-readonly.yaml";
const FIXTURES: &str = "../../tests/fixtures/fault-verdict";

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("labwired-tests")
        .join(labwired_cli::test_support::unique_name(name));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `labwired test` on `firmware`; return (result.json, stderr).
fn run(name: &str, firmware: &Path, stop: &str) -> (serde_json::Value, String) {
    let dir = work_dir(name);
    let firmware = std::fs::canonicalize(firmware).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: \"{}\"\nlimits:\n  max_steps: 5000\n\
             assertions:\n  - expected_stop_reason: {stop}\n",
            firmware.display()
        ),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args(["test", "--system"])
        .arg(std::fs::canonicalize(SYSTEM).unwrap())
        .arg("--script")
        .arg(&script)
        .args(["--no-uart-stdout", "--output-dir"])
        .arg(dir.join("out"))
        .env_remove("LABWIRED_CORTEXM_FAULTS")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let text = std::fs::read_to_string(dir.join("out/result.json"))
        .unwrap_or_else(|e| panic!("no result.json ({e}); stderr:\n{stderr}"));
    (serde_json::from_str(&text).unwrap(), stderr)
}

fn fixture(kind: &str) -> PathBuf {
    Path::new(FIXTURES).join(format!("fault-{kind}.elf"))
}

/// Run a fixture and check the golden sentence, both in result.json and on
/// stderr. Returns the verdict block for further checks.
fn golden(kind: &str, stop: &str, sentence: &str) -> serde_json::Value {
    let (result, stderr) = run(kind, &fixture(kind), stop);
    let v = result
        .get("fault_verdict")
        .unwrap_or_else(|| panic!("{kind}: no fault_verdict in result.json; stderr:\n{stderr}"))
        .clone();
    assert_eq!(v["summary"], sentence, "{kind}: verdict sentence");
    assert!(
        stderr.contains(&format!("FAULT  {sentence}")),
        "{kind}: stderr must print the verdict; got:\n{stderr}"
    );
    v
}

#[test]
fn store_to_read_only_flash_is_named_in_the_verdict() {
    let v = golden(
        "readonly-flash",
        "max_steps",
        "Precise BusFault: data access at 0x0800_1000 (BFAR valid; store to read-only flash), \
         at PC 0x0800_006C in `sensor_read` (fault_fixture.c:61), called from `main`.",
    );
    assert_eq!(v["kind"], "bus_fault");
    assert_eq!(v["fault_address"], "0x08001000");
    assert_eq!(v["precise"], true);
    assert_eq!(v["fault_address_source"], "BFAR");
}

#[test]
fn result_json_carries_the_memory_policy_block() {
    let (result, _stderr) = run("readonly-flash", &fixture("readonly-flash"), "max_steps");
    let mp = result.get("memory_policy").expect("memory_policy block");
    assert_eq!(mp["flash"]["mode"], "fault");
    assert_eq!(mp["flash"]["violations"], 1);
    assert_eq!(mp["flash"]["first"]["addr"], 0x0800_1000u64);
    assert_eq!(mp["ram"]["mode"], "allow");
    assert_eq!(mp["ram"]["violations"], 0);
}
