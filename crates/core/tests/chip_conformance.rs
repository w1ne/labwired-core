// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Standardized per-chip conformance scoreboard + ratchet.
//!
//! ONE uniform battery, run for EVERY modeled chip, so coverage is comparable
//! across the fleet and can never silently regress. Declared-only prototypes
//! appear separately without a conformance level. This sits *on top of* the existing
//! mechanisms rather than replacing them:
//!
//!   * **Estate** (all modeled chips, always): the chip descriptor loads and every wired
//!     peripheral window is reachable (a read at its base faults nowhere).
//!   * **Registers vs silicon** (chips with a committed capture): the fraction of
//!     a real-silicon reset capture (`reg_oracle.json`) the sim reproduces. The
//!     deep per-register gate stays in `*_reset_conformance` / `register_coverage`;
//!     here we track the headline match% so it can't drop.
//!   * **Behavior** (chips with a golden firmware): whether a running-firmware
//!     gate exists (`firmware_survival` / `*_exec_oracle`), which boots real FW
//!     and asserts its register/IO effects. The named gate is **resolved against
//!     the tree** (`resolve_behavior_gate`) — a chip cannot be promoted on a
//!     string that names no test.
//!
//! The board is written to `docs/coverage/chip-conformance.md`; the ratchet
//! baseline is `docs/coverage/chip-conformance.json`. A chip's estate must stay
//! green, its reg-match% may not fall, and a present behavior gate may not vanish.
//! Re-baseline (after a deliberate, explained change):
//!   UPDATE_CONFORMANCE_BASELINE=1 cargo test -p labwired-core --test chip_conformance -- --nocapture
//!
//! "Are the gates enough?" is now a number per chip on the board — and missing
//! coverage is a visible red cell, not a silent gap.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

/// One chip's conformance inputs. `reset_oracle` and `behavior_gate` are `None`
/// until that coverage exists — the scoreboard then shows the gap.
struct ChipConf {
    name: &'static str,
    yaml: &'static str,
    /// Committed real-silicon reset capture (schema labwired-hw-oracle/*-regs).
    reset_oracle: Option<&'static str>,
    /// The running-firmware gate that asserts this chip's behavior, as
    /// `"<test target>"` or `"<test target>::<test fn>"`.
    ///
    /// This is NOT a free-form label. Every value here is **resolved** against
    /// the tree by [`resolve_behavior_gate`]: the named `crates/*/tests/
    /// <target>.rs` must exist and, when a function is named, that function must
    /// exist there carrying a `#[…test…]` attribute. An unresolvable string is a
    /// hard failure, not a silently-honoured claim.
    behavior_gate: Option<&'static str>,
}

/// Explicit declarations that have no modeled platform to measure. This is a
/// closed shortlist, validated against the descriptor before rendering; it
/// never contributes an estate checkmark, conformance level or ratchet record.
const DECLARED_ONLY_CHIPS: &[(&str, &str)] = &[
    (
        "rv1103",
        "Normal descriptor is CPU/memory only. Optional native reduced Linux direct boot and joint I2C3/UART4 basket acceptance pass separately with functional timing and a persistent reduced plant; this does not establish full board L0/L1, vendor BSP/SD/ROM boot or physical qualification. The older generic Cortex-A7 CPU subset is separate.",
    ),
];

