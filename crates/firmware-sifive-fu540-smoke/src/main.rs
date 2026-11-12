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
#[no_mangle]
#[used]
static mut SOFTWARE_TRAP_COUNT: u64 = 0;
#[no_mangle]
#[used]
static mut SOFTWARE_TRAP_MCAUSE: u64 = 0;
#[no_mangle]
#[used]
static mut SOFTWARE_TRAP_MEPC: u64 = 0;
#[no_mangle]
#[used]
static mut SOFTWARE_TRAP_MSTATUS: u64 = 0;
#[no_mangle]
#[used]
static mut PLIC_TRAP_COUNT: u64 = 0;
#[no_mangle]
#[used]
static mut PLIC_TRAP_MCAUSE: u64 = 0;
#[no_mangle]
#[used]
static mut PLIC_TRAP_ID: u64 = 0;
#[no_mangle]
#[used]
static mut PLIC_TRAP_IP: u64 = 0;

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
    ".section .text.fu540_software_trap_entry, \"ax\"",
    ".align 2",
    ".global fu540_software_trap_entry",
    ".type fu540_software_trap_entry, @function",
    "fu540_software_trap_entry:",
    // An interrupt can arrive between any two Rust instructions. Preserve the
    // caller-saved ABI state before recording the trap and clearing hart 1 MSIP.
    "addi sp, sp, -128",
    "sd ra, 0(sp)",
    "sd t0, 8(sp)",
    "sd t1, 16(sp)",
    "sd t2, 24(sp)",
    "sd t3, 32(sp)",
    "sd t4, 40(sp)",
    "sd t5, 48(sp)",
    "sd t6, 56(sp)",
    "sd a0, 64(sp)",
    "sd a1, 72(sp)",
    "sd a2, 80(sp)",
    "sd a3, 88(sp)",
    "sd a4, 96(sp)",
    "sd a5, 104(sp)",
    "sd a6, 112(sp)",
    "sd a7, 120(sp)",
    "csrr t0, mcause",
    "la t1, SOFTWARE_TRAP_MCAUSE",
    "sd t0, 0(t1)",
    "csrr t0, mepc",
    "la t1, SOFTWARE_TRAP_MEPC",
    "sd t0, 0(t1)",
    "csrr t0, mstatus",
    "la t1, SOFTWARE_TRAP_MSTATUS",
    "sd t0, 0(t1)",
    "la t1, SOFTWARE_TRAP_COUNT",
    "ld t0, 0(t1)",
    "addi t0, t0, 1",
    "sd t0, 0(t1)",
    // FU540 hart 1's CLINT MSIP word is at base + 4 * mhartid.
    "li t0, 0x02000004",
    "sw zero, 0(t0)",
    "ld ra, 0(sp)",
    "ld t0, 8(sp)",
    "ld t1, 16(sp)",
    "ld t2, 24(sp)",
    "ld t3, 32(sp)",
    "ld t4, 40(sp)",
    "ld t5, 48(sp)",
    "ld t6, 56(sp)",
    "ld a0, 64(sp)",
    "ld a1, 72(sp)",
    "ld a2, 80(sp)",
    "ld a3, 88(sp)",
    "ld a4, 96(sp)",
    "ld a5, 104(sp)",
    "ld a6, 112(sp)",
    "ld a7, 120(sp)",
    "addi sp, sp, 128",
    "mret",
    ".size fu540_software_trap_entry, .-fu540_software_trap_entry",
    ".section .text.fu540_plic_trap_entry, \"ax\"",
    ".align 2",
    ".global fu540_plic_trap_entry",
    ".type fu540_plic_trap_entry, @function",
    "fu540_plic_trap_entry:",
    // Preserve the interrupted caller-saved state; an interrupt is not an ABI call.
    "addi sp, sp, -128",
    "sd ra, 0(sp)",
    "sd t0, 8(sp)",
    "sd t1, 16(sp)",
    "sd t2, 24(sp)",
    "sd t3, 32(sp)",
    "sd t4, 40(sp)",
    "sd t5, 48(sp)",
    "sd t6, 56(sp)",
    "sd a0, 64(sp)",
    "sd a1, 72(sp)",
    "sd a2, 80(sp)",
    "sd a3, 88(sp)",
    "sd a4, 96(sp)",
    "sd a5, 104(sp)",
    "sd a6, 112(sp)",
    "sd a7, 120(sp)",
    "csrr t0, mcause",
    "la t1, PLIC_TRAP_MCAUSE",
    "sd t0, 0(t1)",
    // U54_1 M-mode context claim/complete is 0x0C201004.
    "li t0, 0x0c201004",
    "lw t4, 0(t0)",
    "la t1, PLIC_TRAP_ID",
    "sd t4, 0(t1)",
    "la t1, PLIC_TRAP_COUNT",
    "ld t2, 0(t1)",
    "addi t2, t2, 1",
    "sd t2, 0(t1)",
    "li t0, 0x10010014",
    "lw t2, 0(t0)",
    "la t1, PLIC_TRAP_IP",
    "sd t2, 0(t1)",
    // Disable TXWM to drop the device level, then complete the claimed source.
    "li t0, 0x10010010",
    "sw zero, 0(t0)",
    "li t0, 0x0c201004",
    "sw t4, 0(t0)",
    "ld ra, 0(sp)",
    "ld t0, 8(sp)",
    "ld t1, 16(sp)",
    "ld t2, 24(sp)",
    "ld t3, 32(sp)",
    "ld t4, 40(sp)",
    "ld t5, 48(sp)",
    "ld t6, 56(sp)",
    "ld a0, 64(sp)",
    "ld a1, 72(sp)",
    "ld a2, 80(sp)",
    "ld a3, 88(sp)",
    "ld a4, 96(sp)",
    "ld a5, 104(sp)",
    "ld a6, 112(sp)",
    "ld a7, 120(sp)",
    "addi sp, sp, 128",
    "mret",
    ".size fu540_plic_trap_entry, .-fu540_plic_trap_entry",
);

