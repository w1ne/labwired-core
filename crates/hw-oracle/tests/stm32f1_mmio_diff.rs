// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! STM32F103 (Nucleo-F103RB / Blue-Pill) MMIO peripheral diff oracle.
//!
//! Register-level cross-validation of the F1 ("classic"/legacy) peripheral
//! models against real STM32F103 silicon over ST-Link SWD. This is the F1
//! board's independent golden gate, the F1 counterpart to
//! `stm32l0_mmio_diff.rs` / `l476_mmio_diff.rs`: it pins F1-specific register
//! semantics so a change to any shared model that would leak into the F1 board
//! is caught here, per-board:
//!   * the F1 RCC layout (CR reset 0x0000_4A83, AHBENR@0x14, APB2ENR@0x18,
//!     APB1ENR@0x1C, source-ready-gated SW->SWS),
//!   * the F1 GPIO layout (CRL/CRH config words, ODR@0x0C, BSRR@0x10, BRR@0x14),
//!   * the *classic* SPI (CR2 reset 0x0000 — no DS field),
//!   * the F1 I²C reset state (TRISE reset 0x0002),
//!   * the Cortex-M3 DBGMCU @0xE004_2000.
//!
//! Philosophy mirrors the other MMIO diff gates: no instruction execution, just
//! register access against the modeled peripherals on both sides, then a masked
//! comparison. Two flavours of case:
//!   * `RESET_CASES` — read a register at fresh reset (only RCC clock-enables as
//!     preamble, which do not perturb a peripheral's own reset value) and pin
//!     its silicon-confirmed reset value.
//!   * `CASES` — write-then-read R/W diffs.
//!
//! ## Running
//!
//! Sim-only (no hardware — runs in normal CI, the per-board isolation gate):
//! ```text
//! cargo test -p labwired-hw-oracle --test stm32f1_mmio_diff f1_
//! ```
//!
//! Full sim-vs-hardware diff (STM32F103 on its ST-Link; with multiple ST-Links
//! attached, pick the F1 probe by serial):
//! ```text
//! LABWIRED_STLINK_SERIAL=<f103-stlink-serial> \
//!   cargo test -p labwired-hw-oracle --test stm32f1_mmio_diff \
//!     --features hw-oracle-stm32 -- --ignored --nocapture
//! ```
//! Set `F103_STRICT=1` to make register divergence a hard failure.
//!
//! Hardware validated against: STM32F103 medium-density (Cortex-M3, DBGMCU
//! IDCODE 0x2003_6410, chipid 0x410, 128 KiB flash / 20 KiB SRAM).

// Register-map style: `BASE + 0x00` and `1 << 0` document offsets/bit positions
// literally — keep them rather than collapsing to bare values.
#![allow(clippy::identity_op)]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

// ── RCC (0x4002_1000, RM0008 §7 — F1 layout) ─────────────────────────────────
const RCC_BASE: u32 = 0x4002_1000;
const RCC_CR: u32 = RCC_BASE + 0x00;
const RCC_CFGR: u32 = RCC_BASE + 0x04;
const RCC_AHBENR: u32 = RCC_BASE + 0x14; // DMA1/CRC
const RCC_APB2ENR: u32 = RCC_BASE + 0x18; // AFIO/GPIO/ADC/TIM1/SPI1/USART1
const RCC_APB1ENR: u32 = RCC_BASE + 0x1C; // TIM2-4/SPI2/USART2-3/I2C1-2
const RCC_CIR: u32 = RCC_BASE + 0x08; // clock-interrupt flags/enables

// AHBENR bits
const DMA1EN: u32 = 1 << 0;
const CRCEN: u32 = 1 << 6;
// APB2ENR bits
const IOPAEN: u32 = 1 << 2;
const IOPBEN: u32 = 1 << 3;
const IOPCEN: u32 = 1 << 4;
const ADC1EN: u32 = 1 << 9;
const SPI1EN: u32 = 1 << 12;
// APB1ENR bits
const TIM2EN: u32 = 1 << 0;
const USART2EN: u32 = 1 << 17;
const I2C1EN: u32 = 1 << 21;

// CR reset (silicon): HSION bit0 + HSIRDY bit1 set, HSICAL factory trim in bits
// 8-15 reads 0x4A on this part → 0x0000_4A83.
const RCC_CR_RESET: u32 = 0x0000_4A83;
// CFGR PPRE1 (APB1 prescaler) bits [10:8]; 0b100 = /2. Plain R/W config bits,
// not gated on a clock-source switch, so safe to set without disturbing SYSCLK.
const CFGR_PPRE1_MASK: u32 = 0b111 << 8;
const CFGR_PPRE1_DIV2: u32 = 0b100 << 8;

// ── GPIO (F1 layout: CRL/CRH config words) ───────────────────────────────────
// GPIOB has no SWD pins (SWD is PA13/PA14); PB3/PB4 are JTDO/NJTRST and read a
// debug-dependent reset state, so we drive PB5 (free) and never assert reset
// values on CRL/CRH here.
const GPIOB_BASE: u32 = 0x4001_0C00;
const GPIO_CRL: u32 = 0x00;
const GPIO_CRH: u32 = 0x04;
const GPIO_ODR: u32 = 0x0C;
const GPIO_BSRR: u32 = 0x10;
const GPIO_BRR: u32 = 0x14;
const GPIOB_ODR: u32 = GPIOB_BASE + GPIO_ODR;
const GPIOB_BSRR: u32 = GPIOB_BASE + GPIO_BSRR;
const GPIOB_BRR: u32 = GPIOB_BASE + GPIO_BRR;
const PB5: u32 = 1 << 5;

// ── SPI1 (0x4001_3000 — classic SPI: no CR2.DS) ──────────────────────────────
const SPI1_BASE: u32 = 0x4001_3000;
const SPI1_CR1: u32 = SPI1_BASE + 0x00;
const SPI1_CR2: u32 = SPI1_BASE + 0x04;
const SPI1_CRCPR: u32 = SPI1_BASE + 0x10;
const SPI1_I2SCFGR: u32 = SPI1_BASE + 0x1C;
const SPI1_I2SPR: u32 = SPI1_BASE + 0x20;

// ── I2C1 (0x4000_5400, RM0008 §26) ───────────────────────────────────────────
const I2C1_BASE: u32 = 0x4000_5400;
const I2C1_CR1: u32 = I2C1_BASE + 0x00;
const I2C1_CR2: u32 = I2C1_BASE + 0x04;
const I2C1_OAR1: u32 = I2C1_BASE + 0x08;
const I2C1_OAR2: u32 = I2C1_BASE + 0x0C;
const I2C1_CCR: u32 = I2C1_BASE + 0x1C;
const I2C1_TRISE: u32 = I2C1_BASE + 0x20;
const I2C_TRISE_RESET: u32 = 0x0002; // silicon-confirmed (RM0008 §26.6.9)

