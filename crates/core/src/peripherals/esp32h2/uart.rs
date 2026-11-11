// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-H2 HP UART controller.
//!
//! ESP-IDF's H2 UART register header uses the C6-compatible head and tail
//! touched by the console smoke (FIFO, CLKDIV, STATUS, CONF0/1, SLEEP_CONF,
//! CLK_CONF and ID). The H2 has its own interrupt source numbers and 96 MHz
//! core clock, so those values stay here rather than borrowing the C6 wrapper.

use crate::peripherals::esp_uart::EspUart;

pub const UART0_INTR_SOURCE_ID: u32 = 33;
pub const UART1_INTR_SOURCE_ID: u32 = 34;
pub const CPU_CLOCK_HZ: u64 = 96_000_000;
pub const UART0_BASE: u64 = 0x6000_0000;
pub const UART1_BASE: u64 = 0x6000_1000;

pub fn default_source_id(base: u64) -> u32 {
    if base == UART1_BASE {
        UART1_INTR_SOURCE_ID
    } else {
        UART0_INTR_SOURCE_ID
    }
}

pub fn new(echo_stdout: bool, source_id: u32) -> EspUart {
    EspUart::new_with_cpu_clock(echo_stdout, source_id, CPU_CLOCK_HZ).with_c6_reg_map()
}
