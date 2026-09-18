use super::*;
use crate::bus::SystemBus;
use crate::DebugControl;
use crate::Machine;

/// CSRRW x0, ustatus (0x000), x0: emitted by the ESP32-C3 ROM before
/// mtvec is initialized.
const CLEAR_USTATUS: u32 = 0x0000_1073;
const C3_ROM_INIT_CSR_800: u32 = 0x8002_9073;
const C3_ROM_INIT_CSR_801: u32 = 0x8012_9073;
const READ_MHARTID_X7: u32 = 0xf140_23f3;
const WRITE_MHARTID_X5: u32 = 0xf142_9073;

#[test]
fn test_riscv_addi() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    // ADDI x1, x0, 5  (x1 = 0 + 5)
    // Op=0x13, rd=1, funct3=0, rs1=0, imm=5
    // 000000000101 00000 000 00001 0010011 -> 0x00500093
    bus.flash.data = vec![
        0x93, 0x00, 0x50, 0x00, // ADDI x1, x0, 5
    ];

    cpu.pc = 0x0000_0000;
    let mut machine = Machine::new(cpu, bus);
    machine.step().unwrap();

    assert_eq!(machine.cpu.read_reg(1), 5);
    assert_eq!(machine.cpu.pc, 4);
}

#[test]
fn esp32c3_rejects_standard_cycle_csr_but_exposes_pccr_machine() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    // CSRRS x5, x0, cycle (0xC00): standard RISC-V cycle CSR is not
    // implemented by the ESP32-C3 core; it must raise illegal instruction.
    let read_standard_cycle = (0xC00u32 << 20) | (5 << 7) | (0b010 << 12) | 0x73;
    bus.flash.data = read_standard_cycle.to_le_bytes().to_vec();
    cpu.pc = 0;
    cpu.mtvec = 0x100;
    let mut machine = Machine::new(cpu, bus);
    machine.step().unwrap();

    assert_eq!(
        machine.cpu.mcause, 2,
        "unsupported CSR must trap as illegal instruction"
    );
    assert_eq!(machine.cpu.mtval, read_standard_cycle);
    assert_eq!(machine.cpu.pc, 0x100);
    assert_eq!(
        machine.cpu.read_csr(0x7E2),
        Some(0),
        "C3 PCCR_MACHINE remains implemented"
    );
}

#[test]
fn esp32c3_accepts_rom_ustatus_clear_before_mtvec_initialization() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new_for(RiscVCoreProfile::Esp32C3);
    bus.flash.data = CLEAR_USTATUS.to_le_bytes().to_vec();
    cpu.pc = 0;
    let mut machine = Machine::new(cpu, bus);

    machine.step().unwrap();

    assert_eq!(machine.cpu.pc, 4, "ustatus write must not trap");
    assert_eq!(machine.cpu.mcause, 0);
    assert_eq!(machine.cpu.mtval, 0);
    assert_eq!(machine.cpu.read_csr(0x000), Some(0));
}

#[test]
fn standard_rv32_rejects_esp32c3_rom_ustatus_clear() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new_for(RiscVCoreProfile::StandardRv32);
    bus.flash.data = CLEAR_USTATUS.to_le_bytes().to_vec();
    cpu.pc = 0;
    cpu.mtvec = 0x100;
    let mut machine = Machine::new(cpu, bus);

    machine.step().unwrap();

    assert_eq!(machine.cpu.mcause, 2);
    assert_eq!(machine.cpu.mtval, CLEAR_USTATUS);
    assert_eq!(machine.cpu.pc, 0x100);
}

#[test]
fn esp32c3_accepts_exact_rom_init_csrs_before_mtvec_initialization() {
    const ROM_PC: u32 = 0x4000_1ea4;
    let mut bus = SystemBus::new();
    bus.flash = crate::memory::LinearMemory::new(12, ROM_PC as u64);
    bus.flash.data = vec![
        0x85, 0x42, // c.li x5, 1
        0x73, 0x90, 0x02, 0x80, // csrrw x0, 0x800, x5
        0x85, 0x42, // c.li x5, 1
        0x73, 0x90, 0x12, 0x80, // csrrw x0, 0x801, x5
    ];
    let mut cpu = RiscV::new_for(RiscVCoreProfile::Esp32C3);
    cpu.pc = ROM_PC;
    let mut machine = Machine::new(cpu, bus);

    machine.step().unwrap();
    assert_eq!(machine.cpu.pc, 0x4000_1ea6);
    assert_eq!(machine.cpu.read_reg(5), 1);
    machine.step().unwrap();
    assert_eq!(machine.cpu.pc, 0x4000_1eaa);
    assert_eq!(machine.cpu.read_reg(0), 0);
    machine.step().unwrap();
    assert_eq!(machine.cpu.pc, 0x4000_1eac);
    machine.step().unwrap();
    assert_eq!(machine.cpu.pc, 0x4000_1eb0);
    assert_eq!(machine.cpu.read_reg(0), 0);
    assert_eq!(machine.cpu.mcause, 0);
    assert_eq!(machine.cpu.mtval, 0);

    for csr in [0x800, 0x801] {
        assert_eq!(machine.cpu.read_csr(csr), Some(0));
        assert!(machine.cpu.write_csr(csr, u32::MAX));
        assert_eq!(machine.cpu.read_csr(csr), Some(0));
    }
}

