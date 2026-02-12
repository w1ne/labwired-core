use std::path::PathBuf;
use std::process::Command;

#[test]
fn test_cli_json_metrics() {
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let bin_path = root.join("target/debug/labwired");
    let elf_path = root.join("tests/fixtures/uart-ok-thumbv7m.elf");

    let mut cmd = Command::new(bin_path);
    cmd.args(["--firmware", elf_path.to_str().unwrap(), "--json"]);

    let output = cmd.output().expect("Failed to execute command");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Find the last JSON line (the performance report)
    let last_json = stdout
        .lines()
        .rfind(|l| l.contains("\"status\":\"finished\""));
    assert!(
        last_json.is_some(),
        "Performance report JSON not found in output. Stdout: {}",
        stdout
    );

    let json: serde_json::Value =
        serde_json::from_str(last_json.unwrap()).expect("Failed to parse JSON");
    assert_eq!(json["status"], "finished");
    assert!(json["total_cycles"].as_u64().unwrap() > 0);
}

#[test]
fn test_cli_vcd_generation() {
    let vcd_path = "integration_test.vcd";
    if PathBuf::from(vcd_path).exists() {
        std::fs::remove_file(vcd_path).ok();
    }

    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let bin_path = root.join("target/debug/labwired");
    let elf_path = root.join("tests/fixtures/uart-ok-thumbv7m.elf");

    let mut cmd = Command::new(bin_path);
    cmd.args(["--firmware", elf_path.to_str().unwrap(), "--vcd", vcd_path]);

    let output = cmd.output().expect("Failed to execute command");
    assert!(output.status.success());

    let path = PathBuf::from(vcd_path);
    assert!(path.exists(), "VCD file was not generated");

    let content = std::fs::read_to_string(&path).expect("Failed to read VCD");
    assert!(content.contains("$timescale"), "VCD header missing");
    assert!(
        content.contains("$var wire 32"),
        "VCD signal definitions missing"
    );

    std::fs::remove_file(vcd_path).ok();
}

#[test]
fn test_asset_init() {
    let output_dir = "test-project-init";
    if PathBuf::from(output_dir).exists() {
        std::fs::remove_dir_all(output_dir).ok();
    }

    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let bin_path = root.join("target/debug/labwired");

    let mut cmd = Command::new(bin_path);
    cmd.args(["asset", "init", "-o", output_dir]);

    let output = cmd.output().expect("Failed to execute command");
    assert!(output.status.success());

    let system_yaml = PathBuf::from(output_dir).join("system.yaml");
    assert!(system_yaml.exists());

    let content = std::fs::read_to_string(system_yaml).expect("Failed to read system.yaml");
    assert!(content.contains("chip: \"stm32f103.yaml\""));

    std::fs::remove_dir_all(output_dir).ok();
}
