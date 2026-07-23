// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Data-driven factory for ESP32-S3 (Xtensa LX7) peripheral models.
//!
//! [`try_build`] maps a canonical peripheral `type` string + its descriptor to a
//! boxed model, letting a chip YAML assemble an ESP32-S3 through the same
//! `SystemBus::from_config` path the Cortex-M and RISC-V chips already use —
//! instead of the hand-wired `system::xtensa::configure_xtensa_esp32s3`.
//!
//! Keeping the family's arms here (not inline in `bus::from_config`) is the
//! migration pattern: each chip family owns its factory, so the central
//! `from_config` match stops growing and shrinks as families move out.
//!
//! Most S3 peripherals take an ESP-IDF interrupt *source id*
//! (`ets_isr_source_t` ordinal); it must match what the firmware's
//! interrupt-allocator expects or the ISR never dispatches. The canonical id is
//! the default and may be overridden per instance via the descriptor's `irq`
//! field (e.g. SPI2=21 vs SPI3=22, TIMG0=50 vs TIMG1=53).

use crate::Peripheral;
use labwired_config::PeripheralConfig;

/// Build an ESP32-S3 peripheral model for `canonical_type`, or `None` if this
/// family does not own that type (so the caller falls through to other
/// factories / the generic match).
pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    use super::*;

    // Read the interrupt source id from the descriptor, defaulting to the
    // canonical ESP-IDF ordinal for this peripheral when the YAML omits it.
    let src = |default: u32| p_cfg.irq.unwrap_or(default);

    let dev: Box<dyn Peripheral> = match canonical_type {
        "esp32s3_uart" => {
            // uart0 echoes to stdout (source 27); uart1/uart2 do not (28/29).
            let echo = p_cfg
                .config
                .get("echo_stdout")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            Box::new(uart::Esp32s3Uart::new(echo, src(27)))
        }
        "esp32s3_timer_group" => {
            // TIMG0 base source id 50, TIMG1 = 53; CPU clock drives the
            // RTCCALICFG auto-RDY the 2nd-stage bootloader polls.
            let cpu_clock_hz = p_cfg
                .config
                .get("cpu_clock_hz")
                .and_then(|v| v.as_u64())
                .unwrap_or(240_000_000) as u32;
            Box::new(timer_group::Esp32s3TimerGroup::new(src(50), cpu_clock_hz))
        }
        "esp32s3_gdma" => Box::new(gdma::Esp32s3Gdma::new(src(66))),
        "esp32s3_spi" => Box::new(gpspi::Esp32s3Spi::new(src(21))),
        "esp32s3_i2s" => Box::new(i2s::Esp32s3I2s::new(src(25))),
        "esp32s3_mcpwm" => Box::new(mcpwm::Esp32s3Mcpwm::new(src(38))),
        "esp32s3_rmt" => Box::new(rmt::Esp32s3Rmt::new(src(40))),
        "esp32s3_pcnt" => Box::new(pcnt::Esp32s3Pcnt::new(src(41))),
        "esp32s3_lcd_cam" => Box::new(lcd_cam::Esp32s3LcdCam::new(src(24))),
        "esp32s3_sdmmc" => Box::new(sdmmc::Esp32s3Sdmmc::new(src(36))),
        "esp32s3_twai" => Box::new(twai::Esp32s3Twai::new(src(37))),
        "esp32s3_usb_otg" => Box::new(usb_otg::Esp32s3UsbOtg::new(src(23))),
        "esp32s3_sar_adc" => Box::new(sar_adc::Esp32s3SarAdc::new(src(64))),
        "esp32s3_aes" => Box::new(aes::Esp32s3Aes::new(src(77))),
        "esp32s3_rsa" => Box::new(rsa::Esp32s3Rsa::new(src(76))),
        "esp32s3_hmac" => Box::new(hmac::Esp32s3Hmac::new(src(0))),
        "esp32s3_ds" => Box::new(ds::Esp32s3Ds::new(src(0))),
        "esp32s3_sha" => Box::new(sha::Esp32s3Sha::new()),
        "esp32s3_rng" => Box::new(rng::Esp32s3Rng::new()),
        "esp32s3_ledc" => Box::new(ledc::Esp32s3Ledc::new()),
        "esp32s3_sens" => Box::new(sens::Esp32s3Sens::new()),
        "esp32s3_extmem" => Box::new(extmem::Esp32s3Extmem::new()),
        "esp32s3_system" => Box::new(system::Esp32s3System::new()),
        "esp32s3_system_stub" => {
            Box::new(crate::peripherals::esp_xtensa_common::system_stub::SystemStub::new())
        }
        "esp32s3_core1_control" => Box::new(core1_control::Esp32s3Core1Control::new()),
        "esp32s3_crosscore_ipi" => Box::new(crosscore_ipi::Esp32s3CrossCoreIpi::new()),
        "esp32s3_gpio" => Box::new(gpio::Esp32s3Gpio::new()),
        "esp32s3_io_mux" => Box::new(io_mux::Esp32s3IoMux::new()),
        "esp32s3_usb_serial_jtag" => Box::new(usb_serial_jtag::UsbSerialJtag::new()),
        "esp32s3_systimer" => {
            let cpu_clock_hz = p_cfg
                .config
                .get("cpu_clock_hz")
                .and_then(|v| v.as_u64())
                .unwrap_or(240_000_000) as u32;
            // Stay on the per-cycle walk. `Systimer::new` defaults to
            // scheduler-driven mode; without the `event-scheduler` feature the
            // walk skips `uses_scheduler()` models, so the FreeRTOS tick alarm
            // never advances → idle/`vTaskDelay` hang, loopTask stuck after
            // first `delay()` (Arduino setup never runs).
            Box::new(systimer::Systimer::new_with_source_legacy_tick(
                cpu_clock_hz,
                57, // ETS_SYSTIMER_TARGET0_INTR_SOURCE
            ))
        }
        // I2C0 default source 42, I2C1 = 43 (ETS_I2C_EXT{0,1}_INTR_SOURCE). The
        // built controller has no I²C slaves attached; board-specific slaves
        // (e.g. TMP102) are attached by the caller after construction.
        "esp32s3_i2c" => Box::new(i2c::Esp32s3I2c::with_intr_source(src(42))),
        _ => return None,
    };
    Some(dev)
}

