// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Deterministic peripheral type, profile, and layout resolution.

use super::*;

impl SystemBus {
    pub(crate) fn canonical_peripheral_type(raw_type: &str) -> String {
        let t = raw_type.to_ascii_lowercase();

        // 1. If the name is ALREADY a canonical model type, return it verbatim.
        //    This is the single source of truth (co-located with the factory in
        //    `generic_factory::MODEL_TYPES`) and replaces both the old core-type
        //    match and the per-name identity pre-emption blocks. It guarantees a
        //    real model type (e.g. `esp32c3_spi`, `nrf52840_twim`, `rp2040_timer`)
        //    is never coerced by the legacy fuzzy heuristics below.
        if crate::peripherals::generic_factory::is_canonical_model_type(&t) {
            return t;
        }

        // 2. Alias table: raw INPUT spellings whose canonical OUTPUT differs from
        //    the input. These are NOT identities (the verbatim case is handled by
        //    membership above), so they must not appear in `MODEL_TYPES`. Mostly
        //    nRF52 vendor synonyms (`nrf52840_i2c` → the TWIM master model, …)
        //    that must resolve before the fuzzy `contains(...)` chain, otherwise
        //    e.g. `nrf52840_saadc` (contains "adc") or `nrf52840_qspi`
        //    (contains "spi") would be coerced onto STM32 layouts. Iterated in
        //    order; first matching group wins.
        const ALIASES: &[(&[&str], &str)] = &[
            // SAADC: nRF52 SAR ADC (vendor "adc"/"saadc" spellings).
            (
                &["nrf52840_saadc", "nrf52_saadc", "nrf52840_adc"],
                "nrf52_saadc",
            ),
            // QSPI: nRF52 external-flash quad-SPI controller.
            (&["nrf52840_qspi", "nrf52_qspi"], "nrf52_qspi"),
            // SPIS / TWIS: SPI / I²C slave with EasyDMA.
            (&["nrf52840_spis", "nrf52_spis"], "nrf52840_spis"),
            (&["nrf52840_twis", "nrf52_twis"], "nrf52840_twis"),
            // TWIM / TWI master: nRF52 I²C master with EasyDMA.
            (
                &["nrf52840_i2c", "nrf52840_twim", "nrf52_twim", "nrf52_i2c"],
                "nrf52840_twim",
            ),
            // UARTE: nRF52 UART with EasyDMA (PSEL/BAUDRATE/CONFIG).
            (
                &["nrf52840_uart", "nrf52_uart", "nrf52_uarte"],
                "nrf52840_uart",
            ),
            // GPIOTE: Nordic GPIO task/event controller (shares "gpio" in name
            // but a totally different register surface).
            (
                &["nrf52840_gpiotasksevents", "nrf52_gpiote"],
                "nrf52_gpiote",
            ),
        ];
        for (inputs, canonical) in ALIASES {
            if inputs.contains(&t.as_str()) {
                return canonical.to_string();
            }
        }

        // Serial-instance mux (SPIM0/TWIM0 share one MMIO window) — must
        // precede the generic "contains(spi)" and "contains(i2c)" matchers.
        if t == "nrf52840_serial"
            || t == "nrf52_serial"
            || t == "nrf52_spim_twim"
            || t == "nrf52840_spim_twim"
        {
            return "nrf52_serial_instance".to_string();
        }

        // 3. Legacy generic SVD-name heuristics (fallback). Fuzzy `contains` /
        //    `starts_with` / `ends_with` matching for raw vendor names we have
        //    not given an explicit canonical type. Ordering matters: specific
        //    mappers come before broader ones so e.g.
        // "quadspi" doesn't get swallowed by the generic "contains(spi)" rule.
        if t.contains("quadspi") || t == "qspi" {
            return "quadspi".to_string();
        }
        if t.contains("lptim") || t == "low_power_timer" {
            return "lptim".to_string();
        }
        if t == "sai" || t.starts_with("sai_") || t.contains("audio") {
            return "sai".to_string();
        }
        if t.contains("otg") || t == "usb_fs" || t == "usb_otg_fs" {
            return "usb_otg".to_string();
        }
        if t == "bxcan" || t == "stm32_can" {
            return "bxcan".to_string();
        }
        if t == "sdmmc" || t == "sdio" || t.starts_with("sdmmc_") {
            return "sdmmc".to_string();
        }
        if t == "comp" || t == "comparator" || t.starts_with("comp_") {
            return "comp".to_string();
        }
        if t == "tsc" || t == "touchsense" {
            return "tsc".to_string();
        }
        if t == "fmc" || t == "fsmc" || t == "memorycontroller" {
            return "fmc".to_string();
        }

        if t.contains("uart") || t.contains("usart") || t == "leuart" || t.ends_with("_sci") {
            return "uart".to_string();
        }
        if t == "sam4s_pio" || (t.contains("gpio") && t != "pio") {
            return "gpio".to_string();
        }
        if t.contains("i2c") || t.contains("iic") || t.contains("smbus") || t.ends_with("_twi") {
            return "i2c".to_string();
        }
        if t.contains("spi") {
            return "spi".to_string();
        }
        if t == "udma" || t.contains("dma") {
            return "dma".to_string();
        }
        // Nordic CLOCK shares its name with the generic "rcc" bin in the
        // canonicalize, but its register layout is Nordic-specific and it
        // is unioned with the POWER peripheral at the same base address.
        // Route it to the dedicated nRF52 model.
        if t == "nrf_clock" || t == "nrf52_clock" || t == "nrf52840_clock" {
            return "nrf52_clock".to_string();
        }
        if t.contains("rcc") || t.contains("cmu") {
            return "rcc".to_string();
        }
        if t == "arm_generictimer" || t == "arm_globaltimer" || t == "arm_sp804_timer" {
            return "systick".to_string();
        }
        if t.contains("timer") || t.ends_with("_gpt") || t.ends_with("_agt") {
            return "timer".to_string();
        }
        if t.contains("adc") {
            return "adc".to_string();
        }

        t
    }

