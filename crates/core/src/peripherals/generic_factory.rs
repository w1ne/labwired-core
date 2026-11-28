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
    "avr_adc",
    "pic24_gpio",
    "rcc",
    "systick",
    "arm_sp804_timer",
    "lpc2294_timer",
    "arm_pl190_vic",
    "sifive_gpio",
    "timer",
    "i2c",
    "spi",
    "exti",
    "afio",
    "dma",
    "dmamux",
    "stm32f4_dma",
    "gpdma",
    "adc",
    "pio",
    "declarative",
    "strict_ir",
    "strict_ir_internal",
    "pwr",
    "flash",
    // STM32U5 system memory (OTP / package / UID / factory flash size).
    "stm32u5_sysmem",
    "stm32u5_hash",
    "stm32u5_saes",
    "stm32u5_pka",
    "rng",
    "simctl",
    "crc",
    "crs",
    "rtc",
    "rtc_f1",
    "bkp_f1",
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
    "analog_comparator",
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
    "rp2350_bootram",
    "cortex_m_coredebug",
    // ESP32-C2 DevKitM profile types. GPIO and UART behavior are shared with
    // the register-compatible C3 models, with C2 clock/source values supplied
    // by its family adapter.
    "esp32c2_gpio",
    "esp32c2_uart",
    // ESP32-C3 behavioral models (esp32 factory).
    // MUST be listed: without membership the fuzzy fallback would coerce the
    // BLE baseband window onto some unrelated register map instead of erroring.
    "esp32c3_bt",
    "esp32c3_i2c",
    "esp32c3_spi",
    // MUST be listed: the fuzzy `contains("dma")` fallback would coerce the
    // C3 GDMA onto the STM32 DMA register map.
    "esp32c3_gdma",
    "esp32c3_gpio",
    "esp32c3_io_mux",
    "esp32c3_apb_saradc",
    "esp32c3_ledc",
    "esp32c3_rmt",
    // MUST be listed: the fuzzy `contains("timer")` fallback would coerce the
    // SYSTIMER onto an unrelated timer register map.
    "esp32c3_systimer",
    // MUST be listed: the fuzzy fallback matches `contains("uart")`, which
    // would coerce the C3's UART onto the STM32 register map — the silently
    // wrong model that wedged every `Serial.print` over 128 bytes.
    "esp32c3_uart",
    // ESP32-C6 (RISC-V HP core) behavioral aliases. The C6 UART head map and
    // GPIO matrix register head match the C3's, but its interrupt-matrix source
    // ids (UART0 = 43) and core clock are its own, and the type names must be
    // canonical or the fuzzy `contains("uart")`/`contains("gpio")` fallback
    // would resolve them to the STM32 maps.
    "esp32c6_uart",
    "esp32c6_gpio",
    "esp32s2_rmt",
    "esp32s2_i2c",
    "esp32s2_spi",
    // C6-only behavioral blocks: PCR (clock/reset, full SVD register map) and
    // GDMA (3 channels, C6 register layout). The fuzzy `contains("dma")`
    // fallback would otherwise coerce `esp32c6_gdma` onto the STM32 DMA
    // register map — a silently wrong model.
    "esp32c6_pcr",
    "esp32c6_gdma",
    // C6 TIMG-with-MWDT (shared `esp32_timg` model + the C3/C6 watchdog path)
    // and LP_TIMER (C6 RTC main timer). `esp32c6_lp_rtc` contains "rtc" and
    // `esp32c6_mwdt` contains "wdt"; without membership the fuzzy chain would
    // hand `esp32c6_lp_rtc`'s `timer`-looking tail or the bare names to other
    // bins. Listed for the same reason as every other family type.
    "esp32c6_mwdt",
    "esp32c6_lp_rtc",
    // ESP32-H2's narrow DevKitM-1 UART profile. It reuses the C6-compatible
    // UART head and PCR UART0 gate; those register windows are size-limited by
    // the H2 descriptor because the full H2 PCR map differs from the C6's.
    "esp32h2_uart",
    "esp32h2_pcr",
    // C6 USB Serial/JTAG — same shared model as C3/S3, own matrix source.
    // Listed so a fuzzy `contains("usb")` path cannot coerce it elsewhere.
    "esp32c6_usb_serial_jtag",
    // C6 MODEM_LPCON + I2C_ANA_MST — sticky register banks for fast-boot
    // regi2c. `esp32c6_i2c_ana_mst` MUST be listed: fuzzy `contains("i2c")`
    // would otherwise coerce it onto the STM32 I2C map.
    "esp32c6_modem_lpcon",
    "esp32c6_i2c_ana_mst",
    // C6 EXTMEM / MSPI cache controller (DONE/freeze handshakes).
    "esp32c6_extmem",
    "esp32c6_systimer",
    // C6 RMT — the shared C3 model at C6 matrix source 49.
    "esp32c6_rmt",
    "esp32c6_ledc",
    // C5 UART0 uses the shared Espressif UART head at its own 240 MHz pacing
    // rate and interrupt source 47. It is intentionally window-limited by the
    // C5 chip descriptor; do not route it through the fuzzy STM32 UART arm.
    "esp32c5_gpio",
    // C5 I2C0 is selected by the C5-aware C3-family attach path.
    "esp32c5_i2c",
    "esp32c5_uart",
    // C61 UART0 uses the shared Espressif FIFO/status head at its 160 MHz
    // core rate and interrupt source 40; the descriptor limits the window.
    // C61's GPIO path uses shifted registers plus the I2C0 matrix subset.
    "esp32c61_gpio",
    "esp32c61_uart",
    "esp32c61_pcr",
    "esp32c61_i2c",
    // P4 HP I2C0 and UART0 are family-wrapped shared Espressif models; the P4
    // descriptor selects their bases and wires the tested GPIO matrix route.
    "esp32p4_i2c",
    "esp32p4_uart",
    // Narrow GPIO data/enable/input heads (S2/P4 classic head, C5/C61 later
    // head). Listed so a fuzzy "gpio" heuristic cannot coerce them onto an
    // STM32 GPIO map.
    "esp32s2_gpio",
    "esp32p4_gpio",
    "esp32c5_gpio",
    "esp32c61_gpio",
    // nRF52 behavioral models (nrf52 factory).
    "nrf52840_twim",
    "nrf52_saadc",
    "nrf52_qspi",
    "nrf52840_spis",
    "nrf52840_twis",
    "nrf52840_uart",
    "nrf5340_dppic",
    "nrf52_gpiote",
    // nRF54L behavioral models (nrf54l factory). Listed here so the fuzzy
    // `contains("uart")` heuristic cannot coerce `nrf54l_uarte` onto the
    // generic STM32 UART layout — it is a distinct silicon register map.
    "nrf54l_uarte",
    "nrf54l_twim",
    // Microchip SERCOM in USART mode. ⚠️ Load-bearing for the same reason
    // `nrf54l_uarte` is: the fuzzy `contains("uart")` heuristic would coerce
    // `sam_sercom_usart` onto the generic STM32 UART layout, whose DR/SR
    // offsets mean nothing to a SERCOM. That is the silent shape — a console
    // that enables cleanly and never emits a byte.
    "sam_sercom_usart",
    "sam_sercom_spi",
    "samd51_nvmctrl_manual_storage",
    "samd51_cmcc_control",
    // Opt-in SAMD51 board models (`build_samd51_board_model`): fixed
    // instance addresses, wired by the SAMD51 chip descriptors.
    "samd51_oscctrl",
    "samd51_gclk",
    "samd51_tc",
    "samd51_tcc",
    "samd51_eic",
    "samd51_dac",
    "samd51_adc",
    // The SAMD21 ADC: a different register layout, same conversion slice.
    "samd21_adc",
    // The same TCC slice on the SAMD21 instances (single IRQ line, PM clock gate).
    "samd21_tcc",
    "samd51_pac",
    "samd51_sercom_i2cm",
    // The same I2C master slice on SAMD21 SERCOM instances.
    "samd21_sercom_i2cm",
    "samd51_sercom_usart",
    "sam_dmac_byte",
    "sam_dmac_sercom4_tx_byte",
    "samd51_dmac_board",
    "samd51_usb_device_banks",
    // SAM D21 USB device controller with a scripted host (`build_samd21_usb`).
    "samd21_usb",
    // SAM D21 TC3..TC7 (`build_samd21_tc`): fixed instance addresses.
    "samd21_tc",
    // SAM D21 RTC (`build_samd21_rtc`): fixed IRQ, optional `gclk_hz`.
    "samd21_rtc",
    // SAM4S TC0..TC2 / TC3..TC5 blocks (`build_sam4s_tc`): fixed addresses.
    "sam4s_tc",
    // EFM32 Series-0 TIMER0..3 (`build_efm32_timer`): IRQ from the yaml.
    "efm32s0_timer",
    // EFM32 Series-1 TIMER0/1 (same builder, Series-1 register layout).
    "efm32s1_timer",
    // EFM32/EFR32 Series-1 WTIMER0/1 (same builder, 32-bit counter).
    "efm32s1_wtimer",
    // EFR32 Series-1 RTCC (`build_efm32_rtcc`): IRQ from the yaml, optional
    // `lfeclk_hz`.
    "efm32s1_rtcc",
    // Nuvoton NPCX ITIM32 / ITIM64 (`build_npcx_itim`): IRQ from the yaml,
    // optional `apb2_hz` / `lfclk_hz`.
    "npcx_itim32",
    "npcx_itim64",
    // NPCX MDMA context 0: routed UART RX into memory; context 1 and shared
    // UART/MDMA IRQ combining are outside this narrowed profile.
    "npcx_mdma",
    // Nuvoton NPCX MTC (`build_npcx_mtc`): IRQ from the yaml, optional
    // `tick_hz`.
    "npcx_mtc",
    // Nuvoton NPCX TWD (`build_npcx_twd`): IRQ from the yaml, optional
    // `lfclk_hz`.
    "npcx_twd",
    // Nuvoton NPCX HFCG / LFCG clock-generator register blocks
    // (`build_npcx_cdcg`): no IRQ, no options.
    "npcx_hfcg",
    "npcx_lfcg",
    // Nuvoton NPCX SPIP (SPI master, Renode `NPCX_SPIP`); devices attach
    // through `external_devices:`.
    "npcx_spip",
    // Nuvoton NPCX FIU user-mode command path and internal flash medium.
    "npcx_fiu",
    // Nuvoton NPCX SMBus master controller; devices attach through
    // `external_devices:` and the model's `I2cSlaveHost` hook.
    "npcx_smbus",
    // Nuvoton NPCX GPIO port (byte registers PDOUT..PLOCK_CTL). MUST be
    // listed: the fuzzy `contains("gpio")` fallback would map it onto the
    // STM32 GPIO register layout.
    "npcx_gpio",
    // Reserve the full name: never fuzzy-coerce an unqualified SAM DMAC to STM32.
    "sam_dmac",
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
    "esp32_pcnt",
    "esp32_rmt",
    "esp32_rtc_cntl",
    "esp32_rtc_io",
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
    "esp32s3_rtc_io",
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
    // ESP32-S2 layouts of the shared S3 models (Arduino profile).
    "esp32s2_apb_saradc",
    "esp32s2_crosscore_ipi",
    "esp32s2_gpio",
    "esp32s2_intmatrix",
    "esp32s2_ledc",
    "esp32s2_rng",
    "esp32s2_sens",
    "esp32s2_systimer",
    "esp32s2_uart",
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
    "nrf54l_rramc",
    "nrf54l_wdt",
    "efr32s2_busalloc",
    "efr32s2_cmu",
    "efr32s2_gpio_head",
    "efr32s2_smu",
    "efr32s2_timerroute",
    "efr32s2_gpio_exti",
    "efr32s2_iadc",
    "efr32s2_timer",
    "virtual_ble",
    // Microchip SAM clock controllers (SAMD21 PM/GCLK, SAMD51 MCLK).
    "sam_pm",
    "sam_gclk",
    "sam_mclk",
    // Renesas RA SYSTEM (HOCO / OSCSF).
    "ra_sysc",
    // Renesas RA8 / RA6 / RA4 SYSTEM, full R_SYSTEM register file (adds the
    // xxxCKCR handshake).
    "ra8_sysc",
    "ra6_sysc",
    "ra4_sysc",
    "ra_mstp",
    // Renesas RX SYSTEM (oscillator stable flags, clock-switch handshakes).
    "rx_sysc",
    // Renesas RX FLASH control registers (0xAA-keyed word writes).
    "rx_flash",
    // Renesas RX RTC registers as read-back storage.
    "rx_rtc",
    // Renesas RX compare match timer unit (CMSTR + two channels).
    "rx_cmt",
    // Microchip PIC32MZ UART (UxMODE/UxSTA/UxTXREG/UxRXREG/UxBRG with the
    // CLR/SET/INV aliases; EVIC flags through explicit IRQs).
    "pic32_uart",
    // Microchip PIC32MZ 16-bit timer (TxCON/TMRx/PRx, Type A or B).
    "pic32_timer",
    // Microchip megaAVR 0-series USART, the TCA0 + TCB0..3 timer block, ADC0,
    // the TWI host and the SPI host.
    "megaavr_usart",
    "megaavr_tc",
    "megaavr_adc",
    "megaavr_twi",
    "megaavr_spi",
    // Texas Instruments MSP430x1xx/x2xx: special function registers, USART
    // and USCI_A (UART mode), watchdog control, 16x16 hardware multiplier,
    // basic clock module (+).
    "msp430_sfr",
    "msp430_usart",
    "msp430_usci_a",
    "msp430_wdt",
    "msp430_mpy",
    "msp430_bcm",
    "msp430_bcm_plus",
    "msp430_bcsctl3",
    // Texas Instruments MSP430x2xx digital I/O port, and byte registers that
    // are storage with reset values (PxSEL2, calibration constants).
    "msp430_port",
    "msp430_bytes",
    // Texas Instruments MSP430x2xx Universal Serial Interface (SPI and I2C
    // master).
    "msp430_usi",
    // Texas Instruments MSP430x2xx Timer_A and its TAIV register.
    "msp430_timer_a",
    "msp430_timer_a_iv",
    // Texas Instruments MSP430x2xx ADC10 (no data transfer controller).
    "msp430_adc10",
    // Texas Instruments MSP430x5xx: SFRIE1/SFRIFG1/SFRRPCR, USCI_A (UART
    // mode, own UCAxIE/UCAxIFG/UCAxIV), WDT_A control, 32-bit hardware
    // multiplier, unified clock system, power management module.
    "msp430_sfr_x5",
    "msp430_usci_a_x5",
    "msp430_wdt_a",
    "msp430_mpy32",
    "msp430_ucs",
    "msp430_pmm",
    // Renesas RA pin function select (PmnPFS / PWPR).
    "ra_pfs",
    // Renesas RA8 pin function select (ports 0-B, PWPR/PWPRS moved).
    "ra8_pfs",
    // Renesas RA ICU event-link registers (IELSRn), the 16-bit AGT, the
    // 12-bit ADC and DAC, the IIC (RIIC) master, the CAN module (normal
    // mailbox mode) and the GPT (saw-wave PWM).
    "ra_icu",
    "ra_agt",
    "ra_adc12",
    "ra_dac12",
    "ra_iic",
    "ra_can",
    "ra_gpt",
    // NXP i.MX RT CCM / IOMUXC.
    "imx_ccm",
    "imx_iomuxc",
    // NXP i.MX RT10xx first-class models (peripherals/imxrt).
    "imxrt_ccm",
    "imxrt_anadig",
    "imxrt_dcdc",
    "imxrt_adc",
    "imxrt_lpuart",
    "imxrt_lpi2c",
    "imxrt_flexspi",
    "imxrt_usb",
    "imxrt_usbphy",
    "imxrt_edma",
    "imxrt_gpt",
    "imxrt_flexio",
    "imxrt_sai",
    // LiteX countdown timer (first modelled LiteX peripheral besides the UART
    // profile). Its type carries the `litex` prefix, so it must be a canonical
    // type or the fuzzy `contains("timer")` fallback would coerce it onto the
    // STM32 timer map.
    "litex_timer",
    // LiteX Control-and-Status (LED/switch/button CSR block).
    "litex_controlandstatus",
    // LiteX SPI-flash bitbang controller with its MT25Q responder.
    "litex_spi_flash",
    // Microchip Mi-V reference-design peripherals. MUST be listed: without
    // membership the fuzzy fallbacks would coerce `miv_coregpio` onto the
    // STM32 GPIO map (it was doing exactly that for the staged descriptors)
    // and `miv_coretimer` onto the STM32 timer map — silently wrong models.
    "miv_coregpio",
    "miv_coretimer",
    // PolarFire SoC MSS GPIO (CoreGPIO semantics, MSS register map).
    "mpfs_gpio",
    // PolarFire SoC MSS I2C0/1 (CoreI2C-family master).
    "mpfs_i2c",
    // PolarFire SoC MSS SPI0/1 (CoreSPI-family master).
    "mpfs_spi",
    // WCH-LinkE debug printf channel (ch32fun's default console).
    "wch_debug_printf",
    // WCH QingKe system timer (STK at 0xE000F000): V2 on the CH32V00x,
    // `profile: v4` on the CH32V20x/V30x.
    "qingke_stk",
    // SAM4S (the older PIO/PMC blocks; not the SAM D21 PORT/PM pair).
    "sam4s_pio",
    "sam4s_pmc",
    // SAM4S system controller: watchdog, reset controller, flash controller.
    "sam4s_wdt",
    "sam4s_rstc",
    "sam4s_eefc",
    // Cortex-M7 L1 cache maintenance window + the SCB cache-ID geometry.
    "cortex_m7_cache",
    // Cortex-M FP extension: FPCCR/FPCAR/FPDSCR/MVFR + FP context stacking.
    "cortex_m_fpu",
    // Armv8-M PMSAv8 MPU region file (RNR/RBAR/RLAR + aliases), shared with TT.
    "armv8m_mpu",
    // TI CC2538: the PL061-style GPIO port (address-masked GPIO_DATA),
    // SYS_CTRL, the 802.15.4 RF core, the sleep timer, the flash controller, the AES/SHA-256
    // cryptoprocessor, the µDMA, the SSI, the GPTimers and the I2C master.
    "cc2538_pl061_gpio",
    "cc2538_sys_ctrl",
    "cc2538_rfcore",
    "cc2538_sleep_timer",
    "cc2538_flash_ctrl",
    "cc2538_aes",
    "cc2538_udma",
    "cc2538_pl022_ssi",
    "cc2538_gptimer",
    "cc2538_i2cm",
    "cc2538_ioc",
    "cc2538_soc_adc",
    "cc2538_cctest",
    "cc2538_ana_regs",
    // Ambiq Apollo4 Plus: the 128-pin GPIO block (PADKEY-locked PINCFG,
    // RD/WT/WTS/WTC/EN banks) and PWRCTRL (DEVPWREN -> DEVPWRSTATUS).
    // Deliberately not `ambiqapollo4_gpio`: that spelling belongs to the
    // staged onboarding/ambiq-apollo4 descriptor, which keeps its placeholder.
    "apollo4_gpio",
    "apollo4_pwrctrl",
    "apollo4_cpu",
    // NXP S32K3 SIUL2: MSCR pad mux + GPDO/GPDI/PGPDO/PGPDI/MPGPDO GPIO.
    // Deliberately not `s32k3xx_systemintegrationunitlite2`: that spelling
    // belongs to the staged onboarding/nxp-s32k388 descriptors, which keep
    // their placeholder.
    "s32k3_siul2",
    // NXP S32K3 SWT (key-locked watchdog with reset) and MC_ME (partition /
    // COFB clock-enable handshake).
    "s32k3_swt",
    "s32k3_mc_me",
    // NXP i.MX RT500 (MIMXRT595S): the LPC-style FLEXCOMM USART and GPIO, and
    // the CLKCTL/RSTCTL (SET/CLR aliased) and IOPCTL (plain) register banks.
    // Not `lpc_usart`: the Renode-import profile heuristic maps that spelling
    // to a 16550 layout, which the FLEXCOMM USART is not.
    "lpc_flexcomm_usart",
    "lpc_gpio",
    "rt5xx_ctl",
    "rt5xx_clkctl0",
    "rt5xx_clkctl1",
    "rt5xx_iopctl",
    "rt5xx_ostimer",
    // NXP Kinetis RNGA (K64F `RNG`), not the STM32 `rng` layout.
    "kinetis_rnga",
    // FocalTech FT9001 CPM / reset controller / TRNG (peripherals/ft9001_sys.rs).
    "ft9001_cpm",
    "ft9001_reset",
    "ft9001_trng",
    // QuickLogic EOS S3 INTR_CTRL (peripherals/eos_s3_intr.rs).
    "eos_s3_intr_ctrl",
    // QuickLogic EOS S3 simple periodic timer (peripherals/eos_s3_spt.rs).
    "eos_s3_spt",
];

