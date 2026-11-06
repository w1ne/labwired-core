// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Factory for the focused ESP32-P4 GPIO profile.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

pub fn try_build(canonical_type: &str, _p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    let dev: Box<dyn Peripheral> = match canonical_type {
        "esp32p4_gpio" => Box::new(super::gpio::Esp32p4Gpio::new()),
        "esp32p4_io_mux" => Box::new(super::io_mux::Esp32p4IoMux::new()),
        _ => return None,
    };
    Some(dev)
}