/// FU540 UART0 at 0x10010000: txdata@+0x00, bit 31 indicates a full FIFO.
const UART0_TXDATA: *mut u32 = 0x1001_0000 as *mut u32;
const UART0_TXCTRL: *mut u32 = 0x1001_0008 as *mut u32;
const UART0_IE: *mut u32 = 0x1001_0010 as *mut u32;
const UART0_IP: *const u32 = 0x1001_0014 as *const u32;
const PLIC_PRIORITY4: *mut u32 = 0x0C00_0010 as *mut u32;
const PLIC_ENABLE_HART1: *mut u32 = 0x0C00_2080 as *mut u32;
const PLIC_PENDING: *const u32 = 0x0C00_1000 as *const u32;
const PLIC_THRESHOLD_HART1: *mut u32 = 0x0C20_1000 as *mut u32;
/// FU540 CLINT `msip` register for hart 1.
const CLINT_MSIP_HART1: *mut u32 = 0x0200_0004 as *mut u32;
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

/// Set hart 1's CLINT MSIP and verify its M-mode interrupt and MRET path.
fn software_interrupt_probe() -> bool {
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(SOFTWARE_TRAP_COUNT), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(SOFTWARE_TRAP_MCAUSE), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(SOFTWARE_TRAP_MEPC), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(SOFTWARE_TRAP_MSTATUS), 0);
        core::ptr::write_volatile(CLINT_MSIP_HART1, 1);
    }

    let pending_before: u64;
    unsafe {
        core::arch::asm!(
            "csrr {mip}, mip",
            "srli {mip}, {mip}, 3",
            "andi {mip}, {mip}, 1",
            mip = lateout(reg) pending_before,
            options(nomem, nostack),
        );
    }

    let loop_start: u64;
    let loop_end: u64;
    let returned_mstatus: u64;
    unsafe {
        core::arch::asm!(
            "la {loop_start}, 1f",
            "la t0, fu540_software_trap_entry",
            "csrw mtvec, t0",
            "csrs mie, {msie}",
            "csrs mstatus, {mie}",
            "1: li t2, 20000",
            "2: addi t2, t2, -1",
            "bnez t2, 2b",
            "la {loop_end}, 3f",
            "3: csrr {returned_mstatus}, mstatus",
            "csrc mstatus, {mie}",
            msie = in(reg) 1u64 << 3,
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
            core::ptr::read_volatile(core::ptr::addr_of!(SOFTWARE_TRAP_COUNT)),
            core::ptr::read_volatile(core::ptr::addr_of!(SOFTWARE_TRAP_MCAUSE)),
            core::ptr::read_volatile(core::ptr::addr_of!(SOFTWARE_TRAP_MEPC)),
            core::ptr::read_volatile(core::ptr::addr_of!(SOFTWARE_TRAP_MSTATUS)),
        )
    };
    let msip_cleared = unsafe { core::ptr::read_volatile(CLINT_MSIP_HART1) == 0 };
    let pending_after: u64;
    unsafe {
        core::arch::asm!(
            "csrr {mip}, mip",
            "srli {mip}, {mip}, 3",
            "andi {mip}, {mip}, 1",
            mip = lateout(reg) pending_after,
            options(nomem, nostack),
        );
    }
    let mepc_in_wait = mepc >= loop_start && mepc < loop_end;
    let entry_mie_clear = entry_mstatus & (1 << 3) == 0;
    let entry_mpie_set = entry_mstatus & (1 << 7) != 0;
    let entry_mpp_machine = (entry_mstatus >> 11) & 3 == 3;
    let return_mie_set = returned_mstatus & (1 << 3) != 0;
    let pass = pending_before == 1
        && count == 1
        && cause == ((1u64 << 63) | 3)
        && mepc_in_wait
        && entry_mie_clear
        && entry_mpie_set
        && entry_mpp_machine
        && return_mie_set
        && msip_cleared
        && pending_after == 0;

    puts("software IRQ mcause=");
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
    puts(" msip_pending=");
    putc(if pending_before == 1 { b'1' } else { b'0' });
    puts(" msip_clear=");
    putc(if msip_cleared && pending_after == 0 {
        b'1'
    } else {
        b'0'
    });
    puts(if pass { " ok\n" } else { " FAIL\n" });
    pass
}

