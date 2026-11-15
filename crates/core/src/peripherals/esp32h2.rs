// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-H2 peripheral profile for the ESP32-H2-DevKitM-1-N4S.
//!
//! UART0's register map and the first three PCR words match the C6 at the
//! offsets used by this profile. The UART window stops before H2's different
//! `CLK_CONF` reset value at 0x88; the PCR window stops after
//! `UART0_PD_CTRL` at 0x08 because the rest of the H2 PCR map diverges from the
//! C6. The board descriptor also limits GPIO to the common data/enable head.
//!
//! This supports a bare-ELF UART console smoke. It does not model the mask-ROM
//! boot path, IO_MUX pad electrical routing, the USB-to-UART bridge, the
//! WS2812 RGB LED protocol, or the H2 radio blocks.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

/// ESP32-H2 UART0 interrupt-matrix source from Espressif's `esp32h2.svd`.
pub const UART0_INTR_SOURCE_ID: u32 = 33;
/// ESP32-H2 HP-core clock (ESP32-H2 Series Datasheet v1.3, up to 96 MHz).
pub const CPU_CLOCK_HZ: u64 = 96_000_000;

pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    let dev: Box<dyn Peripheral> = match canonical_type {
        // The H2's UART registers 0x00..0x84 match the C6 offsets. Its
        // CLK_CONF@0x88 reset value differs, so the H2 chip descriptor maps
        // only the common prefix (size 0x88). UART pacing uses the H2 core
        // clock, not the C6's 160 MHz.
        "esp32h2_uart" => {
            let source_id = p_cfg.irq.unwrap_or(UART0_INTR_SOURCE_ID);
            let echo = p_cfg
                .config
                .get("echo_stdout")
                .and_then(|v| v.as_bool())
                .unwrap_or(source_id == UART0_INTR_SOURCE_ID);
            let cpu_clock_hz = p_cfg
                .config
                .get("cpu_clock_hz")
                .and_then(|v| v.as_u64())
                .unwrap_or(CPU_CLOCK_HZ);
            Box::new(
                crate::peripherals::esp_uart::EspUart::new_with_cpu_clock(
                    echo,
                    source_id,
                    cpu_clock_hz,
                )
                .with_c6_reg_map(),
            )
        }
        // UART0_CONF, UART0_SCLK_CONF and UART0_PD_CTRL share offsets and reset
        // values with the C6 PCR. The H2 descriptor limits the bus window to
        // those three words so C6-only PCR registers cannot be observed.
        "esp32h2_pcr" => crate::peripherals::esp32c6::factory::try_build("esp32c6_pcr", p_cfg)?,
        _ => return None,
    };
    Some(dev)
}

pub const SUPPORTED_TYPES: &[&str] = &["esp32h2_uart", "esp32h2_pcr"];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn uart_cfg() -> PeripheralConfig {
        PeripheralConfig {
            id: "uart0".to_string(),
            r#type: "esp32h2_uart".to_string(),
            base_address: 0x6000_0000,
            size: Some("136B".to_string()),
            irq: Some(UART0_INTR_SOURCE_ID),
            irq_controller: None,
            clock: None,
            config: HashMap::new(),
        }
    }

    #[test]
    fn uart0_uses_h2_interrupt_source_and_register_head() {
        let mut uart = try_build("esp32h2_uart", &uart_cfg()).expect("H2 UART model");
        uart.write_u32(0x14, 694).unwrap();
        assert_eq!(uart.read_u32(0x14).unwrap() & 0xFFF, 694);
        assert_eq!(uart.read_u32(0x70).unwrap() & 0xF0, 0);
        let mut sources = Vec::new();
        uart.matrix_irq_sources_into(&mut sources);
        assert!(sources.is_empty());
        uart.write_u32(0x00, b'!' as u32).unwrap();
        uart.tick_elapsed(8_327);
        assert_eq!((uart.read_u32(0x1C).unwrap() >> 16) & 0x3FF, 1);
        uart.tick_elapsed(1);
        assert_eq!((uart.read_u32(0x1C).unwrap() >> 16) & 0x3FF, 0);
    }

    #[test]
    fn pcr_uart_clock_gate_registers_match_the_h2_prefix() {
        let cfg = PeripheralConfig {
            id: "pcr".to_string(),
            r#type: "esp32h2_pcr".to_string(),
            base_address: 0x6009_6000,
            size: Some("12B".to_string()),
            irq: None,
            irq_controller: None,
            clock: None,
            config: HashMap::new(),
        };
        let pcr = try_build("esp32h2_pcr", &cfg).expect("H2 PCR UART prefix");
        assert_eq!(pcr.clock_gate_reg_offset("UART0_CONF"), Some(0));
        assert_eq!(pcr.read_u32(0).unwrap(), 1);
        assert_eq!(pcr.read_u32(4).unwrap(), 0x0070_0000);
    }
}
