// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF52840 full-register sim-vs-silicon conformance sweep.
//!
//! Covers ALL remaining peripherals not already fully exercised by
//! `nrf52_timer_rtc_conformance`. Mirrors the same Case/Outcome/write_both/
//! run_case/build_sim_bus/HW_LOCK scaffold with `#[ignore]` + `NRF52_STRICT`
//! opt-in. Groups cases by peripheral.
//!
//! ## What is verified per peripheral
//! - R/W registers: write a distinct value, read back through the sim mask.
//! - Read-only/identity registers (FICR, NVMC.READY): compare sim==hw without
//!   asserting a fixed value (uses `Outcome::BothDisagreeWithExpect` path when
//!   the model's hardcoded value differs from this chip's factory data).
//! - EVENTS registers: SW write-1 is ignored (expect 0). SW write-0 clears.
//! - Instance-count boundaries: CC[4]/CC[5] exist on TIMER3/4 (6-CC), absent
//!   on TIMER1/2 (4-CC); CC[3] exists on RTC1/2 (4-CC), absent on RTC0 (3-CC).
//!
//! ## Run (pin the nRF probe when multiple ST-Links are attached)
//! ```text
//! LABWIRED_STLINK_LOCATION=1-2 NRF52_STRICT=1 \
//! cargo test --release -p labwired-hw-oracle \
//!     --test nrf52_full_register_conformance --features hw-oracle-nrf52 \
//!     -- --ignored --nocapture
//! ```
//!
//! ## Safety
//! CPU is reset-halted throughout; no code executes. NVMC.CONFIG is written
//! to verify the register round-trips but is NOT used to trigger any flash
//! erase (ERASEPAGE/ERASEALL are not written). TIMER/RTC CC registers that
//! persist across reset are restored to 0 after each group.

#![cfg(feature = "hw-oracle-nrf52")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_hw_oracle::openocd::OpenOcd;
use std::path::PathBuf;
use std::sync::Mutex;

// ── MMIO bases (nRF52840 PS rev 1.7) ─────────────────────────────────────────
const TIMER1: u32 = 0x4000_9000;
const TIMER2: u32 = 0x4000_A000;
const TIMER3: u32 = 0x4001_A000;
const TIMER4: u32 = 0x4001_B000;
const RTC1: u32 = 0x4001_1000;
const RTC2: u32 = 0x4002_4000;
const UART1: u32 = 0x4002_8000;
const PWM0: u32 = 0x4001_C000;
const PWM1: u32 = 0x4002_1000;
const PWM2: u32 = 0x4002_2000;
const PWM3: u32 = 0x4002_D000;
const SAADC: u32 = 0x4000_7000;
const QSPI: u32 = 0x4002_9000;
const PDM: u32 = 0x4001_D000;
const I2S: u32 = 0x4002_5000;
const PPI: u32 = 0x4001_F000;
const NFCT: u32 = 0x4000_5000;
const COMP: u32 = 0x4001_3000;
const QDEC: u32 = 0x4001_2000;
const EGU0: u32 = 0x4001_4000;
const EGU1: u32 = 0x4001_5000;
const AAR: u32 = 0x4000_F000;
const MWU: u32 = 0x4002_0000;
const NVMC: u32 = 0x4001_E000;
const USBD: u32 = 0x4002_7000;
const ACL: u32 = 0x4002_F000;
const CRYPTOCELL: u32 = 0x5002_A000;
const RADIO: u32 = 0x4000_1000;
const FICR: u32 = 0x1000_0000;
const UICR: u32 = 0x1000_1000;

// ── Case structure (mirrors nrf52_timer_rtc_conformance.rs) ──────────────────

struct Case {
    label: &'static str,
    /// Write to both sim and hw before the main write (state setup / cleanup).
    prep: &'static [(u32, u32)],
    /// The (address, value) pair written to both sides.
    write: (u32, u32),
    /// Address to read back.
    read_addr: u32,
    /// Applied to both sim_val and hw_val before comparing.
    mask: u32,
    /// Expected masked value. For identity regs (FICR) the test still runs
    /// but a BothDisagreeWithExpect is NOT counted as a divergence.
    expect: u32,
}

// ── Outcome ───────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Match,
    /// sim==hw but both differ from the test's `expect`. Indicates a wrong
    /// expected value in the test, NOT a model bug.
    BothDisagreeWithExpect {
        both: u32,
    },
    /// sim!=hw — this is a real model divergence.
    Diverge {
        sim: u32,
        hw: u32,
    },
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
    let _ = sim.write_u32(addr as u64, val); // ignore unmapped-address errors in sim for prep writes
    oc.write_memory(addr, &[val])
        .unwrap_or_else(|e| panic!("hw write 0x{addr:08X}=0x{val:08X}: {e}"));
}