/// Canonical type strings this factory owns. Kept next to [`try_build`] so the
/// two cannot drift; the migration's Stage-3 chip YAML draws from this set.
pub const SUPPORTED_TYPES: &[&str] = &[
    "esp32s3_uart",
    "esp32s3_timer_group",
    "esp32s3_gdma",
    "esp32s3_spi",
    "esp32s3_i2s",
    "esp32s3_mcpwm",
    "esp32s3_rmt",
    "esp32s3_pcnt",
    "esp32s3_lcd_cam",
    "esp32s3_sdmmc",
    "esp32s3_twai",
    "esp32s3_usb_otg",
    "esp32s3_sar_adc",
    "esp32s3_aes",
    "esp32s3_rsa",
    "esp32s3_hmac",
    "esp32s3_ds",
    "esp32s3_sha",
    "esp32s3_rng",
    "esp32s3_ledc",
    "esp32s3_sens",
    "esp32s3_extmem",
    "esp32s3_system",
    "esp32s3_system_stub",
    "esp32s3_core1_control",
    "esp32s3_crosscore_ipi",
    "esp32s3_gpio",
    "esp32s3_io_mux",
    "esp32s3_usb_serial_jtag",
    "esp32s3_systimer",
    "esp32s3_i2c",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(ty: &str) -> PeripheralConfig {
        PeripheralConfig {
            id: ty.to_string(),
            r#type: ty.to_string(),
            base_address: 0x6000_0000,
            size: None,
            irq: None,
            clock: None,
            config: HashMap::new(),
        }
    }

    /// Every ESP32-S3 model that `configure_xtensa_esp32s3` wires by hand must be
    /// buildable from a descriptor, so a chip YAML can assemble the SoC through
    /// `from_config` instead.
    #[test]
    fn builds_every_supported_type() {
        for ty in SUPPORTED_TYPES {
            assert!(
                try_build(ty, &cfg(ty)).is_some(),
                "factory should build {ty}"
            );
        }
    }

    /// Types this family does not own must return `None` so `from_config` falls
    /// through to other factories / the generic match.
    #[test]
    fn declines_foreign_types() {
        for ty in ["uart", "esp32_timg", "i2c", "nrf52840_uart", "unknown_xyz"] {
            assert!(try_build(ty, &cfg(ty)).is_none(), "{ty} is not ours");
        }
    }

    /// A descriptor `irq` overrides the canonical default (e.g. SPI2=21 vs
    /// SPI3=22) without panicking — the per-instance path the YAML relies on.
    #[test]
    fn honors_descriptor_irq_override() {
        let mut c = cfg("esp32s3_spi");
        c.irq = Some(22);
        assert!(try_build("esp32s3_spi", &c).is_some());
    }
}
