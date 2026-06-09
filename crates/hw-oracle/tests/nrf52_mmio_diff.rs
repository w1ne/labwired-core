// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF52840 MMIO peripheral diff oracle.
//!
//! Drives every MMIO register modeled by `seeed-xiao-nrf52840-sense.yaml`
//! on both the software simulator and the physical chip (over ST-Link SWD),
//! then compares the readback. Mirrors the philosophy of the ESP32-S3 / STM32
//! oracle banks but at a lower abstraction level: no instruction execution,
//! just register-level write/read against modeled peripherals.
//!
//! Run:
//! ```text
//! cargo test -p labwired-hw-oracle --test nrf52_mmio_diff \
//!     --features hw-oracle-nrf52 -- --ignored --nocapture
//! ```
//!
//! Hardware setup validated against: Seeed XIAO nRF52840 Sense connected to
//! an ST-Link/V2 (SWDIO/SWCLK/GND/3V3). CPUID 0x410FC241, FICR INFO.PART
//! 0x52840.

#![cfg(feature = "hw-oracle-nrf52")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_hw_oracle::openocd::OpenOcd;
use std::path::PathBuf;
use std::sync::Mutex;

// ── MMIO base + offset map (nRF52840 PS rev 1.7) ─────────────────────────────

const GPIO0_BASE: u32 = 0x5000_0000;
const GPIO0_OUT: u32 = GPIO0_BASE + 0x504;
const GPIO0_OUTSET: u32 = GPIO0_BASE + 0x508;
const GPIO0_OUTCLR: u32 = GPIO0_BASE + 0x50C;
const GPIO0_DIR: u32 = GPIO0_BASE + 0x514;
const GPIO0_DIRSET: u32 = GPIO0_BASE + 0x518;
const GPIO0_DIRCLR: u32 = GPIO0_BASE + 0x51C;

const UART0_BASE: u32 = 0x4000_2000;
const UART0_ENABLE: u32 = UART0_BASE + 0x500;

const SPIM0_BASE: u32 = 0x4000_3000;
const SPIM0_TASKS_START: u32 = SPIM0_BASE + 0x010;
const SPIM0_EVENTS_END: u32 = SPIM0_BASE + 0x118;
const SPIM0_ENABLE: u32 = SPIM0_BASE + 0x500;
const SPIM0_PSEL_SCK: u32 = SPIM0_BASE + 0x508;
const SPIM0_PSEL_MOSI: u32 = SPIM0_BASE + 0x50C;
const SPIM0_PSEL_MISO: u32 = SPIM0_BASE + 0x510;
const SPIM0_FREQUENCY: u32 = SPIM0_BASE + 0x524;
const SPIM0_TXD_MAXCNT: u32 = SPIM0_BASE + 0x548;

// XIAO Sense LED pins on GPIO0.
const LED_RED: u32 = 1 << 26;
const LED_GREEN: u32 = 1 << 30;
const LED_BLUE: u32 = 1 << 6;

// ── Case definition ──────────────────────────────────────────────────────────

/// A single MMIO probe: optional prep write, the write under test, then a
/// masked readback that must match `expect` on both sim and hw.
struct MmioCase {
    label: &'static str,
    prep: &'static [(u32, u32)],
    write: (u32, u32),
    read_addr: u32,
    mask: u32,
    expect: u32,
}

