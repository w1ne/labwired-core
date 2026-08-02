// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! STM32F407 MMIO peripheral diff oracle.
//!
//! F407's first silicon-validation gate. Two purposes:
//!   * **Cross-validate the shared models on F4 silicon.** F407's `uart` and
//!     `i2c` peripherals use the default `Stm32F1` USART layout and the legacy
//!     `F1I2c` model — the exact register models silicon-pinned on the bench
//!     F103 (PRs #214–#217). Sweeping them on a real F407 proves those per-family
//!     writable masks are universal across F1→F4, not F103-specific.
//!   * **First-validate the F4-only registers** — `F4Rcc` clock-enable masks.
//!
//! Sim side always runs (CI isolation gate); the hardware diff is `--ignored`.
//!
//! ## Running
//!
//! Sim-only:
//! ```text
//! cargo test -p labwired-hw-oracle --test stm32f4_mmio_diff f4_
//! ```
//!
//! Full sim-vs-hardware diff (F407 on its ST-Link). With clone dongles that
//! share garbage serials, pin the probe by USB location instead of serial:
//! ```text
//! LABWIRED_STLINK_LOCATION=1-1 \
//!   cargo test -p labwired-hw-oracle --test stm32f4_mmio_diff \
//!     --features hw-oracle-stm32 -- --ignored --nocapture
//! ```
//! Set `F407_STRICT=1` to make register divergence a hard failure.
//!
//! Hardware: STM32F407 (Cortex-M4, DBGMCU IDCODE 0x1001_6413, DEV_ID 0x413).

#![allow(clippy::identity_op)]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

// ── RCC (0x4002_3800, F4 layout) ─────────────────────────────────────────────
const RCC_BASE: u32 = 0x4002_3800;
const RCC_APB1ENR: u32 = RCC_BASE + 0x40;
const USART2EN: u32 = 1 << 17; // APB1ENR
const I2C1EN: u32 = 1 << 21; // APB1ENR

// ── USART2 (0x4000_4400 — APB1, F1 USART layout) ─────────────────────────────
const USART2_BASE: u32 = 0x4000_4400;
const USART2_BRR: u32 = USART2_BASE + 0x08;
const USART2_CR1: u32 = USART2_BASE + 0x0C;
const USART2_CR2: u32 = USART2_BASE + 0x10;
const USART2_CR3: u32 = USART2_BASE + 0x14;
const USART2_GTPR: u32 = USART2_BASE + 0x18;

// ── I2C1 (0x4000_5400 — APB1, legacy F1I2c) ──────────────────────────────────
const I2C1_BASE: u32 = 0x4000_5400;
const I2C1_CR1: u32 = I2C1_BASE + 0x00;
const I2C1_CR2: u32 = I2C1_BASE + 0x04;
const I2C1_OAR1: u32 = I2C1_BASE + 0x08;
const I2C1_OAR2: u32 = I2C1_BASE + 0x0C;
const I2C1_CCR: u32 = I2C1_BASE + 0x1C;
const I2C1_TRISE: u32 = I2C1_BASE + 0x20;

// ── DBGMCU identity (Cortex-M4 APB @ 0xE004_2000) ────────────────────────────
const DBGMCU_IDCODE: u32 = 0xE004_2000;
const F407_IDCODE: u32 = 0x1001_6413; // DEV_ID 0x413, REV_ID 0x1001

// ── EXTI (0x4001_3C00 on F4 — 23 lines, set via config: {lines: 23}) ──────────
const EXTI_BASE: u32 = 0x4001_3C00;
const EXTI_IMR: u32 = EXTI_BASE + 0x00;
const EXTI_EMR: u32 = EXTI_BASE + 0x04;
const EXTI_RTSR: u32 = EXTI_BASE + 0x08;
const EXTI_FTSR: u32 = EXTI_BASE + 0x0C;

// ── RCC extra registers (F4 map) + clock-enable bits used as sweep prep ───────
const RCC_PLLCFGR: u32 = RCC_BASE + 0x04;
const RCC_CIR: u32 = RCC_BASE + 0x0C;
const RCC_AHB1ENR: u32 = RCC_BASE + 0x30;
const RCC_APB2ENR: u32 = RCC_BASE + 0x44;
const PLLCFGR_RESET: u32 = 0x2400_3010; // RM0090 §6.3.2 factory default
const GPIOCEN: u32 = 1 << 2; // AHB1ENR
const SPI1EN: u32 = 1 << 12; // APB2ENR

