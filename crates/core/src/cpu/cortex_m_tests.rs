use super::*;
use crate::{DmaRequest, Machine, SimulationConfig};
use std::collections::HashMap;

struct MockBus {
    mem: HashMap<u64, u8>,
    config: SimulationConfig,
}

impl MockBus {
    fn new() -> Self {
        Self {
            mem: HashMap::new(),
            config: SimulationConfig::default(),
        }
    }
}

impl Bus for MockBus {
    fn read_u8(&self, addr: u64) -> SimResult<u8> {
        Ok(*self.mem.get(&addr).unwrap_or(&0))
    }
    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
        self.mem.insert(addr, value);
        Ok(())
    }
    fn tick_peripherals(&mut self) -> Vec<u32> {
        Vec::new()
    }
    fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
        Ok(())
    }
    fn config(&self) -> &SimulationConfig {
        &self.config
    }
}

fn run_test_instr(cpu: &mut CortexM, bus: &mut MockBus, instr_bin: u32, is_32bit: bool) {
    let pc = cpu.pc;
    if is_32bit {
        bus.write_u16(pc as u64, (instr_bin >> 16) as u16).unwrap();
        bus.write_u16((pc + 2) as u64, (instr_bin & 0xFFFF) as u16)
            .unwrap();
    } else {
        bus.write_u16(pc as u64, instr_bin as u16).unwrap();
    }
    cpu.step_internal(bus, &[], &bus.config.clone()).unwrap();
}

/// A 16-bit data-processing instruction inside an IT block must NOT set
/// flags: its `setflags` is `!InITBlock()`.
///
/// Leaking them corrupts the CONDITION of every instruction still to run in
/// the same block, and the failure is invisible — the block simply does
/// less than the compiler intended.
///
/// Measured on real compiler output. `attachInterrupt` compiled to
///
/// ```text
///   cmp   r2, #1        ; Z=1, C=1  → LS true
///   itt   ls
///   orrls r2, r1        ; leaked Z=0 …
///   strls r2, [r3,#…]   ; … so LS was false here and the STORE VANISHED
/// ```
///
/// The register write never happened and the peripheral was never armed.
/// LSL and the arithmetic forms already carried this guard, with a note
/// citing an earlier H563/WBA52 regression; the logical and multiply forms
/// did not.
#[test]
fn armv7m_sixteen_bit_dp_does_not_set_flags_inside_an_it_block() {
    // ORR, the exact instruction that was measured, plus its siblings.
    // Each is `<op> r2, r1` in its 16-bit T1 encoding.
    for (name, encoding) in [
        ("orr", 0x430Au16),
        ("and", 0x400Au16),
        ("eor", 0x404Au16),
        ("bic", 0x438Au16),
        ("mul", 0x434Au16),
    ] {
        let mut bus = MockBus::new();
        let mut cpu = CortexM::new();
        cpu.pc = 0x1000;

        // Set Z=1 and C=1, the flags `cmp r2, #1` leaves when r2 == 1.
        cpu.write_reg(1, 1);
        cpu.write_reg(2, 1);
        run_test_instr(&mut cpu, &mut bus, 0x2A01, false); // cmp r2, #1
        assert!(((cpu.xpsr >> 30) & 1 == 1), "{name}: setup expects Z set");
        let carry_before = cpu.get_carry();

        // `itt ls` then the instruction under test.
        run_test_instr(&mut cpu, &mut bus, 0xBF9C, false);
        assert_ne!(cpu.it_state, 0, "{name}: IT block did not open");
        run_test_instr(&mut cpu, &mut bus, encoding as u32, false);

        assert!(
            ((cpu.xpsr >> 30) & 1 == 1),
            "{name} inside an IT block cleared Z — the next conditional \
                 instruction in the block would be skipped"
        );
        assert_eq!(
            cpu.get_carry(),
            carry_before,
            "{name} inside an IT block moved C"
        );
    }
}

/// ...and OUTSIDE an IT block the same encoding DOES set them. Without
/// this the guard could be a blanket "never set flags", which would break
/// every ordinary `orrs`.
#[test]
fn armv7m_sixteen_bit_dp_still_sets_flags_outside_an_it_block() {
    let mut bus = MockBus::new();
    let mut cpu = CortexM::new();
    cpu.pc = 0x1000;

    cpu.write_reg(1, 0);
    cpu.write_reg(2, 1);
    run_test_instr(&mut cpu, &mut bus, 0x2A01, false); // cmp r2, #1 → Z=1
    assert!((cpu.xpsr >> 30) & 1 == 1);

    // `ands r2, r1` with r1 = 0 → result 0 … Z stays set. Use ORR with a
    // non-zero result instead, which must CLEAR Z.
    cpu.write_reg(1, 4);
    run_test_instr(&mut cpu, &mut bus, 0x430A, false); // orrs r2, r1
    assert_eq!(cpu.read_reg(2), 5);
    assert!(
        (cpu.xpsr >> 30) & 1 != 1,
        "orrs outside an IT block must update Z"
    );
}

/// `MOV PC, Rm` is a branch (BXWritePC), not a register write.
///
/// rustc lowers a dense byte `match` to `ADR`/`ADD Rd, Rn, idx LSL #2`/
/// `MOV PC, Rd` over a table of 4-byte `B.W` entries. Advancing PC after
/// the write lands 2 bytes into the selected entry, so its second halfword
/// runs as a stray instruction and control falls through to the NEXT
/// entry — every match arm silently executes its successor's.
#[test]
fn armv7m_mov_to_pc_branches_instead_of_advancing() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x451A;
    cpu.r5 = 0x45B0; // a 4-aligned jump-table entry, as ADD LSL #2 produces
    run_test_instr(&mut cpu, &mut bus, 0x46AF, false); // MOV pc, r5
    assert_eq!(cpu.pc, 0x45B0, "MOV PC, Rm must branch to Rm exactly");

    // Thumb bit is the instruction-set selector, never part of the address.
    cpu.pc = 0x451A;
    cpu.r5 = 0x45B1;
    run_test_instr(&mut cpu, &mut bus, 0x46AF, false);
    assert_eq!(cpu.pc, 0x45B0, "MOV PC, Rm must clear the Thumb bit");
}

/// A non-PC destination keeps plain register-move semantics.
#[test]
fn armv7m_mov_reg_still_moves_and_advances_for_normal_registers() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x451A;
    cpu.r5 = 0x45B1;
    run_test_instr(&mut cpu, &mut bus, 0x462F, false); // MOV r7, r5
    assert_eq!(cpu.r7, 0x45B1);
    assert_eq!(cpu.pc, 0x451C, "a normal MOV advances one halfword");
}

#[test]
fn armv7m_ldrexb_strexb_supports_atomic_bool_compare_exchange() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x2000;
    cpu.r0 = 0x3000;
    cpu.r1 = 0xAA;
    bus.write_u8(0x3000, 0).unwrap();

    // LDREXB r1, [r0] and STREXB r2, r1, [r0], emitted by Rust's
    // AtomicBool::compare_exchange on thumbv7em-none-eabi.
    run_test_instr(&mut cpu, &mut bus, 0xE8D01F4F, true);
    assert_eq!(cpu.r1, 0, "LDREXB loads exactly one byte");
    cpu.r1 = 1;
    run_test_instr(&mut cpu, &mut bus, 0xE8C01F42, true);
    assert_eq!(bus.read_u8(0x3000).unwrap(), 1, "STREXB stores one byte");
    assert_eq!(cpu.r2, 0, "uncontended exclusive store succeeds");
}

#[test]
fn armv7m_strexb_fails_without_matching_unchanged_reservation() {
    for case in ["none", "address", "write"] {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r0 = 0x3000;
        cpu.r1 = 1;
        bus.write_u8(0x3000, 0).unwrap();
        if case != "none" {
            run_test_instr(&mut cpu, &mut bus, 0xE8D03F4F, true); // ldrexb r3,[r0]
        }
        if case == "address" {
            cpu.r0 = 0x3001;
        } else if case == "write" {
            bus.write_u8(0x3000, 7).unwrap();
        }
        run_test_instr(&mut cpu, &mut bus, 0xE8C01F42, true);
        assert_eq!(cpu.r2, 1, "{case} invalidates exclusive store");
    }
}

#[test]
fn exception_entry_clears_byte_exclusive_reservation() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.sp = 0x8000;
    cpu.exclusive_byte = Some((0x3000, 0));
    bus.write_u16(0x1000, 0xBF00).unwrap();
    bus.write_u32(16 * 4, 0x5001).unwrap();
    cpu.set_exception_pending(16);
    let cfg = bus.config.clone();
    cpu.step_internal(&mut bus, &[], &cfg).unwrap();
    assert_eq!(cpu.exclusive_byte, None);
}

#[test]
fn cortex_m4_qadd16_and_usat_encode_clamped_coolant_byte() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x2000;
    cpu.r0 = 40;
    cpu.r1 = 90;

    // Emitted for i16::saturating_add(40).clamp(0, 255) as u8.
    run_test_instr(&mut cpu, &mut bus, 0xFA91F010, true); // qadd16 r0,r1,r0
    assert_eq!(cpu.r0 & 0xffff, 130);
    run_test_instr(&mut cpu, &mut bus, 0xF3800008, true); // usat r0,#8,r0
    assert_eq!(cpu.r0, 130);
}

#[test]
fn cps_faultmask_set_clear_and_mask() {
    // CPSID f (0xB671) sets FAULTMASK; CPSIE f (0xB661) clears it. Zephyr's
    // fault handler toggles FAULTMASK; an unmodelled CPS-f decoded as Unknown
    // and the fault path ("ESF could not be retrieved") failed.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    // Sequential PCs so the decode cache doesn't reuse the first opcode.
    cpu.pc = 0x1000;
    run_test_instr(&mut cpu, &mut bus, 0xB671, false); // CPSID f @0x1000
    assert!(cpu.faultmask, "CPSID f sets FAULTMASK");
    run_test_instr(&mut cpu, &mut bus, 0xB661, false); // CPSIE f @0x1002
    assert!(!cpu.faultmask, "CPSIE f clears FAULTMASK");

    // FAULTMASK masks a normal IRQ but NOT NMI (exception 2).
    cpu.pc = 0x2000;
    cpu.sp = 0x2000_0040;
    cpu.faultmask = true;
    bus.write_u16(0x2000, 0xBF00).unwrap(); // NOP
    bus.write_u32(0x40, 0x0000_5000 | 1).unwrap(); // exc 16 vector
    cpu.set_exception_pending(16);
    let cfg = bus.config.clone();
    cpu.step_internal(&mut bus, &[], &cfg).unwrap();
    assert_eq!(cpu.pc, 0x2002, "FAULTMASK must mask the IRQ; the NOP runs");

    // NMI (exception 2) still preempts under FAULTMASK.
    cpu.pc = 0x3000;
    bus.write_u32(0x08, 0x0000_6000 | 1).unwrap(); // exc 2 (NMI) vector
    cpu.set_exception_pending(2);
    cpu.step_internal(&mut bus, &[], &cfg).unwrap();
    assert_eq!(cpu.pc, 0x6000, "NMI is never masked by FAULTMASK");
}

#[test]
fn basepri_masks_equal_or_lower_priority_exceptions() {
    // A non-zero BASEPRI masks any exception whose priority value is >=
    // BASEPRI. Zephyr raises BASEPRI to guard scheduler critical sections; an
    // unmodelled BASEPRI let the timer IRQ fire inside them and corrupt state.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.sp = 0x2000_0040;
    bus.write_u16(0x1000, 0xBF00).unwrap(); // NOP at 0x1000
                                            // Exception 16 (IRQ 0). With no NVIC wired, its priority reads 0xFF.
    let handler = 0x0000_5000u32;
    bus.write_u32(0x40, handler | 1).unwrap(); // VTOR=0 → vector[16] at 0x40
    cpu.set_exception_pending(16);

    let cfg = bus.config.clone();
    // BASEPRI=0x80 masks priority 0xFF (>= 0x80): the NOP runs, no vectoring.
    cpu.basepri = 0x80;
    cpu.step_internal(&mut bus, &[], &cfg).unwrap();
    assert_eq!(
        cpu.pc, 0x1002,
        "BASEPRI must mask exc 16; the NOP should run"
    );

    // Clearing BASEPRI lets the still-pending exception through.
    cpu.basepri = 0;
    cpu.step_internal(&mut bus, &[], &cfg).unwrap();
    assert_eq!(
        cpu.pc,
        handler & !1,
        "with BASEPRI=0 the pending exception is taken"
    );
}

#[test]
fn mrs_ipsr_reads_active_exception() {
    // `mrs Rd, IPSR` (sysm = 5) must return the current exception number,
    // not 0. Zephyr's _isr_wrapper computes the IRQ line as `IPSR - 16` to
    // index the software ISR table; an IPSR of 0 made the index -16, so it
    // `blx`-ed a garbage handler and executed rodata as code. Bare-metal and
    // FreeRTOS firmware never hit this because they don't dispatch ISRs by
    // reading IPSR.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.active_exception = 33; // e.g. an IRQ exception (16 + IRQ 17)

    // mrs r3, IPSR = 0xF3EF 8305
    run_test_instr(&mut cpu, &mut bus, 0xF3EF8305, true);

    assert_eq!(cpu.r3, 33, "MRS IPSR must read the active exception number");
}

