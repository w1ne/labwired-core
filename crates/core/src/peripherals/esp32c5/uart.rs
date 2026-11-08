// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C5 UART controller values for the shared Espressif UART twin.

use crate::peripherals::esp_uart::EspUart;

/// C5 interrupt-matrix source ordinals from ESP-IDF `soc/interrupts.h`.
pub const UART0_INTR_SOURCE_ID: u32 = 47;
pub const UART1_INTR_SOURCE_ID: u32 = 48;

/// C5 high-performance RISC-V core's maximum supported clock.
pub const CPU_CLOCK_HZ: u64 = 240_000_000;
/// Fixed UART source selected by the firmware smoke: PLL_F80M.
pub const UART_SOURCE_CLOCK_HZ: u64 = 80_000_000;
pub const UART0_BASE: u64 = 0x6000_0000;
pub const UART1_BASE: u64 = 0x6000_1000;

pub fn default_source_id(base: u64) -> u32 {
    if base == UART1_BASE {
        UART1_INTR_SOURCE_ID
    } else {
        UART0_INTR_SOURCE_ID
    }
}

/// Build the shared UART twin with C5's core and explicit PLL_F80M source.
/// C5 shares the modern ESP32 UART register tail used by the C6 model.
pub fn new(echo_stdout: bool, source_id: u32) -> EspUart {
    EspUart::new_with_cpu_clock(echo_stdout, source_id, CPU_CLOCK_HZ)
        .with_uart_source_clock_hz(UART_SOURCE_CLOCK_HZ)
        .with_c6_reg_map()
}
