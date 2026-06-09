// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF52840 GPIO sim-vs-silicon conformance test — both ports, full register set.
//!
//! Covers P0 (GPIO0) and P1 (GPIO1) across the full documented register map
//! (nRF52840 PS rev 1.7, chapter 6.9):
//!
//!   OUT 0x504, OUTSET 0x508, OUTCLR 0x50C, IN 0x510 (RO),
//!   DIR 0x514, DIRSET 0x518, DIRCLR 0x51C, LATCH 0x520,
//!   DETECTMODE 0x524, PIN_CNF[n] = 0x700 + n*4
//!
//! # Address translation — the critical subtlety
//!
//! The nRF52840 hardware maps GPIO0 at 0x5000_0000 and GPIO1 at 0x5000_0300.
//! The sim remaps GPIO1 to 0x5000_1000 to avoid colliding with GPIO0's 4 KiB
//! window.  Therefore every GPIO1 operation must use SEPARATE sim and HW
//! addresses:
//!
//!   P0 sim base = 0x5000_0000  HW base = 0x5000_0000  (no remap)
//!   P1 sim base = 0x5000_1000  HW base = 0x5000_0300  (remap delta = 0xD00)
//!
//! A `RegAddr` struct carries both fields; `write_split` / `read_split` issue
//! separate operations to each.
//!
//! # Test cases (per port, per representative pin)
//!
//! Pins chosen: 3, 14, 28 (safe on both ports; avoids XIAO crystal / USB pins).
//! Additional LED pins exercised for P0 (pins 26/30/6).
//!
//!   1. DIRSET k  → DIR bit k set; DIRCLR k → DIR bit k clear.
//!   2. PIN_CNF[k] = output → OUTSET k → OUT bit k set; OUTCLR k → OUT bit k clear.
//!   3. Drive pin HIGH (DIRSET + OUTSET) → read IN bit k = 1.
//!   4. PIN_CNF[k] round-trip: write 0x0000_0001 → read back exact value.
//!   5. Multi-pin OUT after OUTSET mask (whole-register readback).
//!
//! All touched pins are restored to reset state (OUTCLR + PIN_CNF=0x0002 + DIRCLR).
//!
//! # Strict mode
//!
//! Set `NRF52_STRICT=1` to assert zero divergences (consistent with the other
//! nRF52 oracle tests in this crate).
//!
//! Run:
//! ```text
//! cargo test -p labwired-hw-oracle --test nrf52_gpio_conformance \
//!     --features hw-oracle-nrf52 -- --ignored --nocapture
//! ```

#![cfg(feature = "hw-oracle-nrf52")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_hw_oracle::openocd::OpenOcd;
use std::path::PathBuf;
use std::sync::Mutex;

// ── Address constants (nRF52840 PS rev 1.7) ──────────────────────────────────

// GPIO0 / P0
const P0_SIM_BASE: u32 = 0x5000_0000;
const P0_HW_BASE: u32 = 0x5000_0000; // same on real silicon

// GPIO1 / P1
const P1_SIM_BASE: u32 = 0x5000_1000; // remapped in the sim to avoid GPIO0's window
const P1_HW_BASE: u32 = 0x5000_0300; // true nRF52840 silicon address

// Register offsets (from the respective port base)
const OFF_OUT: u32 = 0x504;
const OFF_OUTSET: u32 = 0x508;
const OFF_OUTCLR: u32 = 0x50C;
const OFF_IN: u32 = 0x510;
const OFF_DIR: u32 = 0x514;
const OFF_DIRSET: u32 = 0x518;
const OFF_DIRCLR: u32 = 0x51C;
const OFF_LATCH: u32 = 0x520;
const OFF_DETECTMODE: u32 = 0x524;

/// Compute the PIN_CNF[k] offset for pin k (k = 0..31).
const fn off_pin_cnf(k: u32) -> u32 {
    0x700 + k * 4
}

// FICR identity register for sanity-check (read-only, same address in both worlds).
const FICR_INFO_PART: u32 = 0x1000_0100;
const EXPECTED_PART: u32 = 0x0005_2840; // nRF52840

