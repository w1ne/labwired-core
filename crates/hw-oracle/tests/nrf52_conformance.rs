// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF52840 **conformance differential**: run one bare-metal firmware
//! (`firmware-nrf52840-conformance`) that drives every peripheral and writes an
//! observable-state digest to a fixed RAM block, on the simulator (full-chip
//! `Machine`) AND on real silicon, then diff the two digests. A mismatch in a
//! deterministic field is a real modeling gap; the firmware reduces timing- and
//! analog-dependent state to invariant flags so the diff has no false positives.
//!
//! Build the firmware first (cross-compiled), then run:
//! ```text
//! cargo build -p firmware-nrf52840-conformance --target thumbv7em-none-eabi --release
//! cargo test  -p labwired-hw-oracle --test nrf52_conformance            # sim only
//! NRF52_TARGET=nrf52 cargo test -p labwired-hw-oracle --test nrf52_conformance \
//!     --features hw-oracle-nrf52 -- --ignored --test-threads=1         # sim vs silicon
//! ```

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use labwired_core::Machine;
use labwired_loader::load_elf;
use std::path::{Path, PathBuf};

/// RAM address of the verdict block written by the firmware.
const VERDICT_ADDR: u32 = 0x2000_3000;
/// Written to `VERDICT[0]` last, after every test completes.
const DONE_MAGIC: u32 = 0x5284_0D0E;
/// Number of digest words (index 0 = DONE sentinel, 1..=6 = per-peripheral,
/// rest = reserved zeros).
const DIGEST_WORDS: usize = 16;

