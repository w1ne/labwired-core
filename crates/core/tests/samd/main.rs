// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `samd` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test samd <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

#[path = "../common/mod.rs"]
mod common;

mod arduino_nano_33_iot_bme280;
mod atsamd21_peripheral_estate;
mod atsamd21_variant_geometry;
mod pybadge_config;
mod pybadge_firmware;
mod sam_dmac_byte;
mod sam_dmac_sercom4_tx;
mod sam_dmac_tx_completion;
mod sam_sercom_spi;
mod samd51_sercom_timing;
mod samd51_system_reset_dma;
mod samd51_zflash_guest;
mod samd_interrupt_mask_guest;
mod samd_irq_mask_guest;
mod samd_paired_irq_guest;
mod samd_real_core_reset_guest;
mod samd_real_services_guest;
mod samd_real_spi_cancel_guest;
mod samd_real_spi_error_guest;
mod samd_real_spi_guest;
mod samd_real_spi_lease_guest;
mod samd_real_spi_lifetime_guest;
mod samd_real_spi_stale_guest;
mod samd_real_spi_timeout_guest;
mod samd_real_timer_adversary_guest;
mod samd_real_timer_guest;
mod samd_transfer_state_guest;
mod same70_zephyr_boot;
mod st7735_sam_guest;