    pub(crate) fn profile_name(p_cfg: &PeripheralConfig) -> anyhow::Result<Option<&str>> {
        if let Some(value) = p_cfg.config.get("profile") {
            return value.as_str().map(Some).ok_or_else(|| {
                anyhow::anyhow!("Peripheral '{}' config.profile must be a string", p_cfg.id)
            });
        }
        if let Some(value) = p_cfg.config.get("register_layout") {
            return value.as_str().map(Some).ok_or_else(|| {
                anyhow::anyhow!(
                    "Peripheral '{}' config.register_layout must be a string",
                    p_cfg.id
                )
            });
        }
        Ok(None)
    }

    pub(crate) fn parse_profile_or_default<T>(
        p_cfg: &PeripheralConfig,
        peripheral_kind: &str,
    ) -> anyhow::Result<T>
    where
        T: FromStr<Err = String> + Default,
    {
        let Some(profile_name) = Self::profile_name(p_cfg)? else {
            return Ok(T::default());
        };
        T::from_str(profile_name).map_err(|e| {
            anyhow::anyhow!(
                "Peripheral '{}' has invalid {} profile '{}': {}",
                p_cfg.id,
                peripheral_kind,
                profile_name,
                e
            )
        })
    }

    /// Resolve the UART register layout for a peripheral **deterministically**
    /// from its declared type. The decision order is fixed and total, and there
    /// is no path that silently mismodels a strange UART:
    ///
    ///   1. An explicit `config.profile` always wins (the author's deliberate
    ///      choice), so any UART can be pinned to any modelled layout.
    ///   2. A type whose silicon register map we actually model routes to that
    ///      layout: `*lpuart*` → Kinetis LPUART; `stm32h5`/`stm32f7` → modern
    ///      STM32 USART; any other `stm32…` name and the bare generic `"uart"`
    ///      → the classic STM32 USART map (SR/DR/BRR/CR1…).
    ///   3. Anything else — every vendor UART we do not model yet (PL011, 16550,
    ///      Gaisler APBUART, EFM32/EFR32, Renesas SCI, LiteX, SiFive, SAM, …) —
    ///      ERRORS. It must name a layout via `config.profile` to run. A UART is
    ///      never silently mapped onto an STM32 register map by omission, the
    ///      way `nxp_lpuart` was before this gate existed.
    pub(crate) fn uart_layout_for(
        p_cfg: &PeripheralConfig,
    ) -> anyhow::Result<crate::peripherals::uart::UartRegisterLayout> {
        use crate::peripherals::uart::UartRegisterLayout::{self, Lpuart, Stm32F1, Stm32V2};

        // 1. Explicit author override wins, for any UART type.
        if let Some(name) = Self::profile_name(p_cfg)? {
            return UartRegisterLayout::from_str(name).map_err(|e| {
                anyhow::anyhow!(
                    "Peripheral '{}' has invalid UART profile '{}': {}",
                    p_cfg.id,
                    name,
                    e
                )
            });
        }

        // 2. Route the families we model faithfully, by declared type. Each
        //    family's register map lives in `UartRegisterLayout`; the offsets
        //    come from datasheets / vendor CMSIS headers / in-tree drivers.
        use UartRegisterLayout::*;
        let raw = p_cfg.r#type.to_ascii_lowercase();
        let has = |needle: &str| raw.contains(needle);
        let layout = if has("lpuart") {
            Lpuart
        } else if raw == "uart" {
            // The generic escape hatch: the classic STM32 USART map.
            Stm32F1
        } else if has("stm32") {
            if has("stm32h5") || has("stm32f7") {
                Stm32V2
            } else {
                Stm32F1
            }
        } else if has("pl011") {
            Pl011
        } else if has("16550") {
            Ns16550
        } else if has("da14") {
            // Dialog/Renesas DA1469x = Synopsys DW_apb_uart (16550, 4-byte stride).
            DwApbUart
        } else if has("cadence") {
            Cadence
        } else if has("efr32") {
            Efr32
        } else if has("efm32") {
            Efm32
        } else if raw == "leuart" {
            // Exact: "leuart" is a substring of unrelated names (e.g. "simpleuart").
            Leuart
        } else if has("sci") {
            // Renesas SCI (renesas_sci, renesasraXmY_sci).
            Sci
        } else if has("gaisler") || has("apbuart") {
            Gaisler
        } else if has("npcx") {
            Npcx
        } else if has("max32650") {
            Max32650
        } else if has("opentitan") {
            OpenTitan
        } else if has("sam_usart") || has("samusart") {
            Sam
        } else if has("samd5") || has("same5") || has("sercom") {
            Sercom
        } else if has("imx") {
            Imx
        } else if has("sifive") {
            Sifive
        } else if has("litex") {
            Litex
        } else if has("murax") {
            Murax
        } else if has("coreuart") || has("miv") {
            CoreUart
        } else if has("k6xf") {
            KinetisUart
        } else if has("pulp") || has("udma") {
            Pulp
        } else if has("ft9001") || has("ft900") {
            // Bridgetek FT9xx UART is 16550-compatible.
            Ns16550
        } else if has("cosimulated") {
            // Co-simulation stub with no fixed register map — default to 16550.
            Ns16550
        } else if has("mpc5567") || has("esci") {
            Esci
        } else if has("picosoc") || has("simpleuart") {
            PicoUart
        } else {
            // 3. Unmodelled UART — refuse to guess.
            anyhow::bail!(
                "UART type '{}' (peripheral '{}') has no register layout modelled yet \
                 and no `config.profile` set; it will NOT be silently mapped onto an \
                 STM32. Choose a layout explicitly with \
                 `config: {{ profile: <one of the supported layouts> }}`, or add a \
                 dedicated model for it.",
                p_cfg.r#type,
                p_cfg.id
            );
        };
        Ok(layout)
    }