#[test]
fn c3_rom_init_csrs_do_not_widen_custom_csr_acceptance() {
    let cases = [
        (RiscVCoreProfile::Esp32C3, 0x7ff2_9073, 0x7ff),
        (RiscVCoreProfile::Esp32C3, 0x8062_9073, 0x806),
        (RiscVCoreProfile::StandardRv32, C3_ROM_INIT_CSR_800, 0x800),
        (RiscVCoreProfile::StandardRv32, C3_ROM_INIT_CSR_801, 0x801),
    ];

    for (profile, opcode, csr) in cases {
        let mut bus = SystemBus::new();
        bus.flash.data = opcode.to_le_bytes().to_vec();
        let mut cpu = RiscV::new_for(profile);
        cpu.pc = 0;
        cpu.mtvec = 0x100;
        cpu.x[5] = 1;
        let mut machine = Machine::new(cpu, bus);

        machine.step().unwrap();

        assert_eq!(machine.cpu.mcause, 2, "profile={profile:?} csr={csr:#x}");
        assert_eq!(machine.cpu.mtval, opcode);
        assert_eq!(machine.cpu.pc, 0x100);
        assert_eq!(machine.cpu.read_csr(csr), None);
    }
}

#[test]
fn standard_rv32_profile_exposes_cycle_csr() {
    let cpu = RiscV::new_for(RiscVCoreProfile::StandardRv32);
    assert_eq!(cpu.read_csr(0xC00), Some(0));
    assert_eq!(cpu.read_csr(0x7E2), None);
    assert_eq!(cpu.read_csr(0x000), None);
}

#[test]
fn mhartid_reads_zero_for_all_riscv_profiles() {
    for profile in [RiscVCoreProfile::StandardRv32, RiscVCoreProfile::Esp32C3] {
        let mut bus = SystemBus::new();
        bus.flash.data = READ_MHARTID_X7.to_le_bytes().to_vec();
        let mut cpu = RiscV::new_for(profile);
        cpu.pc = 0;
        let mut machine = Machine::new(cpu, bus);

        machine.step().unwrap();

        assert_eq!(machine.cpu.pc, 4, "profile={profile:?}");
        assert_eq!(machine.cpu.read_reg(7), 0, "profile={profile:?}");
        assert_eq!(machine.cpu.mcause, 0, "profile={profile:?}");
        assert_eq!(machine.cpu.mtval, 0, "profile={profile:?}");
    }
}

#[test]
fn mhartid_write_traps_for_all_riscv_profiles() {
    for profile in [RiscVCoreProfile::StandardRv32, RiscVCoreProfile::Esp32C3] {
        let mut bus = SystemBus::new();
        bus.flash.data = WRITE_MHARTID_X5.to_le_bytes().to_vec();
        let mut cpu = RiscV::new_for(profile);
        cpu.pc = 0;
        cpu.mtvec = 0x100;
        cpu.x[5] = 1;
        let mut machine = Machine::new(cpu, bus);
        assert_eq!(machine.cpu.read_csr(0xF14), Some(0));

        machine.step().unwrap();

        assert_eq!(machine.cpu.mcause, 2, "profile={profile:?}");
        assert_eq!(machine.cpu.mtval, WRITE_MHARTID_X5, "profile={profile:?}");
        assert_eq!(machine.cpu.pc, 0x100, "profile={profile:?}");
    }
}

#[test]
fn test_riscv_beq_taken() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    // 1. ADDI x1, x0, 10
    // 2. ADDI x2, x0, 10
    // 3. BEQ x1, x2, +8 (skip next instruction)
    // 4. ADDI x3, x0, 1 (should be skipped)
    // 5. ADDI x4, x0, 1 (target)

    // imm for BEQ +8:
    // 0x00000063 (BEQ x0, x0, 0)
    // imm[12]=0, imm[10:5]=0, imm[4:1]=4 (bit 3), imm[11]=0
    // offset = 8. binary: 1000.
    // imm[12] = 0
    // imm[11] = 0
    // imm[10:5] = 000000
    // imm[4:1] = 0100 (4)
    // opcode = 1100011 (0x63)
    // funct3 = 000
    // rs1 = 1, rs2 = 2

    // BEQ x1, x2, 8 -> 0x00208463
    // 0000 0000 0010 0000 1000 0100 0110 0011 -> 0x00208463 ?
    // imm[12]=0, imm[10:5]=000000.
    // imm[4:1]=0100. bit 3 is set.
    // imm[11]=0.
    // Verify encoding: https://luplab.gitlab.io/rvcodecjs/#q=beq%20x1,x2,8
    // 00208463

    bus.flash.data = vec![
        0x93, 0x00, 0xA0, 0x00, // ADDI x1, x0, 10 (0x00A00093)
        0x13, 0x01, 0xA0, 0x00, // ADDI x2, x0, 10 (0x00A00113) - wait, rs1=0.
        // ADDI x2, x0, 10: imm=10, rs1=0, funct3=0, rd=2, opcode=0x13
        // 000000001010 00000 000 00010 0010011 -> 0x00A00113. Correct.

        // BEQ x1, x2, 8
        // 0000000 00010 00001 000 01000 1100011 -> 0x00208463
        // imm[12]=0, imm[10:5]=0, rs2=2, rs1=1, funct3=0, imm[4:1]=0100 (+8?), imm[11]=0, opcode=0x63.
        // imm[4:1]=4 -> bit 3 is 1? No, imm[4:1] bits are at positions 11-8.
        // imm[4:1] = 0100 means bit 3 is 1. Yes 1<<3 = 8.
        0x63, 0x84, 0x20, 0x00,
        // Should be skipped (PC+4 from BEQ = 12. BEQ target = 8 + 8 = 16. Wait. PC of BEQ is 8. Target = 8+8=16.)
        // Offset is from current PC.
        // 0: ADDI x1
        // 4: ADDI x2
        // 8: BEQ
        // 12: ADDI x3 (skipped)
        // 16: ADDI x4 (target)
        0x13, 0x01, 0x10, 0x00, // ADDI x3, x0, 1 (0x00100193) - wait this is ADDI x3, x0, 1.
        0x13, 0x02, 0x10, 0x00, // ADDI x4, x0, 1 (0x00100213).
    ];

    cpu.pc = 0x0000_0000;
    let mut machine = Machine::new(cpu, bus);

    // Step 1: x1 = 10
    machine.step().unwrap();
    assert_eq!(machine.cpu.read_reg(1), 10);

    // Step 2: x2 = 10
    machine.step().unwrap();
    assert_eq!(machine.cpu.read_reg(2), 10);

    // Step 3: BEQ taken -> PC = 8 + 8 = 16
    assert_eq!(machine.cpu.pc, 8);
    machine.step().unwrap();
    assert_eq!(machine.cpu.pc, 16);

    // Step 4: ADDI x4, x0, 1
    machine.step().unwrap();
    assert_eq!(machine.cpu.read_reg(4), 1);

    // Ensure x3 is still 0
    assert_eq!(machine.cpu.read_reg(3), 0);
}

