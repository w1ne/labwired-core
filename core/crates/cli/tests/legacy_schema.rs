// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("Failed to canonicalize repo root")
}

fn write_temp_file(prefix: &str, contents: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push("labwired-tests");
    let _ = std::fs::create_dir_all(&dir);

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = dir.join(format!("{}-{}.yaml", prefix, nonce));
    std::fs::write(&path, contents).expect("Failed to write temp file");
    path
}

#[test]
fn legacy_schema_v1_is_still_accepted_with_warning() {
    let root = repo_root();
    let script = write_temp_file(
        "legacy-schema-v1",
        r#"
schema_version: 1
max_steps: 0
assertions: []
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .args([
            "test",
            "--firmware",
            "tests/fixtures/uart-ok-thumbv7m.elf",
            "--script",
            script.to_str().unwrap(),
            "--no-uart-stdout",
        ])
        .output()
        .expect("Failed to execute labwired");

    assert_eq!(output.status.code(), Some(0));
}
