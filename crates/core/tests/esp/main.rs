// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `esp` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test esp <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

#[path = "../common/mod.rs"]
mod common;

mod apollo4_bootrom;
mod brom_boot_smoke;
mod c3_gpio_interrupts;
mod c3_reference_firmware;
mod e2e_esp32c3_snapshot_resume;
mod e2e_external_arduino_esp32_in_sim;
mod e2e_labwired_ereader;
mod esp32_classic_aids_stability;
mod esp32_classic_console;
mod esp32_classic_walk_differential;
mod esp32_family_gpio_head;
mod esp32_onboarding_exec_oracle;
mod esp32_syscon_overlap;
mod esp32_ws2812_rmt;
mod esp32c3_bt_active_walk_pinners;
mod esp32c3_esp_hal_interrupts;
mod esp32c3_pms_violation;
mod esp32c3_rmt_irq_delivery;
mod esp32c3_rtc_slow_calibration;
mod esp32c3_trigger_systimer;
mod esp32c3_workshop_source;
mod esp32c6_stock_arduino_fast_boot;
mod esp32s2_arduino_adc;
mod esp32s2_arduino_boot;
mod esp32s2_arduino_i2c;
mod esp32s2_arduino_ledc;
mod esp32s2_arduino_rmt;
mod esp32s2_arduino_spi;
mod esp32s2_arduino_twai;
mod esp32s3_gdma_oracle;
mod esp32s3_gpspi_oracle;
mod esp32s3_interrupt_map_vendor_offsets;
mod esp32s3_timg_gpio_reset_values;
mod esp32s3_ws2812_rmt;
mod fixture_ble_pong_provenance;
mod intmatrix_alarm;
mod world_esp32c3_pingpong;
mod world_esp_nodes;
mod xtensa_decode;
mod xtensa_elf_vaddr;
mod xtensa_exec;
mod xtensa_length;
mod xtensa_lx7;
mod xtensa_regs;
