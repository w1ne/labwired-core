// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-C61 UART profile for the ESP32-C61-DevKitC-1-N8R2.
//!
//! This profile exposes only the UART0 FIFO/status head used by the board
//! smoke. It does not claim GPIO, IO_MUX routing, the ROM/bootloader path, or
//! the physical USB-to-UART bridge.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

/// ESP32-C61 UART0 interrupt-matrix source from ESP-IDF `interrupts.h`.
pub const UART0_INTR_SOURCE_ID: u32 = 40;
/// ESP32-C61 HP-core maximum clock (ESP32-C61 Series Datasheet v1.1).
pub const CPU_CLOCK_HZ: u64 = 160_000_000;

pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    if canonical_type != "esp32c61_uart" {
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

    Some(Box::new(
        crate::peripherals::esp_uart::EspUart::new_with_cpu_clock(echo, source_id, cpu_clock_hz),
    ))
}

pub const SUPPORTED_TYPES: &[&str] = &["esp32c61_uart"];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn uart_cfg() -> PeripheralConfig {
        PeripheralConfig {
            id: "uart0".to_string(),
            r#type: "esp32c61_uart".to_string(),
            base_address: 0x6000_0000,
            size: Some("32B".to_string()),
            irq: Some(UART0_INTR_SOURCE_ID),
            irq_controller: None,
            clock: None,
            config: HashMap::new(),
        }
    }

    #[test]
    fn c61_uart_uses_its_interrupt_source_and_160_mhz_pacing() {
        let mut uart = try_build("esp32c61_uart", &uart_cfg()).expect("C61 UART model");
        const UART_STATUS: u64 = 0x1C;
        const UART_INT_ENA: u64 = 0x0C;
        const UART_INT_TX_DONE: u32 = 1 << 14;
        assert_eq!(uart.matrix_irq_source_id(), Some(UART0_INTR_SOURCE_ID));
        uart.write_u32(UART_INT_ENA, UART_INT_TX_DONE).unwrap();
        uart.write_u32(0x00, b'C' as u32).unwrap();

        uart.tick_elapsed(13_879);
        assert_eq!((uart.read_u32(UART_STATUS).unwrap() >> 16) & 0x3FF, 1);
        uart.tick_elapsed(1);
        assert_eq!((uart.read_u32(UART_STATUS).unwrap() >> 16) & 0x3FF, 0);

        let mut sources = Vec::new();
        uart.matrix_irq_sources_into(&mut sources);
        assert_eq!(sources, vec![UART0_INTR_SOURCE_ID]);
    }
}