/// The modeled fleet. Every descriptor MUST appear here or in the explicitly
/// validated declaration table (enforced below), so new chips cannot disappear.
const CHIPS: &[ChipConf] = &[
    ChipConf {
        name: "esp32c3",
        yaml: "configs/chips/esp32c3.yaml",
        reset_oracle: Some("scripts/hw-oracle/captures/esp32c3/20260611T161223Z/reg_oracle.json"),
        behavior_gate: Some("firmware_survival::test_esp32c3_demo_survival"),
    },
    ChipConf {
        // ESP32-C6 HP core (RV32IMAC). SIM-DERIVED: nothing here has been
        // diffed against a real C6 over JTAG, so no reset_oracle. Bases and
        // IRQs are cross-checked against the vendored espressif/svd ESP32-C6
        // SVD; the clock/reset (PCR) model is a register-backed stub and the
        // interrupt fabric is not wired (see docs/boards/esp32c6-devkitc.md).
        name: "esp32c6",
        yaml: "configs/chips/esp32c6.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_esp32c6_demo_survival"),
    },
    ChipConf {
        // LiteX VexRiscv SoC (RV32IM — the descriptor declares `core: rv32im`,
        // so runs select the standard RV32 profile). SIM-DERIVED: nothing here
        // has been diffed against a silicon capture, so no reset_oracle. The
        // modelled surface is CPU + memory + the LiteX UART and countdown
        // timer; the SPI-flash / Ethernet / MMCM CSR windows are declared
        // stubs — see
        // examples/litex-vexriscv/VALIDATION.md.
        name: "litex-vexriscv",
        yaml: "configs/chips/litex-vexriscv.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_litex_vexriscv_smoke_survival"),
    },
    ChipConf {
        // Digilent Arty A7-35T (LiteX VexRiscv SoC, RV32IM). SIM-DERIVED: no
        // bitstream capture exists. Modelled: CPU + memory + LiteX UART, timer
        // and the control-and-status LED/switch/button CSR; SPI/Ethernet/MMCM
        // are declared stubs — see examples/arty-litex-vexriscv/VALIDATION.md.
        name: "arty-litex-vexriscv",
        yaml: "configs/chips/arty-litex-vexriscv.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_arty_litex_smoke_survival"),
    },
    ChipConf {
        // Vega board (RI5CY, RV32IMC). SIM-DERIVED: no silicon capture.
        // Modelled: CPU + memory + LPUART0/1; the SCG/PCC clock blocks, INTMUX
        // and low-power timers are declared stubs — see
        // examples/vegaboard-ri5cy/VALIDATION.md.
        name: "vegaboard-ri5cy",
        yaml: "configs/chips/vegaboard-ri5cy.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_vegaboard_ri5cy_smoke_survival"),
    },
    ChipConf {
        // NUCLEO-L552ZE-Q (STM32L552ZET6Q, Cortex-M33 / Armv8-M). SIM-DERIVED:
        // no STM32L552 bench part. The new L5 RCC profile and the v2 GPIO /
        // LPUART / SPI / I2C / timer / DMA models carry the smoke. TrustZone/
        // SAU/GTZC, MPU/PMSAv8 enforcement and the L5 non-secure flash
        // controller are NOT modelled; FLASH/EXTI/ADC/FDCAN/USB are declared
        // stubs — see examples/nucleo-l552ze-q/VALIDATION.md.
        name: "stm32l552",
        yaml: "configs/chips/stm32l552.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nucleo_l552ze_q_smoke_survival"),
    },
    ChipConf {
        // CC2538DK (CC2538SF53, Cortex-M3). SIM-DERIVED: no CC2538 bench part.
        // Modelled: SYS_CTRL (cc2538_sys_ctrl), the PL061-style GPIO A-D
        // (cc2538_pl061_gpio, address-masked GPIO_DATA) and UART0/1 on the
        // pl011 layout, the RF core (cc2538_rfcore; on a shared 802.15.4 air,
        // RX FIFO + AUTOACK), the sleep timer, the flash controller, the
        // AES/SHA-256 engine and the µDMA (cc2538_udma). IOC, PKA, SSI, I2C,
        // GPTimers, USB, WDT and ADC are declared stubs — see
        // examples/cc2538dk/VALIDATION.md.
        name: "cc2538sf53",
        yaml: "configs/chips/cc2538sf53.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_cc2538dk_smoke_survival"),
    },
    ChipConf {
        // EK-RA6M5 (R7FA6M5BH, Cortex-M33). SIM-DERIVED: no RA6M5 bench part.
        // Modelled: the RA PORT (ra_port) and PmnPFS/PWPR (ra_pfs) reused
        // from RA4M1 at the RA6M5 bases, the full R_SYSTEM register file
        // (ra6_sysc) and SCI0 on the sci layout with the classic SCI FIFO
        // registers. MSTP, ICU, FCACHE, the C-cache, PSCU/CPSCU and the
        // listed timers/comms blocks are declared stubs; SCI1-9 and the rest
        // are not mapped — see examples/ek-ra6m5/VALIDATION.md.
        name: "r7fa6m5bh",
        yaml: "configs/chips/r7fa6m5bh.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_ek_ra6m5_smoke_survival"),
    },
    ChipConf {
        // EK-RA8M1 (R7FA8M1AH, Cortex-M85). SIM-DERIVED: no RA8M1 bench part.
        // Modelled: the RA PORT (ra_port), the RA8 PmnPFS/PWPR/PWPRS
        // (ra8_pfs), SYSTEM HOCOCR/OSCSF + the SCICKCR handshake (ra8_sysc)
        // and SCI3/SCI9 on the new sci_b UART layout, the ICU IELSR event
        // link (ra_icu) and AGT0/AGT1 (ra_agt). MSTP, the ICU IRQCR half and
        // the listed timers/comms blocks are declared stubs; the other SCIs
        // and the rest are not mapped — see examples/ek-ra8m1/VALIDATION.md.
        name: "r7fa8m1ah",
        yaml: "configs/chips/r7fa8m1ah.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_ek_ra8m1_smoke_survival"),
    },
    ChipConf {
        // Apollo4 Plus EVB (AMAP42KP-KBR, Cortex-M4F). SIM-DERIVED: no Apollo4
        // bench part. Modelled: the 128-pin GPIO block (apollo4_gpio,
        // PADKEY-locked PINCFG + RD/WT/WTS/WTC/EN), PWRCTRL (apollo4_pwrctrl)
        // and UART0-3 on the pl011 layout. CLKGEN, MCUCTRL, RSTGEN, IOM,
        // MSPI, ADC, timers, RTC, WDT, security, crypto, GPU/display, SDIO,
        // USB and audio are declared stubs — see
        // examples/apollo4p-evb/VALIDATION.md.
        name: "amap42kp",
        yaml: "configs/chips/amap42kp.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_apollo4p_evb_smoke_survival"),
    },
    ChipConf {
        // FRDM-K64F (MK64FN1M0VLL12, Cortex-M4F). SIM-DERIVED: no K64F bench
        // part. UART-only L1: classic Kinetis UART0-4, Kinetis GPIO A-E and the
        // behavioural MCG FEI block. SIM clock gating, PORT pad mux and MCG PLL
        // are not modelled; ENET/PHY, RNGA and FTFE are declared stubs;
        // eDMA/PIT/FTM/I2C/SPI/ADC/DAC/USB/CAN/SDHC are not mapped — see
        // examples/frdm-k64f/VALIDATION.md.
        name: "mk64f12",
        yaml: "configs/chips/mk64f12.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_frdm_k64f_smoke_survival"),
    },
    ChipConf {
        // SAM E70 Xplained (ATSAME70Q21B, Cortex-M7). SIM-DERIVED: no SAM E70
        // bench part. Modelled: PMC (sam4s_pmc) and PIOA-E (sam4s_pio), the
        // SAM4S layouts at the SAM E70 bases, and UART0-4/USART0-2 (sam
        // layout). EEFC, the system controller and the bus matrix are declared
        // stubs; GMAC/TRNG/TC/SPI/TWIHS/MCAN/AFEC/USBHS and the rest are not
        // mapped — see examples/sam-e70-xplained/VALIDATION.md.
        name: "same70q21b",
        yaml: "configs/chips/same70q21b.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_sam_e70_xplained_smoke_survival"),
    },
    ChipConf {
        // S32K388EVB-Q289 (S32K388, CM7_0 Cortex-M7 only). SIM-DERIVED: no
        // S32K388 bench part. Modelled: SIUL2 (s32k3_siul2) and LPUART0-15
        // (imxrt_lpuart, the same register layout). MC_ME/MC_CGM/SWT_0/STM_0
        // and the other clock/system blocks are declared stubs; FlexCAN,
        // LPSPI, LPI2C, eMIOS, eDMA, GMAC and the rest are not mapped — see
        // examples/s32k388-evb/VALIDATION.md.
        name: "s32k388",
        yaml: "configs/chips/s32k388.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_s32k388_evb_smoke_survival"),
    },
    ChipConf {
        // MIMXRT595-EVK (MIMXRT595S, Cortex-M33; the HiFi4 DSP is not
        // modelled). SIM-DERIVED: no MIMXRT595 bench part. Modelled: FLEXCOMM0
        // USART (lpc_flexcomm_usart), the high-speed GPIO (lpc_gpio), CLKCTL/
        // RSTCTL (rt5xx_ctl) and IOPCTL (rt5xx_iopctl), all storage-level for
        // the clock/pad banks. SYSCTL0/1, PMC, CACHE64, WWDT0, INPUTMUX and the
        // FlexSPI0 controller are declared stubs; the other FLEXCOMMs, timers,
        // DMA, USB, display and DSP blocks are not mapped — see
        // examples/mimxrt595-evk/VALIDATION.md.
        name: "mimxrt595s",
        yaml: "configs/chips/mimxrt595s.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_mimxrt595_evk_smoke_survival"),
    },
    ChipConf {
        // SAM4S Xplained (ATSAM4S16C, Cortex-M4). SIM-DERIVED: no SAM4S bench
        // part. Modelled: PMC (sam4s_pmc), the older PIO block (sam4s_pio),
        // UART0/1 and USART0/1 (sam layout); TC/SPI/TWI/EEFC and the system
        // controller are declared stubs — see examples/sam4s-xplained/VALIDATION.md.
        name: "sam4s16c",
        yaml: "configs/chips/sam4s16c.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_sam4s_xplained_smoke_survival"),
    },
    ChipConf {
        // STM32F072B-DISCO (STM32F072RBT6, Cortex-M0). SIM-DERIVED: no
        // STM32F072 bench part. Modelled: RCC (stm32f0 profile), GPIO v2,
        // USART1-4 v2, SPI/I2C/timers, DMA/EXTI/SYSCFG, bxCAN, DAC, RTC;
        // USB/TSC/CRS are declared stubs — see
        // examples/stm32f072b-disco/VALIDATION.md.
        name: "stm32f072",
        yaml: "configs/chips/stm32f072.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_stm32f072b_disco_smoke_survival"),
    },
    ChipConf {
        name: "nrf54l15",
        yaml: "configs/chips/nrf54l15.yaml",
        // No silicon capture: nothing here has been diffed against a real
        // nRF54L15 over SWD. Every register value is MDK/SVD-derived, which is
        // authoritative for the map but is not the same as measured silicon.
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nrf54l15_zephyr_survival"),
    },
    ChipConf {
        name: "atsamd21g18a",
        yaml: "configs/chips/atsamd21g18a.yaml",
        // No silicon capture: nothing here has been diffed against a real SAM
        // D21 over SWD. Every value is ATSAMD21G18A.svd-derived (Microchip,
        // Apache-2.0), which is authoritative for the map but is not measured
        // silicon.
        reset_oracle: None,
        behavior_gate: Some("atsamd21_peripheral_estate::the_estate_answers_at_its_own_addresses"),
    },
    // Exact A-variant SAM D21 orderable geometries on the public G18A engine.
    // They deliberately carry NO behavior gate: the engine's behavior is
    // already measured on atsamd21g18a, and these descriptors add no new
    // peripheral/clock fidelity. Their geometry is checked by
    // `atsamd21_variant_geometry`. L0 is the honest level here.
    ChipConf {
        name: "atsamd21e15a",
        yaml: "configs/chips/atsamd21e15a.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "atsamd21e16a",
        yaml: "configs/chips/atsamd21e16a.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "atsamd21e17a",
        yaml: "configs/chips/atsamd21e17a.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "atsamd21e18a",
        yaml: "configs/chips/atsamd21e18a.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "atsamd21g15a",
        yaml: "configs/chips/atsamd21g15a.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "atsamd21g16a",
        yaml: "configs/chips/atsamd21g16a.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "atsamd21g17a",
        yaml: "configs/chips/atsamd21g17a.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "atsamd21j15a",
        yaml: "configs/chips/atsamd21j15a.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "atsamd21j16a",
        yaml: "configs/chips/atsamd21j16a.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "atsamd21j17a",
        yaml: "configs/chips/atsamd21j17a.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "atsamd21j18a",
        yaml: "configs/chips/atsamd21j18a.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "nrf54lm20a",
        yaml: "configs/chips/nrf54lm20a.yaml",
        // No silicon capture: nothing here has been diffed against a real
        // nRF54LM20A over SWD. Every value is MDK/SVD-derived, which is
        // authoritative for the map but is not measured silicon.
        reset_oracle: None,
        behavior_gate: Some(
            "nrf54lm20a_peripheral_estate::the_estate_answers_at_its_own_addresses",
        ),
    },
    ChipConf {
        name: "esp32",
        yaml: "configs/chips/esp32.yaml",
        reset_oracle: None,
        // The committed Tier-1 fixture (tests/fixtures/tier1/esp32.elf) runs in
        // the PR lane via `test_esp32_tier1_survival`; class PASS lines with the
        // documented dma gap (esp32-no-mem2mem-dma), then `TIER1 done`.
        behavior_gate: Some("firmware_survival::test_esp32_tier1_survival"),
    },
    ChipConf {
        name: "esp32s3",
        yaml: "configs/chips/esp32s3.yaml",
        reset_oracle: None,
        // The committed Tier-1 S3 fixture runs in the PR lane via
        // `test_esp32s3_tier1_survival` (clock/gpio/timer/irq/dma/mcpwm/rmt/i2c
        // PASS, then `TIER1 done`).
        behavior_gate: Some("firmware_survival::test_esp32s3_tier1_survival"),
    },
    ChipConf {
        name: "esp32s3-zero",
        yaml: "configs/chips/esp32s3-zero.yaml",
        reset_oracle: None,
        // Board variant of esp32s3: shares the S3 silicon and the same Tier-1
        // fixture, but runs through the zero descriptor/system so a regression
        // in the zero YAML pair's load/parse/dispatch fails here and not only
        // in the CLI. (The fast-boot builder reads only `cpu_hz`, so the
        // descriptor's flash/RAM geometry is not pinned by this gate.)
        behavior_gate: Some("firmware_survival::test_esp32s3_zero_tier1_survival"),
    },
    ChipConf {
        name: "stm32f401cdu6",
        yaml: "configs/chips/stm32f401cdu6.yaml",
        reset_oracle: None,
        // The onboarding lane is push-to-main/schedule only, so it was withdrawn
        // as a gate; the committed blackpill demo fixture now runs in the PR
        // lane via `test_stm32f401cdu6_demo_survival` (asserts `OK` over USART2).
        behavior_gate: Some("firmware_survival::test_stm32f401cdu6_demo_survival"),
    },
    ChipConf {
        // WeAct F411 Black Pill. Sim-derived from ST's CMSIS header + the modm
        // F411 SVD; there is no bench part, so no reset_oracle.
        //
        // The tier-1 fixture (tests/fixtures/tier1/stm32f411.elf) is a committed
        // ELF and runs in the PR lane via `test_stm32f411_tier1_survival`
        // (asserts `TIER1 done` with class PASS lines).
        name: "stm32f411ceu6",
        yaml: "configs/chips/stm32f411ceu6.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_stm32f411_tier1_survival"),
    },
    ChipConf {
        // micro:bit V1 / Calliope mini 1, S110 application region only. A
        // legacy-UART smoke survival gate and nothing else: no silicon capture
        // and no executing-fidelity differential, so this is L1 smoke. (The
        // official MakeCode images need the SoftDevice HLE, which runs under
        // examples/sd_hle_run.rs, not a gate here.)
        name: "nrf51822",
        yaml: "configs/chips/nrf51822.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nrf51822_microbit_v1_smoke_survival"),
    },
    ChipConf {
        name: "nrf52832",
        yaml: "configs/chips/nrf52832.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nrf52832_demo_survival"),
    },
    ChipConf {
        // micro:bit v2 target. Same standing as its nRF52 siblings: a UARTE
        // EasyDMA smoke survival gate and a config-build gate, and nothing
        // else. No silicon capture (no bench nRF52833 was diffed over SWD) and
        // no executing-fidelity differential, so this is L1 smoke.
        name: "nrf52833",
        yaml: "configs/chips/nrf52833.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nrf52833_microbit_v2_smoke_survival"),
    },
    ChipConf {
        name: "nrf52840",
        yaml: "configs/chips/nrf52840.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nrf52840_demo_survival"),
    },
    ChipConf {
        name: "nrf5340",
        yaml: "configs/chips/nrf5340.yaml",
        reset_oracle: None,
        // Behaviour gate is the real unmodified Zephyr v3.7 hello_world boot on
        // the application core (Cortex-M33). The ELF-independent twin that
        // replays the boot clock/SCS poll loops is tests/nrf5340_clock_boot.rs.
        behavior_gate: Some("firmware_survival::test_nrf5340_zephyr_survival"),
    },
    ChipConf {
        name: "rp2040",
        yaml: "configs/chips/rp2040.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_rp2040_demo_survival"),
    },
    ChipConf {
        name: "stm32f103",
        yaml: "configs/chips/stm32f103.yaml",
        reset_oracle: None,
        behavior_gate: Some("stm32f1_exec_oracle"),
    },
    // The exact NUCLEO-F103RB chip geometry shares the family peripheral
    // definitions. The board UART/GPIO smoke is gated in
    // core-coverage-matrix-smoke.yml; no exact-part silicon capture is claimed.
    ChipConf {
        name: "stm32f103rb",
        yaml: "configs/chips/stm32f103rb.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    // Exact low-density F103 order codes share the F103 engine. Geometry is
    // checked per part, but there is no per-order-code silicon capture or
    // executing-fidelity differential, so these remain honest L0 entries.
    ChipConf {
        name: "stm32f103c4",
        yaml: "configs/chips/stm32f103c4.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f103c6",
        yaml: "configs/chips/stm32f103c6.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f103r4",
        yaml: "configs/chips/stm32f103r4.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f103r4h",
        yaml: "configs/chips/stm32f103r4h.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f103r6",
        yaml: "configs/chips/stm32f103r6.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f103r6h",
        yaml: "configs/chips/stm32f103r6h.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f103t4",
        yaml: "configs/chips/stm32f103t4.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f103t6",
        yaml: "configs/chips/stm32f103t6.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401",
        yaml: "configs/chips/stm32f401.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_stm32f401_blinky_survival"),
    },
    // Exact F401 order codes carry the family's same peripheral engine. Their
    // package/memory geometry is pinned independently; no part-specific
    // silicon or executing-fidelity receipt exists yet (L0).
    ChipConf {
        name: "stm32f401cb",
        yaml: "configs/chips/stm32f401cb.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401cc",
        yaml: "configs/chips/stm32f401cc.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401cd",
        yaml: "configs/chips/stm32f401cd.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401ce",
        yaml: "configs/chips/stm32f401ce.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401rb",
        yaml: "configs/chips/stm32f401rb.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401rc",
        yaml: "configs/chips/stm32f401rc.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401rd",
        yaml: "configs/chips/stm32f401rd.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401re",
        yaml: "configs/chips/stm32f401re.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401vb",
        yaml: "configs/chips/stm32f401vb.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401vc",
        yaml: "configs/chips/stm32f401vc.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401vd",
        yaml: "configs/chips/stm32f401vd.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401ve",
        yaml: "configs/chips/stm32f401ve.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f405",
        yaml: "configs/chips/stm32f405.yaml",
        reset_oracle: None,
        // The committed Tier-1 fixture (tests/fixtures/tier1/stm32f405.elf) runs
        // in the PR lane via `test_stm32f405_tier1_survival`.
        behavior_gate: Some("firmware_survival::test_stm32f405_tier1_survival"),
    },
    ChipConf {
        name: "stm32f767",
        yaml: "configs/chips/stm32f767.yaml",
        reset_oracle: None,
        // The committed Tier-1 fixture (tests/fixtures/tier1/stm32f767.elf) runs
        // in the PR lane via `test_stm32f767_tier1_survival`.
        behavior_gate: Some("firmware_survival::test_stm32f767_tier1_survival"),
    },
    ChipConf {
        name: "rp2350",
        yaml: "configs/chips/rp2350.yaml",
        reset_oracle: None,
        // The committed demo fixture (tests/fixtures/rp2350-demo.elf) runs in the
        // PR lane via `test_rp2350_demo_survival` (asserts `RP2350_SMOKE_OK`).
        behavior_gate: Some("firmware_survival::test_rp2350_demo_survival"),
    },
    ChipConf {
        name: "stm32f407",
        yaml: "configs/chips/stm32f407.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nucleo_f407_smoke_survival"),
    },
    // Exact F407 order codes share the NUCLEO-F407 engine. Geometry is pinned
    // per part; no per-part silicon capture or executing-fidelity differential
    // exists, so these are listed at L0.
    ChipConf {
        name: "stm32f407ie",
        yaml: "configs/chips/stm32f407ie.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f407ig",
        yaml: "configs/chips/stm32f407ig.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f407ve",
        yaml: "configs/chips/stm32f407ve.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f407vg",
        yaml: "configs/chips/stm32f407vg.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f407ze",
        yaml: "configs/chips/stm32f407ze.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f407zg",
        yaml: "configs/chips/stm32f407zg.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        // No silicon reset capture. The external-artifact customer ELF WAKE
        // test runs explicitly in iolinki-reference.yml, but is ignored in
        // the standard PR lane and cannot promote this conformance level.
        // No electrical IO-Link or physical validation claim.
        name: "stm32g0b1re",
        yaml: "configs/chips/stm32g0b1re.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32g474re",
        yaml: "configs/chips/stm32g474re.yaml",
        reset_oracle: None,
        // Unmodified Zephyr hello_world for nucleo_g474re; asserts
        // "Hello World! nucleo_g474re" over UART (PR-run).
        behavior_gate: Some("firmware_survival::test_stm32g474_zephyr_survival"),
    },
    ChipConf {
        name: "stm32h563",
        yaml: "configs/chips/stm32h563.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_stm32h563_demo_survival"),
    },
    ChipConf {
        // First U5 part. Sim-derived (RM0456 + the vendor SVD); no bench part
        // has been captured, so no reset_oracle. The running-firmware gate is
        // the committed stock Zephyr 3.7.2 hello_world for nucleo_u575zi_q
        // (fixture + case added in the Task 8 onboarding close-out; the
        // Arduino L0 serial case covers the Cube-startup CRC path).
        name: "stm32u575",
        yaml: "configs/chips/stm32u575.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_stm32u575_zephyr_survival"),
    },
    ChipConf {
        // NUCLEO-U545RE-Q. U575-family derivative (stm32u545.yaml is drift-
        // guarded against stm32u575.yaml). Sim-derived from ST's U545 SVD; no
        // bench part, so no reset_oracle. Running-firmware gate: the committed
        // bare-metal blinky (USART1 VCP banner + PA5 loop).
        name: "stm32u545",
        yaml: "configs/chips/stm32u545.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_stm32u545_blinky_survival"),
    },
    ChipConf {
        // NUCLEO-U5A5ZJ-Q, sharing MB1549 with U575 but carrying the U5A5
        // 4 MiB flash / 2.5 MB-class SRAM map and crypto accelerators. The
        // register-level SHA/AES/PKA vector is the current behavior gate.
        name: "stm32u5a5zj",
        yaml: "configs/chips/stm32u5a5zj.yaml",
        reset_oracle: None,
        behavior_gate: Some(
            "stm32u5_exec_oracle::u5a5_hash_saes_and_pka_execute_register_level_operations",
        ),
    },
    ChipConf {
        // First Cortex-M7 chip. Sim-derived (RM0468); no silicon capture, so no
        // reset_oracle.
        //
        // Was `Some("tier1::stm32h735")` with the comment "behaviour asserted by
        // the tier-1 fixture self-tests". That string named no test — the
        // `tier1` test target exists but has no stm32h735 case — so the claim
        // was withdrawn at the time.
        //
        // The tier-1 fixture is a committed ELF and now runs in the PR lane via
        // `test_stm32h735_tier1_survival` (asserts `TIER1 done` with class PASS
        // lines), so the behavior claim is live again.
        //
        // Hosted-compile status: not a core-model failure. The in-core
        // h735-telematics-lab and F401 control builds pass (-eabi). The umbrella
        // repo wires the board (compileId `stm32h735` -> ststm32/disco_h735ig/
        // stm32cube) and documents the production failure as a stale deployed
        // image missing framework-stm32cubeh7 (compile-support.ts): an umbrella
        // image-freshness gap, not a core gap.
        name: "stm32h735",
        yaml: "configs/chips/stm32h735.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_stm32h735_tier1_survival"),
    },
    ChipConf {
        name: "stm32l073",
        yaml: "configs/chips/stm32l073.yaml",
        reset_oracle: Some("scripts/hw-oracle/captures/stm32l073/reg_oracle.json"),
        behavior_gate: Some("firmware_survival::test_nucleo_l073rz_smoke_survival"),
    },
    ChipConf {
        name: "stm32l476",
        yaml: "configs/chips/stm32l476.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nucleo_l476rg_demo_survival"),
    },
    ChipConf {
        name: "stm32wb55",
        yaml: "configs/chips/stm32wb55.yaml",
        reset_oracle: None,
        // Dual-core (M4 + M0+): the boot exercises the HSEM inter-core lock and
        // the classic RCC BDCR LSE path; asserts "Hello World! nucleo_wb55rg".
        behavior_gate: Some("firmware_survival::test_stm32wb55_zephyr_survival"),
    },
    ChipConf {
        name: "stm32wba52",
        yaml: "configs/chips/stm32wba52.yaml",
        reset_oracle: None,
        // Cortex-M33: exercises the WBA-specific RCC (CFGR1/BDCR1, the 0x28
        // request/ack) and the PWR VOSR handshake; asserts
        // "Hello World! nucleo_wba52cg".
        behavior_gate: Some("firmware_survival::test_stm32wba52_zephyr_survival"),
    },
    // NXP KW41Z (Cortex-M0+ BLE + 802.15.4). Register surface ingested from the
    // public CMSIS-SVD; radio (BTLE_RF/GENFSK/ZLL/XCVR) not yet modelled. The
    // behavior gate boots bare-metal firmware that prints over LPUART0.
    ChipConf {
        name: "mkw41z4",
        yaml: "configs/chips/mkw41z4.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_kw41z_smoke_survival"),
    },
    // Silicon Labs EFR32MG26 (Series-2, Cortex-M33). Register surface from the
    // simplicity_sdk CMSIS headers (no public SVD exists); L1 smoke only —
    // Real silicon capture, taken over SWD from a BRD2709A on 2026-08-21 —
    // the SECOND chip in this table to have one, after esp32c3. CMU, GPIO,
    // TIMER0/1, USART0, IADC0 and I2C0 are modelled from the vendor CMSIS
    // headers (Silicon Labs publishes no SVD for this family), and this is the
    // capture that says the model agrees with the die.
    //
    // ⚠️ The capture state is `reset_halt+preamble`, not pure reset_halt, and
    // that is a property of the silicon rather than a shortcut: a Series-2
    // peripheral that is not clocked does not read as zero over the debug port,
    // it FAULTS, and openocd abandons the rest of its command list. A bare
    // `reset halt` capture returns the CMU window and dies at GPIO. The
    // preamble writes CLKEN0 and nothing else, so every register below is
    // still its reset value.
    ChipConf {
        name: "efr32mg26",
        yaml: "configs/chips/efr32mg26.yaml",
        reset_oracle: Some(
            "scripts/hw-oracle/captures/efr32mg26/20260903T155944Z-msc/reg_oracle.json",
        ),
        // The BRD2709A agent deck, running. The reset oracle above is an L1
        // claim -- the register FILE matches the die, 219/219 -- which says
        // nothing about whether a driver written against those registers makes
        // a panel light up. This runs the deck firmware in process and asserts
        // the glass is lit and fully inked, that the I2S mic drives its LEFT
        // half and tristates the right, that an IADC conversion lands on 2048,
        // and that five contacts read at their own idle polarities.
        behavior_gate: Some("efr32_deck_behavior::the_deck_firmware_drives_every_part"),
    },
    // Classic Arduino Nano / ATmega328P — sim-smoke twin (PORT/Timer0/USART0).
    // Behavior: PlatformIO nanoatmega328 golden (serial nano-ok + D13 toggle).
    // No silicon SWD capture yet → stays below L2 reg-match.
    ChipConf {
        name: "atmega328p",
        yaml: "configs/chips/atmega328p.yaml",
        reset_oracle: None,
        behavior_gate: Some("avr_nano_golden_survival::arduino_nano_golden_prints_and_blinks"),
    },
    // Behavior: Arduino-core Mega 2560 sketch (LED pins on ports E/G/F/B/H,
    // A8/A9 via MUX5, millis, Serial). No silicon capture → below L2.
    ChipConf {
        name: "atmega2560",
        yaml: "configs/chips/atmega2560.yaml",
        reset_oracle: None,
        behavior_gate: Some("avr_mega2560_io::each_led_lands_on_its_mega_port_pin"),
    },
    // 64-pin ATmega2561, the same public AVR core but a narrower SKU. Behavior:
    // the committed 3-byte-PC image boots, prints on USART0 and drives PB5;
    // the descriptor test pins 54 GPIO, eight ADC inputs and no ports H..L.
    // No silicon capture → below L2.
    ChipConf {
        name: "atmega2561",
        yaml: "configs/chips/atmega2561.yaml",
        reset_oracle: None,
        behavior_gate: Some("avr_atmega2561::atmega2561_boots_and_prints"),
    },
    // 64-pin ATmega1281. The committed AVR-GCC image exercises the exact
    // two-byte-PC profile, USART0 output and PB5 package pin. No silicon
    // capture exists, so the behavior gate supports L1 only.
    ChipConf {
        name: "atmega1281",
        yaml: "configs/chips/atmega1281.yaml",
        reset_oracle: None,
        behavior_gate: Some(
            "avr_atmega1281_firmware::atmega1281_compiled_firmware_prints_and_drives_pb5",
        ),
    },
    ChipConf {
        name: "atsamd21",
        yaml: "configs/chips/atsamd21.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_atsamd21_nano33_smoke_survival"),
    },
    ChipConf {
        name: "atsamd51",
        yaml: "configs/chips/atsamd51.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_atsamd51_metro_m4_smoke_survival"),
    },
    ChipConf {
        name: "atsamd51-pybadge",
        yaml: "configs/chips/atsamd51-pybadge.yaml",
        reset_oracle: None,
        // PyBadge application layout of atsamd51 (reset_vector_offset 0x4000).
        // No running-firmware gate: pybadge_config.rs builds the bus and checks
        // the 0x4000 reset SP/PC with an in-test stub, but runs no firmware, and
        // the Metro M4 survival smoke loads atsamd51.yaml, not this descriptor.
        // Estate-only (L0) until a PyBadge image runs through this chip.
        behavior_gate: None,
    },
    ChipConf {
        name: "ra4m1",
        yaml: "configs/chips/ra4m1.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_ra4m1_uno_r4_smoke_survival"),
    },
    ChipConf {
        name: "imxrt1064",
        yaml: "configs/chips/imxrt1064.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_imxrt1064_teensy41_smoke_survival"),
    },
    ChipConf {
        name: "stm32f746",
        yaml: "configs/chips/stm32f746.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_stm32f746_discovery_smoke_survival"),
    },
    ChipConf {
        // First STM32G0 part (RM0444). SIM-DERIVED: no bench board has been
        // captured, so no reset_oracle. The behaviour gate is the committed
        // NUCLEO-G071RB UART smoke, which additionally pins the dedicated
        // `stm32g0` RCC layout (a wrong IOPENR/APBENR1 offset gags the UART).
        name: "stm32g071",
        yaml: "configs/chips/stm32g071.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nucleo_g071rb_smoke_survival"),
    },
    ChipConf {
        // First PIC24/dsPIC33F target. SIM-DERIVED: no silicon capture yet,
        // so no reset_oracle; the behavior gate is the committed
        // hand-assembled UART smoke. M1a added timers + interrupts, M1b the
        // GPIO ports, PPS, input capture and output compare.
        name: "dspic33fj32mc204",
        yaml: "configs/chips/dspic33fj32mc204.yaml",
        reset_oracle: None,
        behavior_gate: Some("pic24_m0_smoke::pic24_fixture_boots_and_writes_h_to_uart"),
    },
    ChipConf {
        // 28-pin sibling of the 204, same datasheet/core/memory. The gate is
        // a real XC16-built UART firmware, not the hand fixture.
        name: "dspic33fj32mc202",
        yaml: "configs/chips/dspic33fj32mc202.yaml",
        reset_oracle: None,
        behavior_gate: Some("pic24_xc16_202_smoke::xc16_202_firmware_boots_and_writes_hi_to_uart"),
    },
    // PIC18F4520 (DS39631E), the first PIC18 part. The gate boots real SDCC
    // 4.4.0 C firmware (software stack, TBLRD string reads, EUSART TX,
    // PORTB latch). Timers, CCP, MSSP, ADC, EEPROM, EUSART RX and XINST are
    // not exercised, so this is L1, not full-device parity.
    ChipConf {
        name: "pic18f4520",
        yaml: "configs/chips/pic18f4520.yaml",
        reset_oracle: None,
        behavior_gate: Some("pic18f4520_smoke::sdcc_firmware_boots_and_prints_the_banner"),
    },
    // PIC16F877A (DS39582B), the first PIC16 part. The gate boots real SDCC
    // 4.4.0 pic14 C firmware (bank switching, shared-RAM locals, computed
    // PCL jumps into RETLW string tables, USART TX, PORTB latch). Timers,
    // CCP, MSSP, ADC, EEPROM and USART RX are not exercised, so this is L1,
    // not full-device parity.
    ChipConf {
        name: "pic16f877a",
        yaml: "configs/chips/pic16f877a.yaml",
        reset_oracle: None,
        behavior_gate: Some("pic16f877a_smoke::sdcc_firmware_boots_and_prints_the_banner"),
    },
    // PIC16F1829 (DS40001440E), the first enhanced mid-range part. The gate
    // boots real SDCC 4.4.0 C firmware: BSR/MOVLB banking, MOVLP paging,
    // FSR/MOVIW pointers, EUSART TX, LATA, and an ISR without software
    // context save preempted into by TMR0, so the hardware shadows carry
    // main's state. CCP, MSSP, ADC, EEPROM and EUSART RX are not exercised,
    // so this is L1, not full-device parity.
    ChipConf {
        name: "pic16f1829",
        yaml: "configs/chips/pic16f1829.yaml",
        reset_oracle: None,
        behavior_gate: Some("pic16f1829_smoke::sdcc_firmware_boots_and_prints_the_banner"),
    },
    // PIC24F KA family (DS30009937C / DS39927C). The modelled slice is the
    // package GPIO masks and UART1; each gate boots that part's own real
    // XC16 v1.70 image and checks `HI`. Data EEPROM, UART2, timers, ADC, SPI
    // and I2C are not exercised, so this is L1, not full-device parity.
    ChipConf {
        name: "pic24f04ka200",
        yaml: "configs/chips/pic24f04ka200.yaml",
        reset_oracle: None,
        behavior_gate: Some("pic24_ka_smoke::pic24f04ka200_uart_smoke"),
    },
    ChipConf {
        name: "pic24f04ka201",
        yaml: "configs/chips/pic24f04ka201.yaml",
        reset_oracle: None,
        behavior_gate: Some("pic24_ka_smoke::pic24f04ka201_uart_smoke"),
    },
    ChipConf {
        name: "pic24f08ka101",
        yaml: "configs/chips/pic24f08ka101.yaml",
        reset_oracle: None,
        behavior_gate: Some("pic24_ka_smoke::pic24f08ka101_uart_smoke"),
    },
    ChipConf {
        name: "pic24f08ka102",
        yaml: "configs/chips/pic24f08ka102.yaml",
        reset_oracle: None,
        behavior_gate: Some("pic24_ka_smoke::pic24f08ka102_uart_smoke"),
    },
    ChipConf {
        name: "pic24f16ka101",
        yaml: "configs/chips/pic24f16ka101.yaml",
        reset_oracle: None,
        behavior_gate: Some("pic24_ka_smoke::pic24f16ka101_uart_smoke"),
    },
    ChipConf {
        name: "pic24f16ka102",
        yaml: "configs/chips/pic24f16ka102.yaml",
        reset_oracle: None,
        behavior_gate: Some("pic24_ka_smoke::pic24f16ka102_uart_smoke"),
    },
];

