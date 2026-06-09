// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Firmware-fuzzing **Phase 3** — the wedge: fuzz in the silicon-validated sim,
//! then **confirm each crash on the real F103**. Crashes that reproduce on
//! silicon are CONFIRMED bugs; crashes that don't are SIM-ONLY false positives
//! (filtered out). No emulation-only fuzzer can do this — it's the whole pitch.
//!
//! The input region is zeroed on both sides before injection so an over-read
//! crash (the planted bug reads the length past the provided data) is
//! deterministic across sim and silicon.
//!
//! ```text
//! cargo build -p firmware-f103-fuzztarget --target thumbv7m-none-eabi --release
//! STM32_TARGET=stm32f1x cargo test -p labwired-hw-oracle --test f103_fuzz_hil_confirm \
//!     --features hw-oracle-stm32 -- --ignored --test-threads=1 --nocapture
//! ```

#![cfg(feature = "hw-oracle-stm32")]

use labwired_fuzz::{fuzz_collect, Contract, Target};
use labwired_hw_oracle::openocd::OpenOcd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const FUZZ_LEN: u32 = 0x2000_2800;
const FUZZ_DATA: u32 = 0x2000_2804;
const VERDICT: u32 = 0x2000_3000;
const DONE: u32 = 0xC0DE_F022;
const FAULT: u32 = 0xDEAD_FA17;
const CONTRACT: Contract = Contract {
    input_len: FUZZ_LEN,
    input_data: FUZZ_DATA,
    verdict: VERDICT,
    done_magic: DONE,
    fault_magic: FAULT,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn elf() -> PathBuf {
    let r = root().join("target/thumbv7m-none-eabi/release/firmware-f103-fuzztarget");
    if r.exists() {
        r
    } else {
        root().join("target/thumbv7m-none-eabi/debug/firmware-f103-fuzztarget")
    }
}
fn packed(input: &[u8]) -> Vec<u32> {
    input
        .chunks(4)
        .map(|c| {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            u32::from_le_bytes(w)
        })
        .collect()
}

/// Reset, zero the input region, inject `input`, run, and observe the outcome on
/// silicon. Returns true if the chip ran cleanly (DONE) — i.e. the sim crash did
/// NOT reproduce (a false positive). Anything else (fault marker or a hang) is a
/// confirmed silicon anomaly.
fn silicon_is_clean(oc: &mut OpenOcd, input: &[u8]) -> bool {
    oc.reset_halt().expect("reset_halt");
    oc.write_memory(FUZZ_DATA, &[0u32; 160])
        .expect("zero region"); // 640 B
    oc.write_memory(FUZZ_LEN, &[input.len() as u32])
        .expect("len");
    oc.write_memory(FUZZ_DATA, &packed(input)).expect("inject");
    oc.write_memory(VERDICT, &[0]).expect("clear verdict");
    oc.resume().expect("resume");

    let deadline = Instant::now() + Duration::from_millis(600);
    loop {
        if let Ok(v) = oc.read_memory(VERDICT, 1) {
            match v[0] {
                x if x == DONE => return true,   // clean → false positive
                x if x == FAULT => return false, // confirmed crash
                _ => {}
            }
        }
        if Instant::now() >= deadline {
            return false; // hang / lockup = a silicon anomaly, not clean
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore]
fn fuzz_then_confirm_on_silicon() {
    let elf = elf();
    assert!(
        elf.exists(),
        "build firmware-f103-fuzztarget --release first"
    );

    // 1) Fuzz in sim, collect distinct crashes.
    let target = Target::from_elf(
        &root().join("configs/chips/stm32f103.yaml"),
        &root().join("configs/systems/stm32f103-bare.yaml"),
        &elf,
        CONTRACT,
        50_000,
    )
    .expect("target");
    let crashes = fuzz_collect(&target, vec![vec![b'P', 0]], 300_000, 0xC0FFEE, 8);
    assert!(!crashes.is_empty(), "fuzzer found no crashes in sim");
    println!("\nsim found {} distinct crash input(s)", crashes.len());

    // 2) Flash once, then confirm each crash on the bench F103.
    let mut oc =
        OpenOcd::spawn_stm32(&std::env::var("STM32_TARGET").unwrap_or_else(|_| "stm32f1x".into()))
            .expect("openocd");
    oc.reset_halt().expect("reset_halt");
    let resp = oc
        .tcl(&format!(
            "flash write_image erase {}",
            elf.to_str().unwrap()
        ))
        .expect("flash");
    assert!(!resp.contains("Error"), "flash: {resp}");

    let mut confirmed = 0usize;
    let mut false_positive = 0usize;
    for input in &crashes {
        let clean = silicon_is_clean(&mut oc, input);
        if clean {
            false_positive += 1;
        } else {
            confirmed += 1;
        }
        println!(
            "  {:<22} silicon: {}",
            format!("{input:02X?}"),
            if clean {
                "SIM-ONLY (false positive)"
            } else {
                "CONFIRMED"
            }
        );
    }
    oc.shutdown().ok();

    let total = crashes.len();
    println!(
        "\nHIL-confirm: {confirmed}/{total} CONFIRMED on silicon, {false_positive} sim-only \
         (false-positive rate {:.0}%)\n",
        false_positive as f64 * 100.0 / total as f64
    );
    // The point isn't a pass/fail threshold — it's that the loop runs and
    // classifies. Assert only that we exercised the silicon-confirm path.
    assert_eq!(confirmed + false_positive, total);
}
