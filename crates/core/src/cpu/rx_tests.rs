// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RX core unit tests. Every instruction encoding below was produced by the
//! Zephyr SDK 1.0.1 `rx-zephyr-elf-as --misa=v3` (binutils 2.43) from the
//! source in the comment above it, so the decoder is checked against an
//! independent encoder, not against itself. Expected values follow the RXv3
//! software manual (R01US0316).

use super::*;
use crate::bus::SystemBus;
use crate::{Cpu, Machine};

const CODE: u32 = 0x2000_0000;
const DATA: u32 = 0x2000_1000;
const STACK: u32 = 0x2000_2000;
const INTB: u32 = 0x2000_3000;
const EXTB: u32 = 0x2000_3400;

fn machine(code: &[u8]) -> Machine<Rx> {
    let mut bus = SystemBus::new();
    bus.ram.data[..code.len()].copy_from_slice(code);
    let mut cpu = Rx::new();
    cpu.pc = CODE;
    cpu.r[0] = STACK;
    cpu.intb = INTB;
    cpu.extb = EXTB;
    Machine::new(cpu, bus)
}

fn run(m: &mut Machine<Rx>, n: usize) {
    for _ in 0..n {
        m.step().unwrap();
    }
}

fn w32(m: &mut Machine<Rx>, addr: u32, v: u32) {
    use crate::Bus;
    m.bus.write_u32(u64::from(addr), v).unwrap();
}

fn r32(m: &Machine<Rx>, addr: u32) -> u32 {
    use crate::Bus;
    m.bus.read_u32(u64::from(addr)).unwrap()
}

fn r8(m: &Machine<Rx>, addr: u32) -> u8 {
    use crate::Bus;
    m.bus.read_u8(u64::from(addr)).unwrap()
}

// cmp r1, r2 ; scgeu.l r6 ; nop
const CMP_CARRY: &[u8] = &[0x47, 0x12, 0xfc, 0xdb, 0x62, 0x03];

#[test]
fn compare_carry_means_no_borrow() {
    // 3 - 5 borrows: C = 0, S = 1.
    let mut m = machine(CMP_CARRY);
    m.cpu.r[1] = 5;
    m.cpu.r[2] = 3;
    run(&mut m, 2);
    assert_eq!(m.cpu.r[6], 0, "GEU must be false after 3 - 5");
    assert_eq!(m.cpu.psw() & (PSW_C | PSW_S | PSW_Z), PSW_S);
    // 5 - 3: C = 1.
    let mut m = machine(CMP_CARRY);
    m.cpu.r[1] = 3;
    m.cpu.r[2] = 5;
    run(&mut m, 2);
    assert_eq!(m.cpu.r[6], 1);
    assert_eq!(m.cpu.psw() & PSW_C, PSW_C);
}

// sub r1, r2, r3 ; add r1, r2, r4
const SUB3: &[u8] = &[0xff, 0x03, 0x12, 0xff, 0x24, 0x12];

#[test]
fn three_operand_sub_is_rs2_minus_rs() {
    let mut m = machine(SUB3);
    m.cpu.r[1] = 3;
    m.cpu.r[2] = 10;
    run(&mut m, 2);
    assert_eq!(m.cpu.r[3], 7);
    assert_eq!(m.cpu.r[4], 13);
}

// add #1, r1
const ADD_OVF: &[u8] = &[0x62, 0x11];

#[test]
fn add_sets_overflow_and_carry() {
    let mut m = machine(ADD_OVF);
    m.cpu.r[1] = 0x7FFF_FFFF;
    run(&mut m, 1);
    assert_eq!(m.cpu.r[1], 0x8000_0000);
    assert_eq!(m.cpu.psw() & (PSW_O | PSW_S | PSW_C | PSW_Z), PSW_O | PSW_S);
    let mut m = machine(ADD_OVF);
    m.cpu.r[1] = 0xFFFF_FFFF;
    run(&mut m, 1);
    assert_eq!(m.cpu.psw() & (PSW_O | PSW_C | PSW_Z), PSW_C | PSW_Z);
}

