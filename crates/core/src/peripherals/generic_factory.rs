// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Cross-vendor (generic Cortex-M / shared) peripheral factory.
//!
//! Owns the peripheral arms that are not specific to one chip family (UART,
//! timers, SPI, I2C, RCC, flash, DMA, ADC, …) and used to live inline in
//! `bus::from_config`. Family-specific peripherals live in their own factories
//! (`esp32s3::factory`, `esp32::factory`, `nrf52::factory`); the descriptor
//! loaders (`declarative`, `strict_ir`) stay in `from_config` itself.

use crate::bus::SystemBus;
use crate::peripherals::rcc::RccRegisterLayout;
use crate::Peripheral;
use labwired_config::{PeripheralConfig, SystemManifest};

/// Canonical model-type names — the single source of truth for "is this string
/// already a real, modelled peripheral type?".
///
/// This is the set of canonical **output** names of
/// [`crate::bus::SystemBus::canonical_peripheral_type`]: the generic core types
/// built here and by `from_config`'s descriptor loaders, plus the
/// family-specific behavioral models built by the per-vendor factories
/// (`esp32`, `esp32s3`, `nrf52`, RP2040 arms below). `canonical_peripheral_type`
/// consults this set to short-circuit any raw name that is *already* canonical,
/// so the generic fuzzy SVD-name heuristics can never mis-route a real model
/// type (e.g. coerce `esp32c3_spi`, which contains "spi", to the STM32 `spi`
/// model). Adding a new behavioral model means adding its canonical name here —
/// no more per-name identity blocks in `canonical_peripheral_type`.
///
/// Note: this is the canonical-output set, NOT every alternate input spelling
/// the factories tolerate (e.g. `stm32spi`, `stm32dma`). Alias spellings whose
/// canonical output differs from the input live in `canonical_peripheral_type`'s
/// alias table.
pub const MODEL_TYPES: &[&str] = &[
    // Generic core types (built here or by `from_config` descriptor loaders).
    "uart",
    "gpio",
    "avr_gpio",
    "rcc",
    "systick",
    "timer",
    "i2c",
    "spi",
    "exti",
    "afio",
    "dma",
    "stm32f4_dma",
    "gpdma",
    "adc",
    "pio",
    "declarative",
    "strict_ir",
    "strict_ir_internal",
    "pwr",
    "flash",
    "rng",
    "simctl",
    "crc",
    "rtc",
    "rtc_f1",
    "rtc_v3",
    "iwdg",
    "wwdg",
    "dac",
    "dbgmcu",
    "lptim",
    "quadspi",
    "sai",
    "usb_otg",
    "bxcan",
    "fdcan",
    "sdmmc",
    "comp",
    "tsc",
    "fmc",
    // RP2040 native peripherals (built here). `rp2040_adc` and `rp2040_rtc`
    // MUST be listed: the fuzzy fallbacks below match `contains("adc")` and
    // `contains("rtc")`, so without membership here they would be coerced onto
    // the STM32 ADC / RTC register maps — a silently wrong model, not an error.
    "rp2040_timer",
    "rp2040_dma",
    "rp2040_spi",
    "rp2040_i2c",
    "rp2040_pwm",
    "rp2040_adc",
    "rp2040_rtc",
    "rp2040_watchdog",
    "rp2040_io_bank0",
    "rp2040_sio",
    "rp2040_clkrst",
    "rp2040_xip_ssi",
    "rp2040_usb",
    // ESP32-C3 behavioral models (esp32 factory).
    // MUST be listed: without membership the fuzzy fallback would coerce the
    // BLE baseband window onto some unrelated register map instead of erroring.
    "esp32c3_bt",
    "esp32c3_i2c",
    "esp32c3_spi",
    "esp32c3_gpio",
    "esp32c3_io_mux",
    "esp32c3_apb_saradc",
    "esp32c3_ledc",
    "esp32c3_rmt",
    // MUST be listed: the fuzzy fallback matches `contains("uart")`, which
    // would coerce the C3's UART onto the STM32 register map — the silently
    // wrong model that wedged every `Serial.print` over 128 bytes.
    "esp32c3_uart",
    // nRF52 behavioral models (nrf52 factory).
    "nrf52840_twim",
    "nrf52_saadc",
    "nrf52_qspi",
    "nrf52840_spis",
    "nrf52840_twis",
    "nrf52840_uart",
    "nrf52_gpiote",
    // nRF54L behavioral models (nrf54l factory). Listed here so the fuzzy
    // `contains("uart")` heuristic cannot coerce `nrf54l_uarte` onto the
    // generic STM32 UART layout — it is a distinct silicon register map.
    "nrf54l_uarte",
    "nrf54l_twim",
    // ⚠️ Load-bearing. Without this entry the fuzzy `contains("spi")` heuristic
    // coerces `nrf54l_spim` onto the shared `spi` arm, which then sees
    // `contains("nrf")` and picks the nRF52 SPIM offset map. That failure is
    // silent: ENABLE (0x500) and CONFIG (0x554) are at the same addresses on
    // both generations, so the instance enables and configures cleanly and then
    // never sees a start task, because 0x000 means nothing to the nRF52 map.
    "nrf54l_spim",
    // ESP32-classic behavioral models (esp32 factory). Absent while every C3
    // sibling was listed: only the Xtensa builder ever built these, and it
    // registers the bank in Rust without consulting this table, so the gap was
    // invisible until a chip YAML declared one for a plain `from_config` bus.
    "esp32_dport",
    "esp32_efuse",
    "esp32_gpio",
    "esp32_i2c",
    "esp32_ledc",
    "esp32_mcpwm",
    "esp32_rtc_cntl",
    "esp32_sar_adc",
    "esp32_sdio",
    "esp32_sha",
    "esp32_spi",
    "esp32_syscon",
    "esp32_timg",
    "esp32_twai",
    "esp32_uart",
    // ESP32-S3 behavioral models (esp32s3 factory). The whole family was absent,
    // with the same latent hazard: `esp32s3_spi`, `esp32s3_i2c`, `esp32s3_rng`
    // and `esp32s3_sdmmc` each contain a generic substring, so the fuzzy chain
    // would coerce them onto STM32 register maps without a word.
    "esp32s3_aes",
    "esp32s3_core1_control",
    "esp32s3_crosscore_ipi",
    "esp32s3_ds",
    "esp32s3_extmem",
    "esp32s3_gdma",
    "esp32s3_gpio",
    "esp32s3_hmac",
    "esp32s3_i2c",
    "esp32s3_i2s",
    "esp32s3_io_mux",
    "esp32s3_lcd_cam",
    "esp32s3_ledc",
    "esp32s3_mcpwm",
    "esp32s3_pcnt",
    "esp32s3_rmt",
    "esp32s3_rng",
    "esp32s3_rsa",
    "esp32s3_sar_adc",
    "esp32s3_sdmmc",
    "esp32s3_sens",
    "esp32s3_sha",
    "esp32s3_spi",
    "esp32s3_system",
    "esp32s3_system_stub",
    "esp32s3_systimer",
    "esp32s3_timer_group",
    "esp32s3_twai",
    "esp32s3_uart",
    "esp32s3_usb_otg",
    "esp32s3_usb_serial_jtag",
    "esp32s3_wifi_mac",
    // nRF52 behavioral models the factory builds but this table never named.
    // Alias spellings are deliberately NOT here -- `canonical_peripheral_type`
    // maps those to their canonical output, and listing an alias INPUT would
    // short-circuit that mapping.
    "nrf52840_aar",
    "nrf52840_acl",
    "nrf52840_bprot",
    "nrf52840_ccm",
    "nrf52840_comp",
    "nrf52840_cryptocell",
    "nrf52840_ecb",
    "nrf52840_egu",
    "nrf52840_ficr",
    "nrf52840_i2s",
    "nrf52840_lpcomp",
    "nrf52840_mwu",
    "nrf52840_nfct",
    "nrf52840_nvmc",
    "nrf52840_pdm",
    "nrf52840_ppi",
    "nrf52840_pwm",
    "nrf52840_qdec",
    "nrf52840_radio",
    "nrf52840_rng",
    "nrf52840_rtc",
    "nrf52840_temp",
    "nrf52840_uicr",
    "nrf52840_usbd",
    "nrf52840_usbregulator",
    "nrf52840_watchdog",
    "nrf52_aar",
    "nrf52_acl",
    "nrf52_bprot",
    "nrf52_ccm",
    "nrf52_clock",
    "nrf52_comp",
    "nrf52_cryptocell",
    "nrf52_ecb",
    "nrf52_egu",
    "nrf52_ficr",
    "nrf52_i2s",
    "nrf52_lpcomp",
    "nrf52_mwu",
    "nrf52_nfct",
    "nrf52_nvmc",
    "nrf52_pdm",
    "nrf52_ppi",
    "nrf52_pwm",
    "nrf52_qdec",
    "nrf52_radio",
    "nrf52_rng",
    "nrf52_rtc",
    "nrf52_serial_instance",
    "nrf52_temp",
    "nrf52_uicr",
    "nrf52_usbd",
    "nrf52_usbregulator",
    "nrf52_watchdog",
    "nrf52_wdt",
    // Further nRF54L factory arms.
    "nrf54l_clock",
    "nrf54l_grtc",
    "efr32s2_cmu",
    "efr32s2_gpio_head",
    "efr32s2_smu",
    "efr32s2_timerroute",
    "efr32s2_gpio_exti",
    "efr32s2_iadc",
    "efr32s2_timer",
    "virtual_ble",
];

