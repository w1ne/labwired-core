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
use labwired_core::cpu::xtensa_lx7::{IRQ_LEVELS, XtensaLx7};
use labwired_core::cpu::xtensa_sr::{
    EXCCAUSE, EPC1, EPC2, EPC3, INTENABLE, INTERRUPT, INTCLEAR, EPS2, EPS3, VECBASE,
};
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
    // op0 = 0xD in byte 0 → narrow (2-byte) instruction.
    // Write NOP.N halfword 0xf03d little-endian: byte[0]=0x3d, byte[1]=0xf0.
    // xtensa_length::instruction_length(0x3d & 0xF = 0xD) == 2 → narrow path.
    // decode_narrow(0xf03d) → Instruction::Nop — executes successfully, PC advances by 2.
    // (D8: narrow decoder fully implemented; NOP.N is the cleanest test case.)
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    // Write NOP.N (0xf03d) at TEST_PC then a BREAK-style unknown instruction to halt.
    bus.write_u8(TEST_PC as u64,     0x3d).unwrap();  // byte0: op0=0xD
    bus.write_u8(TEST_PC as u64 + 1, 0xf0).unwrap();  // byte1: r=0xF, t=0 → NOP.N
    // After NOP.N (2 bytes), put an unimplemented wide instruction to halt the loop.
    // BREAK instruction at TEST_PC+2 (wide, op0=0x0, r=4 → Break{0,0})
    bus.write_u8(TEST_PC as u64 + 2, 0x00).unwrap();
    bus.write_u8(TEST_PC as u64 + 3, 0x40).unwrap();
    bus.write_u8(TEST_PC as u64 + 4, 0x00).unwrap();

    // NOP.N executes fine; then BREAK triggers BreakpointHit at TEST_PC+2.
    cpu.step(&mut bus, &[]).unwrap(); // NOP.N should succeed
    // PC must have advanced by 2 (narrow instruction).
    assert_eq!(
        cpu.get_pc(),
        TEST_PC + 2,
        "after NOP.N, PC should advance by 2 (narrow = 2 bytes)"
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

// ── Interrupt dispatch tests (G3) ────────────────────────────────────────────
//
// Test hook: inject IRQ bits via `cpu.sr.set_raw(INTERRUPT, mask)`, which
// bypasses the write-only guard on INTERRUPT (hardware-latched semantics).
// INTENABLE is written normally via `cpu.sr.write(INTENABLE, mask)`.

/// Set up CPU for IRQ tests: clear reset EXCM/INTLEVEL so interrupts can fire.
fn cpu_ready_for_irq() -> (XtensaLx7, SystemBus) {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    // After reset: PS = 0x1F (EXCM=1, INTLEVEL=0xF). Clear both so IRQs can fire.
    cpu.ps.set_excm(false);
    cpu.ps.set_intlevel(0);
    // Place PC in RAM so step() can fetch (though for IRQ tests, fetch is never reached).
    cpu.set_pc(TEST_PC);
    // Write a NOP.N at TEST_PC so if step() accidentally fetches, it doesn't fault.
    bus.write_u8(TEST_PC as u64,     0x3d).unwrap();
    bus.write_u8(TEST_PC as u64 + 1, 0xf0).unwrap();
    (cpu, bus)
}

#[test]
fn test_irq_level_1_fires_on_step() {
    // Bit 0 is IRQ_LEVELS[0] = 1 (level 1).
    let (mut cpu, mut bus) = cpu_ready_for_irq();
    let old_pc = cpu.get_pc();
    let vecbase = cpu.sr.read(VECBASE);

    cpu.sr.set_raw(INTERRUPT, 1 << 0);
    cpu.sr.write(INTENABLE, 1 << 0);

    // step() should dispatch the interrupt (returns Ok — dispatch is not an error).
    cpu.step(&mut bus, &[]).unwrap();

    // PC must jump to level-1 vector (VECBASE + 0x300 = kernel exception vector).
    assert_eq!(
        cpu.get_pc(),
        vecbase.wrapping_add(0x300),
        "PC must be at L1 interrupt vector (VECBASE+0x300)"
    );
    // EPC1 = old PC (return address for RFWFE/RFE).
    assert_eq!(cpu.sr.read(EPC1), old_pc, "EPC1 must hold pre-dispatch PC");
    // EXCCAUSE = 4 (Level1InterruptCause) so handler can distinguish from sync exception.
    assert_eq!(cpu.sr.read(EXCCAUSE), 4, "EXCCAUSE=4 for level-1 interrupt");
    // PS.EXCM = 1.
    assert!(cpu.ps.excm(), "PS.EXCM=1 after level-1 interrupt entry");
}

#[test]
fn test_irq_level_2_fires() {
    // Bit 19 is IRQ_LEVELS[19] = 2 (level 2).
    assert_eq!(IRQ_LEVELS[19], 2, "sanity: bit 19 is level 2");
    let (mut cpu, mut bus) = cpu_ready_for_irq();
    let old_pc = cpu.get_pc();
    let old_ps = cpu.ps.as_raw();
    let vecbase = cpu.sr.read(VECBASE);

    cpu.sr.set_raw(INTERRUPT, 1 << 19);
    cpu.sr.write(INTENABLE, 1 << 19);

    cpu.step(&mut bus, &[]).unwrap();

    // PC must jump to level-2 vector (VECBASE + 0x180).
    assert_eq!(
        cpu.get_pc(),
        vecbase.wrapping_add(0x180),
        "PC must be at L2 interrupt vector (VECBASE+0x180)"
    );
    // EPC2 = old PC.
    assert_eq!(cpu.sr.read(EPC2), old_pc, "EPC2 must hold pre-dispatch PC");
    // EPS2 = old PS.
    assert_eq!(cpu.sr.read(EPS2), old_ps, "EPS2 must hold pre-dispatch PS");
    // PS.INTLEVEL = 2.
    assert_eq!(cpu.ps.intlevel(), 2, "PS.INTLEVEL=2 after level-2 interrupt entry");
    // PS.EXCM = 1 (level 2 <= XCHAL_EXCM_LEVEL=3, so medium-priority).
    assert!(cpu.ps.excm(), "PS.EXCM=1 for medium-priority level-2 interrupt");
}

#[test]
fn test_irq_blocked_by_intlevel() {
    // Bit 19 is level 2. Set PS.INTLEVEL=3 — level-2 IRQ must NOT fire.
    let (mut cpu, mut bus) = cpu_ready_for_irq();
    cpu.ps.set_intlevel(3);
    let old_pc = cpu.get_pc();

    cpu.sr.set_raw(INTERRUPT, 1 << 19);  // level-2 IRQ
    cpu.sr.write(INTENABLE, 1 << 19);

    // step() must fetch and execute the NOP.N normally (no dispatch).
    cpu.step(&mut bus, &[]).unwrap();

    // PC advanced by 2 (NOP.N = 2 bytes): IRQ was blocked.
    assert_eq!(
        cpu.get_pc(),
        old_pc.wrapping_add(2),
        "IRQ blocked by INTLEVEL: PC must advance normally"
    );
    // EPC2 must not have been written.
    assert_eq!(cpu.sr.read(EPC2), 0, "EPC2 must be zero (no dispatch occurred)");
}

#[test]
fn test_irq_blocked_by_excm() {
    // With PS.EXCM=1, interrupt dispatch must be gated.
    let (mut cpu, mut bus) = cpu_ready_for_irq();
    cpu.ps.set_excm(true);   // re-enable EXCM
    let old_pc = cpu.get_pc();

    cpu.sr.set_raw(INTERRUPT, 1 << 0);  // level-1 IRQ
    cpu.sr.write(INTENABLE, 1 << 0);

    // step() must not dispatch; it fetches and executes NOP.N.
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(
        cpu.get_pc(),
        old_pc.wrapping_add(2),
        "IRQ blocked by EXCM=1: PC must advance normally"
    );
    assert_eq!(cpu.sr.read(EPC1), 0, "EPC1 must be zero (no dispatch occurred)");
}

#[test]
fn test_irq_blocked_when_disabled_in_intenable() {
    // INTERRUPT bit set, but INTENABLE bit clear — must not fire.
    let (mut cpu, mut bus) = cpu_ready_for_irq();
    let old_pc = cpu.get_pc();

    cpu.sr.set_raw(INTERRUPT, 1 << 0);  // level-1 IRQ pending
    cpu.sr.write(INTENABLE, 0);          // but not enabled

    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(
        cpu.get_pc(),
        old_pc.wrapping_add(2),
        "IRQ blocked by INTENABLE=0: PC must advance normally"
    );
    assert_eq!(cpu.sr.read(EPC1), 0, "EPC1 must be zero (no dispatch occurred)");
}

#[test]
fn test_irq_higher_level_preempts_lower() {
    // Bits 19 (level 2) and 22 (level 3) both pending — level 3 must win.
    assert_eq!(IRQ_LEVELS[19], 2, "sanity: bit 19 is level 2");
    assert_eq!(IRQ_LEVELS[22], 3, "sanity: bit 22 is level 3");
    let (mut cpu, mut bus) = cpu_ready_for_irq();
    let old_pc = cpu.get_pc();
    let old_ps = cpu.ps.as_raw();
    let vecbase = cpu.sr.read(VECBASE);

    cpu.sr.set_raw(INTERRUPT, (1 << 19) | (1 << 22));
    cpu.sr.write(INTENABLE, (1 << 19) | (1 << 22));

    cpu.step(&mut bus, &[]).unwrap();

    // Level-3 wins: vector at VECBASE + 0x1C0.
    assert_eq!(
        cpu.get_pc(),
        vecbase.wrapping_add(0x1C0),
        "highest pending IRQ (level 3) must preempt lower (level 2)"
    );
    assert_eq!(cpu.sr.read(EPC3), old_pc, "EPC3 holds pre-dispatch PC");
    assert_eq!(cpu.sr.read(EPS3), old_ps, "EPS3 holds pre-dispatch PS");
    assert_eq!(cpu.ps.intlevel(), 3, "PS.INTLEVEL=3 after level-3 interrupt");
}

#[test]
fn test_intclear_clears_interrupt_bit() {
    // INTCLEAR is write-only; writing a mask clears the corresponding INTERRUPT bits.
    // This verifies the existing C2 SR table wiring.
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();

    // Inject bits 5 and 7 into INTERRUPT via set_raw (hardware path).
    cpu.sr.set_raw(INTERRUPT, (1 << 5) | (1 << 7));
    assert_eq!(
        cpu.sr.read(INTERRUPT),
        (1 << 5) | (1 << 7),
        "INTERRUPT bits should be set after set_raw"
    );

    // Clear bit 5 via INTCLEAR.
    cpu.sr.write(INTCLEAR, 1 << 5);
    assert_eq!(
        cpu.sr.read(INTERRUPT),
        1 << 7,
        "INTCLEAR with bit 5 must clear bit 5 from INTERRUPT (bit 7 remains)"
    );

    // Clear bit 7 too.
    cpu.sr.write(INTCLEAR, 1 << 7);
    assert_eq!(cpu.sr.read(INTERRUPT), 0, "INTERRUPT must be clear after both bits cleared");

    // Unused variable suppression
    let _ = &mut bus;
}