/// True if `t` is already a canonical model-type name (see [`MODEL_TYPES`]).
pub fn is_canonical_model_type(t: &str) -> bool {
    MODEL_TYPES.contains(&t)
}

/// The chip yaml's DMA `config: { requests: { <request>: <route>, ... } }`
/// table, sorted by request name so the routing order is deterministic.
fn dma_request_table(p_cfg: &PeripheralConfig) -> Vec<(String, serde_yaml::Value)> {
    let mut out: Vec<(String, serde_yaml::Value)> = p_cfg
        .config
        .get("requests")
        .and_then(|v| v.as_mapping())
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.clone())))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

// Opt-in SAMD51 USB device banks: the fixed instance, IRQ and clock gates.
#[cold]
#[inline(never)]
fn build_samd51_usb_device_banks(p_cfg: &PeripheralConfig) -> anyhow::Result<Box<dyn Peripheral>> {
    if p_cfg.base_address != 0x41000000 || p_cfg.irq != Some(80) || !p_cfg.config.is_empty() {
        anyhow::bail!("SAMD51 USB banks require base 0x41000000, IRQ80 and no config overrides");
    }
    if !p_cfg.clock.as_ref().is_some_and(|clocks| {
        let gates = clocks.as_slice();
        gates.len() == 2
            && [("AHBMASK", 10), ("APBBMASK", 0)]
                .into_iter()
                .all(|(reg, bit)| {
                    gates
                        .iter()
                        .any(|gate| gate.controller == "mclk" && gate.reg == reg && gate.bit == bit)
                })
    }) {
        anyhow::bail!(
            "SAMD51 USB banks require exactly the mclk AHBMASK bit10 and APBBMASK bit0 gates"
        );
    }
    Ok(Box::new(
        crate::peripherals::sam::usb_device::Samd51UsbDeviceBanks::new(),
    ))
}

// SAM D21 USB device controller: the one instance, its NVIC line, and the
// scripted host's options.
#[cold]
#[inline(never)]
fn build_samd21_usb(p_cfg: &PeripheralConfig) -> anyhow::Result<Box<dyn Peripheral>> {
    if p_cfg.base_address != 0x41005000 || p_cfg.irq != Some(7) {
        anyhow::bail!("SAMD21 USB requires base 0x41005000 and IRQ 7");
    }
    if let Some(key) = p_cfg
        .config
        .keys()
        .find(|k| !matches!(k.as_str(), "host" | "host_steps"))
    {
        anyhow::bail!("samd21_usb admits `host` and `host_steps`, not `{key}`");
    }
    let mut usb = crate::peripherals::sam::usb21::Samd21Usb::new();
    // `host: false` leaves the port unplugged (no enumeration).
    if p_cfg.config.get("host").and_then(|v| v.as_bool()) == Some(false) {
        usb.set_host_enabled(false);
    }
    // `host_steps:` run after enumeration (usb_host::parse_host_steps).
    if let Some(steps) = p_cfg.config.get("host_steps") {
        let steps = crate::peripherals::usb_host::parse_host_steps(steps)
            .map_err(|e| anyhow::anyhow!("samd21_usb host_steps: {e}"))?;
        usb.queue_host_steps(steps);
    }
    Ok(Box::new(usb))
}

// Opt-in SAMD51 board models: fixed instance addresses, no config overrides.
// Cold construction keeps them out of the shared dispatcher's hot path.
#[cold]
#[inline(never)]
fn build_samd51_board_model(
    kind: &str,
    p_cfg: &PeripheralConfig,
) -> anyhow::Result<Box<dyn Peripheral>> {
    // The DAC's only options are the board's reference levels; the I2C
    // master's is the board's SCL rise time.
    let admitted = |key: &String| match kind {
        "samd51_dac" => key == "vrefa_mv" || key == "vddana_mv",
        "samd51_sercom_i2cm" | "samd21_sercom_i2cm" => key == "scl_rise_ns",
        // Board-level static levels on the AIN inputs: `input_mv: { <ain>: <mV> }`.
        "samd21_adc" => key == "input_mv",
        _ => false,
    };
    if !p_cfg.config.keys().all(admitted) {
        anyhow::bail!("{kind} admits no config overrides");
    }
    Ok(match kind {
        "samd51_oscctrl" => {
            if p_cfg.base_address != 0x40001000 || p_cfg.irq.is_some() {
                anyhow::bail!("SAMD51 OSCCTRL requires base 0x40001000 and no IRQ");
            }
            Box::new(crate::peripherals::sam::oscctrl::Samd51Oscctrl::new())
        }
        "samd51_adc" => {
            use crate::peripherals::sam::adc::{Samd51Adc, ADC_INSTANCES};
            let Some(instance) = ADC_INSTANCES.iter().find(|i| i.base == p_cfg.base_address) else {
                anyhow::bail!("SAMD51 ADC base must be ADC0 or ADC1");
            };
            if p_cfg.irq.is_some() {
                anyhow::bail!("SAMD51 ADC interrupts are not modelled");
            }
            Box::new(Samd51Adc::new(*instance))
        }
        "samd21_adc" => {
            use crate::peripherals::sam::adc21::Samd21Adc;
            if p_cfg.base_address != 0x42004000 {
                anyhow::bail!("SAMD21 ADC requires base 0x42004000");
            }
            if p_cfg.irq.is_some() {
                anyhow::bail!("SAMD21 ADC interrupts are not modelled");
            }
            let mut adc = Samd21Adc::new();
            if let Some(map) = p_cfg.config.get("input_mv").and_then(|v| v.as_mapping()) {
                for (k, v) in map {
                    match (k.as_u64(), v.as_u64()) {
                        (Some(ain), Some(mv)) if ain < 20 => {
                            adc.set_input_millivolts(ain as usize, mv as u32)
                        }
                        _ => anyhow::bail!("SAMD21 ADC input_mv wants {{ <ain 0..19>: <mV> }}"),
                    }
                }
            }
            Box::new(adc)
        }
        "samd51_tcc" => {
            use crate::peripherals::sam::tcc::{SamTcc, TCC_INSTANCES};
            let Some(instance) = TCC_INSTANCES.iter().find(|i| i.base == p_cfg.base_address) else {
                anyhow::bail!("SAMD51 TCC base must be one of TCC0..TCC4");
            };
            // Line 0 plus one line per channel are reported by the model itself.
            if p_cfg.irq.is_some_and(|irq| irq != instance.irq) {
                anyhow::bail!("SAMD51 TCC IRQ must be its first line {}", instance.irq);
            }
            Box::new(SamTcc::new(*instance))
        }
        "samd21_tcc" => {
            use crate::peripherals::sam::tcc::{SamTcc, SAMD21_TCC_INSTANCES};
            let Some(instance) = SAMD21_TCC_INSTANCES
                .iter()
                .find(|i| i.base == p_cfg.base_address)
            else {
                anyhow::bail!("SAMD21 TCC base must be one of TCC0..TCC2");
            };
            if p_cfg.irq.is_some_and(|irq| irq != instance.irq) {
                anyhow::bail!("SAMD21 TCC IRQ must be {}", instance.irq);
            }
            Box::new(SamTcc::new(*instance))
        }
        "samd51_dac" | "samd51_pac" => {
            let base = if kind == "samd51_dac" {
                0x43002400
            } else {
                0x40000000
            };
            if p_cfg.base_address != base || p_cfg.irq.is_some() {
                anyhow::bail!("{kind} requires base {base:#x} and no IRQ");
            }
            if kind == "samd51_dac" {
                let mv = |key: &str| p_cfg.config.get(key).and_then(|v| v.as_f64());
                Box::new(
                    crate::peripherals::sam::dac::Samd51Dac::new()
                        .with_references(mv("vrefa_mv"), mv("vddana_mv")),
                )
            } else {
                Box::new(crate::peripherals::sam::pac::Samd51Pac::new())
            }
        }
        "samd51_sercom_i2cm" | "samd21_sercom_i2cm" => {
            use crate::peripherals::sam::sercom_i2cm::{
                Samd51SercomI2cm, SAMD21_SERCOM_INSTANCES, SERCOM_INSTANCES,
            };
            let instances = if kind == "samd21_sercom_i2cm" {
                &SAMD21_SERCOM_INSTANCES
            } else {
                &SERCOM_INSTANCES
            };
            let Some(instance) = instances.iter().find(|i| i.base == p_cfg.base_address) else {
                anyhow::bail!("{kind} base must be one of SERCOM0..SERCOM5");
            };
            // The model reports its own lines: four on the SAMD51, the
            // instance's single line on the SAMD21.
            if p_cfg.irq.is_some() {
                anyhow::bail!("{kind} takes no single IRQ");
            }
            let rise = p_cfg.config.get("scl_rise_ns").and_then(|v| v.as_u64());
            Box::new(Samd51SercomI2cm::new(*instance).with_scl_rise_ns(rise.unwrap_or(0)))
        }
        "samd51_eic" => {
            // Sixteen channel lines (IRQ 12..27) are reported by the model itself.
            if p_cfg.base_address != 0x40002800 || p_cfg.irq.is_some() {
                anyhow::bail!("SAMD51 EIC requires base 0x40002800 and no single IRQ");
            }
            Box::new(crate::peripherals::sam::eic::Samd51Eic::new())
        }
        "samd51_gclk" => {
            if p_cfg.base_address != 0x40001c00 || p_cfg.irq.is_some() {
                anyhow::bail!("SAMD51 GCLK requires base 0x40001C00 and no IRQ");
            }
            Box::new(crate::peripherals::sam_clock::SamGclk::samd51())
        }
        "samd51_tc" => {
            use crate::peripherals::sam::tc::{Samd51Tc, TC_INSTANCES};
            let Some(instance) = TC_INSTANCES.iter().find(|i| i.base == p_cfg.base_address) else {
                anyhow::bail!("SAMD51 TC base must be one of TC0..TC5");
            };
            if p_cfg.irq != Some(instance.irq) {
                anyhow::bail!(
                    "SAMD51 TC at {:#x} requires IRQ {}",
                    instance.base,
                    instance.irq
                );
            }
            Box::new(Samd51Tc::new(*instance))
        }
        _ => anyhow::bail!("unknown SAMD51 board model {kind}"),
    })
}

// SAM D21 TC3..TC7: fixed instance addresses and IRQs, no config overrides.
#[cold]
#[inline(never)]
fn build_samd21_tc(p_cfg: &PeripheralConfig) -> anyhow::Result<Box<dyn Peripheral>> {
    use crate::peripherals::sam::tc21::{Samd21Tc, TC21_INSTANCES};
    if !p_cfg.config.is_empty() {
        anyhow::bail!("samd21_tc admits no config overrides");
    }
    let Some(instance) = TC21_INSTANCES.iter().find(|i| i.base == p_cfg.base_address) else {
        anyhow::bail!("SAMD21 TC base must be one of TC3..TC7");
    };
    if p_cfg.irq != Some(instance.irq) {
        anyhow::bail!(
            "SAMD21 TC at {:#x} requires IRQ {}",
            instance.base,
            instance.irq
        );
    }
    Ok(Box::new(Samd21Tc::new(*instance)))
}

// SAM4S TC block: fixed address and channel-0 IRQ, no config overrides.
#[cold]
#[inline(never)]
fn build_sam4s_tc(p_cfg: &PeripheralConfig) -> anyhow::Result<Box<dyn Peripheral>> {
    use crate::peripherals::sam::tc4s::{Sam4sTc, SAM4S_TC_BLOCKS};
    if !p_cfg.config.is_empty() {
        anyhow::bail!("sam4s_tc admits no config overrides");
    }
    let Some(block) = SAM4S_TC_BLOCKS
        .iter()
        .find(|b| b.base == p_cfg.base_address)
    else {
        anyhow::bail!("SAM4S TC base must be 0x40010000 (TC0..2) or 0x40014000 (TC3..5)");
    };
    if p_cfg.irq != Some(block.first_irq) {
        anyhow::bail!(
            "SAM4S TC at {:#x} requires IRQ {} (channel 0)",
            block.base,
            block.first_irq
        );
    }
    Ok(Box::new(Sam4sTc::new(*block)))
}

// EFM32 Series-0/1 TIMER and Series-1 WTIMER: base from the chip yaml, IRQ
// required (it differs per part: TIMER1 is IRQ 10 on EFM32G210, 12 on
// EFM32GG), no config keys.
#[cold]
#[inline(never)]
fn build_efm32_timer(ty: &str, p_cfg: &PeripheralConfig) -> anyhow::Result<Box<dyn Peripheral>> {
    use crate::peripherals::efm32::timer::Efm32Timer;
    if !p_cfg.config.is_empty() {
        anyhow::bail!("{ty} admits no config overrides");
    }
    let Some(irq) = p_cfg.irq else {
        anyhow::bail!("{ty} '{}' needs an `irq`", p_cfg.id);
    };
    Ok(Box::new(match ty {
        "efm32s1_wtimer" => Efm32Timer::new_wtimer(irq),
        "efm32s1_timer" => Efm32Timer::new_series1(irq),
        _ => Efm32Timer::new(irq),
    }))
}

// EFR32 Series-1 RTCC: base from the chip yaml, IRQ required (30 on MG12,
// 31 on MG13), optional `lfeclk_hz` (default 32768, as the Renode repl).
#[cold]
#[inline(never)]
fn build_efm32_rtcc(p_cfg: &PeripheralConfig) -> anyhow::Result<Box<dyn Peripheral>> {
    use crate::peripherals::efm32::rtcc::{Efm32Rtcc, RTCC_DEFAULT_LFECLK_HZ};
    if let Some(k) = p_cfg.config.keys().find(|k| k.as_str() != "lfeclk_hz") {
        anyhow::bail!("efm32s1_rtcc admits only `lfeclk_hz`, not `{k}`");
    }
    let Some(irq) = p_cfg.irq else {
        anyhow::bail!("efm32s1_rtcc '{}' needs an `irq`", p_cfg.id);
    };
    let hz = match p_cfg.config.get("lfeclk_hz") {
        None => RTCC_DEFAULT_LFECLK_HZ,
        Some(v) => v.as_u64().filter(|&hz| hz > 0).ok_or_else(|| {
            anyhow::anyhow!("efm32s1_rtcc `lfeclk_hz` must be a positive integer")
        })?,
    };
    Ok(Box::new(Efm32Rtcc::new(irq, hz)))
}

// Nuvoton NPCX ITIM32 / ITIM64: IRQ required; optional `apb2_hz` (default
// the attached core clock) and `lfclk_hz` (default 32768, as Renode).
#[cold]
#[inline(never)]
fn build_npcx_itim(ty: &str, p_cfg: &PeripheralConfig) -> anyhow::Result<Box<dyn Peripheral>> {
    use crate::peripherals::npcx_itim::{NpcxItim, DEFAULT_LFCLK_HZ};
    if let Some(k) = p_cfg
        .config
        .keys()
        .find(|k| !matches!(k.as_str(), "apb2_hz" | "lfclk_hz"))
    {
        anyhow::bail!("{ty} admits only `apb2_hz` and `lfclk_hz`, not `{k}`");
    }
    if p_cfg.irq.is_none() {
        anyhow::bail!("{ty} '{}' needs an `irq`", p_cfg.id);
    }
    let hz = |k: &str| -> anyhow::Result<Option<u64>> {
        match p_cfg.config.get(k) {
            None => Ok(None),
            Some(v) => v
                .as_u64()
                .filter(|&hz| hz > 0)
                .map(Some)
                .ok_or_else(|| anyhow::anyhow!("{ty} `{k}` must be a positive integer")),
        }
    };
    Ok(Box::new(NpcxItim::new(
        ty == "npcx_itim64",
        hz("apb2_hz")?,
        hz("lfclk_hz")?.unwrap_or(DEFAULT_LFCLK_HZ),
    )))
}

