use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_inspect(limit: Option<usize>) -> Value {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!(
        "labwired-inspect-bytes-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let system = dir.join("system.yaml");
    std::fs::write(
        &system,
        format!(
            "name: inspect-display\nchip: {:?}\nexternal_devices:\n  - id: oled\n    type: oled-ssd1306\n    connection: i2c1\n    config:\n      i2c_address: 0x3c\n",
            root.join("configs/chips/stm32f407.yaml")
        ),
    )
    .unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: {:?}\n  system: {:?}\nlimits:\n  max_steps: 8\nassertions: []\n",
            root.join("tests/fixtures/uart-ok-thumbv7m.elf"),
            system
        ),
    )
    .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_labwired"));
    cmd.args(["test", "--script"])
        .arg(&script)
        .arg("--output-dir")
        .arg(&dir)
        .args(["--no-key", "--no-uart-stdout"]);
    if let Some(limit) = limit {
        cmd.arg("--inspect-bytes-max").arg(limit.to_string());
    }
    let output = cmd.output().unwrap();
    let result = std::fs::read(dir.join("result.json"));
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&result.expect("CLI writes result.json")).unwrap()
}

fn framebuffer(result: &Value) -> &Value {
    result["inspect"]["devices"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|d| d["artifacts"].as_array().unwrap())
        .find(|a| a["kind"] == "framebuffer")
        .expect("real attached SSD1306 exports a framebuffer")
}

#[test]
fn default_inspect_keeps_framebuffer_bytes_omitted() {
    let result = run_inspect(None);
    assert!(framebuffer(&result).get("bytes").is_none());
}

#[test]
fn opted_in_inspect_serializes_exact_ssd1306_framebuffer() {
    let result = run_inspect(Some(4096));
    let bytes = framebuffer(&result)["bytes"]
        .as_array()
        .expect("opt-in exports the actual SSD1306 framebuffer bytes");
    assert_eq!(bytes.len(), 128 * 64 / 8);
    assert!(bytes.iter().all(|b| b.as_u64() == Some(0)));
}

#[test]
fn over_budget_framebuffer_is_explicitly_omitted_without_truncation() {
    let result = run_inspect(Some(1));
    let fb = framebuffer(&result);
    assert!(fb.get("bytes").is_none());
    assert_eq!(fb["meta"]["bytes_omitted_reason"], "inspect_bytes_limit");
    assert_eq!(fb["meta"]["byte_length"], 1024);
}

#[test]
fn byte_export_cannot_exceed_one_mib() {
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--script",
            "unused.yaml",
            "--inspect-bytes-max",
            "1048577",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("1048576"));
}
