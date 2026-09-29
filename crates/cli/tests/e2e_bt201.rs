// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// End to end: a BT201 Bluetooth module (`type: bt201`) on the UART of
// `firmware-uart-echo-fixture`, a Cortex-M3 program that echoes every byte it
// reads. The echo closes a real loop through the model:
//
//   phone --(uart_injections device: bt)--> module --UART--> firmware
//   firmware --echo--> module: an `AT` line is a command, other bytes go
//   back to the phone over BLE.
//
// So one run covers phone->MCU passthrough, AT command parsing and replies
// (`OK`, `ER+2`), MCU->phone passthrough, and the link state (the
// `ble_link` stimulus), all read from the model's `at` / `link` / `air`
// logs. The negative controls run the same firmware with the link or the
// device missing.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn ensure_fixture_built(root: &Path) -> PathBuf {
    let bin = labwired_core::test_support::target_dir()
        .join("thumbv6m-none-eabi/release/firmware-uart-echo-fixture");
    if bin.exists() {
        return bin;
    }
    let status = Command::new("cargo")
        .current_dir(root)
        .args([
            "build",
            "-p",
            "firmware-uart-echo-fixture",
            "--release",
            "--target",
            "thumbv6m-none-eabi",
        ])
        .status()
        .expect("execute cargo build");
    assert!(
        status.success(),
        "failed to build firmware-uart-echo-fixture"
    );
    bin
}

struct Run {
    exit_code: Option<i32>,
    stderr: String,
}

/// Run `labwired test` with `extra` (stimuli / injections / assertions)
/// spliced into a script for the echo firmware with a BT201 on `uart1`.
fn run(extra: &str) -> Run {
    let root = repo_root();
    let fw = ensure_fixture_built(&root);
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-bt201");
    std::fs::create_dir_all(&dir).expect("create out dir");
    let chip = root.join("configs/chips/ci-fixture-cortex-m3-uart1.yaml");
    let system = format!(
        r#"name: "bt201-echo"
chip: "{}"
external_devices:
  - id: "bt"
    type: "bt201"
    connection: "uart1"
    config:
      boot_ms: 20
      reply_delay_us: 1000
      status_period_ms: 0
      # The generic UART gives its peers 1 ms of device time per bus tick, so
      # the echo of one packet is spread over some ms: a long gap keeps it one.
      packet_gap_us: 50000
"#,
        chip.display()
    );
    std::fs::write(dir.join("system.yaml"), system).expect("write system");
    let script = format!(
        r#"schema_version: "1.2"
inputs:
  firmware: "{}"
  system: "./system.yaml"
limits:
  max_steps: 60000
{extra}
"#,
        fw.display()
    );
    let script_path = dir.join("script.yaml");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&dir)
        .args([
            "test",
            "--script",
            script_path.to_str().unwrap(),
            "--no-uart-stdout",
            "--output-dir",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute labwired");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        exit_code: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

/// The phone connects, then sends one AT line and one data string.
const PHONE: &str = r#"stimuli:
  - target: { component: "bt", channel: "ble_link" }
    trigger: !after_cycles { cycles: 10000 }
    value: 1
uart_injections:
  - uart: "uart1"
    device: "bt"
    bytes: "AT+BMLABWIRED\r\n"
    trigger: !after_cycles { cycles: 20000 }
  - uart: "uart1"
    device: "bt"
    bytes: "hello"
    trigger: !after_cycles { cycles: 40000 }
"#;

#[test]
fn phone_data_goes_through_the_firmware_and_back() {
    let r = run(&format!(
        r#"{PHONE}assertions:
  # The start-up block reached the firmware and came back: the version line
  # is not a command the module knows.
  - peripheral_log: {{peripheral: uart1, log: at, contains: "AT+VER2.3-20190517 -> ER+2"}}
  - peripheral_log: {{peripheral: uart1, log: link, contains: "TL+03 ble connected"}}
  # Phone -> module -> firmware -> module: an AT command, answered OK.
  - peripheral_log: {{peripheral: uart1, log: air, contains: "phone->mcu 41 54 2b 42 4d"}}
  - peripheral_log: {{peripheral: uart1, log: at, contains: "AT+BMLABWIRED -> OK"}}
  # The OK is data for the module on its way back, so the phone gets it.
  - peripheral_log: {{peripheral: uart1, log: air, contains: "mcu->phone 4f 4b 0d 0a"}}
  # Plain data both ways.
  - peripheral_log: {{peripheral: uart1, log: air, contains: "phone->mcu 68 65 6c 6c 6f"}}
  - peripheral_log: {{peripheral: uart1, log: air, contains: "mcu->phone 68 65 6c 6c 6f"}}
"#
    ));
    assert_eq!(r.exit_code, Some(0), "stderr: {}", r.stderr);
}

#[test]
fn without_a_ble_link_the_phone_data_is_dropped() {
    // Negative control: same script without the connect stimulus.
    let phone_no_link = PHONE.replace("value: 1", "value: 0");
    let r = run(&format!(
        r#"{phone_no_link}assertions:
  - peripheral_log: {{peripheral: uart1, log: air, contains: "phone->mcu dropped (no ble link) 68 65 6c 6c 6f"}}
  - peripheral_log: {{peripheral: uart1, log: air, contains: "mcu->phone 68 65 6c 6c 6f"}}
"#
    ));
    assert_eq!(
        r.exit_code,
        Some(1),
        "the echo must not reach the phone without a link; stderr: {}",
        r.stderr
    );
    assert!(
        r.stderr.contains("mcu->phone 68 65 6c 6c 6f"),
        "{}",
        r.stderr
    );
}

#[test]
fn a_device_that_is_not_attached_is_a_config_error() {
    let r = run(r#"uart_injections:
  - uart: "uart1"
    device: "nope"
    bytes: "x"
    trigger: !after_cycles { cycles: 1000 }
"#);
    assert_eq!(r.exit_code, Some(2), "stderr: {}", r.stderr);
    assert!(
        r.stderr
            .contains("no device 'nope' is attached to 'uart1' (attached: bt)"),
        "{}",
        r.stderr
    );
}

#[test]
fn a_wrong_expected_reply_fails() {
    // Negative control for the assertion path itself.
    let r = run(&format!(
        r#"{PHONE}assertions:
  - peripheral_log: {{peripheral: uart1, log: at, contains: "AT+BMLABWIRED -> ER+2"}}
"#
    ));
    assert_eq!(r.exit_code, Some(1), "stderr: {}", r.stderr);
}
