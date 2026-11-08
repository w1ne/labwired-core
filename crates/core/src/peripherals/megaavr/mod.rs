// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! megaAVR 0-series peripherals (ATmega4808/4809). The CPU-side services
//! (CCP, CPUINT, the VPORT alias) live in `cpu/avr/avrxt.rs`; the PORT block
//! is the `megaavr_port` layout of `avr_gpio`.

pub mod adc;
pub mod portmux;
pub mod spi;
pub mod tc;
pub mod twi;
pub mod usart;
