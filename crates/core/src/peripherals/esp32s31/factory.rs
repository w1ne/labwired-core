// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Factory for the ESP-Mosaico GPIO7/GPIO8 profile.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

pub fn try_build(canonical_type: &str, _p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    let dev: Box<dyn Peripheral> = match canonical_type {
        "esp32s31_gpio" => Box::new(super::gpio::Esp32s31Gpio::new()),
        "esp32s31_io_mux" => Box::new(super::io_mux::Esp32s31IoMux::new()),
        _ => return None,
    };
    Some(dev)
}