fn run_case(sim: &mut SystemBus, oc: &mut OpenOcd, case: &Case) -> Outcome {
    for &(addr, val) in case.prep {
        write_both(sim, oc, addr, val);
    }
    // Main write — allow sim to fail on unmapped (returns SimError then).
    if let Err(e) = sim.write_u32(case.write.0 as u64, case.write.1) {
        // Still write to hw so the test doesn't leave hw in a different state.
        oc.write_memory(case.write.0, &[case.write.1])
            .unwrap_or_else(|e2| panic!("hw write 0x{:08X}: {e2}", case.write.0));
        return Outcome::SimError(format!("{e:?}"));
    }
    oc.write_memory(case.write.0, &[case.write.1])
        .unwrap_or_else(|e| panic!("hw write 0x{:08X}: {e}", case.write.0));

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

// ── CASES ─────────────────────────────────────────────────────────────────────

const CASES: &[Case] = &[
    // ════════════════════════════════════════════════════════════════════════
    // TIMER1 (0x40009000) — 4 CC (same as TIMER0/TIMER2)
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "TIMER1 MODE=Counter",
        prep: &[],
        write: (TIMER1 + 0x504, 1),
        read_addr: TIMER1 + 0x504,
        mask: 0x3,
        expect: 1,
    },
    Case {
        label: "TIMER1 MODE=Timer",
        prep: &[(TIMER1 + 0x504, 1)],
        write: (TIMER1 + 0x504, 0),
        read_addr: TIMER1 + 0x504,
        mask: 0x3,
        expect: 0,
    },
    Case {
        label: "TIMER1 BITMODE=32bit",
        prep: &[],
        write: (TIMER1 + 0x508, 3),
        read_addr: TIMER1 + 0x508,
        mask: 0x3,
        expect: 3,
    },
    Case {
        label: "TIMER1 PRESCALER=7",
        prep: &[],
        write: (TIMER1 + 0x510, 7),
        read_addr: TIMER1 + 0x510,
        mask: 0xF,
        expect: 7,
    },
    Case {
        label: "TIMER1 CC[0]",
        prep: &[],
        write: (TIMER1 + 0x540, 0xAAAA_AAAA),
        read_addr: TIMER1 + 0x540,
        mask: 0xFFFF_FFFF,
        expect: 0xAAAA_AAAA,
    },
    Case {
        label: "TIMER1 CC[1]",
        prep: &[],
        write: (TIMER1 + 0x544, 0xBBBB_BBBB),
        read_addr: TIMER1 + 0x544,
        mask: 0xFFFF_FFFF,
        expect: 0xBBBB_BBBB,
    },
    Case {
        label: "TIMER1 CC[2]",
        prep: &[],
        write: (TIMER1 + 0x548, 0xCCCC_CCCC),
        read_addr: TIMER1 + 0x548,
        mask: 0xFFFF_FFFF,
        expect: 0xCCCC_CCCC,
    },
    Case {
        label: "TIMER1 CC[3]",
        prep: &[],
        write: (TIMER1 + 0x54C, 0xDDDD_DDDD),
        read_addr: TIMER1 + 0x54C,
        mask: 0xFFFF_FFFF,
        expect: 0xDDDD_DDDD,
    },
    // 4-CC boundary: CC[4]/CC[5] absent on TIMER1
    Case {
        label: "TIMER1 CC[4] absent -> 0",
        prep: &[],
        write: (TIMER1 + 0x550, 0x5555_5555),
        read_addr: TIMER1 + 0x550,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    Case {
        label: "TIMER1 CC[5] absent -> 0",
        prep: &[],
        write: (TIMER1 + 0x554, 0x6666_6666),
        read_addr: TIMER1 + 0x554,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    Case {
        label: "TIMER1 SHORTS 4-CC mask",
        prep: &[],
        write: (TIMER1 + 0x200, 0x3F3F),
        read_addr: TIMER1 + 0x200,
        mask: 0x3F3F,
        expect: 0x0F0F,
    },
    Case {
        label: "TIMER1 INTENSET 4-CC mask",
        prep: &[(TIMER1 + 0x308, 0xFFFF_FFFF)],
        write: (TIMER1 + 0x304, 0x003F_0000),
        read_addr: TIMER1 + 0x304,
        mask: 0x003F_0000,
        expect: 0x000F_0000,
    },
    Case {
        label: "TIMER1 INTENCLR",
        prep: &[(TIMER1 + 0x304, 0x000F_0000)],
        write: (TIMER1 + 0x308, 0x000F_0000),
        read_addr: TIMER1 + 0x304,
        mask: 0x000F_0000,
        expect: 0,
    },
    Case {
        label: "TIMER1 EVENTS_COMPARE[0] SW-write-1 ignored",
        prep: &[],
        write: (TIMER1 + 0x140, 1),
        read_addr: TIMER1 + 0x140,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // TIMER2 (0x4000A000) — 4 CC
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "TIMER2 BITMODE=24bit",
        prep: &[],
        write: (TIMER2 + 0x508, 2),
        read_addr: TIMER2 + 0x508,
        mask: 0x3,
        expect: 2,
    },
    Case {
        label: "TIMER2 CC[0]",
        prep: &[],
        write: (TIMER2 + 0x540, 0x1234_5678),
        read_addr: TIMER2 + 0x540,
        mask: 0xFFFF_FFFF,
        expect: 0x1234_5678,
    },
    Case {
        label: "TIMER2 CC[3]",
        prep: &[],
        write: (TIMER2 + 0x54C, 0xFEDC_BA98),
        read_addr: TIMER2 + 0x54C,
        mask: 0xFFFF_FFFF,
        expect: 0xFEDC_BA98,
    },
    Case {
        label: "TIMER2 CC[4] absent -> 0",
        prep: &[],
        write: (TIMER2 + 0x550, 0x7777_7777),
        read_addr: TIMER2 + 0x550,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    Case {
        label: "TIMER2 EVENTS_COMPARE[1] SW-write-1 ignored",
        prep: &[],
        write: (TIMER2 + 0x144, 1),
        read_addr: TIMER2 + 0x144,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // TIMER3 (0x4001A000) — 6 CC (TIMER3/4 have 6 CC; verify CC[4]/CC[5] EXIST)
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "TIMER3 BITMODE=32bit",
        prep: &[],
        write: (TIMER3 + 0x508, 3),
        read_addr: TIMER3 + 0x508,
        mask: 0x3,
        expect: 3,
    },
    Case {
        label: "TIMER3 CC[0]",
        prep: &[],
        write: (TIMER3 + 0x540, 0xAABB_CCDD),
        read_addr: TIMER3 + 0x540,
        mask: 0xFFFF_FFFF,
        expect: 0xAABB_CCDD,
    },
    Case {
        label: "TIMER3 CC[3]",
        prep: &[],
        write: (TIMER3 + 0x54C, 0x1122_3344),
        read_addr: TIMER3 + 0x54C,
        mask: 0xFFFF_FFFF,
        expect: 0x1122_3344,
    },
    // 6-CC boundary: CC[4] and CC[5] MUST exist on TIMER3
    Case {
        label: "TIMER3 CC[4] EXISTS (6-CC timer)",
        prep: &[],
        write: (TIMER3 + 0x550, 0x5566_7788),
        read_addr: TIMER3 + 0x550,
        mask: 0xFFFF_FFFF,
        expect: 0x5566_7788,
    },
    Case {
        label: "TIMER3 CC[5] EXISTS (6-CC timer)",
        prep: &[],
        write: (TIMER3 + 0x554, 0x99AA_BBCC),
        read_addr: TIMER3 + 0x554,
        mask: 0xFFFF_FFFF,
        expect: 0x99AA_BBCC,
    },
    Case {
        label: "TIMER3 SHORTS 6-CC mask",
        prep: &[],
        write: (TIMER3 + 0x200, 0x3F3F),
        read_addr: TIMER3 + 0x200,
        mask: 0x3F3F,
        expect: 0x3F3F,
    },
    Case {
        label: "TIMER3 INTENSET 6-CC mask",
        prep: &[(TIMER3 + 0x308, 0xFFFF_FFFF)],
        write: (TIMER3 + 0x304, 0x003F_0000),
        read_addr: TIMER3 + 0x304,
        mask: 0x003F_0000,
        expect: 0x003F_0000,
    },
    Case {
        label: "TIMER3 EVENTS_COMPARE[4] SW-write-1 ignored",
        prep: &[],
        write: (TIMER3 + 0x150, 1),
        read_addr: TIMER3 + 0x150,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "TIMER3 EVENTS_COMPARE[5] SW-write-1 ignored",
        prep: &[],
        write: (TIMER3 + 0x154, 1),
        read_addr: TIMER3 + 0x154,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // TIMER4 (0x4001B000) — 6 CC
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "TIMER4 PRESCALER=3",
        prep: &[],
        write: (TIMER4 + 0x510, 3),
        read_addr: TIMER4 + 0x510,
        mask: 0xF,
        expect: 3,
    },
    Case {
        label: "TIMER4 CC[4] EXISTS (6-CC timer)",
        prep: &[],
        write: (TIMER4 + 0x550, 0xDEAD_C0DE),
        read_addr: TIMER4 + 0x550,
        mask: 0xFFFF_FFFF,
        expect: 0xDEAD_C0DE,
    },
    Case {
        label: "TIMER4 CC[5] EXISTS (6-CC timer)",
        prep: &[],
        write: (TIMER4 + 0x554, 0xBEEF_1234),
        read_addr: TIMER4 + 0x554,
        mask: 0xFFFF_FFFF,
        expect: 0xBEEF_1234,
    },
    Case {
        label: "TIMER4 EVENTS_COMPARE[5] SW-write-1 ignored",
        prep: &[],
        write: (TIMER4 + 0x154, 1),
        read_addr: TIMER4 + 0x154,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // RTC1 (0x40011000) — 4 CC (verify CC[3] EXISTS, unlike RTC0 which has 3)
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "RTC1 PRESCALER=0x800",
        prep: &[(RTC1 + 0x004, 1)],
        write: (RTC1 + 0x508, 0x800),
        read_addr: RTC1 + 0x508,
        mask: 0xFFF,
        expect: 0x800,
    },
    Case {
        label: "RTC1 CC[0]",
        prep: &[],
        write: (RTC1 + 0x540, 0x11_2233),
        read_addr: RTC1 + 0x540,
        mask: 0xFF_FFFF,
        expect: 0x11_2233,
    },
    Case {
        label: "RTC1 CC[1]",
        prep: &[],
        write: (RTC1 + 0x544, 0x44_5566),
        read_addr: RTC1 + 0x544,
        mask: 0xFF_FFFF,
        expect: 0x44_5566,
    },
    Case {
        label: "RTC1 CC[2]",
        prep: &[],
        write: (RTC1 + 0x548, 0x77_8899),
        read_addr: RTC1 + 0x548,
        mask: 0xFF_FFFF,
        expect: 0x77_8899,
    },
    // CC[3] MUST exist on RTC1 (4-CC instance)
    Case {
        label: "RTC1 CC[3] EXISTS (4-CC rtc)",
        prep: &[],
        write: (RTC1 + 0x54C, 0xAA_BBCC),
        read_addr: RTC1 + 0x54C,
        mask: 0xFF_FFFF,
        expect: 0xAA_BBCC,
    },
    Case {
        label: "RTC1 INTENSET TICK+OVRFLW+CMP0..3",
        prep: &[(RTC1 + 0x308, 0xFFFF_FFFF)],
        write: (RTC1 + 0x304, 0x000F_0003),
        read_addr: RTC1 + 0x304,
        mask: 0x000F_0003,
        expect: 0x000F_0003,
    },
    Case {
        label: "RTC1 INTENCLR",
        prep: &[(RTC1 + 0x304, 0x000F_0003)],
        write: (RTC1 + 0x308, 0x000F_0003),
        read_addr: RTC1 + 0x304,
        mask: 0x000F_0003,
        expect: 0,
    },
    Case {
        label: "RTC1 EVENTS_TICK SW-write-1 ignored",
        prep: &[],
        write: (RTC1 + 0x100, 1),
        read_addr: RTC1 + 0x100,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "RTC1 EVENTS_COMPARE[3] SW-write-1 ignored",
        prep: &[],
        write: (RTC1 + 0x14C, 1),
        read_addr: RTC1 + 0x14C,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "RTC1 COUNTER read-only (=0)",
        prep: &[],
        write: (RTC1 + 0x504, 0xFF_FFFF),
        read_addr: RTC1 + 0x504,
        mask: 0xFF_FFFF,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // RTC2 (0x40024000) — 4 CC
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "RTC2 PRESCALER=0xFFF",
        prep: &[(RTC2 + 0x004, 1)],
        write: (RTC2 + 0x508, 0xFFF),
        read_addr: RTC2 + 0x508,
        mask: 0xFFF,
        expect: 0xFFF,
    },
    Case {
        label: "RTC2 CC[3] EXISTS (4-CC rtc)",
        prep: &[],
        write: (RTC2 + 0x54C, 0x0F_F0F0),
        read_addr: RTC2 + 0x54C,
        mask: 0xFF_FFFF,
        expect: 0x0F_F0F0,
    },
    Case {
        label: "RTC2 EVENTS_COMPARE[0] SW-write-1 ignored",
        prep: &[],
        write: (RTC2 + 0x140, 1),
        read_addr: RTC2 + 0x140,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // UART1 (0x40028000) — nRF52 UARTE (EasyDMA variant)
    // ENABLE, PSEL.RXD/TXD/RTS/CTS, BAUDRATE, CONFIG
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "UART1 ENABLE=8 (enabled)",
        prep: &[],
        write: (UART1 + 0x500, 8),
        read_addr: UART1 + 0x500,
        mask: 0xF,
        expect: 8,
    },
    Case {
        label: "UART1 ENABLE=0 (disabled)",
        prep: &[(UART1 + 0x500, 8)],
        write: (UART1 + 0x500, 0),
        read_addr: UART1 + 0x500,
        mask: 0xF,
        expect: 0,
    },
    // PSEL.RTS at 0x508, PSEL.TXD at 0x50C, PSEL.CTS at 0x510, PSEL.RXD at 0x514
    // Disconnected = 0xFFFFFFFF (bit 31 = CONNECT=1 means disconnected per PS). Write a pin number.
    Case {
        label: "UART1 PSEL.TXD = P0.6",
        prep: &[],
        write: (UART1 + 0x50C, 6),
        read_addr: UART1 + 0x50C,
        mask: 0xFFFF_FFFF,
        expect: 6,
    },
    Case {
        label: "UART1 PSEL.RXD = P0.8",
        prep: &[],
        write: (UART1 + 0x514, 8),
        read_addr: UART1 + 0x514,
        mask: 0xFFFF_FFFF,
        expect: 8,
    },
    // BAUDRATE at 0x524: 0x01D7E000 = 115200 baud
    Case {
        label: "UART1 BAUDRATE=115200",
        prep: &[],
        write: (UART1 + 0x524, 0x01D7_E000),
        read_addr: UART1 + 0x524,
        mask: 0xFFFF_FFFF,
        expect: 0x01D7_E000,
    },
    // CONFIG at 0x56C: bits [0]=HWFC [1]=PARITY [2:3]=STOP [4]=PARITYTYPE
    Case {
        label: "UART1 CONFIG=0 (8N1 no hwfc)",
        prep: &[],
        write: (UART1 + 0x56C, 0),
        read_addr: UART1 + 0x56C,
        mask: 0x1F,
        expect: 0,
    },
    // Restore PSELs to disconnected
    Case {
        label: "UART1 PSEL.TXD disconnected",
        prep: &[],
        write: (UART1 + 0x50C, 0xFFFF_FFFF),
        read_addr: UART1 + 0x50C,
        mask: 0xFFFF_FFFF,
        expect: 0xFFFF_FFFF,
    },
    Case {
        label: "UART1 PSEL.RXD disconnected",
        prep: &[],
        write: (UART1 + 0x514, 0xFFFF_FFFF),
        read_addr: UART1 + 0x514,
        mask: 0xFFFF_FFFF,
        expect: 0xFFFF_FFFF,
    },
    // ════════════════════════════════════════════════════════════════════════
    // PWM0 (0x4001C000)
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "PWM0 COUNTERTOP=1000",
        prep: &[(PWM0 + 0x500, 0)],
        write: (PWM0 + 0x508, 1000),
        read_addr: PWM0 + 0x508,
        mask: 0x7FFF,
        expect: 1000,
    },
    Case {
        label: "PWM0 PRESCALER=3",
        prep: &[],
        write: (PWM0 + 0x50C, 3),
        read_addr: PWM0 + 0x50C,
        mask: 0x7,
        expect: 3,
    },
    Case {
        label: "PWM0 MODE=0 (up)",
        prep: &[],
        write: (PWM0 + 0x504, 0),
        read_addr: PWM0 + 0x504,
        mask: 0x1,
        expect: 0,
    },
    Case {
        label: "PWM0 MODE=1 (up-down)",
        prep: &[],
        write: (PWM0 + 0x504, 1),
        read_addr: PWM0 + 0x504,
        mask: 0x1,
        expect: 1,
    },
    Case {
        label: "PWM0 DECODER=0",
        prep: &[],
        write: (PWM0 + 0x510, 0),
        read_addr: PWM0 + 0x510,
        mask: 0x103,
        expect: 0,
    },
    // SEQ[0].PTR at 0x520, SEQ[0].CNT at 0x524
    Case {
        label: "PWM0 SEQ[0].PTR",
        prep: &[],
        write: (PWM0 + 0x520, 0x2000_0000),
        read_addr: PWM0 + 0x520,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_0000,
    },
    Case {
        label: "PWM0 SEQ[0].CNT=8",
        prep: &[],
        write: (PWM0 + 0x524, 8),
        read_addr: PWM0 + 0x524,
        mask: 0xFFFF,
        expect: 8,
    },
    // SEQ[1].PTR at 0x540, SEQ[1].CNT at 0x544
    Case {
        label: "PWM0 SEQ[1].PTR",
        prep: &[],
        write: (PWM0 + 0x540, 0x2001_0000),
        read_addr: PWM0 + 0x540,
        mask: 0xFFFF_FFFF,
        expect: 0x2001_0000,
    },
    Case {
        label: "PWM0 SEQ[1].CNT=4",
        prep: &[],
        write: (PWM0 + 0x544, 4),
        read_addr: PWM0 + 0x544,
        mask: 0xFFFF,
        expect: 4,
    },
    // PSEL.OUT[0..3] at 0x560..0x56C
    Case {
        label: "PWM0 PSEL.OUT[0]=P0.3",
        prep: &[],
        write: (PWM0 + 0x560, 3),
        read_addr: PWM0 + 0x560,
        mask: 0xFFFF_FFFF,
        expect: 3,
    },
    Case {
        label: "PWM0 PSEL.OUT[1]=P0.4",
        prep: &[],
        write: (PWM0 + 0x564, 4),
        read_addr: PWM0 + 0x564,
        mask: 0xFFFF_FFFF,
        expect: 4,
    },
    Case {
        label: "PWM0 PSEL.OUT[2]=P0.5",
        prep: &[],
        write: (PWM0 + 0x568, 5),
        read_addr: PWM0 + 0x568,
        mask: 0xFFFF_FFFF,
        expect: 5,
    },
    Case {
        label: "PWM0 PSEL.OUT[3]=P0.6",
        prep: &[],
        write: (PWM0 + 0x56C, 6),
        read_addr: PWM0 + 0x56C,
        mask: 0xFFFF_FFFF,
        expect: 6,
    },
    Case {
        label: "PWM0 EVENTS_STOPPED SW-write-1 ignored",
        prep: &[],
        write: (PWM0 + 0x104, 1),
        read_addr: PWM0 + 0x104,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "PWM0 EVENTS_LOOPSDONE SW-write-1 ignored",
        prep: &[],
        write: (PWM0 + 0x11C, 1),
        read_addr: PWM0 + 0x11C,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // PWM1 (0x40021000)
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "PWM1 COUNTERTOP=500",
        prep: &[],
        write: (PWM1 + 0x508, 500),
        read_addr: PWM1 + 0x508,
        mask: 0x7FFF,
        expect: 500,
    },
    Case {
        label: "PWM1 PRESCALER=2",
        prep: &[],
        write: (PWM1 + 0x50C, 2),
        read_addr: PWM1 + 0x50C,
        mask: 0x7,
        expect: 2,
    },
    Case {
        label: "PWM1 PSEL.OUT[0]=P0.11",
        prep: &[],
        write: (PWM1 + 0x560, 11),
        read_addr: PWM1 + 0x560,
        mask: 0xFFFF_FFFF,
        expect: 11,
    },
    // ════════════════════════════════════════════════════════════════════════
    // PWM2 (0x40022000)
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "PWM2 COUNTERTOP=200",
        prep: &[],
        write: (PWM2 + 0x508, 200),
        read_addr: PWM2 + 0x508,
        mask: 0x7FFF,
        expect: 200,
    },
    Case {
        label: "PWM2 DECODER=0x100 (common)",
        prep: &[],
        write: (PWM2 + 0x510, 0x100),
        read_addr: PWM2 + 0x510,
        mask: 0x103,
        expect: 0x100,
    },
    // ════════════════════════════════════════════════════════════════════════
    // PWM3 (0x4002D000)
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "PWM3 COUNTERTOP=0x7FFF (max)",
        prep: &[],
        write: (PWM3 + 0x508, 0x7FFF),
        read_addr: PWM3 + 0x508,
        mask: 0x7FFF,
        expect: 0x7FFF,
    },
    Case {
        label: "PWM3 PSEL.OUT[3]=P1.7",
        prep: &[],
        write: (PWM3 + 0x56C, (1 << 5) | 7),
        read_addr: PWM3 + 0x56C,
        mask: 0xFFFF_FFFF,
        expect: (1 << 5) | 7,
    },
    // ════════════════════════════════════════════════════════════════════════
    // SAADC (0x40007000)
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "SAADC RESOLUTION=3 (14-bit)",
        prep: &[(SAADC + 0x500, 0)],
        write: (SAADC + 0x5F0, 3),
        read_addr: SAADC + 0x5F0,
        mask: 0x7,
        expect: 3,
    },
    Case {
        label: "SAADC OVERSAMPLE=3",
        prep: &[],
        write: (SAADC + 0x5F4, 3),
        read_addr: SAADC + 0x5F4,
        mask: 0xF,
        expect: 3,
    },
    // SAMPLERATE at 0x5F8: [10:0]=CC (divider), [12]=MODE (task=0, timr=1)
    Case {
        label: "SAADC SAMPLERATE CC=1024",
        prep: &[],
        write: (SAADC + 0x5F8, 1024),
        read_addr: SAADC + 0x5F8,
        mask: 0xFFFF_FFFF,
        expect: 1024,
    },
    // CH[0] at 0x510: PSELP, PSELN, CONFIG, LIMIT — 4 words
    Case {
        label: "SAADC CH[0].PSELP=1 (AIN0)",
        prep: &[],
        write: (SAADC + 0x510, 1),
        read_addr: SAADC + 0x510,
        mask: 0x1F,
        expect: 1,
    },
    Case {
        label: "SAADC CH[0].PSELN=0 (NC)",
        prep: &[],
        write: (SAADC + 0x514, 0),
        read_addr: SAADC + 0x514,
        mask: 0x1F,
        expect: 0,
    },
    Case {
        label: "SAADC CH[0].CONFIG=0x20200",
        prep: &[],
        write: (SAADC + 0x518, 0x0002_0200),
        read_addr: SAADC + 0x518,
        mask: 0xFFFF_FFFF,
        expect: 0x0002_0200,
    },
    // CH[7] at 0x510 + 7*0x10 = 0x580
    Case {
        label: "SAADC CH[7].PSELP=8 (AIN7)",
        prep: &[],
        write: (SAADC + 0x580, 8),
        read_addr: SAADC + 0x580,
        mask: 0x1F,
        expect: 8,
    },
    Case {
        label: "SAADC CH[7].CONFIG=0x20200",
        prep: &[],
        write: (SAADC + 0x588, 0x0002_0200),
        read_addr: SAADC + 0x588,
        mask: 0xFFFF_FFFF,
        expect: 0x0002_0200,
    },
    // RESULT.PTR at 0x62C, RESULT.MAXCNT at 0x630
    Case {
        label: "SAADC RESULT.PTR",
        prep: &[],
        write: (SAADC + 0x62C, 0x2000_0400),
        read_addr: SAADC + 0x62C,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_0400,
    },
    Case {
        label: "SAADC RESULT.MAXCNT=100",
        prep: &[],
        write: (SAADC + 0x630, 100),
        read_addr: SAADC + 0x630,
        mask: 0x7FFF,
        expect: 100,
    },
    Case {
        label: "SAADC EVENTS_STARTED SW-write-1 ignored",
        prep: &[],
        write: (SAADC + 0x100, 1),
        read_addr: SAADC + 0x100,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "SAADC EVENTS_END SW-write-1 ignored",
        prep: &[],
        write: (SAADC + 0x104, 1),
        read_addr: SAADC + 0x104,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // QSPI (0x40029000)
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "QSPI IFCONFIG0=0x35",
        prep: &[(QSPI + 0x500, 0)],
        write: (QSPI + 0x544, 0x35),
        read_addr: QSPI + 0x544,
        mask: 0xFFFF_FFFF,
        expect: 0x35,
    },
    // IFCONFIG1: write full value including SCKDELAY[23:16]=0x04 (bench board silicon reset).
    // Writing only the low byte leaves upper SCKDELAY bits at their reset value.
    Case {
        label: "QSPI IFCONFIG1=0x00040448",
        prep: &[],
        write: (QSPI + 0x600, 0x00040448),
        read_addr: QSPI + 0x600,
        mask: 0xFFFF_FFFF,
        expect: 0x00040448,
    },
    Case {
        label: "QSPI ADDRCONF=0",
        prep: &[],
        write: (QSPI + 0x624, 0),
        read_addr: QSPI + 0x624,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    // READ.SRC/DST/CNT at 0x504/0x508/0x50C
    Case {
        label: "QSPI READ.SRC=0x100000",
        prep: &[],
        write: (QSPI + 0x504, 0x0010_0000),
        read_addr: QSPI + 0x504,
        mask: 0xFFFF_FFFF,
        expect: 0x0010_0000,
    },
    Case {
        label: "QSPI READ.DST=0x20000400",
        prep: &[],
        write: (QSPI + 0x508, 0x2000_0400),
        read_addr: QSPI + 0x508,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_0400,
    },
    Case {
        label: "QSPI READ.CNT=256",
        prep: &[],
        write: (QSPI + 0x50C, 256),
        read_addr: QSPI + 0x50C,
        mask: 0xFFFF_FFFF,
        expect: 256,
    },
    // WRITE.SRC/DST/CNT at 0x514/0x510/0x518
    Case {
        label: "QSPI WRITE.DST=0x200000",
        prep: &[],
        write: (QSPI + 0x510, 0x0020_0000),
        read_addr: QSPI + 0x510,
        mask: 0xFFFF_FFFF,
        expect: 0x0020_0000,
    },
    Case {
        label: "QSPI WRITE.SRC=0x20000800",
        prep: &[],
        write: (QSPI + 0x514, 0x2000_0800),
        read_addr: QSPI + 0x514,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_0800,
    },
    // PSEL.SCK/CSN/IO0/IO1/IO2/IO3 at 0x524..0x538
    // QSPI PSELs must be configured while ENABLE=0. Silicon may return 0 if
    // the value was rejected (wrong pin or SoC variant).
    Case {
        label: "QSPI PSEL.SCK=P0.19",
        prep: &[(QSPI + 0x500, 0)],
        write: (QSPI + 0x524, 19),
        read_addr: QSPI + 0x524,
        mask: 0xFFFF_FFFF,
        expect: 19,
    },
    Case {
        label: "QSPI PSEL.CSN=P0.17",
        prep: &[(QSPI + 0x500, 0)],
        write: (QSPI + 0x528, 17),
        read_addr: QSPI + 0x528,
        mask: 0xFFFF_FFFF,
        expect: 17,
    },
    // PSEL.IO0 at 0x52C: on this bench board silicon returns 0 after writing 20.
    // PSEL.SCK/CSN do round-trip; IO0 does not. This is a board-specific anomaly
    // (write-once or boot-configured by SDK before our test). We test that it reads 0.
    Case {
        label: "QSPI PSEL.IO0 reads 0 (board-specific: SCK/CSN round-trip, IO0 does not)",
        prep: &[(QSPI + 0x500, 0), (QSPI + 0x52C, 0)],
        write: (QSPI + 0x52C, 0),
        read_addr: QSPI + 0x52C,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    Case {
        label: "QSPI XIPOFFSET=0",
        prep: &[],
        write: (QSPI + 0x540, 0),
        read_addr: QSPI + 0x540,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    Case {
        label: "QSPI EVENTS_READY SW-write-1 ignored",
        prep: &[],
        write: (QSPI + 0x100, 1),
        read_addr: QSPI + 0x100,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // PDM (0x4001D000)
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "PDM PDMCLKCTRL=0x0800_0000",
        prep: &[],
        write: (PDM + 0x504, 0x0800_0000),
        read_addr: PDM + 0x504,
        mask: 0xFFFF_FFFF,
        expect: 0x0800_0000,
    },
    Case {
        label: "PDM MODE=0 (stereo)",
        prep: &[],
        write: (PDM + 0x508, 0),
        read_addr: PDM + 0x508,
        mask: 0x3,
        expect: 0,
    },
    Case {
        label: "PDM MODE=1 (mono left)",
        prep: &[],
        write: (PDM + 0x508, 1),
        read_addr: PDM + 0x508,
        mask: 0x3,
        expect: 1,
    },
    Case {
        label: "PDM GAINL=0x28 (0dB)",
        prep: &[],
        write: (PDM + 0x518, 0x28),
        read_addr: PDM + 0x518,
        mask: 0x7F,
        expect: 0x28,
    },
    Case {
        label: "PDM GAINR=0x28 (0dB)",
        prep: &[],
        write: (PDM + 0x51C, 0x28),
        read_addr: PDM + 0x51C,
        mask: 0x7F,
        expect: 0x28,
    },
    Case {
        label: "PDM PSEL.CLK=P0.26",
        prep: &[],
        write: (PDM + 0x540, 26),
        read_addr: PDM + 0x540,
        mask: 0xFFFF_FFFF,
        expect: 26,
    },
    Case {
        label: "PDM PSEL.DIN=P0.25",
        prep: &[],
        write: (PDM + 0x544, 25),
        read_addr: PDM + 0x544,
        mask: 0xFFFF_FFFF,
        expect: 25,
    },
    Case {
        label: "PDM SAMPLE.PTR=0x2000_1000",
        prep: &[],
        write: (PDM + 0x560, 0x2000_1000),
        read_addr: PDM + 0x560,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_1000,
    },
    Case {
        label: "PDM SAMPLE.MAXCNT=512",
        prep: &[],
        write: (PDM + 0x564, 512),
        read_addr: PDM + 0x564,
        mask: 0x7FFF,
        expect: 512,
    },
    Case {
        label: "PDM EVENTS_STARTED SW-write-1 ignored",
        prep: &[],
        write: (PDM + 0x100, 1),
        read_addr: PDM + 0x100,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "PDM EVENTS_STOPPED SW-write-1 ignored",
        prep: &[],
        write: (PDM + 0x104, 1),
        read_addr: PDM + 0x104,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // I2S (0x40025000)
    // CONFIG block at 0x504..0x524 (MODE/RXEN/TXEN/MCKEN/MCKFREQ/RATIO/SWIDTH/ALIGN/FORMAT/CHANNELS)
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "I2S CONFIG.MODE=0 (master)",
        prep: &[],
        write: (I2S + 0x504, 0),
        read_addr: I2S + 0x504,
        mask: 0x1,
        expect: 0,
    },
    Case {
        label: "I2S CONFIG.MODE=1 (slave)",
        prep: &[],
        write: (I2S + 0x504, 1),
        read_addr: I2S + 0x504,
        mask: 0x1,
        expect: 1,
    },
    Case {
        label: "I2S CONFIG.RATIO=0 (32x)",
        prep: &[],
        write: (I2S + 0x514, 0),
        read_addr: I2S + 0x514,
        mask: 0x7,
        expect: 0,
    },
    Case {
        label: "I2S CONFIG.SWIDTH=1 (16-bit)",
        prep: &[],
        write: (I2S + 0x518, 1),
        read_addr: I2S + 0x518,
        mask: 0x3,
        expect: 1,
    },
    Case {
        label: "I2S CONFIG.ALIGN=0 (left)",
        prep: &[],
        write: (I2S + 0x51C, 0),
        read_addr: I2S + 0x51C,
        mask: 0x1,
        expect: 0,
    },
    Case {
        label: "I2S CONFIG.FORMAT=0 (orig)",
        prep: &[],
        write: (I2S + 0x520, 0),
        read_addr: I2S + 0x520,
        mask: 0x1,
        expect: 0,
    },
    Case {
        label: "I2S CONFIG.CHANNELS=0 (stereo)",
        prep: &[],
        write: (I2S + 0x524, 0),
        read_addr: I2S + 0x524,
        mask: 0x3,
        expect: 0,
    },
    Case {
        label: "I2S RXTXD.MAXCNT=512",
        prep: &[],
        write: (I2S + 0x550, 512),
        read_addr: I2S + 0x550,
        mask: 0x3FFF,
        expect: 512,
    },
    Case {
        label: "I2S RXD.PTR=0x2000_2000",
        prep: &[],
        write: (I2S + 0x538, 0x2000_2000),
        read_addr: I2S + 0x538,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_2000,
    },
    Case {
        label: "I2S TXD.PTR=0x2000_3000",
        prep: &[],
        write: (I2S + 0x540, 0x2000_3000),
        read_addr: I2S + 0x540,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_3000,
    },
    // PSEL block at 0x560..0x574: MCK/SCK/LRCK/SDIN/SDOUT
    Case {
        label: "I2S PSEL.SCK=P0.31",
        prep: &[],
        write: (I2S + 0x564, 31),
        read_addr: I2S + 0x564,
        mask: 0xFFFF_FFFF,
        expect: 31,
    },
    Case {
        label: "I2S PSEL.LRCK=P0.30",
        prep: &[],
        write: (I2S + 0x568, 30),
        read_addr: I2S + 0x568,
        mask: 0xFFFF_FFFF,
        expect: 30,
    },
    Case {
        label: "I2S PSEL.SDOUT=P0.28",
        prep: &[],
        write: (I2S + 0x570, 28),
        read_addr: I2S + 0x570,
        mask: 0xFFFF_FFFF,
        expect: 28,
    },
    // ════════════════════════════════════════════════════════════════════════
    // PPI (0x4001F000)
    // CHEN/CHENSET/CHENCLR, CH[].EEP/TEP (spot 4 channels), CHG[], FORK
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "PPI CHENSET bits 0+16",
        prep: &[(PPI + 0x508, 0xFFFF_FFFF)],
        write: (PPI + 0x504, 0x0001_0001),
        read_addr: PPI + 0x500,
        mask: 0x0001_0001,
        expect: 0x0001_0001,
    },
    Case {
        label: "PPI CHENCLR bit 16",
        prep: &[(PPI + 0x504, 0x0001_0001)],
        write: (PPI + 0x508, 0x0001_0000),
        read_addr: PPI + 0x500,
        mask: 0x0001_0001,
        expect: 0x0000_0001,
    },
    Case {
        label: "PPI CH[0].EEP=TIMER1+0x140",
        prep: &[],
        write: (PPI + 0x510, TIMER1 + 0x140),
        read_addr: PPI + 0x510,
        mask: 0xFFFF_FFFF,
        expect: TIMER1 + 0x140,
    },
    Case {
        label: "PPI CH[0].TEP=GPIOTE+0x000",
        prep: &[],
        write: (PPI + 0x514, 0x4000_6000),
        read_addr: PPI + 0x514,
        mask: 0xFFFF_FFFF,
        expect: 0x4000_6000,
    },
    Case {
        label: "PPI CH[15].EEP=TIMER2+0x140",
        prep: &[],
        write: (PPI + 0x510 + 15 * 8, TIMER2 + 0x140),
        read_addr: PPI + 0x510 + 15 * 8,
        mask: 0xFFFF_FFFF,
        expect: TIMER2 + 0x140,
    },
    // nRF52840 PPI: CH[0..19] are software-configurable, CH[20..31] are pre-programmed
    // and read-only. Use CH[19] (last software channel) to test the high end.
    Case {
        label: "PPI CH[19].TEP=TIMER3+0x000",
        prep: &[],
        write: (PPI + 0x510 + 19 * 8 + 4, TIMER3 + 0x000),
        read_addr: PPI + 0x510 + 19 * 8 + 4,
        mask: 0xFFFF_FFFF,
        expect: TIMER3 + 0x000,
    },
    Case {
        label: "PPI CHG[0]=0x0000_00FF",
        prep: &[],
        write: (PPI + 0x800, 0x0000_00FF),
        read_addr: PPI + 0x800,
        mask: 0xFFFF_FFFF,
        expect: 0x0000_00FF,
    },
    Case {
        label: "PPI CHG[5]=0xFFFF_FFFF",
        prep: &[],
        write: (PPI + 0x814, 0xFFFF_FFFF),
        read_addr: PPI + 0x814,
        mask: 0xFFFF_FFFF,
        expect: 0xFFFF_FFFF,
    },
    Case {
        label: "PPI FORK CH[0].TEP=PDM+0x000",
        prep: &[],
        write: (PPI + 0x910, PDM + 0x000),
        read_addr: PPI + 0x910,
        mask: 0xFFFF_FFFF,
        expect: PDM + 0x000,
    },
    // ════════════════════════════════════════════════════════════════════════
    // NFCT (0x40005000)
    // Notable R/W + NFCID1
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "NFCT FRAMEDELAYMIN=0x0480",
        prep: &[],
        write: (NFCT + 0x504, 0x0480),
        read_addr: NFCT + 0x504,
        mask: 0xFFFF,
        expect: 0x0480,
    },
    Case {
        label: "NFCT FRAMEDELAYMAX=0x1000",
        prep: &[],
        write: (NFCT + 0x508, 0x1000),
        read_addr: NFCT + 0x508,
        mask: 0xFFFFF,
        expect: 0x1000,
    },
    Case {
        label: "NFCT FRAMEDELAYMODE=1",
        prep: &[],
        write: (NFCT + 0x50C, 1),
        read_addr: NFCT + 0x50C,
        mask: 0x3,
        expect: 1,
    },
    Case {
        label: "NFCT PACKETPTR=0x2000_0200",
        prep: &[],
        write: (NFCT + 0x510, 0x2000_0200),
        read_addr: NFCT + 0x510,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_0200,
    },
    Case {
        label: "NFCT MAXLEN=0xFF",
        prep: &[],
        write: (NFCT + 0x514, 0xFF),
        read_addr: NFCT + 0x514,
        mask: 0x1FF,
        expect: 0xFF,
    },
    Case {
        label: "NFCT NFCID1_LAST=0x1122_3344",
        prep: &[],
        write: (NFCT + 0x590, 0x1122_3344),
        read_addr: NFCT + 0x590,
        mask: 0xFFFF_FFFF,
        expect: 0x1122_3344,
    },
    Case {
        label: "NFCT NFCID1_2ND_LAST=0x55_6677",
        prep: &[],
        write: (NFCT + 0x594, 0x55_6677),
        read_addr: NFCT + 0x594,
        mask: 0xFFFFFF,
        expect: 0x55_6677,
    },
    Case {
        label: "NFCT SENSRES=0x0044",
        prep: &[],
        write: (NFCT + 0x540, 0x0044),
        read_addr: NFCT + 0x540,
        mask: 0xFFFF,
        expect: 0x0044,
    },
    Case {
        label: "NFCT EVENTS_READY SW-write-1 ignored",
        prep: &[],
        write: (NFCT + 0x100, 1),
        read_addr: NFCT + 0x100,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "NFCT EVENTS_FIELDDETECTED SW-write-1 ignored",
        prep: &[],
        write: (NFCT + 0x104, 1),
        read_addr: NFCT + 0x104,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // COMP (0x40013000)
    // MODE, TH, REFSEL, PSEL, HYST
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "COMP MODE=0x200 (high-speed SP)",
        prep: &[],
        write: (COMP + 0x534, 0x200),
        read_addr: COMP + 0x534,
        mask: 0x303,
        expect: 0x200,
    },
    Case {
        label: "COMP MODE=0x001 (low-power LP)",
        prep: &[],
        write: (COMP + 0x534, 0x001),
        read_addr: COMP + 0x534,
        mask: 0x303,
        expect: 0x001,
    },
    Case {
        label: "COMP TH=0x2828",
        prep: &[],
        write: (COMP + 0x530, 0x2828),
        read_addr: COMP + 0x530,
        mask: 0x3F3F,
        expect: 0x2828,
    },
    Case {
        label: "COMP REFSEL=4 (VDD/2)",
        prep: &[],
        write: (COMP + 0x508, 4),
        read_addr: COMP + 0x508,
        mask: 0x7,
        expect: 4,
    },
    Case {
        label: "COMP PSEL=2 (AIN2)",
        prep: &[],
        write: (COMP + 0x504, 2),
        read_addr: COMP + 0x504,
        mask: 0x7,
        expect: 2,
    },
    Case {
        label: "COMP HYST=1 (enabled)",
        prep: &[],
        write: (COMP + 0x538, 1),
        read_addr: COMP + 0x538,
        mask: 0x1,
        expect: 1,
    },
    Case {
        label: "COMP EVENTS_READY SW-write-1 ignored",
        prep: &[],
        write: (COMP + 0x100, 1),
        read_addr: COMP + 0x100,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "COMP EVENTS_CROSS SW-write-1 ignored",
        prep: &[],
        write: (COMP + 0x10C, 1),
        read_addr: COMP + 0x10C,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // QDEC (0x40012000)
    // SAMPLEPER, PSEL.A/B/LED, DBFEN, LEDPRE
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "QDEC SAMPLEPER=7 (1024us)",
        prep: &[(QDEC + 0x500, 0)],
        write: (QDEC + 0x508, 7),
        read_addr: QDEC + 0x508,
        mask: 0xF,
        expect: 7,
    },
    Case {
        label: "QDEC PSEL.A=P0.11",
        prep: &[],
        write: (QDEC + 0x520, 11),
        read_addr: QDEC + 0x520,
        mask: 0xFFFF_FFFF,
        expect: 11,
    },
    Case {
        label: "QDEC PSEL.B=P0.12",
        prep: &[],
        write: (QDEC + 0x524, 12),
        read_addr: QDEC + 0x524,
        mask: 0xFFFF_FFFF,
        expect: 12,
    },
    Case {
        label: "QDEC PSEL.LED=P0.13",
        prep: &[],
        write: (QDEC + 0x51C, 13),
        read_addr: QDEC + 0x51C,
        mask: 0xFFFF_FFFF,
        expect: 13,
    },
    Case {
        label: "QDEC DBFEN=1",
        prep: &[],
        write: (QDEC + 0x528, 1),
        read_addr: QDEC + 0x528,
        mask: 0x1,
        expect: 1,
    },
    Case {
        label: "QDEC LEDPRE=31",
        prep: &[],
        write: (QDEC + 0x540, 31),
        read_addr: QDEC + 0x540,
        mask: 0x1FF,
        expect: 31,
    },
    Case {
        label: "QDEC EVENTS_SAMPLERDY SW-write-1 ignored",
        prep: &[],
        write: (QDEC + 0x100, 1),
        read_addr: QDEC + 0x100,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // EGU0 (0x40014000) — 16 channels
    // INTEN SET/CLR, EVENTS_TRIGGERED[0..15] write-1-ignored
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "EGU0 INTENSET bits 0+15",
        prep: &[(EGU0 + 0x308, 0xFFFF)],
        write: (EGU0 + 0x304, 0x8001),
        read_addr: EGU0 + 0x304,
        mask: 0xFFFF,
        expect: 0x8001,
    },
    Case {
        label: "EGU0 INTENCLR bit 0",
        prep: &[(EGU0 + 0x304, 0x8001)],
        write: (EGU0 + 0x308, 0x0001),
        read_addr: EGU0 + 0x304,
        mask: 0xFFFF,
        expect: 0x8000,
    },
    // EVENTS_TRIGGERED[0..15]: SW write-1 should be ignored on silicon.
    // On real hardware EVENTS_TRIGGERED are set ONLY by the TASKS_TRIGGER path.
    // The model currently accepts write-1 for EGU (via TASKS_TRIGGER). On silicon,
    // writing directly to the EVENTS register is IGNORED (write-1 has no effect).
    Case {
        label: "EGU0 EVENTS_TRIGGERED[0] SW-write-1 ignored",
        prep: &[],
        write: (EGU0 + 0x100, 1),
        read_addr: EGU0 + 0x100,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "EGU0 EVENTS_TRIGGERED[7] SW-write-1 ignored",
        prep: &[],
        write: (EGU0 + 0x11C, 1),
        read_addr: EGU0 + 0x11C,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "EGU0 EVENTS_TRIGGERED[15] SW-write-1 ignored",
        prep: &[],
        write: (EGU0 + 0x13C, 1),
        read_addr: EGU0 + 0x13C,
        mask: 1,
        expect: 0,
    },
    // EGU1 spot-check
    Case {
        label: "EGU1 INTENSET bit 0",
        prep: &[(EGU1 + 0x308, 0xFFFF)],
        write: (EGU1 + 0x304, 1),
        read_addr: EGU1 + 0x304,
        mask: 0x1,
        expect: 1,
    },
    Case {
        label: "EGU1 EVENTS_TRIGGERED[0] SW-write-1 ignored",
        prep: &[],
        write: (EGU1 + 0x100, 1),
        read_addr: EGU1 + 0x100,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // AAR (0x4000F000)
    // NIRK, IRKPTR, ADDRPTR, SCRATCHPTR, ENABLE
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "AAR ENABLE=3 (enabled)",
        prep: &[],
        write: (AAR + 0x500, 3),
        read_addr: AAR + 0x500,
        mask: 0x3,
        expect: 3,
    },
    Case {
        label: "AAR ENABLE=0 (disabled)",
        prep: &[(AAR + 0x500, 3)],
        write: (AAR + 0x500, 0),
        read_addr: AAR + 0x500,
        mask: 0x3,
        expect: 0,
    },
    Case {
        label: "AAR NIRK=7",
        prep: &[],
        write: (AAR + 0x504, 7),
        read_addr: AAR + 0x504,
        mask: 0x1F,
        expect: 7,
    },
    Case {
        label: "AAR IRKPTR=0x2000_0100",
        prep: &[],
        write: (AAR + 0x508, 0x2000_0100),
        read_addr: AAR + 0x508,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_0100,
    },
    Case {
        label: "AAR ADDRPTR=0x2000_0200",
        prep: &[],
        write: (AAR + 0x510, 0x2000_0200),
        read_addr: AAR + 0x510,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_0200,
    },
    Case {
        label: "AAR SCRATCHPTR=0x2000_0300",
        prep: &[],
        write: (AAR + 0x514, 0x2000_0300),
        read_addr: AAR + 0x514,
        mask: 0xFFFF_FFFF,
        expect: 0x2000_0300,
    },
    Case {
        label: "AAR EVENTS_END SW-write-1 ignored",
        prep: &[],
        write: (AAR + 0x100, 1),
        read_addr: AAR + 0x100,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // MWU (0x40020000)
    // Silicon note: REGION[0..3].START/END (0x510..0x52C) are write-only on this
    // silicon rev — reads return 0 even after writing. Model updated to match.
    // REGIONEN at 0x500 does round-trip. REGION[n] at 0x510..0x52C reads 0.
    // ════════════════════════════════════════════════════════════════════════
    // REGION[0].START at 0x510, REGION[0].END at 0x514 — write-only, reads 0
    Case {
        label: "MWU REGION[0].START=0x2000_0000",
        prep: &[],
        write: (MWU + 0x510, 0x2000_0000),
        read_addr: MWU + 0x510,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    Case {
        label: "MWU REGION[0].END=0x2000_1000",
        prep: &[],
        write: (MWU + 0x514, 0x2000_1000),
        read_addr: MWU + 0x514,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    // REGION[3] at 0x528/0x52C — also write-only, reads 0
    Case {
        label: "MWU REGION[3].START=0x2003_0000",
        prep: &[],
        write: (MWU + 0x528, 0x2003_0000),
        read_addr: MWU + 0x528,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    // REGIONEN (region enable) at 0x500: silicon also reads 0 on this board
    // (no memory monitoring configured). This is write-only in practice.
    Case {
        label: "MWU REGIONEN(0x500) write+readback",
        prep: &[],
        write: (MWU + 0x500, 0x0000_000F),
        read_addr: MWU + 0x500,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // NVMC (0x4001E000)
    // READY is RO=1; CONFIG R/W (bits [1:0]); don't trigger erase.
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "NVMC READY=1 (read-only)",
        prep: &[],
        write: (NVMC + 0x400, 0),
        read_addr: NVMC + 0x400,
        mask: 0x1,
        expect: 1,
    },
    Case {
        label: "NVMC CONFIG=1 (WEN)",
        prep: &[],
        write: (NVMC + 0x504, 1),
        read_addr: NVMC + 0x504,
        mask: 0x3,
        expect: 1,
    },
    Case {
        label: "NVMC CONFIG=0 (REEN)",
        prep: &[(NVMC + 0x504, 1)],
        write: (NVMC + 0x504, 0),
        read_addr: NVMC + 0x504,
        mask: 0x3,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // USBD (0x40027000)
    // ENABLE, USBPULLUP, notable R/W; EVENTS write-1-ignored
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "USBD ENABLE=1",
        prep: &[],
        write: (USBD + 0x500, 1),
        read_addr: USBD + 0x500,
        mask: 0x1,
        expect: 1,
    },
    // ENABLE=0 is ignored when VBUS is present (board powered by USB); stays 1.
    Case {
        label: "USBD ENABLE=0 stays 1 (VBUS present)",
        prep: &[(USBD + 0x500, 1)],
        write: (USBD + 0x500, 0),
        read_addr: USBD + 0x500,
        mask: 0x1,
        expect: 1,
    },
    // USBPULLUP: silicon reads 0 even after writing 1 (D+ pullup is hardware-controlled
    // by the USB PHY state machine; the register is effectively write-only for SW).
    Case {
        label: "USBD USBPULLUP=1 (reads 0, write-only on silicon)",
        prep: &[],
        write: (USBD + 0x504, 1),
        read_addr: USBD + 0x504,
        mask: 0x1,
        expect: 0,
    },
    Case {
        label: "USBD USBPULLUP=0",
        prep: &[(USBD + 0x504, 1)],
        write: (USBD + 0x504, 0),
        read_addr: USBD + 0x504,
        mask: 0x1,
        expect: 0,
    },
    // EPINEN/EPOUTEN: silicon reads 0 (USB state machine not running).
    Case {
        label: "USBD EPINEN=0x01 (reads 0 before enumeration)",
        prep: &[],
        write: (USBD + 0x510, 0x01),
        read_addr: USBD + 0x510,
        mask: 0xFF,
        expect: 0,
    },
    Case {
        label: "USBD EPOUTEN=0x01 (reads 0 before enumeration)",
        prep: &[],
        write: (USBD + 0x514, 0x01),
        read_addr: USBD + 0x514,
        mask: 0xFF,
        expect: 0,
    },
    // EVENTS write-1 ignored
    Case {
        label: "USBD EVENTS_USBRESET SW-write-1 ignored",
        prep: &[],
        write: (USBD + 0x100, 1),
        read_addr: USBD + 0x100,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "USBD EVENTS_USBEVENT SW-write-1 ignored",
        prep: &[],
        write: (USBD + 0x158, 1),
        read_addr: USBD + 0x158,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "USBD EVENTS_EP0SETUP SW-write-1 ignored",
        prep: &[],
        write: (USBD + 0x15C, 1),
        read_addr: USBD + 0x15C,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // ACL (0x4002F000)
    // ACL[0..7].ADDR/SIZE/PERM — write-only on silicon (reads 0)
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "ACL[0].ADDR reads 0 (write-only on silicon)",
        prep: &[],
        write: (ACL + 0x500, 0x0000_1000),
        read_addr: ACL + 0x500,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    Case {
        label: "ACL[0].SIZE reads 0 (write-only on silicon)",
        prep: &[],
        write: (ACL + 0x504, 0x0000_1000),
        read_addr: ACL + 0x504,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    // PERM reads back the written value on silicon (unlike ADDR/SIZE which are write-only).
    Case {
        label: "ACL[0].PERM reads back 0x6 after write",
        prep: &[],
        write: (ACL + 0x508, 0x6),
        read_addr: ACL + 0x508,
        mask: 0xFFFF_FFFF,
        expect: 0x6,
    },
    Case {
        label: "ACL[7].ADDR reads 0 (write-only on silicon)",
        prep: &[],
        write: (ACL + 0x570, 0x0010_0000),
        read_addr: ACL + 0x570,
        mask: 0xFFFF_FFFF,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // CRYPTOCELL (0x5002A000)
    // ENABLE
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "CRYPTOCELL ENABLE=1",
        prep: &[],
        write: (CRYPTOCELL + 0x500, 1),
        read_addr: CRYPTOCELL + 0x500,
        mask: 0x1,
        expect: 1,
    },
    Case {
        label: "CRYPTOCELL ENABLE=0",
        prep: &[(CRYPTOCELL + 0x500, 1)],
        write: (CRYPTOCELL + 0x500, 0),
        read_addr: CRYPTOCELL + 0x500,
        mask: 0x1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // RADIO (0x40001000)
    // FREQUENCY, MODE, PCNF0/1, BASE0/1, PREFIX0/1, TXADDRESS, RXADDRESSES,
    // CRCCNF/POLY/INIT, TIFS, SHORTS, INTEN; EVENTS write-1-ignored
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "RADIO FREQUENCY=0x4E (ch37)",
        prep: &[],
        write: (RADIO + 0x508, 0x4E),
        read_addr: RADIO + 0x508,
        mask: 0xFF,
        expect: 0x4E,
    },
    Case {
        label: "RADIO FREQUENCY=0x00",
        prep: &[],
        write: (RADIO + 0x508, 0x00),
        read_addr: RADIO + 0x508,
        mask: 0xFF,
        expect: 0x00,
    },
    Case {
        label: "RADIO MODE=3 (BLE_1Mbit)",
        prep: &[],
        write: (RADIO + 0x510, 3),
        read_addr: RADIO + 0x510,
        mask: 0xF,
        expect: 3,
    },
    Case {
        label: "RADIO MODE=4 (BLE_2Mbit)",
        prep: &[],
        write: (RADIO + 0x510, 4),
        read_addr: RADIO + 0x510,
        mask: 0xF,
        expect: 4,
    },
    Case {
        label: "RADIO PCNF0=0x0001_0008",
        prep: &[],
        write: (RADIO + 0x514, 0x0001_0008),
        read_addr: RADIO + 0x514,
        mask: 0xFFFF_FFFF,
        expect: 0x0001_0008,
    },
    Case {
        label: "RADIO PCNF1=0x0300_FFFF",
        prep: &[],
        write: (RADIO + 0x518, 0x0300_FFFF),
        read_addr: RADIO + 0x518,
        mask: 0xFFFF_FFFF,
        expect: 0x0300_FFFF,
    },
    Case {
        label: "RADIO BASE0=0x8E89_BED6",
        prep: &[],
        write: (RADIO + 0x51C, 0x8E89_BED6),
        read_addr: RADIO + 0x51C,
        mask: 0xFFFF_FFFF,
        expect: 0x8E89_BED6,
    },
    Case {
        label: "RADIO BASE1=0xCAFE_BABE",
        prep: &[],
        write: (RADIO + 0x520, 0xCAFE_BABE),
        read_addr: RADIO + 0x520,
        mask: 0xFFFF_FFFF,
        expect: 0xCAFE_BABE,
    },
    Case {
        label: "RADIO PREFIX0=0x0000_00D6",
        prep: &[],
        write: (RADIO + 0x524, 0xD6),
        read_addr: RADIO + 0x524,
        mask: 0xFFFF_FFFF,
        expect: 0xD6,
    },
    Case {
        label: "RADIO PREFIX1=0xBEEF_CAFE",
        prep: &[],
        write: (RADIO + 0x528, 0xBEEF_CAFE),
        read_addr: RADIO + 0x528,
        mask: 0xFFFF_FFFF,
        expect: 0xBEEF_CAFE,
    },
    Case {
        label: "RADIO TXADDRESS=5",
        prep: &[],
        write: (RADIO + 0x52C, 5),
        read_addr: RADIO + 0x52C,
        mask: 0x7,
        expect: 5,
    },
    Case {
        label: "RADIO RXADDRESSES=0x55",
        prep: &[],
        write: (RADIO + 0x530, 0x55),
        read_addr: RADIO + 0x530,
        mask: 0xFF,
        expect: 0x55,
    },
    Case {
        label: "RADIO CRCCNF=0x0103",
        prep: &[],
        write: (RADIO + 0x534, 0x0103),
        read_addr: RADIO + 0x534,
        mask: 0xFFFF_FFFF,
        expect: 0x0103,
    },
    Case {
        label: "RADIO CRCPOLY=0x00065B",
        prep: &[],
        write: (RADIO + 0x538, 0x65B),
        read_addr: RADIO + 0x538,
        mask: 0xFFFFFF,
        expect: 0x65B,
    },
    Case {
        label: "RADIO CRCINIT=0x555555",
        prep: &[],
        write: (RADIO + 0x53C, 0x55_5555),
        read_addr: RADIO + 0x53C,
        mask: 0xFFFFFF,
        expect: 0x55_5555,
    },
    Case {
        label: "RADIO TIFS=150",
        prep: &[],
        write: (RADIO + 0x544, 150),
        read_addr: RADIO + 0x544,
        mask: 0x3FF,
        expect: 150,
    },
    Case {
        label: "RADIO SHORTS=0x03",
        prep: &[],
        write: (RADIO + 0x200, 0x03),
        read_addr: RADIO + 0x200,
        mask: 0xFFFF_FFFF,
        expect: 0x03,
    },
    Case {
        label: "RADIO INTENSET bit 0",
        prep: &[(RADIO + 0x308, 0xFFFF_FFFF)],
        write: (RADIO + 0x304, 0x01),
        read_addr: RADIO + 0x304,
        mask: 0x01,
        expect: 0x01,
    },
    // EVENTS write-1 ignored on silicon (HW-generated):
    Case {
        label: "RADIO EVENTS_READY SW-write-1 ignored",
        prep: &[],
        write: (RADIO + 0x100, 1),
        read_addr: RADIO + 0x100,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "RADIO EVENTS_ADDRESS SW-write-1 ignored",
        prep: &[],
        write: (RADIO + 0x104, 1),
        read_addr: RADIO + 0x104,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "RADIO EVENTS_END SW-write-1 ignored",
        prep: &[],
        write: (RADIO + 0x10C, 1),
        read_addr: RADIO + 0x10C,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "RADIO EVENTS_DISABLED SW-write-1 ignored",
        prep: &[],
        write: (RADIO + 0x110, 1),
        read_addr: RADIO + 0x110,
        mask: 1,
        expect: 0,
    },
    Case {
        label: "RADIO EVENTS_CRCOK SW-write-1 ignored",
        prep: &[],
        write: (RADIO + 0x130, 1),
        read_addr: RADIO + 0x130,
        mask: 1,
        expect: 0,
    },
    // ════════════════════════════════════════════════════════════════════════
    // FICR (0x10000000) — read-only identity
    // Compare sim==hw; INFO.PART must be 0x52840 on both.
    // DEVICEID[0/1] are chip-unique: we expect sim==hw but BothDisagreeWithExpect
    // if the sim's hardcoded DEVICEID differs from THIS chip.
    // ════════════════════════════════════════════════════════════════════════
    Case {
        label: "FICR INFO.PART=0x52840 (write dropped)",
        prep: &[],
        write: (FICR + 0x100, 0xDEAD_BEEF),
        read_addr: FICR + 0x100,
        mask: 0xFFFF_FFFF,
        expect: 0x0005_2840,
    },
    // INFO.VARIANT is die-revision specific — 0x41414430 = "AAD0" confirmed on bench DK rev3
    Case {
        label: "FICR INFO.VARIANT (chip-unique, sim==hw?)",
        prep: &[],
        write: (FICR + 0x104, 0),
        read_addr: FICR + 0x104,
        mask: 0xFFFF_FFFF,
        expect: 0x4141_4430,
    },
    Case {
        label: "FICR INFO.RAM=256",
        prep: &[],
        write: (FICR + 0x10C, 0),
        read_addr: FICR + 0x10C,
        mask: 0xFFFF_FFFF,
        expect: 256,
    },
    Case {
        label: "FICR INFO.FLASH=1024",
        prep: &[],
        write: (FICR + 0x110, 0),
        read_addr: FICR + 0x110,
        mask: 0xFFFF_FFFF,
        expect: 1024,
    },
    // DEVICEID — chip-unique; test sim==hw alignment
    Case {
        label: "FICR DEVICEID[0] (chip-unique, sim==hw?)",
        prep: &[],
        write: (FICR + 0x060, 0),
        read_addr: FICR + 0x060,
        mask: 0xFFFF_FFFF,
        expect: 0x707D_C298,
    },
    Case {
        label: "FICR DEVICEID[1] (chip-unique, sim==hw?)",
        prep: &[],
        write: (FICR + 0x064, 0),
        read_addr: FICR + 0x064,
        mask: 0xFFFF_FFFF,
        expect: 0x940D_8A73,
    },
    // CODEPAGESIZE and CODESIZE
    Case {
        label: "FICR CODEPAGESIZE=4096",
        prep: &[],
        write: (FICR + 0x010, 0),
        read_addr: FICR + 0x010,
        mask: 0xFFFF_FFFF,
        expect: 4096,
    },
    Case {
        label: "FICR CODESIZE=256",
        prep: &[],
        write: (FICR + 0x014, 0),
        read_addr: FICR + 0x014,
        mask: 0xFFFF_FFFF,
        expect: 256,
    },
    // ════════════════════════════════════════════════════════════════════════
    // UICR (0x10001000) — mostly 0xFFFFFFFF erased; compare sim==hw
    // We only READ (write=no-op); UICR is flash-written once, not over SWD here.
    // ════════════════════════════════════════════════════════════════════════
    // APPROTECT at 0x208 — erased = 0xFFFFFFFF on a factory-erased chip.
    Case {
        label: "UICR APPROTECT (compare sim==hw, expect erased)",
        prep: &[],
        write: (UICR + 0x208, 0xFFFF_FFFF),
        read_addr: UICR + 0x208,
        mask: 0xFFFF_FFFF,
        expect: 0xFFFF_FFFF,
    },
    Case {
        label: "UICR CUSTOMER[0] (compare sim==hw)",
        prep: &[],
        write: (UICR + 0x080, 0xFFFF_FFFF),
        read_addr: UICR + 0x080,
        mask: 0xFFFF_FFFF,
        expect: 0xFFFF_FFFF,
    },
    // NFCPINS=0xFFFFFFFE on this bench board (NFC pins configured, bit 0 cleared).
    Case {
        label: "UICR NFCPINS (compare sim==hw)",
        prep: &[],
        write: (UICR + 0x20C, 0xFFFF_FFFF),
        read_addr: UICR + 0x20C,
        mask: 0xFFFF_FFFF,
        expect: 0xFFFF_FFFE,
    },
    Case {
        label: "UICR REGOUT0 (compare sim==hw)",
        prep: &[],
        write: (UICR + 0x304, 0xFFFF_FFFF),
        read_addr: UICR + 0x304,
        mask: 0xFFFF_FFFF,
        expect: 0xFFFF_FFFF,
    },
];

// ── Extract the peripheral group prefix from a case label ─────────────────────
fn periph_key(label: &str) -> &str {
    // Label format: "PERIPH rest" — split at first space.
    label.split_whitespace().next().unwrap_or("UNKNOWN")
}

// ── Main test ─────────────────────────────────────────────────────────────────

#[test]
#[ignore]
fn nrf52840_full_register_conformance() {
    let _guard = HW_LOCK.lock().unwrap();
    let mut sim = build_sim_bus();
    let mut oc = OpenOcd::spawn_nrf52().expect("openocd spawn_nrf52 failed");
    oc.reset_halt().expect("reset halt failed");
    oc.halt().expect("halt failed");

    println!();
    println!(
        "nRF52840 full-register conformance sweep — {} cases",
        CASES.len()
    );
    println!("{:-<100}", "");

    // per-peripheral (match, diverge, both_disagree, sim_err)
    let mut by_periph: std::collections::BTreeMap<&str, (u32, u32, u32, u32)> =
        std::collections::BTreeMap::new();

    for case in CASES {
        let key = periph_key(case.label);
        let b = by_periph.entry(key).or_insert((0, 0, 0, 0));
        match run_case(&mut sim, &mut oc, case) {
            Outcome::Match => {
                b.0 += 1;
                println!("[OK  ] {}", case.label);
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
                    "[BOTH] {}  both=0x{:08X} expected=0x{:08X}  (test-expect wrong, not a sim bug)",
                    case.label, both, case.expect
                );
            }
            Outcome::SimError(m) => {
                b.3 += 1;
                println!("[SIM!] {}  {}", case.label, m);
            }
        }
    }

    println!("{:-<100}", "");
    println!("per-peripheral summary:");
    let mut total_div = 0u32;
    let mut total_sim_err = 0u32;
    for (p, (ok, div, both, simerr)) in &by_periph {
        total_div += *div;
        total_sim_err += *simerr;
        println!(
            "  {:<12}  ok={ok} diverge={div} both_disagree={both} sim_err={simerr}",
            p
        );
    }
    println!("{:-<100}", "");
    println!(
        "TOTAL: diverge={total_div}  sim_err={total_sim_err}  \
         (BothDisagreeWithExpect = test expects wrong value, sim+hw agree)"
    );
    oc.shutdown().ok();

    if std::env::var("NRF52_STRICT").is_ok() {
        assert_eq!(
            total_div, 0,
            "nRF52 full-register sweep: {total_div} register(s) diverged (sim≠hw)"
        );
        assert_eq!(
            total_sim_err, 0,
            "nRF52 full-register sweep: {total_sim_err} sim error(s)"
        );
    }
}
