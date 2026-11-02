// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RV64 hart unit tests. Every expected value is the ISA manual's: the
//! encodings are built by hand below, so a decoder bug cannot hide behind an
//! assembler that shares it.

use super::*;
use crate::bus::SystemBus;
use crate::{Bus, Cpu, Machine};

const RAM: u64 = 0x2000_0000;
const CLINT: u64 = 0x0200_0000;

fn r(f7: u32, rs2: u32, rs1: u32, f3: u32, rd: u32, op: u32) -> u32 {
    (f7 << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
}
fn i(imm: i32, rs1: u32, f3: u32, rd: u32, op: u32) -> u32 {
    (((imm as u32) & 0xFFF) << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
}
fn s(imm: i32, rs2: u32, rs1: u32, f3: u32) -> u32 {
    let imm = imm as u32;
    (((imm >> 5) & 0x7F) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (f3 << 12)
        | ((imm & 0x1F) << 7)
        | 0x23
}
fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    i(imm, rs1, 0, rd, 0x13)
}
fn amo(f5: u32, rs2: u32, rs1: u32, f3: u32, rd: u32) -> u32 {
    r(f5 << 2, rs2, rs1, f3, rd, 0x2F)
}
fn csr(op_f3: u32, rd: u32, csr: u32, rs1: u32) -> u32 {
    (csr << 20) | (rs1 << 15) | (op_f3 << 12) | (rd << 7) | 0x73
}

/// A machine whose RAM holds `code` (32-bit words) at `RAM`, PC there.
fn machine(code: &[u32]) -> Machine<RiscV64> {
    let mut bytes = Vec::new();
    for w in code {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    machine_bytes(&bytes)
}

fn machine_bytes(code: &[u8]) -> Machine<RiscV64> {
    let mut bus = SystemBus::new();
    bus.ram.data[..code.len()].copy_from_slice(code);
    let mut cpu = RiscV64::new(0, Some(CLINT), 1);
    cpu.pc = RAM;
    Machine::new(cpu, bus)
}

fn run(m: &mut Machine<RiscV64>, n: usize) {
    for _ in 0..n {
        m.step().unwrap();
    }
}

#[test]
fn word_ops_sign_extend_their_32_bit_result() {
    // x1 = 0x7FFF_FFFF via LUI 0x80000 + ADDIW -1, then ADDIW x2, x1, 1.
    let mut m = machine(&[
        (0x80000 << 12) | (1 << 7) | 0x37, // lui x1, 0x80000 -> 0xFFFF_FFFF_8000_0000
        i(-1, 1, 0, 1, 0x1B),              // addiw x1, x1, -1 -> 0x7FFF_FFFF
        i(1, 1, 0, 2, 0x1B),               // addiw x2, x1, 1 -> 0xFFFF_FFFF_8000_0000
        i(0x41F, 2, 5, 3, 0x1B),           // sraiw x3, x2, 31 -> -1
        i(31, 2, 5, 4, 0x1B),              // srliw x4, x2, 31 -> 1
        r(0x20, 1, 0, 0, 5, 0x3B),         // subw x5, x0, x1 -> 0xFFFF_FFFF_8000_0001
    ]);
    run(&mut m, 6);
    assert_eq!(m.cpu.x[1], 0x7FFF_FFFF);
    assert_eq!(m.cpu.x[2], 0xFFFF_FFFF_8000_0000);
    assert_eq!(m.cpu.x[3], u64::MAX);
    assert_eq!(m.cpu.x[4], 1);
    assert_eq!(m.cpu.x[5], 0xFFFF_FFFF_8000_0001);
}

#[test]
fn shifts_use_six_bit_amounts_on_rv64() {
    let mut m = machine(&[
        addi(1, 0, 1),
        i(63, 1, 1, 2, 0x13),         // slli x2, x1, 63
        i(0x400 | 63, 2, 5, 3, 0x13), // srai x3, x2, 63 -> -1
        i(63, 2, 5, 4, 0x13),         // srli x4, x2, 63 -> 1
        addi(5, 0, 32),
        r(0, 5, 1, 1, 6, 0x33), // sll x6, x1, x5 -> 1 << 32
    ]);
    run(&mut m, 6);
    assert_eq!(m.cpu.x[2], 1 << 63);
    assert_eq!(m.cpu.x[3], u64::MAX);
    assert_eq!(m.cpu.x[4], 1);
    assert_eq!(m.cpu.x[6], 1 << 32);
}

#[test]
fn m_extension_corner_cases_follow_the_spec_table() {
    let mut m = machine(&[]);
    let cpu = &mut m.cpu;
    cpu.x[1] = i64::MIN as u64;
    cpu.x[2] = u64::MAX; // -1
    cpu.x[3] = 0;
    cpu.x[4] = 0xFFFF_FFFF_FFFF_FFFE; // -2 signed, huge unsigned
    cpu.x[5] = 3;
    let code = [
        r(1, 2, 1, 4, 10, 0x33), // div   x10 = MIN / -1 = MIN (overflow)
        r(1, 2, 1, 6, 11, 0x33), // rem   x11 = MIN % -1 = 0
        r(1, 3, 5, 4, 12, 0x33), // div   x12 = 3 / 0 = -1
        r(1, 3, 5, 7, 13, 0x33), // remu  x13 = 3 % 0 = 3
        r(1, 5, 4, 1, 14, 0x33), // mulh  x14 = hi(-2 * 3) = -1
        r(1, 5, 4, 3, 15, 0x33), // mulhu x15 = hi((2^64-2) * 3) = 2
        r(1, 5, 4, 2, 16, 0x33), // mulhsu x16 = hi(-2 * 3u) = -1
        r(1, 3, 5, 4, 17, 0x3B), // divw  x17 = 3 / 0 = -1
        r(1, 5, 4, 5, 18, 0x3B), // divuw x18 = 0xFFFF_FFFE / 3 = 0x5555_5554 (positive)
    ];
    for (n, w) in code.iter().enumerate() {
        m.bus.ram.data[n * 4..n * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    run(&mut m, code.len());
    let x = &m.cpu.x;
    assert_eq!(x[10], i64::MIN as u64);
    assert_eq!(x[11], 0);
    assert_eq!(x[12], u64::MAX);
    assert_eq!(x[13], 3);
    assert_eq!(x[14], u64::MAX);
    assert_eq!(x[15], 2);
    assert_eq!(x[16], u64::MAX);
    assert_eq!(x[17], u64::MAX);
    assert_eq!(x[18], 0x5555_5554);
}

#[test]
fn doubleword_and_unsigned_word_loads() {
    let mut m = machine(&[
        // x1 = RAM + 0x100
        (0x20000 << 12) | (1 << 7) | 0x37, // lui x1, 0x20000
        addi(1, 1, 0x100),
        s(0, 2, 1, 3),       // sd x2, 0(x1)
        i(0, 1, 3, 3, 0x03), // ld x3, 0(x1)
        i(4, 1, 6, 4, 0x03), // lwu x4, 4(x1)
        i(4, 1, 2, 5, 0x03), // lw x5, 4(x1)
    ]);
    m.cpu.x[2] = 0x8765_4321_1234_5678;
    run(&mut m, 6);
    assert_eq!(m.cpu.x[3], 0x8765_4321_1234_5678);
    assert_eq!(m.cpu.x[4], 0x8765_4321);
    assert_eq!(m.cpu.x[5], 0xFFFF_FFFF_8765_4321);
}

#[test]
fn misaligned_load_traps_with_the_address_in_mtval() {
    let mut m = machine(&[i(1, 1, 3, 3, 0x03)]); // ld x3, 1(x1)
    m.cpu.x[1] = RAM + 0x100;
    m.cpu.mtvec = RAM + 0x400;
    run(&mut m, 1);
    assert_eq!(m.cpu.mcause, 4);
    assert_eq!(m.cpu.mtval, RAM + 0x101);
    assert_eq!(m.cpu.mepc, RAM);
    assert_eq!(m.cpu.pc, RAM + 0x400);
}

#[test]
fn lr_sc_pair_succeeds_once_and_an_intervening_store_breaks_it() {
    let mut m = machine(&[
        amo(0b00010, 0, 1, 3, 2), // lr.d x2, (x1)
        amo(0b00011, 3, 1, 3, 4), // sc.d x4, x3, (x1) -> 0
        amo(0b00011, 3, 1, 3, 5), // sc.d x5, x3, (x1) -> 1 (no reservation)
        amo(0b00010, 0, 1, 2, 6), // lr.w x6, (x1)
        s(0, 0, 1, 2),            // sw x0, 0(x1)
        amo(0b00011, 3, 1, 2, 7), // sc.w x7, x3, (x1) -> 1
    ]);
    m.cpu.x[1] = RAM + 0x200;
    m.cpu.x[3] = 0xFFFF_FFFF_8000_0042;
    m.bus.write_u64(RAM + 0x200, 7).unwrap();
    run(&mut m, 6);
    assert_eq!(m.cpu.x[2], 7);
    assert_eq!(m.cpu.x[4], 0);
    assert_eq!(m.cpu.x[5], 1);
    assert_eq!(m.cpu.x[6], 0xFFFF_FFFF_8000_0042, "lr.w sign-extends");
    assert_eq!(m.cpu.x[7], 1);
    assert_eq!(m.bus.read_u64(RAM + 0x200).unwrap(), 0xFFFF_FFFF_0000_0000);
}

#[test]
fn amos_return_the_old_value_and_word_forms_compare_32_bits() {
    let mut m = machine(&[
        amo(0b00000, 2, 1, 3, 3), // amoadd.d x3, x2, (x1)
        amo(0b11100, 4, 5, 2, 6), // amomaxu.w x6, x4, (x5)
        amo(0b10000, 4, 5, 2, 7), // amomin.w x7, x4, (x5)
    ]);
    m.cpu.x[1] = RAM + 0x300;
    m.cpu.x[2] = 5;
    m.cpu.x[5] = RAM + 0x308;
    m.cpu.x[4] = 0x8000_0000; // as a word: unsigned 2^31, signed MIN
    m.bus.write_u64(RAM + 0x300, 10).unwrap();
    m.bus.write_u32(RAM + 0x308, 1).unwrap();
    run(&mut m, 3);
    assert_eq!(m.cpu.x[3], 10);
    assert_eq!(m.bus.read_u64(RAM + 0x300).unwrap(), 15);
    assert_eq!(m.cpu.x[6], 1);
    // maxu(1, 0x8000_0000) stored, then min(signed MIN, MIN) keeps MIN.
    assert_eq!(
        m.cpu.x[7], 0xFFFF_FFFF_8000_0000,
        "word AMO result sign-extends"
    );
    assert_eq!(m.bus.read_u32(RAM + 0x308).unwrap(), 0x8000_0000);
}

#[test]
fn compressed_rv64_forms() {
    // c.addiw x8, -1 ; c.sdsp x8, 8(sp) ; c.ldsp x9, 8(sp) ; c.subw x8(s0), x9(s1)
    // c.addiw: 001 imm[5] rd imm[4:0] 01
    let c_addiw = (0b001 << 13) | (1 << 12) | (8 << 7) | (0b11111 << 2) | 0b01;
    // c.sdsp: 111 uimm[5:3|8:6] rs2 10 ; offset 8 -> uimm[5:3]=001
    let c_sdsp = (0b111 << 13) | (0b001 << 10) | (8 << 2) | 0b10;
    // c.ldsp: 011 uimm[5] rd uimm[4:3|8:6] 10 ; offset 8 -> bits[6:5]=01
    let c_ldsp = (0b011 << 13) | (9 << 7) | (0b01 << 5) | 0b10;
    // c.subw rd'=s0(0), rs2'=s1(1): 100 1 11 rd' 00 rs2' 01 (the zero fields
    // rd'=0 and funct2=00 are left out).
    let c_subw = (0b100 << 13) | (1 << 12) | (0b11 << 10) | (1 << 2) | 0b01;
    let mut bytes = Vec::new();
    for h in [c_addiw, c_sdsp, c_ldsp, c_subw] {
        bytes.extend_from_slice(&(h as u16).to_le_bytes());
    }
    let mut m = machine_bytes(&bytes);
    m.cpu.x[2] = RAM + 0x500;
    m.cpu.x[8] = 0x1_0000_0000; // low word 0 -> addiw -1 = -1 (sign-extended)
    run(&mut m, 4);
    assert_eq!(m.cpu.x[9], u64::MAX);
    assert_eq!(m.bus.read_u64(RAM + 0x508).unwrap(), u64::MAX);
    assert_eq!(m.cpu.x[8], 0, "c.subw: -1 - -1");
    assert_eq!(m.cpu.pc, RAM + 8);
}

#[test]
fn clint_timer_interrupt_vectors_and_mret_restores_mie() {
    // mtvec = RAM + 0x100 | vectored; handler for cause 7 at +0x11C: mret.
    let mut m = machine(&[
        csr(0b001, 0, 0x305, 1), // csrw mtvec, x1
        csr(0b001, 0, 0x304, 2), // csrw mie, x2 (MTIE)
        s(0, 3, 4, 3),           // sd x3, 0(x4)  -> mtimecmp = 5
        csr(0b110, 0, 0x300, 8), // csrsi mstatus, MIE
        addi(10, 10, 1),
        addi(10, 10, 1),
        addi(10, 10, 1),
        addi(10, 10, 1),
    ]);
    m.bus.ram.data[0x11C..0x120].copy_from_slice(&0x3020_0073u32.to_le_bytes());
    m.cpu.x[1] = (RAM + 0x100) | 1;
    m.cpu.x[2] = 1 << 7;
    m.cpu.x[3] = 5;
    m.cpu.x[4] = CLINT + 0x4000;
    run(&mut m, 4);
    // mtime counted 4 instructions; the timer is armed but not due yet.
    assert_eq!(m.cpu.clint.mtime, 4);
    run(&mut m, 1); // addi retires, mtime reaches 5 -> MTIP taken after it
    assert_eq!(m.cpu.mcause, (1 << 63) | 7);
    assert_eq!(m.cpu.mepc, RAM + 20, "mepc is the next instruction");
    assert_eq!(m.cpu.pc, RAM + 0x11C);
    assert_eq!(m.cpu.mstatus & MSTATUS_MIE, 0);
    assert_ne!(m.cpu.mstatus & MSTATUS_MPIE, 0);
    // Disarm from the debugger, then MRET back.
    m.cpu.clint.mtimecmp = u64::MAX;
    run(&mut m, 1);
    assert_eq!(m.cpu.pc, RAM + 20);
    assert_ne!(m.cpu.mstatus & MSTATUS_MIE, 0);
    assert_eq!(m.cpu.x[10], 1);
}

#[test]
fn clint_registers_read_back_through_loads() {
    let mut m = machine(&[
        i(0, 1, 3, 2, 0x03), // ld x2, 0(x1) (mtime)
        s(0, 3, 4, 2),       // sw x3, 0(x4) (msip)
        i(0, 4, 2, 5, 0x03), // lw x5, 0(x4)
    ]);
    m.cpu.x[1] = CLINT + 0xBFF8;
    m.cpu.x[3] = 1;
    m.cpu.x[4] = CLINT;
    m.cpu.clint.mtime = 0x1_0000_0002;
    run(&mut m, 3);
    assert_eq!(m.cpu.x[2], 0x1_0000_0002);
    assert!(m.cpu.clint.msip);
    assert_eq!(m.cpu.x[5], 1);
    assert_eq!(m.cpu.read_csr(0x344).unwrap() & MIP_MSIP, MIP_MSIP);
}

#[test]
fn csr_surface_is_m_mode_rv64imac() {
    let mut m = machine(&[
        csr(0b010, 1, 0x301, 0), // csrr x1, misa
        csr(0b010, 2, 0xF14, 0), // csrr x2, mhartid
        csr(0b001, 0, 0x300, 3), // csrw mstatus, x3 (all ones)
        csr(0b010, 4, 0x300, 0), // csrr x4, mstatus
    ]);
    m.cpu.hart_id = 1;
    m.cpu.x[3] = u64::MAX;
    run(&mut m, 4);
    assert_eq!(m.cpu.x[1], MISA_RV64IMAC);
    assert_eq!(m.cpu.x[1] >> 62, 2, "MXL = 64-bit");
    assert_eq!(m.cpu.x[2], 1);
    // MIE, MPIE, MPP=M; nothing for S/U/F.
    assert_eq!(m.cpu.x[4], MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP);
}

#[test]
fn time_csr_and_fp_opcodes_are_illegal_on_the_u54_model() {
    for inst in [
        csr(0b010, 1, 0xC01, 0), // rdtime: not implemented in hardware
        0x0000_3007,             // fld f0, 0(x0)
        csr(0b001, 0, 0xF11, 1), // write to read-only mvendorid
    ] {
        let mut m = machine(&[inst]);
        m.cpu.mtvec = RAM + 0x40;
        run(&mut m, 1);
        assert_eq!(m.cpu.mcause, 2, "{inst:#010x}");
        assert_eq!(m.cpu.mtval, u64::from(inst));
        assert_eq!(m.cpu.pc, RAM + 0x40);
    }
}

#[test]
fn wfi_idle_budget_runs_to_the_timer_compare() {
    let mut m = machine(&[0x1050_0073]); // wfi
    m.cpu.clint.divider = 600;
    m.cpu.mie = MIP_MTIP;
    m.cpu.clint.mtimecmp = 10;
    run(&mut m, 1);
    // One clock retired; 10 ticks of 600 clocks less that one, less the
    // boundary clock the machine still steps.
    assert_eq!(m.cpu.idle_fast_forward_budget(&m.bus), Some(6000 - 1 - 1));
    m.cpu.fast_forward_idle_cycles(6000 - 2);
    assert_eq!(m.cpu.clint.mtime, 9);
    // Nothing enabled: the wait has no end the core knows of.
    m.cpu.mie = 0;
    assert_eq!(m.cpu.idle_fast_forward_budget(&m.bus), Some(u64::MAX));
}

#[test]
fn mpfs_chip_selects_the_first_u54_and_the_mss_clint() {
    let p = Rv64Params::for_chip_name("mpfs095t", 600_000_000);
    assert_eq!(p.hart_id, 1);
    assert_eq!(p.clint_base, Some(0x0200_0000));
    assert_eq!(p.mtime_divider, 600);
}

#[test]
fn snapshot_round_trip_keeps_architectural_state() {
    let mut m = machine(&[addi(1, 0, 42)]);
    run(&mut m, 1);
    m.cpu.clint.mtimecmp = 99;
    let snap = m.cpu.snapshot();
    let mut other = RiscV64::default();
    other.apply_snapshot(&snap);
    assert_eq!(other.x[1], 42);
    assert_eq!(other.pc, RAM + 4);
    assert_eq!(other.clint.mtimecmp, 99);
    assert_eq!(other.minstret, 1);
}
