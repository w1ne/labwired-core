// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! STM32H563 (NUCLEO-H563ZI) GPIO MMIO diff oracle.
//!
//! Closes pending-silicon-verification entry #1: bit-band alias translation
//! is gated on cores that implement it (Cortex-M3/M4 only — `ee1133c`), so
//! on the M33-based H563 word accesses to the GPIO ports at `0x4202_xxxx`
//! must reach the GPIO model un-shadowed instead of being rewritten into
//! bit-band operations against `0x4002_xxxx`. Every case below performs
//! plain 32-bit writes/reads in exactly that address window, on the sim bus
//! and on real silicon, and diffs the masked readback.
//!
//! Philosophy mirrors `l476_mmio_diff.rs`: no instruction execution, just
//! register write/read against the modeled peripherals on both sides, then
//! a masked comparison.
//!
//! H5-specific: peripheral reads are clock-gated on silicon (a disabled
//! port does not return its reset values), so every case enables the
//! relevant `RCC_AHB2ENR` bits first — on top of the register's
//! 0xC000_0000 reset (SRAM2/SRAM3 retention enables), which is preserved,
//! never cleared.
//!
//! ## Running
//!
//! Sim-only (no hardware — runs in normal CI):
//! ```text
//! cargo test -p labwired-hw-oracle --test h563_mmio_diff h563_mmio_sim_only
//! ```
//!
//! Full sim-vs-hardware diff (NUCLEO-H563ZI connected via its on-board
//! STLINK-V3 — AP1 dapdirect recipe, see `OpenOcd::spawn_stm32h563`):
//! ```text
//! cargo test -p labwired-hw-oracle --test h563_mmio_diff \
//!     --features hw-oracle-stm32 -- --ignored --nocapture
//! ```
//! Set `H563_STRICT=1` to make register divergence a hard failure.
//!
//! Hardware validated against: NUCLEO-H563ZI (STM32H563ZI, Cortex-M33
//! r0p4, DBGMCU IDCODE 0x10016484).

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

// ── RCC (0x4402_0C00, h5 profile) ───────────────────────────────────────────
const RCC_BASE: u32 = 0x4402_0C00;
const RCC_AHB2ENR: u32 = RCC_BASE + 0x8C;

/// AHB2ENR resets to 0xC000_0000 (SRAM2/SRAM3 retention clock enables) —
/// every prep keeps these set so a probe never powers down SRAM under a
/// halted CPU.
const AHB2ENR_RESET: u32 = 0xC000_0000;
const GPIOBEN: u32 = 1 << 1;
const GPIOEEN: u32 = 1 << 4;
const GPIOFEN: u32 = 1 << 5;
const GPIOGEN: u32 = 1 << 6;

// ── GPIO (stm32v2 layout, AHB2 0x4202_xxxx — the un-shadowed window) ────────
const GPIOB_BASE: u32 = 0x4202_0400;
const GPIOE_BASE: u32 = 0x4202_1000;
const GPIOF_BASE: u32 = 0x4202_1400;
const GPIOG_BASE: u32 = 0x4202_1800;

const GPIO_ODR: u32 = 0x14;
const GPIO_BSRR: u32 = 0x18;
const GPIO_BRR: u32 = 0x28;

const GPIOB_ODR: u32 = GPIOB_BASE + GPIO_ODR;
const GPIOB_BSRR: u32 = GPIOB_BASE + GPIO_BSRR;
const GPIOB_BRR: u32 = GPIOB_BASE + GPIO_BRR;
const GPIOF_ODR: u32 = GPIOF_BASE + GPIO_ODR;
const GPIOF_BSRR: u32 = GPIOF_BASE + GPIO_BSRR;
const GPIOG_ODR: u32 = GPIOG_BASE + GPIO_ODR;
const GPIOG_BSRR: u32 = GPIOG_BASE + GPIO_BSRR;

// Board LEDs — output-data-path pins that are safe to drive: LD1 = PB0
// (green), LD2 = PF4 (yellow), LD3 = PG4 (red). The SWD pins are PA13/PA14
// (+ PB3 SWO); no case below touches GPIOA or any port-B config register,
// only the atomic BSRR/BRR/ODR data path.
const PB0: u32 = 1 << 0;
const PF4: u32 = 1 << 4;
const PG4: u32 = 1 << 4;

// ── Case definition ──────────────────────────────────────────────────────────

/// A single MMIO probe: optional prep writes (clock enables / pin
/// pre-state), the write under test, then a masked readback that must match
/// `expect` on both sim and hardware. `settle_ticks` runs the sim's
/// peripheral tick loop after the write so autonomous engines (GPDMA,
/// running timers) make progress — silicon runs free while the debugger
/// round-trips, so the hardware side needs no equivalent.
struct MmioCase {
    label: &'static str,
    prep: &'static [(u32, u32)],
    write: (u32, u32),
    read_addr: u32,
    mask: u32,
    expect: u32,
    settle_ticks: u32,
}

