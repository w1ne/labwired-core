// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! AArch64 PE unit tests. Instruction words come from GNU `as` (Zephyr SDK
//! 1.0.1 `aarch64-zephyr-elf`), never from this decoder; every expected value
//! is worked from the Arm ARM pseudocode, not read back from a run.

use super::*;
use crate::bus::SystemBus;
use crate::{Bus, Machine};

const RAM: u64 = 0x2000_0000;

/// A machine whose RAM holds `code` at `RAM`, PC there, at `el` with SP_ELx.
fn machine(code: &[u32], el: u8) -> Machine<Aarch64> {
    let mut bus = SystemBus::new();
    for (i, w) in code.iter().enumerate() {
        bus.ram.data[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
    }
    let mut cpu = Aarch64::new(Aarch64Params::for_chip_name("", 1));
    cpu.pc = RAM;
    cpu.el = el;
    Machine::new(cpu, bus)
}

fn run(m: &mut Machine<Aarch64>, n: usize) {
    for _ in 0..n {
        m.step().unwrap();
    }
}

fn esr_ec(esr: u64) -> u32 {
    (esr >> 26) as u32
}

#[test]
fn data_processing_matches_the_pseudocode() {
    let mut m = machine(
        &[
            0xd2a24680, // mov x0, #0x12340000
            0xf28acf00, // movk x0, #0x5678
            0x92800001, // mov x1, #-1
            0xb1000422, // adds x2, x1, #1
            0x92f00003, // mov x3, #0x7fffffffffffffff
            0xb1000464, // adds x4, x3, #1
            0xeb000005, // subs x5, x0, x0
            0x9a9f17e6, // cset x6, eq
            0xb200f3e7, // mov x7, #0x5555555555555555
            0xd3442c08, // ubfx x8, x0, #4, #8
            0x93401c29, // sxtb x9, w1
            0xb3780c0a, // bfi x10, x0, #8, #4
            0x93c7400b, // extr x11, x0, x7, #16
            0x5ac0000c, // rbit w12, w0
            0xdac0100d, // clz x13, x0
            0xdac00c0e, // rev x14, x0
            0x928000cf, // mov x15, #-7
            0xd2800050, // mov x16, #2
            0x9ad00df1, // sdiv x17, x15, x16
            0x9adf09f2, // udiv x18, x15, xzr
            0x9b507df3, // smulh x19, x15, x16
            0x9bd07df4, // umulh x20, x15, x16
            0x9b1001f5, // madd x21, x15, x16, x0
            0xd3440c16, // lsl x22, x0, #60
            0x937ffed7, // asr x23, x22, #63
            0x110005f8, // add w24, w15, #1
        ],
        1,
    );
    run(&mut m, 4);
    // 0xFFFF_FFFF_FFFF_FFFF + 1: Z and C, no V.
    assert_eq!(m.cpu.x[2], 0);
    assert_eq!(m.cpu.nzcv, PSTATE_Z | PSTATE_C);
    run(&mut m, 2);
    // INT64_MAX + 1: N and V.
    assert_eq!(m.cpu.x[4], 1 << 63);
    assert_eq!(m.cpu.nzcv, PSTATE_N | PSTATE_V);
    run(&mut m, 20);
    let x = &m.cpu.x;
    assert_eq!(x[0], 0x1234_5678);
    assert_eq!(x[5], 0);
    assert_eq!(m.cpu.nzcv, PSTATE_Z | PSTATE_C);
    assert_eq!(x[6], 1);
    assert_eq!(x[7], 0x5555_5555_5555_5555);
    assert_eq!(x[8], 0x67);
    assert_eq!(x[9], u64::MAX);
    assert_eq!(x[10], 0x800);
    assert_eq!(x[11], 0x5678_5555_5555_5555);
    assert_eq!(x[12], 0x1E6A_2C48);
    assert_eq!(x[13], 35);
    assert_eq!(x[14], 0x7856_3412_0000_0000);
    assert_eq!(x[17], (-3i64) as u64);
    assert_eq!(x[18], 0, "UDIV by zero is zero");
    assert_eq!(x[19], u64::MAX);
    assert_eq!(x[20], 1);
    assert_eq!(x[21], 0x1234_566A);
    assert_eq!(x[22], 0x8000_0000_0000_0000);
    assert_eq!(x[23], u64::MAX);
    assert_eq!(x[24], 0xFFFF_FFFA, "W-register results zero the upper half");
}

#[test]
fn loads_stores_and_the_exclusive_monitor() {
    let mut m = machine(
        &[
            0xd2a40000, // mov x0, #0x20000000
            0x91200000, // add x0, x0, #0x800
            0xd2822441, // mov x1, #0x1122
            0xf2a66881, // movk x1, #0x3344, lsl #16
            0xf2caacc1, // movk x1, #0x5566, lsl #32
            0xf2f0f101, // movk x1, #0x8788, lsl #48
            0xf8008c01, // str x1, [x0, #8]!
            0xf8408402, // ldr x2, [x0], #8
            0xb89fc003, // ldursw x3, [x0, #-4]
            0x385ff004, // ldurb w4, [x0, #-1]
            0x389ff005, // ldursb x5, [x0, #-1]
            0xa9010801, // stp x1, x2, [x0, #16]
            0x29421c06, // ldp w6, w7, [x0, #16]
            0x12800008, // mov w8, #-1
            0xf868d809, // ldr x9, [x0, w8, sxtw #3]
            0xc85f7c0a, // ldxr x10, [x0]
            0xc80b7c01, // stxr w11, x1, [x0]
            0xc80c7c01, // stxr w12, x1, [x0]
            0x885ffc0d, // ldaxr w13, [x0]
            0xd5033f5f, // clrex
            0x880efc0d, // stlxr w14, w13, [x0]
        ],
        1,
    );
    run(&mut m, 21);
    let v = 0x8788_5566_3344_1122u64;
    let x = &m.cpu.x;
    assert_eq!(x[0], 0x2000_0810, "pre- then post-index writeback");
    assert_eq!(x[2], v);
    assert_eq!(x[3], 0xFFFF_FFFF_8788_5566);
    assert_eq!(x[4], 0x87);
    assert_eq!(x[5], 0xFFFF_FFFF_FFFF_FF87);
    assert_eq!((x[6], x[7]), (0x3344_1122, 0x8788_5566));
    assert_eq!(x[9], v, "SXTW #3 of -1 addresses the doubleword below");
    assert_eq!(x[10], 0);
    assert_eq!(x[11], 0, "STXR after LDXR succeeds");
    assert_eq!(x[12], 1, "the monitor is clear after a store-exclusive");
    assert_eq!(x[13], 0x3344_1122);
    assert_eq!(x[14], 1, "CLREX clears the monitor");
    assert_eq!(m.bus.read_u64(0x2000_0810).unwrap(), v);
}

#[test]
fn branches_link_and_return() {
    let mut m = machine(
        &[
            0xd2800000, // mov x0, #0
            0x94000002, // bl +8
            0x14000007, // b +0x1c
            0x91000400, // add x0, x0, #1
            0xb4000041, // cbz x1, +8
            0x91019000, // add x0, x0, #100
            0x37000040, // tbnz w0, #0, +8
            0x910fa000, // add x0, x0, #1000
            0xd65f03c0, // ret
            0xf100041f, // cmp x0, #1
            0x54000040, // b.eq +8
            0xd29bd5a0, // mov x0, #0xdead
            0xd503201f, // nop
        ],
        1,
    );
    run(&mut m, 10);
    assert_eq!(m.cpu.x[0], 1);
    assert_eq!(m.cpu.x[30], RAM + 8);
    assert_eq!(m.cpu.pc, RAM + 0x34);
}

#[test]
fn svc_and_undefined_enter_el1_with_the_architected_syndrome() {
    let mut m = machine(
        &[
            0xd4000841, // svc #0x42
            0x00000000, // udf #0
        ],
        1,
    );
    m.cpu.vbar[1] = RAM + 0x800;
    m.cpu.daif = 0;
    run(&mut m, 1);
    // Current EL with SP_ELx: synchronous vector at +0x200.
    assert_eq!(m.cpu.pc, RAM + 0x800 + 0x200);
    assert_eq!(m.cpu.esr[1], (0x15 << 26) | (1 << 25) | 0x42);
    assert_eq!(m.cpu.elr[1], RAM + 4, "SVC returns to the next instruction");
    assert_eq!(m.cpu.spsr[1], 0b0101, "EL1h, DAIF clear");
    assert_eq!(m.cpu.daif, 0xF << 6, "exception entry masks DAIF");
    m.cpu.pc = RAM + 4;
    run(&mut m, 1);
    assert_eq!(esr_ec(m.cpu.esr[1]), ec::UNKNOWN);
    assert_eq!(m.cpu.elr[1], RAM + 4, "UNDEFINED returns to itself");
}

#[test]
fn eret_from_el3_drops_to_el1_and_currentel_reports_it() {
    let mut code = vec![0u32; 0x20];
    code[0] = 0xd69f03e0; // eret
    code[0x10] = 0xd5384240; // mrs x0, currentel
    let mut m = machine(&code, 3);
    m.cpu.spsr[3] = (0xF << 6) | 0b0101;
    m.cpu.elr[3] = RAM + 0x40;
    run(&mut m, 2);
    assert_eq!(m.cpu.el, 1);
    assert!(m.cpu.spsel);
    assert_eq!(m.cpu.x[0], 1 << 2);
    // An ERET to a higher EL is an illegal exception return.
    let mut m = machine(&[0xd69f03e0], 1);
    m.cpu.spsr[1] = 0b1101; // EL3h
    run(&mut m, 1);
    assert_eq!(m.cpu.el, 1);
    assert_eq!(esr_ec(m.cpu.esr[1]), ec::ILLEGAL_STATE);
}

#[test]
fn fp_instructions_trap_until_cpacr_enables_them() {
    let mut m = machine(&[0x9e670020 /* fmov d0, x1 */], 1);
    m.cpu.x[1] = 0xDEAD_BEEF_0000_0001;
    m.cpu.vbar[1] = RAM + 0x800;
    run(&mut m, 1);
    assert_eq!(esr_ec(m.cpu.esr[1]), ec::FP_ACCESS);
    assert_eq!(m.cpu.elr[1], RAM);
    assert_eq!(m.cpu.v[0], 0);
    m.cpu.el = 1;
    m.cpu.pc = RAM;
    m.cpu.cpacr_el1 = 3 << 20;
    run(&mut m, 1);
    assert_eq!(m.cpu.v[0], 0xDEAD_BEEF_0000_0001);
}

/// Stage-1 tables for VA 0x2000_0000.. with a 32-bit input range (T0SZ =
/// 32, start at level 1): page 0 identity RW, page 1 -> PA page 2 read-only,
/// page 2 invalid.
fn map_pages(m: &mut Machine<Aarch64>) {
    let l1 = RAM + 0x1_0000;
    let l2 = RAM + 0x1_1000;
    let l3 = RAM + 0x1_2000;
    let af = 1 << 10;
    m.bus.write_u64(l1, l2 | 0b11).unwrap();
    m.bus.write_u64(l2 + 8 * 0x100, l3 | 0b11).unwrap();
    m.bus.write_u64(l3, RAM | af | 0b11).unwrap();
    m.bus
        .write_u64(l3 + 8, (RAM + 0x2000) | af | (0b10 << 6) | 0b11)
        .unwrap();
    m.cpu.ttbr0[1] = l1;
    m.cpu.tcr[1] = 32;
    m.cpu.sctlr[1] |= 1;
    m.cpu.flush_tlb();
}

#[test]
fn mmu_translates_and_raises_permission_faults() {
    let mut m = machine(
        &[
            0xf9400001, // ldr x1, [x0]
            0xf9000001, // str x1, [x0]
        ],
        1,
    );
    m.bus
        .write_u64(RAM + 0x2000, 0x1122_3344_5566_7788)
        .unwrap();
    map_pages(&mut m);
    m.cpu.vbar[1] = RAM + 0x800;
    m.cpu.x[0] = RAM + 0x1000;
    run(&mut m, 1);
    assert_eq!(
        m.cpu.x[1], 0x1122_3344_5566_7788,
        "VA page 1 reads PA page 2"
    );
    run(&mut m, 1);
    // Data abort, same EL, permission fault level 3, WnR.
    assert_eq!(esr_ec(m.cpu.esr[1]), ec::DABT_SAME);
    assert_eq!(m.cpu.esr[1] & 0x7F, 0x0F | (1 << 6));
    assert_eq!(m.cpu.far[1], RAM + 0x1000);
    assert_eq!(m.cpu.elr[1], RAM + 4);
}

#[test]
fn at_s1e1r_reports_the_translation_in_par() {
    let mut m = machine(
        &[
            0xd5087800, // at s1e1r, x0
            0xd5387403, // mrs x3, par_el1
            0xd5087800, // at s1e1r, x0
        ],
        1,
    );
    map_pages(&mut m);
    m.cpu.x[0] = RAM + 0x1000;
    run(&mut m, 2);
    assert_eq!(m.cpu.x[3] & 0xFFFF_FFFF_F001, RAM + 0x2000);
    m.cpu.x[0] = RAM + 0x2000;
    run(&mut m, 1);
    // F = 1, FST = translation fault level 3.
    assert_eq!(m.cpu.par_el1, 1 | (0x07 << 1));
}

#[test]
fn virtual_timer_irq_through_the_gic_reaches_the_el1_irq_vector() {
    let mut m = machine(
        &[
            0xd50342ff, // msr daifclr, #2
            0xd503201f, // nop
            0xd503201f, // nop
            0xd503201f, // nop
            0xd503201f, // nop
        ],
        1,
    );
    m.cpu.vbar[1] = RAM + 0x800;
    let g = &mut m.cpu.gic;
    g.enabled[aarch64_gic_ppi()] = true;
    g.priority[aarch64_gic_ppi()] = 0xA0;
    g.pmr = 0xF0;
    g.igrpen1 = true;
    m.cpu.timer.virt.ctl = 1;
    m.cpu.timer.virt.cval = 3;
    run(&mut m, 2);
    assert_eq!(m.cpu.pc, RAM + 8, "the compare has not been reached");
    run(&mut m, 1);
    assert_eq!(m.cpu.pc, RAM + 0x800 + 0x200 + 0x80);
    assert_eq!(m.cpu.elr[1], RAM + 0xC);
    assert_eq!(m.cpu.read_sysreg(regs::ICC_IAR1_EL1), Some(27));
    // Acknowledged and running at its priority: no longer signalled.
    assert!(!m.cpu.irq_line());
    assert!(m.cpu.write_sysreg(regs::ICC_EOIR1_EL1, 27));
    // The level is still asserted (CVAL passed): pending again after EOI.
    assert!(m.cpu.irq_line());
    assert!(m.cpu.write_sysreg(regs::CNTV_CTL_EL0, 0b11));
    assert!(!m.cpu.irq_line(), "IMASK drops the line");
}

fn aarch64_gic_ppi() -> usize {
    crate::cpu::aarch64_gic::PPI_VIRT_TIMER
}

#[test]
fn wfi_fast_forwards_to_the_timer_compare() {
    let mut m = machine(&[0xd503207f /* wfi */], 1);
    m.cpu.timer.virt.ctl = 1;
    m.cpu.timer.virt.cval = 1000;
    run(&mut m, 1);
    let budget = m.cpu.idle_fast_forward_budget(&m.bus);
    assert_eq!(budget, Some(998));
}

#[test]
fn system_register_access_is_checked_by_el() {
    let mut m = machine(&[0xd5384240 /* mrs x0, currentel */], 0);
    m.cpu.vbar[1] = RAM + 0x800;
    run(&mut m, 1);
    assert_eq!(
        m.cpu.el, 1,
        "EL0 MRS of CurrentEL is UNDEFINED, taken to EL1"
    );
    assert_eq!(esr_ec(m.cpu.esr[1]), ec::UNKNOWN);
    assert_eq!(m.cpu.elr[1], RAM);
    assert_eq!(m.cpu.read_sysreg(regs::MIDR_EL1), Some(0x410F_D034));
    assert_eq!(
        m.cpu.read_sysreg(regs::ID_AA64PFR0_EL1).map(|v| v & 0xFFFF),
        Some(0x1111)
    );
}