/// Registers a cold-reset sim model can *never* reproduce from a `reset_halt`
/// silicon capture, with the reason. These are excluded from the match% so the
/// headline measures real cold-reset fidelity, not warm-capture overlap. Any
/// mismatch *outside* these ranges is a genuine model gap and stays counted.
///
/// Inclusive `(start, end, reason)` address ranges, per chip.
fn dynamic_excludes(name: &str) -> &'static [(u64, u64, &'static str)] {
    match name {
        "stm32l073" => &[
            (
                0x40021004,
                0x40021004,
                "RCC_ICSCR: per-die HSI16 factory calibration",
            ),
            (
                0x40021008,
                0x40021008,
                "RCC: clock tree configured before reset_halt (warm)",
            ),
            (
                0x40021030,
                0x40021030,
                "RCC: warm peripheral-enable / clock-ready state",
            ),
            (
                0x4002103c,
                0x4002103c,
                "RCC_CSR: reset-cause flags latched by power-on",
            ),
            (
                0x4000002c,
                0x4000002c,
                "TIM2_ARR: timer clock-gated at reset (APB1 off → reads 0)",
            ),
            (
                0x50000010,
                0x50000010,
                "GPIOA_IDR: PA13 is SWDIO. The capture reads the probe level, not the undriven pull-up",
            ),
        ],
        "esp32c3" => &[
            (
                0x60000000,
                0x6000007f,
                "UART0: ROM-driven boot console — warm post-bootloader state",
            ),
            (
                0x60004038,
                0x6000403f,
                "GPIO: strapping / input pin levels (board state)",
            ),
            (
                0x60008018,
                0x60008018,
                "RTC_CNTL: dynamic reset/clock state",
            ),
            (0x60008038, 0x60008038, "RTC_CNTL: dynamic clock state"),
            (0x60008044, 0x60008044, "RTC_CNTL: dynamic state"),
            (0x60008090, 0x60008090, "RTC_CNTL: dynamic timer state"),
            (
                0x600080a8,
                0x600080b3,
                "RTC_CNTL: per-die analog calibration",
            ),
            (
                0x600080bc,
                0x600080cf,
                "RTC_CNTL: XTAL / sensor calibration",
            ),
            (
                0x60009004,
                0x6000903f,
                "IO_MUX: ROM reconfigured pad pull-up/drive per function (warm)",
            ),
            (
                0x60016000,
                0x600160ff,
                "RMT: clock-gated at reset (silicon reads 0)",
            ),
            (0x6001301c, 0x6001301c, "I2C0: dynamic bus/FSM status"),
            (0x600c0040, 0x600c00ff, "SYSTEM: dynamic status"),
            (
                0x600c2040,
                0x600c20ff,
                "INTERRUPT_CORE: dynamic pending/status",
            ),
        ],
        // ⚠️ ONE entry, and it is not the chip.
        //
        // The first pass at this capture excluded ten registers as
        // "undocumented" or "warm". That was the wrong instinct. A register
        // the vendor header calls RESERVED still has a value on the die, and a
        // twin that answers 0 where silicon answers 0xC00000BC is wrong
        // whether or not anybody wrote the answer down. Nine of the ten are
        // now MODELLED at their measured values — CMU +0x40 and SYSCLKCTRL,
        // the five undocumented IADC words at +0x30..0x40, and IADC +0x90 —
        // and the header-documented gaps the pass had lumped in with them
        // (IADC CFG/SCALE/FIFOCFG, TIMER/USART/I2C IPVERSION, USART
        // FRAME/STATUS/IF, TIMER TOP/TOPB) were fixed, not excluded.
        //
        // What is left is the one value the die reported that the DIE did not
        // author.
        "efr32mg26" => &[(
            0x4003c044,
            0x4003c044,
            "GPIO PORTA_DIN: the probe's own footprint. PA1 reads high with its \
             port mode DISABLED, while PB0/PB1 — buttons, pulled up, also \
             DISABLED — read 0 on the same board, so 'disabled reads 0' is the \
             rule this one pin breaks. GPIO_DBGROUTEPEN enables the debug pins \
             out of reset and they override the port mode; a J-Link was \
             clocking SWD throughout the capture. Not asserted as PA1=SWCLK \
             without the UG594 pinout in hand. Either way DIN is a pad read, \
             not a reset value: the model reproduces the mechanism, and no \
             probe is attached to the twin",
        )],
        _ => &[],
    }
}