// nRF52840 PIN_CNF reset value
const PIN_CNF_RESET: u32 = 0x0000_0002;

// Test pins for P0 (safe on P0; avoiding P0.0/1 which are crystal pins on XIAO).
const TEST_PINS_P0: &[u32] = &[3, 14, 28];

// Test pins for P1 (P1 has only 16 pins: 0..15; use pins 3, 10, 14 instead of 28).
const TEST_PINS_P1: &[u32] = &[3, 10, 14];

// Additional P0-only pins (the XIAO RGB LED pins) for the multi-pin mask test.
const P0_LED_PINS: &[u32] = &[6, 26, 30];

// ── Address pair ─────────────────────────────────────────────────────────────

/// A register that lives at different addresses in sim vs silicon.
#[derive(Clone, Copy)]
struct RegAddr {
    sim: u32,
    hw: u32,
}

impl RegAddr {
    fn p0(off: u32) -> Self {
        Self { sim: P0_SIM_BASE + off, hw: P0_HW_BASE + off }
    }
    fn p1(off: u32) -> Self {
        Self { sim: P1_SIM_BASE + off, hw: P1_HW_BASE + off }
    }
}

// ── Outcome ───────────────────────────────────────────────────────────────────

/// Outcome of a single GPIO probe case.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    /// Sim and HW both produced the expected masked value.
    Match,
    /// Sim and HW agreed but neither matched the spec expectation.
    BothDisagreeWithExpect { both: u32, expect: u32 },
    /// Sim and HW disagreed — model fidelity gap.
    Diverge { sim: u32, hw: u32 },
    /// Sim returned an error (likely unmapped address).
    SimError(String),
}

// ── Sim bus builder (identical to nrf52_mmio_diff.rs) ────────────────────────

fn build_sim_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/nrf52840.yaml");
    let system_path =
        manifest_dir.join("../../configs/systems/seeed-xiao-nrf52840-sense.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load manifest {system_path:?}: {e}"));

    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build sim bus: {e}"))
}

// ── Low-level split-address helpers ──────────────────────────────────────────

/// Write `val` to `addr.sim` in the sim and `addr.hw` on the HW separately.
fn write_split(sim: &mut SystemBus, oc: &mut OpenOcd, addr: RegAddr, val: u32) {
    sim.write_u32(addr.sim as u64, val)
        .unwrap_or_else(|e| panic!("sim write 0x{:08X} = 0x{val:08X}: {e:?}", addr.sim));
    oc.write_memory(addr.hw, &[val])
        .unwrap_or_else(|e| panic!("hw write 0x{:08X} = 0x{val:08X}: {e}", addr.hw));
}

/// Read from `addr.sim` in sim, `addr.hw` on HW.
fn read_split(sim: &mut SystemBus, oc: &mut OpenOcd, addr: RegAddr) -> (Result<u32, String>, u32) {
    let sim_result = sim
        .read_u32(addr.sim as u64)
        .map_err(|e| format!("{e:?}"));
    let hw_val = oc
        .read_memory(addr.hw, 1)
        .unwrap_or_else(|e| panic!("hw read 0x{:08X}: {e}", addr.hw))[0];
    (sim_result, hw_val)
}

/// Evaluate a single probe: write preps and the test write, then compare.
fn probe(
    sim: &mut SystemBus,
    oc: &mut OpenOcd,
    label: &str,
    preps: &[(RegAddr, u32)],
    write: (RegAddr, u32),
    read_reg: RegAddr,
    mask: u32,
    expect: u32,
) -> Outcome {
    for &(addr, val) in preps {
        write_split(sim, oc, addr, val);
    }
    write_split(sim, oc, write.0, write.1);

    let (sim_result, hw_raw) = read_split(sim, oc, read_reg);

    let sim_raw = match sim_result {
        Ok(v) => v,
        Err(e) => {
            println!("[SIM!] {label}  sim error: {e}");
            return Outcome::SimError(e);
        }
    };

    let sim_m = sim_raw & mask;
    let hw_m = hw_raw & mask;

    if sim_m == hw_m {
        if sim_m == (expect & mask) {
            Outcome::Match
        } else {
            Outcome::BothDisagreeWithExpect { both: sim_m, expect: expect & mask }
        }
    } else {
        Outcome::Diverge { sim: sim_m, hw: hw_m }
    }
}

