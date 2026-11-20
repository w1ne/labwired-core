// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Bounded imported JH7100 smoke: one RV64IMAC hart multiplies, polls the
//! wide-register UART0 status, and writes its TX window. It does not exercise
//! supervisor-mode boot, floating point, interrupts, or the second U74 hart.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// JH7100 UART0 at 0x11870000: THR@+0 and LSR@+0x14, 32-bit wide registers.
const UART0_THR: *mut u32 = 0x1187_0000 as *mut u32;
const UART0_LSR: *const u32 = 0x1187_0014 as *const u32;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART0_LSR) & (1 << 5) == 0 {}
        core::ptr::write_volatile(UART0_THR, byte as u32);
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

#[entry]
fn main() -> ! {
    let product = multiply(0x1_0000_0001, 17);
    puts("RV64 JH7100 U74 UART0 smoke; mul=");
    for shift in (0..16).rev() {
        let nibble = ((product >> (shift * 4)) & 0xF) as u8;
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
    putc(b'\n');

    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
