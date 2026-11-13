// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! LiteX Ibex smoke against the narrowed imported RISC-V profile.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// LiteX UART CSR window (rxtx @ +0x00, txfull @ +0x04).
const UART_RXTX: *mut u32 = 0x8200_2000 as *mut u32;
const UART_TXFULL: *const u32 = 0x8200_2004 as *const u32;

/// LiteX timer CSR window at the address from the imported profile.
const TIMER_LOAD: *mut u32 = 0x8200_2800 as *mut u32;
const TIMER_RELOAD: *mut u32 = 0x8200_2804 as *mut u32;
const TIMER_EN: *mut u32 = 0x8200_2808 as *mut u32;
const TIMER_UPDATE_VALUE: *mut u32 = 0x8200_280C as *mut u32;
const TIMER_VALUE: *const u32 = 0x8200_2810 as *const u32;
const TIMER_EV_STATUS: *const u32 = 0x8200_2814 as *const u32;
const TIMER_EV_PENDING: *mut u32 = 0x8200_2818 as *mut u32;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART_TXFULL) != 0 {}
        core::ptr::write_volatile(UART_RXTX, byte as u32);
    }
}

fn puts(text: &str) {
    for byte in text.bytes() {
        putc(byte);
    }
}

fn put_hex32(value: u32) {
    for shift in (0..8).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
}

#[inline(never)]
fn multiply(left: u32, right: u32) -> u32 {
    let result: u32;
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

fn timer_one_shot(cycles: u32) {
    unsafe {
        // ev_pending is W1C: a stale or freshly written 1 must not read back.
        // A plain R/W register at this offset (e.g. an STM32 CCMR1) keeps it.
        core::ptr::write_volatile(TIMER_EV_PENDING, 1);
        if core::ptr::read_volatile(TIMER_EV_PENDING) & 1 == 0 {
            puts("timer=cleared\n");
        } else {
            puts("timer=pending_kept\n");
        }
        core::ptr::write_volatile(TIMER_LOAD, cycles);
        core::ptr::write_volatile(TIMER_RELOAD, 0);
        puts("load=");
        put_hex32(core::ptr::read_volatile(TIMER_LOAD));
        putc(b'\n');
        core::ptr::write_volatile(TIMER_EN, 1);

        // update_value latches the live countdown into value.
        core::ptr::write_volatile(TIMER_UPDATE_VALUE, 1);
        let value = core::ptr::read_volatile(TIMER_VALUE);
        if value != 0 && value < cycles {
            puts("timer=counting\n");
        } else {
            puts("value=");
            put_hex32(value);
            putc(b'\n');
        }

        if core::ptr::read_volatile(TIMER_EV_STATUS) & 1 == 0 {
            puts("timer=running\n");
        } else {
            puts("timer=stuck\n");
        }

        while core::ptr::read_volatile(TIMER_EV_PENDING) & 1 == 0 {}
        puts("timer=zero\n");
    }
}

#[entry]
fn main() -> ! {
    puts("LITEX IBEX SMOKE\n");
    puts("cpu=rv32imc uart=litex\n");
    puts("mul=");
    put_hex32(multiply(17, 19));
    putc(b'\n');

    timer_one_shot(20_000);
    loop {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}
