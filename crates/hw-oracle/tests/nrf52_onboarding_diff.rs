// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF52840 onboarding-peripheral diff oracle.
//!
//! Companion to `nrf52_mmio_diff.rs`. That test validates the production
//! chip yaml's 4 modeled peripherals (UART/SPI/GPIO/GPIO1). This test
//! loads `configs/chips/onboarding/nrf52840.yaml` — which claims 24
//! peripherals (TIMER0-4, RTC0-2, RNG, WDT, PPI, CLOCK, GPIOTE, I2S, PDM,
//! RADIO, ECB, plus the four already covered) — and probes one or two
//! Nordic-spec registers per peripheral against real silicon over SWD.
//!
//! Purpose: produce a per-peripheral pass/fail matrix that tells us
//! which `nrf52840_*` types in the dispatcher genuinely model Nordic
//! behaviour and which were silently routed to the wrong (STM32) layout.
//!
//! Run:
//! ```text
//! cargo test -p labwired-hw-oracle --test nrf52_onboarding_diff \
//!     --features hw-oracle-nrf52 -- --ignored --nocapture
//! ```

#![cfg(feature = "hw-oracle-nrf52")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_hw_oracle::openocd::OpenOcd;
use std::path::PathBuf;
use std::sync::Mutex;

// ── Nordic MMIO bases (PS rev 1.7) ───────────────────────────────────────────

const TIMER0: u32 = 0x4000_8000;
const RTC0: u32 = 0x4000_B000;
const RNG: u32 = 0x4000_D000;
const WDT: u32 = 0x4001_0000;
const PPI: u32 = 0x4001_F000;
const PDM: u32 = 0x4001_D000;
const GPIOTE: u32 = 0x4000_6000;
const ECB: u32 = 0x4000_E000;
const TEMP: u32 = 0x4000_C000;
const SAADC: u32 = 0x4000_7000;
const PWM0: u32 = 0x4001_C000;
const QSPI: u32 = 0x4002_9000;
const NFCT: u32 = 0x4000_5000;
const COMP: u32 = 0x4001_3000;
const QDEC: u32 = 0x4001_2000;
const EGU0: u32 = 0x4001_4000;
const FICR: u32 = 0x1000_0000;
const NVMC: u32 = 0x4001_E000;
const USBD: u32 = 0x4002_7000;
const ACL: u32 = 0x4002_F000;
const CRYPTOCELL: u32 = 0x5002_A000;
const MWU: u32 = 0x4002_0000;
const RADIO: u32 = 0x4000_1000;
const AAR: u32 = 0x4000_F000;

// ── Case structure (mirrors nrf52_mmio_diff.rs) ──────────────────────────────

struct MmioCase {
    peripheral: &'static str,
    label: &'static str,
    prep: &'static [(u32, u32)],
    write: (u32, u32),
    read_addr: u32,
    mask: u32,
    expect: u32,
}