fn excluded_reason(name: &str, addr: u64) -> Option<&'static str> {
    dynamic_excludes(name)
        .iter()
        .find(|(lo, hi, _)| addr >= *lo && addr <= *hi)
        .map(|(_, _, why)| *why)
}

// ---------------------------------------------------------------------------
// behavior_gate resolution
//
// `behavior_gate` used to be a hand-typed string that nothing ever opened. Its
// only consumers were `behavior: c.behavior_gate.is_some()` — which feeds
// `level()` and so the committed ratchet baseline — and a `println!` into the
// scoreboard. Three of the seventeen values named a test that does not exist
// (`onboarding-stm32f401cdu6`, `tier1::stm32f411`, `tier1::stm32h735`), so three
// chips were carrying a behaviour claim, and a frozen ratchet floor, on nothing.
// A typo, a renamed test, or a deleted test would all have kept passing.
//
// The strings are now resolved against the tree, and every failure to resolve is
// a hard error. Deleting `firmware_survival::test_esp32c3_demo_survival` (or just
// renaming it) now turns this test red instead of silently keeping esp32c3 at L2.
// ---------------------------------------------------------------------------

/// Where a resolved gate lives: the test source file, and the test function in
/// it when the gate named one.
#[derive(Debug)]
struct GateTarget {
    /// Repo-relative path of the test target source.
    source: String,
}

