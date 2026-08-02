// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! STM32L476 (NUCLEO-L476RG) MMIO peripheral diff oracle.
//!
//! Register-level cross-validation of the L4 peripheral models the Space
//! Invaders / Nokia-5110 demo exercises — RCC clock-enables, GPIO output
//! data path (BSRR/BRR/ODR), SPI1 control registers, and the TIM2 time-base
//! registers — against real NUCLEO-L476RG silicon over ST-Link SWD.
//!
//! Philosophy mirrors `nrf52_mmio_diff.rs`: no instruction execution, just
//! register write/read against the modeled peripherals on both sides, then a
//! masked comparison. Complements `firmware_survival.rs` (which runs whole
//! L476 ELFs against silicon-captured UART output) by pinning individual
//! register semantics.
//!
//! ## Running
//!
//! Sim-only (no hardware — runs in normal CI):
//! ```text
//! cargo test -p labwired-hw-oracle --test l476_mmio_diff l476_mmio_sim_only
//! ```
//!
//! Full sim-vs-hardware diff (NUCLEO-L476RG connected via its on-board
//! ST-Link/V2-1):
//! ```text
//! cargo test -p labwired-hw-oracle --test l476_mmio_diff \
//!     --features hw-oracle-stm32 -- --ignored --nocapture
//! ```
//! Set `L476_STRICT=1` to make register divergence a hard failure.
//!
//! Hardware validated against: NUCLEO-L476RG (STM32L476RG, Cortex-M4F,
//! DBGMCU IDCODE 0x10076415).

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

// ── RCC (0x4002_1000) ────────────────────────────────────────────────────────
const RCC_BASE: u32 = 0x4002_1000;
const RCC_APB1ENR1: u32 = RCC_BASE + 0x58; // TIM2EN @ bit 0
const RCC_APB2ENR: u32 = RCC_BASE + 0x60; // SPI1EN @ bit 12
const RCC_AHB2ENR: u32 = RCC_BASE + 0x4C; // GPIOxEN @ bits 0.. (A=0,B=1,C=2,…)

const GPIOAEN: u32 = 1 << 0;
const GPIOCEN: u32 = 1 << 2;
const SPI1EN: u32 = 1 << 12;
const TIM2EN: u32 = 1 << 0;

// ── GPIO (stm32v2 layout) ──────────────────────────────────────────────────────
const GPIOA_BASE: u32 = 0x4800_0000;
const GPIOC_BASE: u32 = 0x4800_0800;
const GPIO_ODR: u32 = 0x14;
const GPIO_BSRR: u32 = 0x18;
const GPIO_BRR: u32 = 0x28;

const GPIOA_ODR: u32 = GPIOA_BASE + GPIO_ODR;
const GPIOA_BSRR: u32 = GPIOA_BASE + GPIO_BSRR;
const GPIOA_BRR: u32 = GPIOA_BASE + GPIO_BRR;
const GPIOC_ODR: u32 = GPIOC_BASE + GPIO_ODR;
const GPIOC_BSRR: u32 = GPIOC_BASE + GPIO_BSRR;

// PA5 = LD2 (the user LED — safe to drive). NOT a SWD pin (those are
// PA13/PA14); we deliberately exercise only the atomic BSRR/BRR/ODR data
// path and never raw MODER/AFR, so the live SWD debug connection the diff
// itself rides on is never disturbed.
const PA5: u32 = 1 << 5;
const PC8: u32 = 1 << 8; // free pin on the Nucleo morpho header

// ── SPI1 (0x4001_3000) ─────────────────────────────────────────────────────────
const SPI1_BASE: u32 = 0x4001_3000;
const SPI1_CR1: u32 = SPI1_BASE;
const SPI1_CR2: u32 = SPI1_BASE + 0x04;

// ── TIM2 (0x4000_0000, 32-bit GP timer) ─────────────────────────────────────────
const TIM2_BASE: u32 = 0x4000_0000;
const TIM2_CR1: u32 = TIM2_BASE;
const TIM2_PSC: u32 = TIM2_BASE + 0x28;
const TIM2_ARR: u32 = TIM2_BASE + 0x2C;

// ── Case definition ──────────────────────────────────────────────────────────

/// A single MMIO probe: optional prep writes (pin pre-state / enable clocks),
/// the write under test, then a masked readback that must match `expect` on
/// both sim and hardware.
struct MmioCase {
    label: &'static str,
    prep: &'static [(u32, u32)],
    write: (u32, u32),
    read_addr: u32,
    mask: u32,
    expect: u32,
}

