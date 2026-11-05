// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Factory for the modeled ESP32-C5 UART instances.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    let dev: Box<dyn Peripheral> = match canonical_type {
        "esp32c5_pcr" => Box::new(super::pcr::Esp32c5Pcr::new()),
        "esp32c5_gpio" => Box::new(super::gpio::Esp32c5Gpio::new()),
        "esp32c5_io_mux" => Box::new(super::io_mux::Esp32c5IoMux::new()),
        "esp32c5_uart" => {
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