/// Exercise FU540 PLIC thresholding, then claim and complete UART0 TXWM.
fn plic_uart_interrupt_probe() -> bool {
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(PLIC_TRAP_COUNT), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(PLIC_TRAP_MCAUSE), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(PLIC_TRAP_ID), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!(PLIC_TRAP_IP), 0);
        core::ptr::write_volatile(PLIC_PRIORITY4, 3);
        core::ptr::write_volatile(PLIC_ENABLE_HART1, 1 << 4);
        // PLIC only notifies a context when source priority is strictly above
        // its threshold. Start at the same priority to prove the pending source
        // remains masked until firmware lowers the threshold.
        core::ptr::write_volatile(PLIC_THRESHOLD_HART1, 3);
    }

    unsafe {
        core::arch::asm!(
            "la t0, fu540_plic_trap_entry",
            "csrw mtvec, t0",
            "csrs mie, {meie}",
            "csrs mstatus, {mie}",
            meie = in(reg) 1u64 << 11,
            mie = in(reg) 1u64 << 3,
            out("t0") _,
            options(nostack),
        );
    }
    unsafe {
        core::ptr::write_volatile(UART0_TXCTRL, (1 << 16) | 1); // TXEN, TXCNT=1.
        core::ptr::write_volatile(UART0_IE, 1); // TXWM interrupt enable.
    }

    for _ in 0..10_000 {
        if unsafe { core::ptr::read_volatile(PLIC_PENDING) } & (1 << 4) != 0 {
            break;
        }
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let masked_pending = unsafe { core::ptr::read_volatile(PLIC_PENDING) } & (1 << 4) != 0;
    let masked_count = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(PLIC_TRAP_COUNT)) };
    let masked_meip: u64;
    unsafe {
        core::arch::asm!(
            "csrr {mip}, mip",
            "srli {mip}, {mip}, 11",
            "andi {mip}, {mip}, 1",
            mip = lateout(reg) masked_meip,
            options(nomem, nostack),
        );
        core::ptr::write_volatile(PLIC_THRESHOLD_HART1, 2);
    }

    for _ in 0..10_000 {
        if unsafe { core::ptr::read_volatile(core::ptr::addr_of!(PLIC_TRAP_COUNT)) } != 0 {
            break;
        }
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    unsafe {
        core::arch::asm!("csrc mstatus, {mie}", mie = in(reg) 1u64 << 3, options(nomem, nostack));
    }

    let (count, cause, id, ip, pending, released_threshold) = unsafe {
        (
            core::ptr::read_volatile(core::ptr::addr_of!(PLIC_TRAP_COUNT)),
            core::ptr::read_volatile(core::ptr::addr_of!(PLIC_TRAP_MCAUSE)),
            core::ptr::read_volatile(core::ptr::addr_of!(PLIC_TRAP_ID)),
            core::ptr::read_volatile(core::ptr::addr_of!(PLIC_TRAP_IP)),
            core::ptr::read_volatile(PLIC_PENDING),
            core::ptr::read_volatile(PLIC_THRESHOLD_HART1) == 2,
        )
    };
    let meip_clear: u64;
    unsafe {
        core::arch::asm!("csrr {mip}, mip", "srli {mip}, {mip}, 11", "andi {mip}, {mip}, 1", mip = lateout(reg) meip_clear, options(nomem, nostack));
    }
    let pass = count == 1
        && masked_pending
        && masked_count == 0
        && masked_meip == 0
        && released_threshold
        && cause == ((1u64 << 63) | 11)
        && id == 4
        && ip == 1
        && pending & (1 << 4) == 0
        && meip_clear == 0;

    puts("PLIC UART0 source=");
    put_hex64(id);
    puts(" cause=");
    put_hex64(cause);
    puts(" count=");
    put_hex64(count);
    puts(" ip=");
    put_hex64(ip);
    puts(" masked_pending=");
    putc(if masked_pending { b'1' } else { b'0' });
    puts(" masked_meip=");
    putc(if masked_meip == 0 { b'0' } else { b'1' });
    puts(" threshold_release=");
    putc(if released_threshold { b'1' } else { b'0' });
    puts(" pending=");
    put_hex64(u64::from(pending));
    puts(" meip_clear=");
    put_hex64(meip_clear);
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
    let _software_pass = software_interrupt_probe();
    let _plic_pass = plic_uart_interrupt_probe();

    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