const CASES: &[MmioCase] = &[
    // ── RCC clock-enable registers (the gate for every peripheral below) ──
    MmioCase {
        label: "RCC.AHB2ENR GPIOAEN",
        prep: &[(RCC_AHB2ENR, 0)],
        write: (RCC_AHB2ENR, GPIOAEN),
        read_addr: RCC_AHB2ENR,
        mask: GPIOAEN,
        expect: GPIOAEN,
    },
    MmioCase {
        label: "RCC.AHB2ENR GPIOCEN",
        prep: &[(RCC_AHB2ENR, 0)],
        write: (RCC_AHB2ENR, GPIOCEN),
        read_addr: RCC_AHB2ENR,
        mask: GPIOCEN,
        expect: GPIOCEN,
    },
    MmioCase {
        label: "RCC.APB2ENR SPI1EN",
        prep: &[(RCC_APB2ENR, 0)],
        write: (RCC_APB2ENR, SPI1EN),
        read_addr: RCC_APB2ENR,
        mask: SPI1EN,
        expect: SPI1EN,
    },
    MmioCase {
        label: "RCC.APB1ENR1 TIM2EN",
        prep: &[(RCC_APB1ENR1, 0)],
        write: (RCC_APB1ENR1, TIM2EN),
        read_addr: RCC_APB1ENR1,
        mask: TIM2EN,
        expect: TIM2EN,
    },
    // ── GPIOA output data path (PA5 / LD2) ──
    MmioCase {
        label: "GPIOA BSRR set PA5 -> ODR bit5",
        prep: &[(RCC_AHB2ENR, GPIOAEN), (GPIOA_ODR, 0)],
        write: (GPIOA_BSRR, PA5),
        read_addr: GPIOA_ODR,
        mask: PA5,
        expect: PA5,
    },
    MmioCase {
        label: "GPIOA BSRR reset PA5 (high half) -> ODR bit5=0",
        prep: &[(RCC_AHB2ENR, GPIOAEN), (GPIOA_ODR, PA5)],
        write: (GPIOA_BSRR, PA5 << 16),
        read_addr: GPIOA_ODR,
        mask: PA5,
        expect: 0,
    },
    MmioCase {
        label: "GPIOA BRR reset PA5 -> ODR bit5=0",
        prep: &[(RCC_AHB2ENR, GPIOAEN), (GPIOA_ODR, PA5)],
        write: (GPIOA_BRR, PA5),
        read_addr: GPIOA_ODR,
        mask: PA5,
        expect: 0,
    },
    MmioCase {
        label: "GPIOA ODR direct write PA5",
        prep: &[(RCC_AHB2ENR, GPIOAEN), (GPIOA_ODR, 0)],
        write: (GPIOA_ODR, PA5),
        read_addr: GPIOA_ODR,
        mask: PA5,
        expect: PA5,
    },
    // ── GPIOC output data path (PC8 / free morpho pin) ──
    MmioCase {
        label: "GPIOC BSRR set PC8 -> ODR bit8",
        prep: &[(RCC_AHB2ENR, GPIOCEN), (GPIOC_ODR, 0)],
        write: (GPIOC_BSRR, PC8),
        read_addr: GPIOC_ODR,
        mask: PC8,
        expect: PC8,
    },
    // ── SPI1 control registers (clock enabled, SPE off → fully writable) ──
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
        write: (SPI1_CR1, 0x0038), // BR[5:3] = 0b111
        read_addr: SPI1_CR1,
        mask: 0x0038,
        expect: 0x0038,
    },
    MmioCase {
        label: "SPI1 CR2 DS=16-bit",
        prep: &[(RCC_APB2ENR, SPI1EN), (SPI1_CR2, 0x0700)],
        write: (SPI1_CR2, 0x0F00), // DS[11:8] = 0b1111 (16-bit)
        read_addr: SPI1_CR2,
        mask: 0x0F00,
        expect: 0x0F00,
    },
    // ── TIM2 time-base registers (CEN left off so CNT does not run) ──
    MmioCase {
        label: "TIM2 ARR (32-bit reload)",
        prep: &[(RCC_APB1ENR1, TIM2EN)],
        write: (TIM2_ARR, 0x0001_2345),
        read_addr: TIM2_ARR,
        mask: 0xFFFF_FFFF,
        expect: 0x0001_2345,
    },
    MmioCase {
        label: "TIM2 PSC (prescaler)",
        prep: &[(RCC_APB1ENR1, TIM2EN)],
        write: (TIM2_PSC, 0x0000_0050),
        read_addr: TIM2_PSC,
        mask: 0xFFFF,
        expect: 0x0050,
    },
    MmioCase {
        label: "TIM2 CR1 ARPE (no CEN)",
        prep: &[(RCC_APB1ENR1, TIM2EN), (TIM2_CR1, 0)],
        write: (TIM2_CR1, 0x0080), // ARPE (bit 7) — does not start the counter
        read_addr: TIM2_CR1,
        mask: 0x0080,
        expect: 0x0080,
    },
];