const CASES: &[MmioCase] = &[
    // ── RCC clock-enable gate (prep for everything below) ──
    MmioCase {
        label: "RCC.AHB2ENR GPIOBEN",
        prep: &[(RCC_AHB2ENR, AHB2ENR_RESET)],
        write: (RCC_AHB2ENR, AHB2ENR_RESET | GPIOBEN),
        read_addr: RCC_AHB2ENR,
        mask: GPIOBEN,
        expect: GPIOBEN,
        settle_ticks: 0,
    },
    // ── GPIOB output data path (PB0 / LD1) — word writes at 0x4202_0414/18/28.
    // A bit-band-translating bus would shadow all of these (rewrite them into
    // bit ops on 0x4002_xxxx) and the readbacks would stay 0.
    MmioCase {
        label: "GPIOB ODR direct word write PB0 (0x4202_xxxx un-shadowed)",
        prep: &[(RCC_AHB2ENR, AHB2ENR_RESET | GPIOBEN), (GPIOB_ODR, 0)],
        write: (GPIOB_ODR, PB0),
        read_addr: GPIOB_ODR,
        mask: PB0,
        expect: PB0,
        settle_ticks: 0,
    },
    MmioCase {
        label: "GPIOB BSRR set PB0 -> ODR bit0",
        prep: &[(RCC_AHB2ENR, AHB2ENR_RESET | GPIOBEN), (GPIOB_ODR, 0)],
        write: (GPIOB_BSRR, PB0),
        read_addr: GPIOB_ODR,
        mask: PB0,
        expect: PB0,
        settle_ticks: 0,
    },
    MmioCase {
        label: "GPIOB BSRR reset PB0 (high half) -> ODR bit0=0",
        prep: &[(RCC_AHB2ENR, AHB2ENR_RESET | GPIOBEN), (GPIOB_ODR, PB0)],
        write: (GPIOB_BSRR, PB0 << 16),
        read_addr: GPIOB_ODR,
        mask: PB0,
        expect: 0,
        settle_ticks: 0,
    },
    MmioCase {
        label: "GPIOB BRR reset PB0 -> ODR bit0=0",
        prep: &[(RCC_AHB2ENR, AHB2ENR_RESET | GPIOBEN), (GPIOB_ODR, PB0)],
        write: (GPIOB_BRR, PB0),
        read_addr: GPIOB_ODR,
        mask: PB0,
        expect: 0,
        settle_ticks: 0,
    },
    // BSRR carries set+reset for the same pin in one 32-bit word: BS wins
    // (RM0481 §12.4.7). Only an atomic word-level write models this — the
    // same transaction a bit-band shadow would tear apart.
    MmioCase {
        label: "GPIOB BSRR set+reset PB0 in one word -> BS priority",
        prep: &[(RCC_AHB2ENR, AHB2ENR_RESET | GPIOBEN), (GPIOB_ODR, 0)],
        write: (GPIOB_BSRR, PB0 | (PB0 << 16)),
        read_addr: GPIOB_ODR,
        mask: PB0,
        expect: PB0,
        settle_ticks: 0,
    },
    // ── GPIOF / GPIOG data path (LD2 / LD3) — the io-smoke pins that were
    // shadowed before ee1133c.
    MmioCase {
        label: "GPIOF BSRR set PF4 -> ODR bit4",
        prep: &[(RCC_AHB2ENR, AHB2ENR_RESET | GPIOFEN), (GPIOF_ODR, 0)],
        write: (GPIOF_BSRR, PF4),
        read_addr: GPIOF_ODR,
        mask: PF4,
        expect: PF4,
        settle_ticks: 0,
    },
    MmioCase {
        label: "GPIOG BSRR set PG4 -> ODR bit4",
        prep: &[(RCC_AHB2ENR, AHB2ENR_RESET | GPIOGEN), (GPIOG_ODR, 0)],
        write: (GPIOG_BSRR, PG4),
        read_addr: GPIOG_ODR,
        mask: PG4,
        expect: PG4,
        settle_ticks: 0,
    },
];

// ── Register parity sweep ─────────────────────────────────────────────────────
//
// Complementary bit patterns into every R/W GPIO register on the
// debug-pin-free ports E and F, sim vs silicon, masked to implemented bits.
// This pins individual bit behaviour across the whole 0x4202_xxxx config
// surface, not just the data path. (GPIOA carries SWD on PA13/PA14 and
// GPIOB carries SWO on PB3 — neither port's config registers are swept; the
// debug connection the diff rides on is never disturbed.)

const PARITY_PATTERNS: &[u32] = &[0x0000_0000, 0xFFFF_FFFF, 0xA5A5_A5A5, 0x5A5A_5A5A];

const ENABLE_PREAMBLE: &[(u32, u32)] =
    &[(RCC_AHB2ENR, AHB2ENR_RESET | GPIOEEN | GPIOFEN | GPIOGEN)];

struct ParityReg {
    label: &'static str,
    addr: u32,
    mask: u32,
}

const PARITY_REGS: &[ParityReg] = &[
    ParityReg {
        label: "GPIOE MODER",
        addr: GPIOE_BASE,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOE OTYPER",
        addr: GPIOE_BASE + 0x04,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "GPIOE OSPEEDR",
        addr: GPIOE_BASE + 0x08,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOE PUPDR",
        addr: GPIOE_BASE + 0x0C,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOE ODR",
        addr: GPIOE_BASE + 0x14,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "GPIOE AFRL",
        addr: GPIOE_BASE + 0x20,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOE AFRH",
        addr: GPIOE_BASE + 0x24,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOF MODER",
        addr: GPIOF_BASE,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOF OSPEEDR",
        addr: GPIOF_BASE + 0x08,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOF PUPDR",
        addr: GPIOF_BASE + 0x0C,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOF AFRL",
        addr: GPIOF_BASE + 0x20,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOF AFRH",
        addr: GPIOF_BASE + 0x24,
        mask: 0xFFFF_FFFF,
    },
];

// ── Class-model cases: SPI / ADC / RTC / GPDMA / TIM1-PWM / NVIC ─────────────
//
// Sequential probe of every model added for the spi/adc/pwm/rtc/irq/dma
// tier1 classes, mirroring the 2026-06-11 bench captures (validation
// corpus probe-20260611). Cases run in order on ONE sim bus and ONE halted
// board — state deliberately carries from case to case, exactly like the
// original capture scripts.

const RCC_AHB1ENR: u32 = RCC_BASE + 0x88;
const RCC_APB2ENR: u32 = RCC_BASE + 0xA4;
const RCC_APB1HENR: u32 = RCC_BASE + 0xA0;
const RCC_CR: u32 = RCC_BASE;

