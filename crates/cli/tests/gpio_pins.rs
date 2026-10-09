// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `result.json`'s universal `gpio_pins` block on the real `labwired` binary.
//!
//! Runs the STM32F401 H-bridge example (IN1/IN2 push-pull outputs on PA0/PA1,
//! ENA as TIM3 CH1 alternate function on PA6) with those pads watched, and
//! checks the block reports each pad through the chip-agnostic accessors.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn run_watched(watch: &[&str]) -> serde_json::Value {
    let root = repo_root();
    let script = root.join("examples/f401-hbridge-dc-motor/hbridge.yaml");
    assert!(script.exists(), "missing fixture: {}", script.display());
    let out = std::env::temp_dir().join(format!(
        "lw-gpio-pins-{}-{}",
        std::process::id(),
        watch.len()
    ));
    let _ = std::fs::remove_dir_all(&out);
    let mut args = vec![
        "test".to_string(),
        "--script".into(),
        script.to_str().unwrap().into(),
        "--no-uart-stdout".into(),
        "--no-key".into(),
        "--output-dir".into(),
        out.to_str().unwrap().into(),
    ];
    for w in watch {
        args.push("--watch-gpio".into());
        args.push((*w).into());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .args(&args)
        .output()
        .expect("spawn labwired");
    let text = std::fs::read_to_string(out.join("result.json")).unwrap_or_else(|e| {
        panic!(
            "no result.json ({e}); stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let _ = std::fs::remove_dir_all(&out);
    serde_json::from_str(&text).expect("result.json parses")
}

#[test]
fn watched_pins_report_mode_levels_and_drive() {
    let parsed = run_watched(&["gpioa:0", "gpioa:1", "gpioa:6"]);
    let pins = parsed["gpio_pins"].as_array().expect("gpio_pins block");
    assert_eq!(pins.len(), 3, "{parsed:#}");
    println!(
        "{}",
        serde_json::to_string_pretty(&parsed["gpio_pins"]).unwrap()
    );

    for (i, p) in pins.iter().enumerate() {
        assert_eq!(p["pin"], format!("CH{i}"), "{p}");
        for k in ["mode", "func", "output", "input", "pad", "drive"] {
            assert!(p.get(k).is_some(), "key {k} missing: {p}");
        }
    }
    // PA0/PA1 are push-pull outputs; the reverse leg leaves IN2 high.
    assert_eq!(pins[0]["mode"], "output", "{}", pins[0]);
    assert_eq!(pins[1]["mode"], "output", "{}", pins[1]);
    assert_eq!(pins[1]["output"], true, "{}", pins[1]);
    assert_eq!(pins[1]["pad"], true, "{}", pins[1]);
    assert_eq!(pins[1]["drive"], "1", "{}", pins[1]);
    assert_eq!(pins[0]["output"], false, "{}", pins[0]);
    assert_eq!(pins[0]["drive"], "0", "{}", pins[0]);
    // PA6 is handed to TIM3.
    assert_eq!(pins[2]["mode"], "af", "{}", pins[2]);

    // logic_edges carries the same mode/func additively; nothing renamed.
    let ch = parsed["logic_edges"]["channels"].as_array().unwrap();
    assert_eq!(ch[0]["mode"], "output");
    assert_eq!(ch[2]["mode"], "af");
    assert!(ch[0]["transitions"].is_array());
    assert_eq!(ch[0]["channel"], "CH0");
}

#[test]
fn no_watched_pins_omits_the_block() {
    // The hbridge script auto-arms nothing without a gpio_edges clause, so a
    // run with no `--watch-gpio` carries no `gpio_pins`.
    let parsed = run_watched(&[]);
    assert!(parsed.get("gpio_pins").is_none(), "{parsed:#}");
}
