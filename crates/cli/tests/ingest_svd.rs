// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Integration test for `labwired asset ingest-svd` — the one-step agent path
//! from a vendor SVD to runnable declarative `PeripheralDescriptor` YAML.
//!
//! Runnability of the emitted descriptors is covered separately by the
//! declarative loader + `register_coverage` tests, which consume the exact same
//! `PeripheralDescriptor` format produced here.

use std::path::PathBuf;
use std::process::Command;

const SVD: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/test_device.svd"
);

fn out_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lw_ingest_svd_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn ingest_svd_json_emits_descriptor_paths_and_register_counts() {
    let out = out_dir("json");
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "asset",
            "ingest-svd",
            "--input",
            SVD,
            "--output-dir",
            out.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("run labwired");
    assert!(
        output.status.success(),
        "non-zero exit: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout is JSON");
    assert_eq!(json["peripheral_count"], 1, "json: {json}");
    let p = &json["peripherals"][0];
    assert_eq!(p["name"], "GPIOA");
    assert_eq!(p["register_count"], 1);

    // The descriptor file exists and is valid declarative YAML.
    let yaml_path = out.join("gpioa.yaml");
    let yaml = std::fs::read_to_string(&yaml_path).expect("descriptor written");
    assert!(yaml.contains("peripheral: GPIOA"), "yaml: {yaml}");
    assert!(yaml.contains("registers:"), "yaml: {yaml}");

    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn ingest_svd_filter_no_match_exits_config_error() {
    let out = out_dir("filter");
    let status = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "asset",
            "ingest-svd",
            "--input",
            SVD,
            "--output-dir",
            out.to_str().unwrap(),
            "--filter",
            "DEFINITELY_NOT_A_PERIPHERAL",
            "--json",
        ])
        .status()
        .expect("run labwired");
    // EXIT_CONFIG_ERROR = 2
    assert_eq!(status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn ingest_svd_filter_match_is_case_insensitive() {
    let out = out_dir("ci");
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "asset",
            "ingest-svd",
            "--input",
            SVD,
            "--output-dir",
            out.to_str().unwrap(),
            "--filter",
            "gpioa", // lower-case; SVD names it GPIOA
            "--json",
        ])
        .output()
        .expect("run labwired");
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&output.stdout).trim()).unwrap();
    assert_eq!(json["peripheral_count"], 1);
    let _ = std::fs::remove_dir_all(&out);
}
