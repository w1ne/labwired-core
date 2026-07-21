// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32H563 (Cortex-M33) executing walk-vs-scheduler fidelity differential.
//!
//! Closes the H563 gap: the family previously had static register MMIO diffs
//! (`h563_mmio_diff`) and the FDCAN/flash suites but no executing/timing gate on
//! the core-timer path. This runs the stock Zephyr `hello_world` for
//! `nucleo_h563zi` — whose kernel tick is driven from the Cortex SysTick and
//! whose console output is deterministic — on the real `stm32h563`
//! `from_config` bus, once with the SysTick/SCB/DWT + SoC timing models pinned to
//! the legacy walk and once scheduler-driven, and asserts the boot trace +
//! console stream are byte-identical. See `stm32_walk_free`.

#![cfg(feature = "event-scheduler")]

#[path = "stm32_walk_free/mod.rs"]
mod harness;

/// H563 kernel-tick (SysTick) IRQ-cadence + console differential over the real
/// Zephyr boot.
#[test]
fn stm32h563_zephyr_boot_walk_vs_scheduler_is_byte_identical() {
    harness::assert_walk_free_boot_identical(
        "stm32h563",
        "nucleo-h563zi-demo",
        "stm32h563-zephyr-hello.elf",
        b"Hello World! nucleo_h563zi",
        800_000,
        50_000,
    );
}