// ── Register parity sweep ──────────────────────────────────────────────────────
//
// One-to-one parity: write complementary bit patterns to every R/W register
// the demo touches, on BOTH sim and silicon, and require the masked readback
// to agree bit-for-bit. `mask` is the register's implemented/writable bits —
// reserved bits (which silicon reads back as 0) are excluded so the probe
// targets modeled behaviour, not undefined fields.

const GPIOD_BASE: u32 = 0x4800_0C00; // debug-pin-free ports — safe to reconfigure
const GPIOE_BASE: u32 = 0x4800_1000;
const SPI2_BASE: u32 = 0x4000_3800;
const SPI3_BASE: u32 = 0x4000_3C00;

/// Complementary patterns: all-clear, all-set, and the two 0xA5/0x5A
/// alternating masks so every bit is exercised both 0 and 1.
const PARITY_PATTERNS: &[u32] = &[0x0000_0000, 0xFFFF_FFFF, 0xA5A5_A5A5, 0x5A5A_5A5A];

/// Enable every peripheral clock the sweep touches in one shot (GPIOA-E/H,
/// SPI1/2/3, TIM2) so all swept registers are accessible.
const ENABLE_PREAMBLE: &[(u32, u32)] = &[
    (RCC_AHB2ENR, 0x0000_009F), // GPIOA(0) B(1) C(2) D(3) E(4) H(7)
    (RCC_APB2ENR, 1 << 12),     // SPI1
    (RCC_APB1ENR1, (1 << 0) | (1 << 14) | (1 << 15)), // TIM2, SPI2, SPI3
];

struct ParityReg {
    label: &'static str,
    addr: u32,
    mask: u32,
}

const PARITY_REGS: &[ParityReg] = &[
    // GPIO configuration registers on debug-pin-free ports D and E. (GPIOA's
    // PA13/PA14 carry SWD — never swept here, the J-Link rides on them.)
    ParityReg {
        label: "GPIOD MODER",
        addr: GPIOD_BASE,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOD OTYPER",
        addr: GPIOD_BASE + 0x04,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "GPIOD OSPEEDR",
        addr: GPIOD_BASE + 0x08,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOD PUPDR",
        addr: GPIOD_BASE + 0x0C,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOD ODR",
        addr: GPIOD_BASE + 0x14,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "GPIOD AFRL",
        addr: GPIOD_BASE + 0x20,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOD AFRH",
        addr: GPIOD_BASE + 0x24,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOE MODER",
        addr: GPIOE_BASE,
        mask: 0xFFFF_FFFF,
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
        label: "GPIOE AFRL",
        addr: GPIOE_BASE + 0x20,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "GPIOE AFRH",
        addr: GPIOE_BASE + 0x24,
        mask: 0xFFFF_FFFF,
    },
    // SPI control registers (CR2 bit 15 reserved on L4 → masked out).
    ParityReg {
        label: "SPI1 CR1",
        addr: SPI1_BASE,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "SPI1 CR2",
        addr: SPI1_BASE + 0x04,
        // DS[11:8] is forced to >= 8-bit (0b0111) by hardware (RM0351 §40.6.2),
        // so it isn't a clean write==read field — exclude it from the sweep.
        mask: 0x0000_70FF,
    },
    ParityReg {
        label: "SPI2 CR1",
        addr: SPI2_BASE,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "SPI2 CR2",
        addr: SPI2_BASE + 0x04,
        mask: 0x0000_70FF, // DS[11:8] hardware-forced to >= 8-bit (RM0351)
    },
    ParityReg {
        label: "SPI3 CR1",
        addr: SPI3_BASE,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "SPI3 CR2",
        addr: SPI3_BASE + 0x04,
        mask: 0x0000_70FF, // DS[11:8] hardware-forced to >= 8-bit (RM0351)
    },
    // TIM2 (32-bit GP timer) data registers. CEN is never set, so CNT is
    // stable; CCMR masked to the low 16 implemented bits.
    ParityReg {
        label: "TIM2 PSC",
        addr: TIM2_BASE + 0x28,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "TIM2 ARR",
        addr: TIM2_BASE + 0x2C,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "TIM2 CCR1",
        addr: TIM2_BASE + 0x34,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "TIM2 CCR2",
        addr: TIM2_BASE + 0x38,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "TIM2 CCR3",
        addr: TIM2_BASE + 0x3C,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "TIM2 CCR4",
        addr: TIM2_BASE + 0x40,
        mask: 0xFFFF_FFFF,
    },
    ParityReg {
        label: "TIM2 CCMR1",
        addr: TIM2_BASE + 0x18,
        mask: 0x0000_FFFF,
    },
    ParityReg {
        label: "TIM2 CCMR2",
        addr: TIM2_BASE + 0x1C,
        mask: 0x0000_FFFF,
    },
];

