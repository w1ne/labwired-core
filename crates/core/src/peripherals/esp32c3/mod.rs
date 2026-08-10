// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 (RISC-V) specific peripheral models.

pub mod ana_i2c;
pub mod apb_saradc;
pub mod bt;
pub mod cache;
pub mod factory;
pub mod forced_status;
pub mod gpio;
pub mod i2c;
pub mod io_mux;
pub mod ledc;
pub mod pms;
pub mod reg_block;
pub mod rmt;
pub mod rng;
pub mod rtc_timer;
pub mod sar_adc;
pub mod sha;
pub mod spi;
pub mod uart;
pub mod virtual_wifi;
pub mod virtual_wifi_host_net;
pub mod virtual_wifi_inet;
pub mod wifi_mac;