#[test]
fn ldr_to_pc_register_offset_branches_to_target() {
    // `ldr.w pc, [r3, r0, lsl #2]` = 0xF853 0xF020 is GCC's switch
    // jump-table idiom. It must branch to the loaded value, not loaded+4:
    // the load-to-PC path was leaving pc_increment at 4, so PC landed one
    // instruction past the real target. That corrupted control flow into
    // Zephyr's onoff state machine (process_event's EVT_START dispatch),
    // tripping `__ASSERT(state == ONOFF_STATE_OFF)` and hanging boot.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.r3 = 0x2000; // jump-table base
    cpu.r0 = 2; // case index
                // [0x2000 + (2 << 2)] = [0x2008] holds the (thumb) target 0x5001.
    bus.write_u32(0x2008, 0x5001).unwrap();

    run_test_instr(&mut cpu, &mut bus, 0xF853F020, true);

    assert_eq!(
        cpu.pc, 0x5000,
        "ldr.w pc,[rn,rm,lsl#n] must branch to the loaded target (thumb bit \
             cleared), not target+4"
    );
}

#[test]
fn test_arm_dataproc_complex() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;

    // Test CLZ
    cpu.r1 = 0x0000FFFF;
    // Instruction::Clz { rd: 0, rm: 1 }
    // Encoding for CLZ R0, R1 is 0xFAB1 F081
    run_test_instr(&mut cpu, &mut bus, 0xFAB1F081, true);
    assert_eq!(cpu.r0, 16);

    // Test RBIT
    cpu.r1 = 0x00000001;
    // RBIT R0, R1 is 0xFA91 F0A1
    run_test_instr(&mut cpu, &mut bus, 0xFA91F0A1, true);
    assert_eq!(cpu.r0, 0x80000000);

    // Test UDIV
    cpu.r1 = 100;
    cpu.r2 = 10;
    // UDIV R0, R1, R2 is 0xFBB1 F0F2
    run_test_instr(&mut cpu, &mut bus, 0xFBB1F0F2, true);
    assert_eq!(cpu.r0, 10);
}

#[test]
fn test_svc_pends_and_takes_svcall_exception() {
    // Zephyr's fatal path, irq_offload, and userspace syscalls all execute
    // `svc`. Without taking the SVCall exception the PC sticks on the
    // instruction forever (ztest hangs). Executing SVC must pend SVCall
    // (exception 11) and the next step must vector to its handler with a
    // standard 8-word exception frame.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.sp = 0x2000_0040;

    // VTOR defaults to 0, so the SVCall vector (exc 11) is at 11*4 = 0x2C.
    let handler = 0x0000_5000u32;
    bus.write_u32(0x2C, handler | 1).unwrap(); // thumb bit set

    // SVC #2 at 0x1000.
    bus.write_u16(0x1000, 0xDF02).unwrap();

    let cfg = bus.config.clone();
    // 1st step executes SVC: pends SVCall, advances PC past the 16-bit instr.
    cpu.step_internal(&mut bus, &[], &cfg).unwrap();
    assert_eq!(cpu.pc, 0x1002, "SVC should advance PC past the instruction");

    // 2nd step takes the exception: vector to the handler, stack the frame.
    cpu.step_internal(&mut bus, &[], &cfg).unwrap();
    assert_eq!(cpu.pc, handler & !1, "should vector to the SVCall handler");
    assert_eq!(
        cpu.sp,
        0x2000_0040 - 32,
        "should push an 8-word exception frame"
    );
    assert_eq!(cpu.active_exception, 11, "SVCall is exception 11");
    // Stacked return address (frame + 24) is the instruction after the SVC.
    assert_eq!(bus.read_u32(cpu.sp as u64 + 24).unwrap(), 0x1002);
}

#[test]
fn test_strd_predec_writeback() {
    // e96d ce04 → strd ip, lr, [sp, #-16]!  (P=1, U=0, W=1).
    // libgcc __aeabi_uldivmod prologue used by the mbedTLS bignum/RSA
    // path: it stores ip,lr below SP and updates SP. Ignoring writeback
    // left SP stale so the matching `ldr lr,[sp,#4]` read a garbage
    // return address and the RSA verify wild-jumped.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.sp = 0x2000_0040;
    cpu.r12 = 0xAABB_CCDD;
    cpu.lr = 0x0800_5755;

    run_test_instr(&mut cpu, &mut bus, 0xE96DCE04, true);

    // SP updated to SP-16.
    assert_eq!(cpu.sp, 0x2000_0030);
    // Doubleword stored at the new SP.
    assert_eq!(bus.read_u32(0x2000_0030).unwrap(), 0xAABB_CCDD);
    assert_eq!(bus.read_u32(0x2000_0034).unwrap(), 0x0800_5755);
}

#[test]
fn test_ldrd_postindex_writeback() {
    // e8f1 2304 → ldrd r2, r3, [r1], #16  (P=0, U=1, W=1).
    // Post-indexed: load from [r1], then r1 += 16.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.set_register(1, 0x2000_0080);
    bus.write_u32(0x2000_0080, 0x1122_3344).unwrap();
    bus.write_u32(0x2000_0084, 0x5566_7788).unwrap();

    run_test_instr(&mut cpu, &mut bus, 0xE8F12304, true);

    assert_eq!(cpu.get_register(2), 0x1122_3344);
    assert_eq!(cpu.get_register(3), 0x5566_7788);
    // Base updated by +16 after the access.
    assert_eq!(cpu.get_register(1), 0x2000_0090);
}

#[test]
fn test_ldrd_offset_no_writeback() {
    // e9d1 0702 → ldrd r0, r7, [r1, #8]  (P=1, U=1, W=0): base unchanged.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.set_register(1, 0x2000_0100);
    bus.write_u32(0x2000_0108, 0xDEAD_BEEF).unwrap();
    bus.write_u32(0x2000_010C, 0xFEED_FACE).unwrap();

    run_test_instr(&mut cpu, &mut bus, 0xE9D10702, true);

    assert_eq!(cpu.get_register(0), 0xDEAD_BEEF);
    assert_eq!(cpu.get_register(7), 0xFEED_FACE);
    // Offset form: base register must be unchanged.
    assert_eq!(cpu.get_register(1), 0x2000_0100);
}

// Helpers for flag inspection in arithmetic tests.
const C_BIT: u32 = 1 << 29;
const V_BIT: u32 = 1 << 28;
const Z_BIT: u32 = 1 << 30;
const N_BIT: u32 = 1 << 31;

#[test]
fn test_dataproc32_adc_carry_in() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;

    // ADCS R3, R4, R5 with carry-in set: 1 + 1 + 1 = 3
    cpu.r4 = 1;
    cpu.r5 = 1;
    cpu.xpsr |= C_BIT; // carry-in = 1
    run_test_instr(&mut cpu, &mut bus, 0xEB540305, true);
    assert_eq!(cpu.r3, 3);
    assert_eq!(cpu.xpsr & C_BIT, 0); // no carry-out
}

#[test]
fn test_dataproc32_add_sets_carry_and_overflow() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;

    // ADDS R2, R0, R1 : 0xFFFF_FFFF + 1 = 0, carry set, zero set
    cpu.r0 = 0xFFFF_FFFF;
    cpu.r1 = 1;
    run_test_instr(&mut cpu, &mut bus, 0xEB100201, true);
    assert_eq!(cpu.r2, 0);
    assert_ne!(cpu.xpsr & C_BIT, 0);
    assert_ne!(cpu.xpsr & Z_BIT, 0);
    assert_eq!(cpu.xpsr & V_BIT, 0);

    // ADDS overflow: 0x7FFF_FFFF + 1 = 0x8000_0000, V set, N set, C clear
    cpu.pc = 0x1000;
    cpu.r0 = 0x7FFF_FFFF;
    cpu.r1 = 1;
    run_test_instr(&mut cpu, &mut bus, 0xEB100201, true);
    assert_eq!(cpu.r2, 0x8000_0000);
    assert_ne!(cpu.xpsr & V_BIT, 0);
    assert_ne!(cpu.xpsr & N_BIT, 0);
    assert_eq!(cpu.xpsr & C_BIT, 0);
}

#[test]
fn test_simd_uadd8_sel_strlen_kernel() {
    // Reproduces newlib's optimised strlen inner loop, which was silently
    // NOP'd before UADD8/SEL were modelled (C strings measured as garbage).
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;

    // Word bytes (LE lanes): [0x41 'A', 0x42 'B', 0x43 'C', 0x00 terminator]
    cpu.r2 = 0x0043_4241;
    cpu.r12 = 0xFFFF_FFFF; // ip = ~0
    cpu.r4 = 0;

    // UADD8 r2, r2, ip : GE[i] = (byte + 0xFF >= 0x100) = (byte != 0)
    run_test_instr(&mut cpu, &mut bus, 0xFA82_F24C, true);
    assert_eq!(cpu.get_ge(), 0b0111, "GE must flag the three nonzero bytes");

    // SEL r2, r4, ip : GE-set lanes take r4 (0x00), clear lanes take ip (0xFF).
    // Placed at the next PC (0x1004) so we don't overwrite a prefetched word.
    assert_eq!(cpu.pc, 0x1004);
    run_test_instr(&mut cpu, &mut bus, 0xFAA4_F28C, true);
    assert_eq!(
        cpu.r2, 0xFF00_0000,
        "only the null lane (byte 3) becomes 0xFF"
    );
}

#[test]
fn test_simd_usub8_ssub8_ge() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    // USUB8 r1, r2, r3 : GE[i] = (Rn.byte >= Rm.byte), result = wrapping diff
    cpu.r2 = 0x10_05_80_00;
    cpu.r3 = 0x08_05_7F_01;
    run_test_instr(&mut cpu, &mut bus, 0xFAC2_F143, true);
    // lanes: 0x00-0x01=0xFF(borrow,GE0), 0x80-0x7F=0x01(GE1), 0x05-0x05=0(GE1), 0x10-0x08=0x08(GE1)
    assert_eq!(cpu.r1, 0x08_00_01_FF);
    assert_eq!(cpu.get_ge(), 0b1110);
}

// The exact instruction LLVM emits for `u16::saturating_add` on thumbv7em,
// driven with the operands the ILI9341 lab firmware actually had in flight.
//
// Undecoded, this was a 4-byte skip that left Rd holding a stale operand, so
// `x.saturating_add(w - 1)` silently evaluated to `w - 1`: the firmware asked
// an ILI9341 for a window ending at column `w-1` instead of `x+w-1` and
// painted one row of a fourteen-row band. Nothing faulted.
#[test]
fn test_uqadd16_is_a_real_saturating_add_not_a_skip() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;

    // UQADD16 r0, r2, r0  (0xFA92 F050) — the encoding from the lab ELF.
    // r2 = x = 48 (row origin), r0 = h - 1 = 13.
    cpu.r2 = 48;
    cpu.r0 = 13;
    run_test_instr(&mut cpu, &mut bus, 0xFA92_F050, true);
    assert_eq!(
        cpu.r0, 61,
        "the window's last row is origin + height - 1, not height - 1"
    );

    // Saturation is per lane and clamps at 0xFFFF; the upper halfword is an
    // independent lane, never a carry target for the lower one.
    cpu.pc = 0x1004;
    cpu.r2 = 0x0001_FF00;
    cpu.r0 = 0x0002_0200;
    run_test_instr(&mut cpu, &mut bus, 0xFA92_F050, true);
    assert_eq!(
        cpu.r0, 0x0003_FFFF,
        "low lane saturates at 0xFFFF without carrying into the high lane"
    );
}

#[test]
fn test_parallel_halfword_add_sub_variants() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;

    // UQSUB16 r0, r2, r1 (0xFAD2 F051): unsigned saturating, floors at 0.
    cpu.r2 = 0x0005_0010;
    cpu.r1 = 0x0009_0003;
    run_test_instr(&mut cpu, &mut bus, 0xFAD2_F051, true);
    assert_eq!(cpu.r0, 0x0000_000D, "5-9 floors at 0; 0x10-3 = 0x0D");

    // UADD16 r0, r2, r1 (0xFA92 F041): wrapping, and GE carries per lane.
    cpu.pc = 0x1004;
    cpu.r2 = 0xFFFF_0001;
    cpu.r1 = 0x0001_0002;
    run_test_instr(&mut cpu, &mut bus, 0xFA92_F041, true);
    assert_eq!(
        cpu.r0, 0x0000_0003,
        "upper lane wraps rather than saturating"
    );
    assert_eq!(cpu.get_ge(), 0b1100, "only the wrapping lane carried out");

    // QADD16 r0, r2, r1 (0xFA92 F011): SIGNED saturation, clamps at i16::MAX.
    cpu.pc = 0x1008;
    cpu.r2 = 0x0000_7FFF;
    cpu.r1 = 0x0000_0001;
    run_test_instr(&mut cpu, &mut bus, 0xFA92_F011, true);
    assert_eq!(cpu.r0, 0x0000_7FFF, "signed saturation stops at 0x7FFF");

    // SSUB16 r0, r2, r1 (0xFAD2 F001): signed wrapping; GE = lane >= 0.
    cpu.pc = 0x100C;
    cpu.r2 = 0x0005_0001;
    cpu.r1 = 0x0002_0004;
    run_test_instr(&mut cpu, &mut bus, 0xFAD2_F001, true);
    assert_eq!(cpu.r0, 0x0003_FFFD, "1-4 = -3 in the low lane");
    assert_eq!(cpu.get_ge(), 0b1100, "only the non-negative lane sets GE");
}

