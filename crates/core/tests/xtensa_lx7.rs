// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Integration tests for XtensaLx7 CPU struct, fetch loop, and Cpu trait.
//!
//! Bus construction note: the default SystemBus::new() provides:
//!   - flash at 0x0000_0000..0x0010_0000 (1 MB)
//!   - ram   at 0x2000_0000..0x2010_0000 (1 MB)
//! Neither covers 0x4000_0400 (the ESP32-S3 ROM reset vector).
//!
//! Chosen approach: use RAM at 0x2000_0000 for instruction placement in tests
//! and override cpu.pc to 0x2000_0000. This lets us exercise the fetch/decode
//! logic without introducing a new IRAM bus region. The real reset PC
//! (0x4000_0400) is separately verified by the reset_establishes_lx7_initial_state
//! test, which does not attempt to fetch from the bus.

use labwired_core::bus::SystemBus;
use labwired_core::cpu::xtensa_lx7::XtensaLx7;
use labwired_core::cpu::xtensa_sr::VECBASE;
use labwired_core::{Bus, Cpu, SimulationError};

/// Address in default RAM that all fetch tests redirect PC to.
const TEST_PC: u32 = 0x2000_0000;

#[test]
fn reset_establishes_lx7_initial_state() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();

    assert_eq!(
        cpu.get_pc(),
        0x4000_0400,
        "reset PC must be 0x40000400 (ROM reset vector)"
    );
    // HW-verified via OpenOCD on real S3-Zero: ps = 0x1f → INTLEVEL=0xF, EXCM=1.
    assert_eq!(cpu.ps.intlevel(), 0xF, "PS.INTLEVEL=0xF at reset (all interrupts masked)");
    assert!(cpu.ps.excm(), "PS.EXCM=1 at reset (exception mode active)");
    assert!(!cpu.ps.woe(), "PS.WOE=0 at reset (window overflow disabled)");
    assert_eq!(cpu.regs.windowbase(), 0, "WindowBase=0 at reset");
    assert_eq!(
        cpu.regs.windowstart(),
        0x1,
        "WindowStart=0x1 at reset (a0..a3 frame)"
    );
    assert_eq!(
        cpu.sr.read(VECBASE),
        0x4000_0000,
        "VECBASE=0x40000000 at reset"
    );
}

#[test]
fn step_with_wide_instruction_returns_notimplemented_without_advancing_pc() {
    // ADD a3, a4, a5 in wide format: op0=0x0 (byte 0 = 0x00), so length = 3 bytes.
    // Write 0x00_85_30 little-endian: bytes [0x00, 0x85, 0x30].
    // The decoder will see op0=0x0 and try decode_qrst — resulting in some wide instruction.
    let mut cpu = XtensaLx7::new();
    let mut bus = build_bus_with_instruction_at(TEST_PC as u64, 0x00_85_30);
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    let err = cpu.step(&mut bus, &[]).unwrap_err();
    assert!(
        matches!(err, SimulationError::NotImplemented(_)),
        "exec stub should return NotImplemented for decoded wide instruction, got: {:?}",
        err
    );
    // PC must NOT advance when exec fails (our chosen policy: only advance on success).
    assert_eq!(cpu.get_pc(), TEST_PC);
}

#[test]
fn step_dispatches_narrow_via_length_predecoder() {
    // op0 = 0x8 in byte 0 → narrow (2-byte) instruction.
    // Write halfword 0x0008 little-endian: byte[0]=0x08, byte[1]=0x00.
    // xtensa_length::instruction_length(0x08) == 2 → narrow path.
    // decode_narrow returns Unknown(...) in Plan 1 — that causes NotImplemented.
    let mut cpu = XtensaLx7::new();
    let mut bus = build_bus_with_instruction_at(TEST_PC as u64, 0x0008);
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    let err = cpu.step(&mut bus, &[]).unwrap_err();
    assert!(
        matches!(err, SimulationError::NotImplemented(_)),
        "narrow fetch stub should return NotImplemented for Unknown, got: {:?}",
        err
    );
}

#[test]
fn snapshot_and_apply_roundtrip() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();

    // Mutate state.
    cpu.set_pc(0x1234_5678);
    cpu.set_register(3, 0xABCD_EF01);

    let snap = cpu.snapshot();
    let mut cpu2 = XtensaLx7::new();
    cpu2.reset(&mut bus).unwrap();
    cpu2.apply_snapshot(&snap);

    assert_eq!(cpu2.get_pc(), 0x1234_5678);
    assert_eq!(cpu2.get_register(3), 0xABCD_EF01);
}

#[test]
fn set_sp_writes_a1() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();

    cpu.set_sp(0xDEAD_BEEF);
    assert_eq!(cpu.get_register(1), 0xDEAD_BEEF);
}

#[test]
fn get_register_names_returns_sixteen_ar_names() {
    let cpu = XtensaLx7::new();
    let names = cpu.get_register_names();
    assert_eq!(names.len(), 16);
    assert_eq!(names[0], "a0");
    assert_eq!(names[15], "a15");
}

/// Write the low bytes of `word` little-endian at `addr` into default RAM.
/// Only writes 3 bytes (wide instruction size) unless op0 indicates narrow.
fn build_bus_with_instruction_at(addr: u64, word: u32) -> SystemBus {
    let mut bus = SystemBus::new();
    bus.write_u8(addr, (word & 0xFF) as u8).unwrap();
    bus.write_u8(addr + 1, ((word >> 8) & 0xFF) as u8).unwrap();
    bus.write_u8(addr + 2, ((word >> 16) & 0xFF) as u8).unwrap();
    bus
}
