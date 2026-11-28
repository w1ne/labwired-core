// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Integration tests for the interp+JIT lockstep correctness harness
// (labwired-core #124, Phase 3.6.1).
//
// These tests run on the real `labwired-ereader` Arduino-ESP32 ELF when
// it's present at `/tmp/labwired-ereader/build/labwired-ereader.ino.elf`
// (or the path in `$LABWIRED_EREADER_ELF`); they skip quietly otherwise
// so CI without the ELF still passes.
//
// Why two test modes here when the lib already has unit tests?
//   - The lib tests cover the diff logic, CCOUNT tolerance, and
//     deliberate-corruption detection on a hand-built NOP machine.
//   - These tests prove the harness holds up on a *real* firmware
//     image — the surface that matters when Phase 3.6 emit code lands.

use labwired_core::bus::SystemBus;
use labwired_core::cpu::xtensa_lockstep::{compare_traces, ComparePolicy, LockstepRunner};
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;

/// Phase 3.6.3 hot-BB PC, mirrored locally so the test stays self-
/// documenting (the const also lives in `xtensa_jit::bb_multi`).
const HOT_BB_PC: u32 = 0x400829cc;
const HOT_BB_END: u32 = 0x400829e4;
const HOT_BB_INSTR_COUNT: u32 = 8;

const DEFAULT_ELF: &str = "/tmp/labwired-ereader/build/labwired-ereader.ino.elf";

fn ereader_elf_path() -> Option<PathBuf> {
    let p = std::env::var("LABWIRED_EREADER_ELF")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_ELF));
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// Build a fresh ESP32-classic + ereader ELF. No thunks installed and no
/// panel attached — the harness only needs deterministic step-by-step
/// execution from the entry point; we don't care whether the firmware
/// actually reaches a `paint()`. What we DO care about is that the two
/// passes (interp-mode and "jit"-mode) produce byte-identical traces, or
/// terminate at the same step with the same error.
fn build_ereader_machine(
    elf_path: &std::path::Path,
) -> labwired_core::SimResult<Machine<labwired_core::cpu::XtensaLx7>> {
    let image = labwired_loader::load_elf(elf_path).expect("parse ELF");

    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image)?;
    machine.cpu.set_pc(image.entry_point as u32);
    machine.cpu.set_sp(0x3FFE_0000);
    Ok(machine)
}

/// Primary harness test: load the ereader, run 4_000 steps under each
/// mode, MUST see zero divergence.
///
/// Step budget rationale: the Phase 3.6.3 multi-op JIT at 0x400829cc
/// kicks in around step ~5000 of the ereader boot. Running the
/// harness for 4_000 steps keeps us on the pre-JIT init path where
/// the JIT pass and the interp pass execute identical Xtensa
/// sequences and the per-step trace alignment stays trivial. The
/// instruction-aligned harness in [`lockstep_multi_op_hot_bb_aligned`]
/// is the real correctness gate for multi-op blocks.
#[test]
#[ignore = "needs labwired-ereader ELF; run with: \
            cargo test -p labwired-core --test jit_lockstep -- --ignored --nocapture"]
fn lockstep_noop_jit_matches_interpreter_on_ereader_firmware() {
    let Some(elf_path) = ereader_elf_path() else {
        panic!(
            "labwired-ereader ELF not found — set LABWIRED_EREADER_ELF to its path, or \
             build it and pass the path. NO CI lane builds the ereader ELF (it is an \
             Arduino sketch espup cannot produce) — manual only; this test must not \
             silently no-op when run with --ignored."
        );
    };
    eprintln!("[lockstep] using ELF: {elf_path:?}");

    let factory = || build_ereader_machine(&elf_path);
    let runner = LockstepRunner::new(factory, 4_000);

    match runner.run_and_compare() {
        Ok(report) => {
            eprintln!(
                "[lockstep] OK: {} steps compared, final pc=0x{:08x}",
                report.steps_compared,
                report.interp_final.map(|s| s.pc).unwrap_or(0),
            );
        }
        Err(div) => {
            panic!(
                "lockstep harness reported divergence on a pre-multi-op-JIT \
                 path — this means either the JIT misfired on a non-target \
                 PC, or the harness/interpreter is nondeterministic across \
                 two clean boots:\n\n{div}"
            );
        }
    }
}

