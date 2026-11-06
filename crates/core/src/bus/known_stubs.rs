// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The known-stub allowlist: the ONE place that says which peripheral `type:`
//! strings are allowed to be answered by a zero-filled [`StubPeripheral`]
//! instead of by a model.
//!
//! # Why this file exists
//!
//! `SystemBus::from_config` used to end its peripheral-type match with a
//! catch-all that installed a stub for ANY type it did not recognise. A chip
//! YAML could therefore name a peripheral this engine has never implemented,
//! firmware would talk to that address, read back zeros — and the run would
//! report success. That is the one failure a hardware oracle cannot have: a
//! green run that modelled nothing.
//!
//! An unknown type now FAILS THE LOAD. This table is the explicit, measured
//! exception list: every type below was observed reaching that catch-all in the
//! shipped configs, so hard-failing them would break chips that load today.
//! Each entry carries a written reason. Anything not listed here fails.
//!
//! # The rules
//!
//! * **Content-keyed.** The key is the canonical peripheral type string, not a
//!   count and not a file path. Two chips naming the same type share one entry.
//! * **Shrink-only.** `known_stub_allowlist_has_no_stale_entries` fails when an
//!   entry is no longer reached by any shipped chip descriptor. Model a
//!   peripheral (or delete the last chip that names it) and its entry becomes
//!   stale, so it MUST be deleted — the list cannot silently keep dead debt.
//! * **Growth is a deliberate act.** Adding an entry means writing down, in
//!   this file, why answering that silicon with zeros is acceptable. There is
//!   no other way to make a new unknown type load.
//!
//! # The intended exit
//!
//! Most entries are one of two things, and both have a real fix:
//!
//! * A window whose behaviour genuinely is "decode, return zeros". Those should
//!   migrate to the explicit `type: stub`, which says so in the chip YAML
//!   itself and needs no entry here beyond the one `stub` line.
//! * A vendor block nobody has modelled yet. Those exit by being modelled, or
//!   by a `type: declarative` descriptor.

