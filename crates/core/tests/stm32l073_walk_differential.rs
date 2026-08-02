// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32L073 (Cortex-M0+) executing walk-vs-scheduler fidelity differential.
//!
//! Closes the L073 gap: the family previously had only static register MMIO
//! diffs (`stm32l0_mmio_diff`) with no executing/timing gate. This runs the
//! stock Zephyr `hello_world` for `nucleo_l073rz` — whose kernel tick is driven
//! from the Cortex SysTick and whose console output is deterministic — on the
//! real `stm32l073` `from_config` bus, once with the SysTick/SCB/DWT + SoC
//! timing models pinned to the legacy walk and once scheduler-driven, and
//! asserts the boot trace + console stream are byte-identical. See
//! `stm32_walk_free`.

#![cfg(feature = "event-scheduler")]

#[path = "stm32_walk_free/mod.rs"]
mod harness;

/// L073 kernel-tick (SysTick) IRQ-cadence + console differential over the real
/// Zephyr boot.
#[test]
fn stm32l073_zephyr_boot_walk_vs_scheduler_is_byte_identical() {
    harness::assert_walk_free_boot_identical(
        "stm32l073",
        "nucleo-l073rz",
        "stm32l073-zephyr-hello.elf",
        b"Hello World! nucleo_l073rz",
        800_000,
        50_000,
    );
}