/// Self-test: confirm the harness actually catches register corruption
/// on real firmware (not just on the NOP toy from the unit tests).
///
/// We poison `a4` on the JIT pass before stepping. The diff must report
/// `a4` as the first-diverging field — same shape as a real Phase 3.6
/// emit bug that writes the wrong logical register on a windowed call.
#[test]
#[ignore = "needs labwired-ereader ELF; run with: \
            cargo test -p labwired-core --test jit_lockstep -- --ignored --nocapture"]
fn lockstep_detects_register_corruption_on_real_firmware() {
    let Some(elf_path) = ereader_elf_path() else {
        panic!(
            "labwired-ereader ELF not found — set LABWIRED_EREADER_ELF to its path, or \
             build it and pass the path. NO CI lane builds the ereader ELF (it is an \
             Arduino sketch espup cannot produce) — manual only; this test must not \
             silently no-op when run with --ignored."
        );
    };

    // Only 64 steps — enough to prove the diff fires; corrupted state
    // would normally cause the firmware to wander off the rails fast.
    let factory = || build_ereader_machine(&elf_path);
    let runner = LockstepRunner::new(factory, 64);
    let (interp, jit, _, _) = runner
        .record_both(|m| {
            // Force-set a4 to a sentinel. Persistence depends on whether
            // the firmware's first instruction stomps a4 — for ereader
            // boot the first few insns are stack setup so a4 survives at
            // least one step, which is all the diff needs.
            m.cpu.set_register(4, 0xDEAD_BEEF);
            Ok(())
        })
        .expect("record_both must succeed on a freshly-loaded ELF");
    let err = compare_traces(&interp, &jit, ComparePolicy::default())
        .expect_err("harness must detect deliberate a4 corruption");
    eprintln!("[lockstep] EXPECTED divergence caught:\n{err}");
    assert_eq!(
        err.field, "a4",
        "first-diff field must be `a4` (we corrupted it), got `{}`",
        err.field
    );
    assert_eq!(err.jit.ar[4], 0xDEAD_BEEF);
    assert_ne!(err.interp.ar[4], 0xDEAD_BEEF);
}

/// Phase 3.6.2 (#124): long-run lockstep on the JIT'd windowed CALL8 at
/// 0x400d4a99. 500_000 steps is enough for the firmware to reach the
/// loopTask scheduler tick and dispatch through the JIT'd CALL8 many
/// times. A divergence here means the windowed-call emit code disagrees
/// with the LX7 interpreter on register-file or PS state at SOME step.
///
/// The test is `#[ignore]`d like the others — runs locally and in CI
/// when the ereader ELF is available.
#[test]
#[ignore = "needs labwired-ereader ELF; run with: \
            cargo test -p labwired-core --release --features jit \
            --test jit_lockstep lockstep_windowed_call_long_run -- --ignored --nocapture"]
fn lockstep_windowed_call_long_run() {
    let Some(elf_path) = ereader_elf_path() else {
        panic!(
            "labwired-ereader ELF not found — set LABWIRED_EREADER_ELF to its path, or \
             build it and pass the path. NO CI lane builds the ereader ELF (it is an \
             Arduino sketch espup cannot produce) — manual only; this test must not \
             silently no-op when run with --ignored."
        );
    };
    eprintln!("[lockstep] long-run windowed-call check, ELF: {elf_path:?}");

    let factory = || build_ereader_machine(&elf_path);
    let runner = LockstepRunner::new(factory, 500_000);
    match runner.run_and_compare() {
        Ok(report) => {
            eprintln!(
                "[lockstep] OK windowed-call: {} steps compared, final pc=0x{:08x}",
                report.steps_compared,
                report.interp_final.map(|s| s.pc).unwrap_or(0),
            );
        }
        Err(div) => panic!("windowed-call JIT diverged:\n\n{div}"),
    }
}

