// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `stm32` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test stm32 <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

#[path = "../common/mod.rs"]
mod common;

mod cortex_m_event_sleep;
mod exti_handler_once_per_edge;
mod fdcan;
mod fdcan_attach_seam;
mod flash_h5_ops;
mod g0_customer_timing;
mod g0_exti;
mod g0_reference_firmware;
mod g0_tiol112;
mod g0_uart_pad;
mod h563_conformance;
mod modern_uart_timing;
mod nucleo_u545re;
mod nvic_masking_config_path;
mod stm32_v2_timer_output_pad;
mod stm32_variant_geometry;
mod stm32duino_i2c_multibyte_bme280;
mod stm32duino_print_float_vmov;
mod stm32f1_gpio_lock_reset;
mod stm32f401_conformance;
mod stm32f407_dma_walk_differential;
mod stm32g0_dmamux_generators;
mod stm32g0_uart_dma;
mod stm32h735_peripheral_estate;
mod stm32u575_60north;
mod stm32u575_60north_registers;
mod stm32u5_crypto_walk_free;
mod stm32u5_exec_oracle;
mod stm32u5_reset_conformance;
mod timer_input_capture;
mod timer_level_deassert_unpends;
mod timer_masked_level_batches;
mod u545_u575_drift_guard;
mod u5_exti;
mod u5_reference_firmware;

mod l6360_digital;
mod l6362a_digital;
mod stm32_iolink_physical;
mod stm32l476_exti_mux;

mod sn74hc165_gpio;

mod timed_uart_pad;