// ── Per-port GPIO test suite ──────────────────────────────────────────────────

struct PortStats {
    matched: usize,
    diverged: usize,
    both_disagree: usize,
    sim_errors: usize,
}

impl PortStats {
    fn new() -> Self {
        Self { matched: 0, diverged: 0, both_disagree: 0, sim_errors: 0 }
    }

    fn total(&self) -> usize {
        self.matched + self.diverged + self.both_disagree + self.sim_errors
    }
}

fn record(stats: &mut PortStats, label: &str, out: Outcome) {
    match &out {
        Outcome::Match => {
            stats.matched += 1;
            println!("[OK ]  {label}");
        }
        Outcome::Diverge { sim, hw } => {
            stats.diverged += 1;
            println!("[DIFF] {label}  sim=0x{sim:08X} hw=0x{hw:08X}");
        }
        Outcome::BothDisagreeWithExpect { both, expect } => {
            stats.both_disagree += 1;
            println!("[BOTH] {label}  both=0x{both:08X} expected=0x{expect:08X}");
        }
        Outcome::SimError(msg) => {
            stats.sim_errors += 1;
            println!("[SIM!] {label}  sim error: {msg}");
        }
    }
}

fn port_name(p1: bool) -> &'static str {
    if p1 { "P1" } else { "P0" }
}

