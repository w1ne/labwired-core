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

/// #2005: the inert CAN model used to keep this shipped board at one
/// instruction per batch. Use its real descriptor and committed smoke image.
#[test]
fn ra4m1_uno_r4_minima_batches() {
    let manifest = SystemManifest::from_file(
        workspace_root().join("configs/systems/arduino-uno-r4-minima.yaml"),
    )
    .expect("load UNO R4 Minima manifest");
    assert!(
        manifest.walk_deleted.is_none(),
        "RA4M1 batching must derive from peripheral capabilities"
    );
    assert_batches("ra4m1", "arduino-uno-r4-minima", "ra4m1-uno-r4-smoke.elf");
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

// ---------------------------------------------------------------------------
// The boards that are actually slow.
//
// The three tests above are STM32, and they were added because #835 needed
// them. Meanwhile the perf fleet's batch column — the DEFAULT path — is
// bimodal, and none of the slow side is measured here:
//
//     ARM / nRF / STM32 / RP   ~54 Ir/step   width  511.9
//     esp32c3  (RISC-V)        201.8         width  511.9
//     atmega328p (AVR)         241.3         width   25.0   <-- 20x narrow
//     esp32s3  (Xtensa)        364.8         width 1023.9   <-- WIDEST, still 6.7x
//     esp32    (Xtensa)        442.4         width  511.9
//
// Those two shapes are different defects and the width separates them.
// `atmega328p` is bound by something: at 25 instructions per batch the
// per-batch overhead is amortised over twenty times fewer instructions than
// anywhere else. `esp32s3` has the widest batch of any board in the fleet and
// still costs 6.7x ARM, which rules per-batch overhead OUT and puts the cost
// inside the instruction loop.
//
// `quantum_trace` exists to answer the first question — the module says #835
// "ruled out three clauses, and still ended on a guess" before it did — but it
// has never been pointed at the narrow board. That is what this section is
// for: measure the width here, and under `--features quantum-trace` print the
// clause that bound it, so the next step is a name instead of a guess.

/// Mean instructions per CPU batch for an AVR board.
///
/// A separate function from `mean_batch_width` because the CPU differs, not
/// the measurement: AVR is `Avr::new()` + `load_program_image` where Cortex-M
/// is `configure_cortex_m`, and `Machine` is generic over the core. The bus
/// build, the warm-up and the batch accounting are identical on purpose, so
/// the two numbers are comparable.
fn avr_mean_batch_width(chip_name: &str, system_name: &str, fixture: &str) -> f64 {
    let chip_path = workspace_root()
        .join("configs/chips")
        .join(format!("{chip_name}.yaml"));
    let sys_path = workspace_root()
        .join("configs/systems")
        .join(format!("{system_name}.yaml"));
    let chip =
        ChipDescriptor::from_file(&chip_path).unwrap_or_else(|e| panic!("load {chip_name}: {e}"));
    let manifest =
        SystemManifest::from_file(&sys_path).unwrap_or_else(|e| panic!("load {system_name}: {e}"));

    let bus = SystemBus::from_config(&chip, &manifest)
        .unwrap_or_else(|e| panic!("build {chip_name} bus: {e}"));
    let interval = bus.max_safe_tick_interval();
    eprintln!("{system_name}: max_safe_tick_interval = {interval}");

    let bytes = std::fs::read(workspace_root().join("tests/fixtures").join(fixture))
        .unwrap_or_else(|e| panic!("read {fixture}: {e}"));
    let image = labwired_loader::load_elf_bytes(&bytes).expect("load AVR ELF");
    let mut cpu = labwired_core::cpu::Avr::new();
    cpu.load_program_image(&image);
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = interval;
    machine.bus.config.peripheral_tick_interval = interval;

    machine.run(Some(WARMUP_STEPS)).expect("warm-up");
    #[cfg(feature = "quantum-trace")]
    labwired_core::machine::quantum_trace::reset();
    let batches_before = machine.step_profile().cpu_batches;
    machine.run(Some(MEASURE_STEPS)).expect("measured window");
    let batches = machine.step_profile().cpu_batches - batches_before;
    assert!(batches > 0, "{system_name}: no batches committed");
    f64::from(MEASURE_STEPS) / batches as f64
}

/// Print the quantum-trace verdict for a board, when the feature is on.
///
/// A no-op without it, so these tests stay runnable in the ordinary lane and
/// only become an investigation when someone asks for one.
fn report_binder(board: &str) {
    #[cfg(feature = "quantum-trace")]
    {
        use labwired_core::machine::quantum_trace;
        eprintln!("--- {board}: what bound the CPU quantum ---");
        for (clause, hits, share) in quantum_trace::snapshot() {
            eprintln!("    {clause:28} {hits:>10}  {:.1}%", share * 100.0);
        }
        match quantum_trace::dominant() {
            Some(c) => eprintln!("    dominant: {c}"),
            None => eprintln!("    dominant: (nothing recorded)"),
        }
    }
    #[cfg(not(feature = "quantum-trace"))]
    let _ = board;
}

/// AVR: instruction cycles are 1..=4, so a window sized by the longest
/// instruction used to stop short and decay (measured about 25, where
/// `512 / 4` is 128). The planner's fill finishes that tick window. Half of
/// the longest-instruction width is the floor: still far above the decay, and
/// under the 256 the cycle-accurate cores are held to.
#[test]
fn atmega328p_batch_width_fills_its_tick_window() {
    let mean = avr_mean_batch_width("atmega328p", "arduino-uno", "avr/arduino-uno-blinky.elf");
    eprintln!("atmega328p mean batch width: {mean:.2}");
    report_binder("atmega328p");
    assert!(
        mean >= 64.0,
        "atmega328p mean batch width {mean:.2} decayed; the tick fill should \
         keep it near 512/4 = 128"
    );
}
