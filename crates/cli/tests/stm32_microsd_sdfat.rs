// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A real SD driver mounts and writes the `microsd` twin (issue #1951).
//!
//! `fixtures/stm32f411_microsd/sdfat_raw.elf` is SdFat's raw SPI card driver
//! on the F411 Black Pill (see the `.ino`): init, CSD, single-block and
//! multi-block write and read-back. The command shell this replaced answered
//! CMD0 with 0xFF, so the sketch printed `INIT FAIL code=0x1 data=0xFF`.
//! The written blocks must also be visible to an oracle through the `sd_card`
//! inspect artifact in `result.json`.

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

#[test]
fn sdfat_initialises_writes_and_reads_back_the_card() {
    let root = workspace_root();
    let fixture = root.join("crates/cli/tests/fixtures/stm32f411_microsd");
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-microsd-sdfat");
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\n\
             inputs:\n  firmware: \"{}\"\n  system: \"{}\"\n\
             limits:\n  max_steps: 50000000\n  stop_when_assertions_pass: true\n\
             assertions:\n  - uart_contains: \"SD DONE\"\n",
            fixture.join("sdfat_raw.elf").display(),
            fixture.join("system.yaml").display(),
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args(["test", "--script"])
        .arg(&script)
        .arg("--output-dir")
        .arg(&dir)
        .args(["--no-uart-stdout", "--inspect-bytes-max", "4096"])
        .output()
        .expect("run labwired");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let uart = std::fs::read_to_string(dir.join("uart.log")).unwrap_or_default();
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("result.json"))
            .unwrap_or_else(|_| panic!("no result.json; stderr:\n{stderr}")),
    )
    .unwrap();

    for line in [
        "INIT OK type=3 sectors=2097152",
        "SINGLE OK",
        "MULTI OK",
        "SD DONE",
    ] {
        assert!(uart.contains(line), "missing {line:?} in UART:\n{uart}");
    }

    // The oracle view: the card's written blocks in the final inspect block.
    let devices = result["inspect"]["devices"]
        .as_array()
        .unwrap_or_else(|| panic!("no inspect.devices in result.json: {}", result["inspect"]));
    let sd = devices
        .iter()
        .flat_map(|d| d["artifacts"].as_array().into_iter().flatten())
        .find(|a| a["kind"] == "sd_card")
        .unwrap_or_else(|| panic!("no sd_card artifact: {devices:?}"));
    assert_eq!(
        sd["meta"]["written_lbas"],
        serde_json::json!([100, 200, 201, 202])
    );
    // `--inspect-bytes-max` includes the contents, in `written_lbas` order.
    let bytes: Vec<u8> = sd["bytes"]
        .as_array()
        .unwrap_or_else(|| panic!("no sd_card bytes: {}", sd["meta"]))
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    assert_eq!(bytes.len(), 4 * 512);
    assert_eq!(&bytes[..24], b"PATHFINDER FLIGHT LOG v1");
    for (i, lba) in (200u32..203).enumerate() {
        let block = &bytes[(i + 1) * 512..(i + 2) * 512];
        let want: Vec<u8> = (0..512u32).map(|j| (lba * 7 + j) as u8).collect();
        assert_eq!(block, &want[..], "LBA {lba}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