/// Run the full GPIO test suite for one port.
///
/// `p1` selects P1 vs P0; the correct sim/hw base addresses are chosen
/// accordingly so P1 reads from 0x5000_1000 in sim but 0x5000_0300 on HW.
fn run_port_tests(
    sim: &mut SystemBus,
    oc: &mut OpenOcd,
    p1: bool,
    test_pins: &[u32],
    extra_pins: &[u32],
) -> PortStats {
    let port = port_name(p1);
    let base_sim = if p1 { P1_SIM_BASE } else { P0_SIM_BASE };
    let base_hw = if p1 { P1_HW_BASE } else { P0_HW_BASE };

    let reg = |off: u32| RegAddr { sim: base_sim + off, hw: base_hw + off };

    let mut stats = PortStats::new();

    // ── Case 1: DIRSET / DIRCLR ──────────────────────────────────────────────
    for &k in test_pins {
        let bit = 1u32 << k;

        // 1a. DIRSET k → DIR has bit k set
        let label = format!("{port} DIRSET pin {k} → DIR bit set");
        let out = probe(
            sim, oc, &label,
            &[(reg(OFF_DIRCLR), bit)],         // prep: clear first
            (reg(OFF_DIRSET), bit),             // write: set
            reg(OFF_DIR),                       // read: DIR
            bit, bit,                           // mask/expect
        );
        record(&mut stats, &label, out);

        // 1b. DIRCLR k → DIR has bit k clear
        let label = format!("{port} DIRCLR pin {k} → DIR bit clear");
        let out = probe(
            sim, oc, &label,
            &[(reg(OFF_DIRSET), bit)],          // prep: set first
            (reg(OFF_DIRCLR), bit),             // write: clear
            reg(OFF_DIR),                       // read: DIR
            bit, 0,                             // mask/expect
        );
        record(&mut stats, &label, out);
    }

    // ── Case 2: OUTSET / OUTCLR (pin configured as output) ──────────────────
    for &k in test_pins {
        let bit = 1u32 << k;
        let cnf_reg = reg(off_pin_cnf(k));

        // 2a. OUTSET k → OUT bit k set
        let label = format!("{port} pin {k} output OUTSET → OUT bit set");
        let out = probe(
            sim, oc, &label,
            &[
                (cnf_reg, 0x0000_0001),         // DIR=output
                (reg(OFF_OUTCLR), bit),         // OUT=0 first
            ],
            (reg(OFF_OUTSET), bit),             // write: set
            reg(OFF_OUT),
            bit, bit,
        );
        record(&mut stats, &label, out);

        // 2b. OUTCLR k → OUT bit k clear
        let label = format!("{port} pin {k} output OUTCLR → OUT bit clear");
        let out = probe(
            sim, oc, &label,
            &[
                (cnf_reg, 0x0000_0001),         // DIR=output
                (reg(OFF_OUTSET), bit),         // OUT=1 first
            ],
            (reg(OFF_OUTCLR), bit),             // write: clear
            reg(OFF_OUT),
            bit, 0,
        );
        record(&mut stats, &label, out);
    }

    // ── Case 3: IN tracks driven output ──────────────────────────────────────
    // Drive pin HIGH (DIRSET + OUTSET); read IN — should see bit = 1 because the
    // output driver feeds back through the input buffer.
    for &k in test_pins {
        let bit = 1u32 << k;
        let cnf_reg = reg(off_pin_cnf(k));

        let label = format!("{port} pin {k} driven HIGH → IN bit 1");
        let out = probe(
            sim, oc, &label,
            &[
                // Configure as output (DIR=out, INPUT connected).
                // PIN_CNF = bit[0]=DIR(output) | bit[1]=INPUT(connect=0→connected).
                (cnf_reg, 0x0000_0001),
                (reg(OFF_DIRSET), bit),
                (reg(OFF_OUTSET), bit),
            ],
            // "Write" is a dummy second OUTSET to keep the probe structure.
            (reg(OFF_OUTSET), bit),
            reg(OFF_IN),
            bit, bit,
        );
        record(&mut stats, &label, out);
    }

    // ── Case 4: PIN_CNF round-trip ────────────────────────────────────────────
    // Write 0x0000_0001 (DIR=output, INPUT=connected, no pull, drive S0S1, no sense).
    // Read back must equal the written value in all 32 bits (all fields defined).
    for &k in test_pins {
        let cnf_reg = reg(off_pin_cnf(k));
        let test_val: u32 = 0x0000_0001;

        let label = format!("{port} PIN_CNF[{k}] round-trip write 0x{test_val:08X}");
        let out = probe(
            sim, oc, &label,
            &[],
            (cnf_reg, test_val),
            cnf_reg,
            0xFFFF_FFFF, test_val,
        );
        record(&mut stats, &label, out);
    }

    // ── Case 5: Multi-pin OUTSET → whole OUT register readback ───────────────
    // OUTSET with a bitmask covering all test_pins + extra_pins; read OUT.
    {
        let mut all_bits: u32 = 0;
        for &k in test_pins.iter().chain(extra_pins.iter()) {
            all_bits |= 1 << k;
            write_split(sim, oc, reg(off_pin_cnf(k)), 0x0000_0001); // output
        }

        // Clear OUT for all bits first, then set all at once.
        write_split(sim, oc, reg(OFF_OUTCLR), all_bits);
        write_split(sim, oc, reg(OFF_OUTSET), all_bits);

        let (sim_result, hw_raw) = read_split(sim, oc, reg(OFF_OUT));
        let label = format!("{port} multi-pin OUTSET mask 0x{all_bits:08X} → OUT readback");

        let sim_raw = match sim_result {
            Ok(v) => v,
            Err(ref e) => {
                stats.sim_errors += 1;
                println!("[SIM!] {label}  sim error: {e}");
                // Still need to proceed to cleanup.
                0
            }
        };

        if sim_result.is_ok() {
            let sim_m = sim_raw & all_bits;
            let hw_m = hw_raw & all_bits;
            let out = if sim_m == hw_m {
                if sim_m == all_bits { Outcome::Match }
                else { Outcome::BothDisagreeWithExpect { both: sim_m, expect: all_bits } }
            } else {
                Outcome::Diverge { sim: sim_m, hw: hw_m }
            };
            record(&mut stats, &label, out);
        }
    }

    // ── Case 6: LATCH register — write 1 to clear ────────────────────────────
    // On nRF52840, writing a 1 to a LATCH bit clears it (W1C). Write all-ones
    // then read back: expect 0 (all latches cleared) on both sim and HW.
    {
        let label = format!("{port} LATCH W1C: write 0xFFFF_FFFF → read 0x0");
        let out = probe(
            sim, oc, &label,
            &[],
            (reg(OFF_LATCH), 0xFFFF_FFFF),
            reg(OFF_LATCH),
            0xFFFF_FFFF, 0x0000_0000,
        );
        record(&mut stats, &label, out);
    }

    // ── Case 7: DETECTMODE register R/W ──────────────────────────────────────
    // DETECTMODE is a single bit at [0]: 0 = default (DETECT), 1 = LDETECT.
    // Write 1 and read back.
    {
        let label = format!("{port} DETECTMODE write 1 → readback 1");
        let out = probe(
            sim, oc, &label,
            &[(reg(OFF_DETECTMODE), 0)],        // prep: clear
            (reg(OFF_DETECTMODE), 1),
            reg(OFF_DETECTMODE),
            0x1, 0x1,
        );
        record(&mut stats, &label, out);

        // Also verify it can be cleared.
        let label2 = format!("{port} DETECTMODE write 0 → readback 0");
        let out2 = probe(
            sim, oc, &label2,
            &[(reg(OFF_DETECTMODE), 1)],        // prep: set
            (reg(OFF_DETECTMODE), 0),
            reg(OFF_DETECTMODE),
            0x1, 0x0,
        );
        record(&mut stats, &label2, out2);
    }

    // ── Case 8 (P1 only): Boundary case — P1 pin 28 absent (16-pin port) ──────
    // P1 GPIO1 has only 16 pins (0..15); pin 28 does not exist.
    // Write DIRSET for bit 28, then read DIR: expect bit 28 = 0 on both sim and HW.
    if p1 {
        let k = 28;
        let bit = 1u32 << k;

        let label = format!("{port} pin {k} absent (16-pin port) → DIR bit stays 0");
        let out = probe(
            sim, oc, &label,
            &[(reg(OFF_DIRCLR), bit)],          // prep: try to clear (no-op)
            (reg(OFF_DIRSET), bit),             // write: try to set bit 28
            reg(OFF_DIR),                       // read: DIR
            bit, 0,                             // mask/expect: bit 28 must be 0
        );
        record(&mut stats, &label, out);
    }

    stats
}

