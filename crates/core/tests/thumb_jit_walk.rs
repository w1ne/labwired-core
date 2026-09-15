// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! All-bail Thumb-2 frontend walker tests.
//!
//! The foundation milestone walks and classifies a basic block but emits no
//! wasm (`BlockPlan::is_stub`). These tests pin the walk + classification
//! policy before any codegen lands.

#![cfg(feature = "jit-framework")]

use labwired_core::cpu::jit_framework::frontend::{FrontendRefusal, IsaFrontend};
use labwired_core::cpu::jit_framework::thumb::{classify, InstrClass, ThumbFrontend};
use labwired_core::cpu::jit_framework::CodeView;
use labwired_core::decoder::arm::{decode_thumb_16, Instruction};

/// Little-endian halfword encoder for hand-built Thumb-16 blobs.
fn h(bytes: &mut Vec<u8>, half: u16) {
    bytes.extend_from_slice(&half.to_le_bytes());
}

#[test]
fn mov_add_branch_translates_as_empty_stub() {
    // Encodings verified against the shared Thumb-16 decoder before they go
    // into the blob — a wrong opcode would silently walk a different insn.
    assert_eq!(
        decode_thumb_16(0x2001),
        Instruction::MovImm { rd: 0, imm: 1 }
    );
    assert_eq!(
        decode_thumb_16(0x1C40),
        Instruction::AddImm3 {
            rd: 0,
            rn: 0,
            imm: 1
        }
    );
    // Unconditional B T2: 0xE7FC is B with imm11 = -4 → byte offset -8, which
    // from the branch at 0x04 (PC = 0x08) lands back at 0x00.
    assert_eq!(decode_thumb_16(0xE7FC), Instruction::Branch { offset: -8 });

    let mut prog = Vec::new();
    h(&mut prog, 0x2001); // MOV r0, #1
    h(&mut prog, 0x1C40); // ADDS r0, r0, #1
    h(&mut prog, 0xE7FC); // B .-4 (loop back to entry)

    let view = CodeView::new(0, &prog);
    let plan = ThumbFrontend::new()
        .translate_block(0, &view)
        .expect("MOV/ADD/B is a translatable all-bail block");

    assert_eq!(plan.entry_pc, 0);
    assert!(
        plan.instr_count >= 2,
        "block must subsume at least MOV+ADD (got {})",
        plan.instr_count
    );
    assert!(plan.is_stub(), "all-bail frontend emits no wasm");
    assert!(plan.code.is_empty(), "BlockPlan.code must stay empty");
}

#[test]
fn wfi_at_entry_is_unmodeled() {
    assert_eq!(decode_thumb_16(0xBF30), Instruction::Wfi);
    assert_eq!(classify(&Instruction::Wfi), InstrClass::Unmodeled);

    let mut prog = Vec::new();
    h(&mut prog, 0xBF30); // WFI
    let view = CodeView::new(0, &prog);

    let result = ThumbFrontend::new().translate_block(0, &view);
    match result {
        Err(FrontendRefusal::BlockTooShort) | Err(FrontendRefusal::Unsupported) => {}
        Ok(plan) => {
            assert_eq!(
                plan.instr_count, 0,
                "WFI must be cut before, so a WFI-only block has no instructions"
            );
        }
        Err(other) => panic!("unexpected refusal for WFI-only block: {other:?}"),
    }
}

#[test]
fn classify_add_sequential_branch_control_flow() {
    assert_eq!(
        classify(&Instruction::AddImm3 {
            rd: 0,
            rn: 0,
            imm: 1
        }),
        InstrClass::Sequential
    );
    assert_eq!(
        classify(&Instruction::AddImm8 { rd: 0, imm: 1 }),
        InstrClass::Sequential
    );
    assert_eq!(
        classify(&Instruction::AddReg {
            rd: 0,
            rn: 1,
            rm: 2
        }),
        InstrClass::Sequential
    );
    assert_eq!(
        classify(&Instruction::Branch { offset: -4 }),
        InstrClass::ControlFlow
    );
}
