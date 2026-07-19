// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// End-to-end coverage for the examples/esp32c3-leo-airquality demo: the Leo
// Health air-quality sensor (ESP32-C3 + Sensirion SCD41/SGP41/SPS30 + Vishay
// VEML7700 + SSD1306 OLED). The firmware runs the REAL, unmodified Sensirion
// vendor drivers on-target, decodes four sensors over the simulated C3 I²C0
// controller, prints a plain-language verdict, and renders that verdict to the
// OLED.
//
// These tests run the committed firmware ELF through the committed scenario
// scripts via the `labwired test` CLI, so they need no cross-compiler toolchain
// and exercise the exact path a user runs. They are what keeps the example from
// silently rotting: a regression in the C3 I²C engine, any sensor model, the
// kit wiring, or the OLED driver fails the merge gate here.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

struct LeoRun {
    exit_code: Option<i32>,
    stderr: String,
    uart: String,
    status: String,
}

/// Run one Leo scenario script (relative to the repo root) through the
/// `labwired test` CLI and collect its result + UART log.
fn run_leo(script_rel: &str) -> LeoRun {
    let root = repo_root();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // Scenario name in the dir: the two tests run in parallel threads, and a
    // clock-tick nonce collision would make them share one result.json
    // (interleaved writes -> unparseable JSON).
    let scenario = script_rel
        .rsplit('/')
        .next()
        .unwrap_or(script_rel)
        .replace(['.', '/'], "-");
    let out_dir = std::env::temp_dir().join(format!("labwired-leo-{scenario}-{nonce}"));
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .args([
            "test",
            "--script",
            script_rel,
            "--no-uart-stdout",
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute labwired");

    let uart = std::fs::read_to_string(out_dir.join("uart.log")).unwrap_or_default();
    let result_json =
        std::fs::read_to_string(out_dir.join("result.json")).expect("read result.json");
    let result: serde_json::Value = serde_json::from_str(&result_json).expect("parse result.json");
    let status = result["status"].as_str().unwrap_or("").to_string();

    let _ = std::fs::remove_dir_all(&out_dir);
    LeoRun {
        exit_code: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        uart,
        status,
    }
}

fn assert_contains(haystack: &str, needle: &str, ctx: &str) {
    assert!(
        haystack.contains(needle),
        "{ctx}: expected UART to contain {needle:?}\n--- uart ---\n{haystack}"
    );
}

/// The OLED framebuffer is echoed as ASCII art between OLED-FB-BEGIN/END so the
/// rendered screen is verifiable headlessly. Assert the markers are present and
/// the frame actually drew something (lit pixels), i.e. the SSD1306 driver +
/// kit + C3 I²C path produced a real picture, not a blank panel.
fn assert_oled_rendered(uart: &str, ctx: &str) {
    let begin = uart
        .find("OLED-FB-BEGIN")
        .unwrap_or_else(|| panic!("{ctx}: no OLED-FB-BEGIN marker\n{uart}"));
    let end = uart
        .find("OLED-FB-END")
        .unwrap_or_else(|| panic!("{ctx}: no OLED-FB-END marker\n{uart}"));
    assert!(end > begin, "{ctx}: OLED markers out of order");
    let frame = &uart[begin..end];
    let lit = frame.bytes().filter(|&b| b == b'#').count();
    assert!(
        lit > 200,
        "{ctx}: OLED frame looks blank ({lit} lit pixels); the screen did not render"
    );
}

#[test]
fn leo_normal_scenario_boots_all_sensors_and_flips_verdict() {
    let run = run_leo("examples/esp32c3-leo-airquality/test.yaml");
    assert_eq!(
        run.exit_code,
        Some(0),
        "expected exit 0 (all assertions pass); stderr: {}",
        run.stderr
    );
    assert_eq!(run.status, "pass", "expected result.json status=pass");

    // Boot + all four sensors + the screen brought up over the real C3 I²C bus.
    let ctx = "leo normal";
    assert_contains(&run.uart, "LEO BOOT", ctx);
    assert_contains(&run.uart, "SCD41 READY", ctx);
    assert_contains(&run.uart, "SGP41 READY", ctx);
    assert_contains(&run.uart, "SPS30 READY", ctx);
    assert_contains(&run.uart, "VEML7700 READY", ctx);
    assert_contains(&run.uart, "OLED READY", ctx);

    // The headline story: a closed room fills up and the verdict flips from
    // "air quality is good" to "crack a window" as decoded CO₂ crosses 1000 ppm.
    assert_contains(&run.uart, "air quality is good", ctx);
    assert_contains(&run.uart, "crack a window", ctx);

    // ...and stops there. The NORMAL room's CO₂ ladder asymptotes at 1394 ppm,
    // deliberately just under the 1400 ppm "ventilate now" threshold, so this is
    // what separates NORMAL from STUFFY. Without it the scenario would still
    // pass if the ladder were re-paced or re-scaled into the stuffy range —
    // exactly the vacuous pass that hid the mis-paced stimuli ladder before.
    assert!(
        !run.uart.contains("ventilate now"),
        "{ctx}: NORMAL must stay below the 1400 ppm 'ventilate now' threshold \
         (that is the STUFFY scenario); the CO₂ ladder tops out at 1394 ppm\n\
         --- uart ---\n{}",
        run.uart
    );

    // Mold Index: as the closed room's humidity climbs into the mold-favorable
    // band, the derived mold risk escalates from low — the metric Leo's
    // mold-detection use case is built around.
    assert_contains(&run.uart, "mold risk: low", ctx);

    // Surface-condensation channel — the moisture-first differentiator. The
    // MLX90614 IR reads a cold wall; with the SCD41 air T/RH the firmware
    // computes the dew point and surface RH. As the wall cools below the dew
    // point the surface RH hits condensation while the *air* RH is still benign,
    // escalating the mold verdict to HIGH for a reason an air-only humidity
    // index is blind to. This is exercised end-to-end over the real C3 I²C bus.
    assert_contains(&run.uart, "SURFACE:", ctx);
    assert_contains(&run.uart, "CONDENSING - wall is wet", ctx);
    assert_contains(&run.uart, "(surface condensation)", ctx);
    assert_contains(&run.uart, "mold risk: HIGH", ctx);

    // The run completes and the OLED rendered a real frame.
    assert_contains(&run.uart, "LEO DONE", ctx);
    assert_oled_rendered(&run.uart, ctx);
}

#[test]
fn leo_stuffy_scenario_reaches_ventilate_now() {
    let run = run_leo("examples/esp32c3-leo-airquality/test-stuffy.yaml");
    assert_eq!(
        run.exit_code,
        Some(0),
        "expected exit 0 (all assertions pass); stderr: {}",
        run.stderr
    );
    assert_eq!(run.status, "pass", "expected result.json status=pass");

    let ctx = "leo stuffy";
    assert_contains(&run.uart, "LEO BOOT", ctx);
    // A crowded, poorly ventilated room climbs past 1400 ppm to the strongest
    // verdict.
    assert_contains(&run.uart, "ventilate now", ctx);
    // Damp room + cold exterior wall: condensation drives the mold verdict to
    // its worst (SEVERE) via the surface channel.
    assert_contains(&run.uart, "CONDENSING - wall is wet", ctx);
    assert_contains(&run.uart, "mold risk: SEVERE", ctx);
    assert_contains(&run.uart, "LEO DONE", ctx);
    assert_oled_rendered(&run.uart, ctx);
}