#[test]
fn test_dataproc32_multiword_carry_chain() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;

    // 64-bit add: (R1:R0) + (R4:R3) where low words overflow.
    // low: ADDS R2, R0, R1 -> R0=0xFFFF_FFFF + R1=1 -> 0, carry=1
    cpu.r0 = 0xFFFF_FFFF;
    cpu.r1 = 0x0000_0001;
    run_test_instr(&mut cpu, &mut bus, 0xEB100201, true); // ADDS R2,R0,R1
    assert_eq!(cpu.r2, 0);
    assert_ne!(cpu.xpsr & C_BIT, 0);

    // high: ADCS R5, R3, R4 -> 0x10 + 0x20 + carry(1) = 0x31
    cpu.r3 = 0x10;
    cpu.r4 = 0x20;
    run_test_instr(&mut cpu, &mut bus, 0xEB530504, true); // ADCS R5,R3,R4
    assert_eq!(cpu.r5, 0x31);
    // Full result: high=0x31, low=0 -> 0x0000_0031_0000_0000
}

#[test]
fn test_dataproc32_sbc_borrow() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;

    // SBCS R2, R0, R1 with carry-in clear (borrow): 5 - 3 - 1 = 1
    cpu.r0 = 5;
    cpu.r1 = 3;
    cpu.xpsr &= !C_BIT; // carry-in 0 => borrow 1
    run_test_instr(&mut cpu, &mut bus, 0xEB700201, true);
    assert_eq!(cpu.r2, 1);
    assert_ne!(cpu.xpsr & C_BIT, 0); // no final borrow -> C set

    // SBCS producing a borrow: 0 - 1 - 0 = 0xFFFF_FFFE, C clear, N set
    cpu.pc = 0x1000;
    cpu.r0 = 0;
    cpu.r1 = 1;
    cpu.xpsr |= C_BIT; // carry-in 1 => borrow 0
    run_test_instr(&mut cpu, &mut bus, 0xEB700201, true);
    assert_eq!(cpu.r2, 0xFFFF_FFFF);
    assert_eq!(cpu.xpsr & C_BIT, 0); // borrow -> C clear
    assert_ne!(cpu.xpsr & N_BIT, 0);
}

#[test]
fn test_dataproc32_rsb() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;

    // RSB R2, R0, R1 -> R2 = R1 - R0 = 30 - 10 = 20 (no flags)
    cpu.r0 = 10;
    cpu.r1 = 30;
    run_test_instr(&mut cpu, &mut bus, 0xEBC00201, true);
    assert_eq!(cpu.r2, 20);

    // RSBS R2, R0, R1 -> R2 = 0 - 5 = 0xFFFF_FFFB, flags set, C clear (borrow)
    // (pc advances naturally; the decode cache keys on pc, so we must not
    // re-use an address for a different opcode.)
    cpu.r0 = 5;
    cpu.r1 = 0;
    run_test_instr(&mut cpu, &mut bus, 0xEBD00201, true);
    assert_eq!(cpu.r2, 0xFFFF_FFFB);
    assert_ne!(cpu.xpsr & N_BIT, 0);
    assert_eq!(cpu.xpsr & C_BIT, 0);
}

#[test]
fn test_dataproc32_pkh() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;

    // PKHBT R2, R0, R1 : low half from R0, high half from R1
    cpu.r0 = 0xAAAA_BBBB;
    cpu.r1 = 0xCCCC_DDDD;
    run_test_instr(&mut cpu, &mut bus, 0xEAC00201, true);
    assert_eq!(cpu.r2, 0xCCCC_BBBB);

    // PKHTB R2, R0, R1 : high half from R0, low half from R1.
    // pc advances naturally (decode cache keys on pc, not on opcode bytes).
    cpu.r0 = 0xAAAA_BBBB;
    cpu.r1 = 0xCCCC_DDDD;
    run_test_instr(&mut cpu, &mut bus, 0xEAC00221, true);
    assert_eq!(cpu.r2, 0xAAAA_DDDD);
}

#[test]
fn test_dataproc32_umaal() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;

    // UMAAL R0, R1, R2, R3 : (R1:R0) = R2*R3 + R0 + R1
    // R2=0xFFFF_FFFF, R3=0xFFFF_FFFF -> 0xFFFF_FFFE_0000_0001
    // + R0(0x10) + R1(0x20) -> 0xFFFF_FFFE_0000_0031
    cpu.r2 = 0xFFFF_FFFF;
    cpu.r3 = 0xFFFF_FFFF;
    cpu.r0 = 0x10;
    cpu.r1 = 0x20;
    run_test_instr(&mut cpu, &mut bus, 0xFBE20163, true);
    let result = ((cpu.r1 as u64) << 32) | (cpu.r0 as u64);
    assert_eq!(result, 0xFFFF_FFFE_0000_0031);
}

#[test]
fn test_arm_bitfield() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x2000;

    // BFI R1, R0, 4, 8
    cpu.r0 = 0x000000FF;
    cpu.r1 = 0x00000000;
    // BFI R1, R0, 4, 8 is 0xF360 110B
    run_test_instr(&mut cpu, &mut bus, 0xF360110B, true);
    assert_eq!(cpu.r1, 0x00000FF0);

    // BFC R1, 4, 4
    cpu.r1 = 0xFFFFFFFF;
    // BFC R1, 4, 4 is 0xF36F 1107
    run_test_instr(&mut cpu, &mut bus, 0xF36F1107, true);
    assert_eq!(cpu.r1, 0xFFFFFF0F);

    // UBFX R1, R0, 4, 4
    cpu.r0 = 0x000000F0;
    // UBFX R1, R0, 4, 4 is 0xF3C0 1103
    run_test_instr(&mut cpu, &mut bus, 0xF3C01103, true);
    assert_eq!(cpu.r1, 0x0000000F);
}

#[test]
fn test_arm_dataproc_imm() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x3000;

    // ADC R0, R1, #0
    cpu.r1 = 10;
    cpu.xpsr |= 1 << 29; // Set Carry
                         // ADC.W R0, R1, #0 is 0xF141 0000
    run_test_instr(&mut cpu, &mut bus, 0xF1410000, true);
    assert_eq!(cpu.r0, 11);

    // SBC R0, R1, #0
    cpu.r1 = 10;
    cpu.xpsr &= !(1 << 29); // Clear Carry (Borrow)
                            // SBC.W R0, R1, #0 is 0xF161 0000
    run_test_instr(&mut cpu, &mut bus, 0xF1610000, true);
    assert_eq!(cpu.r0, 9);
}

#[test]
fn test_arm_ldrd_strd() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x4000;
    cpu.r2 = 0x5000;

    // STRD R0, R1, [R2, #8]
    cpu.r0 = 0x11111111;
    cpu.r1 = 0x22222222;
    // STRD R0, R1, [R2, #8] is 0xE9C2 0102
    run_test_instr(&mut cpu, &mut bus, 0xE9C20102, true);
    assert_eq!(bus.read_u32(0x5008).unwrap(), 0x11111111);
    assert_eq!(bus.read_u32(0x500C).unwrap(), 0x22222222);

    // LDRD R3, R4, [R2, #8]
    // LDRD R3, R4, [R2, #8] is 0xE9D2 3402
    run_test_instr(&mut cpu, &mut bus, 0xE9D23402, true);
    assert_eq!(cpu.r3, 0x11111111);
    assert_eq!(cpu.r4, 0x22222222);
}

#[test]
fn test_arm_ldrd_negative_offset() {
    // Regression: LDRD T1 with U=0 must subtract imm8*4 from the base.
    // `ldrd r0, r7, [r1, #-32]` = E951 0708 — mbedTLS AES round-key load.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x4000;
    cpu.r1 = 0x5020; // base = 0x5020; addr = 0x5020 - 32 = 0x5000
    bus.write_u32(0x5000, 0xDEAD_BEEF).unwrap();
    bus.write_u32(0x5004, 0xCAFE_BABE).unwrap();
    // E951 0708: ldrd r0, r7, [r1, #-32] (U=0, imm8=8 → offset=32)
    run_test_instr(&mut cpu, &mut bus, 0xE9510708, true);
    assert_eq!(cpu.r0, 0xDEAD_BEEF);
    assert_eq!(cpu.r7, 0xCAFE_BABE);
}

#[test]
fn test_ldr_t4_post_index_pc_function_return() {
    // Regression: `ldr.w pc, [sp], #4` = F85D FB04 (T4 post-index, U=1,
    // W=1) is the clang function-return idiom. Previously decoded to
    // Unknown32 and silently skipped, so the return branched nowhere and
    // execution fell through to a wrong address. Verify it loads PC from
    // [sp] and post-increments sp by 4.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x4000;
    cpu.sp = 0x6000;
    bus.write_u32(0x6000, 0x0000_1235).unwrap(); // return addr (thumb bit set)
    run_test_instr(&mut cpu, &mut bus, 0xF85DFB04, true);
    assert_eq!(
        cpu.pc, 0x0000_1234,
        "PC must come from [sp] (thumb bit cleared)"
    );
    assert_eq!(cpu.sp, 0x6004, "post-index writeback: sp += 4");
}

#[test]
fn test_ldr_t4_pre_index_writeback() {
    // `ldr.w r3, [r1, #8]!` = F851 3F08 (T4 pre-index, U=1, W=1).
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x4000;
    cpu.r1 = 0x5000;
    bus.write_u32(0x5008, 0xABCD_1234).unwrap();
    run_test_instr(&mut cpu, &mut bus, 0xF8513F08, true);
    assert_eq!(cpu.r3, 0xABCD_1234, "loaded from r1+8");
    assert_eq!(cpu.r1, 0x5008, "pre-index writeback: r1 = r1+8");
}

#[test]
fn test_str_t4_pre_decrement_writeback() {
    // `str.w r2, [r1, #-4]!` = F841 2D04 (T4 pre-index, U=0, W=1).
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x4000;
    cpu.r1 = 0x5008;
    cpu.r2 = 0xDEAD_BEEF;
    run_test_instr(&mut cpu, &mut bus, 0xF8412D04, true);
    assert_eq!(bus.read_u32(0x5004).unwrap(), 0xDEAD_BEEF, "stored at r1-4");
    assert_eq!(cpu.r1, 0x5004, "pre-index writeback: r1 = r1-4");
}

#[test]
fn test_thumb2_stmia_ldmdb_wide_addressing() {
    // Regression: the 0xE8xx/0xE9xx LDM/STM group was decoded as STM=>DB,
    // LDM=>IA unconditionally, so STMIA.W (the compiler's struct-copy idiom)
    // stored *below* the base instead of at it. Verify both addressing modes.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x4000;
    cpu.r2 = 0x5000;
    cpu.r0 = 0xAAAA_AAAA;
    cpu.r1 = 0xBBBB_BBBB;

    // STMIA.W r2, {r0, r1}  (no writeback) = 0xE882 0003 — stores at the base.
    run_test_instr(&mut cpu, &mut bus, 0xE882_0003, true);
    assert_eq!(bus.read_u32(0x5000).unwrap(), 0xAAAA_AAAA);
    assert_eq!(bus.read_u32(0x5004).unwrap(), 0xBBBB_BBBB);
    assert_eq!(cpu.r2, 0x5000, "no writeback leaves Rn unchanged");

    // STMIA.W r2!, {r0, r1} (writeback) = 0xE8A2 0003 — advances Rn by 8.
    cpu.r2 = 0x6000;
    run_test_instr(&mut cpu, &mut bus, 0xE8A2_0003, true);
    assert_eq!(bus.read_u32(0x6000).unwrap(), 0xAAAA_AAAA);
    assert_eq!(cpu.r2, 0x6008, "writeback advances Rn");

    // LDMDB.W r2, {r3, r4} = 0xE912 0018 — loads from below the base.
    cpu.r2 = 0x5008;
    run_test_instr(&mut cpu, &mut bus, 0xE912_0018, true);
    assert_eq!(cpu.r3, 0xAAAA_AAAA);
    assert_eq!(cpu.r4, 0xBBBB_BBBB);
    assert_eq!(cpu.r2, 0x5008, "no writeback leaves Rn unchanged");
}