/// True if `t` is already a canonical model-type name (see [`MODEL_TYPES`]).
pub fn is_canonical_model_type(t: &str) -> bool {
    MODEL_TYPES.contains(&t)
}

/// Build a generic peripheral model for `canonical_type`, or `None` if it is not
/// a generic type (so `from_config` falls through to the descriptor loaders).
pub fn try_build(
    canonical_type: &str,
    p_cfg: &PeripheralConfig,
    _manifest: &SystemManifest,
    _bus_trace: &crate::bus::bus_trace::BusTrace,
) -> anyhow::Result<Option<Box<dyn Peripheral>>> {
    let dev: Box<dyn Peripheral> = match canonical_type {
        "systick" | "arm_generictimer" => {
            // CALIB is implementation-defined per chip; the yaml can
            // supply the silicon value via `config: { calib: ... }`.
            match p_cfg.config.get("calib").and_then(|v| v.as_u64()) {
                Some(calib) => Box::new(crate::peripherals::systick::Systick::with_calib(
                    calib as u32,
                )),
                None => Box::new(crate::peripherals::systick::Systick::new()),
            }
        }
        // Silicon Labs Series-2 GPIO external interrupts — the `attachInterrupt`
        // block, in the GPIO head at `GPIO_S_BASE + 0x400`. A separate window
        // from the four port structs, which keep their own model.
        "efr32s2_gpio_exti" => {
            Box::new(crate::peripherals::efr32::gpio_exti::Efr32s2GpioExti::new())
        }
        // Silicon Labs Series-2 incremental ADC — the `analogRead` path.
        // Its own model, NOT an `AdcRegisterLayout` variant: `adc.rs` is one
        // struct per STM32 family by design and shares no register with this.
        "efr32s2_iadc" => Box::new(crate::peripherals::efr32::iadc::Efr32s2Iadc::new()),
        // Silicon Labs Series-2 TIMER. ⚠️ `counter_bits` is REQUIRED and per
        // instance: TIMER0/1/8/9 are 32-bit and TIMER2..7 are 16-bit on this
        // part (`TIMER_CNTWIDTH` in the device header). There is no safe
        // default — guessing 32 gives a `micros()` that never wraps on a
        // 16-bit instance, guessing 16 truncates a 32-bit one.
        "efr32s2_timer" => {
            let bits = p_cfg
                .config
                .get("counter_bits")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "peripheral '{}' (efr32s2_timer) must declare `config: {{ counter_bits: 16|32 }}`                          — the width is per instance on this family (TIMER_CNTWIDTH), not a family constant",
                        p_cfg.id
                    )
                })? as u32;
            let mut timer = crate::peripherals::efr32::timer::Efr32s2Timer::new(bits);
            // ⚠️ The timebase is the PERIPHERAL clock, not `cpu_hz`. On this
            // family they differ by 4.1x out of reset; see the model's header.
            if let Some(hz) = p_cfg.config.get("peripheral_hz").and_then(|v| v.as_u64()) {
                timer.set_peripheral_hz(hz);
            }
            Box::new(timer)
        }
        // LabWired virtual BLE controller. NOT a model of any silicon — see
        // `peripherals/virtual_ble.rs` for why a part whose vendor documents no
        // radio register anywhere gets a declared simulator device instead of
        // an invented register map that would read as silicon in an inspector.
        "virtual_ble" => Box::new(crate::peripherals::virtual_ble::VirtualBle::new_default()),
        // Silicon Labs Series-2 CMU — its own model, NOT an `RccRegisterLayout`
        // variant. `rcc.rs` is one struct per STM32 family by design, and this
        // silicon shares no register with any of them.
        "efr32s2_cmu" => Box::new(crate::peripherals::efr32::cmu::Efr32s2Cmu::new()),
        // The GPIO block HEAD — `GPIO_TypeDef`'s first twelve words, which sit
        // BELOW the four port structs at +0x30. Only one of them is a
        // register: `GPIO_IPVERSION` at +0x00, which reads 7.
        //
        // ⚠️ This window was not mapped at all, so `GPIO->IPVERSION` — the
        // first thing a Series-2 driver touches to identify the block — bus
        // faulted on the twin and returns 7 on silicon. It was invisible
        // because the conformance ratchet dropped faulting reads on the floor
        // (`Err(_) => {}`) instead of reporting them; twelve addresses were
        // being counted as misses with no line saying why.
        // The Security Management Unit — the first peripheral a vendor-built
        // image touches, three instructions into `SystemInit`.
        "efr32s2_smu" => Box::new(crate::peripherals::efr32::smu::Efr32s2Smu::new()),
        // The GPIO block's TIMER pin-mux. A real model, not a stub: it is the
        // difference between a PWM duty that is correct in a register and a
        // waveform that reaches a pad.
        "efr32s2_timerroute" => {
            Box::new(crate::peripherals::efr32::gpio_route::Efr32s2TimerRoute::new())
        }
        "efr32s2_gpio_head" => {
            let mut s = crate::peripherals::stub::StubPeripheral::new(0x00);
            s.values.insert(0x00, 0x0000_0007);
            Box::new(s)
        }
        "rcc" => {
            let layout: RccRegisterLayout = SystemBus::parse_profile_or_default(p_cfg, "RCC")?;
            let mut rcc = crate::peripherals::rcc::Rcc::new_with_layout(layout);
            // F4 ENR writable masks are per-part (implemented-peripheral
            // set). YAML: `config: { rcc_ahb1enr_mask, rcc_apb1enr_mask,
            // rcc_apb2enr_mask }`; default unmasked (0xFFFF_FFFF).
            let m = |k: &str| -> u32 {
                p_cfg
                    .config
                    .get(k)
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32)
                    .unwrap_or(0xFFFF_FFFF)
            };
            rcc.set_f4_enr_masks(
                m("rcc_ahb1enr_mask"),
                m("rcc_apb1enr_mask"),
                m("rcc_apb2enr_mask"),
            );
            Box::new(rcc)
        }
        "dbgmcu" => {
            // Pull IDCODE from YAML config (`idcode: "0x10076415"` or
            // `idcode: 269009941`). Default 0 — firmware probing
            // DBGMCU_IDCODE will then read 0; logged.
            let idcode: u32 = p_cfg
                .config
                .get("idcode")
                .and_then(|v| {
                    if let Some(s) = v.as_str() {
                        let s = s.trim();
                        if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                            u32::from_str_radix(rest, 16).ok()
                        } else {
                            s.parse::<u32>().ok()
                        }
                    } else {
                        v.as_u64().map(|n| n as u32)
                    }
                })
                .unwrap_or(0);
            if idcode == 0 {
                tracing::warn!(
                    "dbgmcu peripheral '{}' has no idcode configured \
                                 — firmware probing DBGMCU_IDCODE will read 0",
                    p_cfg.id
                );
            }
            Box::new(crate::peripherals::dbgmcu::Dbgmcu::new(idcode))
        }
        "timer" | "stm32_timer" | "efm32timer" | "renesasra_agt" | "stm32l0_lptimer" => {
            if p_cfg.r#type.contains("nrf") {
                // Nordic TIMER is task/event-driven and shares no
                // register layout with the STM32 TIMx family —
                // route to the dedicated nRF52 model.
                // TIMER3/4 have 6 CC; TIMER0/1/2 have 4 (default).
                let num_cc: usize = p_cfg
                    .config
                    .get("num_cc")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize)
                    .unwrap_or(4);
                Box::new(crate::peripherals::nrf52::timer::Nrf52Timer::new_with_cc(
                    num_cc,
                ))
            } else {
                // Width selector for 32-bit TIM2/TIM5 (STM32L4 etc).
                // YAML: `config: { width: 32 }`. Defaults to 16 for
                // back-compat with F1-class general-purpose timers.
                // `advanced: true` enables RCR/BDTR/CCR5/6 (TIM1/TIM8).
                let width: u8 = p_cfg
                    .config
                    .get("width")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u8)
                    .unwrap_or(16);
                let advanced = p_cfg
                    .config
                    .get("advanced")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                // `basic: true` (TIM6/TIM7) → counter + UIF only, no
                // capture/compare channels.
                let basic = p_cfg
                    .config
                    .get("basic")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                Box::new(
                    crate::peripherals::timer::Timer::new_with_layout(width, advanced).basic(basic),
                )
            }
        }
        "avr_gpio" => Box::new(crate::peripherals::avr_gpio::AvrGpioPort::new()),
        "spi" | "stm32spi" => {
            let layout: crate::peripherals::spi::SpiRegisterLayout = if p_cfg.r#type.contains("nrf")
            {
                crate::peripherals::spi::SpiRegisterLayout::Nrf52Spim
            } else {
                SystemBus::parse_profile_or_default(p_cfg, "SPI")?
            };
            // Classic-SPI CR2 mask is a per-part delta: F1 0xE7, F4 adds
            // FRF bit 4 → 0xF7. YAML: `config: { cr2_mask: 0xF7 }`.
            let cr2_mask: u32 = p_cfg
                .config
                .get("cr2_mask")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32)
                .unwrap_or(0x0000_00E7);
            let mut spi = crate::peripherals::spi::Spi::new_with_layout_cr2(layout, cr2_mask);
            // Classic-SPI CR1 writable mask, also a per-part delta: F407 silicon
            // does not latch CR1 bit 12 (CRCNEXT) → 0xEFFF; F1/L0/L4 leave it
            // fully writable (the default). YAML: `config: { cr1_mask: 0xEFFF }`.
            if let Some(cr1_mask) = p_cfg.config.get("cr1_mask").and_then(|v| v.as_u64()) {
                spi.set_cr1_mask(cr1_mask as u16);
            }
            // Which datasheet AF map routes this controller's pads. Needed
            // because the H5 "SPI v3" register file is shared by parts whose
            // pinouts DISAGREE (H563/H735 put SPI1_SCK on PB3 where the WBA52
            // puts SPI1_MISO), so the register layout cannot pick the table.
            // YAML: `config: { pad_map: stm32h5 }`. Absent ⇒ no SPI pad
            // routing, which is the fail-closed default — see `SpiPadMap`.
            if let Some(pad_map) = p_cfg.config.get("pad_map").and_then(|v| v.as_str()) {
                spi.set_pad_map(pad_map.parse::<crate::peripherals::spi::SpiPadMap>()?);
            }
            // Hand-written SPI devices attach via the PeripheralKit registry
            // pass, so no external-device attach loop is needed here.
            Box::new(spi)
        }
        "pwr" => {
            // `config: { profile: stm32h5 }` selects the H5 layout
            // (VOSCR/VOSSR voltage scaling); default stays L4.
            match p_cfg.config.get("profile").and_then(|v| v.as_str()) {
                Some("stm32h5") | Some("h5") => Box::new(crate::peripherals::pwr::PwrH5::new()),
                // H7 voltage scaling: VOSRDY/ACTVOSRDY handshake the HAL polls
                // before touching the PLL (RM0468 D3CR/SRDCR + CSR1).
                Some("stm32h7") | Some("h7") => Box::new(crate::peripherals::pwr::PwrH7::new()),
                // L0 has a two-register surface (CR/CSR), not the L4
                // CR1..CR4 / PUCRx set — a distinct reset shape.
                Some("stm32l0") | Some("l0") => Box::new(crate::peripherals::pwr::PwrL0::new()),
                // WBA: VOSR (0x0C) VOS→VOSRDY handshake the SoC init polls.
                Some("stm32wba") | Some("wba") => Box::new(crate::peripherals::pwr::PwrWba::new()),
                // F4 has only PWR_CR/PWR_CSR (RM0368 §5.4) — a distinct reset
                // shape from the L4 CR1..CR4 / PUCRx set.
                Some("stm32f4") | Some("f4") => Box::new(crate::peripherals::pwr::PwrF4::new()),
                _ => Box::new(crate::peripherals::pwr::Pwr::new()),
            }
        }
        "flash" | "flash_iface" => {
            // Layout selected via `config: { profile: stm32f1 | stm32l4 }`
            // in the chip yaml. Missing/unknown profile keeps the L4
            // default — backward compatible with existing chip configs.
            let layout: crate::peripherals::flash::FlashRegisterLayout =
                SystemBus::parse_profile_or_default(p_cfg, "FLASH")?;
            // Opt-in H5 program-error fidelity gate. `config: { error_flags: true }`
            // makes a misaligned / over-not-erased program raise the silicon
            // NSSR error flags instead of silently committing. Default false
            // (and a no-op on non-H5 layouts) — existing configs are unchanged.
            let error_flags = p_cfg
                .config
                .get("error_flags")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            // Opt-in H5 read-while-write fidelity gate. `config: { read_while_write:
            // true }` makes an erase of the bank the CPU is executing from fault
            // (the firmware must run the flash routine from SRAM) instead of
            // silently succeeding. Default false (no-op on non-H5 layouts) —
            // existing configs unchanged. Independent of `error_flags`.
            let read_while_write = p_cfg
                .config
                .get("read_while_write")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Box::new(
                crate::peripherals::flash::Flash::new_with_layout(layout)
                    .with_error_flags(error_flags)
                    .with_read_while_write(read_while_write),
            )
        }
        "rng" => Box::new(crate::peripherals::rng::Rng::new()),
        // Simulation-control device: firmware ends its own run with an exit
        // code. No configuration — the device has no non-deterministic knobs.
        "simctl" => Box::new(crate::peripherals::simctl::SimCtl::new()),
        "rp2040_clkrst" => {
            let profile = match p_cfg.config.get("profile").and_then(|v| v.as_str()) {
                Some("rp2350") => crate::peripherals::rp2040_clocks::ClockResetProfile::Rp2350,
                // Absent/anything else: the RP2040 map (the only map this
                // peripheral had before RP2350 onboarding).
                _ => crate::peripherals::rp2040_clocks::ClockResetProfile::Rp2040,
            };
            Box::new(
                crate::peripherals::rp2040_clocks::Rp2040ClockReset::with_profile(
                    p_cfg.base_address,
                    profile,
                ),
            )
        }
        "rp2040_timer" => Box::new(crate::peripherals::rp2040::timer::Rp2040Timer::new()),
        "rp2040_dma" => Box::new(crate::peripherals::rp2040::dma::Rp2040Dma::new()),
        "rp2040_io_bank0" => Box::new(crate::peripherals::rp2040::io_bank0::Rp2040IoBank0::new()),
        "rp2040_sio" => Box::new(crate::peripherals::rp2040::sio::Rp2040Sio::new()),
        "rp2040_spi" => Box::new(crate::peripherals::rp2040::spi::Rp2040Spi::new()),
        "rp2040_i2c" => Box::new(crate::peripherals::rp2040::i2c::Rp2040I2c::new()),
        "rp2040_pwm" => Box::new(crate::peripherals::rp2040::pwm::Rp2040Pwm::new()),
        "rp2040_adc" => Box::new(crate::peripherals::rp2040::adc::Rp2040Adc::new()),
        "rp2040_rtc" => Box::new(crate::peripherals::rp2040::rtc::Rp2040Rtc::new()),
        "rp2040_watchdog" => Box::new(crate::peripherals::rp2040::watchdog::Rp2040Watchdog::new()),
        "rp2040_xip_ssi" => Box::new(crate::peripherals::rp2040::xip_ssi::Rp2040XipSsi::new()),
        "rp2040_usb" => Box::new(crate::peripherals::rp2040::usb::Rp2040Usb::new()),
        "crc" => {
            // IDR scratch register width: 8-bit on F0/F1/L0, 32-bit
            // on F2+/L4+. YAML: `config: { idr_width: 8 }`; default 32.
            let idr_width: u8 = p_cfg
                .config
                .get("idr_width")
                .and_then(|v| v.as_u64())
                .map(|n| n as u8)
                .unwrap_or(32);
            Box::new(crate::peripherals::crc::Crc::new().with_idr_width(idr_width))
        }
        "rtc" => Box::new(crate::peripherals::rtc::Rtc::new()),
        "rtc_f1" => Box::new(crate::peripherals::rtc_f1::RtcF1::new()),
        "rtc_v3" => Box::new(crate::peripherals::rtc_v3::RtcV3::new()),
        "iwdg" => Box::new(crate::peripherals::iwdg::Iwdg::new()),
        "wwdg" => Box::new(crate::peripherals::wwdg::Wwdg::new()),
        "dac" => Box::new(crate::peripherals::dac::Dac::new()),
        "lptim" => Box::new(crate::peripherals::lptim::Lptim::new()),
        "quadspi" => Box::new(crate::peripherals::quadspi::Quadspi::new()),
        "sai" => Box::new(crate::peripherals::sai::Sai::new()),
        "usb_otg" => Box::new(crate::peripherals::usb_otg::UsbOtg::new()),
        "bxcan" => Box::new(crate::peripherals::bxcan::BxCan::new()),
        "fdcan" => Box::new(crate::peripherals::fdcan::Fdcan::new()),
        "sdmmc" => Box::new(crate::peripherals::sdmmc::Sdmmc::new()),
        "comp" => Box::new(crate::peripherals::comp::Comp::new()),
        "tsc" => Box::new(crate::peripherals::tsc::Tsc::new()),
        "fmc" => Box::new(crate::peripherals::fmc::Fmc::new()),
        "exti" => {
            let layout: crate::peripherals::exti::ExtiRegisterLayout =
                SystemBus::parse_profile_or_default(p_cfg, "EXTI")?;
            // Implemented-line count is part-specific (F103 = 19). YAML:
            // `config: { lines: 19 }`; default 20 for back-compat.
            let lines: u32 = p_cfg
                .config
                .get("lines")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32)
                .unwrap_or(20);
            let line_mask = if lines >= 32 {
                0xFFFF_FFFF
            } else {
                (1u32 << lines) - 1
            };
            Box::new(crate::peripherals::exti::Exti::new_with_layout_lines(
                layout, line_mask,
            ))
        }
        "afio" => Box::new(crate::peripherals::afio::Afio::new()),
        "dma" | "stm32dma" => Box::new(crate::peripherals::dma::Dma1::new()),
        // STM32F4 stream-based DMA (RM0090 §10). `config: { dma2: true }` marks
        // the DMA2 instance (memory-to-memory capable); `config: { stream_irqs:
        // [..8..] }` routes each stream to its own NVIC vector (F4 stream IRQs
        // are non-contiguous, e.g. DMA1_Stream7 = 47).
        "stm32f4_dma" => {
            let mut dma = crate::peripherals::stm32f4_dma::StreamDma::new();
            if p_cfg
                .config
                .get("dma2")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                dma = dma.as_dma2();
            }
            if let Some(arr) = p_cfg
                .config
                .get("stream_irqs")
                .and_then(|v| v.as_sequence())
            {
                let irqs: Vec<u32> = arr
                    .iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u32))
                    .collect();
                dma = dma.with_stream_irqs(irqs);
            }
            Box::new(dma)
        }
        "gpdma" => {
            // `config: { irq_base: N }` routes channel n to NVIC
            // line N + n (H563 GPDMA1: 27..34). Without it the
            // block's single `irq:` line serves every channel.
            let g = crate::peripherals::gpdma::Gpdma::new().with_base(p_cfg.base_address as u32);
            match p_cfg.config.get("irq_base").and_then(|v| v.as_u64()) {
                Some(base) => Box::new(g.with_irq_base(base as u32)),
                None => Box::new(g),
            }
        }
        "adc" => {
            let layout: crate::peripherals::adc::AdcRegisterLayout =
                SystemBus::parse_profile_or_default(p_cfg, "ADC")?;
            Box::new(crate::peripherals::adc::Adc::new_with_layout(layout))
        }
        "pio" => {
            let mut pio = crate::peripherals::pio::Pio::new();
            if let Some(program) = p_cfg.config.get("program").and_then(|v| v.as_str()) {
                pio.load_program_asm(program)?;
            }
            Box::new(pio)
        }
        "esp32_timg" => {
            // The only config-driven consumer of `esp32_timg` is the ESP32-C3
            // (ESP32-classic builds its TIMGs from the embedded descriptor
            // table via esp32/factory.rs, which stays on the canned-ratio
            // path). Give the C3 TIMGs the silicon-faithful RTC_SLOW cal
            // profile so IDF's `rtc_clk_cal` recovers exactly the same
            // RTC_SLOW rate the RTC_CNTL counter ticks at — one constant,
            // no second pin. (Only TIMG0 is ever calibrated by IDF; handing
            // TIMG1 the same profile is harmless and keeps the path uniform.)
            use crate::peripherals::esp32c3::rtc_timer::{C3_XTAL_HZ, RTC_SLOW_HZ_MEASURED};
            Box::new(
                crate::peripherals::esp32::timg::Timg::new(p_cfg.base_address as u32).with_rtc_cal(
                    crate::peripherals::esp32::timg::RtcCalProfile {
                        xtal_hz: C3_XTAL_HZ,
                        slow_hz: RTC_SLOW_HZ_MEASURED,
                    },
                ),
            )
        }
        // Instruction/data cache controllers (H5, WBA, U5…). Zephyr's SoC init
        // enables the cache via ICACHE_CR.EN and never polls a completion flag,
        // so a read-as-zero stub keeps the enable sequence from bus-faulting.
        // No cache behaviour is modelled — the simulator has flat memory.
        "icache" | "dcache" => Box::new(crate::peripherals::stub::StubPeripheral::new(0x00)),
        // SYSCFG — mostly EXTI source select (harmless read-0 stub), plus the H7
        // I/O compensation cell the H7 HAL enables + polls during rcc.freeze:
        // CCCSR @ 0x20, READY = bit 8. Seed it so the poll exits (EN is a
        // read-modify-write the stub drops, but the READY read returns the seed).
        "syscfg" => {
            let mut s = crate::peripherals::stub::StubPeripheral::new(0x00);
            s.values.insert(0x20, 0x0000_0100);
            Box::new(s)
        }
        // Hardware semaphore (WB/WL dual-core inter-core lock). Single-core sim
        // grants every lock to CPU1, so the read-lock path succeeds at once.
        "hsem" => Box::new(crate::peripherals::hsem::Hsem::new()),
        // NXP Kinetis clock peripherals — behavioural so the vendor MCUXpresso
        // clock bring-up (which spins on MCG_S / RSIM_CONTROL status bits)
        // settles instead of hanging. A passive register bank cannot complete
        // these hand-offs. See peripherals/mcg.rs and peripherals/rsim.rs.
        "nxp_mcg" | "kinetis_mcg" => Box::new(crate::peripherals::mcg::Mcg::new()),
        "nxp_rsim" => Box::new(crate::peripherals::rsim::Rsim::new()),
        _ => return Ok(None),
    };
    Ok(Some(dev))
}