// div r2, r1 ; divu r4, r3
const DIV: &[u8] = &[0xfc, 0x23, 0x21, 0xfc, 0x27, 0x43];

#[test]
fn division_by_zero_and_overflow_set_o_and_keep_the_dividend() {
    let mut m = machine(DIV);
    m.cpu.r[1] = 0x8000_0000;
    m.cpu.r[2] = 0xFFFF_FFFF; // -2^31 / -1 overflows
    m.cpu.r[3] = 100;
    m.cpu.r[4] = 7;
    run(&mut m, 1);
    assert_eq!(m.cpu.r[1], 0x8000_0000);
    assert_ne!(m.cpu.psw() & PSW_O, 0);
    run(&mut m, 1);
    assert_eq!(m.cpu.r[3], 14);
    assert_eq!(m.cpu.psw() & PSW_O, 0);

    let mut m = machine(DIV);
    m.cpu.r[1] = (-21i32) as u32;
    m.cpu.r[2] = 4;
    m.cpu.r[3] = 9;
    m.cpu.r[4] = 0;
    run(&mut m, 2);
    assert_eq!(m.cpu.r[1] as i32, -5, "signed division truncates");
    assert_eq!(m.cpu.r[3], 9);
    assert_ne!(m.cpu.psw() & PSW_O, 0);
}

// emul r2, r4
const EMUL: &[u8] = &[0xfc, 0x1b, 0x24];

#[test]
fn emul_writes_the_64_bit_product_to_rd_and_rd_plus_one() {
    let mut m = machine(EMUL);
    m.cpu.r[2] = (-3i32) as u32;
    m.cpu.r[4] = 0x4000_0000;
    run(&mut m, 1);
    let p = (i64::from(m.cpu.r[5]) << 32) | i64::from(m.cpu.r[4]);
    assert_eq!(p, -3 * 0x4000_0000);
}

// shll #1, r1 ; shar #4, r2 ; shlr #31, r3, r4
const SHIFTS: &[u8] = &[0x6c, 0x11, 0x6a, 0x42, 0xfd, 0x9f, 0x34];

#[test]
fn shift_flags() {
    let mut m = machine(SHIFTS);
    m.cpu.r[1] = 0x4000_0001; // MSB changes: O = 1, nothing carried out
    m.cpu.r[2] = 0x8000_0018; // arithmetic: sign fills, C = bit 3 = 1
    m.cpu.r[3] = 0x8000_0000;
    run(&mut m, 1);
    assert_eq!(m.cpu.r[1], 0x8000_0002);
    assert_eq!(m.cpu.psw() & (PSW_O | PSW_C), PSW_O);
    run(&mut m, 1);
    assert_eq!(m.cpu.r[2], 0xF800_0001);
    assert_eq!(m.cpu.psw() & (PSW_O | PSW_C), PSW_C);
    run(&mut m, 1);
    assert_eq!(m.cpu.r[4], 1);
    assert_eq!(m.cpu.r[3], 0x8000_0000, "the source is unchanged");
    // SHLL of 0xC000_0000 by 1: the bit out equals the new MSB, O = 0.
    let mut m = machine(SHIFTS);
    m.cpu.r[1] = 0xC000_0000;
    run(&mut m, 1);
    assert_eq!(m.cpu.psw() & (PSW_O | PSW_C), PSW_C);
}

// add 2[r3].w, r4 ; add 2[r3].uw, r5 ; sub [r3].b, r6 ; mov.l 4[r3], r7
const MEMEX: &[u8] = &[
    0x06, 0x49, 0x34, 0x01, 0x06, 0xc9, 0x35, 0x01, 0x06, 0x00, 0x36, 0xa8, 0x3f,
];

