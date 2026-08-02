// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! STM32L073 (NUCLEO-L073RZ) MMIO peripheral diff oracle.
//!
//! Register-level cross-validation of the L0 peripheral models against real
//! NUCLEO-L073RZ silicon over ST-Link SWD. This is the L0 board's independent
//! golden gate: it pins L0-specific register semantics (the `stm32l0` RCC
//! layout, the IOPORT GPIO bus @0x50000000, the *classic* SPI with CR2 reset
//! 0x0000, the M0+ DBGMCU @0x40015800) so a change to any shared model that
//! would leak into the L0 board is caught here, per-board.
//!
//! Philosophy mirrors `l476_mmio_diff.rs` / `nrf52_mmio_diff.rs`: no
//! instruction execution, just register write/read against the modeled
//! peripherals on both sides, then a masked comparison.
//!
//! ## Running
//!
//! Sim-only (no hardware — runs in normal CI, the per-board isolation gate):
//! ```text
//! cargo test -p labwired-hw-oracle --test stm32l0_mmio_diff l0_mmio_sim_only
//! ```
//!
//! Full sim-vs-hardware diff (NUCLEO-L073RZ on its ST-Link/V2-1; with multiple
//! ST-Links attached, pick the L0 probe by serial):
//! ```text
//! LABWIRED_STLINK_SERIAL=<l073-stlink-serial> \
//!   cargo test -p labwired-hw-oracle --test stm32l0_mmio_diff \
//!     --features hw-oracle-stm32 -- --ignored --nocapture
//! ```
//! Set `L073_STRICT=1` to make register divergence a hard failure.
//!
//! Hardware validated against: NUCLEO-L073RZ (STM32L073RZ, Cortex-M0+,
//! DBGMCU IDCODE 0x20086447).

// Register-map style: `BASE + 0x00` and `1 << 0` document offsets/bit
// positions literally — keep them rather than collapsing to bare values.
#![allow(clippy::identity_op)]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

// ── RCC (0x4002_1000, RM0367 §7 — L0 layout) ─────────────────────────────────
const RCC_BASE: u32 = 0x4002_1000;
const RCC_CR: u32 = RCC_BASE + 0x00;
const RCC_CFGR: u32 = RCC_BASE + 0x0C; // L0: CRRCR @0x08 pushes CFGR to 0x0C
const RCC_IOPENR: u32 = RCC_BASE + 0x2C; // GPIO port clock enable
const RCC_AHBENR: u32 = RCC_BASE + 0x30; // DMA/CRC/RNG
const RCC_APB2ENR: u32 = RCC_BASE + 0x34; // SPI1/TIM21/USART1
const RCC_APB1ENR: u32 = RCC_BASE + 0x38; // TIM2/USART2

const IOPAEN: u32 = 1 << 0;
const IOPBEN: u32 = 1 << 1;
const IOPCEN: u32 = 1 << 2;
const CRCEN: u32 = 1 << 12;
const SPI1EN: u32 = 1 << 12;
const TIM21EN: u32 = 1 << 2;
const TIM2EN: u32 = 1 << 0;
const USART2EN: u32 = 1 << 17;

// CR bits (L0): HSI16ON bit0 / HSI16RDY bit2, MSION bit8 / MSIRDY bit9.
const CR_HSI16ON: u32 = 1 << 0;
const CR_MSI_ON_RDY: u32 = (1 << 8) | (1 << 9);
const CFGR_SW_HSI16: u32 = 0b01;
const CFGR_SWS_MASK: u32 = 0b11 << 2;
const CFGR_SWS_HSI16: u32 = 0b01 << 2;

// ── GPIO (stm32v2 layout, IOPORT bus @ 0x5000_0000) ──────────────────────────
const GPIOA_BASE: u32 = 0x5000_0000;
const GPIOB_BASE: u32 = 0x5000_0400;
const GPIO_ODR: u32 = 0x14;
const GPIO_BSRR: u32 = 0x18;
const GPIO_BRR: u32 = 0x28;
const GPIOA_ODR: u32 = GPIOA_BASE + GPIO_ODR;
const GPIOA_BSRR: u32 = GPIOA_BASE + GPIO_BSRR;
const GPIOA_BRR: u32 = GPIOA_BASE + GPIO_BRR;