#[test]
fn test_riscv_timer_interrupt() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();

    cpu.mtvec = 0x2000;
    cpu.mie = 1 << 7; // MTIE
    cpu.mstatus = 1 << 3; // MIE
    cpu.mtimecmp = 5;

    // Reset memory to hold our test program
    bus.flash.data = vec![0; 0x3000];
    // 0x0: JAL x0, 0 (Infinite loop)
    bus.write_u32(0x0, 0x0000006F).unwrap();
    // 0x2000: ADDI x10, x10, 1
    bus.write_u32(0x2000, 0x00150513).unwrap();
    // 0x2004: MRET
    bus.write_u32(0x2004, 0x30200073).unwrap();

    cpu.pc = 0x0;
    let mut machine = Machine::new(cpu, bus);

    // Step 1-4: mtime increases from 0->1, 1->2, 2->3, 3->4. No interrupt yet.
    for i in 0..4 {
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, 0, "Should be in loop at step {}", i);
    }

    // Step 5: mtime becomes 5, which equals mtimecmp. Trap should be taken.
    machine.step().unwrap();
    assert_eq!(machine.cpu.pc, 0x2000, "Trap should jump to mtvec");

    // Step 6: Execute ISR ADDI x10, x10, 1
    machine.step().unwrap();
    assert_eq!(machine.cpu.read_reg(10), 1);
    assert_eq!(machine.cpu.pc, 0x2004);

    // Step 7: Execute MRET
    machine.step().unwrap();
    assert_eq!(machine.cpu.pc, 0, "MRET should return to 0x0");
    assert!(
        (machine.cpu.mstatus & (1 << 3)) != 0,
        "MIE should be re-enabled"
    );
}

#[test]
fn test_riscv_async_irq_mepc_points_to_next_instruction() {
    // Regression test for the "doubled side effect" bug. Prior to fix,
    // an async IRQ taken at a non-branch instruction stored self.pc
    // (= the just-executed PC) into mepc. MRET then re-executed that
    // instruction, incrementing the ADDI counter twice. Branch
    // instructions (like JAL-to-self) accidentally masked the bug
    // because next_pc == self.pc for self-loops.
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();

    cpu.mtvec = 0x2000;
    cpu.mie = 1 << 7; // MTIE
    cpu.mstatus = 1 << 3; // MIE
    cpu.mtimecmp = 3;

    bus.flash.data = vec![0; 0x3000];
    // Straight-line code: every step advances PC by 4, next_pc != pc.
    // 0x0:  ADDI x10, x10, 1     ; x10 = 1
    // 0x4:  ADDI x10, x10, 1     ; x10 = 2, mtime hits mtimecmp here
    // 0x8:  ADDI x10, x10, 1     ; would make x10 = 3 but interrupt intervenes
    bus.write_u32(0x0, 0x00150513).unwrap();
    bus.write_u32(0x4, 0x00150513).unwrap();
    bus.write_u32(0x8, 0x00150513).unwrap();
    // ISR at 0x2000: just MRET so we can observe mepc on return.
    bus.write_u32(0x2000, 0x30200073).unwrap();

    cpu.pc = 0x0;
    let mut machine = Machine::new(cpu, bus);

    machine.step().unwrap(); // PC 0x0 -> 0x4, x10 = 1, mtime = 1
    machine.step().unwrap(); // PC 0x4 -> 0x8, x10 = 2, mtime = 2
                             // This step executes 0x8 (x10 = 3, next_pc = 0xC). Then mtime -> 3
                             // hits mtimecmp and the trap fires. mepc must be saved as 0xC,
                             // not 0x8, so MRET doesn't re-execute the ADDI.
    machine.step().unwrap();
    assert_eq!(machine.cpu.read_reg(10), 3, "third ADDI executed once");
    assert_eq!(machine.cpu.pc, 0x2000, "trapped into ISR");
    assert_eq!(
        machine.cpu.mepc, 0xC,
        "mepc must be the address of the next instruction, not the one we just finished"
    );

    // Clear MTIP so the next step doesn't re-trap.
    machine.cpu.mip &= !(1 << 7);
    machine.cpu.mtimecmp = u64::MAX;

    machine.step().unwrap(); // MRET: jump to mepc
    assert_eq!(machine.cpu.pc, 0xC, "MRET returned to mepc");
    // Not re-executing the ADDI at 0x8 is what we really care about:
    assert_eq!(
        machine.cpu.read_reg(10),
        3,
        "ADDI at 0x8 must not be re-executed (would read 4 if bug present)"
    );
}

