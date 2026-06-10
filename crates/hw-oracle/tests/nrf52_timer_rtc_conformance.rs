// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF52840 TIMER0 + RTC0 full-register sim-vs-silicon conformance.
//!
//! The onboarding sweep only spot-checks a couple of registers per peripheral.
//! TIMER and RTC are workhorse peripherals, so this bank exercises their FULL
//! register set (all CC[], SHORTS, INTEN set/clear, EVENTS, MODE/BITMODE/
//! PRESCALER, COUNTER read-only-ness) for static register fidelity against real
//! silicon. The CPU is reset-halted, so this verifies register layout / masks /
//! reset values / set-clear semantics — not live counting (that is covered by
//! the behavioral `nrf52_conformance` digest).
//!
//! Run (pin the nRF probe when multiple ST-Links are attached):
//! ```text
//! LABWIRED_STLINK_LOCATION=1-2 cargo test -p labwired-hw-oracle \
//!     --test nrf52_timer_rtc_conformance --features hw-oracle-nrf52 \
//!     -- --ignored --nocapture
//! ```

#![cfg(feature = "hw-oracle-nrf52")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_hw_oracle::openocd::OpenOcd;
use std::path::PathBuf;
use std::sync::Mutex;

const TIMER0: u32 = 0x4000_8000;
const RTC0: u32 = 0x4000_B000;

struct Case {
    label: &'static str,
    prep: &'static [(u32, u32)],
    write: (u32, u32),
    read_addr: u32,
    mask: u32,
    expect: u32,
}