/// Peripheral types allowed to resolve to a zero-filled stub, each with the
/// reason it is tolerated. Sorted by type; enforced by
/// `known_stub_allowlist_is_sorted_and_unique`.
///
/// Derived from a measurement, not from reading the match arms: every entry was
/// observed reaching the unknown-type catch-all while loading every descriptor
/// in `configs/chips/` (including `configs/chips/onboarding/`) and every shipped
/// system manifest under `configs/systems/`, `examples/`, `validation/`,
/// `fixtures/` and `configs/ci/`.
pub const KNOWN_STUBBED_PERIPHERAL_TYPES: &[(&str, &str)] = &[
    (
        "arraymemory",
        "Renode ArrayMemory window (a raw memory array declared as a peripheral). \
         Renode-imported onboarding descriptor; no model in this engine, so the window \
         answers reads with zeros. Used by: onboarding/litex_minerva, \
         onboarding/mpfs-icicle-kit, \
         onboarding/msp430f2619 (+8 more).",
    ),
    (
        "atmel91debugunit",
        "Atmel AT91 debug unit (DBGU). Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: onboarding/at91rm9200.",
    ),
    (
        "bitbanding",
        "Cortex-M bit-band alias region. Bit-banding is modelled by the bus itself \
         (SystemBus::bit_band_enabled), so the imported peripheral entry is inert. \
         Renode-imported onboarding descriptor; no model in this engine, so the window \
         answers reads with zeros. Used by: onboarding/cc2538.",
    ),
    (
        "button",
        "Renode board push-button. LabWired models buttons through `board_io`, not as a bus \
         peripheral (see attach_board_io_buttons), so the imported entry is inert. \
         Renode-imported onboarding descriptor; no model in this engine, so the window \
         answers reads with zeros. Used by: onboarding/arduino_101-shield, \
         onboarding/arty_litex_vexriscv and other imported descriptors.",
    ),
    (
        "cc2520",
        "TI CC2520 802.15.4 radio. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/quark_c1000-cc2520.",
    ),
    (
        "cc2538_cryptoprocessor",
        "TI CC2538 crypto processor. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/cc2538, \
         onboarding/zolertia-firefly.",
    ),
    (
        "cc2538_ssi",
        "TI CC2538 SSI (synchronous serial). Renode-imported onboarding descriptor; no model \
         in this engine, so the window answers reads with zeros. Used by: onboarding/cc2538, \
         onboarding/zolertia-firefly.",
    ),
    (
        "cc2538flashcontroller",
        "TI CC2538 flash controller. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/cc2538, \
         onboarding/zolertia-firefly.",
    ),
    (
        "cc2538rf",
        "TI CC2538 802.15.4 radio. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/cc2538, \
         onboarding/zolertia-firefly.",
    ),
    (
        "cc2538watchdog",
        "TI CC2538 watchdog. Renode-imported onboarding descriptor; no model in this engine, \
         so the window answers reads with zeros. Used by: onboarding/cc2538, \
         onboarding/zolertia-firefly.",
    ),
    (
        "efuse_stub",
        "ESP32-S3 eFuse window, authored as an explicit placeholder so eFuse reads decode \
         instead of faulting. No eFuse state is modelled. Used by: esp32s3-zero.",
    ),
    (
        "emios",
        "NXP eMIOS timer. Renode-imported onboarding descriptor; no model in this engine, so \
         the window answers reads with zeros. Used by: onboarding/mpc5567.",
    ),
    (
        "esamemorycontroller",
        "ESA/Gaisler memory controller. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: onboarding/leon3.",
    ),
    (
        "ethernetphysicallayer",
        "Ethernet PHY (MDIO endpoint). Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/arty_litex_vexriscv, onboarding/gr712rc, onboarding/leon3, \
         onboarding/litex_common, onboarding/litex_picorv32 (+13 more).",
    ),
    (
        "fastethernetcontroller",
        "NXP Fast Ethernet Controller (FEC). Renode-imported onboarding descriptor; no model \
         in this engine, so the window answers reads with zeros. Used by: onboarding/mpc5567.",
    ),
    (
        "flash_xip",
        "ESP32-S3 flash XIP cache window (I-cache / D-cache alias). A memory-map declaration, \
         wired for real by the Xtensa builder. Used by: esp32s3-zero.",
    ),
    (
        "gaisler_faulttolerantmemorycontroller",
        "Gaisler fault-tolerant memory controller. Renode-imported onboarding descriptor; no \
         model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/gr712rc.",
    ),
    (
        "gaislerahbplugandplayinfo",
        "Gaisler AHB plug-and-play info block. Renode-imported onboarding descriptor; no \
         model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/gr712rc, onboarding/leon3.",
    ),
    (
        "gaislerapbcontroller",
        "Gaisler APB bridge controller. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: onboarding/gr712rc, \
         onboarding/leon3.",
    ),
    (
        "gaislereth",
        "Gaisler GRETH Ethernet MAC. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/gr712rc, \
         onboarding/leon3.",
    ),
    (
        "hpet",
        "x86 High Precision Event Timer. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/acrn_x86_64, onboarding/up_squared_x86_64, onboarding/x86.",
    ),
    (
        "intmatrix",
        "ESP32-S3 interrupt matrix. Modelled in Rust and installed by the Xtensa builder; \
         this YAML entry only names the window for the debugger. Used by: esp32s3-zero.",
    ),
    (
        "intmatrix_stub",
        "Classic-ESP32 DPORT/interrupt-matrix window. Modelled in Rust and installed by the \
         Xtensa builder; the YAML entry names the window. Used by: esp32.",
    ),
    (
        "led",
        "Renode board LED. LabWired models board LEDs through `board_io` / external devices, \
         not as a bus peripheral, so the imported entry is inert. Renode-imported onboarding \
         descriptor; no model in this engine, so the window answers reads with zeros. Used \
         by: onboarding/arduino_101-shield, onboarding/arty_litex_vexriscv and other \
         imported descriptors.",
    ),
    (
        "mcpwm",
        "ESP32-S3 MCPWM window. Carries a debug_schema only; no PWM behaviour is modelled on \
         the from_config path. Used by: esp32s3, esp32s3-zero.",
    ),
    (
        "nrf5340_cache_stub",
        "nRF5340 CACHE window, authored placeholder (`_stub` suffix). Used by: nrf5340.",
    ),
    (
        "nrf5340_ctrlap_stub",
        "nRF5340 CTRL-AP window, authored placeholder. Used by: nrf5340.",
    ),
    (
        "nrf5340_dcnf_stub",
        "nRF5340 DCNF window, authored placeholder. Used by: nrf5340.",
    ),
    (
        "nrf5340_ficr_stub",
        "nRF5340 FICR window, authored placeholder. Used by: nrf5340.",
    ),
    (
        "nrf5340_fpu_stub",
        "nRF5340 FPU window, authored placeholder. Used by: nrf5340.",
    ),
    (
        "nrf5340_osc_reg_stub",
        "nRF5340 OSCILLATORS/REGULATORS window, authored placeholder. Used by: nrf5340.",
    ),
    (
        "nrf5340_spu_stub",
        "nRF5340 SPU window, authored placeholder. Used by: nrf5340.",
    ),
    (
        "nrf5340_trim_stub",
        "nRF5340 analog-trim window, authored placeholder. Used by: nrf5340.",
    ),
    (
        "nrf5340_uicr_stub",
        "nRF5340 UICR window, authored placeholder. Used by: nrf5340.",
    ),
    (
        "nrf54l_dppic_stub",
        "nRF54L DPPIC window, authored placeholder. Used by: nrf54l15.",
    ),
    (
        "nrf54l_ficr_stub",
        "nRF54L FICR window, authored placeholder. Used by: nrf54l15.",
    ),
    (
        "nrf54l_regulators_stub",
        "nRF54L REGULATORS window, authored placeholder. Used by: nrf54l15.",
    ),
    (
        "nrf54l_rramc_stub",
        "nRF54L RRAMC window, authored placeholder. Used by: nrf54l15.",
    ),
    (
        "nrf54l_tampc_stub",
        "nRF54L TAMPC window, authored placeholder. Used by: nrf54l15.",
    ),
    (
        "nrf54l_uicr_stub",
        "nRF54L UICR window, authored placeholder. Used by: nrf54l15.",
    ),
    (
        "nrf54l_wdt_stub",
        "nRF54L WDT window, authored placeholder. Used by: nrf54l15.",
    ),
    (
        "nvic",
        "Cortex-M NVIC. The stub is a construction-time transient: `configure_cortex_m` \
         finds this entry (by name or by base 0xE000_E100) and REPLACES it with the real \
         Nvic, publishing bus.nvic — see tests/nvic_masking_config_path.rs, which pins that \
         ordering. Used by: rp2040, rp2350, stm32f401, stm32f405, stm32f407 (+8 more).",
    ),
    (
        "pythonperipheral",
        "Renode PythonPeripheral: a register window whose behaviour lives in an embedded \
         Python snippet the .repl carried. There is nothing to port — the imported descriptor \
         has no register semantics at all. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/arduino_uno_r4_minima, onboarding/arvsom, onboarding/at91rm9200, \
         onboarding/atsamd51g19a, onboarding/beaglev_starlight and other imported descriptors.",
    ),
    (
        "ram",
        "Plain RAM/ROM/XIP window declared as a peripheral. The Xtensa builder \
         (system::xtensa::configure_xtensa_esp32*) is the authoritative wiring for these \
         parts; the chip YAML documents the memory map (see the header comment in esp32.yaml) \
         and from_config is only used by tooling. Used by: esp32, esp32s3-zero.",
    ),
    (
        "rmt",
        "ESP32-S3 RMT window. Carries a debug_schema only; the S3 RMT model is installed by \
         the Xtensa builder. Used by: esp32s3, esp32s3-zero.",
    ),
    (
        "rom_thunk_bank",
        "ESP32-S3 boot-ROM thunk bank. Real ROM entry points are installed by the Xtensa \
         builder, not by from_config; the YAML entry only reserves the window. Used by: \
         esp32s3-zero.",
    ),
    (
        "rtc_cntl_stub",
        "ESP32 / ESP32-S3 RTC_CNTL window. Explicitly authored as a placeholder (the `_stub` \
         suffix is the declaration); the Xtensa builder owns the real wiring. Used by: esp32, \
         esp32s3-zero.",
    ),
    (
        "scb",
        "Cortex-M System Control Block. Same construction-time transient as `nvic`: \
         `configure_cortex_m` replaces the entry at 0xE000_ED00 with the real Scb. Used by: \
         stm32l073, stm32l476.",
    ),
    (
        "stub",
        "The explicit, author-declared 'this window decodes as zeros' type. This is the \
         supported escape hatch a chip YAML uses to say the omission is deliberate — the \
         whole point of the unknown-type guard is that everything else must be named, not \
         inferred. Used by: onboarding/stm32f401cdu6-blackpill, rp2040, rp2350, stm32f103, \
         stm32f401cdu6 (+3 more).",
    ),
    (
        "system_stub",
        "ESP32 / ESP32-S3 SYSTEM (and classic I2S/PCNT/RMT/UHCI) windows, authored as \
         explicit placeholders so the address decodes. Used by: esp32, esp32s3, esp32s3-zero.",
    ),
    (
        "ti_lm74",
        "TI LM74 temperature sensor. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/arduino_101-shield, onboarding/quark_c1000-cc2520.",
    ),
    (
        "usb_serial_jtag",
        "ESP32-S3 USB-Serial-JTAG window. The S3 console path is wired by the Xtensa builder; \
         this YAML entry carries only a debug_schema. Used by: esp32s3-zero.",
    ),
];

/// The reason `canonical_type` is allowed to resolve to a zero-filled stub, or
/// `None` — in which case the load must fail.
///
/// `canonical_type` is the output of `SystemBus::canonical_peripheral_type`,
/// which is what the peripheral factories dispatch on.
pub fn known_stub_reason(canonical_type: &str) -> Option<&'static str> {
    KNOWN_STUBBED_PERIPHERAL_TYPES
        .iter()
        .find(|(t, _)| *t == canonical_type)
        .map(|(_, reason)| *reason)
}
