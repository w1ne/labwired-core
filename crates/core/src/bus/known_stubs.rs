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
        "ak09916",
        "AKM AK09916 magnetometer. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/renesas-ck_ra6m5.",
    ),
    (
        "ambiqapollo4_bootromlogger",
        "Ambiq Apollo4 boot-ROM logger. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/ambiq-apollo4.",
    ),
    (
        "andesatcpit100",
        "Andes ATCPIT100 PIT timer. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/egis_et171.",
    ),
    (
        "andesatcrtc100",
        "Andes ATCRTC100 RTC. Renode-imported onboarding descriptor; no model in this engine, \
         so the window answers reads with zeros. Used by: onboarding/egis_et171.",
    ),
    (
        "andesatcwdt200_watchdog",
        "Andes ATCWDT200 watchdog. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/egis_et171.",
    ),
    (
        "armsnoopcontrolunit",
        "ARM Snoop Control Unit (MPCore SCU). Renode-imported onboarding descriptor; no model \
         in this engine, so the window answers reads with zeros. Used by: \
         onboarding/cortex-a9, onboarding/cortex-a9_smp, onboarding/cortex-r8, \
         onboarding/cortex-r8_smp, onboarding/tegra2.",
    ),
    (
        "armsysctl",
        "ARM Versatile/VExpress system controller. Renode-imported onboarding descriptor; no \
         model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/versatile, onboarding/vexpress.",
    ),
    (
        "arraymemory",
        "Renode ArrayMemory window (a raw memory array declared as a peripheral). \
         Renode-imported onboarding descriptor; no model in this engine, so the window \
         answers reads with zeros. Used by: onboarding/atsamd21j17d-aft, \
         onboarding/litex_minerva, onboarding/msp430f2619, onboarding/nucleo_h753zi, \
         onboarding/nuvoton_npcx9 (+4 more).",
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
         answers reads with zeros. Used by: onboarding/cc2538, onboarding/efm32g210, \
         onboarding/efm32g222, onboarding/efm32g232, onboarding/efm32g842 (+32 more).",
    ),
    (
        "bmp180",
        "Bosch BMP180 pressure sensor. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/ice40up5k-mdp-evn.",
    ),
    (
        "button",
        "Renode board push-button. LabWired models buttons through `board_io`, not as a bus \
         peripheral (see attach_board_io_buttons), so the imported entry is inert. \
         Renode-imported onboarding descriptor; no model in this engine, so the window \
         answers reads with zeros. Used by: onboarding/arduino_101-shield, \
         onboarding/arty_litex_vexriscv, onboarding/brd4116a, onboarding/brd4117a, \
         onboarding/brd4118a (+24 more).",
    ),
    (
        "cadence_ttc",
        "Cadence Triple Timer Counter. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: onboarding/cortex-r8, \
         onboarding/cortex-r8_smp.",
    ),
    (
        "cadencegem",
        "Cadence GEM Ethernet MAC. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/mars_zx3, \
         onboarding/sam_e70, onboarding/sifive-fu540, onboarding/zedboard, \
         onboarding/zynq-7000.",
    ),
    (
        "cc1200",
        "TI CC1200 sub-GHz transceiver. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/zolertia-firefly.",
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
        "cosimulatedcfu",
        "Renode co-simulated custom function unit (needs an external Verilator process \
         LabWired does not run). Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/litex_vexriscv_verilated_cfu.",
    ),
    (
        "cosimulatedriscv32",
        "Renode co-simulated RISC-V core (needs an external Verilator process LabWired does \
         not run). Renode-imported onboarding descriptor; no model in this engine, so the \
         window answers reads with zeros. Used by: onboarding/verilated_ibex.",
    ),
    (
        "dwt",
        "Cortex-M Data Watchpoint and Trace unit. Owned by the CPU/debug model, not the bus. \
         Renode-imported onboarding descriptor; no model in this engine, so the window \
         answers reads with zeros. Used by: onboarding/atsamd21j17d-aft, \
         onboarding/atsamd51g19a, onboarding/eos-s3, onboarding/eos-s3-qomu, \
         onboarding/eos-s3-quickfeather (+10 more).",
    ),
    (
        "efr32_rtcc",
        "Silicon Labs EFR32 RTCC. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/brd4162a, \
         onboarding/efr32mg12, onboarding/efr32mg13, onboarding/sltb004a.",
    ),
    (
        "efuse_stub",
        "ESP32-S3 eFuse window, authored as an explicit placeholder so eFuse reads decode \
         instead of faulting. No eFuse state is modelled. Used by: esp32s3-zero.",
    ),
    (
        "egiset171_aosmu",
        "eGIS ET171 always-on system management unit. Renode-imported onboarding descriptor; \
         no model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/egis_et171.",
    ),
    (
        "egiset171_crypto",
        "eGIS ET171 crypto engine. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/egis_et171.",
    ),
    (
        "egiset171_smu2",
        "eGIS ET171 system management unit 2. Renode-imported onboarding descriptor; no model \
         in this engine, so the window answers reads with zeros. Used by: \
         onboarding/egis_et171.",
    ),
    (
        "ehcihostcontroller",
        "USB EHCI host controller. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/a20, \
         onboarding/colibri-vf61, onboarding/mars_zx3, onboarding/tegra2, onboarding/tegra3 \
         (+3 more).",
    ),
    (
        "emios",
        "NXP eMIOS timer. Renode-imported onboarding descriptor; no model in this engine, so \
         the window answers reads with zeros. Used by: onboarding/mpc5567.",
    ),
    (
        "emulatorcontroller",
        "Renode emulator control peripheral (host-side escape hatch, not silicon). \
         Renode-imported onboarding descriptor; no model in this engine, so the window \
         answers reads with zeros. Used by: onboarding/versatile, onboarding/vexpress.",
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
        "focaltechft9001_cpm",
        "FocalTech FT9001 clock/power manager. Renode-imported onboarding descriptor; no \
         model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/focaltech_ft9001.",
    ),
    (
        "focaltechft9001_reset",
        "FocalTech FT9001 reset controller. Renode-imported onboarding descriptor; no model \
         in this engine, so the window answers reads with zeros. Used by: \
         onboarding/focaltech_ft9001.",
    ),
    (
        "focaltechft9001_trng",
        "FocalTech FT9001 TRNG. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/focaltech_ft9001.",
    ),
    (
        "fslnand",
        "Freescale NAND flash controller. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/colibri-vf61, onboarding/vybrid.",
    ),
    (
        "ft5336",
        "FocalTech FT5336 touch controller. Renode-imported onboarding descriptor; no model \
         in this engine, so the window answers reads with zeros. Used by: \
         onboarding/stm32f7_discovery-bb.",
    ),
    (
        "fusionf0710a",
        "Fusion F0710A touch controller. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/colibri-vf61.",
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
        "hs3001",
        "Renesas HS3001 humidity sensor. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/renesas-ck_ra6m5.",
    ),
    (
        "icm20948",
        "TDK InvenSense ICM-20948 IMU. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/renesas-ck_ra6m5.",
    ),
    (
        "icp_101xx",
        "TDK ICP-101xx barometric sensor. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/renesas-ck_ra6m5.",
    ),
    (
        "imx_trng",
        "NXP i.MX TRNG. Renode-imported onboarding descriptor; no model in this engine, so \
         the window answers reads with zeros. Used by: onboarding/imxrt1064, \
         onboarding/mimxrt1064_evk.",
    ),
    (
        "imxrt700_micfil",
        "NXP i.MX RT700 MICFIL PDM microphone interface. Renode-imported onboarding \
         descriptor; no model in this engine, so the window answers reads with zeros. Used \
         by: onboarding/mimxrt700_evk, onboarding/mimxrt798s.",
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
        "isp1761",
        "NXP ISP1761 USB host/peripheral controller. Renode-imported onboarding descriptor; \
         no model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/versatile, onboarding/vexpress.",
    ),
    (
        "k6xf_ethernet",
        "NXP Kinetis K6xF Ethernet MAC. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: onboarding/imxrt1064, \
         onboarding/mimxrt1064_evk, onboarding/nxp-k6xf.",
    ),
    (
        "k6xf_mcg",
        "NXP Kinetis K6xF multipurpose clock generator. Renode-imported onboarding \
         descriptor; no model in this engine, so the window answers reads with zeros. Used \
         by: onboarding/nxp-k6xf.",
    ),
    (
        "k6xf_rng",
        "NXP Kinetis K6xF random number generator. Renode-imported onboarding descriptor; no \
         model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/nxp-k6xf.",
    ),
    (
        "k6xf_sim",
        "NXP Kinetis K6xF system integration module. Renode-imported onboarding descriptor; \
         no model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/nxp-k6xf.",
    ),
    (
        "led",
        "Renode board LED. LabWired models board LEDs through `board_io` / external devices, \
         not as a bus peripheral, so the imported entry is inert. Renode-imported onboarding \
         descriptor; no model in this engine, so the window answers reads with zeros. Used \
         by: onboarding/arduino_101-shield, onboarding/arduino_nano_33_ble, \
         onboarding/arty_litex_vexriscv, onboarding/brd4116a, onboarding/brd4117a (+30 more).",
    ),
    (
        "lis2ds12",
        "ST LIS2DS12 accelerometer. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/ice40up5k-mdp-evn.",
    ),
    (
        "litex_controlandstatus",
        "LiteX control-and-status register block. Renode-imported onboarding descriptor; no \
         model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/crosslink-nx-evn.",
    ),
    (
        "litex_mmcm_csr32",
        "LiteX MMCM clock-generator CSR block. Renode-imported onboarding descriptor; no \
         model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/arty_litex_vexriscv, onboarding/litex_vexriscv, \
         onboarding/litex_vexriscv_micropython.",
    ),
    (
        "litex_soc_controller",
        "LiteX SoC controller. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/crosslink-nx-evn, onboarding/litex_ibex, onboarding/litex_vexriscv_tftp, \
         onboarding/verilated_ibex.",
    ),
    (
        "lpc_can",
        "NXP LPC CAN controller. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/lpc2294.",
    ),
    (
        "lsm303dlhc_accelerometer",
        "ST LSM303DLHC accelerometer. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/ice40up5k-mdp-evn.",
    ),
    (
        "lsm303dlhc_gyroscope",
        "ST LSM303DLHC magnetometer/gyro half. Renode-imported onboarding descriptor; no \
         model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/ice40up5k-mdp-evn.",
    ),
    (
        "lsm330_accelerometer",
        "ST LSM330 accelerometer. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/ice40up5k-mdp-evn.",
    ),
    (
        "lsm330_gyroscope",
        "ST LSM330 gyroscope. Renode-imported onboarding descriptor; no model in this engine, \
         so the window answers reads with zeros. Used by: onboarding/ice40up5k-mdp-evn.",
    ),
    (
        "lsm9ds1_imu",
        "ST LSM9DS1 IMU. Renode-imported onboarding descriptor; no model in this engine, so \
         the window answers reads with zeros. Used by: onboarding/arduino_nano_33_ble.",
    ),
    (
        "lsm9ds1_magnetic",
        "ST LSM9DS1 magnetometer. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/arduino_nano_33_ble.",
    ),
    (
        "max32650_gcr",
        "Maxim MAX32650 global control registers. Renode-imported onboarding descriptor; no \
         model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/max32652, onboarding/max32652-evkit.",
    ),
    (
        "max32650_tpu",
        "Maxim MAX32650 trust protection unit. Renode-imported onboarding descriptor; no \
         model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/max32652, onboarding/max32652-evkit.",
    ),
    (
        "max32650_wdt",
        "Maxim MAX32650 watchdog. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/max32652, \
         onboarding/max32652-evkit.",
    ),
    (
        "mc3635",
        "mCube MC3635 accelerometer. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/eos-s3-quickfeather.",
    ),
    (
        "mcan",
        "Bosch M_CAN controller. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/nucleo_h753zi, \
         onboarding/stm32h743, onboarding/stm32h753.",
    ),
    (
        "mcpwm",
        "ESP32-S3 MCPWM window. Carries a debug_schema only; no PWM behaviour is modelled on \
         the from_config path. Used by: esp32s3, esp32s3-zero.",
    ),
    (
        "mpfs_sdcontroller",
        "Microchip PolarFire SoC SD/eMMC controller. Renode-imported onboarding descriptor; \
         no model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/polarfire-soc.",
    ),
    (
        "msp430f2xxx_mpy",
        "TI MSP430F2xxx hardware multiplier. Renode-imported onboarding descriptor; no model \
         in this engine, so the window answers reads with zeros. Used by: \
         onboarding/msp430f2619.",
    ),
    (
        "npcx_itim",
        "Nuvoton NPCX interval timer. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/nuvoton_npcx9, \
         onboarding/nuvoton_npcx9m6fb_evb.",
    ),
    (
        "npcx_mtc",
        "Nuvoton NPCX monotonic counter. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/nuvoton_npcx9, onboarding/nuvoton_npcx9m6fb_evb.",
    ),
    (
        "npcx_twd",
        "Nuvoton NPCX timer/watchdog. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/nuvoton_npcx9, \
         onboarding/nuvoton_npcx9m6fb_evb.",
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
        "nrf5340_dppic_stub",
        "nRF5340 DPPIC window, authored placeholder. Used by: nrf5340.",
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
        "opentitan_flashcontroller",
        "OpenTitan flash controller. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/opentitan-earlgrey, onboarding/opentitan-earlgrey-cw310.",
    ),
    (
        "opentitan_romcontroller",
        "OpenTitan ROM controller. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/opentitan-earlgrey, onboarding/opentitan-earlgrey-cw310.",
    ),
    (
        "opentitan_sramcontroller",
        "OpenTitan SRAM controller. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/opentitan-earlgrey, onboarding/opentitan-earlgrey-cw310.",
    ),
    (
        "pac1934",
        "Microchip PAC1934 power monitor. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/mpfs-icicle-kit.",
    ),
    (
        "pca9548",
        "NXP PCA9548 I2C switch. LabWired models bus switches as external devices \
         (validate_i2c_mux_topology), not as an MCU peripheral. Renode-imported onboarding \
         descriptor; no model in this engine, so the window answers reads with zeros. Used \
         by: onboarding/zynqmp-zcu102-revA, onboarding/zynqmp-zcu102-revB, \
         onboarding/zynqmp-zcu104.",
    ),
    (
        "pl031",
        "ARM PrimeCell PL031 RTC. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/cortex-a53-gicv2, onboarding/cortex-a53-gicv3, \
         onboarding/cortex-a53-gicv3_smp, onboarding/cortex-a78, onboarding/vexpress.",
    ),
    (
        "pl050",
        "ARM PrimeCell PL050 PS/2 interface. Renode-imported onboarding descriptor; no model \
         in this engine, so the window answers reads with zeros. Used by: \
         onboarding/versatile, onboarding/vexpress.",
    ),
    (
        "pl110",
        "ARM PrimeCell PL110 colour LCD controller. Renode-imported onboarding descriptor; no \
         model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/versatile, onboarding/vexpress.",
    ),
    (
        "pl310",
        "ARM PL310 L2 cache controller. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: onboarding/mars_zx3, \
         onboarding/tegra2, onboarding/tegra3, onboarding/vexpress, onboarding/zedboard (+1 \
         more).",
    ),
    (
        "pulp_i2s",
        "PULP I2S interface. Renode-imported onboarding descriptor; no model in this engine, \
         so the window answers reads with zeros. Used by: onboarding/A2_CV32E40P, \
         onboarding/core-v-mcu.",
    ),
    (
        "pythonperipheral",
        "Renode PythonPeripheral: a register window whose behaviour lives in an embedded \
         Python snippet the .repl carried. There is nothing to port — the imported descriptor \
         has no register semantics at all. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/arduino_uno_r4_minima, onboarding/arvsom, onboarding/at91rm9200, \
         onboarding/atsamd51g19a, onboarding/beaglev_starlight (+57 more).",
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
        "s32k3xx_lowpowerinterintegratedcircuit",
        "NXP S32K3xx LPI2C. Renode-imported onboarding descriptor; no model in this engine, \
         so the window answers reads with zeros. Used by: onboarding/nxp-s32k388, \
         onboarding/nxp-s32k388evb.",
    ),
    (
        "s32k3xx_miscellaneoussystemcontrolmodule",
        "NXP S32K3xx MSCM. Renode-imported onboarding descriptor; no model in this engine, so \
         the window answers reads with zeros. Used by: onboarding/nxp-s32k388, \
         onboarding/nxp-s32k388evb.",
    ),
    (
        "s32k3xx_realtimeclock",
        "NXP S32K3xx RTC. Renode-imported onboarding descriptor; no model in this engine, so \
         the window answers reads with zeros. Used by: onboarding/nxp-s32k388, \
         onboarding/nxp-s32k388evb.",
    ),
    (
        "s32k3xx_systemintegrationunitlite2",
        "NXP S32K3xx SIUL2. Renode-imported onboarding descriptor; no model in this engine, \
         so the window answers reads with zeros. Used by: onboarding/nxp-s32k388, \
         onboarding/nxp-s32k388evb.",
    ),
    (
        "s32kxx_modeentrymodule",
        "NXP S32K mode entry module. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/nxp-s32k388, \
         onboarding/nxp-s32k388evb.",
    ),
    (
        "sam_tc",
        "Microchip SAM timer/counter. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/sam4s, \
         onboarding/sam4s_xplained, onboarding/sam4s16c, onboarding/sam4s8b.",
    ),
    (
        "sam_trng",
        "Microchip SAM TRNG. Renode-imported onboarding descriptor; no model in this engine, \
         so the window answers reads with zeros. Used by: onboarding/sam_e70.",
    ),
    (
        "samd21_rtc",
        "Microchip SAMD21 RTC. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/atsamd21j17d-aft.",
    ),
    (
        "scb",
        "Cortex-M System Control Block. Same construction-time transient as `nvic`: \
         `configure_cortex_m` replaces the entry at 0xE000_ED00 with the real Scb. Used by: \
         stm32l073, stm32l476.",
    ),
    (
        "sema4",
        "NXP SEMA4 hardware semaphore. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/colibri-vf61, onboarding/vybrid.",
    ),
    (
        "si70xx",
        "Silicon Labs Si70xx humidity/temperature sensor. Renode-imported onboarding \
         descriptor; no model in this engine, so the window answers reads with zeros. Used \
         by: onboarding/sltb001a, onboarding/slwstk6220a.",
    ),
    (
        "smc91x",
        "SMSC LAN91C Ethernet controller. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: onboarding/versatile.",
    ),
    (
        "stm32_crc",
        "STM32 CRC unit as spelled by the Renode import (the modelled type is `crc`; this \
         vendor spelling is not aliased to it). Renode-imported onboarding descriptor; no \
         model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/nucleo_h753zi, onboarding/stm32f0, onboarding/stm32f042, \
         onboarding/stm32f072, onboarding/stm32f072b_discovery (+9 more).",
    ),
    (
        "stm32_independentwatchdog",
        "STM32 IWDG as spelled by the Renode import (the modelled type is `iwdg`). \
         Renode-imported onboarding descriptor; no model in this engine, so the window \
         answers reads with zeros. Used by: onboarding/nucleo_h753zi, \
         onboarding/nucleo_wba52cg, onboarding/stm32f4, onboarding/stm32f4_discovery, \
         onboarding/stm32f4_discovery-kit (+8 more).",
    ),
    (
        "stm32_pwr",
        "STM32 PWR as spelled by the Renode import (the modelled type is `pwr`). \
         Renode-imported onboarding descriptor; no model in this engine, so the window \
         answers reads with zeros. Used by: onboarding/stm32f4, onboarding/stm32f4_discovery, \
         onboarding/stm32f4_discovery-kit, onboarding/stm32f412, onboarding/stm32f429.",
    ),
    (
        "stm32_syscfg",
        "STM32 SYSCFG as spelled by the Renode import. Renode-imported onboarding descriptor; \
         no model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/nucleo_h753zi, onboarding/stm32f7_discovery-bb, onboarding/stm32f746, \
         onboarding/stm32h743, onboarding/stm32h753.",
    ),
    (
        "stm32f1afio",
        "STM32F1 AFIO as spelled by the Renode import (the modelled type is `afio`). \
         Renode-imported onboarding descriptor; no model in this engine, so the window \
         answers reads with zeros. Used by: onboarding/stm32f103.",
    ),
    (
        "stm32f4_flashcontroller",
        "STM32 flash controller as spelled by the Renode import (the modelled type is \
         `flash`). Renode-imported onboarding descriptor; no model in this engine, so the \
         window answers reads with zeros. Used by: onboarding/stm32g0.",
    ),
    (
        "stm32f4_rng",
        "STM32 RNG as spelled by the Renode import (the modelled type is `rng`). \
         Renode-imported onboarding descriptor; no model in this engine, so the window \
         answers reads with zeros. Used by: onboarding/nucleo_h753zi, onboarding/stm32f4, \
         onboarding/stm32f4_discovery, onboarding/stm32f4_discovery-kit, onboarding/stm32f412 \
         (+6 more).",
    ),
    (
        "stm32f4_rtc",
        "STM32 RTC as spelled by the Renode import (the modelled type is `rtc`). \
         Renode-imported onboarding descriptor; no model in this engine, so the window \
         answers reads with zeros. Used by: onboarding/nucleo_h753zi, \
         onboarding/nucleo_wba52cg, onboarding/stm32f0, onboarding/stm32f042, \
         onboarding/stm32f072 (+14 more).",
    ),
    (
        "stm32fsdmmc",
        "STM32F SDMMC as spelled by the Renode import. Renode-imported onboarding descriptor; \
         no model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/stm32f7_discovery-bb, onboarding/stm32f746.",
    ),
    (
        "stm32h7_flashcontroller",
        "STM32H7 flash controller as spelled by the Renode import. Renode-imported onboarding \
         descriptor; no model in this engine, so the window answers reads with zeros. Used \
         by: onboarding/nucleo_h753zi, onboarding/stm32h743, onboarding/stm32h753.",
    ),
    (
        "stm32h7_hardwaresemaphore",
        "STM32H7 HSEM as spelled by the Renode import. Renode-imported onboarding descriptor; \
         no model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/nucleo_h753zi, onboarding/stm32h743, onboarding/stm32h753.",
    ),
    (
        "stm32hsdmmc",
        "STM32H SDMMC as spelled by the Renode import. Renode-imported onboarding descriptor; \
         no model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/nucleo_h753zi, onboarding/stm32h743, onboarding/stm32h753.",
    ),
    (
        "stm32l0_pwr",
        "STM32L0 PWR as spelled by the Renode import. Renode-imported onboarding descriptor; \
         no model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/stm32l071, onboarding/stm32l072.",
    ),
    (
        "stm32ltdc",
        "STM32 LTDC display controller. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/nucleo_h753zi, onboarding/stm32f429, onboarding/stm32f7_discovery-bb, \
         onboarding/stm32f746, onboarding/stm32h743 (+1 more).",
    ),
    (
        "stm32wba_flashcontroller",
        "STM32WBA flash controller as spelled by the Renode import. Renode-imported \
         onboarding descriptor; no model in this engine, so the window answers reads with \
         zeros. Used by: onboarding/nucleo_wba52cg, onboarding/stm32wba52.",
    ),
    (
        "stm32wba_pwr",
        "STM32WBA PWR as spelled by the Renode import. Renode-imported onboarding descriptor; \
         no model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/nucleo_wba52cg, onboarding/stm32wba52.",
    ),
    (
        "stmcan",
        "STM32 bxCAN as spelled by the Renode import (the modelled type is `bxcan`). \
         Renode-imported onboarding descriptor; no model in this engine, so the window \
         answers reads with zeros. Used by: onboarding/stm32f0, onboarding/stm32f042, \
         onboarding/stm32f072, onboarding/stm32f072b_discovery, onboarding/stm32f4 (+7 more).",
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
        "sunximmc",
        "Allwinner sunxi MMC controller. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: onboarding/a20.",
    ),
    (
        "synopsysethernetmac",
        "Synopsys DesignWare Ethernet MAC. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: onboarding/stm32f4, \
         onboarding/stm32f4_discovery, onboarding/stm32f4_discovery-kit, \
         onboarding/stm32f412, onboarding/stm32f429 (+2 more).",
    ),
    (
        "system_stub",
        "ESP32 / ESP32-S3 SYSTEM (and classic I2S/PCNT/RMT/RTCIO/UHCI) windows, authored as \
         explicit placeholders so the address decodes. Used by: esp32, esp32s3, esp32s3-zero.",
    ),
    (
        "tca6416",
        "TI TCA6416 I2C GPIO expander. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/zynqmp-zcu102-revA, onboarding/zynqmp-zcu102-revB, \
         onboarding/zynqmp-zcu104.",
    ),
    (
        "tegradisplay",
        "NVIDIA Tegra display controller. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: onboarding/tegra2, \
         onboarding/tegra3.",
    ),
    (
        "tegradvc",
        "NVIDIA Tegra display and video controller. Renode-imported onboarding descriptor; no \
         model in this engine, so the window answers reads with zeros. Used by: \
         onboarding/tegra2.",
    ),
    (
        "tegrasyncpts",
        "NVIDIA Tegra sync points. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: onboarding/tegra2, \
         onboarding/tegra3.",
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
    (
        "usbhub",
        "USB hub. Renode-imported onboarding descriptor; no model in this engine, so the \
         window answers reads with zeros. Used by: onboarding/a20, onboarding/colibri-vf61, \
         onboarding/tegra2, onboarding/tegra3, onboarding/versatile (+1 more).",
    ),
    (
        "usbkeyboard",
        "USB HID keyboard. Renode-imported onboarding descriptor; no model in this engine, so \
         the window answers reads with zeros. Used by: onboarding/tegra2, onboarding/tegra3, \
         onboarding/versatile, onboarding/vexpress.",
    ),
    (
        "usbmouse",
        "USB HID mouse. Renode-imported onboarding descriptor; no model in this engine, so \
         the window answers reads with zeros. Used by: onboarding/colibri-vf61, \
         onboarding/tegra2, onboarding/tegra3, onboarding/versatile, onboarding/vexpress.",
    ),
    (
        "virtiommioconsole",
        "virtio-mmio console. Renode-imported onboarding descriptor; no model in this engine, \
         so the window answers reads with zeros. Used by: onboarding/cortex_a53_console.",
    ),
    (
        "virtiommioentropy",
        "virtio-mmio entropy source. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/cortex_a53_virtio.",
    ),
    (
        "vybriddcu",
        "NXP Vybrid display control unit. Renode-imported onboarding descriptor; no model in \
         this engine, so the window answers reads with zeros. Used by: \
         onboarding/colibri-vf61, onboarding/vybrid.",
    ),
    (
        "zmod4xxx",
        "Renesas ZMOD4xxx gas sensor. Renode-imported onboarding descriptor; no model in this \
         engine, so the window answers reads with zeros. Used by: \
         onboarding/renesas-ck_ra6m5.",
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