#[test]
fn test_riscv_external_irq_line_vectored_and_mret_restores_mie() {
    // ESP32-C3-style external interrupt: the bus drives a CPU interrupt
    // line (1..31) via `external_irq_lines()`; with vectored mtvec the core
    // traps to base + line*4, and MRET restores MIE from MPIE.
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    cpu.mtvec = 0x2000 | 1; // vectored
    cpu.mstatus = 1 << 3; // MIE
    cpu.mtimecmp = u64::MAX; // no CLINT timer interference

    bus.flash.data = vec![0; 0x3000];
    // NOP loop at 0x0 (ADDI x0,x0,0) and the per-line handlers are NOPs+MRET.
    bus.write_u32(0x0, 0x00000013).unwrap();
    bus.write_u32(0x4, 0x00000013).unwrap();
    // Line 5 vector (0x2000 + 5*4 = 0x2014): MRET.
    bus.write_u32(0x2014, 0x30200073).unwrap();
    // Assert external line 5 (irq_fabric.esp32c3.routing stays false, so the C3
    // aggregation leaves irq_fabric.esp32c3.irq_lines untouched between ticks).
    bus.irq_fabric.esp32c3.irq_lines = 1 << 5;

    cpu.pc = 0x0;
    let mut machine = Machine::new(cpu, bus);
    // First step executes the NOP at 0x0, then takes the pending line-5 trap.
    machine.step().unwrap();
    assert_eq!(
        machine.cpu.pc, 0x2014,
        "vectored trap must jump to mtvec base + line*4"
    );
    assert_eq!(
        machine.cpu.mcause,
        0x8000_0000 | 5,
        "mcause = interrupt|line"
    );
    assert_eq!(machine.cpu.mstatus & (1 << 3), 0, "MIE cleared on trap");
    assert_ne!(machine.cpu.mstatus & (1 << 7), 0, "MPIE holds prior MIE");
    // Drop the line so MRET doesn't immediately re-trap, then MRET.
    machine.bus.irq_fabric.esp32c3.irq_lines = 0;
    machine.step().unwrap();
    assert_ne!(
        machine.cpu.mstatus & (1 << 3),
        0,
        "MRET restores MIE from MPIE"
    );
}

#[test]
fn test_riscv_wfi_is_nop() {
    // WFI must decode and execute as a no-op (PC advances by 4); the idle
    // task's WFI spin relies on this.
    assert_eq!(
        crate::decoder::riscv::decode_rv32(0x1050_0073),
        crate::decoder::riscv::Instruction::Wfi
    );
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    bus.flash.data = vec![0; 0x100];
    bus.write_u32(0x0, 0x1050_0073).unwrap(); // WFI
    cpu.pc = 0x0;
    let mut machine = Machine::new(cpu, bus);
    machine.step().unwrap();
    assert_eq!(machine.cpu.pc, 0x4, "WFI advances PC like a NOP");
}

#[test]
fn test_riscv_wfi_fast_forward_is_off_by_default() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    bus.flash.data = vec![0; 0x100];
    bus.write_u32(0x0, 0x1050_0073).unwrap(); // WFI
    bus.write_u32(0x4, 0xffdf_f06f).unwrap(); // JAL x0, -4
    cpu.pc = 0x0;
    cpu.mtimecmp = u64::MAX;

    let mut machine = Machine::new(cpu, bus);
    machine.bus.legacy_walk_disabled = true;
    machine.run(Some(10)).unwrap();

    assert_eq!(machine.total_cycles, 10);
    assert_eq!(machine.step_profile().cpu_instructions, 10);
}

#[test]
fn test_riscv_wfi_fast_forward_skips_cpu_work_when_enabled() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    bus.flash.data = vec![0; 0x100];
    bus.write_u32(0x0, 0x1050_0073).unwrap(); // WFI
    bus.write_u32(0x4, 0xffdf_f06f).unwrap(); // JAL x0, -4
    cpu.pc = 0x0;
    cpu.mtimecmp = u64::MAX;

    let mut machine = Machine::new(cpu, bus);
    machine.config.idle_fast_forward_enabled = true;
    machine.bus.legacy_walk_disabled = true;
    machine.run(Some(10)).unwrap();

    assert_eq!(machine.total_cycles, 10);
    if cfg!(feature = "event-scheduler") {
        assert!(
            machine.idle_fast_forward_cycles_skipped > 0,
            "idle FF counter must rise when WFI skip fires"
        );
        assert!(
            machine.step_profile().cpu_instructions < 10,
            "fast-forwarded cycles should not retire CPU instructions"
        );
    } else {
        // Without the event scheduler, idle fast-forward intentionally remains inactive.
        assert_eq!(machine.idle_fast_forward_cycles_skipped, 0);
        assert_eq!(machine.step_profile().cpu_instructions, 10);
    }
}

#[test]
fn boxed_riscv_batch_preserves_idle_fast_forward_escape() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    bus.flash.data = vec![0; 0x100];
    bus.write_u32(0x0, 0x1050_0073).unwrap(); // WFI
    bus.write_u32(0x4, 0xffdf_f06f).unwrap(); // JAL x0, -4
    cpu.pc = 0x0;
    cpu.mtimecmp = u64::MAX;

    let mut machine = Machine::new(Box::new(cpu) as Box<dyn Cpu>, bus);
    machine.config.idle_fast_forward_enabled = true;
    machine.bus.legacy_walk_disabled = true;
    machine.run(Some(10)).unwrap();

    assert_eq!(machine.total_cycles, 10);
    if cfg!(feature = "event-scheduler") {
        assert!(
                machine.step_profile().cpu_instructions < 10,
                "boxed C3 CPU path should still leave the batch loop at WFI so Machine can fast-forward"
            );
        assert!(machine.idle_fast_forward_cycles_skipped > 0);
    } else {
        assert_eq!(machine.step_profile().cpu_instructions, 10);
        assert_eq!(machine.idle_fast_forward_cycles_skipped, 0);
    }
}

