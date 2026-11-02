// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Data-driven factory for ESP32-P4 peripheral models.
//!
//! Every arm here parameterizes a SHARED model with the P4's values. The P4
//! reuses the C3/C6/S3 UART register head, the S3/C3 SYSTIMER layout and the
//! C3/C6 TIMG head, so the model bodies live in their shared modules and this
//! factory only supplies the P4's clock, source ids and constructor flags. The
//! two P4 blocks with no shared model (GPIO — two-bank — and CLKRST) are
//! SVD-derived `declarative` descriptors wired directly in `esp32p4.yaml`.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    let dev: Box<dyn Peripheral> = match canonical_type {
        // Real ESP-IDF UART register head + TX FIFO + interrupts, paced at the
        // P4's 360 MHz and built with the C6 register map (FSM_STATUS @0x70,
        // which the P4 has). UART0 is the module console and echoes to stdout;
        // UART1/UART2 are capture-only. Source id comes from the descriptor's
        // `irq:` or the base-address default (32/33/34).
        "esp32p4_uart" => {
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
        // SYSTIMER — same IP and register layout as the S3/C3 (esp32p4.svd
        // SYSTIMER). TARGET sources are 8..=10 on the P4 matrix; unmodelled at
        // L1 (polled smoke), value kept for the debugger.
        "esp32p4_systimer" => {
            let cpu_clock_hz = p_cfg
                .config
                .get("cpu_clock_hz")
                .and_then(|v| v.as_u64())
                .unwrap_or(super::uart::CPU_CLOCK_HZ) as u32;
            let source = p_cfg.irq.unwrap_or(8);
            Box::new(crate::peripherals::esp_systimer::Systimer::new_with_source(
                cpu_clock_hz,
                source,
            ))
        }
        // TIMG — the shared `esp32::timg::Timg` model. The P4 TIMG head
        // (T0CONFIG@0x00 … T0LOAD@0x20, WDTCONFIG0@0x48, WDTWPROTECT@0x64,
        // INT_RAW_TIMERS@0x74) is offset-identical to the C3/C6. Deliberately
        // NOT `with_mwdt()`: the P4 watchdog register block has not been
        // verified against a P4 silicon capture, so the MWDT path stays off and
        // the window is a plain GP-timer head (documented limitation). The type
        // name carries the `timer` marker the tier-1 heuristic reads.
        "esp32p4_timg" => Box::new(crate::peripherals::esp32::timg::Timg::new(
            p_cfg.base_address as u32,
        )),
        _ => return None,
    };
    Some(dev)
}

pub const SUPPORTED_TYPES: &[&str] = &["esp32p4_uart", "esp32p4_systimer", "esp32p4_timg"];