/// Human labels for the digest, for a readable gap report.
#[allow(dead_code)]
const LABELS: [&str; DIGEST_WORDS] = [
    "DONE",
    "gpio_out",
    "timer_count",
    "ecb_ct0",
    "gpiote_out",
    "temp_inrange",
    "rng_live",
    "rsv7",
    "rsv8",
    "rsv9",
    "rsv10",
    "rsv11",
    "rsv12",
    "rsv13",
    "rsv14",
    "rsv15",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Locate the cross-compiled firmware ELF (release preferred, then debug).
fn firmware_elf() -> Option<PathBuf> {
    let base = repo_root().join("target/thumbv7em-none-eabi");
    for profile in ["release", "debug"] {
        let p = base.join(profile).join("firmware-nrf52840-conformance");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Build the sim bus exactly as `nrf52_mmio_diff.rs` does.
fn build_sim_bus() -> SystemBus {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = manifest_dir.join("../../configs/chips/nrf52840.yaml");
    let system_path = manifest_dir.join("../../configs/systems/seeed-xiao-nrf52840-sense.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load manifest {system_path:?}: {e}"));

    // Resolve the chip path relative to the system manifest's directory.
    let anchored = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_str().unwrap().to_string();

    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build sim bus: {e}"))
}

/// Run the firmware on the full-chip simulator and return the digest block.
fn run_sim(elf: &Path) -> Vec<u32> {
    let mut bus = build_sim_bus();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = load_elf(elf).expect("load firmware ELF");
    machine.load_firmware(&image).expect("map firmware");

    const MAX_STEPS: usize = 50_000_000;
    let mut done = false;
    for _ in 0..MAX_STEPS {
        machine.step().expect("sim step");
        if matches!(machine.bus.read_u32(VERDICT_ADDR as u64), Ok(DONE_MAGIC)) {
            done = true;
            break;
        }
    }
    assert!(
        done,
        "firmware did not reach DONE in sim within {MAX_STEPS} steps; \
         digest state: {:08X?}",
        read_block(&machine.bus)
    );
    read_block(&machine.bus)
}

fn read_block(bus: &SystemBus) -> Vec<u32> {
    (0..DIGEST_WORDS)
        .map(|i| {
            bus.read_u32((VERDICT_ADDR + (i as u32) * 4) as u64)
                .unwrap_or(0)
        })
        .collect()
}

// ── Sim-only test (CI): the firmware runs and produces a sane digest ──────────

#[test]
fn conformance_sim() {
    let Some(elf) = firmware_elf() else {
        eprintln!(
            "skip: firmware ELF not built — run \
             `cargo build -p firmware-nrf52840-conformance \
             --target thumbv7em-none-eabi --release` first"
        );
        return;
    };
    let d = run_sim(&elf);

    println!("\nnRF52840 sim conformance digest ({DIGEST_WORDS} words):");
    for (i, (&v, label)) in d.iter().zip(LABELS.iter()).enumerate() {
        println!("  [{i:02}] {label:<14} = 0x{v:08X}");
    }

    assert_eq!(
        d[0], DONE_MAGIC,
        "sim firmware did not finish (DONE sentinel)"
    );

    // Spot-check the two deterministic fields the sim MUST get right.
    assert_eq!(d[1], 0x0000_000F, "gpio_out: expected 0xF (pins 0..3 set)");
    assert_eq!(d[2], 7, "timer_count: expected 7 TASKS_COUNT pulses");

    // The three former residuals are now modeled and deterministic, matching
    // the 2026-06-09 silicon capture. Assert them so this no-hardware CI test
    // guards the fixes against regression (the HW diff path re-confirms vs live
    // silicon when run).
    assert_eq!(
        d[3], 0xD8E0_C469,
        "ecb_ct0: AES-128 ECB must produce the FIPS-197 ciphertext word (silicon value)"
    );
    assert_eq!(
        d[4], 0,
        "gpiote_out: GPIOTE task drives pad/IN, not GPIO.OUT — OUT must stay 0 (silicon=0)"
    );
    assert_eq!(
        d[5], 1,
        "temp_inrange: TEMP must fire DATARDY with an in-range reading (silicon=1)"
    );
    assert_eq!(d[6], 1, "rng_live: RNG VALRDY must fire (liveness)");
}

// ── HW + diff (silicon) ───────────────────────────────────────────────────────

#[cfg(feature = "hw-oracle-nrf52")]
fn run_hw(elf: &PathBuf) -> Vec<u32> {
    use labwired_hw_oracle::openocd::OpenOcd;
    use std::time::{Duration, Instant};

    let mut oc = OpenOcd::spawn_nrf52().expect("spawn openocd for nRF52");
    oc.reset_halt().expect("reset_halt");

    // Flash the firmware — the ELF is linked at 0x0000_0000 (nRF52840 flash
    // starts at address 0). This overwrites the boot region; owner-approved.
    let elf_str = elf.to_str().unwrap();
    let resp = oc
        .tcl(&format!("flash write_image erase {elf_str}"))
        .expect("flash write_image");
    assert!(!resp.contains("Error"), "flash failed: {resp}");
    oc.tcl("reset run").expect("reset run");

    // Poll VERDICT[0] until DONE_MAGIC or timeout (~5 s).
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Ok(v) = oc.read_memory(VERDICT_ADDR, 1) {
            if v.first().copied() == Some(DONE_MAGIC) {
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "firmware did not reach DONE on silicon within 5 s"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    oc.halt().expect("halt");
    let block = oc
        .read_memory(VERDICT_ADDR, DIGEST_WORDS)
        .expect("read digest block");
    oc.shutdown().ok();
    block
}

/// Ratchet baseline: number of digest words that must match sim vs hw.
///
/// Originally measured 13/16 on real silicon (Seeed XIAO nRF52840 Sense,
/// ST-LINK V2, 2026-06-09). All three residuals have since been closed in the
/// sim, so every digest word now equals the captured silicon value (16/16):
///   - `ecb_ct0`:  sim now implements AES-128 (ECB EasyDMA) → 0xD8E0C469.
///   - `gpiote_out`: GPIOTE task drives pad/IN, not GPIO.OUT → 0 (matches HW).
///   - `temp_inrange`: TEMP now produces an in-range reading + DATARDY → 1.
///
/// NOTE: 16 reflects the sim digest matching the 2026-06-09 silicon capture on
/// every word (verified sim-side; see `conformance_sim`). It has not been
/// re-confirmed by a fresh on-silicon run because flashing the conformance
/// firmware overwrites the restored UF2 bootloader. Re-run `conformance_diff`
/// after a re-flash to confirm 16/16 on hardware. Never lower this baseline.
#[cfg(feature = "hw-oracle-nrf52")]
const BASELINE_MATCHED: usize = 16;

#[cfg(feature = "hw-oracle-nrf52")]
#[test]
#[ignore]
fn conformance_diff() {
    let elf = firmware_elf().expect(
        "build firmware-nrf52840-conformance first: \
         cargo build -p firmware-nrf52840-conformance --target thumbv7em-none-eabi --release",
    );
    let sim = run_sim(&elf);
    let hw = run_hw(&elf);

    let mut gaps = Vec::new();
    let mut matched = 0usize;

    for i in 0..DIGEST_WORDS {
        let label = LABELS[i];
        // For liveness-only fields (TEMP, RNG) compare the boolean flag, not
        // the raw value. Both should be 1 (event fired).
        let (sv, hv) = if i == 5 || i == 6 {
            // Clamp to 0/1 so stale poll residue on hardware doesn't cause a
            // false diverge.
            (sim[i].min(1), hw[i].min(1))
        } else {
            (sim[i], hw[i])
        };

        if sv == hv {
            matched += 1;
            println!("[OK ]  {label:<14}  0x{sv:08X}");
        } else {
            gaps.push(format!("  {label:<14}  sim 0x{sv:08X}  vs  hw 0x{hv:08X}"));
            println!("[DIFF] {label:<14}  sim 0x{sv:08X}  vs  hw 0x{hv:08X}");
        }
    }

    let pct = matched as f64 * 100.0 / DIGEST_WORDS as f64;
    println!(
        "\nnRF52840 conformance: {matched}/{DIGEST_WORDS} fields match ({pct:.0}%)\n\
         sim {sim:08X?}\nhw  {hw:08X?}"
    );
    if !gaps.is_empty() {
        println!("gaps (sim vs silicon):\n{}", gaps.join("\n"));
    }

    // Regression ratchet — conformance may never drop below the recorded
    // baseline.  Raise BASELINE_MATCHED after each approved HW run that
    // closes a modeling gap.
    assert!(
        matched >= BASELINE_MATCHED,
        "conformance regressed: {matched}/{DIGEST_WORDS} < baseline {BASELINE_MATCHED}"
    );
}