#[test]
fn memory_extension_sizes_scale_and_extend() {
    let mut m = machine(MEMEX);
    // `2[r3].w` is a byte displacement of 2 (dsp:8 field 1, scaled by 2).
    w32(&mut m, DATA, 0xFFFE_0080);
    w32(&mut m, DATA + 4, 0x1234_5678);
    m.cpu.r[3] = DATA;
    m.cpu.r[4] = 10;
    m.cpu.r[5] = 10;
    m.cpu.r[6] = 0;
    run(&mut m, 4);
    assert_eq!(m.cpu.r[4], 8, ".W sign-extends 0xFFFE");
    assert_eq!(m.cpu.r[5], 10 + 0xFFFE, ".UW zero-extends");
    assert_eq!(m.cpu.r[6], 0x80, "0 - sext8(0x80)");
    assert_eq!(m.cpu.r[7], 0x1234_5678);
}

// mov.b r1, 3[r2] ; mov.w 2[r2], r3 ; mov.l 4[r2], 8[r2] ; mov.l r1, [r2+]
// mov.b [-r2], r4 ; mov.w r1, [r5, r2] ; movu.b [r5, r2], r6
// mov.l #0x12345678, 12[r2] ; movu.w 0x100[r2], r7
const MOVFORMS: &[u8] = &[
    0x80, 0xa9, 0x98, 0x2b, 0xe5, 0x22, 0x01, 0x02, 0xfd, 0x22, 0x21, 0xfd, 0x2c, 0x24, 0xfe, 0x15,
    0x21, 0xfe, 0xc5, 0x26, 0xf9, 0x22, 0x03, 0x78, 0x56, 0x34, 0x12, 0x5d, 0x27, 0x80,
];

#[test]
fn mov_addressing_modes() {
    let mut m = machine(MOVFORMS);
    w32(&mut m, DATA, 0x8001_0203);
    w32(&mut m, DATA + 4, 0xCAFE_F00D);
    w32(&mut m, DATA + 0x200, 0x0000_9ABC);
    m.cpu.r[1] = 0xAABB_CCDD;
    m.cpu.r[2] = DATA;
    m.cpu.r[5] = 3;
    run(&mut m, 1);
    assert_eq!(r8(&m, DATA + 3), 0xDD, "dsp:5 byte store");
    run(&mut m, 1);
    // dsp:5 of 1 scaled by 2: the word at DATA + 2 (0x01, then the 0xDD
    // just stored), sign-extended.
    assert_eq!(m.cpu.r[3], 0xFFFF_DD01);
    run(&mut m, 1);
    assert_eq!(r32(&m, DATA + 8), 0xCAFE_F00D, "memory to memory");
    run(&mut m, 1);
    assert_eq!(r32(&m, DATA), 0xAABB_CCDD);
    assert_eq!(m.cpu.r[2], DATA + 4, "post-increment by the size");
    run(&mut m, 1);
    assert_eq!(m.cpu.r[2], DATA + 3, "pre-decrement by the size");
    assert_eq!(m.cpu.r[4], 0xFFFF_FFAA, "byte at DATA + 3, sign-extended");
    run(&mut m, 1);
    // [r5, r2].w = r2 + r5 * 2 = DATA + 3 + 6
    assert_eq!(r32(&m, DATA + 9) & 0xFFFF, 0xCCDD);
    run(&mut m, 1);
    // [r5, r2].b: base r5 = 3, index r2 * 1
    assert_eq!(m.cpu.r[6], u32::from(r8(&m, DATA + 6)));
    run(&mut m, 1);
    // 12[r2].l with dsp8 3 * 4
    assert_eq!(r32(&m, DATA + 3 + 12), 0x1234_5678);
    run(&mut m, 1);
    // 0x100[r2].w is dsp16 0x80 * 2 from DATA + 3
    let want = u32::from(r8(&m, DATA + 0x103)) | (u32::from(r8(&m, DATA + 0x104)) << 8);
    assert_eq!(m.cpu.r[7], want);
}

// pushm r6-r8 ; push.b r1 ; pop r9 ; popm r6-r8
const STACK_OPS: &[u8] = &[0x6e, 0x68, 0x7e, 0x81, 0x7e, 0xb9, 0x6f, 0x68];

