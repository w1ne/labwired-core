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
    // RP2040 native peripherals (built here).
    "rp2040_timer",
    "rp2040_dma",
    "rp2040_spi",
    "rp2040_i2c",
    "rp2040_xip_ssi",
    "rp2040_usb",
    // ESP32-C3 behavioral models (esp32 factory).
    "esp32c3_i2c",
    "esp32c3_spi",
    "esp32c3_gpio",
    "esp32c3_io_mux",
    "esp32c3_apb_saradc",
    "esp32c3_ledc",
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
    manifest: &SystemManifest,
    bus_trace: &crate::bus::bus_trace::BusTrace,
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
            // Declarative IR SPI devices (`type: ir`) attach here,
            // mirroring the I2C path. Hand-written SPI devices attach via
            // the PeripheralKit registry pass, which ignores `type: ir`,
            // so the two dispatch paths never double-attach the same bus.
            for ext in &manifest.external_devices {
                if ext.connection != p_cfg.id || !ext.r#type.eq_ignore_ascii_case("ir") {
                    continue;
                }
                match crate::peripherals::components::build_spi_device(&ext.r#type, &ext.config) {
                    Some(device) => {
                        tracing::info!("spi attach: '{}' (type=ir) -> '{}'", ext.id, p_cfg.id);
                        // Wrap through the single trace helper (this factory
                        // attaches before the peripheral is on the bus).
                        spi.push_device(crate::bus::bus_trace::wrap_spi(
                            &p_cfg.id, bus_trace, device,
                        ));
                    }
                    None => {
                        tracing::warn!(
                            "spi attach skipped: invalid ir spec for external id '{}' on bus '{}'",
                            ext.id,
                            p_cfg.id
                        );
                    }
                }
            }
            Box::new(spi)
        }
        "pwr" => {
            // `config: { profile: stm32h5 }` selects the H5 layout
            // (VOSCR/VOSSR voltage scaling); default stays L4.
            match p_cfg.config.get("profile").and_then(|v| v.as_str()) {
                Some("stm32h5") | Some("h5") => Box::new(crate::peripherals::pwr::PwrH5::new()),
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
        "rp2040_clkrst" => Box::new(crate::peripherals::rp2040_clocks::Rp2040ClockReset::new(
            p_cfg.base_address,
        )),
        "rp2040_timer" => Box::new(crate::peripherals::rp2040::timer::Rp2040Timer::new()),
        "rp2040_dma" => Box::new(crate::peripherals::rp2040::dma::Rp2040Dma::new()),
        "rp2040_sio" => Box::new(crate::peripherals::rp2040::sio::Rp2040Sio::new()),
        "rp2040_spi" => Box::new(crate::peripherals::rp2040::spi::Rp2040Spi::new()),
        "rp2040_i2c" => Box::new(crate::peripherals::rp2040::i2c::Rp2040I2c::new()),
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
        "esp32_timg" => Box::new(crate::peripherals::esp32::timg::Timg::new(
            p_cfg.base_address as u32,
        )),
        // Instruction/data cache controllers (H5, WBA, U5…). Zephyr's SoC init
        // enables the cache via ICACHE_CR.EN and never polls a completion flag,
        // so a read-as-zero stub keeps the enable sequence from bus-faulting.
        // No cache behaviour is modelled — the simulator has flat memory.
        "icache" | "dcache" => Box::new(crate::peripherals::stub::StubPeripheral::new(0x00)),
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