#[cfg(test)]
mod registry_agreement {
    use super::MODEL_TYPES;

    /// Every type a family factory can build must be REACHABLE — either already
    /// canonical (in [`MODEL_TYPES`]) or mapped by the alias table in
    /// `bus::profiles::canonical_peripheral_type`. Anything else falls into the
    /// fuzzy `contains(...)` chain, which matches on substrings like "uart",
    /// "spi", "i2c", "adc" and picks a vendor's layout by guesswork.
    ///
    /// These were two hand-kept lists that had to agree, and nothing made them.
    /// ESP32-classic was absent from this table while every ESP32-C3 sibling was
    /// present, and it stayed invisible because only the Xtensa builder ever
    /// constructed those peripherals — that path registers the bank in Rust and
    /// never consults the table. The day a chip YAML declared one for a plain
    /// `from_config` bus, `esp32_uart` fell through to the fuzzy chain, which
    /// refused to guess and failed the build. Loud, and lucky: `esp32_spi` and
    /// `esp32_timg` would instead have been coerced onto STM32 layouts and
    /// modelled the wrong silicon in silence.
    ///
    /// When this first ran it found 103 unreachable types across four families —
    /// the whole ESP32-S3 set among them. So the expectation is derived from the
    /// factory sources, not from a third list that could rot the same way.
    ///
    /// Note what is deliberately NOT asserted: that a factory type is in
    /// `MODEL_TYPES` specifically. Alias INPUTS must stay out of it — listing
    /// one short-circuits the very mapping that makes it canonical.
    #[test]
    fn every_family_factory_type_is_reachable() {
        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let profiles = std::fs::read_to_string(src_root.join("bus/profiles.rs"))
            .expect("read bus/profiles.rs");
        let aliases = {
            let start = profiles.find("const ALIASES").expect("ALIASES table");
            let end = profiles[start..].find("];").expect("end of ALIASES") + start;
            &profiles[start..end]
        };
        let alias_names: Vec<&str> = aliases.split('"').skip(1).step_by(2).collect();

        let mut unreachable: Vec<String> = Vec::new();
        for family in ["esp32", "esp32c3", "esp32s3", "nrf52", "nrf54l"] {
            let src = std::fs::read_to_string(
                src_root.join("peripherals").join(family).join("factory.rs"),
            )
            .unwrap_or_else(|e| panic!("read {family}/factory.rs: {e}"));
            for line in src.lines() {
                let Some((head, _)) = line.split_once("=>") else {
                    continue;
                };
                for lit in head.split('"').skip(1).step_by(2) {
                    if !lit.starts_with(family) {
                        continue; // config keys and other string literals
                    }
                    if !MODEL_TYPES.contains(&lit) && !alias_names.contains(&lit) {
                        unreachable.push(format!("{family}/factory.rs: {lit}"));
                    }
                }
            }
        }
        unreachable.sort();
        unreachable.dedup();
        assert!(
            unreachable.is_empty(),
            "family factories build types that canonical_peripheral_type cannot \
             route, so they fall into the fuzzy substring fallback and can be \
             coerced onto an unrelated vendor's register map — silently wrong, \
             not an error: {unreachable:#?}\n\
             Add each to MODEL_TYPES (or to the alias table if its canonical \
             name differs)."
        );
    }
}