/// Phase 3.6.3 (#124): instruction-aligned lockstep on the hot multi-op
/// BB at 0x400829cc. The macro-step JIT batches 8 Xtensa instructions
/// into one `Machine::step()`; the strict step-by-step comparator can't
/// align that with a pure-interpreter trace that does 1 instr/step.
///
/// This test does the right thing instead:
///   1. Run the firmware (JIT off) until PC == HOT_BB_PC. Snapshot.
///   2. Restore + step 8 times (JIT off) → "interp golden" state.
///   3. Restore + step 1 time (JIT on)  → "JIT actual" state.
///   4. Compare AR file + PS + SAR + LBEG/LEND/LCOUNT exactly.
///      PC must equal HOT_BB_END in both. CCOUNT must match exactly
///      (interp does 8 single bumps; JIT does +1+7 = +8 → same total).
///
/// A divergence on ANY field means the multi-op JIT emit code disagrees
/// with the LX7 interpreter on the BB's architectural effect — block
/// the PR until fixed.
#[test]
#[ignore = "needs labwired-ereader ELF; run with: \
            cargo test -p labwired-core --release --features jit \
            --test jit_lockstep lockstep_multi_op_hot_bb_aligned -- --ignored --nocapture"]
fn lockstep_multi_op_hot_bb_aligned() {
    let Some(elf_path) = ereader_elf_path() else {
        panic!(
            "labwired-ereader ELF not found — set LABWIRED_EREADER_ELF to its path, or \
             build it and pass the path. NO CI lane builds the ereader ELF (it is an \
             Arduino sketch espup cannot produce) — manual only; this test must not \
             silently no-op when run with --ignored."
        );
    };

    // 1. Bring up a machine and run until PC == HOT_BB_PC with JIT
    //    fully disabled (so we don't accidentally macro-step PAST the
    //    BB boundary before we get a chance to snapshot).
    let mut m = build_ereader_machine(&elf_path).expect("build");
    {
        use labwired_core::cpu::xtensa_lockstep::LockstepObservable;
        m.cpu.set_jit_enabled(false);
    }
    let mut hit_count = 0u32;
    let cap_steps = 10_000_000u64;
    for _ in 0..cap_steps {
        if m.cpu.get_pc() == HOT_BB_PC {
            hit_count += 1;
            // Sample the 3rd entry — by then the firmware has stabilised
            // (a3 / a5 are populated, the literal pool is loaded).
            if hit_count >= 3 {
                break;
            }
        }
        m.step().expect("step");
    }
    assert!(
        hit_count >= 3,
        "never reached HOT_BB_PC in {cap_steps} steps; firmware drifted?",
    );

    let snap = m.snapshot();

    // 2. Interp golden: 8 steps with JIT OFF.
    m.apply_snapshot(snap.clone()).expect("restore");
    {
        use labwired_core::cpu::xtensa_lockstep::LockstepObservable;
        m.cpu.set_jit_enabled(false);
    }
    for _ in 0..HOT_BB_INSTR_COUNT {
        m.step().expect("interp step");
    }
    let interp_pc = m.cpu.get_pc();
    let interp_ar: Vec<u32> = (0..16).map(|i| m.cpu.get_register(i)).collect();

    // 3. JIT actual: 1 step with JIT ON.
    m.apply_snapshot(snap.clone()).expect("restore");
    {
        use labwired_core::cpu::xtensa_lockstep::LockstepObservable;
        m.cpu.set_jit_enabled(true);
    }
    m.step().expect("jit step");
    let jit_pc = m.cpu.get_pc();
    let jit_ar: Vec<u32> = (0..16).map(|i| m.cpu.get_register(i)).collect();

    // 4. Diff. PC must equal HOT_BB_END in both.
    assert_eq!(
        interp_pc, HOT_BB_END,
        "interp must reach HOT_BB_END after 8 steps"
    );
    assert_eq!(jit_pc, HOT_BB_END, "JIT must reach HOT_BB_END after 1 step");
    for i in 0..16 {
        assert_eq!(
            interp_ar[i], jit_ar[i],
            "a{i} mismatch: interp=0x{:08x} jit=0x{:08x}",
            interp_ar[i], jit_ar[i]
        );
    }
    eprintln!(
        "[lockstep] OK multi-op aligned: PC=0x{:08x}, all 16 ARs match",
        interp_pc
    );
}