// ── Restore helper ────────────────────────────────────────────────────────────

/// Restore every touched pin to nRF52840 reset state on both sim and HW.
///
/// Reset state: OUTCLR(bit) + PIN_CNF[k]=0x0002 + DIRCLR(bit).
fn restore_port(
    sim: &mut SystemBus,
    oc: &mut OpenOcd,
    p1: bool,
    pins: &[u32],
) {
    let base_sim = if p1 { P1_SIM_BASE } else { P0_SIM_BASE };
    let base_hw = if p1 { P1_HW_BASE } else { P0_HW_BASE };
    let reg = |off: u32| RegAddr { sim: base_sim + off, hw: base_hw + off };

    let port = port_name(p1);
    let mut mask: u32 = 0;
    for &k in pins {
        mask |= 1 << k;
        write_split(sim, oc, reg(off_pin_cnf(k)), PIN_CNF_RESET);
    }
    write_split(sim, oc, reg(OFF_OUTCLR), mask);
    write_split(sim, oc, reg(OFF_DIRCLR), mask);
    // Restore DETECTMODE to default (0).
    write_split(sim, oc, reg(OFF_DETECTMODE), 0);

    println!("  [{port}] pins {:?} restored to reset state", pins);
}

// ── HW_LOCK (one OpenOCD at a time) ──────────────────────────────────────────

static HW_LOCK: Mutex<()> = Mutex::new(());

// ── Main test entry point ─────────────────────────────────────────────────────

