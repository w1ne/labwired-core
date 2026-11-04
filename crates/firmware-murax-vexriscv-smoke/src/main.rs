// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Exact-import Murax smoke: RV32IM execution and the Murax UART.
//!
//! The ELF is loaded directly into the imported 0x80000000 memory window. The
//! smoke does not claim a boot ROM, GPIO, timer, or interrupt path.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// Murax UART: DATA@+0x00; STATUS@+0x04, TX FIFO free count in bits [23:16].
const UART_DATA: *mut u32 = 0xF001_0000 as *mut u32;
const UART_STATUS: *const u32 = 0xF001_0004 as *const u32;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART_STATUS) & 0x00FF_0000 == 0 {}
        core::ptr::write_volatile(UART_DATA, byte as u32);
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

#[entry]
fn main() -> ! {
    puts("MURAX RV32IM SMOKE\n");
    puts("cpu=rv32im uart=murax\n");
    puts("mul=");
    put_hex32(multiply(17, 19));
    putc(b'\n');

    loop {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}