/// Regression for the frozen ESP32-C3 watch/clock demos: a firmware idle
/// loop that busy-polls the SYSTIMER (`millis()`) **while also sampling a
/// GPIO button** (`digitalRead`) must still let idle fast-forward coalesce
/// the spin. Before the GPIO `mmio_access_class` fix, the button read booked
/// a side-effecting MMIO access every iteration, so
/// `take_timer_poll_coalesce_eligible()` returned false and idle FF never
/// engaged — the wait was simulated cycle-by-cycle and the guest's time
/// source crawled (hundreds of millions of real cycles per device-second).
///
/// This drives the exact shape (SYSTIMER OP-update + VALUE read + GPIO IN
/// read, no WFI, no armed alarm) through `Machine::advance` and asserts BOTH
/// that the skip engages AND that the SYSTIMER counter still tracks the
/// advanced cycle (single source of truth: it derives from `current_cycle`).
#[test]
fn c3_systimer_busy_poll_with_gpio_read_still_idle_fast_forwards() {
    // RV32 encoders.
    fn lui(rd: u32, imm20: u32) -> u32 {
        (imm20 << 12) | (rd << 7) | 0x37
    }
    fn sw(rs2: u32, imm: i32, rs1: u32) -> u32 {
        let imm = imm as u32;
        ((imm >> 5 & 0x7f) << 25)
            | (rs2 << 20)
            | (rs1 << 15)
            | (0b010 << 12)
            | ((imm & 0x1f) << 7)
            | 0x23
    }
    fn lw(rd: u32, imm: i32, rs1: u32) -> u32 {
        let imm = imm as u32;
        ((imm & 0xfff) << 20) | (rs1 << 15) | (0b010 << 12) | (rd << 7) | 0x03
    }
    fn jal(rd: u32, off: i32) -> u32 {
        let o = off as u32;
        ((o >> 20 & 1) << 31)
            | ((o >> 1 & 0x3ff) << 21)
            | ((o >> 11 & 1) << 20)
            | ((o >> 12 & 0xff) << 12)
            | (rd << 7)
            | 0x6f
    }

    // SYSTIMER at 0x6002_3000 (source 37, C3), GPIO at 0x6000_4000.
    const SYSTIMER_BASE: u64 = 0x6002_3000;
    const GPIO_BASE: u64 = 0x6000_4000;
    const OP: u64 = 0x04; // UNIT0_OP (write UPDATE bit 30)
    const VALUE_LO: u64 = 0x44; // UNIT0_VALUE_LO
    const GPIO_IN: u64 = 0x3C; // input data register (digitalRead)

    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    bus.flash.data = vec![0; 0x100];

    // Program:
    //   LUI  x5, 0x60023      ; SYSTIMER base
    //   LUI  x6, 0x60004      ; GPIO base
    //   LUI  x7, 0x40000      ; UPDATE bit (1<<30)
    // L: SW   x7, 4(x5)       ; UNIT0_OP <- UPDATE   (freerunning poll)
    //   LW   x8, 0x44(x5)     ; VALUE_LO             (freerunning poll)
    //   LW   x9, 0x3C(x6)     ; GPIO IN              (button sample)
    //   JAL  x0, L            ; spin
    let prog = [
        lui(5, 0x60023),
        lui(6, 0x60004),
        lui(7, 0x40000),
        sw(7, OP as i32, 5),
        lw(8, VALUE_LO as i32, 5),
        lw(9, GPIO_IN as i32, 6),
        jal(0, -12),
    ];
    for (i, word) in prog.iter().enumerate() {
        bus.write_u32((i as u64) * 4, *word).unwrap();
    }

    bus.add_peripheral(
        "systimer",
        SYSTIMER_BASE,
        0x100,
        None,
        Box::new(crate::peripherals::esp32s3::systimer::Systimer::new_with_source(160_000_000, 37)),
    );
    bus.add_peripheral(
        "gpio",
        GPIO_BASE,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32c3::gpio::Esp32c3Gpio::new()),
    );

    cpu.pc = 0x0;

    let mut machine = Machine::new(Box::new(cpu) as Box<dyn Cpu>, bus);
    machine.config.idle_fast_forward_enabled = true;
    machine.bus.legacy_walk_disabled = true;
    // Mirror the browser policy (`apply_browser_c3_policy`): the walk-free C3
    // batches at the recommended tick interval. The timer-poll coalesce only
    // sees ≥2 polls per batch when batches are wider than one instruction.
    machine.config.peripheral_tick_interval = crate::bus::RECOMMENDED_TICK_INTERVAL;

    const TARGET: u32 = 2_000_000;
    machine.run(Some(TARGET)).unwrap();
    assert_eq!(machine.total_cycles, u64::from(TARGET));

    // Read the SYSTIMER counter back the way firmware does (UPDATE then
    // read the snapshot). At 160 MHz that is ~10 CPU cycles per 16 MHz tick.
    machine.bus.write_u32(SYSTIMER_BASE + OP, 1 << 30).unwrap();
    let count = machine.bus.read_u32(SYSTIMER_BASE + VALUE_LO).unwrap() as u64;

    if cfg!(feature = "event-scheduler") {
        assert!(
            machine.idle_fast_forward_cycles_skipped > 0,
            "idle FF must coalesce a SYSTIMER busy-poll even when the loop also \
                 samples a GPIO button (skipped = {})",
            machine.idle_fast_forward_cycles_skipped
        );
        assert!(
            machine.step_profile().cpu_instructions < u64::from(TARGET),
            "the spin must be skipped, not fully simulated: retired {} over {} cycles",
            machine.step_profile().cpu_instructions,
            TARGET
        );
        // SYSTIMER count must track the advanced cycle (single source of
        // truth). ~2M cycles / 10 ≈ 200k ticks; allow generous slack.
        assert!(
            count > 100_000,
            "SYSTIMER counter must advance across the skipped window, got {count}"
        );
    } else {
        assert_eq!(machine.idle_fast_forward_cycles_skipped, 0);
    }
}

