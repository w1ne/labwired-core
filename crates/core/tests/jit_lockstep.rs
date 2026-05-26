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

/// Primary harness test: load the ereader, run 100k steps under each
/// mode, MUST see zero divergence. Today the two modes route through
/// the same `Machine::step()`; a non-zero divergence means the harness
/// itself is leaking nondeterminism (a real bug we'd need to fix before
/// trusting it for Phase 3.6).
#[test]
#[ignore = "needs labwired-ereader ELF; run with: \
            cargo test -p labwired-core --test jit_lockstep -- --ignored --nocapture"]
fn lockstep_noop_jit_matches_interpreter_on_ereader_firmware() {
    let Some(elf_path) = ereader_elf_path() else {
        eprintln!("[skip] labwired-ereader ELF missing; set LABWIRED_EREADER_ELF to enable");
        return;
    };
    eprintln!("[lockstep] using ELF: {elf_path:?}");

    let factory = || build_ereader_machine(&elf_path);
    let runner = LockstepRunner::new(factory, 100_000);

    match runner.run_and_compare() {
        Ok(report) => {
            eprintln!(
                "[lockstep] OK: {} steps compared, final pc=0x{:08x}",
                report.steps_compared,
                report.interp_final.map(|s| s.pc).unwrap_or(0),
            );
        }
        Err(div) => {
            // First divergence is the most useful artefact a Phase 3.6
            // post-merge investigation can have. Dump the full report
            // before failing.
            panic!(
                "lockstep harness reported divergence on a no-op JIT path \
                 — this means the harness or interpreter itself is \
                 nondeterministic across two clean Machine boots:\n\n{div}"
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
        eprintln!("[skip] labwired-ereader ELF missing; set LABWIRED_EREADER_ELF to enable");
        return;
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