// ── TIM2 (0x4000_0000, 16-bit GP timer) ──────────────────────────────────────
const TIM2_BASE: u32 = 0x4000_0000;
const TIM2_CR1: u32 = TIM2_BASE + 0x00;
const TIM2_CR2: u32 = TIM2_BASE + 0x04;
const TIM2_SMCR: u32 = TIM2_BASE + 0x08;
const TIM2_DIER: u32 = TIM2_BASE + 0x0C;
const TIM2_CCMR1: u32 = TIM2_BASE + 0x18;
const TIM2_CCMR2: u32 = TIM2_BASE + 0x1C;
const TIM2_CCER: u32 = TIM2_BASE + 0x20;
const TIM2_CNT: u32 = TIM2_BASE + 0x24;
const TIM2_PSC: u32 = TIM2_BASE + 0x28;
const TIM2_ARR: u32 = TIM2_BASE + 0x2C;
const TIM2_CCR1: u32 = TIM2_BASE + 0x34;
const TIM2_CCR2: u32 = TIM2_BASE + 0x38;
const TIM2_CCR3: u32 = TIM2_BASE + 0x3C;
const TIM2_CCR4: u32 = TIM2_BASE + 0x40;
const TIM2_DCR: u32 = TIM2_BASE + 0x48;
const TIM2_ARR_RESET: u32 = 0x0000_FFFF; // silicon-confirmed (16-bit reload)

// ── ADC1 (0x4001_2400, RM0008 §11) ───────────────────────────────────────────
const ADC1_BASE: u32 = 0x4001_2400;
const ADC1_CR1: u32 = ADC1_BASE + 0x04;
const ADC1_CR2: u32 = ADC1_BASE + 0x08;

// ── EXTI (0x4001_0400 — always clocked) ──────────────────────────────────────
// AFIO (0x4001_0000, RM0008 §9) — F1-only alternate-function I/O remapping.
const AFIO_BASE: u32 = 0x4001_0000;
const AFIOEN: u32 = 1 << 0; // RCC APB2ENR
const AFIO_EVCR: u32 = AFIO_BASE + 0x00;
const AFIO_MAPR: u32 = AFIO_BASE + 0x04;
const AFIO_EXTICR1: u32 = AFIO_BASE + 0x08;
const AFIO_EXTICR2: u32 = AFIO_BASE + 0x0C;
const AFIO_EXTICR3: u32 = AFIO_BASE + 0x10;
const AFIO_EXTICR4: u32 = AFIO_BASE + 0x14;
const AFIO_MAPR2: u32 = AFIO_BASE + 0x1C;

const EXTI_BASE: u32 = 0x4001_0400;
const EXTI_IMR: u32 = EXTI_BASE + 0x00;
const EXTI_EMR: u32 = EXTI_BASE + 0x04;
const EXTI_RTSR: u32 = EXTI_BASE + 0x08;
const EXTI_FTSR: u32 = EXTI_BASE + 0x0C;

// ── DBGMCU identity (Cortex-M3: APB @ 0xE004_2000) ───────────────────────────
const DBGMCU_IDCODE: u32 = 0xE004_2000;
const F103_IDCODE: u32 = 0x2003_6410; // silicon-read DEV_ID 0x410, REV_ID 0x2003

// ── IWDG (0x4000_3000 — independent watchdog, always-on LSI domain) ───────────
// Registers read without any RCC clock-enable (the IWDG sits in the always-on
// domain). Reset values silicon-confirmed on the bench F103 (RM0008 §19).
const IWDG_BASE: u32 = 0x4000_3000;
const IWDG_PR: u32 = IWDG_BASE + 0x04;
const IWDG_RLR: u32 = IWDG_BASE + 0x08;
const IWDG_SR: u32 = IWDG_BASE + 0x0C;
const IWDG_RLR_RESET: u32 = 0x0000_0FFF; // 12-bit reload, all ones at reset

// ── WWDG (0x4000_2C00 — window watchdog, APB1) ───────────────────────────────
const WWDG_BASE: u32 = 0x4000_2C00;
const WWDG_CR: u32 = WWDG_BASE + 0x00;
const WWDG_CFR: u32 = WWDG_BASE + 0x04;
const WWDG_RESET: u32 = 0x0000_007F; // CR and CFR both reset to 0x7F
const WWDGEN: u32 = 1 << 11; // RCC APB1ENR

// ── USART2 (0x4000_4400 — APB1) ──────────────────────────────────────────────
const USART2_BASE: u32 = 0x4000_4400;
const USART2_SR: u32 = USART2_BASE + 0x00;
const USART2_SR_RESET: u32 = 0x0000_00C0; // TXE | TC set out of reset
const USART2_BRR: u32 = USART2_BASE + 0x08;
const USART2_CR1: u32 = USART2_BASE + 0x0C;
const USART2_CR2: u32 = USART2_BASE + 0x10;
const USART2_CR3: u32 = USART2_BASE + 0x14;
const USART2_GTPR: u32 = USART2_BASE + 0x18;

// ── DMA1 (0x4002_0000 — AHB, 7-channel controller) ───────────────────────────
const DMA1_BASE: u32 = 0x4002_0000;
const DMA1_ISR: u32 = DMA1_BASE + 0x00; // interrupt status
const DMA1_CCR1: u32 = DMA1_BASE + 0x08; // channel-1 config
const DMA1_CNDTR1: u32 = DMA1_BASE + 0x0C; // channel-1 count
const DMA1_CPAR1: u32 = DMA1_BASE + 0x10; // channel-1 peripheral addr
const DMA1_CMAR1: u32 = DMA1_BASE + 0x14; // channel-1 memory addr
const DMA1_CNDTR2: u32 = DMA1_BASE + 0x20; // channel-2 count
const DMA1_CPAR2: u32 = DMA1_BASE + 0x24; // channel-2 peripheral addr
const DMA1_CMAR2: u32 = DMA1_BASE + 0x28; // channel-2 memory addr

// ── RTC (0x4000_2800 — F1 backup-domain RTC) ─────────────────────────────────
// Register access needs the PWR + BKP APB1 clocks; the CRH/CNT values below are
// independent of the LSI/sync timing (silicon-confirmed on the bench). CRL is
// deliberately not pinned — see the note at its RESET_CASES slot.
const RTC_BASE: u32 = 0x4000_2800;
const RTC_CRH: u32 = RTC_BASE + 0x00;
const RTC_CNTH: u32 = RTC_BASE + 0x18;
const RTC_CNTL: u32 = RTC_BASE + 0x1C;
const PWREN: u32 = 1 << 28; // RCC APB1ENR
const BKPEN: u32 = 1 << 27; // RCC APB1ENR