#[test]
#[ignore]
fn nrf52840_gpio_conformance() {
    let _guard = HW_LOCK.lock().unwrap();

    let mut sim = build_sim_bus();
    let mut oc = OpenOcd::spawn_nrf52().expect("openocd spawn_nrf52 failed");

    oc.reset_halt().expect("reset_halt failed");
    oc.halt().expect("halt failed");

    // ── FICR sanity check ────────────────────────────────────────────────────
    // Verify we're talking to an nRF52840 (FICR INFO.PART = 0x52840).
    // This address is the same in sim and HW (ROM-mapped, no remap needed).
    let hw_part = oc
        .read_memory(FICR_INFO_PART, 1)
        .expect("read FICR INFO.PART")[0];
    println!();
    println!("nRF52840 GPIO conformance — FICR INFO.PART = 0x{hw_part:08X}");
    assert_eq!(
        hw_part, EXPECTED_PART,
        "unexpected chip: FICR INFO.PART=0x{hw_part:08X}, expected 0x{EXPECTED_PART:08X}"
    );

    // Sanity-check P1 OUT is readable at 0x5000_0804 on HW (P1_HW_BASE + OFF_OUT).
    let p1_out_hw_addr = P1_HW_BASE + OFF_OUT;
    let _p1_out = oc
        .read_memory(p1_out_hw_addr, 1)
        .unwrap_or_else(|e| panic!("P1 OUT sanity read at 0x{p1_out_hw_addr:08X}: {e}"));
    println!("P1 OUT sanity read @ 0x{p1_out_hw_addr:08X}: 0x{:08X} (OK)", _p1_out[0]);

    println!("{:-<90}", "");

    // ── P0 ───────────────────────────────────────────────────────────────────
    println!("=== P0 (GPIO0) ===");
    let p0_stats = run_port_tests(&mut sim, &mut oc, false, TEST_PINS_P0, P0_LED_PINS);
    println!("{:-<90}", "");

    // ── P1 ───────────────────────────────────────────────────────────────────
    println!("=== P1 (GPIO1, sim@0x5000_1000, hw@0x5000_0300) ===");
    let p1_stats = run_port_tests(&mut sim, &mut oc, true, TEST_PINS_P1, &[]);
    println!("{:-<90}", "");

    // ── Restore touched pins ─────────────────────────────────────────────────
    println!("Restoring pins to reset state...");
    {
        let all_p0: Vec<u32> = TEST_PINS_P0
            .iter()
            .chain(P0_LED_PINS.iter())
            .copied()
            .collect();
        restore_port(&mut sim, &mut oc, false, &all_p0);
        // P1 restore includes pin 28 (the boundary case) even though we don't drive it.
        let all_p1: Vec<u32> = TEST_PINS_P1
            .iter()
            .chain(&[28])
            .copied()
            .collect();
        restore_port(&mut sim, &mut oc, true, &all_p1);
    }
    println!("Pin restore complete.");
    println!("{:-<90}", "");

    // ── Summary ──────────────────────────────────────────────────────────────
    println!(
        "P0: match={} diverge={} both_disagree={} sim_err={} total={}",
        p0_stats.matched, p0_stats.diverged, p0_stats.both_disagree,
        p0_stats.sim_errors, p0_stats.total()
    );
    println!(
        "P1: match={} diverge={} both_disagree={} sim_err={} total={}",
        p1_stats.matched, p1_stats.diverged, p1_stats.both_disagree,
        p1_stats.sim_errors, p1_stats.total()
    );

    oc.shutdown().ok();

    // ── Strict mode ──────────────────────────────────────────────────────────
    if std::env::var("NRF52_STRICT").is_ok() {
        let total_div = p0_stats.diverged + p1_stats.diverged;
        let total_sim_err = p0_stats.sim_errors + p1_stats.sim_errors;
        assert_eq!(
            total_div, 0,
            "GPIO conformance: {total_div} register(s) diverged (P0:{} P1:{})",
            p0_stats.diverged, p1_stats.diverged
        );
        assert_eq!(
            total_sim_err, 0,
            "GPIO conformance: {total_sim_err} sim error(s) (P0:{} P1:{})",
            p0_stats.sim_errors, p1_stats.sim_errors
        );
    }
}