/// The probe surface — drawn from peripherals touched by
/// `firmware-nrf52840-demo` plus a few corollary regs (DIRCLR/OUTCLR/DIR/OUT)
/// used to pin pre-state on the real chip.
const CASES: &[MmioCase] = &[
    // GPIO0 direction set/clear pairs for each LED pin.
    MmioCase {
        label: "GPIO0 DIRSET pin 26 (LED_RED)",
        prep: &[(GPIO0_DIRCLR, LED_RED)],
        write: (GPIO0_DIRSET, LED_RED),
        read_addr: GPIO0_DIR,
        mask: LED_RED,
        expect: LED_RED,
    },
    MmioCase {
        label: "GPIO0 DIRSET pin 30 (LED_GREEN)",
        prep: &[(GPIO0_DIRCLR, LED_GREEN)],
        write: (GPIO0_DIRSET, LED_GREEN),
        read_addr: GPIO0_DIR,
        mask: LED_GREEN,
        expect: LED_GREEN,
    },
    MmioCase {
        label: "GPIO0 DIRSET pin 6 (LED_BLUE)",
        prep: &[(GPIO0_DIRCLR, LED_BLUE)],
        write: (GPIO0_DIRSET, LED_BLUE),
        read_addr: GPIO0_DIR,
        mask: LED_BLUE,
        expect: LED_BLUE,
    },
    // GPIO0 OUT side-effect from OUTSET / OUTCLR.
    MmioCase {
        label: "GPIO0 OUTSET pin 26 -> OUT bit 26",
        prep: &[(GPIO0_OUTCLR, LED_RED)],
        write: (GPIO0_OUTSET, LED_RED),
        read_addr: GPIO0_OUT,
        mask: LED_RED,
        expect: LED_RED,
    },
    MmioCase {
        label: "GPIO0 OUTCLR pin 26 -> OUT bit 26",
        prep: &[(GPIO0_OUTSET, LED_RED)],
        write: (GPIO0_OUTCLR, LED_RED),
        read_addr: GPIO0_OUT,
        mask: LED_RED,
        expect: 0,
    },
    MmioCase {
        label: "GPIO0 OUTSET pin 30 -> OUT bit 30",
        prep: &[(GPIO0_OUTCLR, LED_GREEN)],
        write: (GPIO0_OUTSET, LED_GREEN),
        read_addr: GPIO0_OUT,
        mask: LED_GREEN,
        expect: LED_GREEN,
    },
    MmioCase {
        label: "GPIO0 OUTSET pin 6 -> OUT bit 6",
        prep: &[(GPIO0_OUTCLR, LED_BLUE)],
        write: (GPIO0_OUTSET, LED_BLUE),
        read_addr: GPIO0_OUT,
        mask: LED_BLUE,
        expect: LED_BLUE,
    },
    // UART0 enable register: direct R/W field, low 4 bits hold ENABLE value.
    MmioCase {
        label: "UART0 ENABLE=4 (UART)",
        prep: &[(UART0_ENABLE, 0)],
        write: (UART0_ENABLE, 4),
        read_addr: UART0_ENABLE,
        mask: 0xF,
        expect: 4,
    },
    MmioCase {
        label: "UART0 ENABLE=0 (disabled)",
        prep: &[(UART0_ENABLE, 4)],
        write: (UART0_ENABLE, 0),
        read_addr: UART0_ENABLE,
        mask: 0xF,
        expect: 0,
    },
    // SPIM0 PSEL_* must be writable while SPIM is disabled (ENABLE=0).
    MmioCase {
        label: "SPIM0 PSEL_SCK = P1.13 (45)",
        prep: &[(SPIM0_ENABLE, 0)],
        write: (SPIM0_PSEL_SCK, 32 + 13),
        read_addr: SPIM0_PSEL_SCK,
        mask: 0x3F, // PIN field is low 6 bits (PORT bit at 5, PIN at 0..4)
        expect: 32 + 13,
    },
    MmioCase {
        label: "SPIM0 PSEL_MOSI = P1.15 (47)",
        prep: &[(SPIM0_ENABLE, 0)],
        write: (SPIM0_PSEL_MOSI, 32 + 15),
        read_addr: SPIM0_PSEL_MOSI,
        mask: 0x3F,
        expect: 32 + 15,
    },
    MmioCase {
        label: "SPIM0 PSEL_MISO = P1.14 (46)",
        prep: &[(SPIM0_ENABLE, 0)],
        write: (SPIM0_PSEL_MISO, 32 + 14),
        read_addr: SPIM0_PSEL_MISO,
        mask: 0x3F,
        expect: 32 + 14,
    },
    MmioCase {
        label: "SPIM0 FREQUENCY = 0x0200_0000 (125 kbps)",
        prep: &[(SPIM0_ENABLE, 0)],
        write: (SPIM0_FREQUENCY, 0x0200_0000),
        read_addr: SPIM0_FREQUENCY,
        mask: 0xFFFF_FFFF,
        expect: 0x0200_0000,
    },
    MmioCase {
        label: "SPIM0 TXD.MAXCNT = 4",
        prep: &[(SPIM0_ENABLE, 0)],
        write: (SPIM0_TXD_MAXCNT, 4),
        read_addr: SPIM0_TXD_MAXCNT,
        mask: 0xFFFF,
        expect: 4,
    },
    MmioCase {
        label: "SPIM0 ENABLE = 7 (SPI Master enabled)",
        prep: &[(SPIM0_ENABLE, 0)],
        write: (SPIM0_ENABLE, 7),
        read_addr: SPIM0_ENABLE,
        mask: 0xF,
        expect: 7,
    },
    MmioCase {
        label: "SPIM0 TASKS_START -> EVENTS_END (sim model)",
        prep: &[
            (SPIM0_ENABLE, 7),
            (SPIM0_TXD_MAXCNT, 4),
            (SPIM0_EVENTS_END, 0),
        ],
        write: (SPIM0_TASKS_START, 1),
        read_addr: SPIM0_EVENTS_END,
        mask: 1,
        expect: 1,
    },
];