#[test]
fn test_riscv_wfi_fast_forward_wakes_on_systimer_event() {
    let mut bus = SystemBus::empty();
    let mut cpu = RiscV::new();
    bus.flash.data = vec![0; 0x3000];
    bus.write_u32(0x0, 0x1050_0073).unwrap(); // WFI
    bus.write_u32(0x4, 0xffdf_f06f).unwrap(); // JAL x0, -4
    bus.write_u32(0x2000 + 11 * 4, 0x3020_0073).unwrap(); // MRET at machine external IRQ vector

    bus.add_peripheral(
        "systimer",
        0x6002_3000,
        0x100,
        None,
        Box::new(crate::peripherals::esp32s3::systimer::Systimer::new_with_source(160_000_000, 11)),
    );
    bus.write_u32(0x6002_3064, 1).unwrap(); // INT_ENA TARGET0
    bus.write_u32(0x6002_301C, 0).unwrap(); // TARGET0_HI
    bus.write_u32(0x6002_3020, 3).unwrap(); // TARGET0_LO: 3 SYSTIMER ticks
    bus.write_u32(0x6002_3050, 1).unwrap(); // COMP0_LOAD
    let conf = bus.read_u32(0x6002_3000).unwrap();
    bus.write_u32(0x6002_3000, conf | (1 << 24)).unwrap(); // TARGET0_WORK_EN

    cpu.pc = 0x0;
    cpu.mtvec = 0x2000 | 1; // vectored machine interrupts
    cpu.mie = 1 << 11; // MEIE
    cpu.mstatus = 1 << 3; // MIE
    cpu.mtimecmp = u64::MAX;

    let mut machine = Machine::new(cpu, bus);
    machine.config.idle_fast_forward_enabled = true;
    machine.run(Some(40)).unwrap();

    assert_eq!(machine.total_cycles, 40);
    if cfg!(feature = "event-scheduler") {
        assert!(machine.idle_fast_forward_cycles_skipped > 0);
        assert!(
            machine.step_profile().cpu_instructions < machine.total_cycles,
            "WFI should skip idle cycles until the SYSTIMER event; retired {} over {} cycles",
            machine.step_profile().cpu_instructions,
            machine.total_cycles
        );
    } else {
        assert_eq!(machine.idle_fast_forward_cycles_skipped, 0);
        assert_eq!(
            machine.step_profile().cpu_instructions,
            machine.total_cycles
        );
    }
    assert_ne!(
        machine.cpu.mcause & 0x8000_0000,
        0,
        "the scheduled SYSTIMER event should become an observable interrupt"
    );
}

