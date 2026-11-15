// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `vendor` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test vendor <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

#[path = "../common/mod.rs"]
mod common;

mod cc2538_rom;
mod cc2538_sleep_timer_irq;
mod ch32v003_compiled_firmware;
mod ch32v005_compiled_firmware;
mod ch32v006_compiled_firmware;
mod ch32x035_compiled_firmware;
mod efr32_deck_behavior;
mod efr32_msc_persistence;
mod efr32_onboarded_parts;
mod efr32_usart_route_silicon;
mod efr32mg26_atomic_aliases;
mod efr32mg26_ble;
mod efr32mg26_clock_gating;
mod efr32mg26_instances;
mod efr32mg26_pwm_pad;
mod efr32mg26_timer_compare_level;
mod ek_rx261_zephyr;
mod kw41z_clock_boot;
mod modulshop_49213;
mod ra6m5_adc12_icu;
mod ra6m5_agt_icu;
mod ra6m5_gpt_pwm;
mod ra6m5_iic_icu;
mod ra6m5_sci_icu;
mod ra8m1_adc12_icu;
mod ra8m1_agt_icu;
mod ra8m1_gpt_pwm;
mod ra8m1_iic_icu;
mod ra8m1_irq_pin_icu;
mod ra8m1_sci_icu;
mod ra_dac12_loopback;
mod rt500_secure_map;