#[test]
fn test_thumb2_pld_is_nop_not_pc_load() {
    // Regression: PLD/PLI/PLDW (preload memory hints) are encoded as
    // byte/halfword "loads" with Rt==15. The 0xF800 LDR/STR handler wrote the
    // loaded value into Rt=15 (PC), so `pld [r0]` — newlib's PLD-optimized
    // strlen/memchr idiom — loaded a byte from [r0] into PC and the CPU jumped
    // to a garbage (flash-alias) address, looping forever. Hints must be NOPs;
    // only a WORD load (op1&7==5) with Rt==15 is a real LDR.W PC branch.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x4000;
    cpu.r0 = 0x5000;
    // A value that would become a bogus PC if the hint were mishandled.
    bus.write_u32(0x5000, 0x0000_0048).unwrap();

    // PLD [r0, #0] = 0xF890 0xF000 (byte-load form, Rt=15).
    run_test_instr(&mut cpu, &mut bus, 0xF890_F000, true);
    assert_eq!(
        cpu.pc, 0x4004,
        "PLD must be a NOP (PC+=4), not a load into PC"
    );

    // PLDW/halfword preload hint [r0] = 0xF8B0 0xF000 (halfword form, Rt=15).
    cpu.pc = 0x4000;
    run_test_instr(&mut cpu, &mut bus, 0xF8B0_F000, true);
    assert_eq!(cpu.pc, 0x4004, "halfword preload hint must be a NOP");

    // The byte was never consumed as a PC — confirm no spurious branch left
    // PC in the flash-alias region.
    assert!(
        cpu.pc >= 0x4000,
        "PLD/PLDW must not have branched into low memory"
    );
}

#[test]
fn test_thumb2_barrier_is_nop() {
    // DMB SY = F3BF 8F5F. Executor must advance PC by 4 and not fault.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    let pc_before = cpu.pc;
    run_test_instr(&mut cpu, &mut bus, 0xF3BF_8F5F, true);
    assert_eq!(cpu.pc, pc_before + 4, "DMB advances PC by 4");
}

#[test]
fn test_thumb2_msr_mrs_primask() {
    // MSR PRIMASK, r0 = F380 8810 ; MRS r1, PRIMASK = F3EF 8110.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.r0 = 1; // request PRIMASK = 1 (interrupts disabled)
    run_test_instr(&mut cpu, &mut bus, 0xF380_8810, true);
    assert!(cpu.primask, "MSR PRIMASK set primask from r0");

    run_test_instr(&mut cpu, &mut bus, 0xF3EF_8110, true);
    assert_eq!(cpu.r1, 1, "MRS reads primask back into r1");

    // Clearing: MSR PRIMASK, r0 with r0 = 0.
    cpu.r0 = 0;
    run_test_instr(&mut cpu, &mut bus, 0xF380_8810, true);
    assert!(!cpu.primask);
}

#[test]
fn test_thumb2_wide_multiplies() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();

    // SMULL rd_lo=r0, rd_hi=r1, rn=r2, rm=r3.
    // Encoding: 1111 1011 1000 rn4 | rd_lo4 rd_hi4 0000 rm4
    //         = F B 8 2 | 0 1 0 3 = FB82_0103
    cpu.r2 = u32::MAX; // -1
    cpu.r3 = 2;
    run_test_instr(&mut cpu, &mut bus, 0xFB82_0103, true);
    // -1 * 2 = -2 → 64-bit 0xFFFF_FFFF_FFFF_FFFE
    assert_eq!(cpu.r0, 0xFFFF_FFFE, "SMULL low half");
    assert_eq!(cpu.r1, 0xFFFF_FFFF, "SMULL high half (sign-extended)");

    // UMULL same operands: u32::MAX * 2 = 0x1_FFFF_FFFE.
    // Encoding: 1111 1011 1010 rn4 | rd_lo4 rd_hi4 0000 rm4 = FBA2_0103
    cpu.r2 = u32::MAX;
    cpu.r3 = 2;
    run_test_instr(&mut cpu, &mut bus, 0xFBA2_0103, true);
    assert_eq!(cpu.r0, 0xFFFF_FFFE, "UMULL low half");
    assert_eq!(cpu.r1, 0x0000_0001, "UMULL high half");
}

/// SMLABB/BT/TB/TT and SMULBB — the DSP halfword multiplies.
///
/// ⚠️ THIS IS A BLINK TEST WEARING A DECODER TEST'S CLOTHES. `smlabb r3,
/// r3, r4, r2` is what GCC emits at -Os for the port-base arithmetic in
/// Arduino's `digitalWrite` on EFR32MG26 — `0x4003C000 + 0x30 * (pin >> 4)`
/// — and while it was undecoded it was a silent no-op, so `digitalWrite`
/// wrote to a garbage address. The BRD2709A Arduino column read 6 pass /
/// 2 fail, and the two failures were blink and SPI: the two sketches that
/// drive a pin.
///
/// The negative cases matter as much as the positive one. Taking the halves
/// as UNSIGNED agrees with the hardware on every small positive operand —
/// which is every pin number — and disagrees on every negative one.
///
/// ⚠️ Every encoding below is `arm-none-eabi-as -mcpu=cortex-m33` output,
/// not hand-derived. Two of them were hand-derived first and both put Rm at
/// r0: the product came out zero and the assertion still read like a
/// sign-extension bug rather than a typo in the test.
#[test]
fn test_thumb2_halfword_multiplies() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();

    // The exact instruction from a compiled digitalWrite:
    //   smlabb r3, r3, r4, r2   =  FB13 2304
    // r3 = SInt(r3[15:0]) * SInt(r4[15:0]) + r2
    cpu.r3 = 0x30; // 48 bytes per GPIO port
    cpu.r4 = 2; // port index for a PC pin
    cpu.r2 = 0x4003_C000; // GPIO block base
    run_test_instr(&mut cpu, &mut bus, 0xFB13_2304, true);
    assert_eq!(
        cpu.r3, 0x4003_C060,
        "SMLABB: PORTC base = 0x4003C000 + 48*2"
    );

    // ⚠️ SIGNED, and the halves are sign-extended BEFORE the multiply.
    // -2 * 3 = -6, not 65534 * 3.
    cpu.r3 = 0x0000_FFFE; // bottom half = -2
    cpu.r4 = 0x0000_0003;
    cpu.r2 = 0;
    run_test_instr(&mut cpu, &mut bus, 0xFB13_2304, true);
    assert_eq!(cpu.r3 as i32, -6, "SMLABB sign-extends both halves");

    // The TOP-half selectors are separate bits and must not be swapped.
    //   smlatb r0, r1, r2, r3  = FB11 3022  (N=1 -> Rn top, M=0 -> Rm bottom)
    cpu.r1 = 0x0005_0000; // top half = 5, bottom = 0
    cpu.r2 = 0x0000_0007; // bottom half = 7
    cpu.r3 = 1;
    run_test_instr(&mut cpu, &mut bus, 0xFB11_3022, true);
    assert_eq!(cpu.r0, 36, "SMLATB: 5*7 + 1, Rn top and Rm bottom");

    //   smlabt r0, r1, r2, r3  = FB11 3012  (N=0 -> Rn bottom, M=1 -> Rm top)
    cpu.r1 = 0x0000_0005;
    cpu.r2 = 0x0007_0000;
    cpu.r3 = 1;
    run_test_instr(&mut cpu, &mut bus, 0xFB11_3012, true);
    assert_eq!(cpu.r0, 36, "SMLABT: Rn bottom and Rm top");

    // Ra == 0b1111 is SMULBB — the product alone, with NO addend. Reading
    // r15 as an accumulator instead would add the PC.
    //   smulbb r0, r1, r2  = FB11 F002
    cpu.r1 = 6;
    cpu.r2 = 7;
    run_test_instr(&mut cpu, &mut bus, 0xFB11_F002, true);
    assert_eq!(cpu.r0, 42, "SMULBB does not accumulate");
}

#[test]
fn test_thumb2_mla_mls() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();

    // MLA r0 = r3 + (r1 * r2)
    // Encoding: 1111 1011 0000 rn4 | ra4 rd4 0000 rm4
    //         = F B 0 1 | 3 0 0 2 = FB01_3002
    cpu.r1 = 2;
    cpu.r2 = 3;
    cpu.r3 = 100;
    run_test_instr(&mut cpu, &mut bus, 0xFB01_3002, true);
    assert_eq!(cpu.r0, 106, "MLA: 100 + 2*3 = 106");

    // MLS r0 = r3 - (r1 * r2) — op selector 0x1 in h2[7:4].
    // Encoding: FB01_3012
    cpu.r1 = 2;
    cpu.r2 = 3;
    cpu.r3 = 100;
    run_test_instr(&mut cpu, &mut bus, 0xFB01_3012, true);
    assert_eq!(cpu.r0, 94, "MLS: 100 - 2*3 = 94");
}

#[test]
fn test_thumb2_vfp_smoke_3_14_times_2() {
    // Exact reproduction of what the NUCLEO-L476RG smoke firmware does:
    //   ldr r3, =0x4048F5C3        ; 3.14f
    //   str r3, [sp, #12]
    //   movw r3, #0x0000           ; 2.0f low half
    //   movt r3, #0x4000           ; 2.0f high half
    //   str r3, [sp, #16]
    //   vldr s15, [sp, #12]        ; load 3.14
    //   vldr s14, [sp, #16]        ; load 2.0
    //   vmul.f32 s15, s15, s14
    //   vstr s15, [sp, #20]
    //   ldr r4, [sp, #20]          ; r4 = IEEE bits of 6.28
    // Hardware-verified result: 3.14f * 2.0f = 6.28f = 0x40C8F5C3.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    // Set up memory so the VLDR/VSTR have somewhere to land.
    cpu.sp = 0x2000_0100;
    bus.write_u32(0x2000_010C, 0x4048_F5C3).unwrap(); // 3.14f
    bus.write_u32(0x2000_0110, 0x4000_0000).unwrap(); // 2.0f

    // VLDR S15, [SP, #12] = EDDD 7A03
    run_test_instr(&mut cpu, &mut bus, 0xEDDD_7A03, true);
    assert_eq!(cpu.fpu_s[15], 0x4048_F5C3);

    // VLDR S14, [SP, #16] = ED9D 7A04
    run_test_instr(&mut cpu, &mut bus, 0xED9D_7A04, true);
    assert_eq!(cpu.fpu_s[14], 0x4000_0000);

    // VMUL.F32 S15, S15, S14 = EE67 7A87
    run_test_instr(&mut cpu, &mut bus, 0xEE67_7A87, true);
    assert_eq!(
        cpu.fpu_s[15], 0x40C8_F5C3,
        "3.14f * 2.0f IEEE-754 bits — must match real Cortex-M4F output"
    );

    // VSTR S15, [SP, #20] = EDCD 7A05
    run_test_instr(&mut cpu, &mut bus, 0xEDCD_7A05, true);
    assert_eq!(bus.read_u32(0x2000_0114).unwrap(), 0x40C8_F5C3);
}

fn vfp_arith_encoding(h1_op: u16, sd: u8, sn: u8, sm: u8, op_b: u32) -> u32 {
    // Build the 32-bit Thumb encoding for VMUL/VADD/VSUB/VDIV.F32.
    // Sd:D = (Vd<<1):D, similarly for Sn and Sm. op_b selects ADD vs
    // SUB at bit[6] of h2.
    let vd = (sd >> 1) & 0xF;
    let d = (sd & 1) as u32;
    let vn = (sn >> 1) & 0xF;
    let n = (sn & 1) as u32;
    let vm = (sm >> 1) & 0xF;
    let m = (sm & 1) as u32;
    let h1 = h1_op | ((d as u16) << 6) | (vn as u16);
    let h2 = ((vd as u16) << 12)
        | 0x0A00
        | ((n as u16) << 7)
        | ((op_b as u16) << 6)
        | ((m as u16) << 5)
        | (vm as u16);
    ((h1 as u32) << 16) | (h2 as u32)
}

#[test]
fn test_thumb2_vfp_add_sub_div() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.fpu_s[0] = (6.0_f32).to_bits();
    cpu.fpu_s[1] = (2.0_f32).to_bits();

    run_test_instr(
        &mut cpu,
        &mut bus,
        vfp_arith_encoding(0xEE30, 2, 0, 1, 0),
        true,
    );
    assert_eq!(cpu.fpu_s[2], (8.0_f32).to_bits(), "VADD: 6 + 2 = 8");

    run_test_instr(
        &mut cpu,
        &mut bus,
        vfp_arith_encoding(0xEE30, 2, 0, 1, 1),
        true,
    );
    assert_eq!(cpu.fpu_s[2], (4.0_f32).to_bits(), "VSUB: 6 - 2 = 4");

    run_test_instr(
        &mut cpu,
        &mut bus,
        vfp_arith_encoding(0xEE80, 2, 0, 1, 0),
        true,
    );
    assert_eq!(cpu.fpu_s[2], (3.0_f32).to_bits(), "VDIV: 6 / 2 = 3");

    run_test_instr(
        &mut cpu,
        &mut bus,
        vfp_arith_encoding(0xEE20, 2, 0, 1, 0),
        true,
    );
    assert_eq!(cpu.fpu_s[2], (12.0_f32).to_bits(), "VMUL: 6 * 2 = 12");
}