// Nuvoton NPCX MTC: IRQ required; optional `tick_hz` (default 1, as Renode).
#[cold]
#[inline(never)]
fn build_npcx_mtc(p_cfg: &PeripheralConfig) -> anyhow::Result<Box<dyn Peripheral>> {
    use crate::peripherals::npcx_mtc::{NpcxMtc, DEFAULT_TICK_HZ};
    if let Some(k) = p_cfg.config.keys().find(|k| k.as_str() != "tick_hz") {
        anyhow::bail!("npcx_mtc admits only `tick_hz`, not `{k}`");
    }
    if p_cfg.irq.is_none() {
        anyhow::bail!("npcx_mtc '{}' needs an `irq`", p_cfg.id);
    }
    let hz = match p_cfg.config.get("tick_hz") {
        None => DEFAULT_TICK_HZ,
        Some(v) => v
            .as_u64()
            .filter(|&hz| hz > 0)
            .ok_or_else(|| anyhow::anyhow!("npcx_mtc `tick_hz` must be a positive integer"))?,
    };
    Ok(Box::new(NpcxMtc::new(hz)))
}

// Nuvoton NPCX TWD: IRQ required; optional `lfclk_hz` (default 32768, as
// Renode).
#[cold]
#[inline(never)]
fn build_npcx_twd(p_cfg: &PeripheralConfig) -> anyhow::Result<Box<dyn Peripheral>> {
    use crate::peripherals::npcx_twd::{NpcxTwd, DEFAULT_LFCLK_HZ};
    if let Some(k) = p_cfg.config.keys().find(|k| k.as_str() != "lfclk_hz") {
        anyhow::bail!("npcx_twd admits only `lfclk_hz`, not `{k}`");
    }
    if p_cfg.irq.is_none() {
        anyhow::bail!("npcx_twd '{}' needs an `irq`", p_cfg.id);
    }
    let hz = match p_cfg.config.get("lfclk_hz") {
        None => DEFAULT_LFCLK_HZ,
        Some(v) => v
            .as_u64()
            .filter(|&hz| hz > 0)
            .ok_or_else(|| anyhow::anyhow!("npcx_twd `lfclk_hz` must be a positive integer"))?,
    };
    Ok(Box::new(NpcxTwd::new(hz)))
}

// Nuvoton NPCX HFCG / LFCG: register files only; no IRQ and no options.
#[cold]
#[inline(never)]
fn build_npcx_cdcg(
    canonical_type: &str,
    p_cfg: &PeripheralConfig,
) -> anyhow::Result<Box<dyn Peripheral>> {
    use crate::peripherals::npcx_cdcg::{CdcgBlock, NpcxCdcg};
    if let Some(k) = p_cfg.config.keys().next() {
        anyhow::bail!("{canonical_type} admits no options, not `{k}`");
    }
    if p_cfg.irq.is_some() {
        anyhow::bail!("{canonical_type} '{}' has no IRQ line", p_cfg.id);
    }
    let block = if canonical_type == "npcx_hfcg" {
        CdcgBlock::Hfcg
    } else {
        CdcgBlock::Lfcg
    };
    Ok(Box::new(NpcxCdcg::new(block)))
}

// NPCX FIU user-mode access path and internal flash medium. The direct XIP
// memory window is intentionally separate and is not declared until it can
// share storage with the controller.
#[cold]
#[inline(never)]
fn build_npcx_fiu(p_cfg: &PeripheralConfig) -> anyhow::Result<Box<dyn Peripheral>> {
    use crate::peripherals::npcx_fiu::NpcxFiu;
    if let Some(k) = p_cfg.config.keys().find(|k| k.as_str() != "flash_size") {
        anyhow::bail!("npcx_fiu admits only `flash_size`, not `{k}`");
    }
    if p_cfg.irq.is_some() {
        anyhow::bail!("npcx_fiu '{}' has no IRQ line", p_cfg.id);
    }
    let size = p_cfg
        .config
        .get("flash_size")
        .and_then(|value| value.as_u64())
        .filter(|size| *size > 0 && *size <= 16 * 1024 * 1024)
        .ok_or_else(|| anyhow::anyhow!("npcx_fiu `flash_size` must be 1..16 MiB in bytes"))?;
    Ok(Box::new(NpcxFiu::new(size as usize)))
}

// NPCX MDMA UART receive channel. The Renode MDMA IRQ is combined with the
// owning UART's IRQ; until the bus can express a shared level input, this
// profile intentionally does not accept `irq:`.
#[cold]
#[inline(never)]
fn build_npcx_mdma(p_cfg: &PeripheralConfig) -> anyhow::Result<Box<dyn Peripheral>> {
    use crate::peripherals::npcx_mdma::NpcxMdma;
    if let Some(k) = p_cfg.config.keys().find(|k| {
        !matches!(
            k.as_str(),
            "source_address" | "destination_address" | "rx_request"
        )
    }) {
        anyhow::bail!(
            "npcx_mdma admits only source_address, destination_address and rx_request, not `{k}`"
        );
    }
    if p_cfg.irq.is_some() {
        anyhow::bail!(
            "npcx_mdma '{}' has no shared UART/MDMA IRQ combiner",
            p_cfg.id
        );
    }
    let address = |key: &str, default: Option<u32>| -> anyhow::Result<u32> {
        match p_cfg.config.get(key) {
            None => {
                default.ok_or_else(|| anyhow::anyhow!("npcx_mdma '{}' needs `{key}`", p_cfg.id))
            }
            Some(value) => value
                .as_u64()
                .filter(|address| *address <= u64::from(u32::MAX))
                .map(|address| address as u32)
                .ok_or_else(|| anyhow::anyhow!("npcx_mdma `{key}` must be a 32-bit address")),
        }
    };
    let rx_request = match p_cfg.config.get("rx_request") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!("npcx_mdma `rx_request` must be a non-empty request name")
                })?
                .to_string(),
        ),
    };
    Ok(Box::new(NpcxMdma::new(
        address("source_address", None)?,
        address("destination_address", None)?,
        rx_request,
    )))
}