/// Resolve a `behavior_gate` string to a real test in this repository.
///
/// Accepted forms:
///   * `"<target>"` — `crates/*/tests/<target>.rs` must exist and declare at
///     least one test function.
///   * `"<target>::<fn>"` — that file must additionally declare `fn <fn>` under
///     a test attribute (`#[test]`, `#[tokio::test]`, `#[thumb_oracle_test]`, …).
///
/// Returns `Err` with a human-readable reason when the gate names nothing.
fn resolve_behavior_gate(gate: &str) -> Result<GateTarget, String> {
    let (target, func) = match gate.split_once("::") {
        Some((t, f)) => (t, Some(f)),
        None => (gate, None),
    };
    if target.is_empty() || func == Some("") {
        return Err(format!(
            "malformed gate `{gate}`: expected `target` or `target::test_fn`"
        ));
    }

    // Locate crates/<pkg>/tests/<target>.rs. Integration-test target names are
    // the file stem, so this is the same mapping cargo uses.
    let mut candidates: Vec<PathBuf> = Vec::new();
    let crates_dir = root("crates");
    let mut pkgs: Vec<PathBuf> = std::fs::read_dir(&crates_dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", crates_dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    pkgs.sort();
    for pkg in pkgs {
        let p = pkg.join("tests").join(format!("{target}.rs"));
        if p.is_file() {
            candidates.push(p);
        }
    }
    let Some(path) = candidates.first() else {
        return Err(format!(
            "`{gate}`: no test target `crates/*/tests/{target}.rs` exists in this tree"
        ));
    };
    // Two crates with the same test-target name would make "which test does this
    // gate mean" a coin flip resolved by directory order. Say so instead.
    if candidates.len() > 1 {
        return Err(format!(
            "`{gate}`: ambiguous — {} crates declare a test target `{target}`: {:?}. \
             Qualify the gate or rename one of them.",
            candidates.len(),
            candidates
        ));
    }
    let rel = path
        .strip_prefix(root(""))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string());
    let src = std::fs::read_to_string(path)
        .map_err(|e| format!("`{gate}`: read {}: {e}", path.display()))?;

    match func {
        None => {
            if !src.lines().any(is_test_attr) {
                return Err(format!(
                    "`{gate}`: {rel} exists but declares no test function"
                ));
            }
        }
        Some(f) => {
            if !declares_test_fn(&src, f) {
                return Err(format!(
                    "`{gate}`: {rel} exists but declares no test function `{f}`"
                ));
            }
            if declares_ignored_test_fn(&src, f) {
                return Err(format!(
                    "`{gate}`: {rel} declares `{f}` but marks it ignored — an \
                     ignored test never runs in the PR lane and cannot hold a \
                     chip's level up"
                ));
            }
        }
    }
    Ok(GateTarget { source: rel })
}

/// A line that is a test-ish attribute: `#[test]`, `#[tokio::test]`,
/// `#[thumb_oracle_test]`, `#[rstest]`, …
fn is_test_attr(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("#[") && t.contains("test")
}

/// The contiguous attribute block immediately above each declaration of
/// `name`, in source order, as its trimmed attribute lines. Wrapped attributes
/// contribute all their lines. The walk is local to the declaration: it stops
/// at the first line above that neither opens an attribute nor continues one,
/// so a wrapped string in one function can never bleed into another's block.
fn attr_blocks<'a>(src: &'a str, name: &str) -> Vec<Vec<&'a str>> {
    let lines: Vec<&str> = src.lines().collect();
    let sig = format!("fn {name}(");
    let is_decl = |t: &str| {
        t.starts_with(&sig)
            || t.starts_with(&format!("pub {sig}"))
            || t.starts_with(&format!("async {sig}"))
            || t.starts_with(&format!("pub async {sig}"))
    };
    let mut blocks = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !is_decl(line.trim()) {
            continue;
        }
        let mut block: Vec<&str> = Vec::new();
        let mut j = i;
        while j > 0 {
            j -= 1;
            let t = lines[j].trim();
            if t.is_empty() || t.starts_with("//") {
                continue;
            }
            if t.starts_with("#[") || continues_attribute(&block, t) {
                block.push(t);
                continue;
            }
            break;
        }
        block.reverse();
        blocks.push(block);
    }
    blocks
}

