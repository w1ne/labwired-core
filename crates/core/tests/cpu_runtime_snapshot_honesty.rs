// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `Cpu::runtime_snapshot` / `Cpu::apply_runtime_snapshot` must not fabricate.
//!
//! What these tests are about, in one sentence: a core that models no runtime
//! snapshot has to SAY SO, and a restore that does nothing must not report
//! success.
//!
//! Before this file existed, the trait defaults were
//! `(CpuKind::ArmCortexM, Vec::new())` and `Ok(())`. That produced two
//! confidently-wrong answers with no way for any caller to detect either:
//!
//!   * an AVR core reported its arch as `ArmCortexM`, and a Cortex-M reported
//!     a well-formed snapshot whose body was EMPTY — `snapshot capture` wrote
//!     that to a file, `WasmSimulator::take_runtime_snapshot` handed it to JS,
//!     and neither could tell it from a real capture;
//!   * `apply_runtime_snapshot` dropped the bytes and returned `Ok(())`, so
//!     `WasmSimulator::apply_runtime_snapshot` reported a good resume after
//!     leaving the CPU cold and the peripherals warm — a machine that has
//!     never existed on silicon.
//!
//! The fix is in the types, not in a runtime check: the capture returns
//! `Option`, which carries no arch tag to invent, and the restore's default is
//! an `Err`. These tests hold both ends, plus the pairing between them.

use labwired_core::bus::SystemBus;
use labwired_core::cpu::avr::Avr;
use labwired_core::runtime_snapshot::CpuKind;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::system::riscv::configure_riscv;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{Cpu, Machine};

// ── 1. The ISAs that DO model a runtime snapshot still round-trip ──────────

#[test]
fn xtensa_lx7_runtime_snapshot_round_trips() {
    let mut bus = SystemBus::new();
    let mut cpu = configure_xtensa_esp32(&mut bus);
    cpu.set_pc(0x400D_BEEF);
    for i in 0..16u8 {
        cpu.set_register(i, 0xDEAD_0000 | u32::from(i));
    }

    let (kind, blob) = cpu
        .runtime_snapshot()
        .expect("Xtensa LX7 models a runtime snapshot");
    assert_eq!(kind, CpuKind::XtensaLx7);
    assert!(!blob.is_empty(), "a modelled snapshot has a body");

    // Clobber, so a restore that did nothing would be visible.
    cpu.set_pc(0);
    for i in 0..16u8 {
        cpu.set_register(i, 0);
    }
    cpu.apply_runtime_snapshot(kind, &blob)
        .expect("Xtensa LX7 restores its own snapshot");

    assert_eq!(cpu.get_pc(), 0x400D_BEEF);
    for i in 0..16u8 {
        assert_eq!(cpu.get_register(i), 0xDEAD_0000 | u32::from(i));
    }
}

#[test]
fn riscv_runtime_snapshot_round_trips() {
    let mut bus = SystemBus::new();
    let mut cpu = configure_riscv(&mut bus);
    cpu.set_pc(0x4200_1234);
    for i in 1..16u8 {
        cpu.set_register(i, 0xC0DE_0000 | u32::from(i));
    }

    let (kind, blob) = cpu
        .runtime_snapshot()
        .expect("RISC-V models a runtime snapshot");
    assert_eq!(kind, CpuKind::RiscV);
    assert!(!blob.is_empty(), "a modelled snapshot has a body");

    cpu.set_pc(0);
    for i in 1..16u8 {
        cpu.set_register(i, 0);
    }
    cpu.apply_runtime_snapshot(kind, &blob)
        .expect("RISC-V restores its own snapshot");

    assert_eq!(cpu.get_pc(), 0x4200_1234);
    for i in 1..16u8 {
        assert_eq!(cpu.get_register(i), 0xC0DE_0000 | u32::from(i));
    }
}

// ── 2. The ISAs that do NOT report honestly, instead of claiming Cortex-M ──