fn vfp_fma_encoding(h1_base: u16, sd: u8, sn: u8, sm: u8, opc3: u32) -> u32 {
    // Build the 32-bit Thumb encoding for VFMA/VFMS/VFNMA/VFNMS.F32.
    // h1_base is 0xEEA0 (VFMA/VFMS group) or 0xEE90 (VFNMA/VFNMS group)
    // with D and Vn cleared; opc3 selects the h2[6] bit.
    let vd = (sd >> 1) & 0xF;
    let d = (sd & 1) as u32;
    let vn = (sn >> 1) & 0xF;
    let n = (sn & 1) as u32;
    let vm = (sm >> 1) & 0xF;
    let m = (sm & 1) as u32;
    let h1 = h1_base | ((d as u16) << 6) | (vn as u16);
    let h2 = ((vd as u16) << 12)
        | 0x0A00
        | ((n as u16) << 7)
        | ((opc3 as u16) << 6)
        | ((m as u16) << 5)
        | (vm as u16);
    ((h1 as u32) << 16) | (h2 as u32)
}

#[test]
fn test_thumb2_vfma_decodes_the_real_h563_opcode() {
    // The exact bytes logged from stm32h563 firmware: 0xeee7 0x7a06.
    // Register-number assembly is Sx = (Vx << 1) | bit, where the D/N/M
    // bits live in different halfwords than the Vd/Vn/Vm fields — with
    // D=1 (h1 bit6), Vn=7 (h1[3:0]), Vd=7 (h2[15:12]), N=0 (h2 bit7),
    // M=0 (h2 bit5), Vm=6 (h2[3:0]) this decodes to
    // VFMA.F32 S15, S14, S12 (not S7,S7,S6 — Vd/Vn/Vm are only half of
    // each register number).
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.fpu_s[14] = (2.0_f32).to_bits();
    cpu.fpu_s[12] = (3.0_f32).to_bits();
    cpu.fpu_s[15] = (2.0_f32).to_bits();
    run_test_instr(&mut cpu, &mut bus, 0xEEE7_7A06, true);
    // S15 = fused(S14 * S12) + S15 = fused(2*3) + 2 = 8
    assert_eq!(cpu.fpu_s[15], (8.0_f32).to_bits());
}

#[test]
fn test_thumb2_vfma_is_truly_fused_not_double_rounded() {
    // Choose operands where round(a*b) then +c differs from the fused
    // single-rounding result. This proves mul_add (fused) is used
    // rather than `a * b + c` (which would round the product first).
    let a: f32 = 1.134_364_2;
    let b: f32 = 1.847_433_7;
    let c: f32 = -2.095_662_8;
    let unfused = (a * b) + c;
    let fused = a.mul_add(b, c);
    assert_ne!(
        unfused.to_bits(),
        fused.to_bits(),
        "test operands must actually exercise the fused/unfused difference"
    );

    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.fpu_s[0] = a.to_bits();
    cpu.fpu_s[1] = b.to_bits();
    cpu.fpu_s[2] = c.to_bits();
    // VFMA.F32 S2, S0, S1
    run_test_instr(
        &mut cpu,
        &mut bus,
        vfp_fma_encoding(0xEEA0, 2, 0, 1, 0),
        true,
    );
    assert_eq!(
        cpu.fpu_s[2],
        fused.to_bits(),
        "VFMA must use fused multiply-add (single rounding), not a*b+c"
    );
}

#[test]
fn test_thumb2_vfms_vfnma_vfnms_sign_handling() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.fpu_s[0] = (6.0_f32).to_bits(); // Sn
    cpu.fpu_s[1] = (2.0_f32).to_bits(); // Sm
    cpu.fpu_s[2] = (5.0_f32).to_bits(); // Sd (accumulator)

    // VFMS.F32 S2, S0, S1 = fused(-6*2) + 5 = -12 + 5 = -7
    run_test_instr(
        &mut cpu,
        &mut bus,
        vfp_fma_encoding(0xEEA0, 2, 0, 1, 1),
        true,
    );
    assert_eq!(cpu.fpu_s[2], (-7.0_f32).to_bits(), "VFMS: -(6*2)+5 = -7");

    cpu.fpu_s[0] = (6.0_f32).to_bits();
    cpu.fpu_s[1] = (2.0_f32).to_bits();
    cpu.fpu_s[2] = (5.0_f32).to_bits();
    // VFNMA.F32 S2, S0, S1 = fused(6*2) - 5 = 12 - 5 = 7
    run_test_instr(
        &mut cpu,
        &mut bus,
        vfp_fma_encoding(0xEE90, 2, 0, 1, 0),
        true,
    );
    assert_eq!(cpu.fpu_s[2], (7.0_f32).to_bits(), "VFNMA: (6*2)-5 = 7");

    cpu.fpu_s[0] = (6.0_f32).to_bits();
    cpu.fpu_s[1] = (2.0_f32).to_bits();
    cpu.fpu_s[2] = (5.0_f32).to_bits();
    // VFNMS.F32 S2, S0, S1 = fused(-6*2) - 5 = -12 - 5 = -17
    run_test_instr(
        &mut cpu,
        &mut bus,
        vfp_fma_encoding(0xEE90, 2, 0, 1, 1),
        true,
    );
    assert_eq!(cpu.fpu_s[2], (-17.0_f32).to_bits(), "VFNMS: -(6*2)-5 = -17");
}

#[test]
fn test_thumb_sxth_sxtb_uxth_uxtb() {
    // Family encoding: 1011 0010 op2:2 mmm:3 ddd:3.
    //   op2 = 00 -> SXTH, 01 -> SXTB, 10 -> UXTH, 11 -> UXTB
    // Surfaced on NUCLEO-L476RG: GCC emits UXTH (0xB280) when
    // truncating a uint16_t expression to fit a u32 register;
    // sim was raising "Unknown instruction" for it.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();

    // SXTH r0, r1 = 0xB208 (Rm=1, Rd=0).
    cpu.r1 = 0x0000_8000; // bit 15 set -> negative as i16
    run_test_instr(&mut cpu, &mut bus, 0xB208, false);
    assert_eq!(cpu.r0, 0xFFFF_8000, "SXTH sign-extends bit 15");

    // SXTB r0, r1 = 0xB248 (Rm=1, Rd=0).
    cpu.r1 = 0x0000_0080;
    run_test_instr(&mut cpu, &mut bus, 0xB248, false);
    assert_eq!(cpu.r0, 0xFFFF_FF80, "SXTB sign-extends bit 7");

    // UXTH r0, r1 = 0xB288.
    cpu.r1 = 0x1234_ABCD;
    run_test_instr(&mut cpu, &mut bus, 0xB288, false);
    assert_eq!(cpu.r0, 0x0000_ABCD, "UXTH zero-extends low 16");

    // UXTB r0, r1 = 0xB2C8.
    cpu.r1 = 0x1234_ABCD;
    run_test_instr(&mut cpu, &mut bus, 0xB2C8, false);
    assert_eq!(cpu.r0, 0x0000_00CD, "UXTB zero-extends low 8");
}

#[test]
fn test_thumb2_addw_subw_plain_immediate() {
    // ADDW r3, r3, #0x789 (T4 plain immediate). Encoding F203 7389:
    //   1111 0 010 0000 0011 | 0 111 0011 10001001
    // imm12 = 0:111:10001001 = 0x789 zero-extended (NOT ThumbExpand).
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.r3 = 0x12344EEF;
    run_test_instr(&mut cpu, &mut bus, 0xF203_7389, true);
    assert_eq!(cpu.r3, 0x12345678, "ADDW r3, r3, #0x789");

    // SUBW r3, r3, #0x10 (T4). Encoding pattern F2A3 0310:
    //   1111 0 010 1010 0011 | 0 000 0011 00010000
    cpu.r3 = 0x100;
    run_test_instr(&mut cpu, &mut bus, 0xF2A3_0310, true);
    assert_eq!(cpu.r3, 0xF0, "SUBW r3, r3, #0x10");
}

#[test]
fn test_thumb2_shift_register_lsr_lsl_asr() {
    // Regression: the Thumb-2 shift-by-register encoding (FA0x..FA7x)
    // was reading shift_type from h2[5:4] instead of h1[6:5], so
    // LSR/ASR/ROR were silently decoded as LSL. Surfaced on
    // NUCLEO-L476RG via __aeabi_u2h emit from `(v >> n) & 0xF` in a
    // stock GCC hex print loop.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();

    // LSR.W r2, r0, r3  (FA20 F203). r0 = 0x60FC303A, r3 = 28 -> r2 = 0x6.
    cpu.r0 = 0x60FC303A;
    cpu.r3 = 28;
    run_test_instr(&mut cpu, &mut bus, 0xFA20_F203, true);
    assert_eq!(cpu.r2, 0x6, "LSR.W by 28 of 0x60FC303A");

    // LSL.W r2, r0, r3  (FA00 F203). r0 = 0x6, r3 = 28 -> r2 = 0x60000000.
    cpu.r0 = 0x6;
    cpu.r3 = 28;
    run_test_instr(&mut cpu, &mut bus, 0xFA00_F203, true);
    assert_eq!(cpu.r2, 0x6000_0000, "LSL.W by 28 of 0x6");

    // ASR.W r2, r0, r3  (FA40 F203). r0 = 0xF0000000, r3 = 4 -> r2 = 0xFF000000.
    cpu.r0 = 0xF000_0000;
    cpu.r3 = 4;
    run_test_instr(&mut cpu, &mut bus, 0xFA40_F203, true);
    assert_eq!(cpu.r2, 0xFF00_0000, "ASR.W by 4 of 0xF0000000 sign-extends");
}

/// SHPR3-driven priority dispatch: PendSV at lowest priority (0xFF) must
/// not preempt an active higher-priority IRQ. This is the load-bearing
/// behaviour for FreeRTOS — SysTick (higher prio) pends PendSV which
/// only takes once SysTick returns. Once SysTick is no longer active,
/// PendSV is takeable from thread mode.
#[test]
fn shpr3_pendsv_does_not_preempt_active_higher_priority_irq() {
    let mut cpu = CortexM::new();
    // Wire SHPR3 with PendSV (byte 2) at 0xFF, SysTick (byte 3) at 0x00.
    let shpr1 = Arc::new(AtomicU32::new(0));
    let shpr2 = Arc::new(AtomicU32::new(0));
    let shpr3 = Arc::new(AtomicU32::new(0x00FF_0000));
    cpu.set_shared_shpr(shpr1, shpr2, shpr3);

    // PendSV at 0xFF, SysTick at 0x00.
    assert_eq!(cpu.exception_priority(14), 0xFF);
    assert_eq!(cpu.exception_priority(15), 0x00);

    // PendSV pending while SysTick is active — must NOT be takeable.
    cpu.active_exception = 15;
    cpu.pending_exceptions[0] = 1u64 << 14;
    assert_eq!(cpu.highest_priority_pending(), Some(14));
    let active_prio = cpu.exception_priority(cpu.active_exception);
    let pend_prio = cpu.exception_priority(14);
    assert!(
        pend_prio >= active_prio,
        "PendSV (0xFF) must not preempt active SysTick (0x00)"
    );

    // SysTick returns; from thread mode PendSV must be takeable.
    cpu.active_exception = 0;
    let active_prio = cpu.exception_priority(0);
    assert!(
        pend_prio < active_prio,
        "PendSV at 0xFF must be takeable from thread mode (256)"
    );
}

/// IRQs read priorities from the shared NVIC IPR. Two pending IRQs
/// with different priorities must dispatch by priority, not by IRQ
/// number.
// --- Thumb-1 register-offset load/store execution tests ---
// Opcodes derived from ARMv7-M A6.2.4: bits[15:9] = 0101 op[2:0],
// bits[8:6]=Rm, bits[5:3]=Rn, bits[2:0]=Rt.
// All four tests use Rt=R0, Rn=R1 (base), Rm=R2 (offset).

#[test]
fn test_exec_strh_reg_offset() {
    // STRH R0, [R1, R2] — op=001 — 0101 001 010 001 000 = 0x5288
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x2000;
    // Base address in R1, zero offset in R2.
    cpu.r1 = 0x3000;
    cpu.r2 = 0x0000;
    // Value to store: 0xBEEF (bottom 16 bits).
    cpu.r0 = 0x0000BEEF;
    run_test_instr(&mut cpu, &mut bus, 0x5288, false);
    // The halfword at 0x3000 should be 0xBEEF.
    assert_eq!(bus.read_u16(0x3000).unwrap(), 0xBEEF);
}

