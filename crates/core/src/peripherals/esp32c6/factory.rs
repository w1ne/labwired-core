// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Data-driven factory for ESP32-C6 peripheral models.
//!
//! Every arm here either parameterizes a shared model with the C6's values or
//! aliases the C3 model whose register map the C6 genuinely shares. The C6 has
//! no C3-only `system` / `rtc_cntl` / `apb_ctrl` blocks and no C6-specific
//! behavioral model yet; the clock/reset (PCR) and IO_MUX windows are
//! SVD-derived `declarative` descriptors wired directly in `esp32c6.yaml`.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    let dev: Box<dyn Peripheral> = match canonical_type {
        // Real ESP-IDF UART register head + TX FIFO + interrupts, paced at the
        // C6's 160 MHz. UART0 is the DevKitC-1's USB-UART bridge console and
        // echoes to stdout; UART1 is capture-only. Source id comes from the
        // descriptor's `irq:` (43/44 on the C6) or the base-address default.
        "esp32c6_uart" => {
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
        // The C6 GPIO matrix shares the C3 model's register head (OUT/W1TS/W1TC
        // @0x04-0x0C, ENABLE @0x20, STRAP @0x38, IN @0x3C, FUNC0_OUT_SEL_CFG
        // @0x554, FUNC0_IN_SEL_CFG @0x154 — cross-checked against esp-idf v5.3
        // `soc/esp32c6/include/soc/gpio_reg.h`). It is sized for the C3's
        // 26-pin block, so C6 GPIO26..GPIO30 are not modelled. The C6's
        // I2C-matrix signal indices differ from the C3's (I2CEXT0_SCL is 45 vs
        // 53), so the C3 I2C pad wiring does not apply — UART0/UART1 TX indices
        // (6/9) happen to match and do.
        // Its GPIO interrupt is matrix source 30 (esp32c6.svd), not the C3's 16.
        "esp32c6_gpio" => {
            Box::new(crate::peripherals::esp32c3::gpio::Esp32c3Gpio::with_intr_source(30))
        }
        // PCR — the C6's clock/reset block. Register-backed (full SVD map) and
        // the clock controller the chip yaml's `clock:` gates resolve through
        // (`Peripheral::clock_gate_reg_offset`); see super::pcr for exactly
        // what is and is not enforced.
        "esp32c6_pcr" => Box::new(super::pcr::Esp32c6Pcr::new()),
        // GDMA — the C6's 3-channel general DMA. `irq:` carries the
        // interrupt-matrix source of IN channel 0 (66 on the C6; the descriptor
        // declares it). Source ids derive contiguously: IN_CHn = base + n,
        // OUT_CHn = base + 3 + n.
        "esp32c6_gdma" => {
            let source = p_cfg.irq.unwrap_or(super::gdma::DMA_IN_CH0_INTR_SOURCE_ID);
            Box::new(super::gdma::Esp32c6Gdma::new(source))
        }
        // The C6 TIMG with the MWDT path armed. Same shared `esp32::timg::Timg`
        // as the C3 (`esp32_timg`): the GP-timer head is offset-identical and
        // the C6 carries one timer per group. Two C6 deltas are parameterized
        // here rather than in the chip yaml:
        //   * `with_mwdt` selects the C3/C6 WDT register layout (WDTCONFIG0..5,
        //     WDTFEED@0x60, WDTWPROTECT@0x64) and the real stage-0
        //     countdown/feed/INT-latch path. The C3 yaml names `esp32_timg`,
        //     which never opts in — that is the chip gate.
        //   * the RTC_SLOW calibration profile is the C3's measured constant,
        //     carried over unchanged from the previous `esp32_timg` wiring so
        //     this move does not silently change RTCCALICFG on the C6. It is
        //     NOT a C6 measurement (known limitation, see the board docs).
        //   * the type name carries the "wdt" marker the tier-1 class
        //     heuristic reads (`id: timg0` supplies "timer").
        "esp32c6_mwdt" => {
            use crate::peripherals::esp32c3::rtc_timer::{C3_XTAL_HZ, RTC_SLOW_HZ_MEASURED};
            Box::new(
                crate::peripherals::esp32::timg::Timg::new(p_cfg.base_address as u32)
                    .with_rtc_cal(crate::peripherals::esp32::timg::RtcCalProfile {
                        xtal_hz: C3_XTAL_HZ,
                        slow_hz: RTC_SLOW_HZ_MEASURED,
                    })
                    .with_mwdt(),
            )
        }
        // LP_TIMER — the C6's RTC main timer (`rtc_time_get`). Its register
        // shape is C6-only (see `lp_timer`), so it is a dedicated model rather
        // than a reuse of the C3 RTC_CNTL timer.
        "esp32c6_lp_rtc" => Box::new(super::lp_timer::Esp32c6LpTimer::new()),
        _ => return None,
    };
    Some(dev)
}

pub const SUPPORTED_TYPES: &[&str] = &[
    "esp32c6_uart",
    "esp32c6_gpio",
    "esp32c6_pcr",
    "esp32c6_gdma",
    "esp32c6_mwdt",
    "esp32c6_lp_rtc",
];
