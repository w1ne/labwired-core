// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-classic (LX6) peripheral implementations.
//!
//! Mirrors the ESP32-S3 set under crates/core/src/peripherals/esp32s3/ but
//! with the classic chip's register layouts (TRM v4.6).
//!
//! Differences worth knowing when porting an S3 model:
//!   - GPIO base    0x3FF44000 (S3: 0x60004000)
//!   - VSPI (SPI3)  0x3FF65000 (S3: 0x60044000 SPI3)
//!   - DPORT base   0x3FF00000 (S3 has SYSTEM at 0x600C0000 instead)
//!   - IO_MUX base  0x3FF49000 (S3: 0x60009000)
//!   - GPIOs 0..39 (S3: 0..48 with USB-Serial-JTAG hardware)

pub mod dport;
pub mod efuse;
pub mod factory;
pub mod flash_mmu;
pub mod gpio;
pub mod i2c;
pub mod ledc;
pub mod mcpwm;
pub mod rtc_cntl;
pub mod sar_adc;
pub mod sdio_stub;
pub mod sha;
pub mod spi;
pub mod syscon;
pub mod timg;
pub mod twai;
pub mod uart;
