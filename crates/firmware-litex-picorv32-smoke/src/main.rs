// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Direct-ELF smoke for the imported LiteX PicoRV32 UART and timer profile.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// LiteX UART CSR window (rxtx @ +0x00, txfull @ +0x04).
const UART_RXTX: *mut u32 = 0xE000_1800 as *mut u32;
const UART_TXFULL: *const u32 = 0xE000_1804 as *const u32;

/// LiteX timer CSR window at the address in Renode's PicoRV32 REPL.
const TIMER_LOAD: *mut u32 = 0xE000_2000 as *mut u32;
const TIMER_RELOAD: *mut u32 = 0xE000_2004 as *mut u32;
const TIMER_EN: *mut u32 = 0xE000_2008 as *mut u32;
const TIMER_EV_STATUS: *const u32 = 0xE000_2014 as *const u32;
const TIMER_EV_PENDING: *mut u32 = 0xE000_2018 as *mut u32;

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
        core::ptr::write_volatile(TIMER_EV_PENDING, 1);
        core::ptr::write_volatile(TIMER_LOAD, cycles);
        core::ptr::write_volatile(TIMER_RELOAD, 0);
        core::ptr::write_volatile(TIMER_EN, 1);

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
    puts("LITEX PICORV32 SMOKE\n");
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
