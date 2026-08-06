// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Bus modules (I²C/SPI/UART kits) must not be declared twice.
//!
//! Identity and attach live in `external_devices:`. A second `board_io`
//! `device_type:` twin is a second home that drifts. Onboard LED/button
//! `board_io` (kind led/button/adc_input/…) is allowed.

use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .unwrap()
        .to_path_buf()
}

fn scan_yaml(path: &std::path::Path, offenders: &mut Vec<String>) {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return,
    };
    let Some(bio_start) = text.find("board_io:") else {
        return;
    };
    let bio = &text[bio_start..];
    // Bus-controller kinds in board_io are always dual debt — with or without
    // device_type. Onboard led/button/adc_input/pwm stay allowed.
    for line in bio.lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("kind:") else {
            continue;
        };
        let kind = rest.trim().trim_matches('"').trim_matches('\'');
        if matches!(kind, "i2c_device" | "spi_device" | "uart_device") {
            offenders.push(format!(
                "{}: board_io kind={kind} (use external_devices only)",
                path.file_name().unwrap().to_string_lossy(),
            ));
        }
    }
}

#[test]
fn no_bus_device_type_in_board_io_across_systems_and_examples() {
    let root = workspace_root();
    let mut offenders = Vec::new();
    for dir in ["configs/systems", "examples"] {
        let base = root.join(dir);
        if !base.exists() {
            continue;
        }
        for entry in walkdir_yaml(&base) {
            scan_yaml(&entry, &mut offenders);
        }
    }
    assert!(
        offenders.is_empty(),
        "bus dual board_io found:\n  - {}",
        offenders.join("\n  - ")
    );
}

fn walkdir_yaml(base: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|e| e.to_str()) == Some("yaml") {
                // system.yaml or configs/systems/*.yaml
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name == "system.yaml" || base.ends_with("systems") {
                    out.push(p);
                }
            }
        }
    }
    out
}
