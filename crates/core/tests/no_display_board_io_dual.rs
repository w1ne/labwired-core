// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Displays must not be declared twice (`external_devices` + `board_io`).
//!
//! `WasmSimulator::display_artifact` resolves panels from `external_devices`
//! first. A second `board_io` copy is a second home that drifts (wrong
//! device_type, stale pins). Sensor duals remain temporarily for
//! `get_i2c_sensor_states` — shrink that list, do not grow displays.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// Device types that are display / panel models — dual board_io is forbidden.
const DISPLAY_TYPES: &[&str] = &[
    "oled-ssd1306",
    "oled-ssd1306-128x32",
    "oled-sh1107",
    "ssd1680_tricolor_290",
    "uc8151d_tricolor_290",
    "pcd8544",
    "ili9341",
    "ssd1306",
    "sh1107",
];

fn workspace_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.parent().and_then(|p| p.parent()).unwrap().to_path_buf()
}

#[test]
fn system_yamls_do_not_dual_declare_displays_in_board_io() {
    let systems = workspace_root().join("configs/systems");
    let display: HashSet<&str> = DISPLAY_TYPES.iter().copied().collect();
    let mut offenders = Vec::new();

    for entry in fs::read_dir(&systems).expect("configs/systems") {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let Some(bio_start) = text.find("board_io:") else {
            continue;
        };
        let bio = &text[bio_start..];
        for cap in regex_lite_device_types(bio) {
            if display.contains(cap.as_str()) {
                offenders.push(format!(
                    "{}: board_io device_type '{}' (use external_devices only)",
                    path.file_name().unwrap().to_string_lossy(),
                    cap
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "display dual board_io found:\n  - {}",
        offenders.join("\n  - ")
    );
}

/// Minimal scan: `device_type: foo` lines (no extra regex crate dependency).
fn regex_lite_device_types(section: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in section.lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("device_type:") else {
            continue;
        };
        let rest = rest.trim().trim_matches('"').trim_matches('\'');
        if !rest.is_empty() && !rest.starts_with('#') {
            out.push(rest.to_string());
        }
    }
    out
}