// SAM D21 RTC: base from the chip yaml, fixed IRQ; `gclk_hz` declares GCLK_RTC.
#[cold]
#[inline(never)]
fn build_samd21_rtc(p_cfg: &PeripheralConfig) -> anyhow::Result<Box<dyn Peripheral>> {
    use crate::peripherals::sam::rtc21::{Samd21Rtc, RTC21_DEFAULT_GCLK_HZ, RTC21_IRQ};
    if let Some(k) = p_cfg.config.keys().find(|k| k.as_str() != "gclk_hz") {
        anyhow::bail!("samd21_rtc admits only `gclk_hz`, not `{k}`");
    }
    if p_cfg.irq != Some(RTC21_IRQ) {
        anyhow::bail!("SAMD21 RTC sits on IRQ {RTC21_IRQ}");
    }
    let hz = match p_cfg.config.get("gclk_hz") {
        None => RTC21_DEFAULT_GCLK_HZ,
        Some(v) => v
            .as_u64()
            .filter(|&hz| hz > 0)
            .ok_or_else(|| anyhow::anyhow!("samd21_rtc `gclk_hz` must be a positive integer"))?,
    };
    Ok(Box::new(Samd21Rtc::new(hz)))
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
        "sifive_gpio" => Box::new(crate::peripherals::sifive_gpio::SifiveGpio::new()),
        "systick" | "arm_generictimer" => {
            // CALIB is implementation-defined per chip; the yaml can
            // supply it plus the external-source divider (for example HCLK/8)
            // via `config: { calib: ..., external_clock_divider: ... }`.
            // Cortex-M0's SYST_CSR reset is 0x2 when CALIB.NOREF is set; set
            // `no_reference_tickint_reset: true` for that core profile. M3/M4
            // reset only CLKSOURCE in that case.
            let calib = p_cfg
                .config
                .get("calib")
                .and_then(|v| v.as_u64())
                .unwrap_or(0x4000_0000) as u32;
            let external_clock_divider = match p_cfg.config.get("external_clock_divider") {
                Some(value) => value
                    .as_u64()
                    .filter(|&divider| divider > 0 && divider <= u32::MAX as u64)
                    .ok_or_else(|| {
                        anyhow::anyhow!("systick `external_clock_divider` must be a positive u32")
                    })? as u32,
                None => 1,
            };
            let no_reference_tickint_reset = match p_cfg.config.get("no_reference_tickint_reset") {
                None => false,
                Some(value) => value.as_bool().ok_or_else(|| {
                    anyhow::anyhow!("systick `no_reference_tickint_reset` must be a boolean")
                })?,
            };
            let systick =
                crate::peripherals::systick::Systick::with_calib_and_external_clock_divider(
                    calib,
                    external_clock_divider,
                );
            let systick = if no_reference_tickint_reset {
                systick.with_no_reference_tickint_reset()
            } else {
                systick
            };
            // `external_ref_hz`: the reference clock CLKSOURCE = 0 selects.
            match p_cfg.config.get("external_ref_hz").and_then(|v| v.as_u64()) {
                Some(hz) => Box::new(systick.with_external_ref_hz(hz)),
                None => Box::new(systick),
            }
        }
        "arm_sp804_timer" => Box::new(crate::peripherals::arm_sp804::ArmSp804Timer::new()),
        "lpc2294_timer" => Box::new(crate::peripherals::lpc::timer::LpcTimer::new()),
        "arm_pl190_vic" => Box::new(crate::peripherals::armv7::pl190_vic::Pl190Vic::new()),
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
        "efr32s2_msc" => Box::new(crate::peripherals::efr32::msc::Efr32s2Msc::new()),
        "efr32s2_usartroute" => {
            Box::new(crate::peripherals::efr32::usart_route::Efr32s2UsartRoute::new())
        }
        "efr32s2_i2croute" => {
            Box::new(crate::peripherals::efr32::usart_route::Efr32s2I2cRoute::new())
        }
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
        // The GPIO block's analog-bus allocation window (ABUSALLOC/BBUSALLOC/
        // CDBUSALLOC). A silicon-correct `analogRead` writes it before every
        // conversion; unmapped, that store bus-faulted and parked the sketch.
        "efr32s2_busalloc" => Box::new(crate::peripherals::efr32::busalloc::Efr32s2BusAlloc::new()),
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
            // `rcc_ahb1enr_reset`: the part's AHB1ENR reset value (F7: DTCMRAMEN).
            if let Some(v) = p_cfg
                .config
                .get("rcc_ahb1enr_reset")
                .and_then(|v| v.as_u64())
            {
                rcc.set_f4_ahb1enr_reset(v as u32);
            }
            // F1 ENR writable masks are per-density. YAML: `config: {
            // rcc_ahbenr_mask, rcc_apb1enr_mask, rcc_apb2enr_mask }`; an
            // absent key keeps the medium-density default.
            let f1 = |k: &str| -> Option<u32> {
                p_cfg
                    .config
                    .get(k)
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32)
            };
            rcc.set_f1_enr_masks(
                f1("rcc_ahbenr_mask"),
                f1("rcc_apb1enr_mask"),
                f1("rcc_apb2enr_mask"),
            );
            // F42x/F446 PLLSAI + DCKCFGR/DCKCFGR2 (`pllsai: true`).
            if p_cfg.config.get("pllsai").and_then(|v| v.as_bool()) == Some(true) {
                rcc.set_f4_pllsai(true);
            }
            // HSE is a board component. U5 reference platforms have none
            // fitted unless the chip yaml says `hse: true`.
            if let Some(present) = p_cfg.config.get("hse").and_then(|v| v.as_bool()) {
                rcc.set_hse_present(present);
            }
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
                let mut timer = crate::peripherals::nrf52::timer::Nrf52Timer::new_with_cc(num_cc);
                // ⚠️ The TIMER base clock is 16 MHz on nRF52, not `cpu_hz`
                // (64 MHz). Absent key = CPU clock; see the model's header.
                if let Some(hz) = p_cfg.config.get("peripheral_hz").and_then(|v| v.as_u64()) {
                    timer.set_peripheral_hz(hz);
                }
                Box::new(timer)
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
                // `input_capture: stm32f4` — the F2/F4-generation input
                // stage: CCxNP writable (both-edge capture) and the F4 AF
                // pad map routes TIMx_CHn pads to this timer. Anything else
                // is refused rather than guessed.
                let ccer_np = match p_cfg.config.get("input_capture").and_then(|v| v.as_str()) {
                    None => false,
                    Some("stm32f4") => true,
                    Some(other) => {
                        return Err(anyhow::anyhow!(
                            "timer '{}': unknown input_capture '{other}' (supported: stm32f4)",
                            p_cfg.id
                        ))
                    }
                };
                // `pad_map: stm32g0 | stm32g4 | stm32l0 | ... | wch_v00x` —
                // the alternate-function table the bus routes this timer's
                // channel outputs through (the F4 one rides on
                // `input_capture: stm32f4`). `wch_v00x` is the CH32V003 fixed
                // map, so it carries no AF nibble.
                let pad_map = match p_cfg.config.get("pad_map").and_then(|v| v.as_str()) {
                    None => None,
                    Some("at32f421") => Some("at32f421"),
                    Some("stm32g0") => Some("stm32g0"),
                    Some("stm32c0") => Some("stm32c0"),
                    Some("stm32g4") => Some("stm32g4"),
                    Some("stm32l0") => Some("stm32l0"),
                    Some("stm32l4") => Some("stm32l4"),
                    Some("stm32wb") => Some("stm32wb"),
                    Some("stm32wba") => Some("stm32wba"),
                    Some("stm32u5") => Some("stm32u5"),
                    Some("stm32h5") => Some("stm32h5"),
                    Some("stm32h7") => Some("stm32h7"),
                    Some("wch_v00x") => Some("wch_v00x"),
                    Some(other) => {
                        return Err(anyhow::anyhow!(
                            "timer '{}': unknown pad_map '{other}' (supported: at32f421, stm32g0, stm32c0, stm32g4, stm32l0, stm32l4, stm32wb, stm32wba, stm32u5, stm32h5, stm32h7, wch_v00x)",
                            p_cfg.id
                        ))
                    }
                };
                // Separate vectors of a split timer interrupt:
                // `capture_compare_irq: N` for CC1..4IF (AT32F421 TMR1:
                // overflow/trigger/hall IRQ13, channel IRQ14),
                // `trigger_com_irq: N` for TIF/COMIF and `break_irq: N` for
                // BIF (STM32F1/F4 TIM1_TRG_COM / TIM1_BRK).
                // `prescaler: power_of_two` — PSC[3:0] is an exponent, the
                // counter clock is the kernel clock / 2^PSC (STM32W108).
                let psc_power_of_two = match p_cfg.config.get("prescaler").and_then(|v| v.as_str())
                {
                    None | Some("linear") => false,
                    Some("power_of_two") => true,
                    Some(other) => {
                        return Err(anyhow::anyhow!(
                        "timer '{}': unknown prescaler '{other}' (supported: linear, power_of_two)",
                        p_cfg.id
                    ))
                    }
                };
                let split_irq = |key: &str| {
                    p_cfg
                        .config
                        .get(key)
                        .map(|value| {
                            value
                                .as_u64()
                                .and_then(|v| u32::try_from(v).ok())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("timer '{}': {key} must be a u32", p_cfg.id)
                                })
                        })
                        .transpose()
                };
                Box::new(
                    crate::peripherals::timer::Timer::new_with_layout(width, advanced)
                        .basic(basic)
                        .ccer_np(ccer_np)
                        .capture_compare_irq(split_irq("capture_compare_irq")?)
                        .trigger_com_irq(split_irq("trigger_com_irq")?)
                        .break_irq(split_irq("break_irq")?)
                        .pad_map(pad_map)
                        .psc_power_of_two(psc_power_of_two),
                )
            }
        }
        "avr_gpio" => {
            let pin_count = p_cfg
                .config
                .get("pin_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(8);
            if !(1..=8).contains(&pin_count) {
                anyhow::bail!(
                    "avr_gpio '{}': config.pin_count must be in 1..=8, got {pin_count}",
                    p_cfg.id
                );
            }
            let layout = p_cfg.config.get("layout").and_then(|v| v.as_str());
            match layout {
                None | Some("atmega") => Box::new(
                    crate::peripherals::avr_gpio::AvrGpioPort::new_with_pin_count(
                        pin_count as u8,
                    ),
                ),
                Some("avrxt_vport") => Box::new(
                    crate::peripherals::avr_gpio::AvrGpioPort::new_avrxt_vport(
                        pin_count as u8,
                    ),
                ),
                Some("megaavr_port") => Box::new(
                    crate::peripherals::avr_gpio::AvrGpioPort::new_megaavr_port(pin_count as u8),
                ),
                Some(other) => anyhow::bail!(
                    "avr_gpio '{}': unknown config.layout '{other}' (supported: atmega, avrxt_vport, megaavr_port)",
                    p_cfg.id
                ),
            }
        }
        "pic24_gpio" => {
            // The dsPIC33FJ32MC204 has no uniform port width: PORTA presents
            // RA0-4/RA7-10, PORTB RB0-15, PORTC RC0-9 (DS70283H pin diagrams,
            // Table 4-20/21/22). The package mask comes from the instance.
            let default_mask = match p_cfg.config.get("port").and_then(|v| v.as_str()) {
                Some("a") => 0x079F,
                Some("b") => 0xFFFF,
                Some("c") => 0x03FF,
                other => {
                    return Err(anyhow::anyhow!(
                        "pic24_gpio '{}': config.port must be \"a\", \"b\" or \"c\", got {other:?}",
                        p_cfg.id
                    ))
                }
            };
            // The package mask is per part (the 28-pin sibling has RA0-RA4,
            // RB0-RB15 and no PORTC); `config.mask` overrides the 44-pin
            // default.
            let mask = p_cfg
                .config
                .get("mask")
                .and_then(|v| v.as_u64())
                .map(|v| v as u16)
                .unwrap_or(default_mask);
            Box::new(crate::peripherals::pic24_gpio::Pic24GpioPort::new(mask))
        }
        "avr_adc" => {
            // `config.channels`: 8 on the ATmega328P (default), 16 on the ATmega2560.
            let channels = p_cfg
                .config
                .get("channels")
                .and_then(|v| v.as_u64())
                .unwrap_or(crate::peripherals::avr_adc::AVR_ADC_CHANNELS as u64);
            Box::new(crate::peripherals::avr_adc::AvrAdcInputs::with_channels(
                channels as usize,
            ))
        }
        "sam_sercom_usart" => {
            Box::new(crate::peripherals::sam::sercom_usart::SamSercomUsart::new())
        }
        "samd51_oscctrl" | "samd51_gclk" | "samd51_tc" | "samd51_tcc" | "samd51_eic"
        | "samd51_dac" | "samd51_adc" | "samd51_pac" | "samd51_sercom_i2cm"
        | "samd21_sercom_i2cm" | "samd21_adc" | "samd21_tcc" => {
            build_samd51_board_model(canonical_type, p_cfg)?
        }
        "samd21_usb" => build_samd21_usb(p_cfg)?,
        "samd21_tc" => build_samd21_tc(p_cfg)?,
        "samd21_rtc" => build_samd21_rtc(p_cfg)?,
        "sam4s_tc" => build_sam4s_tc(p_cfg)?,
        "efm32s0_timer" | "efm32s1_timer" | "efm32s1_wtimer" => {
            build_efm32_timer(canonical_type, p_cfg)?
        }
        "efm32s1_rtcc" => build_efm32_rtcc(p_cfg)?,
        "npcx_itim32" | "npcx_itim64" => build_npcx_itim(canonical_type, p_cfg)?,
        "npcx_mdma" => build_npcx_mdma(p_cfg)?,
        "npcx_mtc" => build_npcx_mtc(p_cfg)?,
        "npcx_twd" => build_npcx_twd(p_cfg)?,
        "npcx_hfcg" | "npcx_lfcg" => build_npcx_cdcg(canonical_type, p_cfg)?,
        "npcx_fiu" => build_npcx_fiu(p_cfg)?,
        "npcx_gpio" => Box::new(crate::peripherals::npcx_gpio::NpcxGpio::new()),
        "npcx_spip" => Box::new(crate::peripherals::npcx_spip::NpcxSpip::new()),
        "npcx_smbus" => {
            if !p_cfg.config.is_empty() {
                anyhow::bail!("npcx_smbus '{}' accepts no config keys", p_cfg.id);
            }
            Box::new(crate::peripherals::npcx_smbus::NpcxSmbus::new())
        }
        "samd51_sercom_usart" => {
            use crate::peripherals::sam::sercom_usart::{SamSercomUsart, UsartPads};
            let key = |k: &str| p_cfg.config.get(k).and_then(|v| v.as_u64());
            let usart = SamSercomUsart::new_samd51();
            // Optional pads: all of tx/rx port and bit, pad_mux, txpo, rxpo.
            match (
                key("tx_port_base"),
                key("tx_bit"),
                key("rx_port_base"),
                key("rx_bit"),
                key("pad_mux"),
                key("txpo"),
                key("rxpo"),
            ) {
                (Some(tp), Some(tb), Some(rp), Some(rb), Some(mux), Some(txpo), Some(rxpo))
                    if tb < 32 && rb < 32 && mux < 16 && txpo < 4 && rxpo < 4 =>
                {
                    let usart = usart.with_pads(UsartPads {
                        tx: (tp, tb as u8),
                        rx: (rp, rb as u8),
                        mux: mux as u8,
                        txpo: txpo as u8,
                        rxpo: rxpo as u8,
                    });
                    // Optional: the core clock channel paces bytes at the baud.
                    match key("gclk_id") {
                        Some(channel) => Box::new(usart.with_pacing(channel)),
                        None => Box::new(usart),
                    }
                }
                (None, None, None, None, None, None, None) => Box::new(usart),
                _ => anyhow::bail!("SAMD51 USART '{}' needs all pad keys or none", p_cfg.id),
            }
        }
        "samd51_nvmctrl_manual_storage" | "samd51_cmcc_control" => {
            let expected = if canonical_type == "samd51_nvmctrl_manual_storage" {
                0x4100_4000
            } else {
                0x4100_6000
            };
            if p_cfg.base_address != expected || p_cfg.irq.is_some() || !p_cfg.config.is_empty() {
                anyhow::bail!(
                    "SAMD51 storage slice requires its exact base, no IRQ and no config overrides"
                );
            }
            if canonical_type == "samd51_nvmctrl_manual_storage" {
                Box::new(crate::peripherals::sam::nvmctrl::Samd51Nvmctrl::new())
            } else {
                Box::new(crate::peripherals::sam::cmcc::Samd51Cmcc::new())
            }
        }
        "sam_dmac" => {
            anyhow::bail!("full SAM DMAC is not admitted; sam_dmac_byte is an opt-in software-byte foundation only");
        }
        "sam_dmac_byte" => {
            if p_cfg.irq != Some(31) || p_cfg.base_address != 0x4100a000 {
                anyhow::bail!("sam_dmac_byte requires SAMD51 base 0x4100a000 and IRQ31");
            }
            Box::new(crate::peripherals::sam::dmac_byte::SamDmacByte::new(
                p_cfg.base_address,
            ))
        }
        "samd51_dmac_board" => {
            if p_cfg.irq != Some(31) || p_cfg.base_address != 0x4100a000 {
                anyhow::bail!("SAMD51 DMAC board profile requires base 0x4100a000 and IRQ31");
            }
            if !p_cfg.clock.as_ref().is_some_and(|clocks| {
                clocks
                    .as_slice()
                    .iter()
                    .any(|gate| gate.controller == "mclk" && gate.reg == "AHBMASK" && gate.bit == 9)
            }) {
                anyhow::bail!(
                    "SAMD51 DMAC board profile requires explicit mclk AHBMASK bit9 binding"
                );
            }
            Box::new(
                crate::peripherals::sam::dmac_byte::SamDmacByte::new_samd51_board(
                    p_cfg.base_address,
                ),
            )
        }
        "samd51_usb_device_banks" => build_samd51_usb_device_banks(p_cfg)?,
        "sam_dmac_sercom4_tx_byte" => {
            if p_cfg.irq != Some(31) || p_cfg.base_address != 0x4100a000 {
                anyhow::bail!("SERCOM4 TX byte profile requires SAMD51 base 0x4100a000 and IRQ31");
            }
            if !p_cfg.clock.as_ref().is_some_and(|clocks| {
                clocks
                    .as_slice()
                    .iter()
                    .any(|gate| gate.controller == "mclk" && gate.reg == "AHBMASK" && gate.bit == 9)
            }) {
                anyhow::bail!(
                    "SERCOM4 TX byte profile requires explicit mclk AHBMASK bit9 binding"
                );
            }
            Box::new(
                crate::peripherals::sam::dmac_byte::SamDmacByte::new_sercom4_tx(p_cfg.base_address),
            )
        }
        "sam_sercom_spi" => {
            let number = |key: &str| -> anyhow::Result<u64> {
                p_cfg
                    .config
                    .get(key)
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| {
                        anyhow::anyhow!("SAM SPI '{}' requires integer config.{key}", p_cfg.id)
                    })
            };
            let sck = number("sck_bit")?;
            let mosi = number("mosi_bit")?;
            let mux = number("pad_mux")?;
            let dopo = number("dopo")?;
            if sck >= 32 || mosi >= 32 || mux >= 16 || dopo >= 4 {
                anyhow::bail!("SAM SPI '{}' has invalid pad configuration", p_cfg.id);
            }
            // Optional: the SERCOM's DMAC trigger sources (both or neither).
            let optional = |key: &str| p_cfg.config.get(key).and_then(|v| v.as_u64());
            let dma = match (optional("dma_tx_trigger"), optional("dma_rx_trigger")) {
                (Some(tx), Some(rx)) => Some((tx as u32, rx as u32)),
                (None, None) => None,
                _ => anyhow::bail!(
                    "SAM SPI '{}' declares one DMA trigger without the other",
                    p_cfg.id
                ),
            };
            Box::new(crate::peripherals::sam::sercom_spi::SamSercomSpi::new(
                crate::peripherals::sam::sercom_spi::SamSpiPads {
                    port_base: number("port_base")?,
                    sck: sck as u8,
                    mosi: mosi as u8,
                    mux: mux as u8,
                    dopo: dopo as u8,
                    // Optional: the SERCOM core clock channel times bytes on
                    // the live clock tree.
                    core_gclk: p_cfg.config.get("gclk_id").and_then(|v| v.as_u64()),
                    dma,
                },
            ))
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
            let limited = p_cfg
                .config
                .get("limited_features")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut spi = crate::peripherals::spi::Spi::new_with_layout_cr2(layout, cr2_mask);
            if limited {
                spi = spi.as_limited();
            }
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
                // `over_drive: true` adds the F42x/F446 ODEN/ODSWEN handshake.
                Some("stm32f4") | Some("f4") => {
                    if p_cfg
                        .config
                        .get("over_drive")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        Box::new(crate::peripherals::pwr::PwrF4::with_over_drive())
                    } else {
                        Box::new(crate::peripherals::pwr::PwrF4::new())
                    }
                }
                // F1: PWR_CR/PWR_CSR, both reset 0 (RM0008 §5.4). DBP gates
                // the F1 BKP backup registers.
                Some("stm32f1") | Some("f1") => Box::new(crate::peripherals::pwr::PwrF1::new()),
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
        "rp2040_timer" => {
            let layout = match p_cfg.config.get("profile").and_then(|v| v.as_str()) {
                Some("rp2350") => crate::peripherals::rp2040::timer::TimerLayout::Rp2350,
                // Absent/anything else: the RP2040 map.
                _ => crate::peripherals::rp2040::timer::TimerLayout::Rp2040,
            };
            Box::new(crate::peripherals::rp2040::timer::Rp2040Timer::new().with_layout(layout))
        }
        "rp2040_dma" => Box::new(crate::peripherals::rp2040::dma::Rp2040Dma::new()),
        "rp2040_io_bank0" => {
            let layout = match p_cfg.config.get("profile").and_then(|v| v.as_str()) {
                Some("rp2350") => crate::peripherals::rp2040::io_bank0::IoBankLayout::Rp2350,
                // Absent/anything else: the RP2040 map.
                _ => crate::peripherals::rp2040::io_bank0::IoBankLayout::Rp2040,
            };
            Box::new(crate::peripherals::rp2040::io_bank0::Rp2040IoBank0::new().with_layout(layout))
        }
        "rp2040_sio" => {
            let layout = match p_cfg.config.get("profile").and_then(|v| v.as_str()) {
                Some("rp2350") => crate::peripherals::rp2040::sio::SioLayout::Rp2350,
                // Absent/anything else: the RP2040 map.
                _ => crate::peripherals::rp2040::sio::SioLayout::Rp2040,
            };
            let mut sio = crate::peripherals::rp2040::sio::Rp2040Sio::new().with_layout(layout);
            // The PADS_BANK0 block holding the pads' PUE/PDE pulls.
            if let Some(block) = p_cfg.config.get("pad_control") {
                let block = block.as_str().ok_or_else(|| {
                    anyhow::anyhow!(
                        "SIO peripheral '{}': `pad_control` must name a peripheral id",
                        p_cfg.id
                    )
                })?;
                sio = sio.with_pad_control(block);
            }
            Box::new(sio)
        }
        "rp2040_spi" => Box::new(crate::peripherals::rp2040::spi::Rp2040Spi::new()),
        "rp2040_i2c" => Box::new(crate::peripherals::rp2040::i2c::Rp2040I2c::new()),
        "rp2040_pwm" => {
            let layout = match p_cfg.config.get("profile").and_then(|v| v.as_str()) {
                Some("rp2350") => crate::peripherals::rp2040::pwm::PwmLayout::Rp2350,
                // Absent/anything else: the RP2040 map.
                _ => crate::peripherals::rp2040::pwm::PwmLayout::Rp2040,
            };
            Box::new(crate::peripherals::rp2040::pwm::Rp2040Pwm::new().with_layout(layout))
        }
        "rp2040_adc" => Box::new(
            crate::peripherals::rp2040::adc::Rp2040Adc::new().with_irq(p_cfg.irq.unwrap_or(22)),
        ),
        "rp2040_rtc" => Box::new(crate::peripherals::rp2040::rtc::Rp2040Rtc::new()),
        "rp2040_watchdog" => Box::new(crate::peripherals::rp2040::watchdog::Rp2040Watchdog::new()),
        "rp2040_xip_ssi" => Box::new(crate::peripherals::rp2040::xip_ssi::Rp2040XipSsi::new()),
        "rp2350_bootram" => Box::new(crate::peripherals::rp2350_bootram::Rp2350BootRam::new()),
        "cortex_m_coredebug" => Box::new(crate::peripherals::scs_debug::ScsDebug::without_probe(
            crate::peripherals::scs_debug::DebugHaltState::new(),
        )),
        "rp2040_usb" => Box::new(
            crate::peripherals::rp2040::usb::Rp2040Usb::new().with_irq(p_cfg.irq.unwrap_or(5)),
        ),
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
        "crs" => match p_cfg.config.get("trim_bits").and_then(|v| v.as_u64()) {
            Some(bits) if (6..=7).contains(&bits) => {
                Box::new(crate::peripherals::crs::Crs::with_trim_bits(bits as u32))
            }
            Some(bits) => {
                return Err(anyhow::anyhow!(
                    "crs `{}`: trim_bits must be 6 or 7, got {bits}",
                    p_cfg.id
                ))
            }
            None => Box::new(crate::peripherals::crs::Crs::new()),
        },
        "rtc" => Box::new(crate::peripherals::rtc::Rtc::new()),
        "rtc_f1" => {
            let rtc = crate::peripherals::rtc_f1::RtcF1::new();
            match p_cfg.config.get("alarm_irq").and_then(|v| v.as_u64()) {
                Some(line) => Box::new(rtc.with_alarm_irq(line as u32)),
                None => Box::new(rtc),
            }
        }
        "bkp_f1" => {
            // `data_registers: 10` (low/medium density, the default) or 42
            // (high/XL density, DR11..DR42 at 0x40..0xBC).
            let data_registers = p_cfg
                .config
                .get("data_registers")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as usize;
            Box::new(crate::peripherals::bkp_f1::BkpF1::new(data_registers)?)
        }
        "rtc_v3" => Box::new(crate::peripherals::rtc_v3::RtcV3::new()),
        "iwdg" => {
            let iwdg = crate::peripherals::iwdg::Iwdg::new();
            // `reset_on_expiry: { lsi_hz: N }` opts in to a counting IWDG
            // that resets the machine; absent keeps the latch-only model.
            match p_cfg.config.get("reset_on_expiry") {
                None => Box::new(iwdg),
                Some(v) => {
                    let lsi_hz = v
                        .get("lsi_hz")
                        .and_then(|hz| hz.as_u64())
                        .filter(|hz| *hz > 0)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "iwdg '{}': reset_on_expiry needs a positive lsi_hz",
                                p_cfg.id
                            )
                        })?;
                    Box::new(iwdg.with_reset_on_expiry(lsi_hz))
                }
            }
        }
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
        // Data-declared comparator: `profile` picks the register file, the
        // `plus` / `minus` lists name what each selector value reaches.
        "analog_comparator" => {
            use crate::peripherals::analog_comparator::{AnalogComparator, ComparatorProfile};
            let cfg = &p_cfg.config;
            let profile = match cfg.get("profile").and_then(|v| v.as_str()) {
                Some("at32f421") => ComparatorProfile::At32F421,
                other => anyhow::bail!(
                    "analog_comparator `{}`: unknown profile {other:?} (expected at32f421)",
                    p_cfg.id
                ),
            };
            let list = |key: &str| -> anyhow::Result<Vec<String>> {
                cfg.get(key)
                    .and_then(|v| v.as_sequence())
                    .ok_or_else(|| {
                        anyhow::anyhow!("analog_comparator `{}`: `{key}` list required", p_cfg.id)
                    })?
                    .iter()
                    .map(|v| {
                        v.as_str().map(str::to_string).ok_or_else(|| {
                            anyhow::anyhow!(
                                "analog_comparator `{}`: `{key}` entries are names",
                                p_cfg.id
                            )
                        })
                    })
                    .collect()
            };
            let vrefint_mv = cfg
                .get("vrefint_mv")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| {
                    anyhow::anyhow!("analog_comparator `{}`: `vrefint_mv` required", p_cfg.id)
                })?;
            let exti_line = match cfg.get("exti_line") {
                None => None,
                Some(v) => Some(v.as_u64().and_then(|l| u8::try_from(l).ok()).ok_or_else(
                    || anyhow::anyhow!("analog_comparator `{}`: bad exti_line", p_cfg.id),
                )?),
            };
            Box::new(AnalogComparator::new(
                profile,
                &list("plus")?,
                &list("minus")?,
                vrefint_mv,
                exti_line,
            )?)
        }
        "tsc" => Box::new(crate::peripherals::tsc::Tsc::new()),
        "fmc" => match p_cfg.config.get("profile").and_then(|v| v.as_str()) {
            Some("stm32h7_boot_disable") => {
                Box::new(crate::peripherals::fmc::H7BootDisable::default())
            }
            _ => Box::new(crate::peripherals::fmc::Fmc::new()),
        },
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
            // `line_mask: 0x2bffff` names the implemented lines exactly when
            // they are not contiguous (AT32F421: 0..17, 19, 21).
            let line_mask = match p_cfg.config.get("line_mask") {
                None => line_mask,
                Some(v) => v
                    .as_u64()
                    .and_then(|m| u32::try_from(m).ok())
                    .ok_or_else(|| {
                        anyhow::anyhow!("exti `{}`: line_mask must be a u32", p_cfg.id)
                    })?,
            };
            let mut exti = crate::peripherals::exti::Exti::new_with_layout_lines(layout, line_mask);
            // `line_irqs: { 21: 12 }` wires internal lines to their vectors.
            if let Some(map) = p_cfg.config.get("line_irqs") {
                let map = map.as_mapping().ok_or_else(|| {
                    anyhow::anyhow!("exti `{}`: line_irqs must map line -> irq", p_cfg.id)
                })?;
                let mut line_irqs = Vec::new();
                for (line, irq) in map {
                    let (Some(line), Some(irq)) = (line.as_u64(), irq.as_u64()) else {
                        anyhow::bail!("exti `{}`: line_irqs entries are integers", p_cfg.id);
                    };
                    line_irqs.push((u8::try_from(line)?, u32::try_from(irq)?));
                }
                exti.set_line_irqs(line_irqs)
                    .map_err(|e| anyhow::anyhow!("exti `{}`: {e}", p_cfg.id))?;
            }
            Box::new(exti)
        }
        "afio" => match p_cfg.config.get("profile").and_then(|v| v.as_str()) {
            None | Some("stm32f1") => Box::new(crate::peripherals::afio::Afio::new()),
            Some("wch_v00x") => Box::new(crate::peripherals::afio::Afio::new_with_layout(
                crate::peripherals::afio::AfioLayout::WchV00x,
            )),
            Some(other) => anyhow::bail!(
                "afio `{}`: unsupported profile '{other}'; supported: stm32f1, wch_v00x",
                p_cfg.id
            ),
        },
        // Channel-based DMA (F1/L0/L4/G0). `config: { channel_irqs: [11, ...,
        // 17] }` gives each channel its own NVIC line (F1/L1); `requests: {
        // spi1_tx: 3, spi1_rx: [2, 1] } }` hard-wires a peripheral request to a channel
        // (F1), or — `[channel, selector]` — to a channel while its L4
        // `CSELR` nibble holds the selector.
        "dma" | "stm32dma" => {
            let mut dma = match p_cfg.config.get("profile").and_then(|v| v.as_str()) {
                None | Some("legacy") => crate::peripherals::dma::Dma1::new(),
                Some("stm32g0") => crate::peripherals::dma::Dma1::new_stm32g0(),
                Some(profile) => {
                    anyhow::bail!("dma `{}`: unsupported profile `{profile}`", p_cfg.id)
                }
            };
            // `channels: N` declares a controller with fewer than seven
            // channels (STM32F030 / AT32F421 DMA1 have five).
            let mut channels = 7usize;
            if let Some(n) = p_cfg.config.get("channels") {
                let n = n
                    .as_u64()
                    .filter(|n| (1..=7).contains(n))
                    .ok_or_else(|| anyhow::anyhow!("dma `{}`: channels must be 1..=7", p_cfg.id))?;
                channels = n as usize;
                dma = dma.with_channel_count(channels);
            }
            // `absent_channels: reserved`: the missing channel blocks read
            // zero and ignore writes (F03x/F04x/F05x DMA1, RM0091 §10.3.2).
            match p_cfg.config.get("absent_channels").map(|v| v.as_str()) {
                None => {}
                Some(Some("reserved")) => dma = dma.with_reserved_absent_channels(),
                Some(_) => anyhow::bail!("dma `{}`: absent_channels must be `reserved`", p_cfg.id),
            }
            if let Some(irqs) = p_cfg.config.get("channel_irqs") {
                let mut irqs: Vec<u32> = irqs
                    .as_sequence()
                    .map(|s| {
                        s.iter()
                            .filter_map(|n| n.as_u64())
                            .map(|n| n as u32)
                            .collect()
                    })
                    .unwrap_or_default();
                if irqs.len() != channels {
                    anyhow::bail!(
                        "dma `{}`: channel_irqs must list {channels} NVIC lines",
                        p_cfg.id
                    )
                }
                // Unimplemented channels never raise a flag; their slot only
                // fills the array.
                irqs.resize(7, irqs[channels - 1]);
                let irqs: [u32; 7] = irqs.try_into().expect("resized to 7");
                dma = dma.with_channel_irqs(irqs);
            }
            for (request, v) in dma_request_table(p_cfg) {
                let pair = v.as_sequence().map(|s| {
                    s.iter()
                        .filter_map(|n| n.as_u64())
                        .map(|n| n as u32)
                        .collect::<Vec<_>>()
                });
                match (v.as_u64(), pair.as_deref()) {
                    (Some(ch), _) => dma = dma.with_request_route(&request, ch as u32, None),
                    (None, Some([ch, sel])) => {
                        dma = dma.with_request_route(&request, *ch, Some(*sel))
                    }
                    _ => anyhow::bail!(
                        "dma `{}`: request `{request}` must be a channel or [channel, cselr]",
                        p_cfg.id
                    ),
                }
            }
            Box::new(dma)
        }
        // STM32 DMA request multiplexer (G0/G4/WB). `config: { dma: dma1,
        // channels: 7, requests: { spi1_tx: 17, ... }, id_registers: {
        // 0x3FC: 0xA3C5DD01, ... } }`.
        "dmamux" => {
            let channels = p_cfg
                .config
                .get("channels")
                .and_then(|v| v.as_u64())
                .unwrap_or(7) as usize;
            let dma = p_cfg
                .config
                .get("dma")
                .and_then(|v| v.as_str())
                .unwrap_or("dma1");
            let mut mux = crate::peripherals::dmamux::Dmamux::new(channels, dma);
            if let Some(total) = p_cfg
                .config
                .get("physical_channels")
                .and_then(|v| v.as_u64())
            {
                anyhow::ensure!(
                    total as usize >= channels && total <= 32,
                    "dmamux physical_channels must cover modeled channels and be <=32"
                );
                mux = mux.with_physical_channels(total as usize);
            }
            for (request, v) in dma_request_table(p_cfg) {
                let Some(id) = v.as_u64() else {
                    anyhow::bail!(
                        "dmamux `{}`: request `{request}` needs a DMAREQ_ID",
                        p_cfg.id
                    );
                };
                mux = mux.with_request(&request, id as u32);
            }
            if let Some(map) = p_cfg
                .config
                .get("id_registers")
                .and_then(|v| v.as_mapping())
            {
                for (k, v) in map {
                    if let (Some(off), Some(val)) = (k.as_u64(), v.as_u64()) {
                        mux = mux.with_id_register(off, val as u32);
                    }
                }
            }
            Box::new(mux)
        }
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
            // `config: { requests: { spi1_tx: [[3, 3], [5, 3]] } }`: the
            // (stream, channel) pairs a peripheral request reaches.
            for (request, v) in dma_request_table(p_cfg) {
                for pair in v.as_sequence().into_iter().flatten() {
                    let ns: Vec<u32> = pair
                        .as_sequence()
                        .into_iter()
                        .flatten()
                        .filter_map(|n| n.as_u64().map(|n| n as u32))
                        .collect();
                    let [stream, chsel] = ns[..] else {
                        anyhow::bail!(
                            "stm32f4_dma `{}`: request `{request}` needs [stream, channel] pairs",
                            p_cfg.id
                        );
                    };
                    dma = dma.with_request_route(&request, stream, chsel);
                }
            }
            Box::new(dma)
        }
        "gpdma" => {
            // `config: { irq_base: N }` routes channel n to NVIC
            // line N + n (H563 GPDMA1: 27..34); `irq_high_base: M` routes the
            // U5's channels 8..15 to M + (n - 8) (80..87). Without either the
            // block's single `irq:` line serves every channel.
            // `config: { requests: { usart1_rx: 24, ... } }` maps the
            // `<peripheral id>_tx` / `_rx` request names the bus routes to the
            // numeric CxTR2.REQSEL values.
            let mut g =
                crate::peripherals::gpdma::Gpdma::new().with_base(p_cfg.base_address as u32);
            if let Some(base) = p_cfg.config.get("irq_base").and_then(|v| v.as_u64()) {
                g = g.with_irq_base(base as u32);
            }
            if let Some(base) = p_cfg.config.get("irq_high_base").and_then(|v| v.as_u64()) {
                g = g.with_irq_high_base(base as u32);
            }
            let mut requests = std::collections::BTreeMap::new();
            if let Some(table) = p_cfg.config.get("requests").and_then(|v| v.as_mapping()) {
                for (k, v) in table {
                    if let (Some(name), Some(id)) = (k.as_str(), v.as_u64()) {
                        requests.insert(name.to_string(), id as u32);
                    }
                }
            }
            if !requests.is_empty() {
                g = g.with_request_table(requests);
            }
            Box::new(g)
        }
        "adc" => {
            // The F0/L0 CHSELR block shares no sequencer with `adc.rs`'s
            // SQRx families, so it is its own model.
            use crate::peripherals::stm32f0_adc::{Stm32F0Adc, Stm32F0AdcFamily};
            match p_cfg.config.get("profile").and_then(|v| v.as_str()) {
                Some(profile @ ("stm32f0" | "stm32l0")) => {
                    let family = if profile == "stm32f0" {
                        Stm32F0AdcFamily::F0
                    } else {
                        Stm32F0AdcFamily::L0
                    };
                    let mut adc = Stm32F0Adc::new(family);
                    // Same board-level `input_mv: { <ch>: <mV> }` as the SQRx
                    // families below; a system that holds a pad (the Arduino
                    // matrix L5 potentiometer) must reach this model too.
                    if let Some(map) = p_cfg.config.get("input_mv").and_then(|v| v.as_mapping()) {
                        for (k, v) in map {
                            match (k.as_u64(), v.as_u64()) {
                                (Some(ch), Some(mv))
                                    if ch < 256
                                        && u16::try_from(mv).is_ok()
                                        && Peripheral::set_adc_channel_input(
                                            &mut adc, ch as u8, mv as u16,
                                        ) => {}
                                _ => anyhow::bail!(
                                    "adc '{}': input_mv wants {{ <channel 0..{}>: <mV> }}",
                                    p_cfg.id,
                                    Peripheral::adc_channel_count(&adc).unwrap_or(1) - 1
                                ),
                            }
                        }
                    }
                    Box::new(adc)
                }
                profile => {
                    // WCH CH32V00x (ch32v00x.h ADC_TypeDef): the STM32F1
                    // register file (STATR..RDATAR at the F1 offsets,
                    // CTLR2.SWSTART bit 22) with a 10-bit SAR, so RDATAR full
                    // scale is 1023.
                    let wch_v00x = profile == Some("wch_v00x");
                    let layout: crate::peripherals::adc::AdcRegisterLayout = if wch_v00x {
                        crate::peripherals::adc::AdcRegisterLayout::Stm32F1
                    } else {
                        SystemBus::parse_profile_or_default(p_cfg, "ADC")?
                    };
                    let mut adc = crate::peripherals::adc::Adc::new_with_layout(layout);
                    if wch_v00x {
                        adc.set_f1_resolution_bits(10);
                        adc.set_f1_internal_channels(false);
                    }
                    // Board-level static channel voltages: `input_mv: { <ch>: <mV> }`.
                    if let Some(map) = p_cfg.config.get("input_mv").and_then(|v| v.as_mapping()) {
                        for (k, v) in map {
                            match (k.as_u64(), v.as_u64()) {
                                (Some(ch), Some(mv)) if ch < u64::from(adc.channel_count()) => {
                                    adc.set_channel_input(ch as u8, mv as u16)
                                }
                                _ => anyhow::bail!(
                                    "adc '{}': input_mv wants {{ <channel 0..{}>: <mV> }}",
                                    p_cfg.id,
                                    adc.channel_count() - 1
                                ),
                            }
                        }
                    }
                    Box::new(adc)
                }
            }
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
        // U5 gets its limited register-contract model below; other chip
        // profiles use the read-as-zero stub. Cache contents and line effects
        // are not simulated because the simulator has flat memory.
        "icache" if p_cfg.config.get("profile").and_then(|v| v.as_str()) == Some("stm32u5") => {
            Box::new(crate::peripherals::icache::Icache::new())
        }
        "icache" | "dcache" => Box::new(crate::peripherals::stub::StubPeripheral::new(0x00)),
        "stm32u5_sysmem" => {
            let kb = p_cfg
                .config
                .get("flash_size_kb")
                .and_then(|v| v.as_u64())
                .unwrap_or(2048) as u16;
            Box::new(crate::peripherals::stm32u5_sysmem::Stm32u5Sysmem::new(kb))
        }
        "stm32u5_hash" => Box::new(crate::peripherals::stm32u5_hash::Stm32u5Hash::new()),
        "stm32u5_saes" => Box::new(crate::peripherals::stm32u5_saes::Stm32u5Saes::new()),
        "stm32u5_pka" => Box::new(crate::peripherals::stm32u5_pka::Stm32u5Pka::new()),
        // SYSCFG on STM32F4 (`profile: stm32f4`): a real register file,
        // because its EXTICR is the EXTI line-source mux external GPIO edges
        // go through.
        "syscfg" if p_cfg.config.get("profile").and_then(|v| v.as_str()) == Some("stm32f4") => {
            Box::new(crate::peripherals::syscfg::Stm32F4Syscfg::new())
        }
        // SYSCFG on STM32F74x/F75x (`profile: stm32f7`): the F4 register
        // file with the F7 MEMRMP / PMC fields (RM0385 §7).
        "syscfg" if p_cfg.config.get("profile").and_then(|v| v.as_str()) == Some("stm32f7") => {
            Box::new(crate::peripherals::syscfg::Stm32F4Syscfg::new_f7())
        }
        // SYSCFG on STM32F76x/F77x (`profile: stm32f76x`): the F7 file with
        // MEMRMP.SWP_FB (RM0410 §7).
        "syscfg" if p_cfg.config.get("profile").and_then(|v| v.as_str()) == Some("stm32f76x") => {
            Box::new(crate::peripherals::syscfg::Stm32F4Syscfg::new_f76x())
        }
        // L4 has the same EXTICR offsets, with three-bit port fields. Only
        // that mux is modeled; do not inherit the F4 compensation-cell fields.
        "syscfg" if p_cfg.config.get("profile").and_then(|v| v.as_str()) == Some("stm32l4") => {
            Box::new(crate::peripherals::syscfg::Stm32L4Syscfg::default())
        }
        // SYSCFG on STM32L0 (`profile: stm32l0`): the same EXTICR line-source
        // mux, among the L0's own configuration words (RM0367 §10).
        // SCFG on AT32F421 (`profile: at32f421`): CFG1 and the EXINTC
        // line-source mux at the STM32F0 SYSCFG offsets (RM_AT32F421 §7.2).
        "syscfg" if p_cfg.config.get("profile").and_then(|v| v.as_str()) == Some("at32f421") => {
            Box::new(crate::peripherals::syscfg::At32F421Scfg::new())
        }
        "syscfg" if p_cfg.config.get("profile").and_then(|v| v.as_str()) == Some("stm32l0") => {
            Box::new(crate::peripherals::syscfg::Stm32L0Syscfg::new())
        }
        // SYSCFG on STM32L1 (`profile: stm32l1`): the F4 offsets with the L1
        // EXTICR port codes (H = 5, F = 6, G = 7; RM0038 §8).
        "syscfg" if p_cfg.config.get("profile").and_then(|v| v.as_str()) == Some("stm32l1") => {
            Box::new(crate::peripherals::syscfg::Stm32L1Syscfg::new())
        }
        // SYSCFG on STM32F0 (`profile: stm32f0`): CFGR1, EXTICR1..4 with the
        // by-letter A..F port codes, CFGR2 (RM0091 §9).
        "syscfg" if p_cfg.config.get("profile").and_then(|v| v.as_str()) == Some("stm32f0") => {
            Box::new(crate::peripherals::syscfg::Stm32F0Syscfg::new())
        }
        // The F04x variant (`profile: stm32f04x`): the same map, with the F04x
        // CFGR1 fields (PA11_PA12_RMP, no F07x/F09x remaps above bit 23).
        "syscfg" if p_cfg.config.get("profile").and_then(|v| v.as_str()) == Some("stm32f04x") => {
            Box::new(crate::peripherals::syscfg::Stm32F0Syscfg::new_f04x())
        }
        // SYSCFG elsewhere — mostly EXTI source select (harmless read-0 stub), plus the H7
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
        // `config: { profile: k6x }` adds PLL0 (C5/C6, S.PLLST/LOCK0) for the
        // K64F; the default is the PLL-less KW41Z / Kinetis-L block.
        "nxp_mcg" | "kinetis_mcg" => match p_cfg.config.get("profile").and_then(|v| v.as_str()) {
            Some("k6x") => Box::new(crate::peripherals::mcg::Mcg::new_k6x()),
            None | Some("kl") => Box::new(crate::peripherals::mcg::Mcg::new()),
            Some(other) => anyhow::bail!("nxp_mcg: unknown profile '{other}' (expected k6x or kl)"),
        },
        "nxp_rsim" => Box::new(crate::peripherals::rsim::Rsim::new()),
        "kinetis_rnga" => Box::new(crate::peripherals::kinetis_rnga::KinetisRnga::new()),
        "ft9001_cpm" => Box::new(crate::peripherals::ft9001_sys::Ft9001Cpm::new()),
        "ft9001_reset" => Box::new(crate::peripherals::ft9001_sys::Ft9001Reset::new()),
        "ft9001_trng" => Box::new(crate::peripherals::ft9001_sys::Ft9001Trng::new()),
        "eos_s3_intr_ctrl" => Box::new(crate::peripherals::eos_s3_intr::EosS3IntrCtrl::new()),
        "eos_s3_spt" => Box::new(crate::peripherals::eos_s3_spt::EosS3Spt::new(
            p_cfg.config.get("clock_hz").and_then(|v| v.as_u64()),
        )),
        "sam_pm" => Box::new(crate::peripherals::sam_clock::SamPm::new()),
        "sam_gclk" => Box::new(crate::peripherals::sam_clock::SamGclk::new()),
        "sam_mclk" => Box::new(crate::peripherals::sam_clock::SamMclk::new()),
        "ra_sysc" => Box::new(crate::peripherals::ra_clock::RaSysc::new()),
        "ra8_sysc" => Box::new(crate::peripherals::ra_clock::RaSyscRegs::ra8()),
        "ra6_sysc" => Box::new(crate::peripherals::ra_clock::RaSyscRegs::ra6()),
        "ra4_sysc" => Box::new(crate::peripherals::ra_clock::RaSyscRegs::ra4()),
        "ra_mstp" => Box::new(crate::peripherals::ra_mstp::RaMstp::new()),
        "rx_sysc" => Box::new(crate::peripherals::rx_sysc::RxSysc::new()),
        // `irq`: the EVIC vector; `timer_type`: "a" (Timer1) or "b";
        // `clock_hz`: SYSCLK; `pbclk_hz`: the timer's PBCLK3; `sosc_hz`.
        "pic32_timer" => {
            use crate::peripherals::pic32_timer as t;
            let get = |k: &str| p_cfg.config.get(k).and_then(|v| v.as_u64());
            let irq = get("irq").ok_or_else(|| {
                anyhow::anyhow!("pic32_timer '{}' needs `irq` (EVIC vector)", p_cfg.id)
            })? as u32;
            let type_a = match p_cfg.config.get("timer_type").and_then(|v| v.as_str()) {
                Some("a") | Some("A") => true,
                Some("b") | Some("B") => false,
                other => anyhow::bail!(
                    "pic32_timer '{}' needs `timer_type: a|b`, got {other:?}",
                    p_cfg.id
                ),
            };
            Box::new(t::Pic32Timer::new(
                get("clock_hz").unwrap_or(t::DEFAULT_CLOCK_HZ),
                get("pbclk_hz").unwrap_or(t::DEFAULT_PBCLK_HZ),
                get("sosc_hz").unwrap_or(t::DEFAULT_SOSC_HZ),
                type_a,
                irq,
            ))
        }
        // `irqs`: the EVIC vectors [fault, receive, transmit]; `clock_hz`:
        // SYSCLK; `pbclk_hz`: the UART's PBCLK2 (baud and RX polling).
        "pic32_uart" => {
            let irqs = p_cfg
                .config
                .get("irqs")
                .and_then(|v| v.as_sequence())
                .map(|s| {
                    s.iter()
                        .filter_map(|v| v.as_u64())
                        .map(|v| v as u32)
                        .collect::<Vec<_>>()
                })
                .filter(|v| v.len() == 3)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "pic32_uart '{}' needs `irqs: [fault, rx, tx]` (EVIC vectors)",
                        p_cfg.id
                    )
                })?;
            let get = |k: &str| p_cfg.config.get(k).and_then(|v| v.as_u64());
            use crate::peripherals::pic32_uart as u;
            Box::new(u::Pic32Uart::with_clocks(
                [irqs[0], irqs[1], irqs[2]],
                get("clock_hz").unwrap_or(u::DEFAULT_CLOCK_HZ),
                get("pbclk_hz").unwrap_or(u::DEFAULT_PBCLK_HZ),
            ))
        }
        // `irqs`: the part's vector numbers [rxc, dre, txc].
        "megaavr_usart" => {
            let irqs = u32_list(p_cfg, "irqs", 3).ok_or_else(|| {
                anyhow::anyhow!(
                    "megaavr_usart '{}' needs `irqs: [rxc, dre, txc]` (vector numbers)",
                    p_cfg.id
                )
            })?;
            Box::new(crate::peripherals::megaavr::usart::MegaAvrUsart::new([
                irqs[0], irqs[1], irqs[2],
            ]))
        }
        // `tcb_irqs`: the TCB0..3 vector numbers.
        "megaavr_tc" => {
            let irqs = u32_list(p_cfg, "tcb_irqs", 4).ok_or_else(|| {
                anyhow::anyhow!(
                    "megaavr_tc '{}' needs `tcb_irqs: [tcb0, tcb1, tcb2, tcb3]` (vector numbers)",
                    p_cfg.id
                )
            })?;
            Box::new(crate::peripherals::megaavr::tc::MegaAvrTimers::new([
                irqs[0], irqs[1], irqs[2], irqs[3],
            ]))
        }
        // `ie1_lines` / `ie2_lines` / `uc1_lines`: the interrupt line behind
        // each bit (0 to 7) of IE1/IFG1, IE2/IFG2 and UC1IE/UC1IFG; 0 = not
        // an interrupt bit. `uc1_lines` is optional.
        "msp430_sfr" => {
            msp430_keys(p_cfg, &["ie1_lines", "ie2_lines", "uc1_lines"])?;
            let lines = |key: &str, required: bool| -> anyhow::Result<[u32; 8]> {
                match u32_list(p_cfg, key, 8) {
                    Some(v) => Ok(std::array::from_fn(|i| v[i])),
                    None if !required && !p_cfg.config.contains_key(key) => Ok([0; 8]),
                    None => anyhow::bail!(
                        "msp430_sfr '{}' needs `{key}`: eight line numbers",
                        p_cfg.id
                    ),
                }
            };
            Box::new(crate::peripherals::msp430::Msp430Sfr::new([
                lines("ie1_lines", true)?,
                lines("ie2_lines", true)?,
                lines("uc1_lines", false)?,
            ]))
        }
        // `flags`: [IFG register (1 or 2), URXIFG bit, UTXIFG bit];
        // `module_enable`: [ME register (1 or 2), URXE bit, UTXE bit].
        "msp430_usart" => {
            msp430_keys(p_cfg, &["flags", "module_enable"])?;
            let triple = |key: &str| -> anyhow::Result<[u32; 3]> {
                let v = u32_list(p_cfg, key, 3).ok_or_else(|| {
                    anyhow::anyhow!(
                        "msp430_usart '{}' needs `{key}: [register, rx_bit, tx_bit]`",
                        p_cfg.id
                    )
                })?;
                Ok([v[0], v[1], v[2]])
            };
            Box::new(crate::peripherals::msp430::Msp430Usart::new(
                triple("flags")?,
                triple("module_enable")?,
            ))
        }
        // `flags`: [flag register (2 = IFG2, 3 = UC1IFG), UCAxRXIFG bit,
        // UCAxTXIFG bit].
        "msp430_usci_a" => {
            msp430_keys(p_cfg, &["flags"])?;
            let v = u32_list(p_cfg, "flags", 3).ok_or_else(|| {
                anyhow::anyhow!(
                    "msp430_usci_a '{}' needs `flags: [register, rx_bit, tx_bit]`",
                    p_cfg.id
                )
            })?;
            Box::new(crate::peripherals::msp430::Msp430Usart::usci_a([
                v[0], v[1], v[2],
            ]))
        }
        "msp430_wdt" => {
            msp430_keys(p_cfg, &[])?;
            Box::new(crate::peripherals::msp430::Msp430Wdt::new())
        }
        "msp430_mpy" => {
            msp430_keys(p_cfg, &[])?;
            Box::new(crate::peripherals::msp430::Msp430Mpy::new())
        }
        "msp430_bcm" => {
            msp430_keys(p_cfg, &[])?;
            Box::new(crate::peripherals::msp430::Msp430Bcm::new())
        }
        "msp430_bcm_plus" => {
            msp430_keys(p_cfg, &[])?;
            Box::new(crate::peripherals::msp430::Msp430Bcm::plus())
        }
        "msp430_bcsctl3" => {
            msp430_keys(p_cfg, &[])?;
            Box::new(crate::peripherals::msp430::Msp430Bcm::bcsctl3())
        }
        "msp430_port" => {
            use crate::peripherals::msp430_port::{Msp430Port, PortRequest};
            msp430_keys(p_cfg, &["interrupt", "sel_reset", "irq_source", "irq_line"])?;
            let byte = |key: &str| -> anyhow::Result<Option<u32>> {
                match p_cfg.config.get(key) {
                    None => Ok(None),
                    Some(v) => v.as_u64().map(|v| Some(v as u32)).ok_or_else(|| {
                        anyhow::anyhow!("msp430_port '{}': `{key}` is not a number", p_cfg.id)
                    }),
                }
            };
            let sel_reset = byte("sel_reset")?.unwrap_or(0);
            if sel_reset > 0xFF {
                anyhow::bail!("msp430_port '{}': `sel_reset` is one byte", p_cfg.id);
            }
            let interrupt = match p_cfg.config.get("interrupt") {
                None => false,
                Some(v) => v.as_bool().ok_or_else(|| {
                    anyhow::anyhow!("msp430_port '{}': `interrupt` is true or false", p_cfg.id)
                })?,
            };
            let request = match (byte("irq_source")?, byte("irq_line")?) {
                (None, None) => None,
                (Some(source), Some(line))
                    if interrupt && source < crate::peripherals::msp430::SOURCES && line < 64 =>
                {
                    Some(PortRequest { source, line })
                }
                _ => anyhow::bail!(
                    "msp430_port '{}': `irq_source` (below {}) and `irq_line` (below 64) come \
                     together, on a port with `interrupt: true`",
                    p_cfg.id,
                    crate::peripherals::msp430::SOURCES
                ),
            };
            if interrupt {
                Box::new(Msp430Port::with_interrupt(sel_reset as u8, request))
            } else {
                Box::new(Msp430Port::basic(sel_reset as u8))
            }
        }
        "msp430_bytes" => {
            msp430_keys(p_cfg, &["reset"])?;
            let reset = p_cfg
                .config
                .get("reset")
                .and_then(|v| v.as_sequence())
                .and_then(|s| {
                    s.iter()
                        .map(|v| v.as_u64().and_then(|v| u8::try_from(v).ok()))
                        .collect::<Option<Vec<u8>>>()
                })
                .filter(|v| !v.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "msp430_bytes '{}' needs `reset`: one byte value per register",
                        p_cfg.id
                    )
                })?;
            Box::new(crate::peripherals::msp430_port::Msp430Bytes::new(reset))
        }
        // `irq_sources`: USIIFG, then USISTTIFG; `irq_line`: the USI vector;
        // `aclk_hz` (default 32768) and `cpu_hz` clock ACLK.
        "msp430_usi" => {
            use crate::peripherals::msp430_usi::{Msp430Usi, UsiConfig, UsiRequest};
            msp430_keys(p_cfg, &["irq_sources", "irq_line", "aclk_hz", "cpu_hz"])?;
            let number = |key: &str| -> anyhow::Result<Option<u64>> {
                match p_cfg.config.get(key) {
                    None => Ok(None),
                    Some(v) => v.as_u64().map(Some).ok_or_else(|| {
                        anyhow::anyhow!("msp430_usi '{}': `{key}` is not a number", p_cfg.id)
                    }),
                }
            };
            let request = match (p_cfg.config.get("irq_sources"), number("irq_line")?) {
                (None, None) => None,
                (Some(_), Some(line)) if line < 64 => {
                    let sources = u32_list(p_cfg, "irq_sources", 2)
                        .filter(|v| v.iter().all(|s| *s < crate::peripherals::msp430::SOURCES))
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "msp430_usi '{}': `irq_sources` is USIIFG, then USISTTIFG \
                                 (two sources below {})",
                                p_cfg.id,
                                crate::peripherals::msp430::SOURCES
                            )
                        })?;
                    Some(UsiRequest {
                        counter_source: sources[0],
                        start_source: sources[1],
                        line: line as u32,
                    })
                }
                _ => anyhow::bail!(
                    "msp430_usi '{}': `irq_sources` and `irq_line` (below 64) come together",
                    p_cfg.id
                ),
            };
            Box::new(Msp430Usi::new(
                request,
                UsiConfig {
                    aclk_hz: number("aclk_hz")?.unwrap_or(32_768),
                    cpu_hz: number("cpu_hz")?.unwrap_or(0),
                },
            ))
        }
        // `blocks`: capture/compare blocks (default 3); `irq_sources`:
        // CCIFG0 ... CCIFGn-1, then TAIFG; `irq_lines`: the TACCR0 line, then
        // the TAIV line; `aclk_hz` (default 32768) and `cpu_hz` clock ACLK.
        "msp430_timer_a" => {
            use crate::peripherals::msp430_timer_a::{Msp430TimerA, TimerAClocks, TimerARequest};
            msp430_keys(
                p_cfg,
                &["blocks", "irq_sources", "irq_lines", "aclk_hz", "cpu_hz"],
            )?;
            let number = |key: &str, default: u64| -> anyhow::Result<u64> {
                match p_cfg.config.get(key) {
                    None => Ok(default),
                    Some(v) => v.as_u64().ok_or_else(|| {
                        anyhow::anyhow!("msp430_timer_a '{}': `{key}` is not a number", p_cfg.id)
                    }),
                }
            };
            let blocks = number("blocks", 3)? as usize;
            if !(2..=7).contains(&blocks) {
                anyhow::bail!("msp430_timer_a '{}': `blocks` is 2 to 7", p_cfg.id);
            }
            let request = match (
                p_cfg.config.get("irq_sources"),
                p_cfg.config.get("irq_lines"),
            ) {
                (None, None) => None,
                _ => {
                    let sources = u32_list(p_cfg, "irq_sources", blocks + 1)
                        .filter(|v| v.iter().all(|s| *s < crate::peripherals::msp430::SOURCES));
                    let lines =
                        u32_list(p_cfg, "irq_lines", 2).filter(|v| v.iter().all(|l| *l < 64));
                    match (sources, lines) {
                        (Some(sources), Some(lines)) => Some(TimerARequest {
                            sources,
                            lines: [lines[0], lines[1]],
                        }),
                        _ => anyhow::bail!(
                            "msp430_timer_a '{}': `irq_sources` ({} sources below {}) and \
                             `irq_lines` (two lines below 64) come together",
                            p_cfg.id,
                            blocks + 1,
                            crate::peripherals::msp430::SOURCES
                        ),
                    }
                }
            };
            let clocks = TimerAClocks {
                aclk_hz: number("aclk_hz", 32_768)?,
                cpu_hz: number("cpu_hz", 0)?,
            };
            Box::new(Msp430TimerA::new(blocks, request, clocks))
        }
        // `irq_sources`: CCIFG1 ... CCIFGn-1, then TAIFG, as the timer names
        // them.
        "msp430_timer_a_iv" => {
            msp430_keys(p_cfg, &["irq_sources"])?;
            let sources = p_cfg
                .config
                .get("irq_sources")
                .and_then(|v| v.as_sequence())
                .and_then(|s| {
                    s.iter()
                        .map(|v| {
                            v.as_u64()
                                .map(|v| v as u32)
                                .filter(|v| *v < crate::peripherals::msp430::SOURCES)
                        })
                        .collect::<Option<Vec<u32>>>()
                })
                .filter(|v| (2..=7).contains(&v.len()))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "msp430_timer_a_iv '{}' needs `irq_sources`: CCIFG1 ... CCIFGn-1, then TAIFG",
                        p_cfg.id
                    )
                })?;
            Box::new(crate::peripherals::msp430_timer_a::Msp430TimerAIv::new(
                sources,
            ))
        }
        // `inputs`: bonded external inputs A0 .. A(n-1); `vcc_mv`: supply.
        "msp430_adc10" => {
            use crate::peripherals::msp430_adc10::{
                Adc10Config, Adc10Request, Msp430Adc10, EXTERNAL_INPUTS,
            };
            msp430_keys(
                p_cfg,
                &[
                    "inputs",
                    "vcc_mv",
                    "adc10osc_hz",
                    "aclk_hz",
                    "cpu_hz",
                    "irq_source",
                    "irq_line",
                ],
            )?;
            let number = |key: &str| -> anyhow::Result<Option<u64>> {
                match p_cfg.config.get(key) {
                    None => Ok(None),
                    Some(v) => v.as_u64().map(Some).ok_or_else(|| {
                        anyhow::anyhow!("msp430_adc10 '{}': `{key}` is not a number", p_cfg.id)
                    }),
                }
            };
            let inputs = number("inputs")?.unwrap_or(u64::from(EXTERNAL_INPUTS));
            if inputs > u64::from(EXTERNAL_INPUTS) {
                anyhow::bail!(
                    "msp430_adc10 '{}': `inputs` is 0 to {EXTERNAL_INPUTS}",
                    p_cfg.id
                );
            }
            let vcc_mv = number("vcc_mv")?.unwrap_or(3000);
            if !(1..=u64::from(u16::MAX)).contains(&vcc_mv) {
                anyhow::bail!("msp430_adc10 '{}': `vcc_mv` is 1 to 65535", p_cfg.id);
            }
            let request = match (number("irq_source")?, number("irq_line")?) {
                (None, None) => None,
                (Some(source), Some(line))
                    if source < u64::from(crate::peripherals::msp430::SOURCES) && line < 64 =>
                {
                    Some(Adc10Request {
                        source: source as u32,
                        line: line as u32,
                    })
                }
                _ => anyhow::bail!(
                    "msp430_adc10 '{}': `irq_source` (below {}) and `irq_line` (below 64) come \
                     together",
                    p_cfg.id,
                    crate::peripherals::msp430::SOURCES
                ),
            };
            Box::new(Msp430Adc10::new(
                request,
                Adc10Config {
                    inputs: inputs as u8,
                    vcc_mv: vcc_mv as u32,
                    // ADC10OSC typical (3.7 to 6.3 MHz in the x2xx data sheets).
                    adc10osc_hz: number("adc10osc_hz")?.unwrap_or(5_000_000),
                    aclk_hz: number("aclk_hz")?.unwrap_or(32_768),
                    cpu_hz: number("cpu_hz")?.unwrap_or(0),
                },
            ))
        }
        // `lines`: the interrupt line behind each bit (0 to 7) of
        // SFRIE1/SFRIFG1; 0 = the bit requests no interrupt.
        "msp430_sfr_x5" => {
            msp430_keys(p_cfg, &["lines"])?;
            let v = u32_list(p_cfg, "lines", 8).ok_or_else(|| {
                anyhow::anyhow!(
                    "msp430_sfr_x5 '{}' needs `lines`: eight line numbers",
                    p_cfg.id
                )
            })?;
            Box::new(crate::peripherals::msp430::Msp430SfrX5::new(
                std::array::from_fn(|i| v[i]),
            ))
        }
        // `unit`: the number of this USCI module on the chip (it selects the
        // interrupt sources, so each module needs its own); `line`: the
        // request line UCRXIFG and UCTXIFG share.
        "msp430_usci_a_x5" => {
            msp430_keys(p_cfg, &["unit", "line"])?;
            let number = |key: &str, limit: u64| -> anyhow::Result<u32> {
                p_cfg
                    .config
                    .get(key)
                    .and_then(|v| v.as_u64())
                    .filter(|v| *v < limit)
                    .map(|v| v as u32)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "msp430_usci_a_x5 '{}' needs `{key}`: a number below {limit}",
                            p_cfg.id
                        )
                    })
            };
            Box::new(crate::peripherals::msp430::Msp430Usart::usci_a_x5(
                number("unit", u64::from(crate::peripherals::msp430::X5_USCI_UNITS))?,
                number("line", 64)?,
            ))
        }
        "msp430_wdt_a" => {
            msp430_keys(p_cfg, &[])?;
            Box::new(crate::peripherals::msp430::Msp430Wdt::wdt_a())
        }
        "msp430_mpy32" => {
            msp430_keys(p_cfg, &[])?;
            Box::new(crate::peripherals::msp430::Msp430Mpy32::new())
        }
        "msp430_ucs" => {
            msp430_keys(p_cfg, &[])?;
            Box::new(crate::peripherals::msp430::Msp430Ucs::new())
        }
        "msp430_pmm" => {
            msp430_keys(p_cfg, &[])?;
            Box::new(crate::peripherals::msp430::Msp430Pmm::new())
        }
        // `vdd_mv`: the board's VDD (the VDDREF reference, default 5000);
        // `input_mv: { <ain>: <mV> }`: static levels on AIN0..AIN15.
        "megaavr_adc" => {
            use crate::peripherals::megaavr::adc::MegaAvrAdc;
            if let Some(k) = p_cfg
                .config
                .keys()
                .find(|k| !matches!(k.as_str(), "vdd_mv" | "input_mv"))
            {
                anyhow::bail!("megaavr_adc '{}' admits no config key '{k}'", p_cfg.id);
            }
            if p_cfg.irq.is_some() {
                anyhow::bail!(
                    "megaavr_adc '{}': ADC0 interrupts are not modelled",
                    p_cfg.id
                );
            }
            let vdd_mv = p_cfg
                .config
                .get("vdd_mv")
                .map_or(Some(5000), |v| v.as_u64());
            let Some(vdd_mv @ 1..) = vdd_mv else {
                anyhow::bail!(
                    "megaavr_adc '{}': vdd_mv wants a positive millivolt count",
                    p_cfg.id
                );
            };
            let mut adc = MegaAvrAdc::new(vdd_mv as u32);
            if let Some(map) = p_cfg.config.get("input_mv").and_then(|v| v.as_mapping()) {
                for (k, v) in map {
                    match (k.as_u64(), v.as_u64()) {
                        (Some(ain), Some(mv)) if ain < 16 => {
                            adc.set_input_millivolts(ain as usize, mv as u32)
                        }
                        _ => anyhow::bail!("megaavr_adc input_mv wants {{ <ain 0..15>: <mV> }}"),
                    }
                }
            }
            Box::new(adc)
        }
        // `irq`: the part's TWIM (host) vector number.
        "megaavr_twi" => {
            if let Some(k) = p_cfg.config.keys().find(|k| k.as_str() != "irq") {
                anyhow::bail!("megaavr_twi '{}' admits no config key '{k}'", p_cfg.id);
            }
            let irq = p_cfg
                .config
                .get("irq")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "megaavr_twi '{}' needs `irq: <TWIM vector number>`",
                        p_cfg.id
                    )
                })?;
            Box::new(crate::peripherals::megaavr::twi::MegaAvrTwi::new(
                irq as u32,
            ))
        }
        // `irq`: the part's SPI vector number.
        "megaavr_spi" => {
            if let Some(k) = p_cfg.config.keys().find(|k| k.as_str() != "irq") {
                anyhow::bail!("megaavr_spi '{}' admits no config key '{k}'", p_cfg.id);
            }
            let irq = p_cfg
                .config
                .get("irq")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "megaavr_spi '{}' needs `irq: <SPI vector number>`",
                        p_cfg.id
                    )
                })?;
            Box::new(crate::peripherals::megaavr::spi::MegaAvrSpi::new(
                irq as u32,
            ))
        }
        "rx_flash" => Box::new(crate::peripherals::rx_sysc::RxFlashRegs::new()),
        "rx_rtc" => Box::new(crate::peripherals::rx_sysc::RxFlashRegs::rtc()),
        // `vectors`: the two channels' CMI vectors; `clock_hz`: the CPU
        // cycle rate; `pclkb_hz`: the CMT count source before CKS.
        "rx_cmt" => {
            use crate::peripherals::rx_cmt as cmt;
            let get = |k: &str| p_cfg.config.get(k).and_then(|v| v.as_u64());
            let vectors = p_cfg
                .config
                .get("vectors")
                .and_then(|v| v.as_sequence())
                .map(|s| {
                    s.iter()
                        .filter_map(|v| v.as_u64())
                        .map(|v| v as u32)
                        .collect::<Vec<_>>()
                })
                .filter(|v| v.len() == 2)
                .map_or([28, 29], |v| [v[0], v[1]]);
            Box::new(cmt::RxCmt::new(
                get("clock_hz").unwrap_or(cmt::DEFAULT_CLOCK_HZ),
                get("pclkb_hz").unwrap_or(cmt::DEFAULT_PCLKB_HZ),
                vectors,
            ))
        }
        "ra_pfs" => Box::new(crate::peripherals::ra_pfs::RaPfs::new()),
        "ra8_pfs" => Box::new(crate::peripherals::ra_pfs::RaPfs::new_ra8()),
        // `ielsr_lines`: IELSRn count (default 96; 0 = only the IRQ-pin half);
        // `irq_pins`: `[port, pin, IRQn]` per IRQ pad; `pfs_base`: PmnPFS base.
        "ra_icu" => {
            use crate::peripherals::ra_icu as icu;
            let lines = p_cfg
                .config
                .get("ielsr_lines")
                .and_then(|v| v.as_u64())
                .map_or(icu::LINES, |v| v as u32);
            let irq_pins = p_cfg
                .config
                .get("irq_pins")
                .and_then(|v| v.as_sequence())
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| {
                            let row = row.as_sequence()?;
                            let get = |i: usize| row.get(i)?.as_u64().map(|v| v as u8);
                            (row.len() == 3).then_some((get(0)?, get(1)?, get(2)?))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let pfs_base = p_cfg.config.get("pfs_base").and_then(|v| v.as_u64());
            Box::new(icu::RaIcu::with_irq_pins(lines, irq_pins, pfs_base))
        }
        // `clock_hz`: the rate the bus cycles run at; `pclkb_hz`: PCLKB for
        // the PCLKB count sources; `elc_event`: the channel's INT event
        // (compare A/B are +1/+2).
        "ra_agt" => {
            use crate::peripherals::ra_agt as agt;
            let get = |k: &str| p_cfg.config.get(k).and_then(|v| v.as_u64());
            Box::new(agt::RaAgt::new(
                get("clock_hz").unwrap_or(agt::DEFAULT_CLOCK_HZ),
                get("pclkb_hz").unwrap_or(agt::DEFAULT_PCLKB_HZ),
                get("elc_event").map_or(agt::DEFAULT_ELC_EVENT, |v| v as u32),
            ))
        }
        // `clock_hz`: the rate the bus cycles run at; `elc_event`: the unit's
        // scan-end event; `vref_mv`: the reference; `adc14`: a 14-bit ADC14
        // unit (ADPRC = 3 is 14-bit); `input_mv: { <ch>: <mV> }`:
        // board-level static channel levels.
        // `clock_hz`: the rate the bus cycles run at; `pclkb_hz`: PCLKB, the
        // RIIC's clock; `elc_event`: the channel's RXI event (TXI/TEI/ERI
        // are +1/+2/+3).
        "ra_iic" => {
            use crate::peripherals::ra_iic as iic;
            let get = |k: &str| p_cfg.config.get(k).and_then(|v| v.as_u64());
            Box::new(iic::RaIic::new(
                get("clock_hz").unwrap_or(iic::DEFAULT_CLOCK_HZ),
                get("pclkb_hz").unwrap_or(iic::DEFAULT_PCLKB_HZ),
                get("elc_event").map_or(iic::DEFAULT_ELC_EVENT, |v| v as u32),
            ))
        }
        // Normal mailbox mode; interrupts are refused, so there is no `irq`.
        "ra_can" => {
            if p_cfg.irq.is_some() {
                anyhow::bail!("ra_can '{}' models no interrupt (drop `irq`)", p_cfg.id);
            }
            if let Some(k) = p_cfg.config.keys().next() {
                anyhow::bail!("ra_can '{}' admits no config key '{k}'", p_cfg.id);
            }
            Box::new(crate::peripherals::ra_can::RaCan::new())
        }
        // `channels`: GPT channel count; `clock_hz`: the rate the bus cycles
        // run at; `pclkd_hz`: PCLKD, the count clock before GTCR.TPCS;
        // `elc_event`: GPT0's capture/compare A event; `events_per_channel`:
        // the ELC event stride (default nine; eight on the RA4M1);
        // `wide_channels`: mask of the 32-bit channels; `gtioc_pads`:
        // `{ GTIOCnA: Pmnn, ... }`, the pads that carry each GTIOC line.
        "ra_gpt" => {
            use crate::peripherals::ra_gpt as gpt;
            let get = |k: &str| p_cfg.config.get(k).and_then(|v| v.as_u64());
            let mut pads = Vec::new();
            if let Some(map) = p_cfg.config.get("gtioc_pads").and_then(|v| v.as_mapping()) {
                for (signal, pad) in map {
                    let (Some(signal), Some(pad)) = (signal.as_str(), pad.as_str()) else {
                        continue;
                    };
                    match gpt::RaGpt::parse_pad(signal, pad) {
                        Some(entry) => pads.push(entry),
                        None => tracing::warn!(
                            "ra_gpt '{}': gtioc_pads entry {signal}: {pad} not understood",
                            p_cfg.id
                        ),
                    }
                }
            }
            Box::new(
                gpt::RaGpt::new(
                    get("channels").map_or(gpt::MAX_CHANNELS, |v| v as usize),
                    get("clock_hz").unwrap_or(gpt::DEFAULT_CLOCK_HZ),
                    get("pclkd_hz").unwrap_or(gpt::DEFAULT_PCLKD_HZ),
                    get("elc_event").map_or(gpt::DEFAULT_ELC_EVENT, |v| v as u32),
                    get("wide_channels").map_or(0xFF, |v| v as u32),
                )
                .with_events_per_channel(
                    get("events_per_channel").map_or(gpt::DEFAULT_EVENTS_PER_CHANNEL, |v| v as u32),
                )
                .with_output_pads(pads),
            )
        }
        "ra_adc12" => {
            use crate::peripherals::ra_adc12 as adc;
            let get = |k: &str| p_cfg.config.get(k).and_then(|v| v.as_u64());
            let mut unit = adc::RaAdc12::new(
                get("clock_hz").unwrap_or(adc::DEFAULT_CLOCK_HZ),
                get("elc_event").map_or(adc::DEFAULT_ELC_EVENT, |v| v as u32),
                get("vref_mv").map_or(adc::DEFAULT_VREF_MV, |v| v as u16),
            )
            .with_adc14(
                p_cfg
                    .config
                    .get("adc14")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            );
            if let Some(map) = p_cfg.config.get("input_mv").and_then(|v| v.as_mapping()) {
                for (k, v) in map {
                    if let (Some(ch), Some(mv)) = (k.as_u64(), v.as_u64()) {
                        unit.set_channel_input(ch as u8, mv as u16);
                    }
                }
            }
            // Board wires from an analog output: `analog_links: { <ch>:
            // "<peripheral>:<output>" }`, bound by the bus after build.
            if let Some(map) = p_cfg
                .config
                .get("analog_links")
                .and_then(|v| v.as_mapping())
            {
                for (k, v) in map {
                    let link = v.as_str().and_then(|s| s.rsplit_once(':'));
                    match (
                        k.as_u64(),
                        link.and_then(|(p, o)| Some((p, o.parse::<u8>().ok()?))),
                    ) {
                        (Some(ch), Some((source, output))) => {
                            unit.add_analog_link(ch as u8, source.to_string(), output)
                        }
                        _ => tracing::warn!(
                            "ra_adc12 '{}': analog_links entry {k:?}: {v:?} not understood \
                             (expected <channel>: \"<peripheral>:<output>\")",
                            p_cfg.id
                        ),
                    }
                }
            }
            Box::new(unit)
        }
        "ra_dac12" => {
            use crate::peripherals::ra_dac12 as dac;
            let vref = p_cfg.config.get("vref_mv").and_then(|v| v.as_u64());
            Box::new(dac::RaDac12::new(
                vref.map_or(dac::DEFAULT_VREF_MV, |v| v as u16),
            ))
        }
        "imx_ccm" => Box::new(crate::peripherals::imx_ccm::ImxCcm::new()),
        "imx_iomuxc" => Box::new(crate::peripherals::imx_iomuxc::ImxIomuxc::new()),
        "imxrt_ccm" => Box::new(crate::peripherals::imxrt::ccm::ImxrtCcm::new()),
        "imxrt_anadig" => Box::new(crate::peripherals::imxrt::anadig::ImxrtAnadig::new()),
        "imxrt_dcdc" => Box::new(crate::peripherals::imxrt::dcdc::ImxrtDcdc::new()),
        "imxrt_adc" => {
            let mut adc = crate::peripherals::imxrt::adc::ImxrtAdc::new();
            // Board-level static channel voltages: `input_mv: { <ch>: <mV> }`.
            if let Some(map) = p_cfg.config.get("input_mv").and_then(|v| v.as_mapping()) {
                for (k, v) in map {
                    if let (Some(ch), Some(mv)) = (k.as_u64(), v.as_u64()) {
                        crate::Peripheral::set_adc_channel_input(&mut adc, ch as u8, mv as u16);
                    }
                }
            }
            Box::new(adc)
        }
        "imxrt_usb" => {
            let mut usb = crate::peripherals::imxrt::usb::ImxrtUsb::new();
            // `host: false` leaves the port unplugged (no enumeration).
            if p_cfg.config.get("host").and_then(|v| v.as_bool()) == Some(false) {
                usb.set_host_enabled(false);
            }
            // `host_steps:` run after enumeration (usb_host::parse_host_steps),
            // e.g. the CDC SET_LINE_CODING / SET_CONTROL_LINE_STATE a terminal
            // sends when it opens the port.
            if let Some(steps) = p_cfg.config.get("host_steps") {
                let steps = crate::peripherals::usb_host::parse_host_steps(steps)
                    .map_err(|e| anyhow::anyhow!("imxrt_usb host_steps: {e}"))?;
                usb.queue_host_steps(steps);
            }
            Box::new(usb)
        }
        "imxrt_edma" => Box::new(crate::peripherals::imxrt::edma::ImxrtEdma::new()),
        "imxrt_gpt" => {
            let d = crate::peripherals::imxrt::gpt::GptClocks::default();
            let get = |k: &str, v: u64| p_cfg.config.get(k).and_then(|x| x.as_u64()).unwrap_or(v);
            Box::new(crate::peripherals::imxrt::gpt::ImxrtGpt::new(
                crate::peripherals::imxrt::gpt::GptClocks {
                    ipg_hz: get("ipg_hz", d.ipg_hz),
                    perclk_hz: get("perclk_hz", d.perclk_hz),
                    osc_hz: get("osc_hz", d.osc_hz),
                    low_hz: get("low_hz", d.low_hz),
                },
            ))
        }
        "imxrt_flexio" => {
            let clk = p_cfg
                .config
                .get("clock_hz")
                .and_then(|v| v.as_u64())
                .unwrap_or(crate::peripherals::imxrt::flexio::DEFAULT_FLEXIO_CLK_HZ);
            Box::new(crate::peripherals::imxrt::flexio::ImxrtFlexio::new(clk))
        }
        "imxrt_sai" => {
            let mclk = p_cfg
                .config
                .get("mclk_hz")
                .and_then(|v| v.as_u64())
                .unwrap_or(crate::peripherals::imxrt::sai::DEFAULT_MCLK_HZ);
            let mut sai = crate::peripherals::imxrt::sai::ImxrtSai::new(mclk);
            if let Some(path) = p_cfg.config.get("rx_wav").and_then(|v| v.as_str()) {
                let gain = p_cfg
                    .config
                    .get("rx_wav_gain")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0);
                sai = sai
                    .with_rx_wav(std::path::Path::new(path), gain)
                    .map_err(|e| anyhow::anyhow!("imxrt_sai rx_wav: {e}"))?;
            }
            if let Some(path) = p_cfg.config.get("tx_wav").and_then(|v| v.as_str()) {
                sai = sai.with_tx_wav(std::path::Path::new(path));
            }
            match p_cfg.config.get("tx_irq").and_then(|v| v.as_u64()) {
                Some(line) => Box::new(sai.with_tx_irq(line as u32)),
                None => Box::new(sai),
            }
        }
        "imxrt_usbphy" => Box::new(crate::peripherals::imxrt::usb::ImxrtUsbPhy::default()),
        "imxrt_flexspi" => {
            // The serial NOR on port A1: its JEDEC ID and the AHB window its
            // array is mapped at (the chip's XIP flash region).
            let get = |k: &str, d: u64| p_cfg.config.get(k).and_then(|v| v.as_u64()).unwrap_or(d);
            let id = get("jedec_id", 0xEF_4018) as u32;
            Box::new(crate::peripherals::imxrt::flexspi::ImxrtFlexspi::new(
                [(id >> 16) as u8, (id >> 8) as u8, id as u8],
                get("xip_base", 0x6000_0000),
                get("xip_size", 16 << 20) as usize,
            ))
        }
        "imxrt_lpuart" => {
            // LPUART functional clock (after CCM UART_CLK_SEL/PODF).
            let clk = p_cfg
                .config
                .get("clock_hz")
                .and_then(|v| v.as_u64())
                .unwrap_or(crate::peripherals::imxrt::lpuart::DEFAULT_UART_CLK_HZ);
            let uart = crate::peripherals::imxrt::lpuart::ImxrtLpuart::new(clk);
            // i.MX RT700 LP_FLEXCOMM: `lp_flexcomm: true` adds PSELID/ISTAT;
            // `lp_flexcomm_present` overrides the present bits [6:4].
            let lp_flexcomm = p_cfg
                .config
                .get("lp_flexcomm")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if lp_flexcomm {
                let present = p_cfg
                    .config
                    .get("lp_flexcomm_present")
                    .and_then(|v| v.as_u64())
                    .map_or(crate::peripherals::imxrt::lpuart::LPFC_PRESENT_ALL, |v| {
                        v as u32
                    });
                Box::new(uart.with_lp_flexcomm(present))
            } else {
                Box::new(uart)
            }
        }
        // LiteX countdown timer (`litex/soc/cores/timer.py`): 32-bit CSRs
        // load/reload/en/update_value/value plus the zero-event manager.
        // `csr_data_width: 8` selects the one-byte-per-slot layout (Fomu).
        "litex_timer" => {
            let timer = crate::peripherals::litex_timer::LitexTimer::new();
            match p_cfg.config.get("csr_data_width").and_then(|v| v.as_u64()) {
                Some(8) => Box::new(timer.with_csr8()),
                _ => Box::new(timer),
            }
        }
        "litex_controlandstatus" => {
            let active_low = p_cfg
                .config
                .get("buttons_active_low")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Box::new(
                crate::peripherals::litex_control_status::LitexControlStatus::new()
                    .with_buttons_active_low(active_low),
            )
        }
        // LiteX SPI-flash bitbang CSRs + Micron MT25Q ID/status responder
        // (Renode `LiteX_SPI_Flash` + `Micron_MT25Q`).
        "litex_spi_flash" => Box::new(crate::peripherals::litex_spi_flash::LitexSpiFlash::new()),
        // Microchip Mi-V reference-design peripherals.
        "miv_coregpio" => Box::new(crate::peripherals::miv::MivCoreGpio::new()),
        "miv_coretimer" => Box::new(crate::peripherals::miv::MivCoreTimer::new()),
        // PolarFire SoC MSS GPIO_0..2: `ports` is the bank width (14, 24
        // or 32, `NB_OF_GPIO_GPIO*` in Microchip's mss_gpio.c).
        "mpfs_gpio" => {
            let ports = p_cfg
                .config
                .get("ports")
                .and_then(|v| v.as_u64())
                .unwrap_or(32);
            if !(1..=32).contains(&ports) {
                anyhow::bail!("mpfs_gpio `ports` must be 1..=32, got {ports}");
            }
            Box::new(crate::peripherals::miv::MivCoreGpio::mpfs(ports as u32))
        }
        // PolarFire SoC MSS I2C0/1: master mode; devices attach through
        // `external_devices:`.
        "mpfs_i2c" => Box::new(crate::peripherals::mpfs_i2c::MpfsI2c::new()),
        // PolarFire SoC MSS SPI0/1: master mode; devices attach through
        // `external_devices:` and follow SLAVE_SELECT.
        "mpfs_spi" => Box::new(crate::peripherals::mpfs_spi::MpfsSpi::new()),
        // WCH-LinkE debug printf channel (DMDATA0/1 + attach sentinel).
        "wch_debug_printf" => Box::new(crate::peripherals::wch_debug_printf::WchDebugPrintf::new()),
        // QingKe system timer: the V2 32-bit layout by default; `profile: v4`
        // selects the CH32V20x/V30x 64-bit layout with MODE and INIT.
        "qingke_stk" => match p_cfg.config.get("profile").and_then(|v| v.as_str()) {
            None | Some("v2") => Box::new(crate::peripherals::qingke_stk::QingKeStk::new()),
            Some("v4") => Box::new(crate::peripherals::qingke_stk::QingKeStk::new_v4()),
            Some(other) => {
                anyhow::bail!("unsupported qingke_stk profile '{other}'; supported: v2, v4")
            }
        },
        "sam4s_pio" => Box::new(crate::peripherals::sam4s::Sam4sPio::new()),
        "sam4s_pmc" => Box::new(crate::peripherals::sam4s::Sam4sPmc::new()),
        "sam4s_wdt" => Box::new(crate::peripherals::sam::sysc4s::Sam4sWdt::new()),
        "sam4s_rstc" => Box::new(crate::peripherals::sam::sysc4s::Sam4sRstc::new()),
        "sam4s_eefc" => {
            // The GETD descriptor reports the plane size, so the part's flash
            // size must be stated; there is no family default.
            let Some(bytes) = p_cfg.config.get("flash_bytes").and_then(|v| v.as_u64()) else {
                anyhow::bail!("sam4s_eefc needs config.flash_bytes (the part's flash size)");
            };
            // SAM E70 parts state their 16 KiB lock regions and nine GPNVM
            // bits; the SAM4S geometry (8 KiB, two) is the default.
            let lock_region = p_cfg
                .config
                .get("lock_region_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(crate::peripherals::sam::sysc4s::EEFC_LOCK_REGION as u64);
            let gpnvm_bits = p_cfg
                .config
                .get("gpnvm_bits")
                .and_then(|v| v.as_u64())
                .unwrap_or(2);
            Box::new(crate::peripherals::sam::sysc4s::Sam4sEefc::with_geometry(
                bytes as u32,
                lock_region as u32,
                gpnvm_bits as u32,
            ))
        }
        "cortex_m7_cache" => {
            let kb = |key: &str| -> anyhow::Result<u32> {
                p_cfg
                    .config
                    .get(key)
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .ok_or_else(|| {
                        anyhow::anyhow!("cortex_m7_cache: config.{key} (KiB) is required")
                    })
            };
            let geometry = crate::peripherals::cortex_m7_cache::L1CacheGeometry::new(
                kb("icache_kb")?,
                kb("dcache_kb")?,
            )?;
            Box::new(crate::peripherals::cortex_m7_cache::CortexM7Cache::new(
                geometry,
            ))
        }
        "armv8m_mpu" => {
            let regions = p_cfg
                .config
                .get("regions")
                .and_then(|v| v.as_u64())
                .unwrap_or(8) as usize;
            if regions > crate::peripherals::armv8m_mpu::MAX_REGIONS || regions % 4 != 0 {
                anyhow::bail!("armv8m_mpu: config.regions = {regions} (0/4/8/12/16)");
            }
            Box::new(crate::peripherals::armv8m_mpu::Armv8mMpu::new(regions))
        }
        "cortex_m_fpu" => {
            let reg = |key: &str, default: u32| -> u32 {
                p_cfg
                    .config
                    .get(key)
                    .and_then(|v| v.as_u64())
                    .map_or(default, |v| v as u32)
            };
            use crate::peripherals::cortex_m_fpu as fpu;
            Box::new(fpu::CortexMFpu::new([
                reg("mvfr0", fpu::DEFAULT_MVFR0),
                reg("mvfr1", fpu::DEFAULT_MVFR1),
                reg("mvfr2", fpu::DEFAULT_MVFR2),
            ]))
        }
        "cc2538_pl061_gpio" => {
            // Pads that carry the reset IOC pull-up (SWRU319C §9.2.3); port C
            // passes 0xF0 because PC0-PC3 have no on-die pulls.
            let mask = p_cfg
                .config
                .get("pull_up_mask")
                .and_then(|v| v.as_u64())
                .unwrap_or(0xFF);
            Box::new(crate::peripherals::cc2538::Cc2538Gpio::with_pull_up_mask(
                mask as u32,
            ))
        }
        "cc2538_sys_ctrl" => Box::new(crate::peripherals::cc2538::Cc2538SysCtrl::new()),
        // `clock_hz`: the CPU clock the bus cycle count runs at, which turns
        // cycles into µs (RF timing) and 32-kHz ticks (sleep timer).
        "cc2538_rfcore" => {
            let hz = p_cfg
                .config
                .get("clock_hz")
                .and_then(|v| v.as_u64())
                .unwrap_or(crate::peripherals::cc2538::rfcore::DEFAULT_CLOCK_HZ);
            Box::new(crate::peripherals::cc2538::rfcore::Cc2538RfCore::new(hz))
        }
        "cc2538_sleep_timer" => {
            let hz = p_cfg
                .config
                .get("clock_hz")
                .and_then(|v| v.as_u64())
                .unwrap_or(crate::peripherals::cc2538::sleep_timer::DEFAULT_CLOCK_HZ);
            Box::new(crate::peripherals::cc2538::sleep_timer::Cc2538SleepTimer::new(hz))
        }
        // `flash_base`/`flash_size`: the main array the controller programs;
        // `clock_hz`: the clock its erase/write timings run at.
        "cc2538_flash_ctrl" => {
            use crate::peripherals::cc2538::flash_ctrl as fc;
            let get = |k: &str| p_cfg.config.get(k).and_then(|v| v.as_u64());
            Box::new(fc::Cc2538FlashCtrl::new(
                get("flash_base").map_or(fc::DEFAULT_FLASH_BASE, |v| v as u32),
                get("flash_size").map_or(fc::DEFAULT_FLASH_SIZE, |v| v as u32),
                get("clock_hz").unwrap_or(fc::DEFAULT_CLOCK_HZ),
            ))
        }
        "cc2538_aes" => Box::new(crate::peripherals::cc2538::aes::Cc2538Aes::new()),
        "cc2538_udma" => Box::new(crate::peripherals::cc2538::udma::Cc2538Udma::new()),
        // SSI0/1: master-mode SPI; devices attach through `external_devices:`.
        "cc2538_pl022_ssi" => Box::new(crate::peripherals::cc2538::ssi::Cc2538Ssi::new()),
        // I2CM master at 0x4002_0000; external devices attach through
        // `external_devices:`. I2CS remains a separate stub window.
        "cc2538_i2cm" => Box::new(crate::peripherals::cc2538::i2cm::Cc2538I2cm::new()),
        // IOC pad mux / override / input-select register file (storage only).
        "cc2538_ioc" => Box::new(crate::peripherals::cc2538::ioc::Cc2538Ioc::new()),
        // SOC_ADC: ADC single/sequence conversions, the RNG LFSR / CRC16 and
        // the analog comparator. Conversions complete inside the write.
        "cc2538_soc_adc" => Box::new(crate::peripherals::cc2538::soc_adc::Cc2538SocAdc::new()),
        // CCTEST (CC_TESTCTRL) and ANA_REGS: register storage only.
        "cc2538_cctest" => Box::new(crate::peripherals::cc2538::cctest::Cc2538CcTest::new()),
        "cc2538_ana_regs" => Box::new(crate::peripherals::cc2538::ana_regs::Cc2538AnaRegs::new()),
        // GPTIMER0-3: timer A pends the entry's `irq:`, timer B `irq_b`.
        "cc2538_gptimer" => Box::new(crate::peripherals::cc2538::gptimer::Cc2538GpTimer::new(
            p_cfg
                .config
                .get("irq_b")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
        )),
        "apollo4_gpio" => Box::new(crate::peripherals::apollo4::Apollo4Gpio::new()),
        "apollo4_pwrctrl" => Box::new(crate::peripherals::apollo4::Apollo4PwrCtrl::new()),
        "apollo4_cpu" => Box::new(crate::peripherals::apollo4::Apollo4Cpu::new()),
        "s32k3_siul2" => Box::new(crate::peripherals::s32k3_siul2::S32k3Siul2::new()),
        // `clock_hz`: the CPU clock the bus cycles run at; `swt_clock_hz`:
        // the counter clock (S32K3: SIRC, 32 kHz).
        "s32k3_swt" => {
            use crate::peripherals::s32k3 as s;
            let get = |k: &str| p_cfg.config.get(k).and_then(|v| v.as_u64());
            Box::new(s::S32k3Swt::new(
                get("clock_hz").unwrap_or(s::DEFAULT_CLOCK_HZ),
                get("swt_clock_hz").unwrap_or(s::DEFAULT_SWT_CLOCK_HZ),
            ))
        }
        "s32k3_mc_me" => Box::new(crate::peripherals::s32k3::S32k3McMe::new()),
        "lpc_flexcomm_usart" => {
            // PSELID present bits: USART/SPI/I2C/I2S (0xF0) unless the yaml
            // narrows them for a FLEXCOMM without every function.
            let present = p_cfg
                .config
                .get("present")
                .and_then(|v| v.as_u64())
                .unwrap_or(0xF0) as u32;
            Box::new(crate::peripherals::lpc::flexcomm_usart::LpcFlexcommUsart::new(present))
        }
        "lpc_gpio" => Box::new(crate::peripherals::lpc::gpio::LpcGpio::new()),
        "rt5xx_ctl" => Box::new(crate::peripherals::lpc::rt5xx_ctl::Rt5xxCtl::new()),
        "rt5xx_clkctl0" => Box::new(crate::peripherals::lpc::rt5xx_ctl::Rt5xxCtl::clkctl0()),
        "rt5xx_clkctl1" => Box::new(crate::peripherals::lpc::rt5xx_ctl::Rt5xxCtl::clkctl1()),
        "rt5xx_iopctl" => Box::new(crate::peripherals::lpc::rt5xx_ctl::Rt5xxCtl::plain()),
        "rt5xx_ostimer" => {
            use crate::peripherals::lpc::ostimer::{
                Rt5xxOsTimer, DEFAULT_CLOCK_HZ, DEFAULT_TICK_HZ,
            };
            let get = |k: &str, d: u64| p_cfg.config.get(k).and_then(|v| v.as_u64()).unwrap_or(d);
            Box::new(Rt5xxOsTimer::new(
                get("clock_hz", DEFAULT_CLOCK_HZ),
                get("tick_hz", DEFAULT_TICK_HZ),
            ))
        }
        _ => return Ok(None),
    };
    Ok(Some(dev))
}

