// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Exact-import K210 smoke: one RV64IMAC hart executes a 64-bit multiply and
//! sends the result through Renode's mapped SiFive UART.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// K210 UART at 0x38000000: txdata@+0x00, bit 31 indicates a full FIFO.
const UART_TXDATA: *mut u32 = 0x3800_0000 as *mut u32;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART_TXDATA) & (1 << 31) != 0 {}
        core::ptr::write_volatile(UART_TXDATA, byte as u32);
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
    puts("RV64 K210 UART smoke; mul=");
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