const CASES: &[Case] = &[
    // ── TIMER0 (PS §6.30) ─────────────────────────────────────────────────
    Case {
        label: "TIMER0 MODE=Counter(1)",
        prep: &[],
        write: (TIMER0 + 0x504, 1),
        read_addr: TIMER0 + 0x504,
        mask: 0x3,
        expect: 1,
    },
    Case {
        label: "TIMER0 MODE=Timer(0)",
        prep: &[(TIMER0 + 0x504, 1)],
        write: (TIMER0 + 0x504, 0),
        read_addr: TIMER0 + 0x504,
        mask: 0x3,
        expect: 0,
    },
    Case {
        label: "TIMER0 BITMODE=32bit(3)",
        prep: &[],
        write: (TIMER0 + 0x508, 3),
        read_addr: TIMER0 + 0x508,
        mask: 0x3,
        expect: 3,
    },
    Case {
        label: "TIMER0 PRESCALER=9",
        prep: &[],
        write: (TIMER0 + 0x510, 9),
        read_addr: TIMER0 + 0x510,
        mask: 0xF,
        expect: 9,
    },
    Case {
        label: "TIMER0 CC[0]",
        prep: &[],
        write: (TIMER0 + 0x540, 0x1111_1111),
        read_addr: TIMER0 + 0x540,
        mask: 0xFFFF_FFFF,
        expect: 0x1111_1111,
    },
    Case {
        label: "TIMER0 CC[1]",
        prep: &[],
        write: (TIMER0 + 0x544, 0x2222_2222),
        read_addr: TIMER0 + 0x544,
        mask: 0xFFFF_FFFF,
        expect: 0x2222_2222,
    },
    Case {
        label: "TIMER0 CC[2]",
        prep: &[],
        write: (TIMER0 + 0x548, 0x3333_3333),
        read_addr: TIMER0 + 0x548,
        mask: 0xFFFF_FFFF,
        expect: 0x3333_3333,
    },
    Case {
        label: "TIMER0 CC[3]",
        prep: &[],
        write: (TIMER0 + 0x54C, 0x4444_4444),
        read_addr: TIMER0 + 0x54C,
        mask: 0xFFFF_FFFF,
        expect: 0x4444_4444,
    },
    Case {
        label: "TIMER0 CC[4] absent (4-CC timer) -> 0",
        prep: &[],
        write: (TIMER0 + 0x550, 0x5555_5555),
        read_addr: TIMER0 + 0x550,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    Case {
        label: "TIMER0 CC[5] absent (4-CC timer) -> 0",
        prep: &[],
        write: (TIMER0 + 0x554, 0x6666_6666),
        read_addr: TIMER0 + 0x554,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    // SHORTS: COMPARE[0..5]_CLEAR (bits 0..5) + COMPARE[0..5]_STOP (bits 8..13).
    Case {
        label: "TIMER0 SHORTS",
        prep: &[],
        write: (TIMER0 + 0x200, 0x0000_3F3F),
        read_addr: TIMER0 + 0x200,
        mask: 0x0000_3F3F,
        expect: 0x0000_0F0F,
    },
    // INTEN via INTENSET (compare interrupts at bits 16..21); read returns mask.
    Case {
        label: "TIMER0 INTENSET COMPARE0..5",
        prep: &[(TIMER0 + 0x308, 0xFFFF_FFFF)],
        write: (TIMER0 + 0x304, 0x003F_0000),
        read_addr: TIMER0 + 0x304,
        mask: 0x003F_0000,
        expect: 0x000F_0000,
    },
    Case {
        label: "TIMER0 INTENCLR COMPARE0..5",
        prep: &[(TIMER0 + 0x304, 0x003F_0000)],
        write: (TIMER0 + 0x308, 0x003F_0000),
        read_addr: TIMER0 + 0x304,
        mask: 0x003F_0000,
        expect: 0,
    },
    Case {
        label: "TIMER0 EVENTS_COMPARE[0] SW-write-1 ignored (HW-only) -> 0",
        prep: &[],
        write: (TIMER0 + 0x140, 1),
        read_addr: TIMER0 + 0x140,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "TIMER0 EVENTS_COMPARE[0] clear",
        prep: &[(TIMER0 + 0x140, 1)],
        write: (TIMER0 + 0x140, 0),
        read_addr: TIMER0 + 0x140,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "TIMER0 EVENTS_COMPARE[5] absent -> 0",
        prep: &[],
        write: (TIMER0 + 0x154, 1),
        read_addr: TIMER0 + 0x154,
        mask: 1,
        expect: 0,
    },
    // ── RTC0 (PS §6.22) ───────────────────────────────────────────────────
    Case {
        label: "RTC0 PRESCALER=0xFFF",
        prep: &[],
        write: (RTC0 + 0x508, 0xFFF),
        read_addr: RTC0 + 0x508,
        mask: 0xFFF,
        expect: 0xFFF,
    },
    Case {
        label: "RTC0 CC[0]",
        prep: &[],
        write: (RTC0 + 0x540, 0x12_3456),
        read_addr: RTC0 + 0x540,
        mask: 0xFF_FFFF,
        expect: 0x12_3456,
    },
    Case {
        label: "RTC0 CC[1]",
        prep: &[],
        write: (RTC0 + 0x544, 0xAB_CDEF),
        read_addr: RTC0 + 0x544,
        mask: 0xFF_FFFF,
        expect: 0xAB_CDEF,
    },
    Case {
        label: "RTC0 CC[2]",
        prep: &[],
        write: (RTC0 + 0x548, 0x0F_0F0F),
        read_addr: RTC0 + 0x548,
        mask: 0xFF_FFFF,
        expect: 0x0F_0F0F,
    },
    Case {
        label: "RTC0 CC[3] absent (3-CC rtc0) -> 0",
        prep: &[],
        write: (RTC0 + 0x54C, 0x00_FFFF),
        read_addr: RTC0 + 0x54C,
        mask: 0xFF_FFFF,
        expect: 0,
    },
    // INTEN: TICK(0) OVRFLW(1) COMPARE0..3(16..19).
    Case {
        label: "RTC0 INTENSET TICK+OVRFLW+CMP0..3",
        prep: &[(RTC0 + 0x308, 0xFFFF_FFFF)],
        write: (RTC0 + 0x304, 0x000F_0003),
        read_addr: RTC0 + 0x304,
        mask: 0x000F_0003,
        expect: 0x0007_0003,
    },
    Case {
        label: "RTC0 INTENCLR",
        prep: &[(RTC0 + 0x304, 0x000F_0003)],
        write: (RTC0 + 0x308, 0x000F_0003),
        read_addr: RTC0 + 0x304,
        mask: 0x000F_0003,
        expect: 0,
    },
    // EVTEN: same bit layout via EVTENSET (0x344) / EVTENCLR (0x348).
    Case {
        label: "RTC0 EVTENSET TICK+CMP0..3",
        prep: &[(RTC0 + 0x348, 0xFFFF_FFFF)],
        write: (RTC0 + 0x344, 0x000F_0001),
        read_addr: RTC0 + 0x344,
        mask: 0x000F_0001,
        expect: 0x0007_0001,
    },
    Case {
        label: "RTC0 EVENTS_TICK SW-write-1 ignored -> 0",
        prep: &[],
        write: (RTC0 + 0x100, 1),
        read_addr: RTC0 + 0x100,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "RTC0 EVENTS_OVRFLW SW-write-1 ignored -> 0",
        prep: &[],
        write: (RTC0 + 0x104, 1),
        read_addr: RTC0 + 0x104,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "RTC0 EVENTS_COMPARE[0] SW-write-1 ignored -> 0",
        prep: &[],
        write: (RTC0 + 0x140, 1),
        read_addr: RTC0 + 0x140,
        mask: 1,
        expect: 0,
    },
    // COUNTER is read-only: a write must be ignored; reset-halt value is 0.
    Case {
        label: "RTC0 COUNTER read-only (=0 at reset)",
        prep: &[],
        write: (RTC0 + 0x504, 0x99_9999),
        read_addr: RTC0 + 0x504,
        mask: 0xFF_FFFF,
        expect: 0,
    },
];

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Match,
    BothDisagreeWithExpect { both: u32 },
    Diverge { sim: u32, hw: u32 },
    SimError(String),
}

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

fn write_both(sim: &mut SystemBus, oc: &mut OpenOcd, addr: u32, val: u32) {
    sim.write_u32(addr as u64, val)
        .unwrap_or_else(|e| panic!("sim write 0x{addr:08X}=0x{val:08X}: {e:?}"));
    oc.write_memory(addr, &[val])
        .unwrap_or_else(|e| panic!("hw write 0x{addr:08X}=0x{val:08X}: {e}"));
}

fn run_case(sim: &mut SystemBus, oc: &mut OpenOcd, case: &Case) -> Outcome {
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
#[ignore]
fn nrf52840_timer_rtc_conformance() {
    let _guard = HW_LOCK.lock().unwrap();
    let mut sim = build_sim_bus();
    let mut oc = OpenOcd::spawn_nrf52().expect("openocd spawn_nrf52 failed");
    oc.reset_halt().expect("reset halt failed");
    oc.halt().expect("halt failed");

    println!();
    println!(
        "nRF52840 TIMER0+RTC0 full-register conformance — {} cases",
        CASES.len()
    );
    println!("{:-<92}", "");

    let mut by_periph: std::collections::BTreeMap<&str, (u32, u32, u32, u32)> =
        std::collections::BTreeMap::new();

    for case in CASES {
        let periph = if case.label.starts_with("TIMER0") {
            "TIMER0"
        } else {
            "RTC0"
        };
        let b = by_periph.entry(periph).or_insert((0, 0, 0, 0));
        match run_case(&mut sim, &mut oc, case) {
            Outcome::Match => {
                b.0 += 1;
                println!("[OK ]  {}", case.label);
            }
            Outcome::Diverge { sim, hw } => {
                b.1 += 1;
                println!(
                    "[DIFF] {}  sim=0x{:08X} hw=0x{:08X} (mask=0x{:08X})",
                    case.label, sim, hw, case.mask
                );
            }
            Outcome::BothDisagreeWithExpect { both } => {
                b.2 += 1;
                println!(
                    "[BOTH] {}  both=0x{:08X} expected=0x{:08X}",
                    case.label, both, case.expect
                );
            }
            Outcome::SimError(m) => {
                b.3 += 1;
                println!("[SIM!] {}  {}", case.label, m);
            }
        }
    }

    println!("{:-<92}", "");
    let mut total_div = 0;
    for (p, (m, d, bo, se)) in &by_periph {
        total_div += *d;
        println!("{p}: match={m} diverge={d} both_disagree={bo} sim_err={se}");
    }
    oc.shutdown().ok();

    if std::env::var("NRF52_STRICT").is_ok() {
        assert_eq!(
            total_div, 0,
            "TIMER/RTC diff: {total_div} register(s) diverged"
        );
    }
}
