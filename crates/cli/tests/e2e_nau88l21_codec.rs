// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! NAU88L21 codec end to end: a bare-register firmware on an i.MX RT LPI2C
//! master reads the codec ID, writes its audio interface registers and reads
//! one back (`tests/fixtures/nau88l21-codec/codec.S`). The gate
//! `codec-smoke.yaml` checks the firmware's view (RAM) and the codec's own
//! logs, addressed by the device id `codec`.
//!
//! Negative controls: the same firmware with the codec strapped to the other
//! address (0x1B) gets NACKs and fails the gate; a wrong expected value
//! fails its check; an unknown log name is a config error that lists the
//! codec's logs; an address the CSB strap cannot select is a build error.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FIXTURE: &str = "../../tests/fixtures/nau88l21-codec";

fn fixture() -> PathBuf {
    std::fs::canonicalize(FIXTURE).unwrap()
}

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("labwired-tests")
        .join(labwired_cli::test_support::unique_name(name));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, script: &Path, system: Option<&Path>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_labwired"));
    cmd.arg("test").arg("--script").arg(script);
    if let Some(system) = system {
        cmd.arg("--system").arg(system);
    }
    cmd.args(["--no-uart-stdout", "--output-dir"])
        .arg(dir.join("out"))
        .env_remove("LABWIRED_STRICT_FIDELITY");
    cmd.output().unwrap()
}

fn result(dir: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(dir.join("out/result.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The gate script with `edit` applied (in the fixture directory, so its
/// relative paths resolve).
fn edited_script(dir: &Path, name: &str, edit: impl Fn(String) -> String) -> PathBuf {
    let text = std::fs::read_to_string(fixture().join("codec-smoke.yaml")).unwrap();
    let text = edit(text).replace(
        "../imxrt-lpi2c-nau88l21.elf",
        &fixture()
            .join("../imxrt-lpi2c-nau88l21.elf")
            .display()
            .to_string(),
    );
    let text = text.replace(
        "system: \"system.yaml\"",
        &format!("system: \"{}\"", fixture().join("system.yaml").display()),
    );
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn firmware_finds_and_configures_the_codec() {
    let dir = work_dir("nau88l21-pass");
    let out = run(&dir, &fixture().join("codec-smoke.yaml"), None);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let r = result(&dir);
    assert_eq!(r["status"], "pass");
    let checks = r["assertions"].as_array().unwrap();
    assert_eq!(checks.len(), 14);
    assert!(checks.iter().all(|c| c["passed"] == true), "{r:#}");
    // The codec is in `inspect` under its manifest id.
    let devices = r["inspect"]["devices"].as_array().unwrap();
    assert!(
        devices
            .iter()
            .any(|d| d["id"] == "codec" && d["attachment"]["bus"] == "lpi2c1"),
        "{devices:#?}"
    );
}

/// The codec answers only at its strap address: at 0x1B the firmware's
/// transfers to 0x54 NACK, it reads 0xFFFF, and the codec logs nothing.
#[test]
fn codec_at_the_other_strap_address_fails_the_gate() {
    let dir = work_dir("nau88l21-csb-low");
    let out = run(
        &dir,
        &fixture().join("codec-smoke.yaml"),
        Some(&fixture().join("system-csb-low.yaml")),
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let r = result(&dir);
    assert_eq!(r["status"], "fail");
    let passed: Vec<bool> = r["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["passed"].as_bool().unwrap())
        .collect();
    // Done marker, fidelity and stop reason still pass; every codec fact
    // fails.
    assert_eq!(
        passed,
        [
            true, false, false, false, false, false, false, false, false, false, false, false,
            true, true
        ],
        "{r:#}"
    );
    assert!(
        stderr(&out).contains("codec.writes has 0 line(s) with \"write 0x001c = 0x000e\""),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_wrong_expected_value_fails() {
    let dir = work_dir("nau88l21-wrong");
    let script = edited_script(&dir, "wrong.yaml", |t| {
        t.replace("expected_value: 0x1A20", "expected_value: 0x1A21")
            .replace("\"dai slave i2s 32-bit\"", "\"dai slave i2s 24-bit\"")
    });
    let out = run(&dir, &script, None);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let r = result(&dir);
    assert_eq!(r["assertions"][1]["passed"], false);
    assert_eq!(r["assertions"][9]["passed"], false);
    let failed = r["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["passed"] == false)
        .count();
    assert_eq!(failed, 2, "{r:#}");
}

#[test]
fn unknown_codec_log_is_a_config_error_that_lists_the_logs() {
    let dir = work_dir("nau88l21-no-log");
    let script = edited_script(&dir, "nolog.yaml", |t| {
        t.replace("log: reads,", "log: audio,")
    });
    let out = run(&dir, &script, None);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains(
            "peripheral 'codec' has no log 'audio'. Its logs: writes, reads, regs, state"
        ),
        "{}",
        stderr(&out)
    );
}

#[test]
fn an_address_the_strap_cannot_select_is_rejected() {
    let dir = work_dir("nau88l21-bad-address");
    let system = dir.join("system.yaml");
    std::fs::write(
        &system,
        std::fs::read_to_string(fixture().join("system.yaml"))
            .unwrap()
            .replace("i2c_address: 0x54", "i2c_address: 0x55")
            .replace(
                "chip: \"chip.yaml\"",
                &format!("chip: \"{}\"", fixture().join("chip.yaml").display()),
            ),
    )
    .unwrap();
    let out = run(&dir, &fixture().join("codec-smoke.yaml"), Some(&system));
    assert_ne!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("i2c_address 0x55 is not possible"),
        "{}",
        stderr(&out)
    );
}