const CASES: &[MmioCase] = &[
    // ── TIMER0 — Nordic spec ───────────────────────────────────────────────
    MmioCase {
        peripheral: "TIMER0",
        label: "BITMODE = 3 (32-bit)",
        prep: &[(TIMER0 + 0x004, 1)], // TASKS_STOP
        write: (TIMER0 + 0x508, 3),   // BITMODE
        read_addr: TIMER0 + 0x508,
        mask: 0x3,
        expect: 3,
    },
    MmioCase {
        peripheral: "TIMER0",
        label: "PRESCALER = 4",
        prep: &[(TIMER0 + 0x004, 1)],
        write: (TIMER0 + 0x510, 4), // PRESCALER
        read_addr: TIMER0 + 0x510,
        mask: 0xF,
        expect: 4,
    },
    MmioCase {
        peripheral: "TIMER0",
        label: "CC[0] = 0xDEADBEEF",
        prep: &[(TIMER0 + 0x004, 1)],
        write: (TIMER0 + 0x540, 0xDEAD_BEEF), // CC[0]
        read_addr: TIMER0 + 0x540,
        mask: 0xFFFF_FFFF,
        expect: 0xDEAD_BEEF,
    },
    // ── RTC0 — Nordic spec (24-bit counter, 12-bit prescaler) ──────────────
    MmioCase {
        peripheral: "RTC0",
        label: "PRESCALER = 0x100",
        prep: &[(RTC0 + 0x004, 1)], // TASKS_STOP
        write: (RTC0 + 0x508, 0x100),
        read_addr: RTC0 + 0x508,
        mask: 0xFFF,
        expect: 0x100,
    },
    MmioCase {
        peripheral: "RTC0",
        label: "CC[0] = 0x12_3456",
        prep: &[(RTC0 + 0x004, 1)],
        write: (RTC0 + 0x540, 0x12_3456),
        read_addr: RTC0 + 0x540,
        mask: 0x00FF_FFFF,
        expect: 0x12_3456,
    },
    // ── RNG — CONFIG (1-bit DERCEN) ────────────────────────────────────────
    MmioCase {
        peripheral: "RNG",
        label: "CONFIG.DERCEN = 1",
        prep: &[(RNG + 0x004, 1)], // TASKS_STOP
        write: (RNG + 0x504, 1),   // CONFIG
        read_addr: RNG + 0x504,
        mask: 0x1,
        expect: 1,
    },
    // ── WDT — CRV (counter reload value) ───────────────────────────────────
    //
    // WDT is one-shot: once started it can't be reconfigured. We probe CRV
    // before any TASKS_START, which is safe.
    MmioCase {
        peripheral: "WDT",
        label: "CRV = 0x20000 (~4s @ 32.768 kHz)",
        prep: &[],
        write: (WDT + 0x504, 0x0002_0000), // CRV
        read_addr: WDT + 0x504,
        mask: 0xFFFF_FFFF,
        expect: 0x0002_0000,
    },
    MmioCase {
        peripheral: "WDT",
        label: "RREN bit 0 = 1",
        prep: &[],
        write: (WDT + 0x508, 1), // RREN
        read_addr: WDT + 0x508,
        mask: 0xF,
        expect: 1,
    },
    // ── PPI — CHEN (channel enable) ─────────────────────────────────────────
    MmioCase {
        peripheral: "PPI",
        label: "CHENSET ch0+ch2+ch4 -> CHEN reads back",
        prep: &[(PPI + 0x508, 0xFFFF_FFFF)], // CHENCLR all
        write: (PPI + 0x504, 0b0001_0101),   // CHENSET
        read_addr: PPI + 0x500,              // CHEN
        mask: 0xFF,
        expect: 0b0001_0101,
    },
    // ── PDM — PDMCLKCTRL ────────────────────────────────────────────────────
    MmioCase {
        peripheral: "PDM",
        label: "PDMCLKCTRL = 0x0800_0000 (1.000 MHz)",
        prep: &[(PDM + 0x004, 1)], // TASKS_STOP
        write: (PDM + 0x504, 0x0800_0000),
        read_addr: PDM + 0x504,
        mask: 0xFFFF_FFFF,
        expect: 0x0800_0000,
    },
    // ── GPIOTE — CONFIG[0] (Task mode on pin 13, set on toggle) ────────────
    MmioCase {
        peripheral: "GPIOTE",
        label: "CONFIG[0] = task+pin13+polarity-toggle",
        prep: &[],
        // mode=3 (Task) | pin=13 (bits 8..12) | port=0 (bit 13) | polarity=3 (toggle, bits 16..17)
        write: (GPIOTE + 0x510, 0x0003_0D03),
        read_addr: GPIOTE + 0x510,
        mask: 0x0007_1F03,
        expect: 0x0003_0D03,
    },
    // ── ECB — ECBDATAPTR ────────────────────────────────────────────────────
    //
    // ECBDATAPTR holds a pointer to a 48-byte work buffer (16 key + 16
    // plaintext + 16 ciphertext). Any RAM address inside SRAM is valid.
    MmioCase {
        peripheral: "ECB",
        label: "ECBDATAPTR = 0x2000_2000",
        prep: &[],
        write: (ECB + 0x504, 0x2000_2000),
        read_addr: ECB + 0x504,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_2000,
    },
    // ── TEMP — INTENSET bit 0 (DATARDY) ─────────────────────────────────────
    MmioCase {
        peripheral: "TEMP",
        label: "INTENSET.DATARDY = 1",
        prep: &[(TEMP + 0x308, 0xFFFF_FFFF)], // INTENCLR all
        write: (TEMP + 0x304, 1),
        read_addr: TEMP + 0x304,
        mask: 0x1,
        expect: 1,
    },
    // ── SAADC — RESOLUTION = 3 (14-bit) ─────────────────────────────────────
    MmioCase {
        peripheral: "SAADC",
        label: "RESOLUTION = 3 (14-bit)",
        prep: &[(SAADC + 0x500, 0)], // ENABLE = 0
        write: (SAADC + 0x5F0, 3),
        read_addr: SAADC + 0x5F0,
        mask: 0x7,
        expect: 3,
    },
    MmioCase {
        peripheral: "SAADC",
        label: "CH[0].PSELP = 1 (AIN0)",
        prep: &[(SAADC + 0x500, 0)],
        write: (SAADC + 0x510, 1),
        read_addr: SAADC + 0x510,
        mask: 0x1F,
        expect: 1,
    },
    // ── PWM0 — COUNTERTOP = 1000 ────────────────────────────────────────────
    MmioCase {
        peripheral: "PWM0",
        label: "COUNTERTOP = 1000",
        prep: &[(PWM0 + 0x500, 0)], // ENABLE = 0
        write: (PWM0 + 0x508, 1000),
        read_addr: PWM0 + 0x508,
        mask: 0x7FFF,
        expect: 1000,
    },
    MmioCase {
        peripheral: "PWM0",
        label: "PSEL.OUT[0] = P0.13",
        prep: &[(PWM0 + 0x500, 0)],
        write: (PWM0 + 0x560, 13),
        read_addr: PWM0 + 0x560,
        mask: 0x3F,
        expect: 13,
    },
    // ── QSPI — IFCONFIG0 (interface configuration) ──────────────────────────
    MmioCase {
        peripheral: "QSPI",
        label: "IFCONFIG0 = 0x0000_0035",
        prep: &[(QSPI + 0x500, 0)], // ENABLE = 0
        write: (QSPI + 0x544, 0x0000_0035),
        read_addr: QSPI + 0x544,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_0035,
    },
    // ── NFCT — NFCID1_LAST tag identifier ───────────────────────────────────
    MmioCase {
        peripheral: "NFCT",
        label: "NFCID1_LAST = 0xDEAD_BEEF",
        prep: &[],
        write: (NFCT + 0x590, 0xDEAD_BEEF),
        read_addr: NFCT + 0x590,
        mask: 0xFFFF_FFFF,
        expect: 0xDEAD_BEEF,
    },
    // ── COMP — TH (threshold) ──────────────────────────────────────────────
    MmioCase {
        peripheral: "COMP",
        label: "TH = 0x0F0F",
        prep: &[(COMP + 0x500, 0)], // ENABLE = 0
        write: (COMP + 0x530, 0x0F0F),
        read_addr: COMP + 0x530,
        mask: 0x3F3F,
        expect: 0x0F0F,
    },
    // ── QDEC — SAMPLEPER ───────────────────────────────────────────────────
    MmioCase {
        peripheral: "QDEC",
        label: "SAMPLEPER = 7 (1024 us)",
        prep: &[(QDEC + 0x500, 0)],
        write: (QDEC + 0x508, 7),
        read_addr: QDEC + 0x508,
        mask: 0xF,
        expect: 7,
    },
    // ── EGU0 — INTENSET (trigger 0 enabled) ────────────────────────────────
    MmioCase {
        peripheral: "EGU0",
        label: "INTENSET trigger 0 = 1",
        prep: &[(EGU0 + 0x308, 0xFFFF_FFFF)], // INTENCLR all
        write: (EGU0 + 0x304, 1),
        read_addr: EGU0 + 0x304,
        mask: 0x1,
        expect: 1,
    },
    // ── FICR — INFO.PART (read-only chip ID) ───────────────────────────────
    //
    // FICR is RO on silicon and pre-seeded in our model. Writing should
    // be silently dropped on both sides; reading INFO.PART returns the
    // chip family code on the XIAO nRF52840 silicon and 0x52840 in sim.
    MmioCase {
        peripheral: "FICR",
        label: "INFO.PART = 0x52840 (read-only)",
        prep: &[],
        write: (FICR + 0x100, 0xDEAD_BEEF), // should be dropped
        read_addr: FICR + 0x100,
        mask: 0xFFFF_FFFF,
        expect: 0x0005_2840,
    },
    // ── NVMC — READY (idle = 1) ────────────────────────────────────────────
    MmioCase {
        peripheral: "NVMC",
        label: "READY = 1 (no flash op pending)",
        prep: &[],
        write: (NVMC + 0x400, 0), // RO — ignored
        read_addr: NVMC + 0x400,
        mask: 0x1,
        expect: 1,
    },
    MmioCase {
        peripheral: "USBD",
        label: "ENABLE = 1 (USB device on)",
        prep: &[],
        write: (USBD + 0x500, 1),
        read_addr: USBD + 0x500,
        mask: 0x1,
        expect: 1,
    },
    MmioCase {
        peripheral: "ACL",
        label: "ACL[0].ADDR write-only (silicon reads 0)",
        prep: &[],
        write: (ACL + 0x500, 0x0000_1000),
        read_addr: ACL + 0x500,
        mask: 0xFFFF_F000,
        expect: 0,
    },
    MmioCase {
        peripheral: "CRYPTOCELL",
        label: "ENABLE = 1",
        prep: &[],
        write: (CRYPTOCELL + 0x500, 1),
        read_addr: CRYPTOCELL + 0x500,
        mask: 0x1,
        expect: 1,
    },
    MmioCase {
        peripheral: "RADIO",
        label: "FREQUENCY = 0x4E (BLE adv ch 37)",
        prep: &[],
        write: (RADIO + 0x508, 0x4E),
        read_addr: RADIO + 0x508,
        mask: 0xFF,
        expect: 0x4E,
    },
    MmioCase {
        peripheral: "RADIO",
        label: "MODE = 3 (BLE_1Mbit)",
        prep: &[],
        write: (RADIO + 0x510, 3),
        read_addr: RADIO + 0x510,
        mask: 0xF,
        expect: 3,
    },
    MmioCase {
        peripheral: "RADIO",
        label: "BASE0 = 0xCAFEBABE",
        prep: &[],
        write: (RADIO + 0x51C, 0xCAFE_BABE),
        read_addr: RADIO + 0x51C,
        mask: 0xFFFF_FFFF,
        expect: 0xCAFE_BABE,
    },
];

