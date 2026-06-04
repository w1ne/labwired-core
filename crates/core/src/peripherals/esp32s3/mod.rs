// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 peripheral implementations (Plan 2+).

pub mod aes;
pub mod core1_control;
pub mod crosscore_ipi;
pub mod hmac;
pub mod extmem;
pub mod flash_xip;
pub mod gpio;
pub mod i2c;
pub mod intmatrix;
pub mod io_mux;
pub mod rng;
pub mod rsa;
pub mod sha;
pub mod rom_thunks;
pub mod spi_mem_flash;
pub mod system_stub;
pub mod systimer;
pub mod tmp102;
pub mod gdma;
pub mod gpspi;
pub mod i2s;
pub mod ledc;
pub mod pcnt;
pub mod rmt;
pub mod sar_adc;
pub mod twai;
pub mod timer_group;
pub mod uart;
pub mod usb_serial_jtag;