// ── GPIOC (0x4002_0800) — swept instead of GPIOA (avoids SWD pins PA13/PA14) ──
const GPIOC_BASE: u32 = 0x4002_0800;
const GPIOC_MODER: u32 = GPIOC_BASE + 0x00;
const GPIOC_OTYPER: u32 = GPIOC_BASE + 0x04;
const GPIOC_OSPEEDR: u32 = GPIOC_BASE + 0x08;
const GPIOC_PUPDR: u32 = GPIOC_BASE + 0x0C;
const GPIOC_AFRL: u32 = GPIOC_BASE + 0x20;
const GPIOC_AFRH: u32 = GPIOC_BASE + 0x24;

const GPIOC_MODER_OUT5: u32 = 0x0000_0400; // PC5 = general-purpose output (01)
const GPIOC_ODR: u32 = GPIOC_BASE + 0x14;
const GPIOC_BSRR: u32 = GPIOC_BASE + 0x18;
const PC5: u32 = 1 << 5;

// ── SPI1 (0x4001_3000 — APB2, classic SPI) ───────────────────────────────────
const SPI1_BASE: u32 = 0x4001_3000;
const SPI1_CR1: u32 = SPI1_BASE + 0x00;
const SPI1_CR2: u32 = SPI1_BASE + 0x04;
const SPI1_CRCPR: u32 = SPI1_BASE + 0x10;
const SPI1_I2SCFGR: u32 = SPI1_BASE + 0x1C;
const SPI1_I2SPR: u32 = SPI1_BASE + 0x20;

// ── TIM2 (0x4000_0000 — APB1, 32-bit GP timer) for behavioural checks ─────────
const TIM2_BASE: u32 = 0x4000_0000;
const TIM2_SR: u32 = TIM2_BASE + 0x10;
const TIM2_EGR: u32 = TIM2_BASE + 0x14;
const TIM2_CNT: u32 = TIM2_BASE + 0x24;
const TIM2_PSC: u32 = TIM2_BASE + 0x28;
const TIM2_ARR: u32 = TIM2_BASE + 0x2C;
const TIM2EN: u32 = 1 << 0; // APB1ENR
const TIM_UG: u32 = 1 << 0; // EGR update-generation
const TIM_UIF: u32 = 1 << 0; // SR update-interrupt flag

struct ResetCase {
    label: &'static str,
    read_addr: u32,
    mask: u32,
    expect: u32,
}

const RESET_CASES: &[ResetCase] = &[
    ResetCase {
        label: "DBGMCU IDCODE = 0x10016413 (DEV_ID 0x413)",
        read_addr: DBGMCU_IDCODE,
        mask: 0x0FFF_FFFF, // REV_ID upper bits vary by die; pin DEV_ID + low rev
        expect: F407_IDCODE & 0x0FFF_FFFF,
    },
    ResetCase {
        label: "RCC.PLLCFGR reset = 0x24003010 (F4 map @0x04)",
        read_addr: RCC_PLLCFGR,
        mask: 0xFFFF_FFFF,
        expect: PLLCFGR_RESET,
    },
];

struct SweepCase {
    label: &'static str,
    prep: &'static [(u32, u32)],
    addr: u32,
    write: u32,
}