#[test]
fn test_exec_wide_register_extend() {
    // Regression: the wide (T2) register-extend instructions were not
    // decoded (fell to Unknown32 and were skipped), leaving Rd stale.
    // clang emits e.g. `uxth.w r2, ip` = FA1F F28C when extending a high
    // register, which corrupted a UDS routine-id argument (read as 0).
    // UXTH.W R2, R12  = FA1F F28C — zero-extend low 16 bits.
    {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r2 = 0xDEAD_BEEF; // stale value that must be overwritten
        cpu.r12 = 0x1234_FF00;
        run_test_instr(&mut cpu, &mut bus, 0xFA1FF28C, true);
        assert_eq!(cpu.r2, 0x0000_FF00, "UXTH.W must zero-extend low 16 bits");
    }
    // UXTB.W R0, R1   = FA5F F081 — zero-extend low 8 bits.
    {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r1 = 0x0000_0085;
        run_test_instr(&mut cpu, &mut bus, 0xFA5FF081, true);
        assert_eq!(cpu.r0, 0x0000_0085, "UXTB.W must zero-extend low 8 bits");
    }
    // SXTB.W R0, R1   = FA4F F081 — sign-extend low 8 bits.
    {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r1 = 0x0000_0085;
        run_test_instr(&mut cpu, &mut bus, 0xFA4FF081, true);
        assert_eq!(cpu.r0, 0xFFFF_FF85, "SXTB.W must sign-extend low 8 bits");
    }
    // UXTH.W R0, R1, ROR #8 = FA1F F091 — rotate then zero-extend.
    {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r1 = 0x0085_0000;
        run_test_instr(&mut cpu, &mut bus, 0xFA1FF091, true);
        // ROR #8 of 0x00850000 = 0x00008500; & 0xFFFF = 0x8500.
        assert_eq!(cpu.r0, 0x0000_8500, "UXTH.W ROR #8 must rotate then extend");
    }
    // UXTAH R0, R1, R2 = FA11 F082 — R0 = R1 + uxth(R2) (extend-and-add).
    // This is the `4 + path_len` form (uxtah r6,r3,r0) that the plain-extend
    // decode missed, leaving the result register stale.
    {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r0 = 0xDEAD_BEEF; // stale value that must be overwritten
        cpu.r1 = 0x0000_0004;
        cpu.r2 = 0x1234_0002;
        run_test_instr(&mut cpu, &mut bus, 0xFA11F082, true);
        assert_eq!(cpu.r0, 0x0000_0006, "UXTAH must add Rn to the extended Rm");
    }
}

#[test]
fn test_exec_wide_load_byte_halfword_extension() {
    // Regression: the wide (32-bit Thumb-2) load encodings select
    // signed vs unsigned via h1 bit 8 (0x0100), not op1 bit 3.
    // Previously LDRB.W T2 (0xF89x) and LDRH.W T2 (0xF8Bx) wrongly
    // sign-extended, corrupting any byte/halfword with the top bit set
    // (e.g. a UDS SID 0x85 read back as 0xFFFFFF85).
    // Each sub-test uses a fresh cpu/bus: instruction memory at a fixed
    // address is treated as ROM by MockBus and will not accept a rewrite,
    // so rerunning at the same pc would refetch the first instruction.
    // LDRB.W R0, [R1, #0]  = F891 0000 — must ZERO-extend.
    {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r1 = 0x3000;
        bus.write_u8(0x3000, 0x85).unwrap();
        run_test_instr(&mut cpu, &mut bus, 0xF8910000, true);
        assert_eq!(cpu.r0, 0x0000_0085, "LDRB.W must zero-extend 0x85");
    }
    // LDRSB.W R0, [R1, #0] = F991 0000 — must SIGN-extend.
    {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r1 = 0x3000;
        bus.write_u8(0x3000, 0x85).unwrap();
        run_test_instr(&mut cpu, &mut bus, 0xF9910000, true);
        assert_eq!(cpu.r0, 0xFFFF_FF85, "LDRSB.W must sign-extend 0x85");
    }
    // LDRH.W R0, [R1, #0]  = F8B1 0000 — must ZERO-extend.
    {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r1 = 0x3000;
        bus.write_u16(0x3000, 0x8042).unwrap();
        run_test_instr(&mut cpu, &mut bus, 0xF8B10000, true);
        assert_eq!(cpu.r0, 0x0000_8042, "LDRH.W must zero-extend 0x8042");
    }
    // LDRSH.W R0, [R1, #0] = F9B1 0000 — must SIGN-extend.
    {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r1 = 0x3000;
        bus.write_u16(0x3000, 0x8042).unwrap();
        run_test_instr(&mut cpu, &mut bus, 0xF9B10000, true);
        assert_eq!(cpu.r0, 0xFFFF_8042, "LDRSH.W must sign-extend 0x8042");
    }
}

#[test]
fn test_exec_ldrsb_reg_offset_positive() {
    // LDRSB R0, [R1, R2] — op=011 — 0101 011 010 001 000 = 0x5688
    // Positive byte (MSB clear): no sign extension needed.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x2000;
    cpu.r1 = 0x3000;
    cpu.r2 = 0x0004;
    bus.write_u8(0x3004, 0x7F).unwrap(); // +127
    run_test_instr(&mut cpu, &mut bus, 0x5688, false);
    assert_eq!(cpu.r0, 0x0000007F);
}

#[test]
fn test_exec_ldrsb_reg_offset_negative() {
    // LDRSB R0, [R1, R2] — sign-extends a byte with MSB set.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x2000;
    cpu.r1 = 0x3000;
    cpu.r2 = 0x0000;
    bus.write_u8(0x3000, 0xFF).unwrap(); // -1 as i8
    run_test_instr(&mut cpu, &mut bus, 0x5688, false);
    // Sign-extended to 32 bits: 0xFFFFFFFF.
    assert_eq!(cpu.r0, 0xFFFFFFFF);
}

#[test]
fn test_exec_ldrh_reg_offset() {
    // LDRH R0, [R1, R2] — op=101 — 0101 101 010 001 000 = 0x5A88
    // Zero-extends the 16-bit halfword.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x2000;
    cpu.r1 = 0x4000;
    cpu.r2 = 0x0002;
    bus.write_u16(0x4002, 0xDEAD).unwrap();
    run_test_instr(&mut cpu, &mut bus, 0x5A88, false);
    assert_eq!(cpu.r0, 0x0000DEAD);
}

#[test]
fn test_exec_ldmia_base_in_list_no_writeback() {
    // LDMIA R2, {R0, R1, R2} = 0xCA07 (base R2 is IN the list → no `!` →
    // no writeback; the LOADED value wins). Regression for the KW41Z-LCD
    // "blank cow" bug: the compiler's struct-copy / stacked-arg reload
    // idiom `ldmia rN, {..., rN}` had its final register clobbered by an
    // unconditional base writeback, silently dropping a loaded argument.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x2000;
    cpu.r2 = 0x4000;
    bus.write_u32(0x4000, 0x1111_1111).unwrap(); // -> r0
    bus.write_u32(0x4004, 0x2222_2222).unwrap(); // -> r1
    bus.write_u32(0x4008, 0x0000_0014).unwrap(); // -> r2 (the loaded value)
    run_test_instr(&mut cpu, &mut bus, 0xCA07, false);
    assert_eq!(cpu.r0, 0x1111_1111);
    assert_eq!(cpu.r1, 0x2222_2222);
    // Must be the value loaded from [base+8], NOT base+12 (0x400C).
    assert_eq!(cpu.r2, 0x0000_0014, "base-in-list LDM must not write back");
}

#[test]
fn test_exec_ldmia_base_not_in_list_writes_back() {
    // LDMIA R3!, {R0, R1} = 0xCB03 (base R3 NOT in list → writeback).
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x2000;
    cpu.r3 = 0x4000;
    bus.write_u32(0x4000, 0xAAAA_AAAA).unwrap();
    bus.write_u32(0x4004, 0xBBBB_BBBB).unwrap();
    run_test_instr(&mut cpu, &mut bus, 0xCB03, false);
    assert_eq!(cpu.r0, 0xAAAA_AAAA);
    assert_eq!(cpu.r1, 0xBBBB_BBBB);
    assert_eq!(
        cpu.r3, 0x4008,
        "base-not-in-list LDM must write back base+8"
    );
}

#[test]
fn test_exec_ldrsh_reg_offset_negative() {
    // LDRSH R0, [R1, R2] — op=111 — 0101 111 010 001 000 = 0x5E88
    // Sign-extends a halfword with MSB set.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x2000;
    cpu.r1 = 0x4000;
    cpu.r2 = 0x0000;
    bus.write_u16(0x4000, 0x8001).unwrap(); // negative i16
    run_test_instr(&mut cpu, &mut bus, 0x5E88, false);
    // Sign-extended: 0xFFFF8001.
    assert_eq!(cpu.r0, 0xFFFF8001);
}

#[test]
fn test_exec_rev_t1_16bit() {
    // REV T1: `rev r3, r3` = 0xBA1B — byte-reverse a 32-bit word.
    // 0x11223344 → 0x44332211.
    {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r3 = 0x1122_3344;
        run_test_instr(&mut cpu, &mut bus, 0xBA1B, false);
        assert_eq!(cpu.r3, 0x4433_2211, "REV T1 must byte-swap the whole word");
    }
    // REV16 T1: `rev16 r5, r5` = 0xBA6D — swap bytes within each halfword.
    // 0x11223344 → bytes in low half swapped + bytes in high half swapped
    // = 0x22114433.
    {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r5 = 0x1122_3344;
        run_test_instr(&mut cpu, &mut bus, 0xBA6D, false);
        assert_eq!(
            cpu.r5, 0x2211_4433,
            "REV16 T1 must swap bytes within each halfword"
        );
    }
    // REVSH T1: `revsh r0, r1` = 0xBAC8 — swap low two bytes, sign-extend.
    // Input r1=0x00008001: low halfword bytes swapped → 0x0180, sign-extended
    // as i16 = 0x0180 (positive, MSB not set) → 0x00000180.
    {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r1 = 0x0000_8001;
        run_test_instr(&mut cpu, &mut bus, 0xBAC8, false);
        assert_eq!(cpu.r0, 0x0000_0180, "REVSH T1 positive case");
    }
    // REVSH sign case: input r1=0x00000180 — low halfword bytes swapped
    // → 0x8001, sign-extended as i16 → 0xFFFF8001.
    {
        let mut cpu = CortexM::new();
        let mut bus = MockBus::new();
        cpu.pc = 0x2000;
        cpu.r1 = 0x0000_0180;
        run_test_instr(&mut cpu, &mut bus, 0xBAC8, false);
        assert_eq!(cpu.r0, 0xFFFF_8001, "REVSH T1 sign-extend case");
    }
}

#[test]
fn nvic_ipr_priority_drives_irq_dispatch_order() {
    let mut cpu = CortexM::new();
    let nvic = Arc::new(crate::peripherals::nvic::NvicState::default());
    // IRQ0 priority = 0xC0, IRQ1 priority = 0x40. IRQ1 has higher
    // priority despite being a larger IRQ number.
    nvic.ipr[0].store(0x0000_40C0, Ordering::Relaxed);
    cpu.set_shared_nvic_state(nvic);

    assert_eq!(cpu.exception_priority(16), 0xC0); // IRQ0 → exc 16
    assert_eq!(cpu.exception_priority(17), 0x40); // IRQ1 → exc 17

    cpu.pending_exceptions[0] = (1u64 << 16) | (1u64 << 17);
    assert_eq!(
        cpu.highest_priority_pending(),
        Some(17),
        "IRQ1 (prio 0x40) outranks IRQ0 (prio 0xC0)"
    );
}

// --- Banked MSP/PSP + CONTROL.SPSEL + EXC_RETURN (ARMv7-M) ---

/// Pend `exc`, point its vector at `handler`, and take the exception by
/// stepping once. Assumes priority lets it through (thread mode / IRQ).
fn take_exception(cpu: &mut CortexM, bus: &mut MockBus, exc: u32, handler: u32) {
    bus.write_u32((exc * 4) as u64, handler).unwrap();
    cpu.set_exception_pending(exc);
    cpu.step_internal(bus, &[], &bus.config.clone()).unwrap();
}

#[test]
fn msr_psp_sets_psp_without_disturbing_msp() {
    // Thread mode, MSP active. MSR PSP, r0 must bank PSP and leave the
    // live MSP stack pointer untouched. MRS r1, PSP reads it back.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.sp = 0x8000; // MSP active
    cpu.r0 = 0x6000;
    // MSR PSP, r0 = F380 8809
    run_test_instr(&mut cpu, &mut bus, 0xF380_8809, true);
    assert_eq!(cpu.psp, 0x6000, "MSR PSP banks the value");
    assert_eq!(cpu.sp, 0x8000, "MSP (active sp) untouched");
    // MSP is the live bank here, so `sp` is authoritative for it.
    assert_eq!(cpu.read_msp(), 0x8000, "MSP read unchanged");

    // MRS r1, PSP = F3EF 8109
    run_test_instr(&mut cpu, &mut bus, 0xF3EF_8109, true);
    assert_eq!(cpu.r1, 0x6000, "MRS PSP reads back banked value");
}