// PA5 = LD2 (user LED — safe to drive). SWD is PA13/PA14, never touched here.
const PA5: u32 = 1 << 5;

// ── SPI1 (0x4001_3000 — classic SPI: no CR2.DS) ──────────────────────────────
const SPI1_BASE: u32 = 0x4001_3000;
const SPI1_CR1: u32 = SPI1_BASE + 0x00;
const SPI1_CR2: u32 = SPI1_BASE + 0x04;

// ── TIM2 (0x4000_0000, 16-bit GP timer on L0) / TIM21 (0x4001_0800, 16-bit) ──
const TIM2_BASE: u32 = 0x4000_0000;
const TIM2_CR1: u32 = TIM2_BASE + 0x00;
const TIM2_PSC: u32 = TIM2_BASE + 0x28;
const TIM2_ARR: u32 = TIM2_BASE + 0x2C;
const TIM21_BASE: u32 = 0x4001_0800;
const TIM21_ARR: u32 = TIM21_BASE + 0x2C;

// ── DBGMCU (APB @ 0x4001_5800 on Cortex-M0+) ─────────────────────────────────
const DBGMCU_IDCODE: u32 = 0x4001_5800;
const L073_IDCODE: u32 = 0x2008_6447; // silicon-read DEV_ID 0x447, REV_ID 0x2008

// ── Case definition ──────────────────────────────────────────────────────────

struct MmioCase {
    label: &'static str,
    prep: &'static [(u32, u32)],
    write: (u32, u32),
    read_addr: u32,
    mask: u32,
    expect: u32,
}