/// **Cross-validation of the shared F1 USART + legacy I2C masks on F4 silicon.**
/// Every case here passed on the bench F407 (`diverge=0`), confirming the masks
/// silicon-pinned on the F103 (PRs #214–#217) are universal across F1→F4 — not
/// F103-specific. CR1 is swept last (UART UE / I2C PE enable bits); I2C1.CR1
/// uses a non-destructive probe (0x2CFB) since writing SWRST (bit 15) resets
/// the peripheral.
///
/// USART2.CR3 is the F4 per-part delta — the F1 USART map masks it to `0x07FF`,
/// the F4 adds bit 11 (ONEBIT) → `0x0FFF`, set via the chip config's `cr3_mask`
/// and silicon-confirmed here. (The pattern the user asked for: one shared map,
/// the differing bit separated per part.)
///
/// RCC AHB1ENR/APB1ENR/APB2ENR (silicon `0x7E7411FF` / `0x36FEC9FF` /
/// `0x00075F33`) are the F407 implemented-peripheral set — part-specific, since
/// `F4Rcc` is shared with the smaller STM32F401. Pinned here via the per-part
/// `rcc_*enr_mask` config fields (F4Rcc per-instance masks); F401 keeps the
/// unmasked default until it is benched.
const SWEEP_CASES: &[SweepCase] = &[
    // USART2 — shared F1 USART layout.
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
    // CR3: per-part delta — F4 mask 0x0FFF (ONEBIT bit 11) via the chip config.
    SweepCase {
        label: "USART2.CR3 (F4: ONEBIT)",
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
        label: "USART2.CR1 (last: UE)",
        prep: &[(RCC_APB1ENR, USART2EN)],
        addr: USART2_CR1,
        write: 0xFFFF_FFFF,
    },
    // I2C1 — legacy F1I2c (CR2/OAR1/OAR2/CCR/TRISE then CR1, no SWRST).
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
    SweepCase {
        label: "I2C1.CR1 (stable bits, no SWRST)",
        prep: &[(RCC_APB1ENR, I2C1EN)],
        addr: I2C1_CR1,
        write: 0x0000_2CFB,
    },
    // ── RCC F4 config registers (PLLCFGR, CIR) — first F4 validation. ─────────
    SweepCase {
        label: "RCC.PLLCFGR",
        prep: &[],
        addr: RCC_PLLCFGR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "RCC.CIR",
        prep: &[],
        addr: RCC_CIR,
        write: 0xFFFF_FFFF,
    },
    // ── GPIOC (stm32f4_gpio) — first F4 GPIO validation. ─────────────────────
    SweepCase {
        label: "GPIOC.MODER",
        prep: &[(RCC_AHB1ENR, GPIOCEN)],
        addr: GPIOC_MODER,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "GPIOC.OTYPER",
        prep: &[(RCC_AHB1ENR, GPIOCEN)],
        addr: GPIOC_OTYPER,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "GPIOC.OSPEEDR",
        prep: &[(RCC_AHB1ENR, GPIOCEN)],
        addr: GPIOC_OSPEEDR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "GPIOC.PUPDR",
        prep: &[(RCC_AHB1ENR, GPIOCEN)],
        addr: GPIOC_PUPDR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "GPIOC.AFRL",
        prep: &[(RCC_AHB1ENR, GPIOCEN)],
        addr: GPIOC_AFRL,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "GPIOC.AFRH",
        prep: &[(RCC_AHB1ENR, GPIOCEN)],
        addr: GPIOC_AFRH,
        write: 0xFFFF_FFFF,
    },
    // ── SPI1 (classic SPI) — cross-validates the shared masks on F4. ──────────
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
        label: "SPI1.CR1 (last: SPE)",
        prep: &[(RCC_APB2ENR, SPI1EN)],
        addr: SPI1_CR1,
        write: 0xFFFF_FFFF,
    },
    // ── EXTI mask registers — F4 base 0x40013C00, 23 lines (0x7FFFFF). ────────
    SweepCase {
        label: "EXTI.IMR (23 lines)",
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
    // ── RCC ENR — per-part masks (F407 implemented-peripheral set, via config). ─
    SweepCase {
        label: "RCC.AHB1ENR",
        prep: &[],
        addr: RCC_AHB1ENR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "RCC.APB1ENR",
        prep: &[],
        addr: RCC_APB1ENR,
        write: 0xFFFF_FFFF,
    },
    SweepCase {
        label: "RCC.APB2ENR",
        prep: &[],
        addr: RCC_APB2ENR,
        write: 0xFFFF_FFFF,
    },
];

/// Behavioural (depth) checks — beyond register masks, these drive a register
/// **sequence** that triggers peripheral *logic*, then read the resulting state
/// and compare sim vs silicon AND vs the expected behaviour. Validates that the
/// F4 GPIO/timer models reproduce the real silicon's behaviour, not just its
/// register layout.
struct BehaviorCase {
    label: &'static str,
    steps: &'static [(u32, u32)],
    read_addr: u32,
    mask: u32,
    expect: u32,
}