#[test]
fn cortex_m_reports_no_snapshot_rather_than_an_empty_one() {
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    assert!(
        cpu.runtime_snapshot().is_none(),
        "CortexM models no runtime snapshot; it must not answer a \
         well-formed ArmCortexM tag with an empty body"
    );
}

#[test]
fn avr_never_reports_itself_as_cortex_m() {
    let cpu = Avr::new();
    // The strong form: not merely "not ArmCortexM", but no tag at all. There
    // is no `CpuKind::Avr`, so any `Some(..)` here would name silicon this
    // core is not.
    assert!(
        cpu.runtime_snapshot().is_none(),
        "an AVR core produced a CpuKind for a snapshot it does not model"
    );
}

/// The forwarding impl for `Box<dyn Cpu>` is what the browser runtime holds.
/// It must forward the honest answer rather than fall back to a default.
#[test]
fn boxed_dyn_cpu_forwards_the_honest_answer_both_ways() {
    let mut bus = SystemBus::new();
    let (cortex, _nvic) = configure_cortex_m(&mut bus);
    let boxed: Box<dyn Cpu> = Box::new(cortex);
    assert!(boxed.runtime_snapshot().is_none());

    let mut xtensa_bus = SystemBus::new();
    let xtensa = configure_xtensa_esp32(&mut xtensa_bus);
    let boxed: Box<dyn Cpu> = Box::new(xtensa);
    let (kind, blob) = boxed
        .runtime_snapshot()
        .expect("a boxed Xtensa still models a runtime snapshot");
    assert_eq!(kind, CpuKind::XtensaLx7);
    assert!(!blob.is_empty());
}

// ── 3. A discarded restore is distinguishable from a successful one ────────

#[test]
fn a_cpu_that_cannot_restore_returns_err_not_ok() {
    let mut bus = SystemBus::new();
    let (mut cortex, _nvic) = configure_cortex_m(&mut bus);
    // Every kind, including its "own": the old default swallowed all of them.
    for kind in [CpuKind::ArmCortexM, CpuKind::RiscV, CpuKind::XtensaLx7] {
        assert!(
            cortex.apply_runtime_snapshot(kind, &[1, 2, 3, 4]).is_err(),
            "CortexM returned Ok(()) after discarding a {kind:?} blob"
        );
    }

    let mut avr = Avr::new();
    assert!(
        avr.apply_runtime_snapshot(CpuKind::XtensaLx7, &[9; 64])
            .is_err(),
        "AVR returned Ok(()) after discarding an Xtensa blob"
    );
}

/// The two outcomes must not merely differ in some internal flag — they differ
/// in the `Result` a caller sees, which is the only thing `WasmSimulator` and
/// the CLI can act on.
#[test]
fn success_and_refusal_are_different_results_on_the_same_call() {
    let mut xtensa_bus = SystemBus::new();
    let mut xtensa = configure_xtensa_esp32(&mut xtensa_bus);
    xtensa.set_pc(0x400D_0042);
    let (kind, blob) = xtensa.runtime_snapshot().expect("modelled");

    let restored = xtensa.apply_runtime_snapshot(kind, &blob);
    assert!(restored.is_ok(), "the modelling core accepts its own blob");

    let mut cortex_bus = SystemBus::new();
    let (mut cortex, _nvic) = configure_cortex_m(&mut cortex_bus);
    let refused = cortex.apply_runtime_snapshot(kind, &blob);
    assert!(
        refused.is_err(),
        "the non-modelling core reported the SAME outcome as a real restore"
    );
}

// ── 4. Same contract one level up, at the Machine ──────────────────────────

#[test]
fn machine_without_a_cpu_snapshot_produces_none() {
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let machine = Machine::new(cpu, bus);
    assert!(
        machine.take_runtime_snapshot().is_none(),
        "a machine whose CPU models no snapshot must not hand back a \
         resumable-looking blob with no CPU state in it"
    );
}