#[test]
fn test_riscv_rv32a_atomics() {
    // Smoke-test the RV32A word atomics. Build a small program in RAM and
    // run instructions by placing them in flash one at a time via step().
    //
    // Layout: put the memory cell we'll operate on at 0x2000_0010 in RAM.
    // Put the program at flash 0x0.
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    bus.flash.data = vec![0; 0x200];

    // Helper: encode an R-type (opcode=0x2F) atomic. All RV32A word
    // atomics share funct3 = 0b010.
    fn amo(funct5: u32, rs2: u32, rs1: u32, rd: u32) -> u32 {
        (funct5 << 27) | (rs2 << 20) | (rs1 << 15) | (0b010 << 12) | (rd << 7) | 0x2F
    }

    // Program layout in flash:
    // 0x00: LUI   x5, 0x20000       ; x5 = 0x20000000
    // 0x04: ADDI  x5, x5, 0x10      ; x5 = 0x20000010   (our atomic cell)
    // 0x08: ADDI  x6, x0, 7         ; x6 = 7
    // 0x0C: ADDI  x7, x0, 42        ; x7 = 42
    // 0x10: SW    x6, 0(x5)         ; mem[x5] = 7
    // 0x14: LR.W  x8, (x5)          ; x8 = 7 (reservation on x5)
    // 0x18: SC.W  x9, x7, (x5)      ; x9 = 0 (success), mem[x5] = 42
    // 0x1C: AMOADD.W x10, x7, (x5)  ; x10 = 42, mem[x5] = 42 + 42 = 84
    // 0x20: AMOMAX.W x11, x0, (x5)  ; x11 = 84, mem[x5] = max(84, 0) = 84
    // 0x24: SC.W  x12, x7, (x5)     ; x12 = 1 (failure — reservation cleared)
    //
    // LUI rd=5, imm[31:12]=0x20000 — rd = 0x20000000.
    let lui = 0x20000000u32 | (5 << 7) | 0x37;
    // ADDI x5, x5, 0x10 -> imm=0x10 rs1=5 funct3=0 rd=5 op=0x13
    let addi_x5 = (0x10 << 20) | (5 << 15) | (5 << 7) | 0x13;
    // ADDI x6, x0, 7
    let addi_x6_7 = (7 << 20) | (6 << 7) | 0x13;
    // ADDI x7, x0, 42
    let addi_x7_42 = (42 << 20) | (7 << 7) | 0x13;
    // SW x6, 0(x5) -> imm=0, funct3=0b010, rs1=5, rs2=6, op=0x23
    let sw = (6 << 20) | (5 << 15) | (0b010 << 12) | 0x23;
    let lr_w = amo(0x02, 0, 5, 8);
    let sc_w = amo(0x03, 7, 5, 9);
    let amoadd = amo(0x00, 7, 5, 10);
    let amomax = amo(0x14, 0, 5, 11);
    let sc_w_fail = amo(0x03, 7, 5, 12);

    let prog = [
        lui, addi_x5, addi_x6_7, addi_x7_42, sw, lr_w, sc_w, amoadd, amomax, sc_w_fail,
    ];
    for (i, w) in prog.iter().enumerate() {
        bus.write_u32((i as u64) * 4, *w).unwrap();
    }

    cpu.pc = 0;
    let mut machine = Machine::new(cpu, bus);

    for _ in 0..prog.len() {
        machine.step().unwrap();
    }

    // After the whole program:
    assert_eq!(machine.cpu.read_reg(5), 0x20000010, "x5 = cell address");
    assert_eq!(machine.cpu.read_reg(8), 7, "LR.W loaded initial value 7");
    assert_eq!(machine.cpu.read_reg(9), 0, "first SC.W succeeded");
    assert_eq!(
        machine.cpu.read_reg(10),
        42,
        "AMOADD.W returned pre-add value"
    );
    assert_eq!(
        machine.cpu.read_reg(11),
        84,
        "AMOMAX.W returned pre-op value (post-AMOADD result)"
    );
    assert_eq!(
        machine.cpu.read_reg(12),
        1,
        "second SC.W fails: AMOADD invalidated the reservation"
    );
    // Final memory state: 84.
    let final_val = machine.bus.read_u32(0x20000010).unwrap();
    assert_eq!(final_val, 84);
}

#[test]
fn test_riscv_mul() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    // ADDI x1, x0, 10
    // ADDI x2, x0, 5
    // MUL x3, x1, x2 (x3 = 10 * 5 = 50)
    // MUL Opcode: 0x33, funct3: 0, funct7: 0x01, rs1: 1, rs2: 2, rd: 3
    // 0000001 00010 00001 000 00011 0110011 -> 0x022081B3
    bus.flash.data = vec![
        0x93, 0x00, 0xA0, 0x00, // ADDI x1, x0, 10
        0x13, 0x01, 0x50, 0x00, // ADDI x2, x0, 5
        0xB3, 0x81, 0x20, 0x02, // MUL x3, x1, x2
    ];

    cpu.pc = 0x0;
    let mut machine = Machine::new(cpu, bus);
    machine.step().unwrap();
    machine.step().unwrap();
    machine.step().unwrap();

    assert_eq!(machine.cpu.read_reg(3), 50);
    assert_eq!(machine.cpu.pc, 12);
}

#[test]
fn test_riscv_compressed_addi() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    // C.ADDI x1, 5 (x1 = 0 + 5)
    // Op: 01, funct3: 000, rd: 1, imm: 5
    // 000 0 00001 00101 01 -> 0x0085 (Wait, C.ADDI imm is split)
    // inst[15:13]=000, inst[12]=imm[5]=0, inst[11:7]=rd=1, inst[6:2]=imm[4:0]=5
    // 000 0   00001   00101   01 -> 0x0085 (Wait, bitwise: 0000 0000 1001 0101 -> 0x0095?)
    // Let's use rvcodecjs: C.ADDI x1, 5 -> 0x0095
    bus.flash.data = vec![
        0x95, 0x00, // C.ADDI x1, 5
        0x13, 0x02, 0x50, 0x00, // ADDI x4, x0, 5 (for alignment check)
    ];

    cpu.pc = 0x0;
    let mut machine = Machine::new(cpu, bus);
    machine.step().unwrap();

    assert_eq!(machine.cpu.read_reg(1), 5);
    assert_eq!(machine.cpu.pc, 2); // PC should increment by 2

    machine.step().unwrap();
    assert_eq!(machine.cpu.read_reg(4), 5);
    assert_eq!(machine.cpu.pc, 6);
}

#[test]
fn test_riscv_compressed_lw_sw() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    // x8 is used for C.LW/SW (s0/fp).
    // 1. ADDI x8, x0, 0x20000000 (RAM start)
    // 2. ADDI x9, x0, 42
    // 3. C.SW x9, 4(x8)
    // 4. C.LW x10, 4(x8)

    // C.SW x9, 4(x8) -> 0xC044 (Little Endian: 44 C0)
    // C.LW x10, 4(x8) -> 0x4048 (Little Endian: 48 40)

    bus.flash.data = vec![
        0x37, 0x04, 0x00, 0x20, // LUI x8, 0x20000 (x8 = 0x20000000)
        0x93, 0x04, 0xA0, 0x02, // ADDI x9, x0, 42
        0x44, 0xC0, // C.SW x9, 4(x8)
        0x48, 0x40, // C.LW x10, 4(x8)
        0x00, 0x00, 0x00, 0x00, // Padding
    ];

    cpu.pc = 0x0;
    cpu.write_reg(2, 0x20001000); // Initialize SP just in case
    let mut machine = Machine::new(cpu, bus);
    machine.step().unwrap(); // LUI
    machine.step().unwrap(); // ADDI
    machine.step().unwrap(); // C.SW
    machine.step().unwrap(); // C.LW

    assert_eq!(machine.cpu.read_reg(10), 42);
    assert_eq!(machine.cpu.pc, 12); // 4 + 4 + 2 + 2 = 12
}