#[test]
fn pushm_puts_the_lowest_register_lowest_and_push_moves_by_four() {
    let mut m = machine(STACK_OPS);
    m.cpu.r[6] = 6;
    m.cpu.r[7] = 7;
    m.cpu.r[8] = 8;
    m.cpu.r[1] = 0x1234_56AB;
    run(&mut m, 1);
    assert_eq!(m.cpu.r[0], STACK - 12);
    assert_eq!(r32(&m, STACK - 12), 6);
    assert_eq!(r32(&m, STACK - 4), 8);
    run(&mut m, 1);
    assert_eq!(m.cpu.r[0], STACK - 16, "PUSH.B still moves SP by 4");
    assert_eq!(r8(&m, STACK - 16), 0xAB);
    m.cpu.r[6] = 0;
    m.cpu.r[7] = 0;
    m.cpu.r[8] = 0;
    run(&mut m, 2);
    assert_eq!(m.cpu.r[9] & 0xFF, 0xAB);
    assert_eq!((m.cpu.r[6], m.cpu.r[7], m.cpu.r[8]), (6, 7, 8));
    assert_eq!(m.cpu.r[0], STACK);
}

// rtsd #16, r6-r7
const RTSD: &[u8] = &[0x3f, 0x67, 0x04];

#[test]
fn rtsd_releases_the_frame_pops_and_returns() {
    // Frame: 8 bytes of locals, then r6, r7, then the return address.
    let mut m = machine(RTSD);
    let sp = STACK - 20;
    m.cpu.r[0] = sp;
    w32(&mut m, sp + 8, 0x66);
    w32(&mut m, sp + 12, 0x77);
    w32(&mut m, sp + 16, 0x2000_0400);
    run(&mut m, 1);
    assert_eq!((m.cpu.r[6], m.cpu.r[7]), (0x66, 0x77));
    assert_eq!(m.cpu.pc, 0x2000_0400);
    assert_eq!(m.cpu.r[0], STACK);
}

// bset #3, [r2].b ; bclr #0, 1[r2].b ; btst #3, [r2].b ; bnot #31, r4
// bmne #5, r5 ; bset r1, r6
const BITS: &[u8] = &[
    0xf0, 0x23, 0xf1, 0x28, 0x01, 0xf4, 0x23, 0xfd, 0xff, 0xf4, 0xfd, 0xe5, 0x15, 0xfc, 0x63, 0x61,
];

#[test]
fn bit_operations() {
    let mut m = machine(BITS);
    w32(&mut m, DATA, 0x0000_FF00);
    m.cpu.r[2] = DATA;
    m.cpu.r[4] = 0;
    m.cpu.r[5] = 0;
    m.cpu.r[1] = 33; // register bit number is taken mod 32
    run(&mut m, 3);
    assert_eq!(r8(&m, DATA), 0x08);
    assert_eq!(r8(&m, DATA + 1), 0xFE);
    assert_eq!(
        m.cpu.psw() & (PSW_C | PSW_Z),
        PSW_C,
        "BTST: C = bit, Z = !bit"
    );
    run(&mut m, 1);
    assert_eq!(m.cpu.r[4], 0x8000_0000);
    run(&mut m, 1);
    assert_eq!(m.cpu.r[5], 1 << 5, "BMNE with Z clear sets the bit");
    run(&mut m, 1);
    assert_eq!(m.cpu.r[6], 1 << 1);
}

// cmp #3, r1 ; scgt.b 0[r2] ; sceq.l r3
const SCC: &[u8] = &[0x61, 0x31, 0xfc, 0xd0, 0x2a, 0xfc, 0xdb, 0x30];

#[test]
fn store_condition() {
    let mut m = machine(SCC);
    w32(&mut m, DATA, 0xFFFF_FFFF);
    m.cpu.r[1] = (-1i32) as u32;
    m.cpu.r[2] = DATA;
    m.cpu.r[3] = 99;
    run(&mut m, 3);
    assert_eq!(
        r32(&m, DATA),
        0xFFFF_FF00,
        "-1 > 3 is false, one byte stored"
    );
    assert_eq!(m.cpu.r[3], 0);
}

