// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32H533RE executing walk-vs-scheduler fidelity differential.
//! The NUCLEO-H533RE firmware exercises the real configured USART2 + GPIOA
//! bus and the test asserts that the UART stream and execution snapshots match.

#![cfg(feature = "event-scheduler")]

#[path = "stm32_walk_free/mod.rs"]
mod harness;

#[test]
fn stm32h533re_boot_walk_vs_scheduler_is_byte_identical() {
    harness::assert_walk_free_boot_identical(
        "stm32h533re",
        "nucleo-h533re",
        "stm32h533re-smoke.elf",
        b"NUCLEO_H533RE_OK",
        800_000,
        50_000,
    );
}
