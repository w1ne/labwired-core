// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Data-driven factory for the ESP32-H2 UART and direct-GPIO subsets.

use super::uart;
use crate::Peripheral;
use labwired_config::PeripheralConfig;

pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    match canonical_type {
        "esp32h2_uart" => {
            let source_id = p_cfg
                .irq
                .unwrap_or_else(|| uart::default_source_id(p_cfg.base_address));
            let echo = p_cfg
                .config
                .get("echo_stdout")
                .and_then(|v| v.as_bool())
                .unwrap_or(source_id == uart::UART0_INTR_SOURCE_ID);
            Some(Box::new(uart::new(echo, source_id)))
        }
        "esp32h2_gpio" => Some(Box::new(super::gpio::Esp32h2Gpio::new())),
        _ => None,
    }
}