// ── Wiring ───────────────────────────────────────────────────────────────────

static HW_LOCK: Mutex<()> = Mutex::new(());

fn build_onboarding_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/onboarding/nrf52840.yaml");
    let system_path = manifest_dir.join("../../configs/systems/onboarding/nrf52840.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load onboarding chip {chip_path:?}: {e}"));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load onboarding manifest {system_path:?}: {e}"));

    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    SystemBus::from_config(&chip, &manifest)
        .unwrap_or_else(|e| panic!("build onboarding bus: {e}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Match,
    BothDisagreeWithExpect { both: u32 },
    Diverge { sim: u32, hw: u32 },
    SimError(String),
}

fn run_case(sim: &mut SystemBus, oc: &mut OpenOcd, case: &MmioCase) -> Outcome {
    for &(addr, val) in case.prep {
        // Sim may not have the address mapped — that's fine, it's a prep.
        let _ = sim.write_u32(addr as u64, val);
        oc.write_memory(addr, &[val])
            .unwrap_or_else(|e| panic!("hw prep write 0x{addr:08X}: {e}"));
    }

    if let Err(e) = sim.write_u32(case.write.0 as u64, case.write.1) {
        return Outcome::SimError(format!("write 0x{:08X}: {e:?}", case.write.0));
    }
    oc.write_memory(case.write.0, &[case.write.1])
        .unwrap_or_else(|e| panic!("hw write 0x{:08X}: {e}", case.write.0));

    let sim_val = match sim.read_u32(case.read_addr as u64) {
        Ok(v) => v,
        Err(e) => return Outcome::SimError(format!("read 0x{:08X}: {e:?}", case.read_addr)),
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
#[ignore]
fn nrf52840_onboarding_diff() {
    let _guard = HW_LOCK.lock().unwrap();

    let mut sim = build_onboarding_bus();
    let mut oc = OpenOcd::spawn_nrf52().expect("openocd spawn_nrf52 failed");

    oc.reset_halt().expect("reset halt failed");
    oc.halt().expect("halt failed");

    println!();
    println!("nRF52840 onboarding-peripherals diff — {} cases", CASES.len());
    println!("{:-<100}", "");

    let mut by_peripheral: std::collections::BTreeMap<&str, (u32, u32, u32, u32)> =
        std::collections::BTreeMap::new();

    for case in CASES {
        let outcome = run_case(&mut sim, &mut oc, case);
        let bucket = by_peripheral.entry(case.peripheral).or_insert((0, 0, 0, 0));
        match &outcome {
            Outcome::Match => {
                bucket.0 += 1;
                println!("[OK ] {:<7} {}", case.peripheral, case.label);
            }
            Outcome::Diverge { sim, hw } => {
                bucket.1 += 1;
                println!(
                    "[DIFF] {:<6} {}  sim=0x{:08X} hw=0x{:08X} (mask=0x{:08X})",
                    case.peripheral, case.label, sim, hw, case.mask
                );
            }
            Outcome::BothDisagreeWithExpect { both } => {
                bucket.2 += 1;
                println!(
                    "[BOTH] {:<6} {}  both=0x{:08X} expected=0x{:08X}",
                    case.peripheral, case.label, both, case.expect
                );
            }
            Outcome::SimError(msg) => {
                bucket.3 += 1;
                println!("[SIM!] {:<6} {}  {}", case.peripheral, case.label, msg);
            }
        }
    }

    println!("{:-<100}", "");
    println!("per-peripheral verdict:");
    for (per, (ok, diff, both, simerr)) in &by_peripheral {
        let verdict = if *diff == 0 && *simerr == 0 && *both == 0 {
            "MODELLED"
        } else if *simerr > 0 {
            "SIM_BROKEN"
        } else if *diff > 0 {
            "WRONG_LAYOUT"
        } else {
            "SPEC_MISMATCH"
        };
        println!(
            "  {:<8}  ok={ok} diff={diff} both={both} sim_err={simerr}  -> {verdict}",
            per
        );
    }

    oc.shutdown().ok();

    if std::env::var("NRF52_STRICT").is_ok() {
        let total_bad: u32 = by_peripheral
            .values()
            .map(|(_, d, _, s)| d + s)
            .sum();
        assert_eq!(total_bad, 0, "onboarding diff: {total_bad} bad case(s)");
    }
}