// ── FDCAN1 (0x4000_A400, M_CAN with fixed message-RAM layout) ──────────────
const FDCAN1: u32 = 0x4000_A400;
const SRAMCAN: u32 = 0x4000_AC00;
const RCC_APB3ENR: u32 = RCC_BASE + 0xA8;
const RCC_BDCR: u32 = RCC_BASE + 0xF0;
/// AHB1ENR reset (capture: 0xD000_0100) + GPDMA1EN(0).
const AHB1_GPDMA: u32 = 0xD000_0101;
const APB2_TIM1_SPI1: u32 = (1 << 11) | (1 << 12);

const SPI1: u32 = 0x4001_3000;
const SPI1_CR1: u32 = SPI1;
const SPI1_CR2: u32 = SPI1 + 0x04;
const SPI1_CFG1: u32 = SPI1 + 0x08;
const SPI1_CFG2: u32 = SPI1 + 0x0C;
const SPI1_SR: u32 = SPI1 + 0x14;
const SPI1_IFCR: u32 = SPI1 + 0x18;
const SPI1_TXDR: u32 = SPI1 + 0x20;
const SPI1_CRCPOLY: u32 = SPI1 + 0x40;
const SSI: u32 = 1 << 12;
const SPE: u32 = 1;
const CSTART: u32 = 1 << 9;
const MASTER_SSM: u32 = (1 << 22) | (1 << 26);

const ADC1: u32 = 0x4202_8000;
const ADC1_ISR: u32 = ADC1;
const ADC1_CR: u32 = ADC1 + 0x08;

const RTC: u32 = 0x4400_7800;
const RTC_TR: u32 = RTC;
const RTC_DR: u32 = RTC + 0x04;
const RTC_ICSR: u32 = RTC + 0x0C;
const RTC_PRER: u32 = RTC + 0x10;
const RTC_CR: u32 = RTC + 0x18;
const RTC_WPR: u32 = RTC + 0x24;
const PWR_DBPCR: u32 = 0x4402_0824;

const GPDMA_C0FCR: u32 = 0x4002_005C;
const GPDMA_C0SR: u32 = 0x4002_0060;
const GPDMA_C0CR: u32 = 0x4002_0064;
const GPDMA_C0TR1: u32 = 0x4002_0090;
const GPDMA_C0TR2: u32 = 0x4002_0094;
const GPDMA_C0BR1: u32 = 0x4002_0098;
const GPDMA_C0SAR: u32 = 0x4002_009C;
const GPDMA_C0DAR: u32 = 0x4002_00A0;
const DMA_SRC: u32 = 0x2000_1000;
const DMA_DST: u32 = 0x2000_1100;

const TIM1: u32 = 0x4001_2C00;

const NVIC_ISER0: u32 = 0xE000_E100;
const NVIC_ICER0: u32 = 0xE000_E180;

