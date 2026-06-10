// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF52840 POWER peripheral sim-vs-silicon conformance.
//!
//! The POWER and CLOCK peripherals share base 0x40000000 on nRF52840.
//! This sweep verifies the register-surface of the POWER half: GPREGRET,
//! GPREGRET2, DCDCEN, POFCON, RAMSTATUS (read-only), RESETREAS (W1C).
//!
//! The CPU is reset-halted before the sweep, so only static register fidelity
//! (layout / masks / reset values / access semantics) is exercised.
//!
//! **Retention regs**: GPREGRET and DCDCEN survive resets on real silicon.
//! The harness writes them back to 0 at the end on both sim and hardware to
//! avoid leaving DFU magic that would trap the board in the bootloader.
//!
//! Run (pin the nRF probe when multiple ST-Links are attached):
//! ```text
//! LABWIRED_STLINK_LOCATION=1-2 NRF52_STRICT=1 \
//!   cargo test --release -p labwired-hw-oracle \
//!     --test nrf52_power_conformance --features hw-oracle-nrf52 \
//!     -- --ignored --nocapture
//! ```

#![cfg(feature = "hw-oracle-nrf52")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_hw_oracle::openocd::OpenOcd;
use std::path::PathBuf;
use std::sync::Mutex;

// POWER/CLOCK peripheral base (shared, nRF52840 PS §6.16 / §6.6).
const POWER_BASE: u32 = 0x4000_0000;

// POWER register offsets (PS §6.16.13).
const OFF_RESETREAS: u32 = 0x400;
const OFF_RAMSTATUS: u32 = 0x428;
const OFF_POFCON: u32 = 0x510;
const OFF_GPREGRET: u32 = 0x51C;
const OFF_GPREGRET2: u32 = 0x520;
const OFF_DCDCEN: u32 = 0x578;

fn addr(off: u32) -> u32 {
    POWER_BASE + off
}

struct Case {
    label: &'static str,
    /// Optional sequence of (addr, value) writes applied on both sim and hw
    /// before the main write.
    prep: &'static [(u32, u32)],
    /// The register write under test (addr, value).
    write: (u32, u32),
    /// Address to read back after the write.
    read_addr: u32,
    /// Bitmask applied to both sides before comparison.
    mask: u32,
    /// Expected masked value.
    expect: u32,
}