const BEHAVIOR_CASES: &[BehaviorCase] = &[
    // GPIO BSRR atomic set: BS5 sets ODR.5.
    BehaviorCase {
        label: "GPIOC.BSRR set PC5 → ODR.5 = 1",
        steps: &[
            (RCC_AHB1ENR, 1 << 2), // GPIOCEN
            (GPIOC_MODER, GPIOC_MODER_OUT5),
            (GPIOC_BSRR, PC5), // BS5
        ],
        read_addr: GPIOC_ODR,
        mask: PC5,
        expect: PC5,
    },
    // GPIO BSRR atomic reset: BR5 (bit 21) clears ODR.5 set just before.
    BehaviorCase {
        label: "GPIOC.BSRR reset PC5 → ODR.5 = 0",
        steps: &[
            (RCC_AHB1ENR, 1 << 2),
            (GPIOC_MODER, GPIOC_MODER_OUT5),
            (GPIOC_BSRR, PC5),       // set
            (GPIOC_BSRR, PC5 << 16), // BR5 reset
        ],
        read_addr: GPIOC_ODR,
        mask: PC5,
        expect: 0,
    },
    // Timer update event: writing EGR.UG reloads CNT to 0 and latches SR.UIF.
    BehaviorCase {
        label: "TIM2.EGR.UG → SR.UIF latched",
        steps: &[
            (RCC_APB1ENR, TIM2EN),
            (TIM2_ARR, 0x0000_FFFF),
            (TIM2_PSC, 0),
            (TIM2_EGR, TIM_UG),
        ],
        read_addr: TIM2_SR,
        mask: TIM_UIF,
        expect: TIM_UIF,
    },
    BehaviorCase {
        label: "TIM2.EGR.UG → CNT reloads to 0",
        steps: &[
            (RCC_APB1ENR, TIM2EN),
            (TIM2_ARR, 0x0000_FFFF),
            (TIM2_PSC, 0),
            (TIM2_EGR, TIM_UG),
        ],
        read_addr: TIM2_CNT,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
];

fn build_sim_bus() -> SystemBus {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = dir.join("../../configs/chips/stm32f407.yaml");
    let system_path = dir.join("../../configs/systems/nucleo-f407.yaml");
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load manifest {system_path:?}: {e}"));
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();
    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build sim bus: {e}"))
}

/// Sim-only wellformedness gate (runs in normal CI): every sweep case writes +
/// reads cleanly against the modeled F407 bus. Catches a typo'd address before
/// it reaches the bench; the model-vs-silicon diff lives in the `hw` module.
/// Sim-only reset gate: the modeled F407 returns the configured DBGMCU IDCODE.
#[test]
fn f4_reset_sim_only() {
    let sim = build_sim_bus();
    for case in RESET_CASES {
        let v = sim
            .read_u32(case.read_addr as u64)
            .unwrap_or_else(|e| panic!("sim read {} 0x{:08X}: {e:?}", case.label, case.read_addr));
        assert_eq!(v & case.mask, case.expect, "{}", case.label);
    }
}

#[test]
fn f4_sweep_sim_only() {
    let mut sim = build_sim_bus();
    for case in SWEEP_CASES {
        for &(addr, val) in case.prep {
            sim.write_u32(addr as u64, val)
                .unwrap_or_else(|e| panic!("sim prep 0x{addr:08X}: {e:?}"));
        }
        sim.write_u32(case.addr as u64, case.write)
            .unwrap_or_else(|e| panic!("sim write {} 0x{:08X}: {e:?}", case.label, case.addr));
        sim.read_u32(case.addr as u64)
            .unwrap_or_else(|e| panic!("sim read {} 0x{:08X}: {e:?}", case.label, case.addr));
    }
}

/// Sim-only behavioural gate (runs in CI): the modeled F407 reproduces the
/// expected peripheral *logic* (GPIO BSRR set/reset, timer UG event). The bench
/// then confirms the silicon does the same in the `hw` module.
#[test]
fn f4_behavior_sim_only() {
    for case in BEHAVIOR_CASES {
        let mut sim = build_sim_bus();
        for &(addr, val) in case.steps {
            sim.write_u32(addr as u64, val)
                .unwrap_or_else(|e| panic!("sim step 0x{addr:08X}: {e:?}"));
        }
        let v = sim
            .read_u32(case.read_addr as u64)
            .unwrap_or_else(|e| panic!("sim read {}: {e:?}", case.label));
        assert_eq!(v & case.mask, case.expect, "{}", case.label);
    }
}

// ── Sim-vs-hardware diff (requires a connected STM32F407) ─────────────────────

#[cfg(feature = "hw-oracle-stm32")]
mod hw {
    use super::*;
    use labwired_hw_oracle::openocd::OpenOcd;
    use std::sync::Mutex;

    static HW_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug)]
    enum Outcome {
        Match,
        Diverge { sim: u32, hw: u32 },
        SimError(String),
    }

    fn write_both(sim: &mut SystemBus, oc: &mut OpenOcd, addr: u32, val: u32) {
        sim.write_u32(addr as u64, val)
            .unwrap_or_else(|e| panic!("sim write 0x{addr:08X}=0x{val:08X}: {e:?}"));
        oc.write_memory(addr, &[val])
            .unwrap_or_else(|e| panic!("hw write 0x{addr:08X}=0x{val:08X}: {e}"));
    }

    fn run_reset_case(oc: &mut OpenOcd, case: &ResetCase) -> Outcome {
        oc.reset_halt().expect("reset halt");
        oc.halt().expect("halt");
        let sim = build_sim_bus();
        let sim_val = match sim.read_u32(case.read_addr as u64) {
            Ok(v) => v,
            Err(e) => return Outcome::SimError(format!("{e:?}")),
        };
        let hw_val = oc
            .read_memory(case.read_addr, 1)
            .unwrap_or_else(|e| panic!("hw read 0x{:08X}: {e}", case.read_addr))[0];
        if (sim_val & case.mask) == case.expect && (hw_val & case.mask) == case.expect {
            Outcome::Match
        } else {
            Outcome::Diverge {
                sim: sim_val & case.mask,
                hw: hw_val & case.mask,
            }
        }
    }

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

    /// Behavioural case: a fresh reset, drive the register sequence on both
    /// sides, then read the resulting state. `Match` requires sim == hw AND both
    /// == the expected behaviour (so it catches a model that silently agrees
    /// with silicon on the wrong value too).
    fn run_behavior_case(oc: &mut OpenOcd, case: &BehaviorCase) -> Outcome {
        oc.reset_halt().expect("reset halt");
        oc.halt().expect("halt");
        let mut sim = build_sim_bus();
        for &(addr, val) in case.steps {
            write_both(&mut sim, oc, addr, val);
        }
        let sim_val = match sim.read_u32(case.read_addr as u64) {
            Ok(v) => v,
            Err(e) => return Outcome::SimError(format!("{e:?}")),
        };
        let hw_val = oc
            .read_memory(case.read_addr, 1)
            .unwrap_or_else(|e| panic!("hw read 0x{:08X}: {e}", case.read_addr))[0];
        if (sim_val & case.mask) == case.expect && (hw_val & case.mask) == case.expect {
            Outcome::Match
        } else {
            Outcome::Diverge {
                sim: sim_val & case.mask,
                hw: hw_val & case.mask,
            }
        }
    }

    #[test]
    #[ignore = "hw-oracle: requires connected STM32F407 (LABWIRED_STLINK_LOCATION)"]
    fn f4_mmio_diff() {
        let _guard = HW_LOCK.lock().unwrap();
        let mut oc = OpenOcd::spawn_stm32("stm32f4x").expect("openocd spawn stm32f4x");

        println!();
        println!(
            "STM32F407 MMIO diff — {} reset + {} sweep + {} behaviour cases",
            RESET_CASES.len(),
            SWEEP_CASES.len(),
            BEHAVIOR_CASES.len()
        );
        println!("{:-<70}", "");

        let (mut matched, mut diverged, mut sim_err) = (0, 0, 0);
        let mut tally = |o: &Outcome, label: &str| match o {
            Outcome::Match => {
                matched += 1;
                println!("[OK ]  {label}");
            }
            Outcome::Diverge { sim, hw } => {
                diverged += 1;
                println!("[DIFF] {label}  sim=0x{sim:08X} hw=0x{hw:08X}");
            }
            Outcome::SimError(msg) => {
                sim_err += 1;
                println!("[SIM!] {label}  {msg}");
            }
        };

        println!("-- reset values --");
        for case in RESET_CASES {
            let o = run_reset_case(&mut oc, case);
            tally(&o, case.label);
        }

        oc.reset_halt().expect("reset halt");
        oc.halt().expect("halt");
        let mut sim = build_sim_bus();
        println!("-- address sweep (silicon = truth) --");
        for case in SWEEP_CASES {
            let o = run_sweep_case(&mut sim, &mut oc, case);
            tally(&o, case.label);
        }

        println!("-- behaviour (depth: drive a sequence, check the logic) --");
        for case in BEHAVIOR_CASES {
            let o = run_behavior_case(&mut oc, case);
            tally(&o, case.label);
        }

        println!("{:-<70}", "");
        let total = RESET_CASES.len() + SWEEP_CASES.len() + BEHAVIOR_CASES.len();
        println!("summary: match={matched} diverge={diverged} sim_err={sim_err} total={total}");
        oc.shutdown().ok();

        if std::env::var("F407_STRICT").is_ok() {
            assert_eq!(
                diverged, 0,
                "F407 MMIO diff: {diverged} register(s) diverged"
            );
            assert_eq!(sim_err, 0, "F407 MMIO diff: {sim_err} sim error(s)");
        }
    }
}
