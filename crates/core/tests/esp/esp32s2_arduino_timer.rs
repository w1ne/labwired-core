// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Stock Arduino-ESP32's `micros()` on the ESP32-S2 advances monotonically
//! across a `delay()`.
//!
//! Fixture: `tests/fixtures/esp32s2-arduino-l7.elf` is the Arduino matrix L7
//! sketch (`validation/arduino-matrix/sketches/L7_timer`) built for PlatformIO
//! `esp32-s2-saola-1` (espressif32 8.0.0), debug info stripped. It uses the L0
//! build's partition table (the two are byte-identical). The sketch samples
//! `micros()` four times — before and after `delay(2)`, after a 2000-iteration
//! busy spin, and after `delay(1)` — and prints `LW_L7_OK` only when the first
//! delta is at least 1 ms and all four samples are non-decreasing.
//!
//! # Why this test exists (the defect it pins)
//!
//! Measured on the Arduino matrix row on 2026-11-24: the row reported
//! `LW_L7_FAIL t0=2514 t1=5000 t2=4565 t3=5506` while the systimer model
//! answered every one of those reads with a strictly increasing counter. The
//! sketch keeps the second sample in `a7` across `call8 delay` (legal under the
//! Xtensa windowed ABI — a callee's window does not overlap its caller's
//! `a0..a7`). With the sim-level shadow-window mode the value came back as
//! `5000`: a call chain that wraps the 64-register file loses the caller's
//! `a4..a7`, because the shadow saves only a0..a3 of a displaced window.
//!
//! `boot::esp32s2::fast_boot` now leaves the core on the firmware's own
//! `_WindowOverflow{4,8}` handlers, as the S3 fast boot already did. The second
//! test is the negative control: with the shadow mode forced back on, the same
//! firmware prints `LW_L7_FAIL` and the DWT-free oracle is clean, so the pass
//! above depends on the window machinery and not on the sketch tolerating a
//! corrupt clock.

use labwired_core::system::builder::build_system_bus;
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Boot the L7 fixture and return the UART0 text once the sketch prints a
/// verdict (or the step budget runs out). `faithful_windows` defaults to what
/// `fast_boot` decides; the control test overrides it after boot.
fn boot(max_steps: u64, shadow_windows: bool) -> (String, u32) {
    let sys = root().join("validation/arduino-matrix/systems/esp32s2.yaml");
    let resolved = labwired_config::ResolvedSystem::from_manifest_file(&sys).expect("system");
    let mut bus = build_system_bus(Some(&resolved)).expect("bus");
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    let elf = std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l7.elf")).unwrap();
    let table =
        std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l0.partitions.bin")).unwrap();
    let flash = labwired_core::system::xtensa::attach_esp32s2_flash(&mut bus, 4 * 1024 * 1024);
    let mut cpu = labwired_core::cpu::XtensaLx7::new();
    cpu.reset(&mut bus).unwrap();
    labwired_core::boot::esp32s2::fast_boot(
        &elf,
        &mut bus,
        &mut cpu,
        &flash,
        Some(table.as_slice()),
    )
    .expect("fast_boot");
    let faithful_after_boot = cpu.faithful_windows;
    assert!(
        faithful_after_boot,
        "fast_boot must hand the core the firmware's own window handlers"
    );
    if shadow_windows {
        cpu.faithful_windows = false;
    }
    let mut machine = Machine::new(cpu, bus);
    let mut steps = 0u32;
    for _ in 0..max_steps {
        machine.step().expect("step");
        steps += 1;
        let out = uart.lock().unwrap();
        if out.windows(9).any(|w| w == b"LW_L7_OK") || out.windows(10).any(|w| w == b"LW_L7_FAIL") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&uart.lock().unwrap()).into_owned();
    (text, steps)
}

#[test]
fn stock_arduino_micros_advances_across_delay() {
    let (text, steps) = boot(12_000_000, false);
    assert!(
        text.contains("LW_L7_OK"),
        "no LW_L7_OK after {steps} steps; UART0: {text:?}"
    );
}

/// Negative control: force the shadow-window mode back on and the same firmware
/// reports a clock that goes backwards. Without this the test above would also
/// pass if the sketch had simply been widened.
#[test]
fn shadow_window_mode_reports_a_non_monotonic_clock() {
    let (text, _steps) = boot(12_000_000, true);
    assert!(
        !text.contains("LW_L7_OK"),
        "LW_L7_OK with the shadow-window mode the fix replaces; UART0: {text:?}"
    );
    assert!(
        text.contains("LW_L7_FAIL"),
        "expected the sketch's own verdict line; UART0: {text:?}"
    );
}
