// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C5 HP UART controller (UART0/UART1).
//!
//! ESP32-C5 TRM v1.0 §32 documents the same UART register head used by the
//! C6 shared twin: FIFO@0x00, STATUS@0x1C, CONF0@0x20, CONF1@0x24, and the
//! newer tail with SLEEP_CONF0..2 at 0x30..0x38 and CLK_CONF at 0x88. The
//! shared [`crate::peripherals::esp_uart::EspUart`] models the FIFO, status,
//! interrupt, and TX pacing behavior. C5-specific source IDs and its 240 MHz
//! HP clock are kept here rather than borrowed from the C6 wrapper.
//!
//! This model does not enforce PCR clock/reset gates or IOMUX pad routing.

use crate::peripherals::esp_uart::EspUart;

/// HP UART interrupt matrix sources from ESP-IDF `soc/esp32c5/include/soc/interrupts.h`.
pub const UART0_INTR_SOURCE_ID: u32 = 47;
pub const UART1_INTR_SOURCE_ID: u32 = 48;

/// ESP32-C5 HP core rate from ESP32-C5 Datasheet v1.5 §4.1.1.1.
pub const CPU_CLOCK_HZ: u64 = 240_000_000;

pub const UART0_BASE: u64 = 0x6000_0000;
pub const UART1_BASE: u64 = 0x6000_1000;

pub fn default_source_id(base: u64) -> u32 {
    if base == UART1_BASE {
        UART1_INTR_SOURCE_ID
    } else {
        UART0_INTR_SOURCE_ID
    }
}

/// Build an ESP32-C5 HP UART using the shared C6-compatible register map.
pub fn new(echo_stdout: bool, source_id: u32) -> EspUart {
    EspUart::new_with_cpu_clock(echo_stdout, source_id, CPU_CLOCK_HZ).with_c6_reg_map()
}
