// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C2 / ESP8684 UART models.
//!
//! C2 exposes the Espressif UART FIFO/status register head at the same AHB
//! base used by its UART0 console. This profile supplies the C2 clock rates to
//! the shared Espressif UART model; it does not model the UART pin matrix or
//! the rest of the C2 peripheral clock tree.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

/// ESP32-C2 UART0 interrupt-matrix source from ESP-IDF `soc.h`.
pub const UART0_INTR_SOURCE_ID: u32 = 5;
/// ESP32-C2 maximum HP-core frequency.
pub const CPU_CLOCK_HZ: u64 = 120_000_000;
/// ESP32-C2 APB clock (`APB_CLK_FREQ` in ESP-IDF `soc.h`).
pub const UART_SCLK_HZ: u64 = 80_000_000;

pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    if canonical_type != "esp32c2_uart" {
        return None;
    }

    let source_id = p_cfg.irq.unwrap_or(UART0_INTR_SOURCE_ID);
    let echo = p_cfg
        .config
        .get("echo_stdout")
        .and_then(|value| value.as_bool())
        .unwrap_or(source_id == UART0_INTR_SOURCE_ID);
    let cpu_clock_hz = p_cfg
        .config
        .get("cpu_clock_hz")
        .and_then(|value| value.as_u64())
        .unwrap_or(CPU_CLOCK_HZ);
    let uart_sclk_hz = p_cfg
        .config
        .get("uart_sclk_hz")
        .and_then(|value| value.as_u64())
        .unwrap_or(UART_SCLK_HZ);

    Some(Box::new(
        crate::peripherals::esp_uart::EspUart::new_with_cpu_clock(echo, source_id, cpu_clock_hz)
            .with_uart_sclk_hz(uart_sclk_hz),
    ))
}

pub const SUPPORTED_TYPES: &[&str] = &["esp32c2_uart"];
