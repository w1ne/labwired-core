// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! End-to-end idle fast-forward proof on a REAL Zephyr WFI firmware
//! (`nrf54l15-tick-test.elf`: `printk` + `k_msleep(10)` in a loop, the
//! canonical tickless-idle pattern the GRTC kernel tick drives).
//!
//! Runs the same ELF twice on the nRF54L15 board model:
//!   * FF OFF — the CPU interprets every idle cycle (the platform's documented
//!     no-FF semantics), so each 10 ms `k_msleep` costs ~1.28 M interpreted
//!     cycles;
//!   * FF ON  — `try_idle_fast_forward` skips the WFI window straight to the
//!     next scheduled GRTC compare deadline.
//!
//! The gate asserts the serial output is byte-identical up to the shared tick
//! (FF must not change behaviour), that `idle_fast_forward_cycles_skipped` is
//! large, and that reaching the same tick retires far fewer instructions.
//!
//! This is the payoff of the GRTC scheduler migration: before it,
//! `idle_fast_forward_legacy_safe()` was false on this board (the GRTC's
//! per-cycle `tick()` kept it always-active), so FF never engaged and an
//! idle-heavy firmware ran at interpreter speed.

#![cfg(feature = "event-scheduler")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, BreakpointPolicy, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn load_system() -> (ChipDescriptor, SystemManifest) {
    let chip_path = workspace_root().join("configs/chips/nrf54l15.yaml");
    let sys_path = workspace_root().join("configs/systems/nrf54l15dk.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nrf54l15 chip");
    let mut manifest = SystemManifest::from_file(&sys_path).expect("load nrf54l15dk system");
    manifest.chip = sys_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_str()
        .unwrap()
        .to_string();
    (chip, manifest)
}

struct RunResult {
    serial: String,
    skipped: u64,
    retired: u64,
    wall_ms: u128,
}

/// Run the tick-test ELF with idle fast-forward `ff` until the serial output
/// contains `until` (or a hard instruction cap is hit), returning the metrics.
fn run_until(ff: bool, until: &str, instruction_cap: u64) -> RunResult {
    let (chip, manifest) = load_system();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build nrf54l15 bus");
    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    machine.config.idle_fast_forward_enabled = ff;
    // NOTE: the legacy walk is deliberately NOT disabled — the board's CLOCK
    // and stub peripherals still need their (dynamically-active) settling
    // ticks during boot. Idle FF engages on its own once every peripheral is
    // inert (`idle_fast_forward_legacy_safe`), which is exactly the WFI window.

    let image =
        labwired_loader::load_elf(&workspace_root().join("tests/fixtures/nrf54l15-tick-test.elf"))
            .expect("load tick-test elf");
    machine.load_firmware(&image).expect("load firmware");

    let start = Instant::now();
    machine.reset_step_profile();
    let chunk = 200_000u64;
    loop {
        let uart = uart_sink.lock().unwrap().clone();
        if String::from_utf8_lossy(&uart).contains(until) {
            break;
        }
        if machine.step_profile().cpu_instructions >= instruction_cap {
            break;
        }
        machine
            .advance(AdvanceRequest::run(Some(chunk)).with_breakpoints(BreakpointPolicy::Ignore))
            .expect("advance");
    }
    let wall_ms = start.elapsed().as_millis();
    let serial = String::from_utf8_lossy(&uart_sink.lock().unwrap()).to_string();
    RunResult {
        serial,
        skipped: machine.idle_fast_forward_cycles_skipped,
        retired: machine.step_profile().cpu_instructions,
        wall_ms,
    }
}

#[test]
fn idle_fast_forward_accelerates_zephyr_msleep_loop() {
    // Reach a modest tick so the FF-off baseline stays quick (each tick is a
    // 10 ms k_msleep = ~1.28 M interpreted cycles with FF off).
    const TARGET: &str = "tick 5";

    let off = run_until(false, TARGET, 40_000_000);
    let on = run_until(true, TARGET, 40_000_000);

    eprintln!(
        "\n[idle-FF] FF OFF: reached {:?}  retired={} instrs  skipped={}  wall={}ms",
        off.serial.contains(TARGET),
        off.retired,
        off.skipped,
        off.wall_ms
    );
    eprintln!(
        "[idle-FF] FF ON : reached {:?}  retired={} instrs  skipped={}  wall={}ms\n",
        on.serial.contains(TARGET),
        on.retired,
        on.skipped,
        on.wall_ms
    );

    assert!(
        off.serial.contains(TARGET) && on.serial.contains(TARGET),
        "both runs must reach {TARGET}\n  off: {:?}\n  on:  {:?}",
        off.serial,
        on.serial
    );

    // FF must not change behaviour: the serial stream up to the shared tick is
    // byte-identical.
    let cut = TARGET.len();
    let off_prefix = &off.serial[..off.serial.find(TARGET).unwrap() + cut];
    let on_prefix = &on.serial[..on.serial.find(TARGET).unwrap() + cut];
    assert_eq!(
        off_prefix, on_prefix,
        "idle fast-forward changed the serial output"
    );

    // FF OFF must interpret the idle cycles (no skipping); FF ON must skip a
    // large idle window.
    assert_eq!(off.skipped, 0, "FF-off must not skip any cycles");
    assert!(
        on.skipped > 3_000_000,
        "idle fast-forward must skip a large idle window (skipped {})",
        on.skipped
    );

    // Reaching the same tick must retire dramatically fewer instructions.
    assert!(
        on.retired * 10 < off.retired,
        "FF must retire far fewer instructions to reach {TARGET} (on={}, off={})",
        on.retired,
        off.retired
    );
}