// smovf ; sstr.w
const STRINGS: &[u8] = &[0x7f, 0x8f, 0x7f, 0x89];

#[test]
fn smovf_copies_and_sstr_fills() {
    let mut m = machine(STRINGS);
    w32(&mut m, DATA, 0x4433_2211);
    m.cpu.r[1] = DATA + 0x10;
    m.cpu.r[2] = DATA;
    m.cpu.r[3] = 3;
    run(&mut m, 1);
    assert_eq!(r32(&m, DATA + 0x10), 0x0033_2211);
    assert_eq!(
        (m.cpu.r[1], m.cpu.r[2], m.cpu.r[3]),
        (DATA + 0x13, DATA + 3, 0)
    );
    m.cpu.r[1] = DATA + 0x20;
    m.cpu.r[2] = 0xBEEF;
    m.cpu.r[3] = 2;
    run(&mut m, 1);
    assert_eq!(r32(&m, DATA + 0x20), 0xBEEF_BEEF);
    assert_eq!(m.cpu.r[1], DATA + 0x24);
}

// scmpu
const SCMPU: &[u8] = &[0x7f, 0x83];

#[test]
fn scmpu_stops_at_the_first_difference() {
    let mut m = machine(SCMPU);
    w32(&mut m, DATA, u32::from_le_bytes(*b"abcd"));
    w32(&mut m, DATA + 8, u32::from_le_bytes(*b"abxd"));
    m.cpu.r[1] = DATA;
    m.cpu.r[2] = DATA + 8;
    m.cpu.r[3] = 4;
    run(&mut m, 1);
    assert_eq!(m.cpu.r[3], 1, "three bytes compared");
    assert_eq!(
        m.cpu.psw() & (PSW_Z | PSW_C),
        0,
        "'c' < 'x': not equal, borrow"
    );
}

// cmp #0, r1 ; beq.s 1f ; mov.l #1, r2 ; 1: bra.b 2f ; nop ; 2: add #1, r3
// cmp #3, r3 ; bne.b 1b
const BRANCH: &[u8] = &[
    0x61, 0x01, 0x13, 0x66, 0x12, 0x2e, 0x03, 0x03, 0x62, 0x13, 0x61, 0x33, 0x21, 0xf9,
];

#[test]
fn branches_are_relative_to_the_branch() {
    let mut m = machine(BRANCH);
    m.cpu.r[1] = 0;
    m.cpu.r[2] = 0;
    m.cpu.r[3] = 0;
    // cmp, beq.s (taken, dsp 3), then three rounds of bra/add/cmp/bne.
    run(&mut m, 2 + 3 * 4);
    assert_eq!(m.cpu.r[2], 0, "BEQ.S skipped the MOV");
    assert_eq!(m.cpu.r[3], 3);
    assert_eq!(m.cpu.pc, CODE + BRANCH.len() as u32);
}

// int #5 ; add #1, r9
const INTRET: &[u8] = &[0x75, 0x60, 0x05, 0x62, 0x19];
// pushc psw ; mvfc usp, r10 ; rte  (the handler)
const HANDLER: &[u8] = &[0x7e, 0xc0, 0xfd, 0x6a, 0x2a, 0x7f, 0x95];

