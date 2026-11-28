// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `analog` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test analog <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

#[path = "../common/mod.rs"]
mod common;

mod analog_adapter;
mod analog_beta_thermistor;
mod analog_coil_series;
mod analog_copper_capacity;
mod analog_devices;
mod analog_diode_current;
mod analog_diode_series_conditioning;
mod analog_linear_golden;
mod analog_macros;
mod analog_motor_circuit;
mod analog_mux_e2e;
mod analog_netlist;
mod analog_ngspice_macros;
mod analog_photoresistor;
mod analog_pin_channel_counts;
mod analog_pin_load;
mod analog_plant_migration_parity;
mod analog_solver;
mod analog_stimulus;
mod analog_variable_resistance;
mod analog_voltage_switch;
mod circuit_bldc;
mod cosim_at32f421_analog;
mod cosim_avr_atmega328p_analog;
mod cosim_avr_atmega32u4_analog;
mod cosim_avr_atmega4809_analog;
mod cosim_avr_atmega_mega_analog;
mod cosim_ch32v003_analog;
mod cosim_ek_ra_adc_pad_analog;
mod cosim_esp32_analog_diode;
mod cosim_esp32_classic_boards_analog;
mod cosim_esp32c3_boards_analog;
mod cosim_esp32c5_boards_analog;
mod cosim_esp32c61_boards_analog;
mod cosim_esp32c6_boards_analog;
mod cosim_esp32s3_boards_analog;
mod cosim_microbit_v2_analog;
mod cosim_nrf52840_family_analog;
mod cosim_nucleo_f103rb_analog;
mod cosim_nucleo_f401re_analog;
mod cosim_nucleo_f767zi_analog;
mod cosim_ra4m1_analog;
mod cosim_rp2040_family_analog;
mod cosim_samd21_family_analog;
mod cosim_stm32f1f4_catalog_analog;
mod cosim_stm32f4_family_analog;
mod cosim_stm32g0b1_analog;
mod cosim_stm32h7_analog;
mod cosim_stm32l476_analog;
mod cosim_stm32l4u5_pin_functions_analog;
mod cosim_stm32wb_g4_l0_analog;
mod declarative_noise;
mod declarative_spi_supply_por;
mod device_supply_rails;
mod device_supply_routing;
mod lm393_moisture_dout;
mod mcp9701_analog;
mod motor_models;
mod mpu6050_noise;
mod ne555_astable;
mod noise_model;
mod power_routing;
mod rain_sensor_analog;
mod shared_supply_domains;
mod sound_sensor_analog;
mod supply_supervisor;
mod world_analog;
mod world_circuit_bldc;