#[test]
fn control_spsel_routes_thread_stack_to_psp() {
    // Set CONTROL.SPSEL=1 in thread mode → active stack becomes PSP, and
    // a PUSH must land on PSP, not MSP.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.sp = 0x8000; // MSP active
    cpu.psp = 0x6000;
    cpu.r0 = 0xDEAD_BEEF;

    // MSR CONTROL, r0 with r0=2 (SPSEL=1) = F380 8814
    cpu.r0 = 0x2;
    run_test_instr(&mut cpu, &mut bus, 0xF380_8814, true);
    assert_eq!(cpu.control & 0x2, 0x2, "CONTROL.SPSEL set");
    assert_eq!(cpu.msp, 0x8000, "leaving MSP banks its value");
    assert_eq!(cpu.sp, 0x6000, "active sp switched to PSP");

    // PUSH {r0} = B401 must decrement and write PSP.
    cpu.r0 = 0xDEAD_BEEF;
    run_test_instr(&mut cpu, &mut bus, 0x0000_B401, false);
    assert_eq!(cpu.sp, 0x5FFC, "PUSH used PSP");
    assert_eq!(bus.read_u32(0x5FFC).unwrap(), 0xDEAD_BEEF);

    // MRS r2, CONTROL = F3EF 8214
    run_test_instr(&mut cpu, &mut bus, 0xF3EF_8214, true);
    assert_eq!(cpu.r2 & 0x2, 0x2, "MRS CONTROL reflects SPSEL");
}

#[test]
fn exception_entry_from_thread_psp_sets_exc_return_fd() {
    // Thread/PSP fault → LR=0xFFFFFFFD, frame stacked on PSP, handler on MSP.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.control = 0x2; // SPSEL=1, thread/PSP
    cpu.sp = 0x6000; // PSP active
    cpu.msp = 0x8000; // banked MSP
    cpu.r0 = 0x1111_1111;

    take_exception(&mut cpu, &mut bus, 16, 0x2000);

    assert_eq!(cpu.lr, 0xFFFF_FFFD, "EXC_RETURN = Thread/PSP");
    assert_eq!(cpu.active_exception, 16, "now in handler");
    assert_eq!(cpu.sp, 0x8000, "handler runs on MSP");
    assert_eq!(cpu.psp, 0x5FE0, "frame stacked on PSP (0x6000-32)");
    assert_eq!(cpu.pc, 0x2000, "branched to handler");
    assert_eq!(
        bus.read_u32(0x5FE0).unwrap(),
        0x1111_1111,
        "r0 on PSP frame"
    );
}

#[test]
fn exception_entry_from_thread_msp_sets_exc_return_f9() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.control = 0x0; // SPSEL=0, thread/MSP
    cpu.sp = 0x8000; // MSP active

    take_exception(&mut cpu, &mut bus, 16, 0x2000);

    assert_eq!(cpu.lr, 0xFFFF_FFF9, "EXC_RETURN = Thread/MSP");
    assert_eq!(cpu.sp, 0x7FE0, "handler on MSP (0x8000-32)");
    assert_eq!(cpu.msp, 0x7FE0, "MSP bank updated");
}

#[test]
fn nested_exception_entry_sets_exc_return_f1() {
    // Already in a handler → nested exception returns to Handler mode (F1),
    // and stacks on MSP.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x2000;
    cpu.active_exception = 11; // SVCall in progress (prio 0)
    cpu.sp = 0x8000; // handler MSP
    cpu.psp = 0x6000; // banked thread PSP, must NOT be touched

    // HardFault (exc 3, prio -1) preempts.
    take_exception(&mut cpu, &mut bus, 3, 0x3000);

    assert_eq!(cpu.lr, 0xFFFF_FFF1, "EXC_RETURN = return to Handler");
    assert_eq!(cpu.active_exception, 3);
    assert_eq!(cpu.sp, 0x7FE0, "nested frame on MSP");
    assert_eq!(cpu.msp, 0x7FE0, "MSP bank advanced");
    assert_eq!(cpu.psp, 0x6000, "PSP bank untouched by nested entry");
}

#[test]
fn exception_round_trip_from_psp_restores_state() {
    // Enter from thread/PSP, BX LR back, PSP + registers restored exactly.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.control = 0x2; // thread/PSP
    cpu.sp = 0x6000; // PSP active
    cpu.msp = 0x8000;
    cpu.r0 = 0xA0;
    cpu.r1 = 0xA1;
    cpu.r2 = 0xA2;
    cpu.r3 = 0xA3;
    cpu.r12 = 0xAC;
    cpu.lr = 0x1000_0001;
    cpu.xpsr = 0x0100_0000;

    // Handler at 0x2000 is a single BX LR (0x4770).
    bus.write_u16(0x2000, 0x4770).unwrap();
    take_exception(&mut cpu, &mut bus, 16, 0x2000);
    assert_eq!(cpu.active_exception, 16);
    assert_eq!(cpu.sp, 0x8000, "in handler on MSP");

    // Execute BX LR (EXC_RETURN).
    let cfg = bus.config.clone();
    cpu.step_internal(&mut bus, &[], &cfg).unwrap();

    assert_eq!(cpu.active_exception, 0, "back to thread mode");
    assert_eq!(cpu.sp, 0x6000, "PSP restored");
    assert_eq!(cpu.control & 0x2, 0x2, "still thread/PSP");
    assert_eq!(cpu.pc, 0x1000, "PC restored from frame");
    assert_eq!(cpu.r0, 0xA0);
    assert_eq!(cpu.r1, 0xA1);
    assert_eq!(cpu.r2, 0xA2);
    assert_eq!(cpu.r3, 0xA3);
    assert_eq!(cpu.r12, 0xAC);
    assert_eq!(cpu.lr, 0x1000_0001, "LR restored from frame");
}

#[test]
fn exception_return_to_msp_restores_msp() {
    // EXC_RETURN 0xFFFFFFF9 returns to thread/MSP.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.control = 0x0; // thread/MSP
    cpu.sp = 0x8000;
    bus.write_u16(0x2000, 0x4770).unwrap(); // BX LR
    take_exception(&mut cpu, &mut bus, 16, 0x2000);
    assert_eq!(cpu.lr, 0xFFFF_FFF9);

    let cfg = bus.config.clone();
    cpu.step_internal(&mut bus, &[], &cfg).unwrap();
    assert_eq!(cpu.active_exception, 0);
    assert_eq!(cpu.sp, 0x8000, "MSP restored");
    assert_eq!(cpu.control & 0x2, 0, "SPSEL stays MSP");
}

#[test]
fn wfi_decodes_and_retires_like_nop() {
    // 0xBF30 must decode to the dedicated WFI variant (not the hint-space
    // Nop) and, with no wake event pending, arm the sleep flag while still
    // advancing PC like any 16-bit hint.
    assert_eq!(decode_thumb_16(0xBF30), Instruction::Wfi);
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    run_test_instr(&mut cpu, &mut bus, 0xBF30, false); // WFI
    assert_eq!(cpu.pc, 0x1002, "WFI advances PC like a NOP");
    assert!(
        cpu.idle_fast_forward_budget(&bus).is_some(),
        "WFI with no pending event arms idle sleep"
    );
}

#[test]
fn wfi_is_nop_when_wake_already_pending() {
    // If a wake-up event is already pending when WFI retires, WFI completes
    // as a plain NOP and never arms sleep. PRIMASK is set only so the
    // pending IRQ isn't taken before the WFI retires — this isolates the
    // WFI arm's wake-pending branch.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.primask = true;
    cpu.set_exception_pending(16); // IRQ0 pending (priority 0xFF < 256)
    run_test_instr(&mut cpu, &mut bus, 0xBF30, false); // WFI
    assert_eq!(cpu.pc, 0x1002, "WFI retires like a NOP");
    assert!(
        cpu.idle_fast_forward_budget(&bus).is_none(),
        "an already-pending wake event means WFI does not arm sleep"
    );
}

#[test]
fn wfi_wakes_and_takes_exception_when_primask_clear() {
    // WFI with nothing pending sleeps; a subsequently-pended exception is a
    // wake event (budget clears), and with PRIMASK clear the next step
    // vectors into the handler.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.sp = 0x8000;
    bus.write_u16(0x1000, 0xBF30).unwrap(); // WFI
    let cfg = bus.config.clone();

    cpu.step_internal(&mut bus, &[], &cfg).unwrap();
    assert_eq!(cpu.pc, 0x1002, "WFI retires like a NOP");
    assert!(
        cpu.idle_fast_forward_budget(&bus).is_some(),
        "core is sleeping"
    );

    // SysTick (exception 15) pends while the core sleeps.
    bus.write_u32(15 * 4, 0x5000 | 1).unwrap(); // VTOR=0 → vector[15]
    cpu.set_exception_pending(15);
    assert!(
        cpu.idle_fast_forward_budget(&bus).is_none(),
        "a pended exception wakes the core"
    );

    cpu.step_internal(&mut bus, &[], &cfg).unwrap();
    assert_eq!(
        cpu.active_exception, 15,
        "PRIMASK clear → the exception is taken"
    );
    assert_eq!(cpu.pc, 0x5000, "vectored into the SysTick handler");
}

#[test]
fn wfi_primask_set_wakes_without_taking() {
    // The canonical `__disable_irq(); wfi();` idle pattern: PRIMASK is set,
    // so a pended exception must WAKE the core (budget clears) but must NOT
    // be taken — the core falls through to the instruction after WFI and the
    // exception stays pending.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.sp = 0x8000;
    cpu.primask = true; // __disable_irq()
    bus.write_u16(0x1000, 0xBF30).unwrap(); // WFI
    bus.write_u16(0x1002, 0xBF00).unwrap(); // NOP (the fall-through target)
    let cfg = bus.config.clone();

    cpu.step_internal(&mut bus, &[], &cfg).unwrap();
    assert_eq!(cpu.pc, 0x1002);
    assert!(
        cpu.idle_fast_forward_budget(&bus).is_some(),
        "core sleeps even with PRIMASK set"
    );

    bus.write_u32(15 * 4, 0x5000 | 1).unwrap();
    cpu.set_exception_pending(15);
    assert!(
        cpu.idle_fast_forward_budget(&bus).is_none(),
        "wake-on-pend fires despite PRIMASK"
    );

    cpu.step_internal(&mut bus, &[], &cfg).unwrap();
    assert_eq!(
        cpu.active_exception, 0,
        "PRIMASK blocks entry: the exception is NOT taken"
    );
    assert_eq!(cpu.pc, 0x1004, "core falls through past the WFI");
    assert!(
        cpu.pending_exceptions[0] & (1 << 15) != 0,
        "the masked exception stays pending"
    );
}

/// Build the minimal `WFI; b .-2` idle loop on a real `SystemBus` and run it
/// with the legacy walk disabled so idle fast-forward is legal.
fn wfi_idle_machine(ff: bool) -> Machine<CortexM> {
    let mut bus = SystemBus::new();
    bus.write_u16(0x0, 0xBF30).unwrap(); // WFI
    bus.write_u16(0x2, 0xE7FD).unwrap(); // B -> 0x0
    let mut cpu = CortexM::new();
    cpu.pc = 0x0;
    cpu.sp = 0x8000;
    let mut machine = Machine::new(cpu, bus);
    machine.config.idle_fast_forward_enabled = ff;
    machine.bus.legacy_walk_disabled = true;
    machine
}

#[test]
fn wfi_fast_forward_is_off_by_default() {
    use crate::DebugControl;
    let mut machine = wfi_idle_machine(false);
    machine.run(Some(10)).unwrap();
    assert_eq!(machine.total_cycles, 10);
    assert_eq!(
        machine.step_profile().cpu_instructions,
        10,
        "without fast-forward every idle cycle retires an instruction"
    );
}

#[test]
fn wfi_fast_forward_skips_cpu_work_when_enabled() {
    // Requires the idle fast-forward machinery (event-scheduler feature),
    // which the `--workspace` and `--features event-scheduler` CI lanes
    // build. Determinism: total_cycles is identical to the off case; only
    // the retired-instruction count drops.
    use crate::DebugControl;
    let mut off = wfi_idle_machine(false);
    off.run(Some(10)).unwrap();

    let mut on = wfi_idle_machine(true);
    on.run(Some(10)).unwrap();

    assert_eq!(
        on.total_cycles, off.total_cycles,
        "idle fast-forward must not change total_cycles"
    );
    assert_eq!(on.total_cycles, 10);
    if cfg!(feature = "event-scheduler") {
        assert!(
            on.step_profile().cpu_instructions < off.step_profile().cpu_instructions,
            "fast-forwarded cycles should not retire CPU instructions"
        );
    }
}

