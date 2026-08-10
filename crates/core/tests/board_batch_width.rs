// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every walk-deleted board must actually batch.
//!
//! A board can be fully scheduler-driven — `walk_deleted`, `max_safe_tick_
//! interval() == 512`, `requires_cycle_accurate() == false` — and still execute
//! one instruction per batch, because any single scheduler event that re-arms
//! one cycle ahead forever pins `plan_cpu_window`'s deadline clamp to 1. Nothing
//! is incorrect when that happens; the board is simply ~500x slower than its
//! siblings, and no functional test can see it.
//!
//! That is exactly how #835 happened. NUCLEO-L073RZ ran at 1.00 steps/batch
//! while L476 and F401 ran at ~511 after #830, and finding out why took a whole
//! investigation of elimination that still ended on a wrong guess (UART). The
//! real cause was I²C: the demo board has no I²C device, so every probe NACKs,
//! the HAL clears CR1.PE to reset the block, and the model leaked BUSY through
//! the disable — leaving `L4I2c::active()` true, and its per-cycle engine chain
//! re-arming at +1 for the rest of the run.
//!
//! So batch width is asserted directly, per board. This is a THROUGHPUT gate
//! (`scripts/perf/board_perf.py` measures host cost per step; this measures how
//! many steps the engine is willing to run per plan), and it fails loudly on the
//! board that regressed rather than on a repo-wide average.

#![cfg(feature = "event-scheduler")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{DebugControl, Machine};
use std::path::PathBuf;

/// Warm-up instructions retired before measuring, so reset and the clock/PLL
/// bring-up (which legitimately runs cycle-accurate) stay out of the number.
const WARMUP_STEPS: u32 = 200_000;
/// Measured window.
const MEASURE_STEPS: u32 = 1_000_000;

/// A walk-deleted board planning batches at its recommended tick interval should
/// average close to that interval. Half of it leaves generous headroom for
/// legitimate clamps (a real peripheral deadline landing mid-window) while still
/// being ~250x away from the 1.00 that #835 was about.
const MIN_MEAN_BATCH: f64 = 256.0;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Mean instructions per CPU batch for a board running its own demo firmware.
fn mean_batch_width(chip_name: &str, system_name: &str, fixture: &str) -> f64 {
    let chip_path = workspace_root()
        .join("configs/chips")
        .join(format!("{chip_name}.yaml"));
    let sys_path = workspace_root()
        .join("configs/systems")
        .join(format!("{system_name}.yaml"));
    let chip =
        ChipDescriptor::from_file(&chip_path).unwrap_or_else(|e| panic!("load {chip_name}: {e}"));
    let mut manifest =
        SystemManifest::from_file(&sys_path).unwrap_or_else(|e| panic!("load {system_name}: {e}"));
    manifest.chip = sys_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_str()
        .unwrap()
        .to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest)
        .unwrap_or_else(|e| panic!("build {chip_name} bus: {e}"));
    assert!(
        bus.legacy_walk_disabled,
        "{system_name}: expected a walk-deleted bus — this gate is about boards that \
         SHOULD batch; if the walk is legitimately live here, drop the board from the list"
    );
    let interval = bus.max_safe_tick_interval();
    assert!(
        interval > 1,
        "{system_name}: max_safe_tick_interval() = 1, so there is no batching to measure"
    );

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = interval;
    machine.bus.config.peripheral_tick_interval = interval;

    let fixture_path = workspace_root().join("tests/fixtures").join(fixture);
    let image = labwired_loader::load_elf(&fixture_path)
        .unwrap_or_else(|e| panic!("load ELF {fixture_path:?}: {e}"));
    machine.load_firmware(&image).expect("load firmware");

    machine.run(Some(WARMUP_STEPS)).expect("warm-up");
    let batches_before = machine.step_profile().cpu_batches;
    machine.run(Some(MEASURE_STEPS)).expect("measured window");
    let batches = machine.step_profile().cpu_batches - batches_before;
    assert!(batches > 0, "{system_name}: no batches committed");
    f64::from(MEASURE_STEPS) / batches as f64
}

fn assert_batches(chip: &str, system: &str, fixture: &str) {
    let mean = mean_batch_width(chip, system, fixture);
    assert!(
        mean >= MIN_MEAN_BATCH,
        "{system}: {mean:.2} instructions per CPU batch, expected >= {MIN_MEAN_BATCH:.0}.\n\
         The board is scheduler-driven but something is clamping the CPU quantum. \
         Name the clause instead of guessing:\n  \
         cargo test --release -p labwired-core --features event-scheduler,quantum-trace ...\n  \
         then read `labwired_core::machine::quantum_trace::snapshot()`."
    );
}

/// The #835 regression. Held at 1.00 by a leaked I²C BUSY bit; ~511 once
/// `L4I2c` honours CR1.PE=0 the way silicon does.
#[test]
fn nucleo_l073rz_batches() {
    assert_batches("stm32l073", "nucleo-l073rz", "nucleo-l073rz-demo.elf");
}

/// The reference the L073 was measured against in #835 — already batching, and
/// here so a shared-path regression cannot quietly take both down at once.
#[test]
fn nucleo_l476rg_batches() {
    assert_batches("stm32l476", "nucleo-l476rg", "nucleo-l476rg-demo.elf");
}

#[test]
fn nucleo_f401re_batches() {
    assert_batches("stm32f401", "nucleo-f401re", "stm32f401-zephyr-hello.elf");
}