/// True when `candidate` continues an attribute opened in `block` (which is in
/// bottom-up order): joined back into source order, the lines below leave a
/// string literal or a bracket open, so `candidate` cannot end the block.
fn continues_attribute(block: &[&str], candidate: &str) -> bool {
    let mut text = String::from(candidate);
    for line in block.iter().rev() {
        text.push('\n');
        text.push_str(line);
    }
    let (in_string, balance) = attr_state(&text);
    in_string || balance != 0
}

/// Scan `text` across newlines, reporting whether it ends inside a string
/// literal and its net bracket balance. String state carries across newlines —
/// so a `#[ignore = "reason \` + `continued"]` pair balances to zero — and
/// brackets inside strings or `//` comments are ignored.
fn attr_state(text: &str) -> (bool, i32) {
    let mut in_string = false;
    let mut escaped = false;
    let mut in_comment = false;
    let mut balance = 0i32;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_comment {
            if c == '\n' {
                in_comment = false;
            }
        } else if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
        } else if c == '/' && chars.peek() == Some(&'/') {
            in_comment = true;
        } else if c == '[' {
            balance += 1;
        } else if c == ']' {
            balance -= 1;
        }
    }
    (in_string, balance)
}

/// `line` with the contents of string literals removed, so brackets and words
/// inside `"…"` cannot be mistaken for attribute syntax.
fn without_strings(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_string = false;
    let mut escaped = false;
    for c in line.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// `src` with the contents of comments and string/char literals replaced by
/// spaces (newlines preserved), so line-oriented attribute scanning cannot see
/// a `#[ignore]`/`#[test]` that lives inside a comment or a fixture string.
fn mask_comments_and_literals(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            out.push('\n');
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            // Nesting block comments: `/* /* */ */`.
            let mut depth = 0u32;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(if chars[i] == '\n' { '\n' } else { ' ' });
                    i += 1;
                }
            }
        } else if let Some(end) = literal_end(&chars, i) {
            while i < end {
                out.push(if chars[i] == '\n' { '\n' } else { ' ' });
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// The end (exclusive) of a string, raw string, or char literal starting at
/// `chars[start]`, or `None` when no literal starts there (a lifetime's `'`
/// does not). An unterminated literal runs to the end of input.
fn literal_end(chars: &[char], start: usize) -> Option<usize> {
    match *chars.get(start)? {
        'r' => raw_string_end(chars, start),
        'b' if chars.get(start + 1) == Some(&'r') => raw_string_end(chars, start),
        '"' => Some(quoted_end(chars, start, '"')),
        'b' | 'c' if chars.get(start + 1) == Some(&'"') => Some(quoted_end(chars, start + 1, '"')),
        '\'' => char_literal_end(chars, start),
        'b' if chars.get(start + 1) == Some(&'\'') => char_literal_end(chars, start + 1),
        _ => None,
    }
}

/// End of the raw string starting at `chars[start]` (`r"…"`, `r#"…"#`, or
/// `br"…"`), or `None` when this `r`/`br` is not a raw-string opener
/// (`r#ident` is a raw identifier, not a string).
fn raw_string_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    if chars.get(i) == Some(&'b') {
        i += 1;
    }
    if chars.get(i) != Some(&'r') {
        return None;
    }
    i += 1;
    let mut hashes = 0usize;
    while chars.get(i) == Some(&'#') {
        hashes += 1;
        i += 1;
    }
    if chars.get(i) != Some(&'"') {
        return None;
    }
    i += 1;
    while i < chars.len() {
        if chars[i] == '"' && (0..hashes).all(|k| chars.get(i + 1 + k) == Some(&'#')) {
            return Some(i + 1 + hashes);
        }
        i += 1;
    }
    Some(chars.len())
}

/// Index one past the closing `delim` of a quoted literal starting at
/// `chars[quote]`, honoring backslash escapes; end of input when unterminated.
fn quoted_end(chars: &[char], quote: usize, delim: char) -> usize {
    let mut i = quote + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i = (i + 2).min(chars.len()),
            c if c == delim => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

/// End of a char literal (`'x'`, `'\n'`, `'\''`) starting at `chars[quote]`, or
/// `None` when the `'` opens a lifetime/label instead.
fn char_literal_end(chars: &[char], quote: usize) -> Option<usize> {
    if chars.get(quote + 1) == Some(&'\\') || chars.get(quote + 2) == Some(&'\'') {
        Some(quoted_end(chars, quote, '\''))
    } else {
        None
    }
}

/// True when `src` declares `fn name(` (or `async fn name(`) with a test
/// attribute in the attribute block immediately above one of its declarations.
fn declares_test_fn(src: &str, name: &str) -> bool {
    let masked = mask_comments_and_literals(src);
    attr_blocks(&masked, name)
        .iter()
        .any(|block| block.iter().any(|l| is_test_attr(l)))
        || declares_macro_generated_test_fn(&masked, name)
}

/// True when the masked source invokes `survival_gate!(<name>, …)` — the macro
/// expands to a `#[test] fn <name>`, so the generated test is a real,
/// cargo-filterable gate even though the literal `fn <name>(` never appears as
/// text. The first macro argument is the generated function name by
/// construction, so matching it here cannot admit a name that names no test.
fn declares_macro_generated_test_fn(masked: &str, name: &str) -> bool {
    const MACRO: &str = "survival_gate!(";
    let mut rest = masked;
    while let Some(pos) = rest.find(MACRO) {
        let after = rest[pos + MACRO.len()..].trim_start();
        let ident: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if ident == name {
            return true;
        }
        rest = after;
    }
    false
}

/// True when `src` declares `fn name(` whose attribute block marks it ignored —
/// `#[ignore]`, `#[ignore = "…"]`, or `#[cfg_attr(…, ignore …)]`, including an
/// attribute rustfmt has broken across lines. An ignored test never runs in the
/// PR lane, so it cannot be a behavior gate.
fn declares_ignored_test_fn(src: &str, name: &str) -> bool {
    let masked = mask_comments_and_literals(src);
    attr_blocks(&masked, name)
        .iter()
        .any(|block| block.iter().any(|l| has_ignore_token(l)))
}

/// True when `line` contains `ignore` as a standalone token outside string
/// literals, so `#[should_panic(expected = "does not ignore")]` is not read as
/// an ignore attribute.
fn has_ignore_token(line: &str) -> bool {
    without_strings(line)
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|t| t == "ignore")
}

/// The gate for one chip, resolved. Panics (hard, on the ratchet's own path)
/// when the chip claims a gate that names nothing.
fn behavior_gate_target(c: &ChipConf) -> Option<GateTarget> {
    let gate = c.behavior_gate?;
    match resolve_behavior_gate(gate) {
        Ok(t) => Some(t),
        Err(why) => panic!(
            "{}: behavior_gate does not resolve — {why}\n\
             A behavior_gate is a promotion to L1 (or L2 alongside a register \
             match) in `level()` and a frozen floor \
             in docs/coverage/chip-conformance.json. It must name a test that \
             exists: `<target>` or `<target>::<test_fn>` under crates/*/tests/. \
             Either point it at a real gate or set it to None (which demotes the \
             chip — re-baseline with UPDATE_CONFORMANCE_BASELINE=1).",
            c.name
        ),
    }
}

/// Standalone listing of every unresolvable gate, so a reviewer sees all of them
/// at once instead of the first panic. `measure()` enforces the same rule on the
/// ratchet's own path, so deleting this test does not reopen the hole.
#[test]
fn behavior_gates_name_tests_that_exist() {
    let mut bad = Vec::new();
    let mut resolved = Vec::new();
    for c in CHIPS {
        let Some(gate) = c.behavior_gate else {
            continue;
        };
        match resolve_behavior_gate(gate) {
            Ok(t) => resolved.push(format!("  {:<14} {gate} -> {}", c.name, t.source)),
            Err(why) => bad.push(format!("  {:<14} {why}", c.name)),
        }
    }
    println!(
        "behavior gates resolved ({}):\n{}",
        resolved.len(),
        resolved.join("\n")
    );
    assert!(
        bad.is_empty(),
        "{} chip(s) claim a behavior gate that names no test in this tree:\n{}\n\
         A behavior_gate promotes the chip in `level()`; an unresolvable one is a \
         claim resting on a string. Point it at a real test or set it to None.",
        bad.len(),
        bad.join("\n")
    );
}

/// A positive control for the resolver: if `resolve_behavior_gate` degenerated
/// into "always Ok", this test would go red. Pairs with the negative controls in
/// the test above (which only fires when a bad gate is present in CHIPS).
#[test]
fn behavior_gate_resolver_rejects_what_does_not_exist() {
    // Positive: a gate that really exists resolves.
    let ok = resolve_behavior_gate("firmware_survival::test_esp32c3_demo_survival")
        .expect("the esp32c3 survival gate must resolve");
    assert!(
        ok.source
            .ends_with("crates/core/tests/firmware_survival.rs"),
        "resolved to an unexpected file: {}",
        ok.source
    );
    assert!(resolve_behavior_gate("stm32f1_exec_oracle").is_ok());

    // Negative: no such test target.
    assert!(resolve_behavior_gate("onboarding-stm32f401cdu6").is_err());
    assert!(resolve_behavior_gate("no_such_test_target_at_all").is_err());
    // Negative: the target exists, the function does not — the exact shape of
    // the `tier1::stm32f411` / `tier1::stm32h735` claims this PR withdrew.
    assert!(resolve_behavior_gate("tier1::stm32f411").is_err());
    assert!(resolve_behavior_gate("firmware_survival::no_such_case").is_err());
    // Negative: a real *non-test* function in a test file is not a gate.
    assert!(resolve_behavior_gate("firmware_survival::workspace_root").is_err());
    // Negative: a real test that is `#[ignore]`d is not a gate — it never runs
    // in the PR lane. This is the `e2e_esp32_epaper` loophole: the function
    // exists, the resolver used to accept it, and the test never executes.
    assert!(
        resolve_behavior_gate("e2e_esp32_epaper::firmware_drives_panel_to_ereader_bitmap").is_err()
    );
}

/// Unit tests for the attribute-block parsing under the resolver: string
/// fixtures, no filesystem. The multiline shapes are what rustfmt produces for
/// long `ignore` / `cfg_attr(…, ignore …)` attributes, and the quoted-text
/// shapes are the false positives a substring match would produce.
#[test]
fn ignored_gate_detection_handles_multiline_and_quoted_text() {
    let cases: &[(&str, bool)] = &[
        // Plain ignored test.
        (
            r#"#[ignore]
#[test]
fn t() {}
"#,
            true,
        ),
        // Ignored with a reason.
        (
            r#"#[ignore = "needs hardware"]
#[test]
fn t() {}
"#,
            true,
        ),
        // rustfmt breaks a long cfg_attr across lines.
        (
            r#"#[cfg_attr(
    not(feature = "esp-epaper-hw"),
    ignore
)]
#[test]
fn t() {}
"#,
            true,
        ),
        // A reason long enough that the string itself is continued.
        (
            r#"#[ignore = "long reason \
            continued"]
#[test]
fn t() {}
"#,
            true,
        ),
        // A doc comment mentioning the word is not an attribute.
        (
            r#"/// This test does not ignore anything.
#[test]
fn t() {}
"#,
            false,
        ),
        // `ignore` inside a string literal is not the ignore attribute.
        (
            r#"#[test]
#[should_panic(expected = "does not ignore")]
fn t() {}
"#,
            false,
        ),
    ];
    for (src, want_ignored) in cases {
        assert!(
            declares_test_fn(src, "t"),
            "fixture must declare a test fn:\n{src}"
        );
        assert_eq!(
            declares_ignored_test_fn(src, "t"),
            *want_ignored,
            "declares_ignored_test_fn for:\n{src}"
        );
    }
}

