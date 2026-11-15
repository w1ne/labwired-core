// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-P4 UART models.
//!
//! The P4 HP UART FIFO/status head matches the shared Espressif UART model
//! through offset 0x1C. This family wrapper supplies the P4 interrupt source,
//! 400 MHz HP-core clock, and 90 MHz APB/UART source clock. The P4 board
//! descriptor limits the exposed window to FIFO and STATUS; UART clock/reset,
//! IO_MUX routing, and USB Serial/JTAG are outside this profile.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

/// ESP32-P4 UART0 interrupt number from ESP-IDF `soc.h`.
pub const UART0_INTR_SOURCE_ID: u32 = 5;
/// ESP32-P4 HP-core maximum frequency.
pub const CPU_CLOCK_HZ: u64 = 400_000_000;
/// ESP32-P4 APB clock (`APB_CLK_FREQ` in ESP-IDF `soc.h`).
pub const UART_SCLK_HZ: u64 = 90_000_000;

pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    if canonical_type != "esp32p4_uart" {
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

pub const SUPPORTED_TYPES: &[&str] = &["esp32p4_uart"];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn uart_cfg() -> PeripheralConfig {
        let mut config = HashMap::new();
        config.insert("echo_stdout".to_string(), serde_yaml::Value::Bool(false));
        config.insert(
            "cpu_clock_hz".to_string(),
            serde_yaml::Value::Number(CPU_CLOCK_HZ.into()),
        );
        config.insert(
            "uart_sclk_hz".to_string(),
            serde_yaml::Value::Number(UART_SCLK_HZ.into()),
        );
        PeripheralConfig {
            id: "uart0".to_string(),
            r#type: "esp32p4_uart".to_string(),
            base_address: 0x500C_A000,
            size: Some("32B".to_string()),
            irq: Some(UART0_INTR_SOURCE_ID),
            irq_controller: None,
            clock: None,
            config,
        }
    }

    #[test]
    fn p4_uart_uses_its_source_and_90_mhz_apb_pacing() {
        let mut uart = try_build("esp32p4_uart", &uart_cfg()).expect("P4 UART model");
        uart.write_u32(0x00, u32::from(b'P')).unwrap();

        // RESET_CLKDIV is 694. One 8N1 byte is floor(10 * 694 * 400 / 90)
        // = 30,844 P4 core cycles.
        uart.tick_elapsed(30_843);
        assert_eq!((uart.read_u32(0x1C).unwrap() >> 16) & 0x3FF, 1);
        uart.tick_elapsed(1);
        assert_eq!((uart.read_u32(0x1C).unwrap() >> 16) & 0x3FF, 0);

        assert_eq!(
            uart.matrix_irq_source_id(),
            Some(UART0_INTR_SOURCE_ID),
            "descriptor IRQ must be exposed as the UART matrix source"
        );
    }
}