// ── Reset-value cases ─────────────────────────────────────────────────────────
// Read at fresh reset. `prep` is RCC clock-enables ONLY (a peripheral must be
// clocked for its registers to read on silicon); enabling a clock does not alter
// the peripheral's own reset value, so the comparison stays honest.

struct ResetCase {
    label: &'static str,
    prep: &'static [(u32, u32)],
    read_addr: u32,
    mask: u32,
    expect: u32,
}

const RESET_CASES: &[ResetCase] = &[
    ResetCase {
        label: "RCC.CR reset = 0x00004A83 (HSION|HSIRDY|HSICAL)",
        prep: &[],
        read_addr: RCC_CR,
        mask: 0xFFFF,
        expect: RCC_CR_RESET,
    },
    ResetCase {
        label: "RCC.CFGR reset = 0 (SW/SWS=HSI)",
        prep: &[],
        read_addr: RCC_CFGR,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    ResetCase {
        label: "SPI1.CR1 reset = 0",
        prep: &[(RCC_APB2ENR, SPI1EN)],
        read_addr: SPI1_CR1,
        mask: 0xFFFF,
        expect: 0,
    },
    ResetCase {
        // Classic SPI: CR2 resets to 0x0000 (no DS/FRXTH bits like the L4 FIFO
        // variant, which resets to 0x0700).
        label: "SPI1.CR2 reset = 0 (classic, no DS)",
        prep: &[(RCC_APB2ENR, SPI1EN)],
        read_addr: SPI1_CR2,
        mask: 0xFFFF,
        expect: 0,
    },
    ResetCase {
        label: "I2C1.CR2 reset = 0",
        prep: &[(RCC_APB1ENR, I2C1EN)],
        read_addr: I2C1_CR2,
        mask: 0xFFFF,
        expect: 0,
    },
    ResetCase {
        // The headline F1 I²C reset fidelity item: TRISE resets to 0x0002.
        label: "I2C1.TRISE reset = 0x0002",
        prep: &[(RCC_APB1ENR, I2C1EN)],
        read_addr: I2C1_TRISE,
        mask: 0xFFFF,
        expect: I2C_TRISE_RESET,
    },
    ResetCase {
        label: "TIM2.ARR reset = 0xFFFF (16-bit reload)",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        read_addr: TIM2_ARR,
        mask: 0xFFFF,
        expect: TIM2_ARR_RESET,
    },
    ResetCase {
        label: "TIM2.CR1 reset = 0",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        read_addr: TIM2_CR1,
        mask: 0xFFFF,
        expect: 0,
    },
    ResetCase {
        label: "ADC1.CR1 reset = 0",
        prep: &[(RCC_APB2ENR, ADC1EN)],
        read_addr: ADC1_CR1,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    ResetCase {
        label: "ADC1.CR2 reset = 0",
        prep: &[(RCC_APB2ENR, ADC1EN)],
        read_addr: ADC1_CR2,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    ResetCase {
        label: "EXTI.IMR reset = 0 (all lines masked)",
        prep: &[],
        read_addr: EXTI_IMR,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    // ── WDT class: IWDG + WWDG (silicon-probed on the bench F103) ──
    ResetCase {
        label: "IWDG.PR reset = 0 (prescaler /4)",
        prep: &[],
        read_addr: IWDG_PR,
        mask: 0x7,
        expect: 0,
    },
    ResetCase {
        label: "IWDG.RLR reset = 0xFFF (12-bit reload)",
        prep: &[],
        read_addr: IWDG_RLR,
        mask: 0xFFF,
        expect: IWDG_RLR_RESET,
    },
    ResetCase {
        label: "IWDG.SR reset = 0 (no pending updates)",
        prep: &[],
        read_addr: IWDG_SR,
        mask: 0x3,
        expect: 0,
    },
    ResetCase {
        label: "WWDG.CR reset = 0x7F",
        prep: &[(RCC_APB1ENR, WWDGEN)],
        read_addr: WWDG_CR,
        mask: 0xFF,
        expect: WWDG_RESET,
    },
    ResetCase {
        label: "WWDG.CFR reset = 0x7F",
        prep: &[(RCC_APB1ENR, WWDGEN)],
        read_addr: WWDG_CFR,
        mask: 0x1FF,
        expect: WWDG_RESET,
    },
    // ── USART register-level reset (matrix uart cell was firmware-only) ──
    ResetCase {
        label: "USART2.SR reset = 0xC0 (TXE|TC)",
        prep: &[(RCC_APB1ENR, USART2EN)],
        read_addr: USART2_SR,
        mask: 0xFF,
        expect: USART2_SR_RESET,
    },
    // ── DMA1 channel-1 reset state (matrix dma cell at register level) ──
    ResetCase {
        label: "DMA1.ISR reset = 0 (no pending flags)",
        prep: &[(RCC_AHBENR, DMA1EN)],
        read_addr: DMA1_ISR,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    ResetCase {
        label: "DMA1.CCR1 reset = 0 (channel disabled)",
        prep: &[(RCC_AHBENR, DMA1EN)],
        read_addr: DMA1_CCR1,
        mask: 0xFFFF,
        expect: 0,
    },
    ResetCase {
        label: "DMA1.CNDTR1 reset = 0",
        prep: &[(RCC_AHBENR, DMA1EN)],
        read_addr: DMA1_CNDTR1,
        mask: 0xFFFF,
        expect: 0,
    },
    ResetCase {
        label: "DMA1.CPAR1 reset = 0",
        prep: &[(RCC_AHBENR, DMA1EN)],
        read_addr: DMA1_CPAR1,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    // ── RTC (backup domain; PWR+BKP clocks gate register access) ──
    ResetCase {
        label: "RTC.CRH reset = 0 (no interrupts enabled)",
        prep: &[(RCC_APB1ENR, PWREN | BKPEN)],
        read_addr: RTC_CRH,
        mask: 0xF,
        expect: 0,
    },
    // NOTE: RTC.CRL is intentionally NOT pinned here. Cold silicon reads 0
    // (RTOFF/RSF only latch once the RTC is clocked + synced), while the sim's
    // RTC model returns 0x2101 — a real but state/clock-dependent discrepancy,
    // not a clean reset value. Reconciling the F1 RTC operational-state model
    // vs silicon is tracked as a follow-up rather than forced into a reset diff.
    ResetCase {
        label: "RTC.CNTH reset = 0",
        prep: &[(RCC_APB1ENR, PWREN | BKPEN)],
        read_addr: RTC_CNTH,
        mask: 0xFFFF,
        expect: 0,
    },
    ResetCase {
        label: "RTC.CNTL reset = 0",
        prep: &[(RCC_APB1ENR, PWREN | BKPEN)],
        read_addr: RTC_CNTL,
        mask: 0xFFFF,
        expect: 0,
    },
];

// ── R/W diff cases ─────────────────────────────────────────────────────────────

struct MmioCase {
    label: &'static str,
    prep: &'static [(u32, u32)],
    write: (u32, u32),
    read_addr: u32,
    mask: u32,
    expect: u32,
}

const CASES: &[MmioCase] = &[
    // ── RCC clock-enable registers (F1-specific offsets) ──
    MmioCase {
        label: "RCC.APB2ENR IOPAEN @0x18",
        prep: &[(RCC_APB2ENR, 0)],
        write: (RCC_APB2ENR, IOPAEN),
        read_addr: RCC_APB2ENR,
        mask: IOPAEN,
        expect: IOPAEN,
    },
    MmioCase {
        label: "RCC.APB2ENR IOPBEN",
        prep: &[(RCC_APB2ENR, 0)],
        write: (RCC_APB2ENR, IOPBEN),
        read_addr: RCC_APB2ENR,
        mask: IOPBEN,
        expect: IOPBEN,
    },
    MmioCase {
        label: "RCC.APB2ENR IOPCEN",
        prep: &[(RCC_APB2ENR, 0)],
        write: (RCC_APB2ENR, IOPCEN),
        read_addr: RCC_APB2ENR,
        mask: IOPCEN,
        expect: IOPCEN,
    },
    MmioCase {
        label: "RCC.APB2ENR SPI1EN @bit12",
        prep: &[(RCC_APB2ENR, 0)],
        write: (RCC_APB2ENR, SPI1EN),
        read_addr: RCC_APB2ENR,
        mask: SPI1EN,
        expect: SPI1EN,
    },
    MmioCase {
        label: "RCC.APB2ENR ADC1EN @bit9",
        prep: &[(RCC_APB2ENR, 0)],
        write: (RCC_APB2ENR, ADC1EN),
        read_addr: RCC_APB2ENR,
        mask: ADC1EN,
        expect: ADC1EN,
    },
    MmioCase {
        label: "RCC.APB1ENR TIM2EN @0x1C",
        prep: &[(RCC_APB1ENR, 0)],
        write: (RCC_APB1ENR, TIM2EN),
        read_addr: RCC_APB1ENR,
        mask: TIM2EN,
        expect: TIM2EN,
    },
    MmioCase {
        label: "RCC.APB1ENR USART2EN @bit17",
        prep: &[(RCC_APB1ENR, 0)],
        write: (RCC_APB1ENR, USART2EN),
        read_addr: RCC_APB1ENR,
        mask: USART2EN,
        expect: USART2EN,
    },
    MmioCase {
        label: "RCC.APB1ENR I2C1EN @bit21",
        prep: &[(RCC_APB1ENR, 0)],
        write: (RCC_APB1ENR, I2C1EN),
        read_addr: RCC_APB1ENR,
        mask: I2C1EN,
        expect: I2C1EN,
    },
    MmioCase {
        label: "RCC.AHBENR DMA1EN @0x14",
        prep: &[(RCC_AHBENR, 0)],
        write: (RCC_AHBENR, DMA1EN),
        read_addr: RCC_AHBENR,
        mask: DMA1EN,
        expect: DMA1EN,
    },
    MmioCase {
        label: "RCC.AHBENR CRCEN",
        prep: &[(RCC_AHBENR, 0)],
        write: (RCC_AHBENR, CRCEN),
        read_addr: RCC_AHBENR,
        mask: CRCEN,
        expect: CRCEN,
    },
    // ── RCC.CFGR PPRE1 (plain R/W config — no clock-source switch) ──
    MmioCase {
        label: "RCC.CFGR PPRE1=/2 (APB1 prescaler R/W)",
        prep: &[(RCC_CFGR, 0)],
        write: (RCC_CFGR, CFGR_PPRE1_DIV2),
        read_addr: RCC_CFGR,
        mask: CFGR_PPRE1_MASK,
        expect: CFGR_PPRE1_DIV2,
    },
    // ── GPIOB output data path (PB5 — free pin, never a JTAG/SWD pin) ──
    MmioCase {
        label: "GPIOB BSRR set PB5 -> ODR bit5",
        prep: &[(RCC_APB2ENR, IOPBEN), (GPIOB_ODR, 0)],
        write: (GPIOB_BSRR, PB5),
        read_addr: GPIOB_ODR,
        mask: PB5,
        expect: PB5,
    },
    MmioCase {
        label: "GPIOB BSRR reset PB5 (high half) -> ODR bit5=0",
        prep: &[(RCC_APB2ENR, IOPBEN), (GPIOB_ODR, PB5)],
        write: (GPIOB_BSRR, PB5 << 16),
        read_addr: GPIOB_ODR,
        mask: PB5,
        expect: 0,
    },
    MmioCase {
        label: "GPIOB BRR reset PB5 -> ODR bit5=0",
        prep: &[(RCC_APB2ENR, IOPBEN), (GPIOB_ODR, PB5)],
        write: (GPIOB_BRR, PB5),
        read_addr: GPIOB_ODR,
        mask: PB5,
        expect: 0,
    },
    // ── SPI1 control registers — classic SPI ──
    MmioCase {
        label: "SPI1 CR1 MSTR",
        prep: &[(RCC_APB2ENR, SPI1EN), (SPI1_CR1, 0)],
        write: (SPI1_CR1, 0x0004), // MSTR (bit 2)
        read_addr: SPI1_CR1,
        mask: 0xFFFF,
        expect: 0x0004,
    },
    MmioCase {
        label: "SPI1 CR1 BR=0b111 (fPCLK/256)",
        prep: &[(RCC_APB2ENR, SPI1EN), (SPI1_CR1, 0)],
        write: (SPI1_CR1, 0x0038),
        read_addr: SPI1_CR1,
        mask: 0x0038,
        expect: 0x0038,
    },
    MmioCase {
        // Classic SPI CR2: TXEIE/RXNEIE/ERRIE/SSOE/DMA bits, NO DS field.
        label: "SPI1 CR2 classic bits (no DS)",
        prep: &[(RCC_APB2ENR, SPI1EN), (SPI1_CR2, 0)],
        write: (SPI1_CR2, 0x00E7), // TXEIE|RXNEIE|ERRIE|SSOE|TXDMAEN|RXDMAEN
        read_addr: SPI1_CR2,
        mask: 0x00F7,
        expect: 0x00E7,
    },
    // ── I2C1 config registers ──
    MmioCase {
        label: "I2C1 CCR (clock control R/W)",
        prep: &[(RCC_APB1ENR, I2C1EN), (I2C1_CCR, 0)],
        write: (I2C1_CCR, 0x0028),
        read_addr: I2C1_CCR,
        mask: 0x0FFF,
        expect: 0x0028,
    },
    MmioCase {
        label: "I2C1 TRISE (R/W, 6-bit field)",
        prep: &[(RCC_APB1ENR, I2C1EN)],
        write: (I2C1_TRISE, 0x0009),
        read_addr: I2C1_TRISE,
        mask: 0x003F,
        expect: 0x0009,
    },
    // ── TIM2 (16-bit) time-base (CEN left off) ──
    MmioCase {
        label: "TIM2 ARR (16-bit reload truncates)",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        write: (TIM2_ARR, 0x0000_ABCD),
        read_addr: TIM2_ARR,
        mask: 0xFFFF,
        expect: 0xABCD,
    },
    MmioCase {
        label: "TIM2 PSC",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        write: (TIM2_PSC, 0x0000_0050),
        read_addr: TIM2_PSC,
        mask: 0xFFFF,
        expect: 0x0050,
    },
    MmioCase {
        label: "TIM2 CR1 ARPE (no CEN)",
        prep: &[(RCC_APB1ENR, TIM2EN), (TIM2_CR1, 0)],
        write: (TIM2_CR1, 0x0080),
        read_addr: TIM2_CR1,
        mask: 0x0080,
        expect: 0x0080,
    },
    // ── ADC1 control register R/W (no ADON — never start a conversion here) ──
    MmioCase {
        label: "ADC1 CR1 SCAN (bit8, R/W)",
        prep: &[(RCC_APB2ENR, ADC1EN), (ADC1_CR1, 0)],
        write: (ADC1_CR1, 0x0100),
        read_addr: ADC1_CR1,
        mask: 0x0100,
        expect: 0x0100,
    },
    // ── EXTI line config R/W ──
    MmioCase {
        label: "EXTI IMR line0 unmask (R/W)",
        prep: &[(EXTI_IMR, 0)],
        write: (EXTI_IMR, 0x0000_0001),
        read_addr: EXTI_IMR,
        mask: 0x0000_0001,
        expect: 0x0000_0001,
    },
    MmioCase {
        label: "EXTI RTSR line0 rising-edge enable (R/W)",
        prep: &[(EXTI_RTSR, 0)],
        write: (EXTI_RTSR, 0x0000_0001),
        read_addr: EXTI_RTSR,
        mask: 0x0000_0001,
        expect: 0x0000_0001,
    },
    // ── DBGMCU identity (read-only) — the hardware oracle ──
    MmioCase {
        label: "DBGMCU IDCODE = 0x20036410 (read-only)",
        prep: &[],
        write: (DBGMCU_IDCODE, 0), // ignored (read-only)
        read_addr: DBGMCU_IDCODE,
        mask: 0xFFFF_FFFF,
        expect: F103_IDCODE,
    },
];

// ── Address-only sweep cases ──────────────────────────────────────────────────
//
// The cheap-model-friendly oracle shape: supply ONLY an address (+ the clock-
// enable prep) and a probe write. The harness writes it to sim AND silicon,
// reads both back, and diffs them — **silicon is the expected value, discovered
// not asserted**. A divergence directly reveals the true silicon writable mask,
// so there is no hand-written `mask`/`expect` for a model (or an LLM drafting
// the table) to get wrong. Use this to grind register coverage: enumerate a
// peripheral's register addresses from the SVD, sweep, fix any DIFF in the model.
struct SweepCase {
    label: &'static str,
    /// RCC clock-enables (and any other safe preamble) applied to both sides.
    prep: &'static [(u32, u32)],
    addr: u32,
    /// Probe value. 0xFFFF_FFFF discovers the writable-bit mask.
    write: u32,
}

/// TIM2 general-purpose register file. SR/EGR are excluded (status / write-only
/// event, not plain read-write). Addresses enumerated from RM0008 §15; masks are
/// NOT asserted — the silicon diff discovers them.
///
/// Order matters — determinism guards learned from the bench (each is a real
/// silicon write-protection interlock that a naive write-everything sweep trips):
///   * **CCRx before CCMRx.** Writing CCMRx sets the CCxS input-capture bits,
///     which flips CCRx to read-only (it then holds a capture value). Probe CCRx
///     while CCMRx is still at its output-mode reset.
///   * **CCMRx before CCER.** CCER.CCxE write-protects the CCxS channel-select
///     bits — set CCxE first and CCMRx bits 0,1,8,9 stop latching. Probe CCMRx
///     while CCxE is still clear.
///   * **CR1 last.** Writing it sets CR1.CEN, starting the real timer, after
///     which a CNT read would race (HW counts, sim doesn't).
///
/// DMAR is intentionally excluded: it is not a plain register but a DCR-windowed
/// alias into the register file, so a flat write→read does not characterise it.
const SWEEP_CASES: &[SweepCase] = &[
    SweepCase {
        label: "TIM2.CR2",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_CR2,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "TIM2.SMCR",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_SMCR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "TIM2.DIER",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_DIER,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "TIM2.CNT",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_CNT,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "TIM2.PSC",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_PSC,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "TIM2.ARR",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_ARR,
        write: 0xFFFF_FFFF,
    },
    // CCRx (output mode) → CCMRx (CCxE still clear) → CCER (sets CCxE). See note.
    SweepCase {
        label: "TIM2.CCR1",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_CCR1,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "TIM2.CCR2",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_CCR2,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "TIM2.CCR3",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_CCR3,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "TIM2.CCR4",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_CCR4,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "TIM2.CCMR1",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_CCMR1,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "TIM2.CCMR2",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_CCMR2,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "TIM2.CCER",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_CCER,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "TIM2.DCR",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_DCR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "TIM2.CR1 (last: sets CEN)",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        addr: TIM2_CR1,
        write: 0xFFFF_FFFF,
    },
    // ── USART2 config registers ──────────────────────────────────────────────
    // SR/DR excluded (status / data with side effects). CR1 last (sets UE).
    SweepCase {
        label: "USART2.BRR",
        prep: &[(RCC_APB1ENR, USART2EN)],
        addr: USART2_BRR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "USART2.CR2",
        prep: &[(RCC_APB1ENR, USART2EN)],
        addr: USART2_CR2,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "USART2.CR3",
        prep: &[(RCC_APB1ENR, USART2EN)],
        addr: USART2_CR3,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "USART2.GTPR",
        prep: &[(RCC_APB1ENR, USART2EN)],
        addr: USART2_GTPR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "USART2.CR1 (last: sets UE)",
        prep: &[(RCC_APB1ENR, USART2EN)],
        addr: USART2_CR1,
        write: 0xFFFF_FFFF,
    },
    // ── SPI1 config registers ────────────────────────────────────────────────
    // SR/DR/RXCRCR/TXCRCR excluded. CR1 last (sets SPE).
    SweepCase {
        label: "SPI1.CR2",
        prep: &[(RCC_APB2ENR, SPI1EN)],
        addr: SPI1_CR2,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "SPI1.CRCPR",
        prep: &[(RCC_APB2ENR, SPI1EN)],
        addr: SPI1_CRCPR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "SPI1.I2SCFGR",
        prep: &[(RCC_APB2ENR, SPI1EN)],
        addr: SPI1_I2SCFGR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "SPI1.I2SPR",
        prep: &[(RCC_APB2ENR, SPI1EN)],
        addr: SPI1_I2SPR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "SPI1.CR1 (last: sets SPE)",
        prep: &[(RCC_APB2ENR, SPI1EN)],
        addr: SPI1_CR1,
        write: 0xFFFF_FFFF,
    },
    // ── I2C1 config registers ────────────────────────────────────────────────
    // SR1/SR2/DR excluded. CR1 last (sets PE; CCR/TRISE/CR2.FREQ need PE=0).
    SweepCase {
        label: "I2C1.CR2",
        prep: &[(RCC_APB1ENR, I2C1EN)],
        addr: I2C1_CR2,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "I2C1.OAR1",
        prep: &[(RCC_APB1ENR, I2C1EN)],
        addr: I2C1_OAR1,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "I2C1.OAR2",
        prep: &[(RCC_APB1ENR, I2C1EN)],
        addr: I2C1_OAR2,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "I2C1.CCR",
        prep: &[(RCC_APB1ENR, I2C1EN)],
        addr: I2C1_CCR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "I2C1.TRISE",
        prep: &[(RCC_APB1ENR, I2C1EN)],
        addr: I2C1_TRISE,
        write: 0xFFFF_FFFF,
    },
    // I2C1.CR1 probed with a non-destructive value: bit 15 (SWRST) resets the
    // peripheral and bits 8/9 (START/STOP) are transient, so 0xFFFF_FFFF can't
    // characterise the stable config mask. 0x2CFB exercises the persistent
    // config bits (PE/SMBUS/SMBTYPE/ENARP/ENPEC/ENGC/NOSTRETCH/ACK/POS/ALERT).
    SweepCase {
        label: "I2C1.CR1 (stable bits, no SWRST)",
        prep: &[(RCC_APB1ENR, I2C1EN)],
        addr: I2C1_CR1,
        write: 0x0000_2CFB,
    },
    // ── DMA1 channel 1+2 data registers (CCR excluded: reserved PSIZE/MSIZE
    //    encoding clamps under an all-ones write; needs a dedicated test). With
    //    CCR unswept the channel stays disabled, so CNDTR/CPAR/CMAR are writable.
    SweepCase {
        label: "DMA1.CNDTR1",
        prep: &[(RCC_AHBENR, DMA1EN)],
        addr: DMA1_CNDTR1,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "DMA1.CPAR1",
        prep: &[(RCC_AHBENR, DMA1EN)],
        addr: DMA1_CPAR1,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "DMA1.CMAR1",
        prep: &[(RCC_AHBENR, DMA1EN)],
        addr: DMA1_CMAR1,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "DMA1.CNDTR2",
        prep: &[(RCC_AHBENR, DMA1EN)],
        addr: DMA1_CNDTR2,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "DMA1.CPAR2",
        prep: &[(RCC_AHBENR, DMA1EN)],
        addr: DMA1_CPAR2,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "DMA1.CMAR2",
        prep: &[(RCC_AHBENR, DMA1EN)],
        addr: DMA1_CMAR2,
        write: 0xFFFF_FFFF,
    },
    // ── AFIO. SELF-DESTRUCT GUARD: MAPR[26:24]=SWJ_CFG; 0b111 disables SWD and
    //    drops the bench, so MAPR is probed with those 3 bits held 0 (0xF8FF_FFFF).
    SweepCase {
        label: "AFIO.EVCR",
        prep: &[(RCC_APB2ENR, AFIOEN)],
        addr: AFIO_EVCR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "AFIO.MAPR (SWJ_CFG held 0)",
        prep: &[(RCC_APB2ENR, AFIOEN)],
        addr: AFIO_MAPR,
        write: 0xF8FF_FFFF,
    },
    SweepCase {
        label: "AFIO.EXTICR1",
        prep: &[(RCC_APB2ENR, AFIOEN)],
        addr: AFIO_EXTICR1,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "AFIO.EXTICR2",
        prep: &[(RCC_APB2ENR, AFIOEN)],
        addr: AFIO_EXTICR2,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "AFIO.EXTICR3",
        prep: &[(RCC_APB2ENR, AFIOEN)],
        addr: AFIO_EXTICR3,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "AFIO.EXTICR4",
        prep: &[(RCC_APB2ENR, AFIOEN)],
        addr: AFIO_EXTICR4,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "AFIO.MAPR2",
        prep: &[(RCC_APB2ENR, AFIOEN)],
        addr: AFIO_MAPR2,
        write: 0xFFFF_FFFF,
    },
    // ── EXTI mask registers (SWIER/PR excluded: trigger/clear side effects).
    SweepCase {
        label: "EXTI.IMR",
        prep: &[],
        addr: EXTI_IMR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "EXTI.EMR",
        prep: &[],
        addr: EXTI_EMR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "EXTI.RTSR",
        prep: &[],
        addr: EXTI_RTSR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "EXTI.FTSR",
        prep: &[],
        addr: EXTI_FTSR,
        write: 0xFFFF_FFFF,
    },
    // ── RCC (safe subset: NO CR/CFGR clock-switch, NO RSTR resets).
    SweepCase {
        label: "RCC.CIR",
        prep: &[],
        addr: RCC_CIR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "RCC.AHBENR",
        prep: &[],
        addr: RCC_AHBENR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "RCC.APB2ENR",
        prep: &[],
        addr: RCC_APB2ENR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "RCC.APB1ENR",
        prep: &[],
        addr: RCC_APB1ENR,
        write: 0xFFFF_FFFF,
    },
];

// ── Register parity sweep (GPIOB config + SPI1 + TIM2 — no SWD pins) ───────────
const PARITY_PATTERNS: &[u32] = &[0x0000_0000, 0xFFFF_FFFF, 0xA5A5_A5A5, 0x5A5A_5A5A];

const ENABLE_PREAMBLE: &[(u32, u32)] = &[
    (RCC_APB2ENR, IOPAEN | IOPBEN | IOPCEN | SPI1EN),
    (RCC_APB1ENR, TIM2EN | I2C1EN),
];

struct ParityReg {
    label: &'static str,
    addr: u32,
    mask: u32,
}

const PARITY_REGS: &[ParityReg] = &[
    // GPIOB config/data words. CRL/CRH are 32-bit config words; ODR 16-bit.
    // (We sweep CRL/CRH freely here because the parity sweep restores them; the
    // JTAG-pin reset quirk only affects the *reset* value, not R/W behaviour.)
    ParityReg {
        label: "GPIOB CRL",
        addr: GPIOB_BASE + GPIO_CRL,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOB CRH",
        addr: GPIOB_BASE + GPIO_CRH,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOB ODR",
        addr: GPIOB_BASE + GPIO_ODR,
        mask: 0x0000_FFFF,
    },
    // SPI1 control registers — classic SPI. Writable masks silicon-confirmed on
    // a genuine F103 (the sweep): CR1 0xFFFF (CRCNEXT bit 12 IS writable, reads
    // back 1 — re-confirmed 2026-06-17), CR2 0xE7 (no DS).
    ParityReg {
        label: "SPI1 CR1",
        addr: SPI1_BASE + 0x00,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "SPI1 CR2",
        addr: SPI1_BASE + 0x04,
        mask: 0x0000_00E7,
    },
    // TIM2 (16-bit) data registers; CEN never set so CNT is stable.
    ParityReg {
        label: "TIM2 PSC",
        addr: TIM2_BASE + 0x28,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "TIM2 ARR",
        addr: TIM2_BASE + 0x2C,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "TIM2 CCR1",
        addr: TIM2_BASE + 0x34,
        mask: 0x0000_FFFF,
    },
];

// ── Sim bus construction ───────────────────────────────────────────────────────

fn build_sim_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/stm32f103.yaml");
    let system_path = manifest_dir.join("../../configs/systems/stm32f103-bare.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load manifest {system_path:?}: {e}"));

    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build sim bus: {e}"))
}

fn sim_reset_read(sim: &mut SystemBus, case: &ResetCase) -> u32 {
    for &(addr, val) in case.prep {
        sim.write_u32(addr as u64, val)
            .unwrap_or_else(|e| panic!("sim reset prep 0x{addr:08X}=0x{val:08X}: {e:?}"));
    }
    let v = sim
        .read_u32(case.read_addr as u64)
        .unwrap_or_else(|e| panic!("sim reset read 0x{:08X}: {e:?}", case.read_addr));
    v & case.mask
}

fn sim_masked_read(sim: &mut SystemBus, case: &MmioCase) -> u32 {
    for &(addr, val) in case.prep {
        sim.write_u32(addr as u64, val)
            .unwrap_or_else(|e| panic!("sim prep write 0x{addr:08X}=0x{val:08X}: {e:?}"));
    }
    sim.write_u32(case.write.0 as u64, case.write.1)
        .unwrap_or_else(|e| {
            panic!(
                "sim write 0x{:08X}=0x{:08X}: {e:?}",
                case.write.0, case.write.1
            )
        });
    let v = sim
        .read_u32(case.read_addr as u64)
        .unwrap_or_else(|e| panic!("sim read 0x{:08X}: {e:?}", case.read_addr));
    v & case.mask
}

// ── Sim-only tests (no hardware — the per-board isolation gate, runs in CI) ────

#[test]
fn f1_reset_sim_only() {
    // A fresh bus per case so reset values are never perturbed by an earlier
    // case's RCC clock-enable preamble bleeding in.
    let mut failures = Vec::new();
    for case in RESET_CASES {
        let mut sim = build_sim_bus();
        let got = sim_reset_read(&mut sim, case);
        if got != case.expect {
            failures.push(format!(
                "  [FAIL] {}: sim=0x{:08X} expected=0x{:08X} (mask=0x{:08X})",
                case.label, got, case.expect, case.mask
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "F103 sim reset values diverged from silicon in {} case(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn f1_mmio_sim_only() {
    let mut sim = build_sim_bus();
    let mut failures = Vec::new();

    for case in CASES {
        let got = sim_masked_read(&mut sim, case);
        if got != case.expect {
            failures.push(format!(
                "  [FAIL] {}: sim=0x{:08X} expected=0x{:08X} (mask=0x{:08X})",
                case.label, got, case.expect, case.mask
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "F103 sim MMIO model diverged from spec in {} case(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn f1_parity_sim_only() {
    let mut sim = build_sim_bus();
    for &(addr, val) in ENABLE_PREAMBLE {
        sim.write_u32(addr as u64, val)
            .unwrap_or_else(|e| panic!("sim preamble 0x{addr:08X}: {e:?}"));
    }

    let mut failures = Vec::new();
    for reg in PARITY_REGS {
        for &pat in PARITY_PATTERNS {
            sim.write_u32(reg.addr as u64, pat)
                .unwrap_or_else(|e| panic!("sim write {} 0x{:08X}: {e:?}", reg.label, pat));
            let got = sim
                .read_u32(reg.addr as u64)
                .unwrap_or_else(|e| panic!("sim read {}: {e:?}", reg.label))
                & reg.mask;
            let want = pat & reg.mask;
            if got != want {
                failures.push(format!(
                    "  [FAIL] {} pat=0x{:08X}: sim=0x{:08X} want=0x{:08X} (mask=0x{:08X})",
                    reg.label, pat, got, want, reg.mask
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "F103 sim parity self-check failed in {} case(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Wellformedness gate for the address sweep (runs in normal CI): every sweep
/// case must write + read cleanly against the modeled bus. This catches a typo'd
/// address (which would sim-error) before it ever reaches the bench; the actual
/// model-vs-silicon comparison happens in the `hw` module's diff.
#[test]
fn f1_sweep_sim_only() {
    let mut sim = build_sim_bus();
    for case in SWEEP_CASES {
        for &(addr, val) in case.prep {
            sim.write_u32(addr as u64, val)
                .unwrap_or_else(|e| panic!("sim prep 0x{addr:08X}: {e:?}"));
        }
        sim.write_u32(case.addr as u64, case.write)
            .unwrap_or_else(|e| {
                panic!("sim sweep write {} 0x{:08X}: {e:?}", case.label, case.addr)
            });
        sim.read_u32(case.addr as u64)
            .unwrap_or_else(|e| panic!("sim sweep read {} 0x{:08X}: {e:?}", case.label, case.addr));
    }
}

// ── Sim-vs-hardware diff (requires connected STM32F103) ─────────────────────────

#[cfg(feature = "hw-oracle-stm32")]
mod hw {
    use super::*;
    use labwired_hw_oracle::openocd::OpenOcd;
    use std::sync::Mutex;

    static HW_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Outcome {
        Match,
        BothDisagreeWithExpect { both: u32 },
        Diverge { sim: u32, hw: u32 },
        SimError(String),
    }

    fn write_both(sim: &mut SystemBus, oc: &mut OpenOcd, addr: u32, val: u32) {
        sim.write_u32(addr as u64, val)
            .unwrap_or_else(|e| panic!("sim write 0x{addr:08X}=0x{val:08X}: {e:?}"));
        oc.write_memory(addr, &[val])
            .unwrap_or_else(|e| panic!("hw write 0x{addr:08X}=0x{val:08X}: {e}"));
    }

    fn classify(sim_m: u32, hw_m: u32, expect: u32) -> Outcome {
        if sim_m == hw_m {
            if sim_m == expect {
                Outcome::Match
            } else {
                Outcome::BothDisagreeWithExpect { both: sim_m }
            }
        } else {
            Outcome::Diverge {
                sim: sim_m,
                hw: hw_m,
            }
        }
    }

    /// Reset-value case: re-`reset halt` so the silicon is in fresh reset state,
    /// then read the register (after applying only the RCC clock-enable prep, on
    /// both sides, from a fresh sim bus). This pins reset values without any
    /// peripheral write disturbing them.
    fn run_reset_case(oc: &mut OpenOcd, case: &ResetCase) -> Outcome {
        oc.reset_halt().expect("reset halt failed");
        oc.halt().expect("halt failed");
        let mut sim = build_sim_bus();

        for &(addr, val) in case.prep {
            write_both(&mut sim, oc, addr, val);
        }
        let sim_val = match sim.read_u32(case.read_addr as u64) {
            Ok(v) => v,
            Err(e) => return Outcome::SimError(format!("{e:?}")),
        };
        let hw_val = oc
            .read_memory(case.read_addr, 1)
            .unwrap_or_else(|e| panic!("hw read 0x{:08X}: {e}", case.read_addr))[0];
        classify(sim_val & case.mask, hw_val & case.mask, case.expect)
    }

    fn run_case(sim: &mut SystemBus, oc: &mut OpenOcd, case: &MmioCase) -> Outcome {
        for &(addr, val) in case.prep {
            write_both(sim, oc, addr, val);
        }
        // Only the IDCODE case is read-only and its write value is 0/ignored, so
        // writing it on hardware is harmless. Keep it simple.
        write_both(sim, oc, case.write.0, case.write.1);

        let sim_val = match sim.read_u32(case.read_addr as u64) {
            Ok(v) => v,
            Err(e) => return Outcome::SimError(format!("{e:?}")),
        };
        let hw_val = oc
            .read_memory(case.read_addr, 1)
            .unwrap_or_else(|e| panic!("hw read 0x{:08X}: {e}", case.read_addr))[0];
        classify(sim_val & case.mask, hw_val & case.mask, case.expect)
    }

    /// Address-only sweep: write the probe, read both sides, diff. Silicon is
    /// the ground truth — `Match` means the model's writable mask agrees with
    /// the chip; `Diverge` prints both so the real mask is revealed.
    fn run_sweep_case(sim: &mut SystemBus, oc: &mut OpenOcd, case: &SweepCase) -> Outcome {
        for &(addr, val) in case.prep {
            write_both(sim, oc, addr, val);
        }
        write_both(sim, oc, case.addr, case.write);
        let sim_val = match sim.read_u32(case.addr as u64) {
            Ok(v) => v,
            Err(e) => return Outcome::SimError(format!("{e:?}")),
        };
        let hw_val = oc
            .read_memory(case.addr, 1)
            .unwrap_or_else(|e| panic!("hw read 0x{:08X}: {e}", case.addr))[0];
        if sim_val == hw_val {
            Outcome::Match
        } else {
            Outcome::Diverge {
                sim: sim_val,
                hw: hw_val,
            }
        }
    }

    #[test]
    #[ignore = "hw-oracle: requires connected STM32F103"]
    fn f1_mmio_diff() {
        let _guard = HW_LOCK.lock().unwrap();

        let mut oc = OpenOcd::spawn_stm32("stm32f1x").expect("openocd spawn_stm32 stm32f1x");

        println!();
        println!(
            "STM32F103 MMIO diff — {} reset + {} R/W + {} sweep cases",
            RESET_CASES.len(),
            CASES.len(),
            SWEEP_CASES.len()
        );
        println!("{:-<90}", "");

        let (mut matched, mut diverged, mut disagree, mut sim_err) = (0, 0, 0, 0);

        let mut tally = |o: &Outcome, label: &str| match o {
            Outcome::Match => {
                matched += 1;
                println!("[OK ]  {label}");
            }
            Outcome::Diverge { sim, hw } => {
                diverged += 1;
                println!("[DIFF] {label}  sim=0x{sim:08X} hw=0x{hw:08X}");
            }
            Outcome::BothDisagreeWithExpect { both } => {
                disagree += 1;
                println!("[BOTH] {label}  both=0x{both:08X}");
            }
            Outcome::SimError(msg) => {
                sim_err += 1;
                println!("[SIM!] {label}  sim error: {msg}");
            }
        };

        println!("-- reset values --");
        for case in RESET_CASES {
            let o = run_reset_case(&mut oc, case);
            tally(&o, case.label);
        }

        // R/W cases run after a final reset_halt so we start from a known state.
        oc.reset_halt().expect("reset halt failed");
        oc.halt().expect("halt failed");
        let mut sim = build_sim_bus();
        println!("-- R/W diffs --");
        for case in CASES {
            let o = run_case(&mut sim, &mut oc, case);
            tally(&o, case.label);
        }

        println!("-- address sweep (silicon = truth) --");
        for case in SWEEP_CASES {
            let o = run_sweep_case(&mut sim, &mut oc, case);
            tally(&o, case.label);
        }

        println!("{:-<90}", "");
        let total = RESET_CASES.len() + CASES.len() + SWEEP_CASES.len();
        println!(
            "summary: match={matched} diverge={diverged} both_disagree={disagree} sim_err={sim_err} total={total}"
        );

        oc.shutdown().ok();

        if std::env::var("F103_STRICT").is_ok() {
            assert_eq!(diverged, 0, "MMIO diff: {diverged} register(s) diverged");
            assert_eq!(sim_err, 0, "MMIO diff: {sim_err} sim error(s)");
        }
    }
}
