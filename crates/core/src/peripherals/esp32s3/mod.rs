// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 peripheral implementations (Plan 2+).

pub mod aes;
pub mod core1_control;
pub mod crosscore_ipi;
pub mod ds;
pub mod extmem;
pub mod factory;
pub mod flash_xip;
pub mod gdma;
pub mod gpio;
pub mod gpspi;
pub mod hmac;
pub mod i2c;
pub mod i2s;
pub mod intmatrix;
pub mod io_mux;
pub mod lcd_cam;
pub mod ledc;
pub mod mcpwm;
pub mod pcnt;
pub mod rmt;
pub mod rng;
pub mod rsa;
pub mod sar_adc;
pub mod sdmmc;
pub mod sens;
pub mod sha;
pub mod spi_mem_flash;
pub mod system;
pub mod systimer;
pub mod timer_group;
pub mod tmp102;
pub mod twai;
pub mod uart;
pub mod usb_otg;
pub mod usb_serial_jtag;
// Fake WiFi/lwIP functional-outcome thunks — behind an off-by-default feature
// so this canned state can never compile into a production/run build. Only the
// `e2e_labwired_wifi` bring-up harness enables it.
#[cfg(feature = "wifi-thunks")]
pub mod wifi_thunks;
