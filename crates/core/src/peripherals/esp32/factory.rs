// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Data-driven factory for classic ESP32 (Xtensa LX6) peripheral models.
//!
//! Mirrors `peripherals::esp32s3::factory`: [`try_build`] maps a canonical
//! peripheral `type` string + its descriptor to a boxed model, so the LX6
//! peripheral set can be assembled from a table instead of hand-wired
//! `add_peripheral` calls in `system::xtensa::configure_xtensa_esp32`.
//!
//! UART takes an ESP-IDF interrupt source id (`ETS_UART{0,1,2}_INTR_SOURCE` =
//! 34/35/36) and an echo flag (UART0 echoes the console). The timer-group, LEDC
//! and MCPWM models take their register base, read from the descriptor.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

/// Build a classic-ESP32 peripheral model for `canonical_type`, or `None` if
/// this family does not own that type (so the caller falls through).
pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    use super::*;

    let base = p_cfg.base_address as u32;
    let dev: Box<dyn Peripheral> = match canonical_type {
        "esp32_uart" => {
            // UART0 echoes TX to the host console (source 34); UART1/2 are
            // capture-only (35/36).
            let echo = p_cfg
                .config
                .get("echo_stdout")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            Box::new(uart::Esp32Uart::new(echo, p_cfg.irq.unwrap_or(34)))
        }
        "esp32_spi" => Box::new(spi::Esp32Spi::new()),
        "esp32_sar_adc" => Box::new(sar_adc::Esp32SarAdc::new()),
        "esp32_i2c" => Box::new(i2c::Esp32I2c::new()),
        "esp32_gpio" => Box::new(gpio::Esp32Gpio::new()),
        "esp32_dport" => Box::new(dport::Dport::new()),
        "esp32_sha" => Box::new(sha::Sha::new()),
        "esp32_rtc_cntl" => Box::new(rtc_cntl::RtcCntl::new()),
        "esp32_timg" => Box::new(timg::Timg::new(base)),
        "esp32_efuse" => Box::new(efuse::Efuse::new()),
        "esp32_syscon" => Box::new(syscon::Syscon::new()),
        "esp32_ledc" => Box::new(ledc::Ledc::new(base)),
        "esp32_twai" => Box::new(twai::Esp32Twai::new()),
        "esp32_mcpwm" => Box::new(mcpwm::Mcpwm::new(base)),
        "esp32_sdio" => Box::new(sdio_stub::HostSlc::new()),
        _ => return None,
    };
    Some(dev)
}

/// Canonical type strings this factory owns. Kept next to [`try_build`].
pub const SUPPORTED_TYPES: &[&str] = &[
    "esp32_uart",
    "esp32_spi",
    "esp32_sar_adc",
    "esp32_i2c",
    "esp32_gpio",
    "esp32_dport",
    "esp32_sha",
    "esp32_rtc_cntl",
    "esp32_timg",
    "esp32_efuse",
    "esp32_syscon",
    "esp32_ledc",
    "esp32_twai",
    "esp32_mcpwm",
    "esp32_sdio",
];
