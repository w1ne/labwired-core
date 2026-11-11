// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Exact-import FU540 smoke: one RV64IMAC U54 hart executes 64-bit multiply,
//! services a CLINT machine-timer interrupt, and sends a banner through the
//! model-backed SiFive UART0.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

#[no_mangle]
#[used]
static mut TIMER_TRAP_COUNT: u64 = 0;
#[no_mangle]
#[used]
static mut TIMER_TRAP_MCAUSE: u64 = 0;
#[no_mangle]
#[used]
static mut TIMER_TRAP_MEPC: u64 = 0;
#[no_mangle]
#[used]
static mut TIMER_TRAP_MSTATUS: u64 = 0;

core::arch::global_asm!(
    ".section .text.fu540_trap_entry, \"ax\"",
    ".align 2",
    ".global fu540_trap_entry",
    ".type fu540_trap_entry, @function",
    "fu540_trap_entry:",
    "csrr t0, mcause",
    "la t1, TIMER_TRAP_MCAUSE",
    "sd t0, 0(t1)",
    "csrr t0, mepc",
    "la t1, TIMER_TRAP_MEPC",
    "sd t0, 0(t1)",
    "csrr t0, mstatus",
    "la t1, TIMER_TRAP_MSTATUS",
    "sd t0, 0(t1)",
    "la t1, TIMER_TRAP_COUNT",
    "ld t0, 0(t1)",
    "addi t0, t0, 1",
    "sd t0, 0(t1)",
    // Disarm the hart-1 compare before MRET so the level-triggered MTIP drops.
    "li t0, -1",
    "li t1, 0x02004008",
    "sd t0, 0(t1)",
    "mret",
    ".size fu540_trap_entry, .-fu540_trap_entry",
);

/// FU540 UART0 at 0x10010000: txdata@+0x00, bit 31 indicates a full FIFO.
const UART0_TXDATA: *mut u32 = 0x1001_0000 as *mut u32;
/// FU540 CLINT `mtime` and hart-1 `mtimecmp` registers.
const CLINT_MTIMECMP_HART1: *mut u64 = 0x0200_4008 as *mut u64;
const CLINT_MTIME: *const u64 = 0x0200_BFF8 as *const u64;

fn put_hex64(value: u64) {
    for shift in (0..16).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
}

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART0_TXDATA) & (1 << 31) != 0 {}
        core::ptr::write_volatile(UART0_TXDATA, byte as u32);
    }
}

fn puts(text: &str) {
    for byte in text.bytes() {
        putc(byte);
    }
}

#[inline(never)]
fn multiply(left: u64, right: u64) -> u64 {
    let result: u64;
    unsafe {
        core::arch::asm!(
            "mul {result}, {left}, {right}",
            result = lateout(reg) result,
            left = in(reg) left,
            right = in(reg) right,
            options(nomem, nostack, preserves_flags),
        );
    }
    result
}

/// Arm the U54's CLINT timer and verify M-mode interrupt entry and MRET.
fn timer_interrupt_probe() -> bool {
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(TIMER_TRAP_COUNT), 0);
        let now = core::ptr::read_volatile(CLINT_MTIME);
        core::ptr::write_volatile(CLINT_MTIMECMP_HART1, now.wrapping_add(5));
    }

    let loop_start: u64;
    let loop_end: u64;
    let returned_mstatus: u64;
    unsafe {
        core::arch::asm!(
            "la t0, fu540_trap_entry",
            "csrw mtvec, t0",
            "csrs mie, {mtie}",
            "csrs mstatus, {mie}",
            "li t2, 20000",
            "la {loop_start}, 1f",
            "1: addi t2, t2, -1",
            "bnez t2, 1b",
            "la {loop_end}, 2f",
            "2: csrr {returned_mstatus}, mstatus",
            "csrc mstatus, {mie}",
            mtie = in(reg) 1u64 << 7,
            mie = in(reg) 1u64 << 3,
            loop_start = lateout(reg) loop_start,
            loop_end = lateout(reg) loop_end,
            returned_mstatus = lateout(reg) returned_mstatus,
            out("t0") _,
            out("t1") _,
            out("t2") _,
            options(nostack),
        );
    }

    let (count, cause, mepc, entry_mstatus) = unsafe {
        (
            core::ptr::read_volatile(core::ptr::addr_of!(TIMER_TRAP_COUNT)),
            core::ptr::read_volatile(core::ptr::addr_of!(TIMER_TRAP_MCAUSE)),
            core::ptr::read_volatile(core::ptr::addr_of!(TIMER_TRAP_MEPC)),
            core::ptr::read_volatile(core::ptr::addr_of!(TIMER_TRAP_MSTATUS)),
        )
    };
    let compare_disarmed = unsafe { core::ptr::read_volatile(CLINT_MTIMECMP_HART1) } == u64::MAX;
    let pending_after: u64;
    unsafe { core::arch::asm!("csrr {}, mip", out(reg) pending_after, options(nomem, nostack)) };
    let mepc_in_wait = mepc >= loop_start && mepc < loop_end;
    let entry_mie_clear = entry_mstatus & (1 << 3) == 0;
    let entry_mpie_set = entry_mstatus & (1 << 7) != 0;
    let entry_mpp_machine = (entry_mstatus >> 11) & 3 == 3;
    let return_mie_set = returned_mstatus & (1 << 3) != 0;
    let mtip_clear = pending_after & (1 << 7) == 0;
    let pass = count == 1
        && cause == ((1u64 << 63) | 7)
        && mepc_in_wait
        && entry_mie_clear
        && entry_mpie_set
        && entry_mpp_machine
        && return_mie_set
        && compare_disarmed
        && mtip_clear;

    puts("timer IRQ mcause=");
    put_hex64(cause);
    puts(" count=");
    put_hex64(count);
    puts(" mepc_in_wait=");
    putc(if mepc_in_wait { b'1' } else { b'0' });
    puts(" entry_mie=");
    putc(if entry_mie_clear { b'0' } else { b'1' });
    puts(" entry_mpie=");
    putc(if entry_mpie_set { b'1' } else { b'0' });
    puts(" entry_mpp=");
    putc(b'0' + ((entry_mstatus >> 11) & 3) as u8);
    puts(" return_mie=");
    putc(if return_mie_set { b'1' } else { b'0' });
    puts(" compare_disarmed=");
    putc(if compare_disarmed { b'1' } else { b'0' });
    puts(" mtip_clear=");
    putc(if mtip_clear { b'1' } else { b'0' });
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

#[entry]
fn main() -> ! {
    let product = multiply(0x1_0000_0001, 17);
    puts("RV64 FU540 UART0 smoke; mul=");
    for shift in (0..16).rev() {
        let nibble = ((product >> (shift * 4)) & 0xF) as u8;
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
    putc(b'\n');

    let _timer_pass = timer_interrupt_probe();

    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
