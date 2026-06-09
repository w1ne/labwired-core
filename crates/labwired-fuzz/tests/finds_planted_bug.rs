// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! The coverage-guided fuzzer finds the planted overflow in
//! `firmware-f103-fuzztarget` from a benign seed — the Phase-1 "it works" proof.

use labwired_fuzz::{fuzz, Contract, Target, Verdict};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn target() -> Option<Target> {
    let elf = root().join("target/thumbv7m-none-eabi/release/firmware-f103-fuzztarget");
    let elf = if elf.exists() {
        elf
    } else {
        root().join("target/thumbv7m-none-eabi/debug/firmware-f103-fuzztarget")
    };
    if !elf.exists() {
        return None;
    }
    Target::from_elf(
        &root().join("configs/chips/stm32f103.yaml"),
        &root().join("configs/systems/stm32f103-bare.yaml"),
        &elf,
        Contract {
            input_len: 0x2000_2800,
            input_data: 0x2000_2804,
            verdict: 0x2000_3000,
            done_magic: 0xC0DE_F022,
            fault_magic: 0xDEAD_FA17,
        },
        100_000,
    )
    .ok()
}

#[test]
fn fuzzer_finds_the_planted_overflow() {
    let Some(t) = target() else {
        eprintln!(
            "skip: build firmware-f103-fuzztarget --target thumbv7m-none-eabi --release first"
        );
        return;
    };
    // Sanity: a benign seed is clean.
    let mut cov = labwired_fuzz::CovMap::new();
    assert_eq!(t.run(&[b'P', 0], &mut cov), Verdict::Clean);

    // Fuzz from the benign seed — coverage-guided — find the crash.
    let report = fuzz(&t, vec![vec![b'P', 0]], 200_000, 0xC0FFEE);
    let crash = report
        .crash
        .expect("fuzzer should find the planted overflow");
    println!(
        "found crash in {} iters (corpus {}, {} edges): {:02X?}",
        report.iterations, report.corpus_size, report.edges_hit, crash
    );
    // The only crashing path is op 'C' with an over-long length.
    assert!(
        crash.contains(&b'C'),
        "crash input must drive the 'C' path: {crash:02X?}"
    );
    // Confirm it reproduces deterministically.
    let mut cov2 = labwired_fuzz::CovMap::new();
    assert_eq!(t.run(&crash, &mut cov2), Verdict::Crash);
}