/// Refuse a config key an MSP430 module does not define, and an `irq:`: the
/// modules request interrupts on their own lines, not through an NVIC number.
fn msp430_keys(p_cfg: &PeripheralConfig, allowed: &[&str]) -> anyhow::Result<()> {
    if let Some(k) = p_cfg.config.keys().find(|k| !allowed.contains(&k.as_str())) {
        anyhow::bail!("{} '{}' admits no config key '{k}'", p_cfg.r#type, p_cfg.id);
    }
    if p_cfg.irq.is_some() {
        anyhow::bail!(
            "{} '{}': interrupts use the request lines in the `msp430_sfr` config, not `irq`",
            p_cfg.r#type,
            p_cfg.id
        );
    }
    Ok(())
}

/// `config.<key>`: exactly `len` unsigned integers, or `None`.
fn u32_list(p_cfg: &PeripheralConfig, key: &str, len: usize) -> Option<Vec<u32>> {
    p_cfg
        .config
        .get(key)
        .and_then(|v| v.as_sequence())
        .map(|s| {
            s.iter()
                .filter_map(|v| v.as_u64())
                .map(|v| v as u32)
                .collect::<Vec<_>>()
        })
        .filter(|v| v.len() == len)
}

#[cfg(test)]
mod registry_agreement {
    use super::MODEL_TYPES;

    /// The families that existed when the scan below became directory-derived.
    /// It is a FLOOR, never the scan set: the test checks every family it finds
    /// on disk, and uses this only to prove the directory read actually saw the
    /// tree. A read that came back empty — wrong path, renamed directory — would
    /// otherwise satisfy every assertion for free, which is the exact failure
    /// mode this whole test exists to prevent. Names only ever get added here.
    const FAMILIES_AT_LEAST: &[&str] = &["esp32", "esp32c3", "esp32s3", "nrf52", "nrf54l"];

    /// Families discovered under `src/peripherals/*/factory.rs` that are
    /// deliberately NOT scanned, each with the reason. Empty today, and the
    /// default for a new family is to be scanned.
    ///
    /// It is not a blanket pass. The test re-derives the verdict for each
    /// excused family and fails if the family would now pass the scan cleanly:
    /// the only defensible reason to except one is that it genuinely builds
    /// types the reachability rule cannot express, so an exception that no
    /// longer excuses anything is stale and must be struck. A stale entry
    /// naming a family with no `factory.rs` on disk fails too — an exemption
    /// that reads as live while excusing nothing is worse than no entry.
    const FAMILY_SCAN_EXCEPTIONS: &[(&str, &str)] = &[];

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
    ///
    /// The families are derived the same way, for the same reason. They used to
    /// be a hardcoded literal, which matched `src/peripherals/*/factory.rs`
    /// exactly and so hid nothing — but a sixth family added tomorrow would get
    /// no coverage and nothing would fail, putting the family list back in the
    /// position the type list was rescued from. `src/` is resolved from
    /// `CARGO_MANIFEST_DIR`, not from the working directory, so the walk cannot
    /// pass vacuously by reading an empty path.
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

        let peripherals = src_root.join("peripherals");
        let mut families: Vec<String> = Vec::new();
        for entry in
            std::fs::read_dir(&peripherals).unwrap_or_else(|e| panic!("read {peripherals:?}: {e}"))
        {
            let path = entry.expect("dir entry").path();
            if path.join("factory.rs").is_file() {
                families.push(
                    path.file_name()
                        .expect("family directory name")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        families.sort();

        // Vacuity guard. Everything below is a loop over `families`; if the walk
        // found nothing, or lost a family that is still on disk, every assertion
        // would hold trivially and the gate would report green while looking at
        // nothing.
        let missing: Vec<&&str> = FAMILIES_AT_LEAST
            .iter()
            .filter(|k| !families.iter().any(|f| f == *k))
            .collect();
        assert!(
            missing.is_empty() && families.len() >= FAMILIES_AT_LEAST.len(),
            "the directory walk of {peripherals:?} found {families:?}, which is missing \
             {missing:?} of the families known to have a factory.rs — this check is \
             vacuous, it is scanning fewer sources than exist. Fix the walk; do not \
             shrink FAMILIES_AT_LEAST."
        );

        // `head` is everything left of a `=>`, i.e. the match patterns: the type
        // names a family factory can be asked to build. `lit.starts_with(family)`
        // keeps config keys and other incidental literals out. That filter reads
        // the family from the DIRECTORY name now, so it is only correct while a
        // family's directory name prefixes the types it builds — true for all
        // five today (and harmlessly a superset for the nested esp32/esp32c3/
        // esp32s3 names, where the broader family also sees the narrower's
        // literals). A family that named its types differently would silently
        // filter to nothing, so the caller asserts each family matched at least
        // one literal rather than trusting it.
        let scan = |family: &str| -> (usize, Vec<String>) {
            let src = std::fs::read_to_string(peripherals.join(family).join("factory.rs"))
                .unwrap_or_else(|e| panic!("read {family}/factory.rs: {e}"));
            let mut considered = 0usize;
            let mut bad = Vec::new();
            for line in src.lines() {
                let Some((head, _)) = line.split_once("=>") else {
                    continue;
                };
                for lit in head.split('"').skip(1).step_by(2) {
                    if !lit.starts_with(family) {
                        continue; // config keys and other string literals
                    }
                    considered += 1;
                    if !MODEL_TYPES.contains(&lit) && !alias_names.contains(&lit) {
                        bad.push(format!("{family}/factory.rs: {lit}"));
                    }
                }
            }
            (considered, bad)
        };

        let mut unreachable: Vec<String> = Vec::new();
        for family in &families {
            if FAMILY_SCAN_EXCEPTIONS.iter().any(|(f, _)| f == family) {
                continue;
            }
            let (considered, bad) = scan(family);
            assert!(
                considered > 0,
                "{family}/factory.rs has no match arm building a type that starts with \
                 \"{family}\", so this family is scanned for nothing and passes for free. \
                 Either its types do not carry the directory name as a prefix (then the \
                 prefix filter is wrong for it), or it is not a family factory at all \
                 (then it needs a FAMILY_SCAN_EXCEPTIONS entry saying so)."
            );
            unreachable.extend(bad);
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

        // An exception holds only while its reason does.
        for (family, reason) in FAMILY_SCAN_EXCEPTIONS {
            assert!(
                families.iter().any(|f| f == family),
                "FAMILY_SCAN_EXCEPTIONS excuses {family:?}, which has no factory.rs under \
                 {peripherals:?}. Strike the entry: a list naming families that do not \
                 exist cannot be read as a statement about the ones that do. Reason on \
                 record:\n  {reason}"
            );
            let (_, bad) = scan(family);
            assert!(
                !bad.is_empty(),
                "{family} is excused from the scan on the grounds that it builds types \
                 the reachability rule cannot express, but every type it builds is now \
                 reachable — the exception excuses nothing and only hides the family from \
                 future regressions. Strike the entry. Reason on record:\n  {reason}"
            );
        }
    }
}
