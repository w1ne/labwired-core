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
/// `expect` on both sim and hardware.
struct MmioCase {
    label: &'static str,
    prep: &'static [(u32, u32)],
    write: (u32, u32),
    read_addr: u32,
    mask: u32,
    expect: u32,
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
    },
    MmioCase {
        label: "GPIOB BSRR set PB0 -> ODR bit0",
        prep: &[(RCC_AHB2ENR, AHB2ENR_RESET | GPIOBEN), (GPIOB_ODR, 0)],
        write: (GPIOB_BSRR, PB0),
        read_addr: GPIOB_ODR,
        mask: PB0,
        expect: PB0,
    },
    MmioCase {
        label: "GPIOB BSRR reset PB0 (high half) -> ODR bit0=0",
        prep: &[(RCC_AHB2ENR, AHB2ENR_RESET | GPIOBEN), (GPIOB_ODR, PB0)],
        write: (GPIOB_BSRR, PB0 << 16),
        read_addr: GPIOB_ODR,
        mask: PB0,
        expect: 0,
    },
    MmioCase {
        label: "GPIOB BRR reset PB0 -> ODR bit0=0",
        prep: &[(RCC_AHB2ENR, AHB2ENR_RESET | GPIOBEN), (GPIOB_ODR, PB0)],
        write: (GPIOB_BRR, PB0),
        read_addr: GPIOB_ODR,
        mask: PB0,
        expect: 0,
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
    },
    MmioCase {
        label: "GPIOG BSRR set PG4 -> ODR bit4",
        prep: &[(RCC_AHB2ENR, AHB2ENR_RESET | GPIOGEN), (GPIOG_ODR, 0)],
        write: (GPIOG_BSRR, PG4),
        read_addr: GPIOG_ODR,
        mask: PG4,
        expect: PG4,
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

// ── Sim bus construction ──────────────────────────────────────────────────────

fn build_sim_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/stm32h563.yaml");
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let manifest = SystemManifest {
        walk_deleted: false,
        schema_version: "1.0".to_string(),
        name: "h563-mmio-diff".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
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
}