/// A local walk must not let one declaration's attributes bleed into another's:
/// the file-global pass this replaced desynced on wrapped strings and both
/// failed open (a plain fn read as a `#[test]`) and false-positived (a plain
/// `#[test]` read as ignored). Two-function fixtures pin both directions.
#[test]
fn ignored_gate_detection_does_not_bleed_across_functions() {
    // Fail-open direction: the wrapped-string attribute belongs to `first`, so
    // plain `t` has no attributes at all and must not inherit `first`'s.
    let fail_open = r#"#[ignore = "reason \
continued"]
#[test]
fn first() {}

fn t() {}
"#;
    assert!(
        !declares_test_fn(fail_open, "t"),
        "fn t has no attributes at all:\n{fail_open}"
    );
    assert!(!declares_ignored_test_fn(fail_open, "t"));

    // False-positive direction: `first`'s wrapped ignore must not be read as
    // `t`'s, even though `t` is itself a plain `#[test]`.
    let false_positive = r#"#[ignore = "reason \
continued"]
fn first() {}

#[test]
fn t() {}
"#;
    assert!(
        declares_test_fn(false_positive, "t"),
        "fn t is a plain #[test]:\n{false_positive}"
    );
    assert!(!declares_ignored_test_fn(false_positive, "t"));

    // `ignore` inside a string literal is not the ignore attribute.
    let quoted = "#[doc = \"ignore\"]\n#[test]\nfn t() {}\n";
    assert!(!declares_ignored_test_fn(quoted, "t"));
}

/// Comments and fixture strings must be invisible to the scanner: a standalone
/// block comment between the attributes used to break the walk (leaving a
/// never-run test looking gated), and `#[test]`/`#[ignore]` text inside a
/// comment or a string literal must not fabricate a test.
#[test]
fn ignored_gate_detection_masks_comments_and_literals() {
    // A block comment between the attributes must not break the walk.
    let block_comment_between = r#"#[ignore]
/* parked pending bench */
#[test]
fn t() {}
"#;
    assert!(declares_test_fn(block_comment_between, "t"));
    assert!(declares_ignored_test_fn(block_comment_between, "t"));

    // `#[test]` inside a block comment is not a test attribute.
    let test_in_block_comment = r#"/*
#[test] */
fn t() {}
"#;
    assert!(!declares_test_fn(test_in_block_comment, "t"));

    // Text inside a normal string literal is not source.
    let test_in_string = r#"const S: &str = "\
#[test]\
fn t() {}
";

fn t() {}
"#;
    assert!(!declares_test_fn(test_in_string, "t"));

    // Text inside a raw string literal is not source either. Built line by line
    // so this fixture's `#[ignore]` does not itself start a source line, which
    // the ignored-test inventory scanner would otherwise collect as real.
    let ignore_in_raw_string = [
        "const S: &str = r#\"",
        "#[ignore]",
        "fn t() {}",
        "\"#;",
        "",
        "fn t() {}",
    ]
    .join("\n");
    assert!(!declares_test_fn(&ignore_in_raw_string, "t"));
    assert!(!declares_ignored_test_fn(&ignore_in_raw_string, "t"));
}

fn dummy_manifest(path: &str) -> SystemManifest {
    SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "chip-conformance".to_string(),
        chip: path.to_string(),
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
        // No override: these harnesses take whatever the chip declares.
        cpu_hz: None,
    }
}

#[derive(Debug, Clone)]
struct Record {
    estate_ok: bool,
    peripherals: usize,
    /// Verifiable (deterministic cold-reset) registers in the capture.
    reg_total: usize,
    /// Verifiable registers the sim reproduces exactly.
    reg_match: usize,
    /// Registers excluded as physically un-reproducible (calibration, gated,
    /// warm-configured, live status) — see `dynamic_excludes`.
    excluded: usize,
    behavior: bool,
}

