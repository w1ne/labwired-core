// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Data-driven factory for ESP32-C3 peripheral models.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    let dev: Box<dyn Peripheral> = match canonical_type {
        // Bluetooth LE link-layer / baseband (0x6003_1000). Register-backed
        // window + the Bluetooth native clock the BT ROM's event scheduler
        // reads; reverse-engineered from live silicon, see `super::bt`.
        "esp32c3_bt" => Box::new(super::bt::Esp32c3Bt::new()),
        "esp32c3_gpio" => Box::new(super::gpio::Esp32c3Gpio::new()),
        "esp32c3_io_mux" => Box::new(super::io_mux::Esp32c3IoMux::new()),
        // RMT TX model (Arduino RGB_BUILTIN / rgbLedWrite). Instant TX_END +
        // LogicTap pulse on TX_START for matrix `led_watch: rmt:0`.
        "esp32c3_rmt" => Box::new(super::rmt::Esp32c3Rmt::new_default()),
        // SYSTIMER (0x6002_3000): the C3 block is the same IP as the S3's (all
        // 30 C3 registers agree offset-for-offset), so the S3 model serves it —
        // a live 16 MHz counter with the UNITx_OP update / VALUE_VALID
        // handshake esp-hal's `Delay` and `esp_timer` poll, plus the three
        // comparators. Only the matrix sources differ: C3 TARGET0..2 are
        // 37..39. Clocked from the core clock (`cpu_clock_hz`, default the
        // C3's 160 MHz: 10 CPU cycles per tick). This is the model the ROM-boot
        // path installs too, so a bare-ELF fast boot and a world node see the
        // same timer as a faithful boot.
        "esp32c3_systimer" => {
            let cpu_clock_hz = p_cfg
                .config
                .get("cpu_clock_hz")
                .and_then(|v| v.as_u64())
                .unwrap_or(160_000_000) as u32;
            Box::new(crate::peripherals::esp_systimer::Systimer::new_with_source(
                cpu_clock_hz,
                SYSTIMER_TARGET0_SOURCE,
            ))
        }
        // Real ESP-IDF UART register map + TX FIFO + interrupts. UART0 is the
        // Arduino `Serial` console and echoes to stdout; UART1 is
        // capture-only. Source id defaults from the base address (see
        // `super::uart::default_source_id`) and an explicit `irq:` wins.
        "esp32c3_uart" => {
            let source_id = p_cfg
                .irq
                .unwrap_or_else(|| super::uart::default_source_id(p_cfg.base_address));
            let echo = p_cfg
                .config
                .get("echo_stdout")
                .and_then(|v| v.as_bool())
                .unwrap_or(source_id == super::uart::UART0_INTR_SOURCE_ID);
            Box::new(super::uart::new(echo, source_id))
        }
        _ => return None,
    };
    Some(dev)
}

pub const SUPPORTED_TYPES: &[&str] = &[
    "esp32c3_bt",
    "esp32c3_gpio",
    "esp32c3_io_mux",
    "esp32c3_rmt",
    "esp32c3_systimer",
    "esp32c3_uart",
];

/// ESP32-C3 interrupt-matrix source of SYSTIMER TARGET0 (TARGET1/2 follow at
/// 38/39) — `ETS_SYSTIMER_TARGET0_EDGE_INTR_SOURCE`.
pub const SYSTIMER_TARGET0_SOURCE: u32 = 37;
