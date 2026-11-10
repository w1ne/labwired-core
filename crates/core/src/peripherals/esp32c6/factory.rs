// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Data-driven factory for ESP32-C6 peripheral models and the narrow ESP32-H2
//! GPIO output alias.
//!
//! Every C6 arm either parameterizes a shared model with the C6's values or
//! aliases the C3 model whose register map the C6 genuinely shares. The H2
//! alias is limited to the direct GPIO output register path used by its board
//! smoke and is not a general H2 peripheral implementation. The C6 has no
//! C3-only `system` / `rtc_cntl` / `apb_ctrl` blocks; its PCR clock/reset and
//! IO_MUX windows are SVD-derived `declarative` descriptors in `esp32c6.yaml`.

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
        // matrix numbering is its own (I2CEXT0_SCL/SDA 45/46 against the C3's
        // 53/54, `FUNCn_IN_SEL_CFG.SEL` at bit 7 against bit 6), so the model
        // is built with the C6 layout; the FSPI and UART TX numbers match.
        // Its GPIO interrupt is matrix source 30 (esp32c6.svd), not the C3's 16.
        "esp32c6_gpio" => Box::new(crate::peripherals::esp32c3::gpio::Esp32c3Gpio::for_esp32c6()),
        // The C61 GPIO block has its own register head and selector placement;
        // this profile covers DevKitC GPIO4/5 I2C0, GPIO6 LEDC, and GPIO8 RGB LED.
        "esp32c61_gpio" => Box::new(crate::peripherals::esp32c3::gpio::Esp32c3Gpio::for_esp32c61()),
        "esp32c61_io_mux" => Box::new(super::io_mux::Esp32c61IoMux::new()),
        // C61 LEDC uses the shared waveform/timer engine with its own register
        // field layout and reset-selected XTAL40M source.
        "esp32c61_ledc" => {
            let source = p_cfg.irq.unwrap_or(43);
            Box::new(crate::peripherals::esp32c3::ledc::Esp32c3Ledc::new_esp32c61(source))
        }
        // The H2 smoke uses only direct GPIO output on GPIO8. Espressif's
        // esp32h2.svd and esp32c6.svd agree on the GPIO base (0x6009_1000) and
        // the OUT/W1TS/W1TC, ENABLE/W1TS, and IN register offsets used here.
        // This alias deliberately does not claim H2 matrix numbering,
        // interrupt sources, or GPIO26..GPIO27.
        "esp32h2_gpio" => Box::new(crate::peripherals::esp32c3::gpio::Esp32c3Gpio::for_esp32c6()),
        // PCR — the C6's clock/reset block. Register-backed (full SVD map) and
        // the clock controller the chip yaml's `clock:` gates resolve through
        // (`Peripheral::clock_gate_reg_offset`); see super::pcr for exactly
        // what is and is not enforced.
        "esp32c6_pcr" => Box::new(super::pcr::Esp32c6Pcr::new()),
        // The C61 moves I2C_CONF/I2C_SCLK_CONF to +0x2C/+0x30, so it has a
        // separate narrow PCR profile for its DevKitC UART0/I2C0 smoke.
        "esp32c61_pcr" => Box::new(super::pcr::Esp32c61Pcr::new()),
        // C6's two-channel RMT TX path, symbol RAM, TX-end status and timed
        // output through GPIO matrix signals 71/72. Source 49 is ETS_RMT_INTR_SOURCE.
        "esp32c6_rmt" => Box::new(super::rmt::Esp32c6Rmt::new(p_cfg.irq.unwrap_or(49))),
        // MCPWM0 timer/operator engine. The shared state engine selects the
        // C6 register layout: START/MOD in CFG1, C6 interrupt bits, and
        // PWM0_OUT0A..2B matrix signals 87..92.
        "esp32c6_mcpwm" => Box::new(
            crate::peripherals::esp32s3::mcpwm::Esp32s3Mcpwm::new_esp32c6(
                p_cfg
                    .irq
                    .unwrap_or(crate::peripherals::esp32s3::mcpwm::ESP32C6_MCPWM0_INTR_SOURCE),
                160_000_000,
            ),
        ),
        // EXTMEM / L1 cache — launch/done handshakes for Arduino cache_hal.
        "esp32c6_extmem" => Box::new(super::extmem::Esp32c6Extmem::new()),
        // SYSTIMER — same IP as C3/S3. TARGET0 matrix source 57 on C6.
        "esp32c6_systimer" => {
            let cpu_clock_hz = p_cfg
                .config
                .get("cpu_clock_hz")
                .and_then(|v| v.as_u64())
                .unwrap_or(160_000_000) as u32;
            let source = p_cfg.irq.unwrap_or(57);
            Box::new(crate::peripherals::esp_systimer::Systimer::new_with_source(
                cpu_clock_hz,
                source,
            ))
        }
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
        // MODEM_LPCON — sticky regs; seed clk_i2c_mst_en for fast-boot.
        "esp32c6_modem_lpcon" => Box::new(super::modem_lpcon::Esp32c6ModemLpcon::new()),
        // Analog I2C master — sticky + instant-complete BUSY/cal bits.
        "esp32c6_i2c_ana_mst" => Box::new(super::i2c_ana_mst::Esp32c6I2cAnaMst::new()),
        // USB Serial/JTAG — same EP1/INT_* IP as C3/S3 (`UsbSerialJtag`), with
        // the C6 matrix source (48) and 160 MHz clock. Base is owned by the
        // chip yaml (`0x6000_F000`); see the model module docs.
        "esp32c6_usb_serial_jtag" => {
            Box::new(crate::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag::new_esp32c6())
        }
        _ => return None,
    };
    Some(dev)
}

pub const SUPPORTED_TYPES: &[&str] = &[
    "esp32c6_uart",
    "esp32c6_gpio",
    "esp32c61_gpio",
    "esp32c61_io_mux",
    "esp32c61_ledc",
    "esp32h2_gpio",
    "esp32c6_pcr",
    "esp32c61_pcr",
    "esp32c6_rmt",
    "esp32c6_mcpwm",
    "esp32c6_extmem",
    "esp32c6_systimer",
    "esp32c6_gdma",
    "esp32c6_mwdt",
    "esp32c6_lp_rtc",
    "esp32c6_modem_lpcon",
    "esp32c6_i2c_ana_mst",
    "esp32c6_usb_serial_jtag",
];
