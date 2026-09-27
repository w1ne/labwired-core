// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Scheduled fault injection with a lockstep verdict, on a real firmware.
//!
//! The fixture is the committed nRF54L15 smart-ring probe (never skips): a
//! Cortex-M33 image that reads four I²C WHO_AM_I registers and prints each
//! answer on the console.

mod common;

use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::*;
use labwired_core::vfi::{FaultAction, FaultPlan, FaultVerdict, RegisterRef, ScheduledFault};

fn open_ring() -> Session {
    let f = common::smart_ring_fixture();
    let blobs = BlobMap::new();
    Session::open(
        BuildRequest {
            chip: &f.chip,
            system: &f.manifest,
            firmware: FirmwareSource::Elf(&f.fw),
            boot: BootMode::FastBoot,
            blobs: &blobs,
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .unwrap()
}

fn plan(at_cycle: u64, action: FaultAction, until_cycle: u64) -> FaultPlan {
    FaultPlan {
        faults: vec![ScheduledFault { at_cycle, action }],
        until_cycle,
    }
}

fn flip(register: &str, bit: u8) -> FaultAction {
    FaultAction::RegisterBitFlip {
        register: RegisterRef::Name(register.into()),
        bit,
    }
}

/// Every verdict the engine can give, each reached on the same real firmware
/// by a different fault at cycle 500 of a 20 000-cycle run.
#[test]
fn each_verdict_on_real_firmware() {
    let s = open_ring();
    let f = common::smart_ring_fixture();

    // A PC flip into unmapped space: the faulted core faults on the fetch.
    let r = s
        .fault_experiment(&plan(500, flip("pc", 28), 20_000))
        .unwrap();
    assert_eq!(r.verdict, FaultVerdict::Crashed, "{}", r.summary);
    assert_eq!(r.injected.len(), 1);
    assert_eq!(r.injected[0].applied_cycle, 500);
    let stop = r.faulted.stopped.as_deref().unwrap();
    assert!(stop.contains("Memory access violation"), "{stop}");
    assert!(r.golden.stopped.is_none());
    assert_eq!(r.golden.cycles, 20_000);

    // R0 carries a byte the firmware prints: the console changes.
    let r = s
        .fault_experiment(&plan(500, flip("R0", 3), 20_000))
        .unwrap();
    assert_eq!(r.verdict, FaultVerdict::OutputChanged, "{}", r.summary);
    let d = r.first_divergence.as_ref().unwrap();
    assert!(!d.control_flow);
    assert_eq!(d.registers.len(), 1);
    assert_eq!(d.registers[0].register, "R0");
    assert_eq!(d.registers[0].golden ^ d.registers[0].faulted, 1 << 3);
    assert!(r.console_divergence_offset.is_some());

    // R7 is overwritten before it is used: the machines become identical again.
    let r = s
        .fault_experiment(&plan(500, flip("r7", 0), 20_000))
        .unwrap();
    assert_eq!(r.verdict, FaultVerdict::Masked, "{}", r.summary);
    assert!(r.first_divergence.is_some());
    let back = r.reconverged_cycle.unwrap();
    assert!(back > 501, "reconverged at {back}");
    assert_eq!(r.golden.pc, r.faulted.pc);

    // R12 is never rewritten in the window: state differs, console does not.
    let r = s
        .fault_experiment(&plan(500, flip("r12", 0), 20_000))
        .unwrap();
    assert_eq!(r.verdict, FaultVerdict::Diverged, "{}", r.summary);
    assert!(r.console_divergence_offset.is_none());

    // A RAM byte the firmware never reads: the upset stays in memory.
    let mid = f.chip.ram.base + f.chip.ram.size / 2;
    let r = s
        .fault_experiment(&plan(
            500,
            FaultAction::MemoryBitFlip {
                address: mid,
                bit: 0,
            },
            20_000,
        ))
        .unwrap();
    assert_eq!(r.verdict, FaultVerdict::Latent, "{}", r.summary);
    assert_eq!(r.memory.len(), 1);
    assert_eq!(
        r.memory[0].golden.unwrap() ^ r.memory[0].faulted.unwrap(),
        1
    );

    // A skipped instruction on the print path loses console bytes.
    let r = s
        .fault_experiment(&plan(500, FaultAction::InstructionSkip, 20_000))
        .unwrap();
    assert_eq!(r.verdict, FaultVerdict::OutputChanged, "{}", r.summary);
    assert!(r.faulted.console_bytes < r.golden.console_bytes);
    assert!(
        r.injected[0].detail.contains("4-byte"),
        "{}",
        r.injected[0].detail
    );

    // A fault scheduled after the end never fires, and the sides agree.
    let r = s
        .fault_experiment(&plan(50_000, FaultAction::InstructionSkip, 20_000))
        .unwrap();
    assert_eq!(r.verdict, FaultVerdict::NotInjected);
    assert_eq!(r.not_injected, vec![0]);
    assert_eq!(r.golden, r.faulted);
}

/// The experiment does not touch the session it runs from.
#[test]
fn the_session_is_unchanged() {
    let mut s = open_ring();
    s.run_cycles(300).unwrap();
    let before = (s.cycles(), s.uart_transcript());
    s.fault_experiment(&plan(500, flip("pc", 28), 5_000))
        .unwrap();
    assert_eq!((s.cycles(), s.uart_transcript()), before);
}

/// Same inputs, same report, twice; and a restored session gives the same
/// report as one that ran straight to the same point.
#[test]
fn deterministic_and_restore_equivalent() {
    let p = FaultPlan {
        faults: vec![
            ScheduledFault {
                at_cycle: 1_200,
                action: flip("R0", 3),
            },
            ScheduledFault {
                at_cycle: 2_000,
                action: FaultAction::InstructionSkip,
            },
        ],
        until_cycle: 20_000,
    };

    let mut straight = open_ring();
    straight.run_cycles(1_000).unwrap();
    let a = straight.fault_experiment(&p).unwrap();
    let b = straight.fault_experiment(&p).unwrap();
    assert_eq!(a, b, "same inputs must give the same report");
    assert_eq!(a.injected.len(), 2);

    let mut restored = open_ring();
    restored.run_cycles(1_000).unwrap();
    let snap = restored.snapshot();
    restored.run_cycles(7_000).unwrap();
    restored.restore(&snap).unwrap();
    let c = restored.fault_experiment(&p).unwrap();
    assert_eq!(a, c, "restore then inject must equal a straight run");
}

/// Plans the engine cannot honour are errors, not verdicts.
#[test]
fn bad_plans_are_errors() {
    let s = open_ring();
    let e = s
        .fault_experiment(&plan(500, flip("r99", 0), 20_000))
        .unwrap_err()
        .to_string();
    assert!(e.contains("unknown register 'r99'"), "{e}");
    let e = s
        .fault_experiment(&plan(500, flip("r0", 40), 20_000))
        .unwrap_err()
        .to_string();
    assert!(e.contains("out of range"), "{e}");
    let e = s
        .fault_experiment(&plan(0, flip("r0", 1), 0))
        .unwrap_err()
        .to_string();
    assert!(e.contains("not after the current cycle"), "{e}");
}

/// The plan's JSON shape, shared by every surface.
#[test]
fn plan_json_shape() {
    let p: FaultPlan = serde_json::from_str(
        r#"{"until_cycle": 100, "faults": [
            {"at_cycle": 5, "kind": "register_bit_flip", "register": "R3", "bit": 1},
            {"at_cycle": 6, "kind": "register_bit_flip", "register": 2, "bit": 0},
            {"at_cycle": 7, "kind": "memory_bit_flip", "address": 536870912, "bit": 7},
            {"at_cycle": 8, "kind": "instruction_skip"}
        ]}"#,
    )
    .unwrap();
    assert_eq!(p.faults.len(), 4);
    assert_eq!(p.faults[1].action, flip_index(2, 0));
    assert!(serde_json::from_str::<FaultPlan>(
        r#"{"until_cycle": 1, "faults": [{"at_cycle": 1, "kind": "bus_nack"}]}"#
    )
    .is_err());
}

fn flip_index(i: u8, bit: u8) -> FaultAction {
    FaultAction::RegisterBitFlip {
        register: RegisterRef::Index(i),
        bit,
    }
}