// ── Test entry point ─────────────────────────────────────────────────────────

// Serialise hardware tests: only one OpenOCD instance can hold the ST-Link
// at a time, and `cargo test` runs tests in parallel by default.
static HW_LOCK: Mutex<()> = Mutex::new(());

fn build_sim_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/nrf52840.yaml");
    let system_path = manifest_dir.join("../../configs/systems/seeed-xiao-nrf52840-sense.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load manifest {system_path:?}: {e}"));

    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build sim bus: {e}"))
}

/// Outcome of a single MMIO case.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    /// Sim and HW both produced the masked-expected value.
    Match,
    /// Sim and HW agreed but neither matched `expect` — model + chip both
    /// disagree with the spec value the test encoded. Worth flagging because
    /// it points at a wrong expectation, not a sim bug.
    BothDisagreeWithExpect { both: u32 },
    /// Sim and HW disagreed — the sim model is wrong (or HW has unexpected
    /// state).
    Diverge { sim: u32, hw: u32 },
    /// Sim returned an error (likely unmapped peripheral).
    SimError(String),
}

fn write_both(sim: &mut SystemBus, oc: &mut OpenOcd, addr: u32, val: u32) {
    sim.write_u32(addr as u64, val)
        .unwrap_or_else(|e| panic!("sim write 0x{addr:08X} = 0x{val:08X}: {e:?}"));
    oc.write_memory(addr, &[val])
        .unwrap_or_else(|e| panic!("hw write 0x{addr:08X} = 0x{val:08X}: {e}"));
}

fn run_case(sim: &mut SystemBus, oc: &mut OpenOcd, case: &MmioCase) -> Outcome {
    for &(addr, val) in case.prep {
        write_both(sim, oc, addr, val);
    }
    write_both(sim, oc, case.write.0, case.write.1);

    // Let async peripherals (EasyDMA: SPIM/TWIM/ECB/CCM) process the work the
    // write kicked off, so their completion events become observable — the
    // faithful models set e.g. EVENTS_END via the bus-tick hook, not
    // synchronously in the register write. Silicon completes it autonomously.
    let _ = sim.tick_peripherals_fully();

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
#[ignore]
fn nrf52840_mmio_diff() {
    let _guard = HW_LOCK.lock().unwrap();

    let mut sim = build_sim_bus();
    let mut oc = OpenOcd::spawn_nrf52().expect("openocd spawn_nrf52 failed");

    // Halt the CPU so we have exclusive control over peripheral regs.
    oc.reset_halt().expect("reset halt failed");
    oc.halt().expect("halt failed");

    // Header
    println!();
    println!("nRF52840 MMIO diff — {} cases", CASES.len());
    println!("{:-<90}", "");

    let mut matched = 0;
    let mut diverged = 0;
    let mut disagree_expect = 0;
    let mut sim_errors = 0;

    for case in CASES {
        let outcome = run_case(&mut sim, &mut oc, case);
        match &outcome {
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
                disagree_expect += 1;
                println!(
                    "[BOTH] {}  both=0x{:08X} expected=0x{:08X}",
                    case.label, both, case.expect
                );
            }
            Outcome::SimError(msg) => {
                sim_errors += 1;
                println!("[SIM!] {}  sim error: {}", case.label, msg);
            }
        }
    }

    println!("{:-<90}", "");
    println!(
        "summary: match={matched} diverge={diverged} both_disagree={disagree_expect} sim_err={sim_errors} total={}",
        CASES.len()
    );

    oc.shutdown().ok();

    // The test always passes structurally — its purpose is to *report*
    // coverage, not to assert it.  CI can grep the summary line.  If you
    // want a strict mode where divergence fails, set NRF52_STRICT=1.
    if std::env::var("NRF52_STRICT").is_ok() {
        assert_eq!(diverged, 0, "MMIO diff: {diverged} register(s) diverged");
    }
}