const CLASS_CASES: &[MmioCase] = &[
    // ── SPI1 (stm32h5 IP) ──
    MmioCase {
        label: "SPI1 CFG1 reset",
        prep: &[],
        write: (RCC_APB2ENR, APB2_TIM1_SPI1),
        read_addr: SPI1_CFG1,
        mask: 0xFFFF_FFFF,
        expect: 0x0007_0007,
        settle_ticks: 0,
    },
    MmioCase {
        label: "SPI1 SR reset (TXP|TXC)",
        prep: &[],
        write: (RCC_APB2ENR, APB2_TIM1_SPI1),
        read_addr: SPI1_SR,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_1002,
        settle_ticks: 0,
    },
    MmioCase {
        label: "SPI1 CRCPOLY reset",
        prep: &[],
        write: (RCC_APB2ENR, APB2_TIM1_SPI1),
        read_addr: SPI1_CRCPOLY,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_0107,
        settle_ticks: 0,
    },
    MmioCase {
        label: "SPI1 CFG1 round-trip",
        prep: &[],
        write: (SPI1_CFG1, 0x7000_0007),
        read_addr: SPI1_CFG1,
        mask: 0xFFFF_FFFF,
        expect: 0x7000_0007,
        settle_ticks: 0,
    },
    MmioCase {
        label: "SPI1 CFG1 reserved bits read zero",
        prep: &[],
        write: (SPI1_CFG1, 0x5555_AAAA),
        read_addr: SPI1_CFG1,
        mask: 0xFFFF_FFFF,
        expect: 0x5055_82AA,
        settle_ticks: 0,
    },
    MmioCase {
        label: "SPI1 MASTER refused while SS low (SSM, SSI=0)",
        prep: &[(SPI1_CR1, 0), (SPI1_CFG1, 0x0007_0007)],
        write: (SPI1_CFG2, MASTER_SSM),
        read_addr: SPI1_CFG2,
        mask: 0xFFFF_FFFF,
        expect: 1 << 26,
        settle_ticks: 0,
    },
    MmioCase {
        // Attempt SPE with the internal SS still low: the enable is refused
        // and MODF stands in SR (capture round 1: SR read 0x1202 after the
        // refused SPE).
        label: "SPI1 MODF latched in SR after refused SPE",
        prep: &[],
        write: (SPI1_CR1, SPE),
        read_addr: SPI1_SR,
        mask: 1 << 9,
        expect: 1 << 9,
        settle_ticks: 0,
    },
    MmioCase {
        label: "SPI1 IFCR clears MODF",
        prep: &[],
        write: (SPI1_IFCR, 0xFFFF_FFFF),
        read_addr: SPI1_SR,
        mask: 1 << 9,
        expect: 0,
        settle_ticks: 0,
    },
    MmioCase {
        label: "SPI1 MASTER sticks with SSI high",
        prep: &[(SPI1_CR1, SSI), (SPI1_CFG2, 0)],
        write: (SPI1_CFG2, MASTER_SSM),
        read_addr: SPI1_CFG2,
        mask: 0xFFFF_FFFF,
        expect: MASTER_SSM,
        settle_ticks: 0,
    },
    MmioCase {
        label: "SPI1 SPE loads CTSIZE, drops TXC",
        prep: &[(SPI1_CR2, 2)],
        write: (SPI1_CR1, SSI | SPE),
        read_addr: SPI1_SR,
        mask: 0xFFFF_1002,
        expect: 0x0002_0002,
        settle_ticks: 0,
    },
    MmioCase {
        label: "SPI1 CFG2 locked while SPE",
        prep: &[],
        write: (SPI1_CFG2, 0),
        read_addr: SPI1_CFG2,
        mask: 0xFFFF_FFFF,
        expect: MASTER_SSM,
        settle_ticks: 0,
    },
    MmioCase {
        label: "SPI1 TXDR sets TXTF",
        prep: &[(SPI1_CR1, SSI | SPE | CSTART)],
        write: (SPI1_TXDR, 0xA5),
        read_addr: SPI1_SR,
        mask: 1 << 4,
        expect: 1 << 4,
        settle_ticks: 0,
    },
    MmioCase {
        label: "SPI1 IFCR clears TXTF",
        prep: &[],
        write: (SPI1_IFCR, 0xFFFF_FFFF),
        read_addr: SPI1_SR,
        mask: 1 << 4,
        expect: 0,
        settle_ticks: 0,
    },
    MmioCase {
        label: "SPI1 disable restores TXC",
        prep: &[],
        write: (SPI1_CR1, SSI),
        read_addr: SPI1_SR,
        mask: 1 << 12,
        expect: 1 << 12,
        settle_ticks: 0,
    },
    // ── ADC1 power-up handshake ──
    MmioCase {
        label: "ADC1 CR resets to DEEPPWD",
        prep: &[(SPI1_CR1, 0), (SPI1_CFG2, 0), (SPI1_CR2, 0)],
        write: (RCC_AHB2ENR, AHB2ENR_RESET | (1 << 10)),
        read_addr: ADC1_CR,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_0000,
        settle_ticks: 0,
    },
    MmioCase {
        label: "ADC1 DEEPPWD clears",
        prep: &[],
        write: (ADC1_CR, 0),
        read_addr: ADC1_CR,
        mask: 0xFFFF_FFFF,
        expect: 0,
        settle_ticks: 0,
    },
    MmioCase {
        label: "ADC1 ADVREGEN",
        prep: &[],
        write: (ADC1_CR, 1 << 28),
        read_addr: ADC1_CR,
        mask: 0xFFFF_FFFF,
        expect: 1 << 28,
        settle_ticks: 0,
    },
    MmioCase {
        label: "ADC1 ADEN raises ADRDY",
        prep: &[],
        write: (ADC1_CR, (1 << 28) | 1),
        read_addr: ADC1_ISR,
        mask: 0x1,
        expect: 0x1,
        settle_ticks: 0,
    },
    // ── RTC v3 bring-up (DBP -> LSI -> RTCEN -> WPR -> init) ──
    // RTCSEL is write-once until a backup-domain reset; the bench board
    // already carries RTCSEL=LSI from the onboarding probes, so the BDCR
    // writes below are idempotent there and first-time on the sim.
    MmioCase {
        label: "RCC BDCR LSION -> LSIRDY",
        prep: &[
            (ADC1_ISR, 0x1),
            (ADC1_CR, 0x2000_0000),
            (RCC_APB3ENR, 1 << 21),
            (PWR_DBPCR, 1),
        ],
        write: (RCC_BDCR, 1 << 26),
        read_addr: RCC_BDCR,
        mask: 1 << 27,
        expect: 1 << 27,
        settle_ticks: 0,
    },
    MmioCase {
        label: "RCC BDCR RTCSEL=LSI + RTCEN",
        prep: &[],
        write: (RCC_BDCR, (1 << 26) | (0x2 << 8) | (1 << 15)),
        read_addr: RCC_BDCR,
        mask: 1 << 15,
        expect: 1 << 15,
        settle_ticks: 0,
    },
    MmioCase {
        label: "RTC WPR unlock + CR.BYPSHAD",
        prep: &[(RTC_WPR, 0xCA), (RTC_WPR, 0x53)],
        write: (RTC_CR, 1 << 5),
        read_addr: RTC_CR,
        mask: 0xFFFF_FFFF,
        expect: 1 << 5,
        settle_ticks: 0,
    },
    MmioCase {
        label: "RTC INIT -> INITF",
        prep: &[],
        write: (RTC_ICSR, 1 << 7),
        read_addr: RTC_ICSR,
        mask: 0xC0,
        expect: 0xC0,
        settle_ticks: 0,
    },
    MmioCase {
        label: "RTC TR write in init mode",
        prep: &[],
        write: (RTC_TR, 0x0012_3456),
        read_addr: RTC_TR,
        mask: 0xFFFF_FFFF,
        expect: 0x0012_3456,
        settle_ticks: 0,
    },
    MmioCase {
        label: "RTC DR write in init mode",
        prep: &[],
        write: (RTC_DR, 0x0026_0611),
        read_addr: RTC_DR,
        mask: 0x00FF_FF3F,
        expect: 0x0026_0611 & 0x00FF_FF3F,
        settle_ticks: 0,
    },
    MmioCase {
        label: "RTC PRER round-trip",
        prep: &[],
        write: (RTC_PRER, 0x007F_00FF),
        read_addr: RTC_PRER,
        mask: 0xFFFF_FFFF,
        expect: 0x007F_00FF,
        settle_ticks: 0,
    },
    MmioCase {
        label: "RTC exit init -> INITS, INITF clear",
        prep: &[],
        write: (RTC_ICSR, 0),
        read_addr: RTC_ICSR,
        mask: 0x50,
        expect: 0x10,
        settle_ticks: 0,
    },
    MmioCase {
        label: "RTC calendar holds written time after relock",
        prep: &[(RTC_WPR, 0xFF)],
        write: (RTC_TR, 0), // locked write — must be dropped on both sides
        read_addr: RTC_TR,
        mask: 0x00FF_FF00,
        expect: 0x0012_3400,
        settle_ticks: 0,
    },
    // ── GPDMA1 channel 0 mem-to-mem (autonomous engine on silicon) ──
    MmioCase {
        label: "GPDMA C0SR idles with IDLEF",
        prep: &[],
        write: (RCC_AHB1ENR, AHB1_GPDMA),
        read_addr: GPDMA_C0SR,
        mask: 0xFFFF_FFFF,
        expect: 0x1,
        settle_ticks: 0,
    },
    MmioCase {
        label: "GPDMA mem-to-mem completes (TCF|HTF|IDLEF)",
        prep: &[
            (DMA_SRC, 0x1122_3344),
            (DMA_SRC + 4, 0x5566_7788),
            (DMA_SRC + 8, 0x99AA_BBCC),
            (DMA_SRC + 12, 0xDDEE_FF00),
            (DMA_DST, 0),
            (DMA_DST + 4, 0),
            (DMA_DST + 8, 0),
            (DMA_DST + 12, 0),
            (GPDMA_C0FCR, 0xFFFF_FFFF),
            (GPDMA_C0TR1, (1 << 3) | (1 << 19)),
            (GPDMA_C0TR2, 1 << 9),
            (GPDMA_C0BR1, 16),
            (GPDMA_C0SAR, DMA_SRC),
            (GPDMA_C0DAR, DMA_DST),
        ],
        write: (GPDMA_C0CR, 0x1),
        read_addr: GPDMA_C0SR,
        mask: 0xFFFF_FFFF,
        expect: 0x301,
        settle_ticks: 64,
    },
    MmioCase {
        label: "GPDMA BNDT drained to 0",
        prep: &[],
        write: (RCC_AHB1ENR, AHB1_GPDMA),
        read_addr: GPDMA_C0BR1,
        mask: 0xFFFF,
        expect: 0,
        settle_ticks: 0,
    },
    MmioCase {
        label: "GPDMA SAR advanced by block",
        prep: &[],
        write: (RCC_AHB1ENR, AHB1_GPDMA),
        read_addr: GPDMA_C0SAR,
        mask: 0xFFFF_FFFF,
        expect: DMA_SRC + 16,
        settle_ticks: 0,
    },
    MmioCase {
        label: "GPDMA dest word 0",
        prep: &[],
        write: (RCC_AHB1ENR, AHB1_GPDMA),
        read_addr: DMA_DST,
        mask: 0xFFFF_FFFF,
        expect: 0x1122_3344,
        settle_ticks: 0,
    },
    MmioCase {
        label: "GPDMA dest word 3",
        prep: &[],
        write: (RCC_AHB1ENR, AHB1_GPDMA),
        read_addr: DMA_DST + 12,
        mask: 0xFFFF_FFFF,
        expect: 0xDDEE_FF00,
        settle_ticks: 0,
    },
    MmioCase {
        label: "GPDMA EN auto-clears at TC",
        prep: &[],
        write: (RCC_AHB1ENR, AHB1_GPDMA),
        read_addr: GPDMA_C0CR,
        mask: 0x1,
        expect: 0,
        settle_ticks: 0,
    },
    MmioCase {
        label: "GPDMA CFCR clears flags back to idle",
        prep: &[],
        write: (GPDMA_C0FCR, 0xFFFF_FFFF),
        read_addr: GPDMA_C0SR,
        mask: 0xFFFF_FFFF,
        expect: 0x1,
        settle_ticks: 0,
    },
    // ── TIM1 PWM surface + run (timer free-runs on silicon while halted) ──
    MmioCase {
        label: "TIM1 CCMR1 PWM mode 1 round-trip",
        prep: &[],
        write: (TIM1 + 0x18, 0x0068),
        read_addr: TIM1 + 0x18,
        mask: 0xFFFF,
        expect: 0x0068,
        settle_ticks: 0,
    },
    MmioCase {
        label: "TIM1 CCER CC1E round-trip",
        prep: &[],
        write: (TIM1 + 0x20, 0x0001),
        read_addr: TIM1 + 0x20,
        mask: 0xFFFF,
        expect: 0x0001,
        settle_ticks: 0,
    },
    MmioCase {
        label: "TIM1 BDTR MOE round-trip",
        prep: &[],
        write: (TIM1 + 0x44, 0x8000),
        read_addr: TIM1 + 0x44,
        mask: 0x8000,
        expect: 0x8000,
        settle_ticks: 0,
    },
    MmioCase {
        label: "TIM1 PWM run latches UIF+CC1..4IF+CC5/6IF",
        prep: &[
            (TIM1 + 0x28, 0),   // PSC
            (TIM1 + 0x2C, 100), // ARR
            (TIM1 + 0x34, 50),  // CCR1
            (TIM1 + 0x14, 1),   // EGR.UG
            (TIM1 + 0x10, 0),   // clear SR
        ],
        write: (TIM1, 0x1), // CEN
        read_addr: TIM1 + 0x10,
        mask: 0x0003_001F,
        expect: 0x0003_001F,
        settle_ticks: 300,
    },
    MmioCase {
        label: "TIM1 stop + SR clear",
        prep: &[(TIM1, 0), (TIM1 + 0x10, 0), (TIM1 + 0x2C, 0xFFFF)],
        write: (TIM1 + 0x24, 0), // CNT
        read_addr: TIM1 + 0x24,
        mask: 0xFFFF_FFFF,
        expect: 0,
        settle_ticks: 0,
    },
    // ── NVIC enable machinery (M33 side of the F103-anchored oracle) ──
    MmioCase {
        label: "NVIC ISER0 set-enable",
        prep: &[(NVIC_ICER0, 0xFFFF_FFFF)],
        write: (NVIC_ISER0, (1 << 27) | (1 << 5)),
        read_addr: NVIC_ISER0,
        mask: 0xFFFF_FFFF,
        expect: (1 << 27) | (1 << 5),
        settle_ticks: 0,
    },
    MmioCase {
        label: "NVIC ICER0 clear-enable is selective",
        prep: &[],
        write: (NVIC_ICER0, 1 << 5),
        read_addr: NVIC_ISER0,
        mask: 0xFFFF_FFFF,
        expect: 1 << 27,
        settle_ticks: 0,
    },
    MmioCase {
        label: "NVIC ICER0 full clear",
        prep: &[],
        write: (NVIC_ICER0, 0xFFFF_FFFF),
        read_addr: NVIC_ISER0,
        mask: 0xFFFF_FFFF,
        expect: 0,
        settle_ticks: 0,
    },
    // ── FDCAN1 internal loopback (capture13, 2026-06-12) ──
    // Sequence replays the bench probe register-for-register: clocks,
    // reset identity, config unlock, test mode, TX buffer 0 through the
    // fixed SRAMCAN layout into RX FIFO0.
    MmioCase {
        label: "FDCAN1 bus+kernel clock (HSE digital bypass ready)",
        prep: &[(RCC_APB1HENR, 1 << 9)],
        // HSEON | HSEBYP | HSEEXT on top of the reset HSI bits.
        write: (RCC_CR, 0x0015_002B),
        read_addr: RCC_CR,
        mask: 1 << 17,
        expect: 1 << 17,
        settle_ticks: 4,
    },
    MmioCase {
        label: "FDCAN1 ENDN identity",
        prep: &[],
        write: (FDCAN1 + 0x54, 0), // IE: benign
        read_addr: FDCAN1 + 0x04,
        mask: 0xFFFF_FFFF,
        expect: 0x8765_4321,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 CREL identity",
        prep: &[],
        write: (FDCAN1 + 0x54, 0),
        read_addr: FDCAN1,
        mask: 0xFFFF_FFFF,
        expect: 0x3214_1218,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 XIDAM reset (fixed-layout map: 0x84 is XIDAM)",
        prep: &[],
        write: (FDCAN1 + 0x54, 0),
        read_addr: FDCAN1 + 0x84,
        mask: 0xFFFF_FFFF,
        expect: 0x1FFF_FFFF,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 TXFQS reset (3 free slots)",
        prep: &[],
        write: (FDCAN1 + 0x54, 0),
        read_addr: FDCAN1 + 0xC4,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_0003,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 CCCR INIT|CCE unlock",
        prep: &[],
        write: (FDCAN1 + 0x18, 0x3),
        read_addr: FDCAN1 + 0x18,
        mask: 0xFF,
        expect: 0x03,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 test mode + monitoring armed",
        prep: &[],
        write: (FDCAN1 + 0x18, 0xA3),
        read_addr: FDCAN1 + 0x18,
        mask: 0xFF,
        expect: 0xA3,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 TEST.LBCK sticks",
        prep: &[],
        write: (FDCAN1 + 0x10, 1 << 4),
        read_addr: FDCAN1 + 0x10,
        mask: 0xFF,
        expect: 0x10,
        settle_ticks: 0,
    },
    MmioCase {
        // TX element 0 staged in SRAMCAN, RX element 0 blanked so stale
        // RAM can't fake the compares below; leaving INIT drops CCE
        // with it (silicon: write 0xA2, read 0xA0).
        label: "FDCAN1 leave INIT (CCE auto-clears)",
        prep: &[
            (SRAMCAN + 0x278, 0x123 << 18), // T0: std ID
            (SRAMCAN + 0x27C, 8 << 16),     // T1: DLC 8
            (SRAMCAN + 0x280, 0xDEAD_BEEF),
            (SRAMCAN + 0x284, 0xCAFE_BABE),
            (SRAMCAN + 0xB0, 0),
            (SRAMCAN + 0xB4, 0),
            (SRAMCAN + 0xB8, 0),
            (SRAMCAN + 0xBC, 0),
        ],
        write: (FDCAN1 + 0x18, 0xA2),
        read_addr: FDCAN1 + 0x18,
        mask: 0xFF,
        expect: 0xA0,
        settle_ticks: 4,
    },
    MmioCase {
        label: "FDCAN1 TXBAR completes buffer 0 (TXBTO)",
        prep: &[],
        write: (FDCAN1 + 0xCC, 0x1),
        read_addr: FDCAN1 + 0xD4,
        mask: 0x1,
        expect: 0x1,
        settle_ticks: 4,
    },
    MmioCase {
        // RF0N | TFE; TC (bit 7) stays clear because TXBTIE = 0 — the
        // per-buffer enable gates IR.TC, not the completion itself.
        label: "FDCAN1 IR after loopback = RF0N|TFE (TC gated by TXBTIE)",
        prep: &[],
        write: (FDCAN1 + 0x54, 0),
        read_addr: FDCAN1 + 0x50,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_0201,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 RXF0S one frame (F0PI=1, F0FL=1)",
        prep: &[],
        write: (FDCAN1 + 0x54, 0),
        read_addr: FDCAN1 + 0x90,
        mask: 0xFFFF_FFFF,
        expect: 0x0001_0001,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 TXFQS after one TX (indices advanced, 3 free)",
        prep: &[],
        write: (FDCAN1 + 0x54, 0),
        read_addr: FDCAN1 + 0xC4,
        mask: 0xFFFF_FFFF,
        expect: 0x0001_0103,
        settle_ticks: 0,
    },
    MmioCase {
        // Silicon leaves R0[17:0] undefined for standard IDs (capture13
        // read 0x048C0246) — mask to the ID field.
        label: "FDCAN1 RX element R0 standard ID",
        prep: &[],
        write: (FDCAN1 + 0x54, 0),
        read_addr: SRAMCAN + 0xB0,
        mask: 0x1FFC_0000,
        expect: 0x123 << 18,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 RX element R1 DLC + ANMF",
        prep: &[],
        write: (FDCAN1 + 0x54, 0),
        read_addr: SRAMCAN + 0xB4,
        mask: 0x800F_0000,
        expect: 0x8008_0000,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 RX payload word 0",
        prep: &[],
        write: (FDCAN1 + 0x54, 0),
        read_addr: SRAMCAN + 0xB8,
        mask: 0xFFFF_FFFF,
        expect: 0xDEAD_BEEF,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 RX payload word 1",
        prep: &[],
        write: (FDCAN1 + 0x54, 0),
        read_addr: SRAMCAN + 0xBC,
        mask: 0xFFFF_FFFF,
        expect: 0xCAFE_BABE,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 PSR after traffic (LEC=0, ACT=idle)",
        prep: &[],
        write: (FDCAN1 + 0x54, 0),
        read_addr: FDCAN1 + 0x44,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_0708,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 IR write-1-to-clear",
        prep: &[],
        write: (FDCAN1 + 0x50, 0xFFFF_FFFF),
        read_addr: FDCAN1 + 0x50,
        mask: 0xFFFF_FFFF,
        expect: 0,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN1 RXF0A ack advances get index, drops fill",
        prep: &[],
        write: (FDCAN1 + 0x94, 0),
        read_addr: FDCAN1 + 0x90,
        mask: 0xFFFF_FFFF,
        expect: 0x0001_0100,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN_CONFIG VERR identity",
        prep: &[],
        write: (FDCAN1 + 0x54, 0),
        read_addr: FDCAN1 + 0x3F4,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_0010,
        settle_ticks: 0,
    },
    MmioCase {
        label: "FDCAN_CONFIG SIDR identity",
        prep: &[],
        write: (FDCAN1 + 0x54, 0),
        read_addr: FDCAN1 + 0x3FC,
        mask: 0xFFFF_FFFF,
        expect: 0xA3C5_DD01,
        settle_ticks: 0,
    },
];

