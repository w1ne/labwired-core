// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-C2 peripheral adapter for the ESP8684-DevKitM-1-H4X profile.
//!
//! The C2 GPIO and UART register heads used here match the C3 models. The
//! adapter supplies the C2's own CPU/APB clocks and UART interrupt source.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

/// ESP32-C2 UART0 interrupt-matrix source from ESP-IDF `interrupts.h`.
pub const UART0_INTR_SOURCE_ID: u32 = 17;
/// ESP32-C2 CPU clock used by this profile.
pub const CPU_CLOCK_HZ: u64 = 120_000_000;
/// ESP32-C2 APB/UART source clock (`APB_CLK_FREQ` in ESP-IDF `soc.h`).
pub const UART_SCLK_HZ: u64 = 40_000_000;

pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    match canonical_type {
        "esp32c2_gpio" => Some(Box::new(
            crate::peripherals::esp32c3::gpio::Esp32c3Gpio::new(),
        )),
        "esp32c2_uart" => {
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
                crate::peripherals::esp_uart::EspUart::new_with_cpu_clock(
                    echo,
                    source_id,
                    cpu_clock_hz,
                )
                .with_uart_sclk_hz(uart_sclk_hz),
            ))
        }
        _ => None,
    }
}

pub const SUPPORTED_TYPES: &[&str] = &["esp32c2_gpio", "esp32c2_uart"];