// ── Sim bus construction ───────────────────────────────────────────────────────

fn build_sim_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/stm32l476.yaml");
    let system_path = manifest_dir.join("../../configs/systems/nucleo-l476rg.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load manifest {system_path:?}: {e}"));

    // Anchor the relative chip path in the manifest to an absolute path so
    // SystemBus::from_config resolves it regardless of CWD.
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build sim bus: {e}"))
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
    let v = sim
        .read_u32(case.read_addr as u64)
        .unwrap_or_else(|e| panic!("sim read 0x{:08X}: {e:?}", case.read_addr));
    v & case.mask
}

// ── Sim-only test (no hardware — runs in normal CI) ────────────────────────────

/// Validate that the L476 sim model produces the spec value for every case.
/// This proves the harness + register semantics independent of hardware; the
/// `--ignored` HW diff below then confirms the same against real silicon.
#[test]
fn l476_mmio_sim_only() {
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
        "L476 sim MMIO model diverged from spec in {} case(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// One-to-one sim parity self-check: every swept register must behave as a
/// clean read/write register over its mask in the sim model. The `_parity_diff`
/// test confirms the same bits against real silicon.
#[test]
fn l476_parity_sim_only() {
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
        "L476 sim parity self-check failed in {} case(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ── Sim-vs-hardware diff (requires connected NUCLEO-L476RG) ─────────────────────

#[cfg(feature = "hw-oracle-stm32")]
mod hw {
    use super::*;
    use labwired_hw_oracle::openocd::OpenOcd;
    use std::sync::Mutex;

    // Serialise hardware tests: only one OpenOCD instance can hold the
    // ST-Link at a time, and cargo runs tests in parallel by default.
    static HW_LOCK: Mutex<()> = Mutex::new(());

    /// Spawn OpenOCD for the connected probe. `L476_PROBE={jlink,stlink}`;
    /// defaults to ST-Link (stock Nucleo). Use "jlink" for a SEGGER J-Link
    /// (incl. a Nucleo on-board ST-Link reflashed to J-Link firmware).
    fn spawn_probe() -> OpenOcd {
        match std::env::var("L476_PROBE").as_deref() {
            Ok("jlink") => OpenOcd::spawn_stm32l4_jlink().expect("openocd spawn_stm32l4_jlink"),
            _ => OpenOcd::spawn_stm32l4().expect("openocd spawn_stm32l4"),
        }
    }

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
    #[ignore = "hw-oracle: requires connected NUCLEO-L476RG"]
    fn l476_mmio_diff() {
        let _guard = HW_LOCK.lock().unwrap();

        let mut sim = build_sim_bus();
        let mut oc = spawn_probe();

        // Halt the CPU so we have exclusive control over the peripheral regs.
        oc.reset_halt().expect("reset halt failed");
        oc.halt().expect("halt failed");

        println!();
        println!("STM32L476 MMIO diff — {} cases", CASES.len());
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

        if std::env::var("L476_STRICT").is_ok() {
            assert_eq!(diverged, 0, "MMIO diff: {diverged} register(s) diverged");
            assert_eq!(sim_err, 0, "MMIO diff: {sim_err} sim error(s)");
        }
    }

    /// One-to-one register parity sweep: complementary bit patterns into every
    /// R/W register the demo touches, sim vs silicon, masked to implemented
    /// bits. The strongest fidelity check — it pins individual bit behaviour,
    /// not just whole-register values.
    #[test]
    #[ignore = "hw-oracle: requires connected NUCLEO-L476RG"]
    fn l476_parity_diff() {
        let _guard = HW_LOCK.lock().unwrap();

        let mut sim = build_sim_bus();
        let mut oc = spawn_probe();
        oc.reset_halt().expect("reset halt failed");
        oc.halt().expect("halt failed");

        // Enable every peripheral clock the sweep touches.
        for &(addr, val) in ENABLE_PREAMBLE {
            write_both(&mut sim, &mut oc, addr, val);
        }

        println!();
        println!(
            "STM32L476 register parity sweep — {} regs x {} patterns",
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

        if std::env::var("L476_STRICT").is_ok() {
            assert_eq!(
                diverged, 0,
                "parity sweep: {diverged} register-pattern(s) diverged"
            );
        }
    }
}
