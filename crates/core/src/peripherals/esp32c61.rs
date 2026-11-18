// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-C61 UART, GPIO-output, and I2C-matrix profile for the
//! ESP32-C61-DevKitC-1-N8R2.
//!
//! The GPIO adapter exposes the TRM-backed output latch and enable registers,
//! plus the I2C0 matrix route used on GPIO6/GPIO22. IO_MUX is storage only;
//! ROM/bootloader behavior and the physical USB bridge are outside this slice.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

/// ESP32-C61 UART0 interrupt-matrix source from ESP-IDF `interrupts.h`.
pub const UART0_INTR_SOURCE_ID: u32 = 40;
/// ESP32-C61 HP-core maximum clock (ESP32-C61 Series Datasheet v1.1).
pub const CPU_CLOCK_HZ: u64 = 160_000_000;

pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    if canonical_type == "esp32c61_gpio" {
        return Some(Box::new(
            crate::peripherals::esp32c3::gpio::Esp32c3Gpio::for_esp32c61(),
        ));
    }
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

pub const SUPPORTED_TYPES: &[&str] = &["esp32c61_gpio", "esp32c61_uart"];

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

    #[test]
    fn c61_gpio_uses_shifted_enable_registers_and_updates_gpio4_output() {
        let mut gpio_cfg = uart_cfg();
        gpio_cfg.id = "gpio".to_string();
        gpio_cfg.r#type = "esp32c61_gpio".to_string();
        gpio_cfg.base_address = 0x6009_1000;
        gpio_cfg.size = Some("104B".to_string());
        let mut gpio = try_build("esp32c61_gpio", &gpio_cfg).expect("C61 GPIO model");

        // C61 TRM: OUT_W1TS@0x08, ENABLE_W1TS@0x38, OUT@0x04,
        // ENABLE@0x34. GPIO4 is J1 pin 3 on the DevKitC-1 v2.0.
        gpio.write_u32(0x38, 1 << 4).unwrap();
        gpio.write_u32(0x08, 1 << 4).unwrap();
        assert_eq!(gpio.read_u32(0x34).unwrap(), 1 << 4);
        assert_eq!(gpio.read_u32(0x04).unwrap(), 1 << 4);
    }
}