#[test]
fn int_switches_to_the_interrupt_stack_and_rte_returns() {
    let mut m = machine(INTRET);
    let handler = CODE + 0x100;
    for (i, b) in HANDLER.iter().enumerate() {
        m.bus.ram.data[0x100 + i] = *b;
    }
    w32(&mut m, INTB + 5 * 4, handler);
    // Thread mode: U = 1 (R0 = USP), I = 1, IPL = 3.
    let isp = STACK + 0x800;
    let usp = STACK;
    m.cpu.set_psw(PSW_U | PSW_I | (3 << PSW_IPL_SHIFT) | PSW_C);
    m.cpu.r[0] = usp;
    m.cpu.isp = isp;
    run(&mut m, 1);
    assert_eq!(m.cpu.pc, handler);
    assert_eq!(m.cpu.psw() & (PSW_U | PSW_I), 0);
    assert_eq!(m.cpu.ipl(), 3, "INT leaves IPL alone");
    assert_eq!(m.cpu.r[0], isp - 8, "PSW then PC on the ISP");
    assert_eq!(r32(&m, isp - 8), CODE + 3, "the return address");
    assert_eq!(r32(&m, isp - 4) & PSW_U, PSW_U);
    // pushc psw / mvfc usp
    run(&mut m, 2);
    assert_eq!(m.cpu.r[10], usp);
    m.cpu.r[0] += 4; // drop the PUSHC
    run(&mut m, 1); // rte
    assert_eq!(m.cpu.pc, CODE + 3);
    assert_eq!(m.cpu.psw() & (PSW_U | PSW_I | PSW_C), PSW_U | PSW_I | PSW_C);
    assert_eq!(m.cpu.r[0], usp, "back on the USP");
    assert_eq!(m.cpu.isp, isp);
    run(&mut m, 1);
    assert_eq!(m.cpu.r[9], 1);
}

/// `rte` at the vector-28 handler.
const RTE: &[u8] = &[0x7f, 0x95];
// wait ; add #1, r9
const WAITI: &[u8] = &[0x7f, 0x96, 0x62, 0x19];

/// A byte store to the ICU, through the core as an instruction makes it.
fn icu_write(m: &mut Machine<Rx>, off: u32, v: u8) {
    m.cpu
        .write(&mut m.bus, RX_ICU_BASE + off, 1, u32::from(v))
        .unwrap();
}

#[test]
fn icu_takes_the_highest_enabled_request_above_ipl() {
    let mut m = machine(WAITI);
    let handler = CODE + 0x200;
    m.bus.ram.data[0x200..0x202].copy_from_slice(RTE);
    w32(&mut m, INTB + 28 * 4, handler);
    w32(&mut m, INTB + 40 * 4, handler);
    m.cpu.set_psw(PSW_I | (2 << PSW_IPL_SHIFT));
    // Vector 28 (CMI0): IPR004; vector 40: IPR040.
    icu_write(&mut m, 0x304, 2); // not above IPL 2
    icu_write(&mut m, 0x300 + 40, 5);
    icu_write(&mut m, 0x200 + 28 / 8, 1 << (28 % 8));
    // 40 requested but not enabled yet.
    m.cpu.set_exception_pending(28);
    m.cpu.set_exception_pending(40);
    run(&mut m, 1); // wait
    assert!(m.cpu.waiting);
    run(&mut m, 3);
    assert!(m.cpu.waiting, "28 is not above IPL 2 and 40 is not enabled");
    assert_eq!(m.cpu.icu.read(28), 1);
    icu_write(&mut m, 0x200 + 40 / 8, 1 << (40 % 8));
    run(&mut m, 1);
    assert!(!m.cpu.waiting);
    assert_eq!(m.cpu.pc, handler);
    assert_eq!(m.cpu.ipl(), 5, "IPL raised to the request's priority");
    assert_eq!(m.cpu.icu.read(40), 0, "acceptance clears IR");
    assert_eq!(m.cpu.icu.read(28), 1, "28 still held");
    // Writing 0 to IR28 withdraws it; writing 1 does not set it.
    icu_write(&mut m, 28, 0);
    assert_eq!(m.cpu.icu.read(28), 0);
    icu_write(&mut m, 28, 1);
    assert_eq!(m.cpu.icu.read(28), 0);
    run(&mut m, 1); // rte
    assert_eq!(m.cpu.pc, CODE + 2, "WAIT resumes after itself");
    assert_eq!(m.cpu.ipl(), 2);
}

#[test]
fn swintr_raises_vector_27() {
    let mut m = machine(WAITI);
    icu_write(&mut m, ICU_SWINTR, 1);
    assert_eq!(m.cpu.icu.read(27), 1);
    assert_eq!(m.cpu.icu.read(ICU_SWINTR), 0, "SWINTR reads 0");
    assert_eq!(ipr_index(27), Some(3));
    assert_eq!(ipr_index(33), None);
}