#[test]
fn boxed_cortexm_batch_preserves_idle_fast_forward_escape() {
    // The `Box<dyn Cpu>` forwarding must still leave the batch loop at WFI so
    // the machine can fast-forward (the WASM runtime holds a boxed CPU).
    use crate::DebugControl;
    let mut bus = SystemBus::new();
    bus.write_u16(0x0, 0xBF30).unwrap(); // WFI
    bus.write_u16(0x2, 0xE7FD).unwrap(); // B -> 0x0
    let mut cpu = CortexM::new();
    cpu.pc = 0x0;
    cpu.sp = 0x8000;
    let mut machine = Machine::new(Box::new(cpu) as Box<dyn Cpu>, bus);
    machine.config.idle_fast_forward_enabled = true;
    machine.bus.legacy_walk_disabled = true;
    machine.run(Some(10)).unwrap();
    assert_eq!(machine.total_cycles, 10);
    if cfg!(feature = "event-scheduler") {
        assert!(
            machine.step_profile().cpu_instructions < 10,
            "boxed CPU path should still leave the batch loop at WFI"
        );
    }
}

/// Exception entry with a stack pointer near zero must WRAP the frame, not
/// panic.
///
/// `frame_ptr = sp.wrapping_sub(32)` already wraps — it always has. The
/// eight stacking stores then computed `frame_ptr + 4 .. + 28` with plain
/// `+`, so a frame pointer that has wrapped past 0 overflowed `u32` on the
/// fifth store. Under `[profile.release] overflow-checks = true` that is a
/// PANIC inside the simulator on perfectly legal guest input: any firmware
/// whose SP has run down past the bottom of its stack (0x10 here) and then
/// takes an exception. Real hardware wraps the address and faults on the
/// access, or writes wherever the wrapped address lands; it does not stop
/// the machine.
#[test]
fn armv7m_exception_entry_wraps_the_stack_frame_instead_of_overflowing() {
    let mut bus = MockBus::new();
    let mut cpu = CortexM::new();
    cpu.pc = 0x1000;
    // A stack pointer 0x10 above zero: the 32-byte frame does not fit
    // below it, so `frame_ptr` wraps to 0xFFFF_FFF0.
    cpu.sp = 0x10;
    cpu.r0 = 0xA0A0_A0A0;
    cpu.r12 = 0xCCCC_CCCC;
    // PendSV (exception 14): priority 0 from SHPR3, no NVIC ISPR to consult.
    cpu.pending_exceptions[0] = 1 << 14;
    // Vector table entry for PendSV.
    bus.write_u32(0x38, 0x2001).unwrap();

    let config = bus.config.clone();
    cpu.step_internal(&mut bus, &[], &config).unwrap();

    assert_eq!(
        cpu.msp, 0xFFFF_FFF0,
        "frame pointer must wrap, not saturate"
    );
    // R0 lands below the wrap, R12 lands above it: 0xFFFF_FFF0 + 16 == 0.
    assert_eq!(bus.read_u32(0xFFFF_FFF0).unwrap(), 0xA0A0_A0A0);
    assert_eq!(bus.read_u32(0x0000_0000).unwrap(), 0xCCCC_CCCC);
    assert_eq!(cpu.pc, 0x2000, "PendSV handler must be entered");
}

/// The matching unstacking path: exception return from a frame whose
/// pointer is near the top of the address space.
///
/// `frame_ptr + 4 .. + 32` had the same plain `+`. The stack pointer being
/// restored is whatever the guest put in MSP/PSP, so this is reachable
/// from a single `MSR MSP, Rn` — or from the wrapped frame the entry path
/// above leaves behind.
#[test]
fn armv7m_exception_return_wraps_the_stack_frame_instead_of_overflowing() {
    let mut bus = MockBus::new();
    let mut cpu = CortexM::new();
    cpu.active_exception = 14;
    cpu.sp = 0xFFFF_FFF0;
    bus.write_u32(0xFFFF_FFF0, 0x1111_1111).unwrap(); // r0
    bus.write_u32(0x0000_0000, 0x2222_2222).unwrap(); // r12, after the wrap
    bus.write_u32(0x0000_0008, 0x0000_3001).unwrap(); // stacked PC

    // 0xFFFF_FFF9 = return to Thread mode on MSP.
    cpu.exception_return(0xFFFF_FFF9, &mut bus).unwrap();

    assert_eq!(cpu.r0, 0x1111_1111);
    assert_eq!(cpu.r12, 0x2222_2222);
    assert_eq!(cpu.pc, 0x0000_3000);
    assert_eq!(cpu.msp, 0x0000_0010, "SP must advance by 32 with a wrap");
}

/// A branch executed from the top of the low half of the address space.
///
/// The target was computed as `(self.pc as i32 + 4 + offset) as u32`. At
/// PC 0x7FFF_FFFC the `+ 4` alone overflows `i32`, so the instruction
/// panicked before the offset was even applied. 0x6000_0000-0x9FFF_FFFF is
/// ordinary executable external RAM in the ARMv7-M memory map, so this is
/// legal guest code, and the ARM result is the wrapped 32-bit address.
#[test]
fn armv7m_branch_wraps_at_the_signed_pc_boundary() {
    for (name, encoding, pc, expected) in [
        // B #0 at the i32 boundary: 0x7FFF_FFFC + 4 + 0.
        ("b", 0xE000u16, 0x7FFF_FFFCu32, 0x8000_0000u32),
        // BEQ #0 with Z set, same boundary.
        ("beq", 0xD000u16, 0x7FFF_FFFCu32, 0x8000_0000u32),
    ] {
        let mut bus = MockBus::new();
        let mut cpu = CortexM::new();
        cpu.pc = pc;
        cpu.xpsr |= 1 << 30; // Z = 1, so the conditional branch is taken.
        bus.write_u16(pc as u64, encoding).unwrap();
        let config = bus.config.clone();
        cpu.step_internal(&mut bus, &[], &config).unwrap();
        assert_eq!(cpu.pc, expected, "{name} must wrap to {expected:#010x}");
    }
}

#[test]
fn test_vfp_fpscr_fz_flushes_denormal_inputs_and_results() {
    // 2^-64 and 2^-85 are both normal; their product is exactly 2^-149,
    // the smallest positive denormal (0x0000_0001).
    let two_pow_m64 = 0x1F80_0000u32;
    let two_pow_m85 = 0x1500_0000u32;
    // Actual denormal operands: 2^-149 and 2^-148.
    let denorm_min = 0x0000_0001u32;
    let denorm_two = 0x0000_0002u32;

    // FZ off: denormal inputs survive and the denormal result is exact.
    assert_eq!(
        vfp_binop(VfpBinOp::Mul, two_pow_m64, two_pow_m85, 0),
        0x0000_0001,
        "2^-64 * 2^-85 = 2^-149 (denormal) with FZ off"
    );
    // FZ on: denormal result flushed to +0.
    assert_eq!(
        vfp_binop(VfpBinOp::Mul, two_pow_m64, two_pow_m85, FPSCR_FZ),
        0x0000_0000,
        "denormal result flushes to zero under FZ"
    );
    // FZ off: denormal inputs add exactly.
    assert_eq!(
        vfp_binop(VfpBinOp::Add, denorm_min, denorm_two, 0),
        0x0000_0003,
        "2^-149 + 2^-148 with FZ off"
    );
    // FZ on: denormal *inputs* flush before the op, so the sum is +0
    // rather than 0x0000_0003.
    assert_eq!(
        vfp_binop(VfpBinOp::Add, denorm_min, denorm_two, FPSCR_FZ),
        0x0000_0000,
        "denormal inputs flush before the add under FZ"
    );
    // The flush keeps the operand's sign.
    assert_eq!(
        vfp_binop(
            VfpBinOp::Add,
            denorm_min | 0x8000_0000,
            denorm_two | 0x8000_0000,
            FPSCR_FZ
        ),
        0x8000_0000,
        "flushed denormals keep their sign"
    );
    // Normal operands/results are untouched by FZ.
    assert_eq!(
        vfp_binop(
            VfpBinOp::Add,
            (1.5f32).to_bits(),
            (2.25f32).to_bits(),
            FPSCR_FZ
        ),
        (3.75f32).to_bits()
    );
}

#[test]
fn test_vfp_fpscr_dn_and_nan_payload_canonicalization() {
    let qnan_aa = 0x7FC0_AAAAu32;
    let qnan_bb = 0x7FC0_BBBBu32;
    let snan = 0x7F80_0001u32;
    let one = (1.0f32).to_bits();

    // Default: the first NaN operand wins, quieted, payload preserved.
    assert_eq!(
        vfp_binop(VfpBinOp::Add, qnan_aa, qnan_bb, 0),
        qnan_aa,
        "first NaN operand propagates"
    );
    assert_eq!(
        vfp_binop(VfpBinOp::Add, qnan_bb, qnan_aa, 0),
        qnan_bb,
        "operand order decides, not the payload value"
    );
    assert_eq!(
        vfp_binop(VfpBinOp::Mul, snan, one, 0),
        0x7FC0_0001,
        "a signaling NaN is quieted, payload preserved"
    );
    assert_eq!(
        vfp_binop(VfpBinOp::Sub, one, snan, 0),
        0x7FC0_0001,
        "the second operand propagates when the first is not NaN"
    );

    // DN: every NaN result becomes the ARM default NaN.
    assert_eq!(
        vfp_binop(VfpBinOp::Add, qnan_aa, qnan_bb, FPSCR_DN),
        VFP_DEFAULT_NAN
    );
    assert_eq!(
        vfp_binop(VfpBinOp::Mul, snan, one, FPSCR_DN),
        VFP_DEFAULT_NAN
    );

    // Invalid operation with no NaN input: default quiet NaN (sign
    // clear). Host FPUs may synthesize 0xFFC0_0000 here; the model must
    // not leak that.
    assert_eq!(
        vfp_binop(
            VfpBinOp::Mul,
            (0.0f32).to_bits(),
            f32::INFINITY.to_bits(),
            0
        ),
        VFP_DEFAULT_NAN,
        "0 * inf is an invalid op with no NaN operand"
    );
    assert_eq!(
        vfp_binop(
            VfpBinOp::Sub,
            f32::INFINITY.to_bits(),
            f32::INFINITY.to_bits(),
            0
        ),
        VFP_DEFAULT_NAN,
        "inf - inf is an invalid op with no NaN operand"
    );
}

#[test]
fn test_vfp_fma_honors_fz_and_dn() {
    // Normal operands whose fused product is the smallest denormal.
    let two_pow_m64 = 0x1F80_0000u32;
    let two_pow_m85 = 0x1500_0000u32;
    let one = (1.0f32).to_bits();

    // Fused (2^-64 * 2^-85) + 0 = 2^-149 with FZ off.
    assert_eq!(
        vfp_fma(two_pow_m64, two_pow_m85, 0, false, false, 0),
        0x0000_0001
    );
    // FZ on: the denormal product flushes to zero before the addend.
    assert_eq!(
        vfp_fma(two_pow_m64, two_pow_m85, 0, false, false, FPSCR_FZ),
        0x0000_0000
    );
    // DN on: NaN operand result is the default NaN.
    assert_eq!(
        vfp_fma(0x7FC0_1234, one, one, false, false, FPSCR_DN),
        VFP_DEFAULT_NAN
    );
}

#[test]
fn test_thumb2_vfp_fpscr_modes_apply_to_instructions() {
    // VADD.F32 S2, S0, S1 with FPSCR.FZ/DN written straight into the
    // core state. Pins the interpreter wiring, not just the helper.
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.fpu_s[0] = 0x0000_0001; // 2^-149 (denormal)
    cpu.fpu_s[1] = 0x0000_0002; // 2^-148 (denormal)
    run_test_instr(
        &mut cpu,
        &mut bus,
        vfp_arith_encoding(0xEE30, 2, 0, 1, 0),
        true,
    );
    assert_eq!(
        cpu.fpu_s[2], 0x0000_0003,
        "VADD without FZ: exact denormal sum"
    );

    cpu.fpu_s[2] = 0;
    cpu.fpscr = FPSCR_FZ;
    run_test_instr(
        &mut cpu,
        &mut bus,
        vfp_arith_encoding(0xEE30, 2, 0, 1, 0),
        true,
    );
    assert_eq!(cpu.fpu_s[2], 0, "VADD with FZ: denormal inputs flush");

    cpu.fpu_s[0] = 0x7FC0_AAAA;
    cpu.fpu_s[1] = (1.0f32).to_bits();
    cpu.fpscr = FPSCR_DN;
    run_test_instr(
        &mut cpu,
        &mut bus,
        vfp_arith_encoding(0xEE30, 2, 0, 1, 0),
        true,
    );
    assert_eq!(cpu.fpu_s[2], VFP_DEFAULT_NAN, "VADD with DN: default NaN");
}
