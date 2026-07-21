// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32F401 (Cortex-M4) executing walk-vs-scheduler fidelity differential.
//!
//! Closes the F401 gap: the family previously had only a survival smoke test and
//! the fleet scoreboard, with no dedicated executing-fidelity gate. This runs the
//! stock Zephyr `hello_world` for `nucleo_f401re` — whose kernel tick is driven
//! from the Cortex SysTick and whose USART2 console output is deterministic — on
//! the real `stm32f401` `from_config` bus, once with the SysTick/SCB/DWT + SoC
//! timing models pinned to the legacy walk and once scheduler-driven, and asserts
//! the boot trace + console stream are byte-identical. See `stm32_walk_free`.

#![cfg(feature = "event-scheduler")]

#[path = "stm32_walk_free/mod.rs"]
mod harness;

/// F401 kernel-tick (SysTick) IRQ-cadence + console differential over the real
/// Zephyr boot.
#[test]
fn stm32f401_zephyr_boot_walk_vs_scheduler_is_byte_identical() {
    harness::assert_walk_free_boot_identical(
        "stm32f401",
        "nucleo-f401re",
        "stm32f401-zephyr-hello.elf",
        b"Hello World! nucleo_f401re",
        800_000,
        50_000,
    );
}