// mvtachi r1 ; mvtaclo r2 ; mvfachi r3 ; mvfacmi r4 ; machi r5, r6
// mvfachi r7 ; racw #1 ; mvfachi r8 ; mvtachi r1, a1 ; mvfachi #2, a1, r9
// emaca r5, r6, a1
const ACC: &[u8] = &[
    0xfd, 0x17, 0x01, 0xfd, 0x17, 0x12, 0xfd, 0x1f, 0x03, 0xfd, 0x1f, 0x24, 0xfd, 0x04, 0x56, 0xfd,
    0x1f, 0x07, 0xfd, 0x18, 0x00, 0xfd, 0x1f, 0x08, 0xfd, 0x17, 0x81, 0xfd, 0x1e, 0x89, 0xfd, 0x0f,
    0x56,
];

#[test]
fn accumulator_moves_multiply_accumulate_and_round() {
    let mut m = machine(ACC);
    m.cpu.r[1] = 0x0000_1234;
    m.cpu.r[2] = 0x6789_ABCD;
    m.cpu.r[5] = 0x0003_0000; // high half 3
    m.cpu.r[6] = 0x0004_0000; // high half 4
    run(&mut m, 4);
    assert_eq!(m.cpu.r[3], 0x0000_1234);
    assert_eq!(m.cpu.r[4], 0x1234_6789, "bits 47:16");
    run(&mut m, 2);
    // A0 += (3 * 4) << 16
    assert_eq!(m.cpu.acc[0], 0x0000_1234_6789_ABCD + (12 << 16));
    assert_eq!(m.cpu.r[7], 0x0000_1234);
    run(&mut m, 2);
    // RACW #1: (A0 << 1) + 0x8000_0000, low word cleared.
    assert_eq!(m.cpu.acc[0], 0x0000_2469_0000_0000);
    assert_eq!(m.cpu.r[8], 0x0000_2469);
    run(&mut m, 2);
    // #2 shifts the accumulator before bits 63:32 are taken.
    assert_eq!(m.cpu.r[9], 0x0000_1234 << 2);
    let before = m.cpu.acc[1];
    run(&mut m, 1);
    assert_eq!(m.cpu.acc[1], before + 0x0003_0000 * 0x0004_0000);
    assert_eq!(
        m.cpu.acc[0], 0x0000_2469_0000_0000,
        "A0 untouched by A1 ops"
    );
}

// racw #1
const RACW: &[u8] = &[0xfd, 0x18, 0x00];

#[test]
fn racw_saturates_to_a_signed_word_in_bits_47_32() {
    let mut m = machine(RACW);
    m.cpu.acc[0] = 0x0001_2345_6789_ABCD;
    run(&mut m, 1);
    assert_eq!(m.cpu.acc[0], 0x0000_7FFF_0000_0000);
    let mut m = machine(RACW);
    m.cpu.acc[0] = -0x0001_0000_0000_0000;
    run(&mut m, 1);
    assert_eq!(m.cpu.acc[0], -0x0000_8000_0000_0000);
}

// fadd r1, r2 ; fcmp r2, r1 ; ftoi r2, r3 ; itof r4, r5 ; round r6, r7
// fmul #0x40000000, r1
const FPU: &[u8] = &[
    0xfc, 0x8b, 0x12, 0xfc, 0x87, 0x21, 0xfc, 0x97, 0x23, 0xfc, 0x47, 0x45, 0xfc, 0x9b, 0x67, 0xfd,
    0x72, 0x31, 0x00, 0x00, 0x00, 0x40,
];