#[test]
fn machine_restore_fails_before_touching_a_single_peripheral() {
    // Capture a real Xtensa machine snapshot, then try to stamp it onto a
    // Cortex-M machine. The peripheral half would happily apply; the CPU half
    // is checked first and fails the whole call.
    let mut xtensa_bus = SystemBus::new();
    let xtensa = configure_xtensa_esp32(&mut xtensa_bus);
    let mut source = Machine::new(xtensa, xtensa_bus);
    source.cpu.set_pc(0x400D_0042);
    let snap = source
        .take_runtime_snapshot()
        .expect("Xtensa machine models a runtime snapshot");

    let mut cortex_bus = SystemBus::new();
    let (cortex, _nvic) = configure_cortex_m(&mut cortex_bus);
    let mut target = Machine::new(cortex, cortex_bus);
    let pc_before = target.cpu.get_pc();

    assert!(
        target.apply_runtime_snapshot(&snap).is_err(),
        "Machine<CortexM> reported a successful resume having restored no CPU"
    );
    assert_eq!(
        target.cpu.get_pc(),
        pc_before,
        "a refused resume must not have moved the CPU"
    );
}

#[test]
fn machine_round_trip_still_works_where_it_is_modelled() {
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.cpu.set_pc(0x400D_0042);
    machine.cpu.set_register(5, 0x1234_5678);

    let snap = machine.take_runtime_snapshot().expect("modelled");
    machine.cpu.set_pc(0);
    machine.cpu.set_register(5, 0);
    machine.apply_runtime_snapshot(&snap).expect("apply");

    assert_eq!(machine.cpu.get_pc(), 0x400D_0042);
    assert_eq!(machine.cpu.get_register(5), 0x1234_5678);
}

// ── 5. The two halves may not come apart ──────────────────────────────────

/// A core that captures but cannot restore is the same lie pointed the other
/// way: `take_runtime_snapshot` hands back a blob, and the resume that reads
/// it hits the refusing default. A core that restores but cannot capture is a
/// restore path nothing in the tree can produce input for.
///
/// Enforced by reading the sources rather than by instantiating every core:
/// a new arch added tomorrow is covered without anyone remembering to list it
/// here. The scan carries its own positive control — it asserts it FOUND the
/// two cores that are known to implement both halves, so a scan that silently
/// matches nothing fails instead of passing.
#[test]
fn every_core_implements_both_snapshot_halves_or_neither() {
    let cpu_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cpu");
    let mut captures: Vec<String> = Vec::new();
    let mut restores: Vec<String> = Vec::new();
    let mut files_read = 0usize;

    let mut stack = vec![cpu_dir.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read crates/core/src/cpu") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).expect("read cpu source");
            files_read += 1;
            let name = path
                .strip_prefix(&cpu_dir)
                .unwrap_or(&path)
                .display()
                .to_string();
            // The `Cpu`-trait overrides, distinguished from the `Peripheral`
            // trait's same-named methods by their return types.
            if src
                .contains("fn runtime_snapshot(&self) -> Option<(crate::runtime_snapshot::CpuKind")
            {
                captures.push(name.clone());
            }
            if src.contains("fn apply_runtime_snapshot(") {
                restores.push(name);
            }
        }
    }

    // Positive control: the scan must see the cores that are known to be
    // there. Without this, a typo'd pattern would match nothing and the
    // pairing assertion below would pass vacuously.
    assert!(files_read >= 4, "scanned only {files_read} cpu sources");
    captures.sort();
    restores.sort();
    for known in ["riscv.rs", "xtensa_lx7.rs"] {
        assert!(
            captures.iter().any(|f| f == known),
            "the capture scan stopped finding {known}, which does implement \
             `Cpu::runtime_snapshot` — fix the pattern, do not relax the \
             assertion below (it would pass on an empty scan)"
        );
        assert!(
            restores.iter().any(|f| f == known),
            "the restore scan stopped finding {known}, which does implement \
             `Cpu::apply_runtime_snapshot`"
        );
    }

    assert_eq!(
        captures, restores,
        "a CPU core implements `runtime_snapshot` and `apply_runtime_snapshot` \
         together or not at all: capture-only means a blob nothing can restore, \
         restore-only means a path nothing can feed"
    );
}