// ── Sim bus construction ──────────────────────────────────────────────────────

fn build_sim_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/stm32h563.yaml");
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let manifest = SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "h563-mmio-diff".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    let mut bus =
        SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build sim bus: {e}"));
    // Wire the real Cortex-M system block (NVIC/SCB/DWT) — the class cases
    // probe NVIC ISER/ICER, which the raw bus constructor stubs out.
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    bus
}

/// Apply a case's prep + write to the sim bus and return the masked readback.
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
    for _ in 0..case.settle_ticks {
        // Advance the bus cycle clock alongside the walk (one cycle per
        // tick), exactly as the Machine run loop does at tick interval 1:
        // scheduler-driven models (the walk-free timers) derive their state
        // lazily from the published clock instead of the walk.
        let now = sim.current_cycle + 1;
        sim.set_current_cycle(now);
        sim.tick_peripherals_fully();
    }
    let v = sim
        .read_u32(case.read_addr as u64)
        .unwrap_or_else(|e| panic!("sim read 0x{:08X}: {e:?}", case.read_addr));
    v & case.mask
}

// ── Sim-only tests (no hardware — run in normal CI) ───────────────────────────

/// Validate that the H563 sim model produces the spec value for every case.
/// On a bus that still applied bit-band translation to the M33, every GPIO
/// write here would be shadowed into 0x4002_xxxx and these readbacks would
/// fail — this is the CI-side regression guard for ee1133c.
#[test]
fn h563_mmio_sim_only() {
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
        "H563 sim MMIO model diverged from spec in {} case(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// One-to-one sim parity self-check over the swept GPIO registers.
#[test]
fn h563_parity_sim_only() {
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
        "H563 sim parity self-check failed in {} case(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Class-model sequence (SPI/ADC/RTC/GPDMA/TIM1/NVIC) against the sim alone.
/// State carries across cases by design — run them in order on one bus.
#[test]
fn h563_class_sim_only() {
    let mut sim = build_sim_bus();
    let mut failures = Vec::new();

    for case in CLASS_CASES {
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
        "H563 sim class-model sequence diverged from silicon spec in {} case(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ── Sim-vs-hardware diff (requires connected NUCLEO-H563ZI) ───────────────────

#[cfg(feature = "hw-oracle-stm32")]
mod hw {
    use super::*;
    use labwired_hw_oracle::openocd::OpenOcd;
    use std::sync::Mutex;

    // Serialise hardware tests: only one OpenOCD instance can hold the
    // ST-Link at a time, and cargo runs tests in parallel by default.
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

    fn run_case(sim: &mut SystemBus, oc: &mut OpenOcd, case: &MmioCase) -> Outcome {
        for &(addr, val) in case.prep {
            write_both(sim, oc, addr, val);
        }
        write_both(sim, oc, case.write.0, case.write.1);
        // Settle: the sim ticks its peripheral engines; silicon has been
        // running free since the write (each TCL round-trip is ~ms), so a
        // matching settle on the hardware side is implicit. The cycle clock
        // advances alongside the walk (one cycle per tick) so scheduler-
        // driven models (the walk-free timers) elapse the same time lazily.
        for _ in 0..case.settle_ticks {
            let now = sim.current_cycle + 1;
            sim.set_current_cycle(now);
            sim.tick_peripherals_fully();
        }

        let sim_val = match sim.read_u32(case.read_addr as u64) {
            Ok(v) => v,
            Err(e) => return Outcome::SimError(format!("{e:?}")),
        };
        let hw_val = oc
            .read_memory(case.read_addr, 1)
            .unwrap_or_else(|e| panic!("hw read 0x{:08X}: {e}", case.read_addr))[0];

        let sim_m = sim_val & case.mask;
        let hw_m = hw_val & case.mask;

        if sim_m == hw_m {
            if sim_m == case.expect {
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

    #[test]
    #[ignore = "hw-oracle: requires connected NUCLEO-H563ZI"]
    fn h563_mmio_diff() {
        let _guard = HW_LOCK.lock().unwrap();

        let mut sim = build_sim_bus();
        let mut oc = OpenOcd::spawn_stm32h563().expect("openocd spawn_stm32h563");

        // Halt the CPU so we have exclusive control over the peripheral regs.
        oc.reset_halt().expect("reset halt failed");
        oc.halt().expect("halt failed");

        println!();
        println!("STM32H563 GPIO MMIO diff — {} cases", CASES.len());
        println!("{:-<90}", "");

        let (mut matched, mut diverged, mut disagree, mut sim_err) = (0, 0, 0, 0);
        for case in CASES {
            match run_case(&mut sim, &mut oc, case) {
                Outcome::Match => {
                    matched += 1;
                    println!("[OK ]  {}", case.label);
                }
                Outcome::Diverge { sim, hw } => {
                    diverged += 1;
                    println!(
                        "[DIFF] {}  sim=0x{:08X} hw=0x{:08X} (mask=0x{:08X})",
                        case.label, sim, hw, case.mask
                    );
                }
                Outcome::BothDisagreeWithExpect { both } => {
                    disagree += 1;
                    println!(
                        "[BOTH] {}  both=0x{:08X} expected=0x{:08X}",
                        case.label, both, case.expect
                    );
                }
                Outcome::SimError(msg) => {
                    sim_err += 1;
                    println!("[SIM!] {}  sim error: {}", case.label, msg);
                }
            }
        }

        println!("{:-<90}", "");
        println!(
            "summary: match={matched} diverge={diverged} both_disagree={disagree} \
             sim_err={sim_err} total={}",
            CASES.len()
        );

        oc.shutdown().ok();

        if std::env::var("H563_STRICT").is_ok() {
            assert_eq!(diverged, 0, "MMIO diff: {diverged} register(s) diverged");
            assert_eq!(sim_err, 0, "MMIO diff: {sim_err} sim error(s)");
        }
    }

    /// One-to-one register parity sweep, sim vs silicon, masked to
    /// implemented bits.
    #[test]
    #[ignore = "hw-oracle: requires connected NUCLEO-H563ZI"]
    fn h563_parity_diff() {
        let _guard = HW_LOCK.lock().unwrap();

        let mut sim = build_sim_bus();
        let mut oc = OpenOcd::spawn_stm32h563().expect("openocd spawn_stm32h563");
        oc.reset_halt().expect("reset halt failed");
        oc.halt().expect("halt failed");

        for &(addr, val) in ENABLE_PREAMBLE {
            write_both(&mut sim, &mut oc, addr, val);
        }

        println!();
        println!(
            "STM32H563 GPIO register parity sweep — {} regs x {} patterns",
            PARITY_REGS.len(),
            PARITY_PATTERNS.len()
        );
        println!("{:-<90}", "");

        let (mut total, mut matched, mut diverged) = (0u32, 0u32, 0u32);
        for reg in PARITY_REGS {
            let mut reg_ok = true;
            for &pat in PARITY_PATTERNS {
                write_both(&mut sim, &mut oc, reg.addr, pat);
                let sim_v = sim.read_u32(reg.addr as u64).expect("sim read") & reg.mask;
                let hw_v = oc.read_memory(reg.addr, 1).expect("hw read")[0] & reg.mask;
                total += 1;
                if sim_v == hw_v {
                    matched += 1;
                } else {
                    diverged += 1;
                    reg_ok = false;
                    println!(
                        "[DIFF] {} pat=0x{:08X}  sim=0x{:08X} hw=0x{:08X} (mask=0x{:08X})",
                        reg.label, pat, sim_v, hw_v, reg.mask
                    );
                }
            }
            if reg_ok {
                println!("[OK ]  {} (x{} patterns)", reg.label, PARITY_PATTERNS.len());
            }
        }

        println!("{:-<90}", "");
        println!("parity: match={matched} diverge={diverged} total={total}");

        oc.shutdown().ok();

        if std::env::var("H563_STRICT").is_ok() {
            assert_eq!(
                diverged, 0,
                "parity sweep: {diverged} register-pattern(s) diverged"
            );
        }
    }

    /// Class-model sequence (SPI/ADC/RTC/GPDMA/TIM1/NVIC), sim vs silicon.
    /// Same ordered, state-carrying flow as the 2026-06-11 capture scripts —
    /// including the autonomous GPDMA mem-to-mem block copy verified through
    /// SRAM contents on both sides.
    #[test]
    #[ignore = "hw-oracle: requires connected NUCLEO-H563ZI"]
    fn h563_class_diff() {
        let _guard = HW_LOCK.lock().unwrap();

        let mut sim = build_sim_bus();
        let mut oc = OpenOcd::spawn_stm32h563().expect("openocd spawn_stm32h563");
        oc.reset_halt().expect("reset halt failed");
        oc.halt().expect("halt failed");

        println!();
        println!("STM32H563 class-model diff — {} cases", CLASS_CASES.len());
        println!("{:-<90}", "");

        let (mut matched, mut diverged, mut disagree, mut sim_err) = (0, 0, 0, 0);
        for case in CLASS_CASES {
            match run_case(&mut sim, &mut oc, case) {
                Outcome::Match => {
                    matched += 1;
                    println!("[OK ]  {}", case.label);
                }
                Outcome::Diverge { sim, hw } => {
                    diverged += 1;
                    println!(
                        "[DIFF] {}  sim=0x{:08X} hw=0x{:08X} (mask=0x{:08X})",
                        case.label, sim, hw, case.mask
                    );
                }
                Outcome::BothDisagreeWithExpect { both } => {
                    disagree += 1;
                    println!(
                        "[BOTH] {}  both=0x{:08X} expected=0x{:08X}",
                        case.label, both, case.expect
                    );
                }
                Outcome::SimError(msg) => {
                    sim_err += 1;
                    println!("[SIM!] {}  sim error: {}", case.label, msg);
                }
            }
        }

        println!("{:-<90}", "");
        println!(
            "summary: match={matched} diverge={diverged} both_disagree={disagree} \
             sim_err={sim_err} total={}",
            CLASS_CASES.len()
        );

        oc.shutdown().ok();

        if std::env::var("H563_STRICT").is_ok() {
            assert_eq!(diverged, 0, "class diff: {diverged} register(s) diverged");
            assert_eq!(disagree, 0, "class diff: {disagree} both-disagree case(s)");
            assert_eq!(sim_err, 0, "class diff: {sim_err} sim error(s)");
        }
    }
}