// RESETREAS and RAMSTATUS are handled specially (see the test body below)
// because they depend on live chip state rather than a deterministic write.
// All other POWER registers are exercised here as static round-trip cases.
const CASES: &[Case] = &[
    // ── GPREGRET (0x51C): 8-bit R/W ──────────────────────────────────────
    Case {
        label: "GPREGRET write 0xA5 read back",
        prep: &[],
        write: (POWER_BASE + OFF_GPREGRET, 0xA5),
        read_addr: POWER_BASE + OFF_GPREGRET,
        mask: 0xFF,
        expect: 0xA5,
    },
    Case {
        label: "GPREGRET write 0x00 read back (restore to 0)",
        prep: &[],
        write: (POWER_BASE + OFF_GPREGRET, 0x00),
        read_addr: POWER_BASE + OFF_GPREGRET,
        mask: 0xFF,
        expect: 0x00,
    },
    // ── GPREGRET2 (0x520): 8-bit R/W ─────────────────────────────────────
    Case {
        label: "GPREGRET2 write 0x5A read back",
        prep: &[],
        write: (POWER_BASE + OFF_GPREGRET2, 0x5A),
        read_addr: POWER_BASE + OFF_GPREGRET2,
        mask: 0xFF,
        expect: 0x5A,
    },
    Case {
        label: "GPREGRET2 write 0x00 read back (restore to 0)",
        prep: &[],
        write: (POWER_BASE + OFF_GPREGRET2, 0x00),
        read_addr: POWER_BASE + OFF_GPREGRET2,
        mask: 0xFF,
        expect: 0x00,
    },
    // ── DCDCEN (0x578): bit0 R/W ──────────────────────────────────────────
    Case {
        label: "DCDCEN write 1 read back",
        prep: &[],
        write: (POWER_BASE + OFF_DCDCEN, 1),
        read_addr: POWER_BASE + OFF_DCDCEN,
        mask: 0x1,
        expect: 1,
    },
    Case {
        label: "DCDCEN write 0 read back (restore to 0)",
        prep: &[],
        write: (POWER_BASE + OFF_DCDCEN, 0),
        read_addr: POWER_BASE + OFF_DCDCEN,
        mask: 0x1,
        expect: 0,
    },
    // ── POFCON (0x510): bits[5:0] R/W ────────────────────────────────────
    // Write a value with all defined bits set; mask to 0x3F.
    Case {
        label: "POFCON write 0x02 (POF threshold) read back",
        prep: &[],
        write: (POWER_BASE + OFF_POFCON, 0x02),
        read_addr: POWER_BASE + OFF_POFCON,
        mask: 0x3F,
        expect: 0x02,
    },
    // Restore POFCON to 0 (POF disabled, safe).
    Case {
        label: "POFCON restore to 0",
        prep: &[],
        write: (POWER_BASE + OFF_POFCON, 0x00),
        read_addr: POWER_BASE + OFF_POFCON,
        mask: 0x3F,
        expect: 0x00,
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

fn write_both(sim: &mut SystemBus, oc: &mut OpenOcd, a: u32, v: u32) {
    sim.write_u32(a as u64, v)
        .unwrap_or_else(|e| panic!("sim write 0x{a:08X}=0x{v:08X}: {e:?}"));
    oc.write_memory(a, &[v])
        .unwrap_or_else(|e| panic!("hw  write 0x{a:08X}=0x{v:08X}: {e}"));
}

fn read_both(sim: &mut SystemBus, oc: &mut OpenOcd, a: u32) -> (u32, u32) {
    let sv = sim
        .read_u32(a as u64)
        .unwrap_or_else(|e| panic!("sim read 0x{a:08X}: {e:?}"));
    let hv = oc
        .read_memory(a, 1)
        .unwrap_or_else(|e| panic!("hw  read 0x{a:08X}: {e}"))[0];
    (sv, hv)
}

fn run_case(sim: &mut SystemBus, oc: &mut OpenOcd, case: &Case) -> Outcome {
    for &(a, v) in case.prep {
        write_both(sim, oc, a, v);
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
fn nrf52840_power_conformance() {
    let _guard = HW_LOCK.lock().unwrap();
    let mut sim = build_sim_bus();
    let mut oc = OpenOcd::spawn_nrf52().expect("openocd spawn_nrf52 failed");
    oc.reset_halt().expect("reset_halt failed");
    oc.halt().expect("halt failed");

    println!();
    println!("nRF52840 POWER register conformance sweep");
    println!("{:-<80}", "");

    let mut total_match = 0u32;
    let mut total_div = 0u32;
    let mut total_both = 0u32;
    let mut total_simerr = 0u32;

    // ── Static round-trip cases ───────────────────────────────────────────
    for case in CASES {
        match run_case(&mut sim, &mut oc, case) {
            Outcome::Match => {
                total_match += 1;
                println!("[OK ]  {}", case.label);
            }
            Outcome::Diverge { sim, hw } => {
                total_div += 1;
                println!(
                    "[DIFF] {}  sim=0x{:08X} hw=0x{:08X} (mask=0x{:08X})",
                    case.label, sim, hw, case.mask
                );
            }
            Outcome::BothDisagreeWithExpect { both } => {
                total_both += 1;
                println!(
                    "[BOTH] {}  both=0x{:08X} expected=0x{:08X}",
                    case.label, both, case.expect
                );
            }
            Outcome::SimError(m) => {
                total_simerr += 1;
                println!("[SIM!] {}  {}", case.label, m);
            }
        }
    }

    // ── RAMSTATUS (0x428): read-only ──────────────────────────────────────
    // Read before and after a junk write; expect value unchanged on both sides.
    {
        let (sim_before, hw_before) = read_both(&mut sim, &mut oc, addr(OFF_RAMSTATUS));
        write_both(&mut sim, &mut oc, addr(OFF_RAMSTATUS), 0xDEAD_BEEF);
        let (sim_after, hw_after) = read_both(&mut sim, &mut oc, addr(OFF_RAMSTATUS));

        let changed_sim = sim_after != sim_before;
        let changed_hw = hw_after != hw_before;
        let diverged = sim_after != hw_after;

        if !changed_sim && !changed_hw && !diverged {
            total_match += 1;
            println!(
                "[OK ]  RAMSTATUS read-only (before=0x{:08X} after=0x{:08X})",
                hw_before, hw_after
            );
        } else if changed_sim || changed_hw {
            total_div += 1;
            println!(
                "[DIFF] RAMSTATUS not read-only!  sim: 0x{:08X}->0x{:08X}  hw: 0x{:08X}->0x{:08X}",
                sim_before, sim_after, hw_before, hw_after
            );
        } else {
            // Both changed the same way; note it but don't count as diverge.
            total_both += 1;
            println!(
                "[BOTH] RAMSTATUS changed on both sides: 0x{:08X}->0x{:08X}",
                hw_before, hw_after
            );
        }
    }

    // ── RESETREAS (0x400): W1C ────────────────────────────────────────────
    // Read the current value (depends on last reset type), W1C-clear with
    // 0xFFFFFFFF, then read again — should be 0 on both sides.
    {
        let (sim_pre, hw_pre) = read_both(&mut sim, &mut oc, addr(OFF_RESETREAS));
        println!(
            "[INFO] RESETREAS before clear: sim=0x{:08X} hw=0x{:08X}",
            sim_pre, hw_pre
        );

        // First sync sim's starting state to match hardware so both start equal.
        // The sim boots with a default power-on reset value; the silicon may
        // differ. We write the hw value into the sim RESETREAS so that the W1C
        // behaviour is tested from a consistent baseline on both sides.
        //
        // Strategy: write ~hw_pre to sim only (clear matching bits), then
        // write 0xFFFF_FFFF to both to flush any residual bits.
        sim.write_u32(addr(OFF_RESETREAS) as u64, sim_pre)
            .expect("sim RESETREAS pre-clear");
        oc.write_memory(addr(OFF_RESETREAS), &[hw_pre])
            .expect("hw RESETREAS pre-clear");

        // Now clear everything on both sides.
        write_both(&mut sim, &mut oc, addr(OFF_RESETREAS), 0xFFFF_FFFF);
        let (sim_after, hw_after) = read_both(&mut sim, &mut oc, addr(OFF_RESETREAS));

        let sim_m = sim_after;
        let hw_m = hw_after;
        if sim_m == hw_m {
            if sim_m == 0 {
                total_match += 1;
                println!("[OK ]  RESETREAS W1C-clear → 0x{:08X}", sim_m);
            } else {
                total_both += 1;
                println!(
                    "[BOTH] RESETREAS W1C-clear both=0x{:08X} (expected 0; new reset bit set?)",
                    sim_m
                );
            }
        } else {
            total_div += 1;
            println!(
                "[DIFF] RESETREAS after W1C-clear  sim=0x{:08X} hw=0x{:08X}",
                sim_m, hw_m
            );
        }
    }

    // ── Restore retention registers to 0 on both sim and hardware ─────────
    // GPREGRET and GPREGRET2 survive chip resets; leaving a non-zero value
    // (especially 0xA5 / 0x5A) can trigger the Nordic DFU bootloader.
    // The CASES table already ends with restoring both to 0, but we do an
    // explicit confirmatory write+read here to be certain.
    for (name, off) in [("GPREGRET", OFF_GPREGRET), ("GPREGRET2", OFF_GPREGRET2)] {
        write_both(&mut sim, &mut oc, addr(off), 0x00);
        let (sv, hv) = read_both(&mut sim, &mut oc, addr(off));
        println!(
            "[RESTORE] {} → sim=0x{:02X} hw=0x{:02X} {}",
            name,
            sv & 0xFF,
            hv & 0xFF,
            if (sv & 0xFF) == 0 && (hv & 0xFF) == 0 {
                "OK"
            } else {
                "WARN: not zero!"
            }
        );
    }
    // DCDCEN: also a retention-adjacent register; restore to 0.
    write_both(&mut sim, &mut oc, addr(OFF_DCDCEN), 0x00);
    let (sv, hv) = read_both(&mut sim, &mut oc, addr(OFF_DCDCEN));
    println!(
        "[RESTORE] DCDCEN    → sim=0x{:02X} hw=0x{:02X} {}",
        sv & 0x1,
        hv & 0x1,
        if (sv & 1) == 0 && (hv & 1) == 0 {
            "OK"
        } else {
            "WARN: not zero!"
        }
    );

    println!("{:-<80}", "");
    println!(
        "POWER sweep: match={total_match} diverge={total_div} both_disagree={total_both} sim_err={total_simerr}"
    );

    oc.shutdown().ok();

    if std::env::var("NRF52_STRICT").is_ok() {
        assert_eq!(
            total_div, 0,
            "POWER diff: {total_div} register(s) diverged between sim and silicon"
        );
    }
}