const CASES: &[MmioCase] = &[
    // ── RCC clock-enable registers (L0-specific offsets) ──
    MmioCase {
        label: "RCC.IOPENR GPIOAEN @0x2C",
        prep: &[(RCC_IOPENR, 0)],
        write: (RCC_IOPENR, IOPAEN),
        read_addr: RCC_IOPENR,
        mask: IOPAEN,
        expect: IOPAEN,
    },
    MmioCase {
        label: "RCC.IOPENR GPIOBEN",
        prep: &[(RCC_IOPENR, 0)],
        write: (RCC_IOPENR, IOPBEN),
        read_addr: RCC_IOPENR,
        mask: IOPBEN,
        expect: IOPBEN,
    },
    MmioCase {
        label: "RCC.IOPENR GPIOCEN",
        prep: &[(RCC_IOPENR, 0)],
        write: (RCC_IOPENR, IOPCEN),
        read_addr: RCC_IOPENR,
        mask: IOPCEN,
        expect: IOPCEN,
    },
    MmioCase {
        label: "RCC.APB1ENR TIM2EN @0x38",
        prep: &[(RCC_APB1ENR, 0)],
        write: (RCC_APB1ENR, TIM2EN),
        read_addr: RCC_APB1ENR,
        mask: TIM2EN,
        expect: TIM2EN,
    },
    MmioCase {
        label: "RCC.APB1ENR USART2EN",
        prep: &[(RCC_APB1ENR, 0)],
        write: (RCC_APB1ENR, USART2EN),
        read_addr: RCC_APB1ENR,
        mask: USART2EN,
        expect: USART2EN,
    },
    MmioCase {
        label: "RCC.APB2ENR SPI1EN @0x34",
        prep: &[(RCC_APB2ENR, 0)],
        write: (RCC_APB2ENR, SPI1EN),
        read_addr: RCC_APB2ENR,
        mask: SPI1EN,
        expect: SPI1EN,
    },
    MmioCase {
        label: "RCC.APB2ENR TIM21EN",
        prep: &[(RCC_APB2ENR, 0)],
        write: (RCC_APB2ENR, TIM21EN),
        read_addr: RCC_APB2ENR,
        mask: TIM21EN,
        expect: TIM21EN,
    },
    MmioCase {
        label: "RCC.AHBENR CRCEN @0x30",
        prep: &[(RCC_AHBENR, 0)],
        write: (RCC_AHBENR, CRCEN),
        read_addr: RCC_AHBENR,
        mask: CRCEN,
        expect: CRCEN,
    },
    // ── RCC clock switch: SW -> SWS readback (the L0 RCC layout headline) ──
    MmioCase {
        label: "RCC.CFGR SW=HSI16 -> SWS=HSI16 (clock switch)",
        prep: &[(RCC_CR, CR_MSI_ON_RDY | CR_HSI16ON)],
        write: (RCC_CFGR, CFGR_SW_HSI16),
        read_addr: RCC_CFGR,
        mask: CFGR_SWS_MASK,
        expect: CFGR_SWS_HSI16,
    },
    // ── GPIOA output data path (PA5 / LD2) on the IOPORT bus ──
    MmioCase {
        label: "GPIOA BSRR set PA5 -> ODR bit5",
        prep: &[(RCC_IOPENR, IOPAEN), (GPIOA_ODR, 0)],
        write: (GPIOA_BSRR, PA5),
        read_addr: GPIOA_ODR,
        mask: PA5,
        expect: PA5,
    },
    MmioCase {
        label: "GPIOA BSRR reset PA5 (high half) -> ODR bit5=0",
        prep: &[(RCC_IOPENR, IOPAEN), (GPIOA_ODR, PA5)],
        write: (GPIOA_BSRR, PA5 << 16),
        read_addr: GPIOA_ODR,
        mask: PA5,
        expect: 0,
    },
    MmioCase {
        label: "GPIOA BRR reset PA5 -> ODR bit5=0",
        prep: &[(RCC_IOPENR, IOPAEN), (GPIOA_ODR, PA5)],
        write: (GPIOA_BRR, PA5),
        read_addr: GPIOA_ODR,
        mask: PA5,
        expect: 0,
    },
    // ── SPI1 control registers — classic SPI, CR2 reset 0x0000 ──
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
    // ── TIM2 (16-bit on L0) / TIM21 (16-bit) time-base (CEN left off) ──
    // STM32L0 TIM2 is 16-bit (silicon-confirmed 2026-06-17: writing 0x12345
    // reads back 0x2345 — top 16 bits are not implemented).
    MmioCase {
        label: "TIM2 ARR (16-bit reload)",
        prep: &[(RCC_APB1ENR, TIM2EN)],
        write: (TIM2_ARR, 0x0001_2345),
        read_addr: TIM2_ARR,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_2345,
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
    MmioCase {
        label: "TIM21 ARR (16-bit reload truncates)",
        prep: &[(RCC_APB2ENR, TIM21EN)],
        write: (TIM21_ARR, 0x0000_ABCD),
        read_addr: TIM21_ARR,
        mask: 0xFFFF,
        expect: 0xABCD,
    },
    // ── DBGMCU identity (read-only) — the hardware oracle ──
    MmioCase {
        label: "DBGMCU IDCODE = 0x20086447 (read-only)",
        prep: &[],
        write: (DBGMCU_IDCODE, 0), // ignored (read-only)
        read_addr: DBGMCU_IDCODE,
        mask: 0xFFFF_FFFF,
        expect: L073_IDCODE,
    },
];

// ── Register parity sweep (GPIOB — no SWD pins, safe to reconfigure) ──────────
const PARITY_PATTERNS: &[u32] = &[0x0000_0000, 0xFFFF_FFFF, 0xA5A5_A5A5, 0x5A5A_5A5A];

const ENABLE_PREAMBLE: &[(u32, u32)] = &[
    (RCC_IOPENR, IOPAEN | IOPBEN | IOPCEN),
    (RCC_APB2ENR, SPI1EN),
    (RCC_APB1ENR, TIM2EN),
];

struct ParityReg {
    label: &'static str,
    addr: u32,
    mask: u32,
}

const PARITY_REGS: &[ParityReg] = &[
    // GPIOB config registers (PB has no SWD pins). MODER/OSPEEDR/PUPDR 32-bit;
    // OTYPER/ODR 16-bit; AFRL/AFRH 32-bit.
    ParityReg {
        label: "GPIOB MODER",
        addr: GPIOB_BASE + 0x00,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOB OTYPER",
        addr: GPIOB_BASE + 0x04,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "GPIOB OSPEEDR",
        addr: GPIOB_BASE + 0x08,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOB PUPDR",
        addr: GPIOB_BASE + 0x0C,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOB ODR",
        addr: GPIOB_BASE + 0x14,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "GPIOB AFRL",
        addr: GPIOB_BASE + 0x20,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOB AFRH",
        addr: GPIOB_BASE + 0x24,
        mask: 0xFFFF_FFFF,
    },
    // SPI1 control registers — classic SPI (same peripheral as F103). CR1 0xFFFF
    // (CRCNEXT bit 12 IS writable — genuine-ST F103 confirmation 2026-06-17;
    // L073 shares the model, bit 12 not separately re-diffed here), CR2 0xE7.
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
    // TIM2 (32-bit) data registers; CEN never set so CNT is stable.
    ParityReg {
        label: "TIM2 PSC",
        addr: TIM2_BASE + 0x28,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "TIM2 ARR",
        addr: TIM2_BASE + 0x2C,
        mask: 0x0000_FFFF, // L0 TIM2 is 16-bit (silicon-confirmed 2026-06-17)
    },
    ParityReg {
        label: "TIM2 CCR1",
        addr: TIM2_BASE + 0x34,
        mask: 0x0000_FFFF, // L0 TIM2 is 16-bit (silicon-confirmed 2026-06-17)
    },
];

// ── Sim bus construction ───────────────────────────────────────────────────────

fn build_sim_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/stm32l073.yaml");
    let system_path = manifest_dir.join("../../configs/systems/nucleo-l073rz.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load manifest {system_path:?}: {e}"));

    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build sim bus: {e}"))
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
fn l0_mmio_sim_only() {
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
        "L073 sim MMIO model diverged from spec in {} case(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn l0_parity_sim_only() {
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
        "L073 sim parity self-check failed in {} case(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ── Sim-vs-hardware diff (requires connected NUCLEO-L073RZ) ─────────────────────

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

    fn run_case(sim: &mut SystemBus, oc: &mut OpenOcd, case: &MmioCase) -> Outcome {
        for &(addr, val) in case.prep {
            write_both(sim, oc, addr, val);
        }
        // Read-only registers (write == read addr with expect != written) must
        // not be written on hardware; here only the IDCODE case is read-only and
        // its write value is 0/ignored, so writing is harmless. Keep it simple.
        write_both(sim, oc, case.write.0, case.write.1);

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
    #[ignore = "hw-oracle: requires connected NUCLEO-L073RZ"]
    fn l0_mmio_diff() {
        let _guard = HW_LOCK.lock().unwrap();

        let mut sim = build_sim_bus();
        let mut oc = OpenOcd::spawn_stm32("stm32l0").expect("openocd spawn_stm32 stm32l0");
        oc.reset_halt().expect("reset halt failed");
        oc.halt().expect("halt failed");

        println!();
        println!("STM32L073 MMIO diff — {} cases", CASES.len());
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
            "summary: match={matched} diverge={diverged} both_disagree={disagree} sim_err={sim_err} total={}",
            CASES.len()
        );

        oc.shutdown().ok();

        if std::env::var("L073_STRICT").is_ok() {
            assert_eq!(diverged, 0, "MMIO diff: {diverged} register(s) diverged");
            assert_eq!(sim_err, 0, "MMIO diff: {sim_err} sim error(s)");
        }
    }
}