/// Run the uniform battery for one chip.
fn measure(c: &ChipConf) -> Record {
    let abs = root(c.yaml);
    let abs_str = abs.to_string_lossy().to_string();
    let chip =
        ChipDescriptor::from_file(&abs).unwrap_or_else(|e| panic!("{}: load chip: {e}", c.name));
    let peripherals = chip.peripherals.len();
    let bus = SystemBus::from_config(&chip, &dummy_manifest(&abs_str))
        .unwrap_or_else(|e| panic!("{}: build bus: {e}", c.name));

    // Estate: every wired peripheral's base reads without a bus fault.
    let estate_ok = chip
        .peripherals
        .iter()
        .all(|p| bus.read_u32(p.base_address).is_ok());

    // Registers vs silicon: how much of the deterministic cold-reset state the
    // sim reproduces. Registers a cold model physically can't reproduce from a
    // warm `reset_halt` capture (calibration, gated, console, live status) are
    // excluded with a reason; everything else is verifiable, and a mismatch
    // there is a real model gap.
    let report = std::env::var("CONFORMANCE_REPORT").is_ok();
    let (mut reg_total, mut reg_match, mut excluded) = (0usize, 0usize, 0usize);
    if let Some(oracle) = c.reset_oracle {
        if let Ok(text) = std::fs::read_to_string(root(oracle)) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                // ⚠️ REPLAY THE CAPTURE'S PREAMBLE INTO THE SIM.
                //
                // A capture may have had to write registers before it could
                // read anything — on EFR32 a peripheral that is not clocked
                // FAULTS the debug port rather than reading zero, so its oracle
                // un-gates CLKEN0 first. Diffing that against a sim still in
                // its cold reset state compares an un-gated die with a gated
                // model, and every clock-gated peripheral reads 0 in the sim
                // and non-zero on silicon. That is the gating working, not a
                // model gap, and counting it as one buries the real gaps in
                // noise.
                //
                // So the sim is put into the SAME state the capture was taken
                // in. Nothing else about the comparison changes.
                let mut bus = bus;
                if let Some(pre) = json.get("preamble").and_then(|p| p.as_array()) {
                    for step in pre {
                        let Some(w) = step.get("write32") else {
                            continue;
                        };
                        let addr = w.get("address").and_then(|a| a.as_str()).map(parse_hex);
                        let val = w.get("value").and_then(|v| v.as_str()).map(parse_hex32);
                        if let (Some(a), Some(v)) = (addr, val) {
                            let _ = bus.write_u32(a, v);
                        }
                    }
                }
                let bus = bus;
                if let Some(blocks) = json.get("blocks").and_then(|b| b.as_object()) {
                    for block in blocks.values() {
                        if let Some(words) = block.get("words").and_then(|w| w.as_object()) {
                            for (addr, val) in words {
                                let a = parse_hex(addr);
                                let v = val.as_str().map(parse_hex32).unwrap_or(0);
                                if let Some(why) = excluded_reason(c.name, a) {
                                    excluded += 1;
                                    if report {
                                        if let Ok(got) = bus.read_u32(a) {
                                            if got != v {
                                                eprintln!(
                                                    "  [{}] EXCLUDED 0x{a:08x}: sim=0x{got:08x} silicon=0x{v:08x} ({why})",
                                                    c.name
                                                );
                                            }
                                        }
                                    }
                                    continue;
                                }
                                reg_total += 1;
                                match bus.read_u32(a) {
                                    Ok(got) if got == v => reg_match += 1,
                                    Ok(got) => {
                                        if report {
                                            eprintln!(
                                                "  [{}] REAL-GAP 0x{a:08x}: sim=0x{got:08x} silicon=0x{v:08x}",
                                                c.name
                                            );
                                        }
                                    }
                                    Err(_) => {
                                        if report {
                                            eprintln!(
                                                "  [{}] BUS-FAULT 0x{a:08x}: silicon=0x{v:08x} (sim read faulted)",
                                                c.name
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Behavior: the chip has a running-firmware gate AND that gate names a test
    // that actually exists. `behavior_gate_target` panics on an unresolvable
    // string, so a fictional gate can never reach `level()` — this is on the
    // ratchet's own path, not a side test.
    let behavior = behavior_gate_target(c).is_some();

    Record {
        estate_ok,
        peripherals,
        reg_total,
        reg_match,
        excluded,
        behavior,
    }
}

fn parse_hex(s: &str) -> u64 {
    u64::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap_or(0)
}
fn parse_hex32(s: &str) -> u32 {
    u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap_or(0)
}

/// A chip's conformance level: L0 estate, L1 estate + (registers-vs-silicon OR
/// a behavior gate), L2 both.
fn level(r: &Record) -> u8 {
    if !r.estate_ok {
        return 0;
    }
    let has_reg = r.reg_total > 0 && r.reg_match * 100 >= r.reg_total * 50;
    match (has_reg, r.behavior) {
        (true, true) => 2,
        (true, false) | (false, true) => 1,
        (false, false) => 0,
    }
}

fn is_declared_only_prototype(name: &str, chip: &ChipDescriptor) -> bool {
    match name {
        "rv1103" => {
            chip.name == "rv1103"
                && matches!(chip.arch, labwired_config::Arch::Arm)
                && chip.core.as_deref() == Some("cortex-a7")
                && chip.peripherals.is_empty()
                && chip.flash.size == 0
        }
        _ => false,
    }
}

#[test]
fn declared_only_classification_rejects_other_chips_or_modeled_platforms() {
    assert_eq!(DECLARED_ONLY_CHIPS.len(), 1);
    assert!(DECLARED_ONLY_CHIPS.iter().any(|(n, _)| *n == "rv1103"));
    let chip = ChipDescriptor::from_file(root("configs/chips/rv1103.yaml")).unwrap();
    assert!(is_declared_only_prototype("rv1103", &chip));
    let mut changed = chip.clone();
    changed.name = "another-prototype".into();
    assert!(!is_declared_only_prototype("rv1103", &changed));
    changed = chip.clone();
    changed.core = Some("cortex-m7".into());
    assert!(!is_declared_only_prototype("rv1103", &changed));
    changed = chip.clone();
    changed.flash.size = 1;
    assert!(!is_declared_only_prototype("rv1103", &changed));
    changed = chip;
    changed.peripherals = ChipDescriptor::from_file(root("configs/chips/stm32f103.yaml"))
        .unwrap()
        .peripherals;
    assert!(!is_declared_only_prototype("rv1103", &changed));
}

#[test]
fn rv1103_declaration_is_visible_without_model_conformance() {
    let chip = ChipDescriptor::from_file(root("configs/chips/rv1103.yaml")).unwrap();
    assert_eq!(chip.core.as_deref(), Some("cortex-a7"));
    assert!(chip.peripherals.is_empty());
    assert!(!CHIPS.iter().any(|chip| chip.name == "rv1103"));
    let board = std::fs::read_to_string(root("docs/coverage/chip-conformance.md")).unwrap();
    assert!(board.contains("| rv1103 | Declared only |"));
    assert!(!board.contains("| rv1103 | **L"));
    let baseline: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root("docs/coverage/chip-conformance.json")).unwrap(),
    )
    .unwrap();
    assert!(!baseline
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["name"] == "rv1103"));
}

#[test]
fn chip_conformance_ratchet() {
    // Every descriptor must be accounted for in exactly one visible class.
    let configured: Vec<String> = std::fs::read_dir(root("configs/chips"))
        .expect("configs/chips")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".yaml"))
        .map(|n| n.trim_end_matches(".yaml").to_string())
        .filter(|n| !n.contains("ci-fixture"))
        .collect();
    for chip in &configured {
        let modeled = CHIPS.iter().any(|c| c.name == chip);
        let declared = DECLARED_ONLY_CHIPS.iter().any(|(name, _)| name == chip);
        assert!(
            modeled ^ declared,
            "chip '{chip}' must have exactly one modeled conformance row or validated declaration"
        );
    }
    for (name, reason) in DECLARED_ONLY_CHIPS {
        assert!(configured.iter().any(|chip| chip == name));
        assert!(!reason.is_empty());
        let chip = ChipDescriptor::from_file(root(&format!("configs/chips/{name}.yaml")))
            .unwrap_or_else(|error| panic!("{name}: load declared prototype: {error}"));
        assert!(
            is_declared_only_prototype(name, &chip),
            "{name}: declaration no longer matches the declared-only prototype; classify its platform explicitly"
        );
    }

    let mut rows = Vec::new();
    let mut board = String::from(
        "# Chip Conformance Scoreboard\n\n\
         Generated by `chip_conformance_ratchet`. L0 estate · L1 estate + (registers-vs-silicon OR a behavior gate) · L2 both.\n\n\
         Reg match = verifiable cold-reset registers reproduced. \"Excluded\" = registers a cold \
         model can't reproduce from a warm capture (calibration / clock-gated / boot-console / live \
         status); see `dynamic_excludes`. A mismatch outside the excluded set is a real model gap.\n\n\
         | Chip | Level | Estate | Peripherals | Reg match (verifiable) | Excluded | Behavior gate |\n\
         |------|-------|--------|-------------|------------------------|----------|---------------|\n",
    );
    for c in CHIPS {
        let r = measure(c);
        let lvl = level(&r);
        let reg = if r.reg_total > 0 {
            format!(
                "{}/{} ({}%)",
                r.reg_match,
                r.reg_total,
                r.reg_match * 100 / r.reg_total
            )
        } else {
            "—".to_string()
        };
        let exc = if r.excluded > 0 {
            r.excluded.to_string()
        } else {
            "—".to_string()
        };
        let beh = c.behavior_gate.unwrap_or("—");
        board.push_str(&format!(
            "| {} | **L{}** | {} | {} | {} | {} | {} |\n",
            c.name,
            lvl,
            if r.estate_ok { "✓" } else { "✗" },
            r.peripherals,
            reg,
            exc,
            beh,
        ));
        rows.push((c.name.to_string(), lvl, r));
    }
    if !DECLARED_ONLY_CHIPS.is_empty() {
        board.push_str(
            "\n## Declared platforms without a model\n\n\
             These descriptors carry declarations only. They receive no estate checkmark, \
             conformance level or measured ratchet record.\n\n\
             | Chip | State | Evidence boundary |\n\
             |------|-------|-------------------|\n",
        );
        for (name, reason) in DECLARED_ONLY_CHIPS {
            board.push_str(&format!("| {name} | Declared only | {reason} |\n"));
        }
    }

    // The board is a COMMITTED artifact, so regenerate it only when explicitly
    // asked and otherwise CHECK it — the same contract
    // `scripts/generate_validation_status.py --check` holds.
    //
    // This used to be an unconditional `fs::write(..).ok()`: running the test
    // silently rewrote a tracked file and passed, so the committed board could
    // (and did) drift from reality — nrf52832 read 10 verified registers when
    // the model reproduced 16, stm32f407 read 29 against 31. Drift in the
    // OPTIMISTIC direction at that: the doc UNDER-sold the chips, and nothing
    // failed to say so. A generator that overwrites its own expectation cannot
    // be a gate.
    let board_path = root("docs/coverage/chip-conformance.md");
    if std::env::var("UPDATE_CONFORMANCE_BASELINE").is_ok() {
        std::fs::write(&board_path, &board).expect("write conformance board");
        println!("updated conformance board: {}", board_path.display());
    } else {
        let committed = std::fs::read_to_string(&board_path).unwrap_or_else(|e| {
            panic!(
                "read {}: {e} — regenerate with UPDATE_CONFORMANCE_BASELINE=1",
                board_path.display()
            )
        });
        assert_eq!(
            committed.trim_end(),
            board.trim_end(),
            "docs/coverage/chip-conformance.md is stale — the measured board no \
             longer matches the committed one. Regenerate in this commit with \
             `UPDATE_CONFORMANCE_BASELINE=1 cargo test -p labwired-core --test \
             chip_conformance` and review the diff: a FALLING reg-match count is \
             a model regression (the ratchet below fails on it), a RISING one is \
             coverage the doc has not been told about yet."
        );
    }

    // Ratchet against the committed baseline: estate may not break, level may not
    // drop, reg-match count may not fall.
    let baseline_path = root("docs/coverage/chip-conformance.json");
    let current: serde_json::Value = serde_json::json!(rows
        .iter()
        .map(|(name, lvl, r)| {
            serde_json::json!({"name": name, "level": lvl, "reg_match": r.reg_match, "excluded": r.excluded, "behavior": r.behavior})
        })
        .collect::<Vec<_>>());

    if std::env::var("UPDATE_CONFORMANCE_BASELINE").is_ok() {
        std::fs::write(
            &baseline_path,
            serde_json::to_string_pretty(&current).unwrap(),
        )
        .expect("write baseline");
        println!("updated conformance baseline: {}", baseline_path.display());
        return;
    }

    let baseline: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&baseline_path).unwrap_or_else(|_| {
            panic!(
                "missing {}; create it with UPDATE_CONFORMANCE_BASELINE=1",
                baseline_path.display()
            )
        }),
    )
    .expect("parse baseline");

    let mut failures = Vec::new();
    for (name, lvl, r) in &rows {
        let base = baseline.as_array().and_then(|a| {
            a.iter()
                .find(|b| b.get("name").and_then(|n| n.as_str()) == Some(name))
        });
        let Some(base) = base else { continue };
        let base_lvl = base.get("level").and_then(|l| l.as_u64()).unwrap_or(0) as u8;
        let base_match = base.get("reg_match").and_then(|m| m.as_u64()).unwrap_or(0) as usize;
        if *lvl < base_lvl {
            failures.push(format!("  {name}: level L{lvl} < baseline L{base_lvl}"));
        }
        if r.reg_match < base_match {
            failures.push(format!(
                "  {name}: reg match {} < baseline {base_match}",
                r.reg_match
            ));
        }
        if !r.estate_ok {
            failures.push(format!(
                "  {name}: estate broken (a peripheral window faults)"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "chip conformance regressed ({} issue(s)):\n{}\n(intentional? re-baseline with UPDATE_CONFORMANCE_BASELINE=1)",
        failures.len(),
        failures.join("\n")
    );
}
