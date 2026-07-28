// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Data-driven factory for ESP32-C3 peripheral models.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    let dev: Box<dyn Peripheral> = match canonical_type {
        "esp32c3_gpio" => Box::new(super::gpio::Esp32c3Gpio::new()),
        "esp32c3_io_mux" => Box::new(super::io_mux::Esp32c3IoMux::new()),
        // RMT TX model (Arduino RGB_BUILTIN / rgbLedWrite). Instant TX_END +
        // LogicTap pulse on TX_START for matrix `led_watch: rmt:0`.
        "esp32c3_rmt" => Box::new(super::rmt::Esp32c3Rmt::new_default()),
        // Real ESP-IDF UART register map + TX FIFO + interrupts. UART0 is the
        // Arduino `Serial` console and echoes to stdout; UART1 is
        // capture-only. Source id defaults from the base address (see
        // `super::uart::default_source_id`) and an explicit `irq:` wins.
        "esp32c3_uart" => {
            let source_id = p_cfg
                .irq
                .unwrap_or_else(|| super::uart::default_source_id(p_cfg.base_address));
            let echo = p_cfg
                .config
                .get("echo_stdout")
                .and_then(|v| v.as_bool())
                .unwrap_or(source_id == super::uart::UART0_INTR_SOURCE_ID);
            Box::new(super::uart::new(echo, source_id))
        }
        _ => return None,
    };
    Some(dev)
}

pub const SUPPORTED_TYPES: &[&str] = &[
    "esp32c3_gpio",
    "esp32c3_io_mux",
    "esp32c3_rmt",
    "esp32c3_uart",
];
