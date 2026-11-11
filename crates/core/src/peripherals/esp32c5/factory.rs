// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Data-driven factory for ESP32-C5 peripheral models.

use super::uart;
use crate::Peripheral;
use labwired_config::PeripheralConfig;

pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    if canonical_type != "esp32c5_uart" {
        return None;
    }

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
