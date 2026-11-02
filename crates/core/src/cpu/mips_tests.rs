// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! MIPS32r2 core unit tests. Every instruction word below was produced by
//! the Zephyr SDK 0.17.0 `mips-zephyr-elf-as -EL -march=mips32r2` (binutils
//! 2.38) from the source in the comment beside it, so the decoder is checked
//! against an independent encoder, not against itself. Expected values
//! follow the MIPS32 ISA (MD00086) and PRA (MD00090) manuals and the PIC32MZ
//! EF data sheet's interrupt controller chapter.

use super::*;
use crate::bus::SystemBus;
use crate::memory::LinearMemory;
use crate::{Bus, Machine};

/// Code runs from the boot flash reset vector (KSEG1 0xBFC00000).
const CODE: u32 = 0xBFC0_0000;
/// Data lives in KSEG1 RAM; physical 0x100.
const DATA: u32 = 0xA000_0100;
const DATA_PA: u64 = 0x100;

fn machine(words: &[u32]) -> Machine<Mips32> {
    let mut bus = SystemBus::new();
    bus.flash_boot_alias = false;
    bus.ram = LinearMemory::new(64 * 1024, 0);
    bus.flash = LinearMemory::new(64 * 1024, 0x1FC0_0000);
    for (i, w) in words.iter().enumerate() {
        bus.flash.data[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    let mut cpu = Mips32::new();
    cpu.set_pc(CODE);
    Machine::new(cpu, bus)
}

fn run(m: &mut Machine<Mips32>, n: usize) {
    for _ in 0..n {
        m.step().unwrap();
    }
}

fn reg(m: &Machine<Mips32>, r: u32) -> u32 {
    m.cpu.reg(r)
}

fn set(m: &mut Machine<Mips32>, r: u32, v: u32) {
    m.cpu.set_reg(r, v);
}

/// Leave the reset state: BEV = 0 (vectors from EBase), ERL = 0.
fn running_kernel(m: &mut Machine<Mips32>) {
    m.cpu.cp0[C0_STATUS] = 0;
    m.cpu.cp0[C0_CAUSE] = CA_IV;
}

const NOP: u32 = 0x0000_0000;

#[test]
fn reset_state_matches_the_m5150_on_pic32mz() {
    let cpu = Mips32::new();
    assert_eq!(cpu.pc, RESET_VECTOR);
    assert_eq!(cpu.status(), ST_BEV | ST_ERL);
    assert_eq!(cpu.read_cp0(15, 1), EBASE_RESET);
    assert_eq!(cpu.read_cp0(12, 2) >> 26 & 0xF, 7, "eight shadow sets");
    assert_ne!(cpu.read_cp0(16, 3) & (1 << 6), 0, "Config3.VEIC");
}

#[test]
fn addi_overflow_traps_and_leaves_rt() {
    // addi v0,v1,1 ; nop
    let mut m = machine(&[0x2062_0001, NOP]);
    set(&mut m, 3, 0x7FFF_FFFF);
    set(&mut m, 2, 0x1234);
    run(&mut m, 1);
    assert_eq!(reg(&m, 2), 0x1234, "rt is not written on overflow");
    assert_eq!((m.cpu.cause() >> 2) & 0x1F, EXC_OV);
    assert_eq!(m.cpu.cp0[C0_EPC], CODE);
    // BEV = 1 after reset: the general vector is 0xBFC00380.
    assert_eq!(m.cpu.pc, 0xBFC0_0380);
}

#[test]
fn delay_slots_branch_likely_and_link() {
    // b +2 ; li a0,7 ; li a1,9 ; beqzl at,+3 ; li a2,5 ; bal +2 ; nop ;
    // nop ; nop ; nop
    let code = [
        0x1000_0002,
        0x2404_0007,
        0x2405_0009,
        0x5020_0003,
        0x2406_0005,
        0x0411_0002,
        NOP,
        NOP,
        NOP,
        NOP,
    ];
    let mut m = machine(&code);
    set(&mut m, 1, 1);
    run(&mut m, 2);
    assert_eq!(reg(&m, 4), 7, "the delay slot of a taken branch runs");
    // The target is relative to the delay slot: +4 + 2*4 skips `li a1,9`.
    assert_eq!(m.cpu.pc, CODE + 0x0C);
    assert_eq!(reg(&m, 5), 0);
    // beqzl at (at = 1, not taken): the delay slot is nullified.
    run(&mut m, 1);
    assert_eq!(m.cpu.pc, CODE + 0x14);
    assert_eq!(reg(&m, 6), 0);
    // bal +2: ra = the address after the delay slot.
    run(&mut m, 2);
    assert_eq!(reg(&m, 31), CODE + 0x1C);
    assert_eq!(m.cpu.pc, CODE + 0x20);
}

#[test]
fn unaligned_word_load_and_store_are_little_endian() {
    // lwr t0,1(t1) ; lwl t0,4(t1) ; swr t0,9(t1) ; swl t0,12(t1)
    let mut m = machine(&[0x9928_0001, 0x8928_0004, 0xB928_0009, 0xA928_000C]);
    set(&mut m, 9, DATA);
    for (i, b) in [0x00u8, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]
        .iter()
        .enumerate()
    {
        m.bus.write_u8(DATA_PA + i as u64, *b).unwrap();
    }
    m.bus.write_u32(DATA_PA + 8, 0xAAAA_AAAA).unwrap();
    m.bus.write_u32(DATA_PA + 12, 0xBBBB_BBBB).unwrap();
    run(&mut m, 2);
    assert_eq!(reg(&m, 8), 0x4433_2211, "the word at DATA+1");
    run(&mut m, 2);
    // The same word stored at DATA+9.
    assert_eq!(m.bus.read_u32(DATA_PA + 8).unwrap(), 0x3322_11AA);
    assert_eq!(m.bus.read_u32(DATA_PA + 12).unwrap(), 0xBBBB_BB44);
}

#[test]
fn multiply_divide_and_bit_field_ops() {
    // mult t2,t3 ; mfhi t4 ; mflo t5 ; div zero,t2,t3 ; madd t2,t3 ;
    // clz t6,t3 ; ext t7,t2,4,8 ; ins s0,t2,8,4 ; seb s1,t2 ; wsbh s2,t2 ;
    // ror s3,t2,8 ; movn s4,t2,t3
    let code = [
        0x014B_0018,
        0x0000_6010,
        0x0000_6812,
        0x014B_001A,
        0x714B_0000,
        0x716E_7020,
        0x7D4F_3900,
        0x7D50_5A04,
        0x7C0A_8C20,
        0x7C0A_90A0,
        0x002A_9A02,
        0x014B_A00B,
    ];
    let mut m = machine(&code);
    set(&mut m, 10, 0x8765_4321);
    set(&mut m, 11, 0x0000_0010);
    set(&mut m, 16, 0xFFFF_FFFF);
    run(&mut m, 3);
    // 0x87654321 (negative) * 16, signed.
    assert_eq!(reg(&m, 12), 0xFFFF_FFF8);
    assert_eq!(reg(&m, 13), 0x7654_3210);
    run(&mut m, 1);
    let q = (0x8765_4321u32 as i32) / 16;
    let r = (0x8765_4321u32 as i32) % 16;
    assert_eq!((m.cpu.lo, m.cpu.hi), (q as u32, r as u32));
    run(&mut m, 1);
    let acc = ((u64::from(r as u32) << 32) | u64::from(q as u32)) as i64
        + i64::from(0x8765_4321u32 as i32) * 16;
    assert_eq!((m.cpu.hi, m.cpu.lo), ((acc >> 32) as u32, acc as u32));
    run(&mut m, 7);
    assert_eq!(reg(&m, 14), 27, "clz 0x10");
    assert_eq!(reg(&m, 15), 0x32, "ext bits 11:4");
    assert_eq!(reg(&m, 16), 0xFFFF_F1FF, "ins bits 11:8 from rs 3:0");
    assert_eq!(reg(&m, 17), 0x21);
    assert_eq!(reg(&m, 18), 0x6587_2143);
    assert_eq!(reg(&m, 19), 0x2187_6543);
    assert_eq!(reg(&m, 20), 0x8765_4321, "movn: rt != 0");
}

#[test]
fn exception_in_a_delay_slot_sets_bd_and_points_at_the_branch() {
    // b +1 ; syscall ; nop
    let mut m = machine(&[0x1000_0001, 0x0000_000C, NOP]);
    run(&mut m, 2);
    assert_eq!((m.cpu.cause() >> 2) & 0x1F, EXC_SYS);
    assert_ne!(m.cpu.cause() & CA_BD, 0);
    assert_eq!(m.cpu.cp0[C0_EPC], CODE);
    assert_ne!(m.cpu.status() & ST_EXL, 0);
}

#[test]
fn ll_sc_and_eret_clears_the_link() {
    // ll v0,0(t1) ; sc v1,0(t1) ; ll v0,0(t1) ; eret ; (at EPC) sc v1,0(t1)
    let mut m = machine(&[
        0xC122_0000,
        0xE123_0000,
        0xC122_0000,
        0x4200_0018,
        0xE123_0000,
    ]);
    set(&mut m, 9, DATA);
    set(&mut m, 3, 0x55);
    m.bus.write_u32(DATA_PA, 0x11).unwrap();
    run(&mut m, 2);
    assert_eq!(reg(&m, 3), 1, "sc after ll succeeds");
    assert_eq!(m.bus.read_u32(DATA_PA).unwrap(), 0x55);
    set(&mut m, 3, 0x66);
    m.cpu.cp0[C0_STATUS] = ST_EXL;
    m.cpu.cp0[C0_EPC] = CODE + 0x10;
    run(&mut m, 3);
    assert_eq!(m.cpu.pc, CODE + 0x14);
    assert_eq!(reg(&m, 3), 0, "eret clears LLbit: sc fails");
    assert_eq!(m.bus.read_u32(DATA_PA).unwrap(), 0x55);
}

#[test]
fn misaligned_and_mapped_accesses_fault() {
    // lw v0,2(t1)
    let mut m = machine(&[0x8D22_0002]);
    running_kernel(&mut m);
    set(&mut m, 9, DATA);
    run(&mut m, 1);
    assert_eq!((m.cpu.cause() >> 2) & 0x1F, EXC_ADEL);
    assert_eq!(m.cpu.cp0[C0_BADVADDR], DATA + 2);
    assert_eq!(m.cpu.pc, EBASE_RESET + 0x180);
    // lw v0,0(zero): KUSEG needs the TLB unless Status.ERL is set.
    let mut m = machine(&[0x8C02_0000]);
    running_kernel(&mut m);
    run(&mut m, 1);
    assert_eq!((m.cpu.cause() >> 2) & 0x1F, EXC_TLBL);
    assert_eq!(m.cpu.pc, EBASE_RESET, "TLB refill vector, EBase + 0");
    let mut m = machine(&[0x8C02_0000]);
    m.bus.write_u32(0, 0xCAFE).unwrap();
    run(&mut m, 1);
    assert_eq!(reg(&m, 2), 0xCAFE, "ERL = 1 at reset: KUSEG is unmapped");
}

#[test]
fn shadow_register_sets_through_rdpgpr_wrpgpr() {
    // wrpgpr a3,v1 ; rdpgpr a0,a3
    let mut m = machine(&[0x41C3_3800, 0x4147_2000]);
    m.cpu.cp0[C0_SRSCTL] = (HSS << 26) | (3 << 6);
    set(&mut m, 3, 0xDEAD);
    run(&mut m, 2);
    assert_eq!(m.cpu.gpr[3][7], 0xDEAD);
    assert_eq!(m.cpu.gpr[0][7], 0, "the current set is untouched");
    assert_eq!(reg(&m, 4), 0xDEAD);
}

#[test]
fn di_ei_return_status_and_toggle_ie() {
    // di v0 ; ei v1
    let mut m = machine(&[0x4162_6000, 0x4163_6020]);
    m.cpu.cp0[C0_STATUS] = ST_IE;
    run(&mut m, 1);
    assert_eq!(reg(&m, 2), ST_IE);
    assert_eq!(m.cpu.status() & ST_IE, 0);
    run(&mut m, 1);
    assert_eq!(reg(&m, 3), 0);
    assert_eq!(m.cpu.status() & ST_IE, ST_IE);
}

/// Flag T1 (IRQ 4) at priority `p`.
fn flag_t1(m: &mut Machine<Mips32>, p: u32) {
    m.cpu.evic.iec[0] |= 1 << 4;
    m.cpu.evic.ipc[1] = p << 2;
    m.cpu.set_exception_pending(4);
}

#[test]
fn evic_single_vector_interrupt_entry_and_eret() {
    let mut m = machine(&[NOP; 0x100]);
    running_kernel(&mut m);
    m.cpu.cp0[C0_STATUS] = ST_IE;
    flag_t1(&mut m, 3);
    assert_eq!(m.cpu.evic.read(EV_INTSTAT), (3 << 8) | 4, "SRIPL / SIRQ");
    run(&mut m, 1);
    // Taken after the first NOP: EBase + 0x200 (single vector, Cause.IV).
    assert_eq!(m.cpu.pc, EBASE_RESET + 0x200);
    assert_eq!(m.cpu.cp0[C0_EPC], CODE + 4);
    assert_eq!((m.cpu.read_cp0(13, 0) >> CA_RIPL_SHIFT) & 0x3F, 3);
    assert_ne!(m.cpu.status() & ST_EXL, 0);
    // The handler clears IFS0.T1IF through the CLR alias, then erets.
    m.cpu.evic.write(EV_IFS + 4, 1 << 4);
    assert_eq!(m.cpu.evic.read(EV_INTSTAT), 0);
    let flow = m.cpu.eret();
    assert!(matches!(flow, Flow::Jump(t) if t == CODE + 4));
    assert_eq!(m.cpu.status() & ST_EXL, 0);
}

#[test]
fn evic_priority_masking_and_multi_vector_offsets() {
    let mut m = machine(&[NOP; 0x100]);
    running_kernel(&mut m);
    // Status.IPL = 3 holds a priority-3 request.
    m.cpu.cp0[C0_STATUS] = ST_IE | (3 << ST_IPL_SHIFT);
    flag_t1(&mut m, 3);
    run(&mut m, 2);
    assert_eq!(m.cpu.pc, CODE + 8, "held at IPL 3");
    // Raise it to 4 and switch to multi-vector mode with OFF004 = 0x400,
    // priority level 4 using shadow set 2.
    m.cpu.evic.write(EV_IPC + 0x10, 4 << 2);
    m.cpu.evic.write(EV_INTCON + 8, INTCON_MVEC);
    m.cpu.evic.write(EV_OFF + 4 * 4, 0x400);
    m.cpu.evic.write(EV_PRISS, 2 << 16);
    run(&mut m, 1);
    assert_eq!(m.cpu.pc, EBASE_RESET + 0x400);
    assert_eq!(m.cpu.read_cp0(12, 2) & 0xF, 2, "CSS = PRI4SS");
    assert_eq!((m.cpu.read_cp0(12, 2) >> 6) & 0xF, 0, "PSS = the old set");
    let _ = m.cpu.eret();
    assert_eq!(m.cpu.read_cp0(12, 2) & 0xF, 0, "eret returns to PSS");
}

#[test]
fn core_timer_match_flags_irq0_and_compare_write_clears_ti() {
    let mut m = machine(&[NOP; 0x100]);
    m.cpu.cp0[C0_COUNT] = 0;
    m.cpu.cp0[C0_COMPARE] = 10;
    run(&mut m, 19);
    assert_eq!(m.cpu.evic.ifs[0] & 1, 0);
    run(&mut m, 1);
    assert_eq!(m.cpu.cp0[C0_COUNT], 10, "Count runs at half the core clock");
    assert_eq!(m.cpu.evic.ifs[0] & 1, 1);
    assert_ne!(m.cpu.cause() & CA_TI, 0);
    m.cpu.write_cp0(11, 0, 100);
    assert_eq!(m.cpu.cause() & CA_TI, 0);
}

#[test]
fn core_software_interrupt_latches_on_the_rising_edge() {
    let mut m = machine(&[NOP; 0x10]);
    m.cpu.write_cp0(13, 0, 1 << 8);
    assert_eq!(m.cpu.evic.ifs[0] & 2, 2);
    // NuttX clears CS0IF, then Cause.IP0: the flag must stay clear.
    m.cpu.evic.write(EV_IFS + 4, 2);
    run(&mut m, 2);
    assert_eq!(m.cpu.evic.ifs[0] & 2, 0);
    m.cpu.write_cp0(13, 0, 0);
    assert_eq!(m.cpu.evic.ifs[0] & 2, 0);
}

#[test]
fn a_fixed_point_loop_is_idle_but_a_polling_loop_is_not() {
    // idle: b idle ; nop
    let mut m = machine(&[0x1000_FFFF, NOP]);
    m.cpu.cp0[C0_COMPARE] = 1000;
    run(&mut m, 6);
    assert!(m.cpu.idle_loop);
    let budget = m.cpu.idle_fast_forward_budget(&m.bus).unwrap();
    assert!(budget > 0 && budget < 2000, "{budget}");
    // pollj: lw v0,0(t1) ; b pollj ; nop
    let mut m = machine(&[0x8D22_0000, 0x1000_FFFE, NOP]);
    set(&mut m, 9, DATA);
    run(&mut m, 12);
    assert!(!m.cpu.idle_loop, "a load can see new data");
    assert_eq!(m.cpu.idle_fast_forward_budget(&m.bus), None);
}

#[test]
fn wait_idles_until_a_request_and_tlb_ops_are_reserved() {
    // wait ; nop
    let mut m = machine(&[0x4200_0020, NOP]);
    run(&mut m, 1);
    assert!(m.cpu.waiting);
    run(&mut m, 3);
    assert_eq!(m.cpu.pc, CODE + 4);
    flag_t1(&mut m, 1);
    run(&mut m, 1);
    assert!(!m.cpu.waiting, "a pending request ends WAIT");
    // tlbwi: the TLB is not modelled.
    let mut m = machine(&[0x4200_0002]);
    run(&mut m, 1);
    assert_eq!((m.cpu.cause() >> 2) & 0x1F, EXC_RI);
}

#[test]
fn micromips_boot_isa_is_refused() {
    let mut bus = SystemBus::new();
    bus.flash_boot_alias = false;
    bus.flash = LinearMemory::new(64 * 1024, 0x1FC0_0000);
    // DEVCFG0.BOOTISA = 0 at 0x1FC0FFCC.
    bus.flash.data[0xFFCC..0xFFD0].copy_from_slice(&0xFFFF_FFBFu32.to_le_bytes());
    let mut cpu = Mips32::new();
    assert!(cpu.reset(&mut bus).is_err());
    bus.flash.data[0xFFCC..0xFFD0].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    assert!(cpu.reset(&mut bus).is_ok());
}