#[test]
fn single_precision_fpu() {
    let mut m = machine(FPU);
    m.cpu.r[1] = 1.25f32.to_bits();
    m.cpu.r[2] = (-3.5f32).to_bits();
    m.cpu.r[4] = (-7i32) as u32;
    m.cpu.r[6] = 2.5f32.to_bits();
    run(&mut m, 1);
    assert_eq!(f32::from_bits(m.cpu.r[2]), -2.25);
    assert_ne!(m.cpu.psw() & PSW_S, 0);
    run(&mut m, 1); // fcmp: r1 (1.25) vs r2 (-2.25)
    assert_eq!(m.cpu.psw() & (PSW_Z | PSW_S | PSW_O), 0);
    run(&mut m, 3);
    assert_eq!(m.cpu.r[3] as i32, -2, "FTOI truncates");
    assert_eq!(f32::from_bits(m.cpu.r[5]), -7.0);
    assert_eq!(m.cpu.r[7], 2, "ROUND to nearest even");
    run(&mut m, 1);
    assert_eq!(f32::from_bits(m.cpu.r[1]), 2.5);
}

// bfmov #4, #8, #8, r1, r2 ; bfmovz #0, #28, #4, r1, r3
const BFMOV: &[u8] = &[0xfc, 0x5e, 0x12, 0x04, 0x41, 0xfc, 0x5a, 0x13, 0x9c, 0x03];

#[test]
fn bit_field_moves() {
    let mut m = machine(BFMOV);
    m.cpu.r[1] = 0x0000_0AB5;
    m.cpu.r[2] = 0xFFFF_FFFF;
    m.cpu.r[3] = 0xFFFF_FFFF;
    run(&mut m, 2);
    // Bits 11:4 of r1 (0xAB) into bits 15:8 of r2, the rest kept.
    assert_eq!(m.cpu.r[2], 0xFFFF_ABFF);
    // Bits 3:0 of r1 (0x5) into bits 31:28 of r3, the rest zeroed.
    assert_eq!(m.cpu.r[3], 0x5000_0000);
}

// movli [r2], r1 ; movco r3, [r2] ; movco r3, [r2]
const MOVLI: &[u8] = &[0xfd, 0x2f, 0x21, 0xfd, 0x27, 0x23, 0xfd, 0x27, 0x23];

#[test]
fn movco_succeeds_once_after_movli() {
    let mut m = machine(MOVLI);
    w32(&mut m, DATA, 41);
    m.cpu.r[2] = DATA;
    m.cpu.r[3] = 42;
    run(&mut m, 2);
    assert_eq!(m.cpu.r[1], 41);
    assert_eq!(r32(&m, DATA), 42);
    assert_eq!(m.cpu.r[3], 0, "success");
    m.cpu.r[3] = 43;
    run(&mut m, 1);
    assert_eq!(m.cpu.r[3], 1, "the reservation is gone");
    assert_eq!(r32(&m, DATA), 42);
}

// .byte 0xff, 0xff, 0xff ; nop
const UNDEF: &[u8] = &[0xff, 0xff, 0xff, 0x03];

#[test]
fn undefined_instruction_vectors_through_extb_and_saves_its_own_pc() {
    let mut m = machine(UNDEF);
    let handler = CODE + 0x300;
    w32(&mut m, EXTB + EXC_UNDEFINED, handler);
    m.cpu.set_psw(PSW_I);
    run(&mut m, 1);
    assert_eq!(m.cpu.pc, handler);
    assert_eq!(r32(&m, m.cpu.r[0]), CODE, "the faulting instruction");
    assert_eq!(m.cpu.psw() & PSW_I, 0);
}

#[test]
fn snapshot_round_trips_the_visible_state() {
    let mut m = machine(ACC);
    m.cpu.r[1] = 0x8000_0001;
    m.cpu.r[2] = 7;
    run(&mut m, 2);
    m.cpu.set_psw(PSW_U | PSW_I | (4 << PSW_IPL_SHIFT));
    m.cpu.set_exception_pending(100);
    let snap = m.cpu.snapshot();
    let mut other = Rx::new();
    other.apply_snapshot(&snap);
    assert_eq!(other.r, m.cpu.r);
    assert_eq!(other.pc, m.cpu.pc);
    assert_eq!(other.psw(), m.cpu.psw());
    assert_eq!(other.acc, m.cpu.acc);
    assert_eq!(other.icu.read(100), 1);
    assert_eq!(other.isp, m.cpu.isp);
}