#[test]
fn riscv_decode_cache_uses_opcode_tag_for_same_pc_changes() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    bus.flash.data = vec![
        0x93, 0x00, 0x10, 0x00, // ADDI x1, x0, 1
    ];

    cpu.pc = 0;
    let mut machine = Machine::new(cpu, bus);
    machine.config.decode_cache_enabled = true;
    let initial_pc = machine.cpu.pc;
    machine.step().unwrap();

    let cache_idx = ((initial_pc >> 1) & 0xFFF) as usize;
    let entry = machine.cpu.decode_cache[cache_idx].expect("first step caches decode");
    assert_eq!(entry.tag, 0);
    assert_eq!(entry.opcode, 0x0010_0093);
    assert_eq!(machine.cpu.read_reg(1), 1);

    machine.bus.flash.data = vec![
        0x93, 0x00, 0x20, 0x00, // ADDI x1, x0, 2
    ];
    machine.cpu.pc = 0;
    machine.step().unwrap();

    let entry = machine.cpu.decode_cache[cache_idx].expect("second step refreshes decode");
    assert_eq!(entry.opcode, 0x0020_0093);
    assert_eq!(machine.cpu.read_reg(1), 2);
}

/// IRAM/`extra_mem` instruction-fetch window: execute from a linear
/// code region that is NOT plain `ram`/`flash`, and confirm a guest
/// store into the window is observed on the next fetch (self-modifying
/// IRAM stays byte-identical to the unwindowed bus path).
#[test]
fn riscv_extra_mem_fetch_window_sees_self_modifying_store() {
    use crate::memory::LinearMemory;

    // IRAM-like base (C3 IRAM is 0x4037_0000); keep it far from flash 0.
    const IRAM: u32 = 0x4037_0000;
    let mut bus = SystemBus::new();
    let mut iram = LinearMemory::new(0x100, IRAM as u64);
    // 0x00: LUI  x6, 0x40370       x6 = IRAM
    // 0x04: ADDI x6, x6, 0x14      x6 -> patch site at IRAM+0x14
    // 0x08: LUI  x5, upper(ADDI x7,x0,1)
    // 0x0c: ADDI x5, x5, low(...)  x5 = encoding of ADDI x7, x0, 1
    // 0x10: SW   x5, 0(x6)         overwrite patch site
    // 0x14: ADDI x7, x0, 99        initially 99; becomes ADDI x7,x0,1
    let addi_x7_1: u32 = 0x0010_0393;
    let addi_x7_99: u32 = 0x0630_0393;
    // LUI rd, imm20: imm20 sits in [31:12].
    let lui_x6 = (0x40370u32 << 12) | (6 << 7) | 0x37; // x6 = 0x4037_0000
                                                       // funct3 = 0 for ADDI is intentional encoding (identity `0 << 12` would trip clippy).
    let addi_x6 = (0x014u32 << 20) | (6 << 15) | (6 << 7) | 0x13; // +0x14
    let lui_x5 = ((addi_x7_1 >> 12) << 12) | (5 << 7) | 0x37;
    let addi_x5 = ((addi_x7_1 & 0xfff) << 20) | (5 << 15) | (5 << 7) | 0x13;
    // SW rs2, 0(rs1): funct3=010, imm=0, opcode=0x23
    let sw_x5 = (5u32 << 20) | (6 << 15) | (0x2 << 12) | 0x23;

    for (i, w) in [lui_x6, addi_x6, lui_x5, addi_x5, sw_x5, addi_x7_99]
        .into_iter()
        .enumerate()
    {
        let off = i * 4;
        iram.data[off..off + 4].copy_from_slice(&w.to_le_bytes());
    }
    bus.extra_mem.push(iram);

    let mut cpu = RiscV::new();
    cpu.pc = IRAM;
    let mut machine = Machine::new(cpu, bus);

    // Execute through SW. The IRAM fetch window arms on first fetch and
    // must be invalidated by the store into the patch site.
    for _ in 0..5 {
        machine.step().unwrap();
    }
    assert_eq!(machine.cpu.pc, IRAM + 0x14);
    machine.step().unwrap();
    assert_eq!(
        machine.cpu.read_reg(7),
        1,
        "self-modifying store into the IRAM fetch window must be visible"
    );
}

/// A vectored trap whose `mtvec` base sits at the top of the address space
/// must WRAP to the vector, not panic.
///
/// `self.pc = base + irq * 4` used plain `+` on `u32`. `mtvec` is written
/// by the guest with `csrw mtvec, rN` and RISC-V only requires the base to
/// be 4-byte aligned, so a base near 0xFFFF_FFFF plus a vectored interrupt
/// overflowed `u32` — a simulator panic on legal guest input. The hardware
/// wraps the address like every other address computation.
#[test]
fn riscv_vectored_trap_wraps_at_the_top_of_the_address_space() {
    let mut cpu = RiscV::new();
    // Vectored mode (mode == 1) with base 0xFFFF_FFF0.
    cpu.mtvec = 0xFFFF_FFF1;
    // Machine timer interrupt: cause = 0x8000_0007, so irq == 7.
    cpu.handle_trap(0x8000_0007, 0x0000_0100);
    assert_eq!(
        cpu.pc, 0x0000_000C,
        "0xFFFF_FFF0 + 7*4 must wrap to 0x0000_000C"
    );
}