    /// Resolve the GPIO register layout for a peripheral **deterministically**
    /// from its declared type, mirroring [`Self::uart_layout_for`]. There is NO
    /// path that silently mismodels a GPIO port onto STM32F1 by omission — a
    /// wrong GPIO layout moves the output-data-register offset, and anything
    /// that latches a pin level from that register (e.g. a SPI display's D/C
    /// line via [`Self::resolve_pin_odr`]) then samples the wrong address and
    /// silently misbehaves. The FRDM-KW41Z "cow" LCD blanked exactly this way:
    /// a `type: gpio` port with no `profile` fell back to Stm32F1 (ODR @0x0C),
    /// so the D/C line resolved to an address the Kinetis firmware (PDOR @0x00)
    /// never drives, D/C stayed low, and every pixel byte decoded as a command.
    ///
    ///   1. An explicit `config.profile` always wins (author's deliberate choice).
    ///   2. A type whose silicon layout the *name* pins down routes to it:
    ///      `*nrf*` → Nordic; `stm32f4`/`*h5*`/`*v2*` → modern STM32;
    ///      `stm32_gpioport`/`stm32f1`/`stm32f2` and the legacy placeholder
    ///      ports (`efmgpioport`/`npcx_gpio`/`imxrt_gpio`, historically run on
    ///      the F1 map) → classic STM32F1.
    ///   3. The bare vendor-neutral `"gpio"` type (or any other gpio-ish type we
    ///      do not model) with NO `profile` ERRORS. It is never silently mapped
    ///      onto STM32F1 by omission.
    pub(crate) fn gpio_layout_for(
        p_cfg: &PeripheralConfig,
    ) -> anyhow::Result<crate::peripherals::gpio::GpioRegisterLayout> {
        use crate::peripherals::gpio::GpioRegisterLayout;

        // 1. Explicit author override wins, for any GPIO type.
        if let Some(name) = Self::profile_name(p_cfg)? {
            return GpioRegisterLayout::from_str(name).map_err(|e| {
                anyhow::anyhow!(
                    "Peripheral '{}' has invalid GPIO profile '{}': {}",
                    p_cfg.id,
                    name,
                    e
                )
            });
        }

        // 2. Route the families whose declared type pins down the layout.
        let raw = p_cfg.r#type.to_ascii_lowercase();
        let has = |needle: &str| raw.contains(needle);
        let layout = if has("nrf") {
            GpioRegisterLayout::Nrf52
        } else if has("stm32f4") || has("h5") || has("stm32v2") {
            GpioRegisterLayout::Stm32V2
        } else if raw == "stm32_gpioport" || has("stm32f1") || has("stm32f2") {
            GpioRegisterLayout::Stm32F1
        } else if raw == "efmgpioport" || raw == "npcx_gpio" || raw == "imxrt_gpio" {
            // Not yet modelled with a dedicated register map; historically ran
            // on the STM32F1 layout. Kept explicit (by type) so the choice is
            // visible rather than an omission-driven silent default.
            GpioRegisterLayout::Stm32F1
        } else if raw == "gpio" {
            // 3a. The bare vendor-neutral "gpio" type with no profile is the
            //     dangerous case (real product chips: KW41Z / STM32 / nRF /
            //     ESP32) — the author meant a *specific* silicon layout and
            //     omitting it silently picked STM32F1, corrupting D/C
            //     resolution (the FRDM-KW41Z "cow" blank). REFUSE to guess.
            anyhow::bail!(
                "GPIO peripheral '{}' is declared with the vendor-neutral `type: gpio` but no \
                 `config.profile`; it will NOT be silently mapped onto STM32F1 (a wrong layout \
                 moves the output register and blanks a display's D/C line). Choose a layout \
                 explicitly with `config: {{ profile: <stm32f1|stm32v2|nrf52|kinetis> }}`.",
                p_cfg.id
            );
        } else {
            // 3b. A vendor-named gpio type we do not model yet (e.g.
            //     `gaisler_gpio`, `cc2538_gpio`, `mpfs_gpio`, `gpio_esp32`, …),
            //     used by the strict-onboarding chip ramp. These historically
            //     ran on the STM32F1 placeholder layout. Keep them loadable so
            //     onboarding isn't blocked, but WARN loudly instead of failing
            //     silently — the placeholder is a known-incomplete model, not a
            //     faithful one. Pin it with `config.profile` (or add a real
            //     model) to silence this and get correct register behaviour.
            tracing::warn!(
                "GPIO type '{}' (peripheral '{}') has no dedicated register model; falling back \
                 to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — \
                 set `config.profile` or add a real model for correct behaviour.",
                p_cfg.r#type,
                p_cfg.id
            );
            GpioRegisterLayout::Stm32F1
        };
        Ok(layout)
    }
}
